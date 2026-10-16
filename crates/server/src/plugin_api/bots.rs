//! The plugin API's bot functions (version 3, issue #104): what a bot backend plugin needs to
//! make and drive its bots through the port's own code paths (docs/implementation/bot-plugins.md).
//! TS's bots don't use them: they are the port's native backend (`crate::bots`).

use super::{Snapshots, on_player, player_edict};
use crate::bot_backend;
use crate::commands::{self, Args};
use crate::rules::{self, cvars};
use crate::weapons::{self, ArmamentEntry, Drop};
use crate::world;
use goldsrc::Global;
use goldsrc::edict::EdictPtr;
use goldsrc::server::engine;
use goldsrc::sys::*;
use shared::weapons::{MAX_AMMO, info};
use std::cell::RefCell;
use std::ffi::{CStr, c_int};
use ts_plugin_api::{BUSY, BotCmd, BotCmdResult, DEFERRED, FULL, INVALID, NOT_PLAYER, OK, PluginId, cmd as cmd_flag, cmd_event, item};

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

/// `run_cmd`'s argument check: a usercmd's `msec` is a byte (0: the server's choice), the impulse
/// a byte, the flags known ones, and the floats finite.
pub fn valid_cmd(cmd: &BotCmd) -> bool {
    (0..=255).contains(&cmd.msec)
        && (0..=255).contains(&cmd.impulse)
        && cmd.flags & !(cmd_flag::SET_VIEW | cmd_flag::AUTO_MSEC) == 0
        && cmd.angles.iter().chain([&cmd.forward, &cmd.side, &cmd.up]).all(|v| v.is_finite())
}

/// What `run_cmd` keeps per bot between commands.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Moves {
    /// [`auto_msec`]'s carried fraction of a millisecond.
    pub carry: f64,
    /// Where the last command left it: origin, speed and game time.
    pub last: Option<([f32; 3], f32, f32)>,
}

static MOVES: Global<RefCell<Snapshots<Moves>>> = Global(RefCell::new(Snapshots(Vec::new())));

/// [`cmd_flag::AUTO_MSEC`]: the frame time in whole milliseconds (1..255) and the fraction left
/// over for the next command, so a bot's commands add up to the server's time. The carry stays
/// within a millisecond, so a long frame cut to 255 doesn't pile up.
pub fn auto_msec(frametime: f32, carry: f64) -> (u8, f64) {
    let total = f64::from(frametime.max(0.0)) * 1000.0 + carry;
    let ms = total.floor().clamp(1.0, 255.0);
    (ms as u8, (total - ms).clamp(-1.0, 1.0))
}

/// The view a [`cmd_flag::SET_VIEW`] command runs with: the game's (`pev->angles`, what a real
/// client gets with `svc_setangle`) when it set `fixangle`, else the command's. True when the
/// game's won.
pub fn view_for(cmd_angles: [f32; 3], fixangle: bool, game_angles: [f32; 3]) -> ([f32; 3], bool) {
    if fixangle { (game_angles, true) } else { (cmd_angles, false) }
}

/// `pev->angles` for view angles `view`, as `CBotBrain::UpdateView` (0x1006e500) and the engine's
/// `SV_FinishPMove` set them: a third of the pitch, upside down, no roll.
pub fn body_angles(view: [f32; 3]) -> [f32; 3] {
    [view[0] * -0.333_333_34, view[1], 0.0]
}

/// The bot moved from `from` to `to` in `secs` farther than its speed (the larger of before and
/// after) allows, with half again and 32 units of slack: a teleport, a respawn, `ts_tp`.
pub fn teleported(from: [f32; 3], to: [f32; 3], speed: f32, secs: f32) -> bool {
    let d = from.iter().zip(to).map(|(a, b)| (b - a) * (b - a)).sum::<f32>().sqrt();
    d > speed * secs.max(0.0) * 1.5 + 32.0
}

fn speed(v: [f32; 3]) -> f32 {
    v.iter().map(|x| x * x).sum::<f32>().sqrt()
}

