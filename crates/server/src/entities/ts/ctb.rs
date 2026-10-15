//! Capture the briefcase (gamerules-entities.md §3.4): `ts_briefcase` (home + capture zone),
//! `ts_flag` (the carried case), and `ts_scoregiver`/`ts_wingiver` (team score / win, §3.3).

use crate::damage::{self, DamageInfo};
use crate::entities::stock::explode::emit_sound_slow;
use crate::messages::Msg;
use crate::player::Player;
use crate::rules::{self, lifecycle, msg, state, teams};
use crate::save::{BaseFuncs, Fields};
use crate::weapons::{SlowPhysics, bounce_drops_loudly, bounce_ground_damping, give_slow_mul};
use crate::world::{self, Cx, Entity, UseType};
use goldsrc::Global;
use goldsrc::edict::EdictPtr;
use goldsrc::ext::entities::{ATTN_NORM, EHandle};
use goldsrc::server::{DONT_IGNORE_MONSTERS, engine};
use goldsrc::sys::*;
use shared::pm::{Arith, Ext, Fpu, FpuMath, Num, ORDER_XYZ, plus_double};
use shared::protocol::rules::{RoundTime, ScoreInfo, TsFlag, TsHVoice};
use shared::weapons::spread::{self, Vec3};
use std::cell::{Cell, RefCell};
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
    let m = TsFlag { state: state_, id: id as u8, team: team.wrapping_add(1) as u8, holder: holder as u8 };
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
    let s = ScoreInfo {
        idx: idx as u8,
        frags: lifecycle::score_frags(e.v().frags),
        deaths: p.deaths as i16,
        class: 0,
        team: teams::score_team(idx),
    };
    msg::all(Msg::ScoreInfo, |w| s.write(w));
}

// ---- ts_briefcase ----------------------------------------------------------------------------------

/// `s_flag_status` (0x1c bytes): what a case carries from its home zone to the case entity, to
/// the carrier (`CBasePlayer +0x7d0`) and back. `Some` on the player plays the part of its
/// `have` byte.
#[derive(Clone, Debug, Default)]
pub struct FlagStatus {
    /// `contents` of the home zone (TS points at the zone's own string).
    pub contents: String,
    pub id: i32,
    /// 0-based team that owns the case (-1 = any team may take it).
    pub team: i32,
    /// Entity index of the last player to take this case (0 for a case the zone just dropped),
    /// so retaking your own drop isn't announced again.
    pub last_holder: i32,
    /// The home `ts_briefcase` (`EHANDLE`).
    pub home: EHandle,
}

/// `ts_briefcase` think: `FlagFall` once (true) or `WaitThink` (false). TS swaps the think
/// pointer (`SetThink`) from other entities' code, which may run while the zone is itself
/// mid-dispatch (a capture schedules the carried case's home), so it is kept outside the object.
static FALL_PENDING: Global<RefCell<Vec<usize>>> = Global(RefCell::new(Vec::new()));

/// `SetThink(FlagFall); pev->nextthink = time + delay` on a `ts_briefcase`.
fn schedule_flag_fall(home: *mut edict_t, delay: f32) {
    {
        let mut pending = FALL_PENDING.0.borrow_mut();
        if !pending.contains(&(home as usize)) {
            pending.push(home as usize);
        }
    }
    home.v().nextthink = engine().time() + delay;
    // `m_pfnThink` is set again (it matters after a restore cleared it). A zone in its own
    // `CaptureTouch` is out for dispatch: its touch sets it.
    crate::world::with::<Briefcase, _>(home, |b| b.funcs.think = true);
}

fn flag_fall_pending(home: *mut edict_t) -> bool {
    FALL_PENDING.0.borrow().contains(&(home as usize))
}

fn take_flag_fall(home: *mut edict_t) -> bool {
    let mut pending = FALL_PENDING.0.borrow_mut();
    let before = pending.len();
    pending.retain(|&e| e != home as usize);
    pending.len() != before
}

pub struct Briefcase {
    pub id: i32,
    /// 0-based team that may capture here (-1 = any).
    owner_team: i32,
    contents: String,
    /// `+0x64`: captures are accepted (`FlagFall` sets it; a case taken from home clears it).
    active: bool,
    /// TS's member pointers: `m_pfnThink` (`FlagFall` / `WaitThink`) and `m_pfnTouch`
    /// (`CaptureTouch`), both set from `Spawn` on.
    funcs: BaseFuncs,
}

impl Briefcase {
    /// The 0-based team that may capture here (-1 any), and whether captures are accepted now
    /// (the plugin API's `entity_info`).
    pub fn capture_team(&self) -> (i32, bool) {
        (self.owner_team, self.active)
    }
}

