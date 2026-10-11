//! SDK maprules.cpp entities, same in TS:
//! * `game_text`: a `TE_TEXTMESSAGE` HUD message to the activator or (spawnflag 1) every player,
//!   gated by an optional `master`.
//! * `game_player_equip` (`CGamePlayerEquip` 0xcd494..0xcd718): gives its listed items to a
//!   player on use, or on touch (which only `PlayerSpawn` does). A map that has one replaces the
//!   TS spawn loadout (`CHalfLifeMultiplay::PlayerSpawn` 0x890ac, see [`equip_on_spawn`]).
//! * `game_score` (`CGameScore` 0xcc4d8..0xcc568): points for the activating player or their
//!   teammates, gated by an optional `master`.

use super::base::*;
use super::items::SF_NORESPAWN;
use crate::world::{self, Cx, Entity, UseType};
use goldsrc::Global;
use goldsrc::edict::EdictPtr;
use goldsrc::ext::entities::{HudTextParms, is_player, te_text_message};
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::CStr;

pub const SF_ENVTEXT_ALLPLAYERS: i32 = 1;

#[derive(Default)]
pub struct GameText {
    parms: HudTextParms,
    master: string_t,
    delay: Delay,
}

/// `UTIL_StringToIntArray` for up to 4 values (missing ones are 0).
pub fn int_array4(v: &str) -> [i32; 4] {
    let mut out = [0; 4];
    for (o, t) in out.iter_mut().zip(v.split_whitespace()) {
        *o = t.parse::<f32>().map(|f| f as i32).unwrap_or(0);
    }
    out
}

impl Entity for GameText {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        let p = &mut self.parms;
        match key.to_bytes() {
            b"channel" => p.channel = kv_i(value),
            b"x" => p.x = kv_f(value),
            b"y" => p.y = kv_f(value),
            b"effect" => p.effect = kv_i(value),
            b"color" => {
                let c = int_array4(kv_str(value));
                (p.r1, p.g1, p.b1, p.a1) = (c[0] as u8, c[1] as u8, c[2] as u8, c[3] as u8);
            }
            b"color2" => {
                let c = int_array4(kv_str(value));
                (p.r2, p.g2, p.b2, p.a2) = (c[0] as u8, c[1] as u8, c[2] as u8, c[3] as u8);
            }
            b"fadein" => p.fade_in = kv_f(value),
            b"fadeout" => p.fade_out = kv_f(value),
            b"holdtime" => p.hold = kv_f(value),
            b"fxtime" => p.fx_time = kv_f(value),
            b"master" => self.master = alloc(value),
            _ => return self.delay.key_value(key, value),
        }
        true
    }

    fn spawn(&mut self, cx: &mut Cx) {
        let v = cx.pev();
        v.solid = SOLID_NOT as i32;
        v.movetype = MOVETYPE_NONE as i32;
        v.effects = EF_NODRAW as i32;
        v.frame = 0.0;
    }

    fn use_(&mut self, cx: &mut Cx, activator: *mut edict_t, _caller: *mut edict_t, _kind: UseType, _value: f32) {
        if self.master != 0 && !is_master_triggered(self.master, activator) {
            return;
        }
        let msg = s(cx.pev().message);
        if cx.pev().spawnflags & SF_ENVTEXT_ALLPLAYERS != 0 {
            let eng = engine();
            for i in 1..=eng.max_clients() {
                let p = eng.player(i);
                if is_player(p) && p.v().flags & FL_FAKECLIENT as i32 == 0 {
                    te_text_message(p, &self.parms, msg);
                }
            }
        } else if is_player(activator) && activator.v().flags & FL_FAKECLIENT as i32 == 0 {
            te_text_message(activator, &self.parms, msg);
        }
        let me = cx.edict;
        self.delay.use_targets(me, me, UseType::Toggle, 0.0);
    }
}

pub const SF_PLAYEREQUIP_USEONLY: i32 = 1;
/// `MAX_EQUIP`.
const MAX_EQUIP: usize = 32;

#[derive(Default)]
pub struct GamePlayerEquip {
    master: string_t,
    /// Item classname and how many to give (at least 1).
    items: Vec<(string_t, i32)>,
}

/// `UTIL_StripToken`: a repeated key is written `name#n`; the item is the part before `#`.
pub fn strip_token(key: &str) -> &str {
    key.split('#').next().unwrap_or("")
}

