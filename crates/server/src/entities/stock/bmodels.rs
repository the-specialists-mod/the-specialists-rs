//! Simple brush entities (SDK bmodels.cpp; same in TS): `func_wall`, `func_illusionary`,
//! `func_wall_toggle`, `func_conveyor`, `func_rotating`, `func_monsterclip`.

use super::base::*;
use super::explode::emit_sound_slow;
use crate::damage::bits as dmg;
use crate::world::{Cx, DamageInfo, Entity, UseType};
use goldsrc::edict::EdictPtr;
use goldsrc::ext::entities::{ATTN_IDLE, ATTN_NORM, ATTN_STATIC, snd, vec};
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::ffi::CStr;

/// `CFuncWall::Spawn`.
fn wall_spawn(cx: &mut Cx) {
    let v = cx.pev();
    v.angles = [0.0; 3];
    v.movetype = MOVETYPE_PUSH as i32;
    v.solid = SOLID_BSP as i32;
    set_own_model(cx);
    cx.pev().flags |= FL_WORLDBRUSH as i32;
}

/// `CFuncWall::Use` (0xa5130): toggles the texture frame; `ShouldToggle` gets `(int)frame`.
fn wall_use(cx: &mut Cx, kind: UseType) {
    let v = cx.pev();
    if should_toggle(kind, super::maprules::x87_int(v.frame) != 0) {
        v.frame = 1.0 - v.frame;
    }
}

/// `func_wall`.
#[derive(Default)]
pub struct FuncWall;

impl Entity for FuncWall {
    fn spawn(&mut self, cx: &mut Cx) {
        wall_spawn(cx);
    }
    fn use_(&mut self, cx: &mut Cx, _a: *mut edict_t, _c: *mut edict_t, kind: UseType, _v: f32) {
        wall_use(cx, kind);
    }
}

/// `func_monsterclip` (`CFuncMonsterClip`): a `func_wall` (`CFuncWall::Spawn`) that only blocks
/// monsters (`FL_MONSTERCLIP`); Spawn (0xa55d0) replaces `effects` with `EF_NODRAW` unless
/// `showtriggers`, and Use (0xa74ec) does nothing.
#[derive(Default)]
pub struct FuncMonsterClip;

impl Entity for FuncMonsterClip {
    fn spawn(&mut self, cx: &mut Cx) {
        wall_spawn(cx);
        if engine().cvar_float(c"showtriggers") == 0.0 {
            cx.pev().effects = EF_NODRAW as i32;
        }
        cx.pev().flags |= FL_MONSTERCLIP as i32;
    }
}

/// `func_illusionary`: non-solid; `skin` can give it contents (e.g. water). `CFuncIllusionary::KeyValue`
/// (0xa54b0) also parses `skin`, but `DispatchKeyValue` sets the entvars field first, so that branch
/// never runs; every other key goes to `CBaseToggle::KeyValue`.
#[derive(Default)]
pub struct FuncIllusionary {
    /// TS's class is a `CBaseToggle`: nothing uses its state, but the keyvalues land there and
    /// are saved.
    t: Toggle<Self>,
}

impl Entity for FuncIllusionary {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        self.t.key_value(key, value)
    }

    /// `CFuncIllusionary` has no `Save` of its own: `CBaseToggle::Save` (no member pointer is
    /// ever set). Its `Precache` is `CBaseEntity`'s.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut crate::save::Fields) {
        self.t.save_chain(cx, f, &mut crate::save::Animating::default(), false, false);
    }

    fn spawn(&mut self, cx: &mut Cx) {
        let v = cx.pev();
        v.angles = [0.0; 3];
        v.movetype = MOVETYPE_NONE as i32;
        v.solid = SOLID_NOT as i32;
        set_own_model(cx);
    }
}

pub const SF_WALL_START_OFF: i32 = 1;

/// `func_wall_toggle`.
#[derive(Default)]
pub struct FuncWallToggle;

impl FuncWallToggle {
    fn set(cx: &mut Cx, on: bool) {
        let v = cx.pev();
        if on {
            v.solid = SOLID_BSP as i32;
            v.effects &= !(EF_NODRAW as i32);
        } else {
            v.solid = SOLID_NOT as i32;
            v.effects |= EF_NODRAW as i32;
        }
        let org = v.origin;
        engine().set_origin(cx.edict, org);
    }
}