impl Default for Briefcase {
    /// `CTSBriefcase::CTSBriefcase` (0x8d788): ids 1..15, wrapping.
    fn default() -> Self {
        let mut id = NEXT_FLAG_ID.0.get() + 1;
        if id > 15 {
            id = 1;
        }
        NEXT_FLAG_ID.0.set(id);
        Self { id, owner_team: -1, contents: String::new(), active: false, funcs: BaseFuncs::default() }
    }
}

impl Briefcase {
    /// The zone's `s_flag_status` (`+0x90`, set in `Spawn`).
    fn status(&self, me: *mut edict_t) -> FlagStatus {
        FlagStatus { contents: self.contents.clone(), id: self.id, team: self.owner_team, last_holder: 0, home: EHandle::new(me) }
    }

    /// `CTSBriefcase::FlagFall` (0x8da00): a new case (`CreateFlag` without a player: no owner,
    /// time scale 1) with no return time, put at the centre of the zone's `mins`/`maxs` without
    /// relinking it; captures open; `TSFlag` 0; then `WaitThink` every second.
    fn flag_fall(&mut self, cx: &mut Cx) {
        let me = cx.edict;
        let flag = create_flag(None, self.status(me));
        if !flag.is_null() {
            crate::world::with::<Flag, _>(flag, |f| f.return_at = 0.0);
            let v = cx.pev();
            let center = [(v.mins[0] + v.maxs[0]) * 0.5, (v.mins[1] + v.maxs[1]) * 0.5, (v.mins[2] + v.maxs[2]) * 0.5];
            flag.v().origin = center;
        }
        self.active = true;
        send_flag(0, self.id, self.owner_team, 0);
        self.funcs.think = true;
        cx.think_in(1.0);
    }
}

impl Entity for Briefcase {
    /// `CBaseEntity::Save` / `Restore`: TS saves none of `CTSBriefcase`'s own state. Both pointers
    /// are set, so TS's restore stops after `"BASE"`, and the restored zone neither drops a case
    /// nor takes captures.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut Fields) {
        super::save_base(cx, f, &mut self.funcs);
    }

    /// `CBaseEntity::Precache` (empty). The restore left TS's think null, so a `FlagFall` the
    /// port had pending for this edict is dropped too.
    fn restored(&mut self, cx: &mut Cx) {
        take_flag_fall(cx.edict);
    }

    /// `CTSBriefcase::KeyValue` (0x8d7f4).
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"contents" => self.contents = String::from_utf8_lossy(&value.to_bytes()[..value.to_bytes().len().min(31)]).into_owned(),
            b"ownerteam" => self.owner_team = crate::entities::stock::base::kv_i(value).wrapping_sub(1),
            _ => return false,
        }
        true
    }

    /// `CTSBriefcase::Spawn` (0x8d88c): an invisible `MOVETYPE_FLY` trigger; the first case falls
    /// a second later.
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
        eng.set_size(e, mins, maxs);
        eng.set_origin(e, org);
        let v = cx.pev();
        v.effects = EF_NODRAW as i32;
        v.solid = SOLID_TRIGGER as i32;
        v.flags = 0;
        v.movetype = MOVETYPE_FLY as i32;
        self.active = false;
        if self.contents.is_empty() {
            self.contents = "a flag".into();
        }
        self.funcs = BaseFuncs { think: true, touch: true, ..BaseFuncs::default() };
        schedule_flag_fall(e, 1.0);
    }

    fn think(&mut self, cx: &mut Cx) {
        if take_flag_fall(cx.edict) {
            self.flag_fall(cx);
        } else if self.funcs.think {
            // `CTSBriefcase::WaitThink` (0x8db14).
            cx.think_in(1.0);
        }
    }

    /// `CTSBriefcase::CaptureTouch` (0x8db9c): open, a living player who carries a case and may
    /// use this zone (`CanInteract`: any team, or the zone's).
    fn touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        if !self.funcs.touch || !self.active || !rules::has_player(other) || !rules::is_alive(other) || !has_flag(other) {
            return;
        }
        if self.owner_team != -1 && self.owner_team != team_index_of(other) {
            return;
        }
        capture(cx, other);
        if flag_fall_pending(cx.edict) {
            self.funcs.think = true;
        }
    }
}

