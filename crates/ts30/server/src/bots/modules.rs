//! TS 3.0's vision, buy and look-around modules (bots.md §7, §9).

use goldsrc::edict::EdictPtr;
use goldsrc::server::{IGNORE_MONSTERS, engine};
use goldsrc::sys::*;
use ts_server::bots::brain::{Brain, add, bit, gun_position, len_f32, order, scale, sub};
use ts_server::bots::modules::{add_item, buy_weapon, in_view_cone, reset_armament, visible};
use ts_server::bots::rules::delay;
use ts_server::player::Player;
use ts_server::rules::cvars;
use ts_server::world::Cx;

// ---- vision (bit 0x40) ---------------------------------------------------------------------------

/// `CBotModuleVision::Scan` (0x1006dc10): every `0.1 / slow / skill` s (at most 0.7 s), list the
/// living players in a ±60° cone whose eyes the bot's eyes can see; teammates (teamplay, same
/// team) go to a list nothing reads, everyone else is an enemy.
pub fn vision(b: &mut Brain, _p: &mut Player, cx: &mut Cx) {
    let now = cx.time();
    if b.vision_next > now {
        return;
    }
    b.vision_next = delay(now, 0.1, cx.pev(), b.skill, 0.7);
    b.visible_enemies = [std::ptr::null_mut(); 16];
    b.visible_mates = [std::ptr::null_mut(); 16];
    b.num_visible_enemies = 0;
    let me = cx.edict;
    let eng = engine();
    for i in 1..=eng.max_clients() {
        // `UTIL_PlayerByIndex` + `IsPlayer()`: any player, one who left included.
        let o = eng.player(i);
        if !ts_server::rules::has_player(o) || o == me {
            continue;
        }
        let ov = o.v();
        if !ts_server::rules::alive(ov) || ov.effects & EF_NODRAW as i32 != 0 {
            continue;
        }
        let eye = add(ov.origin, ov.view_ofs); // EyePosition
        if !in_view_cone(cx.pev(), eye) || !visible(cx.pev(), me, eye) {
            continue;
        }
        let mate = ts_server::rules::teams::same_team(me, o); // PlayerRelationship == GR_TEAMMATE
        for j in 0..16 {
            if mate {
                if b.visible_mates[j].is_null() {
                    b.visible_mates[j] = o;
                    break;
                }
            } else if b.visible_enemies[j].is_null() {
                b.visible_enemies[j] = o;
                b.num_visible_enemies = b.num_visible_enemies.wrapping_add(1);
                break;
            }
        }
    }
}

// ---- buy (bit 0x10) ------------------------------------------------------------------------------

/// `CBotModuleBuy::Think` (0x1006d630): one roll `f` decides the loadout tier; no money involved.
/// Kevlar 70 % (`botarmor` other than 0 always, -1 never), a katana or knives, a pistol above
/// 0.1, an SMG in (0.4, 0.7), a rifle from 0.7. `BuyArmament` applies the list at the respawn.
/// The roll is a `float`; only the kevlar test compares it with a `float` constant (0.3f,
/// 0x10094d80), the others with `double`s ([`BuyTiers::of`]).
pub fn buy(b: &mut Brain, p: &mut Player, _cx: &mut Cx) {
    use shared::weapons::id::*;
    let eng = engine();
    let f = eng.random_float(0.0, 1.0);
    let tiers = BuyTiers::of(f);
    reset_armament(p);
    let armor = cvars::f(c"botarmor");
    if (tiers.kevlar || armor != 0.0) && armor != -1.0 {
        add_item(p, 0x10, 0, true); // kevlar
    }
    let k = eng.random_long(0, 5);
    if k == 5 {
        add_item(p, KATANA as i32, 0, false);
    } else if k != 0 {
        let n = if tiers.ten_knives { 10 } else { 1 };
        let t = eng.random_long(0, 2); // 0 seal knife, 1 combat knife, 2 both
        for _ in 0..n {
            if t == 0 || t == 2 {
                add_item(p, SEAL_KNIFE as i32, 0, false);
            }
            if t != 0 {
                add_item(p, KNIFE as i32, 0, false);
            }
        }
    }
    if tiers.pistol {
        let (id, akimbo, ammo) = match eng.random_long(0, 9) {
            0 => (CONTENDER, false, 13),
            1 => (GLOCK18, false, 1),
            2 => (MK23, true, 2),
            3 => (DEAGLE, false, 5),
            4 => (FIVESEVEN, true, 7),
            5 => (BERETTA, true, 1),
            6 => (GOLDEN_COLTS, false, 2),
            7 => (GLOCK20, false, 9),
            8 => (RUGER, false, 10),
            9 => (RAGING_BULL, false, 11),
            _ => (0, false, 0),
        };
        if id != 0 {
            buy_weapon(p, id, akimbo, ammo);
        }
    }
    if tiers.smg {
        let (id, akimbo, ammo) = match eng.random_long(1, 6) {
            1 => (UZI, true, 1),
            2 => (MP5SD, false, 1),
            3 => (MP5K, false, 1),
            4 => (SKORPION, true, 12),
            5 => (MP7, false, 7),
            6 => (UMP, false, 2),
            _ => (0, false, 0),
        };
        if id != 0 {
            buy_weapon(p, id, akimbo, ammo);
        }
    } else if tiers.rifle {
        let (id, ammo) = match eng.random_long(1, 11) {
            1 => (BENELLI, 3),
            2 => (USAS, 3),
            3 => (SPAS, 3),
            4 => (AK47, 6),
            5 => (M4, 4),
            6 => (AUG, 4),
            7 => (MOSSBERG, 3),
            8 => (M16, 4),
            9 => (SAWEDOFF, 3),
            10 => (BARRETT, 8),
            11 => (M60, 4),
            _ => (0, 0),
        };
        if id != 0 {
            buy_weapon(p, id, false, ammo);
        }
    }
    b.disable(bit::BUY);
}

