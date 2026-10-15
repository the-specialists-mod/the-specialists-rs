//! Safe access to engine edicts through the `*mut edict_t` handles the engine hands out.
//!
//! Game code passes raw edict pointers around as entity handles (they're `Copy`, and the engine
//! gives and takes them everywhere). Holding a raw pointer is safe; dereferencing it isn't. This
//! module is the one place that dereferences them, so the game crates can stay free of `unsafe`.
//!
//! # Contract
//! The engine owns every edict in one array that lives as long as the server, and it runs every
//! game callback on one thread. So for an edict pointer the engine gave us:
//! * it stays valid (the slot is reused, never unmapped), which is why [`EdictPtr::v`] can hand
//!   out a `'static` reference;
//! * the reference must not be held across a call that may re-enter the game for the same
//!   edict (the entity `Cx`/dispatch rules in `ts-server`'s `world.rs` already require that).
//!
//! These accessors panic on a null pointer instead of reading address 0.

use crate::sys::*;

/// Accessors on an engine edict pointer.
pub trait EdictPtr: Copy {
    fn as_mut_ptr(self) -> *mut edict_t;

    /// The edict's `entvars_t` (`pev`). See the module contract.
    fn v(self) -> &'static mut entvars_t {
        let e = self.as_mut_ptr();
        assert!(!e.is_null(), "null edict");
        // SAFETY: non-null edicts from the engine point into its edict array, which outlives
        // every game callback; single-threaded access per the module contract.
        unsafe { &mut (*e).v }
    }

    /// The edict slot is free (not in use by any entity).
    fn is_free(self) -> bool {
        let e = self.as_mut_ptr();
        assert!(!e.is_null(), "null edict");
        // SAFETY: as in `v`.
        unsafe { (*e).free != 0 }
    }

    /// The engine's reuse counter for the slot.
    fn serial_number(self) -> i32 {
        let e = self.as_mut_ptr();
        assert!(!e.is_null(), "null edict");
        // SAFETY: as in `v`.
        unsafe { (*e).serialnumber }
    }

    /// True if the game allocated private data for this edict (it's game-owned).
    fn has_private_data(self) -> bool {
        let e = self.as_mut_ptr();
        // SAFETY: as in `v`.
        !e.is_null() && unsafe { !(*e).pvPrivateData.is_null() }
    }
}

impl EdictPtr for *mut edict_t {
    fn as_mut_ptr(self) -> *mut edict_t {
        self
    }
}

impl EdictPtr for *const edict_t {
    fn as_mut_ptr(self) -> *mut edict_t {
        self.cast_mut()
    }
}

/// The 8-byte tag the game stores in an edict's private data, at [`PRIVATE_TAG_OFFSET`] (the
/// server keeps a generational slab handle there). Only the game allocates private data, and
/// only through [`set_private_tag`], so every non-null private block holds one of these.
pub type PrivateTag = [u32; 2];

/// Where the tag sits in the private block. Not at 0: a block that server plugins take for a C++
/// object must not offer them a vtable pointer (see [`MIN_PRIVATE_SIZE`]).
pub const PRIVATE_TAG_OFFSET: usize = 8;

/// The smallest private block, zeroed past the tag.
///
/// AMX Mod X's hamsandwich hooks a class through the vtable pointer in an entity's private data,
/// which its gamedata for TS puts at offset 0 on Windows and 0x60 on Linux (GCC 2.95's layout).
/// The port's private data is no C++ object, so the platform's vtable word stays zero and
/// `RegisterHam` fails cleanly ("Failed to retrieve vtable") instead of following the tag as a
/// pointer (the other platform's word may hold [`PEV_OFFSET`]'s `pev`: Linux's at 0). 0x80 covers
/// the Linux slot, 8 bytes wide on 64-bit builds.
pub const MIN_PRIVATE_SIZE: usize = 0x80;

const _: () = assert!(PRIVATE_TAG_OFFSET >= 8 && PRIVATE_TAG_OFFSET + size_of::<PrivateTag>() <= 0x60);
const _: () = assert!(MIN_PRIVATE_SIZE >= 0x60 + 8);

/// Allocates `size` bytes of private data for the edict (`pfnPvAllocEntPrivateData`, at least
/// [`MIN_PRIVATE_SIZE`]) and stores `tag` at [`PRIVATE_TAG_OFFSET`].
///
/// The rest of the block is padding nothing in the game reads: the engines zero it (FWGS
/// `Mem_Calloc`, ReHLDS `Mem_Calloc`, HLDS `calloc`), and server plugins that write TS 3.0
/// `CBasePlayer` / `CTSGun` offsets through fakemeta's `set_pdata_*` land there instead of past
/// the end of the block (`ts-server`'s `world::private_size`).
#[cfg(feature = "server")]
pub fn set_private_tag(e: *mut edict_t, tag: PrivateTag, size: usize) {
    let size = size.max(MIN_PRIVATE_SIZE);
    let p = crate::eng!(pfnPvAllocEntPrivateData(e, size as std::ffi::c_long));
    assert!(!p.is_null(), "pfnPvAllocEntPrivateData returned null");
    // SAFETY: the engine just allocated at least `size` >= MIN_PRIVATE_SIZE bytes for us, which
    // holds the tag at PRIVATE_TAG_OFFSET.
    unsafe { p.cast::<u8>().add(PRIVATE_TAG_OFFSET).cast::<PrivateTag>().write_unaligned(tag) };
    sizes().insert(p as usize, size);
    if let Some(off) = PEV_OFFSET {
        // SAFETY: `e` is the engine's edict the block was just allocated for (non-null: the
        // engine dereferenced it), and `&raw mut (*e).v` only computes the field's address.
        let pev = unsafe { &raw mut (*e).v } as usize as u32;
        write_private_u32(e, off, pev);
    }
}

