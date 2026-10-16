//! Explosions and screen effects as TS implements them: `TSExplosion` (grenades and breakables), the SDK
//! `RadiusDamage` (env_explosion), `UTIL_ScreenShake` and `UTIL_ScreenFade` (fades sent as
//! `TSFade`; the ruleset decides whether both divide by the player's time scale,
//! [`ScreenEffects`]), `UTIL_Sparks`/`UTIL_Ricochet`, and TS's `EMIT_SOUND_DYN` pitch scaling.

use crate::damage::{self, DamageInfo};
use crate::messages::{self, Msg};
use crate::rules::has_player;
use crate::weapons::worldgun;
use goldsrc::edict::EdictPtr;
use goldsrc::ext::entities::{MsgWriterExt, Vec3, te, te_begin, vec};
use goldsrc::server::{DONT_IGNORE_MONSTERS, engine};
use goldsrc::sys::*;
use shared::pm::{FpuMath, Msvc, ORDER_XYZ, below87};
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
    let pitch = crate::gameplay::active().server.sound_pitch(pitch, e.v().fuser1, s);
    engine().emit_sound(e, chan as i32, &sample, vol, attn, flags, pitch);
}

/// `GiveSlowMul` / `FixedSlowAmt` (0x990bc): the player's time scale, at least 0.01.
fn slow_of(e: *mut edict_t) -> f32 {
    e.v().fuser1.max(0.01)
}

/// `FixedUnsigned16` (0xec8e8): `v * scale` truncated and clamped to 0..=0xffff (an x87
/// out-of-range conversion gives 0).
fn fixed_u16(v: f32, scale: f32) -> i32 {
    goldsrc::ext::entities::fixed_unsigned16(v, scale)
}

/// How a ruleset's `UTIL_ScreenShake` and `UTIL_ScreenFade` treat their receivers
/// ([`ServerRules::screen_effects`](crate::gameplay::ServerRules::screen_effects)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScreenEffects {
    /// The fixed-point durations (and a shake's frequency) are divided by the receiver's time
    /// scale ([`slowed`]).
    pub slowed: bool,
    /// A NaN shake radius shakes everyone and a NaN distance shakes ([`shakes`]); otherwise
    /// neither shakes.
    pub nan_shakes: bool,
}

/// The slow-motion step of TS's `UTIL_ScreenShake`/`UTIL_ScreenFadeWrite`: the already fixed-point
/// value divided by the receiver's `GiveSlowMul()` and truncated. Not clamped again; `WriteShort`
/// keeps the low 16 bits. The quotient stays on the x87 until `__ftol` truncates it (mp.dll
/// `fdivr` at 0x100767ef / 0x1007681d; issue #219), so it is not rounded to a `float` first.
fn slowed(fixed: i32, slow: f32) -> i32 {
    (f64::from(fixed) / f64::from(slow)) as i32
}

/// A fixed-point duration for `p`: [`slowed`] by its `GiveSlowMul()` when `fx` says so.
fn for_receiver(fx: ScreenEffects, fixed: i32, p: *mut edict_t) -> i32 {
    if fx.slowed { slowed(fixed, slow_of(p)) } else { fixed }
}

/// `UTIL_ScreenShake`'s reach: `radius <= 0` shakes everyone, else `(center - origin).Length() <
/// radius` with the unrounded differences summed `(z² + x²) + y²` and the root unrounded (TS 3.0
/// mp.dll 0x10076738..0x10076778, issue #171; TS 2.1 0x10066fd4..0x1006701b sums the same way).
/// `nan_shakes` (3.0: `test ah,0x41` / `test ah,0x1`): a NaN radius shakes everyone and a NaN
/// length shakes; without it (2.1: `jnp` to the distance test, `test ah,0x5; jp`) neither does.
fn shakes(center: Vec3, origin: Vec3, radius: f32, nan_shakes: bool) -> bool {
    use shared::pm::{below87, not_above87};
    let d = [0, 1, 2].map(|i| f64::from(center[i]) - f64::from(origin[i]));
    let len = || ((d[2] * d[2] + d[0] * d[0]) + d[1] * d[1]).sqrt();
    if nan_shakes { not_above87(radius, 0.0) || below87(len(), f64::from(radius)) } else { radius <= 0.0 || len() < f64::from(radius) }
}

