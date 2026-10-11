//! The game rules state: `g_pGameRules` (DM / Teamplay) and the `CTSGameRules TSGameRules`
//! singleton folded into one struct (decomp gamerules-entities.md §1, §8).

use super::cvars;
use goldsrc::Global;
use goldsrc::server::engine;
use std::cell::RefCell;

/// `CTSGameRules::m_mode`. Modes 2/3 (round TDM, objectives) are unreachable in TS 3.0.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mode {
    #[default]
    Dm = 0,
    TeamDm = 1,
    RoundTdm = 2,
    Objective = 3,
}

#[derive(Clone, Debug, Default)]
pub struct Team {
    pub name: String,
    /// Up to 3 player models.
    pub models: Vec<String>,
    /// `info_player_team%i`, then the fallback.
    pub spawn_class: String,
    pub fallback: String,
    pub cash: i32,
    /// Round wins and captures (`TeamScore`); unused in continuous TDM.
    pub score: i32,
}

/// Global slow motion (`TSGameRules +0x560..+0x56c`).
#[derive(Clone, Copy, Debug, Default)]
pub struct GlobalSlow {
    pub active: bool,
    pub amount: f32,
    pub until: f32,
    pub next: f32,
}

#[derive(Debug, Default)]
pub struct GameRules {
    pub installed: bool,
    pub mode: Mode,
    /// `g_teamplay`: the CHalfLifeTeamplay object is in use.
    pub teamplay: bool,
    pub lms: bool,
    pub the_one: bool,
    /// worldspawn `gamemode` (0 = server decides), consumed by `decide_gameplay`.
    pub forced_gamemode: i32,
    pub map_teams: String,
    pub map_team_models: String,
    pub teams: Vec<Team>,
    /// Team name of each player index (`m_szTeamName`).
    pub player_team: Vec<String>,

    /// Auto power-up spawner: 0xff before the first frame, 1 on, 0 off (the map has its own
    /// `ts_powerup`s).
    pub powerup_spawner: u8,
    pub next_powerup: f32,
    pub round_end: f32,
    pub round_restart: f32,
    pub round_timer: bool,
    /// `+0x18`: slow-motion power-ups refused while set.
    pub slow_blocked: bool,
    pub global_slow: GlobalSlow,
    /// Allowed weapon categories (SrvSett 3) and the map's base mask.
    pub allowed_weapons: i32,
    pub base_weapons: i32,
    /// `ts_mapglobals` flag 64: everyone has super jump.
    pub superjump_map: bool,
    pub the_one_exists: bool,
    pub next_the_one_check: f32,
    /// Players other than The One (regeneration rate).
    pub the_one_others: i32,
    pub check_deads_at: f32,
    pub last_srvsett: [i32; 4],
    /// Chat map votes (`+0x684..+0x1864`).
    pub votes: super::votes::Votes,

    // CHalfLifeMultiplay
    pub game_over: bool,
    pub intermission_start: f32,
    pub intermission_end: f32,
    /// `m_iEndIntermissionButtonHit` (`+8`): a player pressed a button during intermission.
    pub end_intermission_button_hit: bool,
    pub last_frags_left: i32,
    pub last_time_left: i32,

    // CHalfLifeTeamplay
    /// `m_DisableDeathMessage` / `m_DisableDeathPenalty` (`+0x20c`/`+0x210`), always set together:
    /// on while `ChangePlayerTeam` kills the player, so `PlayerKilled` does nothing.
    pub death_suppressed: bool,
    /// `m_szTeamList` (`+0xc`): the listed teams, which the Teamplay constructor rebuilds from the
    /// `TSGameRules` team names.
    pub team_list: Vec<String>,
    /// `m_teamLimit` (`+0x214`): only listed teams are valid. The constructor sets it and
    /// `RecountTeams` (0xe5608) clears it for good when the list holds fewer than two teams.
    pub team_limit: bool,
    /// `team_names` (0x19e760): the teams `RecountTeams` found, the first `num_teams` (0x120540)
    /// of 32 slots. The slots past `num_teams` keep stale names, and `SetDefaultPlayerTeam` reads
    /// slot 0 for `mp_defaultteam` whatever the count.
    pub tp_names: Vec<String>,
    pub num_teams: usize,
    /// `team_scores` (0x19e960): frags per `team_names` slot, summed by `RecountTeams` and used by
    /// the fraglimit (`CHalfLifeTeamplay::Think` 0xe412c).
    pub tp_scores: Vec<i32>,
}

