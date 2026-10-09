# Validation

The port is validated on six engine roots with the local TS 3.0 content. This page covers how
each root was set up, how to run the checks, and the latest results.

## 1. Engine roots

| Root | Engine | Build | OS | Target installed |
|---|---|---|---|---|
| `fwgs` | Xash3D FWGS (built from `../xash3d-fwgs`, `--64bits`) | 0.21 (2026) | Linux, native | `x86_64-unknown-linux-gnu` |
| `fwgs-win` | Xash3D FWGS 0.21 win32 (from `../xash3d-ts`) | 0.21 | Windows, Wine | `i686-pc-windows-gnu` |
| `rehlds-linux` | ReHLDS built from `../ReHLDS` on HLDS `steam_legacy` | 3.x / HLDS 8684 | Linux | `i686-unknown-linux-gnu` |
| `rehlds-win` | ReHLDS 3.15.0.896 release on HLDS `steam_legacy` | HLDS 8684 | Windows, Wine | `i686-pc-windows-gnu` |
| `hlds25-linux` | Stock Valve HLDS, `public` branch (post-anniversary) | 10211 | Linux | `i686-unknown-linux-gnu` |
| `hlds25-win` | Stock Valve HLDS, `public` branch (post-anniversary) | 10210 | Windows, Wine | `i686-pc-windows-gnu` |

### 1.1 Setting the roots up

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

```sh
cargo xtask roots                       # which roots exist
cargo xtask install all                 # assemble <root>/ts and install libraries
cargo xtask server rehlds-linux --map ts_awaken --bots 4 --seconds 60
cargo xtask test                        # install + server check on every root
```

A server check passes when all of these hold:

* the server answers a status query (A2S_INFO, or the Xash `info` query on FWGS 0.21) with the expected map,
* at least `--bots` players are connected after `--seconds`,
* the log contains `[ts] GameInit`,
* there is no panic, fatal error or segfault.

Client checks run the FWGS client with an autotest cfg (`wait` chains, commands, `screenshot`,
`quit`) and inspect the screenshots. FWGS joins GoldSrc-protocol servers with
`connect host:port gs`. That covers the full chain: the Rust client on FWGS, connected to the
Rust server on ReHLDS or HLDS.

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

* A Steam GoldSrc **client**, pre- or post-anniversary. Only FWGS clients were available. Steam
  dedicated servers (HLDS 8684 via ReHLDS, and HL25) were tested.
* Real keyboard and mouse (all client runs are scripted), including the Steam mouse paths in
  `input.rs`.
* Native Windows (only Wine).
* Long sessions, many players, and real network latency.

### 3.3 Skeleton milestone (before the gameplay packages)

The server library loaded on all six roots. The FWGS client joined ReHLDS (`TSHealth` shown in
the HUD), and the Windows client library loaded in FWGS win32.

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
| `SteamAPI_Init() failed` in HLDS logs | Normal for dedicated servers (they use the game-server API) | none |
