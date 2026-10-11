//! `CBotNav` (bots.md §11.7): walks a node path to the brain's goal. Replans whenever the goal
//! changes, gives up when stuck, probes for jumps and ducks, steers around obstacles, and climbs
//! ladders. Movement is relative to the body yaw, so the bot can strafe or back up while it looks
//! elsewhere.

use super::brain::{Brain, add, bit, btn, is_zero, length, scale, slow, sub};
use super::nodes;
use crate::player::Player;
use crate::world::Cx;
use goldsrc::edict::EdictPtr;
use goldsrc::server::{IGNORE_MONSTERS, engine};
use goldsrc::sys::*;

/// `Navigate` results: the path is done / nothing to do (the brain treats both alike) / walking.
pub const ARRIVED: i32 = 0;
pub const FAILED: i32 = 1;
pub const WALKING: i32 = 2;

/// `CBotNav` (brain+0x28, 0x1860 bytes).
#[derive(Clone, Debug, Default)]
pub struct Nav {
    /// +0x0 next think, +0x4 next stuck check, +0x8 next jump/duck probe, +0xc next steer.
    pub next_think: f32,
    pub stuck_time: f32,
    pub probe_time: f32,
    pub steer_time: f32,
    /// +0x10: the current waypoint, zero = none.
    pub waypoint: [f32; 3],
    /// +0x1c: the goal the path was planned for.
    pub planned_goal: [f32; 3],
    /// +0x28: where the last stuck check found the bot.
    pub stuck_pos: [f32; 3],
    /// +0x34: zeroed at spawn, otherwise unused.
    pub unused_34: [f32; 3],
    /// +0x40: set by `RandomWanderPoint`, never read.
    pub wander_anchor: [f32; 3],
    /// +0x4c: a detour around an obstacle, zero = none.
    pub detour: [f32; 3],
    /// +0x58: the path (node origins, the bot's end first); +0x1858 its length, +0x185c the index.
    pub path: Vec<[f32; 3]>,
    pub path_count: i32,
    pub path_index: i32,
}

impl Nav {
    /// `CBotNav::Reset` (0x1006f810), from `CBotBrain::Reset`. The stuck timer and position, the
    /// probe timer and the detour survive.
    pub fn reset(&mut self) {
        self.next_think = 0.0;
        self.steer_time = 0.0;
        self.waypoint = [0.0; 3];
        self.planned_goal = [0.0; 3];
        self.unused_34 = [0.0; 3];
        self.wander_anchor = [0.0; 3];
        self.path.clear();
        self.path_count = 0;
        self.path_index = 0;
    }

    /// `path[i]`: zero past the stored nodes (TS clears the whole array on a replan).
    fn node(&self, i: i32) -> [f32; 3] {
        usize::try_from(i).ok().and_then(|i| self.path.get(i)).copied().unwrap_or([0.0; 3])
    }
}

/// A trace that ignores monsters and the bot: clear all the way.
fn clear(from: [f32; 3], to: [f32; 3], me: *mut edict_t) -> bool {
    engine().trace_line(from, to, IGNORE_MONSTERS, me).flFraction >= 1.0
}

/// `min(3 / slow, 9)`: the stuck check window.
fn stuck_window(now: f32, slow: f32) -> f32 {
    let x = 3.0 / slow as f64;
    (now as f64 + if x >= 9.0 { 9.0 } else { x }) as f32
}

/// `CBasePlayer::IsOnLadder` (0x10033670): `movetype == MOVETYPE_FLY`.
pub fn on_ladder(v: &entvars_t) -> bool {
    v.movetype == MOVETYPE_FLY as i32
}

