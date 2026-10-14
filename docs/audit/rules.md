# Audit: rules (3.0 + 2.1) — port c49ee1a

Method: decomp + objdump (ts_i386.so: Ghidra addr = file vaddr + 0x10000; 2.1 mp.dll ~/dev/hl/ts21/dlls/mp.dll).
Binaries used: ~/scratch/agents/servers/orig/ts/dlls/ts_i386.so, ~/dev/hl/ts21/dlls/mp.dll.

## Issues filed
- #118 Audit 3.0: PlayerKilled non-player killer: TS `pKiller->frags -= 1` (0x8aedc, 0xf8(%esp)=pKiller); port subtracts the victim's frag. (2.1 port correct.)
- #119 Audit 3.0 (+2.1): TK kick/ban ClientPrint goes to pKiller (0x8ae58 push 0x114(%esp)=0xf8; 2.1 FUN_100675b0(param_2)); port `team_kill_punish` sends to the victim.
- #120 Audit 3.0 (+2.1): killer ScoreInfo block: Classify()==CLASS_PLAYER on pKiller (suicide too -> second ScoreInfo) and `m_flNextDecalTime = time` (0xb44; 2.1 +0xa9c). Port: no suicide case, no decal reset.
- #121 Audit 3.0: missing "%s #TS_Killed %s #TS_OwnWeapon" HUD message (victims_weapon).
- #122 Audit 3.0: `restartround` (ResetRestartRound 0x78750) port only zeroes scores; missing round_end=0, TSRoundTime, team cash reset, 4x TeamScore, "Restarting Round". (2.1 port correct.)
- #123 Audit 3.0 (both): gameplay switch reset doesn't restart the mp_timelimit clock (frame.rs uses absolute time; #100 §1.3 requires the reset).
- #127 Audit 2.1: end-of-Think One check loops slots 1..maxClients-1 in TS (last slot skipped -> extra One every 10 s); port checks all slots.
- #128 Audit 3.0: spawn telefrag passes 0x1000 (DMG_NEVERGIB) at 0x6675a; port uses ALWAYSGIB (decomp doc also wrong).

