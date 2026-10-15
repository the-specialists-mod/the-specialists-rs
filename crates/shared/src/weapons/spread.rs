//! Seeded pellet directions (the TS `ShootBulletsPlayer` / client `EV_TS_FireBullets` spread),
//! recoil (`VKickback`), the recoil penalty, timer steps and the small vector helpers the weapon
//! code needs. Pure functions, so the server and the client prediction get the same bits; the
//! ones whose rounding differs between TS's builds take an [`Arith`].

use super::random::shared_random_float;
use super::tables;
use crate::pm::{Arith, Ext, Fpu, Gcc, Msvc, Num};

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

/// A pellet's offset in units of the spread, `(right, up)`. Every TS build stores `x` as a
/// `float`; `y` is what the build multiplies by the spread: mp.dll (0x10047d6f `fst`) and
/// `client.dll` (0x1004596e) keep it unrounded at 53 bits, `ts_i386.so` (0x6fb5d) stores it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PelletOffset {
    pub x: f32,
    pub y: f64,
}

impl From<(f32, f32)> for PelletOffset {
    /// An offset whose `y` is already a `float` (2.1's triangle sums).
    fn from((x, y): (f32, f32)) -> Self {
        PelletOffset { x, y: y as f64 }
    }
}

/// TS 3.0's polar offset from its two draws, `t = SR(seed+i, -π, π)` (stored as a `float`) and
/// `r = SR(seed+i+1, 0, radial)`: `m = 1 - r·r·r` and `(cos t·m, sin t·m)` on the x87 (mp.dll
/// 0x10047d46..0x10047d6f, `client.dll` 0x10045941..0x1004596e). `r` is a 16-bit fraction, so
/// `m` is exact in `f64`; the sine and cosine are the platform's, where the x87's `fsin` and
/// `fcos` can differ in the last bit.
pub fn polar_offset(t: f32, r: f32) -> PelletOffset {
    let (t, r) = (t as f64, r as f64);
    let m = 1.0 - r * r * r;
    PelletOffset { x: (t.cos() * m) as f32, y: t.sin() * m }
}

/// Offsets of pellet `i` (1-based) in units of the spread ([`polar_offset`] of the server's
/// shared random draws; weapons.md §5.4). `radial` is the ammo's radial spread scale (1.0 for
/// all ammo).
pub fn pellet_offset(seed: u32, i: u32, radial: f32) -> PelletOffset {
    let pi = std::f32::consts::PI;
    let t = shared_random_float(seed.wrapping_add(i), -pi, pi);
    let r = shared_random_float(seed.wrapping_add(i).wrapping_add(1), 0.0, radial);
    polar_offset(t, r)
}

/// Which TS build's pellet direction to reproduce: the server's (`ShootBulletsPlayer`, per
/// `ts_pm_arith`) or the client fire event's (`EV_TS_FireBullets`, `client.dll`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PelletSide {
    Server(Arith),
    Client,
}

