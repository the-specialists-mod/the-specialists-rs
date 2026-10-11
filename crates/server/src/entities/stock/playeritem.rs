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

    /// `CWeaponBox::Save` (0xf27c8). `Touch` is the class's own (its `SetTouch(NULL)` leaves the
    /// pointer unset). Nothing is ever packed in TS (see the module docs), so the ammo counts,
    /// ammo names and items are empty.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut crate::save::Fields) {
        f.base_funcs(cx, crate::save::BaseFuncs::default());
        f.table(&crate::save::tables::CWeaponBox);
        f.zero();
        f.zero();
        f.zero();
        f.int(&mut self.ammo_types);
    }

    /// `CWeaponBox::Precache` (0xf2868).
    fn restored(&mut self, _cx: &mut Cx) {
        engine().precache_model(c"models/w_weaponbox.mdl");
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

// ---- weapon_tsgun / cycler_weapon: CBasePlayerItem ----------------------------------------------------------------

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum ItemThink {
    #[default]
    None,
    /// `CBasePlayerItem::FallThink`.
    Fall,
    /// `CBasePlayerItem::AttemptToMaterialize`.
    AttemptToMaterialize,
    /// `CBaseEntity::SUB_Remove`, from `Drop` / `Kill`.
    Remove,
}

/// The `CBasePlayerItem` state both item classes share: the think and touch functions and the
/// `CBaseDelay` keys `SUB_UseTargets` reads.
#[derive(Default)]
struct PlayerItem {
    think: ItemThink,
    /// `SetTouch(DefaultTouch)`.
    touchable: bool,
    delay: Delay,
    /// `m_pPlayer` (+0x80): set by `AddToPlayer` (0xf1c98), never cleared.
    player: *mut edict_t,
    /// `m_iId` (+0x88): 16 for the gun (`CTSGun::Spawn` 0x8053c), 0 for a cycler.
    id: i32,
    /// The `CBasePlayerWeapon` members TS saves, as the item itself has them. A held gun's
    /// timers and clip are the holder's (`weapons::Inventory::gun`): see [`WorldTsGun`].
    weapon: WeaponFields,
}

/// `CBasePlayerWeapon`'s saved members.
#[derive(Default, Clone, Copy)]
struct WeaponFields {
    /// `m_flNextPrimaryAttack` (+0x9c), `m_flNextSecondaryAttack` (+0xa0), `m_flTimeWeaponIdle`
    /// (+0xa8): relative to `UTIL_WeaponTimeBase` (`UseDecrement` is 1), hence plain floats.
    next_primary: f32,
    next_secondary: f32,
    idle: f32,
    /// `m_iPrimaryAmmoType` (+0xac) / `m_iSecondaryAmmoType` (+0xb0): -1 from the first
    /// `AddToPlayer` on (no `ItemInfo` names ammo).
    primary_ammo_type: i32,
    secondary_ammo_type: i32,
    /// `m_iClip` (+0xb4).
    clip: i32,
    /// `m_iDefaultAmmo` (+0xc4).
    default_ammo: i32,
}

impl WeaponFields {
    fn visit(&mut self, f: &mut crate::save::Fields) {
        f.table(&crate::save::tables::CBasePlayerWeapon);
        f.float(&mut self.next_primary);
        f.float(&mut self.next_secondary);
        f.float(&mut self.idle);
        f.int(&mut self.primary_ammo_type);
        f.int(&mut self.secondary_ammo_type);
        f.int(&mut self.clip);
        f.int(&mut self.default_ammo);
    }
}

/// Reaches the `PlayerItem` of an item entity of one class (for `Respawn`'s copy).
type ItemAccess = fn(*mut edict_t, &mut dyn FnMut(&mut PlayerItem));

impl PlayerItem {
    /// `CBasePlayerItem::Save` (0xf1558) and the chain above it: the member pointers are
    /// `FallThink` / `AttemptToMaterialize` / `SUB_Remove` and `DefaultTouch`. TS never animates
    /// an item, so its `CBaseAnimating` members stay zero. `m_pNext` is always NULL: each slot
    /// list holds one item at most (the gun in its own, a cycler in slot 1), and
    /// `RemoveAllItems` empties the lists before the next one is added.
    fn save_head(&mut self, cx: &mut Cx, f: &mut crate::save::Fields) {
        let funcs = crate::save::BaseFuncs { think: self.think != ItemThink::None, touch: self.touchable, ..Default::default() };
        let cleared = f.base_funcs(cx, funcs);
        if cleared.think {
            self.think = ItemThink::None;
        }
        if cleared.touch {
            self.touchable = false;
        }
        f.delay(&mut self.delay.delay, &mut self.delay.kill_target);
        f.animating(&mut crate::save::Animating::default());
        f.table(&crate::save::tables::CBasePlayerItem);
        f.ent(&mut self.player);
        f.zero();
        f.int(&mut self.id);
    }

    /// `CBasePlayerItem::AddToPlayer` (0xf1c98) and `CBasePlayerWeapon::AddToPlayer`'s (0xf1e60)
    /// ammo indices: `GetAmmoIndex(NULL)` is -1, looked up while the primary one is still 0.
    fn added_to_player(&mut self, player: *mut edict_t) {
        self.player = player;
        if self.weapon.primary_ammo_type == 0 {
            self.weapon.primary_ammo_type = -1;
            self.weapon.secondary_ammo_type = -1;
        }
    }

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
            ItemThink::Remove => sub_remove(cx.edict),
            ItemThink::None => {}
        }
    }

    /// `CBasePlayerItem::CheckRespawn` (0xf19fc): `WeaponShouldRespawn` is `GR_WEAPON_RESPAWN_YES`
    /// only under the multiplayer rules (0xcfd20) and without `SF_NORESPAWN`;
    /// `CHalfLifeRules::WeaponShouldRespawn` (0xde88c) never respawns.
    fn should_respawn(cx: &mut Cx) -> bool {
        cx.pev().spawnflags & SF_NORESPAWN == 0 && !crate::rules::state::single_player()
    }

    /// `CBasePlayerItem::Respawn` (0xf1a44): a hidden copy here (`Create`, so its `Spawn` runs)
    /// that materialises after `FlWeaponRespawnTime`.
    fn respawn(cx: &mut Cx, item: ItemAccess) {
        let eng = engine();
        let v = cx.pev();
        let (class, origin, angles, owner) = (v.classname, v.origin, v.angles, v.owner);
        let new = create_entity(s(class), origin, angles, owner);
        if new.is_null() {
            eng.alert(ALERT_TYPE::at_console, &format!("Respawn failed to create {}!\n", s(class).to_string_lossy()));
            return;
        }
        new.v().effects |= EF_NODRAW as i32;
        item(new, &mut |g| {
            g.touchable = false;
            g.think = ItemThink::AttemptToMaterialize;
        });
        // DROP_TO_FLOOR(ENT(pev)): the SDK drops the item being picked up, not the copy.
        eng.drop_to_floor(cx.edict);
        new.v().nextthink = weapon_respawn_time(eng.time(), eng.cvar_float(c"mp_weaponstay"));
    }

    /// `CBasePlayerItem::AttachToPlayer` (0xf1d80): the item follows the player, hidden, until the
    /// player loses it (`Drop`). The think function stays as it was: none after `Materialize`,
    /// but `FallThink` for an item taken before it landed, which `GiveNamedItem`'s always is
    /// (spawned and touched at once). That one keeps thinking every 0.1 s as long as it is held,
    /// never landing (`MOVETYPE_FOLLOW` sets no `FL_ONGROUND`).
    fn attach_to_player(&mut self, cx: &mut Cx, player: *mut edict_t) {
        self.touchable = false;
        let now = engine().time();
        let v = cx.pev();
        v.movetype = MOVETYPE_FOLLOW as i32;
        v.solid = SOLID_NOT as i32;
        v.aiment = player;
        v.effects = EF_NODRAW as i32;
        v.modelindex = 0;
        v.model = 0;
        v.owner = player;
        v.nextthink = now + 0.1;
    }

    /// `CBasePlayerItem::Drop` (0xf1cac): no touch, `SUB_Remove` 0.1 s later.
    fn drop(&mut self, e: *mut edict_t) {
        self.touchable = false;
        self.think = ItemThink::Remove;
        e.v().nextthink = engine().time() + 0.1;
    }
}

