//! The plugin API table (`ts_plugin_api::PluginApi`) that `CreateInterface` hands out: a port
//! addition. Each function converts its arguments and calls `ts_server::plugin_api`, where the
//! logic is (docs/implementation/plugin-api.md).

use std::ffi::c_int;
use ts_plugin_api::{NOT_PLAYER, OK, PlayerInfo, PluginApi, VERSION};
use ts_server::plugin_api as api;

/// The one table. Plugins only read it.
pub static PLUGIN_API: PluginApi = PluginApi {
    size: size_of::<PluginApi>() as u32,
    version: VERSION,
    player_info,
    set_cash,
    set_free_slots,
    set_award,
    run_powerup,
    set_speed,
    set_physics_speed,
    set_bullet_trail,
    set_weapon_ammo,
    projectile_owner,
};

/// Guard: a NULL `out` gives `NOT_PLAYER` (the contract asks for a valid pointer); nothing is
/// written on failure.
unsafe extern "C" fn player_info(player: c_int, out: *mut PlayerInfo) -> c_int {
    if out.is_null() {
        return NOT_PLAYER;
    }
    match api::player_info(player) {
        Ok(info) => {
            // SAFETY: the caller passes a `PlayerInfo` valid for writes (the table's contract);
            // `write_unaligned` doesn't rely on its alignment.
            unsafe { out.write_unaligned(info) };
            OK
        }
        Err(code) => code,
    }
}

extern "C" fn set_cash(player: c_int, cash: c_int) -> c_int {
    api::set_cash(player, cash)
}

extern "C" fn set_free_slots(player: c_int, slots: c_int) -> c_int {
    api::set_free_slots(player, slots)
}

extern "C" fn set_award(player: c_int, award: c_int) -> c_int {
    api::set_award(player, award)
}

extern "C" fn run_powerup(player: c_int, powerup: c_int, duration: f32) -> c_int {
    api::run_powerup(player, powerup, duration)
}

extern "C" fn set_speed(player: c_int, speed: f32, aura: f32, duration: f32) -> c_int {
    api::set_speed(player, speed, aura, duration)
}

extern "C" fn set_physics_speed(player: c_int, speed: f32) -> c_int {
    api::set_physics_speed(player, speed)
}

extern "C" fn set_bullet_trail(player: c_int, on: c_int) -> c_int {
    api::set_bullet_trail(player, on)
}

extern "C" fn set_weapon_ammo(player: c_int, weapon: c_int, clip: c_int, ammo: c_int) -> c_int {
    api::set_weapon_ammo(player, weapon, clip, ammo)
}

extern "C" fn projectile_owner(entity: c_int) -> c_int {
    api::projectile_owner(entity)
}
