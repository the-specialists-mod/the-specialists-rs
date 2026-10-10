# Client: view, events, effects, prediction (render package)

Port of the TS 3.0 `client.dll` view, weapon events, effects and prediction glue. Source of truth:
decomp `docs/client/rendering-effects-prediction.md` (cited as **R§n**), `docs/server/weapons.md`
(**W§n**), `docs/server/architecture.md` §7, and the decompiled client C / `objdump` of
`client.dll` where the doc was vague (noted inline in the code).

| File | Contents |
|---|---|
| `crates/client/src/view.rs` | `V_CalcRefdef`, bob, stair smoothing, stunt roll, recoil + view shake, third-person camera, FOV and time-scale easing, slow-mo tint + `TSFade` |
| `crates/client/src/events.rs`, `events/{fire,impact,misc,info,random}.rs` | the 11 event hooks and handlers, bullets, shells, decals, texture sounds, client weapon info (classes, anim sets), shared random |
| `crates/client/src/effects.rs`, `effects/{psys,fx,emitters,laser,tempents}.rs` | sprite particle system, weapon/impact effects, BSP-lump emitters, wind, lasers/flashlights/shadows, `HUD_AddEntity`, `HUD_TempEntUpdate`, effect messages |
| `crates/client/src/prediction.rs`, `prediction/gun.rs` | Txfer callbacks, `HUD_PostRunCmd`, stopgap local gun predictor |
| `crates/client/src/state.rs` | fields **added** (see "Integration") |
| `crates/goldsrc/src/ext/render.rs` | `ev!`/`efx!`/`tri!` macros, engine wrappers, `Cvar`, `vec3` maths |
| `crates/shared/src/protocol/effects.rs` | `TSFade`, `TSPSys` (wind), `PTakeDam`, `TSFrags`, `TSMBlur`, `TSSlowMo`, `TSBTime` payloads: `decode` + `encode(&mut impl Sink)` |
| `crates/goldsrc-sys/build.rs` | **changed beyond the allowlist lines**: see below |

### `goldsrc-sys/build.rs` change

`cdll_int.h` only forward-declares `event_api_s`, `efx_api_s`, `triangleapi_s`, `screenfade_s`, so
allowlisting alone produced opaque types. Besides the allowlist lines
(`event_args_t|event_api_t|efx_api_t|triangleapi_t|TEMPENTITY|screenfade_t|dlight_t|pmtrace_t|physent_t|mstudioevent_t|TRICULLSTYLE`
and `FTENT_|TRI_|FEV_|FFADE_|TENTPRIORITY_`), one `.header_contents("render_ext.h", ...)` line
includes `r_efx.h`, `event_api.h`, `event_flags.h`, `triangleapi.h`, `screenfade.h`,
`studio_event.h`, `shake.h`. `wrapper.h` is untouched. Float/flag macros bindgen can't export
(`ATTN_NORM`, `FCVAR_ARCHIVE`) and `client_data_t` are defined in `ext/render.rs`.

## What is implemented (TS function -> Rust)

