//! `StartFrame` → `CHalfLifeMultiplay::Think` (0xcf038, time/frag limits, intermission) and
//! `CTSGameRules::Think` (0x77204: global slow, auto power-ups, The One, LMS rounds, SrvSett).

use super::lifecycle::{give_cash, start_observer};
use super::msg::{self, ts_message_all};
use super::powerup::{entity_in_slow, go_slow};
use super::state::{self, Mode};
use super::{alive, cvars, other, scoring, teams, votes};
use crate::messages::{self, Msg};
use goldsrc::edict::EdictPtr;
use goldsrc::ext::rules::SVC_INTERMISSION;
use goldsrc::server::{MsgWriter, engine};
use goldsrc::sys::*;
use shared::protocol::rules::{RoundTime, SrvSett};
use shared::weapons::{Gun, att, id, tables};

pub fn think() {
    if !state::with(|g| g.installed) {
        return;
    }
    // CHalfLifeMultiplay::Think starts with CVoiceGameMgr::Update, even during intermission.
    super::voice::update(engine().globals().frametime as f64);
    let now = engine().time();
    if multiplay_think(now) {
        return;
    }
    ts_think(now);
    // CHalfLifeMultiplay::Think: a passed map vote ends the map now.
    if votes::vote_passed() {
        go_to_intermission();
    }
}

