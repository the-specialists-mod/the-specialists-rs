//! `func_button`, `func_rot_button`, `button_target`, `multisource` (SDK buttons.cpp; identical in
//! TS apart from TS's slow-motion pitch on spark sounds).
//!
//! Lock sentences (`locked_sentence` / `unlocked_sentence`) need the sentence groups from
//! sentences.txt; they're parsed but not played (TS maps don't rely on them).

use super::base::*;
use super::explode::{emit_sound_slow, sparks};
use crate::world::{Cx, DamageInfo, Entity, UseType};
use goldsrc::edict::EdictPtr;
use goldsrc::ext::entities::{ATTN_NORM, EHandle, Vec3, is_player, vec};
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::ffi::CStr;

/// SDK `ButtonSound`.
pub fn button_sound(n: i32) -> &'static CStr {
    match n {
        0 => c"common/null.wav",
        1 => c"buttons/button1.wav",
        2 => c"buttons/button2.wav",
        3 => c"buttons/button3.wav",
        4 => c"buttons/button4.wav",
        5 => c"buttons/button5.wav",
        6 => c"buttons/button6.wav",
        7 => c"buttons/button7.wav",
        8 => c"buttons/button8.wav",
        9 => c"buttons/button9.wav",
        10 => c"buttons/button10.wav",
        11 => c"buttons/button11.wav",
        12 => c"buttons/latchlocked1.wav",
        13 => c"buttons/latchunlocked1.wav",
        14 => c"buttons/lightswitch2.wav",
        21 => c"buttons/lever1.wav",
        22 => c"buttons/lever2.wav",
        23 => c"buttons/lever3.wav",
        24 => c"buttons/lever4.wav",
        25 => c"buttons/lever5.wav",
        _ => c"buttons/button9.wav",
    }
}

pub const DOOR_SOUNDWAIT: f32 = 3.0;
pub const BUTTON_SOUNDWAIT: f32 = 0.5;

/// SDK `locksound_t` (sounds only).
#[derive(Default)]
pub struct LockSounds {
    pub locked_n: i32,
    pub unlocked_n: i32,
    pub locked_sentence: i32,
    pub unlocked_sentence: i32,
    pub locked: Option<&'static CStr>,
    pub unlocked: Option<&'static CStr>,
    pub wait_sound: f32,
}

impl LockSounds {
    pub fn key_value(&mut self, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"locked_sound" => self.locked_n = kv_f(value) as i32,
            b"unlocked_sound" => self.unlocked_n = kv_f(value) as i32,
            b"locked_sentence" => self.locked_sentence = kv_f(value) as i32,
            b"unlocked_sentence" => self.unlocked_sentence = kv_f(value) as i32,
            _ => return false,
        }
        true
    }

    pub fn precache(&mut self) {
        if self.locked_n != 0 {
            let s = button_sound(self.locked_n);
            engine().precache_sound(s);
            self.locked = Some(s);
        }
        if self.unlocked_n != 0 {
            let s = button_sound(self.unlocked_n);
            engine().precache_sound(s);
            self.unlocked = Some(s);
        }
    }

    /// `PlayLockSounds(pev, pls, flocked, fbutton)` without sentences.
    pub fn play(&mut self, e: *mut edict_t, locked: bool, button: bool) {
        let now = engine().time();
        let snd = if locked { self.locked } else { self.unlocked };
        if let Some(s) = snd {
            if now > self.wait_sound {
                emit(e, CHAN_ITEM, s, 1.0, ATTN_NORM, 0, PITCH_NORM as i32);
                self.wait_sound = now + if button { BUTTON_SOUNDWAIT } else { DOOR_SOUNDWAIT };
            }
        }
    }
}

/// SDK `DoSpark` (TS: spark pitch follows the entity's time scale, factor 0.5).
pub fn do_spark(e: *mut edict_t, location: Vec3) {
    let eng = engine();
    let size = e.v().size;
    sparks(vec::mad(location, size, 0.5));
    let vol = eng.random_float(0.25, 0.75) * 0.4;
    let s: &CStr = match (eng.random_float(0.0, 1.0) * 6.0) as i32 {
        0 => c"buttons/spark1.wav",
        1 => c"buttons/spark2.wav",
        2 => c"buttons/spark3.wav",
        3 => c"buttons/spark4.wav",
        4 => c"buttons/spark5.wav",
        _ => c"buttons/spark6.wav",
    };
    emit_sound_slow(e, CHAN_VOICE, s, vol, ATTN_NORM, 0, PITCH_NORM as i32, 0.5);
}

