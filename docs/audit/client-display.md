# Client display audit, round 3 (3.0 + 2.1)

Round 3 audits the client's graphical display and GUI (model rendering, HUD and effects, VGUI menus) against 3.0's and 2.1's `client.dll`, function by function: draw order, positions, sizes, fonts, colours, alpha, sprites and models, timers and fades, cvars, rounding, gating, and the messages handled. The table format is [client.md](client.md)'s (port function | 3.0 original | 2.1 original | status). This round each group also fixed what it found, so the issues below are closed by commits on the group's branch. Decomp helpers: `scripts/round2/fn.sh client|ts21/client <addr>`, `dis.sh cl|cl21`, `rd21.py`.

## Model rendering

Scope: `crates/client/src/studio.rs` and `studio/{math,model}.rs` (`CStudioModelRenderer`), `effects/mblur.rs`, `effects/scope.rs`, `effects.rs` `studio_event`, the view-model block of `view.rs` `calc_normal`, and `crates/ts21/client/src/bullets.rs` `draw`. Neither earlier round covered the studio renderer (only `lod_body`).

2.1's renderer was located for this round. `HUD_GetStudioModelInterface` is 0x10043740: version 1, interface 0x10289fc4 = {1, `StudioDrawModel` 0x10065d50, `StudioDrawPlayer` 0x10066f60}, through the incremental-link thunks 0x10002c20 and 0x100022bb. The engine table is copied as 0xac bytes, 43 slots as in 3.0. `g_StudioRenderer` is 0x102e3168, and `CGameStudioModelRenderer`'s ctor is 0x10043640 (vtable 0x1026e7bc, base 0x102709cc). The vtable has the SDK's 25 slots:

| Slot | Function | Address |
|---|---|---|
| 0 | dtor | 0x10043670 |
| 1 | `Init` | 0x10062bf0 |
| 2 | `StudioDrawModel` | 0x10065d50 |
| 3 | `StudioDrawPlayer` | 0x10066f60 |
| 4 | `GetAnim` | 0x10063bd0 |
| 5 | `SetUpTransform` | 0x10063e50 |
| 6 | `SetupBones` | 0x10064ce0 |
| 7 | `CalcAttachments` | 0x10067fd0 |
| 8 | `SaveBones` | 0x10065820 |
| 9 | `MergeBones` | 0x10065930 |
| 10 | `EstimateInterpolant` | 0x10064520 |
| 11 | `EstimateFrame` | 0x10064b00 |
| 12 | `FxTransform` | 0x10064920 |
| 13 | `SlerpBones` | 0x10063a00 |
| 14 | `CalcBoneAdj` | 0x10062e70 |
| 15 | `CalcBoneQuaterion` | 0x10063210 |
| 16 | `CalcBonePosition` | 0x10063720 |
| 17 | `CalcRotations` | 0x100645d0 |
| 18 | `RenderModel` | 0x100680d0 |
| 19 | `RenderFinal` | 0x100684a0 |
| 20 | `RenderFinal` software | 0x100681d0 |
| 21 | `RenderFinal` hardware | 0x10068310 |
| 22 | `PlayerBlend` | 0x10063d00 |
| 23 | `EstimateGait` | 0x10066300 |
| 24 | `ProcessGait` | 0x10066960 |

Before this round, 2.1 mode ran 3.0's renderer with no hook. The renderer's ruleset choices are now `ClientRules::studio_style` (`StudioStyle`).

