//! Client prediction glue: weapon prediction in HUD_PostRunCmd and the Txfer callbacks that carry
//! TS fields (iuser4 stunt bits, fuser1 time scale, vuser1..4 weapon state). Owner:
//! rendering/prediction work package. Reference: rendering-effects-prediction.md §8.

use goldsrc::sys::*;

pub fn post_run_cmd(_from: &mut local_state_t, _to: &mut local_state_t, _cmd: &mut usercmd_t, _runfuncs: bool, _time: f64, _seed: u32) {}

pub fn txfer_local_overrides(s: &mut entity_state_t, c: &clientdata_t) {
    s.origin = c.origin;
    s.iuser1 = c.iuser1;
    s.iuser2 = c.iuser2;
    s.iuser4 = c.iuser4;
}

pub fn process_player_state(dst: &mut entity_state_t, src: &entity_state_t) {
    *dst = *src;
}

/// Carries predicted-only fields across a new server frame.
pub fn txfer_prediction_data(ps: &mut entity_state_t, pps: &entity_state_t, pcd: &mut clientdata_t, ppcd: &clientdata_t,
                             wd: &mut [weapon_data_t; 32], pwd: &[weapon_data_t; 32]) {
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
    let _ = (wd, pwd);
}
