//! `CHudMessage` (SDK message.cpp as TS 3.0 ships it, element `+0x248` = `0x100cfb68`): `HudText`
//! titles.txt messages (16 slots, effects 0 fade, 1 flickery credits, 2 write-out, drawn a
//! character at a time with `pfnDrawCharacter`) and the `GameTitle` `title_half`/`title_life`
//! sprites. Every float step follows the binary: x87 at 53-bit precision, so `f64` operands,
//! rounded to `f32` where TS stores one.

use super::draw::{Canvas, HudSprites, ftol};
use super::text::Titles;
use goldsrc::ext::hud::Screen;

/// `maxHUDMessages`.
pub const MAX_HUD_MESSAGES: usize = 16;
/// `g_pCustomText` (`0x100dfb44`): the custom message's text buffer.
const CUSTOM_TEXT_SIZE: usize = 1024;

/// `client_textmessage_t`: a titles.txt entry (or the custom message).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TitleMsg {
    pub effect: i32,
    /// r1, g1, b1, a1.
    pub color1: [u8; 4],
    /// r2, g2, b2, a2.
    pub color2: [u8; 4],
    pub x: f32,
    pub y: f32,
    pub fadein: f32,
    pub fadeout: f32,
    pub holdtime: f32,
    pub fxtime: f32,
    /// `pMessage` (empty when the engine left it null).
    pub text: Vec<u8>,
}

impl TitleMsg {
    /// `MessageAdd`'s `g_pCustomMessage` (`0x100dfb18`) for a name titles.txt doesn't have: a
    /// write-out (effect 2) in (100, 100, 100) fading to (240, 110, 0), centred at 0.7 of the
    /// height. Its text is the whole name, `#` included.
    pub fn custom(name: &[u8]) -> Self {
        TitleMsg {
            effect: 2,
            color1: [100; 4],
            color2: [240, 110, 0, 0],
            x: -1.0,
            y: 0.7,
            fadein: 0.01,
            fadeout: 1.5,
            holdtime: 5.0,
            fxtime: 0.25,
            // Guarded: `strcpy` into the 1024-byte `g_pCustomText` runs over with a longer name.
            text: name[..name.len().min(CUSTOM_TEXT_SIZE - 1)].to_vec(),
        }
    }
}

/// What a slot of `m_pMessages` points at.
#[derive(Debug, Clone, PartialEq)]
pub enum Slot {
    /// A titles.txt entry.
    Title(TitleMsg),
    /// The single `g_pCustomMessage`, shared by every custom slot: a newer custom message
    /// rewrites the text the older slots show.
    Custom,
}

/// `m_parms` (`+0x94..+0xd4`), the part that outlives a draw call.
#[derive(Debug, Clone, Copy, Default)]
struct Parms {
    /// `+0x98`: the message time while scanning, else the last `m_flTime` (the level-transition
    /// fix-up reads it).
    time: f32,
    /// `+0xd4`.
    fade_time: f32,
    /// `+0xcc`.
    fade_blend: i32,
    /// `+0xd0`.
    char_time: f32,
}

/// MSVC CRT `rand` (`0x1006c05a`): TS's flicker draws from the CRT's generator (seed 1).
#[derive(Debug, Clone)]
struct CrtRand(u32);

impl CrtRand {
    fn next(&mut self) -> i32 {
        self.0 = self.0.wrapping_mul(214013).wrapping_add(2531011);
        ((self.0 >> 16) & 0x7fff) as i32
    }
}

#[derive(Debug, Clone)]
pub struct HudMessage {
    /// `m_pMessages` (`+0x14`).
    pub slots: [Option<Slot>; MAX_HUD_MESSAGES],
    /// `m_startTime` (`+0x54`).
    pub start_time: [f32; MAX_HUD_MESSAGES],
    /// `m_gameTitleTime` (`+0xd8`).
    pub game_title_time: f32,
    /// `m_pGameTitle` (`+0xdc`).
    pub game_title: Option<TitleMsg>,
    /// `g_pCustomMessage` (`0x100dfb18`), a global `Reset` leaves alone.
    pub custom: TitleMsg,
    /// `HUD_ACTIVE`.
    pub active: bool,
    parms: Parms,
    rand: CrtRand,
}

impl Default for HudMessage {
    fn default() -> Self {
        HudMessage {
            slots: Default::default(),
            start_time: [0.0; MAX_HUD_MESSAGES],
            game_title_time: 0.0,
            game_title: None,
            custom: TitleMsg::default(),
            active: false,
            parms: Parms::default(),
            rand: CrtRand(1),
        }
    }
}

