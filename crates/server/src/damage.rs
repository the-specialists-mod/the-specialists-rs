//! Damage plumbing shared by every subsystem. Weapons build a `DamageInfo` and call `apply`;
//! the victim's `Entity::take_damage` decides what happens (players: player/damage.rs).

use crate::world;
use goldsrc::edict::EdictPtr;
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
    /// First of the eight time-based types (`player::water::TimeBasedDamage`): paralyze, nerve
    /// gas, poison, radiation, drown recover, acid, slow burn, slow freeze.
    pub const PARALYZE: i32 = 1 << 15;
    pub const POISON: i32 = 1 << 17;
    pub const DROWNRECOVER: i32 = 1 << 19;
    pub const ACID: i32 = 1 << 20;
    /// `DMG_TIMEBASED` (`~0x3fff`): the mask `CheckTimeBasedDamage` (0xda00c) gates on. It also
    /// covers `DROWN` and every bit above the eight time-based ones.
    pub const TIMEBASED: i32 = !0x3fff;
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
    /// Dealt through the victim's `TraceAttack` (vtable +0x54) then `ApplyMultiDamage`, rather
    /// than a direct `TakeDamage` (+0x58). `damage::apply` is a single entry point, so this is how
    /// a victim knows to run its TraceAttack step (players: spawn protection, stunt ×0.7, team
    /// check, hitgroup, `PTakeDam`; breakables and doors: their hit effects). Set it with
    /// `DamageInfo::traced()` in every caller that TS routes through TraceAttack: bullets
    /// (`CBulletManager::UpdateBullet`, `ShootBulletsPlayer`, `FireBullets`), melee
    /// (`KnifeFire`, `KatanaFire`, `CloseCombatAttack`, `ColdCockAttack`,
    /// `CTSStunt::CheckForBreakables`), `TSExplosion` (also when occluded), thrown/falling
    /// objects (`CTSKnife::BounceTouch`/`WaitThink`, `CM61Grenade::WaitThink`,
    /// `CTSWorldGun::Fall`), `CBeam::BeamDamage` and `RadiusDamage` when its trace hit something.
    /// Crushers, `trigger_hurt`, falling, drowning, time-based damage, telefrags, team changes,
    /// breakable touch and Xen trees call `TakeDamage` directly and leave it false.
    pub traced: bool,
}

impl DamageInfo {
    pub fn new(inflictor: *mut edict_t, attacker: *mut edict_t, damage: f32, bits: i32) -> Self {
        Self { inflictor, attacker, damage, bits, hitgroup: hitgroup::GENERIC, dir: [0.0; 3], pos: [0.0; 3], weapon: -1, traced: false }
    }

    /// The same damage, dealt through `TraceAttack` (sets the `traced` field).
    pub fn traced(mut self) -> Self {
        self.traced = true;
        self
    }
}

/// Deals damage to `victim`. Returns false if it isn't game-owned or refused the damage.
///
/// Re-entrancy: if `victim` is the entity currently being dispatched (self-damage, e.g. your own
/// grenade), world::dispatch can't reach it and this returns false; handle self-damage at the
/// call site with the `&mut` you already hold.
pub fn apply(victim: *mut edict_t, info: &DamageInfo) -> bool {
    if victim.is_null() || victim.v().takedamage == 0.0 {
        return false;
    }
    world::dispatch(victim, |e, cx| e.take_damage(cx, info)).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traced_is_explicit() {
        let null = std::ptr::null_mut();
        let direct = DamageInfo::new(null, null, 10.0, bits::BULLET);
        assert!(!direct.traced, "bits and hitgroup don't imply a TraceAttack");
        let traced = DamageInfo::new(null, null, 10.0, bits::BLAST).traced();
        assert!(traced.traced, "an occluded blast keeps hitgroup GENERIC but is still traced");
        assert_eq!(traced.hitgroup, hitgroup::GENERIC);
        assert_eq!((traced.damage, traced.bits, traced.weapon), (10.0, bits::BLAST, -1));
    }
}
