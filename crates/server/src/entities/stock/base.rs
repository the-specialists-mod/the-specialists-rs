//! SDK base behaviour shared by the stock classes: `CBaseDelay` (delay, killtarget,
//! `SUB_UseTargets`), `CBaseToggle` (linear/angular moves on `ltime`, wait/lip/master), masters
//! (`UTIL_IsMasterTriggered`), `env_global` state, and keyvalue/precache helpers.
//!
//! SDK member-function pointers (`SetThink`, `SetTouch`, `m_pfnCallWhenMoveDone`) become
//! `Option<fn(&mut T, ...)>` fields on the class; its `Entity::think`/`touch` call them.

use crate::entities::{fire_targets, remove};
use crate::save::{Animating, BaseFuncs, Fields};
use crate::world::{self, Cx, UseType};
use goldsrc::Global;
use goldsrc::edict::EdictPtr;
use goldsrc::ext::entities::{EHandle, Vec3, vec};
use goldsrc::server::{engine, intern};
use goldsrc::sys::*;
use goldsrc::util::parse_vec3;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::CStr;

pub type ThinkFn<T> = Option<fn(&mut T, &mut Cx)>;
pub type TouchFn<T> = Option<fn(&mut T, &mut Cx, *mut edict_t)>;

// ---- keyvalue helpers ----------------------------------------------------------------------------

pub fn kv_str(v: &CStr) -> &str {
    v.to_str().unwrap_or("")
}
/// `atof`: leading number, 0 on garbage.
pub fn kv_f(v: &CStr) -> f32 {
    let s = kv_str(v).trim();
    s.parse().unwrap_or_else(|_| {
        let end = s.find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'))).unwrap_or(s.len());
        s[..end].parse().unwrap_or(0.0)
    })
}
/// `strtod`: [`kv_f`] without the rounding to a float (TS converts the double straight to an
/// integer in some keys).
pub fn kv_d(v: &CStr) -> f64 {
    let s = kv_str(v).trim();
    s.parse().unwrap_or_else(|_| {
        let end = s.find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'))).unwrap_or(s.len());
        s[..end].parse().unwrap_or(0.0)
    })
}
/// A `BYTE` member set from `strtod` the way TS's x87 code does it (`CBasePlatTrain::KeyValue`
/// 0xd1f80, `CBaseDoor::KeyValue` 0xb51a0, `CMomentaryDoor::KeyValue` 0xb81a0): `fistp` to a
/// short with the rounding mode set to truncate (out of the short's range, or NaN, stores
/// 0x8000), then the low byte, which the `Precache`s read unsigned.
pub fn fistp16_low_byte(v: f64) -> u8 {
    let t = v.trunc();
    if (-32768.0..=32767.0).contains(&t) { t as i16 as u8 } else { 0 }
}
/// `atoi`.
pub fn kv_i(v: &CStr) -> i32 {
    let s = kv_str(v).trim();
    s.parse().unwrap_or_else(|_| kv_f(v) as i32)
}
pub fn kv_vec(v: &CStr) -> Vec3 {
    parse_vec3(kv_str(v))
}
pub fn alloc(v: &CStr) -> string_t {
    engine().alloc_string(v)
}
/// `STRING(s)`.
pub fn s(st: string_t) -> &'static CStr {
    if st == 0 { c"" } else { engine().string(st) }
}

/// Precache a model named by an engine string (the engine keeps the pointer, so intern it).
pub fn precache_model_s(st: string_t) -> i32 {
    let name = s(st);
    if name.is_empty() {
        return 0;
    }
    engine().precache_model(intern(kv_str(name)))
}
pub fn precache_model(name: &str) -> i32 {
    engine().precache_model(intern(name))
}
pub fn precache_sound_s(st: string_t) -> i32 {
    let name = s(st);
    if name.is_empty() {
        return 0;
    }
    engine().precache_sound(intern(kv_str(name)))
}
pub fn precache_sound(name: &str) -> i32 {
    engine().precache_sound(intern(name))
}

/// `SET_MODEL(ENT(pev), STRING(pev->model))`.
pub fn set_own_model(cx: &mut Cx) {
    let m = cx.pev().model;
    engine().set_model(cx.edict, s(m));
}

/// TS `EMIT_SOUND_DYN`'s message for a `!NAME` that isn't in sentences.txt.
pub fn unknown_sentence(sample: &CStr) {
    engine().alert(ALERT_TYPE::at_aiconsole, &format!("Unable to find {} in sentences.txt\n", sample.to_string_lossy()));
}

