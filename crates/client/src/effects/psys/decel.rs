//! [`ParticleModel::Decelerate`](super::ParticleModel::Decelerate): TS 2.1's particle update
//! (the particle manager at `0x1031abb0`; update and draw `0x10091020`, move `0x100902f0`,
//! collision `0x10090a00`; issue #261). Addresses are TS 2.1 `client.dll` VAs.
//!
//! Velocities and angular velocities are TS's units per 0.1 s, as in 3.0's update; every step
//! keeps the debug build's stores (issue #286).

use super::{
    Camera, Collision, ONCOLLIDE_BLOOD, ONCOLLIDE_BLOOD_DECAL, ORIENT_BILLBOARD, ORIENT_ROLL, PSys, Particle, client_time, in_view_cone,
    len_stored,
};
use crate::gameplay::DllBuild;
use goldsrc::client::engine;
use goldsrc::ext::render::{self as r, Vec3, vec3 as v};
use goldsrc::ext::tri;
use goldsrc::sys::*;

/// Speeds beyond ±3.3 TS units (a double, `0x10271bd8`, `0x10271be8`) lose [`DECEL`] a frame step.
pub const DEAD_ZONE: f64 = 3.3;
/// A landing slower than this (TS units, a double, `0x10271bf8`) stops the particle.
const STOP_SPEED: f64 = 6.6;
/// Beyond this distance (zoom-scaled) the collision traces every 0.5..0.6 s and a hit kills the
/// speed (`0x10090ae0`, `0x10090bde`).
const NEAR: f32 = 512.0;

/// The deceleration `(dt × 3.3) × 10` on the x87 (doubles `0x10271bd8`, `0x10268220`;
/// `0x100904e6..0x100904fa`).
fn decel(dt: f32) -> f64 {
    dt as f64 * DEAD_ZONE * 10.0
}

/// The particle step (`0x10091920`): the frame time, scaled by the raw slow-motion factor `s`
/// (the last `TSSlowMo` ÷ 100) while one is set, and 0 below 0.05.
pub fn decel_step(frametime: f32, s: f32) -> f32 {
    if s == 0.0 {
        frametime
    } else if s >= 0.05 {
        (frametime as f64 * s as f64) as f32
    } else {
        0.0
    }
}

/// One horizontal component (`0x10090441..0x100905a6`): toward ±[`DEAD_ZONE`] by [`decel`]
/// when outside it, rounded once.
fn decelerate(vel: f32, dt: f32) -> f32 {
    let x = vel as f64;
    if x < -DEAD_ZONE {
        (x + decel(dt)) as f32
    } else if x > DEAD_ZONE {
        (x - decel(dt)) as f32
    } else {
        vel
    }
}

/// The vertical velocity step (`0x100905a8..0x100906b9`): gravity only when positive,
/// `(f32(dt × 10f) × g) × f32(cl_gravity × 0.001)` TS units on the x87 (a tenth of
/// `Particle_Move`'s), then a rise above [`DEAD_ZONE`] is slowed by [`decel`], and so is a fall
/// below it when there is no gravity.
pub fn vertical(vz: f32, gravity: f32, sv_gravity: f32, dt: f32) -> f32 {
    let mut vz = vz;
    if gravity > 0.0 {
        let k = (1.0f32 as f64 * sv_gravity as f64 * 0.001) as f32;
        vz = (vz as f64 - (dt * 10.0) as f64 * gravity as f64 * k as f64) as f32;
    }
    let x = vz as f64;
    if x > DEAD_ZONE {
        (x - decel(dt)) as f32
    } else if gravity <= 0.0 && x < -DEAD_ZONE {
        (x + decel(dt)) as f32
    } else {
        vz
    }
}

