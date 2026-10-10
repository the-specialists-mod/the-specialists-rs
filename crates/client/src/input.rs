//! Keyboard buttons, mouse look and `CL_CreateMove` (TS `InitInput` `10026b30`, `CL_CreateMove`
//! `10026570`, `CL_ButtonBits` `10026870`). Reference: decomp docs/client/architecture-hud.md §5.
//!
//! Mouse input has two back ends, picked at run time:
//! * **FWGS** (detected by its `host_ver` cvar): the engine reads the mouse and hands relative
//!   motion to our `IN_ClientLookEvent` export (an FWGS extension; when it is exported the engine
//!   no longer adds mouse look itself in `IN_EngineAppendMove`). Joystick/touch movement arrives in
//!   `IN_ClientMoveEvent`.
//! * **Steam GoldSrc** (pre- and post-anniversary): the client reads the mouse itself like the
//!   SDK's `inputw32.cpp`: on Windows `GetCursorPos`/`SetCursorPos` re-centering, or SDL2
//!   relative motion when `m_rawinput` is on; on Linux SDL2 relative motion. SDL2 entry points are
//!   looked up at run time in the already-loaded SDL2 library (no link-time dependency).
//!
//! Either way the deltas go through the same TS scaling (`m_filter`, TS acceleration, zoom or
//! `sensitivity`, `m_yaw`/`m_pitch`). While a menu shows the cursor, they move a virtual cursor
//! instead of the view (TS keeps such a cursor at `0x1018d17c`).

use crate::hud;
use goldsrc::Global;
use goldsrc::client::EngineCell;
use goldsrc::client::engine;
use goldsrc::sys::*;
use std::cell::{Cell, RefCell};
use std::ffi::{CStr, c_int, c_void};

/// SDK `kbutton_t`: the engine reads `state` through `KB_Find`.
#[repr(C)]
pub struct KButton {
    down: [c_int; 2],
    /// bit 0: held, bit 1: pressed since last frame, bit 2: released since last frame.
    state: c_int,
}

/// A `kbutton_t`. `KB_Find` hands the engine a pointer to it, so it lives in an [`EngineCell`].
pub struct Button(EngineCell<KButton>);

impl Button {
    const fn new() -> Self {
        Button(EngineCell::new(KButton { down: [0; 2], state: 0 }))
    }

    #[allow(clippy::mut_from_ref)]
    fn get(&self) -> &mut KButton {
        self.0.get()
    }

    fn key_arg() -> c_int {
        let c = engine().cmd_argv(1);
        if c.is_empty() { -1 } else { c.to_str().ok().and_then(|s| s.parse().ok()).unwrap_or(-1) }
    }

    /// SDK `KeyDown`.
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

    /// SDK `KeyUp`.
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

    /// Down at any point since the last `CL_ButtonBits` reset.
    pub(crate) fn held(&self) -> bool {
        self.get().state & 3 != 0
    }

    /// SDK `CL_KeyState`: fraction of the frame the key was down (0, 0.25, 0.5, 0.75 or 1).
    fn key_state(&self) -> f32 {
        let b = self.get();
        let (impulse_down, impulse_up, down) = (b.state & 2 != 0, b.state & 4 != 0, b.state & 1 != 0);
        let val = match (impulse_down, impulse_up, down) {
            (false, false, d) => d as i32 as f32,
            (true, false, true) => 0.5,
            (true, false, false) => 0.0,
            (false, true, false) => 0.0,
            (false, true, true) => 0.0,
            (true, true, false) => 0.25,
            (true, true, true) => 0.75,
        };
        b.state &= 1;
        val
    }

    /// Release the button as if its keys went up (SDK `CL_ResetButtonBits`).
    fn release(&self) {
        let b = self.get();
        b.state &= !1;
    }

    fn ptr(&self) -> *mut c_void {
        self.0.as_ptr().cast()
    }
}

macro_rules! buttons {
    ($($ident:ident, $name:literal;)*) => {
        $(pub static $ident: Button = Button::new();)*

        /// `KB_Find`: the engine looks buttons up by these names (TS registers `in_graph`,
        /// `in_mlook` and `in_jlook`; the rest are found the same way for completeness).
        pub fn find(name: &CStr) -> *mut c_void {
            $(if name == $name { return $ident.ptr(); })*
            std::ptr::null_mut()
        }
    };
}

buttons! {
    IN_KLOOK, c"in_klook";
    IN_JLOOK, c"in_jlook";
    IN_MLOOK, c"in_mlook";
    IN_GRAPH, c"in_graph";
    IN_BREAK, c"in_break";
    IN_STRAFE, c"in_strafe";
    IN_LEFTB, c"in_left";
    IN_RIGHTB, c"in_right";
    IN_FWD, c"in_forward";
    IN_BK, c"in_back";
    IN_LOOKUP, c"in_lookup";
    IN_LOOKDOWN, c"in_lookdown";
    IN_MOVELEFTB, c"in_moveleft";
    IN_MOVERIGHTB, c"in_moveright";
    IN_UPK, c"in_moveup";
    IN_DOWNK, c"in_movedown";
    IN_SPEED, c"in_speed";
    IN_JMP, c"in_jump";
    IN_DCK, c"in_duck";
    IN_ATK, c"in_attack";
    IN_ATK2, c"in_attack2";
    IN_USEK, c"in_use";
    IN_RELOD, c"in_reload";
    IN_ALT1B, c"in_alt1";
    IN_ALT2B, c"in_alt2";
    IN_SCOREB, c"in_score";
}

