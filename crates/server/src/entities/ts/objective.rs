//! TS's objective system (decomp `docs/server/gamerules-entities.md` §3, issue #66):
//! `ts_objective_manager`, `ts_objective_ptr`, `ts_bomb`, `ts_hack`, `ts_teamescape`, the HUD's
//! objective list (`Objective`, `ShowObj`), the bomb's wires (`cutwire_<n>`, `Wires`) and the
//! round rules' part (`InitObjectives`, `SetObjective`, `RoundTimeLeft`, the manager's targets).
//!
//! In TS 3.0 the round modes that pick a manager are unreachable, but the entities are not dead: a
//! `ts_objective_ptr` fired by any entity enables its targets in any mode and puts them on the HUD,
//! and an enabled bomb, hack or escape zone runs. Their timers read the round's end, which is 0
//! outside a round, so in deathmatch or teamplay a bomb's countdown starts below zero and it
//! explodes on its next think, and a hack freezes on its next think. A ruleset changes the details
//! through [`ObjectiveRules`] (`ServerRules::objectives`).

use super::ctb::WinGiver;
use crate::entities::stock::base::{alloc, kv_i};
use crate::entities::stock::explode::{emit_sound_slow, ts_explosion};
use crate::messages::Msg;
use crate::player::Player;
use crate::rules::{alive, has_player, msg, other, state, teams};
use crate::save::{BaseFuncs, Fields, tables};
use crate::world::{self, Cx, Entity, UseType};
use goldsrc::edict::EdictPtr;
use goldsrc::server::engine;
use goldsrc::sys::*;
use shared::protocol::rules::{Objective, ShowObj, Wires};
use std::any::{Any, TypeId};
use std::ffi::CStr;

/// What a ruleset changes in the objective system ([`ServerRules::objectives`](crate::gameplay::ServerRules::objectives)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObjectiveRules {
    /// The `TSHudMessageAll` effect of the objective lines (green, 30 s hold): the manager's
    /// messages, a completed bomb, hack or escape, a defusing wire, a win giver's message.
    pub hud_effect: u8,
    /// The two round teams' names: `attackerteam` 0 and 1 are the players whose team is the first
    /// or second name (`CanInteract`, the escape zone's head count), other values nobody, and a
    /// player on the first team reads the pointer's first text, anyone else its second. `None`
    /// (TS 3.0): `attackerteam` is a team index, -1 meaning anyone, and the text is the player's
    /// team index's.
    pub team_names: Option<[&'static str; 2]>,
    /// The HUD list as TS 3.0 keeps it (`AddObjectiveInList` 0x7ac00): an objective already listed
    /// keeps its index and the first pointer's texts, `Freeze` sends `ShowObj(idx, visible)` both
    /// ways, and `InitHUD` and a team change in the round modes send the list again. `false`: every
    /// `Use` appends (up to 32), only `Freeze(true)` sends `ShowObj(idx, 0)` (an objective that
    /// isn't in the list empties it: TS writes slot -1, the count), and nothing is resent.
    pub indexed_list: bool,
    /// `ts_objective_ptr` enables any `IsObjective()` entity, the win and score givers included
    /// (which also go on the list); `false`: only `ts_bomb`, `ts_hack` and `ts_teamescape`.
    pub pointer_enables_givers: bool,
    /// A bomb's or hack's timer runs out at `!(t > 0)`, so a NaN runs out (3.0: `test ah,0x41;
    /// je`); `false`: at `t <= 0` only (`jp`), and a bomb's wires can be cut while `t` is NaN.
    pub nan_time_runs_out: bool,
    /// `SetObjective` clears the list outside the round modes too (3.0's LMS restart and `Init`).
    pub clear_list_outside_rounds: bool,
}

fn rules() -> &'static ObjectiveRules {
    crate::gameplay::active().server.objectives()
}

/// The objective lines: `TSHudMessageAll(0, 255, 0, effect, 30, text)`.
pub fn hud_line(text: string_t) {
    if text != 0 {
        msg::ts_message_all([0, 255, 0], rules().hud_effect, 30, engine().string(text).to_bytes());
    }
}

/// The objective HUD list (`CTSGameRules +0x2d0..`) and the round's managers (`+0x24c..`,
/// `+0x554`), kept in [`GameRules`](state::GameRules).
#[derive(Debug)]
pub struct Objectives {
    /// The listed objectives (32 slots).
    pub slots: [*mut edict_t; MAX_LISTED],
    /// Each listed objective's four team texts (the pointer's `objmsgspec`, `objmsgmerc`,
    /// `objmsgteam3`, `objmsgteam4`).
    pub texts: [[string_t; 4]; MAX_LISTED],
    pub count: i32,
    /// `InitObjectives`' `ts_objective_manager`s (at most 32).
    pub managers: Vec<*mut edict_t>,
    /// `SetObjective`'s pick: the round's manager.
    pub manager: *mut edict_t,
}

pub const MAX_LISTED: usize = 32;

impl Default for Objectives {
    fn default() -> Self {
        Objectives {
            slots: [std::ptr::null_mut(); MAX_LISTED],
            texts: [[0; 4]; MAX_LISTED],
            count: 0,
            managers: Vec::new(),
            manager: std::ptr::null_mut(),
        }
    }
}

impl Objectives {
    /// `ClearObjectiveList` (0x7abdc).
    pub fn clear_list(&mut self) {
        self.count = 0;
        self.slots = [std::ptr::null_mut(); MAX_LISTED];
        self.texts = [[0; 4]; MAX_LISTED];
    }

    /// `FindIndexByObjective` (0x7ae20): among the listed ones, -1 when absent.
    pub fn find(&self, e: *mut edict_t) -> i32 {
        (0..self.count.clamp(0, MAX_LISTED as i32)).find(|&i| self.slots[i as usize] == e).unwrap_or(-1)
    }

    /// `AddObjectiveInList` (0x7ac00): the index of `e` in the list, -1 when it is full. With
    /// `indexed`, a listed objective keeps its index (and its first texts); otherwise each call
    /// appends.
    pub fn add(&mut self, e: *mut edict_t, texts: [string_t; 4], indexed: bool) -> i32 {
        if indexed {
            let i = self.find(e);
            if i >= 0 {
                return i;
            }
        }
        if !(0..MAX_LISTED as i32).contains(&self.count) {
            return -1;
        }
        let i = self.count as usize;
        self.slots[i] = e;
        self.texts[i] = texts;
        self.count += 1;
        i as i32
    }

