//! `StartFrame` → `CHalfLifeMultiplay::Think` (0xcf038, time/frag limits, intermission) and
//! `CTSGameRules::Think` (0x77204: global slow, auto power-ups, The One, LMS rounds, SrvSett).

use super::msg::{self, ts_message_all};
use super::powerup::go_slow;
use super::state::{self, Mode};
use super::{cvars, votes};
use crate::messages::{self, Msg};
use goldsrc::Global;
use goldsrc::edict::EdictPtr;
use goldsrc::ext::rules::SVC_INTERMISSION;
use goldsrc::server::{MsgWriter, engine};
use goldsrc::sys::*;
use shared::protocol::rules::{RoundTime, TeamScore};
use std::cell::Cell;

pub fn think() {
    // `CHalfLifeRules::Think` (0xde730) is empty: in single player neither the multiplayer nor
    // the TS think runs.
    if !state::with(|g| g.installed && !g.single_player) {
        return;
    }
    let now = engine().time();
    if state::teamplay() { teamplay_think(now) } else { multiplay_think(now) }
}

/// `last_frags`/`last_time` statics of `CHalfLifeMultiplay::Think` (0x1911c0) and
/// `CHalfLifeTeamplay::Think` (0x19e740): `.bss`, so 0 at load and never reset by a map change.
static LAST_LIMITS: Global<Cell<[[i32; 2]; 2]>> = Global(Cell::new([[0; 2]; 2]));

/// The `mp_fragsleft` / `mp_timeleft` update shared by both Thinks. `mp_timeleft` is rewritten
/// when the last frame's value differs from the cvar (TS compares `last_time` with
/// `timeleft.value`, not with the new value), so it follows one frame late.
fn update_limit_cvars(which: usize, frags_left: i32, time_left: i32) {
    let mut all = LAST_LIMITS.0.get();
    let [last_frags, last_time] = all[which];
    if frags_left != last_frags {
        set_cvar_direct(c"mp_fragsleft", frags_left);
    }
    if last_time as f32 != cvars::f(c"mp_timeleft") {
        set_cvar_direct(c"mp_timeleft", time_left);
    }
    all[which] = [frags_left, time_left];
    LAST_LIMITS.0.set(all);
}

/// The time `mp_timelimit` is checked against: TS's `gpGlobals->time`, which a map start
/// restarts, in extended precision as TS's x87 code holds it.
fn game_clock(now: f32) -> f64 {
    now as f64
}

/// Both Thinks' `mp_timelimit` test: `(mp_timeleft, over)`. `limit × 60` stays in an x87
/// register (exact in `f64`), the time left is a truncating `(int)`, and a limit of 0 is off.
fn time_limit(limit_minutes: f32, clock: f64) -> (i32, bool) {
    let limit = limit_minutes as f64 * 60.0;
    if limit == 0.0 {
        return (0, false);
    }
    (super::x87_trunc(limit - clock), limit <= clock)
}

/// `CHalfLifeMultiplay::Think` (0xcf038): voice, intermission, `mp_timelimit`, `mp_fraglimit`
/// (every slot's `pev->frags`, players who left included), the limit cvars, `CTSGameRules::Think`
/// and a passed map vote. `mp_timelimit × 60` stays in an x87 register (extended, so exact in
/// `f64`), and the time and frags left are `(int)` casts: truncated (0xcf1a8, 0xcf252).
fn multiplay_think(now: f32) {
    super::voice::update(engine().globals().frametime as f64);
    if state::with(|g| g.game_over) {
        intermission_think(now);
        return;
    }
    let (time_left, over) = time_limit(cvars::f(c"mp_timelimit"), game_clock(now));
    if over {
        go_to_intermission();
        return;
    }
    let fraglimit = cvars::f(c"mp_fraglimit");
    let mut frags_left = 0;
    if fraglimit != 0.0 {
        frags_left = 9999;
        for e in super::slot_players() {
            let f = e.v().frags;
            if fraglimit <= f {
                go_to_intermission();
                return;
            }
            frags_left = frags_left.min(super::x87_trunc(fraglimit as f64 - f as f64));
        }
    }
    update_limit_cvars(0, frags_left, time_left);
    crate::gameplay::active().server.think(now);
    if votes::vote_passed() {
        go_to_intermission();
    }
}