impl GamePlayerEquip {
    /// `CGamePlayerEquip::EquipPlayer`.
    fn equip(&self, player: *mut edict_t) {
        if !is_player(player) {
            return;
        }
        for &(item, count) in &self.items {
            for _ in 0..count {
                give_named_item(player, s(item));
            }
        }
    }
}

impl Entity for GamePlayerEquip {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        if key == c"master" {
            self.master = alloc(value);
            return true;
        }
        // Entvars keys never get here, so every other key is an item.
        if self.items.len() >= MAX_EQUIP {
            return false;
        }
        let name: String = strip_token(&key.to_string_lossy()).chars().take(127).collect();
        let item = engine().alloc_string(&goldsrc::util::to_cstring(&name));
        self.items.push((item, kv_i(value).max(1)));
        true
    }

    fn spawn(&mut self, cx: &mut Cx) {
        // CRulePointEntity::Spawn
        let v = cx.pev();
        v.solid = SOLID_NOT as i32;
        v.movetype = MOVETYPE_NONE as i32;
        v.effects = EF_NODRAW as i32;
        v.frame = 0.0;
        v.model = 0;
    }

    /// `CGamePlayerEquip::Touch` (0xcd570): master check, and not with `USEONLY`.
    fn touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        if self.master != 0 && !is_master_triggered(self.master, other) {
            return;
        }
        if cx.pev().spawnflags & SF_PLAYEREQUIP_USEONLY != 0 {
            return;
        }
        self.equip(other);
    }

    /// `CGamePlayerEquip::Use` (0xcd718): no master check.
    fn use_(&mut self, _cx: &mut Cx, activator: *mut edict_t, _caller: *mut edict_t, _kind: UseType, _value: f32) {
        self.equip(activator);
    }
}

/// `CBasePlayer::GiveNamedItem` (0xdb524): create the item at the player's feet (never
/// respawning), spawn it and let it touch the player.
pub fn give_named_item(player: *mut edict_t, class: &CStr) {
    // Port: the TS gun isn't an entity, every player always has it.
    if class == c"weapon_tsgun" {
        return;
    }
    let eng = engine();
    let e = eng.create_named_entity(class);
    if e.is_null() || !e.has_private_data() {
        if !e.is_null() {
            eng.remove_entity(e);
        }
        eng.alert(ALERT_TYPE::at_console, "NULL Ent in GiveNamedItem!\n");
        return;
    }
    let origin = player.v().origin;
    let v = e.v();
    v.origin = origin;
    v.spawnflags |= SF_NORESPAWN;
    world::dispatch(e, |ent, cx| ent.spawn(cx));
    if !e.is_free() && e.v().flags & FL_KILLME as i32 == 0 {
        world::dispatch(e, |ent, cx| ent.touch(cx, player));
    }
}

/// `CHalfLifeMultiplay::PlayerSpawn` (0x890ac): when the map has `game_player_equip`s, each one
/// touches the spawning player instead of the TS loadout being bought. Returns false when there
/// are none. The touches run once the player's own spawn dispatch has returned, so the items can
/// reach the `Player` (TS runs them inside `PlayerSpawn`).
pub fn equip_on_spawn(player: *mut edict_t) -> bool {
    let eng = engine();
    let mut equips = Vec::new();
    let mut it = std::ptr::null_mut();
    loop {
        it = eng.find_by_string(it, c"classname", c"game_player_equip");
        if it.is_null() {
            break;
        }
        equips.push(it);
    }
    if equips.is_empty() {
        return false;
    }
    world::defer(player, move || {
        for e in equips {
            if !e.is_free() {
                world::dispatch(e, |ent, cx| ent.touch(cx, player));
            }
        }
    });
    true
}

pub const SF_SCORE_NEGATIVE: i32 = 1;
pub const SF_SCORE_TEAM: i32 = 2;

/// `game_score` (`CGameScore`, 0xcc4d8..0xcc568): `points` (`frags`) to the activator, or with
/// spawnflag 2 to every teammate of the activator (not the activator). Without spawnflag 1 the
/// points can't take a score below zero.
#[derive(Default)]
pub struct GameScore {
    master: string_t,
}

