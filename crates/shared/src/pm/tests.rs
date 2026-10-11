//! Fake-world tests: a floor at z = 0 and a wall at x = 200, hull-aware.

use super::*;
use crate::ts::stunt;
use goldsrc_sys::movevars_s;
use std::alloc::{Layout, alloc_zeroed};
use std::cell::Cell;

const FLOOR_Z: f32 = 0.0;
const WALL_X: f32 = 200.0;

thread_local! {
    /// The playermove under test, so the trace callbacks can see `usehull`.
    static CURRENT: Cell<*const playermove_t> = const { Cell::new(std::ptr::null()) };
    static SOUNDS: Cell<u32> = const { Cell::new(0) };
}

fn hull() -> (Vec3, Vec3) {
    let p = CURRENT.with(|c| c.get());
    let h = if p.is_null() { 0 } else { unsafe { (*p).usehull } };
    hull_bounds(h).unwrap()
}

unsafe extern "C" fn trace(start: *mut f32, end: *mut f32, _flags: i32, _ignore: i32) -> pmtrace_t {
    let s = unsafe { [*start, *start.add(1), *start.add(2)] };
    let e = unsafe { [*end, *end.add(1), *end.add(2)] };
    let (mins, maxs) = hull();
    let mut tr = pmtrace_t { fraction: 1.0, endpos: e, ent: -1, ..Default::default() };
    let (bottom_s, bottom_e) = (s[2] + mins[2], e[2] + mins[2]);
    let (front_s, front_e) = (s[0] + maxs[0], e[0] + maxs[0]);
    if bottom_s < FLOOR_Z - 0.01 || front_s > WALL_X + 0.01 {
        tr.startsolid = 1;
        tr.allsolid = 1;
        tr.fraction = 0.0;
        tr.endpos = s;
        tr.ent = 0;
        return tr;
    }
    let mut hit = |frac: f32, normal: Vec3| {
        if frac < tr.fraction {
            tr.fraction = frac;
            tr.plane.normal = normal;
            tr.ent = 0;
        }
    };
    if bottom_e < FLOOR_Z && bottom_s >= FLOOR_Z {
        hit((bottom_s - FLOOR_Z) / (bottom_s - bottom_e), [0.0, 0.0, 1.0]);
    }
    if front_e > WALL_X && front_s <= WALL_X {
        hit((WALL_X - front_s) / (front_e - front_s), [-1.0, 0.0, 0.0]);
    }
    tr.endpos = ma(s, tr.fraction, sub(e, s));
    tr
}

unsafe extern "C" fn test_position(pos: *mut f32, _tr: *mut pmtrace_t) -> i32 {
    let p = unsafe { [*pos, *pos.add(1), *pos.add(2)] };
    let (mins, maxs) = hull();
    if p[2] + mins[2] < FLOOR_Z - 0.01 || p[0] + maxs[0] > WALL_X + 0.01 { 0 } else { -1 }
}

unsafe extern "C" fn point_contents(_p: *mut f32, truecont: *mut i32) -> i32 {
    if !truecont.is_null() {
        unsafe { *truecont = CONTENTS_EMPTY };
    }
    CONTENTS_EMPTY
}

unsafe extern "C" fn random_long(lo: i32, _hi: i32) -> i32 {
    lo
}

unsafe extern "C" fn float_time() -> f64 {
    0.0
}

unsafe extern "C" fn play_sound(_ch: i32, _s: *const c_char, _v: f32, _a: f32, _f: i32, _p: i32) {
    SOUNDS.with(|c| c.set(c.get() + 1));
}

/// Minimal info-string lookup (`\key\value\key\value`).
unsafe extern "C" fn info_value(s: *const c_char, key: *const c_char) -> *const c_char {
    thread_local!(static BUF: std::cell::RefCell<std::ffi::CString> = std::cell::RefCell::new(std::ffi::CString::default()));
    let s = unsafe { CStr::from_ptr(s) }.to_str().unwrap_or("");
    let key = unsafe { CStr::from_ptr(key) }.to_str().unwrap_or("");
    let parts: Vec<&str> = s.trim_start_matches('\\').split('\\').collect();
    let mut value = "";
    for kv in parts.chunks(2) {
        if kv.len() == 2 && kv[0] == key {
            value = kv[1];
        }
    }
    BUF.with(|b| {
        *b.borrow_mut() = std::ffi::CString::new(value).unwrap();
        b.borrow().as_ptr()
    })
}

