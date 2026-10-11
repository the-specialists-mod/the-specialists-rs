//! TS's VGUI1 spectator panel (`TeamFortressViewport` `+0x884`, created by `0x10064410`): a
//! translucent bar across the bottom `YRES(40)` of the screen while observing, with the camera
//! mode (and target) on top and TS's key help under it.
//!
//! `UpdateSpectatorPanel` (`0x10063870`) fills it. The viewport's `paintBackground`
//! (`0x100648c0`) calls it whenever the mode, target or `nohud` changed, and `SetModes` after
//! every HLTV mode change. It shows while observing with `nohud 0`:
//!
//! * top line (`Label "Spectator"`, `YRES(25)` tall, "Primary Button Text" font): `#Spec_Mode%d`,
//!   or `#Spec_Mode%d : <name>` when the target is a connected player; in the director's locked
//!   view (4) prefixed with `#Directed `;
//! * second line (`#Spec_Help_TSTitle` label, `YRES(15)` tall at `YRES(25)`, "Briefing Text"):
//!   `#Spec_TSHelp%d` for the mode, or `#Spec_Only_Help` for HLTV spectators.
//!
//! Both lines are localised with `#word` substitution and centred at the top of their label. The
//! port has no VGUI1, so the panel is painted with engine fills: the `CTransparentPanel`
//! background (black at VGUI alpha 100) and the labels' texts in the schemes' bitmap fonts and
//! foreground colours ([`crate::hud::vgui`]).

use super::with;
use crate::hud::text::localise;
use crate::hud::vgui;
use crate::state;
use goldsrc::client::engine;
use goldsrc::ext::spectator as spec;

/// What the panel shows (the labels' texts, set by `UpdateSpectatorPanel`).
#[derive(Debug, Default, Clone, PartialEq)]
pub(super) struct Panel {
    /// `(mode, target, nohud)` at the last update (viewport `+0x21c`/`+0x220`/`+0x228`).
    key: Option<(i32, i32, i32)>,
    visible: bool,
    mode_text: String,
    help_text: String,
}

/// The unlocalised texts of `UpdateSpectatorPanel`: (mode line, help line), or `None` when the
/// panel is hidden. `name` is the target's name when it is a connected player.
fn texts(mode: i32, nohud: i32, spectate_only: bool, name: Option<&str>) -> (Option<String>, String) {
    let help = if spectate_only { "#Spec_Only_Help".to_owned() } else { format!("#Spec_TSHelp{mode}") };
    if mode == 0 || nohud != 0 {
        return (None, help);
    }
    let mut text = match name {
        Some(n) => format!("#Spec_Mode{mode} : {n}"),
        None => format!("#Spec_Mode{mode}"),
    };
    if mode == shared::ts::obs::LOCKED_VIEW {
        text = format!("#Directed {text}");
    }
    (Some(text), help)
}

/// `UpdateSpectatorPanel` (`0x10063870`).
pub(super) fn update() {
    let (mode, target) = state::with(|s| (s.spec_mode, s.spec_target));
    let nohud = crate::input::cvars().nohud.value() as i32;
    let e = engine();
    let name = (target >= 1 && target <= e.max_clients()).then(|| e.player_info(target).name).filter(|n| !n.is_empty());
    let (mode_text, help) = texts(mode, nohud, spec::is_spectate_only(), name.as_deref());
    with(|s| {
        let p = &mut s.panel;
        p.key = Some((mode, target, nohud));
        p.visible = mode_text.is_some();
        if let Some(t) = mode_text {
            p.mode_text = localise(&t);
            p.help_text = localise(&help);
        }
    });
}

/// The viewport's per-frame check (`0x100648c0`): update on a mode, target or `nohud` change.
pub(super) fn think() {
    let key = state::with(|s| (s.spec_mode, s.spec_target));
    let nohud = crate::input::cvars().nohud.value() as i32;
    if with(|s| s.panel.key) != Some((key.0, key.1, nohud)) {
        update();
    }
}

/// `YRES(n)`: `n` 480ths of the screen height (TS truncates with `__ftol`).
fn yres(h: i32, n: f32) -> i32 {
    (h as f32 * (1.0 / 480.0) * n) as i32
}

/// Where a VGUI1 label with `setContentAlignment(a_north)` puts text `tw` wide in a label `w`
/// wide: horizontally centred (`w / 2 - tw / 2`, each halved separately), at the top.
fn north_x(w: i32, tw: i32) -> i32 {
    w / 2 - tw / 2
}

/// Paints the panel (the VGUI pass, after the HUD): `CTransparentPanel::paintBackground` (black,
/// VGUI alpha 100 = 155/255 opaque), then the two labels' texts in their scheme fonts and
/// foreground colours, each clipped to its label (`0x10064410`: "Primary Button Text" for the
/// `YRES(25)` mode label, "Briefing Text" for the `YRES(15)` help label at `YRES(25)`).
pub(super) fn draw() {
    let (visible, top, help) = with(|s| (s.panel.visible, s.panel.mode_text.clone(), s.panel.help_text.clone()));
    if !visible {
        return;
    }
    let e = engine();
    let sc = e.screen();
    let (w, h) = (sc.width, sc.height);
    let tall = yres(h, 40.0);
    let y = (h as f32 - h as f32 * (1.0 / 480.0) * 40.0) as i32;
    e.fill_blend(0, y, w, tall, [0, 0, 0], 255 - 100);
    let labels = [(top, "Primary Button Text", y, yres(h, 25.0)), (help, "Briefing Text", y + yres(h, 25.0), yres(h, 15.0))];
    for (text, scheme, ly, lh) in labels {
        let bytes = crate::hud::saytext::bytes(&text);
        let fg = vgui::fg(scheme);
        let drawn = vgui::text_size(scheme, &bytes)
            .is_some_and(|(tw, _)| vgui::draw_text_clipped(scheme, north_x(w, tw), ly, fg, &bytes, (0, ly, w, lh)));
        if !drawn {
            // No bitmap font at all (the game files are missing): the console font, centred.
            let text = text.trim_end_matches(['\r', '\n']);
            let (tw, _) = e.console_string_size(text);
            let c = [fg[0] as f32 / 255.0, fg[1] as f32 / 255.0, fg[2] as f32 / 255.0];
            e.draw_console_string(north_x(w, tw), ly, c, text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_texts() {
        assert_eq!(texts(1, 0, false, Some("bob")), (Some("#Spec_Mode1 : bob".into()), "#Spec_TSHelp1".into()));
        assert_eq!(texts(3, 0, false, None), (Some("#Spec_Mode3".into()), "#Spec_TSHelp3".into()));
        assert_eq!(texts(4, 0, true, Some("x")).0.as_deref(), Some("#Directed #Spec_Mode4 : x"));
        assert_eq!(texts(4, 0, true, None).1, "#Spec_Only_Help");
        assert_eq!(texts(0, 0, false, None).0, None, "not observing");
        assert_eq!(texts(5, 1, false, None).0, None, "nohud");
        assert_eq!((yres(480, 40.0), yres(600, 25.0), yres(768, 15.0)), (40, 31, 24));
    }
}
