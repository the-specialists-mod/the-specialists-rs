//! `CBotModuleAttack` (bit 4, bots.md §8): the module's state in the brain and the helpers the
//! rulesets' attack modules share. The module's think is the rules' hook
//! ([`super::rules::NativeBotRules::attack`]).

use crate::player::Player;
use crate::weapons;
use goldsrc::sys::*;
use shared::weapons::{kind, kind_of};

/// The module's fields (brain+0x18ac).
#[derive(Clone, Copy, Debug, Default)]
pub struct Attack {
    /// +0x04 next think, +0x08 next fire decision, +0x0c next retarget, +0x10 next dodge,
    /// +0x14 cover spot valid until, +0x18 next close-range jump / stock hit.
    pub next_think: f32,
    pub next_fire: f32,
    pub next_retarget: f32,
    pub next_dodge: f32,
    pub next_cover: f32,
    pub next_melee: f32,
    /// +0x1c: the cached cover spot.
    pub cover_cache: [f32; 3],
    /// +0x28: aim offset from the enemy's origin, re-rolled each think.
    pub aim_off: [f32; 3],
    /// +0x34: knife-throw lead, decaying every frame.
    pub lead: [f32; 3],
}

/// A player edict the brain points at, if it still has a player (guard: TS keeps a raw
/// `CBaseEntity *`; the port's edict handle may outlive the player).
pub fn live(e: *mut edict_t) -> Option<*mut edict_t> {
    (!e.is_null() && crate::rules::has_player(e)).then_some(e)
}

/// `BotHoldingMelee` (0x1006cfc0): the gun is the active item and holds kung fu (no weapon
/// status), a knife or a katana.
pub fn holding_melee(p: &Player) -> bool {
    if !weapons::gun_active(p) {
        return false;
    }
    let cur = p.inv.gun.current;
    cur == 0 || matches!(kind_of(cur), kind::KNIFE | kind::KATANA)
}
