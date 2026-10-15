//! TS 2.1's money (decomp `docs/ts21/rules.md` §5): a working economy. `CashBased()` is on in the
//! round modes and with `usecash` in DM/TDM; buying costs money then; kills pay; there is a floor
//! of `initial_cash`.

use crate::cvars as cv;
use crate::rules::{self, team_cash};
use goldsrc::sys::CHAN_ITEM;
use shared::weapons::{MAX_AMMO, MAX_WEAPONS, att, id, info, is_restricted, tables};
use ts_server::player::Player;
use ts_server::rules::lifecycle::give_cash;
use ts_server::rules::state::{self, Mode};
use ts_server::weapons;
use ts_server::world::Cx;

/// Attachment prices (`FUN_10055e00`/`FUN_10055c30`): silencer, laser, flashlight, scope.
pub const ATTACHMENT_PRICES: [(u32, i32); 4] = [(att::SILENCER, 900), (att::LASER, 800), (att::FLASHLIGHT, 500), (att::SCOPE, 1800)];
/// Kevlar (`tki` 0x10), round modes only.
pub const KEVLAR_PRICE: i32 = 1900;
/// Stealth shoes (`tki` 0x20), round modes only.
pub const SHOES_PRICE: i32 = 700;

/// `CashBased()` (0x100435f0): the round modes, or the `(int)usecash` copy Think keeps
/// (`TSGameRules+0x1a0`, -1 from `Init` until the first Think) non-zero.
pub fn cash_based() -> bool {
    let round = state::with(|g| matches!(g.mode, Mode::RoundTdm | Mode::Objective));
    round || rules::with(|r| r.srvsett[1] != 0)
}

/// `GiveKillCash` (0x10042d00): only while cash-based.
pub fn give_kill_cash(p: &mut Player, n: i32) {
    if cash_based() {
        give_cash(p, n);
    }
}

/// `FUN_10042c90`, from `StartObserver`, `Killed`, `TSInit` and the team change: in the round
/// modes the player picks up its team's cash when that is more, then the cash is at least
/// `initial_cash`.
pub fn cash_floor(p: &mut Player) {
    let round = state::with(|g| matches!(g.mode, Mode::RoundTdm | Mode::Objective));
    if round {
        p.cash = p.cash.max(team_cash(&p.ts.team_name));
    }
    p.cash = floor(p.cash, cv::initial_cash());
}

/// `if ((float)cash < initial_cash) cash = (int)initial_cash`.
pub fn floor(cash: i32, initial: f32) -> i32 {
    if (cash as f32) < initial { cv::trunc(initial) } else { cash }
}

/// The money left after paying `price` and the attachments' prices, when cash-based; `cash`
/// otherwise is ignored (the start is 0, so nothing is ever refused for money).
pub fn after_purchase(cash_based: bool, cash: i32, price: i32, attachments: u32) -> i32 {
    if !cash_based {
        return 0;
    }
    ATTACHMENT_PRICES.iter().filter(|(bit, _)| attachments & bit != 0).fold(cash - price, |c, (_, p)| c - p)
}

/// `BuyArmament` (0x100559d0): every listed weapon or item, then each ammo type as many times as
/// listed.
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
        // Guard (as the 3.0 port): a purchase that adds nothing leaves everything as it was, so
        // stop instead of looping through any count the client sent.
        for _ in 0..n.max(0) {
            if !buy_ammo(p, cx, t as i32) {
                break;
            }
        }
    }
}

/// `BuyWeapon(id, attachments)` (0x10055e00): with the gun out, not restricted; M61s only in
/// the round modes, two per life; the price when cash-based; paid only if the weapon is taken.
fn buy_weapon(p: &mut Player, cx: &mut Cx, wid: i32, attachments: u32) {
    if !weapons::gun_active(p) || !(0..MAX_WEAPONS as i32).contains(&wid) {
        return;
    }
    let wid = wid as u8;
    if is_restricted(wid, weapons::restriction()) {
        return;
    }
    let round = state::with(|g| matches!(g.mode, Mode::RoundTdm | Mode::Objective));
    if wid == id::M61 {
        if !round || p.inv.sv.grenades_bought > 1 {
            return;
        }
        p.inv.sv.grenades_bought += 1;
    }
    let based = cash_based();
    let price = info(wid).map_or(0, |w| w.cost);
    let left = after_purchase(based, p.cash, price, attachments);
    if left >= 0 && weapons::pick_up_weapon(p, cx, wid, None, attachments, true, true) && based {
        p.cash = left;
    }
}

/// `CBasePlayer::PickUpWeapon` (0x10055830): `free_slots - info.slots` (the byte), with no akimbo
/// refusal and no stack multiplier. `GiveWeapon` (FUN_1004a040) then builds an akimbo id that
/// isn't owned from its single's record (the single's clip on the left, a full one on the right),
/// so a hand-typed `tkw` with an akimbo id buys the pair.
pub fn pick_up_cost(w: &shared::weapons::WeaponInfo) -> Option<f32> {
    Some(w.slots as f32)
}

