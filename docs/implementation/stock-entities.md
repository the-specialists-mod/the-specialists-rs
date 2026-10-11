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
| `mod.rs` | The `link_entities!` table (one export per classname), `worldspawn` (light styles 0–12 and 63, per-map resets, the world `soundent` and the `bodyque` ring), spawn points, `object_caps` (the +use search itself is `player::player_use`) |
| `base.rs` | `CBaseDelay` (`Delay`, `sub_use_targets`, the `DelayedUse` entity), `CBaseToggle` (`Toggle<T>`, `linear_move` / `angular_move` on `ltime`, their arithmetic in `move_params`), master registry (`is_master_triggered`), `env_global` state table, keyvalue / precache helpers, `create_entity`, `precache_other`, `set_movedir`, axis helpers |
| `explode.rs` | TS `TSExplosion`, SDK `RadiusDamage`, TS `UTIL_ScreenShake` / `UTIL_ScreenFade` (`TSFade`), sparks, ricochet, TS slow-motion pitch `emit_sound_slow` |
| `breakable.rs` | `func_breakable`, `func_pushable`, material tables, `respawn_all`, a map-placed `info_breakable` |
| `doors.rs` | `func_door`, `func_door_rotating`, `func_water`, `momentary_door` |
| `buttons.rs` | `func_button`, `func_rot_button`, `momentary_rot_button`, `button_target`, `multisource`, lock sounds and sentences (`PlayLockSounds`), `DoSpark` |
| `bmodels.rs` | `func_wall`, `func_illusionary`, `func_wall_toggle`, `func_conveyor`, `func_rotating`, `func_pendulum`, `func_monsterclip` |
| `trains.rs` | `func_train`, `path_corner`, `func_plat` (with its trigger; the `CFuncPlat` code is generic over the `Plat` trait, its vtable slots 70–74), `func_platrot`, `func_guntarget` |
| `trackchange.rs` | `func_trackchange`, `func_trackautochange` |
| `tracktrain.rs` | `func_tracktrain`, `func_traincontrols`, `path_track`, `TrainSpeed` |
| `tank.rs` | `func_tank`, `func_tanklaser`, `func_tankrocket`, `func_tankmortar`, `func_tankcontrols` |
| `drive.rs` | The player's side of train and tank driving: `m_iTrain`, `PFLAG_ONTRAIN`, `m_pTank`, `m_iHideHUD`, and the `PreThink` / `PostThink` / `PlayerUse` / `UpdateClientData` code that uses them (`Player::drive`) |
| `triggers.rs` | `trigger_multiple/once/hurt/teleport/push/gravity/auto/relay/changetarget/camera`, `multi_manager`, `func_ladder`, `func_friction`, point entities (`info_target`, `info_teleport_destination`, `info_intermission`, `info_landmark`, `info_null`), `env_global`, `env_message`, `trigger`, `trigger_autosave`, `trigger_cdaudio`, `trigger_counter`, `trigger_endsection`, `trigger_monsterjump` (more `Trigger<K_*>` kinds), `target_cdaudio`, `PlayCDTrack` |
| `levelchange.rs` | `trigger_changelevel` (+ the `fireanddie` it creates), `trigger_transition`, `player_loadsaved`, `player_weaponstrip` |
| `sound.rs` | `ambient_generic` (with the full dynpitchvol / LFO code and the 27 presets), `env_sound`, `speaker` |
| `soundent.rs` | `soundent` (`CSoundEnt`: the AI sound list, `InsertSound`) |
| `sentences.rs` | `SENTENCEG_*`: the sentences.txt loader, `!NAME` lookup, sequential and random group picks, `UTIL_EmitAmbientSound`, the (multiplayer no-op) `SetSuitUpdate` |
| `lights.rs` | `light`, `light_spot`, `light_environment` |
| `effects.rs` | `env_sprite` (+ `CSprite::AnimateAndDie`, `SetAttachment`, `SetTransparency`), `env_glow`, `env_shake`, `env_fade`, `env_explosion` (+ `spark_shower`, `ExplosionCreate`), `env_render`, `env_beam`, `env_laser`, `env_shooter`, `gibshooter` (+ `gib`, `SpawnHeadGib`, `SpawnRandomGibs`), `env_spark` / `env_debris`, `env_bubbles`, `infodecal`, `cycler_sprite`, `cycler`, `cycler_prdroid`, `cycler_wreckage`, `beam` (+ `BeamCreate` / `PointsInit`), `test_effect`, `env_funnel`, `env_beverage`, `item_sodacan` |
| `animating.rs` | `CBaseAnimating` / `CActAnimating` for stock model entities: `ResetSequenceInfo`, `StudioFrameAdvance`, `DispatchAnimEvents`, `SetActivity` / `LookupActivity`, `InitBoneControllers`, `SetSequenceBox` |
| `blood.rs` | `UTIL_ShouldShowBlood`, `UTIL_BloodStream`, `UTIL_BloodDrips`, `UTIL_BloodDecalTrace`, `UTIL_RandomBloodVector`, the blood sprite indices and `g_Language` (`sv_language`, from `crate::precache`), `env_blood` |
| `xen.rs` | `xen_plantlight`, `xen_hair`, `xen_tree` (+ `xen_ttrigger`), `xen_spore_small/medium/large` (+ `xen_hull`) |
| `monsters.rs` | `monster_hevsuit_dead`, and `MonsterBase`: the `CBaseMonster` damage and death code (`TakeDamage`, `DeadTakeDamage`, `Killed`, gibbing, fade) that it, `env_explosion` and `cycler_wreckage` share |
| `items.rs` | `item_healthkit`, `item_battery`, `item_antidote`, `item_suit`, `item_security`, `world_items`, `func_healthcharger`, `func_recharge`, TS `TakeHealth` / `TakeArmor` stand-ins, `EMIT_SOUND_SUIT` |
| `playeritem.rs` | weapons.cpp classes a map can place: `weaponbox`, `weapon_tsgun` and `cycler_weapon` (`CBasePlayerItem`: fall / touch / respawn / attach / drop), and the items' `SetObjectCollisionBox` boxes |
| `maprules.rs` | `game_text`, `game_player_equip` (+ `GiveNamedItem`), `game_score` (+ `AddPoints` / `AddPointsToTeam`), `game_end`, `game_zone_player`, `game_player_hurt`, `game_counter`, `game_counter_set`, `game_team_master`, `game_team_set`, `game_player_team` |

How the SDK patterns map to Rust:

* **`SetThink` / `SetTouch` / `SetUse` / `m_pfnCallWhenMoveDone`.** These become fields of type
  `Option<fn(&mut Self, &mut Cx, ...)>` on the class: `Toggle<T>::{think, touch, move_done}`, or
  `ThinkFn<Self>` in classes without a `Toggle`. Classes with only a few think states use a small
  enum. `Entity::think` / `touch` call through these fields (`run_think`, `run_touch`).
* **Push movers.** `CBaseToggle::LinearMove` (0xe3658) and `AngularMove` (0xe386c) set
  `nextthink = ltime + travel` and `velocity = delta / travel` with TS's x87 precision
  (`move_params`: exact deltas, the length rounded to a float, `travel = length / speed` kept
  unrounded). Like TS they don't check the speed: a speed of 0 gives an infinite `nextthink` and
  no velocity, so the mover never arrives. `MOVETYPE_PUSH` entities think on `ltime`;
  `next_think(cx, d)` picks `ltime` or global time from the movetype.
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
* **Save data.** Each class's `save_fields` walks its TS `Save` chain (`crate::save`, design §4.9)
  and `restored` is its TS `Precache`. TS state the port didn't keep before (keyvalues TS stores,
  member-pointer states) was added where the save shows it; §1.2 lists what each class saves.

### 1.1 Map-wide precaches

The engine numbers models, sounds and events in precache order, so `crate::precache` issues
`CWorld::Precache` (0xf449c)'s precaches exactly as TS does, from worldspawn's spawn:

1. `InstallGameRules`: the multiplayer rules' `CVoiceGameMgr::Init` precaches
   `sprites/voiceicon.spr` (single player doesn't).
2. `W_Precache` (0xf12e8): the `UTIL_PrecacheOther`s (`item_suit`'s `w_suit.mdl`, `ts_bomb`'s
   `misc/beep2.wav`; the other TS classes precache nothing), `CTSGun::Precache`'s 11 events,
   `ts_powerup`'s model, then the explosion, smoke, bubble, blood and laser sprites,
   `models/grenade.mdl`, `sprites/explode1.spr` and the debris / grenade / bullet hit sounds and
   `items/weapondrop1.wav`.
3. `ClientPrecache` (0xaeafc): the SDK step, swim, ladder, wade, debris, pain, HUD and geiger
   sounds and `models/player.mdl` (TS's `giPrecacheGrunt` is 0).
4. `TSPrecache` (0x9f320): player and debris sounds, then for each weapon id 0..37 its non-empty
   shell, `v_`, `p_`, silenced `p_` and `w_` models and its two sounds (the shared tables give
   the same names as TS's `Write_*` functions), then the shared weapon sounds and models and the
   `m61_grenade`, `shrapnel` and `ts_flag` precaches.
5. `common/null.wav`, `items/suitchargeok1.wav`, `items/gunpickup2.wav`,
   `common/bodydrop3.wav`, `common/bodydrop4.wav`, then the gibs: `models/germangibs.mdl` with
   `sv_language 1` (`g_Language`), else `hgibs.mdl` and `agibs.mdl`.

`UTIL_PrecacheOther` creates and removes an entity; the freed edict is reused by the next
allocation, so the port lists the class precaches directly. Entity classes precache the rest in
their `Spawn`, as in TS.

Every class the port links was audited against its TS `Precache` (vtable slot 8) and the
precaches its `Spawn`, `KeyValue` and helpers issue (#104): set and order of first occurrence,
which is what decides the indices (the engine gives a repeated name its first index, and models,
sounds and events are numbered separately). The audit changed:

* `func_healthcharger`: `medshot4`, `medshotno1`, `medcharge4` (`CWallHealth::Precache`
  0xc8cc4); `func_recharge` in `CRecharge::Precache`'s order (0xc6f8c).
* `func_pushable` without `SF_PUSH_BREAKABLE` precaches only its push sounds
  (`CPushable::Precache` 0xc1894), not the breakable's material sounds, gibs and `spawnobject`.
* `locked_sound`/`unlocked_sound` (and the sentences) of doors and buttons are BYTEs from the
  16-bit `fistp` (`CBaseDoor::KeyValue` 0xb51a0, `CBaseButton::KeyValue` 0xa8184), so 256
  precaches nothing.
* `ts_particle`'s `soundonemit` key matches in any case (`strcasecmp`, 0xf8644).
* `UTIL_PrecacheOther` (`base::precache_other`, for `func_breakable`'s `spawnobject`) runs only
  the class's `Precache` (`items::precache_class`; the ground weapons and ammo have an empty one)
  instead of a whole `Spawn`, and logs "NULL Ent in UTIL_PrecacheOther" for a class neither
  library exports (`weapon_9mmhandgun`, `ammo_9mmclip`, `weapon_9mmAR` (the export is
  `weapon_9MMAR`), `weapon_357`, `ammo_357`,
  `weapon_tripmine`).
* Precaches TS doesn't issue were dropped, though their names were already precached: the items'
  `items/suitchargeok1.wav` and the battery's `weapons/items.wav`, `func_tracktrain`'s
  `plats/train_use1.wav`, `weapon_tsgun`'s `models/w_glock18.mdl` (`CTSGun::Spawn` 0x8053c only
  sets it; `TSPrecache`'s weapon loop precaches it with every world model); `env_shooter`
  precaches its model before the material sounds.

Left as they are, all repeats of names the world precached: `env_explosion` and
`cycler_wreckage` precache `zerogxplode.spr`/`steam1.spr` (and `grenade.mdl`) to get the indices
TS keeps in `g_sModelIndexFireball`/`g_sModelIndexSmoke`. Guards: the `precache_model_s`/`precache_sound_s` helpers skip an empty name,
and `ts_model` with an empty `model` is removed; TS would pass "" to the engine.

`goldsrc::ext::entities` adds the following:

* `vec` maths and `EHandle`.
* Entity searches: `entities_in_box`, `find_in_sphere`, `find_by_target_name`.
* Engine calls the base wrapper didn't have: `create_entity`, `set_view`, `static_decal`,
  `decal_index`, `model_frames`, `set_cvar_string`, `make_vectors`, `walk_move`, `trace_model`.
* Sound constants (`ATTN_*`, `snd::*`) and all `TE_*` codes, with `SVC_TEMPENTITY`.
* `MsgWriterExt::vec3` and temp-entity writers: `te_sparks`, `te_break_model`, `te_explosion`,
  `te_smoke`, `te_bsp_decal`, `te_bubbles`, and `te_text_message` (`UTIL_HudMessage`, 511-character
  limit as in TS).

### 1.2 Save data

Every class's `save_fields` walks its TS chain (`save::tables::chain`, design §4.9); the doc
comment on each says what it saves. What matters across classes:

* **Restores that stop after `"BASE"` (TS).** A set member function pointer is counted but not
  written (TS hands `pfnNameForFunction` the first word of a g++ 2.95 member pointer), so the
  restore reads the next block's header as a field and the rest of that entity's chain is lost.
  This hits most triggers (touch set), `func_friction`, `multi_manager`, plats with a
  `targetname`, moving trains, `func_rotating`, spawned `multisource`s, buttons that aren't
  touch-only, pendulums with a distance, a respawning item or charger, a falling or fading
  `monster_hevsuit_dead`, a `speaker`. The port reproduces it: restored objects come back with the
  member pointers cleared and the later tables unread (so `multisource` reads as fully on, a
  restored `func_rot_button` has `m_fRotating` 0, items and chargers stay inert).
* **Member-pointer state the port didn't keep** was added as flags or enums where TS sets a pointer
  that the port's code didn't model (`func_rotating`'s `RotatingUse`, a pendulum's `RopeTouch`,
  `CBreakable::Die`, `multisource`'s `Register`, items' `Materialize`, `func_traincontrols`,
  `env_message`, `func_friction`, the effects' think/use states).
* **Fields TS saves that the port dropped** are kept now: `CBaseToggle`'s `m_flActivateFinished`,
  `m_flTWidth`, `m_flTLength`, `m_cTriggersLeft`, `m_bitsDamageInflict` (in `Toggle`), the
  `CBaseTrigger` keys of `func_ladder` and `trigger_changelevel`, `func_train`'s `sounds`,
  `CBaseDelay` keys of `test_effect` / `env_funnel` / `env_beverage` / the xen plants / `DelayedUse`,
  the cyclers' `CBaseToggle` keys, a door's `m_bHealthValue` as TS's byte (`fistp16_low_byte`), a
  light's `m_iszPattern`, the charger's `dmdelay` (`m_iReactivate`), `func_trackchange`'s
  `train` / `toptrack` / `bottomtrack`. Classes whose `Spawn` never sets `m_toggle_state` start at 0
  (`AtTop`), TS's zero-filled value.
* **`restored` is TS's `Precache`.** Its side effects come with it: `trigger_auto` fires 0.1 s after
  a load, `fireanddie` `m_flDelay` after one, `func_plat` builds a new trigger volume, a restored
  `worldspawn` repeats the whole map precache (game rules, soundent, body queue),
  `ambient_generic` restarts an active looping sound.
* **`OverrideReset`** (a global entity restored over a live one): `func_train`, `func_tracktrain`,
  `func_trackchange` / `func_trackautochange`. `CFuncTrackChange::OverrideReset` installs the real
  `CFuncTrackChange::Find` (0xd5ff4), which `Spawn` never does (§7.3), so its train logic
  (`Find`, `EvaluateTrain`, `UpdateTrain`, `UpdateAutoTargets`, both `Use`s, `CPathTrack::Nearest`)
  is ported for that path.
* **`Activate` after a load.** TS's `ServerActivate` runs every entity's `Activate` after a restore;
  the port has no such hook, so `func_train`, `func_guntarget`, `func_tanklaser` and `env_beam` queue
  it through their next think (0.1 s for the trains).
* **Not saved by TS**, so lost on a restore as in TS: `func_breakable`'s `CBaseDelay` part and its
  respawn state (`CBreakable` saves straight from `CBaseEntity`), the breakable-door state,
  `game_player_equip`'s item list, `game_team_master`'s team index, `cycler_weapon`'s model,
  `weapon_tsgun`'s inventory beyond its timers and clip.

Guards (TS reads or writes out of bounds on a bad save): a restored `multisource` `m_iTotal` is
capped to 0..32 and a `multi_manager` `m_cTargets` to 0..16 (negative `m_index` to 0); a restored
`triggerType` above 2 is `USE_TOGGLE`; a restored xen spore `skin` outside the model list
precaches nothing; a restored `m_toggle_state` outside 0..3 is `AtBottom`; `CFuncTrackChange`'s
`Find` / arrival / `CFuncTrackAuto::Use` leave a track that isn't a `path_track` NULL (TS calls
`Nearest` through NULL) and treat a NULL activator as no train.

Deviations: `CFuncTankRocket::Precache`'s `UTIL_PrecacheOther("rpg_rocket")` (a class TS doesn't
link) is skipped, so its "NULL Ent" console line is missing; a dropped `weapon_tsgun` saved in the
0.1 s before its removal writes its own spawn-time timers, not the player's (the player is
mid-dispatch); `game_team_master` resets its index on a global restore too (TS keeps the live
one); `env_message`'s think state is read from `nextthink != 0` (only the chapter title sets it).
The track change that its own train fires reads the train's path through a side channel
(`tracktrain::FIRING`), since the train is taken out of the world during its think.

## 2. Classes

"SDK" means the class is ported from the SDK file named, with no TS change found in the decomp.
"TS" means the decomp differs and the TS behaviour is what's ported.

| Classname | Source | Notes |
|---|---|---|
| `worldspawn` | `CWorld::KeyValue` 0xf4a18, `CWorld::Precache` 0xf449c | Keys: `skyname` (`sv_skyname`), `sounds` (CD track), `WaveHeight` (`pev.scale` = v/8, `sv_wateramp`), `MaxRange` (`pev.speed`), `chaptertitle`, `startdark`/`gametitle`/`defaultteam` (spawnflags 1/2/4), `newunit` (`sv_newunit 1`); `gamemode`, `mapteams`, `teammodels` go to `rules::world_key_value`. Spawn first forces `sv_gravity 800`, `sv_stepsize 18`, `room_type 0`, then follows `CWorld::Precache`: installs the rules (`rules::install`; the multiplayer rules precache the voice icon), creates the soundent and the body queue, loads sentences and the texture types, issues TS's map-wide precaches in TS's order (§1.1), then sets `sv_zmax` (`MaxRange`, or 4096 when ≤ 0), shows the chapter title through an `env_message` 0.3 s in, and sets `v_dark` and `mp_defaultteam` from the spawnflags. Light styles 0–12 and 63. Resets the masters, path links and env_sound rooms on each map. |
| `func_wall`, `func_illusionary` | SDK bmodels.cpp | `func_wall` now sets `FL_WORLDBRUSH` and toggles its texture frame on Use. `func_illusionary` is `MOVETYPE_NONE`, and `skin` sets its contents. |
| `func_wall_toggle`, `func_conveyor` | SDK | The conveyor speed is sent in `rendercolor`. |
| `func_rotating` | SDK | Fan sounds, spin-up/down, hurt touch. Uses `fabs` where gcc 2.95 used the integer `abs`. |
| `func_pendulum` | SDK | The INSTANT flag thinks on `ltime` (the SDK compares global time against `ltime`). |
| `func_breakable` | **TS** `CBreakable` | See §3.1. |
| `func_pushable` | SDK 2.3 + **TS** | 2.3 `Move` plus TS's 0.3 factor when pulling with +use. A breakable pushable uses the TS breakable code. |
| `func_door`, `func_door_rotating`, `func_water` | **TS** `CBaseDoor` / `CRotDoor` | See §3.2. Lock sounds and sentences as in §3.5. `movesnd`/`stopsnd` are BYTEs set like `func_plat`'s (`CBaseDoor::KeyValue` 0xb51a0, 16-bit `fistp`), `common/null.wav` isn't precached. |
| `func_button`, `func_rot_button` | SDK buttons.cpp | Spark sounds use TS slow-motion pitch. Shootable buttons (health > 0) work through `take_damage`. Lock sounds and sentences as in §3.5. |
| `momentary_door` | SDK 2.3 doors.cpp (TS `CMomentaryDoor` 0xb7d98..0xb87f4) | A `CBaseToggle`, so none of TS's `CBaseDoor` changes (§3.2) apply. Keys `movesnd` (`doors/doormove1..8.wav`, else `common/null.wav`; a BYTE via a 16-bit `fistp`), `stopsnd` and `healthvalue` (ignored), `lip`. Only `USE_SET` moves it: `value` (clamped to ≤ 1, no lower clamp) places it between closed and open, at 10 × the distance per second; the move sound (`CHAN_STATIC`, TS slow-motion pitch) starts when it isn't already moving. No stop sound, touch, `Blocked` or +use (`ObjectCaps` 0); `dmg` defaults to 2 unread. |
| `momentary_rot_button` | SDK 2.3 buttons.cpp (TS `CMomentaryRotButton` 0xa948c..0xaa9d8) | Continuous +use (`ObjectCaps` 0x10) unless spawnflag 1, which also makes it solid. Each use turns it 0.1 s more at `speed` (default 100), reversing after every stop, moves every `momentary_rot_button` with the same `target`, and sends `USE_SET` with its position (`AxisDelta / distance`, from before this turn: the SDK 2.3 latency, not HL25's prediction) to the entities named by `target`. Spawnflag 16 with `returnspeed` > 0 turns it back on release, still updating the targets while above 0. A negative `distance` swaps the ends. `sounds` (`ButtonSound`) on the first turn after a stop, `CHAN_VOICE` with TS slow-motion pitch. |
| `button_target`, `multisource` | SDK | |
| `func_train`, `path_corner` | SDK plats.cpp | `Activate` is emulated (see §6). `movesnd`/`stopsnd` are `CBasePlatTrain::KeyValue`'s (0xd1f80): the low byte of the `strtod` truncated to a short, as for `func_plat`; `common/null.wav` for any other number is only allocated (worldspawn precached it). |
| `func_plat` | **TS** `CFuncPlat` | Adds the TS keys `targettop` and `targetbot`, which fire when the plat reaches the top or bottom. Spawns its own trigger volume. Sounds use TS slow-motion pitch (0.5). `movesnd`/`stopsnd` keep the low byte of the truncated short (`CBasePlatTrain::KeyValue` 0xd1f80: 257 is sound 1). `Blocked` (0xd2e64) logs "`<class>` Blocked by `<class>`" (`at_aiconsole`). |
| `func_platrot` | SDK `CFuncPlatRot` (TS 0xd3010..0xd3294) | A `func_plat` that also turns `rotation` degrees about its `AxisDir` axis (spawnflag 64 Z, 128 X, else Y), from 0 at the bottom to `rotation` at the top, over the move's duration (`RotMove`: under 0.1 s, the whole turn becomes the angular velocity with a think 1 s on). Starts at the top angles with a targetname. |
| `func_trackchange` | **TS** `CFuncTrackChange` (0xd5d84..0xd6a98) | A toggle `func_platrot` that never carries a train in TS (§7.3). Any use at the top goes down, at the bottom goes up; moving, it ignores uses. Spawnflag 8 starts at the bottom, 16 only turns (`AngularMove`, bottom at the spawn height). A 2 s think runs `CFuncTrainControls::Find` on it. |
| `func_trackautochange` | **TS** `CFuncTrackAuto` (0xd6b60, 0xd6c48) | The same platform; its `Use` does nothing reachable in TS (§7.3), so it never moves. |
| `func_guntarget` | **TS** `CGunTarget` (0xd6e48..0xd7da8) | A shootable brush (`FL_MONSTER`, `takedamage` 0 until used). `Activate` puts it on its first `path_corner` (emulated, §6). A use that passes `ShouldToggle` against `m_on` (never set, as in the SDK) restores `max_health`, sets `DAMAGE_AIM` and follows the corners at `speed` (default 100), firing each corner's `message` (once with flag 4) and waiting its `wait`; spawnflag 1 starts it 0.3 s in. Damage wears `health` down; at 0 it stops (`takedamage` 0) and fires its own `message`. Traced hits go through `CBaseMonster::TraceAttack` (0xb3848): no blood, and hitgroups 1–7 scale by TS's never-set (0) `gSkillData.mon*` (a brush hit is always hitgroup 0). `Killed` is never called. |
| `func_tracktrain`, `path_track` | SDK | Includes branches, alternate paths, banking, dead ends, and the `events/train.sc` pitch updates. Players drive it (see §3.5). |
| `func_traincontrols` | SDK `CFuncTrainControls` | Hands its box to the named train as the control volume (`SetControls`), then removes itself. |
| `func_tank`, `func_tanklaser`, `func_tankrocket`, `func_tankmortar` | SDK func_tank.cpp + **TS** | Automatic (tracks a player in its PVS) and player-controlled tanks. See §3.5 for what TS changes. |
| `func_tankcontrols` | SDK | +use passes on to the first `func_tank*` named by `target` (found 0.3 s after spawn). |
| `trigger_multiple`, `trigger_once`, `trigger_teleport`, `trigger_push`, `trigger_gravity` | SDK triggers.cpp | `UTIL_ShowMessage` → `HudText`. |
| `trigger_hurt` | SDK 2.3 | Negative damage heals with TS `TakeHealth`, with no dead-player check (2.3). The per-player bitmask in `impulse` is kept (multiplayer only: with `deathmatch` 0, `CHalfLifeRules`, nothing is hurt again before `dmgtime` except in the same frame). With `DMG_RADIATION` in `damagetype`, **TS** `RadiationThink` (0xe703c) lowers the geiger range of the player who can see the trigger's centre, every 0.25 s; `update_geiger_counter` (`UpdateGeigerCounter` 0xda1f0, from the player's PreThink) sends `Geiger` (range / 4) to suited players when it changes. |
| `trigger_auto`, `trigger_relay`, `trigger_changetarget` | SDK | |
| `trigger_camera` | SDK | `SET_VIEW`, `FL_FROZEN` for TAKECONTROL, `path_corner` moves. |
| `multi_manager` | SDK | Includes thread clones. |
| `func_ladder`, `func_friction` | SDK | |
| `trigger` | **TS** `CBaseTrigger` (link 0xe67c8) | Spawn is `CBaseEntity::Spawn`: no model, not solid, no touch or use. Only `CBaseTrigger::KeyValue` (0xe68bc) runs. |
| `trigger_counter` | **TS** `CTriggerCounter::Spawn` 0xe7b54, `CounterUse` 0xe79b0 | `count` (`CBaseTrigger::KeyValue`, truncated) is `m_cTriggersLeft`, 2 when 0. No `InitTrigger` (no model, not solid). Each use counts down and keeps the activator; at 0 `ActivateMultiTrigger` (master, `noise`, targets, `message`) runs with `wait` -1, so it removes itself 0.1 s later. A `player`-classname activator gets "Only N more to go..." / "There are more to go..." / "Sequence completed!" as `at_console` alerts (no newline) unless spawnflag 1. |
| `trigger_cdaudio`, `target_cdaudio` | **TS** 0xe6bb0..0xe6f08, `PlayCDTrack` 0xe6c34 | `PlayCDTrack((int)health)` stuffs `cd play %3d` (`cd pause` for -1; other values outside -1..30 log "out of range") into the client in slot 1 only, then the entity removes itself. `trigger_cdaudio`: brush, player touch or any use. `target_cdaudio`: point; `radius` (`scale`) > 0 checks slot 1's distance every 0.5 s from 1 s after spawn; any use plays. |
| `trigger_autosave`, `trigger_endsection` | **TS** `CTriggerSave` 0xe945c, `CTriggerEndSection` 0xe95e4 | Removed at spawn under the multiplayer rules (`IsDeathmatch()`); with `deathmatch` 0 (`CHalfLifeRules`) they work: a player touch (master-gated) runs `autosave`; `section` (`message`) goes to `pfnEndSection` on a player touch (not with spawnflag 1) or a use by a player or no activator. |
| `trigger_changelevel`, `trigger_transition` | **TS** `CChangeLevel` 0xe7da0..0xe865c, `CTriggerVolume::Spawn` 0xe7bc0 | Keys `map`, `landmark` (32+ characters log "too long"), `changetarget`, `changedelay`, plus `CBaseTrigger`'s. Touch by a `player` (not with spawnflag 2) or use (with a targetname). `ChangeLevelNow` does nothing under the multiplayer rules; otherwise once per frame: slot 1 (or the world) must overlap one of the `trigger_transition`s named like the landmark (if any exist), then a `fireanddie` is created for `changetarget`, the targets fire, `vecLandmarkOffset` is set from the `info_landmark` and `pfnChangeLevel(map, landmark)` runs. `trigger_transition` only keeps its brush box (model dropped). |
| `trigger_monsterjump` | **TS** `CTriggerMonsterJump` 0xe6a0c..0xe6ae4 | `SetMovedir` + brush; speed 200, height 150. A `FL_MONSTER` toucher is lifted 1 unit and thrown along `movedir`·speed + 150 up; the trigger goes non-solid on the next think. With a targetname it starts off and toggles on use. TS has no monsters, so nothing triggers it. |
| `player_loadsaved` | **TS** `CRevertSaved` 0xddd0c..0xddf38 | Use: `TSFade` out to everyone (`rendercolor`, `duration`, `holdtime`, `renderamt`), `message` to every player after `messagetime`, `reload` after `loadtime` when `deathmatch` is 0. |
| `player_weaponstrip` | **TS** `CStripWeapons::Use` 0xddbb0 | `RemoveAllItems(FALSE)` (`weapons::remove_all_items_keep_suit`: the TS gun holstered and destroyed with its inventory and reserve ammo, view/weapon models cleared, HEV suit bit kept, `UpdateClientData`, `CurWeapon 0,0,0`) on a player activator, or on slot 1 when not under the multiplayer rules. Inside the `Use`, also for a player whose own +use fired it (lent, `world::run_now_or_defer`). |
| `func_monsterclip` | **TS** `CFuncMonsterClip` 0xa55d0 | `func_wall` with `FL_MONSTERCLIP`; `effects` is set to `EF_NODRAW` (not or-ed) unless `showtriggers`; Use does nothing. |
| `info_target`, `info_teleport_destination`, `info_intermission`, `info_landmark`, `info_null` | SDK | `info_null` removes itself. |
| `env_global`, `env_message` | SDK | Global states persist across maps, like `gGlobalState`. |
| `ambient_generic` | SDK sound.cpp | The TS preset table is byte-identical. `!NAME` sentences aren't precached and are sent as `!<index>` (§3.5). |
| `env_sound` | SDK | Per-player room state lives in a side table in `sound.rs`. |
| `light`, `light_spot`, `light_environment` | SDK lights.cpp | Lights without a targetname are removed. Switchable styles are 32 and up. `light_environment` sets `sv_skycolor_*` and `sv_skyvec_*`. |
| `env_sprite`, `env_glow`, `env_render`, `env_spark`, `env_debris`, `env_bubbles`, `infodecal`, `cycler_sprite` | SDK effects.cpp / world.cpp / h_cycler.cpp | A sprite with an `aiment` follows that entity's attachment `body` (`MOVETYPE_FOLLOW`, skin = entity index), as `CSprite::Precache` / `SetAttachment` do; maps can't set `aiment`, so this is for code-created sprites (the `xen_plantlight` glow). `cycler_sprite` is +usable (TS impulse cap). |
| `env_shake` | SDK + **TS** shake | Duration and frequency are divided by each player's time scale (`fuser1`). |
| `env_fade` | **TS** `CFade` | Sent as `TSFade` with durations divided by the time scale. TS spawns it as a brush. New spawnflag 8 fades only the clients inside the brush. |
| `env_explosion`, `spark_shower` | SDK explode.cpp | TS doesn't route these through `TSExplosion`. Uses SDK `RadiusDamage`, scorch decals, fireball and smoke temp entities. `CEnvExplosion` is a `CBaseMonster`, so its `TakeDamage` is `CBaseMonster::TakeDamage` (§6 item 7), reachable when a map sets `takedamage`. |
| `env_beam` | SDK `CLightning` | Server-side beams work as `FL_CUSTOMENTITY` beam entities: `BeamUpdateVars` runs on the first think. Strike beams use temp entities. Damage and decals are included. |
| `env_laser` | SDK `CLaser` | Includes the end sprite and damage. |
| `env_shooter`, `gibshooter`, `gib` | SDK | Gib bounce sounds use TS slow-motion pitch. `gibshooter` gibs bleed red: up to 5 blood decals, one per airborne bounce (`BounceGibTouch` 0xb1ebc, `violence_hblood`); `env_shooter` gibs don't bleed. |
| `cycler_prdroid` | SDK `CCyclerProbe` (TS 0xc752c) | A `cycler` 16 units up with `models/prdroid.mdl` and a ±16 box; renamed `cycler` like every `GenericCyclerSpawn` cycler, so it is +usable as one. |
| `cycler_wreckage` | SDK `CWreckage` (TS 0xc7fe4..0xc8118) | Its model (no collision), animated with `StudioFrameAdvance`; every 0.2 s a `TE_SMOKE` (`sprites/steam1.spr`, scale 50–99, framerate 8–11) at a random point of its box. With `dmgtime` set it smokes ever less often until then (`RandomFloat(0, dmgtime − (int)spawn time)` against the time left) and is removed after it. `TakeDamage` is `CBaseMonster`'s, which refuses: `Spawn` clears `takedamage`. |
| `beam` | SDK `CBeam` (TS 0xb8cb4 / 0xb8cd4) | A map-placed one is just `SOLID_NOT` and takes its end entities from `owner`/`aiment`. |
| `env_lightning` | SDK `CLightning` | The same class as `env_beam`. |
| `test_effect` | SDK `CTestEffect` (TS 0xbcbc8..0xbcf4c) | On use, one `sprites/lgtning.spr` beam (`BeamCreate` width 100, colour 255/180/100, scroll 12) every 0.1 s to where a 128-unit trace in a random direction ends, up to 24; each brightens with its share of the 3 s (truncated to an int); 3 s after the use all are removed. |
| `env_funnel` | SDK `CEnvFunnel` (TS 0xbdcfc..0xbde24) | On use, one `TE_LARGEFUNNEL` to everyone (`sprites/flare6.spr`, spawnflag 1 reverses), then `SUB_Remove`. |
| `env_beverage`, `item_sodacan` | SDK `CEnvBeverage` / `CItemSoda` (TS 0xbde50..0xbe178) | Each use drops a can (`models/can.mdl`, skin from the beverage's, 6 = random 0–5) while the last one was drunk (`frags` 0) and cans are left (`health`, default 10). The can lands after 0.5 s (`weapons/g_bounce3.wav`, TS slow-motion pitch), then a player touching it gets 1 health (`TakeHealth`) and the beverage may drop the next. |
| `speaker` | SDK `CSpeaker` (TS 0xe1988..0xe1d88) | `preset` 1–12 (`C1A0_` … `C3A2_`) or the `message` key (neither: "SPEAKER with no Level/Sentence!" and removed). Starts 5–15 s after spawn unless spawnflag 1; toggled by use. Plays a random sentence of the group (`SENTENCEG_PlayRndSz`, volume `health`/10, attenuation 0.3) every 15–135 s, holding off while `CTalkMonster::g_talkWaitTime` (5 s after any speaker, never reset across maps) hasn't passed; a `!sentence` plays once as an ambient sound. |
| `soundent`, `bodyque` | SDK `CSoundEnt` (TS 0xe2140..0xe281c) / `CCorpse` | worldspawn creates a classless `soundent` (`GetClassPtr<CSoundEnt>`) and four `bodyque`s linked in a ring, before any map entity (TS edict numbering). The soundent keeps the 64-sound list with one never-expiring slot per client and expires sounds every 0.3 s; landed bleeding gibs add a meat sound. A live player's `PostThink` updates its slot (`CBasePlayer::UpdatePlayerSound` 0xda630, `player::impulse::update_player_sound`): its origin, `bits_SOUND_PLAYER`, and a volume that follows the speed on the ground (at most 512, +100 while jumping) and fades 250/s, 0 under impulse 105's `m_fNoPlayerSound`; `Killed` resets the slot (`CSound::Reset`, which also cuts the active list after it, as in TS). Nothing reads it (no monsters); `displaysoundlist 1` prints "Soundlist: active / free (change)". `CopyToBodyQue` is never called in TS (`StartDeathCam` has no callers), so the corpses stay empty. A map-placed `soundent` takes its client reserve from the world's list, as in TS. |
| `fireanddie`, `DelayedUse` (map-placed) | TS `CFireAndDie` / `CBaseDelay` | Linked because TS exports them. A map's `fireanddie` fires its targets (`delay`, `killtarget`) only if the map sets `nextthink`, or `m_flDelay` after a restore (its `Precache` schedules the think); a map's `DelayedUse` never thinks (only `SUB_UseTargets` arms one). |
| `shrapnel` | **TS** `CTSShrapnel` (0x7c0a0..0x7f09c), `entities::ts::shrapnel` | Nothing in TS calls `CTSShrapnel::Create`, so only a map places one: invisible (`EF_NODRAW`), bouncing, slowed like the other `CTSPhysicObject`s. It hits a player for 100 (`DMG_NEVERGIB | DMG_SLASH`, traced), sticks 6 units back into `worldspawn` / `func_wall`, otherwise bounces back at a quarter speed with `knife_hit.wav`; loses 1 u/s per think, is removed under 100 u/s unless stuck, and 30 s after spawning (10 s after its last hit). Continuous +use caps, no `Use`. |
| `cycler` | SDK h_cycler.cpp | Server-side studio animation (`CCycler::Think` 0xc778c): bone controllers zeroed, `frame` advanced, one-shot sequences restart. +use toggles the animation. Damage steps to the next sequence (wrapping to 0) while animating, or one 0.1 s frame step while paused. |
| `env_blood` | SDK effects.cpp `CBlood` | `color` 1 = yellow, else red; `amount`. Drips (`TE_BLOODSPRITE`, amount doubled for multiplayer) or, with spawnflag 2, a stream (red becomes colour 70). Spawnflags 1 random direction, 4 at the activator's (or player 1's) eyes, 8 blood decal along the direction. |
| `game_score` | SDK maprules.cpp (same in TS 0xcc4d8..0xcc568) | `points` (`strtol`, into `frags`) to the activating player (`IsPlayer`: any slot player; `AddPoints` 0xd9e18, `ScoreInfo` to all), or with spawnflag 2 to each other slot player who is a teammate (`AddPointsToTeam` 0xd9f68, `PlayerRelationship`: never outside teamplay); without spawnflag 1 a score isn't taken below 0 (the floor is `(int)-frags`). `frags` is read with a C `(int)` cast (`maprules::x87_int`: truncation, 0x80000000 for NaN/out of range). `master` gates it. |
| `game_end` | SDK maprules.cpp (TS 0xcc668) | `EndMultiplayerGame`: the intermission (`frame::go_to_intermission`); nothing when `deathmatch` was 0 (TS's `CHalfLifeRules`). `master` gates it. |
| `game_zone_player` | SDK maprules.cpp (TS 0xcce90..0xcd148) | Brush (`CRuleBrushEntity::Spawn`: model set, invisible, not solid). Keys `intarget`, `outtarget`, `incount`, `outcount`, `master`. On use, each slot player (`UTIL_PlayerByIndex`, players who left included) is inside when `pfnTraceModel` from its origin to its origin with the human hull (head hull when `FL_DUCKING`) against the brush starts solid; it fires `intarget`/`outtarget` with the player as activator, our activator as caller and our use type and value. Then `incount`/`outcount` get `USE_SET` with the counts (activator, us as caller). |
| `game_player_hurt` | SDK maprules.cpp (TS 0xcd16c) | A player activator takes `dmg` (`DMG_GENERIC`, the entity as inflictor and attacker), or `-dmg` health (`TakeHealth`) when `dmg` is negative, inside the `Use` even for a player whose own +use fired it (lent, `world::run_now_or_defer`); then the targets fire with our use type and value; removed after firing with flag 1. `master` gates it. |
| `game_counter` | SDK maprules.cpp (TS 0xcd24c..0xce510) | Spawn keeps `(int)frags` in `dmg`. `USE_OFF` counts down, `USE_ON`/`USE_TOGGLE` up, `USE_SET` sets `(int)value`. When `(int)frags == (int)health` the targets get `USE_TOGGLE`; flag 1 removes it, flag 2 resets the count to `dmg`. `master` gates it. |
| `game_counter_set` | SDK maprules.cpp (TS 0xcd418) | `USE_SET` with its `frags` to its targets; removed after firing with flag 1. `master` gates it. |
| `monster_hevsuit_dead` | SDK `CDeadHEV` | `models/player.mdl`, body 1, sequence from `pose` (0–3: `deadback`, `deadsitting`, `deadstomach`, `deadtable`; a missing sequence logs "Dead hevsuit with bad pose" and gets `EF_BRIGHTFIELD`). Falls (`MOVETYPE_TOSS`) and takes its sequence box on landing. Crush/fall/blast/sonic/club damage wears it down (10 %) and gibs it when a hit reaches its health: head gib + 4 human gibs with blood, `common/bodysplat.wav`; with `violence_hgibs 0` it fades out instead. |
| `xen_plantlight`, `xen_hair`, `xen_tree`, `xen_ttrigger`, `xen_spore_small`, `xen_spore_medium`, `xen_spore_large`, `xen_hull` | SDK xen.cpp | Studio animation through `animating.rs`. The light plant ducks on player touch (glow sprite attached to attachment 1 goes `EF_NODRAW`, targets fired off) and rises 5 s later (targets on). The tree swings at anything touching its 48×48×128 trigger 64 units ahead or shooting it, and its `TREE_AE_ATTACK` model event deals 25 crush/slash, punch 15 and a 100 u/s shove. Large spores add five invisible `xen_hull` leg boxes. |
| `item_healthkit`, `item_battery`, `item_antidote` | **TS** pickup rules | See §3.3. With `deathmatch` 0 (`CHalfLifeRules`) a taken item is removed for good. |
| `item_suit` | SDK `CItemSuit` (TS 0xcb59c..0xcb60c) | `models/w_suit.mdl`. Refused while the player has the HEV suit bit, which every TS player gets on spawn; otherwise it plays `!HEV_AAx` (`!HEV_A0` with spawnflag 1) through `EMIT_SOUND_SUIT` (0xe0bb0: `CHAN_STATIC`, `suitvolume`, silent at ≤ 0.05, pitch 100 or 98–104) and sets the bit. |
| `item_security` | SDK `CItemSecurity` (TS 0xcb9e8..0xcba58) | `models/w_security.mdl`; always taken, no sound or message. TS adds one to `m_rgItems[ITEM_SECURITY]` (+0x838), which nothing reads; the port doesn't keep it. |
| `world_items` | SDK `CWorldItem` (TS 0xcad0c / 0xcad6c) | Key `type`: 42 `item_antidote`, 43 `item_security`, 44 `item_battery`, 45 `item_suit` (created and spawned at its origin and angles, then given its `target`, `targetname` and spawnflags); anything else logs "unable to create world_item %d". Removes itself. |
| `weaponbox` | SDK `CWeaponBox` (TS 0xf2868..0xf320c) | `models/w_weaponbox.mdl`, toss, trigger, zero size but a ±16 × 0..16 touch box (`SetObjectCollisionBox`). Every key that isn't an entity field is an ammo name (`PackAmmo`; after 32, "WeaponBox too full!"), but TS's `MaxAmmoCarry` (0xf0c0c) knows no ammo (the only `ItemInfoArray` entry, `weapon_tsgun`, names none), so each logs "MaxAmmoCarry() doesn't recognize" and nothing is packed. A living player touching it once it is on the ground hears `items/gunpickup2.wav` and it is removed. |
| `weapon_tsgun` | **TS** `CTSGun::Spawn` 0x8053c + `CBasePlayerItem` 0xf1698..0xf1d80 | Placed by a map or created by `GiveNamedItem` (`PlayerSpawn`, `game_player_equip`). `models/w_glock18.mdl`, falls (`FallInit`/`FallThink`; `weapondrop1.wav` only with an owner), then a trigger with a ±24 × 0..16 touch box. A living player without a gun takes it (`DefaultTouch`, reaching a player lent by its own call: `player::with_player_now`): `weapons::add_gun` (empty, kung fu, weapon bit 16), `CheckRespawn`, `weapons::switch_to_gun` (holstering an active cycler), `AttachToPlayer`, `items/gunpickup2.wav`, its `target` fires (`delay`/`killtarget` honoured). A player who already holds a gun leaves it lying (a `GiveNamedItem` one at its feet, for anyone else). A `GiveNamedItem` gun is taken in the air, so it keeps its `FallThink` (every 0.1 s, never landing) while held. Under the multiplayer rules and without `SF_NORESPAWN` a hidden copy appears at the same spot and materialises (`suitchargeok1.wav`) 20 s later, or at once with `mp_weaponstay`. The taken entity stays as the player's gun item (`AttachToPlayer`: following, hidden, no model) until `RemoveAllItems` drops it (removed 0.1 s later). |
| `cycler_weapon` | SDK `CWeaponCycler` (TS 0xc7c78..0xc8840) | The map's `model` as a slidebox with a ±24 × 0..16 touch box. A living player without one takes it (`DefaultTouch`): HL item slot 1, weapon bit 0, `gunpickup2.wav`, `target` fired, then attached like the gun. Under the multiplayer rules and without `SF_NORESPAWN` a copy respawns, which has no model: TS precaches an empty name there (GoldSrc stops with "Bad string", FWGS warns), and so does the port. It is deployed (view model = its model, `m_flNextAttack` 1, weapon anim 0) as `FShouldSwitchWeapon` decides: under the multiplayer rules only with no active item (no TS gun), in single player always, holstering a held gun to kung fu; a gun given later holsters it. `use cycler_weapon` / `use weapon_tsgun` switch between them (`weapons::select_item`). Its attacks never run (TS's `ItemPostFrame` returns for anything but the gun). Dropped with the gun by `RemoveAllItems` (death think, observer, disconnect, `player_weaponstrip`). |
| `game_player_equip` | SDK maprules.cpp (same in TS 0xcd494) | Unknown keys are items (`name#n` stripped, count ≥ 1, max 32), given through `GiveNamedItem` (item spawned at the player's feet with `SF_NORESPAWN`, then touched). Touch (master, not `USEONLY`) / use (no master). TS `PlayerSpawn` touches every one on spawn instead of giving the gun and buying the loadout (`maprules::equip_on_spawn`, called from `weapons::on_spawn` inside the spawn, the player lent to its slot). `weapon_tsgun` is TS's gun entity: a new, empty gun for a player without one, otherwise left lying at its feet. The item touches reach a player whose own +use or spawn is on the stack (`player::with_player_now`), as in TS. |
| `game_team_master` | TS 0xccae8..0xccd40 | `teamindex` (-1 none), `triggerstate` (0 off, 2 toggle, else on), master. `USE_SET` takes the activator's team (`GetTeamIndex(TeamID)`) or none for a negative value; other uses fire the targets when the activator's team matches (`UTIL_TeamsMatch`: always outside teamplay; `ANYTEAM` with no team), once with flag 1. It is also a master (`IsTriggered` = `TeamMatch`), for spawn points, doors and the rest. |
| `game_team_set` | TS 0xccd64 | `USE_SET` to its targets with 0, or -1 with `CLEARTEAM` (2); once with flag 1. |
| `game_player_team` | TS 0xcd740..0xce658 | A player activator moves to the team of the targeted `game_team_master` (`ChangePlayerTeam(team, kill = flag 2, gib = flag 4)`; nothing outside teamplay); once with flag 1. The change happens inside the `Use`, as in TS: a player whose +use, `game_playerdie`, `game_playerjoin` or `game_playerspawn` fired it, or whose own shot or swing did (a shot button or `func_breakable`), is lent back to its slot meanwhile (`world::lend`, `world::run_now_or_defer`; rules.md §7). |

## 3. TS deviations from the SDK (ported)

### 3.1 `func_breakable` (decomp `CBreakable.c`)

* **New keys.**
  * `respawntime` (int): 0, the default, means respawn 30 s after breaking; a negative value means
    never.
  * `respawntarget`: fired on respawn when spawnflag 8 is set. Without it, `target` is fired. The
    delay is respected.
  * `gibmodel` and `spawnobject`. The `spawnobject` table matches the SDK, checked against the binary.
    `Precache` runs `UTIL_PrecacheOther(spawnobject)` (§1.1).
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
  * A breakable one sets `g_vecAttackDir` from its box centre to the inflictor's origin
    (0xc1532). (Guard: TS reads through a NULL inflictor; the port leaves the global alone.)
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
* **Use** debris flies away from the activator, and `g_vecAttackDir` becomes the `v_forward` of
  the restored angles before `Die`. Use does nothing once the breakable is dead.
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
* **Dead players.** `CItem::ItemTouch` (0xcaf8c) checks only `IsPlayer` and `CanHaveItem` (always
  1). Only the healthkit's and the battery's `MyTouch` refuse a dead player (`deadflag`), so a
  dead player touching an antidote, a suit or a security card takes it.

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
  inside the tank's `Use`, as in TS: +use (`player_use`), `PostThink` (`drive::post_think`) and
  `Killed` / `StartObserver` (`drive::release_tank`) reach the tank from inside the player's own
  dispatch and lend the player to its slot meanwhile (`world::lend`, `world::run_now_or_defer`).
  So `StartObserver`'s `StopControl` redeploys the emptied gun (kung fu) before `RemoveAllItems`.
  A player held by a dispatch that didn't lend it gets its half once that returns.
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
  `USE_SET` value 0. No stock class in the port has `FCAP_ONOFF_USE`. `momentary_rot_button`
  (without spawnflag 1) is a continuous user: it keeps turning while +use is held.

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
* `func_platrot`, `func_trackchange`, `func_trackautochange`, `func_guntarget` (#95; unused by the
  shipped maps)
* `func_pendulum`
* `env_sound`, `env_global`, `env_message`, `env_laser`
* `func_healthcharger`, `func_recharge`
* `trigger_gravity`, `func_friction`, `gibshooter`, `env_spark`, `cycler`
* `env_blood` (8 entities, one map), `game_score` (1), `monster_hevsuit_dead` (1), `xen_*` (12, including `xen_hull`)
* #95, none placed by a shipped map: `beam`, `bodyque`, `cycler_prdroid`, `cycler_weapon`,
  `cycler_wreckage`, `env_beverage`, `env_funnel`, `env_lightning`, `item_security`,
  `item_sodacan`, `item_suit`, `player_loadsaved`, `player_weaponstrip`, `soundent`, `speaker`,
  `target_cdaudio`, `test_effect`, `trigger`, `trigger_autosave`, `trigger_cdaudio`,
  `trigger_changelevel`, `trigger_counter`, `trigger_endsection`, `trigger_monsterjump`,
  `trigger_transition`, `func_monsterclip`, `momentary_door`, `momentary_rot_button`,
  `weaponbox`, `world_items`, map-placed `info_breakable` / `weapon_tsgun` / `fireanddie` /
  `DelayedUse`, and TS's `shrapnel`

**Linked classes.** Since #95 the port exports every entity class `ts_i386.so` and `mp.dll` export,
including those no shipped map places (`nm -D --defined-only` / `objdump -p` compared; the only
remaining export differences aren't entity classes, see §7.5). A class TS links behaves as in TS
when a third-party map places it, within the deviations in §7.

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
  * #102: `move_params` (no speed check, division by the unrounded travel), the items that
    refuse dead players, the `fistp` sound bytes (`strtod` as a double), TS's `ObjectCaps` table,
    `ChangeList`'s flags and transition-list dedupe, the save-table helpers
    (`goldsrc::ext::saverestore`), the precache lists' TS order and the gib models per language
  * #104: `UpdateOnRemove`'s `EntitySetState` (only known names) and `DispatchSpawn`'s
    `globalname` rules (added, dead, dormant), `MonsterBase`'s gib type, `ShouldGibMonster` and
    gibs per `Classify`, the chargers' precache order, the lock keys as bytes, the German build's
    skipped blood splat
  * #95: the soundent free/active lists and expiry, `speaker` presets and toggle use,
    `test_effect` brightness, the item touch boxes and weapon respawn time, the CD audio
    command, counter messages, transition box overlap, weaponstrip targeting, the momentary
    door/wheel maths and the plat/track change helpers. The #95 classes haven't been run
    in-engine (no shipped map places them).
  * #107: `anglemod` (65536-step quantisation, the truncating `fistp` and its out-of-range
    value), `Q_log2`, `Length` (`shared::pm::math`), and `Server_GetBlendingInterface`'s version
    check and constant 0 (`ts_server::studio`). The export lists were compared with
    `nm -D --defined-only` / `objdump -p` (§7.5).
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
   `CBaseEntity::Activate` does in the SDK. Until then, four classes emulate it:
   * `func_train` runs `Activate` from a think 0.1 s after spawning.
   * `func_tracktrain` links all `path_track`s on its first think.
   * `env_beam` runs `BeamUpdateVars` 0.05 s after spawning.
   * `func_guntarget` runs `CGunTarget::Activate` 0.1 s after spawning (or on its first use if
     that comes first); spawnflag 1's `Start` keeps the `ltime + 0.3` `Spawn` gave it.
3. **+use.** The `Entity` trait has no `ObjectCaps`.
   * `stock::object_caps(e)` returns the stock classes' use caps.
   * `player::player_use(player, cx, pressed, released)` implements `CBasePlayer::PlayerUse`
     (0xd9848) for every class (stock caps, else `entities::ts::object_caps`); see §3.7. A press
     first takes or leaves train controls (`drive::player_use`). The weapons' PostThink calls it.
   * TS entities get `Use` through `weapons::use_by_player` / `entities::ts::use_by_player`,
     because the player is being dispatched; stock ones through `Entity::use_`.
4. **Round restart.** `CTSGameRules::RespawnEntities` (`entities::ts::respawn_entities`) calls
   `stock::breakable::respawn_all(false)` first, standing in for the `info_breakable` helpers:
   breakables and breakable pushables with `respawntime` ≥ 0, and every door.
5. **Player state the SDK keeps on `CBasePlayer`.**
   * The health decay above `max_health` (PreThink, `+0x73c`) belongs to the rules package.
   * Kevlar: `items::player_take_armor` calls the one `TakeArmor` port, `weapons::take_armor`
     (vest in `SvState::items & 0x10`, its 10 slots taken from the shared `free_slots`). The HEV
     charger is used from the player's own `PlayerUse`, which lends the player to its slot, so
     the armour lands inside the `Use` (`world::run_now`), as in TS. `DropArmor` (0x93a3c) has no callers in TS and isn't ported.
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
     `0x2000000` (shotgun) / `0x4000000` (kung fu). A direct `TakeDamage` on a door is
     `CBaseEntity::TakeDamage`, as for every class without its own.
   * `CBaseEntity::TakeDamage` (0xab994, `entities::base_take_damage`) is the `Entity::take_damage`
     default: refused with `takedamage` off; `g_vecAttackDir` from the brush centre to the
     inflictor's origin; a `MOVETYPE_WALK`/`STEP` entity is pushed away from a real inflictor
     (`DamageForce`, at most 1000) unless the attacker is `SOLID_TRIGGER`; the health drops, and
     at 0 `CBaseEntity::Killed` (`takedamage` off, `DEAD_DEAD`, `UTIL_Remove`). It matters for
     entities that have `takedamage` without their own `TakeDamage` (a breakable door, a map's
     `takedamage` key). Guards: TS reads the inflictor's origin and the attacker's `solid`
     through NULL; the port skips the direction and push without an inflictor and pushes for a
     NULL attacker.
   * `TakeDamage` follows each class's TS vtable slot (`vtables.tsv`, vptr index 22). Besides
     `CBaseEntity`'s, TS has `CBasePlayer`, `CBaseButton` (`func_button`, `func_rot_button`),
     `CButtonTarget`, `CBreakable`, `CPushable`, `CCycler` (`cycler`, `cycler_prdroid`),
     `CCyclerSprite`, `CGunTarget`, `CXenTree` and `CXenSpore`, and `CBaseMonster::TakeDamage`
     (0xb2430) for `monster_hevsuit_dead`, `env_explosion` and `cycler_wreckage`.
     `monsters::MonsterBase` ports that one for these AI-less classes: refused with `takedamage`
     off; dead (`deadflag` `DEAD_DEAD`) → `DeadTakeDamage` (0xb2a10); otherwise
     `g_vecAttackDir` from a real inflictor (`inflictor->Center() - (0,0,10) - Center()`), the
     health drops and at 0 `Killed` (0xb1b74, gib type from `DMG_ALWAYSGIB`/`DMG_NEVERGIB`):
     the first time `common/null.wav`, and without a gib an `FL_MONSTER` entity `BecomeDead`s;
     a gib (`GIB_ALWAYS`, or `GIB_NORMAL` below -30 health) runs `CallGibMonster`. With
     `Classify` 0 (`env_explosion`, `cycler_wreckage`) that is `EF_NODRAW`, the
     `common/bodysplat.wav` splat and `FadeMonster` (`SUB_StartFadeOut`, then `SUB_FadeOut` and
     `SUB_Remove` replace `env_explosion`'s think until its next use; `CWreckage` overrides
     `Think`, so it keeps smoking); the HEV corpse (`Classify` 4) gibs as a human. TS dropped
     the SDK's push of a `MOVETYPE_WALK` victim, and `MakeIdealYaw` is empty, so nothing else is
     visible. The gibs fly against `g_vecAttackDir`.
   * `SUB_Remove` (`base::sub_remove`: `speaker`, `env_funnel`, `item_sodacan`, a fired
     `trigger_once`, the dead HEV corpse, impulse 202's blood splat) frees the edict at once
     (`REMOVE_ENTITY`); `UTIL_Remove` (`entities::remove`) waits for the end of the frame
     (`FL_KILLME`), as in TS. Both first run `UpdateOnRemove` (0xe2ccc, `base::update_on_remove`;
     no TS class overrides it): an entity with a `globalname` marks its `gGlobalState` entry
     dead (`EntitySetState`, which never adds a name). Its `FL_GRAPHED` half clears node-graph
     links, which the port doesn't have.
   * `DispatchSpawn` (0xaab8c, `entities::dispatch_spawn`, also run by `CBaseEntity::Create`
     and `ChangeLevelNow`'s `fireanddie`) starts the bounds one unit around the origin, spawns,
     fails (-1) for an `FL_KILLME` entity (the rules' `IsAllowedToSpawn` is always 1), then for a
     `globalname`: a new name is added as on with the current map, a dead one fails the spawn
     (a removed global entity never comes back), and one another map added makes the entity
     dormant (`MakeDormant` 0xac270: `FL_DORMANT`, not solid, `MOVETYPE_NONE`, `EF_NODRAW`, no
     think, relinked). Guard: `CGlobalState::EntityAdd` (0xf3ee4) `strcpy`s the name and map
     into 64- and 32-byte fields, overflowing on longer ones; the port keeps whole strings.
   * The SDK `DamageDecal` for glass (`{bproof`-style decals 0x19–0x1b, 0x23 for unbreakable glass)
     is for the weapons package to apply when it hits a `func_breakable`.
8. **Explosions.** `stock::explode::ts_explosion` is TS's `TSExplosion` for both the grenade and
   breakables (§3.4), including the `WorldGun` knock-back and the "spectacular" slow-motion point
   (`rules::scoring::do_spectacularness`, 0x7d969/0x7d9b0).
9. **`entities::fire_targets`** skips `FL_KILLME` entities, as the SDK `FireTargets` does (done).
10. **`pfnResetGlobalState`.** Calls `stock::base::reset_global_state()` (done at integration).
11. **Client hooks** (HUD package): `TSFrags` (glass debris and explosions), `TSFade`, `ItemPickup`,
    and the `events/train.sc` handler. (`Train` is drawn by the client HUD, see §3.5.)
12. **Precache**: `ClientPrecache` and the rest of TS's map-wide precaches are issued by
    `crate::precache` in TS's order (§1.1, #102).
13. **`env_sound`.** Call `stock::sound::reset_player_room(index)` when a client connects, so a new
    client in a reused slot starts with the right DSP state.
14. **Geiger counter.** `rules::lifecycle` calls `stock::triggers::geiger_spawn` on spawn and
    `update_geiger_counter` from PreThink (TS `CheckSuitUpdate`), done in #22.

## 7. What's missing or simplified

* Monster AI sounds: the soundent list, the gibs' `InsertSound` and the players' slots are ported
  (§2). Nothing reads the list (TS has no monsters).

### 7.1 Deviations in the #22 classes

* **Map rules and a mid-dispatch activator.** A player whose own +use, shot or swing, spawn or
  `game_playerdie`/`game_playerjoin`/`game_playerspawn` fired us is lent back to its slot
  (`world::lend`), so `game_score`'s `ScoreInfo` (it reads the deaths from the player object),
  `game_player_hurt`'s damage (before its targets fire) and healing decay timer, and
  `player_weaponstrip`'s `RemoveAllItems(FALSE)` all happen inside the `Use`, as in TS
  (`world::run_now_or_defer`). Only a player held by a dispatch that didn't lend it (no TS path
  is known to reach one) gets them once that dispatch returns, in the same frame.
* **NULL activator.** TS's `game_score` and `game_player_hurt` call `IsPlayer()` through the
  activator and crash on a NULL one; the port treats it as not a player (the hurt still fires its
  targets).
* **German gibs** (`sv_language 1`, `g_Language`) follow TS: `germangibs.mdl` for head and
  random gibs, `germanygibs.mdl` for `gibshooter`, no gib blood decals and red blood drawn in
  colour 0. `CGib::SpawnStickyGibs` isn't ported (nothing in TS calls it).
  `CBloodSplat::Spray` (0xdb428, impulse 202, `player/impulse.rs`) draws no decal with
  `g_Language` 1 but still removes itself 0.1 s later.
* **worldspawn client cvars.** `v_dark` and `room_type` exist only on clients, so a dedicated
  server skips setting them (FWGS would print "variable not found" each map); the result is the
  same. TS sets `gDisplayTitle` from the `gametitle` flag but never reads it, and the
  `GameTitle` message is never sent, so the port keeps only the spawnflag.
* **`cycler` spawn** skips `ChangeYaw(360)`, a no-op because `ideal_yaw` was just set to the yaw.
* **`monster_hevsuit_dead`**, `env_explosion` and `cycler_wreckage` keep only the parts of
  `CBaseMonster` that can show without AI (`MonsterBase`, §6 item 7): `m_bitsDamageType`, the
  conditions, `m_vecEnemyLKP` and the monster states aren't kept, since nothing reads them.

### 7.2 Deviations in the #95 trigger classes

The single-player branches are ported as TS compiled them: they test `g_pGameRules->IsDeathmatch()`
(the port's `rules::state` `single_player`, set when `deathmatch` was 0 at install) or, for
`player_loadsaved`'s reload, `gpGlobals->deathmatch`. The port differs only where TS reads or
writes out of bounds, or where the port lacks save/restore:

* **Long map and landmark names.** `trigger_changelevel` `strcpy`s `map` and `landmark` into
  32-byte fields (and the map again into the 32-byte `st_szNextMap`), so a longer name overflows
  into the next field. The port keeps each name whole (the "too long" error is still logged).
* **Empty slot 1 in `ChangeLevelNow`.** TS takes slot 1's object (or the world's) and crashes in
  `InTransitionVolume` when the edict has none; the port uses the edict's entvars. Likewise a
  `MOVETYPE_FOLLOW` player whose `aiment` has no object: TS crashes in `Intersects`, the port
  reads the aiment's entvars.
* **`trigger_counter` with a NULL activator.** `CounterUse` reads the classname through
  `m_hActivator->` and crashes when the activator is NULL or has no object; the port treats it as
  not a player (no message) and still counts.
* **`player_weaponstrip` on the world.** Without a player activator and outside the multiplayer
  rules, TS calls `CBasePlayer::RemoveAllItems` on the object of edict 1, or of the world when
  `INDEXENT(1)` is NULL, writing past `CWorld`. The port strips only a `Player`. When the player
  being stripped is mid-dispatch (its own +use fired the strip), the strip runs as soon as that
  dispatch returns, in the same frame.
* **`PlayCDTrack` out of range.** TS's "TriggerCDAudio - Track %d out of range" passes no
  argument, so it prints stack garbage; the port prints the track.
* **Transition list.** `ParmsChangeLevel` (0xaea68 → `ChangeList` 0xe87c4) is ported
  (`levelchange::parms_change_level`): with `pSaveData` set it fills `levelList[]` /
  `connectionCount` from every `trigger_changelevel` whose landmark exists (16 at most) and flags,
  in the engine's save table, the entities in each landmark's PVS that may cross
  (`FENTTABLE_MOVEABLE` for `FCAP_ACROSS_TRANSITION`, `FENTTABLE_GLOBAL` for a non-dormant
  `globalname`, plus the connection bit, when inside the transition volume or with
  `FCAP_FORCE_TRANSITION`). `ObjectCaps` comes from a table of TS's classes by classname
  (`ts_object_caps`, generated from the vtables: slot 12 of each linked class). The entities'
  save data is written and read as TS's (§1.2, design §4.9), so they cross, and the `fireanddie`
  that `changetarget` creates fires `m_flDelay` after the next level restores it (its `Precache`).
  Guards: names over 31 bytes are cut where TS `strcpy`s them into the engine's 32-byte fields
  (and in the saved `m_szMapName` / `m_szLandmarkName`); TS's 512-entry PVS arrays overflow from
  the 513th entity (after its "Too many entities" error), the port's list grows; `EntityFlagsSet`
  refuses index `tableCount`, which TS accepts (never reached: the index comes from
  `EntityIndex`).

### 7.3 Deviations and TS behaviour in the #95 plat and track classes

* **Track changes never reach their tracks or train (TS).** `CFuncTrackChange::Spawn` (0xd5d84)
  sets its think to `CFuncTrainControls::Find` (the GOT slot loaded at 0xc5e75 objdump relocates
  to `Find__18CFuncTrainControls`), not `CFuncTrackChange::Find` (0xd5ff4), which only
  `OverrideReset` (save-game restore) installs. So `toptrack`, `bottomtrack` and `train` are never
  looked up and `m_trackTop`/`m_trackBottom`/`m_train` stay NULL: `EvaluateTrain` is always
  `TRAIN_SAFE` (no `buttons/button11.wav` alarm, though it is precached), the train is never moved
  or re-pathed, `UpdateAutoTargets` never toggles a `path_track`, and `CFuncTrackAuto::Use` does
  nothing. The port keeps exactly that. Only a global restore over a live track change reaches the
  real `Find`, through `OverrideReset`; that path and the train logic behind it are ported (§1.2).
  The think itself is
  ported: with a `target` naming a `func_tracktrain`, that train gets the platform's box as its
  control volume and the platform is removed; otherwise "No train" is logged.
* **NULL activator.** `CFuncTrackAuto::Use` reads the activator's classname first and crashes on a
  NULL one (a delayed fire from a non-player); the port ignores the use.
* **`GetDelay`.** `CGunTarget::Wait` takes the corner's `GetDelay()`; the port reads a
  `path_corner`'s `wait` and uses 0 for anything else, as `func_train` does (in TS a
  `CBaseToggle` target would give its own `wait`).
* **`func_guntarget` `Activate`** is emulated by a think (§6).
* **Crushers** (`CFuncPlat::Blocked` 0xd2e64 and the doors', trains' and rotators'
  `Blocked`) call the victim's `TakeDamage` directly, which decides itself whether `takedamage`
  matters: a stopped `func_guntarget` with health left loses it when crushed, a shootable
  `func_breakable` or a button with `takedamage` 0 still takes the hit (`damage::apply` has no
  `takedamage` gate since #102).

### 7.4 Deviations in the #95 momentary classes

* **A `momentary_rot_button` that targets itself** (its `target` is also its `targetname`)
  calls its own `Use` from `UpdateTarget` in TS, which recurses until the stack overflows. The
  port's re-entrant dispatch of the entity being dispatched is skipped, so the button just turns.
* **x87 rounding.** TS evaluates the door's open position, its `Use` destination and distance,
  the wheel's end angles, its position fraction and the `+ 0.1` think times on the x87 stack;
  the port does those steps in `f64` (`mad_x87`, `momentary_door_pos2`, `momentary_door_move`,
  `axis_fraction`, `plus_tenth`), which can still differ from the 80-bit result in the last bit
  of rare values. NaN compares follow the binary (a NaN `returnspeed` counts as positive; a NaN
  position stops a returning wheel at its start).

### 7.5 Deviations in the other #95 classes

`pfnSetAbsBox` now runs `entities::set_object_collision_box`: the item boxes above for
`weaponbox`, `weapon_tsgun` and `cycler_weapon`, else TS's base `::SetObjectCollisionBox`
(0xabf9c), including its cube for rotated BSP models, which the port's old box (size + 1
everywhere) lacked.

* **A deployed `cycler_weapon` read as the gun.** TS's `ItemPostFrame` gate reads the active
  item's `+0xcc` (for a cycler its `m_iModel`) and, with +cancel held during `m_flNextAttack`,
  reads through it as a pointer; a few `ClientCommand` and pickup paths test `m_pActiveItem`
  only for NULL before using it as a `CTSGun`. With a cycler active those read past the cycler
  object (out of bounds); the port leaves the +cancel clause out and treats the cycler as no gun
  there.
* **`speaker` preset outside 1–12.** TS's switch has no default and reads an uninitialised
  pointer; the port plays nothing and logs the "bad sentence group name" error with an empty
  name.
* **`item_security`'s count** (`m_rgItems[ITEM_SECURITY]`) isn't kept: nothing reads it.
* **Exports.** Against `ts_i386.so` the port exports every entity class, plus (#107)
  `Server_GetBlendingInterface` and the `pm_math.c` helpers `anglemod`, `Length` and `Q_log2`
  (`server-dll/src/exports.rs`, design.md §4.1). TS's remaining defined dynamic symbols are
  global variables (cvars and tables), GCC 2.95-mangled C++ functions, `_init`/`_fini`/`_rotr`,
  and the other C functions of `pm_shared.c`/`pm_math.c` (`PM_*`, `TS*`, `PseudoClimb`,
  `Vector*`, `Angle*`, `CrossProduct`, ...; the port's movement is `shared::pm` and nothing looks
  those up). Against `mp.dll` the port has every plain-name export, including (#107)
  `Server_GetBlendingInterface` and `CreateInterface`, and lacks only the 159 C++-mangled member
  functions MSVC exported.
* **`Server_GetBlendingInterface`** (0xa3f80, mp.dll 0x10008850) is ported as TS has it: it
  returns 0, so the engines use their own server bone setup, as they do with TS. TS has no
  server-side bone maths (its `SV_StudioSetupBones`, 0xa3fdc, only logs "SV_StudioSetupBones
  called."), so player hitboxes on the server don't use the client's torso/legs split in TS
  either. Guard: TS writes through the interface out-pointer and reads the studio API without
  NULL checks; the port skips a NULL one.
* **`Length`**: TS reads the vector without a NULL check; the port returns 0 for NULL.
  **`Q_log2`** never returns for a negative argument, as in TS. Nothing calls either.
* **`CreateInterface`** (mp.dll 0x10028f70): nothing in mp.dll registers an interface, so it
  always sets `IFACE_FAILED` (when the return-code pointer isn't NULL) and returns NULL.

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
