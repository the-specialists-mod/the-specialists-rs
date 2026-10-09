//! `func_train` and `path_corner` (SDK plats.cpp / pathcorner.cpp; same in TS).
//!
//! The SDK starts trains from `CFuncTrain::Activate`, which `ServerActivate` calls once every
//! entity has spawned. The entity framework has no activate hook, so the train runs `Activate`
//! from a think 0.1 s (local time) after spawning instead.

use super::base::*;
use crate::entities::fire_targets;
use crate::world::{self, Cx, DamageInfo, Entity, UseType};
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
    move_snd: i32,
    stop_snd: i32,
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
            let cv = unsafe { &mut (*self.current.get()).v };
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
        let tv = unsafe { &(*targ).v };
        let v = cx.pev();
        v.message = v.target;
        v.target = tv.target;
        self.t.wait = path_delay(targ);
        if self.current_valid() {
            let cs = unsafe { (*self.current.get()).v.speed };
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
        let tv = unsafe { &(*targ).v };
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
            b"movesnd" => self.move_snd = kv_f(value) as i32,
            b"stopsnd" => self.stop_snd = kv_f(value) as i32,
            b"volume" => self.volume = kv_f(value),
            _ => return self.t.key_value(key, value),
        }
        true
    }

    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        let (m, st) = (plat_move_sound(self.move_snd), plat_stop_sound(self.stop_snd));
        eng.precache_sound(m);
        eng.precache_sound(st);
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
                v.target = unsafe { (*v.enemy).v.targetname };
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

// ---- func_plat ------------------------------------------------------------------------------------------

pub const SF_PLAT_TOGGLE: i32 = 1;

/// `func_plat` (CFuncPlat): a lift that rises when a player steps into its trigger volume (or
/// toggles on use with spawnflag 1).
#[derive(Default)]
pub struct FuncPlat {
    t: Toggle<Self>,
    move_snd: i32,
    stop_snd: i32,
    volume: f32,
    t_length: f32,
    t_width: f32,
    usable: bool,
}

impl Mover for FuncPlat {
    fn toggle(&mut self) -> &mut Toggle<Self> {
        &mut self.t
    }
}

impl FuncPlat {
    fn is_toggle(cx: &mut Cx) -> bool {
        cx.pev().spawnflags & SF_PLAT_TOGGLE != 0
    }

    fn move_sound(&self, cx: &mut Cx) {
        let n = cx.pev().noise;
        if n != 0 {
            emit(cx.edict, CHAN_STATIC, s(n), self.volume, ATTN_NORM, 0, PITCH_NORM as i32);
        }
    }

    fn stop_sounds(&self, cx: &mut Cx) {
        let v = cx.pev();
        let (n, n1) = (v.noise, v.noise1);
        if n != 0 {
            engine().emit_sound(cx.edict, CHAN_STATIC as i32, s(n), 0.0, 0.0, snd::STOP, PITCH_NORM as i32);
        }
        if n1 != 0 {
            emit(cx.edict, CHAN_WEAPON, s(n1), self.volume, ATTN_NORM, 0, PITCH_NORM as i32);
        }
    }

    pub fn go_up(&mut self, cx: &mut Cx) {
        self.move_sound(cx);
        self.t.state = ToggleState::GoingUp;
        self.t.move_done = Some(Self::hit_top);
        let (p, sp) = (self.t.pos1, cx.pev().speed);
        linear_move(self, cx, p, sp);
    }

    fn go_down(&mut self, cx: &mut Cx) {
        self.move_sound(cx);
        self.t.state = ToggleState::GoingDown;
        self.t.move_done = Some(Self::hit_bottom);
        let (p, sp) = (self.t.pos2, cx.pev().speed);
        linear_move(self, cx, p, sp);
    }

    fn hit_top(&mut self, cx: &mut Cx) {
        self.stop_sounds(cx);
        self.t.state = ToggleState::AtTop;
        if !Self::is_toggle(cx) {
            self.t.think = Some(Self::go_down);
            think_ltime(cx, 3.0);
        }
    }

    fn hit_bottom(&mut self, cx: &mut Cx) {
        self.stop_sounds(cx);
        self.t.state = ToggleState::AtBottom;
    }

    /// `CPlatTrigger::Touch` on behalf of the trigger volume.
    fn trigger_touch(&mut self, cx: &mut Cx) {
        match self.t.state {
            ToggleState::AtBottom => self.go_up(cx),
            ToggleState::AtTop => think_ltime(cx, 1.0),
            _ => {}
        }
    }
}

