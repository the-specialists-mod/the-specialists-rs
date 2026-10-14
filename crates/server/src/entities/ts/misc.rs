//! Small TS entities: `ts_model`, `ts_mapglobals`, `ts_wind`, the client-side effect entities
//! (`ts_fog` and `ts_emitter` are validated, then removed; `ts_particle` stays; the client reads
//! the BSP entity lump), `deadcorpse`, team spawn points, and the `ts_sellzone` stub. The
//! objective classes are in `objective`.

use crate::entities::stock::animating::{Animating, init_bone_controllers};
use crate::entities::stock::base::{Toggle, ToggleState, create_entity, kv_f, kv_i};
use crate::messages::Msg;
use crate::rules::{msg, state};
use crate::save::{BaseFuncs, Fields, tables};
use crate::world::{Cx, Entity};
use goldsrc::edict::EdictPtr;
use goldsrc::server::{engine, intern};
use goldsrc::sys::*;
use shared::protocol::rules::TsPSysWind;
use std::ffi::CStr;

// ---- ts_model ------------------------------------------------------------------------------------

/// `CTSModel` (0x722c4): a static studio model playing its sequence. TS builds it on
/// `CBaseToggle`, whose `KeyValue` it uses (and saves).
pub struct Model {
    t: Toggle<Model>,
    anim: Animating,
}

impl Default for Model {
    /// TS's object is zero-filled: `m_toggle_state` 0 (`TS_AT_TOP`), which nothing changes.
    fn default() -> Self {
        Self { t: Toggle { state: ToggleState::AtTop, ..Toggle::default() }, anim: Animating::default() }
    }
}

impl Entity for Model {
    /// `CBaseToggle::KeyValue` (0xe3534).
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        self.t.key_value(key, value)
    }

    /// `CBaseToggle::Save` / `Restore`: TS's chain down to `CBaseToggle`, no member pointers.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut Fields) {
        let mut anim = super::anim_to_save(&self.anim);
        self.t.save_chain(cx, f, &mut anim, false, false);
        super::anim_from_save(&mut self.anim, &anim);
    }

    // `CTSModel::Precache` (0x72460) is empty.

    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        let e = cx.edict;
        cx.pev().classname = eng.alloc_string(c"ts_model");
        let m = cx.pev().model;
        // `CTSModel::Spawn` (0x722e8) frees the edict at once (`REMOVE_ENTITY`, no
        // `UpdateOnRemove`). Guard: an empty model string is removed too; TS would precache and
        // set "" (an engine error).
        if m == 0 || eng.string(m).is_empty() {
            eng.remove_entity(e);
            return;
        }
        let name = intern(&eng.string(m).to_string_lossy());
        eng.precache_model(name);
        eng.set_model(e, name);
        eng.set_size(e, [0.0; 3], [0.0; 3]);
        let org = cx.pev().origin;
        eng.set_origin(e, org);
        init_bone_controllers(e);
        let v = cx.pev();
        v.solid = SOLID_SLIDEBOX as i32;
        v.movetype = MOVETYPE_NONE as i32;
        v.takedamage = DAMAGE_NO as f32;
        v.effects = 0;
        v.health = 1.0;
        v.yaw_speed = 5.0;
        v.ideal_yaw = v.angles[1];
        // `m_flFrameRate` 75 and `m_flGroundSpeed` 0, which `ResetSequenceInfo` replaces; no think
        // is set, so the `nextthink` bump does nothing.
        self.anim.frame_rate = 75.0;
        self.anim.ground_speed = 0.0;
        v.nextthink += 1.0;
        self.anim.reset_sequence_info(e);
    }
}

// ---- ts_mapglobals -------------------------------------------------------------------------------

