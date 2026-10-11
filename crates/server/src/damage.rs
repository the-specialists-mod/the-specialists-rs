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
    /// Dealt as TS's `ClearMultiDamage`, `victim->TraceAttack` (vtable +0x54),
    /// `ApplyMultiDamage(inflictor, attacker)` sequence rather than a direct `TakeDamage` (+0x58):
    /// [`apply`] then runs the victim's [`world::Entity::trace_attack`] (players: spawn
    /// protection, stunt ×0.7, team check, hitgroup, `PTakeDam`; breakables and doors: their hit
    /// effects), which adds to the multi-damage, and the summed damage reaches `take_damage` with
    /// this field false. Set it with `DamageInfo::traced()` in every caller that TS routes through
    /// one TraceAttack between a Clear and an Apply: `CBulletManager::UpdateBullet`,
    /// `FireBullets` (one shot per `func_tank` call), melee (`KnifeFire`, `KatanaFire`,
    /// `CloseCombatAttack`, `ColdCockAttack`, `CTSStunt::CheckForBreakables`), `TSExplosion`
    /// (also when occluded), thrown/falling objects (`CTSKnife::BounceTouch`/`WaitThink`,
    /// `CTSShrapnel::BounceTouch`, `CM61Grenade::WaitThink`, `CTSWorldGun::Fall`,
    /// `CTSFlag::Fall`), `CBeam::BeamDamage` and `RadiusDamage` when its trace hit something.
    /// `ShootBulletsPlayer` traces every pellet before one Apply, so it drives [`multi`] itself.
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
/// A `traced` hit is TS's `ClearMultiDamage`, `victim->TraceAttack(attacker, …)`,
/// `ApplyMultiDamage(inflictor, attacker)`; it returns whether the victim's `TakeDamage` took
/// the damage (false when its TraceAttack added nothing). Like TS's callers it doesn't check
/// `takedamage` first: each TraceAttack does (a `func_breakable` still sparks, a door still runs
/// its own check), and callers that test it in TS (melee, explosions) test it themselves.
/// Otherwise this is a direct `TakeDamage`, skipped with `takedamage` off.
///
/// Re-entrancy: if `victim` is the entity currently being dispatched (self-damage, e.g. your own
/// grenade), world::dispatch can't reach it and this returns false; handle self-damage at the
/// call site with the `&mut` you already hold.
pub fn apply(victim: *mut edict_t, info: &DamageInfo) -> bool {
    if victim.is_null() {
        return false;
    }
    if info.traced {
        multi::clear();
        trace_attack(victim, info);
        return multi::apply(info.inflictor, info.attacker);
    }
    if victim.v().takedamage == 0.0 {
        return false;
    }
    world::dispatch(victim, |e, cx| e.take_damage(cx, info)).unwrap_or(false)
}

/// `victim->TraceAttack(info.attacker, info.damage, info.dir, &tr, info.bits)` (vtable +0x54):
/// the victim's hit effects, adding what it takes to the [`multi`] damage. Nothing happens to an
/// edict the game doesn't own (or that is being dispatched further up the stack).
pub fn trace_attack(victim: *mut edict_t, info: &DamageInfo) {
    if victim.is_null() {
        return;
    }
    world::dispatch(victim, |e, cx| e.trace_attack(cx, info));
}

/// TS's multi-damage (`gMultiDamage` 0x19f250: entity, amount, type, and `glast_headshot`):
/// `TraceAttack`s add to it and `ApplyMultiDamage` hands the sum to one `TakeDamage`, so the
/// pellets of one shotgun shot that hit the same victim in a row become one hit.
pub mod multi {
    use super::DamageInfo;
    use crate::world;
    use goldsrc::Global;
    use goldsrc::edict::EdictPtr;
    use goldsrc::sys::edict_t;
    use std::cell::Cell;

    /// `gMultiDamage`, plus what the port's `take_damage` reads beyond TS's four arguments
    /// (hitgroup, direction, impact point, weapon id: from the victim's latest TraceAttack).
    #[derive(Clone, Copy, Debug)]
    pub struct MultiDamage {
        pub entity: *mut edict_t,
        pub amount: f32,
        pub bits: i32,
        pub meta: Option<DamageInfo>,
    }

