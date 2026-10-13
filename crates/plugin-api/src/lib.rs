//! The C ABI the server library offers server plugins: a port addition, TS has no such API.
//!
//! TS 3.0 kept player state in its C++ `CBasePlayer`, and AMX Mod X's TS module (`tsx`) read and
//! wrote it at fixed offsets. The port keeps that state in Rust objects (an edict's private data
//! is only a slab handle, see `goldsrc::edict`), so plugins reach it through this table instead.
//! Two kinds of plugin use it (docs/implementation/plugin-api.md, bot-plugins.md):
//! * **metamod plugins** such as the AMX Mod X modules in `the-specialists-mod/tsmod-amxx-rs`,
//!   which find the table through `CreateInterface`, and
//! * **`tsmod_*` libraries** that the server itself loads from its `dlls/` directory at
//!   `GameDLLInit` (bot backends such as `tsmod_rcbot`), which get the table from
//!   [`LOAD_SYMBOL`] and hand back a [`Plugin`] descriptor with their callbacks.
//!
//! # Getting the table (metamod plugins)
//! The server library exports tier1's `CreateInterface(name, return_code)` (`mp.dll` always had
//! it; the port adds it to `ts_i386.so`). Called with [`INTERFACE_NAME`] it returns a pointer to a
//! static [`PluginApi`] and writes 0 (`IFACE_OK`) to a non-NULL `return_code`; any other name
//! (older versions' names included) returns NULL and 1 (`IFACE_FAILED`). Look the export up in
//! the already loaded game library (metamod's `GINFO_REALDLL_FULLPATH`, then
//! `dlopen(RTLD_NOLOAD)` / `GetModuleHandle`). Such a plugin has no [`PluginId`]: it passes
//! [`NO_PLUGIN`] where one is asked for.
//!
//! # Loading (`tsmod_*` libraries)
//! At `GameDLLInit` the server loads every library in its own `dlls/` directory named
//! `tsmod_<name>` plus the server library's platform suffix ([`library_suffix`]) and calls its
//! exported [`LOAD_SYMBOL`] ([`LoadFn`]) once. The plugin checks the table's [`PluginApi::size`]
//! and [`PluginApi::version`], fills the [`Plugin`] it is given (its `size` and `api_version` are
//! checked after the call), and returns [`OK`]. Anything else (a library for another platform or
//! architecture, a missing export, a wrong size or version, a non-`OK` return) skips the library
//! with a warning. Libraries stay loaded for the life of the process.
//!
//! # Rules
//! * Call only on the engine's thread, from inside an engine or game callback (metamod hooks,
//!   AMX Mod X natives and forwards, and every [`Plugin`] callback qualify).
//! * Players are entity indices, 1 to `maxClients`. A slot without a connected player gives
//!   [`NOT_PLAYER`].
//! * A plugin can run while the server is inside one of that player's own callbacks (a message
//!   hook during its think, a death forward during its damage). The player object is then out of
//!   reach: [`PluginApi::player_info`] returns the latest snapshot (refreshed at least once a
//!   frame), and setters queue the change until the callback returns and give [`DEFERRED`]. The
//!   server calls the [`Plugin`] player callbacks only once the player is back in reach.
//! * There is no backward compatibility between versions: a plugin built for another
//!   [`VERSION`] doesn't find the table ([`INTERFACE_NAME`] carries the version) and isn't loaded
//!   ([`Plugin::api_version`]). Version 3 redesigned the table (docs/implementation/plugin-api.md
//!   §8 maps version 2 to it).
//!
//! # Entity hooks
//! AMX Mod X's hamsandwich hooks C++ virtual functions by patching vtables. The port's entities
//! are Rust objects without vtables, so the server calls a plugin hook around each "virtual" it
//! has instead, and runs a virtual on request ([`PluginApi::ham_execute`]).
//! * Functions are numbered by hamsandwich's `Ham` enum (`ham_const.inc`, AMX Mod X 1.10:
//!   `Ham_Spawn` = 0, ...). Their arguments are hamsandwich's for that function
//!   (`hook_native.cpp`'s signature, e.g. `takedamage` = `Int_Entvar_Entvar_Float_Int`), in order,
//!   in [`HamCall::args`].
//! * Hooks are per class, like a vtable: [`PluginApi::class_id`] gives the class of a classname,
//!   and every call carries the class of `this`. Classnames sharing one Rust type share a class.
//! * The server calls the hook only for functions turned on with [`PluginApi::ham_watch`].
//! * The hook runs before the function (`post` = 0) and after it (`post` = 1). `this` is in
//!   reach during both (not taken out), so plugins may call the API and `ham_execute` on it.
//!   Before: argument changes in the [`HamCall`] are used, and [`ham::action::SUPERCEDE`] skips
//!   the function.
//! * Every call before is followed by exactly one call after, also when the function was
//!   superseded (or `this` was removed meanwhile: the API then answers [`NOT_PLAYER`] for it).
//!   After: [`HamCall::orig_ret`] is what the function returned, the zero value of its return
//!   kind if it didn't run. **The post call's action decides the return value**:
//!   [`ham::action::OVERRIDE`] returns [`HamCall::ret`], anything else returns `orig_ret`. A
//!   pre call's `OVERRIDE` or `SUPERCEDE` value in `ret` is still there for the post call, which
//!   must return `OVERRIDE` itself to keep it.
//! * Hooks may nest: a hook may call `ham_execute`, deal damage or remove entities, and the
//!   functions that run call the hook again. Hooks nested 8 deep are not called (the function
//!   runs without them), and `ham_execute` at that depth gives [`BUSY`].
//! * `Ham_Keyvalue`'s argument (hamsandwich's int) is kind [`ham::kind::INT`] with `p` the
//!   `KeyValueData *` (valid for the call, strings read-only; `i` is the pointer's low 32 bits).
//!   A pre hook that supersedes it decides `fHandled`.
//! * One hook per server ([`PluginApi::ham_set_hook`]): the AMX Mod X module dispatches to its
//!   plugins.
//!
//! # Bots
//! A plugin offers bots by registering a [`BotBackend`] under a name
//! ([`PluginApi::register_bot_backend`]). The server cvar `ts_bot_backend` (`native`, TS's own
//! bots, or a registered name) picks the backend when a map starts. The server owns the bot quota
//! (`ts_bot_quota`) and `addbot`/`addcustombot`, and asks the active backend to add a bot
//! ([`BotBackend::add_bot`]); the backend makes the player with [`PluginApi::create_bot`] and
//! drives it every frame with [`PluginApi::run_cmd`] (from [`Plugin::start_frame`]).

