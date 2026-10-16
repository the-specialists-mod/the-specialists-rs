# Client display audit, round 3 (3.0 + 2.1)

Round 3 audits the client's graphical display against 3.0's and 2.1's `client.dll`, function by function: draw order, positions, sizes, fonts, colours, alpha, sprites and models, timers and fades, cvars, rounding, gating, and the messages handled. The table format is [client.md](client.md)'s (port function | 3.0 original | 2.1 original | status). This round each group also fixed what it found, so the issues below are closed by commits on the group's branch. Decomp helpers: `scripts/round2/fn.sh client|ts21/client <addr>`, `dis.sh cl|cl21`, `rd21.py`.

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
