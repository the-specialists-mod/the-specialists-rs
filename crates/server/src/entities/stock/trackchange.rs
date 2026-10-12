//! `func_trackchange` (`CFuncTrackChange`) and `func_trackautochange` (`CFuncTrackAuto`): toggle
//! `func_platrot`s that carry a `func_tracktrain` between two `path_track`s.
//!
//! On a map they never do. `CFuncTrackChange::Spawn` (0xd5d84) installs `CFuncTrainControls::Find`
//! as the think (the GOT slot it loads at 0xc5e75 objdump relocates to `Find__18CFuncTrainControls`),
//! not `CFuncTrackChange::Find` (0xd5ff4). `CFuncTrackChange::Find`, the only code that looks up
//! the `toptrack` / `bottomtrack` paths and the `train`, is then reached only through
//! `CFuncTrackChange::OverrideReset` (0xd5fb0), which only a save-game restore of a global track
//! change calls. Until then `m_trackTop`, `m_trackBottom` and `m_train` (+0x12c..+0x134) stay NULL,
//! and with them:
//!
//! * `EvaluateTrain` (0xd6338) returns `TRAIN_SAFE` (no path), so `m_code` stays 0: no
//!   `buttons/button11.wav` alarm, no `UpdateTrain` (0xd6458), no `SetTrack` on arrival.
//! * `UpdateAutoTargets` (0xd68b4, `CFuncTrackAuto`'s 0xd6b60) returns at once (no tracks).
//! * `CFuncTrackAuto::Use` (0xd6c48) evaluates a NULL path (from a train) or toggles nothing and
//!   updates nothing (otherwise): it never moves the platform.
//!
//! So on a map it is a toggle plat that moves (or, with `SF_TRACK_DONT_MOVE`, only turns)
//! between its top and bottom on use, and a 2 s think that runs `CFuncTrainControls::Find` on
//! itself (a `target` naming a `func_tracktrain` hands that train this box as its control volume
//! and removes the track change; otherwise "No train" is logged). The train logic is all ported
//! for the restored case.

use super::base::*;
use super::explode::emit_sound_slow;
use super::tracktrain::{nearest, path_links, path_next, set_train_path, train_next_think, train_path_length};
use super::trains::*;
use crate::save::{Fields, tables};
use crate::world::{Cx, Entity, UseType};
use goldsrc::edict::EdictPtr;
use goldsrc::ext::entities::{ATTN_NORM, EHandle, Vec3, vec};
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::ffi::CStr;

/// Start at the bottom (`m_vecPosition2`, `m_start` angles).
pub const SF_TRACK_STARTBOTTOM: i32 = 8;
/// Don't move: only turn (`AngularMove`), with the bottom at the spawn height.
pub const SF_TRACK_DONT_MOVE: i32 = 16;

/// `TRAIN_SAFE` / `TRAIN_BLOCKING` / `TRAIN_FOLLOWING` (`m_code`, `EvaluateTrain`'s result).
const TRAIN_SAFE: i32 = 0;
const TRAIN_BLOCKING: i32 = 1;
const TRAIN_FOLLOWING: i32 = 2;

/// `func_trackchange` (`AUTO = false`) and `func_trackautochange` (`AUTO = true`).
#[derive(Default)]
pub struct FuncTrackChange<const AUTO: bool> {
    p: PlatBase<Self>,
    /// `m_end` (+0x114): the angles at the top.
    end: Vec3,
    /// `m_start` (+0x120): the angles at the bottom.
    start: Vec3,
    /// `m_trackTop` / `m_trackBottom` (+0x12c / +0x130): `path_track`s, set by `Find`.
    track_top: EHandle,
    track_bottom: EHandle,
    /// `m_train` (+0x134): a `func_tracktrain`, set by `Find`.
    train: EHandle,
    /// `m_trackTopName` / `m_trackBottomName` / `m_trainName` (+0x138..+0x140): `toptrack`,
    /// `bottomtrack`, `train`.
    top_name: string_t,
    bottom_name: string_t,
    train_name: string_t,
    /// `m_code` (+0x144): the last `EvaluateTrain`.
    code: i32,
    /// `m_targetState` (+0x148): `CFuncTrackAuto`'s wanted position (0 top, 1 bottom).
    target_state: i32,
    /// `m_use` (+0x14c): `EnableUse` / `DisableUse` (only `CFuncTrackAuto::Use` reads it).
    use_enabled: bool,
}