#![no_std]

use core::ffi::{CStr, c_char, c_int, c_void};

/// The `CreateInterface` name of the table. It carries the version, so a metamod plugin built for
/// another version gets NULL.
pub const INTERFACE_NAME: &CStr = c"TSRS_PluginApi003";
/// [`PluginApi::version`] of this definition (and what [`Plugin::api_version`] must be).
pub const VERSION: u32 = 3;
/// The entry point every `tsmod_*` library exports ([`LoadFn`]).
pub const LOAD_SYMBOL: &CStr = c"TSRS_PluginLoad";
/// The prefix of the libraries the server loads.
pub const LIBRARY_PREFIX: &str = "tsmod_";

/// `TSRS_PluginLoad(api, api_size, info, out, out_size)`: called once, at `GameDLLInit`. `api`
/// is the table (`api_size` = its [`PluginApi::size`]); `info` says who the plugin is (valid for
/// the call; keep [`LoadInfo::id`]); `out` is a zeroed [`Plugin`] of `out_size` bytes whose
/// `size` the server has set. Fill it and return [`OK`]; any other value refuses to load.
pub type LoadFn =
    unsafe extern "C" fn(api: *const PluginApi, api_size: u32, info: *const LoadInfo, out: *mut Plugin, out_size: u32) -> c_int;

/// The file name suffix of a `tsmod_*` library for a platform and architecture: the server
/// library's own naming (`mp.dll`, `mp_amd64.dll`, `ts_i386.so`, `ts_amd64.so`). `None` for a
/// platform the port doesn't build.
pub const fn library_suffix(windows: bool, x86_64: bool) -> &'static str {
    match (windows, x86_64) {
        (true, false) => ".dll",
        (true, true) => "_amd64.dll",
        (false, false) => "_i386.so",
        (false, true) => "_amd64.so",
    }
}

/// [`library_suffix`] of the platform this crate is built for.
pub const LIBRARY_SUFFIX: &str = library_suffix(cfg!(windows), cfg!(target_arch = "x86_64"));

/// Done.
pub const OK: c_int = 0;
/// A setter's player was inside its own callback; the change applies when that returns.
pub const DEFERRED: c_int = 1;
/// Not an entity index of a connected player (or, for the entity functions, not a game entity).
pub const NOT_PLAYER: c_int = -1;
/// An argument is out of range; nothing changed.
pub const INVALID: c_int = -2;
/// TS 3.0 has no such state; nothing changed.
pub const UNSUPPORTED: c_int = -3;
/// The player or entity is inside one of its own callbacks and out of reach for something that
/// must run now ([`PluginApi::run_cmd`], [`PluginApi::ham_execute`]), or hooks are nested too deep;
/// nothing ran.
pub const BUSY: c_int = -4;
/// [`PluginApi::create_bot`]: the server has no free client slot.
pub const FULL: c_int = -5;
/// A name is already taken (a command, cvar or bot backend); nothing changed.
pub const EXISTS: c_int = -6;

