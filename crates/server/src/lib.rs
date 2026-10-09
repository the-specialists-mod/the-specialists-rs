//! MyMod server library (the "game DLL").

mod entities;
mod net;
mod world;

use entities::Player;
use goldsrc::server::{self as sv, engine};
use goldsrc::sys::*;
use goldsrc::util::{cstr, parse_vec3};
use goldsrc::{Global, eng};
use std::cell::Cell;
use std::ffi::{c_char, c_int, c_uchar, c_uint, c_void};

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GiveFnptrsToDll(funcs: *const enginefuncs_t, globals: *mut globalvars_t) {
    unsafe { sv::init(funcs, globals) };
    goldsrc::panic::install("mymod_server_panic.log", |m| engine().print(m));
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
        pfnUse: Some(noop_ee),
        pfnTouch: Some(noop_ee),
        pfnBlocked: Some(noop_ee),
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
        pfnClientDisconnect: Some(noop_e),
        pfnClientKill: Some(noop_e),
        pfnClientPutInServer: Some(client_put_in_server),
        pfnClientCommand: Some(client_command),
        pfnClientUserInfoChanged: Some(client_userinfo_changed),
        pfnServerActivate: Some(server_activate),
        pfnServerDeactivate: Some(noop),
        pfnPlayerPreThink: Some(noop_e),
        pfnPlayerPostThink: Some(player_post_think),
        pfnStartFrame: Some(noop),
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

static MSG_HEALTH: Global<Cell<c_int>> = Global(Cell::new(0));

unsafe extern "C" fn noop() {}
unsafe extern "C" fn noop_e(_: *mut edict_t) {}
unsafe extern "C" fn noop_ee(_: *mut edict_t, _: *mut edict_t) {}

unsafe extern "C" fn game_init() {
    let id = eng!(pfnRegUserMsg(shared::protocol::MSG_HEALTH.as_ptr(), shared::protocol::MSG_HEALTH_SIZE));
    MSG_HEALTH.0.set(id);
    engine().print("[mymod] GameInit: Rust server library loaded\n");
}

unsafe extern "C" fn spawn(e: *mut edict_t) -> c_int {
    world::dispatch(e, |ent, cx| ent.spawn(cx));
    0
}

unsafe extern "C" fn think(e: *mut edict_t) {
    world::dispatch(e, |ent, cx| ent.think(cx));
}

/// Keyvalues from the BSP entity lump. The engine handles none of these itself, not even
/// `classname`, so the generic entvars fields are parsed here.
unsafe extern "C" fn key_value(e: *mut edict_t, kvd: *mut KeyValueData) {
    let kvd = unsafe { &mut *kvd };
    let (key, value) = unsafe { (cstr(kvd.szKeyName), cstr(kvd.szValue)) };
    if world::dispatch(e, |ent, cx| ent.key_value(cx, key, value)) == Some(true) {
        kvd.fHandled = 1;
        return;
    }
    let eng = engine();
    let v = unsafe { &mut (*e).v };
    let text = value.to_str().unwrap_or("");
    let handled = match key.to_bytes() {
        b"classname" => { v.classname = eng.alloc_string(value); true }
        b"model" => { v.model = eng.alloc_string(value); true }
        b"targetname" => { v.targetname = eng.alloc_string(value); true }
        b"target" => { v.target = eng.alloc_string(value); true }
        b"origin" => { v.origin = parse_vec3(text); true }
        b"angles" => { v.angles = parse_vec3(text); true }
        b"rendercolor" => { v.rendercolor = parse_vec3(text); true }
        b"rendermode" => { v.rendermode = text.parse().unwrap_or(0); true }
        b"renderfx" => { v.renderfx = text.parse().unwrap_or(0); true }
        b"renderamt" => { v.renderamt = text.parse().unwrap_or(0.0); true }
        b"skin" => { v.skin = text.parse().unwrap_or(0); true }
        b"spawnflags" => { v.spawnflags = text.parse().unwrap_or(0); true }
        _ => false,
    };
    kvd.fHandled = handled as c_int;
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

unsafe extern "C" fn client_connect(_: *mut edict_t, name: *const c_char, addr: *const c_char, _reject: *mut c_char) -> qboolean {
    let (name, addr) = unsafe { (cstr(name), cstr(addr)) };
    engine().print(&format!("[mymod] {} connecting from {}\n", name.to_string_lossy(), addr.to_string_lossy()));
    1
}

unsafe extern "C" fn client_put_in_server(e: *mut edict_t) {
    world::create::<Player>(unsafe { &mut (*e).v });
    world::dispatch(e, |ent, cx| ent.spawn(cx));
}

unsafe extern "C" fn client_command(e: *mut edict_t) {
    let eng = engine();
    match eng.cmd_argv(0).to_bytes() {
        b"hello" => {
            let hp = unsafe { (*e).v.health };
            eng.client_print(e, PRINT_TYPE::print_console, &format!("Hello from the Rust server! Your health is {hp}.\n"));
        }
        _ => {
            let cmd = eng.cmd_argv(0).to_string_lossy();
            eng.client_print(e, PRINT_TYPE::print_console, &format!("Unknown command: {cmd}\n"));
        }
    }
}

unsafe extern "C" fn client_userinfo_changed(_: *mut edict_t, _: *mut c_char) {}

unsafe extern "C" fn server_activate(_: *mut edict_t, edict_count: c_int, client_max: c_int) {
    engine().print(&format!("[mymod] ServerActivate: {edict_count} edicts, {client_max} max clients\n"));
}

unsafe extern "C" fn player_post_think(e: *mut edict_t) {
    let hp = unsafe { (*e).v.health }.clamp(0.0, 255.0) as u8;
    let resend = world::with(e, |p: &mut Player| {
        let changed = p.sent_health != Some(hp);
        p.sent_health = Some(hp);
        changed
    });
    if resend == Some(true) {
        eng!(pfnMessageBegin(MSG_ONE as c_int, MSG_HEALTH.0.get(), std::ptr::null(), e));
        eng!(pfnWriteByte(hp as c_int));
        eng!(pfnMessageEnd());
    }
}

unsafe extern "C" fn get_game_description() -> *const c_char {
    shared::GAME_NAME.as_ptr()
}

unsafe extern "C" fn player_customization(_: *mut edict_t, _: *mut customization_t) {}

unsafe extern "C" fn sys_error(msg: *const c_char) {
    eprintln!("[mymod] Sys_Error: {}", unsafe { cstr(msg) }.to_string_lossy());
}

unsafe extern "C" fn pm_move(pm: *mut playermove_t, _server: qboolean) {
    shared::pm::player_move(unsafe { &mut *pm });
}

unsafe extern "C" fn pm_init(_: *mut playermove_t) {}

unsafe extern "C" fn pm_find_texture_type(_: *mut c_char) -> c_char {
    b'C' as c_char // CHAR_TEX_CONCRETE
}

/// The view origin's PVS/PAS decides which entities this client may see and hear.
unsafe extern "C" fn setup_visibility(view: *mut edict_t, client: *mut edict_t, pvs: *mut *mut c_uchar, pas: *mut *mut c_uchar) {
    let v = unsafe { &(*if view.is_null() { client } else { view }).v };
    let org = [v.origin[0] + v.view_ofs[0], v.origin[1] + v.view_ofs[1], v.origin[2] + v.view_ofs[2]];
    unsafe {
        *pvs = eng!(pfnSetFatPVS(org.as_ptr()));
        *pas = eng!(pfnSetFatPAS(org.as_ptr()));
    }
}

unsafe extern "C" fn get_weapon_data(_: *mut edict_t, info: *mut weapon_data_t) -> c_int {
    unsafe { std::ptr::write_bytes(info, 0, 32) };
    1
}

unsafe extern "C" fn cmd_start(_: *const edict_t, _: *const usercmd_t, _: c_uint) {}
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

unsafe extern "C" fn should_collide(_: *mut edict_t, _: *mut edict_t) -> c_int { 1 }

