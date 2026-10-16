//! TS 2.1's melee on the victim (issue #100 §3.2, checklist item 5): the blocks
//! (`FUN_10043720` katana, `FUN_10043870` knife, `FUN_100439e0` kung fu and the stock hit) and
//! the disarm of `CloseCombatAttack` (`FUN_10056f20`) and the delayed hit (`FUN_1005c680`).
//!
//! The gun's side (the attacks, the block windows they open) is `ts21_gameplay::shared::melee`.

use goldsrc::edict::EdictPtr;
use goldsrc::ext::entities::MsgWriterExt;
use goldsrc::server::engine;
use goldsrc::sys::{CHAN_BODY, edict_t};
use shared::protocol::hud_wire::PTakeDam;
use shared::weapons::gun::{MeleeKind, MeleeResult};
use shared::weapons::spread::{self, Vec3};
use ts_server::damage::{self, DamageInfo};
use ts_server::messages::{self, Msg};
use ts_server::player::Player;
use ts_server::weapons::{emit, send_kfu_power};
use ts21_gameplay::shared::gun::slow_div;
use ts21_gameplay::shared::melee::time_scale_inverse;

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
/// hit of `15 × (Kung Fu ? 2 : 1)` (the doubles 2.0/1.0 × 15) until `(float)(T·0.3 + time)` (`T`
/// unrounded, [`slow_div`]), with no 3 s cap, and the kung-fu block left alone (3.0 closes it).
pub fn backflip_kick(p: &mut Player, t: f64, time: f32) {
    let g = &mut p.inv.gun;
    g.melee_window = (t * 0.3 + time as f64) as f32;
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

/// The blocker's hold-up (0x10043c02..): `(block end − time) + min(3, 0.3·T)` on the FPU, `T`
/// the blocker's (unrounded, [`slow_div`]) and `time` the attacker's, stored as a `float`.
pub fn block_hold(block_end: f32, time: f32, t: f64) -> f32 {
    let w = t * 0.3;
    ((block_end as f64 - time as f64) + if 3.0 <= w { 3.0 } else { w }) as f32
}

/// A block timer (an end time) is running at `time`: `time < end` (2.1 tests `end <= time` or
/// `end < time || end == time` for "over").
pub fn running(end: f32, time: f32) -> bool {
    time < end
}

/// `FUN_10043720`: the victim's katana swing (`pl+0x72c`) blocks a katana.
pub fn katana_blocks(katana: f32, time: f32) -> bool {
    running(katana, time)
}

/// `FUN_10043870`: the victim's knife or katana swing (`pl+0x728`, `+0x72c`) blocks a knife.
pub fn knife_blocks(knife: f32, katana: f32, time: f32) -> bool {
    running(knife, time) || running(katana, time)
}

/// Who blocks a kung-fu hit in `FUN_100439e0`, all at the attacker's `time`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blocker {
    Victim,
    Attacker,
}

