//! The game rules state: `g_pGameRules` (DM / Teamplay) and the `CTSGameRules TSGameRules`
//! singleton folded into one struct (decomp gamerules-entities.md §1, §8).

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

/// Team and model names are C strings in TS (`char[16]` fields filled by `strcpy`/`strncpy`,
/// compared with `strcasecmp`), so the port keeps them as bytes: a CP1252 0xE9 in `mp_teamlist`
/// or a `model`/`team` key stays 0xE9 in `TeamInfo` and matches the same bytes (#188).
#[derive(Clone, Debug, Default)]
pub struct Team {
    pub name: Vec<u8>,
    /// Up to 3 player models.
    pub models: Vec<Vec<u8>>,
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
    /// `deathmatch` was 0 at install: `g_pGameRules` is `CHalfLifeRules` (0xde6fc), the
    /// single-player rules ([`single_player`]).
    pub single_player: bool,
    pub mode: Mode,
    /// `g_teamplay`: the CHalfLifeTeamplay object is in use.
    pub teamplay: bool,
    pub lms: bool,
    pub the_one: bool,
    /// worldspawn `gamemode` (0 = server decides), consumed by `decide_gameplay`.
    pub forced_gamemode: i32,
    /// worldspawn `mapteams` / `teammodels`, as the map gives them.
    pub map_teams: Vec<u8>,
    pub map_team_models: Vec<u8>,
    pub teams: Vec<Team>,
    /// Team name of each player index (`m_szTeamName`).
    pub player_team: Vec<Vec<u8>>,

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
    /// `+0x5884`: a weapons file (`loadweapons`) changed the weapon parameters since the last
    /// `Init`; `InitHUD` then sends them to each joiner. The parameters themselves
    /// (`shared::weapons::table`, TS's `global_weapon_info`) outlive the map.
    pub custom_weapons: bool,
    /// The objective HUD list and the round's managers (`+0x24c..`, `+0x2d0..`, `+0x554`).
    pub objectives: crate::entities::ts::objective::Objectives,
    /// `+0x588c`: the fraction kevlar absorbs (`armor_absorb` of a weapons file).
    pub armor_absorb: f32,
    /// `+0x5888`: `health_absorb` of a weapons file; nothing reads it.
    pub health_absorb: f32,
    pub the_one_exists: bool,
    pub next_the_one_check: f32,
    /// Players other than The One (regeneration rate).
    pub the_one_others: i32,
    pub check_deads_at: f32,
    /// The `SrvSett` copies `CTSGameRules::Think` keeps (the 3.0 plugin's `think`): `[0]` `+0x578`
    /// `(int)realbullet`, `[1]` `+0x558` `(int)usecash`, `[2]` `+0x55c` `(int)ammocount`. `Init`
    /// sets them to -1; `[3]` is unused.
    pub last_srvsett: [i32; 4],
    /// Chat map votes (`+0x684..+0x1864`).
    pub votes: super::votes::Votes,

    // CHalfLifeMultiplay
    pub game_over: bool,
    pub intermission_start: f32,
    pub intermission_end: f32,
    /// `m_iEndIntermissionButtonHit` (`+8`): a player pressed a button during intermission.
    pub end_intermission_button_hit: bool,

    // CHalfLifeTeamplay
    /// `m_DisableDeathMessage` / `m_DisableDeathPenalty` (`+0x20c`/`+0x210`), always set together:
    /// on while `ChangePlayerTeam` kills the player, so `PlayerKilled` does nothing.
    pub death_suppressed: bool,
    /// `m_szTeamList` (`+0xc`): the listed teams, which the Teamplay constructor rebuilds from the
    /// `TSGameRules` team names.
    pub team_list: Vec<Vec<u8>>,
    /// `m_teamLimit` (`+0x214`): only listed teams are valid. The constructor sets it and
    /// `RecountTeams` (0xe5608) clears it for good when the list holds fewer than two teams.
    pub team_limit: bool,
    /// `team_names` (0x19e760): the teams `RecountTeams` found, the first `num_teams` (0x120540)
    /// of 32 slots. The slots past `num_teams` keep stale names, and `SetDefaultPlayerTeam` reads
    /// slot 0 for `mp_defaultteam` whatever the count.
    pub tp_names: Vec<Vec<u8>>,
    pub num_teams: usize,
    /// `team_scores` (0x19e960): frags per `team_names` slot, summed by `RecountTeams` and used by
    /// the fraglimit (`CHalfLifeTeamplay::Think` 0xe412c).
    pub tp_scores: Vec<i32>,
}

