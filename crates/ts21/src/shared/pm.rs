//! TS 2.1's movement (issue #100 §2.1) and speed by free slots (§2.2): the `PmRules` overrides.
//!
//! Addresses are TS 2.1 `mp.dll`'s (decomp `ts21/server-win`, `docs/ts21/movement.md` §2). Its
//! arithmetic is MSVC's at 53-bit precision, like 3.0's `mp.dll` (its CRT sets `_PC_53` at
//! 0x1006fa8b), so `Arith::Msvc` reproduces it. Two things differ from 3.0's machine code
//! everywhere here:
//!
//! * 2.1's compiler tests `fcom` results through the parity flag, so its comparisons follow IEEE
//!   (a NaN is unequal to everything and compares false), which is what Rust's operators do.
//!   3.0's `mp.dll` sends a NaN the other way ([`shared::pm`]'s `eq87`/`below87`). The shared
//!   SDK movement follows 2.1's through `PmRules::ieee_compares` (issue #105).
//! * 2.1's `client.dll` was built without optimisation and stores every intermediate to a
//!   `float`, so 2.1's own prediction could differ from its server in the last bit. Both sides
//!   here run the server's arithmetic, as the port does for 3.0 (docs/implementation/ts21.md).
//!
//! TS 2.1 shipped no Linux server, so there is no GCC build to match: under the server's
//! `ts_pm_arith gcc` these run 2.1's formulas with `ts_i386.so`'s rounding (every value the C
//! stores is rounded to a `float`).

use crate::Ts21;
use shared::gameplay::PmRules;
use shared::gameplay::pm::{Arith, GroundAccel, Jump, TextureLimits, Vec3};
use shared::pm::{Fpu, FpuMath, Gcc, Msvc, Num};

/// `PM_Accelerate` (0x100315f0): the ground acceleration factor while not prone and not ducked,
/// the `double` 0.7 (0x10082938). 3.0 has `PM_GetAccelBySpeed` instead.
pub const GROUND_ACCEL: f64 = 0.7;
/// `TSHandle` (0x10058400): the move scale while `RECOVER`, the `double` 0.1 (0x10080200).
pub const RECOVER_INPUT_SCALE: f64 = 0.1;
/// `TSHandle` (0x10058400): the forward move while `PRONE` is multiplied by the `double` 0.0
/// (0x10080180) rather than set to 0.
pub const PRONE_FORWARD_SCALE: f64 = 0.0;
/// `TSWallJump` (0x100581d0): away from the wall at 170 u/s (the `float` 0x432a0000).
pub const WALLJUMP_SPEED: f32 = 170.0;
/// `TSWallJump` (0x100581d0): `sqrt(80000.0)` (`fsqrt` of the `double` at 0x100890d8, stored),
/// a 50-unit jump: 0x438d6bde, 282.84271.
pub const WALLJUMP_UP_VELOCITY: f32 = f32::from_bits(0x438d_6bde);
/// `TSWallJumpClimb` (0x10058270): `sqrt(96000.0)` (the `double` at 0x100890e0), a 60-unit
/// climb: 0x439aeb59, 309.83865.
pub const WALLCLIMB_VELOCITY: f32 = f32::from_bits(0x439a_eb59);
/// `PM_Jump` (0x10058680): the push along the velocity on a strafe jump and a super jump (the
/// `float` 100 at 0x10081264).
pub const JUMP_PUSH: f32 = 100.0;
/// `PM_Jump` (0x10058680, `test byte [cmd.buttons + 1], 6`): `IN_MOVELEFT | IN_MOVERIGHT`
/// (SDK `in_buttons.h`: 1 << 9, 1 << 10).
pub const STRAFE_BUTTONS: u16 = 0x600;
/// `CBasePlayer::SetPlayerSpeed` (0x100557b0): the clamp's top, and `TSInit`'s (0x100552f0)
/// `SetPlayerSpeed(310)`. Also `GetSpeedBySlots`' speed with 70 or more free slots.
pub const MAX_SPEED: f32 = 310.0;
/// `GetSpeedBySlots` (0x100436e0): from this many free slots on, the speed is [`MAX_SPEED`].
pub const FULL_SPEED_SLOTS: i32 = 70;

impl PmRules for Ts21 {
    fn ground_accel(&self) -> GroundAccel {
        GroundAccel::Constant(GROUND_ACCEL)
    }

    fn recover_input_scale(&self) -> f64 {
        RECOVER_INPUT_SCALE
    }

    fn prone_forward_scale(&self) -> Option<f64> {
        Some(PRONE_FORWARD_SCALE)
    }

    fn wall_jump_speed(&self) -> f32 {
        WALLJUMP_SPEED
    }

    fn wall_jump_up_velocity(&self) -> f32 {
        WALLJUMP_UP_VELOCITY
    }

