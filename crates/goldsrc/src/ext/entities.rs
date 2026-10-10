//! Engine helpers added by the entities work package. Keep additions here to avoid merge conflicts.
//!
//! * `vec`: small `[f32; 3]` maths used by the SDK entity code (`Vector` operators).
//! * `EHandle`: SDK `EHANDLE` (edict pointer + serial number).
//! * Entity searches: `entities_in_box`, `find_in_sphere`, `find_by_target_name`.
//! * Temp entities (`SVC_TEMPENTITY`): `te` constants plus writers for the ones several classes use.
//! * A few engine calls the base wrapper doesn't cover (`set_view`, `static_decal`, ...).

use crate::edict::EdictPtr;
use crate::eng;
use crate::server::{Engine, MsgWriter};
use crate::sys::*;
use std::ffi::CStr;

pub type Vec3 = [f32; 3];

/// `Vector` arithmetic on `[f32; 3]`.
pub mod vec {
    use super::Vec3;

    pub const ZERO: Vec3 = [0.0; 3];

    #[inline]
    pub fn add(a: Vec3, b: Vec3) -> Vec3 {
        [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
    }
    #[inline]
    pub fn sub(a: Vec3, b: Vec3) -> Vec3 {
        [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
    }
    #[inline]
    pub fn scale(a: Vec3, s: f32) -> Vec3 {
        [a[0] * s, a[1] * s, a[2] * s]
    }
    /// `a + b * s`
    #[inline]
    pub fn mad(a: Vec3, b: Vec3, s: f32) -> Vec3 {
        [a[0] + b[0] * s, a[1] + b[1] * s, a[2] + b[2] * s]
    }
    #[inline]
    pub fn mul(a: Vec3, b: Vec3) -> Vec3 {
        [a[0] * b[0], a[1] * b[1], a[2] * b[2]]
    }
    #[inline]
    pub fn dot(a: Vec3, b: Vec3) -> f32 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }
    #[inline]
    pub fn length(a: Vec3) -> f32 {
        dot(a, a).sqrt()
    }
    #[inline]
    pub fn length2d(a: Vec3) -> f32 {
        (a[0] * a[0] + a[1] * a[1]).sqrt()
    }
    /// SDK `Vector::Normalize`: a zero vector normalises to (0, 0, 1).
    pub fn normalize(a: Vec3) -> Vec3 {
        let l = length(a);
        if l == 0.0 { [0.0, 0.0, 1.0] } else { scale(a, 1.0 / l) }
    }
    #[inline]
    pub fn is_zero(a: Vec3) -> bool {
        a == ZERO
    }
    /// Centre of an entity's box (`VecBModelOrigin` / `Center()`).
    #[inline]
    pub fn mid(a: Vec3, b: Vec3) -> Vec3 {
        scale(add(a, b), 0.5)
    }
}

// ---- EHANDLE -----------------------------------------------------------------------------------

/// SDK `EHANDLE`: an edict reference that goes null when the edict is freed and reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EHandle {
    e: *mut edict_t,
    serial: i32,
}

impl Default for EHandle {
    fn default() -> Self {
        Self::NULL
    }
}

impl EHandle {
    pub const NULL: EHandle = EHandle { e: std::ptr::null_mut(), serial: 0 };

    pub fn new(e: *mut edict_t) -> Self {
        if e.is_null() { Self::NULL } else { EHandle { e, serial: e.serial_number() } }
    }

    /// The edict, or null if it was freed (or reused for something else).
    pub fn get(&self) -> *mut edict_t {
        if self.e.is_null() {
            return std::ptr::null_mut();
        }
        let ok = !self.e.is_free() && self.e.serial_number() == self.serial && self.e.has_private_data();
        if ok { self.e } else { std::ptr::null_mut() }
    }

    pub fn is_valid(&self) -> bool {
        !self.get().is_null()
    }
}

// ---- edict helpers ----------------------------------------------------------------------------------

