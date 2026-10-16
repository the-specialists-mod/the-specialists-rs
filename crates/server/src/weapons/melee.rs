//! Melee hit resolution: `CBasePlayer::CheckHull` 0x95e28, `CloseCombatAttack` 0x95f88,
//! `ColdCockAttack` 0x96610, the knife/katana slash traces, the blocks (`VictimKungfuHold`
//! 0x76f50, `VictimKnifeHold` 0x76de4, `VictimKatanaHold` 0x76c8c) and the delayed "lunge" hit
//! of a missed melee attack (the melee-window part of `CTSStunt::CheckForBreakables`).

use super::inventory::{self, Drop};
use super::net;
use crate::damage::{self, DamageInfo};
use crate::messages::{self, Msg};
use crate::player::Player;
use crate::world::Cx;
use goldsrc::edict::EdictPtr;
use goldsrc::server::{DONT_IGNORE_MONSTERS, engine};
use goldsrc::sys::*;
use shared::pm::{Arith, Fpu, FpuMath, Num, ORDER_XYZ, eq87};
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
    let arith = crate::rules::cvars::pm_arith();
    let mut distance = 1e6f32;
    for i in 0..2 {
        for j in 0..2 {
            for k in 0..2 {
                let end = [hull_end[0] + mm[i][0], hull_end[1] + mm[j][1], hull_end[2] + mm[k][2]];
                let t = e.trace_line(src, end, DONT_IGNORE_MONSTERS, skip);
                if t.flFraction < 1.0
                    && let Some(d) = shared::by_arith!(arith, hull_closer(t.vecEndPos, src, distance))
                {
                    *tr = t;
                    distance = d;
                }
            }
        }
    }
}

/// `FindHullIntersection`'s `thisDistance = (tmpTrace.vecEndPos - vecSrc).Length()` and `if
/// (thisDistance < distance)`: the new `float` distance when the corner is closer. mp.dll
/// (`FUN_10063590`, 0x10063724..0x1006376e) sums the unrounded differences `z, y, x` and compares
/// the root unrounded; `ts_i386.so` (0x95d7f..0x95ded) squares each unrounded difference times
/// its stored copy (`x, y`, then `z²`) and compares the stored root. Both take a NaN as closer.
fn hull_closer<F: Fpu>(end: Vec3, src: Vec3, distance: f32) -> Option<f32> {
    let d = [0, 1, 2].map(|i| F::x(end[i]) - F::x(src[i]));
    let len = if F::MSVC {
        (d[2] * d[2] + d[1] * d[1] + d[0] * d[0]).sqrt()
    } else {
        F::x((d[0] * F::x(d[0].f()) + d[1] * F::x(d[1].f()) + d[2] * d[2]).sqrt().f())
    };
    shared::pm::below87(len, F::x(distance)).then(|| len.f())
}

