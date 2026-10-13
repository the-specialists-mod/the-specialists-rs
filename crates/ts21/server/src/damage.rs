//! TS 2.1's bullet damage and what a hit does to a player (issue #100 §3.1, checklist item 4).
//!
//! * `CBasePlayer::ShootBulletsPlayer` (0x1003fd40) and the simulated bullets
//!   (`CBulletManager` update `FUN_1003f910`, think `FUN_1003f770`).
//! * `CBasePlayer::TraceAttack` (0x10053310) and `CBasePlayer::TakeDamage` (0x10052cf0).

use ts_server::damage::{bits, hitgroup};
use ts_server::player::damage::{HitDamage, PlayerHit};

/// The random part of a bullet's damage multiplier (0x100403e9..0x10040425 and 0x1003fb03..):
/// with `damagemult` in force the multiplier restarts from it, then every hit adds
/// `RANDOM_FLOAT(0, 0.05)` (`random`). The multiplier lives across the pellets of one
/// `ShootBulletsPlayer` call (reset to 1.0 only at its start), so a shotgun's later pellets hit
/// harder: pellet `k` gets `1 + Σ U(0, 0.05)` over the pellets that hit before it and itself. The
/// float add is done on the x87 stack and stored back as a float.
pub fn accumulate(acc: &mut f32, cheat: Option<f32>, random: f32) -> f32 {
    if let Some(m) = cheat {
        *acc = m;
    }
    *acc = (random as f64 + *acc as f64) as f32;
    *acc
}

/// A bullet's damage before the random factor, in 2.1's order of operations (0x100403c2 and,
/// for a simulated bullet, 0x1003faf5): the ammo's integer base damage (`ammo.dmg`), the weapon's
/// multiplier as the gun passed it (with the silencer's `× 0.8`), and the falloff. A hitscan
/// pellet computes `base × falloff × mult`, a simulated bullet `base × mult × falloff`, each on the
/// x87 stack and stored once. The gun hands over `base × mult` (`Shot::damage`); the multiplier is
/// whichever of the weapon's two reproduces it. Ammo 2.1 doesn't have keeps the plain product.
pub fn bullet_damage(ammo: u8, weapon: u8, damage: f32, falloff: f32, simulated: bool) -> f32 {
    use ts21_gameplay::shared::tables;
    let base = tables::ammo_damage(ammo);
    if base == 0 {
        return falloff * damage;
    }
    let b = base as f32;
    let m = tables::damage_multiplier(weapon);
    let silenced = (m as f64 * 0.8) as f32;
    let mult = if b * m == damage {
        m
    } else if b * silenced == damage {
        silenced
    } else {
        damage / b
    };
    let (b, f, mu) = (base as f64, falloff as f64, mult as f64);
    (if simulated { b * mu * f } else { b * f * mu }) as f32
}

/// The range-falloff index: MSVC's `__ftol` truncates (0x100403a9, 0x1003ffe0).
pub fn range_index(scaled: f32) -> i32 {
    scaled as i32
}

/// The penetration step (0x10040513..): `min(left, 48)`, the same as the simulated bullets'.
pub fn pierce_step(left: f32) -> f32 {
    if 48.0 <= left { 48.0 } else { left }
}

/// `GetBulletVelocityMult`'s 2.1 counterpart in the bullet update (0x1003f941..0x1003f993):
/// the shooter's time scale (its slow-motion amount for others `pl+0x168`, else its own
/// `fuser1`, 1 for normal speed), ×0.2 with bullet time (`pl+0x15d`). No `1 − s²` curve.
pub fn bullet_velocity_mult(slowmul: f32, bullet_time: bool) -> f32 {
    if bullet_time { (slowmul as f64 * 0.200_000_002_980_232_24) as f32 } else { slowmul }
}

