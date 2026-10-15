//! TS 2.1's movement overrides against the 2.1 `mp.dll` formulas, and TS 3.0's defaults beside
//! them (the core plugin must be unchanged).

use super::*;
use ts30_gameplay::TS30 as CORE;

const ARITHS: [Arith; 2] = [Arith::Msvc, Arith::Gcc];

#[test]
fn constants_are_2_1s() {
    // The `fsqrt` of a `double`, stored: the same in both builds.
    assert_eq!(WALLJUMP_UP_VELOCITY, 80_000f64.sqrt() as f32);
    assert_eq!(WALLJUMP_UP_VELOCITY, Gcc::xd(80_000.0).sqrt().f());
    assert_eq!(WALLCLIMB_VELOCITY, 96_000f64.sqrt() as f32);
    assert_eq!(WALLCLIMB_VELOCITY, Gcc::xd(96_000.0).sqrt().f());
    assert!((WALLJUMP_UP_VELOCITY - 282.842_7).abs() < 1e-4 && (WALLCLIMB_VELOCITY - 309.838_7).abs() < 1e-4);
    // The `float` 1/70 at 0x100885e8.
    assert_eq!(0.014_285_714f32.to_bits(), 0x3c6a_0ea1);
    assert_eq!(STRAFE_BUTTONS, 1 << 9 | 1 << 10);
}

#[test]
fn value_hooks() {
    let t = &crate::TS21;
    assert_eq!(t.ground_accel(), GroundAccel::Constant(0.7));
    assert_eq!(t.recover_input_scale(), 0.1);
    assert_eq!(t.prone_forward_scale(), Some(0.0));
    assert_eq!(
        (t.wall_jump_speed(), t.wall_jump_up_velocity(), t.wall_climb_velocity()),
        (170.0, WALLJUMP_UP_VELOCITY, WALLCLIMB_VELOCITY)
    );
    assert!(!t.wall_dive());
    assert_eq!(t.max_player_speed(), 310.0);

    // TS 3.0's, unchanged.
    assert_eq!(CORE.ground_accel(), GroundAccel::BySpeed);
    assert_eq!(CORE.recover_input_scale(), 0.5);
    assert_eq!(CORE.prone_forward_scale(), None);
    assert_eq!((CORE.wall_jump_speed(), CORE.wall_jump_up_velocity()), (210.0, shared::pm::ts::JUMP_VELOCITY));
    assert_eq!(CORE.wall_climb_velocity(), shared::pm::ts::WALLCLIMB_VELOCITY);
    assert!(CORE.wall_dive());
    assert_eq!(CORE.max_player_speed(), 330.0);
}

#[test]
fn sdk_movement_hooks() {
    // Issue #145: 2.1's SDK table (512 entries, 12 characters), in-eye pitch × -2 (0x10085f14),
    // CalcRoll dividing first (0x10033af1), Friction's edge start through 1 / speed (0x10031728).
    let t = &crate::TS21;
    assert_eq!(t.texture_limits(), TextureLimits { max_entries: 512, name_chars: 12 });
    assert_eq!(IN_EYE_PITCH_SCALE.to_bits(), 0xc000_0000);
    assert_eq!(t.in_eye_pitch(-4.5), 9.0);
    assert_eq!(t.in_eye_pitch(10.0), -20.0);
    assert!(t.roll_divides_first() && t.friction_edge_by_reciprocal());

    // TS 3.0's, unchanged.
    assert_eq!(CORE.texture_limits(), TextureLimits { max_entries: 1024, name_chars: 15 });
    assert_eq!(CORE.in_eye_pitch(-4.5), -4.5);
    assert!(!CORE.roll_divides_first() && !CORE.friction_edge_by_reciprocal());
}

#[test]
fn compares_are_ieee() {
    // Issue #105: 2.1's `mp.dll` compares through the parity flag; 3.0's tests C3/C0.
    assert!(crate::TS21.ieee_compares());
    assert!(!CORE.ieee_compares());
}

#[test]
fn speed_by_free_slots() {
    let t = &crate::TS21;
    // 170 + free * 2, truncated (1/70.f is just above 1/70, so whole steps stay whole).
    for (free, speed) in [(0, 170.0), (1, 172.0), (35, 240.0), (69, 308.0), (70, 310.0), (81, 310.0)] {
        assert_eq!(t.speed_by_slots(free, false), speed, "free {free}");
        // No The One speed in 2.1.
        assert_eq!(t.speed_by_slots(free, true), speed, "free {free}, The One");
    }
    // Every result is within the clamp, so `SetPlayerSpeed`'s cap doesn't bite.
    assert!((0..=81).all(|f| (170.0..=MAX_SPEED).contains(&t.speed_by_slots(f, false))));

    assert_eq!((CORE.speed_by_slots(0, false), CORE.speed_by_slots(81, false), CORE.speed_by_slots(0, true)), (210.0, 330.0, 330.0));
}

