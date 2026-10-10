# Rules package: game rules, player lifecycle, TS entities, HUD sync, bots

Owner files: `crates/server/src/rules/**`, `entities/ts/**`, `player/damage.rs`, `player/hud.rs`,
`bots/**`, `net.rs`, `crates/goldsrc/src/ext/rules.rs`, `crates/shared/src/protocol/{rules,hud}.rs`.
Source of truth: the decomp (`docs/server/gamerules-entities.md`, `architecture.md` §2–§5, §7,
`player-movement.md` §5–§7, `weapons.md` §5.9, `client/architecture-hud.md` §3). Addresses below
are Ghidra addresses.

## 1. Layout

| Rust | Contents |
|---|---|
| `rules/state.rs` | `GameRules` singleton: `g_pGameRules` (DM/Teamplay) and `CTSGameRules TSGameRules` folded into one struct (`Mode`, `lms`, `the_one`, teams, spawner, round, global slow, map flags). `decide_gameplay`, team lists, `random_powerup`. Accessed with `state::with(|g| …)` (short borrows, no re-entrant calls inside). |
| `rules/cvars.rs` | TS cvar table and registration, clamped helpers (`respawn_time`, `round_time`). |
| `rules/lifecycle.rs` | connect / disconnect / kill / userinfo, `TSInit` + spawn, InitHUD (both rules objects), observer, `respawn`, `Killed`, `PlayerDeathThink`, fall damage, `+use`. |
| `rules/scoring.rs` | `PlayerKilled`, `TSGetPointsForFrag`, `DeathNotice`, The One handover. |
| `rules/spawnpoint.rs` | `EntSelectSpawnPoint` + `TSSelectSpawnSpot` + `GetPlayerSpawnSpot`. |
| `rules/powerup.rs` | held/active power-ups, `SlowMotionFlow`, `GoSlow`, `InterpolateSlowMotion`, `TakeArmor`. |
| `rules/frame.rs` | `StartFrame`: Multiplay `Think` (limits, intermission, mapcycle) and `CTSGameRules::Think` (global slow, auto power-ups, LMS rounds, The One, SrvSett). |
| `rules/teams.rs` | teamplay assignment, `ChangePlayerTeam`, TeamInfo/TeamNames/TeamScore. |
| `rules/commands.rs` | client commands owned by the rules, `Host_Say`, server commands. |
| `rules/msg.rs` | `Wire` adapter over `MsgWriter`; `TSHudMessageOne/All`, `ClientPrint`, `SayText`, log. |
| `rules/voice.rs` | `CVoiceGameMgr` + `CMultiplayGameMgrHelper`: who hears whom, `vban`/`VModEnable`, `VoiceMask`/`ReqState`. |
| `player/damage.rs` | `TraceAttack` + `TakeDamage`. |
| `player/hud.rs` | `UpdateClientData` HUD shadow copies. |
| `entities/ts/*` | TS map entities (§4). |
| `bots/mod.rs` | fake clients. |
| `net.rs` | `AddToFullPack` TS changes (CreateBaseline/UpdateClientData were already stock-correct). |
| `shared/protocol/{hud,rules}.rs` | payload structs (`write`/`read` over `Wire`/`WireRead`, `Bytes` for tests). |

## 2. Features: TS function → Rust

