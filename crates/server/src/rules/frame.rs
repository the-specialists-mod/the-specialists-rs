//! `StartFrame` → `CHalfLifeMultiplay::Think` (0xcf038, time/frag limits, intermission) and
//! `CTSGameRules::Think` (0x77204: global slow, auto power-ups, The One, LMS rounds, SrvSett).

use super::lifecycle::{give_cash, start_observer};
use super::msg::{self, ts_message_all};
use super::powerup::{entity_in_slow, go_slow};
use super::state::{self, Mode};
use super::{alive, cvars, other, scoring, teams, votes};
use crate::messages::{self, Msg};
use goldsrc::Global;
use goldsrc::edict::EdictPtr;
use goldsrc::ext::rules::SVC_INTERMISSION;
use goldsrc::server::{MsgWriter, engine};
use goldsrc::sys::*;
use shared::protocol::rules::{RoundTime, SrvSett, TeamScore};
use shared::weapons::{Gun, att, id, tables};
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
/// restarts, less the time of the last gameplay switch's reset (`GameRules::time_base`, 0 after a
/// map start), so the reset gives the new game the whole limit as a new map would (issue #100
/// §1.3). Extended precision, as TS's x87 subtraction.
fn game_clock(now: f32) -> f64 {
    now as f64 - state::with(|g| g.time_base) as f64
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

/// `CTSGameRules::Think` (0x77204), TS 3.0's
/// ([`ServerRules::think`](crate::gameplay::ServerRules::think)).
pub fn ts_think(now: f32) {
    // Global slow motion.
    let (active, amount, until) = state::with(|g| (g.global_slow.active, g.global_slow.amount, g.global_slow.until));
    if until != 0.0 && until < now {
        state::with(|g| g.global_slow.active = false);
    }
    if !state::with(|g| g.global_slow.active) {
        if amount != 0.0 {
            global_slow2(0.0, false);
            state::with(|g| {
                g.global_slow.amount = 0.0;
                g.global_slow.until = 0.0;
            });
        }
    } else if active {
        global_slow2(amount, false);
    }

    if state::with(|g| g.check_deads_at != 0.0 && g.check_deads_at < now) {
        check_deads(now);
    }

    let (mode, lms, the_one) = state::with(|g| (g.mode, g.lms, g.the_one));
    if matches!(mode, Mode::Dm | Mode::TeamDm) {
        auto_powerups(now);
        if the_one && state::with(|g| g.next_the_one_check < now) {
            the_one_check(now);
        }
        crate::gameplay::active().server.entities_think(now);
        if lms && mode == Mode::Dm {
            let (restart, end) = state::with(|g| (g.round_restart, g.round_end));
            if restart != 0.0 && restart < now {
                restart_round(now);
            } else if end < now && restart == 0.0 {
                check_deads(now);
            }
        }
    }

    votes::check_votes(now);
    srvsett_think();
}

/// The end of `CTSGameRules::Think`: the `realbullet` copy (0x775b3..0x77632), the simulated
/// bullets (0x77635), the `usecash` copy (0x77650..0x776d5) and `ammocount` (0x776d8..0x7775b),
/// each broadcast as `SrvSett` when it changes.
pub fn srvsett_think() {
    let rb = cvars::f(c"realbullet");
    if let Some(v) = state::with(|g| refresh_copy(&mut g.last_srvsett[0], rb)) {
        msg::all(Msg::SrvSett, |w| SrvSett { key: 0, value: v as i16 }.write(w));
    }
    // Between the two copies, so a bullet steps with this frame's `realbullet`.
    crate::weapons::bullet_think();
    let cash = cvars::f(c"usecash");
    if let Some(v) = state::with(|g| refresh_copy(&mut g.last_srvsett[1], cash)) {
        // `setne`: whether the truncated copy is non-zero, so 0.5 sends 0.
        msg::all(Msg::SrvSett, |w| SrvSett { key: 1, value: (v != 0) as i16 }.write(w));
    }
    // `ammocount` is compared as an int (truncated first), and a change is `Set_ammo_count` too
    // (`state::ammo_column`).
    let v = ammocount();
    if state::with(|g| std::mem::replace(&mut g.last_srvsett[2], v) != v) {
        msg::all(Msg::SrvSett, |w| SrvSett { key: 2, value: v as i16 }.write(w));
    }
}

/// The `realbullet`/`usecash` update of `CTSGameRules::Think`: `fild` the stored int, `fcomp` it
/// with the float cvar and, unless they're equal, store `(int)cvar` (truncated) and return it to be
/// sent. A fractional value never equals its truncation, so TS re-sends it every Think; a NaN
/// compares unordered, which the `test ah,0x40` takes as equal, so it never updates.
pub(crate) fn refresh_copy(stored: &mut i32, cvar: f32) -> Option<i32> {
    if x87_eq(*stored as f64, cvar as f64) {
        return None;
    }
    *stored = crate::entities::stock::maprules::x87_int(cvar);
    Some(*stored)
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
        } else if !entity_in_slow(e) {
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

/// Automatic power-up spawner (DM/TDM, `nopowerup != 1`, maps without their own power-ups).
pub fn auto_powerups(now: f32) {
    let enabled = state::with(|g| {
        if g.powerup_spawner == 0xff {
            g.powerup_spawner = 1;
        }
        g.powerup_spawner == 1
    });
    // `fld1; fcomp; test ah,0x40` (0x772f9): a NaN `nopowerup` counts as 1.
    if x87_eq(cvars::f(c"nopowerup") as f64, 1.0) || !enabled {
        return;
    }
    let next = state::with(|g| g.next_powerup);
    if now <= next {
        if next - now > 21.0 {
            state::with(|g| g.next_powerup = 0.0);
        }
        return;
    }
    let r = engine().random_float(0.0, 1.0);
    // A NaN `slowmatch` counts as 1 too (0x77358).
    let kind = if r < 0.25 && !x87_eq(cvars::f(c"slowmatch") as f64, 1.0) {
        shared::ts::powerup::SLOWMO
    } else {
        state::with(|g| g.random_powerup(0x1ff))
    };
    let ok = put_powerup(kind);
    state::with(|g| g.next_powerup = now + if ok { 20.0 } else { 1.0 });
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

/// `CheckDeads` (0x79c00), LMS part (modes 2/3 are unreachable).
fn check_deads(now: f32) {
    let (restart, mode, lms, round_end, timer) = state::with(|g| {
        g.check_deads_at = 0.0;
        (g.round_restart, g.mode, g.lms, g.round_end, g.round_timer)
    });
    if restart != 0.0 || mode != Mode::Dm || !lms {
        return;
    }
    let mut total = 0;
    let mut living = Vec::new();
    // `UTIL_PlayerByIndex` and `IsPlayer()`: a player who left counts (dead, so not living).
    for e in super::slot_players() {
        total += 1;
        if alive(e.v()) {
            living.push(e);
        }
    }
    if living.len() == 1 {
        let last = living[0];
        if total < 2 {
            ts_message_all([0, 255, 0], 8, 30, "Waiting for other players.");
        } else {
            let name = super::netname(last);
            ts_message_all([0, 255, 0], 1, 30, &format!("{name} is the Last Man Standing! 10 frags"));
            super::add_frags(last, None, 10.0);
            scoring::broadcast_score(last);
            msg::log(&format!("Player \"{name}\" triggered \"last_man_standing\"\n"));
        }
        state::with(|g| g.round_restart = now + 3.0);
        crate::plugins::round_end();
        if timer {
            // 0x79e89: no clamp here, so an overrun round sends the low byte of a negative count.
            let rt = RoundTime { secs: round_time_byte(round_end as f64 - now as f64), state: 2 };
            msg::all(Msg::RoundTime, |w| rt.write(w));
        }
    } else if (!living.is_empty() && now <= round_end && total > 1) || !timer {
        ts_message_all([0, 255, 0], 1, 30, &format!("{} men standing!", living.len()));
    } else {
        ts_message_all([0, 255, 0], 8, 30, "Round over");
        state::with(|g| g.round_restart = now + 3.0);
        crate::plugins::round_end();
    }
}

/// `RestartRound` (0x77920) for LMS: new round time, refunds, everyone back to observer with a
/// forced respawn in 3 s, map items respawned.
pub fn restart_round(now: f32) {
    let rt = cvars::round_time();
    let (timer, end) = state::with(|g| {
        g.round_end = now + rt;
        g.round_restart = 0.0;
        (g.round_timer, g.round_end)
    });
    if timer {
        // 0x779b1: the stored end time less `time` (truncated), not the round time itself, so the
        // `f32` rounding of `m_flRoundEnd` can make it a second short.
        let secs = round_time_byte(x87_max0(end as f64 - now as f64));
        msg::all(Msg::RoundTime, |w| RoundTime { secs, state: 0 }.write(w));
    }
    // `UTIL_PlayerByIndex` and `IsPlayer()`: a player who left (still `iuser1` 0 if it left alive)
    // is put in observer mode as well; see `rules::set_client_maxspeed`.
    for e in super::slot_players() {
        crate::player::with_player(e, |p, cx| {
            p.ts.join_deadline = now + 10.0;
            p.respawn_at = now + 3.0;
            p.ts.award = 11;
            p.ts.respawns_used = 0;
            p.powerup_until = 0.0;
            p.ts.held_secs = 0;
            p.active_powerup = 0;
            p.ts.held = 0;
            give_cash(p, round_refund(&p.inv.gun));
            if cx.pev().iuser1 == 0 {
                let (o, a) = (cx.pev().origin, cx.pev().v_angle);
                start_observer(p, cx, o, a);
            }
        });
    }
    teams::recount(false);
    crate::gameplay::active().server.respawn_entities();
    // Port addition: plugins hear of the new round.
    crate::plugins::round_start();
}

/// `RestartRound`'s refund of what a player carries: the price of every owned weapon (not kung fu
/// or the briefcase) with its attachments, plus, for each ammo type holding more than one
/// purchase, `price × rounds / per_buy` (all the rounds, as in TS).
pub fn round_refund(gun: &Gun) -> i32 {
    let t = tables();
    let prices = &t.combat.attachment_price;
    let mut total = 0;
    for (wid, st) in gun.status.iter().enumerate().skip(1) {
        if !st.owned || wid == id::BRIEFCASE as usize {
            continue;
        }
        total += t.weapon(wid as u8).map_or(0, |w| w.cost);
        for (bit, price) in
            [(att::SILENCER, prices.silencer), (att::LASER, prices.laser), (att::FLASHLIGHT, prices.flashlight), (att::SCOPE, prices.scope)]
        {
            if st.items & bit != 0 {
                total += price;
            }
        }
    }
    for (ty, &rounds) in gun.reserve.iter().enumerate() {
        if let Some(a) = t.ammo(ty as u8).filter(|a| a.per_buy > 0 && rounds > a.per_buy) {
            total = total.wrapping_add(a.price.wrapping_mul(rounds) / a.per_buy);
        }
    }
    total
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
        let named: Vec<String> = state::with(|g| {
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

/// The One block of `CTSGameRules::Think` (0x77204): counts the players other than The One
/// (`+0x5876`, The One's regeneration rate) and, when there is no One, `SelectRandomTheOne`. TS
/// loops `i = 0 .. maxClients - 1`, so slot 0 (never a player) is tried and the last slot is never
/// counted; dormant players (those who left) are skipped.
pub fn the_one_check(now: f32) {
    let eng = engine();
    let mut exists = false;
    let mut others = 0u8;
    for i in 0..eng.max_clients() {
        let e = eng.player(i);
        if !super::has_player(e) || e.v().flags & FL_DORMANT as i32 != 0 {
            continue;
        }
        if other(e, |p| p.ts.is_the_one).unwrap_or(false) {
            exists = true;
        } else {
            others = others.wrapping_add(1);
        }
    }
    state::with(|g| g.the_one_others = others as i32);
    if !exists {
        select_random_the_one(now);
    }
    state::with(|g| g.next_the_one_check = now + 10.0);
}

/// `CTSGameRules::SelectRandomTheOne` (0x7af28): at most every 10 s; nothing while someone is The
/// One; otherwise a random living, non-observing player (at most 33 candidates) becomes The One and
/// changes team.
fn select_random_the_one(now: f32) {
    if state::with(|g| g.next_the_one_check > now) {
        return;
    }
    state::with(|g| g.next_the_one_check = now + 10.0);
    let mut candidates = Vec::new();
    for e in super::slot_players() {
        if other(e, |p| p.ts.is_the_one).unwrap_or(false) {
            return;
        }
        let v = e.v();
        if alive(v) && v.iuser1 == 0 && candidates.len() < 0x21 {
            candidates.push(e);
        }
    }
    if candidates.is_empty() {
        return;
    }
    let pick = candidates[engine().random_long(0, candidates.len() as i32 - 1) as usize];
    scoring::set_player_the_one_other(pick);
    teams::the_one_switch_other(pick);
}

#[allow(dead_code)]
fn unused() -> MsgWriter {
    messages::all(Msg::RoundTime)
}

#[cfg(test)]
mod tests {
    use super::{Mode, refresh_copy, restart_round_applies, round_time_byte, time_limit, x87_eq, x87_max0};

    #[test]
    fn the_time_limit_counts_from_the_games_start() {
        // Issue #123: 19 minutes into the map, a switch at 1140 s leaves the new game 20 minutes.
        let switch = 1140.0f64;
        assert_eq!(time_limit(20.0, 1150.0 - switch), (1190, false));
        assert_eq!(time_limit(20.0, 1150.0), (50, false), "the map's clock: one minute left");
        assert_eq!(time_limit(20.0, 2340.0 - switch), (0, true));
        // Off at 0; a real map start (base 0) is TS's `timelimit * 60 - time`, truncated.
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

    /// Runs `n` Thinks with the cvar at `cvar`, returning the `SrvSett` values sent.
    fn thinks(stored: &mut i32, cvar: f32, n: usize) -> Vec<i32> {
        (0..n).filter_map(|_| refresh_copy(stored, cvar)).collect()
    }

    #[test]
    fn srvsett_copy_is_truncated_and_compared_as_float() {
        // `Init`'s -1: the first Think sends the value, later ones nothing while it stands.
        let mut c = -1;
        assert_eq!(thinks(&mut c, 0.0, 3), [0]);
        assert_eq!(thinks(&mut c, 2.0, 3), [2]);
        assert_eq!(c, 2);
        // Truncated, not rounded: 1.9 is 1, -0.7 is 0.
        let mut c = -1;
        assert_eq!(refresh_copy(&mut c, 1.9), Some(1));
        assert_eq!(refresh_copy(&mut c, -0.7), Some(0));
        // A fractional value never equals its truncated copy: sent again every Think.
        let mut c = -1;
        assert_eq!(thinks(&mut c, 0.5, 3), [0, 0, 0]);
        assert_eq!(thinks(&mut c, 2.5, 2), [2, 2]);
        // NaN compares unordered, which TS takes as equal: never updated, never sent.
        let mut c = 1;
        assert_eq!(thinks(&mut c, f32::NAN, 2), []);
        assert_eq!(c, 1);
        // Out of range: the integer indefinite, which never equals the float either.
        let mut c = 0;
        assert_eq!(thinks(&mut c, 3.0e9, 2), [i32::MIN, i32::MIN]);
        // The compare is exact (`fild` is exact), so a large integral float settles.
        let mut c = 0;
        assert_eq!(thinks(&mut c, 16_777_216.0, 2), [16_777_216]);
    }

    #[test]
    fn usecash_sends_whether_the_truncated_copy_is_non_zero() {
        let sent = |cvar: f32| {
            let mut c = -1;
            refresh_copy(&mut c, cvar).map(|v| (v != 0) as i16)
        };
        assert_eq!(sent(0.5), Some(0), "0.5 truncates to 0");
        assert_eq!(sent(1.0), Some(1));
        assert_eq!(sent(-1.5), Some(1));
        assert_eq!(sent(0.0), Some(0));
        assert_eq!(sent(f32::NAN), None);
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

    #[test]
    fn refund() {
        use super::round_refund;
        use shared::weapons::{att, gun::Gun, id, tables};
        let t = tables();
        let mut g = Gun::default();
        assert_eq!(round_refund(&g), 0);
        // Kung fu and the briefcase are free.
        g.status[id::KUNGFU as usize].owned = true;
        g.status[id::BRIEFCASE as usize].owned = true;
        assert_eq!(round_refund(&g), 0);
        let m4 = id::M4 as usize;
        g.status[m4].owned = true;
        g.status[m4].items = att::SILENCER | att::SCOPE;
        let gun_value = t.weapon(id::M4).unwrap().cost + 900 + 1800;
        assert_eq!(round_refund(&g), gun_value);
        // One purchase of ammo or less isn't refunded; more refunds every round.
        let ammo = t.weapon(id::M4).unwrap().ammo;
        let a = t.ammo(ammo).unwrap();
        g.reserve[ammo as usize] = a.per_buy;
        assert_eq!(round_refund(&g), gun_value);
        g.reserve[ammo as usize] = a.per_buy * 3;
        assert_eq!(round_refund(&g), gun_value + a.price * 3);
    }
}
