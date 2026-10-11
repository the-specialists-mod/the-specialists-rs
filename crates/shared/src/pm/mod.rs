//! Player movement, shared by the server (`pfnPM_Move`) and client prediction (`HUD_PlayerMove`).
//!
//! A port of TS 3.0's `pm_shared.c` (server `ts_i386.so` 0xf8f78-0x1012dd; `client.dll`
//! 0x10029bb0-0x1002ea30 and 0x10052fd0-0x10054500, the same constants and logic): the Half-Life
//! SDK movement code with the TS changes from decomp `docs/server/player-movement.md` §1.
//! Function names follow the C code. Addresses without a prefix are `ts_i386.so`'s.
//!
//! The arithmetic is each TS build's, bit for bit ([`fpu`]): `Pm<Gcc>` is `ts_i386.so`, `Pm<Msvc>`
//! is `client.dll` and `mp.dll`. The caller picks the build it stands in for ([`Arith`]).
//!
//! * `ts`: the TS stunt hooks (`TSHandle`, `TSDive`, `PseudoClimb`, ...).
//! * `textures`: `sound/materials.txt`.
//! * `sound`: footsteps (`PM_UpdateStepSound`, `PM_PlayStepSound`).
//!
//! Global state (`OldrotArray`, the stuck tables, the texture table) is per library, as in C.

mod ext;
mod fpu;
mod math;
mod sound;
pub mod textures;
pub mod ts;

// The tests stand in for the engine with `extern "C"` callbacks that read raw pointers.
#[cfg(test)]
#[allow(unsafe_code)]
mod tests;

pub use fpu::Arith;
pub use math::Vec3;
use math::*;
/// The two builds' arithmetic, for the weapon code both sides share (`weapons::spread`).
pub(crate) use math::{Fpu, Gcc, Msvc, Num};
/// `ts_i386.so`'s exported mathlib helpers; `server-dll` exports them under their C names.
pub use math::{anglemod, length, q_log2};
use std::marker::PhantomData;

// Spectator modes (`iuser1`) in TS's numbering.
use crate::ts::obs::{CHASE_LOCKED as OBS_CHASE_LOCKED, IN_EYE as OBS_IN_EYE, LOCKED_VIEW as OBS_LOCKED_VIEW, ROAMING as OBS_ROAMING};
use crate::{VEC_DUCK_HULL_MAX, VEC_DUCK_HULL_MIN, VEC_HULL_MAX, VEC_HULL_MIN};
use goldsrc::pmove::PlayerMoveExt;
use goldsrc_sys::*;
use std::ffi::{CStr, c_char};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering::Relaxed};

// ---- constants (pm_shared.c / TS) --------------------------------------------------------------

const MAX_CLIP_PLANES: usize = 5;
const STOP_EPSILON: f64 = 0.1;
const MAX_PHYSENTS: i32 = 600;
const MAX_CLIENTS: usize = 32;
const PM_DEAD_VIEWHEIGHT: f32 = -8.0;
/// TS view heights: standing 28, ducked 16 (SDK 12).
pub const VIEW_STAND: f32 = 28.0;
pub const VIEW_DUCK: f32 = 16.0;
/// TS ladder speed: always 200 (no maxspeed clamp, no duck multiplier).
const MAX_CLIMB_SPEED: f32 = 200.0;
const PLAYER_FALL_PUNCH_THRESHHOLD: f32 = 350.0;
/// TS: 680 (SDK 580).
pub const PLAYER_MAX_SAFE_FALL_SPEED: f32 = 680.0;
const PLAYER_MIN_BOUNCE_SPEED: f32 = 200.0;
const BUNNYJUMP_MAX_SPEED_FACTOR: f32 = 1.7;
const PM_CHECKSTUCK_MINTIME: f64 = 0.05;
const STUCK_MOVEUP: i32 = 1;
const WJ_HEIGHT: f32 = 8.0;
const ATTN_NORM: f32 = 0.8;
const PITCH_NORM: i32 = 100;

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
thread_local! {
    /// What `PM_SpectatorMove` (0xfef3c) finds in its camera-angle slots in the locked view
    /// (`iuser1 == 4`), where it never writes them before `GetChaseOrigin` reads them and they are
    /// copied to `pmove->angles`. In `ts_i386.so` the three floats (`0x30..0x38(%esp)`) sit exactly
    /// where `PM_CheckParamters` (called from the same `PM_PlayerMove` frame, which calls nothing
    /// that reaches that deep in between) pushed its last outgoing arguments: `PM_CalcRoll`'s
    /// `(v_angle, pmove->velocity, rollangle)` when it rolls the view, otherwise only the first
    /// slot is rewritten, by `PM_DropPunchAngle(pmove->punchangle)`. Two of them are addresses, so
    /// the port stores the low 32 bits of the matching addresses; the roll slot is
    /// `movevars->rollangle` exactly. Between moves the engine can reuse that stack, which isn't
    /// reproduced: the slots keep the last values written here. Per thread, like a stack.
    static STALE_ARGS: std::cell::Cell<[u32; 3]> = const { std::cell::Cell::new([0; 3]) };
}

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
    let Some(bytes) = pm.load_file(c"sound/materials.txt") else { return };
    let n = textures::load(&bytes);
    pm.dprint(&format!("PM_Init: {n} material types from sound/materials.txt\n"));
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

/// Quake's `AngleVectors` in the arithmetic of the given TS build (the server's stunt code passes
/// its `ts_pm_arith` selection, as for [`player_move`]).
pub fn angle_vectors(arith: Arith, angles: Vec3) -> (Vec3, Vec3, Vec3) {
    match arith {
        Arith::Gcc => Gcc::angle_vectors(angles),
        Arith::Msvc => Msvc::angle_vectors(angles),
    }
}

/// Runs one usercmd of movement (`PM_Move`: server 0x101188, client 0x1002e990) with the
/// arithmetic of a TS build: the server's `ts_pm_arith` selection (default
/// [`Arith::SERVER_DEFAULT`], `mp.dll`'s) for `pfnPM_Move`, [`Arith::CLIENT`] for
/// `HUD_PlayerMove`. `pmove->server` is set by the engine.
pub fn player_move(p: &mut playermove_t, arith: Arith) {
    player_move_with(p, arith, None);
}

/// A side's own spectator move, run in place of the shared [`PM_SpectatorMove`] step; it returns
/// false to fall back to the shared one. The client passes TS's client-modified
/// `PM_SpectatorMove` (`0x1002c790`).
///
/// [`PM_SpectatorMove`]: Pm::spectator_move
pub type SpectatorMove = fn(&mut playermove_t) -> bool;

/// `GetChaseOrigin` (server 0xfee5c, client 0x1002c6c0): 90 units behind `physents[target]`'s
/// eyes (z + 35) along `angles`, pulled 4 units off any wall in between.
pub fn chase_origin(p: &playermove_t, arith: Arith, angles: Vec3, target: usize) -> Vec3 {
    match arith {
        Arith::Gcc => chase_origin_in::<Gcc>(p, angles, target),
        Arith::Msvc => chase_origin_in::<Msvc>(p, angles, target),
    }
}

