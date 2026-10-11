//! Seeded pellet directions (the TS `ShootBulletsPlayer` / client `EV_TS_FireBullets` spread),
//! recoil (`VKickback`), the recoil penalty, timer steps and the small vector helpers the weapon
//! code needs. Pure functions, so the server and the client prediction get the same bits; the
//! ones whose rounding differs between TS's builds take an [`Arith`].

use super::random::shared_random_float;
use super::{kind, tables};
use crate::pm::{Arith, Fpu, Gcc, Msvc, Num};

pub type Vec3 = [f32; 3];

pub fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub fn scale(a: Vec3, s: f32) -> Vec3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
/// `a + b * s`
pub fn ma(a: Vec3, s: f32, b: Vec3) -> Vec3 {
    [a[0] + b[0] * s, a[1] + b[1] * s, a[2] + b[2] * s]
}
pub fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub fn length(a: Vec3) -> f32 {
    dot(a, a).sqrt()
}
/// SDK `Vector::Normalize` (zero vector -> (0, 0, 1)).
pub fn normalize(a: Vec3) -> Vec3 {
    let l = length(a);
    if l == 0.0 { [0.0, 0.0, 1.0] } else { scale(a, 1.0 / l) }
}

/// SDK `AngleVectors` (pitch, yaw, roll in degrees) -> (forward, right, up).
pub fn angle_vectors(angles: Vec3) -> (Vec3, Vec3, Vec3) {
    let r = |d: f32| (d as f64 * (std::f64::consts::PI * 2.0 / 360.0)) as f32;
    let (sy, cy) = r(angles[1]).sin_cos();
    let (sp, cp) = r(angles[0]).sin_cos();
    let (sr, cr) = r(angles[2]).sin_cos();
    let forward = [cp * cy, cp * sy, -sp];
    let right = [-sr * sp * cy + -cr * -sy, -sr * sp * sy + -cr * cy, -sr * cp];
    let up = [cr * sp * cy + -sr * -sy, cr * sp * sy + -sr * cy, cr * cp];
    (forward, right, up)
}

/// Offsets of pellet `i` (1-based) in units of the spread: `(cos t * m, sin t * m)` with
/// `t = SR(seed+i, -pi, pi)`, `r = SR(seed+i+1, 0, radial)`, `m = 1 - r^3` (weapons.md §5.4,
/// client `EV_TS_FireBullets`). `radial` is the ammo's radial spread scale (1.0 for all ammo).
pub fn pellet_offset(seed: u32, i: u32, radial: f32) -> (f32, f32) {
    let pi = std::f32::consts::PI;
    let t = shared_random_float(seed.wrapping_add(i), -pi, pi) as f64;
    let r = shared_random_float(seed.wrapping_add(i).wrapping_add(1), 0.0, radial) as f64;
    let m = 1.0 - r * r * r;
    ((t.cos() * m) as f32, (t.sin() * m) as f32)
}

/// Direction of pellet `i`: `aim + right * x * spread_x + up * y * spread_y` (not normalised,
/// as in TS).
pub fn pellet_dir(aim: Vec3, right: Vec3, up: Vec3, spread: [f32; 2], seed: u32, i: u32, radial: f32) -> Vec3 {
    let (x, y) = pellet_offset(seed, i, radial);
    let sx = x * spread[0];
    let sy = y * spread[1];
    [aim[0] + sx * right[0] + sy * up[0], aim[1] + sx * right[1] + sy * up[1], aim[2] + sx * right[2] + sy * up[2]]
}

/// What `ShootBulletsPlayer` returns: `(x * spread_x, y * spread_y, 0)` of the last pellet.
pub fn last_pellet_spread(seed: u32, pellets: u32, spread: [f32; 2], radial: f32) -> Vec3 {
    if pellets == 0 {
        return [0.0; 3];
    }
    let (x, y) = pellet_offset(seed, pellets, radial);
    [x * spread[0], y * spread[1], 0.0]
}