/// `CHalfLifeMultiplay::FlWeaponRespawnTime` (0xcfc18) for an item without
/// `ITEM_FLAG_LIMITINWORLD` (`weapon_tsgun`, and `cycler_weapon`'s zeroed `ItemInfo`): 20 s, or at
/// once with `mp_weaponstay` on.
fn weapon_respawn_time(now: f32, weaponstay: f32) -> f32 {
    if weaponstay <= 0.0 { (20.0f64 + now as f64) as f32 } else { now }
}

/// `CBasePlayerItem::Drop` (0xf1cac) on a player's item entity (`RemoveAllItems`). Nothing for
/// a null edict or one that is no player item.
pub fn drop_item(e: *mut edict_t) {
    if e.is_null() {
        return;
    }
    if world::with::<WorldTsGun, _>(e, |g| g.item.drop(e)).is_none() {
        world::with::<WeaponCycler, _>(e, |c| c.item.drop(e));
    }
}

/// Runs `f` on the player `holder` while its gun entity is `gun` (engine-free).
fn with_holder(holder: *mut edict_t, gun: *mut edict_t, f: impl FnOnce(&mut crate::player::Player)) {
    if holder.is_null() {
        return;
    }
    world::with::<crate::player::Player, _>(holder, |p| {
        if p.inv.sv.gun_item.get() == gun {
            f(p);
        }
    });
}

