//! Player damage, armour, death. Owner: rules work package.
//! Reference: decomp docs/server/player-movement.md §6, weapons.md §5.9;
//! `CBasePlayer::TraceAttack` (0x901cc) and `CBasePlayer::TakeDamage` (0x8fa94).
//!
//! `damage::apply` has a single entry point, so the TraceAttack part (spawn protection, stunt and
//! dodge ×0.7, team check, hitgroup multipliers, `PTakeDam`, hit sounds) runs only for damage the
//! caller marked `DamageInfo::traced`, i.e. what TS routes through `TraceAttack` (bullets, melee,
//! explosions even through walls, thrown and falling objects). The TakeDamage part (team check,
//! kevlar, health) runs for everything.

use super::Player;
use crate::damage::{bits, hitgroup};
use crate::messages::Msg;
use crate::rules::{self, msg};
use crate::world::{Cx, DamageInfo};
use goldsrc::edict::EdictPtr;
use goldsrc::server::engine;
use goldsrc::sys::*;
use shared::protocol::hud_wire::{PTakeDam, TsFade, director, fade};
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
    1.0 - kevlar_absorb(armor, group, dmg_bits)
}

/// The fraction kevlar absorbs, `min(absorb, 1)`, or 0 where it doesn't apply.
fn kevlar_absorb(armor: f32, group: i32, dmg_bits: i32) -> f32 {
    if armor != 0.0 && dmg_bits & bits::ARMOR_EXEMPT == 0 && (hitgroup::CHEST..=hitgroup::RIGHTARM).contains(&group) {
        tables().combat.armor_absorb.min(1.0)
    } else {
        0.0
    }
}

/// `PTakeDam` amount byte: half the damage (×8 for shotgun pellets), capped at 127, +128 on kevlar.
/// TraceAttack truncates it (`fistp` under a round-toward-zero control word at 0x904b7), as does
/// `as u8`.
pub fn ptakedam_amount(dmg: f32, dmg_bits: i32, kevlar: bool) -> u8 {
    let d = if dmg_bits & DMG_SHOTGUN != 0 { dmg * 8.0 } else { dmg };
    let a = (d * 0.5).min(127.0);
    (if kevlar { a + 128.0 } else { a }) as u8
}

/// Damage that reaches `CBaseMonster::TakeDamage` after kevlar: `CBasePlayer::TakeDamage`
/// computes `dmg - min(absorb, 1) * dmg` on the x87 stack (0x8fc38), stores it as a float, then
/// truncates it to an int (0x8fc5f) and passes it back as a float. The product of two floats is
/// exact in `f64`, so only the subtraction can round differently from the 80-bit original.
pub fn after_kevlar(amount: f32, armor: f32, group: i32, dmg_bits: i32) -> f32 {
    let absorbed = kevlar_absorb(armor, group, dmg_bits);
    let d = amount as f64;
    ((d - absorbed as f64 * d) as f32).trunc()
}

/// `CBasePlayer::TakeHealth` (0xd8760) on the entvars: refused with `takedamage` off or health
/// already above max; otherwise added with no cap. Returns `None` when refused, else whether the
/// player is now over max (TS then sets the 1/s decay timer, `+0x73c`, to time + 1).
fn add_health(v: &mut entvars_t, amount: f32) -> Option<bool> {
    if v.takedamage == 0.0 || v.health > v.max_health {
        return None;
    }
    v.health += amount;
    Some(v.health > v.max_health)
}

/// `CBasePlayer::TakeHealth` (0xd8760) for code that holds the player.
pub fn take_health(p: &mut Player, cx: &mut Cx, amount: f32) -> bool {
    let now = cx.time();
    let Some(over) = add_health(cx.pev(), amount) else {
        return false;
    };
    if over {
        p.ts.health_timer = now + 1.0;
    }
    true
}

/// `CBasePlayer::TakeHealth` (0xd8760) for entities healing a player edict (healthkits, chargers,
/// doors, negative `trigger_hurt`). The health changes at once; when the player is taken out
/// further up the stack (its own `+use` opening a healing door), the decay timer is set as soon
/// as it is back.
pub fn take_health_edict(e: *mut edict_t, amount: f32) -> bool {
    let timer = engine().time() + 1.0;
    let Some(over) = add_health(e.v(), amount) else {
        return false;
    };
    if over {
        crate::world::defer(e, move || {
            super::with_player(e, |p, _| p.ts.health_timer = timer);
        });
    }
    true
}

/// `CBasePlayer::TakeDamage` (0x8fa94): a hit arms the bullet dance when it does at most 80
/// damage, the victim's horizontal speed is under 80 u/s, and it isn't in the legs.
pub fn arms_bullet_dance(group: i32, speed2d: f32, dmg: f32) -> bool {
    !matches!(group, hitgroup::LEFTLEG | hitgroup::RIGHTLEG) && speed2d < 80.0 && dmg <= 80.0
}

