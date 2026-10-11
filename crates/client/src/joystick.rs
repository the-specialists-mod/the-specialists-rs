//! Joystick input for Steam GoldSrc clients: TS's copy of the SDK's inputw32.cpp joystick code
//! (`IN_StartupJoystick` 10027960, `Joy_AdvancedUpdate_f` 10027b00, `IN_Commands` 10027ca0,
//! `IN_ReadJoystick` 10027de0, `IN_JoyMove` 10027e40). FWGS reads joysticks itself and sends
//! movement through `IN_ClientMoveEvent`, so none of this runs there.
//!
//! The device is read by `goldsrc::joystick` (WinMM on Windows like TS, an SDL2 game controller
//! elsewhere); everything here is the TS logic on WinMM-style readings (axes 0..=65535).

use crate::input::{self, Cvar};
use goldsrc::Global;
use goldsrc::client::engine;
use goldsrc::joystick::{self as dev, MAX_AXES, POV_CENTERED, Reading, Startup};
use std::cell::RefCell;

/// SDK `K_JOY1`, `K_AUX1`, `K_AUX29` (keydefs.h).
pub const K_JOY1: i32 = 203;
pub const K_AUX1: i32 = 207;
pub const K_AUX29: i32 = 235;

/// What a joystick axis drives (SDK `AxisNada`..`AxisTurn`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Axis {
    #[default]
    Nada = 0,
    Forward = 1,
    Look = 2,
    Side = 3,
    Turn = 4,
}

impl Axis {
    fn from_cvar(v: u32) -> Axis {
        match v & 0xf {
            1 => Axis::Forward,
            2 => Axis::Look,
            3 => Axis::Side,
            4 => Axis::Turn,
            _ => Axis::Nada,
        }
    }
}

/// `dwAxisMap` and `dwControlMap` (true = `JOY_RELATIVE_AXIS`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AxisMap {
    pub axis: [Axis; MAX_AXES],
    pub relative: [bool; MAX_AXES],
}

/// `Joy_AdvancedUpdate_f` (10027b00): `joyadvanced 0` is X turn / Y forward; otherwise each
/// `joyadvaxis*` value is an `Axis` in the low nibble plus 0x10 for a relative axis.
pub fn axis_map(advanced: bool, advaxis: [f32; MAX_AXES]) -> AxisMap {
    let mut m = AxisMap::default();
    if !advanced {
        m.axis[0] = Axis::Turn;
        m.axis[1] = Axis::Forward;
        return m;
    }
    for (i, v) in advaxis.iter().enumerate() {
        let v = *v as i64 as u32;
        m.axis[i] = Axis::from_cvar(v);
        m.relative[i] = v & 0x10 != 0;
    }
    m
}

/// `dwFlags` for `joyGetPosEx`: buttons, POV, centred values, and the mapped axes.
pub fn read_flags(m: &AxisMap) -> u32 {
    m.axis.iter().zip(dev::RETURN_AXIS).filter(|(a, _)| **a != Axis::Nada).fold(dev::RETURN_BASE, |f, (_, bit)| f | bit)
}

/// The cvars and buttons `IN_JoyMove` reads, as plain values.
#[derive(Debug, Clone, Copy, Default)]
pub struct MoveParams {
    pub frametime: f32,
    /// `in_speed` held: `cl_movespeedkey` scales everything.
    pub speed_key: bool,
    pub movespeedkey: f32,
    pub jlook: bool,
    pub strafe: bool,
    pub advanced: bool,
    pub lookstrafe: bool,
    pub lookspring: bool,
    pub m_pitch: f32,
    pub wwhack2: bool,
    pub forward_threshold: f32,
    pub side_threshold: f32,
    pub pitch_threshold: f32,
    pub yaw_threshold: f32,
    pub forward_sensitivity: f32,
    pub side_sensitivity: f32,
    pub pitch_sensitivity: f32,
    pub yaw_sensitivity: f32,
    pub forwardspeed: f32,
    pub sidespeed: f32,
    pub pitchspeed: f32,
    pub yawspeed: f32,
}

