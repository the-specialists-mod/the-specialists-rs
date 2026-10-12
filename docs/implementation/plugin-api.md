# Server plugin API

TS 3.0 kept player state in its C++ `CBasePlayer`. AMX Mod X's TS module (`tsx`, source in
`amxmodx/modules/ts/tsx`, `NBase.cpp`, `CMisc.h`) read and wrote it at fixed int offsets, and
plugins used fakemeta's `get_pdata_*` / `set_pdata_*` the same way. Those offsets were for TS 2.x
and are wrong for 3.0 anyway. The port keeps the state in Rust objects (`player::Player`,
`rules::PlayerRules`, `weapons::Inventory`), and an edict's private data holds only a slab handle
(design.md §4.2). So the port adds an API that TS doesn't have: a C table of functions that
plugins get from `CreateInterface`. The AMX Mod X modules in
`the-specialists-mod/tsmod-amxx-rs` use it.

| Part | Where |
|---|---|
| The table, constants and contract (append-only, `no_std`) | `crates/plugin-api` (`ts-plugin-api`) |
| `CreateInterface` and the table's `extern "C"` functions | `crates/server-dll/src/exports.rs`, `plugin_api.rs` |
| The logic: argument checks, deferral, snapshots | `crates/server/src/plugin_api.rs` (tests in `plugin_api/tests.rs`) |
| Power-up runs, time scale, `TSSlowMo` override | `crates/server/src/rules/powerup.rs` |
| Free slots, weapon ammo | `crates/server/src/weapons/mod.rs` |
| Projectile throwers | `crates/server/src/weapons/projectiles.rs` (`Throwers`) |
| Private-data guard | `crates/goldsrc/src/edict.rs`, `crates/server/src/world.rs` (`private_size`) |
| Entity hooks (version 2, §7): numbering, marshalling, pre/post calls, `ham_execute` | `crates/server/src/ham/` (`mod.rs`, `table.rs`, `classes.rs`, `names.rs`; tests in `tests.rs`) |
| Calling the plugin's hook (FFI), `ham_execute`'s pointer arguments | `crates/server-dll/src/plugin_api.rs` (`call_hook`, `ham_execute`) |

## 1. Getting the table

The server library exports tier1's `CreateInterface(name, return_code)` on every platform.
`mp.dll` always had it, with nothing registered (0x10028f70). On Linux it is the one export the
port adds to `ts_i386.so`'s list (`stock-entities.md` §7.5). With `TSRS_PluginApi001`
(`ts_plugin_api::INTERFACE_NAME`) it returns a pointer to the static `PluginApi` and writes 0
(`IFACE_OK`) to a non-NULL `return_code`. Any other name, or a NULL name, returns NULL and writes 1
(`IFACE_FAILED`).

A plugin looks the export up in the game library that is already loaded: metamod's
`GINFO_REALDLL_FULLPATH`, then `dlopen(RTLD_NOLOAD)` / `GetModuleHandle`. It checks
`PluginApi::size` before it uses a field added after version 1.

## 2. Rules: snapshots and deferral

Plugins run inside engine callbacks. A plugin can run while the server is inside one of that same
player's callbacks: a message hook (`register_message`, `register_event`) during the player's
PreThink or `UpdateClientData`, or a forward during its damage. The player object is then taken
out of the slab (`world.rs` take-out dispatch), so:

* **`player_info`** reads the object if it can be reached (in its slot, or lent back,
  `world::with_lent`). Otherwise it returns the last snapshot. A snapshot belongs to a user id,
  so a slot's next client never sees the previous client's. If a player can't be reached and has
  no snapshot yet (inside its first spawn), only `time_scale` (`pev->fuser1`) is filled.
  Snapshots are refreshed:
  * at the end of every `PlayerPostThink` (`Player::post_think`),
  * every `StartFrame`, for every player that can be reached (`plugin_api::start_frame`, after
    the bots and the rules),
  * after every change a setter makes, and
  * whenever `player_info` reads a live object.
