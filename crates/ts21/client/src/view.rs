//! TS 2.1's view bob (`V_CalcBob` 0x100b5c80, issue #164), eye height (issue #167), third-person
//! camera offsets and view roll (issue #240). f64 stands in for the x87 stack, as in the port's
//! 3.0 bob.

use goldsrc::ext::render::{Vec3, vec3 as v};
use goldsrc::sys::ref_params_t;
use shared::ts::stunt;
use ts_client::hud::draw::ftol;
use ts_client::view::{Bob, BobInput, CamInput, CamOffsets, EyeZ, RollEntity};

/// Degrees to radians, the double at 0x10270240.
const DEG: f64 = 0.017453292519943295;

/// `V_CalcBob` (0x100b5c80). The skip test is 3.0's. Unlike 3.0:
/// * on the ground the phase advances by `ft × 3.5 × 100 × s`, `s` the raw `TSSlowMo` factor (1 at
///   normal speed): no speed factor;
/// * the amplitude is `|v_xy| × 0.01` (which also sets the peak) unless below the peak, when it is
///   `(|v_xy| × 0.01 + amp) × 0.5`;
/// * no `cl_bob` levels and no bob pitch: the gun offset is
///   `up·s2·0.11 + right·s1·0.3 + fwd·(0.75 − amp/2)`, z += `0.5 − 0.65·amp`;
/// * the camera bob (0x1032073c) is `(up·s2 + right·s1) × −0.9`, z × 1.6, which
///   `V_CalcNormalRefdef` adds to `vieworg` every frame (0x100b6dac..0x100b6df1). Only the ground
///   branch writes it, so it keeps its last value in the air, in third person and when standing.
pub fn calc(b: &mut Bob, i: &BobInput) -> f32 {
    // The static starts at zero and is added whether or not this frame bobs.
    let eye = b.eye.get_or_insert([0.0; 3]);
    b.pitch = 0.0;
    if i.skip {
        return 0.0;
    }
    let k = |amp: f32| 0.75 - amp as f64 * 0.5;
    let z_part = |amp: f32| 0.5 - amp as f64 * 0.65;
    if i.on_ground && i.time != b.last_time && i.viewheight != 0.0 {
        let factor = i.slowmo_factor100 / 100.0;
        let s = if factor == 0.0 { 1.0 } else { factor };
        b.last_time = i.time;
        b.phase = (i.frametime as f64 * 3.5 * 100.0 * s as f64 + b.phase as f64) as f32;
        if b.phase > 360.0 {
            b.phase = 0.0;
        }
        let speed = (i.simvel[0] as f64 * i.simvel[0] as f64 + i.simvel[1] as f64 * i.simvel[1] as f64).sqrt();
        // `fld peak; fcomp amp; and 0x4100; jne`: at most the amplitude, or unordered.
        if b.peak > b.amp {
            b.amp = ((speed * 0.01 + b.amp as f64) * 0.5) as f32;
        } else {
            b.amp = (speed * 0.01) as f32;
            b.peak = b.amp;
        }
        let phase = b.phase as f64 * DEG;
        let s2 = ((phase + phase).sin() * b.amp as f64) as f32;
        let s1 = ((phase * 1.0).sin() * b.amp as f64) as f32;
        for a in 0..3 {
            let (up, right) = (s2 as f64 * i.up[a] as f64, s1 as f64 * i.right[a] as f64);
            eye[a] = (up * -0.9 + right * -0.9) as f32;
            if a == 2 {
                eye[2] = (eye[2] as f64 * 1.6) as f32;
            }
            b.ofs[a] = (up * 0.11 + right * 0.3 + i.forward[a] as f64 * k(b.amp)) as f32;
        }
        b.ofs[2] = (z_part(b.amp) + b.ofs[2] as f64) as f32;
    } else {
        if b.amp as f64 > 0.01 {
            b.amp = (b.amp as f64 / 2.0) as f32;
        }
        b.ofs = i.forward.map(|f| (f as f64 * k(b.amp)) as f32);
        b.ofs[2] = z_part(b.amp) as f32;
    }
    smooth_z(b, i.frametime);
    b.amp
}

/// The gun offset's z smoothing (0x100b5b70): 3.0's `V_SmoothBobZ`, with the step worked out as
/// `(z_smooth − z) × 0.2 × ft × 100` on the x87 and stored to a float.
fn smooth_z(b: &mut Bob, ft: f32) {
    let d = b.smooth_z - b.ofs[2];
    let mut step = ((b.smooth_z as f64 - b.ofs[2] as f64) * 0.2 * ft as f64 * 100.0) as f32;
    if d.abs() < step.abs() {
        step = d;
    }
    b.ofs[2] = b.smooth_z - step;
    b.smooth_z = b.ofs[2];
}

/// `V_CalcNormalRefdef`'s eye (0x100b6d49..0x100b6da9): `simorg` plus the raw view height on all
/// three axes (no easing of its own), then [`follow_z`] on z. No stair smoothing.
pub fn eye_origin(eye: &mut EyeZ, simorg: [f32; 3], viewheight: [f32; 3], ft: f32) -> [f32; 3] {
    let mut org = [simorg[0] + viewheight[0], simorg[1] + viewheight[1], simorg[2] + viewheight[2]];
    org[2] = follow_z(eye, org[2], ft);
    org
}

