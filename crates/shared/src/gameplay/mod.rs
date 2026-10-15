//! The gameplay hooks both sides' shared code calls: [`PmRules`] (player movement) and
//! [`GunRules`] (the gun), each in its own module. Each ruleset's plugin implements them (TS 3.0,
//! TS 2.1). The default method bodies are still TS 3.0's behaviour until they move to the 3.0
//! plugin (issue #200). See
//! `docs/implementation/gameplay-plugins.md`.
//!
//! The traits live here rather than in `ts-gameplay` because their 3.0 bodies and their
//! arguments are this crate's (`pm`, `weapons`), and this crate's movement and gun code calls
//! them: `ts-gameplay` sits below `shared`, so it can't name either.

pub mod gun;
pub mod pm;

pub use gun::{GunRules, MeleeRules, Predicted, SpreadDecay};
pub use pm::PmRules;
pub use ts_gameplay::{PHYSINFO_KEY, PHYSINFO_KEY_C};

/// Every hook's default body, for this crate's tests of the code behind them (they move to
/// the 3.0 plugin with the bodies, issue #200).
#[cfg(test)]
pub(crate) struct Defaults;

#[cfg(test)]
pub(crate) static DEFAULTS: Defaults = Defaults;

#[cfg(test)]
impl PmRules for Defaults {}
#[cfg(test)]
impl GunRules for Defaults {}
#[cfg(test)]
impl MeleeRules for Defaults {}