/// `CBaseEntity::SUB_Remove` (0xe2d40): `UpdateOnRemove` ([`update_on_remove`]), then an entity
/// removed with health left gets it zeroed and a developer message, then `REMOVE_ENTITY` frees the edict at once (unlike `UTIL_Remove`,
/// `entities::remove`, which waits for the end of the frame). Call it last: the edict is free
/// when it returns.
pub fn sub_remove(e: *mut edict_t) {
    update_on_remove(e);
    let v = e.v();
    if v.health > 0.0 {
        v.health = 0.0;
        engine().alert(ALERT_TYPE::at_aiconsole, "SUB_Remove called on entity with health > 0\n");
    }
    engine().remove_entity(e);
}

/// `pev->nextthink = pev->ltime + delay` (MOVETYPE_PUSH entities think on local time).
pub fn think_ltime(cx: &mut Cx, delay: f32) {
    let v = cx.pev();
    v.nextthink = v.ltime + delay;
}

/// `pev->nextthink = (MOVETYPE_PUSH ? ltime : time) + delay`.
pub fn next_think(cx: &mut Cx, delay: f32) {
    let now = engine().time();
    let v = cx.pev();
    v.nextthink = if v.movetype == MOVETYPE_PUSH as i32 { v.ltime } else { now } + delay;
}

/// `CBaseEntity::Create(classname, origin, angles, owner)` (0xac480): create, place and
/// `DispatchSpawn` an entity (its result is ignored).
pub fn create_entity(classname: &CStr, origin: Vec3, angles: Vec3, owner: *mut edict_t) -> *mut edict_t {
    let eng = engine();
    let e = eng.create_named_entity(classname);
    if e.is_null() || !e.has_private_data() {
        if !e.is_null() {
            eng.remove_entity(e);
        }
        eng.alert(ALERT_TYPE::at_console, "NULL Ent in Create!\n");
        return std::ptr::null_mut();
    }
    let v = e.v();
    v.origin = origin;
    v.angles = angles;
    v.owner = owner;
    crate::entities::dispatch_spawn(e);
    e
}

/// `UTIL_PrecacheOther` (0xeeaf0): create an entity of the class, run only its `Precache`, and
/// free it (`REMOVE_ENTITY`). Its only caller is `func_breakable`'s `spawnobject`; of those classes
/// only the items precache anything (`items::precache_class`): the ground weapons and ammo
/// TS re-points the HL names at have an empty `Precache` (0xa1cd4, 0xa28f0), and a class
/// neither library exports gets the console message.
pub fn precache_other(classname: &CStr) {
    let eng = engine();
    let e = eng.create_named_entity(classname);
    if e.is_null() || eng.index_of(e) == 0 {
        eng.alert(ALERT_TYPE::at_console, "NULL Ent in UTIL_PrecacheOther\n");
        return;
    }
    if e.has_private_data() {
        super::items::precache_class(classname);
    }
    eng.remove_entity(e);
}

/// `VecBModelOrigin`: centre of a brush entity's absolute box.
pub fn bmodel_origin(v: &entvars_t) -> Vec3 {
    vec::mad(v.absmin, v.size, 0.5)
}

/// SDK `SetMovedir`.
pub fn set_movedir(v: &mut entvars_t) {
    if v.angles == [0.0, -1.0, 0.0] {
        v.movedir = [0.0, 0.0, 1.0];
    } else if v.angles == [0.0, -2.0, 0.0] {
        v.movedir = [0.0, 0.0, -1.0];
    } else {
        v.movedir = engine().make_vectors(v.angles).0;
    }
    v.angles = [0.0; 3];
}

pub const SF_DOOR_ROTATE_Z: i32 = 64;
pub const SF_DOOR_ROTATE_X: i32 = 128;

pub fn axis_dir(v: &mut entvars_t) {
    v.movedir = if v.spawnflags & SF_DOOR_ROTATE_Z != 0 {
        [0.0, 0.0, 1.0]
    } else if v.spawnflags & SF_DOOR_ROTATE_X != 0 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
}

pub fn axis_value(flags: i32, a: Vec3) -> f32 {
    if flags & SF_DOOR_ROTATE_Z != 0 {
        a[2]
    } else if flags & SF_DOOR_ROTATE_X != 0 {
        a[0]
    } else {
        a[1]
    }
}

