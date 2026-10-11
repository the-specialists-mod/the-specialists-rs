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
/// arithmetic right shifts before `val` reaches 0. As in TS, a negative `val` never reaches 0, so
/// the loop doesn't end.
pub fn q_log2(val: i32) -> i32 {
    let mut answer: i32 = 0;
    let mut v = val >> 1;
    while v != 0 {
        answer = answer.wrapping_add(1);
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
    }

    #[test]
    fn length_rounds_to_float() {
        assert_eq!(length([3.0, 4.0, 0.0]), 5.0);
        assert_eq!(length(ZERO), 0.0);
        assert_eq!(length([1.0, 1.0, 1.0]), 3f64.sqrt() as f32);
    }
}
