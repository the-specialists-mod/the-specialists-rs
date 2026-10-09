//! Player movement, shared by the server (`pfnPM_Move`) and client prediction (`HUD_PlayerMove`).
//!
//! A deliberately small subset of `halflife-sdk/pm_shared/pm_shared.c`: walking with
//! step-up, air control, friction, gravity, jumping, and noclip. No water, ladders, ducking,
//! or step sounds yet. Keep everything in `f32` and in the same order on both sides.

use crate::{VEC_DUCK_HULL_MAX, VEC_DUCK_HULL_MIN, VEC_HULL_MAX, VEC_HULL_MIN};
use goldsrc_sys::{
    FL_ONGROUND, IN_JUMP, MOVETYPE_NOCLIP, MOVETYPE_WALK, PM_NORMAL,
    playermove_t, pmtrace_t,
};

const MAX_CLIP_PLANES: usize = 5;
const STOP_EPSILON: f32 = 0.1;
const JUMP_VELOCITY: f32 = 268.328_16; // sqrt(2 * 800 * 45): a 45 unit jump at sv_gravity 800
const MAX_FLOOR_SLOPE: f32 = 0.7; // ground normals flatter than this count as floor
const AIR_WISHSPEED_CAP: f32 = 30.0;

type Vec3 = [f32; 3];

fn add(a: Vec3, b: Vec3) -> Vec3 { [a[0] + b[0], a[1] + b[1], a[2] + b[2]] }
fn scale(a: Vec3, s: f32) -> Vec3 { [a[0] * s, a[1] * s, a[2] * s] }
fn ma(a: Vec3, s: f32, b: Vec3) -> Vec3 { [a[0] + s * b[0], a[1] + s * b[1], a[2] + s * b[2]] }
fn dot(a: Vec3, b: Vec3) -> f32 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }
fn length(a: Vec3) -> f32 { dot(a, a).sqrt() }
fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn normalize(a: Vec3) -> (Vec3, f32) {
    let l = length(a);
    if l == 0.0 { (a, 0.0) } else { (scale(a, 1.0 / l), l) }
}

/// `pfnPM_Init` / `HUD_PlayerMoveInit`: called once with the engine's playermove.
pub fn init(_pm: &mut playermove_t) {}

/// Material type for footstep and impact sounds (`CHAR_TEX_*`). TS reads sound/materials.txt.
pub fn find_texture_type(_name: &std::ffi::CStr) -> u8 {
    b'C' // CHAR_TEX_CONCRETE
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
    let (sy, cy) = angles[1].to_radians().sin_cos();
    let (sp, cp) = angles[0].to_radians().sin_cos();
    let (sr, cr) = angles[2].to_radians().sin_cos();
    let forward = [cp * cy, cp * sy, -sp];
    let right = [-sr * sp * cy + cr * sy, -sr * sp * sy - cr * cy, -sr * cp];
    let up = [cr * sp * cy + sr * sy, cr * sp * sy - sr * cy, cr * cp];
    (forward, right, up)
}

struct Pm<'a> {
    p: &'a mut playermove_t,
}

