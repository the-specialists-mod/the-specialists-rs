//! The plugin API's bot functions (version 3, issue #104): what a bot backend plugin needs to
//! make and drive its bots through the port's own code paths (docs/implementation/bot-plugins.md).
//! TS's bots don't use them: they are the port's native backend (`crate::bots`).

use super::{on_player, player_edict};
use crate::bot_backend;
use crate::commands::{self, Args};
use crate::rules::{self, cvars};
use crate::weapons::{self, ArmamentEntry, Drop};
use crate::world;
use goldsrc::edict::EdictPtr;
use goldsrc::server::engine;
use goldsrc::sys::*;
use shared::weapons::{MAX_AMMO, info};
use std::ffi::{CStr, c_int};
use ts_plugin_api::{BUSY, BotCmd, DEFERRED, FULL, INVALID, NOT_PLAYER, OK, PluginId, item};

/// `PluginApi::create_bot`: only for the plugin whose backend is active (a map start selected it),
/// with a name.
pub fn create_bot(plugin: PluginId, name: &CStr, model: &CStr, skill: f32) -> c_int {
    if name.is_empty() || bot_backend::active_plugin() != Some(plugin) {
        return INVALID;
    }
    let backend = bot_backend::active();
    // The name and model bytes as the plugin gave them, as TS's `strcpy`s (#190).
    match bot_backend::create_player(name.to_bytes(), model.to_bytes(), backend.new_state(skill), None, true) {
        Some(e) => engine().index_of(e),
        None => FULL,
    }
}

/// Whether the connected player `e` is a bot (TS's flag, or the engine's fake client).
fn is_bot(e: *mut edict_t) -> bool {
    e.v().flags & FL_FAKECLIENT as i32 != 0 || rules::other(e, |p| p.is_bot != 0).unwrap_or(false)
}

/// `PluginApi::kick_bot`: the server's `kick #<userid>`, queued as the quota's.
pub fn kick_bot(player: c_int) -> c_int {
    let e = player_edict(player);
    if e.is_null() {
        return NOT_PLAYER;
    }
    if !is_bot(e) {
        return INVALID;
    }
    let eng = engine();
    eng.server_command(&format!("kick #{}\n", eng.user_id(e)));
    OK
}

/// What `run_cmd` sends: the command as given, or with `ts_bot_stop` a living bot's movement,
/// buttons and impulse cleared (its view still turns), as the native bots' frozen think does.
pub fn stopped(cmd: BotCmd, stop: bool, playing: bool) -> BotCmd {
    if stop && playing { BotCmd { forward: 0.0, side: 0.0, up: 0.0, buttons: 0, impulse: 0, ..cmd } } else { cmd }
}

/// `run_cmd`'s argument check: a usercmd's `msec` is a byte, and 0 would run nothing.
pub fn valid_cmd(cmd: &BotCmd) -> bool {
    (1..=255).contains(&cmd.msec)
        && (0..=255).contains(&cmd.impulse)
        && cmd.angles.iter().chain([&cmd.forward, &cmd.side, &cmd.up]).all(|v| v.is_finite())
}

/// `PluginApi::run_cmd`: `pfnRunPlayerMove` for a bot, outside its own callbacks.
pub fn run_cmd(player: c_int, cmd: &BotCmd) -> c_int {
    let e = player_edict(player);
    if e.is_null() {
        return NOT_PLAYER;
    }
    if !is_bot(e) || !valid_cmd(cmd) {
        return INVALID;
    }
    // The engine runs the bot's PreThink and PostThink inside the call: its object must be free.
    if world::is_taken_out(e) {
        return BUSY;
    }
    let playing = {
        let v = e.v();
        rules::alive(v) && v.iuser1 == 0
    };
    let c = stopped(*cmd, cvars::f(c"ts_bot_stop") != 0.0, playing);
    e.v().button = c.buttons;
    engine().run_player_move(e, c.angles, c.forward, c.side, c.up, c.buttons as u16, c.impulse as u8, c.msec as u8);
    OK
}

/// `PluginApi::bot_command`: the client command path (`commands::run`), now if the player can be
/// reached, else when its callback returns.
pub fn bot_command(player: c_int, line: &str) -> c_int {
    let e = player_edict(player);
    if e.is_null() {
        return NOT_PLAYER;
    }
    let Some(args) = Args::parse(line) else { return INVALID };
    if world::is_taken_out(e) {
        world::defer(e, move || {
            if rules::is_player(e) {
                commands::run(e, &args);
            }
        });
        return DEFERRED;
    }
    commands::run(e, &args);
    OK
}

/// `PluginApi::select_weapon`: `weapon_<id>` (`weapons::client_command`) for an owned weapon or
/// kung fu (0).
pub fn select_weapon(player: c_int, weapon: c_int) -> c_int {
    if player_edict(player).is_null() {
        return NOT_PLAYER;
    }
    if !(0..shared::weapons::MAX_WEAPONS as c_int).contains(&weapon) {
        return INVALID;
    }
    on_player(player, move |p, cx| {
        if weapon != 0 && !p.inv.gun.status[weapon as usize].owned {
            return false;
        }
        let cmd = format!("weapon_{weapon}");
        let args = Args::parse(&cmd).expect("not empty");
        weapons::client_command(p, cx, &cmd, &args)
    })
}