/// `XPosition` (`0x10028a50`): -1 centres, a negative value aligns `totalWidth` to the right,
/// then the line is kept on screen.
pub fn x_position(x: f32, width: i32, total_width: i32, screen_width: i32) -> i32 {
    let pos = if x == -1.0 {
        (screen_width - width) / 2
    } else if x < 0.0 {
        ftol((1.0 + x as f64) * screen_width as f64 - total_width as f64)
    } else {
        ftol(screen_width as f64 * x as f64)
    };
    if pos + width > screen_width {
        screen_width - width
    } else if pos < 0 {
        0
    } else {
        pos
    }
}

/// `YPosition` (`0x10028ad0`).
pub fn y_position(y: f32, height: i32, screen_height: i32) -> i32 {
    let pos = if y == -1.0 {
        ftol((screen_height - height) as f64 * 0.5)
    } else if y < 0.0 {
        ftol((1.0 + y as f64) * screen_height as f64 - height as f64)
    } else {
        ftol(screen_height as f64 * y as f64)
    };
    if pos + height > screen_height {
        screen_height - height
    } else if pos < 0 {
        0
    } else {
        pos
    }
}

/// `FadeBlend` (`0x100289b0`): the game title's brightness.
pub fn fade_blend(fadein: f32, fadeout: f32, hold: f32, t: f32) -> f64 {
    let fade_time = (fadein as f64 + hold as f64) as f32;
    if t < 0.0 {
        return 0.0;
    }
    if t < fadein {
        return 1.0 - (fadein as f64 - t as f64) / fadein as f64;
    }
    if t <= fade_time {
        return 1.0;
    }
    if fadeout > 0.0 { 1.0 - (t as f64 - fade_time as f64) / fadeout as f64 } else { 0.0 }
}

/// `gHUD.m_scrinfo.charWidths[c]`. The line-counting pass in `MessageDrawScan` indexes it with a
/// signed `char` (`movsbl` at `0x10028f11`), reading before the table for bytes 0x80..0xff; that
/// read is guarded with the byte's own width.
fn char_width(s: &Screen, c: u8) -> i32 {
    s.char_widths[c as usize] as i32
}

impl HudMessage {
    /// `Reset` (`0x10028980`): every slot, start time and the game title cleared; `HUD_ACTIVE`
    /// and the custom message are left.
    pub fn reset(&mut self) {
        self.slots = Default::default();
        self.start_time = [0.0; MAX_HUD_MESSAGES];
        self.game_title_time = 0.0;
        self.game_title = None;
    }

    fn resolve<'a>(&'a self, slot: &'a Slot) -> &'a TitleMsg {
        match slot {
            Slot::Title(m) => m,
            Slot::Custom => &self.custom,
        }
    }

    /// `MsgFunc_HudText` (`0x100295c0`): `MessageAdd(READ_STRING(), m_flTime)`, then the time
    /// is remembered for the level-transition fix-up and the element turned on.
    pub fn msg_hud_text(&mut self, name: &[u8], now: f32, titles: &dyn Titles) {
        self.add(name, now, titles);
        self.parms.time = now;
        self.active = true;
    }

    /// `MsgFunc_GameTitle` (`0x10029610`): the `GAMETITLE` entry; found, it starts now.
    pub fn msg_game_title(&mut self, now: f32, titles: &dyn Titles) {
        self.game_title = titles.message(b"GAMETITLE");
        if self.game_title.is_some() {
            self.game_title_time = now;
            self.active = true;
        }
    }

    /// `MessageAdd` (`0x100293f0`): into the first free slot (none free: dropped). The same text
    /// already showing drops the new one; any message at the same position is replaced.
    pub fn add(&mut self, name: &[u8], time: f32, titles: &dyn Titles) {
        let Some(i) = self.slots.iter().position(Option::is_none) else { return };
        let key = name.strip_prefix(b"#").unwrap_or(name);
        let new = match titles.message(key) {
            Some(m) => Slot::Title(m),
            None => {
                self.custom = TitleMsg::custom(name);
                Slot::Custom
            }
        };
        let m = self.resolve(&new).clone();
        for j in 0..MAX_HUD_MESSAGES {
            let Some(other) = &self.slots[j] else { continue };
            let o = self.resolve(other);
            if o.text == m.text {
                return;
            }
            if (m.y as f64 - o.y as f64).abs() < 0.0001 && (m.x as f64 - o.x as f64).abs() < 0.0001 {
                self.slots[j] = None;
            }
        }
        self.slots[i] = Some(new);
        self.start_time[i] = time;
    }

