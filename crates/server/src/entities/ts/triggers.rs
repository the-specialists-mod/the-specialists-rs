//! TS trigger/logic entities: `ts_trigger`, `ts_dmhill`, `ts_timer`, `ts_customtarget`,
//! `ts_playeramount`, `ts_slowmotion`, `ts_slowmotionpoint` (gamerules-entities.md §5,
//! player-movement.md §5.4–5.5).

use crate::messages::Msg;
use crate::rules::{self, msg, state, teams};
use crate::save::{BaseFuncs, Fields};
use crate::world::{Cx, Entity, UseType};
use goldsrc::edict::EdictPtr;
use goldsrc::server::{IGNORE_MONSTERS, engine};
use goldsrc::sys::*;
use goldsrc::util::to_cstring;
use shared::protocol::rules::ScoreInfo;
use std::ffi::CStr;

fn fire(target: &str, cx: &mut Cx) {
    if !target.is_empty() {
        let t = to_cstring(target);
        crate::entities::fire_targets(&t, cx.edict, cx.edict, UseType::Toggle, 0.0);
    }
}

fn fire_own_target(cx: &mut Cx) {
    let t = cx.pev().target;
    if t != 0 {
        let s = engine().string(t);
        crate::entities::fire_targets(s, cx.edict, cx.edict, UseType::Toggle, 0.0);
    }
}

fn message_of(cx: &mut Cx) -> Option<String> {
    let m = cx.pev().message;
    (m != 0).then(|| engine().string(m).to_string_lossy().into_owned()).filter(|s| !s.is_empty())
}

/// Invisible trigger brush setup shared by the brush entities here.
fn trigger_brush(cx: &mut Cx, classname: &CStr) {
    let eng = engine();
    let e = cx.edict;
    cx.pev().classname = eng.alloc_string(classname);
    let m = cx.pev().model;
    if m != 0 {
        let name = eng.string(m);
        eng.set_model(e, name);
    }
    let v = cx.pev();
    let (mins, maxs, org) = (v.mins, v.maxs, v.origin);
    v.effects |= EF_NODRAW as i32;
    v.solid = SOLID_TRIGGER as i32;
    v.movetype = MOVETYPE_FLY as i32;
    eng.set_size(e, mins, maxs);
    eng.set_origin(e, org);
}

