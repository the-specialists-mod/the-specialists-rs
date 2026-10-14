# Bot plugins

Issue #104. The bots are a selectable layer of their own, separate from the gameplay rulesets
([gameplay-plugins.md](gameplay-plugins.md)). The server reaches bots only through a **bot
backend**:

* **`native`** (the default): TS 3.0's bots, the AI ported from `mp.dll` (`crates/server/src/bots`,
  [rules.md](rules.md) §10, decomp `docs/server/bots.md`).
* **a plugin backend**: a `tsmod_*` library the server loads ([plugin-api.md](plugin-api.md) §1.2)
  that registers a backend under a name. The first is `tsmod_rcbot`, the Rust port of RCBot1 in
  [the-specialists-mod/tsmod-rcbot-rs](https://github.com/the-specialists-mod/tsmod-rcbot-rs).
  RCBot is GPL, so it is never linked into the server; it uses the same plugin API as the AMX Mod
  X modules. The in-repo `tsmod_test` (`crates/tsmod-test`) is a stub backend for the tests.

The whole layer is a port addition: TS has one hard-wired bot AI, on listen servers only.

## 1. Layout

| Part | Where |
|---|---|
| `BotBackend` trait, registry, `ts_bot_backend`, the bot player (`BotSetup`), the owners table | `crates/server/src/bot_backend/mod.rs` |
| `ts_bot_quota` and the bot manager's timing | `bot_backend/quota.rs` |
| The native backend | `crates/server/src/bots/backend.rs` (`Native`), calling the functions in `bots/` |
| The native bots' rules per gameplay ruleset (§2.2) | `crates/server/src/bots/rules.rs` (`NativeBotRules`, `CORE`, the registry); TS 2.1's in `crates/ts21/server/src/bots/` |
| A plugin's backend (C callbacks → `BotBackend`) | `crates/server-dll/src/plugins.rs` (`PluginBackend`) |
| The API's bot functions | `crates/server/src/plugin_api/bots.rs` |

## 2. The `BotBackend` trait

```rust
pub trait BotBackend {
    fn name(&self) -> &str;
    fn add_bot(&self, req: &AddBot) -> Option<(*mut edict_t, Option<f32>)>;  // Addbot / Custom / Quota
    // all below default to nothing
    fn game_init(&self); fn map_init(&self); fn precache(&self);
    fn server_activate(&self); fn server_deactivate(&self);
    fn activate(&self); fn deactivate(&self); fn think(&self);
    fn new_state(&self, skill: f32) -> BotState;          // default: the plugin marker
    fn adopted(&self, e: *mut edict_t);
    fn spawn_pre(&self, p, cx); fn spawn_post(&self, p, cx);
    fn trace_attack(&self, p, cx, dmg); fn killed(&self, p, cx, attacker);
    fn on_kill(&self, killer, victim, now); fn pre_think(&self, p, cx);
    fn client_command(&self, p, cx, cmd, args) -> bool;
}
```

The per-player hooks take the `Player` and run inside its dispatch, at the points the native
bots ran before, so TS's order of calls is kept exactly. A plugin backend implements only
`name`, `add_bot`, `activate`, `deactivate` and `adopted`: plugins hear the game's events through
their own callbacks (plugin-api.md §1.3), at points where the player is in reach.

A bot's backend state lives on its `Player` (`Player::bot: Option<BotState>`, `BotState =
Box<dyn Any>`): the native backend keeps its `Brain` there (`bots::brain_of`), a plugin bot a
`PluginBot` marker (the plugin keeps its own state). `Player::cbot` (the `CBot` class:
`IsNetClient()` false, no starting cash, the vote line) and `Player::is_bot` (`m_bIsBot`) are TS's
player fields and are set for every backend's bots.

### 2.1 The coupling points

Each of the 15 places outside `src/bots` that called the native bots now calls the active
backend through `bot_backend` (no behaviour change for native):

| Site | Now |
|---|---|
| `server-dll` `GameDLLInit` | `bot_backend::game_init` (registers native, then its `game_init`) |
| `server-dll` `StartFrame` | `bot_backend::start_frame` (the host's quota, then `think`) |
| `rules::start_rules` (`CTSGameRules::Init`) | `bot_backend::map_init` (native: nodes and bot texts) |
| `rules::mapcycle` (the map end's `Init`) | `bot_backend::map_init` |
| `rules::server_activate` / `server_deactivate` | `bot_backend::server_activate` / `server_deactivate` (the quota's timing, then the backend) |
| `Player::bot` | `Option<BotState>` (above) |
| `Player::spawn` head / tail | `bot_backend::spawn_pre` / `spawn_post` |
| `Player::trace_attack` | `bot_backend::trace_attack` |
| `Player::pre_think` (node learning) | `bot_backend::pre_think` |
| `rules::lifecycle` `Killed` | `bot_backend::killed` |
| `rules::scoring` and the 2.1 plugin's `scoring` (the kill taunt) | `bot_backend::on_kill` (still only for a kill that scored) |
| `commands` (node and `testpath` commands) | `bot_backend::client_command` |
| `rules::commands` `addbot` / `addcustombot` | `bot_backend::add_bot(AddBot::Addbot / Custom)` |
| `precache` (`sprites/lgtning.spr`) | `bot_backend::precache`, at the same place in the precache order |

Kept as they were, because they are TS's player state and TS's cvars rather than the AI's:
`Player::is_bot`'s clear in `TSPlayerDisconnect`, `next_node_learn` (`pl+0x788`, `TSInit` zeroes
it), the `FL_FAKECLIENT` from `is_bot` at spawn, and the cvars (`botlearn`, `bottalk`, `botarmor`
are in TS's cvar list; `ts_bot_quota`, `ts_bot_stop`, `ts_bot_backend` are the host's).

### 2.2 Rulesets and the native bots

A gameplay ruleset can change how TS's own bots play (TS 2.1's do: #100 §3.10). That is the
native backend's business, not the host's and not the gameplay plugin's:

* **`NativeBotRules`** (`bots/rules.rs`) is the native backend's hook trait. Every method's
  default is TS 3.0's code (`CORE` implements none), so with no other rules the bots are
  unchanged. Hooks: creation (`random_model_in_teamplay`, `learn_on_empty_graph`,
  `stored_skill`, `team_key`, `texts`), the `CBot` overrides (`spawn_post`, `trace_attack`), the
  brain (`delay` for every module timer, the states and the stuck window; `runs_ai`, `turn_rate`,
  `update_view`, `look_at`, `press` / `release` / `update_buttons`, `respawn`), navigation (`ladder_buttons`, `wander_end`, `steer_probe_end`) and whole modules
  (`state_think`, `vision`, `attack`, `buy`, `look_around`, `stunt_mode`), and the node graph's
  link arithmetic (`ladder_link_distance`, `IsReachable`'s ladder shortcuts; `ground_probes`,
  `IsGroundContinuous`'s floor probes). Coarse where a ruleset
  replaces a module, fine where it changes one value inside shared code (the timers, the view,
  the buttons). A replacement gets the same `Brain`, `Player` and `Cx` and uses the native
  helpers, which are public; `Brain::ext` holds per-bot state of its own (Reset drops it).
* **Its own registry, keyed by the gameplay id.** The server library registers a ruleset's rules
  with `bots::rules::register(id, &RULES)` next to the gameplay plugin; `bots::rules::current()`
  resolves them from `gameplay::active_id()` (cached per id), and an id without rules runs
  `CORE`. So `ts_gameplay` picks the bots' rules and `ts_bot_backend` picks the backend,
  independently: a plugin backend never sees these hooks, `ServerRules` keeps no bot hooks, and
  `crates/server` names no ruleset (the isolation lint).
* Why not a field of `ServerPlugin` or hooks in `ServerRules`: the bot layer is a separate axis
  (this document), and putting the native AI's internals (`Brain`, its modules) into the gameplay
  plugin's interface would tie every ruleset to one backend. Why not a backend of its own (a
  second "native 2.1" backend): `ts_bot_backend` is the user's choice of AI, and the gameplay
  ruleset must not change it (§3); the native backend follows each map's ruleset through the
  per-call resolution (`server_activate` loads the ruleset's bot texts if `Init` loaded others).

The host's `create_player` (`BotSetup`) takes `team_key`, because 2.1's `BotSetup` has no `team`
key; the native backend passes its rules' choice and plugin bots get 3.0's.

## 3. Selection: `ts_bot_backend`

Port cvar, default `native`. **Read at map start** (`rules::install`, right after the synchronous
`exec game.cfg` and before the rules' `Init`): `bot_backend::map_start`, through
`bot_backend::wanted`. So:

* `game.cfg` selects it for the map being loaded.
* **A change reloads the map**, from any source (the console, rcon, the server config, a script):
  when the backend `ts_bot_backend` resolves to differs from the running one at one frame and the
  next, the console says `[ts] bots: backend <name>: reloading <map>` and the server changes level
  to the same map (`crate::reload`, shared with `ts_gameplay`: one reload when both change,
  none for a change and back, no loop; [gameplay-plugins.md](gameplay-plugins.md) §4). The
  server config (`servercfgfile`) runs after the map has loaded, so on the Steam engines a
  backend it sets reloads the first map once; FWGS's dedicated server also runs it before the
  first map, which then starts with that backend and needs no reload. When it runs again on a
  later map nothing changes.
* Names are compared case-insensitively; empty means native. An unknown name warns once per
  value (`[ts] bots: unknown ts_bot_backend "x", using native`) and native runs. The reload
  compares the resolved backend, so `NATIVE` while native runs, or an unknown name, reloads
  nothing.
* Each change logs `[ts] bots: backend <name>`, calls the old backend's `deactivate`, **kicks
  every bot the old backend made** (`kick #<userid>`, queued) and calls the new one's
  `activate`. The quota then adds the new backend's bots. The gameplay ruleset (#100) doesn't change
  the backend.

## 4. Who owns what

* **The host** owns the quota (`ts_bot_quota`: names `Bot Neo`, ..., one add or kick per half
  second, a second after `ServerActivate`), `addbot` / `addcustombot` (they ask the backend:
  `AddBot`), the fake client and its join (`bot_backend::create_player`, TS's `BotSetup`), the
  owners table (which backend made which bot, by user id) and `ts_bot_stop` for plugin bots
  (`run_cmd` clears a living bot's movement and buttons).
* **The backend** picks names, models and skills (native: `bots.txt`, TS's seven models),
  thinks, and drives its bots.
* A quota bot the engine kept across a level change (a new `Player`, no state) gets
  `new_state(1.0)` and `adopted` (plugins: `BotBackend::adopt_bot`).

## 5. Writing a plugin backend

1. Export `TSRS_PluginLoad` (plugin-api.md §1.2); check `api->version == 3`.
2. In it (or in `game_init`): `register_bot_backend(id, "rcbot", &backend)` with `add_bot`,
   `activate`, `deactivate`, `adopt_bot`. Register commands (`rcbot ...`) and cvars, read data
   from `data_dir(id)` (`<gamedir>/tsmod/<name>/`: waypoints, learned data).
3. `add_bot(name, model, skill)`: pick what is empty, call `create_bot(id, name, model, skill)`,
   keep the index, return it.
4. Every `start_frame` while active: for each bot, think and call `run_cmd` once. Read the world
   with the engine's own functions (`engine_funcs`: traces, PVS, entity searches) and the API
   (`player_info`, `player_inventory`, `entity_info`, `game_state`, `master_triggered`).
5. Act through the port's code paths: `select_weapon` (TS 3.0 ignores `usercmd.weaponselect`),
   `buy`/`buy_item` (the loadout bought at the next spawn), `bot_command` (`usepowerup`,
   `respawn`, `say`), `drop_weapon`, `kick_bot`.
6. Hear the game through the descriptor's callbacks: spawns, damage, deaths, kills, rounds,
   sounds (hearing), weapon and ammo changes, client commands (`rcbot` typed by a player).

RCBot's needs from the research (issue #104's comment) map as: its engine hooks → the
callbacks; message sniffing → `player_info`/`player_inventory`/`weapon_changed`/`ammo_changed`;
`FakeClientCommand` → `bot_command`; `weaponselect` → `select_weapon`; the free-weapon loadout →
`buy`; model-string teams → `PlayerInfo::team`; `IsTriggered`/`Classify` virtuals →
`master_triggered`/`entity_info`; `pfnRunPlayerMove` → `run_cmd`; `rcbot/waypoints/ts` →
`data_dir`.

## 6. Tests

* Unit (`cargo test`): the native rules' 3.0 defaults (`bots::rules`) and TS 2.1's overrides (`ts21_server::bots::tests`: timers, gate, view, button slots, texts, buy and state helpers), backend selection, quota names, the loader (plugin-api.md §6), the API's
  bot checks, the stub's descriptor.
* `cargo xtask plugins [roots...]` (all seven roots by default, in parallel, ~1 min): installs
  `tsmod_test` and fixtures the loader must skip (`tsmod_test_badver`, `_badsize`, `_refuse`, a
  copy of the server library as `tsmod_noexport`, the other architecture's build as
  `tsmod_wrongarch`, junk, another platform's name), runs a dedicated server on ts_dojo whose
  first config (the server config, on the first map) sets `ts_bot_backend test` (one reload on the
  Steam engines, none on FWGS, which ran the config before the first map; no `changelevel`) and `ts_gameplay` 21 and back (no reload), and whose second (`ts_bot_backend
  TEST`: no reload) adds bots with
  the quota, `addbot` and `addcustombot`. Checks: each skip line, `loaded tsmod_test`, `backend
  test`, at most one `reloading` line and only the backend's, and the stub's `check <what> ok` lines (data_dir, map_name, create_bot, run_cmd,
  player_info, entity_info, game_state, inventory, bot_command, client_command, spawned, buy,
  buy_item, moved more than 64 units, sound), no `FAIL`, no crash. The fixtures are removed
  afterwards and by every `install`, so other scenarios run without plugins.
* Native parity: `cargo xtask bots`, `play`, `test` unchanged.

## 7. Deviations and limits

* The layer, the cvar and the API are port additions.
* Plugin bots count as `CBot`s (`IsNetClient()` false, no starting cash), as TS's bots do.
* A backend switch reloads the map and kicks the old backend's bots at its start, including
  `addbot` ones.
* `create_bot` is refused unless the caller's backend is active, so a backend sees only its own
  bots.
* Licence: `ts-plugin-api` has no explicit licence yet; issue #104 §5 leaves the choice
  (MIT/Apache-2.0) to the maintainers.
