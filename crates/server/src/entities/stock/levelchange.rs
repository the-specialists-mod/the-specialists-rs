//! Single-player level flow and player.cpp point entities that TS still links (#95):
//! `trigger_changelevel` (+ the `fireanddie` it creates), `trigger_transition`,
//! `player_loadsaved`, `player_weaponstrip`.
//!
//! Most of this is single-player code. TS keeps it, gated on the rules it installed: with
//! `deathmatch` 0 `InstallGameRules` (0xc6050) builds `CHalfLifeRules`, whose `IsDeathmatch()` is
//! 0, so a changelevel, the strip-player-1 fallback and the reload run there; under the
//! multiplayer rules they don't.

use super::base::*;
use super::explode::{FFADE_OUT, screen_fade_all};
use super::triggers::{init_trigger, rules_deathmatch, show_message};
use crate::entities::remove;
use crate::world::{self, Cx, Entity, UseType};
use goldsrc::edict::EdictPtr;
use goldsrc::ext::entities::{EHandle, is_player};
use goldsrc::ext::saverestore::{FENTTABLE_GLOBAL, FENTTABLE_MOVEABLE, LevelConnection, MAX_LEVEL_CONNECTIONS};
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::ffi::{CStr, CString};

pub const SF_CHANGELEVEL_USEONLY: i32 = 2;

/// `cchMapNameMost`: TS's map and landmark buffers are `char[32]`.
const MAP_NAME_MOST: usize = 32;

/// `CBaseEntity::Intersects` (0xac1e4): the absolute boxes overlap (touching counts).
fn intersects(a: &entvars_t, b: &entvars_t) -> bool {
    (0..3).all(|i| b.absmin[i] <= a.absmax[i] && a.absmin[i] <= b.absmax[i])
}

/// `trigger_changelevel` (`CChangeLevel`, 0xe7da0..0xe865c).
#[derive(Default)]
pub struct ChangeLevel {
    t: Toggle<Self>,
    /// `m_szMapName` (+0x104).
    map: CString,
    /// `m_szLandmarkName` (+0x124).
    landmark: CString,
    /// `m_changeTarget` (+0x144) and `m_changeTargetDelay` (+0x148).
    change_target: string_t,
    change_delay: f32,
    /// `SetUse(UseChangeLevel)`: only with a targetname.
    usable: bool,
}

impl Mover for ChangeLevel {
    fn toggle(&mut self) -> &mut Toggle<Self> {
        &mut self.t
    }
}

/// A `map` / `landmark` value: TS logs names of 32+ characters at `at_error` and `strcpy`s them
/// anyway, overflowing its 32-byte field into the next one. The port keeps the whole name.
fn level_name(kind: &str, value: &CStr) -> CString {
    if value.to_bytes().len() > MAP_NAME_MOST - 1 {
        engine().alert(ALERT_TYPE::at_error, &format!("{kind} name '{}' too long (32 chars)\n", value.to_string_lossy()));
    }
    value.to_owned()
}

/// `CChangeLevel::FindLandmark` (0xe8078): the first `info_landmark` named `name`.
fn find_landmark(name: &CStr) -> *mut edict_t {
    let eng = engine();
    let mut e = eng.find_by_string(std::ptr::null_mut(), c"targetname", name);
    while !e.is_null() {
        if s(e.v().classname) == c"info_landmark" {
            return e;
        }
        e = eng.find_by_string(e, c"targetname", name);
    }
    eng.alert(ALERT_TYPE::at_error, &format!("Can't find landmark {}\n", name.to_string_lossy()));
    std::ptr::null_mut()
}

