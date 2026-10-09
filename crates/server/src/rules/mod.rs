//! Game rules: modes (DM, TDM, LMS, The One), spawning, scoring, power-ups, slow motion
//! propagation, chat, cvars. Owner: rules work package.
//! Reference: decomp docs/server/gamerules-entities.md, architecture.md §2, §4.

use crate::commands::Args;
use crate::player::Player;
use crate::world::{Cx, DamageInfo};
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::ffi::CStr;

#[derive(Default)]
pub struct PlayerRules {}

pub fn game_init() {}
pub fn precache() {}
pub fn server_activate() {}
pub fn server_deactivate() {}
/// Called every server frame (`pfnStartFrame`).
pub fn start_frame() {}

/// worldspawn keyvalues the rules care about (e.g. `gamemode`). Return true if handled.
pub fn world_key_value(_key: &CStr, _value: &CStr) -> bool {
    false
}

pub fn client_connect(_e: *mut edict_t, _name: &CStr, _addr: &CStr) -> bool {
    true
}
pub fn client_disconnect(_e: *mut edict_t) {}
pub fn client_kill(_e: *mut edict_t) {}
pub fn userinfo_changed(_e: *mut edict_t) {}

/// Pick a spawn point and place the player.
pub fn on_spawn(_p: &mut Player, cx: &mut Cx) {
    let e = engine();
    let mut spot = e.find_by_string(std::ptr::null_mut(), c"classname", c"info_player_deathmatch");
    if spot.is_null() {
        spot = e.find_by_string(std::ptr::null_mut(), c"classname", c"info_player_start");
    }
    let (origin, angles) = if spot.is_null() {
        ([0.0, 0.0, 64.0], [0.0; 3])
    } else {
        let s = unsafe { &(*spot).v };
        ([s.origin[0], s.origin[1], s.origin[2] + 1.0], s.angles)
    };
    cx.pev().angles = angles;
    e.set_origin(cx.edict, origin);
}

pub fn pre_think(_p: &mut Player, _cx: &mut Cx) {}
pub fn post_think(_p: &mut Player, _cx: &mut Cx) {}
pub fn on_killed(_p: &mut Player, cx: &mut Cx, _dmg: &DamageInfo) {
    let pev = cx.pev();
    pev.deadflag = DEAD_DEAD as i32;
    pev.takedamage = DAMAGE_NO as f32;
}
pub fn client_command(_p: &mut Player, _cx: &mut Cx, _cmd: &str, _args: &Args) -> bool {
    false
}
pub fn fill_client_data(_p: &mut Player, _cx: &mut Cx, _cd: &mut clientdata_t) {}