impl Pm<'_> {
    fn mv(&self) -> &goldsrc_sys::movevars_s {
        unsafe { &*self.p.movevars }
    }

    fn trace(&self, start: Vec3, end: Vec3) -> pmtrace_t {
        let (mut s, mut e) = (start, end);
        let f = self.p.PM_PlayerTrace.expect("PM_PlayerTrace");
        unsafe { f(s.as_mut_ptr(), e.as_mut_ptr(), PM_NORMAL as i32, -1) }
    }

    fn check_parameters(&mut self) {
        let p = &mut *self.p;
        let mut maxspeed = p.maxspeed;
        if p.clientmaxspeed > 0.0 {
            maxspeed = maxspeed.min(p.clientmaxspeed);
        }
        p.maxspeed = maxspeed;

        let spd = (p.cmd.forwardmove.powi(2) + p.cmd.sidemove.powi(2) + p.cmd.upmove.powi(2)).sqrt();
        if spd != 0.0 && spd > maxspeed {
            let ratio = maxspeed / spd;
            p.cmd.forwardmove *= ratio;
            p.cmd.sidemove *= ratio;
            p.cmd.upmove *= ratio;
        }

        if p.dead != 0 || p.deadflag != 0 {
            p.cmd.forwardmove = 0.0;
            p.cmd.sidemove = 0.0;
            p.cmd.upmove = 0.0;
        }

        p.angles = p.cmd.viewangles;
        let (f, r, u) = angle_vectors(p.angles);
        (p.forward, p.right, p.up) = (f, r, u);
    }

    fn check_velocity(&mut self) {
        let max = self.mv().maxvelocity;
        for v in &mut self.p.velocity {
            if v.is_nan() {
                *v = 0.0;
            }
            *v = v.clamp(-max, max);
        }
    }

    fn categorize_position(&mut self) {
        let point = add(self.p.origin, [0.0, 0.0, -2.0]);
        if self.p.velocity[2] > 180.0 {
            self.p.onground = -1;
        } else {
            let tr = self.trace(self.p.origin, point);
            if tr.plane.normal[2] < MAX_FLOOR_SLOPE {
                self.p.onground = -1;
            } else {
                self.p.onground = tr.ent;
            }
            if self.p.onground != -1 && tr.startsolid == 0 && tr.allsolid == 0 {
                self.p.origin = tr.endpos;
            }
        }
        if self.p.onground != -1 {
            self.p.flags |= FL_ONGROUND as i32;
        } else {
            self.p.flags &= !(FL_ONGROUND as i32);
        }
    }

    fn add_half_gravity(&mut self) {
        let g = if self.p.gravity != 0.0 { self.p.gravity } else { 1.0 };
        self.p.velocity[2] -= g * self.mv().gravity * 0.5 * self.p.frametime;
    }

    fn friction(&mut self) {
        let vel = self.p.velocity;
        let speed = length(vel);
        if speed < 0.1 {
            return;
        }
        let friction = self.mv().friction * if self.p.friction != 0.0 { self.p.friction } else { 1.0 };
        let control = speed.max(self.mv().stopspeed);
        let drop = control * friction * self.p.frametime;
        let newspeed = (speed - drop).max(0.0) / speed;
        self.p.velocity = scale(vel, newspeed);
    }

    fn accelerate(&mut self, wishdir: Vec3, wishspeed: f32, accel: f32) {
        let current = dot(self.p.velocity, wishdir);
        let add = wishspeed - current;
        if add <= 0.0 {
            return;
        }
        let accelspeed = (accel * self.p.frametime * wishspeed * self.p.friction).min(add);
        self.p.velocity = ma(self.p.velocity, accelspeed, wishdir);
    }

    fn air_accelerate(&mut self, wishdir: Vec3, wishspeed: f32, accel: f32) {
        let wishspd = wishspeed.min(AIR_WISHSPEED_CAP);
        let current = dot(self.p.velocity, wishdir);
        let add = wishspd - current;
        if add <= 0.0 {
            return;
        }
        let accelspeed = (accel * wishspeed * self.p.frametime).min(add);
        self.p.velocity = ma(self.p.velocity, accelspeed, wishdir);
    }

    /// Horizontal wish direction and speed from the usercmd, clamped to maxspeed.
    fn wish(&self) -> (Vec3, f32) {
        let mut f = self.p.forward;
        let mut r = self.p.right;
        f[2] = 0.0;
        r[2] = 0.0;
        let (f, _) = normalize(f);
        let (r, _) = normalize(r);
        let fm = self.p.cmd.forwardmove;
        let sm = self.p.cmd.sidemove;
        let wishvel = [f[0] * fm + r[0] * sm, f[1] * fm + r[1] * sm, 0.0];
        let (dir, speed) = normalize(wishvel);
        (dir, speed.min(self.p.maxspeed))
    }

    /// Slides `velocity` along any planes hit during this frame. Returns the blocked flags
    /// (1 = floor, 2 = wall/step).
    fn fly_move(&mut self) -> i32 {
        let mut blocked = 0;
        let original_velocity = self.p.velocity;
        let primal_velocity = self.p.velocity;
        let mut planes: [Vec3; MAX_CLIP_PLANES] = [[0.0; 3]; MAX_CLIP_PLANES];
        let mut numplanes = 0;
        let mut time_left = self.p.frametime;
        let mut allfraction = 0.0;

        for _ in 0..4 {
            if self.p.velocity == [0.0; 3] {
                break;
            }
            let end = ma(self.p.origin, time_left, self.p.velocity);
            let tr = self.trace(self.p.origin, end);
            allfraction += tr.fraction;

            if tr.allsolid != 0 {
                self.p.velocity = [0.0; 3];
                return 4;
            }
            if tr.fraction > 0.0 {
                self.p.origin = tr.endpos;
                numplanes = 0;
            }
            if tr.fraction == 1.0 {
                break;
            }
            if tr.plane.normal[2] > MAX_FLOOR_SLOPE {
                blocked |= 1;
            }
            if tr.plane.normal[2] == 0.0 {
                blocked |= 2;
            }
            time_left -= time_left * tr.fraction;

            if numplanes >= MAX_CLIP_PLANES {
                self.p.velocity = [0.0; 3];
                break;
            }
            planes[numplanes] = tr.plane.normal;
            numplanes += 1;

            // Find a velocity that runs along every plane we're touching.
            let mut found = None;
            for i in 0..numplanes {
                let v = clip_velocity(original_velocity, planes[i], 1.0);
                if (0..numplanes).all(|j| j == i || dot(v, planes[j]) >= 0.0) {
                    found = Some(v);
                    break;
                }
            }
            match found {
                Some(v) => self.p.velocity = v,
                None if numplanes == 2 => {
                    // Go along the crease.
                    let dir = cross(planes[0], planes[1]);
                    self.p.velocity = scale(dir, dot(dir, self.p.velocity));
                }
                None => {
                    self.p.velocity = [0.0; 3];
                    break;
                }
            }
            // Don't bounce back into a corner.
            if dot(self.p.velocity, primal_velocity) <= 0.0 {
                self.p.velocity = [0.0; 3];
                break;
            }
        }
        if allfraction == 0.0 {
            self.p.velocity = [0.0; 3];
        }
        blocked
    }

    fn walk_move(&mut self) {
        let (wishdir, wishspeed) = self.wish();
        self.p.velocity[2] = 0.0;
        self.accelerate(wishdir, wishspeed, self.mv().accelerate);
        self.p.velocity[2] = 0.0;

        if length(self.p.velocity) < 1.0 {
            self.p.velocity = [0.0; 3];
            return;
        }

        let oldonground = self.p.onground;
        let mut dest = ma(self.p.origin, self.p.frametime, self.p.velocity);
        dest[2] = self.p.origin[2];
        let tr = self.trace(self.p.origin, dest);
        if tr.fraction == 1.0 {
            self.p.origin = tr.endpos;
            return;
        }
        if oldonground == -1 && self.p.waterlevel == 0 {
            return;
        }

        // Try sliding flat, then try stepping up stepsize and sliding; keep whichever got further.
        let original = self.p.origin;
        let originalvel = self.p.velocity;
        self.fly_move();
        let down = self.p.origin;
        let downvel = self.p.velocity;

        self.p.origin = original;
        self.p.velocity = originalvel;
        let stepsize = self.mv().stepsize;
        let up = add(self.p.origin, [0.0, 0.0, stepsize]);
        let tr = self.trace(self.p.origin, up);
        if tr.startsolid == 0 && tr.allsolid == 0 {
            self.p.origin = tr.endpos;
        }
        self.fly_move();

        // Push back down.
        let dn = add(self.p.origin, [0.0, 0.0, -stepsize]);
        let tr = self.trace(self.p.origin, dn);
        if tr.startsolid == 0 && tr.allsolid == 0 {
            self.p.origin = tr.endpos;
        }
        // Stepped onto something too steep: use the flat result.
        if tr.plane.normal[2] < MAX_FLOOR_SLOPE {
            self.p.origin = down;
            self.p.velocity = downvel;
            return;
        }
        let up_dist = (self.p.origin[0] - original[0]).powi(2) + (self.p.origin[1] - original[1]).powi(2);
        let down_dist = (down[0] - original[0]).powi(2) + (down[1] - original[1]).powi(2);
        if down_dist > up_dist {
            self.p.origin = down;
            self.p.velocity = downvel;
        } else {
            self.p.velocity[2] = downvel[2];
        }
    }

    fn air_move(&mut self) {
        let (wishdir, wishspeed) = self.wish();
        self.air_accelerate(wishdir, wishspeed, self.mv().airaccelerate);
        self.fly_move();
    }

    fn jump(&mut self) {
        let p = &mut *self.p;
        if p.cmd.buttons & IN_JUMP as u16 == 0 {
            p.oldbuttons &= !(IN_JUMP as i32);
            return;
        }
        if p.onground == -1 || p.oldbuttons & IN_JUMP as i32 != 0 {
            return; // must release jump before jumping again
        }
        p.onground = -1;
        p.flags &= !(FL_ONGROUND as i32);
        p.velocity[2] = JUMP_VELOCITY;
        p.oldbuttons |= IN_JUMP as i32;
    }

    fn noclip(&mut self) {
        let p = &mut *self.p;
        let fm = p.cmd.forwardmove;
        let sm = p.cmd.sidemove;
        let wishvel = add(add(scale(p.forward, fm), scale(p.right, sm)), [0.0, 0.0, p.cmd.upmove]);
        let (dir, speed) = normalize(wishvel);
        p.velocity = scale(dir, speed.min(p.maxspeed));
        p.origin = ma(p.origin, p.frametime, p.velocity);
    }
}