/// `IN_JoyMove` (10027e40) on one reading: adds to `forward`/`side` and turns `view` (pitch,
/// yaw, roll). TS clamps pitch to ±89 at the end. Returns whether to call `V_StopPitchDrift`: a
/// look axis past its threshold stops the drift, and so does a quiet one without `lookspring`.
pub fn apply_axes(raw: &[u32; MAX_AXES], map: &AxisMap, p: &MoveParams, view: &mut [f32; 3], forward: &mut f32, side: &mut f32) -> bool {
    let mut stop_drift = false;
    let speed = if p.speed_key { p.movespeedkey } else { 1.0 };
    let aspeed = speed * p.frametime;
    for (i, &value) in raw.iter().enumerate() {
        let mut v = value as f32 - 32768.0;
        if p.wwhack2 && map.axis[i] == Axis::Turn {
            // Logitech WingMan Warrior: y = 300·(|x|/800)^1.3, capped at 14000.
            let t = (300.0 * (v.abs() / 800.0).powf(1.3)).min(14000.0);
            v = if v > 0.0 { t } else { -t };
        }
        let v = v / 32768.0;
        let relative = map.relative[i];
        match map.axis[i] {
            Axis::Forward => {
                if !p.advanced && p.jlook {
                    // Forward control becomes look control.
                    if v.abs() > p.pitch_threshold {
                        let d = v * p.pitch_sensitivity * aspeed * p.pitchspeed;
                        if p.m_pitch < 0.0 {
                            view[0] -= d;
                        } else {
                            view[0] += d;
                        }
                        stop_drift = true;
                    } else if !p.lookspring {
                        stop_drift = true;
                    }
                } else if v.abs() > p.forward_threshold {
                    *forward += v * p.forward_sensitivity * speed * p.forwardspeed;
                }
            }
            Axis::Look => {
                if p.jlook {
                    if v.abs() > p.pitch_threshold {
                        view[0] += if relative {
                            v * p.pitch_sensitivity * speed * 180.0
                        } else {
                            v * p.pitch_sensitivity * aspeed * p.pitchspeed
                        };
                        stop_drift = true;
                    } else if !p.lookspring {
                        stop_drift = true;
                    }
                }
            }
            Axis::Side => {
                if v.abs() > p.side_threshold {
                    *side += v * p.side_sensitivity * speed * p.sidespeed;
                }
            }
            Axis::Turn => {
                if p.strafe || (p.lookstrafe && p.jlook) {
                    // Turn control becomes side control.
                    if v.abs() > p.side_threshold {
                        *side -= v * p.side_sensitivity * speed * p.sidespeed;
                    }
                } else if v.abs() > p.yaw_threshold {
                    view[1] += if relative { v * p.yaw_sensitivity * speed * 180.0 } else { v * p.yaw_sensitivity * aspeed * p.yawspeed };
                }
            }
            Axis::Nada => {}
        }
    }
    view[0] = view[0].clamp(-89.0, 89.0);
    stop_drift
}

/// `IN_Commands` button edges: buttons 0-3 are `K_JOY1`+i, the rest `K_AUX1`+i (the SDK's
/// offset, so button 4 is AUX5).
pub fn button_events(old: u32, new: u32, count: u32) -> Vec<(i32, bool)> {
    let mut out = Vec::new();
    for i in 0..count.min(32) {
        let b = 1u32 << i;
        if new & b != old & b {
            let base = if i < 4 { K_JOY1 } else { K_AUX1 };
            out.push((base + i as i32, new & b != 0));
        }
    }
    out
}

/// `IN_Commands` POV: the four cardinal directions as bits (anything else is none).
pub fn pov_bits(pov: u32) -> u32 {
    match pov {
        POV_CENTERED => 0,
        0 => 1,
        9000 => 2,
        18000 => 4,
        27000 => 8,
        _ => 0,
    }
}