/// `CTSMapGlobals` (0x72464). Spawnflags: 1 no knives, 2 no pistols, 4 no SMGs, 8 no rifles,
/// 16 no shotguns, 32 no round timer, 64 super jump for everyone. No keyvalues.
#[derive(Default)]
pub struct MapGlobals {
    /// `player_respawn_num` (`+0x64`): saved, never written by TS 3.0 (its `KeyValue` 0x72528
    /// claims no key); a gameplay plugin may set it (`ServerRules::entity_key_value`).
    pub player_respawn: i32,
    /// `player_at_a_time_num` (`+0x68`): the same.
    pub player_at_a_time: i32,
}

/// What a `ts_mapglobals` sets for the map (`ServerRules::map_globals`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapFlags {
    /// The round timer runs (`GameRules::round_timer`).
    pub round_timer: bool,
    /// Every player has super jump (`GameRules::superjump_map`).
    pub superjump: bool,
    /// The weapon restriction's base mask (`GameRules::base_weapons`).
    pub weapons: i32,
}

impl MapFlags {
    /// TS 3.0's (`CTSMapGlobals::Spawn` 0x72534): flag 32 stops the round timer, flag 64 gives
    /// super jump, and `get_weapon_restriction` (0x7263c) is `InitWeaponRestriction() ^
    /// spawnflags` on the whole int, so the round-timer and super-jump bits land in the mask too
    /// (SrvSett 3).
    pub fn from_spawnflags(flags: i32) -> MapFlags {
        MapFlags { round_timer: flags & 32 == 0, superjump: flags & 64 != 0, weapons: 0x1f ^ flags }
    }
}

impl Entity for MapGlobals {
    /// `CTSMapGlobals::Save` / `Restore` (0x72488 / 0x724d8): `player_respawn_num` (`+0x64`) and
    /// `player_at_a_time_num` (`+0x68`). No member pointers (`Spawn` clears them; `WaitThink` is
    /// never set).
    fn save_fields(&mut self, cx: &mut Cx, f: &mut Fields) {
        f.base_funcs(cx, BaseFuncs::default());
        f.table(&tables::CTSMapGlobals);
        f.int(&mut self.player_respawn);
        f.int(&mut self.player_at_a_time);
    }

    // `CTSMapGlobals::Precache` (0x72664) is empty.

