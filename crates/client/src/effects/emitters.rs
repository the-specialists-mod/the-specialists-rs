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
    /// `ParticleKeyValue` (`0x10055f90`) plus the server's `CTSParticle::CheckValues` clamps
    /// (the client copy has none; the doc recommends applying them).
    pub fn from_kv(kv: &[(String, String)]) -> Template {
        let mut t = Template { followaxis: [0.0, 0.0, 1.0], ..Default::default() };
        for (k, val) in kv {
            let f = num(val);
            let vv = goldsrc::util::parse_vec3(val);
            match k.as_str() {
                "targetname" => t.targetname = val.clone(),
                "target" => t.target = val.clone(),
                "sprite_name" => t.sprite_name = val.clone(),
                "sprite_trans" => t.sprite_trans = f as i32,
                "follow_view" => t.follow_view = (f as i32).clamp(0, 3) as u8,
                "collide" => t.collide = f != 0.0,
                "oncollide" => t.oncollide = (f as i32).clamp(0, 3) as u8,
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
        let clamp_pair = |p: &mut (f32, f32), lo: f32, hi: f32| {
            p.0 = p.0.clamp(lo, hi);
            p.1 = p.1.clamp(lo, hi);
        };
        clamp_pair(&mut t.life, 0.0, f32::MAX);
        clamp_pair(&mut t.color, 0.0, 1.0);
        clamp_pair(&mut t.fadespeed, 0.0, f32::MAX);
        clamp_pair(&mut t.radius, 0.0, f32::MAX);
        t.frame.0 = t.frame.0.max(0);
        t.frame.1 = t.frame.1.max(0);
        t
    }

    /// `Particle_RandomizeFromTemplate` (`0x1003a9b0`). Velocities: TS units per 0.1 s -> u/s.
    pub fn roll(&self, rnd: &mut impl FnMut(f32, f32) -> f32) -> Particle {
        fn pickv(rnd: &mut impl FnMut(f32, f32) -> f32, p: (Vec3, Vec3)) -> Vec3 {
            [0, 1, 2].map(|i| rnd(p.1[i].min(p.0[i]), p.0[i].max(p.1[i])))
        }
        let velocity = pickv(rnd, self.velocity);
        let avel = pickv(rnd, self.avelocity);
        let mut pick = |p: (f32, f32)| rnd(p.1.min(p.0), p.0.max(p.1));
        Particle {
            kind: psys::KIND_MAP,
            rendermode: self.sprite_trans,
            vel: v::scale(velocity, 10.0),
            avel: v::scale(avel, 10.0),
            randvel_lo: v::scale(self.randomvel.1, 10.0),
            randvel_hi: v::scale(self.randomvel.0, 10.0),
            randomdelta: pick(self.randomdelta) * 10.0,
            life: pick(self.life),
            gravity: pick(self.gravity),
            color: pick(self.color),
            fadespeed: pick(self.fadespeed),
            radius: pick(self.radius),
            scalespeed: pick(self.scalespeed),
            frame: pick((self.frame.0 as f32, self.frame.1 as f32)).round(),
            framerate: pick(self.framerate),
            react: pick(self.react),
            wind: pick(self.wind),
            fadein: pick(self.fadein),
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
        for e in ents.iter().filter(|e| classname(e) == "ts_particle").take(512) {
            fx.templates.push(Template::from_kv(e));
        }
        let names: Vec<String> = fx.templates.iter().map(|t| t.targetname.clone()).collect();
        for t in &mut fx.templates {
            t.target_index = (!t.target.is_empty()).then(|| names.iter().position(|n| *n == t.target)).flatten().map(|i| i as u16);
        }
        // pass 2: emitters, linked to the template whose targetname == particle
        for e in ents.iter().filter(|e| classname(e) == "ts_emitter").take(512) {
            let mut em = Emitter::from_kv(e);
            em.template = names.iter().position(|n| *n == em.particle);
            fx.emitters.push(em);
        }
        // pass 3: fog
        if let Some(e) = ents.iter().find(|e| classname(e) == "ts_fog") {
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
        let div = ps.divisor.max(1.0);
        for em in self.emitters.iter_mut().filter(|e| !e.dead) {
            if em.next_emit >= clock {
                continue;
            }
            let Some(ti) = em.template else { continue };
            let mut n = r::random_long(em.ppe.1.min(em.ppe.0), em.ppe.0.max(em.ppe.1));
            // Only 0 is rejected (0x1003ac6d). A negative count goes on: it adds no particles but
            // still plays the emit sound and reschedules.
            if n == 0 {
                continue;
            }
            let lo = v::add(em.origin, em.variance.1);
            let hi = v::add(em.origin, em.variance.0);
            let inside = (lo[0].min(hi[0])..=lo[0].max(hi[0])).contains(&cam.origin[0])
                && (lo[1].min(hi[1])..=lo[1].max(hi[1])).contains(&cam.origin[1]);
            let mut f = 1.0f32;
            if !inside {
                let centre = v::scale(v::add(lo, hi), 0.5);
                let d = v::sub(centre, cam.origin);
                let dist = v::len(d).max(1.0);
                // Behind the camera: skipped this frame and not rescheduled (0x1003ad96 jumps
                // past the `next_emit` writes), so it is tested again next frame.
                if v::dot(cam.forward, v::scale(d, 1.0 / dist)) < 0.1 {
                    continue;
                }
                f = (dist / v::len(v::sub(hi, lo)).max(1.0)).clamp(1.0, 10.0);
            }
            n = (n as f32 / div).ceil() as i32;
            let count = ((n as f32) / f).ceil() as i32;
            let tmpl = &self.templates[ti];
            // The template's sprite is looked up only now, after the random count and the camera
            // test (0x1003ae5f). A missing one is reported and the emitter is tried again next
            // frame: 0x1003ae69..0x1003ae78 skip the `next_emit` writes.
            let Some(spr) = ps.sprite(&tmpl.sprite_name) else {
                engine().print(&format!("Sprite '{}' is not a valid sprite.\n", tmpl.sprite_name));
                continue;
            };
            // The emit sound plays at the scratch particle's origin (`0x1018a720+0x14`), which
            // `0x1004ffe0` zeroes and the loop sets: the last particle's, or the world origin when
            // a negative count added none.
            let mut last = v::ZERO;
            for _ in 0..count {
                let mut p = tmpl.roll(&mut |a, b| r::random_float(a, b));
                p.sprite = spr;
                let mut pos = [
                    r::random_float(lo[0].min(hi[0]), lo[0].max(hi[0])),
                    r::random_float(lo[1].min(hi[1]), lo[1].max(hi[1])),
                    r::random_float(lo[2].min(hi[2]), lo[2].max(hi[2])),
                ];
                if em.cameradensity > 0.01 && em.cameraradius > 0.01 && inside {
                    let rad = em.cameraradius * (2.0 - em.cameradensity.min(1.0));
                    pos[0] = cam.origin[0] + r::random_float(-rad, rad);
                    pos[1] = cam.origin[1] + r::random_float(-rad, rad);
                }
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
            let eps = r::random_float(em.eps.1.min(em.eps.0), em.eps.0.max(em.eps.1));
            let iv = if eps > 0.0 { (1.0 / eps) / ts.max(0.01) } else { 1.0 };
            em.next_emit = clock + (iv * div * f) as f64;
        }
    }

    /// One particle of template `t` at `pos` (collision `oncollide` 2/3 targets).
    pub fn spawn_one(&self, ps: &mut PSys, t: u16, pos: Vec3, now: f64) {
        let Some(tmpl) = self.templates.get(t as usize) else { return };
        let Some(spr) = ps.sprite(&tmpl.sprite_name) else { return };
        let mut p = tmpl.roll(&mut |a, b| r::random_float(a, b));
        p.sprite = spr;
        p.origin = pos;
        if tmpl.spawnflags & 1 == 0 {
            p.vel = v::ZERO;
        }
        ps.add(p, now);
    }
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
        assert_eq!(t.color, (1.0, 0.4), "colour clamped to 0..1");
        assert_eq!(t.target_index, Some(1));
        assert_eq!(fx.templates[1].life.0, 0.0, "life clamped >= 0");
        assert_eq!(fx.emitters.len(), 1);
        let e = &fx.emitters[0];
        assert_eq!(e.template, Some(0));
        assert_eq!(e.ppe, (7, 7));
        assert_eq!(e.variance.1, [-1024.0, -1248.0, 0.0]);
        assert_eq!(e.spawnflags, 1);

        let p = t.roll(&mut |a, b| (a + b) * 0.5);
        assert_eq!(p.vel, [0.0, 0.0, -0.5], "units per 0.1 s -> per second");
        assert_eq!(p.rendermode, 5);
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

    #[test]
    fn tokenizer_tolerates_garbage() {
        assert_eq!(parse_entities("{ \"a\" \"b\" } junk { \"c\" \"d\" }").len(), 1);
        assert!(parse_entities("").is_empty());
        assert_eq!(parse_entities("{ \"a\" ").len(), 0);
    }
}