    fn wall_climb_velocity(&self) -> f32 {
        WALLCLIMB_VELOCITY
    }

    /// `TSDive` (0x100582d0) has no wall dive: the dive always follows the velocity, and it never
    /// clears `WALLDIVE` (2.1's stunt code never sets it).
    fn wall_dive(&self) -> bool {
        false
    }

    /// `FixVelocity` (0x10057d10), which 2.1's dive, wall jump and wall climb call after their
    /// impulse (with twice `maxspeed`) and its strafe jump with `maxspeed`.
    fn fix_velocity(&self, arith: Arith, velocity: &mut Vec3, limit: f32) {
        match arith {
            Arith::Gcc => fix_velocity::<Gcc>(velocity, limit),
            Arith::Msvc => fix_velocity::<Msvc>(velocity, limit),
        }
    }

    /// `PM_Jump` (0x10058680): the super jump adds [`JUMP_PUSH`] along the velocity, unclamped.
    /// The normal jump does the same when a strafe key is held, then `FixVelocity`s the velocity
    /// to `maxspeed` (its z is still the ground's, so this caps the horizontal speed). Both run
    /// before the upward speed is set, as in 2.1.
    fn jump_push(&self, arith: Arith, jump: Jump, buttons: u16, maxspeed: f32, velocity: &mut Vec3) {
        match jump {
            Jump::High => push_along(arith, velocity, JUMP_PUSH),
            Jump::Normal if buttons & STRAFE_BUTTONS != 0 => {
                push_along(arith, velocity, JUMP_PUSH);
                self.fix_velocity(arith, velocity, maxspeed);
            }
            Jump::Normal => {}
        }
    }

    /// `PM_PlayerMove` (0x10033c90): a time scale of 0 is normal speed in 2.1 (its encoding of
    /// normal); otherwise the same freeze below 0.1, but a NaN scale isn't frozen.
    fn scaled_frametime(&self, frametime: f32, fuser1: f32) -> f32 {
        if fuser1 != 0.0 { frametime * if fuser1 < shared::ts::TIME_SCALE_FREEZE { 0.0 } else { fuser1 } } else { frametime }
    }

    /// `PM_DropPunchAngle` (0x10033b10): no zeroing while dead, a time scale of 0 counts as 1, and
    /// the length is stored to a `float` before the decay.
    fn drop_punch_angle(&self, arith: Arith, punch: Vec3, _dead: bool, fuser1: f32, frametime: f32) -> Vec3 {
        match arith {
            Arith::Gcc => drop_punch_angle::<Gcc>(punch, fuser1, frametime),
            Arith::Msvc => drop_punch_angle::<Msvc>(punch, fuser1, frametime),
        }
    }

    /// `PM_UpdateStepSound` (0x10030b00, at 0x10030dad): `(int)(interval / fuser1)` when the time
    /// scale isn't 0, with no 0.1 floor.
    fn step_interval(&self, arith: Arith, interval: i32, fuser1: f32) -> i32 {
        if fuser1 == 0.0 {
            return interval;
        }
        match arith {
            Arith::Gcc => (Gcc::xi(interval) / Gcc::x(fuser1)).trunc_i32(),
            Arith::Msvc => (Msvc::xi(interval) / Msvc::x(fuser1)).trunc_i32(),
        }
    }

    /// `GetSpeedBySlots` (0x100436e0): `free >= 70 ? 310 : (int)(free * (1/70.f) * 140 + 170)`,
    /// 170..310, truncated by `_ftol`. 2.1 has no The One speed. Every step is exact in `f64` (a
    /// 24-bit `float` times small integers), so the truncation sees 2.1's value.
    fn speed_by_slots(&self, free: i32, _the_one: bool) -> f32 {
        if free >= FULL_SPEED_SLOTS {
            return MAX_SPEED;
        }
        (Msvc::xi(free) * Msvc::x(0.014_285_714) * Msvc::x(140.0) + Msvc::x(170.0)).trunc_i32() as f32
    }

    fn max_player_speed(&self) -> f32 {
        MAX_SPEED
    }

    /// Issue #105: 2.1's `mp.dll` tests every `fcom` in its movement through the parity flag
    /// (`test ah,0x44; jp` for `==`, `test ah,0x05; jp` for `<`, `test ah,0x41; jp` for `<=`;
    /// e.g. `PM_Accelerate` 0x1003160a / 0x10031644, `PM_Jump` 0x100586ca / 0x100586ff /
    /// 0x10058775, `VectorNormalize` 0x1002fc3c, `PseudoClimb` 0x10057d84, the glide 0x10058c07),
    /// so the shared SDK movement compares as IEEE: a NaN `waterjumptime`, `flSwimTime`, velocity
    /// or trace fraction goes the C expression's way rather than 3.0's.
    fn ieee_compares(&self) -> bool {
        true
    }

