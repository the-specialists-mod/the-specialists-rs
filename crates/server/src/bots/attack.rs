//! `CBotModuleAttack` (bit 4, bots.md §8): aim, target, move and fire.
//!
//! TS quirks kept on purpose (bots.md §15): the aim test is `fabs(dot) > 0.9`, so a bot also
//! "fires" at an enemy straight behind it; a dead enemy stays the target until the 1–3 s
//! retarget; the cover search caches the enemy's origin instead of its result; and the think
//! reads the enemy before re-targeting, so the first think after spotting one only selects it.

use super::brain::{Brain, add, bit, btn, delay, gun_position, length, normalize, scale, slow, sub};
use crate::player::Player;
use crate::weapons::{self, Drop};
use crate::world::Cx;
use goldsrc::edict::EdictPtr;
use goldsrc::server::{IGNORE_MONSTERS, engine};
use goldsrc::sys::*;
use shared::weapons::{info, kind, kind_of};

/// The module's fields (brain+0x18ac).
#[derive(Clone, Copy, Debug, Default)]
pub struct Attack {
    /// +0x04 next think, +0x08 next fire decision, +0x0c next retarget, +0x10 next dodge,
    /// +0x14 cover spot valid until, +0x18 next close-range jump / stock hit.
    pub next_think: f32,
    pub next_fire: f32,
    pub next_retarget: f32,
    pub next_dodge: f32,
    pub next_cover: f32,
    pub next_melee: f32,
    /// +0x1c: the cached cover spot.
    pub cover_cache: [f32; 3],
    /// +0x28: aim offset from the enemy's origin, re-rolled each think.
    pub aim_off: [f32; 3],
    /// +0x34: knife-throw lead, decaying every frame.
    pub lead: [f32; 3],
}

/// A player edict the brain points at, if it still has a player (guard: TS keeps a raw
/// `CBaseEntity *`; the port's edict handle may outlive the player).
fn live(e: *mut edict_t) -> Option<*mut edict_t> {
    (!e.is_null() && crate::rules::has_player(e)).then_some(e)
}

/// `BotHoldingMelee` (0x1006cfc0): the gun is the active item and holds kung fu (no weapon
/// status), a knife or a katana.
fn holding_melee(p: &Player) -> bool {
    if !weapons::gun_active(p) {
        return false;
    }
    let cur = p.inv.gun.current;
    cur == 0 || matches!(kind_of(cur), kind::KNIFE | kind::KATANA)
}

/// `BotCanStockHit` (0x1006cf90): the current weapon has stock (cold-cock) damage.
fn can_stock_hit(p: &Player) -> bool {
    weapons::gun_active(p) && info(p.inv.gun.current).is_some_and(|w| w.stock > 0)
}

/// `ReleaseAttack` (0x1006d540).
fn release(b: &mut Brain, now: f32) {
    b.press(btn::ATTACK | btn::ATTACK2, 0.0, now);
}

/// `CBotModuleAttack::Think` (0x1006c520).
pub fn think(b: &mut Brain, p: &mut Player, cx: &mut Cx) {
    let now = cx.time();
    let eng = engine();
    // The enemy from before `SelectEnemy`.
    let e = b.enemy;
    // (a) every frame: track the enemy, with the knife lead decaying at 5/s.
    if let Some(e) = live(e)
        && b.num_enemies() != 0
    {
        let target = add(add(e.v().origin, b.attack.aim_off), b.attack.lead);
        let mut k = eng.globals().frametime * 5.0;
        if 1.0 <= k {
            k = 1.0;
        }
        b.attack.lead = sub(b.attack.lead, scale(b.attack.lead, k));
        b.look_at(cx.pev(), target);
    }
    // (b) the throttled decisions.
    if now < b.attack.next_think {
        return;
    }
    b.attack.next_think = delay(now, 0.1, slow(cx.pev()), b.skill, 1.0);
    select_enemy(b, cx);
    let Some(e) = live(e) else { return };
    let eorg = e.v().origin;
    let dist = length(sub(eorg, cx.pev().origin));
    if b.num_enemies() == 0 {
        // Lost sight: chase where it was, unless it died.
        b.disable(bit::ATTACK);
        if crate::rules::alive(e.v()) {
            b.last_seen = eorg;
            b.enemy = std::ptr::null_mut();
        }
        release(b, now);
        return;
    }
    compute_aim_offset(b);
    let melee = holding_melee(p);
    if !melee {
        b.goal = find_cover_spot(b, cx, eorg, e);
        b.go_straight = true;
        b.enable(bit::NAVIGATE);
        if b.attack.next_dodge < now {
            b.attack.next_dodge = (eng.random_long(5, 10) as f32 as f64 + now as f64) as f32;
            if eng.random_long(0, 100) <= 70 {
                b.enable(bit::STUNT);
            } else {
                let t = eng.random_float(0.9, 1.6);
                b.press(btn::DUCK, t, now);
            }
        }
    } else {
        b.go_straight = true;
        let r = eng.random_long(30, 60);
        if r as f32 <= p.inv.gun.melee_bar {
            b.goal = eorg; // charge straight in
        } else {
            // Circle at 128..256 units.
            let dz = cx.pev().origin[2] - eorg[2];
            let x = eng.random_float(-1.0, 1.0);
            let y = eng.random_float(-1.0, 1.0);
            let dir = normalize([x, y, dz]);
            b.goal = add(eorg, scale(dir, eng.random_long(128, 256) as f32));
        }
        b.enable(bit::NAVIGATE);
    }
    // (c) fire when the enemy is within ~25° of the view, either way (`fabs`).
    let aim = add(eorg, b.attack.aim_off);
    let d = normalize(sub(aim, gun_position(cx.pev())));
    let (f, _, _) = eng.angle_vectors(cx.pev().v_angle);
    let dot = d[0] as f64 * f[0] as f64 + d[2] as f64 * f[2] as f64 + d[1] as f64 * f[1] as f64;
    if dot.abs() > 0.9f32 as f64 || melee {
        fire_weapon(b, p, cx, dist);
    } else {
        release(b, now);
    }
    // (d) a jump, or a stock hit inside 96 units, at close range with a gun.
    if !melee && dist < 128.0 && b.attack.next_melee < now {
        let button = if dist < 96.0 && can_stock_hit(p) { btn::ALT2 } else { btn::JUMP };
        b.press(button, 0.6, now);
        b.attack.next_melee = delay(now, 1.5, slow(cx.pev()), b.skill, 10.0);
    }
    if p.ts.held != 0 {
        b.enable(bit::POWERUP);
    }
}

