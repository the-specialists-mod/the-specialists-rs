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
| `mod.rs` | The `link_entities!` table (one export per classname), `worldspawn` (light styles 0–12 and 63, per-map resets), spawn points, `object_caps` (the +use search itself is `player::player_use`) |
| `base.rs` | `CBaseDelay` (`Delay`, `sub_use_targets`, the `DelayedUse` entity), `CBaseToggle` (`Toggle<T>`, `linear_move` / `angular_move` on `ltime`), master registry (`is_master_triggered`), `env_global` state table, keyvalue / precache helpers, `create_entity`, `precache_other`, `set_movedir`, axis helpers |
| `explode.rs` | TS `TSExplosion`, SDK `RadiusDamage`, TS `UTIL_ScreenShake` / `UTIL_ScreenFade` (`TSFade`), sparks, ricochet, TS slow-motion pitch `emit_sound_slow` |
| `breakable.rs` | `func_breakable`, `func_pushable`, material tables, `respawn_all` |
| `doors.rs` | `func_door`, `func_door_rotating`, `func_water` |
| `buttons.rs` | `func_button`, `func_rot_button`, `button_target`, `multisource`, lock sounds and sentences (`PlayLockSounds`), `DoSpark` |
| `bmodels.rs` | `func_wall`, `func_illusionary`, `func_wall_toggle`, `func_conveyor`, `func_rotating`, `func_pendulum` |
| `trains.rs` | `func_train`, `path_corner`, `func_plat` (with its trigger) |
| `tracktrain.rs` | `func_tracktrain`, `func_traincontrols`, `path_track`, `TrainSpeed` |
| `tank.rs` | `func_tank`, `func_tanklaser`, `func_tankrocket`, `func_tankmortar`, `func_tankcontrols` |
| `drive.rs` | The player's side of train and tank driving: `m_iTrain`, `PFLAG_ONTRAIN`, `m_pTank`, `m_iHideHUD`, and the `PreThink` / `PostThink` / `PlayerUse` / `UpdateClientData` code that uses them (`Player::drive`) |
| `triggers.rs` | `trigger_multiple/once/hurt/teleport/push/gravity/auto/relay/changetarget/camera`, `multi_manager`, `func_ladder`, `func_friction`, point entities (`info_target`, `info_teleport_destination`, `info_intermission`, `info_landmark`, `info_null`), `env_global`, `env_message` |
| `sound.rs` | `ambient_generic` (with the full dynpitchvol / LFO code and the 27 presets), `env_sound` |
| `sentences.rs` | `SENTENCEG_*`: the sentences.txt loader, `!NAME` lookup, sequential and random group picks, `UTIL_EmitAmbientSound`, the (multiplayer no-op) `SetSuitUpdate` |
| `lights.rs` | `light`, `light_spot`, `light_environment` |
| `effects.rs` | `env_sprite` (+ `CSprite::AnimateAndDie`, `SetAttachment`, `SetTransparency`), `env_glow`, `env_shake`, `env_fade`, `env_explosion` (+ `spark_shower`, `ExplosionCreate`), `env_render`, `env_beam`, `env_laser`, `env_shooter`, `gibshooter` (+ `gib`, `SpawnHeadGib`, `SpawnRandomGibs`), `env_spark` / `env_debris`, `env_bubbles`, `infodecal`, `cycler_sprite`, `cycler` |
| `animating.rs` | `CBaseAnimating` / `CActAnimating` for stock model entities: `ResetSequenceInfo`, `StudioFrameAdvance`, `DispatchAnimEvents`, `SetActivity` / `LookupActivity`, `InitBoneControllers`, `SetSequenceBox` |
| `blood.rs` | `UTIL_ShouldShowBlood`, `UTIL_BloodStream`, `UTIL_BloodDrips`, `UTIL_BloodDecalTrace`, `UTIL_RandomBloodVector`, the world blood/gib precaches, `env_blood` |
| `xen.rs` | `xen_plantlight`, `xen_hair`, `xen_tree` (+ `xen_ttrigger`), `xen_spore_small/medium/large` (+ `xen_hull`) |
| `monsters.rs` | `monster_hevsuit_dead` (with the `CBaseMonster` corpse code: fall, `DeadTakeDamage`, gibbing, fade) |
| `items.rs` | `item_healthkit`, `item_battery`, `item_antidote`, `func_healthcharger`, `func_recharge`, TS `TakeHealth` / `TakeArmor` stand-ins |
| `maprules.rs` | `game_text`, `game_player_equip` (+ `GiveNamedItem`), `game_score` (+ `AddPoints` / `AddPointsToTeam`), `game_team_master`, `game_team_set`, `game_player_team` |

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
| `worldspawn` | `CWorld::KeyValue` 0xf4a18, `CWorld::Precache` 0xf449c | Keys: `skyname` (`sv_skyname`), `sounds` (CD track), `WaveHeight` (`pev.scale` = v/8, `sv_wateramp`), `MaxRange` (`pev.speed`), `chaptertitle`, `startdark`/`gametitle`/`defaultteam` (spawnflags 1/2/4), `newunit` (`sv_newunit 1`); `gamemode`, `mapteams`, `teammodels` go to `rules::world_key_value`. Spawn first forces `sv_gravity 800`, `sv_stepsize 18`, `room_type 0`, then precaches and installs the rules, then sets `sv_zmax` (`MaxRange`, or 4096 when ≤ 0), shows the chapter title through an `env_message` 0.3 s in, and sets `v_dark` and `mp_defaultteam` from the spawnflags. Light styles 0–12 and 63. Resets the masters, path links and env_sound rooms on each map. |
| `func_wall`, `func_illusionary` | SDK bmodels.cpp | `func_wall` now sets `FL_WORLDBRUSH` and toggles its texture frame on Use. `func_illusionary` is `MOVETYPE_NONE`, and `skin` sets its contents. |
| `func_wall_toggle`, `func_conveyor` | SDK | The conveyor speed is sent in `rendercolor`. |
| `func_rotating` | SDK | Fan sounds, spin-up/down, hurt touch. Uses `fabs` where gcc 2.95 used the integer `abs`. |
| `func_pendulum` | SDK | The INSTANT flag thinks on `ltime` (the SDK compares global time against `ltime`). |
| `func_breakable` | **TS** `CBreakable` | See §3.1. |
| `func_pushable` | SDK 2.3 + **TS** | 2.3 `Move` plus TS's 0.3 factor when pulling with +use. A breakable pushable uses the TS breakable code. |
| `func_door`, `func_door_rotating`, `func_water` | **TS** `CBaseDoor` / `CRotDoor` | See §3.2. Lock sounds and sentences as in §3.5. |
| `func_button`, `func_rot_button` | SDK buttons.cpp | Spark sounds use TS slow-motion pitch. Shootable buttons (health > 0) work through `take_damage`. Lock sounds and sentences as in §3.5. |
| `button_target`, `multisource` | SDK | |
| `func_train`, `path_corner` | SDK plats.cpp | `Activate` is emulated (see §6). |
| `func_plat` | **TS** `CFuncPlat` | Adds the TS keys `targettop` and `targetbot`, which fire when the plat reaches the top or bottom. Spawns its own trigger volume. |
| `func_tracktrain`, `path_track` | SDK | Includes branches, alternate paths, banking, dead ends, and the `events/train.sc` pitch updates. Players drive it (see §3.5). |
| `func_traincontrols` | SDK `CFuncTrainControls` | Hands its box to the named train as the control volume (`SetControls`), then removes itself. |
| `func_tank`, `func_tanklaser`, `func_tankrocket`, `func_tankmortar` | SDK func_tank.cpp + **TS** | Automatic (tracks a player in its PVS) and player-controlled tanks. See §3.5 for what TS changes. |
| `func_tankcontrols` | SDK | +use passes on to the first `func_tank*` named by `target` (found 0.3 s after spawn). |
| `trigger_multiple`, `trigger_once`, `trigger_teleport`, `trigger_push`, `trigger_gravity` | SDK triggers.cpp | `UTIL_ShowMessage` → `HudText`. |
| `trigger_hurt` | SDK 2.3 | Negative damage heals with TS `TakeHealth`, with no dead-player check (2.3). The per-player bitmask in `impulse` is kept. With `DMG_RADIATION` in `damagetype`, **TS** `RadiationThink` (0xe703c) lowers the geiger range of the player who can see the trigger's centre, every 0.25 s; `update_geiger_counter` (`UpdateGeigerCounter` 0xda1f0, from the player's PreThink) sends `Geiger` (range / 4) to suited players when it changes. |
| `trigger_auto`, `trigger_relay`, `trigger_changetarget` | SDK | |
| `trigger_camera` | SDK | `SET_VIEW`, `FL_FROZEN` for TAKECONTROL, `path_corner` moves. |
| `multi_manager` | SDK | Includes thread clones. |
| `func_ladder`, `func_friction` | SDK | |
| `info_target`, `info_teleport_destination`, `info_intermission`, `info_landmark`, `info_null` | SDK | `info_null` removes itself. |
| `env_global`, `env_message` | SDK | Global states persist across maps, like `gGlobalState`. |
| `ambient_generic` | SDK sound.cpp | The TS preset table is byte-identical. `!NAME` sentences aren't precached and are sent as `!<index>` (§3.5). |
| `env_sound` | SDK | Per-player room state lives in a side table in `sound.rs`. |
| `light`, `light_spot`, `light_environment` | SDK lights.cpp | Lights without a targetname are removed. Switchable styles are 32 and up. `light_environment` sets `sv_skycolor_*` and `sv_skyvec_*`. |
| `env_sprite`, `env_glow`, `env_render`, `env_spark`, `env_debris`, `env_bubbles`, `infodecal`, `cycler_sprite` | SDK effects.cpp / world.cpp / h_cycler.cpp | A sprite with an `aiment` follows that entity's attachment `body` (`MOVETYPE_FOLLOW`, skin = entity index), as `CSprite::Precache` / `SetAttachment` do; maps can't set `aiment`, so this is for code-created sprites (the `xen_plantlight` glow). `cycler_sprite` is +usable (TS impulse cap). |
| `env_shake` | SDK + **TS** shake | Duration and frequency are divided by each player's time scale (`fuser1`). |
| `env_fade` | **TS** `CFade` | Sent as `TSFade` with durations divided by the time scale. TS spawns it as a brush. New spawnflag 8 fades only the clients inside the brush. |
| `env_explosion`, `spark_shower` | SDK explode.cpp | TS doesn't route these through `TSExplosion`. Uses SDK `RadiusDamage`, scorch decals, fireball and smoke temp entities. |
| `env_beam` | SDK `CLightning` | Server-side beams work as `FL_CUSTOMENTITY` beam entities: `BeamUpdateVars` runs on the first think. Strike beams use temp entities. Damage and decals are included. |
| `env_laser` | SDK `CLaser` | Includes the end sprite and damage. |
| `env_shooter`, `gibshooter`, `gib` | SDK | Gib bounce sounds use TS slow-motion pitch. `gibshooter` gibs bleed red: up to 5 blood decals, one per airborne bounce (`BounceGibTouch` 0xb1ebc, `violence_hblood`); `env_shooter` gibs don't bleed. |
| `cycler` | SDK h_cycler.cpp | Server-side studio animation (`CCycler::Think` 0xc778c): bone controllers zeroed, `frame` advanced, one-shot sequences restart. +use toggles the animation. Damage steps to the next sequence (wrapping to 0) while animating, or one 0.1 s frame step while paused. |
| `env_blood` | SDK effects.cpp `CBlood` | `color` 1 = yellow, else red; `amount`. Drips (`TE_BLOODSPRITE`, amount doubled for multiplayer) or, with spawnflag 2, a stream (red becomes colour 70). Spawnflags 1 random direction, 4 at the activator's (or player 1's) eyes, 8 blood decal along the direction. |
| `game_score` | SDK maprules.cpp | `points` to the activating player (`AddPoints`, `ScoreInfo` to all), or with spawnflag 2 to each of their teammates; without spawnflag 1 a score isn't taken below 0. `master` gates it. |
| `monster_hevsuit_dead` | SDK `CDeadHEV` | `models/player.mdl`, body 1, sequence from `pose` (0–3: `deadback`, `deadsitting`, `deadstomach`, `deadtable`; a missing sequence logs "Dead hevsuit with bad pose" and gets `EF_BRIGHTFIELD`). Falls (`MOVETYPE_TOSS`) and takes its sequence box on landing. Crush/fall/blast/sonic/club damage wears it down (10 %) and gibs it when a hit reaches its health: head gib + 4 human gibs with blood, `common/bodysplat.wav`; with `violence_hgibs 0` it fades out instead. |
| `xen_plantlight`, `xen_hair`, `xen_tree`, `xen_ttrigger`, `xen_spore_small`, `xen_spore_medium`, `xen_spore_large`, `xen_hull` | SDK xen.cpp | Studio animation through `animating.rs`. The light plant ducks on player touch (glow sprite attached to attachment 1 goes `EF_NODRAW`, targets fired off) and rises 5 s later (targets on). The tree swings at anything touching its 48×48×128 trigger 64 units ahead or shooting it, and its `TREE_AE_ATTACK` model event deals 25 crush/slash, punch 15 and a 100 u/s shove. Large spores add five invisible `xen_hull` leg boxes. |
| `item_healthkit`, `item_battery`, `item_antidote` | **TS** pickup rules | See §3.3. |
| `func_healthcharger`, `func_recharge` | **TS** `CWallHealth` / `CRecharge` | One-shot: the first use refills fully, then the charger recharges after 60 s (health) or 30 s (armour). |
| `game_text` | SDK maprules.cpp | `TE_TEXTMESSAGE` per client, master check. |
| `game_player_equip` | SDK maprules.cpp (same in TS 0xcd494) | Unknown keys are items (`name#n` stripped, count ≥ 1, max 32), given through `GiveNamedItem` (item spawned at the player's feet with `SF_NORESPAWN`, then touched). Touch (master, not `USEONLY`) / use (no master). TS `PlayerSpawn` touches every one on spawn instead of buying the loadout (`maprules::equip_on_spawn`, called from `weapons::on_spawn`; deferred until the player's spawn dispatch returns). `weapon_tsgun` is a no-op: the TS gun is built into every player. |
| `game_team_master` | TS 0xccae8..0xccd40 | `teamindex` (-1 none), `triggerstate` (0 off, 2 toggle, else on), master. `USE_SET` takes the activator's team (`GetTeamIndex(TeamID)`) or none for a negative value; other uses fire the targets when the activator's team matches (`UTIL_TeamsMatch`: always outside teamplay; `ANYTEAM` with no team), once with flag 1. It is also a master (`IsTriggered` = `TeamMatch`), for spawn points, doors and the rest. |
| `game_team_set` | TS 0xccd64 | `USE_SET` to its targets with 0, or -1 with `CLEARTEAM` (2); once with flag 1. |
| `game_player_team` | TS 0xcd740..0xce658 | A player activator moves to the team of the targeted `game_team_master` (`ChangePlayerTeam(team, kill = flag 2, gib = flag 4)`; nothing outside teamplay); once with flag 1. |

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
  when health > max and has no cap. A kit at 100 HP gives 150. Overhealing starts the decay timer
  (`+0x73c` = time + 1), and the player's PreThink (rules package) then takes 1/s back to 100.
  Every player heal (kits, chargers, door heals, negative `trigger_hurt`, drown recovery) goes
  through `player::damage::take_health`/`take_health_edict`.
