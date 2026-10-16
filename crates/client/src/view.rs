//! View: V_CalcRefdef (bob, punch, recoil via cl_viewangles), third-person camera, slow-motion
//! tint and fades. Owner: rendering/prediction work package.
//! Reference: decomp docs/client/rendering-effects-prediction.md §6-7.
//!
//! `TSFade` is hooked here (the HUD package should not hook it, or should forward to
//! [`on_tsfade`]).

use crate::effects::psys::Camera;
use crate::{effects, state};
use goldsrc::Global;
use goldsrc::client::{Cvar, engine};
use goldsrc::ext::render::{self as r, Vec3, vec3 as v};
use goldsrc::sys::*;
use std::cell::{Cell, RefCell};

struct Cvars {
    bob: Cvar,
    waterdist: Cvar,
    slowfade: Cvar,
    camtype: Cvar,
    cam_back: Cvar,
    cam_right: Cvar,
    cam_up: Cvar,
    thirdtransparency: Cvar,
    vsmoothing: Cvar,
    centermove: Cvar,
    centerspeed: Cvar,
}

#[derive(Default)]
struct View {
    // clocks
    last_frame: f64,
    frametime: f32,
    /// `pparams->frametime` of the last `V_CalcRefdef` (`0x10191478`), which `UTIL_ApproachFOV`
    /// steps the zoom by.
    refdef_frametime: f32,
    bob: Bob,
    /// `v_idlescale` (`0x10191414`), the `V_AddIdle` scale: `CHud::UpdateClientData` sets it from
    /// the HUD's concussion level (`0x10024b63`), which only the TFC `Concuss` message sets.
    idlescale: f32,
    // smoothing
    /// The eye z's smoothing ([`ClientRules::eye_origin`](crate::gameplay::ClientRules::eye_origin)).
    eye_z: EyeZ,
    roll: f32,
    // kick (`0x10191d10` target, `0x101914f8` current) and the view-shake spring
    kick_target: [f32; 2],
    kick_cur: [f32; 2],
    /// `CTSGun+0x8c0`: the prediction time of the last kick, a float as in TS.
    last_shot: f32,
    shake: Shake,
    // `V_DriftPitch` state (SDK `pd`, `0x10191460..0x10191470`)
    drift: Drift,
    /// Local player `renderamt` being eased (`0x10097214`); 255 = opaque.
    local_amt: f32,
    /// `0x10190b50`: `pparams->viewangles` as the last normal view left them (after the
    /// third-person and view-entity overrides). The predicted weapon events are played back with
    /// these angles (`HUD_PlaybackEvent` `0x1001ad50`).
    playback_angles: Vec3,
    // `cl_vsmoothing` origin ring (SDK `ViewInterp`, `0x10191508`) and the last recorded simorg
    org_ring: Ring,
    last_simorg: Vec3,
    /// The `cl_viewangles` ring (`0x10191908`, times `0x10191c08`, index `0x10191d0c`) and the
    /// last angles stored in it (`0x10190b84`), smoothed only while `PM_SpectatorMove` sets the
    /// angle flag.
    ang_ring: Ring,
    last_cl_angles: Vec3,
    // third person
    /// `cam_thirdperson` (`0x100df390`), the `thirdperson` toggle's state. TS has no separate
    /// death cam: `TSHealth` 0 runs the toggle.
    third: bool,
    /// The smoothed additions to the camera's up and back offsets
    /// ([`ClientRules::camera_offsets`](crate::gameplay::ClientRules::camera_offsets)).
    cam_ofs_smooth: CamOffsets,
    /// Smoothed z of the local player model in third person (`0x10191d94`). A BSS static in TS:
    /// it starts at 0 and only changes in third person, so the model slides in from wherever it
    /// was when third person was last left (from z 0 the first time).
    model_z: f32,
    /// `cam_ofs` (`0x100df318`): the camera's (pitch, yaw, distance) from [`cam_think`]. A BSS
    /// static: zero until the `thirdperson` toggle (`0x1004fe00`) seeds it with the view angles and
    /// distance 30, only updated in third person and left stale in first person, as in TS.
    cam_ofs: Vec3,
    // TSFade
    fade: Option<(f32, state_fade::Fade)>,
    // outputs
    cam: Camera,
    eye: Vec3,
    hardware: bool,
}

mod state_fade {
    #[derive(Clone, Copy, Default, Debug, PartialEq)]
    pub struct Fade {
        pub duration: f32,
        pub hold: f32,
        pub rgba: [u8; 4],
    }
}

/// `ViewShake` (`0x1018cac8`): a damped spring flipping direction at `freq` Hz.
#[derive(Default, Clone, Copy)]
struct Shake {
    target: Vec3,
    cur: Vec3,
    amp: f32,
    time_left: f32,
    freq: f32,
    flip: f32,
}

impl Shake {
    /// `ViewShake::Start` (`0x10057fa0`). Random flips are unseeded (`pfnRandomFloat`).
    fn start(&mut self, amp: f32, dur: f32, freq: f32) {
        self.time_left = dur;
        self.freq = freq;
        self.amp = amp;
        self.flip = 1.0 / freq;
        self.target = [r::random_float(-amp, amp), r::random_float(-amp, amp), 0.0];
    }
    /// `ViewShake::Update` (`0x10058000`), on `dt = g_flTimeScale·frametime`: returns this
    /// frame's step, which TS adds to `cl_viewangles`. The x87 keeps the rate·dt factor and the
    /// pitch difference unrounded (`0x100580f3..0x1005814b`); the others are stored first.
    fn update(&mut self, dt: f32) -> Vec3 {
        // `time_left > 0` (`0x10058017`; a NaN counts as run out).
        let (target, rate) = if self.time_left > 0.0 {
            self.time_left -= dt;
            self.flip -= dt;
            if self.flip < 0.0 {
                self.flip = 1.0 / self.freq;
                self.target = v::scale(self.target, -0.5);
            }
            (self.target, self.freq)
        } else {
            (v::ZERO, 20.0)
        };
        let k = rate as f64 * dt as f64;
        let k = if k < 1.0 || k.is_nan() { k } else { 1.0 };
        let step = [
            ((target[0] as f64 - self.cur[0] as f64) * k) as f32,
            ((target[1] - self.cur[1]) as f64 * k) as f32,
            ((target[2] - self.cur[2]) as f64 * k) as f32,
        ];
        self.cur = v::add(self.cur, step);
        step
    }
}

/// `V_DriftPitch` / `V_StartPitchDrift` / `V_StopPitchDrift` state (SDK `pitchdrift_t`).
#[derive(Default, Clone, Copy, Debug, PartialEq)]
struct Drift {
    pitchvel: f32,
    nodrift: bool,
    driftmove: f32,
    laststop: f32,
}

/// What `V_DriftPitch` reads from `ref_params_t`, the cvars and the engine.
#[derive(Clone, Copy, Debug, Default)]
struct DriftInput {
    /// Noclip, in the air, demo playback or spectating: no drift.
    blocked: bool,
    forwardmove: f32,
    forwardspeed: f32,
    centermove: f32,
    centerspeed: f32,
    frametime: f32,
    idealpitch: f32,
    now: f32,
}

impl Drift {
    /// `V_StopPitchDrift` (`0x100672f0`).
    fn stop(&mut self, now: f32) {
        self.laststop = now;
        self.nodrift = true;
        self.pitchvel = 0.0;
    }

    /// `V_StartPitchDrift` (`0x10067290`, the `centerview` command, and inlined in
    /// `V_DriftPitch`): starts drifting at `v_centerspeed`, unless something stopped the drift in
    /// this same frame (`now` equals the last stop) or it is already drifting.
    fn start(&mut self, now: f32, centerspeed: f32) {
        if now != self.laststop && (self.nodrift || self.pitchvel == 0.0) {
            self.pitchvel = centerspeed;
            self.nodrift = false;
            self.driftmove = 0.0;
        }
    }

    /// `V_DriftPitch` (`0x10067320`, SDK): eases `pitch` toward the ideal pitch once the player
    /// has walked forward for `v_centermove` seconds without looking around.
    fn update(&mut self, i: &DriftInput, pitch: &mut f32) {
        if i.blocked {
            self.driftmove = 0.0;
            self.pitchvel = 0.0;
            return;
        }
        if self.nodrift {
            if i.forwardmove.abs() < i.forwardspeed {
                self.driftmove = 0.0;
            } else {
                self.driftmove += i.frametime;
            }
            if self.driftmove > i.centermove {
                self.start(i.now, i.centerspeed);
            }
            return;
        }
        let delta = i.idealpitch - *pitch;
        if delta == 0.0 {
            self.pitchvel = 0.0;
            return;
        }
        // The x87 keeps the step and `v_centerspeed·ft` unrounded (`0x100673e8..0x10067403`).
        let mut step = self.pitchvel as f64 * i.frametime as f64;
        self.pitchvel = (i.centerspeed as f64 * i.frametime as f64 + self.pitchvel as f64) as f32;
        if delta > 0.0 {
            if (delta as f64) < step {
                self.pitchvel = 0.0;
                step = delta as f64;
            }
            *pitch = (step + *pitch as f64) as f32;
        } else {
            if (-delta as f64) < step {
                self.pitchvel = 0.0;
                step = -delta as f64;
            }
            *pitch = (*pitch as f64 - step) as f32;
        }
    }
}

/// `V_SmoothStairs` (`0x100677d0`) state: the smoothed eye z (`0x10191d84`), the offset carried
/// off the ground (`0x10191d88`) and last call's `onground` (`0x100972ac`). The statics start as
/// 0, 0 and -1, so the first call counts as "was on the ground". A ruleset's
/// [`ClientRules::eye_origin`](crate::gameplay::ClientRules::eye_origin) runs the smoothing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stairs {
    /// The smoothed eye z.
    pub smooth_z: f32,
    /// The offset carried off the ground.
    pub ofs: f32,
    /// Last call's `onground`.
    pub was_on_ground: bool,
}

impl Default for Stairs {
    fn default() -> Self {
        Stairs { smooth_z: 0.0, ofs: 0.0, was_on_ground: true }
    }
}

