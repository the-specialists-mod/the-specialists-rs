//! TS 2.1's per-player rules: spawn, observer, join, death, respawn timing, overheal and The One
//! (decomp `docs/ts21/rules.md` §3.3, §5.1, §6).

use crate::cash;
use crate::cvars as cv;
use crate::rules::{MERCENARY, SPECIALISTS};
use goldsrc::server::engine;
use shared::protocol::hud_wire::PwUp;
use shared::ts::powerup as pw;
use ts_server::messages::Msg;
use ts_server::player::Player;
use ts_server::rules::msg::{self, ts_message_all};
use ts_server::rules::state::{self, Mode};
use ts_server::rules::{alive, cvars};
use ts_server::world::Cx;

/// The rules part of `TSInit` (0x100552f0) that 2.1 adds: the cash floor, and in the round
/// modes a stored slow motion of 3 s at every spawn.
pub fn on_spawn(p: &mut Player) {
    cash::cash_floor(p);
    if state::with(|g| matches!(g.mode, Mode::RoundTdm | Mode::Objective)) {
        p.ts.held = pw::SLOWMO;
        p.ts.held_secs = 3;
    }
}

/// The end of `CTSGameRules::InitHUD` (0x100451c0): after `StartObserver`, the joiner's cash is
/// set to `(int)initial_cash` (not given: 2.1's `ClientPutInServer` gives none).
pub fn on_init_hud(p: &mut Player) {
    p.cash = cv::initial_cash_int();
    p.hud.cash = None;
}

/// `SetPlayerRespawnTime(p, secs, force)` (0x10042d50) without `force` (`Killed`, `InitHUD`).
/// Mode 3 limits respawns to `ts_mapglobals` `playerrespawn` per round (0 = none);
/// mode 2 and LMS (DM with `lastmanstanding` on, read live) to `respawnnumber` (at least 1);
/// a limited respawn waits at most for the round's end, and the join deadline is 25 s at most.
/// The count is the deaths this round: `CBasePlayer::Killed` adds the current one (`+0x732`)
/// before it calls this, so both limits are lives, not respawns: `respawnnumber 1` (the default)
/// or `playerrespawn 1` leaves a killed player out until the next round, 2 gives one respawn.
pub fn set_respawn_time(p: &mut Player, cx: &mut Cx, secs: f32) {
    set_respawn_time_with(p, cx, secs, false);
}

/// `SetPlayerRespawnTime` with its `force` argument, which only the team change sets: in mode 2,
/// and in mode 3 with a `playerrespawn` limit, the player waits for the next round.
pub fn set_respawn_time_with(p: &mut Player, cx: &mut Cx, secs: f32, force: bool) {
    let now = cx.time();
    let (mode, round_end) = state::with(|g| (g.mode, g.round_end));
    let per_round = crate::rules::with(|r| r.respawns_per_round);
    let t = respawn_time(
        mode,
        cvars::on(c"lastmanstanding"),
        per_round,
        cv::respawn_number(),
        p.ts.respawns_used as i8,
        secs.trunc(),
        now,
        round_end,
    );
    let t = if force && force_waits(mode, per_round) { Respawn::Never } else { t };
    match t {
        Respawn::At(at) => p.respawn_at = at,
        Respawn::Limited { at, deadline } => {
            p.respawn_at = at;
            p.ts.join_deadline = deadline;
        }
        Respawn::Never => p.respawn_at = -1.0,
    }
    p.ts.award = if p.respawn_at < 0.0 { 1 } else { 11 };
}

/// Where `SetPlayerRespawnTime`'s `force` sends the player to the next round (`respawn = -1`):
/// mode 2 (its `force` test sits next to the `respawnnumber` one) and mode 3 when `playerrespawn`
/// limits the respawns. Mode 3 without a limit and LMS DM ignore it.
fn force_waits(mode: Mode, per_round: u8) -> bool {
    match mode {
        Mode::RoundTdm => true,
        Mode::Objective => per_round != 0,
        _ => false,
    }
}

