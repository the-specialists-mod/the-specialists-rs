//! Engine helpers added by the weapons work package. Keep additions here to avoid merge conflicts.

#[cfg(feature = "server")]
pub use server::*;

#[cfg(feature = "server")]
mod server {
    use crate::edict::EdictPtr;
    use crate::eng;
    use crate::server::Engine;
    use crate::sys::*;
    use std::ffi::{CStr, c_int};

    /// `svc_weaponanim` (engine protocol 48).
    pub const SVC_WEAPONANIM: i32 = 35;

    /// Weapon-related engine calls not wrapped by `goldsrc::server`.
    pub trait WeaponsEngine {
        /// `pfnCanSkipPlayer`: the client predicts weapons (`cl_lw 1`), so `SendWeaponAnim(...,
        /// skiplocal = 1)` may skip it.
        fn can_skip_player(&self, e: *const edict_t) -> bool;
        /// `pfnFindEntityInSphere`; returns null when done (the world edict counts as "done").
        fn find_in_sphere(&self, start: *mut edict_t, origin: [f32; 3], radius: f32) -> *mut edict_t;
        /// `pfnPrecacheModel` for a name that isn't `'static` (interned once).
        fn precache_model_str(&self, name: &str) -> i32;
        fn precache_sound_str(&self, name: &str) -> i32;
        /// `pfnTraceModel`-free helper: `pfnTraceLine` ignoring nothing but `skip`, with
        /// `fNoMonsters`.
        fn trace_line_ex(&self, start: [f32; 3], end: [f32; 3], no_monsters: c_int, skip: *mut edict_t) -> TraceResult;
        /// `pfnGetPlayerUserId` != -1 and the edict is a client in use.
        fn is_player_edict(&self, e: *const edict_t) -> bool;
        /// `pfnMessageBegin(MSG_ONE, SVC_WEAPONANIM)` + anim + body.
        fn send_weapon_anim(&self, to: *mut edict_t, anim: i32, body: i32);
        fn get_global_trace_hit(&self) -> *mut edict_t;
        fn model_index_str(&self, name: &CStr) -> i32;
        /// `pfnGetGameDir`: the game directory (e.g. `ts`).
        fn game_dir(&self) -> String;
        /// `PLAYBACK_EVENT_FULL(flags, invoker, index, 0, zero, zero, ...)` with the `bparam`s
        /// passed as ints (TS sends the shot seed in `bparam2`, which a `bool` would truncate).
        #[allow(clippy::too_many_arguments)]
        fn playback_weapon_event(
            &self,
            flags: i32,
            invoker: *mut edict_t,
            index: u16,
            f1: f32,
            f2: f32,
            i1: i32,
            i2: i32,
            b1: i32,
            b2: i32,
        );
    }

    impl WeaponsEngine for Engine {
        fn can_skip_player(&self, e: *const edict_t) -> bool {
            match self.f.pfnCanSkipPlayer {
                // SAFETY: engine callback from the API table above, called with valid arguments.
                Some(f) => unsafe { f(e) != 0 },
                None => false,
            }
        }

        fn find_in_sphere(&self, start: *mut edict_t, origin: [f32; 3], radius: f32) -> *mut edict_t {
            let e = eng!(pfnFindEntityInSphere(start, origin.as_ptr(), radius));
            if e.is_null() || self.index_of(e) == 0 { std::ptr::null_mut() } else { e }
        }

        fn precache_model_str(&self, name: &str) -> i32 {
            self.precache_model(crate::server::intern(name))
        }

        fn precache_sound_str(&self, name: &str) -> i32 {
            self.precache_sound(crate::server::intern(name))
        }

        fn trace_line_ex(&self, start: [f32; 3], end: [f32; 3], no_monsters: c_int, skip: *mut edict_t) -> TraceResult {
            self.trace_line(start, end, no_monsters, skip)
        }

        fn is_player_edict(&self, e: *const edict_t) -> bool {
            if e.is_null() {
                return false;
            }
            let i = self.index_of(e);
            i >= 1 && i <= self.max_clients() && e.v().flags & FL_CLIENT as i32 != 0
        }

        fn send_weapon_anim(&self, to: *mut edict_t, anim: i32, body: i32) {
            crate::server::MsgWriter::begin(MSG_ONE as i32, SVC_WEAPONANIM, None, to).byte(anim).byte(body).end();
        }

        fn get_global_trace_hit(&self) -> *mut edict_t {
            self.globals().trace_ent
        }

        fn model_index_str(&self, name: &CStr) -> i32 {
            self.model_index(name)
        }

        fn game_dir(&self) -> String {
            let mut buf = [0 as std::ffi::c_char; 512];
            eng!(pfnGetGameDir(buf.as_mut_ptr()));
            // SAFETY: `pfnGetGameDir` wrote a NUL-terminated string into `buf`.
            unsafe { CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().into_owned()
        }

        fn playback_weapon_event(
            &self,
            flags: i32,
            invoker: *mut edict_t,
            index: u16,
            f1: f32,
            f2: f32,
            i1: i32,
            i2: i32,
            b1: i32,
            b2: i32,
        ) {
            let (mut o, mut ang) = ([0.0f32; 3], [0.0f32; 3]);
            eng!(pfnPlaybackEvent(flags, invoker, index, 0.0, o.as_mut_ptr(), ang.as_mut_ptr(), f1, f2, i1, i2, b1, b2));
        }
    }
}
