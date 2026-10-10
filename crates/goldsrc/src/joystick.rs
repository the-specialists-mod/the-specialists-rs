//! Native joystick for Steam GoldSrc clients (FWGS reads joysticks itself and sends movement
//! through `IN_ClientMoveEvent`). The game logic (`ts_client::joystick`) ports TS's
//! `IN_JoyMove`/`IN_Commands`; this module only reads the device:
//!
//! * **Windows**: the WinMM joystick API (`joyGetNumDevs`/`joyGetPosEx`/`joyGetDevCapsA`), as
//!   TS's `IN_StartupJoystick` (10027960) and `IN_ReadJoystick` (10027de0) do.
//! * **Elsewhere** (Linux GoldSrc has no WinMM): the first SDL2 game controller, looked up at run
//!   time in the SDL2 library the engine already loaded, as the SDL-era SDK's inputw32.cpp does.
//!   Sticks map to the X/Y (left) and Z/R (right) axes; U/V stay centred.
//!
//! Readings use WinMM's convention for every back end: axes 0..=65535 centred on 32768, a button
//! bit mask, and the POV hat in hundredths of a degree (0xffff = centred).

use crate::Global;
use crate::cl_eng;
use crate::client::ClEngine;
use crate::util::to_cstring;
use std::cell::Cell;

/// `JOY_MAX_AXES`: X, Y, Z, R, U, V.
pub const MAX_AXES: usize = 6;
/// WinMM's centred POV value (`JOY_POVCENTERED`).
pub const POV_CENTERED: u32 = 0xffff;
/// WinMM `JOY_RETURN*` flags for [`read`] (TS builds `dwFlags` from the axis map in
/// `Joy_AdvancedUpdate_f` 10027b00).
pub const RETURN_AXIS: [u32; MAX_AXES] = [0x1, 0x2, 0x4, 0x8, 0x10, 0x20];
/// `JOY_RETURNCENTERED | JOY_RETURNBUTTONS | JOY_RETURNPOV`.
pub const RETURN_BASE: u32 = 0x400 | 0x80 | 0x40;

/// A device found by [`startup`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Caps {
    pub buttons: u32,
    pub has_pov: bool,
    /// SDL controller name (WinMM: empty, TS prints none).
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reading {
    pub axes: [u32; MAX_AXES],
    pub buttons: u32,
    pub pov: u32,
}

/// Result of [`startup`]: the device, or why there is none (TS prints it with `Con_DPrintf`).
#[derive(Debug, Clone, PartialEq)]
pub enum Startup {
    Found(Caps),
    NotFound(String),
    /// Nothing changed since the last scan (SDL rescans at most once a second).
    Unchanged,
}