/// `printf` with the player's name for `%s` (TS passes the format straight to `sprintf`).
pub fn format_name(fmt: &str, name: &str) -> String {
    let mut out = String::new();
    let mut used = false;
    let mut it = fmt.chars().peekable();
    while let Some(c) = it.next() {
        if c == '%' {
            match it.peek() {
                Some('s') if !used => {
                    it.next();
                    out.push_str(name);
                    used = true;
                }
                Some('%') => {
                    it.next();
                    out.push('%');
                }
                _ => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

// ---- ts_trigger ----------------------------------------------------------------------------------

/// `CTSTrigger` (0x7086c).
#[derive(Default)]
pub struct Trigger {
    next_check: f32,
    team: i32,
    ready_at: f32,
    /// `m_pfnTouch` (`WaitTouch`), set by `Spawn`.
    funcs: BaseFuncs,
}

/// `max(nextcheck, 0.5)`, the interval of `ts_trigger` and `ts_dmhill`.
fn check_step(next_check: f32) -> f32 {
    if next_check < 0.5 { 0.5 } else { next_check }
}

impl Entity for Trigger {
    /// `CBaseToggle::Save` / `Restore`: TS builds `CTSTrigger` on `CBaseToggle`, but its
    /// `KeyValue` (0x70890) keeps `nextcheck` / the team in its own unsaved members and never
    /// falls back to `CBaseToggle::KeyValue`, so the inherited tables stay empty. `WaitTouch` is
    /// set from `Spawn` on, so TS's restore stops after `"BASE"`.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut Fields) {
        super::save_base(cx, f, &mut self.funcs);
        super::save_unused_toggle(f);
    }

    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        let v: f32 = value.to_str().unwrap_or("").trim().parse().unwrap_or(0.0);
        match key.to_bytes() {
            b"nextcheck" => self.next_check = v,
            b"attackerteam" => self.team = v.round() as i32,
            b"allowedteam" => self.team = if v.round() as i32 == 2 { -1 } else { v.round() as i32 },
            _ => return false,
        }
        true
    }

    /// `CTSTrigger::Spawn` (0x709b8): no think, but `nextthink` is set all the same.
    fn spawn(&mut self, cx: &mut Cx) {
        trigger_brush(cx, c"ts_trigger");
        self.funcs = BaseFuncs { touch: true, ..BaseFuncs::default() };
        let step = check_step(self.next_check);
        cx.think_in(step);
        self.ready_at = cx.time() + step;
    }

    fn touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        let now = cx.time();
        if !self.funcs.touch || now < self.ready_at || !rules::is_player(other) || !rules::alive(other.v()) {
            return;
        }
        let team_ok = self.team == -1 || state::with(|g| g.team_index(&teams::team_name_of(other))) == self.team;
        if cx.pev().spawnflags & 8 != 0 && !super::has_flag(other) {
            return;
        }
        if team_ok {
            fire_own_target(cx);
        }
        if let Some(m) = message_of(cx) {
            msg::ts_message_all([0, 255, 0], 8, 30, &format_name(&m, &rules::netname(other)));
        }
        self.ready_at = now + self.next_check.max(0.5);
    }
}

// ---- ts_dmhill -----------------------------------------------------------------------------------

/// `CTSDMHill` (0x718f4): king of the hill.
#[derive(Default)]
pub struct DmHill {
    next_check: f32,
    frags: i32,
    frags_team: i32,
    blocked_until: f32,
    king: Option<*mut edict_t>,
    last_touch: f32,
    award_at: f32,
    /// `m_pfnTouch` (`WaitTouch`): `Spawn` sets it when the mode's frag amount is positive.
    funcs: BaseFuncs,
}

impl Entity for DmHill {
    /// `CTSDMHill::Save` / `Restore`: `fragseverycheck` (`+0x70`, `fragamt`),
    /// `fragseverycheckteam` (`+0x74`, `fragamtteam`), `nextcheck` (`+0x78`).
    fn save_fields(&mut self, cx: &mut Cx, f: &mut Fields) {
        super::save_base(cx, f, &mut self.funcs);
        f.table(&crate::save::tables::CTSDMHill);
        f.int(&mut self.frags);
        f.int(&mut self.frags_team);
        f.float(&mut self.next_check);
    }

    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        let v = value.to_str().unwrap_or("").trim();
        match key.to_bytes() {
            b"nextcheck" => self.next_check = v.parse().unwrap_or(0.0),
            b"fragamt" => self.frags = v.parse().unwrap_or(0),
            b"fragamtteam" => self.frags_team = v.parse().unwrap_or(0),
            _ => return false,
        }
        true
    }

    fn spawn(&mut self, cx: &mut Cx) {
        trigger_brush(cx, c"ts_dmhill");
        let teamplay = state::teamplay();
        let enabled = if teamplay { self.frags_team > 0 } else { self.frags > 0 };
        self.funcs = BaseFuncs { touch: enabled, ..BaseFuncs::default() };
        // `CTSDMHill::Spawn` (0x71a8c) sets `nextthink` without a think.
        cx.think_in(check_step(self.next_check));
        self.blocked_until = cx.time() + 1.0;
        self.king = None;
    }

    fn touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        let now = cx.time();
        let step = self.next_check.max(0.5);
        if !self.funcs.touch || now < self.blocked_until || !rules::is_player(other) || !rules::alive(other.v()) {
            return;
        }
        let Some(king) = self.king else {
            self.king = Some(other);
            self.blocked_until = 0.0;
            self.award_at = now + step;
            self.last_touch = now + step;
            return;
        };
        let reset = if king == other { now - self.last_touch > step } else { !teams::same_team(king, other) };
        if reset {
            self.king = None;
            self.blocked_until = 0.0;
            self.award_at = now + step;
            return;
        }
        if now <= self.award_at {
            return;
        }
        let teamplay = state::teamplay();
        let mut text = message_of(cx).map(|m| {
            let who = if teamplay { format!("Team {}", teams::team_name_of(other)) } else { rules::netname(other) };
            format!("{who} {m}")
        });
        if !teamplay {
            rules::add_frags(other, None, self.frags as f32);
            send_score(other);
            if let Some(t) = text.as_mut() {
                t.push_str(&format!(" ,{} frags for him", self.frags));
            }
        } else {
            // `UTIL_FindEntityByClassname(.., "player")`, `IsPlayer()` and a case-sensitive
            // `strcmp` of the `TeamID()`s.
            let team = teams::team_name_of(other);
            for p in rules::slot_players() {
                if teams::team_name_of(p) == team {
                    rules::add_frags(p, None, self.frags_team as f32);
                    send_score(p);
                }
            }
            if let Some(t) = text.as_mut() {
                t.push_str(&format!(" ,{} frags for they", self.frags_team));
            }
        }
        if let Some(t) = text {
            let hold = (step * 10.0).min(30.0).round() as u8;
            msg::ts_message_all([0, 255, 0], 8, hold, &t);
        }
        self.award_at = now + step;
        fire_own_target(cx);
        self.last_touch = now;
    }
}

