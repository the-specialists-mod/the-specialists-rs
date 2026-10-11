//! TS sprite particle system (`g_PSys` `0x1018bde8`; rendering doc §5.2-5.4). Map emitters
//! (type 3) and every weapon effect (muzzle flashes, smoke, sparks, debris) share it.
//!
//! Units follow the TS structures where the doc gives them: `radius += dt*scalespeed*100`,
//! `color -= dt*fadespeed*100`, gravity 1.0 = 800 u/s^2. Velocities are stored in units per
//! second here (TS keeps units per 0.1 s; templates are converted on spawn).

use goldsrc::client::engine;
use goldsrc::ext::render::{self as r, Vec3, vec3 as v};
use goldsrc::ext::tri;
use goldsrc::sys::*;

pub const KIND_EFFECT: u8 = 0;
pub const KIND_MAP: u8 = 3;

/// `follow_view` quad orientations (`Quad_Build` `0x10050080`).
pub const ORIENT_BILLBOARD: u8 = 0;
pub const ORIENT_ROLL: u8 = 1;
pub const ORIENT_FREE: u8 = 2;
pub const ORIENT_AXIS: u8 = 3;

/// `oncollide` for blood (`TS_BloodImpact`'s callback `0x10048710`): queue a decal at the hit
/// ([`PSys::blood_hits`]) and kill the particle.
pub const ONCOLLIDE_BLOOD: u8 = 4;

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
    /// Explicit corners (TS type 4, drawn by `Quad_Emit`): the bullet trail.
    pub corners: Option<[Vec3; 4]>,
    /// World collision (TS: type 3 with `collide` set).
    pub collide: bool,
    /// 0 bounce, 1 kill, 2 kill + spawn target, 3 spawn target + bounce, [`ONCOLLIDE_BLOOD`].
    pub oncollide: u8,
    /// Random-drift accumulator (`+0x68`), units per second.
    pub drift: Vec3,
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
            corners: None,
            collide: false,
            oncollide: 0,
            drift: v::ZERO,
            target: None,
            resting: false,
            next_think: 0.0,
            next_trace: 0.0,
        }
    }
}

struct Sprite {
    name: String,
    model: *const model_s,
    frames: i32,
}

/// Seeded gust model (`TSPSys` subtype 3): identical gusts on every client for one seed.
#[derive(Default, Clone, Copy, Debug)]
pub struct Wind {
    pub seed: i32,
    pub yaw: f32,
    pub base: f32,
    seg_start: f64,
    seg_len: f32,
    from: f32,
    to: f32,
}

impl Wind {
    /// New seed, direction and base speed from `TSPSys`; the current gust speed carries over and
    /// the next think starts a new segment.
    pub fn set(&mut self, seed: i32, yaw: i32, speed: f32, now: f64) {
        let cur = self.speed(now);
        *self = Wind { seed, yaw: yaw as f32, base: speed, seg_start: now, seg_len: 0.0, from: cur, to: cur };
    }
    /// Cosine-interpolated gust speed (`CosineLerp` `0x10050760`).
    pub fn speed(&self, now: f64) -> f32 {
        if self.seg_len <= 0.0 {
            return self.to;
        }
        let t = (((now - self.seg_start) / self.seg_len as f64) as f32).clamp(0.0, 1.0);
        let k = (1.0 - (t * std::f32::consts::PI).cos()) * 0.5;
        self.from + (self.to - self.from) * k
    }
    /// `Particle_Think`'s segment step (`0x10051a72`): once a segment ends, the next starts now
    /// with target speed `SharedRF(seed, 0, base)` and length `SharedRF(seed, 0.001, 1)` from the
    /// same seed, then the seed becomes `SharedRL(seed, 0, 30000)`.
    pub fn advance(&mut self, now: f64) {
        use crate::events::random::{shared_random_float, shared_random_long};
        if now <= self.seg_start + self.seg_len as f64 {
            return;
        }
        let s = self.seed as u32;
        self.from = self.to;
        self.to = shared_random_float(s, 0.0, self.base.max(0.0));
        self.seg_start = now;
        self.seg_len = shared_random_float(s, 0.001, 1.0);
        self.seed = shared_random_long(s, 0, 30000);
    }
    pub fn velocity(&self, now: f64) -> Vec3 {
        let (s, c) = self.yaw.to_radians().sin_cos();
        v::scale([c, s, 0.0], self.speed(now))
    }
}

