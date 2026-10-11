//! `func_train` and `path_corner` (SDK plats.cpp / pathcorner.cpp; same in TS), `func_plat`
//! (TS `CFuncPlat`, with its trigger volume), `func_platrot` (`CFuncPlatRot`) and
//! `func_guntarget` (`CGunTarget`).
//!
//! The SDK starts trains from `CFuncTrain::Activate`, which `ServerActivate` calls once every
//! entity has spawned. The entity framework has no activate hook, so the train runs `Activate`
//! from a think 0.1 s (local time) after spawning instead.

use super::base::*;
use super::explode::emit_sound_slow;
use crate::entities::fire_targets;
use crate::world::{self, Cx, DamageInfo, Entity, UseType};
use goldsrc::edict::EdictPtr;
use goldsrc::ext::entities::{ATTN_NORM, EHandle, Vec3, snd, vec};
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::ffi::CStr;

pub const SF_CORNER_WAITFORTRIG: i32 = 1;
pub const SF_CORNER_TELEPORT: i32 = 2;
pub const SF_CORNER_FIREONCE: i32 = 4;
pub const SF_TRAIN_WAIT_RETRIGGER: i32 = 1;
pub const SF_TRACKTRAIN_PASSABLE: i32 = 8;

/// `path_corner`: a point with a `wait` (its `GetDelay`).
#[derive(Default)]
pub struct PathCorner {
    pub wait: f32,
}

impl Entity for PathCorner {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        if key == c"wait" {
            self.wait = kv_f(value);
            return true;
        }
        false
    }
}

/// `CBasePlatTrain` move sounds.
pub fn plat_move_sound(n: i32) -> &'static CStr {
    match n {
        1 => c"plats/bigmove1.wav",
        2 => c"plats/bigmove2.wav",
        3 => c"plats/elevmove1.wav",
        4 => c"plats/elevmove2.wav",
        5 => c"plats/elevmove3.wav",
        6 => c"plats/freightmove1.wav",
        7 => c"plats/freightmove2.wav",
        8 => c"plats/heavymove1.wav",
        9 => c"plats/rackmove1.wav",
        10 => c"plats/railmove1.wav",
        11 => c"plats/squeekmove1.wav",
        12 => c"plats/talkmove1.wav",
        13 => c"plats/talkmove2.wav",
        _ => c"common/null.wav",
    }
}

/// `CBasePlatTrain::Precache` (0xd21a4) / `CFuncTrain::Precache`: each table sound is precached;
/// `common/null.wav` (any other number) only gets its string (`CWorld::Precache` precached it).
fn precache_plat_sound(n: &'static CStr) {
    if n != c"common/null.wav" {
        engine().precache_sound(n);
    }
}

pub fn plat_stop_sound(n: i32) -> &'static CStr {
    match n {
        1 => c"plats/bigstop1.wav",
        2 => c"plats/bigstop2.wav",
        3 => c"plats/freightstop1.wav",
        4 => c"plats/heavystop2.wav",
        5 => c"plats/rackstop1.wav",
        6 => c"plats/railstop1.wav",
        7 => c"plats/squeekstop1.wav",
        8 => c"plats/talkstop1.wav",
        _ => c"common/null.wav",
    }
}

#[derive(Default)]
pub struct FuncTrain {
    t: Toggle<Self>,
    /// `m_bMoveSnd` / `m_bStopSnd` (`BYTE`s, [`plat_sound_index`]).
    move_snd: u8,
    stop_snd: u8,
    volume: f32,
    current: EHandle,
    activated: bool,
    activate_finished: f32,
}

impl Mover for FuncTrain {
    fn toggle(&mut self) -> &mut Toggle<Self> {
        &mut self.t
    }
}

fn path_delay(e: *mut edict_t) -> f32 {
    world::with::<PathCorner, _>(e, |p| p.wait).unwrap_or(0.0)
}

impl FuncTrain {
    fn stop_move_sound(cx: &mut Cx) {
        let n = cx.pev().noise;
        if n != 0 {
            engine().emit_sound(cx.edict, CHAN_STATIC as i32, s(n), 0.0, 0.0, snd::STOP, PITCH_NORM as i32);
        }
    }

    fn stop_sound(&self, cx: &mut Cx) {
        let n = cx.pev().noise1;
        if n != 0 {
            emit(cx.edict, CHAN_VOICE, s(n), self.volume, ATTN_NORM, 0, PITCH_NORM as i32);
        }
    }

    fn current_valid(&self) -> bool {
        self.current.is_valid()
    }

    fn wait(&mut self, cx: &mut Cx) {
        let me = cx.edict;
        if self.current_valid() {
            let cv = self.current.get().v();
            if cv.message != 0 {
                fire_targets(s(cv.message), me, me, UseType::Toggle, 0.0);
                if cv.spawnflags & SF_CORNER_FIREONCE != 0 {
                    cv.message = 0;
                }
            }
            if cv.spawnflags & SF_TRAIN_WAIT_RETRIGGER != 0 || cx.pev().spawnflags & SF_TRAIN_WAIT_RETRIGGER != 0 {
                cx.pev().spawnflags |= SF_TRAIN_WAIT_RETRIGGER;
                Self::stop_move_sound(cx);
                self.stop_sound(cx);
                cx.pev().nextthink = 0.0;
                return;
            }
        }
        if self.t.wait != 0.0 {
            let w = self.t.wait;
            think_ltime(cx, w);
            Self::stop_move_sound(cx);
            self.stop_sound(cx);
            self.t.think = Some(Self::next);
        } else {
            self.next(cx);
        }
    }

