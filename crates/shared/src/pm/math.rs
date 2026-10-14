//! Quake/Half-Life mathlib helpers as each TS build compiles them ([`super::fpu`]): every
//! expression on the x87 stack ([`Fpu::N`]), rounded to `float` where the result is stored.

pub use super::fpu::{Fpu, Gcc, Msvc, Num};

pub type Vec3 = [f32; 3];

pub const ZERO: Vec3 = [0.0; 3];

/// `M_PI * 2 / 360`, a `double` constant in `AngleVectors` (server 0xfb430).
const DEG2RAD: f64 = 0.017_453_292_519_943_295;

/// `fcom` then `test $0x40` (C3): equal, or unordered, since a NaN sets C3 as well.
#[inline]
pub fn eq87<T: PartialOrd>(a: T, b: T) -> bool {
    a.partial_cmp(&b).is_none_or(|o| o.is_eq())
}

/// `fcom` then `test $0x1` (C0): `a < b`, or unordered, so a NaN takes this branch too.
#[allow(clippy::neg_cmp_op_on_partial_ord)] // the NaN case is the point
#[inline]
pub fn below87<T: PartialOrd>(a: T, b: T) -> bool {
    !(a >= b)
}

/// `fcom` then `test $0x41` (C0 or C3): `a <= b`, or unordered.
#[allow(clippy::neg_cmp_op_on_partial_ord)] // the NaN case is the point
#[inline]
pub fn not_above87<T: PartialOrd>(a: T, b: T) -> bool {
    !(a > b)
}

#[inline]
pub fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

#[inline]
pub fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// `VectorScale` (server 0xfbd68, client 0x100583e0): one product per component, the same in
/// both builds.
#[inline]
pub fn scale(a: Vec3, s: f32) -> Vec3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

/// The mathlib as one build computes it; every [`Fpu`] gets these.
pub trait FpuMath: Fpu {
    /// The order most inline `DotProduct`s are summed in: GCC from x, MSVC usually from z.
    const ZYX: [usize; 3] = if Self::MSVC { [2, 1, 0] } else { [0, 1, 2] };

    /// `DotProduct` (a macro), on the stack and not rounded, summed `a[i]*b[i] + a[j]*b[j] +
    /// a[k]*b[k]` (usually [`FpuMath::ZYX`]).
    #[inline]
    fn dot_in(a: Vec3, b: Vec3, [i, j, k]: [usize; 3]) -> Self::N {
        Self::x(a[i]) * Self::x(b[i]) + Self::x(a[j]) * Self::x(b[j]) + Self::x(a[k]) * Self::x(b[k])
    }

    /// `Length` (server 0xfbcbc, client 0x10058390): `sqrt(0 + a0² + a1² + a2²)`. GCC stores the
    /// root to a `float` before returning it; MSVC returns it unrounded in st0.
    #[inline]
    fn length(a: Vec3) -> Self::N {
        let r = (Self::ZERO + Self::x(a[0]) * Self::x(a[0]) + Self::x(a[1]) * Self::x(a[1]) + Self::x(a[2]) * Self::x(a[2])).sqrt();
        if Self::MSVC { r } else { Self::x(r.f()) }
    }

    /// The squares of `a` summed on the stack in the order `[i, j, k]` (`a[i]² + a[j]² + a[k]²`),
    /// not rounded: the inline `Vector::Length` the game code uses. Each site's compiler picks its
    /// own order ([`ORDER_XYZ`] is the most common), so ports name it.
    #[inline]
    fn sq_in(a: Vec3, [i, j, k]: [usize; 3]) -> Self::N {
        Self::x(a[i]) * Self::x(a[i]) + Self::x(a[j]) * Self::x(a[j]) + Self::x(a[k]) * Self::x(a[k])
    }

    /// The inline `Vector::Length()` as the game code mostly compiles it, the squares summed in
    /// `order`: MSVC keeps the root on the FPU (`fsqrt` with no store, mp.dll e.g. 0x100510c1), GCC
    /// stores it as a `float` first (`fsqrt; fstps; flds`, `ts_i386.so` e.g. 0x7cbeb).
    #[inline]
    fn vec_length(a: Vec3, order: [usize; 3]) -> Self::N {
        let r = Self::sq_in(a, order).sqrt();
        if Self::MSVC { r } else { Self::x(r.f()) }
    }

