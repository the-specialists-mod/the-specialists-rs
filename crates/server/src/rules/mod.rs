//! Game rules: modes (DM, TDM, LMS, The One), spawning, scoring, power-ups, slow motion
//! propagation, chat, cvars. Owner: rules work package.
//! Reference: decomp docs/server/gamerules-entities.md, architecture.md §2, §4.
//!
//! Layout:
//! * `state`: the `GameRules` singleton (`g_pGameRules` + `CTSGameRules`), mode decision, teams.
//! * `cvars`: TS cvar registration. `commands`: client and server commands. `msg`: payload send.
//! * `lifecycle`: connect, put-in-server, InitHUD, spawn, observer/respawn, think, disconnect.
//! * `scoring`: `PlayerKilled` (points, sprees, awards, cash, DeathMsg, ScoreInfo).
//! * `spawnpoint`: `EntSelectSpawnPoint`/`TSSelectSpawnSpot`.
//! * `powerup`: held/active power-ups, slow-motion flow and easing.
//! * `frame`: `StartFrame` (`CHalfLifeMultiplay::Think` + `CTSGameRules::Think`).
//! * `teams`: teamplay assignment and messages.

pub mod commands;
pub mod cvars;
pub mod frame;
pub mod lifecycle;
pub mod msg;
pub mod powerup;
pub mod scoring;
pub mod spawnpoint;
pub mod state;
pub mod teams;

use crate::commands::Args;
use crate::player::Player;
use crate::world::{Cx, DamageInfo};
use goldsrc::edict::EdictPtr;
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::ffi::CStr;

/// Per-player rules state (`CBasePlayer` TS fields; offsets from player-movement.md §8).
#[derive(Default)]
pub struct PlayerRules {
    // ---- HUD / join ------------------------------------------------------------------------
    /// `m_fInitHUD`: send ResetHUD (and InitHUD the first time) on the next think.
    pub init_hud: bool,
    /// `m_fGameHUDInitialized`.
    pub game_hud_initialized: bool,
    /// Put in server (cash given) once.
    pub put_in_server: bool,
    /// Throttle for the respawn countdown HUD text (`+0x754`).
    pub next_hud_msg: f32,
    /// Buttons last frame (`m_afButtonLast`) and the edges this frame.
    pub buttons_last: i32,
    pub buttons_pressed: i32,

    // ---- life / respawn ----------------------------------------------------------------------
    /// LMS: deadline to join this round (`+0x7bc`), respawns used (`+0x7ba`).
    pub join_deadline: f32,
    pub respawns_used: u8,
    /// Death time + 3 s: become an observer (`+0x768`).
    pub observer_at: f32,
    /// Delayed suicide (`kill`, +1 s, `+0x84c`).
    pub suicide_at: f32,
    /// Time of death (death animation wait) and fall speed while airborne (`+0x828`).
    pub died_at: f32,
    pub fall_velocity: f32,
    /// Last model accepted (HLDM model check) and the team name (`m_szTeamName`).
    pub team_name: String,
    /// Spawn protection end (`+0x71c`).
    pub protect_until: f32,
    /// Observer: next allowed button action (`+0x760`), saved mode (`+0x76c`).
    pub obs_next_input: f32,
    pub obs_mode: i32,
    /// Chat flood control (`+0x7c0`).
    pub next_say: f32,

    // ---- scoring -------------------------------------------------------------------------------
    /// Kill spree (`+0x744`), time of the last kill (`+0x748`).
    pub spree: u8,
    pub last_kill: f32,
    /// `TSPAward` status (`+0x74c`).
    pub award: u8,
    /// Team-kill window and count (`+0x7c4`/`+0x7c8`).
    pub tk_until: f32,
    pub tk_count: u8,
    pub is_the_one: bool,
    /// Index of the player that killed us (spectator target).
    pub killer: i32,

    // ---- damage ----------------------------------------------------------------------------------
    pub last_hitgroup: i32,
    pub last_damage: i32,
    /// `PTakeDam` throttle (`+0x750`).
    pub next_ptakedam: f32,
    /// Kevlar item owned (`+0x718 & 0x10`; costs 10 slots).
    pub kevlar: bool,
    /// Bullet dodge window end (`+0x524`, set by the stunt package through `dodge_until`).
    pub dodge_until: f32,

    // ---- power-ups / slow motion -----------------------------------------------------------------
    /// Held (stored) power-up and its seconds (`+0x728`/`+0x730`).
    pub held: u16,
    pub held_secs: i32,
    /// Health decay / The One regen timer (`+0x73c`).
    pub health_timer: f32,
    /// `+use` double tap (`+0x164`).
    pub use_tap_until: f32,
    /// Displayed slow amount (`+0x170`) and bullet-time flag (`+0x175`).
    pub slow_display: f32,
    pub btime: bool,
    /// Activator amounts: self (`+0x17c`), others (`+0x180`); 0 = not an activator.
    pub slow_self: f32,
    pub slow_others: f32,
    /// Next `SlowMotionFlow` (`+0x178`).
    pub next_flow: f32,
    /// Players this activator slowed (bit per player index) and projectiles.
    pub slowed: u64,
    pub slowed_ents: Vec<*mut edict_t>,

    // ---- capture the briefcase ---------------------------------------------------------------------
    /// The `ts_flag` this player carries.
    pub flag: Option<*mut edict_t>,
}

pub fn game_init() {
    cvars::register();
    commands::register_server_commands();
}