    fn next(&mut self, cx: &mut Cx) {
        let eng = engine();
        let target = cx.pev().target;
        let targ = if target != 0 { eng.find_by_target_name(std::ptr::null_mut(), s(target)) } else { std::ptr::null_mut() };
        if targ.is_null() {
            Self::stop_move_sound(cx);
            self.stop_sound(cx);
            return;
        }
        let tv = targ.v();
        let v = cx.pev();
        v.message = v.target;
        v.target = tv.target;
        self.t.wait = path_delay(targ);
        if self.current_valid() {
            let cs = self.current.get().v().speed;
            if cs != 0.0 {
                cx.pev().speed = cs;
            }
        }
        self.current = EHandle::new(targ);
        cx.pev().enemy = targ;
        let v = cx.pev();
        let dest = vec::sub(tv.origin, vec::mid(v.mins, v.maxs));
        if tv.spawnflags & SF_CORNER_TELEPORT != 0 {
            v.effects |= EF_NOINTERP as i32;
            eng.set_origin(cx.edict, dest);
            self.wait(cx);
        } else {
            let n = v.noise;
            if n != 0 {
                Self::stop_move_sound(cx);
                emit(cx.edict, CHAN_STATIC, s(n), self.volume, ATTN_NORM, 0, PITCH_NORM as i32);
            }
            cx.pev().effects &= !(EF_NOINTERP as i32);
            self.t.move_done = Some(Self::wait);
            let speed = cx.pev().speed;
            linear_move(self, cx, dest, speed);
        }
    }

    fn activate(&mut self, cx: &mut Cx) {
        if self.activated {
            return;
        }
        self.activated = true;
        let eng = engine();
        let targ = eng.find_by_target_name(std::ptr::null_mut(), s(cx.pev().target));
        if targ.is_null() {
            return;
        }
        let tv = targ.v();
        let v = cx.pev();
        v.target = tv.target;
        self.current = EHandle::new(targ);
        let dest = vec::sub(tv.origin, vec::mid(v.mins, v.maxs));
        eng.set_origin(cx.edict, dest);
        if cx.pev().targetname == 0 {
            think_ltime(cx, 0.1);
            self.t.think = Some(Self::next);
        } else {
            cx.pev().spawnflags |= SF_TRAIN_WAIT_RETRIGGER;
        }
    }
}

impl Entity for FuncTrain {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"height" => self.t.height = kv_f(value),
            b"rotation" => self.t.final_angle[0] = kv_f(value),
            b"movesnd" => self.move_snd = plat_sound_index(kv_d(value)),
            b"stopsnd" => self.stop_snd = plat_sound_index(kv_d(value)),
            b"volume" => self.volume = kv_f(value),
            _ => return self.t.key_value(key, value),
        }
        true
    }

    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        let (m, st) = (plat_move_sound(self.move_snd as i32), plat_stop_sound(self.stop_snd as i32));
        precache_plat_sound(m);
        precache_plat_sound(st);
        let v = cx.pev();
        v.noise = alloc(m);
        v.noise1 = alloc(st);
        if v.speed == 0.0 {
            v.speed = 100.0;
        }
        if v.target == 0 {
            eng.alert(ALERT_TYPE::at_console, "FuncTrain with no target");
        }
        if v.dmg == 0.0 {
            v.dmg = 2.0;
        }
        v.movetype = MOVETYPE_PUSH as i32;
        v.solid = if v.spawnflags & SF_TRACKTRAIN_PASSABLE != 0 { SOLID_NOT } else { SOLID_BSP } as i32;
        set_own_model(cx);
        let v = cx.pev();
        let (mins, maxs, org) = (v.mins, v.maxs, v.origin);
        eng.set_size(cx.edict, mins, maxs);
        eng.set_origin(cx.edict, org);
        self.activated = false;
        if self.volume == 0.0 {
            self.volume = 0.85;
        }
        // Stand-in for CFuncTrain::Activate (see module docs).
        self.t.think = Some(Self::activate);
        think_ltime(cx, 0.1);
    }

    fn think(&mut self, cx: &mut Cx) {
        run_think(self, cx);
    }

    fn use_(&mut self, cx: &mut Cx, _activator: *mut edict_t, _caller: *mut edict_t, _kind: UseType, _value: f32) {
        if !self.activated {
            self.activate(cx);
        }
        let v = cx.pev();
        if v.spawnflags & SF_TRAIN_WAIT_RETRIGGER != 0 {
            v.spawnflags &= !SF_TRAIN_WAIT_RETRIGGER;
            self.next(cx);
        } else {
            v.spawnflags |= SF_TRAIN_WAIT_RETRIGGER;
            if !v.enemy.is_null() {
                v.target = v.enemy.v().targetname;
            }
            v.nextthink = 0.0;
            v.velocity = [0.0; 3];
            self.stop_sound(cx);
        }
    }

    fn blocked(&mut self, cx: &mut Cx, other: *mut edict_t) {
        let now = engine().time();
        if now < self.activate_finished {
            return;
        }
        self.activate_finished = now + 0.5;
        let me = cx.edict;
        let d = cx.pev().dmg;
        crate::damage::apply(other, &DamageInfo::new(me, me, d, crate::damage::bits::CRUSH));
    }
}

