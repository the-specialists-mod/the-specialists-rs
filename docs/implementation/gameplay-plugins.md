# Gameplay plugins

The port can run more than one ruleset and switch between them mid-game (issue #100). TS 3.0's
rules are the **core plugin** (`"30"`). Each other ruleset is a separate in-process Rust plugin.
The first is TS 2.1 ([ts21.md](ts21.md)). The server cvar `ts_gameplay` picks the active one.

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
              the per-call resolvers, the switch (frame, map_start)
    server.rs ServerRules: server-only hooks
    reset.rs  the reset a switch runs (§4)
  client/src/gameplay/
    mod.rs    ClientPlugin { pm, gun, client } and CORE_PLUGIN, the client registry, resolvers,
              reset_client
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
ruleset. Bots aren't part of a gameplay ruleset, so `ServerRules` has no bot hooks. The reset
(§4) only replays what a map start does for them.

Every plugin object is a `&'static` value, `Sync`, and stateless. Plugin state goes in the
plugin's own statics (`goldsrc::Global` with a `Cell`/`RefCell`, as the game crates do), and
`on_activate` sets it up (§4).

## 2. Selection and the channels

| What | Server | Client |
|---|---|---|
| The choice | `ts_gameplay` (`FCVAR_SERVER`, default `"30"`, port addition). It is read every frame, like `ts_pm_arith`. | — |
| Shared code (movement, gun) | Physinfo key `gp`, set at every spawn next to `mj`/`hl` (`rules/lifecycle.rs`, `stunt/mod.rs`, `player/save.rs`) and for every client on a switch | `pmove->physinfo` / `clientdata_t::physinfo` (`gp`) |
| HUD | — | `SrvSett` key 4, value = the id as a number. Sent in `InitHUD` after keys 0..3, and on a switch. TS 3.0's client ignores key 4. |

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
| Server frame, rules, scoring | `ts_server::gameplay::active().server` (a `Cell` read). `gameplay::frame()` runs at the start of `rules::start_frame` | `ts_gameplay` |
| Client HUD | `ts_client::gameplay::hud_rules(h.gameplay)` | `SrvSett` 4 (`Hud::gameplay`) |
| Client input, FOV, map effects, buy menu | `ts_client::gameplay::current()` (reads `Hud::gameplay` without holding the HUD's borrow) | `SrvSett` 4 |

Movement and the gun read physinfo instead of the cvar so that the server and the client's
prediction run the same rules for the same command, as they must.

## 4. Switching: the reset

When `ts_gameplay` changes, `gameplay::frame()` sees a different plugin at the start of the next
frame. It resets the game to a **brand-new state, as if the map had just started**, without
reloading the map. Then it calls the new plugin's `ServerRules::on_activate()`. Nothing carries
over, so no state is ever converted between rulesets. The reset is core code
(`server/src/gameplay/reset.rs`) and knows no plugin.

The reset runs in two halves 0.6 s apart. The engines don't hand out a freed edict again for
0.5 s (FWGS `SV_AllocEdict`, ReHLDS `ED_Alloc`), and re-spawning a large map into fresh edicts
could run out of them. `reset::begin` runs steps 1-3 on the switch frame, and `reset::finish`
runs steps 4-6 once the wait is over. In between, the map is empty and the players are frozen
and can't be hurt. The rules' Think doesn't run then, because `gameplay::frame()` returns false.
The reset replays a map start in this order:

1. **Clients.** `SrvSett` 4 with the new id goes to everyone, and physinfo `gp` is set on every
   client.
2. **Players.** Each player's items are destroyed (`remove_all_items`). Each player then gets a
   brand-new `Player` object and a zero score. Only the connection's identity stays: the bot flags,
   the bot backend's state for the new game (`BotBackend::reset_state`; native: a fresh `Brain`
   with the same skill), and the spray frames. Until step 6 the players
   are frozen (`FL_FROZEN`), can't be hurt, and count as alive (`DEAD_NO`), so a dead player's
   `PlayerDeathThink` doesn't make a corpse or start observing in the empty map.
3. **Entities.** Every entity past the client slots is removed at once (`UpdateOnRemove`, then
   `pfnRemoveEntity`), so the map's entities get their edicts back. Thrown knives, grenades,
   dropped guns, spawned power-ups and the corpses go with them. The world's soundent stays,
   like worldspawn, and is spawned again in step 4. It keeps its edict, the first after the
   clients, and the players' sound slots exist throughout.

   When the wait is over, `finish` first drops whatever the players' think did in the empty
   map: every player is made brand new again, and any entity created meanwhile is removed.
4. **The world.** `entities::stock::restart_world` runs what `CWorld::Precache` resets and
   creates, in its order: the per-map registries (masters, `env_global` states, sound rooms),
   then `rules::start_rules` (`GameRules::install`: game mode, rounds, teams, The One, slow
   motion, votes; the node graph and bot texts), the soundent (`CSoundEnt::Spawn` again), the
   body queue and the light styles. It leaves out the precaches (the engine has them), the forced `sv_gravity`/
   `sv_stepsize`, the configs and the chapter title.
5. **Map entities.** They are spawned again from the BSP entity lump as FWGS's
   `SV_LoadFromFile`/`SV_ParseEdict` do it, worldspawn excepted. That means `classname` and the
   private data first, then the other keyvalues in order (empty ones and `wad` skipped, trailing
   spaces trimmed from keys, `angle` given as `angles`), then `Spawn`. An entity whose spawn
   returns -1 is freed. After that comes `ServerActivate`'s `Activate` pass, and the bot
   manager's restart.
6. **Players spawn as fresh joins** (`ClientPutInServer`'s spawn). Everyone gets `InitHUD`,
   the score and team broadcasts, the buy menu and the respawn wait again. In teamplay, a bot
   made with a model joins that team again, as `BotSetup` does after the spawn. Like a new bot,
   it starts with no cash.

The server logs
`[ts] gameplay: ruleset 30 -> 21: game reset (N players, N entities removed, N spawned from the map)`.

On the client, a `SrvSett` 4 value that differs from the one it holds is a switch.
`gameplay::reset_client` runs what `HUD_VidInit` resets (HUD, prediction, effects, spectator)
before the server's fresh state arrives, and logs `[ts] gameplay: the server switched to
ruleset N; HUD reset`. Each `InitHUD` clears the value the HUD holds (a connect, a map change, or
the fresh join after a switch), and the `SrvSett` 4 after it only records the ruleset. The value
isn't cleared in `HUD_VidInit`, which the engine also calls on a video mode change (FWGS
`SCR_VidInit`, e.g. a window resize).
TS 3.0's own client ignores key 4. It still gets the fresh join's `ResetHUD`/`InitHUD` and the
re-sent state, but it can't predict another ruleset (issue #100 §1.1).

At map start, `rules::install` calls `gameplay::map_select()`, which makes the plugin `ts_gameplay`
selects active before the rules are built (so its `ServerRules::install` decides them), and
after them `gameplay::map_start()`, which calls `on_activate()`, with no reset. The server config runs after the map has
loaded. If it changes `ts_gameplay`, the first frame resets the game once.

### Differences from a real map start

* The configs (`game.cfg`, the server config) aren't run again, and the cvars keep their values:
  a switch keeps the server's settings.
* `SF_NOT_DEATHMATCH` entities are spawned, as FWGS spawns them by default. FWGS skips them
  only with `BUGCOMP_SPAWNFLAG_NOT_DEATHMATCH`.
* Clients aren't reconnected, so client-side state the server never resets stays. For example,
  the voice settings stay.

The reset is tested in-engine only on FWGS listen servers on `ts_dojo` (`cargo xtask play`).
Dedicated servers on the Steam engines, large maps and team modes are still to be tested
(issue #103).

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
| `GunRules` | `semi_auto_delay(rate)` | 0.1 | `weapons/gun.rs` `standard_fire` |
| `GunRules` | `tables()` | `table::base_tables()` | made the active `weapons::tables()` (below) |
| `GunRules` | `fire(gun, host)` | `Gun::fire_core` (`CTSGun::Fire` and the three shot functions) | `weapons/gun.rs` `Gun::fire` |
| `GunRules` | `timer_step(arith, frametime, fuser1, powerups)` | `Gun::timer_dt` (`frametime x GiveSlowMul x DFR 1.5`) | server `weapons/mod.rs` `post_think`, client `prediction/gun.rs` `post_think` |
| `GunRules` | `command_frame_time(msec)` | `(float)(msec * 0.001)` | client `prediction/gun.rs` `post_think` |
| `GunRules` | `decay_spread(gun, decay)` | `Gun::decay_spread` by the step, while alive | server `weapons/mod.rs` `pre_think`, client `post_think` (first prediction) |
| `GunRules` | `pellet_offset(seed, i, bot_random)` | `None`: the polar pattern as each side computes it | server `weapons/fire.rs` `shoot_bullets`, client `events/fire.rs` `fire_bullets` |
| `GunRules` | `akimbo_event_offsets(linked, toggle)` | linked -5/+5, one gun none | client `events/fire.rs` `fire_akimbo` |
| `GunRules` | `predicted()` | `Predicted::ALL` | `weapons/gun.rs` `weapon_post_frame` (client prediction only) |
| `GunRules` | `cancel_runs_frame_in_reload(info)` | `info.stock != 0` | `weapons/gun.rs` `post_frame_ready` |
| `ServerRules` | `on_activate()` | nothing | `gameplay::map_start`, `gameplay::frame` |
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
| `ServerRules` | `on_init_hud(p, cx)` | nothing | `rules/lifecycle.rs` `ts_init_hud`, at its end |
| `ServerRules` | `set_respawn_time(p, cx, secs)` | `rules::lifecycle::set_respawn_time_core` | `rules/lifecycle.rs` `set_respawn_time` |
| `ServerRules` | `on_killed(p, cx)` | nothing | `rules/lifecycle.rs` `killed`, after the respawn time |
| `ServerRules` | `health_regen(p, cx, now)` | `rules::lifecycle::health_regen_core` | `rules/lifecycle.rs` `pre_think` |
| `ServerRules` | `points_for_frag(frag, sliding, diving)` | `rules::scoring::points_for_frag` | `rules/scoring.rs` `player_killed_core` (and a plugin's own `player_killed`) |
| `ServerRules` | `player_killed(p, cx, d, g)` | `rules::scoring::player_killed_core` | `rules/scoring.rs` `player_killed`, once the killer can be reached |
| `ServerRules` | `cash_based()` | false | `rules/state.rs` `cash_based` |
| `ServerRules` | `buy_armament(p, cx)` | `weapons::buy_armament` | `weapons/mod.rs` spawn, without a `game_player_equip` |
| `ServerRules` | `spawns_entity(class)` | true | `entities/mod.rs` `dispatch_spawn` (-1 when refused; the map load and the reset's re-spawn both go through it) and `dispatch_key_value` (a refused class parses no key) |
| `ServerRules` | `entity_key_value(ent, cx, key, value)` | `None` (the class parses it) | `ham/mod.rs` `class_key_value` (`DispatchKeyValue`'s class part and AMXX's `ExecuteHam`) |
| `ServerRules` | `entity_touch(ent, cx, other)` | false (the class's `Touch`) | `ham/mod.rs` `class_touch` (`DispatchTouch`, the game's touches, `ExecuteHam`) |
| `ServerRules` | `broken_respawn_delay(respawntime)` | `Some(30 s if 0, else the key)` | `entities/stock/breakable.rs` `die`, `doors.rs` (break) |
| `ServerRules` | `breakable_respawn_guarded()` | true (wait for clients in the box, fire `respawntarget`) | `breakable.rs`, `doors.rs` `breakable_respawn` |
| `ServerRules` | `respawn_entities()` | `entities::ts::respawn_entities` | `rules/frame.rs` `restart_round` |
| `ServerRules` | `entities_think(now)` | nothing | `rules/frame.rs` `ts_think` (DM/TDM branch, after The One) |
| `ServerRules` | `map_globals(e, spawnflags)` | `Some(MapFlags::from_spawnflags)` | `entities/ts/misc.rs` `MapGlobals::spawn` |
| `ServerRules` | `ground_weapon_lands_flying()` | false | `entities/ts/ground.rs` `Ground::fall` |
| `ServerRules` | `give_ground_weapon(g, cx, p, pcx)` | `GroundWeapon::give` | `ground.rs` `get_weapon` (+use, `Use`, touch) |
| `ServerRules` | `outgoing_message(msg, w, send)` | `send(w)` | `messages.rs` `outgoing`, the filter `goldsrc::server::MsgWriter::end` runs every message through (`set_outgoing_filter`) |
| `ServerRules` | `powerups()` | `rules::powerup::CORE` (`PowerupRules`: base durations 3/3/2/3, `pwupmult`, instant mask 0x1e6, top-up < 3, swap > 2, no swap on instant types, drop > 2, slow amounts 0.6/0.35 and 0.8/0.225, `TSSlowMo` 0.35/0.225) | `entities/ts/powerup.rs` `setup`/`pickup_plan`, `rules/powerup.rs` `update_state`/`add_powerup`/`hud_slow_amount`/`normal_secs`, `rules/commands.rs` `ts_givepowerup` |
| `ServerRules` | `powerup_kind(key, spawnflags)` | `entities::ts::powerup::kind_for` (`GetRandomPowerup`, never armour) | `entities/ts/powerup.rs` `setup` |
| `ServerRules` | `armor_powerup(p, cx, armor)` | `TakeArmor(100)` (`armor`: `PowerUp::armor`, 100; `entities::ts::powerup::set_armor`) | `rules/powerup.rs` `add_powerup` |
| `ServerRules` | `time_scale_eases()` | `true` (`InterpolateSlowMotion`) | `rules/powerup.rs` `go_slow*`/`deactivate_flow`, `rules/lifecycle.rs` `pre_think` |
| `ServerRules` | `motion_blur()` | `true` | `stunt/mod.rs` `maybe_blur`, `stunt/moves.rs` `go_bullet_time`, `rules/powerup.rs` `use_powerup`/`run_powerup`, `rules/lifecycle.rs` `killed`, `weapons/melee.rs` |
| `ServerRules` | `sound_pitch(pitch, time_scale, s)` | `entities::stock::explode::slow_pitch` | `entities/stock/explode.rs` `emit_sound_slow` |
| `ServerRules` | `stunts()` | `stunt::rules::CORE` (`StuntRules`: recoveries 0.5625/0.375/0.75/0.2625/0.75, get-up keys 0x4006, wall dive, glide climb, ...) | `stunt/moves.rs` |
| `ServerRules` | `stunt_clock()` | `stunt::rules::RELATIVE` (`StuntClock`: seconds left, counted down in the player's time) | `stunt/mod.rs`, `stunt/moves.rs`, `weapons/host.rs` (every stunt timer write and test) |
| `ServerRules` | `can_dive(p, armor, dive)` | `stunt::rules::can_dive` (used < 42/52/62, The One bypasses) | `stunt/moves.rs` `can` |
| `ServerRules` | `bullet_time_allowed(time_scale)` | `max(fuser1, 0.01) <= 0.99` | `stunt/moves.rs` `go_bullet_time` |
| `ClientRules` | `map_effects()` | true | `effects.rs` `frame` (emitters) and `fog_begin` (map fog), asked each frame |
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
| `ClientRules` | `slowmo_icon()` | false | `hud/status.rs` `draw_status_panel` |
| `ClientRules` | `buy_cash_based(teamplay, cashbased)` | false (`CashBased()` hard-wired off: 30000) | `menus/mod.rs` `rules` → `buy::cash` |
| `ClientRules` | `buy_menu()` | `MenuRules::default()`: 3.0's handguns and random pistols, kevlar 10 slots, stealth shoes restricted below team modes, `cl_buykevlar`, the akimbo fix-up, keys 1..11 | `menus/buy.rs`, `menus/mod.rs` |

**Weapon tables.** A ruleset's tables (`GunRules::tables`) are installed as the ones
`shared::weapons::tables()` returns (`table::set_ruleset_tables`): by the server when a plugin
becomes active (`gameplay::set_active`), by the client from each predicted command's rules. So
everything that reads weapon data (the gun, inventory, buying, the HUD, damage, the events)
follows the ruleset without a hook of its own; a change of tables drops custom weapon parameters
(`loadweapons`, `CustomWP`). The map precaches the weapon assets of every registered ruleset's
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

`cargo xtask play` switches the ruleset mid-game ([validation.md](../validation.md) §2, "Gameplay
switch").
