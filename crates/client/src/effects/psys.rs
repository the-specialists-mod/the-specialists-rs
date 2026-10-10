//! TS sprite particle system (`g_PSys` `0x1018bde8`; rendering doc §5.2-5.4). Map emitters
//! (type 3) and every weapon effect (muzzle flashes, smoke, sparks, debris) share it.
//!
//! Units follow the TS structures where the doc gives them: `radius += dt*scalespeed*100`,
//! `color -= dt*fadespeed*100`, gravity 1.0 = 800 u/s^2. Velocities are stored in units per
//! second here (TS keeps units per 0.1 s; templates are converted on spawn).

use goldsrc::ext::render::{self as r, Vec3, vec3 as v};
use goldsrc::sys::*;
use goldsrc::tri;

pub const KIND_EFFECT: u8 = 0;
pub const KIND_MAP: u8 = 3;

/// `follow_view` quad orientations (`Quad_Build` `0x10050080`).
pub const ORIENT_BILLBOARD: u8 = 0;
pub const ORIENT_ROLL: u8 = 1;
pub const ORIENT_FREE: u8 = 2;
pub const ORIENT_AXIS: u8 = 3;

#[derive(Clone, Debug)]
pub struct Particle {
    pub kind: u8,
    pub sprite: u16,
    pub rendermode: i32,
    pub origin: Vec3,
    pub prev: Vec3,
    /// Units per second.
    pub vel: Vec3,
    pub angles: Vec3,
    /// Degrees per second.
    pub avel: Vec3,
    pub randvel_lo: Vec3,
    pub randvel_hi: Vec3,
    pub randomdelta: f32,
    pub life: f32,
    pub gravity: f32,
    pub drag: f32,
    /// Grey level 0..1 (TS), times `rgb` (effects tint).
    pub color: f32,
    pub rgb: Vec3,
    pub alpha: f32,
    pub fadespeed: f32,
    pub radius: f32,
    pub scalespeed: f32,
    pub frame: f32,
    pub framerate: f32,
    pub react: f32,
    pub wind: f32,
    pub fadein: f32,
    pub spawn: f64,
    pub orient: u8,
    pub followaxis: Vec3,
    pub stretch: f32,
    pub collide: bool,
    /// 0 bounce, 1 kill, 2 kill + spawn target, 3 spawn target + bounce.
    pub oncollide: u8,
    pub target: Option<u16>,
    pub resting: bool,
    pub(crate) next_think: f64,
    pub(crate) next_trace: f64,
}

impl Default for Particle {
    fn default() -> Self {
        Particle {
            kind: KIND_EFFECT,
            sprite: 0,
            rendermode: kRenderTransAdd as i32,
            origin: v::ZERO,
            prev: v::ZERO,
            vel: v::ZERO,
            angles: v::ZERO,
            avel: v::ZERO,
            randvel_lo: v::ZERO,
            randvel_hi: v::ZERO,
            randomdelta: 0.0,
            life: 1.0,
            gravity: 0.0,
            drag: 0.0,
            color: 1.0,
            rgb: [1.0; 3],
            alpha: 1.0,
            fadespeed: 0.0,
            radius: 4.0,
            scalespeed: 0.0,
            frame: 0.0,
            framerate: 0.0,
            react: 0.0,
            wind: 0.0,
            fadein: 0.0,
            spawn: 0.0,
            orient: ORIENT_BILLBOARD,
            followaxis: [0.0, 0.0, 1.0],
            stretch: 0.0,
            collide: false,
            oncollide: 0,
            target: None,
            resting: false,
            next_think: 0.0,
            next_trace: 0.0,
        }
    }
}

struct Sprite {
    name: String,
    h: HSPRITE,
    model: *const model_s,
    frames: i32,
}

/// Seeded gust model (`TSPSys` subtype 3): identical gusts on every client for one seed.
#[derive(Default, Clone, Copy, Debug)]
pub struct Wind {
    pub seed: i32,
    pub yaw: f32,
    pub base: f32,
    start: f64,
    seg: u32,
    seg_start: f64,
    seg_len: f32,
    from: f32,
    to: f32,
}

