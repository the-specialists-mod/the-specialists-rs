//! Fake-world tests: a floor at z = 0 and a wall at x = 200, hull-aware.

use super::*;
use crate::gameplay::pm::{GroundAccel, Jump};
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
    tr.endpos = Gcc::ma(s, tr.fraction, sub(e, s));
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

/// Movement rules for these tests: the 3.0 plugin's answers (so the pinned trajectories hold, and
/// the tests see TS's movement), with the hooks a test varies as fields. The plugin's own
/// answers are tested in the plugin.
struct Rules {
    ground_accel: GroundAccel,
    recover_input_scale: f64,
    prone_forward_scale: Option<f64>,
    wall_jump_speed: f32,
    wall_jump_up_velocity: f32,
    wall_climb_velocity: f32,
    wall_dive: bool,
    fix_velocity: fn(Arith, &mut Vec3, f32),
    jump_push: fn(Arith, Jump, u16, f32, &mut Vec3),
    scaled_frametime: fn(f32, f32) -> f32,
    /// `None` decays it as 3.0 does.
    punch: Option<Vec3>,
    /// `None` divides by `max(fuser1, 0.1)`, as 3.0 does.
    step_interval: Option<i32>,
    in_eye_pitch: fn(f32) -> f32,
    roll_divides_first: bool,
    friction_edge_by_reciprocal: bool,
    ieee_compares: bool,
}

/// TS 3.0's numbers (the wall climb is `sqrt(2 * 800 * 70)`).
const BASE: Rules = Rules {
    ground_accel: GroundAccel::BySpeed,
    recover_input_scale: 0.5,
    prone_forward_scale: None,
    wall_jump_speed: 210.0,
    wall_jump_up_velocity: ts::JUMP_VELOCITY,
    wall_climb_velocity: 334.664,
    wall_dive: true,
    fix_velocity: |_, _, _| {},
    jump_push: |_, _, _, _, _| {},
    scaled_frametime: |frametime, fuser1| if below87(fuser1, crate::ts::TIME_SCALE_FREEZE) { 0.0 } else { frametime * fuser1 },
    punch: None,
    step_interval: None,
    in_eye_pitch: |pitch| pitch,
    roll_divides_first: false,
    friction_edge_by_reciprocal: false,
    ieee_compares: false,
};

/// The rules most tests run.
static RULES: Rules = BASE;

impl PmRules for Rules {
    fn ground_accel(&self) -> GroundAccel {
        self.ground_accel
    }
    fn recover_input_scale(&self) -> f64 {
        self.recover_input_scale
    }
    fn prone_forward_scale(&self) -> Option<f64> {
        self.prone_forward_scale
    }
    fn wall_jump_speed(&self) -> f32 {
        self.wall_jump_speed
    }
    fn wall_jump_up_velocity(&self) -> f32 {
        self.wall_jump_up_velocity
    }
    fn wall_climb_velocity(&self) -> f32 {
        self.wall_climb_velocity
    }
    fn wall_dive(&self) -> bool {
        self.wall_dive
    }
    fn fix_velocity(&self, arith: Arith, velocity: &mut Vec3, limit: f32) {
        (self.fix_velocity)(arith, velocity, limit)
    }
    fn jump_push(&self, arith: Arith, jump: Jump, buttons: u16, maxspeed: f32, velocity: &mut Vec3) {
        (self.jump_push)(arith, jump, buttons, maxspeed, velocity)
    }
    fn scaled_frametime(&self, frametime: f32, fuser1: f32) -> f32 {
        (self.scaled_frametime)(frametime, fuser1)
    }
    fn drop_punch_angle(&self, arith: Arith, punch: Vec3, dead: bool, fuser1: f32, frametime: f32) -> Vec3 {
        fn go<F: Fpu>(punch: Vec3, dead: bool, fuser1: f32, frametime: f32) -> Vec3 {
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
            scale(punch, len)
        }
        self.punch.unwrap_or_else(|| crate::by_arith!(arith, go(punch, dead, fuser1, frametime)))
    }
    fn step_interval(&self, arith: Arith, interval: i32, fuser1: f32) -> i32 {
        fn go<F: Fpu>(interval: i32, fuser1: f32) -> i32 {
            let ts = if fuser1 >= 0.1 { fuser1 } else { 0.1 };
            (F::x(1.0) / F::x(ts) * F::xi(interval)).trunc_i32()
        }
        self.step_interval.unwrap_or_else(|| crate::by_arith!(arith, go(interval, fuser1)))
    }
    fn texture_limits(&self) -> crate::gameplay::pm::TextureLimits {
        crate::gameplay::pm::TextureLimits::LARGE
    }
    fn in_eye_pitch(&self, target_pitch: f32) -> f32 {
        (self.in_eye_pitch)(target_pitch)
    }
    fn roll_divides_first(&self) -> bool {
        self.roll_divides_first
    }
    fn friction_edge_by_reciprocal(&self) -> bool {
        self.friction_edge_by_reciprocal
    }
    fn ieee_compares(&self) -> bool {
        self.ieee_compares
    }
    fn speed_by_slots(&self, _free: i32, _the_one: bool) -> f32 {
        unreachable!("server only")
    }
    fn max_player_speed(&self) -> f32 {
        unreachable!("server only")
    }
}

