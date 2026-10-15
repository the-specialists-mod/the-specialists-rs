//! The private-data mirror (port addition, plugin API version 5; tsmod-amxx-rs#10,
//! docs/implementation/plugin-api.md §4).
//!
//! TS 3.0's entities were C++ objects, and AMX Mod X plugins read and wrote their members at fixed
//! offsets (fakemeta's `get_pdata_*` / `set_pdata_*`, hamsandwich's `get_pdata_cbase`). The port's
//! private data is a zeroed block holding a slab tag (`goldsrc::edict`), so those reads gave 0. This
//! module writes the members plugins use into the block at TS's offsets, for the build's platform
//! ([`Layout::HOST`]: `ts_i386.so`'s on Linux, `mp.dll`'s on Windows), and takes some plugin writes
//! back into the game:
//!
//! * a player's `m_pActiveItem` (its `weapon_tsgun`, or a `cycler_weapon`, else NULL);
//! * the `weapon_tsgun`'s `m_pPlayer`, `m_iId` (16), `m_iClip`, current weapon id, the reserve
//!   ammo per ammo type (`gun+0xd4`) and the 38 `s_weapon_status` records' owned / ammo type /
//!   fire mode, clip and attachment words (`gun+0x114`, weapons.md §4.1).
//!
//! Writes to the reserves, the records' clips and `m_iClip` (the current weapon's clip) are taken
//! back; writes to the other mirrored words are overwritten. Pointers (and `pev`, which
//! `goldsrc::edict::set_private_tag` writes) are only mirrored on 32-bit builds, where AMX Mod X
//! runs.
//!
//! The mirror is [`sync`]ed at the end of the player's PostThink, at every StartFrame and around
//! the API's setters: a plugin reads values up to a frame old, and its write takes effect at the
//! next sync, as the API's snapshots and deferred setters do.

use crate::player::Player;
use goldsrc::edict::EdictPtr;
use goldsrc::sys::edict_t;
use shared::weapons::{MAX_AMMO, MAX_WEAPONS};

/// Where TS 3.0 keeps the mirrored members, by platform (decomp bots.md §14.2.1 and §14.3.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    /// `mp.dll`'s offsets instead of `ts_i386.so`'s.
    pub windows: bool,
}

impl Layout {
    /// The layout of the platform this server is built for.
    pub const HOST: Layout = Layout { windows: cfg!(windows) };

    /// `CBasePlayer::m_pActiveItem`: pl+0xa10 in `ts_i386.so`, 0x9fc in `mp.dll` (int 639, and
    /// fakemeta's default Linux shift of 5 ints).
    pub fn active_item(self) -> usize {
        if self.windows { 0x9fc } else { 0xa10 }
    }

    /// A `CTSGun` member at `so` in `ts_i386.so`: `mp.dll` has every member above 0x54 0x10
    /// lower (its `CTSGun` is 0xa00 bytes, the .so's 0xa10).
    fn gun(self, so: usize) -> usize {
        if self.windows { so - 0x10 } else { so }
    }

    /// `m_pPlayer` (+0x80).
    pub fn gun_player(self) -> usize {
        self.gun(0x80)
    }
    /// `m_iId` (+0x88).
    pub fn gun_id(self) -> usize {
        self.gun(0x88)
    }
    /// `m_iClip` (+0xb4): the current weapon's clip.
    pub fn gun_clip(self) -> usize {
        self.gun(0xb4)
    }
    /// The current weapon id (+0xc8, 0 kung fu).
    pub fn gun_current(self) -> usize {
        self.gun(0xc8)
    }
    /// The reserve of ammo type `ammo` (+0xd4, an int per type).
    pub fn gun_reserve(self, ammo: usize) -> usize {
        self.gun(0xd4) + 4 * ammo
    }
    /// Weapon `id`'s `s_weapon_status` record (+0x114, 0x38 bytes each).
    pub fn gun_status(self, id: usize) -> usize {
        self.gun(0x114) + 0x38 * id
    }
}

/// What a mirrored word holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    /// `m_pPlayer`.
    Player,
    /// `m_iId`.
    Id,
    /// The current weapon id.
    Current,
    /// `m_iClip`; taken back as the current weapon's clip.
    Clip,
    /// A reserve; taken back.
    Reserve(u8),
    /// A record's first word: owned (byte 0), ammo type (1), fire mode (2).
    StatusHead(u8),
    /// A record's clip (+4); taken back for an owned weapon.
    StatusClip(u8),
    /// A record's owned (+0x10) and switched-on (+0x14) attachments.
    Items(u8),
    Active(u8),
}

