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
//! port has no VGUI1 (see client-hud.md §7), so the panel is drawn in immediate mode: a black fill
//! at VGUI alpha 100 (`CTransparentPanel`) and the console font in the schemes' orange
//! (255, 170, 0) instead of the scheme's Arial bitmap fonts.

use super::with;
use crate::hud::text::localise;
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

/// Paints the panel (the VGUI pass, after the HUD).
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
    e.fill(0, y, w, tall, [0, 0, 0], 255 - 100);
    let fg = [1.0, 170.0 / 255.0, 0.0];
    for (text, ly) in [(top, y), (help, y + yres(h, 25.0))] {
        let text = text.trim_end_matches(['\r', '\n']);
        let (tw, _) = e.console_string_size(text);
        e.draw_console_string((w - tw) / 2, ly, fg, text);
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
