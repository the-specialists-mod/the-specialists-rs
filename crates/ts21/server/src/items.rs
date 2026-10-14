//! TS 2.1's healing and armour items (2.1 `mp.dll`): the HL SDK's `TakeHealth`, battery and wall
//! chargers, where TS 3.0 has its own (`ts_server::entities::stock::items`), and `item_longjump`,
//! HL's long-jump item where 3.0 has a super-jump power-up.

use goldsrc::edict::EdictPtr;
use goldsrc::server::engine;
use goldsrc::sys::{edict_t, entvars_t};
use std::ffi::CStr;
use ts_server::entities::stock::items::{RulesetItem, emit_sound_suit, ruleset_item};
use ts_server::messages::{self, Msg};
use ts_server::world::{Cx, Entity};

/// `CTSSuperJump`'s model (`Spawn` 0x10021910, `Precache` 0x10021940).
const LONG_JUMP_MODEL: &CStr = c"models/w_longjump.mdl";

/// `item_longjump` in 2.1 (`CTSSuperJump`, vtable 0x10083e00): HL's long-jump `CItem`, not a
/// power-up. `Spawn` precaches and sets `w_longjump.mdl`, then `CItem::Spawn` (0x10021170); the
/// touch, respawn and materialise are `CItem`'s (0x10021250, 0x100212f0, 0x10021360). The
/// classname stays `item_longjump`, so 2.1's automatic power-up spawner stays on.
pub static LONG_JUMP: RulesetItem = RulesetItem { model: LONG_JUMP_MODEL, precache: precache_long_jump, my_touch: long_jump_touch };

/// The game object 2.1 makes for `class`: [`LONG_JUMP`] for `item_longjump`.
pub fn entity_object(class: &CStr) -> Option<Box<dyn Entity>> {
    (class == c"item_longjump").then(|| ruleset_item(&LONG_JUMP))
}

/// The map-start precache (`ServerRules::precache`): the long jump's model, which 2.1's
/// `Spawn` precaches. The model is Half-Life's
/// (`valve`), so it is skipped where the file system has no such file.
pub fn precache() {
    if engine().file_size(LONG_JUMP_MODEL).is_some_and(|n| n > 0) {
        precache_long_jump();
    }
}

fn precache_long_jump() {
    engine().precache_model(LONG_JUMP_MODEL);
}

/// `CTSSuperJump::MyTouch` (0x10021950, vtable slot 60; Ghidra names it
/// `CBaseAnimating::HandleAnimEvent`), the SDK's `CItemLongJump::MyTouch`: a player without the
/// long jump (`m_fLongJump`, `pl+0x92c`) and with the HEV suit bit gets it: physinfo `slj` "1"
/// (2.1's movement doesn't read it), `ItemPickup` with the classname, and `!HEV_A1`. `Spawn`
/// takes it away again (0x10052a73).
fn long_jump_touch(cx: &mut Cx, p: *mut edict_t) -> bool {
    if p.v().weapons >= 0 {
        return false;
    }
    let took = ts_server::player::with_player(p, |pl, _| !std::mem::replace(&mut pl.long_jump, true));
    if took != Some(true) {
        return false;
    }
    let e = engine();
    e.set_physics_key(p, c"slj", c"1");
    let class = e.string(cx.pev().classname);
    messages::one(Msg::ItemPickup, p).string(class).end();
    emit_sound_suit(p, c"!HEV_A1");
    true
}

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

    /// 2.1 makes `item_longjump` an HL item (`CTSSuperJump`, a `CItem`); 3.0 keeps its linked
    /// power-up, and no other class changes.
    #[test]
    fn item_longjump_is_an_hl_item() {
        let v21: &dyn ServerRules = &crate::Ts21Server;
        let core: &dyn ServerRules = &shared::gameplay::CORE;
        let e = v21.entity_object(c"item_longjump").expect("2.1's own object");
        let any: &dyn std::any::Any = &*e;
        assert!(ts_server::entities::stock::items::is_item(any.type_id()));
        assert_eq!(LONG_JUMP.model, c"models/w_longjump.mdl");
        assert!(core.entity_object(c"item_longjump").is_none());
        for class in [c"ts_powerup", c"item_battery", c"item_healthkit"] {
            assert!(v21.entity_object(class).is_none(), "{class:?}");
        }
    }

    #[test]
    fn charger_capacities() {
        let v21: &dyn ServerRules = &crate::Ts21Server;
        let core: &dyn ServerRules = &shared::gameplay::CORE;
        assert_eq!((v21.charger_capacity(true), v21.charger_capacity(false)), (30, 0));
        assert_eq!((core.charger_capacity(true), core.charger_capacity(false)), (1, 1));
    }
}