// ---------------------------------------------------------------------------------------------
// Anti-script gate (§5.3): `HUD_Key_Event` sets it when a physical key's binding is one of the
// gated commands; `HUD_Frame` clears it. Gated `+` handlers do nothing without it.

static REAL_KEY: Global<Cell<bool>> = Global(Cell::new(false));

/// The bindings that set the gate, exactly as TS compares them.
pub const GATED_BINDINGS: [&str; 9] = ["+attack", "+attack2", "+alt2", "+reload", "+forward", "+back", "+moveleft", "+moveright", "+jump"];

/// `ts_dev_scriptinput 1` (port-only development aid): with `sv_cheats 1`, scripted `+attack`,
/// `+jump`, … work without a physical key, for automated in-engine tests.
static DEV_SCRIPT_INPUT: Global<Cell<Cvar>> = Global(Cell::new(Cvar::NULL));

pub fn gate_open() -> bool {
    REAL_KEY.0.get() || (DEV_SCRIPT_INPUT.0.get().value() != 0.0 && cvar_ptr_value(c"sv_cheats") != 0.0)
}

/// TS `FUN_10023450`: gated `+forward`/`+back`/`+moveleft`/`+moveright`/`+attack`/`+attack2`/
/// `+jump` presses and `+duck` share one 0.2 s timer. When the timer has run out it restarts, and
/// a press with bit 0 set (`+attack` only) sends `respawn`. The server ignores it unless the
/// player is an observer whose respawn wait is over, so this is how a click spawns you in.
static NEXT_KEY_TAP: Global<Cell<f32>> = Global(Cell::new(0.0));

fn key_tap(bits: u8) {
    let now = engine().client_time();
    if key_tap_step(&NEXT_KEY_TAP.0, now, bits) == Some(true) {
        engine().server_cmd("respawn");
    }
}

/// Pure part of `key_tap`: `None` while the timer runs, else whether to send `respawn`.
fn key_tap_step(next: &Cell<f32>, now: f32, bits: u8) -> Option<bool> {
    if next.get() > now {
        return None;
    }
    next.set(now + 0.2);
    Some(bits & 1 != 0)
}

fn cvar_ptr_value(name: &CStr) -> f32 {
    Cvar::find(name).value()
}

/// Pure part of `HUD_Key_Event`'s gate check (used by tests).
pub fn binding_is_gated(binding: &str) -> bool {
    GATED_BINDINGS.contains(&binding)
}

// ---------------------------------------------------------------------------------------------
// Cvars

/// Cvar handle (`cvar_t*` owned by the engine).
pub use goldsrc::client::Cvar;

macro_rules! cvars {
    ($($field:ident = $name:literal, $default:literal, $flags:expr;)*) => {
        #[derive(Clone, Copy)]
        pub struct InputCvars { $(pub $field: Cvar,)* }
        impl InputCvars {
            const fn null() -> Self { InputCvars { $($field: Cvar::NULL,)* } }
            fn register() -> Self {
                InputCvars { $($field: Cvar::register($name, $default, $flags),)* }
            }
        }
    };
}

const ARCHIVE: i32 = 1;

