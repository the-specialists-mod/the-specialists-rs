//! TS 2.1's view bob (`V_CalcBob` 0x100b5c80, issue #164). f64 stands in for the x87 stack, as
//! in the port's 3.0 bob.

use ts_client::view::{Bob, BobInput};

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
