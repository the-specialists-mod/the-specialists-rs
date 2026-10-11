//! `CHudSayText` (SDK saytext.cpp as TS 3.0 ships it, element `+0x588` = `0x100cfea8`): five
//! 256-byte chat lines (`0x100e5310`), wrapped to the screen width, scrolled up every
//! `hud_saytext_time` seconds, and printed to the console as they scroll off the top.
//!
//! Strings are kept as bytes (one byte per character, as the engine's string reader gives them),
//! since the wrap and the name colouring work on byte offsets.

/// `MAX_LINES`.
const MAX_LINES: usize = 5;
/// `MAX_CHARS_PER_LINE`: each line buffer is 256 bytes.
const LINE_SIZE: usize = 256;
/// `LINE_START`: the text's x.
const LINE_START: i32 = 10;
/// The deepest [`SayText::wrap`] recursion: every move takes at least a byte off a 255-byte line.
const MAX_WRAP_DEPTH: usize = LINE_SIZE;

/// The say text colour (`0x1009359c`, also team 3's): 1.0 0.7 0.0.
pub const SAY_COLOR: [f32; 3] = [1.0, 0.7, 0.0];

/// What `SayTextPrint` and the wrap need from the engine.
pub trait SayEnv {
    /// `GetConsoleStringSize` (`pfnDrawConsoleStringLen`): (width, height).
    fn string_size(&self, text: &[u8]) -> (i32, i32);
    /// `ConsolePrint`.
    fn console_print(&mut self, text: &[u8]);
}

#[derive(Debug, Clone)]
pub struct SayText {
    /// `g_szLineBuffer` (`0x100e5310`): NUL-free line contents (empty = free).
    pub lines: [Vec<u8>; MAX_LINES],
    /// `g_iNameLengths` (`0x100e5950`): the byte offset just past the speaker's name.
    pub name_len: [usize; MAX_LINES],
    /// `g_pflNameColors` (`0x100e5968`): the speaker's colour (`GetClientColor`), if any.
    pub color: [Option<[f32; 3]>; MAX_LINES],
    /// `m_flScrollTime` (`0x100e5980`).
    pub scroll_time: f32,
    /// `m_HUD_saytext_time` value cached when line 0 is written (`0x10094c64`, initially 5).
    pub saytext_time: f32,
    /// `Y_START` (`0x100e5984`).
    pub y_start: i32,
    /// `line_height` (`0x100e5988`), set by every `GetConsoleStringSize` in the wrap.
    pub line_height: i32,
    /// `HUD_ACTIVE` in the element's flags.
    pub active: bool,
}

impl Default for SayText {
    fn default() -> Self {
        SayText {
            lines: Default::default(),
            name_len: [0; MAX_LINES],
            color: [None; MAX_LINES],
            scroll_time: 0.0,
            saytext_time: 5.0,
            y_start: 0,
            line_height: 0,
            active: false,
        }
    }
}

/// Latin-1 bytes of a network string (the port decodes them one char per byte).
pub fn bytes(s: &str) -> Vec<u8> {
    s.chars().map(|c| if (c as u32) < 256 { c as u8 } else { b'?' }).collect()
}

