# Movement + stunt audit (area: movement), round 2

Port commit 661d918 (after #200 phase 1); validated against main 5032209 (#201 and #198 merged since: they touch the gun's
PreThink weapon step and the plugin callbacks only; `stunt/mod.rs` changed in a comment). 3.0 = `ts_i386.so` Ghidra address
(objdump + 0x10000) unless marked mp.dll / cl / "objdump"; 2.1 = ts21 `mp.dll` VA (client: ts21 `client.dll`).
Helpers: scripts/round2/{fn.sh,dis.sh,rd21.py} (rd21.py reads a PE's .rdata constants), and full objdump listings of the five binaries.
2.1 animation ids: 2.1's ids from 0x30 up are the port's minus one (2.1 0x30 = port 0x31 ... 2.1 0x38 = port 0x39, 2.1 0x34 = port 0x35);
2.1 0x19..0x22 are knife1..kungfu_block (the port plays 3.0's ids, documented). 2.1's glide bit 0x80000 is the port's 0x100000
(so 2.1's mask 0xb0c30 = port 0x130c30).

Status column: **R2** = re-checked by the validator against the decompile/disassembly (verdict given); rows without R2 are the
auditor's (or round 1's where it says so) and were not re-done in validation.

## Issues filed (round 2)
- #205 Audit 2.1: the client's own `PM_SpectatorMove` (crates/client/src/spectator/movement.rs:103) copies the in-eye pitch; 2.1's
  client.dll (FUN_1005c820, `iuser1 == 5`: `angles[0] * -2.0`) multiplies it by -2 like the server hook (#145 fixed only the server).
  2.1's normal view (0x100b6c90) applies those camera angles, and so does the port's (view.rs:1138), so in-eye observing players in
  2.1 mode see the wrong pitch.
- #206 Audit 2.1: the game-side material table (`texturetype::init/find`, textures.rs:196) is 3.0's (1024/15 chars, map
  `_materials.txt`); 2.1 `TEXTURETYPE_Init` FUN_10036580 / `TEXTURETYPE_Find` 0x10036760: 512 entries, 12 chars, only
  `sound/materials.txt`, linear first match. Used in 2.1 only by `CFuncTankGun::Fire` (2.1 `GoRoll` has no texture sound). #200 item
  `TextureLimits::LARGE`.
- #222 Audit 2.1 (low): 2.1 `Animate`'s velocity directions use the out-of-line `Vector::Normalize` 0x10011fc0, which stores `1/len`
  as a float (port `normalized`, anim.rs:476/688/703/788, keeps it unrounded as 3.0 mp.dll 0x100148f0 does); `dive_dir`
  (moves.rs:804) sums z,x,y (3.0 mp.dll), 2.1 0x1005c7c8 sums x,y,z with IEEE zero/NaN tests.
- #226 Audit 2.1 (low): `GoWallJump` 0x1005b320 tests the 10 s cap with the unrounded chain sum but computes the value from the
  stored float chain (0x1005b473..0x1005b494); port `Absolute::wall_chain_cooldown` uses the unrounded sum for both, and its test
  and ts21.md say so. (3.0 mp.dll 0x10068c9b does use the unrounded sum: correct for 3.0.)
- #227 Audit 3.0 (low, both rulesets): fall damage (moves.rs:752) is `(v-680)*100*0.0022522523` in f32 (two roundings); 3.0 and
  2.1 mp.dll (0x10060238 / 0x10054297, identical) do `(v-680f) * 0.22522523f` rounded once and test the unrounded product
  against health; the .so multiplies by 100f and 0.0022522523f in extended and stores once.

Filed by the validation (see "Validation"):
- #242 Audit 2.1 (low): kung-fu `BlockPlayer` lock: 2.1 (0x1004e98f..0x1004e9a7) passes `(float)(slowInv * delay * 0.9f)` and
  `BlockPlayer` 0x10056b60 adds the time as a float sum; port ts21 melee.rs:532 `(delay as f64 * 0.9) as f32` (double 0.9), then
  the core clock (`block_player` stunt/mod.rs:267 / weapons/host.rs:87, `Absolute::start`) multiplies by slowInv and rounds once.
- #243 Audit 3.0 + 2.1 (low): `studio_frame_advance` (anim.rs:191) rounds `interval`, `* frame_rate`, `* framerate` and the `+`
  in f32; the .so (0x935a0, objdump 0x93621..0x93632), 3.0 mp.dll 0x100082f6.. and 2.1 mp.dll 0x100060f6.. add
  `framerate * m_flFrameRate * interval` (interval unrounded) to the frame and store once. Round 1 marked it match.
- #246 Audit 3.0 (low): `maybe_blur` (stunt/mod.rs:181) computes `slowmul*10 + r` in f32 (the .so keeps it extended, objdump
  0x8b388..; mp.dll stores the product as float and adds r unrounded, 0x10069670..); the 3.0 coyote test (`StuntClock::
  air_stunt_allowed` default, rules.rs:187) uses `0.7f32 / slowmul` in f32 where both builds divide the double 0.7 on the x87
  and compare with the double difference (.so objdump 0x8b12b.., mp.dll 0x10069419..). Edge time-scale values only.

Round 1 issues re-checked: #124 (NextStunt truncation: .so objdump 0x8a929 loads RC=truncate, mp.dll `__ftol`; fix verified), #125
(2.1 kick order Arm, Draw, Hit: verified against 0x1005cd00), #126 (prone ×0.3 double), #145 (texture SDK limits, in-eye ×-2,
CalcRoll, Friction: server side verified by disasm; client half missed -> #205), #105 (MsvcIeee), #112 (2.1 Animate priority:
verified case by case, below). All match except as listed.

