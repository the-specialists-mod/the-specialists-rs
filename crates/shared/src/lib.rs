//! Code that must behave identically on server and client.

pub mod pm;
pub mod protocol;

pub const GAME_NAME: &std::ffi::CStr = c"MyMod";

pub const VEC_HULL_MIN: [f32; 3] = [-16.0, -16.0, -36.0];
pub const VEC_HULL_MAX: [f32; 3] = [16.0, 16.0, 36.0];
pub const VEC_DUCK_HULL_MIN: [f32; 3] = [-16.0, -16.0, -18.0];
pub const VEC_DUCK_HULL_MAX: [f32; 3] = [16.0, 16.0, 18.0];
pub const VEC_VIEW: [f32; 3] = [0.0, 0.0, 28.0];
