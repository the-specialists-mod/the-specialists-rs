//! Player lifecycle under the TS rules: connect, put-in-server, InitHUD, spawn (`TSInit`),
//! death (`CBasePlayer::Killed`, `PlayerDeathThink`), observer and respawn, delayed suicide,
//! disconnect, userinfo. Decomp: CBasePlayer.c (Spawn 0x8f..., TSInit 0x92a4c, Killed 0x8ed5c,
//! PreThink 0x908cc, StartObserver 0x99634, StopObserver 0x99a14), global_000ab05c.c (client.cpp),
//! CHalfLifeMultiplay.c (InitHUD 0xcf618), CTSGameRules.c (InitHUD 0x79134).

use super::msg::{self, ts_message_one};
use super::{PlayerRules, alive, cvars, netname, other, powerup, scoring, spawnpoint, state, teams};
use crate::damage::{DamageInfo, hitgroup};
use crate::messages::{self, Msg};
use crate::player::Player;
use crate::world::{Cx, Entity};
use goldsrc::edict::EdictPtr;
use goldsrc::server::{MsgWriter, engine};
use goldsrc::sys::*;
use shared::protocol::hud_wire::TsPAward;
use shared::protocol::hud_wire::{ActItems, SetFov, Spectator, TsFade, TsHealth, TsMBlur, fade};
use shared::protocol::rules::{GameMode, Motd, ScoreInfo, ServerName, SrvSett, hud_dest};
use shared::protocol::rules::{RoundTime, TeamInfo};
use shared::ts::{obs, powerup as pw, stunt};
use std::ffi::CStr;

const SVC_TEMPENTITY: i32 = 23;
const TE_KILLPLAYERATTACHMENTS: i32 = 125;
const EF_NOINTERP: i32 = 32;

/// HLDM models TS refuses (`IsValidTeam(char*)` 0x8b140).
const HLDM_MODELS: [&str; 9] = ["barney", "gina", "gman", "helmet", "hgrunt", "recon", "robo", "scientist", "zombie"];

// ---- connection ----------------------------------------------------------------------------------

/// `ClientConnect`: notify `ts_playeramount`, `CHalfLifeMultiplay::ClientConnected` (0xcf59c:
/// the voice manager), then accept.
pub fn client_connect(e: *mut edict_t, _name: &CStr, _addr: &CStr) -> bool {
    crate::entities::ts::player_amount_changed(true);
    super::voice::client_connected(e);
    true
}

/// `ClientDisconnect` + `CHalfLifeMultiplay::ClientDisconnected` + `TSPlayerDisconnect`.
pub fn client_disconnect(e: *mut edict_t) {
    crate::entities::ts::player_amount_changed(false);
    let game_over = state::with(|g| g.game_over);
    let idx = engine().index_of(e);
    if !game_over {
        let name = netname(e);
        msg::say_all(idx, &format!("- {name} has left the game\n"));
        let v = e.v();
        v.takedamage = DAMAGE_NO as f32;
        v.solid = SOLID_NOT as i32;
        engine().set_origin(e, v.origin);
    }
    crate::entities::fire_targets(c"game_playerleave", e, e, crate::world::UseType::Toggle, 0.0);
    msg::log(&format!("\"{}<{}><{}><>\" disconnected\n", netname(e), engine().user_id(e), engine().auth_id(e).to_string_lossy()));
    crate::player::with_player(e, |p, cx| {
        powerup::deactivate_flow(p, cx);
        crate::weapons::strip_all(p, cx);
        crate::entities::ts::drop_flag(p, cx, false);
        if p.ts.is_the_one {
            state::with(|g| g.the_one_exists = false);
        }
        p.joined = false;
        p.bot = None;
        p.ts = PlayerRules::default();
        p.cash = 0;
        p.frags = 0;
        p.deaths = 0;
        p.team = 0;
    });
    // Nobody keeps slowing this slot.
    for o in super::players() {
        other(o, |q| q.ts.slowed &= !(1u64 << idx.clamp(0, 63)));
    }
    state::with(|g| g.check_deads_at = engine().time() + 1.0);
    // The engines leave FL_CLIENT/FL_FAKECLIENT on a dropped client's edict, and FWGS still maps
    // it to the freed slot: pfnGetPlayerUserId returns the old id, and pfnSetClientMaxspeed then
    // dereferences the slot's NULL edict (the "kick a bot" crash). Clearing the flags takes the
    // edict out of players(), is_player() and the bot list until a client connects to the slot.
    (e.v().flags &= !((FL_CLIENT | FL_FAKECLIENT) as i32));
}

/// `ClientKill`: suicide one second later (handled in `pre_think`), not while observing.
pub fn client_kill(e: *mut edict_t) {
    crate::player::with_player(e, |p, cx| {
        let now = cx.time();
        if cx.pev().iuser1 == 0 && p.ts.suicide_at <= 0.001 {
            p.ts.suicide_at = now + 1.0;
        }
    });
}

