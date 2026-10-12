//! [`PmRules`]: the player-movement hooks (`crate::pm`).
//!
//! Both sides run the movement and must agree, so the active rules come from the player's
//! physinfo key ([`PHYSINFO_KEY`](super::PHYSINFO_KEY)) on both: the server's `pfnPM_Move` and the
//! client's `HUD_PlayerMove` resolve them once per call and pass them to
//! [`player_move`](crate::pm::player_move), which keeps them for the whole command.

use crate::pm::ts::WALLCLIMB_VELOCITY;

/// Movement rules. Every default body is TS 3.0's (`Core`); the movement code calls these where a
/// ruleset can differ. Hooks take and return plain values: the movement code is generic over
/// [`Arith`](crate::pm::Arith) and a trait object can't be, so a hook whose result feeds the
/// build's arithmetic returns the value the 3.0 code used as a constant.
pub trait PmRules: Sync {
    /// `TSHandle` (0xf96a0): the scale on the usercmd's forward, side and up moves while the
    /// stunt bit `RECOVER` is set (getting up after a dive or roll).
    fn recover_input_scale(&self) -> f32 {
        0.5
    }

    /// `TSWallJumpClimb` (0xf94e0): the upward speed of a wall climb.
    fn wall_climb_velocity(&self) -> f32 {
        WALLCLIMB_VELOCITY
    }
}