struct Fixture {
    p: Box<playermove_t>,
    mv: Box<movevars_s>,
    /// The build being tested (`ts_i386.so` unless a test says otherwise).
    arith: Arith,
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
        Fixture { p, mv, arith: Arith::Gcc }
    }

    fn set_physinfo(&mut self, s: &str) {
        for (d, b) in self.p.physinfo.iter_mut().zip(s.bytes().chain(std::iter::once(0))) {
            *d = b as c_char;
        }
    }

    /// Runs `frames` usercmds of `msec` each.
    fn run(&mut self, frames: usize) {
        self.run_with(frames, &RULES);
    }

    /// [`run`](Self::run) under a gameplay plugin's movement rules.
    fn run_with(&mut self, frames: usize, rules: &'static dyn crate::gameplay::PmRules) {
        self.p.movevars = &mut *self.mv;
        CURRENT.with(|c| c.set(&*self.p));
        for _ in 0..frames {
            self.p.cmd.msec = 10;
            player_move(&mut self.p, self.arith, rules);
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
    assert!((f.p.velocity[2] - (RULES.wall_climb_velocity - 12.0)).abs() < 0.05, "vz = {}", f.p.velocity[2]);
    assert_eq!(f.p.iuser4 & stunt::WALLCLIMB_IMPULSE, 0);
}

/// The movement runs the rules it is given: a plugin's wall climb and recovery scale replace the
/// others'.
#[test]
fn movement_hooks_follow_the_plugin() {
    static OTHER: Rules = Rules { recover_input_scale: 0.25, wall_climb_velocity: 300.0, ..BASE };

    let mut f = Fixture::new();
    f.p.origin = [0.0, 0.0, 200.0];
    f.p.iuser4 = stunt::GO_DUCK | stunt::WALLCLIMBING | stunt::WALLCLIMB_IMPULSE;
    f.run_with(1, &OTHER);
    assert!((f.p.velocity[2] - (300.0 - 12.0)).abs() < 0.05, "vz = {}", f.p.velocity[2]);

    let mut f = Fixture::standing();
    f.p.iuser4 = stunt::RECOVER;
    f.p.cmd.forwardmove = 400.0;
    f.run_with(1, &OTHER);
    assert_eq!(f.p.cmd.forwardmove, 100.0);
}

/// Every other movement hook reaches the movement: a plugin's values and functions replace the
/// core's where the 3.0 code had its constants and helpers.
#[test]
fn every_movement_hook_reaches_the_movement() {
    thread_local!(static CALLS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) });
    let take = || CALLS.with(|c| std::mem::take(&mut *c.borrow_mut()));
    fn fix_velocity(arith: Arith, _velocity: &mut Vec3, limit: f32) {
        CALLS.with(|c| c.borrow_mut().push(format!("fix {arith:?} {limit}")));
    }
    fn jump_push(arith: Arith, jump: Jump, buttons: u16, maxspeed: f32, velocity: &mut Vec3) {
        CALLS.with(|c| c.borrow_mut().push(format!("jump {arith:?} {jump:?} {buttons:#x} {maxspeed}")));
        velocity[0] += 50.0;
    }
    static OTHER: Rules = Rules {
        ground_accel: GroundAccel::Constant(0.5),
        prone_forward_scale: Some(0.0),
        wall_jump_speed: 170.0,
        wall_jump_up_velocity: 250.0,
        wall_dive: false,
        fix_velocity,
        jump_push,
        step_interval: Some(1234),
        ..BASE
    };
    static SCALES: Rules = Rules { scaled_frametime: |frametime, _| frametime * 2.0, punch: Some([1.0, 2.0, 3.0]), ..BASE };

    for arith in [Arith::Gcc, Arith::Msvc] {
        // A constant ground factor: 0.01 s * 320 * accelerate 10 * 0.5 from rest (3.0's at 320
        // u/s is 1.058).
        let mut f = Fixture::standing();
        f.arith = arith;
        f.p.cmd.forwardmove = 320.0;
        f.run_with(1, &OTHER);
        assert!((f.p.velocity[0] - 16.0).abs() < 1e-4, "{arith:?}: vx = {}", f.p.velocity[0]);
        let mut core = Fixture::standing();
        core.arith = arith;
        core.p.cmd.forwardmove = 320.0;
        core.run(1);
        assert!((core.p.velocity[0] - 32.0 * accel_by_speed(320.0)).abs() < 1e-3, "{arith:?}: core vx = {}", core.p.velocity[0]);

        // Wall jump: the plugin's speeds, then FixVelocity to twice maxspeed.
        let mut f = Fixture::new();
        f.arith = arith;
        f.p.origin = [0.0, 0.0, 200.0];
        f.p.iuser4 = stunt::GO_DUCK | stunt::WALLJUMPING | stunt::WALLJUMP_IMPULSE;
        take();
        f.run_with(1, &OTHER);
        assert!((f.p.velocity[0] + 170.0).abs() < 1e-3, "vx = {}", f.p.velocity[0]);
        assert!((f.p.velocity[2] - (250.0 - 12.0)).abs() < 0.05, "vz = {}", f.p.velocity[2]);
        assert_eq!(take(), [format!("fix {arith:?} 640")]);

        // Wall climb and dive call it too; without the wall dive the dive follows the velocity
        // and keeps the bit.
        let mut f = Fixture::new();
        f.arith = arith;
        f.p.origin = [0.0, 0.0, 200.0];
        f.p.iuser4 = stunt::GO_DUCK | stunt::WALLCLIMBING | stunt::WALLCLIMB_IMPULSE;
        f.run_with(1, &OTHER);
        assert_eq!(take(), [format!("fix {arith:?} 640")]);
        let mut f = Fixture::standing();
        f.arith = arith;
        f.p.velocity = [0.0, 50.0, 0.0];
        f.p.iuser4 = stunt::GO_DUCK | stunt::DIVING | stunt::DIVE_HIGH | stunt::WALLDIVE;
        take();
        f.run_with(1, &OTHER);
        assert!(f.p.velocity[0].abs() < 1e-3 && (f.p.velocity[1] - 140.0).abs() < 1e-3, "v = {:?}", f.p.velocity);
        assert_ne!(f.p.iuser4 & stunt::WALLDIVE, 0);
        assert_eq!(take(), [format!("fix {arith:?} 640")]);

        // Prone: the forward move is multiplied (a -0 stays), not stored as 0.
        for (rules, bits) in [(&OTHER as &'static dyn PmRules, (-0.0f32).to_bits()), (&RULES, 0)] {
            let mut f = Fixture::standing();
            f.arith = arith;
            f.p.iuser4 = stunt::PRONE;
            f.p.cmd.forwardmove = -320.0;
            // Side and up times a `double` 0.3 in both builds: 30 and 60, not 0.3f's
            // 30.000002 and 60.000004 (#126).
            (f.p.cmd.sidemove, f.p.cmd.upmove) = (100.0, 200.0);
            f.run_with(1, rules);
            assert_eq!(f.p.cmd.forwardmove.to_bits(), bits);
            assert_eq!((f.p.cmd.sidemove.to_bits(), f.p.cmd.upmove.to_bits()), (30.0f32.to_bits(), 60.0f32.to_bits()), "{arith:?}");
        }

        // The jump push runs before the upward speed, with the command's buttons and maxspeed.
        let mut f = Fixture::standing();
        f.arith = arith;
        f.p.cmd.buttons = IN_JUMP as u16 | IN_MOVELEFT as u16;
        take();
        f.run_with(1, &OTHER);
        assert_eq!(take(), [format!("jump {arith:?} Normal {:#x} 320", IN_JUMP | IN_MOVELEFT)]);
        assert!((f.p.velocity[0] - 50.0).abs() < 1e-3 && f.p.velocity[2] > 200.0, "v = {:?}", f.p.velocity);

        // The footstep interval.
        let mut f = Fixture::standing();
        f.arith = arith;
        f.p.cmd.forwardmove = 320.0;
        f.run_with(30, &OTHER);
        f.p.flTimeStepSound = 0;
        f.run_with(1, &OTHER);
        assert_eq!(f.p.flTimeStepSound, 1234);

        // The frame time and the punch.
        let mut f = Fixture::new();
        f.arith = arith;
        f.p.origin = [0.0, 0.0, 500.0];
        f.run_with(1, &SCALES);
        assert_eq!(f.p.frametime, 0.02);
        assert_eq!(f.p.punchangle, [1.0, 2.0, 3.0]);
    }
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
    player_move(&mut f.p, Arith::Gcc, &RULES);
    assert_eq!(f.p.cmd.forwardmove, 1000.0);
    f.p.iuser1 = 3; // roaming spectator
    f.p.cmd.forwardmove = 1000.0;
    player_move(&mut f.p, Arith::Gcc, &RULES);
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
    player_move(&mut f.p, Arith::Gcc, &RULES);
    assert_eq!(f.p.origin, [50.0, 60.0, 70.0], "in-eye sits at the target's origin");
    assert_eq!(f.p.angles, [4.0, 90.0, 0.0]);

    f.p.iuser1 = crate::ts::obs::CHASE_LOCKED;
    player_move(&mut f.p, Arith::Gcc, &RULES);
    assert_eq!(f.p.angles, [6.0, 90.0, 0.0], "locked chase pitches to 10 - pitch");
    assert!(f.p.origin[1] < 60.0, "locked chase sits behind the target");
}

