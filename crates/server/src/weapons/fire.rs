//! Hitscan firing: `CBasePlayer::ShootBulletsPlayer` 0x6f69c and `UTIL_TraceLine_Expanded`.

use super::{assets, bullets, projectiles};
use crate::damage::{self, DamageInfo};
use goldsrc::Global;
use goldsrc::edict::EdictPtr;
use goldsrc::ext::entities::{MsgWriterExt, te, te_begin};
use goldsrc::server::engine;
use goldsrc::sys::*;
use shared::pm::{Arith, Ext, Fpu, Num};
use shared::weapons::gun::Shot;
use shared::weapons::spread::{self, PelletSide, Vec3};
use shared::weapons::{range_multiplier, tables};

/// Damage bits: bullets, and the shotgun flag TS uses for the x1.5 headshot.
pub const DMG_BULLET_TS: i32 = 0x1002;
pub const DMG_SHOTGUN_TS: i32 = 0x0200_1002;

/// `ShootBulletsPlayer`'s static `tr` (end position, fraction): every hitscan trace writes it,
/// and the bubble trail after each pellet reads it, so a simulated bullet's trail
/// (`realbullet` / slow motion) uses the last hitscan trace, of any player and any earlier frame
/// (all zero before the first).
static LAST_TRACE: Global<std::cell::Cell<(Vec3, f32)>> = Global(std::cell::Cell::new(([0.0; 3], 0.0)));

/// Gun position (`GetGunPosition`): origin + view offset.
pub fn gun_position(v: &entvars_t) -> Vec3 {
    spread::add(v.origin, v.view_ofs)
}

/// `pEntity->TraceAttack(shooter, dmg, dir, &tr, bits)` for a bullet that hit something: the
/// hitgroup the trace reports, the damage going into the multi-damage (`damage::multi`). The
/// caller applies it.
pub fn bullet_trace_attack(shooter: *mut edict_t, tr: &TraceResult, dmg: f32, dir: Vec3, bits: i32) {
    let hit = tr.pHit;
    if hit.is_null() {
        return; // the world: `CBaseEntity::TraceAttack` with `takedamage` off does nothing
    }
    let mut info = DamageInfo::new(shooter, shooter, dmg, bits).traced();
    info.hitgroup = tr.iHitgroup;
    info.dir = dir;
    info.pos = tr.vecEndPos;
    damage::trace_attack(hit, &info);
    assets::debug(|| {
        let e = engine();
        let v = hit.v();
        format!(
            "hit {} #{} dmg={dmg:.2} hitgroup={} bits={bits:#x} health={}",
            e.string(v.classname).to_string_lossy(),
            e.index_of(hit),
            tr.iHitgroup,
            v.health
        )
    });
}

