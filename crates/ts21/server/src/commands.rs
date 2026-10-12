//! TS 2.1's client commands where they differ from 3.0's (`pfnClientCommand` 0x1000d4b0, decomp
//! `docs/ts21/rules.md` §10).

use shared::protocol::hud_wire::PwUp;
use shared::ts::powerup as pw;
use ts_server::commands::Args;
use ts_server::messages::Msg;
use ts_server::player::Player;
use ts_server::rules::lifecycle::give_cash;
use ts_server::rules::{alive, cvars, msg};
use ts_server::world::Cx;

/// What 2.1 does with a client command: `Some(handled)` when it differs from 3.0, `None` for
/// 3.0's handling.
pub fn client_command(p: &mut Player, cx: &mut Cx, cmd: &str, args: &Args) -> Option<bool> {
    match cmd {
        // Cheat: `GiveNamedItem(argv(1))`.
        "give" => {
            if !cvars::cheats() {
                return Some(false);
            }
            let class = args.argv(1).to_owned();
            let e = cx.edict;
            ts_server::world::lend(e, p, || ts_server::entities::stock::maprules::give_named_item(e, &class));
            Some(true)
        }
        // Cheat: 4 is a stored infinite clip (3.0: `MultiplyClips`); the rest as 3.0.
        "powerup" if cvars::cheats() && args.argc() > 1 => {
            let n = args.int(1);
            if n == 4 {
                store_powerup(p, cx, pw::INFINITE_CLIP);
            } else {
                ts_server::rules::commands::cheat_powerup(p, cx, n);
            }
            Some(true)
        }
        "cashala" if cvars::cheats() => {
            give_cash(p, 10000);
            Some(true)
        }
        // Not in 2.1: observers respawn with +attack; no MOTD or full-update request; no laser
        // colour.
        "respawn" | "motd" | "fullupdate" => Some(false),
        c if c.starts_with("lasclr_") => Some(false),
        _ => None,
    }
}

/// The cheat's stored power-up: only with nothing active; 5 more seconds of the same type, or 5
/// seconds of a new one.
fn store_powerup(p: &mut Player, cx: &mut Cx, bits: u16) {
    if !alive(cx.pev()) || cx.pev().iuser1 != 0 || p.active_powerup != 0 {
        return;
    }
    msg::one(Msg::PwUp, cx.edict, |w| PwUp { bits: bits as i16, secs: 0 }.write(w));
    p.ts.held_secs = if p.ts.held == bits { p.ts.held_secs + 5 } else { 5 };
    p.ts.held = bits;
    p.powerup_until = 0.0;
}

/// A TS server command (`globalslow`, `restartround`, `weaponrestriction`): a polled cvar in 2.1.
pub fn server_command(name: &str) -> bool {
    crate::cvars::server_command(name)
}