fn send_score(e: *mut edict_t) {
    let idx = engine().index_of(e);
    let s = rules::other(e, |p| ScoreInfo {
        idx: idx as u8,
        frags: rules::lifecycle::score_frags(e.v().frags),
        deaths: p.deaths as i16,
        class: 0,
        team: teams::score_team(idx),
    });
    if let Some(s) = s {
        msg::all(Msg::ScoreInfo, |w| s.write(w));
    }
}

// ---- ts_timer ------------------------------------------------------------------------------------

/// `CTSTimer` (0x72d2c). Without flag 1 it starts and loops. With flag 1 a fire clears the think
/// but not the on/off byte, so after a fire the next Use only switches it off and the one after
/// that runs it again.
#[derive(Default)]
pub struct Timer {
    duration: i32,
    /// `+0x68`: switched by `ToggleTimer`.
    running: bool,
    /// `m_pfnThink` (`WaitThink`).
    funcs: BaseFuncs,
}

impl Timer {
    /// `CTSTimer::ToggleTimer` (0x72e44).
    fn toggle(&mut self, cx: &mut Cx) {
        if self.duration < 1 {
            self.running = false;
            self.funcs.think = false;
            return;
        }
        self.running = !self.running;
        self.funcs.think = self.running;
        if self.running {
            cx.think_in(self.duration as f32);
        }
    }
}

impl Entity for Timer {
    /// `CBaseEntity::Save` / `Restore` (`duration` isn't saved).
    fn save_fields(&mut self, cx: &mut Cx, f: &mut Fields) {
        super::save_base(cx, f, &mut self.funcs);
    }

    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        if key.to_bytes() == b"duration" {
            self.duration = value.to_str().unwrap_or("").trim().parse().unwrap_or(0);
            return true;
        }
        false
    }

    fn spawn(&mut self, cx: &mut Cx) {
        cx.pev().classname = engine().alloc_string(c"ts_timer");
        let v = cx.pev();
        v.effects |= EF_NODRAW as i32;
        v.solid = SOLID_NOT as i32;
        v.movetype = MOVETYPE_FLY as i32;
        self.running = false;
        self.funcs = BaseFuncs::default();
        if cx.pev().spawnflags & 1 == 0 {
            self.toggle(cx);
        }
    }

    /// `CTSTimer::WaitThink` (0x72eb8).
    fn think(&mut self, cx: &mut Cx) {
        if !self.funcs.think {
            return;
        }
        fire_own_target(cx);
        if let Some(m) = message_of(cx) {
            msg::ts_message_all([0, 255, 0], 1, 30, &m);
        }
        if cx.pev().spawnflags & 1 != 0 {
            self.funcs.think = false;
        }
        cx.think_in(self.duration as f32);
    }

    fn use_(&mut self, cx: &mut Cx, _a: *mut edict_t, _c: *mut edict_t, _k: UseType, _v: f32) {
        self.toggle(cx);
    }
}

