//! TS 3.0's `CBotModuleAttack::Think` (bit 4, bots.md §8): aim, target, move and fire. The
//! module's state (`Brain::attack`) and the helpers both rulesets use are the bot layer's
//! (`ts_server::bots::attack`).
//!
//! TS quirks kept on purpose (bots.md §15): the aim test is `fabs(dot) > 0.9`, so a bot also
//! "fires" at an enemy straight behind it; a dead enemy stays the target until the 1–3 s
//! retarget; the cover search caches the enemy's origin instead of its result; and the think
//! reads the enemy before re-targeting, so the first think after spotting one only selects it.

use goldsrc::edict::EdictPtr;
use goldsrc::server::{IGNORE_MONSTERS, engine};
use goldsrc::sys::*;
use shared::weapons::{info, kind, kind_of};
use ts_server::bots::attack::{holding_melee, live};
use ts_server::bots::brain::{Brain, add, bit, btn, diff_x87, gun_position, len_x87, normalize, normalize_in, order, scale, slow, sub};
use ts_server::bots::rules::delay;
use ts_server::player::Player;
use ts_server::weapons::{self, Drop};
use ts_server::world::Cx;

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
        let target = track_point(e.v().origin, b.attack.aim_off, b.attack.lead);
        b.attack.lead = decay_lead(b.attack.lead, eng.globals().frametime);
        b.look_at(cx.pev(), target);
    }
    // (b) the throttled decisions.
    if now < b.attack.next_think {
        return;
    }
    b.attack.next_think = delay(now, 0.1, cx.pev(), b.skill, 1.0);
    select_enemy(b, cx);
    let Some(e) = live(e) else { return };
    let eorg = e.v().origin;
    let dist = len_x87(diff_x87(eorg, cx.pev().origin), order::XZY) as f32;
    if b.num_enemies() == 0 {
        // Lost sight: chase where it was, unless it died.
        b.disable(bit::ATTACK);
        if ts_server::rules::alive(e.v()) {
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
            // Circle at 128..256 units: `Vector(x, y, dz).Normalize()` inline, the squares summed
            // `z, y, x` (0x1006c86b..0x1006c886; issue #171).
            let dz = cx.pev().origin[2] - eorg[2];
            let x = eng.random_float(-1.0, 1.0);
            let y = eng.random_float(-1.0, 1.0);
            let dir = normalize_in([x, y, dz], order::ZYX);
            b.goal = add(eorg, scale(dir, eng.random_long(128, 256) as f32));
        }
        b.enable(bit::NAVIGATE);
    }
    // (c) fire when the enemy is within ~25° of the view, either way (`fabs`).
    let aim = add(eorg, b.attack.aim_off);
    let d = normalize(sub(aim, gun_position(cx.pev())));
    let (f, _, _) = eng.angle_vectors(cx.pev().v_angle);
    let dot = (d[1] as f64 * f[1] as f64 + d[2] as f64 * f[2] as f64) + d[0] as f64 * f[0] as f64;
    if dot.abs() > 0.9f32 as f64 || melee {
        fire_weapon(b, p, cx, dist);
    } else {
        release(b, now);
    }
    // (d) a jump, or a stock hit inside 96 units, at close range with a gun.
    if !melee && dist < 128.0 && b.attack.next_melee < now {
        let button = if dist < 96.0 && can_stock_hit(p) { btn::ALT2 } else { btn::JUMP };
        b.press(button, 0.6, now);
        b.attack.next_melee = delay(now, 1.5, cx.pev(), b.skill, 10.0);
    }
    if p.ts.held != 0 {
        b.enable(bit::POWERUP);
    }
}

/// The tracked point of the attack think (0x1006c551..0x1006c588): the enemy's origin plus the
/// aim offset plus the knife lead. mp.dll stores `aim + origin` as a float for y and z but keeps
/// it on the x87 stack for x.
fn track_point(origin: [f32; 3], aim: [f32; 3], lead: [f32; 3]) -> [f32; 3] {
    let x = ((aim[0] as f64 + origin[0] as f64) + lead[0] as f64) as f32;
    [x, (aim[1] + origin[1]) + lead[1], (aim[2] + origin[2]) + lead[2]]
}

