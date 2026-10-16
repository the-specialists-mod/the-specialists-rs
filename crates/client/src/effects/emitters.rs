//! Map emitters (`CTS_CL_Emitter`, rendering doc §5.1-5.3). The server removes `ts_emitter`,
//! `ts_particle` and `ts_fog` at spawn; the client re-reads the map's BSP entity lump and builds
//! them locally. Parsing is pure (unit-tested); emission talks to the particle system.

use super::psys::{self, Camera, PSys, Particle};
use goldsrc::client::engine;
use goldsrc::ext::render::{self as r, Vec3, vec3 as v};

pub use shared::bsp::{entity_lump, parse_entities};

/// MSVC `atof` (`0x1006b918`) stored as a float: the leading number after any white space
/// (sign, digits, a point, an exponent), 0 when there is none; the rest is ignored.
fn num(s: &str) -> f32 {
    scan_float(s.as_bytes()).map_or(0.0, |(v, _)| v as f32)
}

/// MSVC `atoi` (`0x1006c042`).
fn int(s: &str) -> i32 {
    crate::hud::statusbar::atoi(s.as_bytes())
}

/// The C float at the start of `b` after white space, and the bytes it took.
fn scan_float(b: &[u8]) -> Option<(f64, usize)> {
    let mut i = 0;
    while b.get(i).is_some_and(|&c| c == b' ' || (9..=13).contains(&c)) {
        i += 1;
    }
    let start = i;
    if matches!(b.get(i), Some(b'+' | b'-')) {
        i += 1;
    }
    let digits = |i: &mut usize| {
        let s = *i;
        while b.get(*i).is_some_and(u8::is_ascii_digit) {
            *i += 1;
        }
        *i - s
    };
    let mut n = digits(&mut i);
    if b.get(i) == Some(&b'.') {
        i += 1;
        n += digits(&mut i);
    }
    if n == 0 {
        return None;
    }
    let mantissa = i;
    if matches!(b.get(i), Some(b'e' | b'E')) {
        let mut j = i + 1;
        if matches!(b.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
        if digits(&mut j) > 0 {
            i = j;
        }
    }
    let text = std::str::from_utf8(&b[start..i]).ok()?;
    let v = text.parse::<f64>().or_else(|_| std::str::from_utf8(&b[start..mantissa]).unwrap_or("0").parse()).unwrap_or(0.0);
    Some((v, i))
}

/// `sscanf(value, "%f %f %f", &v.x, &v.y, &v.z)` into `v`: each component read is stored, the
/// rest keep their value. Returns how many were read.
fn scan_vec3(s: &str, v: &mut Vec3) -> usize {
    let b = s.as_bytes();
    let mut at = 0;
    for (k, o) in v.iter_mut().enumerate() {
        let Some((x, used)) = scan_float(&b[at..]) else { return k };
        *o = x as f32;
        at += used;
    }
    3
}

/// TS's key: trailing spaces cut (`0x1003a290`), compared with `stricmp`.
fn key(k: &str) -> String {
    k.trim_end_matches(' ').to_ascii_lowercase()
}

/// A `ts_particle` template. Each (high, low) pair is rolled per particle.
#[derive(Clone, Debug, Default)]
pub struct Template {
    pub targetname: String,
    pub target: String,
    pub target_index: Option<u16>,
    pub sprite_name: String,
    pub sprite_trans: i32,
    pub follow_view: u8,
    pub collide: bool,
    pub oncollide: u8,
    pub spawnflags: i32,
    pub soundonemit: String,
    pub life: (f32, f32),
    pub gravity: (f32, f32),
    pub color: (f32, f32),
    pub fadespeed: (f32, f32),
    pub radius: (f32, f32),
    pub scalespeed: (f32, f32),
    pub frame: (i32, i32),
    pub framerate: (f32, f32),
    pub randomdelta: (f32, f32),
    pub react: (f32, f32),
    pub wind: (f32, f32),
    pub fadein: (f32, f32),
    pub velocity: (Vec3, Vec3),
    pub avelocity: (Vec3, Vec3),
    pub randomvel: (Vec3, Vec3),
    pub followaxis: Vec3,
    pub stretch: f32,
}

impl Template {
    /// `ParticleKeyValue` (`0x10055f90`). The values are kept as the map gives them: the
    /// server's `CTSParticle::CheckValues` clamps have no client copy, so a colour above 1 or a
    /// negative life reaches the particles, as in TS.
    pub fn from_kv(kv: &[(String, String)]) -> Template {
        Self::parse(kv, &mut Vec::new())
    }

    /// [`Self::from_kv`], with the console lines TS prints for a vector key that doesn't hold
    /// three numbers (`"cl: velocitylow has bad value. Should be \"x y z\"\n"`; the numbers
    /// read are kept). Integer keys are `atoi`, the rest `atof`.
    pub fn parse(kv: &[(String, String)], messages: &mut Vec<String>) -> Template {
        // No defaults (TS zeroes the struct): a `follow_view 3` template without `followaxis`
        // builds a zero-height quad, as in TS.
        let mut t = Template::default();
        for (k, val) in kv {
            let f = num(val);
            let i = int(val);
            let k = key(k);
            let mut vector = |name: &str, v: &mut Vec3| {
                if scan_vec3(val, v) != 3 {
                    messages.push(format!("cl: {name} has bad value. Should be \"x y z\"\n"));
                }
            };
            match k.as_str() {
                "targetname" => t.targetname = val.clone(),
                "target" => t.target = val.clone(),
                "sprite_name" => t.sprite_name = val.clone(),
                "sprite_trans" => t.sprite_trans = i,
                // The particle keeps the low byte (`+0x05`); `Quad_Build` treats 4+ as 0.
                "follow_view" => t.follow_view = i as u8,
                "collide" => t.collide = i != 0,
                // Only 1-3 pick a callback (`0x1003af76..0x1003afa9`); anything else has none.
                "oncollide" => t.oncollide = if (1..=3).contains(&i) { i as u8 } else { 0 },
                "spawnflags" => t.spawnflags = i,
                "soundonemit" => t.soundonemit = val.clone(),
                "lifehigh" => t.life.0 = f,
                "lifelow" => t.life.1 = f,
                "gravityhigh" => t.gravity.0 = f,
                "gravitylow" => t.gravity.1 = f,
                "colorhigh" => t.color.0 = f,
                "colorlow" => t.color.1 = f,
                "fadespeedhigh" => t.fadespeed.0 = f,
                "fadespeedlow" => t.fadespeed.1 = f,
                "p_radiushigh" => t.radius.0 = f,
                "p_radiuslow" => t.radius.1 = f,
                "scalespeedhigh" => t.scalespeed.0 = f,
                "scalespeedlow" => t.scalespeed.1 = f,
                "current_framehigh" => t.frame.0 = i,
                "current_framelow" => t.frame.1 = i,
                "frame_ratehigh" => t.framerate.0 = f,
                "frame_ratelow" => t.framerate.1 = f,
                "randomdeltahigh" => t.randomdelta.0 = f,
                "randomdeltalow" => t.randomdelta.1 = f,
                "reactamounthigh" => t.react.0 = f,
                "reactamountlow" => t.react.1 = f,
                "windamounthigh" => t.wind.0 = f,
                "windamountlow" => t.wind.1 = f,
                "fadeintimehigh" => t.fadein.0 = f,
                "fadeintimelow" => t.fadein.1 = f,
                "velocityhigh" => vector("velocityhigh", &mut t.velocity.0),
                "velocitylow" => vector("velocitylow", &mut t.velocity.1),
                "a_velocityhigh" => vector("a_velocityhigh", &mut t.avelocity.0),
                "a_velocitylow" => vector("a_velocitylow", &mut t.avelocity.1),
                "randomvelhigh" => vector("randomvelhigh", &mut t.randomvel.0),
                "randomvellow" => vector("randomvellow", &mut t.randomvel.1),
                "followaxis" => vector("followaxis", &mut t.followaxis),
                "stretchspeed" => t.stretch = f,
                _ => {}
            }
        }
        t
    }

    /// `Particle_RandomizeFromTemplate` (`0x1003a9b0`) plus the fields the spawners copy from the
    /// template (`0x1003ae7d..`, `0x10039d50`). Each value is `RandomFloat(low, high)` (the frame
    /// `RandomLong`) with the template's raw bounds, in TS's order: life, gravity, colour, fade
    /// speed, radius, scale speed, frame, frame rate, random delta, react, wind, fade-in,
    /// velocity, angular velocity. Velocities: TS units per 0.1 s -> u/s.
    pub fn roll(&self, rf: &mut impl FnMut(f32, f32) -> f32, rl: &mut impl FnMut(i32, i32) -> i32) -> Particle {
        let mut pick = |p: (f32, f32)| rf(p.1, p.0);
        let life = pick(self.life);
        let gravity = pick(self.gravity);
        let color = pick(self.color);
        let fadespeed = pick(self.fadespeed);
        let radius = pick(self.radius);
        let scalespeed = pick(self.scalespeed);
        let frame = rl(self.frame.1, self.frame.0) as f32;
        let mut pick = |p: (f32, f32)| rf(p.1, p.0);
        let framerate = pick(self.framerate);
        let randomdelta = pick(self.randomdelta);
        let react = pick(self.react);
        let wind = pick(self.wind);
        let fadein = pick(self.fadein);
        let velocity = [0, 1, 2].map(|i| rf(self.velocity.1[i], self.velocity.0[i]));
        let avel = [0, 1, 2].map(|i| rf(self.avelocity.1[i], self.avelocity.0[i]));
        Particle {
            kind: psys::KIND_MAP,
            vel: v::scale(velocity, 10.0),
            avel: v::scale(avel, 10.0),
            randvel_lo: v::scale(self.randomvel.1, 10.0),
            randvel_hi: v::scale(self.randomvel.0, 10.0),
            randomdelta: randomdelta * 10.0,
            life,
            gravity,
            color,
            fadespeed,
            radius,
            scalespeed,
            frame,
            framerate,
            react,
            wind,
            fadein,
            orient: self.follow_view,
            followaxis: self.followaxis,
            stretch: self.stretch,
            collide: self.collide,
            oncollide: self.oncollide,
            target: self.target_index,
            drag: 0.1,
            ..Default::default()
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Emitter {
    pub origin: Vec3,
    pub particle: String,
    pub template: Option<usize>,
    pub eps: (f32, f32),
    pub ppe: (i32, i32),
    pub variance: (Vec3, Vec3),
    pub cameradensity: f32,
    pub cameraradius: f32,
    pub spawnflags: i32,
    pub next_emit: f64,
    pub dead: bool,
}

impl Emitter {
    pub fn from_kv(kv: &[(String, String)]) -> Emitter {
        let mut e = Emitter::default();
        // `EmitterKeyValue` (`0x10055d20`): `atoi` counts and flags, `atof` scalars, `sscanf`
        // vectors (no message).
        for (k, val) in kv {
            let f = num(val);
            match key(k).as_str() {
                "origin" => _ = scan_vec3(val, &mut e.origin),
                "particle" => e.particle = val.clone(),
                "epshigh" => e.eps.0 = f,
                "epslow" => e.eps.1 = f,
                "ppehigh" => e.ppe.0 = int(val),
                "ppelow" => e.ppe.1 = int(val),
                "variancehigh" => _ = scan_vec3(val, &mut e.variance.0),
                "variancelow" => _ = scan_vec3(val, &mut e.variance.1),
                "cameradensity" => e.cameradensity = f,
                "cameraradius" => e.cameraradius = f,
                "spawnflags" => e.spawnflags = int(val),
                _ => {}
            }
        }
        e
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Fog {
    pub near: f32,
    pub far: f32,
    pub color: Vec3,
}

/// Everything the client builds from one map (`ProcessEnts` `0x1003a5d0`, three passes).
#[derive(Default)]
pub struct MapFx {
    pub templates: Vec<Template>,
    pub emitters: Vec<Emitter>,
    pub fog: Option<Fog>,
    /// The console lines the parse printed, in order.
    pub messages: Vec<String>,
}

/// The entity's `classname` as each pass reads it: the last such key (`stricmp`, trailing
/// spaces cut), compared with `stricmp`.
fn classname(kv: &[(String, String)]) -> &str {
    kv.iter().rev().find(|(k, _)| key(k) == "classname").map_or("", |(_, v)| v.as_str())
}

impl MapFx {
    pub fn from_lump(text: &str) -> MapFx {
        let ents = parse_entities(text);
        let mut fx = MapFx::default();
        // pass 1: templates. Every entity is parsed into the next free slot (so its bad vector
        // keys print) and kept if it is a `ts_particle`, until the 512 slots are taken.
        for e in &ents {
            if fx.templates.len() >= 512 {
                break;
            }
            let t = Template::parse(e, &mut fx.messages);
            if classname(e).eq_ignore_ascii_case("ts_particle") {
                fx.templates.push(t);
            }
        }
        // `0x1003a820`: the first template whose targetname matches, case-insensitively; an
        // empty name matches an unnamed template.
        let names: Vec<String> = fx.templates.iter().map(|t| t.targetname.clone()).collect();
        let lookup = |name: &str| names.iter().position(|n| n.eq_ignore_ascii_case(name));
        for t in &mut fx.templates {
            t.target_index = lookup(&t.target).map(|i| i as u16);
        }
        // pass 2: emitters, linked to the template whose targetname == particle
        for e in ents.iter().filter(|e| classname(e).eq_ignore_ascii_case("ts_emitter")).take(512) {
            let mut em = Emitter::from_kv(e);
            em.template = lookup(&em.particle);
            fx.emitters.push(em);
        }
        // pass 3: fog (`0x1003a3c0` on every entity: a bad `color` prints whatever the class).
        // Every `ts_fog` overwrites the last (`0x1003a7f0`), so the last one counts.
        for e in &ents {
            let mut f = Fog::default();
            for (k, val) in e.iter() {
                match key(k).as_str() {
                    "near" => f.near = num(val),
                    "far" => f.far = num(val),
                    "color" => {
                        if scan_vec3(val, &mut f.color) != 3 {
                            fx.messages.push("color has bad value. Should be \"r g b\"\n".into());
                        }
                    }
                    _ => {}
                }
            }
            if classname(e).eq_ignore_ascii_case("ts_fog") {
                fx.fog = Some(f);
            }
        }
        fx
    }

    /// Emission step (`CTS_CL_Emitter::Frame` `0x1003ab40`, doc §5.3). Emitters are scheduled on
    /// `clock`, the emitter clock (`0x10189478`); `now` (client time) stamps the particles.
    ///
    /// TS walks all 512 emitter slots (`0x1003ac13..0x1003b242`). An unused or removed slot is
    /// zeroed (`0x1003b1c5`), so once the clock is past 0 its `next_emit` of 0 is due and it rolls
    /// `RandomLong(0, 0)` like a live emitter with no particles.
    pub fn emit(&mut self, ps: &mut PSys, cam: &Camera, clock: f64, now: f64, ts: f32) {
        // `0x1018ca60` is an int (1, 2, 4, 6 or 8).
        let div = (ps.divisor as i32).max(1);
        for slot in 0..MAX_EMITTERS {
            let Some(em) = self.emitters.get_mut(slot).filter(|e| !e.dead) else {
                if 0.0 < clock {
                    r::random_long(0, 0);
                }
                continue;
            };
            // `fld DWORD next_emit; fcomp QWORD clock` (0x1003ac13).
            if em.next_emit >= clock {
                continue;
            }
            // The raw keyvalues (0x1003ac54): with ppelow > ppehigh the engine returns ppelow.
            // TS rolls it before it looks at the template: an emitter whose `particle` names no
            // template points before the table (index −1, `0x1003a820`), which TS never checks.
            let n = r::random_long(em.ppe.1, em.ppe.0);
            // Guarded: past the roll the template would be read from before TS's table; the
            // port skips the emitter there.
            let Some(ti) = em.template else { continue };
            // Only 0 is rejected (0x1003ac6d). A negative count goes on: it plays the emit sound
            // and reschedules.
            if n == 0 {
                continue;
            }
            let (low, high) = (em.variance.1, em.variance.0);
            let inside = inside_xy(em.origin, low, high, cam.origin);
            let mut f = 1.0f32;
            if !inside {
                // Behind the camera: skipped this frame and not rescheduled (0x1003ad96 jumps
                // past the `next_emit` writes), so it is tested again next frame.
                let Some(lod) = lod_factor(em.origin, low, high, cam) else { continue };
                f = lod;
            }
            let n = ceil_div(n, div);
            // `(float)i < (float)(n / f)` (0x1003afb3, 0x1003b139). Capped at the largest pool:
            // nothing dies during the loop, so particles past the pool size are dropped anyway,
            // and a map's `ppehigh` of 2^31 would otherwise freeze the game rolling them.
            // (Not `f32::min`, which would turn a NaN limit, no particles, into the cap.)
            let limit = (n as f64 / f as f64) as f32;
            let limit = if limit > MAX_PER_EMIT { MAX_PER_EMIT } else { limit };
            let tmpl = &self.templates[ti];
            // The template's sprite is looked up only now, after the random count and the camera
            // test (0x1003ae5f). A missing one is reported and the emitter is tried again next
            // frame: 0x1003ae69..0x1003ae78 skip the `next_emit` writes.
            let Some(spr) = ps.sprite(&tmpl.sprite_name, tmpl.sprite_trans) else {
                engine().print(&format!("Sprite '{}' is not a valid sprite.\n", tmpl.sprite_name));
                continue;
            };
            // The emit sound plays at the scratch particle's origin (`0x1018a720+0x14`), which
            // `0x1004ffe0` zeroes and the loop sets: the last particle's, or the world origin when
            // a negative count added none.
            let mut last = v::ZERO;
            let mut i = 0;
            while (i as f32) < limit {
                i += 1;
                let mut p = tmpl.roll(&mut r::random_float, &mut r::random_long);
                p.sprite = spr;
                // XY around the camera (0x1003b024..0x1003b0cb), led by the local player's
                // velocity, or `origin + RandomFloat(low, high)` (0x1003b0cd..0x1003b101); z always
                // from the box (0x1003b104).
                let (x, y) = if em.cameradensity > 0.01 && em.cameraradius > 0.01 && inside {
                    let rad = camera_radius(em.cameraradius, em.cameradensity);
                    let (dx, dy) = (r::random_float(-rad, rad), r::random_float(-rad, rad));
                    camera_xy(cam.origin, dx, dy, engine().local_player().map(|e| e.curstate.velocity))
                } else {
                    (em.origin[0] + r::random_float(low[0], high[0]), em.origin[1] + r::random_float(low[1], high[1]))
                };
                let pos = [x, y, em.origin[2] + r::random_float(low[2], high[2])];
                p.origin = pos;
                last = pos;
                ps.add(p, now);
            }
            if !tmpl.soundonemit.is_empty() {
                let pitch = crate::events::slow_pitch();
                // Volume and attenuation both 1.0 (`0x1003b18e`, `0x1003b193`).
                engine().ev_play_sound(0, last, goldsrc::sys::CHAN_STATIC as i32, &tmpl.soundonemit, 1.0, 1.0, 0, pitch);
            }
            if em.spawnflags & 1 != 0 {
                em.dead = true;
                continue;
            }
            let eps = r::random_float(em.eps.1, em.eps.0);
            em.next_emit = next_emit(clock, eps, ts, div, f);
        }
    }

    /// The `oncollide` 2/3 callbacks' spawn (`0x10039d50`/`0x10039f60`, after 2 has killed the
    /// particle that hit): one particle of template `t` at `pos`. A sprite that won't load is
    /// reported. With the template's spawn flag 1 the velocity is rolled again from the
    /// template's bounds; otherwise it is the colliding particle's (`parent_vel`).
    pub fn spawn_one(&self, ps: &mut PSys, t: u16, pos: Vec3, parent_vel: Vec3, now: f64) {
        let Some(tmpl) = self.templates.get(t as usize) else { return };
        let Some(spr) = ps.sprite(&tmpl.sprite_name, tmpl.sprite_trans) else {
            engine().print(&format!("Sprite '{}' is not a valid sprite.\n", tmpl.sprite_name));
            return;
        };
        let mut p = tmpl.roll(&mut r::random_float, &mut r::random_long);
        p.sprite = spr;
        p.origin = pos;
        p.vel = if tmpl.spawnflags & 1 == 0 {
            parent_vel
        } else {
            v::scale([0, 1, 2].map(|i| r::random_float(tmpl.velocity.1[i], tmpl.velocity.0[i])), 10.0)
        };
        ps.add(p, now);
    }
}

/// The next emission time (`0x1003b1d4..0x1003b22c`), stored as a float: with `eps` > 0,
/// `clock + (1 / max(ts, 0.01f)) × (float)(1 / eps) × div × f` (`0x10056fa0` scales by the
/// slow motion), else `clock + div × f`, on the x87.
fn next_emit(clock: f64, eps: f32, ts: f32, div: i32, f: f32) -> f64 {
    let step = if eps > 0.0 {
        let x = (1.0 / eps as f64) as f32;
        let m = if ts < 0.01 { 0.01f32 } else { ts };
        1.0 / m as f64 * x as f64 * div as f64 * f as f64
    } else {
        div as f64 * f as f64
    };
    (step + clock) as f32 as f64
}

/// The camera spawn radius (`0x1003b024..0x1003b05b`): `2r − (2r − r) × min(density, 1)` on the
/// x87, stored as a float.
fn camera_radius(r: f32, density: f32) -> f32 {
    let m = if density > 1.0 { 1.0 } else { density as f64 };
    let r2 = r as f64 + r as f64;
    (r2 - (r2 - r as f64) * m) as f32
}

/// A camera-centred particle's XY: the camera plus the random offsets (stored), then, when there
/// is a local player (`GetLocalPlayer` at 0x1003b0a0), 1.5 (`0x1007a860`) times its
/// `curstate.velocity` x/y (`+0x320`/`+0x324`) added on the x87, so the particles land where a
/// moving player is heading.
fn camera_xy(cam: Vec3, dx: f32, dy: f32, local_velocity: Option<Vec3>) -> (f32, f32) {
    let (x, y) = (dx + cam[0], dy + cam[1]);
    match local_velocity {
        Some(vel) => ((vel[0] as f64 * 1.5 + x as f64) as f32, (vel[1] as f64 * 1.5 + y as f64) as f32),
        None => (x, y),
    }
}

/// The camera is strictly inside the emitter's XY box, `origin + low` .. `origin + high` summed on
/// the x87 (0x1003ac75..0x1003acce): a box whose low is above its high has no inside.
fn inside_xy(origin: Vec3, low: Vec3, high: Vec3, cam: Vec3) -> bool {
    let at = |o: f32, d: f32| o as f64 + d as f64;
    at(origin[0], low[0]) < cam[0] as f64
        && at(origin[1], low[1]) < cam[1] as f64
        && at(origin[0], high[0]) > cam[0] as f64
        && at(origin[1], high[1]) > cam[1] as f64
}

/// Level-of-detail factor of an emitter the camera is outside of, or `None` when its box centre
/// is behind the camera. Horizontal only (0x1003ace9..0x1003ad96): the centre x is
/// `(low + high) × 0.5 + origin` and the offsets from the camera are stored as floats (the centre
/// y too); the XY distance stays on the x87, floored at `1e-4f`, and the forward dot (z × 0) is
/// tested against the double 0.1. The size (`high − low`, 0x1003ad9c..0x1003ae25) is 3D, its z
/// stored, and not floored, so a zero-size box gives an infinite ratio and f = 10. The ratio is
/// clamped to 1..10 and stored.
fn lod_factor(origin: Vec3, low: Vec3, high: Vec3, cam: &Camera) -> Option<f32> {
    let cx = (low[0] as f64 + high[0] as f64) * 0.5 + origin[0] as f64;
    let cy = ((low[1] as f64 + high[1] as f64) * 0.5 + origin[1] as f64) as f32;
    let dx = (cx - cam.origin[0] as f64) as f32;
    let dy_full = cy as f64 - cam.origin[1] as f64;
    let dy = dy_full as f32;
    let mut d = (dy_full * dy as f64 + dx as f64 * dx as f64).sqrt();
    if 1e-4f32 as f64 > d {
        d = 1e-4f32 as f64;
    }
    let (nx, ny, nz) = (dx as f64 / d, dy as f64 / d, 0.0 / d);
    let f = cam.forward;
    if f[1] as f64 * ny + f[2] as f64 * nz + f[0] as f64 * nx < 0.1 {
        return None;
    }
    let (sx, sy) = (high[0] as f64 - low[0] as f64, high[1] as f64 - low[1] as f64);
    let sz = (high[2] as f64 - low[2] as f64) as f32 as f64;
    let ratio = d / (sy * sy + sz * sz + sx * sx).sqrt();
    Some(if 1.0 > ratio {
        1.0
    } else if 10.0 < ratio {
        10.0
    } else {
        ratio as f32
    })
}

/// `CTS_CL_Emitter`'s emitter slots (`0x101663b8`, 0x118 bytes each).
const MAX_EMITTERS: usize = 512;

/// Most particles one emission rolls: `PSys_Init`'s largest pool (port guard, see [`MapFx::emit`]).
const MAX_PER_EMIT: f32 = 4096.0;

/// Particle count over the `cl_fx` divisor, rounded up as `idiv` plus `inc` on a non-zero
/// remainder (0x1003ae29..0x1003ae40): with a divisor above 1 a small negative count becomes 1
/// (-1 / 2 = 0 remainder -1, then + 1).
fn ceil_div(n: i32, div: i32) -> i32 {
    n / div + i32::from(n % div != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LUMP: &str = r#"{
"classname" "worldspawn"
"wad" "\half-life\valve\halflife.wad"
}
{
"origin" "414 680 88"
"color" ".8 .8 .8"
"far" "9000"
"near" "2048"
"classname" "ts_fog"
}
// comment
{
"origin" "426 725 96"
"randomdeltahigh" "0 0 0"
"lifehigh" "30"
"lifelow" "30"
"colorhigh" "1.5"
"colorlow" ".4"
"velocityhigh" "2 2 -0.05"
"velocitylow" "-2 -2 -0.05"
"sprite_name" "sprites/snowy.spr"
"sprite_trans" "5"
"targetname" "snowparticle"
"target" "splash"
"classname" "ts_particle"
}
{
"targetname" "splash"
"lifehigh" "-1"
"classname" "ts_particle"
}
{
"origin" "576 1184 104"
"variancehigh" "1024 1248 0"
"variancelow" "-1024 -1248 0"
"ppehigh" "7"
"ppelow" "7"
"epshigh" "70"
"epslow" "65"
"particle" "snowparticle"
"spawnflags" "1"
"classname" "ts_emitter"
}
"#;

    #[test]
    fn parses_lump() {
        let fx = MapFx::from_lump(LUMP);
        assert_eq!(fx.fog, Some(Fog { near: 2048.0, far: 9000.0, color: [0.8, 0.8, 0.8] }));
        assert_eq!(fx.templates.len(), 2);
        let t = &fx.templates[0];
        assert_eq!(t.sprite_name, "sprites/snowy.spr");
        assert_eq!(t.life, (30.0, 30.0));
        assert_eq!(t.color, (1.5, 0.4), "no clamping in the TS client");
        assert_eq!(t.target_index, Some(1));
        assert_eq!(fx.templates[1].life.0, -1.0, "no clamping in the TS client");
        assert_eq!(fx.emitters.len(), 1);
        let e = &fx.emitters[0];
        assert_eq!(e.template, Some(0));
        assert_eq!(e.ppe, (7, 7));
        assert_eq!(e.variance.1, [-1024.0, -1248.0, 0.0]);
        assert_eq!(e.spawnflags, 1);

        let p = t.roll(&mut |a, b| (a + b) * 0.5, &mut |a, _| a);
        assert_eq!(p.vel, [0.0, 0.0, -0.5], "units per 0.1 s -> per second");
        assert_eq!(t.sprite_trans, 5);
    }

    #[test]
    fn bsp_header() {
        let mut b = vec![0u8; 0x7c];
        b[0..4].copy_from_slice(&30i32.to_le_bytes());
        b[4..8].copy_from_slice(&0x7ci32.to_le_bytes());
        let lump = b"{ \"classname\" \"ts_fog\" \"far\" \"10\" }\0";
        b[8..12].copy_from_slice(&(lump.len() as i32).to_le_bytes());
        b.extend_from_slice(lump);
        let text = entity_lump(&b).unwrap();
        assert_eq!(MapFx::from_lump(text).fog.unwrap().far, 10.0);
        b[0] = 31;
        assert!(entity_lump(&b).is_none());
    }

    fn xorshift(seed: u64) -> impl FnMut() -> u64 {
        let mut x = seed;
        move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        }
    }

    /// A BSP from the game dir can hold anything: short headers, negative or huge lump offsets and
    /// lengths, lumps past the end, non-UTF-8 text.
    #[test]
    fn bsp_header_survives_garbage() {
        let mut next = xorshift(0x5eed_0fb5_f00d);
        for len in [0usize, 3, 4, 8, 11, 12, 13, 64] {
            for _ in 0..200 {
                let mut b: Vec<u8> = (0..len).map(|_| next() as u8).collect();
                let _ = entity_lump(&b);
                if b.len() >= 12 {
                    b[0..4].copy_from_slice(&30i32.to_le_bytes());
                    let _ = entity_lump(&b).map(MapFx::from_lump);
                }
            }
        }
        for (ofs, n) in [(-1, 4), (12, -1), (i32::MAX, i32::MAX), (i32::MIN, 1), (12, 1000), (0, 12)] {
            let mut b = vec![0xffu8; 32];
            b[0..4].copy_from_slice(&29i32.to_le_bytes());
            b[4..8].copy_from_slice(&i32::to_le_bytes(ofs));
            b[8..12].copy_from_slice(&i32::to_le_bytes(n));
            let _ = entity_lump(&b).map(MapFx::from_lump);
        }
    }

    /// Random entity text and keyvalues with extreme numbers parse without a panic.
    #[test]
    fn lump_text_survives_garbage() {
        let mut next = xorshift(0xabcd_ef01);
        let alphabet = b"{}\"/ \n\tab0-.e9\0\xff";
        for _ in 0..300 {
            let len = (next() % 200) as usize;
            let bytes: Vec<u8> = (0..len).map(|_| alphabet[(next() % alphabet.len() as u64) as usize]).collect();
            let text = String::from_utf8_lossy(&bytes);
            let _ = MapFx::from_lump(&text);
        }
        let weird = r#"{ "classname" "ts_particle" "targetname" "a" "target" "a" "current_framehigh" "1e40"
            "follow_view" "-nan" "oncollide" "inf" "lifehigh" "nan" "sprite_trans" "-1e40" }
            { "classname" "ts_emitter" "particle" "a" "ppehigh" "2147483647" "ppelow" "-2147483648"
            "variancehigh" "inf -inf nan" "epshigh" "1e40" }
            { "classname" "ts_fog" "near" "nan" "color" "1e40" }"#;
        let fx = MapFx::from_lump(weird);
        assert_eq!(fx.templates[0].target_index, Some(0), "a template may target itself");
        assert_eq!(fx.emitters[0].ppe, (i32::MAX, i32::MIN));
        // Division by the (int) divisor with any count.
        for n in [i32::MIN, -1, 0, 1, i32::MAX] {
            for d in [1, 2, 8, i32::MAX] {
                let _ = ceil_div(n, d);
            }
        }
    }

    #[test]
    fn tokenizer_tolerates_garbage() {
        assert_eq!(parse_entities("{ \"a\" \"b\" } junk { \"c\" \"d\" }").len(), 1);
        assert!(parse_entities("").is_empty());
        assert_eq!(parse_entities("{ \"a\" ").len(), 0);
    }

    #[test]
    fn camera_spawn_leads_the_player() {
        let cam = [100.0, -50.0, 0.0];
        assert_eq!(camera_xy(cam, 10.0, 20.0, None), (110.0, -30.0));
        assert_eq!(camera_xy(cam, 10.0, 20.0, Some([200.0, -100.0, 999.0])), (410.0, -180.0));
        // `vel × 1.5 + x` rounds once (`0x1003b0b0`): 16777215 × 1.5 isn't a float.
        assert_eq!(camera_xy([0.0; 3], 1.0, 0.0, Some([16_777_215.0, 0.0, 0.0])).0, 25_165_824.0);
        // `2r − (2r − r) × min(density, 1)` (`0x1003b024`).
        assert_eq!(camera_radius(100.0, 0.25), 175.0);
        assert_eq!(camera_radius(100.0, 3.0), 100.0);
    }

    /// `0x1003b1d4..0x1003b22c`: `(float)(1/eps)`, scaled by `1/max(ts, 0.01f)`, the divisor and
    /// the LOD factor, added to the clock on the x87 and stored as a float.
    #[test]
    fn next_emit_is_a_float() {
        assert_eq!(next_emit(10.0, 2.0, 1.0, 1, 1.0), 10.5);
        assert_eq!(next_emit(10.0, 2.0, 0.5, 2, 1.0), 12.0, "slow motion and cl_fx divisor stretch it");
        assert_eq!(next_emit(10.0, 0.0, 1.0, 4, 2.5), 20.0, "no rate: div × f");
        assert_eq!(next_emit(0.1, 0.0, 1.0, 1, 1.0), 1.1f32 as f64, "stored as a float");
        assert_eq!(next_emit(10.0, 2.0, 0.0, 1, 1.0), 60.0, "time scale floored at 0.01f");
    }

    /// `ParticleKeyValue`/`EmitterKeyValue`: `stricmp` keys with trailing spaces cut, `atoi`
    /// for counts and flags, `atof` scalars, `sscanf("%f %f %f")` vectors that keep what they
    /// read and print a line for a template vector that isn't three numbers.
    #[test]
    fn keyvalues_parse_as_c_does() {
        let kv = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<Vec<_>>();
        let mut msgs = Vec::new();
        let t = Template::parse(
            &kv(&[
                ("Collide ", "0.5"),
                ("LIFEHIGH", "2.5s"),
                ("current_framehigh", "1e3"),
                ("velocitylow", "1 x 3"),
                ("followaxis", "0 0 1"),
                ("colorhigh", "1e-1"),
            ]),
            &mut msgs,
        );
        assert!(!t.collide, "atoi(\"0.5\") is 0");
        assert_eq!(t.life.0, 2.5);
        assert_eq!(t.frame.0, 1);
        assert_eq!(t.velocity.1, [1.0, 0.0, 0.0], "sscanf stops at the bad component");
        assert_eq!(t.followaxis, [0.0, 0.0, 1.0]);
        assert_eq!(t.color.0, 0.1);
        assert_eq!(msgs, ["cl: velocitylow has bad value. Should be \"x y z\"\n"]);
        let e = Emitter::from_kv(&kv(&[("PPEHIGH", "12.9"), ("origin", "5 6"), ("epslow", " -2")]));
        assert_eq!((e.ppe.0, e.origin, e.eps.1), (12, [5.0, 6.0, 0.0], -2.0));
        let fx = MapFx::from_lump(r#"{ "classname" "light" "color" "1 2" } { "ClassName" "TS_FOG" "far" "9" }"#);
        assert_eq!(fx.fog.map(|f| f.far), Some(9.0));
        assert_eq!(fx.messages, ["color has bad value. Should be \"r g b\"\n"]);
    }

    #[test]
    fn emitter_camera_tests() {
        let o = v::ZERO;
        let (lo, hi) = ([-10.0, -10.0, 0.0], [10.0, 10.0, 0.0]);
        assert!(inside_xy(o, lo, hi, [0.0, 0.0, 500.0]), "z is ignored");
        assert!(!inside_xy(o, lo, hi, [10.0, 0.0, 0.0]), "strict");
        assert!(!inside_xy(o, hi, lo, [0.0, 0.0, 0.0]), "low above high: never inside");
        // The sums stay on the x87: 1e8 ± 1 are either side of 1e8 there, but 1e8 as floats.
        assert!(inside_xy([1e8, 0.0, 0.0], [-1.0, -1.0, 0.0], [1.0, 1.0, 0.0], [1e8, 0.0, 0.0]));

        let cam = Camera { origin: [0.0, 0.0, 1000.0], forward: [1.0, 0.0, 0.0], ..Default::default() };
        // A box 100 units ahead, far below: the 2D direction is straight ahead.
        let (lo, hi) = ([90.0, -10.0, 0.0], [110.0, 10.0, 0.0]);
        let size = v::sub(hi, lo);
        let f = lod_factor(o, lo, hi, &cam).unwrap();
        assert!((f - 100.0 / size[0].hypot(size[1])).abs() < 1e-4, "XY distance over the 3D size: {f}");
        assert_eq!(lod_factor([100.0, 0.0, 0.0], v::ZERO, v::ZERO, &cam), Some(10.0), "zero size: f = 10");
        let up = Camera { forward: [0.0, 0.0, 1.0], ..cam };
        assert_eq!(lod_factor(o, lo, hi, &up), None, "looking straight up: z is 0 in the dot");

        assert_eq!(ceil_div(7, 2), 4);
        assert_eq!(ceil_div(8, 2), 4);
        assert_eq!(ceil_div(-1, 2), 1);
        assert_eq!(ceil_div(-3, 2), 0);
        assert_eq!(ceil_div(-3, 1), -3);
    }
}