impl Entity for GameScore {
    fn key_value(&mut self, cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"points" => cx.pev().frags = kv_i(value) as f32,
            // CRuleEntity::KeyValue (0xcc380)
            b"master" => self.master = alloc(value),
            _ => return false,
        }
        true
    }

    /// `CRulePointEntity::Spawn` (0xcc42c).
    fn spawn(&mut self, cx: &mut Cx) {
        let v = cx.pev();
        v.solid = SOLID_NOT as i32;
        v.movetype = MOVETYPE_NONE as i32;
        v.effects = EF_NODRAW as i32;
        v.frame = 0.0;
        v.model = 0;
    }

    /// `CGameScore::Use` (0xcc568).
    fn use_(&mut self, cx: &mut Cx, activator: *mut edict_t, _caller: *mut edict_t, _kind: UseType, _value: f32) {
        // CRuleEntity::CanFireForActivator (0xcc3e4)
        if self.master != 0 && !is_master_triggered(self.master, activator) {
            return;
        }
        if !is_player(activator) {
            return;
        }
        let v = cx.pev();
        let (points, allow_negative) = (v.frags as i32, v.spawnflags & SF_SCORE_NEGATIVE != 0);
        if v.spawnflags & SF_SCORE_TEAM != 0 {
            add_points_to_team(activator, points, allow_negative);
        } else {
            add_points(activator, points, allow_negative);
        }
    }
}

/// `CBasePlayer::AddPoints` (0xd9e18): frags plus `ScoreInfo` to everyone.
pub fn add_points(player: *mut edict_t, points: i32, allow_negative: bool) {
    let v = player.v();
    let Some(points) = clamp_points(v.frags, points, allow_negative) else { return };
    v.frags += points as f32;
    // (When the player itself is mid-dispatch the message waits for the next score update.)
    crate::rules::scoring::broadcast_score(player);
}

/// `AddPoints`' floor: without `allow_negative` a score can't be taken below zero (and an
/// already negative one isn't lowered further).
fn clamp_points(frags: f32, points: i32, allow_negative: bool) -> Option<i32> {
    if points < 0 && !allow_negative {
        if frags < 0.0 {
            return None;
        }
        if frags < -points as f32 {
            return Some((-frags) as i32);
        }
    }
    Some(points)
}

/// `CBasePlayer::AddPointsToTeam` (0xd9f68): `AddPoints` for every other player on the same team.
pub fn add_points_to_team(player: *mut edict_t, points: i32, allow_negative: bool) {
    for other in crate::rules::players() {
        if other != player && crate::rules::teams::same_team(player, other) {
            add_points(other, points, allow_negative);
        }
    }
}

/// `CRulePointEntity::Spawn` (0xcc42c).
fn rule_point_spawn(cx: &mut Cx) {
    let v = cx.pev();
    v.solid = SOLID_NOT as i32;
    v.movetype = MOVETYPE_NONE as i32;
    v.effects = EF_NODRAW as i32;
    v.frame = 0.0;
    v.model = 0;
}

/// `CRuleEntity::CanFireForActivator` (0xcc3e4).
fn can_fire_for_activator(master: string_t, activator: *mut edict_t) -> bool {
    master == 0 || is_master_triggered(master, activator)
}

/// `SUB_UseTargets` of a rule entity (a `CBaseEntity`: no delay or killtarget).
fn rule_use_targets(cx: &mut Cx, activator: *mut edict_t, kind: UseType, value: f32) {
    let target = s(cx.pev().target);
    crate::entities::fire_targets(target, activator, cx.edict, kind, value);
}

/// `TeamID()` of any entity: a player's team name, a `game_team_master`'s team, `""` otherwise.
fn team_id(e: *mut edict_t) -> String {
    if e.is_null() {
        return String::new();
    }
    if is_player(e) || crate::rules::has_player(e) {
        return crate::rules::teams::team_name_of(e);
    }
    if s(e.v().classname) == c"game_team_master" {
        return team_master_id(e);
    }
    String::new()
}

/// `m_teamIndex` of every `game_team_master` (`+0x68`), kept outside the entity so a master can
/// be read while it is itself mid-dispatch (its targets can reach `game_player_team`).
static TEAM_MASTERS: Global<RefCell<Option<HashMap<usize, i32>>>> = Global(RefCell::new(None));

fn team_index_of(e: *mut edict_t) -> i32 {
    TEAM_MASTERS.0.borrow().as_ref().and_then(|m| m.get(&(e as usize)).copied()).unwrap_or(0)
}

