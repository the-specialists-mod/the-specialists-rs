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
/// * `StudioRenderFinal_Hardware` 0x10068310 has no `r_drawentities 5` case.
pub const STYLE: StudioStyle =
    StudioStyle { view_model_slowmo: false, torso_yaw_fix: false, player_renderamt: false, dead_player_copy: false, hw_abs_bbox: false };

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sdk_renderer_with_the_gait() {
        assert_eq!(
            STYLE,
            StudioStyle {
                view_model_slowmo: false,
                torso_yaw_fix: false,
                player_renderamt: false,
                dead_player_copy: false,
                hw_abs_bbox: false
            }
        );
    }
}
