//! TS 2.1's explosions (issue #100 §3.2, checklist item 5): the grenade's blast and the shrapnel
//! every `TSExplosion` (0x10047aa0) sprays. TS 3.0 has `CTSShrapnel` but never creates it.

use goldsrc::edict::EdictPtr;
use goldsrc::server::engine;
use goldsrc::sys::{EF_NODRAW, edict_t};
use shared::weapons::spread::Vec3;

/// `CM61Grenade::WaitThink` (0x100477e0, pushes at 0x10047848..): `TSExplosion(origin, pev,
/// owner, 100, DMG_BLAST | DMG_NEVERGIB, 128, 1024)`.
pub const GRENADE_EXPLOSION: (f32, f32, f32) = (100.0, 128.0, 1024.0);

/// The shrapnel's speed (0x10080fa4).
const SHRAPNEL_SPEED: f32 = 2000.0;

/// A piece's direction from its three rolls `RANDOM_FLOAT(-1, 1)`, `(-1, 1)`, `(-0.3, 1)`
/// (0x10047ec6..0x10047f6e): normalised on the x87 stack through a float `1/len`, `(0, 0, 1)` for
/// a zero vector, then scaled by 2000.
pub fn shrapnel_velocity(r: Vec3) -> Vec3 {
    let (x, y, z) = (r[0] as f64, r[1] as f64, r[2] as f64);
    let len = (z * z + y * y + x * x).sqrt();
    let dir = if len == 0.0 {
        [0.0, 0.0, 1.0]
    } else {
        let inv = (1.0 / len) as f32 as f64;
        [inv * x, inv * y, inv * z]
    };
    dir.map(|d| (d * SHRAPNEL_SPEED as f64) as f32)
}

/// The end of `TSExplosion` (0x10047eb9..): `RANDOM_LONG(30, 45)` pieces of shrapnel from the
/// blast point (origin + 1z), each `CTSShrapnel::Create(owner, org, dir·2000)` with the
/// attacker (else the inflictor) as owner; then, if the blast was spectacular,
/// `CreateSlowMotionPoint(org, 0.15, 1.5, 0)` (3.0: 0.25 for 1 s). No world gun is pushed
/// (`explosion_pushes_world_guns`).
pub fn aftermath(org: Vec3, attacker: *mut edict_t) {
    let eng = engine();
    let n = eng.random_long(30, 45);
    for _ in 0..n {
        let r = [eng.random_float(-1.0, 1.0), eng.random_float(-1.0, 1.0), eng.random_float(-0.3, 1.0)];
        spawn_shrapnel(attacker, org, shrapnel_velocity(r));
    }
    if ts_server::rules::scoring::do_spectacularness() {
        ts_server::entities::ts::create_slow_motion_point(org, 0.15, 1.5, false);
    }
}

/// `CTSShrapnel::Create` (0x10046f10) with 2.1's `Spawn` (0x10046ff0): the same piece as 3.0's
/// but visible (no `EF_NODRAW`: `models/shrapnel.mdl` with a random body) and with
/// `iuser4 = 1` (3.0: 4).
fn spawn_shrapnel(owner: *mut edict_t, org: Vec3, velocity: Vec3) {
    let e = ts_server::entities::ts::shrapnel::create(owner, org, velocity);
    if e.is_null() {
        return;
    }
    let v = e.v();
    v.effects &= !(EF_NODRAW as i32);
    v.iuser4 = 1;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shrapnel_flies_at_2000() {
        assert_eq!(shrapnel_velocity([0.0, 0.0, 0.0]), [0.0, 0.0, 2000.0]);
        assert_eq!(shrapnel_velocity([1.0, 0.0, 0.0]), [2000.0, 0.0, 0.0]);
        let v = shrapnel_velocity([0.6, -0.8, 0.0]);
        assert!((v[0] - 1200.0).abs() < 1e-3 && (v[1] + 1600.0).abs() < 1e-3 && v[2] == 0.0, "{v:?}");
    }
}