impl<const AUTO: bool> Mover for FuncTrackChange<AUTO> {
    fn toggle(&mut self) -> &mut Toggle<Self> {
        &mut self.p.t
    }
}

/// `CPathTrack::Instance` (0xd1c30): the edict when it holds a `path_track`.
fn path_instance(e: *mut edict_t) -> *mut edict_t {
    if e.is_null() || s(e.v().classname) != c"path_track" { std::ptr::null_mut() } else { e }
}

/// `FNullEnt` on a `FIND_ENTITY_BY_TARGETNAME` result: none, or the world.
fn null_ent(e: *mut edict_t) -> bool {
    e.is_null() || engine().index_of(e) == 0
}

/// `UTIL_MakeInvVectors` (0xec758): `MAKE_VECTORS`, then the transpose of (forward, -right, up)
/// (TS leaves it in `gpGlobals->v_*`; the port only returns it).
fn make_inv_vectors(angles: Vec3) -> (Vec3, Vec3, Vec3) {
    let (mut f, r, mut u) = engine().make_vectors(angles);
    let mut r = vec::scale(r, -1.0);
    std::mem::swap(&mut f[1], &mut r[0]);
    std::mem::swap(&mut f[2], &mut u[0]);
    std::mem::swap(&mut r[2], &mut u[1]);
    (f, r, u)
}

impl<const AUTO: bool> FuncTrackChange<AUTO> {
    /// The `Spawn` think: `CFuncTrainControls::Find` (see the module docs).
    fn find(&mut self, cx: &mut Cx) {
        super::tracktrain::train_controls_find(cx);
    }

    /// `CFuncTrackChange::Find` (0xd5ff4), `OverrideReset`'s think: look up the two tracks and
    /// the train, take the nodes of each track nearest the platform's centre, then
    /// `UpdateAutoTargets` and `SetThink(NULL)`. A missing one logs an error and leaves the think
    /// (and the nodes found so far) as they are.
    ///
    /// Guard: TS calls `Nearest` through a NULL `m_trackTop` / `m_trackBottom` when the name finds
    /// an entity that isn't a `path_track`; the port leaves it NULL.
    fn track_find(&mut self, cx: &mut Cx) {
        let eng = engine();
        let top = eng.find_by_target_name(std::ptr::null_mut(), s(self.top_name));
        if null_ent(top) {
            let msg = format!("Can't find top track for track change! {}\n", s(self.top_name).to_string_lossy());
            eng.alert(ALERT_TYPE::at_error, &msg);
            return;
        }
        self.track_top = EHandle::new(path_instance(top));
        let bottom = eng.find_by_target_name(std::ptr::null_mut(), s(self.bottom_name));
        if null_ent(bottom) {
            let msg = format!("Can't find bottom track for track change! {}\n", s(self.bottom_name).to_string_lossy());
            eng.alert(ALERT_TYPE::at_error, &msg);
            return;
        }
        self.track_bottom = EHandle::new(path_instance(bottom));
        let train = eng.find_by_target_name(std::ptr::null_mut(), s(self.train_name));
        let found = !null_ent(train);
        if found {
            // `CFuncTrackTrain::Instance` (0xd56f4): only a `func_tracktrain`.
            self.train = EHandle::new(if train_path_length(train).is_some() { train } else { std::ptr::null_mut() });
        }
        if !self.train.is_valid() || !found {
            let msg = format!("Can't find train for track change! {}\n", s(self.train_name).to_string_lossy());
            eng.alert(ALERT_TYPE::at_error, &msg);
            return;
        }
        let v = cx.pev();
        let center = vec::mid(v.absmin, v.absmax);
        let (top, bottom) = (self.track_top.get(), self.track_bottom.get());
        if !bottom.is_null() {
            self.track_bottom = EHandle::new(nearest(bottom, center));
        }
        if !top.is_null() {
            self.track_top = EHandle::new(nearest(top, center));
        }
        let state = self.p.t.state as i32;
        self.update_auto_targets(state);
        self.p.t.think = None;
    }

