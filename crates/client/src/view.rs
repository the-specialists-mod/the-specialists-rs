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
use std::ffi::c_void;

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
    smooth_z: f32,
    step_ofs: f32,
    on_ground: bool,
    roll: f32,
    // kick (`0x10191d10` target, `0x101914f8` current) and the view-shake spring
    kick_target: [f32; 2],
    kick_cur: [f32; 2],
    last_shot: f64,
    shake: Shake,
    // third person
    third: bool,
    /// Third-person state saved by a HUD "leave" request (observer mode), restored on "restore".
    saved_third: Option<bool>,
    forced_dead: bool,
    cam_back: f32,
    cam_up: f32,
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

static VIEW: Global<RefCell<Option<View>>> = Global(RefCell::new(None));
static CVARS: Global<RefCell<Option<Cvars>>> = Global(RefCell::new(None));

fn with<R>(f: impl FnOnce(&mut View) -> R) -> R {
    let mut g = VIEW.0.borrow_mut();
    f(g.get_or_insert_with(View::default))
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
    });
    Cvar::register(c"cl_bobcycle", c"0.8", 0);
    Cvar::register(c"cl_bobup", c"0.5", 0);
    Cvar::register(c"cl_thirdtransparency", c"1", a);
    Cvar::register(c"cl_vsmoothing", c"0.05", a);
    let e = goldsrc::client::engine();
    e.add_command(c"thirdperson", cmd_thirdperson);
    e.add_command(c"firstperson", cmd_firstperson);
    e.add_command(c"ts_test_fade", cmd_test_fade);
    e.add_command(c"ts_test_kick", cmd_test_kick);
    e.add_command(c"ts_test_view", cmd_test_view);
    e.add_command(c"ts_test_angles", cmd_test_angles);
}

/// `ts_test_angles <pitch> <yaw>`: sets the view angles (developer aid for scripted screenshots).
unsafe extern "C" fn cmd_test_angles() {
    let e = goldsrc::client::engine();
    let n = |i: i32| e.cmd_argv(i).to_str().unwrap_or("").parse::<f32>().unwrap_or(0.0);
    let mut a = [n(1), n(2), 0.0];
    goldsrc::cl_eng!(SetViewAngles(a.as_mut_ptr()));
}

/// `ts_test_view`: prints the view state (developer aid).
unsafe extern "C" fn cmd_test_view() {
    let s = with(|w| format!("third {} dead {} cam_back {:.1} cam_up {:.1} origin {:?} eye {:?} angles {:?}\n", w.third, w.forced_dead, w.cam_back, w.cam_up, w.cam.origin, w.eye, w.cam.angles));
    let ts = state::with(|st| format!("ts {:.3}/{:.3} fov {:.1}/{:.1}\n", st.time_scale, st.time_scale_target, st.fov, st.fov_target));
    goldsrc::client::engine().print(&s);
    goldsrc::client::engine().print(&ts);
}

/// `ts_test_fade <r> <g> <b> <a> <duration> <hold>` (developer test of the TSFade path).
unsafe extern "C" fn cmd_test_fade() {
    let e = goldsrc::client::engine();
    let n = |i: i32| e.cmd_argv(i).to_str().unwrap_or("").parse::<f32>().unwrap_or(0.0);
    let f = shared::protocol::effects::TsFade { duration: n(5).max(0.1), hold: n(6), flags: 0, rgba: [n(1) as u8, n(2) as u8, n(3) as u8, n(4) as u8] };
    let mut b = Vec::new();
    f.encode(&mut b);
    on_tsfade(&b);
}

/// `ts_test_kick <view kick> <class>`: one shot's client-side recoil.
unsafe extern "C" fn cmd_test_kick() {
    let e = goldsrc::client::engine();
    let n = |i: i32| e.cmd_argv(i).to_str().unwrap_or("").parse::<f32>().unwrap_or(0.0);
    kick(if n(1) == 0.0 { 5.5 } else { n(1) }, n(2) as u8, false, false);
}