impl Entity for FuncWallToggle {
    fn spawn(&mut self, cx: &mut Cx) {
        wall_spawn(cx);
        if cx.pev().spawnflags & SF_WALL_START_OFF != 0 {
            Self::set(cx, false);
        }
    }
    fn use_(&mut self, cx: &mut Cx, _a: *mut edict_t, _c: *mut edict_t, kind: UseType, _v: f32) {
        let on = cx.pev().solid != SOLID_NOT as i32;
        if should_toggle(kind, on) {
            Self::set(cx, !on);
        }
    }
}

pub const SF_CONVEYOR_VISUAL: i32 = 1;
pub const SF_CONVEYOR_NOTSOLID: i32 = 2;

/// `func_conveyor`: the speed travels to the client in `rendercolor` for the texture scroll.
#[derive(Default)]
pub struct FuncConveyor;

impl FuncConveyor {
    fn update_speed(cx: &mut Cx, speed: f32) {
        let code = (speed.abs() * 16.0) as i32;
        let v = cx.pev();
        v.rendercolor[0] = if speed < 0.0 { 1.0 } else { 0.0 };
        v.rendercolor[1] = (code >> 8) as f32;
        v.rendercolor[2] = (code & 0xff) as f32;
    }
}

impl Entity for FuncConveyor {
    fn spawn(&mut self, cx: &mut Cx) {
        set_movedir(cx.pev());
        wall_spawn(cx);
        let v = cx.pev();
        if v.spawnflags & SF_CONVEYOR_VISUAL == 0 {
            v.flags |= FL_CONVEYOR as i32;
        }
        if v.spawnflags & SF_CONVEYOR_NOTSOLID != 0 {
            v.solid = SOLID_NOT as i32;
            v.skin = 0;
        }
        if v.speed == 0.0 {
            v.speed = 100.0;
        }
        let sp = v.speed;
        Self::update_speed(cx, sp);
    }
    fn use_(&mut self, cx: &mut Cx, _a: *mut edict_t, _c: *mut edict_t, _kind: UseType, _v: f32) {
        let v = cx.pev();
        v.speed = -v.speed;
        let sp = v.speed;
        Self::update_speed(cx, sp);
    }
}

// ---- func_rotating ------------------------------------------------------------------------------------

pub const SF_BRUSH_ROTATE_INSTANT: i32 = 1;
pub const SF_BRUSH_ROTATE_BACKWARDS: i32 = 2;
pub const SF_BRUSH_ROTATE_Z_AXIS: i32 = 4;
pub const SF_BRUSH_ROTATE_X_AXIS: i32 = 8;
pub const SF_BRUSH_ACCDCC: i32 = 16;
pub const SF_BRUSH_HURT: i32 = 32;
pub const SF_ROTATING_NOT_SOLID: i32 = 64;
pub const SF_BRUSH_ROTATE_SMALLRADIUS: i32 = 128;
pub const SF_BRUSH_ROTATE_MEDIUMRADIUS: i32 = 256;
pub const SF_BRUSH_ROTATE_LARGERADIUS: i32 = 512;
const FANPITCHMIN: f32 = 30.0;
const FANPITCHMAX: f32 = 100.0;

#[derive(Default)]
pub struct FuncRotating {
    fan_friction: f32,
    attenuation: f32,
    volume: f32,
    pitch: f32,
    sounds: i32,
    think: ThinkFn<Self>,
    /// `m_pfnTouch` = `HurtTouch` (`SF_BRUSH_HURT`).
    hurt: bool,
    /// `m_pfnUse` = `RotatingUse`: set by `Spawn` (0xa5874), cleared by a restore.
    usable: bool,
}

/// C's `abs((int)x)` as TS compiles it: an x87 truncating `fistp`, then the wrapping `abs`.
fn int_abs(x: f32) -> i32 {
    super::maprules::x87_int(x).wrapping_abs()
}

fn first_nonzero(a: [f32; 3]) -> f32 {
    if a[0] != 0.0 {
        a[0]
    } else if a[1] != 0.0 {
        a[1]
    } else {
        a[2]
    }
}

