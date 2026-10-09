//! `item_healthkit`, `item_battery` (SDK items.cpp / healthkit.cpp with TS's player rules) and
//! the player health/armour helpers they need.
//!
//! TS: a healthkit gives a fixed 50 health through `CBasePlayer::TakeHealth`, which only refuses
//! when the player is above `max_health` and doesn't cap (the player's PreThink decays health
//! above max by 1/s: rules package). A battery calls `TakeArmor(50)`: the first kevlar costs 10
//! free inventory slots (no kevlar if there's no room, but the battery is still used up), armour
//! is capped at 100 and the move speed is recomputed. Items respawn after 30 s unless
//! spawnflag `SF_NORESPAWN` (1 << 30) is set.

use super::base::*;
use crate::entities::remove;
use crate::messages::{self, Msg};
use crate::world::{Cx, Entity, UseType};
use goldsrc::ext::entities::{ATTN_NORM, is_player};
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::ffi::CStr;

pub const SF_NORESPAWN: i32 = 1 << 30;
const ITEM_RESPAWN_TIME: f32 = 30.0;

/// TS `CBasePlayer::TakeHealth`: fails only with takedamage off or health above max; no cap.
pub fn player_take_health(p: *mut edict_t, amount: f32) -> bool {
    if !is_player(p) {
        return false;
    }
    let v = unsafe { &mut (*p).v };
    if v.takedamage == 0.0 || v.health > v.max_health {
        return false;
    }
    v.health += amount;
    true
}

/// `pOther->TakeHealth(amount)` for any entity (SDK `CBaseEntity::TakeHealth` caps at max).
pub fn take_health(e: *mut edict_t, amount: f32) -> bool {
    if is_player(e) {
        return player_take_health(e, amount);
    }
    let v = unsafe { &mut (*e).v };
    if v.takedamage == 0.0 || v.health >= v.max_health {
        return false;
    }
    v.health = (v.health + amount).min(v.max_health);
    true
}

/// Stand-in for TS `CBasePlayer::TakeArmor(amount)` (0x93da4). Kevlar ownership is kept in the
/// networked stunt bit `HAS_KEVLAR`; slots in `Player::slots_used`.
pub fn player_take_armor(p: *mut edict_t, amount: f32) -> bool {
    if !is_player(p) {
        return false;
    }
    let iuser4 = unsafe { (*p).v.iuser4 };
    if iuser4 & shared::ts::stunt::HAS_KEVLAR == 0 {
        let ok = crate::player::with_player(p, |pl, _| {
            if shared::ts::SLOTS_TOTAL - pl.slots_used < 10 {
                return false;
            }
            pl.slots_used += 10;
            true
        })
        .unwrap_or(true);
        if !ok {
            return false;
        }
        unsafe { (*p).v.iuser4 |= shared::ts::stunt::HAS_KEVLAR };
        emit(p, CHAN_ITEM, c"weapons/items.wav", 1.0, ATTN_NORM, 0, PITCH_NORM as i32);
    }
    let v = unsafe { &mut (*p).v };
    v.armorvalue = (v.armorvalue + amount).min(100.0);
    if let Some(used) = crate::player::with_player(p, |pl, _| pl.slots_used) {
        engine().set_client_maxspeed(p, shared::ts::speed_for_free_slots(shared::ts::SLOTS_TOTAL - used));
    }
    true
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    HealthKit,
    Battery,
}

/// `CItem` subclasses: `KIND` 0 = healthkit, 1 = battery.
#[derive(Default)]
pub struct Item<const KIND: u8> {
    respawning: bool,
    touchable: bool,
}

impl<const KIND: u8> Item<KIND> {
    fn kind() -> Kind {
        if KIND == 0 { Kind::HealthKit } else { Kind::Battery }
    }

    fn model() -> &'static CStr {
        match Self::kind() {
            Kind::HealthKit => c"models/w_medkit.mdl",
            Kind::Battery => c"models/w_battery.mdl",
        }
    }

    fn my_touch(&mut self, cx: &mut Cx, p: *mut edict_t) -> bool {
        if unsafe { (*p).v.deadflag } != DEAD_NO as i32 {
            return false;
        }
        let cls = s(cx.pev().classname);
        match Self::kind() {
            Kind::HealthKit => {
                if !player_take_health(p, 50.0) {
                    return false;
                }
                messages::one(Msg::ItemPickup, p).string(cls).end();
                emit(p, CHAN_ITEM, c"items/smallmedkit1.wav", 1.0, ATTN_NORM, 0, PITCH_NORM as i32);
            }
            Kind::Battery => {
                if unsafe { (*p).v.armorvalue } >= 100.0 {
                    return false;
                }
                // TS ignores TakeArmor's result: the battery is used up either way.
                player_take_armor(p, 50.0);
                emit(p, CHAN_ITEM, c"items/gunpickup2.wav", 1.0, ATTN_NORM, 0, PITCH_NORM as i32);
                messages::one(Msg::ItemPickup, p).string(cls).end();
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

    fn touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        if !self.touchable || !is_player(other) {
            return;
        }
        let ov = unsafe { &(*other).v };
        if ov.deadflag != DEAD_NO as i32 || ov.health <= 0.0 {
            return;
        }
        if self.my_touch(cx, other) {
            let me = cx.edict;
            sub_use_targets(me, cx.pev().target, 0, 0.0, other, UseType::Toggle, 0.0);
            self.touchable = false;
            if cx.pev().spawnflags & SF_NORESPAWN == 0 {
                self.respawn(cx);
            } else {
                remove(me);
            }
        }
    }
}

pub type HealthKit = Item<0>;
pub type Battery = Item<1>;