/// `CBasePlayer::TraceAttack` (0x10053310): stunting (`iuser4 & 0x10c10`) ×0.7, no dodge window;
/// a head hit by anything with `DMG_BULLET` doubles (shotgun pellets too, thrown blades never),
/// legs halve. The headshot only picks the hit sound: 2.1 keeps no `glast_headshot` and has no
/// headshot kill message.
pub fn player_hit(hit: &PlayerHit) -> HitDamage {
    let mut amount = hit.damage;
    if hit.stunting {
        amount = (amount as f64 * 0.7) as f32;
    }
    let mut headshot = false;
    match hit.hitgroup {
        hitgroup::HEAD if hit.bits & bits::BULLET != 0 => {
            amount += amount;
            headshot = true;
        }
        hitgroup::LEFTLEG | hitgroup::RIGHTLEG => amount = (amount as f64 * 0.5) as f32,
        _ => {}
    }
    HitDamage { amount, headshot, record_headshot: false }
}

/// `CBasePlayer::TakeDamage` (0x10052cf0, 0x10052e07..0x10052e8f): armour covers chest, stomach
/// and arms (hitgroups 2..5) against everything but `0x40e0` (fall, blast, club, drown). A hit no
/// bigger than the armour uses it up by the damage and halves; a bigger one empties it and deals
/// `dmg − (dmg − armour)`, i.e. exactly the armour that was left. That is what shipped (the
/// intent was surely `dmg − armour` plus half). The result is truncated to an int, as in 3.0.
pub fn armor_damage(armor: &mut f32, amount: f32, group: i32, dmg_bits: i32) -> f32 {
    let mut d = amount;
    if *armor != 0.0 && dmg_bits & 0x40e0 == 0 && (hitgroup::CHEST..=hitgroup::RIGHTARM).contains(&group) {
        if d <= *armor {
            *armor -= d;
            d = (d as f64 * 0.5) as f32;
        } else {
            let (a, x) = (*armor as f64, d as f64);
            d = (x - (x - a)) as f32;
            *armor = 0.0;
        }
    }
    d.trunc()
}