* **Battery.** It refuses players without the HEV suit bit (`weapons` bit 31, set on spawn). It
  calls `TakeArmor(50)`. The first kevlar takes 10 free inventory slots: without them there's no
  armour, but the battery is still used up. Armour is capped at 100 and speed is recomputed from
  the free slots. It then queues the `!HEV_<n>P` charge report with `SetSuitUpdate`, which TS
  drops in multiplayer (§3.5), so no sentence plays.
* **Antidote** (`CItemAntidote`, 0xcb1c0). `models/w_antidote.mdl` (only the model is precached).
  Touching it always succeeds, with no pickup message or sound: it queues `!HEV_DET4` (dropped
  in multiplayer, §3.5) and adds one to the player's antidote count (+0x834,
  `TimeBasedDamage::antidotes`), which cures nerve gas or poison (`CheckTimeBasedDamage`). The
  count is never reset, so it survives death. None of the 97 shipped maps place one.
* **Chargers.** The health charger refills to max in one use. The HEV charger calls
  `TakeArmor(100)` in one use. They recharge after 60 s and 30 s.
* **Items** respawn after 30 s unless `SF_NORESPAWN` (1 << 30) is set, with the `suitchargeok1`
  materialise sound.

### 3.4 Effects

* **`UTIL_ScreenShake`** (0xec978). Only players on the ground shake. The fixed-point duration
  and frequency (`FixedUnsigned16`) are divided by `GiveSlowMul()` = `max(fuser1, 0.01)` and
  truncated, without clamping again (the x87 control word is set to truncate at 0xecad3 and
  0xecb11). `explode::screen_shake` is the only copy; `stunt::screen_shake` re-exports it.