| Port function | 3.0 original | 2.1 original | Status |
|---|---|---|---|
| `studio::init` | `Init` 0x10030030 | 0x10062bf0 | match (`r_drawentities`; `cl_himodels`/`developer` unused in both) |
| `Renderer::draw_model` (normal path: header, transform, bbox and counters, merge/setup bones, save, events, lighting, `colormap` remap, render) | `StudioDrawModel` 0x10031ae0 | 0x10065d50 | match (port guard: a NULL model or header ends the draw) |
| `latch_view_model_rate`, `view_model_rate` | 0x10031b44..0x10031bf4 (×3.5555558 / ×1.7142859 / ×1.5, latch on `prevframe + 0.001 <`) | none | 3.0 match; **2.1 MISMATCH #256** (fixed: `view_model_slowmo`) |
| renderfx 17 dispatch: `draw_corpse` / `draw_ghost` / `draw_corpse_as_player` | 0x10031c9a.. (`rendercolor.r` player, `.b == 1` ghost; ghost gated on local + (not third person or `cl_motionblur < 2`) and `ts > 0.99`; ghost info from iuser1/iuser2/fuser3/fuser4) | 0x10065da0.. (always a corpse through slot 3 `StudioDrawPlayer`, `m_fDoInterp = 0`, live `player_info`) | 3.0 match; **2.1 MISMATCH #256** (fixed: `dead_player_copy`) |
| `Renderer::draw_player` | `StudioDrawPlayer` 0x10032540 (`renderamt = 255`; in-eye hide `g_iUser1 == 5`; gait branch saves and restores `angles`; colours clamped 0..360; `p_` model merged, entity restored) | 0x10066f60 (same flow, no `renderamt`) | 3.0 match; **2.1 MISMATCH #256** (fixed: `player_renderamt`) |
| `draw_dead_player` | `StudioDrawDeadPlayer` 0x10033540 | none | 3.0 match (no gait, no weapon, events, `renderframe` written) |
| `draw_ghost_player`, `turn_torso` | `StudioDrawGhostPlayer` 0x100337f0 (torso block 0x100338f2..0x10033a5a re-traced: ±120 flip, controller `(yaw ± 180)·0.25 + 30)·4.25`, `angles[1] = gaityaw` +360, `prevangles[1]`) | none | 3.0 match (no events, no `renderframe`, weapon drawn) |
| `estimate_gait` | `StudioEstimateGait` 0x10031fa0 (the ×0.75 test compares the x87 value left on the stack: the speed on the origin path, the movement on the velocity path, in both the moving and the stationary branches) | 0x10066300 (same: 2-D, ×0.75 below 180) | match (3.0 ×57.29577951308232 on the x87 vs 2.1 `×180/π`: below an ulp) |
| `process_gait` | `StudioProcessGait` 0x10032250 (pitch blend, torso, gait frame `framerate·fps·dt`, no range check) | 0x10066960 (`dt·fps·framerate`) | match (port guard: out-of-range sequences read as 0) |
| `setup_bones`, `calc_blended`, `split_torso` | `StudioSetupBones` 0x10031080 (yaw fix `a3[0] += a1[2] − a2[2]` re-traced in the asm 0x10031616..0x1003166a) | 0x10064ce0 (`_strcmp` split loop, no yaw fix) | 3.0 match; **2.1 MISMATCH #256** (fixed: `torso_yaw_fix`) |
| `set_up_transform` | 0x10030970 | 0x10063e50 | match |
| `estimate_interpolant` / `estimate_frame` | 0x10030c40 / 0x10030f80 | 0x10064520 / 0x10064b00 (decompile empty; not traced) | match (3.0 `×10` vs port `/0.1`, below an ulp) |
| `get_anim` | 0x100307e0 | 0x10063bd0 | match (SDK) |
| `fx_transform` | 0x10030e60 (15/16 distort, 18 explode; RNG order) | 0x10064920 | match |
| `slerp_bones` | 0x100306a0 | 0x10063a00 | match |
| `bone_adj` | 0x10030140 (RLOOP wrap indexes by controller number `j`; mouth `/64`) | 0x10062e70 | match (port guard: a negative controller index reads as the mouth) |
| `sample_pose`, `model::Anim::{span, rotation_samples, position_sample}` | 0x10030ca0, 0x10030380, 0x10030550 (RLE walk with the `total < valid` restart) | 0x100645d0, 0x10063210, 0x10063720 | match |
| `save_bones` / `merge_bones` | 0x100317b0 / 0x10031890 (`stricmp` 0x10076d90) | 0x10065820 / 0x10065930 (`stricmp` 0x100db540) | match |
| `calc_attachments` | 0x10034560 | 0x10067fd0 | match (port guard: draws 4 where TS calls `exit(-1)` above 4) |
| `render` (glow shell) | `StudioRenderModel` 0x100345f0 | 0x100680d0 | match |
| `render_final` | 0x10034860 → 0x10034690 / 0x10034750 (`r_drawentities` 2/3/4/5; hardware `trivial_accept = 0` with interpolation, `GL_StudioDrawShadow`) | 0x100684a0 → 0x100681d0 / 0x10068310 (hardware has no `5.0` case) | 3.0 match; **2.1 MISMATCH #256** (fixed: `hw_abs_bbox`) |
| `player_blend` | 0x10030890 | 0x10063d00 | match (3.0 `start × (double)⅓`, `((b − s)·255)/(e − s)` on the x87; port f32: below an ulp) |
| `studio::math` (`QuaternionAngles`, `MatrixAngles` gimbal test, `fpatan` argument order) | 0x1002ffe0, 0x1002fef0 | `QuaternionMatrix` 0x10062a00, `ConcatTransforms` 0x10062270 (no `QuaternionAngles`) | match |
| `effects::studio_event` | `HUD_StudioEvent` 0x10044b50 (5002 sparks; 5004 `EV_PlaySound` `CHAN_BODY`, 0.8, `RandomLong(-5, 5) + 25 − ftol(ts·−75)`) | 0x1007b780 (5004 `pfnPlaySoundByNameAtLocation(options, 1.0, attachment[0])`) | 3.0 match; **2.1 MISMATCH #257** (fixed: `slowed_event_sound`) |
| `mblur::MotionBlur::{add, due, spawned}`, `spawn_ghost`, `fade` | `MotionBlur_Add` 0x10029680, `_Update` 0x10029780, ghost think 0x10029710 (`fuser2 −= (1/max(ts)) · ft · 255`, `renderamt = ftol`, die ±1) | none (no `TSMBlur`; the 2.1 server sends none) | match |
| `effects/scope.rs` `draw` | 0x10054510 | 0x10094300 | match per round 2 (render mode 2, `TRI_QUADS`, colour 1, gates and size re-read here; the per-quad vertex order is not re-traced) |
| `view.rs` `calc_normal` view-model block, `calc_gun_angle` | 0x1006850e.. (push at 0x100685a7..0x100685bb), `V_CalcGunAngle` 0x100674a0 | 0x100b6c90 view-model block, 0x100b6610 | 3.0 match; **2.1 MISMATCH #258**: no `(80 − m_iFOV)·0.15` push along the view (fixed: `view_model_fov_push`) |
| ts21 `bullets::draw` (models, trail spin, spiral) | — | 0x10069540, add 0x10069450, trails 0x1007d8b0 / 0x1007dc70 / 0x1007de30 | match apart from #259. Gates (cone, bullet time or factor ≤ 0.1, models loaded, 31 slots). Model by ammo; angles `VectorAngles(dir)` with the pitch negated; draw step `dir·v/0.25·s·ft·10`; the trail time from `GetClientTime`. Buckshot trail every frame at 0.5/0.5. Next trail `slowdiv(0.003 or 0.01)`; spin `sin(spin·DEG) + 1.5` before and after `+max(90m, 22.5)`, wrapped past 360. `cl_wavedetail` picks the trails. Call-site argument order re-traced in the asm: trail `(draw_pos, trail, 0.5, 0.5)`, wave `(draw_pos, dir, w_new)`, spiral `(trail, draw_pos, angles, w_old, w_new)`. Spiral: `AngleVectors` right/up, 3 or 5 segments, `c = (360/n)·π/180` (double π), u split, v 0..1, colour 0.7, fade 0.035. **2.1 MISMATCH #259**: the add sets body 0 while the low-detail flag `0x1031b819` is set (fixed) |

