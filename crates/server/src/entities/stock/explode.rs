//! Explosions and screen effects as TS implements them: `TSExplosion` (breakables), the SDK
//! `RadiusDamage` (env_explosion), `UTIL_ScreenShake` and `UTIL_ScreenFade` (both divided by the
//! player's time scale, fades sent as `TSFade`), `UTIL_Sparks`/`UTIL_Ricochet`, and TS's
//! `EMIT_SOUND_DYN` pitch scaling.

use crate::damage::{self, DamageInfo};
use crate::messages::{self, Msg};
use goldsrc::ext::entities::{MsgWriterExt, Vec3, is_player, te, te_begin, vec};
use goldsrc::server::{DONT_IGNORE_MONSTERS, engine};
use goldsrc::sys::*;
use std::ffi::CStr;

/// TS's `EMIT_SOUND_DYN` with its extra slow-motion factor `s`: when the entity's `fuser1`
/// (time scale) is non-zero, pitch becomes `(s * fuser1 + 1 - s) * pitch`.
#[allow(clippy::too_many_arguments)]
pub fn emit_sound_slow(e: *mut edict_t, chan: u32, sample: &CStr, vol: f32, attn: f32, flags: i32, pitch: i32, s: f32) {
    if sample.is_empty() || e.is_null() {
        return;
    }
    let ts = unsafe { (*e).v.fuser1 };
    let pitch = if ts != 0.0 { ((s * ts + 1.0 - s) * pitch as f32) as i32 } else { pitch };
    engine().emit_sound(e, chan as i32, sample, vol, attn, flags, pitch);
}

/// `GiveSlowMul` / `FixedSlowAmt`: the player's time scale, at least 0.01.
fn slow_of(e: *mut edict_t) -> f32 {
    unsafe { (*e).v.fuser1 }.max(0.01)
}

fn fixed_u16(v: f32, scale: f32) -> i32 {
    ((v * scale) as i32).clamp(0, 0xffff)
}

/// `UTIL_ScreenShake` (TS): players on the ground within `radius` (0 = everyone) get a
/// `ScreenShake`; duration and frequency are divided by their time scale.
pub fn screen_shake(center: Vec3, amplitude: f32, frequency: f32, duration: f32, radius: f32) {
    let eng = engine();
    for i in 1..=eng.max_clients() {
        let p = eng.player(i);
        if !is_player(p) {
            continue;
        }
        let v = unsafe { &(*p).v };
        if v.flags & FL_ONGROUND as i32 == 0 {
            continue;
        }
        let mut local = 0.0;
        if radius <= 0.0 {
            local = amplitude;
        } else {
            let dist = vec::length(vec::sub(center, v.origin));
            if dist < radius {
                local = amplitude;
            }
        }
        if local != 0.0 {
            let slow = slow_of(p);
            messages::one(Msg::ScreenShake, p)
                .short(fixed_u16(local, 4096.0))
                .short(fixed_u16(duration / slow, 4096.0))
                .short(fixed_u16(frequency / slow, 256.0))
                .end();
        }
    }
}

pub const FFADE_IN: i32 = 0;
pub const FFADE_OUT: i32 = 1;
pub const FFADE_MODULATE: i32 = 2;
pub const FFADE_STAYOUT: i32 = 4;

/// `UTIL_ScreenFade` (TS): sent as `TSFade` to one player, durations divided by its time scale.
pub fn screen_fade(p: *mut edict_t, color: Vec3, fade_time: f32, hold: f32, alpha: i32, flags: i32) {
    if !is_player(p) || unsafe { (*p).v.flags } & FL_FAKECLIENT as i32 != 0 {
        return; // IsNetClient()
    }
    let slow = slow_of(p);
    messages::one(Msg::TsFade, p)
        .short(fixed_u16(fade_time / slow, 4096.0))
        .short(fixed_u16(hold / slow, 4096.0))
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

fn center(e: *mut edict_t) -> Vec3 {
    let v = unsafe { &(*e).v };
    vec::mid(v.absmin, v.absmax)
}

fn deal(victim: *mut edict_t, inflictor: *mut edict_t, attacker: *mut edict_t, dmg: f32, bits: i32, dir: Vec3, pos: Vec3) {
    let mut info = DamageInfo::new(inflictor, attacker, dmg, bits);
    info.dir = dir;
    info.pos = pos;
    damage::apply(victim, &info);
}

/// TS `TSExplosion(origin, inflictor, attacker, dmg, bits, rFull, rMax)` (0x7d01c): screen shake,
/// a `TSFrags` explosion effect to everyone, a white flash for players caught in it, and damage
/// with full strength inside `r_full`, linear falloff to `r_max`, and penetration through thin
/// walls (damage minus wall thickness).
///
/// Not ported here (other packages): knocking `WorldGun` entities around, and the
/// "spectacular" slow-motion point at the end.
pub fn ts_explosion(origin: Vec3, inflictor: *mut edict_t, attacker: *mut edict_t, dmg: f32, bits: i32, r_full: f32, r_max: f32) {
    let eng = engine();
    let mut org = origin;
    org[2] += 1.0;
    let attacker = if attacker.is_null() { inflictor } else { attacker };
    screen_shake(org, dmg, 80.0, 1.0, r_max * 2.0);
    messages::all(Msg::TsFrags).vec3(org).coord(dmg as i32 as f32).coord(0.0).coord(0.0).byte(1).end();

    let falloff = if r_max != r_full { dmg / (r_max - r_full) } else { 0.0 };
    let mut e = std::ptr::null_mut();
    loop {
        e = eng.find_in_sphere(e, org, r_max);
        if e.is_null() {
            break;
        }
        if e == inflictor {
            continue;
        }
        let (takedamage, deadflag) = unsafe { ((*e).v.takedamage, (*e).v.deadflag) };
        if takedamage == 0.0 || deadflag == DEAD_DEAD as i32 {
            continue;
        }
        if is_player(e) && unsafe { (*e).v.health } > 0.0 {
            screen_fade(e, [222.0, 227.0, 189.0], 0.1, 0.05, 75, 0);
        }
        let mut spot = center(e);
        spot[2] += 1.0;
        let tr = eng.trace_line(org, spot, DONT_IGNORE_MONSTERS, inflictor);
        let dist = vec::length(vec::sub(spot, org));
        let mut d = dmg;
        if dist > r_full {
            d = (dmg - (dist - r_full) * falloff).max(0.0);
        }
        let dir = vec::normalize(vec::sub(spot, org));
        if tr.flFraction == 1.0 || tr.pHit == e {
            deal(e, inflictor, attacker, d, bits, dir, tr.vecEndPos);
        } else {
            // Through a wall: lose damage equal to its thickness.
            let entry = if tr.fStartSolid != 0 { org } else { tr.vecEndPos };
            let back = eng.trace_line(spot, org, DONT_IGNORE_MONSTERS, e);
            if back.flFraction != 1.0 {
                let thickness = vec::length(vec::sub(back.vecEndPos, entry));
                if thickness < d {
                    deal(e, inflictor, attacker, d - thickness, bits, dir, tr.vecEndPos);
                }
            }
        }
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
        let v = unsafe { &(*e).v };
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
