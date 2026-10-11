//! Teamplay bookkeeping (`CHalfLifeTeamplay`): default team, team change, TeamInfo/TeamNames,
//! team frag totals, The One's two teams. Team names per player index live in the rules state so
//! any code can read them without dispatching to the player.

use super::msg;
use super::{netname, other, state};
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

/// Stores the team on the player: the name and `Player::team`.
fn store_team(p: &mut Player, e: *mut edict_t, name: &str) {
    set_team_name(e, name);
    let i = state::with(|g| g.team_index(name));
    p.team = (i + 1).max(0) as u8;
    p.ts.team_name = name.to_owned();
}

/// Stores the team on the player (name, `Player::team`, `team` and `model` infokeys).
fn apply_team(p: &mut Player, cx: &mut Cx, name: &str) {
    let e = cx.edict;
    let old = team_name_of(e);
    store_team(p, e, name);
    let eng = engine();
    if !old.eq_ignore_ascii_case(name) || eng.info_key(e, c"team").to_bytes() != name.as_bytes() {
        let model = model_for(name);
        if !model.is_empty() {
            eng.set_client_key(e, "model", &model);
        }
        eng.set_client_key(e, "team", name);
    }
}

/// The `model` key `ChangePlayerTeam` always sets (0xe4a0f..0xe4b28): with a team limit
/// (`+0x214`) `CTSGameRules::GetPlayerModel` (0x79a08), a random one of the team's models or `""`
/// when it has none (`pick(n)` gives `RANDOM_LONG(0, n - 1)`); without one the team name itself.
fn change_team_model(team_limit: bool, team: &str, models: &[String], pick: impl FnOnce(i32) -> i32) -> String {
    if !team_limit {
        return team.to_owned();
    }
    if models.is_empty() {
        return String::new();
    }
    models.get(pick(models.len() as i32) as usize).cloned().unwrap_or_default()
}

/// The team `ChangePlayerTeam` (0xe488c) settles on: in The One mode The One goes to the first
/// team and everyone else to the second, whatever was asked; otherwise an empty or unknown name
/// means the smallest team (`None`).
fn change_team_choice(requested: &str, the_one: bool, is_the_one: bool, names: &[String]) -> Option<String> {
    if the_one {
        return names.get(if is_the_one { 0 } else { 1 }).cloned();
    }
    names.iter().find(|n| !requested.is_empty() && n.eq_ignore_ascii_case(requested)).map(|_| requested.to_owned())
}

/// Whether `ChangePlayerTeam` sends the player to observer with a 5 s respawn (0xe4c38..0xe4ca1):
/// every change that isn't a kill, outside The One mode, even to the same team or while already
/// observing.
fn change_team_observes(kill: bool, the_one: bool) -> bool {
    !kill && !the_one
}

/// `CHalfLifeTeamplay::ChangePlayerTeam` (0xe488c): `jointeam` and the teamplay `InitHUD`.
pub fn change_team(p: &mut Player, cx: &mut Cx, name: &str, kill: bool, gib: bool) {
    // TS doesn't check that the player is alive (0xe48e2..0xe4954): `TakeDamage(900)` with the
    // death message and penalty (`+0x20c`/`+0x210`) off for the call.
    if kill {
        let world = engine().entity_of_index(0);
        let b = if gib { bits::ALWAYSGIB } else { bits::NEVERGIB };
        state::with(|g| g.death_suppressed = true);
        crate::player::damage::take_damage(p, cx, &DamageInfo::new(world, world, 900.0, b));
        state::with(|g| g.death_suppressed = false);
    }
    let (the_one, names) = state::with(|g| (g.the_one, g.team_names()));
    let name = change_team_choice(name, the_one, p.ts.is_the_one, &names).unwrap_or_else(fewest);
    let e = cx.edict;
    store_team(p, e, &name);
    let (limit, models) = state::with(|g| {
        let i = g.team_index(&name);
        (g.team_limit(), if i < 0 { Vec::new() } else { g.teams[i as usize].models.clone() })
    });
    let eng = engine();
    eng.set_client_key(e, "model", &change_team_model(limit, &name, &models, |n| eng.random_long(0, n - 1)));
    eng.set_client_key(e, "team", &name);
    recount();
    let idx = cx.index();
    msg::all(Msg::TeamInfo, |w| TeamInfo { idx: idx as u8, team: name.clone() }.write(w));
    let s = ScoreInfo { idx: idx as u8, frags: cx.pev().frags as i16, deaths: p.deaths as i16, class: 0, team: p.team as i16 };
    msg::all(Msg::ScoreInfo, |w| s.write(w));
    if change_team_observes(kill, the_one) {
        let (o, a) = (cx.pev().origin, cx.pev().v_angle);
        super::lifecycle::start_observer(p, cx, o, a);
        super::lifecycle::set_respawn_time(p, cx, 5.0);
    }
    send_team_names(std::ptr::null_mut());
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

#[cfg(test)]
mod tests {
    use super::{change_team_choice, change_team_model, change_team_observes};

    fn names() -> Vec<String> {
        vec!["hitman".to_owned(), "gordon".to_owned()]
    }

    #[test]
    fn change_team_picks_a_listed_team_or_the_smallest() {
        assert_eq!(change_team_choice("GORDON", false, false, &names()).as_deref(), Some("GORDON"));
        assert_eq!(change_team_choice("hitman", false, false, &names()).as_deref(), Some("hitman"));
        // `jointeam` out of range: GetIndexedTeamName gives "" and TS picks the smallest team.
        assert_eq!(change_team_choice("", false, false, &names()), None);
        assert_eq!(change_team_choice("seal", false, false, &names()), None);
    }

    #[test]
    fn change_team_in_the_one_mode_ignores_the_request() {
        assert_eq!(change_team_choice("gordon", true, true, &names()).as_deref(), Some("hitman"));
        assert_eq!(change_team_choice("hitman", true, false, &names()).as_deref(), Some("gordon"));
    }

    #[test]
    fn change_team_observes_unless_kill_or_the_one() {
        assert!(change_team_observes(false, false));
        assert!(!change_team_observes(true, false));
        assert!(!change_team_observes(false, true));
    }

    #[test]
    fn change_team_model_follows_the_team_limit() {
        let models = vec!["seal".to_owned(), "merc".to_owned()];
        // No team limit: the team name is the model, whatever the list holds.
        assert_eq!(change_team_model(false, "hitman", &models, |_| unreachable!()), "hitman");
        // With one: GetPlayerModel picks from the team's models (RANDOM_LONG(0, n - 1)).
        assert_eq!(change_team_model(true, "hitman", &models, |n| n - 1), "merc");
        assert_eq!(change_team_model(true, "hitman", &models, |_| 0), "seal");
        // A team without models (or an unknown one) still sets the key, to "".
        assert_eq!(change_team_model(true, "hitman", &[], |_| unreachable!()), "");
    }
}
