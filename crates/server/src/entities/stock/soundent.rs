//! `soundent` (`CSoundEnt`, SDK soundent.cpp, unchanged in TS): the AI's list of world sounds.
//! TS has no monsters, so nothing reads the list; it only shows in `displaysoundlist 1`'s
//! developer messages and in the edict `CWorld::Precache` creates for it every map (before the
//! `bodyque` ring and every map entity, which keeps the port's edict numbers TS's).

use crate::world::{self, Cx, Entity};
use goldsrc::Global;
use goldsrc::edict::EdictPtr;
use goldsrc::ext::entities::Vec3;
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::cell::Cell;

/// `MAX_WORLD_SOUNDS`.
pub const MAX_WORLD_SOUNDS: usize = 64;
/// `bits_SOUND_MEAT` (gibs landing).
pub const SOUND_MEAT: i32 = 1 << 4;
/// `SOUND_NEVER_EXPIRE`: the per-client reserved slots.
const SOUND_NEVER_EXPIRE: f32 = -1.0;
/// `SOUND_LIST_EMPTY`.
const LIST_EMPTY: i32 = -1;

/// `pSoundEnt`: the soundent `CWorld::Precache` created for this map.
static SOUND_ENT: Global<Cell<*mut edict_t>> = Global(Cell::new(std::ptr::null_mut()));

/// `CSound`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Sound {
    origin: Vec3,
    kind: i32,
    volume: i32,
    expire: f32,
    next: i32,
}

impl Sound {
    /// `CSound::Clear` (0xe208c).
    const CLEAR: Sound = Sound { origin: [0.0; 3], kind: 0, volume: 0, expire: 0.0, next: LIST_EMPTY };
}

/// The sound pool and its free and active lists (`m_iFreeSound`, `m_iActiveSound`).
#[derive(Clone, Debug, PartialEq)]
struct SoundList {
    free: i32,
    active: i32,
    pool: [Sound; MAX_WORLD_SOUNDS],
}

impl Default for SoundList {
    fn default() -> Self {
        SoundList { free: 0, active: LIST_EMPTY, pool: [Sound::CLEAR; MAX_WORLD_SOUNDS] }
    }
}

impl SoundList {
    /// The list half of `CSoundEnt::Initialize` (0xe2444): every sound cleared and free.
    fn reset(&mut self) {
        self.free = 0;
        self.active = LIST_EMPTY;
        for (i, s) in self.pool.iter_mut().enumerate() {
            *s = Sound::CLEAR;
            s.next = i as i32 + 1;
        }
        self.pool[MAX_WORLD_SOUNDS - 1].next = LIST_EMPTY;
    }

    /// `CSoundEnt::IAllocSound` (0xe232c): moves the first free sound to the head of the active
    /// list; `None` (after "Free Sound List is full!") when none is free.
    fn alloc(&mut self) -> Option<usize> {
        if self.free == LIST_EMPTY {
            engine().alert(ALERT_TYPE::at_console, "Free Sound List is full!\n");
            return None;
        }
        let i = self.free as usize;
        self.free = self.pool[i].next;
        self.pool[i].next = self.active;
        self.active = i as i32;
        Some(i)
    }

    /// `CSoundEnt::FreeSound` (0xe22ac): unlinks `i` (after `prev`, or the head) into the free
    /// list.
    fn free_sound(&mut self, i: usize, prev: i32) {
        if prev == LIST_EMPTY {
            self.active = self.pool[i].next;
        } else {
            self.pool[prev as usize].next = self.pool[i].next;
        }
        self.pool[i].next = self.free;
        self.free = i as i32;
    }

    /// The expiry walk of `CSoundEnt::Think` (0xe2188).
    fn expire(&mut self, now: f32) {
        let mut prev = LIST_EMPTY;
        let mut i = self.active;
        while i != LIST_EMPTY {
            let s = self.pool[i as usize];
            if now < s.expire || s.expire == SOUND_NEVER_EXPIRE {
                prev = i;
                i = s.next;
            } else {
                self.free_sound(i as usize, prev);
                i = s.next;
            }
        }
    }

    /// `CSoundEnt::ISoundsInList` (0xe259c).
    fn count(&self, head: i32) -> i32 {
        let mut n = 0;
        let mut i = head;
        while i != LIST_EMPTY {
            n += 1;
            i = self.pool[i as usize].next;
        }
        n
    }
}

/// `soundent` (`CSoundEnt`).
#[derive(Default)]
pub struct SoundEnt {
    list: SoundList,
    /// `m_cLastActiveSounds`.
    last_active: i32,
    /// `m_fShowReport` (`displaysoundlist 1`).
    show_report: bool,
}