impl Wind {
    pub fn set(&mut self, seed: i32, yaw: i32, speed: f32, now: f64) {
        let cur = self.speed(now);
        *self = Wind { seed, yaw: yaw as f32, base: speed, start: now, seg: 0, seg_start: now, seg_len: 0.0, from: cur, to: cur };
        self.next_segment(now);
    }
    fn next_segment(&mut self, now: f64) {
        let s = (self.seed as u32).wrapping_add(self.seg * 2);
        self.seg_len = crate::events::random::shared_random_float(s, 0.001, 1.0);
        self.from = self.to;
        self.to = crate::events::random::shared_random_float(s.wrapping_add(1), 0.0, self.base.max(0.0));
        self.seg_start = now;
        self.seg = self.seg.wrapping_add(1);
    }
    /// Cosine-interpolated gust speed (`CosineLerp` `0x10050760`).
    pub fn speed(&self, now: f64) -> f32 {
        if self.seg_len <= 0.0 {
            return 0.0;
        }
        let t = (((now - self.seg_start) / self.seg_len as f64) as f32).clamp(0.0, 1.0);
        let k = (1.0 - (t * std::f32::consts::PI).cos()) * 0.5;
        self.from + (self.to - self.from) * k
    }
    pub fn advance(&mut self, now: f64) {
        let mut guard = 0;
        while self.seg_len > 0.0 && now - self.seg_start >= self.seg_len as f64 && guard < 64 {
            let end = self.seg_start + self.seg_len as f64;
            self.next_segment(end);
            guard += 1;
        }
        if guard == 64 {
            self.seg_start = now;
        }
    }
    pub fn velocity(&self, now: f64) -> Vec3 {
        let (s, c) = self.yaw.to_radians().sin_cos();
        v::scale([c, s, 0.0], self.speed(now))
    }
}

pub struct PSys {
    parts: Vec<Particle>,
    sprites: Vec<Sprite>,
    pub max: usize,
    cooldown: [f64; 128],
    pub wind: Wind,
    /// Template spawns requested by collisions (`oncollide` 2/3): (template, position).
    pub spawn_requests: Vec<(u16, Vec3)>,
    pub physics: bool,
    pub divisor: f32,
}

impl Default for PSys {
    fn default() -> Self {
        PSys {
            parts: Vec::new(),
            sprites: Vec::new(),
            max: 4096,
            cooldown: [0.0; 128],
            wind: Wind::default(),
            spawn_requests: Vec::new(),
            physics: true,
            divisor: 1.0,
        }
    }
}

/// Camera state the particle pass uses (set by `view` every frame).
#[derive(Clone, Copy, Default)]
pub struct Camera {
    pub origin: Vec3,
    pub angles: Vec3,
    pub forward: Vec3,
    pub right: Vec3,
    pub up: Vec3,
    pub fov: f32,
}

impl PSys {
    pub fn count(&self) -> usize {
        self.parts.len()
    }

    /// Debug summary: (nearest distance, in view cone, sample particle).
    pub fn debug(&self, cam: &Camera) -> String {
        let near = self.parts.iter().map(|p| v::dist(p.origin, cam.origin)).fold(f32::MAX, f32::min);
        let inview = self.parts.iter().filter(|p| in_view_cone(cam, p.origin)).count();
        let sample = self.parts.first().map(|p| {
            format!("org {:?} r {:.1} c {:.2} life {:.1} spr {} mode {}", p.origin, p.radius, p.color, p.life, p.sprite, p.rendermode)
        });
        format!("nearest {near:.0} inview {inview} cam {:?} oldest {:?}", cam.origin, sample)
    }

    pub fn clear(&mut self) {
        self.parts.clear();
        self.cooldown = [0.0; 128];
    }

    /// Forget sprite handles (after a level change the engine reloads them).
    pub fn reset_sprites(&mut self) {
        self.sprites.clear();
        self.parts.clear();
    }

    /// Sprite slot for `name` (`PSys_LoadSprite` `0x100509d0`), loading it on first use. 64
    /// slots in TS; unlimited here.
    pub fn sprite(&mut self, name: &str) -> Option<u16> {
        if let Some(i) = self.sprites.iter().position(|s| s.name.eq_ignore_ascii_case(name)) {
            return (self.sprites[i].h != 0).then_some(i as u16);
        }
        let h = r::spr_load(name);
        let (model, frames) = if h != 0 { (r::sprite_model(h), r::spr_frames(h).max(1)) } else { (std::ptr::null(), 1) };
        if h == 0 || model.is_null() {
            goldsrc::client::engine().print(&format!("WARNING! cannot found {name}\n"));
        }
        self.sprites.push(Sprite { name: name.to_owned(), h: if model.is_null() { 0 } else { h }, model, frames });
        let i = self.sprites.len() - 1;
        (self.sprites[i].h != 0).then_some(i as u16)
    }