    /// `CTSMapGlobals::Spawn` (0x72534).
    fn spawn(&mut self, cx: &mut Cx) {
        let e = cx.edict;
        let eng = engine();
        let flags = cx.pev().spawnflags;
        let v = cx.pev();
        v.movetype = MOVETYPE_NONE as i32;
        v.solid = SOLID_NOT as i32;
        let org = v.origin;
        eng.set_origin(e, org);
        eng.set_size(e, [0.0; 3], [0.0; 3]);
        cx.think_in(10.0);
        let Some(m) = crate::gameplay::active().server.map_globals(e, flags) else { return };
        state::with(|g| {
            g.round_timer = m.round_timer;
            g.superjump_map = m.superjump;
            g.base_weapons = m.weapons;
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
    /// `m_pfnThink` (`BlowThink`), set by `Spawn`.
    funcs: BaseFuncs,
}

impl Entity for Wind {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        // `CTSWind::KeyValue` (0x73ec4): `strcasecmp`, `strtod`.
        let v = kv_f(value);
        let k = key.to_str().unwrap_or("").to_ascii_lowercase();
        match k.as_str() {
            "base" => self.base = v,
            "variance" => self.variance = v,
            "frequency" => self.frequency = v,
            _ => return false,
        }
        true
    }

    /// `CBaseEntity::Save` / `Restore`: the base, with `BlowThink` set (so TS's restore stops after
    /// `"BASE"`, and the restored wind no longer blows).
    fn save_fields(&mut self, cx: &mut Cx, f: &mut Fields) {
        super::save_base(cx, f, &mut self.funcs);
    }

    fn spawn(&mut self, cx: &mut Cx) {
        cx.pev().classname = engine().alloc_string(c"ts_wind");
        self.funcs.think = true;
        cx.think_in(0.01);
    }

    fn think(&mut self, cx: &mut Cx) {
        if !self.funcs.think {
            return;
        }
        let eng = engine();
        let yaw = cx.pev().angles[1];
        let m = TsPSysWind {
            kind: 3,
            seed: eng.random_long(0, 30000),
            // `(int)` of the extended sum: truncated.
            yaw: super::c_int(f64::from(eng.random_long(-45, 45)) + f64::from(yaw)),
            strength: self.base + eng.random_float(-self.variance, self.variance),
        };
        msg::all(Msg::TsPSys, |w| m.write(w));
        // `time + frequency + RandomFloat(-f/2, f/2)` in extended precision, with no lower bound:
        // a wind without a `frequency` blows every frame.
        let f = self.frequency;
        let r = eng.random_float(-f * 0.5, f * 0.5);
        cx.pev().nextthink = (f64::from(eng.time()) + f64::from(f) + f64::from(r)) as f32;
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

/// `ts_emitter` (CTSEmitter): validates the emission rates, then removes itself. TS's `Spawn`
/// (0x741d4) sets `InitThink` before its `UTIL_Remove`, so the object never reaches a save: the
/// default (`CBaseEntity`, no pointers) `save_fields` stands. Its "Too many emitters" branch
/// can't run: the emitter index (`+0x64`) is only set by `CTSSharedEmitter::AddEmitter`, which
/// nothing calls.
#[derive(Default)]
pub struct Emitter {
    /// `epslow` / `epshigh` (`strtod`).
    eps: [f32; 2],
    /// `ppelow` / `ppehigh` (`strtol`).
    ppe: [i32; 2],
}

impl Entity for Emitter {
    /// `CTSEmitter::KeyValue` (0x73e3c) → `CTSSharedEmitter::EmitterKeyValue` (0xf840c):
    /// `strcasecmp` keys; it claims only its own (the client reads the rest from the BSP).
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        let k = key.to_bytes();
        let is = |name: &[u8]| k.eq_ignore_ascii_case(name);
        if is(b"epslow") {
            self.eps[0] = kv_f(value);
        } else if is(b"epshigh") {
            self.eps[1] = kv_f(value);
        } else if is(b"ppelow") {
            self.ppe[0] = kv_i(value);
        } else if is(b"ppehigh") {
            self.ppe[1] = kv_i(value);
        } else {
            return [&b"particle"[..], b"variancelow", b"variancehigh", b"cameradensity", b"cameraradius", b"origin"].iter().any(|n| is(n));
        }
        true
    }

    /// `CTSEmitter::Spawn` (0x741d4): `CheckValues` (0x740ac), then `UTIL_Remove`.
    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        let (eps, ppe) = emitter_complaints(self.eps, self.ppe);
        if eps {
            eng.alert(ALERT_TYPE::at_error, "Emits per second <= 0! Should be greater then zero.\n");
        }
        if ppe {
            eng.alert(ALERT_TYPE::at_error, "Particles per emit <= 0! Should be greater then zero.\n");
        }
        crate::entities::remove(cx.edict);
    }
}

/// `CTSEmitter::CheckValues` (0x740ac): either rate at or below zero is an error.
fn emitter_complaints(eps: [f32; 2], ppe: [i32; 2]) -> (bool, bool) {
    (eps[0] <= 0.0 || eps[1] <= 0.0, ppe[0] < 1 || ppe[1] < 1)
}

/// `CTSSharedEmitter`'s particle table size (`AddParticle` 0xf8178).
const MAX_PARTICLES: u32 = 512;

/// `ts_particle` (CTSParticle): takes a slot in the shared emitter and stays (the client reads
/// the BSP; the server keeps the entity), precaching `soundonemit`.
#[derive(Default)]
pub struct Particle {
    sound: Option<String>,
}

/// The shared emitter's used particle slots. `AddParticle` takes the first free one and only
/// `KillAll` frees them, so a count stands for the table.
pub static PARTICLES: goldsrc::Global<std::cell::Cell<u32>> = goldsrc::Global(std::cell::Cell::new(0));

/// `CTSSharedEmitter::AddParticle` (0xf8178): the slot taken, or `None` when all are used.
fn add_particle() -> Option<u32> {
    let n = PARTICLES.0.get();
    if n >= MAX_PARTICLES {
        return None;
    }
    PARTICLES.0.set(n + 1);
    Some(n)
}

impl Entity for Particle {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        // `CTSSharedEmitter::ParticleKeyValue` (0xf8644) matches keys with `strcasecmp`.
        if key.to_bytes().eq_ignore_ascii_case(b"soundonemit") {
            self.sound = Some(value.to_string_lossy().into_owned());
        }
        true
    }

    /// `CTSParticle::Spawn` (0x74134): removed only when the shared emitter is full; the sound
    /// is precached either way.
    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        cx.pev().classname = eng.alloc_string(c"ts_particle");
        if add_particle().is_none() {
            eng.alert(ALERT_TYPE::at_error, &format!("Too many particles! Particle limit is {MAX_PARTICLES} particles.\n"));
            crate::entities::remove(cx.edict);
        }
        if let Some(s) = self.sound.as_deref().filter(|s| !s.is_empty()) {
            eng.precache_sound(intern(s));
        }
    }
}

