//! The `CBasePlayer` side of the inventory: pickups (`PickUpWeapon` 0x9314c), drops
//! (`DropWeapon` 0x94900, `ThrowWeapon` 0x9529c, `DropAmmo` 0x946f0), the loadout
//! (`BuyArmament` 0x93f10, `BuyWeapon` 0x945bc, `BuyAmmo` 0x941e8, `AddItem` 0x9440c,
//! `TakeArmor` 0x93da4), slots and speed (`GetSpeedBySlots` 0x76bd4, `SetPlayerSpeed` 0x93068).

use super::host::{emit, with_host};
use super::{SLOTS, assets, worldgun};
use crate::player::Player;
use crate::world::Cx;
use goldsrc::server::engine;
use goldsrc::sys::*;
use shared::weapons::spread::{self, Vec3};
use shared::weapons::{MAX_AMMO, MAX_WEAPONS, Status, att, id, info, is_restricted, kind, kind_of, restriction_type, tables};

/// What to drop.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Drop {
    /// `drop`: the held weapon (`DropWeapon(-1)`).
    Current,
    /// `drop_<id>`.
    Id(u8),
    /// Disarm by accumulated melee damage (`DropWeapon(-2, dmg)`).
    Disarm(f32),
}

/// Updates `Player::slots_used` and the move speed from the free slots
/// (`GetSpeedBySlots`: `210 + free * 120 / 81` truncated, 330 when nothing is carried or for
/// The One; the gameplay rules' [`PmRules::speed_by_slots`](shared::gameplay::PmRules)). Only
/// acts when the free slots changed (or after a spawn), so other subsystems may adjust `maxspeed`
/// in between.
pub fn sync_speed(p: &mut Player, cx: &mut Cx) {
    let free = p.inv.sv.free_slots;
    p.slots_used = SLOTS - free;
    if p.inv.sv.speed_for == Some(free) {
        return;
    }
    p.inv.sv.speed_for = Some(free);
    // `SetPlayerSpeed` clamps; its `ROUND` is a no-op on the whole number `GetSpeedBySlots` gives.
    let rules = crate::gameplay::active().pm;
    let speed = rules.speed_by_slots(free, p.ts.is_the_one).clamp(0.0, rules.max_player_speed());
    p.player_speed = speed; // `m_flPlayerSpeed`
    let pev = cx.pev();
    if pev.maxspeed != speed {
        pev.maxspeed = speed;
        crate::rules::set_client_maxspeed(cx.edict, speed);
    }
}

/// `CBasePlayer::PickUpWeapon(id, status, attachments, select, own)`.
/// Refused without the gun (`m_pActiveItem` must be a `CTSGun`).
pub fn pick_up_weapon(p: &mut Player, cx: &mut Cx, id: u8, from: Option<&Status>, attachments: u32, select: bool, own: bool) -> bool {
    if !super::gun_active(p) {
        return false;
    }
    let Some(w) = info(id) else { return false };
    let Some(cost) = crate::gameplay::active().server.pick_up_cost(id, w, from) else { return false };
    // `(int)(free - slots * clip)`, truncated (0x931d4..0x931ef: `fisubr`, `fistp` under chop);
    // the operands are integers, so it is exact (issue #80).
    let after = shared::pm::Num::trunc_i32(f64::from(p.inv.sv.free_slots) - f64::from(cost));
    if after < 0 {
        return false;
    }
    let Some(_target) = p.inv.gun.take_weapon(id, from, attachments) else { return false };
    if own {
        p.inv.sv.owners[id as usize] = cx.edict;
        if w.akimbo != -1 && p.inv.gun.status[w.akimbo as usize].owned {
            p.inv.sv.owners[w.akimbo as usize] = cx.edict;
        }
    }
    p.inv.sv.free_slots = after;
    sync_speed(p, cx);
    assets::debug(|| format!("picked up {id}, free slots {after}"));
    emit(cx.edict, CHAN_ITEM, c"items/gunpickup2.wav", 1.0, 0.8, 100);
    if select || p.inv.gun.current == 0 {
        let mut sel = id;
        if w.akimbo != -1 && p.inv.gun.status[w.akimbo as usize].owned {
            sel = w.akimbo as u8;
        }
        with_host(p, cx, |gun, h| {
            gun.switch_to(sel, h);
        });
    }
    true
}