unsafe extern "C" fn cmd_thirdperson() {
    with(|w| w.third = true);
}
unsafe extern "C" fn cmd_firstperson() {
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

/// Writes the eased FOV into `clientdata` (TS `CHud::UpdateClientData`). The `HUD_UpdateClientData`
/// export in lib.rs should call this and return 1 (integration note in the implementation doc).
pub fn update_client_data(cdata: *mut c_void) -> bool {
    let Some(cd) = (unsafe { cdata.cast::<r::ClientData>().as_mut() }) else { return false };
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

/// `V_CalcRefdef` (`0x10069b90`).
pub fn calc_refdef(p: &mut ref_params_t) {
    if p.intermission != 0 {
        calc_intermission(p);
    } else if p.spectator != 0 || state::with(|s| s.spec_mode) != 0 {
        calc_spectator(p);
    } else {
        calc_normal(p);
    }
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

fn calc_intermission(p: &mut ref_params_t) {
    p.vieworg = p.simorg;
    p.viewangles = p.cl_viewangles;
    if let Some(vm) = r::view_model() {
        vm.model = std::ptr::null_mut();
    }
    with(|w| finish(w, p));
}

/// Spectator modes (`V_CalcSpectatorRefdef` `0x10068f70`, simplified): in-eye copies the
/// target's view; chase modes put the camera behind it; free look uses the engine's origin.
fn calc_spectator(p: &mut ref_params_t) {
    let (mode, target) = state::with(|s| (s.spec_mode, s.spec_target));
    p.vieworg = v::add(p.simorg, p.viewheight);
    p.viewangles = p.cl_viewangles;
    if let Some(t) = (target > 0).then(|| r::entity(target)).flatten() {
        match mode {
            4 | 5 => {
                p.vieworg = v::add(t.origin, shared::VEC_VIEW);
                p.viewangles = [t.angles[0] * -3.0, t.angles[1], 0.0];
            }
            1..=3 => {
                let focus = v::add(t.origin, [0.0, 0.0, 28.0]);
                let (f, _, _) = v::angle_vectors(p.cl_viewangles);
                let want = v::ma(focus, -128.0, f);
                let tr = r::player_trace(focus, want, 2, (PM_STUDIO_IGNORE | PM_WORLD_ONLY) as i32, -1);
                p.vieworg = v::ma(tr.endpos, 4.0, tr.plane.normal);
            }
            _ => {}
        }
    }
    if let Some(vm) = r::view_model() {
        vm.model = std::ptr::null_mut();
    }
    with(|w| {
        w.eye = p.vieworg;
        finish(w, p)
    });
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

    let third = with(|w| {
        if !w.initialised {
            w.initialised = true;
            w.vh = p.viewheight[2];
            w.smooth_z = p.simorg[2];
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

    // vieworg = simorg + smoothed view height, plus stair smoothing.
    with(|w| {
        w.vh += (p.viewheight[2] - w.vh) * (15.0 * ft).min(1.0);
        let on = p.onground != 0;
        if on {
            w.smooth_z += (p.simorg[2] - w.smooth_z) * (15.0 * ft).min(1.0);
            if (p.simorg[2] - w.smooth_z).abs() > 32.0 {
                w.smooth_z = p.simorg[2];
            }
        } else {
            if w.on_ground {
                w.step_ofs = w.smooth_z - p.simorg[2];
            }
            w.smooth_z = p.simorg[2];
        }
        w.on_ground = on;
        let z = if on { w.smooth_z } else { p.simorg[2] + w.step_ofs };
        w.step_ofs -= w.step_ofs * (15.0 * ft).min(1.0);
        p.vieworg = [p.simorg[0] + p.viewheight[0], p.simorg[1] + p.viewheight[1], z + w.vh];
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
    let eye = p.vieworg;
    let (fwd, right, up) = v::angle_vectors(p.viewangles);
    (p.forward, p.right, p.up) = (fwd, right, up);

    // Third-person camera (§7.4).
    if third {
        let (mut back, mut rt, mut upo) = match camtype {
            1 => (-48.0, -16.0, 12.0),
            2 => (-90.0, 0.0, 12.0),
            3 => (-cb, cr, cu),
            _ => (-48.0, 16.0, 12.0),
        };
        back = back.clamp(-128.0, 24.0);
        rt = rt.clamp(-24.0, 24.0);
        upo = upo.clamp(-24.0, 24.0);
        let back_target = if dead {
            -180.0
        } else if iuser4 & 0x130c30 != 0 {
            -92.0
        } else {
            back
        };
        let up_target = if p.viewheight[2] > 0.0 && p.viewheight[2] < 28.0 { 32.0 } else { upo };
        let (b, u) = with(|w| {
            if w.cam_back == 0.0 {
                w.cam_back = back_target;
                w.cam_up = up_target;
            }
            w.cam_back += (back_target - w.cam_back) * (ft * 5.0).min(1.0);
            w.cam_up += (up_target - w.cam_up) * (ft * 5.0).min(1.0);
            (w.cam_back, w.cam_up)
        });
        let cam_angles = [p.cl_viewangles[0], p.cl_viewangles[1], 0.0];
        let (cf, cr2, _) = v::angle_vectors(cam_angles);
        let pivot = v::add(p.simorg, [0.0, 0.0, u + 28.0 - 28.0 + p.viewheight[2].min(28.0)]);
        let want = v::ma(v::ma(pivot, b, cf), rt, cr2);
        let tr = r::player_trace(pivot, want, 2, (PM_STUDIO_IGNORE | PM_WORLD_ONLY) as i32, -1);
        p.vieworg = if tr.fraction < 1.0 { v::add(v::ma(tr.endpos, 4.0, tr.plane.normal), [0.0, 0.0, 1.0]) } else { want };
        p.viewangles = cam_angles;
        if let Some(e) = r::local_player() {
            let pitch = -v::norm_angle(cam_angles[0]) / 3.0;
            e.angles[0] = pitch;
            e.curstate.angles[0] = pitch;
            e.prevstate.angles[0] = pitch;
            e.latched.prevangles[0] = pitch;
        }
    }

    // View model (`V_CalcGunAngle` + TS bob offset and fov push).
    if let Some(vm) = r::view_model() {
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