/// `TakeDamage`'s view punch on the victim (after the suit reports): `punchangle.x -= 3`.
pub const DAMAGE_PUNCH: f32 = 3.0;

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(damage: f32, bits: i32, hitgroup: i32) -> PlayerHit {
        PlayerHit { damage, bits, hitgroup, stunting: false, dodging: false }
    }

    #[test]
    fn the_random_factor_accumulates_over_a_shot() {
        let mut acc = 1.0;
        assert_eq!(accumulate(&mut acc, None, 0.05), 1.05);
        let second = 1.05f32 + 0.05f32;
        assert_eq!(accumulate(&mut acc, None, 0.05), second, "the second pellet gets both rolls");
        assert_eq!(accumulate(&mut acc, None, 0.0), second, "it never shrinks");
        // `damagemult` (with sv_cheats) restarts it on every hit.
        assert_eq!(accumulate(&mut acc, Some(2.0), 0.01), 2.01);
        assert_eq!(accumulate(&mut acc, Some(2.0), 0.01), 2.01);
    }

    #[test]
    fn bullet_damage_uses_2_1s_factors() {
        use ts21_gameplay::shared::tables;
        // The Glock-18 (id 1) with its own ammo: base × mult, with or without the silencer.
        let ammo = tables::tables().weapon(1).unwrap().ammo;
        let (b, m) = (tables::ammo_damage(ammo) as f32, tables::damage_multiplier(1));
        assert_eq!(bullet_damage(ammo, 1, b * m, 1.0, false), b * m);
        let sm = (m as f64 * 0.8) as f32;
        let d = bullet_damage(ammo, 1, b * sm, 0.5, false);
        assert!((d - b * sm * 0.5).abs() < 1e-4, "{d}");
        // An ammo type 2.1 lacks: the plain product.
        assert_eq!(bullet_damage(200, 1, 30.0, 0.5, true), 15.0);
    }

    #[test]
    fn bullet_constants() {
        assert_eq!(range_index(9.99), 9, "truncated, not rounded");
        assert_eq!(range_index(10.5), 10);
        assert_eq!(pierce_step(10.0), 10.0, "3.0 doubles it");
        assert_eq!(pierce_step(60.0), 48.0);
        assert_eq!(bullet_velocity_mult(1.0, false), 1.0);
        assert_eq!(bullet_velocity_mult(0.35, false), 0.35);
        assert!((bullet_velocity_mult(0.35, true) - 0.07).abs() < 1e-6);
    }

    #[test]
    fn hitgroups() {
        let h = player_hit(&hit(30.0, 0x1002, hitgroup::HEAD));
        assert_eq!(h, HitDamage { amount: 60.0, headshot: true, record_headshot: false });
        // Shotgun pellets double too (3.0: ×1.5).
        assert_eq!(player_hit(&hit(10.0, 0x0200_1002, hitgroup::HEAD)).amount, 20.0);
        // Thrown blades (0x1080) aren't headshots in 2.1.
        assert_eq!(player_hit(&hit(80.0, 0x1080, hitgroup::HEAD)), HitDamage { amount: 80.0, headshot: false, record_headshot: false });
        assert_eq!(player_hit(&hit(30.0, 0x1002, hitgroup::LEFTLEG)).amount, 15.0);
        assert_eq!(player_hit(&hit(30.0, 0x1002, hitgroup::CHEST)).amount, 30.0);
        // Stunting ×0.7; the dodge window does nothing.
        let mut s = hit(100.0, 0x1002, hitgroup::HEAD);
        s.stunting = true;
        assert_eq!(player_hit(&s).amount, 140.0);
        s.stunting = false;
        s.dodging = true;
        assert_eq!(player_hit(&s).amount, 200.0);
    }

    #[test]
    fn armour_is_used_up() {
        let mut a = 100.0;
        assert_eq!(armor_damage(&mut a, 40.0, hitgroup::CHEST, 0x1002), 20.0);
        assert_eq!(a, 60.0);
        // Equal: still the "fits" branch.
        assert_eq!(armor_damage(&mut a, 60.0, hitgroup::STOMACH, 0x1002), 30.0);
        assert_eq!(a, 0.0);
        // No armour left: full damage.
        assert_eq!(armor_damage(&mut a, 60.0, hitgroup::STOMACH, 0x1002), 60.0);
    }

    #[test]
    fn a_hit_bigger_than_the_armour_deals_the_armour() {
        let mut a = 100.0;
        assert_eq!(armor_damage(&mut a, 150.0, hitgroup::CHEST, 0x1002), 100.0);
        assert_eq!(a, 0.0);
        let mut a = 20.0;
        assert_eq!(armor_damage(&mut a, 90.0, hitgroup::LEFTARM, 0x1002), 20.0);
        let mut a = 7.5;
        assert_eq!(armor_damage(&mut a, 30.0, hitgroup::RIGHTARM, 0x1002), 7.0, "truncated");
    }

    #[test]
    fn armour_coverage() {
        let mut a = 100.0;
        assert_eq!(armor_damage(&mut a, 40.0, hitgroup::HEAD, 0x1002), 40.0, "head");
        assert_eq!(armor_damage(&mut a, 40.0, hitgroup::LEFTLEG, 0x1002), 40.0, "legs");
        assert_eq!(armor_damage(&mut a, 40.0, hitgroup::CHEST, 0x40), 40.0, "blast");
        assert_eq!(armor_damage(&mut a, 40.0, hitgroup::CHEST, 0x80), 40.0, "club");
        assert_eq!(a, 100.0);
        // Slashing (shrapnel, 0x1004) is covered in 2.1 (3.0 exempts 0x4).
        assert_eq!(armor_damage(&mut a, 100.0, hitgroup::CHEST, 0x1004), 50.0);
        assert_eq!(a, 0.0);
        assert_eq!(armor_damage(&mut a, 33.9, hitgroup::GENERIC, 0x1002), 33.0, "truncated");
    }
}