/// `CHalfLifeTeamplay::Think` (0xe412c). In intermission it runs `CHalfLifeMultiplay::Think`,
/// so the voice manager is updated twice a frame then. The fraglimit reads `team_scores` as the
/// last `RecountTeams` left them (kills, team changes, spawns); in CTB it reads the four
/// `TSGameRules` teams' captures, and every team, named or not, lowers `mp_fragsleft`.
/// The time and frags left are truncated as in the Multiplay `Think` (0xe41d1, 0xe428a, 0xe42e8).
fn teamplay_think(now: f32) {
    super::voice::update(engine().globals().frametime as f64);
    if state::with(|g| g.game_over) {
        multiplay_think(now);
        return;
    }
    let (time_left, over) = time_limit(cvars::f(c"mp_timelimit"), game_clock(now));
    if over {
        go_to_intermission();
        return;
    }
    let fraglimit = cvars::f(c"mp_fraglimit");
    let mut frags_left = 0;
    if fraglimit != 0.0 {
        frags_left = 9999;
        let ctb = state::with(|g| g.is_ctb());
        let scores: Vec<(bool, i32)> = state::with(|g| {
            if ctb {
                g.teams.iter().map(|t| (!t.name.is_empty(), t.score)).collect()
            } else {
                g.team_scores().iter().map(|&s| (true, s)).collect()
            }
        });
        for (counts, s) in scores {
            if counts && fraglimit <= s as f32 {
                go_to_intermission();
                return;
            }
            frags_left = frags_left.min(super::x87_trunc(fraglimit as f64 - s as f64));
        }
    }
    update_limit_cvars(1, frags_left, time_left);
    crate::gameplay::active().server.think(now);
    if votes::vote_passed() {
        go_to_intermission();
    }
}

/// The game-over part of `CHalfLifeMultiplay::Think`: `mp_chattime` kept in 10..120 (compared
/// as `(int)`, truncated: 0xcf0ac), then `ChangeLevel` once the chat time is over and a player pressed a button
/// (`PlayerThink`), or 120 s into the intermission. TS calls it every frame from then on; the
/// engine runs only the first `changelevel` of a map (`last_spawncount` in both FWGS and HLDS).
fn intermission_think(now: f32) {
    let eng = engine();
    let chat = super::x87_trunc(cvars::f(c"mp_chattime") as f64);
    if chat < 10 {
        eng.set_cvar_string(c"mp_chattime", "10");
    } else if chat > 120 {
        eng.set_cvar_string(c"mp_chattime", "120");
    }
    let start = state::with(|g| g.intermission_start);
    let end = start + cvars::f(c"mp_chattime");
    // `m_flIntermissionEndTime` (`+4`), rewritten every frame.
    let go = state::with(|g| {
        g.intermission_end = end;
        now > end && (g.end_intermission_button_hit || now > start + 120.0)
    });
    if go {
        super::mapcycle::change_level();
    }
}

fn set_cvar_direct(name: &std::ffi::CStr, v: i32) {
    let eng = engine();
    eng.cvar_direct_set_by_name(name, &v.to_string());
}

/// `GoToIntermission`: freeze everyone and show the scoreboard; change level after mp_chattime.
pub fn go_to_intermission() {
    let now = engine().time();
    let already = state::with(|g| std::mem::replace(&mut g.game_over, true));
    if already {
        return;
    }
    state::with(|g| {
        g.intermission_start = now;
        g.intermission_end = 0.0;
    });
    MsgWriter::begin(MSG_ALL as i32, SVC_INTERMISSION, None, std::ptr::null_mut()).end();
    engine().print("[ts] intermission\n");
}

/// An x87 `fcom` tested with `test ah,0x40` (C3): equal, or unordered when either side is NaN.
pub fn x87_eq(a: f64, b: f64) -> bool {
    a == b || a.is_nan() || b.is_nan()
}

/// A `RoundTime` byte as TS computes it: `(int)` of the extended-precision `m_flRoundEnd - time`
/// (truncated; NaN or out of range the integer indefinite), of which `WriteByte` sends the low byte.
pub fn round_time_byte(secs: f64) -> u8 {
    let i = if (-2_147_483_648.0..2_147_483_648.0).contains(&secs) { secs as i32 } else { i32::MIN };
    i as u8
}

/// `fVar < 0 ? 0 : fVar` as `RestartRound` (0x779b6) and `InitHUD` (0x79375) compile it: `fldz;
/// fcom st(1); jbe`, so a NaN is kept.
pub fn x87_max0(v: f64) -> f64 {
    if v < 0.0 { 0.0 } else { v }
}