fn set_team_index(e: *mut edict_t, i: i32) {
    TEAM_MASTERS.0.borrow_mut().get_or_insert_with(HashMap::new).insert(e as usize, i);
}

/// `CGameTeamMaster::TeamID` (0xccc94): `GetIndexedTeamName(m_teamIndex)`, `""` below 0.
fn team_master_id(e: *mut edict_t) -> String {
    let i = team_index_of(e);
    if i < 0 {
        return String::new();
    }
    if crate::rules::state::teamplay() { crate::rules::state::with(|g| g.indexed_team_name(i)) } else { String::new() }
}

/// `UTIL_TeamsMatch` (0xee2b8): always outside teamplay; otherwise two non-empty names equal
/// ignoring case.
fn teams_match(a: &str, b: &str) -> bool {
    !crate::rules::state::teamplay() || (!a.is_empty() && !b.is_empty() && a.eq_ignore_ascii_case(b))
}

pub const SF_TEAMMASTER_FIREONCE: i32 = 1;
pub const SF_TEAMMASTER_ANYTEAM: i32 = 2;

/// `CGameTeamMaster::TeamMatch` (0xcccd4): any activator when the master has no team and
/// `ANYTEAM` is set, else the activator's team must match the master's.
fn team_match(master: *mut edict_t, activator: *mut edict_t) -> bool {
    if team_index_of(master) < 0 && master.v().spawnflags & SF_TEAMMASTER_ANYTEAM != 0 {
        return true;
    }
    !activator.is_null() && teams_match(&team_id(activator), &team_master_id(master))
}

/// `CGameTeamMaster::IsTriggered` (0xccc6c) for `UTIL_IsMasterTriggered` (`FCAP_MASTER` is in its
/// `ObjectCaps` 0x82): `None` when `e` isn't a `game_team_master`.
pub fn team_master_triggered(e: *mut edict_t, activator: *mut edict_t) -> Option<bool> {
    (s(e.v().classname) == c"game_team_master").then(|| team_match(e, activator))
}

/// `game_team_master` (`CGameTeamMaster` 0xccae8..0xccd40): a master (and relay) that passes only
/// for activators on its team (`teamindex`, -1 = none). `USE_SET` sets its team: the activator's
/// with a value ≥ 0, none with a negative one. Other uses fire its targets with `triggerstate`
/// (0 off, 2 toggle, anything else on) when the activator matches.
#[derive(Default)]
pub struct GameTeamMaster {
    master: string_t,
    trigger_type: i32,
}

impl Entity for GameTeamMaster {
    /// `CGameTeamMaster::KeyValue` (0xccae8).
    fn key_value(&mut self, cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"teamindex" => set_team_index(cx.edict, kv_i(value)),
            b"triggerstate" => {
                self.trigger_type = match kv_i(value) {
                    0 => UseType::Off as i32,
                    2 => UseType::Toggle as i32,
                    _ => UseType::On as i32,
                }
            }
            b"master" => self.master = alloc(value),
            _ => return false,
        }
        true
    }

    fn spawn(&mut self, cx: &mut Cx) {
        // The object starts zeroed in TS: team 0 until a `teamindex` key says otherwise.
        if TEAM_MASTERS.0.borrow().as_ref().is_none_or(|m| !m.contains_key(&(cx.edict as usize))) {
            set_team_index(cx.edict, 0);
        }
        rule_point_spawn(cx);
    }

    /// `CGameTeamMaster::Use` (0xccba8).
    fn use_(&mut self, cx: &mut Cx, activator: *mut edict_t, _caller: *mut edict_t, kind: UseType, value: f32) {
        if !can_fire_for_activator(self.master, activator) {
            return;
        }
        let me = cx.edict;
        if kind == UseType::Set {
            let i = if value < 0.0 {
                -1
            } else {
                let team = team_id(activator);
                if crate::rules::state::teamplay() { crate::rules::state::with(|g| g.tp_team_index(&team)) } else { -1 }
            };
            set_team_index(me, i);
            return;
        }
        if team_match(me, activator) {
            let kind = match self.trigger_type {
                0 => UseType::Off,
                3 => UseType::Toggle,
                _ => UseType::On,
            };
            rule_use_targets(cx, activator, kind, value);
            if cx.pev().spawnflags & SF_TEAMMASTER_FIREONCE != 0 {
                crate::entities::remove(me);
            }
        }
    }
}

