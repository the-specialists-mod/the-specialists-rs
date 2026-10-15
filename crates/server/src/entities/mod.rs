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

/// `Classify()` (vtable +0x4c). TS overrides `CBaseEntity::Classify` (0xac65c, `CLASS_NONE`) in
/// five classes, none of them derived from: `CBasePlayer` (0xd9e10, `CLASS_PLAYER`),
/// `CGunTarget` (0xd7d7c, `CLASS_MACHINE`), `CDeadHEV` (0xde5ac, `CLASS_HUMAN_PASSIVE`) and
/// `CXenTree` / `CXenHull` (0xf7130 / 0xf72ac, `CLASS_BARNACLE`). A constant per class, so it
/// goes by the classname, as `ts::object_caps` does.
pub fn classify(e: *mut edict_t) -> i32 {
    if crate::rules::has_player(e) {
        return 2;
    }
    if e.is_null() || e.is_free() {
        return 0;
    }
    classify_class(engine().string(e.v().classname).to_bytes())
}

/// [`classify`] of a non-player classname.
fn classify_class(class: &[u8]) -> i32 {
    match class {
        b"func_guntarget" => 1,
        b"monster_hevsuit_dead" => 4,
        b"xen_tree" | b"xen_hull" => 99,
        _ => 0,
    }
}

/// Exports `$class(entvars_t*)` for each classname, creating a `$ty` on the edict.
macro_rules! link_entities {
    ($($class:ident => $ty:ty),* $(,)?) => {
        $(goldsrc::export_entity_class!($class, |pev| {
            const CLASS: &::std::ffi::CStr = match ::std::ffi::CStr::from_bytes_with_nul(concat!(stringify!($class), "\0").as_bytes()) {
                Ok(c) => c,
                Err(_) => panic!("classname"),
            };
            $crate::world::create_linked::<$ty>(pev, CLASS)
        });)*

        /// The classnames this module links, each with its Rust type (`crate::ham`'s classes).
        pub(crate) fn linked_classes() -> Vec<(&'static str, std::any::TypeId)> {
            vec![$((stringify!($class), std::any::TypeId::of::<$ty>())),*]
        }

        /// The classnames this module links, each with a constructor (the save-table tests).
        #[cfg(test)]
        pub(crate) fn linked_entities() -> Vec<$crate::world::Linked> {
            vec![$((
                stringify!($class),
                (|| Box::new(<$ty>::default()) as Box<dyn $crate::world::Entity>) as fn() -> Box<dyn $crate::world::Entity>,
            )),*]
        }
    };
}
pub(crate) use link_entities;

/// `DispatchKeyValue` (0xaad4c, the SDK's cbase.cpp unchanged): the entvars table first, and the
/// class's `KeyValue` only for keys it didn't claim. `class` is the engine's `szClassName`;
/// without it (or without a game object on the edict) the class isn't asked.
/// Returns `fHandled`.
///
/// A class the gameplay plugin refuses ([`spawns_class`]) parses nothing.
pub fn dispatch_key_value(e: *mut edict_t, class: Option<&CStr>, key: &CStr, value: &CStr) -> bool {
    let has_class = class.is_some_and(spawns_class);
    key_value_order(|| entvars_key_value(e.v(), key, value), has_class, || crate::ham::key_value(e, class, key, value))
}

/// Whether the active gameplay plugin lets an entity of `class` spawn
/// (`ServerRules::spawns_entity`).
pub fn spawns_class(class: &CStr) -> bool {
    crate::gameplay::active().server.spawns_entity(class)
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
            // Port guard: `SV_Physics` stops every engine on a movetype it has no case for
            // ("bad movetype"), which TS takes from a map's `movetype` key as it is.
            if key.to_bytes().eq_ignore_ascii_case(b"movetype") && !physics_movetype(v.movetype) {
                let msg = format!("movetype {} would stop the engine; MOVETYPE_NONE (port guard)\n", v.movetype);
                engine().alert(ALERT_TYPE::at_error, &msg);
                v.movetype = MOVETYPE_NONE as i32;
            }
        }
    }
    true
}

