//! `func_button`, `func_rot_button`, `momentary_rot_button`, `button_target`, `multisource` (SDK
//! buttons.cpp; identical in TS apart from TS's slow-motion pitch on spark and wheel sounds), and
//! the lock sounds and sentences doors share (`PlayLockSounds`, 0xb4f50).

use super::base::*;
use super::bmodels::SF_PENDULUM_AUTO_RETURN;
use super::explode::{emit_sound_slow, sparks};
use crate::world::{Cx, DamageInfo, Entity, UseType};
use goldsrc::edict::EdictPtr;
use goldsrc::ext::entities::{ATTN_NORM, EHandle, Vec3, vec};
use goldsrc::server::engine;
use goldsrc::sys::*;
use shared::pm::plus_double;
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

/// Seconds between lock sentences (`DOOR_SENTENCEWAIT`).
pub const DOOR_SENTENCEWAIT: f32 = 6.0;

/// The `locked_sentence` keyvalue's sentence group (`CBaseDoor::Precache`,
/// `CBaseButton::Precache`; the same table in both).
pub fn locked_sentence(n: i32) -> Option<&'static CStr> {
    Some(match n {
        1 => c"NA",    // access denied
        2 => c"ND",    // security lockout
        3 => c"NF",    // blast door
        4 => c"NFIRE", // fire door
        5 => c"NCHEM", // chemical door
        6 => c"NRAD",  // radiation door
        7 => c"NCON",  // gen containment
        8 => c"NH",    // maintenance door
        9 => c"NG",    // broken door
        _ => return None,
    })
}

/// The `unlocked_sentence` keyvalue's sentence group.
pub fn unlocked_sentence(n: i32) -> Option<&'static CStr> {
    Some(match n {
        1 => c"EA",    // access granted
        2 => c"ED",    // security door
        3 => c"EF",    // blast door
        4 => c"EFIRE", // fire door
        5 => c"ECHEM", // chemical door
        6 => c"ERAD",  // radiation door
        7 => c"ECON",  // gen containment
        8 => c"EH",    // maintenance area
        _ => return None,
    })
}

/// SDK `locksound_t`, plus the keyvalue numbers it's built from.
#[derive(Default)]
pub struct LockSounds {
    pub locked_n: i32,
    pub unlocked_n: i32,
    pub locked_sentence_n: i32,
    pub unlocked_sentence_n: i32,
    pub locked: Option<&'static CStr>,
    pub unlocked: Option<&'static CStr>,
    pub locked_sentence: Option<&'static CStr>,
    pub unlocked_sentence: Option<&'static CStr>,
    /// `iLockedSentence` / `iUnlockedSentence`: the next ordinal to play from the group.
    pub i_locked: i32,
    pub i_unlocked: i32,
    pub wait_sound: f32,
    pub wait_sentence: f32,
    /// `bEOFLocked` / `bEOFUnlocked`: the group has been played to its last sentence.
    pub eof_locked: bool,
    pub eof_unlocked: bool,
}

impl LockSounds {
    /// `m_bLockedSound`, `m_bLockedSentence`, `m_bUnlockedSound`, `m_bUnlockedSentence`: the four
    /// BYTE keyvalues, in the save tables' order (`CBaseDoor`, `CBaseButton`).
    pub fn save_bytes(&mut self, f: &mut crate::save::Fields) {
        for n in [&mut self.locked_n, &mut self.locked_sentence_n, &mut self.unlocked_n, &mut self.unlocked_sentence_n] {
            let mut b = *n as u8;
            f.byte(&mut b);
            *n = b as i32;
        }
    }

    pub fn key_value(&mut self, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"locked_sound" => self.locked_n = fistp16_low_byte(kv_d(value)) as i32,
            b"unlocked_sound" => self.unlocked_n = fistp16_low_byte(kv_d(value)) as i32,
            // All four are BYTE members set like `movesnd` (`CBaseDoor::KeyValue` 0xb51a0,
            // `CBaseButton::KeyValue` 0xa8184): 256 is 0, 270 is 14.
            b"locked_sentence" => self.locked_sentence_n = fistp16_low_byte(kv_d(value)) as i32,
            b"unlocked_sentence" => self.unlocked_sentence_n = fistp16_low_byte(kv_d(value)) as i32,
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
        self.locked_sentence = locked_sentence(self.locked_sentence_n);
        self.unlocked_sentence = unlocked_sentence(self.unlocked_sentence_n);
    }

