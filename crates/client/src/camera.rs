//! TS's third-person camera controls (`CAM_Init` `0x10025370`, `CAM_Think` `0x10024c80`): the
//! `+campitchup`/`+campitchdown`/`+camyawleft`/`+camyawright`/`+camin`/`+camout` key buttons, the
//! `+cammousemove`/`+camdistance` mouse modes and `snapto`. They adjust `cam_idealpitch`,
//! `cam_idealyaw` and `cam_idealdist` within `c_min/max*`. `view` owns `cam_thirdperson`,
//! `cam_ofs`, the `thirdperson` toggle and the rest of `CAM_Think`, and calls [`adjust_ideal`].
//! TS's code is the SDK's `in_camera.cpp` with the distance easing removed (see `view`).

use crate::input::Button;
use goldsrc::Global;
use goldsrc::client::{Cvar, engine};
use std::cell::Cell;

// The camera's `kbutton_t`s (TS `0x100df338`, `0x100df370`, `0x100df2e8`, `0x100df348`,
// `0x100df380`, `0x100df328`). `KB_Find` doesn't list them, as in the SDK.
static PITCH_UP: Button = Button::new();
static PITCH_DOWN: Button = Button::new();
static YAW_LEFT: Button = Button::new();
static YAW_RIGHT: Button = Button::new();
static CAM_IN: Button = Button::new();
static CAM_OUT: Button = Button::new();

/// The mouse-mode flags and the last cursor reading.
#[derive(Default, Clone, Copy)]
struct MouseMode {
    /// `cam_mousemove` (`0x100df2dc`).
    mousemove: bool,
    /// `cam_distancemove` (`0x100df354`).
    distancemove: bool,
    /// `iMouseInUse` (`0x100df394`): mouse look, `IN_Accumulate` and the mouse buttons stand down.
    in_use: bool,
    /// `cam_mouse` (`0x100df308`) as an offset from the window centre: TS reads the cursor only
    /// in `+cammousemove`, so `+camdistance` after `-cammousemove` keeps using the last reading.
    cursor: [i32; 2],
    /// Sub-count motion carried to the next reading (FWGS hands over fractional counts).
    frac: [f32; 2],
}

static MOUSE: Global<Cell<MouseMode>> =
    Global(Cell::new(MouseMode { mousemove: false, distancemove: false, in_use: false, cursor: [0; 2], frac: [0.0; 2] }));

fn update(f: impl FnOnce(&mut MouseMode)) {
    let mut m = MOUSE.0.get();
    f(&mut m);
    MOUSE.0.set(m);
}

/// `iMouseInUse`: `+cammousemove` or `+camdistance` holds the mouse.
pub fn mouse_in_use() -> bool {
    MOUSE.0.get().in_use
}

macro_rules! cmd {
    ($name:literal, $body:expr) => {{
        extern "C" fn f() {
            $body
        }
        engine().add_command($name, f);
    }};
}

/// The first part of `CAM_Init` (`0x10025370`), in TS's order: the key buttons, `thirdperson`
/// and `firstperson` (passed in, they live in `view`), the mouse modes and `snapto`.
pub(crate) fn register_commands(thirdperson: extern "C" fn(), firstperson: extern "C" fn()) {
    cmd!(c"+campitchup", PITCH_UP.key_down());
    cmd!(c"-campitchup", PITCH_UP.key_up());
    cmd!(c"+campitchdown", PITCH_DOWN.key_down());
    cmd!(c"-campitchdown", PITCH_DOWN.key_up());
    cmd!(c"+camyawleft", YAW_LEFT.key_down());
    cmd!(c"-camyawleft", YAW_LEFT.key_up());
    cmd!(c"+camyawright", YAW_RIGHT.key_down());
    cmd!(c"-camyawright", YAW_RIGHT.key_up());
    cmd!(c"+camin", CAM_IN.key_down());
    cmd!(c"-camin", CAM_IN.key_up());
    cmd!(c"+camout", CAM_OUT.key_down());
    cmd!(c"-camout", CAM_OUT.key_up());
    let e = engine();
    e.add_command(c"thirdperson", thirdperson);
    e.add_command(c"firstperson", firstperson);
    cmd!(c"+cammousemove", start_mouse_move());
    cmd!(c"-cammousemove", update(|m| (m.mousemove, m.in_use) = (false, false)));
    cmd!(c"+camdistance", start_distance());
    cmd!(c"-camdistance", update(|m| (m.distancemove, m.mousemove, m.in_use) = (false, false, false)));
    cmd!(c"snapto", toggle_snapto());
}

