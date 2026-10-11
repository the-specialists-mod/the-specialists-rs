//! Engine helpers added by the render work package. Keep additions here to avoid merge conflicts.
//!
//! * `ev!`/`efx!`/`tri!` call a slot of `pEventAPI`/`pEfxAPI`/`pTriAPI` (like `cl_eng!`).
//! * Thin wrappers for the calls the view/events/effects modules use most. Cvars are
//!   [`crate::client::Cvar`].
//! * `vec3`: small vector maths on `[f32; 3]` (pure, unit-tested).

use crate::client::engine;
use crate::sys::*;
use crate::util::to_cstring;
use std::ffi::{CStr, c_int};

pub type Vec3 = [f32; 3];

/// Float/flag macros bindgen doesn't export (const.h, cvardef.h).
pub const ATTN_NORM: f32 = 0.8;
pub const FCVAR_ARCHIVE: i32 = 1;

/// `client_data_t` (cdll_int.h), the `HUD_UpdateClientData` argument.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct ClientData {
    pub origin: Vec3,
    pub viewangles: Vec3,
    pub weapon_bits: i32,
    pub fov: f32,
}

/// The engine's event API table, for `ev!`. Panics if the engine has none;
/// [`ClEngine::event_api`](crate::client::ClEngine::event_api) is the null-checked accessor.
pub fn event_api() -> &'static event_api_s {
    engine().event_api().expect("pEventAPI")
}
/// The engine's effects API table, for `efx!`. Panics if the engine has none;
/// [`ClEngine::efx_api`](crate::client::ClEngine::efx_api) is the null-checked accessor.
pub fn efx_api() -> &'static efx_api_s {
    engine().efx_api().expect("pEfxAPI")
}
/// The engine's triangle API table, for `tri!`. Panics if the engine has none;
/// [`ClEngine::tri_api`](crate::client::ClEngine::tri_api) is the null-checked accessor.
pub fn tri_api() -> &'static triangleapi_s {
    engine().tri_api().expect("pTriAPI")
}

/// Calls a `pEventAPI` slot, panicking with its name if the engine left it null.
#[macro_export]
macro_rules! ev {
    ($name:ident ( $($arg:expr),* $(,)? )) => {
        // SAFETY: the engine fills its function tables before the first callback and keeps them

        // for the process; slots are called with the argument types bindgen generated from the

        // engine headers, and callers pass valid pointers for those arguments.

        unsafe { ($crate::ext::render::event_api().$name.expect(stringify!($name)))($($arg),*) }
    };
}
/// Calls a `pEfxAPI` slot, panicking with its name if the engine left it (or the table) null.
#[macro_export]
macro_rules! efx {
    ($name:ident ( $($arg:expr),* $(,)? )) => {
        // SAFETY: the engine fills its function tables before the first callback and keeps them

        // for the process; slots are called with the argument types bindgen generated from the

        // engine headers, and callers pass valid pointers for those arguments.

        unsafe { ($crate::ext::render::efx_api().$name.expect(stringify!($name)))($($arg),*) }
    };
}
/// Calls a `pTriAPI` slot.
#[macro_export]
macro_rules! tri {
    ($name:ident ( $($arg:expr),* $(,)? )) => {
        // SAFETY: the engine fills its function tables before the first callback and keeps them

        // for the process; slots are called with the argument types bindgen generated from the

        // engine headers, and callers pass valid pointers for those arguments.

        unsafe { ($crate::ext::render::tri_api().$name.expect(stringify!($name)))($($arg),*) }
    };
}

use crate::cl_eng;

/// `EV_SetTraceHull` + `EV_PlayerTrace`: the single wrapper for them. Call inside
/// [`with_solid_players`] when players must be solid.
pub fn player_trace(start: Vec3, end: Vec3, hull: i32, flags: i32, ignore_pe: i32) -> pmtrace_s {
    let (mut s, mut e) = (start, end);
    let mut tr = pmtrace_s::default();
    ev!(EV_SetTraceHull(hull));
    ev!(EV_PlayerTrace(s.as_mut_ptr(), e.as_mut_ptr(), flags, ignore_pe, &mut tr));
    tr
}

