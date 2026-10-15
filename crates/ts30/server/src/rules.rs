//! TS 3.0's game rules: the multiplayer install (`DecideGamePlay`, `CTSGameRules::Init`), the
//! server browser's description and `CTSGameRules::Think` (0x77204: global slow, auto power-ups,
//! The One, LMS rounds, SrvSett).

use goldsrc::edict::EdictPtr;
use goldsrc::server::engine;
use goldsrc::sys::FL_DORMANT;
use shared::protocol::rules::{RoundTime, SrvSett};
use shared::weapons::{Gun, att, id, tables};
use std::ffi::CStr;
use ts_server::game_description;
use ts_server::messages::Msg;
use ts_server::rules::frame::{ammocount, global_slow2, put_powerup, round_time_byte, x87_eq, x87_max0};
use ts_server::rules::lifecycle::{give_cash, start_observer};
use ts_server::rules::msg::{self, ts_message_all};
use ts_server::rules::state::{self, GameRules, Mode};
use ts_server::rules::{DescriptionMode, add_frags, cvars, has_player, is_alive, netname, other, scoring, slot_players, teams, votes};

/// The descriptions with the port's version: "The Specialists v3.0.2 (DM)" where TS says
/// "The Specialists (DM)" (port change, so the server browser's Game column tells releases apart).
pub fn description(mode: DescriptionMode) -> &'static CStr {
    match mode {
        DescriptionMode::Single => game_description!(),
        DescriptionMode::Dm => game_description!("DM"),
        DescriptionMode::Lms => game_description!("LMS"),
        DescriptionMode::Teamplay => game_description!("Teamplay"),
        DescriptionMode::TheOne => game_description!("The One"),
    }
}

/// `DecideGamePlay` (0x767ac): returns (teamplay, lms, the_one).
pub fn decide_gameplay(forced: i32, teamplay_cvar: bool, lms_cvar: bool, the_one_cvar: bool) -> (bool, bool, bool) {
    match forced {
        1 => (false, false, false),
        2 => (true, false, false),
        3 => (false, true, false),
        4 => (true, false, true),
        f if f >= 1 => (false, false, false),
        _ => {
            if the_one_cvar {
                (true, false, true)
            } else if lms_cvar {
                (false, true, false)
            } else {
                (teamplay_cvar, false, false)
            }
        }
    }
}

/// `InstallGameRules` and the rules' constructors for a multiplayer map: `DecideGamePlay`, the
/// rules' constructors and `CTSGameRules::Init`.
pub fn install(g: &mut GameRules) {
    let forced = g.forced_gamemode;
    let (teamplay, lms, the_one) =
        decide_gameplay(forced, cvars::on(c"mp_teamplay"), cvars::on(c"lastmanstanding"), cvars::on(c"mp_theonemode"));
    let (map_teams, map_models) = (std::mem::take(&mut g.map_teams), std::mem::take(&mut g.map_team_models));
    *g = GameRules::default();
    g.installed = true;
    g.single_player = engine().globals().deathmatch == 0.0;
    g.teamplay = teamplay;
    g.lms = lms;
    g.the_one = the_one;
    g.player_team = vec![Vec::new(); 33];
    // The `CHalfLifeMultiplay` constructor (0xcedd0) starts with `CTSGameRules::Init`.
    g.ts_init(engine().time());
    if teamplay {
        let list = if !map_teams.is_empty() && cvars::on(c"mp_teamoverride") { map_teams.clone() } else { cvars::bytes(c"mp_teamlist") };
        if !list.is_empty() {
            g.set_team_list(&list);
        }
        g.init_teamplay_list();
        let models = if !map_models.is_empty() { map_models.clone() } else { cvars::bytes(c"mp_teammodels") };
        if !models.is_empty() {
            g.set_team_model_lists(&models);
        }
    }
    g.map_teams = map_teams;
    g.map_team_models = map_models;
    engine().print(&shared::bformat!(
        "[ts] game rules: {} ({})\n",
        match (g.teamplay, g.lms, g.the_one) {
            (_, _, true) => "The One",
            (_, true, _) => "Last Man Standing",
            (true, _, _) => "Team DeathMatch",
            _ => "DeathMatch",
        },
        g.teams.iter().filter(|t| !t.name.is_empty()).map(|t| t.name.as_slice()).collect::<Vec<_>>().join(&b';')
    ));
}

