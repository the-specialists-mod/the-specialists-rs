//! `item_healthkit`, `item_battery`, `item_antidote` (SDK items.cpp / healthkit.cpp with TS's
//! player rules) and the player health/armour helpers they need.
//!
//! TS: a healthkit gives a fixed 50 health through `CBasePlayer::TakeHealth`, which only refuses
//! when the player is above `max_health` and doesn't cap (the player's PreThink decays health
//! above max by 1/s: rules package). A battery calls `TakeArmor(50)`: the first kevlar costs 10
//! free inventory slots (no kevlar if there's no room, but the battery is still used up), armour
//! is capped at 100 and the move speed is recomputed. An antidote adds one to the player's
//! antidote count, which cures nerve gas or poison (`player::water`). Items respawn after 30 s
//! unless spawnflag `SF_NORESPAWN` (1 << 30) is set.

use super::base::*;
use crate::entities::remove;
use crate::messages::{self, Msg};
use crate::world::{Cx, Entity, UseType};
use goldsrc::edict::EdictPtr;
use goldsrc::ext::entities::{ATTN_NORM, is_player};
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::ffi::CStr;

pub const SF_NORESPAWN: i32 = 1 << 30;
const ITEM_RESPAWN_TIME: f32 = 30.0;

/// TS `CBasePlayer::TakeHealth` (0xd8760, `player::damage::take_health_edict`): fails only with
/// takedamage off or health above max; no cap; overhealing starts the 1/s decay.
pub fn player_take_health(p: *mut edict_t, amount: f32) -> bool {
    is_player(p) && crate::player::damage::take_health_edict(p, amount)
}

/// `pOther->TakeHealth(amount)` for any entity (SDK `CBaseEntity::TakeHealth` caps at max).
pub fn take_health(e: *mut edict_t, amount: f32) -> bool {
    if is_player(e) {
        return player_take_health(e, amount);
    }
    let v = e.v();
    if v.takedamage == 0.0 || v.health >= v.max_health {
        return false;
    }
    v.health = (v.health + amount).min(v.max_health);
    true
}

/// TS `CBasePlayer::TakeArmor(amount)` (0x93da4, `weapons::take_armor`) for an edict. An HEV
/// charger is used from the player's own `PlayerUse`, so the player is taken out for a dispatch
/// further up the stack; the armour then lands as soon as that returns (same frame) and this
/// reports success, which both callers ignore anyway, as TS does.
pub fn player_take_armor(p: *mut edict_t, amount: f32) -> bool {
    if !is_player(p) {
        return false;
    }
    let n = amount as i32;
    if crate::world::is_taken_out(p) {
        crate::world::defer(p, move || {
            crate::player::with_player(p, |pl, cx| crate::weapons::take_armor(pl, cx, n));
        });
        return true;
    }
    crate::player::with_player(p, |pl, cx| crate::weapons::take_armor(pl, cx, n)).unwrap_or(false)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    HealthKit,
    Battery,
    Antidote,
    Suit,
    Security,
}

impl Kind {
    /// Whether the item's `MyTouch` tests `deadflag`: only `CHealthKit::MyTouch` (0xc8968) and
    /// `CItemBattery::MyTouch` (0xcb74c) do. `CItem::ItemTouch` (0xcaf8c) itself doesn't, so a
    /// dead player takes an antidote, a suit or a security card.
    fn refuses_dead(self) -> bool {
        matches!(self, Kind::HealthKit | Kind::Battery)
    }
}

/// `CItem` subclasses: `KIND` 0 = healthkit, 1 = battery, 2 = antidote, 3 = suit, 4 = security.
#[derive(Default)]
pub struct Item<const KIND: u8> {
    respawning: bool,
    touchable: bool,
}

impl<const KIND: u8> Item<KIND> {
    fn kind() -> Kind {
        match KIND {
            0 => Kind::HealthKit,
            1 => Kind::Battery,
            2 => Kind::Antidote,
            3 => Kind::Suit,
            _ => Kind::Security,
        }
    }

