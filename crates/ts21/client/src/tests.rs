//! TS 2.1's client hooks against the 2.1 binary's numbers, and the core plugin's (TS 3.0's)
//! answers to the same questions, which must not change.

use super::*;
use ts_client::menus::buy::{self, BuyMenu, CASH, ITEM_KEVLAR, ITEM_STEALTH, Rng, Rules, page};

fn v21() -> &'static dyn ClientRules {
    PLUGIN.client
}

fn v30() -> &'static dyn ClientRules {
    ts_client::gameplay::CORE_PLUGIN.client
}

#[test]
fn crosshair_parts() {
    let shown = CrosshairParts { crosshair: true, divesteps: true, divesteps_last: true };
    let hidden = CrosshairParts { crosshair: false, divesteps: false, divesteps_last: true };
    assert_eq!(v21().crosshair_parts(13, 90.0), shown, "AK: crosshair, divesteps on top");
    for w in [0, 24, 25, 34, 35] {
        assert_eq!(v21().crosshair_parts(w, 90.0), hidden, "weapon {w}: nothing, not even divesteps");
    }
    assert_eq!(v21().crosshair_parts(18, 90.0), hidden, "unscoped Barrett");
    assert_eq!(v21().crosshair_parts(18, 40.0), hidden, "40 isn't zoomed");
    assert_eq!(v21().crosshair_parts(18, 39.9), shown, "m_iFOV is an int");
    assert_eq!(v21().crosshair_parts(18, 20.0), shown);

    // TS 3.0: divesteps first for every weapon; only the crosshair goes for melee and the M61.
    let both = CrosshairParts { crosshair: true, divesteps: true, divesteps_last: false };
    assert_eq!(v30().crosshair_parts(18, 90.0), both, "3.0 has a crosshair on the unscoped Barrett");
    assert_eq!(v30().crosshair_parts(25, 90.0), CrosshairParts { crosshair: false, ..both });
}

#[test]
fn crosshair_gap() {
    // M4A1 (2.1 accuracy 0.0075), predicted spread 0.2: the float-rounded sum and `/ 0.1f × 16`
    // land a pixel short of 3.0's 22 + 32.
    assert_eq!(v21().crosshair_gap(22, 0.2, 0.0075), 53);
    assert_eq!(v30().crosshair_gap(22, 0.2, 0.0075), 54);
    assert_eq!(v21().crosshair_gap(22, 0.1, 0.0), 38);
    assert_eq!(v21().crosshair_gap(12, 0.0, 0.032), 12, "no spread: the base (laser on)");
    for s in [f32::NAN, f32::INFINITY, f32::MAX, -f32::MAX] {
        assert!(v21().crosshair_gap(22, s, 0.01).abs() <= 0x10000);
    }
}

#[test]
fn divestep_limits() {
    // 2.1: top ≤ 21 used (and no kevlar in team modes), centre ≤ 41, bottom ≤ 71.
    assert_eq!(v21().divesteps_blocked(21, 0, 0), [false; 3]);
    assert_eq!(v21().divesteps_blocked(22, 0, 0), [true, false, false]);
    assert_eq!(v21().divesteps_blocked(41, 0, 0), [true, false, false]);
    assert_eq!(v21().divesteps_blocked(42, 0, 0), [true, true, false]);
    assert_eq!(v21().divesteps_blocked(71, 0, 0), [true, true, false]);
    assert_eq!(v21().divesteps_blocked(72, 0, 0), [true; 3]);
    assert_eq!(v21().divesteps_blocked(10, 50, 1), [false; 3], "kevlar counts only when m_Teamplay > 1");
    assert_eq!(v21().divesteps_blocked(10, 50, 2), [true, false, false]);
    assert_eq!(v21().divesteps_blocked(-174, 0, 0), [false; 3], "signed: 255 free slots");
    // 3.0: 42 / 52 / 62, no kevlar rule.
    assert_eq!(v30().divesteps_blocked(41, 100, 3), [false; 3]);
    assert_eq!(v30().divesteps_blocked(42, 0, 0), [true, false, false]);
    assert_eq!(v30().divesteps_blocked(52, 0, 0), [true, true, false]);
    assert_eq!(v30().divesteps_blocked(62, 0, 0), [true; 3]);
}

