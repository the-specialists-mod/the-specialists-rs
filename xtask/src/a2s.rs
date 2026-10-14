//! Minimal A2S_INFO client (works with GoldSrc, HL25 and FWGS servers, including the challenge
//! reply newer servers send).

use std::fmt;
use std::net::UdpSocket;
use std::time::Duration;

pub struct Info {
    pub name: String,
    pub map: String,
    pub dir: String,
    pub game: String,
    pub players: u8,
    pub max_players: u8,
}

impl fmt::Display for Info {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?} map={} dir={} game={:?} players={}/{}", self.name, self.map, self.dir, self.game, self.players, self.max_players)
    }
}

/// A2S_INFO, falling back to the Xash `info` query that older FWGS builds answer instead.
pub fn info(host: &str, port: u16) -> std::io::Result<Info> {
    a2s_info(host, port).or_else(|_| xash_info(host, port))
}

fn a2s_info(host: &str, port: u16) -> std::io::Result<Info> {
    let s = UdpSocket::bind("0.0.0.0:0")?;
    s.set_read_timeout(Some(Duration::from_millis(1500)))?;
    let query = b"\xff\xff\xff\xffTSource Engine Query\x00".to_vec();
    s.send_to(&query, (host, port))?;
    let mut buf = [0u8; 1400];
    let mut n = s.recv(&mut buf)?;
    if n >= 9 && buf[4] == b'A' {
        let mut q = query.clone();
        q.extend_from_slice(&buf[5..9]);
        s.send_to(&q, (host, port))?;
        n = s.recv(&mut buf)?;
    }
    parse(&buf[..n]).ok_or_else(|| std::io::Error::other("bad A2S_INFO reply"))
}

fn parse(d: &[u8]) -> Option<Info> {
    let kind = *d.get(4)?;
    let mut p = 5;
    let cstr = |p: &mut usize| -> Option<String> {
        let end = d[*p..].iter().position(|&b| b == 0)? + *p;
        let s = String::from_utf8_lossy(&d[*p..end]).into_owned();
        *p = end + 1;
        Some(s)
    };
    match kind {
        // Source-style reply: protocol byte, name, map, folder, game, appid(2), players, max.
        b'I' => {
            p += 1;
            let (name, map, dir, game) = (cstr(&mut p)?, cstr(&mut p)?, cstr(&mut p)?, cstr(&mut p)?);
            p += 2;
            Some(Info { name, map, dir, game, players: *d.get(p)?, max_players: *d.get(p + 1)? })
        }
        // Old GoldSrc reply: address, name, map, folder, game, players, max.
        b'm' => {
            let _addr = cstr(&mut p)?;
            let (name, map, dir, game) = (cstr(&mut p)?, cstr(&mut p)?, cstr(&mut p)?, cstr(&mut p)?);
            Some(Info { name, map, dir, game, players: *d.get(p)?, max_players: *d.get(p + 1)? })
        }
        _ => None,
    }
}

/// Xash connectionless `info <protocol>`: replies `info\n\\key\\value...`.
fn xash_info(host: &str, port: u16) -> std::io::Result<Info> {
    let s = UdpSocket::bind("0.0.0.0:0")?;
    s.set_read_timeout(Some(Duration::from_millis(1500)))?;
    s.send_to(b"\xff\xff\xff\xffinfo 49\n", (host, port))?;
    let mut buf = [0u8; 1400];
    let n = s.recv(&mut buf)?;
    let text = String::from_utf8_lossy(&buf[4..n]);
    let body = text.strip_prefix("info\n").ok_or_else(|| std::io::Error::other("bad info reply"))?;
    let mut parts = body.trim_start_matches('\\').split('\\');
    let mut kv = std::collections::HashMap::new();
    while let (Some(k), Some(v)) = (parts.next(), parts.next()) {
        kv.insert(k.to_owned(), v.trim_end().to_owned());
    }
    let get = |k: &str| kv.get(k).cloned().unwrap_or_default();
    Ok(Info {
        name: get("host"),
        map: get("map"),
        dir: get("gamedir"),
        game: String::new(),
        players: get("numcl").parse().unwrap_or(0),
        max_players: get("maxcl").parse().unwrap_or(0),
    })
}

