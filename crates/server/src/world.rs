//! Rust objects attached to engine edicts.
//!
//! Each game-owned edict's private data holds a generational [`Handle`] into a slab of
//! `Box<dyn Entity>`. Callbacks use take-out dispatch: the object is moved out of its slot
//! for the duration of the call, so engine calls that re-enter the game (touch, remove,
//! create) never hit a live `RefCell` borrow.

use goldsrc::server::engine;
use goldsrc::sys::*;
use goldsrc::{Global, eng};
use std::any::Any;
use std::cell::RefCell;
use std::ffi::{CStr, c_long};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(C)]
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
        unsafe { &mut (*self.edict).v }
    }
}

pub trait Entity: Any {
    fn key_value(&mut self, _cx: &mut Cx, _key: &CStr, _value: &CStr) -> bool {
        false
    }
    fn spawn(&mut self, _cx: &mut Cx) {}
    fn think(&mut self, _cx: &mut Cx) {}
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

fn handle_of(e: *const edict_t) -> Option<Handle> {
    if e.is_null() {
        return None;
    }
    let p = unsafe { (*e).pvPrivateData }.cast::<Handle>();
    (!p.is_null()).then(|| unsafe { p.read() })
}

/// Classname export body: attaches a new `T` to the edict that owns `pev`.
pub fn create<T: Entity + Default>(pev: *mut entvars_t) {
    let e = engine().edict_of_vars(pev);
    if e.is_null() || handle_of(e).is_some() {
        return; // already game-owned (e.g. a client edict reused across maps)
    }
    let h = WORLD.0.borrow_mut().insert(Box::new(T::default()));
    let p = eng!(pfnPvAllocEntPrivateData(e, size_of::<Handle>() as c_long));
    unsafe { p.cast::<Handle>().write(h) };
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
    let mut w = WORLD.0.borrow_mut();
    if let Some(slot) = w.slot(h) {
        if matches!(slot.state, State::FreedWhileOut) {
            w.release(h);
        } else {
            slot.state = State::Present(ent);
        }
    }
    Some(r)
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
