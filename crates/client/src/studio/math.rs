//! Studio maths (the TS client's copies of the SDK `studio_util.cpp` helpers).

pub use goldsrc::ext::studio::Mat34;

pub type Quat = [f32; 4];
pub type Vec3 = [f32; 3];

/// `AngleQuaternion` (`0x1002fbd0`): Euler angles in radians (x roll, y pitch, z yaw).
pub fn angle_quaternion(a: Vec3) -> Quat {
    let (sy, cy) = (a[2] * 0.5).sin_cos();
    let (sp, cp) = (a[1] * 0.5).sin_cos();
    let (sr, cr) = (a[0] * 0.5).sin_cos();
    [sr * cp * cy - cr * sp * sy, cr * sp * cy + sr * cp * sy, cr * cp * sy - sr * sp * cy, cr * cp * cy + sr * sp * sy]
}

/// `QuaternionSlerp` (`0x1002fc70`).
pub fn quaternion_slerp(p: Quat, mut q: Quat, t: f32) -> Quat {
    // decide if one of the quaternions is backwards
    let (mut a, mut b) = (0.0, 0.0);
    for i in 0..4 {
        a += (p[i] - q[i]) * (p[i] - q[i]);
        b += (p[i] + q[i]) * (p[i] + q[i]);
    }
    if a > b {
        q = q.map(|x| -x);
    }
    let cosom = p[0] * q[0] + p[1] * q[1] + p[2] * q[2] + p[3] * q[3];
    let mut qt = [0.0; 4];
    if 1.0 + cosom > 0.000001 {
        let (sclp, sclq) = if 1.0 - cosom > 0.000001 {
            let omega = cosom.acos();
            let sinom = omega.sin();
            (((1.0 - t) * omega).sin() / sinom, (t * omega).sin() / sinom)
        } else {
            (1.0 - t, t)
        };
        for i in 0..4 {
            qt[i] = sclp * p[i] + sclq * q[i];
        }
    } else {
        qt = [-q[1], q[0], -q[3], q[2]];
        let sclp = ((1.0 - t) * std::f32::consts::FRAC_PI_2).sin();
        let sclq = (t * std::f32::consts::FRAC_PI_2).sin();
        for i in 0..3 {
            qt[i] = sclp * p[i] + sclq * qt[i];
        }
    }
    qt
}

/// `QuaternionMatrix` (`0x1002fdf0`): the rotation part of a bone matrix.
pub fn quaternion_matrix(q: Quat) -> Mat34 {
    let mut m = [[0.0; 4]; 3];
    m[0][0] = 1.0 - 2.0 * q[1] * q[1] - 2.0 * q[2] * q[2];
    m[1][0] = 2.0 * q[0] * q[1] + 2.0 * q[3] * q[2];
    m[2][0] = 2.0 * q[0] * q[2] - 2.0 * q[3] * q[1];
    m[0][1] = 2.0 * q[0] * q[1] - 2.0 * q[3] * q[2];
    m[1][1] = 1.0 - 2.0 * q[0] * q[0] - 2.0 * q[2] * q[2];
    m[2][1] = 2.0 * q[1] * q[2] + 2.0 * q[3] * q[0];
    m[0][2] = 2.0 * q[0] * q[2] + 2.0 * q[3] * q[1];
    m[1][2] = 2.0 * q[1] * q[2] - 2.0 * q[3] * q[0];
    m[2][2] = 1.0 - 2.0 * q[0] * q[0] - 2.0 * q[1] * q[1];
    m
}

/// `MatrixAngles` (`0x1002fef0`, TS): back to the radians `angle_quaternion` takes.
pub fn matrix_angles(m: &Mat34) -> Vec3 {
    let y = (-m[2][0]).clamp(-1.0, 1.0).asin();
    if (y.abs() - std::f32::consts::FRAC_PI_2).abs() < 0.01 {
        // gimbal lock: roll folded into yaw
        [0.0, y, (-m[0][1]).atan2(m[1][1])]
    } else {
        [m[2][1].atan2(m[2][2]), y, m[1][0].atan2(m[0][0])]
    }
}

/// TS `QuaternionAngles` (`0x1002ffe0`): `QuaternionMatrix` then `MatrixAngles`.
pub fn quaternion_angles(q: Quat) -> Vec3 {
    matrix_angles(&quaternion_matrix(q))
}

/// `AngleMatrix` (`0x1002f920`): Quake angles in degrees (pitch, yaw, roll).
pub fn angle_matrix(a: Vec3) -> Mat34 {
    let (sy, cy) = a[1].to_radians().sin_cos();
    let (sp, cp) = a[0].to_radians().sin_cos();
    let (sr, cr) = a[2].to_radians().sin_cos();
    [
        [cp * cy, sr * sp * cy + cr * -sy, cr * sp * cy + -sr * -sy, 0.0],
        [cp * sy, sr * sp * sy + cr * cy, cr * sp * sy + -sr * cy, 0.0],
        [-sp, sr * cp, cr * cp, 0.0],
    ]
}

/// `ConcatTransforms` (`0x1002fa90`): `a * b` for 3x4 affine matrices.
pub fn concat(a: &Mat34, b: &Mat34) -> Mat34 {
    let mut o = [[0.0; 4]; 3];
    for i in 0..3 {
        for j in 0..4 {
            o[i][j] = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
        o[i][3] += a[i][3];
    }
    o
}

/// `VectorTransform` (`0x1002fa30`).
pub fn transform(v: Vec3, m: &Mat34) -> Vec3 {
    std::array::from_fn(|i| v[0] * m[i][0] + v[1] * m[i][1] + v[2] * m[i][2] + m[i][3])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: &[f32], b: &[f32]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4)
    }

    #[test]
    fn quaternion_round_trip() {
        for a in [[0.1, 0.2, 0.3], [0.0, 0.0, 1.5], [-0.7, 0.4, -2.0]] {
            let back = quaternion_angles(angle_quaternion(a));
            assert!(close(&back, &a), "{a:?} -> {back:?}");
        }
    }

    #[test]
    fn slerp_ends_and_midpoint() {
        let p = angle_quaternion([0.0, 0.0, 0.0]);
        let q = angle_quaternion([0.0, 0.0, 1.0]);
        assert!(close(&quaternion_slerp(p, q, 0.0), &p));
        assert!(close(&quaternion_slerp(p, q, 1.0), &q));
        assert!(close(&quaternion_slerp(p, q, 0.5), &angle_quaternion([0.0, 0.0, 0.5])));
        // q and -q are the same rotation: the slerp takes the short way.
        let neg = q.map(|x| -x);
        assert!(close(&quaternion_slerp(p, neg, 0.5), &angle_quaternion([0.0, 0.0, 0.5])));
    }

    #[test]
    fn matrices_compose() {
        // AngleMatrix(yaw 90) turns +x into +y; the bone path (quaternion z = yaw) agrees.
        let m = angle_matrix([0.0, 90.0, 0.0]);
        assert!(close(&transform([1.0, 0.0, 0.0], &m), &[0.0, 1.0, 0.0]));
        let q = quaternion_matrix(angle_quaternion([0.0, 0.0, std::f32::consts::FRAC_PI_2]));
        assert!(close(&transform([1.0, 0.0, 0.0], &q), &[0.0, 1.0, 0.0]));
        let mut t = angle_matrix([0.0, 0.0, 0.0]);
        t[0][3] = 5.0;
        let c = concat(&t, &m);
        assert!(close(&transform([1.0, 0.0, 0.0], &c), &[5.0, 1.0, 0.0]));
    }
}