fn with_tsgun_item(e: *mut edict_t, f: &mut dyn FnMut(&mut PlayerItem)) {
    world::with::<WorldTsGun, _>(e, |g| f(&mut g.item));
}

fn with_cycler_item(e: *mut edict_t, f: &mut dyn FnMut(&mut PlayerItem)) {
    world::with::<WeaponCycler, _>(e, |c| f(&mut c.item));
}

/// `weapon_tsgun` (`CTSGun`): the TS gun as an entity, placed by a map or created by
/// `GiveNamedItem` (`PlayerSpawn`, `game_player_equip`: spawned at the player's feet with
/// `SF_NORESPAWN`, then touched). A living player without a gun picks it up (empty, kung fu);
/// a player who already holds one leaves it lying there for anyone else. Under the multiplayer
/// rules and without `SF_NORESPAWN` a copy reappears at the same spot 20 s later
/// (`mp_weaponstay 0`). The picked-up entity stays, hidden and following the player, until
/// `RemoveAllItems` drops the gun (`weapons::SvState::gun_item`); the gun's state is the
/// player's (`weapons::Inventory`).
#[derive(Default)]
pub struct WorldTsGun {
    item: PlayerItem,
}

impl Entity for WorldTsGun {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        self.item.delay.key_value(key, value)
    }

    /// `CTSGun::Spawn` (0x8053c) and `CBasePlayerItem::FallInit` (0xf175c). `CTSGun::Precache`
    /// only precaches the gun events, which `W_Precache` already did for the map, and `Spawn`
    /// sets `models/w_glock18.mdl` without precaching it: `TSPrecache`'s weapon loop did
    /// (`weapons::precache_weapon_assets`, the Glock's world model).
    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        let me = cx.edict;
        cx.pev().classname = eng.alloc_string(c"weapon_tsgun");
        eng.set_model(me, c"models/w_glock18.mdl");
        let v = cx.pev();
        v.movetype = MOVETYPE_TOSS as i32;
        v.solid = SOLID_BBOX as i32;
        let org = v.origin;
        eng.set_origin(me, org);
        eng.set_size(me, [0.0; 3], [0.0; 3]);
        self.item.id = shared::protocol::weapons::TSGUN_ID;
        self.item.weapon.clip = 1;
        self.item.weapon.default_ammo = 1;
        self.item.touchable = true;
        self.item.think = ItemThink::Fall;
        cx.pev().nextthink = eng.time() + 0.1;
    }

    /// `CBasePlayerWeapon::Save` (0xf15f8). The gun a player holds keeps its timers and clip in
    /// the holder's `Gun` (`weapons::Inventory`): they are read from there and restored into it
    /// while the holder's `gun_item` is this entity.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut crate::save::Fields) {
        self.item.save_head(cx, f);
        let (me, holder) = (cx.edict, self.item.player);
        let mut w = self.item.weapon;
        with_holder(holder, me, |p| {
            let g = &p.inv.gun;
            (w.next_primary, w.next_secondary, w.idle, w.clip) = (g.next_primary, g.next_secondary, g.idle, g.clip_mirror);
        });
        w.visit(f);
        if f.restoring() {
            self.item.weapon = w;
            with_holder(holder, me, |p| {
                let g = &mut p.inv.gun;
                (g.next_primary, g.next_secondary, g.idle, g.clip_mirror) = (w.next_primary, w.next_secondary, w.idle, w.clip);
            });
        }
    }

    /// `CTSGun::Precache` (0x80660): the gun events (`weapons::precache_events`).
    fn restored(&mut self, _cx: &mut Cx) {
        crate::weapons::precache_events();
    }

    fn think(&mut self, cx: &mut Cx) {
        self.item.think(cx);
    }

    /// `CBasePlayerItem::DefaultTouch` (0xf1b40). The player is reached even while its own call
    /// is on the stack and lent it (`with_player_now`: an equip it used, its spawn's
    /// `GiveNamedItem`), as TS's touch reaches it.
    fn touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        if !self.item.touchable || !is_player(other) {
            return;
        }
        // `CHalfLifeMultiplay::CanHavePlayerItem` (0xcfd38) / `CGameRules::CanHavePlayerItem`
        // (0xc5f48): only a living player (`deadflag`) who doesn't hold a TS gun yet (with
        // `mp_weaponstay` on by item id, otherwise `HasPlayerItem` by classname: the same test).
        if other.v().deadflag != DEAD_NO as i32 {
            return;
        }
        let Some(holds) = crate::player::with_player_now(other, |p, _| crate::weapons::holds_gun(p)) else { return };
        if holds {
            return;
        }
        // AddPlayerItem: AddToPlayer, CheckRespawn (0xf19fc), then FShouldSwitchWeapon /
        // SwitchWeapon.
        let me = cx.edict;
        crate::player::with_player_now(other, |p, pcx| crate::weapons::add_gun(p, pcx, me));
        self.item.added_to_player(other);
        if PlayerItem::should_respawn(cx) {
            PlayerItem::respawn(cx, with_tsgun_item);
        }
        crate::player::with_player_now(other, crate::weapons::switch_to_gun);
        self.item.attach_to_player(cx, other);
        emit_sound_slow(other, CHAN_ITEM, c"items/gunpickup2.wav", 1.0, ATTN_NORM, 0, PITCH_NORM as i32, 0.5);
        self.item.delay.use_targets(cx.edict, other, UseType::Toggle, 0.0);
    }
}