/// Slots in `team_names` / `team_scores`.
const MAX_TP_TEAMS: usize = 32;

pub static RULES: Global<RefCell<Option<GameRules>>> = Global(RefCell::new(None));

/// Runs `f` on the rules state. Keep it short and don't call code that can re-enter the rules
/// (entity spawn/think/use/damage) inside `f`.
pub fn with<R>(f: impl FnOnce(&mut GameRules) -> R) -> R {
    let mut g = RULES.0.borrow_mut();
    f(g.get_or_insert_with(GameRules::default))
}

/// Copy of a few frequently read flags (no borrow held).
pub fn mode() -> (Mode, bool, bool, bool) {
    with(|g| (g.mode, g.teamplay, g.lms, g.the_one))
}

pub fn teamplay() -> bool {
    with(|g| g.teamplay)
}

pub fn the_one_mode() -> bool {
    with(|g| g.the_one)
}

pub fn lms() -> bool {
    with(|g| g.lms)
}

/// `CashBased()` (0x76abc): hard-wired off in TS 3.0, so nothing is ever charged. The cash code
/// paths are ported behind it.
pub fn cash_based() -> bool {
    false
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

/// Default team models (`defTeamModels`).
const DEFAULT_MODELS: [&str; 4] = ["seal;merc", "gordon;laurence", "agent", "hitman;castor"];

impl GameRules {
    /// `InstallGameRules` + `CTSGameRules::Init` (+ the Teamplay constructor's team setup).
    /// Called when worldspawn spawns (its keyvalues have been parsed by then).
    pub fn install(&mut self) {
        let forced = self.forced_gamemode;
        let (teamplay, lms, the_one) =
            decide_gameplay(forced, cvars::on(c"mp_teamplay"), cvars::on(c"lastmanstanding"), cvars::on(c"mp_theonemode"));
        let (map_teams, map_models) = (std::mem::take(&mut self.map_teams), std::mem::take(&mut self.map_team_models));
        *self = GameRules::default();
        self.installed = true;
        self.teamplay = teamplay;
        self.lms = lms;
        self.the_one = the_one;
        self.mode = if teamplay { Mode::TeamDm } else { Mode::Dm };
        self.player_team = vec![String::new(); 33];
        self.init_teams();
        if teamplay {
            let list = if !map_teams.is_empty() && cvars::on(c"mp_teamoverride") { map_teams.clone() } else { cvars::s(c"mp_teamlist") };
            if !list.is_empty() {
                self.set_team_list(&list);
            }
            self.init_teamplay_list();
            let models = if !map_models.is_empty() { map_models.clone() } else { cvars::s(c"mp_teammodels") };
            if !models.is_empty() {
                self.set_team_model_lists(&models);
            }
        }
        self.map_teams = map_teams;
        self.map_team_models = map_models;

        let now = engine().time();
        self.powerup_spawner = 0xff;
        self.next_powerup = 0.0;
        self.round_end = 0.0;
        self.round_restart = now + 5.0;
        self.round_timer = true;
        self.allowed_weapons = 0x1f;
        self.base_weapons = 0x1f;
        self.next_the_one_check = now + 10.0;
        self.votes.next_check = now + 10.0;
        self.last_srvsett = [-1; 4];
        self.last_frags_left = -1;
        self.last_time_left = -1;
        engine().print(&format!(
            "[ts] game rules: {} ({})\n",
            match (self.teamplay, self.lms, self.the_one) {
                (_, _, true) => "The One",
                (_, true, _) => "Last Man Standing",
                (true, _, _) => "Team DeathMatch",
                _ => "DeathMatch",
            },
            self.teams.iter().filter(|t| !t.name.is_empty()).map(|t| t.name.as_str()).collect::<Vec<_>>().join(";")
        ));
    }

    /// `CTSGameRules::InitTeams`.
    fn init_teams(&mut self) {
        self.teams = (0..4)
            .map(|i| Team {
                name: format!("Team{}", i + 1),
                spawn_class: format!("info_player_team{}", i + 1),
                fallback: "info_player_deathmatch".into(),
                cash: 0, // initial_cash.value is 0 because the cvar is never registered
                ..Default::default()
            })
            .collect();
        for (i, models) in DEFAULT_MODELS.into_iter().enumerate() {
            self.set_team_models(i, models);
        }
    }

    /// `CTSGameRules::SetTeamList`: up to 4 teams (2 in The One, where team 0 is "The ONE" and
    /// the first listed name is skipped, as in TS).
    pub fn set_team_list(&mut self, list: &str) {
        for t in &mut self.teams {
            t.name.clear();
        }
        let max = if self.the_one { 2 } else { 4 };
        let mut n = 0;
        for name in list.split(';').filter(|s| !s.is_empty()) {
            if n >= max {
                break;
            }
            if n == 0 && self.the_one {
                self.teams[0].name = "The ONE".into();
                n = 1;
                continue;
            }
            self.teams[n].name = name.chars().take(15).collect();
            n += 1;
        }
    }

    fn set_team_models(&mut self, team: usize, list: &str) {
        self.teams[team].models = list.split(';').filter(|s| !s.is_empty()).take(3).map(|s| s.chars().take(15).collect()).collect();
    }

    /// `SetTeamModelList(char*)`: teams separated by `|`, models by `;`.
    pub fn set_team_model_lists(&mut self, list: &str) {
        let parts: Vec<&str> = list.split('|').collect();
        for i in 0..4 {
            self.set_team_models(i, parts.get(i).copied().unwrap_or(""));
        }
    }

    /// The Teamplay constructor's team setup after `SetTeamList` (0xe3e48): `m_szTeamList` from
    /// the non-empty `TSGameRules` names, `m_teamLimit` on, then `RecountTeams` with no players.
    pub fn init_teamplay_list(&mut self) {
        self.team_list = self.teams.iter().filter(|t| !t.name.is_empty()).map(|t| t.name.clone()).collect();
        self.team_limit = true;
        self.recount_teams(&[]);
    }

    /// The bookkeeping of `CHalfLifeTeamplay::RecountTeams` (0xe5608), given every player's team
    /// name and frags in index order. The listed teams come first, deduplicated; fewer than two
    /// mean none at all and no team limit, and then every player's team that isn't known yet is
    /// added, `""` included (`GetTeamIndex("")` is -1, so each teamless player adds a slot).
    pub fn recount_teams(&mut self, players: &[(String, f32)]) {
        self.tp_names.resize(MAX_TP_TEAMS, String::new());
        self.tp_scores = vec![0; MAX_TP_TEAMS];
        self.num_teams = 0;
        for name in &self.team_list {
            if self.tp_team_index(name) < 0 && self.num_teams < MAX_TP_TEAMS {
                self.tp_names[self.num_teams] = name.clone();
                self.num_teams += 1;
            }
        }
        if self.num_teams < 2 {
            self.num_teams = 0;
            self.team_limit = false;
        }
        for (name, frags) in players {
            let mut i = self.tp_team_index(name);
            if i < 0 && !self.team_limit && self.num_teams < MAX_TP_TEAMS {
                i = self.num_teams as i32;
                self.tp_names[self.num_teams] = name.clone();
                self.num_teams += 1;
            }
            if i >= 0 {
                let s = &mut self.tp_scores[i as usize];
                *s = (*s as f32 + frags).round_ties_even() as i32;
            }
        }
    }

    /// `team_names[0..num_teams]`: what `TeamNames` sends and `jointeam` indexes.
    pub fn team_names(&self) -> Vec<String> {
        self.tp_names.iter().take(self.num_teams).cloned().collect()
    }

    /// `team_scores[0..num_teams]`.
    pub fn team_scores(&self) -> &[i32] {
        &self.tp_scores[..self.num_teams.min(self.tp_scores.len())]
    }

    /// `CHalfLifeTeamplay::GetTeamIndex` (0xe5434): the `team_names` slot (case-insensitive), -1
    /// for none or `""`. `ScoreInfo` sends it + 1.
    pub fn tp_team_index(&self, name: &str) -> i32 {
        if name.is_empty() {
            return -1;
        }
        self.tp_names.iter().take(self.num_teams).position(|t| t.eq_ignore_ascii_case(name)).map_or(-1, |i| i as i32)
    }

    /// `CHalfLifeTeamplay::IsValidTeam` (0xe54d4): any name without a team limit (`""` too).
    pub fn is_valid_team(&self, name: &str) -> bool {
        !self.team_limit || self.tp_team_index(name) != -1
    }

    /// `CHalfLifeTeamplay::TeamWithFewestPlayers` (0xe5510), given every player's team name: the
    /// first team with the fewest players, `""` when there is none (TS would copy from NULL).
    pub fn team_with_fewest(&self, players: &[String]) -> String {
        let mut count = [0; MAX_TP_TEAMS];
        for name in players {
            if let Ok(i) = usize::try_from(self.tp_team_index(name)) {
                count[i] += 1;
            }
        }
        let mut best = None;
        let mut min = MAX_TP_TEAMS as i32;
        for (i, &c) in count.iter().enumerate().take(self.num_teams) {
            if c < min {
                min = c;
                best = Some(i);
            }
        }
        best.and_then(|i| self.tp_names.get(i).cloned()).unwrap_or_default()
    }

    /// `team_names[0]`, which `SetDefaultPlayerTeam` takes under `mp_defaultteam` without looking
    /// at `num_teams`: with no team limit it can be a player's team or a stale listed name.
    pub fn first_team_slot(&self) -> String {
        self.tp_names.first().cloned().unwrap_or_default()
    }

    /// `TSGameRules` teams 0 and 1 (`0x1781c8`/`0x178254`), The One's and everyone else's in The
    /// One mode (`SetDefaultPlayerTeam`, `ChangePlayerTeam`).
    pub fn the_one_team_names(&self) -> [String; 2] {
        [0, 1].map(|i| self.teams.get(i).map(|t| t.name.clone()).unwrap_or_default())
    }

    /// `CTSGameRules::GetTeamIndex` (0x79b04): the `TSGameRules` team (case-insensitive), -1 if
    /// none. Models, spawn points, CTB and triggers use it; `ScoreInfo` uses `tp_team_index`.
    pub fn team_index(&self, name: &str) -> i32 {
        if name.is_empty() {
            return -1;
        }
        self.teams.iter().position(|t| !t.name.is_empty() && t.name.eq_ignore_ascii_case(name)).map_or(-1, |i| i as i32)
    }

    /// `CHalfLifeTeamplay::GetIndexedTeamName` (0xe54a0): the team at `index`, or `""` when it is
    /// out of range (`ChangePlayerTeam` then picks the smallest team).
    pub fn indexed_team_name(&self, index: i32) -> String {
        usize::try_from(index).ok().and_then(|i| self.team_names().get(i).cloned()).unwrap_or_default()
    }

    /// `CTSGameRules::CallCheckDeads` (0x79bc0): runs `CheckDeads` in 1 s, but only while no round
    /// restart is pending.
    pub fn call_check_deads(&mut self, now: f32) {
        if self.round_restart == 0.0 {
            self.check_deads_at = now + 1.0;
        }
    }

    /// `IsCTB()`: continuous TDM with a `ts_briefcase` on the map.
    pub fn is_ctb(&self) -> bool {
        self.mode == Mode::TeamDm && !engine().find_by_string(std::ptr::null_mut(), c"classname", c"ts_briefcase").is_null()
    }

    /// `GetRandomPowerup(mask)`: uniform among the set bits 0x1..0x100, never armour, and no
    /// super jump when the map already gives it to everyone.
    pub fn random_powerup(&self, mask: u16) -> u16 {
        let choices: Vec<u16> = (0..9)
            .map(|i| 1u16 << i)
            .filter(|&b| mask & b != 0 && b != shared::ts::powerup::ARMOR && !(self.superjump_map && b == shared::ts::powerup::SUPERJUMP))
            .collect();
        if choices.is_empty() {
            return 0;
        }
        choices[engine().random_long(0, choices.len() as i32 - 1) as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn team_lists() {
        let mut g = GameRules::default();
        g.init_teams();
        g.set_team_list("hitman;gordon");
        g.init_teamplay_list();
        assert_eq!(g.team_names(), ["hitman", "gordon"]);
        assert_eq!(g.team_index("GORDON"), 1);
        assert_eq!(g.team_index("seal"), -1);
        g.the_one = true;
        g.set_team_list("hitman;gordon;agent");
        g.init_teamplay_list();
        assert_eq!(g.team_names(), ["The ONE", "gordon"]);
        g.set_team_model_lists("a;b|c");
        assert_eq!(g.teams[0].models, ["a", "b"]);
        assert_eq!(g.teams[1].models, ["c"]);
        assert!(g.teams[2].models.is_empty());
    }

    /// The rules after the Teamplay constructor for `list`.
    fn teamplay(list: &str) -> GameRules {
        let mut g = GameRules::default();
        g.init_teams();
        g.set_team_list(list);
        g.init_teamplay_list();
        g
    }

    fn players(list: &[(&str, f32)]) -> Vec<(String, f32)> {
        list.iter().map(|&(n, f)| (n.to_owned(), f)).collect()
    }

    #[test]
    fn recount_with_a_team_limit_keeps_the_listed_teams() {
        let mut g = teamplay("hitman;gordon;HITMAN");
        assert!(g.team_limit);
        assert_eq!(g.team_names(), ["hitman", "gordon"], "duplicates are dropped");
        g.recount_teams(&players(&[("gordon", 3.0), ("seal", 5.0), ("GORDON", 2.0), ("", 1.0)]));
        assert_eq!(g.team_names(), ["hitman", "gordon"]);
        assert_eq!(g.team_scores(), [0, 5]);
        assert_eq!(g.tp_team_index("Gordon"), 1);
        assert!(g.is_valid_team("HITMAN"));
        assert!(!g.is_valid_team("seal"));
        assert!(!g.is_valid_team(""));
    }

    #[test]
    fn recount_without_a_team_limit_takes_the_players_teams() {
        let mut g = teamplay("hitman");
        assert!(!g.team_limit, "RecountTeams drops the limit below two teams");
        assert!(g.team_names().is_empty());
        // team_names[0] still holds the listed team until a player's team overwrites it.
        assert_eq!(g.first_team_slot(), "hitman");
        assert!(g.is_valid_team("anything") && g.is_valid_team(""));
        g.recount_teams(&players(&[("seal", 2.0), ("", 1.0), ("SEAL", 3.0), ("", 4.0), ("merc", 1.0)]));
        // Each teamless player adds an empty team: GetTeamIndex("") is -1.
        assert_eq!(g.team_names(), ["seal", "", "", "merc"]);
        assert_eq!(g.team_scores(), [5, 1, 4, 1]);
        assert_eq!(g.first_team_slot(), "seal");
        assert_eq!(g.tp_team_index("seal"), 0);
        assert_eq!(g.tp_team_index(""), -1);
        assert_eq!(g.indexed_team_name(3), "merc");
        // The limit never comes back, and the TSGameRules list is untouched.
        g.recount_teams(&[]);
        assert!(!g.team_limit && g.team_names().is_empty());
        assert_eq!(g.team_index("hitman"), 0);
        assert_eq!(g.team_index("seal"), -1);
    }

    #[test]
    fn team_with_fewest_players() {
        let g = teamplay("hitman;gordon;agent");
        let names = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(g.team_with_fewest(&names(&[])), "hitman");
        assert_eq!(g.team_with_fewest(&names(&["hitman", "GORDON", "seal", ""])), "agent");
        assert_eq!(g.team_with_fewest(&names(&["hitman", "agent"])), "gordon");
        // No teams at all (TS would copy from a NULL pointer).
        let mut g = teamplay("hitman");
        assert_eq!(g.team_with_fewest(&names(&["seal"])), "");
        g.recount_teams(&players(&[("seal", 0.0), ("merc", 0.0), ("seal", 0.0)]));
        assert_eq!(g.team_with_fewest(&names(&["seal", "merc", "seal"])), "merc");
    }

    #[test]
    fn the_one_team_names_come_from_the_ts_list() {
        let mut g = GameRules::default();
        g.init_teams();
        g.the_one = true;
        g.set_team_list("hitman;gordon");
        g.init_teamplay_list();
        assert_eq!(g.the_one_team_names(), ["The ONE", "gordon"]);
        assert_eq!(g.team_names(), ["The ONE", "gordon"]);
    }

    #[test]
    fn indexed_team_name_is_empty_out_of_range() {
        let g = teamplay("hitman;gordon");
        assert_eq!(g.indexed_team_name(0), "hitman");
        assert_eq!(g.indexed_team_name(1), "gordon");
        assert_eq!(g.indexed_team_name(2), "");
        assert_eq!(g.indexed_team_name(-1), "");
    }

    #[test]
    fn call_check_deads_waits_for_a_pending_restart() {
        let mut g = GameRules::default();
        g.call_check_deads(10.0);
        assert_eq!(g.check_deads_at, 11.0);
        g.check_deads_at = 0.0;
        g.round_restart = 12.0;
        g.call_check_deads(10.0);
        assert_eq!(g.check_deads_at, 0.0);
    }
}
