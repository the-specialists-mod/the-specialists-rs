//! Capture the briefcase (gamerules-entities.md §3.4): `ts_briefcase` (home + capture zone),
//! `ts_flag` (the carried case), and `ts_scoregiver`/`ts_wingiver` (team score / win, §3.3).

use crate::damage::{self, DamageInfo};
use crate::entities::stock::explode::emit_sound_slow;
use crate::messages::Msg;
use crate::player::Player;
use crate::rules::{self, msg, state, teams};
use crate::weapons::{SlowPhysics, bounce_ground_damping, give_slow_mul};
use crate::world::{Cx, Entity, UseType};
use goldsrc::Global;
use goldsrc::edict::EdictPtr;
use goldsrc::ext::entities::ATTN_NORM;
use goldsrc::server::{DONT_IGNORE_MONSTERS, engine};
use goldsrc::sys::*;
use shared::protocol::rules::{ScoreInfo, TsFlag, TsHVoice};
use shared::weapons::spread::{self, Vec3};
use std::cell::Cell;
use std::ffi::CStr;

pub const MODEL: &CStr = c"models/briefcase.mdl";
const FLAG_SLOTS: i32 = 20;

static NEXT_FLAG_ID: Global<Cell<i32>> = Global(Cell::new(0));

/// Glow colour per owner team (`CTSFlag::Spawn` 0x9d978).
fn team_color(team: i32) -> [f32; 3] {
    match team {
        0 => [125.0, 165.0, 210.0],
        1 => [200.0, 90.0, 70.0],
        2 => [225.0, 205.0, 45.0],
        // TS's last branch, which also covers a case any team may take (-1).
        _ => [145.0, 215.0, 140.0],
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
    rules::other(e, |p| score_info_of(e, p));
}

/// `ScoreInfo` for `e` from its `Player`, for when the caller already holds it.
fn score_info_of(e: *mut edict_t, p: &Player) {
    let idx = engine().index_of(e);
    let s = ScoreInfo { idx: idx as u8, frags: e.v().frags as i16, deaths: p.deaths as i16, class: 0, team: teams::score_team(idx) };
    msg::all(Msg::ScoreInfo, |w| s.write(w));
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
    log_flag(player, "captured_briefcase", &carried_contents(player));
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

/// `CTSFlag`. TS destroys the case when a player takes it and creates a new one (`CreateFlag`
/// 0x9d874) when it is dropped; the port keeps one entity per case and hides it on the carrier
/// (`MOVETYPE_FOLLOW`) instead, re-running `Spawn` when it is thrown.
#[derive(Default)]
pub struct Flag {
    pub id: i32,
    pub team: i32,
    pub contents: String,
    pub home: Option<*mut edict_t>,
    pub holder: Option<*mut edict_t>,
    /// `s_flag_status+0x10`: entity index of the last player to take this case. It travels with
    /// the case through drops and resets when the home zone drops a new one, so retaking your
    /// own drop isn't announced again.
    last_holder: i32,
    /// `+0x8c` set: a player threw this case (`DropFlag`), rather than the home zone dropping it
    /// (`FlagFall`). Only a thrown case can be returned by its own team.
    dropped: bool,
    /// `+0x88`: return home after this time (0 = never, the case at home).
    return_at: f32,
    /// `OnGround` (0x9e21c) has run: touch `WaitTouch`, think `WaitThink`. Before it the case is
    /// flying: touch `BounceTouch`, think `Fall`.
    on_ground: bool,
    phys: SlowPhysics,
}

/// `DropFlag` 0x9334c, alive: thrown along `v_angle` at `(90 - pitch) * 2`, so looking down
/// barely tosses the case and looking up throws it far.
fn aimed_throw_speed(pitch: f32) -> f32 {
    (90.0 - pitch) + (90.0 - pitch)
}

/// `DropFlag` 0x9334c: the case starts 16 u in front of the eyes and flies at 0.9 of the
/// player's velocity plus `speed` along `forward` plus `speed * 0.33` up. Returns
/// (origin, velocity).
fn throw(eye: Vec3, forward: Vec3, player_velocity: Vec3, speed: f32) -> (Vec3, Vec3) {
    let origin = spread::ma(eye, 16.0, forward);
    let mut velocity = spread::ma(spread::scale(player_velocity, 0.9), speed, forward);
    velocity[2] += speed * 0.33;
    (origin, velocity)
}

/// `Fall` 0x9df10: a flying case hits a non-player entity whose health is at most
/// `|v| * 30`, for `|v| * 30 * 0.001` damage. Returns (health threshold, damage).
fn impact(speed: f32) -> (f32, f32) {
    let threshold = speed * 30.0;
    (threshold, (threshold as f64 * 0.001f32 as f64) as f32)
}

/// The `"name<uid><authid><uid>"` log lines of `TakeFlag` 0x93830 (`aquired_briefcase`),
/// `DropFlag` 0x9334c (`dropped_briefcase`) and `CaptureTouch` 0x8db9c (`captured_briefcase`).
/// TS passes the user id where other lines put the team, so it appears twice.
fn flag_log_line(name: &str, user_id: i32, auth: &str, event: &str, contents: &str) -> String {
    format!("\"{name}<{user_id}><{auth}><{user_id}>\" triggered \"{event}\" (contents \"{contents}\")\n")
}

fn log_flag(e: *mut edict_t, event: &str, contents: &str) {
    let eng = engine();
    msg::log(&flag_log_line(&rules::netname(e), eng.user_id(e), &eng.auth_id(e).to_string_lossy(), event, contents));
}

impl Flag {
    /// `CTSFlag::RespawnFlag` (0x9de58), when a dropped case's time is up: the home zone drops
    /// a new case 1 s later (`FlagFall`, which sends `TSFlag` 0), and the owning team's name is
    /// announced. A case any team may capture (team -1) returns silently, as TS only names
    /// teams 0..3.
    fn respawn_flag(&mut self, cx: &mut Cx) {
        if let Ok(team) = usize::try_from(self.team)
            && let Some(name) = state::with(|g| g.teams.get(team).map(|t| t.name.clone()))
        {
            msg::ts_message_all([0, 255, 0], 4, 30, &format!("{name} briefcase returned"));
        }
        self.go_home(cx, 1.0);
    }

    /// Removes the case; its `ts_briefcase` drops a new one (`FlagFall`) after `delay`.
    fn go_home(&mut self, cx: &mut Cx, delay: f32) {
        if let Some(home) = self.home {
            crate::world::dispatch(home, |ent, _| {
                if let Some(b) = (ent as &mut dyn std::any::Any).downcast_mut::<Briefcase>() {
                    b.respawn_flag(delay);
                }
            });
        }
        crate::entities::remove(cx.edict);
    }

    /// `CTSFlag::Fall` (0x9df10), every 0.1 s while flying: lands when it rests on the ground
    /// and is slower than `3 * slow` (sound, level pitch and roll, `OnGround`), returns home when
    /// its 20 s are up, and otherwise hurts what is 0.1 s ahead of it, then runs the slow-motion
    /// physics (`CTSPhysicObject::WaitThink`).
    fn fall(&mut self, cx: &mut Cx) {
        let me = cx.edict;
        let v = cx.pev();
        if v.flags & FL_ONGROUND as i32 != 0 && spread::length(v.velocity) < give_slow_mul(v.fuser1) * 3.0 {
            emit_sound_slow(me, CHAN_STATIC, c"items/weapondrop1.wav", 1.0, ATTN_NORM, 0, 100, 0.5);
            let v = cx.pev();
            v.angles[0] = 0.0;
            v.angles[2] = 0.0;
            self.land(cx);
        }
        if self.return_at != 0.0 && cx.time() > self.return_at {
            self.respawn_flag(cx);
            return;
        }
        self.hit_ahead(cx);
        self.phys.update(cx.pev());
    }

    /// The impact half of `Fall` (0x9e0a6..0x9e196): `TraceAttack(pev, |v|·30·0.001, v, tr,
    /// DMG_CLUB | DMG_NEVERGIB)` then `ApplyMultiDamage(pev, pev)` on a non-player hit whose
    /// health is at most `|v|·30`, so a thrown case breaks glass. The case itself is the
    /// attacker, as in TS.
    fn hit_ahead(&mut self, cx: &mut Cx) {
        let e = engine();
        let me = cx.edict;
        let (origin, velocity) = (cx.pev().origin, cx.pev().velocity);
        let tr = e.trace_line(origin, spread::ma(origin, 0.1, velocity), DONT_IGNORE_MONSTERS, me);
        // A null hit is the world in TS, which takes no damage.
        if tr.flFraction >= 1.0 || tr.pHit.is_null() || rules::is_player(tr.pHit) {
            return;
        }
        let (threshold, dmg) = impact(spread::length(velocity));
        if tr.pHit.v().health <= threshold {
            let mut d = DamageInfo::new(me, me, dmg, damage::bits::CLUB | damage::bits::NEVERGIB).traced();
            d.dir = velocity;
            d.pos = tr.vecEndPos;
            damage::apply(tr.pHit, &d);
        }
    }

    /// `CTSFlag::BounceTouch` (0x9dbbc): every touch while flying keeps 0.3 of the vertical
    /// speed. In the air, a fast hit on a floor (normal z > 0.98) plays the drop sound; on the
    /// ground the case is slowed ([`bounce_ground_damping`]) and its roll levelled.
    fn bounce_touch(&mut self, cx: &mut Cx) {
        let e = engine();
        let me = cx.edict;
        let v = cx.pev();
        v.velocity[2] = (v.velocity[2] as f64 * 0.3) as f32;
        let slow = give_slow_mul(v.fuser1);
        if v.flags & FL_ONGROUND as i32 == 0 {
            if spread::length(v.velocity) > slow * 18.0 && e.globals().trace_plane_normal[2] as f64 > 0.98 {
                let pitch = 100 + e.random_long(0, 10);
                emit_sound_slow(me, CHAN_STATIC, c"items/weapondrop1.wav", 1.0, ATTN_NORM, 0, pitch, 0.5);
            }
        } else {
            let k = bounce_ground_damping(v.velocity[0].hypot(v.velocity[1]), slow);
            v.velocity = spread::scale(v.velocity, k);
            v.avelocity = spread::scale(v.avelocity, k);
            v.angles[2] = 0.0;
        }
    }

    /// `CTSFlag::OnGround` (0x9e21c): a 32×32×48 trigger the thrower can touch again.
    fn land(&mut self, cx: &mut Cx) {
        let e = engine();
        let me = cx.edict;
        self.on_ground = true;
        let v = cx.pev();
        v.owner = std::ptr::null_mut();
        v.solid = SOLID_TRIGGER as i32;
        let origin = v.origin;
        e.set_origin(me, origin);
        e.set_size(me, [-16.0, -16.0, 0.0], [16.0, 16.0, 48.0]);
    }

    /// The `CreateFlag` + `Spawn` half of `DropFlag(false)` (0x9334c) on the carried case: it
    /// flies again from `origin` at `velocity`, spinning at random, in the thrower's time scale,
    /// passing through the thrower until it lands, and returns home after 20 s.
    fn thrown(&mut self, cx: &mut Cx, thrower: *mut edict_t, time_scale: f32, origin: Vec3, velocity: Vec3) {
        let e = engine();
        self.holder = None;
        self.spawn(cx);
        self.dropped = true;
        let v = cx.pev();
        v.effects &= !(EF_NODRAW as i32);
        v.aiment = std::ptr::null_mut();
        v.owner = thrower;
        v.fuser1 = time_scale;
        v.angles = [0.0; 3];
        v.avelocity = [0.0, e.random_long(-300, 300) as f32, e.random_long(-300, 300) as f32];
        v.velocity = velocity;
        e.set_origin(cx.edict, origin);
    }
}

impl Entity for Flag {
    /// `CTSFlag::Spawn` (0x9d978): a flying, zero-size `SOLID_BBOX` case with the team's glow,
    /// due home in 20 s (`FlagFall` clears that for a case at home).
    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        let e = cx.edict;
        self.phys = SlowPhysics::default();
        eng.set_model(e, MODEL);
        let v = cx.pev();
        v.movetype = MOVETYPE_BOUNCE as i32;
        v.solid = SOLID_BBOX as i32;
        v.gravity = 0.7;
        v.friction = 0.8;
        v.classname = eng.alloc_string(c"ts_flag");
        let org = v.origin;
        eng.set_origin(e, org);
        eng.set_size(e, [0.0; 3], [0.0; 3]);
        self.return_at = eng.time() + 20.0;
        self.dropped = false;
        self.on_ground = false;
        cx.think_in(0.1);
        let v = cx.pev();
        v.iuser4 = 2;
        v.renderamt = 5.0;
        v.renderfx = kRenderFxGlowShell as i32;
        v.rendercolor = team_color(self.team);
    }

    /// `Fall` while flying, `CTSFlag::WaitThink` (0x9e1b8) once landed; both every 0.1 s.
    fn think(&mut self, cx: &mut Cx) {
        cx.think_in(0.1);
        if self.holder.is_some() {
            return;
        }
        if !self.on_ground {
            self.fall(cx);
        } else if self.return_at != 0.0 && self.return_at < cx.time() {
            self.respawn_flag(cx);
        }
    }

    /// `BounceTouch` while flying (a flying case is only taken with +use), `CTSFlag::WaitTouch`
    /// (0x9e604) once landed.
    fn touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        if self.holder.is_some() {
            return;
        }
        if self.on_ground {
            self.player_take_flag(cx, other, None);
        } else {
            self.bounce_touch(cx);
        }
    }
}