impl PSys {
    /// The update and draw (`0x10091020`), per live particle in slot order: the death test
    /// (life over, the animation over or none, and faded or not fading; no shrink test), the
    /// fade once the life is over (floored at 0, no upper clamp), the scale, the move with its
    /// collision, the draw, the frame step once the life is over, then the life step:
    /// `life −= (now − last) × (s or 1)` on the raw slow-motion factor `s`.
    pub(super) fn update_draw_decel(&mut self, dt: f32, cam: &Camera, collision: &mut dyn FnMut(&mut PSys, Collision)) {
        let now = client_time();
        let k = if self.slowmo == 0.0 { 1.0 } else { self.slowmo };
        self.react_list.clear();
        let mut last = 0;
        let mut i = 0;
        while i <= self.high {
            if !self.parts[i].alive {
                i += 1;
                continue;
            }
            last = last.max(i);
            let frames = self.frames(self.parts[i].sprite);
            let p = &mut self.parts[i];
            // 0x100910f0..0x100912c4
            let anim_done = p.sprite < 0 || p.framerate == 0.0 || (frames as f32) < p.frame;
            let faded = p.fadespeed == 0.0 || p.color <= 0.0;
            if p.life <= 0.0 && anim_done && faded {
                p.alive = false;
                i += 1;
                continue;
            }
            if p.fadespeed != 0.0 && p.life <= 0.0 {
                let c = p.color as f64 - dt as f64 * p.fadespeed as f64 * 100.0;
                p.color = if c <= 0.0 { 0.0 } else { c as f32 };
            }
            if p.scalespeed != 0.0 {
                p.radius = (dt as f64 * p.scalespeed as f64 * 100.0 + p.radius as f64) as f32;
            }
            p.visible = p.sprite >= 0;
            self.move_decel(i, now, dt, cam, collision);
            let p = &self.parts[i];
            if p.visible {
                self.draw_decel(p, cam);
            }
            let p = &mut self.parts[i];
            if p.framerate != 0.0 && p.life <= 0.0 {
                p.frame = (dt as f64 * p.framerate as f64 + p.frame as f64) as f32;
            }
            p.life = (p.life as f64 - (now as f64 - p.life_clock as f64) * k as f64) as f32;
            p.life_clock = now;
            i += 1;
        }
        self.high = last;
    }

    /// The move (`0x100902f0`): the origin moves first, with the old velocity
    /// (`f32(o + f32(v × f32(dt × 10f)))`, 0x10090426..0x1009044d); then the horizontal
    /// deceleration and [`vertical`]; then the collision of a colliding particle
    /// ([`Self::collide_decel`]) and the spin in degrees per 0.1 s: a rolling quad turns by
    /// `avel.z·dt·10.0` rounded once, any orientation but the billboard and roll by all three with
    /// `avel·dt` and `× 10f` stored. No think, drag or resting.
    fn move_decel(&mut self, i: usize, now: f32, dt: f32, cam: &Camera, collision: &mut dyn FnMut(&mut PSys, Collision)) {
        let sv_gravity = self.gravity;
        let p = &mut self.parts[i];
        p.origin = v::ma(p.origin, dt * 10.0, p.vel);
        p.vel[0] = decelerate(p.vel[0], dt);
        p.vel[1] = decelerate(p.vel[1], dt);
        p.vel[2] = vertical(p.vel[2], p.gravity, sv_gravity, dt);
        if p.collide {
            self.collide_decel(i, now, cam, collision);
        }
        let p = &mut self.parts[i];
        if p.corners.is_none() {
            match p.orient {
                ORIENT_BILLBOARD => {}
                ORIENT_ROLL => p.angles[2] = (p.angles[2] as f64 + (p.avel[2] as f64 * dt as f64) * 10.0) as f32,
                _ => p.angles = [0, 1, 2].map(|c| p.angles[c] + p.avel[c] * dt * 10.0),
            }
        }
    }

