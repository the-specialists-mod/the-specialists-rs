//! TS 2.1's map entities (issue #100 §3.9; decomp `docs/ts21/bots-entities.md` (a), `rules.md`
//! §9.2/§9.5). Addresses are 2.1 `mp.dll` VAs.
//!
//! * The classes 2.1 doesn't export don't spawn ([`spawns`]).
//! * Keys 2.1's `KeyValue`s don't have are refused, and those it reads differently are parsed
//!   its way ([`key_value`]).
//! * `ts_trigger` and `ts_groundweapon` touch as 2.1's do ([`touch`]); a ground weapon is given
//!   as 2.1's `GetWeapon` gives it ([`give_ground_weapon`]).
//! * Broken breakables and doors don't rebuild themselves: 2.1's `RespawnEntities`
//!   ([`respawn_entities`]) restores them, every 60 s in deathmatch and teamplay
//!   (the 2.1 Think in `rules.rs`) and at a round restart.
//! * `ts_mapglobals` gives only the weapon mask ([`map_globals`]); its keys are read by
//!   [`map_globals_keys`].

use goldsrc::edict::EdictPtr;
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::any::Any;
use std::ffi::CStr;
use ts_server::entities::stock::breakable::{self, FuncBreakable, FuncPushable};
use ts_server::entities::stock::doors::Door;
use ts_server::entities::ts::ground::{GroundWeapon, weapon_valid};
use ts_server::entities::ts::misc::{MapFlags, MapGlobals};
use ts_server::entities::ts::objective::ObjectivePointer;
use ts_server::entities::ts::powerup::PowerUp;
use ts_server::entities::ts::triggers::Trigger;
use ts_server::player::Player;
use ts_server::rules::state::{self, Mode};
use ts_server::rules::{self, cvars, msg, teams};
use ts_server::world::{Cx, Entity, UseType};

/// The classes TS 3.0's `mp.dll` exports and 2.1's doesn't (the two PE export tables, 307 and
/// 354 names: decomp report (a.1)). In 2.1 the engine finds no spawn function for them ("Can't
/// init") and frees the entity. Every other class the port links is in 2.1 too.
pub const REFUSED_CLASSES: [&str; 33] = [
    // TS 3.0's new entities.
    "ts_briefcase",
    "ts_flag",
    "ts_customtarget",
    "ts_emitter",
    "ts_particle",
    "ts_fog",
    "ts_groundammo",
    "ts_playeramount",
    "ts_scoregiver",
    "ts_sellzone",
    "ts_timer",
    "ts_wind",
    // The breakable respawn helper and the team spawns.
    "info_breakable",
    "info_player_team1",
    "info_player_team2",
    "info_player_team3",
    "info_player_team4",
    // The HLDM weapons and ammo 3.0 turns into TS ground items (2.04 removed them).
    "weapon_9MMAR",
    "weapon_crossbow",
    "weapon_egon",
    "weapon_gauss",
    "weapon_handgrenade",
    "weapon_hornetgun",
    "weapon_rpg",
    "weapon_satchel",
    "weapon_shotgun",
    "weapon_snark",
    "ammo_9mmAR",
    "ammo_ARgrenades",
    "ammo_buckshot",
    "ammo_crossbow",
    "ammo_gaussclip",
    "ammo_rpgclip",
];

/// Whether an entity of `class` spawns in 2.1: not one of [`REFUSED_CLASSES`]. Compared without
/// case, so a classname an engine would match to a 3.0 export another way is refused too.
pub fn spawns(class: &CStr) -> bool {
    let class = class.to_bytes();
    !REFUSED_CLASSES.iter().any(|c| c.as_bytes().eq_ignore_ascii_case(class))
}

// ---- keyvalues -----------------------------------------------------------------------------------

