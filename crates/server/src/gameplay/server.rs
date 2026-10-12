//! [`ServerRules`]: the server-only hooks of a gameplay ruleset (rules, scoring, damage,
//! entities...). Bots aren't part of a ruleset: they are left to a plugin axis of their own.
//!
//! The server's own code resolves them from the active plugin ([`active`](super::active)), which
//! follows `ts_gameplay` once a frame; a switch resets the game first ([`super::reset`]).

use crate::rules::scoring::{self, Frag};

/// Server rules. Every default body is TS 3.0's (`shared::gameplay::Core`); the server code calls
/// these where a ruleset can differ. A hook needing server state takes this crate's types, so
/// its 3.0 body can be the code it replaces.
pub trait ServerRules: Sync {
    /// The plugin has become active on a brand-new game: at every map start (after the rules
    /// are installed, before the map's entities spawn) and after a switch to it has reset the
    /// game ([`super::reset`]). The place to set up the plugin's own state.
    fn on_activate(&self) {}

    /// `TSGetPointsForFrag` (0x89140): the points a killing player scores for `frag`, which also
    /// reports whether the kill was a slide or a stunt.
    fn points_for_frag(&self, frag: &Frag<'_>, sliding: &mut bool, diving: &mut bool) -> i32 {
        scoring::points_for_frag(frag, sliding, diving)
    }
}