pub fn precache_sparks() {
    for s in [
        c"buttons/spark1.wav",
        c"buttons/spark2.wav",
        c"buttons/spark3.wav",
        c"buttons/spark4.wav",
        c"buttons/spark5.wav",
        c"buttons/spark6.wav",
    ] {
        engine().precache_sound(s);
    }
}

pub const SF_BUTTON_DONTMOVE: i32 = 1;
pub const SF_ROTBUTTON_NOTSOLID: i32 = 1;
pub const SF_BUTTON_TOGGLE: i32 = 32;
pub const SF_BUTTON_SPARK_IF_OFF: i32 = 64;
pub const SF_BUTTON_TOUCH_ONLY: i32 = 256;
const SF_DOOR_ROTATE_BACKWARDS: i32 = 2;

#[derive(PartialEq, Eq)]
enum Code {
    Nothing,
    Activate,
    Return,
}

/// `func_button` (`ROT = false`) and `func_rot_button` (`ROT = true`).
#[derive(Default)]
pub struct Button<const ROT: bool> {
    t: Toggle<Self>,
    stay_pushed: bool,
    rotating: bool,
    change_target: string_t,
    ls: LockSounds,
    sounds: i32,
    /// SetUse(ButtonUse) vs SetUse(NULL).
    usable: bool,
}

impl<const ROT: bool> Mover for Button<ROT> {
    fn toggle(&mut self) -> &mut Toggle<Self> {
        &mut self.t
    }
}

impl<const ROT: bool> Button<ROT> {
    fn precache(&mut self, cx: &mut Cx) {
        if cx.pev().spawnflags & SF_BUTTON_SPARK_IF_OFF != 0 {
            precache_sparks();
        }
        self.ls.precache();
    }