struct Fixture {
    p: Box<playermove_t>,
    mv: Box<movevars_s>,
}

impl Fixture {
    fn new() -> Self {
        // playermove_t is ~300 KB: allocate it directly on the heap.
        let mut p = unsafe { Box::from_raw(alloc_zeroed(Layout::new::<playermove_t>()).cast::<playermove_t>()) };
        let mv = Box::new(movevars_s {
            gravity: 800.0,
            stopspeed: 100.0,
            maxspeed: 320.0,
            accelerate: 10.0,
            airaccelerate: 10.0,
            friction: 4.0,
            edgefriction: 2.0,
            stepsize: 18.0,
            maxvelocity: 2000.0,
            spectatormaxspeed: 500.0,
            ..Default::default()
        });
        p.movetype = MOVETYPE_WALK as i32;
        p.maxspeed = 320.0;
        p.gravity = 1.0;
        p.friction = 1.0;
        p.fuser1 = 1.0;
        p.multiplayer = 1;
        p.server = 1;
        p.runfuncs = 1;
        p.onground = -1;
        p.view_ofs = [0.0, 0.0, VIEW_STAND];
        for h in 0..3 {
            let (mins, maxs) = hull_bounds(h).unwrap();
            p.player_mins[h as usize] = mins;
            p.player_maxs[h as usize] = maxs;
        }
        p.PM_PlayerTrace = Some(trace);
        p.PM_TestPlayerPosition = Some(test_position);
        p.PM_PointContents = Some(point_contents);
        p.RandomLong = Some(random_long);
        p.Sys_FloatTime = Some(float_time);
        p.PM_PlaySound = Some(play_sound);
        p.PM_Info_ValueForKey = Some(info_value);
        Fixture { p, mv }
    }

    fn set_physinfo(&mut self, s: &str) {
        for (d, b) in self.p.physinfo.iter_mut().zip(s.bytes().chain(std::iter::once(0))) {
            *d = b as c_char;
        }
    }

    /// Runs `frames` usercmds of `msec` each.
    fn run(&mut self, frames: usize) {
        self.p.movevars = &mut *self.mv;
        CURRENT.with(|c| c.set(&*self.p));
        for _ in 0..frames {
            self.p.cmd.msec = 10;
            player_move(&mut self.p);
        }
    }

    /// A player standing on the floor.
    fn standing() -> Self {
        let mut f = Fixture::new();
        f.p.origin = [0.0, 0.0, 36.0];
        f.run(1);
        assert_ne!(f.p.onground, -1);
        f
    }
}

#[test]
fn falls_and_lands_on_floor() {
    let mut f = Fixture::new();
    f.p.origin = [0.0, 0.0, 100.0];
    f.run(200);
    assert!((f.p.origin[2] - 36.0).abs() < 0.01, "z = {}", f.p.origin[2]);
    assert_eq!(f.p.velocity[2], 0.0);
    assert_ne!(f.p.flags & FL_ONGROUND as i32, 0);
}

#[test]
fn walks_to_maxspeed_and_stops_at_wall() {
    let mut f = Fixture::standing();
    f.p.cmd.forwardmove = 400.0;
    f.run(60);
    let speed = length(f.p.velocity);
    assert!((speed - 320.0).abs() < 1.0, "speed = {speed}");
    f.run(100);
    assert!(f.p.origin[0] <= WALL_X - 16.0 + 0.01, "x = {}", f.p.origin[0]);
    assert!(f.p.origin[0] > WALL_X - 16.0 - 1.0, "x = {}", f.p.origin[0]);
    assert_eq!(f.p.velocity[0], 0.0);
}

#[test]
fn backpedal_is_80_percent() {
    let mut f = Fixture::standing();
    f.p.cmd.forwardmove = -320.0;
    f.run(100);
    let speed = length(f.p.velocity);
    assert!((speed - 256.0).abs() < 1.0, "speed = {speed}");
}

