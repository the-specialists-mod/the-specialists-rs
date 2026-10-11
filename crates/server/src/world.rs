//! Rust objects attached to engine edicts.
//!
//! Each game-owned edict's private data holds a generational [`Handle`] into a slab of
//! `Box<dyn Entity>`. Callbacks use take-out dispatch: the object is moved out of its slot
//! for the duration of the call, so engine calls that re-enter the game (touch, remove,
//! create) never hit a live `RefCell` borrow.
//!
//! An object that is out stays unreachable to other callbacks until its dispatch returns: they
//! skip it, or [`defer`] work on it until then. A callback that holds the object as `&mut T` can
//! [`lend`] it back to its slot around a call that must reach it at once, the way TS calls into
//! the player inside its own think (`game_player_team` changing the team of the player whose
//! +use fired it, or shot the button that fired it). The lent object stays closed to ordinary
//! dispatch and `defer` still queues; only [`run_now_or_defer`] and [`run_now`] open it.

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

/// A linked classname and a constructor for its class (the save-table tests).
#[cfg(test)]
pub type Linked = (&'static str, fn() -> Box<dyn Entity>);

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
    /// `TraceAttack` (vtable +0x54): the hit's effects, then whatever damage the entity takes goes
    /// into the multi-damage (`crate::damage::multi`) for one `take_damage` from
    /// `ApplyMultiDamage`. The default is `CBaseEntity::TraceAttack` (0xb3740): add the damage if
    /// `takedamage` is on (no blood: the port's entities don't bleed).
    fn trace_attack(&mut self, cx: &mut Cx, dmg: &DamageInfo) {
        if cx.pev().takedamage != 0.0 {
            crate::damage::multi::add(dmg.attacker, cx.edict, dmg.damage, dmg.bits);
        }
    }
    /// Apply damage (`TakeDamage`, vtable +0x58). Return true if the entity took it. The default
    /// is `CBaseEntity::TakeDamage` (0xab994, `entities::base_take_damage`), which every TS class
    /// without its own inherits.
    fn take_damage(&mut self, cx: &mut Cx, dmg: &DamageInfo) -> bool {
        crate::entities::base_take_damage(cx, dmg)
    }
    /// `pfnShouldCollide` (NEW_DLL_FUNCTIONS): return false to let `other` pass through.
    fn should_collide(&mut self, _cx: &mut Cx, _other: *mut edict_t) -> bool {
        true
    }
    /// `Save` / `Restore` (vtable +0x30 / +0x34): the class's TS field chain, walked with the
    /// visitor in both directions (`crate::save`). The default is `CBaseEntity`'s part alone.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut crate::save::Fields) {
        crate::save::base_only(cx, f);
    }
    /// What `DispatchRestore` runs after the restore: the class's `Precache` (vtable +0x28; no TS
    /// class has `FCAP_MUST_SPAWN`, which would run `Spawn`).
    fn restored(&mut self, _cx: &mut Cx) {}
    /// `OverrideReset` (vtable +0x98): after a global entity was restored over this one.
    fn override_reset(&mut self, _cx: &mut Cx) {}
    /// `Activate` (vtable +0x3c): `ServerActivate` runs it on every entity once the map has
    /// spawned, and again after a restore (`entities::server_activate`). The default is
    /// `CBaseEntity::Activate` (0xac64c), which does nothing.
    fn activate(&mut self, _cx: &mut Cx) {}
}

enum State {
    Empty,
    Present(Box<dyn Entity>),
    TakenOut,
    FreedWhileOut,
    /// Taken out, then put back by [`lend`] while its dispatch is suspended: still out to
    /// everyone but [`run_now_or_defer`].
    Parked(Box<dyn Entity>),
}

struct Slot {
    serial: u32,
    state: State,
    /// The dispatch that took the object out is suspended in [`lend`]. While set, only that
    /// dispatch releases the slot, and the object's deferred work waits for it.
    lent: bool,
}