/// The capture of `CaptureTouch` (0x8db9c): the log line, the carried case's home drops a new one
/// in 0.25 s, `DropFlag(true)`, the zone's target and message, then with two or more players
/// (`CountPlayers`, players who left included) +10 frags, `ScoreInfo`, the team's score and
/// `TSHVoice`, otherwise every team's score is cleared; `TeamScore` of every named team.
fn capture(cx: &mut Cx, player: *mut edict_t) {
    let eng = engine();
    let name = rules::netname(player);
    let status = rules::other(player, |p| p.ts.flag.clone()).flatten().unwrap_or_default();
    log_flag(player, "captured_briefcase", &status.contents);
    let home = status.home.get();
    if !home.is_null() {
        schedule_flag_fall(home, 0.25);
    }
    crate::player::with_player(player, |p, pcx| drop_flag(p, pcx, true));
    let (target, message) = (cx.pev().target, cx.pev().message);
    if target != 0 {
        crate::entities::fire_targets(eng.string(target), cx.edict, cx.edict, UseType::Toggle, 0.0);
    }
    if message != 0 {
        msg::ts_message_all([0, 255, 0], 8, 30, eng.string(message).to_bytes());
    }
    if rules::slot_players().len() < 2 {
        // TS zeroes each team's cash (and an unused counter) too.
        state::with(|g| {
            for t in &mut g.teams {
                t.cash = 0;
                t.score = 0;
            }
        });
        msg::ts_message_all([0, 0, 0], 8, 30, "Waiting for other players.");
    } else {
        rules::add_frags(player, None, 10.0);
        msg::ts_message_all([0, 255, 0], 4, 30, &shared::bformat!("{} captured the briefcase! 10 #TS_Frg", name));
        score_info(player);
        let team = team_index_of(player);
        if team >= 0 {
            state::with(|g| g.teams[team as usize].score += 1);
        }
        for o in rules::slot_players() {
            let lost = (team != team_index_of(o)) as u8;
            msg::one(Msg::TsHVoice, o, |w| TsHVoice { lost }.write(w));
        }
    }
    teams::send_team_scores();
}

// ---- ts_flag -------------------------------------------------------------------------------------

/// `CTSFlag`: a case lying around or flying. Taking it destroys the entity and moves its
/// `s_flag_status` to the player (`CBasePlayer::TakeFlag` 0x93830); dropping it creates a new
/// one (`CTSFlag::CreateFlag` 0x9d874).
#[derive(Default)]
pub struct Flag {
    /// `+0x90`.
    status: FlagStatus,
    /// `+0x8c` set: a player threw this case (`DropFlag`), rather than the home zone dropping it
    /// (`FlagFall`). Only a thrown case can be returned by its own team.
    thrown: bool,
    /// `+0x88`: return home after this time (0 = never, the case at home).
    return_at: f32,
    /// `+0xac`: `Destroy` ran (the entity is going away this frame).
    destroyed: bool,
    /// `OnGround` (0x9e21c) has run: touch `WaitTouch`, think `WaitThink`. Before it the case is
    /// flying: touch `BounceTouch`, think `Fall`.
    on_ground: bool,
    phys: SlowPhysics,
    /// TS's member pointers: `m_pfnThink` (`Fall` / `WaitThink`) and `m_pfnTouch` (`BounceTouch` /
    /// `WaitTouch`), both set from `Spawn` on.
    funcs: BaseFuncs,
}

impl Flag {
    /// The 0-based team that owns the case (-1 any), for the plugin API's `entity_info`.
    pub fn team(&self) -> i32 {
        self.status.team
    }
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
/// `|v| * 30`, for `|v| * 30 * 0.001` damage (issue #150). Returns (the value compared with the
/// health, the damage), as each build computes them from the speed (`x² + y² + z²`):
///
/// * mp.dll (`CTSFlag::Fall` 0x100727ff..0x1007286e) keeps the root unrounded, compares
///   `speed·30f`, and deals `Length()·0.03f` (MSVC folded `30·0.001f` into one `float`), rounded
///   once.
/// * `ts_i386.so` (0x9e0e3..0x9e183) stores the root, compares `speed·30` and deals
///   `speed·30·0.001f` with the products unrounded.
fn impact(arith: Arith, velocity: Vec3) -> (Ext, f32) {
    fn go<F: Fpu>(v: Vec3) -> (Ext, f32) {
        let s = F::vec_length(v, ORDER_XYZ);
        let threshold = s * F::x(30.0);
        let dmg = if F::MSVC { s * F::x(0.03) } else { threshold * F::x(0.001) };
        (threshold.ext(), dmg.f())
    }
    shared::by_arith!(arith, go(velocity))
}

/// The `"name<uid><authid><uid>"` log lines of `TakeFlag` 0x93830 (`aquired_briefcase`),
/// `DropFlag` 0x9334c (`dropped_briefcase`) and `CaptureTouch` 0x8db9c (`captured_briefcase`).
/// TS passes the user id where other lines put the team, so it appears twice.
fn flag_log_line(name: &[u8], user_id: i32, auth: &[u8], event: &str, contents: &str) -> Vec<u8> {
    shared::bformat!("\"{}<{}><{}><{}>\" triggered \"{}\" (contents \"{}\")\n", name, user_id, auth, user_id, event, contents)
}

fn log_flag(e: *mut edict_t, event: &str, contents: &str) {
    let eng = engine();
    msg::log(&flag_log_line(&rules::netname(e), eng.user_id(e), eng.auth_id(e).to_bytes(), event, contents));
}

impl Flag {
    /// `CTSFlag::Destroy` (0x9de24): out of every player's slowed list, then `UTIL_Remove`.
    fn destroy(&mut self, cx: &mut Cx) {
        self.destroyed = true;
        rules::powerup::remove_from_slowed(cx.edict);
        crate::entities::remove(cx.edict);
    }