/// The slots `PickUpWeapon` (0x9314c) charges for `id`: `None` for the akimbo pair of a single
/// gun (`kind != 4 || single == -1`: the pair only comes from two singles), else `slots`, times
/// the count for a dropped stack (rounded with the free slots by the caller).
pub fn pick_up_cost(_id: u8, w: &shared::weapons::WeaponInfo, from: Option<&Status>) -> Option<f32> {
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

/// The record `CTSGun::TakeWeapon` (0x80a04) copies a dropped gun's status into, given what was
/// owned before the pick-up: `id` when it wasn't owned, its akimbo pair when it was (and isn't
/// stackable); `None` for a stackable one already owned (only its count goes up). The whole
/// 0x38-byte record is copied there, original owner (`+0x30`) included.
pub fn copied_record(id: u8, owned_before: &[bool; MAX_WEAPONS]) -> Option<u8> {
    let w = info(id)?;
    if !owned_before[id as usize] {
        Some(id)
    } else if w.stack {
        None
    } else {
        u8::try_from(w.akimbo).ok().filter(|&a| (a as usize) < MAX_WEAPONS)
    }
}

/// `CBasePlayer::ThrowWeapon(id, status, angle offset, position offset)` (0x9529c): a
/// `WorldGun` 16 u in front of the eyes, thrown along the view at `(90 - pitch) * 2` u/s, or in
/// a random direction at 60..160 u/s by a corpse, plus 0.9 of the player's velocity and a third
/// of the speed upward (disassembly 0x85359, 0x855fb..0x856cf: constants 90, 0.9, 0.33; the
/// decompile garbles this part). No caller passes an angle offset.
fn throw_weapon(p: &mut Player, cx: &mut Cx, id: u8, status: Status, pos_offset: Option<Vec3>) -> *mut edict_t {
    let e = engine();
    let alive = Player::is_alive(cx);
    let pev = cx.pev();
    let (ang, speed) = if alive {
        (pev.v_angle, (90.0 - pev.v_angle[0]) * 2.0)
    } else {
        ([e.random_long(-60, 60) as f32, e.random_long(-180, 180) as f32, 0.0], e.random_long(60, 160) as f32)
    };
    let (f, r, _) = spread::angle_vectors(ang);
    let off = pos_offset.map(|o| [o[0] + 16.0, o[1], o[2]]).unwrap_or([16.0, 0.0, 0.0]);
    let eye = spread::add(pev.origin, pev.view_ofs);
    let mut origin = spread::add(eye, spread::add(spread::scale(f, off[0]), spread::scale(r, off[1])));
    origin[2] += off[2];
    let mut vel = spread::add(spread::scale(pev.velocity, 0.9), spread::scale(f, speed));
    vel[2] += speed * 0.33;
    let owner = p.inv.sv.owners[id as usize];
    let time_scale = super::projectiles::thrower_time_scale(p, cx.pev());
    worldgun::create(cx.edict, id, status, origin, vel, owner, time_scale)
}

/// Whether `DropWeapon` plays `weapons/weapondrop.wav` (0x951c0) for an owned `id` with record
/// `st`: every path does but two of the M61's, a primed grenade (thrown by `GrenadeIdle(1)`) and
/// one with none left, which jump straight past it to the lifetime check (0x84bac and 0x84bd9 to
/// 0x851ed).
fn drop_plays_sound(id: u8, st: &Status) -> bool {
    id != id::M61 || (st.state != 1 && st.clip > 0)
}

/// `CBasePlayer::DropWeapon(id | -1 current | -2 disarm, dmg)`.
pub fn drop_weapon(p: &mut Player, cx: &mut Cx, which: Drop) {
    // Nothing at all without the gun (0x94900 returns first), not even the CTB case.
    if !super::gun_active(p) {
        return;
    }
    let cur = p.inv.gun.current;
    let disarm = matches!(which, Drop::Disarm(_));
    if disarm && (cur == id::BRIEFCASE || cur < 1) {
        return;
    }
    if let Drop::Disarm(dmg) = which {
        p.inv.sv.disarm += dmg;
        let threshold = if cur == id::KATANA {
            30.0
        } else if restriction_type(cur) == kind::KNIFE {
            25.0
        } else {
            let s = info(cur).map(|w| w.slots).unwrap_or(0);
            if s < 21 { 20.0 } else { s.min(50) as f32 }
        };
        if p.inv.sv.disarm < threshold {
            return;
        }
        p.inv.sv.disarm = 0.0;
    }
    let better = !disarm;
    let target = match which {
        Drop::Id(i) => i,
        _ => cur,
    };
    if !(1..=37).contains(&target) {
        // `DropWeapon` 0x94a87: with no gun to drop, `drop` throws the CTB case (`DropFlag(false)`);
        // a disarm never does.
        if !disarm {
            crate::entities::ts::drop_flag(p, cx, false);
        }
        return;
    }
    if !p.inv.gun.status[target as usize].owned {
        return;
    }
    // 0x94aaf: a corpse's grenades become grenade power-ups (`CreatePowerUp(0x20, -1)`), 5 u
    // apart along x from its origin, outside round modes (`TSGameRules.m_mode < 2`, always in
    // 3.0; `ServerRules::dead_grenades_become_powerups`). The record stays in the gun (no `CTSGun::DropWeapon`, no drop sound or speed update).
    if target == id::M61 && !Player::is_alive(cx) && crate::gameplay::active().server.dead_grenades_become_powerups() {
        if p.inv.gun.status[id::M61 as usize].state == 1 {
            with_host(p, cx, |gun, h| gun.grenade_idle(true, h));
        }
        let mut i = 0;
        while i < p.inv.gun.status[id::M61 as usize].clip {
            let mut at = cx.pev().origin;
            at[0] += (5 * i) as f32;
            crate::entities::ts::create_powerup(shared::ts::powerup::GRENADE, -1, at);
            i += 1;
        }
        return;
    }
    // Guard: every id 1..=37 a gun can own has a table entry; one without (29) is never dropped.
    let Some(w) = info(target) else { return };
    let mut thrown: *mut edict_t = std::ptr::null_mut();
    let sound = drop_plays_sound(target, &p.inv.gun.status[target as usize]);
    if target == id::M61 {
        let st = p.inv.gun.status[id::M61 as usize];
        if st.state == 1 {
            // pin pulled: throw it
            with_host(p, cx, |gun, h| gun.grenade_idle(true, h));
        } else if st.clip > 0 {
            let mut s = st;
            s.clip = 1;
            thrown = throw_weapon(p, cx, id::M61, s, None);
            p.inv.sv.free_slots += w.slots;
            if st.clip < 2 {
                with_host(p, cx, |gun, h| gun.remove_weapon(id::M61, better, h));
            } else {
                p.inv.gun.status[id::M61 as usize].clip -= 1;
                if !better {
                    with_host(p, cx, |gun, h| gun.holster_weapon(h));
                }
            }
        }
    } else if !w.stack {
        if kind_of(target) == kind::AKIMBO && w.single != -1 {
            // the pair splits into two singles, half the rounds each
            let single = w.single as u8;
            let mut s = p.inv.gun.status[single as usize];
            s.clip = p.inv.gun.status[target as usize].clip / 2;
            let sw = info(single).map(|i| i.slots).unwrap_or(0);
            for side in [4.0, -4.0] {
                thrown = throw_weapon(p, cx, single, s, Some([0.0, side, 0.0]));
                // 0x94ed9 / 0x94fad: each half of the pair gets the lifetime and the dropper.
                if !matches!(which, Drop::Id(_)) && !thrown.is_null() {
                    worldgun::set_drop_lifetime(thrown, cx.edict);
                }
                p.inv.sv.free_slots += sw;
            }
            with_host(p, cx, |gun, h| {
                gun.remove_weapon(target, better, h);
                gun.remove_weapon(single, better, h);
            });
        } else if w.akimbo == -1 || !p.inv.gun.status[w.akimbo as usize].owned {
            let s = p.inv.gun.status[target as usize];
            thrown = throw_weapon(p, cx, target, s, None);
            p.inv.sv.free_slots += w.slots;
            with_host(p, cx, |gun, h| gun.remove_weapon(target, better, h));
        } else {
            // a single whose pair is owned: drop one gun, keep the other
            let ak = w.akimbo as usize;
            let s = p.inv.gun.status[target as usize];
            p.inv.gun.status[target as usize].clip = p.inv.gun.status[ak].right;
            thrown = throw_weapon(p, cx, target, s, None);
            p.inv.sv.free_slots += w.slots;
            with_host(p, cx, |gun, h| {
                gun.remove_weapon(ak as u8, better, h);
                gun.holster_weapon(h);
                gun.switch_to(target, h);
            });
        }
    } else {
        let mut s = p.inv.gun.status[target as usize];
        s.clip = crate::gameplay::active().server.stack_drop_count(s.clip, disarm);
        let n = s.clip;
        thrown = throw_weapon(p, cx, target, s, None);
        p.inv.sv.free_slots += w.slots * n;
        if n < p.inv.gun.status[target as usize].clip {
            p.inv.gun.status[target as usize].clip -= n;
            if !better {
                with_host(p, cx, |gun, h| gun.holster_weapon(h));
            }
        } else {
            with_host(p, cx, |gun, h| gun.remove_weapon(target, better, h));
        }
    }
    if sound {
        emit(cx.edict, CHAN_ITEM, c"weapons/weapondrop.wav", 1.0, 0.8, 100);
    }
    assets::debug(|| format!("dropped {target} ({which:?}), free slots {}", p.inv.sv.free_slots));
    // 0x95265: `drop`/disarm (id < 0) gives the last gun thrown the longer lifetime and the
    // dropper's glow (`WorldGun` +0xc4 / +0xd0), except the CTB briefcase.
    if matches!(which, Drop::Current | Drop::Disarm(_)) && target != id::BRIEFCASE && !thrown.is_null() {
        worldgun::set_drop_lifetime(thrown, cx.edict);
    }
    sync_speed(p, cx);
}

/// Inventory weight of the kevlar vest (`TakeArmor` 0x93da4).
pub const KEVLAR_SLOTS: i32 = 10;

/// What `CBasePlayer::TakeArmor(n)` (0x93da4) does to the free slots (`pl+0x548`), the loadout
/// items (`pl+0x718`) and `armorvalue`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ArmorChange {
    free_slots: i32,
    items: u32,
    armor: f32,
    /// The vest went on (TS plays `weapons/items.wav`).
    put_on: bool,
}

