//! TS 3.0's server rules (issue #200) and the plugin object `server-dll` registers.
//!
//! * [`PLUGIN`]: the gameplay plugin, [`Ts30Server`] with the shared [`TS30`] hooks.
//! * [`rules`]: the multiplayer install, the server browser's description and
//!   `CTSGameRules::Think`.
//! * [`players`]: The One, teams, the respawn time, the overheal and the spawn loadout.
//! * [`observer`]: the observer part of `PreThink` and `Observer_SetMode`.
//! * [`scoring`]: `PlayerKilled` and `TSGetPointsForFrag`.
//! * [`bots::BOTS`]: the native bot backend's rules for 3.0, which `server-dll` registers with
//!   `ts_server::bots::rules` under the plugin's id (the bots are a layer of their own, so they
//!   aren't part of [`PLUGIN`]).

#![deny(unsafe_code)]

pub mod bots;
pub mod observer;
pub mod players;
pub mod rules;
pub mod scoring;

use goldsrc::server::MsgWriter;
use std::ffi::CStr;
use ts_server::commands::Args;
use ts_server::damage::DamageInfo;
use ts_server::damage::glast::Snapshot;
use ts_server::gameplay::server::CvarDef;
use ts_server::gameplay::{ConnectingClient, ServerPlugin, ServerRules};
use ts_server::messages::Msg;
use ts_server::player::Player;
use ts_server::rules::DescriptionMode;
use ts_server::rules::scoring::Frag;
use ts_server::rules::state::GameRules;
use ts_server::world::Cx;
use ts30_gameplay::TS30;

/// TS 3.0's server-only rules.
pub struct Ts30Server;

impl ServerRules for Ts30Server {
    /// Nothing: 3.0 keeps no state of its own.
    fn on_activate(&self) {}

