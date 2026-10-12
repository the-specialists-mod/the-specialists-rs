//! BSP entity lumps: the map's entity text and its keyvalue blocks. The client builds its map
//! effects and spectator cameras from them; the server re-spawns the map's entities from them
//! when a gameplay switch resets the game (`ts_server::gameplay`).

/// Splits a BSP entity lump into keyvalue blocks (`COM_ParseFile` rules: quoted strings,
/// `{`/`}`, `//` comments). Malformed text stops parsing, keeping complete blocks.
pub fn parse_entities(text: &str) -> Vec<Vec<(String, String)>> {
    let mut toks = Tokens { s: text.as_bytes(), i: 0 };
    let mut out = Vec::new();
    while let Some(t) = toks.next() {
        if t != "{" {
            break; // "found %s when expecting {"
        }
        let mut kv = Vec::new();
        loop {
            let Some(k) = toks.next() else { return out };
            if k == "}" {
                break;
            }
            let Some(val) = toks.next() else { return out };
            if val == "}" {
                break;
            }
            kv.push((k, val));
        }
        out.push(kv);
    }
    out
}

struct Tokens<'a> {
    s: &'a [u8],
    i: usize,
}

impl Tokens<'_> {
    fn next(&mut self) -> Option<String> {
        loop {
            while self.i < self.s.len() && self.s[self.i] <= b' ' {
                self.i += 1;
            }
            if self.i + 1 < self.s.len() && self.s[self.i] == b'/' && self.s[self.i + 1] == b'/' {
                while self.i < self.s.len() && self.s[self.i] != b'\n' {
                    self.i += 1;
                }
                continue;
            }
            break;
        }
        if self.i >= self.s.len() || self.s[self.i] == 0 {
            return None;
        }
        let c = self.s[self.i];
        if c == b'"' {
            self.i += 1;
            let start = self.i;
            while self.i < self.s.len() && self.s[self.i] != b'"' {
                self.i += 1;
            }
            let t = String::from_utf8_lossy(&self.s[start..self.i]).into_owned();
            self.i += 1;
            return Some(t);
        }
        if c == b'{' || c == b'}' {
            self.i += 1;
            return Some((c as char).to_string());
        }
        let start = self.i;
        while self.i < self.s.len() && self.s[self.i] > b' ' && !matches!(self.s[self.i], b'{' | b'}' | b'"') {
            self.i += 1;
        }
        Some(String::from_utf8_lossy(&self.s[start..self.i]).into_owned())
    }
}

/// Entity lump of a BSP file (`LoadEntityLump` `0x1003a860`): version 29 or 30, lump 0.
pub fn entity_lump(bsp: &[u8]) -> Option<&str> {
    let rd = |o: usize| bsp.get(o..o + 4).map(|b| i32::from_le_bytes(b.try_into().unwrap()));
    let ver = rd(0)?;
    if ver != 29 && ver != 30 {
        return None;
    }
    let (ofs, len) = (rd(4)? as usize, rd(8)? as usize);
    let lump = bsp.get(ofs..ofs.checked_add(len)?)?;
    let lump = lump.split(|&b| b == 0).next().unwrap_or(lump);
    std::str::from_utf8(lump).ok().or_else(|| std::str::from_utf8(&lump[..lump.iter().position(|&b| b >= 0x80).unwrap_or(0)]).ok())
}
