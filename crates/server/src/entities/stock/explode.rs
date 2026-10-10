//! Explosions and screen effects as TS implements them: `TSExplosion` (grenades and breakables), the SDK
//! `RadiusDamage` (env_explosion), `UTIL_ScreenShake` and `UTIL_ScreenFade` (both divided by the
//! player's time scale, fades sent as `TSFade`), `UTIL_Sparks`/`UTIL_Ricochet`, and TS's
//! `EMIT_SOUND_DYN` pitch scaling.

use crate::damage::{self, DamageInfo};
use crate::messages::{self, Msg};
use crate::weapons::worldgun;
use goldsrc::edict::EdictPtr;
use goldsrc::ext::entities::{MsgWriterExt, Vec3, is_player, te, te_begin, vec};
use goldsrc::server::{DONT_IGNORE_MONSTERS, engine};
use goldsrc::sys::*;
use std::ffi::CStr;

/// TS's `EMIT_SOUND_DYN` (0xe0a8c) with its extra slow-motion factor `s`: when the entity's
/// `fuser1` (time scale) is non-zero, pitch becomes `(s * fuser1 + 1 - s) * pitch`. `!NAME`
/// sentences are looked up in sentences.txt.
#[allow(clippy::too_many_arguments)]
pub fn emit_sound_slow(e: *mut edict_t, chan: u32, sample: &CStr, vol: f32, attn: f32, flags: i32, pitch: i32, s: f32) {
    if sample.is_empty() || e.is_null() {
        return;
    }
    let Some(sample) = super::sentences::resolve(sample) else {
        super::base::unknown_sentence(sample);
        return;
    };
    let ts = e.v().fuser1;
    let pitch = if ts != 0.0 { ((s * ts + 1.0 - s) * pitch as f32) as i32 } else { pitch };
    engine().emit_sound(e, chan as i32, &sample, vol, attn, flags, pitch);
}

/// `GiveSlowMul` / `FixedSlowAmt` (0x990bc): the player's time scale, at least 0.01.
fn slow_of(e: *mut edict_t) -> f32 {
    e.v().fuser1.max(0.01)
}

/// `FixedUnsigned16` (0xec8e8): `v * scale` truncated and clamped to 0..=0xffff.
fn fixed_u16(v: f32, scale: f32) -> i32 {
    ((v * scale) as i32).clamp(0, 0xffff)
}

/// The slow-motion step of TS's `UTIL_ScreenShake`/`UTIL_ScreenFadeWrite`: the already fixed-point
/// value divided by the receiver's `GiveSlowMul()` and truncated. Not clamped again; `WriteShort`
/// keeps the low 16 bits.
fn slowed(fixed: i32, slow: f32) -> i32 {
    (fixed as f32 / slow) as i32
}

/// TS `UTIL_ScreenShake` (0xec978): players on the ground within `radius` (0 = everyone) get a
/// `ScreenShake`; the fixed-point duration and frequency are divided by their time scale.
pub fn screen_shake(center: Vec3, amplitude: f32, frequency: f32, duration: f32, radius: f32) {
    let eng = engine();
    let duration = fixed_u16(duration, 4096.0);
    let frequency = fixed_u16(frequency, 256.0);
    for i in 1..=eng.max_clients() {
        let p = eng.player(i);
        if !is_player(p) {
            continue;
        }
        let v = p.v();
        if v.flags & FL_ONGROUND as i32 == 0 {
            continue;
        }
        let local = if radius <= 0.0 || vec::length(vec::sub(center, v.origin)) < radius { amplitude } else { 0.0 };
        if local != 0.0 {
            let slow = slow_of(p);
            messages::one(Msg::ScreenShake, p)
                .short(fixed_u16(local, 4096.0))
                .short(slowed(duration, slow))
                .short(slowed(frequency, slow))
                .end();
        }
    }
}

