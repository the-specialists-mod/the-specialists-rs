//! TS additions to player movement: `TSHandle` (0xf96a0) and the stunt impulses it applies.
//!
//! The server's stunt code (`CTSStunt`) sets bits in `iuser4` (`crate::ts::stunt`); these
//! functions consume them on both sides. Constants are the TS ones (decomp
//! `docs/server/player-movement.md` §1.2).

use super::math::*;
use super::{Pm, VIEW_DUCK, VIEW_STAND};
use crate::ts::stunt::*;
use goldsrc_sys::{FL_DUCKING, IN_DUCK, IN_JUMP};

/// Normal jump: `sqrt(2 * 800 * 35)`, a 35-unit jump (SDK 45 / 268.33).
pub const JUMP_VELOCITY: f32 = 236.643_19;
/// Super jump (physinfo `mj` = 1, ducked, duck held, within the duck window).
pub const HIGH_JUMP_VELOCITY: f32 = 400.0;

/// "Airborne stunt": dive, flips, wall jump. Zeroes the usercmd (`PM_CheckParamters`).
pub const AIRBORNE_MASK: i32 = DIVING | FLIP_HIGH | FLIP_LOW | WALLJUMPING; // 0x10c10
/// Bits that block `PM_Jump` (without latching `IN_JUMP`).
pub const NO_JUMP_MASK: i32 = GO_DUCK | DIVING | PRONE | FLIP_HIGH | FLIP_LOW | WALLJUMPING | WALLCLIMBING; // 0x30c34
/// Bits that make `PM_Duck` do nothing (the stunt owns the hull).
pub const NO_DUCK_MASK: i32 = NO_JUMP_MASK | STAND_UP; // 0x30c3c