// ---- func_plat / func_platrot -------------------------------------------------------------------------

pub const SF_PLAT_TOGGLE: i32 = 1;

/// `CFuncPlat::Blocked` (0xd2e64) crush damage.
const PLAT_CRUSH: f32 = 1.0;
/// `EMIT_SOUND_DYN`'s slow-motion pitch factor in every `CFuncPlat` sound.
const PLAT_SLOW: f32 = 0.5;

/// `movesnd` / `stopsnd` (`CBasePlatTrain::KeyValue` 0xd1f80, also `func_train`'s): `strtod`,
/// then `fistp` to a short with the x87 set to truncate (0xc20d8 objdump) keeping the low byte,
/// which `Precache` reads unsigned (`movzbl`, 0xd21b7). Out of the short's range `fistp` stores
/// 0x8000 (byte 0).
fn plat_sound_index(v: f64) -> u8 {
    fistp16_low_byte(v)
}

/// The `CBasePlatTrain` + `CFuncPlat` members shared by `func_plat`, `func_platrot` and the track
/// changes (`trackchange.rs`).
pub struct PlatBase<T> {
    pub t: Toggle<T>,
    move_snd: u8,
    stop_snd: u8,
    volume: f32,
    t_length: f32,
    t_width: f32,
    /// `SetUse(PlatUse)`: set by `CFuncPlat::Spawn` for a plat with a targetname.
    usable: bool,
    /// TS `targettop` / `targetbot` (+0x10c / +0x110): fired when the plat reaches the top / bottom.
    target_top: string_t,
    target_bottom: string_t,
}

impl<T> Default for PlatBase<T> {
    fn default() -> Self {
        Self {
            t: Toggle::default(),
            move_snd: 0,
            stop_snd: 0,
            volume: 0.0,
            t_length: 0.0,
            t_width: 0.0,
            usable: false,
            target_top: 0,
            target_bottom: 0,
        }
    }
}

/// The `CFuncPlat` virtuals (vtable slots 70–74) its subclasses override. The generic `plat_*`
/// functions below are the `CFuncPlat` bodies; they reach these the way TS reaches the vtable.
pub trait Plat: Mover {
    fn plat(&mut self) -> &mut PlatBase<Self>;
    /// `IsTogglePlat` (slot 70): `CBasePlatTrain::IsTogglePlat` (0xd7a90) is spawnflag 1.
    fn is_toggle_plat(&self, cx: &mut Cx) -> bool {
        cx.pev().spawnflags & SF_PLAT_TOGGLE != 0
    }
    /// Slot 71.
    fn go_up(&mut self, cx: &mut Cx);
    /// Slot 72.
    fn go_down(&mut self, cx: &mut Cx);
    /// Slot 73.
    fn hit_top(&mut self, cx: &mut Cx);
    /// Slot 74.
    fn hit_bottom(&mut self, cx: &mut Cx);
}

/// `CFuncPlat::KeyValue` (0xd252c) and `CBasePlatTrain::KeyValue` (0xd1f80).
pub fn plat_key_value<T>(p: &mut PlatBase<T>, key: &CStr, value: &CStr) -> bool {
    match key.to_bytes() {
        b"targettop" => p.target_top = alloc(value),
        b"targetbot" => p.target_bottom = alloc(value),
        b"height" => p.t.height = kv_f(value),
        b"rotation" => p.t.final_angle[0] = kv_f(value),
        b"movesnd" => p.move_snd = plat_sound_index(kv_d(value)),
        b"stopsnd" => p.stop_snd = plat_sound_index(kv_d(value)),
        b"volume" => p.volume = kv_f(value),
        _ => return p.t.key_value(key, value),
    }
    true
}

/// `CFuncPlat::Setup` (0xd25d0).
pub fn plat_setup<T: Plat>(this: &mut T, cx: &mut Cx) {
    let eng = engine();
    let p = this.plat();
    if p.t_length == 0.0 {
        p.t_length = 80.0;
    }
    if p.t_width == 0.0 {
        p.t_width = 10.0;
    }
    let v = cx.pev();
    v.angles = [0.0; 3];
    v.solid = SOLID_BSP as i32;
    v.movetype = MOVETYPE_PUSH as i32;
    let (org, mins, maxs) = (v.origin, v.mins, v.maxs);
    eng.set_origin(cx.edict, org);
    eng.set_size(cx.edict, mins, maxs);
    set_own_model(cx);
    let v = cx.pev();
    let p = this.plat();
    p.t.pos1 = v.origin;
    p.t.pos2 = v.origin;
    p.t.pos2[2] = if p.t.height != 0.0 { v.origin[2] - p.t.height } else { v.origin[2] - v.size[2] + 8.0 };
    if v.speed == 0.0 {
        v.speed = 150.0;
    }
    if p.volume == 0.0 {
        p.volume = 0.85;
    }
}

