//! [`ServerRules`]: the server-only hooks of a gameplay ruleset (rules, scoring, damage,
//! entities...). Bots aren't part of a ruleset: they are left to a plugin axis of their own.
//!
//! The server's own code resolves them from the active plugin ([`active`](super::active)), which
//! follows `ts_gameplay` once a frame; a switch resets the game first ([`super::reset`]).

use crate::entities::ts::ground::GroundWeapon;
use crate::entities::ts::misc::MapFlags;
use crate::messages::Msg;
use crate::player::Player;
use crate::rules::scoring::{self, Frag};
use crate::world::{Cx, Entity};
use goldsrc::server::MsgWriter;
use goldsrc::sys::edict_t;
use std::ffi::CStr;

/// Server rules. Every default body is TS 3.0's (`shared::gameplay::Core`); the server code calls
/// these where a ruleset can differ. A hook needing server state takes this crate's types, so
/// its 3.0 body can be the code it replaces.
pub trait ServerRules: Sync {
    /// The plugin has become active on a brand-new game: at every map start (after the rules
    /// are installed, before the map's entities spawn) and after a switch to it has reset the
    /// game ([`super::reset`]). The place to set up the plugin's own state.
    fn on_activate(&self) {}

    /// `TSGetPointsForFrag` (0x89140): the points a killing player scores for `frag`, which also
    /// reports whether the kill was a slide or a stunt.
    fn points_for_frag(&self, frag: &Frag<'_>, sliding: &mut bool, diving: &mut bool) -> i32 {
        scoring::points_for_frag(frag, sliding, diving)
    }

    // ---- map entities -------------------------------------------------------------------------

    /// Whether an entity of `class` may spawn (`DispatchSpawn`, for the map's entities and the
    /// game's own): a refused one fails its spawn (-1, the engine frees it) and its class parses
    /// no keyvalue. TS 3.0 has an export for every class the port links: all spawn.
    fn spawns_entity(&self, _class: &CStr) -> bool {
        true
    }

    /// A keyvalue for an entity's class (`DispatchKeyValue`'s class part, after the entvars
    /// table), before the class's own `KeyValue`. `None` lets the class parse it (TS 3.0);
    /// `Some(handled)` means the plugin parsed or refused it, and is `fHandled`.
    fn entity_key_value(&self, _ent: &mut dyn Entity, _cx: &mut Cx, _key: &CStr, _value: &CStr) -> Option<bool> {
        None
    }

    /// An entity's `Touch` (`DispatchTouch` and the game's own touches): true when the plugin ran
    /// it instead of the class. TS 3.0: the class's.
    fn entity_touch(&self, _ent: &mut dyn Entity, _cx: &mut Cx, _other: *mut edict_t) -> bool {
        false
    }

    /// How long a broken `func_breakable` or breakable door waits before it rebuilds itself
    /// (`CBreakable::Die` 0x752e0, `CBaseDoor::TraceAttack` 0xb57ac), given its `respawntime` key (≥ 0); `None`
    /// leaves it broken, with no think, until `RespawnEntities` restores it. TS 3.0: the key, 0
    /// meaning 30 s on the `info_breakable` helper (`RespawnThink` 0x75da0).
    fn broken_respawn_delay(&self, respawn_time: f32) -> Option<f32> {
        Some(if respawn_time == 0.0 { 30.0 } else { respawn_time })
    }

    /// Whether `BreakableRespawn` (`CBreakable` 0x74e84, `CBaseDoor`/`CRotDoor` 0xb5604/0xb7838)
    /// waits while a client is inside the box (retrying in 5 s) and fires a breakable's
    /// `respawntarget` (spawnflag 8). TS 3.0: yes. False restores the entity at once and fires
    /// nothing.
    fn breakable_respawn_guarded(&self) -> bool {
        true
    }

    /// `CTSGameRules::RespawnEntities` as `RestartRound` (0x77920) calls it. TS 3.0's
    /// ([`crate::entities::ts::respawn_entities`]).
    fn respawn_entities(&self) {
        crate::entities::ts::respawn_entities();
    }

    /// The map-entity upkeep of `CTSGameRules::Think` (0x77204) in deathmatch and teamplay, after The One's
    /// check (`now` is the game time). TS 3.0 has none.
    fn entities_think(&self, _now: f32) {}

    /// What a `ts_mapglobals` (`e`) sets with its spawnflags (`CTSMapGlobals::Spawn` 0x72534),
    /// or `None` to set nothing. TS 3.0: [`MapFlags::from_spawnflags`].
    fn map_globals(&self, _e: *mut edict_t, spawnflags: i32) -> Option<MapFlags> {
        Some(MapFlags::from_spawnflags(spawnflags))
    }

    /// Whether a landing `ts_groundweapon` (`OnGround`) also becomes `MOVETYPE_FLY` with
    /// `effects |= 2`, as `ts_groundammo`'s does. TS 3.0: no (`CTSGroundWeapon::OnGround`
    /// 0xa226c).
    fn ground_weapon_lands_flying(&self) -> bool {
        false
    }

    /// `CTSGroundWeapon::GetWeapon` (0xa2328) for a player, past its `IsPlayer` test (the
    /// ground weapon `g` is `cx`, the player `p` is `pcx`). TS 3.0's
    /// ([`GroundWeapon::give`]).
    fn give_ground_weapon(&self, g: &mut GroundWeapon, cx: &mut Cx, p: &mut Player, pcx: &mut Cx) {
        g.give(cx, p, pcx);
    }

    // ---- protocol -----------------------------------------------------------------------------

    /// A user message on its way out (`msg`, written into `w`): hands `send` what goes on the
    /// wire in its place. TS 3.0 sends it as it is.
    fn outgoing_message(&self, _msg: Msg, w: MsgWriter, send: &mut dyn FnMut(MsgWriter)) {
        send(w);
    }
}