// `InitInput` 10026b30 and `IN_Init` 10028320 registrations (architecture-hud.md §2.1), in order.
cvars! {
    lookstrafe = c"lookstrafe", c"0", ARCHIVE;
    lookspring = c"lookspring", c"0", ARCHIVE;
    cl_anglespeedkey = c"cl_anglespeedkey", c"0.67", 0;
    cl_yawspeed = c"cl_yawspeed", c"210", 0;
    cl_pitchspeed = c"cl_pitchspeed", c"225", 0;
    cl_upspeed = c"cl_upspeed", c"320", 0;
    cl_forwardspeed = c"cl_forwardspeed", c"400", ARCHIVE;
    cl_backspeed = c"cl_backspeed", c"400", ARCHIVE;
    cl_sidespeed = c"cl_sidespeed", c"400", 0;
    cl_movespeedkey = c"cl_movespeedkey", c"0.3", 0;
    cl_vsmoothing = c"cl_vsmoothing", c"0.05", ARCHIVE;
    m_pitch = c"m_pitch", c"0.022", ARCHIVE;
    m_yaw = c"m_yaw", c"0.022", ARCHIVE;
    m_forward = c"m_forward", c"1", ARCHIVE;
    m_side = c"m_side", c"0.8", ARCHIVE;
    cl_fx = c"cl_fx", c"1", ARCHIVE;
    cam_back = c"cam_back", c"90", ARCHIVE;
    cam_right = c"cam_right", c"0", ARCHIVE;
    cam_up = c"cam_up", c"12", ARCHIVE;
    cl_camtype = c"cl_camtype", c"0", ARCHIVE;
    hud_r = c"hud_r", c"150", ARCHIVE;
    hud_g = c"hud_g", c"175", ARCHIVE;
    hud_b = c"hud_b", c"255", ARCHIVE;
    hud_color = c"hud_color", c"150,175,255", ARCHIVE;
    cl_lasercolor = c"cl_lasercolor", c"10", ARCHIVE;
    cl_buykevlar = c"cl_buykevlar", c"1", ARCHIVE;
    nohud = c"nohud", c"0", 0;
    cl_realhud = c"cl_realhud", c"0", ARCHIVE;
    cl_shadow = c"cl_shadow", c"2", ARCHIVE;
    cl_wavedetail = c"cl_wavedetail", c"0", ARCHIVE;
    cl_thirdpersonswitch = c"cl_thirdpersonswitch", c"0", ARCHIVE;
    cl_usepowerup = c"cl_usepowerup", c"1", ARCHIVE;
    cl_laser = c"cl_laser", c"0", ARCHIVE;
    cl_solidcross = c"cl_solidcross", c"2", ARCHIVE;
    cl_thirdperson = c"cl_thirdperson", c"0", ARCHIVE;
    cl_attackswitch = c"cl_attackswitch", c"0", ARCHIVE;
    cl_thirdtransparency = c"cl_thirdtransparency", c"1", ARCHIVE;
    cl_motionblur = c"cl_motionblur", c"1", ARCHIVE;
    cl_gore = c"cl_gore", c"2", ARCHIVE;
    cl_showequipment = c"cl_showequipment", c"1", ARCHIVE;
    m_filter = c"m_filter", c"0", ARCHIVE;
    sensitivity = c"sensitivity", c"3", ARCHIVE;
    m_accel = c"m_accel", c"0", ARCHIVE;
    m_accelexp = c"m_accelexp", c"1.2", ARCHIVE;
    m_accelmax = c"m_accelmax", c"0", ARCHIVE;
    joystick = c"joystick", c"0", ARCHIVE;
    joyname = c"joyname", c"joystick", 0;
    joyadvanced = c"joyadvanced", c"0", 0;
    joyadvaxisx = c"joyadvaxisx", c"0", 0;
    joyadvaxisy = c"joyadvaxisy", c"0", 0;
    joyadvaxisz = c"joyadvaxisz", c"0", 0;
    joyadvaxisr = c"joyadvaxisr", c"0", 0;
    joyadvaxisu = c"joyadvaxisu", c"0", 0;
    joyadvaxisv = c"joyadvaxisv", c"0", 0;
    joyforwardthreshold = c"joyforwardthreshold", c"0.15", 0;
    joysidethreshold = c"joysidethreshold", c"0.15", 0;
    joypitchthreshold = c"joypitchthreshold", c"0.15", 0;
    joyyawthreshold = c"joyyawthreshold", c"0.15", 0;
    joyforwardsensitivity = c"joyforwardsensitivity", c"-1.0", 0;
    joysidesensitivity = c"joysidesensitivity", c"-1.0", 0;
    joypitchsensitivity = c"joypitchsensitivity", c"1.0", 0;
    joyyawsensitivity = c"joyyawsensitivity", c"-1.0", 0;
    joywwhack1 = c"joywwhack1", c"0.0", 0;
    joywwhack2 = c"joywwhack2", c"0.0", 0;
}

static CVARS: Global<Cell<InputCvars>> = Global(Cell::new(InputCvars::null()));

pub fn cvars() -> InputCvars {
    CVARS.0.get()
}

/// HUD colour from `hud_r/g/b`, used by every TS draw.
pub fn hud_rgb() -> [i32; 3] {
    let c = cvars();
    [c.hud_r.value() as i32, c.hud_g.value() as i32, c.hud_b.value() as i32]
}

// ---------------------------------------------------------------------------------------------
// Commands

/// `cl_attackswitch` (§5.1): swap `+attack2` and `+alt2` when set and a weapon other than kung fu,
/// grenades, knives or the katana is held (TS weapon kinds 2, 3, 10, 11 keep the direct mapping).
pub fn attack_switched() -> bool {
    if cvars().cl_attackswitch.value() == 0.0 {
        return false;
    }
    let id = crate::state::with(|s| s.weapon);
    if id <= 0 {
        return false;
    }
    !matches!(weapon_kind(id), 2 | 3 | 10 | 11)
}