    /// The collision (`0x10090a00`), only while the particle is drawn: the view cone is tested
    /// again, then at the trace time (every frame within [`NEAR`] units of the cull origin,
    /// zoom-scaled, else every `RandomFloat(0.5, 0.6)` s) a point-hull `EV_PlayerTrace(prev,
    /// origin, PM_STUDIO_BOX, -1)`. A clear trace moves `prev` on. A hit: a start in solid with
    /// the end outside just moves `prev` on; both in solid kill the particle, which still
    /// bounces. The particle moves to the hit, its velocity × 0.3 (0 beyond [`NEAR`]) and
    /// reflected; on a floor (`n.z > 0.9`) slower than [`STOP_SPEED`] it stops for good
    /// (velocity, gravity and roll speed 0, no more collision, billboard); then the callback.
    /// `prev` stays where it was after a hit. The distance is 2.1's stored length
    /// ([`len_stored`]) times the zoom, stored; the bounce dot stores its partial sums, and
    /// `VectorMA` (`0x10098330`) rounds each component once.
    fn collide_decel(&mut self, i: usize, now: f32, cam: &Camera, collision: &mut dyn FnMut(&mut PSys, Collision)) {
        let cull = self.cull_origin;
        let p = &mut self.parts[i];
        if p.visible {
            p.visible = in_view_cone(DllBuild::Debug, cam, cull, p.origin);
        }
        if !p.visible || p.next_trace > now {
            return;
        }
        let zoom_scale = if (cam.fov as i32) < 40 { 0.5 } else { 1.0 };
        let d = (len_stored(v::sub(p.origin, cull)) as f64 * zoom_scale) as f32;
        if d > NEAR {
            p.next_trace = (r::random_float(0.5, 0.6) as f64 + now as f64) as f32;
        }
        let tr = r::player_trace(p.prev, p.origin, 2, PM_STUDIO_BOX as i32, -1);
        if tr.fraction == 1.0 {
            p.prev = p.origin;
            return;
        }
        if r::point_contents(p.prev) == CONTENTS_SOLID {
            if r::point_contents(p.origin) != CONTENTS_SOLID {
                p.prev = p.origin;
                return;
            }
            p.alive = false;
        }
        p.origin = tr.endpos;
        p.vel = v::scale(p.vel, if d <= NEAR { 0.3 } else { 0.0 });
        let n = tr.plane.normal;
        let dot = (p.vel[0] as f64 * n[0] as f64) as f32;
        let dot = (dot as f64 + p.vel[1] as f64 * n[1] as f64) as f32;
        let dn = (dot as f64 + p.vel[2] as f64 * n[2] as f64) as f32;
        let s = -2.0 * dn;
        p.vel = [0, 1, 2].map(|c| (p.vel[c] as f64 + s as f64 * n[c] as f64) as f32);
        if n[2] as f64 > 0.9 && (p.vel[2] as f64) < STOP_SPEED {
            p.vel = v::ZERO;
            p.gravity = 0.0;
            p.collide = false;
            p.orient = ORIENT_BILLBOARD;
            p.avel[2] = 0.0;
        }
        let (origin, parent_vel, oncollide, target) = (p.origin, p.vel, p.oncollide, p.target);
        match (oncollide, target) {
            (1, _) => self.parts[i].alive = false,
            (2, target) => {
                self.parts[i].alive = false;
                if let Some(target) = target {
                    collision(self, Collision::Spawn { target, origin, parent_vel });
                }
            }
            (3, Some(target)) => collision(self, Collision::Spawn { target, origin, parent_vel }),
            (ONCOLLIDE_BLOOD | ONCOLLIDE_BLOOD_DECAL, _) => {
                collision(self, Collision::Blood { tr, splat: oncollide == ONCOLLIDE_BLOOD });
                self.parts[i].alive = false;
            }
            _ => {}
        }
    }

