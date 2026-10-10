//! Client-side engine handle (`cl_enginefunc_t`) and helpers.

use crate::Global;
use crate::sys::*;
use crate::util::to_cstring;
use std::cell::{RefCell, UnsafeCell};
use std::ffi::{CStr, c_char, c_int, c_void};
use std::sync::OnceLock;

pub struct ClEngine {
    pub f: cl_enginefunc_t,
}
// SAFETY: the table holds engine function pointers that live for the process; the engine runs
// every client callback on one thread.
unsafe impl Send for ClEngine {}
// SAFETY: as above.
unsafe impl Sync for ClEngine {}

static ENGINE: OnceLock<ClEngine> = OnceLock::new();

/// Called from the client library's `Initialize` export.
///
/// # Safety
/// `funcs` must point to a valid client engine function table.
pub unsafe fn init(funcs: *const cl_enginefunc_t) {
    // SAFETY: `funcs` points at the engine's function table for the duration of this call; it's copied once.
    let _ = ENGINE.set(ClEngine { f: unsafe { *funcs } });
}

pub fn engine() -> &'static ClEngine {
    ENGINE.get().expect("Initialize not called yet")
}

/// Calls a client engine slot, panicking with its name if the engine left it null.
#[macro_export]
macro_rules! cl_eng {
    ($name:ident ( $($arg:expr),* $(,)? )) => {
        // SAFETY: the engine fills its function tables before the first callback and keeps them

        // for the process; slots are called with the argument types bindgen generated from the

        // engine headers, and callers pass valid pointers for those arguments.

        unsafe { ($crate::client::engine().f.$name.expect(stringify!($name)))($($arg),*) }
    };
}

impl ClEngine {
    pub fn print(&self, msg: &str) {
        let s = to_cstring(msg);
        cl_eng!(pfnConsolePrint(s.as_ptr()))
    }

    /// Registers a console command. `f` is a safe `extern "C" fn()` that reads its arguments
    /// through `cmd_argc`/`cmd_argv`. The engine keeps `name`, so it must be `'static`.
    pub fn add_command(&self, name: &'static CStr, f: extern "C" fn()) {
        cl_eng!(pfnAddCommand(name.as_ptr(), Some(f)));
    }

    /// Hooks a server user message. The engine calls one shared trampoline with the message
    /// name; it finds `f` by name and passes the payload as a byte slice. Return value: the
    /// engine ignores it, so the handler has none.
    pub fn hook_user_msg(&self, name: &'static CStr, f: fn(&[u8])) {
        let mut hooks = USER_MSG_HOOKS.0.borrow_mut();
        match hooks.iter_mut().find(|(n, _)| *n == name) {
            Some(h) => h.1 = f,
            None => hooks.push((name, f)),
        }
        drop(hooks);
        cl_eng!(pfnHookUserMsg(name.as_ptr(), Some(user_msg_trampoline)));
    }

    pub fn cvar_float(&self, name: &CStr) -> f32 {
        cl_eng!(pfnGetCvarFloat(name.as_ptr()))
    }

    pub fn view_angles(&self) -> [f32; 3] {
        let mut a = [0.0; 3];
        cl_eng!(GetViewAngles(a.as_mut_ptr()));
        a
    }

    pub fn screen_size(&self) -> (i32, i32) {
        let mut info = SCREENINFO { iSize: size_of::<SCREENINFO>() as i32, ..Default::default() };
        cl_eng!(pfnGetScreenInfo(&mut info));
        (info.iWidth, info.iHeight)
    }

    pub fn draw_string(&self, x: i32, y: i32, rgb: [f32; 3], text: &str) {
        let s = to_cstring(text);
        cl_eng!(pfnDrawSetTextColor(rgb[0], rgb[1], rgb[2]));
        cl_eng!(pfnDrawConsoleString(x, y, s.as_ptr().cast_mut()));
    }