    pub fn frames(&self, sprite: u16) -> i32 {
        self.sprites.get(sprite as usize).map_or(1, |s| s.frames)
    }

    /// `PSys_CooldownReady`/`PSys_SetCooldown`: rate limiting for effect groups.
    pub fn cooldown_ready(&mut self, slot: usize, now: f64, interval: f32) -> bool {
        let c = &mut self.cooldown[slot % 128];
        if now < *c {
            return false;
        }
        *c = now + interval as f64;
        true
    }

    pub fn add(&mut self, mut p: Particle, now: f64) {
        if self.parts.len() >= self.max {
            // Oldest-first replacement keeps steady effects (rain) from starving new ones.
            self.parts.remove(0);
        }
        p.spawn = now;
        p.prev = p.origin;
        p.next_think = now;
        p.next_trace = now;
        self.parts.push(p);
    }

    /// Pushes particles near `org` away from it (`PSys_PushRadial` `0x100524c0`).
    pub fn push_radial(&mut self, org: Vec3, radius: f32, strength: f32) {
        if !self.physics {
            return;
        }
        for p in self.parts.iter_mut().filter(|p| p.react > 0.0) {
            let d = v::sub(p.origin, org);
            let dist = v::len(d);
            if dist < radius && dist > 0.01 {
                let k = (1.0 - dist / radius) * strength * p.react;
                p.vel = v::ma(p.vel, k / dist, d);
                p.resting = false;
            }
        }
    }

    /// Pushes reacting particles away from a bullet line (`PSys_PushAlongLine` `0x10052720`).
    pub fn push_along_line(&mut self, a: Vec3, b: Vec3, strength: f32) {
        if !self.physics {
            return;
        }
        let ab = v::sub(b, a);
        let l2 = v::dot(ab, ab).max(1e-3);
        for p in self.parts.iter_mut().filter(|p| p.react > 0.0) {
            let t = (v::dot(v::sub(p.origin, a), ab) / l2).clamp(0.0, 1.0);
            let c = v::ma(a, t, ab);
            let d = v::sub(p.origin, c);
            let dist = v::len(d);
            if dist < 48.0 && dist > 0.01 {
                let k = (1.0 - dist / 48.0) * strength * p.react * 4.0;
                p.vel = v::ma(p.vel, k / dist, d);
                p.resting = false;
            }
        }
    }

