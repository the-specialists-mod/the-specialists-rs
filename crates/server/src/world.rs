//! Rust objects attached to engine edicts.
//!
//! Each game-owned edict's private data holds a generational [`Handle`] into a slab of
//! `Box<dyn Entity>`. Callbacks use take-out dispatch: the object is moved out of its slot
//! for the duration of the call, so engine calls that re-enter the game (touch, remove,
//! create) never hit a live `RefCell` borrow.

use goldsrc::Global;
use goldsrc::edict::{EdictPtr, private_tag, set_private_tag};
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::any::Any;
use std::cell::RefCell;
use std::ffi::CStr;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Handle {
    index: u32,
    serial: u32,
}

/// Per-callback context: the edict being dispatched to.
pub struct Cx {
    pub edict: *mut edict_t,
}

impl Cx {
    pub fn pev(&mut self) -> &mut entvars_t {
        self.edict.v()
    }
    pub fn index(&self) -> i32 {
        engine().index_of(self.edict)
    }
    pub fn time(&self) -> f32 {
        engine().time()
    }
    /// Schedule `think` after `delay` seconds. For MOVETYPE_PUSH entities the engine compares
    /// `nextthink` against `ltime` instead of the global time (see entities::stock).
    pub fn think_in(&mut self, delay: f32) {
        let now = engine().time();
        self.pev().nextthink = now + delay;
    }
}

/// `USE_TYPE` from the SDK.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UseType {
    Off = 0,
    On = 1,
    Set = 2,
    Toggle = 3,
}

pub use crate::damage::DamageInfo;

/// Behaviour attached to an edict. Every method has a no-op default; implement what the class
/// needs. Methods receive a `Cx` for their own edict and must not hold borrows of other entities
/// across engine calls (see world dispatch notes above).
pub trait Entity: Any {
    /// Class-specific keyvalue. Return true if handled; otherwise the generic entvars parser runs.
    fn key_value(&mut self, _cx: &mut Cx, _key: &CStr, _value: &CStr) -> bool {
        false
    }
    fn spawn(&mut self, _cx: &mut Cx) {}
    fn think(&mut self, _cx: &mut Cx) {}
    fn touch(&mut self, _cx: &mut Cx, _other: *mut edict_t) {}
    fn use_(&mut self, _cx: &mut Cx, _activator: *mut edict_t, _caller: *mut edict_t, _kind: UseType, _value: f32) {}
    fn blocked(&mut self, _cx: &mut Cx, _other: *mut edict_t) {}
    /// Apply damage. Return true if the entity took it.
    fn take_damage(&mut self, _cx: &mut Cx, _dmg: &DamageInfo) -> bool {
        false
    }
    /// `pfnShouldCollide` (NEW_DLL_FUNCTIONS): return false to let `other` pass through.
    fn should_collide(&mut self, _cx: &mut Cx, _other: *mut edict_t) -> bool {
        true
    }
}

enum State {
    Empty,
    Present(Box<dyn Entity>),
    TakenOut,
    FreedWhileOut,
}

struct Slot {
    serial: u32,
    state: State,
}

struct World {
    slots: Vec<Slot>,
    free: Vec<u32>,
}

static WORLD: Global<RefCell<World>> = Global(RefCell::new(World { slots: Vec::new(), free: Vec::new() }));

/// Work waiting for a taken-out object to be returned to its slot (see [`defer`]).
type Deferred = (Handle, Box<dyn FnOnce()>);
static DEFERRED: Global<RefCell<Vec<Deferred>>> = Global(RefCell::new(Vec::new()));

impl World {
    fn insert(&mut self, ent: Box<dyn Entity>) -> Handle {
        let index = match self.free.pop() {
            Some(i) => i,
            None => {
                self.slots.push(Slot { serial: 0, state: State::Empty });
                self.slots.len() as u32 - 1
            }
        };
        let slot = &mut self.slots[index as usize];
        slot.serial = slot.serial.wrapping_add(1);
        slot.state = State::Present(ent);
        Handle { index, serial: slot.serial }
    }

    fn slot(&mut self, h: Handle) -> Option<&mut Slot> {
        self.slots.get_mut(h.index as usize).filter(|s| s.serial == h.serial)
    }

    fn release(&mut self, h: Handle) {
        self.slots[h.index as usize].state = State::Empty;
        self.free.push(h.index);
    }
}

