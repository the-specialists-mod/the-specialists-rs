# Damage audit (area: damage)

Tools: /tmp/audit/dmgbin/dis <ghidra addr> (ts_i386.so, objdump = ghidra-0x10000), dis30 (3.0 mp.dll), dis21 (2.1 mp.dll).
Port HEAD c49ee1a. Port default server arithmetic = 3.0 mp.dll (MSVC), `ts_pm_arith gcc` = .so.

## Coverage

| Port function | 3.0 original | 2.1 original | Status |
|---|---|---|---|
| damage::multi::{clear,add,apply} | Clear/Add/ApplyMultiDamage 0xf0d0c/0xf0d84/0xf0d38 | (same SDK) | match |
| player::damage::trace_attack + hit_damage | CBasePlayer::TraceAttack 0x901cc (mp 0x1005efb0) | 0x10053310 (hook ts21 damage::player_hit) | mismatch #138 (3.0: ×0.7 done in f32, TS uses double 0.7 in both builds) — otherwise match (order, team check, takedamage, hitgroups, kevlar flag, PTakeDam trunc, sounds/pitch/RNG order, pain, dead-body push, dmgreport). 2.1 match |
| ptakedam_amount | 0x90472..0x904cc (trunc) | 2.1 min(dmg,255)/0 kevlar | 3.0 match; 2.1 documented deviation (ts21.md) |
| player::damage::take_damage | CBasePlayer::TakeDamage 0x8fa94 | 0x10052cf0 | 3.0 match (bullet dance arm, FPlayerCanTakeDamage, trunc m_lastDamage + kevlar trunc, tbd reset, director 3/5, suit reports, fade ≥1, IsMultiplayer call for BLAST is a no-op, m_bitsHUDDamage never read) |
| after_kevlar | 0x8fc25..0x8fc50 | — | match (min(absorb,1), x87) |
| CBaseMonster::TakeDamage part (monster_take_damage, update_attack_dir) | 0xb2430 | — | match |
| rules::lifecycle::killed (+death_push, damage_force, killer_blur) | CBasePlayer::Killed 0x8ed5c | (shared) | match (blur rolls, respawn clamp/trunc, headshot slow 0.33, iuser4, push x87 order, hop, gib cond) |
| weapons::fire::shoot_bullets / pierce_shot / range_scaled | ShootBulletsPlayer 0x6f69c (mp 0x100478d0) | 2.1 hooks (bullet_damage, range_index trunc, pierce_step, accumulate) | match except range-index/bubble rounding = existing #80; also the scaled distance is kept unrounded (x87) in both TS builds before the truncation — commented on #80 |
| trace_expanded | UTIL_TraceLine_Expanded 0x6f4f4 | 2.1 plain trace_line | match (per docs) |
| weapons::bullets::{fire,think,update} | CBulletManager FireBullet 0x6eda4 / Think 0x6ec7c / UpdateBullet 0x6ef5c | 2.1 hooks | match (slot search, count, next_update, bits 0x2001002/0x1002, pierce, glast_weaponid); range index = #80 |
| spread::bullet_velocity_mult | GetBulletVelocityMult 0x99160 | ts21 bullet_velocity_mult | match (port f32 vs x87 return: sub-ulp, not filed) |
| ammo pierce/range_unit/velocity | s_ammo_info | — | match |
| weapons::melee::check_hull / find_hull_intersection | CBasePlayer::CheckHull 0x95e28 | — | match (by structure; SDK) |
| weapons::melee::trace_attack (KungFu/ColdCock) | CloseCombatAttack 0x95f88, ColdCockAttack 0x96610 | 2.1 FUN_10056f20 (hooks) | match (blur roll, takedamage/worldbrush/nodraw, team (param_10), block→miss, disarm+protect, knockback >30, kungfuhit pitch, bits 0x4001080, glast). param_8 (players-only) unported but every caller passes 0; PunchScreen is an empty stub |
| weapons::melee::hold | VictimKungfuHold 0x76f50, VictimKnifeHold 0x76de4, VictimKatanaHold 0x76c8c | 2.1 FUN_100439e0/10043870/10043720 (ts21 melee_block) | 3.0 match. 2.1: mismatch #136, kung-fu block reduces the blocker's bar by the FULL hit damage (param_3, fsubs 0x34(%esp) at 0x10043c52), port subtracts `through` |
| gun.rs ccombat_attack (move table) | CTSGun::CCombatAttack 0x86078 | (2.1 shared melee) | match (all constants/ids checked against disasm) |
| gun.rs cold_cock caller | CTSGun::ColdCock 0x81dd0 | — | match (blocked → kevlarhit) |
| weapons::melee::pending_hit + stunt::moves::check_for_breakables | CTSStunt::CheckForBreakables 0x9bd9c | 2.1 FUN_1005c680 | mismatch #135: port returns (and clears lunge_victim) when stored damage is 0 / spent dmg <= 0; TS (both) still traces and applies a 0-damage hit and sets the last victim (2.1 also runs the block + kungfuhit sound). Dive path: match (64u, speed*0.1 alive [double 0.1 in TS, f32 in port: #138], bits 0x1080) |
| gun.rs knife_fire / katana_fire (slash) | CTSGun::KnifeFire 0x84044 / KatanaFire 0x833f0 | (2.1 shared melee: not fully verified) | 3.0 match (constants, bits 0x1004/0x1080, windows rate·0.25 / 0.25, block → no window, IsAlive target) |
| projectiles Grenade think/touch, create_grenade | CM61Grenade::WaitThink 0x7cacc, BounceTouch 0x7ca90 → CTSGrenade::BounceTouch 0x7be28 | hooks grenade_explosion | match (fuse, impact ≥health, 0x1080, IsInWorld, TSExplosion 110/0x1040/300/1024, bounce sounds). f32 vs double 0.1 (ulp only) |
| projectiles Knife bounce_touch/think/settle, thrown_blade(_damage) | CTSKnife::BounceTouch 0x7decc / WaitThink 0x7ea30 | ts21 blades.rs (not verified vs 2.1 binary) | 3.0 match (player hit 0x8001000, owner/pev attacker, glance 0.4/0.98 → return, stick 6u back, 150/100/50) |
| explode::ts_explosion / explosion_damage / falloff_damage | TSExplosion 0x7d01c | 2.1 0x10047aa0 (hooks only) | 3.0 match (shake, TSFrags trunc, skip inflictor, takedamage/DEAD_DEAD, flash, Center+1, occlusion thickness, attacker fallback, WorldGun push, slow point) |
| explode::radius_damage | RadiusDamage 0xb2d64 | same SDK in 2.1 | mismatch #137: TS aims at BodyTarget (vtable +0x104): players Center+view_ofs·RANDOM_FLOAT(0.5,1.1), func_guntarget origin; port uses Center for all. (TraceAttack gets pevInflictor in TS, attacker in port — same entity for the only caller env_explosion) |
| entities::ts::shrapnel touch/think | CTSShrapnel::BounceTouch 0x7c200 / WaitThink 0x7c678 | ts21 explode::spawn_shrapnel (constants only) | match (100 dmg 0x1004, owner/pev attacker, stick on worldspawn/func_wall, pitch 120..130) |
| breakable trace_attack / take_damage_impl | CBreakable::TraceAttack 0xc1060 / TakeDamage 0xc1344 | — | match (hit dir fallback, shotgun ×8, RNG order, sparks/ricochet, crowbar, CLUB ×2, poison ×0.1, attack dir, DamageSound/Die) |
| worldgun fall impact | CTSWorldGun::Fall 0xa0d84 | — | match (health ≤ speed·weight, dmg ·0.001, 0x1080; attacker = documented deviation #1) |
| ts21 damage::{player_hit, armor_damage, bullet_damage, range_index, pierce_step, accumulate, bullet_velocity_mult}, DAMAGE_PUNCH | 2.1 TraceAttack 0x10053310, TakeDamage 0x10052cf0, ShootBulletsPlayer 0x1003fd40 | — | match (armour path checked in disasm; punch −3 after the suit loop) |
| ts21 explode GRENADE_EXPLOSION / aftermath | 2.1 CM61Grenade::WaitThink 0x100477e0, TSExplosion 0x10047aa0 | — | match (100/128/1024, 30..45 shrapnel ×2000, slow point 0.15/1.5); fuse step = documented time-scale encoding |
| ts21 blades thrown_blade(_damage) | 2.1 CTSKnife::Spawn 0x10048100 / BounceTouch 0x10048380 | — | match (1300, 0.9/0.25, 0x1080, 80→50 from 512 u) |
| ts21 melee melee_disarm / blade_block / kungfu_block / backflip / jump kick | FUN_100439e0, FUN_10043870, FUN_10043720, FUN_1005c680 | — | blade blocks match; kungfu_block: mismatch #136 (bar); rest match |
| ts21 shared melee kungfu_move / kungfu_damage | 2.1 CCombatAttack FUN_1004de10 | — | match (all move ids/delays/clamps/reach; the stunting kick's ×0.1 is a float const in TS vs f64 in port: ulp, covered by ts21.md x87 deviation) |

| ts21 shared melee knife_fire (slash) | 2.1 KnifeFire FUN_1004c6c0 | — | match (35/15 by btype, reach 38, hull 3, 0x1080, block window min(rate·5, T·rate·0.5), window only on a miss; blocked/hit skip it; anim ids = documented deviation) |
| ts21 shared melee katana_swing / katana_damage / katana_fire | 2.1 KatanaFire FUN_1004c090 | — | match (35/70 by speed>100, alternation + reroll, back −100/54/1.0, crouch 90 + bits 4|1 + view_ofs 16 + 3·T hold, airborne anim, trunc ×0.85, |v| to TraceAttack, signed v stored, hull 1/3) |
| ts21 shared melee cold_cock | 2.1 AlternateAttack 0x1004ab40/0x1004ad30 + ColdCockAttack FUN_100572a0 | — | mismatch #149 (no anim 0x17; blocked → 2.1 closecombat.wav, port kevlarhit); rest match (0.725 fixed delay, 20 dmg, reach 38, flags 0) |
| shared weapons::melee::trace_attack under 2.1 | 2.1 CloseCombatAttack FUN_10056f20 | — | match (no blur roll, team/block/disarm/knockback/sound order; >15 disarm gate in CCombatAttack 0x1004e856) |
| projectiles Knife bounce_touch tail | CTSKnife::BounceTouch 0x7e18f..0x7ea24 (disasm) | — | match (player hit → shared tail: hitbody sound pitch 100, WaitTouch, flying=0, nextthink+0.1, gravity 0.75, expire +10; "knife" other → return) — timer precision #150 |
| ctb flag fall / hit_ahead | CTSFlag::Fall 0x9df10 | (no flag in 2.1) | logic match; precision #150 (speed·30 unrounded) |
| effects beam damage | CBeam::BeamDamage 0xba1b8 | same SDK | match ((time−dmgtime)·dmg one rounding, DMG_ENERGYBEAM 0x400, decal roll RL(0,4)+bigshot1 ≡ RL(1,5)) |
| tank fire_bullet | CBaseEntity::FireBullets 0xb39a4 via CFuncTankGun::Fire 0xc3f64 | 2.1 FireBullets FUN_100126a0 via 0x1001dc90 | mismatch, existing #89 (texture sound, TE_GUNSHOTDECAL/DamageDecal, bubble trail); commented with 2.1 + spread precision |
| triggers hurt_touch | CBaseTrigger::HurtTouch 0xe73cc | 2.1 0x1003aaa0 (identical) | match |
| triggers radiation_think | CTriggerHurt::RadiationThink 0xe703c (mp 0x100413a0), disasm | — | logic match (centre, view_ofs cleared for PVS, `range <= geiger → store`, nextthink +0.25 exact); length precision → #150 comment |
| melee check_hull / find_hull_intersection | CBasePlayer::CheckHull 0x95e28, FindHullIntersectionplayer 0x95bac (disasm) | — | match (decompile drops the distance update; disasm stores it at 0x95ded) |
| lifecycle damage_force | CBaseMonster::DamageForce 0xb2d08; mp inline 0x1000e7b6 | — | match vs mp.dll (3-float product rounds once in any order; cap on unrounded vs f32 gives same result) |
| precision sweep (timers, impact, fuse, fall, bullet step, 2.1 kick) | see #150 | | mismatch #150 |
| length/normalize sweep (attack dir, death push, knockback, dive, impact, knife, shrapnel, breakable hit dir, TSExplosion dist/falloff/dir, geiger) | mp.dll inline Vector::Length/Normalize (53-bit, one rounding) | | mismatch, comment on #150 (port f32 helpers differ for 66 % of vectors) |
| tank spread_dir | FireBullets mp FUN_10014f00 0x10014f91..0x10015032 | | mismatch, comment on #89 (y unrounded in both builds) |

## Issues filed
- #135 Audit 3.0 (+2.1): delayed melee hit, zero stored damage → TS still hits (0 dmg) and records the victim; port returns and clears lunge_victim.
- #136 Audit 2.1: kung-fu block reduces the blocker's bar by the full damage (port: by `through`).
- #137 Audit 3.0 (+2.1): RadiusDamage uses BodyTarget (players: Center + view_ofs·RANDOM_FLOAT(0.5,1.1); guntarget origin), port uses Center.
- #138 Audit 3.0: TraceAttack stunt/dodge ×0.7 and dive ×0.1 are double constants in TS, f32 in port.
- #149 Audit 2.1: stock hit — no animation 0x17, blocked → kevlarhit instead of closecombat.
- #150 Audit 3.0: damage/projectile float precision (time+0.1/+0.3 doubles, impact ×0.1 double, fuse, flag/worldgun fall, bullet step order, 2.1 kick 0.1f).
- Comment on #89 (existing func_tank issue): applies to 2.1 too; spread precision (.so), then a second comment confirming mp.dll.
- Comment on #150: length/normalize sweep with mp.dll addresses per site, TSExplosion falloff, geiger length.
- Comment on #80 (existing): range-index value must keep x87/53-bit precision before truncation (both builds), port rounds twice in f32.

## Documented deviations seen (not filed)
PTakeDam 3.0 encoding in 2.1 (ts21.md); 2.1 block damage hitgroup/origin and no anim 0x22 (ts21.md); flying-impact attacker = thrower (weapons.md, issue #1); time-scale encoding (ts21.md); x87→f64 maths (ts21.md).

## Not verified / partial
- PlayerKilled: rules audit.
- Nothing else open in this area.