/// TS weapon kind byte (`info+0x348`, server weapons.md §2) from the shared table.
pub fn weapon_kind(id: i32) -> i32 {
    use shared::weapons::WeaponKind as K;
    let Some(w) = shared::weapons::tables().weapon(id as u8) else { return 11 };
    match w.kind {
        K::Kungfu => 0,
        K::Shotgun => 1,
        K::Grenade => 2,
        K::Knife => 3,
        K::Akimbo => 4,
        K::AkimboOnly => 5,
        K::Rifle => 6,
        K::Sniper => 7,
        K::Pistol => 8,
        K::Smg => 9,
        K::Katana => 10,
    }
}

macro_rules! cmd {
    ($name:literal, $body:expr) => {{
        extern "C" fn f() {
            $body
        }
        engine().add_command($name, f);
    }};
}

fn register_commands() {
    // Order follows InitInput (architecture-hud.md §2.2).
    cmd!(c"+moveup", IN_UPK.key_down());
    cmd!(c"-moveup", IN_UPK.key_up());
    cmd!(c"+movedown", IN_DOWNK.key_down());
    cmd!(c"-movedown", IN_DOWNK.key_up());
    cmd!(c"+left", IN_LEFTB.key_down());
    cmd!(c"-left", IN_LEFTB.key_up());
    cmd!(c"+right", IN_RIGHTB.key_down());
    cmd!(c"-right", IN_RIGHTB.key_up());
    // Gated movement.
    cmd!(c"+forward", {
        if gate_open() {
            IN_FWD.key_down();
            key_tap(0x08);
        }
    });
    cmd!(c"-forward", IN_FWD.key_up());
    cmd!(c"+back", {
        if gate_open() {
            IN_BK.key_down();
            key_tap(0x10);
        }
    });
    cmd!(c"-back", IN_BK.key_up());
    cmd!(c"+moveleft", {
        if gate_open() {
            IN_MOVELEFTB.key_down();
            key_tap(0);
        }
    });
    cmd!(c"-moveleft", IN_MOVELEFTB.key_up());
    cmd!(c"+moveright", {
        if gate_open() {
            IN_MOVERIGHTB.key_down();
            key_tap(0);
        }
    });
    cmd!(c"-moveright", IN_MOVERIGHTB.key_up());
    cmd!(c"+lookup", IN_LOOKUP.key_down());
    cmd!(c"-lookup", IN_LOOKUP.key_up());
    cmd!(c"+lookdown", IN_LOOKDOWN.key_down());
    cmd!(c"-lookdown", IN_LOOKDOWN.key_up());
    cmd!(c"+speed", IN_SPEED.key_down());
    cmd!(c"-speed", IN_SPEED.key_up());
    cmd!(c"+attack", {
        if gate_open() {
            IN_ATK.key_down();
            key_tap(0x01);
        }
    });
    cmd!(c"-attack", IN_ATK.key_up());
    // +attack2 / +alt2 drive one of two kbuttons depending on cl_attackswitch. TS's +attack2 also
    // runs `special` (`_special` to the server through the viewport).
    cmd!(c"+attack2", {
        if gate_open() {
            if attack_switched() {
                IN_ALT2B.key_down()
            } else {
                IN_ATK2.key_down()
            }
            engine().server_cmd("_special");
            key_tap(0);
        }
    });
    cmd!(c"-attack2", {
        IN_ATK2.key_up();
        IN_ALT2B.key_up();
    });
    cmd!(c"+use", IN_USEK.key_down());
    cmd!(c"-use", IN_USEK.key_up());
    cmd!(c"+jump", {
        if gate_open() {
            IN_JMP.key_down();
            key_tap(0x02);
        }
    });
    cmd!(c"-jump", IN_JMP.key_up());
    cmd!(c"impulse", {
        let n: i32 = engine().cmd_argv(1).to_str().ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
        IMPULSE.0.set(n);
    });
    cmd!(c"+klook", IN_KLOOK.key_down());
    cmd!(c"-klook", IN_KLOOK.key_up());
    cmd!(c"+jlook", IN_JLOOK.key_down());
    cmd!(c"-jlook", IN_JLOOK.key_up());
    cmd!(c"+duck", {
        IN_DCK.key_down();
        key_tap(0x04);
    });
    cmd!(c"-duck", IN_DCK.key_up());
    cmd!(
        c"+reload",
        if gate_open() {
            IN_RELOD.key_down()
        }
    );
    cmd!(c"-reload", IN_RELOD.key_up());
    cmd!(c"+alt1", IN_ALT1B.key_down());
    cmd!(c"-alt1", IN_ALT1B.key_up());
    cmd!(c"+alt2", {
        if gate_open() {
            if attack_switched() { IN_ATK2.key_down() } else { IN_ALT2B.key_down() }
        }
    });
    cmd!(c"-alt2", {
        IN_ALT2B.key_up();
        IN_ATK2.key_up();
    });
    cmd!(c"+score", {
        IN_SCOREB.key_down();
        crate::menus::show_scores(true);
    });
    cmd!(c"-score", {
        IN_SCOREB.key_up();
        crate::menus::show_scores(false);
    });
    cmd!(c"+showscores", {
        IN_SCOREB.key_down();
        crate::menus::show_scores(true);
    });
    cmd!(c"-showscores", {
        IN_SCOREB.key_up();
        crate::menus::show_scores(false);
    });
    cmd!(c"+graph", IN_GRAPH.key_down());
    cmd!(c"-graph", IN_GRAPH.key_up());
    cmd!(c"+break", IN_BREAK.key_down());
    cmd!(c"-break", IN_BREAK.key_up());
}