* **Setters** check the player first (`NOT_PLAYER`: not a connected player, slot outside
  1..`maxClients`), then their arguments (`INVALID`; nothing changes). Then they make the change
  at once if the object can be reached: `OK`. That includes a player lent to its slot, which
  `world::run_now` opens, as TS changes a player inside its own call. Otherwise they queue it
  with `world::defer` and return `DEFERRED`. The queued change runs when the holding dispatch
  returns, and only if the slot still holds a connected player.
* Messages: only `set_award` (`TSPAward`) and `run_powerup` (`PwUpTime`, `TSMBlur`) send messages
  during the plugin's call, as TS's own paths do. Everything else goes out with the player's next
  `UpdateClientData` (`player::hud`, `weapons::net`), through TS's "on change" caches.

## 3. The functions

Addresses are `ts_i386.so` (Ghidra numbering). "pl+" offsets are `CBasePlayer` fields
(player-movement.md §8, gamerules-entities.md §2.2-2.3, weapons.md).

| Function | TS state | Port state and behaviour |
|---|---|---|
| `player_info` | `joined`: past the menus. `cash` pl+0x540. `free_slots` pl+0x548 (`TSSpace`). `award` pl+0x74c (`TSPAward`). `kill_streak` pl+0x744. `active_powerup` pl+0x724, `powerup_until` pl+0x72c. `stored_powerup` / `_value` pl+0x728 / pl+0x730 (`PwUp`). `kungfu` pl+0x74e, `superjump` pl+0x74f. `time_scale` `pev->fuser1`. `stunt_state` pl+0x498 (`TSState`). The weapon fields are what `WeaponInfo` carries (`CTSGun::UpdateClientData` 0x858c8). | `Player::{joined, cash, active_powerup, powerup_until, kungfu, superjump}`, `inv.sv.free_slots`, `ts.{award, spree, held, held_secs}`, `stunt.stance`, `inv.gun` (current id, record clip/mode/owned attachments, reserve of its ammo type). With kung fu: id 0, `clip` = the kung-fu bar as `WeaponInfo` sends it, the rest 0. `powerup_until` is 0 when nothing runs. While a used power-up runs, `stored_powerup` is still that power-up, as in TS (pl+0x728 holds it until it ends). |
| `set_cash` | pl+0x540. `CBasePlayer::GiveCash` (0x9310c) clamps to 0..30000. `TSCash` from `UpdateClientData` (0x91d00) on change. | 0..30000, else `INVALID`. Sets `Player::cash`. `player::hud` sends `TSCash`. |
| `set_free_slots` | pl+0x548. `GetSpeedBySlots` (0x76bd4) and `SetPlayerSpeed` (0x93068). `TSSpace` on change. | 0..81 (81 is an empty inventory, `TSInit`). `weapons::set_free_slots`: the free weight, then `inventory::sync_speed` (walking speed, `m_flPlayerSpeed`, `pfnSetClientMaxspeed`) as after a pickup. The weapons carried don't change, so later pickups and drops count from the new value. |
| `set_award` | pl+0x74c, broadcast as `TSPAward` (MSG_ALL) by `PlayerKilled` (0x895e4) and `UpdateClientData` on change. | 0..255 (the byte). Sets `ts.award`, sends `TSPAward` at once (as scoring does) and updates the HUD cache so it isn't sent twice. TS's own code reads the byte too (the spree awards, LMS's 1/11), so a plugin's value takes part in those. |
| `run_powerup` | `UsePowerUp` (0x95980) and `UpdatePowerUpState` (0x95748): pl+0x724 active, pl+0x72c end. Base durations from `SetupPowerUp` (0x98014). | One bit of slow motion 1, infinite clip 2, slow pause 8, double fire rate 0x10 or superjump 0x100 (`powerup::RUNNABLE`: the bits TS reads while running, `InfiniteClips` 0x75fa8.., the 0.15 gravity). Anything else is `INVALID`. Negative duration: the base length with `pwupmult` 1 (3 s, 3 s, 2 s, 3 s; superjump has none, so `INVALID`). 0 duration or not finite: `INVALID`. Runs it as `UsePowerUp` does: active bits, end time, `PwUpTime`, and `TSMBlur` 1/2 for the slow ones. The held power-up isn't touched (`ts.plugin_powerup`; see Deviations). `powerup` 0 stops the running power-up at the next PreThink, through TS's expiry path. |
| `set_speed` | tsx's `ts_set_speed` wrote TS 2.x slow-motion fields. In 3.0 a time scale for others comes from activators: pl+0x17c self / pl+0x180 others, `SlowMotionFlow` (0x96bb0) every 0.5 s, `DeactivateSlowMotionFlow` (0x96a9c). | `speed` 0..2 (0 means frozen, floored to TS's 0.01 `FixedSlowAmt`, 0x990bc), `aura` ≥ 0 and `duration` ≥ 0, all finite, else `INVALID`. The player becomes an activator for `duration` s (`ts.plugin_speed`, `powerup::plugin_speed_think`): self and others at `speed`. `SlowMotionFlow` slows the players and projectiles within `aura` units that it can also see. On expiry or death everything goes back to 1 as `DeactivateSlowMotionFlow` does. Duration 0 ends a running one. It wins over a slow-motion power-up's amounts while it lasts. |
| `set_physics_speed` | tsx's `ts_set_physics_speed` (TS 2.x). 3.0's client eases its cosmetic clock (particles, sound pitch, tint) toward the `TSSlowMo` amount (client `0x1004bf10`; `UpdateClientData` sends 0.225 / 0.35 / pl+0x170). | Speed up to 2 (NaN or above 2: `INVALID`). It overrides the `TSSlowMo` amount (`ts.physics_speed`, `powerup::hud_slow_amount`). 0 or below pauses: 0.01, because the client reads 0 as 1. Exactly 1 hands the amount back to TS. The override lasts until the next spawn (`TSInit` resets `PlayerRules`). |
| `set_bullet_trail` | None in 3.0 (see Deviations). | `UNSUPPORTED` (after the `NOT_PLAYER` check). |
| `set_weapon_ammo` | `CTSGun` records (`s_weapon_status`, gun+0xd4 reserve per ammo type). `WStatus` / `WeaponInfo` from `CTSGun::UpdateClientData` (0x858c8). | Weapon 1..37 with a table entry (not 29), clip 0..255, reserve 0..32767. Negative leaves that count alone. Out of range: `INVALID`. Not owned, or a reserve for a weapon without an ammo type (blades, grenade): `INVALID` when it runs (`OK` path) or nothing (deferred path). `weapons::set_weapon_ammo`: sets the clip with the akimbo split and the single/akimbo twin as a reload's finish does, and the reserve of the weapon's ammo type (shared with every weapon of that type, as in TS). Then the client gets `WStatus` for each changed record and `WeaponInfo` if the weapon is in hand, on the next update. |
| `projectile_owner` | Grenade `pev->owner`, thrown knife `CTSKnife+0x8c` (`CreateKnife` 0x7d9e4), thrown gun `CTSWorldGun` (dropper). | `weapons::projectiles::thrower_of`: the thrower of a grenade, knife or thrown gun, from a table filled when they are created (by edict index and serial number, so it answers while the projectile's own call is on the stack). The answer is 0 once the thrower has left or the slot holds someone else (`Thrower::get`). Any other game entity: its `pev->owner` if that is a connected player (a held `weapon_tsgun`), else 0. `NOT_PLAYER` for an index that is out of range, free, or has no game private data. |

## 4. The private-data guard

Plugins written for TS's object layout still use fakemeta's pdata natives, and AMX Mod X's
hamsandwich hooks entity vtables. A small block holding only the tag broke both
([#97](https://github.com/the-specialists-mod/the-specialistsmod-rs/issues/97)): offsets ran
past the end of the heap block, and on Windows hamsandwich followed the tag as a vtable pointer
and crashed the server. So every block the game allocates (`goldsrc::edict::set_private_tag`)
looks like this:

* At least `MIN_PRIVATE_SIZE` = 0x80 bytes. Every engine zeroes it (FWGS and ReHLDS `Mem_Calloc`,
  HLDS `calloc`).
* The slab tag (8 bytes) is at `PRIVATE_TAG_OFFSET` = 8. The words at 0 and 0x60, where
  hamsandwich's TS gamedata reads the vtable on Windows and Linux, stay zero.
* Players get `PLAYER_PRIVATE_SIZE` = 0x3000. That covers `CBasePlayer` (0xb8c in `ts_i386.so`,
  0xb78 in `mp.dll`) and `mp.dll`'s `CBot` (0x25f0, `GetClassPtr<CBot>` 0x1006c240), plus the
  5-int Linux shift.
* `weapon_tsgun` gets `TSGUN_PRIVATE_SIZE` = 0x1000. `CTSGun` is 0xa10 bytes, and `tsxaddon.inc`
  writes ints 41, 50-62, 839 and 850: byte 3420 with the Linux shift.
* Thrown knives get `KNIFE_PRIVATE_SIZE` = 0x100. `CTSKnife` is 0xa4 bytes, and the stock `tsx`
  module read int 31 (35 on Linux).

What plugins write there is **ignored by the game**: nothing reads the padding, so
`set_pdata_int` on a TS offset does nothing (it no longer corrupts the heap), and `get_pdata_*`
reads 0 or whatever a plugin last wrote. Use the plugin API for TS state. Writes to bytes 8..16
would break the entity's link to its object; no TS offset a plugin has a reason to write lies
there (it is `CBaseEntity`'s first members after `pev`/the vtable).

**Stock hamsandwich is unsupported.** The port's entities have no C++ vtables. `RegisterHam`
fails cleanly with `[HAMSANDWICH] Failed to retrieve vtable for "player"` and a native error in
the plugin, and the server keeps running. Version 2 of the table offers the same hooks without
vtables (§7), for the `tsmod-amxx-rs` port of hamsandwich.

Save and restore only compare `pvPrivateData` pointers (`goldsrc::ext::saverestore`), so the
block's size and layout don't matter to them.

## 5. Deviations and limits

* The whole API is a port addition. TS 3.0 has no plugin interface.
* `set_bullet_trail` is `UNSUPPORTED`: TS 3.0 has no per-player bullet trails. Its client draws
  `sprites/bullet_trail.spr` trails (`TS_BulletTrail` 0x10046240) only on the client-side bullets
  it simulates. It does that under slow motion (time scale < 0.99) or when the server's
  `realbullet` is on (`SrvSett` key 0, client `0x1018d16c`), which is a server-wide rule that also
  moves the server to simulated bullets. TS 2.x's per-player flag (tsx's `TSX_BTRAIL_OFFSET`) is
  gone.
* `run_powerup` keeps the held power-up, which TS can't express (there the held one is the one
  that runs). A plugin's run ends without clearing the held power-up and without dropping
  anything on death. A pickup during a plugin's run doesn't clear it. A used power-up that was
  running when a plugin starts one is spent, as if it had run out. Plugin runs ignore TS's
  "You can't use Slow-Motion now!" block (`slowmatch`'s TSGameRules+0x18), and they work on dead
  players and observers too, where TS's expiry ends them on the next PreThink.
* `set_speed`'s aura is checked when `SlowMotionFlow` runs (every 0.5 s, the first time on the
  next PreThink). A player caught once stays slowed until the speed ends, as TS's activator list
  only grows. A speed above 1 makes the activator and the caught players faster (the movement
  frame time is scaled by `fuser1`).
* `set_free_slots` doesn't change the weapons carried. The speed follows the new value as TS's
  `GetSpeedBySlots` computes it (330 for The One or an empty inventory).
* `set_weapon_ammo` on a grenade or blade stack sets the count but not the free weight it takes.
* Entity hooks: §7.6.

## 6. Tests

* Unit tests (`cargo test -p ts-server plugin_api`, `throwers`): status codes and their order,
  deferral, the snapshot fallback and its user-id check, every argument range, the power-ups that
  run and their normal lengths, the snapshot fields, `set_weapon_ammo` (owned records, reserve
  types, the akimbo split, the HUD flags), `set_speed`'s floor and end, the `TSSlowMo` override,
  the thrower table, and compile-time checks that the private blocks cover TS's objects and keep
  hamsandwich's slots clear.
* `CreateInterface` out of the engine (ctypes on `ts_amd64.so`): `TSRS_PluginApi001` gives
  `IFACE_OK`, size 88 (64-bit), version 1, and all function pointers set (version 2 makes it
  136). Other names and a NULL
  name give NULL and `IFACE_FAILED`.
* AMX Mod X 1.10 on ReHLDS 3.15 with Metamod-R, Linux and Windows (`ref/amxx-testenv`,
  `ham_test.sma` and `pdata_test.sma`): `RegisterHam(Ham_Spawn, "player")` fails cleanly on both
  (previously a crash on Windows). `get_pdata_int(id, 100)` and `get_pdata_int(id, 0)` read 0
  (previously heap garbage and the tag). Both servers keep running with their bots.
* Export lists: `nm -D --defined-only` of `ts_i386.so` / `ts_amd64.so` gains only
  `CreateInterface`. `objdump -p` of `mp.dll` / `mp_amd64.dll` is unchanged. Version 2 adds no
  export.
* Entity hooks (`cargo test -p ts-server ham`): the 481 `Ham` names and their numbers, and every
  supported function's argument and return kinds against `hooklist`'s signature (and both lists
  against `ref/amxmodx`'s `ham_const.inc` / `hook_native.cpp` when that is checked out next to
  the repository); marshalling of each kind (NaN and unknown entities ignored, NULL as -1, the
  `TraceResult` and `KeyValueData` pointers); watch gating (armed only with a hook installed);
  pre-hook argument changes reaching the function and the post hook; SUPERCEDE with the post call
  still made and a zero `orig_ret`; the post call's action deciding the return; hooks run inside
  `reach` (the lend); the nesting limit for a hook re-running its own function and for
  `ham_execute` from a hook (`BUSY` at depth 8, every level's function run, the stack unwound);
  `ham_execute`'s statuses and their order (`NOT_PLAYER`, `UNSUPPORTED`, `INVALID` for the count,
  a kind, an entity not in use or a missing pointer, `BUSY` for an object held further up);
  ExecuteHam skipping hooks for its own call only; the class registry (one id per type, link
  order, case-sensitive names). `world.rs`: a slot's type, presence and lendability in every state.
* In engine (FWGS dedicated server, `ts_dojo`, four bots, `sv_cheats 1`, `ts_dev_hamlog 1`): the
  log shows Spawn before and after for every map entity and for each player's first spawn and
  respawn (the inner path), TakeDamage with its inflictor, attacker, damage, bits and return, and
  Killed nested inside the killing TakeDamage. Hooks under the AMX Mod X module are for
  `tsmod-amxx-rs` to test.

## 7. Entity hooks (version 2)

AMX Mod X's hamsandwich hooks C++ virtual functions by patching vtables (`RegisterHam`) and calls
them through the vtable (`ExecuteHam`, `ExecuteHamB`). The port's entities are Rust objects in a
slab (design.md §4.2), so the server calls the plugin's one hook around each function it has, and
runs functions on request. The contract is `ts-plugin-api`'s module docs ("Entity hooks").

### 7.1 Numbering and arguments

Functions are numbered by AMX Mod X 1.10's `Ham` enum (`ham_const.inc`, `Ham_Spawn` = 0 through
`Ham_SC_Player_IsObserver` = 480; `ham::NAMES`), and their arguments and return value follow the
signature `hook_native.cpp`'s `hooklist` gives that number (`ham::SIGS`): Entvar, Cbase and Edict
are `ENTITY` (an index, -1 for NULL), Bool and Short are `INT`, pVector and pFloat are
`VECTOR_OUT` / `FLOAT_OUT`, Trace is `TRACE` (a pointer to a real `TraceResult`), Str is `STRING`.
`Ham_Keyvalue`'s int is the `KeyValueData *` (`p`, and `i` its low 32 bits, as hamsandwich hands
plugins the pointer).

