//! SDK weapons.cpp classes as a map can place them in TS: `weaponbox` (`CWeaponBox`),
//! `weapon_tsgun` (`CTSGun` lying in the world, `CBasePlayerItem`'s fall / touch / respawn) and
//! `cycler_weapon` (`CWeaponCycler`), plus their `SetObjectCollisionBox` overrides.
//!
//! TS registers only `weapon_tsgun` in `CBasePlayerItem::ItemInfoArray`, and its `ItemInfo` has
//! no ammo names, so `MaxAmmoCarry` knows no ammo at all: a map's weaponbox can never hold
//! anything and just disappears with a pickup sound.

use super::base::*;
use super::explode::emit_sound_slow;
use super::items::SF_NORESPAWN;
use crate::entities::remove;
use crate::world::{self, Cx, Entity, UseType};
use goldsrc::edict::EdictPtr;
use goldsrc::ext::entities::{ATTN_NORM, Vec3, is_player};
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::ffi::CStr;

/// `MAX_AMMO_SLOTS`: the most ammo keys a weaponbox takes.
const MAX_AMMO_SLOTS: i32 = 32;

/// `CBasePlayerItem::SetObjectCollisionBox` (0xf1698) and `CWeaponBox::SetObjectCollisionBox`
/// (0xf320c): items take a fixed box around their origin for touching, whatever their size.
/// `None` for every other class (the engine's default box).
pub fn object_collision_box(classname: &CStr, origin: Vec3) -> Option<(Vec3, Vec3)> {
    let half = match classname.to_bytes() {
        b"weaponbox" => 16.0,
        b"weapon_tsgun" | b"cycler_weapon" => 24.0,
        _ => return None,
    };
    Some(([origin[0] - half, origin[1] - half, origin[2]], [origin[0] + half, origin[1] + half, origin[2] + 16.0]))
}

/// `MaxAmmoCarry` (0xf0c0c): the carry limit of a named ammo type, from the ammo names in
/// `ItemInfoArray`. TS's only entry (`weapon_tsgun`, `CTSGun::GetItemInfo` 0x85dd0) names
/// none, so every name is unknown.
fn max_ammo_carry(name: &CStr) -> i32 {
    engine().alert(ALERT_TYPE::at_console, &format!("MaxAmmoCarry() doesn't recognize '{}'!\n", name.to_string_lossy()));
    -1
}

// ---- weaponbox ------------------------------------------------------------------------------------------------------

/// `weaponbox` (`CWeaponBox`). Map keys that aren't entity fields are ammo to pack (name =
/// count); with no known ammo in TS, nothing is ever packed.
#[derive(Default)]
pub struct WeaponBox {
    /// `m_cAmmoTypes`: keys taken so far (counted even when packing fails).
    ammo_types: i32,
}

impl WeaponBox {
    /// `CWeaponBox::PackAmmo` (0xf2e68). Returns whether anything was packed.
    fn pack_ammo(&mut self, name: string_t, count: i32) -> bool {
        if name == 0 {
            engine().alert(ALERT_TYPE::at_console, "NULL String in PackAmmo!\n");
            return false;
        }
        if max_ammo_carry(s(name)) == -1 || count < 1 {
            return false;
        }
        // `CWeaponBox::GiveAmmo` would store it here; unreachable in TS (see the module docs).
        true
    }
}

impl Entity for WeaponBox {
    /// `CWeaponBox::KeyValue` (0xf2890).
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        if self.ammo_types >= MAX_AMMO_SLOTS {
            engine().alert(ALERT_TYPE::at_console, &format!("WeaponBox too full! only {MAX_AMMO_SLOTS} ammotypes allowed\n"));
            return false;
        }
        let count = kv_i(value);
        self.pack_ammo(alloc(key), count);
        self.ammo_types += 1;
        true
    }

    /// `CWeaponBox::Spawn` (0xf2914) / `Precache` (0xf2868).
    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        eng.precache_model(c"models/w_weaponbox.mdl");
        let v = cx.pev();
        v.movetype = MOVETYPE_TOSS as i32;
        v.solid = SOLID_TRIGGER as i32;
        eng.set_size(cx.edict, [0.0; 3], [0.0; 3]);
        eng.set_model(cx.edict, c"models/w_weaponbox.mdl");
    }

    /// `CWeaponBox::Touch` (0xf2a80): a living player touching the box once it has landed takes
    /// its (always empty) contents.
    fn touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        if cx.pev().flags & FL_ONGROUND as i32 == 0 || !is_player(other) || !is_alive(other) {
            return;
        }
        emit_sound_slow(other, CHAN_ITEM, c"items/gunpickup2.wav", 1.0, ATTN_NORM, 0, PITCH_NORM as i32, 0.5);
        remove(cx.edict);
    }
}