/// The pure part of `TakeArmor(n)`. `n < 0` takes the vest off: its slots come back and the
/// armour goes to 0 (no-op without one). Otherwise the first vest needs [`KEVLAR_SLOTS`] free
/// (`None`: refused, nothing changes), then the armour goes up by `n`, capped at 100.
fn armor_change(free_slots: i32, items: u32, armor: f32, n: i32) -> Option<ArmorChange> {
    let mut c = ArmorChange { free_slots, items, armor, put_on: false };
    if n < 0 {
        if items & att::KEVLAR != 0 {
            c.free_slots += KEVLAR_SLOTS;
            c.items &= !att::KEVLAR;
            c.armor = 0.0;
        }
        return Some(c);
    }
    if items & att::KEVLAR == 0 {
        if free_slots < KEVLAR_SLOTS {
            return None;
        }
        c.free_slots -= KEVLAR_SLOTS;
        c.items |= att::KEVLAR;
        c.put_on = true;
    }
    c.armor = (n as f32 + armor).min(100.0);
    Some(c)
}

/// `CBasePlayer::TakeArmor(n)` (0x93da4): the one armour path (buy menu, armour power-up,
/// battery, HEV charger). The vest's weight shares the free-slot count with weapons and the CTB
/// case, so pickups and the speed see it; `n < 0` takes it off and gives the slots back.
/// False when there's no room for the vest.
pub fn take_armor(p: &mut Player, cx: &mut Cx, n: i32) -> bool {
    let Some(c) = armor_change(p.inv.sv.free_slots, p.inv.sv.items, cx.pev().armorvalue, n) else {
        return false;
    };
    p.inv.sv.free_slots = c.free_slots;
    p.inv.sv.items = c.items;
    cx.pev().armorvalue = c.armor;
    if c.put_on {
        emit(cx.edict, CHAN_ITEM, c"weapons/items.wav", 1.0, 0.8, 100);
    }
    sync_speed(p, cx);
    true
}