    /// The appending list's removal (2.1's `DelObj` FUN_10046690): the slot of `e` among all 32
    /// emptied, and its index; one that isn't there writes slot -1, which is the count: the list
    /// starts over.
    pub fn remove_appended(&mut self, e: *mut edict_t) -> i32 {
        match self.slots.iter().position(|&s| s == e) {
            Some(i) => {
                self.slots[i] = std::ptr::null_mut();
                i as i32
            }
            None => {
                self.count = 0;
                -1
            }
        }
    }
}

/// `CBaseEntity::Center`: `(absmax + absmin) * 0.5`.
fn center(v: &entvars_t) -> [f32; 3] {
    std::array::from_fn(|i| (v.absmax[i] + v.absmin[i]) * 0.5)
}

/// The text a player reads for an objective: by team index (3.0's `GetTeamIndex`, nothing without
/// a team) or, with the round teams' names, the first text for the first team and the second for
/// anyone else.
fn team_text(texts: &[string_t; 4], team: &[u8], team_index: i32, names: Option<[&str; 2]>) -> string_t {
    match names {
        Some(n) => texts[if state::team_eq(team, n[0].as_bytes()) { 0 } else { 1 }],
        None => usize::try_from(team_index).ok().and_then(|i| texts.get(i).copied()).unwrap_or(0),
    }
}

/// `strncpy(buf, text, 0x40)` with the 64th byte zeroed: at most 63 bytes.
fn clip63(s: &[u8]) -> Vec<u8> {
    s[..s.len().min(63)].to_vec()
}

fn team_index(e: *mut edict_t) -> i32 {
    let t = teams::team_name_of(e);
    state::with(|g| g.team_index(&t))
}

/// `SendObjectiveToHud(idx, player)` (0x7ac80): the listed objective's centre, its index and the
/// player's text, to that player.
pub fn send_to_hud(idx: i32, player: *mut edict_t) {
    if player.is_null() {
        return;
    }
    let Some((_, [x, y, z], text)) = hud_entry(idx, player) else { return };
    msg::one(Msg::Objective, player, |w| Objective { x, y, z, idx: idx as u8, text }.write(w));
}

/// What `SendObjectiveToHud` sends `player` for list entry `idx`: the objective, its `Center()`
/// and the player's text. A null `player` reads as no team. `None` for an index past the list or
/// an emptied slot.
fn hud_entry(idx: i32, player: *mut edict_t) -> Option<(*mut edict_t, [f32; 3], Vec<u8>)> {
    let (obj, texts) = state::with(|g| {
        let o = &g.objectives;
        (0..o.count.min(MAX_LISTED as i32)).contains(&idx).then(|| (o.slots[idx as usize], o.texts[idx as usize]))
    })?;
    if obj.is_null() {
        return None;
    }
    let (team, index) = if player.is_null() { (Vec::new(), -1) } else { (teams::team_name_of(player), team_index(player)) };
    let t = team_text(&texts, &team, index, rules().team_names);
    let text = if t == 0 { Vec::new() } else { clip63(engine().string(t).to_bytes()) };
    Some((obj, crate::ham::center(obj, center), text))
}

/// The HUD list as the `Objective` messages give it to `player` (null: no team), in list order:
/// index, objective, centre and text (the plugin API's `objectives`, port addition).
pub fn hud_list(player: *mut edict_t) -> Vec<(i32, *mut edict_t, [f32; 3], Vec<u8>)> {
    let n = state::with(|g| g.objectives.count.clamp(0, MAX_LISTED as i32));
    (0..n).filter_map(|i| hud_entry(i, player).map(|(e, c, t)| (i, e, c, t))).collect()
}

/// Whether objective `e` can be completed now: a bomb, hack or escape zone while enabled
/// (`CTSObjective` +0x64, `Freeze`), a win or score giver until it has fired. `None` for anything
/// else, or an object that can't be reached.
pub fn objective_active(e: *mut edict_t) -> Option<bool> {
    world::with_lent::<Bomb, _>(e, |b| b.enabled)
        .or_else(|| world::with_lent::<Hack, _>(e, |h| h.enabled))
        .or_else(|| world::with_lent::<TeamEscape, _>(e, |t| t.enabled))
        .or_else(|| super::ctb::giver_active(e))
}

/// `SendObjectiveList` (0x7add4): the whole list to `player`, from `InitHUD` and a team change in
/// the round modes when the ruleset resends it ([`ObjectiveRules::indexed_list`]).
pub fn resend_list(player: *mut edict_t) {
    let round = state::with(|g| matches!(g.mode, state::Mode::RoundTdm | state::Mode::Objective));
    if !round || !rules().indexed_list {
        return;
    }
    for i in 0..state::with(|g| g.objectives.count) {
        send_to_hud(i, player);
    }
}

/// `CTSObjective::Freeze(b)` (0x8c310): the objective is enabled while not frozen, and the HUD
/// shows or hides it (`ShowObjectiveOnHud`, the index -1 going out as 255). The appending list
/// only removes it on a freeze ([`Objectives::remove_appended`]).
fn freeze(e: *mut edict_t, enabled: &mut bool, frozen: bool) {
    *enabled = !frozen;
    let idx = if rules().indexed_list {
        state::with(|g| g.objectives.find(e))
    } else if frozen {
        state::with(|g| g.objectives.remove_appended(e))
    } else {
        return;
    };
    msg::all(Msg::ShowObj, |w| ShowObj { idx: idx as u8, visible: (!frozen) as u8 }.write(w));
}

/// Whether a bomb's or hack's timer `t` has run out ([`ObjectiveRules::nan_time_runs_out`]).
fn ran_out(nan_runs_out: bool, t: f32) -> bool {
    if nan_runs_out { t.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) } else { t <= 0.0 }
}

/// `CTSObjective::CanInteract(player)` (0x8c344): `attackerteam` -1 or the player's team index
/// (3.0), or the round team named for 0 / 1.
pub fn can_interact(attacker_team: i32, team: &[u8], team_index: i32, names: Option<[&str; 2]>) -> bool {
    match names {
        Some(n) => matches!(attacker_team, 0 | 1) && state::team_eq(team, n[attacker_team as usize].as_bytes()),
        None => attacker_team == -1 || attacker_team == team_index,
    }
}

fn may_use(attacker_team: i32, player: *mut edict_t) -> bool {
    can_interact(attacker_team, &teams::team_name_of(player), team_index(player), rules().team_names)
}

/// `TSCountPlayers(team, true)` (0x9f238): the players on `team` (any, for an empty name) who are
/// alive or still due to respawn (`m_flRespawnTime >= 0`, or a NaN).
fn count_players(team: &[u8]) -> i32 {
    let mut n = 0;
    for e in crate::rules::slot_players() {
        if !team.is_empty() && !state::team_eq(team, &teams::team_name_of(e)) {
            continue;
        }
        if !alive(e.v()) && other(e, |p| p.respawn_at < 0.0).unwrap_or(false) {
            continue;
        }
        n += 1;
    }
    n
}

