//! Capture the briefcase (gamerules-entities.md §3.4): `ts_briefcase` (home + capture zone),
//! `ts_flag` (the carried case), and `ts_scoregiver`/`ts_wingiver` (team score / win, §3.3).

use crate::messages::Msg;
use crate::player::Player;
use crate::rules::{self, msg, state, teams};
use crate::world::{Cx, Entity, UseType};
use goldsrc::Global;
use goldsrc::edict::EdictPtr;
use goldsrc::server::engine;
use goldsrc::sys::*;
use shared::protocol::rules::{ScoreInfo, TsFlag, TsHVoice};
use std::cell::Cell;
use std::ffi::CStr;

pub const MODEL: &CStr = c"models/briefcase.mdl";
const FLAG_SLOTS: i32 = 20;

static NEXT_FLAG_ID: Global<Cell<i32>> = Global(Cell::new(0));

/// Glow colour per owner team.
fn team_color(team: i32) -> [f32; 3] {
    match team {
        0 => [125.0, 165.0, 210.0],
        1 => [200.0, 90.0, 70.0],
        2 => [225.0, 205.0, 45.0],
        3 => [145.0, 215.0, 140.0],
        _ => [255.0, 255.0, 255.0],
    }
}

fn send_flag(state_: u8, id: i32, team: i32, holder: i32) {
    let m = TsFlag { state: state_, id: id as u8, team: (team + 1) as u8, holder: holder as u8 };
    msg::all(Msg::TsFlag, |w| m.write(w));
}

fn team_index_of(e: *mut edict_t) -> i32 {
    let t = teams::team_name_of(e);
    state::with(|g| g.team_index(&t))
}

fn score_info(e: *mut edict_t) {
    let idx = engine().index_of(e);
    if let Some(s) =
        rules::other(e, |p| ScoreInfo { idx: idx as u8, frags: e.v().frags as i16, deaths: p.deaths as i16, class: 0, team: p.team as i16 })
    {
        msg::all(Msg::ScoreInfo, |w| s.write(w));
    }
}

// ---- ts_briefcase ----------------------------------------------------------------------------------

pub struct Briefcase {
    pub id: i32,
    /// 0-based team that may capture here (-1 = any).
    owner_team: i32,
    contents: String,
    active: bool,
    /// Pending `FlagFall` (create the case at home).
    fall_at: f32,
}

impl Default for Briefcase {
    fn default() -> Self {
        let mut id = NEXT_FLAG_ID.0.get() + 1;
        if id > 15 {
            id = 1;
        }
        NEXT_FLAG_ID.0.set(id);
        Self { id, owner_team: -1, contents: String::new(), active: false, fall_at: 0.0 }
    }
}

impl Briefcase {
    /// Schedule the case to reappear at home.
    pub fn respawn_flag(&mut self, delay: f32) {
        self.fall_at = engine().time() + delay;
    }
}

impl Entity for Briefcase {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"contents" => self.contents = value.to_string_lossy().chars().take(31).collect(),
            b"ownerteam" => self.owner_team = value.to_str().unwrap_or("").trim().parse::<i32>().unwrap_or(0) - 1,
            _ => return false,
        }
        true
    }

    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        let e = cx.edict;
        cx.pev().classname = eng.alloc_string(c"ts_briefcase");
        let m = cx.pev().model;
        if m != 0 {
            eng.set_model(e, eng.string(m));
        }
        let v = cx.pev();
        let (mins, maxs, org) = (v.mins, v.maxs, v.origin);
        v.effects = EF_NODRAW as i32;
        v.solid = SOLID_TRIGGER as i32;
        v.flags = 0;
        v.movetype = MOVETYPE_FLY as i32;
        eng.set_size(e, mins, maxs);
        eng.set_origin(e, org);
        if self.contents.is_empty() {
            self.contents = "a flag".into();
        }
        self.active = false;
        self.fall_at = eng.time() + 1.0;
        cx.think_in(0.1);
    }

    fn think(&mut self, cx: &mut Cx) {
        cx.think_in(0.25);
        if self.fall_at == 0.0 || cx.time() < self.fall_at {
            return;
        }
        self.fall_at = 0.0;
        // FlagFall: the case appears at the centre of the zone.
        let v = cx.pev();
        let center = [(v.absmin[0] + v.absmax[0]) * 0.5, (v.absmin[1] + v.absmax[1]) * 0.5, (v.absmin[2] + v.absmax[2]) * 0.5];
        create_flag(center, self.id, self.owner_team, &self.contents, cx.edict);
        self.active = true;
        send_flag(0, self.id, self.owner_team, 0);
    }

    fn touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        if !self.active || !rules::is_player(other) || !rules::alive(other.v()) || !has_flag(other) {
            return;
        }
        if self.owner_team != -1 && self.owner_team != team_index_of(other) {
            return;
        }
        capture(cx, other);
    }
}

