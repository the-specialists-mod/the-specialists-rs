//! SDK maprules.cpp entities, same in TS:
//! * `game_text`: a `TE_TEXTMESSAGE` HUD message to the activator or (spawnflag 1) every player,
//!   gated by an optional `master`.
//! * `game_player_equip` (`CGamePlayerEquip` 0xcd494..0xcd718): gives its listed items to a
//!   player on use, or on touch (which only `PlayerSpawn` does). A map that has one replaces the
//!   TS spawn loadout (`CHalfLifeMultiplay::PlayerSpawn` 0x890ac, see [`equip_on_spawn`]).
//! * `game_score` (`CGameScore` 0xcc4d8..0xcc568): points for the activating player or their
//!   teammates, gated by an optional `master`.
//! * `game_end`, `game_player_hurt`, `game_counter`, `game_counter_set`, `game_zone_player`
//!   (0xcc644..0xcd3f4), and TS's team entities `game_team_master`, `game_team_set`,
//!   `game_player_team`.

use super::base::*;
use super::items::SF_NORESPAWN;
use crate::rules::has_player;
use crate::world::{self, Cx, Entity, UseType};
use goldsrc::Global;
use goldsrc::edict::EdictPtr;
use goldsrc::ext::entities::{HudTextParms, te_text_message};
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::CStr;

pub const SF_ENVTEXT_ALLPLAYERS: i32 = 1;

/// `game_text` (`CGameText`, a `CRulePointEntity`): no `CBaseDelay`, so `delay` and `killtarget`
/// are plain unclaimed keys, and `Use` (0xcca2c) fires no targets.
#[derive(Default)]
pub struct GameText {
    parms: HudTextParms,
    master: string_t,
}

/// `UTIL_StringToIntArray` (0xee400) for 4 values: `atoi` at the start of the string and after
/// each single space (so a doubled space starts the next value at the second one, which `atoi`
/// skips: `"1  2"` is 1, 2, 2), the rest 0 once the string ends.
pub fn int_array4(v: &str) -> [i32; 4] {
    let mut out = [0; 4];
    let mut rest = v;
    for o in out.iter_mut() {
        *o = crate::rules::mapcycle::atoi(rest);
        match rest.find(' ') {
            Some(i) => rest = &rest[i + 1..],
            None => break,
        }
    }
    out
}