### 7.2 Where the hooks run

Each function's callers go through `ts_server::ham` instead of `world::dispatch`:

* **Outer calls** reach the object through its edict (`ham::spawn`, `think`, `touch`, `use_`,
  `take_damage`, ...). If the function is watched and a hook installed, the pre hook runs while
  the object is still in its slot, then `world::dispatch` takes it out and runs the function, and
  the post hook runs once the object is back. They fire only for an object in its slot: one that
  a dispatch further up the stack holds is skipped by `world::dispatch` as well (no call), so
  there is nothing to hook. `TakeHealth` and `TS_GoSlow` don't take the object out (they change
  entvars, or reach a player lent to its slot through `world::run_now`), so they also fire for a
  lent player.
* **Inner calls** happen inside the object's own dispatch, on the `&mut` its caller holds:
  `Player_Jump` and `Player_Duck` from `PreThink`, `Killed` from `CBasePlayer::TakeDamage`, the
  player's respawn `Spawn`, its time-based `TakeHealth`, the HL items' `Respawn`. Each hook call
  lends the object back to its slot and opens it (`world::lend_open`: `lend`, then `run_now`), so
  the hook reaches the entity exactly as for an outer call, and the function goes on with the
  object as the hook left it.
* **Unwatched**, every entry point is the plain call after one test of a bit (`Hooks::armed`, set
  only while a hook is installed and the plugin watches that function).