pub const SF_TEAMSET_FIREONCE: i32 = 1;
pub const SF_TEAMSET_CLEARTEAM: i32 = 2;

/// `game_team_set` (`CGameTeamSet::Use` 0xccd64): `USE_SET` to its targets (a `game_team_master`
/// takes the activator's team, or none with `CLEARTEAM`).
#[derive(Default)]
pub struct GameTeamSet {
    master: string_t,
}

impl Entity for GameTeamSet {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"master" => self.master = alloc(value),
            _ => return false,
        }
        true
    }

    fn spawn(&mut self, cx: &mut Cx) {
        rule_point_spawn(cx);
    }

    fn use_(&mut self, cx: &mut Cx, activator: *mut edict_t, _caller: *mut edict_t, _kind: UseType, _value: f32) {
        if !can_fire_for_activator(self.master, activator) {
            return;
        }
        let value = if cx.pev().spawnflags & SF_TEAMSET_CLEARTEAM != 0 { -1.0 } else { 0.0 };
        rule_use_targets(cx, activator, UseType::Set, value);
        if cx.pev().spawnflags & SF_TEAMSET_FIREONCE != 0 {
            crate::entities::remove(cx.edict);
        }
    }
}

pub const SF_PTEAM_FIREONCE: i32 = 1;
pub const SF_PTEAM_KILL: i32 = 2;
pub const SF_PTEAM_GIB: i32 = 4;

/// `game_player_team` (`CGamePlayerTeam` 0xcd740..0xce658): moves the activating player to the
/// team of the `game_team_master` it targets, through `g_pGameRules->ChangePlayerTeam(player,
/// team, kill, gib)` (empty outside teamplay).
#[derive(Default)]
pub struct GamePlayerTeam {
    master: string_t,
}

/// `CGamePlayerTeam::TargetTeamName` (0xcd764): the `TeamID` of the first entity named `target`
/// whose classname is exactly `game_team_master`, `None` when there is none.
fn target_team_name(target: &CStr) -> Option<String> {
    let eng = engine();
    let mut it = std::ptr::null_mut();
    loop {
        it = eng.find_by_target_name(it, target);
        if it.is_null() {
            return None;
        }
        if s(it.v().classname) == c"game_team_master" {
            return Some(team_master_id(it));
        }
    }
}

impl Entity for GamePlayerTeam {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"master" => self.master = alloc(value),
            _ => return false,
        }
        true
    }

    fn spawn(&mut self, cx: &mut Cx) {
        rule_point_spawn(cx);
    }

    /// `CGamePlayerTeam::Use` (0xcd7d8).
    fn use_(&mut self, cx: &mut Cx, activator: *mut edict_t, _caller: *mut edict_t, _kind: UseType, _value: f32) {
        if !can_fire_for_activator(self.master, activator) {
            return;
        }
        let sf = cx.pev().spawnflags;
        if crate::rules::has_player(activator)
            && let Some(team) = target_team_name(s(cx.pev().target))
            && crate::rules::state::teamplay()
        {
            let (kill, gib) = (sf & SF_PTEAM_KILL != 0, sf & SF_PTEAM_GIB != 0);
            // The player is usually mid-dispatch (its +use or touch fired us): change its team
            // as soon as that returns.
            world::defer(activator, move || {
                crate::player::with_player(activator, |p, pcx| crate::rules::teams::change_team(p, pcx, &team, kill, gib));
            });
        }
        if sf & SF_PTEAM_FIREONCE != 0 {
            crate::entities::remove(cx.edict);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn score_floor() {
        use super::clamp_points;
        assert_eq!(clamp_points(5.0, 3, false), Some(3));
        assert_eq!(clamp_points(5.0, -3, false), Some(-3));
        assert_eq!(clamp_points(2.0, -3, false), Some(-2));
        assert_eq!(clamp_points(-1.0, -3, false), None);
        assert_eq!(clamp_points(2.0, -3, true), Some(-3));
    }

    #[test]
    fn equip_keys() {
        assert_eq!(super::strip_token("weapon_shotgun#2"), "weapon_shotgun");
        assert_eq!(super::strip_token("item_battery"), "item_battery");
    }

    #[test]
    fn colors() {
        assert_eq!(super::int_array4("255 128 0"), [255, 128, 0, 0]);
        assert_eq!(super::int_array4("1 2 3 4 5"), [1, 2, 3, 4]);
    }
}
