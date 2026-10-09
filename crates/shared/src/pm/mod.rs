//! Player movement, shared by the server (`pfnPM_Move`) and client prediction (`HUD_PlayerMove`).
//!
//! A port of TS 3.0's `pm_shared.c` (server `ts_i386.so` 0xf8f78-0x1012dd; the client copy is
//! identical): the Half-Life SDK movement code with the TS changes from decomp
//! `docs/server/player-movement.md` §1. Function names follow the C code. All math is `f32` in
//! the same operation order as the C source (the x87 build keeps some intermediates in extended
//! precision, which can't be reproduced exactly).
//!
//! * `ts`: the TS stunt hooks (`TSHandle`, `TSDive`, `PseudoClimb`, ...).
//! * `textures`: `sound/materials.txt`.
//! * `sound`: footsteps (`PM_UpdateStepSound`, `PM_PlayStepSound`).
//!
//! Global state (`OldrotArray`, the stuck tables, the texture table) is per library, as in C.

mod math;
mod sound;
pub mod textures;
pub mod ts;

#[cfg(test)]
mod tests;

pub use math::Vec3;
use math::*;

use crate::{VEC_DUCK_HULL_MAX, VEC_DUCK_HULL_MIN, VEC_HULL_MAX, VEC_HULL_MIN};
use goldsrc_sys::*;
use std::ffi::{CStr, c_char};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering::Relaxed};

// ---- constants (pm_shared.c / TS) --------------------------------------------------------------

const MAX_CLIP_PLANES: usize = 5;
const STOP_EPSILON: f32 = 0.1;
const MAX_PHYSENTS: i32 = 600;
const MAX_CLIENTS: usize = 32;
const PM_NORMAL: i32 = 0;
const PM_DEAD_VIEWHEIGHT: f32 = -8.0;
/// TS view heights: standing 28, ducked 16 (SDK 12).
pub const VIEW_STAND: f32 = 28.0;
pub const VIEW_DUCK: f32 = 16.0;
const TIME_TO_DUCK: f32 = 0.4;
/// TS ladder speed: always 200 (no maxspeed clamp, no duck multiplier).
const MAX_CLIMB_SPEED: f32 = 200.0;
const PLAYER_FALL_PUNCH_THRESHHOLD: f32 = 350.0;
/// TS: 680 (SDK 580).
pub const PLAYER_MAX_SAFE_FALL_SPEED: f32 = 680.0;
const PLAYER_MIN_BOUNCE_SPEED: f32 = 200.0;
const BUNNYJUMP_MAX_SPEED_FACTOR: f32 = 1.7;
const PM_CHECKSTUCK_MINTIME: f32 = 0.05;
const STUCK_MOVEUP: i32 = 1;
const WJ_HEIGHT: f32 = 8.0;
const ATTN_NORM: f32 = 0.8;
const PITCH_NORM: i32 = 100;
/// `modtype_t::mod_brush`.
const MOD_BRUSH: i32 = 0;
/// Spectator modes (`iuser1`).
const OBS_CHASE_LOCKED: i32 = 1;
const OBS_ROAMING: i32 = 3;
const OBS_IN_EYE: i32 = 4;
const OBS_MAP_FREE: i32 = 5;

const FL_ONGROUND_: i32 = FL_ONGROUND as i32;
const FL_DUCKING_: i32 = FL_DUCKING as i32;
const FL_WATERJUMP_: i32 = FL_WATERJUMP as i32;
const FL_FROZEN_: i32 = FL_FROZEN as i32;
const FL_ONTRAIN_: i32 = FL_ONTRAIN as i32;
const IN_JUMP_: u16 = IN_JUMP as u16;
const IN_DUCK_: u16 = IN_DUCK as u16;
const MT_WALK: i32 = MOVETYPE_WALK as i32;
const MT_FLY: i32 = MOVETYPE_FLY as i32;
const MT_NOCLIP: i32 = MOVETYPE_NOCLIP as i32;
const MT_NONE: i32 = MOVETYPE_NONE as i32;
const MT_TOSS: i32 = MOVETYPE_TOSS as i32;
const MT_BOUNCE: i32 = MOVETYPE_BOUNCE as i32;
const MT_BOUNCEMISSILE: i32 = MOVETYPE_BOUNCEMISSILE as i32;
const MT_FLYMISSILE: i32 = MOVETYPE_FLYMISSILE as i32;

// ---- per-library globals -----------------------------------------------------------------------

/// TS `OldrotArray[32]`: last yaw per player, restored while dead or `iuser4 & DEAD_LOCK`.
static OLDROT: [AtomicU32; MAX_CLIENTS] = [const { AtomicU32::new(0) }; MAX_CLIENTS];
/// `rgStuckLast[MAX_CLIENTS][2]`.
static STUCK_LAST: [[AtomicI32; 2]; MAX_CLIENTS] = [const { [const { AtomicI32::new(0) }; 2] }; MAX_CLIENTS];
/// `rgStuckCheckTime[MAX_CLIENTS][2]` (f32 bits).
static STUCK_CHECK_TIME: [[AtomicU32; 2]; MAX_CLIENTS] = [const { [const { AtomicU32::new(0) }; 2] }; MAX_CLIENTS];
/// `rgv3tStuckTable[54]` (`PM_CreateStuckTable`).
static STUCK_TABLE: LazyLock<[Vec3; 54]> = LazyLock::new(create_stuck_table);

fn create_stuck_table() -> [Vec3; 54] {
    let mut t = [[0.0f32; 3]; 54];
    let mut idx = 0;
    let mut push = |v: Vec3| {
        t[idx] = v;
        idx += 1;
    };
    // Little moves along each axis.
    let mut z = -0.125f32;
    while z <= 0.125 {
        push([0.0, 0.0, z]);
        z += 0.125;
    }
    let mut y = -0.125f32;
    while y <= 0.125 {
        push([0.0, y, 0.0]);
        y += 0.125;
    }
    let mut x = -0.125f32;
    while x <= 0.125 {
        push([x, 0.0, 0.0]);
        x += 0.125;
    }
    // Remaining multi-axis nudges.
    let mut x = -0.125f32;
    while x <= 0.125 {
        let mut y = -0.125f32;
        while y <= 0.125 {
            let mut z = -0.125f32;
            while z <= 0.125 {
                push([x, y, z]);
                z += 0.25;
            }
            y += 0.25;
        }
        x += 0.25;
    }
    // Big moves.
    let zi = [0.0f32, 1.0, 6.0];
    for z in zi {
        push([0.0, 0.0, z]);
    }
    let mut y = -2.0f32;
    while y <= 2.0 {
        push([0.0, y, 0.0]);
        y += 2.0;
    }
    let mut x = -2.0f32;
    while x <= 2.0 {
        push([x, 0.0, 0.0]);
        x += 2.0;
    }
    for z in zi {
        let mut x = -2.0f32;
        while x <= 2.0 {
            let mut y = -2.0f32;
            while y <= 2.0 {
                push([x, y, z]);
                y += 2.0;
            }
            x += 2.0;
        }
    }
    t
}

// ---- public API ----------------------------------------------------------------------------------

/// `pfnPM_Init` / `HUD_PlayerMoveInit` (`PM_Init` 0x101280): builds the stuck table and loads
/// `sound/materials.txt` through the engine's file functions.
pub fn init(pm: &mut playermove_t) {
    LazyLock::force(&STUCK_TABLE);
    if textures::is_loaded() {
        return;
    }
    let path = c"sound/materials.txt";
    let (Some(load), Some(free)) = (pm.COM_LoadFile, pm.COM_FreeFile) else { return };
    let mut len: i32 = 0;
    let data = unsafe { load(path.as_ptr(), 5, &mut len) };
    if data.is_null() {
        return;
    }
    if len <= 0 {
        if let Some(size) = pm.COM_FileSize {
            len = unsafe { size(path.as_ptr()) };
        }
    }
    let bytes = unsafe { std::slice::from_raw_parts(data.cast::<u8>(), len.max(0) as usize) };
    textures::load(bytes);
    unsafe { free(data.cast()) };
}

/// Material type for footstep and impact sounds (`CHAR_TEX_*`), from TS's sound/materials.txt.
pub fn find_texture_type(name: &CStr) -> u8 {
    textures::find(name.to_bytes())
}

