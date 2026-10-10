//! Map entities. `link_entities!` exports one C symbol per classname (the engine looks them up
//! by name). Stock Half-Life classes live in `stock`, TS classes in `ts`.

pub mod stock;
pub mod ts;

use crate::world::{self, UseType};
use goldsrc::edict::EdictPtr;
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::ffi::CStr;
use stock::base::{kv_f, kv_i, kv_vec};

/// Exports `$class(entvars_t*)` for each classname, creating a `$ty` on the edict.
macro_rules! link_entities {
    ($($class:ident => $ty:ty),* $(,)?) => {$(
        goldsrc::export_entity_class!($class, $crate::world::create::<$ty>);
    )*};
}
pub(crate) use link_entities;

/// `DispatchKeyValue` (0xaad4c, the SDK's cbase.cpp unchanged): the entvars table first, and the
/// class's `KeyValue` only for keys it didn't claim. `has_class` is the engine's
/// `szClassName != NULL`; without it (or without a game object on the edict) the class isn't asked.
/// Returns `fHandled`.
pub fn dispatch_key_value(e: *mut edict_t, has_class: bool, key: &CStr, value: &CStr) -> bool {
    key_value_order(
        || entvars_key_value(e.v(), key, value),
        has_class,
        || world::dispatch(e, |ent, cx| ent.key_value(cx, key, value)) == Some(true),
    )
}

/// The order `DispatchKeyValue` asks the two parsers in. Split out so it can be tested without an
/// engine.
fn key_value_order(entvars: impl FnOnce() -> bool, has_class: bool, class: impl FnOnce() -> bool) -> bool {
    entvars() || (has_class && class())
}

/// How `EntvarsKeyvalue` (0xef9f0) stores a value, by the `TYPEDESCRIPTION` field type.
#[derive(Clone, Copy)]
enum Field {
    /// `FIELD_STRING`/`FIELD_MODELNAME`/`FIELD_SOUNDNAME`: `ALLOC_STRING`.
    Str(fn(&mut entvars_t) -> &mut string_t),
    /// `FIELD_FLOAT`/`FIELD_TIME`: `atof`.
    Float(fn(&mut entvars_t) -> &mut f32),
    /// `FIELD_FLOAT` declared over an `int` member (`fixangle`, `light_level`, `deadflag`, `flags`):
    /// the SDK table stores the float's bit pattern. TS ships the same table.
    FloatBits(fn(&mut entvars_t) -> &mut i32),
    /// `FIELD_INTEGER`: `atoi`.
    Int(fn(&mut entvars_t) -> &mut i32),
    /// `FIELD_INTEGER` declared over a byte array (`controller`, `blending`): the int's
    /// little-endian bytes, truncated to the array (the C write's extra bytes land in padding).
    Bytes(fn(&mut entvars_t) -> &mut [u8]),
    /// `FIELD_VECTOR`/`FIELD_POSITION_VECTOR`: `UTIL_StringToVector`.
    Vec(fn(&mut entvars_t) -> &mut [f32; 3]),
    /// `FIELD_EDICT`: can't be set from text. The SDK logs "Bad field in entity!!" and still
    /// reports the key handled.
    Edict,
}