/// The escape zone's head count of its attackers' team (`WaitTouch`).
fn attackers_alive(attacker_team: i32) -> i32 {
    match rules().team_names {
        Some(n) => count_players(n[if attacker_team == 0 { 0 } else { 1 }].as_bytes()),
        // `attackerteam` -1 counts nobody; TS reads past its four teams for a larger index, which
        // the port counts as nobody too.
        None => match usize::try_from(attacker_team).ok().and_then(|i| state::with(|g| g.teams.get(i).map(|t| t.name.clone()))) {
            Some(name) => count_players(&name),
            None => 0,
        },
    }
}

/// How many attackers `escapepercentage` needs (`CTSTeamEscape::WaitTouch` 0x8d4ec): 0 one, 1 a
/// third, 2 two thirds (of the thirds), 3 all of the living ones; anything else none.
pub fn escape_needed(percentage: i32, living: i32) -> i32 {
    match percentage {
        0 => 1,
        1 => living / 3,
        2 => (living / 3) * 2,
        3 => living,
        _ => 0,
    }
}

/// `CTSHack::Use`'s hacking time from `hackingtime`: at least 1 and at most 30 s.
pub fn hack_time(secs: i32) -> f32 {
    if secs < 2 {
        1.0
    } else if secs < 30 {
        secs as f32
    } else {
        30.0
    }
}

/// The objective HUD's "%i seconds left" (`ClientPrint(HUD_PRINTCENTER)`): the time left, not
/// below 0, truncated (`__ftol`; a NaN gives the integer indefinite).
fn print_seconds_left(player: *mut edict_t, left: f32) {
    let l = if left < 0.0 { 0.0 } else { left };
    let secs = if l > -2147483649.0 && l < 2147483648.0 { l as i32 } else { i32::MIN };
    msg::text_one(player, shared::protocol::rules::hud_dest::CENTER, &format!("{secs} seconds left"));
}

/// The round's time left (`TSGameRules +8` less the time), which the bomb and hack count down.
fn round_left(now: f32) -> f32 {
    state::with(|g| g.round_end) - now
}

/// `ltime + d` with the `double` constant (`fadd qword`), rounded once.
fn next_ltime(cx: &mut Cx, d: f64) {
    let v = cx.pev();
    v.nextthink = shared::pm::plus_double(v.ltime, d);
}

/// A bomb's or hack's sound (`EMIT_SOUND_DYN(CHAN_STATIC, ..., RandomFloat(0.92, 1), 1.25, 0,
/// 98 + RandomLong(0, 3))`, slow factor 0.5): the pitch is drawn first.
fn objective_sound(e: *mut edict_t, sample: &CStr) {
    let eng = engine();
    let pitch = 98 + eng.random_long(0, 3);
    let vol = eng.random_float(0.92, 1.0);
    emit_sound_slow(e, CHAN_STATIC, sample, vol, 1.25, 0, pitch, 0.5);
}

/// The brush objectives' `EnableObjective` look (bomb, hack): `true` draws it, `SOLID_BSP`,
/// `MOVETYPE_PUSH`, `FL_WORLDBRUSH`, the model set again; `false` hides it and clears the flags.
fn show_brush(cx: &mut Cx, on: bool) {
    if on {
        let v = cx.pev();
        v.effects &= !(EF_NODRAW as i32);
        v.solid = SOLID_BSP as i32;
        v.movetype = MOVETYPE_PUSH as i32;
        v.flags |= FL_WORLDBRUSH as i32;
        let m = v.model;
        if m != 0 {
            engine().set_model(cx.edict, engine().string(m));
        }
    } else {
        let v = cx.pev();
        v.effects = EF_NODRAW as i32;
        v.solid = SOLID_NOT as i32;
        v.flags = 0;
        v.movetype = MOVETYPE_NONE as i32;
    }
}

/// The brush objectives' `Spawn` (bomb, hack): the model and origin, hidden, no collision,
/// `WaitThink` a second after `ltime`.
fn spawn_brush(cx: &mut Cx) {
    let eng = engine();
    let m = cx.pev().model;
    if m != 0 {
        eng.set_model(cx.edict, eng.string(m));
    }
    let o = cx.pev().origin;
    eng.set_origin(cx.edict, o);
    show_brush(cx, false);
    next_ltime(cx, 1.0);
}

/// The completion of a bomb, hack or escape: its `target` fired (`USE_TOGGLE`, itself as the
/// activator), its `message` on the HUD.
fn complete(cx: &mut Cx) {
    let (target, message) = (cx.pev().target, cx.pev().message);
    if target != 0 {
        crate::entities::fire_targets(engine().string(target), cx.edict, cx.edict, UseType::Toggle, 0.0);
    }
    hud_line(message);
}

/// `CTSObjective`'s saved `attacker_team` (`+0x68`): `CTSObjective::KeyValue`'s `attackerteam`.
fn objective_key(attacker_team: &mut i32, key: &CStr, value: &CStr) -> bool {
    if key.to_bytes() == b"attackerteam" {
        *attacker_team = kv_i(value);
        true
    } else {
        false
    }
}

// ---- ts_objective_manager -----------------------------------------------------------------------

/// `ts_objective_manager` (`CTSObjectiveManager`): the round's HUD lines and the targets it fires
/// when a round picks it, and the targets the round rules fire when the time runs out or a side
/// is dead.
#[derive(Default)]
pub struct ObjectiveManager {
    /// `messages[4]` (`+0x64`): `messagea`..`messaged`.
    messages: [string_t; 4],
    /// `targets[8]` (`+0x74`): `objptargeta`..`objptargeth`.
    targets: [string_t; 8],
    /// `+0x94`: `targetontimeleft`.
    end_round_target: string_t,
    /// `+0x98`, `+0x9c`: `targetonspecdead`, `targetonmercdead` (not saved).
    spec_dead_target: string_t,
    merc_dead_target: string_t,
}

/// The manager targets the round rules fire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManagerTarget {
    /// `targetontimeleft`: the round time ran out.
    TimeLeft,
    /// `targetonspecdead`: no Specialist standing.
    SpecDead,
    /// `targetonmercdead`: no Mercenary standing.
    MercDead,
}

