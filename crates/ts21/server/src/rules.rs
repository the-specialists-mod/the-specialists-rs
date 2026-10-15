//! TS 2.1's game rules: the game mode from the map name, teams, `CTSGameRules::Think` and the
//! round modes 2/3 (decomp `docs/ts21/rules.md` §2, §3, §9).

use crate::cvars::{self as cv, Polled};
use goldsrc::Global;
use goldsrc::edict::EdictPtr;
use goldsrc::server::engine;
use shared::protocol::rules::{RoundTime, SrvSett, TeamScore, TsHVoice};
use std::cell::RefCell;
use ts_server::entities::ts::ctb::GiverWin;
use ts_server::entities::ts::objective::{self, ManagerTarget, ObjectiveRules};
use ts_server::messages::Msg;
use ts_server::rules::frame::{round_time_byte, x87_max0};
use ts_server::rules::lifecycle::start_observer;
use ts_server::rules::msg::{self, ts_message_all};
use ts_server::rules::state::{self, GameRules, Mode, Team};
use ts_server::rules::{alive, cvars, frame, has_player, netname, other, scoring, slot_players, teams, votes};

/// The two round-mode teams (`CHalfLifeTeamplay` 0x10038550, strings 0x10097488/0x1009747c).
pub const SPECIALISTS: &str = "Specialists";
pub const MERCENARY: &str = "Mercenary";

/// `CTSGameRules` state 2.1 has and the port's `GameRules` doesn't (`TSGameRules +0x1a0..`,
/// `+0x2cc`, `+0x54bc..`). `Init` (0x10044810) sets it up, at every map start and switch.
#[derive(Clone, Debug, PartialEq)]
pub struct Rules21 {
    /// `+0x54bc`: `ts_mapglobals` has been read (Think, once).
    pub mapglobals_read: bool,
    /// `+0x54c0`: the map's weapon mask (`spawnflags ^ 0x1f`, 0x1f without `ts_mapglobals`).
    pub map_mask: i32,
    /// `+0x54c8`: the weapon mask last sent as `SrvSett` 3.
    pub mask_sent: i32,
    /// `+0x54cc`: `ts_mapglobals` `playerrespawn`, the respawns per round in mode 3.
    pub respawns_per_round: u8,
    /// `+0x1b8`: the next periodic `RespawnEntities` (DM/TDM without LMS).
    pub next_entity_respawn: f32,
    /// `+0x1bc`: the `globalslow` value the last Think saw (-1 from `Init`).
    pub last_globalslow: f32,
    /// `+0x1c0`, `+0x1a0`, `+0x1a4`: the `(int)` copies of `realbullet`, `usecash` and
    /// `ammocount` last sent as `SrvSett` 0, 1, 2 (-1 from `Init`).
    pub srvsett: [i32; 3],
    /// `+0x2cc`: rounds since the last side swap (-1 from `Init`).
    pub rounds: i32,
    /// `+0x2d0`: the sides are swapped: the Mercenaries spawn at `info_player_start`.
    pub swapped: bool,
}

impl Default for Rules21 {
    fn default() -> Self {
        Rules21 {
            mapglobals_read: false,
            map_mask: 0x1f,
            mask_sent: 0,
            respawns_per_round: 0,
            next_entity_respawn: 0.0,
            last_globalslow: -1.0,
            srvsett: [-1; 3],
            rounds: -1,
            swapped: false,
        }
    }
}

static RULES: Global<RefCell<Option<Rules21>>> = Global(RefCell::new(None));

/// Runs `f` on 2.1's rules state.
pub fn with<R>(f: impl FnOnce(&mut Rules21) -> R) -> R {
    let mut r = RULES.0.borrow_mut();
    f(r.get_or_insert_with(Rules21::default))
}

/// `DecideGamePlay` (0x100433b0): the mode from the first three characters of the map name
/// (case-sensitive). `ts_` maps are teamplay in The One mode and otherwise follow `mp_teamplay`;
/// `tm_` maps are objective rounds (3); `tr_`, `cs_` and `de_` maps round team deathmatch (2);
/// anything else follows `mp_teamplay` (`!= 0` is teamplay, 1). There is no worldspawn
/// `gamemode` key, and LMS isn't decided here (Think reads `lastmanstanding` live).
pub fn decide_gameplay(map: &str, the_one: bool, mp_teamplay: f32) -> Mode {
    let prefix = map.as_bytes().get(..3).unwrap_or(map.as_bytes());
    match prefix {
        b"ts_" if the_one => return Mode::TeamDm,
        b"tm_" => return Mode::Objective,
        b"tr_" | b"cs_" | b"de_" => return Mode::RoundTdm,
        _ => {}
    }
    // `if (mp_teamplay != 0) return 1; return (int)mp_teamplay;` (0 here).
    if mp_teamplay != 0.0 { Mode::TeamDm } else { Mode::Dm }
}

