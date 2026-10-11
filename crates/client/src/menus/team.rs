//! TS's VGUI1 team menu, `CTeamMenuPanel` (`0x124` bytes, constructor `0x100655d0`, created by
//! `CreateTeamMenu` `0x100642c0` as a `CMenuPanel(100, …)` over the whole screen): the SDK's team
//! menu without the spectate button. Drawn with engine fills ([`crate::hud::vgui`]).
//!
//! * "Select Team" in "Title Font" at (`XRES(40)`, `YRES(32)`).
//! * A window at (`XRES(176)`, `YRES(80)`), `XRES(424)`×`YRES(312)`, with an orange `LineBorder`:
//!   the map's name in "Title Font", the map's `maps/<map>.txt` in "Briefing Text" (else
//!   "#Map_Description_not_available") in a `CTFScrollPanel` (its bars automatic, validated at
//!   the end of every `Update`), and the hovered team's info ("<team>: N Player(s) (F points)"
//!   and its other players) in "Team Info Text" in the team colour. The default text's panel is
//!   wider and taller than the scroll panel's client clip, so without a map briefing TS shows
//!   both scroll bars.
//! * A column of `CommandButton`s at `XRES(40)` from `YRES(80)`, `XRES(124)`×`YRES(24)` and
//!   `YRES(8)` apart: one per team (`TeamNames`, at most four, bound to their digits), "Auto
//!   assign" (5), and "#Menu_Cancel", which TS always shows (`Update` `0x10066020`).
//! * Hovering a team button shows its info and arms it (`SetActiveInfo` `0x10066690`); clicking
//!   or its digit (`SlotInput` `0x100665a0`) sends `jointeam <n>` and closes the menu
//!   (`CMenuHandler_StringCommandWatch` `0x10064a20`).

use super::scoreboard::PlayerSlot;
use super::scroll::ScrollPanel;
use super::widgets::{CommandButton, ViewportScheme, team_color, transparent_background};
use crate::hud::state::PlayerExtra;
use crate::hud::vgui::{Align, Color, Image, Panel, Schemes, Surface, ftol, label_content_size, label_text_pos, spill, xs, ys};
use std::rc::Rc;

/// Everything `Update` reads.
pub struct Inputs<'a> {
    /// The viewport's team names (`TeamNames`, localised as the handler stores them).
    pub team_names: &'a [String],
    pub players: &'a [PlayerSlot],
    pub extra: &'a [PlayerExtra],
    /// `g_TeamInfo[n].frags`: the scoreboard's team slot `n` (TS indexes it by team number).
    pub team_frags: &'a dyn Fn(usize) -> i16,
    pub level: &'a str,
    pub load: &'a dyn Fn(&str) -> Option<Vec<u8>>,
    /// The "Briefing Text" font's text size (to size the briefing panel).
    pub briefing_size: &'a dyn Fn(&[u8]) -> (i32, i32),
    pub w: i32,
    pub h: i32,
}

/// What a click or slot did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// `ClientCmd(cmd)` and close the menu.
    Command(String),
    /// `HideTopMenu`.
    Close,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TeamMenu {
    /// `m_pButtons[1..=5]` (index 0 unused).
    pub buttons: [CommandButton; 6],
    pub cancel: CommandButton,
    /// `m_pTeamInfoPanel[1..=5]` texts and visibility.
    pub info: [Vec<u8>; 6],
    pub info_visible: [bool; 6],
    /// `m_iCurrentInfo`.
    pub current_info: i32,
    /// `m_bUpdatedMapName`.
    pub updated_map: bool,
    pub map_title: Vec<u8>,
    /// The briefing text and its panel's size: the constructor's, or the map file's text size.
    pub briefing: Vec<u8>,
    pub briefing_size: (i32, i32),
    /// `m_pScrollPanel`, in the window; its client holds the briefing panel.
    pub scroll: ScrollPanel,
    /// The button under the cursor (`cursorEntered`/`cursorExited`): 1..=5, 6 = cancel.
    hover: Option<usize>,
}

