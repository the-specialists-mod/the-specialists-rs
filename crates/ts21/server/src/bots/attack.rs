//! 2.1's `CBotModuleAttack` (bit 4): `Think` 0x1005e5e0, `SelectEnemy` 0x1005ed60,
//! `FindCoverSpot` 0x1005e9a0, `FireWeapon` 0x1005ee60.
//!
//! Against 3.0's: no per-frame tracking (the aim is set once per think, at a ±32-unit offset
//! divided by `max(skill, 0.5)`), no facing test before firing, a melee bot always charges, the
//! close-range jump has no stock hit, the cover search reaches 1024 units (2048 and a forward
//! fan for a far enemy) and caches what it returns, and the fire control's holds differ.

use super::{look_at, slow_inv, timer};
use goldsrc::edict::EdictPtr;
use goldsrc::server::{IGNORE_MONSTERS, engine};
use goldsrc::sys::*;
use shared::weapons::{kind, kind_of};
use ts_server::bots::attack::{holding_melee, live};
use ts_server::bots::brain::{Brain, bit, btn, diff_x87, len_x87, order};
use ts_server::player::Player;
use ts_server::rules::alive;
use ts_server::weapons::{self, Drop};
use ts_server::world::Cx;

/// `ReleaseAttack` (0x1005f230): `ReleaseButton(IN_ATTACK2)`, then `IN_ATTACK`.
fn release_attack(b: &mut Brain, now: f32) {
    b.release(btn::ATTACK2, now);
    b.release(btn::ATTACK, now);
}

/// `CBotModuleAttack::Think` (0x1005e5e0), every `min(slowInv * 0.1 / skill, 1)` s.
pub fn think(b: &mut Brain, p: &mut Player, cx: &mut Cx) {
    let now = cx.time();
    if now < b.attack.next_think {
        return;
    }
    b.attack.next_think = timer(now, 0.1, cx.pev(), b.skill, 1.0);
    select_enemy(b, cx);
    // Guard: TS keeps a raw `CBaseEntity *`; an enemy that lost its player is no target.
    let Some(e) = live(b.enemy) else { return };
    let eorg = e.v().origin;
    // `(z² + y²) + x²` of the unrounded differences, stored as a float (0x1005e668..0x1005e698).
    let dist = len_x87(diff_x87(eorg, cx.pev().origin), order::ZYX) as f32;
    if b.num_enemies() == 0 {
        b.disable(bit::ATTACK);
        if alive(e.v()) {
            b.last_seen = eorg;
            b.enemy = std::ptr::null_mut();
        }
        release_attack(b, now);
        return;
    }
    let eng = engine();
    let s = if b.skill <= 0.5 { 0.5 } else { b.skill as f64 };
    let inv = 1.0 / s;
    let x = eng.random_long(-32, 32);
    let y = eng.random_long(-32, 32);
    let z = eng.random_long(-16, 32);
    let off = [(x as f64 * inv) as f32, (y as f64 * inv) as f32, (z as f64 * inv) as f32];
    look_at(cx.pev(), [off[0] + eorg[0], off[1] + eorg[1], off[2] + eorg[2]]);
    if !holding_melee(p) {
        b.goal = find_cover_spot(b, cx, eorg, e);
        b.go_straight = true;
        b.enable(bit::NAVIGATE);
        if b.attack.next_dodge < now {
            b.attack.next_dodge = (eng.random_long(5, 10) as f64 + now as f64) as f32;
            if eng.random_long(0, 100) > 70 {
                let t = eng.random_float(0.9, 1.6);
                b.press(btn::DUCK, t, now);
            } else {
                b.enable(bit::STUNT);
            }
        }
    } else {
        b.go_straight = true;
        b.goal = eorg;
        b.enable(bit::NAVIGATE);
    }
    fire_weapon(b, p, cx, dist);
    if dist < 128.0 && b.attack.next_melee < now {
        b.press(btn::JUMP, 0.6, now);
        b.attack.next_melee = timer(now, 1.0, cx.pev(), b.skill, 10.0);
    }
    if p.ts.held != 0 {
        b.enable(bit::POWERUP);
    }
}

/// `SelectEnemy` (0x1005ed60): with no enemy or every 3 s (fixed), the nearest listed enemy,
/// by `(z² + x²) + y²` against the best so far kept as a float.
pub fn select_enemy(b: &mut Brain, cx: &mut Cx) {
    let now = cx.time();
    if !b.enemy.is_null() && now < b.attack.next_retarget {
        return;
    }
    b.attack.next_retarget = (now as f64 + 3.0) as f32;
    let me = cx.pev().origin;
    let mut best = 9999.0f32;
    for j in 0..16 {
        let Some(o) = live(b.visible_enemies[j]) else { continue };
        let d = len_x87(diff_x87(o.v().origin, me), order::ZXY);
        if d < best as f64 {
            b.enemy = o;
            best = d as f32;
        }
    }
}

