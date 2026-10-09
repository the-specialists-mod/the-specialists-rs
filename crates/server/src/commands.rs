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

pub fn client_command(e: *mut edict_t) {
    let args = Args;
    let cmd = args.str(0).to_ascii_lowercase();
    let handled = world::dispatch(e, |ent, cx| {
        let Some(p) = (ent as &mut dyn std::any::Any).downcast_mut::<Player>() else { return false };
        dispatch_player(p, cx, &cmd, &args)
    });
    if handled != Some(true) {
        engine().client_print(e, PRINT_TYPE::print_console, &format!("Unknown command: {cmd}\n"));
    }
}

fn dispatch_player(p: &mut Player, cx: &mut Cx, cmd: &str, args: &Args) -> bool {
    crate::weapons::client_command(p, cx, cmd, args)
        || crate::stunt::client_command(p, cx, cmd, args)
        || crate::rules::client_command(p, cx, cmd, args)
        || crate::bots::client_command(p, cx, cmd, args)
}
