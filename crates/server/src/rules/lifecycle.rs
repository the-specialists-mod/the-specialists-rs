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
use crate::world::Cx;
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
/// the voice manager), then accept. `CHalfLifeRules::ClientConnected` (0xde794) only accepts.
pub fn client_connect(e: *mut edict_t, _name: &CStr, _addr: &CStr) -> bool {
    crate::entities::ts::player_amount_changed(true);
    if !state::single_player() {
        super::voice::client_connected(e);
    }
    true
}

/// `ClientDisconnect` (0xacd4c): `ts_playeramount`, then, unless the game is over, the leave
/// notice and `CHalfLifeMultiplay::ClientDisconnected` (0xcf960). The engine keeps the edict and
/// its `Player` until the slot's next client spawns, so the rules keep seeing the player through
/// `slot_players()` (`UTIL_PlayerByIndex`) with what `TSPlayerDisconnect` left.
pub fn client_disconnect(e: *mut edict_t) {
    crate::entities::ts::player_amount_changed(false);
    let game_over = state::with(|g| g.game_over);
    let eng = engine();
    let idx = eng.index_of(e);
    if !game_over {
        let name = netname(e);
        msg::say_all(idx, &format!("- {name} has left the game\n"));
        let v = e.v();
        v.takedamage = DAMAGE_NO as f32;
        v.solid = SOLID_NOT as i32;
        eng.set_origin(e, v.origin);
        // `CHalfLifeRules::ClientDisconnected` (0xde7a0) is empty.
        if !state::single_player() {
            client_disconnected(e, idx);
        }
    }
    // Port addition (engine safety): the engines leave FL_CLIENT on a dropped client's edict, and
    // FWGS still maps it to the freed slot: pfnGetPlayerUserId returns the old id, and
    // pfnSetClientMaxspeed then dereferences the slot's NULL edict (the "kick a bot" crash).
    // The cleared flag is the port's liveness test (`rules::set_client_maxspeed`, players(),
    // is_player(), the bot list) until a client spawns in the slot. TS's own tests don't see it:
    // `has_player`, `slot_players()` and `ts_flags` (which puts FL_CLIENT back) still find the
    // player, as `IsPlayer()`, `UTIL_PlayerByIndex` and `pev->flags & FL_CLIENT` do in TS.
    (e.v().flags &= !((FL_CLIENT | FL_FAKECLIENT) as i32));
}

