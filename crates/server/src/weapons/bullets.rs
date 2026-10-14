//! `CBulletManager` (0x6ebe0..0x6f4f0): simulated bullets, used when `realbullet` is set or the
//! shooter is in slow motion. 256 slots, each stepped every `0.1 / (realbullet + 1)` s
//! (`realbullet >= 2`: every frame) at `velocity * 40 u/m * dt * GetBulletVelocityMult`.
//!
//! As in TS, the manager is part of the game rules (`TSGameRules + 0x1864`): `CTSGameRules::Think`
//! runs it (after its `realbullet` update) while it counts any bullet ([`start_frame`]); single
//! player and an intermission don't run that Think, so the bullets freeze. A bullet only ends by
//! hitting something.

use super::{assets, fire, projectiles};
use crate::player::Player;
use goldsrc::Global;
use goldsrc::edict::EdictPtr;
use goldsrc::server::engine;
use goldsrc::sys::*;
use shared::pm::{Arith, Ext, Fpu, Gcc};
use shared::weapons::spread::{self, Vec3};
use shared::weapons::{range_multiplier, tables};
use std::cell::RefCell;

const MAX_BULLETS: usize = 256;

/// One slot (0x40 bytes in TS; a null owner is a free slot).
#[derive(Clone, Copy)]
struct Bullet {
    /// +0x00
    owner: *mut edict_t,
    /// +0x04 position, +0x10 start of the current flight (for the range falloff), +0x1c dir.
    pos: Vec3,
    start: Vec3,
    dir: Vec3,
    /// +0x28 damage, +0x2c ammo type, +0x30 weapon id (-1 without a TS gun), +0x38 next update,
    /// +0x3c bullet type (2 after a penetration).
    damage: f32,
    ammo: u8,
    weapon: u8,
    next_update: f32,
    btype: i32,
}

struct Manager {
    slots: [Option<Bullet>; MAX_BULLETS],
    /// +0x4000: bullets counted by the last `Think` plus those fired since.
    count: i32,
}

static MANAGER: Global<RefCell<Manager>> = Global(RefCell::new(Manager { slots: [None; MAX_BULLETS], count: 0 }));

/// `CBulletManager::Reset` (from `CTSGameRules::Init`, every map).
pub fn reset() {
    let mut m = MANAGER.0.borrow_mut();
    m.slots = [None; MAX_BULLETS];
    m.count = 0;
}

/// `CBulletManager::FireBullet` 0x6eda4: the first free slot, or the warning to everyone.
pub fn fire(owner: *mut edict_t, src: Vec3, dir: Vec3, damage: f32, ammo: u8, weapon: u8, btype: i32) {
    let mut m = MANAGER.0.borrow_mut();
    let Some(i) = m.slots.iter().position(Option::is_none) else {
        drop(m);
        const HUD_PRINTTALK: u8 = 3;
        crate::rules::msg::text_all(HUD_PRINTTALK, "WARNING:cannot create bullet!\n");
        return;
    };
    m.slots[i] = Some(Bullet { owner, pos: src, start: src, dir, damage, ammo, weapon, next_update: 0.0, btype });
    m.count += 1;
}

/// The bullet part of `CTSGameRules::Think` (0x7764c): `CBulletManager::Think` while the
/// manager counts any bullet.
pub fn start_frame() {
    if MANAGER.0.borrow().count != 0 {
        think();
    }
}

/// `CBulletManager::Think` 0x6ec7c: every slot in order; a slot is counted before its update.
fn think() {
    let e = engine();
    let now = e.time();
    let rb = crate::gameplay::active().server.weapon_realbullet(assets::realbullet());
    MANAGER.0.borrow_mut().count = 0;
    for i in 0..MAX_BULLETS {
        let b = MANAGER.0.borrow().slots[i];
        let Some(mut b) = b else { continue };
        MANAGER.0.borrow_mut().count += 1;
        if b.next_update >= now {
            continue;
        }
        b.next_update = now;
        if rb < 2 {
            b.next_update += 0.1 / (rb + 1) as f32;
        }
        let keep = update(&mut b, now);
        MANAGER.0.borrow_mut().slots[i] = keep.then_some(b);
    }
}

/// `UpdateBullet`'s step (issue #150): `dt = next_update - time` (the engine's `frametime` below
/// 1 ms), `v40 = 40·velocity` (`fild` of the ammo's integer speed), and per component
/// `mult·(dt·(v40·dir))`, added to the position. The builds round it differently:
///
/// * mp.dll (0x100472f5..0x100473a3): `dt` and `mult` are stored `float`s, and each `Vector`
///   product is stored: `v40·dir`, then `·dt`, then `·mult`.
/// * `ts_i386.so` (0x6eff1..0x6f07e): `dt` and `mult` stay unrounded; `v40·dir` is kept for x and
///   y and stored for z; `·dt` and `·mult` are each stored.
///
/// `mult` is [`ServerRules::bullet_velocity_mult`](crate::gameplay::ServerRules::bullet_velocity_mult)'s
/// value for `arith`.
fn step(arith: Arith, next_update: f32, now: f32, frametime: f32, velocity: f32, dir: Vec3, mult: Ext) -> Vec3 {
    match arith {
        Arith::Msvc => {
            // `fst` stores the difference but the compare reads the unrounded one (0x100472fa).
            let d = next_update as f64 - now as f64;
            let dt = if d < 0.001f32 as f64 { frametime } else { d as f32 } as f64;
            let v40 = 40.0 * velocity as i32 as f64;
            let mult = mult.f() as f64;
            dir.map(|d| (mult * ((v40 * d as f64) as f32 as f64 * dt) as f32 as f64) as f32)
        }
        Arith::Gcc => {
            let x = Gcc::x;
            let dt = x(next_update) - x(now);
            let dt = if dt < x(0.001) { x(frametime) } else { dt };
            let v40 = x(spread::UNITS_PER_METER) * Gcc::xi(velocity as i32);
            [0, 1, 2].map(|i| {
                let p = v40 * x(dir[i]);
                let p = if i == 2 { x(p.f()) } else { p };
                (mult * x((p * dt).f())).f()
            })
        }
    }
}