    /// `CTSFlag::RespawnFlag` (0x9de58), when the case's time is up (or a round restarts): its
    /// home drops a new case 1 s later, the owning team's name is announced (a case any team may
    /// take, team -1, returns silently: TS only names teams 0..3), and the case goes.
    fn respawn_flag(&mut self, cx: &mut Cx) {
        let home = self.status.home.get();
        if !home.is_null() {
            schedule_flag_fall(home, 1.0);
            if let Ok(team) = usize::try_from(self.status.team)
                && team < 4
            {
                let name = state::with(|g| g.teams.get(team).map(|t| t.name.clone()).unwrap_or_default());
                msg::ts_message_all([0, 255, 0], 4, 30, &shared::bformat!("{} briefcase returned", name));
            }
        }
        self.destroy(cx);
    }

    /// `CTSFlag::Fall` (0x9df10), every 0.1 s while flying: lands when it rests on the ground
    /// and is slower than `3 * slow` (sound, level pitch and roll, `OnGround`), returns home when
    /// its 20 s are up, and otherwise hurts what is 0.1 s ahead of it, then runs the slow-motion
    /// physics (`CTSPhysicObject::WaitThink`).
    fn fall(&mut self, cx: &mut Cx) {
        let me = cx.edict;
        let arith = crate::rules::cvars::pm_arith();
        // `this->GiveSlowMul()` only on the ground, as TS's `&&` asks it.
        let landed = cx.pev().flags & FL_ONGROUND as i32 != 0 && {
            let slow = crate::ham::give_slow_mul(me, || give_slow_mul(me.v().fuser1));
            crate::weapons::projectiles::fall_landed(arith, cx.pev().velocity, slow)
        };
        if landed {
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
        let arith = crate::rules::cvars::pm_arith();
        let end = crate::weapons::projectiles::fall_trace_end(arith, origin, velocity);
        let tr = e.trace_line(origin, end, DONT_IGNORE_MONSTERS, me);
        // A null hit is the world in TS, which takes no damage.
        if tr.flFraction >= 1.0 || tr.pHit.is_null() || rules::has_player(tr.pHit) {
            return;
        }
        let (threshold, dmg) = impact(arith, velocity);
        if crate::weapons::projectiles::impact_hits(tr.pHit.v(), threshold) {
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
        cx.pev().velocity[2] = (cx.pev().velocity[2] as f64 * 0.3) as f32;
        let slow = crate::ham::give_slow_mul(me, || give_slow_mul(me.v().fuser1));
        let v = cx.pev();
        let (hspeed, speed) = crate::weapons::bounce_speeds(crate::rules::cvars::pm_arith(), v.velocity);
        if v.flags & FL_ONGROUND as i32 == 0 {
            if bounce_drops_loudly(speed, slow) && e.globals().trace_plane_normal[2] as f64 > 0.98 {
                let pitch = 100 + e.random_long(0, 10);
                emit_sound_slow(me, CHAN_STATIC, c"items/weapondrop1.wav", 1.0, ATTN_NORM, 0, pitch, 0.5);
            }
        } else {
            let k = bounce_ground_damping(hspeed, slow);
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
}

impl Entity for Flag {
    /// `CBaseEntity::Save` / `Restore` (`CTSFlag`'s own state isn't saved), both pointers set.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut Fields) {
        super::save_base(cx, f, &mut self.funcs);
    }

    /// `CTSFlag::Precache` (0x9db94).
    fn restored(&mut self, _cx: &mut Cx) {
        engine().precache_model(MODEL);
    }

    /// `CTSFlag::Spawn` (0x9d978): a flying, zero-size `SOLID_BBOX` case with the team's glow,
    /// due home in 20 s (`FlagFall` clears that for a case at home). It is linked where it is
    /// (the world origin for a new case: `CreateFlag`'s callers then set `pev->origin` without
    /// relinking).
    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        let e = cx.edict;
        self.phys = SlowPhysics::default();
        eng.precache_model(MODEL);
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
        self.thrown = false;
        self.on_ground = false;
        self.funcs = BaseFuncs { think: true, touch: true, ..BaseFuncs::default() };
        // `time + 0.1` with the `double` (mp.dll 0x100721c4), as in `Fall` and `WaitThink`.
        cx.pev().nextthink = plus_double(cx.time(), 0.1);
        let v = cx.pev();
        v.iuser4 = 2;
        v.renderamt = 5.0;
        v.renderfx = kRenderFxGlowShell as i32;
        v.rendercolor = team_color(self.status.team);
        self.destroyed = false;
    }

    /// `Fall` while flying, `CTSFlag::WaitThink` (0x9e1b8) once landed; both every 0.1 s.
    fn think(&mut self, cx: &mut Cx) {
        if !self.funcs.think {
            return;
        }
        // `time + 0.1` with the `double` (`Fall` 0x9df38, mp.dll 0x1007267a; `WaitThink`
        // 0x1007289a), issue #150.
        cx.pev().nextthink = plus_double(cx.time(), 0.1);
        if !self.on_ground {
            self.fall(cx);
        } else if self.return_at != 0.0 && self.return_at < cx.time() {
            self.respawn_flag(cx);
        }
    }

    /// `BounceTouch` while flying (a flying case is only taken with +use), `CTSFlag::WaitTouch`
    /// (0x9e604) once landed.
    fn touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        if !self.funcs.touch {
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

    /// `CTSGameRules::RespawnEntities` (0x77764) on a round restart.
    pub fn round_respawn(&mut self, cx: &mut Cx) {
        self.respawn_flag(cx);
    }

    /// `CTSFlag::PlayerTakeFlag` (0x9e330), shared by the touch and +use. `p` is the player's
    /// object when the caller already holds it; otherwise it is looked up.
    fn player_take_flag(&mut self, cx: &mut Cx, other: *mut edict_t, p: Option<(&mut Player, &mut Cx)>) {
        // A live, visible player (TS: the low byte of `pev->effects` taken as signed, so
        // EF_NODRAW 0x80, e.g. observers).
        if self.destroyed || !rules::has_player(other) || !rules::is_alive(other) || other.v().effects & EF_NODRAW as i32 != 0 {
            return;
        }
        let team = team_index_of(other);
        if self.status.team == -1 || team != self.status.team {
            let took = match p {
                Some((p, pcx)) => take_flag(p, pcx, &self.status),
                None => crate::player::with_player(other, |p, pcx| take_flag(p, pcx, &self.status)).unwrap_or(false),
            };
            if !took {
                return;
            }
            // A case taken from home closes its zone for captures until it is back (`FlagFall`).
            let home = self.status.home.get();
            if !self.thrown && !home.is_null() {
                crate::world::with::<Briefcase, _>(home, |b| b.active = false);
            }
            self.destroy(cx);
        } else {
            let home = self.status.home.get();
            if !self.thrown || home.is_null() {
                return;
            }
            // The owning team returns its thrown case: its home drops a new one 1 s later (which
            // sends `TSFlag` 0); the bonus needs a second player.
            schedule_flag_fall(home, 1.0);
            self.destroy(cx);
            emit_sound_slow(other, CHAN_ITEM, c"items/gunpickup2.wav", 1.0, ATTN_NORM, 0, 100, 0.5);
            if rules::slot_players().len() > 1 {
                let mut p = p.map(|(p, _)| p);
                rules::add_frags(other, p.as_deref_mut(), 5.0);
                msg::ts_message_all([0, 255, 0], 4, 30, &shared::bformat!("{} returned the briefcase! 5 #TS_Frg", rules::netname(other)));
                match p {
                    Some(p) => score_info_of(other, p),
                    None => score_info(other),
                }
            }
        }
    }
}

/// `CBasePlayer::TakeFlag` (0x93830): the case needs 20 free slots and one case at most. Its
/// status moves to the player, which is slowed, plays the pickup sound, sends `TSFlag` 1,
/// announces the pickup unless it was the case's last holder, and logs it.
fn take_flag(p: &mut Player, pcx: &mut Cx, status: &FlagStatus) -> bool {
    if p.inv.sv.free_slots < FLAG_SLOTS || p.ts.flag.is_some() {
        return false;
    }
    let mut st = status.clone();
    p.inv.sv.free_slots -= FLAG_SLOTS;
    crate::weapons::sync_speed(p, pcx);
    let e = pcx.edict;
    emit_sound_slow(e, CHAN_ITEM, c"items/gunpickup2.wav", 1.0, ATTN_NORM, 0, 100, 0.5);
    let idx = engine().index_of(e);
    send_flag(1, st.id, st.team, idx);
    if idx != st.last_holder {
        let name = rules::netname(e);
        msg::ts_message_all([0, 255, 0], 4, 30, &shared::bformat!("{} picked up a briefcase containing {}!", name, st.contents));
    }
    st.last_holder = idx;
    log_flag(e, "aquired_briefcase", &st.contents);
    p.ts.flag = Some(st);
    true
}

/// `CTSFlag::CreateFlag` (0x9d874): a new `ts_flag` with `status`, spawned where the fresh
/// entity is (the world origin). With a thrower it is owned by it (so it passes through it until
/// it lands) and runs in its time scale; without one (`FlagFall`) no owner and scale 1.
fn create_flag(thrower: Option<(*mut edict_t, f32)>, status: FlagStatus) -> *mut edict_t {
    let eng = engine();
    let e = eng.create_named_entity(c"ts_flag");
    if e.is_null() || !e.has_private_data() {
        msg::text_all(shared::protocol::rules::hud_dest::TALK, "WARNING:cannot create flag!\n");
        return std::ptr::null_mut();
    }
    if crate::world::with::<Flag, _>(e, |f| f.status = status).is_none() {
        return e;
    }
    crate::ham::spawn(e);
    crate::world::dispatch(e, |ent, cx| {
        let Some(f) = (ent as &mut dyn std::any::Any).downcast_mut::<Flag>() else { return };
        f.thrown = thrower.is_some();
        let v = cx.pev();
        match thrower {
            Some((owner, time_scale)) => {
                v.owner = owner;
                v.fuser1 = time_scale;
            }
            None => {
                v.owner = std::ptr::null_mut();
                v.fuser1 = 1.0;
            }
        }
    });
    e
}

/// `HaveFlag`: the player carries a case.
pub fn has_flag(e: *mut edict_t) -> bool {
    rules::other(e, |p| p.ts.flag.is_some()).unwrap_or(false)
}

/// `CBasePlayer::DropFlag(captured)` (0x9334c). Either way the carrier gets its 20 slots back and
/// loses the status. A captured case just goes (the capture scheduled its home's `FlagFall`).
/// Otherwise a new case is created (`CreateFlag`) and thrown from the eyes: along `v_angle` at
/// [`aimed_throw_speed`] while alive, or in a random direction (pitch −60..60, yaw −180..180) at
/// 60..160 when dead ([`throw`]); its origin is set without relinking. Then the log line and
/// `TSFlag` 2.
pub fn drop_flag(p: &mut Player, cx: &mut Cx, captured: bool) {
    let Some(status) = p.ts.flag.clone() else { return };
    p.inv.sv.free_slots += FLAG_SLOTS;
    crate::weapons::sync_speed(p, cx);
    if !captured {
        let e = engine();
        let thrower = cx.edict;
        let time_scale = crate::weapons::thrower_time_scale(p, cx.pev());
        let flag = create_flag(Some((thrower, time_scale)), status.clone());
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
        if !flag.is_null() {
            let fv = flag.v();
            fv.origin = origin;
            fv.angles = [0.0; 3];
            fv.avelocity[1] = e.random_long(-300, 300) as f32;
            fv.avelocity[2] = e.random_long(-300, 300) as f32;
            fv.velocity = velocity;
        }
        log_flag(thrower, "dropped_briefcase", &status.contents);
        send_flag(2, status.id, status.team, cx.index());
    }
    p.ts.flag = None;
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
    /// `cash_for_winner` / `cash_for_loser` (`victorycash` / `defeatcash`): saved, otherwise
    /// unused.
    cash_winner: i32,
    cash_loser: i32,
}

impl<const SCORE: bool> Entity for WinGiver<SCORE> {
    /// `CTSWinGiver::KeyValue` (0x8bdfc; `CTSScoreGiver::KeyValue` 0x8c090 calls it).
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        // `strtol(value, 0, 10)` for each.
        let v = crate::rules::mapcycle::atoi(&value.to_string_lossy());
        match key.to_bytes() {
            b"winnerteam" => self.winner = v,
            b"callsnumber" => self.calls_needed = v,
            b"victorycash" => self.cash_winner = v,
            b"defeatcash" => self.cash_loser = v,
            _ => return false,
        }
        true
    }

    /// `CTSWinGiver::Save` / `Restore` (`ts_scoregiver` too): `cash_for_winner` (`+0x6c`),
    /// `cash_for_loser` (`+0x70`), `winnerteam` (`+0x74`), `numberofcall` (`+0x78`, the
    /// `callsnumber` key). `Use` is a virtual; `Spawn` clears the pointers.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut Fields) {
        f.base_funcs(cx, BaseFuncs::default());
        f.table(&crate::save::tables::CTSWinGiver);
        f.int(&mut self.cash_winner);
        f.int(&mut self.cash_loser);
        f.int(&mut self.winner);
        f.int(&mut self.calls_needed);
    }

    /// `CTSWinGiver::Spawn` (0x8bf10) / `CTSScoreGiver::Spawn` (0x8c0b8).
    fn spawn(&mut self, cx: &mut Cx) {
        let v = cx.pev();
        v.effects = EF_NODRAW as i32;
        v.solid = SOLID_NOT as i32;
        v.flags = 0;
        v.movetype = MOVETYPE_NONE as i32;
        cx.think_in(10.0);
    }

    /// `CTSWinGiver::Use` (0x8bfb0; latched until `EnableObjective`: `InitObjectives`, an objective
    /// pointer) / `CTSScoreGiver::Use` (0x8c158): after `callsnumber` calls, `TeamWin(winnerteam,
    /// (int)value, restart, isScore)` (a wingiver always asks for a restart, a scoregiver with
    /// flag 1; [`ServerRules::giver_team_win`](crate::gameplay::ServerRules::giver_team_win)), then
    /// `message`.
    fn use_(&mut self, cx: &mut Cx, _a: *mut edict_t, _c: *mut edict_t, _k: UseType, value: f32) {
        if self.used && !SCORE {
            return;
        }
        self.calls += 1;
        if self.calls < self.calls_needed {
            return;
        }
        let restart = !SCORE || cx.pev().spawnflags & 1 != 0;
        let win =
            GiverWin { team: self.winner, value, restart, is_score: SCORE, victory_cash: self.cash_winner, defeat_cash: self.cash_loser };
        crate::gameplay::active().server.giver_team_win(&win);
        super::objective::hud_line(cx.pev().message);
        self.calls = 0;
        if !SCORE {
            self.used = true;
        }
    }
}

/// What a win or score giver hands `TeamWin` ([`ServerRules::giver_team_win`](crate::gameplay::ServerRules::giver_team_win)).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GiverWin {
    /// `winnerteam`.
    pub team: i32,
    /// The `Use` value.
    pub value: f32,
    pub restart: bool,
    pub is_score: bool,
    /// `victorycash` / `defeatcash`.
    pub victory_cash: i32,
    pub defeat_cash: i32,
}

/// A win or score giver hasn't fired yet (its latch, reset by `EnableObjective`); `None` for
/// anything else.
pub fn giver_active(e: *mut edict_t) -> Option<bool> {
    world::with_lent::<WinGiver<false>, _>(e, |w| !w.used).or_else(|| world::with_lent::<WinGiver<true>, _>(e, |w| !w.used))
}

/// `CTSWinGiver::EnableObjective` on `ent` if it is a wingiver or scoregiver: the call counter and
/// the latch start over (gamerules-entities.md §3.2): `InitObjectives`, an objective pointer, or a
/// plugin (`crate::ham`).
pub fn enable_objective(ent: &mut dyn Entity) {
    let any = ent as &mut dyn std::any::Any;
    if let Some(w) = any.downcast_mut::<WinGiver<false>>() {
        (w.calls, w.used) = (0, false);
    } else if let Some(w) = any.downcast_mut::<WinGiver<true>>() {
        (w.calls, w.used) = (0, false);
    }
}

/// What `TeamWin` gives each player: the winners `cash` (`cashforwin` when 0), everyone else
/// `cashforlose`; and whether they lost (`TSHVoice`).
fn team_win_cash(winner: i32, player_team: i32, cash: i32) -> (i32, bool) {
    let win = if cash == 0 { crate::rules::cvars::CASH_FOR_WIN } else { cash };
    if winner != player_team { (crate::rules::cvars::CASH_FOR_LOSE, true) } else { (win, false) }
}

/// `CTSGameRules::TeamWin(team, cash, restart, isScore)` (0x78a08). With the round timer on and
/// not a score, `RoundTime` (`(int)(end - time)`, state 2); a restart sets the round restart in
/// 3 s and clears the round end (only LMS DM's `Think` acts on it). Fewer than two players
/// (`CountPlayers`): every team back to `initial_cash` and score 0, "Waiting for other
/// players.". Otherwise each `UTIL_PlayerByIndex` + `IsPlayer()` player gets its cash and
/// `TSHVoice`, then `team` 2 is a draw, any other team scores ("%s Win!" / "%s Score!") and the
/// log line. `TeamScore` only in round modes 2/3.
pub fn team_win(team: i32, cash: i32, restart: bool, is_score: bool) {
    let eng = engine();
    let now = eng.time();
    let (timer, end, mode) = state::with(|g| (g.round_timer, g.round_end, g.mode));
    if timer && !is_score {
        let rt = RoundTime { secs: crate::rules::frame::round_time_byte(f64::from(end) - f64::from(now)), state: 2 };
        msg::all(Msg::RoundTime, |w| rt.write(w));
    }
    if restart {
        state::with(|g| {
            g.round_restart = now + 3.0;
            g.round_end = 0.0;
        });
    }
    let players = rules::slot_players();
    if players.len() < 2 {
        state::with(|g| {
            for t in &mut g.teams {
                t.cash = crate::rules::cvars::INITIAL_CASH;
                t.score = 0;
            }
        });
        msg::ts_message_all([0, 0, 0], 8, 30, "Waiting for other players.");
    } else {
        for o in players {
            let (amount, lost) = team_win_cash(team, team_index_of(o), cash);
            // A player whose own call fired the giver gets it at once (lent), any other one held
            // by a dispatch once that returns.
            crate::world::run_now_or_defer(o, move || {
                rules::other(o, |p| lifecycle::give_cash(p, amount));
            });
            msg::one(Msg::TsHVoice, o, |w| TsHVoice { lost: lost as u8 }.write(w));
        }
        if team == 2 {
            msg::ts_message_all([0, 0, 0], 8, 30, "Game Draw");
            msg::log("Round draw\n");
        } else {
            // Guard: TS indexes its four-team table with any `winnerteam`, writing outside it for
            // one that isn't 0..3; the port scores and announces only a real team.
            let won = state::with(|g| {
                let t = usize::try_from(team).ok().and_then(|i| g.teams.get_mut(i))?;
                t.score += 1;
                Some((t.name.clone(), t.score))
            });
            if let Some((name, score)) = won {
                msg::ts_message_all([0, 0, 0], 8, 30, &shared::bformat!("{} {}", name, if is_score { "Score!" } else { "Win!" }));
                // TS's format has two `%d`s it passes nothing for; the port logs the score and the
                // team's player count.
                let members = rules::slot_players().into_iter().filter(|&p| team_index_of(p) == team).count();
                msg::log(&shared::bformat!("Team \"{}\" scored \"{}\" with \"{}\" players\n", name, score, members));
            }
        }
    }
    if matches!(mode, state::Mode::RoundTdm | state::Mode::Objective) {
        teams::send_team_scores();
    }
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
        for arith in [Arith::Gcc, Arith::Msvc] {
            let (threshold, dmg) = impact(arith, [300.0, 0.0, 400.0]);
            assert_eq!(threshold.f(), 15000.0);
            assert!((dmg - 15.0).abs() < 1e-4, "{dmg}");
        }
    }