/// `PluginApi::run_cmd`: `pfnRunPlayerMove` for a bot, outside its own callbacks, and what it did.
pub fn run_cmd(player: c_int, cmd: &BotCmd) -> Result<BotCmdResult, c_int> {
    let e = player_edict(player);
    if e.is_null() {
        return Err(NOT_PLAYER);
    }
    if !is_bot(e) || !valid_cmd(cmd) {
        return Err(INVALID);
    }
    // The engine runs the bot's PreThink and PostThink inside the call: its object must be free.
    if world::is_taken_out(e) {
        return Err(BUSY);
    }
    let eng = engine();
    let (now, frametime) = (eng.time(), eng.globals().frametime);
    let uid = eng.user_id(e);
    let mut moves = MOVES.0.borrow().get(player, uid).unwrap_or_default();
    let mut c = *cmd;
    if c.flags & cmd_flag::AUTO_MSEC != 0 || c.msec == 0 {
        let (ms, carry) = auto_msec(frametime, moves.carry);
        (c.msec, moves.carry) = (ms as c_int, carry);
    }
    let mut events = 0;
    let (before, vel_before, ground_before) = {
        let v = e.v();
        let playing = rules::alive(v) && v.iuser1 == 0;
        c = stopped(c, cvars::f(c"ts_bot_stop") != 0.0, playing);
        if v.fixangle != 0 {
            events |= cmd_event::FIXANGLE;
        }
        if c.flags & cmd_flag::SET_VIEW != 0 {
            // `UpdateView`'s writes, and the `fixangle` only a real client's update clears.
            let (view, game) = view_for(c.angles, v.fixangle != 0, v.angles);
            if game {
                c.angles = view;
            }
            v.v_angle = view;
            v.angles = body_angles(view);
            v.fixangle = 0;
        }
        v.button = c.buttons;
        (v.origin, v.velocity, v.flags & FL_ONGROUND as i32 != 0)
    };
    // Between commands only pushers move a player (its own movement runs in its commands), so a
    // long pause (a dead bot, a frozen backend) allows no more than a tenth of a second's.
    if let Some((origin, last_speed, at)) = moves.last
        && teleported(origin, before, last_speed.max(speed(vel_before)), (now - at).min(0.1))
    {
        events |= cmd_event::TELEPORTED;
    }
    eng.run_player_move(e, c.angles, c.forward, c.side, c.up, c.buttons as u16, c.impulse as u8, c.msec as u8);
    // The engine leaves the globals at the command's end (`SV_RunCmd`); the rest of the frame
    // runs at the server's.
    eng.set_frame_times(now, frametime);
    let v = e.v();
    if v.fixangle != 0 {
        events |= cmd_event::FIXANGLE;
    }
    if teleported(before, v.origin, speed(vel_before).max(speed(v.velocity)), c.msec as f32 / 1000.0) {
        events |= cmd_event::TELEPORTED;
    }
    if !ground_before && v.flags & FL_ONGROUND as i32 != 0 {
        events |= cmd_event::LANDED;
    }
    let out = BotCmdResult {
        // A view the game set during the command (a spawn, a teleport touch) is `pev->angles`,
        // which `v_angle` follows only once `fixangle` is cleared.
        v_angle: if v.fixangle != 0 { v.angles } else { v.v_angle },
        origin: v.origin,
        velocity: v.velocity,
        events,
        msec: c.msec,
    };
    moves.last = Some((v.origin, speed(v.velocity), now));
    MOVES.0.borrow_mut().put(player, uid, moves);
    Ok(out)
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
        BotCmd { angles: [1.0, 2.0, 0.0], forward: 200.0, side: -50.0, up: 10.0, buttons: 2, impulse: 100, msec: 16, flags: 0 }
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
        assert!(valid_cmd(&BotCmd { msec: 0, ..cmd() }), "the server's msec");
        assert!(valid_cmd(&BotCmd { flags: cmd_flag::SET_VIEW | cmd_flag::AUTO_MSEC, ..cmd() }));
        assert!(!valid_cmd(&BotCmd { flags: 0x4, ..cmd() }), "an unknown flag");
        assert!(!valid_cmd(&BotCmd { msec: -1, ..cmd() }));
        assert!(!valid_cmd(&BotCmd { msec: 256, ..cmd() }));
        assert!(!valid_cmd(&BotCmd { impulse: -1, ..cmd() }));
        assert!(!valid_cmd(&BotCmd { forward: f32::NAN, ..cmd() }));
        assert!(!valid_cmd(&BotCmd { angles: [0.0, f32::INFINITY, 0.0], ..cmd() }));
    }

    #[test]
    fn auto_msec_carries_the_fraction() {
        // A 60 fps server: 16.666 ms frames come out 16, 16, 17, ... and add up.
        let (mut carry, mut sum) = (0.0, 0u32);
        for _ in 0..60 {
            let (ms, c) = auto_msec(1.0 / 60.0, carry);
            assert!(ms == 16 || ms == 17);
            (carry, sum) = (c, sum + u32::from(ms));
        }
        assert!((999..=1000).contains(&sum), "{sum}");
        assert_eq!(auto_msec(0.0001, 0.0).0, 1, "never 0: a command must run");
        let (ms, carry) = auto_msec(2.0, 0.0);
        assert_eq!(ms, 255, "a byte");
        assert_eq!(carry, 1.0, "a long frame's rest isn't kept beyond a millisecond");
    }

    #[test]
    fn set_view_takes_the_games_angles_after_fixangle() {
        assert_eq!(view_for([10.0, 20.0, 0.0], false, [0.0, 90.0, 0.0]), ([10.0, 20.0, 0.0], false));
        assert_eq!(view_for([10.0, 20.0, 0.0], true, [0.0, 90.0, 0.0]), ([0.0, 90.0, 0.0], true), "a spawn's or teleport's view");
        // As `UpdateView`: pitch × -1/3 (mp.dll's float constant), no roll.
        assert_eq!(body_angles([30.0, 45.0, 5.0]), [30.0 * -0.333_333_34, 45.0, 0.0]);
    }

    #[test]
    fn teleports_are_moves_beyond_the_speed() {
        let o = [0.0, 0.0, 0.0];
        assert!(!teleported(o, [4.0, 0.0, 0.0], 250.0, 0.016), "a step");
        assert!(!teleported(o, [30.0, 0.0, 0.0], 0.0, 0.016), "within the slack (a step up, a push)");
        assert!(!teleported(o, [0.0, 0.0, -40.0], 2000.0, 0.016), "a fall at sv_maxvelocity");
        assert!(teleported(o, [500.0, 0.0, 0.0], 250.0, 0.016), "a teleport");
        assert!(teleported(o, [0.0, 40.0, 0.0], 0.0, 0.016), "a respawn next to the death");
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
