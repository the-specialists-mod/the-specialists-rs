//! Text helpers: `titles.txt` localisation (`CHudTextMessage::LocaliseTextString` 100348d0 /
//! `LookupString` 100349c0), a small printf for the TS format strings, and `DrawHudString`
//! (10021ce0) / `DrawHudStringLen` (10021ca0) on `pfnDrawCharacter`.

use goldsrc::client::engine;
use goldsrc::ext::hud::Screen;

/// Looks up a `titles.txt` key without the `#`. Separate so tests can supply a table.
pub trait Titles {
    fn get(&self, key: &str) -> Option<String>;
    /// The whole `client_textmessage_t` (`pfnTextMessageGet`), for `CHudMessage`.
    fn message(&self, _key: &[u8]) -> Option<super::message::TitleMsg> {
        None
    }
}

/// Engine titles (`pfnTextMessageGet`).
pub struct EngineTitles;

impl Titles for EngineTitles {
    fn get(&self, key: &str) -> Option<String> {
        let m = engine().text_message(key)?;
        if m.pMessage.is_null() {
            return None;
        }
        Some(m.message().to_bytes().iter().map(|&b| b as char).collect())
    }
    fn message(&self, key: &[u8]) -> Option<super::message::TitleMsg> {
        let m = engine().text_message(&super::saytext::string(key))?;
        Some(super::message::TitleMsg {
            effect: m.effect,
            color1: [m.r1, m.g1, m.b1, m.a1],
            color2: [m.r2, m.g2, m.b2, m.a2],
            x: m.x,
            y: m.y,
            fadein: m.fadein,
            fadeout: m.fadeout,
            holdtime: m.holdtime,
            fxtime: m.fxtime,
            // Guarded: a null `pMessage` (TS would dereference it) reads as empty.
            text: m.message().to_bytes().to_vec(),
        })
    }
}

/// `LocaliseTextString` (`0x100348d0`) into a `size`-byte buffer, as `CHudStatusBar` uses it:
/// each step copies one byte, or replaces a `#word` (word bytes `A`..`z` and digits) with its
/// titles.txt text, and uses up one byte of `size` however much it wrote; when the source ends
/// the buffer is cut at `size - steps - 1`. Guarded: writes past `size` (a long titles text) and
/// the `[-1]` terminator TS writes when `size` steps run out are dropped.
pub fn localise_text_string(t: &dyn Titles, src: &[u8], size: usize) -> Vec<u8> {
    let mut dst = vec![0u8; size];
    let mut put = |i: usize, b: u8| {
        if i < size {
            dst[i] = b;
        }
    };
    let (mut s, mut d, mut budget) = (0, 0, size);
    if src.first().is_some_and(|&b| b != 0) {
        loop {
            if budget == 0 {
                break;
            }
            if src[s] == b'#' {
                let end = s + 1 + src[s + 1..].iter().take_while(|&&c| (b'A'..=b'z').contains(&c) || c.is_ascii_digit()).count();
                let word = &src[s + 1..end];
                // A missing entry, or one with a null `pMessage`, copies the `#` alone.
                match t.get(&super::saytext::string(word)).map(|w| super::saytext::bytes(&w)) {
                    Some(text) => {
                        for &b in &text {
                            put(d, b);
                            d += 1;
                        }
                        put(d, 0);
                        s = end;
                    }
                    None => {
                        put(d, src[s]);
                        d += 1;
                        s += 1;
                    }
                }
            } else {
                put(d, src[s]);
                d += 1;
                s += 1;
                put(d, 0);
            }
            if s >= src.len() || src[s] == 0 {
                if let Some(i) = budget.checked_sub(2) {
                    put(i, 0);
                }
                break;
            }
            budget -= 1;
        }
    } else if let Some(i) = size.checked_sub(1) {
        put(i, 0);
    }
    let len = dst.iter().position(|&b| b == 0).unwrap_or(size);
    dst.truncate(len);
    dst
}

