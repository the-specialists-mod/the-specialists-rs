# Audit: rules (3.0 + 2.1), round 2 — port 661d918

Method: decomp + disassembly, function by function (method.md). Binaries: `ts_i386.so`
(~/scratch/agents/servers/orig/ts/dlls; Ghidra addr = objdump addr + 0x10000), 2.1 `mp.dll`
(~/dev/hl/ts21/dlls; disassembly `objdump -M intel`, strings read from `.rdata`). Scratch helpers in
scripts/round2/ (`fn.py` dumps a decomp function by address or name, `pe.py` reads 2.1 data/strings and
lists the cvar_t table). Focus of this round: the core code both rulesets run with no hook (after
#200 phase 1) checked against 2.1's own functions, and the 2.1 plugin's new round/objective code.

## Issues filed (round 2)
- #231 Audit 2.1: InitHUD order — 2.1's `CHalfLifeMultiplay::InitHUD` (0x10023ec0) calls
  `CTSGameRules::InitHUD` (0x100451c0) at its end, before the teamplay part's
  `ChangePlayerTeam(team,0,0)` (forced `SetPlayerRespawnTime(5,1)`, cash floor); the port runs
  `ts_init_hud` after `teams::init_hud` and overrides the forced respawn (round-TDM joiners play the
  current round) and the floor. Also: 2.1 sends no `Spectator` list, and its cheat notices are
  "This server have ..." with effect 1.
- #233 Audit 2.1: `Host_Say` (0x1000cf00): no `%a`/`%h`, whole text `_strlwr`'d, `ff` reply (not
  broadcast), dead-chat gate in modes 2/3 and with the `lastmanstanding` cvar in any mode.
- #234 Audit 3.0 (+2.1): chat cut at 127 − prefix (TS 126 − prefix: 0xad02c `0x7f - ~len`,
  0x1000cf00 `0x7e - len`); `TSInit` zeroes the chat flood timer (3.0 +0x7c0, 2.1 +0x738), the port
  keeps `next_say`.
- #235 Audit 3.0 (+2.1): `SendMOTDToClient` sends the chunk crossing 0x600 whole (port cuts at
  0x600), no `motd.txt` fallback; `CHalfLifeMultiplay::InitHUD`'s `SVC_INTERMISSION` for a joiner
  during the intermission (reachable through `fullupdate`) is missing.
- #236 Audit 2.1: round HUD texts: "Specialists Win", "Mercenaries Win", "Game Draw",
  "Defeat the opposite team", LMS "Waiting other players" (no "!"/".").
- #237 Audit 2.1: `kill` is the SDK's immediate suicide with a 1 s cooldown (`pfnClientKill`
  0x1000ce50); 2.1's `Killed` (0x10052390) has no `SET_VIEW` / `EnableControl(TRUE)`.
- #238 Audit 2.1: `ChangePlayerTeam` (0x10038cb0) takes any non-empty name (no `IsValidTeam`, no
  The One override); `SetDefaultPlayerTeam` keeps 16 bytes of `mp_theone`/`mp_theothers` (port 15);
  comment: `ClientUserInfoChanged` (0x10038f50) has no team-limit branch, so a model change is a team
  change in 2.1 TDM (port silently resets the model).
- #239 Audit 2.1: `TSSelectSpawnSpot` (FUN_10043010) has no `IsTriggered` (master) test.

- #250 (validation pass) Audit 2.1: `TheOneIsDead` (0x10046930) calls only `SetDefaultPlayerTeam`
  (vtable +0xdc = FUN_10038920) on the victim and the new One; the port's core `the_one_is_dead` runs
  3.0's `SetDefaultPlayerTeam` + `ChangePlayerTeam`, whose 2.1 tail sends the new One to observer
  with a 5 s respawn and resets both players' cash; also "The One is dead" (no full stop).
- #231 comment (validation pass): 2.1's `InitHUD` sends `SrvSett` 0 as `realbullet.value != 0`
  (0x10045278); the port's core sends 3.0's truncated copy (`realbullet 2` → 2, which 2.1's client
  bullet queue (`== 1`) ignores).

All are #200 phase-2 paths (core code with no hook) except #234/#235 (both rulesets wrong) and
#236 (2.1 plugin).