    /// `Draw` (`0x100290b0`), called while `HUD_ACTIVE`: the game title, then each message until
    /// its end time. Nothing drawn turns the element off.
    pub fn draw(&mut self, now: f32, s: &Screen, sprites: &HudSprites, c: &mut dyn Canvas) {
        let mut drawn = 0;
        // Guarded: TS dereferences `m_pGameTitle` here even when the last `GameTitle` found no
        // `GAMETITLE` entry (a null pointer).
        if self.game_title_time > 0.0
            && let Some(t) = self.game_title.clone()
        {
            let local = now - self.game_title_time;
            if self.game_title_time > now {
                self.game_title_time = now;
            }
            if (local as f64) <= t.fadein as f64 + t.fadeout as f64 + t.holdtime as f64 {
                let brightness = fade_blend(t.fadein, t.fadeout, t.holdtime, local) as f32;
                // Guarded: a missing hud.txt entry is index -1, whose rectangle TS reads from
                // before the table; the port uses an empty one and skips that sprite.
                let half = sprites.index("title_half");
                let life = sprites.index("title_life");
                let rect = |i: Option<usize>| i.and_then(|i| sprites.rects.get(i)).copied().unwrap_or_default();
                let (rh, rl) = (rect(half), rect(life));
                let half_width = rh.right - rh.left;
                let full_width = half_width + rl.right - rl.left;
                let full_height = rh.bottom - rh.top;
                let x = x_position(t.x, full_width, full_width, s.width);
                let y = y_position(t.y, full_height, s.height);
                let b = brightness as f64;
                let rgb = [ftol(t.color1[0] as f64 * b), ftol(t.color1[1] as f64 * b), ftol(t.color1[2] as f64 * b)];
                if let Some(i) = half {
                    c.sprite(i, rgb, x, y);
                }
                if let Some(i) = life {
                    c.sprite(i, rgb, x + half_width, y);
                }
                drawn = 1;
            } else {
                self.game_title_time = 0.0;
            }
        }
        // Level transitions: a start time in the future is moved back, allowing the server 0.2 s.
        for i in 0..MAX_HUD_MESSAGES {
            if self.slots[i].is_some() && self.start_time[i] > now {
                let start = self.start_time[i] as f64;
                self.start_time[i] = ((now as f64 + self.parms.time as f64 - start) + 0.2) as f32;
            }
        }
        // TS keeps the end time in a stack slot only effects 0-2 write, so another effect reuses
        // the previous slot's value (or, for the first, whatever the stack held; 0 here).
        let mut end_time = 0.0f32;
        for i in 0..MAX_HUD_MESSAGES {
            let Some(slot) = &self.slots[i] else { continue };
            let m = self.resolve(slot).clone();
            let start = self.start_time[i] as f64;
            match m.effect {
                0 | 1 => end_time = (start + m.holdtime as f64 + m.fadeout as f64 + m.fadein as f64) as f32,
                2 => {
                    end_time = (m.text.len() as f64 * m.fadein as f64 + start + m.holdtime as f64 + m.fadeout as f64) as f32;
                }
                _ => {}
            }
            if now > end_time {
                self.slots[i] = None;
            } else {
                self.draw_scan(&m, (now as f64 - start) as f32, s, c);
                drawn += 1;
            }
        }
        self.parms.time = now;
        if drawn == 0 {
            self.active = false;
        }
    }

    /// `MessageDrawScan` (`0x10028eb0`).
    fn draw_scan(&mut self, m: &TitleMsg, time: f32, s: &Screen, c: &mut dyn Canvas) {
        let mut lines = 1;
        let (mut width, mut total_width) = (0, 0);
        for &ch in &m.text {
            if ch == b'\n' {
                lines += 1;
                total_width = total_width.max(width);
                width = 0;
            } else {
                width += char_width(s, ch);
            }
        }
        self.parms.time = time;
        let mut y = y_position(m.y, lines * s.char_height, s.height);
        self.parms.char_time = 0.0;
        self.scan_start(m, m.text.len());
        // Guarded: TS copies each line into an 80-byte stack buffer without a bound; the port
        // draws the line from the text instead.
        for line in m.text.split(|&b| b == b'\n').take(lines as usize) {
            let line_width: i32 = line.iter().map(|&ch| char_width(s, ch)).sum();
            let mut x = x_position(m.x, line_width, total_width, s.width);
            for &ch in line {
                let next = x + char_width(s, ch);
                let rgb = self.scan_next_char(m, ch, x, y, s, c);
                if x >= 0 && y >= 0 && next <= s.width {
                    c.character(x, y, ch, rgb);
                }
                x = next;
            }
            y += s.char_height;
        }
    }

