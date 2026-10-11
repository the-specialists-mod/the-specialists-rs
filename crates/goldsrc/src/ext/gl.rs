//! Raw OpenGL calls, for the few places TS's client calls `opengl32.dll` itself rather than going
//! through the engine: `V_SetFog` (`0x10056c20`: `glFogi`, `glFogfv`, `glFogf`, `glHint`,
//! `glEnable`) and the `glDisable(GL_FOG)` that ends `TS_DrawEffects` (`0x10056e5e`). They act on
//! the GL state behind the engine's back, as TS's do.
//!
//! The functions are looked up at run time in the GL library the engine's renderer already
//! loaded, never loading one:
//!
//! * Windows (Steam `hw.dll`, FWGS `ref_gl` under Windows or Wine): `opengl32.dll`, which exports
//!   these GL 1.1 entry points directly, as TS imports them;
//! * Linux (Steam `hw.so`, FWGS `ref_gl`): `SDL_GL_GetProcAddress` from the SDL2 the engine
//!   loaded (the GL library SDL loaded for the context), else `libGL.so.1`/`libOpenGL.so.0` if
//!   already loaded.
//!
//! Every call does nothing unless the engine is drawing with desktop OpenGL ([`active`]): FWGS's
//! loaded renderer (`r_refdll_loaded`) is `gl`, or, on engines without that cvar (Steam
//! GoldSrc), `IEngineStudio.IsHardware()` is 1 (`hw`, not the software `sw`). FWGS's software
//! renderer and its GLES renderers (`gles1`, `gles2`, `gl4es`, `gles3compat`, which translate GL
//! through a wrapper the raw calls would bypass) get nothing. A function the library doesn't
//! export is skipped on its own.
//!
//! Call only from the engine's render callbacks (`V_CalcRefdef`, `HUD_DrawTransparentTriangles`),
//! where the renderer's context is current on the calling thread.

use std::ffi::{CStr, c_void};
use std::sync::OnceLock;

pub const GL_FOG: u32 = 0x0b60;
pub const GL_FOG_DENSITY: u32 = 0x0b62;
pub const GL_FOG_START: u32 = 0x0b63;
pub const GL_FOG_END: u32 = 0x0b64;
pub const GL_FOG_MODE: u32 = 0x0b65;
pub const GL_FOG_COLOR: u32 = 0x0b66;
pub const GL_FOG_HINT: u32 = 0x0c54;
pub const GL_NICEST: u32 = 0x1102;
pub const GL_LINEAR: i32 = 0x2601;

// `APIENTRY`: stdcall on 32-bit Windows, C elsewhere.
type Cap = unsafe extern "system" fn(u32);
type Fogi = unsafe extern "system" fn(u32, i32);
type Fogf = unsafe extern "system" fn(u32, f32);
type Fogfv = unsafe extern "system" fn(u32, *const f32);
type Hint = unsafe extern "system" fn(u32, u32);

struct Gl {
    enable: Option<Cap>,
    disable: Option<Cap>,
    fogi: Option<Fogi>,
    fogf: Option<Fogf>,
    fogfv: Option<Fogfv>,
    hint: Option<Hint>,
}

/// The engine draws with desktop OpenGL (see the module docs).
pub fn active() -> bool {
    let refdll = crate::client::Cvar::find(c"r_refdll_loaded");
    if !refdll.is_null() {
        return refdll.string().is_some_and(|s| s.to_bytes() == b"gl");
    }
    crate::ext::studio::hardware_kind() == 1
}

/// The GL functions, looked up on the first call made while [`active`] (the renderer can't change
/// without restarting the engine). `None` when GL isn't active.
fn gl() -> Option<&'static Gl> {
    static GL: OnceLock<Gl> = OnceLock::new();
    if let Some(gl) = GL.get() {
        return Some(gl);
    }
    if !active() {
        return None;
    }
    Some(GL.get_or_init(|| {
        let f = |name: &CStr| {
            let p = lookup(name);
            (!p.is_null()).then_some(p)
        };
        Gl {
            // SAFETY: a non-null GL export with this name is the GL 1.1 function with this
            // `APIENTRY` signature (gl.h). The same holds for the five below.
            enable: f(c"glEnable").map(|p| unsafe { std::mem::transmute::<*mut c_void, Cap>(p) }),
            // SAFETY: as above.
            disable: f(c"glDisable").map(|p| unsafe { std::mem::transmute::<*mut c_void, Cap>(p) }),
            // SAFETY: as above.
            fogi: f(c"glFogi").map(|p| unsafe { std::mem::transmute::<*mut c_void, Fogi>(p) }),
            // SAFETY: as above.
            fogf: f(c"glFogf").map(|p| unsafe { std::mem::transmute::<*mut c_void, Fogf>(p) }),
            // SAFETY: as above.
            fogfv: f(c"glFogfv").map(|p| unsafe { std::mem::transmute::<*mut c_void, Fogfv>(p) }),
            // SAFETY: as above.
            hint: f(c"glHint").map(|p| unsafe { std::mem::transmute::<*mut c_void, Hint>(p) }),
        }
    }))
}

