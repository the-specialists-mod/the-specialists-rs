//! What goes over the wire: entity states, baselines, and the local player's clientdata.
//! TS versions: `AddToFullPack` (0xaf180), `CreateBaseline` (0xaf81c, stock),
//! `UpdateClientData` (0xb0294); decomp server/architecture.md §7. Owner: rules work package.

use goldsrc::edict::EdictPtr;
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::ffi::c_int;
use std::sync::atomic::{AtomicI32, Ordering::Relaxed};

/// `g_groupmask` / `g_groupop`: the game's copy of the engine group-trace filter.
static GROUP_MASK: AtomicI32 = AtomicI32::new(0);
static GROUP_OP: AtomicI32 = AtomicI32::new(0);

/// `UTIL_SetGroupTrace` (0xebecc): filter traces by `groupinfo` (`op` 0 = AND, 1 = NAND).
pub fn set_group_trace(mask: i32, op: i32) {
    GROUP_MASK.store(mask, Relaxed);
    GROUP_OP.store(op, Relaxed);
    engine().set_group_mask(mask, op);
}

/// `UTIL_UnsetGroupTrace` (0xebf0c).
pub fn unset_group_trace() {
    GROUP_MASK.store(0, Relaxed);
    GROUP_OP.store(0, Relaxed);
    engine().set_group_mask(0, 0);
}

/// The group-trace half of `CmdStart` (0xb067c): the player's usercmd runs with its `groupinfo`
/// as the trace filter. Nothing in TS sets `groupinfo`, so this only matters for content that does.
pub fn cmd_start(player: *const edict_t) {
    if !player.is_null() && player.has_private_data() {
        let group = player.v().groupinfo;
        if group != 0 {
            set_group_trace(group, 0);
        }
    }
}

/// `CmdEnd` (0xb06f4): clears the filter `cmd_start` set.
pub fn cmd_end(player: *const edict_t) {
    if !player.is_null() && player.has_private_data() && player.v().groupinfo != 0 {
        unset_group_trace();
    }
}

/// `AddToFullPack` (0xaf180): decide whether `ent` is sent to `host` this frame, and fill `state`.
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
    // Read before borrowing `ent`'s entvars: `host` can be `ent`.
    let host_group = if host.is_null() { 0 } else { host.v().groupinfo };
    let v = ent.v();

    // TS hides only `effects == EF_NODRAW` exactly: EF_NODRAW combined with another bit is sent.
    if v.effects == EF_NODRAW as i32 && ent != host {
        return 0;
    }
    // TS's only model tests are `modelindex == 0` and `pStringBase + pev->model == NULL`, which
    // can't fail, so an entity with a model index but an empty `model` string is still sent.
    if v.modelindex == 0 {
        return 0;
    }
    // Spectators aren't sent to other players.
    if v.flags & FL_SPECTATOR as i32 != 0 && ent != host {
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
    if host_group != 0 {
        set_group_trace(host_group, 0);
        if v.groupinfo != 0 {
            let op = GROUP_OP.load(Relaxed);
            if (op == 0 && v.groupinfo & host_group == 0) || (op == 1 && v.groupinfo & host_group != 0) {
                // TS returns here without `UTIL_UnsetGroupTrace`, leaving the filter on until
                // the next unset.
                return 0;
            }
        }
        unset_group_trace();
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
    state.impacttime = v.impacttime;
    state.starttime = v.starttime;
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
    // TS converts with x87 truncation (the decomp's ROUND is `fistp` under a truncating control
    // word), and the colour through a short, so out-of-range channels wrap instead of clamping.
    state.renderamt = v.renderamt as i32;
    state.renderfx = v.renderfx;
    let channel = |c: f32| c as i16 as u8;
    state.rendercolor = color24 { r: channel(v.rendercolor[0]), g: channel(v.rendercolor[1]), b: channel(v.rendercolor[2]) };
    if !v.aiment.is_null() {
        state.aiment = engine().index_of(v.aiment);
    }
    if !v.owner.is_null() {
        let owner = engine().index_of(v.owner);
        if owner >= 1 && owner <= engine().max_clients() {
            state.owner = owner; // only players matter to the client
        }
    }
    // EFLAG_SLERP: TS keeps the older SDK test (an animating non-player standing still), not
    // the later SDK's FL_FLY.
    if player == 0 && v.animtime != 0.0 && v.velocity == [0.0; 3] {
        state.eflags |= 1;
    }
    // TS: the stunt/state bits go out for every entity (delta.lst has iuser4 on both).
    state.iuser4 = v.iuser4;
    // TS: the viewer's own dropped guns and thrown knives get a grey glow shell. TS reads the
    // private dropper (`WorldGun` +0xd0) and thrower (`knife` +0x8c), not `pev->owner`.
    if !host.is_null() && crate::weapons::glow_for_host(ent, host) {
        state.rendermode = 0;
        state.renderamt = 2;
        state.renderfx = kRenderFxGlowShell as i32;
        state.rendercolor = color24 { r: 100, g: 100, b: 100 };
    }

    if player != 0 {
        state.basevelocity = v.basevelocity;
        // The p_ model the client merges onto the player (`StudioDrawPlayer`).
        state.weaponmodel = engine().model_index(engine().string(v.weaponmodel));
        state.gaitsequence = v.gaitsequence;
        // TS stores `flags & FL_SPECTATOR` (0x4000000) into the 1-bit `spectator` field; every
        // engine clamps an out-of-range unsigned delta integer to its maximum (FWGS
        // `Delta_ClampIntegerField`, HLDS/ReHLDS `MSG_WriteBits`), so the wire carries 1 either
        // way. 0/1 is the same delta and avoids FWGS's debug-build overflow warning.
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
    // `(int)teleport_time`: the decomp's `ROUND` is GCC's truncating cast (0xb0396).
    cd.waterjumptime = crate::rules::x87_trunc(v.teleport_time as f64);
    cd.maxspeed = v.maxspeed;
    cd.fov = v.fov;
    cd.weaponanim = v.weaponanim;
    cd.pushmsec = v.pushmsec;
    // TS never writes `deadflag`: the engine's zeroed clientdata keeps 0.
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