/// `(int)ammocount` as `CTSGameRules::Think` (0x776de) and `InitHUD` (0x792c2) take it: a C cast
/// under GCC's truncating control word, so 2.9 is 2 and NaN or out of range the x87 "integer
/// indefinite" (which, like 0 or 5, `Set_ammo_count` maps to the default column).
pub fn ammocount() -> i32 {
    crate::entities::stock::maprules::x87_int(cvars::f(c"ammocount"))
}

/// `SetGlobalSlowMotion(amount, duration)` (`globalslow`, `ts_slowmotion` flag 2).
pub fn set_global_slow(amount: f32, duration: f32) {
    let now = engine().time();
    let cur = state::with(|g| g.global_slow.amount);
    if amount == cur {
        return;
    }
    if amount == 0.0 {
        state::with(|g| g.global_slow.active = false);
        return;
    }
    global_slow2(0.0, false);
    state::with(|g| {
        g.global_slow.next = now;
        g.global_slow.amount = amount;
        g.global_slow.active = true;
        g.global_slow.until = if duration == 0.0 { 0.0 } else { now + duration };
    });
}

/// `CTSGameRules::GlobalSlowMotion2` (0x79720): every 0.5 s, slow everything not already slowed
/// (0 resets to 1). Every `UTIL_PlayerByIndex` player, players who left included.
pub fn global_slow2(amount: f32, btime: bool) {
    let now = engine().time();
    let due = state::with(|g| {
        if g.global_slow.next <= now || amount == 0.0 {
            g.global_slow.next = now + 0.5;
            true
        } else {
            false
        }
    });
    if !due {
        return;
    }
    let eng = engine();
    for e in super::slot_players() {
        if amount == 0.0 {
            go_slow(e, 1.0, false);
        } else if !crate::ham::in_slow(e) {
            go_slow(e, amount, btime);
        }
    }
    for class in [c"grenade", c"knife", c"WorldGun"] {
        let mut it = std::ptr::null_mut();
        loop {
            it = eng.find_by_string(it, c"classname", class);
            if it.is_null() {
                break;
            }
            let v = it.v();
            if amount == 0.0 {
                v.fuser1 = 1.0;
            } else if v.fuser1 == 1.0 {
                v.fuser1 = amount;
            }
        }
    }
}

/// `PutPowerUp` (0x9797c): a random deathmatch spot, a random clear position near it, no other
/// power-up within 128 units. False when no spot was clear.
pub fn put_powerup(kind: u16) -> bool {
    if kind == 0 {
        return false;
    }
    let eng = engine();
    let mut spots = Vec::new();
    let mut it = std::ptr::null_mut();
    while spots.len() < 32 {
        it = eng.find_by_string(it, c"classname", c"info_player_deathmatch");
        if it.is_null() {
            break;
        }
        spots.push(it);
    }
    while !spots.is_empty() {
        let i = eng.random_long(0, spots.len() as i32 - 1) as usize;
        let org = random_position(spots[i].v().origin);
        let mut clear = true;
        let mut s = std::ptr::null_mut();
        loop {
            s = eng.find_in_sphere(s, org, 128.0);
            if s.is_null() {
                break;
            }
            if eng.string(s.v().classname) == c"ts_powerup" {
                clear = false;
                break;
            }
        }
        if clear {
            crate::entities::ts::create_powerup(kind, -1, org);
            return true;
        }
        spots.remove(i);
    }
    false
}

/// `GetRandomPosition` (0x974f4): up to 16 horizontal directions (23° apart from a random start),
/// 128..1024 units out, pulled back 64 units from what the trace hits.
fn random_position(from: [f32; 3]) -> [f32; 3] {
    let eng = engine();
    let mut angle = eng.random_long(0, 16) * 23;
    for _ in 0..16 {
        let dist = eng.random_long(128, 1024) as f32;
        let r = (angle as f32).to_radians();
        let dir = [r.cos(), r.sin(), 0.0];
        let end = [from[0] + dir[0] * dist, from[1] + dir[1] * dist, from[2]];
        let tr = eng.trace_line(from, end, goldsrc::server::IGNORE_MONSTERS, std::ptr::null_mut());
        let p = [tr.vecEndPos[0] - dir[0] * 64.0, tr.vecEndPos[1] - dir[1] * 64.0, tr.vecEndPos[2]];
        if eng.point_contents(p) != CONTENTS_SOLID {
            return p;
        }
        angle += 23;
    }
    from
}

/// Whether `ResetRestartRound` acts: in the round modes 2/3 (unreachable in 3.0) or with
/// `g_lastManStanding`, whatever the mode.
fn restart_round_applies(mode: Mode, lms: bool) -> bool {
    matches!(mode, Mode::RoundTdm | Mode::Objective) || lms
}

