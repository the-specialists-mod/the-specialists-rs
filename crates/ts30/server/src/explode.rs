//! TS 3.0's explosions and thrown blades.

use shared::weapons::spread::Vec3;
use ts_server::weapons::ThrownBlade;

/// The end of `TSExplosion` (0x7d964/0x7d9ab): `DoSpectacularNess()`, then
/// `CreateSlowMotionPoint(origin + 1z, 0.25, 1.0, 0)`.
pub fn explosion_slow_point(org: Vec3) {
    if ts_server::rules::scoring::do_spectacularness() {
        ts_server::entities::ts::create_slow_motion_point(org, 0.25, 1.0, false);
    }
}

/// The speed of a thrown blade: `KnifeFire` throws knives at 1900 u/s (objdump 0x7465b /
/// 0x746a7), `KatanaThrow` the katana at 1000 u/s (0x73dc1, the float at 0xf7f64).
fn throw_speed(btype: i32) -> f32 {
    if btype == 8 { 1000.0 } else { 1900.0 }
}

/// The thrown blades: [`throw_speed`]; gravity 0.85 katana, 0.75 combat knife, 0.3 seal
/// knife (`CTSKnife::Spawn` 0x7db54); bits `0x08001000` (thrown: the ×2 headshot).
pub fn thrown_blade(btype: i32) -> ThrownBlade {
    let gravity = match btype {
        8 => 0.85,
        6 => 0.75,
        _ => 0.3,
    };
    ThrownBlade { speed: throw_speed(btype), gravity, bits: 0x0800_1000 }
}

/// The thrown-blade damage (`CTSKnife::BounceTouch` 0x7decc): 150 katana, 100 combat
/// knife, 50 seal knife, however far it flew.
pub fn thrown_blade_damage(btype: i32) -> f32 {
    match btype {
        8 => 150.0,
        6 => 100.0,
        _ => 50.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn katana_is_thrown_slower_than_knives() {
        assert_eq!(throw_speed(6), 1900.0);
        assert_eq!(throw_speed(7), 1900.0);
        assert_eq!(throw_speed(8), 1000.0);
    }
}
