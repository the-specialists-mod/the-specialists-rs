//! TS's VGUI1 message window, `CMessageWindowPanel` (constructor `0x1005bb90`), as
//! `CreateTextWindow` (`0x10063c00`) makes it for the MOTD (`VGUIMenu` 5 and the final `MOTD`
//! chunk): the server name (or "Half-Life", 31 characters at most) as the title and the MOTD text.
//!
//! The window is a `CMenuPanel` over the whole screen. While the local player has no class
//! (`g_iPlayerClass`) the screen is shaded (black at VGUI alpha 100) and the window clear; otherwise
//! the screen is clear and the window shaded. The window sits at (`XRES(112)`, `YRES(80)`),
//! `XRES(424)`×`YRES(312)`, with an orange `LineBorder`; the title in "Primary Button Text" (the
//! constructor's `setFont(Scheme::sf_primary1)` replaces "Title Font"), the text in "Briefing
//! Text" wrapped to the scroll panel's width less its 16-pixel scroll bar, and "#Menu_OK" as a
//! `CommandButton` in the bottom-left corner that closes the window.

use super::widgets::{CommandButton, ViewportScheme, transparent_background};
use crate::hud::vgui::{Align, Color, Panel, Schemes, Surface, ftol, label_content_size, label_text_pos, spill, xs, ys};

#[derive(Debug, Clone, PartialEq)]
pub struct Motd {
    pub title: Vec<u8>,
    pub text: Vec<u8>,
    /// `iShadeFullscreen`: the local player had no class when it was made.
    pub shade: bool,
    pub ok: CommandButton,
    hover: bool,
}

/// The window's rectangle and the scroll panel's, in screen coordinates.
pub fn layout(w: i32, h: i32) -> (Panel, Panel) {
    let (xsc, ysc) = (xs(w), ys(h));
    let menu = Panel::root(w, h).child(0, 0, w, h);
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

impl Motd {
    /// `CreateTextWindow(SHOW_MOTD)` and the constructor; `ok_text` is "#Menu_OK" localised.
    pub fn new(server_name: &str, motd: &str, ok_text: &str, shade: bool, w: i32, h: i32) -> Motd {
        let title = if server_name.is_empty() { "Half-Life" } else { server_name };
        let (xsc, ysc) = (xs(w), ys(h));
        let (win, _) = layout(w, h);
        let ok = CommandButton::new(
            &crate::hud::saytext::bytes(ok_text),
            ftol(win.x as f64 + spill(xsc) * 16.0),
            ftol((win.y + win.h) as f64 - ysc * 16.0 - ysc * 30.0),
            ftol(spill(xsc) * 160.0),
            ftol(ysc * 30.0),
        );
        Motd {
            title: crate::hud::saytext::bytes(title).into_iter().take(31).collect(),
            text: crate::hud::saytext::bytes(motd),
            shade,
            ok,
            hover: false,
        }
    }

    /// The OK button arms while the cursor is over it (`CHandler_CommandButtonHighlight`).
    pub fn hover(&mut self, w: i32, h: i32, x: i32, y: i32) {
        let now = self.ok.panel(&Panel::root(w, h)).is_within(x, y);
        if now != self.hover {
            self.ok.armed = now;
            self.hover = now;
        }
    }

    /// A left-button release on OK closes the window (`CMenuHandler_TextWindow(HIDE_TEXTWINDOW)`).
    pub fn click(&self, w: i32, h: i32, x: i32, y: i32) -> bool {
        self.ok.panel(&Panel::root(w, h)).is_within(x, y)
    }

    pub fn paint(&self, s: &mut dyn Surface, sb: &Schemes, w: i32, h: i32) {
        let vs = ViewportScheme::from(sb);
        let menu = Panel::root(w, h).child(0, 0, w, h);
        transparent_background(s, &menu, if self.shade { 100 } else { 255 });
        let (win, scroll) = layout(w, h);
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
        if let Some(f) = sb.font(brief) {
            // Wrapped to the client area with both scroll bars forced on, then sized to that.
            let wrap = scroll.w - 16;
            let (tw, th) = f.text_size_wrapped(&self.text, wrap);
            let p = scroll.child(0, 0, tw, th);
            p.paint_background(s, brief.bg);
            p.text(s, f, 0, 0, brief.fg, &self.text, tw);
        }
        self.ok.paint(s, &menu, sb, &vs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn motd_layout() {
        let (win, scroll) = layout(640, 480);
        assert_eq!((win.x, win.y, win.w, win.h), (112, 80, 424, 312));
        assert_eq!((scroll.x, scroll.y, scroll.w, scroll.h), (128, 128, 392, 203), "TS's 1/480 is a float a hair above it");
        let m = Motd::new("", "Hi", "OK", true, 640, 480);
        assert_eq!(m.title, b"Half-Life");
        assert_eq!((m.ok.x, m.ok.y, m.ok.w, m.ok.h), (128, 345, 160, 30));
        assert_eq!(Motd::new(&"x".repeat(40), "", "OK", false, 640, 480).title.len(), 31);
        let mut m = m;
        m.hover(640, 480, 130, 350);
        assert!(m.ok.armed && m.click(640, 480, 130, 350));
        m.hover(640, 480, 0, 0);
        assert!(!m.ok.armed && !m.click(640, 480, 0, 0));
    }
}