/// Where TS 3.0's `CBaseEntity::pev` sits in an object (AMX Mod X's TS gamedata, "pev": 4 on
/// Windows, 0 on Linux, where GCC 2.95 put the vtable at 0x60), written into every private block
/// on 32-bit builds so plugins that turn a private-data pointer into an entity (fakemeta's
/// `get_pdata_ent`) find it. `None` on 64-bit builds: no AMX Mod X there, and a pointer at 8
/// would hit the tag.
pub const PEV_OFFSET: Option<usize> = if cfg!(target_pointer_width = "32") { if cfg!(windows) { Some(4) } else { Some(0) } } else { None };

/// The size [`set_private_tag`] allocated, by block address (port addition, for the private-data
/// mirror's bounds).
fn sizes() -> std::sync::MutexGuard<'static, std::collections::BTreeMap<usize, usize>> {
    static SIZES: std::sync::Mutex<std::collections::BTreeMap<usize, usize>> = std::sync::Mutex::new(std::collections::BTreeMap::new());
    SIZES.lock().unwrap_or_else(|e| e.into_inner())
}

/// The engine is about to free the edict's private data (`pfnOnFreeEntPrivateData`).
pub fn forget_private(e: *mut edict_t) {
    if e.has_private_data() {
        sizes().remove(&private_address(e));
    }
}

/// The address of the edict's private data, 0 for none: what a TS object pointer to this entity
/// (`m_pActiveItem`, `m_pPlayer`) holds.
pub fn private_address(e: *const edict_t) -> usize {
    if !e.has_private_data() {
        return 0;
    }
    // SAFETY: as in `EdictPtr::v`.
    unsafe { (*e).pvPrivateData as usize }
}

/// Bytes of the edict's private data: what [`set_private_tag`] allocated, 0 for none (or a block
/// the game didn't allocate, which never happens: see [`PrivateTag`]).
pub fn private_size(e: *const edict_t) -> usize {
    let a = private_address(e);
    if a == 0 { 0 } else { sizes().get(&a).copied().unwrap_or(0) }
}

/// The 32-bit word at byte `offset` of the edict's private data; `None` outside the block or on
/// the tag.
pub fn read_private_u32(e: *const edict_t, offset: usize) -> Option<u32> {
    with_private_words(e, |w| w.read(offset)).flatten()
}

/// Writes the 32-bit word at byte `offset` of the edict's private data. False (nothing written)
/// outside the block or on the tag.
pub fn write_private_u32(e: *const edict_t, offset: usize, value: u32) -> bool {
    with_private_words(e, |w| w.write(offset, value)).unwrap_or(false)
}

/// The edict's private data as 32-bit words for the length of a call, bounded by the block's size
/// (looked up once) and clear of the tag. See [`with_private_words`].
pub struct PrivateWords {
    base: *mut u8,
    size: usize,
}

impl PrivateWords {
    fn ok(&self, offset: usize) -> bool {
        let end = offset.saturating_add(4);
        end <= self.size && (end <= PRIVATE_TAG_OFFSET || offset >= PRIVATE_TAG_OFFSET + size_of::<PrivateTag>())
    }

    /// The word at byte `offset`; `None` outside the block or on the tag.
    pub fn read(&self, offset: usize) -> Option<u32> {
        if !self.ok(offset) {
            return None;
        }
        // SAFETY: `offset + 4 <= size`, the block `set_private_tag` allocated, which lives while
        // the edict holds it (the module contract: no engine call that could free it runs inside
        // `with_private_words`); unaligned access, single-threaded.
        Some(unsafe { self.base.add(offset).cast::<u32>().read_unaligned() })
    }

    /// Writes the word at byte `offset`. False (nothing written) outside the block or on the tag.
    pub fn write(&mut self, offset: usize, value: u32) -> bool {
        if !self.ok(offset) {
            return false;
        }
        // SAFETY: as in `read`; the tag is never overwritten.
        unsafe { self.base.add(offset).cast::<u32>().write_unaligned(value) };
        true
    }
}

/// Runs `f` on the edict's private data as words: `None` without game private data. Don't call
/// the engine inside `f` (it could free the block).
pub fn with_private_words<R>(e: *const edict_t, f: impl FnOnce(&mut PrivateWords) -> R) -> Option<R> {
    let size = private_size(e);
    if size == 0 {
        return None;
    }
    // SAFETY: as in `EdictPtr::v`; `private_size` found the block, so `pvPrivateData` is set.
    let base = unsafe { (*e).pvPrivateData.cast::<u8>() };
    Some(f(&mut PrivateWords { base, size }))
}

/// The tag in the edict's private data, if the game allocated any (see [`set_private_tag`]).
pub fn private_tag(e: *const edict_t) -> Option<PrivateTag> {
    if !e.has_private_data() {
        return None;
    }
    // SAFETY: private data is only ever allocated by `set_private_tag`: at least MIN_PRIVATE_SIZE
    // bytes, with a tag at PRIVATE_TAG_OFFSET.
    Some(unsafe { (*e).pvPrivateData.cast::<u8>().add(PRIVATE_TAG_OFFSET).cast::<PrivateTag>().read_unaligned() })
}
