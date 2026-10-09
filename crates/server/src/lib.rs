//! The Specialists server library (the "game DLL"): installed as dlls/ts_i386.so,
//! dlls/ts_amd64.so (FWGS 64-bit) and dlls/mp.dll.
//!
//! This file holds the engine entry points and routes every callback to a subsystem module.
//! See docs/design.md §4 for the module map and hook order.

// Skeleton phase: subsystem APIs exist before every caller does. Remove at integration.
#![allow(dead_code)]

mod bots;
mod commands;
mod damage;
mod entities;
mod messages;
mod net;
mod player;
mod rules;
mod stunt;
mod weapons;
mod world;

use goldsrc::server::{self as sv, engine};
use goldsrc::sys::*;
use goldsrc::util::cstr;
use std::ffi::{c_char, c_int, c_uchar, c_uint, c_void};
use world::UseType;

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GiveFnptrsToDll(funcs: *const enginefuncs_t, globals: *mut globalvars_t) {
    unsafe { sv::init(funcs, globals) };
    goldsrc::panic::install("ts_server_panic.log", |m| engine().print(m));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn GetEntityAPI2(table: *mut DLL_FUNCTIONS, ver: *mut c_int) -> c_int {
    unsafe {
        if table.is_null() || *ver != INTERFACE_VERSION as c_int {
            *ver = INTERFACE_VERSION as c_int;
            return 0;
        }
        *table = dll_functions();
    }
    1
}

/// Legacy entry point (interface version passed by value). Some engines try it first.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn GetEntityAPI(table: *mut DLL_FUNCTIONS, ver: c_int) -> c_int {
    if table.is_null() || ver != INTERFACE_VERSION as c_int {
        return 0;
    }
    unsafe { *table = dll_functions() };
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn GetNewDLLFunctions(table: *mut NEW_DLL_FUNCTIONS, ver: *mut c_int) -> c_int {
    unsafe {
        if table.is_null() || *ver != NEW_DLL_FUNCTIONS_VERSION as c_int {
            *ver = NEW_DLL_FUNCTIONS_VERSION as c_int;
            return 0;
        }
        *table = NEW_DLL_FUNCTIONS {
            pfnOnFreeEntPrivateData: Some(on_free_private_data),
            pfnGameShutdown: Some(game_shutdown),
            pfnShouldCollide: Some(should_collide),
            ..Default::default()
        };
    }
    1
}

/// Every slot is filled: GoldSrc calls most of them without null checks.
fn dll_functions() -> DLL_FUNCTIONS {
    DLL_FUNCTIONS {
        pfnGameInit: Some(game_init),
        pfnSpawn: Some(spawn),
        pfnThink: Some(think),
        pfnUse: Some(use_),
        pfnTouch: Some(touch),
        pfnBlocked: Some(blocked),
        pfnKeyValue: Some(key_value),
        pfnSave: Some(save),
        pfnRestore: Some(restore),
        pfnSetAbsBox: Some(set_abs_box),
        pfnSaveWriteFields: Some(save_fields),
        pfnSaveReadFields: Some(save_fields),
        pfnSaveGlobalState: Some(save_global_state),
        pfnRestoreGlobalState: Some(save_global_state),
        pfnResetGlobalState: Some(noop),
        pfnClientConnect: Some(client_connect),
        pfnClientDisconnect: Some(client_disconnect),
        pfnClientKill: Some(client_kill),
        pfnClientPutInServer: Some(client_put_in_server),
        pfnClientCommand: Some(client_command),
        pfnClientUserInfoChanged: Some(client_userinfo_changed),
        pfnServerActivate: Some(server_activate),
        pfnServerDeactivate: Some(server_deactivate),
        pfnPlayerPreThink: Some(player_pre_think),
        pfnPlayerPostThink: Some(player_post_think),
        pfnStartFrame: Some(start_frame),
        pfnParmsNewLevel: Some(noop),
        pfnParmsChangeLevel: Some(noop),
        pfnGetGameDescription: Some(get_game_description),
        pfnPlayerCustomization: Some(player_customization),
        pfnSpectatorConnect: Some(noop_e),
        pfnSpectatorDisconnect: Some(noop_e),
        pfnSpectatorThink: Some(noop_e),
        pfnSys_Error: Some(sys_error),
        pfnPM_Move: Some(pm_move),
        pfnPM_Init: Some(pm_init),
        pfnPM_FindTextureType: Some(pm_find_texture_type),
        pfnSetupVisibility: Some(setup_visibility),
        pfnUpdateClientData: Some(net::update_client_data),
        pfnAddToFullPack: Some(net::add_to_full_pack),
        pfnCreateBaseline: Some(net::create_baseline),
        pfnRegisterEncoders: Some(noop),
        pfnGetWeaponData: Some(get_weapon_data),
        pfnCmdStart: Some(cmd_start),
        pfnCmdEnd: Some(cmd_end),
        pfnConnectionlessPacket: Some(connectionless_packet),
        pfnGetHullBounds: Some(get_hull_bounds),
        pfnCreateInstancedBaselines: Some(noop),
        pfnInconsistentFile: Some(inconsistent_file),
        pfnAllowLagCompensation: Some(allow_lag_compensation),
    }
}

/// Precache everything; called from worldspawn's spawn (the first entity of every map).
pub(crate) fn precache_all() {
    engine().precache_model(player::PLAYER_MODEL);
    stunt::precache();
    weapons::precache();
    rules::precache();
}

unsafe extern "C" fn noop() {}
unsafe extern "C" fn noop_e(_: *mut edict_t) {}

unsafe extern "C" fn game_init() {
    messages::register_all();
    rules::game_init();
    weapons::game_init();
    stunt::game_init();
    bots::game_init();
    engine().print("[ts] GameInit: Rust TS server library loaded\n");
}

unsafe extern "C" fn spawn(e: *mut edict_t) -> c_int {
    world::dispatch(e, |ent, cx| ent.spawn(cx));
    // The class may have marked itself for removal (FL_KILLME) during spawn.
    if unsafe { (*e).v.flags } & FL_KILLME as i32 != 0 { -1 } else { 0 }
}

unsafe extern "C" fn think(e: *mut edict_t) {
    world::dispatch(e, |ent, cx| ent.think(cx));
}

unsafe extern "C" fn touch(e: *mut edict_t, other: *mut edict_t) {
    world::dispatch(e, |ent, cx| ent.touch(cx, other));
}

unsafe extern "C" fn use_(e: *mut edict_t, other: *mut edict_t) {
    world::dispatch(e, |ent, cx| ent.use_(cx, other, other, UseType::Toggle, 0.0));
}

unsafe extern "C" fn blocked(e: *mut edict_t, other: *mut edict_t) {
    world::dispatch(e, |ent, cx| ent.blocked(cx, other));
}

/// The class handler runs first; unhandled keys fall back to the generic entvars parser. The
/// engine doesn't parse any keyvalues itself and requires `classname` to be handled.
unsafe extern "C" fn key_value(e: *mut edict_t, kvd: *mut KeyValueData) {
    let kvd = unsafe { &mut *kvd };
    let (key, value) = unsafe { (cstr(kvd.szKeyName), cstr(kvd.szValue)) };
    if world::dispatch(e, |ent, cx| ent.key_value(cx, key, value)) == Some(true) {
        kvd.fHandled = 1;
        return;
    }
    kvd.fHandled = entities::entvars_key_value(unsafe { &mut (*e).v }, key, value) as c_int;
}

// Multiplayer only: save/restore are stubs.
unsafe extern "C" fn save(_: *mut edict_t, _: *mut SAVERESTOREDATA) {}
unsafe extern "C" fn restore(_: *mut edict_t, _: *mut SAVERESTOREDATA, _: c_int) -> c_int { 0 }
unsafe extern "C" fn save_fields(_: *mut SAVERESTOREDATA, _: *const c_char, _: *mut c_void, _: *mut TYPEDESCRIPTION, _: c_int) {}
unsafe extern "C" fn save_global_state(_: *mut SAVERESTOREDATA) {}

/// Same as the SDK: grow brush model boxes by one unit so touching works on the edges.
unsafe extern "C" fn set_abs_box(e: *mut edict_t) {
    let v = unsafe { &mut (*e).v };
    for i in 0..3 {
        v.absmin[i] = v.origin[i] + v.mins[i] - 1.0;
        v.absmax[i] = v.origin[i] + v.maxs[i] + 1.0;
    }
}

unsafe extern "C" fn client_connect(e: *mut edict_t, name: *const c_char, addr: *const c_char, _reject: *mut c_char) -> qboolean {
    let (name, addr) = unsafe { (cstr(name), cstr(addr)) };
    rules::client_connect(e, name, addr) as qboolean
}

unsafe extern "C" fn client_disconnect(e: *mut edict_t) {
    rules::client_disconnect(e);
}

unsafe extern "C" fn client_kill(e: *mut edict_t) {
    rules::client_kill(e);
}

unsafe extern "C" fn client_put_in_server(e: *mut edict_t) {
    world::create::<player::Player>(unsafe { &mut (*e).v });
    world::dispatch(e, |ent, cx| ent.spawn(cx));
}

unsafe extern "C" fn client_command(e: *mut edict_t) {
    commands::client_command(e);
}

unsafe extern "C" fn client_userinfo_changed(e: *mut edict_t, _: *mut c_char) {
    rules::userinfo_changed(e);
}

unsafe extern "C" fn server_activate(_: *mut edict_t, edict_count: c_int, client_max: c_int) {
    rules::server_activate();
    engine().print(&format!("[ts] ServerActivate: {edict_count} edicts, {client_max} max clients\n"));
}

unsafe extern "C" fn server_deactivate() {
    rules::server_deactivate();
}

unsafe extern "C" fn player_pre_think(e: *mut edict_t) {
    player::with_player(e, |p, cx| p.pre_think(cx));
}

unsafe extern "C" fn player_post_think(e: *mut edict_t) {
    player::with_player(e, |p, cx| p.post_think(cx));
}

unsafe extern "C" fn start_frame() {
    rules::start_frame();
    bots::start_frame();
}

unsafe extern "C" fn get_game_description() -> *const c_char {
    rules::game_description().as_ptr()
}

unsafe extern "C" fn player_customization(_: *mut edict_t, _: *mut customization_t) {}

unsafe extern "C" fn sys_error(msg: *const c_char) {
    eprintln!("[ts] Sys_Error: {}", unsafe { cstr(msg) }.to_string_lossy());
}

unsafe extern "C" fn pm_move(pm: *mut playermove_t, _server: qboolean) {
    shared::pm::player_move(unsafe { &mut *pm });
}

unsafe extern "C" fn pm_init(pm: *mut playermove_t) {
    shared::pm::init(unsafe { &mut *pm });
}

unsafe extern "C" fn pm_find_texture_type(name: *mut c_char) -> c_char {
    shared::pm::find_texture_type(unsafe { cstr(name) }) as c_char
}

/// The view origin's PVS/PAS decides which entities this client may see and hear.
unsafe extern "C" fn setup_visibility(view: *mut edict_t, client: *mut edict_t, pvs: *mut *mut c_uchar, pas: *mut *mut c_uchar) {
    let v = unsafe { &(*if view.is_null() { client } else { view }).v };
    let org = [v.origin[0] + v.view_ofs[0], v.origin[1] + v.view_ofs[1], v.origin[2] + v.view_ofs[2]];
    unsafe {
        *pvs = goldsrc::eng!(pfnSetFatPVS(org.as_ptr()));
        *pas = goldsrc::eng!(pfnSetFatPAS(org.as_ptr()));
    }
}

unsafe extern "C" fn get_weapon_data(e: *mut edict_t, info: *mut weapon_data_t) -> c_int {
    let info = unsafe { &mut *info.cast::<[weapon_data_t; 32]>() };
    player::with_player(e, |p, cx| weapons::get_weapon_data(p, cx, info)).unwrap_or_else(|| {
        *info = unsafe { std::mem::zeroed() };
        1
    })
}

unsafe extern "C" fn cmd_start(e: *const edict_t, cmd: *const usercmd_t, seed: c_uint) {
    let cmd = unsafe { &*cmd };
    player::with_player(e.cast_mut(), |p, cx| weapons::cmd_start(p, cx, cmd, seed));
}

unsafe extern "C" fn cmd_end(_: *const edict_t) {}

unsafe extern "C" fn connectionless_packet(_: *const netadr_s, _: *const c_char, _: *mut c_char, size: *mut c_int) -> c_int {
    unsafe { *size = 0 };
    0
}

unsafe extern "C" fn get_hull_bounds(hull: c_int, mins: *mut f32, maxs: *mut f32) -> c_int {
    match shared::pm::hull_bounds(hull) {
        Some((lo, hi)) => {
            unsafe {
                mins.cast::<[f32; 3]>().write(lo);
                maxs.cast::<[f32; 3]>().write(hi);
            }
            1
        }
        None => 0,
    }
}

unsafe extern "C" fn inconsistent_file(_: *const edict_t, _: *const c_char, _: *mut c_char) -> c_int { 0 }
unsafe extern "C" fn allow_lag_compensation() -> c_int { 0 }

unsafe extern "C" fn on_free_private_data(e: *mut edict_t) {
    world::free(e);
}

unsafe extern "C" fn game_shutdown() {}

unsafe extern "C" fn should_collide(touched: *mut edict_t, other: *mut edict_t) -> c_int {
    world::dispatch(touched, |ent, cx| ent.should_collide(cx, other)).unwrap_or(true) as c_int
}