#[test]
fn stunt_cooldown() {
    let c = StuntCooldown { remaining: 0.0, received_at: 10.0, tenths: 5 };
    assert!(v21().stunt_cooldown(c, 10.4));
    assert!(v21().stunt_cooldown(c, 10.5), "red until the expiry has passed");
    assert!(!v21().stunt_cooldown(c, 10.51));
    let reset = StuntCooldown::default();
    assert!(v21().stunt_cooldown(reset, 0.0), "expiry 0 at time 0");
    assert!(!v21().stunt_cooldown(reset, 0.01));
    assert!(v21().stunt_cooldown(reset, f32::NAN), "unordered compare: red");
    // 3.0 runs the countdown down instead.
    assert!(v30().stunt_cooldown(StuntCooldown { remaining: 0.01, ..c }, 99.0));
    assert!(!v30().stunt_cooldown(StuntCooldown { remaining: 0.009, ..c }, 10.0));
}

#[test]
fn fov() {
    assert_eq!(v21().pinned_default_fov(), Some(80.0));
    assert_eq!(v30().pinned_default_fov(), None);
    for (cur, snaps) in [(-1, true), (0, true), (1, false), (20, false), (80, false), (81, true), (110, true)] {
        assert_eq!(v21().fov_snaps(cur), snaps, "2.1, m_iFOV {cur}");
    }
    for (cur, snaps) in [(-1, true), (0, true), (1, false), (80, false), (110, false)] {
        assert_eq!(v30().fov_snaps(cur), snaps, "3.0, m_iFOV {cur}");
    }
}

#[test]
fn input() {
    assert!(!v21().input_gate() && !v21().attack_switch() && !v21().attack_sends_respawn());
    assert!(v30().input_gate() && v30().attack_switch() && v30().attack_sends_respawn());
}

#[test]
fn map_effects_and_icon() {
    assert!(!v21().map_effects() && v30().map_effects());
    assert!(v21().slowmo_icon() && !v30().slowmo_icon());
    // No `cl_slowfade` tint in 2.1 (issue #152).
    assert!(!v21().slowmo_tint() && v30().slowmo_tint());
}

#[test]
fn fire_pitch_is_2_1s_helper() {
    // 0x100738a0: 100 + (s == 0 ? 300 : max(s * 300, 150)), s = TSSlowMo / 100, raw.
    assert_eq!(v21().fire_pitch(1.0, 0.0), 400, "normal speed");
    assert_eq!(v21().fire_pitch(0.35, 35.0), 250, "105 <= 150: the floor");
    assert_eq!(v21().fire_pitch(0.5, 50.0), 250, "exactly 150");
    assert_eq!(v21().fire_pitch(0.8, 80.0), 340);
    assert_eq!(v21().fire_pitch(0.01, 1.0), 250, "never below 250");
    assert_eq!(fire_pitch(f32::NAN), 250, "unordered compare: the floor");
    // 3.0 unchanged: 100 - (int)(ts * -300) of the eased, clamped scale.
    assert_eq!(v30().fire_pitch(1.0, 0.0), 400);
    assert_eq!(v30().fire_pitch(0.35, 35.0), 205);
    assert_eq!(v30().fire_pitch(0.01, 1.0), 103);
}

#[test]
fn kungfu_bar() {
    assert!(!v21().kungfu_exhausted(0.0));
    assert!(v30().kungfu_exhausted(0.0) && v30().kungfu_exhausted(0.01) && !v30().kungfu_exhausted(0.02));
    assert_eq!(v21().kungfu_power_on_reset(), Some(100.0));
    assert_eq!(v30().kungfu_power_on_reset(), None);

    let r = KungfuRegen { power: 50.0, step: 0.5, now: 11.0, last: 10.0, slowmo_factor100: 0.0 };
    assert_eq!(v21().kungfu_regen(r), 65.0, "15/s of real time at normal speed");
    assert_eq!(v21().kungfu_regen(KungfuRegen { slowmo_factor100: 35.0, ..r }), 55.25, "× 0.35 slowed");
    assert_eq!(v21().kungfu_regen(KungfuRegen { slowmo_factor100: 5.0, ..r }), 51.5, "floor 0.1");
    assert_eq!(v21().kungfu_regen(KungfuRegen { power: 99.0, ..r }), 100.0);
    assert_eq!(v21().kungfu_regen(KungfuRegen { power: 100.0, ..r }), 100.0);
    assert_eq!(v21().kungfu_regen(KungfuRegen { power: -5.0, ..r }), 10.0, "no 1/s branch below 0");
    assert!(v21().kungfu_regen(KungfuRegen { power: f32::NAN, ..r }).is_nan(), "NaN isn't below 100");
    // 3.0: 15 per step (frame time × time scale), 1 per step while negative.
    assert_eq!(v30().kungfu_regen(r), 57.5);
    assert_eq!(v30().kungfu_regen(KungfuRegen { power: -5.0, ..r }), -4.5);
}

