# Integration

The six work packages (movement, weapons, stock entities, rules, client HUD, client rendering)
were built in parallel against the skeleton APIs in `docs/design.md`. This page records how they
were joined, what was decided along the way, and what is still open.

## 1. Wiring between packages

| Area | What was wired | Where |
|---|---|---|
| Protocol | Server HUD encoders (`protocol::hud_wire`, rules) and client decoders (`protocol::hud`, HUD) are separate modules. `protocol::wire_agreement` pins them to the same layout for the 13 messages both use. | `crates/shared/src/protocol/` |
| Effect messages | The engine keeps one hook per message. The HUD hooks all 64 and forwards `TSPSys`, `PTakeDam`, `TSFrags`, `TSMBlur`, `TSBTime` and `TSFade` raw to the render modules' `on_*` parsers. The render modules no longer register their own hooks. | `client/src/hud/mod.rs` |
| Client shared state | One per-player `active_items`/`laser_color` table (HUD writes, lasers read; indexed by entity number). The local predicted item bits are `local_items`. The spectator flags are filled by the HUD sync. | `client/src/state.rs` |
| Fall damage | Applied once, in the stunt PostThink, together with the landing logic. The rules copy was removed. | `server/src/stunt/moves.rs` |
| Death | `rules::killed` sets `DEAD_LOCK`, fills the stunt animation inputs (last damage, hitgroup, attack direction) and plays `CTSAnimation` 7. | `server/src/rules/lifecycle.rs` |
| Bullet dodge | Damage reads `p.stunt.dodge_until` (stunt sets it). | `server/src/player/damage.rs` |
| Bullet dance | Damage arms it (`stunt::anim::enable_`/`disable_bullet_dance`) on every hit to a living player and calls `stunt::anim::bullet_dance` when a dead one is shot. | `server/src/player/damage.rs` |
| Death animation | `death_think` keeps the player dying until `p.stunt.anim.seq_finished`, advancing the sequence with `stunt::anim::studio_frame_advance`. | `server/src/rules/lifecycle.rs` |
| Weapons → animation | Deploy and holster set the anim extension. Fire, reload, draw, grenade and melee call `stunt::animate`. | `server/src/weapons/` |
| Melee | One delayed-hit window (the gun's). Stunt hands open-window hits to weapons, which handles blocks, disarm and friendly fire. The super-jump roll landing sets the 0.75 s attack delay. | `server/src/weapons/`, `stunt/` |
| Kill scoring | A kill made during the killer's own weapon frame (the killer's object is taken out of the slab) used to score nothing. `world::defer` now queues the work until the object is back. | `server/src/world.rs`, `rules/scoring.rs` |
| Client prediction | `HUD_PostRunCmd` runs the shared `Gun` through a client `GunHost`, unpacked from `clientdata`/`weapon_data`. It plays the shooter's own fire events and calls `view::kick` once per predicted shot. Melee predicts a miss; the server decides hits. | `client/src/prediction/gun.rs` |
| HUD after spawn | `init_hud` clears the server's change-detection caches when it sends `ResetHUD`, so the values the client just zeroed are sent again. Clip, reserve and fire mode come from prediction when it matches the current weapon, because TS only sends `WeaponInfo` on weapon or reserve changes. | `rules/lifecycle.rs`, `client/src/hud/weapons.rs` |
| Third person | `view::frame` consumes the HUD's toggle, leave (observer) and restore requests. | `client/src/view.rs` |
| `+use` | One `PlayerUse` (`player::player_use`) picks a single object of any package (stock entities, ground weapons and ammo, dropped guns, knives, grenades), run from the weapons' PostThink inside the `ItemPostFrame` gate as in TS. The rules keep only the power-up double tap. | `server/src/player/player_use.rs`, `weapons/mod.rs` |
| Spawn | Clears `FL_FROZEN` and resets the view (`trigger_camera`). | `server/src/player/mod.rs` |
| Cvars and commands | One owner each. Rules owns the shared weapon cvars (`realbullet`, `ammocount`, `weaponstay`, `damagemult`, `customweapons`) and `weaponrestriction`; weapons reads them by name and owns `loadweapons`/`saveweapons`. The two restriction masks are one. Cvars the engine already has (`sv_alltalk`) aren't re-registered. | `rules/cvars.rs`, `weapons/` |
| Bots | A random loadout every spawn, reloading, weapon switching, aim gating, and chasing the nearest enemy. Only bots with a live user id are driven, because HL25 drops all bots when the last human leaves and `pfnRunPlayerMove` on a dropped edict crashes the engine. | `server/src/bots/` |
| Misc | `pfnResetGlobalState` resets `env_global`. `fire_targets` skips entities removed this frame. Water sounds are precached. `GetGameDescription` returns the per-mode TS string. `_special` is accepted silently. | `server-dll/src/lib.rs`, `entities/mod.rs`, `commands.rs` |

