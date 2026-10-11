# AGENTS.md

Guidance for coding agents (and people) working in this repository: a Rust port of **The
Specialists 3.0** (TS), a GoldSrc / Half-Life mod. Both game libraries are ported: the server
(`mp.dll`, `ts_i386.so`) and the client (`client.dll`). The port is **wire-compatible with TS
3.0**, so the original TS client can join this server and this client speaks the same protocol.

Start with [README.md](README.md), then [docs/design.md](docs/design.md) (architecture and the
wire contract). Per-subsystem notes are in [docs/implementation/](docs/implementation/), and
testing is in [docs/validation.md](docs/validation.md).

## Sources of truth

| Question | Where to look |
|---|---|
| How TS behaves (numbers, timings, payloads, rules) | The decompilation, [the-specialists-decomp](https://github.com/CrunchyMonkies/the-specialists-decomp), checked out at `../ts30-decomp`. Start at its `docs/README.md`; server addresses use the Ghidra numbering described there. |
| What the engine does | `../xash3d-fwgs` (FWGS source; the bindings are generated from its headers), `../ReHLDS` (Steam dedicated server reimplementation), `../halflife-sdk` (Valve's C++ game code). |
| How this port is organised | `docs/design.md`, `docs/implementation/*.md`. |
| Known gaps | GitHub issues labelled `integration` (and `validation` for untested platforms). |

Port behaviour from the decomp, not from memory of the game. When the port deliberately differs
from TS, say so in the relevant `docs/implementation/*.md` "Deviations" section.

## Crate layout and the `unsafe` boundary

```
crates/
  goldsrc-sys/   bindgen output from the FWGS headers (generated per target)
  goldsrc/       safe wrappers over the engine: server/client function tables, edicts
                 (edict.rs), player-move callbacks (pmove.rs), native mouse (platform.rs), ...
  shared/        game logic both sides run: movement, weapons, protocol, TS constants
  server/        ts-server: server game logic (rlib)
  server-dll/    ts-server-dll: the server library the engine loads (cdylib "ts")
  client/        ts-client: client game logic (rlib)
  client-dll/    ts-client-dll: the client library the engine loads (cdylib "client")
xtask/           build, package, install and test tooling (`cargo xtask ...`)
game/ts/         liblist.gam, gameinfo.txt, delta.lst
```

**`unsafe` code lives only in `goldsrc`, `server-dll` and `client-dll`.** `ts-server`,
`ts-client` and `shared` are `#![deny(unsafe_code)]`. The rules:

1. **Never add `unsafe` to a game crate, and never weaken the lint.** The only `allow`s are the
   `goldsrc::export_entity_class!` macro (`#[no_mangle]` is an unsafe attribute in Rust 2024) and
   `shared`'s player-movement test module, which fakes the engine with `extern "C"` callbacks.
2. **Need new engine access? Add a safe wrapper to `goldsrc`.** Put it on `Engine`/`ClEngine`,
   the `EdictPtr` or `PlayerMoveExt` traits, or a module under `goldsrc/src/ext/` (e.g.
   `ext::tri` for the triangle API). Then call it from game code. Don't use
   `eng!`/`cl_eng!`/`ev!`/`efx!`/`tri!` in game crates: they expand to `unsafe` at the call site,
   and `deny(unsafe_code)` doesn't fire on another crate's macro expansion. A source check in
   `xtask/src/lint.rs` enforces this: it runs as a unit test (`cargo test`) and at the start of
   `cargo xtask build`, and lists the offending lines. Each game crate's `clippy.toml` also lists
   the macros under `disallowed-macros`, which `cargo xtask lint` turns into errors.
3. **A new engine callback or export goes in the `*-dll` crate.** Keep those functions thin:
   convert raw pointers to references (null-check), call a safe function in the game crate, and
   convert the result back. Logic belongs in the game crate. The one exception is
   `client-dll/src/tempents.rs`, the SDK temp-entity loop that walks the engine's linked list.
4. **Every `unsafe` block gets a `// SAFETY:` comment** saying why it holds. `goldsrc` and the
   `*-dll` crates warn on `clippy::undocumented_unsafe_blocks`; `cargo xtask lint` runs clippy
   with warnings as errors for all four targets, which matters because Windows-only code
   (`goldsrc::platform::win`, the WinMM joystick) is never compiled by a Linux clippy run.
5. **Entity pointers:** game code passes `*mut edict_t` around as a handle (holding a raw
   pointer is safe). Read and write through `goldsrc::edict::EdictPtr`: `e.v()` for `entvars_t`,
   `e.is_free()`, `e.has_private_data()`. `v()` returns `&'static mut entvars_t`, so keep that
   borrow short, and never hold it across an engine call that can re-enter the game for the same
   edict (touch, remove, damage). See the module docs in `goldsrc/src/edict.rs` and the take-out
   dispatch rules in `server/src/world.rs`.
6. **Hooks:** console commands are safe `extern "C" fn()`. User messages take `fn(&[u8])` and
   client events take `fn(&event_args_s)`; `goldsrc` provides the trampolines.

These wrappers make the unsafe code small and auditable. They can't make engine memory
provably sound, because the engine shares and mutates it, so respect the borrow rules above.

## Build and test

Requirements: Rust stable, `clang` (bindgen), MinGW-w64 for the Windows targets, Wine for the
Windows engine roots, and `../xash3d-fwgs` (or set `FWGS_DIR`).

```sh
cargo test                                # unit tests (all crates)
cargo xtask build [--release]             # all four targets; must build with no warnings
cargo xtask lint                          # clippy --workspace --all-targets -D warnings, all four targets
                                          # (~40 s cold), plus the engine-macro check
cargo xtask install <root>|all            # assemble <root>/ts and install the libraries
cargo xtask all                           # test + play (meck included), about 3.5 min
cargo xtask test [--map M]                # dedicated server + bots on every engine root, in parallel (~25 s)
cargo xtask play [fwgs fwgs-win]          # listen-server gameplay on ts_dojo: join/buy/spawn, power-up, grenade,
                                          # kill, kick, death/respawn, bot fights; then meck; both roots in parallel (~3 min)
cargo xtask meck [fwgs fwgs-win]          # grenades on MecklenburgV_Rain_Final: a grenade kill and broken func_breakables
cargo xtask client <server-root>          # FWGS client joins a dedicated server; screenshot in target/
cargo xtask dist --release                # dist/<platform>/ts/: libraries + game files
```

Engine roots (`cargo xtask roots`) default to `~/games/<dir>`: `fwgs` (native amd64 FWGS),
`fwgs-win` (FWGS 0.21 win32 under Wine), `rehlds-linux`, `rehlds-win`, `hlds25-linux` and
`hlds25-win`. Setup steps are in `docs/validation.md` §1.

Before calling a change done:
- `cargo fmt` is applied (`rustfmt.toml`: 140 columns, `use_small_heuristics = "Max"`), so
  `cargo fmt --check` is clean.
- `cargo test` passes.
- `cargo xtask build` has no warnings.
- `cargo xtask lint` passes: the workspace is clippy-clean on all four targets. Fix a lint
  rather than silence it; where it fights a deliberate TS- or SDK-faithful pattern, use a
  narrowly scoped `#[allow(clippy::...)]` with a comment saying why, never a crate-wide one.
- Run `cargo xtask all` (or `test` alone for server-only changes that can't affect gameplay).
- When the change touches exports or the `*-dll` crates, check that the exported symbol lists are
  unchanged (`nm -D --defined-only` / `objdump -p`).

Scripted in-engine tests use dev aids that need `sv_cheats 1`:
- `ts_dev_scriptinput 1` lets scripted `+attack` through the client's anti-script gate.
- `ts_dev_touch <file>` writes the end-of-script marker.
- `ts_tp` teleports, and `ts_face` aims a player's view.
- `ts_bot_stop 1` freezes bots.
- `ts_status`, `ts_hurt` and `ts_give` are also available.

## Conventions

- **Wire compatibility is a hard constraint.** Don't rename, reorder or resize user messages.
  Don't change event names or parameter meanings, `delta.lst` field usage, or TS client commands.
  `shared::protocol` holds the table, and tests pin the encoders to the decoders.
- **Cite the decomp.** Doc comments name the TS function and address they port, e.g.
  ``/// `CBasePlayer::StopObserver` (0x99a14)``. Keep doing that.
- **Formatting** is rustfmt's, with the repo `rustfmt.toml`. Don't hand-format against it.
- **Comments say why**, not what. Doc comments go on public items.
- **Docs move with the code.** Update `docs/design.md` (architecture), the matching
  `docs/implementation/*.md` (behaviour, deviations, gaps) and `docs/validation.md` (tests,
  pitfalls) in the same change.
- **Port-only additions are marked.** Cvars, commands and dev aids that TS doesn't have
  (`ts_bot_quota`, `ts_tp`, …) are commented as port additions.

## Pitfalls (learned the hard way)

- **GoldSrc runs command-line `+cvar` before `GameInit` registers the game's cvars.** Set game
  cvars from a cfg via `+servercfgfile` (xtask does).
- **The game queues `exec <servercfgfile>` (`lservercfgfile` on a listen server) on every map**,
  as TS's `CHalfLifeMultiplay` constructor does. The command waits behind a running `wait` script,
  and TS's shipped `listenserver.cfg` adds four bots and sets `sv_cheats 0`, so xtask's listen-server
  scripts set `lservercfgfile ""`.
- **Stock HLDS strips `_i386` and loads `dlls/ts.so`**, so xtask installs both names.
- **Windows DLLs need MSVC-style export sections.** Steam's engine finds exports through the
  section named `.rdata`, so xtask renames MinGW's `.edata`. Use `cargo xtask install`/`dist`, not
  raw `cargo build` DLLs.
- **Don't trust `pfnGetPlayerUserId` for liveness.** After a client drops, FWGS still maps its
  edict to the freed slot (old user id, NULL `cl->edict`). `ClientDisconnect` clears
  `FL_CLIENT`/`FL_FAKECLIENT`, which `players()`/`is_player()` rely on. TS keeps the flag, so ports
  of TS player tests use `rules::has_player`/`ts_flags`/`slot_players`, never the flag.
- **HL25 drops every bot when the last human leaves.**
- **Server commands the game queues run late in scripted tests.** `pfnServerCommand` text (e.g.
  the bot quota's `kick`) goes to the end of the console buffer, behind a running `wait` script.
  End the script; don't `quit` from it.
- **The FWGS engine log is buffered under Wine**, so don't wait for a line in it while the game
  runs. Scripts end with `ts_dev_touch` and xtask polls for that file.
- **FWGS's console buffer is 32 KB.** Long `wait` chains overflow it and the whole cfg is skipped
  (`Cbuf_InsertTextToBuffer: overflow`). xtask scripts use `w10`/`w100` aliases.
- **Windows crash dialogs (`Sys_Crash`) name the nearest export**, which is meaningless. Solve
  the DLL's load base from the frame addresses and the export RVAs, then
  `addr2line -e mp.dll` on the debug build.
- **Stopping engines:** use `pkill -x xash` (or `xash3d`). `pkill -f` patterns can match your
  own shell.

## Git

- Work on `main` unless told otherwise. Commit or push only when asked.
- If git has no HTTPS credentials, push with
  `git -c credential.helper='!gh auth git-credential' push`.
- Track gaps and bugs as GitHub issues. Integration work uses the `integration` label.