/// Precache and install the rules; called from worldspawn's spawn (keyvalues already parsed).
pub fn precache() {
    let e = engine();
    // InstallGameRules: exec game.cfg and run it before deciding the mode.
    e.server_command("exec game.cfg\n");
    e.server_execute();
    state::with(|g| g.install());
    crate::entities::ts::precache();
    for s in [c"items/gunpickup2.wav", c"items/9mmclip1.wav", c"items/weapondrop1.wav", c"weapons/items.wav", c"common/bodysplat.wav"] {
        e.precache_sound(s);
    }
    for s in [
        c"player/headshot.wav",
        c"player/headshot2.wav",
        c"player/kevlarhit.wav",
        c"player/bodyhit1.wav",
        c"player/bodyhit2.wav",
        c"player/bodyhit3.wav",
        c"player/bodyhit4.wav",
        c"player/pain2.wav",
        c"player/pain3.wav",
        c"player/pain4.wav",
    ] {
        e.precache_sound(s);
    }
}

pub fn server_activate() {
    crate::bots::server_activate();
}

pub fn server_deactivate() {
    crate::bots::server_deactivate();
}

/// Called every server frame (`pfnStartFrame`).
pub fn start_frame() {
    frame::think();
}

/// `GetGameDescription` text for the current mode (lib.rs may return it).
pub fn game_description() -> &'static CStr {
    let (_, teamplay, lms, the_one) = state::mode();
    match (teamplay, lms, the_one) {
        (_, _, true) => c"The Specialists (The One)",
        (_, true, _) => c"The Specialists (LMS)",
        (true, _, _) => c"The Specialists (Teamplay)",
        _ => c"The Specialists (DM)",
    }
}

/// worldspawn keyvalues the rules care about (`gamemode`, `mapteams`, `teammodels`,
/// `defaultteam`). Return true if handled.
pub fn world_key_value(key: &CStr, value: &CStr) -> bool {
    let v = value.to_str().unwrap_or("").trim().to_owned();
    match key.to_bytes() {
        b"gamemode" => state::with(|g| g.forced_gamemode = v.parse().unwrap_or(0)),
        b"mapteams" => state::with(|g| g.map_teams = v),
        b"teammodels" => state::with(|g| g.map_team_models = v),
        b"defaultteam" => {
            if v.parse::<i32>().unwrap_or(0) != 0 {
                engine().set_cvar_float(c"mp_defaultteam", 1.0);
            }
        }
        _ => return false,
    }
    true
}

pub fn client_connect(e: *mut edict_t, name: &CStr, addr: &CStr) -> bool {
    lifecycle::client_connect(e, name, addr)
}
pub fn client_disconnect(e: *mut edict_t) {
    lifecycle::client_disconnect(e)
}
pub fn client_kill(e: *mut edict_t) {
    lifecycle::client_kill(e)
}
pub fn userinfo_changed(e: *mut edict_t) {
    lifecycle::userinfo_changed(e)
}

/// Spawn: TS player init, spawn point, power-up/HUD state.
pub fn on_spawn(p: &mut Player, cx: &mut Cx) {
    lifecycle::on_spawn(p, cx)
}

pub fn pre_think(p: &mut Player, cx: &mut Cx) {
    lifecycle::pre_think(p, cx)
}

pub fn post_think(p: &mut Player, cx: &mut Cx) {
    lifecycle::post_think(p, cx)
}

/// The player died (health ≤ 0 in `player::damage`): `CBasePlayer::Killed` + `PlayerKilled`.
pub fn on_killed(p: &mut Player, cx: &mut Cx, dmg: &DamageInfo) {
    lifecycle::killed(p, cx, dmg)
}

pub fn client_command(p: &mut Player, cx: &mut Cx, cmd: &str, args: &Args) -> bool {
    commands::client_command(p, cx, cmd, args)
}

pub fn fill_client_data(p: &mut Player, cx: &mut Cx, cd: &mut clientdata_t) {
    // vuser1[1]: active power-up bits (architecture.md §7.2).
    cd.vuser1[1] = p.active_powerup as f32;
    let _ = cx;
}

// ---- helpers used across the rules modules ------------------------------------------------------

/// `IsAlive()`.
pub fn alive(v: &entvars_t) -> bool {
    v.deadflag == DEAD_NO as i32 && v.health > 0.0
}

/// Runs `f` on another player's Rust object (not the one being dispatched). No engine calls
/// that re-enter the game inside `f`.
pub fn other<R>(e: *mut edict_t, f: impl FnOnce(&mut Player) -> R) -> Option<R> {
    crate::world::with::<Player, R>(e, f)
}

/// Client edicts that have a `Player` object.
pub fn players() -> Vec<*mut edict_t> {
    engine().clients().filter(|&e| is_player(e)).collect()
}

/// True if `e` is a client edict with a Player attached (works for the dispatched one too).
pub fn is_player(e: *mut edict_t) -> bool {
    if e.is_null() {
        return false;
    }
    let v = e.v();
    v.flags & FL_CLIENT as i32 != 0 && e.has_private_data() && engine().string(v.classname) == c"player"
}

pub fn netname(e: *mut edict_t) -> String {
    if e.is_null() {
        return String::new();
    }
    engine().string(e.v().netname).to_string_lossy().into_owned()
}

pub fn eye(v: &entvars_t) -> [f32; 3] {
    [v.origin[0] + v.view_ofs[0], v.origin[1] + v.view_ofs[1], v.origin[2] + v.view_ofs[2]]
}

pub fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

pub fn len2d(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1]).sqrt()
}

pub fn len3(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

/// `pev->frags` and `Player::frags` kept in step (the engine reports `pev->frags`).
pub fn add_frags(e: *mut edict_t, p: Option<&mut Player>, n: f32) {
    let v = e.v();
    v.frags += n;
    let f = v.frags as i32;
    match p {
        Some(p) => p.frags = f,
        None => {
            other(e, |p| p.frags = f);
        }
    }
}
