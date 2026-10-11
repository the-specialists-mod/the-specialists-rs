//! Small TS entities: `ts_model`, `ts_mapglobals`, `ts_wind`, the client-side effect entities
//! (`ts_fog`, `ts_emitter`, `ts_particle`: validated, then removed; the client reads the BSP entity
//! lump), `deadcorpse`, team spawn points, and harmless stubs for the dead objective classes.

use crate::entities::stock::animating::{Animating, init_bone_controllers};
use crate::entities::stock::base::{Toggle, ToggleState, alloc, kv_i};
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
pub struct MapGlobals;

impl Entity for MapGlobals {
    /// `CTSMapGlobals::Save` / `Restore` (0x72488 / 0x724d8). `player_respawn_num` (`+0x64`) and
    /// `player_at_a_time_num` (`+0x68`) are always 0: `KeyValue` (0x72528) claims no key and nothing
    /// writes them. No member pointers (`Spawn` clears them; `WaitThink` is never set).
    fn save_fields(&mut self, cx: &mut Cx, f: &mut Fields) {
        f.base_funcs(cx, BaseFuncs::default());
        f.table(&tables::CTSMapGlobals);
        f.zero();
        f.zero();
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
    /// `m_pfnThink` (`BlowThink`), set by `Spawn`.
    funcs: BaseFuncs,
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

/// `ts_emitter` (CTSEmitter): validates the emission rates, then removes itself. TS's `Spawn`
/// (0x741d4) sets `InitThink` before its `UTIL_Remove`, so the object never reaches a save: the
/// default (`CBaseEntity`, no pointers) `save_fields` stands.
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
        // `CTSSharedEmitter::ParticleKeyValue` (0xf8644) matches keys with `strcasecmp`.
        if key.to_bytes().eq_ignore_ascii_case(b"soundonemit") {
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
    /// `m_pfnThink` (`WaitThink`, from `Spawn`).
    funcs: BaseFuncs,
}

impl Entity for DeadCorpse {
    /// `CBaseEntity::Save` / `Restore` with `WaitThink` set. `CTSDeadCorpse::ObjectCaps` (0x705d0)
    /// is `FCAP_DONT_SAVE`, so this never runs for a save.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut Fields) {
        super::save_base(cx, f, &mut self.funcs);
    }

    fn think(&mut self, cx: &mut Cx) {
        if !self.funcs.think {
            return;
        }
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
    crate::world::with::<DeadCorpse, ()>(e, |c| {
        c.until = until;
        c.funcs.think = true;
    });
}

const EF_NOINTERP: i32 = 32;

// ---- spawn points and stubs -----------------------------------------------------------------------

/// Dead 3.0 objective entities (`ts_objective_manager`, `ts_objective_ptr`, `ts_bomb`, `ts_hack`,
/// `ts_teamescape`, `ts_sellzone`): spawn hidden and do nothing. Brushes keep their model so the
/// engine has a valid bmodel. `ts_objective_ptr` (`CTSObjectivePointer`) is this type itself: TS
/// saves only its `CBaseEntity` part, with no member pointers (`Spawn` 0x8bb44 clears them; `Use`
/// is a virtual). The others are the types below, which add the fields TS saves for their class
/// (set from the same keys as TS's `KeyValue`) and whether TS's `Spawn` set a think / touch. Not
/// ported: their behaviour (`Use`, `WaitThink`, `WaitTouch`, `SellTouch`, `EnableObjective`, the
/// objective list and HUD), their unsaved keys, and the entvars TS's `Spawn` sets beyond the stub's.
#[derive(Default)]
pub struct Stub;

/// The stub's `KeyValue`: entvars keys were parsed before this; it claims every class-specific key.
fn stub_key_value() -> bool {
    true
}

/// The stub's `Spawn`.
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

impl Entity for Stub {
    fn key_value(&mut self, _cx: &mut Cx, _key: &CStr, _value: &CStr) -> bool {
        stub_key_value()
    }
    fn spawn(&mut self, cx: &mut Cx) {
        stub_spawn(cx);
    }
}

/// `ts_objective_manager` (`CTSObjectiveManager`): the stub, with the strings TS saves.
#[derive(Default)]
pub struct ObjectiveManager {
    /// `messages[4]` (`+0x64`): `messagea`..`messaged`.
    messages: [string_t; 4],
    /// `targets[8]` (`+0x74`): `objptargeta`..`objptargeth`.
    targets: [string_t; 8],
    /// `end_round_target` (`+0x94`): `targetontimeleft`.
    end_round_target: string_t,
}

impl Entity for ObjectiveManager {
    /// `CTSObjectiveManager::KeyValue` (0x8b46c) for the saved keys (`ALLOC_STRING`); the stub
    /// claims the rest (`targetonspecdead` / `targetonmercdead` aren't saved).
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        let k = key.to_bytes();
        let slot = match k {
            b"messagea" | b"messageb" | b"messagec" | b"messaged" => Some(&mut self.messages[(k[7] - b'a') as usize]),
            b"objptargeta" | b"objptargetb" | b"objptargetc" | b"objptargetd" | b"objptargete" | b"objptargetf" | b"objptargetg"
            | b"objptargeth" => Some(&mut self.targets[(k[10] - b'a') as usize]),
            b"targetontimeleft" => Some(&mut self.end_round_target),
            _ => None,
        };
        if let Some(slot) = slot {
            *slot = alloc(value);
        }
        stub_key_value()
    }

    fn spawn(&mut self, cx: &mut Cx) {
        stub_spawn(cx);
    }

    /// `CTSObjectiveManager::Save` / `Restore`: no member pointers (`Spawn` 0x8b7bc clears them),
    /// then the thirteen strings.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut Fields) {
        f.base_funcs(cx, BaseFuncs::default());
        f.table(&tables::CTSObjectiveManager);
        for m in &mut self.messages {
            f.string(m);
        }
        for t in &mut self.targets {
            f.string(t);
        }
        f.string(&mut self.end_round_target);
    }
}