/// `CFuncPlat::Precache` (0xd2754): `CBasePlatTrain::Precache` (0xd21a4) and, for a plat that
/// isn't a toggle plat, the trigger volume.
pub fn plat_precache<T: Plat>(this: &mut T, cx: &mut Cx) {
    let p = this.plat();
    let (m, st) = (plat_move_sound(p.move_snd as i32), plat_stop_sound(p.stop_snd as i32));
    let (pos1, pos2) = (p.t.pos1, p.t.pos2);
    // An unknown number is `common/null.wav`, which `CWorld::Precache` precaches and this doesn't.
    precache_plat_sound(m);
    precache_plat_sound(st);
    let v = cx.pev();
    v.noise = alloc(m);
    v.noise1 = alloc(st);
    if !this.is_toggle_plat(cx) {
        spawn_plat_trigger(cx, pos1, pos2);
    }
}

/// `CFuncPlat::Spawn` (0xd2798).
pub fn plat_spawn<T: Plat>(this: &mut T, cx: &mut Cx) {
    plat_setup(this, cx);
    plat_precache(this, cx);
    let p = this.plat();
    if cx.pev().targetname == 0 {
        let pos = p.t.pos2;
        engine().set_origin(cx.edict, pos);
        p.t.state = ToggleState::AtBottom;
    } else {
        let pos = p.t.pos1;
        engine().set_origin(cx.edict, pos);
        p.t.state = ToggleState::AtTop;
        p.usable = true;
    }
}

/// `CFuncPlat::PlatUse` (0xd2a68), installed by `Spawn` for a plat with a targetname.
pub fn plat_use<T: Plat>(this: &mut T, cx: &mut Cx, kind: UseType) {
    if !this.plat().usable {
        return;
    }
    let state = this.plat().t.state;
    if this.is_toggle_plat(cx) {
        if !should_toggle(kind, state == ToggleState::AtBottom) {
            return;
        }
        match state {
            ToggleState::AtTop => this.go_down(cx),
            ToggleState::AtBottom => this.go_up(cx),
            _ => {}
        }
    } else {
        // SetUse(NULL): a one-way plat only comes down once.
        this.plat().usable = false;
        if state == ToggleState::AtTop {
            this.go_down(cx);
        }
    }
}

fn plat_move_sound_play<T: Plat>(this: &mut T, cx: &mut Cx) {
    let (n, vol) = (cx.pev().noise, this.plat().volume);
    if n != 0 {
        emit_sound_slow(cx.edict, CHAN_STATIC, s(n), vol, ATTN_NORM, 0, PITCH_NORM as i32, PLAT_SLOW);
    }
}

fn plat_stop_move_sound(cx: &mut Cx) {
    let n = cx.pev().noise;
    if n != 0 {
        emit_sound_slow(cx.edict, CHAN_STATIC, s(n), 0.0, 0.0, snd::STOP, PITCH_NORM as i32, PLAT_SLOW);
    }
}

fn plat_stop_sounds<T: Plat>(this: &mut T, cx: &mut Cx) {
    plat_stop_move_sound(cx);
    let (n1, vol) = (cx.pev().noise1, this.plat().volume);
    if n1 != 0 {
        emit_sound_slow(cx.edict, CHAN_WEAPON, s(n1), vol, ATTN_NORM, 0, PITCH_NORM as i32, PLAT_SLOW);
    }
}

/// `CFuncPlat::CallGoDown` (0xd7af0).
fn call_go_down<T: Plat>(this: &mut T, cx: &mut Cx) {
    this.go_down(cx);
}

/// `CFuncPlat::CallHitTop` (0xd7b08).
pub fn call_hit_top<T: Plat>(this: &mut T, cx: &mut Cx) {
    this.hit_top(cx);
}

/// `CFuncPlat::CallHitBottom` (0xd7b20).
pub fn call_hit_bottom<T: Plat>(this: &mut T, cx: &mut Cx) {
    this.hit_bottom(cx);
}

/// `CFuncPlat::GoDown` (0xd2b08).
pub fn plat_go_down<T: Plat>(this: &mut T, cx: &mut Cx) {
    plat_move_sound_play(this, cx);
    let p = this.plat();
    p.t.state = ToggleState::GoingDown;
    p.t.move_done = Some(call_hit_bottom::<T>);
    let (pos, speed) = (p.t.pos2, cx.pev().speed);
    linear_move(this, cx, pos, speed);
}

/// `CFuncPlat::GoUp` (0xd2c98).
pub fn plat_go_up<T: Plat>(this: &mut T, cx: &mut Cx) {
    plat_move_sound_play(this, cx);
    let p = this.plat();
    p.t.state = ToggleState::GoingUp;
    p.t.move_done = Some(call_hit_top::<T>);
    let (pos, speed) = (p.t.pos1, cx.pev().speed);
    linear_move(this, cx, pos, speed);
}

/// `CFuncPlat::HitBottom` (0xd2bc0).
pub fn plat_hit_bottom<T: Plat>(this: &mut T, cx: &mut Cx) {
    plat_stop_sounds(this, cx);
    let p = this.plat();
    p.t.state = ToggleState::AtBottom;
    let target = p.target_bottom;
    if target != 0 {
        let me = cx.edict;
        fire_targets(s(target), me, me, UseType::Toggle, 0.0);
    }
}

