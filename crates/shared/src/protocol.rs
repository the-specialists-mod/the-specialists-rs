//! User message names and layouts. Both sides use these so they can't drift apart.

use std::ffi::CStr;

/// `Health`: one byte, the player's current health (clamped to 0..=255).
pub const MSG_HEALTH: &CStr = c"Health";
pub const MSG_HEALTH_SIZE: i32 = 1;