/// `RadiusDamage`'s `flDamage - (vecSpot - tr.vecEndPos).Length() * falloff`, at least 0 (mp.dll
/// `FUN_100145f0` 0x1001479d..0x100147ed; issue #171): the unrounded differences summed `(z² +
/// y²) + x²`, the root, the product and the difference on the FPU, rounded once; a negative (or
/// NaN) result is 0.
fn falloff_adjusted(dmg: f32, spot: Vec3, end: Vec3, falloff: f32) -> f32 {
    let d = [0, 1, 2].map(|i| f64::from(end[i]) - f64::from(spot[i]));
    let adjusted = f64::from(dmg) - ((d[2] * d[2] + d[1] * d[1]) + d[0] * d[0]).sqrt() * f64::from(falloff);
    if shared::pm::below87(adjusted, 0.0) { 0.0 } else { adjusted as f32 }
}

/// TS `UTIL_ScreenShake` (0xec978): players on the ground within `radius` (0 = everyone) get a
/// `ScreenShake`; in TS 3.0 the fixed-point duration and frequency are divided by their time
/// scale (TS 2.1's, 0x10066f50, is the SDK's: not divided; [`ScreenEffects`]).
pub fn screen_shake(center: Vec3, amplitude: f32, frequency: f32, duration: f32, radius: f32) {
    let eng = engine();
    let fx = crate::gameplay::active().server.screen_effects();
    let duration = fixed_u16(duration, 4096.0);
    let frequency = fixed_u16(frequency, 256.0);
    for i in 1..=eng.max_clients() {
        let p = eng.player(i);
        if !has_player(p) {
            continue;
        }
        let v = p.v();
        if v.flags & FL_ONGROUND as i32 == 0 {
            continue;
        }
        let local = if shakes(center, v.origin, radius, fx.nan_shakes) { amplitude } else { 0.0 };
        if local != 0.0 {
            messages::one(Msg::ScreenShake, p)
                .short(fixed_u16(local, 4096.0))
                .short(for_receiver(fx, duration, p))
                .short(for_receiver(fx, frequency, p))
                .end();
        }
    }
}

pub const FFADE_IN: i32 = 0;
pub const FFADE_OUT: i32 = 1;
pub const FFADE_MODULATE: i32 = 2;
pub const FFADE_STAYOUT: i32 = 4;