    fn model() -> &'static CStr {
        match Self::kind() {
            Kind::HealthKit => c"models/w_medkit.mdl",
            Kind::Battery => c"models/w_battery.mdl",
            Kind::Antidote => c"models/w_antidote.mdl",
            Kind::Suit => c"models/w_suit.mdl",
            Kind::Security => c"models/w_security.mdl",
        }
    }

    fn my_touch(&mut self, cx: &mut Cx, p: *mut edict_t) -> bool {
        let cls = s(cx.pev().classname);
        match Self::kind() {
            Kind::HealthKit => {
                // `CHealthKit::MyTouch` (0xc8968): refused while the player is dead (`deadflag`).
                if (Self::kind().refuses_dead() && p.v().deadflag != DEAD_NO as i32) || !player_take_health(p, 50.0) {
                    return false;
                }
                messages::one(Msg::ItemPickup, p).string(cls).end();
                emit(p, CHAN_ITEM, c"items/smallmedkit1.wav", 1.0, ATTN_NORM, 0, PITCH_NORM as i32);
            }
            Kind::Battery => {
                // `CItemBattery::MyTouch` (0xcb74c): a living player with the HEV suit bit
                // (weapons bit 31).
                let v = p.v();
                if (Self::kind().refuses_dead() && v.deadflag != DEAD_NO as i32) || v.armorvalue >= 100.0 || v.weapons >= 0 {
                    return false;
                }
                // TS ignores TakeArmor's result: the battery is used up either way.
                player_take_armor(p, 50.0);
                emit(p, CHAN_ITEM, c"items/gunpickup2.wav", 1.0, ATTN_NORM, 0, PITCH_NORM as i32);
                messages::one(Msg::ItemPickup, p).string(cls).end();
                // The suit's charge report, "!HEV_<n>P" (n = armour / 5 - 1).
                let mut pct = ((p.v().armorvalue * 100.0 * 0.01 + 0.5) as i32) / 5;
                if pct > 0 {
                    pct -= 1;
                }
                let name = std::ffi::CString::new(format!("!HEV_{pct}P")).unwrap_or_default();
                super::sentences::set_suit_update(p, &name, false, 30);
            }
            Kind::Antidote => {
                // `CItemAntidote::MyTouch` (0xcb968): no pickup message or sound, always taken.
                super::sentences::set_suit_update(p, c"!HEV_DET4", false, 60);
                // `m_rgItems[ITEM_ANTIDOTE]++` (+0x834). The player can be taken out further up
                // the stack (a trigger touched from its own think), so wait until it is back.
                crate::world::defer(p, move || {
                    crate::player::with_player(p, |pl, _| pl.tbd.antidotes += 1);
                });
            }
            Kind::Suit => {
                // `CItemSuit::MyTouch` (0xcb60c): refused while the player has the suit bit, which
                // every TS player gets on spawn.
                let v = p.v();
                if v.weapons < 0 {
                    return false;
                }
                let sentence = if cx.pev().spawnflags & SF_SUIT_SHORTLOGON != 0 { c"!HEV_A0" } else { c"!HEV_AAx" };
                emit_sound_suit(p, sentence);
                p.v().weapons |= WEAPON_SUIT;
            }
            Kind::Security => {
                // `CItemSecurity::MyTouch` (0xcba58): `m_rgItems[ITEM_SECURITY]++` (+0x838) and
                // always taken. Nothing in TS reads the count, so the port doesn't keep it.
            }
        }
        true
    }

    /// `CItem::Respawn`.
    fn respawn(&mut self, cx: &mut Cx) {
        self.touchable = false;
        cx.pev().effects |= EF_NODRAW as i32;
        let org = cx.pev().origin;
        engine().set_origin(cx.edict, org);
        self.respawning = true;
        cx.pev().nextthink = engine().time() + ITEM_RESPAWN_TIME;
    }

    /// `CItem::Materialize`.
    fn materialize(&mut self, cx: &mut Cx) {
        if cx.pev().effects & EF_NODRAW as i32 != 0 {
            emit(cx.edict, CHAN_WEAPON, c"items/suitchargeok1.wav", 1.0, ATTN_NORM, 0, 150);
            let v = cx.pev();
            v.effects &= !(EF_NODRAW as i32);
            v.effects |= EF_MUZZLEFLASH as i32;
        }
        self.touchable = true;
    }
}

impl<const KIND: u8> Entity for Item<KIND> {
    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        eng.precache_model(Self::model());
        match Self::kind() {
            Kind::HealthKit => eng.precache_sound(c"items/smallmedkit1.wav"),
            Kind::Battery => {
                eng.precache_sound(c"weapons/items.wav");
                eng.precache_sound(c"items/gunpickup2.wav")
            }
            // `CItemAntidote::Precache` (0xcb940), `CItemSuit::Precache` (0xcb5e4) and
            // `CItemSecurity::Precache` (0xcba30) only precache the model.
            Kind::Antidote | Kind::Suit | Kind::Security => 0,
        };
        eng.precache_sound(c"items/suitchargeok1.wav");
        eng.set_model(cx.edict, Self::model());
        // CItem::Spawn
        let v = cx.pev();
        v.movetype = MOVETYPE_TOSS as i32;
        v.solid = SOLID_TRIGGER as i32;
        let org = v.origin;
        eng.set_origin(cx.edict, org);
        eng.set_size(cx.edict, [-16.0, -16.0, 0.0], [16.0, 16.0, 16.0]);
        self.touchable = true;
        if eng.drop_to_floor(cx.edict) == 0 {
            let o = cx.pev().origin;
            let cls = s(cx.pev().classname).to_string_lossy();
            eng.alert(ALERT_TYPE::at_error, &format!("Item {cls} fell out of level at {},{},{}\n", o[0], o[1], o[2]));
            remove(cx.edict);
        }
    }

    fn think(&mut self, cx: &mut Cx) {
        if self.respawning {
            self.respawning = false;
            self.materialize(cx);
        }
    }

    /// `CItem::ItemTouch` (0xcaf8c): any player (`IsPlayer`; `CanHaveItem` is always 1), dead or
    /// alive: only the healthkit's and the battery's `MyTouch` refuse the dead. The antidote, suit
    /// and security card are taken by a dead player too.
    fn touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        if !self.touchable || !is_player(other) {
            return;
        }
        if self.my_touch(cx, other) {
            let me = cx.edict;
            sub_use_targets(me, cx.pev().target, 0, 0.0, other, UseType::Toggle, 0.0);
            self.touchable = false;
            // `ItemShouldRespawn`: `CHalfLifeRules` (0xde8a0) never respawns.
            if cx.pev().spawnflags & SF_NORESPAWN == 0 && !crate::rules::state::single_player() {
                self.respawn(cx);
            } else {
                remove(me);
            }
        }
    }
}

