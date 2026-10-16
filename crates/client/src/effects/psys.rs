//! TS sprite particle system (`g_PSys` `0x1018bde8`; rendering doc §5.2-5.4). Map emitters
//! (type 3) and every weapon effect (muzzle flashes, smoke, sparks, debris) share it.
//!
//! Units follow the TS structures where the doc gives them: `radius += dt*scalespeed*100`,
//! `color -= dt*fadespeed*100`, gravity 1.0 = `sv_gravity` u/s^2. Velocities are stored in units per
//! second here (TS keeps units per 0.1 s; templates are converted on spawn).
//!
//! The pool is TS's: a fixed array of slots (`PSys_Init` `0x100507e0`, 256..4096 of them),
//! filled at the first free slot (`PSys_Add` `0x10050b60`, which drops the new particle when
//! none is free) and walked in slot order up to the highest live one (`PSys_UpdateDraw`
//! `0x100520d0`), each particle moved and drawn in turn.

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

/// `oncollide` for blood (`TS_BloodImpact`'s callback `0x10048710`): a decal where the drop hit
/// and the particle dies ([`Collision::Blood`]).
pub const ONCOLLIDE_BLOOD: u8 = 4;

/// `oncollide` for blood whose callback only leaves the decal, with no `player/splat.wav` (TS
/// 2.1's `0x10082ba0`): otherwise as [`ONCOLLIDE_BLOOD`].
pub const ONCOLLIDE_BLOOD_DECAL: u8 = 5;

/// What a particle's collision callback (`+0x11c`) asks the caller to do, run inside the
/// particle pass at the moment TS calls it (`0x10052093`), so the particles it spawns land in
/// the pool (and are walked this frame if their slot lies ahead) and the engine random numbers
/// come in TS's order.
pub enum Collision {
    /// `oncollide` 2 (`0x10039d50`, after the particle was killed) or 3 (`0x10039f60`): one
    /// particle of template `target` at the particle's origin, inheriting its velocity
    /// (`parent_vel`) unless the template says otherwise.
    Spawn { target: u16, origin: Vec3, parent_vel: Vec3 },
    /// [`ONCOLLIDE_BLOOD`] (`splat` true) or [`ONCOLLIDE_BLOOD_DECAL`] with the trace that hit.
    Blood { tr: pmtrace_s, splat: bool },
}

#[derive(Clone, Debug)]
pub struct Particle {
    /// `+0x00`: the slot is in use. Callers leave it set (TS's spawners write 1 before
    /// `PSys_Add`); `PSys_Kill` and the blood callback clear it.
    pub alive: bool,
    pub kind: u8,
    /// `+0x114`: sprite cache slot ([`PSys::sprite`]), -1 for none (never drawn).
    pub sprite: i16,
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
    /// Grey level (TS draws `Color4f(c, c, c, fade-in)`).
    pub color: f32,
    pub fadespeed: f32,
    pub radius: f32,
    pub scalespeed: f32,
    pub frame: f32,
    pub framerate: f32,
    pub react: f32,
    pub wind: f32,
    pub fadein: f32,
    /// `+0xd4`: client time of `PSys_Add` (a float, as TS stores it).
    pub spawn: f32,
    pub orient: u8,
    pub followaxis: Vec3,
    pub stretch: f32,
    /// Explicit corners (TS type 4, drawn by `Quad_Emit`): the bullet trail.
    pub corners: Option<[Vec3; 4]>,
    /// Texture coordinates of the four corners, when not (0,0) (1,0) (1,1) (0,1): a ruleset's
    /// trails that tile one sprite over several quads. TS 3.0 sets none.
    pub uv: Option<[[f32; 2]; 4]>,
    /// World collision (TS: type 3 with `collide` set).
    pub collide: bool,
    /// 0 none, 1 kill, 2 kill + spawn target, 3 spawn target, [`ONCOLLIDE_BLOOD`],
    /// [`ONCOLLIDE_BLOOD_DECAL`].
    pub oncollide: u8,
    /// Random-drift accumulator (`+0x68`), units per second.
    pub drift: Vec3,
    pub target: Option<u16>,
    pub resting: bool,
    /// `+0x02`: drawn this frame. Set to "has a sprite" before every move; the collision step
    /// clears it outside the view cone. TS's scratch particle (`0x1004ffe0`) starts it at 0.
    pub(crate) visible: bool,
    /// `+0x110`, `+0xec`: next think and next collision trace (client time, floats).
    pub(crate) next_think: f32,
    pub(crate) next_trace: f32,
}

impl Default for Particle {
    fn default() -> Self {
        Particle {
            alive: true,
            kind: KIND_EFFECT,
            sprite: 0,
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
            uv: None,
            collide: false,
            oncollide: 0,
            drift: v::ZERO,
            target: None,
            resting: false,
            visible: false,
            next_think: 0.0,
            next_trace: 0.0,
        }
    }
}