/// Picks from a fixed list of `long` results.
struct Seq(Vec<i32>);

impl Rng for Seq {
    fn long(&mut self, lo: i32, hi: i32) -> i32 {
        if self.0.is_empty() { lo } else { self.0.remove(0).clamp(lo, hi) }
    }
    fn float(&mut self, lo: f32, _hi: f32) -> f32 {
        lo
    }
}

fn rules(ruleset: &dyn ClientRules, gamemode: i32) -> Rules {
    Rules {
        gamemode,
        restriction: 0x1f,
        cash: buy::cash(ruleset.buy_cash_based(gamemode, 0), 1234),
        menu: ruleset.buy_menu(),
        ..Rules::default()
    }
}

#[test]
fn buy_cash() {
    assert!(!v21().buy_cash_based(0, 0), "DM without usecash: 30000");
    assert!(v21().buy_cash_based(0, 1), "usecash (SrvSett 1)");
    assert!(v21().buy_cash_based(2, 0), "round modes");
    assert!(!v30().buy_cash_based(3, 1), "3.0's CashBased() is always off");
    assert_eq!(buy::cash(true, 4000), 4000);
    assert_eq!(buy::cash(false, 4000), CASH);
}

#[test]
fn buy_menu_items() {
    let r = rules(v21(), 0);
    let mut m = BuyMenu::default();
    // Kevlar takes no slots, and the slot check doesn't apply to items.
    m.current.slots = 81;
    assert!(m.add(ITEM_KEVLAR, 0, false, true, &r));
    assert_eq!((m.current.slots, m.current.total), (81, 1900));
    m.remove(0, &r);
    assert_eq!(m.current.slots, 81);
    assert_eq!(buy::slots_for(ITEM_KEVLAR, false, true, &r), 0);
    // Stealth shoes aren't restricted...
    assert!(!buy::restricted(ITEM_STEALTH, true, &r));
    // ...but the items page only opens when m_Teamplay > 1.
    m.page = page::CATEGORY;
    m.handle_key(page::CATEGORY, 6, &r, &mut Seq(vec![]));
    assert_eq!(m.page, page::CATEGORY);
    m.handle_key(page::CATEGORY, 6, &rules(v21(), 2), &mut Seq(vec![]));
    assert_eq!(m.page, page::ITEMS);

    // TS 3.0: kevlar is 10 slots, stealth shoes restricted outside team modes, page always open.
    let r = rules(v30(), 0);
    assert_eq!(buy::slots_for(ITEM_KEVLAR, false, true, &r), 10);
    assert!(buy::restricted(ITEM_STEALTH, true, &r));
    let mut m = BuyMenu { page: page::CATEGORY, ..BuyMenu::default() };
    m.handle_key(page::CATEGORY, 6, &r, &mut Seq(vec![]));
    assert_eq!(m.page, page::ITEMS);
}

#[test]
fn buy_menu_akimbo_keys_and_lists() {
    // No akimbo fix-up: a Desert Eagle (no akimbo version) bought with key 6 is "akimbo".
    let r = rules(v21(), 0);
    let mut m = BuyMenu::default();
    assert!(m.add(12, 0, true, false, &r));
    let e = m.current.entries[0];
    assert!(e.akimbo);
    assert_eq!(e.cost, buy::cost(12, 0, true, false));
    let r30 = rules(v30(), 0);
    let mut m = BuyMenu::default();
    assert!(m.add(12, 0, true, false, &r30));
    assert!(!m.current.entries[0].akimbo, "3.0 buys it single");

    // Keys 1..10 only (no `a` row).
    let mut m = BuyMenu { page: page::WEAPONS, ..BuyMenu::default() };
    m.handle_key(page::WEAPONS, 11, &r, &mut Seq(vec![]));
    assert_eq!(m.page, page::WEAPONS);

    // The handguns: no 2 and no 36, and 8 in their place.
    let col: Vec<i32> = buy::table(&r.menu).iter().map(|row| row[0]).filter(|&id| id != -1).collect();
    assert_eq!(col, [1, 9, 12, 14, 8, 21, 22, 28, 31]);
    let col30: Vec<i32> = buy::table(&r30.menu).iter().map(|row| row[0]).filter(|&id| id != -1).collect();
    assert_eq!(col30, [1, 9, 12, 14, 2, 21, 22, 28, 31, 36]);
}

