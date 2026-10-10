# Stock Half-Life map entities

The stock-entities work package covers the Half-Life SDK classes that TS 3.0 maps use (design.md §5,
priorities 1 and 2, plus some cheap extras). The code is in `crates/server/src/entities/stock/`, and the
engine helpers are in `crates/goldsrc/src/ext/entities.rs`.

Behaviour comes from the SDK (`halflife-sdk/dlls`). Where TS 3.0 changed a class, the TS version is
ported from the decompilation (`ts30-decomp/server/decomp/<Class>.c`). Every class used here was
diffed against the decomp. Most are the original SDK 2.3 code. The changes are listed in §3.

## 1. Layout and framework

| File | Contents |
|---|---|
| `mod.rs` | The `link_entities!` table (one export per classname), `worldspawn` (light styles 0–12 and 63, per-map resets), spawn points, `object_caps`, `player_use` |
| `base.rs` | `CBaseDelay` (`Delay`, `sub_use_targets`, the `DelayedUse` entity), `CBaseToggle` (`Toggle<T>`, `linear_move` / `angular_move` on `ltime`), master registry (`is_master_triggered`), `env_global` state table, keyvalue / precache helpers, `create_entity`, `precache_other`, `set_movedir`, axis helpers |
| `explode.rs` | TS `TSExplosion`, SDK `RadiusDamage`, TS `UTIL_ScreenShake` / `UTIL_ScreenFade` (`TSFade`), sparks, ricochet, TS slow-motion pitch `emit_sound_slow` |
| `breakable.rs` | `func_breakable`, `func_pushable`, material tables, `respawn_all` |
| `doors.rs` | `func_door`, `func_door_rotating`, `func_water` |
| `buttons.rs` | `func_button`, `func_rot_button`, `button_target`, `multisource`, lock sounds, `DoSpark` |
| `bmodels.rs` | `func_wall`, `func_illusionary`, `func_wall_toggle`, `func_conveyor`, `func_rotating`, `func_pendulum` |
| `trains.rs` | `func_train`, `path_corner`, `func_plat` (with its trigger) |
| `tracktrain.rs` | `func_tracktrain`, `path_track` |
| `triggers.rs` | `trigger_multiple/once/hurt/teleport/push/gravity/auto/relay/changetarget/camera`, `multi_manager`, `func_ladder`, `func_friction`, point entities (`info_target`, `info_teleport_destination`, `info_intermission`, `info_landmark`, `info_null`), `env_global`, `env_message` |
| `sound.rs` | `ambient_generic` (with the full dynpitchvol / LFO code and the 27 presets), `env_sound` |
| `lights.rs` | `light`, `light_spot`, `light_environment` |
| `effects.rs` | `env_sprite`, `env_glow`, `env_shake`, `env_fade`, `env_explosion` (+ `spark_shower`), `env_render`, `env_beam`, `env_laser`, `env_shooter`, `gibshooter` (+ `gib`), `env_spark` / `env_debris`, `env_bubbles`, `infodecal`, `cycler_sprite`, `cycler` |
| `items.rs` | `item_healthkit`, `item_battery`, `func_healthcharger`, `func_recharge`, TS `TakeHealth` / `TakeArmor` stand-ins |
| `maprules.rs` | `game_text` |

How the SDK patterns map to Rust:

* **`SetThink` / `SetTouch` / `SetUse` / `m_pfnCallWhenMoveDone`.** These become fields of type
  `Option<fn(&mut Self, &mut Cx, ...)>` on the class: `Toggle<T>::{think, touch, move_done}`, or
  `ThinkFn<Self>` in classes without a `Toggle`. Classes with only a few think states use a small
  enum. `Entity::think` / `touch` call through these fields (`run_think`, `run_touch`).
* **Push movers.** `CBaseToggle::LinearMove` and `AngularMove` set `nextthink = ltime + travel`
  exactly as the SDK does, and `MOVETYPE_PUSH` entities think on `ltime`. `next_think(cx, d)` picks
  `ltime` or global time from the movetype.