#[cfg(windows)]
fn lookup(name: &CStr) -> *mut c_void {
    use crate::platform::win::{GetModuleHandleA, GetProcAddress};
    // SAFETY: GetModuleHandleA takes a NUL-terminated name, only looks up an already-loaded
    // module (it never loads one) and returns null when opengl32.dll isn't loaded.
    let h = unsafe { GetModuleHandleA(c"opengl32.dll".as_ptr()) };
    if h.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: `h` is a module handle from GetModuleHandleA and `name` is NUL-terminated;
    // GetProcAddress returns null for a missing export.
    unsafe { GetProcAddress(h, name.as_ptr()) }
}

#[cfg(not(windows))]
fn lookup(name: &CStr) -> *mut c_void {
    use std::ffi::{c_char, c_int};
    unsafe extern "C" {
        fn dlopen(file: *const c_char, mode: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    }
    const RTLD_NOW: c_int = 2;
    const RTLD_NOLOAD: c_int = 4;
    type GetProcAddress = unsafe extern "C" fn(*const c_char) -> *mut c_void;

    let gpa = crate::platform::sdl::lookup(c"SDL_GL_GetProcAddress");
    if !gpa.is_null() {
        // SAFETY: a non-null `SDL_GL_GetProcAddress` is SDL2's function with this C signature.
        let gpa = unsafe { std::mem::transmute::<*mut c_void, GetProcAddress>(gpa) };
        // SAFETY: `name` is NUL-terminated. [`gl`] only gets here while the engine draws with GL,
        // so SDL already has the GL library loaded and doesn't load one now.
        let p = unsafe { gpa(name.as_ptr()) };
        if !p.is_null() {
            return p;
        }
    }
    for lib in [c"libGL.so.1", c"libOpenGL.so.0"] {
        // SAFETY: dlopen/dlsym take NUL-terminated names; RTLD_NOLOAD never loads a new library.
        let p = unsafe {
            let h = dlopen(lib.as_ptr(), RTLD_NOW | RTLD_NOLOAD);
            if h.is_null() { std::ptr::null_mut() } else { dlsym(h, name.as_ptr()) }
        };
        if !p.is_null() {
            return p;
        }
    }
    std::ptr::null_mut()
}

/// `glEnable(cap)`.
pub fn enable(cap: u32) {
    if let Some(f) = gl().and_then(|g| g.enable) {
        // SAFETY: GL function looked up above, called with its signature on the render thread
        // (module docs), where the renderer's context is current.
        unsafe { f(cap) }
    }
}

/// `glDisable(cap)`.
pub fn disable(cap: u32) {
    if let Some(f) = gl().and_then(|g| g.disable) {
        // SAFETY: as in [`enable`].
        unsafe { f(cap) }
    }
}

/// `glFogi(pname, param)`.
pub fn fogi(pname: u32, param: i32) {
    if let Some(f) = gl().and_then(|g| g.fogi) {
        // SAFETY: as in [`enable`].
        unsafe { f(pname, param) }
    }
}

/// `glFogf(pname, param)`.
pub fn fogf(pname: u32, param: f32) {
    if let Some(f) = gl().and_then(|g| g.fogf) {
        // SAFETY: as in [`enable`].
        unsafe { f(pname, param) }
    }
}

/// `glFogfv(pname, params)`: four floats, enough for `GL_FOG_COLOR` (the largest fog parameter).
pub fn fogfv(pname: u32, params: &[f32; 4]) {
    if let Some(f) = gl().and_then(|g| g.fogfv) {
        // SAFETY: as in [`enable`]; GL reads at most four floats for a fog parameter.
        unsafe { f(pname, params.as_ptr()) }
    }
}

/// `glHint(target, mode)`.
pub fn hint(target: u32, mode: u32) {
    if let Some(f) = gl().and_then(|g| g.hint) {
        // SAFETY: as in [`enable`].
        unsafe { f(target, mode) }
    }
}
