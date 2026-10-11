//! TS map entities (ts_powerup, ts_groundweapon, ts_model, ...). Owner: rules work package.
//! Reference: decomp docs/server/gamerules-entities.md §3-6 (reconstructed FGD in §6),
//! architecture.md §5 (classname table, HL remaps).

#![allow(non_snake_case)] // weapon_9MMAR & co. are case-sensitive export names

pub mod ctb;
pub mod ground;
pub mod misc;
pub mod powerup;
pub mod shrapnel;
pub mod triggers;

use super::link_entities;
use crate::player::Player;
use crate::world::Cx;
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::ffi::CStr;

link_entities! {
    // power-ups
    ts_powerup => powerup::PowerUp,
    item_longjump => powerup::PowerUp,
    // ground items and the HL classnames TS re-points at them
    ts_groundweapon => ground::GroundWeapon,
    weapon_crossbow => ground::GroundWeapon,
    weapon_shotgun => ground::GroundWeapon,
    weapon_gauss => ground::GroundWeapon,
    weapon_egon => ground::GroundWeapon,
    weapon_rpg => ground::GroundWeapon,
    weapon_9MMAR => ground::GroundWeapon,
    weapon_hornetgun => ground::GroundWeapon,
    weapon_satchel => ground::GroundWeapon,
    weapon_snark => ground::GroundWeapon,
    weapon_handgrenade => ground::GroundWeapon,
    ammo_ARgrenades => ground::GroundWeapon,
    ts_groundammo => ground::GroundAmmo,
    ammo_9mmAR => ground::GroundAmmo,
    ammo_buckshot => ground::GroundAmmo,
    ammo_crossbow => ground::GroundAmmo,
    ammo_gaussclip => ground::GroundAmmo,
    ammo_rpgclip => ground::GroundAmmo,
    // map setup and effects
    ts_model => misc::Model,
    ts_mapglobals => misc::MapGlobals,
    ts_wind => misc::Wind,
    ts_fog => misc::Fog,
    ts_emitter => misc::Emitter,
    ts_particle => misc::Particle,
    deadcorpse => misc::DeadCorpse,
    info_player_team1 => crate::entities::stock::SpawnPoint,
    info_player_team2 => crate::entities::stock::SpawnPoint,
    info_player_team3 => crate::entities::stock::SpawnPoint,
    info_player_team4 => crate::entities::stock::SpawnPoint,
    // logic
    ts_trigger => triggers::Trigger,
    ts_dmhill => triggers::DmHill,
    ts_timer => triggers::Timer,
    ts_customtarget => triggers::CustomTarget,
    ts_playeramount => triggers::PlayerAmount,
    ts_slowmotion => triggers::SlowMotion,
    ts_slowmotionpoint => triggers::SlowMotionPoint,
    // capture the briefcase
    ts_briefcase => ctb::Briefcase,
    ts_flag => ctb::Flag,
    ts_wingiver => ctb::WinGiver<false>,
    ts_scoregiver => ctb::WinGiver<true>,
    // objective entities that are dead in 3.0
    ts_objective_manager => misc::Stub,
    ts_objective_ptr => misc::Stub,
    ts_bomb => misc::Stub,
    ts_hack => misc::Stub,
    ts_teamescape => misc::Stub,
    ts_sellzone => misc::Stub,
    // #95: CTSShrapnel (only a map can place one: nothing calls CTSShrapnel::Create)
    shrapnel => shrapnel::Shrapnel,
}

/// Models and sounds the TS entities need even when created at runtime.
pub fn precache() {
    let e = engine();
    e.precache_model(powerup::MODEL);
    e.precache_model(ctb::MODEL);
    e.precache_model(c"models/ammopack.mdl");
    misc::PARTICLES.0.set(0);
}

/// `ts_playeramount` hooks from ClientConnect/ClientDisconnect.
pub fn player_amount_changed(connected: bool) {
    let eng = engine();
    let mut it = std::ptr::null_mut();
    let mut list = Vec::new();
    loop {
        it = eng.find_by_string(it, c"classname", c"ts_playeramount");
        if it.is_null() {
            break;
        }
        list.push(it);
    }
    for e in list {
        crate::world::dispatch(e, |ent, cx| {
            if let Some(p) = (ent as &mut dyn std::any::Any).downcast_mut::<triggers::PlayerAmount>() {
                p.changed(cx, connected);
            }
        });
    }
}

pub fn drop_flag(p: &mut Player, cx: &mut Cx, captured: bool) {
    ctb::drop_flag(p, cx, captured)
}

pub fn has_flag(e: *mut edict_t) -> bool {
    ctb::has_flag(e)
}

pub fn create_dead_corpse(player: *mut edict_t, life: f32) {
    misc::create_dead_corpse(player, life)
}

pub fn create_slow_motion_point(origin: [f32; 3], amount: f32, duration: f32, btime: bool) {
    triggers::create_slow_motion_point(origin, amount, duration, btime)
}

pub fn create_powerup(kind: u16, secs: i32, origin: [f32; 3]) {
    powerup::create(kind, secs, origin);
}

/// Ground weapon classnames (`ts_groundweapon` and its HL remaps); bots walk to these.
pub fn is_usable(class: &CStr) -> bool {
    class == c"ts_groundweapon" || ground::hl_weapon_ids(class).is_some()
}