/// A loaded plugin, as [`LoadInfo::id`] gives it: passed back to the functions that act for the
/// plugin (its data directory, log prefix, commands, bot backend).
pub type PluginId = c_int;
/// The [`PluginId`] of a plugin the server didn't load (a metamod plugin using `CreateInterface`).
pub const NO_PLUGIN: PluginId = 0;

/// Who the plugin is, given to [`LoadFn`].
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct LoadInfo {
    /// This plugin's id (> 0).
    pub id: PluginId,
    /// `<name>` of `tsmod_<name>`, NUL-terminated.
    pub name: *const c_char,
    /// The full path of the library, NUL-terminated.
    pub path: *const c_char,
}

/// A plugin's description and callbacks, filled by [`LoadFn`]. Every callback is optional and
/// runs on the engine's thread; player indices follow the module's rules.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Plugin {
    /// `size_of::<Plugin>()`: the server sets it before the call and checks it after.
    pub size: u32,
    /// The [`VERSION`] the plugin was built for: must be the server's.
    pub api_version: u32,
    /// Display name and version (NUL-terminated, static): for the log.
    pub name: *const c_char,
    pub version: *const c_char,

    /// `GameDLLInit`, after every plugin has loaded (cvars and commands may be registered here or
    /// in [`LoadFn`]).
    pub game_init: Option<extern "C" fn()>,
    /// The map's precache (`CWorld::Precache`): precache models and sounds here.
    pub precache: Option<extern "C" fn()>,
    /// `ServerActivate`: the map's entities exist. `map` is the map name.
    pub server_activate: Option<unsafe extern "C" fn(map: *const c_char)>,
    /// `ServerDeactivate`.
    pub server_deactivate: Option<extern "C" fn()>,
    /// `StartFrame`, after the server's own bots and before the rules think: bot backends run
    /// their bots ([`PluginApi::run_cmd`]) here.
    pub start_frame: Option<extern "C" fn()>,
    /// `GameShutdown` (the engines that call it).
    pub shutdown: Option<extern "C" fn()>,
    /// A client connected (`ClientConnect`, accepted). Bots made by any backend aren't
    /// "connected"; they get [`Self::client_put_in_server`] only.
    pub client_connect: Option<extern "C" fn(player: c_int, is_bot: c_int)>,
    /// A client (or bot) entered the game and spawned for the first time.
    pub client_put_in_server: Option<extern "C" fn(player: c_int, is_bot: c_int)>,
    /// A client (or bot) is leaving; still a player during the call.
    pub client_disconnect: Option<extern "C" fn(player: c_int, is_bot: c_int)>,
    /// A client command (`argv[0]` the command). Return non-zero to swallow it: the game then
    /// doesn't run it. Called before the game's commands, plugin by plugin in load order.
    pub client_command: Option<unsafe extern "C" fn(player: c_int, argc: c_int, argv: *const *const c_char) -> c_int>,
    /// The player spawned (a first spawn or a respawn).
    pub player_spawned: Option<extern "C" fn(player: c_int)>,
    /// The player was damaged: `attacker` and `inflictor` are entity indices (0 the world, -1
    /// none), `damage` what was asked for, `bits` the `DMG_*` bits, `health` what is left.
    pub player_damaged: Option<extern "C" fn(victim: c_int, attacker: c_int, inflictor: c_int, damage: f32, bits: c_int, health: f32)>,
    /// The player died: `attacker` / `inflictor` as above.
    pub player_killed: Option<extern "C" fn(victim: c_int, attacker: c_int, inflictor: c_int)>,
    /// `killer` (a player) scored a kill of `victim`: `points` the frags it gave (-1 a team
    /// kill, 0 none).
    pub on_kill: Option<extern "C" fn(killer: c_int, victim: c_int, points: c_int)>,
    /// A round started (last man standing) / ended (its winner known, the restart pending).
    pub round_start: Option<extern "C" fn()>,
    pub round_end: Option<extern "C" fn()>,
    /// The game played a sound at `entity` (an index; 0 the world) and `origin` (`pfnEmitSound`,
    /// `pfnEmitAmbientSound`): for bots' hearing. `volume` 0..1, `attenuation` 0..4.
    pub sound: Option<unsafe extern "C" fn(entity: c_int, origin: *const f32, sample: *const c_char, volume: f32, attenuation: f32)>,
    /// The player's weapon in hand changed (`weapon` its TS id, 0 kung fu), or its clip or
    /// reserve did: once a frame at most, from [`PluginApi::player_info`]'s fields.
    pub weapon_changed: Option<extern "C" fn(player: c_int, weapon: c_int)>,
    pub ammo_changed: Option<extern "C" fn(player: c_int, weapon: c_int, clip: c_int, ammo: c_int)>,
}