/// `CBulletManager::UpdateBullet` 0x6ef5c. Returns false when the bullet is destroyed
/// (`DestroyBullet`).
fn update(b: &mut Bullet, now: f32) -> bool {
    let e = engine();
    let rules = crate::gameplay::active().server;
    // Port addition: TS reads the owner without a check; a freed edict can't be read here.
    if b.owner.is_null() || b.owner.is_free() {
        return false;
    }
    let owner = b.owner.v();
    let ammo = tables().ammo(b.ammo);
    let velocity = ammo.map(|a| a.velocity).unwrap_or(330.0);
    // The shooter's slow-motion "others" amount (pl+0x180) if it's an activator, else its
    // GiveSlowMul; bullet time is pl+0x175.
    let fuser1 = owner.fuser1;
    let (slow, btime) = crate::world::with::<Player, _>(b.owner, |p| (p.ts.slow_others, p.ts.btime)).unwrap_or((0.0, false));
    let slowmul = if slow != 0.0 { slow } else { fuser1.max(0.01) };
    let arith = crate::rules::cvars::pm_arith();
    let mult = rules.bullet_velocity_mult(arith, slowmul, btime);
    let step = step(arith, b.next_update, now, e.globals().frametime, velocity, b.dir, mult);
    let end = spread::add(b.pos, step);
    let tr = rules.bullet_trace(b.pos, end, b.owner);
    if tr.flFraction == 1.0 {
        b.pos = end;
        return true;
    }
    b.pos = tr.vecEndPos;
    // The update's own damage multiplier, from 1.0 (`bullet_damage_multiplier`); a penetration
    // (btype 2) has none. Both scale the distance as `fire::hit_distance` keeps it (mp.dll
    // 0x10047475, `ts_i386.so` 0x6f17f and 0x6f208).
    let mut adjust = 1.0;
    let amount = if b.btype == 2 {
        fn go<F: Fpu>(start: Vec3, pos: Vec3) -> Ext {
            fire::pierce_range_scaled::<F>(fire::hit_distance::<F>(start, pos))
        }
        range_multiplier(rules.bullet_range_index(shared::by_arith!(arith, go(b.start, b.pos)))) * b.damage
    } else {
        let unit = ammo.map(|a| a.range_unit).unwrap_or(80.0);
        let falloff = range_multiplier(rules.bullet_range_index(fire::range_scaled_between(arith, b.start, b.pos, unit)));
        let amount = rules.bullet_damage(b.ammo, b.weapon, b.damage, falloff, true);
        adjust = rules.bullet_damage_multiplier(&mut 1.0, assets::damage_cheat());
        amount
    };
    let bits = if b.btype == 1 { fire::DMG_SHOTGUN_TS } else { fire::DMG_BULLET_TS };
    // `glast_weaponid` names the bullet's weapon for a kill it causes (DeathMsg, logs).
    crate::damage::glast::weapon_id(b.weapon as i32, || {
        crate::damage::multi::clear();
        fire::bullet_trace_attack(b.owner, &tr, amount * adjust, b.dir, bits);
        crate::damage::multi::apply(b.owner, b.owner);
    });
    projectiles::report_damage(|| format!("damage:{:.6}\n", amount * adjust));
    let mut left = amount * ammo.map(|a| a.pierce).unwrap_or(0.0);
    if crate::rules::has_player(tr.pHit) {
        left *= 0.5;
    }
    if b.btype != 1 && b.btype != 2 && left > 5.0 {
        let p = spread::ma(tr.vecEndPos, left.min(48.0), b.dir);
        if e.point_contents(p) != CONTENTS_SOLID {
            b.pos = p;
            b.start = p;
            b.damage = left * adjust;
            b.btype = 2;
            return true;
        }
    }
    false
}

#[cfg(test)]
mod precision_tests {
    use super::*;
    use shared::pm::Arith;

    fn f(b: [u32; 3]) -> [f32; 3] {
        b.map(f32::from_bits)
    }

    fn b(v: [f32; 3]) -> [u32; 3] {
        v.map(f32::to_bits)
    }

    /// `UpdateBullet`'s step in each build's order (issue #150): the port's old
    /// `mult·(dt·v40·dir)` in `float`s gave the .so's x here.
    #[test]
    fn step_per_build() {
        let (next, now, mult) = (f32::from_bits(0x4124_1396), f32::from_bits(0x411e_f4c4), f32::from_bits(0x3ece_0e70));
        let dir = f([0x3e2a_032b, 0xbdb2_82f0, 0xbf7b_7579]);
        assert_eq!(b(step(Arith::Msvc, next, now, 0.01, 330.0, dir, Ext::from_f32(mult))), [0x438d_21a7, 0xc314_2fd1, 0xc4d0_be03]);
        assert_eq!(b(step(Arith::Gcc, next, now, 0.01, 330.0, dir, Ext::from_f32(mult))), [0x438d_21a8, 0xc314_2fd1, 0xc4d0_be03]);
        // Under 1 ms the engine's frame time is used.
        for arith in [Arith::Gcc, Arith::Msvc] {
            assert_eq!(step(arith, 1.0, 1.0, 0.01, 100.0, [1.0, 0.0, 0.0], Ext::from_f32(1.0)), [40.0, 0.0, 0.0]);
        }
    }
}