/// The loadout part of `TSInit` (0x92a4c): `pl+0x718 = 0`. `Player::spawn` has already zeroed
/// `armorvalue`, so a vest still worn (a spawn that didn't go through death) gives its slots back.
/// `CBasePlayer::AddItem(bits)` (`tki`): 0x10 kevlar, 0x20 stealth shoes, else attachments for
/// the held weapon. Prices are never charged (`CashBased() == 0`).
fn add_item(p: &mut Player, cx: &mut Cx, bits: u32) {
    if bits & att::KEVLAR != 0 {
        if cx.pev().armorvalue < 100.0 {
            take_armor(p, cx, 100);
        }
        return;
    }
    if bits & att::STEALTH_SHOES != 0 {
        // TS: only in round modes (m_mode >= 2), never in 3.0 DM/TDM.
        if !assets::grenade_buys() {
            return;
        }
        let pev = cx.pev();
        if pev.iuser4 & shared::ts::stunt::STEALTH_SHOES != 0 {
            return;
        }
        pev.iuser4 |= shared::ts::stunt::STEALTH_SHOES;
        p.inv.sv.items |= att::STEALTH_SHOES;
        emit(cx.edict, CHAN_ITEM, c"weapons/items.wav", 1.0, 0.8, 100);
        return;
    }
    if !super::gun_active(p) {
        return; // attachments go on the gun's weapon
    }
    with_host(p, cx, |gun, h| gun.add_item(bits, h));
}