impl Entity for ObjectiveManager {
    /// `CTSObjectiveManager::KeyValue` (0x8b46c, `ALLOC_STRING`).
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        let k = key.to_bytes();
        let slot = match k {
            b"messagea" | b"messageb" | b"messagec" | b"messaged" => &mut self.messages[(k[7] - b'a') as usize],
            b"objptargeta" | b"objptargetb" | b"objptargetc" | b"objptargetd" | b"objptargete" | b"objptargetf" | b"objptargetg"
            | b"objptargeth" => &mut self.targets[(k[10] - b'a') as usize],
            b"targetontimeleft" => &mut self.end_round_target,
            b"targetonspecdead" => &mut self.spec_dead_target,
            b"targetonmercdead" => &mut self.merc_dead_target,
            _ => return false,
        };
        *slot = alloc(value);
        true
    }

    /// `CTSObjectiveManager::Spawn` (0x8b7bc).
    fn spawn(&mut self, cx: &mut Cx) {
        spawn_point_entity(cx);
    }

    /// `CTSObjectiveManager::Save` / `Restore`: no member pointers (`Spawn` 0x8b7bc clears them),
    /// then the thirteen strings.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut Fields) {
        f.base_funcs(cx, BaseFuncs::default());
        f.table(&tables::CTSObjectiveManager);
        for m in &mut self.messages {
            f.string(m);
        }
        for t in &mut self.targets {
            f.string(t);
        }
        f.string(&mut self.end_round_target);
    }
}

impl ObjectiveManager {
    /// `CTSObjectiveManager::Reset` (0x8b850): the four lines on everyone's HUD, then the eight
    /// targets fired (`USE_TOGGLE`, the manager as activator and caller).
    fn reset(&mut self, cx: &mut Cx) {
        for m in self.messages {
            hud_line(m);
        }
        for t in self.targets {
            if t != 0 {
                crate::entities::fire_targets(engine().string(t), cx.edict, cx.edict, UseType::Toggle, 0.0);
            }
        }
    }

    fn target(&self, which: ManagerTarget) -> string_t {
        match which {
            ManagerTarget::TimeLeft => self.end_round_target,
            ManagerTarget::SpecDead => self.spec_dead_target,
            ManagerTarget::MercDead => self.merc_dead_target,
        }
    }
}

/// The manager and pointer `Spawn`s (0x8b7bc, 0x8bb44): hidden, no collision, no flags, and a
/// `nextthink` 10 s on with no think set.
fn spawn_point_entity(cx: &mut Cx) {
    let v = cx.pev();
    v.effects = EF_NODRAW as i32;
    v.solid = SOLID_NOT as i32;
    v.flags = 0;
    v.movetype = MOVETYPE_NONE as i32;
    cx.think_in(10.0);
}

// ---- ts_objective_ptr ---------------------------------------------------------------------------

/// `ts_objective_ptr` (`CTSObjectivePointer`): when used, enables the objectives it targets and
/// puts them on everyone's HUD. TS saves only its `CBaseEntity` part, so the texts don't survive a
/// save.
#[derive(Default)]
pub struct ObjectivePointer {
    /// `+0x64..+0x70`: `objmsgspec`, `objmsgmerc`, `objmsgteam3`, `objmsgteam4`.
    texts: [string_t; 4],
    /// `+0x74`: `objmsgall`, parsed and never sent.
    all: string_t,
}

impl Entity for ObjectivePointer {
    /// `CTSObjectivePointer::KeyValue` (0x8ba1c).
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        let slot = match key.to_bytes() {
            b"objmsgspec" => &mut self.texts[0],
            b"objmsgmerc" => &mut self.texts[1],
            b"objmsgteam3" => &mut self.texts[2],
            b"objmsgteam4" => &mut self.texts[3],
            b"objmsgall" => &mut self.all,
            _ => return false,
        };
        *slot = alloc(value);
        true
    }

    /// `CTSObjectivePointer::Spawn` (0x8bb44).
    fn spawn(&mut self, cx: &mut Cx) {
        spawn_point_entity(cx);
    }

    /// `CTSObjectivePointer::Use` (0x8bbd8): no mode check. Every entity named by `target` that is
    /// an objective is enabled and listed, and each player gets it (`SendObjectiveToHud`) unless
    /// the list is full.
    fn use_(&mut self, cx: &mut Cx, _a: *mut edict_t, _c: *mut edict_t, _k: UseType, _v: f32) {
        let target = cx.pev().target;
        if target == 0 {
            return;
        }
        let name = engine().string(target);
        let givers = rules().pointer_enables_givers;
        let mut e = std::ptr::null_mut();
        loop {
            e = engine().find_by_string(e, c"targetname", name);
            if e.is_null() || engine().index_of(e) == 0 {
                break;
            }
            // `pEntity->IsObjective()`, `pEntity->EnableObjective(TRUE)`: hookable.
            if !crate::ham::is_objective(e, || world::type_of(e).is_some_and(|t| is_objective(t, givers))) {
                continue;
            }
            crate::ham::enable_objective(e, true);
            let idx = state::with(|g| g.objectives.add(e, self.texts, rules().indexed_list));
            if idx == -1 {
                continue;
            }
            let eng = engine();
            for i in 1..=eng.max_clients() {
                let p = eng.player(i);
                if has_player(p) {
                    send_to_hud(idx, p);
                }
            }
        }
    }
}

/// The objective classes (`IsObjective()`): the bomb, hack and escape zone, and with `givers`
/// the win and score givers.
pub fn is_objective(ty: TypeId, givers: bool) -> bool {
    ty == TypeId::of::<Bomb>()
        || ty == TypeId::of::<Hack>()
        || ty == TypeId::of::<TeamEscape>()
        || (givers && (ty == TypeId::of::<WinGiver<false>>() || ty == TypeId::of::<WinGiver<true>>()))
}

/// `EnableObjective(on)` (vtable +0x20) on an objective (a win or score giver only starts counting
/// again); nothing for anything else.
pub fn enable_objective(ent: &mut dyn Entity, cx: &mut Cx, on: bool) {
    let any = ent as &mut dyn Any;
    if let Some(b) = any.downcast_mut::<Bomb>() {
        b.enable(cx, on);
    } else if let Some(h) = any.downcast_mut::<Hack>() {
        h.enable(cx, on);
    } else if let Some(t) = any.downcast_mut::<TeamEscape>() {
        t.enable(cx, on);
    } else {
        super::ctb::enable_objective(ent);
    }
}

// ---- ts_bomb ------------------------------------------------------------------------------------