// ---- ts_customtarget ------------------------------------------------------------------------------

/// `CTSCustomTarget` (0x72f90): fires all / a random / the next of up to 64 targets after
/// `accumulate` uses.
#[derive(Default)]
pub struct CustomTarget {
    targets: Vec<String>,
    method: i32,
    accumulate: i32,
    count: i32,
    next: usize,
}

impl Entity for CustomTarget {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        let v = value.to_str().unwrap_or("");
        match key.to_bytes() {
            b"targets" => {
                self.targets = v.split(';').take_while(|s| !s.is_empty()).take(64).map(|s| s.chars().take(31).collect()).collect();
            }
            b"triggermethod" => self.method = v.trim().parse().unwrap_or(0),
            b"accumulate" => self.accumulate = v.trim().parse().unwrap_or(0),
            _ => return false,
        }
        true
    }

    fn spawn(&mut self, cx: &mut Cx) {
        cx.pev().classname = engine().alloc_string(c"ts_customtarget");
        let v = cx.pev();
        v.effects |= EF_NODRAW as i32;
        v.solid = SOLID_NOT as i32;
        v.movetype = MOVETYPE_FLY as i32;
        self.count = 0;
        self.next = 0;
    }

    fn use_(&mut self, cx: &mut Cx, _a: *mut edict_t, _c: *mut edict_t, _k: UseType, _v: f32) {
        self.count += 1;
        if self.count < self.accumulate {
            return;
        }
        self.count = 0;
        if let Some(m) = message_of(cx) {
            msg::ts_message_all([0, 255, 0], 1, 30, &m);
        }
        let n = self.targets.len();
        let pick: Option<usize> = match self.method {
            1 if n > 0 => Some(engine().random_long(0, n as i32 - 1) as usize),
            2 => {
                let i = self.next;
                self.next += 1;
                if self.next >= n {
                    self.next = 0;
                }
                Some(i)
            }
            _ => None,
        };
        let targets = self.targets.clone();
        match pick {
            Some(i) if i < n => fire(&targets[i], cx),
            _ => {
                for t in &targets {
                    fire(t, cx);
                }
            }
        }
    }
}

// ---- ts_playeramount -----------------------------------------------------------------------------

/// `CTSPlayerAmount` (0x7299c): fires when the connect counter reaches exactly `clients`.
#[derive(Default)]
pub struct PlayerAmount {
    clients: i32,
    connect: String,
    disconnect: String,
    count: i32,
}

impl PlayerAmount {
    pub fn changed(&mut self, cx: &mut Cx, connected: bool) {
        self.count += if connected { 1 } else { -1 };
        if self.count != self.clients {
            return;
        }
        let t = if connected { self.connect.clone() } else { self.disconnect.clone() };
        if t.is_empty() {
            return;
        }
        let c = to_cstring(&t);
        crate::entities::fire_targets(&c, std::ptr::null_mut(), cx.edict, UseType::Toggle, 0.0);
        if cx.pev().spawnflags & 1 != 0 {
            crate::entities::remove(cx.edict);
        }
    }
}

impl Entity for PlayerAmount {
    /// `CBaseDelay::Save` / `Restore`: TS builds `CTSPlayerAmount` on `CBaseDelay`, but its
    /// `KeyValue` (0x729c0) never falls back to `CBaseDelay::KeyValue`, so `m_flDelay` and
    /// `m_iszKillTarget` stay 0 (and its `DelayedUse` branch never runs). No member pointers.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut Fields) {
        f.base_funcs(cx, BaseFuncs::default());
        f.table(&crate::save::tables::CBaseDelay);
        f.zero();
        f.zero();
    }

    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        let v = value.to_string_lossy().chars().take(99).collect::<String>();
        match key.to_bytes() {
            b"connecttarget" => self.connect = v,
            b"disconnecttarget" => self.disconnect = v,
            b"clients" => self.clients = v.trim().parse().unwrap_or(0),
            _ => return false,
        }
        true
    }

    fn spawn(&mut self, cx: &mut Cx) {
        cx.pev().classname = engine().alloc_string(c"ts_playeramount");
        let v = cx.pev();
        v.effects |= EF_NODRAW as i32;
        v.solid = SOLID_NOT as i32;
        v.movetype = MOVETYPE_FLY as i32;
        self.count = 0;
    }
}