// ---- deadcorpse ------------------------------------------------------------------------------------

/// `CTSDeadCorpse`: a copy of a dead player's model left behind when they become an observer,
/// fading out over the last two thirds of its life.
#[derive(Default)]
pub struct DeadCorpse {
    /// `+0x64`: when it goes.
    die_time: f32,
    /// `+0x68`: its life.
    life: f32,
    /// `m_pfnThink` (`WaitThink`, from `Spawn`).
    funcs: BaseFuncs,
}

/// `CTSDeadCorpse::WaitThink`'s `renderamt`: the time left over two thirds of the life (at least
/// a second), as a share of 255.
fn corpse_amount(die_time: f32, life: f32, now: f32) -> f32 {
    let mut span = (life as f64 * 0.6666667f32 as f64) as f32;
    if span < 1.0 {
        span = 1.0;
    }
    let amt = ((die_time as f64 - now as f64) / span as f64 * 255.0) as f32;
    if amt > 255.0 { 255.0 } else { amt }
}

impl Entity for DeadCorpse {
    /// `CBaseEntity::Save` / `Restore` with `WaitThink` set. `CTSDeadCorpse::ObjectCaps` (0x705d0)
    /// is `FCAP_DONT_SAVE`, so this never runs for a save.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut Fields) {
        super::save_base(cx, f, &mut self.funcs);
    }

    /// `CTSDeadCorpse::Spawn` (0x7040c): `WaitThink` in half a second.
    fn spawn(&mut self, cx: &mut Cx) {
        self.funcs.think = true;
        cx.think_in(0.5);
    }

    /// `CTSDeadCorpse::WaitThink` (0x70460): every half second, fade, and go once past
    /// `die_time`.
    fn think(&mut self, cx: &mut Cx) {
        if !self.funcs.think {
            return;
        }
        cx.think_in(0.5);
        let now = cx.time();
        if now <= self.die_time {
            cx.pev().renderamt = corpse_amount(self.die_time, self.life, now);
        } else {
            crate::entities::remove(cx.edict);
        }
    }
}