/// `CChangeLevel::InTransitionVolume` (0xe865c): true for an entity with `FCAP_FORCE_TRANSITION`,
/// else unless `trigger_transition`s named `volume` exist and none of them overlaps the entity
/// (or, for a `MOVETYPE_FOLLOW` entity, what it follows).
fn in_transition_volume(e: *mut edict_t, volume: &CStr) -> bool {
    let eng = engine();
    if object_caps_of(e).is_some_and(|c| c & FCAP_FORCE_TRANSITION != 0) {
        return true;
    }
    let v = e.v();
    // TS takes the aiment's object and crashes in `Intersects` when it has none; the port reads
    // the aiment's entvars.
    let e = if v.movetype == MOVETYPE_FOLLOW as i32 && !v.aiment.is_null() { v.aiment } else { e };
    let mut in_volume = true;
    let mut t = eng.find_by_string(std::ptr::null_mut(), c"targetname", volume);
    while !t.is_null() {
        if t.has_private_data() && s(t.v().classname) == c"trigger_transition" {
            if intersects(t.v(), e.v()) {
                return true;
            }
            in_volume = false;
        }
        t = eng.find_by_string(t, c"targetname", volume);
    }
    in_volume
}

impl ChangeLevel {
    /// `CChangeLevel::TouchChangeLevel` (0xe83e4): classname `player` only.
    fn touch_change_level(&mut self, cx: &mut Cx, other: *mut edict_t) {
        if s(other.v().classname) == c"player" {
            self.change_level_now(cx, other);
        }
    }

    /// `CChangeLevel::ChangeLevelNow` (0xe8180). Once per server frame (`dmgtime`), and only
    /// without the multiplayer rules.
    fn change_level_now(&mut self, cx: &mut Cx, activator: *mut edict_t) {
        if rules_deathmatch() {
            return;
        }
        let eng = engine();
        let now = eng.time();
        if now == cx.pev().dmgtime {
            return;
        }
        cx.pev().dmgtime = now;
        // `INDEXENT(1)`, else the world. TS then uses the edict's object and crashes when it has
        // none (an empty slot 1); the port uses the edict's entvars either way.
        let mut player = eng.entity_of_index(1);
        if player.is_null() {
            player = eng.entity_of_index(0);
        }
        if !in_transition_volume(player, &self.landmark) {
            eng.alert(
                ALERT_TYPE::at_aiconsole,
                &format!("Player isn't in the transition volume {}, aborting\n", self.landmark.to_string_lossy()),
            );
            return;
        }
        if self.change_target != 0 {
            spawn_fire_and_die(self.change_target, self.change_delay, player.v().origin);
        }
        let me = cx.edict;
        self.t.activator = EHandle::new(activator);
        self.t.use_targets(me, activator, UseType::Toggle, 0.0);
        // `st_szNextMap` / `st_szNextSpot`: `char[32]` statics in TS (a longer name overflows
        // them); the spot stays empty when the landmark isn't found.
        let landmark = find_landmark(&self.landmark);
        let spot: &CStr = if landmark.is_null() {
            c""
        } else {
            eng.set_landmark_offset(landmark.v().origin);
            &self.landmark
        };
        eng.alert(ALERT_TYPE::at_console, &format!("CHANGE LEVEL: {} {}\n", self.map.to_string_lossy(), spot.to_string_lossy()));
        eng.change_level_landmark(&self.map, spot);
    }
}

impl Entity for ChangeLevel {
    /// `CChangeLevel::KeyValue` (0xe7da0), then `CBaseTrigger::KeyValue` (0xe68bc).
    fn key_value(&mut self, cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"map" => self.map = level_name("Map", value),
            b"landmark" => self.landmark = level_name("Landmark", value),
            b"changetarget" => self.change_target = alloc(value),
            b"changedelay" => self.change_delay = kv_f(value),
            b"damage" => cx.pev().dmg = kv_f(value),
            // `m_cTriggersLeft` / `m_bitsDamageInflict`: stored, never read by this class.
            b"count" | b"damagetype" => {}
            _ => return self.t.key_value(key, value),
        }
        true
    }

    /// `CChangeLevel::Spawn` (0xe7f30).
    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        if self.map.is_empty() {
            eng.alert(ALERT_TYPE::at_console, "a trigger_changelevel doesn't have a map");
        }
        if self.landmark.is_empty() {
            eng.alert(ALERT_TYPE::at_console, &format!("trigger_changelevel to {} doesn't have a landmark", self.map.to_string_lossy()));
        }
        self.usable = cx.pev().targetname != 0;
        init_trigger(cx);
        if cx.pev().spawnflags & SF_CHANGELEVEL_USEONLY == 0 {
            self.t.touch = Some(Self::touch_change_level);
        }
    }

    fn touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        run_touch(self, cx, other);
    }

    /// `CChangeLevel::UseChangeLevel` (0xe8158).
    fn use_(&mut self, cx: &mut Cx, activator: *mut edict_t, _caller: *mut edict_t, _kind: UseType, _value: f32) {
        if self.usable {
            self.change_level_now(cx, activator);
        }
    }
}

