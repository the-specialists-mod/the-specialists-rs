//! Small TS entities: `ts_model`, `ts_mapglobals`, `ts_wind`, the client-side effect entities
//! (`ts_fog`, `ts_emitter`, `ts_particle`: validated, then removed; the client reads the BSP entity
//! lump), `deadcorpse`, team spawn points, and harmless stubs for the dead objective classes.

use crate::messages::Msg;
use crate::rules::{msg, state};
use crate::world::{Cx, Entity};
use goldsrc::edict::EdictPtr;
use goldsrc::server::{engine, intern};
use goldsrc::sys::*;
use shared::protocol::rules::TsPSysWind;
use std::ffi::CStr;

// ---- ts_model ------------------------------------------------------------------------------------

/// `CTSModel` (0x722c4): a static studio model playing its sequence.
#[derive(Default)]
pub struct Model;

impl Entity for Model {
    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        let e = cx.edict;
        cx.pev().classname = eng.alloc_string(c"ts_model");
        let m = cx.pev().model;
        if m == 0 || eng.string(m).is_empty() {
            crate::entities::remove(e);
            return;
        }
        let name = intern(&eng.string(m).to_string_lossy());
        eng.precache_model(name);
        eng.set_model(e, name);
        eng.set_size(e, [0.0; 3], [0.0; 3]);
        let v = cx.pev();
        let org = v.origin;
        v.solid = SOLID_SLIDEBOX as i32;
        v.movetype = MOVETYPE_NONE as i32;
        v.takedamage = DAMAGE_NO as f32;
        v.effects = 0;
        v.health = 1.0;
        v.yaw_speed = 5.0;
        v.ideal_yaw = v.angles[1];
        v.framerate = 1.0;
        v.animtime = eng.time();
        eng.set_origin(e, org);
    }
}

// ---- ts_mapglobals -------------------------------------------------------------------------------

/// `CTSMapGlobals` (0x72464). Spawnflags: 1 no knives, 2 no pistols, 4 no SMGs, 8 no rifles,
/// 16 no shotguns, 32 no round timer, 64 super jump for everyone. No keyvalues.
#[derive(Default)]
pub struct MapGlobals;

impl Entity for MapGlobals {
    fn spawn(&mut self, cx: &mut Cx) {
        let flags = cx.pev().spawnflags;
        let v = cx.pev();
        v.movetype = MOVETYPE_NONE as i32;
        v.solid = SOLID_NOT as i32;
        state::with(|g| {
            g.round_timer = flags & 32 == 0;
            g.superjump_map = flags & 64 != 0;
            // `get_weapon_restriction` (0x7263c): `InitWeaponRestriction() ^ spawnflags` on the
            // whole int, so the round-timer and super-jump bits land in the mask too (SrvSett 3).
            g.base_weapons = 0x1f ^ flags;
            g.allowed_weapons = g.base_weapons;
        });
    }
}

// ---- ts_wind -------------------------------------------------------------------------------------

/// `CTSWind` (0x73df4): periodically broadcasts `TSPSys` type 3 (wind for the client's particles).
#[derive(Default)]
pub struct Wind {
    base: f32,
    variance: f32,
    frequency: f32,
}

impl Entity for Wind {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        let v: f32 = value.to_str().unwrap_or("").trim().parse().unwrap_or(0.0);
        let k = key.to_str().unwrap_or("").to_ascii_lowercase();
        match k.as_str() {
            "base" => self.base = v,
            "variance" => self.variance = v,
            "frequency" => self.frequency = v,
            _ => return false,
        }
        true
    }

    fn spawn(&mut self, cx: &mut Cx) {
        cx.pev().classname = engine().alloc_string(c"ts_wind");
        cx.think_in(0.01);
    }

    fn think(&mut self, cx: &mut Cx) {
        let eng = engine();
        let yaw = cx.pev().angles[1];
        let m = TsPSysWind {
            kind: 3,
            seed: eng.random_long(0, 30000),
            yaw: (eng.random_long(-45, 45) as f32 + yaw).round() as i32,
            strength: self.base + eng.random_float(-self.variance, self.variance),
        };
        msg::all(Msg::TsPSys, |w| m.write(w));
        let f = self.frequency;
        cx.think_in((f + eng.random_float(-f * 0.5, f * 0.5)).max(0.1));
    }
}

// ---- client-side effect entities -------------------------------------------------------------------

/// `ts_fog`: the client reads `near`/`far`/`color` from the BSP; the server removes it.
#[derive(Default)]
pub struct Fog;

impl Entity for Fog {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, _value: &CStr) -> bool {
        matches!(key.to_bytes(), b"near" | b"far" | b"color")
    }
    fn spawn(&mut self, cx: &mut Cx) {
        crate::entities::remove(cx.edict);
    }
}