## Coverage (core, both rulesets) — `crates/server/src/rules`
| Port function | 3.0 original | 2.1 original | Status |
|---|---|---|---|
| mod.rs `game_init`, `install`, `exec_server_config`, `start_rules` | GameDLLInit 0xc5aa0, InstallGameRules 0xc6050, multiplay ctor 0xcedd0 | pfnGameInit 0x1001e750, InstallGameRules 0x1001eba0, ctor 0x10023850 | match (round 1; plugin selection is a port addition, per gameplay-plugins.md §4) |
| mod.rs `game_description` | GetGameDescription 0xaee84/0xd0c68/0xe5a04 | — | match; 2.1 text is a documented deviation |
| lifecycle `client_connect` | ClientConnect / ClientConnected 0xcf59c | pfnClientConnect 0x1000cd30 | match (ts_playeramount is a 3.0 entity, refused in 2.1) |
| lifecycle `client_disconnect`, `client_disconnected` | 0xacd4c, 0xcf960, TSPlayerDisconnect 0x9f94c | 0x1000cd50, 0x10024120, FUN_100648e0 | match (2.1 sets no FL_DORMANT: no observable effect, ts21 never tests it) |
| lifecycle `client_kill`, `pre_think_end` suicide | ClientKill 0xacec8 | pfnClientKill 0x1000ce50 | 3.0 match; 2.1 mismatch #237 |
| lifecycle `userinfo_changed`, `clean_name`, `name_change_notice` | 0xae638 | pfnClientUserInfoChanged 0x1000e110 | match |
| lifecycle `multiplay_userinfo_changed`, `player_is_cheating` | 0x8b244, 0x9fae0 | 0x10050210, FUN_10064a20, IsValidTeam 0x10050140 | match |
| lifecycle `on_spawn` (TSInit part) | TSInit 0x92a4c | TSInit 0x100552f0 | mismatch #234 (chat timer, both); render/fade/speed/mj/Spectator/the-One reset match |
| lifecycle `player_think` | PreThink head, PlayerThink 0xcfb60 | PreThink 0x100537e0 head | match |
| lifecycle `pre_think` order | PreThink 0x908cc | PreThink 0x100537e0 | match for the rules steps (water, UpdateClientData, TBD, suit, framerate, power-ups, overheal, observer, death think) |
| lifecycle `init_hud_block` | UpdateClientData 0x91d00 | 0x100547c0 | 3.0 match; 2.1 mismatch #231 (order) |
| lifecycle `rules_init_hud` | CHalfLifeMultiplay::InitHUD 0xcf618 | 0x10023ec0 | mismatch #235 (SVC_INTERMISSION, both) |
| lifecycle `ts_init_hud` | CTSGameRules::InitHUD 0x79134 | FUN_100451c0 | 3.0 match; 2.1 mismatch #231 (Spectator loop, notice text/effect; **V**: also `SrvSett` 0 = live `realbullet != 0` in 2.1, #231 comment); keys 1..3, RoundTime (2.1 round timer always on, same clamp), tail resets (respawns, grenades, tk, respawn time, armament, objective list, vote slot) **V** match |
| lifecycle `send_motd`, `motd_chunks` | SendMOTDToClient 0xd0ae4 | 0x10024ee0 | mismatch #235 |
| lifecycle `start_observer` | StartObserver 0x99634 | 0x1005a400 | match (2.1 has no DropFlag: no CTB in 2.1) |
| lifecycle `stop_observer` | StopObserver 0x99a14 | FUN_1005a700 | match |
| lifecycle `killed` | Killed 0x8ed5c | Killed 0x10052390 | 3.0 match; 2.1 mismatch #237 (SET_VIEW, EnableControl); 2.1's headshot slow writes fuser1/display directly (port `go_slow_self` also clears the bullet-time flag: not observable on a dead player) |
| lifecycle `death_think`, `death_slowdown` | PlayerDeathThink 0x8eaec | 0x10052140 | match (2.1's IEEE NaN compares only differ for a NaN velocity) |
| lifecycle `respawn_command`, `can_respawn` | ClientCommand "respawn" | — (2.1 refuses the command) | match |
| frame `multiplay_think`, `time_limit`, `update_limit_cvars` | 0xcf038 | FUN_10023a60 | match (2.1's IEEE compares give the same results for NaN limits) |
| frame `teamplay_think` | 0xe412c | 0x100386d0 | match |
| frame `intermission_think`, `go_to_intermission` | 0xcf038 game-over branch, 0xd0050 | FUN_10023a60 game-over branch | match |
| frame `set_global_slow`, `global_slow2` | SetGlobalSlowMotion, GlobalSlowMotion2 0x79720 | FUN_10045500, FUN_100455f0 | match (2.1's 0 = normal encoding mapped to 1) |
| frame `reset_restart_round` (3.0) | ResetRestartRound 0x78750 | — | match (#122 fixed) **V** (+1 s, RoundTime clamp, score/cash 0 and TeamScore per named team, "Restarting Round" channel 8) |
| frame `put_powerup`, `random_position` | PutPowerUp 0x9797c, GetRandomPosition 0x974f4 | 0x10059840 | see #228 (other area) |
| commands `client_command` (core part) | ClientCommand 0xad838 | pfnClientCommand 0x1000d4b0 | match for fov/use/changeteam/jointeam/cutwire_/voice/menuselect/unknown text |
| commands `host_say`, `say_*`, `format_say`, `clip_say_text` | Host_Say 0xad02c | 0x1000cf00 | mismatch #234 (both), #233 (2.1) |
| commands `force_client_dll_update`, `cheat_powerup` | 0x92594, ClientCommand | — / 2.1 powerup (ts21 hook) | match |
| commands server commands (globalslow, weaponrestriction, restartround) | 0xc5580, 0xc58a0, 0xc5748 | cvars polled by Think | match (#140 fixed; 2.1 as polled cvars, documented) |
| teams `set_default_team`, `default_team_kept` | SetDefaultPlayerTeam 0xe44ac | FUN_10038920 | match (2.1 hook `default_team`: 15 vs 16 bytes, #238) |
| teams `change_team`, `change_team_choice`, `change_team_model` | ChangePlayerTeam 0xe488c | FUN_10038cb0 | 3.0 match; 2.1 mismatch #238 (head; tail hooked, #142) |
| teams `init_hud`, `init_hud_notice` | Teamplay InitHUD 0xe45d8 | 0x10038ac0 | 3.0 match; 2.1 order #231 |
| teams `userinfo_changed` | 0xe4d18 | 0x10038f50 | 3.0 match; 2.1 mismatch #238 (comment: no team-limit branch, The One re-runs SetDefaultPlayerTeam) |
| teams `recount`, `score_team`, `fewest`, `same_team` | RecountTeams 0xe5608, GetTeamIndex 0xe5434, 0xe5510, PlayerRelationship 0xe5278 | FUN_10039730, FUN_10039620 | match **V** 2.1 (`fild score; fadd frags; ftol`, append without a limit, TeamInfo for valid teams; fewest counts every slot's TeamID, strict `<` from 32) |
| scoring `player_killed` (core part), `killer_score_info` | PlayerKilled 0x895e4 / Teamplay 0xe51b8 | 0x1004f7e0 / FUN_100392e0 | match (death-penalty skip, ScoreInfo tail) **V** (2.1: victim then Classify-2 killer ScoreInfo + decal timer, `FUN_10045910` CheckDeads timer) |
| scoring `the_one_is_dead`, teams `the_one_switch(_other)` (not listed before) | TheOneIsDead 0x7b258 (+0xe0 SetDefaultPlayerTeam, +0xdc ChangePlayerTeam) | TheOneIsDead 0x10046930 (+0xdc = SetDefaultPlayerTeam only) | 3.0 match; 2.1 **mismatch #250** (core runs 3.0's ChangePlayerTeam: new One to observer, cash reset; "The One is dead" text) |
| scoring `team_kill_punish`, `team_kill_punishment` | PlayerKilled tail | 0x1004fe1b.. (strings 0x10098a48..0x10098ad8, `banfortk`, ClientPrint console / PrintAll talk) | **V** match both |
| scoring `death_notice` | DeathNotice 0x88f7c / 0xe509c | FUN_1004ede0 / 0x10039200 | match (teammate test, director event) |
| scoring `weapon_name_with`, `log_kill*`, `team_kill_punish`, `points_for_kill`, `active_item_kung_fu`, `do_spectacularness` | PlayerKilled parts, 0x76b48 | 0x1004f7e0 parts | match (round 1 + #119/#131/#148 fixes) |
| spawnpoint `select`, `spot_score`, `telefrag`, `place` | EntSelectSpawnPoint 0x76128, TSSelectSpawnSpot 0x76244 | FUN_10042f50, FUN_10043010 | 3.0 match; 2.1 mismatch #239 (master test); the rest matches |
| votes `parse`, `player_vote`, `check_votes`, `min_votes` | 0x7a1c8, 0x7a5c4, 0x7a1b0 | 0x10045df0, 0x10046150, 0x10045dd0 | match (same strings, trimming, thresholds, "nextmap" after a teamplay toggle) |
| state `ts_init`, `init_teams` (3.0 part both run) | CTSGameRules::Init 0x77d6c | FUN_10044810 | match (round restart +5, entity respawn +60 (2.1), vote/The One timers +10) |
| cvars (core) | GameDLLInit cvar_t | 0x10093a0c.. table | match (round 1; table re-dumped) |
| voice `can_player_hear_player` (not listed before) | CMultiplayGameMgrHelper::CanPlayerHearPlayer 0xd0ed0 | helper vtable 0x10084b40 slot 1 = FUN_10023780 | 3.0 match (round 1); **V** 2.1 match: teamplay `PlayerRelationship` (+0xc8) == 1, then modes 2/3 or the live `lastmanstanding` (port's `g.lms` is `cvar && DM` in 2.1, but outside LMS DM and the round modes no player has a negative respawn time, so the same answers), dead talker with respawn < 0 (or NaN) heard only by such listeners |
| voice `client_command`, `update_masks`, `rules_mask` (not listed before) | CVoiceGameMgr::ClientCommand 0xf7438.., UpdateMasks 0xf79bc | 0x10069d70, FUN_10069f90 | 3.0 match (round 1); **V** 2.1 match (vban first mask only, VModEnable sets/clears and drops the request bit, then UpdateMasks) |
| mapcycle `change_level`, `pick`, `parse_cycle`, `extract_commands` (not listed before) | ChangeLevel 0xd0784, CountPlayers 0xd0640 | ChangeLevel 0x10024c60, FUN_10024b20, FUN_10024b60 | 3.0 match (round 1); **V** 2.1 match (same cycle walk that never tests the last item, `nextmap` vote override, Init, IsMapValid fallback, RULES line, commands after the change); unverified detail: 2.1's `PLAYER COUNT` console line may print the min where 3.0 prints the max (console only) |
| lifecycle `give_cash`, `on_spawn` put-in-server cash (not listed before) | GiveCash 0x9310c (clamp 0..30000), ClientPutInServer 0xacf44 (`sscanf` of the unregistered cvar's "1200") | FUN_10042c50 (same clamp), pfnClientPutInServer 0x1000cec0 (no cash) | **V** match both (ts21 `put_in_server_cash` 0) |
| lifecycle `killed` parts `killer_blur`, `death_push`, `damage_force`, `send_award`, `headshot_death_slows`; `post_think`; commands `fullupdate_spectator`, `set_view_angles`, `globalslow_*`, `weaponrestriction_text`; mod `world_key_value`, `add_frags` | parts of the rows above | — | covered by the `killed` / `client_command` / server-command rows (round 1); not re-checked separately |

## Coverage (TS 3.0 plugin) — `crates/ts30/server/src`
The #200 moves (fb65775, b60f5bc) were checked by diffing the removed core lines against the
added ts30 lines: renames only (`self.` → `g.`, `*_core` → plugin names). Bodies as round 1.
| Port function | Original | Status |
|---|---|---|
| rules `decide_gameplay`, `install` | DecideGamePlay 0x767ac, InstallGameRules | match |
| rules `think` | CTSGameRules::Think 0x77204 | match (LMS `else if` equivalent: RestartRound moves the round end ahead) |
| rules `srvsett_think`, `auto_powerups`, `check_deads`, `restart_round`, `round_refund`, `the_one_check`, `select_random_the_one` | 0x775b3.., 0x79c00, 0x77920, 0x7af28 | match (round 1) |
| players `set_player_the_one`, `default_team`, `change_team_tail`, `set_respawn_time`, `health_regen`, `buy_armament` | 0x7b078, 0xe44ac, 0xe488c tail, 0x75ff4, PreThink 0x908cc | match (round 1; tail re-read from 0xe488c) |
| observer `observer_think`, `set_mode` | PreThink observer block, Observer_SetMode 0x99e44 | match (round 1) |
| scoring `player_killed`, `points_for_frag`, kill tags | 0x895e4, 0x89140 | match (round 1 + #118-#121, #148); 4d498e7 (#198) moved the plugin kill report into core helpers (`kill_report`, `report_kill`): reporting only, no game change |
| scoring kill cash (not listed before) | 0x895e4: `GiveCash(killer, pts == -1 ? -1000 : pts*500+400)`, `GiveCash(victim, 900)` in DM/TDM, unconditional; mode > 1 branch (±1000) unreachable in 3.0 | **V** match |

## Coverage (TS 2.1 plugin) — `crates/ts21/server/src`
| Port function | 2.1 original | Status |
|---|---|---|
| rules `decide_gameplay`, `install`, `setup_teams`, `set_spawn_classes` | DecideGamePlay 0x100433b0, Init 0x10044810, FUN_10042f50 | match |
| rules `init_objectives`, `disable_objectives` | InitObjectives 0x100449b0 | match |
| rules `think` (mapglobals, globalslow, check deads, power-ups, The One, RespawnEntities 60 s, LMS, round end, SrvSett, weapon restriction, restartround, 10 s The One check) | Think 0x10043cc0 | match (TeamWin(2, cashforlose, cashforlose) on round TDM expiry confirmed in the disassembly) |
| rules `globalslow_value`, `globalslow_step`, `global_slow` | Think 0x10043cc0 head | match |
| rules `srvsett_think`, `refresh`, `restriction_mask`, `weapon_restriction_think` | Think tail, FUN_10043630, FUN_10043620 (0x1f) | match |
| rules `select_random_the_one`, `the_one_check_slots` | 0x100466e0, 0x1004439c | match |
| rules `team_win`, `round_cash`, `send_round_scores` | TeamWin 0x10044d00 | mismatch #236 (texts); cash, scores, TSHVoice, under-two-players reset match **V** (disasm 0x10044dd0..: `fild(c + d)`, `<=` initial → initial, winner `> 30000` → 30000; +0x84/+0x88 also zeroed with fewer than two players: written nowhere else, unobservable) |
| rules `round_time_left`, `giver_team_win`, `giver_cash` | RoundTimeLeft 0x100450f0, CTSWinGiver::Use 0x10050d10 | match |
| rules `reset_restart_round` | 0x10044ba0 | match |
| rules `restart_round` | 0x100445d0 | mismatch #236 (text); roundtime clamp (NaN → 30, `and 0x4100`), side swap, deadlines, resets match **V** (per-player fields +0x6c4/+0x68c/+0x732/+0x69c..+0x6a8/+0x6e8, StartObserver if not observing, Recount, RespawnEntities(1)) |
| rules `check_deads` | CheckDeads 0x10045940 | mismatch #236 (LMS text); LMS/mode 2/mode 3 logic matches **V** (LMS +10 frags and ScoreInfo, `%i` = 10; mode 3 manager +0x88/+0x8c targets) |
| rules `team_cash`, `round_team` | FUN_100458b0 | match **V** (`stricmp` FUN_1007e860, modes 2/3 only) |
| players `on_spawn`, `on_init_hud` | TSInit 0x100552f0, InitHUD 0x100451c0 tail | match as hooks; order #231 |
| players `set_respawn_time(_with)`, `respawn_time`, `force_waits` | FUN_10042d50 | match (incl. award 1/11 set inside) **V** |
| players `change_team_tail` | 0x10038e99..0x10038ef3 | match **V** (no The One test in 2.1's tail; its caller from TheOneIsDead is #250) |
| players `health_regen`, `set_player_the_one`, `default_team` | PreThink overheal, 0x100467c0, FUN_10038920 | match (default_team 15 vs 16 bytes, #238) |
| cash `cash_floor`, `floor`, `cash_based`, `give_kill_cash` | FUN_10042c90, 0x100435f0, 0x10042d00 | match **V** (GiveKillCash's `mode != 0 && mode != 1` is CashBased's test: 2.1 has modes 0..3 only) |
| cvars `initial_cash`, `cash_for_win`, `cash_for_lose`, `CVARS` defaults; lib `put_in_server_cash`, `on_start_observer`, `on_init_hud` cash (not listed before) | cvar_t 0x10093b74/0x10093b88/0x10093b9c ("3000", "1500", "-1000"), StartObserver → FUN_10042c90, InitHUD tail `cash = (int)initial_cash` | **V** match |
| commands `client_command` (give, powerup 4, cashala, refusals) | pfnClientCommand 0x1000d4b0 | match (lasclr_ is #215, other area) |
| cvars `respawn_number`, `trunc`, polled cvars | SetPlayerRespawnTime, Think | match |
| observer `observer_think`, `fire`, `countdown`, `deadline_*` | PreThink 0x10053b25..0x10053d63 | match (round 1) |
| scoring `player_killed`, `the_one_kill_bonus`, `spree`, `team_kill`, `points_for_frag` | 0x1004f7e0, FUN_100392e0, 0x1004f3c0 | match (round 1; #198 fixed in 4d498e7) **V** (decompile + disasm: The One gate before the FL_CLIENT test, labels and 2 points, stunt +2 / slide +1 as a byte, double frag `< 1.0`, fury > 2 / Specialist > 8 (+5, hold 40, slow pause), spree byte saturating at 255, 3/6/9/≥10 awards and health (+30/+50 capped 150, 150 set), kill cash `pts*900` / -1000, frags `fild pts + frags`, slowmatch == 1.0, bot OnKill, team-kill window, The One bonus `+10·players` over slots 0..max-1 capped 200; FUN_100445c0 called after the frags is an empty `ret 8`). Edge not filed: 2.1's team-kill window does nothing when `time == until` exactly (port counts it). The `the_one_is_dead` call is #250 |
| lib `on_killed` (cash floor, armour power-up) | Killed 0x10052390 | match |

## Gameplay switch and SrvSett (port additions, no original; validated against the docs)
| Port function | Checked against | Status |
|---|---|---|
| `rules::install` → `gameplay::map_select` / `bot_backend::map_start` / `start_rules` / `exec_server_config` / `gameplay::map_start` / `reload::map_start` | gameplay-plugins.md §4 (selection after `exec game.cfg` + `server_execute`, before the rules are built; `on_activate` after; the server config queued as TS's constructor does) | match the docs **V** |
| `reload::frame`, `Watcher::{frame, map_start}` | §4 (two-frame debounce on resolved values, one reload per map, no loop through a map start, `pfnChangeLevel` to the same map), its unit tests | match the docs **V** (the debounce needs *a* difference on two frames running and reloads for the second frame's values, as documented) |
| `gameplay::refuse_client`, `ConnectingClient::classify`, ts21 `refuse_client`, server-dll `client_connect` reject buffer | §4.2 (bot by FL_FAKECLIENT or `*bot`, HLTV by `*hltv`/`hltv`, port by `_tsrs`; 2.1 refuses `Other` with the documented text; checked before TS's own ClientConnect and the plugins; ≤127 bytes + NUL) | match the docs **V**; the engines run ClientConnect again at a map change, so TS 3.0 clients are dropped when a reload switches to 2.1 (tested by `cargo xtask ts21`, not re-run) |
| `ts_init_hud` `SrvSett` 4..7 | §2, §4.3: key 4 = numeric id (`srvsett_value`), keys 5..7 = `version_numbers()`, sent after 0..3 to the joiner; TS 3.0's client.dll handler 0x1004b840 switches on 0..3 only (re-read) | match **V** |
| client `Hud::apply` `SrvSett` 4..7, `InitHUD` clearing `gameplay`/`server_version`, the weapon-table switch on key 4 | §4 (cleared on InitHUD, not VidInit; key 5 starts the version, 6/7 fill it) | match the docs **V** |
| `ts_gameplay` cvar default, unknown/empty id fallback with one warning | §2 (`default_id()` = first registered; FCVAR_SERVER) | match the docs **V** |

## Not verified / notes
- ulp-level x87 vs f32 differences in 2.1's spawn-spot distance (2.1 sums x, y, z; the port's MSVC
  path sums z, y, x as 3.0 mp.dll), `TeamWin`'s RoundTime byte (`round_end - time` rounded to f32
  before the truncation), and the death push normalisation: not filed (same class as #213/#220).
- 2.1's `PlayerIsCheating` reads a bot flag (+0x14c); bots set `cl_lw` themselves, so no effect.
- 2.1's PreThink steps outside the rules (kung-fu power, stunt timers, `FAllowFlashlight` hide bit)
  belong to other areas (#201 known).
- `motdfile ""`: assumed `LOAD_FILE_FOR_ME("")` loads nothing (engine behaviour), #235.

## Validation (independent pass, after #198 merged; main 5032209)
- Completeness: listed the non-test `fn`s of `crates/server/src/rules/*`, `crates/ts21/server/src/{rules,cash,players,scoring,cvars,lib}.rs`
  and the ts30 rules/scoring hooks. Missing before and now added: the core `the_one_is_dead` /
  `the_one_switch` path (→ #250), `team_kill_punish`, voice (`can_player_hear_player`,
  `client_command`, `update_masks`), mapcycle (`change_level`), `give_cash` / put-in-server cash,
  3.0's kill cash, 2.1's cash cvars and cash hooks, and the gameplay-switch functions (now a table).
  The `killed`/`client_command` helper functions are folded into their parent rows (not re-checked).
- Re-checked "match" rows (22, decompile and, for compares/rounding, disassembly): ts21 scoring
  (points_for_frag, player_killed, spree, team_kill, the_one_kill_bonus), cash (floor, cash_based,
  give_kill_cash, team_cash/round_team), team_win cash, restart_round, check_deads,
  set_respawn_time, change_team_tail, select_random_the_one, on_killed + 2.1 respawntime clamp,
  2.1 TeamWin TSHVoice, 2.1 DeathNotice (teamplay), RecountTeams/fewest (2.1), ts_init_hud (2.1),
  3.0 GiveCash/ClientPutInServer, 3.0 kill cash, 3.0 ResetRestartRound, 2.1 cvar defaults,
  team_kill_punish strings. Wrong: 2 — the `ts_init_hud` row missed `SrvSett` 0 (#231 comment),
  and the ts21 scoring/players "match" rows hid the core `TheOneIsDead` team switch (#250).
- Mismatch rows: #231, #233, #234, #235, #236, #237, #238, #239 exist, are open and describe the
  rows; cited port lines (spawnpoint.rs:63-68, commands.rs:142/:205, lifecycle.rs:124/:263/:459,
  teams.rs:175/:194, players.rs:29) are right at 5032209.
- Addresses spot-checked: 0x10044d00, 0x10045940, 0x100445d0, 0x10042d50, 0x10042c90,
  0x100435f0, 0x10042d00, 0x1004f3c0, 0x1004f7e0, 0x100392e0, 0x10039200, 0x10038cb0,
  0x10038920, 0x10038f50, 0x10046930, 0x100466e0, 0x100451c0, 0x10039730, 0x10039620,
  0x10024c60, 0x10023780, 0x10069d70, 0x9310c, 0xacf44, 0x78750, 0x7b258, client 0x1004b840: all
  exist and are the functions named.
- "Not verified" items: the reasons stand (ulp-level x87 orderings, bot-flag read, other areas,
  engine behaviour for an empty `motdfile`); not re-verified.
- Remaining gaps: the objective/round entities themselves (`entities::ts::objective`, ctb, ground
  weapons' round-mode branches) belong to the entities audit; 2.1's `teamplay_think`, votes and
  `death_think` were not re-checked in this pass; 2.1's `PLAYER COUNT` console line.
