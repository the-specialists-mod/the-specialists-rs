//! Map emitters (`CTS_CL_Emitter`, rendering doc §5.1-5.3). The server removes `ts_emitter`,
//! `ts_particle` and `ts_fog` at spawn; the client re-reads the map's BSP entity lump and builds
//! them locally. Parsing is pure (unit-tested); emission talks to the particle system.

use super::psys::{self, Camera, PSys, Particle};
use goldsrc::client::engine;
use goldsrc::ext::render::{self as r, Vec3, vec3 as v};

/// Splits a BSP entity lump into keyvalue blocks (`COM_ParseFile` rules: quoted strings,
/// `{`/`}`, `//` comments). Malformed text stops parsing, keeping complete blocks.
pub fn parse_entities(text: &str) -> Vec<Vec<(String, String)>> {
    let mut toks = Tokens { s: text.as_bytes(), i: 0 };
    let mut out = Vec::new();
    while let Some(t) = toks.next() {
        if t != "{" {
            break; // "found %s when expecting {"
        }
        let mut kv = Vec::new();
        loop {
            let Some(k) = toks.next() else { return out };
            if k == "}" {
                break;
            }
            let Some(val) = toks.next() else { return out };
            if val == "}" {
                break;
            }
            kv.push((k, val));
        }
        out.push(kv);
    }
    out
}

struct Tokens<'a> {
    s: &'a [u8],
    i: usize,
}

impl Tokens<'_> {
    fn next(&mut self) -> Option<String> {
        loop {
            while self.i < self.s.len() && self.s[self.i] <= b' ' {
                self.i += 1;
            }
            if self.i + 1 < self.s.len() && self.s[self.i] == b'/' && self.s[self.i + 1] == b'/' {
                while self.i < self.s.len() && self.s[self.i] != b'\n' {
                    self.i += 1;
                }
                continue;
            }
            break;
        }
        if self.i >= self.s.len() || self.s[self.i] == 0 {
            return None;
        }
        let c = self.s[self.i];
        if c == b'"' {
            self.i += 1;
            let start = self.i;
            while self.i < self.s.len() && self.s[self.i] != b'"' {
                self.i += 1;
            }
            let t = String::from_utf8_lossy(&self.s[start..self.i]).into_owned();
            self.i += 1;
            return Some(t);
        }
        if c == b'{' || c == b'}' {
            self.i += 1;
            return Some((c as char).to_string());
        }
        let start = self.i;
        while self.i < self.s.len() && self.s[self.i] > b' ' && !matches!(self.s[self.i], b'{' | b'}' | b'"') {
            self.i += 1;
        }
        Some(String::from_utf8_lossy(&self.s[start..self.i]).into_owned())
    }
}

/// Entity lump of a BSP file (`LoadEntityLump` `0x1003a860`): version 29 or 30, lump 0.
pub fn entity_lump(bsp: &[u8]) -> Option<&str> {
    let rd = |o: usize| bsp.get(o..o + 4).map(|b| i32::from_le_bytes(b.try_into().unwrap()));
    let ver = rd(0)?;
    if ver != 29 && ver != 30 {
        return None;
    }
    let (ofs, len) = (rd(4)? as usize, rd(8)? as usize);
    let lump = bsp.get(ofs..ofs.checked_add(len)?)?;
    let lump = lump.split(|&b| b == 0).next().unwrap_or(lump);
    std::str::from_utf8(lump).ok().or_else(|| std::str::from_utf8(&lump[..lump.iter().position(|&b| b >= 0x80).unwrap_or(0)]).ok())
}