| TS (client.dll) | Address | Rust | Status |
|---|---|---|---|
| `InterpolateSlowMotion` | `0x10055ae0` | `view::ease_time_scale` (called in `view::frame`) | done, unit-tested |
| `CHud::Think` FOV easing | `0x10049760` | `view::ease_fov` | done, unit-tested |
| `V_CalcRefdef` | `0x10069b90` | `view::calc_refdef` | done |
| `V_CalcNormalRefdef` | `0x10067890` | `view::calc_normal` | done (see deviations) |
| `V_CalcSpectatorRefdef` | `0x10068f70` | `view::calc_spectator` | simplified (in-eye, chase, free) |
| `V_CalcIntermissionRefdef` | `0x100676e0` | `view::calc_intermission` | done (no idle sway) |
| `V_CalcBob` / `V_SmoothBobZ` | `0x10066e70` / `0x10066de0` | `view::calc_bob` | done (cl_bob levels 0..4) |
| `V_SmoothStairs` + view-height smoothing | `0x100677d0` | in `calc_normal` | done |
| `V_CalcViewRoll` / `V_CalcRoll` | `0x100675b0` / `0x100671f0` | `view::calc_view_roll` | done |
| `V_ApplyKick` | `0x10057ed0` | in `calc_normal` (step into `cl_viewangles`) | done, verified in game |
| `ViewShake::Start` / `Update` | `0x10057fa0` / `0x10058000` | `view::Shake` | done, unit-tested |
| `CTSGun::VKickback` (client) | `0x10008230` | `view::kick` (prediction: `view::kick_angles`) | done, constants checked against the decomp |
| screen fade + `TSFade` | end of `0x10069b90`, `0x1004c1e0` | `view::write_screen_fade`, `view::fade_color`, `view::on_tsfade` | done, verified in game |
| `V_SetFog` | `0x10056c20` | `effects::fog_begin` (TriAPI `Fog`) | done, only with `gl_fog` != 0 |
| `CAM_Think` / `CL_IsThirdPerson` / camera offset | `0x10024c80` / `0x1004fec0` / §7.4 | `view::cam_think` (no-op), `view::is_third_person`, third-person block of `calc_normal` | done, verified in game |
| `HookEvents` | `0x10003240` | `events::init` | all 11 |
| `EV_TS_Fire` / `FireLinked` | `0x1003de40` / `0x1003de10` | `events::fire::{fire, fire_linked}` | done |
| `EV_TS_FireStandard` | `0x1003dfa0` | `fire::fire_standard` | done |
| `EV_TS_FireAkimbo` | `0x1003f090` | `fire::fire_akimbo` | done |
| `EV_TS_FirePump` | `0x10040c20` | `fire::fire_pump` | done |
| `EV_TS_FireBullets` | `0x100450c0` | `fire::fire_bullets`, `fire::pellet_dir` | done (spread unit-tested) |
| fire sound | `0x1003de90` | `fire::fire_sound` | done |
| `EV_TS_EjectShell` | `0x10056890` | `fire::eject_shell` + `effects::track_shell` | done |
| `GetWeaponBody` | `0x10007110` | `fire::weapon_body` | done |
| `GetActiveAnimSet` + info anim tables | `0x1003df60`, builders `0x100047d0..` | `events::info` (generated) | done |
| `UTIL_SharedRandomLong/Float` | `0x1001ae60` / `0x1001aea0` | `events::random` | done, bit-exact vs C reference |
| `TS_PlayTextureSound` | `0x10044bf0` | `impact::texture_sound` | done |
| `TS_DecalGunshot` / `TS_DamageDecal` | `0x10001180` / `0x10001000` | `impact::decal_gunshot` / `damage_decal` | done |
| range table | `0x10007220` | `impact::range_mult` | done |
| `EV_TS_Empty`, `Pump`, `Reload`, `Draw`, `Grenade`, `Knife`, `Katana`, `CloseCombat`, `ColdCock` | §4.1 | `events::misc::*` | done |
| `TS_MeleeMaterialSound` | `0x10045df0` | `misc::melee_material_sound` | done |
| particle system `g_PSys` (`PSys_*`, `Particle_*`, `Quad_*`) | §5.2-5.4 | `effects::psys` | done (see deviations) |
| `PSys_PushRadial` / `PushAlongLine` / `BlastPush` | `0x100524c0` / `0x10052720` / `0x10052c80` | `PSys::push_radial` / `push_along_line` / `blast_push` | done (from the disassembly) |
| `CTS_CL_Emitter::Frame`, `LoadMap`, `LoadEntityLump`, `ProcessEnts`, keyvalue parsers | §5.1-5.3 | `effects::frame`, `effects::emitters` | done, unit-tested, verified in game (ts_frigid snow) |
| wind (`MsgFunc_TSPSys`, `CosineLerp`) | `0x1004bfd0`, `0x10050760` | `effects::on_tspsys`, `psys::Wind` | done, unit-tested |
| `TS_MuzzleFlash`, `TS_MuzzleDlight`, `TS_GunSmoke`, `TS_SpriteDebris` (debris and the muzzle pellet spray), `TS_Sparks`, `TS_Scintille`, `TS_ImpactSmoke`, `TS_SmallPuff`, surface mark `0x10046a70` | §4.4-4.5 | `effects::fx::*` | done (numbers from the disassembly) |
| `FX_ApplyCvar` / `FX_AdaptQuality` (`cl_fx`) | `0x10057d50` / `0x10057a70` | `effects::apply_quality`, `psys::FxTuning` | done |
| `TS_ImpactParticles` | `0x10048f60` | `fx::impact` | done |
| `TS_BloodImpact` (`PTakeDam`) | `0x10048840`, `0x1004bcd0` | `fx::blood`, `effects::on_ptakedam` | done |
| `TS_GlassShatter`, `TS_Explosion` (`TSFrags`) | `0x10047cb0`, `0x100481b0`, `0x1004bd50` | `fx::glass`, `fx::explosion`, `effects::on_tsfrags` | done, verified in game |
| client bullets | `0x10035c20`, `0x10035010`, `0x10035d40` | `fire::ClientBullet`, `fire::client_bullets_frame`, `fx::bullet_trail` | simple version (impact; trail quads with `cl_wavedetail` > 0, as TS) |
| `GetBulletVelocityMult` | `0x10055b30` | `fire::bullet_velocity_mult` | done, unit-tested |
| `HUD_AddEntity` (hemisphere, spectator, anti-peek, item fades) | `0x100447f0` | `effects::add_entity`, `occlusion_visible` | done |
| `HUD_CreateEntities` | `0x1001bce0` | `effects::create_entities` | client bullets only (no ghosts) |
| `HUD_TempEntUpdate` | `0x1001bd00`, `0x100569a0` | `effects::tempents::update` | done (SDK loop, ts-scaled, TS shell sounds) |
| `HUD_StudioEvent` | `0x10044b50` | `effects::studio_event` | done (5002, 5004) |
| `DrawLocalLaser` / `DrawOtherLasers` / flashlights | `0x1003bb00` / `0x1003d600` / `0x1003b250` / `0x1003b750` | `effects::laser` | done (untested: needs server state) |
| `DrawPlayerShadows` | `0x10055760` | `laser::draw_shadows` | done, verified in game |
| `DrawScopeOverlay` | `0x10054510` | `effects::scope` | done (not yet checked in engine) |
| `FX_ApplyCvar` / `FX_AdaptQuality` | `0x10057d50` / `0x10057a70` | `effects::apply_quality` | done |
| view-model framerate x time scale | `StudioDrawModel` `0x10031ae0` | `view::view_model_framerate` (writes `curstate.framerate`) | written, untested (no view model in this branch) |
| `HUD_TxferLocalOverrides` | `0x1001b900` | `prediction::txfer_local_overrides` | done |
| `HUD_ProcessPlayerState` | `0x1001b950` | `prediction::process_player_state` | done |
| `HUD_TxferPredictionData` | `0x1001bb00` | `prediction::txfer_prediction_data` | done |
| `HUD_PostRunCmd` / `HUD_WeaponsPostThink` | `0x100045b0` / `0x10003830` | `prediction::post_run_cmd`, `prediction::gun` | structure + stopgap predictor |

