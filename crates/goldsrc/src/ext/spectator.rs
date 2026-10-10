//! Engine helpers for the spectator camera and HUD (`CHudSpectator`, `V_CalcSpectatorRefdef`).
//! Keep additions here to avoid merge conflicts.

use crate::cl_eng;
use crate::sys::*;

/// `IsSpectateOnly`: true for HLTV spectators (the client's spectator globals are its own).
pub fn is_spectate_only() -> bool {
    cl_eng!(IsSpectateOnly()) != 0
}

/// `pfnGetModelByIndex`: the engine's model for a precache index, as a handle for
/// `cl_entity_t::model` (null when the index is unused). Holding the pointer is safe; the engine
/// owns the model for the level.
pub fn model_by_index(index: i32) -> *mut model_s {
    if index <= 0 {
        return std::ptr::null_mut();
    }
    cl_eng!(pfnGetModelByIndex(index))
}

/// `V_CalcShake`: advances the current screen shake (`svc_shake`) for this frame.
pub fn calc_shake() {
    cl_eng!(V_CalcShake())
}

/// `V_ApplyShake`: adds the current screen shake to a view origin and angles.
pub fn apply_shake(origin: &mut [f32; 3], angles: &mut [f32; 3], factor: f32) {
    cl_eng!(V_ApplyShake(origin.as_mut_ptr(), angles.as_mut_ptr(), factor))
}

/// `pfnSetCrosshair`: the engine-drawn crosshair sprite (`hspr` 0 clears it). `rect` is
/// `[left, right, top, bottom]` like `wrect_t`.
pub fn set_crosshair(hspr: HSPRITE, rect: [i32; 4], rgb: [i32; 3]) {
    let rc = wrect_t { left: rect[0], right: rect[1], top: rect[2], bottom: rect[3] };
    cl_eng!(pfnSetCrosshair(hspr, rc, rgb[0], rgb[1], rgb[2]))
}