/// `CBasePlayer::BuyWeapon(id, attachments)`.
fn buy_weapon(p: &mut Player, cx: &mut Cx, wid: i32, attachments: u32) {
    if !super::gun_active(p) || !(0..MAX_WEAPONS as i32).contains(&wid) {
        return;
    }
    let wid = wid as u8;
    if is_restricted(wid, assets::restriction()) {
        return;
    }
    if wid == id::M61 {
        if !assets::grenade_buys() || p.inv.sv.grenades_bought > 1 {
            return;
        }
        p.inv.sv.grenades_bought += 1;
    }
    pick_up_weapon(p, cx, wid, None, attachments, true, true);
}

/// `CBasePlayer::BuyAmmo(type)`: one purchase (`per_buy` rounds); -1 = held weapon's type,
/// -2 = every owned weapon's type until full. Returns whether any ammo was added.
pub fn buy_ammo(p: &mut Player, cx: &mut Cx, t: i32) -> bool {
    if !super::gun_active(p) {
        return false;
    }
    let per_buy = |t: u8| tables().ammo(t).map(|a| a.per_buy).unwrap_or(0);
    let mut sound = false;
    if t == -2 {
        for wid in (0..MAX_WEAPONS as u8).rev() {
            if !p.inv.gun.status[wid as usize].owned {
                continue;
            }
            let ammo = info(wid).map(|w| w.ammo).unwrap_or(0);
            if ammo == 0 {
                continue;
            }
            while p.inv.gun.add_ammo(ammo, per_buy(ammo), assets::max_carry(ammo)) {
                sound = true;
            }
        }
    } else {
        if p.inv.gun.current == 0 {
            return false;
        }
        let ammo = if t == -1 { info(p.inv.gun.current).map(|w| w.ammo).unwrap_or(0) } else { t as u8 };
        if ammo == 0 || ammo as usize >= MAX_AMMO {
            return false;
        }
        sound = p.inv.gun.add_ammo(ammo, per_buy(ammo), assets::max_carry(ammo));
    }
    if sound {
        emit(cx.edict, CHAN_ITEM, c"items/9mmclip1.wav", 1.0, 0.8, 100);
    }
    sound
}

/// `CBasePlayer::BuyArmament` (from `PlayerSpawn` when the map has no `game_player_equip`).
pub fn buy_armament(p: &mut Player, cx: &mut Cx) {
    let list = p.inv.sv.armament.clone();
    for a in list {
        if a.id == 0 {
            continue;
        }
        if a.item {
            add_item(p, cx, a.id as u32);
        } else {
            buy_weapon(p, cx, a.id, a.attachments);
        }
    }
    // Guard: `tka` takes any purchase count from the client, and TS loops that many times. A
    // purchase that adds nothing leaves everything as it was, so every later one would too: stop
    // there instead of spinning through billions of no-ops at spawn.
    let ammo = p.inv.sv.armament_ammo;
    for (t, &n) in ammo.iter().enumerate() {
        for _ in 0..n.max(0) {
            if !buy_ammo(p, cx, t as i32) {
                break;
            }
        }
    }
}