/// The slab handle stored in a game-owned edict's private data.
fn handle_of(e: *const edict_t) -> Option<Handle> {
    private_tag(e).map(|[index, serial]| Handle { index, serial })
}

/// Classname export body: attaches a new `T` to the edict that owns `pev`.
pub fn create<T: Entity + Default>(pev: *mut entvars_t) {
    let e = engine().edict_of_vars(pev);
    if e.is_null() || handle_of(e).is_some() {
        return; // already game-owned (e.g. a client edict reused across maps)
    }
    let h = WORLD.0.borrow_mut().insert(Box::new(T::default()));
    set_private_tag(e, [h.index, h.serial]);
}

/// Runs `f` on the edict's object, with the object taken out of the slab during the call.
/// Returns `None` if the edict isn't game-owned.
pub fn dispatch<R>(e: *mut edict_t, f: impl FnOnce(&mut dyn Entity, &mut Cx) -> R) -> Option<R> {
    let h = handle_of(e)?;
    let mut ent = {
        let mut w = WORLD.0.borrow_mut();
        let slot = w.slot(h)?;
        match std::mem::replace(&mut slot.state, State::TakenOut) {
            State::Present(ent) => ent,
            other => {
                slot.state = other; // re-entrant dispatch to the same entity: skip
                return None;
            }
        }
    };
    let r = f(&mut *ent, &mut Cx { edict: e });
    let back = {
        let mut w = WORLD.0.borrow_mut();
        match w.slot(h) {
            Some(slot) if matches!(slot.state, State::FreedWhileOut) => {
                w.release(h);
                false
            }
            Some(slot) => {
                slot.state = State::Present(ent);
                true
            }
            None => false,
        }
    };
    // Run what other callbacks deferred until this object was back (or drop it if freed).
    let pending: Vec<Deferred> = {
        let mut d = DEFERRED.0.borrow_mut();
        if d.iter().any(|(dh, _)| *dh == h) {
            let (mine, rest): (Vec<_>, Vec<_>) = d.drain(..).partition(|(dh, _)| *dh == h);
            *d = rest;
            mine
        } else {
            Vec::new()
        }
    };
    if back {
        for (_, job) in pending {
            job();
        }
    }
    Some(r)
}

/// True while `e`'s object is taken out for a dispatch further up the stack (so [`with`] and
/// [`dispatch`] can't reach it).
pub fn is_taken_out(e: *const edict_t) -> bool {
    let Some(h) = handle_of(e) else { return false };
    let mut w = WORLD.0.borrow_mut();
    w.slot(h).is_some_and(|s| matches!(s.state, State::TakenOut))
}

/// Runs `job` as soon as `e`'s object is back in its slot: immediately if it isn't taken out,
/// otherwise when the dispatch that holds it returns. Used when a callback needs to update an
/// entity that is busy further up the stack (a player's weapon frame killing someone).
pub fn defer(e: *const edict_t, job: impl FnOnce() + 'static) {
    match handle_of(e) {
        Some(h) if is_taken_out(e) => DEFERRED.0.borrow_mut().push((h, Box::new(job))),
        Some(_) => job(),
        None => {}
    }
}

/// Short, non-reentrant access to a concrete entity type. Never call the engine inside `f`.
pub fn with<T: Entity, R>(e: *const edict_t, f: impl FnOnce(&mut T) -> R) -> Option<R> {
    let h = handle_of(e)?;
    let mut w = WORLD.0.borrow_mut();
    match &mut w.slot(h)?.state {
        State::Present(ent) => (&mut **ent as &mut dyn Any).downcast_mut::<T>().map(f),
        _ => None,
    }
}

/// `pfnOnFreeEntPrivateData`: the engine is about to free the edict's private data.
pub fn free(e: *mut edict_t) {
    let Some(h) = handle_of(e) else { return };
    let dropped = {
        let mut w = WORLD.0.borrow_mut();
        let Some(slot) = w.slot(h) else { return };
        match std::mem::replace(&mut slot.state, State::Empty) {
            State::Present(ent) => {
                w.release(h);
                Some(ent)
            }
            State::TakenOut => {
                slot.state = State::FreedWhileOut;
                None
            }
            _ => None,
        }
    };
    drop(dropped); // outside the borrow, in case a destructor calls the engine
}