pub fn take_damage(p: &mut Player, cx: &mut Cx, dmg: &DamageInfo) -> bool {
    let eng = engine();
    let now = cx.time();
    let e = cx.edict;
    let mut amount = dmg.damage;
    let attacker_is_teammate = !dmg.attacker.is_null() && dmg.attacker != e && rules::teams::same_team(dmg.attacker, e);
    let friendly_fire = rules::cvars::on(c"mp_friendlyfire");

    if dmg.traced {
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
            // A body still in its death animation twitches when shot.
            crate::stunt::anim::bullet_dance(p, cx);
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
    if !Player::is_alive(cx) || amount == 0.0 {
        return false;
    }
    // Arm the bullet dance (`normal_die*` death poses, and twitching if the body is shot while
    // it falls) for a light hit on a slow body, unless the hit is in the legs. TS decides this
    // before the team check, on the damage after the TraceAttack multipliers.
    let v = cx.pev().velocity;
    if arms_bullet_dance(p.ts.last_hitgroup, (v[0] * v[0] + v[1] * v[1]).sqrt(), amount) {
        crate::stunt::anim::enable_bullet_dance(p);
    } else {
        crate::stunt::anim::disable_bullet_dance(p);
    }
    if attacker_is_teammate && !friendly_fire {
        return false; // FPlayerCanTakeDamage
    }
    // Truncated, like the kevlar-scaled damage below (0x8fbd0).
    p.ts.last_damage = amount as i32;
    let amount = after_kevlar(amount, cx.pev().armorvalue, p.ts.last_hitgroup, dmg.bits);
    // TS tells the HLTV director after CBaseMonster::TakeDamage, even when that refused the
    // damage (`takedamage` off); a kill's DeathNotice event therefore goes out first.
    let tell_director = |e: *mut edict_t, inflictor: *mut edict_t| {
        let eng = engine();
        rules::msg::director_event(eng.index_of(e), eng.index_of(inflictor), director::FLAGS_DAMAGE);
    };
    if cx.pev().takedamage == 0.0 {
        p.tbd.on_damage(dmg.bits);
        tell_director(e, dmg.inflictor);
        return false;
    }

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
    // Time-based damage bookkeeping (`player::water`), after the damage either way.
    p.tbd.on_damage(dmg.bits);
    tell_director(e, dmg.inflictor);
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
    fn bullet_dance_arming() {
        assert!(arms_bullet_dance(hitgroup::CHEST, 0.0, 80.0));
        assert!(!arms_bullet_dance(hitgroup::CHEST, 0.0, 80.5), "heavy hit");
        assert!(!arms_bullet_dance(hitgroup::CHEST, 80.0, 10.0), "moving body");
        assert!(!arms_bullet_dance(hitgroup::LEFTLEG, 0.0, 10.0), "leg hit");
        assert!(!arms_bullet_dance(hitgroup::RIGHTLEG, 0.0, 10.0), "leg hit");
        assert!(arms_bullet_dance(hitgroup::HEAD, 79.9, 10.0));
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
        assert_eq!(ptakedam_amount(31.0, bits::BULLET, false), 15, "15.5 truncates");
        assert_eq!(ptakedam_amount(23.6, bits::BULLET, true), 139, "139.8 truncates");
    }

    #[test]
    fn take_health_rules() {
        let mut v = entvars_t { takedamage: 2.0, health: 90.0, max_health: 100.0, ..Default::default() };
        assert_eq!(add_health(&mut v, 50.0), Some(true), "no cap, overheal starts the decay");
        assert_eq!(v.health, 140.0);
        assert_eq!(add_health(&mut v, 10.0), None, "already above max");
        v.health = 100.0;
        assert_eq!(add_health(&mut v, 0.0), Some(false), "at max is still allowed");
        v.health = 50.0;
        assert_eq!(add_health(&mut v, 10.0), Some(false));
        assert_eq!(v.health, 60.0);
        v.takedamage = 0.0;
        assert_eq!(add_health(&mut v, 10.0), None, "takedamage off");
        assert_eq!(v.health, 60.0);
    }

    #[test]
    fn kevlar_damage_truncates() {
        // 30 × (1 − 0.2125) = 23.625: TS does 23.
        assert_eq!(after_kevlar(30.0, 100.0, hitgroup::CHEST, bits::BULLET), 23.0);
        assert_eq!(after_kevlar(30.9, 0.0, hitgroup::CHEST, bits::BULLET), 30.0, "no armour still truncates");
        assert_eq!(after_kevlar(0.5, 0.0, hitgroup::HEAD, bits::BULLET), 0.0);
    }
}
