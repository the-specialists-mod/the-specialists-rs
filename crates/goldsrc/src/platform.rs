//! Native mouse input for Steam GoldSrc clients (FWGS sends mouse deltas through
//! `IN_ClientLookEvent` instead): SDL2 relative mouse mode, looked up at run time in the SDL2
//! library the engine already loaded, and the Windows cursor re-centring of the SDK's
//! inputw32.cpp.

/// SDL2 entry points from the SDL2 library the engine already loaded (looked up at run time).
pub mod sdl {
    use std::ffi::{CStr, c_int, c_void};
    use std::sync::OnceLock;

    type GetRelativeMouseState = unsafe extern "C" fn(*mut c_int, *mut c_int) -> u32;
    type SetRelativeMouseMode = unsafe extern "C" fn(c_int) -> c_int;

    struct Sdl {
        get_relative: Option<GetRelativeMouseState>,
        set_relative: Option<SetRelativeMouseMode>,
    }

    #[cfg(not(windows))]
    pub(crate) fn lookup(name: &CStr) -> *mut c_void {
        unsafe extern "C" {
            fn dlopen(file: *const std::ffi::c_char, mode: c_int) -> *mut c_void;
            fn dlsym(handle: *mut c_void, name: *const std::ffi::c_char) -> *mut c_void;
        }
        const RTLD_NOW: c_int = 2;
        const RTLD_NOLOAD: c_int = 4;
        // SAFETY: dlopen/dlsym are called with NUL-terminated names; RTLD_NOLOAD never loads a new library.
        unsafe {
            for lib in [c"libSDL2-2.0.so.0", c"libSDL2.so"] {
                let h = dlopen(lib.as_ptr(), RTLD_NOW | RTLD_NOLOAD);
                if !h.is_null() {
                    let p = dlsym(h, name.as_ptr());
                    if !p.is_null() {
                        return p;
                    }
                }
            }
            // RTLD_DEFAULT: anything already in the global namespace.
            dlsym(std::ptr::null_mut(), name.as_ptr())
        }
    }

    #[cfg(windows)]
    pub(crate) fn lookup(name: &CStr) -> *mut c_void {
        let h = unsafe { super::win::GetModuleHandleA(c"SDL2.dll".as_ptr()) };
        if h.is_null() { std::ptr::null_mut() } else { unsafe { super::win::GetProcAddress(h, name.as_ptr()) } }
    }

    fn sdl() -> &'static Sdl {
        static S: OnceLock<Sdl> = OnceLock::new();
        S.get_or_init(|| {
            let g = lookup(c"SDL_GetRelativeMouseState");
            let s = lookup(c"SDL_SetRelativeMouseMode");
            Sdl {
                // SAFETY: a non-null symbol with this name is the SDL2 function with this C signature (SDL2 ABI).
                get_relative: (!g.is_null()).then(|| unsafe { std::mem::transmute::<*mut c_void, GetRelativeMouseState>(g) }),
                // SAFETY: a non-null symbol with this name is the SDL2 function with this C signature (SDL2 ABI).
                set_relative: (!s.is_null()).then(|| unsafe { std::mem::transmute::<*mut c_void, SetRelativeMouseMode>(s) }),
            }
        })
    }

    pub fn available() -> bool {
        sdl().get_relative.is_some()
    }

    pub fn relative_delta() -> [f32; 2] {
        let Some(f) = sdl().get_relative else { return [0.0; 2] };
        let (mut x, mut y) = (0, 0);
        // SAFETY: SDL2 function looked up above, called with its C signature and valid arguments.
        unsafe { f(&mut x, &mut y) };
        [x as f32, y as f32]
    }

    pub fn set_relative(on: bool) {
        if let Some(f) = sdl().set_relative {
            // SAFETY: SDL2 function looked up above, called with its C signature and valid arguments.
            unsafe { f(on as c_int) };
        }
    }
}

/// Windows cursor re-centering (SDK inputw32.cpp, no mouse thread).
#[cfg(windows)]
pub mod win {
    use crate::client::engine;
    use std::ffi::{c_char, c_int, c_void};

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    pub struct Point {
        x: i32,
        y: i32,
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetCursorPos(p: *mut Point) -> c_int;
        fn SetCursorPos(x: c_int, y: c_int) -> c_int;
        fn SystemParametersInfoA(action: u32, param: u32, pv: *mut c_void, winini: u32) -> c_int;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        pub fn GetModuleHandleA(name: *const c_char) -> *mut c_void;
        pub fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
    }

    const SPI_GETMOUSE: u32 = 3;
    const SPI_SETMOUSE: u32 = 4;

    #[derive(Default)]
    pub struct State {
        params_valid: bool,
        original: [c_int; 3],
        restore: bool,
    }

    pub fn startup(s: &mut State) {
        s.params_valid = unsafe { SystemParametersInfoA(SPI_GETMOUSE, 0, s.original.as_mut_ptr().cast(), 0) } != 0;
    }

    fn raw_input() -> bool {
        engine().cvar_float(c"m_rawinput") != 0.0 && super::sdl::available()
    }

    fn center() -> (i32, i32) {
        let f = &engine().f;
        match (f.GetWindowCenterX, f.GetWindowCenterY) {
            (Some(x), Some(y)) => unsafe { (x(), y()) },
            _ => (0, 0),
        }
    }

    pub fn activate(s: &mut State) {
        if s.params_valid {
            // SDK newmouseparms {0, 0, 1}: no Windows acceleration while playing.
            let mut p: [c_int; 3] = [0, 0, 1];
            s.restore = unsafe { SystemParametersInfoA(SPI_SETMOUSE, 0, p.as_mut_ptr().cast(), 0) } != 0;
        }
    }

    pub fn deactivate(s: &mut State) {
        if s.restore {
            unsafe { SystemParametersInfoA(SPI_SETMOUSE, 0, s.original.as_mut_ptr().cast(), 0) };
            s.restore = false;
        }
    }

    /// Cursor offset from the window centre, then re-centre (or SDL relative motion under
    /// `m_rawinput 1`, as the post-anniversary SDK does).
    pub fn delta() -> [f32; 2] {
        if raw_input() {
            return super::sdl::relative_delta();
        }
        let mut p = Point::default();
        if unsafe { GetCursorPos(&mut p) } == 0 {
            return [0.0; 2];
        }
        let (cx, cy) = center();
        let d = [(p.x - cx) as f32, (p.y - cy) as f32];
        if d != [0.0; 2] {
            unsafe { SetCursorPos(cx, cy) };
        }
        d
    }
}