    impl MultiDamage {
        pub const EMPTY: Self = Self { entity: std::ptr::null_mut(), amount: 0.0, bits: 0, meta: None };

        /// The `TakeDamage` arguments `ApplyMultiDamage(inflictor, attacker)` passes, or `None`
        /// without a victim.
        pub fn take_damage_info(&self, inflictor: *mut edict_t, attacker: *mut edict_t) -> Option<DamageInfo> {
            if self.entity.is_null() {
                return None;
            }
            let mut info = self.meta.unwrap_or(DamageInfo::new(inflictor, attacker, 0.0, 0));
            info.inflictor = inflictor;
            info.attacker = attacker;
            info.damage = self.amount;
            info.bits = self.bits;
            info.traced = false;
            Some(info)
        }
    }

    static MULTI: Global<Cell<MultiDamage>> = Global(Cell::new(MultiDamage::EMPTY));
    static LAST_HEADSHOT: Global<Cell<bool>> = Global(Cell::new(false));

    /// `ClearMultiDamage` (0xf0d0c): no victim, no damage, no type. `glast_headshot` is left alone.
    pub fn clear() {
        MULTI.0.set(MultiDamage::EMPTY);
    }

    /// `gMultiDamage.type = bits`: `ShootBulletsPlayer` (0x6f69c) and `FireBullets` (0xb39a4)
    /// start the type at `DMG_BULLET | DMG_NEVERGIB` (0x1002) after the Clear.
    pub fn set_type(bits: i32) {
        let mut m = MULTI.0.get();
        m.bits = bits;
        MULTI.0.set(m);
    }

    /// `AddMultiDamage` on `cell`; `flush` is the `ApplyMultiDamage` of the old victim.
    fn add_to(cell: &Cell<MultiDamage>, victim: *mut edict_t, amount: f32, bits: i32, meta: &DamageInfo, flush: impl FnOnce()) {
        if victim.is_null() {
            return;
        }
        let mut m = cell.get();
        m.bits |= bits;
        let switch = victim != m.entity;
        cell.set(m);
        if switch {
            flush();
            let mut m = cell.get();
            m.entity = victim;
            m.amount = 0.0;
            cell.set(m);
        }
        let mut m = cell.get();
        m.amount += amount;
        m.meta = Some(*meta);
        cell.set(m);
    }

    /// `AddMultiDamage(pevInflictor, victim, amount, bits)` (0xf0d84), from the end of a
    /// TraceAttack (`attacker` is the TraceAttack's `pevAttacker`). The type keeps every bit since
    /// the last Clear, even across a change of victim; a different victim first gets the damage
    /// collected so far applied to the old one, with `attacker` as inflictor and attacker.
    pub fn add(attacker: *mut edict_t, victim: *mut edict_t, amount: f32, bits: i32, meta: &DamageInfo) {
        add_to(&MULTI.0, victim, amount, bits, meta, || {
            apply(attacker, attacker);
        });
    }

    /// `ApplyMultiDamage` on `cell`/`headshot`; `take` is the victim's `TakeDamage`.
    fn apply_from(
        cell: &Cell<MultiDamage>,
        headshot: &Cell<bool>,
        inflictor: *mut edict_t,
        attacker: *mut edict_t,
        take: impl FnOnce(*mut edict_t, &DamageInfo) -> bool,
    ) -> bool {
        let m = cell.get();
        let Some(info) = m.take_damage_info(inflictor, attacker) else {
            return false;
        };
        let took = take(m.entity, &info);
        headshot.set(false);
        took
    }

    /// `ApplyMultiDamage(inflictor, attacker)` (0xf0d38): the victim's `TakeDamage` with the sum,
    /// then `glast_headshot = 0`. The multi-damage is not cleared, so a second Apply would hit
    /// again (no TS caller does that). Returns what `TakeDamage` returned.
    pub fn apply(inflictor: *mut edict_t, attacker: *mut edict_t) -> bool {
        apply_from(&MULTI.0, &LAST_HEADSHOT.0, inflictor, attacker, |victim, info| {
            // Guard: TS keeps a raw `CBaseEntity *`; an edict freed since its TraceAttack has no
            // object to call.
            !victim.is_free() && world::dispatch(victim, |e, cx| e.take_damage(cx, info)).unwrap_or(false)
        })
    }