fn chase_origin_in<F: Fpu>(p: &playermove_t, angles: Vec3, target: usize) -> Vec3 {
    let (forward, _, _) = F::angle_vectors(angles);
    let pe = &p.physents[target];
    let start = [pe.origin[0], pe.origin[1], pe.origin[2] + 35.0];
    let back = F::ma(ZERO, 90.0, forward);
    let end = sub(start, back);
    let Some(tr) = p.trace_line(start, end, 0, 2, target as i32) else { return end };
    F::ma(tr.endpos, 4.0, tr.plane.normal)
}

/// [`player_move`] with an optional [`SpectatorMove`] override.
pub fn player_move_with(p: &mut playermove_t, arith: Arith, spectator: Option<SpectatorMove>) {
    let p = match arith {
        Arith::Gcc => run::<Gcc>(p, spectator),
        Arith::Msvc => run::<Msvc>(p, spectator),
    };
    if p.onground == -1 {
        p.flags &= !FL_ONGROUND_;
    } else {
        p.flags |= FL_ONGROUND_;
    }
    if p.multiplayer == 0 && p.movetype == MT_WALK {
        p.friction = 1.0;
    }
}

fn run<F: Fpu>(p: &mut playermove_t, spectator: Option<SpectatorMove>) -> &mut playermove_t {
    let mut pm = Pm::<F> { p, onladder: false, spectator, fpu: PhantomData };
    pm.player_move();
    pm.p
}

// ---- the movement state machine -----------------------------------------------------------------

/// One usercmd of movement in build `F`'s arithmetic.
pub(crate) struct Pm<'a, F: Fpu> {
    pub(crate) p: &'a mut playermove_t,
    /// `g_onladder`.
    pub(crate) onladder: bool,
    /// The caller's spectator move, if any ([`player_move_with`]).
    pub(crate) spectator: Option<SpectatorMove>,
    pub(crate) fpu: PhantomData<F>,
}

