//! TS 3.0's recoil penalty and view kick: the arithmetic behind `ScaleSpread`, `VKickback` and
//! `PreThink`'s penalty decay, in each TS build's rounding ([`Arith`]).

use shared::by_arith;
use shared::pm::{Arith, Fpu, Num};
use shared::weapons::kind;

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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn penalty_decay() {
        for arith in [Arith::Gcc, Arith::Msvc] {
            assert_eq!(decay_spread_penalty(arith, 0.0, 0.01), 0.0);
            assert_eq!(decay_spread_penalty(arith, 0.0005, 0.01), 0.0);
            assert_eq!(decay_spread_penalty(arith, 0.05, 0.01), (0.05f64 as f32 as f64 - 0.01f32 as f64 * 0.1f32 as f64) as f32);
        }
    }
}
