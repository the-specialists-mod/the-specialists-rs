//! Player damage, armour, death. Owner: rules work package.
//! Reference: decomp docs/server/player-movement.md §6, weapons.md §5.9;
//! `CBasePlayer::TraceAttack` (0x901cc) and `CBasePlayer::TakeDamage` (0x8fa94).
//!
//! `damage::apply` has a single entry point, so the TraceAttack part (spawn protection, stunt and
//! dodge ×0.7, hitgroup multipliers, `PTakeDam`, hit sounds) runs for traced damage: a hitgroup
//! other than GENERIC, or `DMG_BULLET`. The TakeDamage part (team check, kevlar, health) runs for
//! everything.

use super::Player;
use crate::damage::{bits, hitgroup};
use crate::messages::Msg;
use crate::rules::{self, msg};
use crate::world::{Cx, DamageInfo};
use goldsrc::edict::EdictPtr;
use goldsrc::server::engine;
use goldsrc::sys::*;
use shared::protocol::hud_wire::{PTakeDam, TsFade, fade};
use shared::weapons::tables;

/// TS: shotgun pellets (headshot ×1.5 instead of ×2, `PTakeDam` amount ×8).
pub const DMG_SHOTGUN: i32 = 0x0200_0000;

/// Hitgroup multiplier from TraceAttack (head ×2 / ×1.5 for shotguns with bullet or thrown
/// damage, legs ×0.5).
pub fn hitgroup_scale(group: i32, dmg_bits: i32) -> f32 {
    let c = &tables().combat;
    match group {
        hitgroup::HEAD if dmg_bits & (bits::BULLET | bits::THROWN) != 0 => {
            if dmg_bits & DMG_SHOTGUN != 0 {
                c.headshot_shotgun
            } else {
                c.headshot
            }
        }
        hitgroup::LEFTLEG | hitgroup::RIGHTLEG => c.leg,
        _ => 1.0,
    }
}

/// Kevlar: on chest/stomach/arms, for damage types it covers, `dmg -= dmg * min(absorb, 1)`.
/// Armour is never depleted.
pub fn kevlar_scale(armor: f32, group: i32, dmg_bits: i32) -> f32 {
    if armor != 0.0 && dmg_bits & bits::ARMOR_EXEMPT == 0 && (hitgroup::CHEST..=hitgroup::RIGHTARM).contains(&group) {
        1.0 - tables().combat.armor_absorb.min(1.0)
    } else {
        1.0
    }
}

/// `PTakeDam` amount byte: half the damage (×8 for shotgun pellets), capped at 127, +128 on kevlar.
pub fn ptakedam_amount(dmg: f32, dmg_bits: i32, kevlar: bool) -> u8 {
    let d = if dmg_bits & DMG_SHOTGUN != 0 { dmg * 8.0 } else { dmg };
    let a = (d * 0.5).min(127.0);
    (if kevlar { a + 128.0 } else { a }).round().clamp(0.0, 255.0) as u8
}

