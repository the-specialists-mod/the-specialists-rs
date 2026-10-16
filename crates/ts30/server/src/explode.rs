//! TS 3.0's explosions and thrown blades.

use shared::pm::{FpuMath, Msvc, ORDER_XYZ, below87};
use shared::weapons::spread::Vec3;
use ts_server::entities::stock::explode::ScreenEffects;
use ts_server::weapons::ThrownBlade;

/// TS 3.0's `UTIL_ScreenShake` (0xec978, mp.dll 0x100766b0: `fdivr` by `GiveSlowMul()` at
/// 0x100767ef / 0x1007681d) and `UTIL_ScreenFadeWrite` (0xecc54, mp.dll `FUN_10076900`) divide the
/// fixed-point durations and frequency by the receiver's time scale; a NaN radius shakes everyone
/// and a NaN distance shakes (mp.dll 0x10076738..0x10076778, `test ah,0x41` / `test ah,0x1`).
pub const SCREEN_EFFECTS: ScreenEffects = ScreenEffects { slowed: true, nan_shakes: true };

/// The end of `TSExplosion` (0x7d964/0x7d9ab): `DoSpectacularNess()`, then
/// `CreateSlowMotionPoint(origin + 1z, 0.25, 1.0, 0)`.
pub fn explosion_slow_point(org: Vec3) {
    if ts_server::rules::scoring::do_spectacularness() {
        ts_server::entities::ts::create_slow_motion_point(org, 0.25, 1.0, false);
    }
}

/// The speed of a thrown blade: `KnifeFire` throws knives at 1900 u/s (objdump 0x7465b /
/// 0x746a7), `KatanaThrow` the katana at 1000 u/s (0x73dc1, the float at 0xf7f64).
fn throw_speed(btype: i32) -> f32 {
    if btype == 8 { 1000.0 } else { 1900.0 }
}

/// The thrown blades: [`throw_speed`]; gravity 0.85 katana, 0.75 combat knife, 0.3 seal
/// knife (`CTSKnife::Spawn` 0x7db54); bits `0x08001000` (thrown: the ×2 headshot).
pub fn thrown_blade(btype: i32) -> ThrownBlade {
    let gravity = match btype {
        8 => 0.85,
        6 => 0.75,
        _ => 0.3,
    };
    ThrownBlade { speed: throw_speed(btype), gravity, bits: 0x0800_1000 }
}

/// The thrown-blade damage (`CTSKnife::BounceTouch` 0x7decc): 150 katana, 100 combat
/// knife, 50 seal knife, however far it flew.
pub fn thrown_blade_damage(btype: i32) -> f32 {
    match btype {
        8 => 150.0,
        6 => 100.0,
        _ => 50.0,
    }
}

/// `TSExplosion`'s hit on one target ([`ServerRules::explosion_hit`](ts_server::gameplay::ServerRules::explosion_hit))
/// in mp.dll's arithmetic: [`falloff_damage`] by [`blast_distance`], the direction the
/// out-of-line `Normalize` (0x100148f0) of the stored difference `spot − org` (`x² + y² + z²`,
/// `1/len` on the FPU), and behind an occluder `d − thickness` when `thickness < d` (`test
/// ah,0x1`: a NaN thickness counts as thinner, 0x1005181b).
pub fn explosion_hit(org: Vec3, spot: Vec3, dmg: f32, r_full: f32, r_max: f32, occluder: Option<f32>) -> Option<(f32, Vec3)> {
    let d = falloff_damage(dmg, blast_distance(org, spot), r_full, r_max);
    let dir = Msvc::vec_normalize([spot[0] - org[0], spot[1] - org[1], spot[2] - org[2]], ORDER_XYZ);
    match occluder {
        None => Some((d, dir)),
        Some(t) => below87(t, d).then_some((d - t, dir)),
    }
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
    let d = [org[0] - spot[0], org[1] - spot[1], org[2] - spot[2]].map(f64::from);
    (d[2] * d[2] + d[1] * d[1] + d[0] * d[0]).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn katana_is_thrown_slower_than_knives() {
        assert_eq!(throw_speed(6), 1900.0);
        assert_eq!(throw_speed(7), 1900.0);
        assert_eq!(throw_speed(8), 1000.0);
    }
}

#[cfg(test)]
mod explosion_tests {
    use super::*;

    fn f(b: [u32; 3]) -> [f32; 3] {
        b.map(f32::from_bits)
    }

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

    /// `TSExplosion` keeps the distance unrounded through the falloff (issue #150): the old
    /// `float` steps gave 0x41b28c38.
    #[test]
    fn falloff_from_the_unrounded_distance() {
        let (org, spot) = (f([0x43af_fab4, 0xc413_8fd3, 0x4397_a868]), f([0xc3ac_6bb9, 0xc377_32b8, 0x4431_b77e]));
        assert_eq!(falloff_damage(110.0, blast_distance(org, spot), 300.0, 1024.0).to_bits(), 0x41b2_8c36);
        assert_eq!(falloff_damage(110.0, f64::NAN, 300.0, 1024.0), 110.0, "a NaN distance isn't beyond r_full");
    }

    /// The inputs where 2.1 rounds differently (#220): mp.dll's stored differences and `1/len`
    /// on the FPU (2.1: 0x420914f9 and x = 0xbe6c90a0).
    #[test]
    fn rounds_as_mp_dll_where_2_1_differs() {
        let (org, spot) = (f([0xc4dd_0023, 0x41ed_f187, 0xc4e7_408e]), f([0xc4e6_f424, 0xc3f3_371d, 0xc512_5296]));
        assert_eq!(explosion_hit(org, spot, 100.0, 128.0, 1024.0, None).unwrap().0.to_bits(), 0x4209_14f8);
        let (org, spot) = (f([0xc396_f62d, 0x44a3_6d12, 0xc4bc_1959]), f([0xc48a_616a, 0x43fe_ddcf, 0x44df_dabf]));
        let dir = explosion_hit(org, spot, 100.0, 128.0, 1024.0, None).unwrap().1;
        assert_eq!(dir.map(f32::to_bits), [0xbe6c_909f, 0xbe6a_6095, 0x3f72_15b1]);
    }

    #[test]
    fn hit_behind_an_occluder_loses_its_thickness() {
        let (org, spot) = ([0.0; 3], [100.0, 0.0, 0.0]);
        assert_eq!(explosion_hit(org, spot, 110.0, 300.0, 1024.0, None), Some((110.0, [1.0, 0.0, 0.0])));
        assert_eq!(explosion_hit(org, spot, 110.0, 300.0, 1024.0, Some(10.0)), Some((100.0, [1.0, 0.0, 0.0])));
        assert_eq!(explosion_hit(org, spot, 110.0, 300.0, 1024.0, Some(110.0)), None);
        assert!(explosion_hit(org, spot, 110.0, 300.0, 1024.0, Some(f32::NAN)).is_some(), "a NaN thickness is thinner");
        // A zero difference normalises to straight up.
        assert_eq!(explosion_hit(org, org, 110.0, 300.0, 1024.0, None).unwrap().1, [0.0, 0.0, 1.0]);
    }
}