    /// `None`: everyone may join.
    fn refuse_client(&self, _client: ConnectingClient) -> Option<&'static str> {
        None
    }

    /// "The Specialists v3.0.2 (DM)" and so on ([`rules::description`]).
    fn game_description(&self, mode: DescriptionMode) -> &'static CStr {
        rules::description(mode)
    }

    /// None: TS 3.0's own (`ts_server::rules::cvars::CVARS`) are the server's.
    fn cvars(&self) -> &'static [CvarDef] {
        &[]
    }

    /// False: the command runs.
    fn server_command(&self, _name: &str) -> bool {
        false
    }

    /// `None`: 3.0's handlers (weapons, stunts, rules, bots) run.
    fn client_command(&self, _p: &mut Player, _cx: &mut Cx, _cmd: &str, _args: &Args) -> Option<bool> {
        None
    }

    /// `DecideGamePlay`, the rules' constructors and `CTSGameRules::Init` ([`rules::install`]).
    fn install(&self, g: &mut GameRules) {
        rules::install(g);
    }

    /// `CTSGameRules::Think` (0x77204, [`rules::think`]).
    fn think(&self, now: f32) {
        rules::think(now);
    }

    /// `CTSGameRules::SetPlayerTheOne` (0x7b078, [`players::set_player_the_one`]).
    fn set_player_the_one(&self, p: &mut Player, cx: &mut Cx) {
        players::set_player_the_one(p, cx);
    }

    /// In The One mode The One's team or the other one, otherwise the `team` key
    /// ([`players::default_team`]).
    fn default_team(&self, p: &mut Player, cx: &mut Cx) -> Vec<u8> {
        players::default_team(p, cx)
    }

    /// Nothing.
    fn on_spawn(&self, _p: &mut Player, _cx: &mut Cx) {}

    /// `initial_cash`, which TS never registers: [`INITIAL_CASH`](ts_server::rules::cvars::INITIAL_CASH).
    fn put_in_server_cash(&self) -> i32 {
        ts_server::rules::cvars::INITIAL_CASH
    }

    /// Nothing.
    fn on_start_observer(&self, _p: &mut Player, _cx: &mut Cx) {}

    /// The countdown, the join deadline, `Observer_HandleButtons` and `Observer_Idle`
    /// ([`observer::observer_think`]); 3.0 respawns with the `respawn` command instead.
    fn observer_think(&self, p: &mut Player, cx: &mut Cx, now: f32) {
        observer::observer_think(p, cx, now);
    }

    /// `CBasePlayer::Observer_SetMode` (0x99e44, [`observer::set_mode`]).
    fn observer_set_mode(&self, p: &mut Player, cx: &mut Cx, mode: i32) {
        observer::set_mode(p, cx, mode);
    }

    /// Nothing.
    fn on_init_hud(&self, _p: &mut Player, _cx: &mut Cx) {}

    /// `SetPlayerRespawnTime` (0x75ff4, [`players::set_respawn_time`]).
    fn set_respawn_time(&self, p: &mut Player, cx: &mut Cx, secs: f32) {
        players::set_respawn_time(p, cx, secs);
    }

    /// Nothing.
    fn on_killed(&self, _p: &mut Player, _cx: &mut Cx) {}

    /// A change that isn't a kill, outside The One mode, observes with a 5 s respawn
    /// ([`players::change_team_tail`]).
    fn change_team_tail(&self, p: &mut Player, cx: &mut Cx, kill: bool) {
        players::change_team_tail(p, cx, kill);
    }

    /// The 1/s overheal decay and The One's regeneration ([`players::health_regen`]).
    fn health_regen(&self, p: &mut Player, cx: &mut Cx, now: f32) {
        players::health_regen(p, cx, now);
    }

    /// `TSGetPointsForFrag` (0x89140, [`scoring::points_for_frag`]).
    fn points_for_frag(&self, frag: &Frag<'_>, sliding: &mut bool, diving: &mut bool) -> i32 {
        scoring::points_for_frag(frag, sliding, diving)
    }

    /// The body of `CHalfLifeMultiplay::PlayerKilled` (0x895e4, [`scoring::player_killed`]).
    fn player_killed(&self, p: &mut Player, cx: &mut Cx, d: &DamageInfo, g: Snapshot) {
        scoring::player_killed(p, cx, d, g);
    }

    /// `CashBased()` (0x76abc): hard-wired off.
    fn cash_based(&self) -> bool {
        false
    }

    /// `CBasePlayer::BuyArmament` ([`players::buy_armament`]).
    fn buy_armament(&self, p: &mut Player, cx: &mut Cx) {
        players::buy_armament(p, cx);
    }

    /// None.
    fn precache(&self) {}

    /// The message goes out as it is.
    fn outgoing_message(&self, _msg: Msg, w: MsgWriter, send: &mut dyn FnMut(MsgWriter)) {
        send(w);
    }
}

/// The plugin as the server registers it.
pub static PLUGIN: ServerPlugin = ServerPlugin { pm: &TS30, gun: &TS30, server: &Ts30Server };

#[cfg(test)]
mod tests {
    use super::*;

    /// The hooks that need no engine.
    #[test]
    fn hooks_without_the_engine() {
        use ConnectingClient::*;
        let s: &dyn ServerRules = &Ts30Server;
        for c in [Bot, Hltv, Port("3.0.2"), Other] {
            assert_eq!(s.refuse_client(c), None, "3.0 refuses no one");
        }
        assert!(s.cvars().is_empty(), "3.0 registers only its own cvars");
        assert!(!s.cash_based(), "3.0's CashBased is hard-wired off");
        assert_eq!(s.put_in_server_cash(), ts_server::rules::cvars::INITIAL_CASH);
        assert!(!s.server_command("globalslow"), "3.0 runs its server commands");
        assert!(!s.server_command("restartround"));
        assert!(!s.server_command("weaponrestriction"));
        let v = env!("CARGO_PKG_VERSION");
        assert_eq!(s.game_description(DescriptionMode::Lms).to_str().unwrap(), format!("The Specialists v{v} (LMS)"));
    }
}