/// `IsBSPModel()` of the trace's hit (`CheckHull`); NULL counts as the world.
fn is_bsp(e: *mut edict_t) -> bool {
    e.is_null() || crate::ham::is_bsp_model(e)
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
    crate::ham::is_alive(e)
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
    // `m_pPlayer->GetGunPosition()` (`ColdCock`, `KatanaFire`, `KnifeFire`, `CCombatAttack`).
    let start = crate::ham::gun_position(attacker, || {
        let v = attacker.v();
        spread::add(v.origin, v.view_ofs)
    });
    let pev = attacker.v();
    let mut ang = pev.v_angle;
    if hit.zero_roll {
        ang[2] = 0.0;
    }
    let (fwd, _, _) = spread::angle_vectors(ang);
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
                knockback(attacker, victim, false);
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

/// `v.vel += |a.vel| * dir(a -> v)`, `v.vel.z = 235`. `delayed` is `CheckForBreakables`' delayed
/// hit, whose inline `Normalize` mp.dll compiles differently from the attacks'.
fn knockback(attacker: *mut edict_t, victim: *mut edict_t, delayed: bool) {
    let a = attacker.v();
    let v = victim.v();
    v.velocity = knockback_velocity(crate::rules::cvars::pm_arith(), a.velocity, a.origin, v.origin, v.velocity, delayed);
    v.velocity[2] = 235.0;
}

/// The knock-back before `velocity.z = 235` (issue #150): the attacker's speed (`x² + y² + z²`,
/// stored as a `float` in both builds) along the normalised `victim.origin - attacker.origin`,
/// added to the victim's velocity with the x product kept on the FPU and the y and z products
/// stored. The direction is per build:
///
/// * mp.dll keeps the x difference unrounded and stores y and z. `CloseCombatAttack` /
///   `ColdCockAttack` (0x10063bb9..0x10063c36, 0x100640bd..0x1006413f) square z as
///   `dz·(float)dz` (an `fst` then `fmul` of the stored copy) and sum `z, y, x`; the delayed hit
///   (`CheckForBreakables` 0x1006a494..0x1006a518) sums `x, z, y`. Length and `1/len` stay on
///   the FPU, x is normalised from the unrounded difference.
/// * `ts_i386.so` (0x962b8..0x96344, delayed 0x9c312..0x9c3a0) sums `x, y, z` of the unrounded
///   differences, stores the length, and normalises the stored differences.
pub(crate) fn knockback_velocity(arith: Arith, av: Vec3, ao: Vec3, vo: Vec3, vv: Vec3, delayed: bool) -> Vec3 {
    fn go<F: Fpu>(av: Vec3, ao: Vec3, vo: Vec3, vv: Vec3, delayed: bool) -> Vec3 {
        let x = F::x;
        let speed = x(F::sq_in(av, ORDER_XYZ).sqrt().f());
        let d = [0, 1, 2].map(|i| x(vo[i]) - x(ao[i]));
        let ds = d.map(|e| x(e.f()));
        let len = if !F::MSVC {
            x((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().f())
        } else if delayed {
            (d[0] * d[0] + ds[2] * ds[2] + ds[1] * ds[1]).sqrt()
        } else {
            (d[2] * ds[2] + ds[1] * ds[1] + d[0] * d[0]).sqrt()
        };
        let dir = if eq87(len, F::ZERO) {
            [0.0, 0.0, 1.0]
        } else {
            let inv = x(1.0) / len;
            let dx = if F::MSVC { d[0] } else { ds[0] };
            [(inv * dx).f(), (inv * ds[1]).f(), (inv * ds[2]).f()]
        };
        [(speed * x(dir[0]) + x(vv[0])).f(), vv[1] + (speed * x(dir[1])).f(), vv[2] + (speed * x(dir[2])).f()]
    }
    shared::by_arith!(arith, go(av, ao, vo, vv, delayed))
}

/// The delayed hit of a missed melee attack (`CTSStunt::CheckForBreakables` 0x9bd9c with
/// `pl+0x79c` open, alive or not): while the window is open, a head-hull trace 32 u ahead (behind
/// for a negative damage) lands the stored damage, halving it each time. TS never tests the stored
/// damage: once it is spent (or a block zeroed it), each new target in reach still takes a
/// 0-damage hit (`TraceAttack`/`TakeDamage`: blood, pain and damage sounds, a shootable button
/// pressed) and becomes the last victim (#135).
pub fn pending_hit(p: &mut Player, cx: &mut Cx) {
    let g = &mut p.inv.gun;
    if g.melee_window == 0.0 {
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
    let rules = crate::gameplay::active().server;
    let is_player = crate::rules::has_player(victim);
    let player_part = delayed_player_part(is_player, dmg, rules.zero_delayed_hit_reaches_player());
    if player_part && same_team(me, victim) {
        // TS: `friendlyfire 0` and PlayerRelationship == GR_TEAMMATE: no damage, but the
        // victim still counts as hit.
        p.inv.sv.lunge_victim = victim;
        return;
    }
    if player_part {
        let kind = match source {
            0 => MeleeKind::KungFu,
            1 => MeleeKind::Knife,
            _ => MeleeKind::Katana,
        };
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
                knockback(me, victim, true);
            }
            slow_emit(victim, c"player/kungfuhit.wav", 95 + e.random_long(0, 31));
        } else {
            let s: &std::ffi::CStr = if source == 1 { c"weapons/knife/knife_hitbody.wav" } else { c"weapons/katana/hit-body.wav" };
            slow_emit(victim, s, 95 + e.random_long(0, 31));
        }
    }
    p.inv.sv.lunge_victim = victim;
    // The ruleset's direction: 3.0 passes `g_vecZero`, 2.1 the stunt direction.
    let mut info = DamageInfo::new(me, me, dmg, bits).traced();
    info.dir = rules.delayed_hit_dir(p.stunt.dir);
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

/// Whether a delayed hit of `dmg` runs the player part (the team check, the victim's block,
/// disarm, knockback and the hit sound) before its `TraceAttack`: only on a player, and in TS 3.0
/// only with some damage (0x8c1f1: `!IsPlayer || dmg <= 0` goes straight to the hit), unless the
/// ruleset's [`zero_delayed_hit_reaches_player`](crate::gameplay::ServerRules::zero_delayed_hit_reaches_player).
fn delayed_player_part(is_player: bool, dmg: f32, zero_reaches_player: bool) -> bool {
    is_player && (dmg > 0.0 || zero_reaches_player)
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
    use super::{delayed_player_part, spend_delayed_damage};

    /// #135: a spent delayed hit still hits; 3.0 skips only the player part, a ruleset that
    /// reaches the player runs it with 0 damage.
    #[test]
    fn a_spent_delayed_hit_skips_only_the_player_part() {
        assert!(delayed_player_part(true, 7.5, false));
        assert!(!delayed_player_part(true, 0.0, false), "3.0: straight to the 0-damage hit");
        assert!(delayed_player_part(true, 0.0, true));
        assert!(!delayed_player_part(false, 15.0, true), "not a player");
    }

    #[test]
    fn delayed_damage_halves_and_a_backward_one_is_spent_at_once() {
        assert_eq!(spend_delayed_damage(40.0), (40.0, 20.0));
        assert_eq!(spend_delayed_damage(10.0), (10.0, 5.0));
        assert_eq!(spend_delayed_damage(9.0), (9.0, 0.0));
        // the running back kick / backward katana store a negative damage
        assert_eq!(spend_delayed_damage(-80.0), (80.0, 0.0));
    }
}

#[cfg(test)]
mod precision_tests {
    use super::*;
    use shared::pm::Arith;

    fn f(b: [u32; 3]) -> [f32; 3] {
        b.map(f32::from_bits)
    }

    /// `FindHullIntersection`'s corner distance (issue #171): mp.dll's unrounded 740.99998 beats a
    /// kept 741.0; the `.so`'s stored root (and the port's old `float` length) ties it.
    #[test]
    fn hull_corner_distance_rounds_as_each_build() {
        use shared::pm::{Gcc, Msvc};
        let (end, src) = (f([0xc2da_01de, 0x445a_d2ed, 0x44bd_680a]), f([0x4285_7810, 0x431b_6fc8, 0x44bd_680a]));
        assert_eq!(hull_closer::<Msvc>(end, src, 741.0), Some(741.0));
        assert_eq!(hull_closer::<Gcc>(end, src, 741.0), None);
        assert_eq!(hull_closer::<Gcc>(end, src, 742.0), Some(741.0));
        assert!(hull_closer::<Msvc>([f32::NAN; 3], src, 1e6).is_some_and(f32::is_nan));
        assert!(hull_closer::<Gcc>([f32::NAN; 3], src, 1e6).is_some_and(f32::is_nan));
    }

    fn b(v: [f32; 3]) -> [u32; 3] {
        v.map(f32::to_bits)
    }

    /// mp.dll's knock-back keeps the x difference, the length and `1/len` unrounded and squares
    /// differently for a direct and a delayed hit; the .so (and the port's old `float` maths, by
    /// chance here) stores them (issue #150).
    #[test]
    fn knockback_per_build() {
        let (av, ao) = (f([0x4371_ae44, 0x436e_cff0, 0xc3ac_4d57]), f([0x448a_2b8c, 0xc448_fe8c, 0x43d0_f8c0]));
        let (vo, vv) = (f([0xc485_fde3, 0xc4a8_bf70, 0xc456_865a]), f([0x4319_edf2, 0x430b_96c6, 0xc292_e948]));
        assert_eq!(b(knockback_velocity(Arith::Msvc, av, ao, vo, vv, false)), [0xc37e_24d9, 0x4215_059a, 0xc39c_4ea3]);
        assert_eq!(b(knockback_velocity(Arith::Gcc, av, ao, vo, vv, false)), [0xc37e_24da, 0x4215_059a, 0xc39c_4ea3]);
        let (av, ao) = (f([0x4310_3748, 0x43a4_2fb8, 0x4287_aa6c]), f([0x44be_a85e, 0x4493_e9ae, 0x449c_1ae8]));
        let (vo, vv) = (f([0xc46e_57da, 0x44c2_0a52, 0x44a1_a6da]), f([0x4309_2330, 0x4250_2c30, 0xc38e_9ee3]));
        assert_eq!(b(knockback_velocity(Arith::Msvc, av, ao, vo, vv, true)), [0xc35f_d4c8, 0x42d3_9190, 0xc38b_63ba]);
        assert_eq!(b(knockback_velocity(Arith::Gcc, av, ao, vo, vv, true)), [0xc35f_d4ca, 0x42d3_9190, 0xc38b_63ba]);
        // A victim on the attacker's origin is pushed straight up by its speed.
        assert_eq!(knockback_velocity(Arith::Msvc, [3.0, 4.0, 0.0], [1.0; 3], [1.0; 3], [0.0; 3], false), [0.0, 0.0, 5.0]);
    }
}