* **`UTIL_ScreenFade`** (0xecdec). Always sent as `TSFade` (`short dur, short hold, short flags,
  byte r,g,b,a`) to net clients, with the fixed-point durations divided by the time scale the same
  way. The stock `ScreenFade` message is never sent.
* **`EMIT_SOUND_DYN`.** It has an extra factor `s`: pitch becomes `(s*fuser1 + 1 - s) * pitch`,
  truncated (0xe0b1e; `explode::slow_pitch`, unit tested). This is used for break sounds
  (s = 0.9), damage, spark, gib and stunt sounds (0.5). Every port caller goes through
  `explode::emit_sound_slow`, which also resolves `!NAME` sentences.
* **`TSExplosion`** (0x7d01c) is in `explode::ts_explosion`, the only port of it: the M61 grenade
  and exploding breakables both call it. With the blast point `origin + 1z`:
  * A screen shake with amplitude dmg, frequency 80, duration 1, radius 2·rMax.
  * `TSFrags` to everyone: origin, `coord (int)dmg` (truncated: GCC sets the x87 control word to
    truncate, Ghidra's `ROUND` notwithstanding), `0`, `0`, `byte 1`.
  * For each entity in the rMax sphere except the inflictor that takes damage and isn't
    `DEAD_DEAD`: a white `TSFade` (222, 227, 189, 0.1 s, hold 0.05, alpha 75) if it's a living
    player, then `falloff_damage` by the distance to `Center() + 1z`: full inside rFull, linear to
    0 at rMax, never below 0. No `rMax == rFull` guard (no caller passes it).
  * In line of sight (fraction 1 or the trace hits the target) the damage goes through
    `TraceAttack` with that trace (impact point and hitgroup). Otherwise a trace back from the
    target, ignoring it, measures the obstacle from the forward trace's end (the blast point if it
    started solid); only `d − thickness` gets through, with the back trace as the impact. That
    is still `TraceAttack` (0x7d635), so the damage is marked traced even though the back trace
    never reports a hitgroup: players get spawn protection, the stunt ×0.7 and `PTakeDam`. Zero
    damage is still dealt, as in TS.
  * Every `WorldGun` in the sphere (damaged or not) is put back into flight
    (`weapons::worldgun::blast`, 0x7d678): `UTIL_SetOrigin`, zero size, `BounceTouch`/`Fall` in
    0.1 s, `MOVETYPE_BOUNCE`, `SOLID_BBOX`, gravity 0.7, friction 0.8, velocity
    `normalize(gun − p) · (rMax − |gun − p|) / slots · 10` with `p` 50 u above the blast point and
    `slots` the weapon's inventory weight (`get_weapon_info` byte `+0xd0`); a gun on the ground
    loses `FL_ONGROUND` and gets `v.z = |v.z| + 50`; `avelocity` y and z are `RandomLong(-300, 300)`.
  * Then the "spectacular" slow-motion point (0.25 for 1 s) at the blast point.