pub const FFADE_IN: i32 = 0;
pub const FFADE_OUT: i32 = 1;
pub const FFADE_MODULATE: i32 = 2;
pub const FFADE_STAYOUT: i32 = 4;

/// TS `UTIL_ScreenFade` (0xecdec, `UTIL_ScreenFadeBuild`/`UTIL_ScreenFadeWrite`): sent as `TSFade`
/// to one net client, the fixed-point durations divided by its time scale.
pub fn screen_fade(p: *mut edict_t, color: Vec3, fade_time: f32, hold: f32, alpha: i32, flags: i32) {
    if !is_player(p) || p.v().flags & FL_FAKECLIENT as i32 != 0 {
        return; // IsNetClient()
    }
    let slow = slow_of(p);
    messages::one(Msg::TsFade, p)
        .short(slowed(fixed_u16(fade_time, 4096.0), slow))
        .short(slowed(fixed_u16(hold, 4096.0), slow))
        .short(flags)
        .byte(color[0] as i32)
        .byte(color[1] as i32)
        .byte(color[2] as i32)
        .byte(alpha)
        .end();
}

pub fn screen_fade_all(color: Vec3, fade_time: f32, hold: f32, alpha: i32, flags: i32) {
    let eng = engine();
    for i in 1..=eng.max_clients() {
        screen_fade(eng.player(i), color, fade_time, hold, alpha, flags);
    }
}

/// `UTIL_Sparks`.
pub fn sparks(pos: Vec3) {
    goldsrc::ext::entities::te_sparks(pos);
}

/// `UTIL_Ricochet`: `TE_ARMOR_RICOCHET`.
pub fn ricochet(pos: Vec3, scale: f32) {
    te_begin(MSG_PVS, Some(pos), te::ARMOR_RICOCHET).vec3(pos).byte((scale * 10.0) as i32).end();
}

/// `CBaseEntity::Center` (vtable 62, 0xacb40).
fn center(e: *mut edict_t) -> Vec3 {
    let v = e.v();
    vec::mid(v.absmin, v.absmax)
}

fn deal(victim: *mut edict_t, inflictor: *mut edict_t, attacker: *mut edict_t, dmg: f32, bits: i32, dir: Vec3, pos: Vec3) {
    let mut info = DamageInfo::new(inflictor, attacker, dmg, bits);
    info.dir = dir;
    info.pos = pos;
    damage::apply(victim, &info);
}

/// `ClearMultiDamage`, `victim->TraceAttack(attacker, dmg, dir, &tr, bits)`,
/// `ApplyMultiDamage(inflictor, attacker)`: the impact point and hitgroup come from `tr`.
fn trace_attack(victim: *mut edict_t, inflictor: *mut edict_t, attacker: *mut edict_t, dmg: f32, bits: i32, dir: Vec3, tr: &TraceResult) {
    let mut info = DamageInfo::new(inflictor, attacker, dmg, bits);
    info.dir = dir;
    info.pos = tr.vecEndPos;
    info.hitgroup = tr.iHitgroup;
    damage::apply(victim, &info);
}

/// The damage `TSExplosion` deals at `dist` from the blast (0x7d32a..0x7d374): full inside
/// `r_full`, then `dmg − (dist − r_full) · dmg / (r_max − r_full)`, never below 0. Like TS there is
/// no guard for `r_max == r_full` (no caller passes it): the infinite falloff zeroes everything
/// outside `r_full`.
pub fn falloff_damage(dmg: f32, dist: f32, r_full: f32, r_max: f32) -> f32 {
    let falloff = dmg / (r_max - r_full);
    let mut d = dmg;
    if dist > r_full {
        d = dmg - (dist - r_full) * falloff;
        if d < 0.0 {
            d = 0.0;
        }
    }
    d
}

