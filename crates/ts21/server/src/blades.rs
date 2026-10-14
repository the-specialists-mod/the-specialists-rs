//! TS 2.1's thrown knives (issue #100 §3.2, checklist item 5): `CTSKnife` (`CreateKnife`
//! 0x10048010, `Spawn` 0x10048100, `BounceTouch` 0x10048380, `WaitThink` 0x10048c70). 2.1 has no
//! katana throw, so only the combat (6) and seal (7) knives fly.

use shared::weapons::spread::Vec3;
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

/// `CTSKnife::BounceTouch`'s distance from the throw point (+0x84) to the blade (0x100483fd..
/// 0x10048431): the differences unrounded, `(x² + z²) + y²`, the root stored as a `float`.
pub fn flown(from: Vec3, at: Vec3) -> f32 {
    let d = [0, 1, 2].map(|i| f64::from(from[i]) - f64::from(at[i]));
    ((d[0] * d[0] + d[2] * d[2]) + d[1] * d[1]).sqrt() as f32
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

    /// The distance from unrounded differences, `(x² + z²) + y²`: the old `float` length here was
    /// 0x458b3859.
    #[test]
    fn flown_distance_as_mp_dll() {
        let f = |b: [u32; 3]| b.map(f32::from_bits);
        let (from, at) = (f([0xc4df_91c5, 0x4510_2b2b, 0x4486_dbcd]), f([0x4502_f6b0, 0x4458_a787, 0xc40c_2fb8]));
        assert_eq!(flown(from, at).to_bits(), 0x458b_385a);
        assert_eq!(flown([0.0; 3], [3.0, 4.0, 12.0]), 13.0);
    }

    #[test]
    fn knives_fly_slower_and_hit_no_heads() {
        assert_eq!(thrown_blade(6), ThrownBlade { speed: 1300.0, gravity: 0.9, bits: 0x1080 });
        assert_eq!(thrown_blade(7), ThrownBlade { speed: 1300.0, gravity: 0.25, bits: 0x1080 });
    }
}