#[test]
fn jump_height_is_35_units() {
    let mut f = Fixture::standing();
    f.p.cmd.buttons = IN_JUMP as u16;
    f.run(1);
    let mut top = f.p.origin[2];
    for _ in 0..100 {
        f.run(1);
        top = top.max(f.p.origin[2]);
    }
    let height = top - 36.0;
    assert!((height - 35.0).abs() < 0.6, "jump height = {height}");
    // Still holding jump: landed and did not jump again.
    assert_eq!(f.p.velocity[2], 0.0);
    assert!((f.p.origin[2] - 36.0).abs() < 0.01);
}

#[test]
fn friction_has_ts_factor() {
    let mut f = Fixture::standing();
    f.p.velocity = [300.0, 0.0, 0.0];
    f.run(1);
    // drop = speed * friction(4) * 0.75 * 0.01
    let expected = 300.0 - 300.0 * 4.0 * 0.75 * 0.01;
    assert!((f.p.velocity[0] - expected).abs() < 1e-3, "v = {}", f.p.velocity[0]);
}

#[test]
fn edge_friction_doubles_drop() {
    // Over the edge (no floor 16 units ahead at the feet): friction * edgefriction.
    let mut f = Fixture::standing();
    unsafe extern "C" fn edge_trace(start: *mut f32, end: *mut f32, flags: i32, ig: i32) -> pmtrace_t {
        let s = unsafe { [*start, *start.add(1), *start.add(2)] };
        let e = unsafe { [*end, *end.add(1), *end.add(2)] };
        // The edge probe is a straight-down trace from the feet: report nothing below.
        if s[0] != 0.0 && s[0] == e[0] && e[2] < s[2] - 30.0 {
            return pmtrace_t { fraction: 1.0, endpos: e, ent: -1, ..Default::default() };
        }
        unsafe { trace(start, end, flags, ig) }
    }
    f.p.PM_PlayerTrace = Some(edge_trace);
    f.p.velocity = [300.0, 0.0, 0.0];
    f.run(1);
    let expected = 300.0 - 300.0 * 4.0 * 2.0 * 0.75 * 0.01;
    assert!((f.p.velocity[0] - expected).abs() < 1e-3, "v = {}", f.p.velocity[0]);
}

#[test]
fn accel_by_speed_factor() {
    assert_eq!(accel_by_speed(330.0), 1.1);
    assert_eq!(accel_by_speed(400.0), 1.1);
    assert!((accel_by_speed(210.0) - 0.6).abs() < 1e-6);
    assert!((accel_by_speed(270.0) - 0.85).abs() < 1e-5);

    // One 10 ms frame from rest at maxspeed 210: accel * dt * wishspeed * factor.
    let mut f = Fixture::standing();
    f.p.maxspeed = 210.0;
    f.p.cmd.forwardmove = 210.0;
    f.run(1);
    let expected = 10.0 * 0.01 * 210.0 * 0.6;
    assert!((f.p.velocity[0] - expected).abs() < 1e-3, "v = {}", f.p.velocity[0]);

    // Ducked: no factor (and the duck multiplier 0.333 on the command).
    let mut f = Fixture::standing();
    f.p.flags |= FL_DUCKING as i32;
    f.p.usehull = 1;
    f.p.origin[2] = 18.0;
    f.p.maxspeed = 210.0;
    f.p.cmd.forwardmove = 210.0;
    f.p.cmd.buttons = IN_DUCK as u16;
    f.p.oldbuttons = IN_DUCK as i32;
    f.run(1);
    let wish = 210.0f32 * 0.333;
    let expected = 10.0 * 0.01 * wish;
    assert!((f.p.velocity[0] - expected).abs() < 1e-3, "v = {}", f.p.velocity[0]);
}

#[test]
fn time_scale_scales_and_freezes() {
    let mut f = Fixture::new();
    f.p.origin = [0.0, 0.0, 500.0];
    f.p.fuser1 = 0.05;
    f.run(50);
    assert_eq!(f.p.origin[2], 500.0, "frozen below 0.1");
    assert_eq!(f.p.velocity[2], 0.0);

    let mut normal = Fixture::new();
    normal.p.origin = [0.0, 0.0, 500.0];
    normal.run(10);
    let mut slow = Fixture::new();
    slow.p.origin = [0.0, 0.0, 500.0];
    slow.p.fuser1 = 0.5;
    slow.run(20);
    // 20 half-speed frames == 10 normal frames.
    assert!((slow.p.origin[2] - normal.p.origin[2]).abs() < 1e-3, "{} vs {}", slow.p.origin[2], normal.p.origin[2]);
}

