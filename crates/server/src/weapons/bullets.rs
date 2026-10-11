//! `CBulletManager` (0x6ebe0..0x6f4f0): simulated bullets, used when `realbullet` is set or the
//! shooter is in slow motion. 256 slots, each stepped every `0.1 / (realbullet + 1)` s
//! (`realbullet >= 2`: every frame) at `velocity * 40 u/m * dt * GetBulletVelocityMult`.
//!
//! As in TS, the manager is part of the game rules (`TSGameRules + 0x1864`): `CTSGameRules::Think`
//! runs it from `StartFrame` while it counts any bullet ([`start_frame`]). A bullet only ends by
//! hitting something.

use super::{assets, fire, projectiles};
use crate::player::Player;
use goldsrc::Global;
use goldsrc::edict::EdictPtr;
use goldsrc::ext::weapons::WeaponsEngine;
use goldsrc::server::engine;
use goldsrc::sys::*;
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
    let rb = assets::realbullet();
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

/// `CBulletManager::UpdateBullet` 0x6ef5c. Returns false when the bullet is destroyed
/// (`DestroyBullet`).
fn update(b: &mut Bullet, now: f32) -> bool {
    let e = engine();
    // Port addition: TS reads the owner without a check; a freed edict can't be read here.
    if b.owner.is_null() || b.owner.is_free() {
        return false;
    }
    let owner = b.owner.v();
    let ammo = tables().ammo(b.ammo);
    let velocity = ammo.map(|a| a.velocity).unwrap_or(330.0);
    let mut dt = b.next_update - now;
    if dt < 0.001 {
        dt = e.globals().frametime;
    }
    // The shooter's slow-motion "others" amount (pl+0x180) if it's an activator, else its
    // GiveSlowMul; bullet time is pl+0x175.
    let fuser1 = owner.fuser1;
    let (slow, btime) = crate::world::with::<Player, _>(b.owner, |p| (p.ts.slow_others, p.ts.btime)).unwrap_or((0.0, false));
    let slowmul = if slow != 0.0 { slow } else { fuser1.max(0.01) };
    let mult = spread::bullet_velocity_mult(slowmul, btime);
    // `mult * (dt * (velocity * 40) * dir)`, in TS's order (0x6f038..).
    let v40 = velocity * spread::UNITS_PER_METER;
    let step = [mult * (dt * v40 * b.dir[0]), mult * (dt * v40 * b.dir[1]), mult * (dt * v40 * b.dir[2])];
    let end = spread::add(b.pos, step);
    let tr = fire::trace_expanded(b.pos, end, b.owner);
    if tr.flFraction == 1.0 {
        b.pos = end;
        return true;
    }
    b.pos = tr.vecEndPos;
    let dist = spread::length(spread::sub(b.start, b.pos));
    let mut adjust = 1.0;
    let amount = if b.btype == 2 {
        range_multiplier((dist * 0.003_333_333_4 * 10.0).round_ties_even() as i32) * b.damage
    } else {
        adjust = assets::damage_adjust();
        let unit = ammo.map(|a| a.range_unit).unwrap_or(80.0);
        range_multiplier(spread::range_index(dist, unit)) * b.damage
    };
    let bits = if b.btype == 1 { fire::DMG_SHOTGUN_TS } else { fire::DMG_BULLET_TS };
    fire::bullet_damage(b.owner, &tr, amount * adjust, b.dir, bits, b.weapon);
    projectiles::report_damage(|| format!("damage:{:.6}\n", amount * adjust));
    let mut left = amount * ammo.map(|a| a.pierce).unwrap_or(0.0);
    if e.is_player_edict(tr.pHit) {
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