static IMPULSE: Global<Cell<i32>> = Global(Cell::new(0));

// ---------------------------------------------------------------------------------------------
// Mouse

#[derive(Default)]
struct Mouse {
    is_fwgs: bool,
    initialized: bool,
    active: bool,
    old_buttons: i32,
    accum: [f32; 2],
    old: [f32; 2],
    /// Joystick/touch movement from FWGS `IN_ClientMoveEvent`.
    move_accum: [f32; 2],
    /// Virtual cursor (TS `0x1018d17c/180`), used while a menu shows the cursor.
    cursor: [f32; 2],
    #[cfg(windows)]
    win: goldsrc::platform::win::State,
}

static MOUSE: Global<RefCell<Option<Mouse>>> = Global(RefCell::new(None));

/// The closure must not call back into this module (it would hit the `RefCell` borrow).
fn mouse<R>(f: impl FnOnce(&mut Mouse) -> R) -> R {
    let mut m = MOUSE.0.borrow_mut();
    f(m.get_or_insert_with(Mouse::default))
}

/// The virtual cursor position, clamped to the screen.
pub fn cursor() -> (i32, i32) {
    mouse(|m| (m.cursor[0] as i32, m.cursor[1] as i32))
}

/// Centre the virtual cursor (TS does this when the buy menu opens).
pub fn center_cursor() {
    let s = crate::hud::screen();
    mouse(|m| m.cursor = [s.width as f32 / 2.0, s.height as f32 / 2.0]);
}

/// Whether the engine is Xash3D FWGS (it has the `host_ver` cvar; Steam GoldSrc doesn't).
pub fn is_fwgs() -> bool {
    mouse(|m| m.is_fwgs)
}

/// TS mouse acceleration (`IN_MouseMove` 100274d0 via 1006c490):
/// `delta += sign * min(|delta|^m_accelexp * m_accel, m_accelmax)`; m_accelmax 0 = no cap.
pub fn accelerate(d: f32, accel: f32, exp: f32, max: f32) -> f32 {
    if accel <= 0.0 || d == 0.0 {
        return d;
    }
    let mut add = d.abs().powf(exp) * accel;
    if max > 0.0 {
        add = add.min(max);
    }
    d + d.signum() * add
}

/// Scale raw mouse counts to view-angle deltas and apply them (or move the virtual cursor).
fn apply_mouse(raw: [f32; 2], view: &mut [f32; 3]) {
    let c = cvars();
    let (mut mx, mut my) = (raw[0], raw[1]);
    if crate::menus::wants_cursor() {
        let s = crate::hud::screen();
        mouse(|m| {
            m.cursor[0] = (m.cursor[0] + mx).clamp(0.0, (s.width - 1) as f32);
            m.cursor[1] = (m.cursor[1] + my).clamp(0.0, (s.height - 1) as f32);
        });
        return;
    }
    if c.m_filter.value() != 0.0 {
        let old = mouse(|m| std::mem::replace(&mut m.old, [mx, my]));
        mx = (mx + old[0]) * 0.5;
        my = (my + old[1]) * 0.5;
    } else {
        mouse(|m| m.old = [mx, my]);
    }
    let zoom = crate::hud::zoom_sensitivity();
    if zoom != 0.0 {
        mx *= zoom;
        my *= zoom;
    } else {
        let (a, e, x) = (c.m_accel.value(), c.m_accelexp.value(), c.m_accelmax.value());
        mx = accelerate(mx, a, e, x);
        my = accelerate(my, a, e, x);
        let sens = c.sensitivity.value().clamp(0.01, 10000.0);
        mx *= sens;
        my *= sens;
    }
    view[YAW] -= c.m_yaw.value() * mx;
    view[PITCH] = (view[PITCH] + c.m_pitch.value() * my).clamp(-89.0, 89.0);
}

const PITCH: usize = 0;
const YAW: usize = 1;
const ROLL: usize = 2;
const K_MOUSE1: i32 = 241;

/// Raw mouse counts since the last call, for the active back end.
fn take_mouse_delta() -> [f32; 2] {
    let (fwgs, active) = mouse(|m| (m.is_fwgs, m.active));
    if fwgs {
        // FWGS: already accumulated by IN_ClientLookEvent (in degrees: yaw = -x*m_yaw, pitch =
        // y*m_pitch). Convert back to counts so the TS scaling applies uniformly.
        let c = cvars();
        let a = mouse(|m| std::mem::take(&mut m.accum));
        let (my, mp) = (c.m_yaw.value(), c.m_pitch.value());
        return [if my != 0.0 { -a[0] / my } else { 0.0 }, if mp != 0.0 { a[1] / mp } else { 0.0 }];
    }
    if !active {
        return [0.0; 2];
    }
    let mut d = mouse(|m| std::mem::take(&mut m.accum));
    let now = native::delta();
    d[0] += now[0];
    d[1] += now[1];
    d
}

