//! [`ParticleModel::Decelerate`](super::ParticleModel::Decelerate): TS 2.1's particle update
//! (the particle manager at `0x1031abb0`; update and draw `0x10091020`, move `0x100902f0`,
//! collision `0x10090a00`; issue #261). Addresses are TS 2.1 `client.dll` VAs.
//!
//! TS keeps velocities in units per 0.1 s; the port's particles hold units per second, so the
//! TS numbers below are ×10 where they are speeds.

use super::{
    Camera, Collision, ONCOLLIDE_BLOOD, ONCOLLIDE_BLOOD_DECAL, ORIENT_BILLBOARD, ORIENT_ROLL, PSys, client_time, in_view_cone, quad,
};
use goldsrc::ext::render::{self as r, vec3 as v};
use goldsrc::ext::tri;
use goldsrc::sys::*;

/// Speeds above this (u/s; 3.3 TS units, `0x10271bd8`) lose [`DECEL`] a second.
pub const DEAD_ZONE: f32 = 33.0;
/// The deceleration (u/s²): `dt × 3.3 × 10` TS units a frame (`0x100904e6..0x100904fa`).
pub const DECEL: f32 = 330.0;
/// A landing slower than this (u/s; 6.6 TS units, `0x10090d12`) stops the particle.
const STOP_SPEED: f32 = 66.0;
/// Beyond this distance (zoom-scaled) the collision traces every 0.5..0.6 s and a hit kills the
/// speed (`0x10090ae0`, `0x10090bde`).
const NEAR: f32 = 512.0;

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

/// One horizontal component (`0x10090441..0x10090590`): toward ±[`DEAD_ZONE`] by
/// `dt × `[`DECEL`] when outside it.
fn decelerate(vel: f32, dt: f32) -> f32 {
    if vel < -DEAD_ZONE {
        vel + dt * DECEL
    } else if vel > DEAD_ZONE {
        vel - dt * DECEL
    } else {
        vel
    }
}

/// The vertical velocity step (`0x100905a8..0x100906b9`): gravity only when positive,
/// `(dt × 10) × g × (cl_gravity × 0.001)` TS units (a tenth of `Particle_Move`'s), then a rise
/// above [`DEAD_ZONE`] is slowed by [`DECEL`], and so is a fall below it when there is no
/// gravity.
pub fn vertical(vz: f32, gravity: f32, sv_gravity: f32, dt: f32) -> f32 {
    let mut vz = vz;
    if gravity > 0.0 {
        let k = (1.0f32 as f64 * sv_gravity as f64 * 0.001) as f32;
        vz = (vz as f64 - (dt as f64 * 10.0) * gravity as f64 * k as f64 * 10.0) as f32;
    }
    if vz > DEAD_ZONE {
        vz - dt * DECEL
    } else if gravity <= 0.0 && vz < -DEAD_ZONE {
        vz + dt * DECEL
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

    /// The move (`0x100902f0`): the origin moves first, with the old velocity; then the
    /// horizontal deceleration and [`vertical`]; then the collision of a colliding particle
    /// ([`Self::collide_decel`]) and the spin: a rolling quad turns by `avel.z`, any orientation
    /// but the billboard and roll by all three. No think, drag or resting.
    fn move_decel(&mut self, i: usize, now: f32, dt: f32, cam: &Camera, collision: &mut dyn FnMut(&mut PSys, Collision)) {
        let sv_gravity = self.gravity;
        let p = &mut self.parts[i];
        p.origin = v::ma(p.origin, dt, p.vel);
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
                ORIENT_ROLL => p.angles[2] += p.avel[2] * dt,
                _ => p.angles = v::ma(p.angles, dt, p.avel),
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
    /// `prev` stays where it was after a hit.
    fn collide_decel(&mut self, i: usize, now: f32, cam: &Camera, collision: &mut dyn FnMut(&mut PSys, Collision)) {
        let cull = self.cull_origin;
        let p = &mut self.parts[i];
        if p.visible {
            p.visible = in_view_cone(cam, cull, p.origin);
        }
        if !p.visible || p.next_trace > now {
            return;
        }
        let zoom_scale = if (cam.fov as i32) < 40 { 0.5 } else { 1.0 };
        let d = v::dist(p.origin, cull) * zoom_scale;
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
        let dn = v::dot(p.vel, n);
        p.vel = v::ma(p.vel, -dn - dn, n);
        if n[2] > 0.9 && p.vel[2] < STOP_SPEED {
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
    /// frame the engine picks), the cache slot's render mode, `Color4f(c, c, c, 1)` (no fade-in),
    /// no stretch.
    fn draw_decel(&self, p: &super::Particle, cam: &Camera) {
        let Some(s) = self.slot(p.sprite) else { return };
        let model = r::sprite_model(s.handle);
        if model.is_null() || !tri::sprite_texture(model, p.frame as i32) {
            return;
        }
        let c = p.color;
        let corners = quad(p, cam, p.orient, false);
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

    /// `0x100904b6..0x10090590`: ±3.3 TS units (33 u/s) dead zone, `dt × 3.3 × 10` TS units off
    /// outside it.
    #[test]
    fn horizontal_deceleration() {
        assert_eq!(decelerate(640.0, 0.1), 607.0);
        assert_eq!(decelerate(-640.0, 0.1), -607.0);
        assert_eq!(decelerate(33.0, 0.1), 33.0);
        assert_eq!(decelerate(-33.0, 0.1), -33.0);
        // It can cross the dead zone in one step (no clamp).
        assert_eq!(decelerate(40.0, 0.1), 7.0);
    }

    /// `0x100905a8..0x100906b9`: gravity 13 at `sv_gravity` 800 takes 1040 u/s a second (a tenth
    /// of 3.0's `g × sv_gravity`); a negative gravity does nothing; the rise and the gravity-less
    /// fall are slowed.
    #[test]
    fn vertical_step() {
        let g13 = vertical(0.0, 13.0, 800.0, 0.1);
        assert!((g13 + 104.0).abs() < 1e-3, "{g13}");
        // Falling faster than 33 u/s with gravity on: gravity only.
        let fall = vertical(-100.0, 1.0, 800.0, 0.1);
        assert!((fall + 108.0).abs() < 1e-3, "{fall}");
        assert_eq!(vertical(100.0, 0.0, 800.0, 0.1), 67.0, "rise slowed");
        assert_eq!(vertical(-100.0, 0.0, 800.0, 0.1), -67.0, "no gravity: the fall slows too");
        assert_eq!(vertical(-100.0, -5.0, 800.0, 0.1), -67.0, "negative gravity is none");
        assert_eq!(vertical(20.0, 0.0, 800.0, 0.1), 20.0);
    }
}
