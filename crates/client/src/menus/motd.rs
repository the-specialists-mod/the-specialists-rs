//! TS's VGUI1 message window, `CMessageWindowPanel` (constructor `0x1005bb90`), as
//! `CreateTextWindow` (`0x10063c00`) makes it for the MOTD (`VGUIMenu` 5 and the final `MOTD`
//! chunk: the server name, or "Half-Life", 31 characters at most, over the MOTD text) and for the
//! map briefing (`VGUIMenu` 4: the map's name over `maps/<map>.txt`).
//!
//! The window is a `CMenuPanel` the size of the screen (`ScreenWidth`×`ScreenHeight`,
//! `0x100d5908`/`0x100d590c`, `gHUD.m_scrinfo`). While the local player has no class
//! (`g_iPlayerClass`) the whole of it is shaded (black at VGUI alpha 100) and the window clear;
//! otherwise the screen is clear and the window shaded. The window sits at (`XRES(112)`,
//! `YRES(80)`), `XRES(424)`×`YRES(312)`, with an orange `LineBorder`; the title in "Primary
//! Button Text" (the constructor's `setFont(Scheme::sf_primary1)` replaces "Title Font"), the text
//! in "Briefing Text" in a `CTFScrollPanel`, and "#Menu_OK" as a `CommandButton` in the
//! bottom-left corner that closes the window.
//!
//! The scroll panel is set up as the constructor does: both bars forced on (so the text wraps to
//! the client clip less the vertical bar), the text panel sized to the wrapped text, then the bars
//! back to automatic.

use super::scroll::ScrollPanel;
use super::widgets::{CommandButton, ViewportScheme, transparent_background};
use crate::hud::vgui::{Align, Color, Image, Panel, Schemes, Surface, ftol, label_content_size, label_text_pos, spill, xs, ys};
use std::rc::Rc;

/// The "Briefing Text" font's `getTextSizeWrapped(text, width)`.
pub type WrapFn<'a> = dyn Fn(&[u8], i32) -> (i32, i32) + 'a;

#[derive(Debug, Clone, PartialEq)]
pub struct Motd {
    pub title: Vec<u8>,
    pub text: Vec<u8>,
    /// `iShadeFullscreen`: the local player had no class when it was made.
    pub shade: bool,
    pub ok: CommandButton,
    pub scroll: ScrollPanel,
    hover: bool,
}

/// The window's rectangle and the scroll panel's (`iXPos + XRES(16)`, `iYPos + YRES(16) * 3`,
/// `iXSize - XRES(32)`, `iYSize - (YRES(48) + BUTTON_SIZE_Y * 2)`), in screen coordinates.
pub fn layout(w: i32, h: i32) -> (Panel, Panel) {
    let (xsc, ysc) = (xs(w), ys(h));
    let menu = menu_panel(w, h);
    let win = menu.child(ftol(spill(xsc) * 112.0), ftol(ysc * 80.0), ftol(xsc * 424.0), ftol(ysc * 312.0)).bordered();
    let (x, y) = (win.x as f64, win.y as f64);
    let scroll = menu.child(
        ftol(x + spill(xsc) * 16.0),
        ftol(y + ysc * 16.0 * 3.0),
        ftol(win.w as f64 - spill(xsc) * 32.0),
        ftol(win.h as f64 - (ysc * 48.0 + ysc * 30.0 * 2.0)),
    );
    (win, scroll)
}

/// The `CMenuPanel` itself: at (0, 0), `ScreenWidth`×`ScreenHeight`.
fn menu_panel(w: i32, h: i32) -> Panel {
    Panel::root(w, h).child(0, 0, w, h)
}