#[derive(Default)]
struct Joy {
    avail: bool,
    advanced_init: bool,
    /// `-nojoy` on the command line (or FWGS): never look for a device.
    disabled: bool,
    buttons: u32,
    has_pov: bool,
    old_buttons: u32,
    old_pov: u32,
    map: AxisMap,
    /// The last `joyGetPosEx` result (`ji`), which `IN_Commands` reads.
    last: Option<Reading>,
}

static JOY: Global<RefCell<Option<Joy>>> = Global(RefCell::new(None));

fn with<R>(f: impl FnOnce(&mut Joy) -> R) -> R {
    let mut j = JOY.0.borrow_mut();
    f(j.get_or_insert_with(Joy::default))
}

fn dprint(msg: &str) {
    let e = engine();
    if e.cvar_float(c"developer") != 0.0 {
        e.print(msg);
    }
}

fn apply_startup(s: Startup) {
    match s {
        Startup::Found(caps) => {
            let e = engine();
            if caps.name.is_empty() {
                e.print("joystick found\n\n")
            } else {
                e.print(&format!("joystick found {}\n\n", caps.name))
            };
            with(|j| {
                j.buttons = caps.buttons;
                j.has_pov = caps.has_pov;
                j.old_buttons = 0;
                j.old_pov = 0;
                j.avail = true;
                j.advanced_init = false;
            });
        }
        Startup::NotFound(why) => {
            with(|j| {
                j.avail = false;
                j.last = None;
            });
            dprint(&why);
        }
        Startup::Unchanged => {}
    }
}

/// `IN_Init`'s joystick part: `joyadvancedupdate` and `IN_StartupJoystick` (not on FWGS, and not
/// with `-nojoy`).
pub fn init(fwgs: bool) {
    engine().add_command(c"joyadvancedupdate", joy_advanced_update);
    let disabled = fwgs || engine().check_parm("-nojoy");
    with(|j| j.disabled = disabled);
    if disabled {
        return;
    }
    apply_startup(dev::startup(false));
}

extern "C" fn joy_advanced_update() {
    let c = input::cvars();
    let advanced = c.joyadvanced.value() != 0.0;
    if advanced {
        let name = engine().cvar_string(c"joyname");
        if name != "joystick" {
            engine().print(&format!("\n{name} configured\n\n"));
        }
    }
    let ax = [c.joyadvaxisx, c.joyadvaxisy, c.joyadvaxisz, c.joyadvaxisr, c.joyadvaxisu, c.joyadvaxisv].map(Cvar::value);
    let map = axis_map(advanced, ax);
    with(|j| j.map = map);
}

/// `IN_Commands` (10027ca0), from `HUD_UpdateClientData`: joystick buttons and POV as key events.
pub fn commands() {
    let (avail, last, count, has_pov, old, old_pov) = with(|j| (j.avail, j.last, j.buttons, j.has_pov, j.old_buttons, j.old_pov));
    if !avail {
        return;
    }
    // Before the first read, and after a failed one, TS's JOYINFOEX (`0x100df6b0`, zeroed by
    // `IN_ReadJoystick` before `joyGetPosEx`) holds zeros: no buttons, and POV 0, which is "up".
    let (buttons, pov) = last.map_or((0, 0), |r| (r.buttons, r.pov));
    let e = engine();
    for (key, down) in button_events(old, buttons, count) {
        e.key_event(key, down);
    }
    let mut pov_state = old_pov;
    if has_pov {
        pov_state = pov_bits(pov);
        for (i, down) in (0..4).filter_map(|i| {
            let b = 1 << i;
            (pov_state & b != old_pov & b).then_some((i, pov_state & b != 0))
        }) {
            e.key_event(K_AUX29 + i, down);
        }
    }
    with(|j| {
        j.old_buttons = buttons;
        j.old_pov = pov_state;
    });
}