/// `SelectEnemy` (0x1006cec0): every 1–3 s (or with no enemy), the nearest visible enemy by
/// origin. It never clears the enemy.
fn select_enemy(b: &mut Brain, cx: &mut Cx) {
    let now = cx.time();
    if !b.enemy.is_null() && now < b.attack.next_retarget {
        return;
    }
    b.attack.next_retarget = (engine().random_float(1.0, 3.0) as f64 + now as f64) as f32;
    let mut best = 9999.0f32;
    let me = cx.pev().origin;
    let mut i = 0i8;
    while i < b.num_enemies() {
        if let Some(&e) = b.visible_enemies.get(i as usize)
            && let Some(e) = live(e)
        {
            let d = length(sub(e.v().origin, me));
            if d < best {
                b.enemy = e;
                best = d;
            }
        }
        i += 1;
    }
}

/// `ComputeAimOffset` (0x1006c3c0): ±8 units of jitter per axis and a height (body 0..32 or the
/// eyes; the origin for a stunting or ducking enemy), all divided by `max(skill, 0.5)`.
fn compute_aim_offset(b: &mut Brain) {
    let eng = engine();
    let Some(e) = live(b.enemy) else {
        b.attack.aim_off = [0.0; 3];
        return;
    };
    let ev = e.v();
    let z = if ev.iuser4 & 0x10c30 == 0 && ev.flags & FL_DUCKING as i32 == 0 {
        let r = eng.random_long(0, 100);
        if r as f32 as f64 <= 50.0 / b.skill as f64 { eng.random_float(0.0, 32.0) } else { ev.view_ofs[2] }
    } else {
        0.0
    };
    let s = if b.skill <= 0.5 { 0.5 } else { b.skill };
    let x = eng.random_float(-8.0, 8.0);
    let y = eng.random_float(-8.0, 8.0);
    let w = eng.random_float(-8.0, 8.0);
    b.attack.aim_off = [x / s, y / s, ((w as f64 + z as f64) / s as f64) as f32];
}

/// `FindCoverSpot` (0x1006cbc0, guns only): from a random start, eight directions 45° apart,
/// 512 units at the bot's height. The first end point the enemy can't see (and the bot can
/// reach in a straight line) wins; else the farthest visible one the bot can reach, else the
/// enemy's origin. Until the timer runs out every call returns the cache, which TS fills with
/// the enemy's origin after a search that found no hiding spot.
fn find_cover_spot(b: &mut Brain, cx: &mut Cx, enemy_org: [f32; 3], enemy: *mut edict_t) -> [f32; 3] {
    let now = cx.time();
    if now < b.attack.next_cover {
        return b.attack.cover_cache;
    }
    b.attack.next_cover = delay(now, 3.0, slow(cx.pev()), b.skill, 7.0);
    let eng = engine();
    let me = cx.edict;
    let mut ang = eng.random_long(0, 8) * 45;
    let mut best = enemy_org;
    let mut best_dist = 0i32;
    for _ in 1..=8 {
        let org = cx.pev().origin;
        let rad = ang as f64 * 0.017_453_292_519_943_295;
        let end = [(rad.cos() * 512.0) as f32 + org[0], (rad.sin() * 512.0 + org[1] as f64) as f32, org[2]];
        let p = eng.trace_line(org, end, IGNORE_MONSTERS, me).vecEndPos;
        let seen = eng.trace_line(enemy_org, p, IGNORE_MONSTERS, enemy).flFraction >= 1.0;
        let reach = eng.trace_line(org, p, IGNORE_MONSTERS, me).flFraction >= 1.0;
        if !seen {
            if reach {
                b.attack.cover_cache = p;
                return p;
            }
        } else if reach {
            let d = length(sub(p, org)) as i32; // __ftol
            if best_dist < d {
                best_dist = d;
                best = p;
            }
        }
        ang += 45;
    }
    b.attack.cover_cache = enemy_org;
    best
}