/// `FixVelocity`'s MSVC arithmetic written out in `f64`.
fn fix_velocity_msvc(v: Vec3, limit: f32) -> Vec3 {
    let len = (0.0 + f64::from(v[0]) * f64::from(v[0]) + f64::from(v[1]) * f64::from(v[1]) + f64::from(v[2]) * f64::from(v[2])).sqrt();
    let stored = len as f32;
    if len == 0.0 || stored <= limit {
        return v;
    }
    let k = f64::from(limit) / f64::from(stored);
    [(k * f64::from(v[0])) as f32, (f64::from(k as f32) * f64::from(v[1])) as f32, v[2]]
}

#[test]
fn fix_velocity_caps_the_3d_speed_on_x_and_y() {
    let t = &crate::TS21;
    for arith in ARITHS {
        let fix = |mut v: Vec3, limit: f32| {
            t.fix_velocity(arith, &mut v, limit);
            v
        };
        assert_eq!(fix([300.0, 400.0, 0.0], 250.0), [150.0, 200.0, 0.0]);
        // At or below the limit, and the zero vector: untouched.
        assert_eq!(fix([300.0, 400.0, 0.0], 500.0), [300.0, 400.0, 0.0]);
        assert_eq!(fix([0.0; 3], 0.0), [0.0; 3]);
        // The length is 3D but only x and y are scaled: a fast fall shrinks the horizontal speed.
        assert_eq!(fix([30.0, 40.0, 1200.0], 620.0)[2], 1200.0);
        let v = fix([300.0, 0.0, 400.0], 250.0);
        assert_eq!(v, [150.0, 0.0, 400.0]);
    }
    // MSVC scales x by the ratio on the stack and y by the stored one.
    let mut differs = false;
    for i in 0..2000 {
        let f = i as f32;
        let v = [137.3 + f * 0.731, 211.9 - f * 0.377, 25.1 + f * 0.11];
        let limit = 101.7 + f * 0.013;
        let mut ours = v;
        t.fix_velocity(Arith::Msvc, &mut ours, limit);
        let want = fix_velocity_msvc(v, limit);
        assert_eq!(ours.map(f32::to_bits), want.map(f32::to_bits), "{v:?} {limit}");
        let stored = v.map(f64::from).iter().map(|c| c * c).sum::<f64>().sqrt() as f32;
        let k = f64::from(limit) / f64::from(stored);
        differs |= want[0] != (f64::from(k as f32) * f64::from(v[0])) as f32;
    }
    assert!(differs, "no case tells the unrounded ratio from the stored one");

    // TS 3.0's FixVelocity is empty.
    let mut v = [1000.0, 1000.0, 0.0];
    CORE.fix_velocity(Arith::Msvc, &mut v, 10.0);
    assert_eq!(v, [1000.0, 1000.0, 0.0]);
}

#[test]
fn jump_pushes() {
    let t = &crate::TS21;
    for arith in ARITHS {
        let push = |jump, buttons, maxspeed, mut v: Vec3| {
            t.jump_push(arith, jump, buttons, maxspeed, &mut v);
            v
        };
        // Super jump: +100 along the velocity, unclamped.
        assert_eq!(push(Jump::High, 0, 310.0, [300.0, 0.0, 0.0]), [400.0, 0.0, 0.0]);
        // Normal jump: nothing without a strafe key.
        assert_eq!(push(Jump::Normal, 0, 310.0, [100.0, 0.0, 0.0]), [100.0, 0.0, 0.0]);
        for key in [1 << 9, 1 << 10] {
            // From slow: up to +100.
            assert_eq!(push(Jump::Normal, key, 310.0, [100.0, 0.0, 0.0]), [200.0, 0.0, 0.0]);
            // Capped at maxspeed.
            assert_eq!(push(Jump::Normal, key, 310.0, [300.0, 0.0, 0.0]), [310.0, 0.0, 0.0]);
            let v = push(Jump::Normal, key, 310.0, [-150.0, 200.0, 0.0]);
            assert!(((v[0] * v[0] + v[1] * v[1]).sqrt() - 310.0).abs() < 1e-3, "{v:?}");
            assert!(v[0] < 0.0 && v[1] > 0.0);
        }
        // Standing still: no direction, no push.
        assert_eq!(push(Jump::Normal, 1 << 9, 310.0, [0.0; 3]), [0.0; 3]);
        assert_eq!(push(Jump::High, 0, 310.0, [0.0; 3]), [0.0; 3]);
    }
    // MSVC: `dir * 100 + v`, rounded once, with dir from 2.1's VectorNormalize.
    let v = [123.4f32, -56.7, 0.0];
    let mut ours = v;
    t.jump_push(Arith::Msvc, Jump::High, 0, 310.0, &mut ours);
    let len = (f64::from(v[0]).powi(2) + f64::from(v[1]).powi(2) + f64::from(v[2]).powi(2)).sqrt();
    let dir = v.map(|c| (1.0 / len * f64::from(c)) as f32);
    assert_eq!(ours, [(f64::from(dir[0]) * 100.0 + f64::from(v[0])) as f32, (f64::from(dir[1]) * 100.0 + f64::from(v[1])) as f32, 0.0]);

    // TS 3.0 pushes nothing.
    let mut v = [100.0, 0.0, 0.0];
    CORE.jump_push(Arith::Msvc, Jump::Normal, 1 << 9, 310.0, &mut v);
    CORE.jump_push(Arith::Msvc, Jump::High, 0, 310.0, &mut v);
    assert_eq!(v, [100.0, 0.0, 0.0]);
}