/// `fireanddie` (`CFireAndDie`, 0xe7c58..0xe7cac): fires `target` (`USE_TOGGLE`, itself as
/// activator) `m_flDelay` after it is restored on the next level, then removes itself. Only
/// `Precache` (run on restore) schedules the think, so on the level that creates it it does
/// nothing; the port has no save/restore, so it never fires here either. A map-placed one
/// (`delay` / `killtarget` keys, `CBaseDelay::KeyValue`) fires only if the map sets its
/// `nextthink`.
#[derive(Default)]
pub struct FireAndDie {
    delay: Delay,
}

/// The `GetClassPtr<CFireAndDie>` + `DispatchSpawn` part of `ChangeLevelNow`.
fn spawn_fire_and_die(target: string_t, delay: f32, origin: [f32; 3]) {
    let e = engine().create_entity();
    if e.is_null() {
        return;
    }
    world::create::<FireAndDie>(e.v());
    world::with::<FireAndDie, _>(e, |f| f.delay.delay = delay);
    let v = e.v();
    v.target = target;
    v.origin = origin;
    crate::entities::dispatch_spawn(e);
}

impl Entity for FireAndDie {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        self.delay.key_value(key, value)
    }

    /// `CFireAndDie::Spawn` (0xe7c58): only the classname (`Precache` 0xe7c84 would set
    /// `nextthink = time + m_flDelay`).
    fn spawn(&mut self, cx: &mut Cx) {
        cx.pev().classname = engine().alloc_string(c"fireanddie");
    }

    /// `CFireAndDie::Think` (0xe7cac).
    fn think(&mut self, cx: &mut Cx) {
        let me = cx.edict;
        self.delay.use_targets(me, me, UseType::Toggle, 0.0);
        remove(me);
    }
}

/// `trigger_transition` (`CTriggerVolume::Spawn` 0xe7bc0): the brush only sets the box
/// (`SetModel`), then drops its model; `InTransitionVolume` tests against it. Keys go to
/// `CBaseEntity::KeyValue` (none handled).
#[derive(Default)]
pub struct TriggerVolume;