/// TS `TSExplosion(origin, inflictor, attacker, dmg, bits, rFull, rMax)` (0x7d01c), called by the
/// M61 grenade (`CM61Grenade::WaitThink` 0x7cacc) and exploding breakables (`CBreakable::Die`
/// 0x752e0). With the blast point `origin + 1z`:
///
/// - `UTIL_ScreenShake(amp dmg, freq 80, 1 s, radius 2·r_max)`, and a `TSFrags` explosion
///   (type 1, `dmg` truncated) to everyone;
/// - for each entity in the `r_max` sphere except the inflictor, if it takes damage and isn't
///   `DEAD_DEAD`: a white flash if it's a living player, then [`falloff_damage`] by the distance
///   to its `Center() + 1z`. In line of sight that is dealt through `TraceAttack`; otherwise a
///   trace back from the target measures the obstacle and only `d − thickness` gets through;
/// - every `WorldGun` in the sphere is knocked away ([`worldgun::blast`]);
/// - then the "spectacular" slow-motion point at the blast (0.25 for 1 s).
pub fn ts_explosion(origin: Vec3, inflictor: *mut edict_t, attacker: *mut edict_t, dmg: f32, bits: i32, r_full: f32, r_max: f32) {
    let eng = engine();
    let mut org = origin;
    org[2] += 1.0;
    // 0x7d04a: WorldGuns are pushed away from 50 u above the blast point.
    let push_from = vec::add(org, [0.0, 0.0, 50.0]);
    screen_shake(org, dmg, 80.0, 1.0, r_max + r_max);
    // 0x7d0f2: `(int)dmg` (the x87 control word is set to truncate).
    messages::all(Msg::TsFrags).vec3(org).coord(dmg as i32 as f32).coord(0.0).coord(0.0).byte(1).end();
    let attacker = if attacker.is_null() { inflictor } else { attacker };

    let mut e = std::ptr::null_mut();
    loop {
        e = eng.find_in_sphere(e, org, r_max);
        if e.is_null() {
            break;
        }
        if e == inflictor {
            continue;
        }
        let (takedamage, deadflag) = (e.v().takedamage, e.v().deadflag);
        if takedamage != 0.0 && deadflag != DEAD_DEAD as i32 {
            explosion_damage(e, org, inflictor, attacker, dmg, bits, r_full, r_max);
        }
        // 0x7d651: the WorldGun check runs for everything but the inflictor, damaged or not.
        if !e.is_free() && eng.string(e.v().classname).to_bytes() == b"WorldGun" {
            worldgun::blast(e, push_from, r_max);
        }
    }
    // 0x7d964/0x7d9ab: `DoSpectacularNess()`, then `CreateSlowMotionPoint(origin + 1z, 0.25, 1.0, 0)`.
    if crate::rules::scoring::do_spectacularness() {
        crate::entities::ts::create_slow_motion_point(org, 0.25, 1.0, false);
    }
}

/// The per-target damage of [`ts_explosion`] (0x7d1f7..0x7d64e).
#[allow(clippy::too_many_arguments)]
fn explosion_damage(
    e: *mut edict_t,
    org: Vec3,
    inflictor: *mut edict_t,
    attacker: *mut edict_t,
    dmg: f32,
    bits: i32,
    r_full: f32,
    r_max: f32,
) {
    let eng = engine();
    // `IsAlive() && IsPlayer()`.
    let v = e.v();
    if is_player(e) && v.deadflag == DEAD_NO as i32 && v.health > 0.0 {
        screen_fade(e, [222.0, 227.0, 189.0], 0.1, 0.05, 75, 0);
    }
    let mut spot = center(e);
    spot[2] += 1.0;
    let tr = eng.trace_line(org, spot, DONT_IGNORE_MONSTERS, inflictor);
    let d = falloff_damage(dmg, vec::length(vec::sub(org, spot)), r_full, r_max);
    let dir = vec::normalize(vec::sub(spot, org));
    if tr.flFraction == 1.0 || tr.pHit == e {
        trace_attack(e, inflictor, attacker, d, bits, dir, &tr);
        return;
    }
    // Occluded: trace back from the target (ignoring it) and lose the obstacle's thickness. The
    // back trace is the one `TraceAttack` gets.
    let entry = if tr.fStartSolid != 0 { org } else { tr.vecEndPos };
    let back = eng.trace_line(spot, org, DONT_IGNORE_MONSTERS, e);
    if back.flFraction == 1.0 {
        return;
    }
    let thickness = vec::length(vec::sub(back.vecEndPos, entry));
    if thickness < d {
        trace_attack(e, inflictor, attacker, d - thickness, bits, dir, &back);
    }
}