/// The teams of `mode` (`CHalfLifeTeamplay` 0x10038550 and `CTSGameRules::Init` 0x10044810):
///
/// * Modes 2/3: "Specialists" (model `seal`) and "Mercenary" (`merc`), each with the team cash
///   `(int)initial_cash`; the team list is exactly those two.
/// * Mode 1: the SDK's model teams. The team list is `mp_teamlist`, and a player's team is its
///   `model` key; each listed team's model is its own name. In The One mode the first two teams
///   are `mp_theone`'s and `mp_theothers`' models.
/// * Mode 0: TS 3.0's default table is left alone (no teamplay reads it).
///
/// `spawn_round` gives the round modes' spawn classes: Specialists `info_player_start`,
/// Mercenaries `info_player_deathmatch` (swapped by [`Rules21::swapped`]).
pub fn setup_teams(g: &mut GameRules, mode: Mode, the_one: bool, teamlist: &[u8], one: [&[u8]; 2], cash: i32) {
    let model_team = |name: &[u8]| Team {
        name: name.to_vec(),
        models: vec![name.to_vec()],
        spawn_class: "info_player_deathmatch".into(),
        fallback: "info_player_deathmatch".into(),
        ..Default::default()
    };
    match mode {
        Mode::RoundTdm | Mode::Objective => {
            g.teams = vec![
                Team { name: SPECIALISTS.into(), models: vec![b"seal".to_vec()], cash, ..Default::default() },
                Team { name: MERCENARY.into(), models: vec![b"merc".to_vec()], cash, ..Default::default() },
            ];
            set_spawn_classes(g, false);
            g.team_list = vec![SPECIALISTS.into(), MERCENARY.into()];
            g.team_limit = true;
            g.recount_teams(&[]);
        }
        Mode::TeamDm => {
            // The SDK keeps the list's names as `strtok` cuts them, at most 16 bytes each (bytes,
            // whatever their encoding: #188).
            let listed: Vec<Vec<u8>> =
                teamlist.split(|&b| b == b';').filter(|s| !s.is_empty()).map(|s| s[..s.len().min(16)].to_vec()).collect();
            g.teams = if the_one { one.iter().map(|n| model_team(n)).collect() } else { Vec::new() };
            g.teams.extend(listed.iter().map(|n| model_team(n)));
            g.team_list = listed;
            g.team_limit = !teamlist.is_empty();
            g.recount_teams(&[]);
        }
        Mode::Dm => {}
    }
}

/// The round modes' spawn classes (`FUN_10042f50`): the Specialists use `info_player_start` and
/// the Mercenaries `info_player_deathmatch`, the other way round once the sides are swapped.
pub fn set_spawn_classes(g: &mut GameRules, swapped: bool) {
    let (spec, merc) =
        if swapped { ("info_player_deathmatch", "info_player_start") } else { ("info_player_start", "info_player_deathmatch") };
    for t in &mut g.teams {
        let class = if t.name == SPECIALISTS.as_bytes() {
            spec
        } else if t.name == MERCENARY.as_bytes() {
            merc
        } else {
            continue;
        };
        t.spawn_class = class.into();
        t.fallback = class.into();
    }
}

/// `InstallGameRules` (0x1001eba0) for a multiplayer map: `DecideGamePlay`, the rules objects
/// and `CTSGameRules::Init` (0x10044810).
pub fn install(g: &mut GameRules) {
    let eng = engine();
    let now = eng.time();
    let map = eng.string(eng.globals().mapname).to_string_lossy().into_owned();
    // `g_theOneMode`: `mp_theonemode` as `CWorld::Spawn` (0x1006c530) reads it for the map.
    let the_one = cvars::on(c"mp_theonemode");
    let mode = decide_gameplay(&map, the_one, cvars::f(c"mp_teamplay"));
    // The worldspawn keys TS 3.0 reads, parsed before the install, stay as they were.
    let (forced, map_teams, map_models) = (g.forced_gamemode, std::mem::take(&mut g.map_teams), std::mem::take(&mut g.map_team_models));
    *g = GameRules::default();
    (g.forced_gamemode, g.map_teams, g.map_team_models) = (forced, map_teams, map_models);
    g.installed = true;
    g.teamplay = mode != Mode::Dm;
    g.the_one = the_one;
    g.player_team = vec![Vec::new(); 33];
    // 3.0's `Init` resets the same spawner, round, slow-motion, SrvSett, vote and weapon-mask
    // state; 2.1's own part follows.
    g.ts_init(now);
    g.mode = mode;
    let one = [cvars::bytes(c"mp_theone"), cvars::bytes(c"mp_theothers")];
    setup_teams(g, mode, the_one, &cvars::bytes(c"mp_teamlist"), [&one[0], &one[1]], cv::initial_cash_int());
    with(|r| {
        *r = Rules21::default();
        r.next_entity_respawn = now + 60.0;
    });
    init_objectives(g);
    eng.print(&shared::bformat!(
        "[ts] game rules (2.1): {} ({})\n",
        match mode {
            Mode::Objective => "Objective rounds",
            Mode::RoundTdm => "Round Team DeathMatch",
            Mode::TeamDm if the_one => "The One",
            Mode::TeamDm => "Team DeathMatch",
            Mode::Dm => "DeathMatch",
        },
        g.team_list.join(&b';')
    ));
}

/// `InitObjectives` (0x100449b0) in the round modes: the map's `ts_objective_manager`s (up to
/// 32, the round's manager forgotten); mode 3 when there is one, else mode 2. In mode 3 the caller
/// then disables the objectives ([`disable_objectives`]), outside the rules borrow.
pub fn init_objectives(g: &mut GameRules) {
    if !matches!(g.mode, Mode::RoundTdm | Mode::Objective) {
        return;
    }
    g.objectives.managers = objective::find_managers();
    g.objectives.manager = std::ptr::null_mut();
    g.mode = if g.objectives.managers.is_empty() { Mode::RoundTdm } else { Mode::Objective };
}

/// The end of `InitObjectives` in mode 3: `EnableObjective(false)` on every `ts_bomb`, `ts_hack`
/// and `ts_teamescape`, and every `ts_wingiver` counts from 0 again (FUN_10050c90). 2.1 has no
/// `ts_scoregiver`, so these are all its objectives.
pub fn disable_objectives() {
    if state::with(|g| g.mode) == Mode::Objective {
        objective::disable_all();
    }
}