## Event parameters (as handled)

All events arrive with `FEV_NOTHOST`; the shooter's copy comes from local prediction. Where R§4.2
and W§6.1 disagree, the handler accepts both.

| Event | f1 | f2 | i1 | i2 | b1 | b2 | Handler behaviour |
|---|---|---|---|---|---|---|---|
| fire | spread X | spread Y | weapon id | attachments (1 sil, 2 laser, 4 flash, 8 scope) | clip empty / akimbo hand / pump mode | shared-random seed | dispatch on class (4/5 akimbo, 1 pump, else standard) |
| firelinked | spread | spread | id | attachments | hand | seed | both guns, +-5 along right, second seed + 0x21 |
| empty | - | - | - | - | - | - | `weapons/pistol-empty.wav` |
| pump | 0 | 0 | id | attachments | 1 | 0 | anim set[13/14], shell + puff (first person), pump sound (`sound2`) |
| reload | 0 or 8/9/10 | 0 | id | attachments | 0 | 0 | anim set[1] (sawed-off with rounds: [8]) or [phase]; `insert-shell`, `generic_reload`; casings for revolver/sawed-off/contender |
| draw | 0 | 0 | id | attachments | 0 | 0 | in-eye spectating only: set[2] |
| grenade | 0 | 0 | 19/20/21 | 0 | 0 | 0 | in-eye only: M61 set[i1] |
| knife | 0 | 0 | id | hit flag | 0 | 0 | trace 35: sparks + `knife_hit` on hard surfaces, else `knife_miss` |
| katana | 0 | 0 | anim index (15 = back swing) | hit flag | 0 | 0 | trace 50 / -55: sparks + `katana/hit[1]`, else `miss[1]` |
| closecombat | weapon id (block) | 0 | 0 block / 1-3 attack | anim index | 0 | 0 | `player/block.wav` or `player/closecombat.wav` + material sound + puff |
| coldcock | dir.x (unused) | - | id | attachments | 0 | 0 | set[28], trace 30, material sound + puff |

