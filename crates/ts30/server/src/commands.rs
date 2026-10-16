//! TS 3.0's client commands where a ruleset decides: the chat (`Host_Say` 0xad02c) and `kill`
//! (`ClientKill` 0xacec8).

use ts_server::player::Player;
use ts_server::rules::commands::SayRules;
use ts_server::rules::state::{self, Mode};
use ts_server::world::Cx;

/// `Host_Say` (0xad02c): `%a`/`%h` expand; only a vote's text is lowered (`strlwr(text + 4)`,
/// after the `vote` match); no `ff` reply.
pub const SAY_RULES: SayRules = SayRules { expand_codes: true, lower_all: false, ff_reply: false };

/// `Host_Say`'s dead-chat gate (0xad02c): `1 < (byte)(mode - 2) && g_lastManStanding == 0`
/// leaves it open, so it is on in the round modes 2/3 (unreachable in 3.0: `InitObjectives`
/// 0x785d8 only keeps a mode that is already 2/3) or with LMS.
pub fn say_dead_chat_gated() -> bool {
    let (mode, _, lms, _) = state::mode();
    dead_chat_gated(mode, lms)
}

fn dead_chat_gated(mode: Mode, lms: bool) -> bool {
    matches!(mode, Mode::RoundTdm | Mode::Objective) || lms
}

/// `ClientKill` (0xacec8): the suicide one second later (`pl+0x84c = time + 1` while it is
/// `<= 0.001`; `PreThink`'s end, `lifecycle::pre_think_end`, kills), not while observing.
pub fn client_kill(p: &mut Player, cx: &mut Cx) {
    let now = cx.time();
    if cx.pev().iuser1 == 0 && p.ts.suicide_at <= 0.001 {
        p.ts.suicide_at = now + 1.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn say_rules_and_dead_chat_gate() {
        assert_eq!(SAY_RULES, SayRules { expand_codes: true, lower_all: false, ff_reply: false });
        assert!(!dead_chat_gated(Mode::Dm, false));
        assert!(!dead_chat_gated(Mode::TeamDm, false));
        assert!(dead_chat_gated(Mode::Dm, true));
        assert!(dead_chat_gated(Mode::RoundTdm, false));
        assert!(dead_chat_gated(Mode::Objective, false));
    }
}