/// `TSCreateDeadCorpse(player, life)` (0x70234): a visible player leaves a `deadcorpse`
/// (`CBaseEntity::Create`) wearing their model, frame and sequence, drawn as the dead player
/// (`kRenderFxDeadPlayer`, `rendercolor[0]` their index) through `kRenderTransColor` at 255.
pub fn create_dead_corpse(player: *mut edict_t, life: f32) {
    let eng = engine();
    let src = player.v();
    if src.effects & EF_NODRAW as i32 != 0 {
        return;
    }
    let e = create_entity(c"deadcorpse", src.origin, src.angles, std::ptr::null_mut());
    // Guard: TS writes through a failed `Create`'s NULL.
    if e.is_null() {
        return;
    }
    let now = eng.time();
    crate::world::with::<DeadCorpse, ()>(e, |c| {
        c.die_time = life + now;
        c.life = life;
    });
    let src = player.v();
    let idx = eng.index_of(player);
    {
        let v = e.v();
        v.angles = src.angles;
        v.model = src.model;
        v.modelindex = src.modelindex;
        v.frame = src.frame;
        v.colormap = src.colormap;
        v.movetype = MOVETYPE_TOSS as i32;
        v.velocity = src.velocity;
        v.flags = 0;
        v.deadflag = src.deadflag;
        v.rendermode = kRenderTransColor as i32;
        v.rendercolor = [idx as f32, 0.0, 0.0];
        v.renderfx = kRenderFxDeadPlayer as i32;
        v.renderamt = 255.0;
        v.effects = src.effects | EF_NOINTERP;
        v.sequence = src.sequence;
        v.animtime = src.animtime;
    }
    eng.set_origin(e, src.origin);
    eng.set_size(e, src.mins, src.maxs);
}

const EF_NOINTERP: i32 = 32;

// ---- spawn points and stubs -----------------------------------------------------------------------

/// `ts_sellzone`'s stub `KeyValue`: entvars keys were parsed before this; it claims every
/// class-specific key.
fn stub_key_value() -> bool {
    true
}

/// `ts_sellzone`'s stub `Spawn`: hidden and not solid; a brush keeps its model so the engine has a
/// valid bmodel. Its `SellTouch` (gated by `CashBased()`, always off in 3.0) isn't ported.
fn stub_spawn(cx: &mut Cx) {
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

/// `ts_sellzone` (`CTSSellZone`): the stub; TS saves its `CBaseEntity` part only (`attackerteam`
/// isn't saved), with `m_pfnTouch` (`SellTouch`) set by `Spawn` (0x72720).
#[derive(Default)]
pub struct SellZone {
    funcs: BaseFuncs,
}

impl Entity for SellZone {
    fn key_value(&mut self, _cx: &mut Cx, _key: &CStr, _value: &CStr) -> bool {
        stub_key_value()
    }

    fn spawn(&mut self, cx: &mut Cx) {
        stub_spawn(cx);
        self.funcs.touch = true;
    }

    fn save_fields(&mut self, cx: &mut Cx, f: &mut Fields) {
        super::save_base(cx, f, &mut self.funcs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emitter_checks_use_or() {
        // Either bound at or below zero complains (`||`), not only both.
        assert_eq!(emitter_complaints([1.0, 2.0], [1, 3]), (false, false));
        assert_eq!(emitter_complaints([0.0, 2.0], [1, 3]), (true, false));
        assert_eq!(emitter_complaints([1.0, -1.0], [0, 3]), (true, true));
        assert_eq!(emitter_complaints([1.0, 2.0], [4, 0]), (false, true));
    }

    #[test]
    fn particles_fill_the_shared_table() {
        PARTICLES.0.set(0);
        assert_eq!(add_particle(), Some(0));
        PARTICLES.0.set(MAX_PARTICLES - 1);
        assert_eq!(add_particle(), Some(MAX_PARTICLES - 1));
        // Full: the 513th is refused and takes no slot.
        assert_eq!(add_particle(), None);
        assert_eq!(PARTICLES.0.get(), MAX_PARTICLES);
        PARTICLES.0.set(0);
    }

    #[test]
    fn corpse_fades_over_two_thirds_of_its_life() {
        // A 10 s corpse (span 6.666667 s): opaque until the last two thirds, then linear.
        assert_eq!(corpse_amount(110.0, 10.0, 100.0), 255.0);
        assert_eq!(corpse_amount(110.0, 10.0, 110.0), 0.0);
        let half = corpse_amount(110.0, 10.0, 110.0 - 10.0 / 3.0);
        assert!((half - 127.5).abs() < 0.01, "{half}");
        // A short life fades over at least a second.
        assert_eq!(corpse_amount(100.5, 0.5, 100.0), 127.5);
    }
}
