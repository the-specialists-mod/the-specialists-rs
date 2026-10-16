# Bots audit: native TS bots (3.0 vs Windows mp.dll; 2.1 vs 2.1 mp.dll)

Round 2 (Oct 2026), port at 661d918 (after the #200 split). 3.0 = `ts30-decomp/server-win` (mp.dll,
the only 3.0 build with bots), 2.1 = `ts30-decomp/ts21/server-win` (2.1's mp.dll ships the same
bot AI, changed; `CBot` vtable 0x10089220). Method: decomp of every function, disassembly
(`objdump` of `ts_original/dlls/mp.dll` and `ts21/dlls/mp.dll`) wherever rounding, summation order
or constant width matters.

Layout since #200: the ruleset-neutral bot layer is `crates/server/src/bots` (brain, nav, nodes,
files, the stunt/power-up/chat modules, the shared helpers, `rules.rs` = `NativeBotRules`, no
default bodies); TS 3.0's hooks are `crates/ts30/server/src/bots` (`Ts30Bots`: states, vision, buy,
look-around, attack, the `CBot` overrides); TS 2.1's are `crates/ts21/server/src/bots` (`Bots`).
Core code below runs for **both** rulesets unless a hook is named.

Not filed (documented deviations): addbot/quota bots on dedicated servers; `ts_bot_stop`; port
guards (`live()`, RNG index clamps, door-trace limit, buffer caps, `GetClassPtr` not allocating).

## Issues