/// `CreateTextWindow(SHOW_MAPBRIEFING)` (`0x10063c00`, case 1): the briefing file and the title.
/// `map_name` is the viewport's `m_sMapName` (from `VGUIMenu` 4); when it is empty the level's
/// name is used (`maps/<map>.bsp` up to the first '.', plus ".txt") and `m_sMapName` becomes the
/// level's name without its extension and up to the first '/'. `None` (no window) without a level
/// or a file. Guarded: a level name without a '.' (TS writes through `strchr`'s null) keeps all
/// of it.
pub fn map_briefing(map_name: &mut Vec<u8>, level: Option<&[u8]>, load: &dyn Fn(&[u8]) -> Option<Vec<u8>>) -> Option<(Vec<u8>, Vec<u8>)> {
    let path = if !map_name.is_empty() {
        [b"maps/".as_slice(), map_name, b".txt"].concat()
    } else {
        let level = level?;
        let cut = |s: &[u8]| s[..s.iter().position(|&c| c == b'.').unwrap_or(s.len())].to_vec();
        let mut name = cut(level);
        if let Some(i) = name.iter().position(|&c| c == b'/') {
            name.drain(..=i);
        }
        *map_name = name;
        [cut(level), b".txt".to_vec()].concat()
    };
    let text = load(&path)?;
    let text = text.into_iter().take_while(|&c| c != 0).collect();
    Some((map_name.iter().copied().take(31).collect(), text))
}

impl Motd {
    /// `CMessageWindowPanel(text, title, shade, …)`: `ok_text` is "#Menu_OK" localised;
    /// `wrapped(text, width)` is the "Briefing Text" font's `getTextSizeWrapped`.
    pub fn new(title: &[u8], text: &[u8], ok_text: &str, shade: bool, w: i32, h: i32, wrapped: &WrapFn<'_>) -> Motd {
        let (xsc, ysc) = (xs(w), ys(h));
        let (win, sp) = layout(w, h);
        let ok = CommandButton::new(
            &crate::hud::saytext::bytes(ok_text),
            ftol(win.x as f64 + spill(xsc) * 16.0),
            ftol((win.y + win.h) as f64 - ysc * 16.0 - ysc * 30.0),
            ftol(spill(xsc) * 160.0),
            ftol(ysc * 30.0),
        );
        let mut scroll = ScrollPanel::ctf(sp.x, sp.y, sp.w, sp.h);
        // Both bars forced on, so the client clip leaves room for them.
        scroll.set_scroll_bar_auto_visible(false, false);
        scroll.set_scroll_bar_visible(true, true);
        scroll.validate();
        // The text panel, sized to the text wrapped to the client clip's width.
        scroll.content = Some(wrapped(text, scroll.clip.0));
        scroll.set_scroll_bar_auto_visible(true, true);
        scroll.set_scroll_bar_visible(false, false);
        scroll.validate();
        Motd { title: title.to_vec(), text: text.to_vec(), shade, ok, scroll, hover: false }
    }

    /// `CreateTextWindow(SHOW_MOTD)`: the server name (or "Half-Life"), 31 characters at most.
    pub fn for_motd(server_name: &str, motd: &str, ok_text: &str, shade: bool, w: i32, h: i32, wrapped: &WrapFn<'_>) -> Motd {
        let title = if server_name.is_empty() { "Half-Life" } else { server_name };
        let title: Vec<u8> = crate::hud::saytext::bytes(title).into_iter().take(31).collect();
        Motd::new(&title, &crate::hud::saytext::bytes(motd), ok_text, shade, w, h, wrapped)
    }

    /// The cursor moved: the OK button arms while it's over it (`CHandler_CommandButtonHighlight`)
    /// and the scroll bars follow.
    pub fn hover(&mut self, w: i32, h: i32, x: i32, y: i32) {
        let menu = menu_panel(w, h);
        let now = self.ok.panel(&menu).is_within(x, y);
        if now != self.hover {
            self.ok.armed = now;
            self.hover = now;
        }
        self.scroll.cursor_moved(&menu, x, y);
    }

    /// A mouse press: the scroll bars' sliders.
    pub fn press(&mut self, w: i32, h: i32, x: i32, y: i32) {
        self.scroll.mouse_pressed(&menu_panel(w, h), x, y);
    }

    /// A mouse release: the scroll bars take theirs; a left release on OK closes the window
    /// (`CMenuHandler_TextWindow(HIDE_TEXTWINDOW)`, returns true).
    pub fn release(&mut self, w: i32, h: i32, x: i32, y: i32, left: bool) -> bool {
        let menu = menu_panel(w, h);
        if self.scroll.mouse_released(&menu, x, y, left) {
            return false;
        }
        left && self.ok.panel(&menu).is_within(x, y)
    }

