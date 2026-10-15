//! `shared`'s weapon lookups and the client-data packing, checked against TS 3.0's tables (the
//! core crate has no tables of its own: each ruleset brings them).

use goldsrc_sys::{clientdata_t, weapon_data_t};
use shared::protocol::weapons::{pack_clientdata, pack_weapon_data, unpack_clientdata};
use shared::weapons::*;

/// 3.0's tables as the default, as registering the 3.0 plugin makes them.
fn setup() {
    shared::weapons::table::set_default_tables(ts30_gameplay::tables::tables());
}

#[test]
fn modes_and_defaults() {
    setup();
    let ak = info(id::AK47).unwrap();
    assert_eq!(mode_mask(ak), 3);
    assert_eq!(default_mode(mode_mask(ak)), mode::AUTO);
    assert_eq!(default_mode(mode_mask(info(id::BENELLI).unwrap())), mode::PUMP);
    assert_eq!(default_mode(mode_mask(info(id::M16).unwrap())), mode::BURST);
    assert_eq!(default_mode(mode_mask(info(id::BERETTA).unwrap())), mode::SEMI);
    assert_eq!(default_mode(mode_mask(info(id::BERETTA_AKIMBO).unwrap())), mode::SEMI);
    assert_eq!(kind_of(id::GOLDEN_COLTS), kind::AKIMBO_ONLY);
    assert!(is_akimbo(id::UZI_AKIMBO));
}

#[test]
fn restriction() {
    setup();
    assert!(is_restricted(id::AK47, 0));
    assert!(!is_restricted(id::M61, 0x1f));
    assert!(!is_restricted(id::M61, 1));
    assert!(is_restricted(id::KATANA, 0x1e));
    assert!(!is_restricted(id::SAWEDOFF, 0x10));
    assert!(is_restricted(id::SAWEDOFF, 0x02));
    assert!(is_restricted(id::UZI_AKIMBO, 0x02));
    assert!(!is_restricted(id::BARRETT, 0x08));
}

#[test]
fn body_groups() {
    setup();
    // Contender without scope gets +1; M4 silencer+scope = 1 + 8.
    assert_eq!(body_by_items(id::CONTENDER, 0), 1);
    assert_eq!(body_by_items(id::M4, att::SILENCER | att::SCOPE), 9);
    // Glock laser+flashlight uses the combined entry.
    assert_eq!(body_by_items(id::GLOCK18, att::LASER | att::FLASHLIGHT), 2);
}

#[test]
fn anim_tables() {
    setup();
    assert_eq!(anim(id::AK47, false, 2), 2);
    assert!(has_alt_grip(id::AK47));
    assert!(!has_alt_grip(id::AUG));
    assert_eq!(anim(id::KUNGFU, false, 2), 1);
}

#[test]
fn clientdata_round_trip() {
    setup();
    let mut g = Gun::default();
    g.take_weapon(id::BERETTA, None, 0);
    g.take_weapon(id::BERETTA, None, 0);
    g.current = id::BERETTA_AKIMBO;
    g.reserve[1] = 77;
    g.status[id::BERETTA_AKIMBO as usize].toggle = true;
    let mut cd = clientdata_t::default();
    pack_clientdata(&g, 0x10, &mut cd);
    assert_eq!(cd.vuser3[0], 8.0);
    assert_eq!(cd.vuser4[1], 30.0);
    assert_eq!(cd.ammo_nails, 15);
    assert_eq!(cd.ammo_shells, 15, "the single record's clip");
    assert_eq!(cd.vuser2[1], 77.0);
    let mut g2 = Gun::default();
    assert_eq!(unpack_clientdata(&cd, &mut g2), 0x10);
    assert_eq!(g2.current, id::BERETTA_AKIMBO);
    assert_eq!(g2.status[8].clip, 30);
    assert!(g2.status[8].toggle);
    assert_eq!(g2.reserve[1], 77);
    assert_eq!((g2.status[8].left, g2.status[8].right), (15, 15));
    cd.vuser4[1] = 10.0;
    let mut g3 = Gun::default();
    unpack_clientdata(&cd, &mut g3);
    assert_eq!((g3.status[8].left, g3.status[8].right), (15, 0), "right clip clamped at 0");
    // Out-of-range networked values wrap instead of overflowing.
    (cd.vuser4[1], cd.ammo_nails) = (i32::MAX as f32, i32::MIN);
    unpack_clientdata(&cd, &mut g3);
    assert_eq!(g3.status[8].right, 0);
    let mut wd = weapon_data_t::default();
    g.next_primary = -0.5;
    pack_weapon_data(&g, &mut wd);
    assert_eq!(wd.m_iId, 16);
    assert_eq!(wd.m_flNextPrimaryAttack, -0.001);
}

/// `clientdata_t` comes from the server's delta: any id, NaN and huge values unpack without a
/// panic.
#[test]
fn unpack_clientdata_survives_any_values() {
    setup();
    let vals = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0, 0.0, 8.0, 37.0, 38.0, 255.0, 1e10, -1e10];
    let ints = [i32::MIN, -1, 0, 1, i32::MAX];
    for &a in &vals {
        for &n in &ints {
            let cd = clientdata_t {
                vuser1: [a; 3],
                vuser2: [a; 3],
                vuser3: [a; 3],
                vuser4: [a; 3],
                ammo_nails: n,
                ammo_shells: n,
                m_flNextAttack: a,
                ..Default::default()
            };
            let mut g = Gun::default();
            unpack_clientdata(&cd, &mut g);
            assert!(g.status.iter().all(|s| s.right >= 0));
        }
    }
    let mut cd = clientdata_t::default();
    cd.vuser3[0] = id::BERETTA as f32;
    cd.vuser4[1] = -3e9; // saturates to i32::MIN
    cd.ammo_nails = 1;
    let mut g = Gun::default();
    unpack_clientdata(&cd, &mut g);
    assert_eq!(g.status[id::BERETTA as usize].right, i32::MAX, "clip - left wraps as TS's int does");
}