    /// `CFuncTrackChange::EvaluateTrain` (0xd6338): where the train stands relative to the
    /// track `current` (a node or its neighbours): moving through it is blocking; stopped within
    /// its length of the platform it's following; up to 150 units further, blocking.
    fn evaluate_train(&self, cx: &mut Cx, current: *mut edict_t) -> i32 {
        let train = self.train.get();
        if current.is_null() || train.is_null() {
            return TRAIN_SAFE;
        }
        let Some((path, length)) = train_path_length(train) else { return TRAIN_SAFE };
        let (next, prev) = path_links(current);
        if path != current && (prev.is_null() || path != prev) && (next.is_null() || path != next) {
            return TRAIN_SAFE;
        }
        let tv = train.v();
        if tv.speed != 0.0 {
            return TRAIN_BLOCKING;
        }
        let v = cx.pev();
        let (dx, dy) = ((v.origin[0] - tv.origin[0]) as f64, (v.origin[1] - tv.origin[1]) as f64);
        let dist = (dy * dy + dx * dx).sqrt() as f32;
        if dist < length {
            TRAIN_FOLLOWING
        } else if dist <= length + 150.0 {
            TRAIN_BLOCKING
        } else {
            TRAIN_SAFE
        }
    }

    /// `CFuncTrackChange::UpdateTrain` (0xd6458): the train rides along: the platform's
    /// velocities and move end, plus the velocity that turns its offset with the platform to
    /// `dest` angles over the move.
    fn update_train(&self, cx: &mut Cx, dest: Vec3) {
        let train = self.train.get();
        if train.is_null() {
            return;
        }
        let v = cx.pev();
        let time = v.nextthink - v.ltime;
        let (vel, avel, origin, angles) = (v.velocity, v.avelocity, v.origin, v.angles);
        let tv = train.v();
        tv.velocity = vel;
        tv.avelocity = avel;
        let at = time + tv.ltime;
        train_next_think(tv, at, false);
        if time <= 0.0 {
            return;
        }
        let offset = vec::sub(train.v().origin, origin);
        let (f, r, u) = make_inv_vectors(vec::sub(dest, angles));
        let dot = |a: Vec3| (0..3).map(|i| offset[i] as f64 * a[i] as f64).sum::<f64>();
        let local = [dot(f), dot(r), dot(u)];
        let local: Vec3 = [0, 1, 2].map(|i| (local[i] - offset[i] as f64) as f32);
        let k = (1.0f64 / time as f64) as f32;
        train.v().velocity = [0, 1, 2].map(|i| (vel[i] as f64 + (local[i] * k) as f64) as f32);
    }