/// `CBasePlayer::IsAlive` (0x97498).
fn is_alive(p: *mut edict_t) -> bool {
    let v = p.v();
    v.deadflag == DEAD_NO as i32 && v.health > 0.0
}

// ---- weapon_tsgun ---------------------------------------------------------------------------------------------------

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum ItemThink {
    #[default]
    None,
    /// `CBasePlayerItem::FallThink`.
    Fall,
    /// `CBasePlayerItem::AttemptToMaterialize`.
    AttemptToMaterialize,
}

/// `weapon_tsgun` placed by a map: a TS gun lying in the world. A living player without a gun
/// picks it up (the gun TS's `PlayerSpawn` would otherwise give, empty, with kung fu); without
/// `SF_NORESPAWN` a copy reappears at the same spot 20 s later (`mp_weaponstay 0`).
///
/// `game_player_equip` doesn't create one: the port's gun has no entity, and
/// `maprules::give_named_item` gives it directly.
#[derive(Default)]
pub struct WorldTsGun {
    think: ItemThink,
    touchable: bool,
    delay: Delay,
}

impl WorldTsGun {
    /// `CBasePlayerItem::Materialize` (0xf18f0).
    fn materialize(&mut self, cx: &mut Cx) {
        if cx.pev().effects & EF_NODRAW as i32 != 0 {
            emit_sound_slow(cx.edict, CHAN_WEAPON, c"items/suitchargeok1.wav", 1.0, ATTN_NORM, 0, 150, 0.5);
            let v = cx.pev();
            v.effects &= !(EF_NODRAW as i32);
            v.effects |= EF_MUZZLEFLASH as i32;
        }
        let v = cx.pev();
        v.solid = SOLID_TRIGGER as i32;
        let org = v.origin;
        engine().set_origin(cx.edict, org);
        self.touchable = true;
        self.think = ItemThink::None;
    }

    /// `CBasePlayerItem::Respawn` (0xf1a44): a hidden copy here that materialises after
    /// `FlWeaponRespawnTime`.
    fn respawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        let v = cx.pev();
        let (class, origin, angles, owner) = (v.classname, v.origin, v.angles, v.owner);
        let new = create_entity(s(class), origin, angles, owner);
        if new.is_null() {
            eng.alert(ALERT_TYPE::at_console, &format!("Respawn failed to create {}!\n", s(class).to_string_lossy()));
            return;
        }
        new.v().effects |= EF_NODRAW as i32;
        world::with::<WorldTsGun, _>(new, |g| {
            g.touchable = false;
            g.think = ItemThink::AttemptToMaterialize;
        });
        // DROP_TO_FLOOR(ENT(pev)): the SDK drops the item being picked up, not the copy.
        eng.drop_to_floor(cx.edict);
        new.v().nextthink = weapon_respawn_time(eng.time(), eng.cvar_float(c"mp_weaponstay"));
    }
}

/// `CHalfLifeMultiplay::FlWeaponRespawnTime` (0xcfc18) for `weapon_tsgun` (no
/// `ITEM_FLAG_LIMITINWORLD`): 20 s, or at once with `mp_weaponstay` on.
fn weapon_respawn_time(now: f32, weaponstay: f32) -> f32 {
    if weaponstay <= 0.0 { (20.0f64 + now as f64) as f32 } else { now }
}