/// Effect tuning that `cl_fx` sets (`FX_ApplyCvar` `0x10057d50`, `FX_AdaptQuality` `0x10057a70`).
#[derive(Clone, Copy, Debug)]
pub struct FxTuning {
    /// `0x1018ca50`: particle count multiplier (1.5 for `cl_fx` 1/2, 0.7 for 0, `max(2q, 0.1)` adaptive).
    pub mult: f32,
    /// `0x1018ca44`: effect cooldown interval (0.05, 0.2 for `cl_fx` 0, `min(0.05/q, 0.3)` adaptive).
    pub interval: f32,
    /// `0x1018ca48`: puff cooldown interval (0.05, 0.3 for `cl_fx` 0, as `interval` adaptive).
    pub puff_interval: f32,
    /// `0x1018ca58`: muzzle dynamic lights allowed (q ≥ 0.3).
    pub dlights: bool,
    /// `0x1018ca54`: shell life (30, 5 for `cl_fx` 0, `max(30q, 2)` adaptive).
    pub shell_life: f32,
    /// `0x1018ca40`: explosion particle scale (1, 0.35 for `cl_fx` 0, `max(q, 0.25)` adaptive).
    pub scale: f32,
    /// `cl_fx` is not 0 (`TS_GunSmoke` reads the cvar).
    pub smoke: bool,
}

