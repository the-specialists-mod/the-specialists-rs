//! Client command dispatch (`pfnClientCommand`). Each subsystem gets a chance to handle the
//! command for the player who sent it; the first that returns true wins.

use crate::player::Player;
use crate::world::{self, Cx};
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::ffi::CStr;

/// The current command's arguments (`argv(0)` is the command name).
pub struct Args;

impl Args {
    pub fn argc(&self) -> i32 {
        engine().cmd_argc()
    }
    pub fn argv(&self, i: i32) -> &'static CStr {
        engine().cmd_argv(i)
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
    /// Everything after the command name, as the client typed it.
    pub fn rest(&self) -> &'static CStr {
        engine().cmd_args()
    }
}

/// `ClientCommand` (0xad838). Command names are matched as typed: TS compares them with `strcmp`
/// and `strstr(..) == argv(0)` prefixes, so `Say` or `WEAPON_13` are unknown commands. Only the
/// voice manager's `vban`/`VModEnable` use `strcasecmp`. Without private data (no `Player`) TS
/// returns at once.
pub fn client_command(e: *mut edict_t) {
    let args = Args;
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
