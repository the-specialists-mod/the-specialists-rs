//! Melee hit resolution: `CBasePlayer::CheckHull` 0x95e28, `CloseCombatAttack` 0x95f88,
//! `ColdCockAttack` 0x96610, the knife/katana slash traces, the blocks (`VictimKungfuHold`
//! 0x76f50, `VictimKnifeHold` 0x76de4, `VictimKatanaHold` 0x76c8c) and the delayed "lunge" hit
//! of a missed melee attack (the melee-window part of `CTSStunt::CheckForBreakables`).

use super::inventory::{self, Drop};
use super::net;
use crate::damage::{self, DamageInfo};
use crate::messages::{self, Msg};
use crate::player::Player;
use crate::world::{self, Cx};
use goldsrc::edict::EdictPtr;
use goldsrc::server::{DONT_IGNORE_MONSTERS, engine};
use goldsrc::sys::*;
use shared::weapons::gun::{MeleeHit, MeleeKind, MeleeResult};
use shared::weapons::spread::{self, Vec3};

/// `FindHullIntersection` (SDK crowbar.cpp): the closest hit among the hull's corners.
fn find_hull_intersection(src: Vec3, tr: &mut TraceResult, mins: Vec3, maxs: Vec3, skip: *mut edict_t) {
    let e = engine();
    let hull_end = spread::add(src, spread::scale(spread::sub(tr.vecEndPos, src), 2.0));
    let tmp = e.trace_line(src, hull_end, DONT_IGNORE_MONSTERS, skip);
    if tmp.flFraction < 1.0 {
        *tr = tmp;
        return;
    }
    let mm = [mins, maxs];
    let mut distance = 1e6f32;
    for i in 0..2 {
        for j in 0..2 {
            for k in 0..2 {
                let end = [hull_end[0] + mm[i][0], hull_end[1] + mm[j][1], hull_end[2] + mm[k][2]];
                let t = e.trace_line(src, end, DONT_IGNORE_MONSTERS, skip);
                if t.flFraction < 1.0 {
                    let d = spread::length(spread::sub(t.vecEndPos, src));
                    if d < distance {
                        *tr = t;
                        distance = d;
                    }
                }
            }
        }
    }
}

fn is_bsp(e: *mut edict_t) -> bool {
    if e.is_null() {
        return true;
    }
    let v = e.v();
    v.solid == SOLID_BSP as i32 || v.movetype == MOVETYPE_PUSHSTEP as i32
}

/// `CBasePlayer::CheckHull(start, end, tr, hull)`.
pub fn check_hull(start: Vec3, end: Vec3, hull: i32, skip: *mut edict_t) -> TraceResult {
    let e = engine();
    let mut tr = e.trace_line(start, end, DONT_IGNORE_MONSTERS, skip);
    if tr.flFraction >= 1.0 {
        tr = e.trace_hull(start, end, DONT_IGNORE_MONSTERS, hull, skip);
        if tr.flFraction < 1.0 && is_bsp(tr.pHit) {
            let z = if hull == 3 { 18.0 } else { 36.0 };
            find_hull_intersection(start, &mut tr, [-16.0, -16.0, -z], [16.0, 16.0, z], skip);
        }
    }
    tr
}

fn alive(e: *mut edict_t) -> bool {
    let v = e.v();
    v.deadflag == DEAD_NO as i32 && v.health > 0.0
}