pub fn axis_delta(flags: i32, a: Vec3, b: Vec3) -> f32 {
    axis_value(flags, a) - axis_value(flags, b)
}

/// `a + dir * k` the way TS's x87 code evaluates it in `CMomentaryDoor::Spawn` (0xb7e38) and
/// `CMomentaryRotButton::Spawn` (0xa9550): the y and z products are stored as floats before the
/// add, but the x product stays on the FPU stack, so x is rounded to float only once. `f64` holds
/// the exact float × float product.
pub fn mad_x87(a: Vec3, dir: Vec3, k: f32) -> Vec3 {
    [(a[0] as f64 + k as f64 * dir[0] as f64) as f32, a[1] + k * dir[1], a[2] + k * dir[2]]
}

// ---- CBaseDelay ------------------------------------------------------------------------------------

/// `CBaseDelay`: `delay` and `killtarget`, and `SUB_UseTargets`.
#[derive(Default, Clone, Copy)]
pub struct Delay {
    pub delay: f32,
    pub kill_target: string_t,
}

impl Delay {
    pub fn key_value(&mut self, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"delay" => self.delay = kv_f(value),
            b"killtarget" => self.kill_target = alloc(value),
            _ => return false,
        }
        true
    }

    /// `SUB_UseTargets(pActivator, useType, value)` for the entity `me` (whose `pev->target` is
    /// fired).
    pub fn use_targets(&self, me: *mut edict_t, activator: *mut edict_t, kind: UseType, value: f32) {
        let target = me.v().target;
        sub_use_targets(me, target, self.kill_target, self.delay, activator, kind, value);
    }
}

/// The body of `CBaseDelay::SUB_UseTargets` with explicit fields.
pub fn sub_use_targets(
    me: *mut edict_t,
    target: string_t,
    kill_target: string_t,
    delay: f32,
    activator: *mut edict_t,
    kind: UseType,
    value: f32,
) {
    if target == 0 && kill_target == 0 {
        return;
    }
    let eng = engine();
    if delay != 0.0 {
        // A temporary DelayedUse entity fires later.
        let e = eng.create_entity();
        if e.is_null() {
            return;
        }
        world::create::<DelayedUse>(e.v());
        world::with::<DelayedUse, _>(e, |d| {
            d.kill_target = kill_target;
            d.armed = true;
        });
        let v = e.v();
        v.classname = eng.alloc_string(c"DelayedUse");
        v.nextthink = eng.time() + delay;
        v.button = kind as i32;
        v.target = target;
        v.owner = if crate::rules::has_player(activator) { activator } else { std::ptr::null_mut() };
        return;
    }
    if kill_target != 0 {
        let name = s(kill_target);
        eng.alert(ALERT_TYPE::at_aiconsole, &format!("KillTarget: {}\n", name.to_string_lossy()));
        let mut k = eng.find_by_target_name(std::ptr::null_mut(), name);
        while !k.is_null() {
            let next = eng.find_by_target_name(k, name);
            remove(k);
            eng.alert(ALERT_TYPE::at_aiconsole, &format!("killing {}\n", s(k.v().classname).to_string_lossy()));
            k = next;
        }
    }
    if target != 0 {
        fire_targets(s(target), activator, me, kind, value);
    }
}

/// `DelayedUse`: a `CBaseDelay` created by `SUB_UseTargets` to fire after a delay. A map can
/// place one too (the `DelayedUse` export, 0xe3050), but without `SetThink(DelayThink)` it never
/// does anything.
#[derive(Default)]
pub struct DelayedUse {
    /// `m_flDelay` (+0x64): a map-placed one's `delay` key (`SUB_UseTargets` leaves it 0).
    delay: f32,
    kill_target: string_t,
    /// `SetThink(&CBaseDelay::DelayThink)`, only from `SUB_UseTargets`.
    armed: bool,
}