/// The fan sounds are TS's `EMIT_SOUND_DYN(..., 0.5)`, so their pitch drops with the fan's own
/// time scale (`fuser1`): `RampPitchVol` 0xa5e68, `SpinUp` 0xa5fd8, `SpinDown` 0xa6254 and both
/// calls in `RotatingUse` 0xa6474 push 0.5.
impl FuncRotating {
    fn noise(cx: &mut Cx) -> &'static CStr {
        s(cx.pev().noise3)
    }

    /// `CFuncRotating::RampPitchVol` (0xa5e68): the speeds go through the integer `abs`
    /// (`abs((int)x)`), as TS's SDK wrote `abs` of a float.
    fn ramp_pitch_vol(&mut self, cx: &mut Cx) {
        let v = cx.pev();
        let cur = int_abs(first_nonzero(v.avelocity));
        let fin = int_abs(first_nonzero(v.movedir) * v.speed);
        // Guard: TS divides by 0 when the speed truncates to 0 and sends a NaN volume and pitch.
        let pct = if fin != 0 { cur as f32 / fin as f32 } else { 0.0 };
        let vol = self.volume * pct;
        let mut pitch = (FANPITCHMIN + (FANPITCHMAX - FANPITCHMIN) * pct) as i32;
        if pitch == PITCH_NORM as i32 {
            pitch = PITCH_NORM as i32 - 1;
        }
        emit_sound_slow(cx.edict, CHAN_STATIC, Self::noise(cx), vol, self.attenuation, snd::CHANGE_PITCH | snd::CHANGE_VOL, pitch, 0.5);
    }

    fn spin_up(&mut self, cx: &mut Cx) {
        think_ltime(cx, 0.1);
        let v = cx.pev();
        v.avelocity = vec::mad(v.avelocity, v.movedir, v.speed * self.fan_friction);
        let (av, md, sp) = (v.avelocity, v.movedir, v.speed);
        // `SpinUp` (0xa5fd8): integer `abs` of each truncated component, like `RampPitchVol`.
        if (0..3).all(|i| int_abs(av[i]) >= int_abs(md[i] * sp)) {
            v.avelocity = vec::scale(md, sp);
            emit_sound_slow(
                cx.edict,
                CHAN_STATIC,
                Self::noise(cx),
                self.volume,
                self.attenuation,
                snd::CHANGE_PITCH | snd::CHANGE_VOL,
                FANPITCHMAX as i32,
                0.5,
            );
            self.think = Some(Self::rotate);
            self.rotate(cx);
        } else {
            self.ramp_pitch_vol(cx);
        }
    }

    fn spin_down(&mut self, cx: &mut Cx) {
        think_ltime(cx, 0.1);
        let v = cx.pev();
        v.avelocity = vec::mad(v.avelocity, v.movedir, -(v.speed * self.fan_friction));
        let av = v.avelocity;
        let dir = first_nonzero(v.movedir);
        if (dir > 0.0 && av.iter().all(|a| *a <= 0.0)) || (dir < 0.0 && av.iter().all(|a| *a >= 0.0)) {
            v.avelocity = [0.0; 3];
            emit_sound_slow(cx.edict, CHAN_STATIC, Self::noise(cx), 0.0, 0.0, snd::STOP, self.pitch as i32, 0.5);
            self.think = Some(Self::rotate);
            self.rotate(cx);
        } else {
            self.ramp_pitch_vol(cx);
        }
    }

    fn rotate(&mut self, cx: &mut Cx) {
        think_ltime(cx, 10.0);
    }

    /// `SUB_CallUseToggle`.
    fn call_use_toggle(&mut self, cx: &mut Cx) {
        self.rotating_use(cx);
    }

    fn rotating_use(&mut self, cx: &mut Cx) {
        let v = cx.pev();
        let moving = v.avelocity != [0.0; 3];
        if v.spawnflags & SF_BRUSH_ACCDCC != 0 {
            if moving {
                self.think = Some(Self::spin_down);
                think_ltime(cx, 0.1);
            } else {
                self.think = Some(Self::spin_up);
                emit_sound_slow(cx.edict, CHAN_STATIC, Self::noise(cx), 0.01, self.attenuation, 0, FANPITCHMIN as i32, 0.5);
                think_ltime(cx, 0.1);
            }
        } else if moving {
            self.think = Some(Self::spin_down);
            think_ltime(cx, 0.1);
        } else {
            emit_sound_slow(cx.edict, CHAN_STATIC, Self::noise(cx), self.volume, self.attenuation, 0, FANPITCHMAX as i32, 0.5);
            let v = cx.pev();
            v.avelocity = vec::scale(v.movedir, v.speed);
            self.think = Some(Self::rotate);
            self.rotate(cx);
        }
    }

    fn precache(&mut self, cx: &mut Cx) {
        let eng = engine();
        let msg = cx.pev().message;
        let noise: &'static CStr = if msg != 0 && !s(msg).is_empty() {
            let n = goldsrc::server::intern(kv_str(s(msg)));
            eng.precache_sound(n);
            n
        } else {
            let n = match self.sounds {
                1 => c"fans/fan1.wav",
                2 => c"fans/fan2.wav",
                3 => c"fans/fan3.wav",
                4 => c"fans/fan4.wav",
                5 => c"fans/fan5.wav",
                _ => c"common/null.wav",
            };
            if n != c"common/null.wav" {
                eng.precache_sound(n);
            }
            n
        };
        cx.pev().noise3 = alloc(noise);
        if cx.pev().avelocity != [0.0; 3] {
            self.think = Some(Self::spin_up);
            think_ltime(cx, 1.5);
        }
    }
}