#[test]
fn step_interval_scales_with_time_scale() {
    let mut f = Fixture::standing();
    f.p.cmd.forwardmove = 320.0;
    f.run(30);
    // Running on concrete: 300 ms.
    f.p.flTimeStepSound = 0;
    f.run(1);
    assert!(f.p.flTimeStepSound == 300 || f.p.flTimeStepSound == 290, "{}", f.p.flTimeStepSound);
    f.p.fuser1 = 0.5;
    f.p.flTimeStepSound = 0;
    f.run(1);
    assert!(f.p.flTimeStepSound >= 590, "{}", f.p.flTimeStepSound);
}

#[test]
fn dive_impulses() {
    for (bit, speed, h) in [(stunt::DIVE_HIGH, 90.0, 50.0), (stunt::DIVE_MED, 90.0, 25.0), (stunt::DIVE_LOW, 100.0, 10.0)] {
        let mut f = Fixture::standing();
        f.p.velocity = [100.0, 0.0, 0.0];
        f.p.iuser4 = stunt::GO_DUCK | stunt::DIVING | bit;
        f.p.cmd.forwardmove = 320.0; // zeroed by the airborne-stunt mask
        f.run(1);
        assert_eq!(f.p.iuser4 & stunt::DIVE_IMPULSE_MASK, 0, "impulse consumed");
        assert_ne!(f.p.iuser4 & stunt::DIVING, 0);
        assert!((f.p.velocity[0] - (100.0 + speed)).abs() < 1e-3, "vx = {}", f.p.velocity[0]);
        let vz0 = (h * 1600.0f32).sqrt();
        // Three half-gravity steps of 800 * 0.01 * 0.5: TSDive's own fixup, then the walk
        // physics' AddCorrectGravity and FixupGravityVelocity (as in TS).
        assert!((f.p.velocity[2] - (vz0 - 12.0)).abs() < 0.05, "vz = {} (expected ~{})", f.p.velocity[2], vz0 - 12.0);
        assert_eq!(f.p.usehull, 1, "stunt uses the duck hull");
        assert_ne!(f.p.flags & FL_DUCKING as i32, 0);
    }
}

#[test]
fn wall_dive_uses_forward() {
    let mut f = Fixture::standing();
    f.p.velocity = [0.0, 50.0, 0.0];
    f.p.cmd.viewangles = [0.0, 0.0, 0.0]; // facing +x
    f.p.iuser4 = stunt::GO_DUCK | stunt::DIVING | stunt::DIVE_HIGH | stunt::WALLDIVE;
    f.run(1);
    assert!((f.p.velocity[0] - 270.0).abs() < 1e-3, "vx = {}", f.p.velocity[0]);
    assert_eq!(f.p.iuser4 & stunt::WALLDIVE, 0);
}

#[test]
fn wall_jump_and_climb() {
    let mut f = Fixture::new();
    f.p.origin = [0.0, 0.0, 200.0];
    f.p.cmd.viewangles = [0.0, 0.0, 0.0];
    f.p.iuser4 = stunt::GO_DUCK | stunt::WALLJUMPING | stunt::WALLJUMP_IMPULSE;
    f.run(1);
    assert!((f.p.velocity[0] + 210.0).abs() < 1e-3, "vx = {}", f.p.velocity[0]);
    assert!((f.p.velocity[2] - (ts::JUMP_VELOCITY - 12.0)).abs() < 0.05, "vz = {}", f.p.velocity[2]);
    assert_eq!(f.p.iuser4 & stunt::WALLJUMP_IMPULSE, 0);

    let mut f = Fixture::new();
    f.p.origin = [0.0, 0.0, 200.0];
    f.p.iuser4 = stunt::GO_DUCK | stunt::WALLCLIMBING | stunt::WALLCLIMB_IMPULSE;
    f.run(1);
    assert!((f.p.velocity[2] - (ts::WALLCLIMB_VELOCITY - 12.0)).abs() < 0.05, "vz = {}", f.p.velocity[2]);
    assert_eq!(f.p.iuser4 & stunt::WALLCLIMB_IMPULSE, 0);
}