// ---- ts_slowmotion -------------------------------------------------------------------------------

/// `CTSSlowMotion` (0x70cb0): area (touch) or whole-map slow motion, timed or toggled.
#[derive(Default)]
pub struct SlowMotion {
    duration: f32,
    amount: f32,
    toggled_on: bool,
    until: f32,
    global: bool,
    btime: bool,
    radius: f32,
    slowed: Vec<*mut edict_t>,
    /// `m_pfnThink` (`WaitThink`, from `Spawn`) and `m_pfnTouch` (`WaitTouch`, from
    /// `ActivateSlow` for an area slow, until a timed one runs out).
    funcs: BaseFuncs,
}

impl SlowMotion {
    fn active(&self) -> bool {
        self.until != 0.0 || self.toggled_on
    }

    /// `CTSSlowMotion::ActivateSlow` (0x71384).
    fn activate(&mut self, cx: &mut Cx) {
        if self.duration > 0.0 {
            self.until = cx.time() + self.duration;
        } else {
            self.until = 0.0;
            self.toggled_on = !self.toggled_on;
            if !self.toggled_on {
                self.deactivate();
            }
        }
        if !self.global {
            self.funcs.touch = true;
        }
    }

    fn deactivate(&mut self) {
        self.until = 0.0;
        if self.global {
            crate::rules::frame::global_slow2(0.0, false);
        } else {
            for e in std::mem::take(&mut self.slowed) {
                rules::powerup::go_slow(e, 1.0, false);
            }
        }
        self.slowed.clear();
    }

    /// `CheckPlayersDistance`: release players that left the area.
    fn check_distance(&mut self, cx: &mut Cx) {
        let v = cx.pev();
        let c = [(v.absmin[0] + v.absmax[0]) * 0.5, (v.absmin[1] + v.absmax[1]) * 0.5, (v.absmin[2] + v.absmax[2]) * 0.5];
        let r = self.radius;
        self.slowed.retain(|&e| {
            let inside = rules::dist(e.v().origin, c) <= r;
            if !inside {
                rules::powerup::go_slow(e, 1.0, false);
            }
            inside
        });
    }
}

impl Entity for SlowMotion {
    /// `CTSSlowMotion::Save` / `Restore`: `duration` (`+0xf8`) and `slow_ammount` (`+0xfc`,
    /// `slowamt`). The think is set from `Spawn` on, so TS's restore stops after `"BASE"`.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut Fields) {
        super::save_base(cx, f, &mut self.funcs);
        f.table(&crate::save::tables::CTSSlowMotion);
        f.float(&mut self.duration);
        f.float(&mut self.amount);
    }

    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        let v: f32 = value.to_str().unwrap_or("").trim().parse().unwrap_or(0.0);
        match key.to_bytes() {
            b"duration" => self.duration = v,
            b"slowamt" => self.amount = v,
            _ => return false,
        }
        true
    }

    fn spawn(&mut self, cx: &mut Cx) {
        trigger_brush(cx, c"ts_slowmotion");
        let v = cx.pev();
        let d = [v.absmax[0] - v.absmin[0], v.absmax[1] - v.absmin[1], v.absmax[2] - v.absmin[2]];
        self.radius = (rules::len3(d) * 0.5).round();
        self.global = v.spawnflags & 2 != 0;
        self.btime = v.spawnflags & 4 != 0;
        let start_on = v.spawnflags & 1 != 0;
        self.funcs = BaseFuncs { think: true, ..BaseFuncs::default() };
        cx.think_in(0.5);
        if start_on {
            self.activate(cx);
        }
    }

    fn think(&mut self, cx: &mut Cx) {
        if !self.funcs.think {
            return;
        }
        cx.think_in(0.5);
        let now = cx.time();
        if self.toggled_on {
            if self.global { crate::rules::frame::global_slow2(self.amount, self.btime) } else { self.check_distance(cx) }
        }
        if self.until != 0.0 {
            if now <= self.until {
                if self.global { crate::rules::frame::global_slow2(self.amount, self.btime) } else { self.check_distance(cx) }
            } else {
                self.funcs.touch = false;
                self.deactivate();
            }
        }
    }

    fn touch(&mut self, _cx: &mut Cx, other: *mut edict_t) {
        if !self.funcs.touch || self.global || !self.active() || !rules::is_player(other) {
            return;
        }
        if !rules::powerup::entity_in_slow(other) {
            rules::powerup::go_slow(other, self.amount, self.btime);
            if !self.slowed.contains(&other) {
                self.slowed.push(other);
            }
        }
    }

    fn use_(&mut self, cx: &mut Cx, _a: *mut edict_t, _c: *mut edict_t, _k: UseType, _v: f32) {
        self.activate(cx);
        if let Some(m) = message_of(cx) {
            msg::ts_message_all([0, 255, 0], 8, 30, &m);
        }
        fire_own_target(cx);
    }
}