* **Calls between entities.** `world::dispatch` plus a downcast handles method calls (a door's
  siblings in `Blocked`, a plat trigger calling its platform, `respawn_all`). `world::with` reads
  data that the SDK got through a pointer (`path_corner` wait, `path_track` links,
  `multi_manager::HasTarget`).
* **Masters.** A `multisource` writes its state into a registry keyed by edict
  (`base::set_master_state`). That way a door can check its master even while the multisource is
  itself mid-dispatch: during take-out dispatch the multisource object can't be reached, but the
  registry can.
* **Engine-kept strings.** Dynamic model and sound names go through `goldsrc::server::intern`
  (`precache_model_s`, `precache_sound_s`, light patterns).

`goldsrc::ext::entities` adds the following:

* `vec` maths and `EHandle`.
* Entity searches: `entities_in_box`, `find_in_sphere`, `find_by_target_name`.
* Engine calls the base wrapper didn't have: `create_entity`, `set_view`, `static_decal`,
  `decal_index`, `model_frames`, `set_cvar_string`, `make_vectors`, `walk_move`, `trace_model`.
* Sound constants (`ATTN_*`, `snd::*`) and all `TE_*` codes, with `SVC_TEMPENTITY`.
* `MsgWriterExt::vec3` and temp-entity writers: `te_sparks`, `te_break_model`, `te_explosion`,
  `te_smoke`, `te_bsp_decal`, `te_bubbles`, and `te_text_message` (`UTIL_HudMessage`, 511-character
  limit as in TS).

## 2. Classes

"SDK" means the class is ported from the SDK file named, with no TS change found in the decomp.
"TS" means the decomp differs and the TS behaviour is what's ported.