/// `ts_bomb` (`CTSBomb`): a brush shown while enabled, counting the round's time left down once a
/// second and beeping; `+use` held defuses it in `defusetime` seconds, and with `cutwires` its three
/// wires can be cut (`cutwire_<n>`). At 0 the round's time is up and it explodes.
#[derive(Default)]
pub struct Bomb {
    /// `CTSObjective` `+0x64` / `+0x68`.
    enabled: bool,
    attacker_team: i32,
    /// `+0x6c`: seconds to the explosion; `+0x70`: the next beep.
    countdown: f32,
    next_beep: f32,
    /// `+0x74`: defusing seconds left; `+0x78`: the last `Use`.
    defuse_left: f32,
    last_use: f32,
    /// `+0x7c..+0x7e`: the wires (0 cut, 1 defuse, 2 hurry, 3 detonate); `+0x7f`: hurried.
    wires: [u8; 3],
    hurried: bool,
    /// `defusesec` (`+0x80`): `defusetime`.
    defuse_sec: i32,
    /// `cancutwire` (`+0x84`): `cutwires`.
    can_cut_wire: i32,
    /// `Spawn` (0x8c470) sets `m_pfnThink` (`WaitThink`).
    funcs: BaseFuncs,
}

impl Entity for Bomb {
    /// `CTSBomb::KeyValue` (0x8cb68), then `CTSObjective::KeyValue`.
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"defusetime" => self.defuse_sec = kv_i(value),
            b"cutwires" => self.can_cut_wire = kv_i(value),
            _ => return objective_key(&mut self.attacker_team, key, value),
        }
        true
    }

    /// `CTSBomb::Spawn` (0x8c470).
    fn spawn(&mut self, cx: &mut Cx) {
        spawn_brush(cx);
        self.enabled = false;
        self.funcs.think = true;
        (self.countdown, self.next_beep, self.defuse_left, self.last_use) = (0.0, 0.0, 0.0, 0.0);
        self.wires = [0; 3];
        self.hurried = false;
    }

    /// `CTSBomb::WaitThink` (mp.dll 0x1005c7b0): every second (a tenth once hurried) of `ltime`.
    fn think(&mut self, cx: &mut Cx) {
        next_ltime(cx, if self.hurried { 0.1 } else { 1.0 });
        if !self.enabled {
            return;
        }
        self.countdown = (f64::from(self.countdown) - 1.0) as f32;
        if ran_out(rules().nan_time_runs_out, self.countdown) {
            crate::gameplay::active().server.round_time_left(cx.time());
            freeze(cx.edict, &mut self.enabled, true);
            // `Center()`: hookable (`Ham_Center`; the bomb is in its own think, out of reach).
            let c = crate::ham::center(cx.edict, center);
            let bits = crate::damage::bits::BLAST | crate::damage::bits::NEVERGIB;
            ts_explosion(c, cx.edict, cx.edict, 300.0, bits, 2048.0, 4096.0);
            return;
        }
        let now = cx.time();
        if self.next_beep < now {
            self.next_beep = shared::pm::plus_double(now, if self.hurried { 0.1 } else { 1.0 });
            objective_sound(cx.edict, c"misc/beep2.wav");
        }
    }

    /// `CTSBomb::Use` (mp.dll 0x1005c8e0): held `+use` by a player who may use it.
    fn use_(&mut self, cx: &mut Cx, activator: *mut edict_t, _c: *mut edict_t, _k: UseType, _v: f32) {
        if !self.enabled || !has_player(activator) || !may_use(self.attacker_team, activator) {
            return;
        }
        let now = cx.time();
        (self.defuse_left, self.last_use) = held_use(self.defuse_left, self.last_use, now, self.defuse_sec as f32);
        print_seconds_left(activator, self.defuse_left);
        self.last_use = now;
        if self.defuse_left < 0.0 {
            complete(cx);
            (self.countdown, self.next_beep) = (0.0, 0.0);
            freeze(cx.edict, &mut self.enabled, true);
        }
    }

    /// `CTSBomb::Save` / `Restore`: `"CTSObjective"` then `"CTSBomb"`. The set think stops TS's
    /// restore after `"BASE"`.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut Fields) {
        super::save_base(cx, f, &mut self.funcs);
        f.table(&tables::CTSObjective);
        f.int(&mut self.attacker_team);
        f.table(&tables::CTSBomb);
        f.int(&mut self.defuse_sec);
        f.int(&mut self.can_cut_wire);
    }

    /// `CTSBomb::Precache` (0x8c570).
    fn restored(&mut self, _cx: &mut Cx) {
        engine().precache_sound(c"misc/beep2.wav");
    }
}

/// The held `+use` of a bomb or hack: `(left, last)` after this call. The first call starts the
/// clock; a call within half a second of the last one takes the gap off the time left (the first
/// such call sets `full` instead); a later one leaves it.
pub fn held_use(left: f32, last: f32, now: f32, full: f32) -> (f32, f32) {
    let last = if last == 0.0 { now } else { last };
    let gap = now - last;
    let left = if gap < 0.5 { if left == 0.0 { full } else { left - gap } } else { left };
    (left, last)
}

impl Bomb {
    /// `CTSBomb::EnableObjective` (0x8c608): shown with the round's time left and new wires, or
    /// hidden; either way not hurried.
    fn enable(&mut self, cx: &mut Cx, on: bool) {
        self.enabled = on;
        self.hurried = false;
        show_brush(cx, on);
        self.countdown = if on { round_left(cx.time()) } else { 0.0 };
        (self.next_beep, self.last_use, self.defuse_left) = (0.0, 0.0, 0.0);
        if on {
            self.wires = random_wires(|lo, hi| engine().random_long(lo, hi));
        }
    }

    /// `CTSBomb::CutWire(n, player)` (0x8c70c): with `cutwires`, while enabled and counting.
    /// The detonating wire ends the countdown, the hurrying one speeds it up and shows the wires
    /// to `player`, the defusing one completes the bomb.
    fn cut_wire(&mut self, cx: &mut Cx, n: i32, p: &mut Player) {
        let counting = !ran_out(rules().nan_time_runs_out, self.countdown);
        if !self.enabled || self.can_cut_wire == 0 || !counting || !(0..3).contains(&n) {
            return;
        }
        let i = n as usize;
        match self.wires[i] {
            3 => {
                self.wires[i] = 0;
                self.countdown = 0.0;
            }
            2 => {
                self.hurried = true;
                self.wires[i] = 0;
                p.ts.wires = self.wires;
            }
            1 => {
                self.wires[i] = 0;
                complete(cx);
                (self.countdown, self.next_beep) = (0.0, 0.0);
                freeze(cx.edict, &mut self.enabled, true);
            }
            _ => {}
        }
    }
}

/// `SetRandomWires` (0x8c598): all detonating, one random wire defusing and another hurrying.
pub fn random_wires(mut random_long: impl FnMut(i32, i32) -> i32) -> [u8; 3] {
    let mut w = [3u8; 3];
    w[random_long(0, 2) as usize] = 1;
    loop {
        let i = random_long(0, 2) as usize;
        if w[i] != 1 {
            w[i] = 2;
            return w;
        }
    }
}