    /// The inline `Vector::Normalize()`: [`FpuMath::vec_length`], `(0, 0, 1)` when it is 0 or NaN
    /// (both builds test `fcom 0; test ah,0x40`), else each component times the unrounded `1 /
    /// len`, rounded to a `float` (mp.dll 0x100510f9..0x1005113f, out of line at 0x100148f0;
    /// `ts_i386.so` 0x7cc2d..0x7cc88).
    #[inline]
    fn vec_normalize(a: Vec3, order: [usize; 3]) -> Vec3 {
        Self::vec_normalize_n(a, order).map(Num::f)
    }

    /// [`FpuMath::vec_normalize`] before the products are stored, for a site that uses one
    /// unrounded (mp.dll's `Killed` push keeps x on the FPU, 0x1005e3ca).
    #[inline]
    fn vec_normalize_n(a: Vec3, order: [usize; 3]) -> [Self::N; 3] {
        let len = Self::vec_length(a, order);
        if eq87(len, Self::ZERO) {
            return [Self::ZERO, Self::ZERO, Self::x(1.0)];
        }
        let inv = Self::x(1.0) / len;
        a.map(|c| inv * Self::x(c))
    }

    /// `VectorMA(a, s, b, out)` (server 0xfbbc4, client 0x10058410): `s*b + a`, rounded once.
    #[inline]
    fn ma(a: Vec3, s: f32, b: Vec3) -> Vec3 {
        Self::ma_x(a, Self::x(s), b)
    }

    /// `a + s*b` for an unrounded `s`, rounded once per component.
    #[inline]
    fn ma_x(a: Vec3, s: Self::N, b: Vec3) -> Vec3 {
        [(s * Self::x(b[0]) + Self::x(a[0])).f(), (s * Self::x(b[1]) + Self::x(a[1])).f(), (s * Self::x(b[2]) + Self::x(a[2])).f()]
    }

    /// `CrossProduct` (server 0xfbc80, client 0x10029e00).
    #[inline]
    fn cross(a: Vec3, b: Vec3) -> Vec3 {
        let x = Self::x;
        [
            (x(a[1]) * x(b[2]) - x(a[2]) * x(b[1])).f(),
            (x(a[2]) * x(b[0]) - x(a[0]) * x(b[2])).f(),
            (x(a[0]) * x(b[1]) - x(a[1]) * x(b[0])).f(),
        ]
    }

    /// `VectorNormalize` (server 0xfbcf8, client 0x10029e40): normalises in place with the
    /// unrounded reciprocal and returns the old length, a `float` in GCC and unrounded in MSVC.
    #[inline]
    fn normalize(v: &mut Vec3) -> Self::N {
        let x = Self::x;
        let r = (x(v[0]) * x(v[0]) + x(v[1]) * x(v[1]) + x(v[2]) * x(v[2])).sqrt();
        let len = if Self::MSVC { r } else { x(r.f()) };
        // GCC tests `len != 0` (a NaN normalises); MSVC skips on C3, so a NaN doesn't.
        let zero = if Self::MSVC { eq87(len, Self::ZERO) } else { len == Self::ZERO };
        if !zero {
            let il = x(1.0) / len;
            for c in v.iter_mut() {
                *c = (il * x(*c)).f();
            }
        }
        len
    }

    /// `AngleVectors` (server 0xfb430, client 0x10029bb0) -> (forward, right, up). GCC stores
    /// each angle in radians and each sine and cosine to a `float`. MSVC feeds the unrounded
    /// radians to `fsin`/`fcos`, keeps `cos(yaw)` on the stack for the whole function and
    /// factors the products differently.
    fn angle_vectors(angles: Vec3) -> (Vec3, Vec3, Vec3) {
        let (x, xd) = (Self::x, Self::xd);
        if Self::MSVC {
            let rad = |a: f32| x(a) * xd(DEG2RAD);
            let ry = rad(angles[1]);
            let (sy, cy) = (ry.sin().f(), ry.cos());
            let rp = rad(angles[0]);
            let (sp, cp) = (rp.sin().f(), rp.cos().f());
            let rr = rad(angles[2]);
            let (sr, cr) = (rr.sin().f(), rr.cos().f());
            let forward = [(x(cp) * cy).f(), (x(cp) * x(sy)).f(), -sp];
            let a = x(sr) * x(sp);
            let right = [(x(cr) * x(sy) - a * cy).f(), ((a * x(sy) + x(cr) * cy) * x(-1.0)).f(), (x(sr) * x(cp) * x(-1.0)).f()];
            let c = x(cr) * x(sp);
            let up = [(c * cy + x(sr) * x(sy)).f(), (c * x(sy) - x(sr) * cy).f(), (x(cr) * x(cp)).f()];
            return (forward, right, up);
        }
        let rad = |a: f32| (xd(DEG2RAD) * x(a)).f();
        let sc = |a: f32| {
            let r = x(rad(a));
            (r.sin().f(), r.cos().f())
        };
        let (sy, cy) = sc(angles[1]);
        let (sp, cp) = sc(angles[0]);
        let (sr, cr) = sc(angles[2]);
        let forward = [cp * cy, cp * sy, -sp];
        let m1 = x(-1.0);
        let right = [
            (m1 * x(sr) * x(sp) * x(cy) + m1 * x(cr) * -x(sy)).f(),
            (m1 * x(sr) * x(sp) * x(sy) + m1 * x(cr) * x(cy)).f(),
            (m1 * x(sr) * x(cp)).f(),
        ];
        let up = [(x(cr) * x(sp) * x(cy) + -x(sr) * -x(sy)).f(), (x(cr) * x(sp) * x(sy) + -x(sr) * x(cy)).f(), cr * cp];
        (forward, right, up)
    }
}

