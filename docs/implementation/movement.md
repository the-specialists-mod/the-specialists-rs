# Movement: player movement, stunts, player animation

Work package `movement`. Sources of truth: decomp `docs/server/player-movement.md` (§1-3, §6.6,
§7), the decompiled functions in `server/decomp/global_000f1168.c` (PM), `CTSStunt.c`,
`CTSAnimation.c`, `CBasePlayer.c` (PreThink/PostThink/Jump/Duck), and the Half-Life SDK
`pm_shared.c` for the functions TS left unchanged. Addresses are server Ghidra addresses.

| Area | Files |
|---|---|
| Shared player movement (server `PM_Move` and client prediction) | `crates/shared/src/pm/` (`mod.rs` core, `ts.rs` TS hooks, `sound.rs` footsteps, `textures.rs` materials, `math.rs` mathlib, `fpu.rs` the two builds' arithmetic, `ext.rs` x87 extended precision, `tests.rs`) |
| Server stunts (`CTSStunt`) and the stunt parts of `CBasePlayer` | `crates/server/src/stunt/mod.rs` (state, hooks, helpers), `moves.rs` |
| Player animation (`CTSAnimation`) | `crates/server/src/stunt/anim.rs` |
| Studio header access (`LookupSequence`, sequence info) | `crates/goldsrc/src/ext/movement.rs` |
| Stunt message layouts/decoders | `crates/shared/src/protocol/stunt.rs` |

## Design

The stunt **triggers stay on the server**, exactly as in TS: `CTSStunt` sets `pev->iuser4` bits
and `pev->gravity/friction/view_ofs`; `shared::pm` consumes the bits on both sides (`TSHandle`).
The decomp (§9.1) suggests moving the triggers into PM for better prediction; that would change
what the client must predict, so an original TS client would mispredict our server (and our
client an original server). Keeping TS's split is what the wire-compatibility contract needs.
The cost is TS's: one-shot impulses reach client prediction one round trip late.

`shared::pm` is a straight port of TS's `pm_shared.c` (all SDK features plus the TS changes),
one implementation for both libraries. Global C state (`OldrotArray`, stuck tables, texture
table, `iSkipStep`) is per library, as in C.

**Floating point is each TS build's, bit for bit.** TS compiled `pm_shared.c` twice, and the
two builds disagree in the last bits, so the port carries both (`pm/fpu.rs`): `Pm<F>` is generic
over the build, every function names its operations in machine-code order (`F::x` a `float`
operand, `F::xd` a `double`, `F::xi` an `fild`, `.f()` the `fstp dword`) and branches on
`F::MSVC` where the two machine codes differ. Both copies are monomorphised; the caller picks
one with `shared::pm::Arith`:

| Side | TS binary it stands in for | `Arith` |
|---|---|---|
| Server, Linux (and any non-Windows build) | `ts_i386.so` | `Arith::SERVER` = `Gcc` |
| Server, Windows | `mp.dll` | `Arith::SERVER` = `Msvc` |
| Client prediction (`HUD_PlayerMove`), every platform | `client.dll` (TS's only client) | `Arith::CLIENT` = `Msvc` |

* **`ts_i386.so` (`Gcc`)**, GCC 2.95, evaluates each expression on the x87 in extended
  precision (64-bit significand; Linux leaves the FPU at that) and rounds to `float` only where
  it stores: `float` locals (the build spills every one to memory), `pmove` fields, `float`
  arguments. Double constants (`0.001`, `0.333`, `0.8`, `0.65`, `0.7`, `0.1`, ...) stay doubles;
  `(int)` casts truncate. `pm/ext.rs` implements that arithmetic in software (`Ext`, checked
  against the FPU itself by `ext_matches_x87_hardware`). `f64` isn't enough: it rounds twice, and
  the cases where that matters come up in play, e.g. `0.001 * 600 <= 0.6` (`PM_Duck`) is false in
  extended precision, so with 10 ms frames the server's duck completes at 590 ms.
* **`client.dll` and `mp.dll` (`Msvc`)**, MSVC 6, also use the x87, but at **53-bit precision**:
  each DLL's CRT calls `_controlfp(_PC_53, _MCW_PC)` when it loads (`_fpmath`: client
  0x1006accc → 0x1006d7c7, mp.dll 0x1007d2c0 → 0x1007ea5a), and nothing changes it afterwards: TS
  only switches the control word inside CRT math helpers that restore it, Steam's `hw.dll` and
  `swds.dll` only push 64-bit precision around `QueryPerformanceCounter` in `Sys_FloatTime`
  (Quake's `Sys_HighFPPrecision`/`Sys_LowFPPrecision` are in `hw.dll` with no callers), and FWGS
  never touches it. At 53 bits each add, sub, mul, div and sqrt rounds exactly like an IEEE
  `double` (`x87_at_53_bits_is_f64` checks it on the FPU), so the client runs on native `f64`.
  The duck test above is true there: TS's client finishes the duck at 600 ms, a frame before
  its Linux server. MSVC also keeps `float` locals on the FPU stack across statements (both
  `Length` and `VectorNormalize` return the root unrounded), reassociates products, sums most
  `DotProduct`s from z down, uses `float` constants in places, and its compares send a NaN the
  other way in many tests.
* **`mp.dll` is `client.dll`'s code.** Every movement function in the two DLLs is the same
  instruction sequence (addresses aside) except `PM_PlayerMove` (the client clears three
  spectator-camera globals; the floating point is the same), `PM_Move` (the client caches
  `physents[0].model`) and `PM_SpectatorMove` (the client's is its own; its roaming part is
  mp.dll's, instruction for instruction). So the Windows server build follows `mp.dll`, where
  an original TS client agrees with it, and the Linux build follows `ts_i386.so`, as TS's own
  Linux server did.

The MSVC differences, function by function (client address; `mp.dll` is the same):

| Function | What `Msvc` does differently from `Gcc` |
|---|---|
| `AngleVectors` 0x10029bb0 | radians not rounded before `fsin`/`fcos`; `cos(yaw)` stays on the stack (64-bit) for every product that uses it; `right`/`up` factored as `cr*sy - (sr*sp)*cy`, `((sr*sp)*sy + cr*cy) * -1`, `(cr*sp)*cy + sr*sy`, `(cr*sp)*sy - sr*cy` |
| `Length` 0x10058390, `VectorNormalize` 0x10029e40 | root returned unrounded; `VectorNormalize` skips a NaN length |
| `PM_ClipVelocity` 0x1002b190 | dot summed y, z, x; the ±0.1 test reads the stored component |
| `PM_CalcRoll` 0x1002e120 | dot summed z, y, x; `side` stored before `fabs` |
| `PM_DropPunchAngle` 0x1002e1c0 | unrounded length; `× fuser1` before `× frametime`; clamp on the unrounded result (NaN and -0 become +0) |
| `PM_Accelerate` 0x1002b830 | dot z, y, x; `((pmove->friction × frametime) × wishspeed) × accel` kept unrounded (the speed factor multiplies the stored copy); velocity stepped with the unrounded value |
| `PM_AirAccelerate` 0x1002bab0 | dot z, y, x; same product order; clamped to the stored add speed |
| `PM_Friction` 0x1002b910 | drop `(friction × pmove->friction × 0.75) × (control × frametime)` not stored; scale stored after the x component used it; NaN speed clamps to 0 |
| `PM_FlyMove` 0x1002b310 | dots z, x, y and z, y, x; `fraction > 0` ordered; `friction != 1` as an integer compare |
| `PM_WalkMove` 0x10053fd0 | `Length` < 1 unrounded; NaN step distances keep the down move |
| `PM_WaterMove` 0x1002bb70 | lengths unrounded; `(mvfric × pfric × frametime) × speed` and `accel × pfric × frametime × wishspeed`; `wishvel[2] - 60` from the stored value |
| `PM_CheckWater` 0x1002bfe0 | half height added without storing |
| `PM_CheckStuck` 0x1002c340 | `Sys_FloatTime()` compared as a double, stored after |
| `PM_SpectatorMove` mp 0x10038d20 (client roaming 0x1002cd6b) | speeds and friction unrounded, `friction × (control × frametime)`, dot z, y, x, clamp to the stored add speed, `wishvel[2]` reloaded before `+ upmove` |
| `PM_NoClip` 0x1002daf0 | `wishvel[2]` reloaded before `+ upmove` |
| `PM_LadderMove` 0x1002d1b0 | dot z, y, x |
| `PM_PreventMegaBunnyJumping` 0x1002dba0 | `maxspeed × 1.7` tested unrounded; divides by the unrounded speed |
| `PseudoClimb` 0x10052fd0 | eye height `fild(40 or 0) + view_ofs + origin`; speed test unrounded |
| `TSHandle` 0x10053660 | prone ×0.3 with a `double` 0.3 |
| `PM_PlayStepSound` 0x1002a3a0 / `PM_UpdateStepSound` 0x1002aca0 | speed tests on the unrounded `Length`; `_ftol` keeps the low dword of a 64-bit `fistp` |
| NaN-only compare directions | `PM_PlayerMove` water-jump cancel, `PM_CheckParamters` (maxspeed clamp, glide gravity), `PM_CheckVelocity` (too low), `PM_Jump`, `PM_WaterJump`, `PM_CheckWaterJump`, `PM_PushEntity`, `PM_Physics_Toss` (`VectorCompare` counts NaN as equal), `PM_UpdateStepSound` |

The rest (`CrossProduct`, `VectorMA`, `VectorScale`, `PM_SplineFraction`, the gravity functions,
`PM_ReduceTimers`, `PM_GetAccelBySpeed`, `PM_CatagorizePosition`, `PM_Duck`, `PM_UnDuck`,
`PM_FixPlayerCrouchStuck`, `PM_CheckFalling`, `PM_CreateStuckTable`, `TSDive`, `TSWallJump`,
`TSWallJumpClimb`, `TSGoDuck`, `TSStandUp`, `GetChaseOrigin`, `PM_CatagorizeTextureType`) computes
the same as the GCC build once the stack is 53 bits wide.

## Implemented functions

### shared::pm

| TS function | Address | Rust |
|---|---|---|
| `PM_Move` | 0x101188 | `pm::player_move` (sets `FL_ONGROUND`, SP friction reset) |
| `PM_PlayerMove` | 0x1008d8 | `Pm::player_move`, `Pm::walk_physics` (an unknown movetype prints TS's "Bogus pmove player movetype" developer message) |
| `PM_Init` / `PM_CreateStuckTable` / `PM_InitTextureTypes` | 0x101280 / 0x100d74 / 0xfc050 | `pm::init`, `create_stuck_table`, `textures::load` |
| `PM_FindTextureType` / `PM_SortTextures` | 0xfc28c / 0xfbfc8 | `pm::find_texture_type`, `textures::find`, `textures::parse` |
| `PM_CheckParamters` | 0xf9c94 | `Pm::check_parameters` |
| `PM_DropPunchAngle` / `PM_CalcRoll` / `PM_ReduceTimers` | 0x100760 / 0x1006d4 / 0x1007f8 | `drop_punch_angle`, `calc_roll`, `reduce_timers` |
| `TSHandle` | 0xf96a0 | `Pm::ts_handle` (`pm/ts.rs`) |
| `TSDive` / `TSWallJump` / `TSWallJumpClimb` | 0xf9534 / 0xf9444 / 0xf94e0 | `ts_dive`, `ts_wall_jump`, `ts_wall_jump_climb` |
| `TSGoDuck` / `TSStandUp` / `PseudoClimb` | 0xf93a8 / 0xf91fc / 0xf8fc8 | `ts_go_duck`, `ts_stand_up`, `pseudo_climb` |
| `PM_GetAccelBySpeed` | 0xfdcc4 | `accel_by_speed_x` (unrounded), `pm::accel_by_speed` |
| `PM_Accelerate` / `PM_AirAccelerate` / `PM_Friction` | 0xfdd08 / 0xfdfdc / 0xfde08 | `accelerate`, `air_accelerate`, `friction` |
| `PM_WalkMove` / `PM_AirMove` / `PM_FlyMove` / `PM_ClipVelocity` | 0xfa26c / 0xfe434 / 0xfd408 / 0xfd210 | `walk_move`, `air_move`, `fly_move`, `clip_velocity` |
| `PM_AddCorrectGravity` / `PM_FixupGravityVelocity` / `PM_AddGravity` / `PM_CheckVelocity` | 0xfd304 / 0xfd390 / 0xffbb0 / 0xfd0b8 | same names |
| `PM_Jump` / `PM_PreventMegaBunnyJumping` | 0xf9944 / 0x1000d4 | `jump`, `prevent_mega_bunny_jumping` |
| `PM_Duck` / `PM_UnDuck` / `PM_FixPlayerCrouchStuck` / `PM_SplineFraction` | 0xf9f7c / 0xff51c / 0xff41c / 0xff3ec | `duck`, `unduck`, `fix_player_crouch_stuck`, `spline_fraction` |
| `PM_CatagorizePosition` / `PM_CheckStuck` (+ stuck offsets) | 0xfe760 / 0xfe920 | `categorize_position`, `check_stuck` |
| `PM_CheckWater` / `PM_WaterMove` / `PM_WaterJump` / `PM_CheckWaterJump` / `PM_PlayWaterSounds` | 0xfe594 / 0xfe0b8 / 0xffaf8 / 0x100160 / 0x1005ac | same names |
| `PM_Ladder` / `PM_LadderMove` | 0xff9fc / 0xff654 | `ladder`, `ladder_move` |
| `PM_CheckFalling` | 0x1003d0 | `check_falling` |
| `PM_SpectatorMove` / `GetChaseOrigin` | 0xfef3c / 0xfee5c | `spectator_move`, `chase_origin` |
| `PM_NoClip` / `PM_Physics_Toss` / `PM_PushEntity` / `PM_AddToTouched` | 0x10000c / 0xffce8 / 0xffc24 / 0xfcfdc | same names |
| `PM_UpdateStepSound` / `PM_PlayStepSound` / `PM_MapTextureTypeStepType` / `PM_CatagorizeTextureType` | 0xfcc20 / 0xfc344 / 0xfca90 / 0xfcb40 | `sound.rs`, `categorize_texture_type` |
| `AngleVectors` / `VectorMA` / `VectorNormalize` / `Length` / `CrossProduct` / `VectorScale` | 0xfb430 / 0xfbbc4 / 0xfbcf8 / 0xfbcbc / 0xfbc80 / 0xfbd68 | `pm/math.rs` |
| `FixVelocity` (empty), `CanJumpHigh` (no callers), debug `PM_Draw*` | | not ported (dead) |

### Server stunts (`stunt/moves.rs`, `stunt/mod.rs`)

| TS function | Address | Rust |
|---|---|---|
| `CTSStunt::Init` | 0x99fbc | `StuntState::default`, `stunt::on_spawn` |
| `CTSStunt::Play` | 0x9a014 | `moves::play` |
| `CanDiveHigh/Medium/Low`, `SlotBonus`, `NextDive` | 0x9a494-0x9a59c | `can_dive_*` (used slots < 42/52/62, The One bypasses), `NEXT_DIVE` = 0 |
| `GoBulletTime` / `GoWallJump` / `GoWallClimb` / `GoDive` | 0x9a5b4 / 0x9a7d4 / 0x9aa1c / 0x9b1a0 | `go_bullet_time`, `go_wall_jump`, `go_wall_climb`, `go_dive` |
| `CheckForWallJump` / `CheckForWallDive` / `CheckForWallClimb` | 0x9ac14 / 0x9ad44 / 0x9ae6c | `check_for_wall_jump`, `check_for_wall_dive`, `check_for_wall_climb` |
| `HandleStunt` / `HandleDive` / `Landing` / `HandleWheel` / `HandleSlide` / `HandleWallJump` / `HandleWallClimb` | 0x9b000 ... 0x9b9f8 | same names (snake case); wall landings share `land_wall_stunt` |
| `GoWheel` / `GoRoll` | 0x9c9bc / 0x9cfe8 | `go_wheel`, `go_roll` |
| `TEXTURETYPE_PlaySound` (roll with `FREEZE_ANGLES`, dead in TS) | 0xe1330 | `moves::texture_type_play_sound` |
| `TEXTURETYPE_Init` / `LoadStandardMaterials` / `LoadMapMaterials` / `Find` | 0xe1244 / 0xe0e14 / 0xe1014 / 0xe12c4 | `textures::texturetype::{init, find}` (`init` from `stunt::precache`, as TS's `CWorld::Precache`) |
| `CheckForBreakables` | 0x9bd9c | `check_for_breakables` |
| `CBasePlayer::Jump` / `Duck` | 0x92864 / 0x927fc | `moves::jump`, `moves::duck` |
| `CBasePlayer::PreThink` (stunt parts) | 0x908cc | `stunt::pre_think` (not during intermission: `Player::pre_think` stops after `rules::player_think`, as TS returns on `g_fGameOver`): button edges, `framerate = slowmul`, last-ground time, close-combat/block timers, `Play`, `UpdateTimers`, `NO_VIEW` bit, `BlockPlayer` expiry, breakables, `Jump()`/`Duck()`, `m_flFallVelocity`, `UpdatePlayerState` + `TSState` |
| `CBasePlayer::PostThink` (stunt parts) | 0x91610 | `stunt::post_think`: frozen angles, fall damage, gravity reset, super-jump landing, `Animate(1/6)`, `StudioFrameAdvance`, `m_afButtonLast`; only while alive and not in intermission (`Player::post_think` skips it otherwise, as TS does) |
| `CBasePlayer::BlockPlayer` | 0x95b58 | `stunt::block_player` |
| `UpdatePlayerState` | 0x92a08 | in `stunt::pre_think` |
| `GetDirection` | 0x6b008 | `stunt::get_direction` |
| `EMIT_SOUND_DYN` (TS: pitch × time scale) | 0xe0a8c | `entities::stock::explode::emit_sound_slow` |
| `UTIL_ScreenShake` (TS: ÷ receiver time scale) | 0xec978 | `stunt::screen_shake` |

### Animation (`stunt/anim.rs`)

| TS function | Address | Rust |
|---|---|---|
| `CTSAnimation::CTSAnimation` / `Init` | 0x6b0c8 / 0x6b320 | name tables as consts, `AnimState::default` |
| `UpdateTimers` | 0x6b37c | `update_timers` |
| `Animate` | 0x6b470 | `animate` (priorities), `torso` (switch), `death`, `gait` |
| `SetAnim` | 0x6d7e4 | `set_anim` |
| `BlendAnimationsByDir` / `BlendAnimationsForSlide` | 0x6d538 / 0x6d400 | `blend_by_dir`, `blend_for_slide` |
| `HandleCloseCombat` / `IsInCloseCombat` / `GaitCloseCombat` | 0x6d8d0 / 0x6dcf4 / 0x6dd14 | same names |
| `BulletDance` / `Enable/DisableBulletDance` | 0x6d338 / 0x6d314 / 0x6d2f0 | same names |
| `CBaseAnimating::LookupSequence` / `ResetSequenceInfo` / `StudioFrameAdvance` (SDK) | | `goldsrc::ext::movement::lookup_sequence`, `anim::reset_sequence_info`, `anim::studio_frame_advance` |

`BlendJumpAnimations` (0x6d630) has no callers in the server and isn't ported.

## Constants

| What | Value |
|---|---|
| Jump | `vel.z = 236.64319` (`sqrt(2*800*35)`, 35 units; SDK 268.33/45) |
| Super jump (physinfo `mj` = 1, ducked or `bInDuck`, `IN_DUCK` held, `flDuckTime > 0`) | `vel.z = 400`, `iuser4 |= 0x200000` |
| Glide (`0x100000`) | cmd ×(0.5, 0.25, 0.25); gravity 0.3 rising / 0.75 falling with jump held, else 1.0 |
| Friction | `friction * pmove->friction * 0.75 * control * frametime`; edge friction unchanged |
| Accel factor | `maxspeed >= 330 ? 1.1 : (maxspeed-210)*0.008333334*0.5+0.6`, not while prone or ducked |
| Backpedal | `forwardmove *= 0.8` in `PM_WalkMove` |
| Time scale | `frametime = (float)(msec*0.001)*fuser1`, 0 if `fuser1 < 0.1`; timers use raw msec; step interval `(int)((1/max(fuser1, 0.1)) * t)` (truncated); punch decay ×`fuser1` again |
| View heights | stand 28, duck 16 (transition `(1-f)*28 + (16-18)*f`, `TIME_TO_DUCK` 0.4), dead -8 |
| Dives (`TSDive`) | 0x100 → (90, 25) z 200; 0x40 → (90, 50) z 282.84; 0x80 → (100, 10) z 126.49; `vel.xy += speed*normalize(vel)`, wall dive uses `forward*3` |
| Wall jump / climb | `vel.xy = -forward.xy*210`, `vel.z = 236.64319` / `vel.z = 334.664` |
| PseudoClimb | pitch within ±60; eye trace 64 u (+40 above when pitch < -30), feet trace 32 u, needs `|vel| < 0.5*maxspeed`, adds `frametime*1600` |
| Ladder | flat 200 u/s, jump-off `normal*270` |
| Falling | punch threshold 350, safe 680 (fallpain3 above), 0.85 vol above 340, bounce 200 |
| Wall bounce (`PM_FlyMove`, walking with `onground == -1` or `friction != 1`) | overbounce `1 + (1 - friction) * 0.01`: TS hardcodes 0.01 where the SDK reads `movevars->bounce`, and reflects off every plane hit this move (no SDK `numplanes == 1` test) |
| Footsteps | silent while `iuser4 & (0x1 | 0x200)`, attenuation 1.25, no `mp_footsteps` check |
| Materials | `sound/materials.txt`, ≤ 1024 entries (TS file has ~1140: the tail is ignored, as in TS), 15 significant chars |
| cmd lockouts | zeroed while `iuser4 & 0x10c10`; jump blocked (not latched) by `0x30c34`; `PM_Duck` off for `0x30c3c` |
| Stunt slots | used slots < 42 high, < 52 medium, < 62 low |
| Stunt timers | coyote 0.7/slowmul, taps 0.3 s, bullet time 0.55 (cooldown, prone lock, dodge), recover: roll 0.75, dive roll-out 0.5625, flip 0.375, wall landings 0.2625, get-up 0.75, super-jump landing 0.75 |
| Stunt physics on the entity | dive gravity 0.75 (low: friction 0.25; too heavy: gravity 1, friction 0.9), flips gravity 0.75 (forward cartwheel friction 0.25), roll friction 0.3, roll-out/wall landing friction 0.5, flip landing 0.25 |
| Wall checks | 48 u forward (jump) / behind (dive), 54 u forward non-player (climb); `-0.5 < n.z < 0.5`, `vel.z > -370` |
| Wall chain | +0.35 per wall jump; +0.3/0.5/0.75 per climb (high/med/low ability); next stunt `max(0.5, chain)` |
| Motion blur | `RandomLong(0,10) + 10*slowmul > T`: dive 8, flips 7, wall jump 5, roll 9; bullet time always |
| Fall damage | above 680: `(v-680)*100*0.0022522523`, bodysplat if fatal, `DMG_FALL|DMG_NEVERGIB` from the world |
| Super-jump landing | `UTIL_ScreenShake(origin, 20, 30, 0.3, 512)`; duck held → forced roll, else `superjump-land.wav`, `RECOVER|GO_DUCK`, view 16, recover 0.75, anim 0x3a |
| Breakables | 64 u head-hull trace along velocity in the air; `|vel|` damage (`|vel|*0.1` while alive, as TS); with a melee window ±32 u along `v_forward`, damage `|pending|`, halved per hit (0 below 5), knockback above 30 (`+|vel|*dir`, z 235) |

## Deviations from TS 3.0

* **Limits of the floating-point emulation.** `Gcc` assumes the FPU at 64-bit precision, round
  to nearest (the Linux default; nothing in the server changes it), `Msvc` the 53 bits the MSVC
  CRT sets (see Design). `fsin`/`fcos` go through `f64`: GCC rounds both to `float` at once, but
  MSVC's `AngleVectors` multiplies by the 64-bit `fcos(yaw)`, where `f64` carries 53 bits, so a
  component can in principle land one `float` ulp away (`msvc_angle_vectors_match_client_dll`
  runs client.dll's instructions on the FPU: none of 180 000 components differ). `Ext`'s
  exponent range is unbounded (matters beyond ±1e4900), and `f64`'s is narrower than the x87's
  (beyond ±1e308); movement never gets near either. MSVC copies many floats with `fld`/`fstp`,
  which quiets a signalling NaN; the port copies bits.
* **Original TS client against the Linux server.** The port's client predicts like TS's client
  (`Msvc`), so against a Linux server (the port's or TS's) it mispredicts in the last bits
  exactly as TS's own client did, e.g. a duck finishes a frame early. Against a Windows server
  both sides agree.
* **Spectator locked view (`iuser1 == 4`).** TS reads three stack slots it never writes as the
  camera angles: the outgoing arguments `PM_CheckParamters` last pushed (`PM_CalcRoll`'s
  `v_angle` pointer, `pmove->velocity` pointer and `rollangle`, or only the
  `PM_DropPunchAngle(punchangle)` pointer while dead or `DEAD_LOCK`). The port reproduces that
  (`STALE_ARGS`): the roll is `sv_rollangle` exactly, pitch and yaw are the low 32 bits of the
  equivalent Rust addresses as floats. Address values can't match (they depend on the process
  layout in TS too), and when only the first slot is rewritten the other two keep the port's
  last values, where TS's depend on whatever the engine left on that stack. The server never
  sets mode 4 for players.
* **Defensive bounds (only matter on invalid engine input).** `numtouch`, `numphysent`,
  `nummoveent` and `PM_CheckStuck`'s `hitent` are clamped before indexing (TS trusts the
  engine). A null `PM_TestPlayerPosition`/`PM_PlaySound`/`PM_TraceLine`/`PM_TraceModel`/print
  callback is tolerated (only the unit tests lack them): `GetChaseOrigin` then returns the
  unclipped camera spot and `PM_LadderMove` sees a default trace. `TEXTURETYPE_PlaySound`
  clamps its random sample index.
* **Debug aids (not in TS):** cvar `ts_debug_stunt` (logs `iuser4` changes with origin, velocity,
  sequences, timers, and the rise of each airborne phase) and, only while it is set, the client
  commands `ts_debug_alt1` (simulates holding `+alt1` for 3 frames) and
  `ts_debug_face <pitch> <yaw>`. `PM_Init` prints how many material types it loaded
  (`developer 1`).

## Not done / owned elsewhere

* **Jump kick (`CTSStunt::CloseAttack` 0x9c630)**: needs `CBasePlayer::CloseCombatAttack`, the
  melee bar and kung fu (weapons). `moves::jump` is where it plugs in: TS calls it first and
  skips `GoWheel` if it hit.
* **Melee details in `CheckForBreakables`** with an attack window: the block checks
  (`VictimKungfuHold/KnifeHold/KatanaHold`), disarm (`DropWeapon(-2, dmg)`), the melee-bar cost
  of the backflip kick (22.5). (`glast_closecombat` is set around the kung-fu hit:
  `weapons::melee::pending_hit`, `damage::glast`.) The team check approximates `PlayerRelationship` with `mp_friendlyfire` and
  `Player::team`.
* **Super-jump roll landing** also sets `m_flNextAttack = 0.75` (weapons timer): not applied.
* `SetPlayerGravity` (temp gravity timer, pl+0x4ec), `InterpolateSlowMotion`, power-ups,
  `GoSlow` (rules).
* **The client's own `PM_SpectatorMove` (0x1002c790) outside roaming** lives in
  `crates/client/src/spectator/` and still computes in `f32` with Rust's compares. Against
  client.dll: the locked view's blocked test is `fraction != 1.0` where TS uses `eq87`
  (`movement.rs`, asm 0x1002ca94); `VectorAngles` (0x10029e90) takes `fpatan`'s result
  unrounded into `× 57.29577951308232` (a `double`) and tests `eq87`, where `render.rs`'s
  `vector_angles` rounds each step to `f32`; `InterpolateAngles` (0x10029cf0) keeps the
  difference unrounded and tests `-180` with C0, and `NormalizeAngles` (0x10029cb0) is
  `a > 180 ? a - 360 : a < -180 ? a + 360 : a` in doubles with no `%` (`spectator/mod.rs`
  `interpolate_angles`, `render.rs` `norm_angle`). Its `GetChaseOrigin` and roaming part already
  use `Msvc` through `shared::pm`.

## Tests

* `cargo test -p shared`: PM fake-world tests (`pm/tests.rs`, floor at z=0, wall at
  x=200, hull-aware traces): falling/landing, walk to maxspeed and stop at a wall, backpedal
  256 u/s at 320, **jump height 35 ± 0.6**, **friction ×0.75 exact**, edge friction, **accel
  factor** (0.6/0.85/1.1 and one-frame acceleration, none while ducked), **time scale** (frozen
  below 0.1; 20 frames at 0.5 = 10 at 1.0), step interval ÷ time scale, **dive impulses**
  (all three, bits consumed, duck hull), wall dive, wall jump/climb, stunt jump lockout without
  latching, **duck view 16** (transition, hull, origin, unduck), super jump needs `mj`, glide
  gravity, `DEAD_LOCK` yaw cache, cmd clamp only for spectators, stuck table layout,
  **duck completes at 590 ms** (extended-precision compare) on the server build and **600 ms**
  on the client build, locked-view stale angles, jump
  constants as rounded roots; the software extended precision against the x87 FPU (100 000
  random operations and `fstps`) and against `f32` for single operations; **the 53-bit x87
  against `f64`** (`x87_at_53_bits_is_f64`, 100 000 operations at control word 0x27f);
  **client.dll's own instructions on the FPU** against `Pm<Msvc>`: `AngleVectors` (180 000
  components, all equal) and the `PM_Friction` tail (20 000 velocities, all equal, some of them
  different from the GCC build); `_ftol` vs `fistpl` casts; a 1 500-command scripted run
  (walk, turn, strafe into a wall, jumps, ducks, dives, wall jump, prone, slow motion, low
  friction, glide, falling, roaming spectator) hashed for each build, so neither drifts
  (`server_trajectory_is_unchanged` was taken before the movement became generic); the
  `TEXTURETYPE` table (unsorted, first match); plus
  texture-table parsing/sorting/lookup, `AngleVectors`, and message decoders.
  `cargo test -p goldsrc --features server`: studio header lookup on a synthetic header.
* Builds: `cargo build -p ts-server -p ts-client` (native), `--target i686-unknown-linux-gnu`,
  `--target i686-pc-windows-gnu`: all compile (Windows prints the pre-existing stdcall-fixup
  linker note).
* Dedicated server (`cargo xtask server fwgs --map ts_awaken --bots 0 --port 27200`): PASS.
* FWGS listen server with a scripted client (`autotest_mv.cfg`, `ts_debug_stunt 1`) on
  `ts_awaken`, checked from the debug trace:
  * standing jump: apex 34.99 units above the floor;
  * level dive: `0x14` (+ impulse), `vel = (0, 410, 191)` (320 + 90, 200 - gravity steps),
    `dive_f_akimbo`; lands into a slide (`0x24`, `slide_f_akimbo`); jump gets up (`0x5`,
    `slide_f_end`, recover 0.75), stands at 28 when it expires;
  * pitch -30 dive: z 274 (282.84 high); pitch +20 dive: `(0, 420, 117)` (low);
  * dive with duck held: roll-out `0x40005`, `dive_f_end`, recover 0.56;
  * roll (duck double-tap at 320 u/s): `0x40005`, `roll_f`, recover 0.75;
  * jump double-tap standing: back flip `0x404`, `wheel_b`, z 191, lands into recover 0.375;
    running: cartwheel `0x804`, `lowwheel_f`, lands prone (`slide_b_end` on get-up);
  * stunt key facing a wall: wall jump `0x10004`, `vel = (±210, 0, 228)`, `walljump_end`;
    jump double-tap at a wall: wall climb `0x20004`, z 323, `walljump_climb`; both land into
    recover 0.2625;
  * `cl_showerror 1` printed no prediction errors (listen server, so latency ~0);
  * `PM_Init` loaded 1024 material types on both the server and the client.
* **Not tested in-engine:** bullet time (needs slow motion from rules), super jump (needs `mj`
  from a power-up), ladders, water, PseudoClimb, breakables, fall damage, the original TS client
  against this server, network latency effects on impulse prediction.

## Integration notes

* **iuser4 / fuser1** are sent by `stunt::fill_client_data`. The player's `gravity` and
  `friction` reach the client through `entity_state` (net.rs already sends them for players),
  `view_ofs`, `maxspeed`, `bInDuck`, `flDuckTime`, physinfo through clientdata.
* **rules** must: ease `pev->fuser1` toward `Player::slow_target` (stunts, animation and PM run
  on it); set `pev->iuser4 = DEAD_LOCK` (0x2) in `Killed` (TS sets `iuser4 = 2`, clearing every
  stunt bit) and call `stunt::animate(p, cx, 7)` after setting `p.stunt.anim.last_damage`,
  `last_hitgroup`, `attack_dir`; arm the bullet dance in `TakeDamage`
  (`stunt::anim::enable_`/`disable_bullet_dance`) and call `stunt::anim::bullet_dance` when a
  dead body is shot (both wired in `player/damage.rs`); while dying, run
  `stunt::anim::studio_frame_advance` until `anim.seq_finished` (`death_think`); set `p.stunt.the_one`; set `Player::superjump` (the physinfo `mj` key is
  synced from it every PreThink); apply ×0.7 to damage while `stunt::in_airborne_stunt(cx)` and to
  bullets while `cx.time() < p.stunt.dodge_until`; count kills during `in_airborne_stunt` (or
  `glast_closecombat`, `damage::glast`) as spectacular.
* **rules or weapons** must set the move speed (`SetPlayerSpeed`: `set_client_maxspeed` with
  `shared::ts::speed_for_free_slots`, 330 at spawn): PM's accel factor depends on it.
* **weapons** must: keep `Player::slots_used` current (stunt ability); call
  `stunt::set_anim_ext(p, ext, ext2)` on deploy (`ext2`: "onehanded", "twohanded", "akimbo",
  "onehanded_case"; `ext`: idle-gait override like "knife_idle", usually ""); call
  `stunt::animate` with 0/8/9 (fire), 0x33/0x34 (reload/draw), 0x35 (grenade), 0x26 (block),
  10-39 (melee); arm delayed hits through `p.stunt.melee` (window, damage, kind) — they land in
  `CheckForBreakables`; use `p.stunt.close_attack_cd` for the jump kick; use
  `stunt::block_player` for the post-attack slowdown; respect TS's "no attack while
  `iuser4 & 0x30c30`".
* **client (rendering/HUD)**: `NextStunt` and `TSState` are hooked in `hud::state`, `TSMBlur` in
  `effects`, and `+alt1` sets `shared::ts::IN_ALT1` (`input`). The view during stunts is done
  (`client/src/view.rs`): the server sets `view_ofs = 0` (`NO_VIEW`), and TS's normal view just
  follows that view height, easing the eye down to the hull centre at rate 15 (the view model is
  placed at that eye, so it follows); the third-person camera pulls back to -92 for the stunt
  bits `0x130c30`. `NO_VIEW` itself is read only by `V_GetPlayerEye` (`0x10068e20`, in-eye
  spectating: the eye stays at the target's origin instead of +28/+16).
* `stunt::screen_shake` is the TS slow-motion-aware `UTIL_ScreenShake`; the stunt sounds use
  the shared `explode::emit_sound_slow` (TS `EMIT_SOUND_DYN`).
* `stunt::precache` precaches the stunt sounds and every PM footstep/wade/ladder sound.
