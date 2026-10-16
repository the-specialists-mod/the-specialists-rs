//! TS 3.0's studio-renderer choices (`ClientRules::studio_style`): every change the core renderer
//! (`ts_client::studio`) ports from `CStudioModelRenderer` (vtable `0x1007d5f8`, 27 slots).

use ts_client::studio::StudioStyle;

/// All on:
/// * the view-model rate: `StudioDrawModel` `0x10031b44..0x10031bf4`;
/// * the torso yaw fix: `StudioSetupBones` `0x10031616..0x1003166a`;
/// * `renderamt = 255`: `StudioDrawPlayer` `0x10032540`;
/// * the dead and ghost draws: slots 4 and 5, `0x10033540` and `0x100337f0`, picked at
///   `0x10031c9a..`;
/// * the hardware abs bbox: `StudioRenderFinal_Hardware` `0x10034750`.
pub const STYLE: StudioStyle = StudioStyle::ALL;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_renderer_change() {
        assert_eq!(
            STYLE,
            StudioStyle { view_model_slowmo: true, torso_yaw_fix: true, player_renderamt: true, dead_player_copy: true, hw_abs_bbox: true }
        );
    }
}