// ---- cycler_weapon --------------------------------------------------------------------------------------------------

/// `cycler_weapon` (`CWeaponCycler`): a model lying in the world as a player weapon. A living
/// player without one picks it up into HL item slot 1 (`DefaultTouch`, `weapons::add_cycler`);
/// `FShouldSwitchWeapon` then deploys it (`weapons::deploy_cycler`): under the multiplayer rules
/// only when the player has no active item, in single player always, holstering a held TS gun.
/// `use cycler_weapon` / `use weapon_tsgun` switch between them later (`weapons::select_item`).
/// TS's `ItemPostFrame` (0xa03d4) returns at once for any item but the gun, so its
/// `PrimaryAttack` / `SecondaryAttack` never run.
#[derive(Default)]
pub struct WeaponCycler {
    item: PlayerItem,
    /// `m_iszModel`: the model, which `Deploy` shows as the view model.
    model: string_t,
    /// `m_iModel`.
    model_index: i32,
}

impl Entity for WeaponCycler {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        self.item.delay.key_value(key, value)
    }

    /// `CWeaponCycler::Spawn` (0xc7c78). Like TS it precaches and sets `STRING(pev->model)` even
    /// when that is empty, which a copy made by `Respawn` always is (`Create` sets no model): the
    /// engine decides then (GoldSrc stops with "Bad string", FWGS warns and uses the world).
    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        let v = cx.pev();
        v.solid = SOLID_SLIDEBOX as i32;
        v.movetype = MOVETYPE_NONE as i32;
        let model = s(v.model);
        eng.precache_model(model);
        eng.set_model(cx.edict, model);
        let v = cx.pev();
        self.model = v.model;
        self.model_index = v.modelindex;
        let org = v.origin;
        eng.set_origin(cx.edict, org);
        eng.set_size(cx.edict, [-16.0, -16.0, 0.0], [16.0, 16.0, 16.0]);
        self.item.touchable = true;
    }

    /// `CBasePlayerWeapon::Save` (0xf15f8). `m_iszModel` / `m_iModel` aren't saved: a restored
    /// cycler has neither, as in TS. Its timers stay 0 (`ItemPostFrame` never fires it) and so
    /// does its clip (`CWeaponCycler::Deploy` 0xc7d6c only zeroes it again).
    fn save_fields(&mut self, cx: &mut Cx, f: &mut crate::save::Fields) {
        self.item.save_head(cx, f);
        self.item.weapon.visit(f);
    }

    fn think(&mut self, cx: &mut Cx) {
        self.item.think(cx);
    }

    /// `CBasePlayerItem::DefaultTouch` (0xf1b40) with `AddPlayerItem` (0xdc0c4).
    fn touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        if !self.item.touchable || !is_player(other) {
            return;
        }
        let Some(can_have) = crate::player::with_player_now(other, |p, pcx| crate::weapons::can_have_cycler(p, pcx)) else { return };
        if !can_have {
            return;
        }
        let (me, model, model_index) = (cx.edict, self.model, self.model_index);
        // AddToPlayer (bit 0, slot list 1), CheckRespawn, then FShouldSwitchWeapon / Deploy.
        crate::player::with_player_now(other, |p, pcx| crate::weapons::add_cycler(p, pcx, me, model, model_index));
        self.item.added_to_player(other);
        if PlayerItem::should_respawn(cx) {
            PlayerItem::respawn(cx, with_cycler_item);
        }
        crate::player::with_player_now(other, crate::weapons::deploy_cycler);
        self.item.attach_to_player(cx, other);
        emit_sound_slow(other, CHAN_ITEM, c"items/gunpickup2.wav", 1.0, ATTN_NORM, 0, PITCH_NORM as i32, 0.5);
        self.item.delay.use_targets(cx.edict, other, UseType::Toggle, 0.0);
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
