//! TS 3.0's inventory and item rules: what a pickup costs, `TakeHealth`, the battery's and the
//! wall chargers' armour and health.

use goldsrc::edict::EdictPtr;
use goldsrc::sys::{edict_t, entvars_t};
use shared::weapons::{Status, WeaponInfo, kind};
use ts_server::entities::stock::items::player_take_health;
use ts_server::rules::has_player;

/// The slots `PickUpWeapon` (0x9314c) charges for `id`: `None` for the akimbo pair of a single
/// gun (`kind != 4 || single == -1`: the pair only comes from two singles), else `slots`, times
/// the count for a dropped stack (rounded with the free slots by the caller).
pub fn pick_up_cost(_id: u8, w: &WeaponInfo, from: Option<&Status>) -> Option<f32> {
    if shared::weapons::ts_kind(w.kind) == kind::AKIMBO && w.single != -1 {
        return None;
    }
    let mut cost = w.slots as f32;
    if w.stack
        && let Some(s) = from
    {
        cost *= s.clip as f32;
    }
    Some(cost)
}

/// `CBasePlayer::TakeHealth` (0xd8760) on the entvars: refused with `takedamage` off or health
/// already above max; otherwise added with no cap. Returns `None` when refused, else whether the
/// player is now over max (TS then sets the 1/s decay timer, `+0x73c`, to time + 1).
pub fn add_health(v: &mut entvars_t, amount: f32) -> Option<bool> {
    if v.takedamage == 0.0 || v.health > v.max_health {
        return None;
    }
    v.health += amount;
    Some(v.health > v.max_health)
}

/// TS `CBasePlayer::TakeArmor(amount)` (0x93da4, `weapons::take_armor`) for an edict. An HEV
/// charger is used from the player's own `PlayerUse`, which lends the player to its slot
/// (`player_use`), so the armour lands inside the `Use` as in TS (`world::run_now`). A player
/// held by a dispatch that didn't lend it gets the armour once that returns, and this then
/// reports success, which both callers ignore anyway, as TS does. Any `CBasePlayer` takes it,
/// connected or not (`rules::has_player`): the callers have one in hand.
pub fn player_take_armor(p: *mut edict_t, amount: f32) -> bool {
    if !has_player(p) {
        return false;
    }
    let n = amount as i32;
    let take = move || ts_server::player::with_player(p, |pl, cx| ts_server::weapons::take_armor(pl, cx, n));
    if let Some(Some(took)) = ts_server::world::run_now(p, take) {
        return took;
    }
    if ts_server::world::is_taken_out(p) {
        ts_server::world::defer(p, move || {
            take();
        });
        return true;
    }
    false
}

/// One charge: `CRecharge::Use` (0xc6fd0) below 100 armour sets `m_iJuice = 0` and calls
/// `TakeArmor(100)`; `CWallHealth::Use` (0xc8d08) with health not at max calls
/// `TakeHealth(max_health - health)` and empties the charger when it was taken.
pub fn charge_once(armor: bool, juice: &mut i32, activator: *mut edict_t) {
    let pv = activator.v();
    if armor {
        if pv.armorvalue < 100.0 {
            *juice = 0;
            player_take_armor(activator, 100.0);
        }
    } else if pv.health != pv.max_health {
        let amount = pv.max_health - pv.health;
        if player_take_health(activator, amount) {
            *juice = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::weapons::id;

    /// `PickUpWeapon` (0x9314c): no akimbo pair of a single gun, a dropped stack costs per item.
    #[test]
    fn pick_up_cost_refuses_a_single_guns_pair_and_charges_a_stack() {
        let t = ts30_gameplay::tables::tables();
        let beretta = t.weapon(id::BERETTA).unwrap();
        let pair = t.weapon(beretta.akimbo as u8).unwrap();
        assert_eq!(pick_up_cost(pair.id, pair, None), None, "the pair only comes from two singles");
        assert_eq!(pick_up_cost(id::BERETTA, beretta, None), Some(beretta.slots as f32));
        let m61 = t.weapon(id::M61).unwrap();
        assert!(m61.stack);
        let three = Status { owned: true, clip: 3, ..Default::default() };
        assert_eq!(pick_up_cost(id::M61, m61, Some(&three)), Some(m61.slots as f32 * 3.0));
        assert_eq!(pick_up_cost(id::M61, m61, None), Some(m61.slots as f32));
    }

    #[test]
    fn take_health_rules() {
        let mut v = entvars_t { takedamage: 2.0, health: 90.0, max_health: 100.0, ..Default::default() };
        assert_eq!(add_health(&mut v, 50.0), Some(true), "no cap, overheal starts the decay");
        assert_eq!(v.health, 140.0);
        assert_eq!(add_health(&mut v, 10.0), None, "already above max");
        v.health = 100.0;
        assert_eq!(add_health(&mut v, 0.0), Some(false), "at max is still allowed");
        v.health = 50.0;
        assert_eq!(add_health(&mut v, 10.0), Some(false));
        assert_eq!(v.health, 60.0);
        v.takedamage = 0.0;
        assert_eq!(add_health(&mut v, 10.0), None, "takedamage off");
        assert_eq!(v.health, 60.0);
    }
}