/// `ClientUserInfoChanged`: name changes (`%` sanitised, announced, logged) and the HLDM model
/// check (`CHalfLifeMultiplay::ClientUserInfoChanged`).
pub fn userinfo_changed(e: *mut edict_t) {
    if e.is_null() || !e.has_private_data() {
        return;
    }
    let eng = engine();
    let old = netname(e);
    let new = eng.info_key(e, c"name").to_string_lossy().into_owned();
    if !old.is_empty() && old != new {
        let clean = new.replace('%', " ");
        if clean != new {
            eng.set_client_key(e, "name", &clean);
        }
        msg::say_all(eng.index_of(e), &format!("* {old} changed name to {clean}\n"));
        msg::log(&format!("\"{old}<{}><{}><>\" changed name to \"{clean}\"\n", eng.user_id(e), eng.auth_id(e).to_string_lossy()));
    }
    let model = eng.info_key(e, c"model").to_string_lossy().to_ascii_lowercase();
    if HLDM_MODELS.contains(&model.as_str()) {
        let fallback = if state::teamplay() { teams::team_name_of(e) } else { String::new() };
        let fallback = if fallback.is_empty() { "gordon".to_owned() } else { teams::model_for(&fallback) };
        eng.set_client_key(e, "model", &fallback);
        msg::say_one(e, eng.index_of(e), &format!("* Can't change team to '{model}'\n"));
        msg::say_one(e, eng.index_of(e), "* HLDM models not allowed.\n");
    }
}

// ---- spawn -------------------------------------------------------------------------------------

/// `CBasePlayer::Spawn` TS parts (`TSInit`, rules `PlayerSpawn`, spawn spot). Runs after the base
/// player setup in `Player::spawn` and before weapons/stunt `on_spawn`.
pub fn on_spawn(p: &mut Player, cx: &mut Cx) {
    let e = engine();
    let now = cx.time();
    let first = !p.ts.put_in_server;
    let superjump_map = state::with(|g| g.superjump_map);

    // TSInit
    if p.ts.is_the_one {
        state::with(|g| g.the_one_exists = false);
    }
    let was_observer = {
        let v = cx.pev();
        v.iuser1 != 0 || v.iuser2 != 0
    };
    // TSInit resets the per-life state; these survive a respawn.
    let old = std::mem::take(&mut p.ts);
    p.ts = PlayerRules {
        put_in_server: old.put_in_server,
        game_hud_initialized: old.game_hud_initialized,
        obs_mode: old.obs_mode,
        obs_target: old.obs_target,
        tk_until: old.tk_until,
        tk_count: old.tk_count,
        next_say: old.next_say,
        respawns_used: old.respawns_used,
        team_name: old.team_name,
        flag: old.flag,
        ..PlayerRules::default()
    };
    p.slow_target = 1.0;
    p.ts.slow_display = 1.0;
    p.active_powerup = 0;
    p.powerup_until = 0.0;
    p.kungfu = false;
    p.superjump = superjump_map;
    p.respawn_at = 0.0;
    p.slots_used = 0;
    p.ts.protect_until = now + 1.0;
    p.ts.init_hud = true;
    e.set_physics_key(cx.edict, c"slj", c"0");
    e.set_physics_key(cx.edict, c"hl", c"1");
    e.set_physics_key(cx.edict, c"mj", if superjump_map { c"1" } else { c"0" });
    e.set_client_maxspeed(cx.edict, 330.0);
    crate::entities::stock::triggers::geiger_spawn(cx.edict);
    {
        let v = cx.pev();
        v.maxspeed = 330.0;
        v.fuser1 = 1.0;
        v.iuser4 = 0;
        v.renderamt = 125.0;
        v.rendermode = kRenderTransColor as i32;
        v.rendercolor = [0.0; 3];
        v.renderfx = kRenderFxNoDissipation as i32;
        v.sequence = 0;
        v.gaitsequence = 0;
        v.frame = 0.0;
        v.weapons |= 0x8000_0000u32 as i32;
        v.air_finished = now + 12.0;
        v.dmg = 2.0;
        v.velocity = [0.0; 3];
        v.punchangle = [0.0; 3];
    }
    let fadecol = TsFade::new([200, 200, 250], 0x19, 0.2, 1.0, fade::IN);
    msg::one(Msg::TsFade, cx.edict, |w| fadecol.write(w));
    if was_observer {
        let s = Spectator { entindex: cx.index() as u8, observer: 0 };
        msg::all(Msg::Spectator, |w| s.write(w));
        let v = cx.pev();
        v.iuser1 = 0;
        v.iuser2 = 0;
    }

    if first {
        // ClientPutInServer: GiveCash(atoi(initial_cash.string)), EF_NOINTERP.
        p.ts.put_in_server = true;
        give_cash(p, cvars::INITIAL_CASH);
        cx.pev().effects |= EF_NOINTERP;
    }

    if state::teamplay() {
        teams::set_default_team(p, cx);
    }
    spawnpoint::place(p, cx);
}

/// `CBasePlayer::GiveCash`: clamped to 0..30000.
pub fn give_cash(p: &mut Player, n: i32) {
    p.cash = (p.cash + n).clamp(0, 30000);
}

// ---- think --------------------------------------------------------------------------------------