#[test]
fn buy_menu_reset_and_random() {
    // No `cl_buykevlar`: a reset config is empty even with the cvar on.
    let r = Rules { buy_kevlar: true, cash: CASH, ..rules(v21(), 2) };
    let mut m = BuyMenu::default();
    m.reset_config(&r);
    assert!(m.current.entries.iter().all(|e| e.id == 0));
    let r30 = Rules { buy_kevlar: true, cash: CASH, ..rules(v30(), 2) };
    m.reset_config(&r30);
    assert_eq!(m.current.entries[0].id, ITEM_KEVLAR);

    // Random config, p = 0.5 (a pistol and an SMG): melee rand(0,5) = 0 → combat knife, its
    // rand(0,2) = 1 → single; pistol rand(1,9) = 5 → the akimbo Berettas (8); SMG rand(1,6) = 2
    // → MP5SD (6); then attachment rolls.
    let mut m = BuyMenu::default();
    m.random_config(0.5, &r, &mut Seq(vec![0, 1, 5, 0, 0, 0, 2, 0, 0, 0]));
    let ids: Vec<i32> = m.current.entries.iter().map(|e| e.id).filter(|&id| id != 0).collect();
    assert_eq!(ids, [25, 8, 6]);
    // The same rolls in 3.0: rand(0,9) = 5 is the single Beretta (2), and kevlar comes first.
    let mut m = BuyMenu::default();
    m.random_config(0.5, &r30, &mut Seq(vec![0, 1, 5, 0, 0, 0, 0, 2, 0, 0, 0]));
    let ids: Vec<i32> = m.current.entries.iter().map(|e| e.id).filter(|&id| id != 0).collect();
    assert_eq!(ids[..2], [ITEM_KEVLAR, 25]);
    assert_eq!(ids[2], 2);
}

fn bob_input(time: f32, speed: f32, slowmo_factor100: f32) -> ts_client::view::BobInput {
    ts_client::view::BobInput {
        on_ground: true,
        time,
        frametime: 0.01,
        viewheight: 28.0,
        simvel: [speed, 0.0, 0.0],
        ts: 1.0,
        cl_bob: 4.0,
        slowmo_factor100,
        forward: [1.0, 0.0, 0.0],
        right: [0.0, -1.0, 0.0],
        up: [0.0, 0.0, 1.0],
        ..Default::default()
    }
}

fn near(a: f32, b: f64) -> bool {
    (a as f64 - b).abs() < 1e-5
}