impl Flag {
    /// `CTSFlag::Use` (0x9e5dc) when a player's +use picks the case (`player::player_use`). The
    /// player is being dispatched, so its object is passed in.
    pub fn use_by_player(&mut self, cx: &mut Cx, p: &mut Player, pcx: &mut Cx) {
        let other = pcx.edict;
        self.player_take_flag(cx, other, Some((p, pcx)));
    }

    /// `CTSFlag::PlayerTakeFlag` (0x9e330), shared by the touch and +use. `p` is the player's
    /// object when the caller already holds it; otherwise it is looked up.
    fn player_take_flag(&mut self, cx: &mut Cx, other: *mut edict_t, p: Option<(&mut Player, &mut Cx)>) {
        // A live, visible player (TS: `pev->effects` sign bit, EF_NODRAW, e.g. observers).
        if self.holder.is_some() || !rules::is_player(other) || !rules::alive(other.v()) || other.v().effects & EF_NODRAW as i32 != 0 {
            return;
        }
        let team = team_index_of(other);
        if self.team == -1 || team != self.team {
            let me = cx.edict;
            let took = match p {
                Some((p, pcx)) => self.take_by(me, p, pcx),
                None => crate::player::with_player(other, |p, pcx| self.take_by(me, p, pcx)).unwrap_or(false),
            };
            if !took {
                return;
            }
            // A case taken from home closes its zone for captures until it is back (`FlagFall`).
            if !self.dropped
                && let Some(home) = self.home
            {
                crate::world::with::<Briefcase, _>(home, |b| b.active = false);
            }
            self.holder = Some(other);
            self.return_at = 0.0;
            let v = cx.pev();
            v.effects |= EF_NODRAW as i32;
            v.solid = SOLID_NOT as i32;
            v.movetype = MOVETYPE_FOLLOW as i32;
            v.aiment = other;
        } else if self.dropped && self.home.is_some() {
            // The owning team returns its thrown case; the bonus needs a second player. No
            // `TSFlag` here: the home zone's `FlagFall` sends state 0 when the case reappears.
            emit_sound_slow(other, CHAN_ITEM, c"items/gunpickup2.wav", 1.0, ATTN_NORM, 0, 100, 0.5);
            if rules::players().len() > 1 {
                let mut p = p.map(|(p, _)| p);
                rules::add_frags(other, p.as_deref_mut(), 5.0);
                msg::ts_message_all([0, 255, 0], 4, 30, &format!("{} returned the briefcase! 5 #TS_Frg", rules::netname(other)));
                match p {
                    Some(p) => score_info_of(other, p),
                    None => score_info(other),
                }
            }
            // FlagFall at home 1 s later (0x9e330).
            self.go_home(cx, 1.0);
        }
    }