/// The client engine's `PM_TraceLine(start, end, flags, usehull, ignore_pe)`: a trace against the
/// client player-move physents (the world, brush entities and whatever else the last prediction
/// left there), copied out. The single wrapper for it; a missing result reads as no hit.
pub fn pm_trace_line(start: Vec3, end: Vec3, flags: i32, usehull: i32, ignore_pe: i32) -> pmtrace_s {
    let (mut s, mut e) = (start, end);
    let tr = cl_eng!(PM_TraceLine(s.as_mut_ptr(), e.as_mut_ptr(), flags, usehull, ignore_pe));
    // SAFETY: the engine returns null or a pointer to its static trace result, valid until the
    // next trace; it is copied out at once.
    unsafe { tr.as_ref() }.copied().unwrap_or(pmtrace_s { fraction: 1.0, endpos: end, ..Default::default() })
}

/// Makes the other players solid for traces (`EV_SetUpPlayerPrediction` + `EV_PushPMStates` +
/// `EV_SetSolidPlayers`), runs `f`, and restores the pmove state (`EV_PopPMStates`). The single
/// wrapper for that sequence; `player_index0` is the 0-based player left out (-1 for none).
///
/// The order is the SDK's (`ev_hldm.cpp` `EV_HLDM_FireBullets`) and that of TS's events. TS's
/// HUD (`DrawCrosshairID` 1004acc0) pushes before setting up prediction. The two calls are
/// independent (prediction fills the engine's predicted-player list, the push saves the physent
/// counts), so [`ClEngine::crosshair_trace`](crate::client::ClEngine::crosshair_trace) uses this
/// order too.
pub fn with_solid_players<R>(player_index0: i32, f: impl FnOnce() -> R) -> R {
    ev!(EV_SetUpPlayerPrediction(0, 1));
    ev!(EV_PushPMStates());
    ev!(EV_SetSolidPlayers(player_index0));
    let r = f();
    ev!(EV_PopPMStates());
    r
}

/// Texture name hit by a trace from `start` to `end` against physent `ground`.
pub fn trace_texture(ground: i32, start: Vec3, end: Vec3) -> Option<String> {
    let (mut s, mut e) = (start, end);
    let p = ev!(EV_TraceTexture(ground, s.as_mut_ptr(), e.as_mut_ptr()));
    // SAFETY: `EV_TraceTexture` returns null or a NUL-terminated texture name the engine owns; copied at once.
    if p.is_null() { None } else { Some(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()) }
}

pub fn physent(idx: i32) -> Option<&'static physent_s> {
    let p = ev!(EV_GetPhysent(idx));
    // SAFETY: the engine returns null or a pointer to an object it owns that stays valid for the current frame.
    unsafe { p.as_ref() }
}

pub fn index_from_trace(tr: &mut pmtrace_s) -> i32 {
    ev!(EV_IndexFromTrace(tr))
}

/// `EV_IsLocal` takes a 0-based player number; this takes the entity index.
pub fn is_local(entindex: i32) -> bool {
    ev!(EV_IsLocal(entindex - 1)) != 0
}

pub fn is_player(entindex: i32) -> bool {
    entindex >= 1 && entindex <= engine().max_clients()
}

pub fn weapon_animation(seq: i32, body: i32) {
    ev!(EV_WeaponAnimation(seq, body));
}

pub fn model_index(name: &str) -> i32 {
    let s = to_cstring(name);
    ev!(EV_FindModelIndex(s.as_ptr()))
}

pub fn temp_model(pos: Vec3, vel: Vec3, angles: Vec3, life: f32, model: i32, soundtype: i32) -> Option<&'static mut TEMPENTITY> {
    let p = efx!(R_TempModel(pos.as_ptr(), vel.as_ptr(), angles.as_ptr(), life, model, soundtype));
    // SAFETY: the engine returns null or a pointer to an object it owns that stays valid for the current frame.
    unsafe { p.as_mut() }
}

/// Decal texture index for a decal name such as `{shot1`.
pub fn decal_index(name: &str) -> i32 {
    let s = to_cstring(name);
    let id = efx!(Draw_DecalIndexFromName(s.as_ptr()));
    efx!(Draw_DecalIndex(id))
}

pub fn decal_shoot(texture: i32, entity: i32, model_index: i32, pos: Vec3, flags: i32) {
    let mut p = pos;
    efx!(R_DecalShoot(texture, entity, model_index, p.as_mut_ptr(), flags));
}

pub fn alloc_dlight(key: i32) -> Option<&'static mut dlight_s> {
    let p = efx!(CL_AllocDlight(key));
    // SAFETY: the engine returns null or a pointer to an object it owns that stays valid for the current frame.
    unsafe { p.as_mut() }
}

pub fn spark_effect(pos: Vec3, count: i32, vmin: i32, vmax: i32) {
    efx!(R_SparkEffect(pos.as_ptr(), count, vmin, vmax));
}