/// The eye z's smoothing state, which `V_CalcNormalRefdef` builds the eye from
/// ([`ClientRules::eye_origin`](crate::gameplay::ClientRules::eye_origin) and
/// [`body_cam_z`](crate::gameplay::ClientRules::body_cam_z)). Zero-initialised and never reset, as
/// in TS, so the first spawn after the library loads eases the eye up from the origin.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EyeZ {
    /// The smoothed view height (TS 3.0's `0x10191d90`), for a ruleset that eases it.
    pub vh: f32,
    /// The `V_SmoothStairs` state, for a ruleset that smooths stairs.
    pub stairs: Stairs,
    /// Last frame's eye z, for a ruleset that follows the whole eye z instead.
    pub follow_z: f32,
}

const RING: u32 = 64;
const RING_MASK: u32 = RING - 1;

/// A 64-entry history of positions and their times: the SDK `ViewInterp` origin ring TS keeps
/// for `cl_vsmoothing`.
#[derive(Clone, Copy)]
struct Ring {
    pos: [Vec3; RING as usize],
    time: [f32; RING as usize],
    cur: u32,
}

impl Default for Ring {
    fn default() -> Self {
        Ring { pos: [v::ZERO; RING as usize], time: [0.0; RING as usize], cur: 0 }
    }
}

impl Ring {
    fn push(&mut self, pos: Vec3, time: f32) {
        let i = (self.cur & RING_MASK) as usize;
        self.pos[i] = pos;
        self.time[i] = time;
        self.cur = self.cur.wrapping_add(1);
    }

    /// The position at time `t`, interpolated between the newest sample at or before `t` and the
    /// one after it, with the length of that segment (`V_CalcNormalRefdef` step 14). `None` when
    /// the history doesn't reach back to `t` or the samples share a time.
    fn sample(&self, t: f32) -> Option<(Vec3, f32)> {
        let (a, b, frac) = self.sample_pair(t)?;
        let delta = v::sub(b, a);
        Some((v::ma(a, frac, delta), v::len(delta)))
    }

    /// The newest sample at or before `t`, the one after it and the fraction of the way to it.
    fn sample_pair(&self, t: f32) -> Option<(Vec3, Vec3, f32)> {
        let (a, b) = self.pair(t)?;
        let dt = self.time[b] - self.time[a];
        if dt <= 0.0 {
            return None;
        }
        Some((self.pos[a], self.pos[b], ((t - self.time[a]) / dt).min(1.0)))
    }

    /// The slots of the newest sample at or before `t` (with a nonzero time) and the one after it.
    fn pair(&self, t: f32) -> Option<(usize, usize)> {
        let mut idx = self.cur.wrapping_sub(2);
        for _ in 1..RING_MASK {
            let a = (idx & RING_MASK) as usize;
            if self.time[a] <= t {
                if self.time[a] == 0.0 {
                    return None;
                }
                return Some((a, (idx.wrapping_add(1) & RING_MASK) as usize));
            }
            idx = idx.wrapping_sub(1);
        }
        None
    }

    /// The angle ring's lagged view angles (`0x10067bed..0x10067c69`): `InterpolateAngles`
    /// between the two samples around `t`, which (as in TS) normalises both ring entries in place,
    /// with the fraction in `double`s ([`crate::spectator::angle_ring_frac`]).
    fn interpolate_angles(&mut self, t: f32) -> Option<Vec3> {
        let (a, b) = self.pair(t)?;
        let frac = crate::spectator::angle_ring_frac(self.time[a], self.time[b], t)?;
        let (mut x, mut y) = (self.pos[a], self.pos[b]);
        let out = crate::spectator::interpolate_angles(&mut x, &mut y, frac);
        (self.pos[a], self.pos[b]) = (x, y);
        Some(out)
    }
}

static VIEW: Global<RefCell<Option<View>>> = Global(RefCell::new(None));
static CVARS: Global<RefCell<Option<Cvars>>> = Global(RefCell::new(None));
static CAM: Global<Cell<Option<CamCvars>>> = Global(Cell::new(None));

/// The `CAM_Init` cvars [`cam_think`] reads.
#[derive(Clone, Copy)]
struct CamCvars {
    command: Cvar,
    snapto: Cvar,
    idealyaw: Cvar,
    idealpitch: Cvar,
    idealdist: Cvar,
    maxpitch: Cvar,
    minpitch: Cvar,
    maxyaw: Cvar,
    minyaw: Cvar,
    maxdistance: Cvar,
    mindistance: Cvar,
}

/// `CAM_Init` (`0x10025370`, called from `InitInput`, so [`crate::input::init`] calls this):
/// registers TS's camera commands ([`crate::camera::register_commands`], with `thirdperson` and
/// `firstperson`), then the SDK camera cvars with TS's defaults (`cam_idealdist` 100, the SDK's
/// 64). `CAM_Think` reads `cam_command`, `cam_snapto` and `cam_ideal*`; `c_min/max*` bound the
/// `+cammousemove`/`+camdistance` adjustments; `cam_contain` is never read (TS's `CAM_Think` has
/// no contain block).
pub fn cam_init() {
    crate::camera::register_commands(cmd_thirdperson, cmd_firstperson);
    let command = Cvar::register(c"cam_command", c"0", 0);
    let snapto = Cvar::register(c"cam_snapto", c"0", 0);
    let idealyaw = Cvar::register(c"cam_idealyaw", c"0", 0);
    let idealpitch = Cvar::register(c"cam_idealpitch", c"0", 0);
    let idealdist = Cvar::register(c"cam_idealdist", c"100", 0);
    Cvar::register(c"cam_contain", c"0", 0);
    CAM.0.set(Some(CamCvars {
        command,
        snapto,
        idealyaw,
        idealpitch,
        idealdist,
        maxpitch: Cvar::register(c"c_maxpitch", c"90.0", 0),
        minpitch: Cvar::register(c"c_minpitch", c"0.0", 0),
        maxyaw: Cvar::register(c"c_maxyaw", c"135.0", 0),
        minyaw: Cvar::register(c"c_minyaw", c"-135.0", 0),
        maxdistance: Cvar::register(c"c_maxdistance", c"200.0", 0),
        mindistance: Cvar::register(c"c_mindistance", c"30.0", 0),
    }));
}

fn with<R>(f: impl FnOnce(&mut View) -> R) -> R {
    let mut g = VIEW.0.borrow_mut();
    f(g.get_or_insert_with(|| View { local_amt: 255.0, ..View::default() }))
}

fn cv(f: impl FnOnce(&Cvars) -> Cvar) -> f32 {
    CVARS.0.borrow().as_ref().map_or(0.0, |c| f(c).value())
}

/// `V_Init`: registers the view cvars TS registers there; the camera, third-person and
/// smoothing cvars are `InitInput`'s, so this shares those handles rather than registering the
/// names again (see [`Cvar`]). Runs after `input::init`.
pub fn init() {
    let i = crate::input::cvars();
    *CVARS.0.borrow_mut() = Some(Cvars {
        bob: Cvar::register(c"cl_bob", c"4.0", r::FCVAR_ARCHIVE),
        waterdist: Cvar::register(c"cl_waterdist", c"4", 0),
        slowfade: Cvar::register(c"cl_slowfade", c"1", 0),
        camtype: i.cl_camtype,
        cam_back: i.cam_back,
        cam_right: i.cam_right,
        cam_up: i.cam_up,
        thirdtransparency: i.cl_thirdtransparency,
        vsmoothing: i.cl_vsmoothing,
        centermove: Cvar::register(c"v_centermove", c"0.15", 0),
        centerspeed: Cvar::register(c"v_centerspeed", c"500", 0),
    });
    Cvar::register(c"cl_bobcycle", c"0.8", 0);
    Cvar::register(c"cl_bobup", c"0.5", 0);
    // TS's V_Init (`0x10069e90`) registers these but never reads them: the SDK's
    // `V_CalcNormalRefdef` adds them to the view origin, TS's doesn't.
    Cvar::register(c"scr_ofsx", c"0", 0);
    Cvar::register(c"scr_ofsy", c"0", 0);
    Cvar::register(c"scr_ofsz", c"0", 0);
    let e = goldsrc::client::engine();
    e.add_command(c"centerview", cmd_centerview);
    e.add_command(c"ts_test_fade", cmd_test_fade);
    e.add_command(c"ts_test_kick", cmd_test_kick);
    e.add_command(c"ts_test_view", cmd_test_view);
    e.add_command(c"ts_test_angles", cmd_test_angles);
}

/// `centerview` (`V_StartPitchDrift` `0x10067290`, registered first in `V_Init` `0x10069e90`).
extern "C" fn cmd_centerview() {
    let now = engine().client_time();
    let speed = cv(|c| c.centerspeed);
    with(|w| w.drift.start(now, speed));
}

/// `ts_test_angles <pitch> <yaw>`: sets the view angles (developer aid for scripted screenshots).
extern "C" fn cmd_test_angles() {
    let e = goldsrc::client::engine();
    let n = |i: i32| e.cmd_argv(i).to_str().unwrap_or("").parse::<f32>().unwrap_or(0.0);
    let a = [n(1), n(2), 0.0];
    e.set_view_angles(a);
}

/// `ts_test_view`: prints the view state (developer aid). `menu` is whether a menu owns the cursor
/// (`cargo xtask steam` checks that server-set angles still apply then).
extern "C" fn cmd_test_view() {
    let menu = crate::menus::wants_cursor();
    let s = with(|w| {
        format!(
            "third {} menu {menu} cam_back {:.1} cam_up {:.1} model_z {:.1} amt {:.0} origin {:?} eye {:?} angles {:?}\n",
            w.third, w.cam_ofs_smooth.back, w.cam_ofs_smooth.up, w.model_z, w.local_amt, w.cam.origin, w.eye, w.cam.angles
        )
    });
    let ts = state::with(|st| format!("ts {:.3}/{:.3} fov {:.1}/{:.1}\n", st.time_scale, st.time_scale_target, st.fov, st.fov_target));
    goldsrc::client::engine().print(&s);
    goldsrc::client::engine().print(&ts);
}

/// `ts_test_fade <r> <g> <b> <a> <duration> <hold>` (developer test of the TSFade path).
extern "C" fn cmd_test_fade() {
    let e = goldsrc::client::engine();
    let n = |i: i32| e.cmd_argv(i).to_str().unwrap_or("").parse::<f32>().unwrap_or(0.0);
    let f = shared::protocol::effects::TsFade {
        duration: n(5).max(0.1),
        hold: n(6),
        flags: 0,
        rgba: [n(1) as u8, n(2) as u8, n(3) as u8, n(4) as u8],
    };
    let mut b = Vec::new();
    f.encode(&mut b);
    on_tsfade(&b);
}