/// `PM_Duck` compares `0.001 * flDuckTime <= 0.6` in extended precision: at 600 ms left it is
/// false, so with 10 ms frames `ts_i386.so`'s duck (the server with `ts_pm_arith gcc`) completes
/// at 590 ms.
#[test]
fn gcc_duck_completes_at_590_ms() {
    let mut f = Fixture::standing();
    f.arith = Arith::from_cvar(b"gcc");
    assert_eq!(f.arith, Arith::Gcc);
    f.p.cmd.buttons = IN_DUCK as u16;
    f.run(41); // the press frame sets 1000, then 40 frames of 10 ms
    assert_eq!(f.p.flDuckTime, 600.0);
    assert_eq!(f.p.flags & FL_DUCKING as i32, 0, "not yet at 600 ms");
    f.run(1);
    assert_eq!(f.p.flDuckTime, 590.0);
    assert_ne!(f.p.flags & FL_DUCKING as i32, 0);
}

/// TS's client and `mp.dll` run the same compare at 53-bit precision, where `0.001 * 600` rounds
/// to the double 0.6: the duck completes at 600 ms, a frame before `ts_i386.so`'s. The port's
/// server does the same by default, so it agrees with every client's prediction.
#[test]
fn msvc_duck_completes_at_600_ms() {
    for arith in [Arith::CLIENT, Arith::SERVER_DEFAULT, Arith::from_cvar(b"msvc")] {
        let mut f = Fixture::standing();
        f.arith = arith;
        f.p.cmd.buttons = IN_DUCK as u16;
        f.run(40);
        assert_eq!(f.p.flDuckTime, 610.0);
        assert_eq!(f.p.flags & FL_DUCKING as i32, 0);
        f.run(1);
        assert_eq!(f.p.flDuckTime, 600.0);
        assert_ne!(f.p.flags & FL_DUCKING as i32, 0, "done at 600 ms ({arith:?})");
    }
}

/// The client predicts with `client.dll`'s arithmetic; the server defaults to `mp.dll`'s, the
/// same, on every platform. Only `ts_pm_arith gcc` selects `ts_i386.so`'s.
#[test]
fn arith_per_side() {
    assert_eq!(Arith::CLIENT, Arith::Msvc);
    assert_eq!(Arith::SERVER_DEFAULT, Arith::CLIENT);
    assert_eq!(Arith::from_cvar(b"msvc"), Arith::Msvc);
    assert_eq!(Arith::from_cvar(b""), Arith::Msvc);
    assert_eq!(Arith::from_cvar(b"1"), Arith::Msvc);
    assert_eq!(Arith::from_cvar(b"gcc"), Arith::Gcc);
    assert_eq!(Arith::from_cvar(b" GCC "), Arith::Gcc);
}

