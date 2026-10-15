use super::*;

/// The melee back test and speed per build (issue #170): the old all-`float` dot here was exactly
/// -0.5 (backward); every TS build's unrounded dot is -0.49999999 (not backward), and the speed is
/// stored from the build's sum (100.0000076, where the old `float` length was 100).
#[test]
fn melee_back_dot_and_speed_per_build() {
    let f = |b: [u32; 3]| b.map(f32::from_bits);
    let fwd = f([0x3f7c_b7d3, 0xbe23_711c, 0]);
    let vel = f([0xc298_35d3, 0x4348_2ee3, 0xbdcf_3cb1]);
    for form in [BackDot::Kept, BackDot::Stored] {
        assert_eq!(back_dot(Arith::Msvc, form, fwd, vel), (false, -0.5), "{form:?}");
    }
    assert_eq!(back_dot(Arith::Gcc, BackDot::Kept, [1.0, 0.0, 0.0], [-3.0, 0.0, 0.0]), (true, -1.0));
    let v = f([0x410a_37da, 0x41a7_cb22, 0xc2c2_c961]);
    assert_eq!(melee_speed(Arith::Msvc, v).to_bits(), 0x42c8_0001);
}