    /// The draw (`0x10091542..0x1009160a`): frame `ftol(frame)` (not clamped: past the last
    /// frame GoldSrc binds frame 0, which `tri::sprite_texture` asks FWGS for too), the cache
    /// slot's render mode, `Color4f(c, c, c, 1)` (no fade-in), no stretch, the corners [`quad`]'s.
    fn draw_decel(&self, p: &Particle, cam: &Camera) {
        let Some(s) = self.slot(p.sprite) else { return };
        let model = r::sprite_model(s.handle);
        if model.is_null() || !tri::sprite_texture(model, p.frame as i32) {
            return;
        }
        let c = p.color;
        let corners = quad(p, cam, &|a| engine().angle_vectors(a));
        tri::render_mode(s.mode as i32);
        tri::cull_face(TRICULLSTYLE_TRI_NONE);
        tri::begin(TRI_QUADS as i32);
        tri::color4f(c, c, c, 1.0);
        for (uv, vtx) in p.uv.unwrap_or([[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]).iter().zip(corners.iter()) {
            tri::tex_coord2f(uv[0], uv[1]);
            tri::vertex3fv(vtx);
        }
        tri::end();
        tri::render_mode(kRenderNormal as i32);
    }
}

/// 2.1's quad (`0x1008f470`): `R = f32(right × radius)` and `U = f32(up × radius)` stored, the
/// corners `(o + U) − R`, `(o + U) + R`, `(o − U) + R`, `(o − U) − R`, each operation stored.
/// The billboard uses the view's right and up; a rolling quad the engine's `AngleVectors`
/// (`pfnAngleVectors`, `0x102e14b8`) of (view pitch, view yaw, roll), any other orientation the
/// engine's of the particle's angles. `angle_vectors` is the engine's (a parameter for tests).
pub(super) fn quad(p: &Particle, cam: &Camera, angle_vectors: &dyn Fn(Vec3) -> (Vec3, Vec3, Vec3)) -> [Vec3; 4] {
    if let Some(c) = p.corners {
        return c;
    }
    let (right, up) = match p.orient {
        ORIENT_BILLBOARD => (cam.right, cam.up),
        ORIENT_ROLL => {
            let (_, r2, u2) = angle_vectors([cam.angles[0], cam.angles[1], p.angles[2]]);
            (r2, u2)
        }
        _ => {
            let (_, r2, u2) = angle_vectors(p.angles);
            (r2, u2)
        }
    };
    let (rr, uu) = (v::scale(right, p.radius), v::scale(up, p.radius));
    let (top, bottom) = (v::add(p.origin, uu), v::sub(p.origin, uu));
    [v::sub(top, rr), v::add(top, rr), v::add(bottom, rr), v::sub(bottom, rr)]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `0x10091920`: the frame time at normal speed, × the raw factor from 0.05, 0 below.
    #[test]
    fn step_follows_the_raw_factor() {
        assert_eq!(decel_step(0.02, 0.0), 0.02);
        assert_eq!(decel_step(0.02, 0.5), 0.01);
        assert_eq!(decel_step(0.02, 0.05), (0.02f64 * 0.05f32 as f64) as f32);
        assert_eq!(decel_step(0.02, 0.04), 0.0);
    }

    /// `0x10090441..0x100905a6`: a ±3.3 TS-unit dead zone (doubles), `(dt × 3.3) × 10` off
    /// outside it on the x87, rounded once.
    #[test]
    fn horizontal_deceleration() {
        let d = 0.1f32 as f64 * 3.3 * 10.0;
        assert_eq!(decelerate(64.0, 0.1), (64.0 - d) as f32);
        assert_eq!(decelerate(-64.0, 0.1), (d - 64.0) as f32);
        assert_eq!(decelerate(3.3, 0.1), 3.3, "f32(3.3) is below the double 3.3: inside");
        assert_eq!(decelerate(-3.3, 0.1), -3.3);
        // It can cross the dead zone in one step (no clamp); one rounding, where the f32 chain
        // `4 − f32(0.1 × 33)` gives 0.70000005 (0x3f333334).
        assert_eq!(decelerate(4.0, 0.1).to_bits(), 0x3f33_3332);
        assert_ne!(decelerate(4.0, 0.1), 4.0 - 0.1f32 * 33.0);
    }

    /// `0x100905a8..0x100906b9`: gravity 13 at `sv_gravity` 800 takes 10.4 TS units a 0.1 s
    /// step (a tenth of 3.0's `g × sv_gravity`): `vz − (f32(dt × 10) × g) × f32(800 × 0.001)`;
    /// a negative gravity does nothing; the rise and the gravity-less fall are slowed.
    #[test]
    fn vertical_step() {
        let k = (800.0f64 * 0.001) as f32 as f64;
        assert_eq!(vertical(0.0, 13.0, 800.0, 0.1), (0.0 - (0.1f32 * 10.0) as f64 * 13.0 * k) as f32);
        assert_eq!(vertical(0.0, 13.0, 800.0, 0.1).to_bits(), (-10.400_001f32).to_bits());
        // Falling faster than 3.3 with gravity on: gravity only.
        assert_eq!(vertical(-10.0, 1.0, 800.0, 0.1), (-10.0 - (0.1f32 * 10.0) as f64 * k) as f32);
        let d = 0.1f32 as f64 * 3.3 * 10.0;
        assert_eq!(vertical(10.0, 0.0, 800.0, 0.1), (10.0 - d) as f32, "rise slowed");
        assert_eq!(vertical(-10.0, 0.0, 800.0, 0.1), (d - 10.0) as f32, "no gravity: the fall slows too");
        assert_eq!(vertical(-10.0, -5.0, 800.0, 0.1), (d - 10.0) as f32, "negative gravity is none");
        assert_eq!(vertical(2.0, 0.0, 800.0, 0.1), 2.0);
    }

    /// `0x1008f470`: the up offset is added first, then the right one, each stored.
    #[test]
    fn quad_adds_up_first() {
        let cam = Camera { right: [0.1, 0.0, 0.0], up: [0.0, 0.0, 1.0], ..Default::default() };
        let p = Particle { origin: [1.0e8, 0.0, 3.0], radius: 3.0, orient: ORIENT_BILLBOARD, ..Default::default() };
        let q = quad(&p, &cam, &|_| unreachable!());
        let (rr, uu) = (v::scale(cam.right, 3.0), v::scale(cam.up, 3.0));
        assert_eq!(q[0], v::sub(v::add(p.origin, uu), rr));
        assert_eq!(q[2], v::add(v::sub(p.origin, uu), rr));
        // A rolling quad asks the engine's `AngleVectors` for (view pitch, view yaw, roll).
        let p = Particle { orient: ORIENT_ROLL, angles: [0.0, 0.0, 45.0], ..p };
        let cam = Camera { angles: [10.0, 20.0, 0.0], ..cam };
        let q = quad(&p, &cam, &|a| {
            assert_eq!(a, [10.0, 20.0, 45.0]);
            (v::ZERO, [0.0, 1.0, 0.0], [0.0, 0.0, 1.0])
        });
        assert_eq!(q[1], [1.0e8, 3.0, 6.0]);
    }
}