Pitch: `25 + 75*ts + rand(-5,5)` everywhere (see deviations for the fire sound). Fire sound:
`weapons/<sound>` (silenced: `sound2`, volume 0.84-0.92, attenuation x2), attenuation from the
client ammo table (`impact::AMMO_ATTENUATION`, decoded from `FUN_10007480`).

## Deviations from TS 3.0

* **Fire/ricochet/explosion pitch.** TS computes `100 - (int)(ts * -300)` (verified: constant at
  `0x1007d6f4` is -300.0, `0x1003eeed`), i.e. **400** at normal speed. FWGS's `S_StartSound` passes
  any pitch > 1 through unclamped, so copying it would play every gunshot at 4x speed. These use
  the TS curve of every other event sound instead (`events::fire_pitch` = `slow_pitch`).
* **Bob pitch** is added to `cl_viewangles` as a per-frame *delta* (`0.1 * (bob - previous bob)`),
  like the kick and shake steps; TS adds a value it zeroes every frame, which the engine copy-back
  would accumulate. The net view is the same, without drift.
* `V_DriftPitch`, `V_AddIdle` (idle sway; `v_i*` are 0 in TS anyway) and the `cl_vsmoothing`
  origin/angle rings are not ported. Local-player translucency (`cl_thirdtransparency`, crosshair
  on a player) is not ported; the cvar is registered.
* Third-person model z smoothing (rate 16) and the `back == 0` body-cam are not ported.
* **Particle numbers** in `effects::fx` are read from the TS client's disassembly (the decompile
  hides float arguments behind `__ftol`), including every call site's arguments, the `cl_fx`
  tuning globals (`0x1018ca40..0x1018ca60`) and the cooldown slots. Notable corrections: the
  `TS_GunSmoke` "45-55" argument is the animation frame rate (always 5 puffs); the muzzle flash
  spreads `max(3, 9·fxmul)` sprites over `info+0x354` with base radius `info+0x358`, and silenced
  shots draw a dim flash that fades out by 512 units; the shotgun's muzzle spray is
  `TS_SpriteDebris`; glass is always 81 shards on a 9×9 grid; the explosion has no sparks, 24/48
  fireball/smoke particles × the quality scale, and uses the muzzle light. Smoke-like particles
  (life 0, no fade) live for one run of their animation, as in TS. Still to compare visually: the
  sprite frame counts (they set those lifetimes) and the quad width of stretched particles (TS
  scales it by the unnormalised view cross product; the port keeps `radius`).