/// Applies a block by the victim. Returns what the attacker gets, or `None` if not blocked.
/// Victim-side effects (its next attack, its bar) are applied here.
/// `atk_block` is the attacker's own kung-fu block window (the attacker is the player being
/// dispatched, so the caller passes it; victims are other players, reachable with world::with).
pub fn hold(kind: MeleeKind, attacker: *mut edict_t, victim: *mut edict_t, atk_block: f32) -> Option<MeleeResult> {
    if !crate::rules::has_player(victim) {
        return None;
    }
    let r = world::with::<Player, _>(victim, |v| {
        let g = &mut v.inv.gun;
        match kind {
            MeleeKind::KungFu | MeleeKind::ColdCock => {
                if g.melee_bar < 0.1 {
                    return None;
                }
                if g.block_kungfu > 0.0 {
                    // the victim blocks: attacker 0.5 s lockout and -33 power, victim 0.25 s
                    g.melee_damage = 0.0;
                    g.next_attack = 0.25;
                    Some((MeleeResult::Blocked { next_attack: 0.5, bar_cost: 33.0 }, None))
                } else if atk_block > 0.0 && g.melee_source == 0 && g.melee_window > 0.0 {
                    // the attacker was blocking while the victim was mid-attack: the victim gets
                    // 0.5 s, then `UpdateMeleeBar(33)` (an emptied bar makes that 1.5 s)
                    g.melee_damage = 0.0;
                    g.next_attack = 0.5;
                    g.charge_melee_bar(33.0);
                    let bar = g.melee_bar;
                    Some((MeleeResult::Blocked { next_attack: 0.25, bar_cost: 0.0 }, Some(bar)))
                } else {
                    None
                }
            }
            MeleeKind::Knife => {
                if g.block_knife > 0.0 || (g.block_katana > 0.0 && g.melee_bar > 0.1) {
                    g.next_attack = 0.25;
                    Some((MeleeResult::Blocked { next_attack: 0.5, bar_cost: 33.0 }, None))
                } else {
                    None
                }
            }
            MeleeKind::Katana => {
                if g.block_katana > 0.0 && g.melee_bar > 0.1 {
                    g.next_attack = 0.35;
                    Some((MeleeResult::Blocked { next_attack: 0.75, bar_cost: 33.0 }, None))
                } else {
                    None
                }
            }
        }
    })
    .flatten()?;
    let (result, victim_bar) = r;
    if let Some(bar) = victim_bar {
        net::send_kfu_power(victim, bar);
    }
    match kind {
        MeleeKind::KungFu | MeleeKind::ColdCock => {
            let who = if victim_bar.is_some() { victim } else { attacker };
            slow_emit(who, c"player/kungfuhit.wav", 100);
        }
        MeleeKind::Knife | MeleeKind::Katana => {
            let a = attacker.v();
            let v = victim.v();
            let mut mid = spread::scale(spread::add(a.origin, v.origin), 0.5);
            mid[2] += 20.0;
            sparks(mid);
            slow_emit(attacker, c"weapons/knife/knife_hit.wav", 100);
        }
    }
    Some(result)
}

/// `UTIL_Sparks` (TE_SPARKS).
pub fn sparks(pos: Vec3) {
    const TE_SPARKS: i32 = 9;
    const SVC_TEMPENTITY: i32 = 23;
    goldsrc::server::MsgWriter::begin(MSG_PVS as i32, SVC_TEMPENTITY, Some(pos), std::ptr::null_mut())
        .byte(TE_SPARKS)
        .coord(pos[0])
        .coord(pos[1])
        .coord(pos[2])
        .end();
}

/// One melee attack by `attacker` (the player whose gun asked for it): the trace runs `hit.reach`
/// from the gun position along `v_angle`.
pub fn attack(attacker: *mut edict_t, hit: &MeleeHit, atk_block: f32) -> MeleeResult {
    let pev = attacker.v();
    let mut ang = pev.v_angle;
    if hit.zero_roll {
        ang[2] = 0.0;
    }
    let (fwd, _, _) = spread::angle_vectors(ang);
    let start = spread::add(pev.origin, pev.view_ofs);
    let end = spread::ma(start, hit.reach, fwd);
    trace_attack(attacker, hit, start, end, fwd, atk_block, false)
}

