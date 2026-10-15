//! The strategy states (bots.md §6): the helpers the rulesets' state thinks share. One state per
//! TS game mode, chosen at spawn (`Brain::state`); the state's think is the rules' hook
//! ([`super::rules::NativeBotRules::state_think`]).

use crate::player::Player;
use goldsrc::edict::EdictPtr;
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::ffi::CString;

/// `BotArmamentEmpty` (0x1006e4c0): the first loadout entry's id is 0 (`pl+0x594`).
pub fn armament_empty(p: &Player) -> bool {
    p.inv.sv.armament.first().is_none_or(|a| a.id == 0)
}

/// `UTIL_FindEntityByClassname` over the whole edict list.
pub fn find_by_class(start: *mut edict_t, class: &str) -> *mut edict_t {
    let Ok(c) = CString::new(class) else { return std::ptr::null_mut() };
    let e = engine().find_by_string(start, c"classname", &c);
    if e.is_null() || e.is_free() { std::ptr::null_mut() } else { e }
}

/// `TSFoundRandomEntity(class)` (0x10073a30): up to 32 entities of the class, then
/// `RANDOM_LONG(0, n-1)` (no draw when there are none).
pub fn random_entity(class: &str) -> *mut edict_t {
    let mut list = Vec::new();
    let mut e = std::ptr::null_mut();
    while list.len() < 32 {
        e = find_by_class(e, class);
        if e.is_null() {
            break;
        }
        list.push(e);
    }
    if list.is_empty() {
        return std::ptr::null_mut();
    }
    let n = list.len() as i32;
    list[engine().random_long(0, n - 1).clamp(0, n - 1) as usize]
}

/// `CBaseEntity::Center` (vtable slot 60): the middle of the absolute box.
pub fn center(e: *mut edict_t) -> [f32; 3] {
    let v = e.v();
    [(v.absmin[0] + v.absmax[0]) * 0.5, (v.absmin[1] + v.absmax[1]) * 0.5, (v.absmin[2] + v.absmax[2]) * 0.5]
}