// ---- ts_slowmotionpoint --------------------------------------------------------------------------

/// `CTSSlowMotionPoint` (0x714b4): slows every player in sight of a point for a while.
#[derive(Default)]
pub struct SlowMotionPoint {
    until: f32,
    amount: f32,
    btime: bool,
    slowed: Vec<*mut edict_t>,
    /// `m_pfnThink` (`WaitThink`, from `Spawn`).
    funcs: BaseFuncs,
}

impl SlowMotionPoint {
    fn deactivate(&mut self) {
        for e in std::mem::take(&mut self.slowed) {
            rules::powerup::go_slow(e, 1.0, false);
        }
    }
}

impl Entity for SlowMotionPoint {
    /// `CBaseEntity::Save` / `Restore`, with the think set.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut Fields) {
        super::save_base(cx, f, &mut self.funcs);
    }

    fn spawn(&mut self, cx: &mut Cx) {
        let e = cx.edict;
        cx.pev().classname = engine().alloc_string(c"ts_slowmotionpoint");
        engine().set_size(e, [0.0; 3], [0.0; 3]);
        let org = cx.pev().origin;
        engine().set_origin(e, org);
        let v = cx.pev();
        v.effects = EF_NODRAW as i32;
        v.movetype = MOVETYPE_NONE as i32;
        v.solid = SOLID_NOT as i32;
        self.funcs.think = true;
        cx.think_in(0.1);
    }

    fn think(&mut self, cx: &mut Cx) {
        if !self.funcs.think {
            return;
        }
        cx.think_in(0.5);
        if cx.time() > self.until {
            self.deactivate();
            crate::entities::remove(cx.edict);
            return;
        }
        let eng = engine();
        let org = cx.pev().origin;
        for p in rules::players() {
            if rules::powerup::entity_in_slow(p) {
                continue;
            }
            let tr = eng.trace_line(org, rules::eye(p.v()), IGNORE_MONSTERS, std::ptr::null_mut());
            if tr.flFraction >= 1.0 {
                rules::powerup::go_slow(p, self.amount, self.btime);
                if !self.slowed.contains(&p) {
                    self.slowed.push(p);
                }
            }
        }
    }
}

/// `CreateSlowMotionPoint(pos, amount, duration, flag)`.
pub fn create_slow_motion_point(origin: [f32; 3], amount: f32, duration: f32, btime: bool) {
    let eng = engine();
    let e = eng.create_named_entity(c"ts_slowmotionpoint");
    if e.is_null() {
        return;
    }
    (e.v().origin = origin);
    let now = eng.time();
    crate::world::dispatch(e, |ent, cx| {
        ent.spawn(cx);
        if let Some(p) = (ent as &mut dyn std::any::Any).downcast_mut::<SlowMotionPoint>() {
            p.until = now + duration;
            p.amount = amount;
            p.btime = btime;
        }
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn printf_name() {
        assert_eq!(super::format_name("%s reached the top", "bob"), "bob reached the top");
        assert_eq!(super::format_name("100%% done", "x"), "100% done");
        assert_eq!(super::format_name("no name", "x"), "no name");
    }
}
