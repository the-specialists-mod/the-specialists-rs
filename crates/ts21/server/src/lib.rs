//! TS 2.1's server rules (issue #100 §3) and the plugin object `server-dll` registers. Hooks it
//! doesn't override keep TS 3.0's body.
//!
//! * [`rules`]: the mode from the map name, teams, `CTSGameRules::Think` and the round modes.
//! * [`scoring`]: `TSGetPointsForFrag` and `PlayerKilled`.
//! * [`cash`]: `CashBased`, the cash floor, kill cash and buying.
//! * [`players`]: spawn, join, death, respawn timing, overheal and The One.
//! * [`cvars`] and [`commands`]: 2.1's extra cvars, the polled cvars and the client commands.
//! * [`powerup`]: power-ups and slow motion (§3.7).
//! * [`stunt`]: the server stunts and their slot limits (§3.8, §3.5).

#![deny(unsafe_code)]

pub mod cash;
pub mod commands;
pub mod cvars;
pub mod players;
pub mod powerup;
pub mod rules;
pub mod scoring;
pub mod stunt;

use ts_server::commands::Args;
use ts_server::damage::DamageInfo;
use ts_server::damage::glast::Snapshot;
use ts_server::gameplay::server::CvarDef;
use ts_server::gameplay::{ServerPlugin, ServerRules};
use ts_server::player::Player;
use ts_server::rules::powerup::PowerupRules;
use ts_server::rules::scoring::Frag;
use ts_server::rules::state::{self, GameRules, Mode};
use ts_server::stunt::rules::{Dive, StuntClock, StuntRules};
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
            // `CreatePowerUp(0x80, -1)`, then `pev->origin` and `+0x7c = (int)armorvalue`: the
            // armour it gives (`PlayerAddPowerUp`, `powerup::armor`), not its seconds.
            let e = ts_server::entities::ts::powerup::create(shared::ts::powerup::ARMOR, -1, cx.pev().origin);
            ts_server::entities::ts::powerup::set_armor(e, cvars::trunc(armor) as f32);
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

    fn powerups(&self) -> &'static PowerupRules {
        &powerup::POWERUPS
    }

    fn powerup_kind(&self, key: i32, spawnflags: i32) -> (u16, bool) {
        powerup::kind(key, spawnflags)
    }

    fn armor_powerup(&self, _p: &mut Player, cx: &mut Cx, armor: f32) {
        powerup::armor(cx, armor)
    }

    /// `GoSlow` (0x10055220) writes `pev->fuser1` directly; 2.1 has no `InterpolateSlowMotion`.
    fn time_scale_eases(&self) -> bool {
        false
    }

    /// 2.1 has no `TSMBlur` message.
    fn motion_blur(&self) -> bool {
        false
    }

    /// 2.1's `EMIT_SOUND_DYN` (0x10036300) is the SDK's: no time-scaled pitch.
    fn sound_pitch(&self, pitch: i32, time_scale: f32, s: f32) -> i32 {
        let _ = (time_scale, s);
        pitch
    }

    fn stunts(&self) -> &'static StuntRules {
        &stunt::STUNTS
    }

    fn stunt_clock(&self) -> &'static dyn StuntClock {
        &stunt::ABSOLUTE
    }

    fn can_dive(&self, p: &Player, armor: f32, dive: Dive) -> bool {
        stunt::can_dive(p, armor, dive)
    }

    fn bullet_time_allowed(&self, time_scale: f32) -> bool {
        stunt::bullet_time_allowed(time_scale)
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

    /// The 2.1 hooks differ where 2.1 does, and the core plugin keeps 3.0's.
    #[test]
    fn hooks_follow_the_plugin() {
        let core: &dyn ServerRules = &shared::gameplay::CORE;
        let v21: &dyn ServerRules = &Ts21Server;
        assert!(core.time_scale_eases() && !v21.time_scale_eases());
        assert!(core.motion_blur() && !v21.motion_blur());
        assert_eq!(core.sound_pitch(100, 0.5, 0.5), 75);
        assert_eq!(v21.sound_pitch(100, 0.5, 0.5), 100);
        assert_eq!(core.powerups().base(shared::ts::powerup::SLOWMO), 3);
        assert_eq!(v21.powerups().base(shared::ts::powerup::SLOWMO), 5);
        assert!(core.stunts().wall_dive && !v21.stunts().wall_dive);
        assert_eq!(core.stunt_clock().start(10.0, 1.0, 0.5), 0.5);
        assert_eq!(v21.stunt_clock().start(10.0, 1.0, 0.5), 10.5);
        assert!(core.bullet_time_allowed(0.99) && !core.bullet_time_allowed(0.995));
        assert!(v21.bullet_time_allowed(0.995) && !v21.bullet_time_allowed(1.0));
    }
}