/// `hudtextparms_t` as TS's 40 bytes (`CGameText::m_textParms` +0x68): x, y, effect, the two
/// colours, fade-in, fade-out, hold, fx time, channel.
fn parms_bytes(p: &HudTextParms) -> [u8; 40] {
    let mut b = [0u8; 40];
    let words = [p.x.to_bits(), p.y.to_bits(), p.effect as u32];
    for (i, w) in words.iter().enumerate() {
        b[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }
    b[12..20].copy_from_slice(&[p.r1, p.g1, p.b1, p.a1, p.r2, p.g2, p.b2, p.a2]);
    let tail = [p.fade_in.to_bits(), p.fade_out.to_bits(), p.hold.to_bits(), p.fx_time.to_bits(), p.channel as u32];
    for (i, w) in tail.iter().enumerate() {
        b[20 + i * 4..24 + i * 4].copy_from_slice(&w.to_le_bytes());
    }
    b
}

fn parms_from_bytes(b: &[u8; 40]) -> HudTextParms {
    let w = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    HudTextParms {
        x: f32::from_bits(w(0)),
        y: f32::from_bits(w(4)),
        effect: w(8) as i32,
        r1: b[12],
        g1: b[13],
        b1: b[14],
        a1: b[15],
        r2: b[16],
        g2: b[17],
        b2: b[18],
        a2: b[19],
        fade_in: f32::from_bits(w(20)),
        fade_out: f32::from_bits(w(24)),
        hold: f32::from_bits(w(28)),
        fx_time: f32::from_bits(w(32)),
        channel: w(36) as i32,
    }
}

impl Entity for GameText {
    /// `CGameText::Save` (0xcc6d0): `CRuleEntity`'s master, then `m_textParms` as 40 bytes.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut crate::save::Fields) {
        save_rule(cx, f, &mut self.master);
        f.table(&crate::save::tables::CGameText);
        let mut b = parms_bytes(&self.parms);
        f.bytes(&mut b);
        self.parms = parms_from_bytes(&b);
    }

    /// `CGameText::KeyValue` (0xcc770), then `CRuleEntity::KeyValue` (`master` only).
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
            _ => return rule_key_value(&mut self.master, key, value),
        }
        true
    }

    /// `CRulePointEntity::Spawn` (0xcc42c).
    fn spawn(&mut self, cx: &mut Cx) {
        rule_point_spawn(cx);
    }

    /// `CGameText::Use` (0xcca2c): the message only; no targets fire.
    fn use_(&mut self, cx: &mut Cx, activator: *mut edict_t, _caller: *mut edict_t, _kind: UseType, _value: f32) {
        if !can_fire_for_activator(self.master, activator) {
            return;
        }
        let msg = s(cx.pev().message);
        if cx.pev().spawnflags & SF_ENVTEXT_ALLPLAYERS != 0 {
            let eng = engine();
            for i in 1..=eng.max_clients() {
                let p = eng.player(i);
                if has_player(p) && p.v().flags & FL_FAKECLIENT as i32 == 0 {
                    te_text_message(p, &self.parms, msg);
                }
            }
        } else if has_player(activator) && activator.v().flags & FL_FAKECLIENT as i32 == 0 {
            te_text_message(activator, &self.parms, msg);
        }
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
        if !has_player(player) {
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
    /// `CRuleEntity::Save` (0xcc2b4): only the master. TS doesn't save `m_weaponNames` /
    /// `m_weaponCount`, so a restored equip gives nothing (the port's list starts empty too).
    /// `Touch` and `Use` are the class's own.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut crate::save::Fields) {
        save_rule(cx, f, &mut self.master);
    }

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
/// respawning), `DispatchSpawn` it and let it touch the player (`DispatchTouch`, which skips an
/// item that removed itself or is `FL_KILLME`). `weapon_tsgun` is TS's gun entity
/// (`playeritem::WorldTsGun`): taken by a player without one, otherwise left lying there.
pub fn give_named_item(player: *mut edict_t, class: &CStr) {
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
    // `DispatchSpawn` (0xaab8c); TS ignores its result here.
    crate::entities::dispatch_spawn(e);
    if !e.is_free() && e.v().flags & FL_KILLME as i32 == 0 {
        crate::ham::touch(e, player);
    }
}

/// `CHalfLifeMultiplay::PlayerSpawn` (0x890ac): when the map has `game_player_equip`s, each one
/// touches the spawning player instead of the TS loadout being bought. Returns false when there
/// are none. The caller (`weapons::on_spawn`) lends the spawning player to its slot meanwhile,
/// so the items reach it inside `PlayerSpawn`, as in TS.
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
    for e in equips {
        if !e.is_free() {
            crate::ham::touch(e, player);
        }
    }
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
    /// `CRuleEntity::Save` (0xcc2b4); the points are `pev->frags`.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut crate::save::Fields) {
        save_rule(cx, f, &mut self.master);
    }

    /// `CGameScore::KeyValue` (0xcc4fc).
    fn key_value(&mut self, cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"points" => cx.pev().frags = kv_i(value) as f32,
            _ => return rule_key_value(&mut self.master, key, value),
        }
        true
    }

    /// `CGameScore::Spawn` (0xcc4d8) → `CRulePointEntity::Spawn` (0xcc42c).
    fn spawn(&mut self, cx: &mut Cx) {
        rule_point_spawn(cx);
    }

    /// `CGameScore::Use` (0xcc568).
    fn use_(&mut self, cx: &mut Cx, activator: *mut edict_t, _caller: *mut edict_t, _kind: UseType, _value: f32) {
        // `IsPlayer()` through the activator (a NULL one crashes TS; here it isn't a player).
        if !can_fire_for_activator(self.master, activator) || !has_player(activator) {
            return;
        }
        let v = cx.pev();
        let (points, allow_negative) = (x87_int(v.frags), v.spawnflags & SF_SCORE_NEGATIVE != 0);
        if v.spawnflags & SF_SCORE_TEAM != 0 {
            add_points_to_team(activator, points, allow_negative);
        } else {
            add_points(activator, points, allow_negative);
        }
    }
}