impl Plugin {
    /// A descriptor with no name and no callbacks, `size` and `api_version` set.
    pub const EMPTY: Plugin = Plugin {
        size: size_of::<Plugin>() as u32,
        api_version: VERSION,
        name: core::ptr::null(),
        version: core::ptr::null(),
        game_init: None,
        precache: None,
        server_activate: None,
        server_deactivate: None,
        start_frame: None,
        shutdown: None,
        client_connect: None,
        client_put_in_server: None,
        client_disconnect: None,
        client_command: None,
        player_spawned: None,
        player_damaged: None,
        player_killed: None,
        on_kill: None,
        round_start: None,
        round_end: None,
        sound: None,
        weapon_changed: None,
        ammo_changed: None,
    };
}

/// A bot backend, registered with [`PluginApi::register_bot_backend`] (the server copies it).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct BotBackend {
    /// `size_of::<BotBackend>()`.
    pub size: u32,
    /// Add one bot: `addbot` / `addcustombot` / the quota. `name` and `model` may be empty (the
    /// backend picks; in teamplay the model is the team); `skill` ≤ 0 means the backend picks.
    /// Make the player with [`PluginApi::create_bot`] and return its index, or ≤ 0 for none.
    pub add_bot: Option<unsafe extern "C" fn(name: *const c_char, model: *const c_char, skill: f32) -> c_int>,
    /// The backend became the active one (a map start selected it).
    pub activate: Option<extern "C" fn()>,
    /// It no longer is (the next map selected another; its bots are being kicked).
    pub deactivate: Option<extern "C" fn()>,
    /// A quota bot of this backend is still connected but its player object is new (the engine
    /// kept the client across a level change): pick it up again. Called once.
    pub adopt_bot: Option<extern "C" fn(player: c_int)>,
}

/// Entity hook types: see the module docs.
pub mod ham {
    use core::ffi::c_int;

    /// [`HamArg::kind`](super::HamArg::kind).
    pub mod kind {
        use core::ffi::c_int;
        /// No value (a void return).
        pub const VOID: c_int = 0;
        /// `i`: int, bool, short.
        pub const INT: c_int = 1;
        /// `f`.
        pub const FLOAT: c_int = 2;
        /// `i`: an entity index (hamsandwich's Entvar, Cbase and Edict); -1 for none.
        pub const ENTITY: c_int = 3;
        /// `v`.
        pub const VECTOR: c_int = 4;
        /// `p`: a `TraceResult *` valid for the call; changes before the function are used.
        pub const TRACE: c_int = 5;
        /// `p`: a NUL-terminated string valid for the call (read-only).
        pub const STRING: c_int = 6;
        /// `p`: an `ItemInfo *` valid for the call.
        pub const ITEMINFO: c_int = 7;
        /// `v` / `f` passed by pointer (hamsandwich's pVector, pFloat): the function's writes
        /// come back in them.
        pub const VECTOR_OUT: c_int = 8;
        pub const FLOAT_OUT: c_int = 9;
    }

    /// What a hook returns.
    pub mod action {
        use core::ffi::c_int;
        /// Carry on; the function's own result stands.
        pub const CONTINUE: c_int = 0;
        /// Before the function only: skip it. The call after still follows, with a zero
        /// `orig_ret`; its action decides the return value.
        pub const SUPERCEDE: c_int = 1;
        /// After the function: return [`HamCall::ret`](super::super::HamCall::ret) instead of
        /// the function's result. Before it, only the `ret` it leaves counts (for the call after).
        pub const OVERRIDE: c_int = 2;
    }

    /// Most arguments any hamsandwich function takes.
    pub const MAX_ARGS: usize = 8;

    /// The plugin's hook: `post` is 0 before the function, 1 after. Returns an [`action`].
    pub type Hook = unsafe extern "C" fn(call: *mut super::HamCall, post: c_int) -> c_int;
}

/// One argument or return value of a hooked function. Only the field its `kind` names is used.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HamArg {
    pub kind: c_int,
    pub i: c_int,
    pub f: f32,
    pub v: [f32; 3],
    pub p: *const c_void,
}

impl Default for HamArg {
    fn default() -> Self {
        HamArg { kind: ham::kind::VOID, i: 0, f: 0.0, v: [0.0; 3], p: core::ptr::null() }
    }
}

/// A call of an entity function, as hooks see it and [`PluginApi::ham_execute`] takes it.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HamCall {
    /// hamsandwich's `Ham` number.
    pub ham: c_int,
    /// The entity the function runs on, and its [`PluginApi::class_id`].
    pub this: c_int,
    pub class: c_int,
    /// The arguments, `args[..argc]`.
    pub argc: c_int,
    pub args: [HamArg; ham::MAX_ARGS],
    /// The return value to use ([`ham::action`]); its kind is the function's return type.
    pub ret: HamArg,
    /// After the function: what it returned.
    pub orig_ret: HamArg,
}

/// Power-up bits (`PwUp` message, `tsconst.inc`'s `TSPWUP_*`).
pub mod powerup {
    pub const SLOWMO: i32 = 1;
    pub const INFAMMO: i32 = 2;
    pub const KUNGFU: i32 = 4;
    pub const SLOWPAUSE: i32 = 8;
    pub const DFIRERATE: i32 = 16;
    pub const GRENADE: i32 = 32;
    pub const HEALTH: i32 = 64;
    pub const ARMOR: i32 = 128;
    pub const SUPERJUMP: i32 = 256;
}

