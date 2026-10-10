//! Quake/Half-Life mathlib helpers, in `f32` and in the same operation order as the C code.

pub type Vec3 = [f32; 3];

pub const ZERO: Vec3 = [0.0; 3];

/// `M_PI * 2 / 360` as the C code writes it (`0.017453292`).
const DEG2RAD: f32 = 0.017_453_292;

#[inline]
pub fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[inline]
pub fn length(a: Vec3) -> f32 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

#[inline]
pub fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

#[inline]
pub fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

#[inline]
pub fn scale(a: Vec3, s: f32) -> Vec3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

/// `VectorMA(a, s, b, out)`: `a + s*b`.
#[inline]
pub fn ma(a: Vec3, s: f32, b: Vec3) -> Vec3 {
    [a[0] + s * b[0], a[1] + s * b[1], a[2] + s * b[2]]
}

#[inline]
pub fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// `VectorNormalize`: normalises in place, returns the old length.
#[inline]
pub fn normalize(v: &mut Vec3) -> f32 {
    let len = length(*v);
    if len != 0.0 {
        let il = 1.0 / len;
        v[0] *= il;
        v[1] *= il;
        v[2] *= il;
    }
    len
}

/// Quake `AngleVectors` (pitch, yaw, roll in degrees) -> (forward, right, up).
#[allow(clippy::neg_multiply)] // written as in mathlib.c
pub fn angle_vectors(angles: Vec3) -> (Vec3, Vec3, Vec3) {
    let (sy, cy) = ((angles[1] * DEG2RAD).sin(), (angles[1] * DEG2RAD).cos());
    let (sp, cp) = ((angles[0] * DEG2RAD).sin(), (angles[0] * DEG2RAD).cos());
    let (sr, cr) = ((angles[2] * DEG2RAD).sin(), (angles[2] * DEG2RAD).cos());
    let forward = [cp * cy, cp * sy, -sp];
    let right = [-1.0 * sr * sp * cy + -1.0 * cr * -sy, -1.0 * sr * sp * sy + -1.0 * cr * cy, -1.0 * sr * cp];
    let up = [cr * sp * cy + -sr * -sy, cr * sp * sy + -sr * cy, cr * cp];
    (forward, right, up)
}

/// Rounds like x87 `fistp` in the default rounding mode (to nearest, ties to even).
#[inline]
pub fn round_int(x: f32) -> i32 {
    x.round_ties_even() as i32
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