/// `CloseCombatAttack(start, end, dir, hull, dmg, ...)` 0x95f88 / `ColdCockAttack` 0x96610 / the
/// blade slashes from a given trace (`hit.reach` and `hit.zero_roll` are not used). `dir` is the
/// damage direction. `never_teammates` is `CloseCombatAttack`'s last flag, which only the jump
/// kick sets (`CTSStunt::CloseAttack` 0x8c8a5): a teammate is no target even with
/// `mp_friendlyfire 1`.
pub fn trace_attack(
    attacker: *mut edict_t,
    hit: &MeleeHit,
    start: Vec3,
    end: Vec3,
    dir: Vec3,
    atk_block: f32,
    never_teammates: bool,
) -> MeleeResult {
    let e = engine();
    let rules = crate::gameplay::active().server;
    if hit.kind == MeleeKind::KungFu && rules.motion_blur() && (e.random_long(10, 80) as f32) < hit.damage {
        let origin = attacker.v().origin;
        messages::pvs(Msg::TsMBlur, origin).byte(e.index_of(attacker)).byte(if hit.damage > 50.0 { 2 } else { 1 }).end();
    }
    let fwd = dir;
    let tr = check_hull(start, end, hit.hull, attacker);
    if tr.flFraction >= 1.0 || tr.pHit.is_null() {
        return MeleeResult::Miss;
    }
    let victim = tr.pHit;
    let v = victim.v();
    let is_player = crate::rules::has_player(victim);
    match hit.kind {
        // `CloseCombatAttack` 0x95f88 / `ColdCockAttack` 0x96610: no damage taken, a world brush
        // (`FL_WORLDBRUSH`) or an invisible entity is no target, and neither is a teammate with
        // `mp_friendlyfire 0` (`PlayerRelationship` == GR_TEAMMATE), or any teammate for the jump
        // kick.
        MeleeKind::KungFu | MeleeKind::ColdCock => {
            if v.takedamage == 0.0 || v.flags & FL_WORLDBRUSH as i32 != 0 || v.effects & EF_NODRAW as i32 != 0 {
                return MeleeResult::Miss;
            }
            let teammate = if never_teammates { crate::rules::teams::same_team(attacker, victim) } else { same_team(attacker, victim) };
            if is_player && teammate {
                return MeleeResult::Miss;
            }
        }
        // `KnifeFire` / `KatanaFire` (0x7394d): the target must be alive (`IsAlive`) and visible.
        MeleeKind::Knife | MeleeKind::Katana => {
            if !alive(victim) || v.effects & EF_NODRAW as i32 != 0 {
                return MeleeResult::Miss;
            }
        }
    }
    if is_player {
        if let Some(blocked) = rules.melee_block(hit.kind, attacker, victim, hit.damage, atk_block) {
            return blocked;
        }
        if matches!(hit.kind, MeleeKind::KungFu | MeleeKind::ColdCock) {
            // No disarm while the victim's spawn protection (`pl+0x71c`) runs.
            let now = e.time();
            if hit.disarm {
                crate::player::with_player(victim, |vp, vcx| {
                    if vp.ts.protect_until < now
                        && let Some(d) = rules.melee_disarm(hit.damage, vp.ts.is_the_one)
                    {
                        inventory::drop_weapon(vp, vcx, Drop::Disarm(d));
                    }
                });
            }
            if hit.knockback && hit.damage > 30.0 {
                knockback(attacker, victim);
            }
            slow_emit(victim, c"player/kungfuhit.wav", 95 + e.random_long(0, 31));
        }
    }
    let mut info = DamageInfo::new(attacker, attacker, hit.damage, hit.bits).traced();
    info.hitgroup = tr.iHitgroup;
    info.dir = fwd;
    info.pos = tr.vecEndPos;
    // `glast_closecombat` / `glast_coldcock` are set only around these two attacks' hit (the
    // knife and katana slashes set neither), for the kill's TSMBlur chance, points and messages.
    match hit.kind {
        MeleeKind::KungFu => damage::glast::close_combat(|| damage::apply(victim, &info)),
        MeleeKind::ColdCock => damage::glast::coldcock(|| damage::apply(victim, &info)),
        MeleeKind::Knife | MeleeKind::Katana => damage::apply(victim, &info),
    };
    MeleeResult::Hit { player: is_player }
}

/// `v.vel += |a.vel| * dir(a -> v)`, `v.vel.z = 235`.
fn knockback(attacker: *mut edict_t, victim: *mut edict_t) {
    let a = attacker.v();
    let v = victim.v();
    let speed = spread::length(a.velocity);
    let dir = spread::normalize(spread::sub(v.origin, a.origin));
    v.velocity = spread::ma(v.velocity, speed, dir);
    v.velocity[2] = 235.0;
}