fn num(s: &str) -> f32 {
    // atof: leading number of the string; vectors given for scalars use their first component.
    s.split_whitespace().next().and_then(|t| t.parse().ok()).unwrap_or(0.0)
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
        // No defaults (TS zeroes the struct): a `follow_view 3` template without `followaxis`
        // builds a zero-height quad, as in TS.
        let mut t = Template::default();
        for (k, val) in kv {
            let f = num(val);
            let vv = goldsrc::util::parse_vec3(val);
            match k.as_str() {
                "targetname" => t.targetname = val.clone(),
                "target" => t.target = val.clone(),
                "sprite_name" => t.sprite_name = val.clone(),
                "sprite_trans" => t.sprite_trans = f as i32,
                // The particle keeps the low byte (`+0x05`); `Quad_Build` treats 4+ as 0.
                "follow_view" => t.follow_view = f as i32 as u8,
                "collide" => t.collide = f != 0.0,
                // Only 1-3 pick a callback (`0x1003af76..0x1003afa9`); anything else has none.
                "oncollide" => t.oncollide = if (1..=3).contains(&(f as i32)) { f as i32 as u8 } else { 0 },
                "spawnflags" => t.spawnflags = f as i32,
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
                "current_framehigh" => t.frame.0 = f as i32,
                "current_framelow" => t.frame.1 = f as i32,
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
                "velocityhigh" => t.velocity.0 = vv,
                "velocitylow" => t.velocity.1 = vv,
                "a_velocityhigh" => t.avelocity.0 = vv,
                "a_velocitylow" => t.avelocity.1 = vv,
                "randomvelhigh" => t.randomvel.0 = vv,
                "randomvellow" => t.randomvel.1 = vv,
                "followaxis" => t.followaxis = vv,
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
        for (k, val) in kv {
            let f = num(val);
            match k.as_str() {
                "origin" => e.origin = goldsrc::util::parse_vec3(val),
                "particle" => e.particle = val.clone(),
                "epshigh" => e.eps.0 = f,
                "epslow" => e.eps.1 = f,
                "ppehigh" => e.ppe.0 = f as i32,
                "ppelow" => e.ppe.1 = f as i32,
                "variancehigh" => e.variance.0 = goldsrc::util::parse_vec3(val),
                "variancelow" => e.variance.1 = goldsrc::util::parse_vec3(val),
                "cameradensity" => e.cameradensity = f,
                "cameraradius" => e.cameraradius = f,
                "spawnflags" => e.spawnflags = f as i32,
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
}

fn classname(kv: &[(String, String)]) -> &str {
    kv.iter().find(|(k, _)| k == "classname").map_or("", |(_, v)| v.as_str())
}

impl MapFx {
    pub fn from_lump(text: &str) -> MapFx {
        let ents = parse_entities(text);
        let mut fx = MapFx::default();
        // pass 1: templates
        for e in ents.iter().filter(|e| classname(e).eq_ignore_ascii_case("ts_particle")).take(512) {
            fx.templates.push(Template::from_kv(e));
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
        // pass 3: fog. Every `ts_fog` overwrites the last (`0x1003a7f0`), so the last one counts.
        if let Some(e) = ents.iter().rev().find(|e| classname(e).eq_ignore_ascii_case("ts_fog")) {
            let mut f = Fog::default();
            for (k, val) in e.iter() {
                match k.as_str() {
                    "near" => f.near = num(val),
                    "far" => f.far = num(val),
                    "color" => f.color = goldsrc::util::parse_vec3(val),
                    _ => {}
                }
            }
            fx.fog = Some(f);
        }
        fx
    }

    /// Emission step (`CTS_CL_Emitter::Frame` `0x1003ab40`, doc §5.3). Emitters are scheduled on
    /// `clock`, the emitter clock (`0x10189478`); `now` (client time) stamps the particles.
    pub fn emit(&mut self, ps: &mut PSys, cam: &Camera, clock: f64, now: f64, ts: f32) {
        // `0x1018ca60` is an int (1, 2, 4, 6 or 8).
        let div = (ps.divisor as i32).max(1);
        for em in self.emitters.iter_mut().filter(|e| !e.dead) {
            if em.next_emit >= clock {
                continue;
            }
            let Some(ti) = em.template else { continue };
            // The raw keyvalues (0x1003ac54): with ppelow > ppehigh the engine returns ppelow.
            let n = r::random_long(em.ppe.1, em.ppe.0);
            // Only 0 is rejected (0x1003ac6d). A negative count goes on: it plays the emit sound
            // and reschedules.
            if n == 0 {
                continue;
            }
            let (low, high) = (em.variance.1, em.variance.0);
            let lo = v::add(em.origin, low);
            let hi = v::add(em.origin, high);
            let inside = inside_xy(lo, hi, cam.origin);
            let mut f = 1.0f32;
            if !inside {
                // Behind the camera: skipped this frame and not rescheduled (0x1003ad96 jumps
                // past the `next_emit` writes), so it is tested again next frame.
                let Some(lod) = lod_factor(lo, hi, v::sub(high, low), cam) else { continue };
                f = lod;
            }
            let n = ceil_div(n, div);
            // `(float)i < n / f` (0x1003afb3, 0x1003b139). Capped at the largest pool: nothing dies
            // during the loop, so particles past the pool size are dropped anyway, and a map's
            // `ppehigh` of 2^31 would otherwise freeze the game rolling them.
            // (Not `f32::min`, which would turn a NaN limit, no particles, into the cap.)
            let limit = n as f32 / f;
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
                    let rad = em.cameraradius * (2.0 - em.cameradensity.min(1.0));
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
                engine().ev_play_sound(0, last, goldsrc::sys::CHAN_STATIC as i32, &tmpl.soundonemit, 1.0, r::ATTN_NORM, 0, pitch);
            }
            if em.spawnflags & 1 != 0 {
                em.dead = true;
                continue;
            }
            let eps = r::random_float(em.eps.1, em.eps.0);
            let iv = if eps > 0.0 { (1.0 / eps) / ts.max(0.01) } else { 1.0 };
            em.next_emit = clock + (iv * div as f32 * f) as f64;
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

/// A camera-centred particle's XY: the camera plus the random offsets, then, when there is a
/// local player (`GetLocalPlayer` at 0x1003b0a0), 1.5 (`0x1007a860`) times its
/// `curstate.velocity` x/y (`+0x320`/`+0x324`), so the particles land where a moving player is
/// heading.
fn camera_xy(cam: Vec3, dx: f32, dy: f32, local_velocity: Option<Vec3>) -> (f32, f32) {
    let (x, y) = (cam[0] + dx, cam[1] + dy);
    match local_velocity {
        Some(vel) => (vel[0] * 1.5 + x, vel[1] * 1.5 + y),
        None => (x, y),
    }
}

/// The camera is strictly inside the emitter's XY box, with low and high as given
/// (0x1003ac75..0x1003acce): a box whose low is above its high has no inside.
fn inside_xy(lo: Vec3, hi: Vec3, cam: Vec3) -> bool {
    lo[0] < cam[0] && lo[1] < cam[1] && hi[0] > cam[0] && hi[1] > cam[1]
}

/// Level-of-detail factor of an emitter the camera is outside of, or `None` when its box centre
/// is behind the camera. Horizontal only (0x1003ace9..0x1003ad96): the direction has z = 0 and
/// the distance is the XY one, floored at 1e-4. `size` (high - low) is 3D and not floored
/// (0x1003ad9c..0x1003ae25), so a zero-size box gives an infinite ratio and f = 10.
fn lod_factor(lo: Vec3, hi: Vec3, size: Vec3, cam: &Camera) -> Option<f32> {
    let dx = (lo[0] + hi[0]) * 0.5 - cam.origin[0];
    let dy = (lo[1] + hi[1]) * 0.5 - cam.origin[1];
    let dist = (dx * dx + dy * dy).sqrt().max(1e-4);
    // The threshold is the double 0.1 (0x1007a540).
    if (((cam.forward[0] * dx + cam.forward[1] * dy) / dist) as f64) < 0.1 {
        return None;
    }
    Some((dist / v::len(size)).clamp(1.0, 10.0))
}

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
    }

    #[test]
    fn emitter_camera_tests() {
        let (lo, hi) = ([-10.0, -10.0, 0.0], [10.0, 10.0, 0.0]);
        assert!(inside_xy(lo, hi, [0.0, 0.0, 500.0]), "z is ignored");
        assert!(!inside_xy(lo, hi, [10.0, 0.0, 0.0]), "strict");
        assert!(!inside_xy(hi, lo, [0.0, 0.0, 0.0]), "low above high: never inside");

        let cam = Camera { origin: [0.0, 0.0, 1000.0], forward: [1.0, 0.0, 0.0], ..Default::default() };
        // A box 100 units ahead, far below: the 2D direction is straight ahead.
        let (lo, hi) = ([90.0, -10.0, 0.0], [110.0, 10.0, 0.0]);
        let size = v::sub(hi, lo);
        let f = lod_factor(lo, hi, size, &cam).unwrap();
        assert!((f - 100.0 / size[0].hypot(size[1])).abs() < 1e-4, "XY distance over the 3D size: {f}");
        assert_eq!(lod_factor(lo, hi, v::ZERO, &cam), Some(10.0), "zero size: f = 10");
        let up = Camera { forward: [0.0, 0.0, 1.0], ..cam };
        assert_eq!(lod_factor(lo, hi, size, &up), None, "looking straight up: z is 0 in the dot");

        assert_eq!(ceil_div(7, 2), 4);
        assert_eq!(ceil_div(8, 2), 4);
        assert_eq!(ceil_div(-1, 2), 1);
        assert_eq!(ceil_div(-3, 2), 0);
        assert_eq!(ceil_div(-3, 1), -3);
    }
}