// ---- the TS builds' arithmetic -----------------------------------------------------------------
//
// The server (`ts_i386.so`: GCC, x87 extended precision; `mp.dll`: MSVC, 53-bit) and the client
// prediction (`client.dll`, MSVC) compile the weapon code separately, so a `float` expression can
// round differently. Where it matters for a value both sides compute, the function below takes
// the [`Arith`] of the build it stands in for, as player movement does (`shared::pm::fpu`): the
// server passes its `ts_pm_arith` selection (default `Msvc`, `mp.dll`), the client `Arith::CLIENT`.

macro_rules! by_arith {
    ($arith:expr, $f:ident($($a:expr),* $(,)?)) => {
        match $arith {
            Arith::Gcc => $f::<Gcc>($($a),*),
            Arith::Msvc => $f::<Msvc>($($a),*),
        }
    };
}

/// `FixedSlowAmt` (0x990bc; client `0x10055aa0`), behind every `GiveSlowMul`: the time scale with
/// a floor of 0.01. (`ts_i386.so` substitutes the `double` 0.01, `client.dll`/`mp.dll` the `float`:
/// below 0.01 the port takes the `float`.)
pub fn fixed_slow_amt(fuser1: f32) -> f32 {
    if fuser1 < 0.01 { 0.01 } else { fuser1 }
}

/// The weapon timer step (`CBasePlayer::PostThink` 0x91ae7 and `PreThink` 0x909c5, `mp.dll`
/// 0x1006053c, client `0x10003ec5`): `GiveSlowMul() * frametime`, ×1.5 with double fire rate.
/// GCC multiplies the unrounded product by 1.5; MSVC stores it as a `float` first.
pub fn timer_step(arith: Arith, frametime: f32, slowmul: f32, double_fire_rate: bool) -> f32 {
    fn go<F: Fpu>(ft: f32, slow: f32, dfr: bool) -> f32 {
        let p = F::x(slow) * F::x(ft);
        match (dfr, F::MSVC) {
            (false, _) => p.f(),
            (true, false) => (p * F::x(1.5)).f(),
            (true, true) => (F::x(p.f()) * F::x(1.5)).f(),
        }
    }
    by_arith!(arith, go(frametime, slowmul, double_fire_rate))
}

/// `PreThink`'s recoil-penalty decay (0x80d88, `mp.dll` 0x1005fa29, client `0x100040a5`):
/// `pen - dt * 0.1f` rounded once; the unrounded result below 0 clears it.
pub fn decay_spread_penalty(arith: Arith, pen: f32, dt: f32) -> f32 {
    fn go<F: Fpu>(pen: f32, dt: f32) -> f32 {
        // GCC tests `!(0 >= pen)`, MSVC `pen > 0` (they differ only for a NaN).
        let run = if F::MSVC { pen > 0.0 } else { pen > 0.0 || pen.is_nan() };
        if !run {
            return pen;
        }
        let v = F::x(pen) - F::x(dt) * F::x(0.1);
        // MSVC's `test ah,1` also clears a NaN; GCC's `fcompp; test ah,0x41` keeps it.
        let below = if F::MSVC { v.partial_cmp(&F::ZERO).is_none_or(|o| o.is_lt()) } else { v < F::ZERO };
        if below { 0.0 } else { v.f() }
    }
    by_arith!(arith, go(pen, dt))
}

/// `PreThink`'s melee-bar regeneration (0x80ddc, `mp.dll` 0x1005fa65): below 100, `bar + dt * 15`
/// (`+ dt` while below -0.001) rounded once, clamped to 100.
pub fn regen_melee_bar(arith: Arith, bar: f32, dt: f32) -> f32 {
    fn go<F: Fpu>(bar: f32, dt: f32) -> f32 {
        if bar >= 100.0 {
            return bar;
        }
        let step = if bar >= -0.001 { F::x(dt) * F::x(15.0) } else { F::x(dt) };
        let v = (step + F::x(bar)).f();
        if v > 100.0 { 100.0 } else { v }
    }
    by_arith!(arith, go(bar, dt))
}