/// FWGS `IN_ClientLookEvent` (exported by the dll crate): relative mouse look (degrees, already
/// scaled by m_yaw/m_pitch).
pub fn look_event(relyaw: f32, relpitch: f32) {
    mouse(|m| {
        m.accum[0] += relyaw;
        m.accum[1] += relpitch;
    });
}

/// FWGS `IN_ClientMoveEvent` (exported by the dll crate): joystick/touch movement (-1..1).
pub fn move_event(forwardmove: f32, sidemove: f32) {
    mouse(|m| {
        m.move_accum[0] += forwardmove;
        m.move_accum[1] += sidemove;
    });
}

pub fn activate_mouse() {
    mouse(|m| {
        if m.initialized && !m.is_fwgs {
            native::activate(m);
            m.active = true;
        } else if m.initialized {
            m.active = true;
        }
    });
}

pub fn deactivate_mouse() {
    mouse(|m| {
        if m.initialized {
            if !m.is_fwgs {
                native::deactivate(m);
            }
            m.active = false;
        }
    });
}

/// SDK `IN_MouseEvent`: forward button edges to the engine as K_MOUSE1.. keys, except that
/// clicks go to the open menu while it shows the cursor (TS routes them to `CTSBuyMenu`).
pub fn mouse_event(mstate: i32) {
    let old = mouse(|m| std::mem::replace(&mut m.old_buttons, mstate));
    // A press on the scoreboard enters voice squelch mode instead of firing.
    if mstate & !old != 0 && crate::menus::scoreboard_press() {
        return;
    }
    if crate::menus::wants_cursor() {
        let (x, y) = cursor();
        crate::menus::mouse_buttons(mstate, old, x, y);
        return;
    }
    for i in 0..5 {
        let (now, was) = (mstate & (1 << i) != 0, old & (1 << i) != 0);
        if now != was {
            engine().key_event(K_MOUSE1 + i, now);
        }
    }
}

pub fn clear_states() {
    mouse(|m| {
        if m.active {
            m.accum = [0.0; 2];
            m.old_buttons = 0;
        }
    });
}

/// SDK `IN_Accumulate`: collect motion between frames (Steam GoldSrc only).
pub fn accumulate() {
    let (fwgs, active) = mouse(|m| (m.is_fwgs, m.active));
    if fwgs || !active {
        return;
    }
    let d = native::delta();
    mouse(|m| {
        m.accum[0] += d[0];
        m.accum[1] += d[1];
    });
}

/// Platform mouse readers for Steam GoldSrc.
mod native {
    use super::Mouse;

    #[cfg(windows)]
    pub fn activate(m: &mut Mouse) {
        goldsrc::platform::win::activate(&mut m.win);
    }
    #[cfg(windows)]
    pub fn deactivate(m: &mut Mouse) {
        goldsrc::platform::win::deactivate(&mut m.win);
    }
    #[cfg(windows)]
    pub fn delta() -> [f32; 2] {
        goldsrc::platform::win::delta()
    }

    #[cfg(not(windows))]
    pub fn activate(_m: &mut Mouse) {
        goldsrc::platform::sdl::set_relative(true);
    }
    #[cfg(not(windows))]
    pub fn deactivate(_m: &mut Mouse) {
        goldsrc::platform::sdl::set_relative(false);
    }
    #[cfg(not(windows))]
    pub fn delta() -> [f32; 2] {
        goldsrc::platform::sdl::relative_delta()
    }
}

// ---------------------------------------------------------------------------------------------
// Frame / key entry points

pub fn init() {
    CVARS.0.set(InputCvars::register());
    DEV_SCRIPT_INPUT.0.set(Cvar::register(c"ts_dev_scriptinput", c"0", 0));
    register_commands();
    let fwgs = !engine().cvar_pointer(c"host_ver").is_null();
    mouse(|m| {
        m.is_fwgs = fwgs;
        // SDK IN_StartupMouse: `-nomouse` disables the mouse entirely.
        m.initialized = true;
        #[cfg(windows)]
        goldsrc::platform::win::startup(&mut m.win);
    });
    crate::joystick::init(fwgs);
    // TS removed +mlook: mouse look is always on. Report it to the engine through in_mlook.
    IN_MLOOK.get().state = 1;
    engine().print(if fwgs { "[ts] input: FWGS mouse (IN_ClientLookEvent)\n" } else { "[ts] input: client-side mouse (GoldSrc)\n" });
}

/// `IN_Commands` (10027ca0), from `HUD_UpdateClientData`: joystick buttons (Steam GoldSrc only).
pub fn commands() {
    if !is_fwgs() {
        crate::joystick::commands();
    }
}