impl ClEngine {
    /// `CheckParm`: whether the command line has `parm` (e.g. `-nojoy`).
    pub fn check_parm(&self, parm: &str) -> bool {
        let s = to_cstring(parm);
        cl_eng!(CheckParm(s.as_ptr().cast_mut(), std::ptr::null_mut())) != 0
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::ffi::c_char;

    #[repr(C)]
    #[derive(Default)]
    struct JoyInfoEx {
        size: u32,
        flags: u32,
        pos: [u32; MAX_AXES],
        buttons: u32,
        button_number: u32,
        pov: u32,
        reserved: [u32; 2],
    }

    #[repr(C)]
    struct JoyCapsA {
        mid: u16,
        pid: u16,
        pname: [c_char; 32],
        xyz_minmax: [u32; 6],
        num_buttons: u32,
        period: [u32; 2],
        ruv_minmax: [u32; 6],
        caps: u32,
        max_axes: u32,
        num_axes: u32,
        max_buttons: u32,
        reg_key: [c_char; 32],
        oem_vxd: [c_char; 260],
    }

    const _: () = assert!(size_of::<JoyInfoEx>() == 0x34 && size_of::<JoyCapsA>() == 0x194);
    const JOYCAPS_HASPOV: u32 = 0x10;
    const JOY_RETURNCENTERED: u32 = 0x400;

    #[link(name = "winmm")]
    unsafe extern "system" {
        fn joyGetNumDevs() -> u32;
        fn joyGetPosEx(id: u32, info: *mut JoyInfoEx) -> u32;
        fn joyGetDevCapsA(id: usize, caps: *mut JoyCapsA, size: u32) -> u32;
    }

    /// The device TS found (`joy_id`), once startup succeeded.
    static JOY_ID: Global<Cell<Option<u32>>> = Global(Cell::new(None));

    fn get_pos(id: u32, flags: u32) -> Option<JoyInfoEx> {
        let mut info = JoyInfoEx { size: size_of::<JoyInfoEx>() as u32, flags, ..Default::default() };
        // SAFETY: WinMM call with a correctly sized, writable JOYINFOEX (`size` set as the API requires).
        (unsafe { joyGetPosEx(id, &mut info) } == 0).then_some(info)
    }

    /// TS `IN_StartupJoystick` (10027960): the first device that answers `joyGetPosEx`. WinMM
    /// devices aren't rescanned (TS starts once, from `IN_Init`).
    pub fn startup(rescan: bool) -> Startup {
        if rescan {
            return Startup::Unchanged;
        }
        JOY_ID.0.set(None);
        // SAFETY: plain WinMM query without arguments.
        let n = unsafe { joyGetNumDevs() };
        if n == 0 {
            return Startup::NotFound("joystick not found -- driver not present\n\n".into());
        }
        let Some(id) = (0..n).find(|&id| get_pos(id, JOY_RETURNCENTERED).is_some()) else {
            return Startup::NotFound("joystick not found -- no valid joysticks\n\n".into());
        };
        // SAFETY: JOYCAPSA is plain data; all-zero bytes are valid before the call fills it.
        let mut caps: JoyCapsA = unsafe { std::mem::zeroed() };
        // SAFETY: WinMM call with a writable JOYCAPSA of the size passed.
        let r = unsafe { joyGetDevCapsA(id as usize, &mut caps, size_of::<JoyCapsA>() as u32) };
        if r != 0 {
            return Startup::NotFound(format!("joystick not found -- invalid joystick capabilities ({r:x})\n\n"));
        }
        JOY_ID.0.set(Some(id));
        Startup::Found(Caps { buttons: caps.num_buttons, has_pov: caps.caps & JOYCAPS_HASPOV != 0, name: String::new() })
    }

    /// TS `IN_ReadJoystick` (10027de0).
    pub fn read(flags: u32) -> Option<Reading> {
        let info = get_pos(JOY_ID.0.get()?, flags)?;
        Some(Reading { axes: info.pos, buttons: info.buttons, pov: info.pov })
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;
    use crate::platform::sdl::lookup;
    use crate::util::cstr;
    use std::ffi::{CStr, c_char, c_int, c_void};
    use std::sync::OnceLock;

    /// `SDL_CONTROLLER_BUTTON_MAX` in the SDL2 the HL25 SDK ships.
    const BUTTONS: u32 = 21;
    /// SDL_CONTROLLER_AXIS_LEFTX, LEFTY, RIGHTX, RIGHTY for X, Y, Z, R.
    const AXES: [c_int; 4] = [0, 1, 2, 3];

    type NumJoysticks = unsafe extern "C" fn() -> c_int;
    type IsGameController = unsafe extern "C" fn(c_int) -> c_int;
    type Open = unsafe extern "C" fn(c_int) -> *mut c_void;
    type Close = unsafe extern "C" fn(*mut c_void);
    type Name = unsafe extern "C" fn(*mut c_void) -> *const c_char;
    type GetAxis = unsafe extern "C" fn(*mut c_void, c_int) -> i16;
    type GetButton = unsafe extern "C" fn(*mut c_void, c_int) -> u8;
    type Update = unsafe extern "C" fn();

    struct Api {
        num_joysticks: NumJoysticks,
        is_game_controller: IsGameController,
        open: Open,
        close: Close,
        name: Name,
        get_axis: GetAxis,
        get_button: GetButton,
        update: Update,
    }

    fn api() -> Option<&'static Api> {
        static A: OnceLock<Option<Api>> = OnceLock::new();
        A.get_or_init(|| {
            let f = |n: &CStr| {
                let p = lookup(n);
                (!p.is_null()).then_some(p)
            };
            // SAFETY: each non-null symbol with these names is the SDL2 function with the C
            // signature declared in `Api` (SDL2 ABI).
            unsafe {
                Some(Api {
                    num_joysticks: std::mem::transmute::<*mut c_void, NumJoysticks>(f(c"SDL_NumJoysticks")?),
                    is_game_controller: std::mem::transmute::<*mut c_void, IsGameController>(f(c"SDL_IsGameController")?),
                    open: std::mem::transmute::<*mut c_void, Open>(f(c"SDL_GameControllerOpen")?),
                    close: std::mem::transmute::<*mut c_void, Close>(f(c"SDL_GameControllerClose")?),
                    name: std::mem::transmute::<*mut c_void, Name>(f(c"SDL_GameControllerName")?),
                    get_axis: std::mem::transmute::<*mut c_void, GetAxis>(f(c"SDL_GameControllerGetAxis")?),
                    get_button: std::mem::transmute::<*mut c_void, GetButton>(f(c"SDL_GameControllerGetButton")?),
                    update: std::mem::transmute::<*mut c_void, Update>(f(c"SDL_JoystickUpdate")?),
                })
            }
        })
        .as_ref()
    }

    /// The open controller (`s_pJoystick`) and the last scan time.
    static PAD: Global<Cell<*mut c_void>> = Global(Cell::new(std::ptr::null_mut()));
    static LAST_CHECK: Global<Cell<f64>> = Global(Cell::new(0.0));

    /// SDK (SDL) `IN_StartupJoystick`: open the first game controller; with `rescan`, at most
    /// once a second, notice a controller being plugged in or every joystick going away.
    pub fn startup(rescan: bool) -> Startup {
        let Some(a) = api() else {
            return if rescan { Startup::Unchanged } else { Startup::NotFound("joystick not found -- SDL2 not available\n\n".into()) };
        };
        // The SDK uses GetAbsoluteTime, which the 2007 engine table TS targets doesn't have.
        let now = crate::client::engine().client_time() as f64;
        let last = LAST_CHECK.0.get();
        // (The client clock restarts with each map, hence the `now < last` check.)
        if last > 0.0 && now >= last && now - last < 1.0 {
            return Startup::Unchanged;
        }
        LAST_CHECK.0.set(now);
        // SAFETY: SDL2 functions from `api()`, called with their C signatures; the pad pointer is
        // only ever one SDL returned and is closed at most once.
        unsafe {
            let n = (a.num_joysticks)();
            if n <= 0 {
                let pad = PAD.0.replace(std::ptr::null_mut());
                if !pad.is_null() {
                    (a.close)(pad);
                    return Startup::NotFound("joystick not found -- driver not present\n\n".into());
                }
                return if rescan { Startup::Unchanged } else { Startup::NotFound("joystick not found -- driver not present\n\n".into()) };
            }
            if !PAD.0.get().is_null() {
                return Startup::Unchanged;
            }
            for i in 0..n {
                if (a.is_game_controller)(i) == 0 {
                    continue;
                }
                let pad = (a.open)(i);
                if !pad.is_null() {
                    PAD.0.set(pad);
                    let name = cstr((a.name)(pad)).to_string_lossy().into_owned();
                    return Startup::Found(Caps { buttons: BUTTONS, has_pov: false, name });
                }
            }
        }
        Startup::Unchanged
    }

    /// SDK (SDL) `IN_ReadJoystick` + `RawValuePointer`, shifted to WinMM's 0..=65535 range.
    pub fn read(_flags: u32) -> Option<Reading> {
        let a = api()?;
        let pad = PAD.0.get();
        if pad.is_null() {
            return None;
        }
        let mut r = Reading { axes: [32768; MAX_AXES], buttons: 0, pov: POV_CENTERED };
        // SAFETY: SDL2 functions from `api()` on the controller SDL opened for us.
        unsafe {
            (a.update)();
            for (i, &axis) in AXES.iter().enumerate() {
                r.axes[i] = ((a.get_axis)(pad, axis) as i32 + 32768) as u32;
            }
            for b in 0..BUTTONS {
                if (a.get_button)(pad, b as c_int) != 0 {
                    r.buttons |= 1 << b;
                }
            }
        }
        Some(r)
    }
}

/// Finds a joystick. `rescan` is the per-frame call from `IN_JoyMove` (SDL hot-plugging; a no-op
/// for WinMM, which TS scans once).
pub fn startup(rescan: bool) -> Startup {
    imp::startup(rescan)
}

/// Reads the device found by [`startup`]; `flags` are WinMM `JOY_RETURN*` bits.
pub fn read(flags: u32) -> Option<Reading> {
    imp::read(flags)
}
