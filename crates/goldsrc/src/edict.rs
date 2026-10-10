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

/// The 8-byte tag the game stores in an edict's private data (the server keeps a generational
/// slab handle there). Only the game writes private data, and only through [`set_private_tag`],
/// so every non-null private block holds one of these.
pub type PrivateTag = [u32; 2];

/// Allocates the edict's private data (`pfnPvAllocEntPrivateData`) and stores `tag` in it.
#[cfg(feature = "server")]
pub fn set_private_tag(e: *mut edict_t, tag: PrivateTag) {
    let p = crate::eng!(pfnPvAllocEntPrivateData(e, size_of::<PrivateTag>() as std::ffi::c_long));
    assert!(!p.is_null(), "pfnPvAllocEntPrivateData returned null");
    // SAFETY: the engine just allocated at least size_of::<PrivateTag>() bytes for us.
    unsafe { p.cast::<PrivateTag>().write_unaligned(tag) };
}

/// The tag in the edict's private data, if the game allocated any (see [`set_private_tag`]).
pub fn private_tag(e: *const edict_t) -> Option<PrivateTag> {
    if !e.has_private_data() {
        return None;
    }
    // SAFETY: private data is only ever allocated by `set_private_tag`, which writes a tag.
    Some(unsafe { (*e).pvPrivateData.cast::<PrivateTag>().read_unaligned() })
}