| TS (address) | Rust | Notes |
|---|---|---|
| `DecideGamePlay` 0x767ac | `state::decide_gameplay` | worldspawn `gamemode` 1–4 wins over cvars; The One > LMS > `mp_teamplay`. Unit tested. |
| `InstallGameRules` 0xc6050, `CTSGameRules::Init` 0x77d6c, Teamplay ctor 0xe…, `InitTeams`/`SetTeamList`/`SetTeamModelList` | `rules::precache` → `GameRules::install` | `exec game.cfg` + `ServerExecute` first, like TS. The One renames team 0 "The ONE" and skips the first listed name (TS quirk kept). |
| `GameDLLInit` 0xc5aa0 | `rules::game_init` | cvars + 7 TS server commands (+3 test commands). |
| `ClientConnect` 0xacce8 | `lifecycle::client_connect` | `ts_playeramount` connect hook, accepts. |
| `ClientDisconnect` 0xacd4c, `ClientDisconnected` 0xcf960, `TSPlayerDisconnect` | `lifecycle::client_disconnect` | "- %s has left the game", `game_playerleave`, log, strip weapons, drop flag, end slow flow. |
| `ClientKill` 0xacec8 | `lifecycle::client_kill` + `pre_think` | suicide 1 s later, not while observing. |
| `ClientPutInServer` 0xacf44 | first `on_spawn` | cash = `initial_cash` "1200", `EF_NOINTERP` (`server-dll` creates the Player and spawns it). |
| `ClientUserInfoChanged` 0xae638 + 0x8b244 | `lifecycle::userinfo_changed` | name change (% → space) broadcast/log; HLDM models refused. |
| `CBasePlayer::Spawn` / `TSInit` 0x92a4c / `PlayerSpawn` 0x890ac | `lifecycle::on_spawn` | fuser1/slow reset, speed 330, `slj`/`hl`/`mj` physinfo, spawn protection 1 s (transparent, `TSFade` 200,200,250), power-ups reset, `weapons |= 0x80000000`, default team, spawn spot. A map's `game_player_equip`s replace the loadout (`stock::maprules::equip_on_spawn` from `weapons::on_spawn`). |
| `UpdateClientData` init block 0x91d00 | `lifecycle::init_hud` | `ResetHUD` every spawn; first time `InitHUD`, rules InitHUD, TS InitHUD, `game_playerjoin`; `game_playerspawn`; RoundTime; super jump PwUp. |
| `CHalfLifeMultiplay::InitHUD` 0xcf618 / Teamplay 0xe45d8 | `lifecycle::rules_init_hud`, `teams::init_hud` | join TextMsg, log, `GameMode`, own ScoreInfo, MOTD, everyone's ScoreInfo; teamplay: TeamNames, assignment SayText, TeamInfo. |
| `SendMOTDToClient` 0xd0ae4 | `lifecycle::send_motd` | `ServerName` = hostname, `motdfile` (default `motd.txt`) in ≤60-char chunks, ≤0x600 bytes. |
| `CTSGameRules::InitHUD` 0x79134 | `lifecycle::ts_init_hud` | cheat notices, `SrvSett` 0–3, RoundTime, `Spectator` of everyone, **joiner starts as observer** with `respawntime` (5..30 s). |
| `SetPlayerRespawnTime` 0x75ff4 | `lifecycle::set_respawn_time` | LMS respawn budget (`respawnnumber` → 1), join deadline; award 1 / 11. |
| `StartObserver` 0x99634 / `StopObserver` 0x99a14 | `lifecycle::{start_observer, stop_observer}` | TSCash resend, TE_KILLPLAYERATTACHMENTS, ActItems 0, SetFOV 0, `EF_NODRAW`, deadflag 3, TeamInfo, `Spectator 1`, starts in the saved mode (chase free the first time); stop saves the mode. |
| `Observer_SetMode` 0x99e44 / `Observer_FindNextPlayer` 0x99b0c / `Observer_HandleButtons` 0x99d34 / `Observer_Idle` 0x99ca4 | `lifecycle::{observer_set_mode, observer_find_next_player, observer_handle_buttons, observer_idle}` | TS observer modes (`shared::ts::obs`): 1 chase locked, 2 chase free, 3 roaming (maxspeed 420), **5 in-eye**; there are no map modes and 4 is HLTV-only. Jump cycles 3 → 2 → 1 → 5 → 3, forward/back pick the next/previous target (not while roaming), 0.2 s apart. The target (`m_hObserverTarget`, `PlayerRules::obs_target`) is the killer (last inflictor if a player), kept across lives; the next one is a living, visible, non-observing player, only teammates in LMS while a respawn is pending. Twice a second a dead or observing target is replaced. No target → roaming. The camera position is `PM_SpectatorMove`'s. |
| `respawn` client command (ClientCommand) | `lifecycle::respawn_command` | only while observing and after `respawn_at`; "You have to wait until the next round." |
| `CBasePlayer::PreThink` 0x908cc (rules parts) | `lifecycle::pre_think` | button edges, InitHUD, fuser1 easing + `framerate`, power-up state, slow flow, health decay / The One regen, observer countdown text, death think, +use, spawn protection end, delayed suicide. |
| `CBasePlayer::Killed` 0x8ed5c | `lifecycle::killed` | TSMBlur (random), respawn time, headshot victim slow 0.33, `iuser4 = DEAD_LOCK`, `weapons::on_killed`, drop flag, PlayerKilled, knockback (min(dmg×5, 500)), hop when fast/hard hit, TSHealth 0, ActItems 0, SetFOV 0, gib. |
| `PlayerDeathThink` | `lifecycle::death_think` | settle; while dying, advance the death sequence (`studio_frame_advance`, at the slow-motion `framerate`) until it finishes; then corpse (`deadcorpse`, 10 s) + observer after 3 s or any button. |
| `CHalfLifeMultiplay::PlayerKilled` 0x895e4 | `scoring::player_killed` | points (stock/kung fu/knife 2, stunt/slide +1), sprees 3/6/9/10+ (hp, kung fu, super jump, stored slow pause, awards 2/3/4/10), double kill ×2, killed-specialist +5, suicide −1 (−5 more at spree > 8), cash `points×500+400` / victim +900, spectacular slow-motion point (`spectacularness`%), `slowmatch` stored slow-mo (max 3 s), TK kick/ban (`banfortk`), all `#TS_*` TSMessage lines, log lines with TS suffixes, `game_playerdie`/`game_playerkill`, ScoreInfo, CheckDeads in 1 s. |
| `TSGetPointsForFrag` 0x89140 | `scoring::points_for_frag` | `#TS_KungFu`, `Throwing`, `#TS_StStl`, `#TS_SlStl`, `#TS_Stl`. |
| `DeathNotice` 0x88f7c | `scoring::death_notice` | `DeathMsg`; weapon name rules (strip prefixes, Throwing Knife, Shrapnel, M61 Grenade, Kung Fu); HLTV director event (victim, inflictor or killer, 0x27). |
| `TheOneIsDead` 0x7b258, `SetPlayerTheOne` 0x7b078, `SelectRandomTheOne` 0x7af28 | `scoring::{the_one_is_dead, set_the_one_other, apply_the_one, announce_the_one}`, `frame::the_one_check` | hp 150, kung fu, super jump, speed 330, stored slow pause 3 s, award 10, "%s is The One!" (100,150,255), team split. |
| `EntSelectSpawnPoint` 0x76128 / `TSSelectSpawnSpot` 0x76244 | `spawnpoint::{select, spot_score, place}` | up to 32 spots scored by enemy/teammate visibility and nearest player; telefrag (300, ALWAYSGIB) within 128 when the best score < 128; fires the spot's target. Unit tested. |
| `CTSGameRules::Think` 0x77204 | `frame::ts_think` | global slow expiry/apply, CheckDeads, auto power-ups, The One every 10 s, LMS restart/checks, SrvSett 0–2 on change. |
| `CHalfLifeMultiplay::Think` 0xcf038 | `frame::multiplay_think` | `mp_timelimit`, `mp_fraglimit` (team frags; CTB team captures), `mp_fragsleft`/`mp_timeleft`, intermission (`SVC_INTERMISSION`), `mp_chattime` clamped 10..120, then `changelevel` to the next `mapcyclefile` map. |
| `PutPowerUp` 0x9797c, `GetRandomPosition` 0x974f4, `GetRandomPowerup` 0x97894 | `frame::{put_powerup, random_position}`, `GameRules::random_powerup` | 20 s after success, 1 s after failure; only on maps without their own `ts_powerup` (spawner byte). |
| `CheckDeads` 0x79c00, `RestartRound` 0x77920, `ResetRestartRound` | `frame::{check_deads, restart_round, reset_restart_round}` | LMS: last man +10 frags, "%d men standing!", "Round over", RoundTime; restart: round time, refunds (`frame::round_refund`: owned weapons' cost + attachments, and `price × rounds / per_buy` for ammo types above one purchase, into `GiveCash`; unit tested), forced observer, respawn in 3 s, map items back. |
| `SetGlobalSlowMotion` 0x7961c, `GlobalSlowMotion2` 0x79720 | `frame::{set_global_slow, global_slow2}` | every 0.5 s, players and grenade/knife/WorldGun. |
| `PlayerAddPowerUp` 0x97bc8 | `powerup::add_powerup` | armour, kung fu, super jump (`mj`), health 150 + decay, grenade (`weapons::give_weapon(24)`), infinite clip (`weapons::multiply_clip`), stored types. |
| `UsePowerUp` 0x95980, `TSUse` 0x956e0 | `powerup::use_powerup`, +use double tap in `pre_think`, `usepowerup` command | PwUpTime, TSMBlur 1/2; refused while slow-mo is blocked. |
| `UpdatePowerUpState` 0x95748 | `powerup::update_state` | self 0.6 / others 0.35 (slow-mo), 0.8 / 0.225 (slow pause, bullet time); expiry; dead players drop held/active power-ups (DM/TDM). |
| `SlowMotionFlow` 0x96bb0 / `DeactivateSlowMotionFlow` 0x96a9c | `powerup::{slow_flow, deactivate_flow}` | every 0.5 s eye→eye traces; the stronger slow wins; grenades/moving knives/WorldGuns in sight (max 32). |
| `GoSlow`/`InSlow` 0x929d0/0x9298c, `InterpolateSlowMotion` 0x9910c | `powerup::{go_slow_self, go_slow, in_slow, interpolate}` | fuser1 eases toward `Player::slow_target` at 7/s, snaps within 0.05 (unit tested). |
| `TakeArmor` 0x93da4 | `powerup::take_armor` | kevlar costs 10 slots once; cap 100; speed from free slots. |
| `CBasePlayer::TraceAttack` 0x901cc | `player::damage::take_damage` (traced part) | spawn protection, stunt ×0.7 (`iuser4 & 0x10c10`), dodge ×0.7 (`PlayerRules::dodge_until`), friendly fire, head ×2 / ×1.5 shotgun (`0x2000000`), legs ×0.5, PTakeDam (0.3 s throttle, ×8 shotgun, +128 kevlar), head/kevlar/body hit sounds, pain sounds, corpse push. |
| `CBasePlayer::TakeDamage` 0x8fa94 | `player::damage::take_damage` | kevlar `dmg -= dmg×min(armor_absorb,1)` on hitgroups 2–5 unless `ARMOR_EXEMPT`; armour never depleted; rounded; knockback ×5; TSFade (150,10,10); death → `rules::on_killed`. Unit tested. |
| `CBasePlayer::UpdateClientData` 0x91d00 | `player::hud::update` | every 0.1 s on change: TSHealth, TSArmor, TSState, TSCash, TSSpace, PwUp, TSSlowMo, TSBTime, TSPAward (ALL). |
| `Host_Say` 0xad02c | `commands::host_say` / `format_say` | `\x02name: `, `(TEAM)`, `%a`/`%h`, 0.5 s flood gate, LMS dead chat rule, `timeleft`, log. Unit tested. |
| `PlayerVote` 0x7a1c8, `CheckVotes` 0x7a5c4, `CalcMiniminVotants` 0x7a1b0 | `votes::{player_vote, check_votes, min_votes}`, `Votes` in `GameRules` | `vote map <name>` (current/invalid map refused, "%s vote for map %s" to all), `vote stats` (last tally, "(n/need) votes for map"), `vote reset`; handled votes aren't broadcast. Every 10 s every player is counted and votes are grouped by map; the leader passes at `max(1, players/2)`. A passed vote sends `multiplay_think` to intermission and `change_level` loads the voted map. A joiner's vote is cleared in TS InitHUD. Unit tested. |
| `cmd_globalslow` 0xc5580, `cmd_restartround` 0xc5748, `cmd_weaponrestriction` 0xc58a0 | `commands::cmd_*` | clamps unit tested. |
| `CVoiceGameMgr::Init` 0xf7438, `Update` 0xf750c, `ClientConnected` 0xf7548 (via `CHalfLifeMultiplay::ClientConnected` 0xcf59c), `ClientCommand` 0xf770c (via `CHalfLifeMultiplay::ClientCommand` 0xceee8), `UpdateMasks` 0xf79bc | `voice::{register_messages, precache, update, client_connected, client_command, update_masks}` | Every 0.3 s (from the start of the Multiplay `Think`, intermission included) and after each `vban`/`VModEnable`: `ReqState` to clients that haven't answered `VModEnable`; for clients with voice enabled, the audible mask = players the rules allow (`sv_alltalk` ignores the rules); `VoiceMask` (`L audible, L banned`) when either mask changed; then `Voice_SetClientListening(client, other, audible && !banned)` for every pair. Steam HLDS starts every stream closed, so without this nobody hears anybody there. `vban <hex>` takes one mask (32 players); `VModEnable <n>`. |
| `CMultiplayGameMgrHelper::CanPlayerHearPlayer` 0xd0ed0 | `voice::can_player_hear_player` | teamplay: teammates only; round modes 2/3 or LMS: a talker who is dead for the rest of the round (`respawn_at` −1) is heard only by listeners in the same state. |
| `AddToFullPack` 0xaf180 | `net::add_to_full_pack` | `iuser4` for all entities, `playerclass` for non-players, glow shell on the viewer's own `WorldGun`/`knife`, `FL_SPECTATOR` culling, EFLAG_SLERP for `FL_FLY`. |

## 3. Cvars

Registered in TS order with TS defaults and flags (0x4 = FCVAR_SERVER, 0x100 = UNLOGGED):
`displaysoundlist 0`, `mp_fragsleft 0 (0x104)`, `mp_timeleft 0 (0x104)`, `mp_teamplay 0`,
`mp_fraglimit 0`, `mp_timelimit 0`, `mp_friendlyfire 0`, `mp_falldamage 0`, `mp_weaponstay 0`,
`mp_forcerespawn 1`, `mp_autocrosshair 1`, `decalfrequency 30`, `mp_teamlist hitman;gordon`,
`mp_teammodels ""`, `mp_teamoverride 1 (0)`, `mp_defaultteam 0 (0)`, `mp_allowmonsters 0`,
`mp_chattime 10`, `roundtime 150`, `respawntime 3`, `damagemult 0.0`, `slowmatch 1.0`,
`nopowerup 0.0`, `botlearn 0.0`, `realbullet 0.0`, `usecash 0.0`, `ammocount 0`, `bottalk 1`,
`weaponstay 15`, `mp_theonemode 0`, `mp_theone gordon`, `mp_theothers seal`,
`lastmanstanding 0`, `spectacularness 1`, `customweapons ""`, `banfortk 0`, `botarmor 0`,
`voice_serverdebug 0`, `sv_alltalk 0` (FWGS already has it; the engine refuses the duplicate),
and the port's `ts_bot_quota 0`.

TS never registers `initial_cash`, `cashforwin`, `cashforlose`, `respawnnumber`, `dmgreport`;
they are constants (`cvars::INITIAL_CASH` 1200 etc., respawnnumber 1). The weapons package
registers `dmgreport` (default 0, port addition) so its cheat output can be switched on.

Server commands: `globalslow`, `restartround`, `weaponrestriction`, `addbot`/`addcustombot`
(TS: "Bots only in listen servers."; here they add a bot), `loadweapons`/`saveweapons` (print
"not supported yet"), and test helpers `ts_status`, `ts_hurt <#> <dmg> [hitgroup] [attacker#]`,
`ts_givepowerup <#> <bits>`.

Client commands handled here: `say`, `say_team`, `respawn`, `usepowerup`, `powerup <n>` (cheat),
`cashala` (cheat), `fov`, `changeteam` (VGUIMenu 2), `jointeam <n>`, `motd`, `fullupdate`,
`lasclr_<n>` (PlInfo), `menuselect`, `vban`, `VModEnable` (voice manager, §2), `cutwire_<n>` (no-op).

## 4. Map entities (`entities/ts`)

| Class | Status |
|---|---|
| `ts_powerup`, `item_longjump` | Full: keys `pwuptype`/`pwupduration`/`pwupmult`, random set from spawnflags, bodies/durations, fall → ground (bbox ±16×64), pickup rules (single slot, +use swap, instant set 0x1e6), message/target, respawn clamps (<1 → 60, ≤10 → 10, ≤180), hidden with `nopowerup 1`. Temporary ones (auto spawner, drops) live 60 s and fade over the last 30 s. A map power-up disables the automatic spawner; `item_longjump` re-enables it (TS). |
| `ts_groundweapon` + `weapon_crossbow/shotgun/gauss/egon/rpg/9MMAR/9mmAR/hornetgun/satchel/snark/handgrenade`, `ammo_ARgrenades` | Full: random ids per HL class, `w_` model, attachments from spawnflags, flag 16 = on wall (`PutOnWall`), pickup with **+use** (`weapons::give_weapon`), `wextraclip` (× clip size via `give_ammo`, only when the reserve for that ammo type is empty after the pickup, as in `GetWeapon` 0xa2328), respawn (<1 → 60, 1 → 1, ≤180), message/target. `buyable`/`attackerteam` (`GetWeapon` 0xa2328) are ported behind `state::cash_based()`, which is false as in TS: the team check (round modes, `attackerteam` ≠ -1), the price (`ground::buy_price`: cost + spawnflag attachments + ammo price × `wextraclip`; unit tested), "You don't have enough money…", and the charge after the pickup. Unknown/briefcase ids are removed. |
| `ts_groundammo` + `ammo_9mmAR/buckshot/crossbow/gaussclip/rpgclip` | Full: `magsN` = N × per-buy rounds, HL ones give half max carry of their class's ammo types; picked up on touch via `weapons::give_ammo`. |
| `ts_model` | Full. |
| `ts_mapglobals` | Full: weapon mask (SrvSett 3), round timer flag, super jump map flag. |
| `ts_wind` | Full: `TSPSys` type 3. |
| `ts_fog`, `ts_emitter`, `ts_particle` | Validated (emitter rates, 512 particles, `soundonemit` precached) and removed: the client reads the BSP lump. |
| `ts_trigger`, `ts_dmhill`, `ts_timer`, `ts_customtarget`, `ts_playeramount`, `ts_slowmotion`, `ts_slowmotionpoint` | Full per §5 (TS quirks kept: `ts_trigger` team defaults to index 0 and always prints its message; `ts_timer` flag 2 ignored). |
| `ts_briefcase`, `ts_flag`, `ts_scoregiver`, `ts_wingiver` | CTB: case at home 1 s after spawn, take (20 slots), drop on death/observer, 20 s return, owner return +5, capture +10 and team score, TSFlag/TSHVoice/TeamScore. Score/win givers: team score, "%s Score!/Win!", TSHVoice, TeamScore (scoregiver flag 1 restarts the round). |
| `deadcorpse` | Created by the death think (player model, `kRenderFxDeadPlayer`, index in `renderamt` and `rendercolor.x`), removed after 10 s. |
| `info_player_team1..4`, `info_player_teamspawn` | Inert points (team spawns are found by classname). TS doesn't export `info_player_teamspawn`; kept inert. |
| `ts_objective_manager`, `ts_objective_ptr`, `ts_bomb`, `ts_hack`, `ts_teamescape`, `ts_sellzone` | Stubs: hidden, non-solid (dead in 3.0). |

## 5. Messages sent (payloads in `shared::protocol`)

| Message | Dest | Payload | Sent from |
|---|---|---|---|
| ResetHUD | ONE | `B 0` | spawn / observer start (next think) |
| InitHUD | ONE | – | first spawn |
| GameMode | ONE | `B teamplay` | InitHUD |
| ScoreInfo | ONE/ALL | `B idx, S frags, S deaths, S 0, S team+1` | InitHUD, kills, team change, dmhill, CTB, LMS |
| ServerName, MOTD | ONE | `Str` / `B last, Str ≤60` | InitHUD, `motd` |
| TeamNames | ONE/ALL | `B n, n×Str` | teamplay InitHUD, team change |
| TeamInfo | ONE/ALL | `B idx, Str team` | teamplay, observer start, The One |
| TeamScore | ALL | `Str team, S score, S 0` | captures, wins |
| DeathMsg | ALL | `B killer, B victim, Str weapon` | PlayerKilled |
| SayText | ONE/ALL | `B sender, Str` | chat, join/leave, name change |
| TextMsg | ONE/ALL | `B dest, Str` | join notice, timeleft, TK kick |
| TSMessage | ONE/ALL | `B r,g,b, B effect, B hold, Str ≤127` | all TS HUD text |
| TSHealth / TSArmor / TSState / TSSpace | ONE | `B` | hud::update, Killed |
| TSCash | ONE | `S` | hud::update, StartObserver |
| PwUp | ONE | `S bits, B secs` | hud::update (held slot), permanent kung fu/super jump as `(4,0)`/`(0x100,0)` |
| PwUpTime | ONE | `B secs` | UsePowerUp |
| TSSlowMo | ONE | `Co factor×100` | hud::update |
| TSBTime | ONE | `B` | hud::update |
| TSMBlur | PVS | `B ent, B mode` | UsePowerUp, Killed |
| TSPAward | ALL | `B ent, B award` | hud::update |
| PTakeDam | PVS | `Co pos×3, Co dir×3, B amount` | damage |
| TSFade | ONE | `S dur, S hold, S flags, B r,g,b,a` | spawn protection, damage |
| Spectator | ALL/ONE | `B ent, B observer` | observer start/stop, TS InitHUD, spawn |
| ActItems | ALL | `B ent, B 0` | Killed, StartObserver |
| SetFOV | ONE | `B 0` | Killed, StartObserver |
| SrvSett | ONE/ALL | `B key, S value` | InitHUD, cvar changes, weaponrestriction |
| RoundTime | ONE/ALL | `B secs, B state` | LMS rounds, InitHUD |
| TSFlag | ALL | `B state, B id, B team+1, B holder` | CTB |
| TSHVoice | ONE | `B lost` | captures, wins |
| TSPSys | ALL | `B 3, L seed, L yaw, Co strength` | ts_wind |
| VGUIMenu | ONE | `B 2` | changeteam |
| PlInfo | ALL | `B ent, B 0, B colour` | lasclr_N |
| VoiceMask | ONE | `L audible, L banned` | voice `UpdateMasks` on change |
| ReqState | ONE | – | voice `UpdateMasks` until the client answers `VModEnable` |
| engine msg 50 (`SVC_HLTV`) | SPEC | `B 3 (DRC_EVENT), S primary, S secondary, L flags` | `DeathNotice` (victim, inflictor or killer, `0x27`), player `TakeDamage` (self, inflictor, `5`; after the damage, also when `takedamage` is off). TS's old director layout (no length byte, not the SDK's `svc_director` 51); only HLTV proxies receive `MSG_SPEC`. `msg::director_event`, payload `hud_wire::DirectorEvent`. |