/// `CHalfLifeMultiplay::ClientDisconnected` (0xcf960): `game_playerleave`, the log line,
/// `RemoveAllItems(TRUE)`, `TSPlayerDisconnect` (0x9f94c), and the next The One check at once
/// (`+0x587c = 0`). TS neither stops the player's slow-motion flow nor frees the players it slowed.
fn client_disconnected(e: *mut edict_t, idx: i32) {
    if !e.has_private_data() {
        return;
    }
    crate::entities::fire_targets(c"game_playerleave", e, e, crate::world::UseType::Toggle, 0.0);
    let eng = engine();
    let (uid, auth) = (eng.user_id(e), eng.auth_id(e).to_string_lossy().into_owned());
    // The fourth field is the user id again outside teamplay, the `model` key in teamplay.
    let tail = if state::teamplay() { eng.info_key(e, c"model").to_string_lossy().into_owned() } else { uid.to_string() };
    msg::log(&format!("\"{}<{uid}><{auth}><{tail}>\" disconnected\n", netname(e)));
    crate::player::with_player(e, |p, cx| {
        // `RemoveAllItems(TRUE)`, `UpdateClientData` inline included.
        crate::weapons::remove_all_items(p, cx);
        // TSPlayerDisconnect.
        crate::entities::ts::drop_flag(p, cx, false);
        if (idx as u32) < 0x20 {
            state::with(|g| g.votes.reset(idx as usize));
        }
        {
            let v = cx.pev();
            v.solid = SOLID_NOT as i32;
            v.deadflag = DEAD_DEAD as i32;
            v.takedamage = DAMAGE_NO as f32;
            v.flags &= !(FL_FAKECLIENT as i32);
            v.flags |= FL_DORMANT as i32;
            v.frags = 0.0;
            v.effects = EF_NODRAW as i32;
            v.health = 0.0;
        }
        p.frags = 0;
        p.deaths = 0;
        p.respawn_at = -1.0;
        teams::clear_team(p, cx.edict);
        p.ts.is_the_one = false;
        // `m_fGameHUDInitialized` (`+0x9b8`) off, and `UpdateClientData` off for good (`+0x534`
        // = -1).
        p.ts.game_hud_initialized = false;
        p.hud.next_update = -1.0;
        // `m_bIsBot` off (the object stays a `CBot`, so its brain and `IsNetClient` are kept).
        p.is_bot = 0;
        p.joined = false;
    });
    state::with(|g| {
        g.call_check_deads(eng.time());
        g.next_the_one_check = 0.0;
    });
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

/// `ClientUserInfoChanged` (0xae638): a name change (`%` turned into spaces, announced, logged),
/// then the rules' `ClientUserInfoChanged`.
pub fn userinfo_changed(e: *mut edict_t) {
    if e.is_null() || !e.has_private_data() {
        return;
    }
    let eng = engine();
    let old = netname(e);
    if !old.is_empty() && eng.string(e.v().netname).to_bytes() != eng.info_key(e, c"name").to_bytes() {
        let clean = eng.info_key(e, c"name").to_string_lossy().replace('%', " ");
        let idx = eng.index_of(e);
        eng.set_client_key(e, "name", &clean);
        let new = eng.info_key(e, c"name").to_string_lossy().into_owned();
        msg::say_all(idx, &format!("* {old} changed name to {new}\n"));
        // The fourth field is the user id outside teamplay and the `model` key in teamplay.
        let uid = eng.user_id(e);
        let tail = if state::teamplay() { eng.info_key(e, c"model").to_string_lossy().into_owned() } else { uid.to_string() };
        msg::log(&format!("\"{old}<{uid}><{}><{tail}>\" changed name to \"{new}\"\n", eng.auth_id(e).to_string_lossy()));
    }
    // `CHalfLifeRules` keeps `CGameRules::ClientUserInfoChanged` (empty).
    if state::single_player() {
        return;
    }
    crate::player::with_player(e, |p, cx| {
        multiplay_userinfo_changed(p, cx);
        if state::teamplay() {
            teams::userinfo_changed(p, cx);
        }
    });
}

/// `CHalfLifeMultiplay::ClientUserInfoChanged` (0x8b244): a client not running weapon prediction
/// gets `cl_lw 1` (`PlayerIsCheating`); otherwise a `model` key that differs from the team name
/// (`""` outside teamplay) is put back to the team name when it is an HLDM model or
/// `mp_defaultteam` is on.
fn multiplay_userinfo_changed(p: &mut Player, cx: &mut Cx) {
    let e = cx.edict;
    let eng = engine();
    if player_is_cheating(&eng.info_key(e, c"cl_lw").to_string_lossy()) {
        eng.set_client_key(e, "cl_lw", "1");
        return;
    }
    let model = eng.info_key(e, c"model").to_string_lossy().into_owned();
    let team = p.ts.team_name.clone();
    if model.eq_ignore_ascii_case(&team) {
        return;
    }
    if !cvars::on(c"mp_defaultteam") && !HLDM_MODELS.iter().any(|m| m.eq_ignore_ascii_case(&model)) {
        return;
    }
    eng.set_client_key(e, "model", &team);
    let idx = cx.index();
    msg::say_one(e, idx, &format!("* Can't change team to '{model}'\n"));
    msg::say_one(e, idx, "* HLDM models not allowed.\n");
}

/// `PlayerIsCheating` (0x9fae0): `sscanf(cl_lw, "%i") != 1` (its bot flag `+0x160` is never set).
/// A value `%i` can't read leaves TS comparing an uninitialised stack word; the port treats it as
/// not 1.
fn player_is_cheating(cl_lw: &str) -> bool {
    scan_c_int(cl_lw) != Some(1)
}

/// `sscanf("%i")`: optional whitespace and sign, then a hex (`0x`), octal (leading `0`) or decimal
/// number, read up to the first character that doesn't fit.
fn scan_c_int(s: &str) -> Option<i64> {
    let s = s.trim_start();
    let (neg, s) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let (radix, digits) = if (s.starts_with("0x") || s.starts_with("0X")) && s[2..].starts_with(|c: char| c.is_ascii_hexdigit()) {
        (16, &s[2..])
    } else if s.starts_with('0') {
        (8, s)
    } else {
        (10, s)
    };
    let end = digits.find(|c: char| !c.is_digit(radix)).unwrap_or(digits.len());
    if end == 0 {
        return None;
    }
    let v = i64::from_str_radix(&digits[..end], radix).ok()?;
    Some(if neg { -v } else { v })
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
    p.next_node_learn = 0.0;
    // `SetPlayerSpeed(330)` (the gameplay rules' `PmRules::max_player_speed`).
    let speed = crate::gameplay::active().pm.max_player_speed();
    p.player_speed = speed;
    crate::weapons::ts_init(p);
    p.ts.protect_until = now + 1.0;
    p.ts.init_hud = true;
    // `CBasePlayer::Spawn` (0x8f568) zeroes `m_fLongJump` (+0x9d4) before the key.
    p.long_jump = false;
    e.set_physics_key(cx.edict, c"slj", c"0");
    e.set_physics_key(cx.edict, c"hl", c"1");
    e.set_physics_key(cx.edict, c"mj", if superjump_map { c"1" } else { c"0" });
    // Port addition: the gameplay ruleset both sides' movement and guns run.
    crate::gameplay::set_physinfo(cx.edict);
    super::set_client_maxspeed(cx.edict, speed);
    crate::entities::stock::triggers::geiger_spawn(cx.edict);
    {
        let v = cx.pev();
        v.maxspeed = speed;
        v.fuser1 = 1.0;
        v.iuser4 = 0;
        v.renderamt = 125.0;
        v.rendermode = kRenderTransColor as i32;
        v.rendercolor = [0.0; 3];
        v.renderfx = kRenderFxNoDissipation as i32;
        v.sequence = 0;
        v.gaitsequence = 0;
        v.frame = 0.0;
        // The suit bit, from `CHalfLifeMultiplay::PlayerSpawn` (`CHalfLifeRules::PlayerSpawn`
        // 0xde7d8 is empty).
        if !state::single_player() {
            v.weapons |= 0x8000_0000u32 as i32;
        }
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
        // `BotSetup` (0x1006b3b0), TS's put-in-server for bots, gives neither.
        if !p.cbot {
            give_cash(p, crate::gameplay::active().server.put_in_server_cash());
            cx.pev().effects |= EF_NOINTERP;
        }
    }

    crate::gameplay::active().server.on_spawn(p, cx);
    if state::teamplay() {
        teams::set_default_team(p, cx);
    }
    spawnpoint::place(p, cx);
}

/// `CBasePlayer::GiveCash`: clamped to 0..30000 (the sum wraps like TS's C int first: a map's
/// `buy_price` can be anything).
pub fn give_cash(p: &mut Player, n: i32) {
    p.cash = p.cash.wrapping_add(n).clamp(0, 30000);
}

// ---- think --------------------------------------------------------------------------------------

/// Buttons that end the intermission once `mp_chattime` is up (`PlayerThink`'s mask 0x827).
const END_INTERMISSION_BUTTONS: i32 = (IN_ATTACK | IN_JUMP | IN_DUCK | IN_USE | IN_ATTACK2) as i32;

/// The head of `CBasePlayer::PreThink` (0x908cc): the button edges, then
/// `g_pGameRules->PlayerThink` (`CHalfLifeMultiplay::PlayerThink` 0xcfb60). Returns true while
/// the game is over: PreThink returns right here then, so the rest of the player's PreThink
/// (water, time-based damage, stunts, weapon timers, trains, …) doesn't run in intermission.
pub fn player_think(p: &mut Player, cx: &mut Cx) -> bool {
    let buttons = cx.pev().button;
    p.ts.buttons_pressed = buttons & (p.ts.buttons_last ^ buttons);
    p.ts.buttons_last = buttons;
    if !state::with(|g| g.game_over) {
        return false;
    }
    // `m_afButtonPressed` is against `m_afButtonLast`, which PostThink stops updating at
    // intermission (the stunt package's copy, see `stunt::post_think`).
    let pressed = buttons & (p.stunt.button_last ^ buttons);
    if pressed & END_INTERMISSION_BUTTONS != 0 {
        state::with(|g| g.end_intermission_button_hit = true);
    }
    p.ts.buttons_pressed = 0;
    p.stunt.button_pressed = 0;
    p.stunt.button_released = 0;
    cx.pev().button = 0;
    true
}

/// The rules part of `CBasePlayer::PreThink` (0x908cc) after `player_think`. True when
/// [`pre_think_end`] is due this frame.
pub fn pre_think(p: &mut Player, cx: &mut Cx, tail: bool) -> bool {
    let now = cx.time();

    // PreThink order: WaterMove, UpdateClientData (InitHUD), CheckTimeBasedDamage, CheckSuitUpdate.
    crate::player::water::water_move(p, cx);
    crate::player::hud::update_client_data(p, cx);
    crate::player::water::check_time_based_damage(p, cx);

    // CheckSuitUpdate (0xda2d0): the geiger counter, and the suit sentences in single player.
    crate::player::suit::check_suit_update(p, cx);

    // Time scale easing toward the target, and animation rate = time scale.
    let ft = engine().globals().frametime;
    {
        let eases = crate::gameplay::active().server.time_scale_eases();
        let v = cx.pev();
        if eases {
            powerup::interpolate(ft, &mut v.fuser1, p.slow_target);
        } else {
            v.fuser1 = p.slow_target;
        }
        v.framerate = v.fuser1.max(0.01);
    }

    powerup::update_state(p, cx);
    powerup::plugin_speed_think(p, cx);
    powerup::slow_flow(p, cx);
    crate::gameplay::active().server.health_regen(p, cx, now);

    if cx.pev().iuser1 != 0 {
        crate::gameplay::active().server.observer_think(p, cx, now);
        cx.pev().impulse = 0;
        return false;
    }
    if cx.pev().deadflag > DEAD_NO as i32 {
        death_think(p, cx);
        return false;
    }

    // No +use double tap here: TS's `CBasePlayer::TSUse` (0x956e0) is never called. The client
    // sends `usepowerup` for a double tap when `cl_usepowerup` is on (issue #49). Using objects
    // is `player::player_use`, from the weapons' PostThink.

    // TS's PreThink has returned already on the frame the train is lost (issue #60).
    tail
}

/// The end of `CBasePlayer::PreThink` (mp.dll 0x1005ff2b..0x1006002e): the end of spawn
/// protection and, as PreThink's very last step, the delayed suicide of `kill`. It runs after
/// the node learning and the weapon timers (issue #155), only when [`pre_think`] returned true
/// (a living, non-observing player on a frame that kept its train).
pub fn pre_think_end(p: &mut Player, cx: &mut Cx) {
    let now = cx.time();
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
        // `Killed(pev, GIB_NEVER)` straight from PreThink: no inflictor.
        let me = cx.edict;
        let info = DamageInfo::new(std::ptr::null_mut(), me, 0.0, crate::damage::bits::NEVERGIB);
        killed(p, cx, &info);
    }
}