/// `FindCoverSpot` (0x1005e9a0), every `min(slowInv * 3 / skill, 7)` s (else the cached spot):
/// eight directions 45° apart from a random one, 1024 units; for an enemy more than 1024 units
/// away four directions 22° apart from 44° left of it, 2048 units. The first end point the enemy
/// can't see and the bot reaches wins, else the farthest seen one the bot reaches, else the
/// enemy's origin. The result is cached.
pub fn find_cover_spot(b: &mut Brain, cx: &mut Cx, enemy_org: [f32; 3], enemy: *mut edict_t) -> [f32; 3] {
    let now = cx.time();
    if now < b.attack.next_cover {
        return b.attack.cover_cache;
    }
    b.attack.next_cover = timer(now, 3.0, cx.pev(), b.skill, 7.0);
    let eng = engine();
    let me = cx.edict;
    let (mut dirs, mut step, mut radius) = (8, 45, 1024);
    let mut ang = eng.random_long(0, 8).wrapping_mul(45);
    let org = cx.pev().origin;
    if len_x87(diff_x87(enemy_org, org), order::ZYX) > 1024.0 {
        let d = [enemy_org[0] - org[0], enemy_org[1] - org[1], enemy_org[2] - org[2]];
        let yaw = eng.vec_to_angles(d)[1];
        ang = (super::angle_mod(yaw) - 44.0).trunc() as i32;
        (dirs, step, radius) = (4, 22, 2048);
    }
    let radius = radius as f64;
    let mut best = enemy_org;
    let mut best_dist = 0i32;
    for _ in 1..=dirs {
        let org = cx.pev().origin;
        let rad = ang as f64 * 0.017_453_292_519_943_295;
        let (cx_, sy) = ((rad.cos() * radius) as f32, (rad.sin() * radius) as f32);
        let end = [cx_ + org[0], sy + org[1], org[2]];
        let p = eng.trace_line(org, end, IGNORE_MONSTERS, me).vecEndPos;
        let seen = eng.trace_line(enemy_org, p, IGNORE_MONSTERS, enemy).flFraction >= 1.0;
        let reach = eng.trace_line(org, p, IGNORE_MONSTERS, me).flFraction >= 1.0;
        if !seen {
            if reach {
                b.attack.cover_cache = p;
                return p;
            }
        } else if reach {
            let d = len_x87(diff_x87(p, org), order::ZYX) as i32;
            if best_dist < d {
                best_dist = d;
                best = p;
            }
        }
        ang = ang.wrapping_add(step);
    }
    b.attack.cover_cache = best;
    best
}

/// `min(max(slowInv * 0.3 / skill, 0.3), 3)` as `FireWeapon` writes it (a NaN gives 3).
#[allow(clippy::neg_cmp_op_on_partial_ord)]
pub fn kung_fu_wait(slow_inv: f64, skill: f32) -> f64 {
    let x = slow_inv * 0.3 / skill as f64;
    if x <= 0.3 || x < 3.0 { if x <= 0.3 { 0.3 } else { x } } else { 3.0 }
}

/// `FireWeapon` (0x1005ee60), once `next_fire` is due, with the gun the active item.
pub fn fire_weapon(b: &mut Brain, p: &mut Player, cx: &mut Cx, dist: f32) {
    let now = cx.time();
    if now < b.attack.next_fire || !weapons::gun_active(p) {
        return;
    }
    let eng = engine();
    let cur = p.inv.gun.current;
    let Some(st) = p.inv.gun.slot().copied() else {
        // ---- kung fu: kick (80/101) or punch inside 128 units; a better weapon past 256 ----
        if dist < 128.0 {
            if eng.random_long(0, 100) < 81 {
                b.press(btn::ATTACK2, 0.3, now);
                b.release(btn::ATTACK, now);
                if eng.random_long(0, 100) > 80 {
                    if eng.random_long(0, 100) > 50 {
                        b.press(btn::JUMP, 0.1, now);
                    }
                    b.press(btn::DUCK, 0.1, now);
                }
            } else {
                b.press(btn::ATTACK, 0.3, now);
                b.release(btn::ATTACK2, now);
            }
            b.attack.next_fire = (now as f64 + 0.9) as f32;
        } else if dist > 256.0 {
            weapons::get_better_weapon(p, cx, 0);
            b.attack.next_fire = (now as f64 + kung_fu_wait(slow_inv(cx.pev()), b.skill)) as f32;
        }
        return;
    };
    match kind_of(cur) {
        kind::GRENADE => {
            b.press(btn::ATTACK, 0.1, now);
            b.attack.next_fire = (now as f64 + 1.0) as f32;
            return;
        }
        kind::KATANA => {
            if dist < 128.0 {
                b.press(btn::ATTACK, 0.5, now);
                if eng.random_long(0, 100) > 80 {
                    b.press(btn::DUCK, 0.1, now);
                }
            }
            b.attack.next_fire = (now as f64 + 0.5) as f32;
            return;
        }
        kind::KNIFE => {
            // Throw (attack2) 30/101 or past 128 units, else slash; no lead.
            if eng.random_long(0, 100) > 70 || dist > 128.0 {
                b.press(btn::ATTACK2, 0.1, now);
                b.release(btn::ATTACK, now);
            } else {
                b.press(btn::ATTACK, 0.1, now);
            }
            b.attack.next_fire = (now as f64 + 0.3) as f32;
            return;
        }
        _ => {}
    }
    if st.clip > 0 {
        if st.mode != 0 && st.mode != 3 {
            let h = eng.random_float(0.1, 0.2);
            b.press(btn::ATTACK, h, now);
            b.attack.next_fire = h + now;
        } else {
            // `fld h; fadd time; fadd 0.3`: the sum isn't rounded before the 0.3.
            let h = eng.random_float(0.3, 1.7);
            b.press(btn::ATTACK, h, now);
            b.attack.next_fire = (h as f64 + now as f64 + 0.3) as f32;
        }
        return;
    }
    b.release(btn::ATTACK, now);
    if p.inv.gun.current_reserve() < 1 {
        if eng.random_long(0, 100) < 51 {
            weapons::drop_weapon(p, cx, Drop::Current);
        } else {
            weapons::get_better_weapon(p, cx, cur);
        }
    } else {
        b.press(btn::RELOAD, 0.1, now);
    }
    b.attack.next_fire = (now as f64 + 1.0) as f32;
}
