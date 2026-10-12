//! Save/restore: TS's `CSave` / `CRestore` field chains for every entity, `gGlobalState`, and
//! the engine callbacks (`DispatchSave` 0xaaf18, `DispatchRestore` 0xab128, `SaveGlobalState`
//! 0xf42ec, `RestoreGlobalState` 0xf436c, `ResetGlobalState` 0xf43fc).
//!
//! TS saves an entity by walking its class's `Save` chain: `CBaseEntity::Save` writes the
//! `entvars_t` (`"ENTVARS"`) and `CBaseEntity::m_SaveData` (`"BASE"`), and each class below it
//! writes its own `m_SaveData` under its name (`IMPLEMENT_SAVERESTORE`). [`tables`] holds every
//! table as TS has it, with each class's chain, generated from `ts_i386.so` and the decomp by
//! `save/gen_tables.py`. The port's objects
//! aren't TS's memory, so each class maps its state onto the chain in
//! [`Entity::save_fields`](crate::world::Entity::save_fields) through a [`Fields`] visitor: one
//! call per TS field, in table order, used both ways (save reads the port's fields, restore
//! writes them). Unit tests run every class in probe mode against its TS chain.
//!
//! The bytes are TS's: [`goldsrc::ext::saverestore`] has the codec, the token hashing and the
//! entity-index and time/landmark fix-ups.
//!
//! Function fields (`m_pfnThink`, `m_pfnTouch`, `m_pfnUse`, `m_pfnBlocked`,
//! `m_pfnCallWhenMoveDone`): TS's g++ 2.95 member pointers are 8 bytes, delta and vtable index
//! first, and the table's `FIELD_FUNCTION` entry points at that first word: `0xffff0000` for
//! every non-virtual function. `WriteFunction` hands that word to `pfnNameForFunction`, which no
//! engine can name, so TS logs "Invalid function pointer in entity!" and writes no record for a
//! set function while still counting it in the block's field count. On restore the count then
//! runs past the block and the next `ReadFields` finds the wrong header: the rest of that
//! entity's chain isn't read. The port does the same (it passes the same word to the engine):
//! a class only says whether each of TS's member pointers would be set.

pub mod tables;

use crate::world::{self, Cx};
use goldsrc::edict::EdictPtr;
use goldsrc::ext::entities::EHandle;
use goldsrc::ext::saverestore::{EngineHost, EntRef, FieldDesc, FieldType, Host, SaveData, Value};
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::cell::Cell;
use std::ffi::CStr;

/// One TS `TYPEDESCRIPTION` table and the name `WriteFields` writes it under.
pub struct Table {
    pub name: &'static CStr,
    pub fields: &'static [FieldDesc],
}

/// The first word of a g++ 2.95 pointer to a non-virtual member function (delta 0, index -1),
/// what TS's `FIELD_FUNCTION` entries see for a set function.
pub const PTMF_NONVIRTUAL: u64 = 0xffff_0000;

enum Mode<'a> {
    Save {
        sd: &'a mut SaveData,
        host: &'a dyn Host,
        pending: Vec<Value>,
    },
    Restore {
        sd: &'a mut SaveData,
        host: &'a dyn Host,
        global: bool,
        precache: bool,
        decoded: Option<Vec<Option<Value>>>,
        failed: bool,
    },
    /// Tests: records the tables and field types visited.
    Probe {
        log: Vec<(&'static CStr, Vec<FieldType>)>,
        errors: Vec<String>,
    },
    /// Tests: like a restore, but every field gets a made-up non-zero value (strings are ids
    /// 1..=8, entities null).
    Fill {
        seed: i32,
    },
}

/// The visitor a class's `save_fields` walks its TS chain with (see the module docs).
pub struct Fields<'a> {
    mode: Mode<'a>,
    table: Option<&'static Table>,
    pos: usize,
    /// A restore the engine asked for (`DispatchRestore`), as opposed to a test's: engine calls
    /// are allowed.
    live: bool,
}

/// Which TS member function pointers of `CBaseEntity` are set (non-null).
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct BaseFuncs {
    pub think: bool,
    pub touch: bool,
    pub use_: bool,
    pub blocked: bool,
}

impl<'a> Fields<'a> {
    pub fn save(sd: &'a mut SaveData, host: &'a dyn Host) -> Self {
        Fields { mode: Mode::Save { sd, host, pending: Vec::new() }, table: None, pos: 0, live: false }
    }

