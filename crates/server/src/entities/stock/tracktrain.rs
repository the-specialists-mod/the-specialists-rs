//! `func_tracktrain`, `func_traincontrols` and `path_track` (SDK plats.cpp / pathcorner.cpp; TS
//! keeps the SDK code, including the `events/train.sc` pitch updates).
//!
//! Players drive a train by standing inside its control volume (`func_traincontrols`, or the
//! train's own box) and pressing +use; the player side is in `drive.rs`. Each `path_track`
//! links to its neighbours in `Activate`, which `ServerActivate` runs once the map has spawned
//! (and again after a restore).

use super::base::*;
use super::explode::emit_sound_slow;
use super::trains::ent_handle;
use crate::entities::fire_targets;
use crate::save::{BaseFuncs, Fields, tables};
use crate::world::{self, Cx, DamageInfo, Entity, UseType};
use goldsrc::Global;
use goldsrc::edict::EdictPtr;
use goldsrc::ext::entities::{ATTN_NORM, EHandle, Vec3, vec};
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::cell::Cell;
use std::ffi::CStr;

pub const SF_TRACKTRAIN_NOPITCH: i32 = 0x0001;
pub const SF_TRACKTRAIN_NOCONTROL: i32 = 0x0002;
pub const SF_TRACKTRAIN_FORWARDONLY: i32 = 0x0004;
pub const SF_TRACKTRAIN_PASSABLE: i32 = 0x0008;

pub const SF_PATH_DISABLED: i32 = 0x0001;
pub const SF_PATH_FIREONCE: i32 = 0x0002;
pub const SF_PATH_ALTREVERSE: i32 = 0x0004;
pub const SF_PATH_DISABLE_TRAIN: i32 = 0x0008;
pub const SF_PATH_ALTERNATE: i32 = 0x8000;

// ---- path_track ---------------------------------------------------------------------------------------

#[derive(Default)]
pub struct PathTrack {
    next: EHandle,
    prev: EHandle,
    alt: EHandle,
    alt_name: string_t,
}

fn is_path(e: *mut edict_t) -> bool {
    !e.is_null() && s(e.v().classname) == c"path_track"
}

fn sf(e: *mut edict_t) -> i32 {
    e.v().spawnflags
}

fn origin(e: *mut edict_t) -> Vec3 {
    e.v().origin
}

fn data(e: *mut edict_t) -> (EHandle, EHandle, EHandle) {
    world::with::<PathTrack, _>(e, |p| (p.next, p.prev, p.alt)).unwrap_or_default()
}

impl PathTrack {
    /// `CPathTrack::SetPrevious` (0xd1620): `prev` becomes the previous node unless it's the
    /// node's alternate path.
    fn set_previous(&mut self, prev: *mut edict_t) {
        if prev.is_null() {
            return;
        }
        if s(prev.v().targetname) != s(self.alt_name) {
            self.prev = EHandle::new(prev);
        }
    }

    /// `CPathTrack::Link` (0xd1274): `m_pnext` from `target` and `m_paltpath` from `altpath`, each
    /// telling the node it links to that this one comes before it. Both go through
    /// `CPathTrack::Instance` (0xd1c30), so only `path_track`s are linked.
    fn link(&mut self, cx: &mut Cx) {
        let eng = engine();
        let me = cx.edict;
        let target = cx.pev().target;
        if target != 0 {
            let t = eng.find_by_target_name(std::ptr::null_mut(), s(target));
            if t.is_null() || eng.index_of(t) == 0 {
                eng.alert(ALERT_TYPE::at_console, &format!("Dead end link {}\n", s(target).to_string_lossy()));
            } else if is_path(t) {
                self.next = EHandle::new(t);
                self.link_previous(t, me);
            }
        }
        if self.alt_name != 0 {
            let t = eng.find_by_target_name(std::ptr::null_mut(), s(self.alt_name));
            if !t.is_null() && eng.index_of(t) != 0 && is_path(t) {
                self.alt = EHandle::new(t);
                self.link_previous(t, me);
            }
        }
    }

    /// `SetPrevious` on node `t`, which is this one (taken out for the call) when a node links to
    /// itself.
    fn link_previous(&mut self, t: *mut edict_t, me: *mut edict_t) {
        if t == me {
            self.set_previous(me);
        } else {
            world::with::<PathTrack, _>(t, |p| p.set_previous(me));
        }
    }
}

fn get_next(e: *mut edict_t) -> *mut edict_t {
    let (next, _, alt) = data(e);
    let f = sf(e);
    if alt.is_valid() && f & SF_PATH_ALTERNATE != 0 && f & SF_PATH_ALTREVERSE == 0 {
        return alt.get();
    }
    next.get()
}

fn get_prev(e: *mut edict_t) -> *mut edict_t {
    let (_, prev, alt) = data(e);
    let f = sf(e);
    if alt.is_valid() && f & SF_PATH_ALTERNATE != 0 && f & SF_PATH_ALTREVERSE != 0 {
        return alt.get();
    }
    prev.get()
}

