//! The plugin API's entity and game state (version 3, issue #104): what a game entity is, without
//! a plugin guessing from classnames or calling C++ virtuals through private data (which the
//! port's entities don't have, design.md §4.2), and the rules' mode and rounds. Version 4 (issue
//! #117) adds the team scores, the objective list and creating an entity from its keys.

use crate::entities::stock::base::is_master_triggered_name;
use crate::entities::stock::playeritem::WorldTsGun;
use crate::entities::ts::{ctb, ground::GroundWeapon, powerup::PowerUp};
use crate::rules::{self, state};
use crate::weapons::worldgun::WorldGun;
use crate::world;
use goldsrc::edict::EdictPtr;
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::any::TypeId;
use std::ffi::{CStr, c_int};
use ts_plugin_api::{EntityInfo, GameState, INVALID, MAX_TEAMS, NOT_PLAYER, OBJECTIVE_NAME_LEN, OK, ObjectiveInfo, entity, mode, round};

/// A game entity at `index`: in range, in use, with the game's private data.
fn game_entity(index: c_int) -> Option<*mut edict_t> {
    let eng = engine();
    if index < 0 || index >= eng.globals().maxEntities {
        return None;
    }
    let e = eng.entity_of_index(index);
    (!e.is_null() && !e.is_free() && e.has_private_data()).then_some(e)
}

fn index(e: Option<*mut edict_t>) -> c_int {
    e.filter(|e| !e.is_null()).map_or(0, |e| engine().index_of(e))
}

/// A player's rules team as the API counts it: 1.. in teamplay, 0 for none.
pub fn player_team(e: *mut edict_t) -> c_int {
    if !state::teamplay() {
        return 0;
    }
    let name = rules::teams::team_name_of(e);
    state::with(|g| g.team_index(&name)) + 1
}

/// A 0-based TS team (-1 any) as the API counts it.
fn team(t: i32) -> c_int {
    if t < 0 { 0 } else { t + 1 }
}

/// `PluginApi::entity_info`.
pub fn entity_info(ent: c_int) -> Result<EntityInfo, c_int> {
    let e = game_entity(ent).ok_or(NOT_PLAYER)?;
    let visible = e.v().effects & EF_NODRAW as i32 == 0;
    let mut info = EntityInfo { available: visible as c_int, ..EntityInfo::default() };
    if rules::is_player_entity(e) {
        info.kind = entity::PLAYER;
        info.team = player_team(e);
        info.available = (rules::alive(e.v()) && e.v().iuser1 == 0) as c_int;
        return Ok(info);
    }
    let thrower = crate::weapons::projectiles::thrower_of(e);
    let ty = world::type_of(e);
    if ty == Some(TypeId::of::<GroundWeapon>()) {
        info.kind = entity::GROUND_WEAPON;
        info.weapon = world::with_lent::<GroundWeapon, _>(e, |g| g.id).unwrap_or(0);
    } else if ty == Some(TypeId::of::<PowerUp>()) {
        info.kind = entity::POWERUP;
        info.powerup = world::with_lent::<PowerUp, _>(e, |p| p.kind as c_int).unwrap_or(0);
    } else if ty == Some(TypeId::of::<WorldGun>()) {
        info.kind = entity::GUN;
        let (id, dropper) = world::with_lent::<WorldGun, _>(e, |g| (g.id, g.dropper)).unwrap_or((0, None));
        info.weapon = id as c_int;
        info.owner = index(thrower.or(dropper));
    } else if ty == Some(TypeId::of::<WorldTsGun>()) {
        info.kind = entity::GUN;
        let o = e.v().owner;
        if rules::is_player(o) {
            info.owner = engine().index_of(o);
            // The gun in hand: the context of the `Item_*` / `Weapon_*` hooks on `weapon_tsgun`.
            info.weapon = super::player_info(info.owner).map_or(0, |p| p.weapon);
        }
    } else if ty == Some(TypeId::of::<ctb::Flag>()) {
        info.kind = entity::CTB_FLAG;
        info.team = team(world::with_lent::<ctb::Flag, _>(e, |f| f.team()).unwrap_or(-1));
    } else if ty == Some(TypeId::of::<ctb::Briefcase>()) {
        info.kind = entity::CTB_BASE;
        let (t, active) = world::with_lent::<ctb::Briefcase, _>(e, |b| b.capture_team()).unwrap_or((-1, false));
        info.team = team(t);
        info.available = active as c_int;
    } else if ty == Some(TypeId::of::<ctb::WinGiver<false>>()) || ty == Some(TypeId::of::<ctb::WinGiver<true>>()) {
        info.kind = entity::OBJECTIVE;
    } else if thrower.is_some() {
        info.kind = entity::PROJECTILE;
        info.owner = index(thrower);
    } else {
        info.owner = super::projectile_owner(index_of(e)).max(0);
    }
    Ok(info)
}

fn index_of(e: *mut edict_t) -> c_int {
    engine().index_of(e)
}

/// `PluginApi::master_triggered`: `UTIL_IsMasterTriggered` by name, for `activator` (an index; 0
/// or anything that isn't a game entity is no activator).
pub fn master_triggered(master: &CStr, activator: c_int) -> c_int {
    let a = if activator > 0 { game_entity(activator).unwrap_or(std::ptr::null_mut()) } else { std::ptr::null_mut() };
    is_master_triggered_name(master, a) as c_int
}

/// The API's round state from the rules': over while a restart is pending, running while a round
/// mode (last man standing, round TDM) has an end time.
pub fn round_of(round_mode: bool, round_end: f32, round_restart: f32) -> c_int {
    if round_restart > 0.0 {
        round::OVER
    } else if round_mode && round_end > 0.0 {
        round::RUNNING
    } else {
        round::NONE
    }
}

