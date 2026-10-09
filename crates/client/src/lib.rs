//! MyMod client library. Exports the individual `HUD_*`/`IN_*`/... names, which both
//! Steam GoldSrc and FWGS accept.

mod input;

use goldsrc::Global;
use goldsrc::client::{self as cl, engine};
use goldsrc::msg::MsgReader;
use goldsrc::sys::*;
use std::cell::Cell;
use std::ffi::{c_char, c_int, c_void};

static HEALTH: Global<Cell<i32>> = Global(Cell::new(-1));

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Initialize(funcs: *mut cl_enginefunc_t, version: c_int) -> c_int {
    if funcs.is_null() || version != CLDLL_INTERFACE_VERSION as c_int {
        return 0;
    }
    unsafe { cl::init(funcs) };
    goldsrc::panic::install("mymod_client_panic.log", |m| engine().print(m));
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn HUD_Init() {
    input::register_commands();
    engine().hook_user_msg(shared::protocol::MSG_HEALTH, Some(msg_health));
    engine().print("[mymod] client: Rust client library initialized\n");
}

unsafe extern "C" fn msg_health(_name: *const c_char, size: c_int, buf: *mut c_void) -> c_int {
    let mut r = unsafe { MsgReader::from_raw(buf, size) };
    let hp = r.byte();
    if !r.bad {
        HEALTH.0.set(hp as i32);
    }
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn HUD_VidInit() -> c_int {
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn HUD_Redraw(_time: f32, intermission: c_int) -> c_int {
    if intermission != 0 {
        return 1;
    }
    let e = engine();
    let (w, h) = e.screen_size();
    e.draw_string(16, h - 48, [1.0, 0.7, 0.1], "MyMod (Rust client)");
    let hp = match HEALTH.0.get() {
        -1 => "--".to_string(),
        n => n.to_string(),
    };
    e.draw_string(16, h - 28, [1.0, 1.0, 1.0], &format!("Health: {hp}"));
    let local = goldsrc::cl_eng!(GetLocalPlayer());
    if !local.is_null() {
        let o = unsafe { (*local).origin };
        e.draw_string(16, h - 68, [0.6, 0.8, 1.0], &format!("pos {:.0} {:.0} {:.0}", o[0], o[1], o[2]));
    }
    // Crosshair.
    let (cx, cy) = (w / 2, h / 2);
    e.fill_rgba(cx - 6, cy, 13, 1, [255, 160, 0, 255]);
    e.fill_rgba(cx, cy - 6, 1, 13, [255, 160, 0, 255]);
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn HUD_UpdateClientData(_cdata: *mut c_void, _time: f32) -> c_int {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn HUD_Reset() {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn HUD_Shutdown() {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn HUD_PlayerMove(pm: *mut playermove_t, _server: c_int) {
    shared::pm::player_move(unsafe { &mut *pm });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn HUD_PlayerMoveInit(_pm: *mut playermove_t) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn HUD_PlayerMoveTexture(_name: *mut c_char) -> c_char {
    b'C' as c_char // CHAR_TEX_CONCRETE
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn HUD_GetHullBounds(hull: c_int, mins: *mut f32, maxs: *mut f32) -> c_int {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn IN_ActivateMouse() {}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn IN_DeactivateMouse() {}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn IN_MouseEvent(_mstate: c_int) {}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn IN_ClearStates() {}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn IN_Accumulate() {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CL_CreateMove(_frametime: f32, cmd: *mut usercmd_t, active: c_int) {
    input::create_move(unsafe { &mut *cmd }, active != 0);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CL_IsThirdPerson() -> c_int {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CL_CameraOffset(ofs: *mut f32) {
    unsafe { ofs.cast::<[f32; 3]>().write([0.0; 3]) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn KB_Find(name: *const c_char) -> *mut c_void {
    input::find(unsafe { goldsrc::util::cstr(name) })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CAM_Think() {}

/// First-person view from the predicted origin. No bob, roll, or view model yet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn V_CalcRefdef(p: *mut ref_params_t) {
    let p = unsafe { &mut *p };
    for i in 0..3 {
        p.vieworg[i] = p.simorg[i] + p.viewheight[i];
        p.viewangles[i] = p.cl_viewangles[i] + p.punchangle[i];
    }
    let (f, r, u) = engine().angle_vectors(p.viewangles);
    (p.forward, p.right, p.up) = (f, r, u);
    p.nextView = 0;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn HUD_AddEntity(_type: c_int, _ent: *mut c_void, _model: *const c_char) -> c_int {
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn HUD_CreateEntities() {}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn HUD_DrawNormalTriangles() {}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn HUD_DrawTransparentTriangles() {}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn HUD_StudioEvent(_event: *const c_void, _ent: *const c_void) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn HUD_PostRunCmd(
    _from: *mut local_state_t,
    _to: *mut local_state_t,
    _cmd: *mut usercmd_t,
    _runfuncs: c_int,
    _time: f64,
    _random_seed: u32,
) {
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn HUD_TxferLocalOverrides(state: *mut entity_state_t, client: *const clientdata_t) {
    let (s, c) = unsafe { (&mut *state, &*client) };
    s.origin = c.origin;
    s.iuser1 = c.iuser1;
    s.iuser2 = c.iuser2;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn HUD_ProcessPlayerState(dst: *mut entity_state_t, src: *const entity_state_t) {
    unsafe { *dst = *src };
}

/// Carries predicted-only fields across a new server frame.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn HUD_TxferPredictionData(
    ps: *mut entity_state_t,
    pps: *const entity_state_t,
    pcd: *mut clientdata_t,
    ppcd: *const clientdata_t,
    _wd: *mut weapon_data_t,
    _pwd: *const weapon_data_t,
) {
    let (ps, pps, pcd, ppcd) = unsafe { (&mut *ps, &*pps, &mut *pcd, &*ppcd) };
    ps.oldbuttons = pps.oldbuttons;
    ps.flFallVelocity = pps.flFallVelocity;
    ps.iStepLeft = pps.iStepLeft;
    ps.playerclass = pps.playerclass;
    pcd.viewmodel = ppcd.viewmodel;
    pcd.m_iId = ppcd.m_iId;
    pcd.m_flNextAttack = ppcd.m_flNextAttack;
    pcd.fov = ppcd.fov;
    pcd.weaponanim = ppcd.weaponanim;
    pcd.tfstate = ppcd.tfstate;
    pcd.maxspeed = ppcd.maxspeed;
    pcd.deadflag = ppcd.deadflag;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Demo_ReadBuffer(_size: c_int, _buf: *mut u8) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn HUD_ConnectionlessPacket(_from: *const c_void, _args: *const c_char, _buf: *mut c_char, size: *mut c_int) -> c_int {
    unsafe { *size = 0 };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn HUD_Frame(_time: f64) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn HUD_Key_Event(_down: c_int, _key: c_int, _binding: *const c_char) -> c_int {
    1 // let the engine handle the key
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn HUD_TempEntUpdate(
    _frametime: f64,
    _client_time: f64,
    _gravity: f64,
    _free: *mut *mut c_void,
    _active: *mut *mut c_void,
    _add_visible: *mut c_void,
    _play_sound: *mut c_void,
) {
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn HUD_GetUserEntity(_index: c_int) -> *mut c_void {
    std::ptr::null_mut()
}