/// The rules part of `CBasePlayer::PreThink` (0x908cc).
pub fn pre_think(p: &mut Player, cx: &mut Cx) {
    let now = cx.time();
    let buttons = cx.pev().button;
    p.ts.buttons_pressed = buttons & (p.ts.buttons_last ^ buttons);
    p.ts.buttons_last = buttons;
    if state::with(|g| g.game_over) {
        return;
    }

    if p.ts.init_hud {
        init_hud(p, cx);
    }

    // CheckSuitUpdate (0xda2d0): the geiger counter (suit sentences never play in multiplayer).
    crate::entities::stock::triggers::update_geiger_counter(cx.edict);

    // Time scale easing toward the target, and animation rate = time scale.
    let ft = engine().globals().frametime;
    {
        let v = cx.pev();
        powerup::interpolate(ft, &mut v.fuser1, p.slow_target);
        v.framerate = v.fuser1.max(0.01);
    }

    powerup::update_state(p, cx);
    powerup::slow_flow(p, cx);
    health_regen(p, cx, now);

    if cx.pev().iuser1 != 0 {
        observer_think(p, cx, now);
        cx.pev().impulse = 0;
        return;
    }
    if cx.pev().deadflag > DEAD_NO as i32 {
        death_think(p, cx, now);
        return;
    }

    // Fall velocity for PostThink.
    if cx.pev().flags & FL_ONGROUND as i32 == 0 {
        p.ts.fall_velocity = -cx.pev().velocity[2];
    }

    // +use: double tap activates the held power-up (`TSUse`), single press uses ground items.
    if p.ts.buttons_pressed & IN_USE as i32 != 0 {
        if now <= p.ts.use_tap_until {
            powerup::use_powerup(p, cx);
        }
        p.ts.use_tap_until = now + 0.3;
        player_use(p, cx);
    }

    // End of spawn protection.
    if p.ts.protect_until < now && cx.pev().rendermode != 0 {
        let v = cx.pev();
        v.rendermode = 0;
        v.rendercolor = [0.0; 3];
        v.renderamt = 0.0;
        v.renderfx = 0;
    }

    // Delayed suicide.
    if p.ts.suicide_at > 0.001 && p.ts.suicide_at < now {
        p.ts.suicide_at = 0.0;
        cx.pev().health = 0.0;
        let me = cx.edict;
        let info = DamageInfo::new(me, me, 0.0, crate::damage::bits::NEVERGIB);
        killed(p, cx, &info);
    }
}

/// Rules part of `PostThink`. Fall damage (player-movement.md §6.6) is applied by the stunt
/// module's PostThink together with the landing logic, so nothing to do here yet.
pub fn post_think(_p: &mut Player, _cx: &mut Cx) {}

/// Health above max decays 1/s; The One regenerates (`PreThink`, `+0x73c`).
fn health_regen(p: &mut Player, cx: &mut Cx, now: f32) {
    if p.ts.health_timer == 0.0 {
        return;
    }
    if p.ts.is_the_one {
        if p.ts.health_timer < now {
            let n = state::with(|g| g.the_one_others) as f32;
            p.ts.health_timer = now + 1.0 / (n * 0.32).max(1.0);
            let v = cx.pev();
            v.health = (v.health + n * 0.16).min(150.0);
        }
        return;
    }
    let v = cx.pev();
    if v.health <= v.max_health {
        p.ts.health_timer = 0.0;
    } else if p.ts.health_timer < now {
        p.ts.health_timer = now + 1.0;
        v.health -= 1.0;
    }
}

/// `UpdateClientData`'s init block: ResetHUD every spawn, InitHUD and the rules InitHUD once.
fn init_hud(p: &mut Player, cx: &mut Cx) {
    p.ts.init_hud = false;
    let e = cx.edict;
    messages::one(Msg::ResetHUD, e).byte(0).end();
    // The client clears its status and weapon elements on ResetHUD: resend everything (SDK:
    // m_iClientHealth = -1 etc. with m_fInitHUD). Without this a value sent before ResetHUD in
    // the spawn frame (health 100) is never sent again and the HUD stays hidden.
    crate::player::hud::reset(p);
    p.inv.sv.reset_net();
    if !p.ts.game_hud_initialized {
        messages::one(Msg::InitHUD, e).end();
        rules_init_hud(p, cx);
        ts_init_hud(p, cx);
        p.ts.game_hud_initialized = true;
        crate::entities::fire_targets(c"game_playerjoin", e, e, crate::world::UseType::Toggle, 0.0);
    }
    crate::entities::fire_targets(c"game_playerspawn", e, e, crate::world::UseType::Toggle, 0.0);
    let (round_end, timer) = state::with(|g| (g.round_end, g.round_timer));
    if round_end != 0.0 && timer {
        let rt = RoundTime { secs: (round_end - cx.time()).max(0.0).round() as u8, state: 0 };
        msg::one(Msg::RoundTime, e, |w| rt.write(w));
    }
    if state::with(|g| g.superjump_map) && p.superjump {
        msg::one(Msg::PwUp, e, |w| shared::protocol::hud_wire::PwUp { bits: pw::SUPERJUMP as i16, secs: 0 }.write(w));
    }
}

