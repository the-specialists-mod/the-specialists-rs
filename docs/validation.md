# Validation

The port is validated on six engine roots with the local TS 3.0 content. This page covers how
each root was set up, how to run the checks, and the latest results.

## 1. Engine roots

| Root | Engine | Build | OS | Target installed |
|---|---|---|---|---|
| `fwgs` | Xash3D FWGS (built from `../xash3d-fwgs`, `--64bits`) | 0.21 (2026) | Linux, native | `x86_64-unknown-linux-gnu` |
| `fwgs-win` | Xash3D FWGS 0.21 win32 (from `../xash3d-ts`) | 0.21 | Windows, Wine | `i686-pc-windows-gnu` |
| `fwgs-win64` | Xash3D FWGS `win32-amd64` (the FWGS `continuous` release) | 0.21 | Windows, Wine | `x86_64-pc-windows-gnu` |
| `rehlds-linux` | ReHLDS built from `../ReHLDS` on HLDS `steam_legacy` | 3.x / HLDS 8684 | Linux | `i686-unknown-linux-gnu` |
| `rehlds-win` | ReHLDS 3.15.0.896 release on HLDS `steam_legacy` | HLDS 8684 | Windows, Wine | `i686-pc-windows-gnu` |
| `hlds25-linux` | Stock Valve HLDS, `public` branch (post-anniversary) | 10211 | Linux | `i686-unknown-linux-gnu` |
| `hlds25-win` | Stock Valve HLDS, `public` branch (post-anniversary) | 10210 | Windows, Wine | `i686-pc-windows-gnu` |

### 1.1 Setting the roots up

`fwgs-win64` is the FWGS `continuous` release's `xash3d-fwgs-win32-amd64.7z` unpacked into
`~/games/xash_win64`, with `valve/` filled from the `fwgs-win` root (`cargo xtask install
fwgs-win64` links the TS content into `ts/`). Before it existed the windows-x86_64 server library
had the wrong name (issue #101).

```sh
# steamcmd (one instance per install: steamcmd remembers -beta per app, see Pitfalls)
mkdir -p ~/games/steamcmd && cd ~/games/steamcmd
curl -sSL https://steamcdn-a.akamaihd.net/client/installer/steamcmd_linux.tar.gz | tar xz
./steamcmd.sh +force_install_dir ~/games/hlds_linux +login anonymous \
    +app_set_config 90 mod valve +app_update 90 -beta steam_legacy validate +quit
# Windows files: a separate steamcmd copy with +@sSteamCmdForcePlatformType windows
# HL25: the same with -beta public, into ~/games/hlds25_linux and ~/games/hlds25_win

# ReHLDS for Linux, from source
cmake -S ../ReHLDS -B ../ReHLDS/build -DCMAKE_BUILD_TYPE=Release -DCMAKE_POLICY_VERSION_MINIMUM=3.5
cmake --build ../ReHLDS/build -j
cp ../ReHLDS/build/rehlds/{engine_i486.so,dedicated/hlds_linux,filesystem/FileSystem_Stdio/filesystem_stdio.so} ~/games/hlds_linux/

# ReHLDS for Windows: the official release binaries
gh release download -R rehlds/ReHLDS -p '*.zip'   # rehlds-bin-3.15.0.896.zip
cp bin/win32/{swds.dll,hlds.exe} ~/games/hlds_win/ && cp bin/win32/filesystem_stdio.dll ~/games/hlds_win/FileSystem_Stdio.dll

# HLDS on Linux needs the Steam client library in the place the Steam API looks
mkdir -p ~/.steam/sdk32 && ln -sf ~/games/hlds_linux/steamclient.so ~/.steam/sdk32/steamclient.so

# Wine prefix for the Windows servers (headless: no Mono/Gecko installer dialogs)
WINEPREFIX=~/games/wineprefix WINEDLLOVERRIDES="mscoree,mshtml=" wineboot --init
```

Packages used on Arch: `rustup cmake lib32-glibc lib32-gcc-libs mingw-w64-gcc wine strace`.

## 2. Running the checks

TS content: xtask's dev and test commands link `TS_DIR`, by default `../ts_custom` (TS 3.0 plus
custom maps such as `MecklenburgV_Rain_Final`, which `meck` needs), found from the checkout's
parent directories so worktrees under `.claude/worktrees` use it too. The stock TS 3.0 content,
`../ts_original` (`TS_ORIGINAL_DIR`), is only packaged:

```sh
cargo xtask package-xash-win   # dist --release (windows-i686), then ~/ts-xash-win-release.tar.gz
```

The archive's top directory `ts-xash-win/` holds the FWGS win32 engine from `../xash3d-ts`
(`XASH_WIN_DIR`; symlinks followed, without its `ts/`, `.git` and `engine.log`), `ts/` = the stock
content (symlinks followed, without `scrshots/`, `xtask_*`, `condump*.txt`, `custom.hpk`,
`voice_ban.dt` and the Linux `dlls/ts_i386.so`/`ts_i686.so`) with `dist/windows-i686/ts` over it,
and a CRLF `README-RUST-PORT.txt` naming the commit. `TS_PACKAGE_OUT` changes the output path;
the system `tar` writes it. The path rules are unit-tested (`xtask/src/package.rs`).