fn valid(e: *mut edict_t, test: bool) -> *mut edict_t {
    if e.is_null() || (test && sf(e) & SF_PATH_DISABLED != 0) { std::ptr::null_mut() } else { e }
}

fn project(start: *mut edict_t, end: *mut edict_t, org: &mut Vec3, dist: f32) {
    if !start.is_null() && !end.is_null() {
        let dir = vec::normalize(vec::sub(origin(end), origin(start)));
        *org = vec::mad(origin(end), dir, dist);
    }
}

/// `CPathTrack::LookAhead`: where `dist` units along the path from `org` lands; returns the
/// path node reached (null at a dead end).
fn look_ahead(this: *mut edict_t, org: &mut Vec3, dist: f32, mv: bool) -> *mut edict_t {
    let original = dist;
    let mut cur = this;
    let mut pos = *org;
    let mut dist = dist;
    if dist < 0.0 {
        dist = -dist;
        while dist > 0.0 {
            let dir = vec::sub(origin(cur), pos);
            let length = vec::length(dir);
            if length == 0.0 {
                if valid(get_prev(cur), mv).is_null() {
                    if !mv {
                        project(get_next(cur), cur, org, dist);
                    }
                    return std::ptr::null_mut();
                }
                cur = get_prev(cur);
            } else if length > dist {
                *org = vec::mad(pos, dir, dist / length);
                return cur;
            } else {
                dist -= length;
                pos = origin(cur);
                *org = pos;
                if valid(get_prev(cur), mv).is_null() {
                    return std::ptr::null_mut();
                }
                cur = get_prev(cur);
            }
        }
        *org = pos;
    } else {
        while dist > 0.0 {
            let next = valid(get_next(cur), mv);
            if next.is_null() {
                if !mv {
                    project(get_prev(cur), cur, org, dist);
                }
                return std::ptr::null_mut();
            }
            let dir = vec::sub(origin(next), pos);
            let length = vec::length(dir);
            if length == 0.0 && valid(get_next(next), mv).is_null() {
                if dist == original {
                    return std::ptr::null_mut();
                }
                return cur;
            }
            if length > dist {
                *org = vec::mad(pos, dir, dist / length);
                return cur;
            }
            dist -= length;
            pos = origin(next);
            cur = next;
            *org = pos;
        }
        *org = pos;
    }
    cur
}

/// `CPathTrack::Nearest` (0xd1a9c): the node of `this`'s path (following `GetNext`) closest to
/// `pos` in 2D, starting from `this`. After 10000 nodes without ending or coming back to `this`,
/// "Bad sequence of path_tracks" and null.
pub fn nearest(this: *mut edict_t, pos: Vec3) -> *mut edict_t {
    let dist2d = |e: *mut edict_t| {
        let o = origin(e);
        let (dx, dy) = ((pos[0] - o[0]) as f64, (pos[1] - o[1]) as f64);
        (dy * dy + dx * dx).sqrt() as f32
    };
    let mut best = this;
    let mut closest = dist2d(this);
    let mut cur = get_next(this);
    if cur.is_null() || cur == this {
        return this;
    }
    for _ in 1..10000 {
        let d = dist2d(cur);
        if d < closest {
            best = cur;
            closest = d;
        }
        cur = get_next(cur);
        if cur.is_null() || cur == this {
            return best;
        }
    }
    engine().alert(ALERT_TYPE::at_error, &format!("Bad sequence of path_tracks from {}", s(this.v().targetname).to_string_lossy()));
    std::ptr::null_mut()
}

/// `m_pnext` / `m_pprevious` of a `path_track` (null for anything else).
pub fn path_links(e: *mut edict_t) -> (*mut edict_t, *mut edict_t) {
    let (next, prev, _) = data(e);
    (next.get(), prev.get())
}

/// `CPathTrack::GetNext` (0xd15d0) of a `path_track` (null for anything else).
pub fn path_next(e: *mut edict_t) -> *mut edict_t {
    if world::with::<PathTrack, _>(e, |_| ()).is_none() {
        return std::ptr::null_mut();
    }
    get_next(e)
}

impl Entity for PathTrack {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        if key == c"altpath" {
            self.alt_name = alloc(value);
            return true;
        }
        false
    }

    /// `CPathTrack::Save` / `Restore` (0xd10a4 / 0xd10f4). `m_length` (+0x64) is never written
    /// (always 0); `Use` is a virtual and `Precache` is `CBaseEntity`'s.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut Fields) {
        f.base_funcs(cx, BaseFuncs::default());
        f.table(&tables::CPathTrack);
        f.zero();
        ent_handle(f, &mut self.next);
        ent_handle(f, &mut self.alt);
        ent_handle(f, &mut self.prev);
        f.string(&mut self.alt_name);
    }

    fn spawn(&mut self, cx: &mut Cx) {
        cx.pev().solid = SOLID_TRIGGER as i32;
        engine().set_size(cx.edict, [-8.0; 3], [8.0; 3]);
        self.next = EHandle::NULL;
        self.prev = EHandle::NULL;
    }

    /// `CPathTrack::Activate` (0xd143c): a named node links to its neighbours.
    fn activate(&mut self, cx: &mut Cx) {
        if cx.pev().targetname != 0 {
            self.link(cx);
        }
    }

    fn use_(&mut self, cx: &mut Cx, _a: *mut edict_t, _c: *mut edict_t, kind: UseType, _v: f32) {
        let v = cx.pev();
        let bit = if self.alt.is_valid() { SF_PATH_ALTERNATE } else { SF_PATH_DISABLED };
        let on = v.spawnflags & bit == 0;
        if should_toggle(kind, on) {
            if on {
                v.spawnflags |= bit;
            } else {
                v.spawnflags &= !bit;
            }
        }
    }
}