    /// `PlayLockSounds(pev, pls, flocked, fbutton)` (0xb4f50): the lock sound every 3 s (0.5 s
    /// for buttons), and the group's sentences in order every 6 s until the last one has played.
    pub fn play(&mut self, e: *mut edict_t, locked: bool, button: bool) {
        let now = engine().time();
        let sound_wait = if button { BUTTON_SOUNDWAIT } else { DOOR_SOUNDWAIT };
        let (sound, sentence, eof) = if locked {
            (self.locked, self.locked_sentence, self.eof_locked)
        } else {
            (self.unlocked, self.unlocked_sentence, self.eof_unlocked)
        };
        let play_sound = sound.is_some() && now > self.wait_sound;
        let play_sentence = sentence.is_some() && !eof && now > self.wait_sentence;
        // Both at once: the sound is played quieter.
        let vol = if play_sound && play_sentence { 0.25 } else { 1.0 };
        if let (true, Some(snd)) = (play_sound, sound) {
            emit_sound_slow(e, CHAN_ITEM, snd, vol, ATTN_NORM, 0, PITCH_NORM as i32, 0.5);
            self.wait_sound = now + sound_wait;
        }
        if let (true, Some(group)) = (play_sentence, sentence) {
            let prev = if locked { self.i_locked } else { self.i_unlocked };
            let next = super::sentences::play_sequential(e, group, 0.85, ATTN_NORM, 0, PITCH_NORM as i32, prev, false);
            // The ordinal stops advancing once the last sentence has played.
            if locked {
                (self.i_locked, self.i_unlocked, self.eof_locked) = (next, 0, prev == next);
            } else {
                (self.i_unlocked, self.i_locked, self.eof_unlocked) = (next, 0, prev == next);
            }
            self.wait_sentence = now + DOOR_SENTENCEWAIT;
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
                emit_sound_slow(cx.edict, CHAN_VOICE, Self::noise(cx), 1.0, ATTN_NORM, 0, PITCH_NORM as i32, 0.5);
                self.button_return(cx);
            }
        } else {
            self.button_activate(cx);
        }
    }

    /// `CBaseButton::ButtonTouch` (0xa8c24): only an entity whose classname is `"player"`
    /// (any `CBasePlayer`, connected or not: `rules::has_player`).
    fn button_touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        if !crate::rules::has_player(other) {
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
            emit_sound_slow(cx.edict, CHAN_VOICE, Self::noise(cx), 1.0, ATTN_NORM, 0, PITCH_NORM as i32, 0.5);
            self.t.use_targets(cx.edict, other, UseType::Toggle, 0.0);
            self.button_return(cx);
        } else {
            self.button_activate(cx);
        }
    }

    fn button_activate(&mut self, cx: &mut Cx) {
        emit_sound_slow(cx.edict, CHAN_VOICE, Self::noise(cx), 1.0, ATTN_NORM, 0, PITCH_NORM as i32, 0.5);
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
                crate::ham::use_(e, act, me, UseType::Toggle, 0.0);
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
        if button_short(self.t.pos2, self.t.pos1) || v.spawnflags & SF_BUTTON_DONTMOVE != 0 {
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

    /// `CBaseButton::Save` / `Restore` (0xa7e30 / 0xa7e80), for `CRotButton` too. `m_pfnUse`
    /// (`ButtonUse`) is set unless the button is touch-only, so most restores stop after
    /// `"BASE"`.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut crate::save::Fields) {
        let cleared = self.t.save_chain(cx, f, &mut crate::save::Animating::default(), self.usable, false);
        if cleared.use_ {
            self.usable = false;
        }
        f.table(&crate::save::tables::CBaseButton);
        f.boolean(&mut self.stay_pushed);
        f.boolean(&mut self.rotating);
        f.int(&mut self.sounds);
        self.ls.save_bytes(f);
        f.string(&mut self.change_target);
    }

    /// `CBaseButton::Precache` (0xa7ed0), also `CRotButton`'s (whose `Spawn` doesn't call it).
    fn restored(&mut self, cx: &mut Cx) {
        self.precache(cx);
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
            emit_sound_slow(cx.edict, CHAN_VOICE, Self::noise(cx), 1.0, ATTN_NORM, 0, PITCH_NORM as i32, 0.5);
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

// ---- momentary_rot_button ---------------------------------------------------------------------------

/// `SF_MOMENTARY_DOOR`: solid, and not +usable (`CMomentaryRotButton::ObjectCaps` 0xaa9b4).
pub const SF_MOMENTARY_DOOR: i32 = 1;

/// `CBaseToggle::AxisDelta(flags, a, b) / m_flMoveDistance` as TS computes it (`Use` 0xa9850,
/// `Return` 0xa9cb0): the long-double difference divided by the float distance, rounded to float
/// once. A zero distance gives ±inf or NaN, as in TS.
/// `CBaseButton::Spawn`'s `(m_vecPosition2 - m_vecPosition1).Length() < 1` (0x1000bd8a..
/// 0x1000bdbb; issue #171): the unrounded differences summed `z, y, x`, the root unrounded; a NaN
/// counts as short.
fn button_short(pos2: Vec3, pos1: Vec3) -> bool {
    let d = [0, 1, 2].map(|i| f64::from(pos2[i]) - f64::from(pos1[i]));
    shared::pm::below87(((d[2] * d[2] + d[1] * d[1]) + d[0] * d[0]).sqrt(), 1.0)
}

fn axis_fraction(flags: i32, a: Vec3, b: Vec3, distance: f32) -> f32 {
    ((axis_value(flags, a) as f64 - axis_value(flags, b) as f64) / distance as f64) as f32
}

/// What `CMomentaryRotButton::UpdateSelf` (0xa99c0) does with the turned direction and the
/// position.
#[derive(Debug, PartialEq, Eq)]
enum RotStep {
    /// Turning forward and at (or past) the end: stop there.
    End,
    /// Turning back and at (or before) the start: stop there.
    Start,
    /// Keep turning.
    Turn,
}

/// The x87 compares (0xa9a1e, 0xa9a59): a NaN position keeps turning forward but stops at the
/// start when turning back.
fn rot_step(direction: i32, value: f32) -> RotStep {
    if direction > 0 && value >= 1.0 {
        RotStep::End
    } else if direction < 0 && (value <= 0.0 || value.is_nan()) {
        RotStep::Start
    } else {
        RotStep::Turn
    }
}

/// `momentary_rot_button` (`CMomentaryRotButton`): a wheel that turns while a player holds +use
/// and sends its position (0..1) as `USE_SET` to its targets, usually a `momentary_door`. The SDK
/// 2.3 class (no HL25 destination prediction in `Use`), with TS's slow-motion sound pitch.
#[derive(Default)]
pub struct MomentaryRotButton {
    t: Toggle<Self>,
    /// `m_lastUsed`.
    last_used: bool,
    /// `m_direction`: +1 or -1.
    direction: i32,
    /// `m_returnSpeed`.
    return_speed: f32,
    /// `m_start` / `m_end`.
    start: Vec3,
    end: Vec3,
    /// `m_sounds`.
    sounds: i32,
}

impl Mover for MomentaryRotButton {
    fn toggle(&mut self) -> &mut Toggle<Self> {
        &mut self.t
    }
}

impl MomentaryRotButton {
    fn fraction(&self, cx: &mut Cx) -> f32 {
        let v = cx.pev();
        axis_fraction(v.spawnflags, v.angles, self.start, self.t.move_distance)
    }

    /// `CMomentaryRotButton::PlaySound` (0xa97fc).
    fn play_sound(cx: &mut Cx) {
        let n = s(cx.pev().noise);
        emit_sound_slow(cx.edict, CHAN_VOICE, n, 1.0, ATTN_NORM, 0, PITCH_NORM as i32, 0.5);
    }

    /// `CMomentaryRotButton::UpdateAllButtons` (0xa98bc): every `momentary_rot_button` whose
    /// `target` is ours (this one included, in entity order) follows. There's no empty-target
    /// check, so the engine's string search decides what an empty `target` matches.
    fn update_all_buttons(&mut self, cx: &mut Cx, value: f32, start: bool) {
        let eng = engine();
        let me = cx.edict;
        let mut e = std::ptr::null_mut();
        loop {
            e = eng.find_by_string(e, c"target", s(cx.pev().target));
            if e.is_null() {
                break;
            }
            if s(e.v().classname) != c"momentary_rot_button" {
                continue;
            }
            if e == me {
                self.update_one(cx, value, start);
            } else {
                crate::world::dispatch(e, |ent, ecx| {
                    if let Some(b) = (ent as &mut dyn std::any::Any).downcast_mut::<Self>() {
                        b.update_one(ecx, value, start);
                    }
                });
            }
        }
    }

    fn update_one(&mut self, cx: &mut Cx, value: f32, start: bool) {
        if start { self.update_self(cx, value) } else { self.update_self_return(cx, value) }
    }

    /// `CMomentaryRotButton::UpdateSelf` (0xa99c0): the first use after a stop reverses the
    /// direction and plays the sound; each use keeps it turning for another 0.1 s.
    fn update_self(&mut self, cx: &mut Cx, value: f32) {
        let first = !self.last_used;
        if first {
            self.direction = -self.direction;
        }
        self.last_used = true;
        let v = cx.pev();
        v.nextthink = plus_double(v.ltime, 0.1);
        match rot_step(self.direction, value) {
            RotStep::End => {
                v.avelocity = [0.0; 3];
                v.angles = self.end;
                return;
            }
            RotStep::Start => {
                v.avelocity = [0.0; 3];
                v.angles = self.start;
                return;
            }
            RotStep::Turn => {}
        }
        if first {
            Self::play_sound(cx);
        }
        let v = cx.pev();
        // SDK "HACKHACK": several +use packets in one frame each add 0.1 s. Always the second
        // branch here, since nextthink was just set to ltime + 0.1.
        v.nextthink = if v.nextthink >= v.ltime { plus_double(v.nextthink, 0.1) } else { plus_double(v.ltime, 0.1) };
        v.avelocity = vec::scale(v.movedir, self.direction as f32 * v.speed);
        self.t.think = Some(Self::off);
    }

    /// `CMomentaryRotButton::UpdateTarget` (0xa9b3c): `USE_SET` with the position to every entity
    /// named by `target`, with this button as activator and caller.
    fn update_target(&mut self, cx: &mut Cx, value: f32) {
        if cx.pev().target == 0 {
            return;
        }
        let eng = engine();
        let me = cx.edict;
        let mut e = std::ptr::null_mut();
        loop {
            e = eng.find_by_string(e, c"targetname", s(cx.pev().target));
            if e.is_null() {
                break;
            }
            // A button that targets itself recursed until TS crashed; here the re-entrant
            // dispatch is skipped.
            crate::ham::use_(e, me, me, UseType::Set, value);
        }
    }

    /// `CMomentaryRotButton::Off` (0xa9c10): +use was let go. With `SF_PENDULUM_AUTO_RETURN` and a
    /// return speed (the x87 compare also takes a NaN one), it starts turning back.
    fn off(&mut self, cx: &mut Cx) {
        let v = cx.pev();
        v.avelocity = [0.0; 3];
        self.last_used = false;
        if v.spawnflags & SF_PENDULUM_AUTO_RETURN != 0 && (self.return_speed > 0.0 || self.return_speed.is_nan()) {
            self.t.think = Some(Self::return_);
            v.nextthink = plus_double(v.ltime, 0.1);
            self.direction = -1;
        } else {
            self.t.think = None;
        }
    }

    /// `CMomentaryRotButton::Return` (0xa9cb0).
    fn return_(&mut self, cx: &mut Cx) {
        let value = self.fraction(cx);
        self.update_all_buttons(cx, value, false);
        if value > 0.0 {
            self.update_target(cx, value);
        }
    }

    /// `CMomentaryRotButton::UpdateSelfReturn` (0xa9d24): a NaN position stops at the start.
    fn update_self_return(&mut self, cx: &mut Cx, value: f32) {
        let v = cx.pev();
        if value > 0.0 {
            v.avelocity = vec::scale(v.movedir, -self.return_speed);
            v.nextthink = plus_double(v.ltime, 0.1);
        } else {
            v.avelocity = [0.0; 3];
            v.angles = self.start;
            v.nextthink = -1.0;
            self.t.think = None;
        }
    }
}

impl Entity for MomentaryRotButton {
    /// `CMomentaryRotButton::KeyValue` (0xa9758).
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"returnspeed" => self.return_speed = kv_f(value),
            b"sounds" => self.sounds = kv_i(value),
            _ => return self.t.key_value(key, value),
        }
        true
    }

    /// `CMomentaryRotButton::Spawn` (0xa9550). A negative `distance` swaps the ends (the wheel
    /// starts at its end); a NaN one is taken as positive.
    fn spawn(&mut self, cx: &mut Cx) {
        let v = cx.pev();
        axis_dir(v);
        if v.speed == 0.0 {
            v.speed = 100.0;
        }
        let d = self.t.move_distance;
        if d < 0.0 {
            self.start = mad_x87(v.angles, v.movedir, d);
            self.end = v.angles;
            self.direction = 1;
            self.t.move_distance = -d;
        } else {
            self.start = v.angles;
            self.end = mad_x87(v.angles, v.movedir, d);
            self.direction = -1;
        }
        v.solid = if v.spawnflags & SF_MOMENTARY_DOOR != 0 { SOLID_BSP } else { SOLID_NOT } as i32;
        v.movetype = MOVETYPE_PUSH as i32;
        let org = v.origin;
        engine().set_origin(cx.edict, org);
        set_own_model(cx);
        let snd = button_sound(self.sounds);
        engine().precache_sound(snd);
        cx.pev().noise = alloc(snd);
        self.last_used = false;
    }

    fn think(&mut self, cx: &mut Cx) {
        run_think(self, cx);
    }

    /// `CMomentaryRotButton::Save` / `Restore` (0xa948c / 0xa94dc): `Use` is a virtual, the
    /// think is `Off` / `Return`. Its `Precache` is `CBaseEntity`'s.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut crate::save::Fields) {
        self.t.save_chain(cx, f, &mut crate::save::Animating::default(), false, false);
        f.table(&crate::save::tables::CMomentaryRotButton);
        f.boolean(&mut self.last_used);
        f.int(&mut self.direction);
        f.float(&mut self.return_speed);
        f.vector(&mut self.start);
        f.vector(&mut self.end);
        f.int(&mut self.sounds);
    }

    /// `CMomentaryRotButton::Use` (0xa9850): any use type. The targets get the position from
    /// before this use's turn (the SDK's "BUGBUG" latency).
    fn use_(&mut self, cx: &mut Cx, _activator: *mut edict_t, _caller: *mut edict_t, _kind: UseType, _value: f32) {
        let value = self.fraction(cx);
        cx.pev().ideal_yaw = value;
        self.update_all_buttons(cx, value, true);
        let value = cx.pev().ideal_yaw;
        self.update_target(cx, value);
    }
}