/// `IN_JoyMove` (10027e40), from `CL_CreateMove` after the mouse: reads the device and applies
/// it to `view` and the move. Steam GoldSrc only.
pub fn joy_move(frametime: f32, view: &mut [f32; 3], forward: &mut f32, side: &mut f32) {
    if !with(|j| j.advanced_init) {
        joy_advanced_update();
        with(|j| j.advanced_init = true);
    }
    if with(|j| j.disabled) {
        return;
    }
    // SDL controllers come and go; WinMM is scanned once at startup like TS.
    apply_startup(dev::startup(true));
    let c = input::cvars();
    let (avail, map) = with(|j| (j.avail, j.map));
    if !avail || c.joystick.value() == 0.0 {
        return;
    }
    let Some(mut r) = dev::read(read_flags(&map)) else {
        with(|j| j.last = None);
        return;
    };
    if c.joywwhack1.value() != 0.0 {
        // `IN_ReadJoystick`: the WingMan Warrior's U axis is off by 100.
        // A DWORD in TS: wraps rather than overflowing on a bogus driver value.
        r.axes[4] = r.axes[4].wrapping_add(100);
    }
    with(|j| j.last = Some(r));
    let p = MoveParams {
        frametime,
        speed_key: input::IN_SPEED.held(),
        movespeedkey: c.cl_movespeedkey.value(),
        jlook: input::IN_JLOOK.held(),
        strafe: input::IN_STRAFE.held(),
        advanced: c.joyadvanced.value() != 0.0,
        lookstrafe: c.lookstrafe.value() != 0.0,
        lookspring: c.lookspring.value() != 0.0,
        m_pitch: c.m_pitch.value(),
        wwhack2: c.joywwhack2.value() != 0.0,
        forward_threshold: c.joyforwardthreshold.value(),
        side_threshold: c.joysidethreshold.value(),
        pitch_threshold: c.joypitchthreshold.value(),
        yaw_threshold: c.joyyawthreshold.value(),
        forward_sensitivity: c.joyforwardsensitivity.value(),
        side_sensitivity: c.joysidesensitivity.value(),
        pitch_sensitivity: c.joypitchsensitivity.value(),
        yaw_sensitivity: c.joyyawsensitivity.value(),
        forwardspeed: c.cl_forwardspeed.value(),
        sidespeed: c.cl_sidespeed.value(),
        pitchspeed: c.cl_pitchspeed.value(),
        yawspeed: c.cl_yawspeed.value(),
    };
    if apply_axes(&r.axes, &map, &p, view, forward, side) {
        crate::view::stop_pitch_drift();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> MoveParams {
        MoveParams {
            frametime: 0.1,
            movespeedkey: 0.3,
            m_pitch: 0.022,
            forward_threshold: 0.15,
            side_threshold: 0.15,
            pitch_threshold: 0.15,
            yaw_threshold: 0.15,
            forward_sensitivity: -1.0,
            side_sensitivity: -1.0,
            pitch_sensitivity: 1.0,
            yaw_sensitivity: -1.0,
            forwardspeed: 400.0,
            sidespeed: 400.0,
            pitchspeed: 225.0,
            yawspeed: 210.0,
            ..MoveParams::default()
        }
    }

    #[test]
    fn default_and_advanced_axis_maps() {
        let m = axis_map(false, [0.0; 6]);
        assert_eq!(m.axis[0], Axis::Turn);
        assert_eq!(m.axis[1], Axis::Forward);
        assert_eq!(read_flags(&m), 0x4c0 | 1 | 2);
        let m = axis_map(true, [3.0, 1.0, 0.0, 0x12 as f32, 0.0, 0.0]);
        assert_eq!(m.axis[0], Axis::Side);
        assert_eq!(m.axis[3], Axis::Look);
        assert!(m.relative[3] && !m.relative[0]);
        assert_eq!(read_flags(&m), 0x4c0 | 1 | 2 | 8);
    }

    #[test]
    fn stick_moves_and_turns_like_the_sdk() {
        let map = axis_map(false, [0.0; 6]);
        let p = params();
        // Stick full right (X = 65535) and full forward (Y = 0).
        let raw = [65535, 0, 32768, 32768, 32768, 32768];
        let (mut view, mut fwd, mut side) = ([0.0f32; 3], 0.0, 0.0);
        apply_axes(&raw, &map, &p, &mut view, &mut fwd, &mut side);
        assert!((fwd - 400.0).abs() < 0.1, "forward: -1 × -1 × 400 = {fwd}");
        let v = 32767.0 / 32768.0;
        assert!((view[1] - -v * 0.1 * 210.0).abs() < 1e-3, "yaw turns right: {}", view[1]);
        assert_eq!(side, 0.0);
        // Centred within the dead zone: nothing.
        let (mut view, mut fwd, mut side) = ([0.0f32; 3], 0.0, 0.0);
        apply_axes(&[33000; 6], &map, &p, &mut view, &mut fwd, &mut side);
        assert_eq!((view, fwd, side), ([0.0; 3], 0.0, 0.0));
    }

    #[test]
    fn strafe_and_jlook_redirect_axes() {
        let map = axis_map(false, [0.0; 6]);
        let mut p = params();
        p.strafe = true;
        p.jlook = true;
        let raw = [65535, 0, 32768, 32768, 32768, 32768];
        let (mut view, mut fwd, mut side) = ([10.0f32, 0.0, 0.0], 0.0, 0.0);
        apply_axes(&raw, &map, &p, &mut view, &mut fwd, &mut side);
        assert!(side > 399.0, "turn axis strafes: {side}");
        assert_eq!(fwd, 0.0, "forward axis looks instead");
        assert!((view[0] - (10.0 - 0.1 * 225.0)).abs() < 1e-3, "pitch: {}", view[0]);
        p.speed_key = true;
        let (mut view, mut fwd, mut side) = ([88.0f32, 0.0, 0.0], 0.0, 0.0);
        apply_axes(&[32768, 65535, 32768, 32768, 32768, 32768], &map, &p, &mut view, &mut fwd, &mut side);
        assert_eq!(view[0], 89.0, "pitch clamps at 89");
    }

    #[test]
    fn buttons_and_pov_become_keys() {
        assert_eq!(button_events(0, 0b1_0001, 32), vec![(K_JOY1, true), (K_AUX1 + 4, true)]);
        assert_eq!(button_events(0b1, 0, 32), vec![(K_JOY1, false)]);
        assert!(button_events(0, 1 << 5, 4).is_empty(), "only the device's buttons");
        assert_eq!(pov_bits(POV_CENTERED), 0);
        assert_eq!(pov_bits(9000), 2);
        assert_eq!(pov_bits(4500), 0);
    }

    /// Device readings, button counts and cvars of any value.
    #[test]
    fn garbage_readings_and_cvars() {
        for count in [0, 4, 32, 33, u32::MAX] {
            button_events(u32::MAX, 0, count);
            button_events(0, u32::MAX, count);
        }
        for v in [f32::NAN, f32::INFINITY, -f32::INFINITY, -1.0, 1e20, 4.0, 19.0] {
            let map = axis_map(true, [v; MAX_AXES]);
            for raw in [[0; MAX_AXES], [u32::MAX; MAX_AXES], [32768; MAX_AXES]] {
                let p = MoveParams { wwhack2: true, jlook: v.is_nan(), frametime: v, pitch_sensitivity: v, ..params() };
                let (mut view, mut fwd, mut side) = ([v; 3], 0.0, 0.0);
                apply_axes(&raw, &map, &p, &mut view, &mut fwd, &mut side);
            }
        }
    }
}