// ---- func_tracktrain ------------------------------------------------------------------------------------

const TRAIN_STARTPITCH: i32 = 60;
const TRAIN_MAXPITCH: i32 = 200;
const TRAIN_MAXSPEED: i32 = 1000;
/// `EMIT_SOUND_DYN`'s slow-motion pitch factor in every `CFuncTrackTrain` sound.
const TRAIN_SLOW: f32 = 0.5;

/// `CFuncTrackTrain::UpdateSound` (0xd46bc)'s pitch: integer maths on the truncated speed,
/// `60 + |speed| * 140 / 1000` (C's wrapping `int` arithmetic).
fn train_pitch(speed: f32) -> i32 {
    let speed = (speed as i32).wrapping_abs();
    speed.wrapping_mul(TRAIN_MAXPITCH - TRAIN_STARTPITCH) / TRAIN_MAXSPEED + TRAIN_STARTPITCH
}

/// The pitch byte of the `events/train.sc` update: `(int)(pitch * 0.1)`, 6 bits.
fn train_event_pitch(pitch: i32) -> i32 {
    (pitch as f64 * 0.1) as i32 & 0x3f
}
/// `event_flags.h`: `PLAYBACK_EVENT_FULL(10, ...)` in `StopSound` (0xd4610) and `UpdateSound` (0xd46bc).
/// `(int)(pev->speed * 4) / (int)m_speed` from `CFuncTrackTrain::Use` (0xd437c). Guard: TS
/// traps (SIGFPE) dividing by 0 or `INT_MIN / -1`; the port divides by 1 instead.
fn speed_notch(n: i32, max: i32) -> i32 {
    n.checked_div(max).unwrap_or(n)
}

const FEV_RELIABLE: i32 = 1 << 1;
const FEV_UPDATE: i32 = 1 << 3;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum TrainThink {
    #[default]
    None,
    Find,
    Next,
    DeadEnd,
    NearestPath,
}

#[derive(Default)]
pub struct TrackTrain {
    /// `m_ppath` (+0x64).
    path: EHandle,
    length: f32,
    height: f32,
    speed: f32,
    start_speed: f32,
    dir: f32,
    sounds: i32,
    volume: f32,
    bank: f32,
    old_speed: f32,
    sound_playing: bool,
    control_mins: Vec3,
    control_maxs: Vec3,
    adjust_pitch_event: u16,
    think: TrainThink,
}

/// `CFuncTrackTrain::OnControls` (0xd5100): is `pv` inside the train's control volume (in the
/// train's local frame)? Always false with `SF_TRACKTRAIN_NOCONTROL` or for anything that isn't
/// a `func_tracktrain`.
pub fn on_controls(train: *mut edict_t, pv: &entvars_t) -> bool {
    let Some((mins, maxs)) = world::with::<TrackTrain, _>(train, |t| (t.control_mins, t.control_maxs)) else { return false };
    let tv = train.v();
    if tv.spawnflags & SF_TRACKTRAIN_NOCONTROL != 0 {
        return false;
    }
    let offset = vec::sub(pv.origin, tv.origin);
    let (fwd, right, up) = engine().make_vectors(tv.angles);
    let local = [vec::dot(offset, fwd), -vec::dot(offset, right), vec::dot(offset, up)];
    (0..3).all(|i| local[i] >= mins[i] && local[i] <= maxs[i])
}

/// The train whose think is firing targets (its edict, `m_ppath`, `m_length`): its object is
/// taken out of the world meanwhile, and a `func_trackautochange` it fires reads them.
static FIRING: Global<Cell<Option<Firing>>> = Global(Cell::new(None));

/// [`FIRING`]'s train: its edict, `m_ppath` and `m_length`.
type Firing = (*mut edict_t, EHandle, f32);

/// `m_ppath` and `m_length` of a `func_tracktrain` (`CFuncTrackTrain::Instance` 0xd56f4: none for
/// anything else).
pub fn train_path_length(train: *mut edict_t) -> Option<(*mut edict_t, f32)> {
    if let Some((e, path, length)) = FIRING.0.get()
        && e == train
    {
        return Some((path.get(), length));
    }
    world::with::<TrackTrain, _>(train, |t| (t.path.get(), t.length))
}

