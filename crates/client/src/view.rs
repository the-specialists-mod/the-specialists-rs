//! View: V_CalcRefdef (bob, punch, recoil via cl_viewangles), third-person camera, slow-motion
//! tint and fades. Owner: rendering/prediction work package.
//! Reference: decomp docs/client/rendering-effects-prediction.md §6-7.
//!
//! `TSFade` is hooked here (the HUD package should not hook it, or should forward to
//! [`on_tsfade`]).

use crate::effects::psys::Camera;
use crate::{effects, state};
use goldsrc::Global;
use goldsrc::ext::render::{self as r, Cvar, Vec3, vec3 as v};
use goldsrc::sys::*;
use shared::ts::stunt;
use std::cell::RefCell;

struct Cvars {
    bob: Cvar,
    waterdist: Cvar,
    slowfade: Cvar,
    camtype: Cvar,
    cam_back: Cvar,
    cam_right: Cvar,
    cam_up: Cvar,
    thirdperson: Cvar,
    thirdpersonswitch: Cvar,
    default_fov: Cvar,
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
    // bob
    bob_phase: f32,
    bob_amp: f32,
    bob_z: f32,
    bob_pitch: f32,
    // smoothing
    vh: f32,
    stairs: Stairs,
    roll: f32,
    // kick (`0x10191d10` target, `0x101914f8` current) and the view-shake spring
    kick_target: [f32; 2],
    kick_cur: [f32; 2],
    last_shot: f64,
    shake: Shake,
    // `V_DriftPitch` state (SDK `pd`, `0x10191460..0x10191470`)
    drift: Drift,
    /// Local player `renderamt` being eased (`0x10097214`); 255 = opaque.
    local_amt: f32,
    // `cl_vsmoothing` origin ring (SDK `ViewInterp`, `0x10191508`) and the last recorded simorg
    org_ring: Ring,
    last_simorg: Vec3,
    // third person
    third: bool,
    /// Third-person state saved by a HUD "leave" request (observer mode), restored on "restore".
    saved_third: Option<bool>,
    forced_dead: bool,
    /// Eased additions to the camera's up (`0x10191da4`) and back (`0x10191da8`) offsets.
    cam_up_ofs: f32,
    cam_back_ofs: f32,
    /// Smoothed z of the local player model in third person (`0x10191d94`); `None` while in
    /// first person.
    model_z: Option<f32>,
    last_thirdperson_cvar: f32,
    last_weapon: i32,
    // TSFade
    fade: Option<(f32, state_fade::Fade)>,
    // view-model framerate latch (`g_vmFramerate` `0x10094cf0`): (sequence, animtime, rate)
    vm_latch: (i32, f32, f32),
    // outputs
    cam: Camera,
    eye: Vec3,
    hardware: bool,
    initialised: bool,
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
    /// `ViewShake::Update` (`0x10058000`): returns this frame's step (added to cl_viewangles).
    fn update(&mut self, dt: f32) -> Vec3 {
        let (target, rate) = if self.time_left <= 0.0 {
            (v::ZERO, 20.0)
        } else {
            self.time_left -= dt;
            self.flip -= dt;
            if self.flip < 0.0 {
                self.flip = 1.0 / self.freq;
                self.target = v::scale(self.target, -0.5);
            }
            (self.target, self.freq)
        };
        let k = (rate * dt).min(1.0);
        let step = v::scale(v::sub(target, self.cur), k);
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
            // `V_StartPitchDrift`, inlined: blocked while something stopped it this frame.
            if self.driftmove > i.centermove && i.now != self.laststop {
                self.pitchvel = i.centerspeed;
                self.nodrift = false;
                self.driftmove = 0.0;
            }
            return;
        }
        let delta = i.idealpitch - *pitch;
        if delta == 0.0 {
            self.pitchvel = 0.0;
            return;
        }
        let mut step = self.pitchvel * i.frametime;
        self.pitchvel += i.centerspeed * i.frametime;
        if delta > 0.0 {
            if delta < step {
                self.pitchvel = 0.0;
                step = delta;
            }
            *pitch += step;
        } else {
            if -delta < step {
                self.pitchvel = 0.0;
                step = -delta;
            }
            *pitch -= step;
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
        let mut idx = self.cur.wrapping_sub(2);
        for _ in 1..RING_MASK {
            let a = (idx & RING_MASK) as usize;
            if self.time[a] <= t {
                if self.time[a] == 0.0 {
                    return None;
                }
                let b = (idx.wrapping_add(1) & RING_MASK) as usize;
                let dt = self.time[b] - self.time[a];
                if dt <= 0.0 {
                    return None;
                }
                let frac = ((t - self.time[a]) / dt).min(1.0);
                let delta = v::sub(self.pos[b], self.pos[a]);
                return Some((v::ma(self.pos[a], frac, delta), v::len(delta)));
            }
            idx = idx.wrapping_sub(1);
        }
        None
    }
}

