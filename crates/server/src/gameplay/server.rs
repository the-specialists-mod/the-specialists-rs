//! [`ServerRules`]: the server-only hooks of a gameplay ruleset (rules, scoring, damage,
//! entities...). Bots aren't part of a ruleset: they are left to a plugin axis of their own.
//!
//! The server's own code resolves them from the active plugin ([`active`](super::active)), which
//! follows `ts_gameplay` once a frame; a switch resets the game first ([`super::reset`]).

use crate::commands::Args;
use crate::damage::DamageInfo;
use crate::damage::glast::Snapshot;
use crate::entities::ts::ground::GroundWeapon;
use crate::entities::ts::misc::MapFlags;
use crate::messages::Msg;
use crate::player::Player;
use crate::rules::scoring::{self, Frag};
use crate::rules::state::GameRules;
use crate::rules::{cvars, frame, lifecycle, teams};
use crate::world::{Cx, Entity};
use goldsrc::server::MsgWriter;
use goldsrc::sys::edict_t;
use std::ffi::CStr;

/// A server cvar: (name, default, flags), as [`cvars::CVARS`] lists TS 3.0's.
pub type CvarDef = (&'static str, &'static str, i32);

/// Server rules. Every default body is TS 3.0's (`shared::gameplay::Core`); the server code calls
/// these where a ruleset can differ. A hook needing server state takes this crate's types, so
/// its 3.0 body can be the code it replaces.
pub trait ServerRules: Sync {
    /// The plugin has become active on a brand-new game: at every map start (after the rules
    /// are installed, before the map's entities spawn) and after a switch to it has reset the
    /// game ([`super::reset`]). The place to set up the plugin's own state.
    fn on_activate(&self) {}

    // ---- cvars and commands ----------------------------------------------------------------

