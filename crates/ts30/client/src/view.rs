//! TS 3.0's view: the bob (`V_CalcBob`), the eye height (the view-height easing and
//! `V_SmoothStairs`), the third-person camera offsets and the view roll, on the core's state
//! types ([`Bob`], [`EyeZ`], [`Stairs`], [`CamOffsets`]).

use goldsrc::ext::render::{Vec3, vec3 as v};
use goldsrc::sys::ref_params_t;
use shared::ts::stunt;
use ts_client::view::{Bob, BobInput, CamInput, CamOffsets, EyeZ, RollEntity, Stairs};

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
pub fn bob(b: &mut Bob, i: &BobInput) -> f32 {
    if i.skip {
        return 0.0;
    }
    let (lvl, pitch_lvl) = bob_levels(i.cl_bob);
    let amp_part = |amp: f32| 0.75 - amp as f64 * 0.5;
    let z_part = |amp: f32| 0.5 - amp as f64 * 0.65;
    if i.on_ground && i.time != b.last_time && i.viewheight != 0.0 {
        b.last_time = i.time;
        let speed = ((i.simvel[1] as f64).powi(2) + (i.simvel[0] as f64).powi(2)).sqrt();
        let sp = (speed * f64::from(1.0f32 / 300.0)).min(f64::from(1.1f32));
        b.phase = ((i.frametime * 5.5 * i.ts) as f64 * sp * 100.0 + b.phase as f64) as f32;
        if b.phase > 360.0 {
            b.phase = 0.0;
        }
        if b.peak <= b.amp {
            b.amp = (speed * 0.01) as f32;
            b.peak = b.amp;
        } else {
            b.amp = ((b.amp + 2.5) as f64 * sp * 0.5) as f32;
        }
        let phase = b.phase as f64;
        let s2 = ((phase * (std::f64::consts::PI / 90.0)).sin() * b.amp as f64) as f32;
        let s1 = ((phase * (std::f64::consts::PI / 180.0)).sin() * b.amp as f64) as f32;
        let k = amp_part(b.amp);
        for a in 0..3 {
            b.ofs[a] = (s1 as f64 * i.right[a] as f64 * 0.3 + i.forward[a] as f64 * k + s2 as f64 * i.up[a] as f64 * 0.11) as f32;
        }
        b.ofs[2] = ((z_part(b.amp) + b.ofs[2] as f64) * lvl as f64) as f32;
        b.pitch = s2 * pitch_lvl * 0.25;
    } else {
        if b.amp as f64 > 0.01 {
            b.amp = (b.amp as f64 * 0.5) as f32;
        }
        let k = amp_part(b.amp);
        b.ofs = i.forward.map(|f| (f as f64 * k) as f32);
        // The forward z part is overwritten, not added to (`0x10067179..0x100671b9`).
        b.ofs[2] = (z_part(b.amp) * lvl as f64) as f32;
        b.pitch = 0.0;
    }
    b.ofs[0] *= lvl;
    b.ofs[1] *= lvl;
    smooth_bob_z(b, i.frametime);
    b.amp
}