### Not verified
- The 3.0 renderer's sub-ulp float forms aren't replicated. 3.0 multiplies by `0.003921569f`, `×5`, `×10` and `×⅓` and keeps values on the x87 stack; 2.1 and the port divide by 255, 0.2, 0.1 and 3 in f32 or f64. These change bone matrices in the last bit only; no byte output (blend, controller) was found to differ for model data in range.
- 2.1 `StudioEstimateFrame` (0x10064b00): the decompile is empty. Its size and SDK neighbours suggest the SDK's, but it wasn't traced in the asm.
- 2.1's `ev_punchangle` (0x10320fd0) is non-zero there: 2.1's kick (0x10096310, `0x1008b960`) writes it, and `V_CalcGunAngle` (0x100b6610) adds `±1.25 ×` it to the gun. The port's 2.1 view kick is 3.0's (the documented `apply_kick` deviation in ts21.md), so the gun's punch terms stay 0 in 2.1 mode too.
- `effects/scope.rs`: the per-quad vertex order through `0x10050000` (3.0) and its 2.1 twin was taken from round 2, not re-traced.
- `events::time_scale()` clamps to 0.01..1; 3.0's 5004 pitch reads the raw `g_flTimeScale`. They differ only for a `TSSlowMo` factor outside 1..100, which TS's server never sends (shared with the round-2 fire-pitch rows).
- The renderfx 17 corpse in 2.1 mode now writes the live player's `player_info` (`gaitsequence = 0`, `renderframe`), as 2.1 does. Corpses draw in the transparent pass after the opaque players, so this was not seen to change a frame. It wasn't checked in game.

