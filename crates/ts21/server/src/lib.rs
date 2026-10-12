//! TS 2.1's server rules (issue #100 §3) and the plugin object `server-dll` registers. Hooks it
//! doesn't override keep TS 3.0's body.
//!
//! * [`rules`]: the mode from the map name, teams, `CTSGameRules::Think` and the round modes.
//! * [`scoring`]: `TSGetPointsForFrag` and `PlayerKilled`.
//! * [`cash`]: `CashBased`, the cash floor, kill cash and buying.
//! * [`players`]: spawn, join, death, respawn timing, overheal and The One.
//! * [`cvars`] and [`commands`]: 2.1's extra cvars, the polled cvars and the client commands.

#![deny(unsafe_code)]

pub mod cash;
pub mod commands;
pub mod cvars;
pub mod players;
pub mod rules;
pub mod scoring;

use ts_server::commands::Args;
use ts_server::damage::DamageInfo;
use ts_server::damage::glast::Snapshot;
use ts_server::gameplay::server::CvarDef;
use ts_server::gameplay::{ServerPlugin, ServerRules};
use ts_server::player::Player;
use ts_server::rules::scoring::Frag;
use ts_server::rules::state::{self, GameRules, Mode};
use ts_server::world::Cx;
use ts21_gameplay::TS21;

/// TS 2.1's server-only rules.
pub struct Ts21Server;

impl ServerRules for Ts21Server {
    fn cvars(&self) -> &'static [CvarDef] {
        cvars::CVARS
    }

    fn server_command(&self, name: &str) -> bool {
        commands::server_command(name)
    }

    fn client_command(&self, p: &mut Player, cx: &mut Cx, cmd: &str, args: &Args) -> Option<bool> {
        commands::client_command(p, cx, cmd, args)
    }

    fn install(&self, g: &mut GameRules) {
        rules::install(g);
    }

    fn think(&self, now: f32) {
        rules::think(now);
    }

    fn set_player_the_one(&self, p: &mut Player, cx: &mut Cx) {
        players::set_player_the_one(p, cx);
    }

    fn default_team(&self, p: &mut Player, cx: &mut Cx) -> String {
        players::default_team(p, cx)
    }

    fn on_spawn(&self, p: &mut Player, _cx: &mut Cx) {
        players::on_spawn(p);
    }

    /// 2.1's `ClientPutInServer` gives no cash; `InitHUD` sets it ([`players::on_init_hud`]).
    fn put_in_server_cash(&self) -> i32 {
        0
    }

    fn on_start_observer(&self, p: &mut Player, _cx: &mut Cx) {
        cash::cash_floor(p);
    }

    fn on_init_hud(&self, p: &mut Player, _cx: &mut Cx) {
        players::on_init_hud(p);
    }

    fn set_respawn_time(&self, p: &mut Player, cx: &mut Cx, secs: f32) {
        players::set_respawn_time(p, cx, secs);
    }

    /// 2.1's `Killed` (0x10052390): the cash floor and, in DM/TDM, an armour power-up holding the
    /// victim's armour where it died.
    fn on_killed(&self, p: &mut Player, cx: &mut Cx) {
        cash::cash_floor(p);
        let armor = cx.pev().armorvalue;
        if state::with(|g| matches!(g.mode, Mode::Dm | Mode::TeamDm)) && armor > 0.0 {
            // `CreatePowerUp(0x80, -1)`, then `pev->origin` and `+0x7c = (int)armorvalue`.
            ts_server::entities::ts::create_powerup(shared::ts::powerup::ARMOR, cvars::trunc(armor), cx.pev().origin);
        }
    }

    fn health_regen(&self, p: &mut Player, cx: &mut Cx, now: f32) {
        players::health_regen(p, cx, now);
    }

    fn points_for_frag(&self, frag: &Frag<'_>, sliding: &mut bool, diving: &mut bool) -> i32 {
        scoring::points_for_frag(frag, sliding, diving)
    }

    fn player_killed(&self, p: &mut Player, cx: &mut Cx, d: &DamageInfo, g: Snapshot) {
        scoring::player_killed(p, cx, d, g);
    }

    fn cash_based(&self) -> bool {
        cash::cash_based()
    }

    fn buy_armament(&self, p: &mut Player, cx: &mut Cx) {
        cash::buy_armament(p, cx);
    }
}

/// The plugin on the server: TS 2.1's shared hooks and its server hooks.
pub static PLUGIN: ServerPlugin = ServerPlugin { pm: &TS21, gun: &TS21, server: &Ts21Server };

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overrides_without_the_engine() {
        let s: &dyn ServerRules = &Ts21Server;
        assert_eq!(s.cvars(), cvars::CVARS);
        assert_eq!(s.put_in_server_cash(), 0, "2.1 sets the cash in InitHUD instead");
        assert!(!s.server_command("addbot"), "only the polled cvars are 2.1's");
    }
}