/// `CBasePlayer::ShootBulletsPlayer`: seeded pellets, range falloff, one wall penetration; or
/// simulated bullets (`realbullet`, or the shooter in slow motion).
pub fn shoot_bullets(shooter: *mut edict_t, shot: &Shot) {
    let e = engine();
    assets::debug(|| {
        format!("shoot weapon={} pellets={} spread={:.5} dmg={} seed={}", shot.weapon, shot.pellets, shot.spread, shot.damage, shot.seed)
    });
    let rules = crate::gameplay::active().server;
    let arith = crate::rules::cvars::pm_arith();
    let mut src = crate::ham::gun_position(shooter, || gun_position(shooter.v()));
    let pev = shooter.v();
    let real = rules.weapon_realbullet(assets::realbullet()) != 0 || pev.fuser1.max(0.01) < 0.99;
    let (fwd, right, up) = spread::angle_vectors(pev.v_angle);
    if shot.right_offset != 0.0 {
        src = spread::ma(src, shot.right_offset, right);
    }
    let ammo = tables().ammo(shot.ammo);
    let unit = ammo.map(|a| a.range_unit).unwrap_or(80.0);
    let pierce = ammo.map(|a| a.pierce).unwrap_or(0.0);
    let cheat = assets::damage_cheat();
    // `ShootBulletsPlayer`'s damage multiplier, reset once per call (`bullet_damage_multiplier`).
    let mut acc = 1.0f32;
    let mut adjust = 1.0f32;
    let bits = if shot.btype == 1 { DMG_SHOTGUN_TS } else { DMG_BULLET_TS };
    // Every pellet's TraceAttack adds to one multi-damage, applied once after the loop: the
    // pellets that hit one victim in a row reach its TakeDamage as one hit. Simulated bullets
    // apply their own later, but the type is set either way (0x6f7cb).
    if !real {
        damage::multi::clear();
    }
    damage::multi::set_type(DMG_BULLET_TS);

    let mut left = 0.0f32;
    let mut last: Option<(TraceResult, Vec3)> = None;
    // The pellet pattern is the ruleset's; a bot's may use the engine's `RandomFloat`.
    let gun_rules = crate::gameplay::gun_rules(shooter);
    let bot = pev.flags & FL_FAKECLIENT as i32 != 0;
    let mut random_float = |lo: f32, hi: f32| e.random_float(lo, hi);
    for i in 1..=shot.pellets {
        let bot_random: Option<&mut dyn FnMut(f32, f32) -> f32> = if bot { Some(&mut random_float) } else { None };
        let offset = gun_rules
            .pellet_offset(shot.seed, i, bot_random)
            .map_or_else(|| spread::pellet_offset(shot.seed, i, spread::RADIAL_SPREAD), spread::PelletOffset::from);
        let dir = gun_rules.pellet_dir(PelletSide::Server(arith), fwd, right, up, [shot.spread, shot.spread], offset);
        if real {
            bullets::fire(shooter, src, dir, shot.damage, shot.ammo, shot.weapon, shot.btype);
            let (end, fraction) = LAST_TRACE.0.get();
            bubble_trail(src, end, bubble_count(spread::MAX_RANGE, fraction));
            continue;
        }
        let end = spread::ma(src, spread::MAX_RANGE, dir);
        let tr = rules.bullet_trace(src, end, shooter);
        LAST_TRACE.0.set((tr.vecEndPos, tr.flFraction));
        if tr.flFraction != 1.0 {
            let falloff = range_multiplier(rules.bullet_range_index(range_scaled_between(arith, src, tr.vecEndPos, unit)));
            let amount = rules.bullet_damage(shot.ammo, shot.weapon, shot.damage, falloff, false);
            adjust = rules.bullet_damage_multiplier(&mut acc, cheat);
            bullet_trace_attack(shooter, &tr, amount * adjust, dir, bits);
            projectiles::report_damage(|| format!("damage:{:.6}\n", amount * adjust));
            left = amount * pierce;
            if crate::rules::has_player(tr.pHit) {
                left *= 0.5;
            }
        }
        bubble_trail(src, tr.vecEndPos, bubble_count(spread::MAX_RANGE, tr.flFraction));
        last = Some((tr, dir));
    }
    if real {
        return;
    }
    damage::multi::apply(shooter, shooter);
    if shot.btype == 1 || shot.btype == 2 || left <= 5.0 {
        return;
    }
    let Some((tr, dir)) = last else { return };
    let step = rules.pierce_step(left);
    let p = spread::ma(tr.vecEndPos, step, dir);
    if e.point_contents(p) != CONTENTS_SOLID {
        pierce_shot(shooter, p, dir, left * adjust);
    }
}