/// The private blocks, as the mirror uses them (the engine's, or a test's).
pub trait Blocks {
    fn read(&self, e: *mut edict_t, offset: usize) -> Option<u32>;
    fn write(&mut self, e: *mut edict_t, offset: usize, value: u32);
    /// [`read`](Self::read) of each offset (one block lookup for them all, where that matters).
    fn read_words(&self, e: *mut edict_t, offsets: &[usize]) -> Vec<Option<u32>> {
        offsets.iter().map(|&o| self.read(e, o)).collect()
    }
    /// [`write`](Self::write) of each word.
    fn write_words(&mut self, e: *mut edict_t, words: &[(usize, Field, u32)]) {
        for &(o, _, v) in words {
            self.write(e, o, v);
        }
    }
    /// The block's address, 0 for none: a TS pointer to the entity.
    fn address(&self, e: *mut edict_t) -> usize;
    fn serial(&self, e: *mut edict_t) -> i32;
}

/// The engine's private blocks (`goldsrc::edict`).
struct Engine;

impl Blocks for Engine {
    fn read(&self, e: *mut edict_t, offset: usize) -> Option<u32> {
        goldsrc::edict::read_private_u32(e, offset)
    }
    fn write(&mut self, e: *mut edict_t, offset: usize, value: u32) {
        goldsrc::edict::write_private_u32(e, offset, value);
    }
    fn read_words(&self, e: *mut edict_t, offsets: &[usize]) -> Vec<Option<u32>> {
        goldsrc::edict::with_private_words(e, |w| offsets.iter().map(|&o| w.read(o)).collect()).unwrap_or_else(|| vec![None; offsets.len()])
    }
    fn write_words(&mut self, e: *mut edict_t, words: &[(usize, Field, u32)]) {
        goldsrc::edict::with_private_words(e, |w| {
            for &(o, _, v) in words {
                w.write(o, v);
            }
        });
    }
    fn address(&self, e: *mut edict_t) -> usize {
        goldsrc::edict::private_address(e)
    }
    fn serial(&self, e: *mut edict_t) -> i32 {
        e.serial_number()
    }
}

/// Pointers are mirrored where they fit TS's 32-bit words.
const POINTERS: bool = cfg!(target_pointer_width = "32");

/// What the mirror last wrote into a player's gun block, to tell a plugin's write from its own.
#[derive(Clone, Debug)]
pub struct Mirror {
    gun: *mut edict_t,
    serial: i32,
    words: Vec<(usize, Field, u32)>,
}

impl Default for Mirror {
    fn default() -> Self {
        Mirror { gun: std::ptr::null_mut(), serial: 0, words: Vec::new() }
    }
}

/// The gun block's words for player `p`, whose block is at `player_addr`.
pub fn gun_words(l: Layout, p: &Player, player_addr: usize) -> Vec<(usize, Field, u32)> {
    let g = &p.inv.gun;
    let mut w = Vec::with_capacity(4 + MAX_AMMO + 5 * MAX_WEAPONS);
    if POINTERS {
        w.push((l.gun_player(), Field::Player, player_addr as u32));
    }
    w.push((l.gun_id(), Field::Id, shared::protocol::weapons::TSGUN_ID as u32));
    w.push((l.gun_clip(), Field::Clip, g.clip_mirror as u32));
    w.push((l.gun_current(), Field::Current, g.current as u32));
    for (t, r) in g.reserve.iter().enumerate() {
        w.push((l.gun_reserve(t), Field::Reserve(t as u8), *r as u32));
    }
    for (id, s) in g.status.iter().enumerate() {
        let (base, id) = (l.gun_status(id), id as u8);
        w.push((base, Field::StatusHead(id), s.owned as u32 | (s.ammo as u32) << 8 | (s.mode as u32) << 16));
        w.push((base + 4, Field::StatusClip(id), s.clip as u32));
        w.push((base + 0x10, Field::Items(id), s.items));
        w.push((base + 0x14, Field::Active(id), s.active));
    }
    w
}

/// Applies a plugin's write of `value` to `field`. Clips are cut to 0..255 and reserves to
/// 0..32767, the ranges the HUD messages carry (and `set_weapon_ammo` takes).
pub fn take_back(p: &mut Player, field: Field, value: u32) {
    let v = value as i32;
    match field {
        Field::Reserve(t) => {
            let inv = &mut p.inv;
            if let Some(r) = inv.gun.reserve.get_mut(t as usize) {
                *r = v.clamp(0, 32767);
                inv.sv.next_update = 0.0;
            }
        }
        Field::StatusClip(id) => {
            crate::weapons::set_weapon_ammo(&mut p.inv, id, Some(v.clamp(0, 255)), None);
        }
        Field::Clip => {
            let id = p.inv.gun.current;
            if id != 0 {
                crate::weapons::set_weapon_ammo(&mut p.inv, id, Some(v.clamp(0, 255)), None);
            }
        }
        Field::Player | Field::Id | Field::Current | Field::StatusHead(_) | Field::Items(_) | Field::Active(_) => {}
    }
}

