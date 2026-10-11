//! The strategy states (bots.md §6): which modules run, and where to roam. One state per TS game
//! mode, chosen at spawn; combat vs. roaming is done with module bits. Modes 2 and 3 can't be
//! reached in TS 3.0, so only the DM state runs in practice; the other two are ported for
//! completeness.

use super::brain::{Brain, StateKind, bit, delay, is_zero, length, slow, sub};
use crate::player::Player;
use crate::rules::state;
use crate::world::Cx;
use goldsrc::edict::EdictPtr;
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::ffi::CString;

/// `BotArmamentEmpty` (0x1006e4c0): the first loadout entry's id is 0 (`pl+0x594`).
pub fn armament_empty(p: &Player) -> bool {
    p.inv.sv.armament.first().is_none_or(|a| a.id == 0)
}

/// `UTIL_FindEntityByClassname` over the whole edict list.
pub fn find_by_class(start: *mut edict_t, class: &str) -> *mut edict_t {
    let Ok(c) = CString::new(class) else { return std::ptr::null_mut() };
    let e = engine().find_by_string(start, c"classname", &c);
    if e.is_null() || e.is_free() { std::ptr::null_mut() } else { e }
}

/// `TSFoundRandomEntity(class)` (0x10073a30): up to 32 entities of the class, then
/// `RANDOM_LONG(0, n-1)` (no draw when there are none).
pub fn random_entity(class: &str) -> *mut edict_t {
    let mut list = Vec::new();
    let mut e = std::ptr::null_mut();
    while list.len() < 32 {
        e = find_by_class(e, class);
        if e.is_null() {
            break;
        }
        list.push(e);
    }
    if list.is_empty() {
        return std::ptr::null_mut();
    }
    let n = list.len() as i32;
    list[engine().random_long(0, n - 1).clamp(0, n - 1) as usize]
}

/// `CBaseEntity::Center` (vtable slot 60): the middle of the absolute box.
pub fn center(e: *mut edict_t) -> [f32; 3] {
    let v = e.v();
    [(v.absmin[0] + v.absmax[0]) * 0.5, (v.absmin[1] + v.absmax[1]) * 0.5, (v.absmin[2] + v.absmax[2]) * 0.5]
}

/// The current state's `Think` (state vtable slot 0).
pub fn think(b: &mut Brain, p: &mut Player, cx: &mut Cx) {
    let i = b.state as usize;
    let now = cx.time();
    if now < b.states[i].next_think {
        return;
    }
    // `min(0.1 / slow, 1.0)`: not scaled by skill.
    b.states[i].next_think = delay(now, 0.1, slow(cx.pev()), 1.0, 1.0);
    let v = cx.pev();
    if !crate::rules::alive(v) || v.iuser1 != 0 {
        // Dead or observing: buy a loadout, then respawn; chat stays on.
        b.set_modules(if armament_empty(p) { bit::BUY } else { bit::RESPAWN });
        b.enable(bit::CHAT);
        return;
    }
    b.enable(bit::VISION);
    b.enable(bit::CHAT);
    if b.num_enemies() != 0 {
        b.enable(bit::ATTACK);
        b.disable(bit::LOOK_AROUND);
        return;
    }
    if !is_zero(b.last_seen) {
        // Chase the last place an enemy was seen.
        b.goal = b.last_seen;
        b.last_seen = [0.0; 3];
        b.enable(bit::NAVIGATE);
        if b.state == StateKind::Dm {
            b.keep_goal = 1;
        }
        b.enable(bit::LOOK_AROUND);
        return;
    }
    if is_zero(b.goal) && b.states[i].next_pick < now {
        match b.state {
            StateKind::Dm => pick_dm(b, cx),
            StateKind::TeamRounds => pick_team_rounds(b, p),
            StateKind::Objective => {}
        }
        b.states[i].next_pick = (now as f64 + 3.0) as f32;
        b.keep_goal = 0;
        b.enable(bit::LOOK_AROUND);
        return;
    }
    b.keep_goal = 1;
    b.enable(bit::LOOK_AROUND);
}

fn set_goal(b: &mut Brain, goal: [f32; 3]) {
    b.goal = goal;
    b.enable(bit::NAVIGATE);
}

/// `PickRoamGoal` of `CBotStateDM` (0x1006ea93..0x1006ecc3): a `ts_dmhill` (1 in 5; none within
/// 64 units of it), a `ts_powerup` (1 in 5) or a spawn point. In teamplay the spawn point is any
/// team's.
fn pick_dm(b: &mut Brain, cx: &mut Cx) {
    let eng = engine();
    let r = eng.random_long(0, 4);
    if r == 0 {
        let e = random_entity("ts_dmhill");
        if !e.is_null() {
            let c = center(e);
            if length(sub(cx.pev().origin, c)) > 64.0 {
                set_goal(b, c);
            }
            return;
        }
    } else if r == 2 {
        let e = random_entity("ts_powerup");
        if !e.is_null() {
            set_goal(b, e.v().origin);
            return;
        }
    }
    // Spawn points (also when there is no hill or power-up).
    let class = if state::teamplay() {
        let teams: Vec<(String, String, bool)> =
            state::with(|g| g.teams.iter().map(|t| (t.spawn_class.clone(), t.fallback.clone(), !t.name.is_empty())).collect());
        // `CTSGameRules::GetTeamsNum` (0x1004df10) counts the named teams; the draw indexes the
        // team table by position (guard: an index outside the table falls back to the DM spawns).
        let n = teams.iter().filter(|t| t.2).count() as i32;
        let i = eng.random_long(0, n - 1);
        match usize::try_from(i).ok().and_then(|i| teams.get(i)) {
            Some((spawn, fallback, _)) => {
                if find_by_class(std::ptr::null_mut(), spawn).is_null() {
                    fallback.clone()
                } else {
                    spawn.clone()
                }
            }
            None => "info_player_deathmatch".to_owned(),
        }
    } else {
        "info_player_deathmatch".to_owned()
    };
    let e = random_entity(&class);
    if !e.is_null() {
        set_goal(b, e.v().origin);
    }
}

/// `PickRoamGoal` of `CBotStateTeamRounds` (0x1006eebb..0x1006f055): the other team's spawn
/// points: team 0's for a bot on team 1, team 1's for any other bot (the fallback class when
/// the spawn class has none).
fn pick_team_rounds(b: &mut Brain, p: &Player) {
    let ti = state::with(|g| g.team_index(&p.ts.team_name));
    let (a, f) = state::with(|g| {
        let t = if ti == 1 { 0 } else { 1 };
        g.teams.get(t).map(|t| (t.spawn_class.clone(), t.fallback.clone())).unwrap_or_default()
    });
    let collect = |class: &str| {
        let mut list = Vec::new();
        let mut e = std::ptr::null_mut();
        while list.len() < 32 {
            e = find_by_class(e, class);
            if e.is_null() {
                break;
            }
            list.push(e);
        }
        list
    };
    let mut list = collect(&a);
    if list.is_empty() {
        list = collect(&f);
    }
    if !list.is_empty() {
        let n = list.len() as i32;
        let e = list[engine().random_long(0, n - 1).clamp(0, n - 1) as usize];
        set_goal(b, e.v().origin);
    }
}