impl<F: Fpu> Pm<'_, F> {
    /// `TSHandle`: runs after `PM_Ladder`, before the step sound and `PM_Duck`.
    pub(crate) fn ts_handle(&mut self) {
        if self.p.cmd.buttons & IN_JUMP as u16 != 0 && self.p.onground == -1 {
            self.pseudo_climb();
        }
        let c = &mut self.p.cmd;
        if self.p.iuser4 & RECOVER != 0 {
            // A `double` multiplying each move on the FPU stack (mp.dll 0x10089180).
            let k = F::xd(self.rules.recover_input_scale());
            c.forwardmove = (F::x(c.forwardmove) * k).f();
            c.sidemove = (F::x(c.sidemove) * k).f();
            c.upmove = (F::x(c.upmove) * k).f();
        }
        if self.p.iuser4 & PRONE != 0 {
            c.forwardmove = match self.rules.prone_forward_scale() {
                None => 0.0,
                Some(k) => (F::x(c.forwardmove) * F::xd(k)).f(),
            };
            // A `double` 0.3 in every build (ts_i386.so 0xf972f `fldl` of file 0xf7fb8,
            // client.dll 0x100536ed, mp.dll), each product rounded once.
            c.sidemove = (F::x(c.sidemove) * F::xd(0.3)).f();
            c.upmove = (F::x(c.upmove) * F::xd(0.3)).f();
        }
        if self.p.iuser4 & DIVE_IMPULSE_MASK != 0 {
            self.ts_dive();
        }
        if self.p.iuser4 & WALLJUMP_MASK != 0 {
            self.ts_wall_jump();
        }
        if self.p.iuser4 & WALLCLIMB_IMPULSE != 0 {
            self.ts_wall_jump_climb();
        }
        if self.p.iuser4 & GO_DUCK != 0 {
            self.ts_go_duck();
        }
        if self.p.iuser4 & STAND_UP != 0 {
            self.ts_stand_up();
        }
    }

    /// `TSDive` (0xf9534): one-shot dive impulse. `vel.xy += speed * dir`, `vel.z = sqrt(h * 1600)`.
    fn ts_dive(&mut self) {
        let (divespeed, upspeed): (i32, i32) = if self.p.iuser4 & DIVE_MED != 0 {
            self.p.iuser4 &= !DIVE_MED;
            (90, 25)
        } else if self.p.iuser4 & DIVE_HIGH != 0 {
            self.p.iuser4 &= !DIVE_HIGH;
            (90, 50)
        } else {
            self.p.iuser4 &= !DIVE_LOW;
            (100, 10)
        };
        let mut dir = self.p.velocity;
        F::normalize(&mut dir);
        let wall_dive = self.rules.wall_dive();
        if wall_dive && self.p.iuser4 & WALLDIVE != 0 {
            dir = scale(self.p.forward, 3.0);
        }
        let ds = F::xi(divespeed);
        self.p.velocity[0] = (ds * F::x(dir[0]) + F::x(self.p.velocity[0])).f();
        self.p.velocity[1] = (ds * F::x(dir[1]) + F::x(self.p.velocity[1])).f();
        self.p.velocity[2] = F::xi(upspeed * 1600).sqrt().f();
        self.p.onground = -1;
        if wall_dive {
            self.p.iuser4 &= !WALLDIVE;
        }
        self.fixup_gravity_velocity();
        self.fix_velocity_to_twice_maxspeed();
    }

    /// `FixVelocity(pmove->velocity, maxspeed + maxspeed)` after a stunt impulse
    /// ([`PmRules::fix_velocity`](crate::gameplay::PmRules::fix_velocity)).
    fn fix_velocity_to_twice_maxspeed(&mut self) {
        let limit = self.p.maxspeed + self.p.maxspeed;
        self.rules.fix_velocity(F::ARITH, &mut self.p.velocity, limit);
    }

    /// `TSWallJump` (0xf9444).
    fn ts_wall_jump(&mut self) {
        self.p.onground = -1;
        if self.p.iuser4 & WALLJUMP_IMPULSE != 0 {
            let speed = self.rules.wall_jump_speed();
            for i in 0..2 {
                self.p.velocity[i] = -self.p.forward[i] * speed;
            }
            self.p.velocity[2] = self.rules.wall_jump_up_velocity();
            self.p.iuser4 &= !WALLJUMP_IMPULSE;
        }
        self.fixup_gravity_velocity();
        self.fix_velocity_to_twice_maxspeed();
    }

    /// `TSWallJumpClimb` (0xf94e0).
    fn ts_wall_jump_climb(&mut self) {
        self.p.onground = -1;
        if self.p.iuser4 & WALLCLIMB_IMPULSE != 0 {
            self.p.velocity[2] = self.rules.wall_climb_velocity();
            self.p.iuser4 &= !WALLCLIMB_IMPULSE;
        }
        self.fixup_gravity_velocity();
        self.fix_velocity_to_twice_maxspeed();
    }

    /// `TSGoDuck` (0xf93a8): instant duck hull while a stunt runs (no 0.4 s transition).
    fn ts_go_duck(&mut self) {
        if self.p.usehull == 1 {
            return;
        }
        self.p.usehull = 1;
        self.p.flags |= FL_DUCKING as i32;
        if self.p.onground != -1 {
            self.p.origin = self.duck_hull_offset(-1.0);
            self.fix_player_crouch_stuck(1);
            self.categorize_position();
        }
    }

    /// `TSStandUp` (0xf91fc): `PM_UnDuck` that keeps the duck hull while `IN_DUCK` is held.
    /// Nothing on the TS server sets `STAND_UP`, so this is dead code kept for parity.
    fn ts_stand_up(&mut self) {
        let new_origin = self.duck_hull_offset(1.0);
        let trace = self.trace(new_origin, new_origin);
        if self.p.cmd.buttons & IN_DUCK as u16 != 0 {
            self.p.usehull = 1;
            self.p.view_ofs[2] = VIEW_DUCK;
            self.p.iuser4 &= !STAND_UP;
            return;
        }
        if trace.startsolid != 0 {
            self.p.usehull = 1;
            self.p.flags |= FL_DUCKING as i32;
            return;
        }
        self.p.usehull = 0;
        let trace = self.trace(new_origin, new_origin);
        if trace.startsolid != 0 {
            self.p.usehull = 1;
            return;
        }
        self.p.view_ofs[2] = VIEW_STAND;
        self.p.origin = new_origin;
        self.fix_player_crouch_stuck(1);
        self.categorize_position();
        self.p.iuser4 &= !STAND_UP;
    }

    /// `PseudoClimb` (0xf8fc8): ledge mantle assist. In the air with jump held and the pitch
    /// within ±60: if the space 64 units ahead at eye height is clear (+40 when looking up past
    /// 30) but something blocks 32 units ahead at the feet, and the player is slower than half
    /// maxspeed, add `1600 * frametime` upward velocity.
    fn pseudo_climb(&mut self) {
        let pitch = self.p.angles[0];
        // `pitch < -60 || pitch > 60`: 3.0's MSVC (0x10064a14) tests the first on C0, so a NaN
        // pitch returns.
        if F::cmp_lt(pitch, -60.0) || pitch > 60.0 {
            return;
        }
        let climb = pitch < -30.0;
        let savehull = self.p.usehull;
        self.p.usehull = 2;
        // One rounding for the eye height and the +40 (TS adds both on the FPU stack). MSVC
        // starts from `fild` of 40 or 0, then adds the view height and the origin.
        let z = if F::MSVC {
            F::xi(if climb { 40 } else { 0 }) + F::x(self.p.view_ofs[2]) + F::x(self.p.origin[2])
        } else {
            let z = F::x(self.p.origin[2]) + F::x(self.p.view_ofs[2]);
            if climb { z + F::x(40.0) } else { z }
        };
        let mut start = [self.p.origin[0], self.p.origin[1], z.f()];
        let fwd = |i: usize, d: f32, s: f32| (F::x(self.p.forward[i]) * F::x(d) + F::x(s)).f();
        let end = [fwd(0, 64.0, start[0]), fwd(1, 64.0, start[1]), start[2]];
        let tr = self.trace(start, end);
        if F::cmp_eq(tr.fraction, 1.0) {
            start[2] = self.p.origin[2] + self.p.player_mins[savehull as usize & 3][2];
            let end = [fwd(0, 32.0, start[0]), fwd(1, 32.0, start[1]), start[2]];
            self.p.usehull = 2;
            let tr = self.trace(start, end);
            if !F::cmp_eq(tr.fraction, 1.0) && F::length(self.p.velocity) < F::xd(0.5) * F::x(self.p.maxspeed) {
                self.p.onground = -1;
                self.p.velocity[2] = (F::x(16.0) * F::x(self.p.frametime) * F::xd(100.0) + F::x(self.p.velocity[2])).f();
                self.fixup_gravity_velocity();
            }
        }
        self.p.usehull = savehull;
        self.fix_player_crouch_stuck(1);
        self.categorize_position();
    }
}