impl Entity for FuncRotating {
    fn key_value(&mut self, cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"fanfriction" => self.fan_friction = kv_f(value) / 100.0,
            b"Volume" => self.volume = (kv_f(value) / 10.0).clamp(0.0, 1.0),
            b"spawnorigin" => {
                let t = kv_vec(value);
                if t != [0.0; 3] {
                    cx.pev().origin = t;
                }
                return false; // the SDK leaves it unhandled
            }
            b"sounds" => self.sounds = kv_i(value),
            _ => return false,
        }
        true
    }

    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        self.pitch = PITCH_NORM as f32 - 1.0;
        if self.volume == 0.0 {
            self.volume = 1.0;
        }
        let v = cx.pev();
        let sf = v.spawnflags;
        self.attenuation = if sf & SF_BRUSH_ROTATE_SMALLRADIUS != 0 {
            ATTN_IDLE
        } else if sf & SF_BRUSH_ROTATE_MEDIUMRADIUS != 0 {
            ATTN_STATIC
        } else {
            ATTN_NORM
        };
        if self.fan_friction == 0.0 {
            self.fan_friction = 1.0;
        }
        v.movedir = if sf & SF_BRUSH_ROTATE_Z_AXIS != 0 {
            [0.0, 0.0, 1.0]
        } else if sf & SF_BRUSH_ROTATE_X_AXIS != 0 {
            [1.0, 0.0, 0.0]
        } else {
            [0.0, 1.0, 0.0]
        };
        if sf & SF_BRUSH_ROTATE_BACKWARDS != 0 {
            v.movedir = vec::scale(v.movedir, -1.0);
        }
        if sf & SF_ROTATING_NOT_SOLID != 0 {
            v.solid = SOLID_NOT as i32;
            v.skin = CONTENTS_EMPTY;
        } else {
            v.solid = SOLID_BSP as i32;
        }
        v.movetype = MOVETYPE_PUSH as i32;
        let org = v.origin;
        eng.set_origin(cx.edict, org);
        set_own_model(cx);
        let v = cx.pev();
        if v.speed <= 0.0 {
            v.speed = 0.0;
        }
        if sf & SF_BRUSH_ROTATE_INSTANT != 0 {
            self.think = Some(Self::call_use_toggle);
            think_ltime(cx, 1.5);
        }
        self.usable = true;
        self.hurt = sf & SF_BRUSH_HURT != 0;
        self.precache(cx);
    }

    fn think(&mut self, cx: &mut Cx) {
        if let Some(f) = self.think {
            f(self, cx);
        }
    }

    fn touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        if !self.hurt {
            return;
        }
        let ov = other.v();
        if ov.takedamage == 0.0 {
            return;
        }
        let me = cx.edict;
        let v = cx.pev();
        v.dmg = vec::length(v.avelocity) / 10.0;
        let (d, center) = (v.dmg, bmodel_origin(v));
        crate::damage::apply(other, &DamageInfo::new(me, me, d, dmg::CRUSH));
        let ov = other.v();
        ov.velocity = vec::scale(vec::normalize(vec::sub(ov.origin, center)), d);
    }

    fn use_(&mut self, cx: &mut Cx, _a: *mut edict_t, _c: *mut edict_t, _kind: UseType, _v: f32) {
        if self.usable {
            self.rotating_use(cx);
        }
    }

    /// `CFuncRotating::Save` / `Restore` (0xa5634 / 0xa5684). `Use` goes through `m_pfnUse`,
    /// which `Spawn` always sets, so a restore stops after `"BASE"`; `Blocked` is a virtual.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut crate::save::Fields) {
        let funcs = crate::save::BaseFuncs { think: self.think.is_some(), touch: self.hurt, use_: self.usable, blocked: false };
        let cleared = f.base_funcs(cx, funcs);
        if cleared.think {
            self.think = None;
        }
        if cleared.touch {
            self.hurt = false;
        }
        if cleared.use_ {
            self.usable = false;
        }
        f.table(&crate::save::tables::CFuncRotating);
        f.float(&mut self.fan_friction);
        f.float(&mut self.attenuation);
        f.float(&mut self.volume);
        f.float(&mut self.pitch);
        f.int(&mut self.sounds);
    }

    /// `CFuncRotating::Precache` (0xa5aec): the sound, and `SpinUp` in 1.5 s if it was turning.
    fn restored(&mut self, cx: &mut Cx) {
        self.precache(cx);
    }

    fn blocked(&mut self, cx: &mut Cx, other: *mut edict_t) {
        let me = cx.edict;
        let d = cx.pev().dmg;
        crate::damage::apply(other, &DamageInfo::new(me, me, d, dmg::CRUSH));
    }
}