### 3.5 Train and tank driving (decomp `CBasePlayer.c`, `CFuncTank*.c`, `CFuncTrainControls.c`)

The classes are the SDK's. TS changes nothing in them, but the TS data around them matters:

* **Trains.** +use while standing on a `func_tracktrain`, inside its control volume
  (`OnControls`: the train's box plus 72 units up, or the `func_traincontrols` box), takes the
  controls (`PlayerUse` 0xd9848, `plats/train_use1.wav`). `PreThink` (0x908cc; not during intermission) keeps `FL_ONTRAIN`
  set (player movement then ignores input), turns +forward / +back presses into
  `Use(USE_SET, ±1)` speed steps, and lets go when the player jumps, strafes, leaves the ground or
  the train gets `SF_TRACKTRAIN_NOCONTROL`. +use again lets go.
* **Train HUD.** `m_iTrain` holds the notch (`TrainSpeed` 0xd8660: 1 stopped, 2/3/4 forward by
  thirds of the top speed, 5 back) plus `TRAIN_NEW`; `UpdateClientData` (0x91d00) sends `Train`
  (byte `m_iTrain & 0xF`) and clears `TRAIN_NEW`. The TS client hooks `Train` with the stock
  `CHudTrain` (10034ce0..10034df0), which draws `sprites/%d_train.spr` from the `valve`
  directory; the port's client does the same.
* **Tanks.** +use on a `func_tankcontrols` gives the player a `CANCONTROL` tank (`m_pTank`).
  `StartControl` holsters the TS gun (`CTSGun::Holster` → kung fu), clears the weapon and view
  models and sets `HIDEHUD_WEAPONS` (sent as `HideWeapon`). Each `PostThink` (0x91610) fires the
  tank with +attack while the player stays within 30 units of where they pressed +use and has no
  weapon model; otherwise the player lets go and the gun is redeployed (which, since the holster
  switched to kung fu, leaves the player on kung fu). `ItemPostFrame` (0xdc4d0) and so
  `PlayerUse` don't run while a tank is in use. `Killed` and `StartObserver` let go of the tank.
* **No monster bullet damage.** TS's `CHalfLifeMultiplay::RefreshSkillData` (0xcef78) sets only
  the multiplayer player damages, and `CGameRules::RefreshSkillData` reads only `skill`, so
  `gSkillData.monDmg9MM/MP5/12MM` stay 0. A `func_tank` without `bullet_damage` hits for 0; with it,
  `FireBullets` deals that much (`DMG_BULLET`, plus `ALWAYSGIB` above 16, else `NEVERGIB`).
* **`func_tankrocket`** creates `rpg_rocket`, which TS doesn't link, so it fires no rockets; only
  its sprites and targets fire.
* **`func_tankmortar`** explodes `ExplosionCreate(iMagnitude)` (`env_explosion` with damage) where a
  spread trace from the barrel hits.
* **`func_tanklaser`** fires its `laserentity` `env_laser` from the barrel (a full second of the
  laser's `dmg` per shot) and turns it off again when it stops firing.

Port notes:

* The player's half of `StartControl` / `StopControl` (holster or deploy, `HideWeapon` bits) runs
  through `world::defer` once the player object is back in its slot, because +use and `PostThink`
  reach the tank from inside the player's own dispatch. It runs in the same server frame.
* `PlayerUse` is `player::player_use`, run as in TS from `PostThink` → `ItemPostFrame` →
  `ImpulseCommands` (`weapons::post_think`, once the gun's `m_flNextAttack` has run out).
* `CFuncTankLaser::Activate` (find the laser, turn it off; no laser removes the tank) runs from the
  tank's first think instead of `ServerActivate`.
* A tank whose controller disconnects lets go on its next think (port addition; the SDK keeps a
  dangling pointer).
* `FireBullets` for tanks sends a tracer for every shot and a `{shot1..5}` decal on brush
  entities and the world. It doesn't play texture impact sounds (`TEXTURETYPE_PlaySound`) or
  bubble trails under water.
* `firespread 5` reads past TS's 5-entry `gTankSpread` (the SDK resets only values above 5); the
  port uses no spread for it.
* The train HUD is cleared on spawn (TS leaves the last notch shown after dying on a train).
* `m_iWeaponVolume` (AI hearing) isn't kept.

### 3.6 Sentences (decomp `SENTENCEG_*` 0xe00c8–0xe0d64, `PlayLockSounds` 0xb4f50)

* **Loading.** `CWorld::Precache` calls `SENTENCEG_Init`, which reads `sound/sentences.txt` once
  per process (a missing file is retried on the next map, as in TS). Every `NAME words` line is a
  sentence numbered by its position in the file; the client numbers the same file, so the server
  sends `!<index>`. Names ending in digits form groups (`NA0`, `NA1` → `NA`). Limits are TS's:
  1536 sentences, 200 groups, a warning for names of 16+ letters, lines read 510 bytes at a time
  (`memfgets`).
* **Playing.** TS's `EMIT_SOUND_DYN` and `UTIL_EmitAmbientSound` turn `!NAME` into `!<index>`
  (case-insensitive); an unknown name logs "Unable to find … in sentences.txt" (`at_aiconsole`)
  and isn't played. In this port that's `base::emit`, `explode::emit_sound_slow` and
  `sentences::emit_ambient`, so map-provided sounds starting with `!` work wherever those are used.
* **Lock sentences.** `locked_sentence` 1–9 maps to the groups `NA ND NF NFIRE NCHEM NRAD NCON NH NG`
  and `unlocked_sentence` 1–8 to `EA ED EF EFIRE ECHEM ERAD ECON EH` (doors and buttons share the
  table). `PlayLockSounds` plays the group in order (`SENTENCEG_PlaySequentialSz`, volume 0.85,
  `CHAN_VOICE`) at most every 6 s and stops after the last sentence; when the lock sound plays at the
  same time it drops to volume 0.25. Lock sounds now use TS's slow-motion pitch factor (0.5).
* **Suit sentences.** `CBasePlayer::SetSuitUpdate` (0xda410) returns early because TS's rules are
  always multiplayer (`CHalfLifeMultiplay::IsMultiplayer` returns 1), so the HEV queue and
  `CheckSuitUpdate`'s sentence half aren't ported; `sentences::set_suit_update` keeps the call
  sites.
* **Deviation.** `USENTENCEG_Pick` on a group of more than 32 sentences reads past its 32-entry LRU
  array into the next group's record in TS; the port only uses the 32 LRU entries.

### 3.7 +use (`CBasePlayer::PlayerUse` 0xd9848, `player/player_use.rs`)

TS keeps the SDK function, with one addition, and calls it from `ImpulseCommands` (0xdb780), which
`ItemPostFrame` (0xdc4d0) runs only with no tank in use and the gun ready (`m_flNextAttack` run
out, no weapon, or +cancel during a reload). So no +use right after firing or while reloading.

* One search for every package: stock classes (`stock::object_caps`) and the TS entities
  (`entities::ts::object_caps`: ground weapons and ammo, `WorldGun`, `knife`, `grenade`,
  `ts_flag` (not while carried), the hidden objectives; all continuous users). Within 64 units of the origin, caps
  & 0x38, `UTIL_ClampVectorToBox` (0xee5d8) of the eye → `VecBModelOrigin` vector against half
  the size, highest dot with the view above 0.7.
* **TS addition:** if the winner's `CanUsedTroughWalls` (vtable slot 3) is false (ground weapons
  and ammo, dropped guns, the CTB case), a trace from the eyes to its `Center()`
  (`IGNORE_MONSTERS`, skipping the object) must be clear, or nothing happens. Stock classes keep
  the base `CanUsedTroughWalls` (true), so buttons still work through walls as in TS.
* A press plays `common/wpn_select.wav` (`CHAN_ITEM`, 0.4, `ATTN_NORM`) whenever an object is
  found. There is no deny sound.
* `USE_SET` value 1 for held + continuous or pressed + impulse/on-off (continuous also sets
  `PFLAG_USING`, `DriveState::using`, which nothing reads); a release sends on/off users
  `USE_SET` value 0. No stock class in the port has `FCAP_ONOFF_USE`.

## 4. Coverage (97 shipped maps)

These numbers come from running `census.py` (§8) over `ts/maps/*.bsp` with the list of linked classnames.
`tsrp_nukecity_v2.bsp` is not a valid BSP (version 1329865020), so 97 of the 98 files count.

| | Classes | Entities |
|---|---|---|
| All classnames in the maps | 187 | 28,504 |
| Handled by this package | 82 | 25,355 (89.0 %) |
| TS / weapon / ammo / team-spawn classes (other packages) | 70 | 2,948 |
| Still unhandled, stock or foreign | 35 | 201 |

Every priority-1 and priority-2 stock class from design.md §5 is handled. The extras in this package
are:

* `func_plat` (3 maps)
* `func_tracktrain` and `path_track` (4 maps, 161 entities), `func_traincontrols` (1)
* `func_tank` (1), `func_tanklaser` (1), `func_tankmortar` (4), `func_tankcontrols` (6), and
  `func_tankrocket` (unused by the shipped maps)
* `func_pendulum`
* `env_sound`, `env_global`, `env_message`, `env_laser`
* `func_healthcharger`, `func_recharge`
* `trigger_gravity`, `func_friction`, `gibshooter`, `env_spark`, `cycler`
* `env_blood` (8 entities, one map), `game_score` (1), `monster_hevsuit_dead` (1), `xen_*` (12, including `xen_hull`)

**Still unhandled.** TS links these but this port doesn't yet:

* None: every stock class TS links is now handled.

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
  * train driving (ahl_monorail) and the train HUD; `func_tank` (ts_frigid), `func_tankmortar`
    (crossfire, SS_HQ_f3) and `func_tanklaser` (rats3) under player control. Unit tests cover
    `TrainSpeed`, the `Train` / `HideWeapon` wire layouts, barrel angle correction and the spread
    table.
  * real player touch and +use (there are no bots on this branch, and the `player_use` helper isn't
    wired up yet)
  * pushables
  * breakable doors (they need TS blast / shotgun damage from the weapons package)
  * `TSExplosion` damage
  * `env_beam`, `env_laser` and `env_shooter` visuals
  * `env_sound`
  * respawn with a client inside the box
  * `game_text` on screen
  * Everything from #20 and #22 (unit tests only so far): sentences (`!NAME` ambients, door/button
    lock sentences), `cycler` animation, the geiger counter near radiation `trigger_hurt`s, gib
    blood decals, the `xen_plantlight` glow attachment, `env_blood`, `game_score`,
    `monster_hevsuit_dead` gibbing and the `xen_*` animations and tree attack.
  * The client printed `No pfn TSFrags`, `TSFade` and `ItemPickup`. The HUD package hasn't hooked
    those messages yet; the server side is sending them as intended.

## 6. Integration notes (changes needed outside this package)

1. **Keyvalue order (done, #39).** `server-dll`'s `key_value` calls
   `entities::dispatch_key_value`, which follows TS `DispatchKeyValue` (0xaad4c, the SDK's):
   * `entvars_key_value` (`EntvarsKeyvalue` 0xef9f0) runs first, over the full 86-field
     `gEntvarsDescription` table, matched case-insensitively. Its SDK type quirks are kept: `flags`,
     `fixangle`, `deadflag` and `light_level` are `FIELD_FLOAT` over `int` members (they store the
     float's bits), and the edict fields (`owner`, `enemy`, ...) log "Bad field in entity!!" but
     still count as handled.
   * The class's `key_value` only sees the remaining keys, and only when the engine passed a
     `szClassName`. So `multi_manager` and `game_player_equip` treat every other key as a
     target/item without filtering, and the class `skin` branches (`func_door`,
     `func_illusionary`) and `func_pushable`'s `size` presets are gone: they are dead in TS too.
2. **`Activate`.** `ServerActivate` should call an `Entity::activate` hook on every entity, as
   `CBaseEntity::Activate` does in the SDK. Until then, three classes emulate it:
   * `func_train` runs `Activate` from a think 0.1 s after spawning.
   * `func_tracktrain` links all `path_track`s on its first think.
   * `env_beam` runs `BeamUpdateVars` 0.05 s after spawning.
3. **+use.** The `Entity` trait has no `ObjectCaps`.
   * `stock::object_caps(e)` returns the stock classes' use caps.
   * `player::player_use(player, cx, pressed, released)` implements `CBasePlayer::PlayerUse`
     (0xd9848) for every class (stock caps, else `entities::ts::object_caps`); see §3.7. A press
     first takes or leaves train controls (`drive::player_use`). The weapons' PostThink calls it.
   * TS entities get `Use` through `weapons::use_by_player` / `entities::ts::use_by_player`,
     because the player is being dispatched; stock ones through `Entity::use_`.
4. **Round restart.** `CTSGameRules::RespawnEntities` should call
   `stock::breakable::respawn_all(false)`. This covers breakables, breakable pushables and
   breakable doors.
5. **Player state the SDK keeps on `CBasePlayer`.**
   * The health decay above `max_health` (PreThink, `+0x73c`) belongs to the rules package.
   * Kevlar: `items::player_take_armor` calls the one `TakeArmor` port, `weapons::take_armor`
     (vest in `SvState::items & 0x10`, its 10 slots taken from the shared `free_slots`). The HEV
     charger is used from the player's own `PlayerUse`, so the call is deferred until the player
     is back in its slot (same frame). `DropArmor` (0x93a3c) has no callers in TS and isn't ported.
   * The HEV suit bit (`pev->weapons & 1<<31`) is set on spawn (rules package). The battery and
     the HEV charger refuse players without it, as in TS.
6. **`trigger_camera` on death or respawn.** TS `CBasePlayer::Killed` and `Spawn` call
   `SET_VIEW(self, self)` and clear `FL_FROZEN`. The player package must do this too, or a camera
   active when the player dies keeps the view.
7. **Damage.**
   * `take_damage` runs the SDK `TraceAttack` part (breakable hit effects, the door break logic)
     only for damage marked `DamageInfo::traced` (`.traced()`); every caller TS routes through
     `TraceAttack` sets it (see the field's doc comment for the list), and direct `TakeDamage`
     callers (crushers, `trigger_hurt`, breakable touch, ...) leave it false. `RadiusDamage`
     (env_explosion) is traced, with the trace's hitgroup, only when its trace hit something.
   * Doors only break from `TraceAttack`-style damage carrying blast (`0x40`) or the TS bits
     `0x2000000` (shotgun) / `0x4000000` (kung fu).
   * The SDK `DamageDecal` for glass (`{bproof`-style decals 0x19–0x1b, 0x23 for unbreakable glass)
     is for the weapons package to apply when it hits a `func_breakable`.
8. **Explosions.** `stock::explode::ts_explosion` is TS's `TSExplosion` for both the grenade and
   breakables (§3.4), including the `WorldGun` knock-back and the "spectacular" slow-motion point
   (`rules::scoring::do_spectacularness`, 0x7d969/0x7d9b0).
9. **`entities::fire_targets`** skips `FL_KILLME` entities, as the SDK `FireTargets` does (done).
10. **`pfnResetGlobalState`.** Calls `stock::base::reset_global_state()` (done at integration).
11. **Client hooks** (HUD package): `TSFrags` (glass debris and explosions), `TSFade`, `ItemPickup`,
    and the `events/train.sc` handler. (`Train` is drawn by the client HUD, see §3.5.)
12. **Precache** (player package): `player/pl_wade1.wav`, `pl_wade2.wav` and the other SDK
    `ClientPrecache` sounds. The engine plays them for any entity entering water.
13. **`env_sound`.** Call `stock::sound::reset_player_room(index)` when a client connects, so a new
    client in a reused slot starts with the right DSP state.
14. **Geiger counter.** `rules::lifecycle` calls `stock::triggers::geiger_spawn` on spawn and
    `update_geiger_counter` from PreThink (TS `CheckSuitUpdate`), done in #22.

## 7. What's missing or simplified

* Monster AI sounds (`CSoundEnt::InsertSound` from landed gibs) aren't ported: TS has no monsters to
  hear them.

### 7.1 Deviations in the #22 classes

* **`game_score` score message.** `add_points` changes `frags` directly and broadcasts `ScoreInfo`
  through the player object; when the scoring player is the entity currently being dispatched
  (a `game_score` fired from that player's own +use), the message waits for the next score update.
* **Precache location.** TS precaches the blood sprites in `W_Precache` and the gib models in
  `CWorld::Precache`; the port does both from worldspawn (`blood::precache_world`). German gibs
  (`sv_language 1`) aren't supported.
* **worldspawn client cvars.** `v_dark` and `room_type` exist only on clients, so a dedicated
  server skips setting them (FWGS would print "variable not found" each map); the result is the
  same. TS sets `gDisplayTitle` from the `gametitle` flag but never reads it, and the
  `GameTitle` message is never sent, so the port keeps only the spawnflag.
* **`cycler` spawn** skips `ChangeYaw(360)`, a no-op because `ideal_yaw` was just set to the yaw.
* **`monster_hevsuit_dead`** keeps only the corpse half of `CBaseMonster` (TS has no live monsters):
  `g_vecAttackDir` is computed per hit and passed to the gib spawner instead of being a global.

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
`grep -hoP '^\s+\K[a-z_0-9A-Z]+(?=\s+=> )' crates/server/src/entities/stock/*.rs` (classes added for
#22 are linked from their own files: `xen.rs`, `monsters.rs`, `blood.rs`, `maprules.rs`).