/// `CTSGun::ScaleSpread` (0x802a8, `mp.dll` 0x10053ee0, client `0x10008100`), the time since the
/// last shot: `None` below 1 ms (the penalty isn't touched), else the elapsed time as the `float`
/// the rest uses. GCC tests the stored `float`, MSVC the unrounded difference.
pub fn shot_interval(arith: Arith, now: f32, last_shot: f32) -> Option<f32> {
    fn go<F: Fpu>(now: f32, last: f32) -> Option<f32> {
        let d = F::x(now) - F::x(last);
        let tested = if F::MSVC { d } else { F::x(d.f()) };
        (tested >= F::x(0.001)).then(|| d.f())
    }
    by_arith!(arith, go(now, last_shot))
}

/// `ScaleSpread`'s penalty growth after [`shot_interval`]: `x = slowmul * elapsed` (0.5 from 1
/// up), `pen + (1 - x) * akick * k`, clamped to the `double` 0.1 and stored once. GCC rounds `x`
/// to a `float` before `1 - x` and multiplies by `akick` first; MSVC keeps `x` unrounded and
/// multiplies by `k` first.
pub fn grow_spread_penalty(arith: Arith, pen: f32, slowmul: f32, elapsed: f32, akick: f32, k: f32) -> f32 {
    fn go<F: Fpu>(pen: f32, slow: f32, elapsed: f32, akick: f32, k: f32) -> f32 {
        let p = F::x(slow) * F::x(elapsed);
        // `fcom 1.0`: a NaN product counts as below 1 in both builds.
        let x = if p >= F::x(1.0) {
            F::x(0.5)
        } else if F::MSVC {
            p
        } else {
            F::x(p.f())
        };
        let t = F::x(1.0) - x;
        let v = if F::MSVC { t * F::x(k) * F::x(akick) } else { t * F::x(akick) * F::x(k) } + F::x(pen);
        let max = F::xd(0.1);
        (if v > max { max } else { v }).f()
    }
    by_arith!(arith, go(pen, slowmul, elapsed, akick, k))
}

/// `CTSGun::VKickback` (0x80414, `mp.dll` 0x10054000, client `0x10008230`): (pitch, yaw) view
/// kick in degrees for weapon kind `k`. The pitch is subtracted from `v_angle.x`, the yaw added
/// to `v_angle.y`. The yaw is `0.85 * kick` unrounded times the kind's factor; with a silencer,
/// MSVC also scales the unrounded product for the pitch where GCC scales the stored `float`.
pub fn view_kick(arith: Arith, view_kick: f32, k: u8, silenced: bool, double_fire_rate: bool) -> (f32, f32) {
    fn go<F: Fpu>(kick: f32, k: u8, silenced: bool, dfr: bool) -> (f32, f32) {
        let product = F::x(kick) * F::x(0.85);
        let mut pitch = product.f();
        let factor = match k {
            kind::PISTOL => 0.35,
            kind::AKIMBO | kind::AKIMBO_ONLY => 0.15,
            kind::SHOTGUN => -0.25,
            kind::RIFLE | kind::SNIPER => -0.3,
            kind::SMG => -0.35,
            _ => 0.0,
        };
        let mut yaw = if factor == 0.0 { 0.0 } else { (product * F::x(factor)).f() };
        if silenced {
            pitch = if F::MSVC { product * F::x(0.775) } else { F::x(pitch) * F::x(0.775) }.f();
            yaw = (F::x(yaw) * F::x(0.775)).f();
        }
        if dfr {
            pitch = (F::x(pitch) * F::x(0.666_666_7)).f();
            yaw = (F::x(yaw) * F::x(0.666_666_7)).f();
        }
        (pitch, yaw)
    }
    by_arith!(arith, go(view_kick, k, silenced, double_fire_rate))
}

