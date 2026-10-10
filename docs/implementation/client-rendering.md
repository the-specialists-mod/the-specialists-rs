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
| `crates/client/src/studio.rs`, `studio/{model,math}.rs` | the TS studio model renderer (R§2): split torso/legs bones, gait, corpses, motion-blur ghosts; `.mdl` parsing from bytes; quaternion/matrix maths |
| `crates/client/src/effects/mblur.rs` | motion-blur ghost trails (`MotionBlur_*`, R§7.6) |
| `crates/goldsrc/src/ext/studio.rs` | safe wrappers over `engine_studio_api_t` (handles, bone matrices by value, header as bytes) and `CL_TempEntAllocCustom` with callback trampolines |
| `crates/client-dll/src/lib.rs` | `HUD_GetStudioModelInterface` and the two `r_studio_interface_t` callbacks |
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
`studio_event.h`, `shake.h`. `wrapper.h` is untouched. The studio renderer adds a second
`.header_contents("studio_ext.h", ...)` (`com_model.h`, `r_studioint.h`) with
`engine_studio_api_t|r_studio_interface_t|player_info_t|alight_t|cache_user_t` and
`STUDIO_INTERFACE_VERSION|STUDIO_RENDER|STUDIO_EVENTS`; `glpoly2_s` is made opaque (a flexible
array member that would otherwise generate warning-producing helpers). `studio.h` isn't bound: the
renderer reads the model from bytes. Float/flag macros bindgen can't export
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
| `CTSGun::VKickback` (client) | `0x10008230` | `view::kick` | done |
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
| `PSys_PushRadial` / `PushAlongLine` | `0x100524c0` / `0x10052720` | `PSys::push_radial` / `push_along_line` | simplified |
| `CTS_CL_Emitter::Frame`, `LoadMap`, `LoadEntityLump`, `ProcessEnts`, keyvalue parsers | §5.1-5.3 | `effects::frame`, `effects::emitters` | done, unit-tested, verified in game (ts_frigid snow) |
| wind (`MsgFunc_TSPSys`, `CosineLerp`) | `0x1004bfd0`, `0x10050760` | `effects::on_tspsys`, `psys::Wind` | done, unit-tested |
| `TS_MuzzleFlash`, `TS_MuzzleDlight`, `TS_GunSmoke`, pellet spray, `TS_Sparks`, `TS_Scintille`, smoke/debris | §4.4-4.5 | `effects::fx::*` | done (numbers approximated) |
| `TS_ImpactParticles` | `0x10048f60` | `fx::impact` | done |
| `TS_BloodImpact` (`PTakeDam`) | `0x10048840`, `0x1004bcd0` | `fx::blood`, `effects::on_ptakedam` | done |
| `TS_GlassShatter`, `TS_Explosion` (`TSFrags`) | `0x10047cb0`, `0x100481b0`, `0x1004bd50` | `fx::glass`, `fx::explosion`, `effects::on_tsfrags` | done, verified in game |
| client bullets | `0x10035c20`, `0x10035010`, `0x10035d40` | `fire::ClientBullet`, `fire::client_bullets_frame`, `fx::bullet_trail` | simple version (streak + impact) |
| `GetBulletVelocityMult` | `0x10055b30` | `fire::bullet_velocity_mult` | done, unit-tested |
| `HUD_AddEntity` (hemisphere, spectator, anti-peek, item fades) | `0x100447f0` | `effects::add_entity`, `occlusion_visible` | done |
| `HUD_CreateEntities` | `0x1001bce0` | `effects::create_entities` | client bullets, then motion-blur ghosts |
| `HUD_GetStudioModelInterface` | `0x1001f3f0` | `client-dll` + `goldsrc::ext::studio::init` + `studio::init` | done (version 1, 43-slot table copy) |
| `R_StudioDrawModel` / `R_StudioDrawPlayer` | `0x1001f3d0` / `0x1001f3b0` | `studio::draw_model` / `draw_player` | done |
| `StudioDrawModel` (renderfx 17: corpse / ghost) | `0x10031ae0` | `Renderer::draw_model`, `draw_corpse`, `draw_ghost` | done |
| `StudioDrawPlayer` | `0x10032540` | `Renderer::draw_player` | done (SDK flow, `renderamt = 255`, in-eye hide) |
| TS `StudioDrawDeadPlayer` / `StudioDrawGhostPlayer` | `0x10033540` / `0x100337f0` | `draw_dead_player` / `draw_ghost_player` | done |
| `StudioSetupBones` (split at `Bip01 Dorso`/`Bip01 Spine`, torso yaw fix) | `0x10031080` | `Renderer::setup_bones`, `split_torso` | done, unit-tested |
| `StudioEstimateGait` (2-D, x0.75 below 180 u/s) / `StudioProcessGait` (gaitframe x framerate) | `0x10031fa0` / `0x10032250` | `estimate_gait` / `process_gait`, `turn_torso` | done |
| `StudioSetUpTransform`, `EstimateInterpolant`, `EstimateFrame`, `FxTransform`, `GetAnim`, `PlayerBlend` | `0x10030970`, `0x10030c40`, `0x10030f80`, `0x10030e60`, `0x100307e0`, `0x10030890` | `Renderer::*` | done (SDK) |
| `StudioCalcRotations`, `CalcBoneAdj`, `CalcBoneQuaterion`, `CalcBonePosition`, `SlerpBones` | `0x10030ca0`, `0x10030140`, `0x10030380`, `0x10030550`, `0x100306a0` | `sample_pose`, `bone_adj`, `model::Anim`, `slerp_bones` | done, unit-tested |
| `StudioSaveBones` / `StudioMergeBones` / `StudioCalcAttachments` | `0x100317b0` / `0x10031890` / `0x10034560` | `save_bones` / `merge_bones` / `calc_attachments` | done |
| `StudioRenderModel` (glow shell) / `RenderFinal_Hardware` / `_Software` | `0x100345f0` / `0x10034750` / `0x10034690` | `render` / `render_final` | done |
| `AngleQuaternion`, `QuaternionSlerp`, `QuaternionMatrix`, `MatrixAngles`, TS `QuaternionAngles`, `AngleMatrix`, `ConcatTransforms`, `VectorTransform` | `0x1002fbd0`, `0x1002fc70`, `0x1002fdf0`, `0x1002fef0`, `0x1002ffe0`, `0x1002f920`, `0x1002fa90`, `0x1002fa30` | `studio::math` | done, unit-tested |
| `MotionBlur_Reset` / `_Add` / `_Update` / ghost callback | `0x10029660` / `0x10029680` / `0x10029780` / `0x10029710` | `effects::mblur` (`reset`, `add`, `due` + `spawn_ghost`, `ghost_think`) | done, unit-tested |
| `HUD_TempEntUpdate` | `0x1001bd00`, `0x100569a0` | `effects::tempents::update` | done (SDK loop, ts-scaled, TS shell sounds) |
| `HUD_StudioEvent` | `0x10044b50` | `effects::studio_event` | done (5002, 5004) |
| `DrawLocalLaser` / `DrawOtherLasers` / flashlights | `0x1003bb00` / `0x1003d600` / `0x1003b250` / `0x1003b750` | `effects::laser` | done (untested: needs server state) |
| `DrawPlayerShadows` | `0x10055760` | `laser::draw_shadows` | done, verified in game |
| `FX_ApplyCvar` / `FX_AdaptQuality` | `0x10057d50` / `0x10057a70` | `effects::apply_quality` | done |
| view-model framerate x time scale | `StudioDrawModel` `0x10031ae0` | `view::view_model_framerate` (writes `curstate.framerate` once per frame in `V_CalcRefdef`, not in the renderer) | written, untested |
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
* **Particle numbers.** The doc gives sprites, counts, triggers and formulas; per-effect sizes,
  speeds and lifetimes in `effects::fx` are approximations tuned by eye. The `TS_GunSmoke` count
  argument (45-55) is treated as an intensity (one puff per 10).
