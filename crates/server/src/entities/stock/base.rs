//! SDK base behaviour shared by the stock classes: `CBaseDelay` (delay, killtarget,
//! `SUB_UseTargets`), `CBaseToggle` (linear/angular moves on `ltime`, wait/lip/master), masters
//! (`UTIL_IsMasterTriggered`), `env_global` state, and keyvalue/precache helpers.
//!
//! SDK member-function pointers (`SetThink`, `SetTouch`, `m_pfnCallWhenMoveDone`) become
//! `Option<fn(&mut T, ...)>` fields on the class; its `Entity::think`/`touch` call them.

use crate::entities::{fire_targets, remove};
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

/// `EMIT_SOUND_DYN`.
/// `!NAME` sentences are resolved through sentences.txt (TS `EMIT_SOUND_DYN`, 0xe0a8c).
pub fn emit(e: *mut edict_t, chan: u32, sample: &CStr, vol: f32, attn: f32, flags: i32, pitch: i32) {
    if sample.is_empty() {
        return;
    }
    if let Some(sample) = super::sentences::resolve(sample) {
        engine().emit_sound(e, chan as i32, &sample, vol, attn, flags, pitch);
    } else {
        unknown_sentence(sample);
    }
}

/// TS `EMIT_SOUND_DYN`'s message for a `!NAME` that isn't in sentences.txt.
pub fn unknown_sentence(sample: &CStr) {
    engine().alert(ALERT_TYPE::at_aiconsole, &format!("Unable to find {} in sentences.txt\n", sample.to_string_lossy()));
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

/// `CBaseEntity::Create(classname, origin, angles, owner)`: create, place and spawn an entity.
pub fn create_entity(classname: &CStr, origin: Vec3, angles: Vec3, owner: *mut edict_t) -> *mut edict_t {
    let eng = engine();
    let e = eng.create_named_entity(classname);
    if e.is_null() || !e.has_private_data() {
        if !e.is_null() {
            eng.remove_entity(e);
        }
        eng.alert(ALERT_TYPE::at_console, &format!("NULL Ent in Create: {}\n", classname.to_string_lossy()));
        return std::ptr::null_mut();
    }
    let v = e.v();
    v.origin = origin;
    v.angles = angles;
    v.owner = owner;
    world::dispatch(e, |ent, cx| ent.spawn(cx));
    e
}

/// `UTIL_PrecacheOther`: spawn a throwaway instance so its class precaches, then free it.
/// (The `Entity` trait has no separate precache; spawning precaches as a side effect.)
pub fn precache_other(classname: &CStr) {
    let eng = engine();
    let e = eng.create_named_entity(classname);
    if e.is_null() {
        return;
    }
    if e.has_private_data() {
        world::dispatch(e, |ent, cx| ent.spawn(cx));
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
        world::with::<DelayedUse, _>(e, |d| d.kill_target = kill_target);
        let v = e.v();
        v.classname = eng.alloc_string(c"DelayedUse");
        v.nextthink = eng.time() + delay;
        v.button = kind as i32;
        v.target = target;
        v.owner = if goldsrc::ext::entities::is_player(activator) { activator } else { std::ptr::null_mut() };
        return;
    }
    if kill_target != 0 {
        let name = s(kill_target);
        let mut k = eng.find_by_target_name(std::ptr::null_mut(), name);
        while !k.is_null() {
            let next = eng.find_by_target_name(k, name);
            remove(k);
            k = next;
        }
    }
    if target != 0 {
        fire_targets(s(target), activator, me, kind, value);
    }
}

/// `DelayedUse`: a `CBaseDelay` created by `SUB_UseTargets` to fire after a delay.
#[derive(Default)]
pub struct DelayedUse {
    kill_target: string_t,
}

impl crate::world::Entity for DelayedUse {
    fn think(&mut self, cx: &mut Cx) {
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

/// `CBaseToggle::LinearMove`: move to `dest` at `speed`, then call `move_done`.
pub fn linear_move<T: Mover>(this: &mut T, cx: &mut Cx, dest: Vec3, speed: f32) {
    this.toggle().final_dest = dest;
    let origin = cx.pev().origin;
    if dest == origin || speed == 0.0 {
        linear_move_done(this, cx);
        return;
    }
    let delta = vec::sub(dest, origin);
    let travel = vec::length(delta) / speed;
    think_ltime(cx, travel);
    this.toggle().think = Some(linear_move_done::<T>);
    cx.pev().velocity = vec::scale(delta, 1.0 / travel);
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

/// `CBaseToggle::AngularMove`.
pub fn angular_move<T: Mover>(this: &mut T, cx: &mut Cx, dest: Vec3, speed: f32) {
    this.toggle().final_angle = dest;
    let angles = cx.pev().angles;
    if dest == angles || speed == 0.0 {
        angular_move_done(this, cx);
        return;
    }
    let delta = vec::sub(dest, angles);
    let travel = vec::length(delta) / speed;
    think_ltime(cx, travel);
    this.toggle().think = Some(angular_move_done::<T>);
    cx.pev().avelocity = vec::scale(delta, 1.0 / travel);
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
pub fn is_master_triggered(master: string_t, _activator: *mut edict_t) -> bool {
    if master == 0 {
        return true;
    }
    let e = engine().find_by_target_name(std::ptr::null_mut(), s(master));
    if !e.is_null() {
        if let Some(t) = master_triggered(e) {
            return t;
        }
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

static GLOBALS: Global<RefCell<Vec<(String, GlobalState)>>> = Global(RefCell::new(Vec::new()));

/// `gGlobalState.EntityGetState`: unknown names are Off.
pub fn global_state(name: &CStr) -> GlobalState {
    let n = kv_str(name);
    GLOBALS.0.borrow().iter().find(|(k, _)| k == n).map(|(_, st)| *st).unwrap_or(GlobalState::Off)
}

pub fn global_exists(name: &CStr) -> bool {
    let n = kv_str(name);
    GLOBALS.0.borrow().iter().any(|(k, _)| k == n)
}

pub fn set_global_state(name: &CStr, st: GlobalState) {
    let n = kv_str(name);
    let mut g = GLOBALS.0.borrow_mut();
    match g.iter_mut().find(|(k, _)| k == n) {
        Some(e) => e.1 = st,
        None => g.push((n.to_owned(), st)),
    }
}

pub fn global_in_table(name: &CStr) -> bool {
    global_exists(name)
}

/// Map change: forget the per-map master registry. `env_global` states persist across maps like
/// the SDK's `gGlobalState` (only `pfnResetGlobalState` clears them).
pub fn reset_registries() {
    MASTERS.0.borrow_mut().take();
}

/// `pfnResetGlobalState`.
pub fn reset_global_state() {
    GLOBALS.0.borrow_mut().clear();
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn axes() {
        assert_eq!(axis_value(SF_DOOR_ROTATE_Z, [1.0, 2.0, 3.0]), 3.0);
        assert_eq!(axis_value(SF_DOOR_ROTATE_X, [1.0, 2.0, 3.0]), 1.0);
        assert_eq!(axis_delta(0, [0.0, 90.0, 0.0], [0.0, 30.0, 0.0]), 60.0);
    }
}
