//! TS 3.0's bullets and what a hit does to a player: `UTIL_TraceLine_Expanded`, `TraceAttack`'s
//! multipliers and the kevlar in `TakeDamage`.

use goldsrc::edict::EdictPtr;
use goldsrc::server::{DONT_IGNORE_MONSTERS, engine};
use goldsrc::sys::{TraceResult, edict_t};
use shared::weapons::spread::Vec3;
use shared::weapons::tables;
use ts_server::damage::{bits, hitgroup};
use ts_server::player::damage::{HitDamage, PlayerHit, hitgroup_scale, kevlar_absorb};

/// `UTIL_TraceLine_Expanded` 0x6f4f4: grows every player's absolute box by 32 units for the
/// trace so hitboxes outside the bounding box (arms, head while diving) can be hit.
pub fn trace_expanded(start: Vec3, end: Vec3, skip: *mut edict_t) -> TraceResult {
    let e = engine();
    let n = e.max_clients();
    let mut saved: Vec<(*mut edict_t, [f32; 3], [f32; 3])> = Vec::with_capacity(n as usize);
    for i in 1..=n {
        let p = e.player(i);
        if p.is_null() {
            continue;
        }
        let v = p.v();
        saved.push((p, v.absmin, v.absmax));
        for k in 0..3 {
            v.absmin[k] -= 32.0;
            v.absmax[k] += 32.0;
        }
    }
    let tr = e.trace_line(start, end, DONT_IGNORE_MONSTERS, skip);
    for (p, lo, hi) in saved {
        let v = p.v();
        v.absmin = lo;
        v.absmax = hi;
    }
    tr
}

/// `TraceAttack`'s multipliers (0x901cc): stunting ×0.7, bullets in the dodge window ×0.7,
/// then [`hitgroup_scale`]; a bullet or thrown hit in the head is a headshot. Each ×0.7 multiplies
/// by the `double` and stores the `float` (0x90212/0x90241; mp.dll 0x1005efe9/0x1005f00f, issue
/// #138).
pub fn hit_damage(hit: &PlayerHit) -> HitDamage {
    let c = &tables().combat;
    let mut amount = hit.damage;
    if hit.stunting {
        amount = (amount as f64 * c.stunt_damage_taken) as f32;
    }
    if hit.dodging && hit.bits & bits::BULLET != 0 {
        amount = (amount as f64 * c.stunt_damage_taken) as f32;
    }
    let headshot = hit.hitgroup == hitgroup::HEAD && hit.bits & (bits::BULLET | bits::THROWN) != 0;
    amount *= hitgroup_scale(hit.hitgroup, hit.bits);
    HitDamage { amount, headshot, record_headshot: headshot }
}