impl Default for FxTuning {
    fn default() -> Self {
        FxTuning { mult: 1.5, interval: 0.05, puff_interval: 0.05, dlights: true, shell_life: 30.0, scale: 1.0, smoke: true }
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
    /// Blood particles that hit the world this frame ([`ONCOLLIDE_BLOOD`]), for decals.
    pub blood_hits: Vec<Vec3>,
    pub physics: bool,
    pub divisor: f32,
    pub tuning: FxTuning,
    /// View of the last transparent pass (effects cull against it).
    pub cam: Camera,
    /// `GetCullOrigin` (`0x10057270`) of the last pass: the eye in third person, else the view origin.
    pub cull_origin: Vec3,
    pub time_scale: f32,
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
            blood_hits: Vec::new(),
            physics: true,
            divisor: 1.0,
            tuning: FxTuning::default(),
            cam: Camera::default(),
            cull_origin: v::ZERO,
            time_scale: 1.0,
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
    /// slots in TS; unlimited here. Only loaded sprites take a slot: a miss leaves none behind
    /// (`0x10050a43` stores the null handle in the free slot, which still reads as free), so TS
    /// calls `SPR_Load` again and warns on every miss. Callers look a sprite up once per effect
    /// or emission, not per particle.
    pub fn sprite(&mut self, name: &str) -> Option<u16> {
        if let Some(i) = self.sprites.iter().position(|s| s.name.eq_ignore_ascii_case(name)) {
            return Some(i as u16);
        }
        let h = engine().spr_load(name);
        let model = if h != 0 { r::sprite_model(h) } else { std::ptr::null() };
        if model.is_null() {
            // TS prints this through `CHudSayText::SayTextPrint` (0x10050a69: chat and console).
            engine().print(&format!("WARNING! cannot found {name}\n"));
            return None;
        }
        self.sprites.push(Sprite { name: name.to_owned(), model, frames: r::spr_frames(h).max(1) });
        Some((self.sprites.len() - 1) as u16)
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

    /// `PSys_Add` (`0x10050b60`): drag is at least 0.1.
    pub fn add(&mut self, mut p: Particle, now: f64) {
        if self.parts.len() >= self.max {
            // Oldest-first replacement keeps steady effects (rain) from starving new ones.
            self.parts.remove(0);
        }
        p.drag = p.drag.max(0.1);
        p.spawn = now;
        p.prev = p.origin;
        p.next_think = now;
        p.next_trace = now;
        self.parts.push(p);
    }

    /// `SlowTime`: `x / max(ts, 0.01)` (cooldowns and light lifetimes stretch in slow motion).
    pub fn slow(&self, x: f32) -> f32 {
        x / self.time_scale.max(0.01)
    }

    /// `InViewCone` (`0x100570b0`) and `PointVisible` (`0x100572a0`, a point-hull trace from the
    /// cull origin that must not hit anything), the culls most effects start with.
    pub fn visible(&self, pos: Vec3) -> bool {
        in_view_cone(&self.cam, pos) && r::player_trace(self.cull_origin, pos, 2, PM_GLASS_IGNORE as i32, -1).fraction >= 1.0
    }

    /// The velocity, resting and spin kick the line pushes give (`0x10052720`, `0x10052c80`).
    fn kick(p: &mut Particle, dv: Vec3) {
        p.vel = v::add(p.vel, dv);
        if p.resting {
            p.vel[2] = p.vel[2].abs() + 20.0;
            p.resting = false;
        }
        if p.orient == ORIENT_ROLL {
            p.avel[2] += r::random_float(-900.0, 900.0);
        }
    }

    /// `PSys_PushRadial(org, NULL, radius, strength)` (`0x100524c0`): reacting particles within
    /// `radius` get `strength·(radius − d)/radius` (TS units, ×10 here) away from `org`, without
    /// the react factor. Resting ones hop.
    pub fn push_radial(&mut self, org: Vec3, radius: f32, strength: f32) {
        if !self.physics {
            return;
        }
        for p in self.parts.iter_mut().filter(|p| p.react > 0.0) {
            let d = v::sub(p.origin, org);
            let dist = v::len(d);
            if dist > radius || radius <= 0.0 {
                continue;
            }
            let f = (radius - dist) / radius;
            let n = if dist > 0.0 { v::scale(d, 1.0 / dist) } else { v::ZERO };
            p.vel = v::ma(p.vel, strength * f * 10.0, n);
            if p.resting {
                if p.vel[2] < 0.0 {
                    p.vel[2] = p.vel[2].abs() * f * 0.5;
                }
                p.vel[2] += f * 100.0;
                p.resting = false;
            }
            if p.orient == ORIENT_ROLL {
                p.avel[2] += r::random_float(-900.0, 900.0) * f;
            }
        }
    }

    /// `PSys_PushAlongLine(a, b, radius, swirl)` (`0x10052720`): reacting particles within
    /// `radius` of the bullet line (TS's distance, see [`heron_height`]) and inside the sphere
    /// `|mid − p| < |mid − b| − 100` are pushed *along* the bullet by `(radius − h)·react`, plus a
    /// sideways swirl for some ammo.
    pub fn push_along_line(&mut self, a: Vec3, b: Vec3, radius: f32, swirl: bool) {
        if !self.physics {
            return;
        }
        let ab = v::sub(b, a);
        let len = v::len(ab);
        if len <= 0.0 {
            return;
        }
        let dir = v::scale(ab, 1.0 / len);
        let mid = v::scale(v::add(a, b), 0.5);
        let reach = v::dist(mid, b) - 100.0;
        for p in self.parts.iter_mut().filter(|p| p.react > 0.0) {
            if v::dist(mid, p.origin) >= reach {
                continue;
            }
            let h = heron_height(v::dist(p.origin, a), v::dist(p.origin, b), len);
            if h >= radius {
                continue;
            }
            let k = (radius - h) * p.react * 10.0;
            let mut dv = v::scale(dir, k);
            if swirl {
                dv = v::ma(dv, k, v::cross(dir, v::normalize(v::sub(p.origin, a))));
            }
            Self::kick(p, dv);
        }
    }

    /// `PSys_BlastPush(org, dir, radius, fwd)` (`0x10052c80`, the muzzle blast): reacting
    /// particles within `radius` of `org + dir̂·fwd` get `(radius − |dir|)/4` (TS units; the same
    /// for every particle, as in TS) away from `org`.
    pub fn blast_push(&mut self, org: Vec3, dir: Vec3, radius: f32, fwd: f32) {
        if !self.physics {
            return;
        }
        let c = v::ma(org, fwd, v::normalize(dir));
        let k = (radius - v::len(dir)) * 0.25 * 10.0;
        for p in self.parts.iter_mut().filter(|p| p.react > 0.0) {
            if v::dist(c, p.origin) <= radius {
                Self::kick(p, v::scale(v::normalize(v::sub(p.origin, org)), k));
            }
        }
    }

    /// `PSys_UpdateDraw` (`0x100520d0`), simulation half. `dt = frametime * time_scale`.
    pub fn update(&mut self, now: f64, dt: f32, cam: &Camera, players: &[(Vec3, Vec3)]) {
        self.wind.advance(now);
        let wind = self.wind.velocity(now);
        let think_iv = (self.divisor / 30.0).max(dt);
        let physics = self.physics;
        // Zoomed in, the think distances double (`gHUD.m_iFOV < 40`).
        let zoom_scale = if (cam.fov as i32) < 40 { 0.5 } else { 1.0 };
        let mut requests = Vec::new();
        let mut blood_hits = Vec::new();
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
            // 4. think (`Particle_Think` 0x10051270)
            if now >= p.next_think {
                p.next_think = now + think_iv as f64;
                let near = v::dist(p.origin, cam.origin) * zoom_scale < 512.0;
                // Random drift: steer an accumulator toward a fresh random target by at most
                // `randomdelta` (3D length), then add it to the velocity.
                if p.randomdelta != 0.0 && near {
                    let t = [0, 1, 2].map(|i| r::random_float(p.randvel_lo[i], p.randvel_hi[i]));
                    let step = v::sub(t, p.drift);
                    let len = v::len(step);
                    let step = if len > p.randomdelta.abs() { v::scale(step, p.randomdelta.abs() / len) } else { step };
                    p.drift = v::add(p.drift, step);
                    p.vel = v::ma(p.vel, think_iv, p.drift);
                }
                // React: players within 128 units push the particle's *origin* sideways, by how
                // far it lies along their horizontal motion (`dot` < 0 pulls it in behind them).
                if physics && p.react != 0.0 && !p.resting && near {
                    for (porg, pvel) in players {
                        let d = v::sub(p.origin, *porg);
                        let dist = v::len(d);
                        let speed = v::len2d(*pvel);
                        if dist > 128.0 || dist <= 0.0 || speed <= 0.0 {
                            continue;
                        }
                        let (dn, vd) = (v::scale(d, 1.0 / dist), [pvel[0] / speed, pvel[1] / speed]);
                        let dot = dn[0] * vd[0] + dn[1] * vd[1];
                        let k = (128.0 - dist) * speed * dot / 128.0 * think_iv * p.react;
                        p.origin[0] += dn[0] * k;
                        p.origin[1] += dn[1] * k;
                    }
                }
                // Wind (TS units per 0.1 s, so ×10 here).
                if physics && p.wind != 0.0 && !p.resting {
                    p.vel = v::ma(p.vel, think_iv * p.wind * 10.0, wind);
                }
            }
            // 5. collision (TS type-3 particles with `collide`; world only)
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
                        ONCOLLIDE_BLOOD => {
                            blood_hits.push(tr.endpos);
                            return false;
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
        self.blood_hits.extend(blood_hits);
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
            if !tri::sprite_texture(s.model, frame) {
                continue;
            }
            let a = if p.fadein > 0.0 { (((now - p.spawn) as f32) / p.fadein).min(1.0) } else { 1.0 } * p.alpha;
            let c = p.color.clamp(0.0, 1.0);
            let quad = build_quad(p, cam);
            tri::render_mode(p.rendermode);
            tri::cull_face(TRICULLSTYLE_TRI_NONE);
            tri::begin(TRI_QUADS as i32);
            tri::color4f(c * p.rgb[0], c * p.rgb[1], c * p.rgb[2], a);
            for (uv, vtx) in [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]].iter().zip(quad.iter()) {
                tri::tex_coord2f(uv[0], uv[1]);
                tri::vertex3fv(vtx);
            }
            tri::end();
        }
        tri::render_mode(kRenderNormal as i32);
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

/// TS's point-to-line distance (`PSys_PushAlongLine` 0x10052720 and the whizz in
/// `EV_TS_FireBullets`): Heron's formula with `s − len` where `s − b` belongs, i.e.
/// `2·sqrt(s(s − a))·(s − len)/len` for distances `a`, `b` to the ends of a `len` long line.
pub fn heron_height(a: f32, b: f32, len: f32) -> f32 {
    let s = (a + b + len) * 0.5;
    2.0 * (s * (s - a)).max(0.0).sqrt() * (s - len) / len
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
    if let Some(c) = p.corners {
        return c;
    }
    let stretchable = p.orient <= ORIENT_ROLL;
    // `stretchspeed` multiplies TS's speed (units per 0.1 s).
    let speed = v::len(p.vel) * 0.1;
    if stretchable && p.stretch > 0.01 && speed * p.stretch > 2.0 * p.radius {
        let dir = v::normalize(p.vel);
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