/// 2.1's objective system (the objective entities in `ts_server::entities::ts::objective`).
pub const OBJECTIVES: ObjectiveRules = ObjectiveRules {
    // `TSHudMessageAll(0, 255, 0, 1, 30, ...)` (FUN_10050710, `CTSBomb::Use` 0x10051430, ...).
    hud_effect: 1,
    // `CanInteract` (FUN_100513d0) and the pointer's text (`CTSObjectivePointer::Use` 0x100508f0)
    // compare the team's name (`stricmp`); the escape zone counts `TSCountPlayers` by name
    // (FUN_10064230).
    team_names: Some([SPECIALISTS, MERCENARY]),
    // FUN_10046630 appends without a search and keeps no texts; `Freeze` (FUN_10050f10) sends
    // `DelObj` (FUN_10046690) only when freezing; no `InitHUD` or team-change resend.
    indexed_list: false,
    // `CTSObjectivePointer::Use` enables `ts_hack`, `ts_bomb` and `ts_teamescape` by classname.
    pointer_enables_givers: false,
    // No `CheckForWires`: the HUD's wires change only with `cutwire_`.
    wire_hud: false,
    // `test ah,0x41; jp` (`CTSBomb::WaitThink` 0x1005130a, `CTSHack::WaitThink` 0x10051860).
    nan_time_runs_out: false,
    // `SetObjective` (0x10045150) does nothing outside the round modes.
    clear_list_outside_rounds: false,
};

/// The `globalslow` part of Think: the cvar (a fraction) clamped to `[0, 2]` with cheats on at
/// map load, else `[0, 1]`, and 1 means off. Returns the value Think works with.
pub fn globalslow_value(g: f32, cheats: bool) -> f32 {
    let max = if cheats { 2.0 } else { 1.0 };
    let clamped = if max <= g { max } else { g };
    let v = if clamped <= 0.0 {
        0.0
    } else if max <= g {
        max
    } else {
        g
    };
    if v == 1.0 { 0.0 } else { v }
}

/// What the `globalslow` poll does this frame.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SlowStep {
    /// `SetGlobalSlow(v, 0)`: a new amount, until the cvar changes.
    Set(f32),
    /// The cvar went to 0 (or 1): global slow stops.
    Stop,
    /// Nothing changes.
    Keep,
}

/// The decision of Think's `globalslow` block (0x10043d0e..0x10043e0e) from the clamped value,
/// the current amount and the value seen last frame.
pub fn globalslow_step(v: f32, amount: f32, last: f32) -> SlowStep {
    if v != 0.0 {
        if v != amount { SlowStep::Set(v) } else { SlowStep::Keep }
    } else if last != v {
        SlowStep::Stop
    } else {
        SlowStep::Keep
    }
}

/// `CTSGameRules::Think` (0x10043cc0).
pub fn think(now: f32) {
    read_mapglobals();
    global_slow(now);
    if state::with(|g| g.check_deads_at != 0.0 && g.check_deads_at < now) {
        check_deads(now);
    }
    let lms_cvar = cvars::on(c"lastmanstanding");
    let (mode, the_one, the_one_exists) = state::with(|g| {
        // Read live: LMS plays in DM whenever the cvar is on.
        g.lms = lms_cvar && g.mode == Mode::Dm;
        (g.mode, g.the_one, g.the_one_exists)
    });
    if matches!(mode, Mode::Dm | Mode::TeamDm) {
        // The spawner itself (the map's `ts_powerup` test, the type) is the power-up rules'.
        crate::powerup::auto_powerups(now);
        if the_one && !the_one_exists {
            select_random_the_one(now);
        }
        if !lms_cvar || mode != Mode::Dm {
            if with(|r| r.next_entity_respawn < now) {
                crate::entities::respawn_entities(false);
                with(|r| r.next_entity_respawn = now + 60.0);
            }
        } else {
            let restart = state::with(|g| g.round_restart);
            if restart < now && restart != 0.0 {
                restart_round(now);
            }
            let (restart, end) = state::with(|g| (g.round_restart, g.round_end));
            if end < now && restart == 0.0 {
                check_deads(now);
            }
        }
    } else {
        let (restart, end) = state::with(|g| (g.round_restart, g.round_end));
        if end < now && restart == 0.0 {
            if mode == Mode::RoundTdm {
                let lose = cv::cash_for_lose();
                team_win(2, lose, lose, now);
            } else {
                round_time_left(now);
            }
        }
        let restart = state::with(|g| g.round_restart);
        if restart < now && restart != 0.0 {
            restart_round(now);
        }
    }
    votes::check_votes(now);
    srvsett_think();
    weapon_restriction_think();
    let r = cv::polled(Polled::RestartRound);
    if r != 0.0 {
        reset_restart_round(cv::trunc(r), now);
        cv::set_polled(Polled::RestartRound, "0");
    }
    // `customweapons` (a 2.1 weapons file, three values per weapon) is the weapon tables' part.
    if the_one && state::with(|g| g.next_the_one_check < now) {
        let eng = engine();
        let any = the_one_check_slots(eng.max_clients())
            .map(|i| eng.player(i))
            .any(|e| has_player(e) && other(e, |p| p.ts.is_the_one).unwrap_or(false));
        if !any {
            select_random_the_one(now);
        }
        state::with(|g| g.next_the_one_check = now + 10.0);
    }
}

