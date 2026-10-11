//! `CHudStatusBar` (SDK statusbar.cpp as TS 3.0 ships it, element `+0x330` = `0x100cfc50`): two
//! lines of `StatusText` with `%p`/`%i` substitutions from the `StatusValue` slots, drawn above
//! the bottom of the screen (`Draw` `0x1002f6f0`). No TS server sends either message; TS keeps
//! the element, so the port does too.

use super::draw::{Canvas, ftol};
use super::saytext;
use super::text::{EngineTitles, Titles, localise_text_string};
use goldsrc::client::engine;
use goldsrc::ext::hud::Screen;
use shared::protocol::hud::Reader;

/// `MAX_STATUSTEXT_LENGTH`.
pub const MAX_STATUSTEXT_LENGTH: usize = 128;
/// `MAX_STATUSBAR_VALUES`.
pub const MAX_STATUSBAR_VALUES: usize = 8;
/// `MAX_STATUSBAR_LINES` (TS has two).
pub const MAX_STATUSBAR_LINES: usize = 2;
/// `g_ColorYellow` (`0x1009359c`), the lines' default colour.
const YELLOW: [f32; 3] = [1.0, 0.7, 0.0];
/// `MAX_PLAYER_NAME_LENGTH`: `szRepString`'s size.
const NAME_LENGTH: usize = 32;

/// A text colour; `None` is a null colour pointer (no `DrawSetTextColor`).
pub type Color = Option<[f32; 3]>;
/// `GetPlayerInfo` + `GetClientColor` for a player number: the name and its colour, or `None`
/// for a null name.
pub type PlayerLookup<'a> = dyn FnMut(i32) -> Option<(Vec<u8>, Color)> + 'a;

#[derive(Debug, Clone)]
pub struct StatusBar {
    /// `m_szStatusText` (`+0x14`, 2 × 128): the raw lines (at most 127 bytes).
    pub text: [Vec<u8>; MAX_STATUSBAR_LINES],
    /// `m_szStatusBar` (`+0x114`): the parsed lines that are drawn.
    pub bar: [Vec<u8>; MAX_STATUSBAR_LINES],
    /// `m_iStatusValues` (`+0x214`); slot 0 is always 1.
    pub values: [i32; MAX_STATUSBAR_VALUES],
    /// `m_bReparseString` (`+0x234`).
    pub reparse: bool,
    /// `m_pflNameColors` (`+0x238`): `None` is a null pointer (no `DrawSetTextColor`).
    pub name_colors: [Option<[f32; 3]>; MAX_STATUSBAR_LINES],
    /// `HUD_ACTIVE`.
    pub active: bool,
}

impl Default for StatusBar {
    /// `Init` (`0x1002f3c0`) ends with `Reset`.
    fn default() -> Self {
        let mut s = StatusBar {
            text: Default::default(),
            bar: Default::default(),
            values: [0; MAX_STATUSBAR_VALUES],
            reparse: false,
            name_colors: [None; MAX_STATUSBAR_LINES],
            active: false,
        };
        s.reset();
        s
    }
}

/// MSVC `atoi` (`0x1006c042`): leading white space, a sign, then digits (wrapping).
pub fn atoi(s: &[u8]) -> i32 {
    let mut i = 0;
    while s.get(i).is_some_and(|&c| c == b' ' || (9..=13).contains(&c)) {
        i += 1;
    }
    let neg = s.get(i) == Some(&b'-');
    if matches!(s.get(i), Some(b'-' | b'+')) {
        i += 1;
    }
    let mut v: i32 = 0;
    while let Some(&c) = s.get(i).filter(|c| c.is_ascii_digit()) {
        v = v.wrapping_mul(10).wrapping_add((c - b'0') as i32);
        i += 1;
    }
    if neg { v.wrapping_neg() } else { v }
}

/// `READ_BYTE`/`READ_SHORT` as the SDK's `parsemsg.cpp` gives them: -1 past the end.
fn read_byte(r: &mut Reader) -> i32 {
    let v = r.byte();
    if r.bad { -1 } else { v as i32 }
}

impl StatusBar {
    /// `Reset` (`0x1002f430`): off, both lines emptied, the values zeroed (slot 0 = 1) and the
    /// colours back to yellow. The parsed lines and the reparse flag are left.
    pub fn reset(&mut self) {
        self.active = false;
        self.text = Default::default();
        self.values = [0; MAX_STATUSBAR_VALUES];
        self.values[0] = 1;
        self.name_colors = [Some(YELLOW); MAX_STATUSBAR_LINES];
    }