/// The delayed hit of a missed melee attack (`CTSStunt::CheckForBreakables` 0x9bd9c with
/// `pl+0x79c` open, alive or not): while the window is open, a head-hull trace 32 u ahead (behind
/// for a negative damage) lands the stored damage, halving it each time.
pub fn pending_hit(p: &mut Player, cx: &mut Cx) {
    let g = &mut p.inv.gun;
    if g.melee_window == 0.0 || g.melee_damage == 0.0 {
        p.inv.sv.lunge_victim = std::ptr::null_mut();
        return;
    }
    let e = engine();
    let me = cx.edict;
    let pev = cx.pev();
    let (fwd, _, _) = spread::angle_vectors(pev.v_angle);
    let dist = if g.melee_damage >= 0.0 { 32.0 } else { -32.0 };
    let end = spread::ma(pev.origin, dist, fwd);
    let tr = e.trace_hull(pev.origin, end, DONT_IGNORE_MONSTERS, 3, me);
    if tr.flFraction >= 1.0 || tr.pHit.is_null() {
        return;
    }
    let victim = tr.pHit;
    if victim == p.inv.sv.lunge_victim {
        return;
    }
    let v = victim.v();
    if v.takedamage == 0.0 || v.flags & FL_WORLDBRUSH as i32 != 0 || v.effects & EF_NODRAW as i32 != 0 {
        return;
    }
    let source = g.melee_source;
    let bits = if source == 0 { 0x0400_1080 } else { 0x1080 };
    let dmg;
    (dmg, g.melee_damage) = spend_delayed_damage(g.melee_damage);
    if dmg <= 0.0 {
        return;
    }
    let is_player = crate::rules::has_player(victim);
    if is_player && same_team(me, victim) {
        // TS: `friendlyfire 0` and PlayerRelationship == GR_TEAMMATE: no damage, but the
        // victim still counts as hit.
        p.inv.sv.lunge_victim = victim;
        return;
    }
    if is_player {
        let kind = match source {
            0 => MeleeKind::KungFu,
            1 => MeleeKind::Knife,
            _ => MeleeKind::Katana,
        };
        let rules = crate::gameplay::active().server;
        // Lent for the block, so a ruleset's block can reach the attacker as in a direct hit.
        let atk_block = g.block_kungfu;
        let blocked = crate::world::lend(me, p, || rules.melee_block(kind, me, victim, dmg, atk_block));
        if let Some(MeleeResult::Blocked { next_attack, bar_cost }) = blocked {
            let g = &mut p.inv.gun;
            g.melee_damage = 0.0;
            g.next_attack = next_attack;
            if bar_cost > 0.0 {
                g.charge_melee_bar(bar_cost);
                net::send_kfu_power(me, g.melee_bar);
            }
            // A blocked hit still makes the victim the last one hit.
            p.inv.sv.lunge_victim = victim;
            return;
        }
        if source == 0 {
            // No disarm while the victim's spawn protection (`pl+0x71c`) runs.
            let now = e.time();
            crate::player::with_player(victim, |vp, vcx| {
                if vp.ts.protect_until < now
                    && let Some(d) = rules.melee_disarm(dmg, vp.ts.is_the_one)
                {
                    inventory::drop_weapon(vp, vcx, Drop::Disarm(d));
                }
            });
            if dmg > 30.0 {
                knockback(me, victim);
            }
            slow_emit(victim, c"player/kungfuhit.wav", 95 + e.random_long(0, 31));
        } else {
            let s: &std::ffi::CStr = if source == 1 { c"weapons/knife/knife_hitbody.wav" } else { c"weapons/katana/hit-body.wav" };
            slow_emit(victim, s, 95 + e.random_long(0, 31));
        }
    }
    p.inv.sv.lunge_victim = victim;
    // TS passes `g_vecZero` as the direction (`TraceAttack(attacker, dmg, g_vecZero, tr, bits)`).
    let mut info = DamageInfo::new(me, me, dmg, bits).traced();
    info.hitgroup = tr.iHitgroup;
    info.pos = tr.vecEndPos;
    // `glast_closecombat` only for a stored kung-fu attack (`pl+0x7a4 == 0`, set at 0x9c19d).
    // The hit runs inside the attacker in TS, so it is lent meanwhile (a kicked button's
    // `game_player_team` changes its team at once; `ServerHost::reach_player`).
    if source == 0 {
        crate::world::lend(me, p, || damage::glast::close_combat(|| damage::apply(victim, &info)));
    } else {
        crate::world::lend(me, p, || damage::apply(victim, &info));
    }
}

/// One delayed hit's damage and what stays stored: `|stored|` now, half of it kept, zeroed below 5
/// with a signed test (0x9c1d6..0x9c1e7, no `fabs`), so a negative (behind) damage is spent after
/// its first hit.
fn spend_delayed_damage(stored: f32) -> (f32, f32) {
    let rest = stored * 0.5;
    (stored.abs(), if rest < 5.0 { 0.0 } else { rest })
}

/// The melee sounds: TS's `EMIT_SOUND_DYN` (0xe0a8c) on `CHAN_BODY`, volume 1, attenuation 0.8,
/// slow-motion factor 0.5 (the emitting player's time scale lowers the pitch): `CloseCombatAttack`
/// 0x863b3, `ColdCockAttack`, the `Victim*Hold`s and `CheckForBreakables` 0x8c4dd all pass 0.5.
fn slow_emit(e: *mut edict_t, sample: &std::ffi::CStr, pitch: i32) {
    super::host::emit(e, CHAN_BODY, sample, 1.0, 0.8, pitch);
}

/// `friendlyfire 0` and the rules say teammates (team play only).
fn same_team(me: *mut edict_t, victim: *mut edict_t) -> bool {
    engine().cvar_float(c"mp_friendlyfire") == 0.0 && crate::rules::teams::same_team(me, victim)
}

#[cfg(test)]
mod tests {
    use super::spend_delayed_damage;

    #[test]
    fn delayed_damage_halves_and_a_backward_one_is_spent_at_once() {
        assert_eq!(spend_delayed_damage(40.0), (40.0, 20.0));
        assert_eq!(spend_delayed_damage(10.0), (10.0, 5.0));
        assert_eq!(spend_delayed_damage(9.0), (9.0, 0.0));
        // the running back kick / backward katana store a negative damage
        assert_eq!(spend_delayed_damage(-80.0), (80.0, 0.0));
    }
}