/// Sets a `func_tracktrain`'s `m_ppath`. A train firing targets doesn't read it again before its
/// think returns, so the write waits for that.
pub fn set_train_path(train: *mut edict_t, path: *mut edict_t) {
    let path = EHandle::new(path);
    world::run_now_or_defer(train, move || {
        world::with::<TrackTrain, _>(train, |t| t.path = path);
    });
}

/// `CFuncTrackTrain::NextThink` (0xd4154) on a train's entvars: `FL_ALWAYSTHINK` on or off and
/// `nextthink`.
pub fn train_next_think(v: &mut entvars_t, at: f32, always: bool) {
    if always {
        v.flags |= FL_ALWAYSTHINK as i32;
    } else {
        v.flags &= !(FL_ALWAYSTHINK as i32);
    }
    v.nextthink = at;
}

/// `CFuncTrackTrain::SetControls` (0xd503c): the control volume becomes `controls`' box, taken
/// relative to the train's spawn origin.
fn set_controls(train: *mut edict_t, controls: &entvars_t) {
    let offset = vec::sub(controls.origin, train.v().oldorigin);
    let (mins, maxs) = (vec::add(controls.mins, offset), vec::add(controls.maxs, offset));
    world::with::<TrackTrain, _>(train, |t| {
        t.control_mins = mins;
        t.control_maxs = maxs;
    });
}

/// `TrainSpeed` (0xd8660): the HUD notch for a speed: 5 back, 1 stopped, 2/3/4 for under a third,
/// under two thirds, and the rest of `max`. TS divides in double precision.
pub fn train_speed(speed: i32, max: i32) -> i32 {
    if speed < 0 {
        5
    } else if speed == 0 {
        1
    } else {
        let f = speed as f64 / max as f64;
        if f < 0.33 {
            2
        } else if f < 0.66 {
            3
        } else {
            4
        }
    }
}

// ---- func_traincontrols ---------------------------------------------------------------------------------

/// `func_traincontrols` (`CFuncTrainControls`, 0xd5c18): a brush that hands its box to the
/// `func_tracktrain` named by `target` as the control volume, then removes itself.
#[derive(Default)]
pub struct TrainControls {
    /// `SetThink(Find)`, from `Spawn` on.
    think: bool,
}

impl Entity for TrainControls {
    /// `CFuncTrainControls::Spawn` (0xd5c18); its think is [`train_controls_find`].
    fn spawn(&mut self, cx: &mut Cx) {
        self.think = true;
        let eng = engine();
        let v = cx.pev();
        v.solid = SOLID_NOT as i32;
        v.movetype = MOVETYPE_NONE as i32;
        set_own_model(cx);
        let v = cx.pev();
        let (mins, maxs, org) = (v.mins, v.maxs, v.origin);
        eng.set_size(cx.edict, mins, maxs);
        eng.set_origin(cx.edict, org);
        cx.pev().nextthink = eng.time();
    }

    /// TS's chain is `CBaseEntity`'s: `m_pfnThink` is `Find` (a restored one never finds its
    /// train: TS can't restore the pointer).
    fn save_fields(&mut self, cx: &mut Cx, f: &mut Fields) {
        if f.base_funcs(cx, BaseFuncs { think: self.think, ..Default::default() }).think {
            self.think = false;
        }
    }

    fn think(&mut self, cx: &mut Cx) {
        if self.think {
            train_controls_find(cx);
        }
    }
}

/// `CFuncTrainControls::Find` (0xd5ad4) on `cx`'s entity: the first `func_tracktrain` named by its
/// `target` takes its box as the control volume and the entity is removed. TS's
/// `CFuncTrackChange::Spawn` (0xd5d84) installs it as the track change's think too.
pub fn train_controls_find(cx: &mut Cx) {
    let eng = engine();
    let target = s(cx.pev().target);
    let mut e = std::ptr::null_mut();
    loop {
        e = eng.find_by_target_name(e, target);
        if e.is_null() || s(e.v().classname) == c"func_tracktrain" {
            break;
        }
    }
    if e.is_null() {
        eng.alert(ALERT_TYPE::at_console, &format!("No train {}\n", target.to_string_lossy()));
        return;
    }
    set_controls(e, cx.pev());
    crate::entities::remove(cx.edict);
}

fn fix(mut a: f32) -> f32 {
    while a < 0.0 {
        a += 360.0;
    }
    while a > 360.0 {
        a -= 360.0;
    }
    a
}

fn fixup(v: Vec3) -> Vec3 {
    [fix(v[0]), fix(v[1]), fix(v[2])]
}

/// `UTIL_AngleMod`.
fn angle_mod(a: f32) -> f32 {
    if a < 0.0 {
        a + 360.0 * ((a / 360.0) as i32 + 1) as f32
    } else if a >= 360.0 {
        a - 360.0 * ((a / 360.0) as i32) as f32
    } else {
        a
    }
}

/// `UTIL_AngleDistance`.
pub fn angle_distance(next: f32, cur: f32) -> f32 {
    let d = next - cur;
    if d < -180.0 {
        d + 360.0
    } else if d > 180.0 {
        d - 360.0
    } else {
        d
    }
}