## 6. Deviations from TS 3.0

* `addbot`/`addcustombot` add a fake client; `ts_bot_quota` is new (dedicated-server testing).
* `GetGameDescription` returns `rules::game_description()` (wired at integration).
* Map votes: `CheckVotes` also tallies slow-match / teamplay / friendly-fire toggle votes, but
  nothing in TS ever sets them (and the slow-match one writes the non-existent `mp_slowmatch`), so
  only map votes are ported.
* `PlayerIsCheating` / forced `cl_lw 1` is not implemented.
* Spawn points: the `IsTriggered` master check is skipped (stock spawn points store no master).
* A death sequence without studio data (frame rate 0) ends the dying state at once; in TS it
  would never finish. The death/flinch sequences are left to the stunt package (`CTSAnimation`).
* `+use` on ground weapons uses a simplified `PlayerUse`: the best item within 64 units in a 60°
  cone in front of the eyes.
* `wextraclip` gives `extra × clip` rounds (TS passes the raw number to `AddAmmo`). Like TS, it
  is only given when the reserve is empty after the pickup.
* `ts_mapglobals` masks the weapon set to 5 bits (TS XORs the whole spawnflags).
* Round modes 2/3, objectives, buy zones, cash-based buying stay off (dead in 3.0); `TeamWin`
  exists only as the score/win giver path.
