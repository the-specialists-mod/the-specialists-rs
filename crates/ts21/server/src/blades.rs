//! TS 2.1's thrown knives (issue #100 §3.2, checklist item 5): `CTSKnife` (`CreateKnife`
//! 0x10048010, `Spawn` 0x10048100, `BounceTouch` 0x10048380, `WaitThink` 0x10048c70). 2.1 has no
//! katana throw, so only the combat (6) and seal (7) knives fly.

use ts_server::weapons::ThrownBlade;

/// The combat knife's blade type.
const COMBAT_KNIFE: i32 = 6;

/// `KnifeFire` (`FUN_1004c6c0`) throws at 1300 u/s; `CTSKnife::Spawn` sets gravity 0.9 for the
/// combat knife (0x3f666666) and 0.25 for the others (0x3e800000); the hit on a player and the
/// flying impact both use `DMG_NEVERGIB | DMG_CLUB` (0x1080, pushed at 0x100484f1 and
/// 0x10048e61), so a thrown knife is never a headshot.
pub fn thrown_blade(btype: i32) -> ThrownBlade {
    let gravity = if btype == COMBAT_KNIFE { 0.9 } else { 0.25 };
    ThrownBlade { speed: 1300.0, gravity, bits: 0x1080 }
}

/// `CTSKnife::BounceTouch` (0x10048380): the combat knife deals 80 up to 512 units from where
/// it was thrown, then `80 − (d − 512)·(1/1536)·80` (the float 0.0006510417), at least 50; the seal
/// knife 50. The products run on the x87 stack and the result is stored as a float.
pub fn thrown_blade_damage(btype: i32, flown: f32) -> f32 {
    if btype != COMBAT_KNIFE {
        return 50.0;
    }
    if flown < 512.0 {
        return 80.0;
    }
    let d = (80.0 - (flown as f64 - 512.0) * 0.000_651_041_686_069_220_3 * 80.0) as f32;
    if d <= 50.0 { 50.0 } else { d }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combat_knife_falls_off_from_512_units() {
        assert_eq!(thrown_blade_damage(6, 0.0), 80.0);
        assert_eq!(thrown_blade_damage(6, 511.9), 80.0);
        assert_eq!(thrown_blade_damage(6, 512.0), 80.0);
        assert!((thrown_blade_damage(6, 1024.0) - 53.333_33).abs() < 1e-3);
        // 50 is reached at 1088 units, not 1024.
        assert!((thrown_blade_damage(6, 1080.0) - 50.416_67).abs() < 1e-3);
        assert_eq!(thrown_blade_damage(6, 1088.0), 50.0);
        assert_eq!(thrown_blade_damage(6, 4000.0), 50.0);
        assert_eq!(thrown_blade_damage(7, 0.0), 50.0, "seal knife");
    }

    #[test]
    fn knives_fly_slower_and_hit_no_heads() {
        assert_eq!(thrown_blade(6), ThrownBlade { speed: 1300.0, gravity: 0.9, bits: 0x1080 });
        assert_eq!(thrown_blade(7), ThrownBlade { speed: 1300.0, gravity: 0.25, bits: 0x1080 });
    }
}