/// True for a usable edict (non-null, not free).
pub fn is_live(e: *const edict_t) -> bool {
    !e.is_null() && !e.is_free()
}

/// True if the edict is a connected client (player slot with FL_CLIENT).
pub fn is_player(e: *const edict_t) -> bool {
    is_live(e) && e.v().flags & FL_CLIENT as i32 != 0
}

impl Engine {
    pub fn max_entities(&self) -> i32 {
        self.globals().maxEntities
    }

    /// `UTIL_EntitiesInBox`: live entities whose absolute box overlaps `mins..maxs` and whose
    /// `flags` intersect `flag_mask` (0 = any). Free and private-data-less edicts are skipped.
    pub fn entities_in_box(&self, mins: Vec3, maxs: Vec3, flag_mask: i32, max: usize) -> Vec<*mut edict_t> {
        let mut out = Vec::new();
        for i in 1..self.max_entities() {
            let e = self.entity_of_index(i);
            if e.is_null() {
                continue;
            }
            if e.is_free() || !e.has_private_data() {
                continue;
            }
            let v = e.v();
            if flag_mask != 0 && v.flags & flag_mask == 0 {
                continue;
            }
            if (0..3).any(|k| mins[k] > v.absmax[k] || maxs[k] < v.absmin[k]) {
                continue;
            }
            out.push(e);
            if out.len() >= max {
                break;
            }
        }
        out
    }

    /// Next entity after `start` within `radius` of `org`; null when there are no more.

    /// First entity with `targetname == name` after `start` (null-safe).
    pub fn find_by_target_name(&self, start: *mut edict_t, name: &CStr) -> *mut edict_t {
        if name.is_empty() { std::ptr::null_mut() } else { self.find_by_string(start, c"targetname", name) }
    }

    /// `FIND_CLIENT_IN_PVS`: a client whose PVS contains `e` (the engine rotates through them on
    /// successive calls), or null. The engine returns the world edict when there is none.
    pub fn find_client_in_pvs(&self, e: *mut edict_t) -> *mut edict_t {
        let c = eng!(pfnFindClientInPVS(e));
        if c.is_null() || self.index_of(c) == 0 { std::ptr::null_mut() } else { c }
    }

    /// `CREATE_ENTITY()`: a fresh edict with no private data and no classname.
    pub fn create_entity(&self) -> *mut edict_t {
        eng!(pfnCreateEntity())
    }

    pub fn decal_index(&self, name: &CStr) -> i32 {
        eng!(pfnDecalIndex(name.as_ptr()))
    }

    pub fn static_decal(&self, origin: Vec3, decal: i32, entity: i32, model: i32) {
        eng!(pfnStaticDecal(origin.as_ptr(), decal, entity, model))
    }

    pub fn model_frames(&self, model_index: i32) -> i32 {
        eng!(pfnModelFrames(model_index))
    }

    /// `pfnCVarSetString` with a C-string value (`set_cvar_string` in ext::rules takes `&str`).
    pub fn set_cvar_cstr(&self, name: &CStr, value: &CStr) {
        eng!(pfnCVarSetString(name.as_ptr(), value.as_ptr()))
    }

    /// `UTIL_MakeVectors` + read back `v_forward`/`v_right`/`v_up` from the globals as well.
    pub fn make_vectors(&self, angles: Vec3) -> (Vec3, Vec3, Vec3) {
        eng!(pfnMakeVectors(angles.as_ptr()));
        let g = self.globals();
        (g.v_forward, g.v_right, g.v_up)
    }

    pub fn walk_move(&self, e: *mut edict_t, yaw: f32, dist: f32, mode: i32) -> i32 {
        eng!(pfnWalkMove(e, yaw, dist, mode))
    }