// ---- func_pendulum ---------------------------------------------------------------------------------------

pub const SF_PENDULUM_AUTO_RETURN: i32 = 16;
const SF_DOOR_PASSABLE: i32 = 8;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum PendulumThink {
    #[default]
    None,
    UseToggle,
    Swing,
    Stop,
}

/// `func_pendulum` (CPendulum).
#[derive(Default)]
pub struct Pendulum {
    distance: f32,
    damp: f32,
    accel: f32,
    max_speed: f32,
    damp_speed: f32,
    time: f32,
    start: [f32; 3],
    center: [f32; 3],
    think: PendulumThink,
    /// `m_pfnUse` = `PendulumUse` (set by `Spawn` when it has a distance).
    usable: bool,
    /// `m_pfnTouch` = `RopeTouch` (spawnflag 2): set but never called, `Touch` is a virtual.
    rope: bool,
}

/// `CPendulum::Swing` (0xa6b90)'s clamps: `speed > max` gives `max`, else `speed < -max` gives
/// `-max`. Plain compares, so a negative `speed` key (a negative `max`) or a NaN goes through as
/// in TS, where `f32::clamp` would panic.
fn clamp_swing(speed: f32, max: f32) -> f32 {
    if speed > max {
        max
    } else if speed < -max {
        -max
    } else {
        speed
    }
}

impl Pendulum {
    fn pendulum_use(&mut self, cx: &mut Cx) {
        let now = engine().time();
        let v = cx.pev();
        if v.speed != 0.0 {
            if v.spawnflags & SF_PENDULUM_AUTO_RETURN != 0 {
                let delta = axis_delta(v.spawnflags, v.angles, self.start);
                v.avelocity = vec::scale(v.movedir, self.max_speed);
                v.nextthink = v.ltime + delta / self.max_speed;
                self.think = PendulumThink::Stop;
            } else {
                v.speed = 0.0;
                self.think = PendulumThink::None;
                v.avelocity = [0.0; 3];
            }
        } else {
            v.nextthink = v.ltime + 0.1;
            self.time = now;
            self.think = PendulumThink::Swing;
            self.damp_speed = self.max_speed;
        }
    }

    fn swing(&mut self, cx: &mut Cx) {
        let now = engine().time();
        let v = cx.pev();
        let delta = axis_delta(v.spawnflags, v.angles, self.center);
        let dt = now - self.time;
        self.time = now;
        if delta > 0.0 && self.accel > 0.0 {
            v.speed -= self.accel * dt;
        } else {
            v.speed += self.accel * dt;
        }
        v.speed = clamp_swing(v.speed, self.max_speed);
        v.avelocity = vec::scale(v.movedir, v.speed);
        v.nextthink = v.ltime + 0.1;
        if self.damp != 0.0 {
            self.damp_speed -= self.damp * self.damp_speed * dt;
            if self.damp_speed < 30.0 {
                v.angles = self.center;
                v.speed = 0.0;
                self.think = PendulumThink::None;
                v.avelocity = [0.0; 3];
            } else {
                v.speed = clamp_swing(v.speed, self.damp_speed);
            }
        }
    }
}