/// A C `(int)` cast of a float as GCC 2.95 compiles it for TS: `fistp` under a truncating
/// control word, so NaN and anything out of range give the x87 "integer indefinite" 0x80000000.
pub fn x87_int(f: f32) -> i32 {
    if (-2_147_483_648.0..2_147_483_648.0).contains(&f) { f as i32 } else { i32::MIN }
}

/// `CBasePlayer::AddPoints` (0xd9e18): frags, then `ScoreInfo` to everyone.
pub fn add_points(player: *mut edict_t, points: i32, allow_negative: bool) {
    let v = player.v();
    let Some(points) = clamp_points(v.frags, points, allow_negative) else { return };
    v.frags += points as f32;
    // The message reads the deaths from the player object. A player whose own +use fired us lent
    // itself back (`player_use`), so it goes out inside the `Use` as in TS; one held by a dispatch
    // that didn't lend it gets it once that returns.
    world::run_now_or_defer(player, move || crate::rules::scoring::broadcast_score(player));
}

/// `AddPoints`' floor: without `allow_negative` a score can't be taken below zero (and an
/// already negative one isn't lowered further).
fn clamp_points(frags: f32, points: i32, allow_negative: bool) -> Option<i32> {
    if points < 0 && !allow_negative {
        if frags < 0.0 {
            return None;
        }
        if frags < points.wrapping_neg() as f32 {
            return Some(x87_int(-frags));
        }
    }
    Some(points)
}

/// `CBasePlayer::AddPointsToTeam` (0xd9f68): `AddPoints` for every other slot player
/// (`UTIL_PlayerByIndex`) the rules call a teammate (`PlayerRelationship == GR_TEAMMATE`).
pub fn add_points_to_team(player: *mut edict_t, points: i32, allow_negative: bool) {
    let eng = engine();
    let me = eng.index_of(player);
    for i in 1..=eng.max_clients() {
        let other = eng.player(i);
        if !other.is_null() && i != me && crate::rules::teams::same_team(player, other) {
            add_points(other, points, allow_negative);
        }
    }
}

/// `game_end` (`CGameEnd::Use` 0xcc668): `g_pGameRules->EndMultiplayerGame()`, which is
/// `GoToIntermission` for the multiplayer rules (0xd0dbc) and nothing for `CHalfLifeRules`
/// (`deathmatch` 0; 0xc6358).
#[derive(Default)]
pub struct GameEnd {
    master: string_t,
}

impl Entity for GameEnd {
    /// `CRuleEntity::Save` (0xcc2b4).
    fn save_fields(&mut self, cx: &mut Cx, f: &mut crate::save::Fields) {
        save_rule(cx, f, &mut self.master);
    }

    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        rule_key_value(&mut self.master, key, value)
    }

    fn spawn(&mut self, cx: &mut Cx) {
        rule_point_spawn(cx);
    }

    fn use_(&mut self, _cx: &mut Cx, activator: *mut edict_t, _caller: *mut edict_t, _kind: UseType, _value: f32) {
        if !can_fire_for_activator(self.master, activator) {
            return;
        }
        if !crate::rules::state::with(|g| g.single_player) {
            crate::rules::frame::go_to_intermission();
        }
    }
}

pub const SF_PKILL_FIREONCE: i32 = 1;

/// `game_player_hurt` (`CGamePlayerHurt::Use` 0xcd16c): `dmg` damage (`DMG_GENERIC`, the entity
/// as inflictor and attacker) to a player activator, or `-dmg` health when negative; then its
/// targets with the same use type and value; once with flag 1.
#[derive(Default)]
pub struct GamePlayerHurt {
    master: string_t,
}

impl Entity for GamePlayerHurt {
    /// `CRuleEntity::Save` (0xcc2b4).
    fn save_fields(&mut self, cx: &mut Cx, f: &mut crate::save::Fields) {
        save_rule(cx, f, &mut self.master);
    }

    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        rule_key_value(&mut self.master, key, value)
    }

    fn spawn(&mut self, cx: &mut Cx) {
        rule_point_spawn(cx);
    }

    fn use_(&mut self, cx: &mut Cx, activator: *mut edict_t, _caller: *mut edict_t, kind: UseType, value: f32) {
        if !can_fire_for_activator(self.master, activator) {
            return;
        }
        // `IsPlayer()` through the activator (a NULL one crashes TS; here it isn't a player).
        if has_player(activator) {
            let me = cx.edict;
            let dmg = cx.pev().dmg;
            if 0.0 <= dmg {
                // Inside the `Use`, before the targets fire, as in TS: a player whose +use fired
                // us lent itself back (`player_use`); one held by a dispatch that didn't lend it
                // takes the damage once that returns.
                world::run_now_or_defer(activator, move || {
                    crate::damage::apply(activator, &world::DamageInfo::new(me, me, dmg, 0));
                });
            } else {
                crate::player::damage::take_health_edict(activator, -dmg);
            }
        }
        rule_use_targets(cx, activator, kind, value);
        if cx.pev().spawnflags & SF_PKILL_FIREONCE != 0 {
            crate::entities::remove(cx.edict);
        }
    }
}