/// 2.1's `V_CalcBob` (0x100b5c80): the phase rate ignores the speed, there are no `cl_bob` levels
/// or bob pitch, and it fills the camera bob that `V_CalcNormalRefdef` adds to `vieworg`.
#[test]
fn view_bob() {
    let mut b = ts_client::view::Bob::default();
    let amp = v21().view_bob(&mut b, &bob_input(1.0, 200.0, 0.0));
    assert_eq!((b.phase, amp, b.peak, b.pitch), (3.5, 2.0, 2.0, 0.0), "ft × 3.5 × 100; |v| × 0.01");
    let (s2, s1) = ((7.0f64.to_radians()).sin() * 2.0, (3.5f64.to_radians()).sin() * 2.0);
    let eye = b.eye.expect("2.1 has a camera bob");
    assert!(eye[0] == 0.0 && near(eye[1], s1 * 0.9) && near(eye[2], s2 * -0.9 * 1.6), "{eye:?}");
    assert!(near(b.ofs[0], -0.25) && near(b.ofs[1], -s1 * 0.3), "{:?}", b.ofs);
    // z: `s2·0.11 + 0.5 − 0.65·amp`, smoothed from 0 by `d × 0.2 × ft × 100`.
    assert!(near(b.ofs[2], (s2 * 0.11 + 0.5 - 1.3) * 0.2), "{:?}", b.ofs);
    let mut lvl0 = ts_client::view::Bob::default();
    v21().view_bob(&mut lvl0, &ts_client::view::BobInput { cl_bob: 0.0, ..bob_input(1.0, 200.0, 0.0) });
    assert_eq!(lvl0, b, "no cl_bob levels");

    // Half speed slow motion: half the phase step.
    v21().view_bob(&mut b, &bob_input(1.01, 200.0, 50.0));
    assert_eq!(b.phase, 5.25);
    let eye = b.eye;
    // In the air the amplitude halves and the camera bob keeps its last value.
    let air = ts_client::view::BobInput { on_ground: false, ..bob_input(1.02, 200.0, 0.0) };
    v21().view_bob(&mut b, &air);
    assert_eq!((b.amp, b.eye), (1.0, eye), "the camera bob is stale, not cleared");
    // Back on the ground below the peak: `(|v| × 0.01 + amp) × 0.5`.
    assert_eq!(v21().view_bob(&mut b, &bob_input(1.03, 200.0, 0.0)), 1.5);
    assert_eq!(b.peak, 2.0);

    // TS 3.0's bob: speed-scaled phase, a bob pitch, no camera bob.
    let mut c = ts_client::view::Bob::default();
    v30().view_bob(&mut c, &bob_input(1.0, 200.0, 0.0));
    assert_eq!(c.eye, None);
    assert!(c.phase != 3.5 && c.pitch != 0.0, "{c:?}");
    let mut d = ts_client::view::Bob::default();
    d.calc(&bob_input(1.0, 200.0, 0.0));
    assert_eq!(c, d, "the core plugin runs Bob::calc");
}

/// 2.1's client bullets (issue #165): `40 × v × s` with the raw factor (÷5 in bullet time), one
/// fixed step every 0.1 s, drawn while a factor is set; 3.0's curve, stepping and gates stay.
#[test]
fn client_bullet_speed_and_stepping() {
    use ts_client::events::BulletStep;
    let x = [1.0, 0.0, 0.0];
    let at = |factor100: f32, bullet_time: bool, dt: f32, ts: f32| BulletStep {
        velocity: 330.0,
        time_scale: ts,
        bullet_time,
        slowmo_factor100: factor100,
        dt,
    };
    // A fixed step, whatever the time since the last.
    assert_eq!(v21().client_bullet_step(x, &at(0.0, false, 0.05, 1.0)), [1320.0, 0.0, 0.0]);
    assert_eq!(v21().client_bullet_step(x, &at(30.0, false, 0.1, 0.3))[0], 1320.0 * (30.0f32 / 100.0));
    assert!((v21().client_bullet_step(x, &at(30.0, true, 0.1, 0.3))[0] - 79.2).abs() < 1e-4, "bullet time ÷5");
    // 3.0: velocity × 40 × dt × GetBulletVelocityMult on the eased scale.
    assert_eq!(v30().client_bullet_step(x, &at(0.0, false, 0.1, 1.0))[0], 1320.0);
    assert!((v30().client_bullet_step(x, &at(30.0, false, 0.1, 0.3))[0] - 396.0).abs() < 1e-3);
    assert!((v30().client_bullet_step(x, &at(30.0, true, 0.1, 0.3))[0] - 1320.0 * 0.3 / 9.1).abs() < 1e-3);

    assert_eq!(v21().client_bullet_next_update(10.0, 1), (10.0f64 + 0.1) as f32, "0.1 s, realbullet or not");
    assert_eq!(v21().client_bullet_next_update(10.0, 5), (10.0f64 + 0.1) as f32);
    assert_eq!(v30().client_bullet_next_update(10.0, 1), 0.05 + 10.0);
    assert_eq!(v30().client_bullet_next_update(10.0, 5), 10.0, "3.0: every frame");

    // Queueing: `realbullet == 1 || TSSlowMo != 0` (3.0: `realbullet != 0 || ts < 0.99`).
    assert!(v21().client_bullets_queue(1, 1.0, 0.0) && v30().client_bullets_queue(1, 1.0, 0.0));
    assert!(!v21().client_bullets_queue(2, 1.0, 0.0) && v30().client_bullets_queue(2, 1.0, 0.0));
    assert!(!v21().client_bullets_queue(0, 0.5, 0.0) && v30().client_bullets_queue(0, 0.5, 0.0), "easing back");
    assert!(v21().client_bullets_queue(0, 1.0, 30.0) && !v30().client_bullets_queue(0, 1.0, 30.0));
    assert!(v21().client_bullet_draws(1.0, 30.0) && !v21().client_bullet_draws(0.5, 0.0));
    assert!(v30().client_bullet_draws(0.5, 0.0) && !v30().client_bullet_draws(1.0, 30.0));

    // Range falloff: the quotient goes through a float.
    assert_eq!(v21().client_bullet_range_index(800.0, 80.0, false), 10);
    assert_eq!(v21().client_bullet_range_index(1234.5, 90.0, false), 13);
    assert_eq!(v21().client_bullet_range_index(330.0, 80.0, true), 11);
    assert_eq!(v30().client_bullet_range_index(1234.5, 90.0, false), 13);
    assert_eq!(v30().client_bullet_range_index(330.0, 80.0, true), 11);
}