fn team_name(names: &[String], i: usize) -> Vec<u8> {
    names.get(i - 1).map(|n| crate::hud::saytext::bytes(n)).unwrap_or_default()
}

impl TeamMenu {
    /// The constructor's buttons and texts for a `w`×`h` screen; `lookup` is
    /// `BufferedLocaliseTextString`.
    pub fn new(w: i32, h: i32, lookup: &dyn Fn(&str) -> String) -> TeamMenu {
        let (xsc, ysc) = (xs(w), ys(h));
        let (bx, bw, bh) = (ftol(xsc * 40.0), ftol(xsc * 124.0), ftol(ysc * 24.0));
        let buttons = std::array::from_fn(|i| {
            let y = ftol(ysc * 80.0 + (ysc * 24.0 + ysc * 8.0) * i as f64);
            let mut b = CommandButton::new(b"", bx, y, bw, bh);
            b.visible = false;
            if i == 5 {
                b.key = b'5';
                b.set_text(&crate::hud::saytext::bytes(&lookup("Auto assign")));
                b.visible = true;
            }
            b
        });
        let cancel = CommandButton::new(&crate::hud::saytext::bytes(&lookup("#Menu_Cancel")), bx, 0, bw, bh);
        // The scroll panel at (`XRES(16)`, `YRES(48)`) in the window, its bars set automatic
        // before the briefing panel (`XRES(424) - XRES(16)`×`YRES(178)`) joins its client.
        let x16 = spill(xsc * 16.0);
        let mut scroll = ScrollPanel::ctf(ftol(x16), ftol(ysc * 48.0), ftol(xsc * 424.0 - x16 * 2.0), ftol(ysc * 178.0));
        scroll.set_scroll_bar_visible(false, false);
        let briefing_size = (ftol(xsc * 424.0 - xsc * 16.0), ftol(ysc * 178.0));
        scroll.content = Some(briefing_size);
        let mut m = TeamMenu {
            buttons,
            cancel,
            info: Default::default(),
            info_visible: [true; 6],
            current_info: 0,
            updated_map: false,
            map_title: Vec::new(),
            briefing: crate::hud::saytext::bytes(&lookup("#Map_Description_not_available")),
            briefing_size,
            scroll,
            hover: None,
        };
        m.initialize();
        m
    }

    /// `Initialize` (`0x10065ff0`, also from the viewport's `ReInitialize` each `VidInit`).
    pub fn initialize(&mut self) {
        self.updated_map = false;
        self.current_info = 0;
    }

    /// `Reset` (`0x10066730`, from `ShowVGUIMenu` before the menu is queued).
    pub fn reset(&mut self) {
        self.current_info = 0;
    }

    /// `SetActiveInfo` (`0x10066690`): arm button `n` and show its info.
    pub fn set_active_info(&mut self, n: usize) {
        for i in 1..=5 {
            self.buttons[i].armed = false;
            self.info_visible[i] = false;
        }
        if (1..=5).contains(&n) {
            self.buttons[n].armed = true;
            self.info_visible[n] = true;
        }
        self.current_info = n as i32;
    }