pub const SF_GAMECOUNT_FIREONCE: i32 = 1;
pub const SF_GAMECOUNT_RESET: i32 = 2;

/// `game_counter` (`CGameCounter` 0xcd24c..0xce510): counts `frags` down on `USE_OFF`, up on
/// `USE_ON`/`USE_TOGGLE`, to `(int)value` on `USE_SET`. When the count reaches `health` (both
/// cast to int) it fires its targets (`USE_TOGGLE`), once with flag 1, and with flag 2 goes back
/// to the spawn count (kept in `dmg`).
#[derive(Default)]
pub struct GameCounter {
    master: string_t,
}

impl Entity for GameCounter {
    /// `CRuleEntity::Save` (0xcc2b4); the count, target and start count are in entvars.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut crate::save::Fields) {
        save_rule(cx, f, &mut self.master);
    }

    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        rule_key_value(&mut self.master, key, value)
    }

    /// `CGameCounter::Spawn` (0xcd24c).
    fn spawn(&mut self, cx: &mut Cx) {
        let v = cx.pev();
        v.dmg = x87_int(v.frags) as f32;
        rule_point_spawn(cx);
    }

    /// `CGameCounter::Use` (0xcd2a4).
    fn use_(&mut self, cx: &mut Cx, activator: *mut edict_t, _caller: *mut edict_t, kind: UseType, value: f32) {
        if !can_fire_for_activator(self.master, activator) {
            return;
        }
        let v = cx.pev();
        v.frags = count_step(v.frags, kind, value);
        if x87_int(v.frags) == x87_int(v.health) {
            rule_use_targets(cx, activator, UseType::Toggle, 0.0);
            if cx.pev().spawnflags & SF_GAMECOUNT_FIREONCE != 0 {
                crate::entities::remove(cx.edict);
            }
            let v = cx.pev();
            if v.spawnflags & SF_GAMECOUNT_RESET != 0 {
                v.frags = v.dmg;
            }
        }
    }
}

/// `CGameCounter::Use`'s count change.
fn count_step(frags: f32, kind: UseType, value: f32) -> f32 {
    match kind {
        UseType::Off => frags - 1.0,
        UseType::On | UseType::Toggle => frags + 1.0,
        UseType::Set => x87_int(value) as f32,
    }
}

pub const SF_GAMECOUNTSET_FIREONCE: i32 = 1;

/// `game_counter_set` (`CGameCounterSet::Use` 0xcd418): `USE_SET` with its `frags` to its
/// targets (a `game_counter` takes that count); once with flag 1.
#[derive(Default)]
pub struct GameCounterSet {
    master: string_t,
}

impl Entity for GameCounterSet {
    /// `CRuleEntity::Save` (0xcc2b4).
    fn save_fields(&mut self, cx: &mut Cx, f: &mut crate::save::Fields) {
        save_rule(cx, f, &mut self.master);
    }

    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        rule_key_value(&mut self.master, key, value)
    }

    fn spawn(&mut self, cx: &mut Cx) {
        rule_point_spawn(cx);
    }

    fn use_(&mut self, cx: &mut Cx, activator: *mut edict_t, _caller: *mut edict_t, _kind: UseType, _value: f32) {
        if !can_fire_for_activator(self.master, activator) {
            return;
        }
        let frags = cx.pev().frags;
        rule_use_targets(cx, activator, UseType::Set, frags);
        if cx.pev().spawnflags & SF_GAMECOUNTSET_FIREONCE != 0 {
            crate::entities::remove(cx.edict);
        }
    }
}

/// `pfnTraceModel` hull numbers (`human_hull`, `head_hull`).
const HUMAN_HULL: i32 = 1;
const HEAD_HULL: i32 = 3;