/// Loadout items for [`PluginApi::buy_item`] (`tki`, `CBasePlayer::AddItem`).
pub mod item {
    pub const KEVLAR: i32 = 0x10;
    pub const STEALTH_SHOES: i32 = 0x20;
}

/// [`PluginApi::log`] levels.
pub mod log {
    use core::ffi::c_int;
    pub const INFO: c_int = 0;
    pub const WARNING: c_int = 1;
    pub const ERROR: c_int = 2;
    /// Printed only with `developer` on.
    pub const DEBUG: c_int = 3;
}

/// A player's TS state, as [`PluginApi::player_info`] reports it.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct PlayerInfo {
    /// The player has joined the game (past the team/MOTD menus) and isn't spectating.
    pub joined: c_int,
    /// Alive and not observing.
    pub alive: c_int,
    /// A bot (of any backend: TS's `m_bIsBot`).
    pub is_bot: c_int,
    /// The rules team: 1.. in teamplay (the order of `mp_teamlist` / the map's teams; the CTB
    /// and team-win entities' `team` keys count the same way, from 1), 0 for none or outside
    /// teamplay.
    pub team: c_int,
    /// Carries the capture-the-briefcase case (a carried case has no entity of its own).
    pub has_flag: c_int,
    /// Money (`TSCash`).
    pub cash: c_int,
    /// Free inventory slots (`TSSpace`).
    pub free_slots: c_int,
    /// The award/status byte TS broadcasts with `TSPAward` (`pl+0x74c`): 1 out until the next
    /// round, 2 Killer, 3 Demolition, 4 Specialist, 10 The One, 11 respawn pending, 0 none.
    pub award: c_int,
    /// Kills since the last death (`pl+0x744`).
    pub kill_streak: c_int,
    /// The power-up running now ([`powerup`] bits), 0 for none.
    pub active_powerup: c_int,
    /// Server time (`gpGlobals->time`) at which `active_powerup` ends, 0 for none.
    pub powerup_until: f32,
    /// The power-up held but not yet used, and its value (the `PwUp` message's two fields).
    pub stored_powerup: c_int,
    pub stored_powerup_value: c_int,
    /// Permanent kung fu and superjump picked up this life.
    pub kungfu: c_int,
    pub superjump: c_int,
    /// The player's time scale (`pev->fuser1`): 1 normal, below 1 slowed.
    pub time_scale: f32,
    /// The stunt state TS sends with `TSState`.
    pub stunt_state: c_int,
    /// The active weapon's TS id (`WeaponInfo`; 0 for kung fu) and its clip, reserve ammo,
    /// fire mode and attachment bits (`tsconst.inc`'s `TSA_*`).
    pub weapon: c_int,
    pub clip: c_int,
    pub ammo: c_int,
    pub fire_mode: c_int,
    pub attachments: c_int,
}

/// One weapon a player owns, as [`PluginApi::player_inventory`] lists it.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct WeaponEntry {
    /// TS weapon id (1..37).
    pub weapon: c_int,
    /// Rounds in the clip (a stack's count for grenades and blades).
    pub clip: c_int,
    /// Reserve of the weapon's ammo type (shared by every weapon of that type), 0 if it has none.
    pub ammo: c_int,
    /// Fire mode and owned attachment bits (`TSA_*`).
    pub fire_mode: c_int,
    pub attachments: c_int,
}

/// What a game entity is, for [`EntityInfo::kind`].
pub mod entity {
    use core::ffi::c_int;
    /// Anything else (check its classname).
    pub const OTHER: c_int = 0;
    /// A player.
    pub const PLAYER: c_int = 1;
    /// `ts_groundweapon` or an HL weapon name it stands for: `weapon` is its TS id.
    pub const GROUND_WEAPON: c_int = 2;
    /// `ts_powerup`: `powerup` is its bits.
    pub const POWERUP: c_int = 3;
    /// A thrown grenade, knife or gun: `owner` is the thrower.
    pub const PROJECTILE: c_int = 4;
    /// A gun a player holds or dropped (`weapon_tsgun`, the dropped pack): `owner` its player.
    pub const GUN: c_int = 5;
    /// The capture-the-briefcase case lying in the map (`ts_flag`): `team` the team it belongs to.
    /// A carried case has no entity: see [`PlayerInfo::has_flag`](super::PlayerInfo::has_flag).
    pub const CTB_FLAG: c_int = 6;
    /// A capture point (`ts_briefcase`): `team` the team that may capture there (0 any).
    pub const CTB_BASE: c_int = 7;
    /// A team or score objective (`ts_wingiver`, `ts_scoregiver`).
    pub const OBJECTIVE: c_int = 8;
}