/// `FireWeapon` (0x1006d010): per weapon kind, once `next_fire` is due.
fn fire_weapon(b: &mut Brain, p: &mut Player, cx: &mut Cx, dist: f32) {
    let now = cx.time();
    if now < b.attack.next_fire || !weapons::gun_active(p) {
        return;
    }
    let eng = engine();
    let cur = p.inv.gun.current;
    let Some(st) = p.inv.gun.slot().copied() else {
        // ---- kung fu ----
        if 128.0 <= dist {
            if dist <= 256.0 && eng.random_long(0, 100) < 0x5b {
                b.attack.next_fire = (now as f64 + 0.5) as f32;
                return;
            }
            weapons::get_better_weapon(p, cx, 0);
            let x = 0.3 / slow(cx.pev()) as f64 / b.skill as f64;
            let x = x.clamp(0.3, 3.0);
            b.attack.next_fire = (now as f64 + x) as f32;
            return;
        }
        let r = eng.random_float(0.1, 0.9);
        let cap = if 1.5 <= b.skill { 1.5 } else { b.skill };
        let h = (cap as f64 * r as f64) as f32;
        if eng.random_long(0, 100) < 0x51 {
            b.press(btn::ATTACK2, h, now); // kick
            b.press(btn::ATTACK, 0.0, now);
            if eng.random_long(0, 100) >= 0x33 {
                b.press(btn::JUMP, 0.1, now);
                if eng.random_long(0, 100) >= 0x5b {
                    b.press(btn::DUCK, 0.1, now);
                }
            }
        } else {
            b.press(btn::ATTACK, h, now); // punch
            b.press(btn::ATTACK2, 0.0, now);
        }
        b.attack.next_fire = (h as f64 + now as f64) as f32;
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
                if eng.random_long(0, 100) > 0x50 {
                    b.press(btn::DUCK, 0.1, now);
                }
            }
            b.attack.next_fire = (now as f64 + 0.5) as f32;
            return;
        }
        kind::KNIFE => {
            if st.clip < 2 || (eng.random_long(0, 100) < 0x51 && dist <= 256.0) {
                if dist < 128.0 {
                    b.press(btn::ATTACK, 0.1, now); // slash
                }
            } else {
                // Throw, leading the enemy by its velocity over the flight time and lifting the
                // aim for the arc.
                b.press(btn::ATTACK2, 0.1, now);
                b.press(btn::ATTACK, 0.0, now);
                let tt = dist * 0.000_526_315_77;
                if let Some(e) = live(b.enemy) {
                    let v = e.v().velocity;
                    let r = eng.random_float(0.75, 1.25) as f64;
                    b.attack.lead = [((tt * v[0]) as f64 * r) as f32, ((tt * v[1]) as f64 * r) as f32, ((tt * v[2]) as f64 * r) as f32];
                }
                let r = eng.random_float(0.75, 1.25) as f64;
                b.attack.lead[2] = (r * (tt * 360.0) as f64) as f32;
            }
            b.attack.next_fire = (now as f64 + 0.3) as f32;
            return;
        }
        _ => {}
    }
    if st.clip < 1 {
        // Reload, or drop / switch away from an empty gun.
        b.press(btn::ATTACK, 0.0, now);
        if p.inv.gun.current_reserve() < 1 {
            if eng.random_long(0, 100) < 0x33 {
                weapons::drop_weapon(p, cx, Drop::Current);
            } else {
                weapons::get_better_weapon(p, cx, cur);
            }
        } else {
            b.press(btn::RELOAD, 0.1, now);
        }
        b.attack.next_fire = (now as f64 + 1.0) as f32;
        return;
    }
    if st.mode != 0 && st.mode != 3 {
        // Semi, burst, ...: taps; the timer equals the hold, so a one-frame release follows.
        let h = eng.random_float(0.1, 0.3);
        b.press(btn::ATTACK, h, now);
        b.attack.next_fire = h + now;
    } else {
        // Auto and pump: bursts.
        let h = eng.random_float(0.3, 1.0);
        b.press(btn::ATTACK, h, now);
        b.attack.next_fire = ((h + now) as f64 + 0.3) as f32;
    }
}