/// `ts_test_kick <view kick> <class>`: one shot's client-side recoil.
extern "C" fn cmd_test_kick() {
    let e = goldsrc::client::engine();
    let n = |i: i32| e.cmd_argv(i).to_str().unwrap_or("").parse::<f32>().unwrap_or(0.0);
    kick(if n(1) == 0.0 { 5.5 } else { n(1) }, n(2) as u8, false, false, e.client_time());
}

extern "C" fn cmd_thirdperson() {
    toggle_third_person();
}

/// TS's `thirdperson` (`0x1004fe00`, replacing the SDK's `CAM_ToThirdPerson`): a toggle, also
/// run by `cam_command 1` and, from inside their handlers, by the HUD's camera switches: the death
/// cam (`TSHealth`), `cl_thirdpersonswitch` (`WeaponInfo`) and `cl_thirdperson` (`Spectator`).
/// * During demo playback it only switches when `cam_thirdperson` differs from the state the demo
///   last recorded ([`crate::demo::recorded_thirdperson`]), so replayed switches follow the
///   recording.
/// * Otherwise an observer can leave third person but not enter it.
///
/// Switching on seeds `cam_ofs` with the view pitch and yaw at distance 30. Every switch sets
/// `cam_command` back to 0 and, while recording, writes the new state to the demo (type 2).
pub fn toggle_third_person() {
    let e = engine();
    let third = with(|w| w.third);
    if e.demo_playing() {
        if third as i32 == crate::demo::recorded_thirdperson() {
            return;
        }
    } else if crate::hud::observer() && !third {
        return;
    }
    let view = e.view_angles();
    let third = !third;
    with(|w| {
        w.third = third;
        if third {
            w.cam_ofs = [view[0], view[1], 30.0];
        }
    });
    e.cvar_set(c"cam_command", 0.0);
    if e.demo_recording() {
        crate::demo::write_buffer(crate::demo::THIRDPERSON, &(third as i32).to_le_bytes());
    }
}

/// `firstperson` (`CAM_ToFirstPerson` `0x10025320`): clears `cam_thirdperson` and `cam_command`.
extern "C" fn cmd_firstperson() {
    with(|w| w.third = false);
    engine().cvar_set(c"cam_command", 0.0);
}

/// `TSFade` (`0x1004c1e0`): stored with start = now; applied every frame in `calc_refdef`.
pub fn on_tsfade(b: &[u8]) {
    if let Some(f) = shared::protocol::effects::TsFade::decode(b) {
        let now = engine().client_time();
        with(|w| w.fade = Some((now, state_fade::Fade { duration: f.duration, hold: f.hold, rgba: f.rgba })));
    }
}

/// `UTIL_ApproachFOV` (`0x10049620`), from `CHud::Think`: `m_iFOV` (an `int`) moves toward
/// `target` by `max(1, (|d| / 2) * ft * 50)` (integer halving), truncated, without passing it.
/// With `snap` it jumps to the target: [`ClientRules::fov_snaps`](crate::gameplay::ClientRules::fov_snaps)
/// decides, which in TS 3.0 is an unset FOV (below 0.001).
pub fn ease_fov(cur: f32, target: f32, ft: f32, snap: bool) -> f32 {
    let (cur, target) = (cur as i32, target as i32);
    if snap {
        return target as f32;
    }
    if cur == target {
        return cur as f32;
    }
    // C `int` arithmetic: a server FOV far from the current one wraps rather than overflowing.
    let step = ((target.wrapping_sub(cur).wrapping_abs() / 2) as f64 * (ft as f64) * 50.0).max(1.0);
    let next = if cur < target { ((cur as f64 + step) as i32).min(target) } else { ((cur as f64 - step) as i32).max(target) };
    next as f32
}

/// The emitter part of `HUD_Frame` (`CTS_CL_Emitter::Frame` `0x1003ab40`), before the slow-motion
/// easer ([`crate::hud::frame`]). The engine passes the frame time (FWGS `host.frametime`, as the
/// SDK's `HUD_Frame(double time)` does), not an absolute time. TS hands it on unclamped
/// (`0x1001ab02`).
pub fn frame(frametime: f64) {
    let time = engine().client_time() as f64;
    with(|w| {
        w.last_frame = time;
        w.frametime = frametime as f32;
    });
    effects::frame(time, frametime);
}

/// `CAM_Think` (`0x10024c80`), every frame from the engine: `cam_command` 1 runs `thirdperson`
/// (a toggle) and 2 `firstperson` (both reset it to 0); then, in third person, the camera
/// controls adjust `cam_idealpitch`/`cam_idealyaw`/`cam_idealdist`
/// ([`crate::camera::adjust_ideal`], written straight to the cvar values as TS does), and
/// `cam_ofs` follows the view angles plus the ideal angles (snapped with `cam_snapto`, else eased
/// by `MoveToward`) and takes `cam_idealdist`. [`calc_normal`] places the camera along
/// `cam_ofs`'s pitch and yaw (`CL_CameraOffset`); TS ignores the distance there. In first person
/// `cam_ofs` is left as it was.
pub fn cam_think() {
    let Some(c) = CAM.0.get() else { return };
    match c.command.value() as i32 {
        1 => toggle_third_person(),
        2 => cmd_firstperson(),
        _ => {}
    }
    if !is_third_person() {
        return;
    }
    let lim = crate::camera::Limits {
        pitch: (c.minpitch.value(), c.maxpitch.value()),
        yaw: (c.minyaw.value(), c.maxyaw.value()),
        dist: (c.mindistance.value(), c.maxdistance.value()),
    };
    let ideal = crate::camera::adjust_ideal([c.idealpitch.value(), c.idealyaw.value(), c.idealdist.value()], lim);
    c.idealpitch.set_value(ideal[0]);
    c.idealyaw.set_value(ideal[1]);
    c.idealdist.set_value(ideal[2]);
    let view = engine().view_angles();
    let snapto = c.snapto.value() != 0.0;
    with(|w| w.cam_ofs = cam_ofs_step(w.cam_ofs, view, ideal, snapto));
}

/// The `cam_ofs` update at the end of `CAM_Think` (`0x10025160..0x10025247`). `ideal` is
/// (`cam_idealpitch`, `cam_idealyaw`, `cam_idealdist`). Unlike the SDK, TS doesn't ease the
/// distance.
fn cam_ofs_step(cur: Vec3, view: Vec3, ideal: Vec3, snapto: bool) -> Vec3 {
    if snapto {
        return [view[0] + ideal[0], view[1] + ideal[1], ideal[2]];
    }
    let yaw = if cur[1] - view[1] != ideal[1] { move_toward(cur[1], view[1] + ideal[1]) } else { cur[1] };
    let pitch = if cur[0] - view[0] != ideal[0] { move_toward(cur[0], view[0] + ideal[0]) } else { cur[0] };
    [pitch, yaw, ideal[2]]
}

/// `MoveToward` (`0x10024ba0`, SDK `in_camera.cpp`): a quarter of the way to `goal` (the short
/// way round), snapping within 1 degree, then wrapped into [0, 360). The SDK's `maxspeed`
/// argument (2.5) is unused there too.
fn move_toward(mut cur: f32, goal: f32) -> f32 {
    if cur != goal {
        // `abs((int)(cur - goal)) > 180.0`: the difference is truncated first.
        if ((cur - goal) as i32).unsigned_abs() as f32 > 180.0 {
            cur += if cur < goal { 360.0 } else { -360.0 };
        }
        cur = if cur < goal {
            if cur < goal - 1.0 { cur + (goal - cur) * 0.25 } else { goal }
        } else if cur > goal + 1.0 {
            cur - (cur - goal) * 0.25
        } else {
            goal
        };
    }
    if cur < 0.0 {
        cur + 360.0
    } else if cur >= 360.0 {
        cur - 360.0
    } else {
        cur
    }
}

/// `CL_IsThirdPerson` (`0x1004fec0`): `cam_thirdperson`, the `thirdperson` toggle's state, and
/// nothing else. Every TS reader of third person (`CAM_Think`, `V_CalcNormalRefdef`, the bob,
/// events, cull origin, occlusion, lasers, motion-blur ghosts, the voice icon, the crosshair, and
/// the `TSHealth`/`PwUp`/`Spectator` camera switches) uses this flag or `cam_thirdperson`
/// directly; the death cam is the `TSHealth` 0 toggle, so it counts only through the flag.
pub fn is_third_person() -> bool {
    with(|w| w.third)
}

/// `CL_CameraOffset` (`0x10025720`): `cam_ofs`, the camera's (pitch, yaw, distance) that
/// [`cam_think`] keeps (zero until the first `thirdperson`, stale in first person).
pub fn camera_offset() -> Vec3 {
    with(|w| w.cam_ofs)
}

/// View origin of the last refdef (`v_origin`).
pub fn origin() -> Vec3 {
    with(|w| w.cam.origin)
}
/// Eye position (`g_vecEyePos`): origin + view height, also in third person.
pub fn eye() -> Vec3 {
    with(|w| w.eye)
}
/// `v_angles` (`0x101914d0`): the view angles the last refdef stored ([`finish`]), punch
/// included and before the third-person override. `HUD_WeaponsPostThink` copies them into the
/// client player's `pev->v_angle` for every predicted command (`0x100038e7..0x10003906`).
pub fn v_angles() -> Vec3 {
    with(|w| w.cam.angles)
}
/// The angles the predicted weapon events are played back with ([`View::playback_angles`]).
pub fn playback_angles() -> Vec3 {
    with(|w| w.playback_angles)
}
pub fn camera() -> Camera {
    let fov = state::with(|s| s.fov);
    with(|w| Camera { fov, ..w.cam })
}
pub fn hardware() -> bool {
    with(|w| w.hardware)
}

/// `CHud::UpdateClientData` (`0x10024b00`): runs `CHud::Think` (the elements' `Think`s,
/// [`crate::hud::think_elements`], then the zoom easing, [`crate::hud::think_fov`]) and writes the eased FOV into `clientdata`. Returns true (the
/// engine's "changed" flag), as `HUD_UpdateClientData` must.
pub fn update_client_data(cd: &mut r::ClientData) -> bool {
    crate::hud::think_elements();
    cd.fov = crate::hud::think_fov(with(|w| w.refdef_frametime));
    // `v_idlescale = m_iConcuss` (`0x10024b63`, after `CHud::Think`).
    let concuss = crate::hud::concussion();
    with(|w| w.idlescale = concuss);
    true
}

