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
    /// Teamplay frags (SDK `team_scores`), used by the fraglimit.
    pub frags: i32,
    pub players: i32,
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
}

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

    /// Valid team names (non-empty), in order.
    pub fn team_names(&self) -> Vec<String> {
        self.teams.iter().filter(|t| !t.name.is_empty()).map(|t| t.name.clone()).collect()
    }

    /// `CTSGameRules::GetTeamIndex` (case-insensitive), -1 if none.
    pub fn team_index(&self, name: &str) -> i32 {
        if name.is_empty() {
            return -1;
        }
        self.teams.iter().position(|t| !t.name.is_empty() && t.name.eq_ignore_ascii_case(name)).map_or(-1, |i| i as i32)
    }

    /// `m_teamLimit` (`+0x214`): the Teamplay constructor sets it and `RecountTeams` (0xe5608)
    /// clears it when the list holds fewer than two teams.
    pub fn team_limit(&self) -> bool {
        self.team_names().len() >= 2
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
        assert_eq!(g.team_names(), ["hitman", "gordon"]);
        assert_eq!(g.team_index("GORDON"), 1);
        assert_eq!(g.team_index("seal"), -1);
        assert!(g.team_limit());
        g.set_team_list("hitman");
        assert!(!g.team_limit(), "RecountTeams drops the limit below two teams");
        g.the_one = true;
        g.set_team_list("hitman;gordon;agent");
        assert_eq!(g.team_names(), ["The ONE", "gordon"]);
        g.set_team_model_lists("a;b|c");
        assert_eq!(g.teams[0].models, ["a", "b"]);
        assert_eq!(g.teams[1].models, ["c"]);
        assert!(g.teams[2].models.is_empty());
    }

    #[test]
    fn indexed_team_name_is_empty_out_of_range() {
        let mut g = GameRules::default();
        g.init_teams();
        g.set_team_list("hitman;gordon");
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