    /// `MsgFunc_StatusText` (`0x1002f850`): byte line (0 or 1), string text (cut to 127 bytes).
    /// The element turns on whatever the text (TS's `m_szStatusText[0] == 0` test compares the
    /// array's address, so it never turns it off).
    pub fn msg_status_text(&mut self, buf: &[u8]) {
        let mut r = Reader::new(buf);
        let line = read_byte(&mut r);
        if !(0..MAX_STATUSBAR_LINES as i32).contains(&line) {
            return;
        }
        let text = saytext::bytes(&r.string());
        self.text[line as usize] = text[..text.len().min(MAX_STATUSTEXT_LENGTH - 1)].to_vec();
        self.active = true;
        self.reparse = true;
    }

    /// `MsgFunc_StatusValue` (`0x1002f8d0`): byte index (1..7), short value.
    pub fn msg_status_value(&mut self, buf: &[u8]) {
        let mut r = Reader::new(buf);
        let index = read_byte(&mut r);
        if !(1..MAX_STATUSBAR_VALUES as i32).contains(&index) {
            return;
        }
        let v = r.short();
        self.values[index as usize] = if r.bad { -1 } else { v as i32 };
        self.reparse = true;
    }

    /// `ParseStatusString` (`0x1002f480`): the localised line, keeping each `\n`-separated part
    /// whose leading number names a non-zero value, with `%pN` (that value's player name, and
    /// its `GetClientColor` for the line), `%iN` (the value) and `%%` substituted. `player` is
    /// `GetPlayerInfo`'s name and the colour, `None` for a null name (shown as `******`).
    ///
    /// Guarded: TS copies plain characters without checking the 128-byte output (running into
    /// the next line's buffer, or past the last into the values), reads past the source when it
    /// ends in `%`, and reads past a 32-byte name; the port drops the writes and stops the reads.
    pub fn parse(&mut self, line: usize, titles: &dyn Titles, player: &mut PlayerLookup) {
        let mut src = [0u8; MAX_STATUSTEXT_LENGTH];
        let localised = localise_text_string(titles, &self.text[line], MAX_STATUSTEXT_LENGTH);
        src[..localised.len()].copy_from_slice(&localised);
        let at = |i: usize| src.get(i).copied().unwrap_or(0);
        let mut out = [0u8; MAX_STATUSTEXT_LENGTH];
        let mut d = 0usize;
        let mut write = |d: &mut usize, b: u8| {
            if let Some(o) = out.get_mut(*d) {
                *o = b;
            }
            *d += 1;
        };
        let mut s = 0usize;
        while at(s) != 0 {
            while at(s) == b'\n' {
                s += 1;
            }
            if s >= MAX_STATUSTEXT_LENGTH || d >= MAX_STATUSTEXT_LENGTH {
                break;
            }
            let index = atoi(&src[s..]);
            if !(0..MAX_STATUSBAR_VALUES as i32).contains(&index) || self.values[index as usize] == 0 {
                while at(s) != 0 && at(s) != b'\n' {
                    s += 1;
                }
                continue;
            }
            while at(s).is_ascii_digit() {
                s += 1;
            }
            if at(s) == b'\n' || at(s) == 0 {
                continue;
            }
            loop {
                let c = at(s);
                if c == 0 {
                    break;
                }
                if c != b'%' {
                    write(&mut d, c);
                    s += 1;
                } else if at(s + 1) == b'%' {
                    write(&mut d, b'%');
                    s += 2;
                } else {
                    let valtype = at(s + 1);
                    s += 2;
                    let index = atoi(src.get(s..).unwrap_or(&[]));
                    while at(s).is_ascii_digit() {
                        s += 1;
                    }
                    if (0..MAX_STATUSBAR_VALUES as i32).contains(&index) {
                        let value = self.values[index as usize];
                        let rep = match valtype {
                            b'p' => match player(value) {
                                Some((name, color)) => {
                                    self.name_colors[line] = color;
                                    name[..name.len().min(NAME_LENGTH)].to_vec()
                                }
                                None => b"******".to_vec(),
                            },
                            b'i' => value.to_string().into_bytes(),
                            _ => Vec::new(),
                        };
                        for b in rep {
                            if d >= MAX_STATUSTEXT_LENGTH {
                                break;
                            }
                            write(&mut d, b);
                        }
                    }
                }
                if at(s) == b'\n' {
                    break;
                }
            }
        }
        let len = out.iter().position(|&b| b == 0).unwrap_or(MAX_STATUSTEXT_LENGTH);
        self.bar[line] = out[..len].to_vec();
    }