    pub fn paint(&self, s: &mut dyn Surface, sb: &Schemes, w: i32, h: i32, arrows: &[Option<Rc<Image>>; 4]) {
        let vs = ViewportScheme::from(sb);
        let menu = menu_panel(w, h);
        transparent_background(s, &menu, if self.shade { 100 } else { 255 });
        let (win, _) = layout(w, h);
        win.line_border(s, Color::new(178, 118, 0, 0));
        transparent_background(s, &win, if self.shade { 255 } else { 100 });
        let (xsc, ysc) = (xs(w), ys(h));
        let title = sb.get("Title Font");
        if let Some(f) = sb.font_of("Primary Button Text") {
            let (tw, th) = f.text_size(&self.title);
            let (lw, lh) = label_content_size(tw, th);
            let p = menu.child(ftol(win.x as f64 + xsc * 16.0), ftol(win.y as f64 + ysc * 16.0), lw, lh);
            p.paint_background(s, title.bg);
            let (tx, ty) = label_text_pos(Align::West, lw, lh, tw, th);
            p.text(s, f, tx, ty, vs.primary1, &self.title, tw);
        }
        let brief = sb.get("Briefing Text");
        if let (Some(f), Some(p)) = (sb.font(brief), self.scroll.content_panel(&menu)) {
            p.paint_background(s, brief.bg);
            p.text(s, f, 0, 0, brief.fg, &self.text, p.w);
        }
        self.scroll.paint_bars(s, &menu, &vs, arrows);
        self.ok.paint(s, &menu, sb, &vs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny xorshift for the fuzz tests.
    fn xorshift(s: &mut u64) -> u64 {
        *s ^= *s << 13;
        *s ^= *s >> 7;
        *s ^= *s << 17;
        *s
    }

    /// Garbage map names, level names and briefing files.
    #[test]
    fn garbage_briefings() {
        let mut s = 0x0bad_c0de_1234_5678u64;
        let sb = Schemes::load(640, false, &|_| None);
        let mut bytes = move |n: u64| -> Vec<u8> {
            let n = xorshift(&mut s) % n;
            (0..n).map(|_| [b'.', b'/', b'\\', 0, 0xff, b'a'][(xorshift(&mut s) % 6) as usize]).collect()
        };
        for _ in 0..300 {
            let mut name = bytes(80);
            let level = bytes(80);
            let file = bytes(3000);
            let made = map_briefing(&mut name, Some(&level), &|_| Some(file.clone()));
            let Some((title, text)) = made else { panic!("a file loads") };
            assert!(title.len() <= 31 && !text.contains(&0));
            let m = Motd::new(&title, &text, "\u{e9}OK", true, 640, 480, &|t, w| (w, t.len() as i32 * 12));
            m.paint(&mut crate::hud::vgui::tests::Record::default(), &sb, 640, 480, &[None, None, None, None]);
        }
        let _ = Motd::for_motd("\u{1F600}\u{e9}", "\u{1F600}", "", false, 0, 0, &|_, _| (i32::MAX / 2, i32::MAX / 2));
    }

    #[test]
    fn motd_layout() {
        let (win, scroll) = layout(640, 480);
        assert_eq!((win.x, win.y, win.w, win.h), (112, 80, 424, 312));
        assert_eq!((scroll.x, scroll.y, scroll.w, scroll.h), (128, 128, 392, 203), "TS's 1/480 is a float a hair above it");
        let wrapped = |t: &[u8], w: i32| (w.min(t.len() as i32 * 10), 12);
        let m = Motd::for_motd("", "Hi", "OK", true, 640, 480, &wrapped);
        assert_eq!(m.title, b"Half-Life");
        assert_eq!((m.ok.x, m.ok.y, m.ok.w, m.ok.h), (128, 345, 160, 30));
        assert_eq!(Motd::for_motd(&"x".repeat(40), "", "OK", false, 640, 480, &wrapped).title.len(), 31);
        let mut m = m;
        m.hover(640, 480, 130, 350);
        assert!(m.ok.armed && m.release(640, 480, 130, 350, true));
        assert!(!m.release(640, 480, 130, 350, false), "only the left button clicks");
        m.hover(640, 480, 0, 0);
        assert!(!m.ok.armed && !m.release(640, 480, 0, 0, true));
    }

    /// Issue #266: TS 2.1's message window (0x100a0ad0) divides by 640/480 with nothing
    /// spilled, so the scroll panel's `iYSize - (YRES(48) + YRES(30) * 2)` and the OK button's
    /// `bottom - YRES(16) - YRES(30)` come out exact at 640×480: a pixel more than 3.0's.
    #[test]
    fn motd_layout_by_division() {
        use crate::gameplay::DllBuild;
        use crate::hud::vgui::with_scale;
        let (win, scroll) = with_scale(DllBuild::Debug, || layout(640, 480));
        assert_eq!((win.x, win.y, win.w, win.h), (112, 80, 424, 312));
        assert_eq!((scroll.x, scroll.y, scroll.w, scroll.h), (128, 128, 392, 204));
        let m = with_scale(DllBuild::Debug, || Motd::for_motd("", "Hi", "OK", true, 640, 480, &|_, w| (w, 12)));
        assert_eq!((m.ok.x, m.ok.y, m.ok.w, m.ok.h), (128, 346, 160, 30));
        let (_, s30) = with_scale(DllBuild::Optimised, || layout(1920, 1080));
        let (_, s21) = with_scale(DllBuild::Debug, || layout(1920, 1080));
        assert_eq!((s30.h, s21.h), (458, 459));
    }

    /// The text wraps to the client clip less the forced vertical bar; a short text needs no
    /// bars, a long one gets the vertical bar back.
    #[test]
    fn motd_scrolls() {
        let seen = std::cell::Cell::new(0);
        let short = |_: &[u8], w: i32| {
            seen.set(w);
            (100, 50)
        };
        let m = Motd::for_motd("", "Hi", "OK", true, 640, 480, &short);
        assert_eq!(seen.get(), 392 - 16);
        assert!(!m.scroll.vbar.visible && !m.scroll.hbar.visible);
        assert_eq!(m.scroll.clip, (392, 203));
        let mut m = Motd::for_motd("", "long", "OK", true, 640, 480, &|_, _| (376, 600));
        assert!(m.scroll.vbar.visible && !m.scroll.hbar.visible);
        // The down arrow at the bottom of the bar steps the text up by 15.
        let (bx, by) = (128 + 376 + 8, 128 + 203 - 8);
        assert!(!m.release(640, 480, bx, by, true));
        assert_eq!(m.scroll.client.1, -15);
    }

    /// The shade is the whole `ScreenWidth`×`ScreenHeight` panel, at any size (1896×924 here, as
    /// in the reported screenshot, where the world under it is evenly darkened: the darker left
    /// part is the map's near wall, not the shade's edge).
    #[test]
    fn shade_covers_the_screen() {
        let sb = Schemes::load(1896, false, &|_| None);
        let m = Motd::for_motd("", "Hi", "OK", true, 1896, 924, &|_, _| (10, 10));
        let mut s = crate::hud::vgui::tests::Record::default();
        m.paint(&mut s, &sb, 1896, 924, &[None, None, None, None]);
        assert_eq!(s.0[0], (0, 0, 1896, 924, [0, 0, 0], 155));
        let m = Motd { shade: false, ..m };
        let mut s = crate::hud::vgui::tests::Record::default();
        m.paint(&mut s, &sb, 1896, 924, &[None, None, None, None]);
        assert!(s.0.iter().all(|f| f.2 < 1896), "in the game the window is shaded instead");
    }

    #[test]
    fn map_briefing_files() {
        let load = |p: &[u8]| (p == b"maps/ts_dojo.txt").then(|| b"Dojo\0junk".to_vec());
        let mut name = Vec::new();
        assert_eq!(map_briefing(&mut name, Some(b"maps/ts_dojo.bsp"), &load), Some((b"ts_dojo".to_vec(), b"Dojo".to_vec())));
        assert_eq!(name, b"ts_dojo", "m_sMapName is filled from the level");
        assert!(map_briefing(&mut name, None, &load).is_some(), "a set map name doesn't need the level");
        let mut other = b"ts_other".to_vec();
        assert_eq!(map_briefing(&mut other, Some(b"maps/ts_dojo.bsp"), &load), None, "no file, no window");
        assert_eq!(map_briefing(&mut Vec::new(), None, &load), None);
    }
}
