//! [`PmRules`]: the player-movement hooks (`crate::pm`), and the move speed the server derives
//! from the inventory.
//!
//! Both sides run the movement and must agree, so the active rules come from the player's
//! physinfo key ([`PHYSINFO_KEY`](super::PHYSINFO_KEY)) on both: the server's `pfnPM_Move` and the
//! client's `HUD_PlayerMove` resolve them once per call and pass them to
//! [`player_move`](crate::pm::player_move), which keeps them for the whole command.

use crate::pm::ts::{JUMP_VELOCITY, WALLCLIMB_VELOCITY, WALLJUMP_SPEED};
pub use crate::pm::{Arith, Vec3};
use crate::pm::{Gcc, Msvc};

/// How `PM_Accelerate` scales the ground acceleration of a player who is neither prone nor ducked
/// ([`PmRules::ground_accel`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GroundAccel {
    /// `PM_GetAccelBySpeed(maxspeed)` (0xfdcc4): 0.6 at 210 u/s up to 1.1 at 330 u/s, TS 3.0's.
    BySpeed,
    /// A constant factor, a `double`. MSVC multiplies the acceleration while it is still on the
    /// FPU stack (no call in between spills it); GCC multiplies the stored acceleration and
    /// stores the result, as it does with [`GroundAccel::BySpeed`].
    Constant(f64),
}

/// Which jump [`PmRules::jump_push`] is called for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Jump {
    /// The normal jump, before `PM_Jump` sets its upward speed.
    Normal,
    /// The super jump (physinfo `mj`), before `PM_Jump` sets its upward speed.
    High,
}

/// Movement rules. Every default body is TS 3.0's (`Core`); the movement code calls these where a
/// ruleset can differ. The movement code is generic over the build's arithmetic
/// ([`Arith`](crate::pm::Arith)) and a trait object can't be, so a hook either takes and returns
/// plain values (one whose result feeds the build's arithmetic returns the value the 3.0 code used
/// as a constant), or takes the `Arith` of the command and computes in it with
/// [`crate::pm::Fpu`]/[`crate::pm::FpuMath`].
pub trait PmRules: Sync {
    /// `PM_Accelerate` (0xfdd08, client 0x1002b830): the ground acceleration factor while not
    /// prone and not `FL_DUCKING`.
    fn ground_accel(&self) -> GroundAccel {
        GroundAccel::BySpeed
    }

    /// `TSHandle` (0xf96a0): the scale on the usercmd's forward, side and up moves while the
    /// stunt bit `RECOVER` is set (getting up after a dive or roll). A `double` (mp.dll
    /// 0x10089180), multiplying each move on the FPU stack.
    fn recover_input_scale(&self) -> f64 {
        0.5
    }

    /// `TSHandle` (0xf96a0): while `PRONE`, TS 3.0 sets the forward move to 0. `Some(k)`
    /// multiplies it by the `double` k instead (which keeps a -0 or a NaN).
    fn prone_forward_scale(&self) -> Option<f64> {
        None
    }

    /// `TSWallJump` (0xf9444): the speed the wall jump pushes the player away from the wall.
    fn wall_jump_speed(&self) -> f32 {
        WALLJUMP_SPEED
    }

    /// `TSWallJump` (0xf9444): the wall jump's upward speed, the normal jump's.
    fn wall_jump_up_velocity(&self) -> f32 {
        JUMP_VELOCITY
    }

    /// `TSWallJumpClimb` (0xf94e0): the upward speed of a wall climb.
    fn wall_climb_velocity(&self) -> f32 {
        WALLCLIMB_VELOCITY
    }

    /// `TSDive` (0xf9534): whether a dive with the stunt bit `WALLDIVE` goes along `forward × 3`
    /// (then clearing the bit). Without it the dive follows the velocity and the bit stays.
    fn wall_dive(&self) -> bool {
        true
    }

    /// `FixVelocity(velocity, limit)` (0xf8fc4): TS 3.0's is empty, and nothing calls it. The
    /// movement calls this after the dive, wall jump and wall climb impulses with
    /// `limit = 2 × maxspeed`.
    fn fix_velocity(&self, arith: Arith, velocity: &mut Vec3, limit: f32) {
        let _ = (arith, velocity, limit);
    }

    /// `PM_Jump` (0xf9944): called on a jump off the ground just before the upward speed is set,
    /// with the command's buttons and `maxspeed`. TS 3.0 changes nothing.
    fn jump_push(&self, arith: Arith, jump: Jump, buttons: u16, maxspeed: f32, velocity: &mut Vec3) {
        let _ = (arith, jump, buttons, maxspeed, velocity);
    }

    /// `PM_PlayerMove` (0x1008d8): the frame time scaled by the time scale (`fuser1`, 1.0 =
    /// normal), frozen below 0.1 (or NaN).
    fn scaled_frametime(&self, frametime: f32, fuser1: f32) -> f32 {
        if crate::pm::below87(fuser1, crate::ts::TIME_SCALE_FREEZE) { 0.0 } else { frametime * fuser1 }
    }

    /// `PM_DropPunchAngle` (0x100760, client 0x1002e1c0): the decayed punch angle. `frametime`
    /// is `pmove->frametime` when `PM_CheckParamters` runs.
    fn drop_punch_angle(&self, arith: Arith, punch: Vec3, dead: bool, fuser1: f32, frametime: f32) -> Vec3 {
        match arith {
            Arith::Gcc => crate::pm::drop_punch_angle::<Gcc>(punch, dead, fuser1, frametime),
            Arith::Msvc => crate::pm::drop_punch_angle::<Msvc>(punch, dead, fuser1, frametime),
        }
    }

    /// `PM_UpdateStepSound` (0xfe2d4): the footstep interval in ms in slow motion, from the
    /// normal one: `interval / max(fuser1, 0.1)`, truncated.
    fn step_interval(&self, arith: Arith, interval: i32, fuser1: f32) -> i32 {
        match arith {
            Arith::Gcc => crate::pm::step_interval::<Gcc>(interval, fuser1),
            Arith::Msvc => crate::pm::step_interval::<Msvc>(interval, fuser1),
        }
    }

    /// `GetSpeedBySlots` (0x76bd4): the move speed for `free` inventory slots. Server only: the
    /// client gets it as `maxspeed`.
    fn speed_by_slots(&self, free: i32, the_one: bool) -> f32 {
        crate::ts::speed_by_slots(free, the_one)
    }

    /// `CBasePlayer::SetPlayerSpeed` (0x93068): the top of its clamp, and the speed `TSInit`
    /// (0x92a4c) starts a player with. Server only.
    fn max_player_speed(&self) -> f32 {
        330.0
    }
}
