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
    /// The entry's `effect` (0 if none).
    fn effect(&self, _key: &str) -> i32 {
        0
    }
}

/// Engine titles (`pfnTextMessageGet`).
pub struct EngineTitles;

impl Titles for EngineTitles {
    fn effect(&self, key: &str) -> i32 {
        engine().text_message(key).map_or(0, |m| m.effect)
    }
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

/// `LocaliseTextString` (`0x100348d0`) into `dst` (its `size` is `dst.len()`), as TS writes it:
/// each step copies one byte, or replaces a `#word` (word bytes `A`..`z`, which takes in
/// `[\]^_` and the backtick, and digits) with its titles.txt text, and uses up one byte of `size`
/// however much it wrote; when the source ends the buffer is cut at `size - steps - 1`. A `#` with
/// no entry is copied alone and, unlike the other steps, isn't followed by a terminator, so a
/// source ending in one shows whatever the buffer held after it. Returns the length up to the
/// first NUL. Guarded: writes past `size` (a long titles text) and the `[-1]` terminator TS writes
/// when `size` steps run out are dropped.
pub fn localise_text_string_in(t: &dyn Titles, src: &[u8], dst: &mut [u8]) -> usize {
    let size = dst.len();
    let mut put = |i: usize, b: u8| {
        if i < size {
            dst[i] = b;
        }
    };
    let at = |i: usize| src.get(i).copied().unwrap_or(0);
    let (mut s, mut d, mut budget) = (0, 0, size);
    if at(0) != 0 {
        while budget > 0 {
            if at(s) == b'#' {
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
                        put(d, b'#');
                        d += 1;
                        s += 1;
                    }
                }
            } else {
                put(d, at(s));
                d += 1;
                s += 1;
                put(d, 0);
            }
            if at(s) == 0 {
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
    dst.iter().position(|&b| b == 0).unwrap_or(size)
}

/// `LocaliseTextString` into a fresh `size`-byte buffer (TS's callers pass stack buffers; the
/// port starts them zeroed).
pub fn localise_text_string(t: &dyn Titles, src: &[u8], size: usize) -> Vec<u8> {
    let mut dst = vec![0u8; size];
    let n = localise_text_string_in(t, src, &mut dst);
    dst.truncate(n);
    dst
}

thread_local! {
    /// `BufferedLocaliseTextString`'s static buffer (`0x100eb274`, 0x400 bytes), shared by all
    /// its callers (TS's client runs on one thread).
    static BUFFER: std::cell::RefCell<[u8; 0x400]> = const { std::cell::RefCell::new([0; 0x400]) };
}

/// `BufferedLocaliseTextString` (`0x100349a0`): `LocaliseTextString` into the one 0x400-byte
/// static buffer.
pub fn buffered_localise(t: &dyn Titles, src: &[u8]) -> Vec<u8> {
    BUFFER.with_borrow_mut(|b| {
        let n = localise_text_string_in(t, src, b);
        b[..n].to_vec()
    })
}

/// [`buffered_localise`] on the port's Latin-1 strings (one char per byte). TS localises this way
/// wherever it localises a whole text: `AddTSMessage` (`TSMessage`), `ShowMenu`, the buy menu and
/// the HUD labels.
pub fn localise_with(t: &dyn Titles, msg: &str) -> String {
    super::saytext::string(&buffered_localise(t, &super::saytext::bytes(msg)))
}

pub fn localise(msg: &str) -> String {
    localise_with(&EngineTitles, msg)
}

/// `CHudTextMessage::LookupString` (`0x100349c0`): only a whole-string `#key` is looked up (the
/// rest of the string is the key). A found entry with a negative `effect` sets `dest` to its
/// negation.
pub fn lookup_string(t: &dyn Titles, msg: &[u8], dest: Option<&mut i32>) -> Vec<u8> {
    if let Some(key) = msg.strip_prefix(b"#") {
        let key = super::saytext::string(key);
        if let Some(text) = t.get(&key) {
            if let Some(d) = dest {
                let effect = t.effect(&key);
                if effect < 0 {
                    *d = -effect;
                }
            }
            return super::saytext::bytes(&text);
        }
    }
    msg.to_vec()
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

/// The widest field [`format`] pads to (the size of `BufferedLocaliseTextString`'s buffer).
const MAX_FORMAT_WIDTH: usize = 0x400;

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
        // Guarded: the format string can come from the server (`TextMsg`), and a width like
        // `%999999999s` would pad by gigabytes; TS's `sprintf` would overrun its 128-byte buffer.
        let digits = width.trim_start_matches('-');
        let w = match digits.parse::<usize>() {
            Ok(w) => w,
            // All digits but too many for a usize: as wide as it gets.
            Err(_) if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) => usize::MAX,
            Err(_) => 0,
        }
        .min(MAX_FORMAT_WIDTH);
        let pad = w.saturating_sub(s.chars().count());
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

    /// TS's rules in [`localise_with`] (`BufferedLocaliseTextString`): the word takes `A`..`z`
    /// (so `#Buy_Buy^` misses where the SDK's `[A-Za-z0-9_]` would stop), a 0x400-byte buffer
    /// shared by every call, and a trailing unknown `#` left unterminated over the last text.
    #[test]
    fn buffered_localise_rules() {
        assert_eq!(localise_with(&T, "#Buy_Buy^x"), "#Buy_Buy^x");
        assert_eq!(localise_with(&T, "#Buy_Buy-x"), "Buy-x");
        assert_eq!(localise_with(&T, "#Buy_Buy"), "Buy");
        assert_eq!(localise_with(&T, "abcdef"), "abcdef");
        assert_eq!(localise_with(&T, "x#"), "x#cdef", "the old text shows after an unknown trailing #");
        // 600 plain bytes: cut at 0x400 - 600 - 1.
        assert_eq!(localise_with(&T, &"a".repeat(600)).len(), 0x400 - 600 - 1);
    }

    /// `LookupString` (`0x100349c0`): a whole-string key only.
    #[test]
    fn lookup_string_whole_keys() {
        assert_eq!(lookup_string(&T, b"#Buy_Buy", None), b"Buy");
        assert_eq!(lookup_string(&T, b"#Buy_Buy x", None), b"#Buy_Buy x");
        assert_eq!(lookup_string(&T, b"x #Buy_Buy", None), b"x #Buy_Buy");
        let mut dest = 3;
        assert_eq!(lookup_string(&T, b"#Nope", Some(&mut dest)), b"#Nope");
        assert_eq!(dest, 3);
    }

    #[test]
    fn printf_subset() {
        let f = localise_with(&T, "#Buy_FreeSlots");
        assert_eq!(format(&f, &[Arg::I(42)]), "Free Slots : 42");
        assert_eq!(format("%i_%i ", &[Arg::I(5), Arg::I(2)]), "5_2 ");
        assert_eq!(format("%c %s", &[Arg::I('3' as i64), Arg::S("AK47")]), "3 AK47");
        assert_eq!(format("%3d|%-3d|%%", &[Arg::I(7), Arg::I(8)]), "  7|8  |%");
    }

    /// Server-supplied format strings: huge widths are capped, odd conversions and a trailing
    /// `%` are harmless, and localising any bytes into any buffer size stays in bounds.
    #[test]
    fn hostile_formats_and_sources() {
        assert_eq!(format("%999999999s", &[Arg::S("x")]).len(), MAX_FORMAT_WIDTH);
        assert_eq!(format("%99999999999999999999999d", &[Arg::I(1)]).len(), MAX_FORMAT_WIDTH);
        assert_eq!(format("%--5s|%5-3d|%", &[Arg::S("a"), Arg::I(2)]), "a    |2|");
        assert_eq!(format("%c%c", &[Arg::I(-1), Arg::I(0x11_0000)]), "");
        let mut seed = 0x1234_5678_9abc_def1u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..300 {
            let n = (next() % 40) as usize;
            let src: Vec<u8> = (0..n).map(|_| [b'#', b'B', b'%', 0, 0xff, b'u'][(next() % 6) as usize]).collect();
            let size = (next() % 12) as usize;
            assert!(localise_text_string(&T, &src, size).len() <= size);
            let _ = lookup_string(&T, &src, Some(&mut 0));
            let _ = buffered_localise(&T, &src);
            let s = super::super::saytext::string(&src);
            let _ = format(&s, &[Arg::S(&s), Arg::I(i64::MIN)]);
            assert!(string_len(&Screen::default(), &s) >= 0);
        }
    }
}