/// 2.1's draw (0x10069540): the bullet models and the `cl_wavedetail` trails.
#[test]
fn client_bullet_looks() {
    use crate::bullets::*;
    assert_eq!(
        [model_for(3), model_for(4), model_for(6), model_for(8), model_for(1)],
        ["models/pellet.mdl", "models/long_bullet.mdl", "models/long_bullet.mdl", "models/long_bullet.mdl", "models/bullet.mdl"]
    );
    assert_eq!([lod_body(50.0, 90), lod_body(100.0, 90), lod_body(200.0, 90), lod_body(300.0, 90)], [3, 2, 1, 0]);
    assert_eq!([lod_body(1000.0, 10), lod_body(400.0, 40), lod_body(400.0, 41)], [3, 2, 0], "scoped: more detail further out");

    assert_eq!(trails(0.0, true), Trails::None);
    assert_eq!(trails(1.0, true), Trails::Trail);
    assert_eq!(trails(2.0, false), Trails::Trail, "the wave needs bullet time");
    assert_eq!(trails(2.0, true), Trails::TrailAndWave);
    assert_eq!(trails(3.0, true), Trails::Spiral);
    assert_eq!(trails(f32::NAN, true), Trails::Spiral, "unordered compares count as greater");

    assert_eq!(
        [spin_step(0.0), spin_step(30.0), spin_step(10.0), spin_step(5.0)],
        [90.0, ((30.0f32 / 100.0) as f64 * 90.0) as f32, 22.5, 22.5]
    );
    assert_eq!(draw_step([1.0, 0.0, 0.0], 330.0, 0.3, 0.01)[0], 330.0f32 / 0.25 * 0.3 * 0.01 * 10.0);

    let up = [0.0, 0.0, 1.0];
    let t = trail_particle(7, [10.0, 0.0, 0.0], [0.0, 0.0, 0.0], 0.5, 0.5, up);
    assert_eq!((t.color, t.fadespeed, t.radius, t.life), (0.5, 0.02, 0.5, 0.0), "fixed width, not 3.0's 1 − ts²");
    assert_eq!(t.corners, Some([[0.0, 0.0, -0.5], [0.0, 0.0, 0.5], [10.0, 0.0, 0.5], [10.0, 0.0, -0.5]]));
    let w = wave_particle(7, [1.0, 2.0, 3.0], 1.5);
    assert_eq!((w.color, w.fadespeed, w.radius, w.corners), (0.6, 0.035, 1.5, None));

    let s3 = spiral_particles(7, [0.0; 3], [10.0, 0.0, 0.0], [0.0; 3], 1.0, 2.0, 3.0);
    assert_eq!(s3.len(), 3);
    assert_eq!(spiral_particles(7, [0.0; 3], [10.0, 0.0, 0.0], [0.0; 3], 1.0, 2.0, 3.5).len(), 5);
    let p = &s3[0];
    assert_eq!((p.color, p.fadespeed, p.radius), (0.7, 0.035, 1.0));
    let c = p.corners.unwrap();
    // Facing +x: right is −y, so t = 0 starts a radius out on −y.
    assert_eq!(c[0], [0.0, -1.0, 0.0]);
    assert_eq!(c[3], [10.0, -2.0, 0.0]);
    assert_eq!(p.uv.unwrap()[1][0], 1.0 / 3.0);
    assert_eq!(s3[2].uv.unwrap()[1][0], 1.0);
}