/// `game_zone_player` (`CGamePlayerZone` 0xcce90..0xcd148): a brush that, on use, sorts every
/// slot player by whether the hull they stand in (the head hull when ducking) starts inside its
/// model. Each player inside fires `intarget`, each one outside `outtarget` (the player as
/// activator, our activator as caller, our use type and value); then `incount`/`outcount` get
/// `USE_SET` with the number of players inside/outside.
#[derive(Default)]
pub struct GamePlayerZone {
    master: string_t,
    in_target: string_t,
    out_target: string_t,
    in_count: string_t,
    out_count: string_t,
}

impl Entity for GamePlayerZone {
    /// `CGamePlayerZone::Save` (0xccdf0): the master, then the four target names.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut crate::save::Fields) {
        save_rule(cx, f, &mut self.master);
        f.table(&crate::save::tables::CGamePlayerZone);
        f.string(&mut self.in_target);
        f.string(&mut self.out_target);
        f.string(&mut self.in_count);
        f.string(&mut self.out_count);
    }

    /// `CGamePlayerZone::KeyValue` (0xcce90).
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        let field = match key.to_bytes() {
            b"intarget" => &mut self.in_target,
            b"outtarget" => &mut self.out_target,
            b"incount" => &mut self.in_count,
            b"outcount" => &mut self.out_count,
            _ => return rule_key_value(&mut self.master, key, value),
        };
        *field = alloc(value);
        true
    }

    /// `CRuleBrushEntity::Spawn` (0xcc468): the brush model, then `CRuleEntity::Spawn` (0xcc354).
    fn spawn(&mut self, cx: &mut Cx) {
        let model = cx.pev().model;
        engine().set_model(cx.edict, s(model));
        let v = cx.pev();
        v.solid = SOLID_NOT as i32;
        v.movetype = MOVETYPE_NONE as i32;
        v.effects = EF_NODRAW as i32;
    }

    /// `CGamePlayerZone::Use` (0xccfa0).
    fn use_(&mut self, cx: &mut Cx, activator: *mut edict_t, _caller: *mut edict_t, kind: UseType, value: f32) {
        if !can_fire_for_activator(self.master, activator) {
            return;
        }
        let eng = engine();
        let me = cx.edict;
        let (mut inside, mut outside) = (0, 0);
        // `UTIL_PlayerByIndex`, looked up as the loop goes (a fired target can change the slots).
        for i in 1..=eng.max_clients() {
            let p = eng.player(i);
            if p.is_null() {
                continue;
            }
            let v = p.v();
            let hull = if v.flags & FL_DUCKING as i32 != 0 { HEAD_HULL } else { HUMAN_HULL };
            let org = v.origin;
            let tr = eng.trace_model(org, org, hull, me);
            let target = if tr.fStartSolid != 0 {
                inside += 1;
                self.in_target
            } else {
                outside += 1;
                self.out_target
            };
            if target != 0 {
                crate::entities::fire_targets(s(target), p, activator, kind, value);
            }
        }
        if self.in_count != 0 {
            crate::entities::fire_targets(s(self.in_count), activator, me, UseType::Set, inside as f32);
        }
        if self.out_count != 0 {
            crate::entities::fire_targets(s(self.out_count), activator, me, UseType::Set, outside as f32);
        }
    }
}

/// `CRuleEntity::Save` (0xcc2b4): the `CBaseEntity` part (no rule entity sets a member pointer:
/// their `Use`/`Touch` are virtual overrides), then `"CRuleEntity"` (`m_iszMaster` +0x64).
fn save_rule(cx: &mut Cx, f: &mut crate::save::Fields, master: &mut string_t) {
    f.base_funcs(cx, crate::save::BaseFuncs::default());
    f.table(&crate::save::tables::CRuleEntity);
    f.string(master);
}

/// `CRuleEntity::KeyValue` (0xcc380): only `master`.
fn rule_key_value(master: &mut string_t, key: &CStr, value: &CStr) -> bool {
    if key == c"master" {
        *master = alloc(value);
        return true;
    }
    false
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
    if has_player(e) {
        return crate::rules::teams::team_name_of(e);
    }
    if s(e.v().classname) == c"game_team_master" {
        return team_master_id(e);
    }
    String::new()
}