// ---- ts_hack ------------------------------------------------------------------------------------

/// `ts_hack` (`CTSHack`): a brush shown while enabled; `+use` held for `hackingtime` (1..30 s)
/// hacks it, before the round's time left runs out (which freezes it).
#[derive(Default)]
pub struct Hack {
    enabled: bool,
    attacker_team: i32,
    /// mp.dll `+0x5c`: the round's time left; `+0x60`: hacking seconds left; `+0x64`: the last
    /// `Use`; `+0x68`: the next hacking sound.
    timer: f32,
    hack_left: f32,
    last_use: f32,
    next_sound: f32,
    /// `hacksec` (`+0x7c`): `hackingtime`.
    hack_sec: i32,
    /// `Spawn` (0x8cccc) sets `m_pfnThink` (`WaitThink`).
    funcs: BaseFuncs,
}

impl Entity for Hack {
    /// `CTSHack::KeyValue` (0x8d1c8), then `CTSObjective::KeyValue`.
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"hackingtime" => self.hack_sec = kv_i(value),
            _ => return objective_key(&mut self.attacker_team, key, value),
        }
        true
    }

    /// `CTSHack::Spawn` (0x8cccc).
    fn spawn(&mut self, cx: &mut Cx) {
        spawn_brush(cx);
        self.enabled = false;
        self.funcs.think = true;
        (self.timer, self.last_use, self.hack_left) = (0.0, 0.0, 0.0);
    }

    /// `CTSHack::WaitThink` (mp.dll 0x1005cd10): once a second of `ltime`, frozen when the time is
    /// up.
    fn think(&mut self, cx: &mut Cx) {
        next_ltime(cx, 1.0);
        if self.enabled {
            self.timer = (f64::from(self.timer) - 1.0) as f32;
            if ran_out(rules().nan_time_runs_out, self.timer) {
                freeze(cx.edict, &mut self.enabled, true);
            }
        }
    }

    /// `CTSHack::Use` (mp.dll 0x1005cd50).
    fn use_(&mut self, cx: &mut Cx, activator: *mut edict_t, _c: *mut edict_t, _k: UseType, _v: f32) {
        if !self.enabled || !has_player(activator) || !may_use(self.attacker_team, activator) {
            return;
        }
        let now = cx.time();
        (self.hack_left, self.last_use) = held_use(self.hack_left, self.last_use, now, hack_time(self.hack_sec));
        if self.next_sound < now {
            self.next_sound = shared::pm::plus_double(now, 1.0);
            objective_sound(cx.edict, c"misc/hacking.wav");
        }
        print_seconds_left(activator, self.hack_left);
        self.last_use = now;
        if self.hack_left < 0.0 {
            complete(cx);
            self.timer = 0.0;
            freeze(cx.edict, &mut self.enabled, true);
            objective_sound(cx.edict, c"misc/hacked.wav");
        }
    }

    /// `CTSHack::Save` / `Restore`: `"CTSObjective"` then `"CTSHack"`.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut Fields) {
        super::save_base(cx, f, &mut self.funcs);
        f.table(&tables::CTSObjective);
        f.int(&mut self.attacker_team);
        f.table(&tables::CTSHack);
        f.int(&mut self.hack_sec);
    }

    // `CTSHack::Precache` (0x8cdb4) is empty; `TSPrecache` has its sounds.
}

impl Hack {
    /// `CTSHack::EnableObjective` (0x8cdb8).
    fn enable(&mut self, cx: &mut Cx, on: bool) {
        self.enabled = on;
        show_brush(cx, on);
        self.timer = if on { round_left(cx.time()) } else { 0.0 };
        (self.last_use, self.hack_left, self.next_sound) = (0.0, 0.0, 0.0);
    }
}

// ---- ts_teamescape ------------------------------------------------------------------------------

/// `ts_teamescape` (`CTSTeamEscape`): a trigger brush; once enough of its attackers' team touch it
/// within the same second (`escapepercentage`), it completes.
pub struct TeamEscape {
    enabled: bool,
    attacker_team: i32,
    /// `+0x6c`: the players touching it this second, by index (33 slots); `+0xf0`: their count.
    touching: [bool; 33],
    count: i32,
    /// `escape_percentage` (`+0xf4`): `escapepercentage`.
    escape_percentage: i32,
    /// `Spawn` (0x8d390) sets `m_pfnThink` (`WaitThink`) and `m_pfnTouch` (`WaitTouch`).
    funcs: BaseFuncs,
}

impl Default for TeamEscape {
    fn default() -> Self {
        TeamEscape { enabled: false, attacker_team: 0, touching: [false; 33], count: 0, escape_percentage: 0, funcs: BaseFuncs::default() }
    }
}

impl Entity for TeamEscape {
    /// `CTSTeamEscape::KeyValue` (0x8d700), then `CTSObjective::KeyValue`.
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"escapepercentage" => self.escape_percentage = kv_i(value),
            _ => return objective_key(&mut self.attacker_team, key, value),
        }
        true
    }

    /// `CTSTeamEscape::Spawn` (0x8d390): a hidden trigger (`SOLID_TRIGGER`, `MOVETYPE_FLY`).
    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        let m = cx.pev().model;
        if m != 0 {
            eng.set_model(cx.edict, eng.string(m));
        }
        let (mins, maxs, o) = (cx.pev().mins, cx.pev().maxs, cx.pev().origin);
        eng.set_size(cx.edict, mins, maxs);
        eng.set_origin(cx.edict, o);
        let v = cx.pev();
        v.effects = EF_NODRAW as i32;
        v.solid = SOLID_TRIGGER as i32;
        v.flags = 0;
        v.movetype = MOVETYPE_FLY as i32;
        self.enabled = false;
        self.funcs.think = true;
        self.funcs.touch = true;
        cx.think_in(1.0);
        self.clear();
    }

    /// `CTSTeamEscape::WaitThink` (0x8d6b0): the head count starts again every second.
    fn think(&mut self, cx: &mut Cx) {
        cx.think_in(1.0);
        self.clear();
    }

    /// `CTSTeamEscape::WaitTouch` (0x8d4ec).
    fn touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        if !self.enabled || !has_player(other) || !may_use(self.attacker_team, other) {
            return;
        }
        let living = attackers_alive(self.attacker_team);
        if let Some(t) = usize::try_from(engine().index_of(other)).ok().and_then(|i| self.touching.get_mut(i)) {
            if !*t {
                self.count += 1;
            }
            *t = true;
        }
        if escape_needed(self.escape_percentage, living) <= self.count {
            complete(cx);
            freeze(cx.edict, &mut self.enabled, true);
        }
    }

    /// `CTSTeamEscape::Save` / `Restore`: `"CTSObjective"` then `"CTSTeamEscape"`.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut Fields) {
        super::save_base(cx, f, &mut self.funcs);
        f.table(&tables::CTSObjective);
        f.int(&mut self.attacker_team);
        f.table(&tables::CTSTeamEscape);
        f.int(&mut self.escape_percentage);
    }

    // `CTSTeamEscape::Precache` (0x8d49c) is empty.
}