impl crate::world::Entity for DelayedUse {
    /// `CBaseDelay::KeyValue` (0xe2e5c): `delay` / `killtarget` (the delay is unused without the
    /// think, but saved).
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"delay" => self.delay = kv_f(value),
            b"killtarget" => self.kill_target = alloc(value),
            _ => return false,
        }
        true
    }

    /// `CBaseDelay::Save` / `Restore` (0xe2dbc / 0xe2e0c): `m_pfnThink` is `DelayThink` once
    /// `SUB_UseTargets` armed it. A restored one is never armed again (TS can't restore the
    /// pointer either), so it never fires. `Precache` is `CBaseEntity`'s (empty).
    fn save_fields(&mut self, cx: &mut Cx, f: &mut Fields) {
        if f.base_funcs(cx, BaseFuncs { think: self.armed, ..Default::default() }).think {
            self.armed = false;
        }
        f.delay(&mut self.delay, &mut self.kill_target);
    }

    /// `CBaseDelay::DelayThink`.
    fn think(&mut self, cx: &mut Cx) {
        if !self.armed {
            return;
        }
        let v = cx.pev();
        let (owner, button, target) = (v.owner, v.button, v.target);
        let kind = use_type(button);
        sub_use_targets(cx.edict, target, self.kill_target, 0.0, owner, kind, 0.0);
        remove(cx.edict);
    }
}

pub fn use_type(v: i32) -> UseType {
    match v {
        0 => UseType::Off,
        1 => UseType::On,
        2 => UseType::Set,
        _ => UseType::Toggle,
    }
}

/// `ShouldToggle(useType, currentState)`.
pub fn should_toggle(kind: UseType, on: bool) -> bool {
    !(kind != UseType::Toggle && kind != UseType::Set && ((on && kind == UseType::On) || (!on && kind == UseType::Off)))
}

// ---- CBaseToggle -------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ToggleState {
    AtTop = 0,
    #[default]
    AtBottom = 1,
    GoingUp = 2,
    GoingDown = 3,
}

/// `CBaseToggle` state. `T` is the owning class (for the callbacks).
pub struct Toggle<T> {
    pub delay: Delay,
    pub state: ToggleState,
    pub move_distance: f32,
    pub wait: f32,
    pub lip: f32,
    pub pos1: Vec3,
    pub pos2: Vec3,
    pub angle1: Vec3,
    pub angle2: Vec3,
    pub height: f32,
    pub activator: EHandle,
    pub final_dest: Vec3,
    pub final_angle: Vec3,
    pub master: string_t,
    /// `m_flActivateFinished`, `m_flTWidth`, `m_flTLength`, `m_cTriggersLeft` and
    /// `m_bitsDamageInflict`: saved with the rest of `CBaseToggle`.
    pub activate_finished: f32,
    pub t_width: f32,
    pub t_length: f32,
    pub triggers_left: i32,
    pub damage_inflict: i32,
    pub think: ThinkFn<T>,
    pub touch: TouchFn<T>,
    pub move_done: ThinkFn<T>,
}

impl<T> Default for Toggle<T> {
    fn default() -> Self {
        Self {
            delay: Delay::default(),
            state: ToggleState::AtBottom,
            move_distance: 0.0,
            wait: 0.0,
            lip: 0.0,
            pos1: [0.0; 3],
            pos2: [0.0; 3],
            angle1: [0.0; 3],
            angle2: [0.0; 3],
            height: 0.0,
            activator: EHandle::NULL,
            final_dest: [0.0; 3],
            final_angle: [0.0; 3],
            master: 0,
            activate_finished: 0.0,
            t_width: 0.0,
            t_length: 0.0,
            triggers_left: 0,
            damage_inflict: 0,
            think: None,
            touch: None,
            move_done: None,
        }
    }
}