/// `ts_emitter` (CTSEmitter): validates the emission rates, then removes itself.
#[derive(Default)]
pub struct Emitter {
    eps: [f32; 2],
    ppe: [f32; 2],
}

impl Entity for Emitter {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        let v: f32 = value.to_str().unwrap_or("").split_whitespace().next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
        match key.to_bytes() {
            b"epslow" => self.eps[0] = v,
            b"epshigh" => self.eps[1] = v,
            b"ppelow" => self.ppe[0] = v,
            b"ppehigh" => self.ppe[1] = v,
            _ => {}
        }
        true
    }
    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        if self.eps[0] <= 0.0 && self.eps[1] <= 0.0 {
            eng.alert(ALERT_TYPE::at_console, "ts_emitter: Emits per second <= 0!\n");
        }
        if self.ppe[0] <= 0.0 && self.ppe[1] <= 0.0 {
            eng.alert(ALERT_TYPE::at_console, "ts_emitter: Particles per emit <= 0!\n");
        }
        crate::entities::remove(cx.edict);
    }
}

/// `ts_particle` (CTSParticle): precaches `soundonemit`, then removes itself.
#[derive(Default)]
pub struct Particle {
    sound: Option<String>,
}

pub static PARTICLES: goldsrc::Global<std::cell::Cell<u32>> = goldsrc::Global(std::cell::Cell::new(0));

impl Entity for Particle {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        if key.to_bytes() == b"soundonemit" {
            self.sound = Some(value.to_string_lossy().into_owned());
        }
        true
    }
    fn spawn(&mut self, cx: &mut Cx) {
        let n = PARTICLES.0.get() + 1;
        PARTICLES.0.set(n);
        if n > 512 {
            engine().alert(ALERT_TYPE::at_console, "Too many particles!\n");
        }
        if let Some(s) = self.sound.as_deref().filter(|s| !s.is_empty()) {
            engine().precache_sound(intern(s));
        }
        crate::entities::remove(cx.edict);
    }
}

// ---- deadcorpse ------------------------------------------------------------------------------------

/// `CTSDeadCorpse`: a copy of a dead player's model left behind when they become an observer.
#[derive(Default)]
pub struct DeadCorpse {
    pub until: f32,
}

impl Entity for DeadCorpse {
    fn think(&mut self, cx: &mut Cx) {
        if cx.time() >= self.until {
            crate::entities::remove(cx.edict);
        } else {
            cx.think_in(1.0);
        }
    }
}

/// `TSCreateDeadCorpse(player, life)`.
pub fn create_dead_corpse(player: *mut edict_t, life: f32) {
    let eng = engine();
    let e = eng.create_named_entity(c"deadcorpse");
    if e.is_null() {
        return;
    }
    let src = player.v();
    let idx = eng.index_of(player);
    {
        let v = e.v();
        v.classname = eng.alloc_string(c"deadcorpse");
        v.angles = src.angles;
        v.sequence = src.sequence;
        v.frame = src.frame;
        v.velocity = src.velocity;
        v.movetype = MOVETYPE_TOSS as i32;
        v.solid = SOLID_NOT as i32;
        v.renderfx = kRenderFxDeadPlayer as i32;
        v.rendercolor[0] = idx as f32;
        v.renderamt = idx as f32;
        v.effects = src.effects | EF_NOINTERP;
        v.flags = 0;
        v.nextthink = eng.time() + 1.0;
    }
    eng.set_model(e, crate::player::PLAYER_MODEL);
    eng.set_size(e, src.mins, src.maxs);
    eng.set_origin(e, src.origin);
    let until = eng.time() + life;
    crate::world::with::<DeadCorpse, ()>(e, |c| c.until = until);
}

const EF_NOINTERP: i32 = 32;

// ---- spawn points and stubs -----------------------------------------------------------------------

/// Dead 3.0 objective entities (`ts_objective_manager`, `ts_objective_ptr`, `ts_bomb`, `ts_hack`,
/// `ts_teamescape`, `ts_sellzone`): spawn hidden and do nothing. Brushes keep their model so the
/// engine has a valid bmodel.
#[derive(Default)]
pub struct Stub;

impl Entity for Stub {
    fn key_value(&mut self, _cx: &mut Cx, _key: &CStr, _value: &CStr) -> bool {
        // Entvars keys were parsed before this; accept every class-specific key.
        true
    }
    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        let e = cx.edict;
        let m = cx.pev().model;
        if m != 0 && eng.string(m).to_bytes().first() == Some(&b'*') {
            let name = eng.string(m);
            eng.set_model(e, name);
        }
        let v = cx.pev();
        v.solid = SOLID_NOT as i32;
        v.movetype = MOVETYPE_NONE as i32;
        v.effects |= EF_NODRAW as i32;
    }
}