/// `buy`'s argument check: a TS weapon with a table entry (1..37), attachment bits, and 0..255 ammo
/// purchases.
pub fn valid_buy(weapon: c_int, attachments: c_int, ammo: c_int) -> bool {
    let known = u8::try_from(weapon).ok().is_some_and(|id| id != 0 && info(id).is_some());
    known && attachments >= 0 && (0..=255).contains(&ammo)
}

/// `PluginApi::buy`: as the client's `tkw` / `tka` loadout commands (`CBasePlayer::AddItemToArmament`,
/// `AddAmmoToArmament`), which `BuyArmament` applies at the next spawn. The second copy of an
/// akimbo weapon is what makes the pair, as the native bots' `buy_weapon` does.
pub fn buy(player: c_int, weapon: c_int, attachments: c_int, akimbo: c_int, ammo: c_int) -> c_int {
    if player_edict(player).is_null() {
        return NOT_PLAYER;
    }
    if !valid_buy(weapon, attachments, ammo) {
        return INVALID;
    }
    on_player(player, move |p, _| {
        let w = info(weapon as u8).expect("checked");
        let list = &mut p.inv.sv.armament;
        weapons::add_armament(list, ArmamentEntry { id: weapon, attachments: attachments as u32, item: false });
        if akimbo != 0 && w.akimbo != -1 {
            weapons::add_armament(list, ArmamentEntry { id: weapon, attachments: 0, item: false });
        }
        if ammo > 0 && w.ammo != 0 && (w.ammo as usize) < MAX_AMMO {
            p.inv.sv.armament_ammo[w.ammo as usize] = ammo;
        }
        true
    })
}

/// `PluginApi::buy_item`: `tki` with kevlar or stealth shoes.
pub fn buy_item(player: c_int, it: c_int) -> c_int {
    if player_edict(player).is_null() {
        return NOT_PLAYER;
    }
    if it != item::KEVLAR && it != item::STEALTH_SHOES {
        return INVALID;
    }
    on_player(player, move |p, _| {
        weapons::add_armament(&mut p.inv.sv.armament, ArmamentEntry { id: it, attachments: 0, item: true });
        true
    })
}

/// `PluginApi::clear_loadout`: `rst_cfg`.
pub fn clear_loadout(player: c_int) -> c_int {
    on_player(player, |p, _| {
        p.inv.sv.armament.clear();
        p.inv.sv.armament_ammo = [0; MAX_AMMO];
        true
    })
}

/// `PluginApi::drop_weapon`: `drop` (negative id) or `drop_<id>`.
pub fn drop_weapon(player: c_int, weapon: c_int) -> c_int {
    if player_edict(player).is_null() {
        return NOT_PLAYER;
    }
    let which = match weapon {
        w if w < 0 => Drop::Current,
        w if w < shared::weapons::MAX_WEAPONS as c_int => Drop::Id(w as u8),
        _ => return INVALID,
    };
    on_player(player, move |p, cx| {
        weapons::drop_weapon(p, cx, which);
        true
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd() -> BotCmd {
        BotCmd { angles: [1.0, 2.0, 0.0], forward: 200.0, side: -50.0, up: 10.0, buttons: 2, impulse: 100, msec: 16 }
    }

    #[test]
    fn bot_stop_clears_movement_of_living_bots_only() {
        let c = cmd();
        assert_eq!(stopped(c, false, true), c);
        assert_eq!(stopped(c, true, false), c, "dead or observing bots still respawn");
        let s = stopped(c, true, true);
        assert_eq!((s.forward, s.side, s.up, s.buttons, s.impulse), (0.0, 0.0, 0.0, 0, 0));
        assert_eq!((s.angles, s.msec), (c.angles, c.msec), "it still turns");
    }

    #[test]
    fn usercmd_checks() {
        assert!(valid_cmd(&cmd()));
        assert!(!valid_cmd(&BotCmd { msec: 0, ..cmd() }));
        assert!(!valid_cmd(&BotCmd { msec: 256, ..cmd() }));
        assert!(!valid_cmd(&BotCmd { impulse: -1, ..cmd() }));
        assert!(!valid_cmd(&BotCmd { forward: f32::NAN, ..cmd() }));
        assert!(!valid_cmd(&BotCmd { angles: [0.0, f32::INFINITY, 0.0], ..cmd() }));
    }

    #[test]
    fn buy_checks() {
        ts30_gameplay::tables::make_default();
        use shared::weapons::id;
        assert!(valid_buy(id::M4 as c_int, 0, 4));
        assert!(valid_buy(id::BERETTA as c_int, 3, 0));
        assert!(!valid_buy(0, 0, 0), "kung fu isn't bought");
        assert!(!valid_buy(38, 0, 0));
        assert!(!valid_buy(-1, 0, 0));
        assert!(!valid_buy(id::M4 as c_int, -1, 0));
        assert!(!valid_buy(id::M4 as c_int, 0, 256));
    }
}
