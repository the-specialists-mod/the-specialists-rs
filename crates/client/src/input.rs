//! Keyboard buttons (`+forward` etc.) and `CL_CreateMove`. Mouse look is added by FWGS after
//! `CL_CreateMove` returns (`IN_EngineAppendMove`), so there is no mouse code here yet.

use goldsrc::Global;
use goldsrc::client::engine;
use goldsrc::sys::*;
use std::cell::UnsafeCell;
use std::ffi::{CStr, c_int, c_void};

/// Same layout as the SDK's `kbutton_t`: the engine reads `state` through `KB_Find`.
#[repr(C)]
pub struct KButton {
    down: [c_int; 2],
    /// bit 0: held, bit 1: pressed since last frame, bit 2: released since last frame.
    state: c_int,
}

pub struct Button(Global<UnsafeCell<KButton>>);

impl Button {
    const fn new() -> Self {
        Button(Global(UnsafeCell::new(KButton { down: [0; 2], state: 0 })))
    }

    fn get(&self) -> &mut KButton {
        // Engine callbacks are single-threaded and never re-enter while this borrow is alive.
        unsafe { &mut *self.0.0.get() }
    }

    fn key_arg() -> c_int {
        let c = engine().cmd_argv(1);
        if c.is_empty() { -1 } else { c.to_str().ok().and_then(|s| s.parse().ok()).unwrap_or(-1) }
    }

    fn key_down(&self) {
        let b = self.get();
        let k = Self::key_arg();
        if k == b.down[0] || k == b.down[1] {
            return; // repeating key
        }
        if b.down[0] == 0 {
            b.down[0] = k;
        } else if b.down[1] == 0 {
            b.down[1] = k;
        } else {
            return; // three keys on one button
        }
        if b.state & 1 == 0 {
            b.state |= 1 + 2;
        }
    }

    fn key_up(&self) {
        let b = self.get();
        let k = Self::key_arg();
        if k == -1 {
            // Typed manually at the console: release everything.
            b.down = [0; 2];
            b.state = 4;
            return;
        }
        if b.down[0] == k {
            b.down[0] = 0;
        } else if b.down[1] == k {
            b.down[1] = 0;
        } else {
            return;
        }
        if b.down != [0; 2] || b.state & 1 == 0 {
            return;
        }
        b.state = (b.state & !1) | 4;
    }

    /// Whether the button was down at any point this frame. Clears the edge bits.
    fn active(&self) -> bool {
        let b = self.get();
        let on = b.state & 3 != 0;
        b.state &= 1;
        on
    }

    fn ptr(&self) -> *mut c_void {
        self.0.0.get().cast()
    }
}

macro_rules! buttons {
    ($($ident:ident, $name:literal, $down:literal, $up:literal, $bit:expr;)*) => {
        $(pub static $ident: Button = Button::new();)*

        /// `IN_*` bit for each button that maps directly onto `usercmd_t::buttons`.
        static BUTTON_BITS: &[(&Button, u32)] = &[$((&$ident, $bit)),*];

        fn register_commands() {
            $({
                unsafe extern "C" fn down() { $ident.key_down() }
                unsafe extern "C" fn up() { $ident.key_up() }
                engine().add_command($down, down);
                engine().add_command($up, up);
            })*
        }

        pub fn find(name: &CStr) -> *mut c_void {
            $(if name == $name { return $ident.ptr(); })*
            std::ptr::null_mut()
        }
    };
}

buttons! {
    IN_FWD,   c"in_forward",   c"+forward",   c"-forward",   IN_FORWARD;
    IN_BK,    c"in_back",      c"+back",      c"-back",      IN_BACK;
    IN_LEFT,  c"in_moveleft",  c"+moveleft",  c"-moveleft",  IN_MOVELEFT;
    IN_RIGHT, c"in_moveright", c"+moveright", c"-moveright", IN_MOVERIGHT;
    IN_UPK,   c"in_moveup",    c"+moveup",    c"-moveup",    0;
    IN_DOWNK, c"in_movedown",  c"+movedown",  c"-movedown",  0;
    IN_SPEED, c"in_speed",     c"+speed",     c"-speed",     IN_RUN;
    IN_JMP,   c"in_jump",      c"+jump",      c"-jump",      IN_JUMP;
    IN_DCK,   c"in_duck",      c"+duck",      c"-duck",      IN_DUCK;
    IN_ATK,   c"in_attack",    c"+attack",    c"-attack",    IN_ATTACK;
    IN_ATK2,  c"in_attack2",   c"+attack2",   c"-attack2",   IN_ATTACK2;
    IN_USEK,  c"in_use",       c"+use",       c"-use",       IN_USE;
    IN_RELOD, c"in_reload",    c"+reload",    c"-reload",    IN_RELOAD;
}

fn cvar_or(name: &CStr, default: f32) -> f32 {
    match engine().cvar_float(name) {
        v if v > 0.0 => v,
        _ => default,
    }
}

pub fn init() {
    register_commands();
}

pub fn activate_mouse() {}
pub fn deactivate_mouse() {}
pub fn mouse_event(_mstate: i32) {}
pub fn clear_states() {}
pub fn accumulate() {}
pub fn frame(_time: f64) {}
pub fn key_event(_down: bool, _key: i32, _binding: &CStr) {}

pub fn create_move(_frametime: f32, cmd: &mut usercmd_t, active: bool) {
    // Read movement keys first: `active()` clears their edge bits.
    let k = |b: &Button| b.active() as i32 as f32;
    let (fwd, back, left, right, up, down) = (k(&IN_FWD), k(&IN_BK), k(&IN_LEFT), k(&IN_RIGHT), k(&IN_UPK), k(&IN_DOWNK));
    let mut buttons = 0u32;
    for (b, bit) in BUTTON_BITS {
        if *bit != 0 && b.get().state & 3 != 0 {
            buttons |= bit;
        }
        b.get().state &= 1;
    }

    if active {
        cmd.forwardmove = cvar_or(c"cl_forwardspeed", 400.0) * fwd - cvar_or(c"cl_backspeed", 400.0) * back;
        cmd.sidemove = cvar_or(c"cl_sidespeed", 400.0) * (right - left);
        cmd.upmove = cvar_or(c"cl_upspeed", 320.0) * (up - down);
        if buttons & IN_RUN != 0 {
            let s = cvar_or(c"cl_movespeedkey", 0.3);
            cmd.forwardmove *= s;
            cmd.sidemove *= s;
            cmd.upmove *= s;
        }
        cmd.buttons = buttons as u16;
    }
    cmd.viewangles = engine().view_angles();
}
