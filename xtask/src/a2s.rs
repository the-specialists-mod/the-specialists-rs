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

pub fn info(host: &str, port: u16) -> std::io::Result<Info> {
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