    /// `Update` (`0x10066020`).
    pub fn update(&mut self, inp: &Inputs) {
        let (xsc, ysc) = (xs(inp.w), ys(inp.h));
        let x = ftol(xsc * 40.0);
        let step = ysc * 24.0 + ysc * 8.0;
        let mut y = ftol(ysc * 80.0);
        for i in 1..=4 {
            if i > inp.team_names.len() {
                self.buttons[i].visible = false;
                continue;
            }
            let name = team_name(inp.team_names, i);
            let b = &mut self.buttons[i];
            b.set_text(&name);
            b.key = b'0' + i as u8;
            b.visible = true;
            (b.x, b.y) = (x, y);
            y = ftol(y as f64 + step);
            if self.current_info == 0 {
                self.set_active_info(i);
            }
            // The team's other players: "\n" then the names, ", " between them.
            let mut list = b"\n".to_vec();
            let mut total = 0;
            for j in 1..64 {
                let p = &inp.players[j];
                let Some(n) = &p.name else { continue };
                if p.this_player || inp.extra[j].team != i as i32 {
                    continue;
                }
                total += 1;
                if total > 1 {
                    list.extend_from_slice(b", ");
                }
                list.extend_from_slice(n);
                list.truncate(0x43c);
            }
            self.info[i] = if total > 0 {
                let word = if total == 1 { "Player" } else { "Players" };
                let mut s = name.clone();
                s.extend_from_slice(format!(": {total} {word} ({} points)", (inp.team_frags)(i)).as_bytes());
                s.extend_from_slice(&list);
                s.truncate(0x53c);
                s
            } else {
                Vec::new()
            };
        }
        (self.buttons[5].x, self.buttons[5].y) = (x, y);
        y = ftol(y as f64 + step);
        (self.cancel.x, self.cancel.y) = (x, y);
        self.cancel.visible = true;
        if !self.updated_map && !inp.level.is_empty() {
            let level = inp.level;
            // `strchr` for the first '/' (else '\'), then up to the first '.'.
            let file = level.split_once('/').or_else(|| level.split_once('\\')).map_or(level, |(_, f)| f);
            self.map_title = crate::hud::saytext::bytes(file.split('.').next().unwrap_or(file));
            let base = level.split('.').next().unwrap_or(level);
            if let Some(text) = (inp.load)(&format!("{base}.txt")) {
                let text: Vec<u8> = text.into_iter().take_while(|&c| c != 0).collect();
                self.briefing_size = (inp.briefing_size)(&text);
                self.scroll.content = Some(self.briefing_size);
                self.briefing = text;
            }
            self.updated_map = true;
        }
        self.scroll.validate();
    }

    /// `SlotInput` (`0x100665a0`): 5 is auto-assign; 1..teams fires a visible team button.
    pub fn slot_input(&self, n: i32, teams: usize) -> Option<Action> {
        if n == 5 || (n >= 1 && n as usize <= teams && self.buttons[n as usize].visible) {
            return Some(Action::Command(format!("jointeam {n}")));
        }
        None
    }