impl<F: Fpu> Pm<'_, F> {
    // ---- engine callbacks ------------------------------------------------------------------------

    fn mv(&self) -> &movevars_s {
        self.p.movevars()
    }

    pub(crate) fn trace(&self, start: Vec3, end: Vec3) -> pmtrace_t {
        self.p.player_trace(start, end)
    }

    pub(crate) fn point_contents(&self, p: Vec3, truecont: Option<&mut i32>) -> i32 {
        self.p.point_contents(p, truecont)
    }

    /// Without the engine callback, the spot counts as free.
    pub(crate) fn test_player_position(&self, pos: Vec3, tr: Option<&mut pmtrace_t>) -> i32 {
        self.p.test_player_position(pos, tr)
    }

    pub(crate) fn play_sound(&self, channel: u32, sample: &CStr, volume: f32, attn: f32) {
        self.p.play_sound(channel as i32, sample, volume, attn, 0, PITCH_NORM);
    }

    pub(crate) fn random_long(&self, lo: i32, hi: i32) -> i32 {
        self.p.random_long(lo, hi)
    }

    /// `atoi(PM_Info_ValueForKey(physinfo, key))`.
    pub(crate) fn info_int(&self, key: &CStr) -> i32 {
        let v = self.p.physinfo_value(key);
        // atoi: optional whitespace, sign, digits.
        let s = v.trim_start();
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
        // TS: frametime is scaled by the per-player time scale and frozen below 0.1 (or NaN).
        self.p.frametime = (F::xd(0.001) * F::xi(self.p.cmd.msec as i32)).f();
        self.p.frametime = if below87(self.p.fuser1, crate::ts::TIME_SCALE_FREEZE) { 0.0 } else { self.p.frametime * self.p.fuser1 };
        // Duck, step, swim and water-jump timers still run on the unscaled msec.
        self.reduce_timers();
        let (f, r, u) = F::angle_vectors(self.p.angles);
        (self.p.forward, self.p.right, self.p.up) = (f, r, u);

        if self.p.spectator != 0 || self.p.iuser1 > 0 {
            if !self.spectator.is_some_and(|f| f(self.p)) {
                self.spectator_move();
            }
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
            mt => {
                let server = self.p.server;
                self.p.dprint(&format!("Bogus pmove player movetype {mt} on ({server}) 0=cl 1=sv\n"));
            }
        }
    }

    fn walk_physics(&mut self, on_ladder: bool) {
        if !self.in_water() {
            self.add_correct_gravity();
        }
        // Waterjumping: just move.
        if !eq87(self.p.waterjumptime, 0.0) {
            self.water_jump();
            self.fly_move();
            self.check_water();
            return;
        }
        if self.p.waterlevel >= 2 {
            if self.p.waterlevel == 2 {
                self.check_water_jump();
            }
            // Falling: cancel the water jump. MSVC lets a NaN velocity in and a NaN time out.
            let cancel = if F::MSVC {
                below87(self.p.velocity[2], 0.0) && !eq87(self.p.waterjumptime, 0.0)
            } else {
                self.p.velocity[2] < 0.0 && self.p.waterjumptime != 0.0
            };
            if cancel {
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
        let (fm, sm, um) = (F::x(c.forwardmove), F::x(c.sidemove), F::x(c.upmove));
        let spd = (fm * fm + sm * sm + um * um).sqrt().f();
        let maxspeed = self.p.clientmaxspeed;
        // MSVC tests `clientmaxspeed < maxspeed` on C0, so a NaN maxspeed is replaced too.
        let lower = if F::MSVC { below87(maxspeed, self.p.maxspeed) } else { self.p.maxspeed > maxspeed };
        if !eq87(maxspeed, 0.0) && lower {
            self.p.maxspeed = maxspeed;
        }
        // TS: no IN_USE slowdown, and the command is clamped to maxspeed only for spectators
        // (walk/air move clamp the wish speed anyway). The ratio isn't rounded.
        if !eq87(spd, 0.0) && spd > self.p.maxspeed {
            let ratio = F::x(self.p.maxspeed) / F::x(spd);
            if self.p.iuser1 > 0 {
                let c = &mut self.p.cmd;
                c.forwardmove = (ratio * F::x(c.forwardmove)).f();
                c.sidemove = (ratio * F::x(c.sidemove)).f();
                c.upmove = (ratio * F::x(c.upmove)).f();
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
            // A NaN vertical speed rises in GCC's compare and falls in MSVC's.
            let rising = if F::MSVC { !below87(self.p.velocity[2], 0.0) } else { not_above87(0.0, self.p.velocity[2]) };
            self.p.gravity = if self.p.cmd.buttons & IN_JUMP_ == 0 {
                1.0
            } else if rising {
                0.3
            } else {
                0.75
            };
        }

        // The outgoing argument of `PM_DropPunchAngle(pmove->punchangle)`; see `StaleArgs`.
        let mut stale = STALE_ARGS.get();
        stale[0] = std::ptr::addr_of!(self.p.punchangle) as usize as u32;
        self.drop_punch_angle();

        if self.p.dead == 0 && self.p.iuser4 & stunt::DEAD_LOCK == 0 {
            let v_angle = add(self.p.cmd.viewangles, self.p.punchangle);
            let rollangle = self.mv().rollangle;
            let rollspeed = self.mv().rollspeed;
            // `PM_CalcRoll(v_angle, pmove->velocity, rollangle, rollspeed)`'s arguments.
            stale = [std::ptr::addr_of!(v_angle) as usize as u32, std::ptr::addr_of!(self.p.velocity) as usize as u32, rollangle.to_bits()];
            self.p.angles[2] = (calc_roll::<F>(v_angle, self.p.velocity, rollangle, rollspeed) * F::x(4.0)).f();
            self.p.angles[0] = v_angle[0];
            self.p.angles[1] = v_angle[1];
            if let Some(i) = self.player_slot() {
                OLDROT[i].store(self.p.angles[1].to_bits(), Relaxed);
            }
        } else if let Some(i) = self.player_slot() {
            // TS: only the yaw is restored (SDK: angles = oldangles).
            self.p.angles[1] = f32::from_bits(OLDROT[i].load(Relaxed));
        }
        STALE_ARGS.set(stale);

        if self.p.dead != 0 {
            self.p.view_ofs[2] = PM_DEAD_VIEWHEIGHT;
        }
        if self.p.angles[1] > 180.0 {
            self.p.angles[1] -= 360.0;
        }
    }

    /// `PM_DropPunchAngle` (0x100760, client 0x1002e1c0). TS scales by `fuser1` on top of the
    /// already-scaled frametime, and zeroes the punch while dead. MSVC multiplies by `fuser1`
    /// first and clamps the unrounded length (a NaN or a -0 becomes +0).
    fn drop_punch_angle(&mut self) {
        let mut punch = self.p.punchangle;
        let len = if self.p.dead == 0 {
            let len = F::normalize(&mut punch);
            if F::MSVC {
                let d = len - (len * F::xd(0.5) + F::xd(10.0)) * F::x(self.p.fuser1) * F::x(self.p.frametime);
                if d > F::xd(0.0) { d.f() } else { 0.0 }
            } else {
                let len = (len - (len * F::xd(0.5) + F::xd(10.0)) * F::x(self.p.frametime) * F::x(self.p.fuser1)).f();
                if 0.0 > len { 0.0 } else { len }
            }
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

    /// `PM_CheckVelocity` (0xfd0b8). `IS_NAN` tests the exponent bits only, so an infinity
    /// counts as a NaN.
    pub(crate) fn check_velocity(&mut self) {
        let is_nan = |v: f32| v.to_bits() & 0x7f80_0000 == 0x7f80_0000;
        let max = self.mv().maxvelocity;
        for i in 0..3 {
            if is_nan(self.p.velocity[i]) {
                self.p.con_printf(&format!("PM  Got a NaN velocity {i}\n"));
                self.p.velocity[i] = 0.0;
            }
            if is_nan(self.p.origin[i]) {
                self.p.con_printf(&format!("PM  Got a NaN origin on {i}\n"));
                self.p.origin[i] = 0.0;
            }
            // MSVC tests "too low" on C0, so a NaN `maxvelocity` lands there.
            let too_low = |v: f32| if F::MSVC { below87(v, -max) } else { v < -max };
            if self.p.velocity[i] > max {
                self.p.dprint(&format!("PM  Got a velocity too high on {i}\n"));
                self.p.velocity[i] = max;
            } else if too_low(self.p.velocity[i]) {
                self.p.dprint(&format!("PM  Got a velocity too low on {i}\n"));
                self.p.velocity[i] = -max;
            }
        }
    }

    /// `pmove->gravity`, or 1 when it is 0.
    fn ent_gravity(&self) -> f32 {
        if eq87(self.p.gravity, 0.0) { 1.0 } else { self.p.gravity }
    }

    /// `PM_AddCorrectGravity` (0xfd304).
    fn add_correct_gravity(&mut self) {
        if !eq87(self.p.waterjumptime, 0.0) {
            return;
        }
        let g = F::x(self.ent_gravity());
        let (ft, vz) = (F::x(self.p.frametime), F::x(self.p.velocity[2]));
        self.p.velocity[2] = (vz - g * F::x(self.mv().gravity) * F::xd(0.5) * ft).f();
        self.p.velocity[2] = (F::x(self.p.basevelocity[2]) * ft + F::x(self.p.velocity[2])).f();
        self.p.basevelocity[2] = 0.0;
        self.check_velocity();
    }

    /// `PM_FixupGravityVelocity` (0xfd390).
    pub(crate) fn fixup_gravity_velocity(&mut self) {
        if !eq87(self.p.waterjumptime, 0.0) {
            return;
        }
        let g = F::x(self.ent_gravity());
        self.p.velocity[2] = (F::x(self.p.velocity[2]) - g * F::x(self.mv().gravity) * F::x(self.p.frametime) * F::xd(0.5)).f();
        self.check_velocity();
    }

    /// `PM_AddGravity` (0xffbb0).
    fn add_gravity(&mut self) {
        let g = F::x(self.ent_gravity());
        let ft = F::x(self.p.frametime);
        self.p.velocity[2] = (F::x(self.p.velocity[2]) - g * F::x(self.mv().gravity) * ft).f();
        self.p.velocity[2] = (F::x(self.p.basevelocity[2]) * ft + F::x(self.p.velocity[2])).f();
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

    /// `PM_FlyMove` (0xfd408).
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
            if eq87(self.p.velocity[0], 0.0) && eq87(self.p.velocity[1], 0.0) && eq87(self.p.velocity[2], 0.0) {
                break;
            }
            let tl = F::x(time_left);
            let end = [
                (tl * F::x(self.p.velocity[0]) + F::x(self.p.origin[0])).f(),
                (tl * F::x(self.p.velocity[1]) + F::x(self.p.origin[1])).f(),
                (tl * F::x(self.p.velocity[2]) + F::x(self.p.origin[2])).f(),
            ];
            let trace = self.trace(self.p.origin, end);
            all_fraction += trace.fraction;
            if trace.allsolid != 0 {
                self.p.velocity = ZERO;
                return 4;
            }
            // GCC takes this on a NaN fraction; MSVC's compare is ordered.
            let moved = if F::MSVC { trace.fraction > 0.0 } else { below87(0.0, trace.fraction) };
            if moved {
                self.p.origin = trace.endpos;
                original_velocity = self.p.velocity;
                numplanes = 0;
            }
            if eq87(trace.fraction, 1.0) {
                break;
            }
            self.add_to_touched(trace, self.p.velocity);

            if F::x(trace.plane.normal[2]) > F::xd(0.7) {
                blocked |= 1;
            }
            if eq87(trace.plane.normal[2], 0.0) {
                blocked |= 2;
            }
            time_left = (F::x(time_left) - F::x(time_left) * F::x(trace.fraction)).f();

            if numplanes >= MAX_CLIP_PLANES {
                self.p.velocity = ZERO;
                break;
            }
            planes[numplanes] = trace.plane.normal;
            numplanes += 1;

            // The SDK's `numplanes == 1 &&` isn't in TS's build: every plane so far is reflected.
            // MSVC compares the friction's bits with 1.0f as an integer.
            let slippery = if F::MSVC { self.p.friction.to_bits() != 1.0f32.to_bits() } else { !eq87(self.p.friction, 1.0) };
            if self.p.movetype == MT_WALK && (self.p.onground == -1 || slippery) {
                for plane in planes.iter().take(numplanes) {
                    if F::x(plane[2]) > F::xd(0.7) {
                        clip_velocity::<F>(original_velocity, *plane, &mut new_velocity, 1.0);
                        original_velocity = new_velocity;
                    } else {
                        // TS: a fixed 0.01 where the SDK reads `movevars->bounce`.
                        let ob = ((F::x(1.0) - F::x(self.p.friction)) * F::xd(0.01) + F::xd(1.0)).f();
                        clip_velocity::<F>(original_velocity, *plane, &mut new_velocity, ob);
                    }
                }
                self.p.velocity = new_velocity;
                original_velocity = new_velocity;
            } else {
                let mut i = 0;
                while i < numplanes {
                    clip_velocity::<F>(original_velocity, planes[i], &mut self.p.velocity, 1.0);
                    let mut j = 0;
                    while j < numplanes {
                        let order = if F::MSVC { [2, 0, 1] } else { [0, 1, 2] };
                        if j != i && below87(F::dot_in(self.p.velocity, planes[j], order), F::ZERO) {
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
                    let dir = F::cross(planes[0], planes[1]);
                    let d = F::dot_in(dir, self.p.velocity, F::ZYX).f();
                    self.p.velocity = scale(dir, d);
                }
                if not_above87(F::dot_in(self.p.velocity, primal_velocity, F::ZYX), F::ZERO) {
                    self.p.velocity = ZERO;
                    break;
                }
            }
        }
        if eq87(all_fraction, 0.0) {
            self.p.velocity = ZERO;
        }
        blocked
    }

    // ---- acceleration and friction ------------------------------------------------------------------

    /// `PM_Accelerate` (0xfdd08, client 0x1002b830) with TS's `PM_GetAccelBySpeed` factor. The
    /// add speed is tested unrounded, then stored. GCC stores the acceleration before and after
    /// the factor; MSVC multiplies in the other order, keeps the product on the stack (the
    /// factor multiplies a stored copy) and steps the velocity with the unrounded result.
    fn accelerate(&mut self, wishdir: Vec3, wishspeed: f32, accel: f32) {
        if self.p.dead != 0 || !eq87(self.p.waterjumptime, 0.0) {
            return;
        }
        let addspeed = F::x(wishspeed) - F::dot_in(self.p.velocity, wishdir, F::ZYX);
        if not_above87(addspeed, F::ZERO) {
            return;
        }
        let addspeed = addspeed.f();
        let factor = self.p.iuser4 & crate::ts::stunt::PRONE == 0 && self.p.flags & FL_DUCKING_ == 0;
        if F::MSVC {
            let prod = F::x(self.p.friction) * F::x(self.p.frametime) * F::x(wishspeed) * F::x(accel);
            let mut accelspeed = if factor { accel_by_speed_x::<F>(self.p.maxspeed) * F::x(prod.f()) } else { prod };
            if accelspeed > F::x(addspeed) {
                accelspeed = F::x(addspeed);
            }
            self.p.velocity = F::ma_x(self.p.velocity, accelspeed, wishdir);
            return;
        }
        let mut accelspeed = (F::x(accel) * F::x(self.p.frametime) * F::x(wishspeed) * F::x(self.p.friction)).f();
        if factor {
            accelspeed = (accel_by_speed_x::<F>(self.p.maxspeed) * F::x(accelspeed)).f();
        }
        if accelspeed > addspeed {
            accelspeed = addspeed;
        }
        self.p.velocity = F::ma(self.p.velocity, accelspeed, wishdir);
    }

    /// `PM_AirAccelerate` (0xfdfdc, client 0x1002bab0): the acceleration isn't rounded. GCC
    /// clamps it to the unrounded add speed, MSVC to the stored one.
    fn air_accelerate(&mut self, wishdir: Vec3, wishspeed: f32, accel: f32) {
        if self.p.dead != 0 || !eq87(self.p.waterjumptime, 0.0) {
            return;
        }
        let wishspd = if wishspeed > 30.0 { 30.0 } else { wishspeed };
        let addspeed = F::x(wishspd) - F::dot_in(self.p.velocity, wishdir, F::ZYX);
        if not_above87(addspeed, F::ZERO) {
            return;
        }
        let (mut accelspeed, addspeed) = if F::MSVC {
            (F::x(self.p.friction) * F::x(self.p.frametime) * F::x(wishspeed) * F::x(accel), F::x(addspeed.f()))
        } else {
            (F::x(wishspeed) * F::x(accel) * F::x(self.p.frametime) * F::x(self.p.friction), addspeed)
        };
        if accelspeed > addspeed {
            accelspeed = addspeed;
        }
        self.p.velocity = F::ma_x(self.p.velocity, accelspeed, wishdir);
    }

    /// `PM_Friction` (0xfde08, client 0x1002b910): SDK edge friction, TS extra factor 0.75.
    /// GCC stores the drop; MSVC keeps it on the stack, factored as
    /// `(friction * pmove->friction * 0.75) * (control * frametime)`, and stores the scale.
    fn friction(&mut self) {
        if !eq87(self.p.waterjumptime, 0.0) {
            return;
        }
        let vel = self.p.velocity;
        let speed = (F::x(vel[0]) * F::x(vel[0]) + F::x(vel[1]) * F::x(vel[1]) + F::x(vel[2]) * F::x(vel[2])).sqrt().f();
        if below87(speed, 0.1) {
            return;
        }
        let sp = F::x(speed);
        let mut drop = 0.0f32;
        let mut drop_x = F::ZERO;
        if self.p.onground != -1 {
            let z = F::x(self.p.origin[2]) + F::x(self.hull_min(self.p.usehull)[2]);
            let start = [
                (F::x(vel[0]) / sp * F::x(16.0) + F::x(self.p.origin[0])).f(),
                (F::x(vel[1]) / sp * F::x(16.0) + F::x(self.p.origin[1])).f(),
                z.f(),
            ];
            let stop = [start[0], start[1], (z - F::x(34.0)).f()];
            let trace = self.trace(start, stop);
            let friction =
                if eq87(trace.fraction, 1.0) { F::x(self.mv().friction) * F::x(self.mv().edgefriction) } else { F::x(self.mv().friction) };
            let control = if speed >= self.mv().stopspeed { speed } else { self.mv().stopspeed };
            if F::MSVC {
                drop_x = friction * F::x(self.p.friction) * F::x(0.75) * (F::x(control) * F::x(self.p.frametime));
            } else {
                drop = (friction * F::x(self.p.friction) * F::x(0.75) * F::x(control) * F::x(self.p.frametime) + F::x(drop)).f();
            }
        }
        if F::MSVC {
            let newspeed = sp - drop_x;
            let newspeed = if below87(newspeed, F::ZERO) { F::ZERO } else { newspeed } / sp;
            let ns = F::x(newspeed.f());
            self.p.velocity = [(newspeed * F::x(vel[0])).f(), (ns * F::x(vel[1])).f(), (ns * F::x(vel[2])).f()];
            return;
        }
        let newspeed = sp - F::x(drop);
        let newspeed = if F::ZERO > newspeed { F::ZERO } else { newspeed } / sp;
        self.p.velocity = [(newspeed * F::x(vel[0])).f(), (newspeed * F::x(vel[1])).f(), (newspeed * F::x(vel[2])).f()];
    }

    /// Horizontal wish velocity from `fmove`/`smove` with flattened forward/right (`PM_WalkMove`,
    /// `PM_AirMove`). TS also scales `wishvel` down to maxspeed, but nothing reads it after.
    fn flat_wish(&mut self, fmove: f32, smove: f32) -> (Vec3, f32) {
        self.p.forward[2] = 0.0;
        self.p.right[2] = 0.0;
        F::normalize(&mut self.p.forward);
        F::normalize(&mut self.p.right);
        let (f, s) = (F::x(fmove), F::x(smove));
        let wishvel = [
            (f * F::x(self.p.forward[0]) + s * F::x(self.p.right[0])).f(),
            (f * F::x(self.p.forward[1]) + s * F::x(self.p.right[1])).f(),
            0.0,
        ];
        let mut wishdir = wishvel;
        let mut wishspeed = F::normalize(&mut wishdir).f();
        if wishspeed > self.p.maxspeed {
            wishspeed = self.p.maxspeed;
        }
        (wishdir, wishspeed)
    }

    /// `PM_WalkMove` (0xfa26c). TS: backpedalling at 80 % (a `double` 0.8).
    fn walk_move(&mut self) {
        let mut fmove = self.p.cmd.forwardmove;
        if below87(fmove, 0.0) {
            fmove = (F::xd(0.8) * F::x(fmove)).f();
        }
        let smove = self.p.cmd.sidemove;
        let (wishdir, wishspeed) = self.flat_wish(fmove, smove);

        self.p.velocity[2] = 0.0;
        let accel = self.mv().accelerate;
        self.accelerate(wishdir, wishspeed, accel);
        self.p.velocity[2] = 0.0;
        self.p.velocity = add(self.p.velocity, self.p.basevelocity);

        // MSVC compares the unrounded root (GCC's `Length` has stored it).
        if below87(F::length(self.p.velocity), F::x(1.0)) {
            self.p.velocity = ZERO;
            return;
        }

        let oldonground = self.p.onground;
        let ft = F::x(self.p.frametime);
        let dest = [
            (F::x(self.p.velocity[0]) * ft + F::x(self.p.origin[0])).f(),
            (F::x(self.p.velocity[1]) * ft + F::x(self.p.origin[1])).f(),
            self.p.origin[2],
        ];
        let trace = self.trace(self.p.origin, dest);
        if eq87(trace.fraction, 1.0) {
            self.p.origin = trace.endpos;
            return;
        }
        if oldonground == -1 && self.p.waterlevel == 0 {
            return;
        }
        if !eq87(self.p.waterjumptime, 0.0) {
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
        let usedown = if below87(F::x(trace.plane.normal[2]), F::xd(0.7)) {
            true
        } else {
            if trace.startsolid == 0 && trace.allsolid == 0 {
                self.p.origin = trace.endpos;
            }
            // The SDK copies the result into pmove->up (sic).
            self.p.up = self.p.origin;
            let dist2 = |p: Vec3| {
                let (dx, dy) = (F::x(p[0]) - F::x(original[0]), F::x(p[1]) - F::x(original[1]));
                dx * dx + dy * dy
            };
            // On a NaN, GCC steps up and MSVC keeps the down move.
            if F::MSVC { below87(dist2(self.p.up), dist2(down)) } else { dist2(down) > dist2(self.p.up) }
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
        let o = self.p.origin;
        let mut point = [
            ((F::x(mins[0]) + F::x(maxs[0])) * F::xd(0.5) + F::x(o[0])).f(),
            ((F::x(mins[1]) + F::x(maxs[1])) * F::xd(0.5) + F::x(o[1])).f(),
            (F::x(o[2]) + F::x(mins[2]) + F::x(1.0)).f(),
        ];
        self.p.waterlevel = 0;
        self.p.watertype = CONTENTS_EMPTY;
        let is_liquid = |c: i32| c <= CONTENTS_WATER && c > CONTENTS_TRANSLUCENT;
        let mut truecont = 0;
        let cont = self.point_contents(point, Some(&mut truecont));
        if is_liquid(cont) {
            self.p.watertype = cont;
            self.p.waterlevel = 1;
            // MSVC adds the half height without storing it.
            let heightover2 = (F::x(mins[2]) + F::x(maxs[2])) * F::xd(0.5);
            point[2] = if F::MSVC { (heightover2 + F::x(self.p.origin[2])).f() } else { self.p.origin[2] + heightover2.f() };
            if is_liquid(self.point_contents(point, None)) {
                self.p.waterlevel = 2;
                point[2] = self.p.origin[2] + self.p.view_ofs[2];
                if is_liquid(self.point_contents(point, None)) {
                    self.p.waterlevel = 3;
                }
            }
            if (CONTENTS_CURRENT_DOWN..=CONTENTS_CURRENT_0).contains(&truecont) {
                const CURRENT: [Vec3; 6] =
                    [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0, -1.0]];
                let dir = CURRENT[(CONTENTS_CURRENT_0 - truecont) as usize];
                self.p.basevelocity = F::ma(self.p.basevelocity, 50.0 * self.p.waterlevel as f32, dir);
            }
        }
        self.p.waterlevel > 1
    }

    /// `PM_WaterMove` (0xfe0b8, client 0x1002bb70). MSVC keeps both lengths unrounded on the
    /// stack and multiplies the friction and acceleration terms in another order.
    fn water_move(&mut self) {
        let c = self.p.cmd;
        let (fm, sm) = (F::x(c.forwardmove), F::x(c.sidemove));
        let w = |i: usize| F::x(self.p.forward[i]) * fm + F::x(self.p.right[i]) * sm;
        let w2 = w(2);
        let mut wishvel = [w(0).f(), w(1).f(), w2.f()];
        if eq87(c.forwardmove, 0.0) && eq87(c.sidemove, 0.0) && eq87(c.upmove, 0.0) {
            // GCC subtracts from the unrounded sum still on the FPU stack, MSVC reloads it.
            wishvel[2] = (if F::MSVC { F::x(wishvel[2]) } else { w2 } - F::x(60.0)).f();
        } else {
            wishvel[2] += c.upmove;
        }
        let mut wishdir = wishvel;
        let wishspeed = if F::MSVC {
            let len = F::normalize(&mut wishdir);
            let max = F::x(self.p.maxspeed);
            let len = if len > max {
                wishvel = scale(wishvel, (max / len).f());
                max
            } else {
                len
            };
            (len * F::xd(0.8)).f()
        } else {
            let mut wishspeed = F::normalize(&mut wishdir).f();
            if wishspeed > self.p.maxspeed {
                wishvel = scale(wishvel, self.p.maxspeed / wishspeed);
                wishspeed = self.p.maxspeed;
            }
            (F::xd(0.8) * F::x(wishspeed)).f()
        };

        self.p.velocity = add(self.p.velocity, self.p.basevelocity);
        let mut temp = self.p.velocity;
        let speed = F::normalize(&mut temp);
        let newspeed = if !eq87(speed, F::ZERO) {
            let (mvfric, pfric, ft) = (F::x(self.mv().friction), F::x(self.p.friction), F::x(self.p.frametime));
            let ns = if F::MSVC { speed - mvfric * pfric * ft * speed } else { speed - speed * ft * mvfric * pfric };
            let ns = if ns >= F::ZERO { ns.f() } else { 0.0 };
            let s = if F::MSVC { (F::x(ns) / speed).f() } else { ns / speed.f() };
            self.p.velocity = scale(self.p.velocity, s);
            ns
        } else {
            0.0
        };

        if below87(wishspeed, 0.1) {
            return;
        }
        let addspeed = F::x(wishspeed) - F::x(newspeed);
        if addspeed > F::ZERO {
            let addspeed = addspeed.f();
            F::normalize(&mut wishvel);
            let (ws, acc, ft, pfric) = (F::x(wishspeed), F::x(self.mv().accelerate), F::x(self.p.frametime), F::x(self.p.friction));
            let mut accelspeed = if F::MSVC { acc * pfric * ft * ws } else { ws * acc * ft * pfric };
            if accelspeed > F::x(addspeed) {
                accelspeed = F::x(addspeed);
            }
            self.p.velocity = F::ma_x(self.p.velocity, accelspeed, wishvel);
        }

        let dest = F::ma(self.p.origin, self.p.frametime, self.p.velocity);
        let mut start = dest;
        start[2] = (F::x(dest[2]) + F::x(self.mv().stepsize) + F::x(1.0)).f();
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
        // MSVC's compares take a NaN as zero here and as negative below.
        if if F::MSVC { eq87(self.p.waterjumptime, 0.0) } else { self.p.waterjumptime == 0.0 } {
            return;
        }
        self.p.waterjumptime -= self.p.cmd.msec as f32;
        let expired = if F::MSVC { below87(self.p.waterjumptime, 0.0) } else { self.p.waterjumptime < 0.0 };
        if expired || self.p.waterlevel == 0 {
            self.p.waterjumptime = 0.0;
            self.p.flags &= !FL_WATERJUMP_;
        }
        self.p.velocity[0] = self.p.movedir[0];
        self.p.velocity[1] = self.p.movedir[1];
    }

    /// `PM_CheckWaterJump` (0x100160, client 0x1002dc10). The values are the same in both
    /// builds; MSVC's compares send a NaN the other way.
    fn check_water_jump(&mut self) {
        let msvc = F::MSVC;
        if if msvc { !eq87(self.p.waterjumptime, 0.0) } else { self.p.waterjumptime != 0.0 } {
            return;
        }
        if if msvc { below87(self.p.velocity[2], -180.0) } else { self.p.velocity[2] < -180.0 } {
            return;
        }
        let mut flatvelocity = [self.p.velocity[0], self.p.velocity[1], 0.0];
        let curspeed = F::normalize(&mut flatvelocity).f();
        let mut flatforward = [self.p.forward[0], self.p.forward[1], 0.0];
        F::normalize(&mut flatforward);
        let d = F::dot_in(flatvelocity, flatforward, F::ZYX);
        if !eq87(curspeed, 0.0) && if msvc { below87(d, F::ZERO) } else { d < F::ZERO } {
            return;
        }
        let mut start = self.p.origin;
        start[2] += WJ_HEIGHT;
        let end = F::ma(start, 24.0, flatforward);
        let savehull = self.p.usehull;
        self.p.usehull = 2;
        let tr = self.trace(start, end);
        let (frac, n2) = (tr.fraction, tr.plane.normal[2].abs());
        let wall = if msvc { below87(frac, 1.0) && below87(n2, 0.1) } else { frac < 1.0 && n2 < 0.1 };
        if wall {
            start[2] = (F::x(self.hull_max(savehull)[2]) - F::x(WJ_HEIGHT) + F::x(start[2])).f();
            let end = F::ma(start, 24.0, flatforward);
            self.p.movedir = F::ma(ZERO, -50.0, tr.plane.normal);
            let tr = self.trace(start, end);
            if if msvc { eq87(tr.fraction, 1.0) } else { tr.fraction == 1.0 } {
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
        self.p.onground = if below87(F::x(tr.plane.normal[2]), F::xd(0.7)) { -1 } else { tr.ent };
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
        // GCC stores `Sys_FloatTime()` to a float first; MSVC tests the double and stores after.
        let ftime = if F::MSVC { F::xd(self.p.float_time()) } else { F::x(self.p.float_time() as f32) };
        if let Some(slot) = STUCK_CHECK_TIME.get(pi as usize) {
            if not_above87(ftime - F::xd(PM_CHECKSTUCK_MINTIME), F::x(f32::from_bits(slot[idx].load(Relaxed)))) {
                return true;
            }
            slot[idx].store(ftime.f().to_bits(), Relaxed);
        }

        self.p.stuck_touch(hitent, &mut traceresult);

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

    /// `PM_SpectatorMove` (0xfef3c; mp.dll 0x10038d20, and the roaming part of the client's own
    /// 0x1002c790): roaming, or follow a target (`iuser2`) with the TS chase camera
    /// (`GetChaseOrigin`). MSVC keeps the speeds and the friction on the stack.
    fn spectator_move(&mut self) {
        if self.p.iuser1 == OBS_ROAMING || self.p.iuser2 == 0 {
            let speed = F::length(self.p.velocity);
            if below87(speed, F::x(1.0)) {
                self.p.velocity = ZERO;
            } else {
                let stopspeed = F::x(self.mv().stopspeed);
                let control = if speed >= stopspeed { speed } else { stopspeed };
                let newspeed = if F::MSVC {
                    let friction = F::x(self.mv().friction) * F::xd(1.5);
                    let newspeed = speed - friction * (control * F::x(self.p.frametime));
                    if below87(newspeed, F::ZERO) { F::ZERO } else { newspeed }
                } else {
                    let friction = (F::xd(1.5) * F::x(self.mv().friction)).f();
                    let newspeed = speed - (control * F::x(friction) * F::x(self.p.frametime) + F::ZERO);
                    if F::ZERO > newspeed { F::ZERO } else { newspeed }
                };
                self.p.velocity = scale(self.p.velocity, (newspeed / speed).f());
            }
            let (fmove, smove) = (F::x(self.p.cmd.forwardmove), F::x(self.p.cmd.sidemove));
            F::normalize(&mut self.p.forward);
            F::normalize(&mut self.p.right);
            let w = |i: usize| fmove * F::x(self.p.forward[i]) + smove * F::x(self.p.right[i]);
            // MSVC stores all three sums and reloads z to add the up move.
            let w2 = if F::MSVC { F::x(w(2).f()) } else { w(2) };
            let wishvel = [w(0).f(), w(1).f(), (w2 + F::x(self.p.cmd.upmove)).f()];
            let mut wishdir = wishvel;
            let mut wishspeed = F::normalize(&mut wishdir);
            let maxspd = F::x(self.mv().spectatormaxspeed);
            if wishspeed > maxspd {
                wishspeed = maxspd;
            }
            let addspeed = wishspeed - F::dot_in(self.p.velocity, wishdir, F::ZYX);
            if not_above87(addspeed, F::ZERO) {
                return;
            }
            // MSVC clamps to the stored add speed.
            let addspeed = if F::MSVC { F::x(addspeed.f()) } else { addspeed };
            let mut accelspeed = F::x(self.mv().accelerate) * F::x(self.p.frametime) * wishspeed;
            if accelspeed > addspeed {
                accelspeed = addspeed;
            }
            self.p.velocity = F::ma_x(self.p.velocity, accelspeed, wishdir);
            self.p.origin = F::ma(self.p.origin, self.p.frametime, self.p.velocity);
            return;
        }

        let n = self.p.numphysent.clamp(0, 600) as usize;
        let Some(target) = (0..n).find(|&i| self.p.physents[i].info == self.p.iuser2) else { return };
        let pe = &self.p.physents[target];
        let mode = self.p.iuser1;
        // TS never sets the camera angles for the HLTV locked view (4): whatever the stack
        // held is used, which `StaleArgs` reproduces.
        let angles = match mode {
            OBS_CHASE_LOCKED => [10.0 - pe.angles[0], pe.angles[1], pe.angles[2]],
            OBS_IN_EYE => pe.angles,
            OBS_LOCKED_VIEW => STALE_ARGS.get().map(f32::from_bits),
            _ => self.p.angles,
        };
        let origin = if mode == OBS_IN_EYE { pe.origin } else { self.chase_origin(angles, target) };
        self.p.origin = origin;
        self.p.angles = angles;
        self.p.velocity = ZERO;
    }

    /// `GetChaseOrigin` (0xfee5c): 90 units behind the target's eyes (z + 35), pulled 4 units
    /// off any wall in between.
    fn chase_origin(&self, angles: Vec3, target: usize) -> Vec3 {
        chase_origin_in::<F>(self.p, angles, target)
    }

    /// `PM_NoClip` (0x10000c, client 0x1002daf0).
    fn noclip(&mut self) {
        let (fmove, smove) = (F::x(self.p.cmd.forwardmove), F::x(self.p.cmd.sidemove));
        F::normalize(&mut self.p.forward);
        F::normalize(&mut self.p.right);
        let w = |i: usize| fmove * F::x(self.p.forward[i]) + smove * F::x(self.p.right[i]);
        // MSVC stores all three sums and reloads z to add the up move.
        let w2 = if F::MSVC { F::x(w(2).f()) } else { w(2) };
        let wishvel = [w(0).f(), w(1).f(), (w2 + F::x(self.p.cmd.upmove)).f()];
        self.p.origin = F::ma(self.p.origin, self.p.frametime, wishvel);
        self.p.velocity = ZERO;
    }

    /// `PM_PushEntity` (0xffc24, client 0x1002d710).
    fn push_entity(&mut self, push: Vec3) -> pmtrace_t {
        let end = add(self.p.origin, push);
        let trace = self.trace(self.p.origin, end);
        self.p.origin = trace.endpos;
        // MSVC touches on a NaN fraction.
        let hit = if F::MSVC { below87(trace.fraction, 1.0) } else { trace.fraction < 1.0 };
        if hit && trace.allsolid == 0 {
            self.add_to_touched(trace, self.p.velocity);
        }
        trace
    }

    /// `PM_Physics_Toss` (0xffce8, client 0x1002d7e0).
    fn physics_toss(&mut self) {
        self.check_water();
        if self.p.velocity[2] > 0.0 {
            self.p.onground = -1;
        }
        // MSVC's `VectorCompare` (0x1002fa00) counts a NaN component as equal.
        let is_zero = |v: Vec3| if F::MSVC { v.iter().all(|&c| eq87(c, 0.0)) } else { v == ZERO };
        if self.p.onground != -1 && is_zero(self.p.basevelocity) && is_zero(self.p.velocity) {
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
        if eq87(trace.fraction, 1.0) {
            self.check_water();
            return;
        }
        let backoff = match self.p.movetype {
            MT_BOUNCE => (F::xd(2.0) - F::x(self.p.friction)).f(),
            MT_BOUNCEMISSILE => 2.0,
            _ => 1.0,
        };
        let v = self.p.velocity;
        clip_velocity::<F>(v, trace.plane.normal, &mut self.p.velocity, backoff);
        if F::x(trace.plane.normal[2]) > F::xd(0.7) {
            if F::x(self.p.velocity[2]) < F::x(self.mv().gravity) * F::x(self.p.frametime) {
                self.p.onground = trace.ent;
                self.p.velocity[2] = 0.0;
            }
            let v = self.p.velocity;
            let vel = F::x(v[0]) * F::x(v[0]) + F::x(v[1]) * F::x(v[1]) + F::x(v[2]) * F::x(v[2]);
            if below87(vel, F::x(30.0 * 30.0)) || (self.p.movetype != MT_BOUNCE && self.p.movetype != MT_BOUNCEMISSILE) {
                self.p.onground = trace.ent;
                self.p.velocity = ZERO;
            } else {
                let mv = scale(self.p.velocity, ((F::xd(1.0) - F::x(trace.fraction)) * F::x(self.p.frametime) * F::xd(0.9)).f());
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
            if self.p.moveents[i].skin != CONTENTS_LADDER {
                continue;
            }
            match self.p.moveent_hull_contents(i, self.p.origin) {
                Some(c) if c != CONTENTS_EMPTY => return Some(i),
                _ => continue,
            }
        }
        None
    }

    /// `PM_LadderMove` (0xff654, client 0x1002d1b0): TS climbs at a flat 200 u/s.
    fn ladder_move(&mut self, ladder: usize) {
        if self.p.movetype == MT_NOCLIP {
            return;
        }
        let (modelmins, modelmaxs) = self.p.moveent_model_bounds(ladder);
        let ladder_center = scale(add(modelmins, modelmaxs), 0.5);
        self.p.movetype = MT_FLY;

        let mut floor = self.p.origin;
        floor[2] = (F::x(self.hull_min(self.p.usehull)[2]) - F::x(1.0) + F::x(floor[2])).f();
        let on_floor = self.point_contents(floor, None) == CONTENTS_SOLID;

        self.p.gravity = 0.0;
        let trace = self.p.moveent_trace_model(ladder, self.p.origin, ladder_center).unwrap_or_default();
        if eq87(trace.fraction, 1.0) {
            return;
        }
        let (vpn, v_right, _) = F::angle_vectors(self.p.angles);
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
            let velocity = F::ma(scale(vpn, forward), right, v_right);
            let mut perp = F::cross([0.0, 0.0, 1.0], normal);
            F::normalize(&mut perp);
            let n = F::dot_in(velocity, normal, F::ZYX).f();
            let crossv = scale(normal, n);
            let lateral = sub(velocity, crossv);
            let tmp = F::cross(normal, perp);
            self.p.velocity = F::ma(lateral, -n, tmp);
            if on_floor && n > 0.0 {
                self.p.velocity = F::ma(self.p.velocity, MAX_CLIMB_SPEED, normal);
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

    /// The origin moved by the hull difference `player_mins[1] - player_mins[0]`: up (`+1`,
    /// `PM_UnDuck`, `TSStandUp`, when on the ground) or down (`-1`, `PM_Duck`, `TSGoDuck`). Each
    /// component is rounded once, as TS computes the difference on the FPU stack.
    pub(crate) fn duck_hull_offset(&self, sign: f32) -> Vec3 {
        let mut o = self.p.origin;
        if sign > 0.0 && self.p.onground == -1 {
            return o;
        }
        for (i, c) in o.iter_mut().enumerate() {
            let d = F::x(self.p.player_mins[1][i]) - F::x(self.p.player_mins[0][i]);
            *c = if sign > 0.0 { d + F::x(*c) } else { F::x(*c) - d }.f();
        }
        o
    }

    /// `PM_UnDuck` (0xff51c), view height 28.
    fn unduck(&mut self) {
        let new_origin = self.duck_hull_offset(1.0);
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
            let c = &mut self.p.cmd;
            c.forwardmove = (F::xd(0.333) * F::x(c.forwardmove)).f();
            c.sidemove = (F::xd(0.333) * F::x(c.sidemove)).f();
            c.upmove = (F::xd(0.333) * F::x(c.upmove)).f();
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
        // Both duck-time products keep the `double` 0.001 in extended precision: at 600 ms the
        // test below is false (0.6000000000000000124 > 0.6), so the duck completes a frame later
        // than `f32` or `f64` arithmetic would have it.
        let time = F::xd(1.0) - F::xd(0.001) * F::x(self.p.flDuckTime);
        let time = (if F::ZERO > time { F::ZERO } else { time }).f();
        if self.p.bInDuck == 0 {
            return;
        }
        if not_above87(F::xd(0.001) * F::x(self.p.flDuckTime), F::xd(0.6)) || self.p.onground == -1 {
            self.p.usehull = 1;
            self.p.view_ofs[2] = VIEW_DUCK;
            self.p.flags |= FL_DUCKING_;
            self.p.bInDuck = 0;
            if self.p.onground != -1 {
                self.p.origin = self.duck_hull_offset(-1.0);
                self.fix_player_crouch_stuck(STUCK_MOVEUP);
                self.categorize_position();
            }
        } else {
            // fMore = VEC_DUCK_HULL_MIN - VEC_HULL_MIN = 18
            let duck_fraction = spline_fraction::<F>(time, 2.5);
            self.p.view_ofs[2] = ((F::x(1.0) - duck_fraction) * F::x(VIEW_STAND) + (F::x(VIEW_DUCK) - F::x(18.0)) * duck_fraction).f();
        }
    }

    // ---- jumping and falling ------------------------------------------------------------------------

    /// `PM_PreventMegaBunnyJumping` (0x1000d4, client 0x1002dba0). MSVC tests the unrounded
    /// product and divides by the unrounded speed.
    fn prevent_mega_bunny_jumping(&mut self) {
        let m = F::x(BUNNYJUMP_MAX_SPEED_FACTOR) * F::x(self.p.maxspeed);
        let maxscaledspeed = m.f();
        if not_above87(if F::MSVC { m } else { F::x(maxscaledspeed) }, F::ZERO) {
            return;
        }
        let spd = F::length(self.p.velocity);
        if not_above87(spd, F::x(maxscaledspeed)) {
            return;
        }
        let fraction = (F::x(maxscaledspeed) / spd * F::xd(0.65)).f();
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
            // MSVC's compares send a NaN the other way in all three tests below.
            let msvc = F::MSVC;
            if if msvc { !eq87(self.p.waterjumptime, 0.0) } else { self.p.waterjumptime != 0.0 } {
                self.p.waterjumptime -= self.p.cmd.msec as f32;
                if if msvc { below87(self.p.waterjumptime, 0.0) } else { self.p.waterjumptime < 0.0 } {
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
                if if msvc { not_above87(self.p.flSwimTime, 0.0) } else { self.p.flSwimTime <= 0.0 } {
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
                self.p.punchangle[2] = (F::xd(0.013) * F::x(self.p.flFallVelocity)).f();
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
        let Some(name) = self.p.trace_texture(self.p.onground, start, end) else { return };
        let mut name = name.as_slice();
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

/// `PM_ClipVelocity` (0xfd210, client 0x1002b190): slide off the impacting plane. Returns the
/// blocked flags. The near-zero test compares against the `double` ±0.1: GCC tests the
/// unrounded component, MSVC the `float` it has just stored.
fn clip_velocity<F: Fpu>(input: Vec3, normal: Vec3, out: &mut Vec3, overbounce: f32) -> i32 {
    let angle = normal[2];
    let mut blocked = 0;
    if angle > 0.0 {
        blocked |= 1;
    }
    if eq87(angle, 0.0) {
        blocked |= 2;
    }
    let order = if F::MSVC { [1, 2, 0] } else { [0, 1, 2] };
    let backoff = F::dot_in(input, normal, order) * F::x(overbounce);
    for i in 0..3 {
        let o = F::x(input[i]) - backoff * F::x(normal[i]);
        let o = if F::MSVC { F::x(o.f()) } else { o };
        out[i] = if o > F::xd(-STOP_EPSILON) && o < F::xd(STOP_EPSILON) { 0.0 } else { o.f() };
    }
    blocked
}

/// `PM_SplineFraction` (0xff3ec, client 0x1002cf90), returned unrounded. MSVC doubles with
/// `fadd st, st` after the product, which is exact either way.
fn spline_fraction<F: Fpu>(value: f32, scale: f32) -> F::N {
    let value = F::x(scale) * F::x(value);
    let value_squared = value * value;
    value_squared * F::x(3.0) - (value_squared + value_squared) * value
}

/// `PM_CalcRoll` (0x1006d4, client 0x1002e120), returned unrounded. MSVC sums the dot product
/// from z down and stores `side` to a `float` before `fabs`.
fn calc_roll<F: Fpu>(angles: Vec3, velocity: Vec3, rollangle: f32, rollspeed: f32) -> F::N {
    let (_, right, _) = F::angle_vectors(angles);
    let side = F::dot_in(velocity, right, F::ZYX);
    let sign = if side >= F::ZERO { F::x(1.0) } else { F::x(-1.0) };
    let side = if F::MSVC { F::x(side.f()) } else { side }.abs();
    let value = F::x(rollangle);
    let side = if side >= F::x(rollspeed) { value } else { side * value / F::x(rollspeed) };
    side * sign
}

/// `PM_GetAccelBySpeed` (0xfdcc4, client 0x1002b7f0): ground acceleration factor from the
/// slot-weight maxspeed, 0.6 at 210 u/s up to 1.1 at 330 u/s, returned unrounded.
fn accel_by_speed_x<F: Fpu>(maxspeed: f32) -> F::N {
    if maxspeed >= 330.0 { F::x(1.1) } else { (F::x(maxspeed) - F::x(210.0)) * F::x(0.008_333_334) * F::x(0.5) + F::x(0.6) }
}

/// [`accel_by_speed_x`] rounded to a `float` (the same in both builds).
pub fn accel_by_speed(maxspeed: f32) -> f32 {
    accel_by_speed_x::<Gcc>(maxspeed).f()
}
