//! [`GunRules`]: the weapon hooks (`crate::weapons::gun`).
//!
//! The server's gun and the client's weapon prediction run the same code and must agree, so each
//! resolves the rules once per command from the player's physinfo key
//! ([`PHYSINFO_KEY`](super::PHYSINFO_KEY)) and hands them to the gun through
//! [`GunHost::rules`](crate::weapons::GunHost::rules).

/// Gun rules. Every default body is TS 3.0's (`Core`); the gun code calls these where a ruleset
/// can differ.
pub trait GunRules: Sync {
    /// `CTSGun::StandardFire` (0x824a4): the delay before the next primary shot of a gun in semi-auto
    /// mode that can also fire full auto. `rate` is the weapon's own fire rate (`info.rate`),
    /// which full auto uses.
    fn semi_auto_delay(&self, rate: f32) -> f32 {
        let _ = rate;
        0.1
    }
}