/// Hull sizes for `pfnGetHullBounds`/`HUD_GetHullBounds`: 0 standing, 1 ducked, 2 point.
pub fn hull_bounds(hull: i32) -> Option<(Vec3, Vec3)> {
    match hull {
        0 => Some((VEC_HULL_MIN, VEC_HULL_MAX)),
        1 => Some((VEC_DUCK_HULL_MIN, VEC_DUCK_HULL_MAX)),
        2 => Some(([0.0; 3], [0.0; 3])),
        _ => None,
    }
}

/// Same as Quake's `AngleVectors`.
pub fn angle_vectors(angles: Vec3) -> (Vec3, Vec3, Vec3) {
    math::angle_vectors(angles)
}

/// Runs one usercmd of movement (`PM_Move` 0x101188). Called by the engine on the server and
/// during client prediction; `pmove->server` is set by the engine.
pub fn player_move(p: &mut playermove_t) {
    let mut pm = Pm { p, onladder: false };
    pm.player_move();
    let p = pm.p;
    if p.onground == -1 {
        p.flags &= !FL_ONGROUND_;
    } else {
        p.flags |= FL_ONGROUND_;
    }
    if p.multiplayer == 0 && p.movetype == MT_WALK {
        p.friction = 1.0;
    }
}

// ---- the movement state machine -----------------------------------------------------------------

pub(crate) struct Pm<'a> {
    pub(crate) p: &'a mut playermove_t,
    /// `g_onladder`.
    pub(crate) onladder: bool,
}

