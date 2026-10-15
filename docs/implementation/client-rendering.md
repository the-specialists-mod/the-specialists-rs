# Client: view, events, effects, prediction (render package)

Port of the TS 3.0 `client.dll` view, weapon events, effects and prediction glue. Source of truth:
decomp `docs/client/rendering-effects-prediction.md` (cited as **R§n**), `docs/server/weapons.md`
(**W§n**), `docs/server/architecture.md` §7, and the decompiled client C / `objdump` of
`client.dll` where the doc was vague (noted inline in the code).

| File | Contents |
|---|---|
| `crates/client/src/view.rs` | `V_CalcRefdef`, pitch drift, local-player translucency, bob, stair smoothing, stunt roll, recoil + view shake, idle sway, third-person camera (body cam, model z smoothing), `cl_vsmoothing`, FOV and time-scale easing, slow-mo tint + `TSFade` |
| `crates/client/src/events.rs`, `events/{fire,impact,misc,info,random}.rs` | the 11 event hooks and handlers, bullets, shells, decals, texture sounds, client weapon info (classes, anim sets), shared random |
| `crates/client/src/effects.rs`, `effects/{psys,fx,emitters,laser,tempents}.rs` | sprite particle system, weapon/impact effects, BSP-lump emitters, wind, lasers/flashlights/shadows, `HUD_AddEntity`, `HUD_TempEntUpdate`, effect messages |
| `crates/client/src/prediction.rs`, `prediction/gun.rs` | Txfer callbacks, `HUD_PostRunCmd`, local gun prediction (the shared `Gun` state machine) |
| `crates/client/src/studio.rs`, `studio/{model,math}.rs` | the TS studio model renderer (R§2): split torso/legs bones, gait, corpses, motion-blur ghosts; `.mdl` parsing from bytes; quaternion/matrix maths |
| `crates/client/src/effects/mblur.rs` | motion-blur ghost trails (`MotionBlur_*`, R§7.6) |
| `crates/goldsrc/src/ext/studio.rs` | safe wrappers over `engine_studio_api_t` (handles, bone matrices by value, header as bytes) and `CL_TempEntAllocCustom` with callback trampolines |
| `crates/client-dll/src/lib.rs` | `HUD_GetStudioModelInterface` and the two `r_studio_interface_t` callbacks |
| `crates/client/src/state.rs` | fields **added** (see "Integration") |
| `crates/goldsrc/src/ext/render.rs` | `ev!`/`efx!`/`tri!` macros (goldsrc-only; they panic with the table name if `ClEngine::event_api`/`efx_api`/`tri_api` is null), engine wrappers (`level_name` and `sprite_model`, the one `pfnGetLevelName`/`GetSpritePointer` wrappers), `vec3` maths (cvars are `goldsrc::client::Cvar`); `with_solid_players`, the one `EV_SetUpPlayerPrediction` + `EV_PushPMStates` + `EV_SetSolidPlayers` ... `EV_PopPMStates` wrapper (SDK order), and `player_trace`, the one `EV_SetTraceHull` + `EV_PlayerTrace` wrapper. Entities by index, `GetMaxClients`, `GetClientTime`, `SPR_Load` and `EV_PlaySound` go through the `ClEngine` methods in `ext/hud.rs` |
| `crates/goldsrc/src/ext/tri.rs` | safe triangle-API wrappers (`render_mode`, `begin`, `vertex3fv`, `sprite_texture`, ...) for the lasers, particles, scope, client bullets and the studio glow shell; the one wrapper per TriAPI call |
| `crates/goldsrc/src/ext/gl.rs` | raw OpenGL (`glEnable`, `glDisable`, `glFogi/f/fv`, `glHint`) for TS's own `opengl32.dll` calls (fog), looked up at run time in the GL library the renderer loaded: `opengl32.dll` on Windows (Steam `hw.dll`, FWGS under Windows or Wine), `SDL_GL_GetProcAddress` from the engine's SDL2 on Linux (else an already loaded `libGL.so.1`/`libOpenGL.so.0`). Every call is a no-op unless the engine draws with desktop GL: FWGS's `r_refdll_loaded` is `gl`, or (no such cvar: Steam GoldSrc) `IEngineStudio.IsHardware()` is 1 |
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
| `InterpolateSlowMotion` | `0x10055ae0` | `hud::approach_timescale`, TS's only time-scale easer, from `HUD_Frame` (`hud::frame`, after the emitters, by the unclamped frame time) | done, unit-tested |
| `CHud::Think` zoom (from `CHud::UpdateClientData` `0x10024b00`), `UTIL_ApproachFOV`, `HUD_GetFOV` | `0x10049760`, `0x10049620`, `0x100209f0` | `hud::think_fov` from `view::update_client_data` (`HUD_UpdateClientData`), `view::ease_fov` (integer steps of `max(1, (\|d\|/2)·ft·50)` with the last refdef's frame time `0x10191478`), `hud::get_fov` (target `state.fov_target` = `0x100df1a4`, demo record type 0) | done, unit-tested |
| `V_CalcRefdef` | `0x10069b90` | `view::calc_refdef` (dispatch in `view::refdef_path`) | done, dispatch unit-tested; the spectator view runs for `pparams->spectator` or an observer mode, a ruleset may narrow that (`ClientRules::spectator_refdef`, TS 2.1: HLTV only); while paused (outside intermission and spectating) no view is computed, as in TS, so the engine keeps last frame's view and the bob, kick, shake and stair state hold |
| `V_CalcNormalRefdef` | `0x10067890` | `view::calc_normal` | done. After the angle ring TS writes `pparams->cl_viewangles` three more times (`0x10067c94..0x10067d31`): the `V_ApplyKick` step, the `ViewShake::Update` step (scaled by the unclamped `g_flTimeScale`) and `0.1f·bob pitch`, in that order and with TS's x87 rounding; there is no engine call, the engine copies `cl_viewangles` back afterwards (FWGS `V_GetRefParams`), so all three persist. Then `viewangles = cl_viewangles`, `V_CalcViewRoll` and `V_AddIdle` (`0x10067eba`, at `v_idlescale`). The camera the client `PM_SpectatorMove` left (`spectator::view_overrides`) replaces `simorg` (`0x101914f0`, `0x100679aa`) and `cl_viewangles` (`0x1019149c`, `0x10067a4b`); `cl_viewangles` goes into its own ring (`0x10191908`), lagged by `cl_vsmoothing` while the angle flag is set, and the origin ring also runs on the origin flag (`0x1006878f`); smoothed results become the saved spectator camera (`0x10067c73`, `0x10068974`). In normal play `PM_PlayerMove` clears all of it every move, so this only shows when movement spectates and the HUD doesn't (yet), or after an HLTV director reset. This frame's `forward/right/up` come from `cl_viewangles` (`0x10067ebf`). The gun is placed at `vieworg` (`0x10068547`: the finished first-person eye with smoothed view height and stairs, shake, the 1/32 nudge and water, so it follows the stunt `NO_VIEW` descent; the camera in third person, where the engine doesn't draw it), then shaken, then the bob offset and fov push are added. In third person the view angles become `camAngles` (`CL_CameraOffset`'s pitch and yaw, roll 0; `0x100689ef`) and the model's pitch is `-camPitch/3`; the effects/HUD camera (`view::camera`) keeps `v_angles` (`0x101914d0`), the angles before that override. A view entity above `maxclients` (`SET_VIEW`, `trigger_camera`) replaces the origin and angles (`0x10068a73`) |
| `V_CalcSpectatorRefdef` | `0x10068f70` | `view::calc_spectator` → `spectator::calc_refdef` | done: `simorg` + mouse angles unless the client `PM_SpectatorMove` overrode the camera (`0x101914f0`/`0x1019149c`, saved in `0x101913e4`/`0x10190b6c`), `cl_vsmoothing` rings on the flags it set (the angle ring as TS: engine `AngleVectors` of both samples, `0x10029d70`'s angle `acos(a·b) / (|a||b|)` with the division after the arc cosine and no clamp, and a NaN angle passing the `< 22.5` test; ring entries normalised in place), in-eye (mode 5) `V_GetInEyePos` `0x10068e20` and the target's gun as view model; HLTV inset pass (`nextView`, `spectator::inset_view`) ported, unreachable as in TS. A ruleset may replace the mode passes after the smoothing (`ClientRules::hltv_calc_refdef`, TS 2.1's main and inset modes, `ts21.md`). `SetModes`'s locked-view target test is `IsActivePlayer` (`0x100236d0`: a player, solid, not the local one, with a player-info name; issue #182) |
| `PM_SpectatorMove` (client-modified), `PM_PlayerMove` flag reset | `0x1002c790`, `0x1002e330` | `spectator::{spectator_move, begin_player_move}` via `shared::pm::player_move_with` from `HUD_PlayerMove`; `GetChaseOrigin` `0x1002c6c0` is `shared::pm::chase_origin` | done: smoothing flags, camera overrides, last camera kept when the target isn't a physent, director locked view (mode 4), director-reset camera pin; unit-tested angle maths. The angle helpers are client.dll's in its 53-bit x87 arithmetic (`f64`, see `shared::pm::fpu`): `VectorAngles` (`0x10029e90`, `goldsrc::ext::render::vec3::vector_angles`: `fpatan` unrounded into `× 57.29577951308232`, a NaN component counts as zero), `NormalizeAngles` (`0x10029cb0`, `vec3::norm_angle`: one step `a > 180 ? a − 360 : a < −180 ? a + 360 : a`, no modulo, −180 kept) and `InterpolateAngles` (`0x10029cf0`, `spectator::interpolate_angles`: normalises its inputs in place, difference kept unrounded, a NaN takes `+ 360`). The locked view's line test (`0x1002ca94`) treats a NaN fraction as clear |
| `CHudSpectator::DrawOverview` (+ layer, entities, expiry), `AddOverviewEntity`, `DeathMessage` | `0x10024970` (`0x10023c90`, `0x10024160`, `0x100249b0`), `0x100249f0`, `0x10024a60` | `spectator::overview::{draw, add_entity, death}` from `HUD_DrawNormalTriangles`, `effects::add_entity` and the `DeathMsg` handler | done, HLTV spectators only (as TS); tiling, file parser and entity list unit-tested |
| `V_CalcIntermissionRefdef` | `0x100676e0` | `view::calc_intermission` | done |
| `V_DriftPitch` / `V_StartPitchDrift` (`centerview`) / `V_StopPitchDrift` | `0x10067320` / `0x10067290` / `0x100672f0` | `view::Drift` (`centerview` registered in `view::init`, as `V_Init` `0x10069e90` does), `view::stop_pitch_drift` (called from `input` like `CL_AdjustAngles` `0x10026330` and `IN_MouseMove` `0x100274d0`, and from the joystick like `0x10027e40`) | done, unit-tested |
| `V_AddIdle` | `0x10067550` | `view::add_idle` | done: intermission at scale 1 (`0x10067734`), the normal view at `v_idlescale` (`0x10191414`), which `CHud::UpdateClientData` sets from `m_iConcuss` (`0x10024b63`). Only the TFC `Concuss` message (`0x10021a40`) sets that and no TS server sends it, so the port's `update_client_data` writes 0 and the normal-view call adds nothing, as in TS. The `v_i*` levels and cycles are the SDK's unregistered statics (`0x10097244..0x100972a4`: roll 0.5/0.1, pitch 1/0.3, yaw 2/0.3) |
| local-player translucency, `Approach` | start of `0x10067890`, `0x10057e90` | `view::local_amt_step` (crosshair flag from the HUD's `DrawCrosshairID`, step size from `hud::time_delta` = `gHUD.m_flTimeDelta`) | done, unit-tested |
| `cl_vsmoothing` origin ring (SDK `ViewInterp`) | step 14 of `0x10067890` | `view::Ring` | done, unit-tested |
| `V_CalcBob` / `V_SmoothBobZ` | `0x10066e70` / `0x10066de0` | `view::Bob` | done, unit-tested; see "Bob" below |
| `V_CalcGunAngle` | `0x100674a0` | `view::calc_gun_angle` (also the in-eye gun) | done, unit-tested: `(bob − pitch, yaw, roll)` of the view angles, current and latched angles set before the 0.9 shake; no `crosshairangle`; the `ev_punchangle·1.25` terms are always 0 |
| `cl_vsmoothing` angle ring (normal view) | `0x10067a7a..0x10067c90` | `view::Ring::interpolate_angles` + `spectator::interpolate_angles` (`0x10029cf0`) | done, unit-tested. The change test is client.dll's unrounded `Length` (`0x10058390`) against 0 (a NaN counts as no change); the fraction stays a `double` until the `float` argument (`spectator::angle_ring_frac`); `InterpolateAngles` normalises both ring entries in place, as in TS. A negative `cl_vsmoothing` is reset to 0 and that frame lags by the new value, 0 (TS reads the cvar again, `0x10067ba5`) |
| `V_SmoothStairs` + view-height smoothing | `0x100677d0` | `view::Stairs` in `view::EyeZ` (called from `calc_normal`, and again for the body cam, through the ruleset's `eye_origin`/`body_cam_z`) | done, unit-tested; behaviour as TS, see "Stair smoothing" below |
| `V_CalcViewRoll` / `V_CalcRoll` | `0x100675b0` / `0x100671f0` | `view::calc_view_roll`, `view::calc_roll` | done, unit-tested: on the view entity (`GetEntityByIndex(viewentity)`, nothing if NULL); its **`prevstate`** `iuser1` (> 0: roll 0) and `iuser4` pick the target (ground 0/15, dive 35/7.5, flips -10/5, other air -5.5/15); the eased roll follows `V_CalcRoll(ent->angles, simvel, target, 10)`, so it scales with sideways speed and is 0 on the ground; dead with a view height it is then forced to 80. The third-person stunt camera also reads `prevstate.iuser4` (`0x100681c0`) |
| `V_ApplyKick` | `0x10057ed0` | `view::apply_kick`, step added to `cl_viewangles` | done, unit-tested, verified in game |
| `ViewShake::Start` / `Update` | `0x10057fa0` / `0x10058000` | `view::Shake` | done, unit-tested |
| `CTSGun::VKickback` (client) | `0x10008230` | `view::kick` (prediction: `view::kick_angles`) | done; every constant read from `.rdata` (`0x1007a884..0x1007a8ac`) and identical to the server's (`0x80414`, `.rodata` `0xf7f04..0xf7f20`) |
| client melee view shakes: `ColdCock`, `KatanaFire`, `KnifeFire`, `CCombatAttack` | `ViewShake::Start(amp, 0.2, 10)` at `0x10008dc3`, `0x1000a16f`, `0x1000a340`, `0x1000b5e0` | `GunHost::view_shake` -> `view::melee_shake` (runfuncs only) | done: stock damage ×0.3; knife damage (×2/3 stunting) ×0.15; katana's own strength ×0.2 (stunt 35, backward −75, airborne `clamp(speed·0.15384616, 35, 55)`, crouched 65, ground 45); kung fu base damage ×0.35 |
| screen fade + `TSFade` | end of `0x10069b90`, `0x1004c1e0` | `view::write_screen_fade`, `view::fade_color`, `view::on_tsfade` | done, verified in game; unit-tested. The engine's fade is read back (`pfnGetScreenFade`) and only the colour and flags (`FFADE_OUT\|FFADE_STAYOUT`) are replaced; the `TSFade` ramps are TS's unclamped x87 formula (`1 - ((e - hold) - half)/half` on the way out), bytes by `__ftol`'s low byte, at `pparams->time` |
| `V_SetFog` | `0x10056c20` | `effects::fog_begin` / `fog_params` (raw GL through `goldsrc::ext::gl`), `fog_end` (`glDisable(GL_FOG)` at the end of `TS_DrawEffects`, `0x10056e5e`) | done, unit-tested: `glFogi(GL_FOG_MODE, GL_LINEAR)`, `glFogfv(GL_FOG_COLOR, rgb + alpha 1)`, `glFogf(GL_FOG_DENSITY, 1)`, `glHint(GL_FOG_HINT, GL_NICEST)`, start, end, `glEnable(GL_FOG)`. Single player with `gl_fog 2` uses `gl_fog_red/green/blue` and `gl_fog_start/end` (no renderer check); otherwise the map's `ts_fog` while `gl_fog` != 0 and `IsHardware()` is exactly 1 (`0x10056d20`) |
| `CAM_Init` / `CAM_Think` / `MoveToward` / `CL_IsThirdPerson` / camera offset | `0x10025370` / `0x10024c80` / `0x10024ba0` / `0x1004fec0` / §7.4 | `view::cam_init` (from `input::init`, as `InitInput` calls it), `view::cam_think` (`cam_command`, `cam_ofs` from `cam_snapto`/`cam_ideal*`, `move_toward`, unit-tested; in third person only, so `cam_ofs`, a zeroed static until the toggle seeds it, keeps its stale value in first person, as in TS; the camera is placed along its pitch and yaw with roll 0, `0x10067f53`), `view::camera_offset` (the `CL_CameraOffset` export, `0x10025720`: `cam_ofs`), `view::is_third_person` (`cam_thirdperson` only; TS has no separate death-cam flag, the death cam is the `TSHealth` 0 toggle), third-person block of `calc_normal` (`cam_base`, `cam_smooth`, `body_cam`, `smooth_model_z`, `model_pitch`; collision trace through the engine's `PM_TraceLine`, `render::pm_trace_line`) | done, checked against the assembly; camera verified in game before the offset smoothing was rewritten to TS's form; body cam, model z smoothing and the `camAngles` view not yet verified in game |
| camera controls: `+/-campitchup`, `+/-campitchdown`, `+/-camyawleft`, `+/-camyawright`, `+/-camin`, `+/-camout` (kbuttons `0x100df338`/`370`/`2e8`/`348`/`380`/`328`), `+/-cammousemove`, `+/-camdistance`, `snapto`; `CAM_Think`'s adjustment block | `0x10025260..0x10025310`, `0x100255d0`/`0x10025670`, `0x10025680`/`0x10025700`, `0x10025340`; `0x10024cdc..0x1002515d` | `camera::register_commands` (from `view::cam_init`, in `CAM_Init`'s order), `camera::adjust_ideal` (`key_step`: ±2.5° pitch/yaw, ±1 distance, `+camin` under 30 resets to 0/0/30, unbounded; `mouse_step`: half a degree, or one unit of distance in `+camdistance`, per two pixels of cursor offset, bounded by `c_min/max*`; both unit-tested). While a mouse mode holds the mouse (`iMouseInUse` `0x100df394`), `input` skips mouse look (`IN_Move` `0x100282e0`), `IN_Accumulate` (`0x100278b0`) and the mouse buttons (`IN_MouseEvent` `0x100273d0`) | done (see deviations); not exercised in game |
| `thirdperson` toggle | `0x1004fe00` | `view::toggle_third_person` (also `cam_command 1`, and the HUD's switches from inside their handlers: the `TSHealth` death cam, `cl_thirdpersonswitch` in `WeaponInfo`, `cl_thirdperson` and observer in `Spectator`; nothing else reads those two cvars): during demo playback switches only when `cam_thirdperson` differs from the recorded state; otherwise refused for an observer in first person; flips `cam_thirdperson`, seeds `cam_ofs` = (view pitch, view yaw, 30) when switching on, sets `cam_command` 0, and while recording writes demo record type 2 | done; not exercised in game |
| `Demo_WriteBuffer` / `Demo_ReadBuffer` | `0x1001b860` / `0x1001b8c0` | `demo::write_buffer` (`ClEngine::demo_write_buffer`), `demo::read_buffer` (from the `Demo_ReadBuffer` export): `int` type + 4 bytes. Type 0: the zoom target (`float`), written by `hud::get_fov` on every `CHud::Think` while recording (`0x10020a11`) and replacing the target during playback (`0x10020a25`). Type 1: the alternate grip (`int` `vuser2[2]`), written by `prediction::gun::update_alt_grip` when it changes while the `from` weapon is the TS gun (`0x10003bf9`) and applied by `prediction::alt_grip` (`GetActiveAnimSet`, `0x1003df6c`) during playback; `CHudTSWeapons::VidInit` (`0x1004cb50`) clears the grip. Type 2: `cam_thirdperson`. Unknown types print TS's "Unknown demo buffer type, skipping." | done, unit-tested; not exercised in a recorded demo |
| `HookEvents` | `0x10003240` | `events::init` | all 11 |
| `EV_TS_Fire` / `FireLinked` | `0x1003de40` / `0x1003de10` | `events::fire::{fire, fire_linked}` | done |
| `EV_TS_FireStandard` | `0x1003dfa0` | `fire::fire_standard` | done |
| `EV_TS_FireAkimbo` | `0x1003f090` | `fire::fire_akimbo` | done |
| `EV_TS_FirePump` | `0x10040c20` | `fire::fire_pump` | done; its own shell speeds (`RandomLong(20, 40)` sideways and forward, `(0, 20)` up, `0x10040e43`/`0x10040e4d`, the ruleset's `pump_shell_speeds`) and the shotgun shell's sound 2 for every gun (unit-tested) |
| `EV_TS_FireBullets` | `0x100450c0` | `fire::fire_bullets`, `fire::pellet_dir`, `fire::range_index` | done (spread and range index unit-tested). Client bullets instead of traces while the server's `realbullet` (`SrvSett` 0, `0x1018d16c`) is non-zero or ts < 0.99 (`0x10045330`); the range index is `__ftol(dist / (unit·10) · 10)` (`0x10045b41`). Penetration (`0x10045bde..`) uses TS's global remaining damage `DAT_1018a164` (`fire::REMAINING`): only a pellet that hits writes it and nothing resets it, so an event whose pellets all miss penetrates with an earlier event's damage. It starts from the end of the last pellet's trace, hit or miss, along the last pellet's direction. Hit tests are `fraction == 1.0` with `fcomp` (a NaN is a miss), and `remaining > 5` fails on a NaN |
| fire sound | `0x1003de90` | `fire::fire_sound`, `fire::fire_sample` | done, unit-tested: the silenced flag alone picks `sound2` (`info+0x306`) or `sound` (`info+0x2c6`), even an empty one (`0x1003de98`/`0x1003dee5`) |
| `EV_TS_EjectShell` | `0x10056890` | `fire::eject_shell` + `effects::track_shell` | done |
| `GetWeaponBody` | `0x10007110` | `fire::weapon_body` | done |
| `GetActiveAnimSet` + info anim tables | `0x1003df60`, builders `0x100047d0..` | `events::info` (generated; all 37 sets re-extracted from the builders' stores, kung fu's set A was missing) | done |
| `UTIL_SharedRandomLong/Float` | `0x1001ae60` / `0x1001aea0` | `events::random` | done, bit-exact vs C reference |
| `TS_PlayTextureSound` | `0x10044bf0` | `impact::texture_sound` | done, unit-tested. TS never reads its first argument, so the port doesn't take one. Random calls in TS's order: `RandomLong(0, 100)` right after the sky test (`0x10044c2a`), the ricochet's `RandomLong(0, 0x7fff)` (< 0x3fff plays `debris/ric{roll % 5 + 1}`, no second roll), then, when the first roll is > 33 (`0x10044ff8`), the pitch's `RandomLong(-5, 5)` and the sample's. The material sound is attenuation 0.97 (`0x10045034`); the slosh sounds are in TS's order 1, 3, 2, 4, and every type other than D/G/M/V/P/Y/S/W (including T and no texture) uses the seven `debris/cement*.wav` |
| `TS_DecalGunshot` / `TS_DamageDecal` | `0x10001180` / `0x10001000` | `impact::decal_gunshot` / `damage_decal` | done |
| range table | `0x10007220` | `impact::range_mult` | done |
| `EV_TS_Empty`, `Pump`, `Reload`, `Draw`, `Grenade`, `Knife`, `Katana`, `CloseCombat`, `ColdCock` | §4.1 | `events::misc::*` | done |
| `TS_MeleeMaterialSound` | `0x10045df0` | `misc::melee_material_sound` | done |
| `CachePlayerAttachments` / getter | `0x10056e70` / `0x10056be0` | `effects::{cache_player_attachments, player_attachment}` | done (filled in `draw_transparent`, read by every weapon event) |
| particle system `g_PSys` (`PSys_*`, `Particle_*`, `Quad_*`) | §5.2-5.4 | `effects::psys` | done. TS's slot pool: `PSys_Init` (`0x100507e0`) allocates `max` (256..4096) slots on the first `TS_DrawEffects` after `HUD_VidInit`, a later `cl_fx` change keeps the size until then; `PSys_Add` (`0x10050b60`) takes the first free slot and drops the particle when there is none (or before the pool exists, or for a TransColor sprite slot on a software renderer); `PSys_UpdateDraw` (`0x100520d0`) walks the slots in order up to the highest live one, moving, colliding and drawing each particle in turn (`PSys::update_draw`) |
| `PSys_PushRadial` / `PushAlongLine` / `BlastPush` | `0x100524c0` / `0x10052720` / `0x10052c80` | `PSys::push_radial` / `push_along_line` / `blast_push` | done (from the disassembly). They walk the push list the last particle pass built (`+0xc14`: live slots with `react > 0`), skipping slots that have died since |
| `CTS_CL_Emitter::Frame`, `LoadMap`, `LoadEntityLump`, `ProcessEnts`, keyvalue parsers | §5.1-5.3 | `effects::frame`, `effects::emitters` | done, unit-tested, verified in game (ts_frigid snow). Emission runs on the emitters' own clock (`0x10189478`): `HUD_Frame`'s frame times, unclamped, summed since the map loaded. Nothing is emitted, and the clock doesn't run, until the first `TS_DrawEffects` has allocated the particle pool (`0x1003abef` tests `g_PSys+8`, `0x1018bdf0`); the pool is never freed, so emitters don't wait again after a level change. The count is `RandomLong(ppelow, ppehigh)` on the raw keyvalues (`0x1003ac54`; the engine returns `ppelow` when it is above `ppehigh`), divided by the integer `cl_fx` divisor and rounded up with `idiv` + `inc` (`0x1003ae29`). Particles of an emitter with `cameradensity` and `cameraradius` above 0.01 and the camera inside spawn at the camera ± `cameraradius × (2 − min(cameradensity, 1))` on x and y, then 1.5 × the local player's `curstate.velocity` x/y (`GetLocalPlayer`, `0x1003b0a0..0x1003b0cb`, 1.5 at `0x1007a860`) is added so rain and snow lead a moving player (`emitters::camera_xy`). The camera is inside an emitter when it is strictly between `origin + variancelow` and `origin + variancehigh` on x and y, not normalised (`0x1003ac75..0x1003acce`). Outside, the camera test is horizontal (`0x1003ace9..0x1003ae25`): the direction to the box centre has z = 0 and the XY distance is floored at 1e-4; the emitter is skipped for that frame without rescheduling when forward · direction < 0.1 (`0x1003ad96`), otherwise the LOD factor is `clamp(distance / |high − low|, 1, 10)`, with the 3D box size unfloored (a zero-size box gives 10). Only a random particle count of exactly 0 is rejected (`0x1003ac6d`); a negative one still plays the emit sound (at the world origin, the zeroed scratch particle's) and reschedules, and with a divisor above 1 a small negative count rounds up to one particle. The template sprite is looked up after the count and the camera test (`0x1003ae5f`), so a missing sprite doesn't change the random sequence; it prints `Sprite '<name>' is not a valid sprite.` and the emitter is retried next frame without rescheduling (`0x1003ae69..0x1003ae78`) |
| wind (`MsgFunc_TSPSys`, `CosineLerp`) | `0x1004bfd0`, `0x10050760` | `effects::on_tspsys`, `psys::Wind` | done, unit-tested |
| `TS_MuzzleFlash`, `TS_MuzzleDlight`, `TS_GunSmoke`, `TS_SpriteDebris` (debris and the muzzle pellet spray), `TS_Sparks`, `TS_Scintille`, `TS_ImpactSmoke`, `TS_SmallPuff`, surface mark `0x10046a70` | §4.4-4.5 | `effects::fx::*` | done (numbers from the disassembly) |
| `FX_ApplyCvar` / `FX_AdaptQuality` (`cl_fx`) | `0x10057d50` / `0x10057a70` | `effects::apply_cvar` / `effects::adapt_quality`, `psys::FxTuning` | done; both run from `effects::draw_transparent` (`TS_DrawEffects` `0x10056dd0`): `FX_ApplyCvar` first, `FX_AdaptQuality` (`cl_fx` exactly 1) after the particle pass, so its result takes effect next frame. `HUD_VidInit` clears "`TS_DrawEffects` has run" (`0x1018d16a`, cleared at `0x1006600f` by the TS panel's reset); the next `TS_DrawEffects` then re-runs `FX_ApplyCvar` whatever `cl_fx` is, `PSys_Startup` (`0x10056bc0` -> `PSys_Init` `0x100507e0`: the particles, sprite cache and cooldowns are cleared) and the client bullet reset (`0x10034eb0`) |
| `PSys_LoadSprite` | `0x100509d0` | `PSys::sprite` | done. 64 slots keyed by name (case-insensitive) and render mode; the particle pass draws with the slot's mode. Once all 64 are taken a new sprite is silently "no sprite". The laser, flashlight, shadow, scope and client-bullet sprites share the cache, looked up each time TS draws them. A sprite that fails to load takes no slot, so every lookup of it calls `SPR_Load` again and prints `WARNING! cannot found <name>`, as in TS. Lookups are once per effect or per emission, never per particle; a map emitter with a missing sprite is retried every frame (see the emitter row), so like TS it prints both warnings every frame |
| `TS_ImpactParticles` | `0x10048f60` | `fx::impact` | done |
| `TS_BloodImpact` (`PTakeDam`) | `0x10048840`, `0x1004bcd0` | `fx::blood`, `effects::on_ptakedam` | done |
| `TS_GlassShatter`, `TS_Explosion` (`TSFrags`) | `0x10047cb0`, `0x100481b0`, `0x1004bd50` | `fx::glass`, `fx::explosion`, `effects::on_tsfrags` | done, verified in game |
| client bullets (`0x100eb678`, 256 slots) | `ClientBullets_Add` `0x10035c20`, `_Frame` `0x10035b50` (from `0x10034e60`), `_Impact` `0x10035d40`, `_Update` `0x10035010`, `_Clear` `0x10035d10`, `_Reset` `0x10034f60` | `fire::ClientBullets`, `fire::client_bullets_frame`, `fx::bullet_trail` | done, unit-tested (slots, throttle, sizes, quads). See "Client bullets" below |
| `GetBulletVelocityMult` | `0x10055b30` | `fire::bullet_velocity_mult` | done, unit-tested |
| `HUD_AddEntity` (hemisphere, spectator, anti-peek, item fades) | `0x100447f0` | `effects::add_entity`, `occlusion_visible` | done |
| `HUD_CreateEntities` | `0x1001bce0` | `effects::create_entities` | motion-blur ghosts. TS's `ClientBullets_AddEntities` (`0x10034e80`) runs first but adds nothing: its bullet models need `0x1d34d`, which only `0x10034f90` writes, always 0 |
| `HUD_GetStudioModelInterface` | `0x1001f3f0` | `client-dll` + `goldsrc::ext::studio::init` + `studio::init` | done (version 1, 43-slot table copy) |
| `R_StudioDrawModel` / `R_StudioDrawPlayer` | `0x1001f3d0` / `0x1001f3b0` | `studio::draw_model` / `draw_player` | done |
| `StudioDrawModel` (view-model slow motion; renderfx 17: corpse / ghost) | `0x10031ae0` | `Renderer::draw_model`, `latch_view_model_rate`, `draw_corpse`, `draw_ghost` | done (audited against the asm) |
| `StudioDrawPlayer` | `0x10032540` | `Renderer::draw_player` | done (SDK flow, `renderamt = 255`, in-eye hide, colours clamped 0..360, `p_` weapon merged) |
| TS `StudioDrawDeadPlayer` / `StudioDrawGhostPlayer` | `0x10033540` / `0x100337f0` | `draw_dead_player` / `draw_ghost_player` | done (corpse: no gait, events, no weapon; ghost: stored gait yaw turns the torso, no gait advance, no events, no `renderframe`, weapon drawn) |
| `StudioSetupBones` (split at `Bip01 Dorso`/`Bip01 Spine`, torso yaw fix) | `0x10031080` | `Renderer::setup_bones`, `split_torso` | done, unit-tested |
| `StudioEstimateGait` (2-D, x0.75 below 180 u/s) / `StudioProcessGait` (gaitframe x framerate) | `0x10031fa0` / `0x10032250` | `estimate_gait` / `process_gait`, `turn_torso` | done (wrap `x - (int)(x * (1/360)) * 360` and torso controller `(yaw * 0.25 + 30) * 4.25` as in the asm) |
| `StudioSetUpTransform`, `EstimateInterpolant`, `EstimateFrame`, `FxTransform`, `GetAnim`, `PlayerBlend` | `0x10030970`, `0x10030c40`, `0x10030f80`, `0x10030e60`, `0x100307e0`, `0x10030890` | `Renderer::*` | done (SDK) |
| `StudioCalcRotations`, `CalcBoneAdj`, `CalcBoneQuaterion`, `CalcBonePosition`, `SlerpBones` | `0x10030ca0`, `0x10030140`, `0x10030380`, `0x10030550`, `0x100306a0` | `sample_pose`, `bone_adj`, `model::Anim`, `slerp_bones` | done, unit-tested |
| `StudioSaveBones` / `StudioMergeBones` / `StudioCalcAttachments` | `0x100317b0` / `0x10031890` / `0x10034560` | `save_bones` / `merge_bones` / `calc_attachments` | done |
| `StudioRenderModel` (glow shell) / `RenderFinal_Hardware` / `_Software` | `0x100345f0` / `0x10034750` / `0x10034690` | `render` / `render_final` | done |
| `AngleQuaternion`, `QuaternionSlerp`, `QuaternionMatrix`, `MatrixAngles`, TS `QuaternionAngles`, `AngleMatrix`, `ConcatTransforms`, `VectorTransform` | `0x1002fbd0`, `0x1002fc70`, `0x1002fdf0`, `0x1002fef0`, `0x1002ffe0`, `0x1002f920`, `0x1002fa90`, `0x1002fa30` | `studio::math` | done, unit-tested |
| `MotionBlur_Add` / `_Update` / ghost callback | `0x10029680` / `0x10029780` / `0x10029710` | `effects::mblur` (`add`, `due` + `spawn_ghost`, `ghost_think`) | done, unit-tested. Clock is `gHUD.m_flTime` (`0x100cf944`, `hud::now`); the `TSMBlur` level byte is sign-extended (`0x1004bfaf`), so levels above 127 give 5 ghosts, and a missing level byte reads as -1 (5 ghosts). `MotionBlur_Reset` (`0x10029660`) has no caller in TS, so trails survive `HUD_VidInit`. A trail on entity 0 allocates a world-model temp entity and stops at `GetPlayerState(-1)`, as in TS |
| `HUD_TempEntUpdate` | `0x1001bd00`, `0x100569a0` | `effects::tempents::update` | done (SDK loop, ts-scaled, TS shell sounds) |
| `HUD_StudioEvent` | `0x10044b50` | `effects::studio_event` | done (5002, 5004) |
| `DrawLocalLaser` / `DrawOtherLasers` / flashlights | `0x1003bb00` / `0x1003d600` / `0x1003b250` / `0x1003b750` | `effects::laser` | done; `xtask play` checks the beams. Drawn in TS's order (local laser, other lasers, local flashlight, other flashlights). The local ones read the HUD's item bits (`CHudTSWeapons+0x14`, `state.hud_items`: the local `ActItems`, 0x40 = akimbo), not the predicted bits, which have no akimbo bit: that had hidden the second gun's beam and flash sprite. Local beam (`0x1003c5f9`): from each gun's view-model muzzle (attachment 0, 2 for the second gun) one unit down to its hit point less twice its direction, width and colour × `min(d/128, 1)`, `laserbeamanim.spr` stepped at 20 frames per second of frame time, the `PlInfo` colour. Other players (`0x1003d80e`): the direction is the in-eye view (`V_GetInEyePos`, ±0.05° jitter) unless more than ~45° off the cached hand line (attachment 0 → 1), then the hand line (TS's akimbo idle pose holds the guns up, so its laser points up); eased at `15·frametime`; traced from the eye; `laserdot.spr` 2 units short of the hit; the beam from 16 units along it from attachment 0 to 16 units short of the dot. One world beam per player, akimbo or not, as TS. Other flashlights: the in-eye view, 1024 units, a light at the hit while it hits. Both need `laserdot.spr` and `laserbeam.spr` |
| `DrawPlayerShadows` / `DrawPlayerShadow` | `0x10055760` / `0x10055500` | `laser::draw_shadows` | done. Any non-zero `cl_shadow` (negative too) draws them, in one TransAlpha batch: under the view in first person (not observing, `cl_shadow` exactly 1; distance less 12) and under every player `HUD_AddEntity` found visible; each a `PM_NORMAL` point-hull trace 8192 down ignoring physent = the player's index, a 24-unit quad facing `VecToAngles(normal)` one unit off the floor, alpha `1 − d·0.005` below 200 units |
| `DrawScopeOverlay` | `0x10054510` | `effects::scope` | done (not yet checked in engine) |
| `FX_ApplyCvar` / `FX_AdaptQuality` | `0x10057d50` / `0x10057a70` | `effects::apply_cvar` / `effects::adapt_quality` (from `draw_transparent`) | done |
| view-model framerate x time scale | `StudioDrawModel` `0x10031b44..0x10031bf4` | `studio::view_model_rate`, `Renderer::latch_view_model_rate` (in `draw_model`, as TS) | done, unit-tested |
| `HUD_TxferLocalOverrides` | `0x1001b900` | `prediction::txfer_local_overrides` | done |
| `HUD_ProcessPlayerState` | `0x1001b950` | `prediction::process_player_state` | done |
| `HUD_TxferPredictionData` | `0x1001bb00` | `prediction::txfer_prediction_data` | done |
| `HUD_PostRunCmd` / `HUD_WeaponsPostThink` | `0x100045b0` / `0x10003830` | `prediction::post_run_cmd`, `prediction::gun` | ported; the gun's frame is the shared `Gun`. The prediction time (`HUD_PostRunCmd`'s `time`, stored as a float at `*0x10099114`) is what `VKickback` (`0x10008230`) times the view shake with (`CTSGun+0x8c0`, `view::kick_angles`' `now`), not the client clock. Everything after the spawn and `lasthealth` checks needs `from.m_iId == 16` (`local_68`, set at `0x1000385c`, tested at `0x10003965`, which jumps to the epilogue): otherwise nothing is unpacked, the gun doesn't run and `to` keeps the engine's copy of `from`. The SDK `lasthealth` check (`0x10003877`, int `0x100991a4`): health going from <= 0 to > 0 runs the client player's Spawn (`0x100036d0`): the active item's (`0x1009f008`) `Deploy` (vtable +0x120 = `0x10008720`, `gun::deploy`) and the spread reset (`0x1000b9d0`, `prediction::reset_spread`), then the gun's Spawn (`0x100038c8` -> `0x100083f0`, with `CTSGun::Reset` `0x10008470`). The client `Deploy` resets the record's body, state and alternate grip, runs a `DefaultDeploy` (`0x100035f0`) that sets the view model, `m_flNextAttack` 0.5 and idle 3.0 but, unlike the SDK's, plays no view animation, plays the `draw` event (which does nothing for the local player) and syncs the fire-mode memory (`pl+0x486+id`); kung fu runs the early part of `HolsterWeapon` instead. The gun's Spawn then wipes the records and `from` overwrites the timers, so the mode memory is what lasts. The draw animation the player sees on a respawn or a weapon switch is the server's (`DefaultDeploy` skiplocal 0, `svc_weaponanim`): the client's own switch check after `ItemPostFrame` (`0x10003d8d..0x10003dc1`) passes `SelectWeapon` (`0x100086b0`) the gun's current id (`0x1009f2f0` is the gun's `+0xb8`), which `SelectWeapon` rejects (`0x100086b8`), so its `Deploy` (`0x10003db5`) never runs and nothing on the client plays a draw for a networked weapon change. After the gun's frame `to.weaponanim` is the player's predicted `pev->weaponanim` (the client `SendWeaponAnim` `0x10003680` writes it), and a first prediction whose `to.weaponanim` isn't the animation `HUD_SendWeaponAnim` (`0x1001ace0`) last played (`0x100d5cb8`) forces it (`0x10003e5f..0x10003ea1`, body = the gun's `pev->body`, or 2 with no record). The client `SendWeaponAnim` ignores its body argument and always passes the gun's `pev->body` (`0x10003695`); only the rebuild of a current record (`0x10003a8b..0x10003a99`: `GetBodyByItems(id, record items)`, read from `from` before the gun's frame) and the respawn `Deploy` (`0x100087a5`) write it, kung fu's rebuild and the gun's Spawn leave it, and it starts at 0 (`Pred::pev_body`). The gun's own `ItemPostFrame` (vtable `+0x134` = `0x1000b7b0`, `Gun::weapon_post_frame`) runs when the player's `deadflag` (from `from`) isn't 5, `0x10026860` is false and `from`'s `m_flNextAttack` <= 0 (`0x10003d4b..0x10003d76`, `gun::post_frame_gate`); `0x10026860` is `0x100cf9d8 <= 0`, the last `TSHealth` value (`Hud::last_hp`, see client-hud.md), not a menu test. The server's extra passes (kung fu, +cancel during a reload, `Gun::post_frame_ready`) don't apply on the client. The gun's Spawn also runs on the first prediction after every `HUD_VidInit` (`0x10003713`: `VGui_Startup` -> viewport `Initialize` -> the TS panel's reset `0x10065ff0` clears `0x1009ff08`; `prediction::vid_init`). It clears the 38 weapon records and the reserve ammo, which otherwise persist: each command rebuilds only the current record, its akimbo/single twin's clip and its reserve (`0x10003965..0x10003bbe`). It also clears the last shot time (`+0x8a0`, `Gun::last_shot`, the spread-growth clock) and the last view-kick time (`+0x8c0`, `view::reset_kick_time`); its `UpdateClientData` caches and `+0x89c` are never read on the client. Health going from > 0 to <= 0 runs the player's Killed (`0x1000389b` -> `0x100036b0`): the active item's (`0x1009f008`, set by any command with `m_iId` 16) `HolsterWeapon` (`0x1000ab00`). The client player's `IsAlive` (`0x10004700`) reads a `pev->health` nothing on the client writes, so the holster never reaches its living-player part, and the models, FOV, `m_flNextAttack` and idle time it sets are overwritten from `from` straight after; what lasts is clearing a pending knife/katana hit window (`pl+0x788`/`+0x78c`/`+0x790`, `Gun::melee_window`/`melee_damage`/`melee_source`). Both checks act on the carried state before `from` is unpacked, as in TS |

### Stair smoothing (`V_SmoothStairs`, `0x100677d0`)

Read from the assembly (the decompile agrees). The input is the eye z: `V_CalcNormalRefdef` sets
`vieworg = simorg`, adds the smoothed view height to z (`vh += (viewheight.z − vh)·min(15·ft, 1)`)
and calls `V_SmoothStairs(pparams)` (`0x10067a46`). With `k = min(15·ft, 1)` (constants
`0x1007a890` = 15.0, `0x10079e58` = 1.0) and statics `smoothZ` (`0x10191d84`), `offset`
(`0x10191d88`) and the previous `onground` (`0x100972ac`):

```c
if (!onground) {
    if (prev_onground) offset = z - smoothZ;   // fld [ecx+8]; fsub [0x10191d84]
    smoothZ = z;
} else {
    offset = 0;
    smoothZ += (z - smoothZ) * k;
}
z = smoothZ + offset;
offset *= 1 - k;
prev_onground = onground;
```

* **Step up / step down on the ground:** the eye eases toward its new height at rate 15, both
  ways. There is no snap distance (the port used to snap past 32 units; TS doesn't), so
  teleports and respawns also ease, over roughly 0.3 s.
* **Leaving the ground** (jumping, or walking off a ledge): the offset is `z − smoothZ`, the
  current lag with TS's sign. The SDK's `V_CalcNormalRefdef` (`cl_dll/view.cpp`, "smooth out
  stair step ups") only smooths rising z on the ground without `pparams->smoothing`: `oldz` climbs
  at 150 units/s, at most 18 below `simorg.z`, and resets to `simorg.z` otherwise, so it never
  smooths a step down and doesn't touch the view in the air.
  An offset that kept the view continuous would be `smoothZ − z`; TS's doubles the jump instead:
  jumping while the eye still lags below after a step up throws the view the same distance *above*
  the eye for a frame, then the offset decays at rate 15. Leaving the ground while lagging above
  dips it below. With no lag (flat ground, or after the eye has caught up) nothing happens, which
  is why it's hard to notice in play. The assembly is unambiguous, so the port copies it.
* **Landing:** the offset is zeroed and `smoothZ` eases on from the last air z, so landings are
  continuous.
* **Initial state:** the statics start at 0, 0 and -1 (`.data` at `0x100972ac` is `ffffffff`).
  The first frame after the library loads counts as "was on the ground", and the first spawn
  eases up from z 0. The state is never reset on map change, as in TS. The smoothed view height
  (`0x10191d90`, only referenced at `0x10067a23..0x10067a3a`) is in BSS too (past `.data`'s raw
  size), so it also starts at 0: on that first spawn the stair state eases toward an eye whose
  view height is itself still easing up from 0, and both settle on `simorg.z + viewheight` in
  about 0.3 s. The port's `View::vh` starts at 0 and is never reset, the same.
* **Body cam** (`back == 0` after smoothing, e.g. `cl_camtype 3; cam_back 0`): TS calls
  `V_SmoothStairs` again on the camera origin (`0x10068313`), on the same state. On the ground the
  smoothed z is pulled toward both the eye and the camera every frame, so the body cam settles
  between the two, nearer the camera, and closer to it the lower the frame rate (exactly on it at
  `15·ft ≥ 1`). In the air the second call reuses the offset and decays it a second time. The
  port makes the same two calls.

### Bob (`V_CalcBob`, `0x10066e70`; `V_CalcGunAngle`, `0x100674a0`)

Read from the assembly, constants from `.rdata` (`0x10081e20..0x10081e40`, `0x1007d5b0`,
`0x1007d6a0`, `0x1007d74c`, ...). `view::Bob` holds the statics: phase (`0x10191d74`), amplitude
(`0x10191d6c`, also the return value), peak (`0x10191d78`), last time (`0x10191d70`), gun offset
(`0x10191428`), smoothed z (`0x1019144c`) and bob pitch (`0x10190b78`).

* **Skipped** (return 0, no static touched, so the gun offset stays as it was): third person,
  `pparams->spectator`, or `PM_SpectatorMove`'s smoothing flags (`0x101914ec`) non-zero without
  the in-eye "keep bobbing" value 8.
* **`cl_bob` level** (gun level, pitch level): `__ftol` then an unsigned switch, so 0 → (0.1, 0),
  1 → (0.3, 0), 2 → (0.5, 0.15), 3 → (0.7, 0.5) and anything else, negatives included, (1, 1).
* **Advancing** only when on the ground, `pparams->time` differs from the last bobbing frame and
  the view height isn't 0 (a stunt's `NO_VIEW`):
  `sp = min(|vel.xy|/300, 1.1)`, `phase += ft·5.5·ts·sp·100` (`g_flTimeScale`, unclamped), reset
  to 0 (not wrapped) once above 360. The amplitude is `|vel.xy|·0.01`, unclamped, which also sets
  the peak; while the amplitude is below the peak (after the air halving) it eases as
  `(amp + 2.5)·sp·0.5` instead and the peak stays. That converges on `2.5·sp/(2 − sp)`, below
  the `3·sp` the speed formula gives at any `sp ≤ 1.1`, so once the amplitude has dropped below
  the peak (a jump, a fall, a stunt's `NO_VIEW`) it only goes back to the speed formula when the
  easing value reaches the old peak, i.e. by moving faster than when the peak was set; in
  practice the amplitude stays on the easing formula (2.5 at 300 u/s, 0 standing). As in TS.
* **Gun offset**, in world space from the engine's `pparams->forward/right/up` (the previous
  frame's view vectors; `V_CalcBob` runs before this frame's are computed):
  `up·sin(2φ)·amp·0.11 + forward·(0.75 − amp/2) + right·sin(φ)·amp·0.3`, x and y times the level,
  z = `(0.5 − 0.65·amp + z)·level`; bob pitch = `sin(2φ)·amp·pitchLevel·0.25`.
* **Not advancing:** the amplitude halves while above 0.01, the offset is `forward·(0.75 − amp/2)`
  times the level with z replaced (not added to) by `(0.5 − 0.65·amp)·level`, bob pitch 0.
* Either way z then goes through `V_SmoothBobZ`: a step of `(smoothZ − z)·ft·20`, never past z.
* **Use:** the view model is placed at `vieworg`, shaken (0.9), then the offset is added as is
  and the fov push follows. `V_CalcGunAngle` sets the gun's angles to
  `(amp − viewangles.pitch, viewangles.yaw, viewangles.roll)`: the gun pitches up by the bob
  amplitude and rolls with the stunt roll.
* **Bob pitch into the aim.** `V_CalcNormalRefdef` adds `0.1f·bob pitch` (`0x1007a84c`) to
  `pparams->cl_viewangles.pitch` every frame (`0x10067d14`) and zeroes the bob pitch, after the
  kick and shake steps. The engine copies `cl_viewangles` back into its view angles (FWGS
  `V_GetRefParams`; there is no `SetViewAngles` call), so the view pitch is the running sum of
  the bob pitch: a swing that grows with the frame rate (about ±0.4° at 100 fps running, ten
  times that at 1000 fps), and the aim keeps whatever the sum is when the bob stops, its phase
  resets at 360° or its amplitude changes. The port does the same, on the same value
  (`view::add_bob_pitch`). Frames `V_CalcBob` skips (third person, spectating) add 0; the
  spectator and intermission views don't add it.

## Event parameters (as handled)

All events arrive with `FEV_NOTHOST`; the shooter's copy comes from local prediction. Where R§4.2
and W§6.1 disagree, the handler accepts both.

| Event | f1 | f2 | i1 | i2 | b1 | b2 | Handler behaviour |
|---|---|---|---|---|---|---|---|
| fire | spread X | spread Y | weapon id | attachments (1 sil, 2 laser, 4 flash, 8 scope) | clip empty / akimbo hand / pump mode | shared-random seed | dispatch on class (4/5 akimbo, 1 pump, else standard). Anims: standard `[6]` if `b1`, else `[3 + r(0,2)]`; pump `[12]` if `b1` (pump mode), else `[11]`; akimbo `[22 + r(0,1)]` if `b1`, else `[24 + r(0,1)]` |
| firelinked | spread | spread | id | attachments | hand | seed | both guns, +-5 along right, second seed + 0x21; anim `[3 + r(0,2)]` |
| empty | - | - | - | - | - | - | `weapons/pistol-empty.wav`, `CHAN_WEAPON` |
| pump | 0 | 0 | id | attachments | 1 | 0 | anim set[13/14], shell + puff, pump sound (`sound2`, entity -1 at the event origin) |
| reload | 0 or 8/9/10 | 0 | id | attachments | 0 | 0 | anim set[1] (`f1` 0) or set[phase]; `insert-shell` (9, everyone); pump sound (10) and `generic_reload` (0) for others and third person, then for a visible player the casings (sawed-off, Raging Bull, Contender) or `generic_clip` (two for akimbo) |
| draw | 0 | 0 | id | attachments | 0 | 0 | in-eye spectating only: set[2] |
| grenade | 0 | 0 | 19/20/21 | 0 | 0 | 0 | in-eye only: M61 set[i1], body 1 |
| knife | 0 | 0 | id | hit flag | 0 | 0 | trace 35: sparks + `knife_hit` on hard surfaces, else `knife_miss`; then set A[15 + r(0,2)], body `i2` (1 on flesh), first person only |
| katana | 0 | 0 | anim index (15 = back swing) | hit flag | 0 | 0 | trace 50 / -55: sparks + `katana/hit[1]`, else `miss[1]`; then set A[i1], body as the knife |
| closecombat | weapon id (block), 0 (attack) | 0 | 0 block / 1-3 attack | set A index | 0 | 0 | view owner: set A of weapon `f1` at `i2`, body 0, blocks included (`0x100433bb..0x10043401`); then `player/block.wav`, or `player/closecombat.wav` and a trace from the gun position along `forward` by -55 (`i1` 3, back kick), 45 (`i1` 2, kick) or 35 (punch) (`0x1004342c..0x100435b0`), material sound + puff on a hit |
| coldcock | dir.x (unused) | - | id | attachments | 0 | 0 | set A[28], `player/closecombat.wav`, trace 30, material sound + puff |

The "view owner" (`0x1001c5e0`) is the local player or the player spectated in-eye: the fire,
pump, reload, closecombat and coldcock anims play for both, and the fire/pump events' first-person branch runs
for the owner outside third person. Everyone else, and the owner in third person, gets the
player-model effects only if `HUD_AddEntity` marked them visible this frame. `r(a,b)` is
`RandomLong`; "set" is `GetActiveAnimSet`'s, "set A" the normal set (`info+0x252`) the melee
events index directly. Every event sound uses volume `RandomFloat(0.92, 1.0)` and attenuation 0.8,
except the fire sound (`0x1003de90`) and the material sounds.

**Player attachments come from a per-frame cache.** TS's events never read a player entity's
`attachment[]` directly: `TS_DrawEffects` copies attachments 0-3 of every entity with the `player`
flag (indices 0..=maxclients) into `0x1018caf8` (`0x10056eb4`), and the events read that through
`0x10056be0`. The cache matters on FWGS: `CL_EmitEntities` runs `CL_LinkPlayers`, which sets every
linked player's attachments to its origin, then `CL_FireEvents`, before the studio renderer
recomputes them for the players it draws. So at event time the live entity holds the origin, and
the cache holds last frame's drawn positions. The port does the same
(`effects::cache_player_attachments`, `effects::player_attachment`).

In first person TS takes the muzzle flashes and the pellet spray from that cache too
(`0x1003e39e`, `0x1003e5c4`, `0x1003f4b8`, `0x1003f579`, `0x10040fcd`, `0x10041091`), and only the
smoke and shells from the view model (`g_pViewModel+0xb60`). For the local player the cache holds
the view model's muzzle, on both engines: the local player isn't drawn in first person (Steam
`CL_LinkPlayers` skips it unless `chase_active`; FWGS `CL_AddVisibleEntity` skips `R_AddEntity`),
but before the scene each engine runs the view model's `STUDIO_EVENTS` pass (Steam
`R_PreDrawViewModel`, then `R_RenderScene`, then `R_DrawViewModel`, in `hw.so`'s `R_RenderView`;
FWGS `R_RunViewmodelEvents` before `R_RenderScene`), and the studio renderer's events step copies
the view model's attachments into the entity of its `index`, which is the local player's (Steam
`cl.viewent.index`, FWGS `V_SetupViewModel`'s `cl.playernum + 1`; `studio.rs` `events`, as
`StudioDrawModel` does). The transparent pass that fills the cache comes after it. Measured in the
port on native FWGS (`ts_dev_fxlog`): the cached attachment and the view model's attachment 0 are
the same point, 36 units ahead of the eye and 7 off the view axis.

Spectating in-eye the cache doesn't hold the gun: the view model's attachments go to the
spectator's own entity, and the target's are its origin (FWGS `CL_LinkPlayers` every frame, Steam
`CL_ProcessEntityUpdate` on every update), since the in-eye target isn't drawn. TS's flash for an
in-eye target starts at the target's hull centre, 28 units below the eye; the port takes the view
model's attachments there instead (see Deviations).

**The predicted events carry the player's origin and view angles.** TS's client weapon code plays
its events through `HUD_PlaybackEvent` (`0x1001ad50`), the SDK's: only with `runfuncs` and a final
state, with `origin = g_finalstate->playerstate.origin` (the command's `to`, set for the length of
`HUD_WeaponsPostThink`, `0x10003830`) and `angles = 0x10190b50`, the `pparams->viewangles` the last
`V_CalcNormalRefdef` (`0x10067890`) left. Neither engine fills them in for a client-side event
(FWGS `CL_PlaybackEvent` queues them as given), so they decide where the shooter's own bullets,
impacts, decals and whizz traces go and which way the flash, smoke and shells point. The port used
to pass zeros (`goldsrc::ext::render::playback_event`), so every predicted shot flew from the world
origin along +x and the flash sprites streaked along world +x whichever way the player looked; it
now passes both as TS does (`prediction::gun::playback`, `view::playback_angles`).

Pitch: `25 + 75*ts + rand(-5,5)` everywhere (see deviations for the fire sound). Fire sound:
`weapons/<sound>` (silenced: `sound2`, volume 0.84-0.92, attenuation x2), attenuation from the
client ammo table (`impact::AMMO_ATTENUATION`, decoded from `FUN_10007480`). The silenced flag
alone picks the sample: a weapon with no `sound2` (AK-47, USAS-12, ...) fired with the flag set
plays TS's empty buffer, an empty sample name, which the engines ignore.

### Client bullets

`EV_TS_FireBullets` queues one per pellet (`ClientBullets_Add`, first free slot of 256; with all
in use the bullet is dropped) with the pellet direction unnormalised, the damage, ammo and bullet
type. The slot counter (`0x100f1278`) counts bullets added since the last frame plus those the
last frame found in use; `TS_DrawEffects` runs the frame at its very end, just before
`glDisable(GL_FOG)`, while the counter is non-zero. The frame looks up
`sprites/shotgun_pellets.spr` (additive) and walks the slots in order:

1. **Trace step** (`ClientBullets_Impact`) when the client time is past the slot's next-step
   time. The next step is then due 0.1 s later with `realbullet` 0, 0.05 s with 1, otherwise next
   frame; the step covers that time (the frame time when it is under 1 ms):
   `dir · velocity·40 · dt · GetBulletVelocityMult(ts, bullet time)`. It traces with players
   solid except the shooter (hull 2, studio boxes). A hit runs `TS_PlayTextureSound` (from the
   hit point toward the hit point plus the step) and the decal, then clears the slot, unless the
   bullet is type 0 and the damage left (range falloff from the leg's start, on a fixed 30-unit
   scale for exit bullets, times the ammo's pierce) is over 5: then it moves `min(left, 48)` on
   and, outside a solid, carries on from there as an exit bullet (type 2) after the exit point's
   impact. A bullet that never hits keeps flying (no range limit).
2. **Draw** (`ClientBullets_Update`) in slow motion (ts < 0.99), also on the frame the step just
   cleared the slot (it then draws a `w`-sized sprite at the world origin and adds an empty trail
   quad, as TS). Nothing happens outside the view cone (`InViewCone` of the drawn position).
   Otherwise, with the sprite: `RenderMode(TransAdd)`, no culling, `Color4f(1, 1, 1, a)` where
   `a = d²·0.1` within `sqrt(10)` units of the view and 1 beyond; unless the ammo is buckshot (3),
   two streaks `clamp(ts·37.5, 5, 25)` either side of the drawn position along `dir`, `w =
   max(1 − ts², 0.15)` wide across the view's up and then right; then a `w`-radius billboard. The
   drawn position then moves `dir · velocity·40 · mult · frametime`: it is separate from the
   traced position (which moves in throttled steps and alone hits, penetrates or clears), and it
   doesn't move while out of the view cone. With `cl_wavedetail` > 0, at most every
   `SlowTime(0.01)` s, a `bullet_trail.spr` quad (`TS_BulletTrail`, `w` wide, alpha `a·0.75`) joins
   the last trail point to it.

## Deviations from TS 3.0

* **Melee view shakes** run inside the shared `Gun`, so they follow its branch choice (which, like
  the client `KatanaFire`/`CCombatAttack`, reads `FL_DUCKING` on the gun's own `pev`,
  `0x1000a052`/`0x1000b1c1`, so only `+duck` counts).
* **Camera mouse modes read the mouse motion, not the Win32 cursor.** TS's `CAM_Think` reads
  `GetCursorPos`, takes the offset from `GetWindowCenterX/Y` and re-centres with `SetCursorPos`,
  while `iMouseInUse` keeps `IN_MouseMove`/`IN_Accumulate` off the cursor. The port takes the
  pending raw mouse counts from `input` instead (`input::take_cursor_offset`), which on the Windows
  Steam path without `m_rawinput` is that same `GetCursorPos`/`SetCursorPos` re-centring, and on
  FWGS (`IN_ClientLookEvent`) and SDL relative mouse the equivalent motion. Fractional counts
  (FWGS) carry over to the next frame. The engines the port runs on don't all have a Win32 cursor
  to read (FWGS on Linux and SDL relative mouse report motion only), so the cursor itself can't be
  used; the motion is the same. TS's quirks are kept: after `-cammousemove` with `+camdistance`
  still held, the stale `cam_mouse` reading keeps moving the distance, and the distance block's
  re-centre (`0x10025138`) throws away the motion meant for mouse look (`camera::adjust_ideal`
  drains it). The sensitivity-scaled `cam_old_mouse_x/y` TS stores are never read and are left
  out.
* **`TSFrags` type 2 direction.** TS passes the scintille and sparks a direction from stack
  memory the handler never writes (`[esp+0x3c]` at `0x1004be15`/`0x1004be3b`; the message's
  second vector, read into `[esp+0x18]`, is unused), so its value is whatever an earlier call
  left there. The port uses the message's vector. No server sends type 2.
* **Raw GL fog only with desktop OpenGL.** TS calls `opengl32.dll` unconditionally (a software
  renderer has no GL context, so the calls do nothing there). The port looks the functions up in
  the renderer's GL library and calls them only when the engine draws with desktop GL (FWGS
  `r_refdll_loaded` = `gl`; elsewhere `IsHardware()` = 1). On FWGS's GLES renderers (`gles1`,
  `gles2`, `gl4es`, `gles3compat`), which translate GL through a wrapper or lack the fog calls, the
  port skips them, so there is no TS fog and the end-of-pass `glDisable(GL_FOG)` doesn't switch
  off the engine's own fog. On desktop GL the `glDisable` does, as in TS (e.g. FWGS's underwater
  fog, for the rest of the frame).
* **Shell limit.** TS keeps the newest 64 shells by storing tempent pointers in a ring and setting
  the evicted shell's `die`. The port tags each shell with a sequence number in
  `curstate.iuser2` (`effects::track_shell`), and the temp-entity loop (`client-dll/src/tempents.rs`)
  kills shells older than the newest 64. When the engine has already freed an evicted shell and
  reused its slot, TS's write kills the unrelated temp entity now there; the port can't hold
  pointers into the engine's temp-entity pool across frames (FWGS reallocates it), so that case
  differs.
* **Defensive checks where TS would read out of bounds or stop the game.** The studio renderer
  parses the model from the bytes the engine loaded (`studio::model`, bounds-checked), so a
  corrupt model draws a bind pose instead of reading out of bounds. Out-of-range (and negative)
  `curstate.sequence`, `gaitsequence` and `latched.prevsequence` are treated as 0 (TS dropped the
  SDK's gait check in `StudioProcessGait` and never checks for negatives); a bone whose parent
  isn't an earlier bone is treated as a root; an attachment's bone index is clamped; a missing
  sequence-group file reads as the bind pose; a ghost's `PlayerInfo` is only asked for valid
  player numbers (FWGS returns its menu player for -1). A model with more than 4 attachments draws
  the first 4: TS prints `Too many attachments on %s` and calls `exit(-1)` (`0x10034560`), which
  would quit the game. `TSMBlur` for entity 32 or above, or an empty message (entity -1), is
  ignored: TS's 32-entry trail table would be written out of bounds. A map particle whose
  `oncollide` 2/3 target names no template doesn't spawn (TS indexes the template table with -1).
  Cooldown slots wrap at 128. A client bullet or `EV_TS_FireBullets` with an ammo index outside
  the ammo table uses pierce 0, range unit 80 and velocity 330 (TS reads past its table,
  `FUN_100071f0`), and a bullet sprite whose model the engine can't find isn't bound.
* **Transcendentals in the spectator angle maths.** `VectorAngles`'s `fpatan` and `0x10029d70`'s
  `_CIacos` (`fpatan` of `sqrt((1+x)(1−x))`) produce a 64-bit-mantissa result whatever the
  precision control; the port uses `f64::atan2`/`acos`, which can differ in the last `double`
  bit. Every result is rounded to `float` before use, so this only shows when that bit decides
  the `float` rounding.
* **Panic guards (the game must never abort).** Both profiles build with `panic = "abort"`, so
  out-of-range data from the server, the demo, the model or map files, or cvars is bounded instead
  of panicking:
  * An emitter rolls at most 4096 particles per emission (`MAX_PER_EMIT`).
  * `shared_random_long` over the full 32-bit range returns `low + r` instead of overflowing.
  * An event entindex of `i32::MIN` is never local (`is_local` and the shooter and melee indices
    wrap).
  * The shell sequence and the gun's deploy `mode` wrap. The motion-blur `max` saturates.
  * `unpack_clientdata`'s `clip - left` wraps.
  * The overview layer is capped at 128×96 tiles (`MAX_TILE_CELLS` 32), and its inset sum wraps.
  * Studio model offsets past `usize::MAX / 4` read as 0 (`model::at`). Angle wrapping, gait, the
    animation base and `numframes` use wrapping or saturating maths.
  * `input`'s cursor bounds tolerate the 0×0 screen before `HUD_VidInit`. The camera offset, the
    joystick axes and the zoom easing wrap as C `int`s.

## TS behaviour worth knowing (not deviations)

* **Fire/ricochet/explosion pitch** is TS's `100 - (int)(ts * -300)` (`0x1003eeed`, `0x10044dbd`,
  `0x100481e4`; constant `0x1007d6f4` = -300.0), i.e. **400** at normal speed; the fire sound adds
  `RandomLong(-5, 5)` before rolling its volume (`0x1003de90`), the ricochet and explosion add
  nothing. FWGS's `S_StartSound` passes any pitch > 1 through unclamped, so these play at 4x
  speed, as with the original TS client on FWGS.
* **Local-player translucency**: the crosshair-on-player flag is the HUD's `crosshair_id` from the
  previous HUD draw, as in TS (`0x1018a60c`, set to 1 or 2 by `DrawCrosshairID` for a named
  player).
* **`snapto` and `CAM_Think` write `cvar->value` directly** (`Cvar::set_value`), as TS does, so
  `cam_snapto`/`cam_ideal*` printed at the console can show a stale string, as in TS.
* **Particle numbers** in `effects::fx` and the fire/melee event call sites are read from the TS
  client's disassembly (the decompile hides float arguments behind `__ftol`), including every call
  site's arguments, the `cl_fx` tuning globals (`0x1018ca40..0x1018ca60`) and the cooldown slots.
  Notable corrections: the `TS_GunSmoke` "45-55" argument is the animation frame rate (always 5
  puffs, `0x10047130`); the muzzle flash spreads `max(3, 9·fxmul)` sprites over `info+0x354` with
  base radius `info+0x358`, and silenced shots draw a dim flash that fades out by 512 units; the
  muzzle flash core rolls a spin, its size and a second spin (`0x10047861..0x100478c7`) and is
  added even when `ts_muzzleflash6.spr` is missing (then with no sprite); the
  shotgun's muzzle spray is `TS_SpriteDebris`, from attachment 3 in third person (`0x1003ed21`);
  the silenced shot's second wisp sits `muzzle − fwd·ofs` back along the barrel (`0x1003e54b`);
  akimbo flashes from attachment 0 (`b1` set) or 2, both for a linked shot, and smokes once at the
  `b1` gun, with no pellet spray (`0x1003f55c`, `0x1004050e`); the pump's shell and wisp leave
  `muzzle − fwd·ofs` (view model attachment 0, or the cached player attachment 3 in third person and
  for others), only when the weapon has a shell model; glass is always 81 shards on a 9×9 grid; the
  explosion has no sparks, 24/48 fireball/smoke particles × the quality scale, ends early if a
  cloud sprite is missing, and its flash keeps the smoke's drag 3.3 (TS reuses the scratch
  particle); the surface mark's angles are truncated before a negative angle is wrapped
  (`VecToAngles` `0x10046110`). Smoke-like particles (life 0, no fade) live for one run of their
  animation, which comes from the sprite file's own frame count (`SPR_Frames`), as in TS.
* **Map particle templates** (`ParticleKeyValue` `0x10055f90`) keep the map's raw values: the
  client has no copy of the server's `CTSParticle::CheckValues` clamps, so a colour above 1 or a
  negative life reaches the particles. `follow_view` keeps its low byte (4 and up act as 0, and
  can stretch), `oncollide` other than 1-3 has no callback. A particle is rolled as
  `Particle_RandomizeFromTemplate` (`0x1003a9b0`) does: `RandomFloat(low, high)` with the raw
  bounds (the frame `RandomLong`), in TS's order. The `oncollide` 2/3 spawn (`0x10039d50`,
  `0x10039f60`) inherits the colliding particle's velocity unless the template's spawn flag 1 asks
  for a fresh roll, and reports a sprite that won't load.
* Melee events: the knife and katana play the material sound on every hit (`0x10043f63`,
  `0x10044585`) and add metal sparks plus `knife_hit`/`hit`/`hit1` on every texture type except
  0, `'D'`, `'W'` and `'F'` (byte tables `0x10044140`/`0x10044708`; slosh sparks too). Only a
  miss plays `knife_miss`. `TS_MeleeMaterialSound` returns 0 with no sound on `CONTENTS_SKY`
  (`0x10045e0a`), and 0 after the default `pl_step1/2` when `pfnTraceTexture` finds no texture
  (`0x10045e6a`); close combat and the coldcock still puff on a 0. Its lists keep TS's order
  (tile and slosh 1, 3, 2, 4; duct twice), the pitch is rolled before the sample. The katana
  rolls `hit`/`hit1` (`miss`/`miss1`) only when it plays one, after the sparks (`0x10044611`).
  Every event sound rolls its pitch before its volume (`0x10042507..0x1004256f`).
* Stretch lengths use TS's speed (units per 0.1 s): `stretchspeed` is a TS-unit factor in both
  the map templates and the effects.
* `EV_TS_Pump`'s wisp blows along `v_right` (`0x10189e10`, the right vector of the
  `AngleVectors` call at `0x10041b09`).
* `PointVisible` (`0x100572a0`) traces the point hull from the cull origin with
  `PM_GLASS_IGNORE`, ignoring the local player, against whatever physents the engine has set up.
* Particles follow `PSys_UpdateDraw` (`0x100520d0`), `Particle_Move` (`0x10050ce0`),
  `Particle_Think` (`0x10051270`) and `Particle_Collide` (`0x10051d50`): the four-way death test
  with TS's 0.001 thresholds (no sprite counts as a finished animation); the push-list entry;
  fading (clamped to 0..1) only after the life ends; gravity from the engine's `cl_gravity`
  (`0x100963ec`, stored by `HUD_TempEntUpdate` at `0x1001bd25`); think before move (the think and
  trace times start at 0, so a new particle thinks on its first pass); nothing moves, thinks or
  collides while resting; collision is `EV_PlayerTrace(prev, origin, PM_STUDIO_BOX, -1)` with the
  point hull and covers the whole path since the last trace, only inside the view cone (which also
  hides the particle), at `max(d·zoom/8192 (×2 beyond 1024), 0.016)` s from the cull origin; a
  trace stuck from the start kills a particle younger than 0.5 s and rests an older one, a start
  inside a solid kills it after the bounce; the hit lands exactly on the surface, damps by
  `0.3·max(1 − 7·iv, 0)` and rests once `ts·travelled < 3·drag` (`Particle_Rest` also clears the
  roll and its spin); the collision callback runs right there (`oncollide` 2 kills, then spawns
  into the first free slot, possibly the same one, and the rest of the pass works on the new
  particle; blood leaves its decal and clears the drop's in-use byte); a particle killed in its
  collision is still drawn and stepped that frame; the draw uses the frame before this pass's
  step, `(int)min(frame, frames − 1)`, frames advance only after the life ends and aren't
  clamped; only collision culls (`InViewCone` from the cull origin, fixed 0.9 threshold, no FOV
  term). Random drift steers an accumulator (`+0x68`) by at most `randomdelta` and adds it times
  the think interval (`divisor · 0.033333335`, at least the frame time); react moves the
  particle's *origin* by `(128 − d)·|v|·dot/128·T·react` for every solid player within 128 units
  (local: predicted velocity, others: velocity between their last two updates); wind is ×10 (TS
  units), physics-gated, steps its seeded segment inside a wind particle's think and interpolates
  speed and direction; `TSPSys` eases from the old gust to the new base speed over the old segment
  length (`0x1004c00b..0x1004c082`). Think and trace distances halve when zoomed (FOV < 40). The
  spawn, think, trace and cooldown times are client-time floats (`GetClientTime`), as TS stores
  them. TS's per-frame callback slot (`+0x118`) and attached-entity particles (`+0x06`/`+0x07`)
  are never set in 3.0 and aren't ported. The pool, sprite cache, client bullets, emitters and
  motion-blur trails are not reset by `HUD_VidInit` or a level change; the first `TS_DrawEffects`
  after `HUD_VidInit` rebuilds the pool and the sprite cache, and a level change only reloads the
  emitters, templates and fog and restarts the emitter clock (`0x1003ab8e..0x1003abc3`).
* **Quads** (`Quad_Build` `0x10050080`): a resting particle is a screen billboard whatever its
  `follow_view`. A stretched quad runs head (v = 0) to tail, `radius·(toCam̂ × vel̂)` wide, so it
  narrows as the velocity turns toward the camera; `follow_view` 3 uses `followaxis` unnormalised
  (no default: a template without one draws nothing, as in TS) and `toCam̂ × followaxis` across.
* **In-eye flash at the gun.** Spectating in-eye, the target's muzzle flashes and pellet spray
  come from the view model's attachments (while `r_drawviewmodel` lets the view model's events
  run), not from TS's player-attachment cache, which holds the target's origin there (see "Player
  attachments come from a per-frame cache" above): TS's own flash starts 28 units below the eye.
  For the local player the cache already holds the view model's muzzle, so the port reads it as TS
  does.
* The TS shotgun shell bounce plays `weapons/sshell.wav` (`TempEnt_PlayShellSound` `0x100569a0`),
  which TS 3.0 ships.
* **`gl_fog` is an engine cvar on FWGS.** `ref_gl` registers `gl_fog` ("1", `FCVAR_GLCONFIG`)
  and gates its own fog and `TriFog` on it. The client registers it again, as TS's `CHud::Init`
  does, so on FWGS the client's default 0 overwrites the engine's 1 (`Cvar_Get` resets an
  existing cvar) and `gl_fog` gains `FCVAR_ARCHIVE`; the original TS client does the same there.
  Whether Steam GoldSrc / HL25 `hw.dll` also owns `gl_fog` hasn't been checked (no client engine
  here to read). `gl_fog_start/end/red/green/blue`, `cam_*`, `c_*`, `scr_ofs*` and
  `bitmapfonts` are not engine cvars in FWGS; they are client-dll cvars in the SDK.
* Shadows, laser beams and the scope overlay are TriAPI quads, as in TS. The scope overlay
  (`DrawScopeOverlay` `0x10054510`, `effects/scope.rs`) is TriAPI too, not direct GL: after the
  particles, in first person, alive and not observing, with the eased FOV below 40 and the hardware
  renderer, `sprites/quarterscope.spr` frame 0 is drawn four times (render mode TransTexture, no
  culling, white) as mirrored quads of half-size `max(fov·0.1, 1)·0.5` around `v_origin +
  v_forward·10`. The decomp doc's "frame 2" is the sprite-cache render-mode argument; the call
  passes frame 0 (`0x1005457e`) and the sprite has one frame.
* **`pparams->forward/right/up`** are written only by the normal view, from `cl_viewangles`
  (`0x10067eee`); the intermission (`0x100676e0`) and spectator (`0x10068f70`) views leave the
  engine's values. The effects' own vectors are `AngleVectors(v_angles)` every pass (`0x10056f74`).
* **Observer modes use TS's numbering: in-eye is 5, not the SDK's 4** (`shared::ts::obs`). The
  server's `Observer_SetMode`/`Observer_HandleButtons` only ever set 1, 2, 3 and 5,
  `PM_SpectatorMove` places mode 5 at the target, and `HUD_AddEntity` (`0x100447f0`), the draw and
  grenade events (`0x1001c5c0`) and `V_CalcSpectatorRefdef` test `g_iUser1 == 5`. Mode 4 is the
  HLTV director's locked view.
* Spectator refdef: TS computes chase positions there too, but only into globals the function
  overwrites at its end; the camera comes from player movement. TS's client-side
  `PM_SpectatorMove` (`0x1002c790`) differs from the server's, and the port runs it on the client
  (`spectator::movement`, plugged into the shared movement with `shared::pm::player_move_with`):
  it raises the `cl_vsmoothing` flags (origins in chase locked, angles and origins in in-eye and
  the locked view, plus in-eye's "keep bobbing" bit), makes the view use the camera it computed
  (so chase locked looks along the target's view, not the mouse), keeps the last camera while
  the target isn't sent (outside the PVS), and runs the director's locked view (mode 4, HLTV:
  from the primary target towards the secondary, 12.5° pitch and 22.5° to the side the director
  flags pick, swung 45° back when the line is blocked, eased by 0.1 per move). With no secondary
  target TS halves and negates the target's yaw (`0x1002cb7d`); kept. The flags and overrides
  are only written on `runfuncs` moves, so with `cl_predict 0` the view falls back to `simorg`
  and the mouse angles, as in TS.
* The HLTV inset view (`V_CalcSpectatorRefdef`'s second pass, `nextView`) and the overview's
  camera icon are ported, but TS never sets an inset mode (`specmode` and the director reset
  both pass 0), so neither runs, there or here. Like TS, `view::calc_refdef` never writes
  `nextView` itself (the engine zeroes it before the first pass).
* Overview entities (`AddOverviewEntityToList` `0x10024ab0`) are kept by entity index where TS
  stores the `cl_entity_t*`; `GetEntityByIndex` returns that same engine record when drawn, so
  the icons are the same. The name-tag offset uses `WorldToScreen`'s x and y (TS's z is 0 too).
* In-eye gun: as in TS, the gun offset is whatever `V_CalcBob` last left in the normal view
  (`0x10069a70`), and the pitch takes a bob global nothing writes (`0x10191dac`, so 0). TS's `V_GetInEyePos` scales the pitch by -π (the SDK
  uses -3); kept. Like TS (`0x10068e42`) it writes nothing unless the target also has a player-info name (issue #153).
* **Studio state copies.** `player_info_t` is copied in and written back where TS keeps the
  engine pointer, and `StudioDrawPlayer`'s engine state (`GetPlayerState`, not the entity's
  `curstate`, on FWGS) is a copy with the only two fields the renderer writes (`renderamt`,
  `gaitsequence`) written back; nothing reads either between, so the result is TS's.
* **View-model framerate (decomp correction).** R§2.3 says the rate is latched "once per
  animation restart". The asm (`0x10031bb0..0x10031bf4`) latches it on the first view-model draw
  of a frame whose `latched.prevframe` is more than 0.001 past the one the previous draw saw,
  i.e. while the animation advances, so the rate follows the easing time scale; a restart (frame
  falling back) keeps the old rate. At normal speed without double fire rate the framerate isn't
  written, so the last slowed rate holds until the engine's next weapon animation sets it to 1.
  The port does exactly this in `Renderer::draw_model`.
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
* **Spectator hide.** `StudioDrawPlayer` hides the player spectated with `g_iUser1 == 5` (in-eye,
  `shared::ts::obs`) as TS does.

## Missing pieces

* Nothing in the studio renderer (issue #2) or the motion-blur ghosts (issue #3): every
  overridden `CStudioModelRenderer` slot (R§2.2) and `MotionBlur_*` is ported and was audited
  against the asm. What is still open is running them in an engine (see "Testing").

* Nothing in **weapon prediction**: the client runs the server's `shared::weapons::Gun` state
  machine (`prediction/gun.rs`), so every weapon action the TS client copies of `CTSGun` predict
  is predicted here too, with the same local events. Checked against the client's asm: shotgun
  reload phases (`ShotgunReload` `0x1000a4c0`, the three `reload` events at `0x1000a576`,
  `0x1000a66a`, `0x1000a6ef`; `Gun::shotgun_reload`), kung fu (`CCombatAttack` `0x1000af10`,
  `closecombat` at `0x1000b61a`; `Gun::ccombat_attack`), grenades (`GrenadeFire` `0x10009e30`
  and `GrenadeIdle` `0x1000a840`, `grenade` at `0x10009e86` / `0x1000a8b2`; `Gun::grenade_fire`,
  `Gun::grenade_idle`), melee blocking (`CCombatBlock` `0x1000adc0`, `closecombat` at
  `0x1000aed4`; `Gun::ccombat_block`) and akimbo linked shots (`AkimboFire` `0x100096f0` plays
  `firelinked`, the gun's `+0x82e` event, when both guns fire, `0x10009d0e`; `Gun::akimbo_fire`).
  As in TS, damage, traces, projectiles and server sounds stay on the server (`ClientHost`).
* Particle `termvelocity` unused (TS doesn't read it either).

## Integration notes

### Weapons package (`shared::weapons`)

* **Shared random:** `events::random` is a private, bit-exact copy of TS's
  `UTIL_SharedRandomLong/Float` (float range, seed = `seed + bits(low) + bits(high)`; reference
  values from a C build in the unit tests). When `shared::weapons` has its port, delete this module
  and re-export theirs; the tests' reference values (`0x3f065d00` for `(1234, 0, 1)`,
  `0xbfb0e2eb` for `(77, -pi, pi)`, `36` for `long(1234, 0, 100)`) should hold for both.
* **Pellet spread** (`fire::pellet_dir`) must match the server's `ShootBulletsPlayer`: pellet `i` is
  1-based, `ang = SR(seed+i, -pi, pi)`, `r = SR(seed+i+1, 0, 1)`, `m = 1 - r^3`,
  `dir + right*cos(ang)*m*spreadX + up*sin(ang)*m*spreadY`, range 8192. The client rounds it as
  `client.dll` does (`spread::pellet_dir(PelletSide::Client, ..)`, issue #161: `x` stored, `y`
  unrounded, `x·right·sx + y·up·sy + dir` per component on the FPU, one store), which differs
  from mp.dll's and `ts_i386.so`'s rounding in the last bit, as it did in TS.
* **Gun prediction:** `prediction/gun.rs` unpacks the networked state with
  `shared::protocol::weapons` (`vuser3` = id/firemode/attachments, `vuser1[1..2]` = powerups/active
  items, `vuser2` = counter/reserve/alt grip, `vuser4` = hand/clip/in-reload; `weapon_data[16]`
  timers incl. `fuser4`), runs one usercmd of the shared `Gun` (`pre_frame`, `weapon_post_frame`
  behind TS's client gate, `decrement_timers`: `msec*0.001*max(fuser1,0.01)`, x1.5 with DFR) through
  a client `GunHost` that plays the events locally with `FEV_NOTHOST` and applies `VKickback` and
  the melee view shake (runfuncs only), and packs the result into `to`. Note `alt_grip` is read
  from `vuser2[2]` (server doc: slot `+0x2c`).
* **Server events:** the server must `PRECACHE_EVENT(1, "events/<name>.sc")` all 11 in TS order
  (`events::EVENTS`): local playback fails with "event was not precached" otherwise.
* **Client weapon info** not in the TOML tables (class `info+0x348`, anim sets A/B) is in
  `events/info.rs`, generated from the client builders. If the weapons package adds these to
  `weapons.toml`, switch `events::info` over.

### HUD package (`state.rs` fields added; nothing removed)

| Field | Writer | Reader |
|---|---|---|
| `time_scale_target` (existing) | HUD `TSSlowMo` handler (`decode_slowmo` in `protocol::effects`; also play `goslow.wav`) | `hud::frame` (`HUD_Frame`) eases `time_scale` |
| `fov_target` (`0x100df1a4`) | HUD `SetFOV` handler (`cl_lw` 0) and `prediction` (`to.client.fov`, every `HUD_PostRunCmd`); `hud::get_fov` during demo playback | `hud::think_fov` eases `fov` |
| `fov` (`m_iFOV`) | `hud::think_fov` (eased, from `HUD_UpdateClientData`); `SetFOV` sets it outright | HUD (sensitivity, scope), `view`, effects |
| `bullet_time` | render (`TSBTime`) | client bullets (`GetBulletVelocityMult`) |
| `powerups` | prediction (from clientdata) | view-model frame rate (`studio`) |
| `active_items[65]` | HUD `ActItems` handler (index = entity number) | `effects::laser::draw_others`, HUD |
| `laser_color[65]` | HUD `PlInfo` handler (key 0, index = entity number) | lasers, HUD `DrawLaserDots` |
| `spectator[32]` | HUD `Spectator` handler | `effects::add_entity` |
| `laser_dots[2]` | `effects::laser::draw_local` (`ClEngine::world_to_screen_ndc`, −1..1) | HUD `DrawLaserDots` |
| `laser_range` | `effects::laser::draw_local` (first hit distance ×0.025) | HUD `DrawCrosshair` |
| `items_ready` | prediction (`from` active items, 0 while `m_flNextAttack > 0`) | HUD `DrawLaserDots` |
| `spec_mode`/`spec_target`/`spec_flags` | `prediction::process_player_state` (g_iUser1..3) | view, events, HUD |
| `dead`, `weapon` (existing) | HUD | lasers, effects |

**Messages hooked by render** (in `effects::init` / `view::init`): `TSPSys`, `PTakeDam`,
`TSFrags`, `TSMBlur`, `TSBTime`, `TSFade`. The HUD package hooks the other 58. FWGS keeps the first
hook for a name (`hud::init` runs first), so if the HUD hooks one of these it must forward to
`effects::on_tspsys/on_ptakedam/on_tsfrags/on_tsmblur/on_tsbtime` or `view::on_tsfade`.

### lib.rs (not edited by this package)

* `HUD_UpdateClientData` calls `view::update_client_data(cdata)` and returns 1: it runs
  `CHud::Think`'s zoom easing and hands the eased FOV to the engine (TS `CHud::UpdateClientData`).
* `HUD_Frame` passes the engine's **frame time** (FWGS `host.frametime`), and `view::frame` treats
  it as such (absolute time comes from `GetClientTime`). It is not clamped: TS hands it straight to
  the emitters (`0x1001ab02` -> `0x1003ab40`) and the time-scale easer. The particle system,
  client bullets and `FX_AdaptQuality` read TS's copy of `ref_params_t::frametime` (`0x10191478`)
  instead, which FWGS fills from the same `host.frametime`, so `effects` uses one value for both.

## Developer test commands

`ts_dev_fxlog 1` (port addition) makes the weapon events print a `[ts fx]` console line for each
predicted shot's first-person flash (position, units ahead of the eye and off the view axis,
distance from the view model's muzzle, and the dot product of the flash direction with the view
direction), each bullet trace (`bullet from ... to ...`), impact and decal. `cargo xtask play`'s
wall scene reads them.

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
  players during slow motion, and that the view model animates at the slowed rate (`ts_test_fx
  slowmo 0.3`, then fire or reload). Ghosts without a server: `cl_motionblur 2`, third person,
  `ts_test_fx slowmo 0.3`, then `ts_test_fx mblur` (the local player: 10 ghosts at `cl_fx 1`, 15 at `cl_fx 2`); the
  server sends `TSMBlur` on power-up use, stunts and kills.
* **Not tested in engine:** anything that needs the server to send TS state or events (this
  branch's server doesn't yet): other players' fire events, lasers/flashlights, `PTakeDam` from real
  hits, wind from `ts_wind`, local prediction firing (the server doesn't set `m_iId` 16 /
  `vuser*` yet), view-model muzzle flashes and shells (no view model is set by the server yet),
  dead camera, spectator views. On Steam GoldSrc only the scripted HL25 Linux run has been done
  (`docs/validation.md` §5): the view, view model and studio models draw once the client exposes
  `IClientVGUI` (issue #62, `client-hud.md` §7); the `cl_viewangles` write-back for recoil is
  confirmed on FWGS only (R§6).