    fn noise(cx: &mut Cx) -> &'static CStr {
        s(cx.pev().noise)
    }

    fn response_to_touch(&self, cx: &mut Cx) -> Code {
        let sf = cx.pev().spawnflags;
        let st = self.t.state;
        if st == ToggleState::GoingUp
            || st == ToggleState::GoingDown
            || (st == ToggleState::AtTop && !self.stay_pushed && sf & SF_BUTTON_TOGGLE == 0)
        {
            return Code::Nothing;
        }
        if st == ToggleState::AtTop {
            if sf & SF_BUTTON_TOGGLE != 0 && !self.stay_pushed {
                return Code::Return;
            }
            Code::Nothing
        } else {
            Code::Activate
        }
    }

    fn button_spark(&mut self, cx: &mut Cx) {
        self.t.think = Some(Self::button_spark);
        let delay = 0.1 + engine().random_float(0.0, 1.5);
        cx.pev().nextthink = engine().time() + delay;
        let mins = cx.pev().mins;
        do_spark(cx.edict, mins);
    }

    fn button_use(&mut self, cx: &mut Cx, activator: *mut edict_t) {
        let st = self.t.state;
        if st == ToggleState::GoingUp || st == ToggleState::GoingDown {
            return;
        }
        self.t.activator = EHandle::new(activator);
        if st == ToggleState::AtTop {
            if !self.stay_pushed && cx.pev().spawnflags & SF_BUTTON_TOGGLE != 0 {
                emit(cx.edict, CHAN_VOICE, Self::noise(cx), 1.0, ATTN_NORM, 0, PITCH_NORM as i32);
                self.button_return(cx);
            }
        } else {
            self.button_activate(cx);
        }
    }

    fn button_touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        if !is_player(other) {
            return;
        }
        self.t.activator = EHandle::new(other);
        let code = self.response_to_touch(cx);
        if code == Code::Nothing {
            return;
        }
        if !is_master_triggered(self.t.master, other) {
            self.ls.play(cx.edict, true, true);
            return;
        }
        self.t.touch = None;
        if code == Code::Return {
            emit(cx.edict, CHAN_VOICE, Self::noise(cx), 1.0, ATTN_NORM, 0, PITCH_NORM as i32);
            self.t.use_targets(cx.edict, other, UseType::Toggle, 0.0);
            self.button_return(cx);
        } else {
            self.button_activate(cx);
        }
    }

    fn button_activate(&mut self, cx: &mut Cx) {
        emit(cx.edict, CHAN_VOICE, Self::noise(cx), 1.0, ATTN_NORM, 0, PITCH_NORM as i32);
        if !is_master_triggered(self.t.master, self.t.activator.get()) {
            self.ls.play(cx.edict, true, true);
            return;
        }
        self.ls.play(cx.edict, false, true);
        self.t.state = ToggleState::GoingUp;
        self.t.move_done = Some(Self::trigger_and_wait);
        let speed = cx.pev().speed;
        if !self.rotating {
            let p = self.t.pos2;
            linear_move(self, cx, p, speed);
        } else {
            let a = self.t.angle2;
            angular_move(self, cx, a, speed);
        }
    }

    fn trigger_and_wait(&mut self, cx: &mut Cx) {
        if !is_master_triggered(self.t.master, self.t.activator.get()) {
            return;
        }
        self.t.state = ToggleState::AtTop;
        let sf = cx.pev().spawnflags;
        if self.stay_pushed || sf & SF_BUTTON_TOGGLE != 0 {
            self.t.touch = if sf & SF_BUTTON_TOUCH_ONLY == 0 { None } else { Some(Self::button_touch) };
        } else {
            let w = self.t.wait;
            think_ltime(cx, w);
            self.t.think = Some(Self::button_return);
        }
        cx.pev().frame = 1.0;
        let act = self.t.activator.get();
        self.t.use_targets(cx.edict, act, UseType::Toggle, 0.0);
    }

    fn button_return(&mut self, cx: &mut Cx) {
        self.t.state = ToggleState::GoingDown;
        self.t.move_done = Some(Self::button_back_home);
        let speed = cx.pev().speed;
        if !self.rotating {
            let p = self.t.pos1;
            linear_move(self, cx, p, speed);
        } else {
            let a = self.t.angle1;
            angular_move(self, cx, a, speed);
        }
        cx.pev().frame = 0.0;
    }

    fn button_back_home(&mut self, cx: &mut Cx) {
        self.t.state = ToggleState::AtBottom;
        let sf = cx.pev().spawnflags;
        let act = self.t.activator.get();
        if sf & SF_BUTTON_TOGGLE != 0 {
            self.t.use_targets(cx.edict, act, UseType::Toggle, 0.0);
        }
        let target = cx.pev().target;
        if target != 0 {
            let eng = engine();
            let mut e = std::ptr::null_mut();
            loop {
                e = eng.find_by_target_name(e, s(target));
                if e.is_null() {
                    break;
                }
                if s(e.v().classname) != c"multisource" {
                    continue;
                }
                let me = cx.edict;
                crate::world::dispatch(e, |ent, ecx| ent.use_(ecx, act, me, UseType::Toggle, 0.0));
            }
        }
        self.t.touch = if sf & SF_BUTTON_TOUCH_ONLY == 0 { None } else { Some(Self::button_touch) };
        if sf & SF_BUTTON_SPARK_IF_OFF != 0 {
            self.t.think = Some(Self::button_spark);
            cx.pev().nextthink = engine().time() + 0.5;
        }
    }

    fn spawn_button(&mut self, cx: &mut Cx) {
        let snd = button_sound(self.sounds);
        engine().precache_sound(snd);
        cx.pev().noise = alloc(snd);
        self.precache(cx);
        if cx.pev().spawnflags & SF_BUTTON_SPARK_IF_OFF != 0 {
            self.t.think = Some(Self::button_spark);
            cx.pev().nextthink = engine().time() + 0.5;
        }
        set_movedir(cx.pev());
        let v = cx.pev();
        v.movetype = MOVETYPE_PUSH as i32;
        v.solid = SOLID_BSP as i32;
        set_own_model(cx);
        let v = cx.pev();
        if v.speed == 0.0 {
            v.speed = 40.0;
        }
        if v.health > 0.0 {
            v.takedamage = DAMAGE_YES as f32;
        }
        if self.t.wait == 0.0 {
            self.t.wait = 1.0;
        }
        if self.t.lip == 0.0 {
            self.t.lip = 4.0;
        }
        self.t.state = ToggleState::AtBottom;
        self.t.pos1 = v.origin;
        let md = v.movedir;
        let dist = (md[0] * (v.size[0] - 2.0)).abs() + (md[1] * (v.size[1] - 2.0)).abs() + (md[2] * (v.size[2] - 2.0)).abs() - self.t.lip;
        self.t.pos2 = vec::mad(self.t.pos1, md, dist);
        if vec::length(vec::sub(self.t.pos2, self.t.pos1)) < 1.0 || v.spawnflags & SF_BUTTON_DONTMOVE != 0 {
            self.t.pos2 = self.t.pos1;
        }
        self.stay_pushed = self.t.wait == -1.0;
        self.rotating = false;
        if v.spawnflags & SF_BUTTON_TOUCH_ONLY != 0 {
            self.t.touch = Some(Self::button_touch);
            self.usable = false;
        } else {
            self.t.touch = None;
            self.usable = true;
        }
    }

    fn spawn_rot_button(&mut self, cx: &mut Cx) {
        let snd = button_sound(self.sounds);
        engine().precache_sound(snd);
        cx.pev().noise = alloc(snd);
        let v = cx.pev();
        axis_dir(v);
        if v.spawnflags & SF_DOOR_ROTATE_BACKWARDS != 0 {
            v.movedir = vec::scale(v.movedir, -1.0);
        }
        v.movetype = MOVETYPE_PUSH as i32;
        v.solid = if v.spawnflags & SF_ROTBUTTON_NOTSOLID != 0 { SOLID_NOT } else { SOLID_BSP } as i32;
        set_own_model(cx);
        let v = cx.pev();
        if v.speed == 0.0 {
            v.speed = 40.0;
        }
        if self.t.wait == 0.0 {
            self.t.wait = 1.0;
        }
        if v.health > 0.0 {
            v.takedamage = DAMAGE_YES as f32;
        }
        self.t.state = ToggleState::AtBottom;
        self.t.angle1 = v.angles;
        self.t.angle2 = vec::mad(v.angles, v.movedir, self.t.move_distance);
        self.stay_pushed = self.t.wait == -1.0;
        self.rotating = true;
        if v.spawnflags & SF_BUTTON_TOUCH_ONLY == 0 {
            self.t.touch = None;
            self.usable = true;
        } else {
            self.t.touch = Some(Self::button_touch);
            self.usable = false;
        }
    }
}

