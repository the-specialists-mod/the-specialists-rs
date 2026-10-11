//! TS 3.0 server cvars (decomp server/architecture.md §2.1), registered in `GameDLLInit` order
//! with the TS names, defaults and flags. Values are read by name, so a cvar another package or
//! the engine registered first still works.

use goldsrc::server::engine;
use std::ffi::CStr;

const SERVER: i32 = 0x4; // FCVAR_SERVER
const UNLOGGED: i32 = 0x100; // FCVAR_UNLOGGED

/// (name, default, flags), in TS registration order. All of these are registered by TS's
/// `GameDLLInit` except the voice pair (`CVoiceGameMgr::Init`) and `ts_bot_quota`/`ts_bot_stop` (this port).
pub const CVARS: &[(&str, &str, i32)] = &[
    ("displaysoundlist", "0", 0),
    ("mp_fragsleft", "0", SERVER | UNLOGGED),
    ("mp_timeleft", "0", SERVER | UNLOGGED),
    ("mp_teamplay", "0", SERVER),
    ("mp_fraglimit", "0", SERVER),
    ("mp_timelimit", "0", SERVER),
    ("mp_friendlyfire", "0", SERVER),
    ("mp_falldamage", "0", SERVER),
    ("mp_weaponstay", "0", SERVER),
    ("mp_forcerespawn", "1", SERVER),
    ("mp_autocrosshair", "1", SERVER),
    ("decalfrequency", "30", SERVER),
    ("mp_teamlist", "hitman;gordon", SERVER),
    ("mp_teammodels", "", SERVER),
    ("mp_teamoverride", "1", 0),
    ("mp_defaultteam", "0", 0),
    ("mp_allowmonsters", "0", SERVER),
    ("mp_chattime", "10", SERVER),
    ("roundtime", "150", SERVER),
    ("respawntime", "3", SERVER),
    ("damagemult", "0.0", SERVER),
    ("slowmatch", "1.0", SERVER),
    ("nopowerup", "0.0", SERVER),
    ("botlearn", "0.0", SERVER),
    ("realbullet", "0.0", SERVER),
    ("usecash", "0.0", SERVER),
    ("ammocount", "0", SERVER),
    ("bottalk", "1", SERVER),
    ("weaponstay", "15", SERVER),
    ("mp_theonemode", "0", SERVER),
    ("mp_theone", "gordon", SERVER),
    ("mp_theothers", "seal", SERVER),
    ("lastmanstanding", "0", SERVER),
    ("spectacularness", "1", SERVER),
    ("customweapons", "", SERVER),
    ("banfortk", "0", SERVER),
    ("botarmor", "0", SERVER),
    // CVoiceGameMgr::Init
    ("voice_serverdebug", "0", 0),
    ("sv_alltalk", "0", 0),
    // Port addition: fake clients for unattended server tests (bots/mod.rs).
    ("ts_bot_quota", "0", 0),
    // Port addition: 1 = bots stand still and hold fire (they still respawn), for scripted tests.
    ("ts_bot_stop", "0", 0),
];

/// TS defines these `cvar_t`s but never registers them, so they're constants (architecture.md
/// §2.1): `initial_cash` "1200", `cashforwin` "10000", `cashforlose` "1200", `respawnnumber`
/// (value 0, clamped to 1), `dmgreport` 0.
pub const INITIAL_CASH: i32 = 1200;
pub const CASH_FOR_WIN: i32 = 10000;
pub const CASH_FOR_LOSE: i32 = 1200;
pub const RESPAWN_NUMBER: f32 = 1.0;

pub fn register() {
    // Skip cvars the engine already defines (FWGS and newer engines have sv_alltalk; old HLDS
    // leaves it to the game DLL, like TS's CVoiceGameMgr), so nothing is registered twice.
    let e = engine();
    for &(name, value, flags) in CVARS {
        let cname = goldsrc::util::to_cstring(name);
        if !e.cvar_ptr(&cname).is_null() {
            continue;
        }
        e.register_cvar(name, value, flags);
    }
}

pub fn f(name: &CStr) -> f32 {
    engine().cvar_float(name)
}

pub fn s(name: &CStr) -> String {
    engine().cvar_string(name).to_string_lossy().into_owned()
}

pub fn on(name: &CStr) -> bool {
    f(name) != 0.0
}

/// `respawntime`, clamped to 5..30 s as TS does in `Killed` and `CTSGameRules::InitHUD`.
pub fn respawn_time() -> f32 {
    f(c"respawntime").clamp(5.0, 30.0).round()
}

/// `roundtime`, clamped to 30..180 s.
pub fn round_time() -> f32 {
    f(c"roundtime").clamp(30.0, 180.0)
}

/// `g_flWeaponCheat`: `sv_cheats` as it was when the map's worldspawn spawned
/// (`CWorld::Spawn` 0xf4450), 0 before the first map.
static WEAPON_CHEAT: goldsrc::Global<std::cell::Cell<f32>> = goldsrc::Global(std::cell::Cell::new(0.0));

/// `CWorld::Spawn` (0xf4450): latch `g_flWeaponCheat` for the map.
pub fn latch_weapon_cheat() {
    WEAPON_CHEAT.0.set(f(c"sv_cheats"));
}

/// TS's cheat gate, `g_flWeaponCheat != 0`: the cheat commands (`fov`, `powerup`, `cashala`),
/// `CheatImpulseCommands` and the `InitHUD` cheat notice. A new `sv_cheats` takes effect on the
/// next map.
pub fn cheats() -> bool {
    WEAPON_CHEAT.0.get() != 0.0
}

/// `sv_cheats` now, for the port's own dev aids (`ts_tp`, `ts_dev_touch`, ...; not TS).
pub fn dev_cheats() -> bool {
    f(c"sv_cheats") != 0.0
}