/// `AddItem(bits)` (0x10055c30): kevlar (0x10) and stealth shoes (0x20) only in the round modes;
/// otherwise attachments for the held weapon.
fn add_item(p: &mut Player, cx: &mut Cx, bits: u32) {
    let based = cash_based();
    let cash = if based { p.cash } else { 0 };
    let round = state::with(|g| matches!(g.mode, Mode::RoundTdm | Mode::Objective));
    if bits & att::KEVLAR != 0 {
        if !round {
            return;
        }
        let left = if based { cash - KEVLAR_PRICE } else { cash };
        // Sets the armour to 100 outright; no slots.
        if cx.pev().armorvalue < 100.0 && left >= 0 {
            cx.pev().armorvalue = 100.0;
            if based {
                p.cash = left;
            }
            // 2.1 marks it in its own item mask (`+0x690 |= 0x10`). The port's loadout bit is 3.0's
            // vest, which carries 10 slots (`take_armor`), so it stays clear: 2.1's vest weighs
            // nothing.
            weapons::emit(cx.edict, CHAN_ITEM, c"weapons/items.wav", 1.0, 0.8, 100);
        }
        return;
    }
    if bits & att::STEALTH_SHOES != 0 {
        if !round {
            return;
        }
        let left = if based { cash - SHOES_PRICE } else { cash };
        let pev = cx.pev();
        if pev.iuser4 & shared::ts::stunt::STEALTH_SHOES == 0 && left >= 0 {
            pev.iuser4 |= shared::ts::stunt::STEALTH_SHOES;
            if based {
                p.cash = left;
            }
            p.inv.sv.items |= att::STEALTH_SHOES;
            weapons::emit(cx.edict, CHAN_ITEM, c"weapons/items.wav", 1.0, 0.8, 100);
        }
        return;
    }
    if !weapons::gun_active(p) {
        return;
    }
    let left = after_purchase(based, cash, 0, bits);
    if left >= 0 && weapons::add_gun_item(p, cx, bits) && based {
        p.cash = left;
    }
}

/// `BuyAmmo(type)` (0x10055a40): one purchase (`per_buy` rounds, at the ammo's price when
/// cash-based); -1 the held weapon's type, -2 every owned weapon's type (from the top id down)
/// until full or out of money. Returns whether any ammo was added.
fn buy_ammo(p: &mut Player, cx: &mut Cx, t: i32) -> bool {
    if !weapons::gun_active(p) {
        return false;
    }
    let based = cash_based();
    let ammo_of = |a: u8| tables().ammo(a).map_or((0, 0), |a| (a.price, a.per_buy));
    let mut sound = false;
    if t == -2 {
        let mut left = 0;
        for wid in (0..MAX_WEAPONS as u8).rev() {
            if !p.inv.gun.status[wid as usize].owned {
                continue;
            }
            let ammo = info(wid).map_or(0, |w| w.ammo);
            if ammo == 0 {
                continue;
            }
            let (price, per_buy) = ammo_of(ammo);
            if based {
                left = p.cash - price;
            }
            if left < 0 {
                continue;
            }
            while p.inv.gun.add_ammo(ammo, per_buy, weapons::max_carry(ammo)) {
                if based {
                    p.cash = left;
                    left = p.cash - price;
                }
                sound = true;
                if left < 0 {
                    break;
                }
            }
            if left < 0 {
                break;
            }
        }
    } else {
        if p.inv.gun.current == 0 {
            return false;
        }
        let ammo = if t == -1 { info(p.inv.gun.current).map_or(0, |w| w.ammo) } else { t as u8 };
        if ammo == 0 || ammo as usize >= MAX_AMMO {
            return false;
        }
        let (price, per_buy) = ammo_of(ammo);
        let left = p.cash - price;
        if based && left < 0 {
            return false;
        }
        sound = p.inv.gun.add_ammo(ammo, per_buy, weapons::max_carry(ammo));
        if sound && based {
            p.cash = left;
        }
    }
    if sound {
        weapons::emit(cx.edict, CHAN_ITEM, c"items/9mmclip1.wav", 1.0, 0.8, 100);
    }
    sound
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2.1 charges the plain slots: the akimbo pair of a single gun is given, a dropped stack
    /// costs one item's slots, and 3.0's refusal and multiplier stay 3.0's.
    #[test]
    fn pick_up_cost_has_no_akimbo_refusal_or_stack_multiplier() {
        use ts_server::gameplay::ServerRules;
        let t = ts21_gameplay::shared::tables::tables();
        let uzis = t.weapon(16).unwrap();
        assert_ne!(uzis.single, -1, "a single gun's pair");
        assert_eq!(pick_up_cost(uzis), Some(uzis.slots as f32));
        let m61 = t.weapon(id::M61).unwrap();
        let three = shared::weapons::Status { owned: true, clip: 3, ..Default::default() };
        let v21: &dyn ServerRules = &crate::Ts21Server;
        assert_eq!(v21.pick_up_cost(id::M61, m61, Some(&three)), Some(m61.slots as f32));
        let core: &dyn ServerRules = &ts30_server::Ts30Server;
        let t30 = shared::weapons::table::base_tables();
        let uzis30 = t30.weapon(16).unwrap();
        assert_eq!(core.pick_up_cost(16, uzis30, None), None);
    }

    #[test]
    fn the_cash_floor_is_initial_cash() {
        assert_eq!(floor(0, 3000.0), 3000);
        assert_eq!(floor(2999, 3000.0), 3000);
        assert_eq!(floor(5000, 3000.0), 5000);
        assert_eq!(floor(100, 1200.7), 1200, "truncated");
    }

    #[test]
    fn purchases_cost_the_price_and_the_attachments() {
        assert_eq!(after_purchase(true, 5000, 1600, 0), 3400);
        assert_eq!(after_purchase(true, 5000, 1600, att::SILENCER | att::LASER), 1700);
        assert_eq!(after_purchase(true, 5000, 0, att::FLASHLIGHT | att::SCOPE), 2700);
        assert_eq!(after_purchase(true, 1000, 1600, 0), -600, "refused: below 0");
        assert_eq!(after_purchase(false, 0, 1600, att::SCOPE), 0, "free without cash");
    }
}
