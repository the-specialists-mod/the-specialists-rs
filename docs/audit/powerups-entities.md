# Audit: powerups-entities (final)

Port HEAD c49ee1a. so addr = ghidra; file = ghidra-0x10000.

## Findings (to file)
- P1: SlowMotionFlow (0x96bb0) traces UTIL_TraceLine(..., ignore_monsters, ignore_glass=1) -> flags 0x101 (0xed4b0). Port rules/powerup.rs:227,274 uses IGNORE_MONSTERS (1): glass blocks slow-mo spread.
- P2 (minor): SlowMotionFlow entity cap: TS checks count<32 once before the loops and at push; GoSlow is applied to every visible projectile even when the array is full (those then never get restored). Port returns as soon as len>=32 (powerup.rs:260).

Filed: P1+P2 -> #132. 2.1 item_longjump -> #133.

## Coverage (3.0)
| port fn | original | status |
|---|---|---|
| rules/powerup.rs update_state | UpdatePowerUpState 0x95748 | match |
| use_powerup | UsePowerUp 0x95980 (trunc confirmed 0x85a73) | match |
| add_powerup | PlayerAddPowerUp 0x97bc8 | match (armor 0x8c always 100.0) |
| interpolate | InterpolateSlowMotion 0x9910c | match |
| go_slow_plain/go_slow_self | GoSlow 0x929d0 | match |
| in_slow | InSlow 0x9298c | match |
| deactivate_flow | DeactivateSlowMotionFlow 0x96a9c | match |
| slow_flow | SlowMotionFlow 0x96bb0 | mismatch P1, P2 |
| remove_from_slowed | RemoveEntityFromSlowedArray 0x9fb4c | match |
| set_plugin_speed amount.max(0.01) | FixedSlowAmt 0x990bc | match |
| entities/ts/powerup.rs kind_for/setup/body/duration | SetupPowerUp 0x98014 (trunc 0x881f4) | match |
| random_powerup (state.rs) | GetRandomPowerup 0x97894 | match |
| PowerUp::key_value | KeyValue 0x97ebc | match |
| PowerUp::spawn | Spawn 0x98258, CTSSuperJump::Spawn 0xc9f44 | match |
| fall / on_ground | Fall 0x983cc / OnGround 0x9848c | match |
| think Waiting | WaitThink 0x98570 | match (renderamt f32 vs x87 /30: ulp-level) |
| pickup_plan/pickup/after_pickup | WaitTouch 0x98614 | match |
| create | CreatePowerUp 0x97fa0 | match |
| PreThink slow block (lifecycle.rs pre_think) | PreThink 0x908cc (Interpolate, GiveSlowMul framerate, UpdatePowerUpState, SlowMotionFlow order) | match |
| explode.rs emit_sound_slow/slow_pitch | EMIT_SOUND_DYN 0xe0a8c (trunc 0xd0b1e) | match |
| triggers.rs SlowMotion (ts_slowmotion) | KeyValue 0x70d74, Spawn 0x70e14 (radius trunc 0x60f70), WaitTouch 0x70fdc, DeactivateSlow 0x71078, CheckPlayersDistance 0x7118c, WaitThink 0x712a4, ActivateSlow 0x71384, Use 0x71404 | match |
| triggers.rs SlowMotionPoint | Create 0x714d8 (args vec,0.25,1.0,0,0), Spawn 0x71564, Deactivate 0x7166c, WaitThink 0x71728 | match |
| frame.rs set_global_slow / global_slow2 / Think block | SetGlobalSlowMotion 0x7961c, GlobalSlowMotion2 0x79720, Think 0x77204 head; CBaseEntity::GoSlow/InSlow 0xac5e0/0xac5f4 (no overrides) | match |
| ts/mod.rs respawn_entities | RespawnEntities 0x77764 | match |