* The pre hook's argument changes are decoded back into the native arguments (an entity index
  not in use, a NaN or infinite float or vector component, or an unknown use type is ignored,
  with a `[ts] ham error` line for the entity). `ham`, `this`, `class` and `argc` are restored
  after it. The post hook sees the arguments as the function got them.
* The post hook always follows the pre hook, also when the pre hook superseded the function or
  the function could no longer reach its object (removed by the pre hook). `orig_ret` is then
  the zero value. The post call's action alone decides the return value (`OVERRIDE`: `ret`,
  otherwise `orig_ret`); `ret` keeps what the pre hook set, so a module that folds the pre result
  into its post action keeps it.

### 7.3 Re-entrancy

Hooks run with no `RefCell` borrow held and their object in its slot, so a hook may use the
whole API on it, run functions on it or on others (`ham_execute`, which goes through the same
entry points), deal damage, and create or remove entities. A function whose object was removed
by its pre hook doesn't run. `ham_execute` copies the `HamCall` (and a `TraceResult` /
`KeyValueData`) in and writes `ret` back, so passing the hook's own `HamCall` is fine.
`run_hooks = 0` (ExecuteHam) skips the hooks for that call only: a `Killed` that its
`TakeDamage` causes runs them as usual.

Hook calls nest at most `ham::MAX_DEPTH` = 8 deep. Deeper, functions run without hooks and
`ham_execute` refuses with `BUSY`, each with a `[ts] ham error` line, so a plugin that re-runs a
function from its own hook (`ExecuteHamB(Ham_Spawn, id)` in a Spawn hook) stops with an error
instead of overflowing the stack. Calling the plugin's hook is FFI: `ts-server-dll`'s `call_hook`
is the one `unsafe` call, behind the safe `HookFn` the game crate holds.

