//! TS 2.1's server rules (issue #100 §3) and the plugin object `server-dll` registers. Hooks it
//! doesn't override keep TS 3.0's body.
//!
//! * [`rules`]: the mode from the map name, teams, `CTSGameRules::Think` and the round modes.
//! * [`scoring`]: `TSGetPointsForFrag` and `PlayerKilled`.
//! * [`cash`]: `CashBased`, the cash floor, kill cash and buying.
//! * [`players`]: spawn, join, death, respawn timing, overheal and The One.
//! * [`cvars`] and [`commands`]: 2.1's extra cvars, the polled cvars and the client commands.
//! * [`entities`]: the map entities (§3.9).
//! * [`protocol`]: 2.1 on TS 3.0's wire (§1.2).

#![deny(unsafe_code)]

pub mod cash;
pub mod commands;
pub mod cvars;
pub mod entities;
pub mod players;
pub mod protocol;
pub mod rules;
pub mod scoring;

use goldsrc::server::MsgWriter;
use goldsrc::sys::edict_t;
use std::ffi::CStr;
use ts_server::commands::Args;
use ts_server::damage::DamageInfo;
use ts_server::damage::glast::Snapshot;
use ts_server::entities::ts::ground::GroundWeapon;
use ts_server::entities::ts::misc::MapFlags;
use ts_server::gameplay::server::CvarDef;
use ts_server::gameplay::{ServerPlugin, ServerRules};
use ts_server::messages::Msg;
use ts_server::player::Player;
use ts_server::rules::scoring::Frag;
use ts_server::rules::state::{self, GameRules, Mode};
use ts_server::world::{Cx, Entity};
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

    fn spawns_entity(&self, class: &CStr) -> bool {
        entities::spawns(class)
    }

    fn entity_key_value(&self, ent: &mut dyn Entity, cx: &mut Cx, key: &CStr, value: &CStr) -> Option<bool> {
        entities::key_value(ent, cx, key, value)
    }

    fn entity_touch(&self, ent: &mut dyn Entity, cx: &mut Cx, other: *mut edict_t) -> bool {
        entities::touch(ent, cx, other)
    }

    /// 2.1's `CBreakable::Die` (0x10042540) and `CBaseDoor::TraceAttack` (0x10013c50) leave the
    /// entity broken: their think (FUN_10015e10) only pushes `nextthink` on.
    fn broken_respawn_delay(&self, _respawn_time: f32) -> Option<f32> {
        None
    }

    fn breakable_respawn_guarded(&self) -> bool {
        false
    }

    /// `RestartRound` (0x100445d0) forces it.
    fn respawn_entities(&self) {
        entities::respawn_entities(true);
    }

    fn map_globals(&self, e: *mut edict_t, spawnflags: i32) -> Option<MapFlags> {
        entities::map_globals(e, spawnflags)
    }

    /// 2.1's `OnGround` (FUN_10066190) makes the weapon `MOVETYPE_FLY` with `effects |= 2`.
    fn ground_weapon_lands_flying(&self) -> bool {
        true
    }

    fn give_ground_weapon(&self, g: &mut GroundWeapon, cx: &mut Cx, p: &mut Player, pcx: &mut Cx) {
        entities::give_ground_weapon(g, cx, p, pcx);
    }

    fn outgoing_message(&self, m: Msg, w: MsgWriter, send: &mut dyn FnMut(MsgWriter)) {
        protocol::outgoing_message(m, w, send);
    }
}

/// The plugin on the server: TS 2.1's shared hooks and its server hooks.
pub static PLUGIN: ServerPlugin = ServerPlugin { pm: &TS21, gun: &TS21, server: &Ts21Server };

#[cfg(test)]
mod tests {
    use super::*;
    use shared::gameplay::CORE;

    #[test]
    fn overrides_without_the_engine() {
        let s: &dyn ServerRules = &Ts21Server;
        assert_eq!(s.cvars(), cvars::CVARS);
        assert_eq!(s.put_in_server_cash(), 0, "2.1 sets the cash in InitHUD instead");
        assert!(!s.server_command("addbot"), "only the polled cvars are 2.1's");
    }

    /// The 3.0 defaults the 2.1 plugin overrides, unchanged by the hooks.
    #[test]
    fn core_keeps_3_0s_entities() {
        let core: &dyn ServerRules = &CORE;
        assert!(core.spawns_entity(c"ts_flag"));
        assert!(core.spawns_entity(c"weapon_shotgun"));
        assert_eq!(core.broken_respawn_delay(0.0), Some(30.0));
        assert_eq!(core.broken_respawn_delay(12.0), Some(12.0));
        assert!(core.breakable_respawn_guarded());
        assert!(!core.ground_weapon_lands_flying());
        assert_eq!(core.map_globals(std::ptr::null_mut(), 32 | 64), Some(MapFlags::from_spawnflags(32 | 64)));
        assert_eq!(MapFlags::from_spawnflags(32 | 64), MapFlags { round_timer: false, superjump: true, weapons: 0x1f ^ 0x60 });
    }

    #[test]
    fn plugin_overrides() {
        let p: &dyn ServerRules = &Ts21Server;
        assert!(!p.spawns_entity(c"ts_flag"));
        assert!(p.spawns_entity(c"func_breakable"));
        assert_eq!(p.broken_respawn_delay(0.0), None);
        assert_eq!(p.broken_respawn_delay(12.0), None);
        assert!(!p.breakable_respawn_guarded());
        assert!(p.ground_weapon_lands_flying());
    }
}