## Coverage (2.1, mp.dll)
| port | original | status |
|---|---|---|
| ts21 powerup::POWERUPS values | UpdatePowerUpState 0x100568e0, SetupPowerUp 0x10059ce0, WaitTouch 0x1005a160 | match |
| use_powerup (shared, no blur) | UsePowerUp 0x10056a90 (__ftol trunc) | match |
| add_powerup (shared) + powerup::armor | PlayerAddPowerUp 0x100599d0 | match (grenade PickUpWeapon flag arg differs 1 vs 0: weapons area, not checked) |
| powerup::kind | SetupPowerUp 0x10059ce0 | match |
| key_rule pwupmult refused | KeyValue 0x10059c10 | match |
| PowerUp spawn/fall/on_ground/WaitThink (shared) | Spawn 0x10059e70, Fall 0x10059f90, OnGround 0x1005a030 (size asm-checked), WaitThink 0x1005a0e0 | match |
| slow_flow (shared) | FUN_100576d0 | #132 (same glass trace 0x10067810) |
| deactivate_flow (shared) | FUN_10057610 | match modulo documented 0-vs-1 encoding; GoSlow(0) gun clamp = #112 (known) |
| item_longjump | CTSSuperJump::Spawn 0x10021910 (CItem), MyTouch 0x10021950 | mismatch #133 |
| ts21 SlowMotion (shared) | CTSSlowMotion Spawn 0x10040e30, WaitTouch 0x10040fb0, Deactivate FUN_10041030, CheckPlayersDistance FUN_100410a0, WaitThink 0x10041130, ActivateSlow 0x100411e0; InSlow FUN_100551e0 | match (under documented 0/1 encoding) |
| ts21 entities::trigger_touch / key_value AllowedTeam | CTSTrigger KeyValue 0x10040a70, WaitTouch 0x10040c00 | match |
| ts21 give_ground_weapon | GetWeapon FUN_10066250 | match |
| ts21 respawn_entities | RespawnEntities 0x10044400 | match |
| ts21 map_globals | (read from report/port doc; 2.1 Spawn 0x10042110 sets nothing) | not re-derived (Think-side read is rules area) |
| ts21 protocol.rs | documented 3.0-wire design (#100 1.2) | n/a (design) |

## Coverage (3.0 entities)
| port | original | status |
|---|---|---|
| ground.rs GroundWeapon key_value/spawn/fall/on_ground | KeyValue 0xa19f0, Spawn 0xa1b40, Fall 0xa2150, OnGround 0xa226c | match |
| GroundWeapon::give / get_weapon / touch / use_ | GetWeapon 0xa2328, WaitTouch 0xa26cc, Use 0xa230c | match |
| GroundAmmo key_value/spawn/fall/touch | KeyValue 0xa2718, Spawn 0xa27e4, Fall 0xa2d6c, OnGround 0xa2e88, WaitTouch 0xa2f40 (gun check is in give_ammo) | match |
| put_on_wall / wall_distance / wall_surface | PutOnWall 0xa1cd8 | match |
| ctb.rs Briefcase key_value/spawn/flag_fall/touch/capture | KeyValue 0x8d7f4, Spawn 0x8d88c, FlagFall 0x8da00, CanInteract 0x8db48, CaptureTouch 0x8db9c | match |
| ctb.rs Flag spawn/fall/hit_ahead/bounce_touch/land/respawn_flag/think | CreateFlag 0x9d874, Spawn 0x9d978, RespawnFlag 0x9de58, Fall 0x9df10, WaitThink 0x9e1b8, OnGround 0x9e21c, BounceTouch 0x9dbbc | match (BounceTouch 0.9 branch garbled in decomp; port's shared helper cites asm) |
| player_take_flag / take_flag / drop_flag / throw | PlayerTakeFlag 0x9e330, TakeFlag 0x93830, DropFlag 0x9334c | match |
| triggers.rs Trigger | KeyValue 0x70890, Spawn 0x709b8, WaitTouch 0x70ad8 | match |
| DmHill | Spawn 0x71a8c, WaitTouch 0x71be8 | match |
| Timer | Spawn 0x72db0, ToggleTimer 0x72e44, WaitThink 0x72eb8 | match |
| CustomTarget | FireTarget 0x731bc, Use 0x73244 | match |
| PlayerAmount | ClientConnect 0x72aec, ClientDisconnect 0x72c08 | match |
| misc.rs MapGlobals / MapFlags | Spawn 0x72534, get_weapon_restriction 0x7263c, KeyValue 0x72528 (+0x5874/5 bytes only ever zeroed) | match |
| breakable.rs breakable_respawn / respawn_all / Die tail | CBreakable::BreakableRespawn 0x74e84, CInfoBreakable::RespawnThink 0x75da0, CBreakable::Die tail | match |
| doors.rs breakable_respawn / helper | CBaseDoor::BreakableRespawn 0xb5604, CBaseDoor::Spawn helper creation | match |
| maprules.rs GamePlayerEquip / give_named_item | EquipPlayer 0xcd5b4, KeyValue 0xcd494, GiveNamedItem 0xdb524 | match |

## Round 2 (coordinator: close the gaps)

Filed: #143 (2.1 grenade power-up selects the grenades; 2.1 PlayerAddPowerUp passes select=0), #151 (2.1 battery/healthkit/chargers run 3.0 code; + comments: 2.1 HEV capacity 30 from hard-coded gSkillData 0x100a2058, health charger capacity 0x100a2060 never written = 0; 2.1 CBasePlayer::TakeHealth = SDK capped).

Method for stock entities: scripts/sdkdiff.py (3.0 decomp vs ~/dev/hl/halflife-sdk/dlls: strings + fractional constants per Class::Method, 699 pairs, out sdkdiff_all.txt / sdkdiff_frac.txt) and scripts/diff30_21.py (3.0 vs 2.1 same-named methods, 237 differing, diff30_21.txt); every flagged gameplay item checked by hand in decomp/disasm; prior function-level stock audit = issues #43-#89 (not repeated). Method-presence diff 3.0 vs 2.1: only MSVC inlining.

| port | original | status |
|---|---|---|
| add_powerup grenade / weapons::give_weapon | 3.0 PickUpWeapon 0x9314c (param_4 select), PlayerAddPowerUp 0x97bc8 (select 1); 2.1 PickUpWeapon 0x10055830, PlayerAddPowerUp 0x100599d0 (select 0) | 3.0 match, 2.1 mismatch #143 |
| misc.rs Wind | KeyValue 0x73ec4, Spawn 0x74288, BlowThink 0x743a8 | match |
| Fog / Emitter / Particle | CTSFog::Spawn 0x742e4, CTSEmitter Spawn 0x741d4 / CheckValues 0x740ac, CTSParticle Spawn 0x74134 / CheckValues 0x73f98 (server-unused clamps), AddParticle 0xf8178 | match |
| DeadCorpse / create_dead_corpse | TSCreateDeadCorpse 0x70234 (entvars mapped), Spawn 0x7040c, WaitThink 0x70460 | match |
| WinGiver<SCORE> | KeyValue 0x8bdfc, Spawn 0x8bf10/0x8c0b8 (vtable[0xf]=Activate, empty), Use 0x8bfb0/0x8c158 | match (TeamWin = rules area) |
| objective stubs (bomb/hack/teamescape/manager/ptr/sellzone) | modes 2/3 unreachable in 3.0 (docs + InitObjectives gate); SellTouch 0x727e8 gated on CashBased() 0x76abc == 0 | match (dead); 2.1 objectives = #66 + ts21.md open item |
| breakable break_touch | CBreakable::BreakTouch 0xc0c44 | match |
| breakable key_value + SPAWN_OBJECTS | KeyValue 0xc0370, pSpawnObjects 0x1181e0 (dumped) | match; 2.1 KeyValue 0x1001b050 'random' = SDK no-op |
| trigger_hurt hurt_touch | CBaseTrigger::HurtTouch 0xe73cc | match |
| items healthkit/battery (3.0) | CHealthKit::MyTouch 0xc8968, CItemBattery::MyTouch 0xcb74c | match |
| Charger<ARMOR>::use_ (3.0) | CRecharge::Use 0xc6fd0, CWallHealth::Use 0xc8d08 | match |
| items/chargers in 2.1 | 0x10020920, 0x10021600, 0x1001f6e0, 0x10020be0, Spawn/Recharge, FUN_100239c0 skill data, TakeHealth 0x1002ab20 | mismatch #151 |
| doors in 2.1 (DoorTouch speed, KeyValue, TraceAttack loosen 30/50, DoorHitTop) | 0x10014a30, 0x100138c0, 0x10013c50, 0x10014eb0 | match (effects = #109) |
| base.rs LinearMoveDone | 3.0 0xe377c, 2.1 0x100380c0 | match |
| env_explosion Spawn/Use | 3.0 0xbfb1c/0xbfbb0, 2.1 0x1001ab30 | match |
| env_beverage Use | 0xbdeac | match |
| func_tank Fire / skill dmg | 3.0 0xc39d4, 2.1 0x1001d800 (sprite colour round vs ftol, cosmetic); 2.1 monster bullet dmg never set (=3.0) | match |
| FireBullets bubble 1/64 | 0xb39a4 | SDK (compiler) |
| 2.1 ts_slowmotionpoint | Create 0x100412d0, Spawn 0x10041340, WaitThink 0x10041460, ActivateSlow 0x100415b0 | match |
| 2.1 global slow (frame::global_slow2, ts21 rules::global_slow/globalslow_value/step) | FUN_100455f0, FUN_10045500, Think 0x10043cc0 block (NaN traced) | match |
| 2.1 rules::read_mapglobals / entities::map_globals | Think head (+0x54bc gate, FUN_10042190 mask, bytes +0x54/+0x58) | match except playerrespawn not used = documented open item (ts21.md) |

## Round 3 (remaining stock entities, integers included)

Nothing new filed (no mismatch found beyond #43-#89, #109, #132, #133, #143, #151).

Method:
- scripts/sdkint.py: TS 3.0 vs SDK per Class::Method, ints + floats, struct offsets/indices/locals stripped, over every stock class the port links (405 functions with extra constants, sdkint.txt). Residual hits = enum/flag values, compiler-folded divisions (x/10 -> x*0.1, x/2 -> *0.5, /64, rad<->deg), and 0x18-style negatives; every "SDK constant missing in TS" case checked by hand (below).
- scripts/portvts.py: for every port stock fn whose doc cites a TS address, port literals absent from that TS function (plus folded variants): 7 hits, all false positives (inlined callee constants: CCycler::Spawn 80000/5/75=0x42960000; CXenSpore::Spawn 0.7/1.4/0.1/0.4 + sizes; BodyTarget 0.5..1.1 in TrackTarget; SDK-side constants of UTIL_ScreenFade/ParmsChangeLevel/FizzThink).

Hand checks (TS 3.0 decomp/disasm vs port; 2.1 where diff30_21 flagged):
| port | original | status |
|---|---|---|
| breakable take_damage_impl | CBreakable::TakeDamage 0xc1344 (CLUB x2 via x+x, POISON x0.1, crowbar one-hit FL_CLIENT inflictor) | match |
| Bubbling::think | CBubbling::FizzThink 0xb8bb8 (2.5 - 0.1*freq / 0.5 above 19) | match (ulp-level add order only) |
| env_explosion use | CEnvExplosion::Use 0xbfbb0 (magnitude-24)*0.6 | match |
| tracktrain blocked | CFuncTrackTrain::Blocked 0xd418c (ground-on-top lift min(|speed|,50), normalize()*dmg, (0,0,1) for zero) | match |
| trains.rs FuncTrain blocked | CFuncTrain::Blocked 0xd3478 (0.5 s, dmg) | match |
| plat_blocked | CFuncPlat::Blocked 0xd2e64 (1 dmg CRUSH, stop sound, reverse by state 2/3) | match |
| bmodels rotating / pendulum blocked | 0xa6660 / 0xa6b70 | match |
| buttons activate / trigger_and_wait / return / back_home | 0xa8d4c / 0xa8e8c / 0xa8f78 / 0xa9038 | match |
| tracktrain use_ (USE_SET notch, toggle) | 0xd437c; 2.1 0x100279d0 same formula | match |
| cycler generic_spawn | CCycler::GenericCyclerSpawn 0xc75d4 + Spawn 0xc7694 | match |
| xen spores spawn | CXenSpore::Spawn 0xf6880, Large 0xf6540 | match |

## Not verified (coverage gaps, after round 3)
- Stock entities: every function was put through the int+float constant diffs (vs SDK, vs 2.1, port-vs-TS) and all non-noise hits were hand-checked; the gameplay paths (Blocked/damage, buttons, trains, plats, tracktrain Use, breakable damage) were read line by line. Pure-cosmetic paths (sprite/beam/laser rendering fields, ambient_generic LFO/pitch ramps, xen animation, gibs, sentences) were NOT read line by line beyond the scans; logic changes that keep the same constants there would not be caught.
- 3.0-vs-2.1 constant diff can't see 2.1 floats/strings loaded via unresolved DAT_ refs; only constants unique to 2.1 were treated as signals.
- Weapons-area items left: CTSWorldGun / CTSKnife / CM61Grenade 3.0-vs-2.1 differences in diff30_21.txt (not my area).