/// `CHalfLifeMultiplay::InitHUD` / `CHalfLifeTeamplay::InitHUD`.
fn rules_init_hud(p: &mut Player, cx: &mut Cx) {
    let e = cx.edict;
    let eng = engine();
    let teamplay = state::teamplay();
    if teamplay {
        teams::set_default_team(p, cx);
    }
    let name = netname(e);
    let name = if name.is_empty() { "unconnected".to_owned() } else { name };
    msg::text_all(hud_dest::NOTIFY, &format!("{name} has joined the game\n"));
    msg::log(&format!("\"{name}<{}><{}><>\" entered the game\n", eng.user_id(e), eng.auth_id(e).to_string_lossy()));
    // UpdateGameMode
    let gm = GameMode { teamplay: teamplay as u8 };
    msg::one(Msg::GameMode, e, |w| gm.write(w));
    // Our own (empty) score first, then everyone's.
    let me = ScoreInfo { idx: cx.index() as u8, ..Default::default() };
    msg::one(Msg::ScoreInfo, e, |w| me.write(w));
    send_motd(e);
    for i in 1..=eng.max_clients() {
        let o = eng.client(i);
        if o.is_null() || !super::is_player(o) {
            continue;
        }
        let s = if o == e { score_info_of(p, cx.pev(), i) } else { other(o, |q| score_info_of(q, o.v(), i)).unwrap_or_default() };
        msg::one(Msg::ScoreInfo, e, |w| s.write(w));
    }
    if teamplay {
        teams::init_hud(p, cx);
    }
}

pub fn score_info_of(p: &Player, v: &entvars_t, idx: i32) -> ScoreInfo {
    ScoreInfo { idx: idx as u8, frags: v.frags as i16, deaths: p.deaths as i16, class: 0, team: p.team as i16 }
}

/// `CHalfLifeMultiplay::SendMOTDToClient`: ServerName, then the `motdfile` in ≤60-char chunks.
pub fn send_motd(e: *mut edict_t) {
    let eng = engine();
    let host = eng.cvar_string(c"hostname").to_string_lossy().into_owned();
    msg::one(Msg::ServerName, e, |w| ServerName { name: host }.write(w));
    let file = {
        let f = eng.cvar_string(c"motdfile").to_string_lossy().into_owned();
        if f.is_empty() { "motd.txt".to_owned() } else { f }
    };
    let Some(data) = eng.load_file(&file) else { return };
    let text: Vec<u8> = data.into_iter().take_while(|&b| b != 0).take(0x600).collect();
    for (chunk, last) in motd_chunks(&text) {
        let m = Motd { last: last as u8, chunk };
        msg::one(Msg::Motd, e, |w| m.write(w));
    }
}

/// Splits MOTD text into ≤60-byte chunks; the last carries the final flag.
pub fn motd_chunks(text: &[u8]) -> Vec<(String, bool)> {
    let chunks: Vec<&[u8]> = text.chunks(60).collect();
    let n = chunks.len();
    chunks.into_iter().enumerate().map(|(i, c)| (String::from_utf8_lossy(c).into_owned(), i + 1 == n)).collect()
}

/// `CTSGameRules::InitHUD` (0x79134): cheat notice, SrvSett ×4, RoundTime, Spectator flags of
/// everyone, then the joiner starts as an observer with a respawn delay.
fn ts_init_hud(p: &mut Player, cx: &mut Cx) {
    let e = cx.edict;
    if cvars::cheats() {
        ts_message_one(e, [45, 195, 45], 8, 100, "This server has cheats enabled.");
        let dm = cvars::f(c"damagemult");
        if dm > 0.0 {
            ts_message_one(e, [45, 195, 45], 8, 100, &format!("This server has custom weapon damages ({dm:.2})"));
        }
    }
    let (rb, mask, round_end, timer) = state::with(|g| (g.last_srvsett[0].max(0), g.allowed_weapons, g.round_end, g.round_timer));
    let sets = [(0u8, rb), (1, cvars::on(c"usecash") as i32), (2, cvars::f(c"ammocount").round() as i32), (3, mask)];
    for (key, value) in sets {
        let s = SrvSett { key, value: value as i16 };
        msg::one(Msg::SrvSett, e, |w| s.write(w));
    }
    if round_end != 0.0 && timer {
        let rt = RoundTime { secs: (round_end - cx.time()).max(0.0).round() as u8, state: 0 };
        msg::one(Msg::RoundTime, e, |w| rt.write(w));
    }
    let eng = engine();
    for i in 1..=eng.max_clients() {
        let o = eng.client(i);
        if o.is_null() || !super::is_player(o) {
            continue;
        }
        let s = Spectator { entindex: i as u8, observer: (o.v().iuser1 != 0) as u8 };
        msg::one(Msg::Spectator, e, |w| s.write(w));
    }
    let (org, ang) = {
        let v = cx.pev();
        (v.origin, v.v_angle)
    };
    start_observer(p, cx, org, ang);
    p.ts.respawns_used = 0;
    p.ts.tk_until = 0.0;
    p.ts.tk_count = 0;
    set_respawn_time(p, cx, cvars::respawn_time());
    let idx = cx.index() as usize;
    state::with(|g| g.votes.reset(idx));
}