impl<F: Fpu> FpuMath for F {}

/// [`FpuMath::sq_in`] orders: `x² + y² + z²`.
pub const ORDER_XYZ: [usize; 3] = [0, 1, 2];
/// `z² + y² + x²`.
pub const ORDER_ZYX: [usize; 3] = [2, 1, 0];

/// `fld dword t; fadd qword d; fstp dword` in mp.dll's 53-bit arithmetic: a `float` plus a `double`
/// constant, rounded once to a `float`. TS's think timers (`gpGlobals->time + 0.1`) are written so:
/// the `double` 0.1 or 0.3 isn't rounded to a `float` before the add.
#[inline]
pub fn plus_double(t: f32, d: f64) -> f32 {
    (t as f64 + d) as f32
}

/// Dispatches a function generic over [`Fpu`] on an [`Arith`](super::fpu::Arith):
/// `by_arith!(arith, go(a, b))` calls `go::<Gcc>(a, b)` or `go::<Msvc>(a, b)`.
#[macro_export]
macro_rules! by_arith {
    ($arith:expr, $f:ident($($a:expr),* $(,)?)) => {
        match $arith {
            $crate::pm::Arith::Gcc => $f::<$crate::pm::Gcc>($($a),*),
            $crate::pm::Arith::Msvc => $f::<$crate::pm::Msvc>($($a),*),
        }
    };
}

/// `Length` (server 0xfbcbc), which `ts_i386.so` also exports: [`FpuMath::length`] in GCC's
/// arithmetic, rounded to a `float`.
pub fn length(a: Vec3) -> f32 {
    Gcc::length(a).f()
}

/// `anglemod` (server 0xfb3d8), exported by `ts_i386.so` and called by nothing in TS or the
/// engines: `(360.0/65536) * ((int)(a*(65536/360.0)) & 65535)`. Both constants are `double`s
/// (0x100bd8, 0x100be0), the product is truncated by a dword `fistp` (0x80000000 when out of
/// range or NaN, which masks to 0) and the result is stored to a `float`.
pub fn anglemod(a: f32) -> f32 {
    let i = (Gcc::xd(65536.0 / 360.0) * Gcc::x(a)).trunc_i32() & 0xffff;
    (Gcc::xd(360.0 / 65536.0) * Gcc::xi(i)).f()
}