```sh
cargo xtask lint                        # no engine-call macros in game crates; TS 2.1 only in crates/ts21; the 3.0 tests
                                        # without the ts21 feature; clippy -D warnings on every target
cargo xtask roots                       # which roots exist
cargo xtask install all                 # assemble <root>/ts and install libraries
cargo xtask server rehlds-linux --map ts_awaken --bots 4 --seconds 60
cargo xtask all                         # everything below, about 4.5 min
cargo xtask test                        # dedicated server + 2 bots, 15 s, every root in parallel (~25 s)
cargo xtask play                        # gameplay on a listen server, fwgs and fwgs-win in parallel (~4 min, meck and bots included)
cargo xtask meck                        # grenades on MecklenburgV_Rain_Final only (~30 s fwgs, ~1 min fwgs-win)
cargo xtask bots                        # TS's bot AI on ts_dojo only: testpath, four addbot bots, botlearn (~45 s)
```

`cargo xtask lint` runs `cargo clippy --workspace --all-targets -- -D warnings` for each of the
four targets, so the Windows-only code (`goldsrc::platform::win`, the WinMM joystick) is linted
too; `xtask` itself is Unix host tooling and is linted on the Linux targets only. Each game
crate's `clippy.toml` lists `goldsrc`'s engine-call macros under `disallowed-macros`, a second
guard next to the source check in `xtask/src/lint.rs`.

The roots run in parallel: each one's output is printed as a block when it finishes. Windows
roots share one Wine prefix, so `wineserver -k` runs once at the end instead of after each root.

A server check passes when all of these hold:

* the server answers a status query (A2S_INFO, or the Xash `info` query on FWGS 0.21) with the expected map,
* at least `--bots` players are connected after `--seconds`,
* the log contains `[ts] GameInit`,
* there is no panic, fatal error or segfault.

A line ending in "(port guard)" (with `developer` on) means the game skipped or changed an engine
call that TS makes and the engine would stop on (`implementation/integration.md` §6,
`stock-entities.md` §7.8). It isn't a failure, but on the stock maps none is expected apart from
a respawned `cycler_weapon`'s empty precache on the Steam engines. The guards' decisions are
unit-tested in `goldsrc::guard` and `entities` (`cargo test`). The client prints its own
"(port guard)" lines to the console, once per kind (`implementation/integration.md` §6.1; tests in
`goldsrc::cl_guard`).

Client checks run the FWGS client with an autotest cfg (`wait` chains, commands, `screenshot`,
`quit`) and inspect the screenshots. FWGS joins GoldSrc-protocol servers with
`connect host:port gs`. That covers the full chain: the Rust client on FWGS, connected to the
Rust server on ReHLDS or HLDS.

The gameplay check (`cargo xtask play [<fwgs-root>...]`) is one listen-server run on `ts_dojo`,
on the native and the win32 FWGS builds at once. It plays these scenes in order and reads the
results from `ts_status` dumps, kill notices and developer log lines:

| Scene | How | Passes when |
|---|---|---|
| Join | The buy menu opens with a random first loadout: reset it, pick an AK47 with a scope and confirm (`slotN`, as the number keys do), click `+attack` after the respawn wait | `[ts] <host> spawned (#1) with [13]` |
| Wall | `ts_dev_fxlog 1`, stand in front of the dojo's front wall (`100 -760 60`, yaw 70) and fire three single shots | From the `[ts fx]` lines: every first-person flash is more ahead of the eye than off the view axis, within 8 units of the view model's muzzle, and spreads along the view (direction dot > 0.95); three bullets start within 64 units of the shooter and hit within 400; at least three impacts and three decals, all on the wall in front |
| Bot | `ts_bot_stop 1` (the bot stands still and holds fire), `ts_bot_quota 1` (`addbot` bots get `bots.txt` names, so the scenes use a quota bot; it runs TS's AI like an `addbot` bot) | The quota reads 1 and a bot was added |
| Lasers | The host and the bot get akimbo Berettas with a laser (`ts_give 2_2` twice, `ts_as 2 ...` for the bot), the host switches to free semi and looks at the bot from the side, `ts_dev_fxlog 1` | From the `[ts fx] laser` lines: two first-person beams, each starting at its gun's muzzle in front of the eye (the right gun right of the left one); the bot's beam starts 16 units along it from its `p_` model's gun hand, which is 4–64 units from its origin (attachments from the drawn model, not the origin FWGS resets them to) |
| Akimbo | Free semi: `+attack` and `+attack2` in one frame, then 0.3 s apart | From the `[ts fx] akimbo shot` lines: a linked double shot, then the `b0` gun, then the `b1` gun |
| Power-up | Stand on the slow-motion `ts_powerup` (`0 -672 48`, pwuptype 1) with the bot in view, double-tap `+use` | Held `0x1` after the pickup. Then active `0x1`, host time scale 0.6, bot 0.35 |
| Grenade | `ts_give 24`, `weapon_24`, aim at the 1-HP pillar `*15`, throw (`+attack`, release), move away | `func_breakable ... broken by <host> (grenade, ...)`: the explosion (bits `0x1040`) or the grenade still in flight (`0x1080`), which credits the thrower too (issue #1) |
| Kill | Put the bot in front of the host, aim and fire the AK47 | `<host> killed Bot ...` and a frag |
| Kick | `kick` the bot the host just killed | No crash (a dropped bot used to crash the engine on its "respawn"), and the quota adds a replacement |
| Death and respawn | `ts_hurt 1 500`, click through the death cam, click after the wait | Dead with a death, then a second spawn line and alive |
| Bots | Host suicides and stays observer; `ts_bot_stop 0`, `ts_bot_quota 4`, 70 s of play (TS's AI: the shipped ts_dojo node graph, TS's random loadouts) | Four bots. Within one life, all but at most one of the bots measured (at least two) move over 64 units between dumps 5 s apart. At least 2 `Bot ... killed Bot ...` lines |
| Gameplay switch | With the four bots playing: `ts_gameplay 21`, 10 s, `ts_gameplay 30`, then the host joins through the buy menu as in Join (`implementation/gameplay-plugins.md` §4) | Each switch logs `[ts] gameplay: ruleset A -> B: game reset (...)` with every player and map entities spawned again. 1.5 s later (the reset's second half runs after 0.6 s) every player has 0 frags and 0 deaths and none is alive (fresh joins), where someone had scored before. The client logs a HUD reset for each switch. The bots are alive again after 10 s, and the host is alive after rejoining |
| Quota down | `ts_bot_quota 0` | The bots are kicked |
| addbot | `addbot` on the listen server | "<name> connected with skill", a player with a `bots.txt` name in `ts_status`, no "Bots only in listen servers.", and the quota (0) doesn't kick it |
| Escape | `ts_dev_keylog 1`, `cancelselect` twice (Escape's binding; the first may close the weapon menu) | `[ts key] cancelselect: escape`: the client sent `escape`, which opens the engine's main menu |

Screenshots go to `target/play-<root>-N.png` (`-2` is the wall scene). The script can't close the
MOTD, which needs a real Enter key or click, so the scripts set `motdfile` to a file that doesn't
exist and the server sends no MOTD.

The wall scene catches the bug where the client played its predicted weapon events with a zero
origin and zero angles: the shooter's own bullets then flew from the world origin along +x (the
impacts and decals landed there, out of sight) and the first-person flash streaked along world +x
whatever the view (`implementation/client-rendering.md`, "Event parameters"). With that bug put
back, the scene reports `direction-dot 0.342` (cos 70°) and bullets from `[0, 0, 28]`, and all
three wall checks fail.

After that, `play` runs the grenade check on a large third-party map (`cargo xtask meck` runs it on
its own): a listen server on `MecklenburgV_Rain_Final` (37 `func_breakable`s, 158 doors, rain
emitters, fog and wind). Screenshots go to `target/meck-<root>-N.png`.

| Scene | How | Passes when |
|---|---|---|
| Join and bot | As above; one stopped bot | The host spawned alive and a bot was added |
| Grenade 1 | `ts_give 24`, `weapon_24`, stand between the two 10-HP walls `*113`/`*114`, look straight down (pitch 89) and throw, so the M61 drops at the host's feet; teleport away and move the bot onto the grenade | At least one `func_breakable ... broken by <host> (grenade, ...)` before the second throw. A player takes half of the M61's 110, so the bot is left at 45 HP |
| Grenade 2 | The same between the two 50-HP walls `*217`/`*220` | More walls broken (at least three in all), `<host> killed Bot ... with M61 Grenade` and a frag |
| Safety | The host waits at a far spawn point while each grenade goes off | The host is alive at the end; no "(port guard)" line, panic or crash |

The map's doors exposed a bug in the `SOLID_BSP` guard (`stock-entities.md` §7.8): brush
entities link before `SET_MODEL`, and the guard had turned every door into `SOLID_BBOX`. The
"no port guard" check catches a regression.

Then `play` runs the TS bot check (`cargo xtask bots` runs it on its own): TS's bot AI (`mp.dll`
only; `implementation/rules.md` §10) on its own ts_dojo listen server. It needs a server of its
own: the play scene's quota kicks wait behind its script (below), and `maxplayers 8` then leaves
no room for four more bots. The host joins, spawns and suicides, so it watches as an observer.
Screenshot in `target/bots-<root>-1.png`.

| Scene | How | Passes when |
|---|---|---|
| testpath | `botlearn 2` (TS prints `FindPath`'s result only then), `testpath 0 50`, `testpath 10 100`, `testpath 89 25` on the shipped ts_dojo graph | `path:0,47,50`, `path:10,15,63,84,100` and `path:89,88,25`, which is what the original TS 3.0 prints (checked under FWGS win32 with TS's own `mp.dll`; `scripts/ndm.py` gives the first two, see `implementation/rules.md` §10.2) |
| TS bots | Four `addbot`s (TS's `BotCreate`: `bots.txt` names, random models and skills, TS's random loadouts), 40 s of play with `botlearn 2`, `ts_status` every 5 s | Four bots. At least two `(debug)arrived` lines: TS's print for each node path a bot plans to a roam goal, so the bots navigate over the graph. All but at most one of the bots measured move over 64 units within a life, and at least one kill between the bots |
| botlearn | `resetnodes`, 6 s with `botlearn 2`, then `loadnodes` and `botlearn 0` | `(debug)1 nodes`, `(debug)2 nodes`, `(debug)3 nodes`: the bots' walk builds a new graph (each new node prints the count). `savenodes` isn't run: the roots link the content's `maps` directory |

The `path:` replies can show in the client's console after the bots have joined, so the check
searches the whole scene for them.

How the scripts run:
* Game time runs at 2x (`sys_timescale 2`, with `fps_max 100`), and waits are written in game
  seconds (`secs()` in xtask).
* Waits use the `w10`/`w100` aliases. Plain `wait` chains overflow FWGS's 32 KB command buffer
  (`Cbuf_InsertTextToBuffer: overflow`), and the engine then skips the whole cfg.
* The script ends with `ts_dev_touch xtask_<tag>_done`, a marker file xtask polls for. The engine
  log can sit in a write buffer until exit under Wine, so waiting for a line in it used to hold
  the Wine run until its time limit.
* Scripts don't `quit`. Server commands the game queues (the quota's `kick`) wait behind the
  script in the console buffer, so xtask stops the game a few seconds after the marker instead.
* Scripted `+attack` gets past the client's anti-script gate through `ts_dev_scriptinput 1`
  (with `sv_cheats 1`).

Port-only dev aids used by the tests (all need `sv_cheats 1` except `ts_status`):

* `ts_tp <player#> <x> <y> <z> [yaw] [pitch]` teleports a player, drops them to the floor and
  can set their view.
* `ts_face <player#> <target#>` or `ts_face <player#> <x> <y> <z>` aims a player's view.
* `ts_bot_stop 1` makes living bots stand still and hold fire (their AI is skipped). Dead and
  observing bots still buy their loadout and respawn.
* `ts_dev_touch <file>` creates an empty file in the game directory (the end-of-script marker).
* `ts_dev_hamlog 1|0` installs the server's own entity hook and prints a `[ts ham]` line before and
  after every Spawn, TakeDamage and Killed (`implementation/plugin-api.md` §7.6). Refused while a
  plugin's hook is installed.
* `ts_observe <player#> <mode> [target#]` puts an observer in a spectator mode (5 is in-eye) on a
  target, for clients whose input a script can't drive (the original TS client, below).
* `ts_dev_fxlog 1` (client) prints a `[ts fx]` line for each predicted shot's flash, bullet,
  impact and decal, each akimbo shot (`akimbo shot #<ent> weapon <id> linked|gun b0|gun b1`), and,
  at most twice a second, the laser beams (`laser local gun <n> beam ... muzzle ... ahead/right/up`
  for the first-person ones, `laser other #<ent> beam ... hand ... origin ... eye ... aim ...` for
  other players').
* `ts_dev_keylog 1` (client) prints `[ts key]` lines: `HUD_Key_Event`'s answer for Escape
  (`-> 1` hands it to the engine) and what `cancelselect` did.
* `ts_as <player#> <command> [args...]` runs a client command as that player (a bot), e.g.
  `ts_as 2 ts_give 2_2`.
* `ts_status`, `ts_hurt`, `ts_givepowerup`, `ts_give` (existing).
* With `developer` on, each broken `func_breakable` is logged with its attacker, inflictor class
  and damage bits.

### 2.x Plugins

`cargo xtask plugins [roots...]`: a dedicated server per root with the stub plugin `tsmod_test`
and the loader's fixtures; see implementation/bot-plugins.md §6 for what it checks.

## 3. Results

### 3.1 Final run (all work packages merged)

`cargo test`: all suites pass (162 tests). Builds for host, `i686-unknown-linux-gnu` and
`i686-pc-windows-gnu` have no warnings. The server matrix below passed with both debug and
**release** builds (`cargo xtask --release test --map ts_lobby`: all six roots, bots scoring).

**Dedicated servers** (`cargo xtask test --map ts_awaken`, 2 bots, 25 s; also passed earlier on
`ts_lobby`):

| Root | Result | Bot scores after 25 s (A2S_PLAYER) |
|---|---|---|
| `fwgs` | pass | 0 / 0 |
| `fwgs-win` | pass (bots counted from the log; this engine's query doesn't count them) | 0 / 0 |
| `rehlds-linux` | pass | 1 / 0 |
| `rehlds-win` | pass | 0 / 0 |
| `hlds25-linux` | pass | 0 / 0 |
| `hlds25-win` | pass | 0 / 1 |

With 4 bots for 90 s on FWGS the scores were 3 / 0 / 1 / 0 (kills vary between runs, 0–7).

**Rust client joining each server** (`cargo xtask client`: FWGS amd64 client with the Rust client
library, GoldSrc protocol for ReHLDS and HLDS; server with 2 bots, 60 s):

| Server root | Result |
|---|---|
| `fwgs` | pass |
| `rehlds-linux` | pass |
| `rehlds-win` | pass |
| `hlds25-linux` | pass |
| `hlds25-win` | pass |

The client reaches the game: the MOTD, the TS loadout menu, the weapon grid with TS sprites, free
slots, "Press Fire To Play!", and the bots ([screenshot](img/rust-client-vs-rehlds.png)).

**Alive gameplay** (FWGS listen server, scripted: spawn, `ts_give 13_8 1_0`, fire, reload, switch
to pistol): AK47 view model with scope, shells, TS crosshair with spread ring, HUD with health,
stance, free slots and weapon panel. The clip counts down from client prediction, and the client
log has no prediction errors or panics ([screenshot](img/alive-ak47-fwgs.png)).

**Wire compatibility with the original TS 3.0 client:** the unmodified TS 3.0 `client.dll`
(SHA-256 `1811bd4f…`, in FWGS 0.21 win32 under Wine) joined the Rust server on ReHLDS Linux. It
showed the VGUI MOTD, the loadout menu, the weapon grid fed by our weapon messages, "Free Slots",
and the chase camera on a bot, with no user-message errors in its log
([screenshot](img/original-client-vs-rust-server.png)).

**Windows client library:** the Rust `client.dll` and `mp.dll` load and render in FWGS 0.21
win32 under Wine (listen server, `ts_arena`).

### 3.2 Not validated

* `cargo xtask plugins` (the `tsmod_*` loader and the `tsmod_test` bot backend on all seven roots,
  [implementation/bot-plugins.md](implementation/bot-plugins.md) §6) was added with issue #104 and
  not yet run in this document's runs; its checks are unit tested against a synthetic log.

* **Save games and level transitions** (#104): the save/restore code is covered by unit tests
  only (`cargo test`: the codec's byte layout, every linked class walked against TS's save tables,
  every class round-tripped with all fields filled, the global state's blocks). No xtask scenario
  saves, loads or crosses a `trigger_changelevel` yet, and no save written by TS's own server has
  been loaded into the port (or the other way round).
* A Steam GoldSrc **client** beyond the scripted HL25 Linux runs in §5 (listen server, and
  joining HL25 and ReHLDS Linux dedicated servers): the HL25 Windows client (#25), a real keyboard and mouse on either, the in-game menu opened with a real Esc
  key, and muting a second human in the GameUI player list. The pre-anniversary
  client (8684) is out of scope.
* Real keyboard and mouse (all client runs are scripted), including the Steam mouse paths in
  `input.rs`.
* Joysticks (Steam GoldSrc only: WinMM on Windows, SDL2 game controller on Linux; FWGS handles
  them in the engine).
* Voice chat with real microphones: head icons, speaker labels, the talk icon, scoreboard voice
  icons, squelch-mode muting and `voice_ban.dt`, and that Steam HLDS relays voice now that the
  server opens the streams (`Voice_SetClientListening`).
* Native Windows (only Wine).
* Long sessions, many players, and real network latency.
* The TS studio renderer and motion-blur ghosts (issues #2, #3) after they landed: unit tests
  only so far. Since the client now draws every studio model, the first `cargo xtask play` /
  `client` run should check that world models, the view model, players and their `p_` weapons
  still render, then the torso/legs split, corpses and ghosts (steps in
  `implementation/client-rendering.md`, "Test results").

* Drowning, slime and time-based damage (issue #38): unit tests only. In-engine: stay under
  water deep enough to submerge (candidates with `!` water textures: `ts_bikini`, `ts_mathar`,
  `ts_greenhouse`; depth not checked) for over 12 s and expect 3, 4, then 5 damage per second, a
  `pl_wade1` gasp on surfacing and 10 health back every 2 s. `SS_HQ_f3` has a slime
  `func_water` (skin -4, 4 × waterlevel per frame). No map in `~/dev/hl/ts/ts/maps` has lava or
  a poison `trigger_hurt` (`ts_gpc` has a nerve-gas one, which only counts down), so poison
  needs a test map.

* The plugin API's setters in a running game (`implementation/plugin-api.md`): unit tests cover
  the logic, and `CreateInterface` was called outside the engine. No AMX Mod X module has driven
  them in-engine yet; that is for the `tsmod-amxx-rs` modules. The entity hooks (version 2) ran
  in-engine only with `ts_dev_hamlog` on a FWGS dedicated server with bots (§7 of that doc); the
  hamsandwich port in `tsmod-amxx-rs` is to drive them under AMX Mod X. The private-data guard was checked
  under AMX Mod X on ReHLDS Linux and Windows (`ref/amxx-testenv`: hamsandwich fails cleanly,
  fakemeta pdata reads zeroed padding), not under xtask.

### 3.3 Skeleton milestone (before the gameplay packages)

The server library loaded on all six roots. The FWGS client joined ReHLDS (`TSHealth` shown in
the HUD), and the Windows client library loaded in FWGS win32.

### 3.4 Weapon effects against the original TS client

The original TS 3.0 client (`~/games/xash_win_orig`, FWGS 0.21 win32 under Wine, unmodified
`client.dll`) joins a port listen server on native FWGS (`connect 127.0.0.1:<port>`, Xash
protocol). The host runs an `xtask`-style script; `ts_observe 2 5 1` puts the original client in
in-eye mode on the host, who fires at the dojo's front wall from `100 -760 60` (yaw 70); the
original client's own cfg takes a screenshot every few frames. Findings:

* **Decals and impacts.** The original client shows `{shot` decals on the wall where the host's
  shots hit (events from the server carry the real origin and angles); the port's host shows the
  same decals, sparks, debris and smoke after the playback fix. Before it the port showed none at
  the wall: its predicted shots went to the world origin.
* **Flash.** In first person TS's flash comes from the per-player attachment cache, which for the
  local player holds the view model's muzzle on both engines (`client-rendering.md`, "Event
  parameters"); the port's log shows the cached point and the view model's attachment 0 are the
  same. In-eye, the original's flash glows in the middle of the view, away from the barrel (the
  target's cache is its origin); the port flashes at the view model's muzzle there (deviation).
* **Not done:** firing from the original client itself. Its anti-script gate needs a real key or
  click, and keys sent into the Wine window (`SendInput`, posted `WM_KEYDOWN`) didn't reach it.
* Seen on both clients, not investigated: the in-eye view pitches up, away from where the host
  aims, after `ts_tp` set the host's view.

### 3.5 TS's bot AI

The port of TS's bot AI (`implementation/rules.md` §10) was checked three ways:

* **Unit tests** (`cargo test -p ts-server bots`): the node graph against all 43 shipped `.ndm`
  files and `FindPath` against the decomp's replica on 30,809 node pairs over five maps (all
  pairs of ts_dojo and ts_japa), every one matching; the text files, name picker and brain
  helpers.
* **In engine** (`cargo xtask bots`, part of `play`), on both FWGS roots: `testpath` prints
  TS's paths; four `addbot` bots roam ts_dojo over the node graph (TS's `(debug)arrived` print
  for every path they plan: 15–20 in 40 s), move 450–1350 units within a life and kill each
  other (2–4 kills); with `resetnodes` and `botlearn 2` their walk learns a new graph. The play
  scene's four quota bots (now TS bots too) scored 9 bot-on-bot kills in 70 s with TS's random
  loadouts (Benelli, Glock-20C, knives, ...), and bot chat from `bot_say.txt` appears. The
  dedicated-server matrix passes with TS bots thinking on all six roots (port addition), and
  each root logs `[ts] bots: 64 names, chat lines [25, 29, 42], <n> nodes` at map start.
* **Against the original** TS 3.0 (FWGS win32 under Wine with TS's own `mp.dll` and client,
  `~/games/xash_win_orig`, a temporary cfg only): `addbot` bots fight there too, and with
  `botlearn 2` its `testpath 89 25`, `testpath 0 50` and `testpath 10 100` print
  `path:89,88,25`, `path:0,47,50` and `path:10,15,63,84,100` (read from screenshots: the
  original client shows those lines on the HUD only). The port prints the same three.
  `scripts/ndm.py` gives `89,87,25` for the first: it rounds the walk heuristic to a float,
  which the x87 code (`fsqrt` straight into `__ftol`, 0x100719eb) doesn't.

## 4. Pitfalls found during setup

| Symptom | Cause | Fix |
|---|---|---|
| Stock HLDS: `Couldn't get DLL API from <garbage>` | Stock HLDS (8684 and HL25) strips `_i386` from `gamedll_linux` and loads `dlls/ts.so`. ReHLDS loads the name as written. | xtask installs both `ts_i386.so` and `ts.so`. |
| "HL25" install reported build 8684 | steamcmd remembered `-beta steam_legacy` for app 90 from an earlier run | Use a fresh steamcmd copy and `-beta public` explicitly. Check `BetaKey` in `steamapps/appmanifest_90.acf`. |
| Windows HLDS under Wine: no output, no answer | First-run Wine prefix setup (Mono/Gecko dialogs) blocked startup; a half-created prefix then failed to load `kernel32.dll` | Create the prefix headlessly (§1.1). `hlds.exe -console` writes to its own console window: use `-condebug` (`qconsole.log`). |
| Windows HLDS answers only after 20–30 s on the first start | Wine plus `server.cfg` plus VAC initialisation | xtask waits up to 120 s for Windows roots. |
| FWGS 0.21 win32 doesn't answer A2S_INFO | Old FWGS answers only the Xash `info` query | xtask falls back to `info 49`. |
| Release `mp.dll` never loads on HL25 Windows (crash in `swds.dll` right after `LoadLibrary`); the debug build works | The Steam engine reads the DLL's export names from the file: it finds the section *named* `.rdata` and translates the export RVAs with it (MSVC puts exports there). MinGW emits `.edata`, so the engine walks garbage. The debug build only survived because the garbage happened to be readable memory. | xtask renames `.rdata` to `.rodata` and `.edata` to `.rdata` in every Windows DLL it installs or packages (`msvc_style_export_section`). |
| Bots never join on GoldSrc; fine on FWGS | Command-line `+cvar` runs before GameInit registers the game's cvars | xtask writes the quota into `xtask_server.cfg` and passes `+servercfgfile`. |
| HL25 crashes after the last human leaves | HL25 drops every bot then; driving a dropped bot with `pfnRunPlayerMove` crashes `SV_RunCmd` | Bots are driven only while they have a live user id. |
| Crash in `pfnSetClientMaxspeed` (FWGS) after a bot is kicked; the Windows dialog shows `Host_Shutdown (xash.dll)` under `mp.dll` frames | A dropped client keeps `FL_CLIENT`/`FL_FAKECLIENT`, and FWGS still maps its edict to the freed slot: `pfnGetPlayerUserId` returns the old id, and `pfnSetClientMaxspeed` dereferences the slot's NULL edict when the bot respawns | `ClientDisconnect` clears both flags, so dropped players drop out of `players()`, `is_player()` and the bot list, and `rules::set_client_maxspeed` skips the engine call for them (TS's own player tests still see them: `has_player`, `ts_flags`, `slot_players`) |
| `addbot` bot kicked half a second later | `ts_bot_quota` (default 0) kicked bots above the quota | The quota counts and kicks only its own bots; `addbot` bots (listen servers, as TS) are left alone |
| `addbot` did nothing on a listen server ("Create Server") | The port followed the Linux `ts_i386.so`, a dedicated-only build whose `addbot` only prints "Bots only in listen servers." | `addbot`/`addcustombot` add a bot as TS's Windows `mp.dll` does on a listen server, and (a chosen deviation) on a dedicated server too. `xtask test` runs `addbot` from every dedicated server's config and fails unless a bot connects |
| Escape did nothing in game (FWGS 0.21 win32, Steam GoldSrc) | Those engines run Escape's hard binding, `cancelselect`, instead of opening the menu; TS's `cancelselect` sends `escape` when no weapon menu is open, the port's only closed the weapon menu (newer FWGS opens the menu itself) | `cancelselect` sends `escape` as TS (`hud::cancel_select`); `xtask play` checks it |
| Quota kicks never happen in a scripted test | `pfnServerCommand("kick ...")` is appended to the console buffer behind the rest of the running `wait` script | The listen script ends right after `ts_bot_quota 0`; xtask stops the game 10 s later |
| `SteamAPI_Init() failed` in HLDS logs | Normal for dedicated servers (they use the game-server API) | none |
| Steam HL25 client: the game runs, but the world is black and the "Loading… Parsing game info" dialog never goes away (issue #62) | A client without `IClientVGUI` is treated as VGUI1, and the engine then only draws the world when the client's VGUI1 viewport calls `VGui_ViewportPaintBackground` (TS's does). The port has no VGUI1 viewport, so `V_CalcRefdef` and `HUD_Redraw` were never called and the screen kept the dialog's last frame | `client-dll` exports `ClientFactory`/`CreateInterface` and an `IClientVGUI` with `UseVGUI1() == false` (`implementation/client-hud.md` §7) |
| Steam HL25 client: `ts_tp`/`ts_face` (and spawn angles) ignored, so scripted shots miss; `ts_test_view` shows the yaw stuck | While a menu owns the cursor (the MOTD stays open in scripts) `CL_CreateMove` held the view angles, overwriting the server's `svc_setangle`. Not seen on FWGS | Only the player's own turning is held; angles the engine changed since the last `CL_CreateMove` replace the held ones (`input::menu_hold`) |
| Steam client on a dedicated server: after `changelevel` the player stays an observer, clicks do nothing | The 0.2 s click timer (`respawn` on `+attack`) is in client time, which restarts with the map, so the old map's timer blocked every click | `spectator::vid_init` zeroes it, as TS's `CHudSpectator::VidInit` does |
| Third person after every death and respawn | Entering observer mode saved the death cam's third person and leaving restored it | Leaving re-applies `cl_thirdperson` (TS `Spectator` handler `10065250`) |
| `rcon changelevel` from the Steam client changes the map twice; the client then stalls on "Retrying" (`Skipping reconnect ..., no pvPrivateData` in the server log) | The HL25 client sometimes sends an `rcon` twice (seen for `ts_bot_quota` and `changelevel`) | Drive the dedicated server's console through its stdin (§5), not `rcon` |
| `ts_bot_quota` on `maxplayers 1` prints `[ts] bots: no free client slot` every 0.5 s | The quota retried with a message each time | A failed add is reported once per streak |
| Steam client can't join an FWGS dedicated server (`Retrying 127.0.0.1:...`, nothing in the server log) | FWGS servers speak protocol 49 only; the Steam client is protocol 48 | none (engine limitation) |
| `PF_MessageEnd_I: Unknown User Msg 114` ×3 on a Steam listen server (`developer 1`) | Message 114 is `SrvSett`. The first `CTSGameRules::Think` broadcasts keys 0–2 (`realbullet`, `usecash`, `ammocount`) before any client has received the user-message table, and the engine drops a message it hasn't registered with a client yet | Harmless: each client gets all four `SrvSett` values in `InitHUD` |
| Four more `addbot`s in the play scene: two fail with "Could not find any free client slots" | The quota's kicks of its four bots wait behind the script (above), and `maxplayers 8` | The TS bot check runs as its own listen server (`cargo xtask bots`) |
| `testpath`'s `path:` lines show up after later lines (the bots' joins) in the client's console | The client's say-text display, not the server (it prints at once) | The check searches the whole scene |
| `testpath 89 25` on ts_dojo: the port prints `path:89,88,25`, the decomp's `scripts/ndm.py` replica `89,87,25` | The replica rounds `FindPath`'s walk heuristic to a float before truncating it; `mp.dll` keeps it on the x87 stack (and the original TS 3.0 prints `89,88,25`) | The unit tests' reference data (`bots/testdata/gen.py`) is generated with that rounding removed |
| The original TS client shows `testpath`'s and `botlearn 2`'s replies on the HUD only, not in the console log | Its say-text print doesn't echo `TextMsg` talk lines to the console | Read them from a `screenshot` (the original's MOTD and buy menu cover the chat: `motdfile` to a missing file, `slot10`) |
| `savenodes` in a scripted test would overwrite the content's `.ndm` | The roots' `ts/maps` is a symlink to the content directory | The scripts never run `savenodes`; `resetnodes` is followed by `loadnodes` |

## 5. Steam Half-Life client (HL25, issue #23)

The Steam client isn't one of xtask's roots: it needs a logged-in Steam, and only one copy can run
at a time. This is the manual setup used for issue #62, on the Linux 25th-anniversary client
(32-bit `hl_linux`, engine build 10211, app build 15961492).

1. Steam running and logged in, Half-Life (app 70) installed at
   `~/.local/share/Steam/steamapps/common/Half-Life`.
2. Assemble `Half-Life/ts` the way `cargo xtask install` does: symlink every entry of the TS
   content directory (`../ts_custom`: maps, models, sound, wads, `resource`, ...) into it, copy the files
   from `game/ts` over the links, and install the i686 Linux build:

   ```sh
   cargo build -p ts-client-dll -p ts-server-dll --target i686-unknown-linux-gnu
   T=target/i686-unknown-linux-gnu/debug HL=~/.local/share/Steam/steamapps/common/Half-Life
   cp $T/libclient.so $HL/ts/cl_dlls/client.so
   cp $T/libts.so $HL/ts/dlls/ts_i386.so && cp $T/libts.so $HL/ts/dlls/ts.so
   ```

3. Write a cfg in `Half-Life/ts` (the `xtask play` script works as is, ending in `quit`):
   `developer 1`, `sv_lan 1`, `maxplayers 8`, `lservercfgfile ""`, `sv_cheats 1`,
   `ts_dev_scriptinput 1`, `map ts_dojo`, then `w10`/`w100` waits, commands, `screenshot`, `quit`.
4. Launch through the Steam runtime, without `steam -applaunch`:

   ```sh
   cd ~/.local/share/Steam/steamapps/common/Half-Life
   SteamAppId=70 SteamGameId=70 setsid ~/.local/share/Steam/ubuntu12_32/steam-runtime/run.sh \
     ./hl.sh -game ts -windowed -width 1280 -height 800 -condebug +exec <cfg> &
   ```

   The console goes to `Half-Life/qconsole.log`, screenshots to `Half-Life/ts/HalfLifeNN.tga`.
   Make sure the game exits (`quit`, or `pkill -x hl_linux`). It renders even while the desktop is
   locked.
5. Compare with stock Half-Life launched the same way (`-game valve`, `map crossfire`) to tell the
   port's problems from the setup's.

For debugging, `hl.sh` runs the game under gdb when `DEBUGGER=gdb` is set, and passes
`DEBUGGER_ARGS` through (`-batch -x <script>` with breakpoints and `run`). `hw.so` ships with its
symbols and DWARF, so breakpoints on engine functions (`VGui_ViewportPaintBackground`,
`CBaseUI::ActivateGameUI`) and `print cls.state` work.

**Result (issue #62 fixed):** the scripted `xtask play` scenes all pass on the Steam client
(listen server, `maxplayers 8`): join and buy, spawn, slow-motion power-up, grenade, kill, kick,
death and respawn, and four bots fighting. The world, HUD, buy menu, MOTD, view model and the
slow-motion indicator draw; the console and the game menu open over the game and give the mouse back.
The listen server did not start paused. Expected log noise: `DemoPlayer::Init: couldn't get
engine interface`, `System::AddModule: couldn't initialize module (null)`, `failed to locate
sequence file ts_dojo`, `runtime sample conversion` for three TS player sounds, and
`Unknown User Msg 114` (§4). The `ts_dojo` sky is white because the map's `construct_*` sky
images are white.

### 5.1 Steam client on dedicated servers, `maxplayers 1`, GameUI

**Dedicated server.** Start the server by hand with the dev aids on, and drive its console through
stdin so the client script can send server commands at the right moment (`rcon` from the HL25
client is sometimes sent twice, which turns one `changelevel` into two and strands the client):

```sh
cd ~/games/hlds25_linux          # or ~/games/hlds_linux (ReHLDS)
cat > ts/t23_server.cfg <<'CFG'
exec server.cfg
sv_cheats 1
ts_bot_quota 0
ts_bot_stop 1
developer 1
CFG
mkfifo /tmp/srv.fifo; sleep 100000 > /tmp/srv.fifo &   # keep a writer open
LD_LIBRARY_PATH=$PWD ./hlds_linux -condebug -game ts -port 27115 +maxplayers 8 +sv_lan 1 \
  +servercfgfile t23_server.cfg +map ts_dojo < /tmp/srv.fifo
```

The client cfg (`sv_cheats 1` and `ts_dev_scriptinput 1` on the client too, `fps_max 100`, the
`w10`/`w100` aliases) runs `connect 127.0.0.1:27115` and marks server commands with
`echo SRVCMD <command>`; a shell loop follows `qconsole.log` and writes each one to the fifo:

```sh
tail -F qconsole.log | grep --line-buffered SRVCMD | while read -r _ cmd; do echo "$cmd" > /tmp/srv.fifo; done
```

The script: wait 15 s, buy an AK47 with the `xtask play` key sequence and click to spawn
(`ts_status`); `ts_bot_quota 1`; `say` and `say_team`; `+showscores`; `ts_tp 1 0 -1344 60 90 0`,
`ts_tp 2 0 -1200 60 270 0`, then `+attack` with `ts_face 1 2` every 0.2 s for 5 s;
`ts_hurt 1 500`, click through the death cam and click to respawn; `changelevel ts_awaken`, wait
30 s, buy, click, `ts_status`, `+showscores`; `ts_test_view` after aiming and after each spawn.
A second run with `mp_teamplay 1` (set, then `changelevel` so the rules reinstall) checks the team
menu: auto-assigned team, `changeteam`, `slot2`.

| Check | `hlds25-linux` (10211) | `rehlds-linux` | `fwgs` |
|---|---|---|---|
| Join, MOTD, buy menu, click to spawn | pass | pass | can't join: FWGS servers only speak protocol 49 (§4) |
| Chat (`say`, `say_team`) in the client and the server log | pass | pass | – |
| Scoreboard (names, scores, ping, voice column) | pass | pass | – |
| Shoot a bot dead (kill notice, frag, cash) | pass, after the menu-angles fix | pass | – |
| Death cam, respawn, first person again | pass, after the third-person fix | pass | – |
| `changelevel` to `ts_awaken`: reconnect, buy menu ("Use Last Config"), spawn | pass, after the click-timer fix | pass | – |
| Team menu, `changeteam` (`mp_teamplay 1`) | pass | not run | – |
| Logs | expected noise only (below) | expected noise only | – |

Client log noise, all also present with stock Half-Life or explained in §4: `DemoPlayer::Init`,
`System::AddModule`, three `runtime sample conversion` warnings, and `.so: cannot open shared
object file` at exit. Server log: `couldn't exec maps/<map>_load.cfg`, `Cvar_Set: variable
room_type not found` and the Steam API lines. No voice errors (voice itself was not exercised).
The HUD's additive sprites vanish over the white `ts_dojo` sky (expected).

**`maxplayers 1` listen server** (`maxplayers 1`, `map ts_dojo`): join, buy, spawn, walk, fire,
`ts_hurt` death and respawn all work; the rules are deathmatch, as in TS. (Then, `addbot` reported
`no free client slot` once; on a listen server it now adds a bot, as TS's Windows server does.) The game does not pause while the game menu or the console is open:
server time keeps running. Stock Half-Life (`-game valve`, `map crossfire`, `maxplayers 1`) with
the menu opened the same way doesn't pause either, so this is the engine's behaviour.

**GameUI player list.** Nothing on the console opens it, so the run used gdb (`DEBUGGER=gdb
DEBUGGER_ARGS="-batch -x <script>" ./hl.sh ...`; `gameui.so` has symbols): a breakpoint on the
client's `ts_test_view` command runs

```
set language c++
call ((CBaseUI*)&g_BaseUI)->ActivateGameUI()
call ((CTaskbar*)g_pTaskbar)->OnOpenPlayerListDialog()
```

The dialog lists the players, with an empty Properties column and **Mute in-game voice**
disabled. `g_pGameClientExports` is NULL: `CGameUI::Initialize` asks the client factory for
`GameClientExports001`, the port returns NULL (as TS 3.0 does, it has no such interface), and
every use in `CPlayerListDialog` checks for NULL. So the GameUI list can't mute anyone; muting
works through the scoreboard's squelch mode, as in TS. The TS menu background (`resource/
BackgroundLayout.txt`) draws over the world while the menu is up.

**Listen server again:** the `xtask play` script (`~/games/xash/ts/xtask_play.cfg`, `quit`
appended) on the final build: kill, power-up (host 0.6, bot 0.35), death and respawn, and twelve
bot-on-bot kills.