impl<T> Toggle<T> {
    /// `CBaseToggle::KeyValue` (+ `CBaseDelay::KeyValue`).
    pub fn key_value(&mut self, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"lip" => self.lip = kv_f(value),
            b"wait" => self.wait = kv_f(value),
            b"master" => self.master = alloc(value),
            b"distance" => self.move_distance = kv_f(value),
            _ => return self.delay.key_value(key, value),
        }
        true
    }

    pub fn is_locked_by_master(&self) -> bool {
        self.master != 0 && !is_master_triggered(self.master, self.activator.get())
    }

    pub fn use_targets(&self, me: *mut edict_t, activator: *mut edict_t, kind: UseType, value: f32) {
        self.delay.use_targets(me, activator, kind, value);
    }

    /// The `"CBaseToggle"` table (`m_pfnCallWhenMoveDone` is the `move_done` slot).
    pub fn save_toggle(&mut self, f: &mut Fields) {
        f.table(&crate::save::tables::CBaseToggle);
        let mut state = self.state as i32;
        f.int(&mut state);
        // Guard: TS keeps any int; the port's state is one of the four.
        self.state = match state {
            0 => ToggleState::AtTop,
            2 => ToggleState::GoingUp,
            3 => ToggleState::GoingDown,
            _ => ToggleState::AtBottom,
        };
        f.float(&mut self.activate_finished);
        f.float(&mut self.move_distance);
        f.float(&mut self.wait);
        f.float(&mut self.lip);
        f.float(&mut self.t_width);
        f.float(&mut self.t_length);
        f.vector(&mut self.pos1);
        f.vector(&mut self.pos2);
        f.vector(&mut self.angle1);
        f.vector(&mut self.angle2);
        f.int(&mut self.triggers_left);
        f.float(&mut self.height);
        f.ehandle(&mut self.activator);
        f.func_slot(&mut self.move_done);
        f.vector(&mut self.final_dest);
        f.vector(&mut self.final_angle);
        f.string(&mut self.master);
        f.int(&mut self.damage_inflict);
    }

    /// TS's chain from `CBaseEntity` down to `CBaseToggle`: entvars and `"BASE"` (no goal entity;
    /// `m_pfnThink` / `m_pfnTouch` are the `think` / `touch` slots, `m_pfnUse` / `m_pfnBlocked`
    /// as given), `"CBaseDelay"`, `"CBaseAnimating"` (`anim`), `"CBaseToggle"`. Returns the
    /// `Use` / `Blocked` pointers a restore cleared (the think and touch slots are cleared here).
    pub fn save_chain(&mut self, cx: &mut Cx, f: &mut Fields, anim: &mut Animating, use_: bool, blocked: bool) -> BaseFuncs {
        let funcs = BaseFuncs { think: self.think.is_some(), touch: self.touch.is_some(), use_, blocked };
        let cleared = f.base_funcs(cx, funcs);
        if cleared.think {
            self.think = None;
        }
        if cleared.touch {
            self.touch = None;
        }
        f.delay(&mut self.delay.delay, &mut self.delay.kill_target);
        f.animating(anim);
        self.save_toggle(f);
        cleared
    }
}

/// Classes built on `CBaseToggle`.
pub trait Mover: Sized + 'static {
    fn toggle(&mut self) -> &mut Toggle<Self>;
}

/// Calls the class's current think callback.
pub fn run_think<T: Mover>(this: &mut T, cx: &mut Cx) {
    if let Some(f) = this.toggle().think {
        f(this, cx);
    }
}

pub fn run_touch<T: Mover>(this: &mut T, cx: &mut Cx, other: *mut edict_t) {
    if let Some(f) = this.toggle().touch {
        f(this, cx, other);
    }
}

/// The arithmetic of `CBaseToggle::LinearMove` (0xe3658) and `AngularMove` (0xe386c) for a move
/// from `from` to `dest` (which differ) at `speed`: `(nextthink, velocity)`. The x87 code keeps
/// the exact difference for the length, rounds the length and the stored deltas to floats, and
/// keeps `travel = length / speed` unrounded for `ltime + travel` and the per-axis
/// `delta / travel`. There is no speed check: a speed of 0 gives an infinite `nextthink` and a
/// zero velocity (the mover never arrives), a negative one a move away that finishes at once.
pub fn move_params(dest: Vec3, from: Vec3, speed: f32, ltime: f32) -> (f32, Vec3) {
    let exact = [dest[0] as f64 - from[0] as f64, dest[1] as f64 - from[1] as f64, dest[2] as f64 - from[2] as f64];
    let len = (exact[2] * exact[2] + exact[1] * exact[1] + exact[0] * exact[0]).sqrt() as f32;
    let travel = len as f64 / speed as f64;
    let vel = exact.map(|d| ((d as f32) as f64 / travel) as f32);
    ((travel + ltime as f64) as f32, vel)
}

/// `CBaseToggle::LinearMove` (0xe3658): move to `dest` at `speed`, then call `move_done`.
pub fn linear_move<T: Mover>(this: &mut T, cx: &mut Cx, dest: Vec3, speed: f32) {
    this.toggle().final_dest = dest;
    let origin = cx.pev().origin;
    if dest == origin {
        linear_move_done(this, cx);
        return;
    }
    let (next, vel) = move_params(dest, origin, speed, cx.pev().ltime);
    cx.pev().nextthink = next;
    this.toggle().think = Some(linear_move_done::<T>);
    cx.pev().velocity = vel;
}

pub fn linear_move_done<T: Mover>(this: &mut T, cx: &mut Cx) {
    let dest = this.toggle().final_dest;
    engine().set_origin(cx.edict, dest);
    let v = cx.pev();
    v.velocity = [0.0; 3];
    v.nextthink = -1.0;
    if let Some(f) = this.toggle().move_done {
        f(this, cx);
    }
}