    /// Server cvars the ruleset needs besides TS 3.0's ([`cvars::CVARS`]). They are registered
    /// when the plugin is ([`super::register`], at `GameDLLInit`), skipping any name that already
    /// exists, so the server has the union of every ruleset's cvars and each ruleset reads the
    /// ones it knows. 3.0: none.
    fn cvars(&self) -> &'static [CvarDef] {
        &[]
    }

    /// A TS server command (`globalslow`, `restartround`, `weaponrestriction`) before TS 3.0's
    /// handler runs it; true when the ruleset has handled it. The arguments are the engine's
    /// (`Cmd_Argv`). 3.0: false, the command runs.
    fn server_command(&self, _name: &str) -> bool {
        false
    }

    /// A player's client command (`ClientCommand` 0xad838) before TS 3.0's handlers (weapons,
    /// stunts, rules, bots): `Some(handled)` decides it (false prints "Unknown command"), `None`
    /// lets 3.0's code run.
    fn client_command(&self, _p: &mut Player, _cx: &mut Cx, _cmd: &str, _args: &Args) -> Option<bool> {
        None
    }

    // ---- game rules ------------------------------------------------------------------------

    /// `InstallGameRules` and the rules' constructors for a multiplayer map (`deathmatch` set):
    /// the game mode, teams and `CTSGameRules::Init` ([`GameRules::install_multiplayer`]). Runs
    /// inside the rules state's borrow, so it must not call code that reads the rules state.
    fn install(&self, g: &mut GameRules) {
        g.install_multiplayer();
    }

    /// `CTSGameRules::Think` (0x77204), from the multiplay or teamplay `Think` each frame.
    fn think(&self, now: f32) {
        frame::ts_think(now);
    }

    /// `CTSGameRules::SetPlayerTheOne` (0x7b078): makes a living, non-observing player The One.
    fn set_player_the_one(&self, p: &mut Player, cx: &mut Cx) {
        scoring::set_player_the_one_core(p, cx);
    }

    /// The team `CHalfLifeTeamplay::SetDefaultPlayerTeam` (0xe44ac) reads before it checks it
    /// against the team list ([`teams::default_team_core`]).
    fn default_team(&self, p: &mut Player, cx: &mut Cx) -> String {
        teams::default_team_core(p, cx)
    }

    // ---- players ---------------------------------------------------------------------------

    /// End of the rules part of `TSInit` at every spawn (`lifecycle::on_spawn`), before the
    /// team and the spawn point. 3.0: nothing.
    fn on_spawn(&self, _p: &mut Player, _cx: &mut Cx) {}

    /// The cash a player joins with (`ClientPutInServer`: `GiveCash(atoi(initial_cash.string))`).
    /// 3.0: [`cvars::INITIAL_CASH`].
    fn put_in_server_cash(&self) -> i32 {
        cvars::INITIAL_CASH
    }

    /// Start of `CBasePlayer::StartObserver` (0x99634), before the cash goes to the client.
    /// 3.0: nothing.
    fn on_start_observer(&self, _p: &mut Player, _cx: &mut Cx) {}

    /// End of `CTSGameRules::InitHUD` (0x79134), once a joiner is observing with its respawn
    /// time set. 3.0: nothing.
    fn on_init_hud(&self, _p: &mut Player, _cx: &mut Cx) {}

    /// `SetPlayerRespawnTime` (0x75ff4): when the player may respawn (`respawn_at`, -1 for not
    /// this round), the join deadline and the award.
    fn set_respawn_time(&self, p: &mut Player, cx: &mut Cx, secs: f32) {
        lifecycle::set_respawn_time_core(p, cx, secs);
    }

    /// `CBasePlayer::Killed` (0x8ed5c) right after the respawn time is set. 3.0: nothing.
    fn on_killed(&self, _p: &mut Player, _cx: &mut Cx) {}

    /// The overheal and The One's regeneration in `CBasePlayer::PreThink` (0x908cc, `+0x73c`).
    fn health_regen(&self, p: &mut Player, cx: &mut Cx, now: f32) {
        lifecycle::health_regen_core(p, cx, now);
    }

    // ---- scoring and cash ------------------------------------------------------------------

    /// `TSGetPointsForFrag` (0x89140): the points a killing player scores for `frag`, which also
    /// reports whether the kill was a slide or a stunt.
    fn points_for_frag(&self, frag: &Frag<'_>, sliding: &mut bool, diving: &mut bool) -> i32 {
        scoring::points_for_frag(frag, sliding, diving)
    }

    /// The body of `CHalfLifeMultiplay::PlayerKilled` (0x895e4) for victim `p`, once the killer
    /// can be reached (`scoring::player_killed` defers it until then): points, sprees, awards,
    /// cash, messages, `DeathMsg`, logs and `ScoreInfo`.
    fn player_killed(&self, p: &mut Player, cx: &mut Cx, d: &DamageInfo, g: Snapshot) {
        scoring::player_killed_core(p, cx, d, g);
    }

    /// `CashBased()` (0x76abc): whether buying costs money. 3.0: hard-wired off.
    fn cash_based(&self) -> bool {
        false
    }

    /// `CBasePlayer::BuyArmament` (from `PlayerSpawn` when the map has no `game_player_equip`):
    /// the loadout the client sent (`tkw`/`tki`/`tka`).
    fn buy_armament(&self, p: &mut Player, cx: &mut Cx) {
        crate::weapons::buy_armament(p, cx);
    }

    // ---- map entities -------------------------------------------------------------------------

    /// Whether an entity of `class` may spawn (`DispatchSpawn`, for the map's entities and the
    /// game's own): a refused one fails its spawn (-1, the engine frees it) and its class parses
    /// no keyvalue. TS 3.0 has an export for every class the port links: all spawn.
    fn spawns_entity(&self, _class: &CStr) -> bool {
        true
    }

    /// A keyvalue for an entity's class (`DispatchKeyValue`'s class part, after the entvars
    /// table), before the class's own `KeyValue`. `None` lets the class parse it (TS 3.0);
    /// `Some(handled)` means the plugin parsed or refused it, and is `fHandled`.
    fn entity_key_value(&self, _ent: &mut dyn Entity, _cx: &mut Cx, _key: &CStr, _value: &CStr) -> Option<bool> {
        None
    }

    /// An entity's `Touch` (`DispatchTouch` and the game's own touches): true when the plugin ran
    /// it instead of the class. TS 3.0: the class's.
    fn entity_touch(&self, _ent: &mut dyn Entity, _cx: &mut Cx, _other: *mut edict_t) -> bool {
        false
    }

    /// How long a broken `func_breakable` or breakable door waits before it rebuilds itself
    /// (`CBreakable::Die` 0x752e0, `CBaseDoor::TraceAttack` 0xb57ac), given its `respawntime` key (≥ 0); `None`
    /// leaves it broken, with no think, until `RespawnEntities` restores it. TS 3.0: the key, 0
    /// meaning 30 s on the `info_breakable` helper (`RespawnThink` 0x75da0).
    fn broken_respawn_delay(&self, respawn_time: f32) -> Option<f32> {
        Some(if respawn_time == 0.0 { 30.0 } else { respawn_time })
    }

    /// Whether `BreakableRespawn` (`CBreakable` 0x74e84, `CBaseDoor`/`CRotDoor` 0xb5604/0xb7838)
    /// waits while a client is inside the box (retrying in 5 s) and fires a breakable's
    /// `respawntarget` (spawnflag 8). TS 3.0: yes. False restores the entity at once and fires
    /// nothing.
    fn breakable_respawn_guarded(&self) -> bool {
        true
    }

    /// `CTSGameRules::RespawnEntities` as `RestartRound` (0x77920) calls it. TS 3.0's
    /// ([`crate::entities::ts::respawn_entities`]).
    fn respawn_entities(&self) {
        crate::entities::ts::respawn_entities();
    }

    /// The map-entity upkeep of `CTSGameRules::Think` (0x77204) in deathmatch and teamplay, after The One's
    /// check (`now` is the game time). TS 3.0 has none.
    fn entities_think(&self, _now: f32) {}

    /// What a `ts_mapglobals` (`e`) sets with its spawnflags (`CTSMapGlobals::Spawn` 0x72534),
    /// or `None` to set nothing. TS 3.0: [`MapFlags::from_spawnflags`].
    fn map_globals(&self, _e: *mut edict_t, spawnflags: i32) -> Option<MapFlags> {
        Some(MapFlags::from_spawnflags(spawnflags))
    }

    /// Whether a landing `ts_groundweapon` (`OnGround`) also becomes `MOVETYPE_FLY` with
    /// `effects |= 2`, as `ts_groundammo`'s does. TS 3.0: no (`CTSGroundWeapon::OnGround`
    /// 0xa226c).
    fn ground_weapon_lands_flying(&self) -> bool {
        false
    }

    /// `CTSGroundWeapon::GetWeapon` (0xa2328) for a player, past its `IsPlayer` test (the
    /// ground weapon `g` is `cx`, the player `p` is `pcx`). TS 3.0's
    /// ([`GroundWeapon::give`]).
    fn give_ground_weapon(&self, g: &mut GroundWeapon, cx: &mut Cx, p: &mut Player, pcx: &mut Cx) {
        g.give(cx, p, pcx);
    }

    // ---- protocol -----------------------------------------------------------------------------

    /// A user message on its way out (`msg`, written into `w`): hands `send` what goes on the
    /// wire in its place. TS 3.0 sends it as it is.
    fn outgoing_message(&self, _msg: Msg, w: MsgWriter, send: &mut dyn FnMut(MsgWriter)) {
        send(w);
    }
}

#[cfg(test)]
mod tests {
    use super::ServerRules;
    use crate::rules::cvars;

    /// The core plugin's hooks that need no engine keep TS 3.0's values.
    #[test]
    fn core_hooks_are_ts_3_0() {
        let core: &dyn ServerRules = &shared::gameplay::CORE;
        assert!(core.cvars().is_empty(), "3.0 registers only its own cvars");
        assert!(!core.cash_based(), "3.0's CashBased is hard-wired off");
        assert_eq!(core.put_in_server_cash(), cvars::INITIAL_CASH);
        assert!(!core.server_command("globalslow"), "3.0 runs its server commands");
        assert!(!core.server_command("restartround"));
        assert!(!core.server_command("weaponrestriction"));
    }
}