static VIEW: Global<RefCell<Option<View>>> = Global(RefCell::new(None));
static CVARS: Global<RefCell<Option<Cvars>>> = Global(RefCell::new(None));

fn with<R>(f: impl FnOnce(&mut View) -> R) -> R {
    let mut g = VIEW.0.borrow_mut();
    f(g.get_or_insert_with(|| View { local_amt: 255.0, ..View::default() }))
}

fn cv(f: impl FnOnce(&Cvars) -> Cvar) -> f32 {
    CVARS.0.borrow().as_ref().map_or(0.0, |c| f(c).value())
}

pub fn init() {
    let a = r::FCVAR_ARCHIVE;
    *CVARS.0.borrow_mut() = Some(Cvars {
        bob: Cvar::register(c"cl_bob", c"4.0", a),
        waterdist: Cvar::register(c"cl_waterdist", c"4", 0),
        slowfade: Cvar::register(c"cl_slowfade", c"1", 0),
        camtype: Cvar::register(c"cl_camtype", c"0", a),
        cam_back: Cvar::register(c"cam_back", c"90", a),
        cam_right: Cvar::register(c"cam_right", c"0", a),
        cam_up: Cvar::register(c"cam_up", c"12", a),
        thirdperson: Cvar::register(c"cl_thirdperson", c"0", a),
        thirdpersonswitch: Cvar::register(c"cl_thirdpersonswitch", c"0", a),
        default_fov: Cvar::register(c"default_fov", c"90", a),
        thirdtransparency: Cvar::register(c"cl_thirdtransparency", c"1", a),
        vsmoothing: Cvar::register(c"cl_vsmoothing", c"0.05", a),
        centermove: Cvar::register(c"v_centermove", c"0.15", 0),
        centerspeed: Cvar::register(c"v_centerspeed", c"500", 0),
    });
    Cvar::register(c"cl_bobcycle", c"0.8", 0);
    Cvar::register(c"cl_bobup", c"0.5", 0);
    let e = goldsrc::client::engine();
    e.add_command(c"thirdperson", cmd_thirdperson);
    e.add_command(c"firstperson", cmd_firstperson);
    e.add_command(c"ts_test_fade", cmd_test_fade);
    e.add_command(c"ts_test_kick", cmd_test_kick);
    e.add_command(c"ts_test_view", cmd_test_view);
    e.add_command(c"ts_test_angles", cmd_test_angles);
}

/// `ts_test_angles <pitch> <yaw>`: sets the view angles (developer aid for scripted screenshots).
extern "C" fn cmd_test_angles() {
    let e = goldsrc::client::engine();
    let n = |i: i32| e.cmd_argv(i).to_str().unwrap_or("").parse::<f32>().unwrap_or(0.0);
    let mut a = [n(1), n(2), 0.0];
    goldsrc::cl_eng!(SetViewAngles(a.as_mut_ptr()));
}

