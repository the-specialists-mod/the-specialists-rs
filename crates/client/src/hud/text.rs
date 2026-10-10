//! Text helpers: `titles.txt` localisation (`CHudTextMessage::LocaliseTextString` 100348d0 /
//! `LookupString` 100349c0), a small printf for the TS format strings, and `DrawHudString`
//! (10021ce0) / `DrawHudStringLen` (10021ca0) on `pfnDrawCharacter`.

use goldsrc::client::engine;
use goldsrc::ext::hud::Screen;

/// Looks up a `titles.txt` key without the `#`. Separate so tests can supply a table.
pub trait Titles {
    fn get(&self, key: &str) -> Option<String>;
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
            if !word.is_empty() {
                if let Some(text) = t.get(&word) {
                    out.push_str(&text);
                    i = j;
                    continue;
                }
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
        // TS's missing-# bug: "Buy_SMGs" stays raw (we keep the string, the menu adds the #).
        assert_eq!(localise_with(&T, "Buy_SMGs"), "Buy_SMGs");
        assert_eq!(localise_with(&T, "#Nope here"), "#Nope here");
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
