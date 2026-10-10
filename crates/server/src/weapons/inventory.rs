//! The `CBasePlayer` side of the inventory: pickups (`PickUpWeapon` 0x9314c), drops
//! (`DropWeapon` 0x94900, `ThrowWeapon` 0x9529c, `DropAmmo` 0x946f0), the loadout
//! (`BuyArmament` 0x93f10, `BuyWeapon` 0x945bc, `BuyAmmo` 0x941e8, `AddItem` 0x9440c,
//! `TakeArmor` 0x93da4), slots and speed (`GetSpeedBySlots` 0x76bd4, `SetPlayerSpeed` 0x93068).

use super::host::{emit, with_host};
use super::{SLOTS, assets, net, worldgun};
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
/// (`210 + free * 120 / 81`, rounded, 330 when nothing is carried).
/// Only acts when the free slots changed (or after a spawn), so other subsystems may adjust
/// `maxspeed` in between.
pub fn sync_speed(p: &mut Player, cx: &mut Cx) {
    let free = p.inv.sv.free_slots;
    p.slots_used = SLOTS - free;
    if p.inv.sv.speed_for == Some(free) {
        return;
    }
    p.inv.sv.speed_for = Some(free);
    let speed = if free < SLOTS { shared::ts::speed_for_free_slots(free).round_ties_even() } else { 330.0 };
    let speed = speed.clamp(0.0, 330.0);
    let pev = cx.pev();
    if pev.maxspeed != speed {
        pev.maxspeed = speed;
        engine().set_client_maxspeed(cx.edict, speed);
    }
}

/// `CBasePlayer::PickUpWeapon(id, status, attachments, select, own)`.
pub fn pick_up_weapon(p: &mut Player, cx: &mut Cx, id: u8, from: Option<&Status>, attachments: u32, select: bool, own: bool) -> bool {
    let Some(w) = info(id) else { return false };
    if kind_of(id) == kind::AKIMBO && w.single != -1 {
        return false; // the pair only comes from two singles
    }
    let mut cost = w.slots as f32;
    if w.stack {
        if let Some(s) = from {
            cost *= s.clip as f32;
        }
    }
    let after = (p.inv.sv.free_slots as f32 - cost).round_ties_even() as i32;
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

/// `CBasePlayer::ThrowWeapon(id, status, angle offset, position offset)`: a `WorldGun` 16 u in
/// front of the eyes (random direction for corpses).
fn throw_weapon(p: &mut Player, cx: &mut Cx, id: u8, status: Status, pos_offset: Option<Vec3>) -> *mut edict_t {
    let e = engine();
    let alive = Player::is_alive(cx);
    let pev = cx.pev();
    let (ang, speed) = if alive {
        (pev.v_angle, 250.0)
    } else {
        ([e.random_long(-60, 60) as f32, e.random_long(-180, 180) as f32, 0.0], e.random_long(60, 160) as f32)
    };
    let (f, r, _) = spread::angle_vectors(ang);
    let off = pos_offset.map(|o| [o[0] + 16.0, o[1], o[2]]).unwrap_or([16.0, 0.0, 0.0]);
    let eye = spread::add(pev.origin, pev.view_ofs);
    let mut origin = spread::add(eye, spread::add(spread::scale(f, off[0]), spread::scale(r, off[1])));
    origin[2] += off[2];
    let mut vel = spread::ma(pev.velocity, speed, f);
    vel[2] += speed * 0.5;
    let owner = p.inv.sv.owners[id as usize];
    let time_scale = super::projectiles::thrower_time_scale(p, cx.pev());
    worldgun::create(cx.edict, id, status, origin, vel, owner, time_scale)
}

/// `CBasePlayer::DropWeapon(id | -1 current | -2 disarm, dmg)`.
pub fn drop_weapon(p: &mut Player, cx: &mut Cx, which: Drop) {
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
        // the briefcase/flag belongs to the rules
        return;
    }
    if !p.inv.gun.status[target as usize].owned {
        return;
    }
    let w = info(target).expect("owned weapon has a table entry");
    let mut thrown: *mut edict_t = std::ptr::null_mut();
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
        if disarm {
            s.clip = 1;
        }
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
    emit(cx.edict, CHAN_ITEM, c"weapons/weapondrop.wav", 1.0, 0.8, 100);
    assets::debug(|| format!("dropped {target} ({which:?}), free slots {}", p.inv.sv.free_slots));
    if matches!(which, Drop::Current | Drop::Disarm(_)) && !thrown.is_null() {
        worldgun::set_drop_lifetime(thrown, cx.edict);
    }
    sync_speed(p, cx);
}

