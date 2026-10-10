//! Engine helpers added by the rules work package. Keep additions here to avoid merge conflicts.

use crate::edict::EdictPtr;
use crate::eng;
use crate::server::Engine;
use crate::sys::*;
use crate::util::{cstr, to_cstring};
use std::ffi::CStr;

/// `SVC_INTERMISSION` (engine message number; not in the bindings' allowlist).
pub const SVC_INTERMISSION: i32 = 30;
/// Older `SVC_HLTV`/director number TS uses with `MSG_SPEC` (architecture.md §3.2).
pub const SVC_HLTV: i32 = 50;

impl Engine {
    /// `pfnLoadFileForMe` + `pfnFreeFile`: reads a file from the game's search path.
    pub fn load_file(&self, name: &str) -> Option<Vec<u8>> {
        let n = to_cstring(name);
        let mut len = 0;
        let p = eng!(pfnLoadFileForMe(n.as_ptr(), &mut len));
        if p.is_null() {
            return None;
        }
        // SAFETY: the engine returned `len` bytes at non-null `p` (checked above); they're copied before the file is freed.
        let out = unsafe { std::slice::from_raw_parts(p as *const u8, len.max(0) as usize) }.to_vec();
        eng!(pfnFreeFile(p.cast()));
        Some(out)
    }

    /// Next entity after `start` whose bounding box touches the sphere; null when done.
    pub fn find_in_sphere(&self, start: *mut edict_t, org: [f32; 3], radius: f32) -> *mut edict_t {
        let e = eng!(pfnFindEntityInSphere(start, org.as_ptr(), radius));
        if e.is_null() || self.index_of(e) == 0 { std::ptr::null_mut() } else { e }
    }

    pub fn set_cvar_string(&self, name: &CStr, value: &str) {
        let v = to_cstring(value);
        eng!(pfnCVarSetString(name.as_ptr(), v.as_ptr()))
    }

    /// `pfnCvar_DirectSet` (no "cvar changed" broadcast; used for mp_timeleft/mp_fragsleft).
    pub fn cvar_direct_set(&self, c: &cvar_t, value: &str) {
        let v = to_cstring(value);
        eng!(pfnCvar_DirectSet((c as *const cvar_t).cast_mut(), v.as_ptr()))
    }

    pub fn change_level(&self, map: &str) {
        let m = to_cstring(map);
        eng!(pfnChangeLevel(m.as_ptr(), std::ptr::null()))
    }

    pub fn is_map_valid(&self, map: &str) -> bool {
        let m = to_cstring(map);
        eng!(pfnIsMapValid(m.as_ptr().cast_mut())) != 0
    }

    pub fn auth_id(&self, e: *mut edict_t) -> &'static CStr {
        let p = eng!(pfnGetPlayerAuthId(e));
        // SAFETY: the engine returns null or a NUL-terminated string it owns; it's used or copied before the next engine call.
        unsafe { cstr(p) }
    }

    /// Sets a key in a client's userinfo (`pfnSetClientKeyValue`), e.g. `model` or `team`.
    pub fn set_client_key(&self, e: *mut edict_t, key: &str, value: &str) {
        let buf = eng!(pfnGetInfoKeyBuffer(e));
        if buf.is_null() {
            return;
        }
        let (k, v) = (to_cstring(key), to_cstring(value));
        eng!(pfnSetClientKeyValue(self.index_of(e), buf, k.as_ptr().cast_mut(), v.as_ptr().cast_mut()))
    }

    pub fn set_view(&self, client: *mut edict_t, view: *mut edict_t) {
        eng!(pfnSetView(client, view))
    }

    /// Writes a line to the server log (`UTIL_LogPrintf`: `at_logged`).
    pub fn log(&self, msg: &str) {
        self.alert(ALERT_TYPE::at_logged, msg);
    }

    /// Edict of a living, connected client slot `i`, or null.
    pub fn client(&self, i: i32) -> *mut edict_t {
        let e = self.player(i);
        if e.is_null() || e.v().flags & FL_CLIENT as i32 == 0 { std::ptr::null_mut() } else { e }
    }

    /// Iterator over connected client edicts (slots 1..=maxClients).
    pub fn clients(&self) -> impl Iterator<Item = *mut edict_t> + '_ {
        (1..=self.max_clients()).map(|i| self.client(i)).filter(|e| !e.is_null())
    }

    pub fn number_of_entities(&self) -> i32 {
        eng!(pfnNumberOfEntities())
    }

    /// Server console command buffer flush (`pfnServerExecute`).
    pub fn server_execute(&self) {
        eng!(pfnServerExecute())
    }

    /// `Cvar_DirectSet` on the named cvar, if it exists (bypasses the `FCVAR_SPONLY`-style
    /// checks `pfnCvar_SetString` applies).
    pub fn cvar_direct_set_by_name(&self, name: &CStr, value: &str) {
        let c = self.cvar_ptr(name);
        if !c.is_null() {
            let v = to_cstring(value);
            eng!(pfnCvar_DirectSet(c, v.as_ptr()))
        }
    }

    pub fn cvar_ptr(&self, name: &CStr) -> *mut cvar_t {
        eng!(pfnCVarGetPointer(name.as_ptr()))
    }
}
