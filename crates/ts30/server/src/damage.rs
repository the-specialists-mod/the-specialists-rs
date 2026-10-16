//! TS 3.0's bullets and what a hit does to a player: `UTIL_TraceLine_Expanded`, `TraceAttack`'s
//! multipliers and the kevlar in `TakeDamage`.

use goldsrc::edict::EdictPtr;
use goldsrc::server::{DONT_IGNORE_MONSTERS, engine};
use goldsrc::sys::{TraceResult, edict_t};
use shared::pm::{FpuMath, Msvc, ORDER_XYZ, ORDER_ZYX};
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

/// The direction `CBaseMonster::TakeDamage` (0xb2430) stores in `g_vecAttackDir`, from `to` (the
/// victim's centre) toward `from` (the inflictor's centre 10 units down), normalised twice in
/// mp.dll's arithmetic (issue #150): each inline `Normalize` keeps the length and `1/len` on the
/// FPU, the first sums `z² + y² + x²` (0x10013ff0..0x1001400c), the second `x² + y² + z²`
/// (0x10014065..0x10014081).
pub fn attack_dir(from: Vec3, to: Vec3) -> Vec3 {
    let d = [from[0] - to[0], from[1] - to[1], from[2] - to[2]];
    Msvc::vec_normalize(Msvc::vec_normalize(d, ORDER_ZYX), ORDER_XYZ)
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
        ts30_gameplay::tables::make_default();
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
        ts30_gameplay::tables::make_default();
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

#[cfg(test)]
mod attack_dir_tests {
    use super::attack_dir;
    use ts_server::damage::attack_dir_ends;

    fn f(b: [u32; 3]) -> [f32; 3] {
        b.map(f32::from_bits)
    }

    fn between(inflictor: ([f32; 3], [f32; 3]), victim: ([f32; 3], [f32; 3])) -> [f32; 3] {
        let (from, to) = attack_dir_ends((&inflictor.0, &inflictor.1), (&victim.0, &victim.1));
        attack_dir(from, to)
    }

    #[test]
    fn attack_dir_points_from_the_victim_to_below_the_inflictor() {
        // Victim box centred on the origin; inflictor centred 100 u ahead and 10 u up, so the
        // `- (0, 0, 10)` makes the direction level.
        let victim = ([-16.0, -16.0, -36.0], [16.0, 16.0, 36.0]);
        assert_eq!(between(([96.0, -4.0, 6.0], [104.0, 4.0, 14.0]), victim), [1.0, 0.0, 0.0]);
        // Same centre (after the 10 u drop): SDK `Normalize` gives straight up.
        assert_eq!(between(([-16.0, -16.0, -26.0], [16.0, 16.0, 46.0]), victim), [0.0, 0.0, 1.0]);
    }

    /// Where 2.1 rounds differently (#220's comment; 2.1: x = 0x3f0a2b69).
    #[test]
    fn attack_dir_where_2_1_differs() {
        let from = f([0x43e7_4f11, 0xc41a_fab3, 0x4532_987d]);
        let to = f([0xc52a_0810, 0x4506_6cf9, 0xc49d_cb04]);
        assert_eq!(attack_dir(from, to).map(f32::to_bits), [0x3f0a_2b68, 0xbef0_89a3, 0x3f32_d4c7]);
    }

    /// `CBaseMonster::TakeDamage`'s attack direction in mp.dll's arithmetic (issue #150); the old
    /// normalisations gave x = 0x3f229782.
    #[test]
    fn attack_dir_as_mp_dll() {
        let inf = (f([0xc372_b68a, 0xc3b4_a9e8, 0x435e_4780]), f([0xc2af_ec14, 0x43b1_95fc, 0x4378_de60]));
        let vic = (f([0xc397_0edc, 0xc398_98a1, 0xc210_e010]), f([0xc387_4705, 0x412a_7ae0, 0x43d6_4eae]));
        assert_eq!(between(inf, vic).map(f32::to_bits), [0x3f22_9781, 0x3f41_c1bd, 0x3e1d_e57d]);
    }
}