/// `CTSGun::VKickback` client half (`0x10008230`) for a client weapon class (development
/// command `ts_test_kick`, at `now`). Prediction calls [`kick_angles`] with the shared
/// `VKickback` result.
pub fn kick(view_kick: f32, class: u8, silenced: bool, double_rate: bool, now: f32) {
    let mut pitch = view_kick * 0.85;
    let mut yaw = pitch
        * match class {
            8 => 0.35,
            4 | 5 => 0.15,
            1 => -0.25,
            6 | 7 => -0.3,
            9 => -0.35,
            _ => 0.0,
        };
    if silenced {
        pitch *= 0.775;
        yaw *= 0.775;
    }
    if double_rate {
        pitch *= 2.0 / 3.0;
        yaw *= 2.0 / 3.0;
    }
    kick_angles(pitch, yaw, Some(silenced), now);
}

/// Queues one shot's view kick (`pitch` up, `yaw` right, degrees) and, with `shake` =
/// `Some(silenced)`, starts the shake spring as TS 3.0's `VKickback` does (`None`: the kick
/// alone, for rulesets without the shake). Call once per shot, with `runfuncs` only. `now` is the
/// command's prediction time: TS times the shake with `*0x10099114`, which
/// `HUD_WeaponsPostThink` sets to `HUD_PostRunCmd`'s `time` as a float, not with the client clock.
pub fn kick_angles(pitch: f32, yaw: f32, shake: Option<bool>, now: f32) {
    let ts = state::with(|s| s.time_scale);
    with(|w| {
        w.kick_target[0] -= pitch;
        w.kick_target[1] += yaw;
        let Some(silenced) = shake else { return };
        if let Some(amp) = shake_amplitude(now, w.last_shot, ts) {
            w.shake.start(amp * if silenced { 0.775 } else { 1.0 }, 0.1, 35.0);
        }
        w.last_shot = now;
    });
}

/// The client melee attacks' view shake: `ViewShake::Start(amp, 0.2, 10)` (`0x10008da7`,
/// `0x1000a15c`, `0x1000a32d`, `0x1000b5cd`). It restarts the same spring as the gun kick.
pub fn melee_shake(amp: f32) {
    with(|w| w.shake.start(amp, 0.2, 10.0));
}

/// `VKickback`'s shake strength (`0x10008346..0x100083ad`): `(0.15 - (now - last) * ts) * 15`,
/// capped at 1, or `None` at or below 0.001. A time scale of 0 counts as 1.
fn shake_amplitude(now: f32, last: f32, ts: f32) -> Option<f32> {
    let ts = if ts == 0.0 { 1.0 } else { ts };
    let amp = (0.15 - (now - last) * ts) * 15.0;
    (amp > 0.001).then_some(amp.min(1.0))
}

/// The client gun's Spawn (`0x100083f0`) zeroes `CTSGun+0x8c0`, the time of the last kick that
/// `VKickback` (`0x10008230`) scales the shake by.
pub fn reset_kick_time() {
    with(|w| w.last_shot = 0.0);
}

/// What `V_CalcBob` reads.
#[derive(Clone, Copy, Debug, Default)]
pub struct BobInput {
    /// Third person, `pparams->spectator`, or `PM_SpectatorMove` smoothing flags set without the
    /// "keep bobbing" bit (`0x10066e74..0x10066e9b`): return 0 and leave every static alone.
    pub skip: bool,
    pub cl_bob: f32,
    pub on_ground: bool,
    pub time: f32,
    pub frametime: f32,
    pub viewheight: f32,
    pub simvel: Vec3,
    /// `g_flTimeScale` (`0x100963e8`), unclamped.
    pub ts: f32,
    /// The last `TSSlowMo` value as sent (the factor × 100; 0 = normal speed, and before any).
    pub slowmo_factor100: f32,
    /// `pparams->forward/right/up` as the engine passes them in: the vectors of the previous
    /// frame's view angles (FWGS `V_SetRefParams`), since `V_CalcBob` runs before
    /// `V_CalcNormalRefdef` computes this frame's.
    pub forward: Vec3,
    pub right: Vec3,
    pub up: Vec3,
}

/// `V_CalcBob` (`0x10066e70`) and `V_SmoothBobZ` (`0x10066de0`) statics, which a ruleset's
/// [`ClientRules::view_bob`](crate::gameplay::ClientRules::view_bob) advances.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Bob {
    /// `pparams->time` of the last bobbing frame (`0x10191d70`).
    pub last_time: f32,
    /// Phase in degrees (`0x10191d74`), reset to 0 once past 360.
    pub phase: f32,
    /// Amplitude (`0x10191d6c`), also `V_CalcBob`'s return value.
    pub amp: f32,
    /// The amplitude last set from the speed (`0x10191d78`); below it the amplitude eases back.
    pub peak: f32,
    /// Smoothed z of the gun offset (`0x1019144c`).
    pub smooth_z: f32,
    /// World-space gun offset (`0x10191428`), added to the view model's origin. Left as it was
    /// on a skipped frame, as the in-eye gun also reads it.
    pub ofs: Vec3,
    /// Bob pitch (`0x10190b78`); `V_CalcNormalRefdef` consumes it and zeroes it every frame.
    pub pitch: f32,
    /// A camera bob `V_CalcNormalRefdef` adds to `vieworg` every frame, right after the view
    /// height, once a ruleset's bob has set it. TS 3.0 has none (always `None`).
    pub eye: Option<Vec3>,
}

/// `V_CalcGunAngle` (`0x100674a0`): the gun's angles are `(bob − pitch, yaw, roll)` of the view
/// angles (stunt roll included), with no `crosshairangle` terms. TS also subtracts
/// `ev_punchangle·1.25` from the pitch and adds it to the yaw, but that is always 0 (see the punch
/// note in [`calc_normal`]). The current and latched angles are set here, before the 0.9 shake.
pub(crate) fn calc_gun_angle(vm: &mut cl_entity_t, viewangles: Vec3, bob: f32) {
    vm.angles = gun_angles(viewangles, bob);
    vm.curstate.angles = vm.angles;
    vm.latched.prevangles = vm.angles;
}

fn gun_angles(viewangles: Vec3, bob: f32) -> Vec3 {
    [bob - viewangles[0], viewangles[1], viewangles[2]]
}

/// The gun offset `V_CalcBob` last left (`0x10191428`), which `V_CalcSpectatorRefdef` adds to the
/// in-eye gun (`0x10069a70`).
pub fn gun_offset() -> Vec3 {
    with(|w| w.bob.ofs)
}

/// What `V_CalcViewRoll` reads from an entity: its angles and its `prevstate` observer mode and
/// stunt bits (TS reads `prevstate` here, `+0x260`, `+0x26c`, not `curstate` `+0x3c0`, so the
/// stunt bits lag the latest packet by one).
#[derive(Clone, Copy, Debug, Default)]
pub struct RollEntity {
    pub angles: Vec3,
    /// `prevstate.iuser1`: the observer mode.
    pub iuser1: i32,
    /// `prevstate.iuser4`: the stunt bits.
    pub iuser4: i32,
}

/// Which view `V_CalcRefdef` computes this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RefdefPath {
    Intermission,
    Spectator,
    Normal,
    /// Paused outside intermission and spectating: no view is computed, so the engine keeps last
    /// frame's `vieworg`/`viewangles` and the bob, kick, shake and stair state don't advance.
    None,
}

/// The dispatch at the top of `V_CalcRefdef` (`0x10069b90`): `V_CalcNormalRefdef` only runs while
/// `pparams->paused == 0`; the intermission and spectator views run regardless.
fn refdef_path(intermission: bool, spectating: bool, paused: bool) -> RefdefPath {
    if intermission {
        RefdefPath::Intermission
    } else if spectating {
        RefdefPath::Spectator
    } else if paused {
        RefdefPath::None
    } else {
        RefdefPath::Normal
    }
}

/// `V_CalcRefdef` (`0x10069b90`).
pub fn calc_refdef(p: &mut ref_params_t) {
    with(|w| w.refdef_frametime = p.frametime);
    // TS 3.0 runs the spectator view for HLTV viewers and observing players; a ruleset may
    // limit it (TS 2.1: HLTV only, so observers get the normal view).
    let spectating = crate::gameplay::current().spectator_refdef(p.spectator != 0, state::with(|s| s.spec_mode));
    match refdef_path(p.intermission != 0, spectating, p.paused != 0) {
        RefdefPath::Intermission => calc_intermission(p),
        RefdefPath::Spectator => calc_spectator(p),
        RefdefPath::Normal => calc_normal(p),
        RefdefPath::None => {}
    }
    // The screen fade and fog are written every frame, paused or not. `nextView` is left alone,
    // as TS does: the engine zeroes it before the first pass, and only the spectator view asks
    // for a second one (its inset window).
    write_screen_fade(p.time);
    effects::fog_begin();
}

/// Stores the camera the effects, HUD and events read (`v_origin` `0x10190b28` and `v_angles`
/// `0x101914d0`). `angles` is `v_angles`: the view angles before the third-person override. The
/// effects' vectors are `AngleVectors(v_angles)` (`0x10056f74`). `pparams->forward/right/up` are
/// left alone: only the normal view writes them (`0x10067eee`, from `cl_viewangles`); the
/// intermission (`0x100676e0`) and spectator (`0x10068f70`) views never do.
fn finish(w: &mut View, p: &mut ref_params_t, angles: Vec3) {
    let (f, r_, u) = v::angle_vectors(angles);
    w.cam = Camera { origin: p.vieworg, angles, forward: f, right: r_, up: u, fov: 90.0 };
    w.hardware = p.hardware != 0;
}

/// `V_StopPitchDrift` (`0x100672f0`): TS input calls it while the mouse or the look keys move
/// the view (`CL_AdjustAngles` 0x10026330, `IN_MouseMove` 0x100274d0).
pub fn stop_pitch_drift() {
    let now = engine().client_time();
    with(|w| w.drift.stop(now));
}

/// `V_AddIdle` (`0x10067550`): idle sway with TS's fixed (unregistered) `v_i*` values
/// (`0x10097244..0x100972a4`): roll cycle 0.5 level 0.1, pitch cycle 1 level 0.3, yaw cycle 2
/// level 0.3, each `sin(cycle·time)·level·scale` added in that order on the x87.
fn add_idle(angles: &mut Vec3, time: f32, scale: f32) {
    let idle =
        |angle: f32, cycle: f32, level: f32| ((cycle as f64 * time as f64).sin() * level as f64 * scale as f64 + angle as f64) as f32;
    angles[2] = idle(angles[2], 0.5, 0.1);
    angles[0] = idle(angles[0], 1.0, 0.3);
    angles[1] = idle(angles[1], 2.0, 0.3);
}