/// Slots in `team_names` / `team_scores`.
const MAX_TP_TEAMS: usize = 32;

/// `strcasecmp` on two team names, as TS compares them (`CTSGameRules::GetTeamIndex` 0x79b04,
/// `CHalfLifeTeamplay::GetTeamIndex` 0xe5434, `PlayerRelationship` 0xe5278,
/// `ClientUserInfoChanged` 0xe4d18): byte for byte with only A-Z folded, which is glibc's
/// `strcasecmp` in the C locale TS runs in (and MSVC's `_stricmp`). 0xC9 and 0xE9 (`É`/`é` in
/// CP1252) are different teams.
pub fn team_eq(a: &[u8], b: &[u8]) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// `strtok(list, sep)`: the non-empty pieces between separators.
fn tokens(list: &[u8], sep: u8) -> impl Iterator<Item = &[u8]> {
    list.split(move |&b| b == sep).filter(|s| !s.is_empty())
}

/// A team or model name `strcpy`d into one of `TSGameRules`' 16-byte fields (`+0x28` names,
/// `+0x38..` models): TS copies the whole token and a longer one runs into the next field; the
/// port keeps the first 15 bytes, so the name stays terminated (rules.md, input TS crashes on).
fn field(name: &[u8]) -> Vec<u8> {
    name[..name.len().min(15)].to_vec()
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

/// `armor_absorb` after `CTSGameRules::Init` (0x3e59999a).
pub const INIT_ARMOR_ABSORB: f32 = 0.2125;
/// `health_absorb` after `CTSGameRules::Init` (0x3dcccccd).
pub const INIT_HEALTH_ABSORB: f32 = 0.1;

/// `TSGameRules+0x588c`, the fraction kevlar absorbs (`CBasePlayer::TakeDamage` 0x8fc0c).
pub fn armor_absorb() -> f32 {
    with(|g| g.armor_absorb)
}

/// `g_pGameRules` is `CHalfLifeRules` (0xde6fc): `deathmatch` was 0 when the map's rules were
/// installed. Its hooks are empty or SDK defaults: no rules `Think` (so no `CTSGameRules::Think`),
/// no multiplayer `InitHUD`/`PlayerSpawn`/`PlayerKilled`/`DeathNotice`/`ClientDisconnected`/
/// `ClientUserInfoChanged`, no voice manager or `menuselect`, spawn spots without their target,
/// items and chargers that don't come back, and `respawn` reloads the map.
pub fn single_player() -> bool {
    with(|g| g.single_player)
}

pub fn the_one_mode() -> bool {
    with(|g| g.the_one)
}

pub fn lms() -> bool {
    with(|g| g.lms)
}

/// The max-carry column `Set_ammo_count` (0x7f2cc) last applied: `compile_ammo_info` sets the
/// default one (0) at each map's precache (`W_Precache`, after `CTSGameRules::Init` has set
/// `+0x55c` to -1), and the end of `CTSGameRules::Think` applies `(int)ammocount` (truncated,
/// `frame::ammocount`) once it differs from `+0x55c` (`last_srvsett[2]`). 1, 2 and 3 pick the
/// reduced columns, any other value the default one. So a changed `ammocount` takes effect at
/// the next `Think`, not at once.
pub fn ammo_column() -> usize {
    with(|g| ammo_column_of(g.last_srvsett[2]))
}

/// `TSGameRules + 0x578`: the `(int)realbullet` copy `CTSGameRules::Think` last stored (-1 from
/// `Init` until the first Think; a single-player map keeps the last map's, 0 on a fresh server),
/// what `ShootBulletsPlayer` and `CBulletManager::Think` test.
pub fn realbullet() -> i32 {
    with(|g| g.last_srvsett[0])
}

pub(crate) fn ammo_column_of(applied: i32) -> usize {
    match applied {
        1..=3 => applied as usize,
        _ => 0,
    }
}

/// `CashBased()` (0x76abc): hard-wired off in TS 3.0, so nothing is ever charged. The cash code
/// paths are ported behind it. A ruleset decides ([`ServerRules::cash_based`](crate::gameplay::ServerRules::cash_based)).
pub fn cash_based() -> bool {
    crate::gameplay::active().server.cash_based()
}

/// Default team models (`defTeamModels`).
const DEFAULT_MODELS: [&[u8]; 4] = [b"seal;merc", b"gordon;laurence", b"agent", b"hitman;castor"];

impl GameRules {
    /// `InstallGameRules` + `CTSGameRules::Init` (+ the Teamplay constructor's team setup).
    /// Called when worldspawn spawns (its keyvalues have been parsed by then).
    pub fn install(&mut self) {
        if engine().globals().deathmatch == 0.0 {
            self.install_single_player();
            return;
        }
        crate::gameplay::active().server.install(self);
    }

    /// `InstallGameRules` (0xc6050) with `deathmatch` 0: `g_teamplay = 0` and `CHalfLifeRules`,
    /// whose constructor runs neither `DecideGamePlay` (LMS and The One keep the last map's
    /// values) nor `CTSGameRules::Init`, so the TS state carries over from the last map (`.bss`
    /// zeroes on a fresh server: four teams with no names). The multiplayer and teamplay rules
    /// state, and the players' team names, start empty.
    fn install_single_player(&mut self) {
        self.installed = true;
        self.single_player = true;
        self.teamplay = false;
        self.player_team = vec![Vec::new(); 33];
        self.game_over = false;
        self.intermission_start = 0.0;
        self.intermission_end = 0.0;
        self.end_intermission_button_hit = false;
        self.death_suppressed = false;
        self.team_list.clear();
        self.team_limit = false;
        self.tp_names.clear();
        self.num_teams = 0;
        self.tp_scores.clear();
        if self.teams.len() < 4 {
            self.teams.resize(4, Team::default());
        }
        engine().print("[ts] game rules: single player (deathmatch 0)\n");
    }

    /// `CTSGameRules::Init` (0x77d6c), from the multiplayer rules' constructors and from
    /// `CHalfLifeMultiplay::ChangeLevel` (0xd0784): the mode from `g_teamplay`, `InitTeams`, and
    /// the spawner, round, slow-motion, `SrvSett`, vote, weapon-restriction, super-jump and The One
    /// state. `CBulletManager::Reset` is the caller's (`weapons::bullets::reset`, outside the
    /// rules borrow); `InitObjectives`/`SetObjective` only act in the unreachable modes 2/3; two
    /// fields nothing reads (+0x14, +0x570) aren't modelled.
    pub fn ts_init(&mut self, now: f32) {
        self.mode = if self.teamplay { Mode::TeamDm } else { Mode::Dm };
        self.init_teams();
        self.powerup_spawner = 0xff;
        self.global_slow = GlobalSlow::default();
        self.last_srvsett = [-1; 4];
        self.next_powerup = 0.0;
        self.round_end = 0.0;
        self.slow_blocked = false;
        self.round_restart = now + 5.0;
        self.round_timer = true;
        // +0x684..+0xf84 (the per-player records), the next check and the chosen map; the tallies
        // after them stay.
        for r in &mut self.votes.records {
            r.clear();
        }
        self.votes.next_check = now + 10.0;
        self.votes.chosen.clear();
        self.allowed_weapons = 0x1f; // `InitWeaponRestriction` (0x76ad0)
        self.base_weapons = 0x1f;
        self.the_one_exists = false;
        self.next_the_one_check = now + 10.0;
        self.check_deads_at = 0.0;
        self.the_one_others = 0;
        self.superjump_map = false;
        self.custom_weapons = false;
        self.health_absorb = INIT_HEALTH_ABSORB;
        self.armor_absorb = INIT_ARMOR_ABSORB;
        // `InitObjectives` / `SetObjective`: no managers and an empty list.
        self.objectives = Default::default();
    }

    /// `CTSGameRules::InitTeams`.
    pub fn init_teams(&mut self) {
        self.teams = (0..4)
            .map(|i| Team {
                name: format!("Team{}", i + 1).into_bytes(),
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

    /// `CTSGameRules::SetTeamList` (0x780dc): up to 4 teams (2 in The One, where team 0 is
    /// "The ONE" and the first listed name is skipped, as in TS). `strtok(.., ";")` on the bytes:
    /// empty entries are skipped, every other byte is kept. TS `strcpy`s each name into a 16-byte
    /// field; the port keeps the first 15 bytes (see [`field`]).
    pub fn set_team_list(&mut self, list: &[u8]) {
        for t in &mut self.teams {
            t.name.clear();
        }
        let max = if self.the_one { 2 } else { 4 };
        let mut n = 0;
        for name in tokens(list, b';') {
            if n >= max {
                break;
            }
            if n == 0 && self.the_one {
                self.teams[0].name = b"The ONE".to_vec();
                n = 1;
                continue;
            }
            self.teams[n].name = field(name);
            n += 1;
        }
    }

    /// `CTSGameRules::SetTeamModelList(int, char*)` (0x78438): up to 3 models, `strtok` on `;`.
    fn set_team_models(&mut self, team: usize, list: &[u8]) {
        self.teams[team].models = tokens(list, b';').take(3).map(field).collect();
    }

    /// `SetTeamModelList(char*)` (0x78258): teams separated by `|`, models by `;`.
    pub fn set_team_model_lists(&mut self, list: &[u8]) {
        let parts: Vec<&[u8]> = list.split(|&b| b == b'|').collect();
        for i in 0..4 {
            self.set_team_models(i, parts.get(i).copied().unwrap_or(b""));
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
    pub fn recount_teams(&mut self, players: &[(Vec<u8>, f32)]) {
        self.tp_names.resize(MAX_TP_TEAMS, Vec::new());
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
                // `(int)(score + frags)`, truncated (0xe57e9).
                *s = super::x87_trunc(*s as f64 + *frags as f64);
            }
        }
    }

    /// `team_names[0..num_teams]`: what `TeamNames` sends and `jointeam` indexes.
    pub fn team_names(&self) -> Vec<Vec<u8>> {
        self.tp_names.iter().take(self.num_teams).cloned().collect()
    }

    /// `team_scores[0..num_teams]`.
    pub fn team_scores(&self) -> &[i32] {
        &self.tp_scores[..self.num_teams.min(self.tp_scores.len())]
    }

    /// `CHalfLifeTeamplay::GetTeamIndex` (0xe5434): the `team_names` slot (`strcasecmp`, which
    /// folds only A-Z: see [`team_eq`]), -1 for none or `""`. `ScoreInfo` sends it + 1.
    pub fn tp_team_index(&self, name: &[u8]) -> i32 {
        if name.is_empty() {
            return -1;
        }
        self.tp_names.iter().take(self.num_teams).position(|t| team_eq(t, name)).map_or(-1, |i| i as i32)
    }

    /// `CHalfLifeTeamplay::IsValidTeam` (0xe54d4): any name without a team limit (`""` too).
    pub fn is_valid_team(&self, name: &[u8]) -> bool {
        !self.team_limit || self.tp_team_index(name) != -1
    }

    /// `CHalfLifeTeamplay::TeamWithFewestPlayers` (0xe5510), given every player's team name: the
    /// first team with the fewest players, `""` when there is none (TS would copy from NULL).
    pub fn team_with_fewest(&self, players: &[Vec<u8>]) -> Vec<u8> {
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
    pub fn first_team_slot(&self) -> Vec<u8> {
        self.tp_names.first().cloned().unwrap_or_default()
    }

    /// `TSGameRules` teams 0 and 1 (`0x1781c8`/`0x178254`), The One's and everyone else's in The
    /// One mode (`SetDefaultPlayerTeam`, `ChangePlayerTeam`).
    pub fn the_one_team_names(&self) -> [Vec<u8>; 2] {
        [0, 1].map(|i| self.teams.get(i).map(|t| t.name.clone()).unwrap_or_default())
    }

    /// `CTSGameRules::GetTeamIndex` (0x79b04): the `TSGameRules` team (`strcasecmp`, A-Z only),
    /// -1 if none. Models, spawn points, CTB and triggers use it; `ScoreInfo` uses `tp_team_index`.
    pub fn team_index(&self, name: &[u8]) -> i32 {
        if name.is_empty() {
            return -1;
        }
        self.teams.iter().position(|t| !t.name.is_empty() && team_eq(&t.name, name)).map_or(-1, |i| i as i32)
    }

    /// `CHalfLifeTeamplay::GetIndexedTeamName` (0xe54a0): the team at `index`, or `""` when it is
    /// out of range (`ChangePlayerTeam` then picks the smallest team).
    pub fn indexed_team_name(&self, index: i32) -> Vec<u8> {
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

    /// Byte names as text, for readable asserts.
    fn strs<T: AsRef<[u8]>>(v: &[T]) -> Vec<String> {
        v.iter().map(|n| String::from_utf8_lossy(n.as_ref()).into_owned()).collect()
    }

    #[test]
    fn ts_init_resets_only_the_ts_state() {
        // A teamplay game near its end, as `ChangeLevel` finds it.
        let mut g = GameRules { teamplay: true, game_over: true, intermission_end: 50.0, ..Default::default() };
        g.ts_init(0.0);
        g.set_team_list(b"Red;Blue");
        g.init_teamplay_list();
        g.powerup_spawner = 1;
        g.round_restart = 0.0;
        g.round_timer = false;
        g.slow_blocked = true;
        g.global_slow = GlobalSlow { active: true, amount: 0.5, until: 30.0, next: 31.0 };
        g.last_srvsett = [1, 1, 1, 0x1f];
        g.votes.records[3] = "ts_awaken".into();
        g.votes.tallies = vec![("ts_awaken".into(), 1)];
        g.votes.chosen = "ts_awaken".into();
        g.allowed_weapons = 3;
        g.base_weapons = 0x3f;
        g.superjump_map = true;
        g.the_one_exists = true;
        g.check_deads_at = 12.0;
        g.the_one_others = 4;
        // A weapons file loaded on the last map.
        g.custom_weapons = true;
        g.armor_absorb = 0.9;
        g.health_absorb = 0.5;

        g.ts_init(100.0);
        assert!(!g.custom_weapons, "the custom-weapons notice is per map");
        assert_eq!((g.armor_absorb.to_bits(), g.health_absorb.to_bits()), (0x3e59999a, 0x3dcccccd));
        assert_eq!(g.mode, Mode::TeamDm, "the mode follows g_teamplay");
        assert_eq!(strs(&g.teams.iter().map(|t| t.name.clone()).collect::<Vec<_>>()), ["Team1", "Team2", "Team3", "Team4"]);
        assert_eq!(strs(&g.teams[0].models), ["seal", "merc"]);
        assert_eq!((g.powerup_spawner, g.next_powerup, g.round_end), (0xff, 0.0, 0.0));
        assert_eq!((g.round_restart, g.round_timer, g.slow_blocked), (105.0, true, false));
        assert!(!g.global_slow.active && g.global_slow.amount == 0.0);
        assert_eq!(g.last_srvsett, [-1; 4]);
        // Until the first Think, the precache's `Set_ammo_count(0)` stands.
        assert_eq!(ammo_column_of(g.last_srvsett[2]), 0);
        assert!(g.votes.records.iter().all(String::is_empty));
        assert_eq!(g.votes.tallies.len(), 1, "the tallies after the records stay");
        assert_eq!((g.votes.next_check, g.votes.chosen.as_str()), (110.0, ""));
        assert_eq!((g.allowed_weapons, g.base_weapons, g.superjump_map), (0x1f, 0x1f, false));
        assert_eq!((g.the_one_exists, g.next_the_one_check, g.check_deads_at, g.the_one_others), (false, 110.0, 0.0, 0));
        // The multiplayer and teamplay rules objects aren't touched.
        assert!(g.teamplay && g.game_over && g.intermission_end == 50.0);
        assert_eq!(strs(&g.team_list), ["Red", "Blue"]);
        assert!(g.team_limit);
        assert_eq!(strs(&g.team_names()), ["Red", "Blue"]);
    }

    #[test]
    fn team_lists() {
        let mut g = GameRules::default();
        g.init_teams();
        g.set_team_list(b"hitman;gordon");
        g.init_teamplay_list();
        assert_eq!(strs(&g.team_names()), ["hitman", "gordon"]);
        assert_eq!(g.team_index(b"GORDON"), 1);
        assert_eq!(g.team_index(b"seal"), -1);
        g.the_one = true;
        g.set_team_list(b"hitman;gordon;agent");
        g.init_teamplay_list();
        assert_eq!(strs(&g.team_names()), ["The ONE", "gordon"]);
        g.set_team_model_lists(b"a;b|c");
        assert_eq!(strs(&g.teams[0].models), ["a", "b"]);
        assert_eq!(strs(&g.teams[1].models), ["c"]);
        assert!(g.teams[2].models.is_empty());
    }

    /// #188: `mp_teamlist` is cut by `strtok` on the bytes and compared with `strcasecmp`, which
    /// folds only A-Z: a CP1252 0xE9 name keeps its byte and matches only itself (any ASCII case).
    #[test]
    fn team_lists_are_bytes() {
        let mut g = GameRules::default();
        g.init_teams();
        g.set_team_list(b"\xe9quipe;;Rouge;");
        g.set_team_model_lists(b"mod\xe8le;seal|merc");
        g.init_teamplay_list();
        assert_eq!(g.team_names(), [b"\xe9quipe".to_vec(), b"Rouge".to_vec()]);
        assert_eq!(g.teams[0].models, [b"mod\xe8le".to_vec(), b"seal".to_vec()]);
        assert_eq!(g.team_index(b"\xe9QUIPE"), 0);
        assert_eq!(g.tp_team_index(b"\xe9Quipe"), 0);
        // No folding outside A-Z (0xC9 is `É`), and no UTF-8 equivalence.
        assert_eq!(g.team_index(b"\xc9QUIPE"), -1);
        assert_eq!(g.tp_team_index("équipe".as_bytes()), -1);
        assert!(!g.is_valid_team(b"\xc9quipe"));
        assert!(team_eq(b"ROUGE", b"rouge") && !team_eq(b"\xe9", b"\xc9"));
        // A name too long for TS's 16-byte field: the first 15 bytes, even mid-sequence.
        g.set_team_list("abcdefghijklmné;Bleu".as_bytes());
        assert_eq!(g.teams[0].name, b"abcdefghijklmn\xc3");
        assert_eq!(g.teams[1].name, b"Bleu");
    }

    /// The rules after the Teamplay constructor for `list`.
    fn teamplay(list: &str) -> GameRules {
        let mut g = GameRules::default();
        g.init_teams();
        g.set_team_list(list.as_bytes());
        g.init_teamplay_list();
        g
    }

    fn players(list: &[(&str, f32)]) -> Vec<(Vec<u8>, f32)> {
        list.iter().map(|&(n, f)| (n.as_bytes().to_vec(), f)).collect()
    }

    #[test]
    fn recount_with_a_team_limit_keeps_the_listed_teams() {
        let mut g = teamplay("hitman;gordon;HITMAN");
        assert!(g.team_limit);
        assert_eq!(strs(&g.team_names()), ["hitman", "gordon"], "duplicates are dropped");
        g.recount_teams(&players(&[("gordon", 3.0), ("seal", 5.0), ("GORDON", 2.0), ("", 1.0)]));
        assert_eq!(strs(&g.team_names()), ["hitman", "gordon"]);
        assert_eq!(g.team_scores(), [0, 5]);
        assert_eq!(g.tp_team_index(b"Gordon"), 1);
        assert!(g.is_valid_team(b"HITMAN"));
        assert!(!g.is_valid_team(b"seal"));
        assert!(!g.is_valid_team(b""));
    }

    #[test]
    fn recount_without_a_team_limit_takes_the_players_teams() {
        let mut g = teamplay("hitman");
        assert!(!g.team_limit, "RecountTeams drops the limit below two teams");
        assert!(g.team_names().is_empty());
        // team_names[0] still holds the listed team until a player's team overwrites it.
        assert_eq!(g.first_team_slot(), b"hitman");
        assert!(g.is_valid_team(b"anything") && g.is_valid_team(b""));
        g.recount_teams(&players(&[("seal", 2.0), ("", 1.0), ("SEAL", 3.0), ("", 4.0), ("merc", 1.0)]));
        // Each teamless player adds an empty team: GetTeamIndex("") is -1.
        assert_eq!(strs(&g.team_names()), ["seal", "", "", "merc"]);
        assert_eq!(g.team_scores(), [5, 1, 4, 1]);
        assert_eq!(g.first_team_slot(), b"seal");
        assert_eq!(g.tp_team_index(b"seal"), 0);
        assert_eq!(g.tp_team_index(b""), -1);
        assert_eq!(g.indexed_team_name(3), b"merc");
        // The limit never comes back, and the TSGameRules list is untouched.
        g.recount_teams(&[]);
        assert!(!g.team_limit && g.team_names().is_empty());
        assert_eq!(g.team_index(b"hitman"), 0);
        assert_eq!(g.team_index(b"seal"), -1);
    }

    #[test]
    fn team_with_fewest_players() {
        let g = teamplay("hitman;gordon;agent");
        let names = |l: &[&str]| l.iter().map(|s| s.as_bytes().to_vec()).collect::<Vec<_>>();
        assert_eq!(g.team_with_fewest(&names(&[])), b"hitman");
        assert_eq!(g.team_with_fewest(&names(&["hitman", "GORDON", "seal", ""])), b"agent");
        assert_eq!(g.team_with_fewest(&names(&["hitman", "agent"])), b"gordon");
        // No teams at all (TS would copy from a NULL pointer).
        let mut g = teamplay("hitman");
        assert_eq!(g.team_with_fewest(&names(&["seal"])), b"");
        g.recount_teams(&players(&[("seal", 0.0), ("merc", 0.0), ("seal", 0.0)]));
        assert_eq!(g.team_with_fewest(&names(&["seal", "merc", "seal"])), b"merc");
    }

    #[test]
    fn the_one_team_names_come_from_the_ts_list() {
        let mut g = GameRules::default();
        g.init_teams();
        g.the_one = true;
        g.set_team_list(b"hitman;gordon");
        g.init_teamplay_list();
        assert_eq!(strs(&g.the_one_team_names()), ["The ONE", "gordon"]);
        assert_eq!(strs(&g.team_names()), ["The ONE", "gordon"]);
    }

    #[test]
    fn indexed_team_name_is_empty_out_of_range() {
        let g = teamplay("hitman;gordon");
        assert_eq!(g.indexed_team_name(0), b"hitman");
        assert_eq!(g.indexed_team_name(1), b"gordon");
        assert_eq!(g.indexed_team_name(2), b"");
        assert_eq!(g.indexed_team_name(-1), b"");
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

    #[test]
    fn ammo_column_follows_set_ammo_count() {
        // `Set_ammo_count`'s switch: 1..3 pick a column, anything else (the constructor's -1,
        // negative or large values) the default one.
        for (applied, col) in [(-1, 0), (0, 0), (1, 1), (2, 2), (3, 3), (4, 0), (7, 0), (-3, 0)] {
            assert_eq!(ammo_column_of(applied), col, "{applied}");
        }
    }

    #[test]
    fn ammocount_cvar_is_truncated_then_mapped() {
        use crate::entities::stock::maprules::x87_int;
        // `(int)ammocount`: truncated toward zero, NaN and out of range the integer indefinite.
        for (cvar, col) in [
            (0.0, 0),
            (0.9, 0),
            (1.0, 1),
            (1.5, 1),
            (2.9, 2),
            (3.0, 3),
            (3.99, 3),
            (4.0, 0),
            (5.0, 0),
            (-0.9, 0),
            (-2.0, 0),
            (f32::NAN, 0),
            (3.0e9, 0),
        ] {
            assert_eq!(ammo_column_of(x87_int(cvar)), col, "{cvar}");
        }
    }
}