/// `m_teamIndex` of every spawned `game_team_master` (`+0x68`), kept outside the entity so a
/// master can be read while it is itself mid-dispatch (its targets can reach
/// `game_player_team`). Keyed by edict address: each master publishes its own value from `Spawn`
/// (or its restore), and a new map starts empty ([`reset`]), so no entry outlives its map.
static TEAM_MASTERS: Global<RefCell<Option<HashMap<usize, i32>>>> = Global(RefCell::new(None));

/// New map (`CWorld::Precache`): the previous map's masters are gone.
pub fn reset() {
    *TEAM_MASTERS.0.borrow_mut() = None;
}

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
    /// `m_teamIndex` as `KeyValue` set it (0 in TS's zeroed object), published to
    /// [`TEAM_MASTERS`] by `Spawn`, which holds the live value after that.
    team_index: i32,
}

impl Entity for GameTeamMaster {
    /// `CRuleEntity::Save` (0xcc2b4). `m_teamIndex` isn't saved, so a restored master is on
    /// team 0 (TS's object starts zeroed).
    fn save_fields(&mut self, cx: &mut Cx, f: &mut crate::save::Fields) {
        save_rule(cx, f, &mut self.master);
        if f.restoring() {
            set_team_index(cx.edict, 0);
        }
    }

    /// `CGameTeamMaster::KeyValue` (0xccae8).
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"teamindex" => self.team_index = kv_i(value),
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

    /// `CRulePointEntity::Spawn` (0xcc42c); the master's team from its `teamindex` key.
    fn spawn(&mut self, cx: &mut Cx) {
        set_team_index(cx.edict, self.team_index);
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
    /// `CRuleEntity::Save` (0xcc2b4).
    fn save_fields(&mut self, cx: &mut Cx, f: &mut crate::save::Fields) {
        save_rule(cx, f, &mut self.master);
    }

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
    /// `CRuleEntity::Save` (0xcc2b4).
    fn save_fields(&mut self, cx: &mut Cx, f: &mut crate::save::Fields) {
        save_rule(cx, f, &mut self.master);
    }

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
        if has_player(activator)
            && let Some(team) = target_team_name(s(cx.pev().target))
            && crate::rules::state::teamplay()
        {
            let (kill, gib) = (sf & SF_PTEAM_KILL != 0, sf & SF_PTEAM_GIB != 0);
            // Inside the `Use`, as in TS: a player whose +use fired us lent itself back
            // (`player_use`); one that is otherwise mid-dispatch changes once that returns.
            world::run_now_or_defer(activator, move || {
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
        // The floor truncates a fractional score (`(int)-frags`).
        assert_eq!(clamp_points(2.7, -3, false), Some(-2));
        // `-INT_MIN` wraps in TS, so the floor never applies.
        assert_eq!(clamp_points(5.0, i32::MIN, false), Some(i32::MIN));
    }

    #[test]
    fn x87_casts() {
        use super::x87_int;
        assert_eq!(x87_int(2.9), 2);
        assert_eq!(x87_int(-2.9), -2);
        assert_eq!(x87_int(-2_147_483_648.0), i32::MIN);
        assert_eq!(x87_int(2_147_483_648.0), i32::MIN);
        assert_eq!(x87_int(f32::NAN), i32::MIN);
        assert_eq!(x87_int(f32::NEG_INFINITY), i32::MIN);
    }

    #[test]
    fn counter_steps() {
        use super::count_step;
        use crate::world::UseType;
        assert_eq!(count_step(3.0, UseType::Off, 9.0), 2.0);
        assert_eq!(count_step(3.0, UseType::On, 9.0), 4.0);
        assert_eq!(count_step(3.0, UseType::Toggle, 9.0), 4.0);
        // `USE_SET` takes the value cast to int.
        assert_eq!(count_step(3.0, UseType::Set, 7.8), 7.0);
        assert_eq!(count_step(3.0, UseType::Set, -7.8), -7.0);
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
        // `atoi` per value: no exponents, trailing junk ignored.
        assert_eq!(super::int_array4("1e3 12abc 7.9 -4"), [1, 12, 7, -4]);
        // TS steps one character past each space, so a doubled space repeats the next value.
        assert_eq!(super::int_array4("255 128  64"), [255, 128, 64, 64]);
        assert_eq!(super::int_array4(""), [0, 0, 0, 0]);
    }
}