/// `UTIL_ApproachAngle`.
pub fn approach_angle(target: f32, value: f32, speed: f32) -> f32 {
    let target = angle_mod(target);
    let value = angle_mod(value);
    let speed = speed.abs();
    let mut delta = target - value;
    if delta < -180.0 {
        delta += 360.0;
    } else if delta > 180.0 {
        delta -= 360.0;
    }
    if delta > speed {
        value + speed
    } else if delta < -speed {
        value - speed
    } else {
        target
    }
}

impl TrackTrain {
    fn next_think(cx: &mut Cx, at: f32, always: bool) {
        train_next_think(cx.pev(), at, always);
    }

    /// `FireTargets(target, this, this, USE_TOGGLE, 0)`, with `m_ppath` / `m_length` readable
    /// meanwhile ([`train_path_length`]).
    fn fire(&self, me: *mut edict_t, target: &CStr) {
        let outer = FIRING.0.replace(Some((me, self.path, self.length)));
        fire_targets(target, me, me, UseType::Toggle, 0.0);
        FIRING.0.set(outer);
    }

    /// `CFuncTrackTrain::Precache` (0xd5968).
    fn precache(&mut self, cx: &mut Cx) {
        let eng = engine();
        if self.volume == 0.0 {
            self.volume = 1.0;
        }
        let noise: Option<&'static CStr> = match self.sounds {
            1 => Some(c"plats/ttrain1.wav"),
            2 => Some(c"plats/ttrain2.wav"),
            3 => Some(c"plats/ttrain3.wav"),
            4 => Some(c"plats/ttrain4.wav"),
            5 => Some(c"plats/ttrain6.wav"),
            6 => Some(c"plats/ttrain7.wav"),
            _ => None,
        };
        cx.pev().noise = match noise {
            Some(n) => {
                eng.precache_sound(n);
                alloc(n)
            }
            None => 0,
        };
        eng.precache_sound(c"plats/ttrain_brake1.wav");
        eng.precache_sound(c"plats/ttrain_start1.wav");
        // `plats/train_use1.wav`, played by the player taking the controls, is ClientPrecache's.
        self.adjust_pitch_event = eng.precache_event(c"events/train.sc");
    }

    fn stop_sound(&mut self, cx: &mut Cx) {
        if self.sound_playing && cx.pev().noise != 0 {
            let encode = (self.sounds & 7) << 12;
            engine().playback_event(
                FEV_RELIABLE | FEV_UPDATE,
                cx.edict,
                self.adjust_pitch_event,
                0.0,
                [0.0; 3],
                [0.0; 3],
                0.0,
                0.0,
                encode,
                0,
                true,
                false,
            );
            emit_sound_slow(cx.edict, CHAN_ITEM, c"plats/ttrain_brake1.wav", self.volume, ATTN_NORM, 0, 100, TRAIN_SLOW);
        }
        self.sound_playing = false;
    }

    /// `CFuncTrackTrain::UpdateSound` (0xd46bc).
    fn update_sound(&mut self, cx: &mut Cx) {
        let noise = cx.pev().noise;
        if noise == 0 {
            return;
        }
        let pitch = train_pitch(cx.pev().speed);
        if !self.sound_playing {
            emit_sound_slow(cx.edict, CHAN_ITEM, c"plats/ttrain_start1.wav", self.volume, ATTN_NORM, 0, 100, TRAIN_SLOW);
            emit_sound_slow(cx.edict, CHAN_STATIC, s(noise), self.volume, ATTN_NORM, 0, pitch, TRAIN_SLOW);
            self.sound_playing = true;
        } else {
            let us_sound = (self.sounds & 7) << 12;
            let us_pitch = train_event_pitch(pitch) << 6;
            let us_volume = ((self.volume * 40.0) as i32) & 0x3f;
            engine().playback_event(
                FEV_RELIABLE | FEV_UPDATE,
                cx.edict,
                self.adjust_pitch_event,
                0.0,
                [0.0; 3],
                [0.0; 3],
                0.0,
                0.0,
                us_sound | us_pitch | us_volume,
                0,
                false,
                false,
            );
        }
    }

    fn next(&mut self, cx: &mut Cx) {
        let eng = engine();
        let mut time = 0.5;
        if cx.pev().speed == 0.0 {
            let msg = format!("TRAIN({}): Speed is 0\n", s(cx.pev().targetname).to_string_lossy());
            eng.alert(ALERT_TYPE::at_aiconsole, &msg);
            self.stop_sound(cx);
            return;
        }
        let path = self.path.get();
        if path.is_null() {
            let msg = format!("TRAIN({}): Lost path\n", s(cx.pev().targetname).to_string_lossy());
            eng.alert(ALERT_TYPE::at_aiconsole, &msg);
            self.stop_sound(cx);
            return;
        }
        self.update_sound(cx);
        let v = cx.pev();
        let mut next_pos = v.origin;
        next_pos[2] -= self.height;
        let pnext = look_ahead(path, &mut next_pos, v.speed * 0.1, true);
        next_pos[2] += self.height;
        v.velocity = vec::scale(vec::sub(next_pos, v.origin), 10.0);
        let mut front = v.origin;
        front[2] -= self.height;
        look_ahead(path, &mut front, if self.length > 0.0 { self.length } else { 100.0 }, false);
        front[2] += self.height;
        let delta = vec::sub(front, v.origin);
        let mut angles = eng.vec_to_angles(delta);
        angles[1] += 180.0;
        let mut angles = fixup(angles);
        v.angles = fixup(v.angles);
        if pnext.is_null() || (delta[0] == 0.0 && delta[1] == 0.0) {
            angles = v.angles;
        }
        let vx = if v.spawnflags & SF_TRACKTRAIN_NOPITCH == 0 { angle_distance(angles[0], v.angles[0]) } else { 0.0 };
        let vy = angle_distance(angles[1], v.angles[1]);
        v.avelocity[1] = vy * 10.0;
        v.avelocity[0] = vx * 10.0;
        if self.bank != 0.0 {
            let z = v.angles[2];
            v.avelocity[2] = if v.avelocity[1] < -5.0 {
                angle_distance(approach_angle(-self.bank, z, self.bank * 2.0), z)
            } else if v.avelocity[1] > 5.0 {
                angle_distance(approach_angle(self.bank, z, self.bank * 2.0), z)
            } else {
                angle_distance(approach_angle(0.0, z, self.bank * 4.0), z) * 4.0
            };
        }
        if !pnext.is_null() {
            if pnext != path {
                let fire = if cx.pev().speed >= 0.0 { pnext } else { path };
                self.path = EHandle::new(pnext);
                let fv = fire.v();
                if fv.message != 0 {
                    self.fire(cx.edict, s(fv.message));
                    if fv.spawnflags & SF_PATH_FIREONCE != 0 {
                        fv.message = 0;
                    }
                }
                let v = cx.pev();
                if fv.spawnflags & SF_PATH_DISABLE_TRAIN != 0 {
                    v.spawnflags |= SF_TRACKTRAIN_NOCONTROL;
                }
                if v.spawnflags & SF_TRACKTRAIN_NOCONTROL != 0 && fv.speed != 0.0 {
                    v.speed = fv.speed;
                    let msg = format!("TrackTrain {} speed to {:4.2}\n", s(v.targetname).to_string_lossy(), v.speed);
                    eng.alert(ALERT_TYPE::at_aiconsole, &msg);
                }
            }
            self.think = TrainThink::Next;
            let at = cx.pev().ltime + time;
            Self::next_think(cx, at, true);
        } else {
            self.stop_sound(cx);
            let v = cx.pev();
            v.velocity = vec::sub(next_pos, v.origin);
            v.avelocity = [0.0; 3];
            let distance = vec::length(v.velocity);
            self.old_speed = v.speed;
            v.speed = 0.0;
            if distance > 0.0 {
                time = distance / self.old_speed;
                v.velocity = vec::scale(v.velocity, self.old_speed / distance);
                self.think = TrainThink::DeadEnd;
                let at = cx.pev().ltime + time;
                Self::next_think(cx, at, false);
            } else {
                self.dead_end(cx);
            }
        }
    }

    /// `CFuncTrackTrain::DeadEnd` (0xd4ec8).
    fn dead_end(&mut self, cx: &mut Cx) {
        let eng = engine();
        let msg = format!("TRAIN({}): Dead end ", s(cx.pev().targetname).to_string_lossy());
        eng.alert(ALERT_TYPE::at_aiconsole, &msg);
        let mut track = self.path.get();
        if !track.is_null() {
            loop {
                let n = if self.old_speed < 0.0 { valid(get_prev(track), true) } else { valid(get_next(track), true) };
                if n.is_null() {
                    break;
                }
                track = n;
            }
        }
        let v = cx.pev();
        v.velocity = [0.0; 3];
        v.avelocity = [0.0; 3];
        if track.is_null() {
            eng.alert(ALERT_TYPE::at_aiconsole, "\n");
        } else {
            let msg = format!("at {}\n", s(track.v().targetname).to_string_lossy());
            eng.alert(ALERT_TYPE::at_aiconsole, &msg);
            let net = track.v().netname;
            if net != 0 {
                self.fire(cx.edict, s(net));
            }
        }
    }

    fn find(&mut self, cx: &mut Cx) {
        let eng = engine();
        let t = eng.find_by_target_name(std::ptr::null_mut(), s(cx.pev().target));
        if t.is_null() {
            return;
        }
        // `CPathTrack::Instance` (0xd1c30) gives NULL for anything but a `path_track`, so TS's
        // "func_track_train must be on a path of path_track" branch is unreachable.
        if !is_path(t) {
            self.path = EHandle::NULL;
            return;
        }
        self.path = EHandle::new(t);
        let mut next_pos = origin(t);
        next_pos[2] += self.height;
        let mut look = next_pos;
        look[2] -= self.height;
        look_ahead(t, &mut look, self.length, false);
        look[2] += self.height;
        let v = cx.pev();
        v.angles = eng.vec_to_angles(vec::sub(look, next_pos));
        v.angles[1] += 180.0;
        if v.spawnflags & SF_TRACKTRAIN_NOPITCH != 0 {
            v.angles[0] = 0.0;
        }
        eng.set_origin(cx.edict, next_pos);
        let at = cx.pev().ltime + 0.1;
        Self::next_think(cx, at, false);
        self.think = TrainThink::Next;
        cx.pev().speed = self.start_speed;
        self.update_sound(cx);
    }

    fn nearest_path(&mut self, cx: &mut Cx) {
        let eng = engine();
        let org = cx.pev().origin;
        let mut nearest = std::ptr::null_mut();
        let mut closest = 1024.0;
        let mut e = std::ptr::null_mut();
        loop {
            e = eng.find_in_sphere(e, org, 1024.0);
            if e.is_null() {
                break;
            }
            let ev = e.v();
            if crate::rules::ts_flags(e) & (FL_CLIENT | FL_MONSTER) as i32 == 0 && is_path(e) {
                let d = vec::length(vec::sub(org, ev.origin));
                if d < closest {
                    closest = d;
                    nearest = e;
                }
            }
        }
        if nearest.is_null() {
            eng.alert(ALERT_TYPE::at_console, "Can't find a nearby track !!!\n");
            self.think = TrainThink::None;
            return;
        }
        let msg = format!(
            "TRAIN: {}, Nearest track is {}\n",
            s(cx.pev().targetname).to_string_lossy(),
            s(nearest.v().targetname).to_string_lossy()
        );
        eng.alert(ALERT_TYPE::at_aiconsole, &msg);
        let n = get_next(nearest);
        if !n.is_null() && vec::length(vec::sub(org, origin(n))) < vec::length(vec::sub(org, origin(nearest))) {
            nearest = n;
        }
        self.path = EHandle::new(nearest);
        if cx.pev().speed != 0.0 {
            let at = cx.pev().ltime + 0.1;
            Self::next_think(cx, at, false);
            self.think = TrainThink::Next;
        }
    }
}

