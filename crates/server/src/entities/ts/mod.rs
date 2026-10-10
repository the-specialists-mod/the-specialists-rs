//! TS map entities (ts_powerup, ts_groundweapon, ts_model, ...). Owner: rules work package.
//! Reference: decomp docs/server/gamerules-entities.md §3-6 (reconstructed FGD in §6),
//! architecture.md §5 (classname table, HL remaps).

#![allow(non_snake_case)] // weapon_9MMAR & co. are case-sensitive export names

pub mod ctb;
pub mod ground;
pub mod misc;
pub mod powerup;
pub mod triggers;

use super::link_entities;
use crate::player::Player;
use crate::world::Cx;
use goldsrc::edict::EdictPtr;
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
    weapon_9mmAR => ground::GroundWeapon,
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
    info_player_team1 => misc::TeamSpawn,
    info_player_team2 => misc::TeamSpawn,
    info_player_team3 => misc::TeamSpawn,
    info_player_team4 => misc::TeamSpawn,
    info_player_teamspawn => misc::TeamSpawn,
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

/// Classnames `+use` can pick up (ground weapons and their HL remaps).
pub fn is_usable(class: &CStr) -> bool {
    class == c"ts_groundweapon" || ground::hl_weapon_ids(class).is_some()
}

/// `+use` on a ground item from the player's own think (the player is being dispatched).
pub fn player_use_item(item: *mut edict_t, p: &mut Player, cx: &mut Cx) {
    crate::world::dispatch(item, |ent, icx| {
        if let Some(g) = (ent as &mut dyn std::any::Any).downcast_mut::<ground::GroundWeapon>() {
            g.player_use(icx, p, cx);
        }
    });
}

/// `RespawnEntities` (round restart): hidden ground items and map power-ups come back now.
pub fn respawn_entities() {
    let eng = engine();
    for class in [c"ts_groundweapon", c"ts_groundammo", c"ts_powerup"] {
        let mut it = std::ptr::null_mut();
        loop {
            it = eng.find_by_string(it, c"classname", class);
            if it.is_null() {
                break;
            }
            let v = it.v();
            if v.effects & EF_NODRAW as i32 != 0 {
                v.nextthink = eng.time() + 0.1;
            }
        }
    }
}