* Particles: unsorted like TS, oldest replaced at the cap, react (player push) formula inferred,
  collision is world-only (`PM_WORLD_ONLY`), and particles that spawn inside a brush (emitters right
  under a sky) don't count as hits.
* `weapons/sshell.wav` (TS shotgun-shell bounce) isn't shipped with TS 3.0; `sshell1/2.wav` are used.
* Fog goes through TriAPI `Fog` (FWGS) instead of raw OpenGL; colours are converted from 0..1 to
  0..255.
* Shadows/laser beams are TriAPI quads (TS: the same with TriAPI, plus a direct-GL scope overlay).
  The scope overlay (`DrawScopeOverlay`) is not drawn here (HUD package or later).
* The spectator refdef is a simplified version (in-eye: target origin + view, chase: 128 units
  behind with a world trace).
* `HUD_AddEntity`'s in-eye hiding uses spectator mode 4 (SDK `OBS_IN_EYE`); the doc notes TS may use 5.

* **Studio renderer safety checks.** The renderer parses the model from the bytes the engine
  loaded (`studio::model`, bounds-checked), so a corrupt model draws a bind pose instead of reading
  out of bounds. Where TS would index past a table it clamps or skips: gait sequences out of range
  are treated as 0 (TS dropped the SDK's check in `StudioProcessGait`), more than 4 attachments
  are ignored (TS prints "Too many attachments" and calls `exit(-1)`), a bone whose parent isn't
  an earlier bone is treated as a root, `PlayerInfo` is only asked for valid player numbers, and a
  missing sequence-group file reads as the bind pose. `player_info_t` is copied in and written
  back where TS keeps the engine pointer (same result; the engine doesn't touch it mid-draw), and
  `StudioDrawPlayer`'s engine state is a copy with `renderamt`/`gaitsequence` written back.
* **View-model framerate.** TS latches the slowed view-model framerate inside `StudioDrawModel`;
  the port keeps doing it in `view` (once per frame, before the draw). Same value.
* **Torso yaw fix (decomp correction).** R§2.3 reads the asm as `a3[2] = a3[0] + d`. The x87
  stores are to `a3[0]`: `a3[0] += a1[2] - a2[2]` (Ghidra's C agrees). That turns the split bone
  about its own x axis (the Biped spine's long axis) by the yaw between the upper-body and gait
  roots, which is the intended correction, so no typo is involved.
* **Gait x0.75.** The condition R§2.3 marks as inferred is, from the asm, "estimated 2-D speed
  below 180 units/s" (the value left on the x87 stack is `movement / dt`). TS always uses gait
  estimation (`m_fGaitEstimation = 1`), so the velocity variant (where the compared value is the
  movement itself) is ported but unused.
* **Ghost torso.** `StudioDrawGhostPlayer` writes the stored gait yaw into the ghost entity's
  angles, so from the second frame on the torso controllers read 0 offset: the twist only shows on
  the ghost's first frame. That's TS's behaviour, kept.
* **Spectator hide.** `StudioDrawPlayer` hides the player spectated with `g_iUser1 == 5` as TS
  does; the port's in-eye mode is 4, which `HUD_AddEntity` already handles (R§2 open question).
* **Motion blur.** `MotionBlur_Reset` has no caller in the TS image; the port resets the trails on
  `HUD_VidInit`. `TSMBlur` for entity index 32 is ignored (TS's 32-entry table would overflow).
* **Shell limit.** TS keeps the newest 64 shells by storing tempent pointers in a ring and setting
  the evicted shell's `die`. The port tags each shell with a sequence number in
  `curstate.iuser2` (`effects::track_shell`), and the temp-entity loop (`client-dll/src/tempents.rs`)
  kills shells older than the newest 64. That's the same result without keeping pointers to engine
  memory, and it can't write to a tempent the engine already freed.

## Missing pieces

* **Weapon prediction** is a stopgap (below). Shotgun reload phases, kung fu, grenades, melee
  blocking and akimbo linked shots are not predicted (the server plays those; the shooter only
  sees them once prediction moves to `shared::weapons`).
* Whizz volume formula approximated; particle `termvelocity` unused (TS doesn't read it either).
* Scope overlay, player translucency, view-angle smoothing, idle sway (see deviations).

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
| `player_items[32]` | HUD `ActItems` handler (index entindex-1) | `effects::laser::draw_others` |
| `laser_color[32]` | HUD `PlInfo` handler (key 0) | lasers |
| `spectator[32]` | HUD `Spectator` handler | `effects::add_entity` |
| `laser_dots[2]` | `effects::laser::draw_local` (screen x,y) | HUD `DrawLaserDots` |
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
explosion|sparks|wind <speed>|mblur [ent] [level]|stats`, `ts_test_fade r g b a dur hold`, `ts_test_kick <kick>
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
* **Studio renderer and ghosts** (issues #2, #3): unit-tested only (model parsing and the RLE
  animation walk, the maths, the torso split, pose sampling with controllers, ghost trail
  scheduling and fade). Not yet run in an engine: check with `cargo xtask play` / `client` that
  every model still draws (world models, view model, players with `p_` weapons, items), that a
  running player aims the torso while the legs follow the movement, corpses, and the ghosts behind
  players during slow motion. Ghosts without a server: `cl_motionblur 2`, third person,
  `ts_test_fx slowmo 0.3`, then `ts_test_fx mblur` (the local player: 10 ghosts at `cl_fx 1`, 15 at `cl_fx 2`); the
  server sends `TSMBlur` on power-up use, stunts and kills.
* **Not tested in engine:** anything that needs the server to send TS state or events (this
  branch's server doesn't yet): other players' fire events, lasers/flashlights, `PTakeDam` from real
  hits, wind from `ts_wind`, local prediction firing (the server doesn't set `m_iId` 16 /
  `vuser*` yet), view-model muzzle flashes and shells (no view model is set by the server yet),
  dead camera, spectator views. Steam GoldSrc has not been tried (the `cl_viewangles` write-back
  for recoil is confirmed on FWGS only, R§6).
