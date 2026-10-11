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

/// `CChangeLevel::InTransitionVolume` (0xe865c): true unless `trigger_transition`s named
/// `volume` exist and none of them overlaps the entity (or, for a `MOVETYPE_FOLLOW` entity, what
/// it follows). The `FCAP_FORCE_TRANSITION` shortcut is left out: the only caller passes the
/// slot-1 player (`ObjectCaps` 0) or the world (`FCAP_ACROSS_TRANSITION`).
fn in_transition_volume(e: *mut edict_t, volume: &CStr) -> bool {
    let eng = engine();
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
    world::dispatch(e, |ent, cx| ent.spawn(cx));
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
        // The activator may be the player being dispatched (its +use fired us): strip it once its
        // object is back in its slot, in the same frame.
        world::defer(player, move || {
            crate::player::with_player(player, crate::weapons::remove_all_items_keep_suit);
        });
    }
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
    fn strip_targets() {
        assert_eq!(strip_target(true, true), Some(true));
        assert_eq!(strip_target(true, false), Some(true));
        assert_eq!(strip_target(false, false), Some(false));
        assert_eq!(strip_target(false, true), None);
    }
}