/// The 2.1 tail of `ChangePlayerTeam` (0x10038e99..0x10038ef3): a change that isn't a kill puts
/// the player in observer with `SetPlayerRespawnTime(5, force)`, in The One mode too; then,
/// kill or not, the cash is set to `(int)initial_cash` and gets the floor (`FUN_10042c90`).
pub fn change_team_tail(p: &mut Player, cx: &mut Cx, kill: bool) {
    if !kill {
        let (o, a) = (cx.pev().origin, cx.pev().v_angle);
        ts_server::rules::lifecycle::start_observer(p, cx, o, a);
        set_respawn_time_with(p, cx, 5.0, true);
    }
    p.cash = cv::initial_cash_int();
    cash::cash_floor(p);
}

/// When a player may respawn.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Respawn {
    At(f32),
    Limited { at: f32, deadline: f32 },
    Never,
}

/// The decision of `SetPlayerRespawnTime` (0x10042d50). `used` is the signed byte of respawns used
/// this round (`+0x732`).
#[allow(clippy::too_many_arguments)] // the inputs TS reads, each from its own global or field
pub fn respawn_time(mode: Mode, lms: bool, per_round: u8, respawn_number: f32, used: i8, secs: f32, now: f32, round_end: f32) -> Respawn {
    let limited = || {
        let left = round_end - now;
        let at = (if secs <= left { secs } else { left }) + now;
        let deadline = (if 25.0 <= left { 25.0 } else { left }) + now;
        Respawn::Limited { at, deadline }
    };
    match mode {
        Mode::Objective => {
            if per_round == 0 {
                Respawn::At(secs + now)
            } else if per_round as i8 <= used {
                Respawn::Never
            } else {
                limited()
            }
        }
        Mode::RoundTdm => {
            if (used as f32) < respawn_number {
                limited()
            } else {
                Respawn::Never
            }
        }
        _ if !lms || mode != Mode::Dm => Respawn::At(secs + now),
        _ if (used as f32) < respawn_number => limited(),
        _ => Respawn::Never,
    }
}

/// The overheal in 2.1's `PreThink` (0x100537e0): health above 100 drops by 1 a second, The One
/// included (no regeneration).
pub fn health_regen(p: &mut Player, cx: &mut Cx, now: f32) {
    if p.ts.health_timer == 0.0 {
        return;
    }
    let v = cx.pev();
    if v.health <= 100.0 {
        p.ts.health_timer = 0.0;
    } else if p.ts.health_timer < now {
        p.ts.health_timer = now + 1.0;
        v.health -= 1.0;
    }
}

/// `CTSGameRules::SetPlayerTheOne` (0x100467c0): a living, non-observing player gets health 150,
/// kung fu and super jump, a stored 3 s slow pause when it has nothing, award 10 and the
/// announcement. No speed change and no log line (3.0 has both).
pub fn set_player_the_one(p: &mut Player, cx: &mut Cx) {
    if !alive(cx.pev()) || cx.pev().iuser1 != 0 {
        return;
    }
    let e = cx.edict;
    let now = cx.time();
    p.ts.is_the_one = true;
    cx.pev().health = 150.0;
    p.kungfu = true;
    p.superjump = true;
    p.ts.health_timer = now + 1.0;
    engine().set_physics_key(e, c"mj", c"1");
    msg::one(Msg::PwUp, e, |w| PwUp { bits: pw::KUNGFU as i16, secs: 0 }.write(w));
    msg::one(Msg::PwUp, e, |w| PwUp { bits: pw::SUPERJUMP as i16, secs: 0 }.write(w));
    if p.active_powerup == 0 && p.ts.held == 0 {
        p.active_powerup = 0;
        p.ts.held = pw::SLOWPAUSE;
        p.powerup_until = 0.0;
        p.ts.held_secs = 3;
    }
    p.ts.award = 10;
    state::with(|g| g.the_one_exists = true);
    ts_message_all([100, 150, 255], 1, 30, &shared::bformat!("{} is The One!", ts_server::rules::netname(e)));
}