impl<const ROT: bool> Entity for Button<ROT> {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"changetarget" => self.change_target = alloc(value),
            b"sounds" => self.sounds = kv_i(value),
            _ => return self.ls.key_value(key, value) || self.t.key_value(key, value),
        }
        true
    }

    fn spawn(&mut self, cx: &mut Cx) {
        if ROT { self.spawn_rot_button(cx) } else { self.spawn_button(cx) }
    }

    fn think(&mut self, cx: &mut Cx) {
        run_think(self, cx);
    }

    fn touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        run_touch(self, cx, other);
    }

    fn use_(&mut self, cx: &mut Cx, activator: *mut edict_t, _caller: *mut edict_t, _kind: UseType, _value: f32) {
        if self.usable {
            self.button_use(cx, activator);
        }
    }

    fn take_damage(&mut self, cx: &mut Cx, info: &DamageInfo) -> bool {
        let code = self.response_to_touch(cx);
        if code == Code::Nothing {
            return false;
        }
        self.t.touch = None;
        self.t.activator = EHandle::new(info.attacker);
        if info.attacker.is_null() {
            return false;
        }
        if code == Code::Return {
            emit(cx.edict, CHAN_VOICE, Self::noise(cx), 1.0, ATTN_NORM, 0, PITCH_NORM as i32);
            if cx.pev().spawnflags & SF_BUTTON_TOGGLE == 0 {
                self.t.use_targets(cx.edict, info.attacker, UseType::Toggle, 0.0);
            }
            self.button_return(cx);
        } else {
            self.button_activate(cx);
        }
        false
    }
}

impl<const ROT: bool> Button<ROT> {
    /// `ObjectCaps`: usable with +use unless it's shootable.
    pub fn impulse_use(cx_pev: &entvars_t) -> bool {
        cx_pev.takedamage == 0.0
    }
}

// ---- button_target ----------------------------------------------------------------------------------

pub const SF_BTARGET_USE: i32 = 1;
pub const SF_BTARGET_ON: i32 = 2;

#[derive(Default)]
pub struct ButtonTarget {
    delay: Delay,
}

