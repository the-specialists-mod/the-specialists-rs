//! TS 2.1's melee on the victim (issue #100 §3.2, checklist item 5): the blocks
//! (`FUN_10043720` katana, `FUN_10043870` knife, `FUN_100439e0` kung fu and the stock hit) and
//! the disarm of `CloseCombatAttack` (`FUN_10056f20`) and the delayed hit (`FUN_1005c680`).
//!
//! The gun's side (the attacks, the block windows they open) is `ts21_gameplay::shared::melee`.

use goldsrc::edict::EdictPtr;
use goldsrc::ext::entities::MsgWriterExt;
use goldsrc::sys::{CHAN_BODY, edict_t};
use shared::protocol::hud_wire::PTakeDam;
use shared::weapons::gun::{MeleeKind, MeleeResult};
use shared::weapons::spread::{self, Vec3};
use ts_server::damage::{self, DamageInfo};
use ts_server::messages::{self, Msg};
use ts_server::player::Player;
use ts_server::weapons::{emit, send_kfu_power};
use ts21_gameplay::shared::melee::{time_scale_inverse, whiff_window};

/// The kung-fu block's and the stock hit's damage bits (pushed at 0x10043b30).
const CLOSE_COMBAT_BITS: i32 = 0x0400_1080;

/// `CloseCombatAttack` / the delayed hit: the victim drops its gun at once (`FUN_10056080(-2)`, no
/// disarm count) when the hit deals more than 15 (0x10083008) and the victim isn't The One
/// (`pl+0x731`). The spawn protection test stays the caller's.
pub fn melee_disarm(damage: f32, victim_is_the_one: bool) -> Option<f32> {
    (damage > 15.0 && !victim_is_the_one).then_some(f32::MAX)
}

/// A jump kick that hit (`FUN_1005cd00` 0x1005d040..): no kung-fu power spent, and a run-out
/// `m_flNextAttack` becomes `0.7·T` (3.0: 0.35).
pub fn jump_kick_hit(p: &mut Player, t: f32) {
    if p.inv.gun.next_attack < 0.0 {
        p.inv.gun.next_attack = (t as f64 * 0.7) as f32;
    }
}

/// The backflip kick (`GoWheel` `FUN_1005d0e0` 0x1005d2xx): no kung-fu power spent; a delayed
/// hit of `15 × (Kung Fu ? 2 : 1)` (the doubles 2.0/1.0 × 15) for `0.3·T`, with no 3 s cap, and
/// the kung-fu block left alone (3.0 closes it).
pub fn backflip_kick(p: &mut Player, t: f32) {
    let g = &mut p.inv.gun;
    g.melee_window = (t as f64 * 0.3) as f32;
    g.melee_damage = ((if p.kungfu { 2.0 } else { 1.0 }) * 15.0f64) as f32;
    g.melee_source = 0;
}

/// A player's `T` (`FUN_100551c0`) from its time scale.
pub fn player_t(fuser1: f32) -> f32 {
    time_scale_inverse(shared::weapons::spread::fixed_slow_amt(fuser1))
}

/// What a kung-fu block lets through to the blocker: `(1 − bar·0.01) × dmg` (the double 0.01),
/// stored as a float. A full bar blocks everything.
pub fn blocked_damage(bar: f32, damage: f32) -> f32 {
    ((1.0 - bar as f64 * 0.01) * damage as f64) as f32
}

/// The blocker's bar after a kung-fu block of a `damage` hit: the full hit, not what got through
/// (0x10043c4c: `bar - [esp+0x34]`, the damage argument; `through` is in the victim slot at
/// `+0x30`), stopped at 0.
pub fn block_bar_left(bar: f32, damage: f32) -> f32 {
    let b = bar - damage;
    if b < 0.0 { 0.0 } else { b }
}

/// The blocker's hold-up: its block time left plus `min(3, 0.3·T)` (0x10043c02..).
pub fn block_hold(block_left: f32, t: f32) -> f32 {
    (block_left as f64 + whiff_window(t) as f64) as f32
}