| Classname | Source | Notes |
|---|---|---|
| `worldspawn` | SDK world.cpp | Sets light styles 0–12 and 63 (only 0 was set before) and `sv_skyname`. Other keys go to `rules::world_key_value`. Resets the masters, path links and env_sound rooms on each map. |
| `func_wall`, `func_illusionary` | SDK bmodels.cpp | `func_wall` now sets `FL_WORLDBRUSH` and toggles its texture frame on Use. `func_illusionary` is `MOVETYPE_NONE`, and `skin` sets its contents. |
| `func_wall_toggle`, `func_conveyor` | SDK | The conveyor speed is sent in `rendercolor`. |
| `func_rotating` | SDK | Fan sounds, spin-up/down, hurt touch. Uses `fabs` where gcc 2.95 used the integer `abs`. |
| `func_pendulum` | SDK | The INSTANT flag thinks on `ltime` (the SDK compares global time against `ltime`). |
| `func_breakable` | **TS** `CBreakable` | See §3.1. |
| `func_pushable` | SDK 2.3 + **TS** | 2.3 `Move` plus TS's 0.3 factor when pulling with +use. A breakable pushable uses the TS breakable code. |
| `func_door`, `func_door_rotating`, `func_water` | **TS** `CBaseDoor` / `CRotDoor` | See §3.2. Lock sentences aren't played. |
| `func_button`, `func_rot_button` | SDK buttons.cpp | Spark sounds use TS slow-motion pitch. Shootable buttons (health > 0) work through `take_damage`. |
| `button_target`, `multisource` | SDK | |
| `func_train`, `path_corner` | SDK plats.cpp | `Activate` is emulated (see §6). |
| `func_plat` | **TS** `CFuncPlat` | Adds the TS keys `targettop` and `targetbot`, which fire when the plat reaches the top or bottom. Spawns its own trigger volume. |
| `func_tracktrain`, `path_track` | SDK | Includes branches, alternate paths, banking, dead ends, and the `events/train.sc` pitch updates. Player driving is not ported. |
| `trigger_multiple`, `trigger_once`, `trigger_teleport`, `trigger_push`, `trigger_gravity` | SDK triggers.cpp | `UTIL_ShowMessage` → `HudText`. |
| `trigger_hurt` | SDK 2.3 | Negative damage heals with TS `TakeHealth`, with no dead-player check (2.3). The per-player bitmask in `impulse` is kept. `RadiationThink` is not ported (it only drives the HEV geiger counter). |
| `trigger_auto`, `trigger_relay`, `trigger_changetarget` | SDK | |
| `trigger_camera` | SDK | `SET_VIEW`, `FL_FROZEN` for TAKECONTROL, `path_corner` moves. |
| `multi_manager` | SDK | Includes thread clones. Fixes the keyvalue order (see §6). |
| `func_ladder`, `func_friction` | SDK | |
| `info_target`, `info_teleport_destination`, `info_intermission`, `info_landmark`, `info_null` | SDK | `info_null` removes itself. |
| `env_global`, `env_message` | SDK | Global states persist across maps, like `gGlobalState`. |
| `ambient_generic` | SDK sound.cpp | The TS preset table is byte-identical. `!SENTENCE` sounds are not played. |
| `env_sound` | SDK | Per-player room state lives in a side table in `sound.rs`. |
| `light`, `light_spot`, `light_environment` | SDK lights.cpp | Lights without a targetname are removed. Switchable styles are 32 and up. `light_environment` sets `sv_skycolor_*` and `sv_skyvec_*`. |
| `env_sprite`, `env_glow`, `env_render`, `env_spark`, `env_debris`, `env_bubbles`, `infodecal`, `cycler_sprite` | SDK effects.cpp / world.cpp / h_cycler.cpp | |
| `env_shake` | SDK + **TS** shake | Duration and frequency are divided by each player's time scale (`fuser1`). |
| `env_fade` | **TS** `CFade` | Sent as `TSFade` with durations divided by the time scale. TS spawns it as a brush. New spawnflag 8 fades only the clients inside the brush. |
| `env_explosion`, `spark_shower` | SDK explode.cpp | TS doesn't route these through `TSExplosion`. Uses SDK `RadiusDamage`, scorch decals, fireball and smoke temp entities. |
| `env_beam` | SDK `CLightning` | Server-side beams work as `FL_CUSTOMENTITY` beam entities: `BeamUpdateVars` runs on the first think. Strike beams use temp entities. Damage and decals are included. |
| `env_laser` | SDK `CLaser` | Includes the end sprite and damage. |
| `env_shooter`, `gibshooter`, `gib` | SDK | Gib bounce sounds use TS slow-motion pitch. Blood decals from human gibs are not ported. |
| `cycler` | SDK, simplified | The client animates from `animtime`/`framerate`. Server studio bookkeeping isn't ported. |
| `item_healthkit`, `item_battery` | **TS** pickup rules | See §3.3. |
| `func_healthcharger`, `func_recharge` | **TS** `CWallHealth` / `CRecharge` | One-shot: the first use refills fully, then the charger recharges after 60 s (health) or 30 s (armour). |
| `game_text` | SDK maprules.cpp | `TE_TEXTMESSAGE` per client, master check. |

## 3. TS deviations from the SDK (ported)

### 3.1 `func_breakable` (decomp `CBreakable.c`)

* **New keys.**
  * `respawntime` (int): 0, the default, means respawn 30 s after breaking; a negative value means
    never.
  * `respawntarget`: fired on respawn when spawnflag 8 is set. Without it, `target` is fired. The
    delay is respected.
  * `gibmodel` and `spawnobject`. The `spawnobject` table matches the SDK, checked against the binary.
  * `explodemagnitude` (`pev->impulse`).
  * `material` ≥ 10 becomes wood.