/// `dmg * CBasePlayer::GetMeleeMultiplier()` (0x965d0, `mp.dll` 0x10063e60): the multiplier
/// `(max(bar, 0) * 0.5 + 50) * 0.01f` (0.5 with an empty bar, 1.0 with a full one) stays on the
/// FPU and the product is stored once.
pub fn melee_damage(arith: Arith, dmg: f32, bar: f32) -> f32 {
    fn go<F: Fpu>(dmg: f32, bar: f32) -> f32 {
        // GCC keeps a NaN bar (`jbe`), MSVC zeroes it (`fcomp 0; test ah,1`).
        let b = F::x(bar);
        let b = if F::MSVC {
            if b >= F::ZERO { b } else { F::ZERO }
        } else if b < F::ZERO {
            F::ZERO
        } else {
            b
        };
        ((b * F::x(0.5) + F::x(50.0)) * F::x(0.01) * F::x(dmg)).f()
    }
    by_arith!(arith, go(dmg, bar))
}

/// `GetBulletVelocityMult(slowmul, bullet_time)` 0x99160 for simulated bullets.
pub fn bullet_velocity_mult(slowmul: f32, bullet_time: bool) -> f32 {
    if slowmul >= 0.99 {
        return 1.0;
    }
    let d = ((1.0 - slowmul * slowmul) * if bullet_time { 10.0 } else { 1.0 }).max(1.0);
    slowmul / d
}

/// Ammo constants shared by every type (weapons.md §3): trace length and radial spread scale.
pub const MAX_RANGE: f32 = 8192.0;
pub const RADIAL_SPREAD: f32 = 1.0;
pub const UNITS_PER_METER: f32 = 40.0;

/// Range-falloff index for distance `dist` with the ammo's range unit
/// (`round(dist / (unit * 10) * 10)`, round-to-nearest like the x87 default).
pub fn range_index(dist: f32, unit: f32) -> i32 {
    let d = dist / (unit * 10.0);
    (d * 10.0).round_ties_even() as i32
}