/// The slots Think's The One check looks at (0x1004439c..0x100443d5): `UTIL_PlayerByIndex(i)` for
/// `i` in `0..maxClients`. Slot 0 is never a player (0x10066db0 returns NULL), and the last slot
/// is never checked, so while The One sits in it a random One is picked again every 10 s.
fn the_one_check_slots(max_clients: i32) -> std::ops::Range<i32> {
    0..max_clients
}

/// The `ts_mapglobals` read at the head of Think, once per `Init`: the weapon mask and, from the
/// first one, the respawns per round in mode 3 (`playerrespawn`, `CTSGameRules+0x54cc`; its
/// `playeratatime` goes to `+0x54cd`, which nothing reads).
fn read_mapglobals() {
    if with(|r| std::mem::replace(&mut r.mapglobals_read, true)) {
        return;
    }
    let found = !engine().find_by_string(std::ptr::null_mut(), c"classname", c"ts_mapglobals").is_null();
    // The port's `ts_mapglobals` has already folded its spawnflags into `base_weapons`.
    let mask = if found { state::with(|g| g.base_weapons) } else { 0x1f };
    let (per_round, _at_a_time) = crate::entities::map_globals_keys();
    with(|r| {
        r.map_mask = mask;
        r.respawns_per_round = per_round as u8;
    });
}

/// The global slow-motion block of Think.
fn global_slow(now: f32) {
    let v = globalslow_value(cv::polled(Polled::GlobalSlow), cvars::cheats());
    let (amount, last) = (state::with(|g| g.global_slow.amount), with(|r| r.last_globalslow));
    match globalslow_step(v, amount, last) {
        SlowStep::Set(v) => frame::set_global_slow(v, 0.0),
        SlowStep::Stop => state::with(|g| g.global_slow.active = false),
        SlowStep::Keep => {}
    }
    state::with(|g| {
        if g.global_slow.until != 0.0 && g.global_slow.until < now {
            g.global_slow.active = false;
        }
    });
    with(|r| r.last_globalslow = v);
    let (active, amount) = state::with(|g| (g.global_slow.active, g.global_slow.amount));
    if !active {
        if amount != 0.0 {
            frame::global_slow2(0.0, false);
            state::with(|g| {
                g.global_slow.amount = 0.0;
                g.global_slow.until = 0.0;
            });
        }
    } else {
        frame::global_slow2(amount, false);
    }
}

/// The `SrvSett` 0..2 part of Think. Key 0 carries `realbullet != 0` (3.0 sends the value).
fn srvsett_think() {
    let rb = cvars::f(c"realbullet");
    if let Some(v) = with(|r| refresh(&mut r.srvsett[0], rb)) {
        // The copy the bullet manager tests (`state::realbullet`).
        state::with(|g| g.last_srvsett[0] = v);
        msg::all(Msg::SrvSett, |w| SrvSett { key: 0, value: (v != 0) as i16 }.write(w));
    }
    ts_server::weapons::bullet_think();
    let cash = cvars::f(c"usecash");
    if let Some(v) = with(|r| refresh(&mut r.srvsett[1], cash)) {
        state::with(|g| g.last_srvsett[1] = v);
        msg::all(Msg::SrvSett, |w| SrvSett { key: 1, value: (v != 0) as i16 }.write(w));
    }
    let ammo = cvars::f(c"ammocount");
    if let Some(v) = with(|r| refresh(&mut r.srvsett[2], ammo)) {
        // `Set_ammo_count`: the port keeps the applied column in the rules state.
        state::with(|g| g.last_srvsett[2] = v);
        msg::all(Msg::SrvSett, |w| SrvSett { key: 2, value: v as i16 }.write(w));
    }
}

/// `if ((float)stored != cvar) stored = (int)cvar`, and the new copy to send.
fn refresh(stored: &mut i32, cvar: f32) -> Option<i32> {
    if *stored as f32 == cvar {
        return None;
    }
    *stored = cv::trunc(cvar);
    Some(*stored)
}

/// `weaponrestriction` values (`FUN_10043630`): 1 nothing, 2 knives, 3 knives and pistols,
/// 4 knives, pistols and shotguns, anything else every category. 5 ignores the map's mask.
pub fn restriction_mask(w: f32, map_mask: i32) -> i32 {
    if w == 5.0 {
        return 0x1f;
    }
    let by_id = match cv::trunc(w) {
        1 => 0,
        2 => 0x01,
        3 => 0x03,
        4 => 0x13,
        _ => 0x1f,
    };
    by_id & map_mask
}

/// The weapon mask, from the `weaponrestriction` cvar every frame; `SrvSett` 3 when it changes.
fn weapon_restriction_think() {
    let map_mask = with(|r| r.map_mask);
    let mask = restriction_mask(cv::polled(Polled::WeaponRestriction), map_mask);
    state::with(|g| g.allowed_weapons = mask);
    let changed = with(|r| std::mem::replace(&mut r.mask_sent, mask) != mask);
    if changed {
        msg::all(Msg::SrvSett, |w| SrvSett { key: 3, value: mask as i16 }.write(w));
    }
}

