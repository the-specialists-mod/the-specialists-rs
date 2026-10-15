//! [`PmRules`]: the player-movement hooks (`crate::pm`), and the move speed the server derives
//! from the inventory.
//!
//! Both sides run the movement and must agree, so the active rules come from the player's
//! physinfo key ([`PHYSINFO_KEY`](super::PHYSINFO_KEY)) on both: the server's `pfnPM_Move` and the
//! client's `HUD_PlayerMove` resolve them once per call and pass them to
//! [`player_move`](crate::pm::player_move), which keeps them for the whole command.

pub use crate::pm::textures::TextureLimits;
pub use crate::pm::{Arith, Vec3};

/// How `PM_Accelerate` scales the ground acceleration of a player who is neither prone nor ducked
/// ([`PmRules::ground_accel`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GroundAccel {
    /// `PM_GetAccelBySpeed(maxspeed)` (0xfdcc4, [`accel_by_speed`](crate::pm::accel_by_speed)):
    /// 0.6 at 210 u/s up to 1.1 at 330 u/s.
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

/// Movement rules: the movement code calls these where a ruleset can differ. The movement code is
/// generic over the build's arithmetic ([`Arith`](crate::pm::Arith)) and a trait object can't be,
/// so a hook either takes and returns plain values (one whose result feeds the build's arithmetic
/// returns a value the movement uses as a constant), or takes the `Arith` of the command and
/// computes in it with [`crate::pm::Fpu`]/[`crate::pm::FpuMath`].
pub trait PmRules: Sync {
    /// `PM_Accelerate`: the ground acceleration factor while not prone and not `FL_DUCKING`.
    fn ground_accel(&self) -> GroundAccel;

    /// `TSHandle`: the scale on the usercmd's forward, side and up moves while the stunt bit
    /// `RECOVER` is set (getting up after a dive or roll), a `double` multiplying each move on the
    /// FPU stack.
    fn recover_input_scale(&self) -> f64;

    /// `TSHandle`: the forward move while `PRONE`. `None` sets it to 0; `Some(k)` multiplies it by
    /// the `double` k instead (which keeps a -0 or a NaN).
    fn prone_forward_scale(&self) -> Option<f64>;

    /// `TSWallJump`: the speed the wall jump pushes the player away from the wall.
    fn wall_jump_speed(&self) -> f32;

    /// `TSWallJump`: the wall jump's upward speed.
    fn wall_jump_up_velocity(&self) -> f32;

    /// `TSWallJumpClimb`: the upward speed of a wall climb.
    fn wall_climb_velocity(&self) -> f32;

    /// `TSDive`: whether a dive with the stunt bit `WALLDIVE` goes along `forward × 3` (then
    /// clearing the bit). Without it the dive follows the velocity and the bit stays.
    fn wall_dive(&self) -> bool;

    /// `FixVelocity(velocity, limit)`: the movement calls this after the dive, wall jump and wall
    /// climb impulses with `limit = 2 × maxspeed`.
    fn fix_velocity(&self, arith: Arith, velocity: &mut Vec3, limit: f32);

    /// `PM_Jump`: called on a jump off the ground just before the upward speed is set, with the
    /// command's buttons and `maxspeed`.
    fn jump_push(&self, arith: Arith, jump: Jump, buttons: u16, maxspeed: f32, velocity: &mut Vec3);

    /// `PM_PlayerMove`: the frame time scaled by the time scale (`fuser1`, 1.0 = normal).
    fn scaled_frametime(&self, frametime: f32, fuser1: f32) -> f32;

    /// `PM_DropPunchAngle`: the decayed punch angle. `frametime` is `pmove->frametime` when
    /// `PM_CheckParamters` runs.
    fn drop_punch_angle(&self, arith: Arith, punch: Vec3, dead: bool, fuser1: f32, frametime: f32) -> Vec3;

    /// `PM_UpdateStepSound`: the footstep interval in ms in slow motion, from the normal one.
    fn step_interval(&self, arith: Arith, interval: i32, fuser1: f32) -> i32;

    /// `PM_InitTextureTypes` / `PM_FindTextureType` / `PM_CatagorizeTextureType`: the footstep
    /// material table's size and name length.
    fn texture_limits(&self) -> TextureLimits;

    /// `PM_SpectatorMove` in eye: the observer's pitch from the target's entity pitch.
    fn in_eye_pitch(&self, target_pitch: f32) -> f32;

    /// `PM_CalcRoll`: below `rollspeed` the roll is `side * rollangle / rollspeed` (false) or
    /// `side / rollspeed * rollangle` (true), both on the FPU stack.
    fn roll_divides_first(&self) -> bool;

    /// `PM_Friction`'s edge-trace start, 16 units ahead: `vel[i] / speed * 16 + origin[i]`
    /// (false) or `(1 / speed) * vel[i] * 16 + origin[i]` with the reciprocal kept on the FPU
    /// stack (true).
    fn friction_edge_by_reciprocal(&self) -> bool;

    /// Whether the movement's `float` compares follow IEEE (a NaN compares false and unequal)
    /// under [`Arith::Msvc`]: the build's compiler tests `fcom` through the parity flag
    /// ([`crate::pm::Fpu::IEEE_CMP`], [`crate::pm::MsvcIeee`]). False tests C3/C0, so a NaN takes
    /// the "equal" and "below" branches. Only NaN movement state behaves differently.
    /// [`Arith::Gcc`] keeps `ts_i386.so`'s compares.
    fn ieee_compares(&self) -> bool;

    /// `GetSpeedBySlots`: the move speed for `free` inventory slots. Server only: the client gets
    /// it as `maxspeed`.
    fn speed_by_slots(&self, free: i32, the_one: bool) -> f32;

    /// `CBasePlayer::SetPlayerSpeed`: the top of its clamp, and the speed `TSInit` starts a player
    /// with. Server only.
    fn max_player_speed(&self) -> f32;
}