/// `Q_log2` (server 0xfbd8c), exported by `ts_i386.so` and called by nothing: the number of
/// arithmetic right shifts before `val` reaches 0. In TS a negative `val` never reaches 0 (it
/// stops at -1), so the loop never ends and hangs the caller. Port guard: the shifts are logical
/// (`val` read as unsigned), which gives TS's answer for every non-negative `val` and 31, the
/// shift count of a 32-bit value, for a negative one.
pub fn q_log2(val: i32) -> i32 {
    let mut answer: i32 = 0;
    let mut v = (val as u32) >> 1;
    while v != 0 {
        answer += 1;
        v >>= 1;
    }
    answer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn angle_vectors_basis() {
        for (f, r, u) in [Gcc::angle_vectors([0.0, 90.0, 0.0]), Msvc::angle_vectors([0.0, 90.0, 0.0])] {
            assert!((f[1] - 1.0).abs() < 1e-6 && f[0].abs() < 1e-6);
            assert!((r[0] - 1.0).abs() < 1e-6, "{r:?}");
            assert!((u[2] - 1.0).abs() < 1e-6);
            assert!(Gcc::dot_in(f, r, [0, 1, 2]).f().abs() < 1e-6);
        }
    }

    #[test]
    fn anglemod_quantises_to_65536_steps() {
        assert_eq!(anglemod(0.0), 0.0);
        assert_eq!(anglemod(90.0), 90.0);
        assert_eq!(anglemod(360.0), 0.0);
        assert_eq!(anglemod(450.0), 90.0);
        // (int) truncates toward zero, then the mask wraps negatives into [0, 360).
        assert_eq!(anglemod(-90.0), 270.0);
        // One step is 360/65536 (0.00549...): less than a step truncates to 0.
        assert_eq!(anglemod(0.005), 0.0);
        assert_eq!(anglemod(0.006), (360.0f64 / 65536.0) as f32);
        // 10 * 182.0444... = 1820.44 -> 1820 steps.
        assert_eq!(anglemod(10.0), (1820.0f64 * 360.0 / 65536.0) as f32);
        // fistp's 0x80000000 for an out-of-range or NaN product masks to 0.
        assert_eq!(anglemod(1e30), 0.0);
        assert_eq!(anglemod(f32::NAN), 0.0);
    }

    #[test]
    fn q_log2_counts_shifts() {
        assert_eq!(q_log2(0), 0);
        assert_eq!(q_log2(1), 0);
        assert_eq!(q_log2(2), 1);
        assert_eq!(q_log2(3), 1);
        assert_eq!(q_log2(256), 8);
        assert_eq!(q_log2(511), 8);
        assert_eq!(q_log2(i32::MAX), 30);
        // TS never returns for these; the port ends (port guard).
        assert_eq!(q_log2(-1), 31);
        assert_eq!(q_log2(i32::MIN), 31);
    }

    /// The inline `Vector::Normalize` (issue #150): mp.dll's unrounded length and `1/len` against
    /// the all-`float` arithmetic the port used before, which `ts_i386.so` happens to match here.
    #[test]
    fn vec_normalize_per_build() {
        let v = [0x431b_d958, 0x4158_e140, 0x43b1_25fa].map(f32::from_bits); // (155.849, 13.555, 354.297)
        assert_eq!(Msvc::vec_normalize(v, ORDER_XYZ).map(f32::to_bits), [0x3ece_07a3, 0x3d0f_5b17, 0x3f6a_2fed]);
        assert_eq!(Gcc::vec_normalize(v, ORDER_XYZ).map(f32::to_bits), [0x3ece_07a2, 0x3d0f_5b17, 0x3f6a_2fed]);
        let v = [0xc373_43ba, 0xc34e_d102, 0xc23b_51e8].map(f32::from_bits);
        assert_eq!(Msvc::vec_normalize(v, ORDER_XYZ).map(f32::to_bits), [0xbf40_f9bc, 0xbf24_0fea, 0xbe14_989a]);
        assert_eq!(Gcc::vec_normalize(v, ORDER_XYZ).map(f32::to_bits), [0xbf40_f9bc, 0xbf24_0feb, 0xbe14_989a]);
        // Zero and NaN lengths give straight up in both builds (`test ah,0x40`).
        for v in [ZERO, [f32::NAN, 1.0, 0.0]] {
            assert_eq!(Msvc::vec_normalize(v, ORDER_ZYX), [0.0, 0.0, 1.0]);
            assert_eq!(Gcc::vec_normalize(v, ORDER_XYZ), [0.0, 0.0, 1.0]);
        }
        // MSVC returns the root unrounded, GCC the stored float.
        let v = [1.0, 1.0, 1.0];
        assert_eq!(Msvc::vec_length(v, ORDER_XYZ), 3f64.sqrt());
        assert_eq!(Gcc::vec_length(v, ORDER_XYZ).d(), 3f64.sqrt() as f32 as f64);
    }

    /// `time + 0.1`/`0.3` with the `double` (issue #150): in [1, 2) `t + 0.3f` is an exact tie, so a
    /// time with its last bit set rounds the other way.
    #[test]
    fn plus_double_rounds_once() {
        let t = f32::from_bits(0x3f80_0001);
        assert_eq!(plus_double(t, 0.3).to_bits(), 0x3fa6_6667);
        assert_eq!((t + 0.3).to_bits(), 0x3fa6_6668, "the float 0.3");
    }

    #[test]
    fn length_rounds_to_float() {
        assert_eq!(length([3.0, 4.0, 0.0]), 5.0);
        assert_eq!(length(ZERO), 0.0);
        assert_eq!(length([1.0, 1.0, 1.0]), 3f64.sqrt() as f32);
    }
}