/// `PluginApi::game_state`.
pub fn game_state() -> GameState {
    let eng = engine();
    let ctb = !eng.find_by_string(std::ptr::null_mut(), c"classname", c"ts_briefcase").is_null();
    let mut s = state::with(|g| {
        let m = match g.mode {
            state::Mode::Dm => mode::DEATHMATCH,
            state::Mode::TeamDm => mode::TEAM_DEATHMATCH,
            state::Mode::RoundTdm => mode::ROUND_TEAM_DEATHMATCH,
            state::Mode::Objective => mode::OBJECTIVE,
        };
        let round_mode = g.lms || g.mode == state::Mode::RoundTdm;
        GameState {
            mode: m,
            teamplay: g.teamplay as c_int,
            last_man_standing: g.lms as c_int,
            the_one: g.the_one as c_int,
            ctb: 0,
            teams: if g.teamplay { g.team_names().len() as c_int } else { 0 },
            round: round_of(round_mode, g.round_end, g.round_restart),
            round_ends_at: if round_mode { g.round_end } else { 0.0 },
            round_restart_at: g.round_restart.max(0.0),
            game_over: g.game_over as c_int,
            time: 0.0,
            max_clients: 0,
            team_scores: team_scores(g.teams.iter().map(|t| (!t.name.is_empty(), t.score))),
        }
    });
    s.ctb = ctb as c_int;
    s.time = eng.time();
    s.max_clients = eng.max_clients();
    s
}

/// `GameState::team_scores`: the `TSGameRules` teams' scores, what `TeamScore` sends (`TeamWin`
/// 0x78a08, `CaptureTouch` 0x8db9c, `RecountTeams` 0xe5608); 0 for a team without a name.
fn team_scores(teams: impl Iterator<Item = (bool, i32)>) -> [c_int; MAX_TEAMS] {
    let mut out = [0; MAX_TEAMS];
    for (slot, (named, score)) in out.iter_mut().zip(teams) {
        *slot = if named { score } else { 0 };
    }
    out
}

/// `PluginApi::objectives`: the HUD list for `player` (0: no team), or `NOT_PLAYER`.
pub fn objectives(player: c_int) -> Result<Vec<ObjectiveInfo>, c_int> {
    let p = if player == 0 { std::ptr::null_mut() } else { super::player_edict(player) };
    if player != 0 && p.is_null() {
        return Err(NOT_PLAYER);
    }
    let list = crate::entities::ts::objective::hud_list(p);
    Ok(list
        .into_iter()
        .map(|(id, e, origin, text)| {
            let mut name = [0; OBJECTIVE_NAME_LEN];
            for (d, &b) in name.iter_mut().zip(text.as_bytes().iter().take(OBJECTIVE_NAME_LEN - 1)) {
                *d = b as std::ffi::c_char;
            }
            let active = crate::entities::ts::objective::objective_active(e).unwrap_or(e.v().effects & EF_NODRAW as i32 == 0);
            ObjectiveInfo { id, entity: index_of(e), origin, active: active as c_int, name }
        })
        .collect())
}

/// `PluginApi::create_entity`: `CREATE_NAMED_ENTITY(classname)`, not spawned. A class the engine
/// can't make (no such export) or that has no game object is refused and freed again, as
/// `CBaseEntity::Create` (0xac480) does.
pub fn create_entity(classname: &CStr) -> c_int {
    let eng = engine();
    if classname.is_empty() {
        return INVALID;
    }
    let e = eng.create_named_entity(classname);
    if e.is_null() || !e.has_private_data() || world::type_of(e).is_none() {
        if !e.is_null() {
            eng.remove_entity(e);
        }
        return INVALID;
    }
    eng.index_of(e)
}

/// `PluginApi::entity_key_value`: `DispatchKeyValue` with the entity's classname as the engine's
/// `szClassName`.
pub fn entity_key_value(entity: c_int, key: &CStr, value: &CStr) -> c_int {
    let Some(e) = game_entity(entity) else { return NOT_PLAYER };
    let class = engine().string(e.v().classname);
    let class = (!class.is_empty()).then_some(class);
    crate::entities::dispatch_key_value(e, class, key, value) as c_int
}

/// `PluginApi::spawn_entity`: `DispatchSpawn`; a failed spawn (-1) removes the entity as the
/// engine does for a map entity, one that removed itself is gone already.
pub fn spawn_entity(entity: c_int) -> c_int {
    let Some(e) = game_entity(entity) else { return NOT_PLAYER };
    let r = crate::entities::dispatch_spawn(e);
    if e.is_free() || !e.has_private_data() {
        return INVALID;
    }
    if r < 0 {
        engine().remove_entity(e);
        return INVALID;
    }
    OK
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds() {
        assert_eq!(round_of(false, 0.0, 0.0), round::NONE);
        assert_eq!(round_of(false, 100.0, 0.0), round::NONE, "deathmatch keeps no round");
        assert_eq!(round_of(true, 100.0, 0.0), round::RUNNING);
        assert_eq!(round_of(true, 100.0, 103.0), round::OVER);
        assert_eq!(team(-1), 0);
        assert_eq!(team(0), 1);
    }

    #[test]
    fn team_scores_follow_the_rules_teams() {
        assert_eq!(team_scores([(true, 3), (true, 5)].into_iter()), [3, 5, 0, 0]);
        assert_eq!(team_scores([(true, 1), (false, 9), (true, 2), (true, 4), (true, 7)].into_iter()), [1, 0, 2, 4], "unnamed is 0");
    }
}