#[test]
fn stunt_blocks_jump_without_latching() {
    let mut f = Fixture::standing();
    f.p.iuser4 = stunt::PRONE;
    f.p.cmd.buttons = IN_JUMP as u16;
    f.run(1);
    assert_eq!(f.p.velocity[2], 0.0);
    assert_eq!(f.p.oldbuttons & IN_JUMP as i32, 0, "not latched while blocked");
    // Prone: no forward movement, side movement at 30 %.
    f.p.cmd.buttons = 0;
    f.p.cmd.forwardmove = 320.0;
    f.run(5);
    assert_eq!(f.p.velocity[0], 0.0);
    // Stunt over: the held jump fires immediately.
    f.p.iuser4 = 0;
    f.p.cmd.forwardmove = 0.0;
    f.p.cmd.buttons = IN_JUMP as u16;
    f.run(1);
    assert!(f.p.velocity[2] > 200.0);
}

#[test]
fn duck_view_height_is_16() {
    let mut f = Fixture::standing();
    f.p.cmd.buttons = IN_DUCK as u16;
    f.run(10); // 0.1 s: in transition
    assert_eq!(f.p.flags & FL_DUCKING as i32, 0);
    assert!(f.p.view_ofs[2] < VIEW_STAND && f.p.view_ofs[2] > VIEW_DUCK - 18.0, "view {}", f.p.view_ofs[2]);
    f.run(40);
    assert_ne!(f.p.flags & FL_DUCKING as i32, 0);
    assert_eq!(f.p.view_ofs[2], VIEW_DUCK);
    assert_eq!(f.p.usehull, 1);
    assert!((f.p.origin[2] - 18.0).abs() < 0.01, "z = {}", f.p.origin[2]);
    // Release: stand up.
    f.p.cmd.buttons = 0;
    f.run(1);
    assert_eq!(f.p.view_ofs[2], VIEW_STAND);
    assert_eq!(f.p.usehull, 0);
    assert!((f.p.origin[2] - 36.0).abs() < 0.01, "z = {}", f.p.origin[2]);
}

#[test]
fn super_jump_needs_mj_and_duck() {
    // Without mj: normal jump.
    let mut f = Fixture::standing();
    f.p.cmd.buttons = IN_DUCK as u16;
    f.run(5);
    f.p.cmd.buttons = (IN_DUCK | IN_JUMP) as u16;
    f.run(1);
    assert!(f.p.velocity[2] < 240.0, "vz = {}", f.p.velocity[2]);
    assert_eq!(f.p.iuser4 & stunt::SUPERJUMP_START, 0);

    let mut f = Fixture::standing();
    f.set_physinfo("\\mj\\1");
    f.p.cmd.buttons = IN_DUCK as u16;
    f.run(5);
    f.p.cmd.buttons = (IN_DUCK | IN_JUMP) as u16;
    f.run(1);
    assert!((f.p.velocity[2] - (400.0 - 8.0)).abs() < 0.05, "vz = {}", f.p.velocity[2]);
    assert_ne!(f.p.iuser4 & stunt::SUPERJUMP_START, 0);
}

#[test]
fn glide_gravity() {
    let mut f = Fixture::new();
    f.p.origin = [0.0, 0.0, 300.0];
    f.p.velocity = [0.0, 0.0, 100.0];
    f.p.iuser4 = stunt::SUPERJUMP_GLIDE;
    f.p.cmd.buttons = IN_JUMP as u16;
    f.run(1);
    assert_eq!(f.p.gravity, 0.3);
    f.p.velocity[2] = -100.0;
    f.run(1);
    assert_eq!(f.p.gravity, 0.75);
    f.p.cmd.buttons = 0;
    f.run(1);
    assert_eq!(f.p.gravity, 1.0);
}

