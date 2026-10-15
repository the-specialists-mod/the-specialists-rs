//! TS 3.0's movement rules and its move speed by free slots: the `PmRules` answers.
//!
//! Addresses without a prefix are `ts_i386.so`'s (decomp `docs/server/player-movement.md` §1).

use crate::Ts30;
use shared::gameplay::PmRules;
use shared::gameplay::pm::{Arith, GroundAccel, Jump, TextureLimits, Vec3};
use shared::pm::ts::JUMP_VELOCITY;
use shared::pm::{Fpu, FpuMath, Gcc, Msvc, Num};

/// Wall jump: pushed away from the wall at 210 u/s, up at the normal jump speed.
pub const WALLJUMP_SPEED: f32 = 210.0;
/// Wall climb: `sqrt(2 * 800 * 70)`.
pub const WALLCLIMB_VELOCITY: f32 = 334.664;

impl PmRules for Ts30 {
    /// `PM_Accelerate` (0xfdd08, client 0x1002b830): `PM_GetAccelBySpeed(maxspeed)`.
    fn ground_accel(&self) -> GroundAccel {
        GroundAccel::BySpeed
    }

    /// `TSHandle` (0xf96a0): the `double` 0.5 (mp.dll 0x10089180).
    fn recover_input_scale(&self) -> f64 {
        0.5
    }

    /// `TSHandle` (0xf96a0): while `PRONE`, TS 3.0 sets the forward move to 0.
    fn prone_forward_scale(&self) -> Option<f64> {
        None
    }

    /// `TSWallJump` (0xf9444).
    fn wall_jump_speed(&self) -> f32 {
        WALLJUMP_SPEED
    }

    /// `TSWallJump` (0xf9444): the normal jump's.
    fn wall_jump_up_velocity(&self) -> f32 {
        JUMP_VELOCITY
    }

    /// `TSWallJumpClimb` (0xf94e0).
    fn wall_climb_velocity(&self) -> f32 {
        WALLCLIMB_VELOCITY
    }

    /// `TSDive` (0xf9534): the wall dive goes along `forward × 3`.
    fn wall_dive(&self) -> bool {
        true
    }

    /// `FixVelocity(velocity, limit)` (0xf8fc4): TS 3.0's is empty, and nothing calls it.
    fn fix_velocity(&self, arith: Arith, velocity: &mut Vec3, limit: f32) {
        let _ = (arith, velocity, limit);
    }

    /// `PM_Jump` (0xf9944): TS 3.0 changes nothing.
    fn jump_push(&self, arith: Arith, jump: Jump, buttons: u16, maxspeed: f32, velocity: &mut Vec3) {
        let _ = (arith, jump, buttons, maxspeed, velocity);
    }

    /// `PM_PlayerMove` (0x1008d8): frozen below 0.1 (or NaN).
    fn scaled_frametime(&self, frametime: f32, fuser1: f32) -> f32 {
        if shared::pm::below87(fuser1, shared::ts::TIME_SCALE_FREEZE) { 0.0 } else { frametime * fuser1 }
    }

    /// `PM_DropPunchAngle` (0x100760, client 0x1002e1c0): [`drop_punch_angle`].
    fn drop_punch_angle(&self, arith: Arith, punch: Vec3, dead: bool, fuser1: f32, frametime: f32) -> Vec3 {
        match arith {
            Arith::Gcc => drop_punch_angle::<Gcc>(punch, dead, fuser1, frametime),
            Arith::Msvc => drop_punch_angle::<Msvc>(punch, dead, fuser1, frametime),
        }
    }

    /// `PM_UpdateStepSound` (0xfe2d4): `interval / max(fuser1, 0.1)`, truncated
    /// ([`step_interval`]).
    fn step_interval(&self, arith: Arith, interval: i32, fuser1: f32) -> i32 {
        match arith {
            Arith::Gcc => step_interval::<Gcc>(interval, fuser1),
            Arith::Msvc => step_interval::<Msvc>(interval, fuser1),
        }
    }

