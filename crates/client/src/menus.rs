//! Immediate-mode menus drawn with the engine (buy menu, team select, scoreboard, MOTD). TS draws
//! its buy menu this way too, not with VGUI. Owner: HUD/UI work package.
//! Reference: decomp docs/client/architecture-hud.md §6.

use std::ffi::CStr;

pub fn init() {}
pub fn redraw(_time: f32) {}
/// Returns true if a menu consumed the key.
pub fn key_event(_down: bool, _key: i32, _binding: &CStr) -> bool {
    false
}
