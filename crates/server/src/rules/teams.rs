//! Teamplay bookkeeping (`CHalfLifeTeamplay`): default team, team change, TeamInfo/TeamNames,
//! team frag totals, The One's two teams. Team names per player index live in the rules state so
//! any code can read them without dispatching to the player.

use super::msg;
use super::{alive, netname, other, state};
use crate::damage::{DamageInfo, bits};
use crate::messages::Msg;
use crate::player::Player;
use crate::world::Cx;
use goldsrc::edict::EdictPtr;
use goldsrc::server::engine;
use goldsrc::sys::*;
use shared::protocol::rules::{ScoreInfo, TeamInfo, TeamNames, TeamScore};

fn idx(e: *mut edict_t) -> usize {
    if e.is_null() { 0 } else { engine().index_of(e).clamp(0, 32) as usize }
}

/// The team name of a player (`TeamID()`), empty outside teamplay.
pub fn team_name_of(e: *mut edict_t) -> String {
    let i = idx(e);
    state::with(|g| g.player_team.get(i).cloned().unwrap_or_default())
}

fn set_team_name(e: *mut edict_t, name: &str) {
    let i = idx(e);
    state::with(|g| {
        if g.player_team.len() < 33 {
            g.player_team.resize(33, String::new());
        }
        g.player_team[i] = name.to_owned();
    });
}

/// `PlayerRelationship == GR_TEAMMATE`.
pub fn same_team(a: *mut edict_t, b: *mut edict_t) -> bool {
    if !state::teamplay() || a.is_null() || b.is_null() {
        return false;
    }
    let (ta, tb) = (team_name_of(a), team_name_of(b));
    !ta.is_empty() && ta.eq_ignore_ascii_case(&tb)
}

/// `CTSGameRules::GetPlayerModel`: a random model from the team's list.
pub fn model_for(team: &str) -> String {
    let models = state::with(|g| {
        let i = g.team_index(team);
        if i < 0 { Vec::new() } else { g.teams[i as usize].models.clone() }
    });
    if models.is_empty() {
        return String::new();
    }
    models[engine().random_long(0, models.len() as i32 - 1) as usize].clone()
}

/// `RecountTeams`: player counts and team frag totals.
pub fn recount() {
    let players = super::players();
    let names: Vec<String> = players.iter().map(|&e| team_name_of(e)).collect();
    let frags: Vec<i32> = players.iter().map(|&e| e.v().frags as i32).collect();
    state::with(|g| {
        for t in &mut g.teams {
            t.players = 0;
            t.frags = 0;
        }
        for (n, f) in names.iter().zip(frags) {
            let i = g.team_index(n);
            if i >= 0 {
                g.teams[i as usize].players += 1;
                g.teams[i as usize].frags += f;
            }
        }
    });
}

pub fn update_team_scores() {
    if state::teamplay() {
        recount();
    }
}

/// `TeamWithFewestPlayers`.
fn fewest() -> String {
    recount();
    state::with(|g| g.teams.iter().filter(|t| !t.name.is_empty()).min_by_key(|t| t.players).map(|t| t.name.clone()).unwrap_or_default())
}

/// `SetDefaultPlayerTeam` (0xe44ac): the `team` infokey, The One split, or the smallest team.
pub fn set_default_team(p: &mut Player, cx: &mut Cx) {
    let e = cx.edict;
    let (the_one, names, defaultteam) = state::with(|g| (g.the_one, g.team_names(), super::cvars::on(c"mp_defaultteam")));
    let mut name = if the_one {
        names.get(if p.ts.is_the_one { 0 } else { 1 }).cloned().unwrap_or_default()
    } else {
        let cur = team_name_of(e);
        if !cur.is_empty() { cur } else { engine().info_key(e, c"team").to_string_lossy().into_owned() }
    };
    let valid = state::with(|g| g.team_index(&name) >= 0);
    if name.is_empty() || !valid || (defaultteam && !the_one) {
        name = if !defaultteam { fewest() } else { names.first().cloned().unwrap_or_default() };
    }
    apply_team(p, cx, &name);
}