/// `CFuncPlat::HitTop` (0xd2d50): a plat that isn't a toggle plat goes back down 3 s later.
pub fn plat_hit_top<T: Plat>(this: &mut T, cx: &mut Cx) {
    plat_stop_sounds(this, cx);
    this.plat().t.state = ToggleState::AtTop;
    if !this.is_toggle_plat(cx) {
        this.plat().t.think = Some(call_go_down::<T>);
        think_ltime(cx, 3.0);
    }
    let target = this.plat().target_top;
    if target != 0 {
        let me = cx.edict;
        fire_targets(s(target), me, me, UseType::Toggle, 0.0);
    }
}

/// `CFuncPlat::Blocked` (0xd2e64): 1 crush damage to the blocker (a direct `TakeDamage`), then
/// the plat turns back.
pub fn plat_blocked<T: Plat>(this: &mut T, cx: &mut Cx, other: *mut edict_t) {
    let me = cx.edict;
    let eng = engine();
    let msg = format!("{} Blocked by {}\n", s(cx.pev().classname).to_string_lossy(), s(other.v().classname).to_string_lossy());
    eng.alert(ALERT_TYPE::at_aiconsole, &msg);
    crate::damage::apply(other, &DamageInfo::new(me, me, PLAT_CRUSH, crate::damage::bits::CRUSH));
    plat_stop_move_sound(cx);
    match this.plat().t.state {
        ToggleState::GoingUp => this.go_down(cx),
        ToggleState::GoingDown => this.go_up(cx),
        _ => {}
    }
}

/// `CPlatTrigger::Touch` (0xd29d0) on behalf of the trigger volume.
fn plat_trigger_touch<T: Plat>(this: &mut T, cx: &mut Cx) {
    match this.plat().t.state {
        ToggleState::AtBottom => this.go_up(cx),
        ToggleState::AtTop => think_ltime(cx, 1.0),
        _ => {}
    }
}

/// `func_plat` (`CFuncPlat`): a lift that rises when a player steps into its trigger volume (or
/// toggles on use with spawnflag 1).
#[derive(Default)]
pub struct FuncPlat {
    p: PlatBase<Self>,
}

impl Mover for FuncPlat {
    fn toggle(&mut self) -> &mut Toggle<Self> {
        &mut self.p.t
    }
}

impl Plat for FuncPlat {
    fn plat(&mut self) -> &mut PlatBase<Self> {
        &mut self.p
    }
    fn go_up(&mut self, cx: &mut Cx) {
        plat_go_up(self, cx);
    }
    fn go_down(&mut self, cx: &mut Cx) {
        plat_go_down(self, cx);
    }
    fn hit_top(&mut self, cx: &mut Cx) {
        plat_hit_top(self, cx);
    }
    fn hit_bottom(&mut self, cx: &mut Cx) {
        plat_hit_bottom(self, cx);
    }
}

impl Entity for FuncPlat {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        plat_key_value(&mut self.p, key, value)
    }

    fn spawn(&mut self, cx: &mut Cx) {
        plat_spawn(self, cx);
    }

    fn think(&mut self, cx: &mut Cx) {
        run_think(self, cx);
    }

    fn use_(&mut self, cx: &mut Cx, _a: *mut edict_t, _c: *mut edict_t, kind: UseType, _v: f32) {
        plat_use(self, cx, kind);
    }

    fn blocked(&mut self, cx: &mut Cx, other: *mut edict_t) {
        plat_blocked(self, cx, other);
    }
}

/// `CFuncPlatRot::SetupRotation` (0xd3010): `(m_end, m_start)`. With a `rotation`, `m_start` is
/// the current angles (which `CFuncPlat::Setup` has just zeroed) and `m_end` adds `rotation`
/// degrees about the `AxisDir` axis; without one both are zero. A plat with a targetname starts
/// at `m_end`.
pub fn setup_rotation(cx: &mut Cx, rotation: f32) -> (Vec3, Vec3) {
    let v = cx.pev();
    let (end, start) = if rotation != 0.0 {
        axis_dir(v);
        (vec::add(v.angles, vec::scale(v.movedir, rotation)), v.angles)
    } else {
        ([0.0; 3], [0.0; 3])
    };
    if v.targetname != 0 {
        v.angles = end;
    }
    (end, start)
}

/// `CFuncPlatRot::RotMove` (0xd3294): turn to `dest` in `time`. Under 0.1 s the whole difference
/// becomes the angular velocity for a 1 s think.
pub fn rot_move(cx: &mut Cx, dest: Vec3, time: f32) {
    let v = cx.pev();
    let (avelocity, think_in_1s) = rot_move_velocity(vec::sub(dest, v.angles), time);
    v.avelocity = avelocity;
    if think_in_1s {
        v.nextthink = v.ltime + 1.0;
    }
}

/// `RotMove`'s angular velocity for `delta` in `time`, and whether it reschedules the think 1 s on.
fn rot_move_velocity(delta: Vec3, time: f32) -> (Vec3, bool) {
    if time < 0.1 { (delta, true) } else { ([delta[0] / time, delta[1] / time, delta[2] / time], false) }
}

/// The `RotMove` time `CFuncPlatRot::GoUp`/`GoDown` pass: what the linear move just scheduled.
pub fn move_time(cx: &mut Cx) -> f32 {
    let v = cx.pev();
    v.nextthink - v.ltime
}

/// `CFuncPlatRot::HitTop` (0xd323c) / `HitBottom` (0xd319c) after the `CFuncPlat` part: stop
/// turning and snap to `angles`.
pub fn rot_settle(cx: &mut Cx, angles: Vec3) {
    let v = cx.pev();
    v.avelocity = [0.0; 3];
    v.angles = angles;
}