/// `CBasePlayer::Killed` (0x8ed5c) weapon part: the active item's `Holster` (the gun's, or a
/// cycler's), `DropAmmo` (0x946f0, the active gun's reserve copied into a pack), `DropFlag(false)` (the CTB case,
/// with or without a gun), then `DropWeapon` for every id (each one twice when it has an akimbo
/// pair), and the loadout cleared (`ResetArmament` 0x93e88: `+0x554..+0x714`; the client
/// resubmits it from the buy menu). What is left in the gun (the reserve, a corpse's grenades)
/// goes with the gun itself on the first death think (`weapons::remove_all_items`, which also
/// sends `CurWeapon 0,0,0`). The free slots, loadout items (a vest's 10 slots stay taken) and
/// speed are only what the drops left; the next spawn's `TSInit` resets them
/// (`weapons::ts_init`).
pub fn on_killed(p: &mut Player, cx: &mut Cx) {
    super::holster_active(p, cx);
    if super::gun_active(p) {
        worldgun::drop_ammo(cx.edict, p.inv.gun.reserve);
    }
    crate::entities::ts::drop_flag(p, cx, false);
    for wid in 1..MAX_WEAPONS as u8 {
        if wid == id::BRIEFCASE {
            continue;
        }
        drop_weapon(p, cx, Drop::Id(wid));
        if info(wid).map(|w| w.akimbo != -1).unwrap_or(false) {
            drop_weapon(p, cx, Drop::Id(wid));
        }
    }
    p.inv.sv.armament.clear();
    p.inv.sv.armament_ammo = [0; MAX_AMMO];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropped_gun_status_lands_in_takeweapons_record() {
        let mut owned = [false; MAX_WEAPONS];
        assert_eq!(copied_record(id::BERETTA, &owned), Some(id::BERETTA));
        // A second single pistol becomes the akimbo pair.
        owned[id::BERETTA as usize] = true;
        let pair = info(id::BERETTA).unwrap().akimbo;
        assert!(pair > 0);
        assert_eq!(copied_record(id::BERETTA, &owned), Some(pair as u8));
        // A stackable one already owned only counts up.
        let stack = (0..MAX_WEAPONS as u8).find(|&i| info(i).is_some_and(|w| w.stack)).unwrap();
        owned[stack as usize] = true;
        assert_eq!(copied_record(stack, &owned), None);
    }

    #[test]
    fn no_drop_sound_for_a_primed_or_empty_grenade() {
        let st = |state, clip| Status { owned: true, state, clip, ..Default::default() };
        assert!(drop_plays_sound(id::M61, &st(0, 2)));
        assert!(!drop_plays_sound(id::M61, &st(1, 2)), "primed: GrenadeIdle(1) throws it, silently");
        assert!(!drop_plays_sound(id::M61, &st(0, 0)), "nothing to throw");
        // Every other weapon, whatever its record says.
        assert!(drop_plays_sound(id::AK47, &st(1, 0)));
    }

    /// `PickUpWeapon` (0x9314c): no akimbo pair of a single gun, a dropped stack costs per item.
    #[test]
    fn pick_up_cost_refuses_a_single_guns_pair_and_charges_a_stack() {
        let t = shared::weapons::table::base_tables();
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
    fn first_vest_costs_ten_slots() {
        let c = armor_change(SLOTS, 0, 0.0, 100).unwrap();
        assert_eq!(c, ArmorChange { free_slots: SLOTS - 10, items: att::KEVLAR, armor: 100.0, put_on: true });
        // A second vest only tops the armour up.
        let c = armor_change(c.free_slots, c.items | att::STEALTH_SHOES, 30.0, 50).unwrap();
        assert_eq!(c, ArmorChange { free_slots: SLOTS - 10, items: att::KEVLAR | att::STEALTH_SHOES, armor: 80.0, put_on: false });
    }

    #[test]
    fn no_room_for_the_vest() {
        assert_eq!(armor_change(9, 0, 0.0, 50), None);
        assert_eq!(armor_change(10, 0, 0.0, 50).map(|c| c.free_slots), Some(0));
        // Already wearing one: no slots needed.
        assert_eq!(armor_change(0, att::KEVLAR, 20.0, 50).map(|c| c.armor), Some(70.0));
    }

    #[test]
    fn taking_the_vest_off_returns_its_slots() {
        let c = armor_change(SLOTS - 30, att::KEVLAR | att::STEALTH_SHOES, 64.0, -1).unwrap();
        assert_eq!(c, ArmorChange { free_slots: SLOTS - 20, items: att::STEALTH_SHOES, armor: 0.0, put_on: false });
        // Without a vest nothing changes (armour included).
        let c = armor_change(SLOTS, 0, 5.0, -1).unwrap();
        assert_eq!((c.free_slots, c.items, c.armor), (SLOTS, 0, 5.0));
    }
}