/// The inverse of [`bytes`].
pub fn string(b: &[u8]) -> String {
    b.iter().map(|&c| c as char).collect()
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

impl SayText {
    /// `InitHUDData` (`0x1002eaa0`): clears the lines, name lengths and colours.
    pub fn clear(&mut self) {
        self.lines = Default::default();
        self.name_len = [0; MAX_LINES];
        self.color = [None; MAX_LINES];
    }

    fn first_free(&self) -> usize {
        self.lines.iter().position(|l| l.is_empty()).unwrap_or(MAX_LINES)
    }

    /// `ScrollTextUp` (`0x1002ead0`): prints line 0 to the console and moves the others up. A
    /// wrapped continuation (starting with a space) scrolls off with it, its space replaced by
    /// `\x02`. Returns the number of lines removed.
    pub fn scroll_up(&mut self, env: &mut dyn SayEnv) -> usize {
        env.console_print(&self.lines[0]);
        self.lines.rotate_left(1);
        self.lines[MAX_LINES - 1].clear();
        self.name_len.rotate_left(1);
        self.color.rotate_left(1);
        // TS shifts these two arrays by one entry from past their ends; the new last entries are
        // never drawn (the last line is empty) and are rewritten before use.
        self.name_len[MAX_LINES - 1] = 0;
        self.color[MAX_LINES - 1] = None;
        if self.lines[0].first() == Some(&b' ') {
            self.lines[0][0] = 2;
            return self.scroll_up(env) + 1;
        }
        1
    }

    /// `SayTextPrint` (`0x1002ed00`) after its mute and `AllowedToPrintText` checks:
    /// `name`/`color` are the speaker's (when the text starts with `\x02` and `client > 0`).
    /// `screen` is (`ScreenWidth`, `ScreenHeight`); `saytext_cvar` is `hud_saytext_time`, read
    /// only when the text lands on line 0. The caller plays `misc/talk.wav` afterwards.
    pub fn print(
        &mut self,
        text: &[u8],
        speaker: Option<(&[u8], Option<[f32; 3]>)>,
        now: f32,
        saytext_cvar: impl FnOnce() -> f32,
        screen: (i32, i32),
        env: &mut dyn SayEnv,
    ) {
        let mut i = self.first_free();
        if i == MAX_LINES {
            self.scroll_up(env);
            i = MAX_LINES - 1;
        }
        self.name_len[i] = 0;
        self.color[i] = None;
        if text.first() == Some(&2)
            && let Some((name, color)) = speaker
            && let Some(at) = find(text, name)
        {
            self.name_len[i] = at + name.len();
            self.color[i] = color;
        }
        // `strncpy(line, text, 255)`.
        let n = text.iter().position(|&b| b == 0).unwrap_or(text.len()).min(LINE_SIZE - 1);
        self.lines[i] = text[..n].to_vec();
        self.wrap(i, screen.0, env);
        if i == 0 {
            self.saytext_time = saytext_cvar();
            self.scroll_time = now + self.saytext_time;
        }
        self.active = true;
        let h = screen.1;
        // C int arithmetic on the engine's sizes.
        let base = if h < 480 { h.wrapping_sub(45) } else { h.wrapping_sub(60) };
        self.y_start = base.wrapping_sub(self.line_height.wrapping_mul(6));
    }

    /// `EnsureTextFitsInOneLineAndWrapIfHaveTo` (`0x1002ee90`), with its SDK quirks: a break that
    /// isn't at a space drops the rest of the text (the unsigned length check always fails), and
    /// `/(...)` colour codes are skipped when measuring.
    pub fn wrap(&mut self, line: usize, screen_width: i32, env: &mut dyn SayEnv) {
        self.wrap_at_depth(line, screen_width, env, 0);
    }

    /// [`Self::wrap`]'s recursion. Guarded: on a screen narrower than a character, a line starting
    /// with a space moves whole to the free line its last move emptied, back and forth, until TS's
    /// stack overflows; the port stops after [`MAX_WRAP_DEPTH`] moves.
    fn wrap_at_depth(&mut self, mut line: usize, screen_width: i32, env: &mut dyn SayEnv, depth: usize) {
        if depth > MAX_WRAP_DEPTH {
            return;
        }
        let max_width = screen_width.saturating_sub(40);
        let (w, h) = env.string_size(&self.lines[line]);
        self.line_height = h;
        if w.wrapping_add(LINE_START) <= max_width {
            return;
        }
        let mut length = LINE_START;
        let mut last_break: Option<usize> = None;
        let mut x = 0usize;
        let buf = self.lines[line].clone();
        while x < buf.len() {
            if buf[x] == b'/' && buf.get(x + 1) == Some(&b'(') {
                x += 2;
                while x < buf.len() && buf[x] != b')' {
                    x += 1;
                }
                if x < buf.len() {
                    x += 1;
                }
                if x >= buf.len() {
                    return;
                }
            }
            if buf[x] == b' ' && x != 0 {
                last_break = Some(x);
            }
            let (cw, ch) = env.string_size(&buf[x..x + 1]);
            self.line_height = ch;
            length = length.wrapping_add(cw);
            if length > max_width {
                // `x - 1` when no space came first; at the very first character that is one
                // byte before the line, which TS then reads as that byte. The port clamps it.
                let brk = last_break.unwrap_or(x.saturating_sub(1));
                let mut j;
                loop {
                    j = self.first_free();
                    if j < MAX_LINES {
                        break;
                    }
                    let moved = self.scroll_up(env);
                    line = line.saturating_sub(moved);
                }
                let rest = self.lines[line][brk.min(self.lines[line].len())..].to_vec();
                if rest.first() == Some(&b' ') {
                    // `(int)(strlen(j) - strlen(rest)) <= 256`
                    if self.lines[j].len() as i32 - rest.len() as i32 <= LINE_SIZE as i32 {
                        let room = LINE_SIZE - 1 - self.lines[j].len().min(LINE_SIZE - 1);
                        self.lines[j].extend_from_slice(&rest[..rest.len().min(room)]);
                    }
                } else if (self.lines[j].len() as u32).wrapping_sub(rest.len() as u32).wrapping_sub(2) < LINE_SIZE as u32 {
                    let room = LINE_SIZE - 1 - self.lines[j].len().min(LINE_SIZE - 1);
                    let mut add = vec![b' '];
                    add.extend_from_slice(&rest);
                    self.lines[j].extend_from_slice(&add[..add.len().min(room)]);
                }
                let l = &mut self.lines[line];
                l.truncate(brk.min(l.len()));
                self.wrap_at_depth(j, screen_width, env, depth + 1);
                return;
            }
            x += 1;
        }
    }
}

/// One line of [`SayText::draw_plan`]: the coloured name part (drawn at `LINE_START`) and the
/// rest (drawn after it without a colour, i.e. in the engine's default), or the whole line in
/// [`SAY_COLOR`].
#[derive(Debug, Clone, PartialEq)]
pub enum DrawLine {
    Name { y: i32, name: Vec<u8>, color: [f32; 3], rest: Vec<u8> },
    Plain { y: i32, text: Vec<u8> },
}

impl SayText {
    /// `CHudSayText::Draw` (`0x1002eb40`) after its `AllowedToPrintText` check: the scroll timer
    /// (clamped if the clock went back), then the five lines from `Y_START`.
    pub fn draw_plan(&mut self, now: f32, env: &mut dyn SayEnv) -> Vec<DrawLine> {
        let mut y = self.y_start;
        let next = self.saytext_time + now;
        if next <= self.scroll_time {
            self.scroll_time = next;
        }
        if self.scroll_time <= now {
            if self.lines[0].is_empty() {
                self.active = false;
            } else {
                self.scroll_time = next;
                self.scroll_up(env);
            }
        }
        let mut out = Vec::new();
        for i in 0..MAX_LINES {
            let l = &self.lines[i];
            if !l.is_empty() {
                match self.color[i] {
                    Some(color) if l[0] == 2 => {
                        let n = self.name_len[i];
                        // `strncpy(buf, line, min(n, 64))`, cut at `min(n, 63)`.
                        let name = l[..n.min(63).min(l.len())].to_vec();
                        let rest = l.get(n..).map_or_else(Vec::new, <[u8]>::to_vec);
                        out.push(DrawLine::Name { y, name, color, rest });
                    }
                    _ => out.push(DrawLine::Plain { y, text: l.clone() }),
                }
            }
            y = y.wrapping_add(self.line_height);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixed 8×12 characters; console output collected.
    #[derive(Default)]
    struct Env {
        printed: Vec<Vec<u8>>,
    }
    impl SayEnv for Env {
        fn string_size(&self, text: &[u8]) -> (i32, i32) {
            (8 * text.len() as i32, 12)
        }
        fn console_print(&mut self, text: &[u8]) {
            self.printed.push(text.to_vec());
        }
    }

    fn print(s: &mut SayText, env: &mut Env, text: &str, w: i32) {
        s.print(&bytes(text), Some((b"bob", Some([1.0; 3]))), 10.0, || 5.0, (w, 480), env);
    }

    #[test]
    fn prints_scrolls_and_colours_names() {
        let (mut s, mut env) = (SayText::default(), Env::default());
        print(&mut s, &mut env, "\u{2}bob: hi\n", 640);
        assert_eq!(s.name_len[0], 4);
        assert_eq!((s.scroll_time, s.y_start), (15.0, 480 - 60 - 72));
        for i in 0..5 {
            print(&mut s, &mut env, &format!("line {i}"), 640);
        }
        assert_eq!(env.printed, vec![bytes("\u{2}bob: hi\n")], "line 0 goes to the console as it scrolls off");
        assert_eq!(s.lines[4], b"line 4");
        let plan = s.draw_plan(12.0, &mut env);
        assert_eq!(plan.len(), 5);
        assert_eq!(s.draw_plan(15.0, &mut env).len(), 4, "the scroll timer removes a line");
    }

    #[test]
    fn wraps_at_spaces_and_loses_text_after_a_hard_break() {
        let (mut s, mut env) = (SayText::default(), Env::default());
        // 200 px wide: the line may be 160 px (20 chars) including the 10 px start.
        print(&mut s, &mut env, "aaaa bbbb cccc dddd eeee", 200);
        assert_eq!(s.lines[0], b"aaaa bbbb cccc");
        assert_eq!(s.lines[1], b" dddd eeee");
        let (mut s, mut env) = (SayText::default(), Env::default());
        print(&mut s, &mut env, "abcdefghijklmnopqrstuvwxyz", 200);
        assert_eq!(s.lines[0], b"abcdefghijklmnopq", "cut one character before the overflow");
        assert!(s.lines[1].is_empty(), "SDK quirk: the remainder isn't copied");
    }

    /// A screen narrower than a character: a line starting with a space used to bounce between
    /// two free lines forever (TS overflows its stack); the wrap now gives up.
    #[test]
    fn narrow_screens_dont_recurse_forever() {
        for w in [i32::MIN, -1, 0, 30, 45] {
            let (mut s, mut env) = (SayText::default(), Env::default());
            print(&mut s, &mut env, " leading space", w);
            print(&mut s, &mut env, "  x y z", w);
            print(&mut s, &mut env, "\u{2}bob: a b c", w);
            let _ = s.draw_plan(f32::MAX, &mut env);
        }
    }

    /// Random bytes (NULs, `/(` colour codes, `\x02`, spaces) at random widths and heights.
    #[test]
    fn random_text_and_screens() {
        let mut seed = 0xdead_beef_cafe_f00du64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let (mut s, mut env) = (SayText::default(), Env::default());
        for i in 0..300 {
            let n = (next() % 400) as usize;
            let text: Vec<u8> = (0..n).map(|_| [b' ', b'/', b'(', b')', 2, 0, b'b', 0xff][(next() % 8) as usize]).collect();
            let w = [(next() % 2000) as i32, i32::MAX, i32::MIN, 0][i % 4];
            let name: &[u8] = if i % 3 == 0 { b"" } else { b"b" };
            s.print(&text, Some((name, None)), i as f32, || f32::NAN, (w, (next() % 1000) as i32 - 100), &mut env);
            for l in &s.lines {
                assert!(l.len() < LINE_SIZE);
            }
            let _ = s.draw_plan(i as f32 * 0.5, &mut env);
        }
    }

    #[test]
    fn continuation_lines_scroll_off_together() {
        let (mut s, mut env) = (SayText::default(), Env::default());
        s.lines = [b"first".to_vec(), b" more".to_vec(), b"x".to_vec(), Vec::new(), Vec::new()];
        assert_eq!(s.scroll_up(&mut env), 2);
        assert_eq!(env.printed, vec![b"first".to_vec(), b"\x02more".to_vec()]);
        assert_eq!(s.lines[0], b"x");
    }
}