/// `func_platrot` (`CFuncPlatRot`): a `func_plat` that also turns `rotation` degrees about its
/// axis (spawnflags 64 / 128 as for rotating doors) while it moves.
#[derive(Default)]
pub struct FuncPlatRot {
    p: PlatBase<Self>,
    /// `m_end` (+0x114): the angles at the top.
    end: Vec3,
    /// `m_start` (+0x120): the angles at the bottom.
    start: Vec3,
}

impl Mover for FuncPlatRot {
    fn toggle(&mut self) -> &mut Toggle<Self> {
        &mut self.p.t
    }
}

impl Plat for FuncPlatRot {
    fn plat(&mut self) -> &mut PlatBase<Self> {
        &mut self.p
    }
    /// `CFuncPlatRot::GoUp` (0xd31f4).
    fn go_up(&mut self, cx: &mut Cx) {
        plat_go_up(self, cx);
        let t = move_time(cx);
        rot_move(cx, self.end, t);
    }
    /// `CFuncPlatRot::GoDown` (0xd3154).
    fn go_down(&mut self, cx: &mut Cx) {
        plat_go_down(self, cx);
        let t = move_time(cx);
        rot_move(cx, self.start, t);
    }
    /// `CFuncPlatRot::HitTop` (0xd323c).
    fn hit_top(&mut self, cx: &mut Cx) {
        plat_hit_top(self, cx);
        rot_settle(cx, self.end);
    }
    /// `CFuncPlatRot::HitBottom` (0xd319c).
    fn hit_bottom(&mut self, cx: &mut Cx) {
        plat_hit_bottom(self, cx);
        rot_settle(cx, self.start);
    }
}

impl Entity for FuncPlatRot {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        plat_key_value(&mut self.p, key, value)
    }

    /// `CFuncPlatRot::Spawn` (0xd3128).
    fn spawn(&mut self, cx: &mut Cx) {
        plat_spawn(self, cx);
        let rotation = self.p.t.final_angle[0];
        (self.end, self.start) = setup_rotation(cx, rotation);
    }

    fn think(&mut self, cx: &mut Cx) {
        run_think(self, cx);
    }

    fn use_(&mut self, cx: &mut Cx, _a: *mut edict_t, _c: *mut edict_t, kind: UseType, _v: f32) {
        plat_use(self, cx, kind);
    }

    fn blocked(&mut self, cx: &mut Cx, other: *mut edict_t) {
        plat_blocked(self, cx, other);
    }
}

/// `PlatSpawnInsideTrigger` / `CPlatTrigger::SpawnInsideTrigger` (0xd2858): the trigger volume
/// that calls the lift.
fn spawn_plat_trigger(cx: &mut Cx, pos1: Vec3, pos2: Vec3) {
    let eng = engine();
    let e = eng.create_entity();
    if e.is_null() {
        return;
    }
    world::create::<PlatTrigger>(e.v());
    let platform = EHandle::new(cx.edict);
    world::with::<PlatTrigger, _>(e, |t| t.platform = platform);
    let pv = cx.pev();
    let tv = e.v();
    tv.classname = eng.alloc_string(c"plat_trigger");
    tv.solid = SOLID_TRIGGER as i32;
    tv.movetype = MOVETYPE_NONE as i32;
    tv.origin = pv.origin;
    let mut tmin = vec::add(pv.mins, [25.0, 25.0, 0.0]);
    let mut tmax = vec::add(pv.maxs, [25.0, 25.0, 8.0]);
    tmin[2] = tmax[2] - (pos1[2] - pos2[2] + 8.0);
    if pv.size[0] <= 50.0 {
        tmin[0] = (pv.mins[0] + pv.maxs[0]) / 2.0;
        tmax[0] = tmin[0] + 1.0;
    }
    if pv.size[1] <= 50.0 {
        tmin[1] = (pv.mins[1] + pv.maxs[1]) / 2.0;
        tmax[1] = tmin[1] + 1.0;
    }
    eng.set_size(e, tmin, tmax);
}

#[derive(Default)]
pub struct PlatTrigger {
    platform: EHandle,
}

impl Entity for PlatTrigger {
    /// `CPlatTrigger::Touch` (0xd29d0): a live player calls the plat (`func_plat` or
    /// `func_platrot`; the track changes are toggle plats and have no trigger).
    fn touch(&mut self, _cx: &mut Cx, other: *mut edict_t) {
        if !goldsrc::ext::entities::is_player(other) {
            return;
        }
        let ov = other.v();
        if ov.deadflag != DEAD_NO as i32 || ov.health <= 0.0 {
            return;
        }
        let p = self.platform.get();
        if p.is_null() {
            return;
        }
        world::dispatch(p, |ent, pcx| {
            let any = ent as &mut dyn std::any::Any;
            if let Some(plat) = any.downcast_mut::<FuncPlat>() {
                plat_trigger_touch(plat, pcx);
            } else if let Some(plat) = any.downcast_mut::<FuncPlatRot>() {
                plat_trigger_touch(plat, pcx);
            }
        });
    }
}

// ---- func_guntarget -------------------------------------------------------------------------------------

/// `FGT_START_ON`: start moving 0.3 s after spawning.
pub const SF_GUNTARGET_START_ON: i32 = 1;