/// `V_CalcIntermissionRefdef` (`0x100676e0`): the view sways with idle scale 1.
fn calc_intermission(p: &mut ref_params_t) {
    p.vieworg = p.simorg;
    p.viewangles = p.cl_viewangles;
    if let Some(vm) = r::view_model() {
        vm.model = std::ptr::null_mut();
    }
    add_idle(&mut p.viewangles, p.time, 1.0);
    let angles = p.viewangles;
    with(|w| finish(w, p, angles));
}

/// `V_CalcSpectatorRefdef` (`0x10068f70`): see `crate::spectator::calc_refdef`.
fn calc_spectator(p: &mut ref_params_t) {
    crate::spectator::calc_refdef(p);
    let angles = p.viewangles;
    with(|w| {
        w.eye = p.vieworg;
        finish(w, p, angles)
    });
}

/// `FUN_10057e90`: moves `cur` toward `target` by at most `step`.
fn approach(target: f32, cur: f32, step: f32) -> f32 {
    let d = target - cur;
    if d > step {
        cur + step
    } else if d < -step {
        cur - step
    } else {
        target
    }
}

/// Local player translucency (start of `V_CalcNormalRefdef`): `renderamt` eases toward 100 while
/// the crosshair is on a player and `cl_thirdtransparency` is set, else back to 255; 50 per frame
/// fading out, 5 per frame fading in (50 when the frame has no time). Returns the new amount.
fn local_amt_step(cur: f32, on_player: bool, enabled: bool, frame_has_time: bool) -> f32 {
    let target = if on_player && enabled { 100.0 } else { 255.0 };
    let step = if cur < target && frame_has_time { 5.0 } else { 50.0 };
    approach(target, cur, step)
}

/// What the third-person camera offsets read ([`ClientRules::camera_offsets`]).
///
/// [`ClientRules::camera_offsets`]: crate::gameplay::ClientRules::camera_offsets
#[derive(Clone, Copy, Debug)]
pub struct CamInput {
    /// `cl_camtype` (a cvar not every ruleset's client has).
    pub camtype: i32,
    /// The SDK camera cvars `cam_back`, `cam_right`, `cam_up`.
    pub cam_back: f32,
    pub cam_right: f32,
    pub cam_up: f32,
    /// Ducked view (0 < view height < 28).
    pub ducked: bool,
    pub alive: bool,
    /// The local player's `prevstate.iuser4 & 0x130c30`: dive, prone, flips, wall moves or
    /// super-jump glide.
    pub stunt: bool,
    pub ft: f32,
}

/// The camera offsets' smoothing state, kept across frames: the additions to the up and back
/// offsets (TS 3.0's eased floats `0x10191da4`/`0x10191da8`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CamOffsets {
    pub up: f32,
    pub back: f32,
}

/// Body cam (`back == 0`, `0x10068255`): at the eye of the model, pushed forward and up as the
/// camera pitches down (`s = max(sin pitch, 0)`), and further forward while ducked.
fn body_cam(eye: Vec3, ducked: bool, pitch: f32, fwd: Vec3, up: Vec3) -> Vec3 {
    let s = pitch.to_radians().sin().max(0.0);
    let mut org = v::add(eye, [0.0, 0.0, if ducked { 8.0 } else { 1.0 }]);
    if ducked {
        org = v::ma(org, (1.0 - s) * 8.0, fwd);
    }
    v::add(org, v::add(v::scale(fwd, s * 4.0), v::scale(up, s * 18.0)))
}

/// Third-person model z smoothing (`0x10068480`): `cur` follows `z` at rate 16 without
/// overshooting.
fn smooth_model_z(cur: f32, z: f32, ft: f32) -> f32 {
    let d = cur - z;
    let mut step = d * ft * 16.0;
    if step.abs() > d.abs() {
        step = d;
    }
    cur - step
}

/// `V_ApplyKick` (`0x10057ed0`): eases the current kick (`0x101914f8`) toward the target
/// (`0x10191d10`) at `min(ft·15, 1)` and returns the step (`0x10190b38`, roll 0) TS adds to
/// `cl_viewangles`. Within 0.1° (squared length below the double 0.01) all three are zeroed. The
/// x87 keeps the yaw difference for the length test and the pitch step for the add to the current
/// kick unrounded.
fn apply_kick(target: &mut [f32; 2], cur: &mut [f32; 2], ft: f32) -> Vec3 {
    let d0 = (target[0] - cur[0]) as f64;
    let d1 = target[1] as f64 - cur[1] as f64;
    // A NaN length counts as settled (`0x10057f02`).
    let moving = d0 * d0 + d1 * d1 >= 0.01;
    if !moving {
        *target = [0.0; 2];
        *cur = [0.0; 2];
        return v::ZERO;
    }
    let k = ft * 15.0;
    let k = if k > 1.0 { 1.0 } else { k };
    let s0 = d0 * k as f64;
    let s1 = (d1 as f32) * k;
    cur[0] = (s0 + cur[0] as f64) as f32;
    cur[1] += s1;
    [s0 as f32, s1, 0.0]
}

/// `0x10067d14`: `pitch + bobpitch·0.1f`, rounded once (x87).
fn add_bob_pitch(pitch: f32, bob_pitch: f32) -> f32 {
    (bob_pitch as f64 * f64::from(0.1f32) + pitch as f64) as f32
}

