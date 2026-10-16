# Damage audit (area: damage), round 2

Port HEAD 661d918 (main moved to 14240b0 meanwhile: #201's PreThink step, outside this area). Tools:
`scripts/round2/dis.sh so|mp|mp21 <ghidra addr> <len>` (objdump, .so = Ghidra − 0x10000),
`scripts/round2/fn.sh <dir> <addr>` (decomp by address), `scripts/round2/c.py mp|mp21 <addr>...`
(PE constants). Port default server arithmetic = 3.0 mp.dll (MSVC), `ts_pm_arith gcc` = .so.

Since round 1 (#200) the 3.0 rule bodies moved: `crates/ts30/server/src/{damage,melee,explode}.rs`
(hit multipliers, kevlar, victim holds, stunt kicks, thrown blades, slow point) and
`crates/ts30/src/melee.rs` (the gun's melee attacks); the core keeps `player/damage.rs`,
`damage.rs`, `weapons/melee.rs`, `entities/stock/explode.rs`, which both rulesets run.
Round-1 fixes #135 #136 #137 #138 #149 #150 re-checked in the moved code: all in place.

## Coverage

| Port function | 3.0 original | 2.1 original | Status |
|---|---|---|---|
| core `damage::multi::{clear,add,apply}` | Clear/Add/ApplyMultiDamage 0xf0d0c/0xf0d84/0xf0d38 | FUN_1006a2f0 / 1006a340 / 1006a310 (SDK) | match (type OR before the victim switch, flush with attacker as both, `glast_headshot = 0` only with a victim) — **re-checked (validation): match** (3.0 decomp 0xf0d0c/0xf0d84/0xf0d38; 2.1 SDK shape, no glast) |
| core `damage::apply` / `trace_attack` | Clear, TraceAttack +0x54, Apply | same | match |
| core `player::damage::trace_attack` | CBasePlayer::TraceAttack 0x901cc (mp 0x1005efb0) | 0x10053310 | match: protect, ×0.7 before the team check (IsPlayer + PlayerRelationship + friendlyfire), takedamage, last hitgroup, kevlar flag (armour, BULLET, IsAlive, group 2..5), dead-body BulletDance + `vel += dir·50` off ground, `time + 0.3` double (2.1 too, 0x100534dd), sounds RL order / pitch 95+RL(0,31) / attn 1.25 / slow 0.5 & 0.15 (2.1 SDK sound, `sound_pitch` hook), pain RL(1,7), AddMultiDamage(attacker). **mismatch #219** (PTakeDam kevlar `+128` added in f32; TS truncates the unrounded x87 sum; `as u8` saturates). dmgreport: 2.1 has none, but the cvar is never registered (no effect) — **re-checked (validation): match** (3.0 decomp: protect, iuser4 0x10c10 ×0.7, dodge ×0.7 only with BULLET, team check, takedamage, hitgroup switch, kevlar flag, body push, PTakeDam gate, sound order; 2.1: no dodge, head ×2 BULLET only, `fadd` double 0.3 at 0x100534dd, PTakeDam `min(dmg,255)`/0 = documented deviation ts21.md) |
| `ptakedam_amount` | 0x90472..0x904cc / mp 0x1005f201 | 2.1 `__ftol` | #219; 2.1 encoding = documented deviation — **re-checked (validation): #219 confirmed; 2.1 encoding documented (ts21.md "PTakeDam's amount byte keeps 3.0's encoding")** |
| ts30 `damage::hit_damage` (`player_hit`) | 0x901cc (0.7 double 0x90212/0x90241) | — | match (#138 fixed) — **re-checked (validation): match** |
| ts21 `damage::player_hit` | — | 0x10053310 (0.7 double 0x1005334a, head ×2 BULLET only, legs ×0.5) | match — **re-checked (validation): match** (0x10053310 case 1 `param & 2` only, `x + x`) |
| core `arms_bullet_dance` | TakeDamage 0x8fa94 | 0x10052d54..0x10052da9 | match (legs, `speed < 80` unrounded in mp.dll/2.1, `dmg <= 80`) — **re-checked (validation): match** (3.0 `group-6 < 2 || speed >= 80 || 80 < dmg` → disable; 2.1 `dmg > 80` via `a<80 == (a==80)`) |
| core `player::damage::take_damage` | 0x8fa94 (mp 0x1005e9c0) | 0x10052cf0 (disasm) | match: alive/`dmg == 0`, FPlayerCanTakeDamage, `m_lastDamage` trunc, armour hook, TBD reset, director (inflictor), trivial/major/critical, suit loop (gated on CBaseMonster's return), punch hook (2.1 after the loop, before the fade, regardless of `took`), fade `>= 1` on the untruncated value (≡ truncated), health reports and RL(0,3) order. **mismatch #216**: 3.0's red flash goes through `UTIL_ScreenFade`, which divides by GiveSlowMul; port writes it undivided (right for 2.1's SDK fade) — **re-checked (validation): match + #216 confirmed** (3.0 decomp: `m_lastDamage = trunc(dmg)` before kevlar, kevlar `(bits & 0x80040e4)==0`, group 2..5, then `trunc`, TBD reset, director byte 3/victim/inflictor/long 5, suit loop, fade `1.0 <= dmg_after_kevlar` via `UTIL_ScreenFade` (divides); BLAST → `IsTeamplay` call with no effect; `m_bitsHUDDamage = -1` dead in the port (no Damage message, save.rs). 2.1 decomp: same steps, `punchangle.x -= 3` after the loop, SDK fade) |
| core `damage_reports` / `health_reports` | 0x8fe2e..0x901b3 | 0x10052fc1..0x10053299 | match (both) |
| ts30 `damage::after_kevlar` (`armor_damage`) | 0x8fc25..0x8fc5f | — | match — **re-checked (validation): match** |
| ts21 `damage::armor_damage` | — | 0x10052e07..0x10052e8f (disasm) | match (0x40e0, `dmg <= armour` → armour −= dmg, ×0.5 double; else `x − (x − a)` on the FPU, armour 0; trunc) — **re-checked (validation): match** (disasm 0x10052e07..0x10052e8f incl. NaN branches: NaN armour passes `!= 0`, NaN damage takes the `x − (x − a)` branch, as the port) |
| ts21 `DAMAGE_PUNCH` | — | 0x1005314e (3.0f at 0x100816d0) | match — **re-checked (validation): match** |
| core `monster_take_damage` | CBaseMonster::TakeDamage 0xb2430 | 0x10011630 (SDK, folded with CBaseEntity's) | match (PainSound is empty in both; IsPlayer: dmg_inflictor, dmg_take, godmode; health; Killed gib flags) — **re-checked (validation): match** (0xb2430: takedamage, IsAlive→DeadTakeDamage (unreachable from the player path), PainSound if not dead (empty), FNullEnt+Instance for the dir, IsPlayer → dmg_inflictor/dmg_take/godmode, MONSTERSTATE_SCRIPT test unreachable for players, gib flag 2/1/0) |
| core `damage::update_attack_dir` / `attack_dir_between` | 0xb2430; mp 0x10013fc2..0x10014081 | 0x10011735..0x10011812 | 3.0 match; 2.1 **mismatch, comment on #220** (2.1's first Normalize keeps x unrounded, squares z unrounded × stored) — **re-checked (validation): 3.0 match** (decomp; #220 comment present) |
| core `take_damage_refused` / `trace_attack_refused` | FPlayerCanTakeDamage 0xe5208, PlayerRelationship 0xe5278 | 2.1 gamerules +0x40 / +0xc8 | match (3.0); 2.1 gamerules classes left to the rules audit |
| core `weapons::melee::check_hull` / `find_hull_intersection` / `hull_closer` | CheckHull 0x95e28, FindHullIntersection 0x95bac | 2.1 FUN_10056dd0 | round-1 match, not re-checked |
| core `weapons::melee::trace_attack` (kung fu / stock) | CloseCombatAttack 0x95f88, ColdCockAttack 0x96610 | FUN_10056f20, FUN_100572a0 | match (blur roll RL(10,80) kung fu only via `motion_blur`, takedamage/WORLDBRUSH/NODRAW, never-teammates flag, block → miss (CC) / target kept (stock), disarm after protect via `melee_disarm`, knockback >30, kungfuhit 95+RL slow 0.5 attn 0.8, bits 0x4001080, glast flags) — **re-checked (validation): match** (CloseCombatAttack 0x95f88: blur roll before the hull, `!IsPlayer && param_8` refusal — every caller passes 0, team check `(param_10 || ff == 0) && ally`, block → null (miss), disarm after protect, knockback `> 30`, PunchScreen (empty, 0x95f70)) |
| core `knockback_velocity` | 0x962b8.. / mp 0x10063bb9.. | 2.1 0x10057xxx | 3.0 match (#150 fix); 2.1 arithmetic not compared bit-for-bit — **re-checked (validation): 3.0 match** (.so delayed hit 0x9c312..0x9c3e9 re-read: unrounded `x,y,z` sum, stored length, NaN length → (0,0,1) via `eq87`, `1/len` × stored diffs, x product unrounded into the add, y/z stored) |
| core `weapons::melee::pending_hit` + `delayed_player_part` + `spend_delayed_damage` | CheckForBreakables 0x9bd9c | FUN_1005c680 (disasm) | match (±32 v_angle head hull, lunge victim, takedamage/WORLDBRUSH/NODRAW, `|d|` then half, `< 5` → 0 signed, team check keeps the victim, blocks by source 0/1/2 → 0x100439e0/0x10043870/0x10043720, disarm 15/The One via hook, knockback >30, sounds, bits 0x4001080/0x1080, #135 hook). **mismatch #218** (2.1 passes the roll direction `CTSStunt+0x24` as TraceAttack's dir; 3.0 g_vecZero) — **re-checked (validation): match + #218 confirmed** (3.0 0x9bd9c decomp: ±32 v_forward head hull from origin, `IsPlayer && window == 0` return, takedamage/WORLDBRUSH/NODRAW, lunge victim, bits, `|d|`/half/`< 5`, `!IsPlayer || d <= 0` → hit, ff/ally → no damage but victim kept, holds, disarm/knockback/sounds, PunchScreen empty, `g_vecZero`; 2.1 disasm 0x1005cca5..0x1005ccd9 pushes `&CTSStunt+0x24`, written by GoRoll 0x1005d758) |
| core `stunt::moves::check_for_breakables` (dive part) | 0x9bd9c | FUN_1005c680 | match (≥64, 64u along velocity, players skipped without a window, `speed·0.1` double alive / `speed` dead, 0x1080); dir: #218 |
| ts30 `melee::hold` (`melee_block`) | VictimKungfuHold 0x76f50, VictimKnifeHold 0x76de4, VictimKatanaHold 0x76c8c | — | match (re-checked kung fu: victim bar ≥ 0.1, the two blockers, 0.5/0.25, UpdateMeleeBar(33) on the one hit, kungfuhit pitch 100 on the non-blocker) — **re-checked (validation): match** (VictimKungfuHold 0x76f50 both cases incl. bar ≥ 0.1 on the original victim, damage zeroed on both, 0.5/0.25, UpdateMeleeBar(33) on the non-blocker; VictimKnifeHold 0x76de4 sparks mid+20, knife_hit on attacker, 0.5/0.25, bar 33) |
| ts30 `melee::jump_kick_hit` / `backflip_kick` | CloseAttack 0x9c630, GoWheel | — | round-1 match |
| ts21 server `melee::{melee_block, blade_block, kungfu_block, melee_disarm, jump_kick_hit, backflip_kick, blocked_damage, block_bar_left, block_hold}` | — | FUN_100439e0/10043870/10043720, FUN_10056f20, FUN_1005cd00, FUN_1005d0e0 | match (#136 fixed: bar − full damage) — **re-checked (validation): match** for `kungfu_block` (FUN_100439e0 decomp: conditions, `(1 − bar·0.01)·dmg`, bits 0x4001080, kungfuhit on attacker, both stored hits zeroed, bar − full damage, The One exempt); the blocker's `Animate(0x22)` (`kungfu_block` sequence, HandleCloseCombat 0x1003f340) is a documented deviation (ts21.md). `block_hold` adds `whiff_window`'s *stored* `f32` where 2.1 adds the unrounded `T·0.3` (0x10043c02..0x10043c1a); that code is being rewritten with #202 (timers as end times), not filed |
| ts30 shared `melee::{knife_fire, katana_fire, ccombat_attack, ccombat_block, cold_cock, melee_speed, back_dot}` | KnifeFire 0x84044, KatanaFire 0x833f0, CCombatAttack 0x86078, CCombatBlock 0x85ebc, ColdCock 0x81dd0 | — | round-1 match; re-checked KnifeFire (reach 35, hull 3, ×0.6666667f, bar multiplier, IsAlive/NODRAW, 0x1004, window `rate·0.25` on a miss only, hit-body sound) |
| ts21 shared `melee::*` (kung fu, knife, katana, stock) | — | FUN_1004de10, FUN_1004c6c0, FUN_1004c090, 0x1004ab40 | round-1 match; #149 fix present (punch anim, blocked → closecombat.wav) |
| core `entities::stock::explode::ts_explosion` / `explosion_damage` / `falloff_damage` / `blast_distance` / `occluder_thickness` | TSExplosion 0x7d01c (mp 0x10051450) | 0x10047aa0 (disasm) | 3.0 match (shake 80/1/2·rMax, TSFrags trunc, inflictor skip, takedamage/DEAD_DEAD, flash on IsAlive+IsPlayer, Center+1, LOS/occluded, attacker fallback, WorldGun hook, aftermath hook). 2.1: logic match; **mismatch #220** (unrounded differences, float 1/len Normalize, NaN branches); no inflictor skip in 2.1 (not observable: every 2.1 inflictor has takedamage 0) — **re-checked (validation): 3.0 match** (0x7d01c decomp + disasm 0x6d18d: attacker = inflictor when NULL, inflictor skip, falloff stored float with unrounded `rMax − rFull`, TSCreateDebris (port `TsFrags`) `(int)dmg`, fade (222,227,189) 0.1/0.05/75 on IsAlive+IsPlayer, Center+1, trace ignoring the inflictor, back trace ignoring the target, `thickness < d`, slow point); 2.1 #220 arithmetic not re-derived (issue body's disasm cited, issue open) |
| core `screen_shake` / `screen_fade` / `screen_fade_all` / `slowed` / `shakes` | UTIL_ScreenShake 0xec978 (mp 0x100766b0), UTIL_ScreenFadeWrite 0xecc54 (mp FUN_10076900) | UTIL_ScreenShake 0x10066f50, UTIL_ScreenFade 0x10067290 (SDK) | 3.0: **mismatch #219** (f32 divide; TS divides on the x87 then truncates). 2.1: **mismatch #217** (2.1 never divides by the time scale; port always does) — **re-checked (validation): #217 and #219 confirmed** (3.0 0xec978/0xecc54: `(longdouble)fixed / GiveSlowMul()`, FixedSlowAmt 0x990bc compares to the double 0.01; 2.1 0x10066f50 / FUN_10067180 write the fixed values undivided; 2.1's shake distance `(z² + x²) + y²` of unrounded differences = the port's `shakes`) |
| core `radius_damage` / `body_target` / `falloff_adjusted` | RadiusDamage 0xb2d64 (mp FUN_100145f0) | 2.1 SDK | round-1 + #137 fix: match (3.0); 2.1 RadiusDamage not compared bit-for-bit |
| ts30 `explode::{explosion_slow_point, thrown_blade, thrown_blade_damage, throw_speed}` | 0x7d964, KnifeFire 0x7465b, KatanaThrow 0x73dc1, CTSKnife::Spawn 0x7db54, BounceTouch 0x7decc | — | match — **re-checked (validation): match** (DoSpectacularNess 0x76b48, `CreateSlowMotionPoint(org, 0.25, 1.0, 0, 0)`; CM61Grenade 110/300/1024) |
| ts21 `explode::{GRENADE_EXPLOSION, aftermath, shrapnel_velocity, spawn_shrapnel}` | — | WaitThink 0x100477e0, TSExplosion 0x10047eaf..0x10047fbf | match (RL(30,45), RF order, 2000, owner = attacker/inflictor, then slow point 0.15/1.5) — **re-checked (validation): match** (pushes 0x10047848..0x10047857 = 1024/128/0x1040/100; shrapnel disasm 0x10047ec6..0x10047fb9: RF order x,y,z, `(z²+y²)+x²`, NaN length → normalise, float `1/len`, z product stored, ×2000; owner `edi`; slow point 1.5/…) |
| ts21 `blades::{thrown_blade, flown, thrown_blade_damage}` | — | CTSKnife 0x10048100 / 0x10048380 | round-1 match |
| ts21 `damage::{accumulate, bullet_damage, range_index, pierce_step, bullet_velocity_mult}` | — | ShootBulletsPlayer 0x1003fd40, FUN_1003f910 | round-1 match (bullets: weapons area) |
| core `player::damage::take_health` / `take_health_edict` (+ `player_add_health` hook) | CBasePlayer::TakeHealth 0xd8760 | CBasePlayer::TakeHealth 0x1002ab20 → CBaseMonster 0x100115e0 → CBaseEntity 0x1000c230 | **added in validation: match** (3.0: refuse with takedamage 0 or `max < health`, uncapped add, `time + 1` decay timer when over max; 2.1: refuse with takedamage 0 or `max <= health`, add, cap at max, so the timer never starts; 2.1's `m_bitsDamageType &= ~(type & 0x3fff)` with DMG_GENERIC is a no-op). Hook bodies are the items area (`ts30/server/src/items.rs:29`, `ts21/server/src/items.rs:76`) |
| core `player::damage::{hitgroup_scale, kevlar_scale, kevlar_absorb}` | TraceAttack 0x901cc hitgroup switch; TakeDamage 0x8fc25 kevlar | — (2.1 has its own `player_hit`/`armor_damage`) | **added in validation: match** (head ×2 / ×1.5 shotgun for BULLET\|THROWN, legs ×0.5; kevlar exempt mask 0x080040e4, groups 2..5, `min(absorb, 1)`; only reached through the ts30 hooks) |
| core `emit_sound_slow` + `sound_pitch` hook (ts30 `powerup::slow_pitch`) | EMIT_SOUND_DYN 0xe0a8c | EMIT_SOUND_DYN 0x10036300 (SDK) | **added in validation: match** (.so 0xd0aa7..: `(1 − s) + s·fuser1` unrounded (just `+ s` at fuser1 0), stored float, `× pitch`, truncated; sentences `!`; 2.1 plain pitch via the hook) |
| core `weapons::melee::{attack, slow_emit, same_team, sparks}` | ColdCock/KatanaFire/KnifeFire/CCombatAttack `GetGunPosition` + v_angle trace; EMIT_SOUND_DYN CHAN_BODY 1/0.8/0.5; UTIL_Sparks | 2.1 attacks (FUN_10056f20, FUN_100572a0) | **added in validation: partial** (attack: gun position hook + `zero_roll`; same_team = `mp_friendlyfire == 0 && ally`, as 0x95f88 / 0x9c3xx; not re-derived for each 2.1 caller) |
| `zero_delayed_hit_reaches_player` hook | 0x9c1f1 `!IsPlayer \|\| dmg <= 0` → hit (false) | FUN_1005c680 no `dmg <= 0` test (true) | **added in validation: match** (3.0 side re-read in 0x9bd9c) |
| ts30 `damage::trace_expanded` (`bullet_trace`) | UTIL_TraceLine_Expanded 0x6f4f4 | plain UTIL_TraceLine (hook) | **added in validation: not checked here** (bullets: weapons area) |
| `grenade_explosion` hook | CM61Grenade 110 / 300 / 1024, bits 0x1040 | 100 / 128 / 1024 (0x10047848..) | **added in validation: match** |
| ts21 shared `melee::{time_scale_inverse, fixed_delay, whiff_window, block_window, kungfu_damage, katana_swing, katana_damage, kungfu_move, motion_of}` | — | FUN_100551c0, KnifeFire 0x1004c6e2.., 0x1004e2dd.. | **listed in validation**: round-1 match; only `whiff_window` was re-read (via the block hold, see the ts21 server melee row) |

Counted (auditor; validation: 32 rows, now 40): 34 rows / ~60 port functions; 29 rows fully match for both rulesets; 5 new issues
(#216 #217 #218 #219 #220) and one comment (#220).

## Issues filed (round 2)
- #216 Audit 3.0: TakeDamage's red hit flash isn't divided by the victim's slow motion (also TSInit's spawn fade, lifecycle.rs:315, same pattern).
- #217 Audit 2.1: screen shakes and fades are divided by the receiver's slow motion (2.1's are the SDK's): TSExplosion, landing shake, env_shake/env_fade, player_loadsaved fade.
- #218 Audit 2.1: the delayed melee hit and the dive hit pass the roll direction (`CTSStunt+0x24`) to TraceAttack; port passes zero (3.0's g_vecZero).
- #219 Audit 3.0: PTakeDam's kevlar +128 and the shake/fade slow-motion divide round in f32 (and FixedSlowAmt's double 0.01).
- #220 Audit 2.1: TSExplosion distance/direction roundings and NaN branches; comment: 2.1's `g_vecAttackDir` first normalisation.

## Not verified / partial
- Bit-level arithmetic of 2.1's knockback (FUN_10056f20, FUN_1005c680), 2.1 RadiusDamage, 2.1 CheckHull/FindHullIntersection: logic matches, roundings not compared (the port runs mp.dll 3.0's).
- The 3.0 and 2.1 gun-side melee attacks (ts30 `melee.rs`, ts21 shared `melee.rs`) were moved unchanged since round 1's full check; only KnifeFire and the #149/#136 fixes were re-read.
- 2.1 gamerules' FPlayerCanTakeDamage / PlayerRelationship (rules audit).
- TSInit's fade hold (`m_flProtect − time` vs the port's 1.0) and whether `fuser1 ≠ 1` at spawn: noted in #216, rules area.
- Bullets/projectiles/breakables/worldgun/flag rows of round 1 belong to the weapons/entities areas now; not re-checked here.

## Validation (independent re-check)

Rows: 32 before (the auditor's "34" over-counts), 40 after (8 added: TakeHealth chain, the core hit/kevlar helpers, the slowed
`EMIT_SOUND_DYN` pitch, the core melee helpers, the `zero_delayed_hit_reaches_player` and
`grenade_explosion` hooks, `trace_expanded` (pointer to the weapons area) and the 2.1 shared
melee helpers that round 1 covered only as "`melee::*`").

Re-checked against the decompile/disassembly (22 rows, all marked "re-checked (validation)"
above): multi-damage, CBasePlayer TraceAttack (3.0 full, 2.1 PTakeDam/hitgroups/timer),
`ptakedam_amount`, both `player_hit`s, `arms_bullet_dance` (both), CBasePlayer TakeDamage (3.0
full, 2.1 decomp), both armour hooks (2.1 incl. NaN branches), `DAMAGE_PUNCH`, CBaseMonster
TakeDamage, `g_vecAttackDir` (3.0), CloseCombatAttack, the .so delayed knockback, the delayed hit
(3.0 full, 2.1 direction), the 3.0 kung-fu and knife holds, the 2.1 kung-fu block, TSExplosion
(3.0), shake/fade (both builds), the 3.0 slow point, the 2.1 grenade constants and shrapnel,
plus the new TakeHealth and EMIT_SOUND_DYN rows. Result: no "match" row was wrong; the five
mismatch issues (#216 #217 #218 #219 #220 + the #220 comment) are confirmed against the
original and their port file:line references are right (player/damage.rs:304-307,
explode.rs:47-49/75-98/107-121/250/292, melee.rs:329-332, stunt/moves.rs:872-875, :604,
damage.rs:71-75). Cited addresses spot-checked: all exist and are the named functions.

Findings not filed:
- 2.1 kung-fu block plays `Animate(0x22)` (`kungfu_block`) on the blocker: documented deviation (ts21.md).
- 2.1 `block_hold` rounds `min(3, 0.3·T)` to `f32` before adding it (2.1 keeps it on the FPU):
  last-bit only, and the function is being rewritten for #202.
- 2.1 PTakeDam encoding: documented deviation.
- NaN-only edges (NaN `fuser1` in `slow_of`, NaN shake radius in 2.1, NaN `block_knife`): not filed.

Remaining gaps (unchanged from the auditor's list): bit-level 2.1 knockback/RadiusDamage/CheckHull;
the 3.0 gun-side melee attacks other than KnifeFire and CloseCombatAttack's server half; 2.1
gamerules' FPlayerCanTakeDamage/PlayerRelationship (rules area); TSInit's spawn fade (#216 note,
rules area); the 2.1 shared melee helpers beyond `whiff_window`.