    pub fn trace_model(&self, start: Vec3, end: Vec3, hull: i32, ent: *mut edict_t) -> TraceResult {
        let mut tr = TraceResult::default();
        eng!(pfnTraceModel(start.as_ptr(), end.as_ptr(), hull, ent, &mut tr));
        tr
    }

    /// `FIND_CLIENT_IN_PVS`: a client that can see `e`, or null (the engine's world edict is
    /// mapped to null like `FNullEnt`).
    pub fn find_client_in_pvs(&self, e: *mut edict_t) -> *mut edict_t {
        let c = eng!(pfnFindClientInPVS(e));
        if c.is_null() || self.index_of(c) == 0 { std::ptr::null_mut() } else { c }
    }

    /// `UTIL_SetOrigin`.
    pub fn set_origin_v(&self, e: *mut edict_t, org: Vec3) {
        self.set_origin(e, org)
    }
}

// ---- sounds -------------------------------------------------------------------------------------------

pub const ATTN_NORM: f32 = 0.8;
pub const ATTN_IDLE: f32 = 2.0;
pub const ATTN_STATIC: f32 = 1.25;

/// Sound flags for `pfnEmitSound` / `pfnEmitAmbientSound`.
pub mod snd {
    pub const VOLUME: i32 = 1 << 0;
    pub const ATTENUATION: i32 = 1 << 1;
    pub const LARGE_INDEX: i32 = 1 << 2;
    pub const PITCH: i32 = 1 << 3;
    pub const SENTENCE: i32 = 1 << 4;
    pub const STOP: i32 = 1 << 5;
    pub const CHANGE_VOL: i32 = 1 << 6;
    pub const CHANGE_PITCH: i32 = 1 << 7;
    pub const SPAWNING: i32 = 1 << 8;
}

// ---- temp entities -------------------------------------------------------------------------------

pub const SVC_TEMPENTITY: i32 = 23;

/// `TE_*` codes (SDK common/const.h).
pub mod te {
    pub const BEAMPOINTS: i32 = 0;
    pub const BEAMENTPOINT: i32 = 1;
    pub const GUNSHOT: i32 = 2;
    pub const EXPLOSION: i32 = 3;
    pub const TAREXPLOSION: i32 = 4;
    pub const SMOKE: i32 = 5;
    pub const TRACER: i32 = 6;
    pub const LIGHTNING: i32 = 7;
    pub const BEAMENTS: i32 = 8;
    pub const SPARKS: i32 = 9;
    pub const LAVASPLASH: i32 = 10;
    pub const TELEPORT: i32 = 11;
    pub const EXPLOSION2: i32 = 12;
    pub const BSPDECAL: i32 = 13;
    pub const IMPLOSION: i32 = 14;
    pub const SPRITETRAIL: i32 = 15;
    pub const SPRITE: i32 = 17;
    pub const BEAMSPRITE: i32 = 18;
    pub const BEAMTORUS: i32 = 19;
    pub const BEAMDISK: i32 = 20;
    pub const BEAMCYLINDER: i32 = 21;
    pub const BEAMFOLLOW: i32 = 22;
    pub const GLOWSPRITE: i32 = 23;
    pub const BEAMRING: i32 = 24;
    pub const STREAK_SPLASH: i32 = 25;
    pub const DLIGHT: i32 = 27;
    pub const ELIGHT: i32 = 28;
    pub const TEXTMESSAGE: i32 = 29;
    pub const LINE: i32 = 30;
    pub const BOX: i32 = 31;
    pub const KILLBEAM: i32 = 99;
    pub const LARGEFUNNEL: i32 = 100;
    pub const BLOODSTREAM: i32 = 101;
    pub const SHOWLINE: i32 = 102;
    pub const BLOOD: i32 = 103;
    pub const DECAL: i32 = 104;
    pub const FIZZ: i32 = 105;
    pub const MODEL: i32 = 106;
    pub const EXPLODEMODEL: i32 = 107;
    pub const BREAKMODEL: i32 = 108;
    pub const GUNSHOTDECAL: i32 = 109;
    pub const SPRITE_SPRAY: i32 = 110;
    pub const ARMOR_RICOCHET: i32 = 111;
    pub const PLAYERDECAL: i32 = 112;
    pub const BUBBLES: i32 = 113;
    pub const BUBBLETRAIL: i32 = 114;
    pub const BLOODSPRITE: i32 = 115;
    pub const WORLDDECAL: i32 = 116;
    pub const WORLDDECALHIGH: i32 = 117;
    pub const DECALHIGH: i32 = 118;
    pub const PROJECTILE: i32 = 119;
    pub const SPRAY: i32 = 120;
    pub const PLAYERSPRITES: i32 = 121;
    pub const PARTICLEBURST: i32 = 122;
    pub const FIREFIELD: i32 = 123;