/// `CTSGameRules::ResetRestartRound(n)` (0x78750), from the `restartround` server command with
/// `n = 2`: the round restarts in 1 s and the round end is cleared; with the round timer on,
/// `RoundTime` gets the time to the restart (not below 0, truncated: 0x787e4); `n > 1` also puts
/// the four teams' score at 0 and their cash at `(int)initial_cash.value` (0: TS never registers
/// the cvar) and sends the `TeamScore` of each named one; then "Restarting Round" to all (black,
/// channel 8, hold 30). The team field `+0x20` it also clears isn't modelled.
pub fn reset_restart_round(n: i32) {
    let now = engine().time();
    if !state::with(|g| restart_round_applies(g.mode, g.lms)) {
        return;
    }
    let (restart, timer) = state::with(|g| {
        g.round_restart = now + 1.0;
        g.round_end = 0.0;
        (g.round_restart, g.round_timer)
    });
    if timer {
        let secs = round_time_byte(x87_max0(restart as f64 - now as f64));
        msg::all(Msg::RoundTime, |w| RoundTime { secs, state: 0 }.write(w));
    }
    if n > 1 {
        let named: Vec<Vec<u8>> = state::with(|g| {
            g.teams
                .iter_mut()
                .take(4)
                .filter_map(|t| {
                    t.score = 0;
                    t.cash = 0;
                    (!t.name.is_empty()).then(|| t.name.clone())
                })
                .collect()
        });
        for team in named {
            let m = TeamScore { team, score: 0, deaths: 0 };
            msg::all(Msg::TeamScore, |w| m.write(w));
        }
    }
    ts_message_all([0, 0, 0], 8, 30, "Restarting Round");
}

#[allow(dead_code)]
fn unused() -> MsgWriter {
    messages::all(Msg::RoundTime)
}

#[cfg(test)]
mod tests {
    use super::{Mode, restart_round_applies, round_time_byte, time_limit, x87_eq, x87_max0};

    #[test]
    fn the_time_limit_counts_from_the_map_start() {
        assert_eq!(time_limit(20.0, 1150.0), (50, false), "the map's clock: under a minute left");
        assert_eq!(time_limit(20.0, 1200.0), (0, true));
        // Off at 0; otherwise TS's `timelimit * 60 - time`, truncated.
        assert_eq!(time_limit(0.0, 1e6), (0, false));
        assert_eq!(time_limit(1.0, 30.4), (29, false));
        assert_eq!(time_limit(1.0, 60.0), (0, true));
    }

    #[test]
    fn restartround_needs_a_round_mode_or_lms() {
        // Issue #122: `(mode - 2) < 2 || g_lastManStanding`, so LMS in teamplay restarts too.
        assert!(restart_round_applies(Mode::Dm, true));
        assert!(restart_round_applies(Mode::TeamDm, true));
        assert!(restart_round_applies(Mode::RoundTdm, false));
        assert!(restart_round_applies(Mode::Objective, false));
        assert!(!restart_round_applies(Mode::Dm, false));
        assert!(!restart_round_applies(Mode::TeamDm, false));
        // The restart 1 s ahead: `RoundTime` 1.
        let now = 100.3f32;
        assert_eq!(round_time_byte(x87_max0((now + 1.0) as f64 - now as f64)), 1);
    }

    #[test]
    fn x87_compares_and_round_time_bytes() {
        assert!(x87_eq(1.0, 1.0) && x87_eq(f64::NAN, 1.0) && x87_eq(1.0, f64::NAN));
        assert!(!x87_eq(1.0, 0.5));
        // `fldz; fcom st(1); jbe`: negatives clamp, NaN is kept.
        assert_eq!(x87_max0(-3.0), 0.0);
        assert_eq!(x87_max0(4.5), 4.5);
        assert!(x87_max0(f64::NAN).is_nan());
        // Truncated, then `WriteByte`'s low byte.
        assert_eq!(round_time_byte(29.99), 29);
        assert_eq!(round_time_byte(300.0), 44);
        assert_eq!(round_time_byte(-2.5), 254);
        assert_eq!(round_time_byte(f64::NAN), 0);
        // `RestartRound` sends the stored end time less `time`: the `f32` end can fall short.
        let now = 1000.013f32;
        let end = now + 30.0;
        assert_eq!(round_time_byte(x87_max0(end as f64 - now as f64)), 29);
    }
}