    fn button_panels(&self, menu: &Panel) -> impl Iterator<Item = (usize, Panel)> + '_ {
        let menu = *menu;
        (1..=5)
            .map(|i| (i, &self.buttons[i]))
            .chain(std::iter::once((6, &self.cancel)))
            .filter(|(_, b)| b.visible)
            .map(move |(i, b)| (i, b.panel(&menu)))
    }

    /// The window panel (`m_pTeamWindow`), which holds the scroll panel.
    fn window(w: i32, h: i32) -> Panel {
        let (xsc, ysc) = (xs(w), ys(h));
        let menu = Panel::root(w, h).child(0, 0, w, h);
        menu.child(ftol(spill(xsc) * 176.0), ftol(ysc * 80.0), ftol(xsc * 424.0), ftol(ysc * 312.0)).bordered()
    }

    /// The cursor moved to (`x`, `y`): `cursorEntered`/`cursorExited` on the buttons. A team
    /// button shows its info (`CHandler_MenuButtonOver`); the cancel button arms while hovered
    /// (`CHandler_CommandButtonHighlight`). The briefing's scroll bars follow too.
    pub fn hover(&mut self, w: i32, h: i32, x: i32, y: i32) {
        self.scroll.cursor_moved(&Self::window(w, h), x, y);
        let menu = Panel::root(w, h).child(0, 0, w, h);
        let now = self.button_panels(&menu).filter(|(_, p)| p.is_within(x, y)).map(|(i, _)| i).last();
        if now == self.hover {
            return;
        }
        if self.hover == Some(6) {
            self.cancel.armed = false;
        }
        match now {
            Some(6) => self.cancel.armed = true,
            Some(i) => self.set_active_info(i),
            None => {}
        }
        self.hover = now;
    }

    /// A mouse press: the briefing's scroll bar sliders.
    pub fn press(&mut self, w: i32, h: i32, x: i32, y: i32) {
        self.scroll.mouse_pressed(&Self::window(w, h), x, y);
    }

    /// A mouse release (`left`: the left button): the briefing's scroll bars take theirs, else a
    /// left release fires the button under it.
    pub fn release(&mut self, w: i32, h: i32, x: i32, y: i32, left: bool) -> Option<Action> {
        if self.scroll.mouse_released(&Self::window(w, h), x, y, left) || !left {
            return None;
        }
        self.click(w, h, x, y)
    }

    /// A left-button release at (`x`, `y`): the button under it fires.
    pub fn click(&self, w: i32, h: i32, x: i32, y: i32) -> Option<Action> {
        let menu = Panel::root(w, h).child(0, 0, w, h);
        let hit = self.button_panels(&menu).filter(|(_, p)| p.is_within(x, y)).map(|(i, _)| i).last()?;
        Some(if hit == 6 { Action::Close } else { Action::Command(format!("jointeam {hit}")) })
    }

    /// Paints the menu (`paintBackground` `0x10066640` updates it first while the map name is
    /// unset, then the panel's black at VGUI alpha 100; then its children).
    pub fn paint(&self, s: &mut dyn Surface, sb: &Schemes, w: i32, h: i32, arrows: &[Option<Rc<Image>>; 4]) {
        let (xsc, ysc) = (xs(w), ys(h));
        let vs = ViewportScheme::from(sb);
        let menu = Panel::root(w, h).child(0, 0, w, h);
        transparent_background(s, &menu, 100);
        let title = sb.get("Title Font");
        let tfont = sb.font(title);
        let fitted_label = |s: &mut dyn Surface, parent: &Panel, x: i32, y: i32, text: &[u8]| {
            let Some(f) = tfont else { return };
            let (tw, th) = f.text_size(text);
            let (lw, lh) = label_content_size(tw, th);
            let p = parent.child(x, y, lw, lh);
            p.paint_background(s, title.bg);
            let (tx, ty) = label_text_pos(Align::West, lw, lh, tw, th);
            p.text(s, f, tx, ty, title.fg, text, tw);
        };
        fitted_label(s, &menu, ftol(xsc * 40.0), ftol(ysc * 32.0), b"Select Team");
        // The window: an orange line border, no background (`CTransparentPanel(255)`).
        let win = Self::window(w, h);
        win.line_border(s, Color::new(178, 118, 0, 0));
        fitted_label(s, &win, ftol(xsc * 16.0), ftol(ysc * 16.0), &self.map_title);
        let brief = sb.get("Briefing Text");
        if let (Some(f), Some(p)) = (sb.font(brief), self.scroll.content_panel(&win)) {
            // `TextImage::setText` sizes the text image to the text (after `setFont`), and the
            // map file's panel is set to that size too: the text never wraps.
            let (tw, _) = f.text_size(&self.briefing);
            p.paint_background(s, brief.bg);
            p.text(s, f, 0, 0, brief.fg, &self.briefing, tw);
        }
        self.scroll.paint_bars(s, &win, &vs, arrows);
        if let Some(f) = sb.font_of("Team Info Text") {
            for i in 1..=5 {
                if !self.info_visible[i] {
                    continue;
                }
                let (x424, y234) = (spill(xsc * 424.0), spill(ysc * 234.0));
                let p = win.child(ftol(xsc * 16.0), ftol(y234), ftol(x424 - xsc * 16.0), ftol(x424 - y234));
                let (tw, _) = f.text_size(&self.info[i]);
                p.text(s, f, 0, 0, team_color(i as i32, 0), &self.info[i], tw);
            }
        }
        for b in self.buttons[1..].iter().chain(std::iter::once(&self.cancel)) {
            b.paint(s, &menu, sb, &vs);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(name: &str, this: bool) -> PlayerSlot {
        PlayerSlot { name: Some(name.as_bytes().to_vec()), ping: 0, packetloss: 0, this_player: this, entity: true }
    }

    #[test]
    fn update_lays_out_the_buttons() {
        let mut m = TeamMenu::new(640, 480, &|s| s.trim_start_matches('#').to_owned());
        assert_eq!((m.buttons[1].x, m.buttons[1].w, m.buttons[1].h), (40, 124, 24));
        assert!(!m.buttons[1].visible && m.buttons[5].visible && m.buttons[5].key == b'5');
        let names = ["Specialists".to_owned(), "Mercenaries".to_owned()];
        let mut players = vec![PlayerSlot::default(); 65];
        let mut extra = vec![PlayerExtra::default(); 65];
        players[1] = slot("me", true);
        players[2] = slot("bob", false);
        players[3] = slot("eve", false);
        extra[1].team = 1;
        extra[2].team = 1;
        extra[3].team = 1;
        let frags = |i: usize| i as i16 * 10;
        let load = |p: &str| (p == "maps/ts_dojo.txt").then(|| b"Line\nLonger line".to_vec());
        let size = |t: &[u8]| (t.len() as i32, 2);
        let inp = Inputs {
            team_names: &names,
            players: &players,
            extra: &extra,
            team_frags: &frags,
            level: "maps/ts_dojo.bsp",
            load: &load,
            briefing_size: &size,
            w: 640,
            h: 480,
        };
        m.update(&inp);
        assert_eq!((m.buttons[1].y, m.buttons[2].y, m.buttons[5].y, m.cancel.y), (80, 112, 144, 176));
        assert_eq!(m.buttons[1].label(), b"  1  Specialists");
        assert!(!m.buttons[3].visible && m.cancel.visible, "TS always shows cancel");
        assert_eq!(m.current_info, 1, "the first team's info shows first");
        assert!(m.info_visible[1] && !m.info_visible[2] && m.buttons[1].armed);
        assert_eq!(m.info[1], b"Specialists: 2 Players (10 points)\nbob, eve");
        assert!(m.info[2].is_empty());
        assert_eq!((m.map_title.as_slice(), m.briefing_size), (&b"ts_dojo"[..], (16, 2)));
        assert!(!m.scroll.vbar.visible && !m.scroll.hbar.visible, "a small briefing needs no bars");
        assert!(m.updated_map);
        // Input.
        assert_eq!(m.slot_input(2, 2), Some(Action::Command("jointeam 2".into())));
        assert_eq!(m.slot_input(3, 2), None);
        assert_eq!(m.slot_input(5, 2), Some(Action::Command("jointeam 5".into())));
        m.hover(640, 480, 50, 120);
        assert_eq!(m.current_info, 2, "hovering a team button shows its info");
        assert!(m.buttons[2].armed && !m.buttons[1].armed);
        m.hover(640, 480, 50, 180);
        assert!(m.cancel.armed && m.buttons[2].armed, "team buttons stay armed");
        m.hover(640, 480, 300, 300);
        assert!(!m.cancel.armed);
        assert_eq!(m.click(640, 480, 50, 180), Some(Action::Close));
        assert_eq!(m.click(640, 480, 50, 85), Some(Action::Command("jointeam 1".into())));
        assert_eq!(m.click(640, 480, 300, 300), None);
    }

    /// The default text's panel (`XRES(408)`×`YRES(178)`) is larger than the client clip
    /// (`XRES(392)`×`YRES(178)`): both bars show.
    #[test]
    fn default_briefing_shows_both_bars() {
        let m = TeamMenu::new(640, 480, &|s| s.to_owned());
        let mut m = m;
        m.scroll.validate();
        assert_eq!((m.scroll.w, m.scroll.h, m.briefing_size), (392, 178, (408, 178)));
        assert!(m.scroll.vbar.visible && m.scroll.hbar.visible);
        assert_eq!(m.scroll.clip, (376, 162));
    }

    #[test]
    fn button_steps_scale() {
        let m = TeamMenu::new(1024, 768, &|s| s.to_owned());
        assert_eq!((m.buttons[1].x, m.buttons[1].w, m.buttons[1].h, m.buttons[1].y), (64, 198, 38, 179));
    }
}