    /// `Draw` (`0x1002f6f0`), while `HUD_ACTIVE`: reparse if a message changed anything (each
    /// line's colour back to yellow first), then line 0 at x 5 and `height - 55` (`- 45` below
    /// 480 px) and line 1 a text height above it, or with `hud_centerid` set, centred at half
    /// the height plus `hud_centerid` text heights.
    pub fn draw(&mut self, s: &Screen, centerid: f32, titles: &dyn Titles, player: &mut PlayerLookup, c: &mut dyn Canvas) {
        if self.reparse {
            for i in 0..MAX_STATUSBAR_LINES {
                self.name_colors[i] = Some(YELLOW);
                self.parse(i, titles, player);
            }
            self.reparse = false;
        }
        for i in 0..MAX_STATUSBAR_LINES {
            let (w, h) = c.console_size(&self.bar[i]);
            // C int arithmetic on the engine's sizes.
            let base = if s.height < 480 { s.height.wrapping_sub(45) } else { s.height.wrapping_sub(55) };
            let mut y = base.wrapping_sub(h.wrapping_mul(i as i32));
            let mut x = 5;
            if i == 1 && centerid != 0.0 && !centerid.is_nan() {
                x = s.width.wrapping_sub(w).max(2) / 2;
                y = ftol(h as f64 * centerid as f64 + (s.height / 2) as f64);
            }
            c.console_string(x, y, self.name_colors[i], &self.bar[i]);
        }
    }
}

/// Hooks `StatusText`/`StatusValue` as `CHudStatusBar::Init` does. No TS server registers them.
pub fn hook() {
    let e = engine();
    e.hook_user_msg(c"StatusText", |b| super::with(|h| h.status_bar.msg_status_text(b)));
    e.hook_user_msg(c"StatusValue", |b| super::with(|h| h.status_bar.msg_status_value(b)));
}