    /// `PSys_UpdateDraw` (`0x100520d0`), simulation half. `dt = frametime * time_scale`.
    pub fn update(&mut self, now: f64, dt: f32, cam: &Camera, players: &[(Vec3, Vec3)]) {
        self.wind.advance(now);
        let wind = self.wind.velocity(now);
        let think_iv = (self.divisor / 30.0).max(dt);
        let physics = self.physics;
        let mut requests = Vec::new();
        self.parts.retain_mut(|p| {
            // 1. death
            let anim_done = p.framerate <= 0.0;
            let faded = p.fadespeed <= 0.0 || p.color <= 0.001;
            let not_shrinking = p.scalespeed >= 0.0 || p.radius <= 0.01;
            if (p.life < 0.001 && faded && anim_done && not_shrinking) || p.radius <= 0.0 || p.color <= 0.0 {
                return false;
            }
            // 2. fade / scale
            if p.life <= 0.0 {
                p.color -= dt * p.fadespeed * 100.0;
            }
            p.radius += dt * p.scalespeed * 100.0;
            // 3. move
            p.prev = p.origin;
            if !p.resting {
                p.vel[2] -= 800.0 * dt * p.gravity;
                if p.drag > 0.0 {
                    let k = (p.drag * dt).min(1.0);
                    p.vel[0] -= k * p.vel[0];
                    p.vel[1] -= k * p.vel[1];
                    p.vel[2] -= k * (1.0 - p.gravity.abs()).max(0.0) * p.vel[2];
                }
                p.origin = v::ma(p.origin, dt, p.vel);
            }
            p.angles = v::ma(p.angles, dt, p.avel);
            // 4. think
            if now >= p.next_think {
                p.next_think = now + think_iv as f64;
                let near = v::dist(p.origin, cam.origin) < 512.0;
                if p.randomdelta > 0.0 && near {
                    for i in 0..3 {
                        let t = r::random_float(p.randvel_lo[i], p.randvel_hi[i]);
                        p.vel[i] += (t - p.vel[i]).clamp(-p.randomdelta, p.randomdelta);
                    }
                }
                if physics && p.react > 0.0 && near {
                    for (porg, pvel) in players {
                        let d = v::sub(p.origin, *porg);
                        let dist = v::len(d);
                        if dist < 128.0 && dist > 0.01 {
                            let k = (1.0 - dist / 128.0) * v::len2d(*pvel) * p.react * think_iv;
                            p.vel = v::ma(p.vel, k / dist, d);
                            p.resting = false;
                        }
                    }
                }
                if p.wind != 0.0 {
                    p.vel = v::ma(p.vel, think_iv * p.wind, wind);
                    p.resting = false;
                }
            }
            // 5. collision (map particles; world only)
            if p.collide && !p.resting && now >= p.next_trace {
                let dist = v::dist(p.origin, cam.origin);
                let mut iv = (dist * (cam.fov / 90.0) / 8192.0).max(0.016);
                if dist > 1024.0 {
                    iv *= 2.0;
                }
                p.next_trace = now + iv as f64;
                let tr = r::player_trace(p.prev, p.origin, 2, (PM_STUDIO_BOX | PM_WORLD_ONLY) as i32, -1);
                // Particles spawned inside a brush (emitters just under a sky) aren't hits.
                if tr.fraction < 1.0 && tr.allsolid == 0 && tr.startsolid == 0 {
                    let n = tr.plane.normal;
                    p.origin = v::ma(tr.endpos, 0.5, n);
                    let damp = 0.3 * (1.0 - 7.0 * iv).max(0.0);
                    let d = v::dot(p.vel, n);
                    p.vel = v::scale(v::ma(p.vel, -2.0 * d, n), damp);
                    if v::len(p.vel) < 20.0 && n[2] > 0.7 {
                        p.resting = true;
                        p.vel = v::ZERO;
                    }
                    match p.oncollide {
                        1 => return false,
                        2 => {
                            if let Some(t) = p.target {
                                requests.push((t, p.origin));
                            }
                            return false;
                        }
                        3 => {
                            if let Some(t) = p.target {
                                requests.push((t, p.origin));
                            }
                        }
                        _ => {}
                    }
                }
            }
            // 6. animate, age
            if p.life <= 0.0 || p.kind == KIND_EFFECT {
                p.frame += dt * p.framerate;
            }
            p.life -= dt;
            true
        });
        // Clamp animation to the sprite's frames (needs the sprite table, so outside retain).
        for p in &mut self.parts {
            let n = self.sprites.get(p.sprite as usize).map_or(1, |s| s.frames) as f32;
            if p.frame > n - 1.0 {
                p.frame = n - 1.0;
                p.framerate = 0.0;
            }
        }
        self.spawn_requests.extend(requests);
    }

    /// Drawing half: unsorted TriAPI quads (`Quad_Build`/`Quad_Emit`).
    pub fn draw(&self, now: f64, dt: f32, cam: &Camera) {
        let _ = dt;
        for p in &self.parts {
            let Some(s) = self.sprites.get(p.sprite as usize) else { continue };
            if s.model.is_null() || !in_view_cone(cam, p.origin) {
                continue;
            }
            let frame = (p.frame as i32).clamp(0, s.frames - 1);
            if tri!(SpriteTexture(s.model as *mut model_s, frame)) == 0 {
                continue;
            }
            let a = if p.fadein > 0.0 { (((now - p.spawn) as f32) / p.fadein).min(1.0) } else { 1.0 } * p.alpha;
            let c = p.color.clamp(0.0, 1.0);
            let quad = build_quad(p, cam);
            tri!(RenderMode(p.rendermode));
            tri!(CullFace(TRICULLSTYLE_TRI_NONE));
            tri!(Begin(TRI_QUADS as i32));
            tri!(Color4f(c * p.rgb[0], c * p.rgb[1], c * p.rgb[2], a));
            for (uv, vtx) in [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]].iter().zip(quad.iter()) {
                tri!(TexCoord2f(uv[0], uv[1]));
                tri!(Vertex3fv(vtx.as_ptr()));
            }
            tri!(End());
        }
        tri!(RenderMode(kRenderNormal as i32));
    }
}