pub type HealthKit = Item<0>;
pub type Battery = Item<1>;
/// `item_antidote` (`CItemAntidote`, 0xcb1c0; Spawn 0xcb8f8).
pub type Antidote = Item<2>;
/// `item_suit` (`CItemSuit`, 0xcb178; Spawn 0xcb59c).
pub type Suit = Item<3>;
/// `item_security` (`CItemSecurity`, 0xcb1e4; Spawn 0xcb9e8).
pub type Security = Item<4>;

/// `item_suit` spawnflag: the short logon sentence.
pub const SF_SUIT_SHORTLOGON: i32 = 1;
/// The HEV suit bit in `pev->weapons` (`WEAPON_SUIT`, 31).
pub const WEAPON_SUIT: i32 = 1 << 31;

/// `EMIT_SOUND_SUIT` (0xe0bb0): a suit sentence on the player's `CHAN_STATIC` at `suitvolume`
/// (a client cvar: 0 on a dedicated server, so silent there), pitch 100 or, half the time,
/// 98..104.
pub fn emit_sound_suit(p: *mut edict_t, sample: &CStr) {
    let eng = engine();
    let vol = eng.cvar_float(c"suitvolume");
    let pitch = if eng.random_long(0, 1) != 0 { eng.random_long(0, 6) + 98 } else { PITCH_NORM as i32 };
    if vol > 0.05 {
        super::explode::emit_sound_slow(p, CHAN_STATIC, sample, vol, ATTN_NORM, 0, pitch, 0.5);
    }
}

/// `world_items` (`CWorldItem`): spawns the item its `type` names (42 antidote, 43 security, 44
/// battery, 45 suit), hands it its `target`, `targetname` and spawnflags, and removes itself.
#[derive(Default)]
pub struct WorldItem {
    kind: i32,
}

impl WorldItem {
    fn class_for(kind: i32) -> Option<&'static CStr> {
        Some(match kind {
            42 => c"item_antidote",
            43 => c"item_security",
            44 => c"item_battery",
            45 => c"item_suit",
            _ => return None,
        })
    }
}

impl Entity for WorldItem {
    /// `CWorldItem::KeyValue` (0xcad0c).
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        if key != c"type" {
            return false;
        }
        self.kind = kv_i(value);
        true
    }

    /// `CWorldItem::Spawn` (0xcad6c).
    fn spawn(&mut self, cx: &mut Cx) {
        let v = cx.pev();
        let (origin, angles, target, targetname, flags) = (v.origin, v.angles, v.target, v.targetname, v.spawnflags);
        let item = Self::class_for(self.kind).map_or(std::ptr::null_mut(), |c| create_entity(c, origin, angles, std::ptr::null_mut()));
        if item.is_null() {
            engine().alert(ALERT_TYPE::at_console, &format!("unable to create world_item {}\n", self.kind));
        } else {
            let iv = item.v();
            iv.target = target;
            iv.targetname = targetname;
            iv.spawnflags = flags;
        }
        remove(cx.edict);
    }
}

// ---- func_healthcharger / func_recharge ---------------------------------------------------------------

/// `func_healthcharger` (`ARMOR = false`) and `func_recharge` (`ARMOR = true`), TS versions: a full
/// charger empties in one go (full health via `TakeHealth(max - health)`, or `TakeArmor(100)`),
/// then recharges after 60 s (health) / 30 s (armour). Continuous +use.
#[derive(Default)]
pub struct Charger<const ARMOR: bool> {
    full: bool,
    on: i32,
    next_charge: f32,
    sound_time: f32,
    recharge_at: Option<f32>,
}

