use super::*;
use crate::TS30;

#[test]
fn slot_speed() {
    assert_eq!(speed_by_slots(0, false), 210.0);
    assert_eq!(speed_by_slots(81, false), 330.0);
    // Truncated, not rounded: 212.96 and 319.63 (issue #9).
    assert_eq!(speed_by_slots(2, false), 212.0);
    assert_eq!(speed_by_slots(74, false), 319.0);
    assert_eq!(speed_by_slots(80, false), 328.0);
    // Exact multiples of 81/120 stay whole (the float constant is just above 1/81).
    assert_eq!(speed_by_slots(27, false), 250.0);
    assert_eq!(speed_by_slots(54, false), 290.0);
    // The One runs at 330 whatever they carry.
    assert_eq!(speed_by_slots(0, true), 330.0);
    assert_eq!((TS30.speed_by_slots(2, false), TS30.max_player_speed()), (212.0, 330.0));
}

#[test]
fn wall_climb_is_the_rounded_root() {
    assert_eq!(WALLCLIMB_VELOCITY, Gcc::xd(112_000.0).sqrt().f());
}

/// TS 3.0's movement constants and choices.
#[test]
fn movement_hooks_are_3_0s() {
    assert_eq!((TS30.recover_input_scale(), TS30.wall_climb_velocity()), (0.5, WALLCLIMB_VELOCITY));
    assert_eq!((TS30.wall_jump_speed(), TS30.wall_jump_up_velocity()), (210.0, JUMP_VELOCITY));
    assert_eq!((TS30.ground_accel(), TS30.prone_forward_scale()), (GroundAccel::BySpeed, None));
    assert!(TS30.wall_dive() && !TS30.roll_divides_first() && !TS30.friction_edge_by_reciprocal());
    assert!(!TS30.ieee_compares());
    assert_eq!(TS30.texture_limits(), TextureLimits::LARGE);
    assert_eq!(TS30.in_eye_pitch(-4.5), -4.5);
    let mut v = [500.0, 0.0, 0.0];
    TS30.fix_velocity(Arith::Msvc, &mut v, 10.0);
    TS30.jump_push(Arith::Gcc, Jump::High, 0x600, 320.0, &mut v);
    assert_eq!(v, [500.0, 0.0, 0.0], "both are empty");
}

#[test]
fn time_scale_freezes_below_a_tenth() {
    assert_eq!(TS30.scaled_frametime(0.01, 0.5), 0.005);
    assert_eq!(TS30.scaled_frametime(0.01, 0.05), 0.0);
    assert_eq!(TS30.scaled_frametime(0.01, f32::NAN), 0.0, "a NaN scale is frozen");
}

/// Running on concrete (300 ms) in slow motion: sparser steps, at most ten times sparser.
#[test]
fn step_interval_scales_with_time_scale() {
    for arith in [Arith::Gcc, Arith::Msvc] {
        assert_eq!(TS30.step_interval(arith, 300, 1.0), 300);
        assert_eq!(TS30.step_interval(arith, 300, 0.5), 600);
        assert_eq!(TS30.step_interval(arith, 300, 0.05), (1.0 / f64::from(0.1f32) * 300.0) as i32);
        assert_eq!(TS30.step_interval(arith, 300, f32::NAN), TS30.step_interval(arith, 300, 0.1), "a NaN scale counts as 0.1");
    }
}

#[test]
fn punch_decays_and_dead_players_lose_it() {
    for arith in [Arith::Gcc, Arith::Msvc] {
        assert_eq!(TS30.drop_punch_angle(arith, [10.0, 0.0, 0.0], true, 1.0, 0.01), [0.0; 3]);
        assert_eq!(TS30.drop_punch_angle(arith, [10.0, 0.0, 0.0], false, 0.0, 0.01), [10.0, 0.0, 0.0]);
        assert_eq!(TS30.drop_punch_angle(arith, [10.0, 0.0, 0.0], false, 1.0, 0.01), [9.85, 0.0, 0.0]);
        assert_eq!(TS30.drop_punch_angle(arith, [0.0, 0.1, 0.0], false, 1.0, 0.01), [0.0; 3], "clamped at 0");
    }
}