    /// `MessageScanStart` (`0x10028d40`): the fade for this message time.
    fn scan_start(&mut self, m: &TitleMsg, length: usize) {
        let p = &mut self.parms;
        let time = p.time;
        match m.effect {
            0 | 1 => {
                p.fade_time = (m.holdtime as f64 + m.fadein as f64) as f32;
                p.fade_blend = if time < m.fadein {
                    ftol((m.fadein as f64 - time as f64) * (1.0 / m.fadein as f64) * 255.0)
                } else if time > p.fade_time {
                    if m.fadeout > 0.0 { ftol((time as f64 - p.fade_time as f64) / m.fadeout as f64 * 255.0) } else { 255 }
                } else {
                    0
                };
                p.char_time = 0.0;
                if m.effect == 1 && self.rand.next() % 100 < 10 {
                    p.char_time = 1.0;
                }
            }
            2 => {
                p.fade_time = (length as f64 * m.fadein as f64 + m.holdtime as f64) as f32;
                p.fade_blend = if time > p.fade_time && m.fadeout > 0.0 {
                    ftol((time as f64 - p.fade_time as f64) / m.fadeout as f64 * 255.0)
                } else {
                    0
                };
            }
            _ => {}
        }
    }

    /// `MessageScanNextChar` (`0x10028b60`): the character's colour; a flickering credits
    /// character (effect 1) is first drawn in the second colour.
    fn scan_next_char(&mut self, m: &TitleMsg, ch: u8, x: i32, y: i32, s: &Screen, c: &mut dyn Canvas) -> [i32; 3] {
        let p = &mut self.parms;
        let mut src = [m.color1[0] as i32, m.color1[1] as i32, m.color1[2] as i32];
        let mut dest = [0; 3];
        let blend = match m.effect {
            0 | 1 => p.fade_blend,
            2 => {
                // `fcoms` compares the unrounded sum; `fsts` then stores it as a float.
                let sum = p.char_time as f64 + m.fadein as f64;
                p.char_time = sum as f32;
                if sum > p.time as f64 {
                    src = [0; 3];
                    0
                } else {
                    let delta = (p.time as f64 - sum) as f32;
                    if p.time > p.fade_time {
                        p.fade_blend
                    } else if delta > m.fxtime {
                        0
                    } else {
                        dest = [m.color2[0] as i32, m.color2[1] as i32, m.color2[2] as i32];
                        ftol(255.0 - ((1.0 / m.fxtime as f64) * delta as f64 * 255.0 + 0.5))
                    }
                }
            }
            // Any other effect is pure source.
            _ => 0,
        }
        .clamp(0, 255);
        let rgb = [0, 1, 2].map(|k| (src[k] * (255 - blend) + dest[k] * blend) >> 8);
        if m.effect == 1 && p.char_time != 0.0 && x >= 0 && y >= 0 && x + char_width(s, ch) <= s.width {
            c.character(x, y, ch, [m.color2[0] as i32, m.color2[1] as i32, m.color2[2] as i32]);
        }
        rgb
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use goldsrc::sys::wrect_t;

    #[derive(Default)]
    struct Rec {
        chars: Vec<(i32, i32, u8, [i32; 3])>,
        sprites: Vec<(usize, [i32; 3], i32, i32)>,
    }
    impl Canvas for Rec {
        fn console_size(&self, t: &[u8]) -> (i32, i32) {
            (t.len() as i32 * 8, 12)
        }
        fn console_string(&mut self, x: i32, _: i32, _: Option<[f32; 3]>, t: &[u8]) -> i32 {
            x + t.len() as i32 * 8
        }
        fn character(&mut self, x: i32, y: i32, ch: u8, rgb: [i32; 3]) {
            self.chars.push((x, y, ch, rgb));
        }
        fn sprite(&mut self, i: usize, rgb: [i32; 3], x: i32, y: i32) {
            self.sprites.push((i, rgb, x, y));
        }
    }

    struct T;
    impl Titles for T {
        fn get(&self, _: &str) -> Option<String> {
            None
        }
        fn message(&self, key: &[u8]) -> Option<TitleMsg> {
            let base = TitleMsg { effect: 0, color1: [200, 100, 50, 0], holdtime: 2.0, fadein: 0.5, fadeout: 1.0, ..Default::default() };
            match key {
                b"HELLO" => Some(TitleMsg { x: -1.0, y: 0.5, text: b"Hi\nthere".to_vec(), ..base }),
                b"OTHER" => Some(TitleMsg { x: -1.0, y: 0.5, text: b"Bye".to_vec(), ..base }),
                b"GAMETITLE" => Some(TitleMsg { x: -1.0, y: -1.0, color1: [255, 255, 255, 0], ..base }),
                _ => None,
            }
        }
    }

    fn screen() -> Screen {
        Screen { width: 640, height: 480, char_height: 10, char_widths: [6; 256] }
    }

    #[test]
    fn positions_match_the_sdk() {
        assert_eq!(x_position(-1.0, 100, 100, 640), 270);
        assert_eq!(x_position(0.25, 100, 100, 640), 160);
        assert_eq!(x_position(-0.25, 50, 100, 640), 380, "right-aligned on the total width");
        assert_eq!(x_position(0.95, 100, 100, 640), 540, "kept on screen");
        assert_eq!(y_position(-1.0, 31, 480), 224, "(480 - 31) * 0.5 truncated");
        assert_eq!(y_position(0.7, 10, 480), 335, "0.7f32 * 480 is just under 336");
        assert_eq!(y_position(-0.1, 10, 480), 421);
        assert_eq!(ftol(-0.9), 0);
        assert_eq!(ftol(f64::INFINITY), i32::MIN);
    }

    #[test]
    fn fade_blend_ramps() {
        assert_eq!(fade_blend(1.0, 2.0, 3.0, -0.1), 0.0);
        assert_eq!(fade_blend(1.0, 2.0, 3.0, 0.25), 0.25);
        assert_eq!(fade_blend(1.0, 2.0, 3.0, 4.0), 1.0);
        assert_eq!(fade_blend(1.0, 2.0, 3.0, 5.0), 0.5);
        assert_eq!(fade_blend(1.0, 0.0, 3.0, 5.0), 0.0);
    }

    #[test]
    fn add_replaces_same_position_and_skips_duplicates() {
        let mut h = HudMessage::default();
        h.msg_hud_text(b"#HELLO", 1.0, &T);
        assert!(h.active);
        assert_eq!(h.parms.time, 1.0);
        h.add(b"HELLO", 2.0, &T);
        assert_eq!(h.slots.iter().flatten().count(), 1, "same text: not added");
        assert_eq!(h.start_time[0], 1.0);
        h.add(b"OTHER", 3.0, &T);
        assert!(h.slots[0].is_none(), "same position: replaced");
        assert!(matches!(&h.slots[1], Some(Slot::Title(m)) if m.text == b"Bye"));
        h.add(b"custom", 4.0, &T);
        assert!(h.slots[1].is_some(), "another position (0.7) leaves it");
        assert_eq!(h.slots[0], Some(Slot::Custom), "the first free slot");
    }

    #[test]
    fn custom_messages_share_the_global() {
        let mut h = HudMessage::default();
        h.add(b"#unknown one", 1.0, &T);
        assert_eq!(h.slots[0], Some(Slot::Custom));
        assert_eq!(h.custom.text, b"#unknown one", "the name, # included");
        assert_eq!((h.custom.effect, h.custom.x, h.custom.y, h.custom.holdtime), (2, -1.0, 0.7, 5.0));
        h.add(b"two", 2.0, &T);
        assert_eq!(h.slots.iter().flatten().count(), 1, "the rewritten global matches itself");
        assert_eq!(h.custom.text, b"two", "the shown slot now reads the new text");
        assert_eq!(h.start_time[0], 1.0);
    }

    #[test]
    fn full_slots_drop_new_messages() {
        let mut h = HudMessage::default();
        for i in 0..MAX_HUD_MESSAGES {
            h.slots[i] = Some(Slot::Title(TitleMsg { x: i as f32, text: vec![b'a' + i as u8], ..Default::default() }));
        }
        h.add(b"HELLO", 1.0, &T);
        assert!(h.slots.iter().all(|s| !matches!(s, Some(Slot::Title(m)) if m.text == b"Hi\nthere")));
    }

    #[test]
    fn fade_message_draws_per_character_and_expires() {
        let s = screen();
        let sprites = HudSprites::default();
        let mut h = HudMessage::default();
        h.msg_hud_text(b"HELLO", 10.0, &T);
        let mut r = Rec::default();
        // 1.5 s in: past the 0.5 s fade-in, inside the 2 s hold: full colour.
        h.draw(11.5, &s, &sprites, &mut r);
        let total_height = 2 * 10;
        let y0 = y_position(0.5, total_height, 480);
        assert_eq!(r.chars.len(), 7);
        assert_eq!(r.chars[0], (x_position(-1.0, 12, 0, 640), y0, b'H', [199, 99, 49]), "(c * 255) >> 8");
        assert_eq!(r.chars[2].1, y0 + 10, "second line");
        assert_eq!(r.chars[2].0, x_position(-1.0, 30, 0, 640), "the last line's width isn't in totalWidth");
        assert!(h.active);
        // Fading out: 3 s in, a second past the hold, fadeout 1: blend 255 * 0.5.
        let mut r = Rec::default();
        h.draw(13.0, &s, &sprites, &mut r);
        let blend = ftol(0.5 * 255.0);
        assert_eq!(r.chars[0].3, [(200 * (255 - blend)) >> 8, (100 * (255 - blend)) >> 8, (50 * (255 - blend)) >> 8]);
        // Over: end = start + hold + fadeout + fadein = 13.5.
        let mut r = Rec::default();
        h.draw(13.6, &s, &sprites, &mut r);
        assert!(r.chars.is_empty());
        assert!(h.slots.iter().all(Option::is_none));
        assert!(!h.active, "nothing drawn turns the element off");
    }

    #[test]
    fn write_out_scans_characters_in() {
        let s = screen();
        let mut h = HudMessage::default();
        h.add(b"ab", 0.0, &T);
        let mut r = Rec::default();
        // 0.015 s in: 'a' (char time 0.01) is 0.005 s into its 0.25 s fx; 'b' (0.02) not yet.
        h.draw(0.015, &s, &HudSprites::default(), &mut r);
        let delta = (0.015f32 as f64 - (0.0f32 as f64 + 0.01f32 as f64)) as f32;
        let blend = ftol(255.0 - ((1.0 / 0.25f32 as f64) * delta as f64 * 255.0 + 0.5));
        let a = [100 * (255 - blend) + 240 * blend, 100 * (255 - blend) + 110 * blend, 100 * (255 - blend)].map(|v| v >> 8);
        assert_eq!(r.chars[0].3, a);
        assert_eq!(r.chars[1].3, [0, 0, 0], "not reached yet: black");
        assert_eq!(r.chars[0].1, y_position(0.7, 10, 480));
    }

    #[test]
    fn level_transition_moves_start_back() {
        let s = screen();
        let mut h = HudMessage::default();
        h.msg_hud_text(b"HELLO", 50.0, &T);
        // New map: the clock restarted at 2.
        h.draw(2.0, &s, &HudSprites::default(), &mut Rec::default());
        assert_eq!(h.start_time[0], ((2.0f64 + 50.0 - 50.0) + 0.2) as f32);
    }

    #[test]
    fn game_title_sprites_fade() {
        let s = screen();
        let rect = |w: i32, h: i32| wrect_t { left: 0, right: w, top: 0, bottom: h };
        let sprites = HudSprites {
            names: vec!["title_half".into(), "title_life".into()],
            handles: vec![1, 2],
            rects: vec![rect(100, 40), rect(60, 40)],
            number_0: None,
        };
        let mut h = HudMessage::default();
        h.msg_game_title(4.0, &T);
        assert!(h.active);
        let mut r = Rec::default();
        h.draw(4.25, &s, &sprites, &mut r); // half the 0.5 s fade-in
        let x = x_position(-1.0, 160, 160, 640);
        let y = y_position(-1.0, 40, 480);
        assert_eq!(r.sprites, [(0, [127, 127, 127], x, y), (1, [127, 127, 127], x + 100, y)]);
        h.draw(7.6, &s, &sprites, &mut Rec::default()); // past 0.5 + 1 + 2
        assert_eq!(h.game_title_time, 0.0);
    }

    #[test]
    fn reset_keeps_the_flag() {
        let mut h = HudMessage::default();
        h.msg_hud_text(b"HELLO", 1.0, &T);
        h.reset();
        assert!(h.slots.iter().all(Option::is_none));
        assert!(h.active, "Reset leaves HUD_ACTIVE; the next Draw clears it");
    }
}