## Coverage
| Port fn | 3.0 original | 2.1 original | Status |
|---|---|---|---|
| ts30 `PmRules` bodies (crates/ts30/src/pm.rs) | as round 1 (moved unchanged from the pre-#200 core: drop_punch_angle, step_interval, scaled_frametime, speed_by_slots compared with c49ee1a) | n/a | match |
| ts21 `PmRules` (crates/ts21/src/shared/pm.rs) | n/a | round 1 + new hooks: `ieee_compares`, `texture_limits` SDK, `in_eye_pitch` (0x100325d0 fmul -2.0f, mode 5), `roll_divides_first` (0x10033af1 full disasm), `friction_edge_by_reciprocal` (0x100316b0 full disasm) | match |
| ts21 `fix_velocity` (FixVelocity) | n/a (3.0's is empty, 0xf8fc4) | 0x10057d10 | match. **R2**: disasm: `Length` 0x1002fbf0 unrounded, `!= 0` IEEE on the unrounded length, `stored > limit`, ratio `fst` float, x by the stack ratio, y by the stored one |
| ts21 `jump_push` / `push_along` / `normalize` + core `pm::jump` in 2.1 | — | PM_Jump 0x10058680, VectorNormalize 0x1002fc10 | match. **R2**: disasm 0x10058680..0x10058a43: dead/stunt-mask 0x34·0x30c00 returns, waterjump/swim tests IEEE, onground/oldbuttons, PreventMega then step sound then `mj` (bInDuck/FL_DUCKING, IN_DUCK, `flDuckTime > 0`), high: push 100f then z = sqrt(160000.0); normal: buttons & 0x600 push + FixVelocity(maxspeed), z = sqrt(56000.0) = 0x436ca4a8 = `JUMP_VELOCITY`; normalize keeps `1/len` on the stack, IEEE zero test |
| ts21 `drop_punch_angle` | — | 0x10033b10 | match. **R2**: disasm: len stored float, `fuser1 ? fuser1 : 1.0` (IEEE), `len - (0.5len+10)*frametime*scale`, `> 0` else 0 (NaN 0), VectorScale |
| ts21 `step_interval` | — | 0x10030dad | match. **R2**: `fuser1 == 0` (IEEE) skips, else `__ftol(interval / fuser1)` |
| ts21 `scaled_frametime` | — | PM_PlayerMove 0x10033cc2..0x10033d0c | match. **R2**: `msec * 0.001` double stored; `fuser1 != 0` then `fuser1 < 0.1f ? 0 : fuser1` (`test ah,5; jp`: NaN keeps fuser1; the decompile shows it inverted) |
| ts21 `speed_by_slots`, `max_player_speed` | — | 0x100436e0, 0x100557b0 | match (round 1) |
| ts30 `drop_punch_angle` / `step_interval` / `speed_by_slots` helpers | 0x100760 / 0xfe2d4 / 0x76bd4 | n/a | match (round 1, unchanged code) |
| pm::player_move_with (MsvcIeee dispatch) | — | #105 | match |
| pm::Pm::player_move / walk_physics | PM_PlayerMove 0x1008d8 | 0x10033c90 | match (round 1). **R2** (2.1 head): CheckParamters, numtouch, frametime, scale, ReduceTimers, AngleVectors, spectator branch, CheckStuck, Catagorize, fall velocity: same order |
| pm::check_parameters | PM_CheckParamters 0xf9c94 | 0x10058a60 | match. **R2** (2.1 decompile + disasm 0x10058bfc): maxspeed min, spectator-only ratio clamp, zeroing on FL_FROZEN·FL_ONTRAIN / dead / iuser4 0x10 / 0x10c00 (= port `AIRBORNE_MASK` 0x10c10), glide ×0.5/0.25/0.25, gravity 1 / `vz < 0` 0.75 else 0.3 (NaN 0.3: decompile shows `0 <= vz`, disasm `test ah,5; jp`), DropPunch, CalcRoll×4 unless dead or iuser4&2, OLDROT, dead view -8, yaw wrap |
| pm::reduce_timers | PM_ReduceTimers 0x1007f8 | 3.0 mp.dll instruction diff (round 1) | match (round 1) |
| pm::accelerate | PM_Accelerate 0xfdd08 | 0x100315f0 | match. **R2** (2.1 disasm): dead / waterjump IEEE, addspeed dot z,y,x, `<= 0` on the unrounded value then stored, `friction*frametime*wishspeed*accel` (×0.7 double unless PRONE (0x20) or FL_DUCKING), clamp to the stored addspeed (NaN kept), `vel += a*dir` |
| pm::air_accelerate | 0xfdfdc | 3.0 mp.dll instruction diff (round 1) | match. **R2** (.so disasm, Gcc path): wishspd 30 clamp, dot x,y,z, `<= 0` unrounded, `wishspeed*accel*frametime*friction`, clamp to the unrounded addspeed (NaN keeps accelspeed) |
| pm::walk_move / flat_wish / air_move | 0xfa26c | 3.0 mp.dll instruction diff | match (round 1) |
| pm::friction / edge_ahead | 0xfde08, mp.dll | 0x100316b0..0x1003184f | match. **R2** (2.1 full disasm): waterjump `==` IEEE, speed stored, `< 0.1f`, reciprocal edge start x/y, z = mins+origin stored, stop z-34 unrounded, fraction == 1.0 → friction·edgefriction, ×pmfriction ×0.75 × (control·frametime), clamp 0 IEEE, scale stored, x by unrounded |
| pm::calc_roll | 0x1006d4 | FUN_10033a80 (the auditor's 0x10033a70 is mid-function; the body is 0x10033a80..0x10033b08) | match. **R2** (disasm): dot z,y,x, side stored, sign `-1` if `side < 0` (IEEE), fabs, `|side| < rollspeed` → `/rollspeed*rollangle` |
| pm::check_falling | PM_CheckFalling 0x1003d0 | FUN_100337d0 | match. **R2** (2.1 decompile): 350/680/340/200 thresholds, waterlevel, CHAN_VOICE vol 1 attn 0.8 (`tfc` for the middle), step sound, punch z = fall·0.013, the SDK's punch[0] clamp, fall velocity reset |
| pm::duck / unduck / fix_player_crouch_stuck / duck_hull_offset | PM_Duck 0xf9f7c / PM_UnDuck 0xff51c | 3.0 mp.dll instruction diff | match (round 1) |
| pm::ladder / ladder_move | 0xff654 | 3.0 mp.dll instruction diff | match (round 1) |
| pm::fly_move, categorize_position, check_water, water_move, water_jump, check_water_jump, check_stuck (+stuck table), noclip, physics_toss, push_entity, add_to_touched, clip_velocity, check_velocity, add_gravity / add_correct_gravity / fixup_gravity_velocity, prevent_mega_bunny_jumping, play_water_sounds / play_wade_sound | SDK functions in the .so (round 1 read FlyMove..Toss; WaterMove extended divide not filed) | 3.0 mp.dll instruction diff (round 1) | match (round 1; not in the auditor's table, not re-checked) |
| pm::update_step_sound / play_step_sound / map_texture_type_step_type | 0xfcc20 / 0xfc344 | 0x10030b00 / 0x10030220 | match (round 1). **R2** (2.1 disasm 0x10030d70..): walking ? 400 : 300, `(int)(flduck + t)`, then the step_interval hook, then FL_DUCKING ×0.35 |
| pm::accel_by_speed (PM_GetAccelBySpeed) | 0xfdcc4 | n/a (2.1 constant 0.7) | match (round 1) |
| pm::chase_origin (GetChaseOrigin) | 0xfee5c | 3.0 mp.dll instruction diff | match (round 1) |
| pm::spectator_move (server) | 0xfef3c / mp.dll 0x10038d20 | 0x100324c0 (mode 1 `10 - p`, 5 ×-2, 4 stale, origin for 5) | match |
| pm::categorize_texture_type, textures::find_in / parse / load | 0xfcb40 / 0xfc28c | 0x10030a20 / 0x100301c0 (SDK limits) | match |
| pm::ts::ts_handle | TSHandle 0xf96a0 | 0x10058400 | match (round 1 + #126 fix: double 0.3) |
| pm::ts::ts_dive | 0xf9534 | 0x100582d0 (bits 0x100/0x40/0x80, speeds 90/25, 90/50, 100/10, FixVelocity 2·maxspeed) | match. **R2** (2.1 decompile): same bits/speeds, VectorNormalize (2.1's, IEEE), onground before the loop (no effect), `sqrt(h*1600)`, FixupGravity, FixVelocity(maxspeed+maxspeed float) |
| pm::ts::ts_wall_jump / ts_wall_jump_climb / ts_go_duck / ts_stand_up / pseudo_climb | 0xf9444 / 0xf94e0 / 0xf93a8 / 0xf91fc / 0xf8fc8 | 0x100581d0 / 0x10058270 / mp.dll instruction diff | match (round 1, not changed since except hooks above) |
| client-dll HUD_PlayerMove -> player_move_with(CLIENT, gp rules, client spectator) | cl 0x1002e990/0x1002e330 | 2.1 cl PM_Move | match |
| client spectator::spectator_move / follow / locked_view_angles / swing_round / line_blocked / begin_player_move | cl 0x1002c790 | cl21 FUN_1005c820 (same flags 1/6/0xe, locked view, saved camera) | 3.0 match; **#205** (2.1 in-eye pitch). **R2**: 2.1 cl decompile line 167 `angles[0] * -2.0` under `iuser1 == 5`; port movement.rs:103-108 copies `pe.angles`; client-dll lib.rs:113 passes it for every ruleset |
| shared::protocol::stunt::encode_next_stunt | .so objdump 0x8a918 (Ghidra 0x9a918; RC truncate), mp.dll __ftol | Absolute::next_stunt_byte | match. **R2**: objdump 0x8a918..0x8a936 `fmul [10f]; fldcw RC=0xc; fistp` |
| stunt::pre_think | PreThink 0x908cc | 0x100537e0 (buttons, framerate hook, last ground, Play, block expiry `pl+0x460 < time`, observer return, CheckForBreakables, deadflag, Jump, Duck on IN_DUCK/FL_DUCKING/pflag 8, fall velocity) | match. **R2** (2.1 decompile): framerate `fuser1 == 0 \|\| 0.1 <= fuser1 ? 1/slowInv : 0` (= `anim_framerate` with the port's normal 1), last ground, Play, timers, `pl+0x460 != 0 && iuser4&1 && pl+0x460 < time` clears bit 1 and the timer |
| stunt::post_think, moves::super_jump_landing | PostThink 0x91610 | 0x10054180 (on-ground gravity 1, shake 20/30/0.3/512, roll tap +0.3 double, GoRoll -> next attack 0.75·T, else land wav, GO_DUCK\|RECOVER, view 16, recover 0.75·T, Animate 0x39 = port 0x3a; Animate 6/1; frame advance; button last) | match (frame advance itself: **#243**) |
| moves::fall_damage | PostThink (mp.dll 0x10060232..) | 0x10054291.. | **#227** (rounding, both rulesets); rest match. **R2**: disasm of both mp.dll sites confirms `fsub 680f; fst; fmul 0.22522523f; fst; fcomp [health]` |
| moves::play | Play 0x9a014 | 0x1005acf0 (2.1 resets chain +0x3c and climb tap +0x18; no climb counter, port's `wall_climbs` is never read) | match |
| moves::handle_stunt + go_dive | 0x9b000 / 0x9b1a0 | 0x1005b9f0 (inline dive; speed unrounded xy; dir XYZ unrounded) | match (go_dive's blur chance in 3.0: **#246**) |
| moves::check_for_wall_jump / check_for_wall_dive / check_for_wall_climb / wall_check / is_player / stunt_dir_back | 0x9ac14 / 0x9ad44 / 0x9ae6c | 0x1005b7a0 / none / 0x1005b890 | match (round 1, unchanged code) |
| moves::go_bullet_time | 0x9a5b4 | 0x1005b160 (fuser1 != 0 hook, 0.55·T next and prone lock, last_dir 3, next attack 0.55·T if < base) | match. **R2** (2.1 decompile): `fuser1 != 0` gate (`bullet_time_allowed`: port normal 1), dir = -forward, flag, next = 0.55·slowInv + time, NextStunt `__ftol`, `iuser4 |= 4|0x20`, last_dir 3, prone lock = float + time (sub-ulp, round 1 note), view 0, `m_flNextAttack < WTB (0)` → `0 + slowInv·0.55`, Animate 0x35 (= port 0x36), land.wav chan 4; no blur, no dodge |
| moves::wall_chain_add | GoWallJump/Climb fadd qword | 0x1005b42a / 0x1005b67b.. fadd qword | match (#124 comment fix); value use **#226** |
| moves::go_wall_jump | 0x9a7d4 / mp.dll 0x10068c9b | 0x1005b320 | 3.0 match; **#226** (2.1 value uses stored chain). **R2**: 2.1 disasm 0x1005b427..0x1005b4c9 confirms |
| moves::go_wall_climb | 0x9aa1c | 0x1005b550 (0.3/0.5/0.75 doubles, stored float throughout) | match. **R2** (2.1 decompile): order (bits, view, Animate 0x37 = port 0x38, gravity 1), CanDiveHigh/Medium adds, cap `10 <= slowInv·max(0.5, chain)`, NextStunt, airwheel.wav; no blur |
| moves::go_roll | GoRoll 0x9cfe8 | 0x1005d600 (speed unrounded, 0xb0c30 = port 0x130c30, tap +0.3 double, no texture sound) | match. **R2** (2.1 decompile): air + tap expired returns without re-arming, `80 <= |v.xy|` and `time <= tap`, mask, `time < recover`, ladder, dir normalise (zero → (0,0,1)), friction 0.3, clear glide, `|= 0x40000|4|1`, view 0, recover 0.75·slowInv, Animate 0x2f, land.wav, tap 0; else tap = time + 0.3 |
| moves::go_wheel | 0x9c9bc | 0x1005d0e0 (stored speed >= 80, coyote on doubles, high/low branches, backflip kick hook) | match. **R2** (2.1 decompile + disasm 0x1005d1dc): gate `time <= tap && next <= time`, mask 0xb0c30, recover, ladder, ground or `time - last <= slowInv·0.7`; stored speed `< 80` (`test ah,5; jp`) → back dir; a NaN speed takes the moving branch in 2.1 and the back branch in the port (NaN-only, added to the notes) |
| moves::close_attack (+gate, kick_steps) | 0x9c630 | 0x1005cd00 (Arm before Draw before Hit; strength XYZ unrounded ×1/15 clamp 10..20) | match |
| moves::jump / duck | 0x92864 / 0x927fc | 0x100550e0 / 0x10055090 | match |
| moves::handle_slide / side_roll / normalized_forward | 0x9b9f8 | 0x1005c1f0 (get-up 0x4002, prone lock `<`, seq_finished = 1, normalised forward XYZ) | match |
| moves::check_for_breakables (dive part) + dive_fast_enough / dive_dir / dive_impact_damage | 0x9bd9c / mp.dll 0x10069ffd.. | 0x1005c680 (gate -1, on-ground reset, speed >= 64 XYZ unrounded, trace, victim tests, ×0.1 alive) | **#222** (dir sum order; NaN speed passes in 2.1); melee-window path = weapons area |
| moves::landing, handle_dive, handle_wheel, land_wall_stunt (handle_wall_jump / handle_wall_climb) | 0x9bbf4 / 0x9b7e4 / 0x9b820 / 0x9b4fc / 0x9b644 | 0x1005c4e0 / 0x1005c020 / 0x1005c040 / 0x1005bd80 / 0x1005bec0 | match (round 1, unchanged code) |
| stunt::slow_mul (GiveSlowMul), stunt_sound, send_next_stunt, set_anim_ext, in_airborne_stunt | 0xac5b4, EMIT_SOUND_DYN chan 4 vol 1 attn 0.8 pitch 95+rand(0,31) | the same EMIT_SOUND_DYN args (2.1 pitch via `sound_pitch`, no slow scaling) | match (`AIRBORNE_MASK` 0x10c10 = 2.1 CheckParamters' 0x10 \| 0x10c00) |
| stunt::motion_blur / maybe_blur | TSMBlur in GoDive/GoWheel/GoRoll/GoWallJump/GoBulletTime | none (2.1 `motion_blur()` false) | **#246** (3.0: f32 `10·slowmul + r`); 2.1 match |
| stunt::block_player (BlockPlayer) | 0x95b58 (stores the seconds, `iuser4 |= 1`) | 0x10056b60 (ground only, `pl+0x460 = t + time`) | 3.0 match; **#242** (2.1 duration rounding, with ts21 melee.rs:532) |
| stunt::get_direction / direction_in | GetDirection 0x6b008 | 0x1003d5c0 | match (round 1) |
| stunt::hspeed_cmp / hspeed_stored / normalized | HandleStunt 0x9b16a, GoRoll, Animate | GoRoll / GoWheel / Animate | match except **#222** (Animate's normalise in 2.1) |
| stunt::on_spawn / sync_superjump / texture_type_init | TSInit 0x92a4c (stunt + animation reset, physinfo) | TSInit 0x100552f0 | stunt/animation reset match; physinfo keys and spawn values are the players area's (not re-checked here) |
| stunt::client_command / debug_* / fill_client_data / game_init | none (port additions: `ts_debug_stunt` aids; iuser4/fuser1 into clientdata, TS's delta.lst carries both) | none | n/a (documented port additions) |
| ts30 RELATIVE clock (`StuntClock` default bodies, rules.rs): start / running / done / tick / tick_block / next_stunt_byte / wall_chain_cooldown / attack_delay | CTSStunt::Play 0x9a014 countdown, PreThink block countdown | n/a | match (round 1) |
| ts30 RELATIVE `air_stunt_allowed` | HandleStunt / GoWheel coyote (.so objdump 0x8b12b.., mp.dll 0x10069419..) | n/a | **#246** (f32 `0.7f/slowmul`; TS double 0.7 / slowmul on the x87). Was "match" (lumped into the ts30 stunt row) |
| ts30 stunt (STUNTS, can_dive) | constants in CTSStunt.c, CanDive* 0x9a494.. | n/a | match (values unchanged by the move, tests compare the floats) |
| ts21 stunt (STUNTS, ABSOLUTE start/running/done/next_stunt_byte/air_stunt_allowed/attack_delay, can_dive, fixed_slow_amt / slow_inv, bullet_time_allowed, anim_framerate) | n/a | 0x1005a3b0 / 0x1005a3d0 (`fuser1 != 0 ? 1/max(0.1f, f) : 1`), 0x1005b0b0.., PreThink framerate | as round 1 + kick_cooldown_on_try (#125), Animate flags (#112, #174): match except **#226**, **#242** |
| ts21 `grenade_fuse_step` | n/a | CM61Grenade::WaitThink 0x10047887.. | weapons area (not re-checked here) |
| anim::animate priority switch | Animate 0x6b470 | 0x1003d8b0, every case incl. 3, 4, 5 (goduck_holds), 7, 0x28, 0x29/0x2e/0x2f, 0x2a/0x2b (strafe keys), 0x30(=0x31), 0x31(=0x32), 0x35(=0x36, zeroes +0x34/+0x38: +0x34 unused), 0x36(=0x37), 0x37(=0x38, no dive exception), 0x38(=0x39), close-combat branch (no gait follow, 0x39 lets through) | match (#112 fix verified) |
| anim::torso cases (+ aiming / shot_running / shot_over / fire_pose_2 / arm_hold / death) | Animate | 0x1003d8b0: fire (slow one-handed `2`, shot `time + 2T` after SetAnim, prone slide shoot), idle (cur 0/8/9/0x32/0x33 = port 0x33/0x34, aim `time <= shot`), lying, jump (0x24 gate), goduck latch, death (80 stored speed, damage > 50 attack dir negated, forward/back/spin poses, bullet dance normal_die1..3), dive, get-up (SLIDE_END), side rolls, dive end/roll/wheel/lowwheel tables, reload/draw, grenade (shot 2T, grenade T), bullettime, wall poses, morfy_jump | match; velocity normalise rounding **#222** |
| anim::gait | Animate tail | 0x1003d8b0 tail (return ids, cc gait, 0x30c30, run2 > 180 / walk2 > 0 / idle_ext2 / combat_idle on `!(shot <= time)`, duck crawl/crouch_idle, whole-body follow on `shot <= time \|\| fin`) | match (NaN speed differs: unreachable) |
| anim::handle_close_combat | 0x6d8d0 | 0x1003f340 (flags 0x92/0x94 by gait, shot 2T; 2.1 has no cc_flag, unread in 2.1) | match (3.0 ids documented) |
| anim::gait_close_combat / melee_legs_follow / is_in_close_combat | 0x6dd14 / 0x6dcf4 | FUN_1003f680 / FUN_1003f650 (2.1 sets map to 3.0's for every move 2.1 has; 2.1 lacks uppercut and stock; speed 80 unrounded) | match |
| anim::update_timers / anim_timer / dance_due | 0x6b37c.. | end times (#112) | match |
| anim::enable_bullet_dance / disable_bullet_dance / bullet_dance | 0x6d314 / 0x6d2f0 / 0x6d338 | FUN_1003ee00 / FUN_1003ee50 | match. **R2** (2.1 decompile 0x1003ee00): flag 1, next = slowInv·0.15 + time, until = slowInv·3.0 + time (`anim_timer` → `Absolute::start`) |
| anim::set_anim / reset_sequence_info | SetAnim 0x6d7e4 / ResetSequenceInfo 0xa37c4 | SDK (same) | match (round 1; not in the auditor's table) |
| anim::studio_frame_advance | SDK StudioFrameAdvance (.so 0x935a0 objdump, mp.dll 0x10008290) | 0x10006090 | **#243** (f32 per step; TS one rounding). Round 1 marked it match |
| anim::blend_by_dir / blend_for_slide / slide_name / ext2_or_akimbo / name tables | 0x6d538 / 0x6d400 / 0x6d630, ctor 0x6b0c8 | — (3.0 names played, documented) | match (round 1; not in the auditor's table) |
| texturetype::init / find (game-side) | TEXTURETYPE_Init 0xe1244 / Find 0xe12c4 | FUN_10036580 / 0x10036760 | 3.0 match; **#206**. **R2**: 2.1 decompile `while (.. && count < 0x200)`, `i + 0xd` cut, 13-byte slots, only `sound/materials.txt` |

## Not verified / notes
- The melee-window half of CheckForBreakables, jump_kick_hit, backflip_kick, the 2.1 gun-side timer model: weapons area (#218 is open
  on the dive hit's TraceAttack direction, filed by another area).
- Animate's in-air dive blend for id 2: the decompile drops the ext2 test's use (register-passed); round 1 judged it a match; not re-done.
- NaN-only differences in core stunt code (2.1 IEEE tests): GoRoll `80 <= speed` (2.1 refuses a NaN speed, port rolls), GoWheel's
  stored-speed `< 80` (2.1 takes the moving branch on a NaN, the port the back flip; added in validation), Animate gait
  `speed <= 180` (2.1 run2, port idle), normalise zero tests, the wall-chain `<= 0.5` / cap with a NaN slowInv. Need a NaN velocity
  or time scale: not filed (mentioned in #222 for CheckForBreakables).
- 2.1 client.dll roaming spectator move stores every intermediate as float (unoptimised build); the port runs the server's arithmetic
  on both sides, documented (ts21.md).
- 2.1 PreThink's `pl+0x464` gravity reset timer: nothing in 2.1 writes it; not stunt code.
- Stunt sounds: 2.1 pitch through `sound_pitch` (2.1 returns the pitch, no slow-motion scaling): matches 2.1's EMIT_SOUND_DYN calls
  (chan 4, vol 1, attn 0.8, pitch 95 + RandomLong(0, 31)).
- `air_stunt_allowed` (3.0) calls `time_scale.max(0.01)` directly, not `slow_mul` (the `GiveSlowMul` plugin hook); mentioned in #246.
- Round 1's notes still apply (sub-ulp timer arithmetic, 2.1 PreThink framerate floor, NO_VIEW, GoDive's glide clear, bullet-time
  prone lock adding the float-rounded `0.55*slowInv`).

## Validation (round 2 check of this table)
- Rows: the auditor's table had 37 rows; this one has 73 (36 added or split out). Functions the auditor's table didn't list or
  covered only as "etc." / "round 1": the whole SDK pm set (check_parameters, reduce_timers, accelerate, air_accelerate, walk_move,
  check_falling, duck/unduck, ladder, fly/water/stuck/toss helpers, step sounds, chase_origin), the ts21/ts30 PmRules helper fns,
  stunt mod.rs helpers (slow_mul, stunt_sound, maybe_blur/motion_blur, block_player, hspeed_*, on_spawn, the port-only debug/client
  data), the Relative/Absolute clock bodies individually, anim set_anim / reset_sequence_info / studio_frame_advance / blend
  helpers / bullet dance, moves wall checks / landing / handle_* / dive helpers, and the client spectator helpers.
- Re-checked against the disassembly or decompile (R2 marks): 26 rows — ts21 fix_velocity, PM_Jump (+push_along, VectorNormalize),
  drop_punch_angle, step_interval, scaled_frametime, PM_PlayerMove head, CheckParamters, Accelerate (2.1), AirAccelerate (.so),
  Friction (2.1 full), CalcRoll (2.1 full), CheckFalling, UpdateStepSound tail, TSDive (2.1), client in-eye (#205), NextStunt
  encode (.so), PreThink (2.1), fall damage (#227, both mp.dll), GoBulletTime, GoWallJump (#226), GoWallClimb, GoRoll, GoWheel,
  bullet dance (2.1), TEXTURETYPE_Init (#206), plus the new BlockPlayer, StudioFrameAdvance, motion blur and coyote comparisons.
  Wrong verdicts found: 2 rows hid mismatches — the ts30 RELATIVE clock / stunt row (coyote test, and maybe_blur under it: #246)
  and round 1's `studio_frame_advance` "match" (#243); one new mismatch in an unlisted function (`block_player` in 2.1: #242).
  One wrong address (calc_roll 2.1 0x10033a70 → 0x10033a80) and one objdump-vs-Ghidra address (encode_next_stunt) fixed.
- Mismatch rows: #205, #206, #222, #226, #227 exist, are open, labelled bug and describe the same findings; their port file:line
  references check out (view.rs reference is 1138, not 1139).
- No "unverifiable" rows in the auditor's table.
- Address spot-check: 63 cited original addresses looked up in the decomp indexes; all exist except 2.1 0x10033a70 (fixed above).
- Remaining gaps: the SDK water/stuck/toss/ladder/duck helpers rest on round 1's 3.0 mp.dll ↔ 2.1 instruction diff (not re-done);
  Animate's case-by-case 2.1 check (#112) and CloseAttack 2.1 were not re-done; TSInit's physinfo/spawn values belong to the players
  area.