/// `gSkillData.monHead` .. `monLeg` (0x191a4c..0x191a5c), the `CBaseMonster::TraceAttack`
/// hitgroup multipliers. Nothing in TS writes them (`CGameRules::RefreshSkillData` 0xc5fd8 only
/// sets the skill level, `CHalfLifeMultiplay`'s 0xcef78 only player damages), so they are 0 in
/// `.bss`.
const MON_HITGROUP_SCALE: f32 = 0.0;

/// The `CBaseMonster::TraceAttack` hitgroup switch: head, chest, stomach, arms and legs (1–7) are
/// scaled by [`MON_HITGROUP_SCALE`]; generic (0, every brush hit) and anything else is kept.
fn monster_hitgroup_damage(hitgroup: i32, damage: f32) -> f32 {
    match hitgroup {
        1..=7 => damage * MON_HITGROUP_SCALE,
        _ => damage,
    }
}

/// `func_guntarget` (`CGunTarget`, 0xd6e48..0xd7da8): a shootable brush that moves along a
/// `path_corner` chain once used and fires `message` when its health runs out.
///
/// The SDK class is a `CBaseMonster`, and so is TS's, but only for its vtable: `Spawn`,
/// `TakeDamage`, `Use` and the thinks are `CGunTarget`'s own and never reach monster code
/// (`TakeDamage` doesn't call `Killed`). The inherited `CBaseMonster::TraceAttack` (0xb3848) runs
/// for traced hits, `CBaseMonster::KeyValue` (0xce80c) is just `CBaseToggle::KeyValue`, and
/// `FL_MONSTER` is set. `ObjectCaps` (0xd7da8) is 0.
///
/// `CGunTarget::Activate` (0xd6f38) runs from `ServerActivate`; the entity framework has no
/// activate hook, so it runs from a think 0.1 s (local time) after spawning, or from `Use` if that
/// comes first, as `func_train` does.
#[derive(Default)]
pub struct GunTarget {
    t: Toggle<Self>,
    /// `m_hTargetEnt` (+0x138).
    target_ent: EHandle,
    activated: bool,
    /// When `Spawn` scheduled `Start` (`ltime + 0.3`), kept while the activate think runs first.
    start_at: f32,
}

impl Mover for GunTarget {
    fn toggle(&mut self) -> &mut Toggle<Self> {
        &mut self.t
    }
}

/// `CBaseEntity::GetNextTarget` (0xabd88): the entity named by `target`, or null (no target, not
/// found or the world, or an edict without a game object).
fn get_next_target(cx: &mut Cx) -> *mut edict_t {
    let target = cx.pev().target;
    if target == 0 {
        return std::ptr::null_mut();
    }
    let e = engine().find_by_target_name(std::ptr::null_mut(), s(target));
    if e.is_null() || !e.has_private_data() { std::ptr::null_mut() } else { e }
}

impl GunTarget {
    /// `CGunTarget::Activate` (0xd6f38): start at the first path corner.
    fn activate(&mut self, cx: &mut Cx) {
        self.activated = true;
        let t = get_next_target(cx);
        if t.is_null() {
            return;
        }
        self.target_ent = EHandle::new(t);
        let v = cx.pev();
        let dest = vec::sub(t.v().origin, vec::mid(v.mins, v.maxs));
        engine().set_origin(cx.edict, dest);
    }

    /// Stand-in for `ServerActivate` (see the type docs), then `Spawn`'s `Start` think if it set one.
    fn activate_think(&mut self, cx: &mut Cx) {
        if !self.activated {
            self.activate(cx);
        }
        if cx.pev().spawnflags & SF_GUNTARGET_START_ON != 0 {
            self.t.think = Some(Self::start);
            cx.pev().nextthink = self.start_at;
        }
    }

    /// `CGunTarget::Start` (0xd701c).
    fn start(&mut self, cx: &mut Cx) {
        let me = cx.edict;
        self.use_(cx, me, me, UseType::On, 0.0);
    }

    /// `CGunTarget::Next` (0xd7038).
    fn next(&mut self, cx: &mut Cx) {
        self.t.think = None;
        self.target_ent = EHandle::new(get_next_target(cx));
        let t = self.target_ent.get();
        if t.is_null() {
            self.stop(cx);
            return;
        }
        self.t.move_done = Some(Self::wait);
        let v = cx.pev();
        let dest = vec::sub(t.v().origin, vec::mid(v.mins, v.maxs));
        let speed = v.speed;
        linear_move(self, cx, dest, speed);
    }

    /// `CGunTarget::Wait` (0xd7160): at a corner, fire its `message`, wait its delay, head for its
    /// `target`.
    fn wait(&mut self, cx: &mut Cx) {
        let t = self.target_ent.get();
        if t.is_null() {
            self.stop(cx);
            return;
        }
        let message = t.v().message;
        if message != 0 {
            let me = cx.edict;
            fire_targets(s(message), me, me, UseType::Toggle, 0.0);
            let tv = t.v();
            if tv.spawnflags & SF_CORNER_FIREONCE != 0 {
                tv.message = 0;
            }
        }
        self.t.wait = path_delay(t);
        cx.pev().target = t.v().target;
        self.t.think = Some(Self::next);
        if self.t.wait == 0.0 {
            self.next(cx);
        } else {
            let w = self.t.wait;
            think_ltime(cx, w);
        }
    }