/// A game entity as [`PluginApi::entity_info`] reports it.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct EntityInfo {
    /// [`entity`] kind.
    pub kind: c_int,
    /// TS weapon id ([`entity::GROUND_WEAPON`]), else 0.
    pub weapon: c_int,
    /// [`powerup`] bits ([`entity::POWERUP`]), else 0.
    pub powerup: c_int,
    /// The responsible player (thrower, holder, carrier), 0 for none.
    pub owner: c_int,
    /// Team as in [`PlayerInfo::team`], 0 for none or any.
    pub team: c_int,
    /// It can be picked up or used now (visible and not waiting to respawn).
    pub available: c_int,
}

/// `m_mode` values of [`GameState::mode`] (`CTSGameRules::m_mode`).
pub mod mode {
    use core::ffi::c_int;
    pub const DEATHMATCH: c_int = 0;
    pub const TEAM_DEATHMATCH: c_int = 1;
    /// Unreachable in TS 3.0, listed for completeness.
    pub const ROUND_TEAM_DEATHMATCH: c_int = 2;
    pub const OBJECTIVE: c_int = 3;
}

/// [`GameState::round`] values.
pub mod round {
    use core::ffi::c_int;
    /// The game has no rounds (deathmatch, team deathmatch).
    pub const NONE: c_int = 0;
    /// A round is running ([`GameState::round_ends_at`] tells when its time runs out).
    pub const RUNNING: c_int = 1;
    /// The round is over and the next starts at [`GameState::round_restart_at`].
    pub const OVER: c_int = 2;
}

/// The game's rules and state, as [`PluginApi::game_state`] reports it.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct GameState {
    /// [`mode`].
    pub mode: c_int,
    pub teamplay: c_int,
    /// Last man standing, and The One modes.
    pub last_man_standing: c_int,
    pub the_one: c_int,
    /// The map has a capture-the-briefcase case.
    pub ctb: c_int,
    /// The teams in play (teamplay), else 0.
    pub teams: c_int,
    /// [`round`].
    pub round: c_int,
    pub round_ends_at: f32,
    pub round_restart_at: f32,
    /// The game is over (intermission).
    pub game_over: c_int,
    /// `gpGlobals->time`, `maxClients`.
    pub time: f32,
    pub max_clients: c_int,
}

/// A bot's usercmd for [`PluginApi::run_cmd`]: `pfnRunPlayerMove`'s arguments.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct BotCmd {
    /// View angles (pitch, yaw, roll), as `pev->v_angle`.
    pub angles: [f32; 3],
    pub forward: f32,
    pub side: f32,
    pub up: f32,
    /// `IN_*` bits.
    pub buttons: c_int,
    pub impulse: c_int,
    /// The command's duration in milliseconds (1..255).
    pub msec: c_int,
}

/// A server command a plugin registered: `argv[0]` is the command name.
pub type CommandFn = unsafe extern "C" fn(argc: c_int, argv: *const *const c_char);

/// The table [`INTERFACE_NAME`] and [`LOAD_SYMBOL`] hand out. Every function follows the module's
/// rules. Strings are NUL-terminated and only read during the call; buffers get a NUL-terminated
/// string cut to fit, and the function returns the full length (or a status below 0).
#[repr(C)]
pub struct PluginApi {
    /// `size_of::<PluginApi>()` in the server that built it.
    pub size: u32,
    /// [`VERSION`] of the server that built it.
    pub version: u32,

    // ---- the host -----------------------------------------------------------------------------
    /// The engine's function table (`enginefuncs_t *`, the FWGS headers' layout, as the server
    /// got it) and globals (`globalvars_t *`). Don't write to the table.
    pub engine_funcs: *const c_void,
    pub globals: *mut c_void,
    /// Prints `msg` to the server console as `[tsmod_<name>] msg` ([`log`] level).
    pub log: unsafe extern "C" fn(plugin: PluginId, level: c_int, msg: *const c_char),
    /// The plugin's own data directory, `<gamedir>/tsmod/<name>/` with a trailing slash (created
    /// on first use): waypoints, learned data, configs. [`INVALID`] for [`NO_PLUGIN`].
    pub data_dir: unsafe extern "C" fn(plugin: PluginId, buf: *mut c_char, len: c_int) -> c_int,
    /// The current map's name.
    pub map_name: unsafe extern "C" fn(buf: *mut c_char, len: c_int) -> c_int,
    /// Registers a server command (console, configs, rcon) for the process. [`EXISTS`] if the
    /// server or a plugin already has the name.
    pub register_command: unsafe extern "C" fn(plugin: PluginId, name: *const c_char, f: CommandFn) -> c_int,
    /// Registers a cvar (`pfnCVarRegister`) and returns the engine's `cvar_t *` (read its
    /// `value`/`string`; set it with the engine's `pfnCvar_DirectSet`), or the existing one if
    /// the name is taken. NULL for a bad name.
    pub register_cvar: unsafe extern "C" fn(plugin: PluginId, name: *const c_char, value: *const c_char, flags: c_int) -> *mut c_void,
    /// Registers a bot backend under `name` (not `native`): [`OK`], [`EXISTS`] or [`INVALID`].
    /// Only plugins the server loaded can.
    pub register_bot_backend: unsafe extern "C" fn(plugin: PluginId, name: *const c_char, backend: *const BotBackend) -> c_int,