    /// A `CRestore` with `m_global` = `global` and `m_precache` on.
    pub fn restore(sd: &'a mut SaveData, host: &'a dyn Host, global: bool) -> Self {
        Self::restore_with(sd, host, global, true)
    }

    pub fn restore_with(sd: &'a mut SaveData, host: &'a dyn Host, global: bool, precache: bool) -> Self {
        Fields { mode: Mode::Restore { sd, host, global, precache, decoded: None, failed: false }, table: None, pos: 0, live: false }
    }

    pub fn probe() -> Fields<'static> {
        Fields { mode: Mode::Probe { log: Vec::new(), errors: Vec::new() }, table: None, pos: 0, live: false }
    }

    /// See `Mode::Fill`.
    pub fn fill() -> Fields<'static> {
        Fields { mode: Mode::Fill { seed: 0 }, table: None, pos: 0, live: false }
    }

    /// A restore the engine asked for: classes may call the engine (the tests run `save_fields`
    /// without one).
    pub fn live(&self) -> bool {
        self.live && self.restoring()
    }

    pub fn restoring(&self) -> bool {
        matches!(self.mode, Mode::Restore { .. })
    }

    /// Restoring, and every table so far was read (`Restore` returning 1 so far).
    pub fn restore_ok(&self) -> bool {
        matches!(self.mode, Mode::Restore { failed: false, .. })
    }

    /// The probe log: each table's name and the field types visited.
    pub fn probe_log(&self) -> (&[(&'static CStr, Vec<FieldType>)], &[String]) {
        match &self.mode {
            Mode::Probe { log, errors } => (log, errors),
            _ => (&[], &[]),
        }
    }

    /// Starts the next table of the chain: `WriteFields` / `ReadFields` for `t`.
    pub fn table(&mut self, t: &'static Table) {
        self.finish();
        self.table = Some(t);
        self.pos = 0;
        match &mut self.mode {
            Mode::Save { .. } => {}
            Mode::Restore { sd, host, global, precache, decoded, failed } => {
                *decoded = if *failed { None } else { sd.read_fields(*host, t.name, t.fields, *global, *precache) };
                if decoded.is_none() {
                    *failed = true;
                }
            }
            Mode::Probe { log, .. } => log.push((t.name, Vec::new())),
            Mode::Fill { .. } => {}
        }
    }

    /// Ends the chain (writes the last table when saving).
    pub fn finish(&mut self) {
        let Some(t) = self.table.take() else { return };
        match &mut self.mode {
            Mode::Save { sd, host, pending } => {
                let mut vals = std::mem::take(pending);
                // Fields the class didn't visit stay empty (the probe tests catch those).
                for d in &t.fields[vals.len().min(t.fields.len())..] {
                    vals.push(Value::zero(d));
                }
                vals.truncate(t.fields.len());
                sd.write_fields(*host, t.name, t.fields, &vals);
            }
            Mode::Probe { errors, .. } if self.pos != t.fields.len() => {
                errors.push(format!("{:?}: visited {} of {} fields", t.name, self.pos, t.fields.len()));
            }
            _ => {}
        }
    }

    /// The next field: on save, records `cur` (TS's contents for it); on restore, returns the
    /// field's new contents when its table was read (else the port keeps its value).
    fn next(&mut self, ok: &[FieldType], cur: Value) -> Option<Value> {
        let Some(t) = self.table else {
            if let Mode::Probe { errors, .. } = &mut self.mode {
                errors.push("field outside a table".into());
            }
            return None;
        };
        let Some(d) = t.fields.get(self.pos) else {
            if let Mode::Probe { errors, .. } = &mut self.mode {
                errors.push(format!("{:?}: more than {} fields", t.name, t.fields.len()));
            }
            return None;
        };
        let i = self.pos;
        self.pos += 1;
        let fits = ok.is_empty() || ok.contains(&d.ty);
        match &mut self.mode {
            Mode::Save { pending, .. } => {
                pending.push(if fits { shaped(d, cur) } else { Value::zero(d) });
                None
            }
            Mode::Restore { decoded, .. } => {
                let v = decoded.as_mut()?.get_mut(i)?.take()?;
                fits.then_some(v)
            }
            Mode::Probe { log, errors } => {
                if !fits {
                    errors.push(format!("{:?}.{:?}: {:?} visited as {:?}", t.name, d.name, d.ty, ok));
                }
                if let Some((_, types)) = log.last_mut() {
                    types.push(d.ty);
                }
                None
            }
            Mode::Fill { seed } => {
                *seed += 1;
                fits.then(|| synth(d, *seed))
            }
        }
    }

    // ---- typed fields ----------------------------------------------------------------------------

    /// `FIELD_FLOAT` / `FIELD_TIME`.
    pub fn float(&mut self, v: &mut f32) {
        if let Some(Value::Floats(f)) = self.next(&[FieldType::Float, FieldType::Time], Value::Floats(vec![*v])) {
            *v = f[0];
        }
    }

    pub fn floats(&mut self, v: &mut [f32]) {
        if let Some(Value::Floats(f)) = self.next(&[FieldType::Float, FieldType::Time], Value::Floats(v.to_vec())) {
            copy_into(v, &f);
        }
    }

    /// A `FIELD_FLOAT` TS declares over an `int` (`entvars_t::flags`, `fixangle`, ...): the bits.
    pub fn float_bits(&mut self, v: &mut i32) {
        if let Some(Value::Floats(f)) = self.next(&[FieldType::Float], Value::Floats(vec![f32::from_bits(*v as u32)])) {
            *v = f[0].to_bits() as i32;
        }
    }

    /// `FIELD_INTEGER` / `FIELD_BOOLEAN` / `FIELD_POINTER`.
    pub fn int(&mut self, v: &mut i32) {
        if let Some(Value::Ints(i)) = self.next(&[FieldType::Integer, FieldType::Boolean, FieldType::Pointer], Value::Ints(vec![*v])) {
            *v = i[0];
        }
    }

    pub fn ints(&mut self, v: &mut [i32]) {
        if let Some(Value::Ints(i)) = self.next(&[FieldType::Integer, FieldType::Boolean], Value::Ints(v.to_vec())) {
            copy_into(v, &i);
        }
    }

    /// A `FIELD_BOOLEAN` (or integer) kept as a `bool`: TRUE is 1.
    pub fn boolean(&mut self, v: &mut bool) {
        let mut i = *v as i32;
        self.int(&mut i);
        *v = i != 0;
    }

    /// `FIELD_STRING` / `FIELD_MODELNAME` / `FIELD_SOUNDNAME`.
    pub fn string(&mut self, v: &mut string_t) {
        if let Some(Value::Ints(i)) = self.next(&[FieldType::String, FieldType::ModelName, FieldType::SoundName], Value::Ints(vec![*v])) {
            *v = i[0];
        }
    }

    pub fn strings(&mut self, v: &mut [string_t]) {
        if let Some(Value::Ints(i)) = self.next(&[FieldType::String], Value::Ints(v.to_vec())) {
            copy_into(v, &i);
        }
    }

    /// `FIELD_VECTOR` / `FIELD_POSITION_VECTOR`.
    pub fn vector(&mut self, v: &mut [f32; 3]) {
        if let Some(Value::Vecs(x)) = self.next(&[FieldType::Vector, FieldType::PositionVector], Value::Vecs(vec![*v])) {
            *v = x[0];
        }
    }

    /// `FIELD_CLASSPTR` / `FIELD_EVARS` / `FIELD_EDICT` / `FIELD_ENTITY`, as the edict (null for
    /// none). A `CLASSPTR` restores only to an edict with a game object.
    pub fn ent(&mut self, v: &mut *mut edict_t) {
        let ok = [FieldType::ClassPtr, FieldType::EVars, FieldType::Edict, FieldType::Entity];
        if let Some(Value::Ents(e)) = self.next(&ok, Value::Ents(vec![EntRef::of(*v)])) {
            *v = e[0].edict;
        }
    }

    pub fn ents(&mut self, v: &mut [*mut edict_t]) {
        let ok = [FieldType::ClassPtr, FieldType::EVars, FieldType::Edict, FieldType::Entity];
        if let Some(Value::Ents(e)) = self.next(&ok, Value::Ents(v.iter().map(|&x| EntRef::of(x)).collect())) {
            for (d, s) in v.iter_mut().zip(e) {
                *d = s.edict;
            }
        }
    }

    /// `FIELD_EHANDLE`: written as the entity it names now (-1 for a stale one).
    pub fn ehandle(&mut self, v: &mut EHandle) {
        if let Some(Value::Ents(e)) = self.next(&[FieldType::EHandle], Value::Ents(vec![ehandle_ref(v)])) {
            *v = EHandle::new(e[0].edict);
        }
    }

    pub fn ehandles(&mut self, v: &mut [EHandle]) {
        if let Some(Value::Ents(e)) = self.next(&[FieldType::EHandle], Value::Ents(v.iter().map(ehandle_ref).collect())) {
            for (d, s) in v.iter_mut().zip(e) {
                *d = EHandle::new(s.edict);
            }
        }
    }

    /// `FIELD_FUNCTION`: whether TS's member pointer is set (see the module docs). Returns
    /// whether the restore cleared it.
    pub fn func(&mut self, set: bool) -> bool {
        self.next(&[FieldType::Function], Value::Func(if set { PTMF_NONVIRTUAL } else { 0 })).is_some()
    }

    /// `FIELD_FUNCTION` for a port callback slot: set when it holds one. A restore clears it: the
    /// only word TS can get back is `pfnFunctionFromName`'s for a name the engine gave out, which
    /// it stores as the member pointer's delta and index with no function, so TS never runs a
    /// restored function either.
    pub fn func_slot<F>(&mut self, slot: &mut Option<F>) {
        if self.func(slot.is_some()) {
            *slot = None;
        }
    }

    /// `FIELD_CHARACTER` arrays (and single bytes).
    pub fn bytes(&mut self, v: &mut [u8]) {
        if let Some(Value::Bytes(b)) = self.next(&[FieldType::Character], Value::Bytes(v.to_vec())) {
            copy_into(v, &b);
        }
    }

    pub fn byte(&mut self, v: &mut u8) {
        self.bytes(std::slice::from_mut(v));
    }

    /// `FIELD_SHORT`.
    pub fn short(&mut self, v: &mut i16) {
        if let Some(Value::Shorts(s)) = self.next(&[FieldType::Short], Value::Shorts(vec![*v])) {
            *v = s[0];
        }
    }

    /// A TS field the port keeps no state for: saved empty, its restored value dropped. Each use
    /// says why the field is always empty (or unused) in TS.
    pub fn zero(&mut self) {
        let d = self.table.and_then(|t| t.fields.get(self.pos)).copied();
        if let Some(d) = d {
            self.next(&[], Value::zero(&d));
        } else {
            self.next(&[], Value::Func(0));
        }
    }

    /// `FIELD_INTEGER` computed from the port's state: saves `cur`, returns the restored value.
    pub fn int_val(&mut self, cur: i32) -> Option<i32> {
        let mut v = cur;
        self.int(&mut v);
        self.restoring().then_some(v)
    }

    // ---- the shared chain heads ---------------------------------------------------------------

    /// `"ENTVARS"`: `gEntvarsDescription` over the edict's `entvars_t`.
    pub fn entvars(&mut self, v: &mut entvars_t) {
        self.table(&tables::ENTVARS);
        self.entvars_fields(v);
    }

    fn entvars_fields(&mut self, v: &mut entvars_t) {
        self.string(&mut v.classname);
        self.string(&mut v.globalname);
        self.vector(&mut v.origin);
        self.vector(&mut v.oldorigin);
        self.vector(&mut v.velocity);
        self.vector(&mut v.basevelocity);
        self.vector(&mut v.movedir);
        self.vector(&mut v.angles);
        self.vector(&mut v.avelocity);
        self.vector(&mut v.punchangle);
        self.vector(&mut v.v_angle);
        self.float_bits(&mut v.fixangle);
        self.float(&mut v.idealpitch);
        self.float(&mut v.pitch_speed);
        self.float(&mut v.ideal_yaw);
        self.float(&mut v.yaw_speed);
        self.int(&mut v.modelindex);
        self.string(&mut v.model);
        self.string(&mut v.viewmodel);
        self.string(&mut v.weaponmodel);
        self.vector(&mut v.absmin);
        self.vector(&mut v.absmax);
        self.vector(&mut v.mins);
        self.vector(&mut v.maxs);
        self.vector(&mut v.size);
        self.float(&mut v.ltime);
        self.float(&mut v.nextthink);
        self.int(&mut v.solid);
        self.int(&mut v.movetype);
        self.int(&mut v.skin);
        self.int(&mut v.body);
        self.int(&mut v.effects);
        self.float(&mut v.gravity);
        self.float(&mut v.friction);
        self.float_bits(&mut v.light_level);
        self.float(&mut v.frame);
        self.float(&mut v.scale);
        self.int(&mut v.sequence);
        self.float(&mut v.animtime);
        self.float(&mut v.framerate);
        // `FIELD_INTEGER` over `byte controller[4]` and `byte blending[2]` (+2 padding bytes,
        // zero in TS's calloc'd edicts).
        let mut c = i32::from_le_bytes(v.controller);
        self.int(&mut c);
        v.controller = c.to_le_bytes();
        let mut b = i32::from_le_bytes([v.blending[0], v.blending[1], 0, 0]);
        self.int(&mut b);
        v.blending = [b as u8, (b >> 8) as u8];
        self.int(&mut v.rendermode);
        self.float(&mut v.renderamt);
        self.vector(&mut v.rendercolor);
        self.int(&mut v.renderfx);
        self.float(&mut v.health);
        self.float(&mut v.frags);
        self.int(&mut v.weapons);
        self.float(&mut v.takedamage);
        self.float_bits(&mut v.deadflag);
        self.vector(&mut v.view_ofs);
        self.int(&mut v.button);
        self.int(&mut v.impulse);
        self.ent(&mut v.chain);
        self.ent(&mut v.dmg_inflictor);
        self.ent(&mut v.enemy);
        self.ent(&mut v.aiment);
        self.ent(&mut v.owner);
        self.ent(&mut v.groundentity);
        self.int(&mut v.spawnflags);
        self.float_bits(&mut v.flags);
        self.int(&mut v.colormap);
        self.int(&mut v.team);
        self.float(&mut v.max_health);
        self.float(&mut v.teleport_time);
        self.float(&mut v.armortype);
        self.float(&mut v.armorvalue);
        self.int(&mut v.waterlevel);
        self.int(&mut v.watertype);
        self.string(&mut v.target);
        self.string(&mut v.targetname);
        self.string(&mut v.netname);
        self.string(&mut v.message);
        self.float(&mut v.dmg_take);
        self.float(&mut v.dmg_save);
        self.float(&mut v.dmg);
        self.float(&mut v.dmgtime);
        self.string(&mut v.noise);
        self.string(&mut v.noise1);
        self.string(&mut v.noise2);
        self.string(&mut v.noise3);
        self.float(&mut v.speed);
        self.float(&mut v.air_finished);
        self.float(&mut v.pain_finished);
        self.float(&mut v.radsuit_finished);
    }

    /// `CBaseEntity::Save` / `Restore` (0xabe40 / 0xabe9c): `"ENTVARS"`, then `"BASE"`
    /// (`m_pGoalEnt` and the four member pointers). A restore then re-applies a saved model:
    /// `PRECACHE_MODEL`, `SET_MODEL` and the saved size (`SET_MODEL` would reset it). Returns the
    /// member pointers a restore cleared.
    pub fn base(&mut self, cx: &mut Cx, goal: &mut *mut edict_t, funcs: BaseFuncs) -> BaseFuncs {
        self.entvars(cx.pev());
        self.table(&tables::BASE);
        self.ent(goal);
        let cleared = BaseFuncs {
            think: self.func(funcs.think),
            touch: self.func(funcs.touch),
            use_: self.func(funcs.use_),
            blocked: self.func(funcs.blocked),
        };
        if self.live() {
            let v = cx.pev();
            if v.modelindex != 0 && v.model != 0 {
                let (mins, maxs, model) = (v.mins, v.maxs, v.model);
                let eng = engine();
                let name = eng.string(model);
                eng.precache_model(name);
                eng.set_model(cx.edict, name);
                eng.set_size(cx.edict, mins, maxs);
            }
        }
        cleared
    }

    /// [`base`](Self::base) for a class with no goal entity.
    pub fn base_funcs(&mut self, cx: &mut Cx, funcs: BaseFuncs) -> BaseFuncs {
        let mut goal = std::ptr::null_mut();
        self.base(cx, &mut goal, funcs)
    }

    /// `"CBaseDelay"`: `m_flDelay`, `m_iszKillTarget`.
    pub fn delay(&mut self, delay: &mut f32, kill_target: &mut string_t) {
        self.table(&tables::CBaseDelay);
        self.float(delay);
        self.string(kill_target);
    }

    /// `"CBaseAnimating"`.
    pub fn animating(&mut self, a: &mut Animating) {
        self.table(&tables::CBaseAnimating);
        self.float(&mut a.frame_rate);
        self.float(&mut a.ground_speed);
        self.float(&mut a.last_event_check);
        self.boolean(&mut a.sequence_finished);
        self.boolean(&mut a.sequence_loops);
    }
}

/// `CBaseAnimating`'s saved state (+0x6c..+0x7c).
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Animating {
    /// `m_flFrameRate`.
    pub frame_rate: f32,
    /// `m_flGroundSpeed`.
    pub ground_speed: f32,
    /// `m_flLastEventCheck`.
    pub last_event_check: f32,
    /// `m_fSequenceFinished`.
    pub sequence_finished: bool,
    /// `m_fSequenceLoops`.
    pub sequence_loops: bool,
}

/// `Mode::Fill`'s value for field `d`.
fn synth(d: &FieldDesc, k: i32) -> Value {
    use FieldType::*;
    let n = d.count.max(0) as usize;
    match d.ty {
        Float | Time => Value::Floats((0..n).map(|j| k as f32 + j as f32 + 0.25).collect()),
        String | ModelName | SoundName => Value::Ints((0..n).map(|j| (k + j as i32) % 8 + 1).collect()),
        Integer | Boolean | Pointer => Value::Ints((0..n).map(|j| k + j as i32).collect()),
        Vector | PositionVector => Value::Vecs((0..n).map(|j| [k as f32, j as f32 + 1.0, 3.5]).collect()),
        Entity | ClassPtr | EHandle | EVars | Edict => Value::Ents(vec![EntRef::NULL; n]),
        // Unset: a set one would end the restore after its table.
        Function => Value::Func(0),
        Short => Value::Shorts((0..n).map(|j| (k + j as i32) as i16).collect()),
        Character => Value::Bytes((0..n).map(|j| (k + j as i32) as u8 | 1).collect()),
    }
}

fn copy_into<T: Copy>(dst: &mut [T], src: &[T]) {
    for (d, s) in dst.iter_mut().zip(src) {
        *d = *s;
    }
}

fn ehandle_ref(h: &EHandle) -> EntRef {
    EntRef::of(h.get())
}

/// `cur` with the descriptor's element count (padded with zeros or cut).
fn shaped(d: &FieldDesc, cur: Value) -> Value {
    let n = d.count.max(0) as usize;
    let zero = Value::zero(d);
    fn fit<T: Clone>(mut v: Vec<T>, n: usize, z: T) -> Vec<T> {
        v.resize(n, z);
        v
    }
    match (cur, zero) {
        (Value::Floats(v), Value::Floats(_)) => Value::Floats(fit(v, n, 0.0)),
        (Value::Ints(v), Value::Ints(_)) => Value::Ints(fit(v, n, 0)),
        (Value::Vecs(v), Value::Vecs(_)) => Value::Vecs(fit(v, n, [0.0; 3])),
        (Value::Ents(v), Value::Ents(_)) => Value::Ents(fit(v, n, EntRef::NULL)),
        (Value::Shorts(v), Value::Shorts(_)) => Value::Shorts(fit(v, n, 0)),
        (Value::Bytes(v), Value::Bytes(_)) => Value::Bytes(fit(v, n, 0)),
        (Value::Func(f), Value::Func(_)) => Value::Func(f),
        (_, z) => z,
    }
}

/// The default `save_fields` for a class whose TS chain the port doesn't map yet: TS's
/// `CBaseEntity` part only (entvars, no goal entity, no member pointers).
pub fn base_only(cx: &mut Cx, f: &mut Fields) {
    f.base_funcs(cx, BaseFuncs::default());
}

// ---- gInitHUD ------------------------------------------------------------------------------------------

/// `gInitHUD`: set by `ResetGlobalState` (a new game), copied into a player's `m_fInitHUD` by
/// `CBasePlayer::Precache`, cleared when a player's HUD is initialised.
static INIT_HUD: goldsrc::Global<Cell<bool>> = goldsrc::Global(Cell::new(true));

pub fn init_hud() -> bool {
    INIT_HUD.0.get()
}

pub fn set_init_hud(on: bool) {
    INIT_HUD.0.set(on);
}

// ---- engine callbacks ---------------------------------------------------------------------------------

/// `FCAP_DONT_SAVE`.
const FCAP_DONT_SAVE: u32 = 0x8000_0000;
/// `FCAP_MUST_SPAWN`.
const FCAP_MUST_SPAWN: u32 = 0x4;

/// TS's `ObjectCaps()` for the edict's object (0 for a name TS doesn't know).
fn object_caps(e: *mut edict_t) -> u32 {
    let v = e.v();
    crate::entities::stock::levelchange::ts_object_caps(crate::entities::stock::base::s(v.classname), v).unwrap_or(0)
}

/// `DispatchSave` (0xaaf18): `pTable[currentIndex]` gets the entity's location, classname and
/// size around its `Save` chain. `FCAP_DONT_SAVE` classes write nothing. A `MOVETYPE_PUSH`
/// entity's `ltime` becomes the current time first, its `nextthink` moved with it.
///
/// Guard: TS writes `pTable[currentIndex]` without a bound; the port skips an index outside it.
pub fn dispatch_save(e: *mut edict_t, sd: &mut SaveData) {
    if e.is_null() || !e.has_private_data() {
        return;
    }
    let index = sd.current_index();
    if sd.table_entry(index).is_none_or(|t| t.pent != e) {
        engine().alert(ALERT_TYPE::at_error, "ENTITY TABLE OR INDEX IS WRONG!!!!\n");
    }
    if object_caps(e) & FCAP_DONT_SAVE != 0 {
        return;
    }
    let v = e.v();
    if v.movetype == MOVETYPE_PUSH as i32 {
        let delta = v.ltime;
        v.ltime = engine().time();
        v.nextthink = (v.nextthink - delta) + v.ltime;
    }
    let location = sd.size();
    let classname = v.classname;
    if let Some(t) = sd.table_entry(index) {
        sd.set_table_entry(index, location, t.size, classname);
    }
    let host = EngineHost;
    let mut f = Fields::save(sd, &host);
    world::dispatch(e, |ent, cx| ent.save_fields(cx, &mut f));
    f.finish();
    drop(f);
    if let Some(t) = sd.table_entry(index) {
        sd.set_table_entry(index, t.location, sd.size() - t.location, t.classname);
    }
}

/// `DispatchRestore` (0xab128). For a global entity (`globalEntity`), the saved entvars are read
/// first to find the live entity of the same `globalname` and class (`FindGlobalEntity`
/// 0xab05c) and restore over it in global mode (its global fields stay), with the landmark offset
/// moved by the two entities' `mins`; it fails when the save's map didn't own the global. Then
/// the object's `Restore` chain, then its `Precache` (`Spawn` for `FCAP_MUST_SPAWN`). A non-global
/// entity with a `globalname` is checked against `gGlobalState`: dead fails (-1), another map's
/// goes dormant, an unknown one is added. A global one is relinked and `OverrideReset`.
pub fn dispatch_restore(pent: *mut edict_t, sd: &mut SaveData, global_entity: bool) -> i32 {
    use crate::entities::stock::base::{GlobalState, s};
    if pent.is_null() || !pent.has_private_data() {
        return 0;
    }
    let eng = engine();
    let host = EngineHost;
    let mut pent = pent;
    let mut global_mode = false;
    let mut old_offset = [0.0f32; 3];
    if global_entity {
        // The saved entvars into a scratch entvars_t (global mode off, no precaching).
        let mut tmp = entvars_t::default();
        {
            let mut f = Fields::restore_with(sd, &host, false, false);
            f.entvars(&mut tmp);
            f.finish();
        }
        if let Some(t) = sd.table_entry(sd.current_index()) {
            sd.seek(t.location);
        }
        // Guard: TS reads the table entry of a global name it doesn't have through NULL.
        let level = crate::entities::stock::base::global_level(s(tmp.globalname));
        if level.as_deref() != Some(&sd.current_map_name()[..]) {
            return 0;
        }
        old_offset = sd.landmark_offset();
        let Some(found) = find_global_entity(tmp.classname, tmp.globalname) else { return 0 };
        global_mode = true;
        let mins = found.v().mins;
        let off = sd.landmark_offset();
        sd.set_landmark_offset([0, 1, 2].map(|i| (off[i] - mins[i]) + tmp.mins[i]));
        pent = found;
        crate::entities::stock::base::global_update_level(s(found.v().globalname), &crate::entities::stock::base::map_name());
    }
    let caps = object_caps(pent);
    let mut f = Fields::restore(sd, &host, global_mode);
    f.live = true;
    world::dispatch(pent, |ent, cx| {
        ent.save_fields(cx, &mut f);
        f.finish();
        if caps & FCAP_MUST_SPAWN != 0 {
            ent.spawn(cx);
        } else {
            ent.restored(cx);
        }
    });
    drop(f);
    let alive = pent.has_private_data() && !pent.is_free();
    if !global_entity {
        if alive && pent.v().globalname != 0 {
            let name = s(pent.v().globalname);
            match crate::entities::stock::base::global_entry(name) {
                None => {
                    eng.alert(
                        ALERT_TYPE::at_error,
                        &format!(
                            "Global Entity {} ({}) not in table!!!\n",
                            name.to_string_lossy(),
                            s(pent.v().classname).to_string_lossy()
                        ),
                    );
                    crate::entities::stock::base::global_add(
                        crate::entities::stock::base::kv_str(name),
                        &crate::entities::stock::base::map_name(),
                        GlobalState::On,
                    );
                }
                Some((_, GlobalState::Dead)) => return -1,
                Some((level, _)) => {
                    if level != crate::entities::stock::base::map_name() {
                        crate::entities::make_dormant(pent);
                    }
                }
            }
        }
    } else {
        sd.set_landmark_offset(old_offset);
        if alive {
            let origin = pent.v().origin;
            eng.set_origin(pent, origin);
            crate::ham::override_reset(pent);
        }
    }
    0
}

/// `FindGlobalEntity` (0xab05c): the entity with this `globalname` (the world when there is
/// none), if its class name matches.
fn find_global_entity(classname: string_t, globalname: string_t) -> Option<*mut edict_t> {
    use crate::entities::stock::base::s;
    let eng = engine();
    let mut e = eng.find_by_string(std::ptr::null_mut(), c"globalname", s(globalname));
    if e.is_null() {
        e = eng.entity_of_index(0);
    }
    if e.is_null() || !e.has_private_data() {
        return None;
    }
    let found = s(e.v().classname);
    if found != s(classname) {
        eng.alert(
            ALERT_TYPE::at_console,
            &format!("Global entity found {}, wrong class {}\n", s(globalname).to_string_lossy(), found.to_string_lossy()),
        );
        return None;
    }
    Some(e)
}

/// `SaveGlobalState` (0xf42ec) → `CGlobalState::Save` (0xf3ff8): `"GLOBAL"` (`m_listCount`),
/// then a `"GENT"` block per entry, newest first (the list's order).
pub fn save_global_state(sd: &mut SaveData) {
    save_global_state_with(sd, &EngineHost);
}

fn save_global_state_with(sd: &mut SaveData, host: &dyn Host) {
    let list = crate::entities::stock::base::global_list();
    let t = &tables::GLOBAL;
    sd.write_fields(host, t.name, t.fields, &[Value::Ints(vec![list.len() as i32])]);
    let g = &tables::GENT;
    for (name, level, state) in list {
        sd.write_fields(
            host,
            g.name,
            g.fields,
            &[Value::Bytes(c_field(&name, 64)), Value::Bytes(c_field(&level, 32)), Value::Ints(vec![state as i32])],
        );
    }
}

/// A `char[n]` holding `s`: TS `strcpy`s into it (a longer name runs past the field; the port
/// cuts it to `n - 1` bytes and a NUL).
fn c_field(s: &str, n: usize) -> Vec<u8> {
    let mut v = s.as_bytes()[..s.len().min(n - 1)].to_vec();
    v.resize(n, 0);
    v
}

fn c_str_of(b: &[u8]) -> String {
    String::from_utf8_lossy(&b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())]).into_owned()
}

