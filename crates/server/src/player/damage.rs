//! Player damage, armour, death. Owner: rules work package.
//! Reference: decomp docs/server/player-movement.md §6, weapons.md §5.9;
//! `CBasePlayer::TraceAttack` (0x901cc) and `CBasePlayer::TakeDamage` (0x8fa94).
//!
//! As in TS the two are separate: [`trace_attack`] (spawn protection, stunt and dodge ×0.7,
//! team check, hitgroup multipliers, `PTakeDam`, hit sounds) only adds its scaled damage to the
//! multi-damage (`crate::damage::multi`), and [`take_damage`] (team check, kevlar, health) gets the
//! sum from `ApplyMultiDamage`, so the pellets of one shotgun shot reach it as one hit. Direct
//! damage (falls, drowning, triggers, crushers) goes straight to `take_damage`.

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
use std::ffi::CStr;

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
pub fn kevlar_scale(armor: f32, group: i32, dmg_bits: i32, absorb: f32) -> f32 {
    1.0 - kevlar_absorb(armor, group, dmg_bits, absorb)
}

/// The fraction kevlar absorbs, `min(absorb, 1)`, or 0 where it doesn't apply.
fn kevlar_absorb(armor: f32, group: i32, dmg_bits: i32, absorb: f32) -> f32 {
    if armor != 0.0 && dmg_bits & bits::ARMOR_EXEMPT == 0 && (hitgroup::CHEST..=hitgroup::RIGHTARM).contains(&group) {
        absorb.min(1.0)
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
pub fn after_kevlar(amount: f32, armor: f32, group: i32, dmg_bits: i32, absorb: f32) -> f32 {
    let absorbed = kevlar_absorb(armor, group, dmg_bits, absorb);
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
/// doors, negative `trigger_hurt`, `game_player_hurt`). The health changes at once, and so does
/// the decay timer when the player is lent to its slot (its own `+use` opening a healing door,
/// `player_use`); one held by a dispatch that didn't lend it gets the timer once that returns.
pub fn take_health_edict(e: *mut edict_t, amount: f32) -> bool {
    let timer = engine().time() + 1.0;
    let Some(over) = add_health(e.v(), amount) else {
        return false;
    };
    if over {
        crate::world::run_now_or_defer(e, move || {
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

/// `CBasePlayer::TraceAttack` (0x901cc): spawn protection, stunt and dodge ×0.7, team check,
/// hitgroup multipliers, the dead-body twitch, `PTakeDam` and the hit sounds, then
/// `AddMultiDamage` with the scaled damage (the `TakeDamage` comes from `ApplyMultiDamage`).
pub fn trace_attack(p: &mut Player, cx: &mut Cx, dmg: &DamageInfo) {
    let eng = engine();
    let now = cx.time();
    let e = cx.edict;
    let mut amount = dmg.damage;

    if now < p.ts.protect_until {
        return;
    }
    if cx.pev().iuser4 & rules::scoring::STUNTING != 0 {
        amount *= tables().combat.stunt_damage_taken;
    }
    if now < p.stunt.dodge_until && dmg.bits & bits::BULLET != 0 {
        amount *= tables().combat.stunt_damage_taken;
    }
    if trace_attack_refused(dmg.attacker, e) {
        return;
    }
    if cx.pev().takedamage == 0.0 {
        return;
    }
    p.ts.last_hitgroup = dmg.hitgroup;
    let headshot = dmg.hitgroup == hitgroup::HEAD && dmg.bits & (bits::BULLET | bits::THROWN) != 0;
    if headshot {
        crate::damage::multi::set_headshot();
    }
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
    crate::damage::multi::add(dmg.attacker, e, amount, dmg.bits);
    // TraceAttack's `dmgreport` cheat log (any non-zero value), after the AddMultiDamage.
    if crate::weapons::dmgreport() != 0.0 && !dmg.attacker.is_null() {
        let (by, to) = (eng.string(dmg.attacker.v().netname), eng.string(cx.pev().netname));
        let line = format!("{} damaged {} ({amount:.6} hp)\n", by.to_string_lossy(), to.to_string_lossy());
        eng.alert(ALERT_TYPE::at_console, &line);
    }
}

/// TraceAttack's team check (0x90256): a player attacker (`IsPlayer()`, so also one who has left) on the victim's team
/// (`PlayerRelationship == GR_TEAMMATE`) without `mp_friendlyfire`. Unlike
/// `FPlayerCanTakeDamage` it doesn't exempt the victim itself, so in teamplay without friendly
/// fire a player's own traced damage (its grenade's blast) is refused.
fn trace_attack_refused(attacker: *mut edict_t, victim: *mut edict_t) -> bool {
    rules::has_player(attacker) && rules::teams::same_team(attacker, victim) && !rules::cvars::on(c"mp_friendlyfire")
}

/// `CHalfLifeTeamplay::FPlayerCanTakeDamage` (0xe5208) refusing: a teammate other than the
/// victim, without `mp_friendlyfire`.
fn take_damage_refused(attacker: *mut edict_t, victim: *mut edict_t) -> bool {
    !attacker.is_null() && attacker != victim && rules::teams::same_team(attacker, victim) && !rules::cvars::on(c"mp_friendlyfire")
}

/// `CBasePlayer::TakeDamage` (0x8fa94) with `CBaseMonster::TakeDamage` (0xb2430) inside it:
/// what `ApplyMultiDamage` (the TraceAttacks' sum) or a direct hit deals. The hitgroup is the
/// one the last TraceAttack left (`m_LastHitGroup`).
pub fn take_damage(p: &mut Player, cx: &mut Cx, dmg: &DamageInfo) -> bool {
    let e = cx.edict;
    let amount = dmg.damage;
    // `flHealthPrev`, for the suit's health reports.
    let health_prev = cx.pev().health;
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
    if take_damage_refused(dmg.attacker, e) {
        return false; // FPlayerCanTakeDamage
    }
    // Truncated, like the kevlar-scaled damage below (0x8fbd0).
    p.ts.last_damage = amount as i32;
    let after = after_kevlar(amount, cx.pev().armorvalue, p.ts.last_hitgroup, dmg.bits, crate::rules::state::armor_absorb());

    let took = monster_take_damage(p, cx, dmg, after);

    // The rest of CBasePlayer::TakeDamage runs whether or not that took it: each time-based type
    // taken restarts its countdown and `m_bitsDamageType |= bits` (`player::water`), then the
    // HLTV director hears of it (after a kill's DeathNotice), then the red fade for a hit of 1+.
    p.tbd.on_damage(dmg.bits);
    let eng = engine();
    rules::msg::director_event(eng.index_of(e), eng.index_of(dmg.inflictor), director::FLAGS_DAMAGE);
    // "How bad is it, doc?": the HEV suit's damage reports (queued in single player only,
    // `player::suit`), on the health and `m_lastDamageAmount` the hit left.
    let health = cx.pev().health;
    let severity = Severity { trivial: health > 75.0 || p.ts.last_damage < 5, major: p.ts.last_damage > 25 };
    if took {
        for (name, no_repeat) in damage_reports(dmg.bits, severity, p.ts.last_damage) {
            super::suit::set_suit_update(p, cx, Some(name), false, no_repeat);
        }
    }
    if after >= 1.0 {
        let f = TsFade::new([150, 10, 10], 0x4b, 0.3, 0.15, fade::IN);
        msg::one(Msg::TsFade, e, |w| f.write(w));
    }
    if took {
        health_reports(p, cx, severity, health < 30.0, health_prev, dmg.bits);
    }
    took
}

/// `TakeDamage`'s `ftrivial` (health over 75 or under 5 damage) and `fmajor` (over 25 damage).
#[derive(Clone, Copy, Debug)]
struct Severity {
    trivial: bool,
    major: bool,
}

/// Damage types `TakeDamage`'s suit reports name that `crate::damage::bits` doesn't.
const DMG_SONIC: i32 = 1 << 9;
const DMG_NERVEGAS: i32 = 1 << 16;
const DMG_RADIATION: i32 = 1 << 18;

/// `CBasePlayer::TakeDamage`'s report loop (`SetSuitUpdate` calls 0x8fe2e..0x8ffa3), for
/// damage that was taken: one sentence per kind of damage (unless the hit was trivial and not
/// time-based), as `(name, no-repeat seconds)` in TS's order.
fn damage_reports(dmg_bits: i32, s: Severity, last_damage: i32) -> Vec<(&'static CStr, i32)> {
    let mut out = Vec::new();
    let mut b = dmg_bits;
    let mut found = true;
    while (!s.trivial || b & bits::TIMEBASED != 0) && found && b != 0 {
        found = false;
        if b & bits::CLUB != 0 {
            if s.major {
                out.push((c"!HEV_DMG4", 30));
            }
            b &= !bits::CLUB;
            found = true;
        }
        if b & (bits::CRUSH | bits::FALL) != 0 {
            out.push((if s.major { c"!HEV_DMG5" } else { c"!HEV_DMG4" }, 30));
            b &= !(bits::CRUSH | bits::FALL);
            found = true;
        }
        if b & bits::BULLET != 0 {
            if last_damage > 5 {
                out.push((c"!HEV_DMG6", 30));
            }
            b &= !bits::BULLET;
            found = true;
        }
        if b & bits::SLASH != 0 {
            out.push((if s.major { c"!HEV_DMG1" } else { c"!HEV_DMG0" }, 30));
            b &= !bits::SLASH;
            found = true;
        }
        if b & DMG_SONIC != 0 {
            if s.major {
                out.push((c"!HEV_DMG2", 60));
            }
            b &= !DMG_SONIC;
            found = true;
        }
        if b & (bits::POISON | bits::PARALYZE) != 0 {
            out.push((c"!HEV_DMG3", 60));
            b &= !(bits::POISON | bits::PARALYZE);
            found = true;
        }
        if b & bits::ACID != 0 {
            out.push((c"!HEV_DET1", 60));
            b &= !bits::ACID;
            found = true;
        }
        if b & DMG_NERVEGAS != 0 {
            out.push((c"!HEV_DET0", 60));
            b &= !DMG_NERVEGAS;
            found = true;
        }
        if b & DMG_RADIATION != 0 {
            out.push((c"!HEV_DET2", 60));
            b &= !DMG_RADIATION;
            found = true;
        }
        if b & bits::SHOCK != 0 {
            b &= !bits::SHOCK;
            found = true;
        }
    }
    out
}

/// The end of `CBasePlayer::TakeDamage` (`SetSuitUpdate` calls 0x9006e..0x901b3), for damage
/// that was taken: the suit's health reports. The `RandomLong(0, 3)` rolls run under any rules
/// (only the queueing is single-player).
fn health_reports(p: &mut Player, cx: &mut Cx, s: Severity, critical: bool, health_prev: f32, dmg_bits: i32) {
    let eng = engine();
    if !s.trivial && s.major && health_prev >= 75.0 {
        super::suit::set_suit_update(p, cx, Some(c"!HEV_MED1"), false, 1800);
        super::suit::set_suit_update(p, cx, Some(c"!HEV_HEAL7"), false, 1800);
    }
    if !s.trivial && critical && health_prev < 75.0 {
        let health = cx.pev().health;
        if health < 6.0 {
            super::suit::set_suit_update(p, cx, Some(c"!HEV_HLTH3"), false, 600);
        } else if health < 20.0 {
            super::suit::set_suit_update(p, cx, Some(c"!HEV_HLTH2"), false, 600);
        }
        if eng.random_long(0, 3) == 0 && health_prev < 50.0 {
            super::suit::set_suit_update(p, cx, Some(c"!HEV_DMG7"), false, 300);
        }
    }
    if dmg_bits & bits::TIMEBASED != 0 && health_prev < 75.0 {
        if health_prev >= 50.0 {
            super::suit::set_suit_update(p, cx, Some(c"!HEV_HLTH1"), false, 600);
        } else if eng.random_long(0, 3) == 0 {
            super::suit::set_suit_update(p, cx, Some(c"!HEV_DMG7"), false, 300);
        }
    }
}

/// `CBaseMonster::TakeDamage` (0xb2430) for a live player: refused with `takedamage` off;
/// `g_vecAttackDir` from a real inflictor (`damage::update_attack_dir`); the damage counters (the inflictor only when there is one); refused in god mode (`FL_GODMODE`);
/// then the health drops and `Killed` runs at 0 or below (returning 0). TS has no knockback here
/// (no `DamageForce` call, no velocity write); only `Killed` pushes the body.
fn monster_take_damage(p: &mut Player, cx: &mut Cx, dmg: &DamageInfo, amount: f32) -> bool {
    {
        let v = cx.pev();
        if v.takedamage == 0.0 {
            return false;
        }
        crate::damage::update_attack_dir(dmg.inflictor, v);
        if !dmg.inflictor.is_null() {
            v.dmg_inflictor = dmg.inflictor;
        }
        v.dmg_take += amount;
        if v.flags & FL_GODMODE as i32 != 0 {
            return false;
        }
        v.health -= amount;
    }
    if cx.pev().health <= 0.0 {
        rules::on_killed(p, cx, dmg);
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `armor_absorb` as `CTSGameRules::Init` sets it.
    const ABSORB: f32 = crate::rules::state::INIT_ARMOR_ABSORB;

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
        let s = kevlar_scale(100.0, hitgroup::CHEST, bits::BULLET, ABSORB);
        assert!((s - (1.0 - 0.2125)).abs() < 1e-6);
        assert_eq!(kevlar_scale(0.0, hitgroup::CHEST, bits::BULLET, ABSORB), 1.0, "no armour");
        assert_eq!(kevlar_scale(100.0, hitgroup::HEAD, bits::BULLET, ABSORB), 1.0, "head isn't covered");
        assert_eq!(kevlar_scale(100.0, hitgroup::LEFTLEG, bits::BULLET, ABSORB), 1.0, "legs aren't covered");
        assert_eq!(kevlar_scale(100.0, hitgroup::STOMACH, bits::BLAST, ABSORB), 1.0, "blast is exempt");
        assert_eq!(kevlar_scale(100.0, hitgroup::LEFTARM, bits::THROWN, ABSORB), 1.0, "thrown knives are exempt");
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
    fn suit_damage_reports() {
        let names = |bits, trivial, major, last| -> Vec<(String, i32)> {
            damage_reports(bits, Severity { trivial, major }, last).into_iter().map(|(n, t)| (n.to_str().unwrap().to_owned(), t)).collect()
        };
        assert_eq!(names(bits::BULLET, true, false, 30), [], "trivial and not time-based");
        assert_eq!(names(bits::BULLET, false, false, 6), [("!HEV_DMG6".into(), 30)]);
        assert_eq!(names(bits::BULLET, false, false, 5), [], "bullets over 5 damage only");
        assert_eq!(names(bits::CLUB, false, false, 10), [], "a club only reports major hits");
        assert_eq!(names(bits::FALL, false, true, 30), [("!HEV_DMG5".into(), 30)]);
        assert_eq!(names(bits::SLASH | bits::CRUSH, false, false, 10), [("!HEV_DMG4".into(), 30), ("!HEV_DMG0".into(), 30)]);
        assert_eq!(names(bits::POISON, true, false, 2), [("!HEV_DMG3".into(), 60)], "time-based reports even when trivial");
        assert_eq!(
            names(bits::ACID | DMG_NERVEGAS | DMG_RADIATION | bits::SHOCK | bits::BLAST, false, true, 30),
            [("!HEV_DET1".into(), 60), ("!HEV_DET0".into(), 60), ("!HEV_DET2".into(), 60)]
        );
        assert_eq!(names(DMG_SONIC, false, false, 10), [], "sonic only when major");
        assert_eq!(names(bits::BLAST, false, true, 30), [], "no report for blast");
    }

    #[test]
    fn kevlar_damage_truncates() {
        // 30 × (1 − 0.2125) = 23.625: TS does 23.
        assert_eq!(after_kevlar(30.0, 100.0, hitgroup::CHEST, bits::BULLET, ABSORB), 23.0);
        assert_eq!(after_kevlar(30.9, 0.0, hitgroup::CHEST, bits::BULLET, ABSORB), 30.0, "no armour still truncates");
        assert_eq!(after_kevlar(0.5, 0.0, hitgroup::HEAD, bits::BULLET, ABSORB), 0.0);
    }
}
