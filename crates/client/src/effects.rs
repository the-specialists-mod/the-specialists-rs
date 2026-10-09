//! Entity/temp-entity hooks, TS particle system and map emitters, laser sights. Owner:
//! rendering/prediction work package.
//! Reference: decomp docs/client/rendering-effects-prediction.md §3, §5.

use goldsrc::sys::cl_entity_t;
use std::ffi::{c_char, c_void};

pub fn init() {}
pub fn vid_init() {}
pub fn add_entity(_kind: i32, _ent: *mut cl_entity_t, _model: *const c_char) -> bool {
    true
}
pub fn create_entities() {}
pub fn draw_normal() {}
pub fn draw_transparent() {}
pub fn studio_event(_event: *const c_void, _ent: *const cl_entity_t) {}
#[allow(clippy::too_many_arguments)]
pub fn temp_ent_update(_frametime: f64, _client_time: f64, _gravity: f64, _free: *mut *mut c_void, _active: *mut *mut c_void,
                       _add_visible: *mut c_void, _play_sound: *mut c_void) {
}