* Stretch lengths use TS's speed (units per 0.1 s): `stretchspeed` is a TS-unit factor in both
  the map templates and the effects.
* TSFrags type 2 passes an uninitialised direction to its scintille/sparks in TS (`0x1004be0c`);
  the port uses the message's direction. No server sends type 2.
* The explosion sound uses the port's fire pitch curve (see the first bullet), not TS's 400.
* `EV_TS_Pump`'s smoke wisp direction is unclear in the decompile; `v_right` is used, as in the
  silenced shot's second smoke call.
* `PointVisible` traces from the cull origin with `PM_GLASS_IGNORE`, ignoring no entity; TS passes
  the local player's entity index as the physent to ignore.
* Particles: unsorted like TS, oldest replaced at the cap, collision is world-only
  (`PM_WORLD_ONLY`), and particles that spawn inside a brush (emitters right under a sky) don't
  count as hits. `Particle_Think` follows the disassembly: the random drift steers an accumulator
  (`+0x68`) by at most `randomdelta` (3D length) and adds it times the think interval; react moves
  the particle's *origin* sideways by `(128 − d)·|v|·dot/128·T·react` for every solid player within
  128 units (local: predicted velocity, others: velocity between their last two updates); wind is
  ×10 (TS units), physics-gated and skips resting particles; the gust segments chain through
  `SharedRandomFloat/Long` as in TS. Think distances halve when zoomed (FOV < 40).
* `weapons/sshell.wav` (TS shotgun-shell bounce) isn't shipped with TS 3.0; `sshell1/2.wav` are used.
* Fog goes through TriAPI `Fog` (FWGS) instead of raw OpenGL; colours are converted from 0..1 to
  0..255.
* Shadows, laser beams and the scope overlay are TriAPI quads, as in TS. The scope overlay
  (`DrawScopeOverlay` `0x10054510`, `effects/scope.rs`) is TriAPI too, not direct GL: after the
  particles, in first person, alive and not observing, with the eased FOV below 40 and the hardware
  renderer, `sprites/quarterscope.spr` frame 0 is drawn four times (render mode TransTexture, no
  culling, white) as mirrored quads of half-size `max(fov·0.1, 1)·0.5` around `v_origin +
  v_forward·10`. The decomp doc's "frame 2" is the sprite-cache render-mode argument; the call
  passes frame 0 (`0x1005457e`) and the sprite has one frame.
* The spectator refdef is a simplified version (in-eye: target origin + view, chase: 128 units
  behind with a world trace).
* `HUD_AddEntity`'s in-eye hiding uses spectator mode 4 (SDK `OBS_IN_EYE`); the doc notes TS may use 5.

* **Shell limit.** TS keeps the newest 64 shells by storing tempent pointers in a ring and setting
  the evicted shell's `die`. The port tags each shell with a sequence number in
  `curstate.iuser2` (`effects::track_shell`), and the temp-entity loop (`client-dll/src/tempents.rs`)
  kills shells older than the newest 64. That's the same result without keeping pointers to engine
  memory, and it can't write to a tempent the engine already freed.

## Missing pieces

* **Motion-blur ghosts** (`MotionBlur_*`, `TSMBlur`): need the TS studio renderer (renderfx 17 with
  `rendercolor.b = 1`). `TSMBlur` is decoded and recorded only.
* **TS studio renderer** (split bone `Bip01 Dorso`, gait x framerate, corpse/ghost draws):
  `HUD_GetStudioModelInterface` still returns 0.
* **Weapon prediction** is a stopgap (below). Shotgun reload phases, kung fu, grenades, melee
  blocking and akimbo linked shots are not predicted (the server plays those; the shooter only
  sees them once prediction moves to `shared::weapons`).