/// The two players' midpoint, 20 units up.
fn midpoint(a: *mut edict_t, b: *mut edict_t) -> Vec3 {
    let mut m = spread::scale(spread::add(a.v().origin, b.v().origin), 0.5);
    m[2] += 20.0;
    m
}

/// A player's gun state, if it has one.
fn gun_of<R>(e: *mut edict_t, f: impl FnOnce(&mut Player) -> R) -> Option<R> {
    ts_server::world::with::<Player, _>(e, f)
}

/// The victim's block (`ServerRules::melee_block`). The attacker is lent to its slot (a direct
/// hit runs inside its weapon frame, the delayed hit is lent around this), so both sides are
/// changed here. The result only carries the attacker's `next_attack` as it now is: the gun
/// drops its stored hit and keeps that.
pub fn melee_block(kind: MeleeKind, attacker: *mut edict_t, victim: *mut edict_t, damage: f32, atk_block: f32) -> Option<MeleeResult> {
    if !ts_server::rules::has_player(victim) {
        return None;
    }
    let blocked = match kind {
        MeleeKind::Katana => blade_block(attacker, victim, |p| p.inv.gun.block_katana > 0.0),
        MeleeKind::Knife => blade_block(attacker, victim, |p| p.inv.gun.block_knife > 0.0 || p.inv.gun.block_katana > 0.0),
        MeleeKind::KungFu | MeleeKind::ColdCock => kungfu_block(attacker, victim, damage, atk_block),
    };
    if !blocked {
        return None;
    }
    let next_attack = gun_of(attacker, |p| {
        p.inv.gun.melee_damage = 0.0;
        p.inv.gun.next_attack
    })
    .unwrap_or(0.0);
    Some(MeleeResult::Blocked { next_attack, bar_cost: 0.0 })
}

/// `FUN_10043720` / `FUN_10043870`: a victim whose own swing's window is open blocks a blade:
/// sparks (`TSFrags` type 2) between the two and the attacker's knife_hit.wav. Nothing else: no
/// bar, no hold-up.
fn blade_block(attacker: *mut edict_t, victim: *mut edict_t, blocks: impl FnOnce(&mut Player) -> bool) -> bool {
    if !gun_of(victim, blocks).unwrap_or(false) {
        return false;
    }
    let pos = midpoint(attacker, victim);
    let angles = attacker.v().angles;
    messages::pvs(Msg::TsFrags, pos).vec3(pos).vec3(angles).byte(2).end();
    emit(attacker, CHAN_BODY, c"weapons/knife/knife_hit.wav", 1.0, 0.8, 100);
    true
}