/// `CaptureTouch` (0x8db9c).
fn capture(cx: &mut Cx, player: *mut edict_t) {
    let eng = engine();
    let name = rules::netname(player);
    msg::log(&format!(
        "\"{name}<{}><{}><>\" triggered \"captured_briefcase\"\n",
        eng.user_id(player),
        eng.auth_id(player).to_string_lossy()
    ));
    crate::player::with_player(player, |p, pcx| drop_flag(p, pcx, true));
    let (target, message) = (cx.pev().target, cx.pev().message);
    if target != 0 {
        crate::entities::fire_targets(eng.string(target), cx.edict, cx.edict, UseType::Toggle, 0.0);
    }
    if message != 0 {
        msg::ts_message_all([0, 255, 0], 8, 30, &eng.string(message).to_string_lossy());
    }
    let n = rules::players().len();
    if n < 2 {
        state::with(|g| {
            for t in &mut g.teams {
                t.score = 0;
            }
        });
        msg::ts_message_all([0, 0, 0], 8, 30, "Waiting for other players.");
    } else {
        rules::add_frags(player, None, 10.0);
        msg::ts_message_all([0, 255, 0], 4, 30, &format!("{name} captured the briefcase! 10 #TS_Frg"));
        score_info(player);
        let team = team_index_of(player);
        if team >= 0 {
            state::with(|g| g.teams[team as usize].score += 1);
        }
        for o in rules::players() {
            let lost = (team != team_index_of(o)) as u8;
            msg::one(Msg::TsHVoice, o, |w| TsHVoice { lost }.write(w));
        }
    }
    teams::send_team_scores();
}

// ---- ts_flag -------------------------------------------------------------------------------------

#[derive(Default)]
pub struct Flag {
    pub id: i32,
    pub team: i32,
    pub contents: String,
    pub home: Option<*mut edict_t>,
    pub holder: Option<*mut edict_t>,
    /// Dropped: return home at this time.
    return_at: f32,
}

impl Flag {
    fn go_home(&mut self, cx: &mut Cx) {
        if let Some(home) = self.home {
            crate::world::dispatch(home, |ent, _| {
                if let Some(b) = (ent as &mut dyn std::any::Any).downcast_mut::<Briefcase>() {
                    b.respawn_flag(0.25);
                }
            });
        }
        crate::entities::remove(cx.edict);
    }
}

impl Entity for Flag {
    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        let e = cx.edict;
        cx.pev().classname = eng.alloc_string(c"ts_flag");
        eng.set_model(e, MODEL);
        let v = cx.pev();
        v.movetype = MOVETYPE_BOUNCE as i32;
        v.solid = SOLID_TRIGGER as i32;
        v.gravity = 0.7;
        v.friction = 0.8;
        v.renderfx = kRenderFxGlowShell as i32;
        v.renderamt = 5.0;
        let org = v.origin;
        eng.set_size(e, [-8.0, -8.0, 0.0], [8.0, 8.0, 16.0]);
        eng.set_origin(e, org);
        cx.think_in(0.5);
    }

    fn think(&mut self, cx: &mut Cx) {
        cx.think_in(0.5);
        if self.holder.is_none() && self.return_at != 0.0 && cx.time() > self.return_at {
            msg::ts_message_all([0, 255, 0], 8, 30, &format!("{} briefcase returned", self.contents));
            send_flag(0, self.id, self.team, 0);
            self.go_home(cx);
        }
    }

    fn touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        if self.holder.is_some() || !rules::is_player(other) || !rules::alive(other.v()) {
            return;
        }
        let team = team_index_of(other);
        if self.team == -1 || team != self.team {
            // TakeFlag: needs 20 free slots.
            let me = cx.edict;
            let took = crate::player::with_player(other, |p, _| {
                if p.ts.flag.is_some() || shared::ts::SLOTS_TOTAL - p.slots_used < FLAG_SLOTS {
                    return false;
                }
                p.slots_used += FLAG_SLOTS;
                p.ts.flag = Some(me);
                true
            })
            .unwrap_or(false);
            if !took {
                return;
            }
            self.holder = Some(other);
            self.return_at = 0.0;
            let v = cx.pev();
            v.effects |= EF_NODRAW as i32;
            v.solid = SOLID_NOT as i32;
            v.movetype = MOVETYPE_FOLLOW as i32;
            v.aiment = other;
            let name = rules::netname(other);
            msg::ts_message_all([0, 255, 0], 8, 30, &format!("{name} picked up a briefcase containing {}!", self.contents));
            send_flag(1, self.id, self.team, engine().index_of(other));
            let eng = engine();
            msg::log(&format!(
                "\"{name}<{}><{}><>\" triggered \"aquired_briefcase\"\n",
                eng.user_id(other),
                eng.auth_id(other).to_string_lossy()
            ));
        } else if self.return_at != 0.0 {
            // The owning team returns its dropped case.
            rules::add_frags(other, None, 5.0);
            score_info(other);
            msg::ts_message_all([0, 255, 0], 8, 30, &format!("{} returned the briefcase!", rules::netname(other)));
            send_flag(0, self.id, self.team, 0);
            self.go_home(cx);
        }
    }
}

