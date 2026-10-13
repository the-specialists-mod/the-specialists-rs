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
use shared::ts::stunt;
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
    /// Smoothed view height (`0x10191d90`). Zero-initialised and never reset, as in TS, so the
    /// first spawn after the library loads eases the eye up from the origin.
    vh: f32,
    stairs: Stairs,
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
    /// Eased additions to the camera's up (`0x10191da4`) and back (`0x10191da8`) offsets.
    cam_up_ofs: f32,
    cam_back_ofs: f32,
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
/// 0, 0 and -1, so the first call counts as "was on the ground".
#[derive(Clone, Copy, Debug, PartialEq)]
struct Stairs {
    smooth_z: f32,
    ofs: f32,
    was_on_ground: bool,
}

impl Default for Stairs {
    fn default() -> Self {
        Stairs { smooth_z: 0.0, ofs: 0.0, was_on_ground: true }
    }
}

impl Stairs {
    /// `V_SmoothStairs` (`0x100677d0`), on the eye z (`vieworg.z`, view height already added).
    /// On the ground the smoothed z follows at `min(15·ft, 1)` and the offset is zeroed; in the
    /// air it is the eye z, plus an offset set on the frame the ground is left to
    /// `z − smoothed z` (that sign is TS's, from the assembly: `fld [z]; fsub [0x10191d84]`) and
    /// decayed at the same rate after each use. No snap: TS has none.
    fn smooth(&mut self, z: f32, on_ground: bool, ft: f32) -> f32 {
        let k = (15.0 * ft).min(1.0);
        if on_ground {
            self.ofs = 0.0;
            self.smooth_z += (z - self.smooth_z) * k;
        } else {
            if self.was_on_ground {
                self.ofs = z - self.smooth_z;
            }
            self.smooth_z = z;
        }
        let out = self.smooth_z + self.ofs;
        self.ofs *= 1.0 - k;
        self.was_on_ground = on_ground;
        out
    }
}

