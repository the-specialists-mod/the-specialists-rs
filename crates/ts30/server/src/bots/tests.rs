//! TS 3.0's bot rules' values (moved from the bot layer's tests with the hook bodies, issue #200).

use super::*;

#[test]
fn rules_are_ts_3_0() {
    let c: &dyn NativeBotRules = &BOTS;
    assert!(!c.random_model_in_teamplay());
    assert!(c.learn_on_empty_graph());
    assert!(c.team_key());
    assert_eq!(c.texts(), None);
    // `BotSetup`'s floor.
    assert_eq!(c.stored_skill(0.01), 0.1);
    assert_eq!(c.stored_skill(-1.0), 0.1);
    assert_eq!(c.stored_skill(0.1), 0.1);
    assert_eq!(c.stored_skill(1.7), 1.7);
    // `clamp(skill, 0.5, 2) * 10`.
    assert_eq!(c.turn_rate(0.1), 5.0);
    assert_eq!(c.turn_rate(1.3), 13.0);
    assert_eq!(c.turn_rate(3.0), 20.0);
    // `min(k / slow / skill, max)` with slow = max(fuser1, 0.01).
    let mut v = entvars_t { fuser1: 1.0, ..Default::default() };
    assert_eq!(c.delay(10.0, 0.1, &v, 1.0, 1.0), brain::delay(10.0, 0.1, 1.0, 1.0, 1.0));
    v.fuser1 = 0.05;
    assert_eq!(c.delay(10.0, 0.1, &v, 1.0, 1.0), 11.0);
    assert!(!c.runs_ai(&v));
    v.fuser1 = 0.1;
    assert!(!c.runs_ai(&v));
    v.fuser1 = 0.0;
    assert!(!c.runs_ai(&v));
    v.fuser1 = 0.5;
    assert!(c.runs_ai(&v));
}

#[test]
fn buttons_are_per_bit_timers() {
    let c: &dyn NativeBotRules = &BOTS;
    let mut b = Brain::new(1.0);
    c.press(&mut b, btn::ATTACK | btn::DUCK, 0.5, 10.0);
    c.update_buttons(&mut b, 10.0);
    assert_eq!(b.buttons, btn::ATTACK | btn::DUCK);
    c.release(&mut b, btn::DUCK, 10.2);
    c.update_buttons(&mut b, 10.2);
    assert_eq!(b.buttons, btn::ATTACK);
    // Held while `time < expire`: not at the expiry itself.
    c.update_buttons(&mut b, 10.5);
    assert_eq!(b.buttons, 0);
}
