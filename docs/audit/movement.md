# Movement + stunt audit (area: movement)

Port commit c49ee1a. Helpers: /tmp/audit/fn.sh (decomp by addr/name), /tmp/audit/dis.sh, /tmp/audit/{so,mp,cl,mp21,cl21}.asm (full objdump).

## Issues filed
- #124 Audit 3.0: NextStunt byte. mp.dll truncates `t*10` (`__ftol` 0x1007d310, e.g. 0x10068cdc); the .so rounds to nearest-even
  on the exact product (objdump 0x8a918). Port protocol/stunt.rs:23 rounds an f32 product for every arith. Default arith: bullet
  time 0.55 sends 6 (TS 5), a 0.7 wall chain sends 7 (TS 6). gcc arith: 17 reachable chain values off by one. HUD cooldown off by 0.1 s.
- #125 Audit 2.1: CloseAttack 0x1005cd00 arms kickCD = time + 0.7*slowInv BEFORE the kick is tried and draws RandomLong(0,2) on
  every try. 3.0 and the port (moves.rs:683-684) do both only on a hit, so in 2.1 mode a held jump re-tries the kick every frame.
- #124 comment: wall-chain sums use double 0.35/0.3 in .so, mp.dll and 2.1 mp.dll; port adds f32 constants (moves.rs go_wall_jump/climb):
  339/5460 chains differ (0.3+0.35: TS 0.65000004, port 0.64999998).
- #112 comment (2.1): Animate 0x1003d8b0 priority rules differ beyond the id shift (goduck hold, side roll held by strafe keys,
  walljump_end held on the ground and blocks dive, no gait follow under melee); interacts with Play's m_fSequenceFinished recovery end.
- #145 Audit 2.1: shared SDK pm differs from 2.1 mp.dll beyond NaN (#105): texture table 512 entries / 13-byte slots / 12-char
  compare + sztexturename[12] (0x1002ff30, 0x100301c0, 0x10030a20); in-eye spectator pitch x-2 (0x100325d0); CalcRoll
  (side/rollspeed)*rollangle (0x10033af1); Friction (1/speed)*vel*16 (0x10031728). Comment on #105 (its step 1 done).
- #126 Audit 3.0 (gcc arith only): TSHandle prone side/up *0.3. The .so (0xf96a0) uses a double 0.3; port pm/ts.rs:52 uses f32 0.3, giving 1-ulp diffs.