/// `V_SmoothBobZ` (`0x10066de0`): the smoothed z moves `(z_smooth − z)·ft·20` toward the
/// offset's z, never past it, and becomes the offset's z.
pub fn smooth_bob_z(b: &mut Bob, ft: f32) {
    let d = b.smooth_z - b.ofs[2];
    let mut step = d * ft * 20.0;
    if step.abs() > d.abs() {
        step = d;
    }
    b.smooth_z -= step;
    b.ofs[2] = b.smooth_z;
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

/// The eye (`0x100679ee..0x10067a46`): `simorg` with the view height eased at
/// `min(15·ft, 1)` on z only, then `V_SmoothStairs` on that eye z.
pub fn eye(e: &mut EyeZ, simorg: Vec3, viewheight: Vec3, on_ground: bool, ft: f32) -> Vec3 {
    e.vh = ease_view_height(e.vh, viewheight[2], ft);
    [simorg[0], simorg[1], smooth_stairs(&mut e.stairs, simorg[2] + e.vh, on_ground, ft)]
}

/// The third-person body cam z: a second `V_SmoothStairs` this frame (`0x10068313`) on
/// the same state.
pub fn body_cam_z(e: &mut EyeZ, z: f32, on_ground: bool, ft: f32) -> f32 {
    smooth_stairs(&mut e.stairs, z, on_ground, ft)
}

/// View-height easing in `V_CalcNormalRefdef` (`0x100679ee..0x10067a43`):
/// `vh += (target − vh)·min(15·ft, 1)`.
pub fn ease_view_height(vh: f32, target: f32, ft: f32) -> f32 {
    vh + (target - vh) * (15.0 * ft).min(1.0)
}

/// `V_SmoothStairs` (`0x100677d0`), on the eye z (`vieworg.z`, view height already added).
/// On the ground the smoothed z follows at `min(15·ft, 1)` and the offset is zeroed; in the
/// air it is the eye z, plus an offset set on the frame the ground is left to
/// `z − smoothed z` (that sign is TS's, from the assembly: `fld [z]; fsub [0x10191d84]`) and
/// decayed at the same rate after each use. No snap: TS has none.
pub fn smooth_stairs(s: &mut Stairs, z: f32, on_ground: bool, ft: f32) -> f32 {
    let k = (15.0 * ft).min(1.0);
    if on_ground {
        s.ofs = 0.0;
        s.smooth_z += (z - s.smooth_z) * k;
    } else {
        if s.was_on_ground {
            s.ofs = z - s.smooth_z;
        }
        s.smooth_z = z;
    }
    let out = s.smooth_z + s.ofs;
    s.ofs *= 1.0 - k;
    s.was_on_ground = on_ground;
    out
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

/// The smoothing half of the third-person block of `V_CalcNormalRefdef` (`0x10068139..0x1006823e`):
/// eases the extra up offset toward `32 - up` while ducked, and the extra back offset toward
/// -180 (dead) or -92 (stunt), both at `ft * 5` (unclamped). Returns the offsets to use.
fn cam_smooth(up_ofs: &mut f32, back_ofs: &mut f32, back: f32, up: f32, s: &CamInput) -> (f32, f32) {
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

/// The third-person camera offsets: [`cam_base`] of `cl_camtype`, then [`cam_smooth`].
pub fn camera_offsets(ofs: &mut CamOffsets, c: &CamInput) -> (f32, f32, f32) {
    let (back, right, up) = cam_base(c.camtype, c.cam_back, c.cam_right, c.cam_up);
    let (back, up) = cam_smooth(&mut ofs.up, &mut ofs.back, back, up, c);
    (back, right, up)
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

/// `V_CalcViewRoll` (`0x100675b0`): the stunt roll, all from the view entity
/// (`GetEntityByIndex(pparams->viewentity)`). Nothing without one; an observer
/// (`prevstate.iuser1 > 0`) gets roll 0. Otherwise the eased roll (`0x10191d80`) follows
/// `V_CalcRoll(ent->angles, simvel, target, 10)` at `min(rate·ft, 1)` and is added to the view
/// roll, so it scales with sideways speed and is 0 on the ground. Dead with a view height, the
/// roll is then set to 80 (after the easing has run).
pub fn view_roll(roll: &mut f32, p: &mut ref_params_t, ent: Option<RollEntity>) {
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

#[cfg(test)]
mod tests {
    use super::*;

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
            slowmo_factor100: 0.0,
            forward: [1.0, 0.0, 0.0],
            right: [0.0, -1.0, 0.0],
            up: [0.0, 0.0, 1.0],
        }
    }

    #[test]
    fn bob_skips_without_touching_state() {
        let mut b = Bob { phase: 10.0, amp: 1.0, peak: 1.0, ofs: [1.0, 2.0, 3.0], pitch: 0.5, ..Bob::default() };
        let before = b;
        assert_eq!(bob(&mut b, &BobInput { skip: true, ..bob_input(1.0, 200.0) }), 0.0);
        assert_eq!(b, before, "the gun offset is left stale");
    }

    /// The phase only advances on the ground, with a new time and a non-zero view height.
    #[test]
    fn bob_advances_only_when_it_should() {
        let mut b = Bob::default();
        let amp = bob(&mut b, &bob_input(1.0, 200.0));
        assert!((amp - 2.0).abs() < 1e-5, "amplitude = speed * 0.01: {amp}");
        assert_eq!(b.peak, b.amp);
        // phase = ft * 5.5 * ts * min(200/300, 1.1) * 100
        assert!((b.phase - 0.01 * 5.5 * (200.0 / 300.0) * 100.0).abs() < 1e-4, "{}", b.phase);
        let phase = b.phase;
        // Same time: no advance, and the amplitude halves.
        bob(&mut b, &bob_input(1.0, 200.0));
        assert_eq!(b.phase, phase);
        assert_eq!(b.amp, 1.0);
        assert_eq!(b.pitch, 0.0);
        // No view height (a stunt's NO_VIEW) or in the air: likewise.
        bob(&mut b, &BobInput { viewheight: 0.0, ..bob_input(2.0, 200.0) });
        bob(&mut b, &BobInput { on_ground: false, ..bob_input(3.0, 200.0) });
        assert_eq!(b.phase, phase);
        assert_eq!(b.amp, 0.25);
        // Halving stops at 0.01.
        b.amp = 0.01;
        bob(&mut b, &BobInput { on_ground: false, ..bob_input(4.0, 200.0) });
        assert_eq!(b.amp, 0.01);
    }

    /// Below the peak the amplitude eases back as `(amp + 2.5) * sp * 0.5`, leaving the peak.
    #[test]
    fn bob_amplitude_eases_back_below_peak() {
        let mut b = Bob { amp: 0.5, peak: 3.0, ..Bob::default() };
        let amp = bob(&mut b, &bob_input(1.0, 300.0));
        assert!((amp - (0.5 + 2.5) * 1.0 * 0.5).abs() < 1e-5, "{amp}");
        assert_eq!(b.peak, 3.0);
        // Reaching the peak again sets it from the speed.
        b.amp = 3.0;
        let amp = bob(&mut b, &bob_input(2.0, 150.0));
        assert!((amp - 1.5).abs() < 1e-5, "{amp}");
        assert_eq!(b.peak, b.amp);
        // sp is capped at 1.1 in the phase step; the amplitude isn't.
        let mut b = Bob::default();
        let amp = bob(&mut b, &bob_input(1.0, 600.0));
        assert!((amp - 6.0).abs() < 1e-5, "{amp}");
        assert!((b.phase - 0.01 * 5.5 * 1.1 * 100.0).abs() < 1e-4, "{}", b.phase);
    }

    /// Past 360 degrees the phase resets to 0 rather than wrapping.
    #[test]
    fn bob_phase_resets_past_360() {
        let mut b = Bob { phase: 359.0, ..Bob::default() };
        bob(&mut b, &BobInput { frametime: 0.05, ..bob_input(1.0, 300.0) });
        assert_eq!(b.phase, 0.0);
        let mut b = Bob { phase: 355.0, ..Bob::default() };
        bob(&mut b, &BobInput { frametime: 0.001, ..bob_input(1.0, 300.0) });
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
        bob(&mut b, &input);
        let k = 0.75 - 0.01 * 0.5;
        assert_eq!(b.ofs[0], 0.0);
        assert!((b.ofs[1] - k * 0.5).abs() < 1e-6, "{:?}", b.ofs);
        // ft * 20 >= 1: z snaps to (0.5 - 0.65 amp) * level (the forward z isn't added).
        assert!((b.ofs[2] - (0.5 - 0.65 * 0.01) * 0.5).abs() < 1e-6, "{:?}", b.ofs);

        // On the ground at phase 45: up * sin(90)*amp*0.11 + right * sin(45)*amp*0.3.
        let mut b = Bob { phase: 45.0 - 0.01 * 5.5 * 100.0, amp: 0.0, peak: 0.0, ..Bob::default() };
        let input = BobInput { frametime: 0.01, ..bob_input(1.0, 300.0) };
        bob(&mut b, &input);
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
        smooth_bob_z(&mut b, 0.01);
        assert!((b.ofs[2] - 8.0).abs() < 1e-5, "{:?}", b.ofs);
        assert_eq!(b.smooth_z, b.ofs[2]);
        b.ofs[2] = 0.0;
        smooth_bob_z(&mut b, 0.1);
        assert_eq!(b.ofs[2], 0.0, "ft * 20 > 1 lands on the target");
    }

    #[test]
    fn stair_smoothing_on_ground() {
        let mut s = Stairs { smooth_z: 64.0, ..Stairs::default() };
        // Step up 18 units: the eye eases up at 15·ft per frame.
        let z = smooth_stairs(&mut s, 82.0, true, 0.01);
        assert!((z - (64.0 + 18.0 * 0.15)).abs() < 1e-4, "{z}");
        for _ in 0..200 {
            smooth_stairs(&mut s, 82.0, true, 0.01);
        }
        assert!((smooth_stairs(&mut s, 82.0, true, 0.01) - 82.0).abs() < 1e-3);
        // Step down and big jumps ease the same way: there is no snap.
        let z = smooth_stairs(&mut s, -100.0, true, 0.01);
        assert!((z - (82.0 - 182.0 * 0.15)).abs() < 1e-3, "{z}");
        // Slow frames (15·ft ≥ 1) follow exactly.
        assert_eq!(smooth_stairs(&mut s, 10.0, true, 0.1), 10.0);
        assert_eq!(s.ofs, 0.0);
    }

    #[test]
    fn stair_smoothing_leaving_ground() {
        // Lagging 10 below the eye (walking up stairs), then leaving the ground: TS's offset is
        // `z − smoothed`, so the view jumps 10 *above* the eye, then decays at rate 15.
        let mut s = Stairs { smooth_z: 90.0, ..Stairs::default() };
        let z = smooth_stairs(&mut s, 100.0, false, 0.01);
        assert_eq!(z, 110.0);
        assert_eq!(s.smooth_z, 100.0, "the air tracks the eye");
        assert!((s.ofs - 8.5).abs() < 1e-5);
        let z = smooth_stairs(&mut s, 101.0, false, 0.01);
        assert!((z - 109.5).abs() < 1e-4, "only the first air frame sets the offset: {z}");
        // Lagging above (stepping down) dips the view below the eye instead.
        let mut s = Stairs { smooth_z: 110.0, ..Stairs::default() };
        assert_eq!(smooth_stairs(&mut s, 100.0, false, 0.01), 90.0);
        // Landing drops the offset and eases on from the last air z.
        let z = smooth_stairs(&mut s, 80.0, true, 0.01);
        assert!((z - (100.0 - 20.0 * 0.15)).abs() < 1e-4 && s.ofs == 0.0, "{z}");
        // Leaving flat ground with no lag adds nothing.
        let mut s = Stairs { smooth_z: 50.0, ..Stairs::default() };
        assert_eq!(smooth_stairs(&mut s, 50.0, false, 0.01), 50.0);
    }

    #[test]
    fn stair_smoothing_initial_state() {
        // TS's statics: smoothed z 0 and "was on the ground", so a first frame in the air carries
        // the whole eye z as the offset, and a first frame on the ground eases up from z 0.
        let mut s = Stairs::default();
        assert_eq!(smooth_stairs(&mut s, 40.0, false, 0.01), 80.0);
        let mut s = Stairs::default();
        assert!((smooth_stairs(&mut s, 40.0, true, 0.01) - 6.0).abs() < 1e-5);
    }

    #[test]
    fn view_height_initial_state() {
        // TS's view-height static starts at 0 and feeds the stair smoothing, which also starts at
        // z 0: on the first frame on the ground both ease, the stair state toward an eye that is
        // itself still low.
        let mut w = EyeZ::default();
        assert_eq!(w.vh, 0.0);
        w.vh = ease_view_height(w.vh, 28.0, 0.01);
        assert!((w.vh - 4.2).abs() < 1e-5, "{}", w.vh);
        let z = smooth_stairs(&mut w.stairs, 100.0 + w.vh, true, 0.01);
        assert!((z - 104.2 * 0.15).abs() < 1e-4, "{z}");
        // Both settle on simorg + view height.
        for _ in 0..200 {
            w.vh = ease_view_height(w.vh, 28.0, 0.01);
            smooth_stairs(&mut w.stairs, 100.0 + w.vh, true, 0.01);
        }
        assert!((w.vh - 28.0).abs() < 1e-3 && (smooth_stairs(&mut w.stairs, 128.0, true, 0.01) - 128.0).abs() < 1e-3);
        // A slow first frame (15·ft ≥ 1) takes the view height at once.
        assert_eq!(ease_view_height(0.0, 28.0, 0.1), 28.0);
    }

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
        view_roll(&mut roll, &mut p, Some(ent));
        assert_eq!((roll, p.viewangles[2]), (0.0, 0.0), "no strafe roll on the ground");

        p.onground = 0;
        view_roll(&mut roll, &mut p, Some(RollEntity { iuser4: stunt::DIVING, ..ent }));
        // Target 10.5 (see above) at min(7.5 · 0.01, 1).
        assert!((roll - 10.5 * 0.075).abs() < 1e-4, "{roll}");
        assert_eq!(p.viewangles[2], roll);

        p.viewangles[2] = 3.0;
        view_roll(&mut roll, &mut p, Some(RollEntity { iuser1: 4, ..ent }));
        assert_eq!(p.viewangles[2], 0.0, "observer");

        let before = roll;
        p.health = 0;
        p.viewheight = [0.0, 0.0, 8.0];
        view_roll(&mut roll, &mut p, Some(ent));
        assert_eq!(p.viewangles[2], 80.0);
        assert_ne!(roll, before, "the easing still runs when dead");

        p.viewangles[2] = 3.0;
        view_roll(&mut roll, &mut p, None);
        assert_eq!(p.viewangles[2], 3.0);
        assert_eq!(view_roll_target(false, stunt::FLIP_LOW), (-10.0, 5.0));
        assert_eq!(view_roll_target(false, 0), (-5.5, 15.0));
    }

    #[test]
    fn third_person_offsets() {
        assert_eq!(cam_base(0, 0.0, 0.0, 0.0), (-48.0, 16.0, 12.0));
        assert_eq!(cam_base(3, 500.0, -50.0, 50.0), (-128.0, -24.0, 24.0));
        assert_eq!(cam_base(3, -50.0, 0.0, 0.0).0, 24.0);
        let s = CamInput { camtype: 0, cam_back: 90.0, cam_right: 0.0, cam_up: 12.0, ducked: false, alive: true, stunt: false, ft: 0.01 };
        let (mut u, mut b) = (0.0, 0.0);
        assert_eq!(cam_smooth(&mut u, &mut b, -48.0, 12.0, &s), (-48.0, 12.0));
        // Stunt pulls the camera back to -92, ducking raises it to 32; dead goes to -180.
        let stunt = CamInput { ducked: true, stunt: true, ..s };
        for _ in 0..2000 {
            cam_smooth(&mut u, &mut b, -48.0, 12.0, &stunt);
        }
        let (back, up) = cam_smooth(&mut u, &mut b, -48.0, 12.0, &stunt);
        assert!((back + 92.0).abs() < 1e-3 && (up - 32.0).abs() < 1e-3, "{back} {up}");
        let dead = CamInput { alive: false, ..s };
        for _ in 0..2000 {
            cam_smooth(&mut u, &mut b, -48.0, 12.0, &dead);
        }
        let (back, up) = cam_smooth(&mut u, &mut b, -48.0, 12.0, &dead);
        assert!((back + 180.0).abs() < 1e-3 && up == 12.0, "dead ignores the up offset: {back} {up}");
        // No stunt pull-back when the camera is already at the -128 limit.
        let (mut u, mut b) = (0.0, 0.0);
        assert_eq!(cam_smooth(&mut u, &mut b, -128.0, 12.0, &CamInput { stunt: true, ..s }).0, -128.0);
        // The whole: cl_camtype 0's (-48, 16, 12), whatever the cam_* cvars say.
        assert_eq!(camera_offsets(&mut CamOffsets::default(), &s), (-48.0, 16.0, 12.0));
    }
}