impl Entity for TrackTrain {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"wheels" => self.length = kv_f(value),
            b"height" => self.height = kv_f(value),
            b"startspeed" => self.start_speed = kv_f(value),
            b"sounds" => self.sounds = kv_i(value),
            b"volume" => self.volume = kv_i(value) as f32 * 0.1,
            b"bank" => self.bank = kv_f(value),
            _ => return false,
        }
        true
    }

    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        let v = cx.pev();
        self.speed = if v.speed == 0.0 { 100.0 } else { v.speed };
        v.speed = 0.0;
        v.velocity = [0.0; 3];
        v.avelocity = [0.0; 3];
        v.impulse = self.speed as i32;
        self.dir = 1.0;
        if v.target == 0 {
            eng.alert(ALERT_TYPE::at_console, "FuncTrain with no target");
        }
        v.solid = if v.spawnflags & SF_TRACKTRAIN_PASSABLE != 0 { SOLID_NOT } else { SOLID_BSP } as i32;
        v.movetype = MOVETYPE_PUSH as i32;
        set_own_model(cx);
        let v = cx.pev();
        let (mins, maxs, org) = (v.mins, v.maxs, v.origin);
        eng.set_size(cx.edict, mins, maxs);
        eng.set_origin(cx.edict, org);
        let v = cx.pev();
        v.oldorigin = v.origin;
        self.control_mins = v.mins;
        self.control_maxs = v.maxs;
        self.control_maxs[2] += 72.0;
        let at = v.ltime + 0.1;
        Self::next_think(cx, at, false);
        self.think = TrainThink::Find;
        self.precache(cx);
    }

    /// `CFuncTrackTrain::Save` / `Restore` (0xd3eec / 0xd3f3c). `Use` and `Blocked` are
    /// virtuals; `m_pfnThink` is `think`. `m_soundPlaying` (+0x94) isn't saved: a restored train
    /// starts silent.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut Fields) {
        if f.base_funcs(cx, BaseFuncs { think: self.think != TrainThink::None, ..Default::default() }).think {
            self.think = TrainThink::None;
        }
        f.table(&tables::CFuncTrackTrain);
        ent_handle(f, &mut self.path);
        f.float(&mut self.length);
        f.float(&mut self.height);
        f.float(&mut self.speed);
        f.float(&mut self.dir);
        f.float(&mut self.start_speed);
        f.vector(&mut self.control_mins);
        f.vector(&mut self.control_maxs);
        f.int(&mut self.sounds);
        f.float(&mut self.volume);
        f.float(&mut self.bank);
        f.float(&mut self.old_speed);
    }

    /// `CFuncTrackTrain::Precache` (0xd5968).
    fn restored(&mut self, cx: &mut Cx) {
        self.precache(cx);
    }

    /// `CFuncTrackTrain::OverrideReset` (0xd569c): find the nearest track 0.1 s on.
    fn override_reset(&mut self, cx: &mut Cx) {
        let at = cx.pev().ltime + 0.1;
        Self::next_think(cx, at, false);
        self.think = TrainThink::NearestPath;
    }

    fn think(&mut self, cx: &mut Cx) {
        match self.think {
            TrainThink::Find => self.find(cx),
            TrainThink::Next => self.next(cx),
            TrainThink::DeadEnd => self.dead_end(cx),
            TrainThink::NearestPath => self.nearest_path(cx),
            TrainThink::None => {}
        }
    }

    fn use_(&mut self, cx: &mut Cx, _a: *mut edict_t, _c: *mut edict_t, kind: UseType, value: f32) {
        let v = cx.pev();
        if kind != UseType::Set {
            if !should_toggle(kind, v.speed != 0.0) {
                return;
            }
            if v.speed == 0.0 {
                v.speed = self.speed * self.dir;
                self.next(cx);
            } else {
                v.speed = 0.0;
                v.velocity = [0.0; 3];
                v.avelocity = [0.0; 3];
                self.stop_sound(cx);
                self.think = TrainThink::None;
            }
        } else {
            let mut delta = (speed_notch((v.speed * 4.0) as i32, self.speed as i32)) as f32 * 0.25 + 0.25 * value;
            delta = delta.clamp(-1.0, 1.0);
            if v.spawnflags & SF_TRACKTRAIN_FORWARDONLY != 0 && delta < 0.0 {
                delta = 0.0;
            }
            v.speed = self.speed * delta;
            self.next(cx);
            let v = cx.pev();
            let msg = format!("TRAIN({}), speed to {:.2}\n", s(v.targetname).to_string_lossy(), v.speed);
            engine().alert(ALERT_TYPE::at_aiconsole, &msg);
        }
    }

    fn blocked(&mut self, cx: &mut Cx, other: *mut edict_t) {
        let me = cx.edict;
        let ov = other.v();
        let v = cx.pev();
        if ov.flags & FL_ONGROUND as i32 != 0 && ov.groundentity == me {
            let d = v.speed.abs().min(50.0);
            if ov.velocity[2] == 0.0 {
                ov.velocity[2] += d;
            }
            return;
        }
        ov.velocity = vec::scale(vec::normalize(vec::sub(ov.origin, v.origin)), v.dmg);
        let msg =
            format!("TRAIN({}): Blocked by {} (dmg:{:.2})\n", s(v.targetname).to_string_lossy(), s(ov.classname).to_string_lossy(), v.dmg);
        engine().alert(ALERT_TYPE::at_aiconsole, &msg);
        let v = cx.pev();
        if v.dmg <= 0.0 {
            return;
        }
        let d = v.dmg;
        crate::damage::apply(other, &DamageInfo::new(me, me, d, crate::damage::bits::CRUSH));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn angles() {
        assert_eq!(angle_distance(10.0, 350.0), 20.0);
        assert_eq!(angle_distance(350.0, 10.0), -20.0);
        assert_eq!(approach_angle(90.0, 0.0, 10.0), 10.0);
        assert_eq!(approach_angle(0.0, 350.0, 20.0), 0.0);
        assert_eq!(fixup([-10.0, 370.0, 5.0]), [350.0, 10.0, 5.0]);
    }

    #[test]
    fn pitch_event_flags() {
        // TS passes 10 (`FEV_RELIABLE | FEV_UPDATE`); `FEV_CLIENT` (1 << 6) makes the engines drop it.
        assert_eq!(FEV_RELIABLE | FEV_UPDATE, 10);
    }

    #[test]
    fn use_set_notch() {
        assert_eq!(speed_notch(400, 100), 4);
        assert_eq!(speed_notch(-200, 100), -2);
        // A negative `speed` key divides as TS does.
        assert_eq!(speed_notch(400, -100), -4);
        // Guards where TS traps.
        assert_eq!(speed_notch(400, 0), 400);
        assert_eq!(speed_notch(i32::MIN, -1), i32::MIN);
    }

    #[test]
    fn hud_notches() {
        assert_eq!(train_speed(-50, 300), 5);
        assert_eq!(train_speed(0, 300), 1);
        assert_eq!(train_speed(75, 300), 2);
        assert_eq!(train_speed(150, 300), 3);
        assert_eq!(train_speed(225, 300), 4);
        assert_eq!(train_speed(300, 300), 4);
    }

    #[test]
    fn sound_pitch_is_integer_maths() {
        // 60 + |speed| * 140 / 1000, truncating at each step.
        assert_eq!(train_pitch(0.0), 60);
        assert_eq!(train_pitch(7.9), 60, "7 * 140 / 1000 = 0");
        assert_eq!(train_pitch(-8.0), 61);
        assert_eq!(train_pitch(150.0), 81);
        assert_eq!(train_pitch(1000.0), 200);
        assert_eq!(train_event_pitch(81), 8);
        assert_eq!(train_event_pitch(200), 20);
        assert_eq!(train_event_pitch(70), 7);
    }
}