/// The player's `m_pActiveItem` entity (`weapons::active_item`), null for none.
fn active_item(p: &Player) -> *mut edict_t {
    match crate::weapons::active_item(p) {
        crate::weapons::Active::Gun => p.inv.sv.gun_item.get(),
        crate::weapons::Active::Cycler => p.inv.sv.cycler.get(),
        crate::weapons::Active::None => std::ptr::null_mut(),
    }
}

/// Takes the plugins' writes to player `e`'s gun block back, then writes the mirror anew.
pub fn sync(p: &mut Player, e: *mut edict_t) {
    let gun = p.inv.sv.gun_item.get();
    let active = active_item(p);
    sync_with(&mut Engine, Layout::HOST, p, e, gun, active);
}

/// [`sync`] on `blocks`, with the player's gun entity `gun` and active item `active` (null for
/// none).
pub fn sync_with(blocks: &mut impl Blocks, l: Layout, p: &mut Player, e: *mut edict_t, gun: *mut edict_t, active: *mut edict_t) {
    let last = std::mem::take(&mut p.pdata);
    if !gun.is_null() && last.gun == gun && last.serial == blocks.serial(gun) {
        let offsets: Vec<usize> = last.words.iter().map(|w| w.0).collect();
        for (&(_, field, was), now) in last.words.iter().zip(blocks.read_words(gun, &offsets)) {
            if let Some(now) = now
                && now != was
            {
                take_back(p, field, now);
            }
        }
    }
    if POINTERS {
        let addr = if active.is_null() { 0 } else { blocks.address(active) };
        blocks.write(e, l.active_item(), addr as u32);
    }
    if gun.is_null() {
        return;
    }
    let words = gun_words(l, p, blocks.address(e));
    blocks.write_words(gun, &words);
    p.pdata = Mirror { gun, serial: blocks.serial(gun), words };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Blocks in memory, by fake edict pointer.
    #[derive(Default)]
    struct Mem(HashMap<usize, Vec<u8>>);

    impl Mem {
        fn block(&mut self, e: *mut edict_t, size: usize) {
            self.0.insert(e as usize, vec![0; size]);
        }
        fn get(&self, e: *mut edict_t, off: usize) -> u32 {
            self.read(e, off).expect("in the block")
        }
        fn put(&mut self, e: *mut edict_t, off: usize, v: u32) {
            self.write(e, off, v);
        }
    }

    impl Blocks for Mem {
        fn read(&self, e: *mut edict_t, off: usize) -> Option<u32> {
            let b = self.0.get(&(e as usize))?.get(off..off + 4)?;
            Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        }
        fn write(&mut self, e: *mut edict_t, off: usize, v: u32) {
            if let Some(b) = self.0.get_mut(&(e as usize)).and_then(|b| b.get_mut(off..off + 4)) {
                b.copy_from_slice(&v.to_le_bytes());
            }
        }
        fn address(&self, e: *mut edict_t) -> usize {
            if self.0.contains_key(&(e as usize)) { 0x1000 + e as usize } else { 0 }
        }
        fn serial(&self, _e: *mut edict_t) -> i32 {
            1
        }
    }

    const PL: *mut edict_t = 0x10 as *mut edict_t;
    const GUN: *mut edict_t = 0x20 as *mut edict_t;

    fn setup() -> (Mem, Player) {
        let mut m = Mem::default();
        m.block(PL, crate::world::PLAYER_PRIVATE_SIZE);
        m.block(GUN, crate::world::TSGUN_PRIVATE_SIZE);
        let mut p = Player::default();
        let g = &mut p.inv.gun;
        g.status[13].owned = true;
        g.status[13].ammo = 6;
        g.status[13].mode = 1;
        g.status[13].clip = 30;
        g.status[13].items = 0x4;
        g.reserve[6] = 90;
        g.current = 13;
        g.clip_mirror = 30;
        (m, p)
    }

    /// The decomp's offsets (bots.md §14.2.1, §14.3.1): the .so's, and `mp.dll`'s 0x10 lower in
    /// the gun (0x14 in the player).
    #[test]
    fn layouts_are_ts_offsets() {
        let (so, dll) = (Layout { windows: false }, Layout { windows: true });
        assert_eq!((so.active_item(), dll.active_item()), (0xa10, 0x9fc));
        assert_eq!(dll.active_item() / 4, 639, "the int offset plugins pass");
        assert_eq!((so.gun_player(), dll.gun_player()), (0x80, 0x70));
        assert_eq!((so.gun_id(), dll.gun_id()), (0x88, 0x78));
        assert_eq!((so.gun_clip(), dll.gun_clip()), (0xb4, 0xa4));
        assert_eq!((so.gun_current(), dll.gun_current()), (0xc8, 0xb8));
        assert_eq!((so.gun_reserve(0), dll.gun_reserve(0)), (0xd4, 0xc4));
        assert_eq!((so.gun_status(0), dll.gun_status(0)), (0x114, 0x104));
        assert_eq!(so.gun_status(MAX_WEAPONS), 0x964, "the inventory's end (weapons.md §4.1)");
        // tsxaddon.inc's reserve offsets are mp.dll's: int 49 + ammo type (`tsweaponoffset`).
        assert_eq!(dll.gun_reserve(6) / 4, 55, "AK47");
        assert_eq!(dll.gun_clip() / 4, 41, "its \"special clip storage\"");
        for l in [so, dll] {
            assert!(l.gun_status(MAX_WEAPONS) <= crate::world::TSGUN_PRIVATE_SIZE);
            assert!(l.active_item() + 4 <= crate::world::PLAYER_PRIVATE_SIZE);
            assert!(l.gun_player() >= 16, "clear of the tag");
        }
    }

    #[test]
    fn sync_writes_the_mirror() {
        let l = Layout { windows: true };
        let (mut m, mut p) = setup();
        sync_with(&mut m, l, &mut p, PL, GUN, GUN);
        assert_eq!(m.get(GUN, l.gun_id()), 16);
        assert_eq!(m.get(GUN, l.gun_current()), 13);
        assert_eq!(m.get(GUN, l.gun_clip()), 30);
        assert_eq!(m.get(GUN, l.gun_reserve(6)), 90);
        assert_eq!(m.get(GUN, l.gun_status(13)), 1 | 6 << 8 | 1 << 16);
        assert_eq!(m.get(GUN, l.gun_status(13) + 4), 30);
        assert_eq!(m.get(GUN, l.gun_status(13) + 0x10), 4);
        assert_eq!(m.get(GUN, l.gun_status(12)), 0, "not owned");
        if POINTERS {
            assert_eq!(m.get(PL, l.active_item()) as usize, m.address(GUN));
            assert_eq!(m.get(GUN, l.gun_player()) as usize, m.address(PL));
        }
        // No active item: NULL.
        sync_with(&mut m, l, &mut p, PL, GUN, std::ptr::null_mut());
        assert_eq!(m.get(PL, l.active_item()), 0);
    }

    #[test]
    fn plugin_writes_come_back() {
        let l = Layout { windows: false };
        let (mut m, mut p) = setup();
        sync_with(&mut m, l, &mut p, PL, GUN, GUN);
        // tsxaddon-style writes: a reserve, a record's clip, m_iClip; and words that don't come back.
        m.put(GUN, l.gun_reserve(6), 250);
        m.put(GUN, l.gun_reserve(3), 40_000);
        m.put(GUN, l.gun_status(13) + 4, 12);
        m.put(GUN, l.gun_current(), 5);
        m.put(GUN, l.gun_status(5) + 4, 9);
        sync_with(&mut m, l, &mut p, PL, GUN, GUN);
        let g = &p.inv.gun;
        assert_eq!(g.reserve[6], 250);
        assert_eq!(g.reserve[3], 32767, "cut to WeaponInfo's short");
        assert_eq!(g.status[13].clip, 12);
        assert_eq!(g.clip_mirror, 12, "the gun in hand");
        assert_eq!(g.current, 13, "the weapon id isn't taken");
        assert_eq!(g.status[5].clip, 0, "a weapon the player doesn't own");
        assert_eq!(m.get(GUN, l.gun_current()), 13, "rewritten");
        assert_eq!(m.get(GUN, l.gun_reserve(3)), 32767);
        // m_iClip sets the current weapon's clip.
        m.put(GUN, l.gun_clip(), 7);
        sync_with(&mut m, l, &mut p, PL, GUN, GUN);
        assert_eq!(p.inv.gun.status[13].clip, 7);
        // Nothing written: nothing changes, even when the game changed the state meanwhile.
        p.inv.gun.reserve[6] = 10;
        sync_with(&mut m, l, &mut p, PL, GUN, GUN);
        assert_eq!(p.inv.gun.reserve[6], 10);
        assert_eq!(m.get(GUN, l.gun_reserve(6)), 10);
    }

    /// A new gun entity (another edict, or the slot reused) isn't compared with the last one's
    /// words.
    #[test]
    fn a_new_gun_starts_over() {
        let l = Layout { windows: false };
        let (mut m, mut p) = setup();
        sync_with(&mut m, l, &mut p, PL, GUN, GUN);
        let other = 0x30 as *mut edict_t;
        m.block(other, crate::world::TSGUN_PRIVATE_SIZE);
        m.put(other, l.gun_reserve(6), 1);
        sync_with(&mut m, l, &mut p, PL, other, other);
        assert_eq!(p.inv.gun.reserve[6], 90);
        assert_eq!(m.get(other, l.gun_reserve(6)), 90);
    }
}