/// One `PSys_LoadSprite` cache slot (`+0x0c`, 0x28 bytes): name, render mode key, handle, frames.
struct Sprite {
    name: String,
    mode: i16,
    handle: HSPRITE,
    frames: i16,
}

/// `PSys_LoadSprite`'s cache size.
const SPRITE_SLOTS: usize = 64;

/// Seeded gust model (`TSPSys` subtype 3, globals `0x10189438..0x10189470`): identical gusts on
/// every client for one seed. Speed and direction are cosine-interpolated from the `from` to the
/// `to` values over the current segment.
#[derive(Default, Clone, Copy, Debug)]
pub struct Wind {
    pub seed: i32,
    pub base: f32,
    speed_from: f32,
    speed_to: f32,
    dir_from: Vec3,
    dir_to: Vec3,
    seg_start: f64,
    seg_len: f32,
}

impl Wind {
    /// `MsgFunc_TSPSys` subtype 3 (`0x1004c00b..0x1004c082`): new seed, `to` direction
    /// `AngleVectors(0, yaw, 0)`, `to` speed = base = `speed`, and the segment restarts now. The
    /// `from` values and the segment length are left alone, so the gust eases from where it was
    /// to the full base speed over the old segment length.
    pub fn set(&mut self, seed: i32, yaw: i32, speed: f32, now: f64) {
        self.seed = seed;
        self.dir_to = v::angle_vectors([0.0, yaw as f32, 0.0]).0;
        self.speed_to = speed;
        self.base = speed;
        self.seg_start = now;
    }
    fn lerp_k(&self, now: f64) -> f32 {
        // `CosineLerp` (`0x10050760`) with `t = (now - start) / len`, unclamped (the step below
        // runs first, so t <= 1 except right after `TSPSys`).
        let t = ((now - self.seg_start) as f32) / self.seg_len;
        (1.0 - (t * std::f32::consts::PI).cos()) * 0.5
    }
    /// The interpolated gust speed.
    pub fn speed(&self, now: f64) -> f32 {
        let k = self.lerp_k(now);
        (1.0 - k) * self.speed_from + k * self.speed_to
    }
    /// `Particle_Think`'s segment step (`0x10051aa1..0x10051b69`), run by the think of a wind
    /// particle: once `start + len < now`, `from` takes the `to` speed and direction, the new `to`
    /// speed is `SharedRF(seed, 0, base)`, the segment starts now and lasts
    /// `SharedRF(seed, 0.001, 1)`, and the seed becomes `SharedRL(seed, 0, 30000)`.
    pub fn advance(&mut self, now: f64) {
        use crate::events::random::{shared_random_float, shared_random_long};
        if self.seg_start + self.seg_len as f64 >= now {
            return;
        }
        let s = self.seed as u32;
        self.speed_from = self.speed_to;
        self.dir_from = self.dir_to;
        self.speed_to = shared_random_float(s, 0.0, self.base);
        self.seg_start = now;
        self.seg_len = shared_random_float(s, 0.001, 1.0);
        self.seed = shared_random_long(s, 0, 30000);
    }
    /// `normalize(dir)·speed` (TS units per 0.1 s), or `None` unless the speed is positive
    /// (`0x10051bc4`).
    pub fn velocity(&self, now: f64) -> Option<Vec3> {
        let k = self.lerp_k(now);
        let speed = (1.0 - k) * self.speed_from + k * self.speed_to;
        // NaN (a zero-length segment right after `TSPSys`) fails the test too.
        if speed.is_nan() || speed <= 0.0 {
            return None;
        }
        let dir = v::add(v::scale(self.dir_from, 1.0 - k), v::scale(self.dir_to, k));
        let l = v::len(dir);
        let n = if l == 0.0 { [0.0, 0.0, 1.0] } else { v::scale(dir, 1.0 / l) };
        Some(v::scale(n, speed))
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
    /// `0x1018ca59`: low detail, which `HUD_AddEntity` gives `iuser4` bit 8 entities body 0 for
    /// (`0x10044ad7`). Set for `cl_fx` other than 1 and 2 and while the adaptive quality is below
    /// 0.3, so always the opposite of `dlights` (`0x10057d7a`, `0x10057bdb`).
    pub low_detail: bool,
    /// `0x1018ca54`: shell life (30, 5 for `cl_fx` 0, `max(30q, 2)` adaptive).
    pub shell_life: f32,
    /// `0x1018ca40`: explosion particle scale (1, 0.35 for `cl_fx` 0, `max(q, 0.25)` adaptive).
    pub scale: f32,
    /// `cl_fx` is not 0 (`TS_GunSmoke` reads the cvar).
    pub smoke: bool,
}

impl Default for FxTuning {
    fn default() -> Self {
        FxTuning {
            mult: 1.5,
            interval: 0.05,
            puff_interval: 0.05,
            dlights: true,
            low_detail: false,
            shell_life: 30.0,
            scale: 1.0,
            smoke: true,
        }
    }
}

pub struct PSys {
    /// The pool (`+0x08`): empty until [`PSys::init`] (`PSys_Init`), which TS runs on the first
    /// `TS_DrawEffects` after every `HUD_VidInit`. Until then `PSys_Add` and the update do
    /// nothing.
    parts: Vec<Particle>,
    /// `+0xc10`: the highest slot the update walks to.
    high: usize,
    /// `+0xc14`/`+0xc18`: the slots the last update found alive with `react > 0`. The pushes walk
    /// this list (and skip slots that died since), not the live pool.
    react_list: Vec<u16>,
    sprites: Vec<Sprite>,
    /// `0x1018ca5c`: the pool size `FX_ApplyCvar` asks for; [`PSys::init`] takes it.
    pub max: usize,
    /// `+0xa0c`: cooldown slots, client time floats.
    cooldown: [f32; 128],
    /// `TS_MuzzleDlight`'s timer (`0x1018a45c`): not a cooldown slot, and [`PSys::init`] keeps it.
    pub muzzle_timer: super::fx::MuzzleTimer,
    pub wind: Wind,
    pub physics: bool,
    pub divisor: f32,
    pub tuning: FxTuning,
    /// View of the last transparent pass (effects cull against it).
    pub cam: Camera,
    /// `GetCullOrigin` (`0x10057270`) of the last pass: the eye in third person, else the view origin.
    pub cull_origin: Vec3,
    pub time_scale: f32,
    /// `0x100963ec`: the engine's `cl_gravity` (`sv_gravity`), which `HUD_TempEntUpdate`
    /// (`0x1001bd25`) stores whenever temp entities are active; 800 until then.
    pub gravity: f32,
    /// `IEngineStudio.IsHardware()` (`0x100ea860`), which `PSys_Add` asks (set each frame).
    pub hardware: bool,
}

impl Default for PSys {
    fn default() -> Self {
        PSys {
            parts: Vec::new(),
            high: 0,
            react_list: Vec::new(),
            sprites: Vec::new(),
            max: 4096,
            cooldown: [0.0; 128],
            muzzle_timer: Default::default(),
            wind: Wind::default(),
            physics: true,
            divisor: 1.0,
            tuning: FxTuning::default(),
            cam: Camera::default(),
            cull_origin: v::ZERO,
            time_scale: 1.0,
            gravity: 800.0,
            hardware: true,
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

/// TS's `GetClientTime()` (a float).
fn client_time() -> f32 {
    engine().client_time()
}

impl PSys {
    /// Live particles.
    pub fn count(&self) -> usize {
        self.parts.iter().filter(|p| p.alive).count()
    }

    /// Debug summary: (nearest distance, in view cone, sample particle).
    pub fn debug(&self, cam: &Camera) -> String {
        let live = || self.parts.iter().filter(|p| p.alive);
        let near = live().map(|p| v::dist(p.origin, cam.origin)).fold(f32::MAX, f32::min);
        let inview = live().filter(|p| in_view_cone(cam, self.cull_origin, p.origin)).count();
        let sample =
            live().next().map(|p| format!("org {:?} r {:.1} c {:.2} life {:.1} spr {}", p.origin, p.radius, p.color, p.life, p.sprite));
        format!("nearest {near:.0} inview {inview} cam {:?} first {:?} pool {} high {}", cam.origin, sample, self.parts.len(), self.high)
    }

    /// `PSys_Init` (`0x100507e0`): a fresh pool of `max` slots clamped to 256..4096, an empty
    /// sprite cache, cleared cooldowns and push list.
    pub fn init(&mut self) {
        let n = self.max.clamp(256, 4096);
        self.parts = vec![Particle { alive: false, ..Default::default() }; n];
        self.high = 0;
        self.react_list.clear();
        self.sprites.clear();
        self.cooldown = [0.0; 128];
    }

    /// Sprite slot for `name` with render mode `mode` (`PSys_LoadSprite` `0x100509d0`), loading
    /// it on first use. The cache is keyed by name (case-insensitively) and mode, and holds 64
    /// sprites: once full, a new sprite is "no sprite" without a warning. Only loaded sprites
    /// take a slot: a miss leaves none behind (`0x10050a43` stores the null handle in the free
    /// slot, which still reads as free), so TS calls `SPR_Load` again and warns on every miss.
    pub fn sprite(&mut self, name: &str, mode: i32) -> Option<i16> {
        let mode = mode as i16;
        if let Some(i) = self.sprites.iter().position(|s| s.name.eq_ignore_ascii_case(name) && s.mode == mode) {
            return Some(i as i16);
        }
        if self.sprites.len() >= SPRITE_SLOTS {
            return None;
        }
        let handle = engine().spr_load(name);
        if handle == 0 {
            // `SayTextPrint(text, 1, -1)` (0x10050a69): a chat line. The HUD queues it if its state
            // is borrowed at the time.
            crate::hud::say_text_print(&crate::hud::saytext::bytes(&format!("WARNING! cannot found {name}\n")), -1);
            return None;
        }
        let frames = r::spr_frames(handle) as i16;
        self.sprites.push(Sprite { name: name.to_owned(), mode, handle, frames });
        Some((self.sprites.len() - 1) as i16)
    }

    /// The engine handle of a cache slot (0 for none).
    pub fn sprite_handle(&self, sprite: i16) -> HSPRITE {
        self.slot(sprite).map_or(0, |s| s.handle)
    }

    fn slot(&self, sprite: i16) -> Option<&Sprite> {
        usize::try_from(sprite).ok().and_then(|i| self.sprites.get(i))
    }

    /// `PSys_GetFrames` (`0x10050980`): the slot's frame count, 0 for no sprite.
    pub fn frames(&self, sprite: i16) -> i32 {
        self.slot(sprite).map_or(0, |s| s.frames as i32)
    }

    /// `PSys_CooldownReady` (`0x10050ad0`) then `PSys_SetCooldown` (`0x10050b00`): rate limiting
    /// for effect groups, on the client clock.
    pub fn cooldown_ready(&mut self, slot: usize, _now: f64, interval: f32) -> bool {
        let now = client_time();
        let c = &mut self.cooldown[slot % 128];
        if *c > now {
            return false;
        }
        *c = now + interval;
        true
    }

    /// `PSys_Add` (`0x10050b60`): the first free slot, or nothing when the pool is full (or not
    /// allocated yet), or when the sprite's cache slot was loaded for TransColor and the renderer
    /// isn't hardware. The copy starts with `prev` at its origin, no drift, not resting, the
    /// think and trace times at 0, the spawn time from the client clock, and a drag floor of 0.1.
    pub fn add(&mut self, mut p: Particle, _now: f64) {
        if self.parts.is_empty() {
            return;
        }
        if self.slot(p.sprite).is_some_and(|s| s.mode == kRenderTransColor as i16) && !self.hardware {
            return;
        }
        // `+0xc0c`, the search start, only ever moves down from 0, so the search is from 0.
        let Some(i) = self.parts.iter().position(|q| !q.alive) else { return };
        p.prev = p.origin;
        p.drift = v::ZERO;
        p.resting = false;
        p.next_trace = 0.0;
        p.spawn = client_time();
        if p.drag <= 0.1 {
            p.drag = 0.1;
        }
        p.next_think = 0.0;
        self.parts[i] = p;
        self.high = self.high.max(i);
    }

    /// `PSys_Kill` (`0x10050b20`): the slot is free and its life 0.
    fn kill(&mut self, i: usize) {
        self.parts[i].alive = false;
        self.parts[i].life = 0.0;
    }

    /// `SlowTime`: `x / max(ts, 0.01)` (cooldowns and light lifetimes stretch in slow motion).
    pub fn slow(&self, x: f32) -> f32 {
        x / self.time_scale.max(0.01)
    }

    /// `InViewCone` (`0x100570b0`) and `PointVisible` (`0x100572a0`, a point-hull trace from the
    /// cull origin, ignoring the local player, that must not hit anything), the culls most effects
    /// start with.
    pub fn visible(&self, pos: Vec3) -> bool {
        in_view_cone(&self.cam, self.cull_origin, pos) && point_visible(self.cull_origin, pos)
    }

    /// The live particles of the last update's push list (`+0xc14`), by slot.
    fn pushed(&self) -> Vec<usize> {
        self.react_list.iter().map(|&i| i as usize).filter(|&i| self.parts[i].alive).collect()
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

    /// `PSys_PushRadial(org, NULL, radius, strength)` (`0x100524c0`): particles on the push list
    /// within `radius` get `strength·(radius − d)/radius` (TS units, ×10 here) away from `org`,
    /// without the react factor. Resting ones hop.
    pub fn push_radial(&mut self, org: Vec3, radius: f32, strength: f32) {
        if !self.physics {
            return;
        }
        for i in self.pushed() {
            let p = &mut self.parts[i];
            let d = v::sub(p.origin, org);
            let dist = v::len(d);
            if dist > radius {
                continue;
            }
            let f = (radius - dist) / radius;
            let n = v::normalize(d);
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

    /// `PSys_PushAlongLine(a, b, radius, swirl)` (`0x10052720`): particles on the push list within
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
        for i in self.pushed() {
            let p = &mut self.parts[i];
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

    /// `PSys_BlastPush(org, dir, radius, fwd)` (`0x10052c80`, the muzzle blast): particles on the
    /// push list within `radius` of `org + dir̂·fwd` get `(radius − |dir|)/4` (TS units; the same
    /// for every particle, as in TS) away from `org`. A zero vector normalises to (0, 0, 1).
    pub fn blast_push(&mut self, org: Vec3, dir: Vec3, radius: f32, fwd: f32) {
        if !self.physics {
            return;
        }
        // TS also needs `|dir| <= radius` (0x10052e8d..0x10052ea5).
        if v::len(dir) > radius {
            return;
        }
        let c = v::ma(org, fwd, normalize_up(dir));
        let k = (radius - v::len(dir)) * 0.25 * 10.0;
        for i in self.pushed() {
            let p = &mut self.parts[i];
            if v::dist(c, p.origin) <= radius {
                Self::kick(p, v::scale(normalize_up(v::sub(p.origin, org)), k));
            }
        }
    }

    /// `PSys_UpdateDraw` (`0x100520d0`). `dt = frametime * time_scale`. Slots are walked in order
    /// up to the highest one in use (which particles spawned during the walk can raise); per live
    /// particle, in TS's order: the death test, the push-list entry, fade and scale,
    /// `Particle_Move` (`0x10050ce0`: gravity, drag, think, move, collision with its callback,
    /// quad spin), the draw, then the frame and life steps. A particle that dies in its
    /// collision is still drawn and stepped that frame, as in TS; when the callback reuses its
    /// slot (`oncollide` 2 kills, then spawns into the first free slot), the rest of the pass
    /// works on the new particle there. `collision` runs each callback the moment TS calls it.
    pub fn update_draw(&mut self, dt: f32, cam: &Camera, players: &[(Vec3, Vec3)], collision: &mut dyn FnMut(&mut PSys, Collision)) {
        if self.parts.is_empty() {
            return;
        }
        let now = client_time();
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
            // 1. death (0x10052174..0x10052227): all four must hold. No sprite counts as an
            // animation that is over.
            let anim_done = p.sprite < 0 || p.framerate <= 0.001 || frames <= p.frame as i32;
            let faded = p.fadespeed <= 0.001 || p.color < 0.001;
            let not_shrinking = p.scalespeed >= -0.001 || p.radius < 0.001;
            if p.life < 0.001 && anim_done && faded && not_shrinking {
                self.kill(i);
                i += 1;
                continue;
            }
            if p.react > 0.0 {
                self.react_list.push(i as u16);
            }
            // 2. fade once the life is over, clamped to 0..1 (0x10052266..0x100522ca); scale.
            if p.fadespeed != 0.0 && p.life <= 0.0 {
                let mut c = p.color - dt * p.fadespeed * 100.0;
                if c <= 0.0 {
                    c = 0.0;
                }
                p.color = if c > 1.0 { 1.0 } else { c };
            }
            if p.scalespeed != 0.0 {
                p.radius += dt * p.scalespeed * 100.0;
            }
            p.visible = p.sprite >= 0;
            // 3. `Particle_Move`.
            self.particle_move(i, now, dt, cam, players, collision);
            // 4. draw (0x10052333..0x10052436), then the frame and life steps (0x1005243d..).
            let p = &self.parts[i];
            if p.visible {
                self.draw_one(p, now, cam);
            }
            let p = &mut self.parts[i];
            if p.framerate != 0.0 && p.life < 0.001 {
                p.frame += dt * p.framerate;
            }
            p.life -= dt;
            i += 1;
        }
        self.high = last;
    }

    /// `Particle_Move` (`0x10050ce0`) for slot `i`, up to the quad (built when drawn). Nothing
    /// moves or thinks while resting (0x10050de1).
    fn particle_move(
        &mut self,
        i: usize,
        now: f32,
        dt: f32,
        cam: &Camera,
        players: &[(Vec3, Vec3)],
        collision: &mut dyn FnMut(&mut PSys, Collision),
    ) {
        let think_iv = (self.divisor * 0.033_333_335).max(dt);
        let (physics, sv_gravity, cull) = (self.physics, self.gravity, self.cull_origin);
        // Zoomed in, the think and trace distances halve (`gHUD.m_iFOV < 40`, 0x10051276, 0x10051e0b).
        let zoom_scale = if (cam.fov as i32) < 40 { 0.5 } else { 1.0 };
        let p = &mut self.parts[i];
        if !p.resting {
            // `vel.z -= dt·gravity·0.1·cl_gravity` in TS units (0x10050e0c).
            p.vel[2] -= sv_gravity * dt * p.gravity;
            let k = (p.drag * dt).min(1.0);
            p.vel[0] -= k * p.vel[0];
            p.vel[1] -= k * p.vel[1];
            p.vel[2] -= k * (1.0 - p.gravity.abs()).max(0.0) * p.vel[2];
            // `Particle_Think` (0x10051270) runs before the move (0x10050f84).
            if now > p.next_think {
                let near = v::dist(p.origin, cull) * zoom_scale < 512.0;
                // Random drift: steer an accumulator toward a fresh random target by at
                // most `randomdelta` (3D length), then add it to the velocity.
                if p.randomdelta != 0.0 && near {
                    let t = [0, 1, 2].map(|i| r::random_float(p.randvel_lo[i], p.randvel_hi[i]));
                    let step = v::sub(t, p.drift);
                    let len = v::len(step);
                    let step = if len > p.randomdelta { v::scale(step, p.randomdelta / len) } else { step };
                    p.drift = v::add(p.drift, step);
                    p.vel = v::ma(p.vel, think_iv, p.drift);
                }
                // React (0x10051566..0x10051a5f): players within 128 units move the
                // particle's *origin* along the direction to it, by how far it lies along
                // their horizontal motion (`dot` < 0 pulls it in behind them).
                if physics && p.react != 0.0 && near {
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
                // Wind (0x10051a72): the segment steps here, inside a wind particle's think.
                // TS units per 0.1 s, so x10 here.
                if physics && p.wind != 0.0 {
                    self.wind.advance(now as f64);
                    if let Some(w) = self.wind.velocity(now as f64) {
                        p.vel = v::ma(p.vel, think_iv * p.wind * 10.0, w);
                    }
                }
                p.next_think = now + think_iv;
            }
            p.origin = v::ma(p.origin, dt, p.vel);
        }
        // `Particle_Collide` (0x10051d50) for TS type-3 particles that aren't resting.
        if !p.resting {
            if p.collide {
                self.collide(i, now, cam, zoom_scale, collision);
            } else {
                p.prev = p.origin;
            }
        }
        // The quad step spins moving rolling (z) and free (all) quads (0x10051076, 0x100510b3).
        let p = &mut self.parts[i];
        if !p.resting {
            match p.orient {
                ORIENT_ROLL => p.angles[2] += dt * p.avel[2],
                ORIENT_FREE => p.angles = v::ma(p.angles, dt, p.avel),
                _ => {}
            }
        }
    }

    /// `Particle_Collide` (`0x10051d50`): the trace covers the whole path since the last trace
    /// (`prev` only moves on after one, 0x10052098), only inside the view cone (which also hides
    /// the particle), at `max(d·zoom/8192 (×2 beyond 1024), 0.016)` s. It is TS's
    /// `EV_PlayerTrace(prev, origin, PM_STUDIO_BOX, -1)` with the point hull, against whatever
    /// physents the engine has set up.
    fn collide(&mut self, i: usize, now: f32, cam: &Camera, zoom_scale: f32, collision: &mut dyn FnMut(&mut PSys, Collision)) {
        let cull = self.cull_origin;
        let ts = self.time_scale;
        let p = &mut self.parts[i];
        let moved = v::sub(p.prev, p.origin);
        if v::dot(moved, moved) < 0.001 || !p.visible {
            return;
        }
        p.visible = in_view_cone(cam, cull, p.origin);
        if !p.visible || p.next_trace > now {
            return;
        }
        let d = v::dist(p.origin, cull) * zoom_scale;
        let mut iv = d * (1.0 / 8192.0);
        if d > 1024.0 {
            iv *= 2.0;
        }
        if iv <= 0.016 {
            iv = 0.016;
        }
        p.next_trace = iv + now;
        let tr = r::player_trace(p.prev, p.origin, 2, PM_STUDIO_BOX as i32, -1);
        if tr.fraction == 0.0 {
            // Stuck from the start: young particles die, older ones rest.
            if p.spawn + 0.5 <= now {
                rest(p);
            } else {
                self.kill(i);
            }
        } else if tr.fraction != 1.0 {
            // A start inside a solid kills it, but TS finishes the bounce and the callback
            // (0x10051f3e).
            if r::point_contents(p.prev) == CONTENTS_SOLID {
                self.kill(i);
            }
            let p = &mut self.parts[i];
            p.origin = tr.endpos;
            let n = tr.plane.normal;
            let travelled = v::dist(p.origin, p.prev);
            p.vel = v::scale(p.vel, 0.3 * (1.0 - 7.0 * iv).max(0.0));
            let dn = v::dot(n, p.vel);
            p.vel = v::ma(p.vel, -2.0 * dn, n);
            // Rests once a trace interval moves it less than 3·drag (0x10052059).
            if ts * travelled < p.drag * 3.0 {
                rest(p);
            }
            let (origin, parent_vel, p_oncollide) = (p.origin, p.vel, p.oncollide);
            match (p_oncollide, p.target) {
                (1, _) => self.kill(i),
                (2, target) => {
                    self.kill(i);
                    if let Some(target) = target {
                        collision(self, Collision::Spawn { target, origin, parent_vel });
                    }
                }
                (3, Some(target)) => collision(self, Collision::Spawn { target, origin, parent_vel }),
                (ONCOLLIDE_BLOOD | ONCOLLIDE_BLOOD_DECAL, _) => {
                    collision(self, Collision::Blood { tr, splat: p_oncollide == ONCOLLIDE_BLOOD });
                    // The callback clears only the in-use byte (0x10048826).
                    self.parts[i].alive = false;
                }
                _ => {}
            }
        }
        let p = &mut self.parts[i];
        p.prev = p.origin;
    }

    /// One particle's quad (`0x10052333..0x10052436`): frame `(int)min(frame, frames − 1)` before
    /// this frame's step, the cache slot's render mode, `Color4f(c, c, c, fade-in)`.
    fn draw_one(&self, p: &Particle, now: f32, cam: &Camera) {
        let Some(s) = self.slot(p.sprite) else { return };
        let last = (s.frames as i32 - 1) as f32;
        let frame = if p.frame < last { p.frame } else { last } as i32;
        let mut a = 1.0;
        if p.fadein > 0.0 && p.spawn + p.fadein > now {
            a = (now - p.spawn) / p.fadein;
        }
        let model = r::sprite_model(s.handle);
        if model.is_null() || !tri::sprite_texture(model, frame) {
            return;
        }
        let c = p.color;
        let quad = build_quad(p, cam);
        tri::render_mode(s.mode as i32);
        tri::cull_face(TRICULLSTYLE_TRI_NONE);
        tri::begin(TRI_QUADS as i32);
        tri::color4f(c, c, c, a);
        for (uv, vtx) in p.uv.unwrap_or([[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]).iter().zip(quad.iter()) {
            tri::tex_coord2f(uv[0], uv[1]);
            tri::vertex3fv(vtx);
        }
        tri::end();
        tri::render_mode(kRenderNormal as i32);
    }
}

/// `PointVisible` (`0x100572a0`): a point-hull `EV_PlayerTrace` from `from` (the cull origin) to
/// `to` with `PM_GLASS_IGNORE`, ignoring the local player, that must not hit anything.
pub fn point_visible(from: Vec3, to: Vec3) -> bool {
    let local = engine().local_player().map_or(0, |e| e.index);
    r::player_trace(from, to, 2, PM_GLASS_IGNORE as i32, local).fraction == 1.0
}

/// TS's normalise with a zero vector giving (0, 0, 1) (`0x1003d5c0` + `0x1001fb70`).
fn normalize_up(a: Vec3) -> Vec3 {
    let l = v::len(a);
    if l == 0.0 { [0.0, 0.0, 1.0] } else { v::scale(a, 1.0 / l) }
}

/// `Particle_Rest` (`0x10051d10`): resting, no velocity, no roll and no spin.
fn rest(p: &mut Particle) {
    p.resting = true;
    p.vel = v::ZERO;
    p.avel[2] = 0.0;
    p.angles[2] = 0.0;
}

/// `InViewCone` (`0x100570b0`), measured from the cull origin (`GetCullOrigin` `0x10057270`):
/// points within 300 units pass; otherwise the point must be in front and
/// `(|d̂·right| + 0.5·|d̂·up|)·(1 + dist·0.2/8192) <= 0.9` (no FOV term).
pub fn in_view_cone(cam: &Camera, cull: Vec3, org: Vec3) -> bool {
    let d = v::sub(org, cull);
    let dist = v::len(d);
    if dist <= 300.0 {
        return true;
    }
    let dir = v::scale(d, 1.0 / dist);
    if v::dot(dir, cam.forward) < 0.0 {
        return false;
    }
    let k = (v::dot(dir, cam.right).abs() + 0.5 * v::dot(dir, cam.up).abs()) * (1.0 + dist * (1.0 / 8192.0) * 0.2);
    k <= 0.9
}

/// TS's point-to-line distance (`PSys_PushAlongLine` 0x10052720 and the whizz in
/// `EV_TS_FireBullets`): Heron's formula with `s − len` where `s − b` belongs, i.e.
/// `2·sqrt(s(s − a))·(s − len)/len` for distances `a`, `b` to the ends of a `len` long line.
pub fn heron_height(a: f32, b: f32, len: f32) -> f32 {
    let s = (a + b + len) * 0.5;
    2.0 * (s * (s - a)).max(0.0).sqrt() * (s - len) / len
}

/// The four corners (uv (0,0) (1,0) (1,1) (0,1)) for `p`: `Quad_Build` (`0x10050080`) with the
/// arguments `Particle_Move` (`0x10050ce0`) passes it.
///
/// A resting particle ignores `follow_view` and is a screen billboard (`0x10051151`). Stretching
/// (`stretchspeed > 0.01` and `|vel|·stretch > 2·radius`) applies to billboards, rolling quads and
/// resting particles: the quad runs from the head (`origin + vel̂·L/2`, v = 0) to the tail (v = 1),
/// `radius·(toCam̂ × vel̂)` wide (u), so its width shrinks as the velocity turns toward the camera.
/// `follow_view` 3 uses `followaxis` unnormalised for the height and `toCam̂ × followaxis` for the
/// width, as TS does.
pub fn build_quad(p: &Particle, cam: &Camera) -> [Vec3; 4] {
    if let Some(c) = p.corners {
        return c;
    }
    // `0x10051190`: a zero vector normalises to (0, 0, 1).
    let to_cam_n = || {
        let d = v::sub(cam.origin, p.origin);
        let l = v::len(d);
        if l == 0.0 { [0.0, 0.0, 1.0] } else { v::scale(d, 1.0 / l) }
    };
    let orient = if p.resting { ORIENT_BILLBOARD } else { p.orient };
    // Every `follow_view` but 2 and 3 (4 and up act as 0) can stretch (0x10051151).
    if !matches!(orient, ORIENT_FREE | ORIENT_AXIS) && p.stretch > 0.01 {
        // `stretchspeed` multiplies TS's speed (units per 0.1 s).
        let speed = v::len(p.vel) * 0.1;
        let l = speed * p.stretch;
        if 2.0 * p.radius < l {
            let dir = v::scale(p.vel, 1.0 / v::len(p.vel));
            let half = v::scale(dir, l * 0.5);
            let w = v::scale(v::cross(to_cam_n(), dir), p.radius);
            let head = v::add(p.origin, half);
            let tail = v::sub(p.origin, half);
            return [v::sub(head, w), v::add(head, w), v::add(tail, w), v::sub(tail, w)];
        }
    }
    let (right, up) = match orient {
        ORIENT_ROLL => {
            let (_, r2, u2) = v::angle_vectors([cam.angles[0], cam.angles[1], p.angles[2]]);
            (r2, u2)
        }
        ORIENT_FREE => {
            let (_, r2, u2) = v::angle_vectors(p.angles);
            (r2, u2)
        }
        ORIENT_AXIS => (v::cross(to_cam_n(), p.followaxis), p.followaxis),
        _ => (cam.right, cam.up),
    };
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
        for i in 1..100 {
            let t = 10.0 + i as f64 * 0.05;
            a.advance(t);
            b.advance(t);
            assert_eq!(a.speed(t), b.speed(t));
            assert!(a.speed(t) <= 50.0 + 1e-3);
        }
        let w = a.velocity(15.0).expect("a positive gust");
        assert!(w[2].abs() < 1e-6);
        // A new message eases toward the full base speed over the old segment length.
        a.set(77, 90, 80.0, 15.0);
        assert!(a.speed(15.0) < 80.0);
    }

    #[test]
    fn quad_billboard() {
        let cam = Camera { forward: [1.0, 0.0, 0.0], right: [0.0, -1.0, 0.0], up: [0.0, 0.0, 1.0], fov: 90.0, ..Default::default() };
        let p = Particle { origin: [100.0, 0.0, 0.0], radius: 2.0, ..Default::default() };
        let q = build_quad(&p, &cam);
        assert_eq!(q[0], [100.0, 2.0, 2.0]);
        assert_eq!(q[2], [100.0, -2.0, -2.0]);
        assert!(in_view_cone(&cam, cam.origin, [1000.0, 0.0, 0.0]));
        assert!(!in_view_cone(&cam, cam.origin, [-1000.0, 0.0, 0.0]));
        assert!(in_view_cone(&cam, cam.origin, [-300.0, 0.0, 0.0]), "300 units still pass");
    }

    /// `Quad_Build`'s stretch (`0x100500af..0x10050729`): head first (v = 0), width across u,
    /// `radius·(toCam̂ × vel̂)`.
    #[test]
    fn quad_stretch() {
        let cam = Camera { forward: [1.0, 0.0, 0.0], right: [0.0, -1.0, 0.0], up: [0.0, 0.0, 1.0], fov: 90.0, ..Default::default() };
        let p = Particle { origin: [100.0, 0.0, 0.0], vel: [0.0, 0.0, 1000.0], stretch: 1.0, radius: 2.0, ..Default::default() };
        assert_eq!(build_quad(&p, &cam), [[100.0, -2.0, 50.0], [100.0, 2.0, 50.0], [100.0, 2.0, -50.0], [100.0, -2.0, -50.0]]);
        // Too slow to stretch (L = 100·0.03 = 3 < 2r): a plain billboard.
        let p = Particle { stretch: 0.03, ..p };
        assert_eq!(build_quad(&p, &cam)[0], [100.0, 2.0, 2.0]);
        // Resting: a billboard whatever `follow_view` says.
        let p = Particle { orient: ORIENT_AXIS, resting: true, ..p };
        assert_eq!(build_quad(&p, &cam)[0], [100.0, 2.0, 2.0]);
    }
}