* Particle `termvelocity` unused (TS doesn't read it either).
* Player translucency, view-angle smoothing, idle sway (see deviations).

## Integration notes

### Weapons package (`shared::weapons`)

* **Shared random:** `events::random` is a private, bit-exact copy of TS's
  `UTIL_SharedRandomLong/Float` (float range, seed = `seed + bits(low) + bits(high)`; reference
  values from a C build in the unit tests). When `shared::weapons` has its port, delete this module
  and re-export theirs; the tests' reference values (`0x3f065d00` for `(1234, 0, 1)`,
  `0xbfb0e2eb` for `(77, -pi, pi)`, `36` for `long(1234, 0, 100)`) should hold for both.
* **Pellet spread** (`fire::pellet_dir`) must match the server's `ShootBulletsPlayer`: pellet `i` is
  1-based, `ang = SR(seed+i, -pi, pi)`, `r = SR(seed+i+1, 0, 1)`, `m = 1 - r^3`,
  `dir + right*cos(ang)*m*spreadX + up*sin(ang)*m*spreadY`, range 8192.
* **Gun prediction:** `prediction/gun.rs` decodes the networked state (`GunState::from_clientdata`:
  `vuser3` = id/firemode/attachments, `vuser1[1..2]` = powerups/active items, `vuser2` =
  counter/reserve/alt grip, `vuser4` = hand/clip/in-reload; `weapon_data[16]` timers incl.
  `fuser4`), decrements the timers (`msec*0.001*max(fuser1,0.01)`, x1.5 with DFR), writes `to`,
  and runs a minimal fire/empty/reload/knife/katana predictor (`predict_fire`, `predict_reload`)
  that plays the events locally with `FEV_NOTHOST` and calls `view::kick` (runfuncs only). Replace
  `predict_fire`/`predict_reload` with the shared state machine; keep `view::kick(view_kick, class,
  silenced, double_rate)` as the client half of `VKickback` (call it once per shot with runfuncs).
  Note `alt_grip` is read from `vuser2[2]` (server doc: slot `+0x2c`).
* **Server events:** the server must `PRECACHE_EVENT(1, "events/<name>.sc")` all 11 in TS order
  (`events::EVENTS`): local playback fails with "event was not precached" otherwise.
* **Client weapon info** not in the TOML tables (class `info+0x348`, anim sets A/B) is in
  `events/info.rs`, generated from the client builders. If the weapons package adds these to
  `weapons.toml`, switch `events::info` over.

### HUD package (`state.rs` fields added; nothing removed)

| Field | Writer | Reader |
|---|---|---|
| `time_scale_target` (existing) | HUD `TSSlowMo` handler (`decode_slowmo` in `protocol::effects`; also play `goslow.wav`) | `view::frame` eases `time_scale` |
| `fov_target` | HUD `SetFOV` handler and `prediction` (`to.client.fov`) | `view::frame` eases `fov` |
| `fov` (existing) | `view::frame` (eased) | HUD (sensitivity, scope), `view` |
| `bullet_time` | render (`TSBTime`) | client bullets |
| `powerups`, `attachments`, `active_items`, `firemode`, `clip`, `reserve_ammo` | prediction (from clientdata) | HUD may read; lasers |
| `active_items[65]` | HUD `ActItems` handler (index = entity number) | `effects::laser::draw_others`, HUD |
| `laser_color[65]` | HUD `PlInfo` handler (key 0, index = entity number) | lasers, HUD `DrawLaserDots` |
| `spectator[32]` | HUD `Spectator` handler | `effects::add_entity` |
| `laser_dots[2]` | `effects::laser::draw_local` (`WorldToScreen` −1..1) | HUD `DrawLaserDots` |
| `laser_range` | `effects::laser::draw_local` (first hit distance ×0.025) | HUD `DrawCrosshair` |
| `items_ready` | prediction (`from` active items, 0 while `m_flNextAttack > 0`) | HUD `DrawLaserDots` |
| `third_person` | `view` | HUD |
| `spec_mode`/`spec_target`/`spec_flags` | `prediction::process_player_state` (g_iUser1..3) | view, events, HUD |
| `dead`, `weapon` (existing) | HUD | view (`cl_thirdpersonswitch`), lasers |