    /// The SDK's table (issue #145): `PM_InitTextureTypes` (0x1002ff30) stops at 512 entries
    /// and cuts names to 12 characters in 13-byte slots, `pfnPM_FindTextureType` (0x100301c0)
    /// compares 12 (`strnicmp(.., 0xc)`), and `PM_CatagorizeTextureType` (0x10030a20) ends the
    /// name at `sztexturename[12]`. The shipped `materials.txt` lists about 1140, so entries
    /// 513 on aren't known. 2.1's `client.dll` table (0x10058610) has the same limits.
    fn texture_limits(&self) -> TextureLimits {
        TextureLimits::SDK
    }

    /// `PM_SpectatorMove` (0x100324c0, at 0x100325d0): in eye, the target's pitch × the float
    /// -2.0 (0x10085f14). The engine sets every player's entity pitch to `-v_angle / 3`
    /// (`SV_RunCmd`), in 2.1 as now, so the in-eye observer's view pitch is two thirds of the
    /// target's, as in 2.1. The product is exact.
    fn in_eye_pitch(&self, target_pitch: f32) -> f32 {
        target_pitch * IN_EYE_PITCH_SCALE
    }

    /// `PM_CalcRoll` (0x10033af1): `fdiv [rollspeed]; fmul [rollangle]`.
    fn roll_divides_first(&self) -> bool {
        true
    }

    /// `PM_Friction` (0x10031728..0x1003174d): `fld 1.0; fdiv speed`, then `inv * vel[i] * 16 +
    /// origin[i]` for x and y.
    fn friction_edge_by_reciprocal(&self) -> bool {
        true
    }
}

/// `PM_SpectatorMove`'s in-eye pitch factor, the float at 0x10085f14.
pub const IN_EYE_PITCH_SCALE: f32 = -2.0;

/// `VectorNormalize` (0x1002fc10): 3.0's, except that its `length != 0` test is IEEE's, so a NaN
/// length normalises. Returns the length (unrounded in MSVC, a `float` in GCC).
fn normalize<F: FpuMath>(v: &mut Vec3) -> F::N {
    let x = F::x;
    let r = (x(v[0]) * x(v[0]) + x(v[1]) * x(v[1]) + x(v[2]) * x(v[2])).sqrt();
    let len = if F::MSVC { r } else { x(r.f()) };
    if len != F::ZERO {
        let il = x(1.0) / len;
        for c in v.iter_mut() {
            *c = (il * x(*c)).f();
        }
    }
    len
}

/// `FixVelocity` (0x10057d10): when the 3D speed is above `limit`, scales x and y (not z) by
/// `limit / speed`. The speed is stored to a `float`, and the ratio too, but MSVC scales x by
/// the ratio still on the stack and y by the stored one.
fn fix_velocity<F: FpuMath>(v: &mut Vec3, limit: f32) {
    let len = F::length(*v);
    let stored = len.f();
    if len != F::ZERO && stored > limit {
        let k = F::x(limit) / F::x(stored);
        let kf = k.f();
        let kx = if F::MSVC { k } else { F::x(kf) };
        v[0] = (kx * F::x(v[0])).f();
        v[1] = (F::x(kf) * F::x(v[1])).f();
    }
}

/// `PM_Jump` (0x10058680): `v.xy += push * normalize(v).xy`, each component rounded once.
fn push_along(arith: Arith, v: &mut Vec3, push: f32) {
    fn go<F: FpuMath>(v: &mut Vec3, push: f32) {
        let mut dir = *v;
        normalize::<F>(&mut dir);
        for i in 0..2 {
            v[i] = (F::x(dir[i]) * F::x(push) + F::x(v[i])).f();
        }
    }
    match arith {
        Arith::Gcc => go::<Gcc>(v, push),
        Arith::Msvc => go::<Msvc>(v, push),
    }
}

/// `PM_DropPunchAngle` (0x10033b10): `len -= (0.5·len + 10) · frametime · (fuser1 ? fuser1 : 1)`,
/// clamped at 0 (a NaN too), with the length stored to a `float` first. Dead players too.
fn drop_punch_angle<F: FpuMath>(punch: Vec3, fuser1: f32, frametime: f32) -> Vec3 {
    let mut dir = punch;
    let len = F::x(normalize::<F>(&mut dir).f());
    let scale = if fuser1 != 0.0 { F::x(fuser1) } else { F::xd(1.0) };
    let d = len - (F::xd(0.5) * len + F::xd(10.0)) * F::x(frametime) * scale;
    let len = if d > F::xd(0.0) { d.f() } else { 0.0 };
    [dir[0] * len, dir[1] * len, dir[2] * len]
}

#[cfg(test)]
mod tests;