/// `HUD_Frame`: TS clears the anti-script gate here (after the frame's commands ran), and the HUD
/// eases the slow-motion clock.
pub fn frame(time: f64) {
    REAL_KEY.0.set(false);
    hud::frame(time);
}

/// `HUD_Key_Event` (10025a80): TS forwards keydown 'a' to the weapon element as slot 11, then
/// opens the gate when the binding is one of the gated commands.
pub fn key_event(down: bool, key: i32, binding: &CStr) {
    if down && key == b'a' as i32 {
        hud::select_slot(11);
    }
    if down && binding_is_gated(binding.to_str().unwrap_or("")) {
        REAL_KEY.0.set(true);
    }
}

/// TS `CL_AdjustAngles` (10026330): keyboard turning with the anti-turn-bind clamps.
fn adjust_angles(frametime: f32, view: &mut [f32; 3]) {
    let c = cvars();
    let speed = if IN_SPEED.held() { frametime * c.cl_anglespeedkey.value().max(0.67) } else { frametime };
    let yawspeed = c.cl_yawspeed.value().min(200.0);
    let pitchspeed = c.cl_pitchspeed.value().min(220.0);
    if !IN_STRAFE.held() {
        view[YAW] -= speed * yawspeed * IN_RIGHTB.key_state();
        view[YAW] += speed * yawspeed * IN_LEFTB.key_state();
        view[YAW] = view[YAW].rem_euclid(360.0);
    }
    if IN_KLOOK.held() {
        crate::view::stop_pitch_drift();
        view[PITCH] -= speed * pitchspeed * IN_FWD.key_state();
        view[PITCH] += speed * pitchspeed * IN_BK.key_state();
    }
    let up = IN_LOOKUP.key_state();
    let down = IN_LOOKDOWN.key_state();
    view[PITCH] -= speed * pitchspeed * up;
    view[PITCH] += speed * pitchspeed * down;
    if up != 0.0 || down != 0.0 {
        crate::view::stop_pitch_drift();
    }
    view[PITCH] = view[PITCH].clamp(-89.0, 89.0);
    view[ROLL] = view[ROLL].clamp(-50.0, 50.0);
}

/// Last view angles while alive: TS freezes the view when dead (`0x100df578`).
static LIVE_ANGLES: Global<Cell<[f32; 3]>> = Global(Cell::new([0.0; 3]));
/// View angles held while a menu owns the mouse.
static MENU_ANGLES: Global<Cell<Option<[f32; 3]>>> = Global(Cell::new(None));

/// `CL_ButtonBits`: kbuttons to `usercmd_t::buttons`, with TS's bit map (§5.1). Clears the
/// pressed/released edges like the SDK.
pub fn button_bits(reset: bool) -> u32 {
    let mut bits = 0u32;
    let map: [(&Button, u32); 15] = [
        (&IN_ATK, IN_ATTACK),
        (&IN_DCK, IN_DUCK),
        (&IN_JMP, IN_JUMP),
        (&IN_FWD, IN_FORWARD),
        (&IN_BK, IN_BACK),
        (&IN_USEK, IN_USE),
        (&IN_ALT2B, shared::ts::IN_ALT2 as u32),
        (&IN_LEFTB, IN_LEFT),
        (&IN_RIGHTB, IN_RIGHT),
        (&IN_MOVELEFTB, IN_MOVELEFT),
        (&IN_MOVERIGHTB, IN_MOVERIGHT),
        (&IN_ATK2, IN_ATTACK2),
        (&IN_RELOD, IN_RELOAD),
        (&IN_ALT1B, IN_ALT1),
        (&IN_SCOREB, IN_SCORE),
    ];
    for (b, bit) in map {
        if b.held() {
            bits |= bit;
        }
        if reset {
            b.get().state &= !2;
        }
    }
    // +score is also forced while dead or in intermission (TS `0x100d5b20`).
    let (health, intermission) = (crate::state::with(|s| s.health), hud::intermission());
    if health < 1 || intermission {
        bits |= IN_SCORE;
    }
    bits
}