fn calc_normal(p: &mut ref_params_t) {
    // `g_flTimeScale` (`0x100963e8`), unclamped: `V_CalcBob` and `ViewShake::Update` scale by it.
    let ts = state::with(|s| s.time_scale);
    let dead = p.health <= 0;
    // The stunt camera's `iuser4` is the local player's `prevstate` one (`0x100681c0`, `+0x26c`).
    let iuser4 = engine().local_player().map_or(0, |e| e.prevstate.iuser4);
    let bob_level = cv(|c| c.bob);
    let waterdist = cv(|c| c.waterdist);
    let (camtype, cb, cr, cu) = (cv(|c| c.camtype) as i32, cv(|c| c.cam_back), cv(|c| c.cam_right), cv(|c| c.cam_up));
    let fov = state::with(|s| s.fov);
    let ft = p.frametime;

    // `V_DriftPitch` (step 1).
    let drift = DriftInput {
        blocked: r::is_no_clipping() || p.onground == 0 || p.demoplayback != 0 || p.spectator != 0,
        forwardmove: r::refdef_forwardmove(p),
        forwardspeed: crate::input::cvars().cl_forwardspeed.value(),
        centermove: cv(|c| c.centermove),
        centerspeed: cv(|c| c.centerspeed),
        frametime: ft,
        idealpitch: p.idealpitch,
        now: engine().client_time(),
    };
    with(|w| w.drift.update(&drift, &mut p.cl_viewangles[0]));

    // Local player translucency (step 2): `TS_IsLocalDead` (0x1004c250) is false while alive or
    // in a demo. The local model is only drawn in third person, so that's where it shows.
    // A ruleset may leave the local entity's render mode alone (`ClientRules::local_translucency`).
    let rules = crate::gameplay::current();
    let e = goldsrc::client::engine();
    let translucent = !state::with(|s| s.dead) || e.demo_playing() || e.demo_recording();
    let on_player = crate::hud::with(|h| h.crosshair_id.is_some());
    let transparency = cv(|c| c.thirdtransparency) as i32 != 0;
    // TS tests `gHUD.m_flTimeDelta` (`0x10067930`), the last HUD redraw's frame time.
    let hud_dt = crate::hud::time_delta();
    engine().with_local_player_mut(|ent| {
        if !rules.local_translucency() {
            return;
        }
        if translucent {
            let amt = with(|w| {
                w.local_amt = local_amt_step(w.local_amt, on_player, transparency, hud_dt != 0.0);
                w.local_amt
            });
            ent.curstate.rendermode = kRenderTransColor as i32;
            ent.curstate.renderamt = amt as i32;
        } else {
            ent.curstate.rendermode = kRenderNormal as i32;
            ent.curstate.renderamt = 255;
        }
    });

    let third = is_third_person();

    // What the client `PM_SpectatorMove` left for the view: normally nothing, as `PM_PlayerMove`
    // clears it on every move, but set in frames where movement spectates and the HUD doesn't
    // (yet), and the origin after an HLTV director reset.
    let ov = crate::spectator::view_overrides();
    let bob_in = BobInput {
        skip: third || p.spectator != 0 || (ov.smooth_flags != 0 && ov.smooth_flags & crate::spectator::smooth::BOB == 0),
        cl_bob: bob_level,
        on_ground: p.onground != 0,
        time: p.time,
        frametime: ft,
        viewheight: p.viewheight[2],
        simvel: p.simvel,
        ts,
        slowmo_factor100: state::with(|s| s.slowmo_factor100),
        forward: p.forward,
        right: p.right,
        up: p.up,
    };

    // `V_CalcBob` (`0x100679a5`) runs before the spectator camera replaces `simorg`.
    let (bob, gun_ofs) = with(|w| (rules.view_bob(&mut w.bob, &bob_in), w.bob.ofs));

    // The spectator camera origin replaces `simorg` (`0x100679aa`).
    if let Some(org) = ov.origin {
        p.simorg = org;
    }

    // vieworg = simorg plus the ruleset's smoothed view height (TS 3.0: on z only, then stair
    // smoothing of that eye z). During a stunt the server sends `view_ofs` 0 (`NO_VIEW`), so the eye
    // eases down to the hull centre like TS's.
    let on_ground = p.onground != 0;
    with(|w| {
        p.vieworg = rules.eye_origin(&mut w.eye_z, p.simorg, p.viewheight, on_ground, ft);
        if let Some(eye) = w.bob.eye {
            p.vieworg = v::add(p.vieworg, eye);
        }
    });

    // The spectator camera angles replace `cl_viewangles` (`0x10067a4b`), then the angle ring
    // (`0x10067a7a..0x10067c90`): `cl_viewangles` is recorded whenever it changes, and lagged
    // `cl_vsmoothing` seconds behind while the angle flag is set, the result also becoming the
    // spectator camera's angles. Unlike the spectator view's ring there is no 22.5° limit.
    if let Some(a) = ov.angles {
        p.cl_viewangles = a;
    }
    let mut vsmoothing = cv(|c| c.vsmoothing);
    with(|w| {
        // `0x10067aaf`: the unrounded `Length` of the change, a NaN counting as none.
        if !crate::spectator::x87_eq0(crate::spectator::length(v::sub(p.cl_viewangles, w.last_cl_angles))) {
            w.ang_ring.push(p.cl_viewangles, p.time);
            w.last_cl_angles = p.cl_viewangles;
        }
    });
    if !crate::spectator::x87_eq0(vsmoothing as f64) && ov.smooth_flags & crate::spectator::smooth::ANGLES != 0 {
        // A negative `cl_vsmoothing` is reset to 0, and TS reads the cvar again for the lag
        // (`0x10067b96..0x10067ba8`), so this frame then lags by 0.
        if vsmoothing < 0.0 {
            e.cvar_set(c"cl_vsmoothing", 0.0);
            vsmoothing = 0.0;
        }
        if let Some(a) = with(|w| w.ang_ring.interpolate_angles(p.time - vsmoothing)) {
            p.cl_viewangles = a;
            crate::spectator::store_smoothed_angles(p.cl_viewangles);
        }
    }

    // TS's per-frame additions to `pparams->cl_viewangles` (`0x10067c94..0x10067d31`), in its
    // order: the kick step, the view-shake step, then a tenth of the bob pitch. There is no engine
    // call: the engine copies `cl_viewangles` back into its view angles after `V_CalcRefdef` (FWGS
    // `V_GetRefParams`), so all three are kept and add up from frame to frame. The bob pitch is
    // added whole every frame, so the view pitch integrates it: frame-rate dependent, and the aim
    // drifts by whatever the sum is when the bob stops or its phase resets, as in TS.
    with(|w| {
        let kick = apply_kick(&mut w.kick_target, &mut w.kick_cur, ft);
        p.cl_viewangles = v::add(p.cl_viewangles, kick);
        let shake = w.shake.update(ts * ft);
        p.cl_viewangles = v::add(p.cl_viewangles, shake);
        // `0x10067d14`: `cl_viewangles.pitch += bobpitch · 0.1f` (`0x1007a84c`) on the x87, then
        // the bob pitch is zeroed.
        let pitch = std::mem::take(&mut w.bob.pitch);
        p.cl_viewangles[0] = add_bob_pitch(p.cl_viewangles[0], pitch);
    });
    p.viewangles = p.cl_viewangles;

    cl_calc_shake(p);
    p.vieworg = v::add(p.vieworg, [1.0 / 32.0; 3]);

    // Water offset (SDK).
    let mut water_ofs = 0.0;
    if p.waterlevel >= 2 {
        let wd = waterdist as i32;
        let mut point = p.vieworg;
        if p.waterlevel == 2 {
            point[2] -= wd as f32;
            for _ in 0..wd {
                if r::point_contents(point) > CONTENTS_WATER {
                    break;
                }
                point[2] += 1.0;
            }
            water_ofs = point[2] + wd as f32 - p.vieworg[2];
        } else {
            point[2] += wd as f32;
            for _ in 0..wd {
                if r::point_contents(point) <= CONTENTS_WATER {
                    break;
                }
                point[2] -= 1.0;
            }
            water_ofs = point[2] - wd as f32 - p.vieworg[2];
        }
    }
    p.vieworg[2] += water_ofs;

    // `V_CalcViewRoll`: the ruleset's, from the view entity and the local player.
    let roll_of = |e: &cl_entity_s| RollEntity { angles: e.angles, iuser1: e.prevstate.iuser1, iuser4: e.prevstate.iuser4 };
    let roll_view = engine().entity(p.viewentity).map(roll_of);
    let roll_local = engine().local_player().map(roll_of);
    with(|w| rules.view_roll(&mut w.roll, p, roll_view, roll_local));
    // `V_AddIdle` (`0x10067eba`) at `v_idlescale`, the HUD's concussion level (0 in TS games).
    let idlescale = with(|w| w.idlescale);
    add_idle(&mut p.viewangles, p.time, idlescale);
    let mut eye = p.vieworg;
    // TS takes this frame's vectors from `cl_viewangles` (`0x10067ebf`), so without the shake
    // and stunt roll `viewangles` has by now.
    let (fwd, right, up) = v::angle_vectors(p.cl_viewangles);
    (p.forward, p.right, p.up) = (fwd, right, up);

    // Third-person camera (§7.4). `CL_CameraOffset`'s pitch and yaw with roll 0 (`camAngles`)
    // become the view angles at the end (`0x100689ef`).
    let mut third_angles = None;
    if third {
        let ent_org = engine().local_player().map_or(p.simorg, |e| e.origin);
        let vh = p.viewheight[2];
        let state = CamInput {
            camtype,
            cam_back: cb,
            cam_right: cr,
            cam_up: cu,
            ducked: vh < 28.0 && vh != 0.0,
            alive: !dead,
            stunt: iuser4 & 0x130c30 != 0,
            ft,
        };
        let (back, right_ofs, up_ofs) = with(|w| rules.camera_offsets(&mut w.cam_ofs_smooth, &state));
        // TS (`0x10067f53`) orbits the camera along `CL_CameraOffset`'s pitch and yaw, roll 0;
        // `cam_think` keeps that following the view.
        let cam_angles = with(|w| [w.cam_ofs[0], w.cam_ofs[1], 0.0]);
        let (cf, cr2, cu2) = v::angle_vectors(cam_angles);
        if back == 0.0 {
            let org = body_cam(v::add(ent_org, p.viewheight), state.ducked, cam_angles[0], cf, cu2);
            // TS runs `V_SmoothStairs` a second time this frame, on the body cam (`0x10068313`),
            // advancing the same state: see the client-rendering doc.
            let z = with(|w| rules.body_cam_z(&mut w.eye_z, org[2], on_ground, ft));
            p.vieworg = [org[0], org[1], z];
        } else {
            let want = v::add(v::add(v::ma(p.vieworg, back, cf), v::scale(cu2, up_ofs)), v::scale(cr2, right_ofs));
            let mut start = v::add(ent_org, [0.0, 0.0, up_ofs + 28.0]);
            if r::point_contents(start) == CONTENTS_SOLID {
                start = ent_org;
            }
            // `PM_TraceLine(start, vieworg, PM_TRACELINE_PHYSENTSONLY, 2, -1)` (`0x10068410`).
            let tr = r::pm_trace_line(start, want, 0, 2, -1);
            p.vieworg = if tr.fraction != 1.0 { v::add(v::ma(tr.endpos, 4.0, tr.plane.normal), [0.0, 0.0, 1.0]) } else { want };
        }
        third_angles = Some(cam_angles);
        engine().with_local_player_mut(|e| {
            // The model's z eases at rate 16 (TS `0x10191d94`) so stair steps and landings don't
            // jerk the body in front of the camera.
            let z = with(|w| {
                w.model_z = smooth_model_z(w.model_z, e.origin[2], ft);
                w.model_z
            });
            e.origin[2] = z;
            eye = v::add(e.origin, p.viewheight);
            let pitch = model_pitch(cam_angles[0]);
            e.angles[0] = pitch;
            e.curstate.angles[0] = pitch;
            e.prevstate.angles[0] = pitch;
            e.latched.prevangles[0] = pitch;
        });
    }

    // View model (`0x1006850e`): `V_CalcGunAngle`, placed at `vieworg` (the finished first-person
    // eye: smoothed view height and stairs, shake, the 1/32 nudge, water; in third person the
    // camera, where the engine doesn't draw the gun), shaken, then the world-space bob offset and
    // the fov push.
    let mut vm_ent = r::view_model();
    if let Some(vm) = vm_ent.as_deref_mut() {
        calc_gun_angle(vm, p.viewangles, bob);
        let mut org = p.vieworg;
        let mut ang = vm.angles;
        goldsrc::ext::spectator::apply_shake(&mut org, &mut ang, 0.9);
        vm.angles = ang;
        org = v::add(org, gun_ofs);
        org = v::ma(org, view_model_fov_push(fov, crate::studio::style().view_model_fov_push), fwd);
        org[2] -= 1.0;
        org[2] += match p.viewsize as i32 {
            110 => 1.0,
            100 => 2.0,
            90 => 1.0,
            80 => 0.5,
            _ => 0.0,
        };
        vm.origin = org;
    }

    // Punch (`0x10068654`; the server never sets it in TS). TS also adds `ev_punchangle`
    // (`0x10191480`) and decays it with `V_DropPunchAngle` (`0x10069e40`), but nothing in the TS
    // client ever makes it non-zero (its only store is the zeroing at `0x1000b9e4`).
    p.viewangles = v::add(p.viewangles, p.punchangle);

    // `cl_vsmoothing` (step 14, SDK `ViewInterp`): in multiplayer with the server's `pushmsec`
    // smoothing request, the view lags `cl_vsmoothing` seconds behind along the recorded simorg
    // path, so lifts and trains don't jitter. Jumps of 64 units or more are not smoothed. The
    // spectator origin flag also turns it on (`0x1006878f`: `flags & 4 || (smoothing && maxclients
    // > 1)`), and a smoothed origin becomes the spectator camera's origin. Read again: the angle
    // ring may have just reset a negative value.
    let vsmoothing = cv(|c| c.vsmoothing);
    let smooth = vsmoothing != 0.0 && (ov.smooth_flags & crate::spectator::smooth::ORIGIN != 0 || (p.smoothing != 0 && p.maxclients > 1));
    if smooth && vsmoothing < 0.0 {
        e.cvar_set(c"cl_vsmoothing", 0.0);
    }
    let sample = with(|w| {
        if v::len(v::sub(p.simorg, w.last_simorg)) != 0.0 {
            w.org_ring.push(p.simorg, p.time);
            w.last_simorg = p.simorg;
        }
        if smooth { w.org_ring.sample(p.time - vsmoothing) } else { None }
    });
    if let Some((org, _)) = sample.filter(|&(_, seg)| seg < 64.0) {
        let d = v::sub(org, p.simorg);
        p.simorg = v::add(p.simorg, d);
        p.vieworg = v::add(p.vieworg, d);
        if let Some(vm) = vm_ent {
            vm.origin = v::add(vm.origin, d);
        }
        // `0x10068974`.
        crate::spectator::store_smoothed_origin(p.simorg);
    }

    // `v_angles` keeps the angles from before the third-person and view-entity overrides
    // (`0x100689d5`).
    let mut v_angles = p.viewangles;
    if let Some(a) = third_angles {
        p.viewangles = a;
    }
    // A view entity other than a player (`SET_VIEW`, e.g. `trigger_camera`) takes over the view
    // (`0x10068a73`), `v_angles` included.
    if p.viewentity > p.maxclients
        && let Some(ent) = engine().entity(p.viewentity)
    {
        p.vieworg = ent.origin;
        p.viewangles = ent.angles;
        v_angles = ent.angles;
    }

    with(|w| {
        w.eye = eye;
        w.playback_angles = p.viewangles;
        finish(w, p, v_angles);
    });
}