/// `FUN_100439e0`'s test: the victim while its kung-fu block (`+0x724`) runs; else the attacker
/// while its own runs and the victim is in a kung-fu attack's (source `+0x720` 0) delayed-hit
/// window (`+0x718`) that hasn't ended.
pub fn kungfu_blocker(v_block: f32, v_source: u8, v_window: f32, atk_block: f32, time: f32) -> Option<Blocker> {
    if running(v_block, time) {
        Some(Blocker::Victim)
    } else if running(atk_block, time) && v_source == 0 && running(v_window, time) {
        Some(Blocker::Attacker)
    } else {
        None
    }
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
/// drops its stored hit and keeps that. The block timers are end times, tested against
/// `gpGlobals->time` at the attacker's think.
pub fn melee_block(kind: MeleeKind, attacker: *mut edict_t, victim: *mut edict_t, damage: f32, atk_block: f32) -> Option<MeleeResult> {
    if !ts_server::rules::has_player(victim) {
        return None;
    }
    let time = engine().time();
    let blocked = match kind {
        MeleeKind::Katana => blade_block(attacker, victim, |p| katana_blocks(p.inv.gun.block_katana, time)),
        MeleeKind::Knife => blade_block(attacker, victim, |p| knife_blocks(p.inv.gun.block_knife, p.inv.gun.block_katana, time)),
        MeleeKind::KungFu | MeleeKind::ColdCock => kungfu_block(attacker, victim, damage, atk_block, time),
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
/// held up (`m_flNextAttack`, [`block_hold`]), its gun's timers are cleared (2.1 writes the relative hold-up into
/// the absolute timers, which have then passed), its bar pays the hit's full damage (not The
/// One's; [`block_bar_left`]), and both stored hits are dropped. kungfuhit.wav on the attacker.
fn kungfu_block(attacker: *mut edict_t, victim: *mut edict_t, damage: f32, atk_block: f32, time: f32) -> bool {
    let Some((v_block, v_source, v_window)) = gun_of(victim, |p| (p.inv.gun.block_kungfu, p.inv.gun.melee_source, p.inv.gun.melee_window))
    else {
        return false;
    };
    let blocker = match kungfu_blocker(v_block, v_source, v_window, atk_block, time) {
        Some(Blocker::Victim) => victim,
        Some(Blocker::Attacker) => attacker,
        None => return false,
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
    let t = slow_div(blocker.v().fuser1);
    let bar_left = gun_of(blocker, |p| {
        let g = &mut p.inv.gun;
        g.melee_damage = 0.0;
        g.next_attack = block_hold(g.block_kungfu, time, t);
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

/// The direction both hits of 2.1's `CTSStunt::CheckForBreakables` (`FUN_1005c680`), the delayed
/// melee hit and the dive into a breakable, pass to `TraceAttack`: the stunt's stored direction
/// (`CTSStunt+0x24`, copied at 0x1005ccb5 for the one call at 0x1005ccce), which `GoRoll`
/// (0x1005d600, `velocity.Normalize()`), the dives and the back moves write (3.0: `g_vecZero`).
pub fn delayed_hit_dir(stunt_dir: Vec3) -> Vec3 {
    stunt_dir
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #218: `CheckForBreakables`' hits carry the stunt direction (`CTSStunt+0x24`), not 3.0's
    /// `g_vecZero`.
    #[test]
    fn delayed_hits_carry_the_stunt_direction() {
        assert_eq!(delayed_hit_dir([0.6, -0.8, 0.0]), [0.6, -0.8, 0.0]);
    }

    #[test]
    fn kicks_cost_no_power() {
        let mut p = Player::default();
        p.inv.gun.next_attack = -0.001;
        p.inv.gun.melee_bar = 40.0;
        jump_kick_hit(&mut p, 1.0);
        assert!((p.inv.gun.next_attack - 0.7).abs() < 1e-6 && p.inv.gun.melee_bar == 40.0);
        jump_kick_hit(&mut p, 1.0);
        assert!((p.inv.gun.next_attack - 0.7).abs() < 1e-6, "only a run-out one");
        p.inv.gun.block_kungfu = 10.2;
        backflip_kick(&mut p, 2.0, 10.0);
        let g = &p.inv.gun;
        assert_eq!((g.melee_window, g.melee_damage, g.melee_source, g.block_kungfu, g.melee_bar), (10.6, 15.0, 0, 10.2, 40.0));
        p.kungfu = true;
        backflip_kick(&mut p, 20.0, 10.0);
        assert_eq!((p.inv.gun.melee_window, p.inv.gun.melee_damage), (16.0, 30.0), "no cap");
        // `(float)(T·0.3 + time)` rounded once (#202): at time scale 0.45 and 1 s that is
        // 1.6666666, where the duration stored first gives 1.6666667.
        backflip_kick(&mut p, slow_div(0.45), 1.0);
        assert_eq!(p.inv.gun.melee_window, 1.666_666_6);
        assert_ne!(1.0 + (slow_div(0.45) * 0.3) as f32, 1.666_666_6);
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
        assert!((block_hold(10.1, 10.0, 1.0) - 0.4).abs() < 1e-6);
        assert_eq!(block_hold(10.0, 10.0, 100.0), 3.0);
    }

    /// #202: the blocks are end times tested at the attacker's time: running while `time < end`
    /// (over at `end == time`), whatever the victim's last PreThink saw.
    #[test]
    fn blocks_are_end_times_at_the_attackers_time() {
        assert!(katana_blocks(10.5, 10.4));
        assert!(!katana_blocks(10.5, 10.5), "over at its end");
        assert!(!katana_blocks(0.0, 10.0), "none");
        assert!(knife_blocks(10.5, 0.0, 10.4) && knife_blocks(0.0, 10.5, 10.4));
        assert!(!knife_blocks(10.4, 10.4, 10.4));
        let b = |v_block, v_source, v_window, atk_block, time| kungfu_blocker(v_block, v_source, v_window, atk_block, time);
        assert_eq!(b(10.5, 0, 0.0, 0.0, 10.4), Some(Blocker::Victim));
        assert_eq!(b(10.5, 0, 0.0, 0.0, 10.5), None, "the victim's block is over at its end");
        // The attacker's own block against a victim in a kung-fu window that hasn't ended.
        assert_eq!(b(0.0, 0, 10.6, 10.5, 10.4), Some(Blocker::Attacker));
        assert_eq!(b(0.0, 1, 10.6, 10.5, 10.4), None, "a blade's window");
        assert_eq!(b(0.0, 0, 10.4, 10.5, 10.4), None, "the window ends at the time");
        assert_eq!(b(0.0, 0, 10.6, 10.4, 10.4), None, "the attacker's block ended");
        // An attacker whose time is ahead of the victim's last PreThink sees the block ended.
        assert_eq!(b(10.5, 0, 0.0, 0.0, 10.55), None);
    }

    /// The hold-up is `(end − time) + min(3, 0.3·T)` on the FPU, stored once; a block that ended
    /// before the attacker's time shortens it.
    #[test]
    fn block_hold_from_the_end_time() {
        let (end, time) = (f32::from_bits(0x4479_8a3d), 1000.0f32);
        assert_eq!(block_hold(end, time, 1.0), ((end as f64 - 1000.0) + 0.3) as f32);
        assert!((block_hold(9.9, 10.0, 1.0) - 0.2).abs() < 1e-6);
        assert_eq!(block_hold(10.0, 10.0, slow_div(0.3)), (slow_div(0.3) * 0.3) as f32);
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
