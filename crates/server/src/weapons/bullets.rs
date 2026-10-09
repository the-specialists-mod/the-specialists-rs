//! `CBulletManager` (0x6ebe0..0x6f4f0): simulated bullets, used when `realbullet` is set or the
//! shooter is in slow motion. Up to 256 bullets step every `0.1 / (realbullet + 1)` s
//! (`realbullet >= 2`: every frame) at `velocity * 40 u/m * dt * GetBulletVelocityMult`.
//!
//! TS drives it from `CTSGameRules::Think`; here a hidden `ts_bulletmanager` entity thinks every
//! frame while bullets are in flight (so hits on any player, including the one whose hooks
//! fired the bullet, dispatch normally).

use super::{assets, fire};
use crate::entities::{self, link_entities};
use crate::world::{Cx, Entity};
use goldsrc::Global;
use goldsrc::ext::weapons::WeaponsEngine;
use goldsrc::server::engine;
use goldsrc::sys::*;
use shared::weapons::spread::{self, Vec3};
use shared::weapons::{range_multiplier, tables};
use std::cell::{Cell, RefCell};

link_entities! {
    ts_bulletmanager => BulletManager,
}

const MAX_BULLETS: usize = 256;
/// Bullets that travel this far without hitting anything are dropped (TS has no limit).
const MAX_TRAVEL: f32 = 16384.0;

#[derive(Clone, Copy)]
struct Bullet {
    owner: *mut edict_t,
    pos: Vec3,
    start: Vec3,
    dir: Vec3,
    damage: f32,
    ammo: u8,
    weapon: u8,
    btype: i32,
    next_update: f32,
    travelled: f32,
}

static BULLETS: Global<RefCell<Vec<Bullet>>> = Global(RefCell::new(Vec::new()));
static MANAGER: Global<Cell<*mut edict_t>> = Global(Cell::new(std::ptr::null_mut()));

/// Forget everything (new map).
pub fn reset() {
    BULLETS.0.borrow_mut().clear();
    MANAGER.0.set(std::ptr::null_mut());
}

/// `CBulletManager::FireBullet`.
pub fn fire(owner: *mut edict_t, src: Vec3, dir: Vec3, damage: f32, ammo: u8, weapon: u8, btype: i32) {
    {
        let mut b = BULLETS.0.borrow_mut();
        if b.len() >= MAX_BULLETS {
            engine().print("WARNING:cannot create bullet!\n");
            return;
        }
        b.push(Bullet { owner, pos: src, start: src, dir, damage, ammo, weapon, btype, next_update: 0.0, travelled: 0.0 });
    }
    ensure_manager();
}

/// Creates the manager entity if bullets are pending and none is running.
pub fn ensure_manager() {
    if BULLETS.0.borrow().is_empty() || !MANAGER.0.get().is_null() {
        return;
    }
    let e = engine().create_named_entity(c"ts_bulletmanager");
    if e.is_null() {
        return;
    }
    MANAGER.0.set(e);
    crate::world::dispatch(e, |ent, cx| ent.spawn(cx));
}

#[derive(Default)]
pub struct BulletManager;

impl Entity for BulletManager {
    fn spawn(&mut self, cx: &mut Cx) {
        let pev = cx.pev();
        pev.classname = engine().alloc_string(c"ts_bulletmanager");
        pev.solid = SOLID_NOT as i32;
        pev.movetype = MOVETYPE_NONE as i32;
        pev.effects |= EF_NODRAW as i32;
        pev.nextthink = engine().time() + 0.001;
    }

    fn think(&mut self, cx: &mut Cx) {
        run();
        if BULLETS.0.borrow().is_empty() {
            MANAGER.0.set(std::ptr::null_mut());
            entities::remove(cx.edict);
        } else {
            cx.pev().nextthink = engine().time() + 0.001;
        }
    }
}

/// `CBulletManager::Think`.
fn run() {
    let e = engine();
    let now = e.time();
    let rb = assets::cvars().realbullet as i32;
    let mut i = 0;
    loop {
        let b = {
            let list = BULLETS.0.borrow();
            match list.get(i) {
                Some(b) => *b,
                None => break,
            }
        };
        if b.next_update >= now {
            i += 1;
            continue;
        }
        let mut b = b;
        b.next_update = now + if rb < 2 { 0.1 / (rb + 1) as f32 } else { 0.0 };
        let keep = update(&mut b, now);
        let mut list = BULLETS.0.borrow_mut();
        if keep {
            list[i] = b;
            i += 1;
        } else {
            list.remove(i);
        }
    }
}

/// `CBulletManager::UpdateBullet` 0x6ef5c. Returns false when the bullet is destroyed.
fn update(b: &mut Bullet, now: f32) -> bool {
    let e = engine();
    if b.owner.is_null() || unsafe { (*b.owner).free != 0 } {
        return false;
    }
    let owner = unsafe { &(*b.owner).v };
    let ammo = tables().ammo(b.ammo);
    let velocity = ammo.map(|a| a.velocity).unwrap_or(330.0);
    let mut dt = b.next_update - now;
    if dt < 0.001 {
        dt = e.globals().frametime;
    }
    // Bullet time (pl+0x175) belongs to the movement package; treated as off.
    let mult = spread::bullet_velocity_mult(owner.fuser1.max(0.0), false);
    let step = spread::scale(b.dir, velocity * spread::UNITS_PER_METER * dt * mult);
    let end = spread::add(b.pos, step);
    let tr = fire::trace_expanded(b.pos, end, b.owner);
    if tr.flFraction == 1.0 {
        b.pos = end;
        b.travelled += spread::length(step);
        return b.travelled < MAX_TRAVEL;
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