/// The direction of a pellet at offset `o`: `aim + right·x·spread[0] + up·y·spread[1]`, not
/// normalised, rounded as `side`'s TS build does (issue #161). Each build orders and stores the
/// products and sums differently:
///
/// * mp.dll (0x10047d6f..0x10047e71): `a = x·sx`, `b = y·sy` unrounded; `a·right[i]` and
///   `b·up[i]` stored (`Vector` constructor arguments); x is `(aim + a·right) + b·up` with the
///   inner sum unrounded, y and z add the stored `aim + a·right` (`Vector::operator+`).
/// * `ts_i386.so` (0x6fb63..0x6fc12): `x`, `y` stored; `a·right` and `b·up` stored for x and y,
///   the z products unrounded; `(aim + a·right) + b·up` in extended precision, one store each.
/// * `client.dll` (0x10045970..0x100459de): `x·right[i]·sx` and `y` times `up[i]` and `sy` (in
///   the order `up, sy` for x and z, `sy, up` for y), summed, plus `aim[i]`, one store each.
pub fn pellet_dir(side: PelletSide, aim: Vec3, right: Vec3, up: Vec3, spread: [f32; 2], o: PelletOffset) -> Vec3 {
    let (sx, sy) = (spread[0] as f64, spread[1] as f64);
    match side {
        PelletSide::Server(Arith::Msvc) => {
            let (a, b) = (o.x as f64 * sx, o.y * sy);
            let ar = |i: usize| (a * right[i] as f64) as f32 as f64;
            let bu = |i: usize| (b * up[i] as f64) as f32 as f64;
            let sum = |i: usize| aim[i] as f64 + ar(i);
            [((sum(0) + bu(0)) as f32), ((sum(1) as f32 as f64 + bu(1)) as f32), ((sum(2) as f32 as f64 + bu(2)) as f32)]
        }
        PelletSide::Server(Arith::Gcc) => {
            let x = Gcc::x;
            let (a, b) = (x(o.x) * x(spread[0]), x(o.y as f32) * x(spread[1]));
            let c = |i: usize| {
                let (ar, bu) = (a * x(right[i]), b * x(up[i]));
                // x and y store both products; z keeps them on the stack.
                let (ar, bu) = if i < 2 { (x(ar.f()), x(bu.f())) } else { (ar, bu) };
                (x(aim[i]) + ar + bu).f()
            };
            [c(0), c(1), c(2)]
        }
        PelletSide::Client => {
            let (xf, y) = (o.x as f64, o.y);
            let (r, u) = (right.map(f64::from), up.map(f64::from));
            [
                ((xf * r[0] * sx + y * u[0] * sy) + aim[0] as f64) as f32,
                ((y * sy * u[1] + xf * r[1] * sx) + aim[1] as f64) as f32,
                ((y * u[2] * sy + xf * r[2] * sx) + aim[2] as f64) as f32,
            ]
        }
    }
}

// ---- the TS builds' arithmetic -----------------------------------------------------------------
//
// The server (`ts_i386.so`: GCC, x87 extended precision; `mp.dll`: MSVC, 53-bit) and the client
// prediction (`client.dll`, MSVC) compile the weapon code separately, so a `float` expression can
// round differently. Where it matters for a value both sides compute, the function below takes
// the [`Arith`] of the build it stands in for, as player movement does (`shared::pm::fpu`): the
// server passes its `ts_pm_arith` selection (default `Msvc`, `mp.dll`), the client `Arith::CLIENT`.

use crate::by_arith;

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

/// The `double` `1/15` both builds scale the speed by in `CTSStunt::CloseAttack` (`ts_i386.so`
/// 0xf9b10, `mp.dll` 0x10094a90).
const ONE_FIFTEENTH: f64 = 0.066_666_666_666_666_67;

/// The jump kick's strength (`CTSStunt::CloseAttack` 0x9c630, 0x8c6e7..0x8c7e6; `mp.dll`
/// 0x1006a7e5..0x1006a8bb): `|velocity| / 15` clamped to 10..20, tested against the `double`s
/// 10 and 20 and stored as a `float`. GCC stores the length as a `float` before scaling it; MSVC
/// keeps it on the FPU. A NaN speed is 10 in both builds (the `< 10` test is `jb`/`test ah,1`).
/// The kick's damage is this, doubled with the Kung Fu power-up; the melee bar pays 1.5 times it.
pub fn jump_kick_strength(arith: Arith, velocity: Vec3) -> f32 {
    fn go<F: Fpu>(v: Vec3) -> f32 {
        let (x, y, z) = (F::x(v[0]), F::x(v[1]), F::x(v[2]));
        let len = (x * x + y * y + z * z).sqrt();
        let len = if F::MSVC { len } else { F::x(len.f()) };
        let t = len * F::xd(ONE_FIFTEENTH);
        if t > F::xd(20.0) {
            20.0
        } else if t >= F::xd(10.0) {
            t.f()
        } else {
            10.0
        }
    }
    by_arith!(arith, go(velocity))
}