/// The status bar's turn in `CHud::Redraw`.
pub fn draw(dc: &super::Dc) {
    let local = super::local_index();
    let centerid = super::cvars().centerid.value();
    let mut canvas = super::draw::EngineCanvas(&dc.g.sprites);
    super::with(|h| {
        if !h.status_bar.active {
            return;
        }
        let players = &h.players;
        let mut player = |v: i32| super::notice::player_name(v, 64).map(|n| (n, super::notice::client_color(players, local, v)));
        h.status_bar.draw(&dc.g.screen, centerid, &EngineTitles, &mut player, &mut canvas);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    struct T;
    impl Titles for T {
        fn get(&self, key: &str) -> Option<String> {
            (key == "Health").then(|| "Health".into())
        }
    }

    fn players(v: i32) -> Option<(Vec<u8>, Color)> {
        (v == 3).then(|| (b"Neo".to_vec(), Some([0.6, 0.8, 1.0])))
    }

    fn msg(line: u8, text: &str) -> Vec<u8> {
        let mut b = vec![line];
        b.extend_from_slice(text.as_bytes());
        b.push(0);
        b
    }

    #[test]
    fn atoi_like_msvc() {
        assert_eq!(atoi(b"12ab"), 12);
        assert_eq!(atoi(b" \t-7"), -7);
        assert_eq!(atoi(b"x1"), 0);
        assert_eq!(atoi(b""), 0);
    }

    #[test]
    fn messages_and_reset() {
        let mut s = StatusBar::default();
        assert_eq!(s.values[0], 1);
        assert!(!s.active);
        s.msg_status_text(&msg(2, "x"));
        assert!(!s.active, "line 2 is out of range");
        s.msg_status_text(&msg(1, ""));
        assert!(s.active && s.reparse, "even an empty text turns it on");
        s.msg_status_value(&[0, 5, 0]);
        assert_eq!(s.values[0], 1, "slot 0 can't be set");
        s.msg_status_value(&[7, 0xfe, 0xff]);
        assert_eq!(s.values[7], -2, "signed short");
        s.msg_status_value(&[4]);
        assert_eq!(s.values[4], -1, "READ_SHORT past the end");
        s.msg_status_text(&msg(0, &"a".repeat(200)));
        assert_eq!(s.text[0].len(), 127);
        s.reset();
        assert!(!s.active && s.text[0].is_empty() && s.values == [1, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn parse_substitutes_and_skips_parts() {
        let mut s = StatusBar::default();
        s.msg_status_text(&msg(0, "1Friend: %p1\n2 #Health %i2%%\n3never"));
        s.msg_status_value(&[1, 3, 0]);
        s.msg_status_value(&[2, 75, 0]);
        s.parse(0, &T, &mut players);
        assert_eq!(s.bar[0], b"Friend: Neo Health 75%", "part 3 has a zero value; parts join without the newline");
        assert_eq!(s.name_colors[0], Some([0.6, 0.8, 1.0]));
        // An unknown player shows six stars and leaves the colour.
        s.name_colors[0] = Some(YELLOW);
        s.msg_status_value(&[1, 9, 0]);
        s.parse(0, &T, &mut players);
        assert_eq!(s.bar[0], b"Friend: ****** Health 75%");
        assert_eq!(s.name_colors[0], Some(YELLOW));
        // Slot 0 is always 1, so "0..." parts always show; other value types are empty.
        s.msg_status_text(&msg(1, "0a%x1b%i9c"));
        s.parse(1, &T, &mut players);
        assert_eq!(s.bar[1], b"abc", "%x is empty, %i9 is out of range");
    }

    #[test]
    fn parse_caps_the_output() {
        let mut s = StatusBar::default();
        s.msg_status_text(&msg(0, "0%p1%p1%p1%p1%p1"));
        s.msg_status_value(&[1, 1, 0]);
        let mut long = |_: i32| Some((vec![b'n'; 40], None));
        s.parse(0, &T, &mut long);
        assert_eq!(s.bar[0], vec![b'n'; 128], "names cut at 32 bytes, substitutions stop at 128");
    }

    /// `LocaliseTextString`'s budget cuts a long line: 121 steps in 128 bytes leave 6.
    #[test]
    fn long_lines_are_cut_by_localise() {
        let mut s = StatusBar::default();
        s.msg_status_text(&msg(0, &format!("0{}", "x".repeat(120))));
        s.parse(0, &T, &mut players);
        assert_eq!(s.bar[0], b"xxxxx");
    }

    /// Random `StatusText`/`StatusValue` payloads, parsed and laid out on odd screens.
    #[test]
    fn hostile_messages() {
        struct Null;
        impl Canvas for Null {
            fn console_size(&self, _: &[u8]) -> (i32, i32) {
                (i32::MAX, i32::MIN)
            }
            fn console_string(&mut self, x: i32, _: i32, _: Option<[f32; 3]>, _: &[u8]) -> i32 {
                x
            }
            fn character(&mut self, _: i32, _: i32, _: u8, _: [i32; 3]) {}
            fn sprite(&mut self, _: usize, _: [i32; 3], _: i32, _: i32) {}
        }
        let mut seed = 0x5151_7272_9393_a4a4u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let mut s = StatusBar::default();
        let mut long = |v: i32| (v % 2 == 0).then(|| (vec![b'%'; 200], None));
        for i in 0..400 {
            let n = (next() % 200) as usize;
            let b: Vec<u8> = (0..n).map(|_| [b'%', b'p', b'i', b'1', b'0', b'\n', 0, 0xff, b'-'][(next() % 9) as usize]).collect();
            s.msg_status_text(&[&[(i % 3) as u8][..], &b].concat());
            s.msg_status_value(&b);
            s.msg_status_value(&[(next() % 9) as u8, next() as u8, next() as u8]);
            let screen = Screen { width: [i32::MIN, 0, 640][i % 3], height: [i32::MIN, 100, 480][i % 3], ..Screen::default() };
            s.draw(&screen, [f32::NAN, f32::INFINITY, -1e30, 2.0][i % 4], &T, &mut long, &mut Null);
            assert!(s.bar.iter().all(|l| l.len() <= MAX_STATUSTEXT_LENGTH));
        }
        assert_eq!((atoi(b"99999999999"), atoi(b"-2147483648")), (1215752191, i32::MIN), "wraps like MSVC's");
    }

    #[test]
    fn draw_positions() {
        #[derive(Default)]
        struct Rec(Vec<(i32, i32, Color, Vec<u8>)>);
        impl Canvas for Rec {
            fn console_size(&self, t: &[u8]) -> (i32, i32) {
                (t.len() as i32 * 8, 14)
            }
            fn console_string(&mut self, x: i32, y: i32, c: Option<[f32; 3]>, t: &[u8]) -> i32 {
                self.0.push((x, y, c, t.to_vec()));
                x
            }
            fn character(&mut self, _: i32, _: i32, _: u8, _: [i32; 3]) {}
            fn sprite(&mut self, _: usize, _: [i32; 3], _: i32, _: i32) {}
        }
        let screen = Screen { width: 800, height: 600, ..Default::default() };
        let mut s = StatusBar::default();
        s.msg_status_text(&msg(0, "0zero"));
        s.msg_status_text(&msg(1, "0one"));
        let mut r = Rec::default();
        s.draw(&screen, 0.0, &T, &mut players, &mut r);
        assert!(!s.reparse);
        assert_eq!(r.0[0], (5, 545, Some(YELLOW), b"zero".to_vec()));
        assert_eq!(r.0[1], (5, 531, Some(YELLOW), b"one".to_vec()));
        let mut r = Rec::default();
        s.draw(&Screen { height: 400, ..screen }, 2.0, &T, &mut players, &mut r);
        assert_eq!(r.0[0].1, 355, "below 480 px: height - 45");
        assert_eq!((r.0[1].0, r.0[1].1), ((800 - 24) / 2, 200 + 28), "hud_centerid 2");
    }
}
