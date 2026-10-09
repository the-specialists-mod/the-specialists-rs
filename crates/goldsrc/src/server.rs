//! Server-side engine handle (`enginefuncs_t` + `globalvars_t`) and helpers.

use crate::sys::*;
use crate::util::{cstr, to_cstring};
use std::ffi::{CStr, c_int};
use std::sync::OnceLock;

pub struct Engine {
    pub f: enginefuncs_t,
    pub globals: *mut globalvars_t,
}
// The engine runs every game-DLL callback on one thread.
unsafe impl Send for Engine {}
unsafe impl Sync for Engine {}

static ENGINE: OnceLock<Engine> = OnceLock::new();

/// Called from the server library's `GiveFnptrsToDll` export.
///
/// # Safety
/// `funcs` must point to a valid engine function table.
pub unsafe fn init(funcs: *const enginefuncs_t, globals: *mut globalvars_t) {
    let _ = ENGINE.set(Engine { f: unsafe { *funcs }, globals });
}

pub fn engine() -> &'static Engine {
    ENGINE.get().expect("GiveFnptrsToDll not called yet")
}

/// Calls an engine slot, panicking with its name if the engine left it null.
#[macro_export]
macro_rules! eng {
    ($name:ident ( $($arg:expr),* $(,)? )) => {
        unsafe { ($crate::server::engine().f.$name.expect(stringify!($name)))($($arg),*) }
    };
}

impl Engine {
    pub fn globals(&self) -> &globalvars_t {
        unsafe { &*self.globals }
    }

    pub fn time(&self) -> f32 {
        self.globals().time
    }

    pub fn max_clients(&self) -> i32 {
        self.globals().maxClients
    }

    /// Prints to the server console (always shown, unlike `at_console` alerts).
    pub fn print(&self, msg: &str) {
        let s = to_cstring(msg);
        eng!(pfnServerPrint(s.as_ptr()));
    }

    pub fn alert(&self, kind: ALERT_TYPE, msg: &str) {
        let s = to_cstring(msg);
        // Variadic: format in Rust, pass through "%s".
        eng!(pfnAlertMessage(kind, c"%s".as_ptr().cast_mut(), s.as_ptr()));
    }

    /// Engine string pool lookup. `string_t` is an offset into the engine's pool.
    pub fn string(&self, s: string_t) -> &'static CStr {
        let p = eng!(pfnSzFromIndex(s));
        unsafe { cstr(p) }
    }

    /// Copies `s` into the engine string pool.
    pub fn alloc_string(&self, s: &CStr) -> string_t {
        eng!(pfnAllocString(s.as_ptr()))
    }

    /// The engine keeps the pointer, so the name must be `'static`.
    pub fn precache_model(&self, name: &'static CStr) -> i32 {
        eng!(pfnPrecacheModel(name.as_ptr()))
    }

    pub fn set_model(&self, e: *mut edict_t, model: &CStr) {
        eng!(pfnSetModel(e, model.as_ptr()))
    }

    pub fn set_size(&self, e: *mut edict_t, mins: [f32; 3], maxs: [f32; 3]) {
        eng!(pfnSetSize(e, mins.as_ptr(), maxs.as_ptr()))
    }

    pub fn set_origin(&self, e: *mut edict_t, org: [f32; 3]) {
        eng!(pfnSetOrigin(e, org.as_ptr()))
    }

    pub fn edict_of_vars(&self, pev: *mut entvars_t) -> *mut edict_t {
        eng!(pfnFindEntityByVars(pev))
    }

    pub fn index_of(&self, e: *const edict_t) -> i32 {
        eng!(pfnIndexOfEdict(e))
    }

    /// Next entity after `start` whose `field` equals `value`; null when there are no more.
    pub fn find_by_string(&self, start: *mut edict_t, field: &CStr, value: &CStr) -> *mut edict_t {
        let e = eng!(pfnFindEntityByString(start, field.as_ptr(), value.as_ptr()));
        // The engine returns the world edict (index 0) for "not found".
        if e.is_null() || self.index_of(e) == 0 { std::ptr::null_mut() } else { e }
    }

    pub fn client_print(&self, e: *mut edict_t, kind: PRINT_TYPE, msg: &str) {
        let s = to_cstring(msg);
        eng!(pfnClientPrintf(e, kind, s.as_ptr()))
    }

    pub fn cmd_argv(&self, i: c_int) -> &'static CStr {
        let p = eng!(pfnCmd_Argv(i));
        unsafe { cstr(p) }
    }
}