impl Entity for Pendulum {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"distance" => self.distance = kv_f(value),
            b"damp" => self.damp = kv_f(value) * 0.001,
            _ => return false,
        }
        true
    }

    fn spawn(&mut self, cx: &mut Cx) {
        let v = cx.pev();
        axis_dir(v);
        v.solid = if v.spawnflags & SF_DOOR_PASSABLE != 0 { SOLID_NOT } else { SOLID_BSP } as i32;
        v.movetype = MOVETYPE_PUSH as i32;
        let org = v.origin;
        engine().set_origin(cx.edict, org);
        set_own_model(cx);
        if self.distance == 0.0 {
            return;
        }
        let v = cx.pev();
        if v.speed == 0.0 {
            v.speed = 100.0;
        }
        self.accel = v.speed * v.speed / (2.0 * self.distance.abs());
        self.max_speed = v.speed;
        self.start = v.angles;
        self.center = vec::mad(v.angles, v.movedir, self.distance * 0.5);
        if v.spawnflags & SF_BRUSH_ROTATE_INSTANT != 0 {
            self.think = PendulumThink::UseToggle;
            // `CPendulum::Spawn` (0xa67f0) uses the global time, not `ltime`, although the
            // engine compares a pusher's `nextthink` against `ltime`: the first swing waits that
            // much longer.
            v.nextthink = engine().time() + 0.1;
        }
        v.speed = 0.0;
        self.usable = true;
        self.rope = v.spawnflags & 2 != 0;
    }

    fn think(&mut self, cx: &mut Cx) {
        match self.think {
            PendulumThink::UseToggle => {
                self.think = PendulumThink::None;
                self.pendulum_use(cx);
            }
            PendulumThink::Swing => self.swing(cx),
            PendulumThink::Stop => {
                let v = cx.pev();
                v.angles = self.start;
                v.speed = 0.0;
                v.avelocity = [0.0; 3];
                self.think = PendulumThink::None;
            }
            PendulumThink::None => {}
        }
    }

    fn use_(&mut self, cx: &mut Cx, _a: *mut edict_t, _c: *mut edict_t, _kind: UseType, _v: f32) {
        if self.usable {
            self.pendulum_use(cx);
        }
    }

    fn blocked(&mut self, _cx: &mut Cx, _other: *mut edict_t) {
        self.time = engine().time();
    }

    /// `CPendulum::Save` / `Restore` (0xa66ac / 0xa66fc). `Spawn` sets `m_pfnUse` whenever it
    /// has a distance, so a restore of a working pendulum stops after `"BASE"`. Its `Precache` is
    /// `CBaseEntity`'s.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut crate::save::Fields) {
        let funcs =
            crate::save::BaseFuncs { think: self.think != PendulumThink::None, touch: self.rope, use_: self.usable, blocked: false };
        let cleared = f.base_funcs(cx, funcs);
        if cleared.think {
            self.think = PendulumThink::None;
        }
        if cleared.touch {
            self.rope = false;
        }
        if cleared.use_ {
            self.usable = false;
        }
        f.table(&crate::save::tables::CPendulum);
        f.float(&mut self.accel);
        f.float(&mut self.distance);
        f.float(&mut self.time);
        f.float(&mut self.damp);
        f.float(&mut self.max_speed);
        f.float(&mut self.damp_speed);
        f.vector(&mut self.center);
        f.vector(&mut self.start);
    }

    fn touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        let me = cx.edict;
        let v = cx.pev();
        if v.dmg <= 0.0 || other.v().takedamage == 0.0 {
            return;
        }
        let damage = (v.dmg * v.speed * 0.01).abs();
        let center = bmodel_origin(v);
        crate::damage::apply(other, &DamageInfo::new(me, me, damage, dmg::CRUSH));
        let ov = other.v();
        ov.velocity = vec::scale(vec::normalize(vec::sub(ov.origin, center)), damage);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pendulum_clamp_is_plain_compares() {
        assert_eq!(clamp_swing(150.0, 100.0), 100.0);
        assert_eq!(clamp_swing(-150.0, 100.0), -100.0);
        assert_eq!(clamp_swing(50.0, 100.0), 50.0);
        // A negative `speed` key: TS's compares, no panic.
        assert_eq!(clamp_swing(0.0, -100.0), -100.0);
        assert_eq!(clamp_swing(-150.0, -100.0), 100.0);
        assert!(clamp_swing(f32::NAN, 100.0).is_nan());
        assert!(clamp_swing(50.0, f32::NAN) == 50.0);
    }

    #[test]
    fn rotating_speeds_use_integer_abs() {
        assert_eq!(int_abs(100.7), 100);
        assert_eq!(int_abs(-100.7), 100);
        assert_eq!(int_abs(0.9), 0);
        // SpinUp reaches full speed once the truncated speeds match.
        assert!(int_abs(100.2) >= int_abs(100.7));
        assert_eq!(int_abs(f32::NAN), i32::MIN);
    }
}