pub fn create_move(frametime: f32, cmd: &mut usercmd_t, active: bool) {
    let e = engine();
    let c = cvars();
    let mut view = e.view_angles();

    // A menu that owns the mouse keeps the view still and moves the virtual cursor instead.
    if crate::menus::wants_cursor() {
        let held = MENU_ANGLES.0.get().unwrap_or(view);
        MENU_ANGLES.0.set(Some(held));
        view = held;
    } else {
        MENU_ANGLES.0.set(None);
    }

    *cmd = usercmd_t { ..Default::default() };
    if active {
        adjust_angles(frametime, &mut view);
        let strafe = IN_STRAFE.held();
        if strafe {
            cmd.sidemove += c.cl_sidespeed.value() * IN_RIGHTB.key_state();
            cmd.sidemove -= c.cl_sidespeed.value() * IN_LEFTB.key_state();
        }
        cmd.sidemove += c.cl_sidespeed.value() * IN_MOVERIGHTB.key_state();
        cmd.sidemove -= c.cl_sidespeed.value() * IN_MOVELEFTB.key_state();
        cmd.upmove += c.cl_upspeed.value() * IN_UPK.key_state();
        cmd.upmove -= c.cl_upspeed.value() * IN_DOWNK.key_state();
        if !IN_KLOOK.held() {
            cmd.forwardmove += c.cl_forwardspeed.value() * IN_FWD.key_state();
            cmd.forwardmove -= c.cl_backspeed.value() * IN_BK.key_state();
        }
        if IN_SPEED.held() {
            let s = c.cl_movespeedkey.value();
            cmd.forwardmove *= s;
            cmd.sidemove *= s;
            cmd.upmove *= s;
        }
        // Joystick/touch movement from FWGS.
        let mv = mouse(|m| std::mem::take(&mut m.move_accum));
        cmd.forwardmove += mv[0] * c.cl_forwardspeed.value();
        cmd.sidemove += mv[1] * c.cl_sidespeed.value();

        // IN_Move: mouse look, then the joystick (Steam GoldSrc; FWGS sent it above). TS's
        // `IN_MouseMove` (100274d0) stops the pitch drift first, every frame, so the view never drifts
        // while the mouse is active.
        if mouse(|m| m.active) {
            crate::view::stop_pitch_drift();
        }
        let raw = take_mouse_delta();
        if !hud::intermission() {
            apply_mouse(raw, &mut view);
        }
        if !is_fwgs() {
            crate::joystick::joy_move(frametime, &mut view, &mut cmd.forwardmove, &mut cmd.sidemove);
        }
    } else {
        let _ = take_mouse_delta();
    }

    cmd.impulse = IMPULSE.0.replace(0) as u8;
    let mut bits = button_bits(true);
    // The weapon menu takes +attack / +attack2 as select / drop (TS CHudTSWeapons::Think).
    let consumed = hud::weapon_menu_buttons(bits);
    if consumed & IN_ATTACK != 0 {
        IN_ATK.release();
    }
    if consumed & IN_ATTACK2 != 0 {
        IN_ATK2.release();
    }
    bits &= !consumed;
    // The scoreboard's squelch mode takes the mouse: no +attack (`CL_CreateMove` 10026570).
    if crate::voice::in_squelch_mode() {
        bits &= !IN_ATTACK;
    }
    cmd.buttons = bits as u16;

    // Frozen angles while dead (TS: g_iAlive == 0 keeps the last live angles).
    if crate::state::with(|s| s.dead) && !hud::observer() {
        view = LIVE_ANGLES.0.get();
    } else {
        LIVE_ANGLES.0.set(view);
    }
    if crate::menus::wants_cursor()
        && let Some(a) = MENU_ANGLES.0.get()
    {
        view = a;
    }
    e.set_view_angles(view);
    cmd.viewangles = view;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_tap_sends_respawn_on_attack_only_and_throttles() {
        let next = Cell::new(0.0);
        assert_eq!(key_tap_step(&next, 1.0, 0x01), Some(true)); // +attack: respawn
        assert_eq!(key_tap_step(&next, 1.1, 0x01), None); // within 0.2 s
        assert_eq!(key_tap_step(&next, 1.25, 0x08), Some(false)); // +forward restarts the timer
        assert_eq!(key_tap_step(&next, 1.35, 0x01), None); // ...which also holds back +attack
        assert_eq!(key_tap_step(&next, 1.5, 0x01), Some(true));
    }

    #[test]
    fn gated_bindings_match_ts() {
        assert!(binding_is_gated("+attack"));
        assert!(binding_is_gated("+jump"));
        assert!(!binding_is_gated("+duck"));
        assert!(!binding_is_gated("+alt1"));
        assert!(!binding_is_gated("+attack;wait")); // literal comparison, like TS
    }

    #[test]
    fn ts_acceleration() {
        assert_eq!(accelerate(5.0, 0.0, 1.2, 0.0), 5.0);
        let a = accelerate(4.0, 0.5, 1.0, 0.0);
        assert!((a - 6.0).abs() < 1e-5);
        let a = accelerate(-4.0, 0.5, 1.0, 1.0);
        assert!((a + 5.0).abs() < 1e-5, "capped by m_accelmax");
    }

    #[test]
    fn key_state_fractions() {
        let b = Button::new();
        b.get().state = 1 | 2;
        assert_eq!(b.key_state(), 0.5);
        assert_eq!(b.get().state, 1);
        assert_eq!(b.key_state(), 1.0);
        b.get().state = 4;
        assert_eq!(b.key_state(), 0.0);
    }

    #[test]
    fn button_bits_layout() {
        assert_eq!(shared::ts::IN_ALT2 as u32, IN_CANCEL);
        assert_eq!(IN_ALT1, 0x4000);
        assert_eq!(IN_SCORE, 0x8000);
    }
}