/// `InViewCone` (`0x100570b0`): close points always pass; otherwise a widened cone test.
pub fn in_view_cone(cam: &Camera, org: Vec3) -> bool {
    let d = v::sub(org, cam.origin);
    let dist = v::len(d);
    if dist < 300.0 {
        return true;
    }
    let dir = v::scale(d, 1.0 / dist);
    if v::dot(dir, cam.forward) < 0.0 {
        return false;
    }
    let k = (v::dot(dir, cam.right).abs() + 0.5 * v::dot(dir, cam.up).abs()) * (1.0 + dist * 0.2 / 8192.0);
    k < 0.9 * (cam.fov.max(10.0) / 90.0).max(1.0)
}

/// The four corners (uv (0,0) (1,0) (1,1) (0,1)) for `p`.
pub fn build_quad(p: &Particle, cam: &Camera) -> [Vec3; 4] {
    let (right, up) = match p.orient {
        ORIENT_ROLL => {
            let (_, r2, u2) = v::angle_vectors([cam.angles[0], cam.angles[1], p.angles[2]]);
            (r2, u2)
        }
        ORIENT_FREE => {
            let (_, r2, u2) = v::angle_vectors(p.angles);
            (r2, u2)
        }
        ORIENT_AXIS => {
            let axis = v::normalize(p.followaxis);
            let to_cam = v::sub(cam.origin, p.origin);
            let r2 = v::normalize(v::cross(axis, to_cam));
            (r2, axis)
        }
        _ => (cam.right, cam.up),
    };
    let stretchable = p.orient <= ORIENT_ROLL;
    let speed = v::len(p.vel);
    if stretchable && p.stretch > 0.01 && speed * p.stretch > 2.0 * p.radius {
        let dir = v::scale(p.vel, 1.0 / speed);
        let half = speed * p.stretch * 0.5;
        let w = v::scale(v::cross(v::sub(cam.origin, p.origin), dir), p.radius / v::len(v::sub(cam.origin, p.origin)).max(1.0));
        let a = v::ma(p.origin, -half, dir);
        let b = v::ma(p.origin, half, dir);
        return [v::sub(a, w), v::sub(b, w), v::add(b, w), v::add(a, w)];
    }
    let rr = v::scale(right, p.radius);
    let uu = v::scale(up, p.radius);
    [v::add(v::sub(p.origin, rr), uu), v::add(v::add(p.origin, rr), uu), v::sub(v::add(p.origin, rr), uu), v::sub(v::sub(p.origin, rr), uu)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wind_is_deterministic() {
        let mut a = Wind::default();
        let mut b = Wind::default();
        a.set(1234, 30, 50.0, 10.0);
        b.set(1234, 30, 50.0, 10.0);
        for i in 0..100 {
            let t = 10.0 + i as f64 * 0.05;
            a.advance(t);
            b.advance(t);
            assert_eq!(a.speed(t), b.speed(t));
            assert!(a.speed(t) <= 50.0 + 1e-3);
        }
        let w = a.velocity(12.0);
        assert!(w[2] == 0.0);
    }

    #[test]
    fn quad_billboard() {
        let cam = Camera { forward: [1.0, 0.0, 0.0], right: [0.0, -1.0, 0.0], up: [0.0, 0.0, 1.0], fov: 90.0, ..Default::default() };
        let p = Particle { origin: [100.0, 0.0, 0.0], radius: 2.0, ..Default::default() };
        let q = build_quad(&p, &cam);
        assert_eq!(q[0], [100.0, 2.0, 2.0]);
        assert_eq!(q[2], [100.0, -2.0, -2.0]);
        assert!(in_view_cone(&cam, [1000.0, 0.0, 0.0]));
        assert!(!in_view_cone(&cam, [-1000.0, 0.0, 0.0]));
    }
}