/// SDK `LocaliseTextString`: every `#word` (word = `[A-Za-z0-9_]`) that names a titles.txt
/// entry is replaced by its text; unknown words are left as written.
pub fn localise_with(t: &dyn Titles, msg: &str) -> String {
    let mut out = String::with_capacity(msg.len());
    let chars: Vec<char> = msg.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '#' {
            let mut j = i + 1;
            while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') {
                j += 1;
            }
            let word: String = chars[i + 1..j].iter().collect();
            if !word.is_empty()
                && let Some(text) = t.get(&word)
            {
                out.push_str(&text);
                i = j;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

pub fn localise(msg: &str) -> String {
    if !msg.contains('#') {
        return msg.to_owned();
    }
    localise_with(&EngineTitles, msg)
}

/// SDK `LookupString`: only a whole-string `#key` is looked up.
pub fn lookup(msg: &str) -> String {
    match msg.strip_prefix('#') {
        Some(key) => EngineTitles.get(key).unwrap_or_else(|| msg.to_owned()),
        None => msg.to_owned(),
    }
}

/// Argument for [`format`].
pub enum Arg<'a> {
    I(i64),
    S(&'a str),
}

/// Minimal `sprintf` for the TS titles strings: `%i %d %s %c %%`, with optional width digits.
pub fn format(fmt: &str, args: &[Arg]) -> String {
    let mut out = String::new();
    let mut it = fmt.chars().peekable();
    let mut next = 0;
    while let Some(c) = it.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let mut width = String::new();
        while let Some(d) = it.peek().copied().filter(|d| d.is_ascii_digit() || *d == '-') {
            width.push(d);
            it.next();
        }
        let Some(conv) = it.next() else { break };
        let s = match conv {
            '%' => "%".to_owned(),
            'i' | 'd' | 'u' => match args.get(next) {
                Some(Arg::I(v)) => v.to_string(),
                Some(Arg::S(s)) => s.to_string(),
                None => String::new(),
            },
            's' => match args.get(next) {
                Some(Arg::S(s)) => s.to_string(),
                Some(Arg::I(v)) => v.to_string(),
                None => String::new(),
            },
            'c' => match args.get(next) {
                Some(Arg::I(v)) => char::from_u32(*v as u32).map(String::from).unwrap_or_default(),
                Some(Arg::S(s)) => s.chars().next().map(String::from).unwrap_or_default(),
                None => String::new(),
            },
            other => {
                out.push('%');
                out.push_str(&width);
                out.push(other);
                continue;
            }
        };
        if conv != '%' {
            next += 1;
        }
        let w: i32 = width.trim_start_matches('-').parse().unwrap_or(0);
        let pad = (w as usize).saturating_sub(s.chars().count());
        if width.starts_with('-') {
            out.push_str(&s);
            out.extend(std::iter::repeat_n(' ', pad));
        } else {
            out.extend(std::iter::repeat_n(' ', pad));
            out.push_str(&s);
        }
    }
    out
}

/// Width of `text` in the HUD font (`DrawHudStringLen`), up to the first newline.
pub fn string_len(screen: &Screen, text: &str) -> i32 {
    text.chars().take_while(|&c| c != '\n').map(|c| screen.char_widths[(c as u32 & 0xff) as usize] as i32).sum()
}

/// `DrawHudString`: one `pfnDrawCharacter` per byte, stopping at `\n` or `max_x`. Returns the x
/// after the last character.
pub fn draw_string(screen: &Screen, x: i32, y: i32, max_x: i32, text: &str, rgb: [i32; 3]) -> i32 {
    let e = engine();
    let mut x = x;
    for c in text.chars() {
        if c == '\n' {
            break;
        }
        let ch = (c as u32 & 0xff) as u8;
        let w = screen.char_widths[ch as usize] as i32;
        if x + w > max_x {
            break;
        }
        e.draw_character(x, y, ch, rgb);
        x += w;
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    struct T;
    impl Titles for T {
        fn get(&self, key: &str) -> Option<String> {
            match key {
                "Buy_Buy" => Some("Buy".into()),
                "Chg_Join" => Some("Join".into()),
                "Buy_FreeSlots" => Some("Free Slots : %i".into()),
                _ => None,
            }
        }
    }

    #[test]
    fn localise_tokens_anywhere() {
        assert_eq!(localise_with(&T, "1 #Buy_Buy"), "1 Buy");
        assert_eq!(localise_with(&T, "1 #Chg_Join Specialists"), "1 Join Specialists");
        // A string without a `#` stays raw.
        assert_eq!(localise_with(&T, "Buy_SMGs"), "Buy_SMGs");
        assert_eq!(localise_with(&T, "#Nope here"), "#Nope here");
    }

    /// `LocaliseTextString` (`0x100348d0`) with TS's step budget.
    #[test]
    fn localise_text_string_budget() {
        assert_eq!(localise_text_string(&T, b"1 #Buy_Buy x", 128), b"1 Buy x");
        assert_eq!(localise_text_string(&T, b"#Nope here", 128), b"#Nope here", "unknown: the # is copied alone");
        // Word bytes run `A`..`z`, so `[` belongs to the word and the lookup misses.
        assert_eq!(localise_text_string(&T, b"#Buy_Buy[", 128), b"#Buy_Buy[");
        assert_eq!(localise_text_string(&T, b"#Buy_Buy.", 128), b"Buy.");
        assert_eq!(localise_text_string(&T, b"", 128), b"");
        // Five steps ("a", "#Chg_Join", ".", "z", "z") in 9 bytes: cut at 9 - 5 - 1 = 3.
        assert_eq!(localise_text_string(&T, b"a#Chg_Join.zz", 9), b"aJo");
    }

    #[test]
    fn printf_subset() {
        let f = localise_with(&T, "#Buy_FreeSlots");
        assert_eq!(format(&f, &[Arg::I(42)]), "Free Slots : 42");
        assert_eq!(format("%i_%i ", &[Arg::I(5), Arg::I(2)]), "5_2 ");
        assert_eq!(format("%c %s", &[Arg::I('3' as i64), Arg::S("AK47")]), "3 AK47");
        assert_eq!(format("%3d|%-3d|%%", &[Arg::I(7), Arg::I(8)]), "  7|8  |%");
    }
}