/// `SetPlayerRespawnTime` (0x75ff4).
pub fn set_respawn_time(p: &mut Player, cx: &mut Cx, secs: f32) {
    let now = cx.time();
    let (mode, _, lms, _) = state::mode();
    let round_end = state::with(|g| g.round_end);
    if lms && mode == state::Mode::Dm {
        if p.ts.respawns_used as f32 >= cvars::RESPAWN_NUMBER.max(1.0) {
            p.respawn_at = -1.0;
        } else {
            p.respawn_at = secs.min(round_end - now) + now;
            p.ts.join_deadline = (round_end - now).min(10.0) + now;
        }
    } else {
        p.respawn_at = now + secs;
    }
    p.ts.award = if p.respawn_at < 0.0 { 1 } else { 11 };
}

// ---- observer ------------------------------------------------------------------------------------

/// `CBasePlayer::StartObserver` (0x99634).
pub fn start_observer(p: &mut Player, cx: &mut Cx, origin: [f32; 3], angles: [f32; 3]) {
    let e = cx.edict;
    let idx = cx.index();
    crate::entities::stock::drive::release_tank(p, cx);
    p.hud.cash = Some(p.cash);
    messages::one(Msg::TsCash, e).short(p.cash).end();
    {
        let v = cx.pev();
        v.iuser4 = 0;
        v.fuser1 = 1.0;
    }
    p.ts.slow_display = 1.0;
    p.slow_target = 1.0;
    MsgWriter::begin(MSG_PAS as i32, SVC_TEMPENTITY, Some(origin), std::ptr::null_mut())
        .byte(TE_KILLPLAYERATTACHMENTS)
        .byte(idx & 0xff)
        .end();
    crate::weapons::strip_all(p, cx);
    crate::entities::ts::drop_flag(p, cx, false);
    msg::all(Msg::ActItems, |w| ActItems { entindex: idx as u8, mask: 0 }.write(w));
    msg::one(Msg::SetFov, e, |w| SetFov { fov: 0 }.write(w));
    {
        let v = cx.pev();
        v.fov = 0.0;
        v.effects = EF_NODRAW as i32;
        v.view_ofs = [0.0; 3];
        v.v_angle = angles;
        v.angles = angles;
        v.fixangle = 1;
        v.solid = SOLID_NOT as i32;
        v.takedamage = DAMAGE_NO as f32;
        v.movetype = MOVETYPE_NONE as i32;
        v.flags &= !(FL_DUCKING as i32);
        v.deadflag = DEAD_RESPAWNABLE as i32;
        v.health = 1.0;
        v.velocity = [0.0; 3];
    }
    p.ts.init_hud = true;
    let team = teams::team_name_of(e);
    msg::all(Msg::TeamInfo, |w| TeamInfo { idx: idx as u8, team }.write(w));
    engine().set_origin(e, origin);
    p.ts.obs_next_idle = 0.0;
    let mode = if p.ts.obs_mode == 0 { obs::CHASE_FREE } else { p.ts.obs_mode };
    observer_set_mode(p, cx, mode);
    msg::all(Msg::Spectator, |w| Spectator { entindex: idx as u8, observer: 1 }.write(w));
}

/// `CBasePlayer::Observer_SetMode` (0x99e44). Only TS's modes 1, 2, 3 and 5 are accepted; the
/// chase and in-eye modes need a target (`m_hObserverTarget`) and fall back to roaming.
fn observer_set_mode(p: &mut Player, cx: &mut Cx, mode: i32) {
    if mode == cx.pev().iuser1 {
        return;
    }
    {
        let v = cx.pev();
        v.view_ofs = [0.0; 3];
        v.viewmodel = 0;
    }
    if mode == obs::ROAMING {
        let v = cx.pev();
        v.iuser1 = obs::ROAMING;
        v.iuser2 = 0;
        engine().set_client_maxspeed(cx.edict, 420.0);
        return;
    }
    if !matches!(mode, obs::CHASE_LOCKED | obs::CHASE_FREE | obs::IN_EYE) {
        return;
    }
    if !obs_handle_valid(p.ts.obs_target) {
        observer_find_next_player(p, cx, false);
    }
    if !obs_handle_valid(p.ts.obs_target) {
        observer_set_mode(p, cx, obs::ROAMING);
        return;
    }
    let v = cx.pev();
    v.iuser1 = mode;
    v.iuser2 = p.ts.obs_target;
}

/// `m_hObserverTarget` is an `EHANDLE` in TS: it goes null when the player leaves.
fn obs_handle_valid(idx: i32) -> bool {
    idx > 0 && super::is_player(engine().client(idx))
}