/// The classes whose keys 2.1 reads differently from 3.0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// `func_breakable` and `func_pushable` (`CBreakable::KeyValue` 0x1001b050).
    Breakable,
    /// `func_door`, `func_door_rotating`, `func_water` (`CBaseDoor::KeyValue`).
    Door,
    /// `ts_groundweapon` (`CTSGroundWeapon::KeyValue` 0x10065bb0).
    GroundWeapon,
    /// `ts_trigger` (`CTSTrigger::KeyValue` 0x10040a70).
    Trigger,
    /// `ts_mapglobals` (`CTSMapGlobals::KeyValue` 0x10042090).
    MapGlobals,
    /// `ts_powerup` (`CTSPowerUp::KeyValue` 0x10059c10).
    PowerUp,
    /// `ts_objective_ptr` (`CTSObjectivePointer::KeyValue` 0x10050800).
    ObjectivePointer,
}

/// What 2.1 does with a class key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Key {
    /// The same as 3.0: the class parses it.
    Class,
    /// 2.1's class doesn't know it (`fHandled` 0, nothing stored).
    Refused,
    /// `ts_trigger`'s `allowedteam` as is (3.0 turns 2 into "anyone").
    AllowedTeam,
    /// `ts_mapglobals`'s `playerrespawn`, which 3.0 ignores.
    PlayerRespawn,
    /// `ts_mapglobals`'s `playeratatime`, which 3.0 ignores.
    PlayerAtATime,
}

/// 2.1's handling of `key` for an entity of `kind` (keys compared as each 2.1 `KeyValue` does,
/// with case).
fn key_rule(kind: Kind, key: &[u8]) -> Key {
    match (kind, key) {
        // No per-entity respawn timer in 2.1.
        (Kind::Breakable, b"respawntime" | b"respawntarget") | (Kind::Door, b"respawntime") => Key::Refused,
        // No buying or team restriction.
        (Kind::GroundWeapon, b"buyable" | b"attackerteam") => Key::Refused,
        (Kind::Trigger, b"allowedteam") => Key::AllowedTeam,
        (Kind::Trigger, b"attackerteam") => Key::Refused,
        (Kind::MapGlobals, b"playerrespawn") => Key::PlayerRespawn,
        (Kind::MapGlobals, b"playeratatime") => Key::PlayerAtATime,
        (Kind::PowerUp, b"pwupmult") => Key::Refused,
        // Two teams' messages only.
        (Kind::ObjectivePointer, b"objmsgspec" | b"objmsgmerc") => Key::Class,
        (Kind::ObjectivePointer, _) => Key::Refused,
        _ => Key::Class,
    }
}

fn kind_of(ent: &mut dyn Entity, _cx: &mut Cx) -> Option<Kind> {
    let any = ent as &mut dyn Any;
    if any.is::<FuncBreakable>() || any.is::<FuncPushable>() {
        Some(Kind::Breakable)
    } else if any.is::<Door>() {
        Some(Kind::Door)
    } else if any.is::<GroundWeapon>() {
        Some(Kind::GroundWeapon)
    } else if any.is::<Trigger>() {
        Some(Kind::Trigger)
    } else if any.is::<MapGlobals>() {
        Some(Kind::MapGlobals)
    } else if any.is::<PowerUp>() {
        Some(Kind::PowerUp)
    } else {
        any.is::<ObjectivePointer>().then_some(Kind::ObjectivePointer)
    }
}

/// `ServerRules::entity_key_value` in 2.1.
pub fn key_value(ent: &mut dyn Entity, cx: &mut Cx, key: &CStr, value: &CStr) -> Option<bool> {
    let kind = kind_of(ent, cx)?;
    let rule = key_rule(kind, key.to_bytes());
    let any = ent as &mut dyn Any;
    match rule {
        Key::Class => None,
        Key::Refused => Some(false),
        Key::AllowedTeam => {
            // `strtod`, then `__ftol` (truncation; the integer indefinite out of range).
            let t = any.downcast_mut::<Trigger>()?;
            t.team = ts_server::entities::ts::c_int(ts_server::entities::stock::base::kv_d(value));
            Some(true)
        }
        Key::PlayerRespawn | Key::PlayerAtATime => {
            let m = any.downcast_mut::<MapGlobals>()?;
            let v = rules::mapcycle::atoi(&value.to_string_lossy());
            if rule == Key::PlayerRespawn {
                m.player_respawn = v;
            } else {
                m.player_at_a_time = v;
            }
            Some(true)
        }
    }
}