/// `CTSObjective`'s saved `attacker_team` (`+0x68`): `CTSObjective::KeyValue`'s `attackerteam`.
fn objective_key(attacker_team: &mut i32, key: &CStr, value: &CStr) {
    if key.to_bytes() == b"attackerteam" {
        *attacker_team = kv_i(value);
    }
}

/// `ts_bomb` (`CTSBomb`): the stub, with what TS saves.
#[derive(Default)]
pub struct Bomb {
    /// `attacker_team` (`CTSObjective` `+0x68`).
    attacker_team: i32,
    /// `defusesec` (`+0x80`): `defusetime`.
    defuse_sec: i32,
    /// `cancutwire` (`+0x84`): `cutwires`.
    can_cut_wire: i32,
    /// `Spawn` (0x8c470) sets `m_pfnThink` (`WaitThink`).
    funcs: BaseFuncs,
}

impl Entity for Bomb {
    /// `CTSBomb::KeyValue` (0x8cb68), then `CTSObjective::KeyValue`.
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"defusetime" => self.defuse_sec = kv_i(value),
            b"cutwires" => self.can_cut_wire = kv_i(value),
            _ => objective_key(&mut self.attacker_team, key, value),
        }
        stub_key_value()
    }

    fn spawn(&mut self, cx: &mut Cx) {
        stub_spawn(cx);
        self.funcs.think = true;
    }

    /// `CTSBomb::Save` / `Restore`: `"CTSObjective"` then `"CTSBomb"`. The set think stops TS's
    /// restore after `"BASE"`.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut Fields) {
        super::save_base(cx, f, &mut self.funcs);
        f.table(&tables::CTSObjective);
        f.int(&mut self.attacker_team);
        f.table(&tables::CTSBomb);
        f.int(&mut self.defuse_sec);
        f.int(&mut self.can_cut_wire);
    }

    /// `CTSBomb::Precache` (0x8c570).
    fn restored(&mut self, _cx: &mut Cx) {
        engine().precache_sound(c"misc/beep2.wav");
    }
}

/// `ts_hack` (`CTSHack`): the stub, with what TS saves.
#[derive(Default)]
pub struct Hack {
    /// `attacker_team` (`CTSObjective` `+0x68`).
    attacker_team: i32,
    /// `hacksec` (`+0x7c`): `hackingtime`.
    hack_sec: i32,
    /// `Spawn` (0x8cccc) sets `m_pfnThink` (`WaitThink`).
    funcs: BaseFuncs,
}

impl Entity for Hack {
    /// `CTSHack::KeyValue` (0x8d1c8), then `CTSObjective::KeyValue`.
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"hackingtime" => self.hack_sec = kv_i(value),
            _ => objective_key(&mut self.attacker_team, key, value),
        }
        stub_key_value()
    }

    fn spawn(&mut self, cx: &mut Cx) {
        stub_spawn(cx);
        self.funcs.think = true;
    }

    /// `CTSHack::Save` / `Restore`: `"CTSObjective"` then `"CTSHack"`.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut Fields) {
        super::save_base(cx, f, &mut self.funcs);
        f.table(&tables::CTSObjective);
        f.int(&mut self.attacker_team);
        f.table(&tables::CTSHack);
        f.int(&mut self.hack_sec);
    }

    // `CTSHack::Precache` (0x8cdb4) is empty.
}

/// `ts_teamescape` (`CTSTeamEscape`): the stub, with what TS saves.
#[derive(Default)]
pub struct TeamEscape {
    /// `attacker_team` (`CTSObjective` `+0x68`).
    attacker_team: i32,
    /// `escape_percentage` (`+0xf4`): `escapepercentage`.
    escape_percentage: i32,
    /// `Spawn` (0x8d390) sets `m_pfnThink` (`WaitThink`) and `m_pfnTouch` (`WaitTouch`).
    funcs: BaseFuncs,
}

impl Entity for TeamEscape {
    /// `CTSTeamEscape::KeyValue` (0x8d700), then `CTSObjective::KeyValue`.
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"escapepercentage" => self.escape_percentage = kv_i(value),
            _ => objective_key(&mut self.attacker_team, key, value),
        }
        stub_key_value()
    }

    fn spawn(&mut self, cx: &mut Cx) {
        stub_spawn(cx);
        self.funcs.think = true;
        self.funcs.touch = true;
    }

    /// `CTSTeamEscape::Save` / `Restore`: `"CTSObjective"` then `"CTSTeamEscape"`.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut Fields) {
        super::save_base(cx, f, &mut self.funcs);
        f.table(&tables::CTSObjective);
        f.int(&mut self.attacker_team);
        f.table(&tables::CTSTeamEscape);
        f.int(&mut self.escape_percentage);
    }

    // `CTSTeamEscape::Precache` (0x8d49c) is empty.
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