impl Entity for TriggerVolume {
    fn spawn(&mut self, cx: &mut Cx) {
        let v = cx.pev();
        v.solid = SOLID_NOT as i32;
        v.movetype = MOVETYPE_NONE as i32;
        set_own_model(cx);
        let v = cx.pev();
        v.model = 0;
        v.modelindex = 0;
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RevertThink {
    Message,
    Load,
}

/// `player_loadsaved` (`CRevertSaved`, 0xddd0c..0xddf38): fades every client out, shows
/// `message` after `messagetime`, then `reload`s the last save `loadtime` after the use (with
/// `deathmatch` 0 only).
#[derive(Default)]
pub struct RevertSaved {
    /// `m_messageTime` (+0x64) and `m_loadTime` (+0x68).
    message_time: f32,
    load_time: f32,
    think: Option<RevertThink>,
}

impl RevertSaved {
    /// `CRevertSaved::MessageThink` (0xddeb8): `UTIL_ShowMessageAll` (an unset message shows
    /// as an empty string).
    fn message_think(&mut self, cx: &mut Cx) {
        let eng = engine();
        let message = s(cx.pev().message);
        for i in 1..=eng.max_clients() {
            show_message(message, eng.player(i));
        }
        let wait = self.load_time - self.message_time;
        if wait <= 0.0 {
            Self::load_think();
        } else {
            cx.pev().nextthink = wait + eng.time();
            self.think = Some(RevertThink::Load);
        }
    }

    /// `CRevertSaved::LoadThink` (0xddf38): `gpGlobals->deathmatch`, not the rules.
    fn load_think() {
        let eng = engine();
        if eng.globals().deathmatch == 0.0 {
            eng.server_command("reload\n");
        }
    }
}

impl Entity for RevertSaved {
    /// `CRevertSaved::KeyValue` (0xddd0c).
    fn key_value(&mut self, cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"duration" => cx.pev().dmg_take = kv_f(value),
            b"holdtime" => cx.pev().dmg_save = kv_f(value),
            b"messagetime" => self.message_time = kv_f(value),
            b"loadtime" => self.load_time = kv_f(value),
            _ => return false,
        }
        true
    }

    /// `CPointEntity::Spawn`.
    fn spawn(&mut self, cx: &mut Cx) {
        cx.pev().solid = SOLID_NOT as i32;
    }

    fn think(&mut self, cx: &mut Cx) {
        match self.think {
            Some(RevertThink::Message) => self.message_think(cx),
            Some(RevertThink::Load) => Self::load_think(),
            None => {}
        }
    }

    /// `CRevertSaved::Use` (0xdde24): `UTIL_ScreenFadeAll(rendercolor, duration, holdtime,
    /// (int)renderamt, FFADE_OUT)`, then `MessageThink` after `messagetime`.
    fn use_(&mut self, cx: &mut Cx, _a: *mut edict_t, _c: *mut edict_t, _k: UseType, _v: f32) {
        let v = cx.pev();
        let (color, duration, hold, alpha) = (v.rendercolor, v.dmg_take, v.dmg_save, v.renderamt as i32);
        screen_fade_all(color, duration, hold, alpha, FFADE_OUT);
        cx.pev().nextthink = self.message_time + engine().time();
        self.think = Some(RevertThink::Message);
    }
}

/// `player_weaponstrip` (`CStripWeapons::Use` 0xddbb0): `RemoveAllItems(FALSE)` on a player
/// activator, or (only without the multiplayer rules) on the slot-1 player.
#[derive(Default)]
pub struct StripWeapons;

/// Whom `CStripWeapons::Use` strips: a player activator, else slot 1 when `IsDeathmatch()` is
/// false, else no one.
fn strip_target(activator_is_player: bool, deathmatch: bool) -> Option<bool> {
    if activator_is_player {
        Some(true)
    } else if !deathmatch {
        Some(false)
    } else {
        None
    }
}

impl Entity for StripWeapons {
    /// `CPointEntity::Spawn`.
    fn spawn(&mut self, cx: &mut Cx) {
        cx.pev().solid = SOLID_NOT as i32;
    }