// ---- touches -------------------------------------------------------------------------------------

/// `ServerRules::entity_touch` in 2.1: `ts_trigger` and `ts_groundweapon`.
pub fn touch(ent: &mut dyn Entity, cx: &mut Cx, other: *mut edict_t) -> bool {
    let any = ent as &mut dyn Any;
    if let Some(t) = any.downcast_mut::<Trigger>() {
        trigger_touch(t, cx, other);
    } else if let Some(g) = any.downcast_mut::<GroundWeapon>() {
        // `CTSGroundWeapon::WaitTouch` (0x10066440): `GetWeapon` for a toucher holding +use.
        if g.touchable() && !other.is_null() && other.v().button & IN_USE as i32 != 0 {
            g.get_weapon_for(cx, other);
        }
    } else {
        return false;
    }
    true
}

/// Whether a `ts_trigger` with `allowedteam` `allowed` fires for a player of team `team`
/// (`TeamID()`): 0 only "Specialists", 1 only "Mercenary" (`stricmp`, FUN_1007e860), 2 or more
/// anyone, a negative value no one. A trigger without the key is 0.
fn trigger_fires(allowed: i32, team: &[u8]) -> bool {
    match allowed {
        0 => team.eq_ignore_ascii_case(b"Specialists"),
        1 => team.eq_ignore_ascii_case(b"Mercenary"),
        a => a >= 2,
    }
}

/// `CTSTrigger::WaitTouch` (0x10040c00): a living player fires the target (as activator and
/// caller) if its team may, and either way the trigger waits `max(nextcheck, 0.5)` again. No
/// `message`, no case-carrier spawnflag.
fn trigger_touch(t: &mut Trigger, cx: &mut Cx, other: *mut edict_t) {
    let now = cx.time();
    if !t.touchable() || now < t.ready_at || !rules::has_player(other) || !rules::alive(other.v()) {
        return;
    }
    if trigger_fires(t.team, &teams::team_name_of(other)) {
        let target = cx.pev().target;
        if target != 0 {
            ts_server::entities::fire_targets(engine().string(target), cx.edict, cx.edict, UseType::Toggle, 0.0);
        }
    }
    // `nextcheck <= 0.5 ? 0.5 : nextcheck`.
    t.ready_at = now + if t.next_check <= 0.5 { 0.5 } else { t.next_check };
}

/// `ServerRules::give_ground_weapon` in 2.1: `GetWeapon` (FUN_10066250) past `IsPlayer`. A
/// living, visible player picks a visible weapon up (`PickUpWeapon(id, NULL, spawnflags, 1)`),
/// with no price; then the weapon hides and schedules its respawn, a negative `wextraclip` is
/// clamped to 0, the extra rounds go to an empty reserve (`items/9mmclip1.wav`, the SDK's
/// `EMIT_SOUND_DYN` 0x10036300: no slow-motion pitch), `message` shows at the top
/// (`TSHudMessageOne(0,0,0, 1, 30)`) and `target` fires.
pub fn give_ground_weapon(g: &mut GroundWeapon, cx: &mut Cx, p: &mut Player, pcx: &mut Cx) {
    let hidden = |v: &entvars_t| v.effects & EF_NODRAW as i32 != 0;
    if !rules::alive(pcx.pev()) || hidden(pcx.pev()) || hidden(cx.pev()) {
        return;
    }
    // Only a restored id can be invalid (`Spawn` removes the rest); the gun refuses it.
    let flags = cx.pev().spawnflags;
    if !weapon_valid(g.id) || !ts_server::weapons::give_weapon(p, pcx, g.id as u8, flags as u32) {
        return;
    }
    g.taken(cx);
    g.extra_clip = g.extra_clip.max(0);
    let extra = g.extra_clip;
    if extra != 0
        && let Some(w) = shared::weapons::tables().weapon(g.id as u8)
    {
        let empty = p.inv.gun.reserve.get(w.ammo as usize).is_some_and(|&r| r < 1);
        if empty && ts_server::weapons::give_ammo(p, pcx, w.ammo, extra) != 0 {
            engine().emit_sound(pcx.edict, CHAN_ITEM as i32, c"items/9mmclip1.wav", 1.0, 0.8, 0, 100);
        }
    }
    let (message, target) = (cx.pev().message, cx.pev().target);
    if message != 0 {
        msg::ts_message_one(pcx.edict, [0, 0, 0], 1, 30, engine().string(message).to_bytes());
    }
    if target != 0 {
        ts_server::entities::fire_targets(engine().string(target), cx.edict, cx.edict, UseType::Toggle, 0.0);
    }
}

