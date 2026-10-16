//! TS sprite particle system (`g_PSys` `0x1018bde8`; rendering doc §5.2-5.4). Map emitters
//! (type 3) and every weapon effect (muzzle flashes, smoke, sparks, debris) share it.
//!
//! Units are TS's own: `radius += dt*scalespeed*100`, `color -= dt*fadespeed*100`, gravity 1.0 =
//! a tenth of `sv_gravity`, and velocities and angular velocities per 0.1 s (`origin += vel ×
//! f32(dt × 10)`), so every sum TS rounds is rounded in the same units (issue #286). Each step
//! keeps TS's float forms: an x87 value is an `f64`, a value TS stores a `f32`.
//!
//! The pool is TS's: a fixed array of slots (`PSys_Init` `0x100507e0`, 256..4096 of them),
//! filled at the first free slot (`PSys_Add` `0x10050b60`, which drops the new particle when
//! none is free) and walked in slot order up to the highest live one (`PSys_UpdateDraw`
//! `0x100520d0`), each particle moved and drawn in turn.

use goldsrc::client::engine;
use goldsrc::ext::render::{self as r, Vec3, vec3 as v};
use goldsrc::ext::tri;
use goldsrc::sys::*;

mod decel;
pub use decel::decel_step;

/// The particle update [`PSys::update_draw`] runs, a ruleset's choice
/// ([`ClientRules::particle_model`](crate::gameplay::ClientRules::particle_model)).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ParticleModel {
    /// `PSys_UpdateDraw` (`0x100520d0`): gravity `g · sv_gravity`, drag, the think step (drift,
    /// react, wind), resting, stretch, fade-in; a 4096-slot pool and 64 sprites keyed by name and
    /// render mode.
    #[default]
    Drag,
    /// The older update ([`decel`]): gravity a tenth of `Drag`'s and only when positive, a fixed
    /// deceleration toward 3.3 TS units (33 u/s) instead of drag, no think step, its own collision (a particle
    /// that lands stops for good), life on the raw slow-motion factor, no stretch or fade-in; a
    /// 2048-slot pool and 32 sprites keyed by name.
    Decelerate,
}

impl ParticleModel {
    /// `PSys_Init`'s upper pool size.
    fn pool_max(self) -> usize {
        match self {
            ParticleModel::Drag => 4096,
            ParticleModel::Decelerate => 2048,
        }
    }