impl Pm<'_> {
    // ---- engine callbacks ------------------------------------------------------------------------

    fn mv(&self) -> &movevars_s {
        unsafe { &*self.p.movevars }
    }

    pub(crate) fn trace(&self, start: Vec3, end: Vec3) -> pmtrace_t {
        let (mut s, mut e) = (start, end);
        let f = self.p.PM_PlayerTrace.expect("PM_PlayerTrace");
        unsafe { f(s.as_mut_ptr(), e.as_mut_ptr(), PM_NORMAL, -1) }
    }

    pub(crate) fn point_contents(&self, p: Vec3, truecont: Option<&mut i32>) -> i32 {
        let mut p = p;
        let f = self.p.PM_PointContents.expect("PM_PointContents");
        let tc = truecont.map_or(std::ptr::null_mut(), |r| r as *mut i32);
        unsafe { f(p.as_mut_ptr(), tc) }
    }

    pub(crate) fn test_player_position(&self, pos: Vec3, tr: Option<&mut pmtrace_t>) -> i32 {
        let mut pos = pos;
        let Some(f) = self.p.PM_TestPlayerPosition else {
            // Without the callback, treat the spot as free.
            return -1;
        };
        let t = tr.map_or(std::ptr::null_mut(), |r| r as *mut pmtrace_t);
        unsafe { f(pos.as_mut_ptr(), t) }
    }

    pub(crate) fn play_sound(&self, channel: u32, sample: &CStr, volume: f32, attn: f32) {
        if let Some(f) = self.p.PM_PlaySound {
            unsafe { f(channel as i32, sample.as_ptr(), volume, attn, 0, PITCH_NORM) }
        }
    }

    pub(crate) fn random_long(&self, lo: i32, hi: i32) -> i32 {
        match self.p.RandomLong {
            Some(f) => unsafe { f(lo, hi) },
            None => lo,
        }
    }

    /// `atoi(PM_Info_ValueForKey(physinfo, key))`.
    pub(crate) fn info_int(&self, key: &CStr) -> i32 {
        let Some(f) = self.p.PM_Info_ValueForKey else { return 0 };
        let v = unsafe { f(self.p.physinfo.as_ptr(), key.as_ptr()) };
        if v.is_null() {
            return 0;
        }
        let s = unsafe { CStr::from_ptr(v) }.to_bytes();
        // atoi: optional whitespace, sign, digits.
        let s = std::str::from_utf8(s).unwrap_or("").trim_start();
        let end = s.char_indices().find(|&(i, c)| !(c.is_ascii_digit() || (i == 0 && (c == '-' || c == '+')))).map_or(s.len(), |(i, _)| i);
        s[..end].parse().unwrap_or(0)
    }

    fn hull_min(&self, hull: i32) -> Vec3 {
        self.p.player_mins[hull as usize & 3]
    }

    fn hull_max(&self, hull: i32) -> Vec3 {
        self.p.player_maxs[hull as usize & 3]
    }

    fn player_slot(&self) -> Option<usize> {
        let i = self.p.player_index;
        (0..MAX_CLIENTS as i32).contains(&i).then_some(i as usize)
    }

    // ---- PM_PlayerMove (0x1008d8) ------------------------------------------------------------------

    fn player_move(&mut self) {
        self.check_parameters();
        self.p.numtouch = 0;
        // TS: frametime is scaled by the per-player time scale and frozen below 0.1.
        self.p.frametime = self.p.cmd.msec as f32 * 0.001;
        self.p.frametime = if self.p.fuser1 < crate::ts::TIME_SCALE_FREEZE { 0.0 } else { self.p.frametime * self.p.fuser1 };
        // Duck, step, swim and water-jump timers still run on the unscaled msec.
        self.reduce_timers();
        let (f, r, u) = angle_vectors(self.p.angles);
        (self.p.forward, self.p.right, self.p.up) = (f, r, u);

        if self.p.spectator != 0 || self.p.iuser1 > 0 {
            self.spectator_move();
            self.categorize_position();
            return;
        }

        // TS: return on the first stuck result (no duck retry).
        if self.p.movetype != MT_NOCLIP && self.p.movetype != MT_NONE && self.check_stuck() {
            return;
        }

        self.categorize_position();
        self.p.oldwaterlevel = self.p.waterlevel;
        if self.p.onground == -1 {
            self.p.flFallVelocity = -self.p.velocity[2];
        }

        self.onladder = false;
        let mut ladder: Option<usize> = None;
        if self.p.dead == 0 && self.p.flags & FL_ONTRAIN_ == 0 {
            ladder = self.ladder();
            if ladder.is_some() {
                self.onladder = true;
            }
        }

        self.ts_handle();
        self.update_step_sound();
        self.duck();

        if self.p.dead == 0 && self.p.flags & FL_ONTRAIN_ == 0 {
            match ladder {
                Some(i) => self.ladder_move(i),
                None => {
                    if self.p.movetype != MT_WALK && self.p.movetype != MT_NOCLIP {
                        self.p.movetype = MT_WALK;
                    }
                }
            }
        }
        // TS removed the SDK's `IN_USE` velocity *0.3 here.

        match self.p.movetype {
            MT_NONE => {}
            MT_NOCLIP => self.noclip(),
            MT_TOSS | MT_BOUNCE => self.physics_toss(),
            MT_FLY => {
                self.check_water();
                if self.p.cmd.buttons & IN_JUMP_ != 0 {
                    if ladder.is_none() {
                        self.jump();
                    }
                } else {
                    self.p.oldbuttons &= !(IN_JUMP as i32);
                }
                self.p.velocity = add(self.p.velocity, self.p.basevelocity);
                self.fly_move();
                self.p.velocity = sub(self.p.velocity, self.p.basevelocity);
            }
            MT_WALK => self.walk_physics(ladder.is_some()),
            _ => {} // "Bogus pmove player movetype"
        }
    }

    fn walk_physics(&mut self, on_ladder: bool) {
        if !self.in_water() {
            self.add_correct_gravity();
        }
        // Waterjumping: just move.
        if self.p.waterjumptime != 0.0 {
            self.water_jump();
            self.fly_move();
            self.check_water();
            return;
        }
        if self.p.waterlevel >= 2 {
            if self.p.waterlevel == 2 {
                self.check_water_jump();
            }
            // Falling: cancel the water jump.
            if self.p.velocity[2] < 0.0 && self.p.waterjumptime != 0.0 {
                self.p.waterjumptime = 0.0;
            }
            if self.p.cmd.buttons & IN_JUMP_ != 0 {
                self.jump();
            } else {
                self.p.oldbuttons &= !(IN_JUMP as i32);
            }
            self.water_move();
            self.p.velocity = sub(self.p.velocity, self.p.basevelocity);
            self.categorize_position();
        } else {
            if self.p.cmd.buttons & IN_JUMP_ != 0 {
                if !on_ladder {
                    self.jump();
                }
            } else {
                self.p.oldbuttons &= !(IN_JUMP as i32);
            }
            if self.p.onground != -1 {
                self.p.velocity[2] = 0.0;
                self.friction();
            }
            self.check_velocity();
            if self.p.onground != -1 {
                self.walk_move();
            } else {
                self.air_move();
            }
            self.categorize_position();
            self.p.velocity = sub(self.p.velocity, self.p.basevelocity);
            self.check_velocity();
            if !self.in_water() {
                self.fixup_gravity_velocity();
            }
            if self.p.onground != -1 {
                self.p.velocity[2] = 0.0;
            }
            self.check_falling();
        }
        self.play_water_sounds();
    }

    // ---- PM_CheckParamters (0xf9c94) ---------------------------------------------------------------

    fn check_parameters(&mut self) {
        let c = &self.p.cmd;
        let spd = (c.upmove * c.upmove + c.sidemove * c.sidemove + c.forwardmove * c.forwardmove).sqrt();
        let maxspeed = self.p.clientmaxspeed;
        if maxspeed != 0.0 {
            self.p.maxspeed = maxspeed.min(self.p.maxspeed);
        }
        // TS: no IN_USE slowdown, and the command is clamped to maxspeed only for spectators
        // (walk/air move clamp the wish speed anyway).
        if spd != 0.0 && spd > self.p.maxspeed {
            let ratio = self.p.maxspeed / spd;
            if self.p.iuser1 > 0 {
                self.p.cmd.forwardmove *= ratio;
                self.p.cmd.sidemove *= ratio;
                self.p.cmd.upmove *= ratio;
            }
        }
        use crate::ts::stunt;
        if self.p.flags & (FL_FROZEN_ | FL_ONTRAIN_) != 0 || self.p.dead != 0 || self.p.iuser4 & ts::AIRBORNE_MASK != 0 {
            self.p.cmd.forwardmove = 0.0;
            self.p.cmd.sidemove = 0.0;
            self.p.cmd.upmove = 0.0;
        }
        // Super-jump glide: weaker steering, floaty gravity while jump is held.
        if self.p.iuser4 & stunt::SUPERJUMP_GLIDE != 0 {
            self.p.cmd.forwardmove *= 0.5;
            self.p.cmd.sidemove *= 0.25;
            self.p.cmd.upmove *= 0.25;
            self.p.gravity = if self.p.cmd.buttons & IN_JUMP_ == 0 {
                1.0
            } else if self.p.velocity[2] >= 0.0 {
                0.3
            } else {
                0.75
            };
        }

        self.drop_punch_angle();

        if self.p.dead == 0 && self.p.iuser4 & stunt::DEAD_LOCK == 0 {
            let v_angle = add(self.p.cmd.viewangles, self.p.punchangle);
            let rollangle = self.mv().rollangle;
            let rollspeed = self.mv().rollspeed;
            self.p.angles[2] = calc_roll(v_angle, self.p.velocity, rollangle, rollspeed) * 4.0;
            self.p.angles[0] = v_angle[0];
            self.p.angles[1] = v_angle[1];
            if let Some(i) = self.player_slot() {
                OLDROT[i].store(self.p.angles[1].to_bits(), Relaxed);
            }
        } else if let Some(i) = self.player_slot() {
            // TS: only the yaw is restored (SDK: angles = oldangles).
            self.p.angles[1] = f32::from_bits(OLDROT[i].load(Relaxed));
        }

        if self.p.dead != 0 {
            self.p.view_ofs[2] = PM_DEAD_VIEWHEIGHT;
        }
        if self.p.angles[1] > 180.0 {
            self.p.angles[1] -= 360.0;
        }
    }

    /// `PM_DropPunchAngle` (0x100760). TS scales by `fuser1` on top of the already-scaled
    /// frametime, and zeroes the punch while dead.
    fn drop_punch_angle(&mut self) {
        let mut punch = self.p.punchangle;
        let len = if self.p.dead == 0 {
            let len = normalize(&mut punch);
            (len - (len * 0.5 + 10.0) * self.p.frametime * self.p.fuser1).max(0.0)
        } else {
            0.0
        };
        self.p.punchangle = scale(punch, len);
    }

    fn reduce_timers(&mut self) {
        let msec = self.p.cmd.msec as i32;
        if self.p.flTimeStepSound > 0 {
            self.p.flTimeStepSound -= msec;
            if self.p.flTimeStepSound < 0 {
                self.p.flTimeStepSound = 0;
            }
        }
        if self.p.flDuckTime > 0.0 {
            self.p.flDuckTime -= msec as f32;
            if self.p.flDuckTime < 0.0 {
                self.p.flDuckTime = 0.0;
            }
        }
        if self.p.flSwimTime > 0.0 {
            self.p.flSwimTime -= msec as f32;
            if self.p.flSwimTime < 0.0 {
                self.p.flSwimTime = 0.0;
            }
        }
    }

    // ---- velocity helpers --------------------------------------------------------------------------

    pub(crate) fn check_velocity(&mut self) {
        let max = self.mv().maxvelocity;
        for i in 0..3 {
            if self.p.velocity[i].is_nan() {
                self.p.velocity[i] = 0.0;
            }
            if self.p.origin[i].is_nan() {
                self.p.origin[i] = 0.0;
            }
            if self.p.velocity[i] > max {
                self.p.velocity[i] = max;
            } else if self.p.velocity[i] < -max {
                self.p.velocity[i] = -max;
            }
        }
    }

    fn ent_gravity(&self) -> f32 {
        if self.p.gravity != 0.0 { self.p.gravity } else { 1.0 }
    }

    fn add_correct_gravity(&mut self) {
        if self.p.waterjumptime != 0.0 {
            return;
        }
        let g = self.ent_gravity();
        self.p.velocity[2] -= g * self.mv().gravity * 0.5 * self.p.frametime;
        self.p.velocity[2] += self.p.basevelocity[2] * self.p.frametime;
        self.p.basevelocity[2] = 0.0;
        self.check_velocity();
    }

    pub(crate) fn fixup_gravity_velocity(&mut self) {
        if self.p.waterjumptime != 0.0 {
            return;
        }
        let g = self.ent_gravity();
        self.p.velocity[2] -= g * self.mv().gravity * self.p.frametime * 0.5;
        self.check_velocity();
    }

    fn add_gravity(&mut self) {
        let g = self.ent_gravity();
        self.p.velocity[2] -= g * self.mv().gravity * self.p.frametime;
        self.p.velocity[2] += self.p.basevelocity[2] * self.p.frametime;
        self.p.basevelocity[2] = 0.0;
        self.check_velocity();
    }

    fn add_to_touched(&mut self, mut tr: pmtrace_t, impactvelocity: Vec3) -> bool {
        let n = self.p.numtouch.clamp(0, MAX_PHYSENTS);
        if self.p.touchindex[..n as usize].iter().any(|t| t.ent == tr.ent) {
            return false;
        }
        tr.deltavelocity = impactvelocity;
        if n >= MAX_PHYSENTS {
            return false; // "Too many entities were touched!"
        }
        self.p.touchindex[n as usize] = tr;
        self.p.numtouch = n + 1;
        true
    }

    // ---- PM_FlyMove ---------------------------------------------------------------------------------

    pub(crate) fn fly_move(&mut self) -> i32 {
        let numbumps = 4;
        let mut blocked = 0;
        let mut numplanes = 0usize;
        let mut planes = [[0.0f32; 3]; MAX_CLIP_PLANES];
        let mut original_velocity = self.p.velocity;
        let primal_velocity = self.p.velocity;
        let mut new_velocity = [0.0f32; 3];
        let mut all_fraction = 0.0f32;
        let mut time_left = self.p.frametime;

        for _ in 0..numbumps {
            if self.p.velocity[0] == 0.0 && self.p.velocity[1] == 0.0 && self.p.velocity[2] == 0.0 {
                break;
            }
            let end = [
                self.p.origin[0] + time_left * self.p.velocity[0],
                self.p.origin[1] + time_left * self.p.velocity[1],
                self.p.origin[2] + time_left * self.p.velocity[2],
            ];
            let trace = self.trace(self.p.origin, end);
            all_fraction += trace.fraction;
            if trace.allsolid != 0 {
                self.p.velocity = ZERO;
                return 4;
            }
            if trace.fraction > 0.0 {
                self.p.origin = trace.endpos;
                original_velocity = self.p.velocity;
                numplanes = 0;
            }
            if trace.fraction == 1.0 {
                break;
            }
            self.add_to_touched(trace, self.p.velocity);

            if trace.plane.normal[2] > 0.7 {
                blocked |= 1;
            }
            if trace.plane.normal[2] == 0.0 {
                blocked |= 2;
            }
            time_left -= time_left * trace.fraction;

            if numplanes >= MAX_CLIP_PLANES {
                self.p.velocity = ZERO;
                break;
            }
            planes[numplanes] = trace.plane.normal;
            numplanes += 1;

            if numplanes == 1 && self.p.movetype == MT_WALK && (self.p.onground == -1 || self.p.friction != 1.0) {
                for plane in planes.iter().take(numplanes) {
                    if plane[2] > 0.7 {
                        clip_velocity(original_velocity, *plane, &mut new_velocity, 1.0);
                        original_velocity = new_velocity;
                    } else {
                        let ob = 1.0 + self.mv().bounce * (1.0 - self.p.friction);
                        clip_velocity(original_velocity, *plane, &mut new_velocity, ob);
                    }
                }
                self.p.velocity = new_velocity;
                original_velocity = new_velocity;
            } else {
                let mut i = 0;
                while i < numplanes {
                    clip_velocity(original_velocity, planes[i], &mut self.p.velocity, 1.0);
                    let mut j = 0;
                    while j < numplanes {
                        if j != i && dot(self.p.velocity, planes[j]) < 0.0 {
                            break;
                        }
                        j += 1;
                    }
                    if j == numplanes {
                        break;
                    }
                    i += 1;
                }
                if i == numplanes {
                    if numplanes != 2 {
                        self.p.velocity = ZERO;
                        break;
                    }
                    let dir = cross(planes[0], planes[1]);
                    let d = dot(dir, self.p.velocity);
                    self.p.velocity = scale(dir, d);
                }
                if dot(self.p.velocity, primal_velocity) <= 0.0 {
                    self.p.velocity = ZERO;
                    break;
                }
            }
        }
        if all_fraction == 0.0 {
            self.p.velocity = ZERO;
        }
        blocked
    }

    // ---- acceleration and friction ------------------------------------------------------------------

    /// `PM_Accelerate` (0xfdd08) with TS's `PM_GetAccelBySpeed` factor.
    fn accelerate(&mut self, wishdir: Vec3, wishspeed: f32, accel: f32) {
        if self.p.dead != 0 || self.p.waterjumptime != 0.0 {
            return;
        }
        let currentspeed = dot(self.p.velocity, wishdir);
        let addspeed = wishspeed - currentspeed;
        if addspeed <= 0.0 {
            return;
        }
        let mut accelspeed = accel * self.p.frametime * wishspeed * self.p.friction;
        if self.p.iuser4 & crate::ts::stunt::PRONE == 0 && self.p.flags & FL_DUCKING_ == 0 {
            accelspeed *= accel_by_speed(self.p.maxspeed);
        }
        if accelspeed > addspeed {
            accelspeed = addspeed;
        }
        for i in 0..3 {
            self.p.velocity[i] += accelspeed * wishdir[i];
        }
    }

    fn air_accelerate(&mut self, wishdir: Vec3, wishspeed: f32, accel: f32) {
        if self.p.dead != 0 || self.p.waterjumptime != 0.0 {
            return;
        }
        let wishspd = wishspeed.min(30.0);
        let currentspeed = dot(self.p.velocity, wishdir);
        let addspeed = wishspd - currentspeed;
        if addspeed <= 0.0 {
            return;
        }
        let mut accelspeed = accel * wishspeed * self.p.frametime * self.p.friction;
        if accelspeed > addspeed {
            accelspeed = addspeed;
        }
        for i in 0..3 {
            self.p.velocity[i] += accelspeed * wishdir[i];
        }
    }

    /// `PM_Friction` (0xfde08): SDK edge friction, TS extra factor 0.75.
    fn friction(&mut self) {
        if self.p.waterjumptime != 0.0 {
            return;
        }
        let vel = self.p.velocity;
        let speed = (vel[0] * vel[0] + vel[1] * vel[1] + vel[2] * vel[2]).sqrt();
        if speed < 0.1 {
            return;
        }
        let mut drop = 0.0f32;
        if self.p.onground != -1 {
            let start_z = self.p.origin[2] + self.hull_min(self.p.usehull)[2];
            let start = [self.p.origin[0] + vel[0] / speed * 16.0, self.p.origin[1] + vel[1] / speed * 16.0, start_z];
            let stop = [start[0], start[1], start_z - 34.0];
            let trace = self.trace(start, stop);
            let friction = if trace.fraction == 1.0 {
                self.mv().friction * self.mv().edgefriction
            } else {
                self.mv().friction
            };
            let control = if speed < self.mv().stopspeed { self.mv().stopspeed } else { speed };
            drop += control * friction * self.p.friction * 0.75 * self.p.frametime;
        }
        let mut newspeed = speed - drop;
        if newspeed < 0.0 {
            newspeed = 0.0;
        }
        newspeed /= speed;
        self.p.velocity = scale(vel, newspeed);
    }

    /// Horizontal wish velocity from `fmove`/`smove` with flattened forward/right.
    fn flat_wish(&mut self, fmove: f32, smove: f32) -> (Vec3, f32) {
        self.p.forward[2] = 0.0;
        self.p.right[2] = 0.0;
        normalize(&mut self.p.forward);
        normalize(&mut self.p.right);
        let wishvel = [
            self.p.forward[0] * fmove + self.p.right[0] * smove,
            self.p.forward[1] * fmove + self.p.right[1] * smove,
            0.0,
        ];
        let mut wishdir = wishvel;
        let mut wishspeed = normalize(&mut wishdir);
        if wishspeed > self.p.maxspeed {
            wishspeed = self.p.maxspeed;
        }
        (wishdir, wishspeed)
    }

    /// `PM_WalkMove` (0xfa26c). TS: backpedalling at 80 %.
    fn walk_move(&mut self) {
        let mut fmove = self.p.cmd.forwardmove;
        if fmove < 0.0 {
            fmove *= 0.8;
        }
        let smove = self.p.cmd.sidemove;
        let (wishdir, wishspeed) = self.flat_wish(fmove, smove);

        self.p.velocity[2] = 0.0;
        let accel = self.mv().accelerate;
        self.accelerate(wishdir, wishspeed, accel);
        self.p.velocity[2] = 0.0;
        self.p.velocity = add(self.p.velocity, self.p.basevelocity);

        let spd = length(self.p.velocity);
        if spd < 1.0 {
            self.p.velocity = ZERO;
            return;
        }

        let oldonground = self.p.onground;
        let dest = [
            self.p.origin[0] + self.p.velocity[0] * self.p.frametime,
            self.p.origin[1] + self.p.velocity[1] * self.p.frametime,
            self.p.origin[2],
        ];
        let trace = self.trace(self.p.origin, dest);
        if trace.fraction == 1.0 {
            self.p.origin = trace.endpos;
            return;
        }
        if oldonground == -1 && self.p.waterlevel == 0 {
            return;
        }
        if self.p.waterjumptime != 0.0 {
            return;
        }

        let original = self.p.origin;
        let originalvel = self.p.velocity;
        self.fly_move();
        let down = self.p.origin;
        let downvel = self.p.velocity;

        self.p.origin = original;
        self.p.velocity = originalvel;

        let stepsize = self.mv().stepsize;
        let mut dest = self.p.origin;
        dest[2] += stepsize;
        let trace = self.trace(self.p.origin, dest);
        if trace.startsolid == 0 && trace.allsolid == 0 {
            self.p.origin = trace.endpos;
        }
        self.fly_move();

        let mut dest = self.p.origin;
        dest[2] -= stepsize;
        let trace = self.trace(self.p.origin, dest);
        let usedown = if trace.plane.normal[2] < 0.7 {
            true
        } else {
            if trace.startsolid == 0 && trace.allsolid == 0 {
                self.p.origin = trace.endpos;
            }
            // The SDK copies the result into pmove->up (sic).
            self.p.up = self.p.origin;
            let downdist = (down[0] - original[0]) * (down[0] - original[0]) + (down[1] - original[1]) * (down[1] - original[1]);
            let updist = (self.p.up[0] - original[0]) * (self.p.up[0] - original[0]) + (self.p.up[1] - original[1]) * (self.p.up[1] - original[1]);
            downdist > updist
        };
        if usedown {
            self.p.origin = down;
            self.p.velocity = downvel;
        } else {
            self.p.velocity[2] = downvel[2];
        }
    }

    fn air_move(&mut self) {
        let fmove = self.p.cmd.forwardmove;
        let smove = self.p.cmd.sidemove;
        let (wishdir, wishspeed) = self.flat_wish(fmove, smove);
        let accel = self.mv().airaccelerate;
        self.air_accelerate(wishdir, wishspeed, accel);
        self.p.velocity = add(self.p.velocity, self.p.basevelocity);
        self.fly_move();
    }

    // ---- water --------------------------------------------------------------------------------------

    fn in_water(&self) -> bool {
        self.p.waterlevel > 1
    }

    pub(crate) fn check_water(&mut self) -> bool {
        let hull = self.p.usehull;
        let (mins, maxs) = (self.hull_min(hull), self.hull_max(hull));
        let mut point = [
            self.p.origin[0] + (mins[0] + maxs[0]) * 0.5,
            self.p.origin[1] + (mins[1] + maxs[1]) * 0.5,
            self.p.origin[2] + mins[2] + 1.0,
        ];
        self.p.waterlevel = 0;
        self.p.watertype = CONTENTS_EMPTY;
        let is_liquid = |c: i32| c <= CONTENTS_WATER && c > CONTENTS_TRANSLUCENT;
        let mut truecont = 0;
        let cont = self.point_contents(point, Some(&mut truecont));
        if is_liquid(cont) {
            self.p.watertype = cont;
            self.p.waterlevel = 1;
            let heightover2 = (mins[2] + maxs[2]) * 0.5;
            point[2] = self.p.origin[2] + heightover2;
            if is_liquid(self.point_contents(point, None)) {
                self.p.waterlevel = 2;
                point[2] = self.p.origin[2] + self.p.view_ofs[2];
                if is_liquid(self.point_contents(point, None)) {
                    self.p.waterlevel = 3;
                }
            }
            if (CONTENTS_CURRENT_DOWN..=CONTENTS_CURRENT_0).contains(&truecont) {
                const CURRENT: [Vec3; 6] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0, -1.0]];
                let dir = CURRENT[(CONTENTS_CURRENT_0 - truecont) as usize];
                self.p.basevelocity = ma(self.p.basevelocity, 50.0 * self.p.waterlevel as f32, dir);
            }
        }
        self.p.waterlevel > 1
    }

    fn water_move(&mut self) {
        let c = self.p.cmd;
        let mut wishvel = [0.0f32; 3];
        for (i, w) in wishvel.iter_mut().enumerate() {
            *w = self.p.forward[i] * c.forwardmove + self.p.right[i] * c.sidemove;
        }
        if c.forwardmove == 0.0 && c.sidemove == 0.0 && c.upmove == 0.0 {
            wishvel[2] -= 60.0;
        } else {
            wishvel[2] += c.upmove;
        }
        let mut wishdir = wishvel;
        let mut wishspeed = normalize(&mut wishdir);
        if wishspeed > self.p.maxspeed {
            wishvel = scale(wishvel, self.p.maxspeed / wishspeed);
            wishspeed = self.p.maxspeed;
        }
        wishspeed *= 0.8;

        self.p.velocity = add(self.p.velocity, self.p.basevelocity);
        let mut temp = self.p.velocity;
        let speed = normalize(&mut temp);
        let newspeed = if speed != 0.0 {
            let mut ns = speed - self.p.frametime * speed * self.mv().friction * self.p.friction;
            if ns < 0.0 {
                ns = 0.0;
            }
            self.p.velocity = scale(self.p.velocity, ns / speed);
            ns
        } else {
            0.0
        };

        if wishspeed < 0.1 {
            return;
        }
        let addspeed = wishspeed - newspeed;
        if addspeed > 0.0 {
            normalize(&mut wishvel);
            let mut accelspeed = self.mv().accelerate * wishspeed * self.p.frametime * self.p.friction;
            if accelspeed > addspeed {
                accelspeed = addspeed;
            }
            for i in 0..3 {
                self.p.velocity[i] += accelspeed * wishvel[i];
            }
        }

        let dest = ma(self.p.origin, self.p.frametime, self.p.velocity);
        let mut start = dest;
        start[2] += self.mv().stepsize + 1.0;
        let trace = self.trace(start, dest);
        if trace.startsolid == 0 && trace.allsolid == 0 {
            self.p.origin = trace.endpos;
            return;
        }
        self.fly_move();
    }

    fn water_jump(&mut self) {
        if self.p.waterjumptime > 10000.0 {
            self.p.waterjumptime = 10000.0;
        }
        if self.p.waterjumptime == 0.0 {
            return;
        }
        self.p.waterjumptime -= self.p.cmd.msec as f32;
        if self.p.waterjumptime < 0.0 || self.p.waterlevel == 0 {
            self.p.waterjumptime = 0.0;
            self.p.flags &= !FL_WATERJUMP_;
        }
        self.p.velocity[0] = self.p.movedir[0];
        self.p.velocity[1] = self.p.movedir[1];
    }

    fn check_water_jump(&mut self) {
        if self.p.waterjumptime != 0.0 {
            return;
        }
        if self.p.velocity[2] < -180.0 {
            return;
        }
        let mut flatvelocity = [self.p.velocity[0], self.p.velocity[1], 0.0];
        let curspeed = normalize(&mut flatvelocity);
        let mut flatforward = [self.p.forward[0], self.p.forward[1], 0.0];
        normalize(&mut flatforward);
        if curspeed != 0.0 && dot(flatvelocity, flatforward) < 0.0 {
            return;
        }
        let mut start = self.p.origin;
        start[2] += WJ_HEIGHT;
        let end = ma(start, 24.0, flatforward);
        let savehull = self.p.usehull;
        self.p.usehull = 2;
        let tr = self.trace(start, end);
        if tr.fraction < 1.0 && tr.plane.normal[2].abs() < 0.1 {
            start[2] += self.hull_max(savehull)[2] - WJ_HEIGHT;
            let end = ma(start, 24.0, flatforward);
            self.p.movedir = ma(ZERO, -50.0, tr.plane.normal);
            let tr = self.trace(start, end);
            if tr.fraction == 1.0 {
                self.p.waterjumptime = 2000.0;
                self.p.velocity[2] = 225.0;
                self.p.oldbuttons |= IN_JUMP as i32;
                self.p.flags |= FL_WATERJUMP_;
            }
        }
        self.p.usehull = savehull;
    }

    fn play_water_sounds(&mut self) {
        let (old, new) = (self.p.oldwaterlevel, self.p.waterlevel);
        if (old == 0 && new != 0) || (old != 0 && new == 0) {
            self.play_wade_sound();
        }
    }

    pub(crate) fn play_wade_sound(&self) {
        let s = match self.random_long(0, 3) {
            0 => c"player/pl_wade1.wav",
            1 => c"player/pl_wade2.wav",
            2 => c"player/pl_wade3.wav",
            3 => c"player/pl_wade4.wav",
            _ => return,
        };
        self.play_sound(CHAN_BODY, s, 1.0, ATTN_NORM);
    }

    // ---- position -----------------------------------------------------------------------------------

    pub(crate) fn categorize_position(&mut self) {
        self.check_water();
        let point = [self.p.origin[0], self.p.origin[1], self.p.origin[2] - 2.0];
        if self.p.velocity[2] > 180.0 {
            self.p.onground = -1;
            return;
        }
        let tr = self.trace(self.p.origin, point);
        self.p.onground = if tr.plane.normal[2] < 0.7 { -1 } else { tr.ent };
        if self.p.onground != -1 {
            self.p.waterjumptime = 0.0;
            if self.p.waterlevel < 2 && tr.startsolid == 0 && tr.allsolid == 0 {
                self.p.origin = tr.endpos;
            }
        }
        if tr.ent > 0 {
            self.add_to_touched(tr, self.p.velocity);
        }
    }

    fn get_random_stuck_offsets(&self, n: i32, server: i32) -> (usize, Vec3) {
        let slot = STUCK_LAST.get(n as usize).map(|s| &s[(server != 0) as usize]);
        let idx = slot.map_or(0, |s| s.fetch_add(1, Relaxed));
        let i = idx.rem_euclid(54) as usize;
        (i, STUCK_TABLE[i])
    }

    fn reset_stuck_offsets(&self, n: i32, server: i32) {
        if let Some(s) = STUCK_LAST.get(n as usize) {
            s[(server != 0) as usize].store(0, Relaxed);
        }
    }

    /// `PM_CheckStuck`: returns true if the player can't move.
    fn check_stuck(&mut self) -> bool {
        let (pi, sv) = (self.p.player_index, self.p.server);
        let mut traceresult = pmtrace_t::default();
        let hitent = self.test_player_position(self.p.origin, Some(&mut traceresult));
        if hitent == -1 {
            self.reset_stuck_offsets(pi, sv);
            return false;
        }
        let base = self.p.origin;

        // Deal with precision error in network.
        if sv == 0 && (hitent == 0 || !self.p.physents[hitent.clamp(0, 599) as usize].model.is_null()) {
            self.reset_stuck_offsets(pi, sv);
            for _ in 0..54 {
                let (_, offset) = self.get_random_stuck_offsets(pi, sv);
                let test = add(base, offset);
                if self.test_player_position(test, Some(&mut traceresult)) == -1 {
                    self.reset_stuck_offsets(pi, sv);
                    self.p.origin = test;
                    return false;
                }
            }
        }

        let idx = if sv != 0 { 0 } else { 1 };
        let ftime = self.p.Sys_FloatTime.map_or(0.0, |f| unsafe { f() }) as f32;
        if let Some(slot) = STUCK_CHECK_TIME.get(pi as usize) {
            if f32::from_bits(slot[idx].load(Relaxed)) >= ftime - PM_CHECKSTUCK_MINTIME {
                return true;
            }
            slot[idx].store(ftime.to_bits(), Relaxed);
        }

        if let Some(f) = self.p.PM_StuckTouch {
            unsafe { f(hitent, &mut traceresult) };
        }

        let (i, offset) = self.get_random_stuck_offsets(pi, sv);
        let test = add(base, offset);
        let hit = self.test_player_position(test, None);
        if hit == -1 {
            self.reset_stuck_offsets(pi, sv);
            if i >= 27 {
                self.p.origin = test;
            }
            return false;
        }

        // Flailing while stuck in another player: try to force them out.
        if self.p.cmd.buttons & (IN_JUMP | IN_DUCK | IN_ATTACK) as u16 != 0 && self.p.physents[hit.clamp(0, 599) as usize].player != 0 {
            let (xystep, zstep) = (8.0f32, 18.0f32);
            let (xyminmax, zminmax) = (xystep, 4.0 * zstep);
            let mut z = 0.0f32;
            while z <= zminmax {
                let mut x = -xyminmax;
                while x <= xyminmax {
                    let mut y = -xyminmax;
                    while y <= xyminmax {
                        let test = [base[0] + x, base[1] + y, base[2] + z];
                        if self.test_player_position(test, None) == -1 {
                            self.p.origin = test;
                            return false;
                        }
                        y += xystep;
                    }
                    x += xystep;
                }
                z += zstep;
            }
        }
        true
    }

    // ---- spectators, noclip, toss -------------------------------------------------------------------

    /// `PM_SpectatorMove` (0xfef3c): roaming, or follow a target (`iuser2`) with the TS chase
    /// camera (`GetChaseOrigin`).
    fn spectator_move(&mut self) {
        if self.p.iuser1 == OBS_ROAMING || self.p.iuser2 == 0 {
            let speed = length(self.p.velocity);
            if speed < 1.0 {
                self.p.velocity = ZERO;
            } else {
                let friction = self.mv().friction * 1.5;
                let control = if speed < self.mv().stopspeed { self.mv().stopspeed } else { speed };
                let drop = control * friction * self.p.frametime;
                let newspeed = (speed - drop).max(0.0) / speed;
                self.p.velocity = scale(self.p.velocity, newspeed);
            }
            let fmove = self.p.cmd.forwardmove;
            let smove = self.p.cmd.sidemove;
            normalize(&mut self.p.forward);
            normalize(&mut self.p.right);
            let mut wishvel = [0.0f32; 3];
            for (i, w) in wishvel.iter_mut().enumerate() {
                *w = self.p.forward[i] * fmove + self.p.right[i] * smove;
            }
            wishvel[2] += self.p.cmd.upmove;
            let mut wishdir = wishvel;
            let mut wishspeed = normalize(&mut wishdir);
            let maxspd = self.mv().spectatormaxspeed;
            if wishspeed > maxspd {
                wishspeed = maxspd;
            }
            let currentspeed = dot(self.p.velocity, wishdir);
            let addspeed = wishspeed - currentspeed;
            if addspeed <= 0.0 {
                return;
            }
            let mut accelspeed = self.mv().accelerate * self.p.frametime * wishspeed;
            if accelspeed > addspeed {
                accelspeed = addspeed;
            }
            for i in 0..3 {
                self.p.velocity[i] += accelspeed * wishdir[i];
            }
            self.p.origin = ma(self.p.origin, self.p.frametime, self.p.velocity);
            return;
        }

        let n = self.p.numphysent.clamp(0, 600) as usize;
        let Some(target) = (0..n).find(|&i| self.p.physents[i].info == self.p.iuser2) else { return };
        let pe = &self.p.physents[target];
        let mode = self.p.iuser1;
        let angles = match mode {
            OBS_CHASE_LOCKED => [10.0 - pe.angles[0], pe.angles[1], pe.angles[2]],
            OBS_MAP_FREE | OBS_IN_EYE => pe.angles,
            _ => self.p.angles,
        };
        let origin = if mode == OBS_MAP_FREE { pe.origin } else { self.chase_origin(angles, target) };
        self.p.origin = origin;
        self.p.angles = angles;
        self.p.velocity = ZERO;
    }

    /// `GetChaseOrigin` (0xfee5c): 90 units behind the target's eyes (z + 35), pulled 4 units
    /// off any wall in between.
    fn chase_origin(&self, angles: Vec3, target: usize) -> Vec3 {
        let (forward, _, _) = angle_vectors(angles);
        let pe = &self.p.physents[target];
        let mut start = [pe.origin[0], pe.origin[1], pe.origin[2] + 35.0];
        let back = ma(ZERO, 90.0, forward);
        let mut end = sub(start, back);
        let Some(f) = self.p.PM_TraceLine else { return end };
        let tr = unsafe { f(start.as_mut_ptr(), end.as_mut_ptr(), 0, 2, target as i32) };
        if tr.is_null() {
            return end;
        }
        let tr = unsafe { &*tr };
        ma(tr.endpos, 4.0, tr.plane.normal)
    }

    fn noclip(&mut self) {
        let fmove = self.p.cmd.forwardmove;
        let smove = self.p.cmd.sidemove;
        normalize(&mut self.p.forward);
        normalize(&mut self.p.right);
        let mut wishvel = [0.0f32; 3];
        for (i, w) in wishvel.iter_mut().enumerate() {
            *w = self.p.forward[i] * fmove + self.p.right[i] * smove;
        }
        wishvel[2] += self.p.cmd.upmove;
        self.p.origin = ma(self.p.origin, self.p.frametime, wishvel);
        self.p.velocity = ZERO;
    }

    fn push_entity(&mut self, push: Vec3) -> pmtrace_t {
        let end = add(self.p.origin, push);
        let trace = self.trace(self.p.origin, end);
        self.p.origin = trace.endpos;
        if trace.fraction < 1.0 && trace.allsolid == 0 {
            self.add_to_touched(trace, self.p.velocity);
        }
        trace
    }

    fn physics_toss(&mut self) {
        self.check_water();
        if self.p.velocity[2] > 0.0 {
            self.p.onground = -1;
        }
        if self.p.onground != -1 && self.p.basevelocity == ZERO && self.p.velocity == ZERO {
            return;
        }
        self.check_velocity();
        if self.p.movetype != MT_FLY && self.p.movetype != MT_BOUNCEMISSILE && self.p.movetype != MT_FLYMISSILE {
            self.add_gravity();
        }
        self.p.velocity = add(self.p.velocity, self.p.basevelocity);
        self.check_velocity();
        let mv = scale(self.p.velocity, self.p.frametime);
        self.p.velocity = sub(self.p.velocity, self.p.basevelocity);
        let trace = self.push_entity(mv);
        self.check_velocity();
        if trace.allsolid != 0 {
            self.p.onground = trace.ent;
            self.p.velocity = ZERO;
            return;
        }
        if trace.fraction == 1.0 {
            self.check_water();
            return;
        }
        let backoff = match self.p.movetype {
            MT_BOUNCE => 2.0 - self.p.friction,
            MT_BOUNCEMISSILE => 2.0,
            _ => 1.0,
        };
        let v = self.p.velocity;
        clip_velocity(v, trace.plane.normal, &mut self.p.velocity, backoff);
        if trace.plane.normal[2] > 0.7 {
            if self.p.velocity[2] < self.mv().gravity * self.p.frametime {
                self.p.onground = trace.ent;
                self.p.velocity[2] = 0.0;
            }
            let vel = dot(self.p.velocity, self.p.velocity);
            if vel < 30.0 * 30.0 || (self.p.movetype != MT_BOUNCE && self.p.movetype != MT_BOUNCEMISSILE) {
                self.p.onground = trace.ent;
                self.p.velocity = ZERO;
            } else {
                let mv = scale(self.p.velocity, (1.0 - trace.fraction) * self.p.frametime * 0.9);
                self.push_entity(mv);
            }
        }
        self.check_water();
    }

    // ---- ladders ------------------------------------------------------------------------------------

    /// `PM_Ladder`: index into `moveents` of the ladder the player is touching.
    fn ladder(&mut self) -> Option<usize> {
        let n = self.p.nummoveent.clamp(0, 64) as usize;
        for i in 0..n {
            let pe: *mut physent_t = &mut self.p.moveents[i];
            let (model, skin) = unsafe { ((*pe).model, (*pe).skin) };
            if model.is_null() || skin != CONTENTS_LADDER {
                continue;
            }
            let Some(model_type) = self.p.PM_GetModelType else { continue };
            if unsafe { model_type(model) } != MOD_BRUSH {
                continue;
            }
            let (Some(hull_for_bsp), Some(hull_contents)) = (self.p.PM_HullForBsp, self.p.PM_HullPointContents) else { continue };
            let mut test = [0.0f32; 3];
            let hull = unsafe { hull_for_bsp(pe, test.as_mut_ptr()) };
            if hull.is_null() {
                continue;
            }
            // hull_t { clipnodes*, planes*, int firstclipnode, ... }
            let num = unsafe { *hull.cast::<u8>().add(2 * size_of::<*const ()>()).cast::<i32>() };
            let mut test = sub(self.p.origin, test);
            if unsafe { hull_contents(hull.cast(), num, test.as_mut_ptr()) } == CONTENTS_EMPTY {
                continue;
            }
            return Some(i);
        }
        None
    }

    /// `PM_LadderMove` (0xff654): TS climbs at a flat 200 u/s.
    fn ladder_move(&mut self, ladder: usize) {
        if self.p.movetype == MT_NOCLIP {
            return;
        }
        let pe: *mut physent_t = &mut self.p.moveents[ladder];
        let (mut modelmins, mut modelmaxs) = ([0.0f32; 3], [0.0f32; 3]);
        if let Some(f) = self.p.PM_GetModelBounds {
            unsafe { f((*pe).model, modelmins.as_mut_ptr(), modelmaxs.as_mut_ptr()) };
        }
        let mut ladder_center = scale(add(modelmins, modelmaxs), 0.5);
        self.p.movetype = MT_FLY;

        let mut floor = self.p.origin;
        floor[2] += self.hull_min(self.p.usehull)[2] - 1.0;
        let on_floor = self.point_contents(floor, None) == CONTENTS_SOLID;

        self.p.gravity = 0.0;
        let mut trace: trace_t = unsafe { std::mem::zeroed() };
        let mut origin = self.p.origin;
        if let Some(f) = self.p.PM_TraceModel {
            unsafe { f(pe, origin.as_mut_ptr(), ladder_center.as_mut_ptr(), &mut trace) };
        }
        if trace.fraction == 1.0 {
            return;
        }
        let (vpn, v_right, _) = angle_vectors(self.p.angles);
        let b = self.p.cmd.buttons as u32;
        let mut forward = 0.0f32;
        let mut right = 0.0f32;
        if b & IN_BACK != 0 {
            forward -= MAX_CLIMB_SPEED;
        }
        if b & IN_FORWARD != 0 {
            forward += MAX_CLIMB_SPEED;
        }
        if b & IN_MOVELEFT != 0 {
            right -= MAX_CLIMB_SPEED;
        }
        if b & IN_MOVERIGHT != 0 {
            right += MAX_CLIMB_SPEED;
        }
        let normal = trace.plane.normal;
        if b & IN_JUMP != 0 {
            self.p.movetype = MT_WALK;
            self.p.velocity = scale(normal, 270.0);
            return;
        }
        if forward != 0.0 || right != 0.0 {
            let velocity = ma(scale(vpn, forward), right, v_right);
            let mut perp = cross([0.0, 0.0, 1.0], normal);
            normalize(&mut perp);
            let n = dot(velocity, normal);
            let crossv = scale(normal, n);
            let lateral = sub(velocity, crossv);
            let tmp = cross(normal, perp);
            self.p.velocity = ma(lateral, -n, tmp);
            if on_floor && n > 0.0 {
                self.p.velocity = ma(self.p.velocity, MAX_CLIMB_SPEED, normal);
            }
        } else {
            self.p.velocity = ZERO;
        }
    }

    // ---- ducking ------------------------------------------------------------------------------------

    pub(crate) fn fix_player_crouch_stuck(&mut self, direction: i32) {
        if self.test_player_position(self.p.origin, None) == -1 {
            return;
        }
        let test = self.p.origin;
        for _ in 0..36 {
            self.p.origin[2] += direction as f32;
            if self.test_player_position(self.p.origin, None) == -1 {
                return;
            }
        }
        self.p.origin = test;
    }

    /// `PM_UnDuck` (0xff51c), view height 28.
    fn unduck(&mut self) {
        let mut new_origin = self.p.origin;
        if self.p.onground != -1 {
            for i in 0..3 {
                new_origin[i] += self.p.player_mins[1][i] - self.p.player_mins[0][i];
            }
        }
        let trace = self.trace(new_origin, new_origin);
        if trace.startsolid == 0 {
            self.p.usehull = 0;
            let trace = self.trace(new_origin, new_origin);
            if trace.startsolid != 0 {
                self.p.usehull = 1;
                return;
            }
            self.p.flags &= !FL_DUCKING_;
            self.p.bInDuck = 0;
            self.p.view_ofs[2] = VIEW_STAND;
            self.p.flDuckTime = 0.0;
            self.p.origin = new_origin;
            self.categorize_position();
        }
    }

    /// `PM_Duck` (0xf9f7c): SDK ducking with view height 16, disabled while a stunt owns the hull.
    fn duck(&mut self) {
        let buttons = self.p.cmd.buttons as i32;
        let old = self.p.oldbuttons;
        let buttons_changed = old ^ buttons;
        let pressed = buttons_changed & buttons;
        if buttons & IN_DUCK as i32 != 0 {
            self.p.oldbuttons |= IN_DUCK as i32;
        } else {
            self.p.oldbuttons &= !(IN_DUCK as i32);
        }
        if self.p.iuser4 & ts::NO_DUCK_MASK != 0 {
            return;
        }
        if self.p.iuser3 != 0 || self.p.dead != 0 {
            if self.p.flags & FL_DUCKING_ != 0 {
                self.unduck();
            }
            return;
        }
        if self.p.flags & FL_DUCKING_ != 0 {
            self.p.cmd.forwardmove *= 0.333;
            self.p.cmd.sidemove *= 0.333;
            self.p.cmd.upmove *= 0.333;
        }
        if buttons & IN_DUCK as i32 == 0 && self.p.bInDuck == 0 && self.p.flags & FL_DUCKING_ == 0 {
            return;
        }
        if buttons & IN_DUCK as i32 == 0 {
            self.unduck();
            return;
        }
        if pressed & IN_DUCK as i32 != 0 && self.p.flags & FL_DUCKING_ == 0 {
            self.p.flDuckTime = 1000.0;
            self.p.bInDuck = 1;
        }
        let time = (1.0 - self.p.flDuckTime * 0.001).max(0.0);
        if self.p.bInDuck == 0 {
            return;
        }
        if self.p.flDuckTime * 0.001 <= 1.0 - TIME_TO_DUCK || self.p.onground == -1 {
            self.p.usehull = 1;
            self.p.view_ofs[2] = VIEW_DUCK;
            self.p.flags |= FL_DUCKING_;
            self.p.bInDuck = 0;
            if self.p.onground != -1 {
                for i in 0..3 {
                    self.p.origin[i] -= self.p.player_mins[1][i] - self.p.player_mins[0][i];
                }
                self.fix_player_crouch_stuck(STUCK_MOVEUP);
                self.categorize_position();
            }
        } else {
            // fMore = VEC_DUCK_HULL_MIN - VEC_HULL_MIN = 18
            let duck_fraction = spline_fraction(time, 1.0 / TIME_TO_DUCK);
            self.p.view_ofs[2] = (1.0 - duck_fraction) * VIEW_STAND + (VIEW_DUCK - 18.0) * duck_fraction;
        }
    }

    // ---- jumping and falling ------------------------------------------------------------------------

    fn prevent_mega_bunny_jumping(&mut self) {
        let maxscaledspeed = BUNNYJUMP_MAX_SPEED_FACTOR * self.p.maxspeed;
        if maxscaledspeed <= 0.0 {
            return;
        }
        let spd = length(self.p.velocity);
        if spd <= maxscaledspeed {
            return;
        }
        let fraction = (maxscaledspeed / spd) * 0.65;
        self.p.velocity = scale(self.p.velocity, fraction);
    }

    /// `PM_Jump` (0xf9944): 35-unit jump, super jump with physinfo `mj`, stunt lockout.
    fn jump(&mut self) {
        use crate::ts::stunt;
        if self.p.dead == 0 {
            // A stunt blocks jumping without latching IN_JUMP, so a held jump fires as soon as
            // the stunt ends.
            if self.p.iuser4 & ts::NO_JUMP_MASK != 0 {
                return;
            }
            if self.p.waterjumptime != 0.0 {
                self.p.waterjumptime -= self.p.cmd.msec as f32;
                if self.p.waterjumptime < 0.0 {
                    self.p.waterjumptime = 0.0;
                }
                return;
            }
            if self.p.waterlevel >= 2 {
                self.p.onground = -1;
                self.p.velocity[2] = match self.p.watertype {
                    CONTENTS_WATER => 100.0,
                    CONTENTS_SLIME => 80.0,
                    _ => 50.0,
                };
                if self.p.flSwimTime <= 0.0 {
                    self.p.flSwimTime = 1000.0;
                    self.play_wade_sound();
                }
                return;
            }
            if self.p.onground != -1 {
                if self.p.oldbuttons & IN_JUMP as i32 != 0 {
                    return; // don't pogo stick
                }
                self.p.onground = -1;
                self.prevent_mega_bunny_jumping();
                let step = sound::map_texture_type_step_type(self.p.chtexturetype as u8);
                self.play_step_sound(step, 1.0);

                let can_high_jump = self.info_int(c"mj") == 1
                    && (self.p.bInDuck != 0 || self.p.flags & FL_DUCKING_ != 0)
                    && self.p.cmd.buttons & IN_DUCK_ != 0
                    && self.p.flDuckTime > 0.0;
                if can_high_jump {
                    self.p.velocity[2] = ts::HIGH_JUMP_VELOCITY;
                    self.p.iuser4 |= stunt::SUPERJUMP_START;
                } else {
                    self.p.velocity[2] = ts::JUMP_VELOCITY;
                }
                self.fixup_gravity_velocity();
            }
        }
        self.p.oldbuttons |= IN_JUMP as i32;
    }

    /// `PM_CheckFalling` (0x1003d0): TS safe fall speed 680.
    fn check_falling(&mut self) {
        if self.p.onground != -1 && self.p.dead == 0 && self.p.flFallVelocity >= PLAYER_FALL_PUNCH_THRESHHOLD {
            let mut fvol = 0.5f32;
            if self.p.waterlevel > 0 {
            } else if self.p.flFallVelocity > PLAYER_MAX_SAFE_FALL_SPEED {
                self.play_sound(CHAN_VOICE, c"player/pl_fallpain3.wav", 1.0, ATTN_NORM);
                fvol = 1.0;
            } else if self.p.flFallVelocity > PLAYER_MAX_SAFE_FALL_SPEED / 2.0 {
                if self.info_int(c"tfc") == 1 {
                    self.play_sound(CHAN_VOICE, c"player/pl_fallpain3.wav", 1.0, ATTN_NORM);
                }
                fvol = 0.85;
            } else if self.p.flFallVelocity < PLAYER_MIN_BOUNCE_SPEED {
                fvol = 0.0;
            }
            if fvol > 0.0 {
                self.p.flTimeStepSound = 0;
                self.update_step_sound();
                let step = sound::map_texture_type_step_type(self.p.chtexturetype as u8);
                self.play_step_sound(step, fvol);
                self.p.punchangle[2] = self.p.flFallVelocity * 0.013;
                if self.p.punchangle[0] > 8.0 {
                    self.p.punchangle[0] = 8.0;
                }
            }
        }
        if self.p.onground != -1 {
            self.p.flFallVelocity = 0.0;
        }
    }

    /// `PM_CatagorizeTextureType`: texture under the player, 64 units down.
    pub(crate) fn categorize_texture_type(&mut self) {
        let start = self.p.origin;
        let end = [start[0], start[1], start[2] - 64.0];
        self.p.sztexturename[0] = 0;
        self.p.chtexturetype = textures::CHAR_TEX_CONCRETE as c_char;
        let Some(f) = self.p.PM_TraceTexture else { return };
        let (mut s, mut e) = (start, end);
        let name = unsafe { f(self.p.onground, s.as_mut_ptr(), e.as_mut_ptr()) };
        if name.is_null() {
            return;
        }
        let mut name = unsafe { CStr::from_ptr(name) }.to_bytes();
        if matches!(name.first(), Some(b'-' | b'+')) {
            name = name.get(2..).unwrap_or(&[]);
        }
        if matches!(name.first(), Some(b'{' | b'!' | b'~' | b' ')) {
            name = &name[1..];
        }
        let n = name.len().min(self.p.sztexturename.len() - 1);
        for (d, s) in self.p.sztexturename.iter_mut().zip(&name[..n]) {
            *d = *s as c_char;
        }
        self.p.sztexturename[n] = 0;
        self.p.sztexturename[15] = 0;
        let len = self.p.sztexturename.iter().position(|&c| c == 0).unwrap_or(15);
        let bytes: Vec<u8> = self.p.sztexturename[..len].iter().map(|&c| c as u8).collect();
        self.p.chtexturetype = textures::find(&bytes) as c_char;
    }
}