/// `CBotNav::Navigate` (0x1006fc00), every 0.1 s (frames in between keep the last move).
pub fn navigate(b: &mut Brain, p: &mut Player, cx: &mut Cx, goal: [f32; 3]) -> i32 {
    if is_zero(goal) {
        return FAILED;
    }
    let now = cx.time();
    if now < b.nav.next_think {
        return WALKING;
    }
    b.nav.next_think = (now as f64 + 0.1) as f32;
    if b.nav.planned_goal != goal {
        replan(b, cx, goal); // always returns 3, so TS's "== 1" exit is dead
    }
    let me = cx.edict;
    let origin = cx.pev().origin;
    if is_zero(b.nav.detour) {
        let wp = b.nav.waypoint;
        if length(sub(wp, origin)) < 48.0 && clear(origin, wp, me) {
            b.nav.path_index += 1;
            if b.nav.path_count <= b.nav.path_index {
                b.set_move(0.0, 0.0);
                b.nav.waypoint = [0.0; 3];
                b.nav.planned_goal = [0.0; 3];
                return ARRIVED;
            }
            b.nav.waypoint = b.nav.node(b.nav.path_index);
            if is_zero(b.nav.waypoint) {
                b.set_move(0.0, 0.0);
                b.nav.planned_goal = [0.0; 3];
                return FAILED;
            }
        }
    } else if length(sub(b.nav.detour, origin)) < 48.0 && clear(origin, b.nav.detour, me) {
        b.nav.detour = [0.0; 3]; // reached; no waypoint advance this tick
    }

    if b.nav.stuck_time < now {
        b.nav.stuck_time = stuck_window(now, slow(cx.pev()));
        if length(sub(b.nav.stuck_pos, origin)) < 96.0 {
            b.nav.waypoint = [0.0; 3];
            b.set_move(0.0, 0.0);
            b.nav.detour = [0.0; 3];
            b.nav.planned_goal = [0.0; 3];
            return FAILED;
        }
        b.nav.stuck_pos = origin;
    }

    if b.nav.probe_time < now {
        // Blocked at the eyes only: duck; at the feet only: jump (and tuck the legs).
        b.nav.probe_time = (now as f64 + 0.5) as f32;
        let eng = engine();
        let (f, _, _) = eng.angle_vectors(eng.vec_to_angles(sub(b.nav.waypoint, origin)));
        let v = cx.pev();
        let eye = add(origin, v.view_ofs);
        let eye_clear = clear(eye, add(add(origin, v.view_ofs), scale(f, 64.0)), me);
        let feet_clear = clear(origin, add(scale(f, 64.0), origin), me);
        if eye_clear {
            if !feet_clear {
                b.press(btn::JUMP, 1.0, now);
                b.press(btn::DUCK, 0.5, now);
            }
        } else if feet_clear {
            b.press(btn::DUCK, 1.0, now);
        }
    }

    if on_ladder(cx.pev()) {
        if 0.0 <= cx.pev().origin[2] - b.nav.waypoint[2] {
            b.press(btn::DUCK | btn::BACK, 1.0, now);
            b.press(btn::JUMP | btn::FORWARD, 0.0, now);
        } else {
            b.press(btn::DUCK | btn::FORWARD, 1.0, now);
            b.press(btn::JUMP | btn::BACK, 0.0, now);
        }
        b.set_move(0.0, 0.0);
        let wp = b.nav.waypoint;
        b.look_at(cx.pev(), wp);
        b.disable(bit::LOOK_AROUND);
    }

    let steer = steer_point(b, cx);
    let eng = engine();
    let yaw = eng.vec_to_angles(sub(steer, cx.pev().origin))[1];
    let a = super::brain::angle_mod(cx.pev().angles[1]);
    let bb = super::brain::angle_mod(yaw);
    let mut d = (bb as f64 - a as f64).abs();
    if bb <= a {
        d = (360.0 - d).abs();
    }
    let rad = d * 0.017_453_292_519_943_295;
    let speed = p.player_speed as f64;
    b.set_move((rad.cos() * speed) as f32, -(rad.sin() * speed) as f32);
    WALKING
}