/// `RestoreGlobalState` (0xf436c) → `CGlobalState::Restore` (0xf4088): clears the table, reads
/// `"GLOBAL"`, then that many `"GENT"` blocks, each `EntityAdd`ed (TS's list prepends, so it ends
/// up in the saved order reversed: the oldest entry was saved last); a block that isn't there
/// stops it. `RestoreGlobalState` reads with `m_precache` on, which these tables don't use.
pub fn restore_global_state(sd: &mut SaveData) {
    restore_global_state_with(sd, &EngineHost);
}

fn restore_global_state_with(sd: &mut SaveData, host: &dyn Host) {
    use crate::entities::stock::base::{GlobalState, global_add, global_clear};
    global_clear();
    let t = &tables::GLOBAL;
    let Some(out) = sd.read_fields(host, t.name, t.fields, false, true) else { return };
    let count = match &out[0] {
        Some(Value::Ints(i)) => i[0],
        _ => 0,
    };
    let g = &tables::GENT;
    for _ in 0..count.max(0) {
        let Some(out) = sd.read_fields(host, g.name, g.fields, false, true) else { return };
        let bytes = |i: usize| match &out[i] {
            Some(Value::Bytes(b)) => c_str_of(b),
            _ => String::new(),
        };
        let state = match &out[2] {
            Some(Value::Ints(i)) => i[0],
            _ => 0,
        };
        let state = match state {
            1 => GlobalState::On,
            2 => GlobalState::Dead,
            _ => GlobalState::Off,
        };
        global_add(&bytes(0), &bytes(1), state);
    }
}

/// `ResetGlobalState` (0xf43fc): an empty `gGlobalState` and `gInitHUD` on.
pub fn reset_global_state() {
    crate::entities::stock::base::global_clear();
    set_init_hud(true);
}

#[cfg(test)]
mod tests;