/// 2.1's weapon events (issue #160): builder parameters, reaches, the block branch, pitches and
/// attenuations; the core plugin keeps 3.0's.
#[test]
fn weapon_events() {
    use crate::events::*;
    use ts_client::effects::fx::FxStyle;
    use ts_client::gameplay::MeleeReach as M;
    assert_eq!(v30().fx_style(), &FxStyle::TS30);
    let fx = v21().fx_style();
    assert_eq!((fx.flash_color, fx.flash_fade, fx.flash_scale_speed, fx.third_person_flash), (1.0, 0.15, 0.1, 1.0));
    assert!(!fx.muzzle_blast && fx.muzzle_spark && FxStyle::TS30.muzzle_blast && !FxStyle::TS30.muzzle_spark);
    assert_eq!((fx.silenced_count, fx.silenced_color, fx.silenced_fade, fx.silenced_spin), (1.0, 1.0 / 640.0, Some(0.13), 60));
    assert_eq!((FxStyle::TS30.silenced_count, FxStyle::TS30.silenced_color), (0.7, 0.3 / 512.0));
    assert_eq!((fx.smoke_gravity, fx.drag, fx.debris_react), (0.1, 0.0, 0.0));
    assert_eq!((FxStyle::TS30.smoke_gravity, FxStyle::TS30.drag, FxStyle::TS30.debris_react), (0.01, 3.3, 0.3));
    assert!(fx.smoke_puffs_by_cl_fx && !fx.stretch && fx.debris_fade_any_mode && !fx.debris_count_by_quality);

    // Reaches: close combat -56/48/38, the coldcock by its weapon id the same way, knife 38,
    // katana -54/48 (3.0: -55/45/35, 30, 35, -55/50).
    let reach = |r: &dyn ClientRules| {
        [
            M::CloseCombat(3),
            M::CloseCombat(2),
            M::CloseCombat(1),
            M::Coldcock(3),
            M::Coldcock(2),
            M::Coldcock(9),
            M::Knife,
            M::Katana(15),
            M::Katana(16),
        ]
        .map(|m| r.melee_reach(m))
    };
    assert_eq!(reach(v21()), [-56.0, 48.0, 38.0, -56.0, 48.0, 38.0, 38.0, -54.0, 48.0]);
    assert_eq!(reach(v30()), [-55.0, 45.0, 35.0, 30.0, 30.0, 30.0, 35.0, -55.0, 50.0]);

    assert!(!v21().close_combat_blocks() && v30().close_combat_blocks(), "2.1's event has no block branch");
    assert!(!v21().reload_event_phases() && v30().reload_event_phases());
    assert!(!v21().bullet_whizz_and_push() && v30().bullet_whizz_and_push());
    assert_eq!((v21().pellet_spray_gravity(), v30().pellet_spray_gravity()), (2.0, 0.0));
    assert_eq!((v21().shell_sound(11), v30().shell_sound(11), v30().shell_sound(5)), (1, 2, 1));

    // The .454 Casull is 2.1's ammo 12 (0.3); 3.0's 12 is the .32 ACP (0.5).
    assert_eq!((v21().ammo_attenuation(12), v30().ammo_attenuation(12)), (0.3, 0.5));
    assert_eq!((v21().ammo_attenuation(6), v30().ammo_attenuation(6)), (0.2, 0.2));
    assert_eq!((v21().ammo_attenuation(40), v30().ammo_attenuation(40)), (0.4, 0.4));

    // The ricochet: `100 + 300 s` on the raw factor (400 at normal speed).
    assert_eq!([ricochet_pitch(0.0), ricochet_pitch(50.0), ricochet_pitch(10.0)], [400, 250, 130]);

    assert_eq!(b"CMTPYVW".map(material_volume), [0.9, 0.9, 0.8, 0.8, 0.8, 0.5, 0.9].map(Some));
    assert_eq!(material_volume(b'F'), None, "flesh plays nothing");
}
