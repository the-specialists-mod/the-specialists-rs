//! `CBotNav` (bots.md §11.7): walks a node path to the brain's goal. Replans whenever the goal
//! changes, gives up when stuck, probes for jumps and ducks, steers around obstacles, and climbs
//! ladders. Movement is relative to the body yaw, so the bot can strafe or back up while it looks
//! elsewhere.

use super::brain::{Brain, add, bit, btn, diff_x87, is_zero, len_x87, scale, sub};
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
fn stuck_window(now: f32, pev: &entvars_t) -> f32 {
    // `min(3 / slow, 9)`: the modules' timer without the skill (dividing by 1 is exact).
    super::rules::delay(now, 3.0, pev, 1.0, 9.0)
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
    let rules = super::rules::current();
    let dist_order = rules.nav_distance_order();
    if is_zero(b.nav.detour) {
        let wp = b.nav.waypoint;
        if len_x87(diff_x87(wp, origin), dist_order) < 48.0 && clear(origin, wp, me) {
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
    } else if len_x87(diff_x87(b.nav.detour, origin), dist_order) < 48.0 && clear(origin, b.nav.detour, me) {
        b.nav.detour = [0.0; 3]; // reached; no waypoint advance this tick
    }

    if b.nav.stuck_time < now {
        b.nav.stuck_time = stuck_window(now, cx.pev());
        if len_x87(diff_x87(b.nav.stuck_pos, origin), dist_order) < 96.0 {
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
        let eye_clear = clear(eye, eye_probe_end(origin, v.view_ofs, f), me);
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
        let down = 0.0 <= cx.pev().origin[2] - b.nav.waypoint[2];
        super::rules::current().ladder_buttons(b, down, now);
        b.set_move(0.0, 0.0);
        let wp = b.nav.waypoint;
        b.look_at(cx.pev(), wp);
        b.disable(bit::LOOK_AROUND);
    }

    let steer = steer_point(b, cx);
    let eng = engine();
    let yaw = eng.vec_to_angles(sub(steer, cx.pev().origin))[1];
    let speed = p.player_speed as f64;
    let (forward, side) = move_toward(cx.pev().angles[1], yaw, speed);
    b.set_move(forward, side);
    WALKING
}

/// `Navigate`'s eye probe end, `origin + view_ofs + v_forward * 64`, as both TS `mp.dll`s
/// compute it (3.0 0x10070105..0x1007014e, 2.1 0x10062070..0x10062095): the x sum
/// `origin.x + view_ofs.x` stays on the x87 stack for `+ v_forward.x * 64` and is rounded once;
/// the y and z sums are stored as floats first.
fn eye_probe_end(origin: [f32; 3], view_ofs: [f32; 3], f: [f32; 3]) -> [f32; 3] {
    let x = (f64::from(origin[0]) + f64::from(view_ofs[0]) + f64::from(f[0]) * 64.0) as f32;
    [x, (origin[1] + view_ofs[1]) + f[1] * 64.0, (origin[2] + view_ofs[2]) + f[2] * 64.0]
}

/// `Navigate`'s move (0x10070381..0x100703e8): full speed toward `yaw` relative to the body yaw.
/// The steer yaw's `UTIL_AngleMod` stays on the x87 stack for the difference; the comparison
/// uses the float copies.
fn move_toward(body_yaw: f32, yaw: f32, speed: f64) -> (f32, f32) {
    let a = super::brain::angle_mod(body_yaw);
    let bb = super::brain::angle_mod_x87(yaw);
    let mut d = (bb - a as f64).abs();
    if bb as f32 <= a {
        d = (360.0 - d).abs();
    }
    let rad = d * 0.017_453_292_519_943_295;
    ((rad.cos() * speed) as f32, -(rad.sin() * speed) as f32)
}

/// `CBotNav::Replan` (0x1006fa40): a fresh plan for `goal`. The attack module's goals go straight
/// (no path: the first arrival ends it); otherwise a node path from the node nearest the bot to
/// the node nearest the goal, or a random point 1024..4096 units away when there is none.
fn replan(b: &mut Brain, cx: &mut Cx, goal: [f32; 3]) {
    let now = cx.time();
    b.nav.stuck_pos = [0.0; 3];
    b.nav.stuck_time = stuck_window(now, cx.pev());
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
    let end = super::rules::current().wander_end(dir, dist, origin);
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
    let tr = eng.trace_line(origin, super::rules::current().steer_probe_end(f, origin), 0, me);
    if tr.flFraction < 1.0 && !is_door(tr.pHit) {
        b.nav.detour = super::rules::current().forward_detour(tr.vecEndPos, f);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eye_probe_end_keeps_the_x_sum_unrounded() {
        // mp.dll adds v_forward.x * 64 to the unrounded origin.x + view_ofs.x (0x10070128 faddp);
        // y and z are stored first. Here the two differ in the last bit.
        let (o, f) = (f32::from_bits(0x43fd_61f3), f32::from_bits(0x3f18_ccfd));
        let e = eye_probe_end([o; 3], [17.0; 3], [f; 3]);
        assert_eq!(e[0].to_bits(), 0x440c_7dc9);
        assert_eq!(e[1].to_bits(), 0x440c_7dca);
        assert_eq!(e[2].to_bits(), 0x440c_7dca);
    }

    #[test]
    fn move_is_relative_to_the_body_yaw() {
        // Target to the left (counter-clockwise): forward ~0, full speed left (negative side).
        let (f, s) = move_toward(0.0, 90.0, 300.0);
        assert!(f.abs() < 1e-3 && s == -300.0, "{f} {s}");
        // Target to the right: 360 - 90 = 270 degrees, so full speed right.
        let (f, s) = move_toward(90.0, 0.0, 300.0);
        assert!(f.abs() < 1e-3 && s == 300.0, "{f} {s}");
        // Straight ahead: equal yaws take the `360 - d` branch, so a full turn (sin ~ -2e-16).
        let (f, s) = move_toward(45.0, 45.0, 250.0);
        assert!(f == 250.0 && s > 0.0 && s < 1e-12, "{f} {s}");
    }

    #[test]
    fn move_uses_the_unrounded_angle_mod() {
        // The steer yaw's `UTIL_AngleMod` (360 + yaw) is used unrounded in the difference.
        let (body, yaw) = (10.0f32, -0.123_456_79f32);
        let d = ((360.0 + yaw as f64) - 10.0).abs();
        let rad = d * 0.017_453_292_519_943_295;
        assert_eq!(move_toward(body, yaw, 300.0), ((rad.cos() * 300.0) as f32, -(rad.sin() * 300.0) as f32));
    }
}