/// The end of the jump kick's trace, `GetGunPosition() + v_forward * 48` (`CloseAttack`
/// 0x8c836..0x8c873; `mp.dll` 0x1006a913..0x1006a935): GCC adds the unrounded product and stores
/// once; MSVC's `Vector::operator*` stores the product as `float`s before `operator+`.
pub fn jump_kick_end(arith: Arith, src: Vec3, fwd: Vec3) -> Vec3 {
    fn go<F: Fpu>(src: Vec3, fwd: Vec3) -> Vec3 {
        let c = |i: usize| {
            let d = F::x(48.0) * F::x(fwd[i]);
            let d = if F::MSVC { F::x(d.f()) } else { d };
            (F::x(src[i]) + d).f()
        };
        [c(0), c(1), c(2)]
    }
    by_arith!(arith, go(src, fwd))
}

/// `GetBulletVelocityMult(slowmul, bullet_time)` (0x99160, mp.dll 0x10067080) for simulated
/// bullets: 1 from 0.99 up, else `slowmul / max((1 - slowmul²)·k, 1)` with `k` 10 in bullet
/// time, all on the FPU. The value is returned as the x87 holds it for the caller, which is
/// what the two `UpdateBullet`s differ in: mp.dll stores it as a `float` (0x100472e8), so
/// `Msvc` gives that `float`; `ts_i386.so` multiplies by it unrounded (0x6f060), so `Gcc` gives
/// the extended-precision quotient.
pub fn bullet_velocity_mult(arith: Arith, slowmul: f32, bullet_time: bool) -> Ext {
    fn go<F: Fpu>(s: f32, bt: bool) -> F::N {
        let x = F::x;
        if s >= 0.99 {
            return x(1.0);
        }
        let d = (x(1.0) - x(s) * x(s)) * x(if bt { 10.0 } else { 1.0 });
        // `1 > d` takes 1 (a NaN `d` stays).
        let d = if x(1.0) > d { x(1.0) } else { d };
        x(s) / d
    }
    match arith {
        Arith::Gcc => go::<Gcc>(slowmul, bullet_time),
        Arith::Msvc => Ext::from_f32(go::<Msvc>(slowmul, bullet_time).f()),
    }
}