/// The knife lead's decay (0x1006c58c..0x1006c5e9): `lead -= min(frametime * 5, 1) * lead`, with
/// the factor unrounded, the x/y products stored as floats and the z product kept on the stack.
fn decay_lead(lead: [f32; 3], ft: f32) -> [f32; 3] {
    let mut k = ft as f64 * 5.0;
    if 1.0 <= k {
        k = 1.0;
    }
    let step = |l: f32| l - (k * l as f64) as f32;
    [step(lead[0]), step(lead[1]), (lead[2] as f64 - k * lead[2] as f64) as f32]
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
            let d = len_x87(diff_x87(e.v().origin, me), order::ZYX);
            if d < best as f64 {
                b.enemy = e;
                best = d as f32;
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
    b.attack.next_cover = delay(now, 3.0, cx.pev(), b.skill, 7.0);
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
            let d = len_x87(diff_x87(p, org), order::ZYX) as i32; // __ftol of the unrounded length
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
                let tt = knife_flight_time(dist);
                if let Some(e) = live(b.enemy) {
                    let v = e.v().velocity;
                    let r = eng.random_float(0.75, 1.25);
                    b.attack.lead = v.map(|c| knife_lead(tt, c, r));
                }
                let r = eng.random_float(0.75, 1.25);
                b.attack.lead[2] = knife_lead(tt, 360.0, r);
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
        b.attack.next_fire = burst_next_fire(h, now);
    }
}

/// The knife throw's flight time, `dist * (1/1900)f` (`FireWeapon` 0x1006d14f): mp.dll keeps it
/// on the x87 for every product that uses it, never stored.
fn knife_flight_time(dist: f32) -> f64 {
    f64::from(dist) * f64::from(0.000_526_315_77f32)
}

/// One component of the knife lead (0x1006d17f..0x1006d1c9, 0x1006d1fb): `tt * v` stored as a
/// `float`, then times the `RandomFloat(0.75, 1.25)` factor `r` and stored again. The lift is the
/// same with `v` = 360 (`tt * 360` stored at 0x1006d169).
fn knife_lead(tt: f64, v: f32, r: f32) -> f32 {
    (tt * f64::from(v)) as f32 * r
}

/// The auto/pump burst's next decision (0x1006d347..0x1006d353): `h + time + 0.3` (a `double`)
/// on the x87, stored once.
fn burst_next_fire(h: f32, now: f32) -> f32 {
    (f64::from(h) + f64::from(now) + 0.3) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The knife's flight time stays unrounded (#154): the `f32` `tt` gave one ulp more here.
    #[test]
    fn knife_lead_uses_the_unrounded_flight_time() {
        let tt = knife_flight_time(552.132_8);
        assert_eq!(knife_lead(tt, 26.537_535, 0.934_977_6).to_bits(), 0x40e6_ba8e);
        let tt = knife_flight_time(1_991.725_2);
        assert_eq!(knife_lead(tt, 360.0, 1.168_230_8).to_bits(), 0x43dc_6ee4);
        assert_eq!(knife_lead(knife_flight_time(1900.0), 10.0, 1.0), 10.0);
    }

    /// `h + time + 0.3` rounded once (#154); `(h + time) as f32 + 0.3` gave 0x431858aa.
    #[test]
    fn burst_timer_rounds_once() {
        assert_eq!(burst_next_fire(0.713_769_7, 151.332_57).to_bits(), 0x4318_58a9);
    }

    #[test]
    fn tracking_rounds_as_mp_dll() {
        // x keeps `aim + origin` on the x87 stack before adding the lead; y and z store it.
        let (org, aim, lead) = ([1000.1f32, 1000.1, 1000.1], [0.000_031_f32; 3], [0.000_02f32; 3]);
        let p = track_point(org, aim, lead);
        assert_eq!(p[0], ((aim[0] as f64 + org[0] as f64) + lead[0] as f64) as f32);
        assert_eq!(p[1], (aim[1] + org[1]) + lead[1]);
        // The lead decays at 5/s with the factor unrounded; a long frame clears it.
        let ft = 0.013f32;
        let k = ft as f64 * 5.0;
        let l = [37.3f32, -12.9, 101.7];
        let n = decay_lead(l, ft);
        assert_eq!(n[0], l[0] - (k * l[0] as f64) as f32);
        assert_eq!(n[2], (l[2] as f64 - k * l[2] as f64) as f32);
        assert_eq!(decay_lead(l, 0.5), [0.0; 3]);
    }
}
