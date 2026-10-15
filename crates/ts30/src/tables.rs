//! TS 3.0's weapon, ammo, asset and combat tables (`data/*.toml`, decoded from the decomp): the
//! `GunRules::tables` answer, and the default tables while 3.0 is the default ruleset.

use shared::weapons::table::{self, Tables};
use std::sync::OnceLock;

/// The embedded TS 3.0 tables, parsed once; panics on a malformed embedded table (caught by
/// `cargo test`).
pub fn tables() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(|| parse().expect("embedded weapon tables"))
}

/// Makes these the default tables ([`table::set_default_tables`]), as registering the 3.0 plugin
/// does: for tests that read the weapon tables without registering a ruleset.
pub fn make_default() {
    table::set_default_tables(tables());
}

fn parse() -> Result<Tables, toml::de::Error> {
    table::parse(
        include_str!("../data/weapons.toml"),
        include_str!("../data/ammo.toml"),
        include_str!("../data/assets.toml"),
        include_str!("../data/combat.toml"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::weapons::table::FireMode;

    #[test]
    fn tables_parse() {
        let t = parse().unwrap();
        assert_eq!(t.weapons().count(), 37);
        let ak = t.weapon(13).unwrap();
        assert_eq!(ak.name, "AK47");
        assert_eq!(ak.clip, 30);
        assert_eq!(ak.modes, vec![FireMode::Auto, FireMode::Semi]);
        assert!(t.weapon(29).is_none(), "29 is the briefcase");
        assert_eq!(t.ammo(4).unwrap().name, "5.56 Nato");
        assert_eq!(t.asset(13).unwrap().model, "ak47");
    }

    #[test]
    fn weapon_params_override() {
        let t = parse().unwrap();
        let c = t.with_weapon_params(13, 0.0, 1.0, 2.0, 50.0, 99).unwrap();
        let ak = c.weapon(13).unwrap();
        assert_eq!((ak.accuracy, ak.view_kick, ak.acc_kick, ak.damage, ak.cost), (0.001, 1.0, 2.0, 50.0, 99));
        assert!(t.with_weapon_params(29, 0.0, 0.0, 0.0, 0.0, 0).is_none());
    }
}
