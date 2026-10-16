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
/// panel is hidden. `mode` is the number the panel shows (the observer mode in TS 3.0), and the
/// `#Directed ` prefix follows the observer mode. `name` is the target's name when it is a
/// connected player.
fn texts(mode: i32, observer_mode: i32, shows: bool, spectate_only: bool, name: Option<&str>) -> (Option<String>, String) {
    let help = if spectate_only { "#Spec_Only_Help".to_owned() } else { format!("#Spec_TSHelp{mode}") };
    if mode == 0 || !shows {
        return (None, help);
    }
    let mut text = match name {
        Some(n) => format!("#Spec_Mode{mode} : {n}"),
        None => format!("#Spec_Mode{mode}"),
    };
    if observer_mode == shared::ts::obs::LOCKED_VIEW {
        text = format!("#Directed {text}");
    }
    (Some(text), help)
}

/// `UpdateSpectatorPanel` (`0x10063870`).
pub(super) fn update() {
    let (mode, target) = state::with(|s| (s.spec_mode, s.spec_target));
    let nohud_value = crate::input::cvars().nohud.value();
    // The `ftol`'d value is only the viewport's change key (`+0x228`).
    let nohud = nohud_value as i32;
    let e = engine();
    // 0x10063952: `g_PlayerInfoList[target].name`, the engine's raw bytes (one char per byte
    // here, as the localiser and the label draw take them).
    let name = (target >= 1 && target <= e.max_clients())
        .then(|| crate::hud::saytext::string(&e.player_info(target).name_bytes))
        .filter(|n| !n.is_empty());
    let rules = crate::gameplay::current();
    let only = spec::is_spectate_only();
    // A ruleset may number an HLTV viewer's line by a mode of its own (TS 2.1: its main mode).
    let number = if only { rules.hltv_panel_mode().unwrap_or(mode) } else { mode };
    let (mode_text, help) = texts(number, mode, rules.spectator_panel_shows(nohud_value), only, name.as_deref());
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

/// The bar's height on a screen `h` tall (`YRES(40)`), for what must stay clear of it.
pub(crate) fn height(h: i32) -> i32 {
    yres(h, 40.0)
}

/// `YRES(n)`: `n` 480ths of the screen height (TS truncates with `__ftol`), in the client's
/// x87 arithmetic ([`vgui::ys`]).
fn yres(h: i32, n: f64) -> i32 {
    vgui::ftol(vgui::ys(h) * n)
}

/// The bar's top: `ScreenHeight - YRES(40)` (`0x1006446a`: `fsub` of the unrounded product,
/// then `__ftol`), so at 480 it is 439 (the product is a hair above 40).
fn bar_top(h: i32) -> i32 {
    vgui::ftol(h as f64 - vgui::ys(h) * 40.0)
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
    let y = bar_top(h);
    e.fill_blend(0, y, w, tall, [0, 0, 0], 255 - 100);
    let labels = [(top, "Primary Button Text", y, yres(h, 25.0)), (help, "Briefing Text", y + yres(h, 25.0), yres(h, 15.0))];
    for (text, scheme, ly, lh) in labels {
        let bytes = crate::hud::saytext::bytes(&text);
        let fg = vgui::fg(scheme);
        let drawn = vgui::text_size(scheme, &bytes)
            .is_some_and(|(tw, _)| vgui::draw_text_clipped(scheme, north_x(w, tw), ly, fg, &bytes, (0, ly, w, lh)));
        if !drawn {
            // No bitmap font at all (the game files are missing): the console font, centred.
            let text = crate::hud::saytext::bytes(text.trim_end_matches(['\r', '\n']));
            let (tw, _) = e.console_bytes_size(&text);
            let c = [fg[0] as f32 / 255.0, fg[1] as f32 / 255.0, fg[2] as f32 / 255.0];
            e.draw_console_bytes(north_x(w, tw), ly, Some(c), &text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_texts() {
        assert_eq!(texts(1, 1, true, false, Some("bob")), (Some("#Spec_Mode1 : bob".into()), "#Spec_TSHelp1".into()));
        assert_eq!(texts(3, 3, true, false, None), (Some("#Spec_Mode3".into()), "#Spec_TSHelp3".into()));
        assert_eq!(texts(4, 4, true, true, Some("x")).0.as_deref(), Some("#Directed #Spec_Mode4 : x"));
        assert_eq!(texts(4, 4, true, true, None).1, "#Spec_Only_Help");
        assert_eq!(texts(0, 0, true, false, None).0, None, "not observing");
        assert_eq!(texts(5, 5, false, false, None).0, None, "nohud");
        // A ruleset's own number (2.1's HLTV main mode): the line and the visibility follow it,
        // `#Directed` the observer mode (2.1's map main mode 5 is observer mode 4).
        assert_eq!(texts(5, 4, true, true, None).0.as_deref(), Some("#Directed #Spec_Mode5"));
        assert_eq!(texts(4, 5, true, true, None).0.as_deref(), Some("#Spec_Mode4"));
        assert_eq!(texts(0, 4, true, true, None).0, None, "main mode 0: hidden");
        assert_eq!((yres(480, 40.0), yres(600, 25.0), yres(768, 15.0)), (40, 31, 24));
    }

    /// Issue #265: TS 3.0's bar (`0x10064410`) at 53-bit precision: `YRES(40)` tall, its top at
    /// `ftol(h - h × (1/480)f × 40)`, a pixel above `h - YRES(40)` at most sizes.
    #[test]
    fn bar_geometry() {
        for (h, tall, y, y25, y15) in [
            (480, 40, 439, 25, 15),
            (600, 50, 549, 31, 18),
            (720, 60, 659, 37, 22),
            (768, 64, 703, 40, 24),
            (800, 66, 733, 41, 25),
            (1080, 90, 989, 56, 33),
        ] {
            assert_eq!((height(h), bar_top(h), yres(h, 25.0), yres(h, 15.0)), (tall, y, y25, y15), "{h}");
        }
    }
}
