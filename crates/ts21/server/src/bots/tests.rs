use super::attack::kung_fu_wait;
use super::*;
use ts_server::bots::files::{BotNames, BotSay, FREETIME, KILLED, OFFENSE};
use ts_server::bots::rules::CORE;

fn pev(fuser1: f32) -> entvars_t {
    entvars_t { fuser1, ..Default::default() }
}

#[test]
fn creation_overrides() {
    let r: &dyn NativeBotRules = &BOTS;
    assert!(r.random_model_in_teamplay());
    assert!(!r.learn_on_empty_graph());
    assert!(!r.team_key());
    // No 0.1 floor (`this[0x97f] = skill`).
    assert_eq!(r.stored_skill(0.05), 0.05);
    assert_eq!(r.stored_skill(1.5), 1.5);
    assert_eq!(r.texts(), Some(TEXTS));
    // 3.0 unchanged.
    let c: &dyn NativeBotRules = &CORE;
    assert!(!c.random_model_in_teamplay() && c.learn_on_empty_graph() && c.team_key());
    assert_eq!(c.stored_skill(0.05), 0.1);
    assert_eq!(c.texts(), None);
}

#[test]
fn bot_texts_are_2_1s() {
    let names = BotNames::parse(Some(TEXTS.names));
    // 31 lines: 22 names and 9 blank lines, which the loader skips.
    assert_eq!(TEXTS.names.split(|&c| c == b'\n').filter(|l| !l.is_empty()).count(), 31);
    assert_eq!(names.count, 22);
    let skilled: Vec<(String, f32)> = (0..names.count)
        .filter(|&i| names.skill(i) > 0.0)
        .map(|i| (String::from_utf8_lossy(names.name(i)).into_owned(), names.skill(i)))
        .collect();
    let want = [
        ("The Sprayer", 0.1),
        ("Hollywood", 1.1),
        ("Bennett", 0.8),
        ("Tango", 0.15),
        ("Sadsa", 0.2),
        ("Dillon", 1.0),
        ("Action Specialist", 0.95),
        ("The Hitman", 1.5),
    ];
    assert_eq!(skilled, want.map(|(n, s)| (n.to_owned(), s as f32)));
    assert_eq!(names.name(0), b"Lombardi");
    let say = BotSay::parse(Some(TEXTS.say));
    assert_eq!(say.count, [10, 11, 20]);
    assert_eq!(say.line(OFFENSE as usize, 0), b"You suck, %s!");
    assert_eq!(say.line(FREETIME as usize, 0), b"How do you flip?");
    assert_eq!(say.line(KILLED as usize, 19), b"Affanculo..");
}

#[test]
fn timers_multiply_by_slow_inv() {
    // `time + min(slowInv * k / skill, max)`, slowInv = 1 / max(fuser1, 0.1f).
    assert_eq!(timer(10.0, 0.1, &pev(1.0), 1.0, 1.0), (10.0f64 + 0.1) as f32);
    assert_eq!(timer(10.0, 0.1, &pev(0.5), 0.5, 1.0), (10.0f64 + (1.0 / 0.5f32 as f64) * 0.1 / 0.5) as f32);
    assert_eq!(timer(10.0, 3.0, &pev(0.01), 1.0, 7.0), 17.0);
    assert_eq!(timer(0.0, 2.0, &pev(1.0), 0.25, 5.0), 5.0);
    // A NaN takes the cap, as the x87 compare does.
    assert_eq!(timer(0.0, 0.1, &pev(1.0), f32::NAN, 1.0), 1.0);
    let r: &dyn NativeBotRules = &BOTS;
    assert_eq!(r.delay(4.0, 1.0, &pev(0.2), 2.0, 10.0), timer(4.0, 1.0, &pev(0.2), 2.0, 10.0));
    // 3.0 divides by `max(fuser1, 0.01)`.
    assert_eq!(CORE.delay(10.0, 3.0, &pev(0.01), 1.0, 7.0), 17.0);
    assert_eq!(CORE.delay(10.0, 0.1, &pev(0.05), 1.0, 1.0), 11.0);
    assert_eq!(timer(10.0, 0.1, &pev(0.05), 1.0, 1.0), 11.0);
}

#[test]
fn think_gate_and_turn_speed() {
    let r: &dyn NativeBotRules = &BOTS;
    assert!(r.runs_ai(&pev(1.0)));
    assert!(r.runs_ai(&pev(0.0)));
    assert!(r.runs_ai(&pev(0.1)));
    assert!(!r.runs_ai(&pev(0.09)));
    assert!(r.runs_ai(&pev(f32::NAN)));
    // 3.0: `max(fuser1, 0.01) > 0.1`.
    assert!(!CORE.runs_ai(&pev(0.1)) && !CORE.runs_ai(&pev(0.0)));
    assert_eq!(r.turn_rate(0.1), 30.0);
    assert_eq!(r.turn_rate(3.0), 30.0);
    assert_eq!(CORE.turn_rate(3.0), 20.0);
}

#[test]
fn angle_mod_is_the_sdks() {
    assert_eq!(angle_mod(10.0), 10.0);
    assert_eq!(angle_mod(-10.0), 350.0);
    assert_eq!(angle_mod(360.0), 0.0);
    assert_eq!(angle_mod(725.0), 5.0);
    // `a + 360 * ((int)(a / 360) + 1)`: from -360 down it stays negative (3.0's gives 350).
    assert_eq!(angle_mod(-370.0), -370.0);
    assert_eq!(angle_mod(-720.0), -1080.0);
    // Unrounded on the x87 stack.
    let a = -10.123457f32;
    assert_eq!(angle_mod(a), 360.0 + a as f64);
}

