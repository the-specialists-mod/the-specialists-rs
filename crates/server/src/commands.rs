//! Client command dispatch (`pfnClientCommand`). Each subsystem gets a chance to handle the
//! command for the player who sent it; the first that returns true wins.

use crate::player::Player;
use crate::world::{self, Cx};
use goldsrc::edict::EdictPtr;
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::ffi::CStr;

/// The current command's arguments (`argv(0)` is the command name). `skip` drops leading engine
/// arguments: `ts_as <player#> <command> ...` runs `<command>` with `skip` 2.
pub struct Args {
    skip: i32,
}

impl Args {
    pub fn argc(&self) -> i32 {
        (engine().cmd_argc() - self.skip).max(0)
    }
    pub fn argv(&self, i: i32) -> &'static CStr {
        engine().cmd_argv(i + self.skip)
    }
    pub fn str(&self, i: i32) -> &'static str {
        self.argv(i).to_str().unwrap_or("")
    }
    pub fn int(&self, i: i32) -> i32 {
        self.str(i).trim().parse().unwrap_or(0)
    }
    pub fn float(&self, i: i32) -> f32 {
        self.str(i).trim().parse().unwrap_or(0.0)
    }
    /// Everything after the command name, as the client typed it (for `ts_as`, everything after
    /// `ts_as`).
    pub fn rest(&self) -> &'static CStr {
        engine().cmd_args()
    }
}

/// `ts_as <player#> <command> [args...]` (sv_cheats, port-only dev aid): runs a client command
/// as player `#`, e.g. `ts_as 2 ts_give 2_2` or `ts_as 2 useitems` for a bot. Unknown commands
/// are ignored.
pub extern "C" fn cmd_as() {
    let eng = engine();
    if eng.cvar_float(c"sv_cheats") == 0.0 || eng.cmd_argc() < 3 {
        eng.print("usage (sv_cheats 1): ts_as <player#> <command> [args...]\n");
        return;
    }
    let e = eng.client(eng.cmd_argv(1).to_string_lossy().trim().parse().unwrap_or(0));
    if e.is_null() || !e.has_private_data() {
        return;
    }
    let args = Args { skip: 2 };
    let cmd = args.str(0);
    world::dispatch(e, |ent, cx| {
        let Some(p) = (ent as &mut dyn std::any::Any).downcast_mut::<Player>() else { return false };
        dispatch_player(p, cx, cmd, &args)
    });
}

/// `ClientCommand` (0xad838). Command names are matched as typed: TS compares them with `strcmp`
/// and `strstr(..) == argv(0)` prefixes, so `Say` or `WEAPON_13` are unknown commands. Only the
/// voice manager's `vban`/`VModEnable` use `strcasecmp`. Without private data (no `Player`) TS
/// returns at once.
pub fn client_command(e: *mut edict_t) {
    let args = Args { skip: 0 };
    let cmd = args.str(0);
    let handled = world::dispatch(e, |ent, cx| {
        let Some(p) = (ent as &mut dyn std::any::Any).downcast_mut::<Player>() else { return false };
        dispatch_player(p, cx, cmd, &args)
    });
    if handled == Some(false) {
        // ClientCommand's tail (0xae5de): `argv(0)` as typed, cut to 127 bytes, through
        // `ClientPrint(HUD_PRINTCONSOLE)` (a `TextMsg`). The TS client sends `_special` on every
        // +attack2, and TS answers it with this too.
        let typed = args.argv(0).to_bytes();
        let typed = String::from_utf8_lossy(&typed[..typed.len().min(127)]).into_owned();
        crate::rules::msg::text_one(e, shared::protocol::rules::hud_dest::CONSOLE, &format!("Unknown command: {typed}\n"));
    }
}

fn dispatch_player(p: &mut Player, cx: &mut Cx, cmd: &str, args: &Args) -> bool {
    crate::weapons::client_command(p, cx, cmd, args)
        || crate::stunt::client_command(p, cx, cmd, args)
        || crate::rules::client_command(p, cx, cmd, args)
        || crate::bots::client_command(p, cx, cmd, args)
}