impl Entity for ButtonTarget {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        self.delay.key_value(key, value)
    }

    fn spawn(&mut self, cx: &mut Cx) {
        let v = cx.pev();
        v.movetype = MOVETYPE_PUSH as i32;
        v.solid = SOLID_BSP as i32;
        set_own_model(cx);
        let v = cx.pev();
        v.takedamage = DAMAGE_YES as f32;
        if v.spawnflags & SF_BTARGET_ON != 0 {
            v.frame = 1.0;
        }
    }

    fn use_(&mut self, cx: &mut Cx, activator: *mut edict_t, _caller: *mut edict_t, kind: UseType, _value: f32) {
        let v = cx.pev();
        if !should_toggle(kind, v.frame != 0.0) {
            return;
        }
        v.frame = 1.0 - v.frame;
        let k = if v.frame != 0.0 { UseType::On } else { UseType::Off };
        self.delay.use_targets(cx.edict, activator, k, 0.0);
    }

    fn take_damage(&mut self, cx: &mut Cx, info: &DamageInfo) -> bool {
        let me = cx.edict;
        self.use_(cx, info.attacker, me, UseType::Toggle, 0.0);
        true
    }
}

// ---- multisource -------------------------------------------------------------------------------------

const SF_MULTI_INIT: i32 = 1;
const MS_MAX_TARGETS: usize = 32;

#[derive(Default)]
pub struct MultiSource {
    delay: Delay,
    entities: Vec<EHandle>,
    triggered: Vec<bool>,
    globalstate: string_t,
    registered: bool,
}

impl MultiSource {
    fn publish(&self, cx: &mut Cx) {
        let init = cx.pev().spawnflags & SF_MULTI_INIT != 0;
        set_master_state(cx.edict, MasterState { init, all_on: self.triggered.iter().all(|t| *t), global: self.globalstate });
    }

    fn register(&mut self, cx: &mut Cx) {
        let eng = engine();
        self.entities.clear();
        let name = s(cx.pev().targetname);
        let mut e = std::ptr::null_mut();
        loop {
            e = eng.find_by_string(e, c"target", name);
            if e.is_null() || self.entities.len() >= MS_MAX_TARGETS {
                break;
            }
            if e.has_private_data() {
                self.entities.push(EHandle::new(e));
            }
        }
        let mut e = std::ptr::null_mut();
        loop {
            e = eng.find_by_string(e, c"classname", c"multi_manager");
            if e.is_null() || self.entities.len() >= MS_MAX_TARGETS {
                break;
            }
            if super::triggers::multi_manager_has_target(e, name) {
                self.entities.push(EHandle::new(e));
            }
        }
        self.triggered = vec![false; self.entities.len()];
        cx.pev().spawnflags &= !SF_MULTI_INIT;
        self.registered = true;
        self.publish(cx);
    }
}

impl Entity for MultiSource {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"style" | b"height" | b"killtarget" | b"value1" | b"value2" | b"value3" => {}
            b"globalstate" => self.globalstate = alloc(value),
            _ => return false,
        }
        true
    }

    fn spawn(&mut self, cx: &mut Cx) {
        let v = cx.pev();
        v.solid = SOLID_NOT as i32;
        v.movetype = MOVETYPE_NONE as i32;
        v.nextthink = engine().time() + 0.1;
        v.spawnflags |= SF_MULTI_INIT;
        self.publish(cx);
    }

    fn think(&mut self, cx: &mut Cx) {
        if !self.registered {
            self.register(cx);
        }
    }

    fn use_(&mut self, cx: &mut Cx, _activator: *mut edict_t, caller: *mut edict_t, _kind: UseType, _value: f32) {
        let Some(i) = self.entities.iter().position(|h| h.get() == caller && !caller.is_null()) else {
            let cls = if caller.is_null() { c"" } else { s(caller.v().classname) };
            engine().alert(ALERT_TYPE::at_console, &format!("MultiSrc:Used by non member {}.\n", cls.to_string_lossy()));
            return;
        };
        self.triggered[i] ^= true;
        self.publish(cx);
        let all = self.triggered.iter().all(|t| *t);
        let global_ok = self.globalstate == 0 || global_state(s(self.globalstate)) == GlobalState::On;
        if cx.pev().spawnflags & SF_MULTI_INIT == 0 && all && global_ok {
            let kind = if self.globalstate != 0 { UseType::On } else { UseType::Toggle };
            self.delay.use_targets(cx.edict, std::ptr::null_mut(), kind, 0.0);
        }
    }
}