/// `CAM_StartMouseMove` (`0x100255d0`): only in third person (`cam_thirdperson`). TS's cursor
/// read and sensitivity-scaled `cam_old_mouse_x/y` here are never used, so they're left out.
fn start_mouse_move() {
    let third = crate::view::is_third_person();
    update(|m| {
        if !third {
            (m.mousemove, m.in_use) = (false, false);
        } else if !m.mousemove {
            (m.mousemove, m.in_use) = (true, true);
        }
    });
}

/// `CAM_StartDistance` (`0x10025680`): as [`start_mouse_move`], and the mouse sets the distance.
fn start_distance() {
    let third = crate::view::is_third_person();
    update(|m| {
        if !third {
            (m.distancemove, m.mousemove, m.in_use) = (false, false, false);
        } else if !m.distancemove {
            (m.distancemove, m.mousemove, m.in_use) = (true, true, true);
        }
    });
}

/// `snapto` (`0x10025340`): `cam_snapto->value = !cam_snapto->value`, written to the value only.
fn toggle_snapto() {
    let c = Cvar::find(c"cam_snapto");
    c.set_value(if c.value() == 0.0 { 1.0 } else { 0.0 });
}

/// `c_minpitch`.. `c_maxdistance` (`0x100df2fc`, `0x100df324`, `0x100df2d0`, `0x100df310`,
/// `0x100df38c`, `0x100df2f8`) as (min, max).
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub pitch: (f32, f32),
    pub yaw: (f32, f32),
    pub dist: (f32, f32),
}

/// The mouse step of `CAM_Think` for one axis: `off` is the cursor's offset from the window
/// centre. Moving towards the bound adds `(off / 2) * scale` (integer halving, as TS) while
/// still inside it, then clamps.
pub fn mouse_step(v: f32, off: i32, (min, max): (f32, f32), scale: f32) -> f32 {
    let mut v = v;
    if off > 0 {
        if v < max {
            v += (off / 2) as f32 * scale;
        }
        if v > max {
            v = max;
        }
    } else if off < 0 {
        if v > min {
            v -= -(off / 2) as f32 * scale;
        }
        if v < min {
            v = min;
        }
    }
    v
}

/// The key step of `CAM_Think` (`0x10024f10..0x10025023`) on (pitch, yaw, dist). `held` is
/// `CL_KeyState(button) != 0`; it's called in TS's order and skipped where TS's `else if` skips
/// it, since `CL_KeyState` clears the button's impulse bits. Keys are not bounded by `c_*`.
pub fn key_step(ideal: [f32; 3], mut held: impl FnMut(Key) -> bool) -> [f32; 3] {
    let [mut pitch, mut yaw, mut dist] = ideal;
    if held(Key::PitchUp) {
        pitch += 2.5;
    } else if held(Key::PitchDown) {
        pitch -= 2.5;
    }
    if held(Key::YawLeft) {
        yaw -= 2.5;
    } else if held(Key::YawRight) {
        yaw += 2.5;
    }
    if held(Key::In) {
        dist -= 1.0;
        // Pulled in past the minimum: back to straight behind at 30 units.
        if dist < 30.0 {
            (pitch, yaw, dist) = (0.0, 0.0, 30.0);
        }
    } else if held(Key::Out) {
        dist += 1.0;
    }
    [pitch, yaw, dist]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    PitchUp,
    PitchDown,
    YawLeft,
    YawRight,
    In,
    Out,
}