/// TS `UTIL_ScreenFade` (0xecdec, `UTIL_ScreenFadeBuild`/`UTIL_ScreenFadeWrite`): sent as `TSFade`
/// to one net client; in TS 3.0 the fixed-point durations are divided by its time scale (TS 2.1's,
/// 0x10067290 → `FUN_10067180`, is the SDK's: not divided; [`ScreenEffects`]).
pub fn screen_fade(p: *mut edict_t, color: Vec3, fade_time: f32, hold: f32, alpha: i32, flags: i32) {
    if !has_player(p) || p.v().flags & FL_FAKECLIENT as i32 != 0 {
        return; // IsNetClient()
    }
    let fx = crate::gameplay::active().server.screen_effects();
    messages::one(Msg::TsFade, p)
        .short(for_receiver(fx, fixed_u16(fade_time, 4096.0), p))
        .short(for_receiver(fx, fixed_u16(hold, 4096.0), p))
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

/// `ClearMultiDamage`, `victim->TraceAttack(attacker, dmg, dir, &tr, bits)`,
/// `ApplyMultiDamage(inflictor, attacker)`: the impact point and hitgroup come from `tr`.
fn trace_attack(victim: *mut edict_t, inflictor: *mut edict_t, attacker: *mut edict_t, dmg: f32, bits: i32, dir: Vec3, tr: &TraceResult) {
    let mut info = DamageInfo::new(inflictor, attacker, dmg, bits).traced();
    info.dir = dir;
    info.pos = tr.vecEndPos;
    info.hitgroup = tr.iHitgroup;
    damage::apply(victim, &info);
}

/// The damage `TSExplosion` deals at `dist` from the blast (0x7d32a..0x7d374): full inside
/// `r_full`, then `dmg − (dist − r_full) · dmg / (r_max − r_full)`, never below 0. Like TS there is
/// no guard for `r_max == r_full` (no caller passes it): the infinite falloff zeroes everything
/// outside `r_full`.
///
/// In mp.dll's arithmetic (issue #150): the falloff is stored as a `float` (0x1005154a..0x1005156f);
/// `dist` (from [`blast_distance`]) stays unrounded through `dmg - (dist - r_full)·falloff`, which
/// is rounded once (0x100516d9..0x10051715). A NaN distance deals `dmg`, a NaN result 0.
pub fn falloff_damage(dmg: f32, dist: f64, r_full: f32, r_max: f32) -> f32 {
    let falloff = (dmg as f64 / (r_max as f64 - r_full as f64)) as f32 as f64;
    if dist > r_full as f64 {
        let d = dmg as f64 - (dist - r_full as f64) * falloff;
        // `test ah,0x41`: at most 0, or NaN, is 0.
        if d > 0.0 { d as f32 } else { 0.0 }
    } else {
        dmg
    }
}

/// `TSExplosion`'s distance from the blast `org` to the target's `spot`: the differences stored
/// as `float`s, `z² + y² + x²`, the root unrounded (mp.dll 0x100516bb..0x100516d7).
pub fn blast_distance(org: Vec3, spot: Vec3) -> f64 {
    let d = vec::sub(org, spot).map(f64::from);
    (d[2] * d[2] + d[1] * d[1] + d[0] * d[0]).sqrt()
}

/// The thickness of what occludes a target: `|a - b|` from the unrounded differences
/// (`z² + y² + x²`), stored as a `float` (mp.dll 0x100517da..0x1005180d).
fn occluder_thickness(a: Vec3, b: Vec3) -> f32 {
    let d = [0, 1, 2].map(|i| a[i] as f64 - b[i] as f64);
    (d[2] * d[2] + d[1] * d[1] + d[0] * d[0]).sqrt() as f32
}

#[cfg(test)]
mod explosion_tests {
    use super::*;

    #[test]
    fn explosion_falloff() {
        // The grenade: 110 full inside 300, linear to 0 at 1024.
        assert_eq!(falloff_damage(110.0, 0.0, 300.0, 1024.0), 110.0);
        assert_eq!(falloff_damage(110.0, 300.0, 300.0, 1024.0), 110.0);
        assert!((falloff_damage(110.0, 662.0, 300.0, 1024.0) - 55.0).abs() < 1e-4);
        // At r_max the stored `float` falloff (110/724 rounded) leaves a trace of damage, as in
        // mp.dll: 110 - 724·0.15193370f = 1.3113022e-6.
        assert_eq!(falloff_damage(110.0, 1024.0, 300.0, 1024.0), 1.311_302_2e-6);
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
        // The super-jump landing shake (0.3 s) at a 0.35 time scale: 1228 / 0.35 = 3508.57.
        assert_eq!(slowed(fixed_u16(0.3, 4096.0), 0.35), 3508);
        // No clamp after the division: `WriteShort` keeps the low 16 bits.
        assert_eq!(slowed(fixed_u16(1.0, 4096.0), 0.01), 409_600);
        // #219: the quotient isn't rounded to a `float` before the truncation: 1228 /
        // 0.34995726 (0x3eb32d99) = 3508.99995, which a `float` makes 3509.
        assert_eq!(slowed(1228, f32::from_bits(0x3eb3_2d99)), 3508);
        assert_eq!(fixed_u16(20.0, 4096.0), 0xffff);
        // Past i32's range x87 gives 0x80000000, which the clamp takes to 0.
        assert_eq!(fixed_u16(1.0e6, 4096.0), 0);
    }
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
///
/// The ruleset decides the world-gun push and what follows the damage
/// (`ServerRules::explosion_pushes_world_guns`, `explosion_aftermath`).
pub fn ts_explosion(origin: Vec3, inflictor: *mut edict_t, attacker: *mut edict_t, dmg: f32, bits: i32, r_full: f32, r_max: f32) {
    let eng = engine();
    let rules = crate::gameplay::active().server;
    let push_guns = rules.explosion_pushes_world_guns();
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
        if push_guns && !e.is_free() && eng.string(e.v().classname).to_bytes() == b"WorldGun" {
            worldgun::blast(e, push_from, r_max);
        }
    }
    rules.explosion_aftermath(org, inflictor, attacker);
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
    if has_player(e) && v.deadflag == DEAD_NO as i32 && v.health > 0.0 {
        screen_fade(e, [222.0, 227.0, 189.0], 0.1, 0.05, 75, 0);
    }
    let mut spot = center(e);
    spot[2] += 1.0;
    let tr = eng.trace_line(org, spot, DONT_IGNORE_MONSTERS, inflictor);
    let d = falloff_damage(dmg, blast_distance(org, spot), r_full, r_max);
    // The out-of-line `Normalize` (0x100148f0) of the stored difference, mp.dll's `x² + y² + z²`.
    let dir = Msvc::vec_normalize(vec::sub(spot, org), ORDER_XYZ);
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
    let thickness = occluder_thickness(back.vecEndPos, entry);
    // `test ah,0x1`: a NaN thickness counts as thinner (0x1005181b).
    if below87(thickness, d) {
        trace_attack(e, inflictor, attacker, d - thickness, bits, dir, &back);
    }
}

/// Which `BodyTarget` (vtable +0x104) an entity has: only `CBasePlayer` (0x9736c; `mp.dll`
/// 0x1000fc90) and `CGunTarget` (0xd7d84; 0x1002a3a0) override `CBaseEntity`'s (0xacc28).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum BodyKind {
    Player,
    GunTarget,
    Other,
}