/// `CBaseToggle::AngularMove` (0xe386c): turn to `dest` at `speed` ([`move_params`]).
pub fn angular_move<T: Mover>(this: &mut T, cx: &mut Cx, dest: Vec3, speed: f32) {
    this.toggle().final_angle = dest;
    let angles = cx.pev().angles;
    if dest == angles {
        angular_move_done(this, cx);
        return;
    }
    let (next, avel) = move_params(dest, angles, speed, cx.pev().ltime);
    cx.pev().nextthink = next;
    this.toggle().think = Some(angular_move_done::<T>);
    cx.pev().avelocity = avel;
}

pub fn angular_move_done<T: Mover>(this: &mut T, cx: &mut Cx) {
    let a = this.toggle().final_angle;
    let v = cx.pev();
    v.angles = a;
    v.avelocity = [0.0; 3];
    v.nextthink = -1.0;
    if let Some(f) = this.toggle().move_done {
        f(this, cx);
    }
}

// ---- masters (multisource) and env_global ---------------------------------------------------------

#[derive(Clone, Copy, Default)]
pub struct MasterState {
    /// Still initialising (`SF_MULTI_INIT`).
    pub init: bool,
    /// Every input is on.
    pub all_on: bool,
    pub global: string_t,
}

/// Masters are kept outside the entity objects so a door can query one while the multisource is
/// itself mid-dispatch (take-out dispatch can't reach it then).
static MASTERS: Global<RefCell<Option<HashMap<usize, MasterState>>>> = Global(RefCell::new(None));

pub fn set_master_state(e: *mut edict_t, st: MasterState) {
    MASTERS.0.borrow_mut().get_or_insert_with(HashMap::new).insert(e as usize, st);
}

pub fn clear_master(e: *mut edict_t) {
    if let Some(m) = MASTERS.0.borrow_mut().as_mut() {
        m.remove(&(e as usize));
    }
}

fn master_triggered(e: *mut edict_t) -> Option<bool> {
    let st = *MASTERS.0.borrow().as_ref()?.get(&(e as usize))?;
    Some(!st.init && st.all_on && (st.global == 0 || global_state(s(st.global)) == GlobalState::On))
}

/// `UTIL_IsMasterTriggered`: true when there is no master or it's satisfied.
pub fn is_master_triggered(master: string_t, activator: *mut edict_t) -> bool {
    if master == 0 {
        return true;
    }
    let e = engine().find_by_target_name(std::ptr::null_mut(), s(master));
    if !e.is_null()
        && let Some(t) = master_triggered(e).or_else(|| super::maprules::team_master_triggered(e, activator))
    {
        return t;
    }
    engine().alert(ALERT_TYPE::at_console, "Master was null or not a master!\n");
    true
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GlobalState {
    Off = 0,
    On = 1,
    Dead = 2,
}

/// `gGlobalState`'s entries: name, the map that added it, state, oldest first (TS's list is
/// newest first; `Find` returns the newest entry of a name).
static GLOBALS: Global<RefCell<Vec<(String, String, GlobalState)>>> = Global(RefCell::new(Vec::new()));

/// `gGlobalState.EntityGetState`: unknown names are Off.
pub fn global_state(name: &CStr) -> GlobalState {
    let n = kv_str(name);
    GLOBALS.0.borrow().iter().rev().find(|(k, _, _)| k == n).map(|(_, _, st)| *st).unwrap_or(GlobalState::Off)
}

pub fn global_exists(name: &CStr) -> bool {
    let n = kv_str(name);
    GLOBALS.0.borrow().iter().any(|(k, _, _)| k == n)
}

/// `EntitySetState`, or `EntityAdd(name, gpGlobals->mapname, st)` for a new name.
pub fn set_global_state(name: &CStr, st: GlobalState) {
    let n = kv_str(name);
    let mut g = GLOBALS.0.borrow_mut();
    if !entity_set_state(&mut g, n, st) {
        g.push((n.to_owned(), map_name(), st));
    }
}

/// `STRING(gpGlobals->mapname)`.
pub fn map_name() -> String {
    let eng = engine();
    eng.string(eng.globals().mapname).to_string_lossy().into_owned()
}

/// `CGlobalState::EntitySetState` (0xf3f60): only a name already in the table changes; returns
/// whether it was found.
fn entity_set_state(g: &mut [(String, String, GlobalState)], name: &str, st: GlobalState) -> bool {
    match g.iter_mut().rev().find(|(k, _, _)| k == name) {
        Some(e) => {
            e.2 = st;
            true
        }
        None => false,
    }
}

/// `CBaseEntity::UpdateOnRemove` (0xe2ccc), which `UTIL_Remove` (0xeea90) and `SUB_Remove`
/// (0xe2d40) run first (no TS class overrides it): an entity with a `globalname` marks its global
/// entry dead, so `DispatchSpawn` won't spawn it again on a later map. The first half, clearing
/// `WorldGraph` links to an `FL_GRAPHED` entity, has nothing to clear: the port has no node graph.
pub fn update_on_remove(e: *mut edict_t) {
    let g = e.v().globalname;
    if g != 0 {
        entity_set_state(&mut GLOBALS.0.borrow_mut(), kv_str(s(g)), GlobalState::Dead);
    }
}

/// What `DispatchSpawn` (0xaab8c) does with a spawned entity's `globalname`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GlobalSpawn {
    /// Not in the table: `EntityAdd(globalname, mapname, GLOBAL_ON)`, spawned.
    Added,
    /// In the table and dead: the spawn fails (-1).
    Dead,
    /// Added by another map: `MakeDormant`.
    Dormant,
    /// Added by this map: spawned as is.
    Spawned,
}