#[test]
fn time_scale_zero_is_normal() {
    let t = &crate::TS21;
    assert_eq!(t.scaled_frametime(0.01, 1.0), 0.01);
    assert_eq!(t.scaled_frametime(0.01, 0.5), 0.005);
    assert_eq!(t.scaled_frametime(0.01, 0.0), 0.01, "2.1's normal speed");
    assert_eq!(t.scaled_frametime(0.01, 0.05), 0.0, "frozen below 0.1");
    assert!(t.scaled_frametime(0.01, f32::NAN).is_nan(), "a NaN scale isn't frozen");
    // TS 3.0: 0 and NaN freeze.
    assert_eq!(
        (CORE.scaled_frametime(0.01, 0.5), CORE.scaled_frametime(0.01, 0.0), CORE.scaled_frametime(0.01, f32::NAN)),
        (0.005, 0.0, 0.0)
    );
}

#[test]
fn punch_decays_while_dead() {
    let t = &crate::TS21;
    for arith in ARITHS {
        // 10 - (0.5 * 10 + 10) * 0.01 = 9.85.
        let p = t.drop_punch_angle(arith, [10.0, 0.0, 0.0], true, 1.0, 0.01);
        assert_eq!(p, [9.85, 0.0, 0.0]);
        assert_eq!(t.drop_punch_angle(arith, [10.0, 0.0, 0.0], false, 0.0, 0.01), p, "a time scale of 0 counts as 1");
        assert_eq!(t.drop_punch_angle(arith, [0.0, 0.1, 0.0], false, 1.0, 0.01), [0.0; 3], "clamped at 0");
        assert_eq!(t.drop_punch_angle(arith, [0.0, 3.0, 4.0], false, 0.5, 0.02)[0], 0.0);
        // TS 3.0 zeroes it while dead, and a scale of 0 stops the decay.
        assert_eq!(CORE.drop_punch_angle(arith, [10.0, 0.0, 0.0], true, 1.0, 0.01), [0.0; 3]);
        assert_eq!(CORE.drop_punch_angle(arith, [10.0, 0.0, 0.0], false, 0.0, 0.01), [10.0, 0.0, 0.0]);
        assert_eq!(CORE.drop_punch_angle(arith, [10.0, 0.0, 0.0], false, 1.0, 0.01), [9.85, 0.0, 0.0]);
    }
    // MSVC, written out: the length is stored to a float before the decay.
    let v = [3.3f32, -1.7, 0.9];
    let len = (f64::from(v[0]).powi(2) + f64::from(v[1]).powi(2) + f64::from(v[2]).powi(2)).sqrt();
    let dir = v.map(|c| (1.0 / len * f64::from(c)) as f32);
    let lf = f64::from(len as f32);
    let d = (lf - (0.5 * lf + 10.0) * f64::from(0.013f32) * f64::from(0.37f32)) as f32;
    assert_eq!(t.drop_punch_angle(Arith::Msvc, v, false, 0.37, 0.013), dir.map(|c| c * d));
}

#[test]
fn step_interval_has_no_floor() {
    let t = &crate::TS21;
    for arith in ARITHS {
        assert_eq!(t.step_interval(arith, 300, 1.0), 300);
        assert_eq!(t.step_interval(arith, 300, 0.0), 300, "2.1's normal speed");
        assert_eq!(t.step_interval(arith, 300, 0.5), 600);
        assert_eq!(t.step_interval(arith, 300, 0.35), (300.0 / f64::from(0.35f32)) as i32);
        // No 0.1 floor. The `float` 0.05 is just above 0.05, so the quotient truncates to 5999.
        assert_eq!(t.step_interval(arith, 300, 0.05), (300.0 / f64::from(0.05f32)) as i32);
        assert_eq!(t.step_interval(arith, 300, 0.05), 5999);
        // TS 3.0: `1/max(fuser1, 0.1) * interval`.
        assert_eq!(CORE.step_interval(arith, 300, 0.5), 600);
        assert_eq!(CORE.step_interval(arith, 300, 0.05), (1.0 / f64::from(0.1f32) * 300.0) as i32);
    }
}