#[test]
fn ideal_angles_snap_the_view() {
    let mut v = entvars_t { punchangle: [1.5, -2.0, 0.25], ..Default::default() };
    set_ideal_angles(&mut v, [10.0, 200.0, 3.0]);
    assert_eq!(v.v_angle, [11.5, 198.0, 3.25]);
    assert_eq!(v.angles[0], 11.5 * -0.333_333_34);
    assert_eq!(v.angles[2], 0.0);
    assert_eq!(v.ideal_yaw, -160.0);
    set_ideal_angles(&mut v, [0.0, -190.0, 0.0]);
    assert_eq!(v.ideal_yaw, 170.0);
    set_ideal_angles(&mut v, [0.0, 180.0, 0.0]);
    assert_eq!(v.ideal_yaw, 180.0);
}

#[test]
fn yaw_turns_at_a_constant_rate() {
    // 30 * 0.01 * 10 = 3 degrees a frame, the short way round.
    let mut v = entvars_t { ideal_yaw: 90.0, ..Default::default() };
    change_yaw(&mut v, 30.0, 0.01);
    assert_eq!(v.angles[1], (30.0f64 * 0.01f32 as f64 * 10.0) as f32);
    assert_eq!(v.v_angle[1], v.angles[1]);
    let mut v = entvars_t { ideal_yaw: -170.0, angles: [0.0, 170.0, 0.0], ..Default::default() };
    change_yaw(&mut v, 30.0, 0.01);
    let step = (30.0f64 * 0.01f32 as f64 * 10.0) as f32;
    assert_eq!(v.angles[1], (step as f64 + 170.0) as f32);
    // Close enough: lands on the target.
    let mut v = entvars_t { ideal_yaw: 1.0, ..Default::default() };
    change_yaw(&mut v, 30.0, 0.1);
    assert_eq!(v.angles[1], 1.0);
    // Dead: nothing.
    let mut v = entvars_t { ideal_yaw: 90.0, deadflag: 2, ..Default::default() };
    change_yaw(&mut v, 30.0, 0.1);
    assert_eq!(v.angles[1], 0.0);
    // The pitch isn't turned.
    let mut v = entvars_t { ideal_yaw: 90.0, v_angle: [12.0, 0.0, 0.0], ..Default::default() };
    change_yaw(&mut v, 30.0, 0.1);
    assert_eq!(v.v_angle[0], 12.0);
}

#[test]
fn button_slots() {
    let mut s = Buttons::default();
    s.press(btn::JUMP, 1.0, 10.0);
    s.press(btn::DUCK | btn::BACK, 1.0, 10.0);
    s.press(btn::JUMP | btn::FORWARD, 0.0, 10.0);
    // Held while `time <= expire`: a zero-length press holds at its own time.
    assert_eq!(s.update(10.0), btn::JUMP | btn::DUCK | btn::BACK | btn::FORWARD);
    // Masks are slots, not bits: the zero-length JUMP|FORWARD press doesn't let go of JUMP.
    assert_eq!(s.update(10.5), btn::JUMP | btn::DUCK | btn::BACK);
    s.release(btn::JUMP);
    assert_eq!(s.update(10.6), btn::DUCK | btn::BACK);
    // The same mask again reuses its slot.
    s.press(btn::DUCK | btn::BACK, 2.0, 10.6);
    assert_eq!(s.mask.iter().filter(|&&m| m != 0).count(), 1);
    assert_eq!(s.update(12.6), btn::DUCK | btn::BACK);
    assert_eq!(s.update(12.7), 0);
    // Through the brain: the slots live in its ruleset state, which `Reset` drops.
    let mut b = Brain::new(1.0);
    let r: &dyn NativeBotRules = &BOTS;
    r.press(&mut b, btn::ATTACK, 0.1, 5.0);
    r.update_buttons(&mut b, 5.05);
    assert_eq!(b.buttons, btn::ATTACK);
    r.release(&mut b, btn::ATTACK, 5.05);
    r.update_buttons(&mut b, 5.06);
    assert_eq!(b.buttons, 0);
    // 3.0's per-bit timers on the same brain type: a release of JUMP|FORWARD lets go of JUMP.
    let mut b = Brain::new(1.0);
    CORE.press(&mut b, btn::JUMP, 1.0, 10.0);
    CORE.press(&mut b, btn::JUMP | btn::FORWARD, 0.0, 10.0);
    CORE.update_buttons(&mut b, 10.5);
    assert_eq!(b.buttons, 0);
}

#[test]
fn team_rounds_roams_to_the_other_side() {
    assert_eq!(team_rounds_class("Specialists", false), "info_player_deathmatch");
    assert_eq!(team_rounds_class("specialists", false), "info_player_deathmatch");
    assert_eq!(team_rounds_class("Mercenary", false), "info_player_start");
    assert_eq!(team_rounds_class("", false), "info_player_start");
    assert_eq!(team_rounds_class("Specialists", true), "info_player_start");
    assert_eq!(team_rounds_class("Mercenary", true), "info_player_deathmatch");
}

#[test]
fn kung_fu_wait_is_clamped() {
    assert_eq!(kung_fu_wait(1.0, 1.0), 0.3);
    assert_eq!(kung_fu_wait(1.0, 0.5), 0.3 / 0.5);
    assert_eq!(kung_fu_wait(10.0, 0.1), 3.0);
    assert_eq!(kung_fu_wait(1.0, f32::NAN), 3.0);
}