### 7.4 Classes

`class_id(classname)` gives the class of a classname linked with `link_entities!` (the names
the engine creates through exports), and `entity_class_id` that of a live game entity: one id
per Rust type, numbered in link order from 1 when first asked and stable for the server process
(`ham/classes.rs`). Classnames sharing a type share a class, as hamsandwich's hook on one vtable
covers every name of that C++ class: `func_door` / `func_door_rotating` / `func_water`, the HL
weapon names and `ts_groundweapon`, the four `info_player_team*`. An entity of a type nothing
links gets the next id the first time it is asked about. `world.rs` keeps each object's type in
its slot, so the class is known while the object is out.

### 7.5 The functions

"Fires" is `ham_supported` = 1: the hook runs when the game calls it, and `ham_execute` runs it.
"Execute" is 0: TS has it, the port computes it on request but never calls it as a hookable unit,
so hooks don't fire. Everything else is `UNSUPPORTED`. Addresses are `ts_i386.so`.

| # | `Ham_` | Signature | Fires | Execute | Port |
|---|---|---|---|---|---|
| 0 | `Spawn` | `Void_Void` | all | | `DispatchSpawn`, `ClientPutInServer`, a player's respawn (inner), bots, the game's create helpers (projectiles, `ExplosionCreate`, power-ups, slow-motion points, the CTB case, the sound list) |
| 2 | `Keyvalue` | `Void_Int` | all | | `DispatchKeyValue`'s class part, after the entvars parser (SDK order). A superseding hook's `fHandled` counts. |
| 3 | `ObjectCaps` | `Int_Void` | | all | `levelchange::ts_object_caps` (TS's caps by class, 0 for a name TS doesn't know) |
| 4 | `Activate` | `Void_Void` | all | | `ServerActivate` |
| 6 | `Classify` | `Int_Void` | | all | 2 (`CLASS_PLAYER`) for players, else 0 |
| 8 | `TraceAttack` | `Void_Entvar_Float_Vector_Trace_Int` | all | | `damage::trace_attack` (bullets, melee, explosions, thrown and falling objects). The trace carries `iHitgroup`, `vecEndPos` and `pHit`; changes to the attacker, damage, direction, bits, hit group and end position are used. |
| 9 | `TakeDamage` | `Int_Entvar_Entvar_Float_Int` | all | | `damage::apply`, `ApplyMultiDamage` (`damage::multi::apply`) |
| 10 | `TakeHealth` | `Int_Float_Int` | all | | `items::take_health` (kits, chargers, healing doors, negative `trigger_hurt`, `game_player_hurt`), a player's time-based healing (inner) |
| 11 | `Killed` | `Void_Entvar_Int` | player | others | `rules::on_killed` from `CBasePlayer::TakeDamage` (inner); `iGib` is `DMG_NEVERGIB` / `DMG_ALWAYSGIB` (1 / 2) and back. Others: `CBaseEntity::Killed` (0xabd4c). |
| 12 | `BloodColor` | `Int_Void` | | all | 247 for players, -1 (`DONT_BLEED`) else |
| 25 | `OverrideReset` | `Void_Void` | all | | a global entity restored over this one |
| 32 | `IsAlive` | `Int_Void` | | all | `deadflag == DEAD_NO && health > 0` |
| 33 | `IsBSPModel` | `Int_Void` | | all | `SOLID_BSP` or `MOVETYPE_PUSHSTEP` |
| 36 | `IsInWorld` | `Int_Void` | | all | inside ±4096, slower than 2000 |
| 37 | `IsPlayer` | `Int_Void` | | all | the player class |
| 38 | `IsNetClient` | `Int_Void` | | all | false for `mp.dll`'s `CBot`s (`Player::cbot`) and non-players |
| 41 | `Think` | `Void_Void` | all | | `DispatchThink` |
| 42 | `Touch` | `Void_Cbase` | all | | `DispatchTouch`, `game_zone_player`, the Xen tree |
| 43 | `Use` | `Void_Cbase_Cbase_Int_Float` | all | | `DispatchUse`, every target fired (`fire_targets`), buttons, trains, tanks and their controls, effects |
| 44 | `Blocked` | `Void_Cbase` | all | | `DispatchBlocked` |
| 45 | `Respawn` | `Cbase_Void` | HL items | | `CItem::ItemTouch` (inner); returns the item |
| 48 | `Center` | `Vector_Void` | | all | `(absmin + absmax) / 2` |
| 49 | `EyePosition` | `Vector_Void` | | all | `origin + view_ofs` |
| 55 | `Player_Jump` | `Void_Void` | player | | `stunt::jump` from PreThink while +jump is held (inner) |
| 56 | `Player_Duck` | `Void_Void` | player | | `stunt::duck` from PreThink (inner) |
| 57 | `Player_PreThink` | `Void_Void` | player | | `PlayerPreThink` |
| 58 | `Player_PostThink` | `Void_Void` | player | | `PlayerPostThink` |
| 59 | `Player_GetGunPosition` | `Vector_Void` | | player | `origin + view_ofs` |
| 94 | `TS_BreakableRespawn` | `Int_Int` | `func_breakable`, `func_pushable`, the doors | others (empty, 0xac5ac) | `RespawnEntities` on a round restart (0x77764) |
| 95 | `TS_CanUsedThroughWalls` | `Int_Void` | | all | `entities::ts::can_use_through_walls` (0 for ground weapons, ammo, dropped guns, the case) |
| 123 | `TS_GiveSlowMul` | `Void_Void` | | all | nothing: hamsandwich declares it void, so its `max(fuser1, 0.01)` can't be returned |
| 124 | `TS_GoSlow` | `Void_Float_Int` | all | | `rules::powerup::go_slow` (slow-motion activators, `globalslow`, the slow-motion triggers) |
| 125 | `TS_InSlow` | `Int_Void` | | all | `rules::powerup::entity_in_slow` |
| 126 | `TS_IsObjective` | `Int_Void` | | all | true for `ts_wingiver` / `ts_scoregiver` |
| 127 | `TS_EnableObjective` | `Void_Int` | | all | a wingiver's counter and latch start over; nothing else |
| 128 | `TS_OnFreeEntPrivateData` | `Void_Void` | | all | nothing (TS's is empty, 0xac650) |
| 129 | `TS_ShouldCollide` | `Int_Cbase` | all | | `pfnShouldCollide` |

The rest of TS's gamedata (`virtual.games/ts/offsets-common.txt`) is `UNSUPPORTED`:

* `Precache`, `SetObjectCollisionBox`: the port runs them inside the class's spawn or restore
  and the engine's own bounds code, never as a call of their own.
* The monster, AI and helper virtuals (`DeathNotice`, `TraceBleed`, `IsTriggered`,
  `MyMonsterPointer`, `GetToggleState`, `AddPoints`, `AddPlayerItem`, `GiveAmmo`, `GetDelay`,
  `IsMoving`, `DamageDecal`, `TeamId`, `GetNextTarget`, `FVisible`, `BodyTarget`, ...): the port
  has no function for them, or inlines TS's call where it ports it.
* `Player_ShouldFadeOnDeath`, `Player_ImpulseCommands`, `Player_UpdateClientData`: plain code
  inside PreThink, PostThink and `UpdateClientData`, not yet split out as hookable units.
* Every `Item_*` and `Weapon_*` function, and `TS_Weapon_AlternateAttack`: TS's weapons are one
  `CTSGun` per player, whose state the port keeps in the player's inventory (`weapons::Inventory`),
  not in an entity with its own dispatch.
* `TS_RespawnWait` is `Deprecated` in hamsandwich itself.

### 7.6 Deviations and limits

* The hooks and `ham_execute` are a port addition with hamsandwich's semantics; TS has no plugin
  interface.
* A class is a Rust type, not a C++ class: classnames TS builds from different classes but the
  port from one type share hooks (`info_player_team1..4` with `info_player_deathmatch`), and the
  port's generic types are one class per parameter (`item_healthkit` and `item_battery` differ, as
  in TS).