impl<const ARMOR: bool> Charger<ARMOR> {
    fn sounds() -> (&'static CStr, &'static CStr, &'static CStr, f32) {
        if ARMOR {
            (c"items/suitchargeok1.wav", c"items/suitcharge1.wav", c"items/suitchargeno1.wav", 0.85)
        } else {
            (c"items/medshot4.wav", c"items/medcharge4.wav", c"items/medshotno1.wav", 1.0)
        }
    }

    fn off(&mut self, cx: &mut Cx) {
        let (_, charge, _, _) = Self::sounds();
        if self.on > 1 {
            engine().emit_sound(cx.edict, CHAN_STATIC as i32, charge, 0.0, 0.0, goldsrc::ext::entities::snd::STOP, PITCH_NORM as i32);
        }
        self.on = 0;
        // `FlHealthChargerRechargeTime` / `FlHEVChargerRechargeTime` are 0 in single player
        // (`CHalfLifeRules` 0xde934, `CGameRules`), and an empty charger then stays empty.
        if !self.full && !crate::rules::state::single_player() {
            let t = if ARMOR { 30.0 } else { 60.0 };
            self.recharge_at = Some(t);
            think_ltime(cx, t);
        } else {
            self.recharge_at = None;
        }
    }

    fn recharge(&mut self, cx: &mut Cx) {
        let (start, _, _, vol) = Self::sounds();
        emit(cx.edict, CHAN_ITEM, start, vol, ATTN_NORM, 0, PITCH_NORM as i32);
        self.full = true;
        cx.pev().frame = 0.0;
        self.recharge_at = None;
    }
}

impl<const ARMOR: bool> Entity for Charger<ARMOR> {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, _value: &CStr) -> bool {
        // style/height/value1-3/dmdelay are swallowed; the rest go to CBaseToggle (unused here).
        matches!(key.to_bytes(), b"style" | b"height" | b"value1" | b"value2" | b"value3" | b"dmdelay")
    }

    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        let (a, b, c, _) = Self::sounds();
        for snd in [a, b, c] {
            eng.precache_sound(snd);
        }
        let v = cx.pev();
        v.solid = SOLID_BSP as i32;
        v.movetype = MOVETYPE_PUSH as i32;
        let (org, mins, maxs) = (v.origin, v.mins, v.maxs);
        eng.set_origin(cx.edict, org);
        eng.set_size(cx.edict, mins, maxs);
        set_own_model(cx);
        self.full = true;
        cx.pev().frame = 0.0;
    }

    fn think(&mut self, cx: &mut Cx) {
        if self.recharge_at.is_some() {
            self.recharge(cx);
        } else {
            self.off(cx);
        }
    }

    fn use_(&mut self, cx: &mut Cx, activator: *mut edict_t, _c: *mut edict_t, _k: UseType, _v: f32) {
        if !is_player(activator) {
            return;
        }
        let now = engine().time();
        let (start, charge, deny, vol) = Self::sounds();
        if !self.full {
            cx.pev().frame = 1.0;
            self.off(cx);
        }
        // `CRecharge::Use` (0xc6fd0) also refuses players without the HEV suit bit.
        if !self.full || (ARMOR && activator.v().weapons >= 0) {
            if self.sound_time <= now {
                self.sound_time = now + 0.62;
                emit(cx.edict, CHAN_ITEM, deny, vol, ATTN_NORM, 0, PITCH_NORM as i32);
            }
            return;
        }
        self.recharge_at = None;
        think_ltime(cx, 0.25);
        if self.next_charge >= now {
            return;
        }
        if self.on == 0 {
            self.on = 1;
            emit(cx.edict, CHAN_ITEM, start, vol, ATTN_NORM, 0, PITCH_NORM as i32);
            self.sound_time = now + 0.56;
        }
        if self.on == 1 && self.sound_time <= now {
            self.on = 2;
            emit(cx.edict, CHAN_STATIC, charge, vol, ATTN_NORM, 0, PITCH_NORM as i32);
        }
        let pv = activator.v();
        if ARMOR {
            if pv.armorvalue < 100.0 {
                self.full = false;
                player_take_armor(activator, 100.0);
            }
        } else if pv.health != pv.max_health {
            let amount = pv.max_health - pv.health;
            if player_take_health(activator, amount) {
                self.full = false;
            }
        }
        self.next_charge = now + 0.1;
    }
}

pub type HealthCharger = Charger<false>;
pub type ArmorCharger = Charger<true>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_healthkit_and_battery_refuse_the_dead() {
        assert!(Kind::HealthKit.refuses_dead());
        assert!(Kind::Battery.refuses_dead());
        assert!(!Kind::Antidote.refuses_dead());
        assert!(!Kind::Suit.refuses_dead());
        assert!(!Kind::Security.refuses_dead());
    }
}