#[test]
fn dead_lock_keeps_yaw() {
    let mut f = Fixture::standing();
    f.p.player_index = 7;
    f.p.cmd.viewangles = [10.0, 45.0, 0.0];
    f.run(1);
    assert_eq!(f.p.angles[1], 45.0);
    f.p.iuser4 = stunt::DEAD_LOCK;
    f.p.cmd.viewangles = [10.0, 120.0, 0.0];
    f.run(1);
    assert_eq!(f.p.angles[1], 45.0);
}

#[test]
fn command_is_not_clamped_for_players() {
    let mut f = Fixture::standing();
    f.p.cmd.forwardmove = 1000.0;
    f.p.cmd.msec = 10;
    f.p.movevars = &mut *f.mv;
    CURRENT.with(|c| c.set(&*f.p));
    player_move(&mut f.p);
    assert_eq!(f.p.cmd.forwardmove, 1000.0);
    f.p.iuser1 = 3; // roaming spectator
    f.p.cmd.forwardmove = 1000.0;
    player_move(&mut f.p);
    assert!((f.p.cmd.forwardmove - 320.0).abs() < 1e-3);
}

#[test]
fn stuck_table_layout() {
    let t = &*STUCK_TABLE;
    assert_eq!(t[0], [0.0, 0.0, -0.125]);
    assert_eq!(t[18], [0.0, 0.0, 1.0]);
    assert_eq!(t[52], [2.0, 2.0, 6.0]);
    assert_eq!(t[53], [0.0, 0.0, 0.0]);
}

#[test]
fn hull_bounds_match_sdk() {
    assert_eq!(hull_bounds(1), Some(([-16.0, -16.0, -18.0], [16.0, 16.0, 18.0])));
    assert_eq!(hull_bounds(3), None);
}

#[test]
fn in_eye_spectator_is_mode_5() {
    let mut f = Fixture::standing();
    f.p.movevars = &mut *f.mv;
    CURRENT.with(|c| c.set(&*f.p));
    f.p.numphysent = 2;
    f.p.physents[1].info = 2;
    f.p.physents[1].origin = [50.0, 60.0, 70.0];
    f.p.physents[1].angles = [4.0, 90.0, 0.0];
    f.p.iuser1 = crate::ts::obs::IN_EYE;
    f.p.iuser2 = 2;
    player_move(&mut f.p);
    assert_eq!(f.p.origin, [50.0, 60.0, 70.0], "in-eye sits at the target's origin");
    assert_eq!(f.p.angles, [4.0, 90.0, 0.0]);

    f.p.iuser1 = crate::ts::obs::CHASE_LOCKED;
    player_move(&mut f.p);
    assert_eq!(f.p.angles, [6.0, 90.0, 0.0], "locked chase pitches to 10 - pitch");
    assert!(f.p.origin[1] < 60.0, "locked chase sits behind the target");
}

/// `PM_Duck` compares `0.001 * flDuckTime <= 0.6` in extended precision: at 600 ms left it is
/// false, so with 10 ms frames the duck completes at 590 ms.
#[test]
fn duck_completes_at_590_ms() {
    let mut f = Fixture::standing();
    f.p.cmd.buttons = IN_DUCK as u16;
    f.run(41); // the press frame sets 1000, then 40 frames of 10 ms
    assert_eq!(f.p.flDuckTime, 600.0);
    assert_eq!(f.p.flags & FL_DUCKING as i32, 0, "not yet at 600 ms");
    f.run(1);
    assert_eq!(f.p.flDuckTime, 590.0);
    assert_ne!(f.p.flags & FL_DUCKING as i32, 0);
}

/// The HLTV locked view (mode 4) takes the stale `PM_CalcRoll` arguments as its angles; the roll
/// slot is `movevars->rollangle`.
#[test]
fn locked_view_reads_stale_calc_roll_args() {
    let mut f = Fixture::standing();
    f.mv.rollangle = 2.0;
    f.p.movevars = &mut *f.mv;
    CURRENT.with(|c| c.set(&*f.p));
    f.p.numphysent = 2;
    f.p.physents[1].info = 2;
    f.p.physents[1].origin = [50.0, 60.0, 70.0];
    f.p.iuser1 = crate::ts::obs::LOCKED_VIEW;
    f.p.iuser2 = 2;
    player_move(&mut f.p);
    assert_eq!(f.p.angles[2], 2.0);
}

