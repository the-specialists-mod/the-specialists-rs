//! TS 3.0's studio-renderer choices (`ClientRules::studio_style`): every change the core renderer
//! (`ts_client::studio`) ports from `CStudioModelRenderer` (vtable `0x1007d5f8`, 27 slots).

use ts_client::studio::StudioStyle;

/// All on:
/// * the view-model rate: `StudioDrawModel` `0x10031b44..0x10031bf4`;
/// * the torso yaw fix: `StudioSetupBones` `0x10031616..0x1003166a`;
/// * `renderamt = 255`: `StudioDrawPlayer` `0x10032540`;
/// * the dead and ghost draws: slots 4 and 5, `0x10033540` and `0x100337f0`, picked at
///   `0x10031c9a..`;
/// * the hardware abs bbox: `StudioRenderFinal_Hardware` `0x10034750`;
/// * the slowed 5004 sound: `HUD_StudioEvent` `0x10044b50`;
/// * the view model's zoom push: `V_CalcNormalRefdef` `0x100685a7..0x100685bb`.
pub const STYLE: StudioStyle = StudioStyle::ALL;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_renderer_change() {
        let s = STYLE;
        assert!(s.view_model_slowmo && s.torso_yaw_fix && s.player_renderamt && s.dead_player_copy && s.hw_abs_bbox);
        assert!(s.slowed_event_sound, "0x10044b69: EV_PlaySound with the slow-motion pitch");
        assert!(s.view_model_fov_push, "0x100685a7: (80 - m_iFOV) * 0.15 along the view");
    }
}