/// `ObjectCaps()` of the TS entities, for the player's +use search (`player::player_use`):
/// all of them are `FCAP_CONTINUOUS_USE` (0x10). Ground weapons (`CTSGroundWeapon::ObjectCaps`
/// 0xa34a0), ground ammo (0xa34f4, whose `Use` 0xa2f3c is empty: it is picked up on touch, but
/// still takes the +use search), dropped guns (`CTSWorldGun` 0xa3404), thrown blades (`CTSKnife`
/// 0x7f04c), grenades (`CM61Grenade` 0x7effc; classname `grenade`), shrapnel (`CTSShrapnel`
/// 0x7f09c, no `Use`), the CTB case (`CTSFlag` 0x9e704) and the dead hidden objectives (`CTSBomb` 0x8e738, `CTSHack` 0x8e788,
/// `CTSTeamEscape` 0x8e820). 0 for anything else.
pub fn object_caps(class: &CStr) -> i32 {
    let usable = matches!(
        class.to_bytes(),
        b"ts_groundweapon"
            | b"ts_groundammo"
            | b"WorldGun"
            | b"knife"
            | b"grenade"
            | b"shrapnel"
            | b"ts_flag"
            | b"ts_bomb"
            | b"ts_hack"
            | b"ts_teamescape"
    ) || ground::hl_weapon_ids(class).is_some()
        || ground::hl_ammo_weapons(class).is_some();
    if usable { crate::entities::stock::FCAP_CONTINUOUS_USE } else { 0 }
}

/// `CanUsedTroughWalls()` (vtable slot 3; `CBaseEntity` 0xac5b0 returns 1): false for ground
/// weapons (0xa34a8), ground ammo (0xa34fc), dropped guns (0xa340c) and the CTB case (0x9e70c),
/// which `PlayerUse` only reaches with a clear line from the eyes.
pub fn can_use_through_walls(class: &CStr) -> bool {
    !(matches!(class.to_bytes(), b"ts_groundweapon" | b"ts_groundammo" | b"WorldGun" | b"ts_flag")
        || ground::hl_weapon_ids(class).is_some()
        || ground::hl_ammo_weapons(class).is_some())
}

/// The `Use` of a ground weapon (`CTSGroundWeapon::Use` 0xa230c → `GetWeapon`) or the CTB case
/// (`CTSFlag::Use` 0x9e5dc → `PlayerTakeFlag`) when a player's +use picks it; the player is being
/// dispatched, so it is passed directly. False if `ent` is neither.
pub fn use_by_player(ent: &mut dyn std::any::Any, icx: &mut Cx, p: &mut Player, cx: &mut Cx) -> bool {
    if let Some(g) = ent.downcast_mut::<ground::GroundWeapon>() {
        g.player_use(icx, p, cx);
    } else if let Some(f) = ent.downcast_mut::<ctb::Flag>() {
        f.use_by_player(icx, p, cx);
    } else {
        return false;
    }
    true
}

/// `CTSGameRules::RespawnEntities` (0x77764), from `RestartRound`:
/// 1. every broken breakable (`info_breakable` → `BreakableRespawn(false)`);
/// 2. in round modes 2/3 or LMS, `CTSGroundWeapon::Fall` on every `ts_groundweapon` (only that
///    classname: not the HL remaps or ground ammo), which shows a taken one again at once;
/// 3. in LMS, `CTSPowerUp::Fall` on every `ts_powerup`;
/// 4. `CTSFlag::RespawnFlag` on every case.
///
/// TS shares one `UTIL_FindEntityByClassname` cursor between the four loops; each loop only
/// stops early on an entity without an edict (or worldspawn), which no class here can be, so
/// every loop starts from the beginning.
pub fn respawn_entities() {
    crate::entities::stock::breakable::respawn_all(false);
    let (round_mode, lms) = crate::rules::state::with(|g| {
        (matches!(g.mode, crate::rules::state::Mode::RoundTdm | crate::rules::state::Mode::Objective), g.lms)
    });
    if round_mode || lms {
        for_each_class(c"ts_groundweapon", |ent, cx| {
            if let Some(g) = ent.downcast_mut::<ground::GroundWeapon>() {
                g.fall(cx);
            }
        });
    }
    if lms {
        for_each_class(c"ts_powerup", |ent, cx| {
            if let Some(p) = ent.downcast_mut::<powerup::PowerUp>() {
                p.fall(cx);
            }
        });
    }
    for_each_class(c"ts_flag", |ent, cx| {
        if let Some(f) = ent.downcast_mut::<ctb::Flag>() {
            f.round_respawn(cx);
        }
    });
}

/// `UTIL_FindEntityByClassname` over `class`, dispatching each match (the next one is looked up
/// after the call, as TS's loops do, so an entity removed by the call is still stepped past).
fn for_each_class(class: &CStr, mut f: impl FnMut(&mut dyn std::any::Any, &mut Cx)) {
    let eng = engine();
    let mut it = std::ptr::null_mut();
    loop {
        it = eng.find_by_string(it, c"classname", class);
        if it.is_null() {
            return;
        }
        crate::world::dispatch(it, |ent, cx| f(ent as &mut dyn std::any::Any, cx));
    }
}