/// A movetype `SV_Physics` runs for a non-player entity (ReHLDS and FWGS both stop the server on
/// any other, `MOVETYPE_WALK` and the unused `ANGLENOCLIP`/`ANGLECLIP` included).
fn physics_movetype(m: i32) -> bool {
    [MOVETYPE_NONE, MOVETYPE_STEP, MOVETYPE_FLY, MOVETYPE_TOSS, MOVETYPE_PUSH, MOVETYPE_NOCLIP]
        .into_iter()
        .chain([MOVETYPE_FLYMISSILE, MOVETYPE_BOUNCE, MOVETYPE_BOUNCEMISSILE, MOVETYPE_FOLLOW, MOVETYPE_PUSHSTEP])
        .any(|t| t as i32 == m)
}

/// Whether the engines can trace a `SOLID_BSP` entity: a pusher (`MOVETYPE_PUSH`/`PUSHSTEP`)
/// with a brush model (`*n`). An entity that has a model name but no model index yet is still in
/// its `Spawn`: brush entities (the SDK's doors among them) link through `UTIL_SetOrigin` before
/// `SET_MODEL`, and only a trace is fatal, so it is checked again when `SET_MODEL` links it.
fn bsp_hull_ok(movetype: i32, modelindex: i32, model: &[u8]) -> bool {
    if modelindex == 0 && !model.is_empty() {
        return true;
    }
    (movetype == MOVETYPE_PUSH as i32 || movetype == MOVETYPE_PUSHSTEP as i32) && modelindex != 0 && model.first() == Some(&b'*')
}