## 2. Decisions

* **Two shared-random implementations.** The TS client and server generators differ: the client
  keeps the random range as a float, while the server truncates it to an integer. That gives
  different pellet angles for the same seed. Both are kept (`shared::weapons::random` for the
  server, `client/src/events/random.rs` for the client's event playback), with a test that shows
  where they agree and where they differ. This matches TS, where the original client and server
  disagree the same way.
* **Development aid.** `ts_dev_scriptinput 1` (only with `sv_cheats 1`) lets scripted `+attack`
  etc. through the client's anti-script gate, for automated tests.
* **Dead code.** Both crates keep a crate-level `#![allow(dead_code)]` (see §4).

## 3. Test evidence (integration branch)

* `cargo test`: all pass. Builds for host, `i686-unknown-linux-gnu` and `i686-pc-windows-gnu`
  have no warnings. The Windows linker's stdcall note is handled by `--enable-stdcall-fixup`, and
  the export stays undecorated.
* `cargo xtask server fwgs --map ts_awaken --bots 4 --seconds 90` passes, and the bots fight.
  The A2S_PLAYER scores after 90 s were: Bot Neo 3, Bot Trinity 0, Bot Morpheus 1, Bot Niobe 0.
  Kill counts vary between runs (0–7).
* Listen server: predicted AK and pistol firing show the shooter's own muzzle flash and fire
  event. The server fires every predicted shot. After the `ResetHUD` fix, the HUD shows health,
  free slots, the weapon grid, the weapon info and the crosshair.

## 4. Unused API (why `dead_code` is allowed)

These items have no caller yet. They're kept because they're part of a TS feature that is
switched off in 3.0, or an SDK constant kept for completeness:

| Group | Items |
|---|---|
| Round and objective modes (dead in TS 3.0) | `rules::state::Mode::{RoundTdm, Objective}`, `send_award`, `CASH_FOR_WIN`/`CASH_FOR_LOSE`, `PlayerRules::cash`, `stock::breakable::respawn_all`, `weapons::assets::{set_round_mode_items, set_grenade_buys}` |
| Weapons API for later features | `weapons::{drop_weapon, set_restriction}`, `glows_for`/`knife_glows_for`/`glow_for_host` (used by `AddToFullPack`), `Gun::{current, anim_extension}`, `shared_random_long` |
| Stunt helpers | `in_airborne_stunt` (damage uses the `STUNTING` mask), `block_player` |
| Entity helpers | `entities::use_targets`, `stock::base::{precache_model, precache_sound}`, `is_locked_by_master`, `clear_master`, `reset_player_room`, the `SF_*`/`FCAP_*`/`FFADE_*` constants |
| Messages | `messages::{pas, one_unreliable}` |
| SDK constants | unused `DMG_*` bits and hitgroups in `damage.rs`, `NearestPath` |
| Client | unused HUD cvar fields (`drawhistory_time`, `classautokill`, …), `hud::text::lookup`, `DIGITS_4`, `weapon_menu_consumed_attack`, `is_fwgs`, `effects::bytes`, `state::TsEffect::MotionBlur` (the HUD forwards `TSMBlur` raw) |

## 5. Known gaps after integration

* Recoil was checked against the decomp: the client `VKickback` (`0x10008230`), `V_ApplyKick`
  (`0x10057ed0`) and `ViewShake` (`0x10057fa0`/`0x10058000`) match the port's constants, and
  each shot permanently raises the view by `view_kick·0.85` (yaw by weapon class; ×0.775
  silenced, ×2/3 with double fire rate). Nothing pulls it back (`TSKickAlign` is unused), so the
  steep climb under sustained fire is TS behaviour. A side-by-side check of the feel is still
  open.
* The TS studio renderer (issue #2) and motion-blur ghosts (issue #3) are fully ported and were
  audited slot by slot against the asm (`client-rendering.md`); the view-model slow-motion rate
  now lives in the renderer as in TS. They are unit-tested but not yet checked in an engine
  (torso/legs split while running and aiming, corpses, ghosts during slow motion).
* Not tested with real keyboard and mouse. The only Steam GoldSrc client run so far is the
  scripted HL25 Linux listen server (`docs/validation.md` §5).