impl Entity for WorldTsGun {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        self.delay.key_value(key, value)
    }

    /// `CTSGun::Spawn` (0x8053c) and `CBasePlayerItem::FallInit` (0xf175c). `CTSGun::Precache`
    /// only precaches the gun events, which the port's weapons precache for every map.
    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        let me = cx.edict;
        cx.pev().classname = eng.alloc_string(c"weapon_tsgun");
        eng.precache_model(c"models/w_glock18.mdl");
        eng.set_model(me, c"models/w_glock18.mdl");
        let v = cx.pev();
        v.movetype = MOVETYPE_TOSS as i32;
        v.solid = SOLID_BBOX as i32;
        let org = v.origin;
        eng.set_origin(me, org);
        eng.set_size(me, [0.0; 3], [0.0; 3]);
        self.touchable = true;
        self.think = ItemThink::Fall;
        cx.pev().nextthink = eng.time() + 0.1;
    }

    fn think(&mut self, cx: &mut Cx) {
        match self.think {
            // CBasePlayerItem::FallThink (0xf1818)
            ItemThink::Fall => {
                let eng = engine();
                cx.pev().nextthink = eng.time() + 0.1;
                let v = cx.pev();
                if v.flags & FL_ONGROUND as i32 == 0 {
                    return;
                }
                let owner = v.owner;
                if !owner.is_null() && eng.index_of(owner) != 0 {
                    let pitch = 95 + eng.random_long(0, 29);
                    emit_sound_slow(cx.edict, CHAN_ITEM, c"items/weapondrop1.wav", 1.0, ATTN_NORM, 0, pitch, 0.5);
                }
                let v = cx.pev();
                v.angles[0] = 0.0;
                v.angles[2] = 0.0;
                self.materialize(cx);
            }
            // CBasePlayerItem::AttemptToMaterialize (0xf1998): `FlWeaponTryRespawn` is 0 for an
            // item without ITEM_FLAG_LIMITINWORLD.
            ItemThink::AttemptToMaterialize => self.materialize(cx),
            ItemThink::None => {}
        }
    }

    /// `CBasePlayerItem::DefaultTouch` (0xf1b40).
    fn touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        if !self.touchable || !is_player(other) {
            return;
        }
        // `CHalfLifeMultiplay::CanHavePlayerItem` (0xcfd38) / `CGameRules::CanHavePlayerItem`
        // (0xc5f48): only a living player (`deadflag`) who doesn't hold a TS gun yet (with
        // `mp_weaponstay` on by item id, otherwise `HasPlayerItem` by classname: the same test).
        if other.v().deadflag != DEAD_NO as i32 {
            return;
        }
        let Some(has_gun) = crate::player::with_player(other, |p, _| crate::weapons::has_gun(p)) else { return };
        if has_gun {
            return;
        }
        // AddPlayerItem → AddToPlayer, then CheckRespawn (0xf19fc) inside it.
        crate::weapons::give_gun_to(other);
        if cx.pev().spawnflags & SF_NORESPAWN == 0 {
            self.respawn(cx);
        }
        // AttachToPlayer (0xf1d80): TS keeps this entity as the player's gun until the gun is
        // destroyed; the port's gun is no entity, so it goes now.
        self.touchable = false;
        self.think = ItemThink::None;
        let v = cx.pev();
        v.movetype = MOVETYPE_FOLLOW as i32;
        v.solid = SOLID_NOT as i32;
        v.aiment = other;
        v.effects = EF_NODRAW as i32;
        v.modelindex = 0;
        v.model = 0;
        v.owner = other;
        remove(cx.edict);
        emit_sound_slow(other, CHAN_ITEM, c"items/gunpickup2.wav", 1.0, ATTN_NORM, 0, PITCH_NORM as i32, 0.5);
        self.delay.use_targets(cx.edict, other, UseType::Toggle, 0.0);
    }
}

// ---- cycler_weapon --------------------------------------------------------------------------------------------------

/// `cycler_weapon` (`CWeaponCycler`): a model lying in the world as a player weapon.
///
/// Deviation: TS's `DefaultTouch` adds it to the player's HL item slots and switches to it
/// (holstering the TS gun), and it then plays its sequences on +attack / +attack2. The port has
/// no general player item list (only the TS gun, `weapons`), so it can't be picked up.
#[derive(Default)]
pub struct WeaponCycler {
    /// `m_iszModel` / `m_iModel`: the model, which `Deploy` would show as the view model.
    model: string_t,
    model_index: i32,
}

impl Entity for WeaponCycler {
    /// `CWeaponCycler::Spawn` (0xc7c78).
    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        let v = cx.pev();
        v.solid = SOLID_SLIDEBOX as i32;
        v.movetype = MOVETYPE_NONE as i32;
        precache_model_s(v.model);
        set_own_model(cx);
        let v = cx.pev();
        self.model = v.model;
        self.model_index = v.modelindex;
        let org = v.origin;
        eng.set_origin(cx.edict, org);
        eng.set_size(cx.edict, [-16.0, -16.0, 0.0], [16.0, 16.0, 16.0]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_boxes() {
        let o = [10.0, 20.0, 30.0];
        assert_eq!(object_collision_box(c"weaponbox", o), Some(([-6.0, 4.0, 30.0], [26.0, 36.0, 46.0])));
        assert_eq!(object_collision_box(c"weapon_tsgun", o), Some(([-14.0, -4.0, 30.0], [34.0, 44.0, 46.0])));
        assert_eq!(object_collision_box(c"cycler_weapon", o), Some(([-14.0, -4.0, 30.0], [34.0, 44.0, 46.0])));
        assert_eq!(object_collision_box(c"item_healthkit", o), None);
    }

    #[test]
    fn respawn_time() {
        assert_eq!(weapon_respawn_time(100.0, 0.0), 120.0);
        assert_eq!(weapon_respawn_time(100.0, 1.0), 100.0);
    }
}
