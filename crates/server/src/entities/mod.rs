//! Map entities. `link_entities!` exports one C symbol per classname (the engine looks them up
//! by name). Stock Half-Life classes live in `stock`, TS classes in `ts`.

pub mod stock;
pub mod ts;

use crate::world::{self, UseType};
use goldsrc::edict::EdictPtr;
use goldsrc::server::engine;
use goldsrc::sys::*;
use goldsrc::util::parse_vec3;
use std::ffi::CStr;

/// Exports `$class(entvars_t*)` for each classname, creating a `$ty` on the edict.
macro_rules! link_entities {
    ($($class:ident => $ty:ty),* $(,)?) => {$(
        goldsrc::export_entity_class!($class, $crate::world::create::<$ty>);
    )*};
}
pub(crate) use link_entities;

/// Generic entvars keyvalues, used when the class doesn't handle a key. The engine parses none
/// of these itself (not even `classname`).
pub fn entvars_key_value(v: &mut entvars_t, key: &CStr, value: &CStr) -> bool {
    let eng = engine();
    let text = value.to_str().unwrap_or("");
    let f = || text.trim().parse::<f32>().unwrap_or(0.0);
    let i = || text.trim().parse::<i32>().unwrap_or(0);
    match key.to_bytes() {
        b"classname" => v.classname = eng.alloc_string(value),
        b"model" => v.model = eng.alloc_string(value),
        b"targetname" => v.targetname = eng.alloc_string(value),
        b"target" => v.target = eng.alloc_string(value),
        b"netname" => v.netname = eng.alloc_string(value),
        b"message" => v.message = eng.alloc_string(value),
        b"globalname" => v.globalname = eng.alloc_string(value),
        b"noise" => v.noise = eng.alloc_string(value),
        b"noise1" => v.noise1 = eng.alloc_string(value),
        b"noise2" => v.noise2 = eng.alloc_string(value),
        b"noise3" => v.noise3 = eng.alloc_string(value),
        b"origin" => v.origin = parse_vec3(text),
        b"angles" => v.angles = parse_vec3(text),
        b"rendercolor" => v.rendercolor = parse_vec3(text),
        b"rendermode" => v.rendermode = i(),
        b"renderfx" => v.renderfx = i(),
        b"renderamt" => v.renderamt = f(),
        b"skin" => v.skin = i(),
        b"body" => v.body = i(),
        b"spawnflags" => v.spawnflags = i(),
        b"health" => v.health = f(),
        b"max_health" => v.max_health = f(),
        b"speed" => v.speed = f(),
        b"dmg" => v.dmg = f(),
        b"scale" => v.scale = f(),
        b"framerate" => v.framerate = f(),
        b"frame" => v.frame = f(),
        b"sequence" => v.sequence = i(),
        b"team" => v.team = i(),
        b"gravity" => v.gravity = f(),
        b"friction" => v.friction = f(),
        _ => return false,
    }
    true
}

/// Fires `use_` on every entity whose targetname is `target` (SDK `FireTargets`).
pub fn fire_targets(target: &CStr, activator: *mut edict_t, caller: *mut edict_t, kind: UseType, value: f32) {
    if target.is_empty() {
        return;
    }
    let mut e = std::ptr::null_mut();
    loop {
        e = engine().find_by_string(e, c"targetname", target);
        if e.is_null() {
            break;
        }
        if e.v().flags & FL_KILLME as i32 != 0 {
            continue; // already removed this frame
        }
        world::dispatch(e, |ent, cx| ent.use_(cx, activator, caller, kind, value));
    }
}

/// Fires the entity's own `target` (SDK `SUB_UseTargets` without delay/killtarget, which
/// stock::Delay adds).
pub fn use_targets(pev: &entvars_t, activator: *mut edict_t, caller: *mut edict_t, kind: UseType, value: f32) {
    if pev.target != 0 {
        fire_targets(engine().string(pev.target), activator, caller, kind, value);
    }
}

/// Removes an entity at the end of the frame (SDK `UTIL_Remove`): safe from inside callbacks.
pub fn remove(e: *mut edict_t) {
    if e.is_null() {
        return;
    }
    let v = e.v();
    v.flags |= FL_KILLME as i32;
    v.targetname = 0;
}