/// A2S_PLAYER: (name, score) for each connected player, bots included.
pub fn players(host: &str, port: u16) -> std::io::Result<Vec<(String, i32)>> {
    let s = UdpSocket::bind("0.0.0.0:0")?;
    s.set_read_timeout(Some(Duration::from_millis(1500)))?;
    let mut buf = [0u8; 1400];
    s.send_to(b"\xff\xff\xff\xffU\xff\xff\xff\xff", (host, port))?;
    let mut n = s.recv(&mut buf)?;
    if n >= 9 && buf[4] == b'A' {
        let mut q = b"\xff\xff\xff\xffU".to_vec();
        q.extend_from_slice(&buf[5..9]);
        s.send_to(&q, (host, port))?;
        n = s.recv(&mut buf)?;
    }
    let d = &buf[..n];
    if d.len() < 6 || d[4] != b'D' {
        return Err(std::io::Error::other("bad A2S_PLAYER reply"));
    }
    let mut out = Vec::new();
    let mut p = 6;
    for _ in 0..d[5] {
        p += 1; // index
        let end = d[p..].iter().position(|&b| b == 0).map(|e| e + p).ok_or_else(|| std::io::Error::other("truncated"))?;
        let name = String::from_utf8_lossy(&d[p..end]).into_owned();
        p = end + 1;
        let score = i32::from_le_bytes(d.get(p..p + 4).ok_or_else(|| std::io::Error::other("truncated"))?.try_into().unwrap());
        p += 8; // score + duration
        out.push((name, score));
    }
    Ok(out)
}

/// Runs `command` on the server by rcon and returns what it printed. FWGS takes `rcon <password>
/// <command>` as is (`SV_RemoteCommand`); the GoldSrc engines (HLDS, ReHLDS) want a `challenge rcon`
/// first and then `rcon <challenge> "<password>" <command>`. Each call runs one command (the
/// engines don't split the text at `;`). The reply can be several packets; an empty string when
/// the command printed nothing.
pub fn rcon(host: &str, port: u16, goldsrc: bool, password: &str, command: &str) -> std::io::Result<String> {
    let s = UdpSocket::bind("0.0.0.0:0")?;
    s.set_read_timeout(Some(Duration::from_millis(2000)))?;
    let mut buf = [0u8; 2048];
    let request = if goldsrc {
        s.send_to(b"\xff\xff\xff\xffchallenge rcon\n", (host, port))?;
        let n = s.recv(&mut buf)?;
        let text = String::from_utf8_lossy(&buf[4.min(n)..n]).into_owned();
        let challenge = text
            .strip_prefix("challenge rcon ")
            .map(|c| c.trim_matches(|ch: char| ch.is_whitespace() || ch == '\0').to_owned())
            .ok_or_else(|| std::io::Error::other(format!("bad rcon challenge reply {text:?}")))?;
        format!("rcon {challenge} \"{password}\" {command}\n")
    } else {
        format!("rcon {password} {command}\n")
    };
    let mut packet = b"\xff\xff\xff\xff".to_vec();
    packet.extend_from_slice(request.as_bytes());
    s.send_to(&packet, (host, port))?;
    let mut out = String::new();
    s.set_read_timeout(Some(Duration::from_millis(700)))?;
    while let Ok(n) = s.recv(&mut buf) {
        out.push_str(&rcon_text(&buf[..n]));
        s.set_read_timeout(Some(Duration::from_millis(300)))?;
    }
    Ok(out)
}

/// The text of an rcon reply packet: `l<text>` (GoldSrc's `A2C_PRINT`) or `print\n<text>` (FWGS).
fn rcon_text(d: &[u8]) -> String {
    let body = d.strip_prefix(b"\xff\xff\xff\xff").unwrap_or(d);
    let body = body.strip_prefix(b"print\n").or_else(|| body.strip_prefix(b"l")).unwrap_or(body);
    let end = body.iter().position(|&b| b == 0).unwrap_or(body.len());
    String::from_utf8_lossy(&body[..end]).replace('\r', "")
}

#[cfg(test)]
mod tests {
    #[test]
    fn rcon_replies() {
        assert_eq!(super::rcon_text(b"\xff\xff\xff\xfflhello\n\0"), "hello\n");
        assert_eq!(super::rcon_text(b"\xff\xff\xff\xffprint\nhello\n"), "hello\n");
    }
}