// ---- RespawnEntities -----------------------------------------------------------------------------

/// `CTSGameRules::RespawnEntities(force)` (0x10044400): `BreakableRespawn(force)` (vtable slot
/// 0) on every `func_breakable`, then every `func_door`, then every `func_door_rotating`, by
/// classname (so not `func_pushable` or `func_water`); in round modes 2/3 or with
/// `lastmanstanding` on, `CTSGroundWeapon::Fall` on every `ts_groundweapon`, and with
/// `lastmanstanding` on, `CTSPowerUp::Fall` on every `ts_powerup`. Think passes `false`,
/// `RestartRound` `true`.
///
/// 2.1's `BreakableRespawn` (`CBreakable` 0x100423f0, `CBaseDoor` 0x10013b80, `CRotDoor`
/// 0x10015400) restores a broken (or forced) entity at once: no wait for players in the way and
/// no target (`ServerRules::breakable_respawn_guarded`).
pub fn respawn_entities(force: bool) {
    let eng = engine();
    for class in [c"func_breakable", c"func_door", c"func_door_rotating"] {
        let mut e = std::ptr::null_mut();
        loop {
            e = eng.find_by_string(e, c"classname", class);
            if e.is_null() {
                break;
            }
            ts_server::ham::breakable_respawn(e, force, |ent, cx, force| {
                breakable::breakable_respawn(ent, cx, force);
            });
        }
    }
    let lms = cvars::f(c"lastmanstanding") != 0.0;
    let round_mode = matches!(state::with(|g| g.mode), Mode::RoundTdm | Mode::Objective);
    if round_mode || lms {
        ts_server::entities::ts::for_each_class(c"ts_groundweapon", |ent, cx| {
            if let Some(g) = ent.downcast_mut::<GroundWeapon>() {
                g.fall(cx);
            }
        });
        if lms {
            ts_server::entities::ts::for_each_class(c"ts_powerup", |ent, cx| {
                if let Some(p) = ent.downcast_mut::<PowerUp>() {
                    p.fall(cx);
                }
            });
        }
    }
}

// ---- ts_mapglobals -------------------------------------------------------------------------------

/// `ServerRules::map_globals` in 2.1. 2.1's `CTSMapGlobals::Spawn` (0x10042110) sets nothing; the
/// rules' first Think (0x10043cc0) reads the first `ts_mapglobals` (`UTIL_FindEntityByClassname`
/// from the start) once: the weapon mask `spawnflags ^ InitWeaponRestriction()` (FUN_10042190).
/// So only the first one counts, and flags 32 (no round timer) and 64 (super jump for all) do
/// nothing: the round timer runs and super jump comes from the `superjump` cvar.
pub fn map_globals(e: *mut edict_t, spawnflags: i32) -> Option<MapFlags> {
    let first = engine().find_by_string(std::ptr::null_mut(), c"classname", c"ts_mapglobals");
    (first == e).then_some(map_flags(spawnflags))
}

