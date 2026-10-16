# Audit: powerups-entities (round 2)

Port HEAD 661d918 (after the #200 split); validated against 5032209 (#198/#201 merged: no change in this area's files). `.so` addresses are Ghidra's (objdump + 0x10000); 2.1 and
3.0 `mp.dll` addresses are VAs. Round 1 (c49ee1a) is in git history; its issues #132, #133, #143
and #151 are fixed and were re-checked against the new code.

## Findings (filed)
- **#221** (2.1): `ts_slowmotion` `Use` (0x10041240) and `ts_dmhill` `WaitTouch` (0x10041830,
  tail at 0x10041d58) send `TSHudMessageAll(.., 1, ..)` (top). The core sends 3.0's 8 (bottom),
  with no hook: `entities/ts/triggers.rs:695` and `:277`.
- **#223** (2.1): `deactivate_flow` releases the slowed players through `set_slow_target`
  directly (`rules/powerup.rs:319-325`). 2.1's `DeactivateSlowMotionFlow` (FUN_10057610) calls
  `GoSlow(victim, 0)`, so its `m_flNextAttack` clamp (`on_go_slow`, #112) never runs for them.
  The plugins' GoSlow hook is skipped too.
- **#224** (2.1): `GroundWeapon::spawn` → `spawn_weapon_id` (`entities/ts/ground.rs:37`) turns an
  akimbo pair into its single (3.0 `Spawn` 0xa1b40, because 3.0's `PickUpWeapon` refuses pairs).
  2.1's `Spawn` (0x10065c70) keeps the id, and its `PickUpWeapon`/`TakeWeapon` give the pair
  (ids 10, 16, 30).
- **#228** (3.0 both builds + 2.1): `GetRandomPosition` steps **22°**: `.so` `(int)(360.0/16)`
  under the truncating control word (the decompile's `*0x17` is wrong), `mp.dll` `r*22`/`+0x16`,
  2.1 the same. The port uses 23° (`rules/frame.rs:314,325`), with f32 trig and no
  normalisation.
- **#229** (3.0 + 2.1): `InterpolateSlowMotion` (`.so` 0x9910c, `mp.dll` 0x10067030) and
  `SavePhysics`' gravity `(s·g)·s` (`mp.dll` 0x100501d4, 2.1 0x10046a70) round once on the x87.
  The port rounds at each step in f32 (`rules/powerup.rs:66`, `weapons/projectiles.rs:142,154`).
  This gives ulp drift in about 25% of cases (simulated).
- **#241** (2.1, validation): 2.1's `PreThink` (0x100537e0) runs `CheckForWires` (FUN_10055650)
  every 0.5 s in the objective mode, as 3.0 does; `ts21_server::rules::OBJECTIVES` sets
  `wire_hud: false` ("no CheckForWires", also in ts21.md), so `check_for_wires`
  (`entities/ts/objective.rs:966`) never runs in 2.1.
- **#245** (3.0 + 2.1, validation): a dead player's power-up drops through
  `drop_powerup(.., p.ts.held_secs)` (`rules/powerup.rs:137,156`); both builds' and 2.1's
  `UpdatePowerUpState` call `CreatePowerUp(type, -1)` (`.so` 0x958a8/0x9592b, 2.1 0x100569db/
  0x10056a4a), i.e. the base duration.

## Coverage: core power-ups and slow motion (`crates/server/src/rules/powerup.rs`, `frame.rs`, `lifecycle.rs`)
| port fn | 3.0 original | 2.1 original | status |
|---|---|---|---|
| `update_state` (+ `PowerupRules` 3.0/2.1) | UpdatePowerUpState 0x95748 (0.6/0.35, 0.8/0.225 btime, drop > 2) | FUN_100568e0 (0.55/0.35, 0.6/0.15, drop > 4, gravity 0.15) | values/branches match; death drop seconds mismatch #245 (re-checked: the row hid it) |
| `drop_powerup` | CreatePowerUp(type, **-1**) 0x97fa0 from UpdatePowerUpState | CreatePowerUp 0x10059c90 (-1) | mismatch #245 (passes held seconds) |
| `use_powerup` | UsePowerUp 0x95980 (trunc), TSMotionBlur PVS 1/2 | UsePowerUp 0x10056a90 (`__ftol`, no blur, same flag 0x10 refusal) | match (re-checked both) |
| `add_powerup` | PlayerAddPowerUp 0x97bc8 (armour 0x80 first; clip needs gun; grenade select 1) | PlayerAddPowerUp 0x100599d0 (no clip case, armour += cap 100, select 0) | match (2.1 re-checked: branch order 0x80,4,0x100,0x40,0x20, held slot, SDK pickup sound) |
| `interpolate` | InterpolateSlowMotion 0x9910c / mp.dll 0x10067030 | none (`time_scale_eases` false) | mismatch #229 (rounding) |
| `go_slow_self` / `go_slow_plain` / `set_slow_target` | CBasePlayer::GoSlow 0x929d0 (observer → 1), CBaseEntity::GoSlow 0xac5e0 | GoSlow 0x10055220 (+ release clamp = `on_go_slow`) | match (re-checked both) |
| `in_slow` / `entity_in_slow` | InSlow 0x9298c, CBaseEntity::InSlow 0xac5f4 (`fuser1 != 1`) | FUN_100551e0 | match (documented 0/1 encoding; re-checked) |
| `slow_flow` / `list_slowed` | SlowMotionFlow 0x96bb0 (glass ignored, 32-cap; knife class string checked in binary) | FUN_100576d0 (writes entity `fuser1` directly) | match (#132 fixed) |
| `deactivate_flow` | DeactivateSlowMotionFlow 0x96a9c | FUN_10057610 | 3.0 match; 2.1 mismatch #223 |
| `remove_from_slowed` | RemoveEntityFromSlowedArray 0x9fb4c | same name (shrapnel caller) | match |
| `hud_slow_amount` values | UpdateClientData (0.35/0.225) | 0x10054d1e (0.35/0.15) | values match (UpdateClientData body = rules/HUD area) |
| `set_plugin_speed` / `plugin_speed_think` / `run_powerup` | FixedSlowAmt 0x990bc (0.01 floor) | n/a | port additions (match the cited floor) |
| `lifecycle::pre_think` slow block | PreThink 0x908cc: Interpolate, framerate = GiveSlowMul, UpdatePowerUpState, SlowMotionFlow | PreThink 0x100537e0 (decomp ~l.2428): framerate `fuser1==0 or >=0.1 ? 1/FUN_100551c0 : 0` (FixedSlowMul 0x1005a3d0, floor FUN_1005a3b0), FUN_1005acf0 (stunt), UpdatePowerUpState, SlowMotionFlow | match (2.1 re-checked; stunt Play order = stunts area) |
| `frame::set_global_slow` / `global_slow2` | SetGlobalSlowMotion 0x7961c, GlobalSlowMotion2 0x79720 | FUN_10045500, FUN_100455f0 | match |
| `frame::put_powerup` | PutPowerUp 0x9797c (32 spots, `RandomLong(0, n-1)`, 128-unit sphere, a refused spot is removed by shifting the array down, not swap-remove) | FUN_10059840 (`CreatePowerUp(1 << (idx & 31), -1)`) | match (re-checked; `Vec::remove` shifts as TS does; port's `kind == 0` guard unreachable) |
| `frame::random_position` | GetRandomPosition 0x974f4 / mp.dll FUN_10065f40 | FUN_10059510 | mismatch #228 (re-checked: `.so` 0x97523 (objdump 0x87523) `fld 360.0; fidiv 16`, RC=0xc, `fistp` → 22) |
| `state::random_powerup` | GetRandomPowerup 0x97894 (`(char)bit >= 0` drops armour, super-jump map drops 0x100) | n/a (2.1 `1 << RandomLong(0,8)`) | match (re-checked) |
| `normal_secs` / `runnable` | n/a | n/a | port additions (plugin API helpers) |

## Coverage: ruleset hooks
| port fn | original | status |
|---|---|---|
| ts30 `powerup::POWERUPS` | SetupPowerUp 0x98014, WaitTouch 0x98614, UpdatePowerUpState 0x95748 | match |
| ts30 `powerup::kind_for` | SetupPowerUp 0x98014 + GetRandomPowerup (armour/superjump redraw) | match (re-checked) |
| ts30 `powerup::fuse_step` | grenade WaitThink 0x7cbbb / mp.dll 0x10051096 | match (round 1/#150) |
| ts30 `powerup::slow_pitch` | EMIT_SOUND_DYN 0xe0aa7..0xe0b1e | match |
| ts30 `anim_framerate` = max(ts, 0.01) | GiveSlowMul / FixedSlowAmt | match |
| ts21 `powerup::POWERUPS` | SetupPowerUp 0x10059ce0, WaitTouch 0x1005a160, UpdatePowerUpState 0x100568e0, UpdateClientData 0x10054d1e | match (re-checked: take/swap conditions, `& 0x1e4`, `< 5`, `> 4`, amounts) |
| ts21 `powerup::kind` / `random_kind` | SetupPowerUp 0x10059ce0 (`superjump` cvar unregistered → `RandomLong(0,8)`) | match |
| ts21 `powerup::armor` / `add_armor` | PlayerAddPowerUp 0x100599d0 (`+0x7c`, `100 <= a`) | match |
| ts21 `powerup::auto_powerups` / `spawns_slowmo` / `random_kind` | Think 0x10043cc0 block (decomp l.113-148: modes 0/1 only, first-Think `ts_powerup` test, `nopowerup != 1`, 21 s reset, `0.25 <= r || slowmatch == 1` → `RandomLong(0,8)`, +20/+1 s) | match (re-derived; the mode gate is the caller's, `rules.rs:286`) |
| ts21 `powerup::is_type` | n/a | port helper | n/a |
| ts21 `powerup::on_go_slow` / `go_slow_release` | GoSlow 0x10055220 tail | match (the caller gap is #223) |
| ts21 `stunt::anim_framerate` | PreThink 0x100537e0 framerate | match |
| ts30 `entities::respawn_entities` | RespawnEntities 0x77764 | match (round 1; not re-checked) |
| ts30 `entities::give_ground_weapon` | GetWeapon 0xa2328 | match (round 1, re-read) |
| ts30 `entities::OBJECTIVES` | `TSHudMessageAll(..,8,..)` (Reset 0x8b850, CutWire 0x8c70c, Use 0x8c9a4/0x8cf14, WaitTouch 0x8d4ec, WinGiver 0x8bfb0); team indexes (CanInteract 0x8c38c); indexed list (AddObjectiveInList 0x7ac00); IsObjective incl. givers; CheckForWires; NaN runs out (mp.dll); SetObjective clears always (0x790b4) | match (re-checked; NaN: the `.so` differs, see objective section) |
| ts21 `rules::OBJECTIVES` | 2.1: flag 1, team names (FUN_100513d0), appending list (FUN_10046630/FUN_10046690), pointer by classname, NaN keeps running, SetObjective 0x10045150 round modes only | `wire_hud: false` is wrong: mismatch #241; the rest match |
| ts30 `entities::round_time_left` | RoundTimeLeft 0x79024 (`round_end != 0 && timer`, manager `+0x94` else `TeamWin(2,0,1,0)`) | match |
| ts21 `rules::round_time_left` | RoundTimeLeft 0x100450f0 (no timer flag; manager `+0x84`; else `TeamWin(2, cashforlose)`) | match |
| ts30 `entities::giver_team_win` | CTSWinGiver::Use 0x8bfb0 `TeamWin(team, (int)value, 1, 0)` | match |
| ts30 `items::pick_up_cost` | PickUpWeapon 0x9314c (kind 4 with single refused) | match |
| ts30 `items::add_health` / `charge_once` / `player_take_armor` | TakeHealth 0xd8760, CRecharge::Use 0xc6fd0, CWallHealth::Use 0xc8d08 | match (round 2 of round 1) |
| ts21 `items::LONG_JUMP` / `long_jump_touch` | CTSSuperJump::Spawn 0x10021910, MyTouch 0x10021950 | match (#133 fixed) |
| ts21 `items::add_health` | CBaseEntity::TakeHealth 0x1000c230 | match |
| ts21 `items::battery_armor` + core battery touch | CItemBattery::MyTouch 0x10021600 (deadflag, `!HEV_%1dP` pct in double: brute-forced 0..100 step 1e-4, equal) | match (#151 fixed) |
| ts21 `items::charger_*` | 0x1001f6e0, 0x10020be0, FUN_100239c0 | match (round 1 #151) |
| ts21 `entities::trigger_touch` / `trigger_fires` | CTSTrigger::WaitTouch 0x10040c00 | match |
| ts21 `entities::spawns` / `key_rule` / `kind_of` / `key_value` / `touch` | 2.1 KeyValues (pointer 0x10050800: `objmsgspec`/`objmsgmerc` only; SlowMotion 0x10040db0 has `slowamt` (string at 0x1009812c, unnamed in strings.tsv)); class list | match (pointer keys and `slowamt` re-checked) |
| ts21 `entities::map_globals` / `map_flags` / `map_globals_keys` | CTSMapGlobals::Spawn 0x10042110, Think FUN_10042190 | match (round 1; rules area) |
| ts21 `entities::breakable_attack_dir` | 2.1 CBreakable damage dir (damage area) | not checked here (damage area) |
| ts21 `entities::give_ground_weapon` | FUN_10066250 (alive, both visible, `PickUpWeapon(id, 0, spawnflags, 1)`, round 2/3 null think, delay 60/1..180, `extra & -(extra>=1)`, empty reserve → `9mmclip1` SDK sound, flag 1, `FireTargets`) | match (re-checked) |
| ts21 `entities::respawn_entities` | RespawnEntities 0x10044400 (breakable, door, door_rotating slot 0; modes 2/3 or LMS ground weapons; LMS power-ups) | match (re-checked) |
| ts21 `explode::spawn_shrapnel` tweaks | CTSShrapnel::Create 0x10046f10, Spawn 0x10046ff0 (visible, iuser4 1) | match |

## Coverage: objectives (`crates/server/src/entities/ts/objective.rs`, added by the validation)
The port's core runs all of this for both rulesets; `ObjectiveRules` carries the differences.
3.0 = `.so` (mp.dll where noted), 2.1 = `mp.dll` 2.1 (several 2.1 functions are misnamed in the
export: `CTSObjective::Freeze` 0x100510c0/0x10051750/0x10051d10 are the three `EnableObjective`s,
the real `Freeze` is FUN_10050f10, vtable slots 60/61).
| port | 3.0 original | 2.1 original | status |
|---|---|---|---|
| `Objectives::clear_list` / `find` / `add` | ClearObjectiveList 0x7abdc, FindIndexByObjective 0x7ae20, AddObjectiveInList 0x7ac00 (find first, `< 32`, 4 texts) | FUN_10046610, FUN_10046630 (append, `> 31` → -1, no texts) | match |
| `Objectives::remove_appended` | n/a | DelObj FUN_10046690 (+ find FUN_10046670 over all 32; slot -1 = count) | match (the 1-byte `DelObj` goes out as 3.0's 2-byte `ShowObj(idx, 0)`: documented 3.0 wire) |
| `send_to_hud` / `hud_entry` / `team_text` / `clip63` | SendObjectiveToHud 0x7ac80 (team index text, `strncpy 0x40`, `Center()`) | inline in CTSObjectivePointer::Use 0x100508f0 (`stricmp("Specialists")` → `+0x54` else `+0x58`) | match |
| `hud_list` | n/a | n/a | port addition (plugin API) |
| `resend_list` | SendObjectiveList 0x7add4 (InitHUD, team change) | none | match |
| `freeze` | Freeze 0x8c310 → ShowObjectiveOnHud 0x7aebc | FUN_10050f10 (freeze → DelObj; unfreeze only re-enables) | match |
| `ran_out` (`nan_time_runs_out`) | mp.dll `test ah,0x41` (NaN runs out); `.so` Bomb 0x8c8b4 `test ah,1; jne` and Hack `<= 0`: a NaN keeps running | 0x1005130a / 0x10051860 `jp` (NaN keeps running) | match for mp.dll (port default); the `.so` NaN branch differs (NaN `round_end` only, not filed) |
| `can_interact` / `may_use` | CanInteract 0x8c344/0x8c38c (-1 or team index) | FUN_100513d0 (0 → "Specialists", 1 → "Mercenary", stricmp) | match |
| `count_players` / `attackers_alive` / `escape_needed` | TSCountPlayers 0x9f238 (empty = all, `!IsAlive && respawn < 0` skipped), `&teams[at]` for `at >= 0` | FUN_10064230 (same), name by `at == 0` | match |
| `hack_time` / `held_use` / `print_seconds_left` | Use 0x8c9a4 / 0x8cf14 (`< 0.5` gap, `fldcw`-truncated `%i`, clamp 0) | Use 0x10051430 / 0x10051870 (disasm: clamp 0, `__ftol`) | match |
| `next_ltime` / `objective_sound` | `ltime + 1.0/0.1` double (`fadd qword`), `RandomLong(0,3)` before `RandomFloat(0.92,1)`, CHAN_STATIC, 1.25, slow 0.5 | same, SDK EMIT_SOUND_DYN 0x10036300 (no slow: `sound_pitch`) | match |
| `show_brush` / `spawn_brush` / `complete` | EnableObjective 0x8c608/0x8cdb8, Spawn 0x8c470/0x8cccc | 0x100510c0/0x10051750, Spawn 0x10050fb0/0x10051690 | match (port guards a null model) |
| `ObjectiveManager` key_value/spawn/reset/target | KeyValue 0x8b46c, Spawn 0x8b7bc, Reset 0x8b850 (4 lines, 8 targets) | KeyValue 0x100503a0 (same 15 keys), Spawn 0x100506a0, Reset FUN_10050710 | match (portvts' 7.0 = `k[7]`, false positive) |
| `ObjectivePointer` key_value/spawn/use_ | KeyValue 0x8ba1c, Spawn 0x8bb44, Use 0x8bbd8 (IsObjective → Enable(1) → Add → every player) | KeyValue 0x10050800 (2 keys; `key_rule`), Use 0x100508f0 (`ts_hack`/`ts_bomb`/`ts_teamescape` by classname) | match |
| `is_objective` / `enable_objective` | IsObjective vtable 0x1c (givers too), EnableObjective 0x20 | by classname | match |
| `Bomb` key_value/spawn/think/use_/enable/cut_wire/`random_wires` | KeyValue 0x8cb68, Spawn 0x8c470, WaitThink 0x8c860 (mp.dll 0x1005c7b0), Use 0x8c9a4, EnableObjective 0x8c608, CutWire 0x8c70c, SetRandomWires 0x8c598 | KeyValue 0x100515a0, Spawn 0x10050fb0, WaitThink 0x100512c0 (`TSExplosion(Center, 300, 0x1040, 2048, 4096)`, Freeze slot 61), Use 0x10051430, Enable 0x100510c0, CutWire FUN_100511a0 (`-1 < n < 3`), FUN_10051080 | match |
| `Hack` key_value/spawn/think/use_/enable | 0x8d1c8, 0x8cccc, 0x8ceb0, 0x8cf14, 0x8cdb8 (mp.dll 0x1005cd10/0x1005cd50) | 0x10051ac0, 0x10051690, 0x10051830, 0x10051870, 0x10051750 | match |
| `TeamEscape` key_value/spawn/think/touch/enable | 0x8d700, 0x8d390, 0x8d6b0, 0x8d4ec (`idx < 0x21`), 0x8d4a0 | 0x10051ee0, 0x10051c20, 0x10051eb0, 0x10051d50 (`-1 < idx < 0x21`), 0x10051d10 | match |
| `check_for_wires` | CheckForWires 0x92f08 (PreThink, mode 3, 0.5 s) | FUN_10055650, called from PreThink (decomp l.2626, mode 3, `+0x710` 0.5 s) | 3.0 match; 2.1 mismatch #241 (gated off) |
| `cut_wire_command` / `scan_i` | ClientCommand 0xad838 `sscanf("%i")` → CutWire 0x92fb4 (IsAlive, `iuser1 == 0`) | pfnClientCommand → FUN_10055700 (same) | match (unparsed `n` guard documented) |
| `send_wires` | UpdateClientData 0x91d00 | UpdateClientData (decomp l.3025-3035, `DAT_100cbc90`) | match |
| `find_managers` / `disable_all` | InitObjectives 0x785d8 (modes 2/3, 32 managers, mode 3, every IsObjective → Enable(0)) | FUN_100449b0 (same; disables by classname, givers reset FUN_10050c90) | match (no score giver in 2.1) |
| `set_objective` / `fire_manager_target` / `has_manager` | SetObjective 0x790b4 (clear always; `RandomLong(0,n-1)`; Reset) | FUN_10045150 (round modes only) | match |
| `dev_objective` | n/a | n/a | port addition |

## Coverage: TS map entities (`crates/server/src/entities/ts/`)
| port | 3.0 original | 2.1 original | status |
|---|---|---|---|
| powerup.rs `PowerUp::key_value` | KeyValue 0x97ebc | 0x10059c10 (`pwupmult` refused by ts21 `key_rule`) | match |
| `PowerUp::setup` / `body` / `duration` | SetupPowerUp 0x98014 | 0x10059ce0 | match |
| `PowerUp::spawn` (+ `item_longjump`, spawner byte) | Spawn 0x98258, CTSSuperJump::Spawn 0xc9f44 | Spawn 0x10059e70 (no spawner byte: the Think-side check gives the same result) | match |
| `fall` / `on_ground` | Fall 0x983cc, OnGround 0x9848c | 0x10059f90, 0x1005a030 | match |
| `think` Waiting | WaitThink 0x98570 | 0x1005a0e0 | match (re-checked; renderamt: port `/30.0*255` in f32, TS `*0.033333335f*255` on the x87: can differ by an ulp, which flips the truncated byte only at an exact integer; cosmetic, not filed) |
| `pickup_plan` / `pickup` / `after_pickup` | WaitTouch 0x98614 | 0x1005a160 (respawn delay clamp the same) | match (2.1 re-checked; swap drop passes -1 like TS) |
| `create` | CreatePowerUp 0x97fa0 | 0x10059c90 | match (TS writes the origin after Spawn without SET_ORIGIN; port relinks at once: one physics frame) |
| triggers.rs `SlowMotion` key_value/spawn/think/touch/deactivate/check_distance/activate | 0x70d74, 0x70e14, 0x712a4, 0x70fdc, 0x71078, 0x7118c, 0x71384 | 0x10040db0, 0x10040e30, 0x10041130, 0x10040fb0, FUN_10041030, FUN_100410a0, 0x100411e0 | match |
| `SlowMotion::use_` | Use 0x71404 (flag 8) | Use 0x10041240 (flag 1) | 3.0 match; 2.1 mismatch #221 |
| `slow_radius` / `slow_inside` | per build (#171) | 2.1 Spawn 0x10040e30 (z,x,y sum, root unrounded, `*0.5` double, `__ftol`); CheckPlayersDistance FUN_100410a0 (`test ah,5; jp`: release only at `radius < len`) | match (2.1 re-checked; one edge: a NaN distance keeps the player in 2.1, releases in 3.0 mp.dll, which the core runs for both; NaN origin only, not filed) |
| `SlowMotionPoint` + `create_slow_motion_point` | 0x714d8, 0x71564, 0x7166c, 0x71728, 0x718ac | 0x100412d0, 0x10041340, 0x10041460, 0x100415b0 | match |
| `DmHill` key_value/spawn/touch/`dmhill_hold` | 0x719b8, 0x71a8c, 0x71be8 | 0x10041660, 0x10041710, 0x10041830 | match except the flag (#221) (3.0 touch re-checked: order of tests, king reset via PlayerRelationship = `same_team` false in DM); NaN `nextcheck`: 3.0 keeps NaN in `max(nc,0.5)`, port `f32::max` gives 0.5 (not filed: map keyvalue "nan") |
| `Trigger` key_value/spawn/touch | 0x70890, 0x709b8, 0x70ad8 | ts21 hook | match (same NaN note for `next_check.max(0.5)` at :171) |
| `Timer` / `CustomTarget` / `PlayerAmount` | 0x72db0.., 0x731bc.., 0x72aec.. | refused in 2.1 | match (round 1) |
| ground.rs `GroundWeapon` key_value/spawn/fall/put_on_wall/touch/use | 0xa19f0, 0xa1b40, 0xa2150, 0xa1cd8, 0xa26cc, 0xa230c | 0x10065bb0, 0x10065c70, 0x100660b0 (+FUN_10066190 OnGround hook), FUN_10065dc0, 0x10066440 (hook), 0x10066240 | 3.0 match; 2.1 match except #224 |
| ground.rs `GroundAmmo` | 0xa2718.. | refused in 2.1 | match (round 1) |
| ctb.rs `Briefcase` / `Flag` / take/drop/throw | 0x8d7f4.., 0x9d874.., 0x9e330, 0x93830, 0x9334c | not in 2.1 | match (round 1; message flags 8/4 re-checked) |
| ctb.rs `WinGiver<SCORE>` | 0x8bdfc, 0x8bf10, 0x8bfb0 | 0x10050bb0, 0x10050ca0, 0x10050d10 (flag 1 through `hud_effect`; latch, counter, reset FUN_10050c90) | match (2.1 Use re-checked; TeamWin team clamp = rules area) |
| ctb.rs `impact` (flying case damage) | Fall 0x9df10 (`.so` 0x9e0e3..), mp.dll 0x100727ff | n/a | match (round 1/#150; portvts flags 0.03 = MSVC's folded 30·0.001, documented) |
| ctb.rs `enable_objective` (giver reset) | CTSObjective::EnableObjective 0x8e6e4 + WinGiver latch | FUN_10050c90 | match |
| misc.rs `DeadCorpse` / `create_dead_corpse` | WaitThink 0x70460, TSCreateDeadCorpse 0x70234 | FUN_100408c0, FUN_100406f0 | match |
| misc.rs MapGlobals / MapFlags / Wind / Fog / Emitter / Particle | round 1 | MapGlobals = ts21 hook; the rest refused | match (round 1) |
| shrapnel.rs `Shrapnel` spawn/think/touch | Spawn 0x7c0a0, WaitThink 0x7c678, BounceTouch 0x7c200 | 0x10046ff0, 0x10047450, 0x10047110 (ATTN 0.8, CHAN_BODY, no slow pitch through `sound_pitch`) | match |
| `weapons::projectiles::SlowPhysics` (shrapnel, grenades, knives, world guns) | SavePhysics 0x7bac0 / mp.dll 0x100500f0, RestorePhysycs 0x7bcd4, WaitThink 0x7bd98 | 0x10046a70, 0x10046c50, 0x10046d10 | logic match; gravity rounding #229 |

## Coverage: stock entities (`crates/server/src/entities/stock/`)
Round 1's tables (git c49ee1a, sections "Coverage (3.0 entities)", "Round 2", "Round 3") stay
valid as far as the scans see: the stock code changed since (x87 lengths and normalisations,
2.1's SDK items/chargers/debris, func_tank, the plugin hooks), but every changed function that cites
a TS address was re-run through the port-vs-TS literal scan (below) with no new hit. The
function-level rows of round 1 (breakable, doors, buttons, trains, tracktrain, plats, func_tank,
maprules, trigger_hurt, items, chargers, env_explosion, env_beverage, cycler, xen) are carried
over as "match (round 1, scan clean)" and were not re-read line by line here.

## Not verified (coverage gaps)
- objective.rs: now covered (section above). Not re-derived: the round rules that call it
  (`CheckDeads`, `RestartRound`'s objective part, the manager's spec/merc-dead targets: rules area).
- `entities/ts/shrapnel.rs` and `SlowPhysics` beyond #229, `DeadCorpse`, `Timer`/`CustomTarget`/
  `PlayerAmount`, `ctb.rs` Briefcase/Flag: carried from round 1, not re-read.
- The `PlayerAddPowerUp` callees: `weapons::give_weapon_select` / `multiply_clip` /
  `take_armor` (weapons area); `UpdateClientData`'s `TSSlowMo` send (HUD area); the commands
  `ts_givepowerup`/`powerup`/`usepowerup` (commands area); the death drops in `Killed` (players area).
- `shared` slow motion: pmove's `fuser1` frame scaling, the 0.1 freeze and footsteps (movement
  area). The client's eased cosmetic clock (client area).
- Stock entities (`entities/stock/*`): only re-read where 2.1 runs them through a hook (items,
  chargers, long jump, battery); the port-vs-TS literal scan was re-run over all of them (see
  Validation), the function-level reading of round 1 was not repeated.
- Edge NaN paths in map keyvalues (`nextcheck`, `duration`, `slowamt`, `pwupmult`): noted, not filed.
- Key parsing `kv_f` vs `strtod`: inputs like `"1e"` or hex floats differ (Rust parse); not filed.

## Validation (round 2, second pass)
- Rows: 63 → 98 data rows (35 added: the objective section, `drop_powerup`, the ts21 `OBJECTIVES`/
  `round_time_left`/entities key rows, ts30 `round_time_left`/`giver_team_win`, `ctb.rs` `impact`/
  `enable_objective`, port helpers; plus a stock-entities summary).
- Missing port functions found: all of `objective.rs` except `Hack` (≈45 fns), `rules/powerup.rs`
  `drop_powerup`/`normal_secs`/`runnable`, ts21 `entities::{spawns,key_rule,kind_of,key_value,touch,
  map_globals,map_flags,map_globals_keys,breakable_attack_dir}`, ts21 `powerup::is_type`,
  ts21 `rules::{OBJECTIVES,round_time_left,giver_team_win}`, ts30 `entities::{round_time_left,
  giver_team_win}`, `ctb.rs` `impact`/`enable_objective`.
- "match" rows re-checked against decompile/disassembly (20): `update_state` (both),
  `use_powerup` (both), `add_powerup` (2.1), `go_slow*`, `in_slow`, PreThink slow block (2.1),
  `put_powerup`, `random_position` (#228 asm), `random_powerup`/`kind_for`, ts21 `POWERUPS`,
  ts21 `auto_powerups` (was "not re-derived"), `PowerUp::think`, `pickup_plan`/`after_pickup`
  (2.1), `DmHill::touch` (3.0), `slow_radius`/`slow_inside` (2.1), ts21 `respawn_entities`,
  `WinGiver::use_` (2.1), ts21 `give_ground_weapon`, ts30 `OBJECTIVES`, `Hack`.
  Wrong: 1 hid a real mismatch (`update_state`'s death drop → #245); 1 description wrong
  (`put_powerup` "swap-remove": TS shifts, as the port does); 2 NaN-only edges noted
  (2.1 `CheckPlayersDistance`, `.so` objective timers). New objective rows: 1 mismatch (#241).
- Unverifiable rows: none were marked so; the one "not re-derived" row (ts21 `auto_powerups`) is
  now verified.
- Mismatch rows: #221, #223, #224, #228, #229 exist, describe the same thing, and their port
  `file:line`s are right at 5032209 (`triggers.rs:277,695`, `powerup.rs:66,319-325`,
  `ground.rs:37-44,423`, `frame.rs:312-327`, `projectiles.rs:142,154`).
- Addresses: ~80 cited `.so`/mp.dll addresses checked against functions.tsv: all exist and
  name the cited function, except 0x10021950 (2.1 long-jump `MyTouch`), which the export names
  `CBaseAnimating::HandleAnimEvent` (a naming artefact; the body is the touch), and 0x100501d4
  (an instruction inside `SavePhysics`, as cited).
- Scripts: round 1's port-dependent scan (`portvts.py`) re-run as `scripts/round2/pv_portvts.py`
  (extended to mp.dll 3.0/2.1 addresses and all cited addresses) over this area's core, ts30 and
  ts21 files and all of `entities/stock`: 14 hits, all false positives (port-only additions, folded
  constants, const generics, `k[7]` indexes; outputs `pv_portvts_area.txt`, `pv_portvts_stock.txt`).
  `diff30_21.py` re-run (`pv_diff30_21.txt`, 237 functions, the same as round 1: it reads only the
  originals); its hits in this area's classes are all explained by the rows above.
  `sdkdiff.py`/`sdkint.py` read only the originals and the SDK, so their round-1 output stands.
- Remaining gaps: the ones listed above, plus `Shrapnel` (2.1 bounce maths) and the `slow_flow`
  2.1 body (FUN_100576d0) were not re-derived in this pass.