/// `CBasePlayer::Observer_FindNextPlayer` (0x99b0c): steps from the current target (or from us)
/// to the next living, visible, non-observing player, wrapping around. In LMS (and the
/// unreachable round modes) an observer still waiting to respawn may only watch teammates.
fn observer_find_next_player(p: &mut Player, cx: &mut Cx, reverse: bool) {
    let eng = engine();
    let me = cx.index();
    let start = if obs_handle_valid(p.ts.obs_target) { p.ts.obs_target } else { me };
    p.ts.obs_target = 0;
    let n = eng.max_clients();
    let teammates_only = p.respawn_at >= 0.0 && state::with(|g| g.lms || matches!(g.mode, state::Mode::RoundTdm | state::Mode::Objective));
    let mut i = start;
    loop {
        i += if reverse { -1 } else { 1 };
        if i > n {
            i = 1;
        }
        if i < 1 {
            i = n;
        }
        let o = eng.client(i);
        if i != me
            && super::is_player(o)
            && {
                let v = o.v();
                v.iuser1 == 0 && v.effects & EF_NODRAW as i32 == 0 && alive(v)
            }
            && (!teammates_only || teams::same_team(cx.edict, o))
        {
            p.ts.obs_target = i;
            break;
        }
        if i == start {
            break;
        }
    }
    if p.ts.obs_target == 0 {
        observer_set_mode(p, cx, obs::ROAMING);
    } else {
        cx.pev().iuser2 = p.ts.obs_target;
        let org = eng.client(p.ts.obs_target).v().origin;
        eng.set_origin(cx.edict, org);
    }
}

/// Observer part of `CBasePlayer::PreThink` (0x908cc): respawn countdown text,
/// `Observer_HandleButtons`, `Observer_Idle`.
///
/// The countdown runs only while the respawn time is ahead. "Press Fire To Play!" (30 s hold) is
/// sent by the tick that lands in its last half second, and isn't repeated after that.
fn observer_think(p: &mut Player, cx: &mut Cx, now: f32) {
    let e = cx.edict;
    if p.respawn_at > 0.0 && now < p.respawn_at && p.ts.next_hud_msg < now {
        p.ts.next_hud_msg = now + 1.0;
        // x87 `fistp` rounding, ties to even.
        let left = (p.respawn_at - now).round_ties_even() as i32;
        if left < 1 {
            ts_message_one(e, [0, 0, 0], 16, 30, "Press Fire To Play!");
        } else {
            ts_message_one(e, [0, 0, 0], 16, 20, &format!("{}", (p.respawn_at - now).min(30.0).round_ties_even() as i32));
        }
    }
    if p.ts.join_deadline != 0.0 {
        if now <= p.ts.join_deadline {
            if p.ts.next_hud_msg < now {
                p.ts.next_hud_msg = now + 1.0;
                let left = (p.ts.join_deadline - now).round() as i32;
                ts_message_one(e, [0, 0, 0], 16, 20, &format!("{left} seconds left for respawn, press Fire to Play!"));
            }
        } else {
            p.ts.join_deadline = 0.0;
            p.respawn_at = -1.0;
            p.ts.award = 1;
            ts_message_one(e, [195, 45, 45], 16, 20, "You have to wait until the next round.");
            state::with(|g| g.check_deads_at = now + 1.0);
        }
    }

    observer_handle_buttons(p, cx, now);
    observer_idle(p, cx, now);
}

/// `CBasePlayer::Observer_HandleButtons` (0x99d34): jump cycles roaming → chase free → chase
/// locked → in-eye → roaming; forward/back pick the next/previous target. 0.2 s between actions.
fn observer_handle_buttons(p: &mut Player, cx: &mut Cx, now: f32) {
    if p.ts.obs_next_input > now {
        return;
    }
    let buttons = cx.pev().button;
    if buttons & IN_JUMP as i32 != 0 {
        let mode = match cx.pev().iuser1 {
            obs::ROAMING => obs::CHASE_FREE,
            obs::CHASE_FREE => obs::CHASE_LOCKED,
            obs::CHASE_LOCKED => obs::IN_EYE,
            _ => obs::ROAMING,
        };
        observer_set_mode(p, cx, mode);
        p.ts.obs_next_input = now + 0.2;
    }
    if buttons & IN_FORWARD as i32 != 0 && cx.pev().iuser1 != obs::ROAMING {
        observer_find_next_player(p, cx, false);
        p.ts.obs_next_input = now + 0.2;
    }
    if buttons & IN_BACK as i32 != 0 && cx.pev().iuser1 != obs::ROAMING {
        observer_find_next_player(p, cx, true);
        p.ts.obs_next_input = now + 0.2;
    }
}

/// `CBasePlayer::Observer_Idle` (0x99ca4): twice a second, move on when the target died or
/// became an observer. The camera itself follows `iuser2` in `PM_SpectatorMove`.
fn observer_idle(p: &mut Player, cx: &mut Cx, now: f32) {
    if p.ts.obs_next_idle > now {
        return;
    }
    p.ts.obs_next_idle = now + 0.5;
    if cx.pev().iuser1 == obs::ROAMING || p.ts.obs_target == 0 {
        return;
    }
    let t = engine().client(p.ts.obs_target);
    if !obs_handle_valid(p.ts.obs_target) || !alive(t.v()) || t.v().iuser1 != 0 {
        observer_find_next_player(p, cx, false);
    }
}

