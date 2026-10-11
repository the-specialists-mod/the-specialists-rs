//! Pieces of TS's VGUI1 viewport that its menus share: the team colours, the scheme colours the
//! viewport constructor puts in VGUI's `Scheme`, `CTransparentPanel` and `CommandButton`.

use crate::hud::vgui::{Align, Color, Panel, Schemes, Surface, label_text_pos};

/// `iTeamColors` (`0x10096974`, `iNumberOfTeamColors` = 5 at `0x10096970`).
pub const TEAM_COLORS: [[u8; 3]; 5] = [[255, 170, 0], [125, 165, 210], [200, 90, 70], [225, 205, 45], [145, 215, 140]];

/// `iTeamColors[team % iNumberOfTeamColors]` (team numbers are never negative: `ScoreInfo`
/// clamps them, `0x100650f6`).
pub fn team_color(team: i32, a: u8) -> Color {
    let c = TEAM_COLORS[team.rem_euclid(5) as usize];
    Color::new(c[0], c[1], c[2], a)
}

/// VGUI's `Scheme::sc_white` (VGUI1's default scheme; the viewport leaves it).
pub const SC_WHITE: Color = Color::new(255, 255, 255, 0);

/// The scheme colours `TeamFortressViewport`'s constructor (`0x100607f0`) sets from
/// "Primary Button Text": `sc_primary1` its foreground, `sc_primary3` its background,
/// `sc_secondary2` its armed foreground, `sc_primary2` its armed background, and `sc_secondary1`
/// the fixed button-border orange (178, 118, 0).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewportScheme {
    pub primary1: Color,
    pub primary2: Color,
    pub primary3: Color,
    pub secondary1: Color,
    pub secondary2: Color,
}

impl ViewportScheme {
    pub fn from(s: &Schemes) -> ViewportScheme {
        let p = s.get("Primary Button Text");
        ViewportScheme {
            primary1: p.fg,
            primary2: p.armed_bg,
            primary3: p.bg,
            secondary1: Color::new(178, 118, 0, 0),
            secondary2: p.armed_fg,
        }
    }
}

/// `CTransparentPanel::paintBackground`: black at the panel's transparency over its full size
/// (`_size`, not the paint size), when the transparency isn't 0.
pub fn transparent_background(s: &mut dyn Surface, p: &Panel, transparency: u8) {
    if transparency != 0 {
        p.fill_rect(s, 0, 0, p.w, p.h, Color::new(0, 0, 0, transparency));
    }
}

/// TS's `CommandButton` (constructor `0x1005a0b0`, `Init` `0x1005a170`): a VGUI `Button` whose
/// text is `"  %c  %s"` with its bound key, else `"     %s"`, cut to 31 characters
/// (`RecalculateText` `0x1005a1f0`; the main text is kept to 31 as well).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CommandButton {
    pub text: Vec<u8>,
    pub key: u8,
    pub visible: bool,
    pub armed: bool,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl CommandButton {
    pub fn new(text: &[u8], x: i32, y: i32, w: i32, h: i32) -> CommandButton {
        let mut b = CommandButton { visible: true, x, y, w, h, ..CommandButton::default() };
        b.set_text(text);
        b
    }

    /// `CommandButton::setText` (`0x1005a250`).
    pub fn set_text(&mut self, text: &[u8]) {
        self.text = text.iter().copied().take_while(|&c| c != 0).take(31).collect();
    }

    /// The label text `RecalculateText` gives VGUI.
    pub fn label(&self) -> Vec<u8> {
        let mut s = if self.key != 0 { vec![b' ', b' ', self.key, b' ', b' '] } else { b"     ".to_vec() };
        s.extend_from_slice(&self.text);
        s.truncate(31);
        s
    }

    pub fn panel(&self, parent: &Panel) -> Panel {
        parent.child(self.x, self.y, self.w, self.h)
    }

    /// `paintBackground` (`0x1005a370`): the armed background, then the orange outline; `paint`
    /// (`0x1005a2f0`): the text, west-aligned, in the armed or normal foreground and the
    /// "Primary Button Text" font (`Scheme::sf_primary1`).
    pub fn paint(&self, s: &mut dyn Surface, parent: &Panel, schemes: &Schemes, vs: &ViewportScheme) {
        if !self.visible {
            return;
        }
        let p = self.panel(parent);
        if self.armed {
            p.fill_rect(s, 0, 0, p.w, p.h, vs.primary2);
        }
        p.outlined_rect(s, 0, 0, p.w, p.h, vs.secondary1);
        let Some(font) = schemes.font_of("Primary Button Text") else { return };
        let text = self.label();
        let (tw, th) = font.text_size(&text);
        let (x, y) = label_text_pos(Align::West, p.w, p.h, tw, th);
        p.text(s, font, x, y, if self.armed { vs.secondary2 } else { vs.primary1 }, &text, tw);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_button_text() {
        let mut b = CommandButton::new(b"Specialists", 0, 0, 10, 10);
        assert_eq!(b.label(), b"     Specialists");
        b.key = b'1';
        assert_eq!(b.label(), b"  1  Specialists");
        b.set_text(&[b'x'; 40]);
        assert_eq!((b.text.len(), b.label().len()), (31, 31));
        assert_eq!(team_color(6, 0), Color::new(125, 165, 210, 0));
    }
}