/// The mouse and key adjustments of `CAM_Think` (`0x10024cdc..0x1002515d`) to the ideal
/// (pitch, yaw, dist), read from and written back to `cam_ideal*` by the caller. Only called in
/// third person. In `+cammousemove` the cursor turns the camera, half a degree per two pixels; in
/// `+camdistance` its vertical offset sets the distance, a unit per two pixels.
pub(crate) fn adjust_ideal(ideal: [f32; 3], lim: Limits) -> [f32; 3] {
    let mut m = MOUSE.0.get();
    let [mut pitch, mut yaw, mut dist] = ideal;
    if m.mousemove {
        // GetCursorPos, then SetCursorPos(centre) in either branch below: read and re-centre.
        let d = crate::input::take_cursor_offset();
        let d = [d[0] + m.frac[0], d[1] + m.frac[1]];
        m.cursor = [d[0].trunc() as i32, d[1].trunc() as i32];
        m.frac = [d[0].fract(), d[1].fract()];
        if !m.distancemove {
            yaw = mouse_step(yaw, m.cursor[0], lim.yaw, 0.5);
            pitch = mouse_step(pitch, m.cursor[1], lim.pitch, 0.5);
        }
    } else if m.distancemove {
        // `-cammousemove` with `+camdistance` still held: TS doesn't read the cursor, but the
        // distance block still re-centres it (`SetCursorPos`, `0x10025138`), and with mouse look
        // back on that throws away the motion since the last re-centre. Drop it the same way.
        crate::input::take_cursor_offset();
    }
    [pitch, yaw, dist] = key_step([pitch, yaw, dist], |k| {
        let b = match k {
            Key::PitchUp => &PITCH_UP,
            Key::PitchDown => &PITCH_DOWN,
            Key::YawLeft => &YAW_LEFT,
            Key::YawRight => &YAW_RIGHT,
            Key::In => &CAM_IN,
            Key::Out => &CAM_OUT,
        };
        b.key_state() != 0.0
    });
    if m.distancemove {
        dist = mouse_step(dist, m.cursor[1], lim.dist, 1.0);
    }
    MOUSE.0.set(m);
    [pitch, yaw, dist]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_step_halves_and_clamps() {
        let b = (-135.0, 135.0);
        // 7 px right: 7/2 = 3 (integer), * 0.5 = 1.5 degrees.
        assert_eq!(mouse_step(10.0, 7, b, 0.5), 11.5);
        assert_eq!(mouse_step(10.0, -7, b, 0.5), 8.5);
        // One pixel is lost to the halving.
        assert_eq!(mouse_step(10.0, 1, b, 0.5), 10.0);
        assert_eq!(mouse_step(10.0, 0, b, 0.5), 10.0);
        // Clamped at the bound it moves towards.
        assert_eq!(mouse_step(134.0, 100, b, 0.5), 135.0);
        assert_eq!(mouse_step(-134.0, -100, b, 0.5), -135.0);
        // Outside the bound it was moving away from: only the bound it moves towards applies.
        assert_eq!(mouse_step(150.0, -4, b, 0.5), 149.0);
        assert_eq!(mouse_step(150.0, 4, b, 0.5), 135.0);
        // Distance: one unit per two pixels, bounded by c_min/maxdistance.
        assert_eq!(mouse_step(100.0, 20, (30.0, 200.0), 1.0), 110.0);
        assert_eq!(mouse_step(35.0, -20, (30.0, 200.0), 1.0), 30.0);
        // Extreme cursor offsets (a huge or infinite mouse delta saturates to these) and bounds.
        assert_eq!(mouse_step(0.0, i32::MIN, (-1e12, 0.0), 1.0), (i32::MIN / 2) as f32);
        mouse_step(0.0, i32::MAX, b, 0.5);
        mouse_step(f32::NAN, i32::MIN, (f32::NAN, f32::NAN), f32::INFINITY);
    }

    #[test]
    fn key_step_matches_cam_think() {
        let none = |_| false;
        assert_eq!(key_step([0.0, 0.0, 100.0], none), [0.0, 0.0, 100.0]);
        assert_eq!(key_step([0.0, 0.0, 100.0], |k| k == Key::PitchUp), [2.5, 0.0, 100.0]);
        assert_eq!(key_step([0.0, 0.0, 100.0], |k| k == Key::PitchDown), [-2.5, 0.0, 100.0]);
        assert_eq!(key_step([0.0, 0.0, 100.0], |k| k == Key::YawLeft), [0.0, -2.5, 100.0]);
        assert_eq!(key_step([0.0, 0.0, 100.0], |k| k == Key::YawRight), [0.0, 2.5, 100.0]);
        assert_eq!(key_step([0.0, 0.0, 100.0], |k| k == Key::Out), [0.0, 0.0, 101.0]);
        assert_eq!(key_step([5.0, 5.0, 100.0], |k| k == Key::In), [5.0, 5.0, 99.0]);
        // Under 30 resets the angles too. Keys aren't bounded by c_max*.
        assert_eq!(key_step([5.0, 5.0, 30.5], |k| k == Key::In), [0.0, 0.0, 30.0]);
        assert_eq!(key_step([89.0, 0.0, 300.0], |k| matches!(k, Key::PitchUp | Key::Out)), [91.5, 0.0, 301.0]);
        // `else if`: the second button of a pair isn't read (its impulse bits survive).
        let mut asked = Vec::new();
        key_step([0.0; 3], |k| {
            asked.push(k);
            true
        });
        assert_eq!(asked, [Key::PitchUp, Key::YawLeft, Key::In]);
    }
}