/// `gEntvarsDescription` (0x1227c0): the SDK's 86-entry table (util.cpp), which TS ships as is.
const ENTVARS_FIELDS: [(&str, Field); 86] = {
    use Field::*;
    [
        ("classname", Str(|v| &mut v.classname)),
        ("globalname", Str(|v| &mut v.globalname)),
        ("origin", Vec(|v| &mut v.origin)),
        ("oldorigin", Vec(|v| &mut v.oldorigin)),
        ("velocity", Vec(|v| &mut v.velocity)),
        ("basevelocity", Vec(|v| &mut v.basevelocity)),
        ("movedir", Vec(|v| &mut v.movedir)),
        ("angles", Vec(|v| &mut v.angles)),
        ("avelocity", Vec(|v| &mut v.avelocity)),
        ("punchangle", Vec(|v| &mut v.punchangle)),
        ("v_angle", Vec(|v| &mut v.v_angle)),
        ("fixangle", FloatBits(|v| &mut v.fixangle)),
        ("idealpitch", Float(|v| &mut v.idealpitch)),
        ("pitch_speed", Float(|v| &mut v.pitch_speed)),
        ("ideal_yaw", Float(|v| &mut v.ideal_yaw)),
        ("yaw_speed", Float(|v| &mut v.yaw_speed)),
        ("modelindex", Int(|v| &mut v.modelindex)),
        ("model", Str(|v| &mut v.model)),
        ("viewmodel", Str(|v| &mut v.viewmodel)),
        ("weaponmodel", Str(|v| &mut v.weaponmodel)),
        ("absmin", Vec(|v| &mut v.absmin)),
        ("absmax", Vec(|v| &mut v.absmax)),
        ("mins", Vec(|v| &mut v.mins)),
        ("maxs", Vec(|v| &mut v.maxs)),
        ("size", Vec(|v| &mut v.size)),
        ("ltime", Float(|v| &mut v.ltime)),
        ("nextthink", Float(|v| &mut v.nextthink)),
        ("solid", Int(|v| &mut v.solid)),
        ("movetype", Int(|v| &mut v.movetype)),
        ("skin", Int(|v| &mut v.skin)),
        ("body", Int(|v| &mut v.body)),
        ("effects", Int(|v| &mut v.effects)),
        ("gravity", Float(|v| &mut v.gravity)),
        ("friction", Float(|v| &mut v.friction)),
        ("light_level", FloatBits(|v| &mut v.light_level)),
        ("frame", Float(|v| &mut v.frame)),
        ("scale", Float(|v| &mut v.scale)),
        ("sequence", Int(|v| &mut v.sequence)),
        ("animtime", Float(|v| &mut v.animtime)),
        ("framerate", Float(|v| &mut v.framerate)),
        ("controller", Bytes(|v| &mut v.controller)),
        ("blending", Bytes(|v| &mut v.blending)),
        ("rendermode", Int(|v| &mut v.rendermode)),
        ("renderamt", Float(|v| &mut v.renderamt)),
        ("rendercolor", Vec(|v| &mut v.rendercolor)),
        ("renderfx", Int(|v| &mut v.renderfx)),
        ("health", Float(|v| &mut v.health)),
        ("frags", Float(|v| &mut v.frags)),
        ("weapons", Int(|v| &mut v.weapons)),
        ("takedamage", Float(|v| &mut v.takedamage)),
        ("deadflag", FloatBits(|v| &mut v.deadflag)),
        ("view_ofs", Vec(|v| &mut v.view_ofs)),
        ("button", Int(|v| &mut v.button)),
        ("impulse", Int(|v| &mut v.impulse)),
        ("chain", Edict),
        ("dmg_inflictor", Edict),
        ("enemy", Edict),
        ("aiment", Edict),
        ("owner", Edict),
        ("groundentity", Edict),
        ("spawnflags", Int(|v| &mut v.spawnflags)),
        ("flags", FloatBits(|v| &mut v.flags)),
        ("colormap", Int(|v| &mut v.colormap)),
        ("team", Int(|v| &mut v.team)),
        ("max_health", Float(|v| &mut v.max_health)),
        ("teleport_time", Float(|v| &mut v.teleport_time)),
        ("armortype", Float(|v| &mut v.armortype)),
        ("armorvalue", Float(|v| &mut v.armorvalue)),
        ("waterlevel", Int(|v| &mut v.waterlevel)),
        ("watertype", Int(|v| &mut v.watertype)),
        ("target", Str(|v| &mut v.target)),
        ("targetname", Str(|v| &mut v.targetname)),
        ("netname", Str(|v| &mut v.netname)),
        ("message", Str(|v| &mut v.message)),
        ("dmg_take", Float(|v| &mut v.dmg_take)),
        ("dmg_save", Float(|v| &mut v.dmg_save)),
        ("dmg", Float(|v| &mut v.dmg)),
        ("dmgtime", Float(|v| &mut v.dmgtime)),
        ("noise", Str(|v| &mut v.noise)),
        ("noise1", Str(|v| &mut v.noise1)),
        ("noise2", Str(|v| &mut v.noise2)),
        ("noise3", Str(|v| &mut v.noise3)),
        ("speed", Float(|v| &mut v.speed)),
        ("air_finished", Float(|v| &mut v.air_finished)),
        ("pain_finished", Float(|v| &mut v.pain_finished)),
        ("radsuit_finished", Float(|v| &mut v.radsuit_finished)),
    ]
};

