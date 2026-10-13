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