    /// `TE_EXPLOSION` flags.
    pub const EXPLFLAG_NONE: i32 = 0;
    pub const EXPLFLAG_NOADDITIVE: i32 = 1;
    pub const EXPLFLAG_NODLIGHTS: i32 = 2;
    pub const EXPLFLAG_NOSOUND: i32 = 4;
    pub const EXPLFLAG_NOPARTICLES: i32 = 8;

    /// `TE_BREAKMODEL` flags.
    pub const BREAK_TYPEMASK: i32 = 0x4f;
    pub const BREAK_GLASS: i32 = 0x01;
    pub const BREAK_METAL: i32 = 0x02;
    pub const BREAK_FLESH: i32 = 0x04;
    pub const BREAK_WOOD: i32 = 0x08;
    pub const BREAK_SMOKE: i32 = 0x10;
    pub const BREAK_TRANS: i32 = 0x20;
    pub const BREAK_CONCRETE: i32 = 0x40;
}

/// Extra writers on `MsgWriter`.
pub trait MsgWriterExt: Sized {
    fn vec3(self, v: Vec3) -> Self;
}

impl MsgWriterExt for MsgWriter {
    fn vec3(self, v: Vec3) -> Self {
        self.coord(v[0]).coord(v[1]).coord(v[2])
    }
}

/// Starts a temp-entity message (`MESSAGE_BEGIN(dest, SVC_TEMPENTITY, origin)` + `WRITE_BYTE(te)`).
pub fn te_begin(dest: u32, origin: Option<Vec3>, kind: i32) -> MsgWriter {
    MsgWriter::begin(dest as i32, SVC_TEMPENTITY, origin, std::ptr::null_mut()).byte(kind)
}

/// `UTIL_Sparks`: `TE_SPARKS` to the PVS.
pub fn te_sparks(pos: Vec3) {
    te_begin(MSG_PVS, Some(pos), te::SPARKS).vec3(pos).end();
}

/// `TE_BREAKMODEL` (func_breakable / doors / pushables).
#[allow(clippy::too_many_arguments)]
pub fn te_break_model(dest: u32, pos: Vec3, size: Vec3, velocity: Vec3, random_vel: i32, model: i32, count: i32, life: i32, flags: i32) {
    te_begin(dest, Some(pos), te::BREAKMODEL)
        .vec3(pos)
        .vec3(size)
        .vec3(velocity)
        .byte(random_vel)
        .short(model)
        .byte(count)
        .byte(life)
        .byte(flags)
        .end();
}

/// `TE_EXPLOSION`: sprite, scale in 0.1 units, framerate, flags.
pub fn te_explosion(pos: Vec3, sprite: i32, scale: i32, framerate: i32, flags: i32) {
    te_begin(MSG_PAS, Some(pos), te::EXPLOSION).vec3(pos).short(sprite).byte(scale).byte(framerate).byte(flags).end();
}

/// `TE_SMOKE`.
pub fn te_smoke(pos: Vec3, sprite: i32, scale: i32, framerate: i32) {
    te_begin(MSG_PAS, Some(pos), te::SMOKE).vec3(pos).short(sprite).byte(scale).byte(framerate).end();
}