    #[test]
    fn log_lines_repeat_the_user_id() {
        assert_eq!(
            flag_log_line(b"Neo", 7, b"STEAM_0:1:2", "aquired_briefcase", "two elephants"),
            b"\"Neo<7><STEAM_0:1:2><7>\" triggered \"aquired_briefcase\" (contents \"two elephants\")\n"
        );
    }

    #[test]
    fn team_win_cash_goes_to_both_sides() {
        // The Use value for the winners, cashforwin when it is 0; cashforlose for the rest.
        assert_eq!(team_win_cash(1, 1, 500), (500, false));
        assert_eq!(team_win_cash(1, 1, 0), (10000, false));
        assert_eq!(team_win_cash(1, 0, 500), (1200, true));
        // A draw (2) has no winners in a two-team game.
        assert_eq!(team_win_cash(2, 0, 0), (1200, true));
        assert_eq!(team_win_cash(2, -1, 0), (1200, true));
    }

    #[test]
    fn unknown_teams_glow_green() {
        assert_eq!(team_color(-1), [145.0, 215.0, 140.0]);
        assert_eq!(team_color(3), [145.0, 215.0, 140.0]);
        assert_eq!(team_color(1), [200.0, 90.0, 70.0]);
    }
}

#[cfg(test)]
mod precision_tests {
    use super::*;
    use shared::pm::Arith;

    fn f(b: [u32; 3]) -> [f32; 3] {
        b.map(f32::from_bits)
    }

    fn b(v: [f32; 3]) -> [u32; 3] {
        v.map(f32::to_bits)
    }

    /// The flag's impact damage (issue #150): mp.dll's unrounded `Length() * 0.03f`, the .so's
    /// `speed * 30 * 0.001f` (which the port computed before).
    #[test]
    fn flag_impact_per_build() {
        let v = f([0x4427_f24a, 0x431b_b090, 0xc43d_9372]);
        assert_eq!(impact(Arith::Msvc, v).1.to_bits(), 0x41f5_fe0e);
        assert_eq!(impact(Arith::Gcc, v).1.to_bits(), 0x41f5_fe0f);
        let _ = b(v);
    }
}