* **Respawn (`BreakableRespawn`).** When the timer runs out, the breakable is rebuilt if no client
  (`FL_CLIENT|FL_FAKECLIENT`) is inside its box. Otherwise it retries every 5 s. Rebuilding restores
  health, clears `deadflag`/`effects` and re-runs Spawn. `breakable::respawn_all(force)` is the
  `CTSGameRules::RespawnEntities` entry point.
* **Spawn** saves the max health and sets `fuser1 = 1`.
* **TraceAttack.** It records the hit position, direction, damage and bits. Shotgun damage
  (`0x2000000`) counts ×8 for debris. 50 % of hits make sparks (computer) or a ricochet
  (unbreakable glass).
* **TakeDamage.**
  * A crowbar instant-break applies to club damage from a client.
  * Club damage ×2, poison ×0.1.
  * On death it calls `Die` directly (TS skips `Killed`).
* **Die.**
  * Pitch is 95–124 (100 for rolls 3–7). Volume is `rand(0.85,1) + |health|/100`. The attacker's
    `fuser1` is copied for the slow-motion pitch.
  * Glass sends **`TSFrags`** (`TSCreateDebris`, MSG_PVS): centre, size, `byte 0`, hit point,
    hit direction, `long` damage bits, `coord` magnitude.
  * Other materials send `TE_BREAKMODEL`, with velocity = hit direction × damage × material
    multiplier (wood/metal 2.5, flesh 4, concrete/rocks 2), random 10, count 0, life 25.
  * Anything standing on top loses `FL_ONGROUND`.
  * Explodables call `TSExplosion(center, self, attacker, mag, BLAST|NEVERGIB, mag, mag*2.5)`.
  * The breakable is hidden (`EF_NODRAW`, `DEAD_DEAD`) instead of removed while it waits to
    respawn.
* **Use** debris flies away from the activator. Use does nothing once the breakable is dead.
* **Implementation difference.** TS keeps the timer on a hidden `info_breakable` helper entity.
  Here the breakable thinks on its own `ltime`, `MOVETYPE_PUSH` style, with the same timings.

### 3.2 Doors (decomp `CBaseDoor.c`, `CRotDoor.c`)

* **Breakable doors.**
  * Keys `maxdamage` (> 0 makes the door take damage), `doormaterial`, and `respawntime`
    (negative is clamped to 0; 0 means 30 s).
  * Only damage with blast or the TS shotgun / kung-fu bits (`0x6000040`) counts. Blast counts ×2.
    Damage accumulates until it reaches `maxdamage`.
  * When a closed door reaches the limit, the first such hit may only loosen it: a roll over 50
    (over 30 for shotgun hits) loosens it. A loosened door no longer opens, and the next hit breaks
    it.
  * Breaking plays the material sound and sends `TSFrags` (glass, all zero) or `TE_BREAKMODEL`
    (velocity = direction × damage × 2, ×8 for shotgun). The door is then hidden.
  * The door respawns at its spawn origin and angles once no client is in the way.
  * `func_water` is excluded.
* **Touch.** `DoorTouch` opens at `max(speed, toucher speed)`.
* **Direction.** Rotating doors choose their direction from the activator's velocity, not from its
  view angles.
* **Activation.** `DoorActivate` refuses while the door is loosened or hidden.
* **Moves.** The SDK 2.3 `LinearMoveDone` is used, without HL25's re-move on small errors.

### 3.3 Items and chargers

* **Healthkit.** It gives a fixed 50 health through TS `CBasePlayer::TakeHealth`, which refuses only
  when health > max and has no cap. A kit at 100 HP gives 150. The decay back to 100 at 1/s is in
  the player's PreThink (rules package).
* **Battery.** It calls `TakeArmor(50)`. The first kevlar takes 10 free inventory slots: without
  them there's no armour, but the battery is still used up. Armour is capped at 100 and speed is
  recomputed from the free slots.
* **Chargers.** The health charger refills to max in one use. The HEV charger calls
  `TakeArmor(100)` in one use. They recharge after 60 s and 30 s.