/// The `respawn` client command: leave observer mode once the respawn time has passed.
pub fn respawn_command(p: &mut Player, cx: &mut Cx) {
    let now = cx.time();
    if cx.pev().iuser1 == 0 {
        return;
    }
    if p.respawn_at < 0.0 && p.ts.next_hud_msg < now {
        p.ts.next_hud_msg = now + 1.0;
        ts_message_one(cx.edict, [195, 45, 45], 16, 20, "You have to wait until the next round.");
    }
    if p.respawn_at <= 0.0 || now <= p.respawn_at {
        return;
    }
    p.respawn_at = 0.0;
    stop_observer(p, cx);
    cx.pev().impulse = 0;
}

/// True if the `respawn` command would succeed now (bots use this).
pub fn can_respawn(p: &Player, cx: &mut Cx) -> bool {
    cx.pev().iuser1 != 0 && p.respawn_at > 0.0 && cx.time() > p.respawn_at
}

/// `CBasePlayer::StopObserver` (0x99a14) → `respawn()` → `Spawn()`.
pub fn stop_observer(p: &mut Player, cx: &mut Cx) {
    if p.respawn_at > 0.0 && p.respawn_at > cx.time() {
        return;
    }
    let idx = cx.index();
    let (u1, u2) = (cx.pev().iuser1, cx.pev().iuser2);
    if u1 != 0 || u2 != 0 {
        p.ts.obs_mode = u1;
        msg::all(Msg::Spectator, |w| Spectator { entindex: idx as u8, observer: 0 }.write(w));
        let v = cx.pev();
        v.iuser1 = 0;
        v.iuser2 = 0;
    }
    cx.pev().button = 0;
    p.joined = true;
    p.spawn(cx);
    cx.pev().nextthink = -1.0;
    if engine().cvar_float(c"developer") != 0.0 {
        let owned: Vec<usize> = (1..p.inv.gun.status.len()).filter(|&i| p.inv.gun.status[i].owned).collect();
        engine().print(&format!("[ts] {} spawned (#{idx}) with {owned:?}\n", netname(cx.edict)));
    }
}

// ---- death --------------------------------------------------------------------------------------

/// `CBasePlayer::Killed` (0x8ed5c) with `CHalfLifeMultiplay::PlayerKilled` in the middle.
pub fn killed(p: &mut Player, cx: &mut Cx, dmg: &DamageInfo) {
    let eng = engine();
    let now = cx.time();
    let e = cx.edict;
    let idx = cx.index();
    let close_combat = scoring::is_close_combat(dmg);

    // Motion blur trails on the killer (50 %, 80 % for melee) and off on the victim (50 %).
    let attacker = dmg.attacker;
    if !attacker.is_null() && eng.random_long(0, 100) <= if close_combat { 0x4f } else { 0x31 } {
        let org = attacker.v().origin;
        let b = TsMBlur { entindex: eng.index_of(attacker) as u8, mode: 1 };
        msg::pvs(Msg::TsMBlur, org, |w| b.write(w));
    }
    if eng.random_long(0, 100) < 0x32 {
        let b = TsMBlur { entindex: idx as u8, mode: 0 };
        msg::pvs(Msg::TsMBlur, cx.pev().origin, |w| b.write(w));
    }

    p.ts.observer_at = now + 3.0;
    p.hud.cash = None;
    eng.set_view(e, e);
    crate::entities::stock::drive::release_tank(p, cx);
    p.ts.respawns_used = p.ts.respawns_used.saturating_add(1);
    p.ts.suicide_at = 0.0;
    // The spectator target is the last inflictor when it's a player (TS: `g_pevLastInflictor`);
    // otherwise only iuser2 is cleared and `m_hObserverTarget` keeps its old value.
    if super::is_player(dmg.inflictor) && dmg.inflictor != e {
        let k = eng.index_of(dmg.inflictor);
        cx.pev().iuser2 = k;
        p.ts.obs_target = k;
    } else {
        cx.pev().iuser2 = 0;
    }
    set_respawn_time(p, cx, cvars::respawn_time());

    // Headshot death: slow the victim a little unless global slow is on.
    if p.ts.last_hitgroup == hitgroup::HEAD && !state::with(|g| g.global_slow.active) {
        powerup::go_slow_self(p, cx, 0.33, false);
    }
    cx.pev().iuser4 = stunt::DEAD_LOCK;
    // Death pose: CTSAnimation::Animate(7) reads the last damage, hitgroup and attack direction.
    p.stunt.anim.last_damage = p.ts.last_damage;
    p.stunt.anim.last_hitgroup = p.ts.last_hitgroup;
    p.stunt.anim.attack_dir = dmg.dir;
    crate::stunt::animate(p, cx, 7);

    powerup::deactivate_flow(p, cx);
    crate::weapons::on_killed(p, cx);
    crate::entities::ts::drop_flag(p, cx, false);

    scoring::player_killed(p, cx, dmg);

    // Dying: physics, knockback, HUD.
    let force = (p.ts.last_damage as f32 * 5.0).min(500.0);
    {
        let v = cx.pev();
        v.modelindex = eng.model_index(crate::player::PLAYER_MODEL);
        v.deadflag = DEAD_DYING as i32;
        v.movetype = MOVETYPE_TOSS as i32;
        v.flags &= !(FL_ONGROUND as i32);
        for i in 0..3 {
            v.velocity[i] += dmg.dir[i] * force;
        }
        if super::len2d(v.velocity) > 80.0 || p.ts.last_damage > 50 {
            v.velocity[2] = 180.0;
            v.gravity = 0.5;
        }
    }
    p.hud.health = Some(0);
    msg::one(Msg::TsHealth, e, |w| TsHealth { hp: 0 }.write(w));
    msg::all(Msg::ActItems, |w| ActItems { entindex: idx as u8, mask: 0 }.write(w));
    cx.pev().fov = 0.0;
    msg::one(Msg::SetFov, e, |w| SetFov { fov: 0 }.write(w));

    let gib = dmg.bits & crate::damage::bits::ALWAYSGIB != 0 || (cx.pev().health < -40.0 && dmg.bits & crate::damage::bits::NEVERGIB == 0);
    let v = cx.pev();
    v.takedamage = DAMAGE_NO as f32;
    if gib {
        v.solid = SOLID_NOT as i32;
        v.effects |= EF_NODRAW as i32;
    } else {
        v.angles[0] = 0.0;
        v.angles[2] = 0.0;
    }
}