## Coverage (3.0)
| Port function | Original | Status |
|---|---|---|
| rules::game_init, cvars::CVARS/register | GameDLLInit 0xc5aa0 + cvar_t data | match |
| rules::install / exec_server_config | InstallGameRules 0xc6050, CHalfLifeMultiplay ctor | match |
| state::decide_gameplay | DecideGamePlay 0x767ac | match |
| rules::game_description | GetGameDescription 0xd0c68 / 0xe5a04 | match |
| GameRules::ts_init | CTSGameRules::Init 0x77d6c | match (IsCTB cache +0x5890/91 not modelled; both callers run after map load -> harmless) |
| GameRules::init_teams | InitTeams 0x77f60 | match |
| GameRules::is_ctb | IsCTB 0x77f08 | match (uncached, see above) |
| frame::multiplay_think, update_limit_cvars | CHalfLifeMultiplay::Think 0xcf038 | match (x87 product kept in register, file 0xbf162) |
| frame::teamplay_think | CHalfLifeTeamplay::Think 0xe412c | match |
| frame::intermission_think | Think game-over branch | match |
| frame::go_to_intermission | GoToIntermission 0xd0050 | match (chattime clamp one frame later, unobservable) |
| frame::ts_think | CTSGameRules::Think 0x77204 | match |
| frame::srvsett_think / refresh_copy | Think tail | match |
| frame::auto_powerups | Think power-up block | match |
| frame::put_powerup | PutPowerUp 0x9797c | match |
| state::random_powerup | GetRandomPowerup 0x97894 | match |
| frame::check_deads | CheckDeads 0x79c00 (LMS part) | match |
| frame::restart_round, round_refund | RestartRound 0x77920 | match (TS refunds only with the gun as active item; observers have none via RemoveAllItems -> equivalent in practice) |
| frame::reset_restart_round | ResetRestartRound 0x78750 | mismatch #122 |
| entities::ts::respawn_entities | RespawnEntities 0x77764 | match |
| frame::the_one_check | Think The One block | match |
| frame::select_random_the_one | SelectRandomTheOne 0x7af28 | match |
| scoring::set_player_the_one_core | SetPlayerTheOne 0x7b078 | match |
| scoring::the_one_is_dead | TheOneIsDead 0x7b258 | match |
| scoring::player_killed_core | CHalfLifeMultiplay::PlayerKilled 0x895e4 | mismatch #118 #119 #120 #121; spree/award/double kill/cash/slowmatch/TK window/log tags otherwise match |
| scoring::weapon_name, rename_weapon, log_kill, log_weapon | PlayerKilled weapon-name + log part | match |
| scoring::team_kill_punish | PlayerKilled TK block | mismatch #119 |
| scoring::points_for_frag | TSGetPointsForFrag 0x89140 | match (edge: active non-gun item under closecombat counts as kung fu in the port; TS needs the gun or no item) |
| scoring::death_notice, teams::same_team | DeathNotice 0x88f7c / 0xe509c, PlayerRelationship 0xe5278 | match |
| votes::parse/player_vote/min_votes | PlayerVote 0x7a1c8, CalcMiniminVotants 0x7a1b0 | match (token >63 chars truncated before IsMapValid: edge) |
| votes::check_votes/tally | CheckVotes 0x7a5c4 | match |
| lifecycle::health_regen_core | CBasePlayer::PreThink 0x908cc (+0x73c) | match (`(short)` read of +0x5876; +0x5877 only ever 0; ulp-level x87 rounding only) |
| lifecycle::set_respawn_time_core | SetPlayerRespawnTime 0x75ff4 | match |
| lifecycle::ts_init_hud | CTSGameRules::InitHUD 0x79134 | match (its +0x714/+0x538/+0x53c/+0x7cc/+0x49a/ResetArmament resets are redundant with the port's fresh Player per connection) |
| spawnpoint::select, spot_score | EntSelectSpawnPoint 0x76128 / TSSelectSpawnSpot 0x76244 | match |
| spawnpoint::telefrag | TSSelectSpawnSpot tail | mismatch #128 |
| teams::change_team | CHalfLifeTeamplay::ChangePlayerTeam 0xe488c | match |
| gameplay::reset::begin/finish | (real map start) | mismatch #123; rest matches gameplay-plugins.md §4 |

## Coverage (2.1)
| Port function | Original (2.1 mp.dll) | Status |
|---|---|---|
| ts21 scoring::player_killed, spree, team_kill, the_one_kill_bonus | PlayerKilled 0x1004f7e0 | match except shared #119/#120 (frag var 0x20(%esp): all writes checked) |
| ts21 rules::think | CTSGameRules::Think 0x10043cc0 | mismatch #127; globalslow/powerup/LMS/SrvSett/weaponrestriction/restartround order match |
| ts21 rules::check_deads | CheckDeads 0x10045940 | match |
| ts21 rules::restart_round | RestartRound 0x100445d0 | match |
| ts21 rules::reset_restart_round | ResetRestartRound 0x10044ba0 | match |
| ts21 rules::select_random_the_one | SelectRandomTheOne 0x100466e0 | match |
| ts21 rules::decide_gameplay/install/setup_teams | DecideGamePlay 0x100433b0 (prefixes de_/cs_/tr_/tm_/ts_ dumped), Init 0x10044810 | match (Init's +0x54c8 = -1 vs port mask_sent 0: unobservable, InitHUD sends the mask) |
| ts21 scoring::points_for_frag/kill_kind/points | TSGetPointsForFrag 0x1004f3c0 | match (same non-gun-item edge as 3.0) |
| ts21 rules::team_win | TeamWin 0x10044d00, player count 0x10045d80 | match |
| ts21 kill weapon name (core::weapon_name) | DeathNotice FUN_1004ede0 | mismatch #131 |
| ts21 cvars::CVARS, Polled | GameDLLInit cvar_t table 0x10093a0c.. (dumped) | match (respawntime 5 vs 3: both clamped to 5..30, verified 0x1004543c) |
| ts21 commands::client_command | ClientCommand 0x1000d4b0 (give, powerup, cashala) | match |
| ts21 players::set_respawn_time/respawn_time | SetPlayerRespawnTime 0x10042d50 | match without force; forced call missing -> #142 |
| ts21 team change (core teams::change_team) | FUN_10038cb0 | mismatch #142 |
| ts21 players::on_spawn/on_init_hud/health_regen/set_player_the_one | TSInit / InitHUD / PreThink / 0x100467c0 | match (SetPlayerTheOne read) |
| ts21 observer::observer_think | PreThink 0x100537e0 observer block | match (camera = documented deviation) |
| ts21 cash::cash_floor, give_kill_cash, buy_weapon, add_item, cash_based | 0x10042c90, 0x10042d00/0x10042c50, 0x10055e00, 0x10055c30, 0x100435f0 | match |
| ts21 spawn telefrag (core) | FUN_10043010 (0x1004335e push 0x1000) | mismatch, covered by #128 (commented) |



## Coverage (3.0, round 2)
| Port function | Original | Status |
|---|---|---|
| commands::client_command (fov, use, changeteam, jointeam, motd, fullupdate, powerup/cashala cheats) | ClientCommand 0xad838 | match |
| commands::host_say + say helpers | Host_Say 0xad02c | match (LMS dead chat, timeleft, vote strlwr, 0.5 s flood) |
| commands::cmd_globalslow / globalslow_amount | cmd_globalslow 0xc5580 | mismatch #140 |
| commands::cmd_weaponrestriction / restriction_by_id | cmd_weaponrestriction 0xc58a0, display 0xc5770, WeaponRestrictionByID 0x76ad8 | mismatch #140 (ids match) |
| commands::cmd_restartround | cmd_restartround 0xc5748 | -> #122 |
| lifecycle::respawn_command | ClientCommand "respawn" | match |
| lifecycle::client_connect / client_disconnect / client_disconnected | ClientDisconnect 0xacd4c, CHalfLifeMultiplay::ClientDisconnected 0xcf960, TSPlayerDisconnect 0x9f94c | match |
| lifecycle::client_kill | ClientKill 0xacec8 (float store verified) | match |
| lifecycle::userinfo_changed / multiplay_userinfo_changed / teams::userinfo_changed | 0xae638, 0x8b244, 0xe4d18 | match |
| lifecycle::start_observer / stop_observer | 0x99634 / 0x99a14 | match |
| lifecycle::observer_set_mode / find_next_player / handle_buttons / idle | 0x99e44 / 0x99b0c / 0x99d34 / 0x99ca4 | match |
| lifecycle::observer_think_core | PreThink 0x908cc observer block | match |
| lifecycle::death_think | PlayerDeathThink 0x8eaec | match (ulp-level velocity normalisation) |
| lifecycle::killed | CBasePlayer::Killed 0x8ed5c | mismatch #139 (headshot slow gate); rest match (gib test confirms #128's NEVERGIB effect) |
| msg::ts_message_one/all, text, say | TSHudMessageOne/All 0x768d4/0x769d4 | match (127-byte cut) |
| voice::* | CVoiceGameMgr 0xf7438.., CanPlayerHearPlayer 0xd0ed0 | match |
| mapcycle::change_level / pick / parse | ChangeLevel 0xd0784, CountPlayers 0xd0640 | match |
| powerup::use_powerup / add_powerup / update_state (rules side) | UsePowerUp 0x95980, PlayerAddPowerUp 0x97bc8, UpdatePowerUpState 0x95748 | match |

## Remaining unverifiable
- ulp-level x87 vs f32 rounding (health regen, death-think velocity, countdown seconds): not filed.
- `IsCTB` cache not modelled (harmless: callers run after map load).
- Port's FL_CLIENT-free points_for_frag path (TS checks pKiller FL_CLIENT): unreachable with a player killer.

## Round 3
- #147 Audit 3.0: float arithmetic rounds to f32 per step where TS keeps x87 extended (The One regen 0x80c0e.., death-think 0x7eb16.., countdown/deadline 0x80ed4..). Measured: 30/3769 health steps, 368/620k timers, ~24% death-think velocity components differ.
- #148 Audit 3.0 (+2.1): close-combat kill holding a non-gun active item (cycler_weapon) scores kung fu in the port; TS gives base points (stunt CheckForBreakables 0x9bd9c sets glast_closecombat).
- Not filed: 2.1 Init's last-sent weapon mask (+0x54c8) -1 vs port 0: no observable effect (InitHUD sends the mask to every client before any Think could; ChangeLevel's Init is followed only by intermission thinks).
- Not filed: IsCTB cache (+0x5890/91) not modelled: harmless, both callers run after map load, ts_briefcase never spawns later.