* `jointeam` (voluntary, not The One) puts the player in observer with a 5 s respawn.
* `PlayerSpawn` with a `game_player_equip`: TS gives no `weapon_tsgun` (no gun at all unless
  the equip lists it) and doesn't copy the stored cash; the port's TS gun is part of the player,
  so the kung fu gun is kept. The equip touches run right after the player's spawn instead of
  inside it.
* `VoiceMask`/`ReqState` are registered once in `GameInit` after the TS table (TS registers them
  in the `CHalfLifeMultiplay` constructor, before `LinkUserMessages`), so their message numbers
  differ; clients look messages up by name, so this doesn't affect compatibility.
* When a `vban`/`VModEnable` arrives, the sender's own `Player` is checked out for the command,
  so the immediate mask update treats that player as in-round for the LMS dead-talk rule; the
  next 0.3 s update corrects it.
* A suicide doesn't make the player its own spectator target (TS sets `m_hObserverTarget` to
  the dead player and `Observer_Idle` replaces it within 0.5 s).

## 7. Missing / partial

* `loadweapons`/`saveweapons`, `CustomWP`.
* Untested in-engine: CTB (no test map with briefcases), `ts_dmhill`, `ts_slowmotion`,
  `ts_wind`, ground ammo pickup (weapons stubs give nothing), chat from a real client.

