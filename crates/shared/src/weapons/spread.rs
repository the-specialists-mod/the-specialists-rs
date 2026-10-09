//! Seeded pellet directions (the TS `ShootBulletsPlayer` / client `EV_TS_FireBullets` spread),
//! recoil (`VKickback`) and the small vector helpers the weapon code needs. Pure functions, so the
//! server and the client prediction get the same bits.

use super::random::shared_random_float;
use super::{kind, tables};

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

/// `CTSGun::VKickback` 0x80414: (pitch, yaw) view kick in degrees for weapon kind `k`. The
/// pitch is subtracted from `v_angle.x`, the yaw added to `v_angle.y`.
pub fn view_kick(view_kick: f32, k: u8, silenced: bool, double_fire_rate: bool) -> (f32, f32) {
    let mut pitch = view_kick * 0.85;
    let mut yaw = pitch
        * match k {
            kind::PISTOL => 0.35,
            kind::AKIMBO | kind::AKIMBO_ONLY => 0.15,
            kind::SHOTGUN => -0.25,
            kind::RIFLE | kind::SNIPER => -0.3,
            kind::SMG => -0.35,
            _ => 0.0,
        };
    if silenced {
        pitch *= 0.775;
        yaw *= 0.775;
    }
    if double_fire_rate {
        pitch *= 0.666_666_7;
        yaw *= 0.666_666_7;
    }
    (pitch, yaw)
}

/// `CBasePlayer::GetMeleeMultiplier` 0x965d0: 0.5 with an empty bar, 1.0 with a full one.
pub fn melee_multiplier(bar: f32) -> f32 {
    ((bar.max(0.0) as f64 * 0.5 + 50.0) * 0.01) as f32
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

    #[test]
    fn kick() {
        let (p, y) = view_kick(10.0, kind::PISTOL, false, false);
        assert!((p - 8.5).abs() < 1e-5 && (y - 2.975).abs() < 1e-5);
        let (p, _) = view_kick(10.0, kind::RIFLE, true, true);
        assert!((p - 8.5 * 0.775 * 0.6666667).abs() < 1e-4);
    }

    #[test]
    fn melee_and_bullets() {
        assert_eq!(melee_multiplier(100.0), 1.0);
        assert_eq!(melee_multiplier(-5.0), 0.5);
        assert_eq!(bullet_velocity_mult(1.0, false), 1.0);
        assert!((bullet_velocity_mult(0.35, false) - 0.35).abs() < 1e-6);
        assert!(bullet_velocity_mult(0.35, true) < 0.05);
        assert_eq!(range_index(800.0, 80.0), 10);
    }
}
