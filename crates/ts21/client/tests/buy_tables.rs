//! Issue #111: the buy menu reads the active ruleset's weapon and ammo tables
//! (`shared::weapons::tables()`), so in 2.1 mode it has 2.1's prices, slots, attachment masks and
//! ammo limits, which are the same as 2.1 `client.dll`'s hard-coded tables (weapon info
//! `0x102a3310 + id x 0x3f4`, ammo `0x10299308 + t x 0x38`). Its own test binary: the tables are
//! process-wide, and the crate's unit tests use 3.0's.

use shared::gameplay::GunRules;
use shared::weapons::table;
use ts_client::menus::buy::{self, BuyMenu, CASH, Rules};

/// The money left after buying `id` once, with ammo bought round-robin up to the limit.
fn ammo_rounds(id: i32, ammocount: i32) -> (i32, i32) {
    let r = Rules { ammocount, restriction: 0x1f, cash: CASH, ..Rules::default() };
    let mut m = BuyMenu::default();
    assert!(m.add(id, 0, false, false, &r));
    m.recompute_ammo(&r);
    let t = shared::weapons::tables().weapon(id as u8).unwrap().ammo as usize;
    (t as i32, m.current.ammo[t].0)
}

#[test]
fn buy_menu_reads_the_rulesets_tables() {
    ts30_gameplay::tables::make_default();
    table::set_ruleset_tables(ts21_gameplay::TS21.tables());
    // Id 8, the akimbo-only Berettas: $1800 and 20 slots (2.1 client `0x1001b627`, `0x1001b610`).
    assert_eq!(buy::cost(8, 0, false, false), 1800);
    assert_eq!(buy::slots_for(8, false, false, &Rules::default()), 20);
    // Id 17 is the STEYR-TMP, with silencer and laser.
    assert_eq!((buy::cost(17, 0, false, false), buy::slots_for(17, false, false, &Rules::default())), (1600, 20));
    let tmp = BuyMenu { pending_weapon: 17, ..BuyMenu::default() };
    assert!(tmp.attachment_allowed(1) && tmp.attachment_allowed(2) && !tmp.attachment_allowed(8));
    // 2.1 has no id 2: nothing to price.
    assert_eq!(buy::cost(2, 0, false, false), 0);
    // `.45Acp` (the MK23's) carries 2.1's 200 in ammocount column 0 (3.0: 175).
    assert_eq!(ammo_rounds(9, 0), (2, 200));

    // TS 3.0's, unchanged.
    table::set_ruleset_tables(ts30_gameplay::tables::tables());
    assert_eq!((buy::cost(8, 0, false, false), buy::slots_for(8, false, false, &Rules::default())), (900, 10));
    assert_eq!((buy::cost(17, 0, false, false), buy::slots_for(17, false, false, &Rules::default())), (1100, 15));
    let skorpion = BuyMenu { pending_weapon: 17, ..BuyMenu::default() };
    assert!(!skorpion.attachment_allowed(1));
    assert!(buy::cost(2, 0, false, false) > 0);
    assert_eq!(ammo_rounds(9, 0), (2, 175));
}