    // ---- players ------------------------------------------------------------------------------
    /// Fills `out` with `player`'s state: [`OK`] or [`NOT_PLAYER`]. `out` must be valid for
    /// writes; it is left untouched on failure.
    pub player_info: unsafe extern "C" fn(player: c_int, out: *mut PlayerInfo) -> c_int,
    /// Fills `out[..max]` with the weapons the player owns (kung fu not included), by id, and
    /// returns how many it owns (which may be more than `max`), or [`NOT_PLAYER`].
    pub player_inventory: unsafe extern "C" fn(player: c_int, out: *mut WeaponEntry, max: c_int) -> c_int,
    /// Sets the money and updates the client's HUD (`TSCash`).
    pub set_cash: extern "C" fn(player: c_int, cash: c_int) -> c_int,
    /// Sets the free inventory slots (and the walking speed that depends on them).
    pub set_free_slots: extern "C" fn(player: c_int, slots: c_int) -> c_int,
    /// Sets the award/status byte and broadcasts it (`TSPAward`).
    pub set_award: extern "C" fn(player: c_int, award: c_int) -> c_int,
    /// Runs `powerup` (one [`powerup`] bit) now for `duration` seconds, without taking it from
    /// the player's stored power-up. 0 stops the running power-up. A negative duration uses the
    /// power-up's normal length.
    pub run_powerup: extern "C" fn(player: c_int, powerup: c_int, duration: f32) -> c_int,
    /// Slows (or speeds up) the player: time scale `speed` (1 normal, 0 frozen) for `duration`
    /// seconds, and everyone within `aura` units of them too.
    pub set_speed: extern "C" fn(player: c_int, speed: f32, aura: f32, duration: f32) -> c_int,
    /// Time scale of what the player sees of the world (sparks, sounds); negative pauses it.
    pub set_physics_speed: extern "C" fn(player: c_int, speed: f32) -> c_int,
    /// Shows (non-zero) or hides bullet trails for this player only.
    pub set_bullet_trail: extern "C" fn(player: c_int, on: c_int) -> c_int,
    /// Sets the clip and reserve ammo of the player's weapon with TS id `weapon` (a negative
    /// value leaves that count alone) and updates the HUD.
    pub set_weapon_ammo: extern "C" fn(player: c_int, weapon: c_int, clip: c_int, ammo: c_int) -> c_int,

    // ---- entities and the game -----------------------------------------------------------------
    /// The player responsible for game entity `entity` (a thrown knife, a grenade, a placed
    /// C4): their index, 0 if none, or [`NOT_PLAYER`] if `entity` isn't a game entity.
    pub projectile_owner: extern "C" fn(entity: c_int) -> c_int,
    /// Fills `out` with what game entity `entity` is: [`OK`] or [`NOT_PLAYER`] (not a game
    /// entity). Other entities are [`entity::OTHER`].
    pub entity_info: unsafe extern "C" fn(entity: c_int, out: *mut EntityInfo) -> c_int,
    /// `UTIL_IsMasterTriggered(master, activator)`: 1 if the entities named `master` (a `master`
    /// keyvalue: a `multisource` or `game_team_master`) let `activator` (a player, or 0) through,
    /// else 0. An empty name is 1. As in TS, a name that isn't a master counts as triggered.
    pub master_triggered: unsafe extern "C" fn(master: *const c_char, activator: c_int) -> c_int,
    /// Fills `out` with the rules and round state: [`OK`].
    pub game_state: unsafe extern "C" fn(out: *mut GameState) -> c_int,