/// `ts_test_view`: prints the view state (developer aid).
extern "C" fn cmd_test_view() {
    let s = with(|w| {
        format!(
            "third {} dead {} cam_back {:.1} cam_up {:.1} model_z {:?} amt {:.0} origin {:?} eye {:?} angles {:?}\n",
            w.third, w.forced_dead, w.cam_back_ofs, w.cam_up_ofs, w.model_z, w.local_amt, w.cam.origin, w.eye, w.cam.angles
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
    kick(if n(1) == 0.0 { 5.5 } else { n(1) }, n(2) as u8, false, false);
}

extern "C" fn cmd_thirdperson() {
    with(|w| w.third = true);
}
extern "C" fn cmd_firstperson() {
    with(|w| w.third = false);
}

/// `TSFade` (`0x1004c1e0`): stored with start = now; applied every frame in `calc_refdef`.
pub fn on_tsfade(b: &[u8]) {
    if let Some(f) = shared::protocol::effects::TsFade::decode(b) {
        let now = r::client_time();
        with(|w| w.fade = Some((now, state_fade::Fade { duration: f.duration, hold: f.hold, rgba: f.rgba })));
    }
}

/// `InterpolateSlowMotion` (`0x10055ae0`): ease toward `target` at `min(7 ft, 1)`, snap within 0.05.
pub fn ease_time_scale(cur: f32, target: f32, ft: f32) -> f32 {
    let d = target - cur;
    if d.abs() > 0.05 { cur + d * (7.0 * ft).min(1.0) } else { target }
}

/// `CHud::Think` FOV easing: integer steps of `max(1, |d|/2 * ft * 50)`.
pub fn ease_fov(cur: f32, target: f32, ft: f32) -> f32 {
    let d = target - cur;
    if d.abs() < 0.5 {
        return target;
    }
    let step = (d.abs() / 2.0 * ft * 50.0).max(1.0).floor().min(d.abs());
    cur + step * d.signum()
}

/// Called from `HUD_Frame`. The engine passes the frame time (FWGS `host.frametime`, as the
/// SDK's `HUD_Frame(double time)` does), not an absolute time.
pub fn frame(frametime: f64) {
    let ft = (frametime as f32).clamp(0.0, 0.25);
    let time = r::client_time() as f64;
    with(|w| {
        w.last_frame = time;
        w.frametime = ft;
    });
    let default_fov = {
        let d = cv(|c| c.default_fov);
        if d == 0.0 { 80.0 } else { d.clamp(70.0, 110.0) }
    };
    state::with(|s| {
        s.time_scale = ease_time_scale(s.time_scale, s.time_scale_target.max(0.0), ft);
        let target = if s.fov_target > 0.0 { s.fov_target } else { default_fov };
        s.fov = if s.fov <= 0.0 { target } else { ease_fov(s.fov, target, ft) };
    });
    // Camera requests from the HUD (death cam on TSHealth 0, cl_thirdpersonswitch, observer).
    let (toggles, leave, restore) = state::with(|s| {
        (std::mem::take(&mut s.thirdperson_toggles), std::mem::take(&mut s.thirdperson_leave), std::mem::take(&mut s.thirdperson_restore))
    });
    with(|w| {
        if toggles % 2 == 1 {
            w.third = !w.third;
        }
        if leave {
            w.saved_third = Some(w.third);
            w.third = false;
        }
        if restore {
            if let Some(t) = w.saved_third.take() {
                w.third = t;
            }
        }
    });
    // Automatic camera switches: cl_thirdperson on change, cl_thirdpersonswitch on weapon change.
    let tp = cv(|c| c.thirdperson);
    let sw = cv(|c| c.thirdpersonswitch);
    let weapon = state::with(|s| s.weapon);
    with(|w| {
        if tp != w.last_thirdperson_cvar {
            w.last_thirdperson_cvar = tp;
            w.third = tp != 0.0;
        }
        if weapon != w.last_weapon {
            if sw != 0.0 {
                w.third = weapon == 0;
            }
            w.last_weapon = weapon;
        }
    });
    effects::frame(time, ft);
}

pub fn cam_think() {}

pub fn is_third_person() -> bool {
    with(|w| w.third || w.forced_dead)
}

/// View origin of the last refdef (`v_origin`).
pub fn origin() -> Vec3 {
    with(|w| w.cam.origin)
}
/// Eye position (`g_vecEyePos`): origin + view height, also in third person.
pub fn eye() -> Vec3 {
    with(|w| w.eye)
}
pub fn camera() -> Camera {
    let fov = state::with(|s| s.fov);
    with(|w| Camera { fov, ..w.cam })
}
pub fn hardware() -> bool {
    with(|w| w.hardware)
}

/// Writes the eased FOV into `clientdata` (TS `CHud::UpdateClientData`). Returns true (the
/// engine's "changed" flag), as `HUD_UpdateClientData` must.
pub fn update_client_data(cd: &mut r::ClientData) -> bool {
    cd.fov = state::with(|s| s.fov);
    true
}

/// `CTSGun::VKickback` client half (`0x10008230`) for a client weapon class (development
/// command `ts_test_kick`). Prediction calls [`kick_angles`] with the shared `VKickback` result.
pub fn kick(view_kick: f32, class: u8, silenced: bool, double_rate: bool) {
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
    kick_angles(pitch, yaw, silenced);
}

/// Queues one shot's view kick (`pitch` up, `yaw` right, degrees) and starts the shake spring.
/// Call once per shot, with `runfuncs` only.
pub fn kick_angles(pitch: f32, yaw: f32, silenced: bool) {
    let ts = state::with(|s| s.time_scale);
    let now = r::client_time() as f64;
    with(|w| {
        w.kick_target[0] -= pitch;
        w.kick_target[1] += yaw;
        let ts = if ts == 0.0 { 1.0 } else { ts };
        let amp = (0.15 - (now - w.last_shot) as f32 * ts) * 15.0;
        if amp > 0.001 {
            w.shake.start(amp.min(1.0) * if silenced { 0.775 } else { 1.0 }, 0.1, 35.0);
        }
        w.last_shot = now;
    });
}

/// `V_CalcBob` (`0x10066e70`, TS rewrite). Returns the gun offset (fwd, right, up components)
/// and stores the bob pitch.
fn calc_bob(w: &mut View, p: &ref_params_t, level: f32, ts: f32) -> Vec3 {
    let (lvl, pitch_lvl) = match level as i32 {
        i32::MIN..=0 => (0.1, 0.0),
        1 => (0.3, 0.0),
        2 => (0.5, 0.15),
        3 => (0.7, 0.5),
        _ => (1.0, 1.0),
    };
    let ft = p.frametime;
    if p.onground != 0 {
        let sp = (v::len2d(p.simvel) / 300.0).min(1.1);
        w.bob_phase = (w.bob_phase + sp * ft * 5.5 * ts * 100.0) % 720.0;
        w.bob_amp += (sp - w.bob_amp) * (ft * 10.0).min(1.0);
    } else {
        w.bob_amp *= 0.5;
    }
    let ph = w.bob_phase.to_radians();
    let amp = w.bob_amp;
    let up = (2.0 * ph).sin() * amp * 0.11 * lvl;
    w.bob_z += (up - w.bob_z) * (ft * 20.0).min(1.0);
    w.bob_pitch = (2.0 * ph).sin() * amp * pitch_lvl * 0.25;
    [(0.75 - amp / 2.0) * lvl, ph.sin() * amp * 0.3 * lvl, w.bob_z]
}

/// `V_CalcViewRoll` (`0x100675b0`): stunt roll from the `iuser4` bits.
fn calc_view_roll(w: &mut View, p: &mut ref_params_t, iuser4: i32) {
    if p.health <= 0 && p.viewheight[2] != 0.0 {
        p.viewangles[2] = 80.0;
        return;
    }
    if p.spectator != 0 {
        return;
    }
    let (target, rate) = if p.onground != 0 {
        (0.0, 15.0)
    } else if iuser4 & stunt::DIVING != 0 {
        (35.0, 7.5)
    } else if iuser4 & (stunt::FLIP_HIGH | stunt::FLIP_LOW) != 0 {
        (-10.0, 5.0)
    } else {
        (-5.5, 15.0)
    };
    w.roll += (target - w.roll) * (rate * p.frametime).min(1.0);
    // SDK V_CalcRoll with TS's side*0.01, rollspeed 10.
    let (_, right, _) = v::angle_vectors(p.viewangles);
    let side = v::dot(p.simvel, right) * 0.01;
    let strafe = side.signum() * if side.abs() < 10.0 { side.abs() * 2.0 / 10.0 } else { 2.0 };
    p.viewangles[2] += w.roll * if p.onground != 0 { 1.0 } else { 0.25 } + strafe;
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
    let spectating = p.spectator != 0 || state::with(|s| s.spec_mode) != 0;
    match refdef_path(p.intermission != 0, spectating, p.paused != 0) {
        RefdefPath::Intermission => calc_intermission(p),
        RefdefPath::Spectator => calc_spectator(p),
        RefdefPath::Normal => calc_normal(p),
        RefdefPath::None => {}
    }
    // The screen fade and fog are written every frame, paused or not.
    write_screen_fade(p.time);
    effects::fog_begin();
    p.nextView = 0;
}

fn finish(w: &mut View, p: &mut ref_params_t) {
    let (f, r_, u) = v::angle_vectors(p.viewangles);
    (p.forward, p.right, p.up) = (f, r_, u);
    w.cam = Camera { origin: p.vieworg, angles: p.viewangles, forward: f, right: r_, up: u, fov: 90.0 };
    w.hardware = p.hardware != 0;
}

/// `V_StopPitchDrift` (`0x100672f0`): TS input calls it while the mouse or the look keys move
/// the view (`CL_AdjustAngles` 0x10026330, `IN_MouseMove` 0x100274d0).
pub fn stop_pitch_drift() {
    let now = r::client_time();
    with(|w| w.drift.stop(now));
}

/// `V_AddIdle` (`0x10067550`): idle sway with TS's fixed (unregistered) `v_i*` values: roll
/// cycle 0.5 level 0.1, pitch cycle 1 level 0.3, yaw cycle 2 level 0.3.
fn add_idle(angles: &mut Vec3, time: f32, scale: f32) {
    angles[2] += scale * (time * 0.5).sin() * 0.1;
    angles[0] += scale * time.sin() * 0.3;
    angles[1] += scale * (time * 2.0).sin() * 0.3;
}

/// `V_CalcIntermissionRefdef` (`0x100676e0`): the view sways with idle scale 1.
fn calc_intermission(p: &mut ref_params_t) {
    p.vieworg = p.simorg;
    p.viewangles = p.cl_viewangles;
    if let Some(vm) = r::view_model() {
        vm.model = std::ptr::null_mut();
    }
    add_idle(&mut p.viewangles, p.time, 1.0);
    with(|w| finish(w, p));
}

/// `V_CalcSpectatorRefdef` (`0x10068f70`): see `crate::spectator::calc_refdef`.
fn calc_spectator(p: &mut ref_params_t) {
    crate::spectator::calc_refdef(p);
    with(|w| {
        w.eye = p.vieworg;
        finish(w, p)
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

fn calc_normal(p: &mut ref_params_t) {
    let ts = state::with(|s| s.time_scale).clamp(0.01, 1.0);
    let dead = p.health <= 0;
    let local = r::local_player();
    let iuser4 = local.as_ref().map_or(0, |e| e.curstate.iuser4);
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
        now: r::client_time(),
    };
    with(|w| w.drift.update(&drift, &mut p.cl_viewangles[0]));

    // Local player translucency (step 2): `TS_IsLocalDead` (0x1004c250) is false while alive or
    // in a demo. The local model is only drawn in third person, so that's where it shows.
    let e = goldsrc::client::engine();
    let translucent = !state::with(|s| s.dead) || e.demo_playing() || e.demo_recording();
    let on_player = crate::hud::with(|h| h.crosshair_id.is_some());
    let transparency = cv(|c| c.thirdtransparency) as i32 != 0;
    if let Some(ent) = r::local_player() {
        if translucent {
            let amt = with(|w| {
                w.local_amt = local_amt_step(w.local_amt, on_player, transparency, ft != 0.0);
                w.local_amt
            });
            ent.curstate.rendermode = kRenderTransColor as i32;
            ent.curstate.renderamt = amt as i32;
        } else {
            ent.curstate.rendermode = kRenderNormal as i32;
            ent.curstate.renderamt = 255;
        }
    }

    let third = with(|w| {
        if !w.initialised {
            w.initialised = true;
            w.vh = p.viewheight[2];
        }
        // Death forces third person (TS: `TSHealth` 0); leaving death returns.
        w.forced_dead = dead;
        state::with(|s| s.third_person = w.third || dead);
        w.third || dead
    });

    let (gun_ofs, bob_pitch_step, kick_step, shake_step) = with(|w| {
        let old_pitch = w.bob_pitch;
        let ofs = if third { [0.0; 3] } else { calc_bob(w, p, bob_level, ts) };
        let bob_step = (w.bob_pitch - old_pitch) * 0.1;
        // Weapon kick (`V_ApplyKick` `0x10057ed0`): ease current toward target, step goes into
        // cl_viewangles.
        let mut step = [0.0f32; 2];
        let d = [w.kick_target[0] - w.kick_cur[0], w.kick_target[1] - w.kick_cur[1]];
        if d[0] * d[0] + d[1] * d[1] >= 0.01 {
            let k = (ft * 15.0).min(1.0);
            step = [d[0] * k, d[1] * k];
            w.kick_cur[0] += step[0];
            w.kick_cur[1] += step[1];
        } else {
            w.kick_target = [0.0; 2];
            w.kick_cur = [0.0; 2];
        }
        let sh = w.shake.update(ft * ts);
        (ofs, bob_step, step, sh)
    });

    // vieworg = simorg with the smoothed view height on z (only z), then stair smoothing of that
    // eye z (`0x100679ee..0x10067a46`). During a stunt the server sends `view_ofs` 0 (`NO_VIEW`), so the eye
    // eases down to the hull centre like TS's.
    let on_ground = p.onground != 0;
    with(|w| {
        w.vh += (p.viewheight[2] - w.vh) * (15.0 * ft).min(1.0);
        let z = w.stairs.smooth(p.simorg[2] + w.vh, on_ground, ft);
        p.vieworg = [p.simorg[0], p.simorg[1], z];
    });

    // Recoil and shake written back into the engine's view angles (FWGS copies cl_viewangles
    // back after V_CalcRefdef; rendering doc §6).
    p.cl_viewangles[0] += kick_step[0] + shake_step[0] + bob_pitch_step;
    p.cl_viewangles[1] += kick_step[1] + shake_step[1];
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

    with(|w| calc_view_roll(w, p, iuser4));
    // `V_AddIdle` would follow, scaled by the HUD's concussion level. Only the TFC `Concuss`
    // message sets that and TS never sends it, so the scale is always 0 here.
    let mut eye = p.vieworg;
    let (fwd, right, up) = v::angle_vectors(p.viewangles);
    (p.forward, p.right, p.up) = (fwd, right, up);

    // Third-person camera (§7.4).
    if third {
        let ent = r::local_player();
        let ent_org = ent.as_ref().map_or(p.simorg, |e| e.origin);
        let vh = p.viewheight[2];
        let state = CamState { ducked: vh < 28.0 && vh != 0.0, alive: !dead, stunt: iuser4 & 0x130c30 != 0, ft };
        let (back, right_ofs, up_ofs) = cam_base(camtype, cb, cr, cu);
        let (back, up_ofs) = with(|w| cam_smooth(&mut w.cam_up_ofs, &mut w.cam_back_ofs, back, up_ofs, state));
        let cam_angles = [p.cl_viewangles[0], p.cl_viewangles[1], 0.0];
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
            let tr = r::player_trace(start, want, 2, (PM_STUDIO_IGNORE | PM_WORLD_ONLY) as i32, -1);
            p.vieworg = if tr.fraction != 1.0 { v::add(v::ma(tr.endpos, 4.0, tr.plane.normal), [0.0, 0.0, 1.0]) } else { want };
        }
        p.viewangles = cam_angles;
        if let Some(e) = ent {
            // The model's z eases at rate 16 (TS `0x10191d94`) so stair steps and landings don't
            // jerk the body in front of the camera.
            let z = with(|w| {
                let z = smooth_model_z(w.model_z.unwrap_or(e.origin[2]), e.origin[2], ft);
                w.model_z = Some(z);
                z
            });
            e.origin[2] = z;
            eye = v::add(e.origin, p.viewheight);
            let pitch = -v::norm_angle(cam_angles[0]) / 3.0;
            e.angles[0] = pitch;
            e.curstate.angles[0] = pitch;
            e.prevstate.angles[0] = pitch;
            e.latched.prevangles[0] = pitch;
        }
    } else {
        with(|w| w.model_z = None);
    }

    // View model (`V_CalcGunAngle` + TS bob offset and fov push).
    let mut vm_ent = r::view_model();
    if let Some(vm) = vm_ent.as_deref_mut() {
        let base_angles = p.cl_viewangles;
        vm.angles = [-base_angles[0] + p.crosshairangle[0] * 0.25, base_angles[1] + p.crosshairangle[1], base_angles[2]];
        let mut org = v::add(v::add(p.simorg, p.viewheight), [0.0, 0.0, water_ofs]);
        let mut ang = vm.angles;
        goldsrc::cl_eng!(V_ApplyShake(org.as_mut_ptr(), ang.as_mut_ptr(), 0.9));
        vm.angles = ang;
        org = v::ma(org, gun_ofs[0], fwd);
        org = v::ma(org, gun_ofs[1], right);
        org = v::ma(org, gun_ofs[2], up);
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
        vm.curstate.framerate = with(|w| view_model_framerate(w, vm.curstate.sequence, vm.curstate.animtime, ts));
        vm.curstate.angles = vm.angles;
        vm.latched.prevangles = vm.angles;
    }

    // Punch (the server never sets it in TS, but keep the SDK add).
    if !third {
        p.viewangles = v::add(p.viewangles, p.punchangle);
    }

    // `cl_vsmoothing` (step 14, SDK `ViewInterp`): in multiplayer with the server's `pushmsec`
    // smoothing request, the view lags `cl_vsmoothing` seconds behind along the recorded simorg
    // path, so lifts and trains don't jitter. Jumps of 64 units or more are not smoothed.
    let vsmoothing = cv(|c| c.vsmoothing);
    let smooth = vsmoothing != 0.0 && p.smoothing != 0 && p.maxclients > 1;
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
    }

    with(|w| {
        w.eye = eye;
        finish(w, p);
    });
}

/// TS `StudioDrawModel` change (§2.3): the view model animates at the time scale, faster with the
/// local power-ups (x3.5556 slow-pause, x1.71429 slow-motion, x1.5 double fire rate), latched once
/// per animation restart.
fn view_model_framerate(w: &mut View, sequence: i32, animtime: f32, ts: f32) -> f32 {
    if w.vm_latch.0 != sequence || w.vm_latch.1 != animtime {
        let pw = state::with(|s| s.powerups);
        let rate = if pw & 0x10 != 0 || !(0.999..=1.001).contains(&ts) {
            let mut r = ts;
            if pw & 8 != 0 {
                r *= 3.5556;
            }
            if pw & 1 != 0 {
                r *= 1.71429;
            }
            if pw & 0x10 != 0 {
                r *= 1.5;
            }
            r
        } else {
            1.0
        };
        w.vm_latch = (sequence, animtime, rate);
    }
    w.vm_latch.2
}

fn cl_calc_shake(p: &mut ref_params_t) {
    goldsrc::cl_eng!(V_CalcShake());
    goldsrc::cl_eng!(V_ApplyShake(p.vieworg.as_mut_ptr(), p.viewangles.as_mut_ptr(), 1.0));
}

/// Slow-motion tint + `TSFade` blend (end of `V_CalcRefdef`).
pub fn fade_color(ts: f32, slowfade: bool, fade: Option<(f32, state_fade::Fade)>, t: f32) -> [u8; 4] {
    let tint = if slowfade && ts < 1.0 { [0.0, 100.0, 125.0, (1.0 - ts) * 50.0] } else { [0.0; 4] };
    if let Some((start, f)) = fade {
        if start < t && t < start + f.duration {
            let half = ((f.duration - f.hold) / 2.0).max(1e-3);
            let e = t - start;
            let k = if e < half {
                e / half
            } else if e < half + f.hold {
                1.0
            } else {
                ((f.duration - e) / half).max(0.0)
            }
            .clamp(0.0, 1.0);
            let c = f.rgba.map(|x| x as f32);
            return [
                (c[0] * k) as u8, // red has no tint
                (c[1] * k + tint[1] * (1.0 - k)) as u8,
                (c[2] * k + tint[2] * (1.0 - k)) as u8,
                (c[3] * k + tint[3] * (1.0 - k)) as u8,
            ];
        }
    }
    tint.map(|x| x as u8)
}

/// Rewritten every frame, as TS does (so it overrides engine `ScreenFade` messages).
fn write_screen_fade(_time: f32) {
    let ts = state::with(|s| s.time_scale);
    let slowfade = cv(|c| c.slowfade) != 0.0;
    let t = r::client_time();
    let fade = with(|w| w.fade);
    let c = fade_color(ts, slowfade, fade, t);
    let mut sf = screenfade_s {
        fadeSpeed: 0.0,
        fadeEnd: t,
        fadeTotalEnd: 0.0,
        fadeReset: t,
        fader: c[0],
        fadeg: c[1],
        fadeb: c[2],
        fadealpha: c[3],
        fadeFlags: (FFADE_OUT | FFADE_STAYOUT) as i32,
    };
    r::set_screen_fade(&mut sf);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_scale_easing() {
        let mut ts = 1.0;
        for _ in 0..200 {
            ts = ease_time_scale(ts, 0.35, 0.01);
        }
        assert_eq!(ts, 0.35);
        assert!((ease_time_scale(1.0, 0.35, 0.01) - (1.0 - 0.65 * 0.07)).abs() < 1e-6);
        assert_eq!(ease_time_scale(0.97, 1.0, 0.01), 1.0, "snaps within 0.05");
    }

    #[test]
    fn fov_easing() {
        let mut f = 90.0;
        let mut n = 0;
        while f != 20.0 && n < 1000 {
            f = ease_fov(f, 20.0, 0.016);
            n += 1;
        }
        assert_eq!(f, 20.0);
        assert!(n > 3, "zoom is animated");
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
        let mut s = Shake::default();
        s.target = [1.0, -1.0, 0.0];
        s.time_left = 0.1;
        s.freq = 35.0;
        s.flip = 1.0 / 35.0;
        let mut total = v::ZERO;
        for _ in 0..400 {
            total = v::add(total, s.update(0.01));
        }
        assert!(v::len(total) < 1e-3, "net view change returns to zero: {total:?}");
    }
}