/// `CTSGameRules::Think` (0x77204), from the multiplay or teamplay `Think` each frame.
pub fn think(now: f32) {
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
        ts_server::gameplay::active().server.entities_think(now);
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
    ts_server::weapons::bullet_think();
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
fn refresh_copy(stored: &mut i32, cvar: f32) -> Option<i32> {
    if x87_eq(*stored as f64, cvar as f64) {
        return None;
    }
    *stored = ts_server::entities::stock::maprules::x87_int(cvar);
    Some(*stored)
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
    for e in slot_players() {
        total += 1;
        if is_alive(e) {
            living.push(e);
        }
    }
    if living.len() == 1 {
        let last = living[0];
        if total < 2 {
            ts_message_all([0, 255, 0], 8, 30, "Waiting for other players.");
        } else {
            let name = netname(last);
            ts_message_all([0, 255, 0], 1, 30, &shared::bformat!("{} is the Last Man Standing! 10 frags", name));
            add_frags(last, None, 10.0);
            scoring::broadcast_score(last);
            msg::log(&shared::bformat!("Player \"{}\" triggered \"last_man_standing\"\n", name));
        }
        state::with(|g| g.round_restart = now + 3.0);
        ts_server::plugins::round_end();
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
        ts_server::plugins::round_end();
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
    // `InitObjectives` and `SetObjective` (0x785d8, 0x790b4): outside the round modes only the
    // objective list is cleared.
    let round = state::with(|g| matches!(g.mode, Mode::RoundTdm | Mode::Objective));
    ts_server::entities::ts::objective::set_objective(round);
    // `UTIL_PlayerByIndex` and `IsPlayer()`: a player who left (still `iuser1` 0 if it left alive)
    // is put in observer mode as well; see `rules::set_client_maxspeed`.
    for e in slot_players() {
        ts_server::player::with_player(e, |p, cx| {
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
    ts_server::gameplay::active().server.respawn_entities();
    // Port addition: plugins hear of the new round.
    ts_server::plugins::round_start();
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
        if !has_player(e) || e.v().flags & FL_DORMANT as i32 != 0 {
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
    for e in slot_players() {
        if other(e, |p| p.ts.is_the_one).unwrap_or(false) {
            return;
        }
        if is_alive(e) && e.v().iuser1 == 0 && candidates.len() < 0x21 {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptions_carry_the_release_version() {
        let v = env!("CARGO_PKG_VERSION");
        assert_eq!(description(DescriptionMode::Dm).to_str().unwrap(), format!("The Specialists v{v} (DM)"));
        assert_eq!(description(DescriptionMode::Single).to_str().unwrap(), format!("The Specialists v{v}"));
    }

    #[test]
    fn gameplay_decision() {
        // worldspawn gamemode wins over cvars
        assert_eq!(decide_gameplay(1, true, true, true), (false, false, false));
        assert_eq!(decide_gameplay(2, false, false, false), (true, false, false));
        assert_eq!(decide_gameplay(3, true, false, false), (false, true, false));
        assert_eq!(decide_gameplay(4, false, false, false), (true, false, true));
        // cvar fallback priority: The One, LMS, teamplay
        assert_eq!(decide_gameplay(0, true, true, true), (true, false, true));
        assert_eq!(decide_gameplay(0, true, true, false), (false, true, false));
        assert_eq!(decide_gameplay(0, true, false, false), (true, false, false));
        assert_eq!(decide_gameplay(0, false, false, false), (false, false, false));
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
    fn refund() {
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