fn body_kind(e: *mut edict_t) -> BodyKind {
    if has_player(e) {
        BodyKind::Player
    } else if engine().string(e.v().classname) == c"func_guntarget" {
        BodyKind::GunTarget
    } else {
        BodyKind::Other
    }
}

/// `BodyTarget(vecSrc)`: a player's is `Center() + view_ofs * RANDOM_FLOAT(0.5, 1.1)` (each
/// product stored as a float, then added: plain `f32` maths), drawn only for players; a gun
/// target's its origin; anything else's `Center()`.
fn body_target_of(kind: BodyKind, center: Vec3, origin: Vec3, view_ofs: Vec3, random: impl FnOnce() -> f32) -> Vec3 {
    match kind {
        BodyKind::Player => vec::mad(center, view_ofs, random()),
        BodyKind::GunTarget => origin,
        BodyKind::Other => center,
    }
}

fn body_target(e: *mut edict_t) -> Vec3 {
    let (kind, c) = (body_kind(e), center(e));
    let v = e.v();
    body_target_of(kind, c, v.origin, v.view_ofs, || engine().random_float(0.5, 1.1))
}

/// SDK `RadiusDamage(vecSrc, inflictor, attacker, dmg, radius, CLASS_NONE, bits)` (env_explosion;
/// 0xb2d64): traces to and measures from the target's `BodyTarget` ([`body_target`], 0xb2e82).
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
        let spot = body_target(e);
        let mut tr = eng.trace_line(src, spot, DONT_IGNORE_MONSTERS, inflictor);
        if tr.flFraction == 1.0 || tr.pHit == e {
            if tr.fStartSolid != 0 {
                tr.vecEndPos = src;
                tr.flFraction = 0.0;
            }
            let adjusted = falloff_adjusted(dmg, src, tr.vecEndPos, falloff);
            if tr.flFraction != 1.0 {
                // TraceAttack path (0xb3052): direction, impact point and hitgroup are known. The
                // out-of-line `Normalize` (mp.dll 0x10014839; issue #171).
                let dir = damage::normalize(vec::sub(tr.vecEndPos, src));
                trace_attack(e, inflictor, attacker, adjusted, bits, dir, &tr);
            } else {
                // Plain TakeDamage (0xb3090).
                damage::apply(e, &DamageInfo::new(inflictor, attacker, adjusted, bits));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BodyKind, body_target_of};

    /// #137: `RadiusDamage` aims at `BodyTarget`, which only players draw a random number for.
    #[test]
    fn radius_damage_aims_at_the_body_target() {
        let (c, o, ofs) = ([10.0, 20.0, 30.0], [1.0, 2.0, 3.0], [0.0, 0.0, 28.0]);
        assert_eq!(body_target_of(BodyKind::Player, c, o, ofs, || 0.5), [10.0, 20.0, 44.0]);
        assert_eq!(body_target_of(BodyKind::Player, c, o, ofs, || 1.1), [10.0, 20.0, 30.0 + 28.0 * 1.1f32]);
        let no_draw = || -> f32 { panic!("only players draw") };
        assert_eq!(body_target_of(BodyKind::GunTarget, c, o, ofs, no_draw), o);
        assert_eq!(body_target_of(BodyKind::Other, c, o, ofs, no_draw), c);
    }

    /// Every TS server sound that reaches `pfnEmitSound` goes through `EMIT_SOUND_DYN` (0xe0a8c)
    /// and its slow-motion factor (issue #33), so in the port only [`super::emit_sound_slow`]
    /// may call `emit_sound`. Ambient sounds (`emit_ambient_sound`) go straight to the engine,
    /// as `UTIL_EmitAmbientSound` does in TS.
    #[test]
    fn only_emit_sound_slow_calls_the_engine() {
        // Split so this test's own source doesn't match.
        const CALL: &str = concat!(".emit", "_sound(");
        fn walk(dir: &std::path::Path, out: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(&path, out);
                } else if path.extension().is_some_and(|x| x == "rs") {
                    let text = std::fs::read_to_string(&path).unwrap();
                    for (i, line) in text.lines().enumerate() {
                        if line.contains(CALL) {
                            out.push(format!("{}:{}: {}", path.display(), i + 1, line.trim()));
                        }
                    }
                }
            }
        }
        let mut hits = Vec::new();
        walk(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut hits);
        hits.retain(|h| !h.contains("engine().emit_sound(e, chan as i32, &sample, vol, attn, flags, pitch)"));
        assert!(hits.is_empty(), "sounds that skip EMIT_SOUND_DYN's slow-motion pitch:\n{}", hits.join("\n"));
    }
}