    /// `glast_headshot = 1` (`CBasePlayer::TraceAttack`, 0x90341): a bullet or thrown hit in the
    /// head since the last `ApplyMultiDamage` with a victim.
    pub fn set_headshot() {
        LAST_HEADSHOT.0.set(true);
    }

    /// `glast_headshot`, which `PlayerKilled` (0x895e4) reads for the `#TS_Headshot` message.
    pub fn headshot() -> bool {
        LAST_HEADSHOT.0.get()
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::damage::bits;

        fn fake(n: usize) -> *mut edict_t {
            std::ptr::without_provenance_mut(n * 0x100)
        }

        #[test]
        fn pellets_on_one_victim_sum_into_one_take_damage() {
            let cell = Cell::new(MultiDamage::EMPTY);
            let hs = Cell::new(true);
            let (shooter, a) = (fake(1), fake(2));
            cell.set(MultiDamage { bits: 0x1002, ..MultiDamage::EMPTY });
            let meta = DamageInfo::new(shooter, shooter, 0.0, 0x0200_1002);
            for _ in 0..3 {
                // Only the first Add "flushes", the cleared (null) victim.
                add_to(&cell, a, 7.5, 0x0200_1002, &meta, || assert!(cell.get().entity.is_null(), "same victim never flushes"));
            }
            let mut calls = Vec::new();
            let took = apply_from(&cell, &hs, shooter, shooter, |v, info| {
                calls.push((v, info.damage, info.bits, info.traced));
                true
            });
            assert!(took);
            assert_eq!(calls, vec![(a, 22.5, 0x0200_1002, false)]);
            assert!(!hs.get(), "Apply resets glast_headshot");
            // Not cleared: a second Apply hits again.
            assert!(apply_from(&cell, &hs, shooter, shooter, |_, info| info.damage == 22.5));
        }

        #[test]
        fn a_new_victim_flushes_the_old_one_first() {
            let cell = Cell::new(MultiDamage::EMPTY);
            let (a, b) = (fake(2), fake(3));
            let meta = DamageInfo::new(fake(1), fake(1), 0.0, 0);
            add_to(&cell, a, 10.0, bits::BULLET, &meta, || {});
            let mut flushed = None;
            add_to(&cell, b, 4.0, bits::CLUB, &meta, || flushed = Some(cell.get()));
            let f = flushed.expect("victim changed");
            assert_eq!((f.entity, f.amount), (a, 10.0));
            assert_eq!(f.bits, bits::BULLET | bits::CLUB, "the old victim gets the new bits too");
            let m = cell.get();
            assert_eq!((m.entity, m.amount, m.bits), (b, 4.0, bits::BULLET | bits::CLUB));
        }

        #[test]
        fn no_victim_nothing_applied() {
            let cell = Cell::new(MultiDamage::EMPTY);
            let hs = Cell::new(true);
            add_to(&cell, std::ptr::null_mut(), 5.0, bits::BULLET, &DamageInfo::new(fake(1), fake(1), 0.0, 0), || panic!());
            assert_eq!(cell.get().bits, 0, "a null victim adds nothing, not even its bits");
            assert!(!apply_from(&cell, &hs, fake(1), fake(1), |_, _| panic!("no victim")));
            assert!(hs.get(), "glast_headshot is only reset with a victim");
        }

        #[test]
        fn the_first_add_after_a_clear_flushes_nothing_real() {
            // The "old victim" of the first Add is the cleared null entity: its Apply is a no-op.
            let cell = Cell::new(MultiDamage::EMPTY);
            let hs = Cell::new(false);
            let meta = DamageInfo::new(fake(1), fake(1), 0.0, 0);
            add_to(&cell, fake(2), 1.0, 0, &meta, || assert!(!apply_from(&cell, &hs, fake(1), fake(1), |_, _| panic!())));
            assert_eq!(cell.get().amount, 1.0);
        }
    }
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