// ---- free helpers ---------------------------------------------------------------------------------

/// `PM_ClipVelocity`: slide off the impacting plane. Returns the blocked flags.
fn clip_velocity(input: Vec3, normal: Vec3, out: &mut Vec3, overbounce: f32) -> i32 {
    let angle = normal[2];
    let mut blocked = 0;
    if angle > 0.0 {
        blocked |= 1;
    }
    if angle == 0.0 {
        blocked |= 2;
    }
    let backoff = dot(input, normal) * overbounce;
    for i in 0..3 {
        let change = normal[i] * backoff;
        out[i] = input[i] - change;
        if out[i] > -STOP_EPSILON && out[i] < STOP_EPSILON {
            out[i] = 0.0;
        }
    }
    blocked
}

fn spline_fraction(value: f32, scale: f32) -> f32 {
    let value = scale * value;
    let value_squared = value * value;
    3.0 * value_squared - 2.0 * value_squared * value
}

fn calc_roll(angles: Vec3, velocity: Vec3, rollangle: f32, rollspeed: f32) -> f32 {
    let (_, right, _) = angle_vectors(angles);
    let side = dot(velocity, right);
    let sign = if side < 0.0 { -1.0 } else { 1.0 };
    let side = side.abs();
    let value = rollangle;
    let side = if side < rollspeed { side * value / rollspeed } else { value };
    side * sign
}

/// `PM_GetAccelBySpeed` (0xfdcc4): ground acceleration factor from the slot-weight maxspeed,
/// 0.6 at 210 u/s up to 1.1 at 330 u/s.
pub fn accel_by_speed(maxspeed: f32) -> f32 {
    if maxspeed >= 330.0 { 1.1 } else { (maxspeed - 210.0) * 0.008_333_334 * 0.5 + 0.6 }
}