impl Entity for FuncPlat {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"height" => self.t.height = kv_f(value),
            b"rotation" => self.t.final_angle[0] = kv_f(value),
            b"movesnd" => self.move_snd = kv_f(value) as i32,
            b"stopsnd" => self.stop_snd = kv_f(value) as i32,
            b"volume" => self.volume = kv_f(value),
            _ => return self.t.key_value(key, value),
        }
        true
    }

    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        // Setup
        if self.t_length == 0.0 {
            self.t_length = 80.0;
        }
        if self.t_width == 0.0 {
            self.t_width = 10.0;
        }
        let v = cx.pev();
        v.angles = [0.0; 3];
        v.solid = SOLID_BSP as i32;
        v.movetype = MOVETYPE_PUSH as i32;
        let org = v.origin;
        eng.set_origin(cx.edict, org);
        set_own_model(cx);
        let v = cx.pev();
        self.t.pos1 = v.origin;
        self.t.pos2 = v.origin;
        self.t.pos2[2] = if self.t.height != 0.0 { v.origin[2] - self.t.height } else { v.origin[2] - v.size[2] + 8.0 };
        if v.speed == 0.0 {
            v.speed = 150.0;
        }
        if self.volume == 0.0 {
            self.volume = 0.85;
        }
        // Precache
        let (m, st) = (plat_move_sound(self.move_snd), plat_stop_sound(self.stop_snd));
        eng.precache_sound(m);
        eng.precache_sound(st);
        let v = cx.pev();
        v.noise = alloc(m);
        v.noise1 = alloc(st);
        if !Self::is_toggle(cx) {
            spawn_plat_trigger(cx, self.t.pos1, self.t.pos2);
        }
        if cx.pev().targetname != 0 {
            let p = self.t.pos1;
            eng.set_origin(cx.edict, p);
            self.t.state = ToggleState::AtTop;
            self.usable = true;
        } else {
            let p = self.t.pos2;
            eng.set_origin(cx.edict, p);
            self.t.state = ToggleState::AtBottom;
        }
    }

    fn think(&mut self, cx: &mut Cx) {
        run_think(self, cx);
    }

    fn use_(&mut self, cx: &mut Cx, _a: *mut edict_t, _c: *mut edict_t, kind: UseType, _v: f32) {
        if !self.usable {
            return;
        }
        if Self::is_toggle(cx) {
            let on = self.t.state == ToggleState::AtBottom;
            if !should_toggle(kind, on) {
                return;
            }
            match self.t.state {
                ToggleState::AtTop => self.go_down(cx),
                ToggleState::AtBottom => self.go_up(cx),
                _ => {}
            }
        } else {
            self.usable = false;
            if self.t.state == ToggleState::AtTop {
                self.go_down(cx);
            }
        }
    }

    fn blocked(&mut self, cx: &mut Cx, other: *mut edict_t) {
        let me = cx.edict;
        crate::damage::apply(other, &DamageInfo::new(me, me, 1.0, crate::damage::bits::CRUSH));
        let n = cx.pev().noise;
        if n != 0 {
            engine().emit_sound(me, CHAN_STATIC as i32, s(n), 0.0, 0.0, snd::STOP, PITCH_NORM as i32);
        }
        match self.t.state {
            ToggleState::GoingUp => self.go_down(cx),
            ToggleState::GoingDown => self.go_up(cx),
            _ => {}
        }
    }
}

/// `PlatSpawnInsideTrigger`: the trigger volume that calls the lift.
fn spawn_plat_trigger(cx: &mut Cx, pos1: Vec3, pos2: Vec3) {
    let eng = engine();
    let e = eng.create_entity();
    if e.is_null() {
        return;
    }
    world::create::<PlatTrigger>(unsafe { &mut (*e).v });
    let platform = EHandle::new(cx.edict);
    world::with::<PlatTrigger, _>(e, |t| t.platform = platform);
    let pv = cx.pev();
    let tv = unsafe { &mut (*e).v };
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
    fn touch(&mut self, _cx: &mut Cx, other: *mut edict_t) {
        if !goldsrc::ext::entities::is_player(other) {
            return;
        }
        let ov = unsafe { &(*other).v };
        if ov.deadflag != DEAD_NO as i32 || ov.health <= 0.0 {
            return;
        }
        let p = self.platform.get();
        if p.is_null() {
            return;
        }
        world::dispatch(p, |ent, pcx| {
            if let Some(plat) = (ent as &mut dyn std::any::Any).downcast_mut::<FuncPlat>() {
                plat.trigger_touch(pcx);
            }
        });
    }
}