    /// `CGunTarget::Stop` (0xd7258).
    fn stop(&mut self, cx: &mut Cx) {
        let v = cx.pev();
        v.velocity = [0.0; 3];
        v.nextthink = 0.0;
        v.takedamage = DAMAGE_NO as f32;
    }
}

impl Entity for GunTarget {
    /// `CBaseMonster::KeyValue` (0xce80c) = `CBaseToggle::KeyValue`.
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        self.t.key_value(key, value)
    }

    /// `CGunTarget::Spawn` (0xd6e48).
    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        let v = cx.pev();
        v.solid = SOLID_BSP as i32;
        v.movetype = MOVETYPE_PUSH as i32;
        let org = v.origin;
        eng.set_origin(cx.edict, org);
        set_own_model(cx);
        let v = cx.pev();
        if v.speed == 0.0 {
            v.speed = 100.0;
        }
        v.takedamage = DAMAGE_NO as f32;
        v.flags |= FL_MONSTER as i32;
        v.max_health = v.health;
        self.start_at = v.ltime + 0.3;
        // Stand-in for CGunTarget::Activate (see the type docs); it schedules `Start` at
        // `start_at` for FGT_START_ON.
        self.activated = false;
        self.t.think = Some(Self::activate_think);
        think_ltime(cx, 0.1);
    }

    fn think(&mut self, cx: &mut Cx) {
        run_think(self, cx);
    }

    /// `CGunTarget::Use` (0xd733c). `m_on` (+0x164) is cleared by `Spawn` and never set (as in
    /// the SDK), so every use that passes `ShouldToggle` (re)starts the target at full health.
    fn use_(&mut self, cx: &mut Cx, _a: *mut edict_t, _c: *mut edict_t, kind: UseType, _v: f32) {
        if !self.activated {
            self.activate(cx);
        }
        let on = false;
        if !should_toggle(kind, on) {
            return;
        }
        cx.pev().takedamage = DAMAGE_AIM as f32;
        self.target_ent = EHandle::new(get_next_target(cx));
        if !self.target_ent.is_valid() {
            return;
        }
        let v = cx.pev();
        v.health = v.max_health;
        self.next(cx);
    }

    /// `CBaseMonster::TraceAttack` (0xb3848): with `takedamage` on, the hitgroup multiplier
    /// (0 for hitgroups 1–7, see [`MON_HITGROUP_SCALE`]), no blood (`BloodColor` is
    /// `DONT_BLEED`, so `SpawnBlood` and `TraceBleed` do nothing), into the multi-damage.
    fn trace_attack(&mut self, cx: &mut Cx, dmg: &DamageInfo) {
        if cx.pev().takedamage == 0.0 {
            return;
        }
        let damage = monster_hitgroup_damage(dmg.hitgroup, dmg.damage);
        crate::damage::multi::add(dmg.attacker, cx.edict, damage, dmg.bits);
    }

    /// `CGunTarget::TakeDamage` (0xd729c): wear the health down; at 0 stop and fire `message`.
    /// Always returns 0.
    fn take_damage(&mut self, cx: &mut Cx, dmg: &DamageInfo) -> bool {
        let v = cx.pev();
        if v.health > 0.0 {
            v.health -= dmg.damage;
            if v.health <= 0.0 {
                v.health = 0.0;
                self.stop(cx);
                let message = cx.pev().message;
                if message != 0 {
                    let me = cx.edict;
                    fire_targets(s(message), me, me, UseType::Toggle, 0.0);
                }
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plat_sound_bytes() {
        assert_eq!(plat_sound_index(3.0), 3);
        assert_eq!(plat_sound_index(3.9), 3, "truncated");
        assert_eq!(plat_sound_index(-0.5), 0);
        assert_eq!(plat_sound_index(257.0), 1, "low byte of the short");
        assert_eq!(plat_sound_index(-255.0), 1);
        assert_eq!(plat_sound_index(40000.0), 0, "out of short range: 0x8000");
        assert_eq!(plat_sound_index(f64::NAN), 0);
        // The double, not a float: 255.99999999 truncates to 255 (as a float it would be 256).
        assert_eq!(plat_sound_index(kv_d(c"255.99999999")), 255);
        assert_eq!(plat_sound_index(kv_d(c"32767.9")), 255);
        assert_eq!(plat_sound_index(kv_d(c"32768")), 0);
    }

    #[test]
    fn rot_move_split() {
        // A move shorter than 0.1 s (or none: LinearMoveDone left nextthink at -1) turns the
        // whole way in the next second.
        assert_eq!(rot_move_velocity([0.0, 90.0, 0.0], 0.05), ([0.0, 90.0, 0.0], true));
        assert_eq!(rot_move_velocity([0.0, 90.0, 0.0], -5.0), ([0.0, 90.0, 0.0], true));
        assert_eq!(rot_move_velocity([0.0, 90.0, -30.0], 2.0), ([0.0, 45.0, -15.0], false));
    }

    #[test]
    fn guntarget_hitgroups() {
        assert_eq!(monster_hitgroup_damage(0, 12.0), 12.0);
        for g in 1..=7 {
            assert_eq!(monster_hitgroup_damage(g, 12.0), 0.0);
        }
        assert_eq!(monster_hitgroup_damage(10, 12.0), 12.0);
    }
}