/// The map flags of 2.1's (first) `ts_mapglobals`.
fn map_flags(spawnflags: i32) -> MapFlags {
    MapFlags { round_timer: true, superjump: false, weapons: 0x1f ^ spawnflags }
}

/// The two keys of the first `ts_mapglobals` (`(playerrespawn, playeratatime)`), as the rules'
/// first Think (0x10043cc0) copies them into `CTSGameRules+0x54cc` / `+0x54cd`: bytes (the
/// `int`s truncated), 0 without one. `playerrespawn` limits respawns per round in objective
/// mode (`SetPlayerRespawnTime`, FUN_10042d50); nothing reads `playeratatime`.
pub fn map_globals_keys() -> (i8, i8) {
    let first = engine().find_by_string(std::ptr::null_mut(), c"classname", c"ts_mapglobals");
    if first.is_null() {
        return (0, 0);
    }
    ts_server::world::with::<MapGlobals, _>(first, |m| (m.player_respawn as i8, m.player_at_a_time as i8)).unwrap_or((0, 0))
}

/// The `g_vecAttackDir` 2.1's `CBreakable::TakeDamage` (0x1001b960..0x1001bad7) stores, issue
/// #109: the box centre `size * 0.5 + absmin` is stored as a `float` for x only (y and z stay on
/// the FPU), each difference to the inflictor's origin is stored, the length sums `z² + y² + x²`
/// on the FPU, and a zero length (`fcom 0; test ah,0x44; jp`) gives `(0, 0, 1)` while a NaN one is
/// normalised. 3.0's goes through `Vector::Normalize` instead.
pub fn breakable_attack_dir(org: [f32; 3], absmin: [f32; 3], size: [f32; 3]) -> [f32; 3] {
    let half = |i: usize| f64::from(size[i]) * 0.5 + f64::from(absmin[i]);
    let centre = [half(0) as f32 as f64, half(1), half(2)];
    let d: [f64; 3] = std::array::from_fn(|i| (f64::from(org[i]) - centre[i]) as f32 as f64);
    let len = (d[2] * d[2] + d[1] * d[1] + d[0] * d[0]).sqrt();
    if len == 0.0 {
        return [0.0, 0.0, 1.0];
    }
    let inv = 1.0 / len;
    d.map(|c| (c * inv) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The centre's x is stored, its y and z aren't: with a box at 2^24 a half unit survives in y.
    #[test]
    fn breakable_attack_dir_keeps_the_centre_on_the_fpu() {
        let big = 16_777_216.0;
        // Centre y 2^24 + 0.5 (a `float` would round it to 2^24): d = (1.5, 1.5, 0).
        let d = breakable_attack_dir([1.5, big + 2.0, 0.0], [0.0, big, 0.0], [0.0, 1.0, 0.0]);
        let n = (1.5 / 4.5f64.sqrt()) as f32;
        assert_eq!(d, [n, n, 0.0]);
        // x's centre is stored, 2^24: d = (2, 1.5, 0).
        assert_eq!(breakable_attack_dir([big + 2.0, 1.5, 0.0], [big, 0.0, 0.0], [1.0, 0.0, 0.0]), [0.8, 0.6, 0.0]);
        assert_eq!(breakable_attack_dir([1.0; 3], [0.0; 3], [2.0; 3]), [0.0, 0.0, 1.0], "zero length");
        assert!(breakable_attack_dir([f32::NAN, 0.0, 0.0], [0.0; 3], [2.0; 3])[0].is_nan(), "a NaN is normalised");
    }

    #[test]
    fn three_0_only_classes_are_refused() {
        for c in [c"ts_flag", c"ts_briefcase", c"info_breakable", c"info_player_team3", c"weapon_shotgun", c"ammo_9mmAR", c"ts_fog"] {
            assert!(!spawns(c), "{c:?}");
        }
        // The engines may find an export without case.
        assert!(!spawns(c"TS_FLAG"));
        for c in [c"ts_groundweapon", c"ts_powerup", c"func_breakable", c"info_player_deathmatch", c"ts_mapglobals", c"ts_trigger"] {
            assert!(spawns(c), "{c:?}");
        }
        assert_eq!(REFUSED_CLASSES.len(), 33);
    }

    #[test]
    fn keys_2_1_lacks_are_refused() {
        assert_eq!(key_rule(Kind::Breakable, b"respawntime"), Key::Refused);
        assert_eq!(key_rule(Kind::Breakable, b"respawntarget"), Key::Refused);
        assert_eq!(key_rule(Kind::Breakable, b"material"), Key::Class);
        assert_eq!(key_rule(Kind::Breakable, b"explodemagnitude"), Key::Class);
        assert_eq!(key_rule(Kind::Door, b"respawntime"), Key::Refused);
        assert_eq!(key_rule(Kind::Door, b"maxdamage"), Key::Class);
        assert_eq!(key_rule(Kind::GroundWeapon, b"buyable"), Key::Refused);
        assert_eq!(key_rule(Kind::GroundWeapon, b"attackerteam"), Key::Refused);
        assert_eq!(key_rule(Kind::GroundWeapon, b"wextraclip"), Key::Class);
        assert_eq!(key_rule(Kind::Trigger, b"attackerteam"), Key::Refused);
        assert_eq!(key_rule(Kind::Trigger, b"allowedteam"), Key::AllowedTeam);
        assert_eq!(key_rule(Kind::Trigger, b"nextcheck"), Key::Class);
        assert_eq!(key_rule(Kind::MapGlobals, b"playerrespawn"), Key::PlayerRespawn);
        assert_eq!(key_rule(Kind::MapGlobals, b"playeratatime"), Key::PlayerAtATime);
        assert_eq!(key_rule(Kind::PowerUp, b"pwupmult"), Key::Refused);
        assert_eq!(key_rule(Kind::PowerUp, b"pwuptype"), Key::Class);
        assert_eq!(key_rule(Kind::ObjectivePointer, b"objmsgspec"), Key::Class);
        assert_eq!(key_rule(Kind::ObjectivePointer, b"objmsgmerc"), Key::Class);
        assert_eq!(key_rule(Kind::ObjectivePointer, b"objmsgteam3"), Key::Refused);
        assert_eq!(key_rule(Kind::ObjectivePointer, b"objmsgall"), Key::Refused);
        // 2.1 compares with case.
        assert_eq!(key_rule(Kind::Breakable, b"RespawnTime"), Key::Class);
    }

    #[test]
    fn trigger_teams_by_name() {
        // No `allowedteam` key: the Specialists only.
        assert!(trigger_fires(0, b"Specialists"));
        assert!(trigger_fires(0, b"specialists"));
        assert!(!trigger_fires(0, b"Mercenary"));
        assert!(!trigger_fires(0, b""));
        assert!(trigger_fires(1, b"MERCENARY"));
        assert!(!trigger_fires(1, b"Specialists"));
        // 2 and up: anyone, even without a team.
        assert!(trigger_fires(2, b""));
        assert!(trigger_fires(7, b"Specialists"));
        // Negative: no one.
        assert!(!trigger_fires(-1, b"Specialists"));
        assert!(!trigger_fires(i32::MIN, b"Mercenary"));
    }

    #[test]
    fn map_flags_keep_only_the_weapon_mask() {
        let m = map_flags(32 | 64 | 2);
        assert!(m.round_timer);
        assert!(!m.superjump);
        assert_eq!(m.weapons, 0x1f ^ (32 | 64 | 2));
        // TS 3.0's for the same flags.
        let m30 = MapFlags::from_spawnflags(32 | 64 | 2);
        assert_eq!((m30.round_timer, m30.superjump, m30.weapons), (false, true, 0x1f ^ (32 | 64 | 2)));
    }
}