* Calls the port makes without the hook: a breakable's or door's `Spawn` inside its own
  `BreakableRespawn`, `CTestEffect`'s restore `Spawn`, and `Use` calls the game makes on an object
  it already holds. In TS each would be a virtual call hamsandwich sees.
* Outer hooks don't fire for an object a dispatch further up the stack holds (a `Touch` of a
  player during its own think); the port doesn't run the function then either.
* `TakeHealth`'s bits are always `DMG_GENERIC`, as in every TS caller; changes are ignored.
  `Keyvalue`'s strings are read-only.
* `ham_execute(Ham_TraceAttack)` only adds to the multi-damage, as TS's `TraceAttack` does;
  nothing applies it unless the plugin deals the damage. Of the trace, the port uses `iHitgroup`
  and `vecEndPos`; `flFraction` is 0 and the plane fields are empty.
* `ham_execute(Ham_Killed)` on a non-player runs `CBaseEntity::Killed`, which removes it, also
  for classes whose TS `Killed` does more (the dead monsters).
* `Ham_TS_GiveSlowMul` returns nothing (hamsandwich's signature), so plugins read `fuser1`.
* `ts_dev_hamlog 1|0` (sv_cheats) is a dev aid that installs the server's own hook and logs
  Spawn, TakeDamage and Killed (`[ts ham]` lines). It is refused while a plugin's hook is
  installed, and a plugin's `ham_set_hook` replaces it.