    /// `PM_InitTextureTypes` / `PM_FindTextureType` / `PM_CatagorizeTextureType` (`mp.dll`
    /// 0x10036630 / 0x100368d0): 1024 entries, 15 characters.
    fn texture_limits(&self) -> TextureLimits {
        TextureLimits::LARGE
    }

    /// `PM_SpectatorMove` (`mp.dll` 0x10038d20) in eye: TS 3.0 copies the target's entity pitch
    /// unchanged.
    fn in_eye_pitch(&self, target_pitch: f32) -> f32 {
        target_pitch
    }

    /// `PM_CalcRoll` (`mp.dll` 0x1003a341): `side * rollangle / rollspeed`.
    fn roll_divides_first(&self) -> bool {
        false
    }

    /// `PM_Friction`'s edge-trace start: `vel[i] / speed * 16 + origin[i]`.
    fn friction_edge_by_reciprocal(&self) -> bool {
        false
    }

    /// TS 3.0's `mp.dll` and `client.dll` test C3/C0, so a NaN takes the "equal" and "below"
    /// branches.
    fn ieee_compares(&self) -> bool {
        false
    }

    /// `GetSpeedBySlots` (0x76bd4): [`speed_by_slots`].
    fn speed_by_slots(&self, free: i32, the_one: bool) -> f32 {
        speed_by_slots(free, the_one)
    }

    /// `CBasePlayer::SetPlayerSpeed` (0x93068) and `TSInit` (0x92a4c): 330.
    fn max_player_speed(&self) -> f32 {
        330.0
    }
}

/// `PM_DropPunchAngle` (0x100760, client 0x1002e1c0). TS scales by `fuser1` on top of the
/// already-scaled frametime, and zeroes the punch while dead. MSVC multiplies by `fuser1` first
/// and clamps the unrounded length (a NaN or a -0 becomes +0).
pub fn drop_punch_angle<F: FpuMath>(punch: Vec3, dead: bool, fuser1: f32, frametime: f32) -> Vec3 {
    let mut punch = punch;
    let len = if !dead {
        let len = F::normalize(&mut punch);
        if F::MSVC {
            let d = len - (len * F::xd(0.5) + F::xd(10.0)) * F::x(fuser1) * F::x(frametime);
            if d > F::xd(0.0) { d.f() } else { 0.0 }
        } else {
            let len = (len - (len * F::xd(0.5) + F::xd(10.0)) * F::x(frametime) * F::x(fuser1)).f();
            if 0.0 > len { 0.0 } else { len }
        }
    } else {
        0.0
    };
    [punch[0] * len, punch[1] * len, punch[2] * len]
}

/// The slow-motion footstep interval of `PM_UpdateStepSound` (0xfe2d4): the interval times
/// `1 / max(fuser1, 0.1)` (a NaN time scale counts as 0.1), the reciprocal unrounded, truncated.
pub fn step_interval<F: Fpu>(interval: i32, fuser1: f32) -> i32 {
    let ts = if fuser1 >= 0.1 { fuser1 } else { 0.1 };
    (F::x(1.0) / F::x(ts) * F::xi(interval)).trunc_i32()
}

/// `GetSpeedBySlots` (0x76bd4, mp.dll 0x1004cbe0): the move speed for `free` inventory slots,
/// `(int)(free * 0.012345679f * 120 + 210)` truncated (GCC's `fistp` under a truncating control
/// word, MSVC's `__ftol`), or 330 for The One and an empty inventory (`free > 80`). Every step is
/// exact in `f64` (a 24-bit float times small integers), so the truncation sees TS's value.
/// `CBasePlayer::SetPlayerSpeed` clamps the result to `0..=330`.
pub fn speed_by_slots(free: i32, the_one: bool) -> f32 {
    if the_one || free > shared::ts::SLOTS_TOTAL - 1 {
        return 330.0;
    }
    (f64::from(free) * f64::from(0.012_345_679_f32) * 120.0 + 210.0) as i32 as f32
}

#[cfg(test)]
mod tests;