/// The tiers one buy roll `f` selects in `CBotModuleBuy::Think` (0x1006d630). The kevlar test
/// compares with the `float` 0.3f (`fcomp DWORD [0x10094d80]` at 0x1006d655); the knife count,
/// pistol, SMG and rifle tests with the `double`s 0.75, 0.1, 0.4 and 0.7 (`fcomp QWORD` at
/// 0x1006d6b1, 0x1006d718, 0x1006d8d2, 0x1006d8e7, 0x1006da10), so `f` widens first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuyTiers {
    pub kevlar: bool,
    pub ten_knives: bool,
    pub pistol: bool,
    pub smg: bool,
    pub rifle: bool,
}

impl BuyTiers {
    pub fn of(f: f32) -> BuyTiers {
        let d = f64::from(f);
        BuyTiers { kevlar: f > 0.3, ten_knives: d > 0.75, pistol: d > 0.1, smg: d > 0.4 && d < 0.7, rifle: d >= 0.7 }
    }
}

// ---- look around (bit 0x80) ----------------------------------------------------------------------

/// `CBotModuleLookAround::Think` (0x1006dde0): look at a spot found by [`find_look_point`] every
/// `0.5 / slow / skill` s (at most 5 s).
pub fn look_around(b: &mut Brain, _p: &mut Player, cx: &mut Cx) {
    let now = cx.time();
    if b.look_next > now {
        return;
    }
    let point = find_look_point(cx);
    b.look_at(cx.pev(), point);
    b.look_next = delay(now, 0.5, cx.pev(), b.skill, 5.0);
}

/// `FindLookPoint` (0x1006dea0): from a random yaw, eight directions 45° apart, each with a random
/// pitch (`sin(pitch)` as z, not normalised), 1024 units from the gun position. The first one
/// that is clear wins, else the end point farthest from the origin.
fn find_look_point(cx: &mut Cx) -> [f32; 3] {
    let eng = engine();
    let me = cx.edict;
    let mut best = -1.0f32;
    let mut best_pos = [0.0f32; 3];
    let mut yaw = eng.random_float(0.0, 360.0);
    for _ in 0..8 {
        let pitch = eng.random_float(-30.0, 30.0);
        let rad = std::f64::consts::PI / 180.0;
        let d = [(yaw as f64 * rad).cos() as f32, (yaw as f64 * rad).sin() as f32, (pitch as f64 * rad).sin() as f32];
        let v = cx.pev();
        let src = gun_position(v);
        let tr = eng.trace_line(src, add(src, scale(d, 1024.0)), IGNORE_MONSTERS, me);
        if tr.flFraction == 1.0 {
            return tr.vecEndPos;
        }
        // The difference is stored as floats; the length stays on the x87 stack for the compare.
        let len = len_f32(sub(tr.vecEndPos, cx.pev().origin), order::ZYX);
        if len > best as f64 {
            best = len as f32;
            best_pos = tr.vecEndPos;
        }
        yaw += 45.0;
    }
    best_pos
}

#[cfg(test)]
mod tests {
    use super::BuyTiers;

    /// The `float` roll against mp.dll's `double` limits (0x1006d630): 0.1f and 0.4f lie above
    /// 0.1 and 0.4, 0.7f below 0.7.
    #[test]
    fn buy_tiers_compare_doubles() {
        let t = BuyTiers::of(0.1);
        assert!(t.pistol && !t.smg && !t.rifle);
        let t = BuyTiers::of(0.4);
        assert!(t.pistol && t.smg && !t.rifle);
        let t = BuyTiers::of(0.7);
        assert!(t.smg && !t.rifle && !t.ten_knives);
        let t = BuyTiers::of(0.70000005);
        assert!(!t.smg && t.rifle);
        // Kevlar compares with the float 0.3f: 0.3f itself is not above it.
        assert!(!BuyTiers::of(0.3).kevlar && BuyTiers::of(0.30000004).kevlar);
        assert!(BuyTiers::of(0.75000006).ten_knives && !BuyTiers::of(0.75).ten_knives);
    }
}