/// The eye z smoother (0x100b6b80, thunk 0x10001cb7), called on the eye (0x100b6da4) and again
/// on the third-person body cam (0x100b7c85), the same static (0x103206fc): `d = last − z`, `step = (last − z) × 0.1 × ft × 100`
/// on the x87, both stored to floats; `step = d` when `|d| < |step|`; `z = last − step`, which
/// becomes `last`. A `10·ft` follow, on the ground or not.
pub fn follow_z(eye: &mut EyeZ, z: f32, ft: f32) -> f32 {
    let last = eye.follow_z;
    let d = last - z;
    let mut step = ((last as f64 - z as f64) * 0.1 * ft as f64 * 100.0) as f32;
    // `fcomp 0.0; and 0x4100; jne`: the negation branch takes `<= 0` and unordered.
    let abs = |x: f32| if x > 0.0 { x } else { 0.0 - x };
    if abs(d) < abs(step) {
        step = d;
    }
    eye.follow_z = last - step;
    eye.follow_z
}

/// `V_CalcNormalRefdef`'s third-person camera offsets (0x100b7700..0x100b7ab9). 2.1 has no
/// `cl_camtype`: always the SDK cvars, `back = −clamp(cam_back, −24, 128)`, `right =
/// clamp(cam_right, −24, 24)`, `up = clamp(cam_up, −24, 24)` (each `v <= −24 ? −24 : v`, then
/// `hi <= v ? hi : v`).
///
/// The extra offsets are `int`s (0x10320fc8 up, 0x10320f7c back), kept in [`CamOffsets`] as whole
/// numbers, stepped by `__ftol(ofs ± ft × 100)` toward their targets and set to the target
/// (`__ftol`'d) once at or past it:
/// * up, while ducked (and `up < 32`): toward `32 − up`; otherwise down to 0 (0 at once from 0 or
///   below);
/// * back, dead: down toward `−180 − back`; during a stunt with `back > −128`: down toward
///   `−92 − back` (a target above the offset is taken at once); otherwise up to 0 (0 at once from
///   0 or above).
///
/// The back offset is then added, and the up offset only while alive.
pub fn camera_offsets(ofs: &mut CamOffsets, c: &CamInput) -> (f32, f32, f32) {
    let clamp = |x: f32, hi: f32| {
        let x = if x <= -24.0 { -24.0 } else { x };
        if hi <= x { hi } else { x }
    };
    let (back, right, up) = (-clamp(c.cam_back, 128.0), clamp(c.cam_right, 24.0), clamp(c.cam_up, 24.0));
    let step = c.ft as f64 * 100.0;
    let (mut ou, mut ob) = (ofs.up as i32, ofs.back as i32);
    if c.ducked && up < 32.0 {
        let target = 32.0 - up as f64;
        // `fcomp; and 0x4100; jne`: at or past the target, or unordered.
        ou = if target > ou as f32 as f64 { ftol(ou as f64 + step) } else { ftol(target) };
    } else if ou <= 0 {
        ou = 0;
    } else {
        ou = ftol(ou as f64 - step);
    }
    // Down toward a target, or to it at once when it isn't below (`test ah, 5; jp`).
    let down_to = |o: i32, target: f64| if target < o as f32 as f64 { ftol(o as f64 - step) } else { ftol(target) };
    if !c.alive {
        ob = down_to(ob, -180.0 - back as f64);
    } else if c.stunt && back > -128.0 {
        ob = down_to(ob, -92.0 - back as f64);
    } else if ob < 0 {
        ob = ftol(ob as f64 + step);
    } else {
        ob = 0;
    }
    (ofs.up, ofs.back) = (ou as f32, ob as f32);
    (ob as f32 + back, right, if c.alive { ou as f32 + up } else { up })
}

/// 2.1's `V_CalcRoll` (0x100b6170): `side = right·vel` (z, y, x order) stored as a float, sign
/// ±1 (+1 at 0), `q = |side| / 100.0`; `rollspeed <= q ? rollangle : (float)(q × rollangle /
/// rollspeed)`, times the sign.
fn calc_roll(angles: Vec3, vel: Vec3, rollangle: f32, rollspeed: f32) -> f32 {
    let (_, right, _) = v::angle_vectors(angles);
    let side = (right[2] as f64 * vel[2] as f64 + right[1] as f64 * vel[1] as f64 + right[0] as f64 * vel[0] as f64) as f32;
    let sign = if 0.0 <= side { 1.0 } else { -1.0 };
    let abs = if side <= 0.0 { 0.0 - side } else { side };
    let q = abs as f64 / 100.0;
    let r = if rollspeed as f64 <= q { rollangle } else { (q * rollangle as f64 / rollspeed as f64) as f32 };
    (r as f64 * sign) as f32
}

/// `V_CalcViewRoll` (0x100b6910): nothing without a view entity. The observer test and the stunt
/// bit are the local player's `prevstate` (`+0x260`, `+0x26c`), the angles the view entity's.
/// Not observing (`iuser1 < 1`): `viewangles[2] += V_CalcRoll(angles, simvel, dive ? 30 : 0.5,
/// 10)`, at once, on the ground and in the air alike; dead with a view height, 80. Observing: 0.
pub fn view_roll(p: &mut ref_params_t, view: Option<RollEntity>, local: Option<RollEntity>) {
    let Some(view) = view else { return };
    let local = local.unwrap_or_default();
    if local.iuser1 >= 1 {
        p.viewangles[2] = 0.0;
        return;
    }
    let rollangle = if local.iuser4 & stunt::DIVING != 0 { 30.0 } else { 0.5 };
    p.viewangles[2] += calc_roll(view.angles, p.simvel, rollangle, 10.0);
    if p.health < 1 && p.viewheight[2] != 0.0 {
        p.viewangles[2] = 80.0;
    }
}