/// Rules part of `PostThink`. Fall damage (player-movement.md §6.6) is applied by the stunt
/// module's PostThink together with the landing logic, so nothing to do here yet.
pub fn post_think(_p: &mut Player, _cx: &mut Cx) {}

/// TS 3.0's overheal: health above max decays 1/s; The One regenerates (`PreThink`, `+0x73c`).
pub fn health_regen_core(p: &mut Player, cx: &mut Cx, now: f32) {
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

/// `UpdateClientData`'s init block (0x91d40), inside its 0.1 s throttle: ResetHUD every spawn
/// and observer start, InitHUD and the rules InitHUDs once. Nothing is reset for a resend here:
/// the spawn already did that (`Spawn`/`TSInit`), and the values follow in the same update.
pub fn init_hud_block(p: &mut Player, cx: &mut Cx) {
    p.ts.init_hud = false;
    // `gInitHUD = 0` (UpdateClientData 0x91d00).
    crate::save::set_init_hud(false);
    let e = cx.edict;
    messages::one(Msg::ResetHUD, e).byte(0).end();
    if !p.ts.game_hud_initialized {
        messages::one(Msg::InitHUD, e).end();
        // `CHalfLifeRules::InitHUD` (0xde79c) is empty; `game_playerjoin` only fires when
        // `IsMultiplayer()`. `CTSGameRules::InitHUD` runs either way.
        let single = state::single_player();
        if !single {
            rules_init_hud(p, cx);
        }
        ts_init_hud(p, cx);
        p.ts.game_hud_initialized = true;
        if !single {
            crate::world::lend(e, p, || crate::entities::fire_targets(c"game_playerjoin", e, e, crate::world::UseType::Toggle, 0.0));
        }
    }
    // Lent back to its slot for both, so a `game_player_team` they reach changes our team at
    // once, as in TS.
    crate::world::lend(e, p, || crate::entities::fire_targets(c"game_playerspawn", e, e, crate::world::UseType::Toggle, 0.0));
    let (round_end, timer) = state::with(|g| (g.round_end, g.round_timer));
    if round_end != 0.0 && timer {
        // 0x79370: clamped at 0 and truncated.
        let secs = super::frame::round_time_byte(super::frame::x87_max0(round_end as f64 - cx.time() as f64));
        let rt = RoundTime { secs, state: 0 };
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
    let raw = netname(e);
    let name = if raw.is_empty() { "unconnected" } else { raw.as_str() };
    msg::text_all(hud_dest::NOTIFY, &format!("{name} has joined the game\n"));
    // The log has the raw `netname`, and the user id (DM) or the `model` key (teamplay) last.
    let uid = eng.user_id(e);
    let tail = if teamplay { eng.info_key(e, c"model").to_string_lossy().into_owned() } else { uid.to_string() };
    msg::log(&format!("\"{raw}<{uid}><{}><{tail}>\" entered the game\n", eng.auth_id(e).to_string_lossy()));
    // UpdateGameMode
    let gm = GameMode { teamplay: teamplay as u8 };
    msg::one(Msg::GameMode, e, |w| gm.write(w));
    // Our own (empty) score first, then everyone's.
    let me = ScoreInfo { idx: cx.index() as u8, ..Default::default() };
    msg::one(Msg::ScoreInfo, e, |w| me.write(w));
    send_motd(e);
    // `UTIL_PlayerByIndex`: players who left are listed too.
    for i in 1..=eng.max_clients() {
        let o = eng.player(i);
        if !super::has_player(o) {
            continue;
        }
        let s = if o == e { score_info_of(p, cx.pev(), i) } else { other(o, |q| score_info_of(q, o.v(), i)).unwrap_or_default() };
        msg::one(Msg::ScoreInfo, e, |w| s.write(w));
    }
    if teamplay {
        teams::init_hud(p, cx);
    }
}

/// The `ScoreInfo` TS sends for a player (`AddPoints` 0xd9e18, `PlayerKilled`, `InitHUD`, team
/// changes): `WRITE_SHORT((int)pev->frags)`, `WRITE_SHORT(m_iDeaths)`, 0, team + 1.
pub fn score_info_of(p: &Player, v: &entvars_t, idx: i32) -> ScoreInfo {
    ScoreInfo { idx: idx as u8, frags: score_frags(v.frags), deaths: p.deaths as i16, class: 0, team: teams::score_team(idx) }
}

/// The frags short: a C `(int)` cast (`fistp` under a truncating control word, 0xd9e74..0xd9e8d;
/// out of range gives 0x80000000), of which `WRITE_SHORT` keeps the low 16 bits, so a score past
/// ±32767 wraps instead of sticking at the limit.
pub fn score_frags(frags: f32) -> i16 {
    crate::entities::stock::maprules::x87_int(frags) as i16
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

/// `CTSGameRules::InitHUD` (0x79134): cheat notice, custom weapons, SrvSett ×4, RoundTime, Spectator flags of
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
    // `+0x5884`, set by `loadweapons` on this map (`CTSGameRules::Init` clears it).
    if state::with(|g| g.custom_weapons) {
        crate::weapons::inform_custom_weapons(e);
    }
    // 0x79214..0x7930a: `realbullet` is Think's copy (-1 before the first Think), `usecash` the live
    // cvar `!= 0` (`fldz; fcomp; sete`, so 0.5 sends 1 and a NaN 0), `ammocount` the live cvar
    // truncated, and the weapon mask.
    let (rb, mask, round_end, timer) = state::with(|g| (g.last_srvsett[0], g.allowed_weapons, g.round_end, g.round_timer));
    let cash = !super::frame::x87_eq(cvars::f(c"usecash") as f64, 0.0);
    // Port addition: key 4, the gameplay ruleset for the HUD (`crate::gameplay`); TS's client
    // ignores it.
    let ruleset = (ts_gameplay::SRVSETT_KEY, crate::gameplay::srvsett_value() as i32);
    let sets = [(0u8, rb), (1, cash as i32), (2, super::frame::ammocount()), (3, mask), ruleset];
    for (key, value) in sets {
        let s = SrvSett { key, value: value as i16 };
        msg::one(Msg::SrvSett, e, |w| s.write(w));
    }
    if round_end != 0.0 && timer {
        // 0x79370: clamped at 0 and truncated.
        let secs = super::frame::round_time_byte(super::frame::x87_max0(round_end as f64 - cx.time() as f64));
        let rt = RoundTime { secs, state: 0 };
        msg::one(Msg::RoundTime, e, |w| rt.write(w));
    }
    let eng = engine();
    // `UTIL_PlayerByIndex` + `IsPlayer`: players who left are listed too.
    for i in 1..=eng.max_clients() {
        let o = eng.player(i);
        if !super::has_player(o) {
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
    crate::gameplay::active().server.on_init_hud(p, cx);
}

/// `SetPlayerRespawnTime` as the ruleset has it
/// ([`ServerRules::set_respawn_time`](crate::gameplay::ServerRules::set_respawn_time)).
pub fn set_respawn_time(p: &mut Player, cx: &mut Cx, secs: f32) {
    crate::gameplay::active().server.set_respawn_time(p, cx, secs);
}

/// TS 3.0's `SetPlayerRespawnTime` (0x75ff4).
pub fn set_respawn_time_core(p: &mut Player, cx: &mut Cx, secs: f32) {
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
///
/// TS leaves `pev->velocity` alone, clears the HEV suit bit through `RemoveAllItems(TRUE)` and
/// sends `CurWeapon 0,0,0` from there.
pub fn start_observer(p: &mut Player, cx: &mut Cx, origin: [f32; 3], angles: [f32; 3]) {
    crate::gameplay::active().server.on_start_observer(p, cx);
    let e = cx.edict;
    let idx = cx.index();
    p.hud.cash = Some(p.cash);
    messages::one(Msg::TsCash, e).short(p.cash).end();
    p.ts.obs_next_idle = 0.0;
    {
        let v = cx.pev();
        v.iuser4 = 0;
        v.fuser1 = 1.0;
    }
    p.ts.slow_display = 1.0;
    p.slow_target = 1.0;
    // TS sends this from `pev->origin`, not the new position.
    let here = cx.pev().origin;
    MsgWriter::begin(MSG_PAS as i32, SVC_TEMPENTITY, Some(here), std::ptr::null_mut())
        .byte(TE_KILLPLAYERATTACHMENTS)
        .byte(idx & 0xff)
        .end();
    // The gun is emptied and holstered first (`CTSGun::Reset`, `HolsterWeapon`, then its
    // `Holster`), the case dropped, then a tank let go (`Use(USE_OFF)`, which re-deploys the
    // emptied gun: kung fu). `DropFlag(false)` (0x9975f) comes before `RemoveAllItems`
    // (0x99986), which leaves `free_slots` alone, as it leaves the speed.
    crate::weapons::observer_holster(p, cx);
    crate::entities::ts::drop_flag(p, cx, false);
    crate::entities::stock::drive::release_tank(p, cx);
    // `SetSuitUpdate(NULL, FALSE, 0)` (0x997ac).
    crate::player::suit::set_suit_update(p, cx, None, false, 0);
    msg::all(Msg::ActItems, |w| ActItems { entindex: idx as u8, mask: 0 }.write(w));
    msg::one(Msg::SetFov, e, |w| SetFov { fov: 0 }.write(w));
    p.ts.ifov = 0;
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
    }
    p.ts.init_hud = true;
    let team = teams::team_name_of(e);
    msg::all(Msg::TeamInfo, |w| TeamInfo { idx: idx as u8, team }.write(w));
    // `RemoveAllItems(TRUE)`: the gun's `Holster` again, the items destroyed, then the tail
    // (with `UpdateClientData` inline, which sends ResetHUD now if the throttle allows). The free
    // slots, loadout items and speed stay as they were (`TSSpace` is not resent).
    crate::weapons::remove_all_items(p, cx);
    engine().set_origin(e, origin);
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
        super::set_client_maxspeed(cx.edict, 420.0);
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

/// `ts_observe` (port-only development aid): puts an observing player in observer `mode` on
/// player `target` (0 keeps the current one), as `Observer_SetMode` would after the observer's
/// own `+jump` and `+attack`. Scripted tests use it for clients whose input can't be scripted (the
/// original TS client's anti-script gate). Returns false for a player who isn't observing.
pub fn dev_observe(p: &mut Player, cx: &mut Cx, mode: i32, target: i32) -> bool {
    if cx.pev().iuser1 == 0 {
        return false;
    }
    if target > 0 && obs_handle_valid(target) {
        p.ts.obs_target = target;
        if cx.pev().iuser1 == mode && mode != obs::ROAMING {
            cx.pev().iuser2 = target;
        }
    }
    observer_set_mode(p, cx, mode);
    true
}

/// `m_hObserverTarget` (`+0x75c`) is a raw `CBasePlayer*` in TS: a player who left is still a
/// target (dead, invisible, dormant) until `Observer_Idle` replaces it, as long as the slot keeps
/// its player.
fn obs_handle_valid(idx: i32) -> bool {
    idx > 0 && super::has_player(engine().player(idx))
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
        // `UTIL_PlayerByIndex`: a player who left is still a candidate (only during intermission
        // is it alive and visible).
        let o = eng.player(i);
        if i != me
            && super::has_player(o)
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
        let org = eng.player(p.ts.obs_target).v().origin;
        eng.set_origin(cx.edict, org);
    }
}

/// TS 3.0's observer part of `CBasePlayer::PreThink` (0x908cc): respawn countdown text,
/// `Observer_HandleButtons`, `Observer_Idle`
/// ([`ServerRules::observer_think`](crate::gameplay::ServerRules::observer_think)). An observer
/// leaves with the `respawn` client command ([`respawn_command`]), not from here.
///
/// The countdown runs only while the respawn time is ahead. "Press Fire To Play!" (30 s hold) is
/// sent by the tick that lands in its last second, and isn't repeated after that.
pub fn observer_think_core(p: &mut Player, cx: &mut Cx, now: f32) {
    let e = cx.edict;
    if p.respawn_at > 0.0 && now < p.respawn_at && p.ts.next_hud_msg < now {
        p.ts.next_hud_msg = now + 1.0;
        let (hold, text) = respawn_countdown_text(p.respawn_at - now);
        ts_message_one(e, [0, 0, 0], 16, hold, &text);
    }
    if p.ts.join_deadline != 0.0 {
        if now <= p.ts.join_deadline {
            if p.ts.next_hud_msg < now {
                p.ts.next_hud_msg = now + 1.0;
                // `(int)` cast: truncation (0x91063, like the countdown).
                let left = (p.ts.join_deadline - now) as i32;
                ts_message_one(e, [0, 0, 0], 16, 20, &format!("{left} seconds left for respawn, press Fire to Play!"));
            }
        } else {
            p.ts.join_deadline = 0.0;
            p.respawn_at = -1.0;
            p.ts.award = 1;
            ts_message_one(e, [195, 45, 45], 16, 20, "You have to wait until the next round.");
            state::with(|g| g.call_check_deads(now));
        }
    }

    observer_handle_buttons(p, cx, now);
    observer_idle(p, cx, now);
}

/// The observer respawn countdown text and hold time for `left` seconds to go (PreThink
/// 0x90ec4..0x90fab). TS converts with a plain `(int)` cast, which GCC emits as `fistp` under a
/// truncating control word (`fldcw` with RC = 11), so 4.7 s shows "4" and the last second
/// shows "Press Fire To Play!". The number is capped at 30.
fn respawn_countdown_text(left: f32) -> (u8, String) {
    if (left as i32) < 1 { (30, "Press Fire To Play!".to_owned()) } else { (20, format!("{}", left.min(30.0) as i32)) }
}

/// `CBasePlayer::Observer_HandleButtons` (0x99d34): jump cycles roaming → chase free → chase
/// locked → in-eye → roaming; forward/back pick the next/previous target. 0.2 s between actions.
pub fn observer_handle_buttons(p: &mut Player, cx: &mut Cx, now: f32) {
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
pub fn observer_idle(p: &mut Player, cx: &mut Cx, now: f32) {
    if p.ts.obs_next_idle > now {
        return;
    }
    p.ts.obs_next_idle = now + 0.5;
    if cx.pev().iuser1 == obs::ROAMING || p.ts.obs_target == 0 {
        return;
    }
    let t = engine().player(p.ts.obs_target);
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
    // `respawn` (0xace58): with neither coop nor deathmatch the map reloads instead.
    let g = engine().globals();
    if g.coop == 0.0 && g.deathmatch == 0.0 {
        engine().server_command("reload\n");
    } else {
        crate::ham::player_spawn(p, cx);
    }
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
    // The damage globals as this hit left them (`glast_closecombat`, `glast_coldcock`,
    // `glast_weaponid`, `glast_headshot`), for the blur below and for PlayerKilled.
    let globals = crate::damage::glast::snapshot();

    // Motion blur trails on the killer and off on the victim (50 %). Guard: TS doesn't test
    // for a NULL attacker.
    let attacker = dmg.attacker;
    let blur = crate::gameplay::active().server.motion_blur();
    if blur && !attacker.is_null() && killer_blur(globals.close_combat, || eng.random_long(0, 100)) {
        let org = attacker.v().origin;
        let b = TsMBlur { entindex: eng.index_of(attacker) as u8, mode: 1 };
        msg::pvs(Msg::TsMBlur, org, |w| b.write(w));
    }
    if blur && eng.random_long(0, 100) < 0x32 {
        let b = TsMBlur { entindex: idx as u8, mode: 0 };
        msg::pvs(Msg::TsMBlur, cx.pev().origin, |w| b.write(w));
    }

    p.ts.observer_at = now + 3.0;
    p.hud.cash = None;
    eng.set_view(e, e);
    // `EnableControl(TRUE)` (0x8eee0): a player frozen by a camera or trigger is let go.
    cx.pev().flags &= !(FL_FROZEN as i32);
    p.ts.respawns_used = p.ts.respawns_used.saturating_add(1);
    p.ts.suicide_at = 0.0;
    // `g_pevLastInflictor`, the inflictor while `TakeDamage` kills (NULL for the `kill` command's
    // direct `Killed`): a player inflictor, the victim included, becomes `iuser2` and the
    // spectator target; another one only clears `iuser2`; without one both are left alone.
    let inflictor = dmg.inflictor;
    if !inflictor.is_null() {
        if super::has_player(inflictor) {
            let k = eng.index_of(inflictor);
            cx.pev().iuser2 = k;
            p.ts.obs_target = k;
        } else {
            cx.pev().iuser2 = 0;
        }
    }
    set_respawn_time(p, cx, cvars::respawn_time());
    crate::gameplay::active().server.on_killed(p, cx);

    // Headshot death: slow the victim a little unless `TSGameRules+0x18` is set (0x7f01e file
    // address; 2.1's `+0x14`). Nothing sets it outside `Init`, so global slow motion doesn't stop it.
    if headshot_death_slows(p.ts.last_hitgroup, state::with(|g| g.slow_blocked)) {
        powerup::go_slow_self(p, cx, 0.33, false);
    }
    cx.pev().iuser4 = stunt::DEAD_LOCK;

    // Holster, ammo pack, CTB case, every weapon (0x8f06c..0x8f185). A slow-motion power-up runs
    // out on the next PreThink (`UpdatePowerUpState` while dead), not here.
    crate::weapons::on_killed(p, cx);

    if !state::single_player() {
        scoring::player_killed(p, cx, dmg, globals); // 0x8f1b1 (`CHalfLifeRules::PlayerKilled` 0xde82c is empty)
    }

    // `m_pTank->Use(this, this, USE_OFF)` after PlayerKilled (0x8f1e1).
    crate::entities::stock::drive::release_tank(p, cx);
    // `CSound::Reset` on the player's `soundent` slot (0x8f1f0).
    crate::entities::stock::soundent::reset_player_sound(idx);

    // Death pose: CTSAnimation::Animate(7) (0x8f22e), after all of the above, reads the last
    // damage, hitgroup and attack direction (`g_vecAttackDir`).
    let attack_dir = crate::damage::attack_dir();
    p.stunt.anim.last_damage = p.ts.last_damage;
    p.stunt.anim.last_hitgroup = p.ts.last_hitgroup;
    p.stunt.anim.attack_dir = attack_dir;
    crate::stunt::animate(p, cx, 7);

    // Dying: physics, knockback, HUD.
    {
        let v = cx.pev();
        v.modelindex = eng.model_index(crate::player::PLAYER_MODEL);
        v.deadflag = DEAD_DYING as i32;
        v.movetype = MOVETYPE_TOSS as i32;
        v.flags &= !(FL_ONGROUND as i32);
        v.velocity = death_push(v.velocity, v.size, p.ts.last_damage, attack_dir);
        // TS also needs `pl+0x470 == 0` for the hop. No other instruction in `ts_i386.so`
        // addresses that word (0x8f3c2 is the only one) and the engine zeroes private data, so
        // it is always 0.
        if super::len2d(v.velocity) > 80.0 || p.ts.last_damage > 50 {
            v.velocity[2] = 180.0;
            v.gravity = 0.5;
        }
    }
    // `SetSuitUpdate(NULL, FALSE, 0)` (0x8f3e9): the queued suit sentences are dropped.
    crate::player::suit::set_suit_update(p, cx, None, false, 0);
    p.hud.health = Some(0);
    msg::one(Msg::TsHealth, e, |w| TsHealth { hp: 0 }.write(w));
    msg::all(Msg::ActItems, |w| ActItems { entindex: idx as u8, mask: 0 }.write(w));
    p.ts.ifov = 0;
    cx.pev().fov = 0.0;
    msg::one(Msg::SetFov, e, |w| SetFov { fov: 0 }.write(w));

    let gib = dmg.bits & crate::damage::bits::ALWAYSGIB != 0 || (cx.pev().health < -40.0 && dmg.bits & crate::damage::bits::NEVERGIB == 0);
    // `takedamage` stays on (TS's Killed never clears it; `StartObserver` does), so the body
    // still takes TraceAttacks (the bullet dance twitch).
    if gib {
        cx.pev().solid = SOLID_NOT as i32;
        // `GibMonster()` (0x8f501): the splat and the human gibs.
        crate::entities::stock::monsters::gib_player(e);
        cx.pev().effects |= EF_NODRAW as i32;
    } else {
        let v = cx.pev();
        v.angles[0] = 0.0;
        v.angles[2] = 0.0;
        // `SetThink(&CBasePlayer::PlayerDeathThink)`, `nextthink = time + 0.1` (a double
        // constant, 0x8f550): `Player::think` runs it once then, besides PreThink's calls.
        p.death_think = true;
        v.nextthink = (now as f64 + 0.1) as f32;
    }
    // `CBot::Killed` (0x1006ba00) after `CBasePlayer::Killed`: the bot's answer to its killer.
    crate::bot_backend::killed(p, cx, dmg.attacker);
    // Port addition: plugins hear of the death once the player is back in reach.
    crate::plugins::player_killed(cx.edict, dmg);
}

/// `Killed`'s roll for the killer's TSMBlur (0x8ed73..0x8ede1): `RandomLong(0, 100) <= 0x31`;
/// with `glast_closecombat` a first roll `<= 0x4f` wins, and a miss rolls again with the 0x31
/// test (about 90 % in all).
fn killer_blur(close_combat: bool, mut roll: impl FnMut() -> i32) -> bool {
    (close_combat && roll() <= 0x4f) || roll() <= 0x31
}

/// `CBasePlayer::Killed`'s headshot slow (0x7f00f..0x7f032 file addresses): `m_LastHitGroup ==
/// HITGROUP_HEAD` and `TSGameRules+0x18` clear (`GameRules::slow_blocked`). Global slow motion
/// (`+0x560`) isn't read.
fn headshot_death_slows(last_hitgroup: i32, slow_blocked: bool) -> bool {
    last_hitgroup == hitgroup::HEAD && !slow_blocked
}

/// `CBaseMonster::DamageForce` (0xb2d08): `73728 / (size.x · size.y · size.z) · damage · 5`
/// (computed on the x87 stack, stored as a float), at most 1000.
pub fn damage_force(size: [f32; 3], damage: f32) -> f32 {
    let f = ((73728.0 / (size[0] as f64 * size[1] as f64 * size[2] as f64)) * damage as f64 * 5.0) as f32;
    if f > 1000.0 { 1000.0 } else { f }
}

/// `Killed`'s push (0x8f2a0..0x8f391): `velocity += F · normalize(g_vecAttackDir)`, with
/// `F = -DamageForce(m_lastDamageAmount)`, or -500 once that reaches 500. `g_vecAttackDir`
/// points from the body towards the inflictor, so the body flies away from it.
fn death_push(velocity: [f32; 3], size: [f32; 3], last_damage: i32, attack_dir: [f32; 3]) -> [f32; 3] {
    // Inline `Normalize` (0x8f2a2..0x8f2fa): the length is stored as a float, `1 / length` stays
    // on the x87 stack, the products are stored as floats.
    let a = attack_dir;
    let len = (a[0] as f64 * a[0] as f64 + a[1] as f64 * a[1] as f64 + a[2] as f64 * a[2] as f64).sqrt() as f32;
    let dir = if len == 0.0 {
        [0.0, 0.0, 1.0]
    } else {
        let inv = 1.0 / len as f64;
        a.map(|c| (c as f64 * inv) as f32)
    };
    let force = damage_force(size, last_damage as f32);
    let f = f64::from(if force >= 500.0 { -500.0 } else { -force });
    // The x product stays on the x87 stack for its add (0x8f352 `fsts`); y and z are stored as
    // floats first (0x8f358, 0x8f35c) and added from memory.
    [(velocity[0] as f64 + f * dir[0] as f64) as f32, velocity[1] + (f * dir[1] as f64) as f32, velocity[2] + (f * dir[2] as f64) as f32]
}

/// `CBasePlayer::PlayerDeathThink` (0x8eaec): settle, play out the death animation, then become
/// an observer after 3 s or on any button, leaving a corpse. Every PreThink of a dead player
/// that isn't observing calls it, and so does the player's think (`Player::think`) that `Killed`
/// schedules 0.1 s after the death.
pub fn death_think(p: &mut Player, cx: &mut Cx) {
    let now = cx.time();
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
    // `if (HasWeapons()) RemoveAllItems(TRUE)` (0x8ec15): the gun (and a picked-up
    // `cycler_weapon`) goes on the first death think, with `CurWeapon 0,0,0`.
    if crate::weapons::has_weapons(p) {
        crate::weapons::remove_all_items(p, cx);
    }
    // Stay dying until the death animation finishes, advancing it at the animation rate
    // (`framerate` is the player's time scale, so slow motion stretches it). A sequence with a
    // frame rate of 0 never finishes, as in TS.
    let anim = &p.stunt.anim;
    if cx.pev().modelindex != 0 && !anim.seq_finished && cx.pev().deadflag == DEAD_DYING as i32 {
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

/// Broadcasts `TSPAward` for a player (used by scoring for the killer).
pub fn send_award(idx: i32, award: u8) {
    msg::all(Msg::TsPAward, |w| TsPAward { entindex: idx as u8, award }.write(w));
}

#[cfg(test)]
mod tests {
    use super::{
        damage_force, death_push, headshot_death_slows, killer_blur, player_is_cheating, respawn_countdown_text, scan_c_int, score_frags,
    };

    #[test]
    fn headshot_deaths_slow_during_global_slow_too() {
        // Issue #139: only `TSGameRules+0x18` (never set) stops it.
        use crate::damage::hitgroup;
        assert!(headshot_death_slows(hitgroup::HEAD, false));
        assert!(!headshot_death_slows(hitgroup::HEAD, true));
        assert!(!headshot_death_slows(hitgroup::CHEST, false));
    }

    #[test]
    fn cl_lw_is_read_like_sscanf_i() {
        assert_eq!(scan_c_int("1"), Some(1));
        assert_eq!(scan_c_int("  +1abc"), Some(1));
        assert_eq!(scan_c_int("0x1"), Some(1));
        assert_eq!(scan_c_int("01"), Some(1));
        assert_eq!(scan_c_int("010"), Some(8));
        assert_eq!(scan_c_int("-2"), Some(-2));
        assert_eq!(scan_c_int(""), None);
        assert_eq!(scan_c_int("x"), None);
        // PlayerIsCheating (0x9fae0): anything but 1 forces cl_lw 1.
        assert!(!player_is_cheating("1"));
        assert!(player_is_cheating("0"));
        assert!(player_is_cheating(""));
    }

    #[test]
    fn respawn_countdown_truncates_like_ts() {
        // PreThink 0x90ec4: `(int)(m_flRespawnTime - time)`, truncated, capped at 30.
        assert_eq!(respawn_countdown_text(4.7), (20, "4".to_owned()));
        assert_eq!(respawn_countdown_text(1.0), (20, "1".to_owned()));
        assert_eq!(respawn_countdown_text(45.0), (20, "30".to_owned()));
        assert_eq!(respawn_countdown_text(30.9), (20, "30".to_owned()));
        // The whole last second (not only the last half) says "Press Fire To Play!".
        assert_eq!(respawn_countdown_text(0.9), (30, "Press Fire To Play!".to_owned()));
        assert_eq!(respawn_countdown_text(0.4), (30, "Press Fire To Play!".to_owned()));
    }

    #[test]
    fn score_frags_keep_the_low_16_bits() {
        assert_eq!(score_frags(12.0), 12);
        assert_eq!(score_frags(-3.9), -3, "truncated");
        assert_eq!(score_frags(32767.0), 32767);
        assert_eq!(score_frags(32768.0), -32768, "wraps instead of clamping");
        assert_eq!(score_frags(70000.0), 70000i32 as i16);
        assert_eq!(score_frags(-32769.0), 32767);
        assert_eq!(score_frags(f32::NAN), 0, "0x80000000 keeps 0");
    }

    #[test]
    fn killer_blur_rerolls_after_a_close_combat_miss() {
        let rolls = |seq: &[i32], close: bool| {
            let mut it = seq.iter().copied();
            let mut n = 0;
            let hit = killer_blur(close, || {
                n += 1;
                it.next().unwrap()
            });
            (hit, n)
        };
        assert_eq!(rolls(&[0x31], false), (true, 1));
        assert_eq!(rolls(&[0x32], false), (false, 1));
        assert_eq!(rolls(&[0x4f], true), (true, 1));
        assert_eq!(rolls(&[0x50, 0x31], true), (true, 2), "a miss rolls the 50 % test");
        assert_eq!(rolls(&[0x50, 0x32], true), (false, 2));
    }

    #[test]
    fn death_push_is_away_from_the_inflictor() {
        let stand = [32.0, 32.0, 72.0];
        assert_eq!(damage_force(stand, 30.0), 150.0);
        assert_eq!(damage_force([32.0, 32.0, 36.0], 30.0), 300.0, "a ducked body is lighter");
        assert_eq!(damage_force(stand, 300.0), 1000.0);
        // g_vecAttackDir points at the inflictor (+x): the body goes -x.
        assert_eq!(death_push([10.0, 0.0, 0.0], stand, 30, [1.0, 0.0, 0.0]), [-140.0, 0.0, 0.0]);
        // Capped at 500 from DamageForce 500 up.
        assert_eq!(death_push([0.0; 3], stand, 100, [0.0, -2.0, 0.0]), [0.0, 500.0, 0.0]);
        // A zero direction (no inflictor yet) normalises to straight up: pushed down.
        assert_eq!(death_push([0.0; 3], stand, 10, [0.0; 3]), [0.0, 0.0, -50.0]);
    }
}