/// How far the view model moves along the view for the zoom (`0x100685a7..0x100685bb`):
/// `(80 − m_iFOV)` (`fild`) `× 0.15f`, back above FOV 80. 0 without
/// [`StudioStyle::view_model_fov_push`](crate::studio::StudioStyle::view_model_fov_push) (TS 2.1
/// `V_CalcNormalRefdef` 0x100b6c90 has no FOV term, issue #258).
fn view_model_fov_push(fov: f32, push: bool) -> f32 {
    if push { ((80.0 - fov) as i32) as f32 * 0.15 } else { 0.0 }
}

/// The local model's pitch in third person (`0x100689ef..0x10068a64`): the camera pitch brought
/// into [-180, 180] by one turn (not a modulo), inverted and divided by 3.
fn model_pitch(pitch: f32) -> f32 {
    let pitch = if pitch > 180.0 {
        pitch - 360.0
    } else if pitch < -180.0 {
        pitch + 360.0
    } else {
        pitch
    };
    pitch * (-1.0 / 3.0)
}

fn cl_calc_shake(p: &mut ref_params_t) {
    goldsrc::ext::spectator::calc_shake();
    goldsrc::ext::spectator::apply_shake(&mut p.vieworg, &mut p.viewangles, 1.0);
}

/// Slow-motion tint + `TSFade` blend (end of `V_CalcRefdef`).
pub fn fade_color(ts: f32, slowfade: bool, fade: Option<(f32, state_fade::Fade)>, t: f32) -> [u8; 4] {
    // `__ftol` keeps the low byte of the truncated value (`0x10069c32`, `0x10069d5f`..).
    let byte = |x: f64| (x as i64) as u8;
    let tint_a = if slowfade && ts < 1.0 { byte((1.0 - ts as f64) * 50.0) } else { 0 };
    let tint = if slowfade && ts < 1.0 { [0u8, 100, 125, tint_a] } else { [0; 4] };
    // `0x10069c97..0x10069cbd`: after the start and before start + duration (no clamps on the
    // durations: TS divides by the ramp even when it is 0 or negative).
    if let Some((start, f)) = fade
        && t > start
        && start as f64 + f.duration as f64 > t as f64
    {
        let half = ((f.duration - f.hold) as f64 * 0.5) as f32;
        let e = t - start;
        let k = if e < half {
            e as f64 / half as f64
        } else if (half as f64 + f.hold as f64) < e as f64 {
            1.0 - (e as f64 - f.hold as f64 - half as f64) / half as f64
        } else {
            1.0
        };
        let [r, g, b, a] = f.rgba.map(f64::from);
        let a_scaled = a * f64::from(1.0f32 / 255.0);
        return [
            byte(r * k), // red has no tint
            byte(g * k + tint[1] as f64 * (1.0 - k)),
            byte(b * k + tint[2] as f64 * (1.0 - k)),
            byte(tint[3] as f64 * (1.0 - k) + k * a_scaled * 255.0),
        ];
    }
    tint
}