/// `FUN_100439e0`: the victim blocks while its kung-fu block (`pl+0x724`) runs; or the attacker
/// does, while its own block runs and the victim is in a kung-fu attack's delayed-hit window. The
/// blocker takes `(1 − bar/100)` of the hit from the attacker (`TraceAttack`, close combat), is
/// held up (`m_flNextAttack`), its gun's timers are cleared (2.1 writes the relative hold-up into
/// the absolute timers, which have then passed), its bar pays the hit's full damage (not The
/// One's; [`block_bar_left`]), and both stored hits are dropped. kungfuhit.wav on the attacker.
fn kungfu_block(attacker: *mut edict_t, victim: *mut edict_t, damage: f32, atk_block: f32) -> bool {
    let Some((v_block, v_source, v_window)) = gun_of(victim, |p| (p.inv.gun.block_kungfu, p.inv.gun.melee_source, p.inv.gun.melee_window))
    else {
        return false;
    };
    let blocker = if v_block > 0.0 {
        victim
    } else if atk_block > 0.0 && v_source == 0 && v_window > 0.0 {
        attacker
    } else {
        return false;
    };
    let Some(bar) = gun_of(blocker, |p| p.inv.gun.melee_bar) else { return false };
    let through = blocked_damage(bar, damage);
    let pos = midpoint(attacker, victim);
    let m = PTakeDam {
        x: pos[0],
        y: pos[1],
        z: pos[2],
        dx: 0.0,
        dy: 0.0,
        dz: 0.0,
        amount: ts_server::player::damage::ptakedam_amount(through, CLOSE_COMBAT_BITS, false),
    };
    ts_server::rules::msg::pvs(Msg::PTakeDam, pos, |w| m.write(w));
    let mut info = DamageInfo::new(attacker, attacker, through, CLOSE_COMBAT_BITS).traced();
    info.dir = spread::sub(victim.v().origin, attacker.v().origin);
    info.pos = blocker.v().origin;
    damage::glast::close_combat(|| damage::apply(blocker, &info));
    emit(attacker, CHAN_BODY, c"player/kungfuhit.wav", 1.0, 0.8, 100);
    gun_of(victim, |p| p.inv.gun.melee_damage = 0.0);
    let t = time_scale_inverse(shared::weapons::spread::fixed_slow_amt(blocker.v().fuser1));
    let bar_left = gun_of(blocker, |p| {
        let g = &mut p.inv.gun;
        g.melee_damage = 0.0;
        g.next_attack = block_hold(g.block_kungfu, t);
        g.next_primary = 0.0;
        g.next_secondary = 0.0;
        if p.ts.is_the_one {
            return None;
        }
        g.melee_bar = block_bar_left(g.melee_bar, damage);
        Some(g.melee_bar)
    })
    .flatten();
    if let Some(b) = bar_left {
        send_kfu_power(blocker, b);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kicks_cost_no_power() {
        let mut p = Player::default();
        p.inv.gun.next_attack = -0.001;
        p.inv.gun.melee_bar = 40.0;
        jump_kick_hit(&mut p, 1.0);
        assert!((p.inv.gun.next_attack - 0.7).abs() < 1e-6 && p.inv.gun.melee_bar == 40.0);
        jump_kick_hit(&mut p, 1.0);
        assert!((p.inv.gun.next_attack - 0.7).abs() < 1e-6, "only a run-out one");
        p.inv.gun.block_kungfu = 0.2;
        backflip_kick(&mut p, 2.0);
        let g = &p.inv.gun;
        assert_eq!((g.melee_window, g.melee_damage, g.melee_source, g.block_kungfu, g.melee_bar), (0.6, 15.0, 0, 0.2, 40.0));
        p.kungfu = true;
        backflip_kick(&mut p, 20.0);
        assert_eq!((p.inv.gun.melee_window, p.inv.gun.melee_damage), (6.0, 30.0), "no cap");
    }

    #[test]
    fn disarm_needs_more_than_15() {
        assert_eq!(melee_disarm(15.0, false), None);
        assert_eq!(melee_disarm(15.5, false), Some(f32::MAX), "at once");
        assert_eq!(melee_disarm(90.0, true), None, "not The One");
    }

    #[test]
    fn a_block_lets_through_what_the_bar_lacks() {
        assert_eq!(blocked_damage(100.0, 40.0), 0.0);
        assert_eq!(blocked_damage(0.0, 40.0), 40.0);
        assert!((blocked_damage(75.0, 40.0) - 10.0).abs() < 1e-5);
        assert!((block_hold(0.1, 1.0) - 0.4).abs() < 1e-6);
        assert_eq!(block_hold(0.0, 100.0), 3.0);
    }

    /// The bar pays the whole hit (#136): a 40 kick blocked with 75 costs 40, not the 10 let
    /// through; a full bar still pays.
    #[test]
    fn a_block_costs_the_bar_the_full_hit() {
        assert_eq!(block_bar_left(75.0, 40.0), 35.0);
        assert_eq!(block_bar_left(100.0, 40.0), 60.0);
        assert_eq!(block_bar_left(30.0, 40.0), 0.0);
    }
}