## 8. Tests

* `cargo test`: payload encoders (sizes match registration), `decide_gameplay`, team lists,
  spawn scoring, easing, damage multipliers/kevlar/PTakeDam byte, chat formatting, globalslow
  clamps, weapon restriction, mapcycle, map vote parsing/tally, round refunds, power-up bodies/respawn clamps, HL remaps, the HLTV
  director event layout (`hud_wire::director_event_layout`), in-eye spectator movement at mode 5
  (`pm::tests::in_eye_spectator_is_mode_5`).
* Not yet run in-engine: the TS observer mode cycle (jump: roaming → chase free → chase locked →
  in-eye) and target stepping, and the director messages (they need an HLTV proxy on a Steam
  server to be seen at all).
* Builds: `cargo build -p ts-server -p ts-client` (x86_64), `--target i686-unknown-linux-gnu`,
  `--target i686-pc-windows-gnu`.
* In-engine (FWGS root `~/games/xash_rules`):
  * `cargo xtask server fwgs --map ts_awaken --bots 4 --seconds 60 --port 27230`: **PASS**
    (4 bots, no panic). Also `ts_lobby` with 6 bots: PASS.
  * Scripted sessions with `ts_status`/`ts_hurt`: joiners start as observers and respawn after
    5 s; head ×2 / chest ×1 damage; death → corpse + observer → respawn; killer frags/cash
    (+900/+900); double kill ×2; LMS (+10 last man, round restart, out-of-round award 1);
    The One selection and team split; slow-motion power-up flow (activator 0.6, others 0.35,
    restored on expiry); health power-up 150 with decay; teamplay assignment from
    `mp_teamlist`; `mp_timelimit` → intermission → changelevel via mapcycle.