/// SDK `RadiusDamage(vecSrc, inflictor, attacker, dmg, radius, CLASS_NONE, bits)` (env_explosion).
pub fn radius_damage(src: Vec3, inflictor: *mut edict_t, attacker: *mut edict_t, dmg: f32, radius: f32, bits: i32) {
    let eng = engine();
    let falloff = if radius != 0.0 { dmg / radius } else { 1.0 };
    let in_water = eng.point_contents(src) == CONTENTS_WATER;
    let mut src = src;
    src[2] += 1.0;
    let attacker = if attacker.is_null() { inflictor } else { attacker };
    let mut e = std::ptr::null_mut();
    loop {
        e = eng.find_in_sphere(e, src, radius);
        if e.is_null() {
            break;
        }
        let v = e.v();
        if v.takedamage == 0.0 {
            continue;
        }
        if in_water && v.waterlevel == 0 || !in_water && v.waterlevel == 3 {
            continue;
        }
        let spot = center(e);
        let mut tr = eng.trace_line(src, spot, DONT_IGNORE_MONSTERS, inflictor);
        if tr.flFraction == 1.0 || tr.pHit == e {
            if tr.fStartSolid != 0 {
                tr.vecEndPos = src;
                tr.flFraction = 0.0;
            }
            let adjusted = (dmg - vec::length(vec::sub(src, tr.vecEndPos)) * falloff).max(0.0);
            if tr.flFraction != 1.0 {
                // TraceAttack path: direction and impact point are known.
                let dir = vec::normalize(vec::sub(tr.vecEndPos, src));
                deal(e, inflictor, attacker, adjusted, bits, dir, tr.vecEndPos);
            } else {
                // Plain TakeDamage.
                deal(e, inflictor, attacker, adjusted, bits, [0.0; 3], [0.0; 3]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explosion_falloff() {
        // The grenade: 110 full inside 300, linear to 0 at 1024.
        assert_eq!(falloff_damage(110.0, 0.0, 300.0, 1024.0), 110.0);
        assert_eq!(falloff_damage(110.0, 300.0, 300.0, 1024.0), 110.0);
        assert!((falloff_damage(110.0, 662.0, 300.0, 1024.0) - 55.0).abs() < 1e-4);
        assert_eq!(falloff_damage(110.0, 1024.0, 300.0, 1024.0), 0.0);
        // Centres can lie outside the sphere: clamped at 0.
        assert_eq!(falloff_damage(110.0, 1100.0, 300.0, 1024.0), 0.0);
        // A breakable with impulse 40: r_full 40, r_max 100.
        assert!((falloff_damage(40.0, 70.0, 40.0, 100.0) - 20.0).abs() < 1e-4);
        // No guard for r_max == r_full: full inside, nothing outside.
        assert_eq!(falloff_damage(50.0, 10.0, 64.0, 64.0), 50.0);
        assert_eq!(falloff_damage(50.0, 65.0, 64.0, 64.0), 0.0);
    }

    #[test]
    fn slow_motion_divides_the_fixed_point_value() {
        // `UTIL_ScreenShake`: 1 s → 4096, at a quarter time scale 16384.
        assert_eq!(slowed(fixed_u16(1.0, 4096.0), 0.25), 16384);
        // Truncated after the division, not before.
        assert_eq!(slowed(fixed_u16(0.1, 4096.0), 0.3), 1363);
        assert_eq!(fixed_u16(20.0, 4096.0), 0xffff);
    }
}