/// `CBotNav::Replan` (0x1006fa40): a fresh plan for `goal`. The attack module's goals go straight
/// (no path: the first arrival ends it); otherwise a node path from the node nearest the bot to
/// the node nearest the goal, or a random point 1024..4096 units away when there is none.
fn replan(b: &mut Brain, cx: &mut Cx, goal: [f32; 3]) {
    let now = cx.time();
    b.nav.stuck_pos = [0.0; 3];
    b.nav.stuck_time = stuck_window(now, slow(cx.pev()));
    b.nav.probe_time = (now as f64 + 0.5) as f32;
    b.nav.path.clear();
    b.nav.path_index = 0;
    b.nav.path_count = 0;
    b.nav.planned_goal = [0.0; 3];
    b.nav.waypoint = [0.0; 3];
    b.nav.detour = [0.0; 3];
    if b.go_straight {
        b.nav.waypoint = goal;
        b.nav.planned_goal = goal;
        b.go_straight = false;
        return;
    }
    let origin = cx.pev().origin;
    let me = cx.edict;
    let debug = crate::rules::cvars::f(c"botlearn") == 2.0;
    let mut path = Vec::new();
    let found = nodes::with(|g| g.find_path_between_points(&mut nodes::EngineWorld, origin, goal, &mut path, me, debug));
    if found {
        b.nav.path_count = path.len() as i32;
        b.nav.path = path;
        b.nav.waypoint = b.nav.node(0);
    } else {
        b.nav.waypoint = random_wander_point(b, origin);
    }
    b.nav.planned_goal = goal;
}

/// `RandomWanderPoint` (0x1006f8c0): 22..352° in 22° steps, 1024..4096 units, where a trace
/// from the bot ends.
fn random_wander_point(b: &mut Brain, origin: [f32; 3]) -> [f32; 3] {
    let eng = engine();
    let r = eng.random_long(1, 16);
    if is_zero(b.nav.wander_anchor) {
        b.nav.wander_anchor = origin;
    }
    let ang = (r * 22) as f64 * 0.017_453_292_519_943_295;
    let (c, s) = (ang.cos() as f32, ang.sin());
    let l = ((c as f64) * (c as f64) + s * s).sqrt();
    let dir = if l == 0.0 { [0.0, 0.0, 1.0] } else { [((1.0 / l) * c as f64) as f32, ((1.0 / l) * s) as f32, 0.0] };
    let dist = eng.random_long(1024, 4096) as f32;
    let end = add(scale(dir, dist), origin);
    eng.trace_line(origin, end, IGNORE_MONSTERS, std::ptr::null_mut()).vecEndPos
}

/// The door test of `GetSteerPoint` and `LinkNewNode`: `func_door` or `func_door_rotating`.
fn is_door(e: *mut edict_t) -> bool {
    if e.is_null() || e.is_free() {
        return false;
    }
    let c = engine().string(e.v().classname);
    c == c"func_door" || c == c"func_door_rotating"
}

/// `CBotNav::GetSteerPoint` (0x10070400): every 0.5 s, three traces that hit players too (24
/// units ahead, and the parallel lines 16 units right and left of the line to the waypoint);
/// anything but a door blocking one gives a detour point stepped back or aside. In between, the
/// cached detour or the waypoint. An older detour isn't cleared when all three are clear.
fn steer_point(b: &mut Brain, cx: &mut Cx) -> [f32; 3] {
    let now = cx.time();
    if now < b.nav.steer_time {
        return if is_zero(b.nav.detour) { b.nav.waypoint } else { b.nav.detour };
    }
    b.nav.steer_time = (now as f64 + 0.5) as f32;
    let eng = engine();
    let me = cx.edict;
    let origin = cx.pev().origin;
    let wp = b.nav.waypoint;
    let (f, r, _) = eng.angle_vectors(eng.vec_to_angles(sub(wp, origin)));
    let tr = eng.trace_line(origin, add(scale(f, 24.0), origin), 0, me);
    if tr.flFraction < 1.0 && !is_door(tr.pHit) {
        b.nav.detour = sub(tr.vecEndPos, scale(f, 48.0));
        return b.nav.detour;
    }
    let right = scale(r, 16.0);
    let tr = eng.trace_line(add(right, origin), add(right, wp), 0, me);
    if tr.flFraction < 1.0 && !is_door(tr.pHit) {
        b.nav.detour = sub(tr.vecEndPos, scale(r, 48.0)); // step left
        return b.nav.detour;
    }
    let tr = eng.trace_line(sub(origin, right), sub(wp, right), 0, me);
    if tr.flFraction < 1.0 && !is_door(tr.pHit) {
        b.nav.detour = add(tr.vecEndPos, scale(r, 48.0)); // step right
        return b.nav.detour;
    }
    wp
}