    /// `PSys_LoadSprite`'s cache size.
    fn sprite_slots(self) -> usize {
        match self {
            ParticleModel::Drag => 64,
            ParticleModel::Decelerate => 32,
        }
    }
}

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
    /// `+0x38`: TS units, per 0.1 s.
    pub vel: Vec3,
    pub angles: Vec3,
    /// `+0x44`: degrees per 0.1 s.
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
    /// Random-drift accumulator (`+0x68`), TS units.
    pub drift: Vec3,
    pub target: Option<u16>,
    pub resting: bool,
    /// `+0x02`: drawn this frame. Set to "has a sprite" before every move; the collision step
    /// clears it outside the view cone. TS's scratch particle (`0x1004ffe0`) starts it at 0.
    pub(crate) visible: bool,
    /// `+0x110`, `+0xec`: next think and next collision trace (client time, floats).
    pub(crate) next_think: f32,
    pub(crate) next_trace: f32,
    /// [`ParticleModel::Decelerate`]'s life clock: the client time of the last life step (the
    /// add's time at first).
    pub(crate) life_clock: f32,
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
            life_clock: 0.0,
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
    /// `CosineLerp` (`0x10050760`)'s weight, in doubles: `(1 − cos(t·π)) · 0.5` with
    /// `t = f32((now − start) / len)` (the difference unrounded), unclamped (the step below runs
    /// first, so t <= 1 except right after `TSPSys`).
    fn lerp_k(&self, now: f64) -> f64 {
        let t = ((now - self.seg_start) / self.seg_len as f64) as f32;
        (1.0 - (t as f64 * std::f64::consts::PI).cos()) * 0.5
    }
    /// `CosineLerp(a, b)`: `(1 − k)·a + k·b` on the x87, stored by the caller.
    fn lerp(k: f64, a: f32, b: f32) -> f32 {
        ((1.0 - k) * a as f64 + k * b as f64) as f32
    }
    /// The interpolated gust speed.
    pub fn speed(&self, now: f64) -> f32 {
        Self::lerp(self.lerp_k(now), self.speed_from, self.speed_to)
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
    /// The gust's direction and speed (TS units per 0.1 s), or `None` unless the speed is positive
    /// (`0x10051a72..0x10051bc4`): each direction component and the speed a stored `CosineLerp`,
    /// the direction times the x87 reciprocal of its length `(z² + y²) + x²`.
    pub fn velocity(&self, now: f64) -> Option<(Vec3, f32)> {
        let k = self.lerp_k(now);
        let speed = Self::lerp(k, self.speed_from, self.speed_to);
        // NaN (a zero-length segment right after `TSPSys`) fails the test too.
        if speed.is_nan() || speed <= 0.0 {
            return None;
        }
        let dir = [0, 1, 2].map(|i| Self::lerp(k, self.dir_from[i], self.dir_to[i]));
        let d = dir.map(|c| c as f64);
        let l = ((d[2] * d[2] + d[1] * d[1]) + d[0] * d[0]).sqrt();
        let n = if l == 0.0 { [0.0, 0.0, 1.0] } else { dir.map(|c| (c as f64 * (1.0 / l)) as f32) };
        Some((n, speed))
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
    /// The ruleset's update (set each frame).
    pub model: ParticleModel,
    /// The ruleset's `client.dll` build, whose float forms the view cone follows (set each frame).
    pub build: crate::gameplay::DllBuild,
    /// The last `TSSlowMo` factor as sent ÷ 100 (0 at normal speed), which
    /// [`ParticleModel::Decelerate`] runs the particles' life on (set each frame).
    pub slowmo: f32,
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
            model: ParticleModel::Drag,
            build: crate::gameplay::DllBuild::Optimised,
            slowmo: 0.0,
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

/// A velocity in units per second as TS's particle units (per 0.1 s): `v / 10.0f` per component
/// (`Vector::operator/`, 3.0 `0x1003efb0`, 2.1 `0x10069fd0`), as the fire events hand the
/// shooter's velocity to the muzzle flash and gun smoke.
pub fn ts_units(v: Vec3) -> Vec3 {
    v.map(|c| c / 10.0)
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
        let inview = live().filter(|p| in_view_cone(self.build, cam, self.cull_origin, p.origin)).count();
        let sample =
            live().next().map(|p| format!("org {:?} r {:.1} c {:.2} life {:.1} spr {}", p.origin, p.radius, p.color, p.life, p.sprite));
        format!("nearest {near:.0} inview {inview} cam {:?} first {:?} pool {} high {}", cam.origin, sample, self.parts.len(), self.high)
    }

    /// `PSys_Init` (`0x100507e0`): a fresh pool of `max` slots clamped to 256..4096, an empty
    /// sprite cache, cleared cooldowns and push list. (With [`ParticleModel::Decelerate`]
    /// [`PSys::add`] uses only the first 2048.)
    pub fn init(&mut self) {
        let n = self.max.clamp(256, ParticleModel::Drag.pool_max());
        self.parts = vec![Particle { alive: false, ..Default::default() }; n];
        self.high = 0;
        self.react_list.clear();
        self.sprites.clear();
        self.cooldown = [0.0; 128];
    }

    /// Sprite slot for `name` with render mode `mode` (`PSys_LoadSprite` `0x100509d0`), loading
    /// it on first use. The cache is keyed by name (case-insensitively) and mode, and holds 64
    /// sprites: once full, a new sprite is "no sprite" without a warning. With
    /// [`ParticleModel::Decelerate`] (`0x1008fd20`) the key is the name alone, so a sprite keeps
    /// the render mode it was first loaded with, and the cache holds 32. Only loaded sprites
    /// take a slot: a miss leaves none behind (`0x10050a43` stores the null handle in the free
    /// slot, which still reads as free), so TS calls `SPR_Load` again and warns on every miss.
    pub fn sprite(&mut self, name: &str, mode: i32) -> Option<i16> {
        let mode = mode as i16;
        let by_name = self.model == ParticleModel::Decelerate;
        if let Some(i) = self.sprites.iter().position(|s| s.name.eq_ignore_ascii_case(name) && (by_name || s.mode == mode)) {
            return Some(i as i16);
        }
        if self.sprites.len() >= self.model.sprite_slots() {
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
    /// [`ParticleModel::Decelerate`] (`0x1008ffc0`) searches the first 2048 slots and has no drag
    /// (its life clock starts at the spawn time). False when nothing was added (TS's −1, on
    /// which every builder stops).
    pub fn add(&mut self, mut p: Particle, _now: f64) -> bool {
        if self.parts.is_empty() {
            return false;
        }
        if self.slot(p.sprite).is_some_and(|s| s.mode == kRenderTransColor as i16) && !self.hardware {
            return false;
        }
        // `+0xc0c`, the search start, only ever moves down from 0, so the search is from 0.
        let cap = self.parts.len().min(self.model.pool_max());
        let Some(i) = self.parts[..cap].iter().position(|q| !q.alive) else { return false };
        p.prev = p.origin;
        p.drift = v::ZERO;
        p.resting = false;
        p.next_trace = 0.0;
        p.spawn = client_time();
        p.life_clock = p.spawn;
        if p.drag <= 0.1 {
            p.drag = 0.1;
        }
        p.next_think = 0.0;
        self.parts[i] = p;
        self.high = self.high.max(i);
        true
    }

    /// `PSys_Kill` (`0x10050b20`): the slot is free and its life 0.
    fn kill(&mut self, i: usize) {
        self.parts[i].alive = false;
        self.parts[i].life = 0.0;
    }

    /// `SlowTime` (`0x10056fa0`): `(1 / max(ts, 0.01f)) × x` (cooldowns and light lifetimes
    /// stretch in slow motion), stored as a float as its callers pass it on.
    pub fn slow(&self, x: f32) -> f32 {
        self.slow_x87(x) as f32
    }

    /// `SlowTime` as it leaves the x87, unrounded (a caller that compares it directly).
    pub fn slow_x87(&self, x: f32) -> f64 {
        let ts = self.time_scale;
        let m = if ts < 0.01 { 0.01f32 } else { ts };
        1.0 / m as f64 * x as f64
    }

    /// `InViewCone` (`0x100570b0`) and `PointVisible` (`0x100572a0`, a point-hull trace from the
    /// cull origin, ignoring the local player, that must not hit anything), the culls most effects
    /// start with.
    pub fn visible(&self, pos: Vec3) -> bool {
        self.in_view(pos) && point_visible(self.cull_origin, pos)
    }

    /// [`in_view_cone`] from the last pass's camera and cull origin, in the build's float forms.
    pub fn in_view(&self, pos: Vec3) -> bool {
        in_view_cone(self.build, &self.cam, self.cull_origin, pos)
    }

    /// The live particles of the last update's push list (`+0xc14`), by slot.
    fn pushed(&self) -> Vec<usize> {
        self.react_list.iter().map(|&i| i as usize).filter(|&i| self.parts[i].alive).collect()
    }

    /// The resting hop and spin kick the line pushes give after the velocity (`0x10052c04..`,
    /// `0x10052f60..`): a resting particle gets `|vel.z| + 2` and moves again, a rolling quad
    /// `RandomFloat(−90, 90)` more spin.
    fn kick(p: &mut Particle) {
        if p.resting {
            p.vel[2] = p.vel[2].abs() + 2.0;
            p.resting = false;
        }
        if p.orient == ORIENT_ROLL {
            p.avel[2] += r::random_float(-90.0, 90.0);
        }
    }

    /// `PSys_PushRadial(org, NULL, radius, strength)` (`0x100524c0`): particles on the push list
    /// within `radius` get `strength·(radius − d)/radius` away from `org`, without the react
    /// factor. Resting ones hop. `f = f32((r − d)/r)` from the x87 length; x's push is one x87
    /// expression, y's and z's store `n × strength` and `× f` first.
    pub fn push_radial(&mut self, org: Vec3, radius: f32, strength: f32) {
        if !self.physics {
            return;
        }
        for i in self.pushed() {
            let p = &mut self.parts[i];
            let d = v::sub(p.origin, org);
            let dist = len_x87(d);
            if dist > radius as f64 {
                continue;
            }
            let n = norm_x87(d, d);
            let f = ((radius as f64 - dist) / radius as f64) as f32;
            let (sf, ff) = (strength as f64, f as f64);
            p.vel[0] = (n[0] as f64 * sf * ff + p.vel[0] as f64) as f32;
            for (v, n) in p.vel.iter_mut().zip(n).skip(1) {
                *v = (((n * strength) as f64 * ff) as f32 as f64 + *v as f64) as f32;
            }
            if p.resting {
                if p.vel[2] < 0.0 {
                    p.vel[2] = (p.vel[2].abs() as f64 * ff * 0.5) as f32;
                }
                p.vel[2] = (ff * 10.0 + p.vel[2] as f64) as f32;
                p.resting = false;
            }
            if p.orient == ORIENT_ROLL {
                p.avel[2] = (r::random_float(-90.0, 90.0) as f64 * ff + p.avel[2] as f64) as f32;
            }
        }
    }

    /// `PSys_PushAlongLine(a, b, radius, swirl)` (`0x10052720`): particles on the push list within
    /// `radius` of the bullet line (TS's distance, see [`heron_height`]) and inside the sphere
    /// `|mid − p| < |mid − b| − 100` are pushed *along* the bullet by `(radius − h)·react`, and,
    /// for some ammo, sideways by the same amount in a second add. Each vector step is stored
    /// (`0x1003d240`, `0x10035b10`).
    #[allow(clippy::neg_cmp_op_on_partial_ord)] // TS's x87 tests: an unordered (NaN) value fails them
    pub fn push_along_line(&mut self, a: Vec3, b: Vec3, radius: f32, swirl: bool) {
        if !self.physics {
            return;
        }
        let ab = v::sub(b, a);
        let len = len_x87(ab) as f32;
        let dir = norm_x87(ab, [0.0, 0.0, 1.0]);
        let mid = v::scale(v::add(a, b), 0.5);
        let reach = (len_x87(v::sub(mid, b)) - 100.0) as f32;
        if !(reach > 0.0) {
            return;
        }
        for i in self.pushed() {
            let p = &mut self.parts[i];
            if !(len_x87(v::sub(mid, p.origin)) < reach as f64) {
                continue;
            }
            let da = len_x87(v::sub(p.origin, a)) as f32;
            let db = len_x87(v::sub(p.origin, b));
            let h = heron_height_x87(da, db, len);
            if !(h < radius as f64) {
                continue;
            }
            let k = (radius as f64 - h) as f32;
            let react = p.react;
            p.vel = v::add(p.vel, v::scale(v::scale(dir, k), react));
            if swirl {
                let side = cross_x87(dir, norm_x87(v::sub(p.origin, a), [0.0, 0.0, 1.0]));
                p.vel = v::add(p.vel, v::scale(v::scale(side, k), react));
            }
            Self::kick(p);
        }
    }

    /// `PSys_BlastPush(org, dir, radius, fwd)` (`0x10052c80`, the muzzle blast): particles on the
    /// push list within `fwd` of `org + dir̂·fwd` (`0x10052e26` compares with the forward offset,
    /// not `radius`) get `(radius − |dir|)/4` (the same for every particle, as in TS) away from
    /// `org`. A zero vector normalises to (0, 0, 1).
    pub fn blast_push(&mut self, org: Vec3, dir: Vec3, radius: f32, fwd: f32) {
        if !self.physics {
            return;
        }
        let x = dir.map(|c| c as f64);
        let dlen = (x[1] * x[1] + x[2] * x[2]) + x[0] * x[0];
        // TS also needs `|dir| <= radius` (0x10052e8d..0x10052ea5), with the stored length.
        let dlen_f = dlen.sqrt() as f32;
        if dlen_f > radius {
            return;
        }
        let n = if dlen == 0.0 { [0.0, 0.0, 1.0] } else { dir.map(|c| (c as f64 * (1.0 / dlen.sqrt())) as f32) };
        let c = v::ma(org, fwd, n);
        let k = radius - dlen_f;
        for i in self.pushed() {
            let p = &mut self.parts[i];
            let d = v::sub(c, p.origin).map(|c| c as f64);
            if ((d[2] * d[2] + d[1] * d[1]) + d[0] * d[0]).sqrt() <= fwd as f64 {
                let push = v::scale(norm_x87(v::sub(p.origin, org), [0.0, 0.0, 1.0]), k).map(|c| c / 4.0);
                p.vel = v::add(p.vel, push);
                Self::kick(p);
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
    /// With [`ParticleModel::Decelerate`] the pass is [`decel`]'s.
    pub fn update_draw(&mut self, dt: f32, cam: &Camera, players: &[(Vec3, Vec3)], collision: &mut dyn FnMut(&mut PSys, Collision)) {
        if self.parts.is_empty() {
            return;
        }
        if self.model == ParticleModel::Decelerate {
            self.update_draw_decel(dt, cam, collision);
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
            // 2. fade once the life is over, clamped to 0..1 on the x87 value (0x10052266..0x100522ca);
            // scale (one rounding each).
            if p.fadespeed != 0.0 && p.life <= 0.0 {
                let c = p.color as f64 - dt as f64 * p.fadespeed as f64 * 100.0;
                p.color = if c <= 0.0 {
                    0.0
                } else if c > 1.0 {
                    1.0
                } else {
                    c as f32
                };
            }
            if p.scalespeed != 0.0 {
                p.radius = (dt as f64 * p.scalespeed as f64 * 100.0 + p.radius as f64) as f32;
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
                p.frame = (dt as f64 * p.framerate as f64 + p.frame as f64) as f32;
            }
            p.life -= dt;
            i += 1;
        }
        self.high = last;
    }

    /// `Particle_Move` (`0x10050ce0`) for slot `i`, up to the quad (built when drawn). Nothing
    /// moves or thinks while resting (0x10050de1).
    #[allow(clippy::neg_cmp_op_on_partial_ord)] // TS's x87 tests: an unordered (NaN) value fails them
    fn particle_move(
        &mut self,
        i: usize,
        now: f32,
        dt: f32,
        cam: &Camera,
        players: &[(Vec3, Vec3)],
        collision: &mut dyn FnMut(&mut PSys, Collision),
    ) {
        // `f32(divisor × 0.033333335f)`, at least the frame time (0x100512ff).
        let think_iv = (self.divisor * 0.033_333_335).max(dt);
        let (physics, sv_gravity, cull) = (self.physics, self.gravity, self.cull_origin);
        // Zoomed in, the think and trace distances halve (`gHUD.m_iFOV < 40`, 0x10051276, 0x10051e0b).
        let zoom_scale = if (cam.fov as i32) < 40 { 0.5 } else { 1.0 };
        let p = &mut self.parts[i];
        if !p.resting {
            // `dt × 10f`, stored (0x10050def).
            let dt10 = dt * 10.0;
            // `vel.z += dt·gravity·0.1·−cl_gravity` on the x87, stored, then added (0x10050e0c).
            if p.gravity != 0.0 {
                let gz = (dt as f64 * p.gravity as f64 * 0.1 * -(sv_gravity as f64)) as f32;
                p.vel[2] += gz;
            }
            // Drag (0x10050e8b..0x10050f81): `k = min(drag·dt, 1)` and z's `1 − |g|` (0 when not
            // above 0) on the x87, each step `k·v` stored, then subtracted.
            let g = p.gravity.abs() as f64;
            let fz = if 1.0 - g > 0.0 { 1.0 - g } else { 0.0 };
            let k = p.drag as f64 * dt as f64;
            let k = if k >= 1.0 { 1.0 } else { k };
            let step = [(k * p.vel[0] as f64) as f32, (p.vel[1] as f64 * k) as f32, (fz * p.vel[2] as f64 * k) as f32];
            p.vel = v::sub(p.vel, step);
            // `Particle_Think` (0x10051270) runs before the move (0x10050f84).
            if now > p.next_think {
                let near = ((len_x87(v::sub(p.origin, cull)) * zoom_scale as f64) as f32) < 512.0;
                // Random drift (0x1005134a..0x10051563): steer an accumulator toward a fresh
                // random target by at most `randomdelta` (3D length), then add it to the velocity.
                if p.randomdelta != 0.0 && near {
                    let t = [0, 1, 2].map(|i| r::random_float(p.randvel_lo[i], p.randvel_hi[i]));
                    let mut step = v::sub(t, p.drift);
                    let x = step.map(|c| c as f64);
                    if ((x[0] * x[0] + x[2] * x[2]) + x[1] * x[1]).sqrt() > p.randomdelta as f64 {
                        let rd = p.randomdelta as f64;
                        step = norm_x87(step, [0.0, 0.0, 1.0]).map(|c| (c as f64 * rd) as f32);
                    }
                    p.drift = v::add(step, p.drift);
                    p.vel = v::ma(p.vel, think_iv, p.drift);
                }
                // React (0x10051573..0x10051a5f): players within 128 units move the
                // particle's *origin* along the direction to it, by how far it lies along
                // their horizontal motion (`dot` < 0 pulls it in behind them).
                if physics && p.react != 0.0 && near {
                    let ivr = think_iv as f64 * p.react as f64;
                    for (porg, pvel) in players {
                        let d = v::sub(p.origin, *porg);
                        let dist = len_x87(d);
                        if !(dist <= 128.0) {
                            continue;
                        }
                        let (vx, vy) = (pvel[0] as f64, pvel[1] as f64);
                        let speed = (vx * vx + vy * vy).sqrt();
                        if !(speed > 0.0) {
                            continue;
                        }
                        let vd = [(vx * (1.0 / speed)) as f32, (vy * (1.0 / speed)) as f32];
                        let dn = norm_x87(d, [0.0, 0.0, 1.0]);
                        let dot = (dn[0] as f64 * vd[0] as f64 + dn[1] as f64 * vd[1] as f64) as f32;
                        let k1 = (128.0 - dist as f32 as f64) * speed as f32 as f64 * dot as f64 * 0.0078125;
                        for (o, n) in p.origin.iter_mut().zip(dn).take(2) {
                            *o += ((n as f64 * k1) as f32 as f64 * ivr) as f32;
                        }
                    }
                }
                // Wind (0x10051a72..0x10051ce8): the segment steps here, inside a wind
                // particle's think; `n × ((speed × T) × wind)` stored, then added.
                if physics && p.wind != 0.0 {
                    self.wind.advance(now as f64);
                    if let Some((n, speed)) = self.wind.velocity(now as f64) {
                        let k = speed as f64 * think_iv as f64 * p.wind as f64;
                        p.vel = [0, 1, 2].map(|c| p.vel[c] + (n[c] as f64 * k) as f32);
                    }
                }
                p.next_think = now + think_iv;
            }
            p.origin = v::ma(p.origin, dt10, p.vel);
        }
        // `Particle_Collide` (0x10051d50) for TS type-3 particles that aren't resting.
        if !p.resting {
            if p.collide {
                self.collide(i, now, cam, zoom_scale, collision);
            } else {
                p.prev = p.origin;
            }
        }
        // The quad step spins moving rolling (z) and free (all) quads in degrees per 0.1 s
        // (0x10051076: `avel.z·dt·10.0 + angle` rounded once; 0x100510b3: x's and y's `dt·avel` and
        // `× 10f` stored, z's rounded once).
        let p = &mut self.parts[i];
        if !p.resting {
            match p.orient {
                ORIENT_ROLL => p.angles[2] = (p.avel[2] as f64 * dt as f64 * 10.0 + p.angles[2] as f64) as f32,
                ORIENT_FREE => {
                    for c in 0..2 {
                        p.angles[c] += dt * p.avel[c] * 10.0;
                    }
                    p.angles[2] = (dt as f64 * p.avel[2] as f64 * 10.0 + p.angles[2] as f64) as f32;
                }
                _ => {}
            }
        }
    }

    /// `Particle_Collide` (`0x10051d50`): the trace covers the whole path since the last trace
    /// (`prev` only moves on after one, 0x10052098), only inside the view cone (which also hides
    /// the particle), at `max(d·zoom/8192 (×2 beyond 1024), 0.016)` s. It is TS's
    /// `EV_PlayerTrace(prev, origin, PM_STUDIO_BOX, -1)` with the point hull, against whatever
    /// physents the engine has set up. The bounce damps by `0.3·max(1 − 7·iv, 0)` with the
    /// interval *before* the 0.016 floor (0x10051fa0), and reflects with the x87 dot.
    fn collide(&mut self, i: usize, now: f32, cam: &Camera, zoom_scale: f32, collision: &mut dyn FnMut(&mut PSys, Collision)) {
        let (cull, build) = (self.cull_origin, self.build);
        let ts = self.time_scale;
        let p = &mut self.parts[i];
        let moved = v::sub(p.prev, p.origin);
        if v::dot(moved, moved) < 0.001 || !p.visible {
            return;
        }
        p.visible = in_view_cone(build, cam, cull, p.origin);
        if !p.visible || p.next_trace > now {
            return;
        }
        // 0x10051e69..0x10051ebb: `D·(1/8192)f` stored, doubled beyond 1024; the 0.016 floor
        // only for the next trace time.
        let d = len_x87(v::sub(p.origin, cull)) * zoom_scale as f64;
        let mut iv = (d * (1.0f32 / 8192.0) as f64) as f32;
        if d > 1024.0 {
            iv += iv;
        }
        p.next_trace = if iv <= 0.016 { 0.016 } else { iv } + now;
        let tr = r::player_trace(p.prev, p.origin, 2, PM_STUDIO_BOX as i32, -1);
        if tr.fraction == 0.0 {
            // Stuck from the start: young particles die, older ones rest.
            if p.spawn as f64 + 0.5 <= now as f64 {
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
            let t = v::sub(p.origin, p.prev).map(|c| c as f64);
            let travelled = ((t[2] * t[2] + t[1] * t[1]) + t[0] * t[0]).sqrt() as f32;
            p.vel = bounce(p.vel, n, iv);
            // Rests once a trace interval moves it less than 3·drag (0x10052059).
            if p.drag as f64 * 3.0 > ts as f64 * travelled as f64 {
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

/// `Particle_Collide`'s bounce (`0x10051fa0..`): the velocity damped by `0.3f·max(1 − iv·7, 0)`
/// with the trace interval *before* its 0.016 floor (x's product rounded once, y's and z's
/// `× 0.3f` stored first), then reflected off `n` with the x87 dot `(ny·vy + nz·vz) + nx·vx`,
/// `s = f32(−2·dot)` and each `v + s·n` rounded once (`0x10058410`).
fn bounce(vel: Vec3, n: Vec3, iv: f32) -> Vec3 {
    let f = (1.0 - iv as f64 * 7.0).max(0.0);
    let vel = [(vel[0] as f64 * 0.3f32 as f64 * f) as f32, ((vel[1] * 0.3) as f64 * f) as f32, ((vel[2] * 0.3) as f64 * f) as f32];
    let (vv, nn) = (vel.map(|c| c as f64), n.map(|c| c as f64));
    let dn = (nn[1] * vv[1] + nn[2] * vv[2]) + nn[0] * vv[0];
    let s = (-2.0 * dn) as f32;
    [0, 1, 2].map(|c| (vv[c] + s as f64 * nn[c]) as f32)
}

/// `Particle_Rest` (`0x10051d10`): resting, no velocity, no roll and no spin.
fn rest(p: &mut Particle) {
    p.resting = true;
    p.vel = v::ZERO;
    p.avel[2] = 0.0;
    p.angles[2] = 0.0;
}

/// `InViewCone` (`0x100570b0`; 2.1 `0x100968f0`), measured from the cull origin
/// (`GetCullOrigin` `0x10057270`): points within 300 units pass; otherwise the point must be in
/// front and `(|d̂·right| + 0.5·|d̂·up|)·(1 + dist/8192·0.2) <= 0.9` (no FOV term).
///
/// The float forms are the build's. 3.0 keeps the length (`0x10058390`), the dots and the factor
/// on the x87 (the length is stored for the factor, `0x1018bce0`), normalises with the x87
/// reciprocal (`0x10029e40`) and compares the unrounded product with `0.9f` (`0x1005723d`). 2.1
/// stores the length's running sum (`0x10097fc0`), normalises with a float reciprocal of a float
/// length (`0x100981d0`), stores each dot's first two partial sums and the absolute dots, and
/// stores `|r| + |u| × 0.5` and the product before the compare.
#[allow(clippy::neg_cmp_op_on_partial_ord)] // 3.0's x87 tests: an unordered (NaN) dot is behind, a NaN product inside
pub fn in_view_cone(build: crate::gameplay::DllBuild, cam: &Camera, cull: Vec3, org: Vec3) -> bool {
    let d = v::sub(org, cull);
    match build {
        crate::gameplay::DllBuild::Optimised => {
            let x = d.map(|c| c as f64);
            let dist = ((x[0] * x[0] + x[1] * x[1]) + x[2] * x[2]).sqrt();
            if dist <= 300.0 {
                return true;
            }
            let n = norm_x87(d, d).map(|c| c as f64);
            let dot = |a: Vec3| (a[1] as f64 * n[1] + a[2] as f64 * n[2]) + a[0] as f64 * n[0];
            if !(dot(cam.forward) >= 0.0) {
                return false;
            }
            let k = dot(cam.up).abs() * 0.5 + dot(cam.right).abs();
            let k = (dist as f32 as f64 * (1.0f32 / 8192.0) as f64 * 0.2 + 1.0) * k;
            !(k > 0.9f32 as f64)
        }
        crate::gameplay::DllBuild::Debug => {
            let dist = len_stored(d);
            if dist <= 300.0 {
                return true;
            }
            let ss = d.iter().fold(0.0f64, |s, &c| s + c as f64 * c as f64) as f32;
            let l = (ss as f64).sqrt() as f32;
            let n = if l != 0.0 {
                let inv = 1.0f32 / l;
                d.map(|c| inv * c)
            } else {
                d
            };
            let dot = |a: Vec3| {
                let s = n[0] as f64 * a[0] as f64;
                let s = (s as f32 as f64 + n[1] as f64 * a[1] as f64) as f32;
                s as f64 + n[2] as f64 * a[2] as f64
            };
            if dot(cam.forward) < 0.0 {
                return false;
            }
            let (r, u) = (dot(cam.right).abs() as f32, dot(cam.up).abs() as f32);
            let k = (r as f64 + u as f64 * 0.5) as f32;
            let k = ((dist as f64 / 8192.0 * 0.2 + 1.0) * k as f64) as f32;
            k <= 0.9
        }
    }
}

/// 2.1's `Length` (`0x10097fc0`): each square added to a stored float sum, the root stored.
pub fn len_stored(a: Vec3) -> f32 {
    let s = a.iter().fold(0.0f32, |s, &x| (x as f64 * x as f64 + s as f64) as f32);
    (s as f64).sqrt() as f32
}

/// A vector's length as TS's optimised build takes it (`0x1003d5c0`, `0x10029e40`, `Length`
/// `0x10058390`): `(x² + y²) + z²` and the root on the x87, unrounded.
pub fn len_x87(a: Vec3) -> f64 {
    let x = a.map(|c| c as f64);
    ((x[0] * x[0] + x[1] * x[1]) + x[2] * x[2]).sqrt()
}

/// TS's optimised normalise (`0x10029e40`, `0x10051190`): each component times the x87
/// reciprocal of [`len_x87`], stored; a zero vector gives `zero` (left alone, or (0, 0, 1)).
pub fn norm_x87(a: Vec3, zero: Vec3) -> Vec3 {
    let l = len_x87(a);
    if l == 0.0 {
        return zero;
    }
    let inv = 1.0 / l;
    a.map(|c| (c as f64 * inv) as f32)
}

/// A cross product with each component rounded once (`0x10051220`).
fn cross_x87(a: Vec3, b: Vec3) -> Vec3 {
    let (a, b) = (a.map(|c| c as f64), b.map(|c| c as f64));
    [(a[1] * b[2] - a[2] * b[1]) as f32, (a[2] * b[0] - a[0] * b[2]) as f32, (a[0] * b[1] - a[1] * b[0]) as f32]
}

/// TS's point-to-line distance (`PSys_PushAlongLine` 0x10052720 and the whizz in
/// `EV_TS_FireBullets`): Heron's formula with `s − len` where `s − b` belongs, i.e.
/// `2·sqrt(s(s − a))·(s − len)/len` for distances `a`, `b` to the ends of a `len` long line.
pub fn heron_height(a: f32, b: f32, len: f32) -> f32 {
    let s = (a + b + len) * 0.5;
    2.0 * (s * (s - a)).max(0.0).sqrt() * (s - len) / len
}

/// [`heron_height`] as `PSys_PushAlongLine` works it out on the x87 (`0x10052aac..0x10052af6`):
/// `s = ((b + a) + len)·0.5` with `b` unrounded, then `2·sqrt(s·((s − a)(s − len)(s − len)))/len`
/// (a negative product gives NaN, which no particle passes).
fn heron_height_x87(a: f32, b: f64, len: f32) -> f64 {
    let (a, len) = (a as f64, len as f64);
    let s = ((b + a) + len) * 0.5;
    let t = (s - a) * (s - len) * (s - len);
    2.0 * (s * t).sqrt() / len
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
    quad(p, cam, if p.resting { ORIENT_BILLBOARD } else { p.orient }, true)
}

/// [`build_quad`] with the orientation given and, with `stretch` false, no stretching.
fn quad(p: &Particle, cam: &Camera, orient: u8, stretch: bool) -> [Vec3; 4] {
    if let Some(c) = p.corners {
        return c;
    }
    // `0x10051190`: `f32(cam − origin)` normalised with the x87 reciprocal; zero gives (0, 0, 1).
    let to_cam_n = || norm_x87(v::sub(cam.origin, p.origin), [0.0, 0.0, 1.0]);
    // Every `follow_view` but 2 and 3 (4 and up act as 0) can stretch (0x10051151).
    if stretch && !matches!(orient, ORIENT_FREE | ORIENT_AXIS) && p.stretch > 0.01 {
        // `|vel|·stretchspeed` in TS units, unrounded; the direction normalised in place
        // (`VectorNormalize` 0x10029e40); `half = f32(f32(dir × L) × 0.5f)`; the width the
        // once-rounded cross product times the radius.
        let len = len_x87(p.vel);
        let l = len * p.stretch as f64;
        if l > 2.0 * p.radius as f64 {
            let dir = norm_x87(p.vel, p.vel);
            let half = dir.map(|c| (c as f64 * l) as f32 * 0.5);
            let w = v::scale(cross_x87(to_cam_n(), dir), p.radius);
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
        ORIENT_AXIS => (cross_x87(to_cam_n(), p.followaxis), p.followaxis),
        _ => (cam.right, cam.up),
    };
    let rr = v::scale(right, p.radius);
    let uu = v::scale(up, p.radius);
    [v::add(v::sub(p.origin, rr), uu), v::add(v::add(p.origin, rr), uu), v::sub(v::add(p.origin, rr), uu), v::sub(v::sub(p.origin, rr), uu)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gameplay::DllBuild;

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
        let (w, _) = a.velocity(15.0).expect("a positive gust");
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
        assert!(in_view_cone(DllBuild::Optimised, &cam, cam.origin, [1000.0, 0.0, 0.0]));
        assert!(!in_view_cone(DllBuild::Optimised, &cam, cam.origin, [-1000.0, 0.0, 0.0]));
        assert!(in_view_cone(DllBuild::Optimised, &cam, cam.origin, [-300.0, 0.0, 0.0]), "300 units still pass");
    }

    /// `Quad_Build`'s stretch (`0x100500af..0x10050729`): head first (v = 0), width across u,
    /// `radius·(toCam̂ × vel̂)`.
    #[test]
    fn quad_stretch() {
        let cam = Camera { forward: [1.0, 0.0, 0.0], right: [0.0, -1.0, 0.0], up: [0.0, 0.0, 1.0], fov: 90.0, ..Default::default() };
        let p = Particle { origin: [100.0, 0.0, 0.0], vel: [0.0, 0.0, 100.0], stretch: 1.0, radius: 2.0, ..Default::default() };
        assert_eq!(build_quad(&p, &cam), [[100.0, -2.0, 50.0], [100.0, 2.0, 50.0], [100.0, 2.0, -50.0], [100.0, -2.0, -50.0]]);
        // Too slow to stretch (L = 100·0.03 = 3 < 2r): a plain billboard.
        let p = Particle { stretch: 0.03, ..p };
        assert_eq!(build_quad(&p, &cam)[0], [100.0, 2.0, 2.0]);
        // Resting: a billboard whatever `follow_view` says.
        let p = Particle { orient: ORIENT_AXIS, resting: true, ..p };
        assert_eq!(build_quad(&p, &cam)[0], [100.0, 2.0, 2.0]);
    }

    /// `InViewCone` near the 0.9 threshold: 3.0 (0x100570b0) keeps the dots and the product on
    /// the x87, 2.1 (0x100968f0) stores its partial sums; one ulp of y apart, the two builds
    /// answer differently (as the old all-f32 form did from both).
    #[test]
    fn view_cone_forms() {
        let cam = Camera { forward: [1.0, 0.0, 0.0], right: [0.0, -1.0, 0.0], up: [0.0, 0.0, 1.0], fov: 90.0, ..Default::default() };
        let at = |y: u32| [f32::from_bits(0x4592_7feb), f32::from_bits(y), f32::from_bits(0xc282_25df)];
        let both = |y| (in_view_cone(DllBuild::Optimised, &cam, v::ZERO, at(y)), in_view_cone(DllBuild::Debug, &cam, v::ZERO, at(y)));
        assert_eq!(both(0x45ab_a629), (true, false));
        assert_eq!(both(0x45ab_a62a), (false, true));
        assert_eq!(both(0x45ab_a62b), (false, false));
        assert_eq!(len_stored([3.0, 4.0, 12.0]), 13.0);
    }

    /// `Particle_Move` (0x10050ce0) in TS units: gravity `f32(dt·g·0.1·−cl_gravity)` added
    /// (0x10050e0c), the drag steps `f32(min(drag·dt, 1)·v)` subtracted (z's times `1 − |g|`,
    /// 0x10050ef6..0x10050f81), then `origin += f32(f32(dt × 10f)·v)` (0x10050def, 0x10050fa0).
    /// The roll spins `avel.z·dt·10.0 + angle` rounded once (0x10051076).
    #[test]
    fn move_in_ts_units() {
        let mut ps = PSys { parts: vec![Particle::default()], ..Default::default() };
        let p0 = Particle {
            origin: [1.0, 2.0, 3.0],
            vel: [33.3, -7.1, 12.9],
            gravity: 0.25,
            drag: 1.7,
            orient: ORIENT_ROLL,
            avel: [0.0, 0.0, 77.7],
            angles: [0.0, 0.0, 10.1],
            next_think: f32::MAX,
            ..Default::default()
        };
        ps.parts[0] = p0.clone();
        let dt = 0.013f32;
        ps.particle_move(0, 0.0, dt, &Camera { fov: 90.0, ..Default::default() }, &[], &mut |_, _| {});
        let (d, g) = (dt as f64, p0.gravity as f64);
        let mut vel = p0.vel;
        vel[2] += (d * g * 0.1 * -800.0) as f32;
        let k = p0.drag as f64 * d;
        let step = [(k * vel[0] as f64) as f32, (vel[1] as f64 * k) as f32, ((1.0 - g) * vel[2] as f64 * k) as f32];
        let vel = v::sub(vel, step);
        let dt10 = dt * 10.0;
        let origin = [0, 1, 2].map(|c| p0.origin[c] + dt10 * vel[c]);
        let p = &ps.parts[0];
        assert_eq!((p.vel, p.origin), (vel, origin));
        assert_eq!(p.angles[2], (77.7f32 as f64 * d * 10.0 + 10.1f32 as f64) as f32);
        // No gravity or drag: 1 + f32(0.13 × 33.3) is 5.3289995 in TS's units, where the
        // per-second form (`1 + 0.013 × 333`) gives 5.329.
        ps.parts[0] = Particle { origin: [1.0, 0.0, 0.0], vel: [33.3, 0.0, 0.0], drag: 0.0, next_think: f32::MAX, ..Default::default() };
        ps.particle_move(0, 0.0, dt, &Camera { fov: 90.0, ..Default::default() }, &[], &mut |_, _| {});
        assert_eq!(ps.parts[0].origin[0].to_bits(), 5.328_999_5f32.to_bits());
        assert_ne!(ps.parts[0].origin[0], 1.0 + dt * (33.3f32 * 10.0));
    }

    /// `PSys_BlastPush` (0x10052c80): the sphere around `org + dir̂·fwd` has the forward offset as
    /// its radius (0x10052e26), and the push is `f32(n̂ × (radius − |dir|)) / 4f` added
    /// (0x10052f0c..0x10052f4b), then a resting particle hops `|vz| + 2`.
    #[test]
    fn blast_push_sphere_is_the_forward_offset() {
        let mut ps = PSys::default();
        let p = Particle { origin: [80.0, 0.0, 0.0], react: 1.0, resting: true, ..Default::default() };
        ps.parts = vec![p];
        ps.react_list = vec![0];
        ps.blast_push(v::ZERO, [1.0, 0.0, 0.0], 10.0, 50.0);
        let p = &ps.parts[0];
        assert_eq!(p.vel, [2.25, 0.0, 2.0], "30 units from the centre is inside the 50-unit sphere");
        assert!(!p.resting);
        // Beyond `fwd`, nothing.
        ps.parts[0] = Particle { origin: [101.0, 0.0, 0.0], react: 1.0, ..Default::default() };
        ps.blast_push(v::ZERO, [1.0, 0.0, 0.0], 10.0, 50.0);
        assert_eq!(ps.parts[0].vel, v::ZERO);
    }

    /// `PSys_PushAlongLine`'s distance (0x10052aac..0x10052af6): TS's Heron with `s − len` where
    /// `s − b` belongs: 1.5 for a point 3 units off the middle of an 8-unit line (true height 3).
    #[test]
    fn push_line_height() {
        assert_eq!(heron_height_x87(5.0, 5.0, 8.0), 1.5);
        assert_eq!(heron_height(5.0, 5.0, 8.0), 1.5);
    }

    /// `Particle_Collide`'s bounce (0x10051fa0): the damping takes the interval before its 0.016
    /// floor (0.3 × (1 − 7 × 0.004) here, not 0.3 × (1 − 7 × 0.016)), and the reflection keeps the
    /// dot on the x87.
    #[test]
    fn bounce_uses_the_raw_interval() {
        let v2 = bounce([10.0, 0.0, -20.0], [0.0, 0.0, 1.0], 0.004);
        let f = 1.0 - 0.004f32 as f64 * 7.0;
        assert_eq!(v2, [(10.0 * 0.3f32 as f64 * f) as f32, 0.0, -((-20.0f32 * 0.3) as f64 * f) as f32]);
        assert!(v2[0] > 2.9, "{v2:?}");
    }
}