/// `DispatchSpawn`'s `gGlobalState` step for `globalname`.
pub fn global_spawn(name: &CStr) -> GlobalSpawn {
    let level = map_name();
    global_spawn_in(&mut GLOBALS.0.borrow_mut(), kv_str(name), level)
}

fn global_spawn_in(g: &mut Vec<(String, String, GlobalState)>, name: &str, level: String) -> GlobalSpawn {
    match g.iter().rev().find(|(k, _, _)| k == name) {
        None => {
            g.push((name.to_owned(), level, GlobalState::On));
            GlobalSpawn::Added
        }
        Some((_, _, GlobalState::Dead)) => GlobalSpawn::Dead,
        Some((_, l, _)) if *l != level => GlobalSpawn::Dormant,
        Some(_) => GlobalSpawn::Spawned,
    }
}

/// `CGlobalState::DumpGlobals` (0xf3e74) entries, newest first (`EntityAdd` prepends to the list):
/// name, the map that added it, state.
pub fn global_list() -> Vec<(String, String, GlobalState)> {
    GLOBALS.0.borrow().iter().rev().cloned().collect()
}

pub fn global_in_table(name: &CStr) -> bool {
    global_exists(name)
}

/// Map change: forget the per-map master registry. `env_global` states persist across maps like
/// the SDK's `gGlobalState` (only `pfnResetGlobalState` clears them).
pub fn reset_registries() {
    MASTERS.0.borrow_mut().take();
}

/// `CGlobalState::ClearStates` (0xf42a8).
pub fn global_clear() {
    GLOBALS.0.borrow_mut().clear();
}

/// `CGlobalState::EntityAdd` (0xf3ee4): a new entry (the newest).
pub fn global_add(name: &str, level: &str, st: GlobalState) {
    GLOBALS.0.borrow_mut().push((name.to_owned(), level.to_owned(), st));
}

/// `CGlobalState::EntityFromTable` (0xf3f98): the entry's map and state.
pub fn global_entry(name: &CStr) -> Option<(String, GlobalState)> {
    let n = kv_str(name);
    GLOBALS.0.borrow().iter().rev().find(|(k, _, _)| k == n).map(|(_, l, st)| (l.clone(), *st))
}

/// The map that owns the entry `name` (`EntityFromTable(name)->levelName`).
pub fn global_level(name: &CStr) -> Option<Vec<u8>> {
    global_entry(name).map(|(l, _)| l.into_bytes())
}