    /// `UpdateAutoTargets` (vtable slot 73): `CFuncTrackChange`'s (0xd68b4) enables the top
    /// track's node at the top and the bottom's at the bottom, disabling the other;
    /// `CFuncTrackAuto`'s (0xd6b60) works from `m_targetState` instead, enabling the next node
    /// of the wanted track (and starting a stopped train that's following) and disabling the
    /// other's.
    fn update_auto_targets(&mut self, state: i32) {
        let (top, bottom) = (self.track_top.get(), self.track_bottom.get());
        if top.is_null() || bottom.is_null() {
            return;
        }
        if !AUTO {
            let set = |e: *mut edict_t, on: bool| {
                let v = e.v();
                if on {
                    v.spawnflags &= !super::tracktrain::SF_PATH_DISABLED;
                } else {
                    v.spawnflags |= super::tracktrain::SF_PATH_DISABLED;
                }
            };
            set(top, state == ToggleState::AtTop as i32);
            set(bottom, state == ToggleState::AtBottom as i32);
            return;
        }
        let (target, next_target) = if self.target_state == ToggleState::AtTop as i32 {
            (path_next(top), path_next(bottom))
        } else {
            (path_next(bottom), path_next(top))
        };
        if !target.is_null() {
            target.v().spawnflags &= !super::tracktrain::SF_PATH_DISABLED;
            let train = self.train.get();
            if self.code == TRAIN_FOLLOWING && !train.is_null() && train.v().speed == 0.0 {
                // `m_train->Use(this, this, USE_ON, 0)`; `this` is the caller's edict, which
                // the train's `Use` doesn't read.
                crate::ham::use_(train, std::ptr::null_mut(), std::ptr::null_mut(), UseType::On, 0.0);
            }
        }
        if !next_target.is_null() {
            next_target.v().spawnflags |= super::tracktrain::SF_PATH_DISABLED;
        }
    }

    /// The `HitTop` / `HitBottom` tail: a following train is put on the arrival track's node
    /// nearest it, `SetThink(NULL)`, `nextthink = -1`, `UpdateAutoTargets`, `EnableUse`.
    fn arrived(&mut self, cx: &mut Cx, track: *mut edict_t) {
        let train = self.train.get();
        if self.code == TRAIN_FOLLOWING && !train.is_null() {
            // Guard: TS calls `Nearest` through the track pointer even when it's NULL.
            let path = if track.is_null() { std::ptr::null_mut() } else { nearest(track, train.v().origin) };
            set_train_path(train, path);
        }
        self.p.t.think = None;
        cx.pev().nextthink = -1.0;
        let state = self.p.t.state as i32;
        self.update_auto_targets(state);
        self.use_enabled = true;
    }

    /// The `GoUp` / `GoDown` tail: a following train rides along and leaves its track.
    fn carry_train(&mut self, cx: &mut Cx, dest: Vec3) {
        let train = self.train.get();
        if self.code == TRAIN_FOLLOWING && !train.is_null() {
            self.update_train(cx, dest);
            set_train_path(train, std::ptr::null_mut());
        }
    }

    /// `CFuncTrackChange::Use` (0xd6918): at the top go down, at the bottom go up, unless the
    /// train blocks it (the alarm); moving ignores it.
    fn change_use(&mut self, cx: &mut Cx) {
        let state = self.p.t.state;
        let track = match state {
            ToggleState::AtTop => self.track_top.get(),
            ToggleState::AtBottom => self.track_bottom.get(),
            _ => return,
        };
        self.code = self.evaluate_train(cx, track);
        if self.code == TRAIN_BLOCKING {
            emit_sound_slow(cx.edict, CHAN_VOICE, c"buttons/button11.wav", 1.0, ATTN_NORM, 0, PITCH_NORM as i32, 0.5);
            return;
        }
        self.use_enabled = false;
        if state == ToggleState::AtTop {
            self.go_down(cx);
        } else {
            self.go_up(cx);
        }
    }

