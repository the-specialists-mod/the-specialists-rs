//! Damage plumbing shared by every subsystem. Weapons build a `DamageInfo` and call `apply`;
//! the victim's `Entity::take_damage` decides what happens (players: player/damage.rs).

use crate::world;
use goldsrc::sys::*;

/// Damage bits (SDK `DMG_*`) used by TS.
pub mod bits {
    pub const GENERIC: i32 = 0;
    pub const CRUSH: i32 = 1 << 0;
    pub const BULLET: i32 = 1 << 1;
    pub const SLASH: i32 = 1 << 2;
    pub const BURN: i32 = 1 << 3;
    pub const FALL: i32 = 1 << 5;
    pub const BLAST: i32 = 1 << 6;
    pub const CLUB: i32 = 1 << 7;
    pub const SHOCK: i32 = 1 << 8;
    pub const DROWN: i32 = 1 << 14;
    pub const NEVERGIB: i32 = 1 << 12;
    pub const ALWAYSGIB: i32 = 1 << 13;
    /// TS: thrown knife/katana (part of the armour-exempt mask 0x80040e4, weapons.md §5.9).
    pub const THROWN: i32 = 0x0800_0000;
    /// Damage types kevlar never absorbs.
    pub const ARMOR_EXEMPT: i32 = 0x0800_40e4;
}

/// Hitgroups (studio hitboxes).
pub mod hitgroup {
    pub const GENERIC: i32 = 0;
    pub const HEAD: i32 = 1;
    pub const CHEST: i32 = 2;
    pub const STOMACH: i32 = 3;
    pub const LEFTARM: i32 = 4;
    pub const RIGHTARM: i32 = 5;
    pub const LEFTLEG: i32 = 6;
    pub const RIGHTLEG: i32 = 7;
}

#[derive(Clone, Copy, Debug)]
pub struct DamageInfo {
    /// What did the damage (bullet owner, grenade, trigger...).
    pub inflictor: *mut edict_t,
    /// Who gets the credit.
    pub attacker: *mut edict_t,
    pub damage: f32,
    pub bits: i32,
    pub hitgroup: i32,
    /// Direction of travel (normalised) and impact point, when known.
    pub dir: [f32; 3],
    pub pos: [f32; 3],
    /// TS weapon id that did it, or -1.
    pub weapon: i32,
}

impl DamageInfo {
    pub fn new(inflictor: *mut edict_t, attacker: *mut edict_t, damage: f32, bits: i32) -> Self {
        Self { inflictor, attacker, damage, bits, hitgroup: hitgroup::GENERIC, dir: [0.0; 3], pos: [0.0; 3], weapon: -1 }
    }
}

/// Deals damage to `victim`. Returns false if it isn't game-owned or refused the damage.
///
/// Re-entrancy: if `victim` is the entity currently being dispatched (self-damage, e.g. your own
/// grenade), world::dispatch can't reach it and this returns false; handle self-damage at the
/// call site with the `&mut` you already hold.
pub fn apply(victim: *mut edict_t, info: &DamageInfo) -> bool {
    if victim.is_null() || unsafe { (*victim).v.takedamage } == 0.0 {
        return false;
    }
    world::dispatch(victim, |e, cx| e.take_damage(cx, info)).unwrap_or(false)
}