* **Items** respawn after 30 s unless `SF_NORESPAWN` (1 << 30) is set, with the `suitchargeok1`
  materialise sound.

### 3.4 Effects

* **`UTIL_ScreenShake`.** Only players on the ground shake. Duration and frequency are divided by
  `max(fuser1, 0.01)`.
* **`UTIL_ScreenFade`.** Always sent as `TSFade` (`short dur, short hold, short flags, byte r,g,b,a`)
  with durations divided by the time scale. The stock `ScreenFade` message is never sent.
* **`EMIT_SOUND_DYN`.** It has an extra factor `s`: pitch becomes `(s*fuser1 + 1 - s) * pitch`.
  This is used for break sounds (s = 0.9), damage, spark and gib sounds (0.5).
* **`TSExplosion`** (0x7d01c) is in `explode::ts_explosion`:
  * A screen shake with amplitude dmg, frequency 80, duration 1, radius 2·rMax.
  * `TSFrags` to everyone: origin, `coord (int)dmg`, `0`, `0`, `byte 1`.
  * A white `TSFade` for players caught in it.
  * Full damage inside rFull, linear falloff to rMax.
  * Wall penetration: damage minus the wall thickness.

## 4. Coverage (97 shipped maps)

These numbers come from running `census.py` (§8) over `ts/maps/*.bsp` with the list of linked classnames.
`tsrp_nukecity_v2.bsp` is not a valid BSP (version 1329865020), so 97 of the 98 files count.

| | Classes | Entities |
|---|---|---|
| All classnames in the maps | 187 | 28,504 |
| Handled by this package | 67 | 25,320 (88.8 %) |
| TS / weapon / ammo / team-spawn classes (other packages) | 70 | 2,948 |
| Still unhandled, stock or foreign | 50 | 236 |

Every priority-1 and priority-2 stock class from design.md §5 is handled. The extras in this package
are:

* `func_plat` (3 maps)
* `func_tracktrain` and `path_track` (4 maps, 161 entities)
* `func_pendulum`
* `env_sound`, `env_global`, `env_message`, `env_laser`
* `func_healthcharger`, `func_recharge`
* `trigger_gravity`, `func_friction`, `gibshooter`, `env_spark`, `cycler`

**Still unhandled.** TS links these but this port doesn't yet:

* `func_tank`, `func_tankmortar`, `func_tanklaser` and `func_tankcontrols` (5 entities in total;
  they need player control).
* `func_traincontrols` (1).
* `env_blood` (8, one map).
* `game_score` (1).
* `monster_hevsuit_dead` (1).
* `xen_*` decoration (11).

TS doesn't link the rest either, so on the original server they also log "No spawn function". That
covers:

* TFC entities: `info_tfgoal`, `item_tfgoal`, `info_tfdetect`.
* CS entities: `hostage_entity`, `func_bomb_target`, `armoury_entity`, `func_buyzone`.
* HL monsters and `info_node`.
* `trigger_roundbased` and `trigger_deathmatch`.
* Odd item names.

## 5. Tests

* **`cargo test`.** All pass, including the new unit tests:
  * vector maths in `goldsrc` ext
  * atof/atoi keyvalue parsing, `ShouldToggle`, axis helpers
  * `UTIL_StripToken`, `UTIL_Approach`
  * the sky colour formula, game_text colours
  * the angle helpers for track trains
* **Builds.** `cargo build -p ts-server` passes for the host target, `i686-unknown-linux-gnu` and
  `i686-pc-windows-gnu`. Windows prints the existing stdcall-fixup linker note.