pub fn set_screen_fade(f: &mut screenfade_s) {
    cl_eng!(pfnSetScreenFade(f));
}

/// Hooks a client event (`pfnHookEvent`). Events don't pass their name to the hook, so each
/// hooked event gets its own trampoline slot (up to [`MAX_EVENT_HOOKS`]).
pub fn hook_event(name: &'static CStr, f: fn(&event_args_s)) {
    let slot = {
        let mut hooks = EVENT_HOOKS.0.borrow_mut();
        match hooks.iter().position(|(n, _)| *n == name) {
            Some(i) => {
                hooks[i].1 = f;
                i
            }
            None => {
                assert!(hooks.len() < MAX_EVENT_HOOKS, "too many event hooks");
                hooks.push((name, f));
                hooks.len() - 1
            }
        }
    };
    cl_eng!(pfnHookEvent(name.as_ptr(), Some(EVENT_TRAMPOLINES[slot])));
}

pub const MAX_EVENT_HOOKS: usize = 32;
type EventHook = (&'static CStr, fn(&event_args_s));
static EVENT_HOOKS: crate::Global<std::cell::RefCell<Vec<EventHook>>> = crate::Global(std::cell::RefCell::new(Vec::new()));

unsafe extern "C" fn event_trampoline<const N: usize>(args: *mut event_args_s) {
    // SAFETY: the engine passes the event's arguments (or null).
    let Some(args) = (unsafe { args.as_ref() }) else { return };
    let f = EVENT_HOOKS.0.borrow().get(N).map(|h| h.1);
    if let Some(f) = f {
        f(args);
    }
}

macro_rules! trampolines {
    ($($n:literal)*) => { [$(event_trampoline::<$n> as unsafe extern "C" fn(*mut event_args_s)),*] };
}
static EVENT_TRAMPOLINES: [unsafe extern "C" fn(*mut event_args_s); MAX_EVENT_HOOKS] =
    trampolines!(0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31);

pub fn precache_event(name: &'static CStr) -> u16 {
    cl_eng!(pfnPrecacheEvent(1, name.as_ptr()))
}

/// Client-side event playback (weapon prediction). `invoker` is null on the client.
#[allow(clippy::too_many_arguments)]
pub fn playback_event(flags: i32, index: u16, f1: f32, f2: f32, i1: i32, i2: i32, b1: bool, b2: i32) {
    let mut zero = [0.0f32; 3];
    let mut zero2 = [0.0f32; 3];
    cl_eng!(pfnPlaybackEvent(flags, std::ptr::null(), index, 0.0, zero.as_mut_ptr(), zero2.as_mut_ptr(), f1, f2, i1, i2, b1 as c_int, b2));
}

pub fn view_model() -> Option<&'static mut cl_entity_s> {
    let p = cl_eng!(GetViewModel());
    // SAFETY: the engine returns null or a pointer to an object it owns that stays valid for the current frame.
    unsafe { p.as_mut() }
}
pub fn random_float(lo: f32, hi: f32) -> f32 {
    cl_eng!(pfnRandomFloat(lo, hi))
}
pub fn random_long(lo: i32, hi: i32) -> i32 {
    cl_eng!(pfnRandomLong(lo, hi))
}
pub fn point_contents(p: Vec3) -> i32 {
    cl_eng!(PM_PointContents(p.as_ptr(), std::ptr::null_mut()))
}
/// `IsNoClipping`: the local player is in noclip.
pub fn is_no_clipping() -> bool {
    cl_eng!(IsNoClipping()) != 0
}
/// `pparams->cmd->forwardmove` (0 when the engine passes no command).
pub fn refdef_forwardmove(p: &ref_params_t) -> f32 {
    // SAFETY: the engine passes null or a pointer to its current usercmd, valid for the
    // V_CalcRefdef call `p` belongs to.
    unsafe { p.cmd.as_ref() }.map_or(0.0, |c| c.forwardmove)
}

/// Loads a game file through the engine filesystem (searches the game dirs and paks).
pub fn load_file(path: &str) -> Option<Vec<u8>> {
    let s = to_cstring(path);
    let mut len: c_int = 0;
    let p = cl_eng!(COM_LoadFile(s.as_ptr(), 5, &mut len));
    if p.is_null() {
        return None;
    }
    // SAFETY: the engine returned `len` bytes at non-null `p` (checked above); they're copied before the file is freed.
    let v = unsafe { std::slice::from_raw_parts(p.cast::<u8>(), len.max(0) as usize) }.to_vec();
    cl_eng!(COM_FreeFile(p.cast()));
    Some(v)
}

