//! TS 2.1's studio renderer (`ClientRules::studio_style`, issue #256): `HUD_GetStudioModelInterface`
//! 0x10043740 hands out {1, `StudioDrawModel` 0x10065d50, `StudioDrawPlayer` 0x10066f60}. The
//! `CGameStudioModelRenderer` vtable 0x1026e7bc (base 0x102709cc) has the SDK's 25 slots: no
//! dead-player or ghost draws. 2.1 has TS's gait changes (`StudioEstimateGait` 0x10066300: 2-D,
//! ×0.75 below 180; `StudioProcessGait` 0x10066960: × `curstate.framerate`) and the
//! `Bip01 Dorso`/`Bip01 Spine` split (`StudioSetupBones` 0x10064ce0), which the core renderer
//! always runs.

use ts_client::studio::StudioStyle;

/// The SDK's renderer apart from the gait:
/// * `StudioDrawModel` 0x10065d50 leaves the view model's framerate alone;
/// * `StudioSetupBones` 0x10064ce0 has no torso yaw fix after the split loop;
/// * `StudioDrawPlayer` 0x10066f60 doesn't force `renderamt`;
/// * renderfx 17 (0x10065da0..) always draws a corpse through `StudioDrawPlayer` (slot 3) on the
///   player's own `player_info`;
/// * `StudioRenderFinal_Hardware` 0x10068310 has no `r_drawentities 5` case;
/// * `HUD_StudioEvent` 0x1007b780 plays 5004 with `pfnPlaySoundByNameAtLocation(options, 1.0,
///   attachment[0])` (`cl_enginefuncs` slot 63 at 0x102e1554, issue #257).
pub const STYLE: StudioStyle = StudioStyle {
    view_model_slowmo: false,
    torso_yaw_fix: false,
    player_renderamt: false,
    dead_player_copy: false,
    hw_abs_bbox: false,
    slowed_event_sound: false,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sdk_renderer_with_the_gait() {
        let s = STYLE;
        assert!(!s.view_model_slowmo, "0x10065d50: no view-model rate");
        assert!(!s.torso_yaw_fix, "0x10064ce0: no QuaternionAngles at the split bone");
        assert!(!s.player_renderamt, "0x10066f60: renderamt untouched");
        assert!(!s.dead_player_copy, "0x10065da0: corpses through StudioDrawPlayer, no ghosts");
        assert!(!s.hw_abs_bbox, "0x10068310: no r_drawentities 5 case");
        assert!(!s.slowed_event_sound, "0x1007b780: 5004 through PlaySoundByNameAtLocation");
    }
}
