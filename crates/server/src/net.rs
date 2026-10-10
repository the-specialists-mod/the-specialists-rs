//! What goes over the wire: entity states, baselines, and the local player's clientdata.
//! TS versions: `AddToFullPack` (0xaf180), `CreateBaseline` (0xaf81c, stock),
//! `UpdateClientData` (0xb0294); decomp server/architecture.md §7. Owner: rules work package.

use goldsrc::edict::EdictPtr;
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::ffi::c_int;

/// `pfnAddToFullPack`: decide whether `ent` is sent to `host` this frame, and fill `state`.
/// `pset` is the engine's PVS buffer, passed back to it untouched.
pub fn add_to_full_pack(
    state: &mut entity_state_t,
    e: c_int,
    ent: *mut edict_t,
    host: *mut edict_t,
    hostflags: c_int,
    player: c_int,
    pset: *mut u8,
) -> c_int {
    let v = ent.v();

    if v.effects & EF_NODRAW as i32 != 0 && ent != host {
        return 0;
    }
    // Spectators aren't sent to other players.
    if v.flags & FL_SPECTATOR as i32 != 0 && ent != host {
        return 0;
    }
    if v.modelindex == 0 || engine().string(v.model).is_empty() {
        return 0;
    }
    // The engine already culled by PVS for players; check everything else here.
    if ent != host && !engine().check_visibility(ent, pset) {
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
    // Non-moving animated entities are interpolated by the client (SDK: FL_FLY -> EFLAG_SLERP).
    if v.flags & FL_FLY as i32 != 0 {
        state.eflags |= 1;
    }
    // TS: the stunt/state bits go out for every entity (delta.lst has iuser4 on both).
    state.iuser4 = v.iuser4;
    // TS: the viewer's own dropped guns and thrown knives get a grey glow shell.
    if !host.is_null() && v.owner == host {
        let cls = engine().string(v.classname).to_bytes();
        if cls == b"WorldGun" || cls == b"knife" {
            state.rendermode = 0;
            state.renderamt = 2;
            state.renderfx = kRenderFxGlowShell as i32;
            state.rendercolor = color24 { r: 100, g: 100, b: 100 };
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
        state.velocity = v.velocity; // TS copies it; delta.lst doesn't send it for players
    } else {
        state.playerclass = v.playerclass; // "decalable" glass hack (stock)
    }
    1
}

/// `pfnCreateBaseline`: the reference state that deltas are encoded against.
pub fn create_baseline(
    player: c_int,
    eindex: c_int,
    b: &mut entity_state_t,
    entity: *mut edict_t,
    playermodelindex: c_int,
    player_mins: [f32; 3],
    player_maxs: [f32; 3],
) {
    let v = entity.v();
    b.origin = v.origin;
    b.angles = v.angles;
    b.frame = v.frame;
    b.skin = v.skin as i16;
    b.rendermode = v.rendermode as u8 as i32;
    b.renderamt = v.renderamt as u8 as i32;
    b.rendercolor = color24 { r: v.rendercolor[0] as u8, g: v.rendercolor[1] as u8, b: v.rendercolor[2] as u8 };
    b.renderfx = v.renderfx as u8 as i32;

    if player != 0 {
        b.mins = player_mins;
        b.maxs = player_maxs;
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
pub fn update_client_data(ent: *const edict_t, cd: &mut clientdata_t) {
    if ent.is_null() || !ent.has_private_data() {
        return;
    }
    let v = ent.v();
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

    let info = engine().physics_info(ent).to_bytes();
    let n = info.len().min(cd.physinfo.len() - 1);
    for (d, s) in cd.physinfo.iter_mut().zip(&info[..n]) {
        *d = *s as _;
    }
    cd.physinfo[n] = 0;

    // Subsystem fields (stunt bits, time scale, weapon state, ...).
    crate::player::with_player(ent.cast_mut(), |p, cx| p.fill_client_data(cx, cd));
}