/// Fires `use_` on every entity whose targetname is `target` (SDK `FireTargets`).
pub fn fire_targets(target: &CStr, activator: *mut edict_t, caller: *mut edict_t, kind: UseType, value: f32) {
    if target.is_empty() {
        return;
    }
    let name = target.to_string_lossy();
    engine().alert(ALERT_TYPE::at_aiconsole, &format!("Firing: ({name})\n"));
    let mut e = std::ptr::null_mut();
    loop {
        e = engine().find_by_string(e, c"targetname", target);
        if e.is_null() {
            break;
        }
        if e.v().flags & FL_KILLME as i32 != 0 {
            continue; // already removed this frame
        }
        let cls = engine().string(e.v().classname).to_string_lossy();
        engine().alert(ALERT_TYPE::at_aiconsole, &format!("Found: {cls}, firing ({name})\n"));
        crate::ham::use_(e, activator, caller, kind, value);
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
/// `pfnSetAbsBox` (`DispatchObjectCollsionBox` → the class's `SetObjectCollisionBox`): `absmin`
/// and `absmax` for the engine's touch links. Player items take a fixed box around their origin
/// (`stock::playeritem::object_collision_box`); everything else gets the base box.
pub fn set_object_collision_box(e: *mut edict_t) {
    let v = e.v();
    let classname = if v.classname == 0 { c"" } else { engine().string(v.classname) };
    // Port guard: every engine stops the server when a trace meets a `SOLID_BSP` entity that
    // isn't a pusher (`SV_HullForEntity`) or has no brush model (`SV_HullForBsp`: "Hit a %s with
    // no model"), e.g. a map `func_wall` given a studio model, which TS sets up as it is. The
    // engine links the entity after this call, so it is made `SOLID_BBOX` (its box) instead.
    if v.solid == SOLID_BSP as i32 && v.modelindex != 1 {
        let model = if v.model == 0 { c"" } else { engine().string(v.model) };
        if !bsp_hull_ok(v.movetype, v.modelindex, model.to_bytes()) {
            let msg = format!(
                "{} with SOLID_BSP, movetype {} and model \"{}\" would stop the engine; made SOLID_BBOX (port guard)\n",
                classname.to_string_lossy(),
                v.movetype,
                model.to_string_lossy()
            );
            engine().alert(ALERT_TYPE::at_error, &msg);
            v.solid = SOLID_BBOX as i32;
        }
    }
    if let Some((mins, maxs)) = stock::playeritem::object_collision_box(classname, v.origin) {
        v.absmin = mins;
        v.absmax = maxs;
        return;
    }
    (v.absmin, v.absmax) = base_collision_box(v.solid, v.origin, v.angles, v.mins, v.maxs);
}

/// `::SetObjectCollisionBox` (0xabf9c): a rotated BSP model gets a cube as wide as its largest
/// extent, anything else its size; then one unit more on every side so touching works on the
/// edges.
fn base_collision_box(solid: i32, origin: [f32; 3], angles: [f32; 3], mins: [f32; 3], maxs: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let (mut lo, mut hi) = ([0.0; 3], [0.0; 3]);
    if solid == SOLID_BSP as i32 && angles != [0.0; 3] {
        let mut max = 0.0f32;
        for i in 0..3 {
            for x in [mins[i].abs(), maxs[i].abs()] {
                if max < x {
                    max = x;
                }
            }
        }
        for i in 0..3 {
            lo[i] = origin[i] - max;
            hi[i] = origin[i] + max;
        }
    } else {
        for i in 0..3 {
            lo[i] = origin[i] + mins[i];
            hi[i] = origin[i] + maxs[i];
        }
    }
    for i in 0..3 {
        lo[i] -= 1.0;
        hi[i] += 1.0;
    }
    (lo, hi)
}

/// `DispatchSpawn` (0xaab8c): the engine's spawn callback, and what `CBaseEntity::Create`
/// (0xac480) runs. The bounds start one unit around the origin, the class spawns, and an entity
/// that marked itself `FL_KILLME` fails (-1; the rules' `IsAllowedToSpawn` is always 1 in TS).
/// Then the `globalname` step: a new name is added to `gGlobalState` as on, a dead one fails the
/// spawn, and one another map added makes the entity dormant (`MakeDormant` 0xac270).
///
/// First, a class the gameplay plugin refuses ([`spawns_class`]) fails at once, as the engine
/// fails a classname the game library doesn't export.
pub fn dispatch_spawn(e: *mut edict_t) -> i32 {
    if e.is_null() || !e.has_private_data() {
        return 0;
    }
    let v = e.v();
    if v.classname != 0 {
        let class = engine().string(v.classname);
        if !spawns_class(class) {
            engine().alert(ALERT_TYPE::at_aiconsole, &format!("Can't init {} (not in this ruleset)\n", class.to_string_lossy()));
            return -1;
        }
    }
    let v = e.v();
    v.absmin = v.origin.map(|o| o - 1.0);
    v.absmax = v.origin.map(|o| o + 1.0);
    crate::ham::spawn(e);
    // A class that `REMOVE_ENTITY`s itself in its spawn leaves no private data: TS's re-read
    // `pvPrivateData` is NULL and it returns 0.
    if e.is_free() || !e.has_private_data() {
        return 0;
    }
    if e.v().flags & FL_KILLME as i32 != 0 {
        return -1;
    }
    let global = e.v().globalname;
    if global != 0 {
        match stock::base::global_spawn(stock::base::s(global)) {
            stock::base::GlobalSpawn::Dead => return -1,
            stock::base::GlobalSpawn::Dormant => make_dormant(e),
            stock::base::GlobalSpawn::Added | stock::base::GlobalSpawn::Spawned => {}
        }
    }
    0
}

/// `ServerActivate` (0xae928)'s entity loop: `Activate` on every edict in use from index
/// `client_max` on (the SDK's off-by-one: edict `client_max` is the last player slot, whose
/// `Activate` is `CBaseEntity`'s no-op) that has private data; a dormant one gets "Can't
/// instance" instead. The engine calls `ServerActivate` on a new map and after a
/// restore.
pub fn server_activate(edict_count: i32, client_max: i32) {
    let eng = engine();
    for i in 0..edict_count {
        let e = eng.entity_of_index(i);
        if e.is_null() || e.is_free() {
            continue;
        }
        if !activates(i, client_max, e.has_private_data()) {
            continue;
        }
        let v = e.v();
        if v.flags & FL_DORMANT as i32 != 0 {
            eng.alert(ALERT_TYPE::at_console, &format!("Can't instance {}\n", eng.string(v.classname).to_string_lossy()));
            continue;
        }
        crate::ham::activate(e);
    }
}

/// Whether `ServerActivate`'s loop looks at edict `index` (in use): past the first `client_max`
/// edicts, with private data.
fn activates(index: i32, client_max: i32, has_private_data: bool) -> bool {
    index >= client_max && has_private_data
}

/// `CBaseEntity::MakeDormant` (0xac270).
pub fn make_dormant(e: *mut edict_t) {
    let v = e.v();
    v.flags |= FL_DORMANT as i32;
    v.solid = SOLID_NOT as i32;
    v.movetype = MOVETYPE_NONE as i32;
    v.effects |= EF_NODRAW as i32;
    v.nextthink = 0.0;
    let origin = v.origin;
    engine().set_origin(e, origin);
}

/// `UTIL_Remove` (0xeea90): `UpdateOnRemove`, then `FL_KILLME` (the engine frees the edict at
/// the end of the frame) and no `targetname`.
pub fn remove(e: *mut edict_t) {
    if e.is_null() {
        return;
    }
    stock::base::update_on_remove(e);
    let v = e.v();
    v.flags |= FL_KILLME as i32;
    v.targetname = 0;
}

/// `CBaseEntity::TakeDamage` (0xab994), what every class without its own `TakeDamage` runs
/// (`world::Entity::take_damage`'s default): refused with `takedamage` off; `g_vecAttackDir`
/// from the brush centre to the inflictor's origin; a `MOVETYPE_WALK`/`STEP` entity hit by a
/// real inflictor (not the world) of an attacker that isn't `SOLID_TRIGGER` is pushed away from
/// the inflictor's centre (`DamageForce`); then the health drops, and at 0 `Killed`
/// ([`base_killed`]). Returns whether it is still alive.
pub fn base_take_damage(cx: &mut world::Cx, dmg: &crate::damage::DamageInfo) -> bool {
    use goldsrc::ext::entities::vec;
    if cx.pev().takedamage == 0.0 {
        return false;
    }
    let (inflictor, attacker) = (dmg.inflictor, dmg.attacker);
    // Guard: TS reads the inflictor's origin (and below, the attacker's `solid`) through NULL.
    if !inflictor.is_null() {
        let v = cx.pev();
        let dir = vec::sub(inflictor.v().origin, vec::mad(v.absmin, v.size, 0.5));
        crate::damage::set_attack_dir(attack_dir_normalize(dir));
        let walks = v.movetype == MOVETYPE_WALK as i32 || v.movetype == MOVETYPE_STEP as i32;
        let trigger = !attacker.is_null() && attacker.v().solid == SOLID_TRIGGER as i32;
        if engine().index_of(inflictor) != 0 && walks && !trigger {
            let iv = inflictor.v();
            let v = cx.pev();
            v.velocity = base_push(v.velocity, v.origin, (iv.absmin, iv.absmax), v.size, dmg.damage);
        }
    }
    let v = cx.pev();
    v.health -= dmg.damage;
    let alive = v.health > 0.0;
    if !alive {
        base_killed(cx);
    }
    alive
}

/// `CBaseEntity::TakeDamage`'s inline `g_vecAttackDir = vecTemp.Normalize()` (mp.dll
/// 0x1000e5a7..0x1000e608): the squares summed `y, z, x`, the length and `1 / length` unrounded,
/// each component stored.
fn attack_dir_normalize(dir: [f32; 3]) -> [f32; 3] {
    use shared::pm::FpuMath;
    shared::pm::Msvc::vec_normalize(dir, [1, 2, 0])
}

/// `CBaseEntity::TakeDamage`'s push: `velocity += normalize(origin - inflictor centre) ·
/// DamageForce(damage)` (at most 1000). mp.dll (0x1000e6fd..0x1000e81d) normalises with the
/// out-of-line `Length` (0x1000e920, unrounded), and its `Vector` `*` and `+` (0x1000e8e0,
/// 0x1000e8a0) store the product before the add.
fn base_push(velocity: [f32; 3], origin: [f32; 3], inflictor: ([f32; 3], [f32; 3]), size: [f32; 3], damage: f32) -> [f32; 3] {
    let centre = [0, 1, 2].map(|i| (inflictor.0[i] + inflictor.1[i]) * 0.5);
    let dir = crate::damage::normalize([0, 1, 2].map(|i| origin[i] - centre[i]));
    let force = crate::rules::lifecycle::damage_force(shared::pm::Arith::Msvc, size, damage);
    [0, 1, 2].map(|i| velocity[i] + force * dir[i])
}

/// `CBaseEntity::Killed` (0xabd4c): `takedamage` off, `DEAD_DEAD`, `UTIL_Remove`.
pub fn base_killed(cx: &mut world::Cx) {
    let v = cx.pev();
    v.takedamage = DAMAGE_NO as f32;
    v.deadflag = DEAD_DEAD as i32;
    remove(cx.edict);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// TS's `Classify` overrides by class; `func_breakable` and the rest are `CLASS_NONE`, so
    /// `TEXTURETYPE_PlaySound` reads their texture.
    #[test]
    fn classify_by_class() {
        assert_eq!(classify_class(b"func_guntarget"), 1);
        assert_eq!(classify_class(b"monster_hevsuit_dead"), 4);
        assert_eq!(classify_class(b"xen_tree"), 99);
        assert_eq!(classify_class(b"xen_hull"), 99);
        for class in [&b"func_breakable"[..], b"func_tank", b"worldspawn", b"ts_groundweapon", b"cycler"] {
            assert_eq!(classify_class(class), 0);
        }
    }

    /// Issue #70: `GetDelay` (vtable +0x90) is `CBaseToggle::GetDelay` (0xe3dac, `m_flWait`) in
    /// these TS classes' vtables, `CPathCorner::GetDelay` (0xd1e40) in `CPathCorner`'s, and
    /// `CBaseEntity::GetDelay` (0xac6b0, 0) in every other's. A `wait` key must come back from
    /// `get_delay` for each linked class whose `KeyValue` reaches `CBaseToggle::KeyValue`.
    #[test]
    fn get_delay_is_the_toggle_wait() {
        const TOGGLE: &[&str] = &[
            "CBaseButton",
            "CBaseDoor",
            "CBaseTrigger",
            "CChangeLevel",
            "CCyclerProbe",
            "CDeadHEV",
            "CFuncIllusionary",
            "CFuncPlat",
            "CFuncPlatRot",
            "CFuncTrackAuto",
            "CFuncTrackChange",
            "CFuncTrain",
            "CGenericCycler",
            "CGunTarget",
            "CLadder",
            "CMomentaryDoor",
            "CMomentaryRotButton",
            "CMultiManager",
            "CRecharge",
            "CRotButton",
            "CRotDoor",
            "CTriggerCDAudio",
            "CTriggerCounter",
            "CTriggerEndSection",
            "CTriggerGravity",
            "CTriggerHurt",
            "CTriggerMonsterJump",
            "CTriggerMultiple",
            "CTriggerOnce",
            "CTriggerPush",
            "CTriggerSave",
            "CTriggerTeleport",
            "CTSModel",
            "CWallHealth",
            "CWreckage",
            "CPathCorner",
        ];
        // Not listed although `CBaseToggle`s: `CEnvExplosion` and `CTSTrigger`, whose `KeyValue`
        // (0xbfab8, 0x70890) never reaches `CBaseToggle::KeyValue`, and `CBasePlayer`, which
        // gets no keyvalues. Their `m_flWait` stays 0, so the default `get_delay` is theirs.
        let mut linked = stock::linked_entities();
        linked.extend(ts::linked_entities());
        linked.extend(stock::monsters::linked_entities());
        linked.extend(crate::player::linked_entities());
        let mut seen = Vec::new();
        for (class, ctor) in linked {
            let ts_class = crate::save::tables::class_of(class.as_bytes()).unwrap_or("");
            let mut ent = ctor();
            // Other classes' `KeyValue` may need the engine (string allocation), so only the
            // toggles get the key; the rest must answer 0 regardless.
            if !TOGGLE.contains(&ts_class) {
                assert_eq!(ent.get_delay(), 0.0, "{class} ({ts_class})");
                continue;
            }
            let mut ed = edict_t::default();
            let mut cx = crate::world::Cx { edict: &mut ed };
            assert!(ent.key_value(&mut cx, c"wait", c"2.5"), "{class} parses wait");
            assert_eq!(ent.get_delay(), 2.5, "{class} ({ts_class})");
            seen.push(ts_class);
        }
        for c in TOGGLE {
            assert!(seen.contains(c), "{c} isn't linked");
        }
    }

    /// mp.dll's `CBaseEntity::TakeDamage`: the attack direction sums `y, z, x` with the length
    /// unrounded, and the push stores `force · dir` before adding it. The old arithmetic gave
    /// `[0xbe10d495, 0xbf33e31c, 0xbf32826d]` and a pushed z of 0x4367f60d.
    #[test]
    fn take_damage_direction_and_push_round_as_mp_dll() {
        let f = |b: [u32; 3]| b.map(f32::from_bits);
        let dir = attack_dir_normalize(f([0xc25b_aa10, 0xc388_6add, 0xc387_5f68]));
        assert_eq!(dir.map(f32::to_bits), [0xbe10_d495, 0xbf33_e31b, 0xbf32_826c]);
        // `base_push`'s steps with a force of 710.42.
        let vel = f([0xc309_2109, 0x4363_ca6c, 0xc382_bc55]);
        let org = f([0x42d7_0491, 0x435e_0d98, 0xc323_9be0]);
        let cen = f([0x436d_44d9, 0x435f_5139, 0xc390_71e1]);
        let force = f32::from_bits(0x4431_9aeb);
        let d = crate::damage::normalize([0, 1, 2].map(|i| org[i] - cen[i]));
        let pushed = [0, 1, 2].map(|i| vel[i] + force * d[i]);
        assert_eq!(pushed.map(f32::to_bits), [0xc422_0d05, 0x435e_cfc6, 0x4367_f60c]);
    }

    #[test]
    fn base_take_damage_pushes_away_from_the_inflictor() {
        // A 32x32x72 box hit for 10 from a centre 100 units along -x: DamageForce
        // 73728 / 73728 * 10 * 5 = 50 along +x.
        let v = base_push([0.0; 3], [0.0; 3], ([-110.0, -10.0, -10.0], [-90.0, 10.0, 10.0]), [32.0, 32.0, 72.0], 10.0);
        assert_eq!(v, [50.0, 0.0, 0.0]);
        // Capped at 1000.
        let v = base_push([0.0, 0.0, 5.0], [0.0; 3], ([-1.0, -1.0, -11.0], [1.0, 1.0, -9.0]), [1.0, 1.0, 1.0], 100.0);
        assert_eq!(v, [0.0, 0.0, 1005.0]);
    }

    #[test]
    fn engine_fatal_entity_states_are_recognised() {
        assert!(bsp_hull_ok(MOVETYPE_PUSH as i32, 5, b"*3"));
        assert!(bsp_hull_ok(MOVETYPE_PUSHSTEP as i32, 5, b"*3"));
        assert!(!bsp_hull_ok(MOVETYPE_NONE as i32, 5, b"*3"));
        // Still in its Spawn: the model is set (and checked) by the next link.
        assert!(bsp_hull_ok(MOVETYPE_PUSH as i32, 0, b"*3"));
        assert!(bsp_hull_ok(MOVETYPE_NONE as i32, 0, b"*3"));
        assert!(!bsp_hull_ok(MOVETYPE_PUSH as i32, 0, b""));
        assert!(!bsp_hull_ok(MOVETYPE_PUSH as i32, 5, b"models/tree.mdl"));
        assert!(!bsp_hull_ok(MOVETYPE_PUSH as i32, 5, b""));
        for m in [0, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13] {
            assert!(physics_movetype(m), "{m}");
        }
        for m in [-1, 1, 2, 3, 14, 99] {
            assert!(!physics_movetype(m), "{m}");
        }
    }

    #[test]
    fn server_activate_skips_the_first_client_max_edicts() {
        // Edicts 0..client_max (the world and all but the last player slot) are skipped.
        assert!(!activates(0, 32, true));
        assert!(!activates(31, 32, true));
        assert!(activates(32, 32, true));
        assert!(activates(100, 32, true));
        // No private data.
        assert!(!activates(100, 32, false));
        // A listen server with one slot: the world's index 0 is still skipped.
        assert!(!activates(0, 1, true));
        assert!(activates(1, 1, true));
    }

    #[test]
    fn collision_boxes() {
        let (o, mins, maxs) = ([100.0, 0.0, 0.0], [-10.0, -40.0, 0.0], [10.0, 40.0, 64.0]);
        // Unrotated (or not a BSP model): the size plus one unit.
        assert_eq!(base_collision_box(SOLID_BSP as i32, o, [0.0; 3], mins, maxs), ([89.0, -41.0, -1.0], [111.0, 41.0, 65.0]));
        assert_eq!(base_collision_box(SOLID_BBOX as i32, o, [0.0, 90.0, 0.0], mins, maxs), ([89.0, -41.0, -1.0], [111.0, 41.0, 65.0]));
        // A rotated BSP model: a cube of the largest extent (64).
        assert_eq!(base_collision_box(SOLID_BSP as i32, o, [0.0, 90.0, 0.0], mins, maxs), ([35.0, -65.0, -65.0], [165.0, 65.0, 65.0]));
    }

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