## HUD and effects

Scope: `crates/client/src/hud/{mod,draw,message,saytext,statusbar,icons,geiger,text,notice,tfc}.rs` (the SDK HUD elements TS kept, HUD init/VidInit/sprite list, number drawing, redraw), `crates/client/src/effects/{psys,emitters,fx,laser}.rs` (the particle system) and `view.rs` `apply_kick`. Round 2 did `hud/state.rs`, `status.rs`, `weapons.rs`, the death notice, `CHudMenu`, `CHud::Redraw` and the builders' values (`client.md`).

Issues: **#260** (2.1 chat), **#261** (2.1 particle update), both fixed; **#267** (2.1 muzzle flash follow) filed, then found invalid (the builder is dead code, see below).

| Port function | 3.0 original | 2.1 original | Status |
|---|---|---|---|
| hud/message.rs `HudMessage::reset`, `x_position`, `y_position`, `fade_blend` | `Reset` 0x10028980, `XPosition` 0x10028a50, `YPosition` 0x10028ad0, `FadeBlend` 0x100289b0 | 0x10054ac0, 0x10054c00, 0x10054cc0, 0x10054b30 | match (2.1 stores `FadeBlend`'s result to a float first: at most one colour step, not filed) |
| `HudMessage::scan_start`, `scan_next_char`, `draw_scan` | `MessageScanStart` 0x10028d40, `MessageScanNextChar` 0x10028b60, `MessageDrawScan` 0x10028eb0 | 0x10055170, 0x10054d90, 0x10055420 | match (effects 0/1/2, the 10 % flicker `rand`, the 80-byte line buffer, signed `char` width index; 2.1 stores the write-out time to a float before comparing, not filed) |
| `HudMessage::draw`, `add`, `msg_hud_text`, `msg_game_title`, `TitleMsg::custom` | `Draw` 0x100290b0, `MessageAdd` 0x100293f0, `MsgFunc_HudText` 0x100295c0, `MsgFunc_GameTitle` 0x10029610 | 0x10055800, 0x10055dd0, 0x10056100, 0x10056190 | match (game title sprites, level-transition fix-up + 0.2, end times, the same-text and same-position checks, the custom message's values (2, 100s, (240, 110, 0), −1, 0.7, 0.01, 1.5, 5, 0.25)) |
| hud/saytext.rs `SayText::print`, `wrap`, `scroll_up`, `clear`, `draw_plan` | `SayTextPrint` 0x1002ed00, `EnsureTextFits..` 0x1002ee90, `ScrollTextUp` 0x1002ead0, `InitHUDData` 0x1002eaa0, `Draw` 0x1002eb40 | 0x10060890, 0x10060b10, 0x10060490, 0x10060410, 0x10060540 | match (five 256-byte lines, `Y_START` `h − 45/60 − 6 × line height`, wrap at `w − 40`, name 64/63, `hud_saytext_time` "5") |
| hud/mod.rs `say_text_print` (mute check), `draw_say_text` (plain line colour) | 0x1002ed00 (`IsPlayerBlocked` 0x10018c80), 0x1002ec72 (`g_ColorYellow`) | 0x10060890 (no mute check), 0x10060771 (no colour) | 3.0 match; **2.1 MISMATCH #260, fixed** (`say_text_style`) |
| hud/statusbar.rs `StatusBar` (`reset`, `parse`, `draw`, `StatusText`, `StatusValue`) | `Reset` 0x1002f430, `ParseStatusString` 0x1002f480, `Draw` 0x1002f6f0, 0x1002f850, 0x1002f8d0, `Init` 0x1002f3c0 | 0x100614c0, 0x10061590, 0x10061ae0, 0x10061d80, 0x10061e60, 0x10061410 | match (`h − 55/45`, `hud_centerid` "0" flags 1, `%p`/`%i`/`%%`, `******`, line colour (1, 0.7, 0) = 2.1's 0x10289f9c) |
| hud/icons.rs `StatusIcons::reset`, `enable`, `disable`, `draw` | 0x1002f110, 0x1002f210, 0x1002f330, 0x1002f130 | 0x10060ef0, 0x10061120, 0x10061320, 0x10060f40 | match (4 icons, `stricmp` reuse, `grenade` → `weapons/timer.wav` (6, 1.0, 0.8, 0, 100), `y −= h + 5` from `h/2` at x 5) |
| hud/geiger.rs `draw` | `CHudGeiger::Draw` 0x1001f4e0, `MsgFunc_Geiger` 0x1001f4a0 | 0x100438e0, 0x10043870 | match (ranges, chances, scales, `rand` count; 2.1 divides by 255.0 where 3.0 multiplies by the float 1/255: under 1e-8, not filed) |
| hud/mod.rs `draw_train`, `Train` | `CHudTrain::Draw` 0x10034d40, `MsgFunc_Train` 0x10034df0 | 0x10068d00 (`UnpackRGB(0xffa000)`), 0x10068e20 | match |
| hud/text.rs `localise_text_string`, `buffered_localise`, `lookup_string`; hud/state.rs `TextMsg` | `LocaliseTextString` 0x100348d0, 0x100349a0, `LookupString` 0x100349c0, `MsgFunc_TextMsg` 0x10034a60 | 0x10068580, 0x10068750, 0x10068790, 0x100688f0 | match (the SDK's `size − steps − 1` terminator in both, 1 notify (`\x01`) / 2 console / 3 chat (`SayTextPrint(.., 128, −1)`) / 4 centre print) |
| hud/notice.rs `client_color` | `GetClientColor` 0x1001af40 | 0x1003fe10 | match (white local, teams 1..4 colours identical) |
| hud/tfc.rs (`SecAmmoVal`/`SecAmmoIcon`, `ValClass`, `Feign`, `Detpack`, `BuildSt`, `RandomPC`, `AllowSpec`) | `CHud::Init` 0x10020170, 0x1001a5e0 | 0x10045d50, 0x1003d6b0 | match (the same hooks; values only stored) |
| hud/mod.rs `register_cvars`, the message/command list | `CHud::Init` 0x10020170 | 0x10045d50 | match, except 3.0's `gl_fog*` cvars (2.1 has none; see Not verified) |
| hud/mod.rs `vid_init`; hud/draw.rs `HudSprites::load`, `font_height` | `CHud::VidInit` 0x100206c0 | 0x10046610 | match (res 320/640, `sprites/hud.txt`, `sprites/%s.spr`, `m_iFontHeight` from `number_0`, the element VidInits). 2.1 doesn't reset the time scale 0x100963e8 (it has none; its hooks use the raw factor) |
| hud/draw.rs `draw_number`, `number_cells` | `DrawHudNumberTS` 0x1004c290 | 0x10088620 | match (the 10000s place without `% 10`, reserve masks, leading zeros) |
| effects/psys.rs `PSys::init`, `add`, `sprite` | `PSys_Init` 0x100507e0, `PSys_Add` 0x10050b60, `PSys_LoadSprite` 0x100509d0 | 0x1008fa90 (256..2048), 0x1008ffc0 (no drag floor), 0x1008fd20 (32 slots, keyed by name) | 3.0 match; **2.1 MISMATCH #261, fixed** (`particle_model`) |
| `PSys::update_draw`, `particle_move`, `collide`, `draw_one`, `build_quad` | `PSys_UpdateDraw` 0x100520d0, `Particle_Move` 0x10050ce0, `Particle_Collide` 0x10051d50, `Quad_Build` 0x10050080 | 0x10091020, 0x100902f0, 0x10090a00, 0x1008f470 | 3.0 match (gravity `dt·g·0.1·cl_gravity` TS units re-checked at 0x10050e0c). **2.1 MISMATCH #261, fixed**: `effects/psys/decel.rs` (`ParticleModel::Decelerate`): gravity ×0.1 and positive only, 330 u/s² deceleration toward 33 u/s, no drag/think, its own collision and floor stop, life on the raw factor (step `0x10091920`), no fade-in, stretch or axis quads. 2.1's quad build and view cone (0x100968f0) are 3.0's |
| (2.1 particle follow field `+0xba`/`+0xbb`, think callback `+0xd0`) | none | 0x100902f0, 0x10091020; set only by 0x1007fe50 (attachment-following flash) and 0x10083b30 (concrete impact: `debris_concrete3.spr` chips, collision callback 0x10083590 a `gun_smoke_add.spr` puff, a smouldering chip with think 0x10083740) | no effect: neither builder is called, jumped to or referenced (thunks 0x10002d65, 0x1000405c unused; the fire events use 0x10080a30, the impacts 0x10084240). #267 invalid |
| effects/emitters.rs (map emitters, `ts_fog`) | `CTS_CL_Emitter` 0x1003ab40.. | none (0x1004a500 reads only `info_player_start`) | match: off in 2.1 through `map_effects` (round 2); the emitter code itself was not re-audited this round |
| effects/fx.rs builders, effects/laser.rs | see `client.md` | see `client.md` | round 2 (values per ruleset through `fx_style`, `laser_style`, `tsfrags_effects`, `blood_impact`); they now run on 2.1's particle update in 2.1 mode |
| view.rs `apply_kick` | `V_ApplyKick` 0x10057ed0 | 0x10097de0 (from 0x100b53c0's `V_CalcNormalRefdef`) | match: the same ease (`min(ft·15, 1)`), 0.01 settle test (double) and zeroing; 2.1's debug build stores the differences and the pitch step to floats (at most one ulp of the kick), not filed. 2.1 adds only the kick to `cl_viewangles` (no view-shake step, no bob pitch), which the port gets from its hooks (no shake from 2.1's weapons, `Bob::pitch` 0) |

### Not verified
- The `gl_fog`/`gl_fog_*` cvars and the single-player user fog (`effects.rs` `fog_begin`) exist only in 3.0; whether 2.1 mode should register them was left to the rendering group.
- ~~`SpriteTexture` with a frame past the last (2.1 doesn't clamp it): GoldSrc and FWGS pick different frames; the port passes the frame through as 2.1 does.~~ Round 4: **#270**, fixed. GoldSrc's `R_GetSpriteFrame` (`hw.so` 0x143420) binds frame 0 for a frame below 0 or past the last, and FWGS binds the last frame. `tri::sprite_texture` now asks both engines for GoldSrc's frame. 3.0 never passes one: it clamps particles (0x10052333), and the laser beam wraps before `frames`. Left as is: GoldSrc binds nothing for a grouped frame (`SPR_GROUP`) where FWGS animates the group. That needs the engine's private sprite layout to detect, and a grouped particle sprite is a mapper's asset question, not a frame index.
- `effects/emitters.rs` and the 3.0 builders in `effects/fx.rs` were not re-compared against 3.0 this round (round 1/2 rows stand).

## GUI

Scope: `crates/client/src/menus/{mod,buy,motd,team,scoreboard,scroll,widgets}.rs`, `hud/vgui.rs`, `spectator/{panel,overview}.rs`, and the GUI hooks in `ts30-client`/`ts21-client`. Issues: **#262, #263, #266** (2.1), **#265** (3.0), **#264, #268** (both). All are fixed in this round.

| Port function | 3.0 original | 2.1 original | Status |
|---|---|---|---|
| menus/mod.rs `draw_buy` dispatch (team page alone, else summary + page, children before parents) | 0x10036750 | 0x1006b5b0 | match |
| `Column::row`/`row_scaled` (127×19 box at `ftol(50·scale)`, text `ftol(c·scale)`, `DrawHudString` at the row, y += 20) | 0x1004fed0, 0x1004ff10 | 0x1008f200, 0x1008f270 (unlocalised text) | match |
| team page ("Change Team:" green, two `chgteam` rows at HUD/2, cancel (100,100,205)) | 0x100367e0 | 0x1006b6a0 | match |
| main page (Empty/Use Last/Use New config red/green by `cash − total < 0`, rows 2..7, cancel only on page 0; scale 1.0/0.5) | 0x10037540 | 0x1006ca30 ("7 Buy Random") | match |
| category page (`table[0][i] != −1`, chosen category full brightness `0x1003788c`, "6 Player Accessory" 3.0 always / 2.1 `m_Teamplay > 1`) | 0x100377d0 | 0x1006cde0 (32×32 table, stride 0x80) | match |
| weapons page (`%c` row ≥ 9 → 'A' / 2.1 `%i`; affordable = cost and slots; chosen weapon full brightness `0x10037ad4`; weapon box at the chosen row) | 0x10037960 | 0x1006d070 | match |
| options page (Buy, attachments 900/800/500/1800 with own price, akimbo / full load, back) | 0x10037ba0 | 0x1006d470 (pending weapon at `+0xc`) | match |
| items page (Kevlar 1900 / Stealth Shoes 700; 3.0 money+slots, 2.1 money only) | 0x10037fe0 | 0x1006db50 | match |
| remove / save / load pages | 0x100373c0, 0x10037260 | 0x1006c780, 0x1006c540 | match |
| `draw_weapon_box` (fill 0xff×0x2f alpha 0x19 at (0x108, h/2−0x30), sprite, credits/slots at +128) | 0x10038180 | 0x1006ddd0 | match |
| `draw_summary` (bar 0x280×0x13, header red at `cash − total ≤ 0`, free slots, 5 buckets, sprites, numbers flags 2/5, attachments, akimbo second gun) | 0x10036af0 | 0x1006b9a0 | 3.0 match; **2.1 MISMATCH #262** (item names HUD/2, never red), fixed (`MenuRules::summary_items_dim`) |
| `attachment_summary` | 0x100368f0 | 0x1006b830 | match |
| buy.rs `handle_key` (all pages) | 0x10038840 | 0x1006e830 (keys 1..10; pending weapon reset to 0, not −1: unobservable) | match |
| `hit_test` | 0x10038410 | 0x1006e1b0 (hole message) | match |
| `mouse_buttons`/`click` (left release: hit or key 1; right release: key 10) | 0x10038360 | 0x1006e080 | match |
| `add`, `remove`, `recompute_ammo`, `add_ammo_buy` | 0x10038f60, 0x10038e40, 0x100392b0, 0x10039380 | 0x1006f5b0, 0x1006f340, 0x1006fcb0, 0x1006fe30 | match |
| `submit` (sort, `rst_cfg`/`tkw`/`tki`/`tka` order, messages) | 0x10039400 | 0x1006ff20 (strings 0x1026b144..) | match |
| `random_config` (melee, pistol/SMG/rifle branches, RNG call order), `random_attachments` | 0x100398e0, 0x10039810 | 0x10070860, 0x10070720 | **MISMATCH #264 (both)**: `p` against `double` bounds; fixed |
| `open_buy`/`BuyMenu::open` | 0x10036670 (demo refused) | 0x1006b3f0 (no demo check) | 3.0 match; **2.1 MISMATCH #263**, fixed (`MenuRules::refused_in_demo`) |
| `BuyMenu::close`, `reset_config` | 0x10036720, 0x10038800 | 0x1006b530, inline `memset` | match |
| `restricted`, `cash`, `SLOTS` | 0x100365e0, 0x10036640, 0x10036660 | 0x1006b330 (ids 1..63, no Golden Colts case: no menu id is affected), 0x1006b390, 0x1006b3d0 | match |
| team.rs `TeamMenu::new` (constants 40/32, 176/80/424/312, 16/16, scroll 16/48/424−32/178, briefing 424−16×178, buttons 40/80+32i/124×24, info 16/234/424−16/424−234, colours, fonts, "Auto assign", `#Menu_Cancel`) | 0x100655d0 | 0x100b2ca0 | match (same constants and structure). 2.1 divides by 640/480 and spills nothing: **2.1 MISMATCH #266**, fixed (`ClientRules::vgui_scale`) |
| team.rs `update`, `paint` (`paintBackground` → `Update` while the map name is unset, black at alpha 100) | 0x10066020, 0x10066640 | 0x100b3eb0, 0x100b4940 → 0x1003a480 | match (round 2 compared `Update`'s logic; this round its geometry) |
| `Open`, `Reset` | 0x10066610, 0x10066730 | 0x100b4900, 0x100b4b60 | match |
| motd.rs `layout`, `Motd::new` (window 112/80/424/312, title at +16/+16 "Title Font" then `sf_primary1`, scroll +16/+48, −32, −(48+60), OK +16 / bottom−16−30, 160×30; shade 100/255) | 0x1005bb90 | 0x100a0ad0 | match in structure; **2.1 MISMATCH #266** (at 640×480 the scroll panel is 204 high and the OK button at y 346 in 2.1, where 3.0 has 203 and 345), fixed |
| `map_briefing`, `for_motd` (`CreateTextWindow` 1/3: titles to 31 characters, `maps/<m_sMapName>.txt` or the level's, `m_sMapName` from the level) | 0x10063c00 | 0x100af6c0 | match |
| scoreboard.rs `layout` (insets 104/40, 512/400 cases, title 27, columns `cw·XRES`, last column `wide − XRES(4) − end`, header `wide − 2·XRES(4)`) | 0x10063a50, 0x1005ce70 | 0x100a2d60 | match in structure; **2.1 MISMATCH #266** (last column 1 wider), fixed |
| scoreboard update timer (`m_fNextUpdateTime = time + 0.5` as a float) | 0x100649aa | 0x100b1210 | match |
| menus `paint_vgui` (viewport `paintBackground`: scoreboard cursor/update, team check, spectator panel key) | 0x100648c0 | 0x100b1210 | **MISMATCH #268 (both)**: no `Update` on a local team change; fixed |
| spectator/panel.rs `draw` (bar `YRES(40)` at `h − YRES(40)`, black alpha 100, "Primary Button Text" `YRES(25)` and "Briefing Text" `YRES(15)` labels, `a_north`) | 0x10064410 | 0x100b04d0 | **3.0 MISMATCH #265** (computed in `f32`, 440 at 480; TS 439), fixed; 2.1 divides (440): #266 |
| panel `update`/`texts`/`think` (`#Spec_Mode%d [: name]`, `#Directed `, `#Spec_TSHelp%d`/`#Spec_Only_Help`, key mode/target/nohud) | 0x10063870 | 0x100af0c0 | match (the `IsSpectateOnly() != 2` test can't fail on these engines) |
| hud/vgui.rs `xs`/`ys`/`spill` | `* 0x1007d43c`, `* 0x1007d438`, spills | `/ 0x1026f548`, `/ 0x1026f544`, no spills | was 3.0 for both rulesets; now per ruleset (#266) |
| vgui.rs `Schemes::load` (resolution table, `gfx\vgui\fonts\%d_%s.tga`, font sharing by name/size/weight, defaults) | 0x1005c300 | 0x100a16e0 | match in structure (SDK code; both clients load the same Half-Life fonts and schemes) |
| spectator/overview.rs `draw_entities` name-tag projection (`(x+1)·w·0.5`, `(1−y)·h·0.5`, offset (32, 32), z of the offset −0, tag y + length) | 0x10024160 | 2.1's own (`ts21-client` spectator, #181) | match. The second projection's off-screen result: TS uses it anyway, while the port takes offset 0 (a tag at the icon's own y in that rare case; not filed, HLTV only) |

### Not verified
- Overview layer tiling (`DrawOverviewLayer` 0x10023c90) and the entity quads/beam: structure only. Ghidra's output for 0x10024160 is garbled, and only the name-tag store was traced in the disassembly. The port computes the tag projection in `f32` where TS keeps 53-bit before storing a `float`, which can differ only in the last bit.
- Scroll bars (`menus/scroll.rs`, `CTFScrollPanel`) and `CommandButton` painting (`widgets.rs`): not compared against 2.1 (SDK code in both).
- Voice labels (`voice::redraw`): not in this group's comparison.
- `jointeam` setting `g_iTeamNumber` locally (`0x10064a82`) is still not emulated (documented in client-hud.md). #268's team check reads the player state.
- 2.1's VGUI scaling was brute-forced over every width 320..3840 and height 240..2400 for the MOTD expressions, and at the common resolutions for the rest. Expressions not compared operation by operation (the scoreboard's `FillGrid` row heights) take the same `vgui_scale`.