/// `pfnGetLevelName`: the single wrapper for it. `maps/<name>.bsp` while a map is loaded, empty
/// otherwise.
pub fn level_name() -> String {
    let p = cl_eng!(pfnGetLevelName());
    // SAFETY: the engine returns null or a NUL-terminated string it owns; it's used or copied before the next engine call.
    unsafe { crate::util::cstr(p) }.to_string_lossy().into_owned()
}

pub fn spr_frames(h: HSPRITE) -> i32 {
    cl_eng!(pfnSPR_Frames(h))
}
/// `GetSpritePointer`: the single wrapper for it. The sprite's model, for TriAPI
/// [`sprite_texture`](crate::ext::tri::sprite_texture) and visible entities; null if `h` is not
/// loaded.
pub fn sprite_model(h: HSPRITE) -> *const model_s {
    cl_eng!(GetSpritePointer(h))
}

/// Small vector helpers on `[f32; 3]`.
pub mod vec3 {
    use super::Vec3;
    pub const ZERO: Vec3 = [0.0; 3];
    pub fn add(a: Vec3, b: Vec3) -> Vec3 {
        [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
    }
    pub fn sub(a: Vec3, b: Vec3) -> Vec3 {
        [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
    }
    pub fn scale(a: Vec3, s: f32) -> Vec3 {
        [a[0] * s, a[1] * s, a[2] * s]
    }
    /// `a + b * s`
    pub fn ma(a: Vec3, s: f32, b: Vec3) -> Vec3 {
        [a[0] + b[0] * s, a[1] + b[1] * s, a[2] + b[2] * s]
    }
    pub fn dot(a: Vec3, b: Vec3) -> f32 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }
    pub fn cross(a: Vec3, b: Vec3) -> Vec3 {
        [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
    }
    pub fn len(a: Vec3) -> f32 {
        dot(a, a).sqrt()
    }
    pub fn len2d(a: Vec3) -> f32 {
        (a[0] * a[0] + a[1] * a[1]).sqrt()
    }
    pub fn dist(a: Vec3, b: Vec3) -> f32 {
        len(sub(a, b))
    }
    pub fn normalize(a: Vec3) -> Vec3 {
        let l = len(a);
        if l > 0.0 { scale(a, 1.0 / l) } else { a }
    }
    /// Quake `AngleVectors` (pitch, yaw, roll in degrees) -> (forward, right, up).
    pub fn angle_vectors(a: Vec3) -> (Vec3, Vec3, Vec3) {
        let (sp, cp) = a[0].to_radians().sin_cos();
        let (sy, cy) = a[1].to_radians().sin_cos();
        let (sr, cr) = a[2].to_radians().sin_cos();
        let f = [cp * cy, cp * sy, -sp];
        let r = [-sr * sp * cy + cr * sy, -sr * sp * sy - cr * cy, -sr * cp];
        let u = [cr * sp * cy + sr * sy, cr * sp * sy - sr * cy, cr * cp];
        (f, r, u)
    }
    /// Quake `VectorAngles` (pitch is positive up, as the engine returns it).
    pub fn vector_angles(f: Vec3) -> Vec3 {
        if f[0] == 0.0 && f[1] == 0.0 {
            return [if f[2] > 0.0 { 90.0 } else { 270.0 }, 0.0, 0.0];
        }
        let mut yaw = f[1].atan2(f[0]).to_degrees();
        if yaw < 0.0 {
            yaw += 360.0;
        }
        let mut pitch = f[2].atan2(len2d(f)).to_degrees();
        if pitch < 0.0 {
            pitch += 360.0;
        }
        [pitch, yaw, 0.0]
    }
    /// Normalises an angle to (-180, 180].
    pub fn norm_angle(mut a: f32) -> f32 {
        a %= 360.0;
        if a > 180.0 {
            a -= 360.0;
        } else if a <= -180.0 {
            a += 360.0;
        }
        a
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn vectors() {
            let (f, r, u) = angle_vectors([0.0, 90.0, 0.0]);
            assert!(dist(f, [0.0, 1.0, 0.0]) < 1e-5);
            assert!(dist(r, [1.0, 0.0, 0.0]) < 1e-5);
            assert!(dist(u, [0.0, 0.0, 1.0]) < 1e-5);
            let a = vector_angles([1.0, 1.0, 0.0]);
            assert!((a[1] - 45.0).abs() < 1e-4);
            assert_eq!(norm_angle(350.0), -10.0);
            assert_eq!(cross([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]), [0.0, 0.0, 1.0]);
        }
    }
}