/// `CTSGameRules::SelectRandomTheOne` (0x100466e0): every 10 s at most, a random living,
/// non-observing player (33 at most) becomes The One and takes its team. Unlike 3.0 it doesn't
/// look for a One first.
pub fn select_random_the_one(now: f32) {
    if state::with(|g| g.next_the_one_check > now) {
        return;
    }
    state::with(|g| g.next_the_one_check = now + 10.0);
    let mut candidates = Vec::new();
    for e in slot_players() {
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
    // `g_pGameRules->SetDefaultPlayerTeam(p)` (teamplay vtable +0xdc).
    if state::teamplay() {
        ts_server::player::with_player(pick, teams::set_default_team);
    }
}

/// The team (0 Specialists, 1 Mercenary) a player is on in the round modes.
pub fn round_team(name: &[u8]) -> Option<usize> {
    if name.eq_ignore_ascii_case(SPECIALISTS.as_bytes()) {
        Some(0)
    } else if name.eq_ignore_ascii_case(MERCENARY.as_bytes()) {
        Some(1)
    } else {
        None
    }
}

/// `GetTeamCash` (0x100458b0): the player's team cash in the round modes, 0 otherwise.
pub fn team_cash(team: &[u8]) -> i32 {
    state::with(|g| {
        if !matches!(g.mode, Mode::RoundTdm | Mode::Objective) {
            return 0;
        }
        round_team(team).and_then(|i| g.teams.get(i)).map_or(0, |t| t.cash)
    })
}

/// The team cash after a round (`TeamWin` 0x10044d00), from the cash before, the payout and
/// `initial_cash`: the winner `min(max(c + win, initial), 30000)`, anyone else `max(c + lose,
/// initial)`, in floating point and truncated.
pub fn round_cash(cash: i32, delta: i32, initial: f32, winner: bool) -> i32 {
    let c = cash as f32 + delta as f32;
    let c = if c < initial { initial } else { c };
    let c = if winner && c > 30000.0 { 30000.0 } else { c };
    cv::trunc(c)
}

/// `CTSGameRules::TeamWin(team, win, lose)` (0x10044d00): 0 the Specialists win, 1 the
/// Mercenaries, 2 a draw. The round is over (`RoundTime` frozen, restart in 3 s). With fewer than
/// two players the team cash and scores start again. Otherwise the team cash is paid out
/// ([`round_cash`]), the winner scores, and every player hears whether its team lost
/// (`TSHVoice`). Then the result, and the round modes' `TeamScore`s.
pub fn team_win(team: i32, win: i32, lose: i32, now: f32) {
    let left = state::with(|g| g.round_end - now);
    msg::all(Msg::RoundTime, |w| RoundTime { secs: round_time_byte(left as f64), state: 2 }.write(w));
    state::with(|g| {
        g.round_end = 0.0;
        g.round_restart = now + 3.0;
    });
    let players = slot_players();
    let initial = cv::initial_cash();
    let result = match team {
        0 => "Specialists Win!",
        1 => "Mercenaries Win!",
        _ => "Game Draw!",
    };
    if players.len() < 2 {
        let cash = cv::initial_cash_int();
        state::with(|g| {
            for t in g.teams.iter_mut().take(2) {
                t.cash = cash;
                t.score = 0;
            }
        });
        ts_message_all([0, 0, 0], 1, 30, result);
        ts_message_all([0, 0, 0], 1, 30, "Waiting other players.");
    } else {
        state::with(|g| {
            let (spec, merc) = (g.teams[0].cash, g.teams[1].cash);
            match team {
                0 => {
                    g.teams[0].cash = round_cash(spec, win, initial, true);
                    g.teams[1].cash = round_cash(merc, lose, initial, false);
                    g.teams[0].score += 1;
                }
                1 => {
                    g.teams[1].cash = round_cash(merc, win, initial, true);
                    g.teams[0].cash = round_cash(spec, lose, initial, false);
                    g.teams[1].score += 1;
                }
                _ => {
                    g.teams[0].cash = round_cash(spec, lose, initial, false);
                    g.teams[1].cash = round_cash(merc, lose, initial, false);
                }
            }
        });
        for e in players {
            let lost = if team == 2 {
                true
            } else {
                let spec = teams::team_name_of(e).eq_ignore_ascii_case(SPECIALISTS.as_bytes());
                if spec { team != 0 } else { team != 1 }
            };
            msg::one(Msg::TsHVoice, e, |w| TsHVoice { lost: lost as u8 }.write(w));
        }
        ts_message_all([0, 0, 0], 1, 30, result);
    }
    send_round_scores();
    dev_round(&state::with(|g| {
        let t = |i: usize| g.teams.get(i).map_or((0, 0), |t| (t.cash, t.score));
        let ((sc, ss), (mc, ms)) = (t(0), t(1));
        format!("TeamWin({team}, {win}, {lose}): {SPECIALISTS} cash {sc} score {ss}, {MERCENARY} cash {mc} score {ms}")
    }));
}

/// Port addition: with `developer` on, a `[ts] round (2.1): ...` console line, for scripted tests
/// (`cargo xtask ts21`); 2.1 shows the rounds on the HUD only.
fn dev_round(text: &str) {
    if cvars::on(c"developer") {
        engine().print(&format!("[ts] round (2.1): {text}\n"));
    }
}

/// The round modes' `TeamScore`s: "Specialists" and "Mercenary".
pub fn send_round_scores() {
    let (round, scores) =
        state::with(|g| (matches!(g.mode, Mode::RoundTdm | Mode::Objective), [0, 1].map(|i| g.teams.get(i).map_or(0, |t| t.score))));
    if !round {
        return;
    }
    for (team, score) in [(SPECIALISTS, scores[0]), (MERCENARY, scores[1])] {
        let m = TeamScore { team: team.into(), score: score as i16, deaths: 0 };
        msg::all(Msg::TeamScore, |w| m.write(w));
    }
}

/// `CTSGameRules::RoundTimeLeft` (0x100450f0), mode 3's round expiry and the bomb's explosion:
/// while the round has an end time, the round manager's `targetontimeleft` (the manager as
/// activator), else a draw. Think calls it every frame past the end until a `TeamWin` (a win giver
/// the target fires, usually) ends the round.
pub fn round_time_left(now: f32) {
    if state::with(|g| g.round_end) == 0.0 {
        return;
    }
    if !objective::fire_manager_target(ManagerTarget::TimeLeft) {
        let lose = cv::cash_for_lose();
        team_win(2, lose, lose, now);
    }
}

/// The win giver's `TeamWin` (`CTSWinGiver::Use` 0x10050d10): winner 0 or 1, anything else a
/// draw; the cash is `cashforwin` / `cashforlose`, or the giver's own `victorycash` /
/// `defeatcash` while the cvar is 0 (`fcomp 0; test ah,0x44; jnp`: a NaN cvar is used), both
/// truncated; a draw pays the losing amount to both sides. The `Use` value is unused.
pub fn giver_team_win(w: &GiverWin, now: f32) {
    let (team, win, lose) = giver_cash(w, cvars::f(c"cashforwin"), cvars::f(c"cashforlose"));
    team_win(team, win, lose, now);
}

/// [`giver_team_win`]'s `TeamWin(team, win, lose)` arguments for the cvars `cashforwin` and
/// `cashforlose`.
pub fn giver_cash(w: &GiverWin, cash_for_win: f32, cash_for_lose: f32) -> (i32, i32, i32) {
    let pick = |cvar: f32, own: i32| if cvar == 0.0 { own } else { cv::trunc(cvar) };
    let lose = pick(cash_for_lose, w.defeat_cash);
    match w.team {
        0 | 1 => (w.team, pick(cash_for_win, w.victory_cash), lose),
        _ => (2, lose, lose),
    }
}

/// `CTSGameRules::ResetRestartRound(n)` (0x10044ba0), from the `restartround` cvar: in the round
/// modes or with `lastmanstanding` on, the round restarts in 3 s; `n >= 2` also starts the team
/// cash and scores again.
pub fn reset_restart_round(n: i32, now: f32) {
    let mode = state::with(|g| g.mode);
    if !(matches!(mode, Mode::RoundTdm | Mode::Objective) || cvars::on(c"lastmanstanding")) {
        return;
    }
    let restart = state::with(|g| {
        g.round_end = 0.0;
        g.round_restart = now + 3.0;
        g.round_restart
    });
    // The time to the restart, not below 0 (0x10044bf4).
    let secs = round_time_byte(x87_max0(restart as f64 - now as f64));
    msg::all(Msg::RoundTime, |w| RoundTime { secs, state: 0 }.write(w));
    if n > 1 {
        let cash = cv::initial_cash_int();
        state::with(|g| {
            for t in g.teams.iter_mut().take(2) {
                t.cash = cash;
                t.score = 0;
            }
        });
        let m = |team: &str| TeamScore { team: team.into(), score: 0, deaths: 0 };
        msg::all(Msg::TeamScore, |w| m(SPECIALISTS).write(w));
        msg::all(Msg::TeamScore, |w| m(MERCENARY).write(w));
    }
    ts_message_all([0, 0, 0], 1, 30, "Restarting Round");
}

/// `CTSGameRules::RestartRound` (0x100445d0): a new round of `roundtime` (30..180 s); in mode 2
/// the sides swap every third round; the objectives start again; every player picks up its team
/// cash (round modes), gets a join deadline (25 s), its respawn in 3 s, and goes to observer.
/// No weapon refund. Then `RespawnEntities`.
pub fn restart_round(now: f32) {
    let rt = cvars::round_time();
    let end = state::with(|g| {
        g.round_restart = 0.0;
        g.round_end = rt + now;
        g.round_end
    });
    let secs = round_time_byte(x87_max0(end as f64 - now as f64));
    msg::all(Msg::RoundTime, |w| RoundTime { secs, state: 0 }.write(w));
    let mode = state::with(|g| g.mode);
    let swapped = with(|r| {
        if mode == Mode::RoundTdm {
            r.rounds += 1;
            if r.rounds > 2 {
                r.rounds = 0;
                r.swapped = !r.swapped;
            }
        } else {
            r.swapped = false;
        }
        r.swapped
    });
    state::with(|g| {
        set_spawn_classes(g, swapped);
        init_objectives(g);
    });
    disable_objectives();
    // `SetObjective` (0x10045150): the list cleared, a random manager's `Reset`.
    let mode = state::with(|g| g.mode);
    objective::set_objective(matches!(mode, Mode::RoundTdm | Mode::Objective));
    if mode == Mode::RoundTdm {
        ts_message_all([0, 0, 0], 1, 30, "Defeat the opposite team!");
    }
    let per_round = with(|r| r.respawns_per_round);
    dev_round(&format!("RestartRound: {mode:?}, {rt} s, sides swapped {swapped}, respawns per round {per_round}"));
    for e in slot_players() {
        ts_server::player::with_player(e, |p, cx| {
            let round = matches!(mode, Mode::RoundTdm | Mode::Objective);
            if round {
                p.cash = p.cash.max(team_cash(&p.ts.team_name));
            }
            if !round || per_round != 0 || mode == Mode::RoundTdm {
                p.ts.join_deadline = now + 25.0;
            }
            p.ts.award = 11;
            p.inv.sv.grenades_bought = 0;
            p.ts.respawns_used = 0;
            p.powerup_until = 0.0;
            p.ts.held_secs = 0;
            p.active_powerup = 0;
            p.respawn_at = now + 3.0;
            p.ts.held = 0;
            if cx.pev().iuser1 == 0 {
                let (o, a) = (cx.pev().origin, cx.pev().v_angle);
                start_observer(p, cx, o, a);
            }
        });
    }
    teams::recount(false);
    crate::entities::respawn_entities(true);
}

/// `CTSGameRules::CheckDeads` (0x10045940). "Alive" is alive or with a respawn pending
/// (`respawn_at >= 0`).
///
/// * LMS (DM with `lastmanstanding` on): one left of two or more wins 10 frags and the round; one
///   left alone waits for others; with the round time up or under two players, "Round over".
/// * Mode 2: a side with nobody left loses (`TeamWin`), both empty is a draw.
/// * Mode 3: with Mercenaries but no Specialists standing the round manager's
///   `targetonspecdead`; with Specialists but no Mercenaries its `targetonmercdead`; with nobody
///   a draw. Without a manager nothing happens.
pub fn check_deads(now: f32) {
    let (restart, mode, round_end) = state::with(|g| {
        g.check_deads_at = 0.0;
        (g.round_restart, g.mode, g.round_end)
    });
    if restart != 0.0 || mode == Mode::TeamDm {
        return;
    }
    let standing = |e: *mut goldsrc::sys::edict_t| alive(e.v()) || other(e, |p| p.respawn_at >= 0.0).unwrap_or(false);
    if mode == Mode::Dm {
        if !cvars::on(c"lastmanstanding") {
            return;
        }
        let players = slot_players();
        let total = players.len();
        let living: Vec<_> = players.into_iter().filter(|&e| standing(e)).collect();
        if living.len() == 1 {
            let last = living[0];
            if total < 2 {
                ts_message_all([0, 255, 0], 1, 30, "Waiting other players.");
            } else {
                ts_message_all([0, 255, 0], 1, 30, &shared::bformat!("{} is the Last Man Standing! 10 frags", netname(last)));
                ts_server::rules::add_frags(last, None, 10.0);
                scoring::broadcast_score(last);
            }
            state::with(|g| g.round_restart = now + 3.0);
            let rt = RoundTime { secs: round_time_byte(round_end as f64 - now as f64), state: 2 };
            msg::all(Msg::RoundTime, |w| rt.write(w));
            return;
        }
        if round_end < now || total < 2 {
            ts_message_all([0, 255, 0], 1, 30, "Round over");
            state::with(|g| g.round_restart = now + 3.0);
        }
        return;
    }
    let (mut merc, mut spec) = (false, false);
    for e in slot_players() {
        if !standing(e) {
            continue;
        }
        match round_team(&teams::team_name_of(e)) {
            Some(1) => merc = true,
            Some(0) => spec = true,
            _ => {}
        }
    }
    let (win, lose) = (cv::cash_for_win(), cv::cash_for_lose());
    if mode == Mode::Objective {
        match (spec, merc) {
            (false, true) => {
                objective::fire_manager_target(ManagerTarget::SpecDead);
            }
            (true, false) => {
                objective::fire_manager_target(ManagerTarget::MercDead);
            }
            (false, false) => team_win(2, lose, lose, now),
            (true, true) => {}
        }
        return;
    }
    match (spec, merc) {
        (false, false) => team_win(2, lose, lose, now),
        (true, false) => team_win(0, win, lose, now),
        (false, true) => team_win(1, win, lose, now),
        (true, true) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn win_giver_cash() {
        let w = GiverWin { team: 1, value: 7.0, restart: true, is_score: false, victory_cash: 2000, defeat_cash: -500 };
        assert_eq!(giver_cash(&w, 1500.0, -1000.0), (1, 1500, -1000), "the cvars");
        assert_eq!(giver_cash(&w, 0.0, 0.0), (1, 2000, -500), "the giver's own while a cvar is 0");
        assert_eq!(giver_cash(&w, f32::NAN, 0.0), (1, i32::MIN, -500), "a NaN cvar is used");
        let draw = GiverWin { team: 3, ..w };
        assert_eq!(giver_cash(&draw, 1500.0, -1000.0), (2, -1000, -1000), "a draw pays the losing amount");
    }

    #[test]
    fn objective_rules_are_2_1s() {
        let r = OBJECTIVES;
        assert_eq!(r.hud_effect, 1);
        assert_eq!(r.team_names, Some([SPECIALISTS, MERCENARY]));
        assert!(!r.indexed_list && !r.pointer_enables_givers && !r.wire_hud);
        assert!(!r.nan_time_runs_out && !r.clear_list_outside_rounds);
    }

    #[test]
    fn the_one_check_skips_the_last_slot() {
        // Issue #127: `for (i = 0; i < maxClients; i++)`, so slot maxClients is never seen.
        assert_eq!(the_one_check_slots(2).collect::<Vec<_>>(), [0, 1]);
        assert!(!the_one_check_slots(32).any(|i| i == 32));
    }

    #[test]
    fn the_map_prefix_decides_the_mode() {
        assert_eq!(decide_gameplay("tm_temple", false, 0.0), Mode::Objective);
        assert_eq!(decide_gameplay("tm_temple", true, 1.0), Mode::Objective);
        for m in ["tr_x", "cs_office", "de_dust"] {
            assert_eq!(decide_gameplay(m, false, 0.0), Mode::RoundTdm, "{m}");
            assert_eq!(decide_gameplay(m, true, 1.0), Mode::RoundTdm, "{m}");
        }
        assert_eq!(decide_gameplay("ts_dojo", true, 0.0), Mode::TeamDm, "The One makes ts_ maps teamplay");
        assert_eq!(decide_gameplay("ts_dojo", false, 0.0), Mode::Dm);
        assert_eq!(decide_gameplay("ts_dojo", false, 1.0), Mode::TeamDm);
        // Any other prefix follows mp_teamplay only; The One doesn't make it teamplay.
        assert_eq!(decide_gameplay("MecklenburgV", true, 0.0), Mode::Dm);
        assert_eq!(decide_gameplay("MecklenburgV", false, 2.0), Mode::TeamDm);
        // Case-sensitive, like strcmp.
        assert_eq!(decide_gameplay("TM_temple", false, 0.0), Mode::Dm);
        assert_eq!(decide_gameplay("tm", false, 0.0), Mode::Dm);
        assert_eq!(decide_gameplay("", false, 0.0), Mode::Dm);
    }

    /// Byte names as text, for readable asserts.
    fn strs<T: AsRef<[u8]>>(v: &[T]) -> Vec<String> {
        v.iter().map(|n| String::from_utf8_lossy(n.as_ref()).into_owned()).collect()
    }

    #[test]
    fn round_modes_have_two_teams_with_cash() {
        let mut g = GameRules::default();
        setup_teams(&mut g, Mode::RoundTdm, false, b"hitman;gordon", [b"gordon", b"seal"], 3000);
        assert_eq!(strs(&g.team_names()), [SPECIALISTS, MERCENARY]);
        assert!(g.team_limit);
        assert_eq!((strs(&g.teams[0].models), strs(&g.teams[1].models)), (vec!["seal".to_owned()], vec!["merc".to_owned()]));
        assert_eq!((g.teams[0].cash, g.teams[1].cash), (3000, 3000));
        assert_eq!((g.teams[0].spawn_class.as_str(), g.teams[1].spawn_class.as_str()), ("info_player_start", "info_player_deathmatch"));
        set_spawn_classes(&mut g, true);
        assert_eq!((g.teams[0].spawn_class.as_str(), g.teams[1].spawn_class.as_str()), ("info_player_deathmatch", "info_player_start"));
        assert_eq!(g.teams[0].fallback, g.teams[0].spawn_class, "no fallback class in 2.1");
    }

    #[test]
    fn teamplay_teams_are_models() {
        let mut g = GameRules::default();
        setup_teams(&mut g, Mode::TeamDm, false, b"hitman;gordon;hitman", [b"gordon", b"seal"], 3000);
        assert_eq!(strs(&g.team_names()), ["hitman", "gordon"]);
        assert!(g.team_limit);
        assert_eq!(strs(&g.teams[g.team_index(b"gordon") as usize].models), ["gordon"]);
        // The One: its teams come first, from the model cvars.
        let mut g = GameRules::default();
        setup_teams(&mut g, Mode::TeamDm, true, b"hitman;gordon", [b"gordon", b"seal"], 3000);
        assert_eq!(strs(&g.the_one_team_names()), ["gordon", "seal"]);
        assert_eq!(strs(&g.team_names()), ["hitman", "gordon"]);
        // An empty list means no team limit.
        let mut g = GameRules::default();
        setup_teams(&mut g, Mode::TeamDm, false, b"", [b"gordon", b"seal"], 3000);
        assert!(!g.team_limit);
    }

    #[test]
    fn globalslow_is_a_clamped_fraction() {
        assert_eq!(globalslow_value(0.5, false), 0.5);
        assert_eq!(globalslow_value(1.0, false), 0.0, "1.0 means off");
        assert_eq!(globalslow_value(1.5, false), 0.0, "clamped to 1 without cheats: off");
        assert_eq!(globalslow_value(1.5, true), 1.5);
        assert_eq!(globalslow_value(3.0, true), 2.0);
        assert_eq!(globalslow_value(-1.0, true), 0.0);
        assert_eq!(globalslow_step(0.5, 0.0, 0.0), SlowStep::Set(0.5));
        assert_eq!(globalslow_step(0.5, 0.5, 0.5), SlowStep::Keep);
        assert_eq!(globalslow_step(0.0, 0.5, 0.5), SlowStep::Stop);
        assert_eq!(globalslow_step(0.0, 0.0, 0.0), SlowStep::Keep);
        // `Init` stores -1 as the last value, so the first Think with the cvar at 0 stops.
        assert_eq!(globalslow_step(0.0, 0.0, -1.0), SlowStep::Stop);
    }

    #[test]
    fn weaponrestriction_values() {
        assert_eq!(restriction_mask(0.0, 0x1f), 0x1f);
        assert_eq!(restriction_mask(1.0, 0x1f), 0);
        assert_eq!(restriction_mask(2.0, 0x1f), 0x01);
        assert_eq!(restriction_mask(3.0, 0x1f), 0x03);
        assert_eq!(restriction_mask(4.0, 0x1f), 0x13);
        assert_eq!(restriction_mask(4.0, 0x03), 0x03, "masked by the map");
        assert_eq!(restriction_mask(5.0, 0x03), 0x1f, "5 ignores the map");
        assert_eq!(restriction_mask(2.9, 0x1f), 0x01, "truncated");
    }

    #[test]
    fn round_cash_floors_and_caps() {
        assert_eq!(round_cash(3000, 1500, 3000.0, true), 4500);
        assert_eq!(round_cash(29500, 1500, 3000.0, true), 30000);
        assert_eq!(round_cash(3000, -1000, 3000.0, false), 3000, "the loser keeps initial_cash");
        assert_eq!(round_cash(29500, 1500, 3000.0, false), 31000, "only the winner is capped");
        assert_eq!(round_cash(5000, -1000, 3000.0, false), 4000);
    }

    #[test]
    fn rules_state_starts_like_init() {
        let r = Rules21::default();
        assert_eq!((r.rounds, r.swapped, r.last_globalslow, r.srvsett), (-1, false, -1.0, [-1; 3]));
    }
}
