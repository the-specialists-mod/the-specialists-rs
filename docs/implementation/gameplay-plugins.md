# Gameplay plugins

The port can run more than one ruleset (issue #100). TS 3.0's rules are the **core plugin**
(`"30"`). Each other ruleset is a separate in-process Rust plugin. The first is TS 2.1
([ts21.md](ts21.md)). The server cvar `ts_gameplay` picks the one a map runs, at the map's start;
changing it reloads the map (§4).

The plugins keep each ruleset's code apart from 3.0's. The 3.0 crates (`shared`, `ts-server`,
`ts-client`) contain no code, constants or `if`s for another ruleset. They call **hooks** at the
points where a ruleset can differ, and each hook's default body is the 3.0 behaviour.

The C plugin API ([plugin-api.md](plugin-api.md), for AMX Mod X) is a different thing. It is a
narrow table of player setters, and it can't reach movement, prediction or the HUD.

## 1. Layout

```
crates/
  gameplay/   ts-gameplay: ids, the channels that carry them, the generic Registry<P>,
              info-string lookup. No dependencies, so it sits below shared.
  shared/src/gameplay/
    mod.rs    Core (the 3.0 plugin object, CORE), re-exports
    pm.rs     PmRules: movement hooks (shared::pm)
    gun.rs    GunRules: gun hooks (shared::weapons::gun, through GunHost::rules)
  server/src/gameplay/
    mod.rs    ServerPlugin { pm, gun, server } and CORE_PLUGIN, the server registry, active(),
              the per-call resolvers, the latch at a map start (map_select, map_start,
              wanted_id), the client check (refuse_client)
    server.rs ServerRules: server-only hooks; ConnectingClient
  server/src/reload.rs
              the reload on a change of ts_gameplay or ts_bot_backend (§4)
  client/src/gameplay/
    mod.rs    ClientPlugin { pm, gun, client } and CORE_PLUGIN, the client registry, resolvers
    client.rs ClientRules: client-only hooks (HUD, input)
  ts21/       the TS 2.1 plugin (ts21.md)
    src/shared/  ts21-gameplay: Ts21 (impl PmRules, GunRules), ID "21"
    server/      ts21-server: impl ServerRules, the server's PLUGIN
    client/      ts21-client: impl ClientRules, the client's PLUGIN
  server-dll/, client-dll/   register "30" (CORE_PLUGIN) and, with the default-on feature `ts21`,
                             "21" (ts21_server::PLUGIN / ts21_client::PLUGIN)
```

**Why each trait lives with its types.** Issue #100 §1.0 first put all four traits in
`ts-gameplay`. That can't work with a one-way dependency:

* `shared`'s movement and gun code call `PmRules` and `GunRules`, so those traits must be in
  `shared` or below it.
* A hook's 3.0 body is the code it replaces, and that code takes the owning crate's types. For
  example, `ServerRules::points_for_frag` takes the server's `DamageInfo` and damage snapshot,
  and its default calls `rules::scoring::points_for_frag`.

So each trait is defined in the crate that owns its types, with default methods holding the 3.0
behaviour. `ts-gameplay` keeps what every side shares and what names no game type: the ids and
the registry. `Core` implements every trait with the defaults. A plugin implements the same
traits and overrides only what its ruleset changes. Each trait has its own module file, so work on
different areas touches different files.

**Why a plugin is a struct of hook objects, in one package per side.** On each side a plugin is
`ServerPlugin { pm, gun, server }` or `ClientPlugin { pm, gun, client }`. A plugin's server
hooks and client hooks are separate packages (`ts21-server`, `ts21-client`), not features of one
package. Cargo unifies a package's features across a build of both libraries (`cargo xtask build`
builds them together). With features, the plugin would get both, and the server crate would be
linked into `client.so` (which then exported the server's 189 entity classes) and the client
crate into the server library. Separate packages need separate types for the side hooks (the
orphan rule), and the struct lets them share the one `Ts21` object for the shared hooks without
delegation.

`Registry<P>` is generic over the plugin type, so another independent axis can be added later
with its own registry and cvar. One example is a bot plugin, chosen apart from the gameplay
ruleset. Bots aren't part of a gameplay ruleset, so `ServerRules` has no bot hooks. A ruleset that changes TS's own bots registers
`NativeBotRules` with the native bot backend's registry under its gameplay id instead
([bot-plugins.md](bot-plugins.md) §2.2; TS 2.1's: `ts21_server::bots::BOTS`).

Every plugin object is a `&'static` value, `Sync`, and stateless. Plugin state goes in the
plugin's own statics (`goldsrc::Global` with a `Cell`/`RefCell`, as the game crates do), and
`on_activate` sets it up (§4).

## 2. Selection and the channels

| What | Server | Client |
|---|---|---|
| The choice | `ts_gameplay` (`FCVAR_SERVER`, default `"30"`, port addition). Read at each map start; a change reloads the map (§4). | — |
| Shared code (movement, gun) | Physinfo key `gp`, set at every spawn next to `mj`/`hl` (`rules/lifecycle.rs`, `stunt/mod.rs`, `player/save.rs`) | `pmove->physinfo` / `clientdata_t::physinfo` (`gp`) |
| HUD | — | `SrvSett` key 4, value = the id as a number. Sent in `InitHUD` after keys 0..3. TS 3.0's client ignores it (its handler, client.dll 0x1004b840, switches on keys 0..3 only). |
| Versions (port addition, `shared::protocol::port`) | The port client's userinfo key `_tsrs` (its version; §4.2) | `SrvSett` keys 5, 6, 7: the server library's major, minor and patch version, after key 4. Shown in the buy menu (§4.3). |

Ids are short decimal numbers (`ts_gameplay::srvsett_value`), because the HUD channel is a
`short`. `Registry::add` refuses any other id. An empty id resolves silently to the core
plugin. So does every id when the server is TS 3.0's or an older port, since neither sends `gp`
nor key 4. An **unknown** id resolves to the core plugin, and the console gets a warning once per
value: `[ts] gameplay: unknown ruleset "99" ...` on the server, or "the server's ruleset ... isn't
in this client" on the client.

## 3. Where the hot paths resolve the rules

Each path resolves the rules once per call and passes a `&'static dyn` trait object down. No
helper looks them up again.

| Path | Resolved by | From |
|---|---|---|
| Server `pfnPM_Move` (`server-dll`) | `ts_server::gameplay::pm_rules(pm)`, then passed to `shared::pm::player_move(pm, arith, rules)`, which keeps it in `Pm::rules` | the moving player's `gp` |
| Client `HUD_PlayerMove` (`client-dll`) | `ts_client::gameplay::pm_rules(pm)`, then `player_move_with(pm, arith, rules, spectator)` | `gp` |
| Server gun | `weapons::host::with_host` sets `ServerHost::rules` from `gameplay::gun_rules(e)`, and the gun reads `GunHost::rules()` | the player's `gp` (`pfnGetPhysicsKeyValue`) |
| Client weapon prediction | `prediction::gun::post_think` sets `ClientHost::rules` from `gameplay::gun_rules(&from.client)` | `from`'s `gp`, per command |
| Server frame, rules, scoring | `ts_server::gameplay::active().server` (a `Cell` read) | `ts_gameplay` at the map start |
| Client HUD | `ts_client::gameplay::hud_rules(h.gameplay)` | `SrvSett` 4 (`Hud::gameplay`) |
| Client input, FOV, map effects, buy menu | `ts_client::gameplay::current()` (reads `Hud::gameplay` without holding the HUD's borrow) | `SrvSett` 4 |

Movement and the gun read physinfo instead of the cvar so that the server and the client's
prediction run the same rules for the same command, as they must.

## 4. The ruleset is chosen at a map start

`ts_gameplay` is read only at a map start (issue #100 §1.1, §1.3). A new map is the fresh state:
the engine reloads the map, the rules are built again and every client goes through
`ClientConnect`, `ClientPutInServer` and `InitHUD` again, so nothing carries over between
rulesets and nothing is ever converted. There is no reset of a running game: a change reloads
the map instead.

* **Map start.** `rules::install` (worldspawn's `InstallGameRules`, after `exec game.cfg` has run)
  calls `gameplay::map_select()`, which makes the plugin `ts_gameplay` selects active for the
  whole map before the rules are built (so its `ServerRules::install` decides them), with its
  weapon tables, and logs `[ts] gameplay: ruleset N for this map`. After the rules,
  `gameplay::map_start()` calls its `on_activate()`.
* **A change reloads the map** (the user's decision, #100). `reload::frame()` (start of
  `rules::start_frame`) compares, every frame, the ruleset `ts_gameplay` resolves to
  (`gameplay::wanted_id`) and the backend `ts_bot_backend` resolves to (`bot_backend::wanted`,
  [bot-plugins.md](bot-plugins.md) §3) with what the map runs. When either differs at one frame
  and still differs at the next, the console says `[ts] gameplay: ruleset 21: reloading ts_dojo`
  (and `[ts] bots: backend <name>: reloading <map>` for the backend) and the server changes level
  to the same map (`pfnChangeLevel`), whose map start reads both cvars as above. It doesn't
  matter where the change came from: the console, rcon, the server config, a script.
  * *Debounced:* a config that sets both cvars, or sets one and then back, reloads once or not at
    all; the reload is for the values of the second frame.
  * *Resolved values:* an unknown id is the core plugin (with the warning of §2) and an unknown
    backend native, so a value that falls back to what runs does nothing and can't reload in a
    loop. `ts_bot_backend NATIVE` while native runs is no change.
  * *One per map:* once a reload is asked for, nothing more until the next map start.
  * *No loop through a map start:* if the map a reload started still runs something else
    (`game.cfg`, which runs inside the map start, sets the value back), the console says
    `[ts] the map start set ruleset 30 / bot backend native (game.cfg?), not 21 / native; not
    reloading for that again`, and that selection isn't reloaded for until the cvars select
    something else.
  * *Timing:* FWGS defers `pfnChangeLevel` to the end of the host frame (`COM_ChangeLevel`), so
    the reload happens even while a `wait` script runs. The Steam engines (HLDS, ReHLDS, HL25)
    queue `changelevel <map>` in the console buffer (`PF_changelevel_I`), so there it runs after a
    running `wait` script (AGENTS.md pitfalls); from the console, rcon or a config it runs at once.
* **The server config.** The engines run the server config (`servercfgfile`, `lservercfgfile` on
  a listen server) only once the map has loaded (TS's `CHalfLifeMultiplay` constructor queues
  it), after `map_select`. A `ts_gameplay` it sets is a change like any other, so the first map
  is reloaded under it before anyone has joined; when the config runs again on the reloaded map
  (and every later one), it sets what the map already runs and nothing happens. FWGS's dedicated
  server also runs the server config once before the first map, so there the first map already
  starts under it and isn't reloaded (`cargo xtask plugins` sees both).

So `ts_gameplay` can be set in `game.cfg` (it applies to the map being loaded), in the server
config (through `+servercfgfile`), at the console or by rcon (each reloads the current map). On
the command line, `+ts_gameplay` doesn't work, as for every game cvar (AGENTS.md pitfalls).

On the client, each `InitHUD` clears the ruleset the HUD holds (`Hud::gameplay`), and the
`SrvSett` 4 after it records the map's; a new map already resets the client (`HUD_VidInit`). The
client logs `[ts] gameplay: the server runs ruleset N` when it differs from the last one it saw.
The value isn't cleared in `HUD_VidInit`, which the engine also calls on a video mode change
(FWGS `SCR_VidInit`, e.g. a window resize).

### 4.1 What a map change keeps

Cvars keep their values (a map start runs `game.cfg` and the server config again). Bots the
engine keeps across a level change stay, and the quota picks them up (`bot_backend::quota`).
Client-side settings that aren't game state (voice) stay.

Tested in-engine on FWGS listen servers on `ts_dojo` (`cargo xtask play`, "Gameplay reload"),
and the server config's reload (for `ts_bot_backend`, with `ts_gameplay` set and back) on every
engine's dedicated server (`cargo xtask plugins`). `cargo xtask ts21` (issue #103,
[validation.md](../validation.md) §2.y) checks `ts_gameplay 21` from the server config (one
reload on the Steam engines, none on FWGS) and from rcon (one reload each way) on every engine's
dedicated server, then 2.1's team modes, and that TS 3.0's own client is refused by ReHLDS and
HL25 (§4.2).

### 4.2 Only the port's client plays a ruleset other than 3.0

TS 3.0's own client predicts 3.0's movement and weapons and ignores `SrvSett` 4, so it can't
play another ruleset (issue #100 §1.1). The port's client registers the `FCVAR_USERINFO` cvar
`_tsrs`, whose value is its version (`shared::protocol::port::USERINFO_KEY`), so its userinfo
carries the key and TS 3.0's doesn't. The engines strip `_` keys only from the userinfo they send
to other clients (FWGS `SV_FullClientUpdate`, ReHLDS `Info_RemovePrefixedKeys`), not from the
server's copy.

In `ClientConnect`, which the engines run for every client's `new` (FWGS `SV_New_f`, ReHLDS
`SV_ConnectClient`'s continuation), again at every map change, and never for their own fake
clients, `gameplay::refuse_client` classifies the client (`ConnectingClient::classify`):

| Client | Seen as |
|---|---|
| A bot: `FL_FAKECLIENT` on the edict, or a `*bot` userinfo key (a bot plugin's `ClientConnect`) | `Bot` |
| An HLTV proxy: `*hltv` (HLDS, ReHLDS) or `hltv` (FWGS) set | `Hltv` |
| `_tsrs` set | `Port(version)` |
| anything else (TS 3.0's client) | `Other` |

and asks the active ruleset (`ServerRules::refuse_client`). TS 3.0's answer is always none. TS
2.1's refuses `Other` with `Invalid Version: Running 2.1 mode. Upgrade to the rust port at
tsmod.org`. A refused client gets the text through the engine's reject buffer (`szRejectReason`,
128 bytes, written by `server-dll`'s `client_connect`; FWGS and ReHLDS print it to the client and
drop it, HLDS `echo`es it), before TS's part of `ClientConnect` counts it. The server logs
`[ts] gameplay: refused a client in ruleset 21 (Other): ...`.

### 4.3 Versions in the buy menu

While the buy menu (TS's weapon selection) is open, its bottom right corner shows three rows in
the menu's own style (the dimmed row colour box, the HUD font): the server's ruleset
(`ClientRules::ruleset_name`: `TS 3.0`, `TS 2.1`), `Client <version>` (this library's) and
`Server <version>` (`SrvSett` 5..7) or `Server version unknown` (TS 3.0's server, an older port).
The box stays right of the menu's columns (`menus::version_box`, x past `0x188 + 128`) and
above the spectator bar (`YRES(40)`), which is always up while the menu is open (dead or
observing).

## 5. The hook contract

* **The default body is TS 3.0's exact behaviour.** That includes the floating point: a hook
  whose result feeds the movement's `Arith` maths returns the value the 3.0 code used as a
  constant (`PmRules::recover_input_scale` returns `0.5f32`, which the code multiplies as
  before). Adding a hook must not change 3.0 behaviour. `cargo xtask all` has to pass unchanged.
* **Hook at the level where rulesets differ.** Hook a whole function where another ruleset runs
  a different algorithm, and a value or table where only constants change (issue #100 §1.0).
* **Neutral names.** A hook names what it decides (`semi_auto_delay`, `divestep_limits`), never
  a ruleset. Doc comments cite the TS 3.0 function and address, as everywhere else.
* **Value tables.** Where a ruleset changes only numbers, the hook returns a `&'static` table
  (`PowerupRules`, `StuntRules`) whose 3.0 instance is a `const` next to the code that reads it,
  pinned by a unit test to the literals it replaced.
* **Stunt timers.** The stunt code never does arithmetic on its timers (`StuntState::next_stunt`,
  `recover`, `close_attack_cd`, `prone_lock`, `block_timer`) itself: it stores what
  `stunt_clock().start(now, fuser1, secs)` returns and asks `running`/`done`, so a ruleset can
  keep end times instead of countdowns. Code outside the stunt module that writes them (the gun's
  `ServerHost`, `stunt::block_player`) passes seconds of the player's time through the clock too.
* **Arguments.** Shared hooks take plain values or `shared` types, because the movement code is
  generic over `Arith` and a trait object can't be. A hook that replaces a calculation, not a
  value, takes the command's `Arith` (`Fpu::ARITH` in the movement) and computes in that build
  with the public `shared::pm::{Fpu, FpuMath, Num, Gcc, Msvc}`: a default dispatches to the 3.0
  function (`PmRules::drop_punch_angle` calls `pm::drop_punch_angle::<F>`), and a plugin matches
  on the `Arith` the same way. Server and client hooks may take their own crate's types. Make
  those types `pub` (for example `rules::scoring::Frag`), so a plugin can implement the hook.

The hooks so far:

| Trait | Hook | 3.0 default | Called from |
|---|---|---|---|
| `PmRules` | `ground_accel()` | `GroundAccel::BySpeed` (`PM_GetAccelBySpeed`); `Constant(k)` multiplies by the `double` k | `pm/mod.rs` `accelerate` |
| `PmRules` | `recover_input_scale()` | 0.5 (a `double`) | `pm/ts.rs` `ts_handle` |
| `PmRules` | `prone_forward_scale()` | `None`: the forward move is set to 0 while prone; `Some(k)` multiplies it | `pm/ts.rs` `ts_handle` |
| `PmRules` | `wall_jump_speed()`, `wall_jump_up_velocity()` | `WALLJUMP_SPEED` (210), `JUMP_VELOCITY` (236.64319) | `pm/ts.rs` `ts_wall_jump` |
| `PmRules` | `wall_climb_velocity()` | `WALLCLIMB_VELOCITY` (334.664) | `pm/ts.rs` `ts_wall_jump_climb` |
| `PmRules` | `wall_dive()` | true: `WALLDIVE` dives along `forward × 3` and clears the bit | `pm/ts.rs` `ts_dive` |
| `PmRules` | `fix_velocity(arith, velocity, limit)` | nothing (TS 3.0's `FixVelocity` is empty) | `pm/ts.rs`, after the dive, wall jump and wall climb impulses, `limit = 2 × maxspeed` |
| `PmRules` | `jump_push(arith, jump, buttons, maxspeed, velocity)` | nothing | `pm/mod.rs` `jump`, before the upward speed of a normal or super jump |
| `PmRules` | `scaled_frametime(frametime, fuser1)` | `fuser1 < 0.1` (or NaN) ? 0 : `frametime × fuser1` | `pm/mod.rs` `player_move` |
| `PmRules` | `drop_punch_angle(arith, punch, dead, fuser1, frametime)` | `pm::drop_punch_angle` (zeroed while dead) | `pm/mod.rs` `check_parameters` |
| `PmRules` | `step_interval(arith, interval, fuser1)` | `pm::step_interval`: `1/max(fuser1, 0.1) × interval`, truncated | `pm/sound.rs` `update_step_sound` |
| `PmRules` | `speed_by_slots(free, the_one)` | `ts::speed_by_slots` | server `weapons/inventory.rs` `sync_speed` (the active plugin's) |
| `PmRules` | `max_player_speed()` | 330: `SetPlayerSpeed`'s clamp and `TSInit`'s speed | server `inventory.rs` `sync_speed`, `rules/lifecycle.rs` `on_spawn` |
| `PmRules` | `texture_limits()` | `TextureLimits::TS30`: 1024 entries, 15 characters (`TextureLimits::SDK`: 512, 12) | `pm/mod.rs` `categorize_texture_type` (`textures::find_in`); the exported `pfnPM_FindTextureType` and the client's impact lookup keep 3.0's table |
| `PmRules` | `in_eye_pitch(target_pitch)` | the target's pitch, copied | `pm/mod.rs` `spectator_move` (in eye) |
| `PmRules` | `roll_divides_first()` | false: `side × rollangle / rollspeed` | `pm/mod.rs` `calc_roll` |
| `PmRules` | `friction_edge_by_reciprocal()` | false: `vel / speed × 16 + origin` | `pm/mod.rs` `friction` (`edge_ahead`) |
| `PmRules` | `ieee_compares()` | false: under `Arith::Msvc` the movement's `float` compares test C3/C0 as 3.0's `mp.dll` does, so a NaN is equal and below; true runs the `MsvcIeee` build (same arithmetic, IEEE compares) | `pm/mod.rs` `player_move_with` (every `Fpu::cmp_eq`/`cmp_lt`/`cmp_le` and `FpuMath::normalize`) |
| `GunRules` | `semi_auto_delay(rate)` | 0.1 | `weapons/gun.rs` `standard_fire` |
| `GunRules` | `tables()` | `table::base_tables()` | made the active `weapons::tables()` (below) |
| `GunRules` | `fire(gun, host)` | `Gun::fire_core` (`CTSGun::Fire` and the three shot functions) | `weapons/gun.rs` `Gun::fire` |
| `GunRules` | `timer_step(arith, frametime, fuser1, powerups)` | `Gun::timer_dt` (`frametime x GiveSlowMul x DFR 1.5`) | server `weapons/mod.rs` `post_think`, client `prediction/gun.rs` `post_think` |
| `GunRules` | `command_frame_time(msec)` | `(float)(msec * 0.001)` | client `prediction/gun.rs` `post_think` |
| `GunRules` | `decay_spread(gun, decay) -> bool` | `Gun::decay_spread` by the step, while alive; true: the client's crosshair copy (`state.spread`) takes the penalty on every first prediction (the server ignores the value) | server `weapons/mod.rs` `pre_think`, client `post_think` (first prediction) |
| `GunRules` | `pellet_offset(seed, i, bot_random)` | `None`: the polar pattern as each side computes it | server `weapons/fire.rs` `shoot_bullets`, client `events/fire.rs` `fire_bullets` |
| `GunRules` | `pellet_dir(side, aim, right, up, spread, offset)` | `spread::pellet_dir`: the direction rounded as the side's TS build does (`PelletSide::Server(arith)`: mp.dll or `ts_i386.so`; `Client`: `client.dll`), issue #161 | the same two |
| `GunRules` | `akimbo_event_offsets(linked, toggle)` | linked -5/+5, one gun none | client `events/fire.rs` `fire_akimbo` |
| `GunRules` | `predicted()` | `Predicted::ALL` | `weapons/gun.rs` `weapon_post_frame` (client prediction only) |
| `GunRules` | `cancel_runs_frame_in_reload(info)` | `info.stock != 0` | `weapons/gun.rs` `post_frame_ready` |
| `GunRules` | `better_weapon(gun, excluded)` | `Gun::better_weapon_core`: the first owned id in `g_iWeaponPriority` | `weapons/gun.rs` `get_better_weapon` (`remove_weapon`, the server's `weapons::get_better_weapon` for bots) |
| `GunRules` | `blades_need_press()` | false: the knife and katana attack while the button is held | `weapons/gun.rs` `primary_attack`, `secondary_attack` |
| `GunRules` | `katana_throws()` | true: +attack2 throws the katana | `weapons/gun.rs` `secondary_attack` |
| `GunRules` | `infinite_clip_skips_idle_reload()` | true: `!InfiniteClips && clip < 1` reloads | `weapons/gun.rs` `weapon_idle` |
| `GunRules` | `holster_draws_kungfu()` | true: `Unzoom`, then weapon 0's view model, draw animation and draw event | `weapons/gun.rs` `holster_weapon` |
| `MeleeRules` (supertrait of `GunRules`) | `knife_fire(gun, host)`, `katana_fire`, `ccombat_attack`, `ccombat_block`, `cold_cock` | false: 3.0's `Gun` method runs | the top of each `Gun` method in `weapons/gun.rs` |
| `ServerRules` | `on_activate()` | nothing | `gameplay::map_start` |
| `ServerRules` | `refuse_client(client)` | none (everyone joins) | `gameplay::refuse_client`, from `rules::client_connect` (§4.2) |
| `ServerRules` | `game_description(mode)` | `rules::core_description`: "The Specialists v3.0.2 (DM)" (LMS, Teamplay, The One; no mode without rules), the version from `Cargo.toml` | `rules::game_description` (`pfnGetGameDescription`, the server browser's Game column); `ts_server::game_description!` builds a plugin's text |
| `ServerRules` | `cvars()` | none | `gameplay::register` (registered after 3.0's, skipping existing names) |
| `ServerRules` | `server_command(name)` | false (3.0's command runs) | `rules/commands.rs` `globalslow`, `restartround`, `weaponrestriction` |
| `ServerRules` | `client_command(p, cx, cmd, args)` | `None` (3.0's handlers run) | `commands.rs` `dispatch_player`, before the weapons' |
| `ServerRules` | `install(g)` | `GameRules::install_multiplayer` | `rules/state.rs` `GameRules::install` (multiplayer maps) |
| `ServerRules` | `think(now)` | `rules::frame::ts_think` | `rules/frame.rs` multiplay and teamplay `Think` |
| `ServerRules` | `set_player_the_one(p, cx)` | `rules::scoring::set_player_the_one_core` | `rules/scoring.rs` `set_player_the_one` |
| `ServerRules` | `default_team(p, cx)` | `rules::teams::default_team_core` (the `team` key) | `rules/teams.rs` `set_default_team` |
| `ServerRules` | `on_spawn(p, cx)` | nothing | `rules/lifecycle.rs` `on_spawn`, end of `TSInit` |
| `ServerRules` | `put_in_server_cash()` | `cvars::INITIAL_CASH` (1200) | `rules/lifecycle.rs` `on_spawn` (first spawn) |
| `ServerRules` | `on_start_observer(p, cx)` | nothing | `rules/lifecycle.rs` `start_observer`, before `TSCash` |
| `ServerRules` | `observer_think(p, cx, now)` | `rules::lifecycle::observer_think_core` (countdown, join deadline, `Observer_HandleButtons`, `Observer_Idle`; 3.0 respawns with the `respawn` command) | `rules/lifecycle.rs` `pre_think`, while `iuser1` is set (PreThink zeroes `impulse` and returns after it) |
| `ServerRules` | `observer_set_mode(p, cx, mode)` | `rules::lifecycle::observer_set_mode` (`Observer_SetMode`) | `rules/lifecycle.rs` `start_observer` and `dev_observe` (`ts_observe`); a ruleset with its own camera calls its own from its `observer_think` |
| `ServerRules` | `on_init_hud(p, cx)` | nothing | `rules/lifecycle.rs` `ts_init_hud`, at its end |
| `ServerRules` | `set_respawn_time(p, cx, secs)` | `rules::lifecycle::set_respawn_time_core` | `rules/lifecycle.rs` `set_respawn_time` |
| `ServerRules` | `on_killed(p, cx)` | nothing | `rules/lifecycle.rs` `killed`, after the respawn time |
| `ServerRules` | `change_team_tail(p, cx, kill)` | `rules::teams::change_team_tail_core` (not a kill, outside The One mode: observer, respawn in 5 s) | `rules/teams.rs` `change_team`, after `TeamInfo`/`ScoreInfo` |
| `ServerRules` | `health_regen(p, cx, now)` | `rules::lifecycle::health_regen_core` | `rules/lifecycle.rs` `pre_think` |
| `ServerRules` | `points_for_frag(frag, sliding, diving)` | `rules::scoring::points_for_frag` | `rules/scoring.rs` `player_killed_core` (and a plugin's own `player_killed`) |
| `ServerRules` | `player_killed(p, cx, d, g)` | `rules::scoring::player_killed_core` | `rules/scoring.rs` `player_killed`, once the killer can be reached |
| `ServerRules` | `cash_based()` | false | `rules/state.rs` `cash_based` |
| `ServerRules` | `buy_armament(p, cx)` | `weapons::buy_armament` | `weapons/mod.rs` spawn, without a `game_player_equip` |
| `ServerRules` | `pick_up_cost(id, w, from)` | `weapons::pick_up_cost`: `None` (refused) for the akimbo pair of a single gun, else `slots`, × the count for a dropped stack | `weapons/inventory.rs` `pick_up_weapon` |
| `ServerRules` | `dead_grenades_become_powerups()` | `!weapons::grenade_buys()` (`m_mode < 2`: always) | `weapons/inventory.rs` `drop_weapon` |
| `ServerRules` | `drop_lifetime_and_glow()` | true: `drop`/disarm gives `clamp(2 × weaponstay, 10, 120)` and the dropper's glow | `weapons/worldgun.rs` `set_drop_lifetime` (from `inventory.rs` `drop_weapon`) |
| `ServerRules` | `stack_drop_count(held, disarm)` | the whole stack, one on a disarm (`DropWeapon` 0x94900) | `weapons/inventory.rs` `drop_weapon` |
| `ServerRules` | `world_gun_fall_impact()` | true: `Fall`'s 0.1 s look-ahead trace hurts what it hits | `weapons/worldgun.rs` `WorldGun::think` |
| `ServerRules` | `world_gun_touch(gun, attacker, id, other)` | nothing | `weapons/worldgun.rs` `WorldGun::touch` (flying) |
| `ServerRules` | `ammo_pack_stays_when_nothing_fits()` | false: the touch removes the pack | `weapons/worldgun.rs` `AmmoPack::touch` |
| `ServerRules` | `player_add_health(v, amount)` | `player::damage::add_health` (refused above max, uncapped, starts the overheal decay) | `player/damage.rs` `take_health_plain`, `take_health_edict_plain` |
| `ServerRules` | `battery_armor(p, amount)` | `items::player_take_armor` (`TakeArmor`: kevlar slots, speed) | `entities/stock/items.rs` `Item::my_touch` (battery) |
| `ServerRules` | `charger_capacity(armor)` | 1 | `entities/stock/items.rs` `Charger::spawn`, `recharge` |
| `ServerRules` | `charger_charge(armor, juice, activator)` | `items::charge_once`: `TakeArmor(100)` / `TakeHealth(max - health)`, then empty | `entities/stock/items.rs` `Charger::use_` |
| `ServerRules` | `entity_object(class)` | `None`: the type linked to the classname | `world::create_linked`, the body of every `link_entities!` export |
| `ServerRules` | `precache()` | nothing; run for every registered ruleset at each map start | `gameplay::precache_rulesets`, from the world's precache after TS 3.0's |
| `ServerRules` | `spawns_entity(class)` | true | `entities/mod.rs` `dispatch_spawn` (-1 when refused; the map load goes through it) and `dispatch_key_value` (a refused class parses no key) |
| `ServerRules` | `entity_key_value(ent, cx, key, value)` | `None` (the class parses it) | `ham/mod.rs` `class_key_value` (`DispatchKeyValue`'s class part and AMXX's `ExecuteHam`) |
| `ServerRules` | `entity_touch(ent, cx, other)` | false (the class's `Touch`) | `ham/mod.rs` `class_touch` (`DispatchTouch`, the game's touches, `ExecuteHam`) |
| `ServerRules` | `broken_respawn_delay(respawntime)` | `Some(30 s if 0, else the key)` | `entities/stock/breakable.rs` `die`, `doors.rs` (break) |
| `ServerRules` | `breakable_respawn_guarded()` | true (wait for clients in the box, fire `respawntarget`) | `breakable.rs`, `doors.rs` `breakable_respawn` |
| `ServerRules` | `objectives()` | `entities::ts::objective::CORE` (`ObjectiveRules`: HUD effect 8, team indexes, the indexed list, givers enabled by pointers, the wire HUD, NaN timers run out, the list cleared outside rounds) | `entities/ts/objective.rs`, `ctb.rs` (the givers' message) |
| `ServerRules` | `round_time_left(now)` | `objective::round_time_left_core` (timed round: manager's `targetontimeleft`, else a draw) | `objective.rs` `Bomb::think` |
| `ServerRules` | `giver_team_win(&GiverWin)` | `ctb::giver_team_win_core` (`TeamWin(winnerteam, (int)value, restart, isScore)`) | `ctb.rs` `WinGiver::use_` |
| `ServerRules` | `break_debris_from_hit()` | true (debris along the last hit, long glass `TSFrags`, attacker's `fuser1` copied) | `breakable.rs` `die`, `doors.rs` (break); false is the SDK's (`g_vecAttackDir × 200` if `explosion directed`, a door's none, 13-byte glass `TSFrags`) |
| `ServerRules` | `breakable_attack_dir(org, absmin, size)` | `damage::normalize(org - (absmin + size × 0.5))` | `breakable.rs` `take_damage_impl` (`g_vecAttackDir`) |
| `ServerRules` | `respawn_entities()` | `entities::ts::respawn_entities` | `rules/frame.rs` `restart_round` |
| `ServerRules` | `entities_think(now)` | nothing | `rules/frame.rs` `ts_think` (DM/TDM branch, after The One) |
| `ServerRules` | `map_globals(e, spawnflags)` | `Some(MapFlags::from_spawnflags)` | `entities/ts/misc.rs` `MapGlobals::spawn` |
| `ServerRules` | `ground_weapon_lands_flying()` | false | `entities/ts/ground.rs` `Ground::fall` |
| `ServerRules` | `give_ground_weapon(g, cx, p, pcx)` | `GroundWeapon::give` | `ground.rs` `get_weapon` (+use, `Use`, touch) |
| `ServerRules` | `outgoing_message(msg, w, send)` | `send(w)` | `messages.rs` `outgoing`, the filter `goldsrc::server::MsgWriter::end` runs every message through (`set_outgoing_filter`) |
| `ServerRules` | `powerups()` | `rules::powerup::CORE` (`PowerupRules`: base durations 3/3/2/3, `pwupmult`, instant mask 0x1e6, top-up < 3, swap > 2, no swap on instant types, drop > 2, slow amounts 0.6/0.35 and 0.8/0.225, `TSSlowMo` 0.35/0.225, the grenade power-up selects the grenade) | `entities/ts/powerup.rs` `setup`/`pickup_plan`, `rules/powerup.rs` `update_state`/`add_powerup`/`hud_slow_amount`/`normal_secs`, `rules/commands.rs` `ts_givepowerup` |
| `ServerRules` | `powerup_kind(key, spawnflags)` | `entities::ts::powerup::kind_for` (`GetRandomPowerup`, never armour) | `entities/ts/powerup.rs` `setup` |
| `ServerRules` | `armor_powerup(p, cx, armor)` | `TakeArmor(100)` (`armor`: `PowerUp::armor`, 100; `entities::ts::powerup::set_armor`) | `rules/powerup.rs` `add_powerup` |
| `ServerRules` | `time_scale_eases()` | `true` (`InterpolateSlowMotion`) | `rules/powerup.rs` `go_slow*`/`deactivate_flow`, `rules/lifecycle.rs` `pre_think` |
| `ServerRules` | `on_go_slow(p, amount)` | nothing | `rules/powerup.rs` `go_slow_self`, `go_slow_plain` (a player's `GoSlow`) |
| `ServerRules` | `grenade_fuse_step(elapsed, time_scale)` | `max(fuser1, 0.01) * 0.1f + elapsed` | `weapons/projectiles.rs` grenade `WaitThink` |
| `ServerRules` | `anim_framerate(time_scale)` | `max(fuser1, 0.01)` | `rules/lifecycle.rs` `pre_think`, `stunt/mod.rs` `pre_think` |
| `ServerRules` | `motion_blur()` | `true` | `stunt/mod.rs` `maybe_blur`, `stunt/moves.rs` `go_bullet_time`, `rules/powerup.rs` `use_powerup`/`run_powerup`, `rules/lifecycle.rs` `killed`, `weapons/melee.rs` |
| `ServerRules` | `sound_pitch(pitch, time_scale, s)` | `entities::stock::explode::slow_pitch` | `entities/stock/explode.rs` `emit_sound_slow` |
| `ServerRules` | `stunts()` | `stunt::rules::CORE` (`StuntRules`: recoveries 0.5625/0.375/0.75/0.2625/0.75, get-up keys 0x4006, wall dive, glide climb, ..., and `CTSAnimation`'s priority rules and timer model) | `stunt/moves.rs`, `stunt/anim.rs` |
| `ServerRules` | `stunt_clock()` | `stunt::rules::RELATIVE` (`StuntClock`: seconds left, counted down in the player's time) | `stunt/mod.rs`, `stunt/moves.rs`, `weapons/host.rs` (every stunt timer write and test) |
| `ServerRules` | `can_dive(p, armor, dive)` | `stunt::rules::can_dive` (used < 42/52/62, The One bypasses) | `stunt/moves.rs` `can` |
| `ServerRules` | `bullet_time_allowed(time_scale)` | `max(fuser1, 0.01) <= 0.99` | `stunt/moves.rs` `go_bullet_time` |
| `ClientRules` | `map_effects()` | true | `effects.rs` `frame` (emitters) and `fog_begin` (map fog), asked each frame |
| `ClientRules` | `ruleset_name()` | `"TS 3.0"` | `menus::draw_buy_menu` (§4.3) |
| `ClientRules` | `crosshair_parts(weapon, fov)` | divesteps first for every weapon, then the crosshair unless weapon 0/24/25/34/35 | `hud/weapons.rs` `draw_crosshair` |
| `ClientRules` | `crosshair_gap(base, spread, accuracy)` | `base + spread × 160` (f32), clamped to ±0x10000 | `hud/weapons.rs` `draw_crosshair` |
| `ClientRules` | `divesteps_blocked(used, armor, teamplay)` | `used ≥ 42 / 52 / 62` | `hud/weapons.rs` `draw_divesteps` |
| `ClientRules` | `stunt_cooldown(cooldown, now)` | the `NextStunt` countdown is ≥ 0.01 | `hud/weapons.rs` `draw_divesteps` |
| `ClientRules` | `pinned_default_fov()` | none (`default_fov` clamped to 70..110) | `hud/mod.rs` `think_fov` |
| `ClientRules` | `fov_snaps(cur)` | `m_iFOV` below 0.001 | `hud/mod.rs` `think_fov` → `view::ease_fov` |
| `ClientRules` | `input_gate()` | true (the anti-script real-key gate) | `input.rs` `gate_open` |
| `ClientRules` | `attack_switch()` | true (`cl_attackswitch` works) | `input.rs` `attack_switched` |
| `ClientRules` | `attack_sends_respawn()` | true | `input.rs` `key_tap` |
| `ClientRules` | `kungfu_exhausted(power)` | `power ≤ 0.01` (empty red frame) | `hud/weapons.rs` `draw_kungfu_bar` |
| `ClientRules` | `kungfu_regen(r)` | +15 per step (1 while negative), up to 100 | `hud/state.rs` `regen_kungfu` (weapon `Think`) |
| `ClientRules` | `kungfu_power_on_reset()` | none (only `VidInit` refills the bar) | `hud/state.rs` `reset_weapons` |
| `ClientRules` | `kungfu_power_from_weapon_info(clip)` | none (`KFuPower` alone sets the bar) | `hud/state.rs` `WeaponInfo` (id 0) |
| `ClientRules` | `kungfu_label(power, hud, w, h)` | none: 3.0's `DrawKungFuLabel`; a ruleset returns its bar as `hud::draw::HudOp`s | `hud/weapons.rs` `draw_kungfu_label` |
| `ClientRules` | `status_look()` | `StatusLook::default()`: 3.0's stance health fill, kevlar icon alone, team-coloured team bar, titles labels, icons × 2/3 | `hud/status.rs` `draw_status_panel` |
| `ClientRules` | `slowmo_icon()` | false | `hud/status.rs` `draw_status_panel` |
| `ClientRules` | `slowmo_tint()` | true (the `cl_slowfade` tint while slowed) | `view.rs` `write_screen_fade` |
| `ClientRules` | `fire_pitch(time_scale, slowmo_factor100)` | `100 - (int)(ts * -300)` | `events.rs` `weapon_fire_pitch` (the weapon fire sound only; ricochets and explosions keep 3.0's `fire_pitch`) |
| `ClientRules` | `view_bob(bob, input)` | `Bob::calc` (`V_CalcBob` 0x10066e70): the gun offset and bob pitch, no camera bob (`Bob::eye` stays `None`) | `view.rs` `calc_normal` (the gun offset and angle; `Bob::eye` is added to `vieworg`) |
| `ClientRules` | `eye_origin(eye, simorg, viewheight, on_ground, ft)` / `body_cam_z(eye, z, on_ground, ft)` | `EyeZ::eye` (the view height eased at `min(15·ft, 1)` on z, then `V_SmoothStairs` 0x100677d0) / `EyeZ::body_cam_z` (`V_SmoothStairs` again, 0x10068313) | `view.rs` `calc_normal` (the eye before the camera bob; the third-person body cam) |
| `ClientRules` | `client_bullets_queue(realbullet, time_scale, slowmo_factor100)` | `realbullet != 0 \|\| ts < 0.99` | `events/fire.rs` `fire_bullets` |
| `ClientRules` | `client_bullet_next_update(now, realbullet)` | `next_update`: 0.1 s, 0.05 s or every frame by `realbullet` | `events/fire.rs` `client_bullets_frame` |
| `ClientRules` | `client_bullet_step(dir, step)` | `dir·velocity·40·dt·GetBulletVelocityMult` | `events/fire.rs` `client_bullet_impact` |
| `ClientRules` | `client_bullet_range_index(dist, unit, exit)` | exit `ftol(dist·0.0033333334f·10)`, else `range_index` | `events/fire.rs` `client_bullet_impact` |
| `ClientRules` | `client_bullet_draws(time_scale, slowmo_factor100)` | `ts < 0.99` | `events/fire.rs` `client_bullets_frame` |
| `ClientRules` | `client_bullet_draw(bullet, draw)` | false: the core `shotgun_pellets.spr` streaks and `bullet_trail.spr` trail; a ruleset may queue `BulletModels`, which `effects::create_entities` adds (`goldsrc::ext::models`) | `events/fire.rs` `client_bullets_frame` |
| `ClientRules` | `fx_style()` | `FxStyle::TS30`: the muzzle flash, gun smoke, impact smoke, puff and debris parameters | `effects/fx.rs` builders |
| `ClientRules` | `impact_particles(ps, now, pos, n, tex, bullet_type)` | false: `fx::impact`'s body | `effects/fx.rs` `impact` |
| `ClientRules` | `gun_smoke_rate(id)` | `RandomLong(5, 10)` for 33, else `RandomLong(45, 55)` | `events/fire.rs` `gun_fx` |
| `ClientRules` | `pellet_spray_gravity()` | 0 | `events/fire.rs` `gun_fx` |
| `ClientRules` | `shell_side_speed(id)` / `shell_sound(id)` | `RandomLong(-40, 40)` for 17, else `RandomLong(120, 140)` / 2 for 11, else 1 | `events/fire.rs` `Shot::standard` |
| `ClientRules` | `pump_shell_speeds()` | 20-40 (sideways and forward), 0-20 (up) | `events/fire.rs` `Shot::pump` |
| `ClientRules` | `ammo_attenuation(ammo)` | `AMMO_ATTENUATION`, 0.4 past it | `events/fire.rs` `fire_sound` |
| `ClientRules` | `bullet_whizz_and_push()` | true | `events/fire.rs` `fire_bullets` |
| `ClientRules` | `bullet_material_sound()` / `ricochet_pitch(ts, factor)` | 0.97 and `slow_pitch` / `fire_pitch` | `events/impact.rs` `texture_sound` |
| `ClientRules` | `event_sound()` | `RandomFloat(0.92, 1)`, `ATTN_NORM`, `slow_pitch` | `events/misc.rs` (empty, pump, reload, melee) |
| `ClientRules` | `melee_material_sound(tex)` | 0.9, `ATTN_NORM`, `slow_pitch` | `events/misc.rs` `melee_material_sound` |
| `ClientRules` | `melee_reach(m)` | close combat -55/45/35, coldcock 30, knife 35, katana -55/50 | `events/misc.rs` |
| `ClientRules` | `close_combat_blocks()` / `blade_hit_particles(..)` / `reload_event_phases()` | true / false (scintille + sparks) / true | `events/misc.rs` |
| `ClientRules` | `buy_cash_based(teamplay, cashbased)` | false (`CashBased()` hard-wired off: 30000) | `menus/mod.rs` `rules` → `buy::cash` |
| `ClientRules` | `buy_menu()` | `MenuRules::default()`: 3.0's handguns and random pistols, kevlar 10 slots, stealth shoes restricted below team modes, `cl_buykevlar`, the akimbo fix-up, keys 1..11; the look: text through `titles.txt` (`text: None`), item rows red by money or slots, six category mouse rows, no "hole" message | `menus/buy.rs`, `menus/mod.rs` |
| `ServerRules` | `bullet_trace(start, end, shooter)` | `weapons::trace_expanded` (players' boxes +32) | `weapons/fire.rs` `shoot_bullets`, `pierce_shot`; `weapons/bullets.rs` `update` |
| `ServerRules` | `bullet_damage(ammo, weapon, damage, falloff, simulated)` | `falloff × damage` | `shoot_bullets`, `bullets.rs` `update` (first hit) |
| `ServerRules` | `bullet_range_index(scaled)` | truncated (`Ext::trunc_i32`); `scaled` is the unrounded `dist / (unit·10)·10` as an exact `Ext` (`fire::range_scaled`), issue #80 | the same three |
| `ServerRules` | `bullet_damage_multiplier(acc, cheat)` | `cheat` (`damagemult` with `sv_cheats`) or 1; `acc` unused | `shoot_bullets` (per hit, `acc` per call), `bullets.rs` `update` |
| `ServerRules` | `pierce_step(left)` | `min(2·left, 48)` | `shoot_bullets` |
| `ServerRules` | `weapon_realbullet(realbullet)` | `realbullet` | `shoot_bullets`, `bullets.rs` `think` |
| `ServerRules` | `bullet_velocity_mult(arith, slowmul, bullet_time) -> Ext` | `spread::bullet_velocity_mult`: the value as `arith`'s build holds it (mp.dll's stored `float`, `ts_i386.so`'s unrounded quotient) | `bullets.rs` `update` |
| `ServerRules` | `player_hit(&PlayerHit) -> HitDamage` | `player::damage::hit_damage` (stunt and dodge ×0.7, head ×2 / ×1.5 shotgun, legs ×0.5) | `player/damage.rs` `trace_attack` |
| `ServerRules` | `armor_damage(armor, amount, hitgroup, bits)` | `player::damage::after_kevlar` (absorb `armor_absorb`, armour kept) | `player/damage.rs` `take_damage` |
| `ServerRules` | `damage_punch()` | 0 | `player/damage.rs` `take_damage` |
| `ServerRules` | `melee_block(kind, attacker, victim, damage, atk_block)` | `weapons::melee::hold` (3.0's victim holds) | `weapons/melee.rs` `trace_attack`, `pending_hit` (the attacker lent to its slot) |
| `ServerRules` | `jump_kick_hit(p, cx, strength)` | `stunt::moves::jump_kick_hit_core` (`m_flNextAttack` 0.35 if run out, `UpdateMeleeBar(strength × 1.5)`) | `stunt/moves.rs` `close_attack` |
| `ServerRules` | `backflip_kick(p, cx)` | `stunt::moves::backflip_kick_core` (22.5 power, a delayed 15/30 for 0.3 s) | `stunt/moves.rs` `go_wheel` backward |
| `ServerRules` | `melee_disarm(damage, victim_is_the_one)` | `Some(damage)` (the disarm count) | `weapons/melee.rs` `trace_attack`, `pending_hit` |
| `ServerRules` | `zero_delayed_hit_reaches_player()` | `false` (a spent delayed hit goes straight to its 0-damage `TraceAttack`, 0x8c1f1) | `weapons/melee.rs` `pending_hit` |
| `ServerRules` | `grenade_explosion()` | (110, 300, 1024) | `weapons/projectiles.rs` grenade `think` |
| `ServerRules` | `explosion_pushes_world_guns()` | true | `entities/stock/explode.rs` `ts_explosion` |
| `ServerRules` | `explosion_aftermath(org, inflictor, attacker)` | `explode::explosion_slow_point` (0.25 for 1 s) | `ts_explosion` |
| `ServerRules` | `thrown_blade(btype) -> ThrownBlade` | `projectiles::thrown_blade` (1900 / 1000 u/s, gravity 0.75 / 0.3 / 0.85, bits `0x08001000`) | `projectiles.rs` `throw_blade`, `Knife::spawn`, `think`, `bounce_touch` |
| `ServerRules` | `thrown_blade_damage(btype, from, at)` | `projectiles::thrown_blade_damage` (100 / 50 / 150) | `projectiles.rs` `Knife::bounce_touch` |

**Melee.** `MeleeRules`' hooks replace a whole attack and take `&mut dyn GunHost`, so the `Gun`
helpers a plugin calls take `?Sized` hosts. `GunHost` gained `melee_hold(bits, recover, crouch)`
(the held moves' `iuser4` bits, crouched view and stunt recovery, in seconds the stunt clock
scales; 3.0's `melee_crouch` is `melee_hold(bits, 2, true)`) and `the_one()`, and `Sound` gained
`CloseCombatMiss` and `Block`.

**Weapon tables.** A ruleset's tables (`GunRules::tables`) are installed as the ones
`shared::weapons::tables()` returns (`table::set_ruleset_tables`): by the server when a plugin
becomes active (`gameplay::set_active`), by the client from each predicted command's rules. So
everything that reads weapon data (the gun, inventory, buying, the HUD, damage, the events)
follows the ruleset without a hook of its own; a change of tables drops custom weapon parameters
(`loadweapons`, `CustomWP`), and so does every map start
(`weapons::rebuild_weapon_info`, `W_Precache`'s `compile_weapons_info`). The map precaches the weapon assets of every registered ruleset's
tables, TS 3.0's first so its precache order is unchanged (`weapons::precache_weapon_assets`),
and the server keeps one set of model strings per ruleset. The gun's `GunHost` also tells the
rules the raw time scale (`time_scale`) and whether it is the client's prediction
(`prediction`), where TS's two copies differ.

## 6. Adding a hook

1. Add a method to the trait's file with the 3.0 behaviour as its default body:
   `shared/src/gameplay/pm.rs` (`PmRules`), `shared/src/gameplay/gun.rs` (`GunRules`),
   `server/src/gameplay/server.rs` (`ServerRules`) or `client/src/gameplay/client.rs`
   (`ClientRules`). Move the 3.0 code into the default, or have the default call it (make the
   3.0 function and its argument types `pub`).
2. Call it where the 3.0 code was, through the rules the path already resolved (§3):
   * movement: `self.rules` in `Pm`
   * server code outside the movement and the gun (the move speed): `crate::gameplay::active().pm`
   * gun: `host.rules()`
   * server: `crate::gameplay::active().server`
   * HUD: `crate::gameplay::hud_rules(h.gameplay)`
   * entities: `crate::gameplay::active().server` too. A hook that changes a class takes the
     dispatched object (`&mut dyn Entity`, downcast with `Any`) or the class's type, whose
     fields the plugin needs are `pub` (`Trigger`, `GroundWeapon`, `MapGlobals`).

   Don't resolve again inside a helper. Pass the rules down instead.
3. Override it in the plugin. For TS 2.1, the shared hooks go in `crates/ts21/src/shared/` (on
   `Ts21`), the server hooks in `crates/ts21/server/src/` (`Ts21Server`) and the client hooks in
   `crates/ts21/client/src/` (`Ts21Client`).
4. Add a row to §5's table, and a unit test where the behaviour can be tested without the engine
   (e.g. `shared/src/pm/tests.rs` `movement_hooks_follow_the_plugin` and
   `every_movement_hook_reaches_the_movement`, which run the movement in a fake world).
5. Check: `cargo test`, `cargo xtask lint` (isolation, the 3.0-only build), `cargo xtask all`.

## 7. Adding a ruleset

1. Create three packages, as `crates/ts21` does. The shared one depends on `ts-gameplay` and
   `shared`, implements `PmRules` and `GunRules` on a unit struct with a `static`, and defines the
   id (a new number). The server one depends on it and `ts-server`, implements `ServerRules`, and
   defines `pub static PLUGIN: ServerPlugin`. The client one does the same with `ts-client`,
   `ClientRules` and `ClientPlugin`. Add the side packages to the workspace members.
2. Register it in `server-dll` (`register_gameplay`, from `GameDLLInit`) and `client-dll`
   (`register_gameplay`, from `Initialize`), behind a default-on cargo feature named after it.
   Each library depends only on its own side's package.
3. Extend the isolation check in `xtask/src/lint.rs` (`RULESET_CRATE`, `RULESET_TOKENS`) to
   cover the new crate, and add its docs.

## 8. Isolation checks

`cargo xtask lint` (and `cargo xtask build` and `cargo test`, through `xtask`'s unit test)
checks the isolation:

* **Isolation.** It fails if a source, manifest or config outside `crates/ts21/` names TS 2.1
  (`ts21`, `v21`, `ts_2_1`, `gp21`, ...: `lint::RULESET_TOKENS`). The two libraries may name the
  plugin crate and the `ts21` feature to register it. A crate that depends on the plugin has to
  name it in its manifest, so the same check finds those.
* **3.0 on its own.** `cargo test -p ts-server-dll -p ts-client-dll -p ts-server -p ts-client -p
  shared -p ts-gameplay --no-default-features`: the libraries build without the TS 2.1 plugin,
  and the 3.0 tests pass.

`cargo xtask play` sets the ruleset to 2.1 and back, each reloading the map
([validation.md](../validation.md) §2, "Gameplay reload").
