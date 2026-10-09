//! Client-side engine handle (`cl_enginefunc_t`) and helpers.

use crate::sys::*;
use crate::util::to_cstring;
use std::ffi::CStr;
use std::sync::OnceLock;

pub struct ClEngine {
    pub f: cl_enginefunc_t,
}
unsafe impl Send for ClEngine {}
unsafe impl Sync for ClEngine {}

static ENGINE: OnceLock<ClEngine> = OnceLock::new();

/// Called from the client library's `Initialize` export.
///
/// # Safety
/// `funcs` must point to a valid client engine function table.
pub unsafe fn init(funcs: *const cl_enginefunc_t) {
    let _ = ENGINE.set(ClEngine { f: unsafe { *funcs } });
}

pub fn engine() -> &'static ClEngine {
    ENGINE.get().expect("Initialize not called yet")
}

/// Calls a client engine slot, panicking with its name if the engine left it null.
#[macro_export]
macro_rules! cl_eng {
    ($name:ident ( $($arg:expr),* $(,)? )) => {
        unsafe { ($crate::client::engine().f.$name.expect(stringify!($name)))($($arg),*) }
    };
}

impl ClEngine {
    pub fn print(&self, msg: &str) {
        let s = to_cstring(msg);
        cl_eng!(pfnConsolePrint(s.as_ptr()))
    }

    /// The engine keeps `name`, so it must be `'static`.
    pub fn add_command(&self, name: &'static CStr, f: unsafe extern "C" fn()) {
        cl_eng!(pfnAddCommand(name.as_ptr(), Some(f)));
    }

    pub fn hook_user_msg(&self, name: &'static CStr, f: pfnUserMsgHook) {
        cl_eng!(pfnHookUserMsg(name.as_ptr(), f));
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
        unsafe { crate::util::cstr(p) }
    }

    pub fn angle_vectors(&self, angles: [f32; 3]) -> ([f32; 3], [f32; 3], [f32; 3]) {
        let (mut f, mut r, mut u) = ([0.0; 3], [0.0; 3], [0.0; 3]);
        cl_eng!(pfnAngleVectors(angles.as_ptr(), f.as_mut_ptr(), r.as_mut_ptr(), u.as_mut_ptr()));
        (f, r, u)
    }
}