/// The table entry for `key`. `EntvarsKeyvalue` compares with `strcasecmp`.
fn entvars_field(key: &CStr) -> Option<Field> {
    let key = key.to_bytes();
    ENTVARS_FIELDS.iter().find(|(name, _)| name.as_bytes().eq_ignore_ascii_case(key)).map(|&(_, f)| f)
}

/// Stores a non-string, non-edict field; returns false for the kinds that need the engine.
fn set_entvars_number(v: &mut entvars_t, field: Field, value: &CStr) -> bool {
    match field {
        Field::Float(f) => *f(v) = kv_f(value),
        Field::FloatBits(f) => *f(v) = kv_f(value).to_bits() as i32,
        Field::Int(f) => *f(v) = kv_i(value),
        Field::Bytes(f) => {
            let dst = f(v);
            let n = dst.len();
            dst.copy_from_slice(&kv_i(value).to_le_bytes()[..n]);
        }
        Field::Vec(f) => *f(v) = kv_vec(value),
        Field::Str(_) | Field::Edict => return false,
    }
    true
}

/// `EntvarsKeyvalue` (0xef9f0): sets the `entvars_t` field named `key` and returns true, or
/// returns false if `key` isn't one. The engine parses none of these itself (not even
/// `classname`).
pub fn entvars_key_value(v: &mut entvars_t, key: &CStr, value: &CStr) -> bool {
    let Some(field) = entvars_field(key) else {
        return false;
    };
    match field {
        Field::Str(f) => *f(v) = engine().alloc_string(value),
        Field::Edict => engine().alert(ALERT_TYPE::at_error, "Bad field in entity!!\n"),
        _ => {
            set_entvars_number(v, field, value);
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// Runs `key_value_order` and returns which parsers were asked, in order, and `fHandled`.
    fn order(entvars_claims: bool, has_class: bool, class_claims: bool) -> (Vec<&'static str>, bool) {
        let calls = RefCell::new(Vec::new());
        let handled = key_value_order(
            || {
                calls.borrow_mut().push("entvars");
                entvars_claims
            },
            has_class,
            || {
                calls.borrow_mut().push("class");
                class_claims
            },
        );
        (calls.into_inner(), handled)
    }

    #[test]
    fn entvars_parsed_before_the_class() {
        // An entvars key never reaches the class (multi_manager can't see "targetname").
        assert_eq!(order(true, true, true), (vec!["entvars"], true));
        // Anything else goes to the class, which decides fHandled.
        assert_eq!(order(false, true, true), (vec!["entvars", "class"], true));
        assert_eq!(order(false, true, false), (vec!["entvars", "class"], false));
        // No szClassName: the class isn't asked.
        assert_eq!(order(false, false, true), (vec!["entvars"], false));
    }

    #[test]
    fn entvars_table_matches_the_sdk() {
        assert_eq!(ENTVARS_FIELDS.len(), 86);
        // Keys that classes also name: these belong to entvars.
        for k in [c"dmg", c"speed", c"health", c"frags", c"target", c"message", c"netname", c"skin", c"size", c"spawnflags", c"model"] {
            assert!(entvars_field(k).is_some(), "{k:?}");
        }
        // strcasecmp.
        assert!(entvars_field(c"TargetName").is_some());
        // Class keys.
        for k in [c"wait", c"delay", c"killtarget", c"master", c"points", c"pose", c"skyname", c"damage", c""] {
            assert!(entvars_field(k).is_none(), "{k:?}");
        }
    }

    #[test]
    fn entvars_numbers_store_like_the_sdk() {
        let mut v = entvars_t::default();
        let mut set = |k: &CStr, val: &CStr| set_entvars_number(&mut v, entvars_field(k).unwrap(), val);
        assert!(set(c"renderamt", c"150"));
        assert!(set(c"spawnflags", c"3"));
        assert!(set(c"angles", c"0 90 0"));
        assert!(set(c"flags", c"1"));
        assert!(set(c"controller", c"258"));
        assert!(set(c"blending", c"513"));
        assert!(!set(c"target", c"t"));
        assert!(!set(c"owner", c"1"));
        assert_eq!(v.renderamt, 150.0);
        assert_eq!(v.spawnflags, 3);
        assert_eq!(v.angles, [0.0, 90.0, 0.0]);
        assert_eq!(v.flags, 1.0f32.to_bits() as i32);
        assert_eq!(v.controller, [2, 1, 0, 0]);
        assert_eq!(v.blending, [1, 2]);
    }
}