fn clip_velocity(v: Vec3, normal: Vec3, overbounce: f32) -> Vec3 {
    let backoff = dot(v, normal) * overbounce;
    let mut out = [0.0; 3];
    for i in 0..3 {
        out[i] = v[i] - normal[i] * backoff;
        if out[i] > -STOP_EPSILON && out[i] < STOP_EPSILON {
            out[i] = 0.0;
        }
    }
    out
}

/// Runs one usercmd of movement. Called by the engine on the server and during client prediction.
pub fn player_move(p: &mut playermove_t) {
    p.frametime = p.cmd.msec as f32 * 0.001;
    let mut pm = Pm { p };
    pm.check_parameters();

    if pm.p.movetype == MOVETYPE_NOCLIP as i32 {
        pm.noclip();
        return;
    }
    if pm.p.movetype != MOVETYPE_WALK as i32 {
        return; // MOVETYPE_NONE etc.: the server controls the player
    }

    pm.categorize_position();
    if pm.p.onground == -1 {
        pm.add_half_gravity();
    }
    pm.jump();
    if pm.p.onground != -1 {
        pm.p.velocity[2] = 0.0;
        pm.friction();
    }
    pm.check_velocity();

    if pm.p.onground != -1 {
        pm.walk_move();
    } else {
        pm.air_move();
    }

    pm.categorize_position();
    pm.check_velocity();
    if pm.p.onground == -1 {
        pm.add_half_gravity();
    } else {
        pm.p.velocity[2] = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use goldsrc_sys::movevars_s;
    use std::alloc::{Layout, alloc_zeroed};

    const FLOOR_Z: f32 = 0.0;
    const WALL_X: f32 = 200.0;

    /// Fake world: a floor at z = 0 and a wall at x = 200, for the standing hull.
    unsafe extern "C" fn trace(start: *mut f32, end: *mut f32, _flags: i32, _ignore: i32) -> pmtrace_t {
        let s = unsafe { [*start, *start.add(1), *start.add(2)] };
        let e = unsafe { [*end, *end.add(1), *end.add(2)] };
        let mut tr = pmtrace_t { fraction: 1.0, endpos: e, ent: -1, ..Default::default() };
        let mut hit = |frac: f32, normal: Vec3| {
            if frac < tr.fraction {
                tr.fraction = frac;
                tr.plane.normal = normal;
                tr.ent = 0;
            }
        };
        let (bottom_s, bottom_e) = (s[2] + VEC_HULL_MIN[2], e[2] + VEC_HULL_MIN[2]);
        if bottom_e < FLOOR_Z && bottom_s >= FLOOR_Z {
            hit((bottom_s - FLOOR_Z) / (bottom_s - bottom_e), [0.0, 0.0, 1.0]);
        }
        let (front_s, front_e) = (s[0] + VEC_HULL_MAX[0], e[0] + VEC_HULL_MAX[0]);
        if front_e > WALL_X && front_s <= WALL_X {
            hit((WALL_X - front_s) / (front_e - front_s), [-1.0, 0.0, 0.0]);
        }
        tr.endpos = ma(s, tr.fraction, [e[0] - s[0], e[1] - s[1], e[2] - s[2]]);
        tr
    }

    fn setup() -> (Box<playermove_t>, Box<movevars_s>) {
        // playermove_t is ~300 KB: allocate it directly on the heap.
        let p = unsafe { Box::from_raw(alloc_zeroed(Layout::new::<playermove_t>()).cast::<playermove_t>()) };
        let mv = Box::new(movevars_s {
            gravity: 800.0,
            stopspeed: 100.0,
            maxspeed: 320.0,
            accelerate: 10.0,
            airaccelerate: 10.0,
            friction: 4.0,
            stepsize: 18.0,
            maxvelocity: 2000.0,
            ..Default::default()
        });
        let mut p = p;
        p.movetype = MOVETYPE_WALK as i32;
        p.maxspeed = 320.0;
        p.gravity = 1.0;
        p.friction = 1.0;
        p.PM_PlayerTrace = Some(trace);
        (p, mv)
    }

    fn run(p: &mut playermove_t, mv: &mut movevars_s, frames: usize) {
        p.movevars = mv;
        for _ in 0..frames {
            p.cmd.msec = 10;
            player_move(p);
        }
    }

    #[test]
    fn falls_and_lands_on_floor() {
        let (mut p, mut mv) = setup();
        p.origin = [0.0, 0.0, 100.0];
        run(&mut p, &mut mv, 200);
        assert!((p.origin[2] - 36.0).abs() < 0.01, "z = {}", p.origin[2]);
        assert_eq!(p.velocity[2], 0.0);
        assert_ne!(p.flags & FL_ONGROUND as i32, 0);
    }

    #[test]
    fn walks_to_maxspeed_and_stops_at_wall() {
        let (mut p, mut mv) = setup();
        p.origin = [0.0, 0.0, 36.0];
        p.cmd.forwardmove = 400.0;
        run(&mut p, &mut mv, 30);
        let speed = length(p.velocity);
        assert!((speed - 320.0).abs() < 1.0, "speed = {speed}");
        run(&mut p, &mut mv, 100);
        assert!(p.origin[0] <= WALL_X - 16.0 + 0.01, "x = {}", p.origin[0]);
        assert!(p.origin[0] > WALL_X - 16.0 - 1.0, "x = {}", p.origin[0]);
        assert_eq!(p.velocity[0], 0.0);
    }

    #[test]
    fn jump_needs_release() {
        let (mut p, mut mv) = setup();
        p.origin = [0.0, 0.0, 36.0];
        p.cmd.buttons = IN_JUMP as u16;
        run(&mut p, &mut mv, 1);
        assert!(p.velocity[2] > 200.0);
        run(&mut p, &mut mv, 200); // still holding jump: lands, doesn't re-jump
        assert_eq!(p.velocity[2], 0.0);
        assert!((p.origin[2] - 36.0).abs() < 0.01);
    }
}