Round 1 (#154, #155, #156, #157, #162, #163) are all closed; each fix was re-checked against the
binaries (see the table). New in round 2:

- #209 Audit 3.0: bot buy compares the tier roll against `float`s; mp.dll's 0.1, 0.4 and 0.7 are
  `double`s (0.3 is a float).
- #210 Audit 3.0: Navigate's eye probe end rounds `origin.x + view_ofs.x` before adding
  `v_forward.x * 64` (3.0 and 2.1 keep it on the x87). Core code, both rulesets.
- #213 Audit 2.1: core nav and the view cone use 3.0's arithmetic under 2.1: Navigate's three
  distances sum `z, y, x` in 2.1 (3.0 `x, z, y`); GetSteerPoint's forward detour keeps
  `v_forward * 48` unrounded in 2.1; 2.1's `FInViewCone` (0x100122a0) rounds `dy`, `1/len` and
  not the direction, unlike 3.0's (0x10014ab0).

## Coverage (port fn -> 3.0 original / 2.1 original -> status)

| Port function | 3.0 (mp.dll) | 2.1 (mp.dll) | Status |
|---|---|---|---|
| `server/rules/commands.rs` cmd_addbot / cmd_addcustombot | 0x10026110 / 0x100261a0 | 0x1001e660 / 0x1001e6c0 | match (round 1; unchanged) |
| `bots::add_bot` / `add_custom_bot` / `pick_model` / `choose_model` | BotCreate 0x1006b7c0 / BotCreateCustom 0x1006b590 | 0x1005dc30 / 0x1005da10 | match (hooks `random_model_in_teamplay`, `learn_on_empty_graph` per ruleset) |
| `bots::frame_strings` (model[32]/name[64], #191) | BotCreate frame | same frame | match (+ documented guard) |
| `bots::create` / `bot_backend::create_player` | BotSetup 0x1006b3b0 | 0x1005d8c0 | match (`stored_skill`, `team_key` hooks: 3.0 floor 0.1 + team key; 2.1 none) |
| `bots::ts_init` / `load_texts` / `texts_for_ruleset` | CTSGameRules::Init 0x1004d910 → BotLoadFiles 0x1006bee0 | 0x1005e180 | match (2.1 texts embedded, byte-identical) |
| `files::BotNames::parse` / `pick` / `scan_f32` | BotLoadFiles / BotPickName 0x1006c150 | 0x1005e180 / 0x1005e380 | match (round 1) |
| `files::BotSay::parse` / `pick`, `format_line`, `say_text` | BotLoadSay 0x1006bc00, BotSayLine 0x1006bbc0, BotSay 0x1006e7a0 | 0x1005df50 / 0x1005df10 / 0x100604a0 | match (re-checked 3.0 disasm: `sprintf(buf, line, name)` once, 0x1006e405..0x1006e41e). **V**: confirmed — BotSayLine takes only the section (`add esp,4` leaves the name pushed as sprintf's third argument; Ghidra's two-argument `sprintf` is a mis-decompile) |
| `bots::start_frame` | pfnStartFrame 0x10010e00, BotPlayerByIndex 0x1006b360 | 2.1 pfnStartFrame | match (+ documented deviations) |
| `bots::spawn_pre` / `Brain::reset` / `Nav::reset` | CBot::Spawn 0x1006bb40, Reset 0x1006f2c0, CBotNav::Reset 0x1006f810 | 0x1005def0, 0x10061220, 0x10061790 | match both (**V: both Reset disasm re-checked**: memsets, section 1, chat timer) (memsets 0x1a24+0x50 / 0x1924+0x64 vs 2.1's 0x19f8+0x120 / 0x18f8+0x64 cover the same fields; chat timer `RF(60,180)*clamp(skill,.5,1)`; state by mode byte) |
| `Ts30Bots::spawn_post` → `laser_colour` | CBot::Spawn tail | none (2.1 slot 3 = Reset+Spawn) | match |
| `Ts30Bots::trace_attack` → `turn_to_attacker` | CBot::TraceAttack 0x1006bac0 | none | match |
| `bots::killed` | CBot::Killed 0x1006ba00 | 0x1005de40 | match (**V: re-checked 3.0 decomp**: attacker `Classify()==2`, `RL(0,100) > 90`, killer netname, `RF(0.5,2)+time`) |
| `bots::on_kill` + scoring call sites (`ts30/server/scoring.rs`, `ts21/server/scoring.rs`) | CBot::OnKill 0x1006b970; PlayerKilled 0x1005b190 | 0x1005ddd0; 0x1004fe02 | match both (#157 fix: both call `bot_backend::on_kill` when points > 0, then `plugins::on_kill`). **V**: 2.1 call site re-checked (0x1004fe02: `points > 0 && m_bIsBot`); 0x1005b190 / 0x1004fe02 are call sites inside PlayerKilled 0x10059d10 / 0x1004f7e0 |
| `brain::think` / `think_with` | CBotBrain::Think 0x1006f500 | 0x100614e0 | match (module order; hooks `runs_ai`, `turn_rate`, `update_buttons`, `respawn`). **V: 3.0 re-checked** (order 0x10,0x100,0x40,4,0x80,8,0x200,0x400,1; turn rate `clamp(skill,.5,2)*10`; `button` before RunMove) |
| `brain::respawn_now` / ts21 `respawn` | inline 0x1006f5b8 | 0x1006155c (press ATTACK 0.1, SetMove 0) | match both (**V: 3.0 re-checked**: `0 < respawn_at < time`, StopObserver, impulse 0, SetMove 0) |
| `brain::run_move` | RunMove 0x1006e6d0 | 0x100603d0 | match both (msec window, clamp 5..100). **V: both disasm re-checked** (`next > time` → frames++; `time + 0.5` double; `450.0/frames` double; clamps; ftol; v_angle/forward/side/button/msec) |
| `brain::update_view` / `punch_step` / `turn_fraction` | UpdateView 0x1006e500 | — (2.1 uses `change_yaw`, next row) | match (round 1 #45) |
| ts21 `change_yaw` (`update_view` hook), `turn_rate` 30 | — | ChangeYaw 0x10060120, Think 0x100614e0 | match (**V: new row, disasm re-checked**: dead/equal early-outs, `speed = turn*ft*10` on the x87, `mv` stored as float, wrap ±180 by ideal-vs-cur compare, clamp to ±speed, `angle_mod(mv + cur_x87)`) |
| `Brain::set_ideal_toward` / ts21 `look_at`, `set_ideal_angles` | LookAt 0x1006f740, SetIdealAngles 0x1006e4e0 | 0x100616b0, 0x10060090 | match |
| `Brain::press_bits` / `update_bits` / ts21 `Buttons` | 0x1006f470 / 0x1006f4b0 | 0x100613e0 / 0x10061450 / 0x10061490 | match |
| `brain::delay` / `Ts30Bots::delay` / ts21 `timer`, `slow_inv` | §5.8 formula (all module sites) | FixedSlowMul 0x1005a3d0 sites | match (**V**: re-checked at LookAround 0x1006de37, Stunt, PowerUp, Vision (3.0) and Stunt/PowerUp/Vision/LookAround/DM state (2.1); only a NaN quotient differs, unreachable) |
| `brain::clamp_skill`, `angle_mod`, `angle_mod_x87` | Think/Reset clamps, UTIL_AngleMod 0x10076250 | 2.1 uses its own `angle_mod` (0x100668f0) in its hooks | match (**V**: clamp re-checked in Reset 3.0/2.1 and Chat 3.0) (see gaps: core `move_toward` uses 3.0's AngleMod under 2.1 too; differs only below -360°) |
| `Ts30Bots::state_think` → `states::think` / `pick_dm` / `pick_team_rounds` | CBotStateDM::Think 0x1006e900, TeamRounds 0x1006ed00 | — | match (hill distance ZXY in disasm 0x1006eb46). **V: DM re-checked** (timer `min(0.1/slow,1)`, branch order, hill-too-close returns without a spawn fallback, teamplay team draw + fallback class, `next_pick = time+3`) |
| ts21 `state_think` / `pick_dm` / `pick_team_rounds` / `team_rounds_class` | — | 0x100605f0, 0x10060950, 0x10061020 | match (decomp re-read; hill ZXY disasm 0x10060843). **V: DM re-checked** (timer `slowInv*0.1` max 1, chase sets keepGoal 1, no keepGoal write while a goal is held, spawns always `info_player_deathmatch`) |
| `states::armament_empty`, `find_by_class`, `random_entity`, `center` | 0x1006e4c0, UTIL_FindEntityByClassname, TSFoundRandomEntity 0x10073a30, Center | 0x10060000, 0x100649c0 | match |
| `Ts30Bots::vision` → `modules::vision` (ts30) | CBotModuleVision::Scan 0x1006dc10 | — | match (**V: re-checked**: both lists cleared, `IsPlayer`/`IsAlive` (0x1000a9d0: deadflag 0 && health > 0)/not self/no EF_NODRAW, no observer test, cone then FVisible, `break` after the first free slot, count++) |
| ts21 `vision` | — | 0x1005f890 | logic match (**V: re-checked**: only enemies cleared, duplicate skip, observer + EF_NODRAW skip, no `break`, count = 1, PlayerRelationship 0x10039420 = both team names set + `_stricmp`); its `in_view_cone` is 3.0's: **mismatch #213** |
| `modules::in_view_cone` / `cone_dot` | CBaseMonster::FInViewCone 0x10014ab0 | 0x100122a0 (differs) | 3.0 match (disasm); 2.1 **mismatch #213** (**V: both disasm re-checked**; FOV `[+0x12c]` is 0.5 in both: 3.0 0x1005e787, 2.1 0x10052ad7) |
| `modules::visible` | CBaseEntity::FVisible 0x10014cb0 | 0x10012460 | match both (disasm: eye → point, ignore monsters + glass, fraction == 1). **V: both re-checked** |
| `modules::add_item` / `add_ammo` / `reset_armament` | AddItemToArmament / AddAmmoToArmament / ResetArmament | 2.1 same | match |
| `modules::random_attachments` / `buy_weapon` | BotRandomAttachments 0x1006d560 | 0x1005f250 | match |
| `Ts30Bots::buy` → `modules::buy` (ts30) | CBotModuleBuy::Think 0x1006d630 | — | ids/ammo/draws match; tier compares **mismatch #209** (**V**: constants read from the binary: 0.3f, 0.75/0.1/0.4/0.7 doubles; kevlar and SMG/rifle branch structure re-checked) |
| ts21 `buy` | — | 0x1005f320 | match (round 1; double compares). **V: re-checked** knife/katana/seal branches, `f <= 0.1` nine knives, all pistol ids/ammo and akimbo draws, SMG/rifle gates; `random_attachments` vs 0x1005f250 |
| `Ts30Bots::look_around` → `look_around` / `find_look_point` | 0x1006dde0 / 0x1006dea0 | — | match (**V: disasm re-checked**: `0.5/slow/skill` max 5; the normalised direction at 0x1006df69 is discarded, so the unnormalised `(cos, sin, sin pitch)` × 1024 is right; ZYX length, `>` best; yaw += 45f) |
| ts21 `look_around` | — | 0x1005fa90 | match (round 1). **V: re-checked** (`2·slowInv/skill` max 5, ±179 / ±60 branches, uninitialised-angles case skipped as documented, turn `RF(2,4)*30`) |
| `modules::stunt` + `stunt_mode` hooks | 0x1006e130 | 0x1005fc40 | match both (2.1 decomp re-read: only the `RL(1,2)` roll and the timer differ, both hooked). **V: both re-checked** (3.0 `RL(0,100) < 26` → flip; `0xc30` mask; `time + 0.2` double at 0x1006e238) |
| `modules::powerup` | 0x1006e280 | 0x1005fd90 | match both (**V: both re-checked**); `use_powerup` body is the power-ups area (2.1 UsePowerUp 0x10056a90 has no TsMBlur, gated by `motion_blur()`) |
| `modules::chat` / `bot_say` | 0x1006e330 / 0x1006e7a0 | 0x1005fe40 / 0x100604a0 | match both (**V: chat re-checked** both: `bottalk`, due-or-queued, `RF(60,180)*clamp(skill,.5,1)+time`, non-empty test, back to freetime) |
| `Ts30Bots::attack` → `attack::think` / `track_point` / `decay_lead` | CBotModuleAttack::Think 0x1006c520 | — | match (melee circle `(rf1, rf2, dz)` ZYX re-checked in disasm 0x1006c847..0x1006c8c9) |
| ts30 `select_enemy` / `compute_aim_offset` / `find_cover_spot` | 0x1006cec0 / 0x1006c3c0 / 0x1006cbc0 | — | match |
| ts30 `fire_weapon` / `knife_flight_time` / `knife_lead` / `burst_next_fire` | FireWeapon 0x1006d010 | — | match (#154 fix verified: tt on the stack 0x1006d14f, tt·v stored, ×r stored) |
| ts21 `attack::think` / `select_enemy` / `find_cover_spot` / `fire_weapon` / `release_attack` | — | 0x1005e5e0 / 0x1005ed60 / 0x1005e9a0 / 0x1005ee60 / 0x1005f230 | match (aim offset `x·(1/s)` re-checked in disasm 0x1005e772..0x1005e7ac) |
| ts21 `attack::kung_fu_wait` | — | FireWeapon 0x1005f193..0x1005f222 | match (**V: new row, disasm re-checked**: `slowInv*0.3/skill` with double 0.3, `<= 0.3` → 0.3, `>= 3` → 3). The doc comment's "a NaN gives 3" is wrong (mp.dll gives 0.3), unreachable (finite skill); not filed |
| `attack::holding_melee`, ts30 `can_stock_hit` | 0x1006cfc0 / 0x1006cf90 | 0x1005ee10 | match |
| `weapons::get_better_weapon` (bots' callers) | CTSGun::GetBetterWeapon 0x100548d0 | 0x1004a410 | match (#162 fix; body = weapons area) |
| `weapons::drop_weapon(Drop::Current)` from bots | DropWeapon(-1, 0) | 0x10056080 | call semantics match; body = weapons area |
| `nav::navigate` (core) | Navigate 0x1006fc00 | 0x10061b70 | 3.0: eye probe x **mismatch #210**, rest match (XZY distances disasm 0x1006fd29/0x1006fdbd/0x1006ff50); 2.1: eye probe **#210**, distances ZYX **mismatch #213**, ladder hook match (#156 fix). **V**: eye probe re-checked in both (3.0 0x100700dc..0x1007014e, 2.1 0x10062047..0x100620af), 3.0 XZY at 0x1006fd29 and 2.1 ZYX at 0x10061c62/0x10061d4f/0x10061ecf re-checked |
| `nav::move_toward` | 0x10070381..0x100703e8 | 0x1006228b..0x1006230c | match both (2.1's AngleMod differs only below -360°, see gaps). **V: both disasm re-checked** (b stored and kept, `b <= a` → `|360-d|`, `-sin·speed`, `cos·speed`) |
| `nav::replan` / `stuck_window` | Replan 0x1006fa40 | 0x100619b0 | match both |
| `nav::random_wander_point` + `wander_end` hooks | 0x1006f8c0 | 0x10061840 | match both (3.0 disasm: x/y products stored) |
| `nav::steer_point` + `steer_probe_end` hooks, `is_door` | GetSteerPoint 0x10070400 | 0x10062330 | 3.0 match (disasm: all ×48 products stored); 2.1 forward detour **mismatch #213** (side detours match). **V**: 3.0 forward-detour stores (0x10070617..0x10070676), 2.1 unrounded products (0x100624ac..0x100624ef), 2.1 side detours (`r*16`, `r*48` stored, 0x10062538..0x100626cc) and 2.1 probe end (z on the x87, 0x10062418) re-checked |
| `nav::on_ladder`, `clear` | IsOnLadder 0x10033670, UTIL_TraceLine | same | match |
| `nodes::*` graph (add, too close, in air, link, find path, push/pop, between points, load/save, commands, learn) | CNodeGraph 0x10070e30..0x10071e60, PreThink 0x1005fed2 | 0x10062bb0..0x10063b50 | match (round 1 line by line; reference-path tests; unchanged since) |
| `nodes::ladder_link_distance` / `ground_probes` (3.0) | IsReachable 0x100711e0 / IsGroundContinuous 0x10070fe0 | — | match |
| ts21 `nodes::ladder_link_distance` / `ground_probes` | — | 0x10062ed0 / 0x10062d50 | match (#163 fix re-checked instruction by instruction 0x10062d66..0x10062e7a) |
| `bots::learn` + PreThink order (`player::pre_think`) / `nodes::learn` | PreThink 0x1005fed2 | 0x100537e0 | match (#155 fix: learning, `CheckForWires`, then the end steps). **V: 3.0 learn block re-checked** (`botlearn > 0`, `next < time`, `time + 0.1` double, ladder flag, AddNode 0x10070e30) |
| `nodes::map_init` / `save` / `client_command` / `atoi` / `EngineWorld` | Load 0x10071cb0, Save 0x10071e60, pfnClientCommand 0x1000f8a0 | 2.1 same | match (round 1; covered by the `nodes::*` row; `save` path is a documented portable equivalent) |
| `bots::client_command` | pfnClientCommand 0x1000f8a0 | 0x1000d4b0 | match |
| `bots::backend::Native`, `rules::register` / `for_id` / `current` | (dispatch / ruleset registry only) | | not original logic |
| `bots::add_quota_bot` | port addition (`ts_bot_quota`) | | documented deviation |
| `Brain::new` | `GetClassPtr<CBot>` 0x1006c240 zero-filled object | 2.1 same | match (port guard: no allocation in `GetClassPtr`, documented) |
| `bot_backend/` (`mod.rs`, `quota.rs`) + `plugin-api/src/lib.rs` bot table | port addition | | not original logic (path corrected: there is no `plugin_api/bots.rs`) |

Counted (corrected in validation): 57 rows before validation (the auditor's "62" was a miscount), 62 after;
the mismatch rows are 5 (ts30 buy, ts21 vision, in_view_cone, navigate, steer_point) carrying 3 issues
(#209, #210, #213). All 6 round-1 fixes verified in place.

## Not verified / limits

- Engine `RandomFloat`'s return precision on the x87 (e.g. the melee circle's second draw is used
  straight from `st0`) can't be modelled; assumed a float everywhere.
- Core `nav::move_toward` uses 3.0's `UTIL_AngleMod` for both rulesets; 2.1's (0x100668f0, the
  SDK's) only differs for angles at or below -360°, which a bot's yaw (from `VecToAngles` and
  `change_yaw`) never reaches in practice. Not filed.
- NaN skill: 2.1 stores the skill unfloored, and TS's clamps map a NaN to a bound (validation:
  `Reset` in both builds maps it to 0.5, the kung-fu timer to 0.3) where the port's
  `clamp_skill` keeps NaN and `kung_fu_wait` gives 3. Unreachable unless
  `addcustombot`'s skill parses to NaN (the port's `atof` doesn't); not filed.
- `UsePowerUp`, `DropWeapon` and `GetBetterWeapon` bodies belong to the power-ups and weapons areas.
- `nav::move_toward` (core) uses 3.0's angle mod under 2.1; re-checked: 2.1's `move` sequence is
  otherwise identical (0x100622a2..0x1006230c).
- 2.1's attack, look-around and buy hooks were spot-checked (aim offset, timers) rather than
  re-derived in full; round 1's full check stands for them (code unchanged since).

## Validation (independent check, Oct 2026)

Re-checked against the decompile and `objdump` of both mp.dll builds (port at 14240b0; the one
commit since 661d918 changes no bot code, only docs/implementation/bot-plugins.md).

- Completeness: every `fn` in `crates/server/src/bots`, `crates/ts30/server/src/bots` and
  `crates/ts21/server/src/bots` enumerated. Missing or only vaguely covered, now rows: ts21
  `change_yaw` (+ `turn_rate`), ts21 `kung_fu_wait`, `nodes::map_init`/`save`/`client_command`/
  `atoi`, `Brain::new`, `bots::add_quota_bot`, the `rules` registry. The two gameplay-relevant
  ones (`change_yaw`, `kung_fu_wait`) were compared in the disassembly: match. The rest are
  helpers (`gun_position`, `slow`, vector maths, `set_goal`, ts30 `attack::release`) covered by
  their callers' rows.
- Match rows re-checked (marked **V**): 23 of the 57 non-mismatch rows, plus all 5 mismatch rows (Reset ×2, Think, respawn,
  RunMove ×2, change_yaw, the module timers, clamp_skill, both DM states, both visions, visible
  ×2, both buys, ts30 look-around + FindLookPoint, ts21 look-around, stunt ×2, power-up ×2,
  chat ×2, BotSayLine/sprintf, killed, on_kill call site, move_toward ×2, 2.1 steer side detours
  and probe end, nodes learn, kung_fu_wait). **None was wrong.** One port doc-comment error found
  (`kung_fu_wait`: NaN gives 0.3 in mp.dll, not 3; unreachable, not filed).
- Mismatch rows: #209, #210, #213 exist, are open, labelled `bug`, and describe the same
  findings; every claim in them was re-verified instruction by instruction (buy constants read
  from the binary; eye-probe x in both builds; 2.1's ZYX sums at all three sites; 2.1's unrounded
  ×48 products; both FInViewCone sequences). Port line refs (nav.rs:99/114/120/137/259,
  ts30 modules.rs:88/106/119, ts21 mod.rs:489) are correct at 14240b0.
- Unverifiable rows: none in the table; the "Not verified" items are modelling limits
  (x87 RandomFloat precision) or out-of-area bodies, and are real.
- Addresses/paths: all 100+ cited function addresses exist in functions.tsv except 0x1005b190,
  which is a call site inside PlayerKilled 0x10059d10 (noted in the row). Path fixed:
  `plugin_api/bots.rs` doesn't exist (now `bot_backend/` + `plugin-api/src/lib.rs`).
- No new issues filed.
- Remaining gaps: 2.1's attack module (select_enemy, find_cover_spot, the gun branches of
  fire_weapon), the node graph and BotCreate/BotSetup were not re-derived in this pass (round 1's
  line-by-line check and the reference-path tests stand); ts30 attack/FireWeapon not re-sampled.