    /// `CBasePlayer::TakeFlag` (0x93830): the case needs 20 free slots and one case at most. It
    /// slows the carrier, plays the pickup sound, sends `TSFlag` 1, announces the pickup unless
    /// the taker was the case's last holder, and logs it.
    fn take_by(&mut self, me: *mut edict_t, p: &mut Player, pcx: &mut Cx) -> bool {
        if p.inv.sv.free_slots < FLAG_SLOTS || p.ts.flag.is_some() {
            return false;
        }
        p.ts.flag = Some(me);
        p.inv.sv.free_slots -= FLAG_SLOTS;
        crate::weapons::sync_speed(p, pcx);
        let e = pcx.edict;
        emit_sound_slow(e, CHAN_ITEM, c"items/gunpickup2.wav", 1.0, ATTN_NORM, 0, 100, 0.5);
        let idx = engine().index_of(e);
        send_flag(1, self.id, self.team, idx);
        if idx != self.last_holder {
            let name = rules::netname(e);
            msg::ts_message_all([0, 255, 0], 4, 30, &format!("{name} picked up a briefcase containing {}!", self.contents));
        }
        self.last_holder = idx;
        log_flag(e, "aquired_briefcase", &self.contents);
        true
    }
}

/// `CTSBriefcase::FlagFall` (0x8da00): a new case (`CreateFlag` without a player: no owner, time
/// scale 1, no return time) falls from the centre of its zone.
fn create_flag(origin: [f32; 3], id: i32, team: i32, contents: &str, home: *mut edict_t) {
    let eng = engine();
    let e = eng.create_named_entity(c"ts_flag");
    if e.is_null() {
        return;
    }
    (e.v().origin = origin);
    let contents = contents.to_owned();
    crate::world::dispatch(e, |ent, cx| {
        let Some(f) = (ent as &mut dyn std::any::Any).downcast_mut::<Flag>() else { return };
        f.id = id;
        f.team = team;
        f.contents = contents;
        f.home = Some(home);
        f.spawn(cx);
        f.return_at = 0.0;
        let v = cx.pev();
        v.owner = std::ptr::null_mut();
        v.fuser1 = 1.0;
    });
}