pub fn take_damage(p: &mut Player, cx: &mut Cx, dmg: &DamageInfo) -> bool {
    let eng = engine();
    let now = cx.time();
    let e = cx.edict;
    let mut amount = dmg.damage;
    let attacker_is_teammate = !dmg.attacker.is_null() && dmg.attacker != e && rules::teams::same_team(dmg.attacker, e);
    let friendly_fire = rules::cvars::on(c"mp_friendlyfire");

    let traced = dmg.hitgroup != hitgroup::GENERIC || dmg.bits & bits::BULLET != 0;
    if traced {
        if now < p.ts.protect_until {
            return false;
        }
        if cx.pev().iuser4 & rules::scoring::STUNTING != 0 {
            amount *= tables().combat.stunt_damage_taken;
        }
        if now < p.stunt.dodge_until && dmg.bits & bits::BULLET != 0 {
            amount *= tables().combat.stunt_damage_taken;
        }
        if attacker_is_teammate && !friendly_fire {
            return false;
        }
        if cx.pev().takedamage == 0.0 {
            return false;
        }
        p.ts.last_hitgroup = dmg.hitgroup;
        let headshot = dmg.hitgroup == hitgroup::HEAD && dmg.bits & (bits::BULLET | bits::THROWN) != 0;
        amount *= hitgroup_scale(dmg.hitgroup, dmg.bits);
        let is_alive = Player::is_alive(cx);
        let kevlar_hit = cx.pev().armorvalue != 0.0
            && dmg.bits & bits::BULLET != 0
            && is_alive
            && (hitgroup::CHEST..=hitgroup::RIGHTARM).contains(&dmg.hitgroup);
        if !is_alive {
            let v = cx.pev();
            if v.flags & FL_ONGROUND as i32 == 0 {
                for i in 0..3 {
                    v.velocity[i] += dmg.dir[i] * 50.0;
                }
            }
        }
        if now > p.ts.next_ptakedam {
            p.ts.next_ptakedam = now + 0.3;
            let m = PTakeDam {
                x: dmg.pos[0],
                y: dmg.pos[1],
                z: dmg.pos[2],
                dx: dmg.dir[0],
                dy: dmg.dir[1],
                dz: dmg.dir[2],
                amount: ptakedam_amount(amount, dmg.bits, kevlar_hit),
            };
            msg::pvs(Msg::PTakeDam, cx.pev().origin, |w| m.write(w));
            if dmg.bits & (bits::BULLET | bits::BLAST) != 0 {
                let sample = if headshot {
                    if eng.random_long(0, 1) == 0 { c"player/headshot.wav" } else { c"player/headshot2.wav" }
                } else if kevlar_hit {
                    c"player/kevlarhit.wav"
                } else {
                    [c"player/bodyhit1.wav", c"player/bodyhit2.wav", c"player/bodyhit3.wav", c"player/bodyhit4.wav"]
                        [eng.random_long(0, 3) as usize]
                };
                eng.emit_sound(e, CHAN_BODY as i32, sample, 1.0, 1.25, 0, 95 + eng.random_long(0, 31));
            }
            if is_alive {
                let pain = match eng.random_long(1, 7) {
                    1 => Some(c"player/pain2.wav"),
                    2 => Some(c"player/pain3.wav"),
                    3 => Some(c"player/pain4.wav"),
                    _ => None,
                };
                if let Some(s) = pain {
                    eng.emit_sound(e, CHAN_VOICE as i32, s, 1.0, 1.25, 0, 100);
                }
            }
        }
        // TraceAttack's `dmgreport` cheat log (any non-zero value).
        if crate::weapons::dmgreport() != 0.0 && !dmg.attacker.is_null() {
            let (by, to) = (eng.string(dmg.attacker.v().netname), eng.string(cx.pev().netname));
            let line = format!("{} damaged {} ({amount:.6} hp)\n", by.to_string_lossy(), to.to_string_lossy());
            eng.alert(ALERT_TYPE::at_console, &line);
        }
    }

    // CBasePlayer::TakeDamage
    if !Player::is_alive(cx) || amount == 0.0 || cx.pev().takedamage == 0.0 {
        return false;
    }
    if attacker_is_teammate && !friendly_fire {
        return false; // FPlayerCanTakeDamage
    }
    p.ts.last_damage = amount.round() as i32;
    amount *= kevlar_scale(cx.pev().armorvalue, p.ts.last_hitgroup, dmg.bits);
    let amount = amount.round();

    // CBaseMonster::TakeDamage: knockback, health, death.
    {
        let v = cx.pev();
        if !dmg.inflictor.is_null() && v.movetype == MOVETYPE_WALK as i32 && dmg.dir != [0.0; 3] {
            let attacker_trigger = !dmg.attacker.is_null() && dmg.attacker.v().solid == SOLID_TRIGGER as i32;
            if !attacker_trigger {
                let force = amount * 5.0;
                for i in 0..3 {
                    v.velocity[i] += dmg.dir[i] * force;
                }
            }
        }
        v.dmg_take += amount;
        v.dmg_inflictor = dmg.inflictor;
        v.health -= amount;
    }
    if amount >= 1.0 {
        let f = TsFade::new([150, 10, 10], 0x4b, 0.3, 0.15, fade::IN);
        msg::one(Msg::TsFade, e, |w| f.write(w));
    }
    if cx.pev().health <= 0.0 {
        rules::on_killed(p, cx, dmg);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multipliers() {
        assert_eq!(hitgroup_scale(hitgroup::HEAD, bits::BULLET), 2.0);
        assert_eq!(hitgroup_scale(hitgroup::HEAD, bits::BULLET | DMG_SHOTGUN), 1.5);
        assert_eq!(hitgroup_scale(hitgroup::HEAD, bits::CLUB), 1.0, "melee isn't a headshot");
        assert_eq!(hitgroup_scale(hitgroup::LEFTLEG, bits::BULLET), 0.5);
        assert_eq!(hitgroup_scale(hitgroup::CHEST, bits::BULLET), 1.0);
    }

    #[test]
    fn kevlar() {
        let s = kevlar_scale(100.0, hitgroup::CHEST, bits::BULLET);
        assert!((s - (1.0 - 0.2125)).abs() < 1e-6);
        assert_eq!(kevlar_scale(0.0, hitgroup::CHEST, bits::BULLET), 1.0, "no armour");
        assert_eq!(kevlar_scale(100.0, hitgroup::HEAD, bits::BULLET), 1.0, "head isn't covered");
        assert_eq!(kevlar_scale(100.0, hitgroup::LEFTLEG, bits::BULLET), 1.0, "legs aren't covered");
        assert_eq!(kevlar_scale(100.0, hitgroup::STOMACH, bits::BLAST), 1.0, "blast is exempt");
        assert_eq!(kevlar_scale(100.0, hitgroup::LEFTARM, bits::THROWN), 1.0, "thrown knives are exempt");
    }

    #[test]
    fn ptakedam_byte() {
        assert_eq!(ptakedam_amount(30.0, bits::BULLET, false), 15);
        assert_eq!(ptakedam_amount(30.0, bits::BULLET, true), 143);
        assert_eq!(ptakedam_amount(500.0, bits::BULLET, false), 127);
        assert_eq!(ptakedam_amount(10.0, bits::BULLET | DMG_SHOTGUN, false), 40);
    }
}
