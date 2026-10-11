//! Quake/Half-Life mathlib helpers as TS's server builds them (`ts_i386.so`): every expression
//! in x87 extended precision ([`Ext`]), rounded to `float` where the result is stored.

pub use super::ext::{Ext, x, xd};

pub type Vec3 = [f32; 3];

pub const ZERO: Vec3 = [0.0; 3];

/// `M_PI * 2 / 360`, a `double` constant in `AngleVectors` (0xfb430).
const DEG2RAD: f64 = 0.017_453_292_519_943_295;

/// `fcom` then `test $0x40` (C3): equal, or unordered, since a NaN sets C3 as well.
#[inline]
pub fn eq87(a: f32, b: f32) -> bool {
    a == b || a.is_nan() || b.is_nan()
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

/// `DotProduct` (a macro): extended precision, not rounded.
#[inline]
pub fn dotx(a: Vec3, b: Vec3) -> Ext {
    x(a[0]) * x(b[0]) + x(a[1]) * x(b[1]) + x(a[2]) * x(b[2])
}

/// `DotProduct` stored to a `float`.
#[inline]
pub fn dot(a: Vec3, b: Vec3) -> f32 {
    dotx(a, b).f()
}

/// `Length` (0xfbcbc): the root is stored to a `float` before it's returned.
#[inline]
pub fn length(a: Vec3) -> f32 {
    (Ext::ZERO + x(a[0]) * x(a[0]) + x(a[1]) * x(a[1]) + x(a[2]) * x(a[2])).sqrt().f()
}

#[inline]
pub fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

#[inline]
pub fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// `VectorScale` (0xfbd68).
#[inline]
pub fn scale(a: Vec3, s: f32) -> Vec3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

/// `VectorMA(a, s, b, out)` (0xfbbc4): `s*b + a`, rounded once.
#[inline]
pub fn ma(a: Vec3, s: f32, b: Vec3) -> Vec3 {
    ma_x(a, x(s), b)
}

/// `a + s*b` for an unrounded `s`, rounded once per component (TS inlines this after
/// `PM_AirAccelerate`, `PM_WaterMove` and `PM_SpectatorMove` compute `accelspeed`).
#[inline]
pub fn ma_x(a: Vec3, s: Ext, b: Vec3) -> Vec3 {
    [(s * x(b[0]) + x(a[0])).f(), (s * x(b[1]) + x(a[1])).f(), (s * x(b[2]) + x(a[2])).f()]
}

/// `CrossProduct` (0xfbc80).
#[inline]
pub fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [(x(a[1]) * x(b[2]) - x(a[2]) * x(b[1])).f(), (x(a[2]) * x(b[0]) - x(a[0]) * x(b[2])).f(), (x(a[0]) * x(b[1]) - x(a[1]) * x(b[0])).f()]
}

/// `VectorNormalize` (0xfbcf8): normalises in place, returns the old length (a `float`). The
/// reciprocal stays in extended precision.
#[inline]
pub fn normalize(v: &mut Vec3) -> f32 {
    let len = (x(v[0]) * x(v[0]) + x(v[1]) * x(v[1]) + x(v[2]) * x(v[2])).sqrt().f();
    if len != 0.0 {
        let il = x(1.0) / x(len);
        for c in v.iter_mut() {
            *c = (il * x(*c)).f();
        }
    }
    len
}

/// `AngleVectors` (0xfb430) -> (forward, right, up). Each angle and each sine/cosine is
/// stored to a `float`; the products are rounded once per component.
pub fn angle_vectors(angles: Vec3) -> (Vec3, Vec3, Vec3) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn angle_vectors_basis() {
        let (f, r, u) = angle_vectors([0.0, 90.0, 0.0]);
        assert!((f[1] - 1.0).abs() < 1e-6 && f[0].abs() < 1e-6);
        assert!((r[0] - 1.0).abs() < 1e-6, "{r:?}");
        assert!((u[2] - 1.0).abs() < 1e-6);
        assert!(dot(f, r).abs() < 1e-6);
    }
}