fn create_flag(origin: [f32; 3], id: i32, team: i32, contents: &str, home: *mut edict_t) {
    let eng = engine();
    let e = eng.create_named_entity(c"ts_flag");
    if e.is_null() {
        return;
    }
    (e.v().origin = origin);
    let contents = contents.to_owned();
    crate::world::dispatch(e, |ent, cx| {
        if let Some(f) = (ent as &mut dyn std::any::Any).downcast_mut::<Flag>() {
            f.id = id;
            f.team = team;
            f.contents = contents;
            f.home = Some(home);
        }
        ent.spawn(cx);
        cx.pev().rendercolor = team_color(team);
    });
}

/// True if the player carries a case.
pub fn has_flag(e: *mut edict_t) -> bool {
    rules::other(e, |p| p.ts.flag.is_some()).unwrap_or(false)
}

/// `CBasePlayer::DropFlag(captured)`: a captured case is removed (home recreates it); otherwise
/// it drops at the player's feet and returns home after 20 s.
pub fn drop_flag(p: &mut Player, cx: &mut Cx, captured: bool) {
    let Some(flag) = p.ts.flag.take() else { return };
    p.slots_used = (p.slots_used - FLAG_SLOTS).max(0);
    let (org, vel) = (cx.pev().origin, cx.pev().velocity);
    let holder = cx.index();
    let name = rules::netname(cx.edict);
    crate::world::dispatch(flag, |ent, fcx| {
        let Some(f) = (ent as &mut dyn std::any::Any).downcast_mut::<Flag>() else { return };
        f.holder = None;
        if captured {
            send_flag(0, f.id, f.team, 0);
            f.go_home(fcx);
            return;
        }
        f.return_at = engine().time() + 20.0;
        let v = fcx.pev();
        v.aiment = std::ptr::null_mut();
        v.movetype = MOVETYPE_BOUNCE as i32;
        v.solid = SOLID_TRIGGER as i32;
        v.effects &= !(EF_NODRAW as i32);
        v.velocity = vel;
        engine().set_origin(fcx.edict, org);
        send_flag(2, f.id, f.team, holder);
        msg::log(&format!("\"{name}\" triggered \"dropped_briefcase\"\n"));
    });
}

// ---- ts_wingiver / ts_scoregiver --------------------------------------------------------------------

/// `CTSWinGiver` / `CTSScoreGiver`: after `callsnumber` uses, the `winnerteam` index wins a round
/// (latched) or scores (scoregiver).
#[derive(Default)]
pub struct WinGiver<const SCORE: bool> {
    winner: i32,
    calls_needed: i32,
    calls: i32,
    used: bool,
}

impl<const SCORE: bool> Entity for WinGiver<SCORE> {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        let v: i32 = value.to_str().unwrap_or("").trim().parse().unwrap_or(0);
        match key.to_bytes() {
            b"winnerteam" => self.winner = v,
            b"callsnumber" => self.calls_needed = v,
            b"victorycash" | b"defeatcash" => {}
            _ => return false,
        }
        true
    }

    fn spawn(&mut self, cx: &mut Cx) {
        let v = cx.pev();
        v.effects |= EF_NODRAW as i32;
        v.solid = SOLID_NOT as i32;
    }

    fn use_(&mut self, cx: &mut Cx, _a: *mut edict_t, _c: *mut edict_t, _k: UseType, _v: f32) {
        if self.used && !SCORE {
            return;
        }
        self.calls += 1;
        if self.calls < self.calls_needed {
            return;
        }
        self.calls = 0;
        if !SCORE {
            self.used = true;
        }
        team_win(self.winner, SCORE);
        let m = cx.pev().message;
        if m != 0 {
            msg::ts_message_all([0, 255, 0], 8, 30, &engine().string(m).to_string_lossy());
        }
        if SCORE && cx.pev().spawnflags & 1 != 0 {
            crate::rules::frame::restart_round(engine().time());
        }
    }
}

/// `CTSGameRules::TeamWin` (score part): team score, announcement, `TSHVoice`, `TeamScore`.
fn team_win(team: i32, is_score: bool) {
    if rules::players().len() < 2 {
        state::with(|g| {
            for t in &mut g.teams {
                t.score = 0;
            }
        });
        msg::ts_message_all([0, 255, 0], 8, 30, "Waiting for other players.");
        return;
    }
    if team == 2 {
        msg::ts_message_all([0, 255, 0], 8, 30, "Game Draw");
    } else {
        let name = state::with(|g| {
            let t = g.teams.get_mut(team.max(0) as usize)?;
            t.score += 1;
            Some(t.name.clone())
        });
        if let Some(name) = name {
            msg::ts_message_all([0, 255, 0], 8, 30, &format!("{name} {}", if is_score { "Score!" } else { "Win!" }));
        }
    }
    for o in rules::players() {
        let lost = (team != team_index_of(o)) as u8;
        msg::one(Msg::TsHVoice, o, |w| TsHVoice { lost }.write(w));
    }
    teams::send_team_scores();
}