#[test]
fn ts_constants_are_the_rounded_roots() {
    use super::math::xd;
    assert_eq!(ts::JUMP_VELOCITY, xd(56000.0).sqrt().f());
    assert_eq!(ts::WALLCLIMB_VELOCITY, xd(112_000.0).sqrt().f());
    assert_eq!(ts::HIGH_JUMP_VELOCITY, xd(160_000.0).sqrt().f());
}

/// The software extended precision (`ext.rs`) against the FPU itself: x86-64 Linux runs the
/// x87 at 64-bit precision, round to nearest, as TS's 32-bit server did.
#[cfg(target_arch = "x86_64")]
#[test]
fn ext_matches_x87_hardware() {
    use super::ext::{Ext, x, xd};
    use std::arch::asm;

    /// `fld b; fld a; op st, st(1); fstp out; fstp st(0)`: `a op b` (op 4 = `fsqrt` of `a`).
    fn hw(a: [u8; 10], b: [u8; 10], op: u8) -> [u8; 10] {
        let mut o = [0u8; 10];
        let (pa, pb, po) = (a.as_ptr(), b.as_ptr(), o.as_mut_ptr());
        macro_rules! run {
            ($ins:literal) => {
                // SAFETY: loads two 10-byte values and stores one; the x87 stack is left empty.
                unsafe {
                    asm!(
                        "fld tbyte ptr [{b}]", "fld tbyte ptr [{a}]", $ins, "fstp tbyte ptr [{o}]", "fstp st(0)",
                        a = in(reg) pa, b = in(reg) pb, o = in(reg) po,
                        out("st(0)") _, out("st(1)") _, out("st(2)") _, out("st(3)") _,
                        out("st(4)") _, out("st(5)") _, out("st(6)") _, out("st(7)") _,
                    )
                }
            };
        }
        match op {
            0 => run!("fadd st, st(1)"),
            1 => run!("fsub st, st(1)"),
            2 => run!("fmul st, st(1)"),
            3 => run!("fdiv st, st(1)"),
            _ => run!("fsqrt"),
        }
        o
    }

    /// `fld tbyte; fstp dword`.
    fn hw_f32(a: [u8; 10]) -> f32 {
        let mut o = 0f32;
        // SAFETY: loads a 10-byte value and stores it as a float; the x87 stack is left empty.
        unsafe {
            asm!(
                "fld tbyte ptr [{a}]", "fstp dword ptr [{o}]",
                a = in(reg) a.as_ptr(), o = in(reg) &mut o as *mut f32,
                out("st(0)") _, out("st(1)") _, out("st(2)") _, out("st(3)") _,
                out("st(4)") _, out("st(5)") _, out("st(6)") _, out("st(7)") _,
            )
        }
        o
    }

    let mut seed = 0x1234_5678_9abc_def0u64;
    let mut rnd = || {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        seed
    };
    let float = |r: &mut dyn FnMut() -> u64| -> f32 {
        let b = r();
        let exp = 100 + (b >> 40) as u32 % 56; // 2^-27 .. 2^28
        f32::from_bits(((b >> 63) as u32) << 31 | exp << 23 | (b as u32 & 0x7f_ffff))
    };
    let consts = [0.001f64, 0.333, 0.8, 0.7, 0.65, 0.013, 0.35, 0.3, 0.5, 0.9, 0.05, 0.01, 1.5];
    for i in 0..20_000 {
        // Operands with full 64-bit significands, as the expressions in movement produce.
        let a = x(float(&mut rnd)) * x(float(&mut rnd)) + x(float(&mut rnd));
        let b = if i % 3 == 0 { xd(consts[i % consts.len()]) } else { x(float(&mut rnd)) / x(float(&mut rnd)) };
        for op in 0..5u8 {
            let ours: Ext = match op {
                0 => a + b,
                1 => a - b,
                2 => a * b,
                3 => a / b,
                _ => a.abs().sqrt(),
            };
            let src = if op == 4 { a.abs() } else { a };
            assert_eq!(ours.to_bits80(), hw(src.to_bits80(), b.to_bits80(), op), "op {op} on {a:?} {b:?}");
            assert_eq!(ours.f().to_bits(), hw_f32(ours.to_bits80()).to_bits(), "fstps of {ours:?}");
        }
    }
}
