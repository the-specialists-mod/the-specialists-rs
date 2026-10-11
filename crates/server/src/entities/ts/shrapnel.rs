//! `shrapnel` (`CTSShrapnel`, a `CTSPhysicObject`). Nothing in TS 3.0 calls
//! `CTSShrapnel::Create` (0x7bf9c), so the only shrapnel is one a map places: an invisible
//! (`EF_NODRAW`) bouncing piece that hits players for 100, sticks in the world, and is gone
//! after 30 s (10 s after its last hit).

use crate::damage::{self, DamageInfo};
use crate::entities::remove;
use crate::entities::stock::explode::emit_sound_slow;
use crate::save::{BaseFuncs, Fields};
use crate::weapons::SlowPhysics;
use crate::world::{Cx, Entity};
use goldsrc::edict::EdictPtr;
use goldsrc::ext::entities::{ATTN_NORM, Vec3, is_player, vec};
use goldsrc::server::engine;
use goldsrc::sys::*;

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

/// `Vector::Normalize` as TS inlines it: `(0, 0, 1)` for a zero vector.
fn normalize_or_up(v: Vec3) -> Vec3 {
    let len = vec::length(v);
    if len == 0.0 {
        return [0.0, 0.0, 1.0];
    }
    let k = 1.0 / len;
    [k * v[0], k * v[1], k * v[2]]
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
        v.nextthink = now + 0.1;
        let speed = vec::length(v.velocity);
        if speed <= 100.0 {
            if speed < 100.0 && v.movetype != MOVETYPE_FLY as i32 {
                remove(cx.edict);
            }
        } else {
            let dir = normalize_or_up(v.velocity);
            v.velocity = vec::scale(dir, speed - 1.0);
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
        if ov.takedamage == 0.0 || !is_player(other) {
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
    }
}