/// `TE_BSPDECAL` to everyone (infodecal on a brush entity or the world).
pub fn te_bsp_decal(dest: u32, pos: Vec3, decal: i32, entity: i32, model_index: i32) {
    let w = te_begin(dest, None, te::BSPDECAL).vec3(pos).short(decal).short(entity);
    let w = if entity != 0 { w.short(model_index) } else { w };
    w.end();
}

/// `TE_BUBBLES` (env_bubbles uses an entity, this is the plain temp-entity version).
#[allow(clippy::too_many_arguments)]
pub fn te_bubbles(mins: Vec3, maxs: Vec3, height: f32, model: i32, count: i32, speed: f32) {
    let mid = vec::mid(mins, maxs);
    te_begin(MSG_PAS, Some(mid), te::BUBBLES).vec3(mins).vec3(maxs).coord(height).short(model).byte(count).coord(speed).end();
}

/// SDK `hudtextparms_t` (game_text / `UTIL_HudMessage`).
#[derive(Clone, Copy, Debug, Default)]
pub struct HudTextParms {
    pub x: f32,
    pub y: f32,
    pub effect: i32,
    pub r1: u8,
    pub g1: u8,
    pub b1: u8,
    pub a1: u8,
    pub r2: u8,
    pub g2: u8,
    pub b2: u8,
    pub a2: u8,
    pub fade_in: f32,
    pub fade_out: f32,
    pub hold: f32,
    pub fx_time: f32,
    pub channel: i32,
}

/// `UTIL_HudMessage`: `TE_TEXTMESSAGE` to one client (`to` non-null) or everyone.
pub fn te_text_message(to: *mut edict_t, p: &HudTextParms, text: &CStr) {
    let (dest, ed) = if to.is_null() { (MSG_BROADCAST, std::ptr::null_mut()) } else { (MSG_ONE, to) };
    let fixed = |v: f32, bits: i32| (v * (1 << bits) as f32) as i32;
    let mut w = MsgWriter::begin(dest as i32, SVC_TEMPENTITY, None, ed)
        .byte(te::TEXTMESSAGE)
        .byte(p.channel & 0xff)
        .short(fixed(p.x, 13))
        .short(fixed(p.y, 13))
        .byte(p.effect)
        .byte(p.r1 as i32)
        .byte(p.g1 as i32)
        .byte(p.b1 as i32)
        .byte(p.a1 as i32)
        .byte(p.r2 as i32)
        .byte(p.g2 as i32)
        .byte(p.b2 as i32)
        .byte(p.a2 as i32)
        .short(fixed(p.fade_in, 8))
        .short(fixed(p.fade_out, 8))
        .short(fixed(p.hold, 8));
    if p.effect == 2 {
        w = w.short(fixed(p.fx_time, 8));
    }
    // TS (old SDK `UTIL_HudMessage`) truncates at 511 characters.
    let bytes = text.to_bytes();
    if bytes.len() > 511 {
        let cut = crate::util::to_cstring(&String::from_utf8_lossy(&bytes[..511]));
        w.string(&cut).end();
    } else {
        w.string(text).end();
    }
}

#[cfg(test)]
mod tests {
    use super::vec::*;

    #[test]
    fn vector_maths() {
        assert_eq!(add([1.0, 2.0, 3.0], [1.0, 1.0, 1.0]), [2.0, 3.0, 4.0]);
        assert_eq!(normalize([0.0, 0.0, 0.0]), [0.0, 0.0, 1.0]);
        assert_eq!(normalize([0.0, 3.0, 0.0]), [0.0, 1.0, 0.0]);
        assert_eq!(length([3.0, 4.0, 0.0]), 5.0);
        assert_eq!(mid([0.0, 0.0, 0.0], [2.0, 4.0, 6.0]), [1.0, 2.0, 3.0]);
    }
}
