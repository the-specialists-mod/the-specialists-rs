//! 2.1's node-graph arithmetic where it differs from 3.0's (`ts_server::bots::nodes`): two of the
//! link tests keep the node differences on the x87 instead of storing them as floats. The rest
//! of 2.1's graph (`AddNode`, `IsTooClose`, `IsInAir`, `LinkNewNode`, `FindPath`, the files and
//! commands) is 3.0's.

use ts_server::bots::nodes::ftol;

/// `IsReachable`'s ladder shortcuts (`FUN_10062ed0`, 0x10062ef7..0x10062f13, the same at
/// 0x10062f28 and 0x10062f59): `sqrt(dy² + dx²)` of the unrounded differences.
pub fn ladder_link_distance(a: [f32; 3], b: [f32; 3]) -> f64 {
    let dx = f64::from(a[0]) - f64::from(b[0]);
    let dy = f64::from(a[1]) - f64::from(b[1]);
    (dy * dy + dx * dx).sqrt()
}

/// `IsGroundContinuous` (`FUN_10062d50`, `a` = the new node): `__ftol(|b - a| * 0.0625f)` probes
/// along the line, each traced 72 units down. 2.1's x87 sequence (0x10062d66..0x10062e7a):
/// `dx`, `dy` stay unrounded and `dz` is also stored (`fst`), the length is
/// `(dz × (float)dz + dy²) + dx²`, the step is `(float)(1.0f / steps)` times each difference
/// (stored as floats), and probe `i`'s x is `step.x * i + a.x` rounded once, its y and z
/// `(float)(step * i) + a`.
pub fn ground_probes(a: [f32; 3], b: [f32; 3], no_floor: &mut dyn FnMut([f32; 3]) -> bool) -> bool {
    let dx = f64::from(b[0]) - f64::from(a[0]);
    let dy = f64::from(b[1]) - f64::from(a[1]);
    let dz = f64::from(b[2]) - f64::from(a[2]);
    let dzf = f64::from(dz as f32);
    let steps = ftol(((dz * dzf + dy * dy) + dx * dx).sqrt() * f64::from(0.0625f32));
    if steps < 1 {
        return true;
    }
    let inv = f64::from((1.0 / f64::from(steps)) as f32);
    let s = [(inv * dx) as f32, (inv * dy) as f32, (inv * dzf) as f32];
    for i in 1..=steps {
        let k = f64::from(i);
        let p = [(f64::from(s[0]) * k + f64::from(a[0])) as f32, (f64::from(s[1]) * k) as f32 + a[1], (f64::from(s[2]) * k) as f32 + a[2]];
        if no_floor(p) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use ts_server::bots::nodes as core;

    /// Every probe point, all with a floor.
    fn points(f: fn([f32; 3], [f32; 3], &mut dyn FnMut([f32; 3]) -> bool) -> bool, a: [f32; 3], b: [f32; 3]) -> Vec<[f32; 3]> {
        let mut out = Vec::new();
        assert!(f(a, b, &mut |p| {
            out.push(p);
            false
        }));
        out
    }

    /// The differences aren't rounded: just under 64 links in 2.1 where 3.0's float difference
    /// is 64.0 (#163).
    #[test]
    fn ladder_distance_is_unrounded() {
        let (a, b) = ([63.999_996, 0.0, 0.0], [-2.5e-6, 0.0, 0.0]);
        assert_eq!(core::ladder_link_distance(a, b), 64.0);
        let d = ladder_link_distance(a, b);
        assert!(d < 64.0);
        assert_eq!(d, f64::from(a[0]) - f64::from(b[0]));
        assert_eq!(ladder_link_distance([3.0, 4.0, 100.0], [0.0, 0.0, -100.0]), 5.0, "2D");
    }

    /// The probe count from `(dz × (float)dz + dy²) + dx²` of the unrounded differences (#163).
    #[test]
    fn ground_probe_count() {
        let bits = |v: [u32; 3]| v.map(f32::from_bits);
        let (a, b) = (bits([0xc1c4_bcfb, 0x4441_f1b1, 0x44f2_b1eb]), bits([0xc271_345e, 0x4434_aeef, 0x44f3_086c]));
        assert_eq!((points(ground_probes, a, b).len(), points(core::ground_probes, a, b).len()), (4, 3));
        assert!(points(ground_probes, [0.0; 3], [15.0, 0.0, 0.0]).is_empty(), "under 16 units: none");
        assert!(!ground_probes([0.0; 3], [64.0, 0.0, 0.0], &mut |_| true), "a miss ends it");
    }

    /// The step is `1/steps` stored, times the difference, and x is added unrounded (#163).
    #[test]
    fn ground_probe_points() {
        let (a, b) = ([-1_310.613_2, -106.028_27, 900.773_07], [-1_276.727_8, -210.438_98, 911.782_3]);
        let (p21, p30) = (points(ground_probes, a, b), points(core::ground_probes, a, b));
        assert_eq!((p21.len(), p30.len()), (6, 6));
        assert_eq!(p21[1][1].to_bits(), (-140.831_85f32).to_bits());
        assert_eq!(p30[1][1].to_bits(), (-140.831_83f32).to_bits());
        assert_eq!(p21[5][1].to_bits(), (-210.438_995f32).to_bits());
        assert_eq!(p30[5][1].to_bits(), (-210.438_965f32).to_bits());
        assert_eq!(p21[0], p30[0]);
    }
}