* **Dedicated server, all maps.**
  * `cargo xtask server fwgs --map ts_lobby --bots 0 --port 27220`: PASS.
  * Every map was loaded for 6 s on the FWGS dedicated server (private root, port 27221).
  * 97 of 97 valid maps load. There were no panics, no Host_Error and no Rust-side precache
    errors.
  * Remaining log noise is map content: missing sprites on 3 maps, model-less brush entities
    exactly as the SDK would see them, two empty `ambient_generic`s, and one healthkit outside the
    level.
  * `late precache of player/pl_wade2.wav`: the engine plays it when an entity enters water. The
    SDK precaches it in `ClientPrecache`, which belongs to the player package.
* **Listen server** (FWGS client, `sv_enttools_enable 1`, scripted `ent_fire` / `ent_list` /
  `ent_info`). These were verified in-engine:
  * **ts_lobby:**
    * `func_door` pairs slide open.
    * `func_door_rotating` swings to 85°.
    * `func_plat` (toggle plat) moves down 613 of 704 units in 4 s.
    * 39 `func_breakable`s break, sending `TE_BREAKMODEL` and `TSFrags`.
    * A broken breakable is solid again after 30 s.
    * The `multi_manager` chain drives a switchable light style (33 m/a alternating).
  * **ts_bikini:** `trigger_camera` takes the view; env_explosion, conveyor reverse, rotating door
    and `trigger_relay` chains all fire with no errors.
  * **crossfire:**
    * buttons, env_shake, env_fade, multi_manager, both chargers and lights all fire.
    * The healthkit takes the player from 100 to 150 HP (TS rule).
    * `trigger_hurt` with 10000 damage kills the player.
  * **ts_antrodome:** the four fish `func_tracktrain`s move along their `path_track` loops.
* **Not verified in-engine:**
  * real player touch and +use (there are no bots on this branch, and the `player_use` helper isn't
    wired up yet)
  * pushables
  * breakable doors (they need TS blast / shotgun damage from the weapons package)
  * `TSExplosion` damage
  * `env_beam`, `env_laser` and `env_shooter` visuals
  * `env_sound`
  * respawn with a client inside the box
  * `game_text` on screen
  * The client printed `No pfn TSFrags`, `TSFade` and `ItemPickup`. The HUD package hasn't hooked
    those messages yet; the server side is sending them as intended.

## 6. Integration notes (changes needed outside this package)

1. **Keyvalue order.** The SDK's `DispatchKeyValue` parses entvars fields before the class
   `KeyValue`. `server-dll`'s `key_value` callback does it the other way round.
   * `multi_manager` works around this itself: it treats any unknown key as a target name, so it
     calls `entvars_key_value` first.
   * Other classes don't care.
   * Changing that callback to entvars-first would match the SDK (decomp doc gamerules-entities.md §8
     says the same).
2. **`Activate`.** `ServerActivate` should call an `Entity::activate` hook on every entity, as
   `CBaseEntity::Activate` does in the SDK. Until then, three classes emulate it:
   * `func_train` runs `Activate` from a think 0.1 s after spawning.
   * `func_tracktrain` links all `path_track`s on its first think.
   * `env_beam` runs `BeamUpdateVars` 0.05 s after spawning.
3. **+use.** The `Entity` trait has no `ObjectCaps`.
   * `stock::object_caps(e)` returns the stock classes' use caps.
   * `stock::player_use(player, buttons, old_buttons)` implements `CBasePlayer::PlayerUse` for
     them: it finds the closest usable entity within 64 units in the view cone, impulse or
     continuous.
   * The player package should call it from PreThink, and fold in TS's own +use targets: power-ups
     and ground weapons, `CanUsedTroughWalls`.
4. **Round restart.** `CTSGameRules::RespawnEntities` should call
   `stock::breakable::respawn_all(false)`. This covers breakables, breakable pushables and
   breakable doors.
5. **Player state the SDK keeps on `CBasePlayer`.**
   * The health decay above `max_health` (PreThink, `+0x73c`) belongs to the rules package.
   * Kevlar: `items::player_take_armor` is a stand-in for TS `TakeArmor`. It uses the networked
     `HAS_KEVLAR` stunt bit and `Player::slots_used`. The weapons/rules package should own the real
     function, plus `DropArmor`.
   * The HEV suit bit (`pev->weapons & 1<<31`) isn't set by `Player::spawn`. Battery and charger
     therefore skip TS's suit check.