/// Returns true while the game is over (intermission).
fn multiplay_think(now: f32) -> bool {
    let eng = engine();
    if state::with(|g| g.game_over) {
        let chat = cvars::f(c"mp_chattime");
        if chat < 10.0 {
            eng.set_cvar_string(c"mp_chattime", "10");
        } else if chat > 120.0 {
            eng.set_cvar_string(c"mp_chattime", "120");
        }
        let start = state::with(|g| g.intermission_start);
        let end = start + cvars::f(c"mp_chattime").clamp(10.0, 120.0);
        // After the chat time the level changes once someone presses a button
        // (`PlayerThink`), or at the latest 120 s into the intermission.
        let changed = state::with(|g| {
            if g.intermission_end == 0.0 && now > end && (g.end_intermission_button_hit || now > start + 120.0) {
                g.intermission_end = now;
                true
            } else {
                false
            }
        });
        if changed {
            change_level();
        }
        return true;
    }

    let timelimit = cvars::f(c"mp_timelimit") * 60.0;
    let mut time_left = 0;
    if timelimit != 0.0 {
        time_left = (timelimit - now).round() as i32;
        if timelimit <= now {
            go_to_intermission();
            return true;
        }
    }
    let fraglimit = cvars::f(c"mp_fraglimit");
    let mut frags_left = 0;
    if fraglimit != 0.0 {
        frags_left = 9999;
        let (teamplay, ctb) = state::with(|g| (g.teamplay, g.mode == Mode::TeamDm && g.is_ctb()));
        if teamplay {
            teams::recount();
            let scores: Vec<i32> =
                state::with(|g| g.teams.iter().filter(|t| !t.name.is_empty()).map(|t| if ctb { t.score } else { t.frags }).collect());
            for s in scores {
                if s as f32 >= fraglimit {
                    go_to_intermission();
                    return true;
                }
                frags_left = frags_left.min((fraglimit - s as f32) as i32);
            }
        } else {
            for e in super::players() {
                let f = e.v().frags;
                if f >= fraglimit {
                    go_to_intermission();
                    return true;
                }
                frags_left = frags_left.min((fraglimit - f).round() as i32);
            }
        }
    }
    let (lf, lt) =
        state::with(|g| (std::mem::replace(&mut g.last_frags_left, frags_left), std::mem::replace(&mut g.last_time_left, time_left)));
    if lf != frags_left {
        set_cvar_direct(c"mp_fragsleft", frags_left);
    }
    if lt != time_left {
        set_cvar_direct(c"mp_timeleft", time_left);
    }
    false
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

/// `ChangeLevel` (0xd0784): the map a vote chose, else the next map from `mapcyclefile`
/// (after the current one), else the same map.
fn change_level() {
    let eng = engine();
    let current = eng.string(eng.globals().mapname).to_string_lossy().into_owned();
    let file = {
        let f = eng.cvar_string(c"mapcyclefile").to_string_lossy().into_owned();
        if f.is_empty() { "mapcycle.txt".to_owned() } else { f }
    };
    let maps: Vec<String> = eng
        .load_file(&file)
        .map(|d| {
            String::from_utf8_lossy(&d).lines().filter_map(|l| l.split("//").next()?.split_whitespace().next().map(str::to_owned)).collect()
        })
        .unwrap_or_default();
    let mut next = next_map(&maps, &current, |m| eng.is_map_valid(m));
    // A passed map vote wins over the cycle (an invalid one falls back to the current map).
    if let Some(voted) = state::with(|g| g.votes.override_map().map(str::to_owned)) {
        next = if eng.is_map_valid(&voted) { voted } else { current.clone() };
    }
    state::with(|g| g.votes.chosen.clear());
    eng.print(&format!("[ts] changing level to {next}\n"));
    eng.change_level(&next);
}

/// Picks the entry after `current` in the cycle (wrapping), skipping invalid maps.
pub fn next_map(cycle: &[String], current: &str, valid: impl Fn(&str) -> bool) -> String {
    if cycle.is_empty() {
        return current.to_owned();
    }
    let start = cycle.iter().position(|m| m.eq_ignore_ascii_case(current)).map_or(0, |i| i + 1);
    (0..cycle.len()).map(|k| &cycle[(start + k) % cycle.len()]).find(|m| valid(m)).cloned().unwrap_or_else(|| current.to_owned())
}

fn ts_think(now: f32) {
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

    // SrvSett broadcasts when the cvars change.
    let values = [cvars::f(c"realbullet").round() as i32, cvars::on(c"usecash") as i32, cvars::f(c"ammocount").round() as i32];
    for (key, v) in values.into_iter().enumerate() {
        let changed = state::with(|g| std::mem::replace(&mut g.last_srvsett[key], v) != v);
        if changed {
            msg::all(Msg::SrvSett, |w| SrvSett { key: key as u8, value: v as i16 }.write(w));
        }
    }
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

/// `GlobalSlowMotion2`: every 0.5 s, slow everything not already slowed (0 resets to 1).
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
    for e in super::players() {
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
fn auto_powerups(now: f32) {
    let enabled = state::with(|g| {
        if g.powerup_spawner == 0xff {
            g.powerup_spawner = 1;
        }
        g.powerup_spawner == 1
    });
    if cvars::f(c"nopowerup") == 1.0 || !enabled {
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
    let kind =
        if r < 0.25 && cvars::f(c"slowmatch") != 1.0 { shared::ts::powerup::SLOWMO } else { state::with(|g| g.random_powerup(0x1ff)) };
    let ok = put_powerup(kind);
    state::with(|g| g.next_powerup = now + if ok { 20.0 } else { 1.0 });
}

/// `PutPowerUp` (0x9797c): a random deathmatch spot, a random clear position near it, no other
/// power-up within 128 units.
fn put_powerup(kind: u16) -> bool {
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
    for e in super::players() {
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
        if timer {
            let rt = RoundTime { secs: (round_end - now).max(0.0).round() as u8, state: 2 };
            msg::all(Msg::RoundTime, |w| rt.write(w));
        }
    } else if (!living.is_empty() && now <= round_end && total > 1) || !timer {
        ts_message_all([0, 255, 0], 1, 30, &format!("{} men standing!", living.len()));
    } else {
        ts_message_all([0, 255, 0], 8, 30, "Round over");
        state::with(|g| g.round_restart = now + 3.0);
    }
}

/// `RestartRound` (0x77920) for LMS: new round time, refunds, everyone back to observer with a
/// forced respawn in 3 s, map items respawned.
pub fn restart_round(now: f32) {
    let rt = cvars::round_time();
    let timer = state::with(|g| {
        g.round_end = now + rt;
        g.round_restart = 0.0;
        g.round_timer
    });
    if timer {
        msg::all(Msg::RoundTime, |w| RoundTime { secs: rt.round() as u8, state: 0 }.write(w));
    }
    for e in super::players() {
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
    teams::recount();
    crate::entities::ts::respawn_entities();
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
            total += a.price * rounds / a.per_buy;
        }
    }
    total
}

/// `restartround` server command → `ResetRestartRound(2)`: LMS zeroes scores and restarts in 1 s.
pub fn reset_restart_round() {
    let now = engine().time();
    let lms = state::with(|g| g.lms && g.mode == Mode::Dm);
    if lms {
        state::with(|g| {
            for t in &mut g.teams {
                t.score = 0;
            }
            g.round_restart = now + 1.0;
        });
    }
}

/// The One: every 10 s, if nobody is The One, pick a random living player.
fn the_one_check(now: f32) {
    let mut exists = false;
    let mut others = 0;
    let mut candidates = Vec::new();
    for e in super::players() {
        let one = other(e, |p| p.ts.is_the_one).unwrap_or(false);
        let v = e.v();
        if one {
            exists = true;
        } else {
            others += 1;
        }
        if alive(v) && v.iuser1 == 0 {
            candidates.push(e);
        }
    }
    state::with(|g| {
        g.the_one_others = others;
        g.the_one_exists = exists;
    });
    if !exists && !candidates.is_empty() {
        let pick = candidates[engine().random_long(0, candidates.len() as i32 - 1) as usize];
        scoring::set_the_one_other(pick);
    }
    state::with(|g| g.next_the_one_check = now + 10.0);
}

#[allow(dead_code)]
fn unused() -> MsgWriter {
    messages::all(Msg::RoundTime)
}

#[cfg(test)]
mod tests {
    use super::next_map;

    #[test]
    fn mapcycle() {
        let c: Vec<String> = ["ts_a", "ts_b", "ts_c"].iter().map(|s| s.to_string()).collect();
        assert_eq!(next_map(&c, "ts_a", |_| true), "ts_b");
        assert_eq!(next_map(&c, "ts_c", |_| true), "ts_a");
        assert_eq!(next_map(&c, "other", |_| true), "ts_a");
        assert_eq!(next_map(&c, "ts_a", |m| m != "ts_b"), "ts_c");
        assert_eq!(next_map(&[], "ts_x", |_| true), "ts_x");
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