/// `CGlobalState::EntityUpdate` (0xf4258): the entry now belongs to `level`.
pub fn global_update_level(name: &CStr, level: &str) {
    let n = kv_str(name);
    if let Some(e) = GLOBALS.0.borrow_mut().iter_mut().rev().find(|(k, _, _)| k == n) {
        e.1 = level.to_owned();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> Vec<(String, String, GlobalState)> {
        vec![("a".into(), "map1".into(), GlobalState::On), ("b".into(), "map2".into(), GlobalState::Off)]
    }

    #[test]
    fn entity_set_state_only_changes_known_names() {
        let mut g = table();
        assert!(entity_set_state(&mut g, "b", GlobalState::Dead));
        assert_eq!(g[1].2, GlobalState::Dead);
        // UpdateOnRemove's EntitySetState never adds a name.
        assert!(!entity_set_state(&mut g, "c", GlobalState::Dead));
        assert_eq!(g.len(), 2);
    }

    #[test]
    fn dispatch_spawn_global_rules() {
        let mut g = table();
        assert_eq!(global_spawn_in(&mut g, "a", "map1".into()), GlobalSpawn::Spawned);
        assert_eq!(global_spawn_in(&mut g, "a", "map3".into()), GlobalSpawn::Dormant);
        assert_eq!(global_spawn_in(&mut g, "new", "map3".into()), GlobalSpawn::Added);
        assert_eq!(g.last(), Some(&("new".to_owned(), "map3".to_owned(), GlobalState::On)));
        assert_eq!(global_spawn_in(&mut g, "new", "map3".into()), GlobalSpawn::Spawned);
        // A removed global entity (marked dead) doesn't spawn again, on any map.
        entity_set_state(&mut g, "a", GlobalState::Dead);
        assert_eq!(global_spawn_in(&mut g, "a", "map1".into()), GlobalSpawn::Dead);
        assert_eq!(global_spawn_in(&mut g, "a", "map3".into()), GlobalSpawn::Dead);
    }

    #[test]
    fn toggle_rules() {
        assert!(should_toggle(UseType::Toggle, true));
        assert!(should_toggle(UseType::On, false));
        assert!(!should_toggle(UseType::On, true));
        assert!(!should_toggle(UseType::Off, false));
        assert!(should_toggle(UseType::Off, true));
    }

    #[test]
    fn atof_like() {
        assert_eq!(kv_f(c"1.5"), 1.5);
        assert_eq!(kv_f(c" 2 "), 2.0);
        assert_eq!(kv_f(c"3abc"), 3.0);
        assert_eq!(kv_f(c"x"), 0.0);
        assert_eq!(kv_i(c"7.9"), 7);
    }

    #[test]
    fn move_params_divide_without_a_speed_check() {
        // 3-4-5 triangle at 10 u/s: 0.5 s, velocity = delta / travel.
        let (next, vel) = move_params([3.0, 4.0, 0.0], [0.0; 3], 10.0, 2.0);
        assert_eq!(next, 2.5);
        assert_eq!(vel, [6.0, 8.0, 0.0]);
        // Speed 0: never arrives (nextthink inf), no velocity (delta / inf).
        let (next, vel) = move_params([0.0, 0.0, 64.0], [0.0; 3], 0.0, 1.0);
        assert_eq!(next, f32::INFINITY);
        assert_eq!(vel, [0.0, 0.0, 0.0]);
        // Negative speed: a nextthink in the past and the velocity reversed.
        let (next, vel) = move_params([0.0, 0.0, 64.0], [0.0; 3], -32.0, 5.0);
        assert_eq!(next, 3.0);
        assert_eq!(vel, [0.0, 0.0, -32.0]);
        // Division by the unrounded travel, not multiplication by its rounded reciprocal.
        let (_, vel) = move_params([100.0, 0.0, 0.0], [0.0; 3], 3.0, 0.0);
        assert_eq!(vel[0], (100.0f64 / (100.0f64 / 3.0)) as f32);
    }

    #[test]
    fn axes() {
        assert_eq!(axis_value(SF_DOOR_ROTATE_Z, [1.0, 2.0, 3.0]), 3.0);
        assert_eq!(axis_value(SF_DOOR_ROTATE_X, [1.0, 2.0, 3.0]), 1.0);
        assert_eq!(axis_delta(0, [0.0, 90.0, 0.0], [0.0, 30.0, 0.0]), 60.0);
    }

    #[test]
    fn x87_mad() {
        assert_eq!(mad_x87([1.0, 2.0, 3.0], [0.0, 0.0, 1.0], 64.0), [1.0, 2.0, 67.0]);
        // x keeps the exact product: 1 + 3 × 0.1f rounds once.
        let k = 0.1f32;
        assert_eq!(mad_x87([1.0, 1.0, 0.0], [3.0, 3.0, 0.0], k)[0], (1.0f64 + 3.0 * k as f64) as f32);
        assert_eq!(mad_x87([1.0, 1.0, 0.0], [3.0, 3.0, 0.0], k)[1], 1.0 + 3.0 * k);
    }
}