    // ---- bots ---------------------------------------------------------------------------------
    /// Makes a bot player as TS's `BotSetup` does (a fake client, joined and spawned, in
    /// teamplay on the team `model` names) for the active backend: its index, [`FULL`] or
    /// [`INVALID`] (an empty name, or the caller's backend isn't the active one). `skill` is
    /// only reported.
    pub create_bot: unsafe extern "C" fn(plugin: PluginId, name: *const c_char, model: *const c_char, skill: f32) -> c_int,
    /// Kicks a bot (queued as the server's `kick #<userid>`): [`OK`], [`NOT_PLAYER`], or
    /// [`INVALID`] for a human.
    pub kick_bot: extern "C" fn(player: c_int) -> c_int,
    /// Runs one usercmd for the bot now (`pfnRunPlayerMove`: its PreThink, movement and
    /// PostThink run). With the port's `ts_bot_stop 1` a living bot moves and presses nothing
    /// (it still turns). [`OK`], [`NOT_PLAYER`], [`INVALID`] (a human, or a bad `msec`), [`BUSY`]
    /// (inside one of the bot's own callbacks: call it from [`Plugin::start_frame`]).
    pub run_cmd: unsafe extern "C" fn(player: c_int, cmd: *const BotCmd) -> c_int,
    /// Runs a client command as if the player typed it (`use`, `usepowerup`, `respawn`,
    /// `jointeam 1`, `say hi`): split on spaces, double quotes group. [`OK`] (also for a command
    /// the game doesn't know), [`DEFERRED`], [`NOT_PLAYER`] or [`INVALID`] (empty).
    pub bot_command: unsafe extern "C" fn(player: c_int, line: *const c_char) -> c_int,
    /// Switches to the weapon with TS id `weapon` (0 kung fu) as `weapon_<id>` does: [`OK`],
    /// [`DEFERRED`], [`NOT_PLAYER`], or [`INVALID`] (not owned, or out of range).
    pub select_weapon: extern "C" fn(player: c_int, weapon: c_int) -> c_int,
    /// Adds a weapon to the player's loadout, which TS buys at the next spawn
    /// (`CBasePlayer::BuyArmament`, under the server's weapon restrictions and free slots; TS
    /// 3.0 charges no money): with `attachments` (`TSA_*`), a second copy for an akimbo pair
    /// when `akimbo` is non-zero, and `ammo` purchases of its ammo type. [`OK`], [`DEFERRED`],
    /// [`NOT_PLAYER`] or [`INVALID`].
    pub buy: extern "C" fn(player: c_int, weapon: c_int, attachments: c_int, akimbo: c_int, ammo: c_int) -> c_int,
    /// Adds an [`item`] (kevlar, stealth shoes) to the loadout.
    pub buy_item: extern "C" fn(player: c_int, item: c_int) -> c_int,
    /// Empties the loadout (`rst_cfg`).
    pub clear_loadout: extern "C" fn(player: c_int) -> c_int,
    /// Drops the weapon with TS id `weapon`, or the one in hand for a negative id (`drop`,
    /// `drop_<id>`).
    pub drop_weapon: extern "C" fn(player: c_int, weapon: c_int) -> c_int,

    // ---- entity hooks (module docs) ------------------------------------------------------------
    /// Installs the plugin's hook (NULL removes it). One hook per server; the plugin dispatches.
    pub ham_set_hook: extern "C" fn(hook: Option<ham::Hook>),
    /// Turns calling the hook for function `ham` on (non-zero) or off: [`OK`], or
    /// [`UNSUPPORTED`] if the port has no such function for any class.
    pub ham_watch: extern "C" fn(ham: c_int, on: c_int) -> c_int,
    /// Whether class `class` has function `ham`: 1 (hooks fire, `ham_execute` runs it), 0 (it
    /// exists in TS but the port never calls it for this class: hooks never fire, `ham_execute`
    /// runs the default), or [`UNSUPPORTED`].
    pub ham_supported: extern "C" fn(ham: c_int, class: c_int) -> c_int,
    /// The class of entities linked under `classname` (> 0), or [`INVALID`] for an unknown name.
    pub class_id: unsafe extern "C" fn(classname: *const c_char) -> c_int,
    /// The class of game entity `entity` (> 0), or [`NOT_PLAYER`] if it isn't one.
    pub entity_class_id: extern "C" fn(entity: c_int) -> c_int,
    /// Runs `call.ham` on `call.this` with `call.args`, putting the result in `call.ret`. With
    /// `run_hooks` non-zero the hook runs around it as for a call by the game (hamsandwich's
    /// ExecuteHamB); otherwise it doesn't (ExecuteHam), for this call only: the calls it makes
    /// (a `Killed` from `TakeDamage`) run the hook as usual. `call.class` is ignored. [`OK`],
    /// [`NOT_PLAYER`] (not a game entity), [`INVALID`] (wrong arguments, or an `ENTITY` argument
    /// that is neither -1 nor an entity in use), [`UNSUPPORTED`] or [`BUSY`].
    pub ham_execute: unsafe extern "C" fn(call: *mut HamCall, run_hooks: c_int) -> c_int,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suffixes_follow_the_server_library() {
        assert_eq!(library_suffix(true, false), ".dll"); // mp.dll
        assert_eq!(library_suffix(true, true), "_amd64.dll"); // mp_amd64.dll
        assert_eq!(library_suffix(false, false), "_i386.so"); // ts_i386.so
        assert_eq!(library_suffix(false, true), "_amd64.so"); // ts_amd64.so
    }

    #[test]
    fn the_interface_name_carries_the_version() {
        assert_eq!(VERSION, 3);
        assert!(INTERFACE_NAME.to_bytes().ends_with(b"003"));
    }
}
