//! [`ServerRules`]: the server-only hooks of a gameplay ruleset (rules, scoring, damage,
//! entities...). Bots aren't part of a ruleset: they are left to a plugin axis of their own.
//!
//! The server's own code resolves them from the active plugin ([`active`](super::active)), which
//! follows `ts_gameplay` once a frame; a switch resets the game first ([`super::reset`]).

use crate::commands::Args;
use crate::damage::DamageInfo;
use crate::damage::glast::Snapshot;
use crate::player::Player;
use crate::rules::powerup::{self, PowerupRules};
use crate::rules::scoring::{self, Frag};
use crate::rules::state::GameRules;
use crate::rules::{cvars, frame, lifecycle, teams};
use crate::stunt::rules::{self as stunt, Dive, StuntClock, StuntRules};
use crate::world::Cx;

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

    // ---- power-ups and slow motion ----------------------------------------------------------

    /// The power-up values: durations, the instant types, the pickup thresholds, the slow-motion
    /// amounts (`SetupPowerUp` 0x98014, `WaitTouch`, `UpdatePowerUpState` 0x95748).
    fn powerups(&self) -> &'static PowerupRules {
        &powerup::CORE
    }

    /// `SetupPowerUp`'s type for a `ts_powerup` with the type `key` (`pwuptype`, < 1 random)
    /// and `spawnflags`: the type, and whether it is drawn again on each respawn.
    fn powerup_kind(&self, key: i32, spawnflags: i32) -> (u16, bool) {
        crate::entities::ts::powerup::kind_for(key, spawnflags)
    }

    /// `PlayerAddPowerUp` (0x97bc8) for an armour power-up worth `armor` (100 unless a ruleset
    /// makes it otherwise): `TakeArmor(100)`, which costs slots.
    fn armor_powerup(&self, p: &mut Player, cx: &mut Cx, armor: f32) {
        let _ = armor;
        crate::weapons::take_armor(p, cx, 100);
    }

    /// `GoSlow` (0x929d0) sets a target time scale that `InterpolateSlowMotion` (0x9910c) eases
    /// `pev->fuser1` toward each PreThink. False: `GoSlow` writes `fuser1` at once.
    fn time_scale_eases(&self) -> bool {
        true
    }

    /// The ruleset sends `TSMBlur` (stunts, power-ups, kills, kung fu), and draws the random
    /// numbers deciding it.
    fn motion_blur(&self) -> bool {
        true
    }

    /// `EMIT_SOUND_DYN`'s pitch (0xe0aa7) for an entity whose time scale is `time_scale`, with
    /// the slow factor `s` ([`slow_pitch`](crate::entities::stock::explode::slow_pitch)).
    fn sound_pitch(&self, pitch: i32, time_scale: f32, s: f32) -> i32 {
        crate::entities::stock::explode::slow_pitch(pitch, time_scale, s)
    }

    // ---- stunts (CTSStunt) ------------------------------------------------------------------

    /// The stunt values (recoveries, keys, which moves exist).
    fn stunts(&self) -> &'static StuntRules {
        &stunt::CORE
    }

    /// How the stunt timers keep time.
    fn stunt_clock(&self) -> &'static dyn StuntClock {
        &stunt::RELATIVE
    }

    /// `CanDiveHigh/Medium/Low`: the player can do a stunt of this height. `armor` is
    /// `pev->armorvalue`.
    fn can_dive(&self, p: &Player, armor: f32, dive: Dive) -> bool {
        let _ = armor;
        stunt::can_dive(p, dive)
    }

    /// `GoBulletTime` (0x9a5b4): bullet time is possible at the time scale `time_scale`
    /// (`pev->fuser1`): `GiveSlowMul() <= 0.99`.
    fn bullet_time_allowed(&self, time_scale: f32) -> bool {
        time_scale.max(0.01) <= 0.99
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
