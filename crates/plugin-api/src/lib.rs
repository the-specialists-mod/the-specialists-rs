//! The C ABI the server library offers server plugins: a port addition, TS has no such API.
//!
//! TS 3.0 kept player state in its C++ `CBasePlayer`, and AMX Mod X's TS module (`tsx`) read and
//! wrote it at fixed offsets. The port keeps that state in Rust objects (an edict's private data
//! is only a slab handle, see `goldsrc::edict`), so plugins reach it through this table instead.
//! The AMX Mod X modules in `the-specialists-mod/tsmod-amxx-rs` are the client.
//!
//! # Getting the table
//! The server library exports tier1's `CreateInterface(name, return_code)` (`mp.dll` always had
//! it; the port adds it to `ts_i386.so`). Called with [`INTERFACE_NAME`] it returns a pointer to a
//! static [`PluginApi`] and writes 0 (`IFACE_OK`) to a non-NULL `return_code`; any other name
//! returns NULL and 1 (`IFACE_FAILED`). Look the export up in the already loaded game library
//! (metamod's `GINFO_REALDLL_FULLPATH`, then `dlopen(RTLD_NOLOAD)` / `GetModuleHandle`).
//!
//! # Rules
//! * Call only on the engine's thread, from inside an engine or game callback (metamod hooks,
//!   AMX Mod X natives and forwards all qualify).
//! * Players are entity indices, 1 to `maxClients`. A slot without a connected player gives
//!   [`NOT_PLAYER`].
//! * A plugin can run while the server is inside one of that player's own callbacks (a message
//!   hook during its think, a death forward during its damage). The player object is then out of
//!   reach: [`PluginApi::player_info`] returns the latest snapshot (refreshed at least once a
//!   frame), and setters queue the change until the callback returns and give [`DEFERRED`].
//! * Check [`PluginApi::size`] before using a field added after version 1: newer servers only
//!   append fields. Version 2 added the entity hooks ([`ham`]).
//!
//! # Entity hooks (version 2)
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
//!   Before: argument changes in the [`HamCall`] are used, [`ham::action::SUPERCEDE`] skips the
//!   function and returns [`HamCall::ret`]. After: [`HamCall::orig_ret`] is what the function
//!   returned. Either way [`ham::action::OVERRIDE`] makes the server return [`HamCall::ret`].

#![no_std]

use core::ffi::{CStr, c_char, c_int};

/// The `CreateInterface` name of the table (every version; check [`PluginApi::version`]).
pub const INTERFACE_NAME: &CStr = c"TSRS_PluginApi001";
/// [`PluginApi::version`] of this definition.
pub const VERSION: u32 = 2;

/// Done.
pub const OK: c_int = 0;
/// A setter's player was inside its own callback; the change applies when that returns.
pub const DEFERRED: c_int = 1;
/// Not an entity index of a connected player (or, for [`PluginApi::projectile_owner`], not a
/// game entity).
pub const NOT_PLAYER: c_int = -1;
/// An argument is out of range; nothing changed.
pub const INVALID: c_int = -2;
/// TS 3.0 has no such state; nothing changed.
pub const UNSUPPORTED: c_int = -3;
/// [`PluginApi::ham_execute`]: `this` is inside one of its own callbacks (not a hook) and out of
/// reach; nothing ran.
pub const BUSY: c_int = -4;

/// Entity hook types (version 2): see the module docs.
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
        /// Before the function only: skip it and return [`HamCall::ret`](super::super::HamCall::ret).
        pub const SUPERCEDE: c_int = 1;
        /// Return [`HamCall::ret`](super::super::HamCall::ret) instead of the function's result.
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
    pub p: *const core::ffi::c_void,
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

/// A player's TS state, as [`PluginApi::player_info`] reports it.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct PlayerInfo {
    /// The player has joined the game (past the team/MOTD menus) and isn't spectating.
    pub joined: c_int,
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

/// The table [`INTERFACE_NAME`] returns. Every function follows the module's rules.
#[repr(C)]
pub struct PluginApi {
    /// `size_of::<PluginApi>()` in the server that built it.
    pub size: u32,
    /// [`VERSION`] of the server that built it.
    pub version: u32,
    /// Fills `out` with `player`'s state: [`OK`] or [`NOT_PLAYER`]. `out` must be valid for
    /// writes; it is left untouched on failure.
    pub player_info: unsafe extern "C" fn(player: c_int, out: *mut PlayerInfo) -> c_int,
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
    /// The player responsible for game entity `entity` (a thrown knife, a grenade, a placed
    /// C4): their index, 0 if none, or [`NOT_PLAYER`] if `entity` isn't a game entity.
    pub projectile_owner: extern "C" fn(entity: c_int) -> c_int,

    // ---- version 2: entity hooks (module docs) ----
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
    /// ExecuteHamB); otherwise it doesn't (ExecuteHam). [`OK`], [`NOT_PLAYER`] (not a game
    /// entity), [`INVALID`] (wrong arguments), [`UNSUPPORTED`] or [`BUSY`].
    pub ham_execute: unsafe extern "C" fn(call: *mut HamCall, run_hooks: c_int) -> c_int,
}