/// The server's default run is the client's prediction, command for command.
#[test]
fn server_default_trajectory_is_the_clients() {
    let server = trajectory_hash(|p| player_move(p, Arith::SERVER_DEFAULT, &RULES));
    assert_eq!(server, trajectory_hash(|p| player_move(p, Arith::CLIENT, &RULES)));
    assert_ne!(server, trajectory_hash(|p| player_move(p, Arith::from_cvar(b"gcc"), &RULES)));
}

/// MSVC's `_ftol` keeps the low dword of a 64-bit `fistp`; GCC's `fistpl` gives 0x80000000.
#[test]
fn int_casts_per_build() {
    assert_eq!(Num::trunc_i32(-7.9f64), -7);
    assert_eq!(Num::trunc_i32(4_294_967_301.5f64), 5);
    assert_eq!(Num::trunc_i32(f64::NAN), 0);
    assert_eq!(Gcc::xd(4_294_967_301.5).trunc_i32(), i32::MIN);
    assert_eq!(Gcc::xd(-7.9).trunc_i32(), -7);
}

/// The two builds share the logic but not the last bits, so the same commands drift apart; the
/// client's run is pinned like the server's.
#[test]
fn msvc_trajectory_is_pinned() {
    let msvc = trajectory_hash(|p| player_move(p, Arith::Msvc, &RULES));
    assert_ne!(msvc, trajectory_hash(|p| player_move(p, Arith::Gcc, &RULES)));
    assert_eq!(msvc, 0xb197_b0b5_2fa8_d4e3, "{msvc:#x}");
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
    player_move(&mut f.p, Arith::Gcc, &RULES);
    assert_eq!(f.p.angles[2], 2.0);
}

