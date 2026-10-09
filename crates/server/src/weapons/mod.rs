//! Server half of TS weapons: the per-player CTSGun inventory, firing, hit resolution,
//! penetration, grenades, knives, kung fu. Predicted logic is shared::weapons.
//! Owner: weapons work package. Reference: decomp docs/server/weapons.md.

use crate::commands::Args;
use crate::player::Player;
use crate::world::Cx;
use goldsrc::sys::{clientdata_t, usercmd_t, weapon_data_t};

#[derive(Default)]
pub struct Inventory {}

pub fn game_init() {}
pub fn precache() {}
pub fn on_spawn(_p: &mut Player, _cx: &mut Cx) {}
pub fn pre_think(_p: &mut Player, _cx: &mut Cx) {}
pub fn post_think(_p: &mut Player, _cx: &mut Cx) {}
pub fn on_killed(_p: &mut Player, _cx: &mut Cx) {}
pub fn client_command(_p: &mut Player, _cx: &mut Cx, _cmd: &str, _args: &Args) -> bool {
    false
}
pub fn fill_client_data(_p: &mut Player, _cx: &mut Cx, _cd: &mut clientdata_t) {}

/// `pfnGetWeaponData`: fill up to 32 `weapon_data_t`. Return 1.
pub fn get_weapon_data(_p: &mut Player, _cx: &mut Cx, info: &mut [weapon_data_t; 32]) -> i32 {
    *info = unsafe { std::mem::zeroed() };
    1
}

/// `pfnCmdStart`: remember the shared random seed for this usercmd.
pub fn cmd_start(_p: &mut Player, _cx: &mut Cx, _cmd: &usercmd_t, _seed: u32) {}

// ---- API for other subsystems (rules: loadouts, ground weapons, power-ups) -------------------

/// Gives weapon `id` with attachment bits. Returns false if it doesn't fit.
pub fn give_weapon(_p: &mut Player, _cx: &mut Cx, _id: u8, _attachments: u8) -> bool {
    false
}
/// Gives `rounds` of ammo type `ammo`. Returns rounds actually added.
pub fn give_ammo(_p: &mut Player, _cx: &mut Cx, _ammo: u8, _rounds: i32) -> i32 {
    0
}
/// Removes every weapon (death, spectate).
pub fn strip_all(_p: &mut Player, _cx: &mut Cx) {}
/// Infinite Clip power-up: double the current gun's clip. False if no gun.
pub fn multiply_clip(_p: &mut Player, _cx: &mut Cx) -> bool {
    false
}