**Messages hooked by render** (in `effects::init` / `view::init`): `TSPSys`, `PTakeDam`,
`TSFrags`, `TSMBlur`, `TSBTime`, `TSFade`. The HUD package hooks the other 58. FWGS keeps the first
hook for a name (`hud::init` runs first), so if the HUD hooks one of these it must forward to
`effects::on_tspsys/on_ptakedam/on_tsfrags/on_tsmblur/on_tsbtime` or `view::on_tsfade`.

### lib.rs (not edited by this package)

* `HUD_UpdateClientData` should call `view::update_client_data(cdata)` and return 1, so the eased
  zoom FOV reaches the engine (TS `CHud::UpdateClientData`). Until then the engine uses its own
  `default_fov`/server fov.
* `HUD_Frame` passes the engine's **frame time** (FWGS `host.frametime`), and `view::frame` treats
  it as such (absolute time comes from `GetClientTime`).

## Developer test commands

Registered by the client for scripted tests (no server support needed): `ts_test_fire <id> [att]
[seed]`, `ts_test_event <name> <i1> <i2> <f1>`, `ts_test_fx slowmo <s>|bullettime <0|1>|blood|glass|
explosion|sparks|wind <speed>|stats`, `ts_test_fade r g b a dur hold`, `ts_test_kick <kick>
<class>`, `ts_test_view`, `ts_test_angles <pitch> <yaw>`.

## Test results

* `cargo build -p ts-client -p ts-server`: OK on the host (x86_64), `i686-unknown-linux-gnu` and
  `i686-pc-windows-gnu` (only the pre-existing stdcall-fixup linker note on the server dll).
* `cargo test`: all pass (client 17, shared 7, goldsrc 3), also on `i686-unknown-linux-gnu`.
  Covered: shared random vs C reference, pellet spread bounds/determinism, bullet velocity mult,
  BSP header/entity-lump parsing, template/emitter keyvalues and clamps, tokenizer robustness, wind
  determinism, quad building/view cone, time-scale/FOV easing, fade blend, view-shake settling,
  texture-name/decal helpers, clientdata gun-state round trip, effects payload round trips.
* **In engine** (FWGS root `~/games/xash_render`, listen server, screenshots inspected):
  * `ts_awaken`: AK/shotgun test fire leaves seeded decals on the wall with debris; explosion
    (fireball, dlight, sparks), glass shards with gravity and collision, slow-motion blue-green
    tint at ts 0.35/0.2, `TSFade` red fade in/hold/out, third-person camera (camtype 0 shoulder, 2
    pulled back to -90 with smoothing), player shadow, recoil kick written into the engine's view
    angles (pitch -8.5, yaw -2.55 for AK class) persisting after the shot.
  * `ts_frigid`: 9 emitters / 1 template / fog read from the BSP lump; snow spawns around the
    emitter boxes and falls (seen in first and third person). Ground collision is inferred from the
    live-particle count only, and the `gl_fog 1` screenshot shows at most a faint haze (TS's fog
    starts at 2048 units on this map), so fog is not confirmed.
* **Not tested in engine:** anything that needs the server to send TS state or events (this
  branch's server doesn't yet): other players' fire events, lasers/flashlights, `PTakeDam` from real
  hits, wind from `ts_wind`, local prediction firing (the server doesn't set `m_iId` 16 /
  `vuser*` yet), view-model muzzle flashes and shells (no view model is set by the server yet),
  dead camera, spectator views. Steam GoldSrc has not been tried (the `cl_viewangles` write-back
  for recoil is confirmed on FWGS only, R§6).
