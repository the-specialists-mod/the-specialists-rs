//! Hitscan firing: `CBasePlayer::ShootBulletsPlayer` 0x6f69c and `UTIL_TraceLine_Expanded`.

use super::{assets, bullets, projectiles};
use crate::damage::{self, DamageInfo};
use goldsrc::Global;
use goldsrc::edict::EdictPtr;
use goldsrc::ext::entities::{MsgWriterExt, te, te_begin};
use goldsrc::ext::weapons::WeaponsEngine;
use goldsrc::server::{DONT_IGNORE_MONSTERS, engine};
use goldsrc::sys::*;
use shared::weapons::gun::Shot;
use shared::weapons::spread::{self, Vec3};
use shared::weapons::{range_multiplier, tables};

/// Damage bits: bullets, and the shotgun flag TS uses for the x1.5 headshot.
pub const DMG_BULLET_TS: i32 = 0x1002;
pub const DMG_SHOTGUN_TS: i32 = 0x0200_1002;

/// `UTIL_TraceLine_Expanded` 0x6f4f4: grows every player's absolute box by 32 units for the
/// trace so hitboxes outside the bounding box (arms, head while diving) can be hit.
pub fn trace_expanded(start: Vec3, end: Vec3, skip: *mut edict_t) -> TraceResult {
    let e = engine();
    let n = e.max_clients();
    let mut saved: Vec<(*mut edict_t, [f32; 3], [f32; 3])> = Vec::with_capacity(n as usize);
    for i in 1..=n {
        let p = e.player(i);
        if p.is_null() {
            continue;
        }
        let v = p.v();
        saved.push((p, v.absmin, v.absmax));
        for k in 0..3 {
            v.absmin[k] -= 32.0;
            v.absmax[k] += 32.0;
        }
    }
    let tr = e.trace_line(start, end, DONT_IGNORE_MONSTERS, skip);
    for (p, lo, hi) in saved {
        let v = p.v();
        v.absmin = lo;
        v.absmax = hi;
    }
    tr
}

/// `ShootBulletsPlayer`'s static `tr` (end position, fraction): every hitscan trace writes it,
/// and the bubble trail after each pellet reads it, so a simulated bullet's trail
/// (`realbullet` / slow motion) uses the last hitscan trace, of any player and any earlier frame
/// (all zero before the first).
static LAST_TRACE: Global<std::cell::Cell<(Vec3, f32)>> = Global(std::cell::Cell::new(([0.0; 3], 0.0)));

/// Gun position (`GetGunPosition`): origin + view offset.
pub fn gun_position(v: &entvars_t) -> Vec3 {
    spread::add(v.origin, v.view_ofs)
}

/// Applies bullet damage from `tr` (TraceAttack + the hitgroup the trace reports).
pub fn bullet_damage(shooter: *mut edict_t, tr: &TraceResult, dmg: f32, dir: Vec3, bits: i32, weapon: u8) {
    let hit = tr.pHit;
    if hit.is_null() || dmg <= 0.0 {
        return;
    }
    let mut info = DamageInfo::new(shooter, shooter, dmg, bits).traced();
    info.hitgroup = tr.iHitgroup;
    info.dir = dir;
    info.pos = tr.vecEndPos;
    info.weapon = weapon as i32;
    let ok = damage::apply(hit, &info);
    assets::debug(|| {
        let e = engine();
        let v = hit.v();
        format!(
            "hit {} #{} dmg={dmg:.2} hitgroup={} bits={bits:#x} applied={ok} health={}",
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
    let pev = shooter.v();
    let real = assets::realbullet() != 0 || pev.fuser1.max(0.01) < 0.99;
    let (fwd, right, up) = spread::angle_vectors(pev.v_angle);
    let mut src = gun_position(pev);
    if shot.right_offset != 0.0 {
        src = spread::ma(src, shot.right_offset, right);
    }
    let ammo = tables().ammo(shot.ammo);
    let unit = ammo.map(|a| a.range_unit).unwrap_or(80.0);
    let pierce = ammo.map(|a| a.pierce).unwrap_or(0.0);
    let adjust = assets::damage_adjust();
    let bits = if shot.btype == 1 { DMG_SHOTGUN_TS } else { DMG_BULLET_TS };

    let mut left = 0.0f32;
    let mut last: Option<(TraceResult, Vec3)> = None;
    for i in 1..=shot.pellets {
        let dir = spread::pellet_dir(fwd, right, up, [shot.spread, shot.spread], shot.seed, i, spread::RADIAL_SPREAD);
        if real {
            bullets::fire(shooter, src, dir, shot.damage, shot.ammo, shot.weapon, shot.btype);
            let (end, fraction) = LAST_TRACE.0.get();
            bubble_trail(src, end, (spread::MAX_RANGE * fraction / 64.0).round_ties_even() as i32);
            continue;
        }
        let end = spread::ma(src, spread::MAX_RANGE, dir);
        let tr = trace_expanded(src, end, shooter);
        LAST_TRACE.0.set((tr.vecEndPos, tr.flFraction));
        if tr.flFraction != 1.0 {
            let dist = spread::length(spread::sub(src, tr.vecEndPos));
            let amount = range_multiplier(spread::range_index(dist, unit)) * shot.damage;
            bullet_damage(shooter, &tr, amount * adjust, dir, bits, shot.weapon);
            projectiles::report_damage(|| format!("damage:{:.6}\n", amount * adjust));
            left = amount * pierce;
            if e.is_player_edict(tr.pHit) {
                left *= 0.5;
            }
        }
        bubble_trail(src, tr.vecEndPos, (spread::MAX_RANGE * tr.flFraction / 64.0).round_ties_even() as i32);
        last = Some((tr, dir));
    }
    if real || shot.btype == 1 || shot.btype == 2 || left <= 5.0 {
        return;
    }
    let Some((tr, dir)) = last else { return };
    let step = (left + left).min(48.0);
    let p = spread::ma(tr.vecEndPos, step, dir);
    if e.point_contents(p) != CONTENTS_SOLID {
        pierce_shot(shooter, p, dir, left * adjust, shot.weapon);
    }
}

/// The recursion with `btype 2`: a straight 4096-unit trace from the exit point, range falloff
/// with a fixed 30-unit index unit, no further penetration.
fn pierce_shot(shooter: *mut edict_t, src: Vec3, dir: Vec3, dmg: f32, weapon: u8) {
    let end = spread::ma(src, 4096.0, dir);
    let tr = trace_expanded(src, end, shooter);
    LAST_TRACE.0.set((tr.vecEndPos, tr.flFraction));
    if tr.flFraction != 1.0 {
        let dist = spread::length(spread::sub(src, tr.vecEndPos));
        let idx = (dist * 0.003_333_333_4 * 10.0).round_ties_even() as i32;
        let amount = range_multiplier(idx) * dmg;
        bullet_damage(shooter, &tr, amount, dir, DMG_BULLET_TS, weapon);
        projectiles::report_damage(|| format!("damage(piercing):{amount:.6}\n"));
    }
    bubble_trail(src, tr.vecEndPos, (tr.flFraction * 4096.0 / 64.0).round_ties_even() as i32);
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

/// `UTIL_BubbleTrail` 0xee93c: a `TE_BUBBLETRAIL` along a bullet's path when either end is at
/// least 8 units under water. `count` is capped at 255.
fn bubble_trail(from: Vec3, to: Vec3, count: i32) {
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