impl SoundEnt {
    /// `CSoundEnt::Initialize` (0xe2444). The client reserve is taken from `pSoundEnt` (the
    /// world's soundent), which is this one except for a second, map-placed soundent.
    fn initialize(&mut self, me: *mut edict_t) {
        self.list.reset();
        let eng = engine();
        let global = SOUND_ENT.0.get();
        for _ in 0..eng.globals().maxClients {
            let reserved = if global == me {
                reserve(&mut self.list)
            } else {
                world::with::<SoundEnt, _>(global, |g| reserve(&mut g.list)).unwrap_or(false)
            };
            if !reserved {
                eng.alert(ALERT_TYPE::at_console, "Could not AllocSound() for Client Reserve! (DLL)\n");
                return;
            }
        }
        self.show_report = eng.cvar_float(c"displaysoundlist") == 1.0;
    }
}

/// One never-expiring client slot of the reserve.
fn reserve(list: &mut SoundList) -> bool {
    match list.alloc() {
        Some(i) => {
            list.pool[i].expire = SOUND_NEVER_EXPIRE;
            true
        }
        None => false,
    }
}

impl Entity for SoundEnt {
    /// `CSoundEnt::Spawn` (0xe2140).
    fn spawn(&mut self, cx: &mut Cx) {
        cx.pev().solid = SOLID_NOT as i32;
        self.initialize(cx.edict);
        cx.pev().nextthink = engine().time() + 1.0;
    }

    /// `CSoundEnt::Think` (0xe2188). TS frees through `pSoundEnt` while walking this soundent's
    /// list; only the world's soundent ever has active sounds, so that is the same list.
    fn think(&mut self, cx: &mut Cx) {
        let eng = engine();
        let now = eng.time();
        cx.pev().nextthink = now + 0.3;
        self.list.expire(now);
        if self.show_report {
            let active = self.list.count(self.list.active);
            let free = self.list.count(self.list.free);
            eng.alert(ALERT_TYPE::at_aiconsole, &format!("Soundlist: {} / {}  ({})\n", active, free, active - self.last_active));
            self.last_active = active;
        }
    }
}

/// `CWorld::Precache` (0xf449c): `pSoundEnt = GetClassPtr<CSoundEnt>(NULL)` (a new edict with
/// no classname), then its `Spawn`.
pub fn create_world_soundent() {
    let e = engine().create_entity();
    SOUND_ENT.0.set(e);
    if e.is_null() {
        return;
    }
    world::create::<SoundEnt>(e.v());
    world::dispatch(e, |ent, cx| ent.spawn(cx));
}

/// `CSoundEnt::InsertSound` (0xe238c): a sound of `kind` and `volume` at `origin` for `duration`
/// seconds in the world's soundent.
pub fn insert_sound(kind: i32, origin: Vec3, volume: i32, duration: f32) {
    let e = SOUND_ENT.0.get();
    if e.is_null() || e.is_free() {
        return;
    }
    let now = engine().time();
    let added = world::with::<SoundEnt, _>(e, |s| {
        let Some(i) = s.list.alloc() else { return false };
        let snd = &mut s.list.pool[i];
        snd.origin = origin;
        snd.kind = kind;
        snd.volume = volume;
        snd.expire = now + duration;
        true
    });
    if added == Some(false) {
        engine().alert(ALERT_TYPE::at_console, "Could not AllocSound() for InsertSound() (DLL)\n");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> SoundList {
        let mut l = SoundList::default();
        l.reset();
        l
    }

    fn take(l: &mut SoundList) -> usize {
        // alloc() without the engine's "full" message (tests never fill the pool).
        assert_ne!(l.free, LIST_EMPTY);
        l.alloc().unwrap()
    }

    #[test]
    fn alloc_moves_free_head_to_active_head() {
        let mut l = fresh();
        assert_eq!((l.count(l.free), l.count(l.active)), (64, 0));
        assert_eq!(take(&mut l), 0);
        assert_eq!(take(&mut l), 1);
        assert_eq!(l.active, 1);
        assert_eq!(l.pool[1].next, 0);
        assert_eq!((l.count(l.free), l.count(l.active)), (62, 2));
    }

    #[test]
    fn expiry_keeps_reserved_and_live_sounds() {
        let mut l = fresh();
        let a = take(&mut l);
        l.pool[a].expire = SOUND_NEVER_EXPIRE;
        let b = take(&mut l);
        l.pool[b].expire = 5.0;
        let c = take(&mut l);
        l.pool[c].expire = 20.0;
        l.expire(10.0);
        // b expired and went back to the head of the free list.
        assert_eq!(l.count(l.active), 2);
        assert_eq!(l.free, b as i32);
        assert_eq!(l.active, c as i32);
        assert_eq!(l.pool[c].next, a as i32);
        l.expire(30.0);
        assert_eq!(l.count(l.active), 1);
        assert_eq!(l.active, a as i32);
    }
}