## Coverage (3.0 = ts_i386.so Ghidra addr unless noted; 2.1 = ts21 mp.dll)
| Port fn | 3.0 original | 2.1 original | Status |
|---|---|---|---|
| pm::Pm::player_move / walk_physics | PM_PlayerMove 0x1008d8 | 0x10033c90 (decompile; inlines Toss/NoClip/PlayWaterSounds; scaled_frametime override) | match |
| check_parameters | PM_CheckParamters 0xf9c94 | 0x10058a60 (glide bit remapped) | match |
| reduce_timers | PM_ReduceTimers 0x1007f8 | 3.0 decompile/disasm; 2.1 = 3.0 mp.dll instruction diff | match |
| ts::ts_handle | TSHandle 0xf96a0, cl 0x10053660 | 0x10058400 (RECOVER 0.1 / prone 0.0 overrides) | #126 (gcc only) |
| ts_dive | TSDive 0xf9534 | 0x100582d0 (wall_dive=false, FixVelocity) | match |
| ts_wall_jump / ts_wall_jump_climb | 0xf9444 / 0xf94e0 | 0x100581d0 / 0x10058270 (consts) | match |
| ts_go_duck / ts_stand_up / pseudo_climb | 0xf93a8 / 0xf91fc / 0xf8fc8 | 3.0 decompile/disasm; 2.1 = 3.0 mp.dll instruction diff | match |
| jump | PM_Jump 0xf9944 | 0x10058680 (jump_push override) | match |
| duck / unduck | PM_Duck 0xf9f7c / PM_UnDuck 0xff51c | 3.0 decompile/disasm; 2.1 = 3.0 mp.dll instruction diff | match |
| check_falling | PM_CheckFalling 0x1003d0 | 3.0 decompile/disasm; 2.1 = 3.0 mp.dll instruction diff | match |
| drop_punch_angle | 0x100760 | 0x10033b10 (override) | match |
| accelerate | PM_Accelerate 0xfdd08 | 0x100315f0 (disasm: Constant(0.7) path) | match |
| friction | PM_Friction 0xfde08 (disasm) | 3.0 decompile/disasm; 2.1 = 3.0 mp.dll instruction diff | match |
| walk_move / flat_wish / air_accelerate | 0xfa26c / 0xfdfdc | 3.0 decompile/disasm; 2.1 = 3.0 mp.dll instruction diff | match |
| ladder_move | PM_LadderMove 0xff654 | 3.0 decompile/disasm; 2.1 = 3.0 mp.dll instruction diff | match |
| update_step_sound / step_interval | 0xfcc20 | 0x10030b00 (override) | match |
| play_step_sound | 0xfc344 (Length rounds, objdump ebcbc) | 0x10030220 (disasm: bits 1/0x200, >220, attn 1.25) | match |
| PmRules defaults (3.0) | as above | n/a | match |
| ts21 PmRules overrides | n/a | ts21 pm.rs vs 2.1 disasm (signature + x87 diff) | match |
| speed_by_slots | GetSpeedBySlots 0x76bd4 (disasm) | 0x100436e0 (override) | match |
| stunt::get_direction | GetDirection 0x6b008 | 0x1003d5c0 (pushes 0.6/0.3 at every call) | match |
| moves::play | CTSStunt::Play 0x9a014 | 0x1005acf0 | match |
| handle_stunt | HandleStunt 0x9b000 | 0x1005b9f0 (decompile line by line) | match |
| check_for_wall_jump/dive/climb | 0x9ac14 / 0x9ad44 / 0x9ae6c | 0x1005b7a0 / none / 0x1005b890 (decompile line by line) | match |
| go_bullet_time | 0x9a5b4 | 0x1005b160 (decompile line by line) | match |
| go_wall_jump / go_wall_climb | 0x9a7d4 / 0x9aa1c | 0x1005b320 / 0x1005b550 (decompile line by line) | match |
| go_dive | 0x9b1a0 | inlined in 0x1005b9f0 (decompile line by line) | match |
| land_wall_stunt (HandleWallJump/Climb) | 0x9b4fc / 0x9b644 | 0x1005bd80 / 0x1005bec0 (decompile line by line) | match |
| handle_dive / landing | 0x9b7e4 / 0x9bbf4 | 0x1005c020 / 0x1005c4e0 | match |
| handle_wheel / handle_slide / side_roll | 0x9b820 / 0x9b9f8 | 0x1005c040 / 0x1005c1f0 (decompile line by line) | match |
| go_wheel | GoWheel 0x9c9bc | 0x1005d0e0 (decompile line by line) | match |
| go_roll | GoRoll 0x9cfe8 | 0x1005d600 | match |
| close_attack (+gate, strength) | CloseAttack 0x9c630 | 0x1005cd00 | 3.0 match; #125 (2.1) |
| moves::jump / duck | CBasePlayer::Jump 0x92864 / Duck 0x927fc | 0x100550e0 | match |
| fall_damage / super_jump_landing / post_think | PostThink 0x91610 | 0x10054180 | match |
| stunt::pre_think order, block timer | PreThink 0x908cc | 0x100537e0 | match (see notes) |
| rules::can_dive / slots_used | CanDive* 0x9a494.. | 0x1005b0b0.. (disasm, armour <= 0 rule) | match |
| StuntRules CORE / RELATIVE clock | constants in CTSStunt.c | STUNTS / Absolute | match |
| check_for_breakables | CheckForBreakables 0x9bd9c (no-window path; window path = weapons) | 0x1005c680 | match |
| anim::set_anim / reset_sequence_info / studio_frame_advance | SetAnim 0x6d7e4 / ResetSequenceInfo | — | match |
| anim::update_timers / bullet_dance / enable/disable | 0x6b37c / 0x6d338 / 0x6d314 / 0x6d2f0 | 2.1 absolute timers (#112 item 4) | match (3.0) |
| anim::gait_close_combat / is_in_close_combat | 0x6dd14 / 0x6dcf4 | 2.1 has no GaitCloseCombat | match (3.0) |
| anim::blend_by_dir / blend_for_slide / jump blend, name tables | 0x6d538 / 0x6d400 / 0x6d630, ctor 0x6b0c8 (+ .rodata strings) | — | match |
| anim::handle_close_combat | HandleCloseCombat 0x6d8d0 | 0x1003f340 (documented: 3.0 ids) | match (3.0) |
| anim::animate (priority switch, torso, death, gait) | Animate 0x6b470 every case | 0x1003d8b0 | 3.0 match; 2.1 priority differs -> #112 comment |
| wall chain accumulation | GoWallJump/Climb fadd qword | 0x1005b42a.. fadd qword | #124 comment |
| moves::duck (2.1) | — | CBasePlayer::Duck 0x10055090 | match |
| send_next_stunt / encode_next_stunt | .so 0x8a918, mp.dll 0x10068cdc | Absolute::next_stunt_byte (trunc f64) | #124 (3.0); 2.1 match |

## Not verified / notes
- SDK pass (final): tools /tmp/audit/norm.py (normalised objdump, .rdata constants resolved), sig.py (constants/fields/
  immediates/calls signature), fpu.py (x87 op-sequence diff), pairs.py (3.0 mp.dll <-> 2.1 mp.dll function map by source order).
  * 3.0 mp.dll vs client.dll: all 42 SDK pm functions identical except PlayerMove's client-only runfuncs spectator globals
    and the client's own SpectatorMove (handled by crates/client/src/spectator).
  * 3.0 .so vs port Gcc path: FlyMove, CatagorizePosition, CheckWater, CheckWaterJump, WaterMove, WaterJump, PreventMega,
    CheckStuck, UnDuck (startsolid at result+4, disasm), NoClip, Toss, Friction (disasm) read; match. WaterMove speed rescale
    divides in extended where the port divides in f32 (double-rounding ~2^-40): not filed.
  * 2.1 vs 3.0 mp.dll: identical x87 sequences/constants for all but Friction, CalcRoll, SpectatorMove, texture fns -> #145;
    Accelerate/UpdateStepSound/PlayerMove frametime are overridden; compare encodings are #105.
- Follow-up pass: every 2.1 handler (Play, HandleStunt+inlined GoDive, GoBulletTime, GoWallJump/Climb, CheckForWallJump/Climb,
  HandleWallJump/Climb, HandleDive/Landing, HandleWheel, HandleSlide, GoWheel, GoRoll, CloseAttack, CheckForBreakables,
  Jump/Duck, PostThink/PreThink parts, CanDive*) read line by line in the decompile, with disasm for the timer/x87 points
  (coyote diff kept as double; bullet-time proneLock adds the float-rounded 0.55*slowInv: sub-ulp, not filed).
- 2.1 GoDive doesn't clear glide/wall bits (3.0 does): unreachable, since HandleStunt's 0xb0000 gate precedes it and 2.1 has no wall dive.
- 2.1 Animate torso names are 2.1-model names; the port deliberately plays 3.0's (ts21.md), so they weren't compared.
- CheckForBreakables' close-combat-window path is the weapons area's (weapons::pending_melee_hit); not audited here.
- Known, documented gap: the 2.1 backflip kick window (0.3*T relative) relies on gun timers counting down in real time
  (ts21.md "Not done here": gun-side timer model, #100 item 3). Not filed again.
- 2.1 PreThink framerate is FixedSlowAmt (0 below 0.1) vs port max(fuser1, 0.01): unreachable (2.1 time scales >= 0.15).
- 2.1 has no NO_VIEW bit; the port sets it, but it only feeds the port's client visuals and the ts21 protocol mapping strips it.
- Fall damage factor 2.1 `(v-680)*0.22522523` vs 3.0 `*100*0.0022522523`: last-bit difference at most, not filed.
- Stunt timer countdown (Play) uses an f32 dt where TS keeps slowmul*frametime unrounded: sub-ulp, not filed.