/// `CBasePlayer::PlayerDeathThink` (0x8eaec): settle, play out the death animation, then become
/// an observer after 3 s or on any button, leaving a corpse.
fn death_think(p: &mut Player, cx: &mut Cx, now: f32) {
    {
        let v = cx.pev();
        if v.flags & FL_ONGROUND as i32 != 0 {
            let speed = super::len3(v.velocity);
            let f = speed - 20.0;
            if f > 0.0 {
                for i in 0..3 {
                    v.velocity[i] = v.velocity[i] / speed * f;
                }
            } else {
                v.velocity = [0.0; 3];
            }
        }
    }
    // Stay dying until the death animation finishes, advancing it at the animation rate
    // (`framerate` is the player's time scale, so slow motion stretches it). Port addition: a
    // sequence without studio data (frame rate 0) would never finish, so it ends at once.
    let anim = &p.stunt.anim;
    if cx.pev().modelindex != 0 && !anim.seq_finished && anim.frame_rate > 0.0 && cx.pev().deadflag == DEAD_DYING as i32 {
        crate::stunt::anim::studio_frame_advance(p, cx);
        return;
    }
    let v = cx.pev();
    if v.movetype != MOVETYPE_NONE as i32 && v.flags & FL_ONGROUND as i32 != 0 {
        v.movetype = MOVETYPE_NONE as i32;
    }
    if v.deadflag == DEAD_DYING as i32 {
        v.deadflag = DEAD_DEAD as i32;
    }
    v.framerate = 0.0;
    v.effects |= EF_NOINTERP;
    if v.deadflag == DEAD_DEAD as i32 {
        v.deadflag = DEAD_RESPAWNABLE as i32;
    } else if (p.ts.observer_at < now || v.button & !(IN_SCORE as i32) != 0) && v.iuser1 == 0 {
        let (org, ang) = (v.origin, v.v_angle);
        crate::entities::ts::create_dead_corpse(cx.edict, 10.0);
        start_observer(p, cx, org, ang);
    }
}

// ---- +use ----------------------------------------------------------------------------------------

/// SDK `PlayerUse` reduced to what TS maps need: the closest usable TS item (ground weapons)
/// within 64 units in front of the player gets `Use`.
fn player_use(p: &mut Player, cx: &mut Cx) {
    let eng = engine();
    let (eye, angles) = {
        let v = cx.pev();
        (super::eye(v), v.v_angle)
    };
    let (fwd, _, _) = eng.angle_vectors(angles);
    let mut best = std::ptr::null_mut();
    let mut best_dot = 0.5f32;
    let mut it = std::ptr::null_mut();
    loop {
        it = eng.find_in_sphere(it, eye, 64.0);
        if it.is_null() {
            break;
        }
        let v = it.v();
        let cls = eng.string(v.classname);
        if !crate::entities::ts::is_usable(cls) || v.effects & EF_NODRAW as i32 != 0 {
            continue;
        }
        let c = [(v.absmin[0] + v.absmax[0]) * 0.5, (v.absmin[1] + v.absmax[1]) * 0.5, (v.absmin[2] + v.absmax[2]) * 0.5];
        let d = [c[0] - eye[0], c[1] - eye[1], c[2] - eye[2]];
        let l = super::len3(d).max(0.001);
        let dot = (d[0] * fwd[0] + d[1] * fwd[1] + d[2] * fwd[2]) / l;
        if dot > best_dot {
            best_dot = dot;
            best = it;
        }
    }
    if !best.is_null() {
        crate::entities::ts::player_use_item(best, p, cx);
    }
}

/// Broadcasts `TSPAward` for a player (used by scoring for the killer).
pub fn send_award(idx: i32, award: u8) {
    msg::all(Msg::TsPAward, |w| TsPAward { entindex: idx as u8, award }.write(w));
}
