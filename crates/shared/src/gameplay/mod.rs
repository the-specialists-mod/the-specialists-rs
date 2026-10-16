//! The gameplay hooks both sides' shared code calls: [`PmRules`] (player movement) and
//! [`GunRules`] (the gun), each in its own module. Each ruleset's plugin implements them (TS 3.0,
//! TS 2.1). See `docs/implementation/gameplay-plugins.md`.
//!
//! The traits live here rather than in `ts-gameplay` because their arguments are this crate's
//! (`pm`, `weapons`), and this crate's movement and gun code calls them: `ts-gameplay` sits below
//! `shared`, so it can't name either.

pub mod gun;
pub mod pm;

pub use gun::{GunRules, MeleeRules, Predicted, SpreadDecay, WeaponPreThink};
pub use pm::PmRules;
pub use ts_gameplay::{PHYSINFO_KEY, PHYSINFO_KEY_C};