impl TeamEscape {
    fn clear(&mut self) {
        self.touching = [false; 33];
        self.count = 0;
    }

    /// `CTSTeamEscape::EnableObjective` (0x8d4a0): a trigger while enabled, else not solid.
    fn enable(&mut self, cx: &mut Cx, on: bool) {
        self.enabled = on;
        self.clear();
        cx.pev().solid = if on { SOLID_TRIGGER as i32 } else { SOLID_NOT as i32 };
    }
}

// ---- wires --------------------------------------------------------------------------------------

/// `CBasePlayer::CheckForWires` (0x92f08; 2.1 FUN_10055650), from `PreThink` twice a second in the
/// objective mode (3.0 and 2.1 alike: 2.1's `PreThink` 0x100537e0 calls it on the same timer,
/// issue #241): the player sees the wires of the enabled bombs within 64 units it may use (the
/// last one found), or none.
pub fn check_for_wires(p: &mut Player, cx: &mut Cx) {
    let now = cx.time();
    let due = p.ts.next_wire_check < now;
    if state::with(|g| g.mode) != state::Mode::Objective || !due {
        return;
    }
    p.ts.next_wire_check = now + 0.5;
    p.ts.wires = [0; 3];
    let me = cx.edict;
    let origin = cx.pev().origin;
    let mut e = std::ptr::null_mut();
    loop {
        e = engine().find_in_sphere(e, origin, 64.0);
        if e.is_null() {
            break;
        }
        if engine().string(e.v().classname) != c"ts_bomb" {
            continue;
        }
        let seen = world::with::<Bomb, _>(e, |b| (b.enabled, b.attacker_team, b.wires));
        if let Some((true, team, wires)) = seen
            && may_use(team, me)
        {
            p.ts.wires = wires;
        }
    }
}

/// The `cutwire_<n>` client command (`ClientCommand` 0xad838 → `CBasePlayer::CutWire` 0x92fb4):
/// a living player who isn't observing cuts wire `n` (`sscanf("%i")`) of every `ts_bomb` within 64
/// units; the HUD's wires are cleared first.
pub fn cut_wire_command(p: &mut Player, cx: &mut Cx, arg: &str) {
    // TS leaves `n` uninitialised when nothing parses; the port cuts nothing then.
    let Some(n) = scan_i(arg) else { return };
    if !alive(cx.pev()) || cx.pev().iuser1 != 0 {
        return;
    }
    p.ts.wires = [0; 3];
    let origin = cx.pev().origin;
    let mut e = std::ptr::null_mut();
    loop {
        e = engine().find_in_sphere(e, origin, 64.0);
        if e.is_null() {
            break;
        }
        if engine().string(e.v().classname) == c"ts_bomb" {
            world::dispatch(e, |ent, bcx| {
                if let Some(b) = (ent as &mut dyn Any).downcast_mut::<Bomb>() {
                    b.cut_wire(bcx, n, p);
                }
            });
        }
    }
}

/// `sscanf(s, "%i", &n)`: leading spaces, a sign, then decimal, `0x` hex or `0` octal digits.
pub fn scan_i(s: &str) -> Option<i32> {
    let s = s.trim_start();
    let (neg, s) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let (radix, digits) = if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        (16, h)
    } else if s.starts_with('0') {
        (8, s)
    } else {
        (10, s)
    };
    let end = digits.find(|c: char| !c.is_digit(radix)).unwrap_or(digits.len());
    if end == 0 {
        // "0x" with no hex digit still reads the 0.
        return (radix == 16).then_some(0);
    }
    let v = i64::from_str_radix(&digits[..end], radix).ok()?;
    let v = if neg { -v } else { v };
    Some(v as i32)
}

/// `UpdateClientData`'s `Wires` (0x91d00): the player's wires when they changed.
pub fn send_wires(p: &mut Player, e: *mut edict_t) {
    if p.ts.wires != p.ts.wires_sent {
        p.ts.wires_sent = p.ts.wires;
        let [w0, w1, w2] = p.ts.wires;
        msg::one(Msg::Wires, e, |w| Wires { w0, w1, w2 }.write(w));
    }
}

// ---- the round rules' part ----------------------------------------------------------------------

/// `InitObjectives`' manager search (0x785d8): up to 32 `ts_objective_manager`s, which make the
/// round mode 3. Store them in [`Objectives::managers`].
pub fn find_managers() -> Vec<*mut edict_t> {
    let mut found = Vec::new();
    let mut e = std::ptr::null_mut();
    loop {
        e = engine().find_by_string(e, c"classname", c"ts_objective_manager");
        if e.is_null() || engine().index_of(e) == 0 {
            break;
        }
        if found.len() < MAX_LISTED {
            found.push(e);
        }
    }
    found
}

/// `InitObjectives` in the objective mode: `EnableObjective(false)` on every objective (the win
/// and score givers start counting again).
pub fn disable_all() {
    let eng = engine();
    for i in 1..eng.max_entities() {
        let e = eng.entity_of_index(i);
        if e.is_null() || e.is_free() || !crate::ham::is_objective(e, || world::type_of(e).is_some_and(|t| is_objective(t, true))) {
            continue;
        }
        crate::ham::enable_objective(e, false);
    }
}

/// `SetObjective` (0x790b4): the list cleared (only in the round modes, `round`, unless
/// [`ObjectiveRules::clear_list_outside_rounds`]); in the round modes a random one of
/// [`Objectives::managers`] (`RandomLong(0, n - 1)`) runs its `Reset` and becomes the round's.
pub fn set_objective(round: bool) {
    if !round && !rules().clear_list_outside_rounds {
        return;
    }
    state::with(|g| g.objectives.clear_list());
    if !round {
        return;
    }
    let managers = state::with(|g| {
        g.objectives.manager = std::ptr::null_mut();
        g.objectives.managers.clone()
    });
    if managers.is_empty() {
        return;
    }
    let m = managers[engine().random_long(0, managers.len() as i32 - 1) as usize];
    dev_objective(&format!("the round's manager is #{} of {}", engine().index_of(m), managers.len()));
    world::dispatch(m, |ent, cx| {
        if let Some(mgr) = (ent as &mut dyn Any).downcast_mut::<ObjectiveManager>() {
            mgr.reset(cx);
        }
    });
    state::with(|g| g.objectives.manager = m);
}