    /// `CFuncTrackAuto::Use` (0xd6c48): from the train, move once it's following and the
    /// platform isn't where `m_targetState` wants it; from anything else, toggle
    /// `m_targetState` (unless the train is already on that track's next node) and update the
    /// targets.
    ///
    /// Guards: TS reads the activator's classname through a NULL activator, and `m_train`'s path
    /// through a NULL `m_train` (with tracks but no train); the port takes a NULL activator as
    /// not a train and a missing train as on no path.
    fn auto_use(&mut self, cx: &mut Cx, activator: *mut edict_t, kind: UseType) {
        if !self.use_enabled {
            return;
        }
        let state = self.p.t.state;
        let track = match state {
            ToggleState::AtTop => self.track_top.get(),
            ToggleState::AtBottom => self.track_bottom.get(),
            _ => std::ptr::null_mut(),
        };
        if !activator.is_null() && s(activator.v().classname) == c"func_tracktrain" {
            self.code = self.evaluate_train(cx, track);
            if self.code == TRAIN_FOLLOWING && state as i32 != self.target_state {
                self.use_enabled = false;
                if state == ToggleState::AtTop {
                    self.go_down(cx);
                } else {
                    self.go_up(cx);
                }
            }
        } else {
            let next = if track.is_null() { std::ptr::null_mut() } else { path_next(track) };
            if !next.is_null() {
                let on_it = train_path_length(self.train.get()).is_some_and(|(path, _)| path == next);
                if !on_it && should_toggle(kind, self.target_state != 0) {
                    self.target_state = (self.target_state == 0) as i32;
                }
            }
            let t = self.target_state;
            self.update_auto_targets(t);
        }
    }

    /// `CFuncTrackChange::Precache` (0xd5e94): the alarm, then `CFuncPlat::Precache` (a toggle
    /// plat: no trigger).
    fn precache(&mut self, cx: &mut Cx) {
        engine().precache_sound(c"buttons/button11.wav");
        plat_precache(self, cx);
    }
}

impl<const AUTO: bool> Plat for FuncTrackChange<AUTO> {
    fn plat(&mut self) -> &mut PlatBase<Self> {
        &mut self.p
    }

    /// `CFuncTrackChange::IsTogglePlat` (0xd7cb0): always, so no trigger volume and no automatic
    /// return from the top.
    fn is_toggle_plat(&self, _cx: &mut Cx) -> bool {
        true
    }

    /// `CFuncTrackChange::GoUp` (0xd67a4).
    fn go_up(&mut self, cx: &mut Cx) {
        if self.code == TRAIN_BLOCKING {
            return;
        }
        self.update_auto_targets(ToggleState::GoingUp as i32);
        if cx.pev().spawnflags & SF_TRACK_DONT_MOVE == 0 {
            plat_go_up(self, cx);
            self.p.t.move_done = Some(call_hit_top::<Self>);
            let t = move_time(cx);
            rot_move(cx, self.end, t);
        } else {
            self.p.t.state = ToggleState::GoingUp;
            self.p.t.move_done = Some(call_hit_top::<Self>);
            let (end, speed) = (self.end, cx.pev().speed);
            angular_move(self, cx, end, speed);
        }
        let end = self.end;
        self.carry_train(cx, end);
    }

    /// `CFuncTrackChange::GoDown` (0xd6694).
    fn go_down(&mut self, cx: &mut Cx) {
        if self.code == TRAIN_BLOCKING {
            return;
        }
        self.update_auto_targets(ToggleState::GoingDown as i32);
        if cx.pev().spawnflags & SF_TRACK_DONT_MOVE == 0 {
            plat_go_down(self, cx);
            self.p.t.move_done = Some(call_hit_bottom::<Self>);
            let t = move_time(cx);
            rot_move(cx, self.start, t);
        } else {
            self.p.t.move_done = Some(call_hit_bottom::<Self>);
            self.p.t.state = ToggleState::GoingDown;
            let (start, speed) = (self.start, cx.pev().speed);
            angular_move(self, cx, start, speed);
        }
        let start = self.start;
        self.carry_train(cx, start);
    }

    /// `CFuncTrackChange::HitTop` (0xd6a98): `CFuncPlatRot::HitTop`, then the tail.
    fn hit_top(&mut self, cx: &mut Cx) {
        plat_hit_top(self, cx);
        rot_settle(cx, self.end);
        let top = self.track_top.get();
        self.arrived(cx, top);
    }

    /// `CFuncTrackChange::HitBottom` (0xd69f4): `CFuncPlatRot::HitBottom`, then the tail.
    fn hit_bottom(&mut self, cx: &mut Cx) {
        plat_hit_bottom(self, cx);
        rot_settle(cx, self.start);
        let bottom = self.track_bottom.get();
        self.arrived(cx, bottom);
    }
}

