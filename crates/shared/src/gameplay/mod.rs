//! The gameplay hooks both sides' shared code calls: [`PmRules`] (player movement) and
//! [`GunRules`] (the gun), each in its own module. The default method bodies are TS 3.0's
//! behaviour, so [`Core`], the core plugin, implements them as they are. A plugin overrides what
//! its ruleset changes. See `docs/implementation/gameplay-plugins.md`.
//!
//! The traits live here rather than in `ts-gameplay` because their 3.0 bodies and their
//! arguments are this crate's (`pm`, `weapons`), and this crate's movement and gun code calls
//! them: `ts-gameplay` sits below `shared`, so it can't name either.

pub mod gun;
pub mod pm;

pub use gun::{GunRules, MeleeRules, Predicted, SpreadDecay};
pub use pm::PmRules;
pub use ts_gameplay::{DEFAULT_ID, PHYSINFO_KEY, PHYSINFO_KEY_C};

/// TS 3.0's rules: the core plugin, registered as [`DEFAULT_ID`] and the fallback for any id
/// that isn't registered. Every hook keeps its default (3.0) body. The server and client crates
/// implement their own hook traits for it too.
pub struct Core;

/// The core plugin object.
pub static CORE: Core = Core;

impl PmRules for Core {}
impl GunRules for Core {}
impl MeleeRules for Core {}
