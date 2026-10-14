//! `shrapnel` (`CTSShrapnel`, a `CTSPhysicObject`). Nothing in TS 3.0 calls
//! `CTSShrapnel::Create` (0x7bf9c, ported as [`create`] for rulesets that do), so the only
//! shrapnel is one a map places: an invisible (`EF_NODRAW`) bouncing piece that hits players for
//! 100, sticks in the world, and is gone after 30 s (10 s after its last hit).

use crate::damage::{self, DamageInfo};
use crate::entities::remove;
use crate::entities::stock::explode::emit_sound_slow;
use crate::rules::has_player;
use crate::save::{BaseFuncs, Fields};
use crate::weapons::SlowPhysics;
use crate::world::{Cx, Entity};
use goldsrc::edict::EdictPtr;
use goldsrc::ext::entities::{ATTN_NORM, Vec3, vec};
use goldsrc::server::engine;
use goldsrc::sys::*;
use shared::pm::{FpuMath, Msvc, ORDER_XYZ, below87, plus_double};

/// `CTSShrapnel::BounceTouch`'s damage bits: `DMG_NEVERGIB | DMG_SLASH`.
const SHRAPNEL_DAMAGE_BITS: i32 = 0x1004;

/// `shrapnel` (`CTSShrapnel`).
#[derive(Default)]
pub struct Shrapnel {
    phys: SlowPhysics,
    /// `+0x88`: removal time (0 = never).
    expire: f32,
    /// `+0x8c`: who threw it (only `Create` sets it).
    thrower: Option<*mut edict_t>,
    /// TS's member pointers: `m_pfnThink` (`WaitThink`) and `m_pfnTouch` (`BounceTouch`), set by
    /// `Spawn`.
    funcs: BaseFuncs,
}

/// `Vector::Normalize` as TS inlines it: `(0, 0, 1)` for a zero vector. In mp.dll's arithmetic
/// (`BounceTouch` 0x10050a0e..0x10050a6b and the out-of-line copy at 0x100148f0 it uses for a hit,
/// `WaitThink` 0x10050c86..0x10050cf2): `x² + y² + z²`, the length and `1/len` unrounded (issue
/// #150).
fn normalize_or_up(v: Vec3) -> Vec3 {
    Msvc::vec_normalize(v, ORDER_XYZ)
}

/// `CTSShrapnel::Create(owner, origin, velocity)` (0x7bf9c): a spawned piece at `origin`, flying
/// at `velocity` plus the owner's, with a random body (0..2), the owner as its thrower (the
/// attacker of its hits) and the owner's time scale (`GoSlow(owner->fuser1)`, which only writes
/// `fuser1`). As in TS, the fields are set after `Spawn` and the origin without `SET_ORIGIN`.
/// Returns the edict (null if none was free).
pub fn create(owner: *mut edict_t, origin: Vec3, velocity: Vec3) -> *mut edict_t {
    let eng = engine();
    let e = eng.create_named_entity(c"shrapnel");
    if e.is_null() {
        return e;
    }
    crate::ham::spawn(e);
    // Port addition: TS reads the owner unchecked; without one the piece keeps `Spawn`'s state.
    let (owner_velocity, owner_time) = if owner.is_null() { ([0.0; 3], 1.0) } else { (owner.v().velocity, owner.v().fuser1) };
    let body = eng.random_long(0, 2);
    crate::world::dispatch(e, |ent, cx| {
        if let Some(s) = (ent as &mut dyn std::any::Any).downcast_mut::<Shrapnel>() {
            s.thrower = (!owner.is_null()).then_some(owner);
        }
        let v = cx.pev();
        v.origin = origin;
        v.velocity = vec::add(velocity, owner_velocity);
        v.body = body;
        v.fuser1 = owner_time;
    });
    e
}