/// The end of `V_CalcRefdef` (`0x10069bd8..0x10069e20`), every frame: the engine's fade is read
/// back (`pfnGetScreenFade`) and only its colour and flags (`FFADE_OUT | FFADE_STAYOUT`) are
/// replaced, so its speed and end times stay the engine's.
fn write_screen_fade(time: f32) {
    let ts = state::with(|s| s.time_scale);
    // A ruleset without the tint (`ClientRules::slowmo_tint`, issue #152) keeps the `TSFade` blend.
    let slowfade = crate::gameplay::current().slowmo_tint() && cv(|c| c.slowfade) != 0.0;
    let fade = with(|w| w.fade);
    let c = fade_color(ts, slowfade, fade, time);
    let mut sf = r::get_screen_fade();
    (sf.fader, sf.fadeg, sf.fadeb, sf.fadealpha) = (c[0], c[1], c[2], c[3]);
    sf.fadeFlags = (FFADE_OUT | FFADE_STAYOUT) as i32;
    r::set_screen_fade(&mut sf);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `VKickback`'s shake: full strength for back-to-back shots, fading over 0.15 s of
    /// prediction time (scaled by the time scale, 0 = 1), none after.
    #[test]
    fn kick_shake_amplitude() {
        assert_eq!(shake_amplitude(10.0, 10.0, 1.0), Some(1.0));
        assert_eq!(shake_amplitude(10.1, 10.0, 0.0), shake_amplitude(10.1, 10.0, 1.0));
        let a = shake_amplitude(10.1, 10.0, 1.0).unwrap();
        assert!((a - 0.75).abs() < 1e-3, "{a}");
        let slow = shake_amplitude(10.1, 10.0, 0.5).unwrap();
        assert!(slow == 1.0, "slow motion stretches the window: {slow}");
        assert_eq!(shake_amplitude(10.2, 10.0, 1.0), None);
        // After a gun Spawn (last kick 0) any later time gives no shake.
        assert_eq!(shake_amplitude(10.0, 0.0, 1.0), None);
    }

    /// `V_CalcGunAngle`: bob minus pitch, yaw, and the view roll.
    #[test]
    fn gun_angle_takes_bob_and_roll() {
        assert_eq!(gun_angles([10.0, 90.0, 5.0], 2.0), [-8.0, 90.0, 5.0]);
        assert_eq!(gun_angles([-20.0, 0.0, 0.0], 0.0), [20.0, 0.0, 0.0]);
    }

    #[test]
    fn ring_sample_pair() {
        let mut r = Ring::default();
        r.push([0.0, 0.0, 0.0], 1.0);
        r.push([10.0, 20.0, 0.0], 2.0);
        r.push([10.0, 20.0, 30.0], 3.0);
        assert_eq!(r.sample_pair(1.5), Some(([0.0; 3], [10.0, 20.0, 0.0], 0.5)));
        assert_eq!(r.sample(1.5), Some(([5.0, 10.0, 0.0], 500f32.sqrt())));
        assert_eq!(r.sample_pair(0.5), None, "history doesn't reach back");
    }

    #[test]
    fn move_toward_matches_sdk() {
        assert_eq!(move_toward(10.0, 10.0), 10.0);
        assert_eq!(move_toward(0.0, 40.0), 10.0, "a quarter of the way");
        assert_eq!(move_toward(39.5, 40.0), 40.0, "snaps within 1 degree");
        assert_eq!(move_toward(80.0, 40.0), 70.0);
        // The short way round, then wrapped into [0, 360).
        assert_eq!(move_toward(350.0, 10.0), 355.0);
        assert_eq!(move_toward(10.0, 350.0), 5.0);
        assert_eq!(move_toward(270.0, -90.0), 270.0, "the same angle settles");
        assert_eq!(move_toward(-30.0, -30.0), 330.0);
    }

    #[test]
    fn cam_ofs_follows_view() {
        let view = [20.0, 100.0, 0.0];
        // Snapped: view + ideal, distance from cam_idealdist.
        assert_eq!(cam_ofs_step([0.0, 0.0, 30.0], view, [5.0, 90.0, 100.0], true), [25.0, 190.0, 100.0]);
        // Eased: each axis a quarter of the way; the distance isn't eased.
        assert_eq!(cam_ofs_step([0.0, 60.0, 30.0], view, [0.0, 0.0, 100.0], false), [5.0, 70.0, 100.0]);
        // Already at view + ideal: unchanged (not even wrapped).
        assert_eq!(cam_ofs_step([-10.0, 100.0, 30.0], [-10.0, 100.0, 0.0], [0.0; 3], false), [-10.0, 100.0, 0.0]);
    }

    /// [`ease_fov`] without the snap.
    fn ease(cur: f32, target: f32, ft: f32) -> f32 {
        ease_fov(cur, target, ft, false)
    }

    #[test]
    fn fov_easing() {
        let mut f = 90.0;
        let mut n = 0;
        while f != 20.0 && n < 1000 {
            f = ease(f, 20.0, 0.016);
            n += 1;
        }
        assert_eq!(f, 20.0);
        assert!(n > 3, "zoom is animated");
        // `UTIL_ApproachFOV`: half the gap (integer), times ft * 50, at least 1, truncated.
        // 0.016f is a hair above 0.016, so 90 - 28.000001 truncates to 61, as on the x87.
        assert_eq!(ease(90.0, 20.0, 0.016), 61.0);
        assert_eq!(ease(20.0, 90.0, 0.016), 48.0);
        assert_eq!(ease(21.0, 20.0, 0.016), 20.0, "at least one degree");
        assert_eq!(ease(22.0, 20.0, 0.0), 21.0);
        assert_eq!(ease_fov(0.0, 35.0, 0.016, true), 35.0, "a snap jumps to the target");
        assert_eq!(ease_fov(85.0, 20.0, 0.016, true), 20.0);
        assert_eq!(ease(85.0, 20.0, 1.0), 20.0, "never passes the target");
        // A server FOV of any value (the `int` difference wraps as in C, no overflow).
        for (cur, target) in [(90.0, -3e9), (90.0, 3e9), (3e9, -3e9), (-3e9, 3e9), (90.0, f32::NAN), (f32::INFINITY, f32::NEG_INFINITY)] {
            for ft in [0.0, 0.016, f32::NAN, f32::INFINITY, -1.0] {
                for snap in [false, true] {
                    ease_fov(cur, target, ft, snap);
                }
            }
        }
    }

    #[test]
    fn fade_blend() {
        assert_eq!(fade_color(1.0, true, None, 0.0), [0, 0, 0, 0]);
        assert_eq!(fade_color(0.5, true, None, 0.0), [0, 100, 125, 25]);
        assert_eq!(fade_color(0.5, false, None, 0.0), [0; 4]);
        let f = state_fade::Fade { duration: 2.0, hold: 1.0, rgba: [255, 255, 255, 200] };
        assert_eq!(fade_color(1.0, true, Some((10.0, f)), 11.0), [255, 255, 255, 200]);
        let mid = fade_color(0.5, true, Some((10.0, f)), 10.25);
        assert_eq!(mid[0], 127);
        assert_eq!(fade_color(1.0, true, Some((10.0, f)), 13.0), [0; 4]);
        // Ramp out: 1 - ((e - hold) - half) / half.
        assert_eq!(fade_color(1.0, true, Some((10.0, f)), 11.75)[0], 127);
        // No ramps (hold = duration): full colour from the start, no clamped ramp.
        let held = state_fade::Fade { duration: 1.0, hold: 1.0, rgba: [40, 50, 60, 70] };
        assert_eq!(fade_color(1.0, true, Some((10.0, held)), 10.0005), [40, 50, 60, 70]);
        // Mid-fade the slow-motion tint shows through the other channels, not red.
        assert_eq!(fade_color(0.5, true, Some((10.0, f)), 10.25), [127, 177, 190, 112]);
    }

    fn drift_input(forwardmove: f32, now: f32) -> DriftInput {
        DriftInput {
            forwardmove,
            forwardspeed: 400.0,
            centermove: 0.15,
            centerspeed: 500.0,
            frametime: 0.05,
            idealpitch: 0.0,
            now,
            ..Default::default()
        }
    }

    #[test]
    fn pitch_drift() {
        // Not stopped yet: drifts toward the ideal pitch, accelerating, without overshoot.
        let mut d = Drift::default();
        let mut pitch = 10.0;
        d.update(&drift_input(0.0, 1.0), &mut pitch);
        assert_eq!(pitch, 10.0, "first step has no velocity");
        for _ in 0..20 {
            d.update(&drift_input(0.0, 1.0), &mut pitch);
        }
        assert_eq!(pitch, 0.0);
        // Stopped (mouse look): only restarts after walking forward for v_centermove seconds, and
        // not in the frame something stopped it.
        let mut d = Drift::default();
        d.stop(2.0);
        let mut pitch = -20.0;
        for _ in 0..10 {
            d.update(&drift_input(400.0, 2.0), &mut pitch);
        }
        assert!(d.nodrift && pitch == -20.0, "stopped this frame");
        d.update(&drift_input(0.0, 2.1), &mut pitch);
        assert_eq!(d.driftmove, 0.0, "standing still resets the walk timer");
        for _ in 0..3 {
            d.update(&drift_input(400.0, 2.1), &mut pitch);
        }
        assert!(d.nodrift, "0.15 s not reached yet");
        d.update(&drift_input(400.0, 2.1), &mut pitch);
        assert!(!d.nodrift && d.pitchvel == 500.0);
        d.update(&drift_input(400.0, 2.1), &mut pitch);
        assert!(pitch > -20.0);
        let mut blocked = drift_input(400.0, 2.1);
        blocked.blocked = true;
        d.update(&blocked, &mut pitch);
        assert_eq!(d.pitchvel, 0.0);
    }

    #[test]
    fn centerview_starts_drift() {
        // `centerview` restarts a stopped drift at once, but not in the frame it was stopped.
        let mut d = Drift::default();
        d.stop(3.0);
        d.start(3.0, 500.0);
        assert!(d.nodrift && d.pitchvel == 0.0);
        d.start(3.1, 500.0);
        assert!(!d.nodrift && d.pitchvel == 500.0 && d.driftmove == 0.0);
        // Already drifting with some speed: left alone.
        d.pitchvel = 700.0;
        d.start(3.2, 500.0);
        assert_eq!(d.pitchvel, 700.0);
        let mut pitch = 30.0;
        for _ in 0..20 {
            d.update(&drift_input(0.0, 3.3), &mut pitch);
        }
        assert_eq!(pitch, 0.0);
    }

    #[test]
    fn view_model_zoom_push() {
        assert_eq!(view_model_fov_push(90.0, true), -1.5, "pulled back at FOV 90");
        assert_eq!(view_model_fov_push(80.0, true), 0.0);
        assert_eq!(view_model_fov_push(20.0, true), 60.0 * 0.15);
        assert_eq!(view_model_fov_push(20.0, false), 0.0, "TS 2.1: no push");
    }

    #[test]
    fn third_person_model_pitch() {
        assert_eq!(model_pitch(30.0), -10.0);
        assert_eq!(model_pitch(330.0), 10.0, "one turn down");
        assert_eq!(model_pitch(-300.0), -20.0, "one turn up");
        assert_eq!(model_pitch(-180.0), 60.0, "-180 is kept");
    }

    #[test]
    fn refdef_dispatch() {
        assert_eq!(refdef_path(false, false, false), RefdefPath::Normal);
        assert_eq!(refdef_path(false, false, true), RefdefPath::None, "paused: no normal view");
        assert_eq!(refdef_path(false, true, true), RefdefPath::Spectator, "spectating ignores pause");
        assert_eq!(refdef_path(true, true, true), RefdefPath::Intermission, "intermission first");
    }

    #[test]
    fn origin_ring() {
        let mut r = Ring::default();
        assert!(r.sample(1.0).is_none());
        r.push([0.0; 3], 1.0);
        r.push([10.0, 0.0, 0.0], 1.1);
        r.push([20.0, 0.0, 0.0], 1.2);
        let (p, seg) = r.sample(1.05).unwrap();
        assert!((p[0] - 5.0).abs() < 1e-3 && seg == 10.0);
        // Newer than the newest pair's start: lerps the last segment, clamped at its end.
        let (p, _) = r.sample(1.15).unwrap();
        assert!((p[0] - 15.0).abs() < 1e-3);
        assert!(r.sample(0.5).is_none(), "history doesn't reach back that far");
        for i in 0..200 {
            r.push([i as f32, 0.0, 0.0], 2.0 + i as f32 * 0.01);
        }
        let (p, _) = r.sample(2.0 + 190.5 * 0.01).unwrap();
        assert!((p[0] - 190.5).abs() < 1e-2, "{p:?}");
    }

    #[test]
    fn local_translucency() {
        let mut a = 255.0;
        for _ in 0..3 {
            a = local_amt_step(a, true, true, true);
        }
        assert_eq!(a, 105.0, "50 per frame fading out");
        a = local_amt_step(a, true, true, true);
        assert_eq!(a, 100.0);
        assert_eq!(local_amt_step(a, true, false, true), 105.0, "cl_thirdtransparency 0 fades back in");
        assert_eq!(local_amt_step(a, false, true, true), 105.0, "5 per frame fading in");
        assert_eq!(local_amt_step(a, false, true, false), 150.0, "50 when the frame has no time");
    }

    #[test]
    fn body_cam_and_model_z() {
        let (fwd, up) = ([1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
        assert_eq!(body_cam([0.0; 3], false, 0.0, fwd, up), [0.0, 0.0, 1.0]);
        assert_eq!(body_cam([0.0; 3], true, 0.0, fwd, up), [8.0, 0.0, 8.0]);
        let down = body_cam([0.0; 3], false, 90.0, fwd, up);
        assert!((down[0] - 4.0).abs() < 1e-4 && (down[2] - 19.0).abs() < 1e-4, "{down:?}");
        assert_eq!(body_cam([0.0; 3], false, -45.0, fwd, up), [0.0, 0.0, 1.0], "looking up adds nothing");
        assert_eq!(smooth_model_z(10.0, 0.0, 0.01), 10.0 - 1.6);
        assert_eq!(smooth_model_z(10.0, 0.0, 1.0), 0.0, "no overshoot");
    }

    #[test]
    fn idle_sway() {
        let mut a = [0.0; 3];
        add_idle(&mut a, 1.0, 0.0);
        assert_eq!(a, [0.0; 3]);
        add_idle(&mut a, std::f32::consts::FRAC_PI_2, 1.0);
        assert!((a[0] - 0.3).abs() < 1e-6 && a[1].abs() < 1e-6 && (a[2] - 0.1 * (std::f32::consts::FRAC_PI_4).sin()).abs() < 1e-6);
    }

    #[test]
    fn shake_settles() {
        let mut s = Shake { target: [1.0, -1.0, 0.0], time_left: 0.1, freq: 35.0, flip: 1.0 / 35.0, ..Default::default() };
        let mut total = v::ZERO;
        for _ in 0..400 {
            total = v::add(total, s.update(0.01));
        }
        assert!(v::len(total) < 1e-3, "net view change returns to zero: {total:?}");
    }

    /// `V_ApplyKick`: eases toward the target at min(ft·15, 1); within 0.1° everything is zeroed.
    #[test]
    fn kick_eases_then_settles() {
        let (mut target, mut cur) = ([-4.0f32, 2.0], [0.0f32; 2]);
        let k = 0.01f32 * 15.0;
        let step = apply_kick(&mut target, &mut cur, 0.01);
        assert_eq!(step, [-4.0 * k, 2.0 * k, 0.0]);
        assert_eq!(cur, [step[0], step[1]]);
        let rest = [target[0] - cur[0], target[1] - cur[1]];
        let step = apply_kick(&mut target, &mut cur, 1.0);
        assert_eq!(cur, target, "ft·15 is capped at 1");
        assert_eq!(step, [rest[0], rest[1], 0.0]);
        let (mut target, mut cur) = ([0.05f32, 0.05], [0.0f32; 2]);
        assert_eq!(apply_kick(&mut target, &mut cur, 0.01), v::ZERO);
        assert_eq!((target, cur), ([0.0; 2], [0.0; 2]));
    }

    /// TS adds a tenth of the bob pitch to `cl_viewangles` every frame, and the engine keeps it,
    /// so the view pitch is the running sum: it grows with the frame count, not the time.
    #[test]
    fn bob_pitch_accumulates_in_view_angles() {
        // A bob pitch that depends only on the time, as a ruleset's bob gives at a steady speed.
        let run = |frames: usize, ft: f32| {
            let mut pitch = 0.0f32;
            for n in 1..=frames {
                let bob_pitch = (n as f32 * ft * 10.0).sin() * 0.75;
                pitch = add_bob_pitch(pitch, bob_pitch);
            }
            pitch
        };
        // The same 0.1 s of running: ten times the frames, about ten times the pitch.
        let slow = run(10, 0.01);
        let fast = run(100, 0.001);
        assert!(slow > 0.0 && fast > 5.0 * slow, "{slow} {fast}");
        assert_eq!(add_bob_pitch(10.0, 3.0), (3.0f64 * f64::from(0.1f32) + 10.0) as f32);
    }
}