/// Stores the team on the player (name, `Player::team`, `team` and `model` infokeys).
fn apply_team(p: &mut Player, cx: &mut Cx, name: &str) {
    let e = cx.edict;
    let old = team_name_of(e);
    set_team_name(e, name);
    let i = state::with(|g| g.team_index(name));
    p.team = (i + 1).max(0) as u8;
    p.ts.team_name = name.to_owned();
    let eng = engine();
    if !old.eq_ignore_ascii_case(name) || eng.info_key(e, c"team").to_bytes() != name.as_bytes() {
        let model = model_for(name);
        if !model.is_empty() {
            eng.set_client_key(e, "model", &model);
        }
        eng.set_client_key(e, "team", name);
    }
}

/// `CHalfLifeTeamplay::ChangePlayerTeam` (0xe488c).
pub fn change_team(p: &mut Player, cx: &mut Cx, name: &str, kill: bool, gib: bool) {
    let e = cx.edict;
    if kill && alive(cx.pev()) {
        let world = engine().entity_of_index(0);
        let b = if gib { bits::ALWAYSGIB } else { bits::NEVERGIB };
        crate::player::damage::take_damage(p, cx, &DamageInfo::new(world, world, 900.0, b));
    }
    apply_team(p, cx, name);
    let idx = cx.index();
    msg::all(Msg::TeamInfo, |w| TeamInfo { idx: idx as u8, team: name.to_owned() }.write(w));
    let s = ScoreInfo { idx: idx as u8, frags: cx.pev().frags as i16, deaths: p.deaths as i16, class: 0, team: p.team as i16 };
    msg::all(Msg::ScoreInfo, |w| s.write(w));
    send_team_names(std::ptr::null_mut());
    recount();
    let _ = e;
}

/// `TeamNames` to one player (or everyone with a null edict).
pub fn send_team_names(to: *mut edict_t) {
    let t = TeamNames { names: state::with(|g| g.team_names()) };
    if to.is_null() {
        msg::all(Msg::TeamNames, |w| t.write(w));
    } else {
        msg::one(Msg::TeamNames, to, |w| t.write(w));
    }
}

/// Teamplay part of `InitHUD`: TeamNames, the assignment notice, TeamInfo of everyone.
pub fn init_hud(p: &mut Player, cx: &mut Cx) {
    let e = cx.edict;
    send_team_names(e);
    recount();
    let name = p.ts.team_name.clone();
    let model = engine().info_key(e, c"model").to_string_lossy().into_owned();
    let text =
        if model.eq_ignore_ascii_case(&name) { format!("* you are on team '{name}'\n") } else { format!("* assigned to team {name}\n") };
    change_team(p, cx, &name, false, false);
    msg::say_one(e, cx.index(), &text);
    let eng = engine();
    for i in 1..=eng.max_clients() {
        let o = eng.client(i);
        if o.is_null() || !super::is_player(o) {
            continue;
        }
        let t = team_name_of(o);
        if state::with(|g| g.team_index(&t)) >= 0 {
            msg::one(Msg::TeamInfo, e, |w| TeamInfo { idx: i as u8, team: t.clone() }.write(w));
        }
    }
}

/// `TeamScore` of every team to all (after captures / wins).
pub fn send_team_scores() {
    let teams: Vec<(String, i32)> =
        state::with(|g| g.teams.iter().filter(|t| !t.name.is_empty()).map(|t| (t.name.clone(), t.score)).collect());
    for (team, score) in teams {
        let m = TeamScore { team, score: score as i16, deaths: 0 };
        msg::all(Msg::TeamScore, |w| m.write(w));
    }
}

/// The One mode: The One is on team 0, everyone else on team 1.
pub fn the_one_teams() {
    let names = state::with(|g| g.team_names());
    if names.len() < 2 {
        return;
    }
    for e in super::players() {
        let one = other(e, |p| p.ts.is_the_one).unwrap_or(false);
        let want = &names[if one { 0 } else { 1 }];
        if !team_name_of(e).eq_ignore_ascii_case(want) {
            set_team_name(e, want);
            let t = want.clone();
            other(e, |p| {
                p.team = if one { 1 } else { 2 };
                p.ts.team_name = t.clone();
            });
            let i = engine().index_of(e);
            msg::all(Msg::TeamInfo, |w| TeamInfo { idx: i as u8, team: want.clone() }.write(w));
        }
    }
    let _ = netname;
}