/// View-height easing in `V_CalcNormalRefdef` (`0x100679ee..0x10067a43`):
/// `vh += (target − vh)·min(15·ft, 1)`.
fn ease_view_height(vh: f32, target: f32, ft: f32) -> f32 {
    vh + (target - vh) * (15.0 * ft).min(1.0)
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

/// `ts_test_view`: prints the view state (developer aid).
extern "C" fn cmd_test_view() {
    let s = with(|w| {
        format!(
            "third {} cam_back {:.1} cam_up {:.1} model_z {:.1} amt {:.0} origin {:?} eye {:?} angles {:?}\n",
            w.third, w.cam_back_ofs, w.cam_up_ofs, w.model_z, w.local_amt, w.cam.origin, w.eye, w.cam.angles
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
    kick_angles(pitch, yaw, silenced, now);
}

/// Queues one shot's view kick (`pitch` up, `yaw` right, degrees) and starts the shake spring.
/// Call once per shot, with `runfuncs` only. `now` is the command's prediction time: TS times the
/// shake with `*0x10099114`, which `HUD_WeaponsPostThink` sets to `HUD_PostRunCmd`'s `time` as a
/// float, not with the client clock.
pub fn kick_angles(pitch: f32, yaw: f32, silenced: bool, now: f32) {
    let ts = state::with(|s| s.time_scale);
    with(|w| {
        w.kick_target[0] -= pitch;
        w.kick_target[1] += yaw;
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

/// `cl_bob`'s (gun offset level, pitch level) (`0x10066ea1..0x10066f00`). TS truncates the cvar
/// (`__ftol`) and switches on it unsigned (`cmp eax, 3; ja`), so 0..3 pick a row and everything
/// else, negative values included, is the full bob.
fn bob_levels(cl_bob: f32) -> (f32, f32) {
    // `__ftol` gives 0x80000000 for NaN and out-of-range values: the default row.
    let n = if cl_bob.is_nan() { u32::MAX } else { cl_bob as i32 as u32 };
    match n {
        0 => (0.1, 0.0),
        1 => (0.3, 0.0),
        2 => (0.5, 0.15),
        3 => (0.7, 0.5),
        _ => (1.0, 1.0),
    }
}

/// What `V_CalcBob` reads.
#[derive(Clone, Copy, Debug, Default)]
struct BobInput {
    /// Third person, `pparams->spectator`, or `PM_SpectatorMove` smoothing flags set without the
    /// "keep bobbing" bit (`0x10066e74..0x10066e9b`): return 0 and leave every static alone.
    skip: bool,
    cl_bob: f32,
    on_ground: bool,
    time: f32,
    frametime: f32,
    viewheight: f32,
    simvel: Vec3,
    /// `g_flTimeScale` (`0x100963e8`), unclamped.
    ts: f32,
    /// `pparams->forward/right/up` as the engine passes them in: the vectors of the previous
    /// frame's view angles (FWGS `V_SetRefParams`), since `V_CalcBob` runs before
    /// `V_CalcNormalRefdef` computes this frame's.
    forward: Vec3,
    right: Vec3,
    up: Vec3,
}

/// `V_CalcBob` (`0x10066e70`) and `V_SmoothBobZ` (`0x10066de0`) statics.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Bob {
    /// `pparams->time` of the last bobbing frame (`0x10191d70`).
    last_time: f32,
    /// Phase in degrees (`0x10191d74`), reset to 0 once past 360.
    phase: f32,
    /// Amplitude (`0x10191d6c`), also `V_CalcBob`'s return value.
    amp: f32,
    /// The amplitude last set from the speed (`0x10191d78`); below it the amplitude eases back.
    peak: f32,
    /// Smoothed z of the gun offset (`0x1019144c`).
    smooth_z: f32,
    /// World-space gun offset (`0x10191428`), added to the view model's origin. Left as it was
    /// on a skipped frame, as the in-eye gun also reads it.
    ofs: Vec3,
    /// Bob pitch (`0x10190b78`); `V_CalcNormalRefdef` consumes it and zeroes it every frame.
    pitch: f32,
}

impl Bob {
    /// `V_CalcBob` (`0x10066e70`): returns the amplitude, which `V_CalcGunAngle` adds to the gun's
    /// pitch, or 0 when skipped.
    /// * The phase and amplitude only advance on the ground, when the time has changed and the
    ///   view height isn't 0 (a stunt's `NO_VIEW`); otherwise the amplitude halves while above
    ///   0.01 and the gun offset is the forward vector part alone.
    /// * Advancing: `sp = min(|vel.xy|/300, 1.1)`, `phase += ft·5.5·ts·sp·100`, and the amplitude
    ///   is `|vel.xy|·0.01` (unclamped, which also sets the peak) unless it is below the peak,
    ///   when it eases back as `(amp + 2.5)·sp·0.5` and the peak is left alone.
    /// * The offset mixes the previous frame's vectors in world space:
    ///   `up·sin(2φ)·amp·0.11 + forward·(0.75 − amp/2) + right·sin(φ)·amp·0.3`, scaled by the
    ///   `cl_bob` level, with `0.5 − 0.65·amp` added to z first; z then eases at `ft·20`.
    fn calc(&mut self, i: &BobInput) -> f32 {
        if i.skip {
            return 0.0;
        }
        let (lvl, pitch_lvl) = bob_levels(i.cl_bob);
        let amp_part = |amp: f32| 0.75 - amp as f64 * 0.5;
        let z_part = |amp: f32| 0.5 - amp as f64 * 0.65;
        if i.on_ground && i.time != self.last_time && i.viewheight != 0.0 {
            self.last_time = i.time;
            let speed = ((i.simvel[1] as f64).powi(2) + (i.simvel[0] as f64).powi(2)).sqrt();
            let sp = (speed * f64::from(1.0f32 / 300.0)).min(f64::from(1.1f32));
            self.phase = ((i.frametime * 5.5 * i.ts) as f64 * sp * 100.0 + self.phase as f64) as f32;
            if self.phase > 360.0 {
                self.phase = 0.0;
            }
            if self.peak <= self.amp {
                self.amp = (speed * 0.01) as f32;
                self.peak = self.amp;
            } else {
                self.amp = ((self.amp + 2.5) as f64 * sp * 0.5) as f32;
            }
            let phase = self.phase as f64;
            let s2 = ((phase * (std::f64::consts::PI / 90.0)).sin() * self.amp as f64) as f32;
            let s1 = ((phase * (std::f64::consts::PI / 180.0)).sin() * self.amp as f64) as f32;
            let k = amp_part(self.amp);
            for a in 0..3 {
                self.ofs[a] = (s1 as f64 * i.right[a] as f64 * 0.3 + i.forward[a] as f64 * k + s2 as f64 * i.up[a] as f64 * 0.11) as f32;
            }
            self.ofs[2] = ((z_part(self.amp) + self.ofs[2] as f64) * lvl as f64) as f32;
            self.pitch = s2 * pitch_lvl * 0.25;
        } else {
            if self.amp as f64 > 0.01 {
                self.amp = (self.amp as f64 * 0.5) as f32;
            }
            let k = amp_part(self.amp);
            self.ofs = i.forward.map(|f| (f as f64 * k) as f32);
            // The forward z part is overwritten, not added to (`0x10067179..0x100671b9`).
            self.ofs[2] = (z_part(self.amp) * lvl as f64) as f32;
            self.pitch = 0.0;
        }
        self.ofs[0] *= lvl;
        self.ofs[1] *= lvl;
        self.smooth_z(i.frametime);
        self.amp
    }

    /// `V_SmoothBobZ` (`0x10066de0`): the smoothed z moves `(z_smooth − z)·ft·20` toward the
    /// offset's z, never past it, and becomes the offset's z.
    fn smooth_z(&mut self, ft: f32) {
        let d = self.smooth_z - self.ofs[2];
        let mut step = d * ft * 20.0;
        if step.abs() > d.abs() {
            step = d;
        }
        self.smooth_z -= step;
        self.ofs[2] = self.smooth_z;
    }
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

/// What `V_CalcViewRoll` reads from the view entity (`GetEntityByIndex(pparams->viewentity)`).
/// TS reads `prevstate` here (`+0x260`, `+0x26c`), not `curstate` (`+0x3c0`), so the stunt bits
/// lag the latest packet by one.
#[derive(Clone, Copy, Debug, Default)]
struct RollEntity {
    angles: Vec3,
    /// `prevstate.iuser1`: the observer mode.
    iuser1: i32,
    /// `prevstate.iuser4`: the stunt bits.
    iuser4: i32,
}

/// `V_CalcViewRoll`'s roll target and easing rate (`0x100675e8..0x10067632`): 0 at 15 on the
/// ground; in the air a dive 35 at 7.5, a flip -10 at 5, anything else -5.5 at 15.
fn view_roll_target(on_ground: bool, iuser4: i32) -> (f32, f32) {
    if on_ground {
        (0.0, 15.0)
    } else if iuser4 & stunt::DIVING != 0 {
        (35.0, 7.5)
    } else if iuser4 & (stunt::FLIP_HIGH | stunt::FLIP_LOW) != 0 {
        (-10.0, 5.0)
    } else {
        (-5.5, 15.0)
    }
}

/// `V_CalcRoll` (`0x100671f0`, SDK): `side = right(angles)·velocity`, then
/// `|side|·0.01 < rollspeed ? |side|·0.01·rollangle/rollspeed : rollangle`, with side's sign
/// (a side of 0 counts as positive). On the x87, with the 0.01 a double; returned unrounded.
fn calc_roll(angles: Vec3, vel: Vec3, rollangle: f32, rollspeed: f32) -> f64 {
    let (_, right, _) = v::angle_vectors(angles);
    let side = right[0] as f64 * vel[0] as f64 + right[1] as f64 * vel[1] as f64 + right[2] as f64 * vel[2] as f64;
    let sign = if side >= 0.0 { 1.0 } else { -1.0 };
    let side = if side > 0.0 { side } else { -side } * 0.01;
    if side < rollspeed as f64 { side * rollangle as f64 / rollspeed as f64 * sign } else { rollangle as f64 * sign }
}

/// `V_CalcViewRoll` (`0x100675b0`): the stunt roll. Nothing without a view entity; an observer
/// (`prevstate.iuser1 > 0`) gets roll 0. Otherwise the eased roll (`0x10191d80`) follows
/// `V_CalcRoll(ent->angles, simvel, target, 10)` at `min(rate·ft, 1)` and is added to the view
/// roll, so it scales with sideways speed and is 0 on the ground. Dead with a view height, the
/// roll is then set to 80 (after the easing has run).
fn calc_view_roll(roll: &mut f32, p: &mut ref_params_t, ent: Option<RollEntity>) {
    let Some(ent) = ent else { return };
    if ent.iuser1 > 0 {
        p.viewangles[2] = 0.0;
        return;
    }
    let (target, rate) = view_roll_target(p.onground != 0, ent.iuser4);
    let r = calc_roll(ent.angles, p.simvel, target, 10.0);
    let k = rate as f64 * p.frametime as f64;
    let k = if k > 1.0 { 1.0 } else { k };
    *roll = ((r - *roll as f64) * k + *roll as f64) as f32;
    p.viewangles[2] += *roll;
    if p.health < 1 && p.viewheight[2] != 0.0 {
        p.viewangles[2] = 80.0;
    }
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
    let spectating = p.spectator != 0 || state::with(|s| s.spec_mode) != 0;
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

/// Third-person camera offsets (back, right, up) from `cl_camtype`, clamped (§7.4).
fn cam_base(camtype: i32, cam_back: f32, cam_right: f32, cam_up: f32) -> (f32, f32, f32) {
    let (back, right, up) = match camtype {
        1 => (-48.0, -16.0, 12.0),
        2 => (-90.0, 0.0, 12.0),
        3 => (-cam_back, cam_right, cam_up),
        _ => (-48.0, 16.0, 12.0),
    };
    (back.clamp(-128.0, 24.0), right.clamp(-24.0, 24.0), up.clamp(-24.0, 24.0))
}

/// What the third-person offset smoothing reads.
#[derive(Clone, Copy, Debug)]
struct CamState {
    /// Ducked view (0 < view height < 28).
    ducked: bool,
    alive: bool,
    /// `iuser4 & 0x130c30`: dive, prone, flips, wall moves or super-jump glide.
    stunt: bool,
    ft: f32,
}

/// The smoothing half of the third-person block of `V_CalcNormalRefdef` (`0x10068139..0x1006823e`):
/// eases the extra up offset toward `32 - up` while ducked, and the extra back offset toward
/// -180 (dead) or -92 (stunt), both at `ft * 5` (unclamped). Returns the offsets to use.
fn cam_smooth(up_ofs: &mut f32, back_ofs: &mut f32, back: f32, up: f32, s: CamState) -> (f32, f32) {
    let up_target = if s.ducked && up < 32.0 { 32.0 - up } else { 0.0 };
    *up_ofs += (up_target - *up_ofs) * s.ft * 5.0;
    let back_target = if !s.alive {
        -180.0 - back
    } else if s.stunt && back > -128.0 {
        -92.0 - back
    } else {
        0.0
    };
    *back_ofs += (back_target - *back_ofs) * s.ft * 5.0;
    (back + *back_ofs, if s.alive { up + *up_ofs } else { up })
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
    let e = goldsrc::client::engine();
    let translucent = !state::with(|s| s.dead) || e.demo_playing() || e.demo_recording();
    let on_player = crate::hud::with(|h| h.crosshair_id.is_some());
    let transparency = cv(|c| c.thirdtransparency) as i32 != 0;
    // TS tests `gHUD.m_flTimeDelta` (`0x10067930`), the last HUD redraw's frame time.
    let hud_dt = crate::hud::time_delta();
    engine().with_local_player_mut(|ent| {
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
        forward: p.forward,
        right: p.right,
        up: p.up,
    };

    // `V_CalcBob` (`0x100679a5`) runs before the spectator camera replaces `simorg`.
    let (bob, gun_ofs) = with(|w| (w.bob.calc(&bob_in), w.bob.ofs));

    // The spectator camera origin replaces `simorg` (`0x100679aa`).
    if let Some(org) = ov.origin {
        p.simorg = org;
    }

    // vieworg = simorg with the smoothed view height on z (only z), then stair smoothing of that
    // eye z (`0x100679ee..0x10067a46`). During a stunt the server sends `view_ofs` 0 (`NO_VIEW`), so the eye
    // eases down to the hull centre like TS's.
    let on_ground = p.onground != 0;
    with(|w| {
        w.vh = ease_view_height(w.vh, p.viewheight[2], ft);
        let z = w.stairs.smooth(p.simorg[2] + w.vh, on_ground, ft);
        p.vieworg = [p.simorg[0], p.simorg[1], z];
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

    let roll_ent =
        engine().entity(p.viewentity).map(|e| RollEntity { angles: e.angles, iuser1: e.prevstate.iuser1, iuser4: e.prevstate.iuser4 });
    with(|w| calc_view_roll(&mut w.roll, p, roll_ent));
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
        let state = CamState { ducked: vh < 28.0 && vh != 0.0, alive: !dead, stunt: iuser4 & 0x130c30 != 0, ft };
        let (back, right_ofs, up_ofs) = cam_base(camtype, cb, cr, cu);
        let (back, up_ofs) = with(|w| cam_smooth(&mut w.cam_up_ofs, &mut w.cam_back_ofs, back, up_ofs, state));
        // TS (`0x10067f53`) orbits the camera along `CL_CameraOffset`'s pitch and yaw, roll 0;
        // `cam_think` keeps that following the view.
        let cam_angles = with(|w| [w.cam_ofs[0], w.cam_ofs[1], 0.0]);
        let (cf, cr2, cu2) = v::angle_vectors(cam_angles);
        if back == 0.0 {
            let org = body_cam(v::add(ent_org, p.viewheight), state.ducked, cam_angles[0], cf, cu2);
            // TS runs `V_SmoothStairs` a second time this frame, on the body cam (`0x10068313`),
            // advancing the same state: see the client-rendering doc.
            let z = with(|w| w.stairs.smooth(org[2], on_ground, ft));
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
        org = v::ma(org, ((80.0 - fov) as i32) as f32 * 0.15, fwd);
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
    let slowfade = cv(|c| c.slowfade) != 0.0;
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

    /// `V_CalcRoll`: proportional to the sideways speed below `rollspeed / 0.01`, the full
    /// `rollangle` above it, with the side's sign.
    #[test]
    fn calc_roll_scales_with_side_speed() {
        // Yaw 0: right is -y, so moving toward -y is moving right.
        assert!((calc_roll(v::ZERO, [0.0, -300.0, 0.0], 35.0, 10.0) - 10.5).abs() < 1e-4);
        assert!((calc_roll(v::ZERO, [0.0, 300.0, 0.0], 35.0, 10.0) + 10.5).abs() < 1e-4);
        assert_eq!(calc_roll(v::ZERO, [0.0, -2000.0, 0.0], -5.5, 10.0), -5.5);
        assert_eq!(calc_roll(v::ZERO, [300.0, 0.0, 0.0], 35.0, 10.0), 0.0, "straight ahead");
    }

    /// `V_CalcViewRoll`: no roll on the ground, a speed-scaled eased roll in the air, 0 for an
    /// observer, 80 when dead after the easing has run, nothing without a view entity.
    #[test]
    fn view_roll_follows_ts() {
        let ent = RollEntity::default();
        let mut p = ref_params_t { frametime: 0.01, onground: 1, health: 100, simvel: [0.0, -300.0, 0.0], ..Default::default() };
        let mut roll = 0.0;
        calc_view_roll(&mut roll, &mut p, Some(ent));
        assert_eq!((roll, p.viewangles[2]), (0.0, 0.0), "no strafe roll on the ground");

        p.onground = 0;
        calc_view_roll(&mut roll, &mut p, Some(RollEntity { iuser4: stunt::DIVING, ..ent }));
        // Target 10.5 (see above) at min(7.5 · 0.01, 1).
        assert!((roll - 10.5 * 0.075).abs() < 1e-4, "{roll}");
        assert_eq!(p.viewangles[2], roll);

        p.viewangles[2] = 3.0;
        calc_view_roll(&mut roll, &mut p, Some(RollEntity { iuser1: 4, ..ent }));
        assert_eq!(p.viewangles[2], 0.0, "observer");

        let before = roll;
        p.health = 0;
        p.viewheight = [0.0, 0.0, 8.0];
        calc_view_roll(&mut roll, &mut p, Some(ent));
        assert_eq!(p.viewangles[2], 80.0);
        assert_ne!(roll, before, "the easing still runs when dead");

        p.viewangles[2] = 3.0;
        calc_view_roll(&mut roll, &mut p, None);
        assert_eq!(p.viewangles[2], 3.0);
        assert_eq!(view_roll_target(false, stunt::FLIP_LOW), (-10.0, 5.0));
        assert_eq!(view_roll_target(false, 0), (-5.5, 15.0));
    }

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

    /// `cl_bob` truncates and switches unsigned: 0..3 pick a row, negatives get the full bob.
    #[test]
    fn bob_level_table() {
        assert_eq!(bob_levels(0.0), (0.1, 0.0));
        assert_eq!(bob_levels(1.0), (0.3, 0.0));
        assert_eq!(bob_levels(2.0), (0.5, 0.15));
        assert_eq!(bob_levels(3.9), (0.7, 0.5), "truncated");
        assert_eq!(bob_levels(4.0), (1.0, 1.0));
        assert_eq!(bob_levels(100.0), (1.0, 1.0));
        assert_eq!(bob_levels(-0.5), (0.1, 0.0), "truncates to 0");
        assert_eq!(bob_levels(-1.0), (1.0, 1.0), "unsigned compare");
        assert_eq!(bob_levels(f32::NAN), (1.0, 1.0));
    }

    fn bob_input(time: f32, speed: f32) -> BobInput {
        BobInput {
            skip: false,
            cl_bob: 4.0,
            on_ground: true,
            time,
            frametime: 0.01,
            viewheight: 28.0,
            simvel: [speed, 0.0, 0.0],
            ts: 1.0,
            forward: [1.0, 0.0, 0.0],
            right: [0.0, -1.0, 0.0],
            up: [0.0, 0.0, 1.0],
        }
    }

    #[test]
    fn bob_skips_without_touching_state() {
        let mut b = Bob { phase: 10.0, amp: 1.0, peak: 1.0, ofs: [1.0, 2.0, 3.0], pitch: 0.5, ..Bob::default() };
        let before = b;
        assert_eq!(b.calc(&BobInput { skip: true, ..bob_input(1.0, 200.0) }), 0.0);
        assert_eq!(b, before, "the gun offset is left stale");
    }

    /// The phase only advances on the ground, with a new time and a non-zero view height.
    #[test]
    fn bob_advances_only_when_it_should() {
        let mut b = Bob::default();
        let amp = b.calc(&bob_input(1.0, 200.0));
        assert!((amp - 2.0).abs() < 1e-5, "amplitude = speed * 0.01: {amp}");
        assert_eq!(b.peak, b.amp);
        // phase = ft * 5.5 * ts * min(200/300, 1.1) * 100
        assert!((b.phase - 0.01 * 5.5 * (200.0 / 300.0) * 100.0).abs() < 1e-4, "{}", b.phase);
        let phase = b.phase;
        // Same time: no advance, and the amplitude halves.
        b.calc(&bob_input(1.0, 200.0));
        assert_eq!(b.phase, phase);
        assert_eq!(b.amp, 1.0);
        assert_eq!(b.pitch, 0.0);
        // No view height (a stunt's NO_VIEW) or in the air: likewise.
        b.calc(&BobInput { viewheight: 0.0, ..bob_input(2.0, 200.0) });
        b.calc(&BobInput { on_ground: false, ..bob_input(3.0, 200.0) });
        assert_eq!(b.phase, phase);
        assert_eq!(b.amp, 0.25);
        // Halving stops at 0.01.
        b.amp = 0.01;
        b.calc(&BobInput { on_ground: false, ..bob_input(4.0, 200.0) });
        assert_eq!(b.amp, 0.01);
    }

    /// Below the peak the amplitude eases back as `(amp + 2.5) * sp * 0.5`, leaving the peak.
    #[test]
    fn bob_amplitude_eases_back_below_peak() {
        let mut b = Bob { amp: 0.5, peak: 3.0, ..Bob::default() };
        let amp = b.calc(&bob_input(1.0, 300.0));
        assert!((amp - (0.5 + 2.5) * 1.0 * 0.5).abs() < 1e-5, "{amp}");
        assert_eq!(b.peak, 3.0);
        // Reaching the peak again sets it from the speed.
        b.amp = 3.0;
        let amp = b.calc(&bob_input(2.0, 150.0));
        assert!((amp - 1.5).abs() < 1e-5, "{amp}");
        assert_eq!(b.peak, b.amp);
        // sp is capped at 1.1 in the phase step; the amplitude isn't.
        let mut b = Bob::default();
        let amp = b.calc(&bob_input(1.0, 600.0));
        assert!((amp - 6.0).abs() < 1e-5, "{amp}");
        assert!((b.phase - 0.01 * 5.5 * 1.1 * 100.0).abs() < 1e-4, "{}", b.phase);
    }

    /// Past 360 degrees the phase resets to 0 rather than wrapping.
    #[test]
    fn bob_phase_resets_past_360() {
        let mut b = Bob { phase: 359.0, ..Bob::default() };
        b.calc(&BobInput { frametime: 0.05, ..bob_input(1.0, 300.0) });
        assert_eq!(b.phase, 0.0);
        let mut b = Bob { phase: 355.0, ..Bob::default() };
        b.calc(&BobInput { frametime: 0.001, ..bob_input(1.0, 300.0) });
        assert!(b.phase > 355.0 && b.phase <= 360.0, "{}", b.phase);
    }

    /// The gun offset is a world-space mix of the given vectors, scaled by the level, with
    /// `0.5 - 0.65 amp` on z, and z eased at ft * 20.
    #[test]
    fn bob_gun_offset_is_world_space() {
        // Facing +y: the forward part lands on y, not x.
        let input = BobInput {
            on_ground: false,
            cl_bob: 2.0,
            frametime: 1.0,
            forward: [0.0, 1.0, 0.0],
            right: [1.0, 0.0, 0.0],
            ..bob_input(1.0, 0.0)
        };
        let mut b = Bob { amp: 0.01, ..Bob::default() };
        b.calc(&input);
        let k = 0.75 - 0.01 * 0.5;
        assert_eq!(b.ofs[0], 0.0);
        assert!((b.ofs[1] - k * 0.5).abs() < 1e-6, "{:?}", b.ofs);
        // ft * 20 >= 1: z snaps to (0.5 - 0.65 amp) * level (the forward z isn't added).
        assert!((b.ofs[2] - (0.5 - 0.65 * 0.01) * 0.5).abs() < 1e-6, "{:?}", b.ofs);

        // On the ground at phase 45: up * sin(90)*amp*0.11 + right * sin(45)*amp*0.3.
        let mut b = Bob { phase: 45.0 - 0.01 * 5.5 * 100.0, amp: 0.0, peak: 0.0, ..Bob::default() };
        let input = BobInput { frametime: 0.01, ..bob_input(1.0, 300.0) };
        b.calc(&input);
        assert!((b.phase - 45.0).abs() < 1e-4);
        let amp = 3.0f32;
        let s1 = 45f32.to_radians().sin() * amp;
        assert!((b.ofs[0] - (0.75 - amp / 2.0)).abs() < 1e-4, "{:?}", b.ofs);
        assert!((b.ofs[1] - -s1 * 0.3).abs() < 1e-4, "{:?}", b.ofs);
        let z = (amp * 0.11 + 0.5 - 0.65 * amp) * 0.01 * 20.0;
        assert!((b.ofs[2] - z).abs() < 1e-4, "eased a fifth of the way: {:?}", b.ofs);
        assert!((b.pitch - amp * 0.25).abs() < 1e-4, "bob pitch = sin(2 phase) amp 0.25: {}", b.pitch);
    }

    #[test]
    fn bob_z_smoothing_never_overshoots() {
        let mut b = Bob { smooth_z: 10.0, ofs: [0.0, 0.0, 0.0], ..Bob::default() };
        b.smooth_z(0.01);
        assert!((b.ofs[2] - 8.0).abs() < 1e-5, "{:?}", b.ofs);
        assert_eq!(b.smooth_z, b.ofs[2]);
        b.ofs[2] = 0.0;
        b.smooth_z(0.1);
        assert_eq!(b.ofs[2], 0.0, "ft * 20 > 1 lands on the target");
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

    /// [`ease_fov`] with TS 3.0's snap rule.
    fn ease30(cur: f32, target: f32, ft: f32) -> f32 {
        use crate::gameplay::ClientRules;
        ease_fov(cur, target, ft, shared::gameplay::CORE.fov_snaps(cur as i32))
    }

    #[test]
    fn fov_easing() {
        let mut f = 90.0;
        let mut n = 0;
        while f != 20.0 && n < 1000 {
            f = ease30(f, 20.0, 0.016);
            n += 1;
        }
        assert_eq!(f, 20.0);
        assert!(n > 3, "zoom is animated");
        // `UTIL_ApproachFOV`: half the gap (integer), times ft * 50, at least 1, truncated.
        // 0.016f is a hair above 0.016, so 90 - 28.000001 truncates to 61, as on the x87.
        assert_eq!(ease30(90.0, 20.0, 0.016), 61.0);
        assert_eq!(ease30(20.0, 90.0, 0.016), 48.0);
        assert_eq!(ease30(21.0, 20.0, 0.016), 20.0, "at least one degree");
        assert_eq!(ease30(22.0, 20.0, 0.0), 21.0);
        assert_eq!(ease30(0.0, 35.0, 0.016), 35.0, "unset snaps");
        assert_eq!(ease30(85.0, 20.0, 1.0), 20.0, "never passes the target");
        // A server FOV of any value (the `int` difference wraps as in C, no overflow).
        for (cur, target) in [(90.0, -3e9), (90.0, 3e9), (3e9, -3e9), (-3e9, 3e9), (90.0, f32::NAN), (f32::INFINITY, f32::NEG_INFINITY)] {
            for ft in [0.0, 0.016, f32::NAN, f32::INFINITY, -1.0] {
                ease30(cur, target, ft);
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
    fn stair_smoothing_on_ground() {
        let mut s = Stairs { smooth_z: 64.0, ..Stairs::default() };
        // Step up 18 units: the eye eases up at 15·ft per frame.
        let z = s.smooth(82.0, true, 0.01);
        assert!((z - (64.0 + 18.0 * 0.15)).abs() < 1e-4, "{z}");
        for _ in 0..200 {
            s.smooth(82.0, true, 0.01);
        }
        assert!((s.smooth(82.0, true, 0.01) - 82.0).abs() < 1e-3);
        // Step down and big jumps ease the same way: there is no snap.
        let z = s.smooth(-100.0, true, 0.01);
        assert!((z - (82.0 - 182.0 * 0.15)).abs() < 1e-3, "{z}");
        // Slow frames (15·ft ≥ 1) follow exactly.
        assert_eq!(s.smooth(10.0, true, 0.1), 10.0);
        assert_eq!(s.ofs, 0.0);
    }

    #[test]
    fn stair_smoothing_leaving_ground() {
        // Lagging 10 below the eye (walking up stairs), then leaving the ground: TS's offset is
        // `z − smoothed`, so the view jumps 10 *above* the eye, then decays at rate 15.
        let mut s = Stairs { smooth_z: 90.0, ..Stairs::default() };
        let z = s.smooth(100.0, false, 0.01);
        assert_eq!(z, 110.0);
        assert_eq!(s.smooth_z, 100.0, "the air tracks the eye");
        assert!((s.ofs - 8.5).abs() < 1e-5);
        let z = s.smooth(101.0, false, 0.01);
        assert!((z - 109.5).abs() < 1e-4, "only the first air frame sets the offset: {z}");
        // Lagging above (stepping down) dips the view below the eye instead.
        let mut s = Stairs { smooth_z: 110.0, ..Stairs::default() };
        assert_eq!(s.smooth(100.0, false, 0.01), 90.0);
        // Landing drops the offset and eases on from the last air z.
        let z = s.smooth(80.0, true, 0.01);
        assert!((z - (100.0 - 20.0 * 0.15)).abs() < 1e-4 && s.ofs == 0.0, "{z}");
        // Leaving flat ground with no lag adds nothing.
        let mut s = Stairs { smooth_z: 50.0, ..Stairs::default() };
        assert_eq!(s.smooth(50.0, false, 0.01), 50.0);
    }

    #[test]
    fn stair_smoothing_initial_state() {
        // TS's statics: smoothed z 0 and "was on the ground", so a first frame in the air carries
        // the whole eye z as the offset, and a first frame on the ground eases up from z 0.
        let mut s = Stairs::default();
        assert_eq!(s.smooth(40.0, false, 0.01), 80.0);
        let mut s = Stairs::default();
        assert!((s.smooth(40.0, true, 0.01) - 6.0).abs() < 1e-5);
    }

    #[test]
    fn view_height_initial_state() {
        // TS's view-height static starts at 0 and feeds the stair smoothing, which also starts at
        // z 0: on the first frame on the ground both ease, the stair state toward an eye that is
        // itself still low.
        let mut w = View::default();
        assert_eq!(w.vh, 0.0);
        w.vh = ease_view_height(w.vh, 28.0, 0.01);
        assert!((w.vh - 4.2).abs() < 1e-5, "{}", w.vh);
        let z = w.stairs.smooth(100.0 + w.vh, true, 0.01);
        assert!((z - 104.2 * 0.15).abs() < 1e-4, "{z}");
        // Both settle on simorg + view height.
        for _ in 0..200 {
            w.vh = ease_view_height(w.vh, 28.0, 0.01);
            w.stairs.smooth(100.0 + w.vh, true, 0.01);
        }
        assert!((w.vh - 28.0).abs() < 1e-3 && (w.stairs.smooth(128.0, true, 0.01) - 128.0).abs() < 1e-3);
        // A slow first frame (15·ft ≥ 1) takes the view height at once.
        assert_eq!(ease_view_height(0.0, 28.0, 0.1), 28.0);
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
    fn third_person_offsets() {
        assert_eq!(cam_base(0, 0.0, 0.0, 0.0), (-48.0, 16.0, 12.0));
        assert_eq!(cam_base(3, 500.0, -50.0, 50.0), (-128.0, -24.0, 24.0));
        assert_eq!(cam_base(3, -50.0, 0.0, 0.0).0, 24.0);
        let s = CamState { ducked: false, alive: true, stunt: false, ft: 0.01 };
        let (mut u, mut b) = (0.0, 0.0);
        assert_eq!(cam_smooth(&mut u, &mut b, -48.0, 12.0, s), (-48.0, 12.0));
        // Stunt pulls the camera back to -92, ducking raises it to 32; dead goes to -180.
        let stunt = CamState { ducked: true, stunt: true, ..s };
        for _ in 0..2000 {
            cam_smooth(&mut u, &mut b, -48.0, 12.0, stunt);
        }
        let (back, up) = cam_smooth(&mut u, &mut b, -48.0, 12.0, stunt);
        assert!((back + 92.0).abs() < 1e-3 && (up - 32.0).abs() < 1e-3, "{back} {up}");
        let dead = CamState { alive: false, ..s };
        for _ in 0..2000 {
            cam_smooth(&mut u, &mut b, -48.0, 12.0, dead);
        }
        let (back, up) = cam_smooth(&mut u, &mut b, -48.0, 12.0, dead);
        assert!((back + 180.0).abs() < 1e-3 && up == 12.0, "dead ignores the up offset: {back} {up}");
        // No stunt pull-back when the camera is already at the -128 limit.
        let (mut u, mut b) = (0.0, 0.0);
        assert_eq!(cam_smooth(&mut u, &mut b, -128.0, 12.0, CamState { stunt: true, ..s }).0, -128.0);
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
        let run = |frames: usize, ft: f32| {
            let mut b = Bob::default();
            let mut pitch = 0.0f32;
            for n in 1..=frames {
                b.calc(&BobInput { frametime: ft, ..bob_input(n as f32 * ft, 300.0) });
                pitch = add_bob_pitch(pitch, std::mem::take(&mut b.pitch));
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