## 9. Integration notes

**Weapons package**
* Implement `give_weapon`, `give_ammo`, `strip_all`, `multiply_clip`, `on_killed` (drop weapons).
  Ground items, power-ups (grenade 24, infinite clip), observers and death call them.
* Read `Player::active_powerup` for double fire rate (0x10) and infinite clip (0x2),
  `Player::kungfu` for ×2 melee.
* Fill `DamageInfo` fully: `hitgroup` (traced damage is detected by hitgroup ≠ 0 or
  `DMG_BULLET`), `pos`, `dir` (travel direction), `weapon` id (0 = kung fu → close combat),
  `inflictor` (`"knife"` entity for thrown knives), bit `0x2000000` for shotgun pellets,
  `DMG_THROWN` for thrown knives.
* Register no cvars that are in `rules::cvars::CVARS` (`realbullet`, `ammocount`, `damagemult`,
  `customweapons`, `weaponstay`): read them by name.
* `Player::slots_used` drives `TSSpace` and the kevlar/flag slot costs (rules adds/removes 10/20).
* Dropped guns should use classname `WorldGun` and `pev->owner` = dropper for the glow shell and
  slow-motion propagation.

**Stunt / movement package**
* `Player::slow_target` is written by the rules; `pev->fuser1` is eased in `rules::pre_think`
  (before the stunt hook). The rules also set `pev->framerate = max(fuser1, 0.01)`.
* Set `PlayerRules::dodge_until` for the bullet-time dodge window (×0.7 bullets).
* `iuser4 = DEAD_LOCK` is set on death; the death sequence (`CTSAnimation::Animate(7)`) is
  yours. Fall damage (680/1124 u/s) is applied in `rules::post_think` from the airborne
  `-velocity.z` recorded in `pre_think`; don't apply it a second time.
* `mj` physinfo is set by the rules (spawn with the map flag, super jump pickup, specialist,
  The One).

**Client / HUD package**
* Payload structs in `shared::protocol::{hud,rules}` have `read(&mut impl WireRead)`; implement
  `WireRead` for the client message reader (coords are `short / 8`).
* Players join as observers: on `+attack` while observing, send the `respawn` command (TS client
  behaviour). `usepowerup` activates the held power-up (+use double tap works server-side too).

**Server entry points (`server-dll/src/lib.rs`)**
* `GetGameDescription` returns `rules::game_description()` (done at integration).