/// The distance from `src` to a hit at `end`, for the range falloff, as each build keeps it
/// (issue #80): mp.dll subtracts on the FPU and sums `z² + y² + x²` (`ShootBulletsPlayer`
/// 0x10047fbb..0x10047ffd, `UpdateBullet` 0x10047450..0x10047475) and leaves the root unrounded;
/// `ts_i386.so` sums `x² + y² + z²` of the unrounded differences and stores the root as a
/// `float` (0x6fd47..0x6fd7e, `UpdateBullet` 0x6f14d..0x6f181).
pub fn hit_distance<F: Fpu>(src: Vec3, end: Vec3) -> F::N {
    let d = [0, 1, 2].map(|i| F::x(end[i]) - F::x(src[i]));
    if F::MSVC { (d[2] * d[2] + d[1] * d[1] + d[0] * d[0]).sqrt() } else { F::x((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().f()) }
}

/// A penetration shot's distance (`ShootBulletsPlayer` with btype 2): mp.dll stores the
/// differences as `float`s and sums `x² + z² + y²` (0x10047b6e..0x10047bc1); `ts_i386.so` is as
/// in [`hit_distance`] (0x6f8d3..0x6f90e).
pub fn pierce_distance<F: Fpu>(src: Vec3, end: Vec3) -> F::N {
    if !F::MSVC {
        return hit_distance::<F>(src, end);
    }
    let d = [0, 1, 2].map(|i| F::x(end[i] - src[i]));
    (d[0] * d[0] + d[2] * d[2] + d[1] * d[1]).sqrt()
}

/// A hit's distance in tenths of the ammo's range unit, `dist / (unit·10)·10`, as
/// `ShootBulletsPlayer` and `UpdateBullet` compute it before the conversion to an index
/// ([`ServerRules::bullet_range_index`](crate::gameplay::ServerRules::bullet_range_index)):
/// `fild unit; fmul 10f; fdiv; fmul 10f` with nothing rounded before the truncation (mp.dll
/// 0x10048005..0x1004801c, `ts_i386.so` 0x6fd8c..0x6fdbb). The value is returned exactly.
pub fn range_scaled<F: Fpu>(dist: F::N, unit: f32) -> Ext {
    let ten = F::x(10.0);
    (dist / (F::xi(unit as i32) * ten) * ten).ext()
}

/// The same for a penetration's straight trace, a fixed 30-unit range unit:
/// `dist·0.0033333334f·10f`, unrounded (mp.dll 0x10047bc3..0x10047bd5, `ts_i386.so` 0x6f912..0x6f921).
pub fn pierce_range_scaled<F: Fpu>(dist: F::N) -> Ext {
    (dist * F::x(0.003_333_333_4) * F::x(10.0)).ext()
}

/// [`hit_distance`] then [`range_scaled`] in `arith`'s build.
pub fn range_scaled_between(arith: Arith, src: Vec3, end: Vec3, unit: f32) -> Ext {
    fn go<F: Fpu>(src: Vec3, end: Vec3, unit: f32) -> Ext {
        range_scaled::<F>(hit_distance::<F>(src, end), unit)
    }
    shared::by_arith!(arith, go(src, end, unit))
}

/// The recursion with `btype 2`: a straight 4096-unit trace from the exit point, range falloff
/// with a fixed 30-unit index unit, no further penetration.
fn pierce_shot(shooter: *mut edict_t, src: Vec3, dir: Vec3, dmg: f32) {
    damage::multi::clear();
    damage::multi::set_type(DMG_BULLET_TS);
    let rules = crate::gameplay::active().server;
    let end = spread::ma(src, 4096.0, dir);
    let tr = rules.bullet_trace(src, end, shooter);
    LAST_TRACE.0.set((tr.vecEndPos, tr.flFraction));
    if tr.flFraction != 1.0 {
        fn go<F: Fpu>(src: Vec3, end: Vec3) -> Ext {
            pierce_range_scaled::<F>(pierce_distance::<F>(src, end))
        }
        let idx = rules.bullet_range_index(shared::by_arith!(crate::rules::cvars::pm_arith(), go(src, tr.vecEndPos)));
        let amount = range_multiplier(idx) * dmg;
        bullet_trace_attack(shooter, &tr, amount, dir, DMG_BULLET_TS);
        projectiles::report_damage(|| format!("damage(piercing):{amount:.6}\n"));
    }
    bubble_trail(src, tr.vecEndPos, bubble_count(4096.0, tr.flFraction));
    damage::multi::apply(shooter, shooter);
}

/// `UTIL_WaterLevel` 0xee73c: the water surface height above `pos` between `minz` and `maxz`
/// (bisected to within 1 unit): `minz` if `pos` at `minz` is dry, `maxz` if still under water there.
fn water_level(pos: Vec3, mut minz: f32, mut maxz: f32) -> f32 {
    let e = engine();
    let at = |z: f32| e.point_contents([pos[0], pos[1], z]) == CONTENTS_WATER;
    if !at(minz) {
        return minz;
    }
    if at(maxz) {
        return maxz;
    }
    let mut z = maxz;
    while maxz - minz > 1.0 {
        z = minz + (maxz - minz) * 0.5;
        if at(z) {
            minz = z;
        } else {
            maxz = z;
        }
    }
    z
}

/// `ShootBulletsPlayer`'s bubble count, `(int)(range * fraction * 0.015625)`: the pellets' and
/// simulated bullets' `(float)ammo.range` (8192 for every ammo, 0x6ffcf..0x6fffb) and the
/// penetration's `4096.0f` (0x6fa5a..0x6fa85). The product is exact on the x87 (at most 48 bits)
/// and `(int)` truncates (`fistp` under chop; mp.dll `__ftol`), issue #80.
pub(crate) fn bubble_count(range: f32, fraction: f32) -> i32 {
    (f64::from(range) * f64::from(fraction) * 0.015625).trunc_i32()
}

/// `UTIL_BubbleTrail` 0xee93c: a `TE_BUBBLETRAIL` along a bullet's path when either end is at
/// least 8 units under water. `count` is capped at 255.
pub(crate) fn bubble_trail(from: Vec3, to: Vec3, count: i32) {
    let mut height = water_level(from, from[2], from[2] + 256.0) - from[2];
    if height < 8.0 {
        let h = water_level(to, to[2], to[2] + 256.0) - to[2];
        if h < 8.0 {
            return;
        }
        height = h + to[2] - from[2];
    }
    let model = assets::bubbles_model();
    te_begin(MSG_BROADCAST, None, te::BUBBLETRAIL).vec3(from).vec3(to).coord(height).short(model).byte(count.min(255)).coord(8.0).end();
}

#[cfg(test)]
mod precision_tests {
    use super::*;
    use shared::pm::Arith;

    fn f(b: [u32; 3]) -> [f32; 3] {
        b.map(f32::from_bits)
    }

    /// The bubble count truncates (issue #80): 0.30859375 of 8192 units is 39.5, which the old
    /// ties-even rounding sent as 40, and 0.996 is 127.488 (was 127, but 0.9 of 4096 → 57.6 was 58).
    #[test]
    fn bubble_count_truncates() {
        assert_eq!(bubble_count(spread::MAX_RANGE, 0.3), 38);
        assert_eq!(bubble_count(spread::MAX_RANGE, 0.308_593_75), 39);
        assert_eq!(bubble_count(spread::MAX_RANGE, 0.996), 127, "127.488");
        assert_eq!(bubble_count(4096.0, 0.996), 63, "the penetration's 63.744");
        assert_eq!(bubble_count(4096.0, 0.9), 57, "57.6");
        assert_eq!(bubble_count(4096.0, 1.0), 64);
    }

    fn b(v: [f32; 3]) -> [u32; 3] {
        v.map(f32::to_bits)
    }

    /// The range index truncates the unrounded `dist / (unit·10)·10` (issue #80): at 1679.9999
    /// units with an 80-unit range unit TS gets 20; the port's old `float` steps made it 21.
    #[test]
    fn range_index_truncates_the_unrounded_value() {
        let end = [f32::from_bits(0x44d1_ffff), 0.0, 0.0];
        for arith in [Arith::Gcc, Arith::Msvc] {
            assert_eq!(range_scaled_between(arith, [0.0; 3], end, 80.0).trunc_i32(), 20);
            assert_eq!(range_scaled_between(arith, [0.0; 3], [1680.0, 0.0, 0.0], 80.0).trunc_i32(), 21);
        }
        // The penetration's fixed 30-unit scale: 389.99997 is 12.99999..., truncated to 12.
        fn pierce<F: Fpu>(d: f32) -> i32 {
            pierce_range_scaled::<F>(pierce_distance::<F>([0.0; 3], [d, 0.0, 0.0])).trunc_i32()
        }
        let d = f32::from_bits(0x43c2_ffff);
        assert_eq!((pierce::<shared::pm::Msvc>(d), pierce::<shared::pm::Gcc>(d)), (12, 12));
        let _ = b(end);
    }
}