// ---- button_target ----------------------------------------------------------------------------------

pub const SF_BTARGET_USE: i32 = 1;
pub const SF_BTARGET_ON: i32 = 2;

/// `button_target` (`CButtonTarget`, 0xaa254..0xaa394): a `CBaseEntity`, so it has no
/// `delay`/`killtarget` keys and its targets fire through `CBaseEntity::SUB_UseTargets`
/// (0xe2efc), a plain `FireTargets`.
#[derive(Default)]
pub struct ButtonTarget;

impl Entity for ButtonTarget {
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
        if !should_toggle(kind, super::maprules::x87_int(v.frame) != 0) {
            return;
        }
        v.frame = 1.0 - v.frame;
        let k = if v.frame != 0.0 { UseType::On } else { UseType::Off };
        let target = v.target;
        if target != 0 {
            crate::entities::fire_targets(s(target), activator, cx.edict, k, 0.0);
        }
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
    entities: Vec<EHandle>,
    triggered: Vec<bool>,
    globalstate: string_t,
    registered: bool,
    /// TS's `m_pfnThink`: `Register` from `Spawn` (0xa7a28), then `SUB_DoNothing`; a restore
    /// clears it, so a restored multisource never registers.
    think_set: bool,
}

/// The input `CMultiSource::Use` (0xa7a90) toggles: the first entry whose handle is `caller`
/// (a NULL caller matches a member that's gone), else the last one.
fn member_index(entities: &[EHandle], caller: *mut edict_t) -> Option<usize> {
    entities.iter().position(|h| h.get() == caller).or(entities.len().checked_sub(1))
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
        v.nextthink = plus_double(engine().time(), 0.1);
        v.spawnflags |= SF_MULTI_INIT;
        self.think_set = true;
        self.publish(cx);
    }

    fn think(&mut self, cx: &mut Cx) {
        if self.think_set && !self.registered {
            self.register(cx);
        }
    }

    /// `CMultiSource::Save` / `Restore` (0xa785c / 0xa78ac). The think is always set after
    /// `Spawn`, so a restore stops after `"BASE"`: no members, which `IsTriggered` (0xa7b84)
    /// takes as all on.
    ///
    /// Guard: TS loops over `m_iTotal` entries of its 32-entry arrays; the port caps a restored
    /// count at 32 (and 0).
    fn save_fields(&mut self, cx: &mut Cx, f: &mut crate::save::Fields) {
        let cleared = f.base_funcs(cx, crate::save::BaseFuncs { think: self.think_set, ..Default::default() });
        if cleared.think {
            self.think_set = false;
        }
        f.table(&crate::save::tables::CMultiSource);
        let mut ents = [EHandle::NULL; MS_MAX_TARGETS];
        let mut trig = [0i32; MS_MAX_TARGETS];
        for (i, h) in self.entities.iter().take(MS_MAX_TARGETS).enumerate() {
            ents[i] = *h;
            trig[i] = self.triggered.get(i).copied().unwrap_or(false) as i32;
        }
        let mut total = self.entities.len() as i32;
        f.ehandles(&mut ents);
        f.ints(&mut trig);
        f.int(&mut total);
        f.string(&mut self.globalstate);
        let n = total.clamp(0, MS_MAX_TARGETS as i32) as usize;
        self.entities = ents[..n].to_vec();
        self.triggered = trig[..n].iter().map(|t| *t != 0).collect();
    }

    /// `CMultiSource` has `CBaseEntity::Precache`; the port's master table is rebuilt from the
    /// restored state.
    fn restored(&mut self, cx: &mut Cx) {
        self.publish(cx);
    }

    /// `CMultiSource::Use` (0xa7a90). TS keeps the SDK's search bug: its "Used by non member"
    /// test (`i > m_iTotal`) can't be true, so a caller that isn't a member (a `DelayedUse`, say)
    /// toggles the last input. With no members TS toggles `m_rgTriggered[-1]`, which lies in
    /// `m_rgEntities[31]` (nothing reads it); the port toggles nothing.
    fn use_(&mut self, cx: &mut Cx, _activator: *mut edict_t, caller: *mut edict_t, _kind: UseType, _value: f32) {
        if let Some(i) = member_index(&self.entities, caller) {
            self.triggered[i] ^= true;
        }
        self.publish(cx);
        let all = self.triggered.iter().all(|t| *t);
        let global_ok = self.globalstate == 0 || global_state(s(self.globalstate)) == GlobalState::On;
        if cx.pev().spawnflags & SF_MULTI_INIT == 0 && all && global_ok {
            let msg = format!("Multisource {} enabled ({} inputs)\n", s(cx.pev().targetname).to_string_lossy(), self.entities.len());
            engine().alert(ALERT_TYPE::at_aiconsole, &msg);
            let kind = if self.globalstate != 0 { UseType::On } else { UseType::Toggle };
            // `CBaseEntity::SUB_UseTargets` (0xe2efc): no activator.
            let target = cx.pev().target;
            if target != 0 {
                crate::entities::fire_targets(s(target), std::ptr::null_mut(), cx.edict, kind, 0.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multisource_non_member_toggles_last_input() {
        let fake = std::ptr::dangling_mut::<edict_t>();
        let none: [EHandle; 0] = [];
        assert_eq!(member_index(&none, fake), None);
        // Members that are gone read as NULL: a NULL caller matches the first of them.
        let gone = [EHandle::NULL, EHandle::NULL, EHandle::NULL];
        assert_eq!(member_index(&gone, std::ptr::null_mut()), Some(0));
        // A caller that isn't a member (`fake` is never dereferenced: the handles are NULL).
        assert_eq!(member_index(&gone, fake), Some(2));
    }

    #[test]
    fn lock_keys_are_bytes() {
        let mut l = LockSounds::default();
        for (k, v) in
            [(c"locked_sound", c"256"), (c"unlocked_sound", c"270.9"), (c"locked_sentence", c"-1"), (c"unlocked_sentence", c"70000")]
        {
            assert!(l.key_value(k, v));
        }
        // 256 wraps to 0 (no sound precached), 270.9 truncates to 270 = 14.
        assert_eq!((l.locked_n, l.unlocked_n), (0, 14));
        // -1 is 0xff; past the short's range the x87 store gives 0x8000, low byte 0.
        assert_eq!((l.locked_sentence_n, l.unlocked_sentence_n), (255, 0));
        assert!(!l.key_value(c"sounds", c"1"));
    }

    /// `CBaseButton::Spawn`'s `< 1` travel test (issue #171): the unrounded differences, so
    /// `1 - 2⁻²⁶` of travel (a `float` difference rounds it to 1) stays short; a NaN is short too.
    #[test]
    fn button_travel_test() {
        assert!(button_short([1.0, 0.0, 0.0], [2f32.powi(-26), 0.0, 0.0]));
        assert_eq!(1.0f32 - 2f32.powi(-26), 1.0);
        assert!(!button_short([1.0, 0.0, 0.0], [0.0; 3]));
        assert!(button_short([f32::NAN, 0.0, 0.0], [0.0; 3]));
    }

    #[test]
    fn momentary_fraction() {
        assert_eq!(axis_fraction(0, [0.0, 45.0, 0.0], [0.0; 3], 90.0), 0.5);
        assert_eq!(axis_fraction(SF_DOOR_ROTATE_Z, [0.0, 45.0, 30.0], [0.0, 0.0, 0.0], 60.0), 0.5);
        assert!(axis_fraction(0, [0.0; 3], [0.0; 3], 0.0).is_nan());
        assert_eq!(axis_fraction(0, [0.0, 1.0, 0.0], [0.0; 3], 0.0), f32::INFINITY);
        assert_eq!(plus_double(1.0, 0.1), 1.1);
    }

    #[test]
    fn momentary_steps() {
        assert_eq!(rot_step(1, 1.0), RotStep::End);
        assert_eq!(rot_step(1, 0.5), RotStep::Turn);
        assert_eq!(rot_step(1, f32::NAN), RotStep::Turn);
        assert_eq!(rot_step(-1, 0.0), RotStep::Start);
        assert_eq!(rot_step(-1, -0.1), RotStep::Start);
        assert_eq!(rot_step(-1, f32::NAN), RotStep::Start);
        assert_eq!(rot_step(-1, 1.5), RotStep::Turn);
        assert_eq!(rot_step(1, -1.0), RotStep::Turn);
    }
}