#[test]
fn ts_constants_are_the_rounded_roots() {
    let xd = Gcc::xd;
    assert_eq!(ts::JUMP_VELOCITY, xd(56000.0).sqrt().f());
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

/// A long scripted run through most of movement (walking with turning, strafing into the wall,
/// friction, jumps, ducking, stunt impulses, slow motion, a bouncy low-friction slide, falling,
/// spectating), hashing every float bit of the state after each command.
fn trajectory_hash(mv: fn(&mut playermove_t)) -> u64 {
    let mut f = Fixture::new();
    f.p.origin = [-37.3, 11.7, 64.25];
    f.p.player_index = 3;
    f.mv.rollangle = 2.0;
    f.mv.rollspeed = 200.0;
    f.p.movevars = &mut *f.mv;
    CURRENT.with(|c| c.set(&*f.p));
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    let mut mix = |v: u32| {
        h ^= v as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    };
    for i in 0..1500u32 {
        let t = i as f32;
        let c = &mut f.p.cmd;
        c.msec = [10, 7, 13, 16, 9][i as usize % 5];
        c.viewangles = [((t * 0.37) % 120.0) - 60.0, (t * 1.13) % 360.0, 0.0];
        c.forwardmove = [320.0, 400.0, -320.0, 0.0, 250.0][(i / 37) as usize % 5];
        c.sidemove = [0.0, 150.0, -320.0, 0.0][(i / 23) as usize % 4];
        c.upmove = 0.0;
        let mut b = 0u16;
        if i % 97 < 3 || (300..340).contains(&i) {
            b |= IN_JUMP as u16;
        }
        if (150..260).contains(&i) || (600..700).contains(&i) {
            b |= IN_DUCK as u16;
        }
        c.buttons = b;
        f.p.iuser4 = match i {
            400 => stunt::GO_DUCK | stunt::DIVING | stunt::DIVE_HIGH,
            460 => stunt::GO_DUCK | stunt::DIVING | stunt::DIVE_LOW,
            520 => stunt::GO_DUCK | stunt::WALLJUMPING | stunt::WALLJUMP_IMPULSE,
            560 => stunt::RECOVER,
            580 => stunt::PRONE,
            590 => 0,
            800 => stunt::SUPERJUMP_GLIDE,
            900 => 0,
            _ => f.p.iuser4 & !(stunt::DIVE_IMPULSE_MASK | stunt::WALLJUMP_IMPULSE),
        };
        f.p.fuser1 = if (700..800).contains(&i) { 0.37 } else { 1.0 };
        f.p.friction = if (1000..1100).contains(&i) { 0.3 } else { 1.0 };
        if i == 1100 {
            f.p.origin = [10.0, -20.0, 300.0];
            f.p.velocity = [123.4, -56.7, 89.1];
        }
        if i == 1350 {
            f.p.iuser1 = crate::ts::obs::ROAMING;
        }
        mv(&mut f.p);
        let p = &*f.p;
        for v in [p.origin, p.velocity, p.view_ofs, p.angles, p.punchangle, p.basevelocity] {
            v.iter().for_each(|c| mix(c.to_bits()));
        }
        mix(p.flDuckTime.to_bits());
        mix(p.flFallVelocity.to_bits());
        mix(p.frametime.to_bits());
        mix(p.flags as u32);
        mix(p.iuser4 as u32);
        mix(p.onground as u32);
        mix(p.usehull as u32);
        mix(p.flTimeStepSound as u32);
    }
    h
}

/// `ts_i386.so`'s arithmetic (the server's `ts_pm_arith gcc`), pinned when the movement became
/// generic over the build.
#[test]
fn server_trajectory_is_unchanged() {
    let h = trajectory_hash(|p| player_move(p, Arith::Gcc, &RULES));
    assert_eq!(h, 0x3bb0_c84b_4c23_562f, "{h:#x}");
}

/// Runs `body` with the x87 control word set to `cw`, restoring the old one after.
#[cfg(target_arch = "x86_64")]
fn with_x87_cw<T>(cw: u16, body: impl FnOnce() -> T) -> T {
    use std::arch::asm;
    let mut old = 0u16;
    // SAFETY: stores and loads the x87 control word through valid pointers.
    unsafe { asm!("fnstcw word ptr [{o}]", "fldcw word ptr [{n}]", o = in(reg) &mut old as *mut u16, n = in(reg) &cw as *const u16) };
    let r = body();
    // SAFETY: as above.
    unsafe { asm!("fldcw word ptr [{o}]", o = in(reg) &old as *const u16) };
    r
}

/// The premise of `Msvc`: at the precision MSVC's CRT sets (`_PC_53`, control word 0x27f) the
/// x87's add, sub, mul, div and sqrt round exactly like `f64`, even on operands that are
/// themselves unrounded results.
#[cfg(target_arch = "x86_64")]
#[test]
fn x87_at_53_bits_is_f64() {
    use std::arch::asm;

    fn hw(a: f64, b: f64, op: u8) -> f64 {
        let mut o = 0f64;
        let (pa, pb, po) = (&a as *const f64, &b as *const f64, &mut o as *mut f64);
        macro_rules! run {
            ($ins:literal) => {
                // SAFETY: loads two doubles and stores one; the x87 stack is left empty.
                unsafe {
                    asm!(
                        "fld qword ptr [{b}]", "fld qword ptr [{a}]", $ins, "fstp qword ptr [{o}]", "fstp st(0)",
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

    let mut seed = 0x0fed_cba9_8765_4321u64;
    let mut rnd = || {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        seed
    };
    let mut float = || {
        let b = rnd();
        let exp = 100 + (b >> 40) as u32 % 56;
        f32::from_bits(((b >> 63) as u32) << 31 | exp << 23 | (b as u32 & 0x7f_ffff)) as f64
    };
    with_x87_cw(0x27f, || {
        for i in 0..20_000 {
            let a = float() * float() + float();
            let b = if i % 3 == 0 { [0.001, 0.333, 0.8, 0.7, 0.65, 0.3, 1.5][i % 7] } else { float() / float() };
            for (op, ours) in [a + b, a - b, a * b, a / b, a.abs().sqrt()].into_iter().enumerate() {
                let src = if op == 4 { a.abs() } else { a };
                assert_eq!(ours.to_bits(), hw(src, b, op as u8).to_bits(), "op {op} on {a:e} {b:e}");
            }
        }
    });
}

/// `Msvc::angle_vectors` against client.dll's `AngleVectors` (0x10029bb0) itself: its
/// instructions, run on the FPU at 53-bit precision. The only gap is `fsin`/`fcos`, which the
/// port takes from `f64` (the FPU's own results carry 64 bits): it could flip the last bit of a
/// component, though none of these 180 000 components differ.
#[cfg(target_arch = "x86_64")]
#[test]
fn msvc_angle_vectors_match_client_dll() {
    use std::arch::asm;

    fn hw(angles: Vec3) -> [f32; 9] {
        let mut out = [0f32; 9];
        let mut s = [0f32; 6];
        let k: (f64, f32) = (0.017_453_292_519_943_295, -1.0);
        // SAFETY: client.dll 0x10029bb0's body with its stack slots moved to `s` (sr, cr, cp, sp,
        // -, sy) and its constants to `k`; it reads `angles` and writes `out`, and leaves the x87
        // stack empty.
        unsafe {
            asm!(
                "fld dword ptr [{a}+4]", "fmul qword ptr [{k}]", "fld st(0)", "fsin", "fstp dword ptr [{s}+20]", "fcos",
                "fld dword ptr [{a}]", "fmul qword ptr [{k}]", "fld st(0)", "fsin", "fstp dword ptr [{s}+12]", "fcos",
                "fstp dword ptr [{s}+8]",
                "fld dword ptr [{a}+8]", "fmul qword ptr [{k}]", "fld st(0)", "fsin", "fstp dword ptr [{s}]", "fcos",
                "fstp dword ptr [{s}+4]",
                // forward
                "fld dword ptr [{s}+8]", "fmul st, st(1)", "fstp dword ptr [{o}]",
                "fld dword ptr [{s}+8]", "fmul dword ptr [{s}+20]", "fstp dword ptr [{o}+4]",
                "fld dword ptr [{s}+12]", "fchs", "fstp dword ptr [{o}+8]",
                // right (`.byte 0xde, 0xe9` is Intel's `fsubp st(1), st`: st(1) - st)
                "fld dword ptr [{s}]", "fmul dword ptr [{s}+12]", "fld dword ptr [{s}+4]", "fmul dword ptr [{s}+20]",
                "fld st(1)", "fmul st, st(3)", ".byte 0xde, 0xe9", "fstp dword ptr [{o}+12]",
                "fmul dword ptr [{s}+20]", "fld dword ptr [{s}+4]", "fmul st, st(2)", "faddp st(1), st",
                "fmul dword ptr [{k}+8]", "fstp dword ptr [{o}+16]",
                "fld dword ptr [{s}]", "fmul dword ptr [{s}+8]", "fmul dword ptr [{k}+8]", "fstp dword ptr [{o}+20]",
                // up
                "fld dword ptr [{s}+4]", "fmul dword ptr [{s}+12]", "fld st(0)", "fmul st, st(2)",
                "fld dword ptr [{s}]", "fmul dword ptr [{s}+20]", "faddp st(1), st", "fstp dword ptr [{o}+24]",
                "fmul dword ptr [{s}+20]", "fld dword ptr [{s}]", "fmul st, st(2)", ".byte 0xde, 0xe9",
                "fstp dword ptr [{o}+28]", "fstp st(0)",
                "fld dword ptr [{s}+4]", "fmul dword ptr [{s}+8]", "fstp dword ptr [{o}+32]",
                a = in(reg) angles.as_ptr(), k = in(reg) &k as *const (f64, f32), s = in(reg) s.as_mut_ptr(),
                o = in(reg) out.as_mut_ptr(),
                out("st(0)") _, out("st(1)") _, out("st(2)") _, out("st(3)") _,
                out("st(4)") _, out("st(5)") _, out("st(6)") _, out("st(7)") _,
            )
        }
        out
    }

    let mut seed = 0x5eed_u64;
    let mut angle = || {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        ((seed >> 40) as f32 / (1u64 << 24) as f32) * 720.0 - 360.0
    };
    let (mut n, mut flips) = (0, 0);
    with_x87_cw(0x27f, || {
        for _ in 0..20_000 {
            let a = [angle(), angle(), if n % 4 == 0 { 0.0 } else { angle() }];
            let (f, r, u) = Msvc::angle_vectors(a);
            let ours = [f, r, u].concat();
            for (o, h) in ours.iter().zip(hw(a)) {
                n += 1;
                let d = (o.to_bits() as i64 - h.to_bits() as i64).abs();
                assert!(d <= 1, "{a:?}: {ours:?} vs {:?}", hw(a));
                flips += d as usize;
            }
        }
    });
    assert!(flips * 10_000 < n, "{flips} of {n} components off by one ulp");
}

/// `Pm<Msvc>::friction` against client.dll's `PM_Friction` tail (0x1002ba27..0x1002baa4) run on
/// the FPU at 53-bit precision: the drop and the scale stay on the stack, and only the x
/// component uses the unrounded scale.
#[cfg(target_arch = "x86_64")]
#[test]
fn msvc_friction_matches_client_dll() {
    use std::arch::asm;

    /// `[mvfric, pfric, control, frametime, speed, v0, v1, v2]` -> velocity.
    fn hw(i: [f32; 8]) -> Vec3 {
        let (mut o, mut ns) = ([0f32; 3], 0f32);
        let c075 = 0.75f32;
        // SAFETY: the instructions from client.dll with `pmove` fields and locals moved to `i`, `o`
        // and `ns`; the x87 stack is left empty.
        unsafe {
            asm!(
                "fld dword ptr [{i}]", "fmul dword ptr [{i}+4]", "fmul dword ptr [{c}]",
                "fld dword ptr [{i}+8]", "fmul dword ptr [{i}+12]", "fmulp st(1), st",
                "fsubr dword ptr [{i}+16]", "fdiv dword ptr [{i}+16]", "fst dword ptr [{ns}]",
                "fmul dword ptr [{i}+20]", "fld dword ptr [{ns}]", "fmul dword ptr [{i}+24]", "fstp dword ptr [{o}+4]",
                "fld dword ptr [{ns}]", "fmul dword ptr [{i}+28]", "fstp dword ptr [{o}+8]", "fstp dword ptr [{o}]",
                i = in(reg) i.as_ptr(), c = in(reg) &c075 as *const f32, ns = in(reg) &mut ns as *mut f32,
                o = in(reg) o.as_mut_ptr(),
                out("st(0)") _, out("st(1)") _, out("st(2)") _, out("st(3)") _,
                out("st(4)") _, out("st(5)") _, out("st(6)") _, out("st(7)") _,
            )
        }
        o
    }

    let mut f = Fixture::standing();
    let mut seed = 0xf00d_u64;
    let mut unit = || {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        (seed >> 40) as f32 / (1u64 << 24) as f32
    };
    let mut differs_from_gcc = 0;
    for _ in 0..20_000 {
        let v = [unit() * 600.0 - 300.0, unit() * 600.0 - 300.0, 0.0];
        let (pfric, ft) = (0.2 + unit() * 1.3, 0.001 + unit() * 0.02);
        let speed = ((v[0] as f64 * v[0] as f64 + v[1] as f64 * v[1] as f64) + 0.0).sqrt() as f32;
        if speed < 0.1 {
            continue;
        }
        let control = if speed >= f.mv.stopspeed { speed } else { f.mv.stopspeed };
        if (f.mv.friction * pfric * 0.75 * control * ft) as f64 >= speed as f64 * 0.99 {
            continue; // stays clear of the clamp to 0, which the tail above leaves out
        }
        let run = |f: &mut Fixture, arith: Arith| {
            f.p.velocity = v;
            f.p.friction = pfric;
            f.p.frametime = ft;
            f.p.movevars = &mut *f.mv;
            CURRENT.with(|c| c.set(&*f.p));
            match arith {
                Arith::Gcc => Pm::<Gcc> { p: &mut f.p, onladder: false, spectator: None, rules: &RULES, fpu: PhantomData }.friction(),
                Arith::Msvc => Pm::<Msvc> { p: &mut f.p, onladder: false, spectator: None, rules: &RULES, fpu: PhantomData }.friction(),
            }
            f.p.velocity
        };
        let ours = run(&mut f, Arith::Msvc);
        let dll = with_x87_cw(0x27f, || hw([f.mv.friction, pfric, control, ft, speed, v[0], v[1], v[2]]));
        assert_eq!(ours.map(f32::to_bits), dll.map(f32::to_bits), "{v:?} pfric {pfric} ft {ft}");
        differs_from_gcc += (run(&mut f, Arith::Gcc) != dll) as u32;
    }
    assert!(differs_from_gcc > 0, "the test should tell the builds apart");
}

/// A ruleset with the other SDK operation orders and in-eye pitch
/// (`PmRules::roll_divides_first`, `friction_edge_by_reciprocal`, `in_eye_pitch`).
static REORDERED: Rules = Rules { in_eye_pitch: |pitch| pitch * -2.0, roll_divides_first: true, friction_edge_by_reciprocal: true, ..BASE };

#[test]
fn in_eye_pitch_follows_the_rules() {
    let mut f = Fixture::standing();
    f.p.movevars = &mut *f.mv;
    CURRENT.with(|c| c.set(&*f.p));
    f.p.numphysent = 2;
    f.p.physents[1].info = 2;
    f.p.physents[1].angles = [-4.5, 90.0, 1.0];
    f.p.iuser1 = crate::ts::obs::IN_EYE;
    f.p.iuser2 = 2;
    player_move(&mut f.p, Arith::Msvc, &REORDERED);
    assert_eq!(f.p.angles, [9.0, 90.0, 1.0]);
    player_move(&mut f.p, Arith::Msvc, &RULES);
    assert_eq!(f.p.angles, [-4.5, 90.0, 1.0], "3.0 copies the target's angles");
}

#[test]
fn calc_roll_order_follows_the_rules() {
    // Yaw 0: right is -y, so the side speed is -vy.
    let (ra, rs) = (2.7f32, 300.0f32);
    let mut differ = 0;
    for i in 1..3000 {
        let side = i as f32 * 0.0937;
        let v = [0.0, -side, 0.0];
        let div_first = calc_roll::<Msvc>(ZERO, v, ra, rs, true);
        let mul_first = calc_roll::<Msvc>(ZERO, v, ra, rs, false);
        assert_eq!(div_first, side as f64 / rs as f64 * ra as f64);
        assert_eq!(mul_first, side as f64 * ra as f64 / rs as f64);
        // Last-bit differences on the FPU stack; the stored `roll * 4` rarely shows them.
        differ += (div_first != mul_first) as u32;
    }
    assert!(differ > 0, "the orders differ in the last bit for some speeds");
}

#[test]
fn friction_edge_start_order_follows_the_rules() {
    thread_local!(static START: Cell<Vec3> = const { Cell::new([0.0; 3]) });
    unsafe extern "C" fn record(start: *mut f32, end: *mut f32, flags: i32, ig: i32) -> pmtrace_t {
        let s = unsafe { [*start, *start.add(1), *start.add(2)] };
        let e = unsafe { [*end, *end.add(1), *end.add(2)] };
        if s[0] == e[0] && s[1] == e[1] && e[2] < s[2] - 30.0 {
            START.with(|c| c.set(s));
        }
        unsafe { trace(start, end, flags, ig) }
    }
    let mut f = Fixture::standing();
    f.p.PM_PlayerTrace = Some(record);
    f.p.origin = [12.345_678, -98.765_43, f.p.origin[2]];
    let origin = f.p.origin;
    let mut differ = 0;
    for i in 0..500 {
        let v = [37.0 + i as f32 * 0.731, -120.0 + i as f32 * 0.377, 0.0];
        let speed = ((v[0] as f64 * v[0] as f64 + v[1] as f64 * v[1] as f64) + 0.0).sqrt() as f32;
        let mut run = |rules: &'static dyn crate::gameplay::PmRules| {
            f.p.velocity = v;
            f.p.origin = origin;
            f.p.movevars = &mut *f.mv;
            CURRENT.with(|c| c.set(&*f.p));
            Pm::<Msvc> { p: &mut f.p, onladder: false, spectator: None, rules, fpu: PhantomData }.friction();
            START.with(|c| c.get())
        };
        let recip = run(&REORDERED);
        let core = run(&RULES);
        let sp = speed as f64;
        for k in 0..2 {
            let r = 1.0 / sp * v[k] as f64 * 16.0 + origin[k] as f64;
            let c = v[k] as f64 / sp * 16.0 + origin[k] as f64;
            assert_eq!(edge_ahead::<Msvc>(v[k], sp, origin[k], true), r);
            assert_eq!(edge_ahead::<Msvc>(v[k], sp, origin[k], false), c);
            // The stored start: the store to a float rarely shows the last-bit difference.
            assert_eq!((recip[k], core[k]), (r as f32, c as f32));
            differ += (r != c) as u32;
        }
    }
    assert!(differ > 0, "the orders differ in the last bit for some velocities");
}

// ---- issue #105: IEEE compares (`PmRules::ieee_compares`) -----------------------------------------

/// A ruleset whose `mp.dll` compares through the parity flag (`PmRules::ieee_compares`).
static IEEE: Rules = Rules { ieee_compares: true, ..BASE };

/// A `Pm` over the fixture in build `F` under the core rules.
fn pm_in<F: Fpu>(f: &mut Fixture) -> Pm<'_, F> {
    f.p.movevars = &mut *f.mv;
    CURRENT.with(|c| c.set(&*f.p));
    Pm::<F> { p: &mut f.p, onladder: false, spectator: None, rules: &RULES, fpu: PhantomData }
}

#[test]
fn compares_per_build() {
    let nan = f32::NAN;
    // TS 3.0's builds test C3/C0: a NaN is equal and below.
    assert!(Msvc::cmp_eq(nan, 0.0) && Msvc::cmp_lt(nan, 0.0) && Msvc::cmp_le(nan, 0.0) && Msvc::cmp_lt(0.0, nan));
    assert!(Gcc::cmp_eq(nan, 0.0) && Gcc::cmp_lt(nan, 0.0) && Gcc::cmp_le(nan, 0.0));
    // The parity-flag build: the C operator's IEEE result.
    assert!(!MsvcIeee::cmp_eq(nan, 0.0) && !MsvcIeee::cmp_lt(nan, 0.0) && !MsvcIeee::cmp_le(nan, 0.0) && !MsvcIeee::cmp_lt(0.0, nan));
    // Ordered operands: every build agrees with the C operator.
    for (a, b) in [(1.0f32, 2.0f32), (2.0, 1.0), (1.0, 1.0), (-0.0, 0.0), (f32::INFINITY, 1.0)] {
        for r in [
            (Msvc::cmp_eq(a, b), Msvc::cmp_lt(a, b), Msvc::cmp_le(a, b)),
            (MsvcIeee::cmp_eq(a, b), MsvcIeee::cmp_lt(a, b), MsvcIeee::cmp_le(a, b)),
        ] {
            assert_eq!(r, (a == b, a < b, a <= b), "{a} {b}");
        }
    }
    // The same arithmetic as `Msvc`, and hooks see `Arith::Msvc`.
    const { assert!(MsvcIeee::MSVC && MsvcIeee::IEEE_CMP && !Msvc::IEEE_CMP && !Gcc::IEEE_CMP) };
    assert_eq!(MsvcIeee::ARITH, Arith::Msvc);
}

#[test]
fn nan_waterjumptime_follows_the_rules() {
    let run = |arith: Arith, rules: &'static dyn crate::gameplay::PmRules| {
        // In the air: on the ground `PM_CatagorizePosition` would zero the time first.
        let mut f = Fixture::new();
        f.p.origin = [0.0, 0.0, 100.0];
        f.arith = arith;
        f.p.waterjumptime = f32::NAN;
        f.p.movedir = [5.0, 7.0, 0.0];
        f.run_with(1, rules);
        [f.p.velocity[0], f.p.velocity[1], f.p.waterjumptime]
    };
    // IEEE: `if (pmove->waterjumptime)` holds, so the player moves along `movedir`, and
    // `PM_WaterJump` ends the water jump out of the water.
    assert_eq!(run(Arith::Msvc, &IEEE), [5.0, 7.0, 0.0]);
    // TS 3.0 (C3 counts the NaN as 0): a normal air move without input, and the NaN stays.
    let [vx, vy, t] = run(Arith::Msvc, &RULES);
    assert!(vx == 0.0 && vy == 0.0 && t.is_nan());
    // `ts_i386.so`'s compares are kept under any rules.
    let bits = |r: [f32; 3]| r.map(f32::to_bits);
    assert_eq!(bits(run(Arith::Gcc, &IEEE)), bits(run(Arith::Gcc, &RULES)));
}

#[test]
fn accelerate_nan_tests_follow_the_build() {
    fn accel<F: Fpu>(waterjumptime: f32, wishspeed: f32) -> Vec3 {
        let mut f = Fixture::standing();
        f.p.waterjumptime = waterjumptime;
        f.p.velocity = ZERO;
        pm_in::<F>(&mut f).accelerate([1.0, 0.0, 0.0], wishspeed, 10.0);
        f.p.velocity
    }
    // `waterjumptime` (2.1 0x1003160a `test ah,0x44; jp`, 3.0 0x10037de3 `test ah,0x40; je`):
    // IEEE returns, 3.0 accelerates.
    assert_eq!(accel::<MsvcIeee>(f32::NAN, 100.0), ZERO);
    assert!(accel::<Msvc>(f32::NAN, 100.0)[0] > 0.0);
    // `addspeed <= 0` (0x10031644 `test ah,0x41; jnp`, 0x10037e15 `jne`): IEEE goes on.
    assert!(accel::<MsvcIeee>(0.0, f32::NAN)[0].is_nan());
    assert_eq!(accel::<Msvc>(0.0, f32::NAN), ZERO);
    // Finite values: the same arithmetic.
    assert_eq!(accel::<MsvcIeee>(0.0, 100.0), accel::<Msvc>(0.0, 100.0));
}

#[test]
fn jump_nan_tests_follow_the_build() {
    fn jump<F: Fpu>(waterjumptime: f32, waterlevel: i32, swimtime: f32) -> (f32, f32, u32) {
        let mut f = Fixture::standing();
        f.p.waterjumptime = waterjumptime;
        f.p.waterlevel = waterlevel;
        f.p.watertype = CONTENTS_WATER;
        f.p.flSwimTime = swimtime;
        f.p.cmd.msec = 10;
        SOUNDS.with(|c| c.set(0));
        pm_in::<F>(&mut f).jump();
        (f.p.velocity[2], f.p.flSwimTime, SOUNDS.with(|c| c.get()))
    }
    // `waterjumptime != 0` (0x100586ca `jnp`, 3.0 0x100651fa `test ah,0x40; jne`): IEEE counts
    // the water jump down and doesn't jump, 3.0 jumps off the ground.
    assert_eq!(jump::<MsvcIeee>(f32::NAN, 0, 0.0).0, 0.0);
    assert!(jump::<Msvc>(f32::NAN, 0, 0.0).0 > 200.0);
    // `flSwimTime <= 0` (0x10058775 `test ah,0x41; jp`, 0x100652a5 `je`): only 3.0 wades.
    let (vz, swim, sounds) = jump::<MsvcIeee>(0.0, 2, f32::NAN);
    assert!(vz == 100.0 && swim.is_nan() && sounds == 0);
    assert_eq!(jump::<Msvc>(0.0, 2, f32::NAN), (100.0, 1000.0, 1));
}

#[test]
fn normalize_nan_follows_the_build() {
    // `VectorNormalize`'s `if (length)` (2.1 0x1002fc3c `test ah,0x44; jnp`): IEEE normalises
    // a NaN length, 3.0's MSVC skips it.
    let mut v = [f32::NAN, 1.0, 0.0];
    assert!(MsvcIeee::normalize(&mut v).is_nan() && v[1].is_nan());
    let mut v = [f32::NAN, 1.0, 0.0];
    Msvc::normalize(&mut v);
    assert_eq!(v[1], 1.0);
    let mut v = [3.0, 4.0, 0.0];
    assert_eq!(MsvcIeee::normalize(&mut v), 5.0);
    assert_eq!(v, [0.6, 0.8, 0.0]);
}

#[test]
fn pseudo_climb_nan_pitch_follows_the_build() {
    // A NaN pitch: 3.0 returns at `pitch < -60` (0x10064a14 `test ah,0x1; jne`); 2.1 doesn't
    // (0x10057d84 `test ah,0x5; jnp`, 0x10057d98 `and eax,0x4100; je`) and ends with
    // `PM_CatagorizePosition`, which puts the player 1 unit above the floor on the ground.
    fn climb<F: Fpu>() -> i32 {
        let mut f = Fixture::new();
        f.p.origin = [0.0, 0.0, 37.0];
        f.p.angles[0] = f32::NAN;
        f.p.cmd.buttons = IN_JUMP as u16;
        pm_in::<F>(&mut f).ts_handle();
        f.p.onground
    }
    assert_eq!(climb::<MsvcIeee>(), 0);
    assert_eq!(climb::<Msvc>(), -1);
}