/// `CBasePlayer::TakeArmor(100)`: kevlar costs 10 slots once.
fn take_armor(p: &mut Player, cx: &mut Cx) -> bool {
    if p.inv.sv.items & att::KEVLAR == 0 {
        if p.inv.sv.free_slots < 10 {
            return false;
        }
        p.inv.sv.free_slots -= 10;
        p.inv.sv.items |= att::KEVLAR;
        emit(cx.edict, CHAN_ITEM, c"weapons/items.wav", 1.0, 0.8, 100);
    }
    let pev = cx.pev();
    pev.armorvalue = (pev.armorvalue + 100.0).min(100.0);
    sync_speed(p, cx);
    true
}

/// `CBasePlayer::AddItem(bits)` (`tki`): 0x10 kevlar, 0x20 stealth shoes, else attachments for
/// the held weapon. Prices are never charged (`CashBased() == 0`).
fn add_item(p: &mut Player, cx: &mut Cx, bits: u32) {
    if bits & att::KEVLAR != 0 {
        if cx.pev().armorvalue < 100.0 {
            take_armor(p, cx);
        }
        return;
    }
    if bits & att::STEALTH_SHOES != 0 {
        // TS: only in round modes (m_mode >= 2), never in 3.0 DM/TDM.
        if !assets::grenade_buys() {
            return;
        }
        let pev = cx.pev();
        if pev.iuser4 & 0x200 != 0 {
            return;
        }
        pev.iuser4 |= 0x200;
        p.inv.sv.items |= att::STEALTH_SHOES;
        emit(cx.edict, CHAN_ITEM, c"weapons/items.wav", 1.0, 0.8, 100);
        return;
    }
    with_host(p, cx, |gun, h| gun.add_item(bits, h));
}

/// `CBasePlayer::BuyWeapon(id, attachments)`.
fn buy_weapon(p: &mut Player, cx: &mut Cx, wid: i32, attachments: u32) {
    if !(0..MAX_WEAPONS as i32).contains(&wid) {
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
/// -2 = every owned weapon's type until full.
pub fn buy_ammo(p: &mut Player, cx: &mut Cx, t: i32) {
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
            return;
        }
        let ammo = if t == -1 { info(p.inv.gun.current).map(|w| w.ammo).unwrap_or(0) } else { t as u8 };
        if ammo == 0 || ammo as usize >= MAX_AMMO {
            return;
        }
        sound = p.inv.gun.add_ammo(ammo, per_buy(ammo), assets::max_carry(ammo));
    }
    if sound {
        emit(cx.edict, CHAN_ITEM, c"items/9mmclip1.wav", 1.0, 0.8, 100);
    }
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
    let ammo = p.inv.sv.armament_ammo;
    for (t, &n) in ammo.iter().enumerate() {
        for _ in 0..n.max(0) {
            buy_ammo(p, cx, t as i32);
        }
    }
}

/// `CBasePlayer::Killed` weapon part: holster, drop the ammo pack and every weapon, clear the
/// inventory and the loadout (TS `ResetArmament`; the client resubmits it from the buy menu).
pub fn on_killed(p: &mut Player, cx: &mut Cx) {
    with_host(p, cx, |gun, h| gun.holster_weapon(h));
    worldgun::drop_ammo(cx.edict, p.inv.gun.reserve);
    for wid in 1..MAX_WEAPONS as u8 {
        if wid == id::BRIEFCASE {
            continue;
        }
        drop_weapon(p, cx, Drop::Id(wid));
        if info(wid).map(|w| w.akimbo != -1).unwrap_or(false) {
            drop_weapon(p, cx, Drop::Id(wid));
        }
    }
    p.inv.gun.reset_inventory();
    p.inv.sv.free_slots = SLOTS;
    p.inv.sv.items = 0;
    p.inv.sv.owners = [std::ptr::null_mut(); MAX_WEAPONS];
    p.inv.sv.armament.clear();
    p.inv.sv.armament_ammo = [0; MAX_AMMO];
    net::send_act_items(cx.edict, 0);
    p.inv.sv.sent_act_items = Some(0);
    net::send_cur_weapon_reset(cx.edict);
    sync_speed(p, cx);
}
