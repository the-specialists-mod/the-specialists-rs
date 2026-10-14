//! TS 2.1's healing and armour items (2.1 `mp.dll`): the HL SDK's `TakeHealth`, battery and wall
//! chargers, where TS 3.0 has its own (`ts_server::entities::stock::items`).

use goldsrc::edict::EdictPtr;
use goldsrc::sys::{edict_t, entvars_t};

/// `gSkillData.suitchargerCapacity` (0x100a2058): 2.1 reads no `sk_*` cvar (`RefreshSkillData`
/// 0x1001eb60 reads only `skill`), and `FUN_100239c0`, from `CHalfLifeMultiplay`'s constructor,
/// sets it to 30.0 (`0x41f00000`). `CRecharge::Spawn` (0x1001f620) and `Recharge` (0x1001f930)
/// `__ftol` it.
pub const HEV_CHARGER_CAPACITY: i32 = 30;

/// `gSkillData.healthchargerCapacity` (0x100a2060): nothing writes it (only `CWallHealth::Spawn`
/// 0x10020b8c and `Recharge` 0x10020dc7 read it), so it stays 0: every `func_healthcharger` is
/// empty from the start, and its `Recharge` empties it again.
pub const HEALTH_CHARGER_CAPACITY: i32 = 0;

/// `CBasePlayer::TakeHealth` (0x1002ab20) is `CBaseMonster::TakeHealth` (0x100115e0), then
/// `CBaseEntity::TakeHealth` (0x1000c230): refused with `takedamage == 0` or `health >=
/// max_health` (a NaN passes both), then capped at `max_health`. Never over the max, so no
/// overheal decay starts.
pub fn add_health(v: &mut entvars_t, amount: f32) -> Option<bool> {
    if v.takedamage == 0.0 || v.health >= v.max_health {
        return None;
    }
    v.health += amount;
    if v.max_health < v.health {
        v.health = v.max_health;
    }
    Some(v.health > v.max_health)
}

/// `CItemBattery::MyTouch` (0x10021600): `armorvalue += 50`, then `100 <= armorvalue ? 100 :
/// armorvalue`. No `TakeArmor`: no kevlar slots and no speed change.
pub fn battery_armor(p: *mut edict_t, amount: f32) {
    let v = p.v();
    v.armorvalue = cap_armor(v.armorvalue + amount);
}

fn cap_armor(a: f32) -> f32 {
    if 100.0 <= a { 100.0 } else { a }
}

/// `m_iJuice` from `Spawn` and `Recharge`.
pub fn charger_capacity(armor: bool) -> i32 {
    if armor { HEV_CHARGER_CAPACITY } else { HEALTH_CHARGER_CAPACITY }
}

/// One 0.1 s charge (the SDK's): `CRecharge::Use` (0x1001f6e0) below 100 armour takes one juice
/// and gives one armour (`> 100` set back to 100); `CWallHealth::Use` (0x10020be0) takes one juice
/// when `TakeHealth(1, DMG_GENERIC)` took.
pub fn charger_charge(armor: bool, juice: &mut i32, activator: *mut edict_t) {
    if armor {
        hev_charge(juice, &mut activator.v().armorvalue);
    } else if ts_server::entities::stock::items::player_take_health(activator, 1.0) {
        *juice -= 1;
    }
}

/// `CRecharge::Use`'s charge: below 100 armour, one juice for one armour, `> 100` back to 100.
fn hev_charge(juice: &mut i32, armor: &mut f32) {
    if *armor < 100.0 {
        *juice -= 1;
        *armor += 1.0;
        if 100.0 < *armor {
            *armor = 100.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ts_server::gameplay::ServerRules;

    fn ent(health: f32, max: f32) -> entvars_t {
        entvars_t { health, max_health: max, takedamage: 2.0, ..Default::default() }
    }

    /// The SDK's `TakeHealth`: refused at max, capped there; 3.0's adds past it (and starts the
    /// decay) and is refused only above max.
    #[test]
    fn take_health_caps_at_max() {
        let v21: &dyn ServerRules = &crate::Ts21Server;
        let core: &dyn ServerRules = &shared::gameplay::CORE;
        let mut v = ent(99.0, 100.0);
        assert_eq!(v21.player_add_health(&mut v, 50.0), Some(false));
        assert_eq!(v.health, 100.0, "a healthkit at 99 gives 100");
        assert_eq!(v21.player_add_health(&mut v, 50.0), None, "and is left at 100");
        let mut v = ent(99.0, 100.0);
        assert_eq!(core.player_add_health(&mut v, 50.0), Some(true));
        assert_eq!(v.health, 149.0, "3.0: uncapped");
        let mut v = ent(100.0, 100.0);
        assert_eq!(core.player_add_health(&mut v, 50.0), Some(true), "3.0 takes it at exactly max");
        let mut v = ent(50.0, 100.0);
        v.takedamage = 0.0;
        assert_eq!(v21.player_add_health(&mut v, 10.0), None);
        let mut v = ent(f32::NAN, 100.0);
        assert_eq!(v21.player_add_health(&mut v, 10.0).map(|_| ()), Some(()), "a NaN health passes the test");
    }

    #[test]
    fn battery_caps_at_100() {
        assert_eq!(cap_armor(80.0 + 50.0), 100.0);
        assert_eq!(cap_armor(30.0 + 50.0), 80.0);
        assert!(cap_armor(f32::NAN).is_nan(), "`100 <= NaN` is false: kept");
    }

    /// A full HEV charger gives 30 armour, one a charge (10 a second), and then is empty.
    #[test]
    fn hev_charger_trickles() {
        let (mut juice, mut armor) = (charger_capacity(true), 50.0);
        let mut charges = 0;
        while juice > 0 && armor < 100.0 {
            hev_charge(&mut juice, &mut armor);
            charges += 1;
        }
        assert_eq!((charges, juice, armor), (30, 0, 80.0));
        let (mut juice, mut armor) = (5, 99.5);
        hev_charge(&mut juice, &mut armor);
        assert_eq!((juice, armor), (4, 100.0));
        hev_charge(&mut juice, &mut armor);
        assert_eq!((juice, armor), (4, 100.0), "full: nothing taken");
    }

    #[test]
    fn charger_capacities() {
        let v21: &dyn ServerRules = &crate::Ts21Server;
        let core: &dyn ServerRules = &shared::gameplay::CORE;
        assert_eq!((v21.charger_capacity(true), v21.charger_capacity(false)), (30, 0));
        assert_eq!((core.charger_capacity(true), core.charger_capacity(false)), (1, 1));
    }
}
