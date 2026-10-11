# Design: The Specialists 3.0 in Rust

This repository ports **The Specialists 3.0** (TS) to Rust: the server game library
(`ts_i386.so`/`mp.dll`) and the client library (`client.dll`). The source of truth for behaviour
is the decompilation in [the-specialists-decomp](https://github.com/CrunchyMonkies/the-specialists-decomp)
(referred to below as **the decomp**). Its `docs/` describe every subsystem with addresses,
constants, and payloads; this document says how those pieces are organised in Rust.

The codebase started from the minimal `mymod` workspace
([rust-hlmod](https://github.com/CrunchyMonkies/rust-hlmod)), whose general GoldSrc-in-Rust
design (`docs/rust.md` in the parent workspace) still applies.

## 1. Goals

| Goal | How |
|---|---|
| Behave like TS 3.0 | Port logic and constants from the decomp, including the TS changes to player movement. Deviations are listed in §10. |
| **Wire-compatible with TS 3.0** | Same user messages (names, sizes, registration order, payloads), same events, same `delta.lst`, same `clientdata`/`weapon_data` field usage (§2). An original TS client can join this server; this client can join an original TS server. |
| Linux and Windows | Four targets: `i686-unknown-linux-gnu`, `i686-pc-windows-gnu`, `x86_64-unknown-linux-gnu`, `x86_64-pc-windows-gnu` (§8). |
| Engines | Xash3D FWGS (development, native 64-bit), ReHLDS on pre-anniversary HLDS, and stock Valve HLDS post-anniversary (HL25). Steam GoldSrc clients, post-anniversary (HL25); the pre-anniversary client is out of scope. |
| Validated against real content | `cargo xtask` installs into each engine root with the local TS 3.0 content and runs dedicated servers with bots (§9). |

## 2. Wire compatibility contract

The protocol between the two libraries is exactly TS 3.0's, so either side can be swapped for
the original.

* **User messages:** `shared::protocol::MESSAGES` lists all 64 in TS registration order with TS
  sizes. The server registers them in that order in `GameInit` (`messages::register_all`). Payload
  layouts follow decomp `server/architecture.md` §3.2 and `client/architecture-hud.md` §3. Do not
  rename, reorder or resize them.
* **Events:** the same `events/*.sc` names and the same `fparam`/`iparam`/`bparam` meanings
  (decomp `server/weapons.md` §6.1, `client/rendering-effects-prediction.md` §4.2).
* **Networked fields** (`delta.lst` is TS's original file, byte for byte; both engines accept its two missing commas):
  * `pev->iuser4` / `clientdata.iuser4`: stunt bitfield (`shared::ts::stunt`).
  * `pev->fuser1` / `clientdata.fuser1` / `weapon_data.fuser1`: per-entity time scale.
  * `clientdata.vuser1..4` and `ammo_*`: TS weapon state (decomp `server/architecture.md` §7.2).
  * `weapon_data_t`: a single entry for the TS gun (decomp `server/architecture.md` §7.3).
* **Client commands:** TS's own (`tkw`/`tki`/`tka` loadout lists, `weapon_<n>`, `drop_*`, …; decomp
  `server/architecture.md` §4).

## 3. Repository layout

```
crates/
  goldsrc-sys/   bindgen output from the FWGS headers (per target)
  goldsrc/       safe engine wrappers: server (traces, sounds, events, MsgWriter, cvars, bots),
                 client, edicts (edict.rs), player-move callbacks (pmove.rs), native mouse (platform.rs)
  shared/        runs on both sides: pm (movement), weapons (tables + predicted logic), protocol, ts
    data/        weapons.toml, ammo.toml, combat.toml, assets.toml (decoded from the decomp)
  server/        ts-server: server game logic (rlib, no unsafe)
  server-dll/    ts-server-dll: cdylib -> dlls/ts_i386.so, ts_amd64.so, mp.dll (exports, callbacks)
  client/        ts-client: client game logic (rlib, no unsafe)
  client-dll/    ts-client-dll: cdylib -> cl_dlls/client.so, client_amd64.so, client.dll
xtask/           build, dist, install, server tests
game/ts/         liblist.gam, gameinfo.txt, delta.lst (installed over the TS content)
docs/            design.md (this), implementation/*.md (per subsystem), validation.md
```

## 4. Server architecture

### 4.1 Entry points and dispatch

`server-dll/src/lib.rs` exports `GiveFnptrsToDll`, `GetEntityAPI`, `GetEntityAPI2`,
`GetNewDLLFunctions`, and fills every `DLL_FUNCTIONS` slot. Each callback converts the engine's
raw pointers and is routed to a module of `ts-server` (§4.8 explains the split):

| Callback | Goes to |
|---|---|
| `GameInit` | `messages::register_all`, then `rules`, `weapons`, `stunt`, `bots` `game_init` (cvars, server commands) |
| `Spawn`/`Think`/`Touch`/`Use`/`Blocked`/`KeyValue` | the edict's `Entity` (world dispatch). Keyvalues go to `entities::entvars_key_value` first and only the rest reach the class, as TS `DispatchKeyValue` does. |
| worldspawn spawn | `CWorld::Precache` order: force `sv_gravity`/`sv_stepsize`, `rules::install` (installs the rules, the multiplayer voice icon, queues the server config), soundent and body queue, sentences and texture types, then `precache::{w_precache, client_precache, ts_precache, world_sounds_and_gibs}` (TS's map-wide precaches in TS's order, so model/sound/event indices match), light styles, `sv_zmax`, chapter title, `v_dark`, `mp_defaultteam` |
| `ClientConnect`/`Disconnect`/`Kill`/`UserInfoChanged` | `rules` |
| `ClientPutInServer` | creates the `Player` entity and spawns it |
| `ClientCommand` | `commands::client_command`: weapons, stunt, rules, bots, in that order |
| `PlayerPreThink`/`PostThink` | `Player::pre_think`/`post_think` (hook order in §4.3) |
| `StartFrame` | `rules::start_frame`, `bots::start_frame` |
| `PM_Move`/`PM_Init`/`PM_FindTextureType` | `shared::pm` |
| `UpdateClientData`/`AddToFullPack`/`CreateBaseline` | `net` (+ `Player::fill_client_data`) |
| `GetWeaponData`/`CmdStart` | `weapons` |

### 4.2 Entity framework (`world.rs`, `entities/`)

* One Rust object per game-owned edict, kept in a generational slab. The edict's private data
  holds the handle. `OnFreeEntPrivateData` drops the object.
* **Take-out dispatch:** during a callback the object is moved out of its slot. Engine calls that
  re-enter the game (touch, remove, create, damage to *another* entity) therefore work. Damage to
  *yourself* can't be dispatched; handle it with the `&mut self` you hold. Work for a taken-out
  object can wait for it (`world::defer`); a callback can lend its object back to the slot
  around a call that must reach it (`world::lend`: `world::run_now_or_defer` then runs at once,
  and `world::with_lent` reads it, e.g. `PlayerKilled` naming the thrown knife inside its touch).
* `Entity` trait methods: `key_value`, `spawn`, `think`, `touch`, `use_`, `blocked`,
  `trace_attack`, `take_damage`, `should_collide`. `trace_attack` and `take_damage` default to
  `CBaseEntity`'s (`entities::base_take_damage`); the rest are no-ops.
* `entities::link_entities!` exports one C symbol per classname (through
  `goldsrc::export_entity_class!`, so the export lands in the server library even though it's
  declared in `ts-server`). `fire_targets`/`use_targets`
  implement SDK `FireTargets`; `entities::remove` is the deferred `UTIL_Remove` (`FL_KILLME`) and
  `stock::base::sub_remove` the immediate `SUB_Remove`.
* Stock Half-Life classes live in `entities/stock/`, TS classes in `entities/ts/`.

### 4.3 The player

`player::Player` is the `player` entity. Each subsystem keeps its state in its own struct:

| Field | Type | Owner |
|---|---|---|
| `stunt` | `stunt::StuntState` | movement package |
| `inv` | `weapons::Inventory` | weapons package |
| `ts` | `rules::PlayerRules` | rules package |
| `hud` | `player::hud::HudCache` | rules package |
| `tbd` | `player::water::TimeBasedDamage` | rules package (damage types taken, time-based damage, drown recovery) |
| `bot` | `Option<bots::Bot>` | rules package |
| `drive` | `entities::stock::drive::DriveState` | stock entities (train/tank driving) |

Fields several subsystems read live on `Player` itself: `slow_target`, `active_powerup`,
`powerup_until`, `kungfu`, `superjump`, `slots_used`, `cash`, `team`, `frags`, `deaths`,
`respawn_at`, `joined`. The current time scale is `pev->fuser1` and the stunt state is
`pev->iuser4`, because both are networked.

Hook order, fixed in `player/mod.rs`:

* **spawn:** base player setup, then `rules::on_spawn` (spawn point, loadout), `weapons::on_spawn`, `stunt::on_spawn`, `drive::on_spawn`, `hud::reset`.
* **pre_think:** as in TS `PreThink` (0x908cc), `rules::player_think` first (button edges, `PlayerThink`); during intermission PreThink stops there. Otherwise `drive::pre_think` (train speed control), then `rules` (which runs `hud::update_client_data`, TS's `UpdateClientData`, right after `WaterMove`), `stunt`, `weapons`.
* **post_think:** as in TS `PostThink` (0x91610), during intermission or while dead only the weapon timers run down (plus `rules`); otherwise `drive::post_think` (tank firing / letting go), `weapons` (item post frame, skipped while a tank is in use; once the gun is ready it first runs `player::player_use::impulse_commands`, TS's `ImpulseCommands`: the single `PlayerUse` for every usable entity (stock, TS items, dropped guns), then the spray (201), 99, 204 and the cheat impulses), `stunt`, `rules`.
* **fill_client_data:** `stunt`, `weapons`, `rules`.
* **take_damage:** `player::damage::take_damage`, which on death calls `rules::on_killed`. That in turn calls `weapons::on_killed`. Self-inflicted damage from inside the player's own callbacks (fall, drowning, slime/lava, poison) calls `take_damage` directly, since `damage::apply` can't reach the dispatched entity.

### 4.4 Subsystems and their APIs

* **`stunt`**: `CTSStunt`/`CTSAnimation`. It sets `iuser4` bits and impulses, and picks `sequence`/`gaitsequence`. The physics they trigger runs in `shared::pm`.
* **`weapons`**: the per-player TS gun (one container, 38 weapon records), firing, hit resolution, penetration, `CBulletManager`, grenades, knives, kung fu. API for other subsystems: `give_weapon`, `give_ammo`, `strip_all`, `multiply_clip`, and TS's `weapon_tsgun` item itself (`has_gun`, `give_gun`, `remove_all_items`).
* **`rules`**: game modes, spawn selection, scoring and awards, death and respawn, power-ups (held slot, activation), slow-motion propagation, chat (`say`), cvars, TS map entities (`entities/ts`), HUD sync (`player/hud.rs`), player damage (`player/damage.rs`), drowning and time-based damage (`player/water.rs`).
* **`bots`**: fake clients driven by `pfnRunPlayerMove`. They let a dedicated server be exercised without a human. Controlled by the `ts_bot_quota` cvar.

### 4.5 Damage

`damage::DamageInfo` carries the inflictor, attacker, damage, `DMG_*` bits, hitgroup, direction, position and weapon id. `damage::apply(victim, &info)` dispatches to the victim's `take_damage` (TS `TakeDamage`), or, for a `traced` hit, runs TS's `ClearMultiDamage`, the victim's `trace_attack` (`TraceAttack`, which adds to `damage::multi`) and `ApplyMultiDamage` (one `take_damage` with the sum). `ShootBulletsPlayer` traces every pellet before its one Apply, so a shotgun's pellets on one victim are one hit. Hitgroup multipliers and armour follow decomp `server/weapons.md` §5.9 and `server/player-movement.md` §6.

### 4.6 Messages

* `messages::{one, one_unreliable, all, pvs, pas}(Msg, …)` return a `MsgWriter`; call `.byte()`, `.short()`, …, then `.end()`.
* HUD values are sent from `hud::update_client_data` (TS `CBasePlayer::UpdateClientData`, 0.1 s throttle, InitHUD block included) when they change, like TS's shadow-copy approach; it ends with the gun's own `UpdateClientData` (`weapons::update_client_data`). `fullupdate` and `RemoveAllItems` (`StartObserver`, disconnect) run it inline, as TS does.

### 4.7 Time scale (slow motion)

Every entity has a time scale in `pev->fuser1` (1.0 = normal):

* Player movement multiplies frametime by it and freezes below 0.1.
* Weapon timers, animation `framerate`, and grenade and knife physics are scaled by it.
* The server eases each player's `fuser1` toward `Player::slow_target` at about 7/s, snapping within 0.05.
* Sources of slow motion: power-ups (0.35 / 0.225 for others in line of sight), global slow, slow-motion points.

### 4.8 The `unsafe` boundary

The game crates (`ts-server`, `ts-client`, `shared`) are `#![deny(unsafe_code)]`. All `unsafe`
code is in `goldsrc` (safe wrappers over the engine tables, edicts, player-move callbacks, client
hooks, native mouse) and the two `*-dll` crates (exports and callbacks, which convert raw
pointers to references). Game code keeps `*mut edict_t` as an entity handle and reads it through
`goldsrc::edict::EdictPtr` (`e.v()`). Game crates also can't use `goldsrc`'s engine-call macros
(`eng!`, `cl_eng!`, `ev!`, `efx!`, `tri!`), whose `unsafe` expansion `deny(unsafe_code)` doesn't
see; `xtask/src/lint.rs` checks for them (in `cargo test` and `cargo xtask build`), and so does
clippy's `disallowed-macros` (each game crate's `clippy.toml`, in `cargo xtask lint`). The rules for
working with this are in `AGENTS.md`.

## 5. Map entities

Census of the 97 shipped maps: how many maps use each class. Implement in this order.

| Priority | Classes |
|---|---|
| 1 (most maps) | `worldspawn`, `func_wall`, `info_player_deathmatch`, `func_illusionary`, `light`, `func_breakable`, `light_environment`, `func_ladder`, `info_player_start`, `ambient_generic`, `env_sprite`, `ts_powerup`, `trigger_hurt`, `trigger_multiple`, `func_button`, `func_door_rotating`, `multi_manager`, `ts_model`, `func_water`, `ts_groundweapon`, `func_door`, `info_target`, `env_glow`, `light_spot`, `infodecal` |
| 2 | `trigger_teleport`, `trigger_auto`, `info_teleport_destination`, `func_rotating`, `trigger_push`, `func_train`, `path_corner`, `func_conveyor`, `item_healthkit`, `ts_mapglobals`, `func_wall_toggle`, `trigger_camera`, `trigger_changetarget`, `trigger_relay`, `cycler_sprite`, `info_null`, `env_shake`, `env_explosion`, `multisource`, `trigger_once`, `item_battery`, `env_render`, `func_pushable`, `env_beam`, `info_player_teamspawn`, `ts_groundammo`, `game_text`, `env_fade`, `env_shooter` |
| 3 | HL weapon/ammo names that TS re-points at ground items (`weapon_*`, `ammo_*`: decomp `server/architecture.md` §5), `ts_emitter`/`ts_particle`/`ts_fog` (validate and remove; the client reads them), the remaining TS objective entities (stubs: they're dead in 3.0) |

Unknown classnames make the engine print "No spawn function" and free the entity, so maps
still load.

## 6. Client architecture

`client-dll/src/lib.rs` exports the individual entry points (`Initialize`, `HUD_*`, `IN_*`,
`CL_CreateMove`, `V_CalcRefdef`, `KB_Find`, …), converts the engine's pointers, and delegates to
`ts-client`. It also holds `tempents.rs`, the SDK temp-entity loop over the engine's linked list,
and `vgui.rs`: the `ClientFactory`/`CreateInterface` exports and an `IClientVGUI` object
(`UseVGUI1() == false`). Steam GoldSrc needs it to draw the world, because the port has no VGUI1
viewport to call `VGui_ViewportPaintBackground` as TS's does (issue #62); FWGS ignores it.

| Module | Contents | Package |
|---|---|---|
| `input` | kbuttons (`+alt1` = stunt `IN_ALT1`, `+alt2` = 0x40), `CL_CreateMove`, the anti-script gate | HUD/UI |
| `hud` | message hooks (all 64 must be hooked), HUD state, one immediate-mode draw pass in TS order | HUD/UI |
| `menus` | buy menu (TS's own immediate-mode menu, `tsbuylist.tsa`); TS's VGUI1 team menu, scoreboard and MOTD window, painted with engine fills through `hud::vgui` | HUD/UI |
| `voice` | `CVoiceStatus`: talking players' head icons and labels, the talk icon, scoreboard voice icons and squelch-mode muting, `voice_ban.dt`, `VModEnable`/`vban` | HUD/UI |
| `joystick` | the SDK joystick code TS ships (`IN_JoyMove`, `IN_Commands`), Steam GoldSrc only; devices are read by `goldsrc::joystick` (WinMM / SDL2) | HUD/UI |
| `view` | `V_CalcRefdef`: bob, punch, client-side recoil through `cl_viewangles`, slow-motion tint and `TSFade` | rendering |
| `camera` | third-person camera controls (`CAM_Init`'s `+cam*` commands, `snapto`, the `CAM_Think` mouse and key adjustments); `view` owns `cam_thirdperson` and the `thirdperson` toggle | rendering |
| `demo` | client data in demos (`Demo_WriteBuffer` / `Demo_ReadBuffer`): the zoom target (type 0), the alternate grip (type 1) and the `thirdperson` state (type 2) | rendering |
| `events` | `fire.sc`, `empty.sc`, … handlers: sounds, muzzle flash, shells, impacts, decals | rendering |
| `prediction` | `HUD_PostRunCmd` weapon prediction, Txfer callbacks carrying `iuser4`/`fuser1`/`vuser*` | rendering |
| `effects` | temp entities, TS particle system, BSP-lump map emitters, laser sights, motion-blur ghosts | rendering |
| `spectator` | `CHudSpectator` (HLTV director events, `specmode`) and the spectator view: in-eye camera and gun, `cl_vsmoothing` | rendering |
| `studio` | the TS studio model renderer (`HUD_GetStudioModelInterface`): split torso/legs bones, gait, corpses, ghosts | rendering |
| `state` | values both sides read: time scale, health, dead, weapon, fov | shared |

`HUD_GetStudioModelInterface` installs the TS studio renderer (`studio`), so every studio model
is drawn by the client: the SDK `CStudioModelRenderer` with TS's split torso/legs bones, gait and
renderfx 17 corpses and motion-blur ghosts. The engine side is `goldsrc::ext::studio`.

## 7. Shared crate

* `pm`: TS player movement, an exact port of the server's `pm_shared` with the TS changes (decomp `server/player-movement.md` §1). The client's copy in `client.dll` (and `mp.dll`'s) has the same logic apart from `PM_SpectatorMove` (HLTV camera additions, see `docs/implementation/client-rendering.md`), so one implementation serves both sides. The floating point differs, though: `ts_i386.so` is GCC at x87 extended precision, `client.dll`/`mp.dll` are MSVC at 53 bits with other rounding points, so the code is generic over the build (`pm::Arith`): the server uses `Arith::SERVER` (`mp.dll`'s on Windows, `ts_i386.so`'s elsewhere) and client prediction `Arith::CLIENT` (`client.dll`'s). See `docs/implementation/movement.md`.
* `weapons`: `tables()` parses the embedded TOML once. The predicted gun state machine (fire modes, timers × `fuser1`, spread penalty, seeded pellets) goes here.
* `protocol`: the message table and per-area payload encoders and decoders.
* `ts`: stunt bits, power-up bits, observer modes (TS numbering: in-eye is 5), `IN_ALT1`, the slot/speed formula.

## 8. Build targets and binary compatibility

| Target | Installed as | Used by |
|---|---|---|
| `i686-unknown-linux-gnu` | `dlls/ts_i386.so`, `cl_dlls/client.so` | ReHLDS, HLDS (Linux), Steam HL Linux |
| `i686-pc-windows-gnu` | `dlls/mp.dll`, `cl_dlls/client.dll` | ReHLDS, HLDS (Windows), Steam HL Windows |
| `x86_64-unknown-linux-gnu` | `dlls/ts_amd64.so`, `cl_dlls/client_amd64.so` | FWGS 64-bit |
| `x86_64-pc-windows-gnu` | `dlls/ts_amd64.dll`, `cl_dlls/client_amd64.dll` | FWGS 64-bit Windows |

* **Bindings** are generated per target from the FWGS headers, which `_Static_assert` the 32-bit layouts.
* **Windows** builds use MinGW-w64 (UCRT). `GiveFnptrsToDll` is exported undecorated, and the only imports are system DLLs (`api-ms-win-crt-*`, present on Windows 10+ and Wine).
* **Linux i686** builds currently need glibc 2.34. Building against an older glibc for old HLDS hosts (`cargo zigbuild`) is a later step.
* **Windows export section.** Steam's engine finds the export names by looking for the section named
  `.rdata`, as MSVC lays it out. MinGW's `.edata` breaks that, so the xtask renames the sections of
  every Windows DLL it installs or packages (see `docs/validation.md` §4). DLLs straight out of
  `cargo build` are not patched: use `cargo xtask install`/`dist`.
* `panic = "abort"`. A panic hook writes `ts_server_panic.log` / `ts_client_panic.log`.

## 9. Validation

| Root | Engine | Path (default) |
|---|---|---|
| `fwgs` | Xash3D FWGS, amd64 (`xash` dedicated, `xash3d` client) | `~/games/xash` |
| `rehlds-linux` | ReHLDS (built from `../ReHLDS`) on HLDS `steam_legacy` | `~/games/hlds_linux` |
| `rehlds-win` | ReHLDS 3.15 release on HLDS `steam_legacy`, under Wine | `~/games/hlds_win` |
| `hlds25-linux` | stock HLDS, post-anniversary (`public` branch) | `~/games/hlds25_linux` |
| `hlds25-win` | stock HLDS, post-anniversary, under Wine | `~/games/hlds25_win` |
| `fwgs-win` | Xash3D FWGS 0.21 win32 (from `xash3d-ts`), under Wine | `~/games/xash_win` |

The `cargo xtask` commands:

* `cargo xtask install all` assembles `<root>/ts`. It symlinks the TS content directories and copies config files, so the local TS install is never modified. Then it installs `game/ts` and the libraries for that root's target.
* `cargo xtask server <root> --bots N` starts the dedicated server and checks:
  * that it answers A2S_INFO with the right map,
  * that the bots joined,
  * that the log has the Rust `GameInit` line and no panic or fatal error.
* `cargo xtask test` runs the same check on every root.
* `cargo xtask client <root>` runs that root's dedicated server and joins it with the FWGS
  client and this client library (GoldSrc protocol for ReHLDS and HLDS), saving a screenshot.

The FWGS client can join GoldSrc-protocol servers (`connect host:port gs`), so one FWGS client
with this client library validates the full chain against ReHLDS and HLDS. See
`docs/validation.md` for results.

Test maps: `ts_arena` (6 classes, smoke test), `ts_awaken` and `ts_bay107` (power-ups, ground
weapons), and `ts_lobby` (19 classes, the classic).

## 10. Deliberate deviations from TS 3.0

The port matches TS, including its bugs. The exceptions are out-of-bounds reads and writes that
only happen on bad input, which are guarded instead (each implementation doc lists them):

* the scoreboard overrun (74 writes into a 64-byte array)
* the always-true objective bounds checks

Dead 3.0 features (round-based and objective modes, cash) are implemented only as far as the
port needs them; `docs/implementation/*.md` records what's live.
