//! TS 2.1's server rules (issue #100 §3) and the plugin object `server-dll` registers.
//!
//! * [`entities`]: the map entities (§3.9).
//! * [`protocol`]: 2.1 on TS 3.0's wire (§1.2).

#![deny(unsafe_code)]

pub mod entities;
pub mod protocol;

use goldsrc::server::MsgWriter;
use goldsrc::sys::edict_t;
use std::ffi::CStr;
use ts_server::entities::ts::ground::GroundWeapon;
use ts_server::entities::ts::misc::MapFlags;
use ts_server::gameplay::{ServerPlugin, ServerRules};
use ts_server::messages::Msg;
use ts_server::player::Player;
use ts_server::world::{Cx, Entity};
use ts21_gameplay::TS21;

/// TS 2.1's server-only rules.
pub struct Ts21Server;

impl ServerRules for Ts21Server {
    fn on_activate(&self) {
        entities::reset();
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

    fn entities_think(&self, now: f32) {
        entities::entities_think(now);
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