/// Ammo constants shared by every type (weapons.md §3): trace length and radial spread scale.
pub const MAX_RANGE: f32 = 8192.0;
pub const RADIAL_SPREAD: f32 = 1.0;
pub const UNITS_PER_METER: f32 = 40.0;

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
                let o = pellet_offset(seed, i, 1.0);
                assert!(o.x as f64 * o.x as f64 + o.y * o.y <= 1.0 + 1e-6);
                assert_eq!(pellet_offset(seed, i, 1.0), o);
            }
        }
        for side in [PelletSide::Server(Arith::Gcc), PelletSide::Server(Arith::Msvc), PelletSide::Client] {
            let d = pellet_dir(side, [1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0], pellet_offset(5, 1, 1.0));
            assert_eq!(d, [1.0, 0.0, 0.0]);
        }
    }

    /// Each TS build rounds a pellet's direction its own way (issue #161); none of them is the
    /// all-`float` arithmetic the port used before (`[0x3f307a77, 0x3f39b0a2, 0xbc65f7ac]` here).
    #[test]
    fn pellet_dir_per_build() {
        let (aim, right, up) = angle_vectors([3.0, 47.0, 0.0]);
        assert_eq!(aim.map(f32::to_bits), [0x3f2e_5a31, 0x3f3a_f84f, 0xbd56_5e3b]);
        let o = pellet_offset(45, 1, 1.0);
        assert_eq!((o.x.to_bits(), o.y), (0x3e72_c94c, 0.958_810_467_690_113_6), "y kept at 53 bits");
        let dir = |side| pellet_dir(side, aim, right, up, [0.04, 0.04], o).map(f32::to_bits);
        assert_eq!(dir(PelletSide::Server(Arith::Gcc)), [0x3f30_7a77, 0x3f39_b0a1, 0xbc65_f7af]);
        assert_eq!(dir(PelletSide::Server(Arith::Msvc)), [0x3f30_7a77, 0x3f39_b0a2, 0xbc65_f7b0]);
        assert_eq!(dir(PelletSide::Client), [0x3f30_7a77, 0x3f39_b0a1, 0xbc65_f7ae]);
    }

    /// `GetBulletVelocityMult` keeps `s²`, `1 - s²` and `·10` on the FPU (issue #150): the port's
    /// old all-`float` result was 0x3d857831. mp.dll stores the quotient, the .so keeps it.
    #[test]
    fn bullet_velocity_mult_per_build() {
        let s = f32::from_bits(0x3efc_833a);
        let m = bullet_velocity_mult(Arith::Msvc, s, true);
        assert_eq!(m.f().to_bits(), 0x3d85_7832);
        assert_eq!(m, Ext::from_f32(m.f()), "a float");
        let g = bullet_velocity_mult(Arith::Gcc, s, true);
        assert_eq!(g.f().to_bits(), 0x3d85_7832);
        assert_ne!(g, m, "extended");
    }

    #[test]
    fn jump_kick_strength_is_speed_over_15_clamped_to_10_20() {
        // the `double` constant as both binaries store it
        assert_eq!(ONE_FIFTEENTH.to_bits(), 0x3fb1_1111_1111_1111);
        for arith in [Arith::Gcc, Arith::Msvc] {
            let s = |v: Vec3| jump_kick_strength(arith, v);
            assert_eq!(s([0.0; 3]), 10.0);
            assert_eq!(s([149.0, 0.0, 0.0]), 10.0);
            assert_eq!(s([150.0, 0.0, 0.0]), 10.0);
            assert_eq!(s([0.0, 180.0, -135.0]), 15.0);
            assert_eq!(s([300.0, 0.0, 0.0]), 20.0);
            assert_eq!(s([0.0, 0.0, -1000.0]), 20.0);
            assert_eq!(s([f32::NAN, 0.0, 0.0]), 10.0);
            assert_eq!(s([250.0, 0.0, 0.0]), (250.0f64 * ONE_FIFTEENTH) as f32);
        }
    }

    #[test]
    fn jump_kick_strength_gcc_rounds_the_length_first() {
        // MSVC scales the unrounded length, GCC the stored `float`: one ulp apart here
        let v = [150.0, 7.0, 0.0];
        let len = (22_549.0f64).sqrt();
        assert_eq!(jump_kick_strength(Arith::Msvc, v), (len * ONE_FIFTEENTH) as f32);
        assert_eq!(jump_kick_strength(Arith::Gcc, v), (len as f32 as f64 * ONE_FIFTEENTH) as f32);
        assert_eq!(jump_kick_strength(Arith::Gcc, v), 10.010_882);
        assert_eq!(jump_kick_strength(Arith::Msvc, v), 10.010_883);
    }

    #[test]
    fn jump_kick_end_is_48_units_ahead() {
        for arith in [Arith::Gcc, Arith::Msvc] {
            assert_eq!(jump_kick_end(arith, [10.0, 20.0, 64.0], [1.0, 0.0, 0.0]), [58.0, 20.0, 64.0]);
            assert_eq!(jump_kick_end(arith, [0.0, 0.0, 0.0], [0.0, 0.0, -1.0]), [0.0, 0.0, -48.0]);
        }
        // MSVC rounds `fwd * 48` before the add, GCC rounds once
        let (src, fwd) = ([1000.1, -3.3, 0.7], [0.123_456_79, 0.987_654_3, -0.1]);
        for i in 0..3 {
            let gcc = (src[i] as f64 + 48.0 * fwd[i] as f64) as f32;
            let msvc = (src[i] as f64 + (48.0 * fwd[i] as f64) as f32 as f64) as f32;
            assert_eq!(jump_kick_end(Arith::Gcc, src, fwd)[i], gcc);
            assert_eq!(jump_kick_end(Arith::Msvc, src, fwd)[i], msvc);
        }
    }

    // Reference bits below are exact models of the x87 at 64 bits (`ts_i386.so`) and 53 bits
    // (`mp.dll`, `client.dll`), each operation rounded once, in the machine code's order.

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
    fn melee_bar_regen() {
        for arith in [Arith::Gcc, Arith::Msvc] {
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
        for arith in [Arith::Gcc, Arith::Msvc] {
            assert_eq!(bullet_velocity_mult(arith, 1.0, false).f(), 1.0);
            assert!((bullet_velocity_mult(arith, 0.35, false).f() - 0.35).abs() < 1e-6);
            assert!(bullet_velocity_mult(arith, 0.35, true).f() < 0.05);
        }
    }
}