/// Fires the round's manager's `which` target (`USE_TOGGLE`, the manager as activator and caller).
/// False when there is no manager or it has no such target.
pub fn fire_manager_target(which: ManagerTarget) -> bool {
    let m = state::with(|g| g.objectives.manager);
    if m.is_null() {
        return false;
    }
    let Some(t) = world::with::<ObjectiveManager, _>(m, |mgr| mgr.target(which)) else { return false };
    if t == 0 {
        return false;
    }
    dev_objective(&format!("the round's manager fires {:?} ({which:?})", engine().string(t)));
    crate::entities::fire_targets(engine().string(t), m, m, UseType::Toggle, 0.0);
    true
}

/// Port addition: with `developer` on, an `[ts] objective: ...` console line for the round
/// manager, for scripted tests (`cargo xtask`).
fn dev_objective(text: &str) {
    if crate::rules::cvars::on(c"developer") {
        engine().print(&format!("[ts] objective: {text}\n"));
    }
}

/// Whether there is a round manager (2.1's `CheckDeads` fires its targets with no test of the
/// string).
pub fn has_manager() -> bool {
    !state::with(|g| g.objectives.manager).is_null()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ent(i: usize) -> *mut edict_t {
        (0x1000 + i * 0x10) as *mut edict_t
    }

    #[test]
    fn indexed_list_keeps_an_index_and_its_first_texts() {
        let mut o = Objectives::default();
        assert_eq!(o.add(ent(1), [1, 2, 3, 4], true), 0);
        assert_eq!(o.add(ent(2), [5, 0, 0, 0], true), 1);
        assert_eq!(o.add(ent(1), [9, 9, 9, 9], true), 0, "already listed");
        assert_eq!(o.texts[0], [1, 2, 3, 4]);
        assert_eq!(o.count, 2);
        for i in 3..=32 {
            assert_eq!(o.add(ent(i), [0; 4], true), (i - 1) as i32);
        }
        assert_eq!(o.add(ent(40), [0; 4], true), -1, "full at 32");
        assert_eq!(o.find(ent(32)), 31);
        o.clear_list();
        assert_eq!((o.count, o.find(ent(1))), (0, -1));
    }

    #[test]
    fn appending_list_and_its_removal() {
        let mut o = Objectives::default();
        assert_eq!(o.add(ent(1), [0; 4], false), 0);
        assert_eq!(o.add(ent(1), [0; 4], false), 1, "appended again");
        assert_eq!(o.remove_appended(ent(1)), 0, "the first slot");
        assert_eq!(o.count, 2);
        assert_eq!(o.remove_appended(ent(7)), -1);
        assert_eq!(o.count, 0, "slot -1 is the count");
        assert_eq!(o.add(ent(3), [0; 4], false), 0, "the list starts over");
    }

    #[test]
    fn interaction_by_team_index_or_name() {
        assert!(can_interact(-1, b"", -1, None), "3.0: -1 is anyone");
        assert!(can_interact(2, b"x", 2, None) && !can_interact(2, b"x", 1, None));
        let names = Some(["Specialists", "Mercenary"]);
        assert!(can_interact(0, b"specialists", 5, names));
        assert!(can_interact(1, b"Mercenary", 0, names) && !can_interact(1, b"Specialists", 1, names));
        assert!(!can_interact(-1, b"Specialists", 0, names), "no 'anyone' by name");
        assert!(!can_interact(2, b"Mercenary", 2, names));
    }

    #[test]
    fn team_texts() {
        let t = [11, 12, 13, 14];
        assert_eq!(team_text(&t, b"", -1, None), 0, "no team, no text");
        assert_eq!(team_text(&t, b"a", 2, None), 13);
        assert_eq!(team_text(&t, b"a", 7, None), 0);
        let names = Some(["Specialists", "Mercenary"]);
        assert_eq!(team_text(&t, b"SPECIALISTS", 3, names), 11);
        assert_eq!(team_text(&t, b"", -1, names), 12, "anyone else reads the second");
    }

    #[test]
    fn escape_shares() {
        assert_eq!(escape_needed(0, 9), 1);
        assert_eq!(escape_needed(1, 8), 2);
        assert_eq!(escape_needed(2, 8), 4, "two thirds of the thirds");
        assert_eq!(escape_needed(3, 8), 8);
        assert_eq!(escape_needed(4, 8), 0);
    }

    #[test]
    fn hacking_time_is_clamped() {
        assert_eq!(hack_time(0), 1.0);
        assert_eq!(hack_time(1), 1.0);
        assert_eq!(hack_time(10), 10.0);
        assert_eq!(hack_time(30), 30.0);
        assert_eq!(hack_time(99), 30.0);
    }

    #[test]
    fn held_use_counts_only_continuous_holds() {
        // First use: the clock starts and the full time is taken.
        assert_eq!(held_use(0.0, 0.0, 10.0, 5.0), (5.0, 10.0));
        // Held: the frame's gap comes off.
        assert_eq!(held_use(5.0, 10.0, 10.25, 5.0), (4.75, 10.0));
        // Let go for half a second or more: the time left stays.
        assert_eq!(held_use(4.75, 10.0, 11.0, 5.0), (4.75, 10.0));
    }

    #[test]
    fn timers_run_out_per_ruleset() {
        assert!(ran_out(true, 0.0) && ran_out(true, -1.0) && ran_out(true, f32::NAN));
        assert!(!ran_out(true, 0.5));
        assert!(ran_out(false, 0.0) && !ran_out(false, f32::NAN), "a NaN keeps going");
    }

    #[test]
    fn wires_are_one_defuse_one_hurry() {
        let mut draws = [1, 1, 2].into_iter();
        assert_eq!(random_wires(|_, _| draws.next().unwrap()), [3, 1, 2], "a draw on the defuse wire is drawn again");
        let mut draws = [0, 2].into_iter();
        assert_eq!(random_wires(|_, _| draws.next().unwrap()), [1, 3, 2]);
    }

    #[test]
    fn scan_i_is_sscanf_percent_i() {
        assert_eq!(scan_i("2"), Some(2));
        assert_eq!(scan_i(" -1x"), Some(-1));
        assert_eq!(scan_i("0x2"), Some(2));
        assert_eq!(scan_i("010"), Some(8));
        assert_eq!(scan_i("08"), Some(0), "octal stops at the 8");
        assert_eq!(scan_i("x"), None);
        assert_eq!(scan_i(""), None);
    }
}