impl Entity for Shrapnel {
    /// `CBaseEntity::Save` / `Restore`: `CTSShrapnel`'s own state isn't saved; both pointers set.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut Fields) {
        super::save_base(cx, f, &mut self.funcs);
    }

    /// `CTSShrapnel::Precache` (0x7c1c8).
    fn restored(&mut self, _cx: &mut Cx) {
        let eng = engine();
        eng.precache_model(c"models/shrapnel.mdl");
        eng.precache_sound(c"weapons/knife/knife_hit.wav");
    }

    /// `CTSShrapnel::Spawn` (0x7c0a0) with `CTSPhysicObject::Spawn` (0x7ba54). TS precaches the
    /// model for every map (`TSPrecache` → `UTIL_PrecacheOther("shrapnel")`); the port precaches
    /// it here, before the `SET_MODEL` that needs it.
    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        self.phys = SlowPhysics::default();
        cx.pev().fuser1 = 1.0;
        eng.precache_model(c"models/shrapnel.mdl");
        eng.set_model(cx.edict, c"models/shrapnel.mdl");
        let v = cx.pev();
        v.gravity = 1.0;
        v.friction = 1.0;
        v.movetype = MOVETYPE_BOUNCE as i32;
        v.solid = SOLID_BBOX as i32;
        eng.set_size(cx.edict, [0.0; 3], [0.0; 3]);
        let now = eng.time();
        let v = cx.pev();
        v.nextthink = now;
        v.fuser1 = 1.0;
        self.expire = now + 30.0;
        v.iuser4 = 4;
        v.effects |= EF_NODRAW as i32;
        self.funcs = BaseFuncs { think: true, touch: true, ..BaseFuncs::default() };
    }

    /// `CTSShrapnel::WaitThink` (0x7c678).
    fn think(&mut self, cx: &mut Cx) {
        if !self.funcs.think {
            return;
        }
        let eng = engine();
        let now = eng.time();
        let v = cx.pev();
        v.nextthink = plus_double(now, 0.1);
        // mp.dll (0x10050c2f..0x10050d6f): the root unrounded for both tests (a NaN speed is at
        // most 100 and below it), stored as a `float` for the slow-down.
        let speed = Msvc::sq_in(v.velocity, ORDER_XYZ).sqrt();
        if speed > 100.0 {
            let dir = normalize_or_up(v.velocity);
            v.velocity = vec::scale(dir, speed as f32 - 1.0);
        } else if below87(speed, 100.0) && v.movetype != MOVETYPE_FLY as i32 {
            remove(cx.edict);
        }
        if self.expire == 0.0 || now <= self.expire {
            let v = cx.pev();
            if v.movetype != MOVETYPE_FLY as i32 {
                self.phys.update(v);
            }
        } else {
            crate::rules::powerup::remove_from_slowed(cx.edict);
            remove(cx.edict);
        }
    }

    /// `CTSShrapnel::BounceTouch` (0x7c200).
    fn touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        if !self.funcs.touch || other.is_null() || other.v().effects & EF_NODRAW as i32 != 0 {
            return;
        }
        let eng = engine();
        let me = cx.edict;
        let ov = other.v();
        if ov.takedamage == 0.0 || !has_player(other) {
            let class = if ov.classname == 0 { c"" } else { eng.string(ov.classname) };
            if class == c"worldspawn" || class == c"func_wall" {
                // Stick 6 units back along the flight path.
                let v = cx.pev();
                let dir = normalize_or_up(v.velocity);
                let org = vec::sub(v.origin, vec::scale(dir, 6.0));
                eng.set_origin(me, org);
                let mut angles = eng.vec_to_angles(dir);
                angles[2] = 90.0;
                let v = cx.pev();
                v.angles = angles;
                v.solid = SOLID_NOT as i32;
                v.movetype = MOVETYPE_FLY as i32;
                v.velocity = [0.0; 3];
                v.avelocity = [0.0; 3];
            } else {
                let v = cx.pev();
                v.velocity = vec::scale(v.velocity, -0.25);
            }
            let pitch = eng.random_long(20, 30) + 100;
            emit_sound_slow(me, CHAN_BODY, c"weapons/knife/knife_hit.wav", 1.0, ATTN_NORM, 0, pitch, 0.5);
        } else {
            // ClearMultiDamage, TraceAttack(100, DMG_NEVERGIB | DMG_SLASH) along the flight with
            // the engine's touch trace, ApplyMultiDamage(this, thrower or this).
            let attacker = self.thrower.unwrap_or(me);
            let gt = eng.globals();
            let mut info = DamageInfo::new(me, attacker, 100.0, SHRAPNEL_DAMAGE_BITS).traced();
            info.hitgroup = gt.trace_hitgroup;
            info.pos = gt.trace_endpos;
            info.dir = normalize_or_up(cx.pev().velocity);
            damage::apply(other, &info);
            let v = cx.pev();
            v.velocity = vec::scale(v.velocity, -0.25);
        }
        // SetThink(WaitThink) again (it already is) and 10 more seconds.
        self.expire = eng.time() + 10.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize() {
        assert_eq!(normalize_or_up([0.0; 3]), [0.0, 0.0, 1.0]);
        assert_eq!(normalize_or_up([3.0, 0.0, 4.0]), [0.6, 0.0, 0.8]);
        // mp.dll's unrounded length and `1/len` (issue #150); the old `float` maths gave x =
        // 0x3f27c9ab.
        let v = [0x4427_f24a, 0x431b_b090, 0xc43d_9372].map(f32::from_bits);
        assert_eq!(normalize_or_up(v).map(f32::to_bits), [0x3f27_c9ac, 0x3e1b_8ae8, 0xbf3d_6598]);
        assert_eq!(normalize_or_up([f32::NAN, 0.0, 0.0]), [0.0, 0.0, 1.0]);
    }
}