/// Damage that reaches `CBaseMonster::TakeDamage` after kevlar: `CBasePlayer::TakeDamage`
/// computes `dmg - min(absorb, 1) * dmg` on the x87 stack (0x8fc38), stores it as a float, then
/// truncates it to an int (0x8fc5f) and passes it back as a float. The product of two floats is
/// exact in `f64`, so only the subtraction can round differently from the 80-bit original.
pub fn after_kevlar(amount: f32, armor: f32, group: i32, dmg_bits: i32, absorb: f32) -> f32 {
    let absorbed = kevlar_absorb(armor, group, dmg_bits, absorb);
    let d = amount as f64;
    ((d - absorbed as f64 * d) as f32).trunc()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Ts30Server;
    use shared::pm::{Arith, Ext};
    use ts_server::gameplay::ServerRules;
    use ts_server::weapons::ThrownBlade;

    /// `armor_absorb` as `CTSGameRules::Init` sets it.
    const ABSORB: f32 = ts_server::rules::state::INIT_ARMOR_ABSORB;

    /// The stunt and dodge ×0.7 multiply by the `double` 0.7 (issue #138): 9 × 0.7 is
    /// 6.3000002 (0x40c9999a), where the `float` 0.7 gave 6.2999997 (0x40c99999).
    #[test]
    fn stunt_and_dodge_take_the_double_seven_tenths() {
        let hit = |stunting, dodging| {
            hit_damage(&PlayerHit { damage: 9.0, bits: bits::BULLET, hitgroup: hitgroup::CHEST, stunting, dodging }).amount
        };
        assert_eq!(hit(true, false).to_bits(), 0x40c9_999a);
        assert_eq!(hit(false, true).to_bits(), 0x40c9_999a);
        assert_eq!(hit(true, true), ((9.0f64 * 0.7) as f32 as f64 * 0.7) as f32, "rounded after each");
        assert_eq!(hit(false, false), 9.0);
    }

    #[test]
    fn kevlar_damage_truncates() {
        // 30 × (1 − 0.2125) = 23.625: TS does 23.
        assert_eq!(after_kevlar(30.0, 100.0, hitgroup::CHEST, bits::BULLET, ABSORB), 23.0);
        assert_eq!(after_kevlar(30.9, 0.0, hitgroup::CHEST, bits::BULLET, ABSORB), 30.0, "no armour still truncates");
        assert_eq!(after_kevlar(0.5, 0.0, hitgroup::HEAD, bits::BULLET, ABSORB), 0.0);
    }

    fn hit(damage: f32, bits: i32, hitgroup: i32) -> PlayerHit {
        PlayerHit { damage, bits, hitgroup, stunting: false, dodging: false }
    }

    /// The damage hooks' defaults are TS 3.0's.
    #[test]
    fn damage_defaults_are_3_0() {
        let r: &dyn ServerRules = &Ts30Server;
        // Truncated, not rounded (issue #80).
        assert_eq!(r.bullet_range_index(Ext::from_f64(9.99)), 9);
        assert_eq!(r.bullet_range_index(Ext::from_f64(10.0)), 10);
        let mut acc = 1.0;
        assert_eq!(r.bullet_damage_multiplier(&mut acc, None), 1.0);
        assert_eq!(r.bullet_damage_multiplier(&mut acc, None), 1.0, "nothing accumulates");
        assert_eq!(r.bullet_damage_multiplier(&mut acc, Some(3.0)), 3.0);
        assert_eq!(r.bullet_damage(1, 5, 33.0, 0.5, false), 16.5);
        assert_eq!(r.pierce_step(10.0), 20.0);
        assert_eq!(r.pierce_step(30.0), 48.0);
        assert_eq!(r.weapon_realbullet(2), 2);
        for arith in [Arith::Gcc, Arith::Msvc] {
            assert_eq!(r.bullet_velocity_mult(arith, 0.35, true), shared::weapons::spread::bullet_velocity_mult(arith, 0.35, true));
        }
        assert_eq!(r.damage_punch(), 0.0);
        assert!(!r.zero_delayed_hit_reaches_player(), "a spent delayed hit goes straight to the 0-damage hit");
        assert_eq!(r.grenade_explosion(), (110.0, 300.0, 1024.0));
        assert!(r.explosion_pushes_world_guns());
        assert_eq!(r.thrown_blade(6), ThrownBlade { speed: 1900.0, gravity: 0.75, bits: 0x0800_1000 });
        assert_eq!(r.thrown_blade(7), ThrownBlade { speed: 1900.0, gravity: 0.3, bits: 0x0800_1000 });
        assert_eq!(r.thrown_blade(8), ThrownBlade { speed: 1000.0, gravity: 0.85, bits: 0x0800_1000 });
        assert_eq!([6, 7, 8].map(|t| r.thrown_blade_damage(t, [0.0; 3], [2000.0, 0.0, 0.0])), [100.0, 50.0, 150.0]);
    }

    #[test]
    fn hit_defaults_are_3_0() {
        let r: &dyn ServerRules = &Ts30Server;
        let h = r.player_hit(&hit(30.0, bits::BULLET, hitgroup::HEAD));
        assert_eq!(h, HitDamage { amount: 60.0, headshot: true, record_headshot: true });
        assert_eq!(r.player_hit(&hit(10.0, 0x0200_1002, hitgroup::HEAD)).amount, 15.0, "shotgun ×1.5");
        assert!(r.player_hit(&hit(100.0, 0x0800_1000, hitgroup::HEAD)).headshot, "thrown");
        assert_eq!(r.player_hit(&hit(30.0, bits::BULLET, hitgroup::RIGHTLEG)).amount, 15.0);
        let mut d = hit(100.0, bits::BULLET, hitgroup::CHEST);
        d.dodging = true;
        assert!((r.player_hit(&d).amount - 70.0).abs() < 1e-4, "dodge ×0.7 for bullets");
        d.bits = bits::CLUB;
        assert_eq!(r.player_hit(&d).amount, 100.0, "not for melee");
        d.stunting = true;
        assert!((r.player_hit(&d).amount - 70.0).abs() < 1e-4);
    }
}
