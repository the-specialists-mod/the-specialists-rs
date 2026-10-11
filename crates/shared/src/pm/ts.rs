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
/// Wall jump: pushed away from the wall at 210 u/s, up at the normal jump speed.
pub const WALLJUMP_SPEED: f32 = 210.0;
/// Wall climb: `sqrt(2 * 800 * 70)`.
pub const WALLCLIMB_VELOCITY: f32 = 334.664;

/// "Airborne stunt": dive, flips, wall jump. Zeroes the usercmd (`PM_CheckParamters`).
pub const AIRBORNE_MASK: i32 = DIVING | FLIP_HIGH | FLIP_LOW | WALLJUMPING; // 0x10c10
/// Bits that block `PM_Jump` (without latching `IN_JUMP`).
pub const NO_JUMP_MASK: i32 = GO_DUCK | DIVING | PRONE | FLIP_HIGH | FLIP_LOW | WALLJUMPING | WALLCLIMBING; // 0x30c34
/// Bits that make `PM_Duck` do nothing (the stunt owns the hull).
pub const NO_DUCK_MASK: i32 = NO_JUMP_MASK | STAND_UP; // 0x30c3c

impl Pm<'_> {
    /// `TSHandle`: runs after `PM_Ladder`, before the step sound and `PM_Duck`.
    pub(crate) fn ts_handle(&mut self) {
        if self.p.cmd.buttons & IN_JUMP as u16 != 0 && self.p.onground == -1 {
            self.pseudo_climb();
        }
        let c = &mut self.p.cmd;
        if self.p.iuser4 & RECOVER != 0 {
            c.forwardmove *= 0.5;
            c.sidemove *= 0.5;
            c.upmove *= 0.5;
        }
        if self.p.iuser4 & PRONE != 0 {
            c.forwardmove = 0.0;
            c.sidemove *= 0.3;
            c.upmove *= 0.3;
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
        normalize(&mut dir);
        if self.p.iuser4 & WALLDIVE != 0 {
            dir = scale(self.p.forward, 3.0);
        }
        let ds = Ext::from_i32(divespeed);
        self.p.velocity[0] = (ds * x(dir[0]) + x(self.p.velocity[0])).f();
        self.p.velocity[1] = (ds * x(dir[1]) + x(self.p.velocity[1])).f();
        self.p.velocity[2] = Ext::from_i32(upspeed * 1600).sqrt().f();
        self.p.onground = -1;
        self.p.iuser4 &= !WALLDIVE;
        self.fixup_gravity_velocity();
    }

    /// `TSWallJump` (0xf9444).
    fn ts_wall_jump(&mut self) {
        self.p.onground = -1;
        if self.p.iuser4 & WALLJUMP_IMPULSE != 0 {
            let speed = WALLJUMP_SPEED;
            for i in 0..2 {
                self.p.velocity[i] = -self.p.forward[i] * speed;
            }
            self.p.velocity[2] = JUMP_VELOCITY;
            self.p.iuser4 &= !WALLJUMP_IMPULSE;
        }
        self.fixup_gravity_velocity();
    }

    /// `TSWallJumpClimb` (0xf94e0).
    fn ts_wall_jump_climb(&mut self) {
        self.p.onground = -1;
        if self.p.iuser4 & WALLCLIMB_IMPULSE != 0 {
            self.p.velocity[2] = WALLCLIMB_VELOCITY;
            self.p.iuser4 &= !WALLCLIMB_IMPULSE;
        }
        self.fixup_gravity_velocity();
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
        if !(-60.0..=60.0).contains(&pitch) {
            return;
        }
        let climb = pitch < -30.0;
        let savehull = self.p.usehull;
        self.p.usehull = 2;
        // One rounding for the eye height and the +40 (TS adds both on the FPU stack).
        let z = x(self.p.origin[2]) + x(self.p.view_ofs[2]);
        let z = if climb { z + x(40.0) } else { z };
        let mut start = [self.p.origin[0], self.p.origin[1], z.f()];
        let fwd = |i: usize, d: f32, s: f32| (x(self.p.forward[i]) * x(d) + x(s)).f();
        let end = [fwd(0, 64.0, start[0]), fwd(1, 64.0, start[1]), start[2]];
        let tr = self.trace(start, end);
        if eq87(tr.fraction, 1.0) {
            start[2] = self.p.origin[2] + self.p.player_mins[savehull as usize & 3][2];
            let end = [fwd(0, 32.0, start[0]), fwd(1, 32.0, start[1]), start[2]];
            self.p.usehull = 2;
            let tr = self.trace(start, end);
            if !eq87(tr.fraction, 1.0) && x(length(self.p.velocity)) < xd(0.5) * x(self.p.maxspeed) {
                self.p.onground = -1;
                self.p.velocity[2] = (x(16.0) * x(self.p.frametime) * xd(100.0) + x(self.p.velocity[2])).f();
                self.fixup_gravity_velocity();
            }
        }
        self.p.usehull = savehull;
        self.fix_player_crouch_stuck(1);
        self.categorize_position();
    }
}