6. **`trigger_camera` on death or respawn.** TS `CBasePlayer::Killed` and `Spawn` call
   `SET_VIEW(self, self)` and clear `FL_FROZEN`. The player package must do this too, or a camera
   active when the player dies keeps the view.
7. **Damage.**
   * `take_damage` treats a non-zero `DamageInfo::pos` or `dir` as the SDK `TraceAttack` path.
     Weapons should fill both for bullets.
   * Doors only break from `TraceAttack`-style damage carrying blast (`0x40`) or the TS bits
     `0x2000000` (shotgun) / `0x4000000` (kung fu).
   * The SDK `DamageDecal` for glass (`{bproof`-style decals 0x19–0x1b, 0x23 for unbreakable glass)
     is for the weapons package to apply when it hits a `func_breakable`.
8. **Explosions.** `stock::explode::ts_explosion` is TS's `TSExplosion`. The grenade code (weapons
   package) should call it as well and add the two pieces that belong to other packages: knocking
   `WorldGun` entities around, and the "spectacular" slow-motion point.
9. **`entities::fire_targets`** doesn't skip `FL_KILLME` entities. The SDK `FireTargets` does. This
   is a small fix in `entities/mod.rs`.
10. **`pfnResetGlobalState`.** Calls `stock::base::reset_global_state()` (done at integration).
11. **Client hooks** (HUD package): `TSFrags` (glass debris and explosions), `TSFade`, `ItemPickup`,
    and the `events/train.sc` handler.
12. **Precache** (player package): `player/pl_wade1.wav`, `pl_wade2.wav` and the other SDK
    `ClientPrecache` sounds. The engine plays them for any entity entering water.
13. **`env_sound`.** Call `stock::sound::reset_player_room(index)` when a client connects, so a new
    client in a reused slot starts with the right DSP state.

## 7. What's missing or simplified

* Sentences (`!NAME` ambient sounds, door/button lock sentences, the battery's HEV sentence). There
  is no sentences.txt loader.
* Train driving (`func_traincontrols`, the player's train HUD) and `func_tank*`.
* `cycler` server-side studio animation: only `animtime`/`framerate`/`sequence` are driven.
* `trigger_hurt` `RadiationThink` (HEV geiger), human-gib blood decals, `env_sprite` attachments
  (`aiment`).
* `env_blood`, `game_score`, `monster_hevsuit_dead` and `xen_*` (see §4).

## 8. Census script

```python
# python3 -I census.py <maps dir> [implemented-classnames.txt]
import struct, sys, glob, re, collections
def ents(path):
    d = open(path, 'rb').read()
    ver, off, ln = struct.unpack_from('<iii', d, 0)          # BSP v30: lump 0 = entities
    txt = d[off:off + ln].decode('latin-1')
    return [dict(re.findall(r'"([^"]*)"\s*"([^"]*)"', b)) for b in re.findall(r'\{([^{}]*)\}', txt)]
impl = set(open(sys.argv[2]).read().split()) if len(sys.argv) > 2 else set()
maps, tot = collections.Counter(), collections.Counter()
for m in sorted(glob.glob(sys.argv[1] + '/*.bsp')):
    seen = set()
    for e in ents(m):
        c = e.get('classname', '?'); tot[c] += 1; seen.add(c)
    maps.update(seen)
for c, n in maps.most_common():
    print(f"{'Y' if c in impl else '-'} {n:3d} maps {tot[c]:5d} ents  {c}")
```

To get the implemented list, use
`grep -hoP '^\s+\K[a-z_0-9A-Z]+(?=\s+=> )' crates/server/src/entities/stock/mod.rs`.
