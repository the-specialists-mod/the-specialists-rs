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

/// `bits_SOUND_COMBAT`.
const SOUND_COMBAT: i32 = 1 << 0;
/// `bits_SOUND_PLAYER`.
const SOUND_PLAYER: i32 = 1 << 2;

/// A C `(int)` cast on the x87 stack: truncation, and the "integer indefinite" for NaN or out of
/// range.
fn x87_int(f: f64) -> i32 {
    if (-2_147_483_648.0..2_147_483_648.0).contains(&f) { f as i32 } else { i32::MIN }
}

/// What `CBasePlayer::UpdatePlayerSound` reads from the player.
#[derive(Clone, Copy, Debug, Default)]
pub struct PlayerSoundInput {
    pub origin: Vec3,
    pub velocity: Vec3,
    pub on_ground: bool,
    /// `pev->button & IN_JUMP`.
    pub jumping: bool,
    /// `m_fNoPlayerSound` (impulse 105).
    pub silent: bool,
    pub frametime: f32,
}

/// The sound part of `CBasePlayer::UpdatePlayerSound` (0xda630) on the client's reserved slot:
/// the body volume (speed on the ground, at most 512, +100 while jumping) or the louder fading
/// `m_iWeaponVolume`; the slot's volume jumps up to that target and fades 250/s below it (0 once
/// under it). `weapon_volume` is `m_iWeaponVolume` (pl+0x7f8), which nothing else in TS writes,
/// so it only ever runs down from 0. TS's `m_iExtraSoundTypes` and `m_iWeaponFlash` are never set
/// either, so they add nothing.
/// Returns `m_iTargetVolume` (pl+0x7f4), which the player keeps for its save data.
fn update_player_slot(s: &mut Sound, weapon_volume: &mut i32, inp: &PlayerSoundInput) -> i32 {
    s.kind = 0;
    let mut body = 0;
    if inp.on_ground {
        let v = inp.velocity.map(f64::from);
        body = x87_int((v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()).min(512);
    }
    if inp.jumping {
        body += 100;
    }
    let target = if body < *weapon_volume {
        s.kind |= SOUND_COMBAT;
        *weapon_volume
    } else {
        body
    };
    let fade = inp.frametime as f64 * 250.0;
    *weapon_volume = x87_int(*weapon_volume as f32 as f64 - fade);
    let mut volume = s.volume;
    if volume < target {
        volume = target;
    } else if target < volume {
        let v = x87_int(volume as f32 as f64 - fade);
        volume = if v < target { 0 } else { v };
    }
    s.origin = inp.origin;
    s.kind |= SOUND_PLAYER;
    s.volume = if inp.silent { 0 } else { volume };
    target
}

/// `CSoundEnt::ClientSoundIndex` + `SoundPointerForIndex` (0xe26f8, 0xe2678): the world
/// soundent's slot for player `index` (1-based), or the console note TS prints without one.
fn with_client_sound<R>(index: i32, f: impl FnOnce(&mut Sound) -> R) -> Result<R, &'static str> {
    let e = SOUND_ENT.0.get();
    let slot = index - 1;
    let lost = "Client lost reserved sound!\n";
    if e.is_null() || e.is_free() {
        return Err(lost);
    }
    if slot < 0 {
        return Err("SoundPointerForIndex() - Index < 0!\n");
    }
    if slot >= MAX_WORLD_SOUNDS as i32 {
        return Err("SoundPointerForIndex() - Index too large!\n");
    }
    world::with::<SoundEnt, _>(e, |s| f(&mut s.list.pool[slot as usize])).ok_or(lost)
}

/// `CBasePlayer::UpdatePlayerSound` (0xda630), from the end of a live player's `PostThink`.
/// `target_volume` (`m_iTargetVolume`) is only written when the player's slot exists.
pub fn update_player_sound(index: i32, weapon_volume: &mut i32, target_volume: &mut i32, inp: &PlayerSoundInput) {
    let eng = engine();
    match with_client_sound(index, |s| update_player_slot(s, weapon_volume, inp)) {
        Ok(t) => *target_volume = t,
        Err(note) => {
            // A bad index prints its own note before the lost-sound one.
            if !note.starts_with("Client") {
                eng.alert(ALERT_TYPE::at_console, note);
            }
            eng.alert(ALERT_TYPE::at_console, "Client lost reserved sound!\n");
        }
    }
}

/// `CSound::Reset` (0xe20d8) on the player's slot, from `CBasePlayer::Killed` (0x8f1f0): origin,
/// type and volume cleared and the `next` link set to `SOUND_LIST_EMPTY`, which cuts the active
/// list after this slot, as in TS.
pub fn reset_player_sound(index: i32) {
    if let Err(note) = with_client_sound(index, reset_sound)
        && !note.starts_with("Client")
    {
        engine().alert(ALERT_TYPE::at_console, note);
    }
}

fn reset_sound(s: &mut Sound) {
    s.origin = [0.0; 3];
    s.kind = 0;
    s.volume = 0;
    s.next = LIST_EMPTY;
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

    #[test]
    fn player_sound_follows_speed_and_silence() {
        let mut s = Sound::CLEAR;
        let mut weapon = 0;
        let run = PlayerSoundInput {
            origin: [1.0, 2.0, 3.0],
            velocity: [300.0, 400.0, 0.0],
            on_ground: true,
            frametime: 0.125,
            ..Default::default()
        };
        update_player_slot(&mut s, &mut weapon, &run);
        assert_eq!((s.volume, s.kind, s.origin), (500, SOUND_PLAYER, [1.0, 2.0, 3.0]));
        // `m_iWeaponVolume` only runs down from 0, so it never wins.
        assert_eq!(weapon, -31);
        // Faster than 512 caps; a jump adds 100.
        update_player_slot(&mut s, &mut weapon, &PlayerSoundInput { velocity: [900.0, 0.0, 0.0], jumping: true, ..run });
        assert_eq!(s.volume, 612);
        // Standing still: the volume fades 250/s towards the target (612 - 31.25, truncated).
        let still = PlayerSoundInput { velocity: [0.0; 3], ..run };
        update_player_slot(&mut s, &mut weapon, &still);
        assert_eq!(s.volume, 580);
        // A target (560) just under the volume: below it after the fade means 0.
        update_player_slot(&mut s, &mut weapon, &PlayerSoundInput { velocity: [460.0, 0.0, 0.0], jumping: true, ..run });
        assert_eq!(s.volume, 0);
        // Impulse 105: the slot is still updated, at volume 0.
        update_player_slot(&mut s, &mut weapon, &PlayerSoundInput { silent: true, ..run });
        assert_eq!((s.volume, s.kind), (0, SOUND_PLAYER));
        // Killed's `CSound::Reset` cuts the list after the slot.
        s.next = 5;
        reset_sound(&mut s);
        assert_eq!((s.volume, s.kind, s.origin, s.next), (0, 0, [0.0; 3], LIST_EMPTY));
    }
}