struct World {
    slots: Vec<Slot>,
    free: Vec<u32>,
}

/// Work waiting for a taken-out object to be returned to its slot (see [`defer`]).
type Deferred = (Handle, Box<dyn FnOnce()>);

/// The slab and the deferred work. The game has one ([`STORE`]); tests make their own.
struct Store {
    world: RefCell<World>,
    deferred: RefCell<Vec<Deferred>>,
}

static STORE: Global<Store> = Global(Store::new());

impl World {
    fn insert(&mut self, ent: Box<dyn Entity>) -> Handle {
        let index = match self.free.pop() {
            Some(i) => i,
            None => {
                self.slots.push(Slot { serial: 0, state: State::Empty, lent: false });
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

impl Store {
    const fn new() -> Self {
        Store { world: RefCell::new(World { slots: Vec::new(), free: Vec::new() }), deferred: RefCell::new(Vec::new()) }
    }

    fn insert(&self, ent: Box<dyn Entity>) -> Handle {
        self.world.borrow_mut().insert(ent)
    }

    fn dispatch<R>(&self, h: Handle, e: *mut edict_t, f: impl FnOnce(&mut dyn Entity, &mut Cx) -> R) -> Option<R> {
        let mut ent = {
            let mut w = self.world.borrow_mut();
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
        // `None`: the object is lent, and its deferred work waits for the dispatch that lent it.
        let back = {
            let mut w = self.world.borrow_mut();
            match w.slot(h) {
                // Opened by `run_now_or_defer`: the suspended dispatch that lent it frees the slot.
                Some(slot) if slot.lent => {
                    if !matches!(slot.state, State::FreedWhileOut) {
                        slot.state = State::Present(ent);
                    }
                    None
                }
                Some(slot) if matches!(slot.state, State::FreedWhileOut) => {
                    w.release(h);
                    Some(false)
                }
                Some(slot) => {
                    slot.state = State::Present(ent);
                    Some(true)
                }
                None => Some(false),
            }
        };
        let Some(back) = back else { return Some(r) };
        // Run what other callbacks deferred until this object was back (or drop it if freed).
        let pending: Vec<Deferred> = {
            let mut d = self.deferred.borrow_mut();
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

    fn is_taken_out(&self, h: Handle) -> bool {
        self.world.borrow_mut().slot(h).is_some_and(|s| matches!(s.state, State::TakenOut | State::Parked(_)))
    }

    fn defer(&self, h: Handle, job: impl FnOnce() + 'static) {
        if self.is_taken_out(h) {
            self.deferred.borrow_mut().push((h, Box::new(job)));
        } else {
            job();
        }
    }

    fn lend<T: Entity + Default, R>(&self, h: Option<Handle>, obj: &mut T, f: impl FnOnce() -> R) -> R {
        let parked = h.filter(|&h| {
            let mut w = self.world.borrow_mut();
            match w.slot(h) {
                Some(slot) if !slot.lent && matches!(slot.state, State::TakenOut) => {
                    slot.state = State::Parked(Box::new(std::mem::take(obj)));
                    slot.lent = true;
                    true
                }
                _ => false,
            }
        });
        let r = f();
        if let Some(h) = parked {
            let back = {
                let mut w = self.world.borrow_mut();
                w.slot(h).and_then(|slot| {
                    slot.lent = false;
                    // Freed meanwhile: leave `FreedWhileOut` for the dispatch that holds the slot.
                    match std::mem::replace(&mut slot.state, State::TakenOut) {
                        State::Parked(ent) => Some(ent),
                        other => {
                            slot.state = other;
                            None
                        }
                    }
                })
            };
            if let Some(mut ent) = back
                && let Some(o) = (&mut *ent as &mut dyn Any).downcast_mut::<T>()
            {
                *obj = std::mem::take(o);
            }
        }
        r
    }

    /// Opens a lent object for ordinary access: `None` when it isn't lent, else whether this call
    /// opened it (it may be open already, or freed).
    fn open_lent(&self, h: Handle) -> Option<bool> {
        let mut w = self.world.borrow_mut();
        let slot = w.slot(h).filter(|s| s.lent)?;
        match std::mem::replace(&mut slot.state, State::TakenOut) {
            State::Parked(ent) => {
                slot.state = State::Present(ent);
                Some(true)
            }
            other => {
                slot.state = other;
                Some(false)
            }
        }
    }

    /// Closes a lent object that `open_lent` opened (a freed one stays `FreedWhileOut`).
    fn close_lent(&self, h: Handle) {
        let mut w = self.world.borrow_mut();
        let Some(slot) = w.slot(h).filter(|s| s.lent) else { return };
        match std::mem::replace(&mut slot.state, State::TakenOut) {
            State::Present(ent) => slot.state = State::Parked(ent),
            other => slot.state = other,
        }
    }

    fn run_now_or_defer(&self, h: Handle, job: impl FnOnce() + 'static) {
        match self.open_lent(h) {
            Some(opened) => {
                job();
                if opened {
                    self.close_lent(h);
                }
            }
            None => self.defer(h, job),
        }
    }

    fn run_now<R>(&self, h: Handle, job: impl FnOnce() -> R) -> Option<R> {
        match self.open_lent(h) {
            Some(opened) => {
                let r = job();
                if opened {
                    self.close_lent(h);
                }
                Some(r)
            }
            None if self.is_taken_out(h) => None,
            None => Some(job()),
        }
    }

    fn with<T: Entity, R>(&self, h: Handle, f: impl FnOnce(&mut T) -> R) -> Option<R> {
        let mut w = self.world.borrow_mut();
        match &mut w.slot(h)?.state {
            State::Present(ent) => (&mut **ent as &mut dyn Any).downcast_mut::<T>().map(f),
            _ => None,
        }
    }

    fn with_lent<T: Entity, R>(&self, h: Handle, f: impl FnOnce(&T) -> R) -> Option<R> {
        let mut w = self.world.borrow_mut();
        match &w.slot(h)?.state {
            State::Present(ent) | State::Parked(ent) => (&**ent as &dyn Any).downcast_ref::<T>().map(f),
            _ => None,
        }
    }

    fn free(&self, h: Handle) {
        let dropped = {
            let mut w = self.world.borrow_mut();
            let Some(slot) = w.slot(h) else { return };
            match std::mem::replace(&mut slot.state, State::Empty) {
                // Lent: the suspended dispatch releases the slot when it returns.
                State::Present(ent) | State::Parked(ent) if slot.lent => {
                    slot.state = State::FreedWhileOut;
                    Some(ent)
                }
                State::Present(ent) => {
                    w.release(h);
                    Some(ent)
                }
                State::TakenOut => {
                    slot.state = State::FreedWhileOut;
                    None
                }
                other => {
                    slot.state = other;
                    None
                }
            }
        };
        drop(dropped); // outside the borrow, in case a destructor calls the engine
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
    let h = STORE.0.insert(Box::new(T::default()));
    set_private_tag(e, [h.index, h.serial]);
}

/// Runs `f` on the edict's object, with the object taken out of the slab during the call.
/// Returns `None` if the edict isn't game-owned.
pub fn dispatch<R>(e: *mut edict_t, f: impl FnOnce(&mut dyn Entity, &mut Cx) -> R) -> Option<R> {
    STORE.0.dispatch(handle_of(e)?, e, f)
}

/// True while `e`'s object is taken out for a dispatch further up the stack (so [`with`] and
/// [`dispatch`] can't reach it), lent back or not.
pub fn is_taken_out(e: *const edict_t) -> bool {
    handle_of(e).is_some_and(|h| STORE.0.is_taken_out(h))
}

/// Runs `job` as soon as `e`'s object is back in its slot: immediately if it isn't taken out,
/// otherwise when the dispatch that holds it returns. Used when a callback needs to update an
/// entity that is busy further up the stack (a player's weapon frame killing someone).
pub fn defer(e: *const edict_t, job: impl FnOnce() + 'static) {
    if let Some(h) = handle_of(e) {
        STORE.0.defer(h, job);
    }
}

/// Puts `obj`, the object of `e` that a dispatch further up the stack took out, back in its
/// slot for the duration of `f`, so [`run_now_or_defer`] can reach it. Until `f` returns, `obj`
/// holds `T::default()`. Runs `f` alone when `e`'s object isn't out (or is already lent).
pub fn lend<T: Entity + Default, R>(e: *const edict_t, obj: &mut T, f: impl FnOnce() -> R) -> R {
    STORE.0.lend(handle_of(e), obj, f)
}

/// Runs `job` now when `e`'s object is lent ([`lend`]), with the object reachable through
/// [`dispatch`] and [`with`] meanwhile; otherwise as [`defer`] does.
pub fn run_now_or_defer(e: *const edict_t, job: impl FnOnce() + 'static) {
    if let Some(h) = handle_of(e) {
        STORE.0.run_now_or_defer(h, job);
    }
}

/// Runs `job` now when `e`'s object can be reached meanwhile: it is in its slot, or lent
/// ([`lend`], opened for the call as [`run_now_or_defer`] does). `None`, without running `job`,
/// while a dispatch further up the stack holds it without lending it (or `e` isn't game-owned).
/// For callers that need the answer at once, as TS gets it (`GiveNamedItem`'s item touching a
/// player whose own `Use` is on the stack).
pub fn run_now<R>(e: *const edict_t, job: impl FnOnce() -> R) -> Option<R> {
    STORE.0.run_now(handle_of(e)?, job)
}

/// Short, non-reentrant access to a concrete entity type. Never call the engine inside `f`.
pub fn with<T: Entity, R>(e: *const edict_t, f: impl FnOnce(&mut T) -> R) -> Option<R> {
    STORE.0.with(handle_of(e)?, f)
}

/// Read-only [`with`] that also reaches an object lent back to its slot by [`lend`]: TS reads
/// the private data of an object whose own call is further up the stack (`PlayerKilled` naming
/// the thrown knife that is inside its touch). Never call the engine inside `f`.
pub fn with_lent<T: Entity, R>(e: *const edict_t, f: impl FnOnce(&T) -> R) -> Option<R> {
    STORE.0.with_lent(handle_of(e)?, f)
}

/// `pfnOnFreeEntPrivateData`: the engine is about to free the edict's private data.
pub fn free(e: *mut edict_t) {
    if let Some(h) = handle_of(e) {
        STORE.0.free(h);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    #[derive(Default)]
    struct Thing {
        value: i32,
    }
    impl Entity for Thing {}

    fn store() -> Rc<Store> {
        Rc::new(Store::new())
    }

    fn value(s: &Store, h: Handle) -> Option<i32> {
        s.with::<Thing, _>(h, |t| t.value)
    }

    #[test]
    fn taken_out_objects_are_skipped_and_deferred() {
        let s = store();
        let h = s.insert(Box::new(Thing { value: 1 }));
        let s2 = s.clone();
        s.dispatch(h, std::ptr::null_mut(), |_, _| {
            assert!(s2.is_taken_out(h));
            assert!(s2.dispatch(h, std::ptr::null_mut(), |_, _| ()).is_none(), "re-entrant dispatch skips");
            let s3 = s2.clone();
            s2.defer(h, move || s3.with::<Thing, _>(h, |t| t.value = 2).unwrap());
            assert_eq!(value(&s2, h), None);
        });
        assert_eq!(value(&s, h), Some(2), "deferred work ran once the object was back");
    }

    #[test]
    fn lent_object_is_reachable_only_through_run_now_or_defer() {
        let s = store();
        let h = s.insert(Box::new(Thing { value: 1 }));
        let s2 = s.clone();
        s.dispatch(h, std::ptr::null_mut(), |ent, _| {
            let me = (ent as &mut dyn Any).downcast_mut::<Thing>().unwrap();
            // Queued before the lend: still waits for this dispatch.
            let s3 = s2.clone();
            s2.defer(h, move || s3.with::<Thing, _>(h, |t| t.value *= 10).unwrap());
            s2.lend(Some(h), me, || {
                assert!(s2.is_taken_out(h), "a lent object is still out");
                assert!(s2.dispatch(h, std::ptr::null_mut(), |_, _| ()).is_none());
                assert_eq!(value(&s2, h), None);
                let s3 = s2.clone();
                s2.run_now_or_defer(h, move || {
                    // Open: ordinary dispatch reaches it, and new deferred work runs at once.
                    s3.dispatch(h, std::ptr::null_mut(), |e, _| (e as &mut dyn Any).downcast_mut::<Thing>().unwrap().value += 4).unwrap();
                    let s4 = s3.clone();
                    s3.defer(h, move || s4.with::<Thing, _>(h, |t| t.value += 1).unwrap());
                    assert_eq!(value(&s3, h), Some(6));
                });
                assert!(s2.is_taken_out(h), "closed again");
            });
            assert_eq!(me.value, 6, "the lender gets the object back with the changes");
            me.value += 1;
        });
        assert_eq!(value(&s, h), Some(70), "the earlier deferred job ran after the dispatch");
    }

    #[test]
    fn run_now_or_defer_defers_without_a_lend() {
        let s = store();
        let h = s.insert(Box::new(Thing { value: 1 }));
        let s2 = s.clone();
        s.dispatch(h, std::ptr::null_mut(), |_, _| {
            let s3 = s2.clone();
            s2.run_now_or_defer(h, move || s3.with::<Thing, _>(h, |t| t.value = 5).unwrap());
            assert_eq!(value(&s2, h), None);
        });
        assert_eq!(value(&s, h), Some(5));
        // Not taken out at all: runs at once.
        let s2 = s.clone();
        s.run_now_or_defer(h, move || s2.with::<Thing, _>(h, |t| t.value = 6).unwrap());
        assert_eq!(value(&s, h), Some(6));
    }

    #[test]
    fn run_now_reaches_free_and_lent_objects_only() {
        let s = store();
        let h = s.insert(Box::new(Thing { value: 1 }));
        let s2 = s.clone();
        assert_eq!(s.run_now(h, || s2.with::<Thing, _>(h, |t| t.value)), Some(Some(1)), "in its slot");
        let s2 = s.clone();
        s.dispatch(h, std::ptr::null_mut(), |ent, _| {
            let me = (ent as &mut dyn Any).downcast_mut::<Thing>().unwrap();
            assert_eq!(s2.run_now(h, || 9), None, "taken out and not lent: nothing runs");
            s2.lend(Some(h), me, || {
                let s3 = s2.clone();
                assert_eq!(s2.run_now(h, || s3.with::<Thing, _>(h, |t| std::mem::replace(&mut t.value, 3))), Some(Some(1)));
                assert!(s2.is_taken_out(h), "closed again");
                // Already open (nested): stays open for the outer call.
                let s3 = s2.clone();
                s2.run_now_or_defer(h, move || {
                    let s4 = s3.clone();
                    assert_eq!(s3.run_now(h, || s4.with::<Thing, _>(h, |t| t.value)), Some(Some(3)));
                    assert_eq!(s3.with::<Thing, _>(h, |t| t.value), Some(3), "still open");
                });
            });
            assert_eq!(me.value, 3);
        });
        assert_eq!(value(&s, h), Some(3));
    }

    #[test]
    fn freed_while_lent_is_released_once_by_the_dispatch() {
        let s = store();
        let h = s.insert(Box::new(Thing { value: 1 }));
        let s2 = s.clone();
        s.dispatch(h, std::ptr::null_mut(), |ent, _| {
            let me = (ent as &mut dyn Any).downcast_mut::<Thing>().unwrap();
            s2.lend(Some(h), me, || {
                let s3 = s2.clone();
                s2.run_now_or_defer(h, move || s3.free(h));
                assert!(s2.world.borrow().free.is_empty(), "not released while the dispatch is out");
            });
            assert_eq!(me.value, 0, "a freed object isn't handed back");
        });
        assert_eq!(value(&s, h), None);
        assert_eq!(s.world.borrow().free, vec![h.index], "released exactly once");
        let h2 = s.insert(Box::new(Thing { value: 9 }));
        assert_eq!(h2.index, h.index);
        assert_eq!(value(&s, h2), Some(9));
    }

    #[test]
    fn with_lent_reads_a_lent_object_only() {
        let s = store();
        let h = s.insert(Box::new(Thing { value: 4 }));
        let read = |s: &Store| s.with_lent::<Thing, _>(h, |t| t.value);
        assert_eq!(read(&s), Some(4), "in its slot");
        let s2 = s.clone();
        s.dispatch(h, std::ptr::null_mut(), |ent, _| {
            let me = (ent as &mut dyn Any).downcast_mut::<Thing>().unwrap();
            assert_eq!(read(&s2), None, "taken out and not lent");
            me.value = 5;
            s2.lend(Some(h), me, || {
                assert_eq!(read(&s2), Some(5), "lent back");
                assert_eq!(value(&s2, h), None, "still closed to `with`");
            });
        });
        assert_eq!(read(&s), Some(5));
    }

    /// The `game_player_hurt` / `player_weaponstrip` shape: a player's +use (its dispatch, lending
    /// it) fires another entity, whose `Use` changes the player before it goes on, as TS's direct
    /// `TakeDamage` / `RemoveAllItems` calls do. The scoring shape: inside a third dispatch (the
    /// victim's `Killed`) `run_now` reaches the lent killer for the rest of the call.
    #[test]
    fn a_lent_player_is_changed_inside_another_entitys_call() {
        let s = store();
        let player = s.insert(Box::new(Thing { value: 100 }));
        let rule = s.insert(Box::new(Thing { value: 0 }));
        let victim = s.insert(Box::new(Thing { value: 0 }));
        let log = Rc::new(RefCell::new(Vec::new()));
        let (s2, log2) = (s.clone(), log.clone());
        s.dispatch(player, std::ptr::null_mut(), |ent, _| {
            let me = (ent as &mut dyn Any).downcast_mut::<Thing>().unwrap();
            s2.lend(Some(player), me, || {
                let (s3, log3) = (s2.clone(), log2.clone());
                s2.dispatch(rule, std::ptr::null_mut(), |_, _| {
                    let (s4, log4) = (s3.clone(), log3.clone());
                    s3.run_now_or_defer(player, move || {
                        s4.with::<Thing, _>(player, |t| t.value -= 30).unwrap();
                        log4.borrow_mut().push("hurt");
                    });
                    log3.borrow_mut().push("targets");
                    let s4 = s3.clone();
                    s3.dispatch(victim, std::ptr::null_mut(), |_, _| {
                        let s5 = s4.clone();
                        let scored = s4.run_now(player, || s5.with::<Thing, _>(player, |t| t.value += 1));
                        assert_eq!(scored, Some(Some(())), "the lent killer is reached inside the victim's call");
                    });
                });
            });
            assert_eq!(me.value, 71, "both changes came back with the lent object");
        });
        assert_eq!(*log.borrow(), vec!["hurt", "targets"], "the damage lands before the targets fire");
        assert_eq!(value(&s, player), Some(71));
    }

    #[test]
    fn lend_without_dispatch_just_runs() {
        let s = store();
        let h = s.insert(Box::new(Thing { value: 1 }));
        let mut other = Thing { value: 3 };
        assert_eq!(s.lend(Some(h), &mut other, || 7), 7);
        assert_eq!(other.value, 3);
        assert_eq!(value(&s, h), Some(1));
        assert_eq!(s.lend(None, &mut other, || 8), 8);
    }
}