    fn use_(&mut self, _cx: &mut Cx, activator: *mut edict_t, _c: *mut edict_t, _k: UseType, _v: f32) {
        let eng = engine();
        let player = match strip_target(is_player(activator), rules_deathmatch()) {
            Some(true) => activator,
            // `INDEXENT(1)`, else the world edict. TS calls `CBasePlayer::RemoveAllItems` on
            // whatever object that edict holds (the world's `CWorld` when slot 1 has no edict,
            // writing past it); the port only strips a `Player` (`with_player` below).
            Some(false) => {
                let e = eng.entity_of_index(1);
                if e.is_null() { eng.entity_of_index(0) } else { e }
            }
            None => return,
        };
        if player.is_null() {
            return;
        }
        // `RemoveAllItems(FALSE)` inside the `Use`, as in TS: a player whose +use fired us lent
        // itself back (`player_use`); one held by a dispatch that didn't lend it is stripped once
        // that returns.
        world::run_now_or_defer(player, move || {
            crate::player::with_player(player, crate::weapons::remove_all_items_keep_suit);
        });
    }
}

// ---- ParmsChangeLevel: the transition list -----------------------------------------------------------

/// SDK `FCAP_*` bits `ChangeList` reads.
const FCAP_ACROSS_TRANSITION: u32 = 0x0000_0002;
const FCAP_FORCE_TRANSITION: u32 = 0x0000_0080;
const FCAP_DONT_SAVE: u32 = 0x8000_0000;

/// TS's `ObjectCaps()` (vtable slot 12) for an entity's class, by `pev->classname`: the classes
/// every `LINK_ENTITY_TO_CLASS` name of `ts_i386.so` builds (and the names TS gives the objects it
/// creates itself: `gib`, `plat_trigger`, `WorldGun`, `AmmoPack`, `grenade`), each with its
/// class's `ObjectCaps`. `None` for a name TS doesn't know: no TS object, as for an edict
/// without private data.
pub fn ts_object_caps(classname: &CStr, v: &entvars_t) -> Option<u32> {
    // CBeam::ObjectCaps (0xbb7cc) / CSprite::ObjectCaps: SF_BEAM_TEMPORARY / SF_SPRITE_TEMPORARY.
    let temporary = if v.spawnflags & 0x8000 != 0 { FCAP_DONT_SAVE } else { 0 };
    Some(match classname.to_bytes() {
        // CBaseEntity::ObjectCaps (0xac5c8)
        b"DelayedUse"
        | b"TSAmmoPack"
        | b"AmmoPack"
        | b"cycler_weapon"
        | b"cycler_wreckage"
        | b"env_beverage"
        | b"env_debris"
        | b"env_explosion"
        | b"env_funnel"
        | b"env_render"
        | b"env_shooter"
        | b"env_spark"
        | b"game_counter"
        | b"game_counter_set"
        | b"game_end"
        | b"game_player_equip"
        | b"game_player_hurt"
        | b"game_player_team"
        | b"game_score"
        | b"game_team_set"
        | b"game_text"
        | b"game_zone_player"
        | b"gibshooter"
        | b"info_null"
        | b"infodecal"
        | b"item_antidote"
        | b"item_battery"
        | b"item_healthkit"
        | b"item_longjump"
        | b"item_security"
        | b"item_sodacan"
        | b"item_suit"
        | b"monster_hevsuit_dead"
        | b"test_effect"
        | b"ts_briefcase"
        | b"ts_customtarget"
        | b"ts_mapglobals"
        | b"ts_model"
        | b"ts_objective_manager"
        | b"ts_objective_ptr"
        | b"ts_playeramount"
        | b"ts_powerup"
        | b"ts_scoregiver"
        | b"ts_sellzone"
        | b"ts_slowmotionpoint"
        | b"ts_timer"
        | b"ts_wingiver"
        | b"weapon_tsgun"
        | b"weaponbox"
        | b"world_items"
        | b"worldspawn"
        | b"xen_ttrigger" => FCAP_ACROSS_TRANSITION,
        // CPointEntity, CBaseTrigger, CBasePlatTrain, CActAnimating, CFuncWall, CFuncTank and the
        // other classes whose ObjectCaps returns 0.
        b"env_blood"
        | b"env_fade"
        | b"env_global"
        | b"env_glow"
        | b"env_message"
        | b"env_shake"
        | b"env_sound"
        | b"info_breakable"
        | b"info_intermission"
        | b"info_landmark"
        | b"info_player_deathmatch"
        | b"info_player_start"
        | b"info_player_team1"
        | b"info_player_team2"
        | b"info_player_team3"
        | b"info_player_team4"
        | b"info_target"
        | b"info_teleport_destination"
        | b"light"
        | b"light_environment"
        | b"light_spot"
        | b"path_corner"
        | b"path_track"
        | b"player_loadsaved"
        | b"player_weaponstrip"
        | b"target_cdaudio"
        | b"trigger_transition"
        | b"ts_emitter"
        | b"ts_fog"
        | b"ts_particle"
        | b"ts_wind"
        | b"xen_hull"
        | b"func_ladder"
        | b"trigger"
        | b"trigger_autosave"
        | b"trigger_cdaudio"
        | b"trigger_changelevel"
        | b"trigger_counter"
        | b"trigger_endsection"
        | b"trigger_gravity"
        | b"trigger_hurt"
        | b"trigger_monsterjump"
        | b"trigger_multiple"
        | b"trigger_once"
        | b"trigger_push"
        | b"trigger_teleport"
        | b"func_plat"
        | b"func_platrot"
        | b"func_trackautochange"
        | b"func_trackchange"
        | b"func_train"
        | b"xen_hair"
        | b"xen_plantlight"
        | b"xen_spore_large"
        | b"xen_spore_medium"
        | b"xen_spore_small"
        | b"xen_tree"
        | b"func_conveyor"
        | b"func_monsterclip"
        | b"func_wall"
        | b"func_wall_toggle"
        | b"func_tank"
        | b"func_tanklaser"
        | b"func_tankmortar"
        | b"func_tankrocket"
        | b"ambient_generic"
        | b"trigger_auto"
        | b"player"
        | b"respawn"
        | b"func_breakable"
        | b"env_bubbles"
        | b"func_friction"
        | b"func_illusionary"
        | b"func_rotating"
        | b"func_traincontrols"
        | b"func_guntarget"
        | b"momentary_door"
        | b"multi_manager"
        | b"func_pendulum"
        | b"speaker"
        | b"ts_dmhill"
        | b"ts_slowmotion"
        | b"ts_trigger"
        | b"trigger_camera"
        | b"trigger_changetarget"
        | b"trigger_relay" => 0,
        // CBaseButton::ObjectCaps: +use only when it can't be shot.
        b"func_button" | b"func_rot_button" => {
            if v.takedamage == 0.0 {
                8
            } else {
                0
            }
        }
        // CBaseDoor::ObjectCaps: SF_DOOR_USE_ONLY gives FCAP_IMPULSE_USE.
        b"func_door" | b"func_door_rotating" | b"func_water" => {
            if v.spawnflags & 0x100 != 0 {
                8
            } else {
                0
            }
        }
        b"beam" | b"env_beam" | b"env_laser" | b"env_lightning" | b"env_sprite" => temporary,
        b"button_target" => {
            if v.spawnflags & 1 != 0 {
                8
            } else {
                0
            }
        }
        b"momentary_rot_button" => {
            if v.spawnflags & 1 != 0 {
                0
            } else {
                0x10
            }
        }
        b"bodyque" | b"gib" | b"spark_shower" | b"soundent" | b"deadcorpse" | b"plat_trigger" => FCAP_DONT_SAVE,
        b"cycler" | b"cycler_prdroid" => 10,
        b"cycler_sprite" => 0x8000_000a,
        b"fireanddie" | b"game_team_master" => 0x82,
        b"func_tankcontrols" => 8,
        b"func_tracktrain" => 0x40,
        b"multisource" => 0x80,
        b"m61_grenade"
        | b"grenade"
        | b"knife"
        | b"shrapnel"
        | b"ts_bomb"
        | b"ts_flag"
        | b"ammo_9mmAR"
        | b"ammo_buckshot"
        | b"ammo_crossbow"
        | b"ammo_gaussclip"
        | b"ammo_rpgclip"
        | b"ts_groundammo"
        | b"ammo_ARgrenades"
        | b"ts_groundweapon"
        | b"weapon_9MMAR"
        | b"weapon_crossbow"
        | b"weapon_egon"
        | b"weapon_gauss"
        | b"weapon_handgrenade"
        | b"weapon_hornetgun"
        | b"weapon_rpg"
        | b"weapon_satchel"
        | b"weapon_shotgun"
        | b"weapon_snark"
        | b"ts_hack"
        | b"ts_teamescape"
        | b"TSWorldGun"
        | b"WorldGun"
        | b"func_pushable"
        | b"func_recharge"
        | b"func_healthcharger" => 0x10,
        _ => return None,
    })
}

/// [`ts_object_caps`] of a live edict with a game object.
fn object_caps_of(e: *mut edict_t) -> Option<u32> {
    if e.is_null() || e.is_free() || !e.has_private_data() {
        return None;
    }
    let v = e.v();
    ts_object_caps(s(v.classname), v)
}

/// `CChangeLevel::AddTransitionToList` (0xe8438): adds `map` / `landmark` unless that map is
/// already listed through the same landmark entity. The names are cut to the 31 bytes the
/// engine's fields hold (TS `strcpy`s them whole).
fn add_transition_to_list(list: &mut Vec<LevelConnection>, map: &CStr, landmark_name: &CStr, landmark: *mut edict_t) -> bool {
    let cut = |n: &CStr| n.to_bytes()[..n.to_bytes().len().min(MAP_NAME_MOST - 1)].to_vec();
    let map = cut(map);
    if landmark.is_null() || list.iter().any(|l| l.pent_landmark == landmark && l.map == map) {
        return false;
    }
    list.push(LevelConnection { map, landmark: cut(landmark_name), pent_landmark: landmark, landmark_origin: landmark.v().origin });
    true
}

/// The `ChangeList` flags of an entity in a landmark's PVS: `FENTTABLE_MOVEABLE` for
/// `FCAP_ACROSS_TRANSITION`, `FENTTABLE_GLOBAL` for a non-dormant entity with a `globalname`;
/// nothing for `FCAP_DONT_SAVE`.
fn transition_flags(caps: u32, globalname: string_t, flags: i32) -> i32 {
    if caps & FCAP_DONT_SAVE != 0 {
        return 0;
    }
    let mut f = 0;
    if caps & FCAP_ACROSS_TRANSITION != 0 {
        f |= FENTTABLE_MOVEABLE;
    }
    // CBaseEntity::IsDormant (0xac2d8): FL_DORMANT.
    if globalname != 0 && flags & FL_DORMANT as i32 == 0 {
        f |= FENTTABLE_GLOBAL;
    }
    f
}

/// `ParmsChangeLevel` (0xaea68) → `BuildChangeList` (0xe8634) → `CChangeLevel::ChangeList`
/// (0xe87c4): while the engine saves the level (`pSaveData` set), list every
/// `trigger_changelevel` whose landmark exists (up to `MAX_LEVEL_CONNECTIONS`) as
/// `levelList[]` / `connectionCount`, and flag in the save table the entities in each
/// landmark's PVS that may cross: `FENTTABLE_MOVEABLE` / `FENTTABLE_GLOBAL` plus the
/// connection's bit, for those inside the transition volume.
///
/// Guards: TS keeps the PVS list in 512-entry arrays and writes past them from the 513th entity
/// (after logging "Too many entities across a transition!"); the port's list grows. A
/// `trigger_changelevel` edict whose object isn't a `ChangeLevel` gets a fresh one from TS's
/// `GetClassPtr` (empty names, so no landmark); the port skips it.
pub fn parms_change_level() {
    let eng = engine();
    let Some(mut save) = eng.save_data() else { return };
    let mut list: Vec<LevelConnection> = Vec::new();
    let mut e = eng.find_by_string(std::ptr::null_mut(), c"classname", c"trigger_changelevel");
    while !e.is_null() {
        let names = world::with::<ChangeLevel, _>(e, |c| (c.map.clone(), c.landmark.clone()));
        if let Some((map, landmark_name)) = names {
            let landmark = find_landmark(&landmark_name);
            if add_transition_to_list(&mut list, &map, &landmark_name, landmark) && list.len() >= MAX_LEVEL_CONNECTIONS {
                break;
            }
        }
        e = eng.find_by_string(e, c"classname", c"trigger_changelevel");
    }
    for (i, l) in list.iter().enumerate() {
        save.set_connection(i, l);
    }
    if save.has_table() {
        for (i, l) in list.iter().enumerate() {
            let mut movers = Vec::new();
            for ent in eng.entities_in_pvs(l.pent_landmark) {
                let Some(caps) = object_caps_of(ent) else { continue };
                let v = ent.v();
                let flags = transition_flags(caps, v.globalname, v.flags);
                if flags != 0 {
                    movers.push((ent, flags));
                    if movers.len() > 512 {
                        eng.alert(ALERT_TYPE::at_error, "Too many entities across a transition!");
                    }
                }
            }
            let volume = CString::new(l.landmark.clone()).unwrap_or_default();
            for (ent, flags) in movers {
                if in_transition_volume(ent, &volume) {
                    let index = save.entity_index(ent);
                    save.entity_flags_set(index, (1 << i) | flags);
                }
            }
        }
    }
    save.set_connection_count(list.len() as i32);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxed(min: [f32; 3], max: [f32; 3]) -> entvars_t {
        entvars_t { absmin: min, absmax: max, ..Default::default() }
    }

    #[test]
    fn intersects_inclusive() {
        let a = boxed([0.0; 3], [10.0; 3]);
        assert!(intersects(&a, &boxed([10.0, 10.0, 10.0], [20.0; 3])));
        assert!(intersects(&a, &boxed([2.0; 3], [3.0; 3])));
        assert!(!intersects(&a, &boxed([10.5, 0.0, 0.0], [20.0; 3])));
        assert!(!intersects(&a, &boxed([0.0, 0.0, -5.0], [10.0, 10.0, -0.1])));
    }

    #[test]
    fn object_caps_by_class() {
        let v = entvars_t::default();
        assert_eq!(ts_object_caps(c"worldspawn", &v), Some(FCAP_ACROSS_TRANSITION));
        assert_eq!(ts_object_caps(c"item_healthkit", &v), Some(FCAP_ACROSS_TRANSITION));
        assert_eq!(ts_object_caps(c"player", &v), Some(0));
        assert_eq!(ts_object_caps(c"func_wall", &v), Some(0));
        assert_eq!(ts_object_caps(c"multisource", &v), Some(FCAP_FORCE_TRANSITION));
        assert_eq!(ts_object_caps(c"fireanddie", &v), Some(0x82));
        assert_eq!(ts_object_caps(c"bodyque", &v), Some(FCAP_DONT_SAVE));
        assert_eq!(ts_object_caps(c"no_such_class", &v), None);
        // State-dependent ones.
        assert_eq!(ts_object_caps(c"func_button", &v), Some(8), "not shootable: +use");
        let shootable = entvars_t { takedamage: 1.0, ..Default::default() };
        assert_eq!(ts_object_caps(c"func_button", &shootable), Some(0));
        let flags = |sf| entvars_t { spawnflags: sf, ..Default::default() };
        assert_eq!(ts_object_caps(c"func_door", &flags(0x100)), Some(8));
        assert_eq!(ts_object_caps(c"env_sprite", &flags(0x8000)), Some(FCAP_DONT_SAVE));
        assert_eq!(ts_object_caps(c"env_beam", &flags(0)), Some(0));
        assert_eq!(ts_object_caps(c"momentary_rot_button", &flags(1)), Some(0));
        assert_eq!(ts_object_caps(c"momentary_rot_button", &flags(0)), Some(0x10));
        assert_eq!(ts_object_caps(c"button_target", &flags(1)), Some(8));
    }

    #[test]
    fn change_list_flags() {
        assert_eq!(transition_flags(FCAP_ACROSS_TRANSITION, 0, 0), FENTTABLE_MOVEABLE);
        assert_eq!(transition_flags(0, 5, 0), FENTTABLE_GLOBAL);
        assert_eq!(transition_flags(0, 5, FL_DORMANT as i32), 0, "dormant globals stay");
        assert_eq!(transition_flags(FCAP_ACROSS_TRANSITION, 5, 0), FENTTABLE_MOVEABLE | FENTTABLE_GLOBAL);
        assert_eq!(transition_flags(0x8000_000a, 5, 0), 0, "FCAP_DONT_SAVE");
    }

    #[test]
    fn transition_list_dedupes_by_map_and_landmark() {
        let mut a = edict_t::default();
        a.v.origin = [1.0, 2.0, 3.0];
        let mut b = edict_t::default();
        let (pa, pb) = (&mut a as *mut edict_t, &mut b as *mut edict_t);
        let mut list = Vec::new();
        assert!(add_transition_to_list(&mut list, c"ts_a", c"lm", pa));
        assert!(!add_transition_to_list(&mut list, c"ts_a", c"other", pa), "same map, same landmark entity");
        assert!(add_transition_to_list(&mut list, c"ts_b", c"lm", pa));
        assert!(add_transition_to_list(&mut list, c"ts_a", c"lm", pb));
        assert!(!add_transition_to_list(&mut list, c"ts_c", c"lm", std::ptr::null_mut()));
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].landmark_origin, [1.0, 2.0, 3.0]);
        let long = CString::new("m".repeat(40)).unwrap();
        assert!(add_transition_to_list(&mut list, &long, c"lm", pa));
        assert_eq!(list[3].map.len(), 31, "cut to the engine's char[32]");
    }

    #[test]
    fn strip_targets() {
        assert_eq!(strip_target(true, true), Some(true));
        assert_eq!(strip_target(true, false), Some(true));
        assert_eq!(strip_target(false, false), Some(false));
        assert_eq!(strip_target(false, true), None);
    }
}
