//! TS 2.1's client commands where they differ from 3.0's (`pfnClientCommand` 0x1000d4b0, decomp
//! `docs/ts21/rules.md` §10), the chat's text rules (`Host_Say` 0x1000cf00) and `kill`
//! (`pfnClientKill` 0x1000ce50).

use shared::protocol::hud_wire::PwUp;
use shared::ts::powerup as pw;
use ts_server::commands::Args;
use ts_server::messages::Msg;
use ts_server::player::Player;
use ts_server::rules::commands::SayRules;
use ts_server::rules::lifecycle::{self, give_cash};
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

/// A TS server command: `globalslow`, `restartround`, `weaponrestriction` are polled cvars in
/// 2.1; `loadweapons`, `saveweapons` don't exist ([`crate::customweapons::server_command`]).
pub fn server_command(name: &str) -> bool {
    crate::cvars::server_command(name) || crate::customweapons::server_command(name)
}

/// `Host_Say` (0x1000cf00): the text goes after the prefix as it is (no `%a`/`%h`, 0x1000d0dd),
/// the whole of it is lowered (`_strlwr` at 0x1000d0ec, after the line is built), and a text
/// starting with `ff` (`strncmp(text, "ff", 2)`, 0x1000d164) answers the sender instead of being
/// broadcast.
pub const SAY_RULES: SayRules = SayRules { expand_codes: false, lower_all: true, ff_reply: true };

/// `pfnClientKill` (0x1000ce50), the SDK's with TS's observer test: when not observing and
/// `m_fNextSuicideTime` (`+0x7a8`) has passed, it moves a second on and the player dies at once
/// (`health = 0`, `Killed(pev, GIB_NEVER)`). There is no alive test (as in the SDK), and no
/// delayed suicide in 2.1.
pub fn client_kill(p: &mut Player, cx: &mut Cx) {
    let now = cx.time();
    if kill_allowed(cx.pev().iuser1, p.ts.next_suicide, now) {
        p.ts.next_suicide = now + 1.0;
        lifecycle::suicide(p, cx);
    }
}

/// `pev->iuser1 == 0 && m_fNextSuicideTime <= time` (0x1000ce7a).
fn kill_allowed(iuser1: i32, next_suicide: f32, now: f32) -> bool {
    iuser1 == 0 && next_suicide <= now
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn say_rules_are_2_1s() {
        assert_eq!(SAY_RULES, SayRules { expand_codes: false, lower_all: true, ff_reply: true });
    }

    /// `pfnClientKill` (0x1000ce50): not while observing, and a second apart.
    #[test]
    fn kill_cooldown() {
        assert!(kill_allowed(0, 0.0, 5.0));
        assert!(kill_allowed(0, 5.0, 5.0), "`<=`: the second has passed");
        assert!(!kill_allowed(0, 5.5, 5.0));
        assert!(!kill_allowed(3, 0.0, 5.0), "observing");
    }
}
