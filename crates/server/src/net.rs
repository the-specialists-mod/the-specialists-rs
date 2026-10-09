//! What goes over the wire: entity states, baselines, and the local player's clientdata.

use goldsrc::eng;
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::ffi::c_int;

/// `pfnAddToFullPack`: decide whether `ent` is sent to `host` this frame, and fill `state`.
pub unsafe extern "C" fn add_to_full_pack(
    state: *mut entity_state_t,
    e: c_int,
    ent: *mut edict_t,
    host: *mut edict_t,
    hostflags: c_int,
    player: c_int,
    pset: *mut u8,
) -> c_int {
    let (state, v) = unsafe { (&mut *state, &(*ent).v) };

    if v.effects & EF_NODRAW as i32 != 0 && ent != host {
        return 0;
    }
    if v.modelindex == 0 || engine().string(v.model).is_empty() {
        return 0;
    }
    // The engine already culled by PVS for players; check everything else here.
    if ent != host && eng!(pfnCheckVisibility(ent, pset)) == 0 {
        return 0;
    }
    // Don't send to the local client an entity it predicts itself.
    if v.flags & FL_SKIPLOCALHOST as i32 != 0 && hostflags & 1 != 0 && v.owner == host {
        return 0;
    }

    *state = entity_state_t::default();
    state.number = e;
    state.entityType = if v.flags & FL_CUSTOMENTITY as i32 != 0 { ENTITY_BEAM } else { ENTITY_NORMAL } as i32;
    state.animtime = ((1000.0 * v.animtime) as i32) as f32 / 1000.0; // quantized like the SDK
    state.origin = v.origin;
    state.angles = v.angles;
    state.mins = v.mins;
    state.maxs = v.maxs;
    state.startpos = v.startpos;
    state.endpos = v.endpos;
    state.modelindex = v.modelindex;
    state.frame = v.frame;
    state.skin = v.skin as i16;
    state.effects = v.effects;
    state.scale = v.scale;
    state.solid = v.solid as i16;
    state.colormap = v.colormap;
    state.movetype = v.movetype;
    state.sequence = v.sequence;
    state.framerate = v.framerate;
    state.body = v.body;
    state.controller = v.controller;
    state.blending[..2].copy_from_slice(&v.blending);
    state.rendermode = v.rendermode;
    state.renderamt = v.renderamt as i32;
    state.renderfx = v.renderfx;
    state.rendercolor = color24 { r: v.rendercolor[0] as u8, g: v.rendercolor[1] as u8, b: v.rendercolor[2] as u8 };
    if !v.aiment.is_null() {
        state.aiment = engine().index_of(v.aiment);
    }
    if !v.owner.is_null() {
        let owner = engine().index_of(v.owner);
        if owner >= 1 && owner <= engine().max_clients() {
            state.owner = owner; // only players matter to the client
        }
    }

    if player != 0 {
        state.basevelocity = v.basevelocity;
        state.weaponmodel = 0;
        state.gaitsequence = v.gaitsequence;
        state.spectator = (v.flags & FL_SPECTATOR as i32 != 0) as i32;
        state.friction = v.friction;
        state.gravity = v.gravity;
        state.usehull = (v.flags & FL_DUCKING as i32 != 0) as i32;
        state.health = v.health as i32;
    }
    1
}

/// `pfnCreateBaseline`: the reference state that deltas are encoded against.
pub unsafe extern "C" fn create_baseline(
    player: c_int,
    eindex: c_int,
    baseline: *mut entity_state_t,
    entity: *mut edict_t,
    playermodelindex: c_int,
    player_mins: *mut f32,
    player_maxs: *mut f32,
) {
    let (b, v) = unsafe { (&mut *baseline, &(*entity).v) };
    b.origin = v.origin;
    b.angles = v.angles;
    b.frame = v.frame;
    b.skin = v.skin as i16;
    b.rendermode = v.rendermode as u8 as i32;
    b.renderamt = v.renderamt as u8 as i32;
    b.rendercolor = color24 { r: v.rendercolor[0] as u8, g: v.rendercolor[1] as u8, b: v.rendercolor[2] as u8 };
    b.renderfx = v.renderfx as u8 as i32;

    if player != 0 {
        unsafe {
            b.mins = *player_mins.cast::<[f32; 3]>();
            b.maxs = *player_maxs.cast::<[f32; 3]>();
        }
        b.colormap = eindex;
        b.modelindex = playermodelindex;
        b.friction = 1.0;
        b.movetype = MOVETYPE_WALK as i32;
        b.scale = v.scale;
        b.solid = SOLID_SLIDEBOX as i16;
        b.framerate = 1.0;
        b.gravity = 1.0;
    } else {
        b.mins = v.mins;
        b.maxs = v.maxs;
        b.colormap = 0;
        b.modelindex = v.modelindex;
        b.movetype = v.movetype;
        b.scale = v.scale;
        b.solid = v.solid as i16;
        b.framerate = v.framerate;
        b.gravity = v.gravity;
    }
}

/// `pfnUpdateClientData`: the local player's state, which client prediction starts from.
pub unsafe extern "C" fn update_client_data(ent: *const edict_t, _sendweapons: c_int, cd: *mut clientdata_t) {
    if ent.is_null() || unsafe { (*ent).pvPrivateData.is_null() } {
        return;
    }
    let (cd, v) = unsafe { (&mut *cd, &(*ent).v) };
    cd.flags = v.flags;
    cd.health = v.health;
    cd.viewmodel = 0;
    cd.waterlevel = v.waterlevel;
    cd.watertype = v.watertype;
    cd.weapons = v.weapons;
    cd.origin = v.origin;
    cd.velocity = v.velocity;
    cd.view_ofs = v.view_ofs;
    cd.punchangle = v.punchangle;
    cd.bInDuck = v.bInDuck;
    cd.flTimeStepSound = v.flTimeStepSound;
    cd.flDuckTime = v.flDuckTime as _;
    cd.flSwimTime = v.flSwimTime as _;
    cd.waterjumptime = v.teleport_time as i32;
    cd.maxspeed = v.maxspeed;
    cd.fov = v.fov;
    cd.weaponanim = v.weaponanim;
    cd.pushmsec = v.pushmsec;
    cd.deadflag = v.deadflag;
    cd.iuser1 = v.iuser1;
    cd.iuser2 = v.iuser2;

    let info = eng!(pfnGetPhysicsInfoString(ent));
    let info = unsafe { goldsrc::util::cstr(info) }.to_bytes();
    let n = info.len().min(cd.physinfo.len() - 1);
    for (d, s) in cd.physinfo.iter_mut().zip(&info[..n]) {
        *d = *s as _;
    }
    cd.physinfo[n] = 0;
}