    pub fn fill_rgba(&self, x: i32, y: i32, w: i32, h: i32, rgba: [i32; 4]) {
        cl_eng!(pfnFillRGBA(x, y, w, h, rgba[0], rgba[1], rgba[2], rgba[3]));
    }

    pub fn cmd_argv(&self, i: i32) -> &'static CStr {
        let p = cl_eng!(Cmd_Argv(i));
        // SAFETY: the engine returns null or a NUL-terminated string it owns; it's used or copied before the next engine call.
        unsafe { crate::util::cstr(p) }
    }

    pub fn angle_vectors(&self, angles: [f32; 3]) -> ([f32; 3], [f32; 3], [f32; 3]) {
        let (mut f, mut r, mut u) = ([0.0; 3], [0.0; 3], [0.0; 3]);
        cl_eng!(pfnAngleVectors(angles.as_ptr(), f.as_mut_ptr(), r.as_mut_ptr(), u.as_mut_ptr()));
        (f, r, u)
    }
}

type UserMsgHook = (&'static CStr, fn(&[u8]));
static USER_MSG_HOOKS: Global<RefCell<Vec<UserMsgHook>>> = Global(RefCell::new(Vec::new()));

/// The one `pfnUserMsgHook` every hooked message goes through (GoldSrc and FWGS pass the
/// message name to the hook).
unsafe extern "C" fn user_msg_trampoline(name: *const c_char, size: c_int, buf: *mut c_void) -> c_int {
    // SAFETY: the engine passes the message name and its payload buffer of `size` bytes.
    let (name, bytes) = unsafe {
        let name = crate::util::cstr(name);
        let bytes: &[u8] = if buf.is_null() || size <= 0 { &[] } else { std::slice::from_raw_parts(buf.cast(), size as usize) };
        (name, bytes)
    };
    // Copy the handler out so it can hook or unhook messages itself.
    let f = USER_MSG_HOOKS.0.borrow().iter().find(|(n, _)| *n == name).map(|h| h.1);
    if let Some(f) = f {
        f(bytes);
    }
    1
}

/// A cvar registered with or found in the engine. The engine owns the `cvar_t` for the rest of
/// the process, so the handle is `Copy`; a null handle reads as 0.
#[derive(Clone, Copy)]
pub struct Cvar(*mut cvar_s);

impl Cvar {
    pub const NULL: Cvar = Cvar(std::ptr::null_mut());

    pub fn register(name: &'static CStr, value: &'static CStr, flags: i32) -> Cvar {
        Cvar(cl_eng!(pfnRegisterVariable(name.as_ptr(), value.as_ptr(), flags)))
    }

    pub fn find(name: &CStr) -> Cvar {
        Cvar(cl_eng!(pfnGetCvarPointer(name.as_ptr())))
    }

    pub fn is_null(self) -> bool {
        self.0.is_null()
    }

    pub fn value(self) -> f32 {
        // SAFETY: non-null cvar pointers come from the engine and live for the process.
        if self.0.is_null() { 0.0 } else { unsafe { (*self.0).value } }
    }
}

/// Storage the engine also reads and writes through a pointer we hand it, such as the
/// `kbutton_t`s returned from `KB_Find`. Access is single-threaded (engine callbacks), and a
/// borrow from [`EngineCell::get`] must not be held across a call back into the engine.
pub struct EngineCell<T>(UnsafeCell<T>);
// SAFETY: only used from the engine's single client thread.
unsafe impl<T> Sync for EngineCell<T> {}

impl<T> EngineCell<T> {
    pub const fn new(v: T) -> Self {
        EngineCell(UnsafeCell::new(v))
    }

    #[allow(clippy::mut_from_ref)]
    pub fn get(&self) -> &mut T {
        // SAFETY: single-threaded engine callbacks; callers keep borrows short (see type docs).
        unsafe { &mut *self.0.get() }
    }

    /// The pointer to give the engine.
    pub fn as_ptr(&self) -> *mut T {
        self.0.get()
    }
}