impl<const AUTO: bool> Entity for FuncTrackChange<AUTO> {
    /// `CFuncTrackChange::KeyValue` (0xd5ed0).
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"train" => self.train_name = alloc(value),
            b"toptrack" => self.top_name = alloc(value),
            b"bottomtrack" => self.bottom_name = alloc(value),
            _ => return plat_key_value(&mut self.p, key, value),
        }
        true
    }

    /// `CFuncTrackChange::Spawn` (0xd5d84).
    fn spawn(&mut self, cx: &mut Cx) {
        plat_setup(self, cx);
        if cx.pev().spawnflags & SF_TRACK_DONT_MOVE != 0 {
            self.p.t.pos2[2] = cx.pev().origin[2];
        }
        let rotation = self.p.t.final_angle[0];
        (self.end, self.start) = setup_rotation(cx, rotation);
        let (pos, state, angles, target) = if cx.pev().spawnflags & SF_TRACK_STARTBOTTOM == 0 {
            (self.p.t.pos1, ToggleState::AtTop, self.end, ToggleState::AtBottom)
        } else {
            (self.p.t.pos2, ToggleState::AtBottom, self.start, ToggleState::AtTop)
        };
        engine().set_origin(cx.edict, pos);
        self.p.t.state = state;
        cx.pev().angles = angles;
        self.target_state = target as i32;
        self.use_enabled = true;
        think_ltime(cx, 2.0);
        self.p.t.think = Some(Self::find);
        self.precache(cx);
    }

    /// `CFuncTrackChange::Save` / `Restore` (0xd5ce4 / 0xd5d34; `CFuncTrackAuto` uses them):
    /// the `CFuncPlatRot` chain, then the track change's own table (its pointers and names are
    /// `FTYPEDESC_GLOBAL`). `Use` and `Touch` are virtuals.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut Fields) {
        self.p.save_chain(cx, f);
        save_plat_rot(f, &mut self.end, &mut self.start);
        f.table(&tables::CFuncTrackChange);
        ent_handle(f, &mut self.track_top);
        ent_handle(f, &mut self.track_bottom);
        ent_handle(f, &mut self.train);
        f.string(&mut self.top_name);
        f.string(&mut self.bottom_name);
        f.string(&mut self.train_name);
        f.int(&mut self.code);
        f.int(&mut self.target_state);
        f.boolean(&mut self.use_enabled);
    }

    /// `CFuncTrackChange::Precache` (0xd5e94).
    fn restored(&mut self, cx: &mut Cx) {
        self.precache(cx);
    }

    /// `CFuncTrackChange::OverrideReset` (0xd5fb0): `CFuncTrackChange::Find` 1 s on.
    fn override_reset(&mut self, cx: &mut Cx) {
        think_ltime(cx, 1.0);
        self.p.t.think = Some(Self::track_find);
    }

    fn think(&mut self, cx: &mut Cx) {
        run_think(self, cx);
    }

    fn use_(&mut self, cx: &mut Cx, activator: *mut edict_t, _c: *mut edict_t, kind: UseType, _v: f32) {
        if AUTO {
            self.auto_use(cx, activator, kind);
        } else {
            self.change_use(cx);
        }
    }

    /// `CFuncPlat::Blocked` (0xd2e64); `CFuncTrackChange::Touch` (0xd5ecc) is empty.
    fn blocked(&mut self, cx: &mut Cx, other: *mut edict_t) {
        plat_blocked(self, cx, other);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_found_is_safe() {
        let t = FuncTrackChange::<false>::default();
        let mut ed = edict_t::default();
        let mut cx = Cx { edict: &mut ed };
        assert_eq!(t.evaluate_train(&mut cx, std::ptr::null_mut()), TRAIN_SAFE);
    }
}