#[cfg(test)]
mod precision_tests {
    use super::*;

    /// `RadiusDamage`'s falloff and `UTIL_ScreenShake`'s reach (issue #171).
    #[test]
    fn falloff_and_shake_reach_round_as_mp_dll() {
        let (spot, end) = (f([0x43c4_deef, 0xc349_360b, 0xc30a_cf60]), f([0xc3a7_05a0, 0xc3b1_2628, 0xc3d9_6e1e]));
        assert_eq!(falloff_adjusted(f32::from_bits(0x4342_fc12), spot, end, f32::from_bits(0x3e1f_3505)).to_bits(), 0x428d_0fd5);
        assert_eq!(falloff_adjusted(10.0, [0.0; 3], [100.0, 0.0, 0.0], 1.0), 0.0);
        for nan_shakes in [true, false] {
            assert!(!shakes([0.0; 3], [100.0, 0.0, 0.0], 100.0, nan_shakes));
            assert!(shakes([0.0; 3], [99.0, 0.0, 0.0], 100.0, nan_shakes));
            assert!(shakes([0.0; 3], [1e9, 0.0, 0.0], 0.0, nan_shakes), "radius 0: everyone");
        }
        // 3.0 (mp.dll): a NaN radius shakes everyone, a NaN distance shakes.
        assert!(shakes([0.0; 3], [100.0, 0.0, 0.0], f32::NAN, true));
        assert!(shakes([f32::NAN, 0.0, 0.0], [100.0, 0.0, 0.0], 50.0, true));
        // 2.1 (0x10066fe3 `jnp`, 0x1006701b `test ah,0x5; jp`): neither does.
        assert!(!shakes([0.0; 3], [100.0, 0.0, 0.0], f32::NAN, false));
        assert!(!shakes([f32::NAN, 0.0, 0.0], [100.0, 0.0, 0.0], 50.0, false));
    }

    fn f(b: [u32; 3]) -> [f32; 3] {
        b.map(f32::from_bits)
    }

    fn b(v: [f32; 3]) -> [u32; 3] {
        v.map(f32::to_bits)
    }

    /// `TSExplosion` keeps the distance unrounded through the falloff (issue #150): the old
    /// `float` steps gave 0x41b28c38.
    #[test]
    fn falloff_from_the_unrounded_distance() {
        let (org, spot) = (f([0x43af_fab4, 0xc413_8fd3, 0x4397_a868]), f([0xc3ac_6bb9, 0xc377_32b8, 0x4431_b77e]));
        assert_eq!(falloff_damage(110.0, blast_distance(org, spot), 300.0, 1024.0).to_bits(), 0x41b2_8c36);
        assert_eq!(falloff_damage(110.0, f64::NAN, 300.0, 1024.0), 110.0, "a NaN distance isn't beyond r_full");
        assert_eq!(occluder_thickness([3.0, 0.0, 0.0], [0.0, 4.0, 0.0]), 5.0);
        let _ = b(org);
    }
}