/// The team `SetDefaultPlayerTeam` (`FUN_10038920`) reads: in the round modes the player's own
/// team stays; in The One mode `mp_theone`'s or `mp_theothers`' model, which also becomes the
/// `model` and `team` keys; otherwise the `model` key (the SDK's model teams).
pub fn default_team(p: &mut Player, cx: &mut Cx) -> String {
    let e = cx.edict;
    let (mode, the_one) = state::with(|g| (g.mode, g.the_one));
    if matches!(mode, Mode::RoundTdm | Mode::Objective) {
        return p.ts.team_name.clone();
    }
    let eng = engine();
    if the_one {
        let name = cvars::s(if p.ts.is_the_one { c"mp_theone" } else { c"mp_theothers" });
        let name: String = name.chars().take(15).collect();
        eng.set_client_key(e, "model", &name);
        eng.set_client_key(e, "team", &name);
        return name;
    }
    eng.info_key(e, c"model").to_string_lossy().into_owned()
}

/// Whether a team name is one of the round modes' (for logs and tests).
pub fn is_round_team(name: &str) -> bool {
    name == SPECIALISTS || name == MERCENARY
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn respawn_in_deathmatch_waits_the_time() {
        assert_eq!(respawn_time(Mode::Dm, false, 0, 1.0, 5, 5.0, 100.0, 0.0), Respawn::At(105.0));
        assert_eq!(respawn_time(Mode::TeamDm, true, 0, 1.0, 5, 5.0, 100.0, 0.0), Respawn::At(105.0), "LMS is DM only");
    }

    #[test]
    fn lms_and_round_tdm_count_respawns() {
        // `used` is the deaths this round, the current one included (`Killed` counts it first):
        // with respawnnumber 1 the first death is out, with 2 it waits and the second is out.
        // (0 is `InitHUD`'s call, before any death.)
        assert_eq!(respawn_time(Mode::Dm, true, 0, 1.0, 0, 5.0, 100.0, 200.0), Respawn::Limited { at: 105.0, deadline: 125.0 });
        assert_eq!(respawn_time(Mode::Dm, true, 0, 1.0, 1, 5.0, 100.0, 200.0), Respawn::Never);
        assert_eq!(respawn_time(Mode::RoundTdm, false, 0, 2.0, 1, 5.0, 100.0, 103.0), Respawn::Limited { at: 103.0, deadline: 103.0 });
        assert_eq!(respawn_time(Mode::RoundTdm, false, 0, 1.0, 1, 5.0, 100.0, 200.0), Respawn::Never);
    }

    #[test]
    fn objective_rounds_follow_playerrespawn() {
        assert_eq!(respawn_time(Mode::Objective, false, 0, 1.0, 9, 5.0, 100.0, 200.0), Respawn::At(105.0), "0 = unlimited");
        assert_eq!(respawn_time(Mode::Objective, false, 2, 1.0, 1, 5.0, 100.0, 200.0), Respawn::Limited { at: 105.0, deadline: 125.0 });
        assert_eq!(respawn_time(Mode::Objective, false, 2, 1.0, 2, 5.0, 100.0, 200.0), Respawn::Never);
    }

    #[test]
    fn a_team_change_sits_out_the_round() {
        // Issue #142: the team change's `force` waits for the next round in mode 2, and in mode
        // 3 with a `playerrespawn` limit; elsewhere the plain respawn time stands.
        assert!(force_waits(Mode::RoundTdm, 0));
        assert!(force_waits(Mode::Objective, 2));
        assert!(!force_waits(Mode::Objective, 0));
        assert!(!force_waits(Mode::Dm, 0), "LMS DM ignores force");
        assert!(!force_waits(Mode::TeamDm, 0));
    }

    #[test]
    fn a_team_change_resets_the_cash_to_initial_cash() {
        // `m_iCash = (int)initial_cash`, then the floor: 9000 becomes 3000 (round modes then take
        // the team's cash when more).
        assert_eq!(cash::floor(cv::trunc(3000.0), 3000.0), 3000);
        assert_eq!(cash::floor(cv::trunc(800.9), 800.9), 800);
    }
}