/// Range unit of ammo type `ammo` (80 for 9mm if unknown).
pub fn ammo_range_unit(ammo: u8) -> f32 {
    tables().ammo(ammo).map(|a| a.range_unit).unwrap_or(80.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vectors() {
        let (f, r, u) = angle_vectors([0.0, 90.0, 0.0]);
        assert!((f[1] - 1.0).abs() < 1e-6 && f[0].abs() < 1e-6);
        assert!((r[0] - 1.0).abs() < 1e-6);
        assert!((u[2] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn pellets_are_deterministic_and_inside_the_cone() {
        for seed in [0u32, 1, 77, 31999] {
            for i in 1..=8 {
                let (x, y) = pellet_offset(seed, i, 1.0);
                assert!(x * x + y * y <= 1.0 + 1e-6);
                assert_eq!(pellet_offset(seed, i, 1.0), (x, y));
            }
        }
        let d = pellet_dir([1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0], 5, 1, 1.0);
        assert_eq!(d, [1.0, 0.0, 0.0]);
    }

    // Reference bits below are exact models of the x87 at 64 bits (`ts_i386.so`) and 53 bits
    // (`mp.dll`, `client.dll`), each operation rounded once, in the machine code's order.

    #[test]
    fn kick() {
        for arith in [Arith::Gcc, Arith::Msvc] {
            let (p, y) = view_kick(arith, 10.0, kind::PISTOL, false, false);
            assert!((p - 8.5).abs() < 1e-5 && (y - 2.975).abs() < 1e-5);
            let (p, _) = view_kick(arith, 10.0, kind::RIFLE, true, true);
            assert!((p - 8.5 * 0.775 * 0.6666667).abs() < 1e-4);
            assert_eq!(view_kick(arith, 10.0, 0, false, false).1, 0.0, "no yaw for other kinds");
            // The yaw multiplies the unrounded 0.85 * kick (f32 steps give 0x3ee47ae2).
            let (p, y) = view_kick(arith, 1.5, kind::PISTOL, false, false);
            assert_eq!((p.to_bits(), y.to_bits()), (0x3fa3_3334, 0x3ee4_7ae1));
        }
        // Silenced: GCC scales the stored pitch, MSVC the unrounded product.
        let bits = |a| {
            let (p, y) = view_kick(a, 5.16, kind::PISTOL, true, false);
            (p.to_bits(), y.to_bits())
        };
        assert_eq!(bits(Arith::Gcc), (0x4059_8bad, 0x3f98_482b));
        assert_eq!(bits(Arith::Msvc), (0x4059_8bac, 0x3f98_482b));
    }

    #[test]
    fn timer_step_per_build() {
        let ft = (0.001f64) as f32; // msec 1
        assert_eq!(timer_step(Arith::Gcc, ft, 1.0, false), ft);
        assert_eq!(timer_step(Arith::Msvc, 0.01, 0.5, false), 0.005);
        // Double fire rate in slow motion: GCC scales the unrounded product by 1.5.
        assert_eq!(timer_step(Arith::Gcc, ft, 0.65, true).to_bits(), 0x3a7f_9725);
        assert_eq!(timer_step(Arith::Msvc, ft, 0.65, true).to_bits(), 0x3a7f_9724);
        assert_eq!(fixed_slow_amt(0.0), 0.01);
        assert_eq!(fixed_slow_amt(-1.0), 0.01);
        assert_eq!(fixed_slow_amt(0.35), 0.35);
    }

    #[test]
    fn spread_penalty_growth() {
        for arith in [Arith::Gcc, Arith::Msvc] {
            assert_eq!(shot_interval(arith, 10.0, 10.0), None);
            assert_eq!(shot_interval(arith, 10.5, 10.0), Some(0.5));
            assert_eq!(grow_spread_penalty(arith, 0.0123, 1.0, 0.1, 0.0175, 2.0).to_bits(), 0x3d33_67a1);
            // Clamped to 0.1; from 1 s of game time on, x is 0.5.
            assert_eq!(grow_spread_penalty(arith, 0.09, 1.0, 0.1, 0.05, 2.0), 0.1);
            assert_eq!(grow_spread_penalty(arith, 0.0, 1.0, 3.0, 0.01, 2.0), 0.01);
            // `slowmul * elapsed` just below 1 (0.99999998): both builds compare it unrounded,
            // so the growth is (almost) nothing, where a `float` product (1.0) would add 0.01.
            assert_eq!(grow_spread_penalty(arith, 0.02, 0.65, 1.538_461_6, 0.01, 2.0), 0.02);
        }
    }

    #[test]
    fn penalty_decay_and_melee_bar() {
        for arith in [Arith::Gcc, Arith::Msvc] {
            assert_eq!(decay_spread_penalty(arith, 0.0, 0.01), 0.0);
            assert_eq!(decay_spread_penalty(arith, 0.0005, 0.01), 0.0);
            assert_eq!(decay_spread_penalty(arith, 0.05, 0.01), (0.05f64 as f32 as f64 - 0.01f32 as f64 * 0.1f32 as f64) as f32);
            assert_eq!(regen_melee_bar(arith, 100.0, 0.01), 100.0);
            assert_eq!(regen_melee_bar(arith, 99.95, 0.01), 100.0);
            assert_eq!(regen_melee_bar(arith, -5.0, 0.5), -4.5);
            assert_eq!(regen_melee_bar(arith, 50.0, 0.1), 51.5);
        }
    }

    #[test]
    fn melee_and_bullets() {
        for arith in [Arith::Gcc, Arith::Msvc] {
            assert_eq!(melee_damage(arith, 20.0, 100.0), 20.0);
            assert_eq!(melee_damage(arith, 20.0, -5.0), 10.0);
            // The multiplier stays unrounded until the product is stored (19.56034...).
            assert_eq!(melee_damage(arith, 20.0, 95.6).to_bits(), 0x419c_7ae1);
        }
        assert_eq!(bullet_velocity_mult(1.0, false), 1.0);
        assert!((bullet_velocity_mult(0.35, false) - 0.35).abs() < 1e-6);
        assert!(bullet_velocity_mult(0.35, true) < 0.05);
        assert_eq!(range_index(800.0, 80.0), 10);
    }
}