/// True if the player carries a case.
pub fn has_flag(e: *mut edict_t) -> bool {
    rules::other(e, |p| p.ts.flag.is_some()).unwrap_or(false)
}

/// What the case `e` carries holds, for the capture log.
fn carried_contents(e: *mut edict_t) -> String {
    let flag = rules::other(e, |p| p.ts.flag).flatten();
    flag.and_then(|f| crate::world::with::<Flag, _>(f, |f| f.contents.clone())).unwrap_or_default()
}

/// `CBasePlayer::DropFlag(captured)` (0x9334c). Either way the carrier gets its 20 slots back.
/// A captured case is removed; the capture already scheduled its home's `FlagFall`, which sends
/// `TSFlag` 0, so nothing is sent here. Otherwise the case is thrown from the eyes: along
/// `v_angle` at [`aimed_throw_speed`] while alive, or in a random direction (pitch −60..60, yaw
/// −180..180) at 60..160 when dead ([`throw`]), then logged and announced with `TSFlag` 2.
pub fn drop_flag(p: &mut Player, cx: &mut Cx, captured: bool) {
    let Some(flag) = p.ts.flag.take() else { return };
    p.inv.sv.free_slots += FLAG_SLOTS;
    crate::weapons::sync_speed(p, cx);
    if captured {
        crate::world::dispatch(flag, |ent, fcx| {
            if let Some(f) = (ent as &mut dyn std::any::Any).downcast_mut::<Flag>() {
                f.holder = None;
                f.go_home(fcx, 0.25);
            }
        });
        return;
    }
    let e = engine();
    let (angles, speed) = if rules::alive(cx.pev()) {
        let a = cx.pev().v_angle;
        (a, aimed_throw_speed(a[0]))
    } else {
        let pitch = e.random_long(-60, 60) as f32;
        let yaw = e.random_long(-180, 180) as f32;
        ([pitch, yaw, 0.0], e.random_long(60, 160) as f32)
    };
    let (forward, _, _) = spread::angle_vectors(angles);
    let v = cx.pev();
    let (origin, velocity) = throw(spread::add(v.origin, v.view_ofs), forward, v.velocity, speed);
    let time_scale = crate::weapons::thrower_time_scale(p, cx.pev());
    let thrower = cx.edict;
    let holder = cx.index();
    crate::world::dispatch(flag, |ent, fcx| {
        let Some(f) = (ent as &mut dyn std::any::Any).downcast_mut::<Flag>() else { return };
        f.thrown(fcx, thrower, time_scale, origin, velocity);
        log_flag(thrower, "dropped_briefcase", &f.contents);
        send_flag(2, f.id, f.team, holder);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-3)
    }

    #[test]
    fn aimed_throw_speed_follows_pitch() {
        assert_eq!(aimed_throw_speed(0.0), 180.0);
        // looking down tosses it at the feet, looking up throws it far
        assert_eq!(aimed_throw_speed(89.0), 2.0);
        assert_eq!(aimed_throw_speed(-45.0), 270.0);
    }

    #[test]
    fn throw_starts_in_front_of_the_eyes_with_lift() {
        let (origin, velocity) = throw([0.0, 0.0, 28.0], [1.0, 0.0, 0.0], [100.0, 0.0, 0.0], 180.0);
        assert!(close(origin, [16.0, 0.0, 28.0]), "{origin:?}");
        // 0.9 of the player's velocity + 180 forward + 180 * 0.33 up
        assert!(close(velocity, [270.0, 0.0, 59.4]), "{velocity:?}");
        // a dead thrower: random angles, here straight down at the slowest speed
        let (forward, _, _) = spread::angle_vectors([60.0, 90.0, 0.0]);
        let (origin, velocity) = throw([0.0; 3], forward, [0.0, 0.0, -50.0], 60.0);
        assert!(close(origin, spread::scale(forward, 16.0)), "{origin:?}");
        assert!(close(velocity, [0.0, 30.0, -45.0 - 51.961 + 19.8]), "{velocity:?}");
    }

    #[test]
    fn impact_scales_with_speed() {
        let (threshold, dmg) = impact(500.0);
        assert_eq!(threshold, 15000.0);
        assert!((dmg - 15.0).abs() < 1e-4, "{dmg}");
    }

    #[test]
    fn log_lines_repeat_the_user_id() {
        assert_eq!(
            flag_log_line("Neo", 7, "STEAM_0:1:2", "aquired_briefcase", "two elephants"),
            "\"Neo<7><STEAM_0:1:2><7>\" triggered \"aquired_briefcase\" (contents \"two elephants\")\n"
        );
    }

    #[test]
    fn unknown_teams_glow_green() {
        assert_eq!(team_color(-1), [145.0, 215.0, 140.0]);
        assert_eq!(team_color(3), [145.0, 215.0, 140.0]);
        assert_eq!(team_color(1), [200.0, 90.0, 70.0]);
    }
}
