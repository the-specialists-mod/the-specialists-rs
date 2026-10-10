# Movement: player movement, stunts, player animation

Work package `movement`. Sources of truth: decomp `docs/server/player-movement.md` (§1-3, §6.6,
§7), the decompiled functions in `server/decomp/global_000f1168.c` (PM), `CTSStunt.c`,
`CTSAnimation.c`, `CBasePlayer.c` (PreThink/PostThink/Jump/Duck), and the Half-Life SDK
`pm_shared.c` for the functions TS left unchanged. Addresses are server Ghidra addresses.

| Area | Files |
|---|---|
| Shared player movement (server `PM_Move` and client prediction) | `crates/shared/src/pm/` (`mod.rs` core, `ts.rs` TS hooks, `sound.rs` footsteps, `textures.rs` materials, `math.rs`, `tests.rs`) |
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
one implementation for both libraries. Math is `f32` in the C operation order. Global C state
(`OldrotArray`, stuck tables, texture table, `iSkipStep`) is per library, as in C.

## Implemented functions

### shared::pm

| TS function | Address | Rust |
|---|---|---|
| `PM_Move` | 0x101188 | `pm::player_move` (sets `FL_ONGROUND`, SP friction reset) |
| `PM_PlayerMove` | 0x1008d8 | `Pm::player_move`, `Pm::walk_physics` |
| `PM_Init` / `PM_CreateStuckTable` / `PM_InitTextureTypes` | 0x101280 / 0x100d74 / 0xfc050 | `pm::init`, `create_stuck_table`, `textures::load` |
| `PM_FindTextureType` / `PM_SortTextures` | 0xfc28c / 0xfbfc8 | `pm::find_texture_type`, `textures::find`, `textures::parse` |
| `PM_CheckParamters` | 0xf9c94 | `Pm::check_parameters` |
| `PM_DropPunchAngle` / `PM_CalcRoll` / `PM_ReduceTimers` | 0x100760 / 0x1006d4 / 0x1007f8 | `drop_punch_angle`, `calc_roll`, `reduce_timers` |
| `TSHandle` | 0xf96a0 | `Pm::ts_handle` (`pm/ts.rs`) |
| `TSDive` / `TSWallJump` / `TSWallJumpClimb` | 0xf9534 / 0xf9444 / 0xf94e0 | `ts_dive`, `ts_wall_jump`, `ts_wall_jump_climb` |
| `TSGoDuck` / `TSStandUp` / `PseudoClimb` | 0xf93a8 / 0xf91fc / 0xf8fc8 | `ts_go_duck`, `ts_stand_up`, `pseudo_climb` |
| `PM_GetAccelBySpeed` | 0xfdcc4 | `pm::accel_by_speed` |
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
| Friction | `control * friction * pmove->friction * 0.75 * frametime`; edge friction unchanged |
| Accel factor | `maxspeed >= 330 ? 1.1 : (maxspeed-210)*0.008333334*0.5+0.6`, not while prone or ducked |
| Backpedal | `forwardmove *= 0.8` in `PM_WalkMove` |
| Time scale | `frametime = msec*0.001*fuser1`, 0 if `fuser1 < 0.1`; timers use raw msec; step interval `round(t / max(fuser1, 0.1))`; punch decay ×`fuser1` again |
| View heights | stand 28, duck 16 (transition `(1-f)*28 + (16-18)*f`, `TIME_TO_DUCK` 0.4), dead -8 |
| Dives (`TSDive`) | 0x100 → (90, 25) z 200; 0x40 → (90, 50) z 282.84; 0x80 → (100, 10) z 126.49; `vel.xy += speed*normalize(vel)`, wall dive uses `forward*3` |
| Wall jump / climb | `vel.xy = -forward.xy*210`, `vel.z = 236.64319` / `vel.z = 334.664` |
| PseudoClimb | pitch within ±60; eye trace 64 u (+40 above when pitch < -30), feet trace 32 u, needs `|vel| < 0.5*maxspeed`, adds `frametime*1600` |
| Ladder | flat 200 u/s, jump-off `normal*270` |
| Falling | punch threshold 350, safe 680 (fallpain3 above), 0.85 vol above 340, bounce 200 |
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

* **x87 precision.** TS keeps some intermediates in 80-bit registers; this port is `f32`
  throughout, in the same operation order. Expect last-bit differences, not behavioural ones.
* **Spectator in-eye mode (`iuser1 == 4`).** The decompiled `PM_SpectatorMove` leaves the angles
  uninitialised in this mode; the port uses the target's angles.
* **Defensive bounds.** `player_index`, `numtouch`, `numphysent`, `nummoveent` are clamped
  before indexing (TS trusts the engine). `PM_TestPlayerPosition`/`PM_PlaySound`/... being null
  is tolerated (only matters for the unit tests).
* **`TEXTURETYPE_PlaySound` on a roll with `FREEZE_ANGLES`.** No TS code sets `0x80000`, so the
  branch is dead; only `land.wav` is ported.
* **`PFLAG_DUCKING`** (`m_afPhysicsFlags & 8`) isn't tracked here; `Duck()` is called on
  `IN_DUCK` or `FL_DUCKING`, which covers the cases that reach it.
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
  of the backflip kick (22.5), and `glast_closecombat` (passed as damage bit `0x4000000`
  instead). The team check approximates `PlayerRelationship` with `mp_friendlyfire` and
  `Player::team`.
* **Super-jump roll landing** also sets `m_flNextAttack = 0.75` (weapons timer): not applied.
* `SetPlayerGravity` (temp gravity timer, pl+0x4ec), `InterpolateSlowMotion`, power-ups,
  `GoSlow` (rules).
* The HUD hooks for `NextStunt`, `TSState`, `TSMBlur` (HUD/rendering packages; decoders in
  `shared::protocol::stunt`).

## Tests

* `cargo test -p shared`: 27 tests pass. PM fake-world tests (`pm/tests.rs`, floor at z=0, wall at
  x=200, hull-aware traces): falling/landing, walk to maxspeed and stop at a wall, backpedal
  256 u/s at 320, **jump height 35 ± 0.6**, **friction ×0.75 exact**, edge friction, **accel
  factor** (0.6/0.85/1.1 and one-frame acceleration, none while ducked), **time scale** (frozen
  below 0.1; 20 frames at 0.5 = 10 at 1.0), step interval ÷ time scale, **dive impulses**
  (all three, bits consumed, duck hull), wall dive, wall jump/climb, stunt jump lockout without
  latching, **duck view 16** (transition, hull, origin, unduck), super jump needs `mj`, glide
  gravity, `DEAD_LOCK` yaw cache, cmd clamp only for spectators, stuck table layout; plus
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
  damage bit `0x4000000`) as spectacular.
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
* **client (rendering/HUD)** must: hook `NextStunt`, `TSState`, `TSMBlur`; bind `+alt1` to
  `shared::ts::IN_ALT1`. The view during stunts is done (`client/src/view.rs`): the server sets
  `view_ofs = 0` (`NO_VIEW`), and TS's normal view just follows that view height, easing the eye
  down to the hull centre at rate 15; the third-person camera pulls back to -92 for the stunt
  bits `0x130c30`. `NO_VIEW` itself is read only by `V_GetPlayerEye` (`0x10068e20`, in-eye
  spectating: the eye stays at the target's origin instead of +28/+16).
* `stunt::screen_shake` is the TS slow-motion-aware `UTIL_ScreenShake`; the stunt sounds use
  the shared `explode::emit_sound_slow` (TS `EMIT_SOUND_DYN`).
* `stunt::precache` precaches the stunt sounds and every PM footstep/wade/ladder sound.
