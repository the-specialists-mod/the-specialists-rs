# Rules package: game rules, player lifecycle, TS entities, HUD sync, bots

Owner files: `crates/server/src/rules/**`, `entities/ts/**`, `player/damage.rs`, `player/water.rs`, `player/suit.rs`,
`player/hud.rs`, `bots/**`, `net.rs`, `crates/goldsrc/src/ext/rules.rs`, `crates/shared/src/protocol/{rules,hud}.rs`.
Source of truth: the decomp (`docs/server/gamerules-entities.md`, `architecture.md` §2–§5, §7,
`player-movement.md` §5–§7, `weapons.md` §5.9, `client/architecture-hud.md` §3). Addresses below
are Ghidra addresses.

## 1. Layout

| Rust | Contents |
|---|---|
| `rules/state.rs` | `GameRules` singleton: `g_pGameRules` (DM/Teamplay) and `CTSGameRules TSGameRules` folded into one struct (`Mode`, `lms`, `the_one`, teams, spawner, round, global slow, map flags). `decide_gameplay`, team lists, `random_powerup`. Accessed with `state::with(|g| …)` (short borrows, no re-entrant calls inside). |
| `rules/cvars.rs` | TS cvar table and registration, clamped helpers (`respawn_time`, `round_time`). |
| `rules/lifecycle.rs` | connect / disconnect / kill / userinfo, `TSInit` + spawn, InitHUD (both rules objects), observer, `respawn`, `Killed`, `PlayerDeathThink`, fall damage, the `+use` double tap (`TSUse`; using objects is `player::player_use`). |
| `rules/scoring.rs` | `PlayerKilled`, `TSGetPointsForFrag`, `DeathNotice`, The One handover. |
| `rules/spawnpoint.rs` | `EntSelectSpawnPoint` + `TSSelectSpawnSpot` + `GetPlayerSpawnSpot`. |
| `rules/powerup.rs` | held/active power-ups, `SlowMotionFlow`, `GoSlow`, `InterpolateSlowMotion`, `TakeArmor`. |
| `rules/frame.rs` | `StartFrame`: Multiplay `Think` (limits, intermission) and `CTSGameRules::Think` (global slow, auto power-ups, LMS rounds, The One, SrvSett). |
| `rules/mapcycle.rs` | `ChangeLevel` and the SDK map cycle (`ReloadMapCycleFile`, min/max players, rule commands). |
| `rules/teams.rs` | teamplay assignment, `ChangePlayerTeam`, TeamInfo/TeamNames/TeamScore. |
| `rules/commands.rs` | client commands owned by the rules, `Host_Say`, server commands. |
| `rules/msg.rs` | `Wire` adapter over `MsgWriter`; `TSHudMessageOne/All`, `ClientPrint`, `SayText`, log. |
| `rules/voice.rs` | `CVoiceGameMgr` + `CMultiplayGameMgrHelper`: who hears whom, `vban`/`VModEnable`, `VoiceMask`/`ReqState`. |
| `player/damage.rs` | `TraceAttack` + `TakeDamage`. |
| `player/water.rs` | `WaterMove`, `CheckTimeBasedDamage` and the `m_bitsDamageType` / time-based bookkeeping (`TimeBasedDamage`, `Player::tbd`). |
| `player/suit.rs` | The HEV suit sentence queue: `SetSuitUpdate`, `CheckSuitUpdate`, `EMIT_GROUPID_SUIT` (`SuitQueue`, `Player::suit`). |
| `player/hud.rs` | `CBasePlayer::UpdateClientData`: the 0.1 s throttle, the InitHUD block, HUD shadow copies, the gun's update. |
| `player/impulse.rs` | The spray (impulse 201, `CSprayCan`), `CheatImpulseCommands` (`CBloodSplat`, `FindEntityForward`), `PlayerCustomization` and the spray state (`Player::impulse`). |
| `entities/ts/*` | TS map entities (§4). |
| `bots/*` | TS's bot AI (`mp.dll` only) and the port's `ts_bot_quota`: §10. |
| `net.rs` | `AddToFullPack` TS changes (CreateBaseline/UpdateClientData were already stock-correct). |
| `shared/protocol/{hud,rules}.rs` | payload structs (`write`/`read` over `Wire`/`WireRead`, `Bytes` for tests). |

## 2. Features: TS function → Rust

| TS (address) | Rust | Notes |
|---|---|---|
| `DecideGamePlay` 0x767ac | `state::decide_gameplay` | worldspawn `gamemode` 1–4 wins over cvars; The One > LMS > `mp_teamplay`. Unit tested. |
| `InstallGameRules` 0xc6050, `CTSGameRules::Init` 0x77d6c, Teamplay ctor 0xe…, `InitTeams`/`SetTeamList`/`SetTeamModelList` | `rules::install` → `GameRules::install` (`ts_init` is `CTSGameRules::Init`) | `exec game.cfg` + `ServerExecute` first, like TS. With `deathmatch` 0 the rules are `CHalfLifeRules` (`install_single_player`, §2.1). Then, as the `CHalfLifeMultiplay` ctor (0xcedd0) does, queues `exec <servercfgfile>` (`lservercfgfile` on a listen server) without executing it, so the server config runs after every map load and overrides what `CWorld::Precache` forced (`sv_gravity 800`, `mp_defaultteam`). The One renames team 0 "The ONE" and skips the first listed name (TS quirk kept). |
| `GameDLLInit` 0xc5aa0 | `rules::game_init` | cvars + 7 TS server commands (+3 test commands). |
| `GetGameDescription` 0xaee84 | `rules::game_description` | "Half-Life" before the rules exist, "The Specialists" when `deathmatch` was 0, else "The Specialists (The One/LMS/Teamplay/DM)". |
| `ClientConnect` 0xacce8 | `lifecycle::client_connect` | `ts_playeramount` connect hook, accepts. |
| `ClientDisconnect` 0xacd4c, `ClientDisconnected` 0xcf960, `TSPlayerDisconnect` 0x9f94c | `lifecycle::{client_disconnect, client_disconnected}` | `ts_playeramount`; then only while the game isn't over: "- %s has left the game", not solid, and `ClientDisconnected`: `game_playerleave`, the log line (`<uid>` again in DM, the `model` key in teamplay), `RemoveAllItems(TRUE)` (strip weapons), `TSPlayerDisconnect` (drop the case, the vote record cleared for slots < 32, not solid, `DEAD_DEAD`, no damage, `FL_FAKECLIENT` off, `FL_DORMANT` on, frags/deaths 0, respawn time -1, no team, `EF_NODRAW`, health 0, not The One, `CallCheckDeads`), and the next The One check at once (`+0x587c = 0`). The slow-motion flow isn't stopped and the players it slowed stay slowed, as in TS. The engine keeps the edict and its `Player` until the slot's next client spawns, so every TS `UTIL_PlayerByIndex` loop (`rules::slot_players`) still finds the player: `RecountTeams` (one `""` team without a team limit), `TeamWithFewestPlayers`, the InitHUD `ScoreInfo`/`TeamInfo`/`Spectator` loops, the multiplay fraglimit, `CountPlayers` (CTB), `CheckVotes`, the voice masks, The One, `GlobalSlowMotion2`, `SlowMotionFlow`, `ts_slowmotionpoint`, `CheckDeads`, `RestartRound`, `TSSelectSpawnSpot` and `Observer_FindNextPlayer`. Every TS player test (`IsPlayer()`, `IsNetClient()`, `Classify() == CLASS_PLAYER`, the classname `"player"`, `pev->flags & FL_CLIENT`) is `rules::has_player` / `rules::ts_flags` in the port, so it is still true for the player who left (§6). |
| `ClientKill` 0xacec8 | `lifecycle::client_kill` + `pre_think` | suicide 1 s later, not while observing. |
| `ClientPutInServer` 0xacf44 | `player::client_put_in_server`, first `on_spawn` | `SetCustomDecalFrames(-1)`, cash = `initial_cash` "1200", `EF_NOINTERP`, then `Spawn`. |
| `PlayerCustomization` 0xaeef0, `SetCustomDecalFrames` 0xdd488 | `player::impulse::player_customization` (thin `server-dll` callback) | An uploaded `t_decal` sets the spray's frame count (`nUserData2`, kept only for 1..7, else -1); sounds, skins and models are ignored; other types, a NULL customization or no player print TS's console notes. |
| `ClientUserInfoChanged` 0xae638, `CHalfLifeMultiplay::ClientUserInfoChanged` 0x8b244, `CHalfLifeTeamplay::ClientUserInfoChanged` 0xe4d18 | `lifecycle::{userinfo_changed, multiplay_userinfo_changed}`, `teams::userinfo_changed` | Name change (`%` → space, the key always rewritten, "* %s changed name to %s" SayText to all, log with `<uid>` in DM or the `model` key in teamplay). Then Multiplay: `PlayerIsCheating` (0x9fae0: `sscanf(cl_lw, "%i") != 1`) sets `cl_lw 1` and skips the model check; otherwise a `model` key that differs (case-insensitively) from the team name (`""` in DM) is put back to it, with "* Can't change team to '%s'" and "* HLDM models not allowed.", when it is an HLDM model (`IsValidTeam(char*)` 0x8b140) or `mp_defaultteam` is on. Teamplay, for a `model` still different from the team: with a team limit `GetPlayerModel` (a random team model); without one, under `mp_defaultteam` the `model` and `team` keys are put back ("* Not allowed to change teams in this game!"), otherwise the model is a team change: "* %s has changed to team '%s'" to all, `joined team` log, `ChangePlayerTeam(model, 0, 0)` (observer, 5 s) and `RecountTeams(true)`. |
| `CBasePlayer::Spawn` / `TSInit` 0x92a4c / `PlayerSpawn` 0x890ac | `lifecycle::on_spawn` | fuser1/slow reset, speed 330, 81 free slots and no loadout items (`weapons::ts_init`; the only place they are reset), `slj`/`hl`/`mj` physinfo, spawn protection 1 s (transparent, `TSFade` 200,200,250), power-ups reset, `weapons |= 0x80000000` (the suit, from the multiplayer `PlayerSpawn`), default team (`SetDefaultPlayerTeam`, name only), spawn spot. A map's `game_player_equip`s replace both the gun and the loadout (`stock::maprules::equip_on_spawn` from `weapons::on_spawn`; without one, `weapons::give_gun` + `BuyArmament`). |
| `UpdateClientData` init block 0x91d00 | `lifecycle::init_hud_block` (from `hud::update_client_data`, inside its throttle) | `ResetHUD` every spawn and observer start; first time `InitHUD`, rules InitHUD, TS InitHUD, `game_playerjoin`; `game_playerspawn`; RoundTime; super jump PwUp. Nothing is reset for a resend here (the spawn did that); the changed values follow in the same update. |
| `CHalfLifeMultiplay::InitHUD` 0xcf618 / Teamplay 0xe45d8 | `lifecycle::rules_init_hud`, `teams::init_hud` | join TextMsg, log, `GameMode`, own ScoreInfo, MOTD, everyone's ScoreInfo; teamplay: `SetDefaultPlayerTeam` first, TeamNames, assignment SayText (`* you are on team '%s'` when the `model` key equals the team, case-sensitive `strcmp`; sent after `ChangePlayerTeam`, so a teamplay joiner already starts observer here, before `CTSGameRules::InitHUD` does it again), then the TeamInfo of every player on a valid team (`IsValidTeam`, so every player without a team limit). |
| Teamplay constructor 0xe3e48 (team part), `RecountTeams` 0xe5608, `GetTeamIndex` 0xe5434, `IsValidTeam` 0xe54d4, `TeamWithFewestPlayers` 0xe5510 | `GameRules::{init_teamplay_list, recount_teams, tp_team_index, is_valid_team, team_with_fewest}`, `teams::{recount, score_team}` | Two team lists, as in TS. The `TSGameRules` teams (`GameRules::teams`, `CTSGameRules::GetTeamIndex` 0x79b04 = `team_index`) hold names, models, spawn classes and CTB scores. The Teamplay list (`team_names`/`num_teams`/`team_scores`, `tp_names`/`num_teams`/`tp_scores`) is rebuilt by every `RecountTeams` from `m_szTeamList` (the non-empty `TSGameRules` names, deduplicated). Fewer than two listed teams mean no teams and no team limit (`+0x214`, cleared for good): every player's team that isn't listed yet is then added, `""` included (one empty slot per teamless player, since `GetTeamIndex("")` is -1), and `IsValidTeam` accepts any name. `TeamNames`, `jointeam`, the smallest team, the teamplay fraglimit (`team_scores`) and the `ScoreInfo` team (`GetTeamIndex + 1` at send time, `teams::score_team`) use this list. `RecountTeams(true)` (from `SetDefaultPlayerTeam`) also sends every player on a valid team to all as `TeamInfo`, and in CTB that player the `TeamScore` of each listed team. Unit tested (`state::tests`). |
| `CHalfLifeTeamplay::SetDefaultPlayerTeam` 0xe44ac | `teams::set_default_team` | Every teamplay spawn and InitHUD: `m_szTeamName` = the `team` key (The One mode: `TSGameRules` team 0 for The One, team 1 for the rest), `RecountTeams(true)`, then unless it is non-empty, valid and `mp_defaultteam` is off: `team_names[0]` under `mp_defaultteam` (slot 0 whatever `num_teams` says, so without a team limit a player's team or the stale listed name), else the smallest team. Only the name is stored: the `model` and `team` keys come from `ChangePlayerTeam` (InitHUD, `jointeam`), so after a respawn the keys keep what the last team change set. |
| `CHalfLifeTeamplay::ChangePlayerTeam` 0xe488c, `GetIndexedTeamName` 0xe54a0 | `teams::change_team`, `GameRules::indexed_team_name` | `jointeam <n>` (team `n - 1`, `""` out of range) and the teamplay InitHUD: The One mode puts The One on the first `TSGameRules` team and everyone else on the second; an empty name or one `IsValidTeam` refuses goes to the smallest team (without a team limit any other name is kept); with kill set, `TakeDamage(900)` (never/always gib) whether or not the player is alive, with `PlayerKilled` and the death notice off for the call (`+0x20c`/`+0x210`, `GameRules::death_suppressed`); **always** the `model` key (with a team limit `+0x214`, i.e. two or more listed teams, `GetPlayerModel` 0x79a08: a random team model or `""`; without one the team name) and the `team` key, RecountTeams, TeamInfo + ScoreInfo to all, then **unless it's a kill or The One mode, always** `StartObserver` + `SetPlayerRespawnTime(5)` (0xe4c38..0xe4ca1; respawn award 11), even for the same team or an observer; TeamNames to all. Unit tested (`teams::tests`). Outside teamplay `CGameRules::ChangePlayerTeam` (0xc632c) is empty, so `jointeam` does nothing. |
| `CBasePlayer::ForceClientDllUpdate` 0x92594 | `commands::force_client_dll_update` (`fullupdate`, impulse 204) | forget the player's last-sent HUD values (`hud::force_reset`: not `TSPAward`, not the gun's `WStatus`/`WeaponInfo` caches, `ActItems` back to 0), flag a new train HUD, reset the throttle and run `UpdateClientData` inline, then `Spectator` (own index, observer = `deadflag == DEAD_RESPAWNABLE`, 0x9267b) to the player; no `m_fInitHUD`, so no `ResetHUD` unless one was pending. |
| `CBasePlayer::ImpulseCommands` 0xdb780 | `player_use::impulse_commands` (from the weapons' PostThink, when the gun is ready) | `PlayerUse`, then impulse 201: the spray; 99: `Logo` registered on first use (`REG_USER_MSG("Logo", 1)`) and sent as 1, the next press sends 0 and forgets the number; 204: `ForceClientDllUpdate`; anything else (0 included) `CheatImpulseCommands`; `pev->impulse = 0` after any impulse. |
| Impulse 201, `CSprayCan` 0xdb0d4/0xdb1c4 | `impulse::{spray, SprayCan}` | Once `m_flNextDecalTime` (0 every spawn) has passed and a wall is within 128 units of the eyes (monsters ignored): next spray in `decalfrequency`, and a class-less entity 32 units above the origin, facing `v_angle`, owned by the player, `player/sprayer.wav` (`CHAN_VOICE`). Every 0.1 s it traces 128 units ahead: without a custom spray (`-1`) the `{lambda06` decal and it's gone; with one, `TE_PLAYERDECAL` frame by frame (`pev->frame`) until the last. |
| `CBasePlayer::CheatImpulseCommands` 0xdb9e4, `CBloodSplat` 0xdb360/0xdb428, `FindEntityForward` 0xdb5f4 | `impulse::cheat_impulse_commands` | Only with `g_flWeaponCheat` (`cvars::cheats`). 102: one human gib (`CGib::SpawnRandomGibs`); 103: the forward trace only (`ReportAIState` is empty); 104: `-- Globals --` and `name: map (Off/On/Dead)` per `env_global` state, newest first; 105: `m_fNoPlayerSound` toggle ("Player is silent"/"Player is audible"); 106: classname, target name, model and global name of what is ahead (8192 units, monsters included, not the world); 107: the texture 1024 units along the stale `v_forward`; 195–197: `CBaseEntity::Create("node_viewer_*")`, which neither library has ("NULL Ent in Create!"); 199: "0" (`FindNearestNode` is a stub); 202: a class-less blood splat that sprays a red blood decal 128 units ahead after 0.1 s (none with `g_Language` 1) and is removed 0.1 s later. TS 3.0 has no impulse 101. |
| `CTSGameRules::CallCheckDeads` 0x79bc0 | `GameRules::call_check_deads` | `CheckDeads` in 1 s, only while no round restart is pending; from `PlayerKilled`, the LMS join deadline in `PreThink` and `TSPlayerDisconnect`. Unit tested. |
| `SendMOTDToClient` 0xd0ae4 | `lifecycle::send_motd` | `ServerName` = hostname, `motdfile` (default `motd.txt`) in ≤60-char chunks, ≤0x600 bytes. |
| `CTSGameRules::InitHUD` 0x79134 | `lifecycle::ts_init_hud` | cheat notices (`g_flWeaponCheat`), `SrvSett` 0–3 (0x79214..: 0 is Think's `realbullet` copy `+0x578`, -1 before the first Think; 1 the live `usecash != 0`, so 0.5 sends 1 here but 0 from Think, and NaN 0; 2 the live `(int)ammocount`; 3 the weapon mask), RoundTime (`max(end - time, 0)` truncated, low byte), `Spectator` of everyone, **joiner starts as observer** with `respawntime` (5..30 s, NaN 5, truncated: `cvars::respawn_time`, as `Killed` 0x8ef8f). |
| `SetPlayerRespawnTime` 0x75ff4 | `lifecycle::set_respawn_time` | LMS respawn budget (`respawnnumber` → 1), join deadline; award 1 / 11. |
| `StartObserver` 0x99634 / `StopObserver` 0x99a14 | `lifecycle::{start_observer, stop_observer}` | TSCash resend, TE_KILLPLAYERATTACHMENTS (at `pev->origin`), the active item holstered as TS does (`weapons::observer_holster`: for the gun `CTSGun::Reset`, `HolsterWeapon`, then `Holster`, i.e. `HolsterWeapon` again, 0x9971e..0x9974f; a cycler's `Holster`), so the gun is emptied before either holster runs (a primed grenade isn't thrown) and a living player gets two kung-fu holster animations and draw events, the CTB case dropped before the inventory is cleared, a tank let go (its `StopControl` redeploys the emptied gun at once: the player is lent, `drive::release_tank`), `SetSuitUpdate(NULL)` (0x997ac), ActItems 0, SetFOV 0, `EF_NODRAW`, deadflag 3, health 1, velocity left as it was, TeamInfo, then the `RemoveAllItems(TRUE)` (`weapons::remove_all_items`: the active item's third `Holster`, after `deadflag` is set so it sends nothing; free slots, loadout items and speed left alone, so no `TSSpace` 81 or speed change; then the tail: no view/weapon model, `pev->weapons = 0` so the HEV suit bit and its geiger counter go, `UpdateClientData` inline, so ResetHUD goes out here when the 0.1 s throttle allows, then `CurWeapon 0,0,0`)), `Spectator 1`, starts in the saved mode (chase free the first time); stop saves the mode. |
| `Observer_SetMode` 0x99e44 / `Observer_FindNextPlayer` 0x99b0c / `Observer_HandleButtons` 0x99d34 / `Observer_Idle` 0x99ca4 | `lifecycle::{observer_set_mode, observer_find_next_player, observer_handle_buttons, observer_idle}` | TS observer modes (`shared::ts::obs`): 1 chase locked, 2 chase free, 3 roaming (maxspeed 420), **5 in-eye**; there are no map modes and 4 is HLTV-only. Jump cycles 3 → 2 → 1 → 5 → 3, forward/back pick the next/previous target (not while roaming), 0.2 s apart. The target (`m_hObserverTarget`, `PlayerRules::obs_target`) is the killer (last inflictor if a player), kept across lives; the next one is a living, visible, non-observing player, only teammates in LMS while a respawn is pending. Twice a second a dead or observing target is replaced. No target → roaming. The camera position is `PM_SpectatorMove`'s. |
| `respawn` client command (ClientCommand) | `lifecycle::respawn_command` | only while observing and after `respawn_at`; "You have to wait until the next round." |
| `CHalfLifeMultiplay::PlayerThink` 0xcfb60 | `lifecycle::player_think` | from the head of `PreThink`, after the button edges: during intermission a newly pressed attack/attack2/jump/duck/use (0x827, against `m_afButtonLast`, which PostThink no longer updates) sets `end_intermission_button_hit`, and the edges and `pev->button` are cleared. It returns true then and `Player::pre_think` stops, as TS returns on `g_fGameOver`: no water, time-based damage, stunt, weapon timers or train controls in intermission. |
| `CBasePlayer::PreThink` 0x908cc (rules parts) | `lifecycle::pre_think` | `WaterMove`, InitHUD, `CheckTimeBasedDamage`, `CheckSuitUpdate` (`suit::check_suit_update`: with the HEV suit bit the geiger counter, and in single player the next queued sentence), fuser1 easing + `framerate`, power-up state, slow flow, health decay / The One regen, observer countdown text (once a second while the respawn time is ahead; seconds truncated like TS's `(int)` cast, capped at 30; "Press Fire To Play!" in the last second, not repeated; the LMS join countdown also truncates), death think, +use double tap, spawn protection end, delayed suicide. |
| `CBasePlayer::Killed` 0x8ed5c | `lifecycle::killed` | In TS's order: TSMBlur on the killer (`RandomLong(0,100) <= 0x31`; under `glast_closecombat` a first roll `<= 0x4f`, then the 0x31 roll again on a miss) and off on the victim, respawn time, headshot victim slow 0.33, `iuser4 = DEAD_LOCK`, `weapons::on_killed` (holster and ammo pack with a gun, `DropFlag`, then every `DropWeapon`), PlayerKilled, tank let go (`USE_OFF`), `Animate(7)` (reads `g_vecAttackDir`), push `velocity += -min(DamageForce(last damage), 500) × normalize(g_vecAttackDir)` (`DamageForce` 0xb2d08 = `73728 / volume × dmg × 5`, at most 1000: a ducked body flies twice as far), so away from the inflictor, hop when fast/hard hit (TS also tests `pl+0x470`, a word nothing writes), TSHealth 0, `SetSuitUpdate(NULL)` (0x8f3e9: the suit's play list is emptied), TSHealth 0, ActItems 0, SetFOV 0, gib or schedule `PlayerDeathThink` (0.1 s). `takedamage` stays on (only `StartObserver` clears it), so the body still takes TraceAttacks; slow motion isn't stopped here (the dead player's next `UpdatePowerUpState` does it). |
| `PlayerDeathThink` 0x8eaec | `lifecycle::death_think` | Runs from every PreThink of a dead, non-observing player (`deadflag > 0`, call at 0x91106), and as the player's think: `Killed` sets `m_pfnThink = PlayerDeathThink` and `nextthink = time + 0.1` (double constant, 0x8f550) for a death that doesn't gib, and the engine's `SV_PlayerRunThink` calls `pfnThink` from the player's usercmd, right after PreThink, once that is due (`Player::think`, `CBaseEntity::Think` 0xe2c50, which `CBasePlayer` doesn't override). So the tick 0.1 s after the death runs it twice; nothing clears `m_pfnThink`, and `StopObserver` sets `nextthink = -1`. It does: settle; `RemoveAllItems(TRUE)` while the player still has the gun (`weapons::remove_all_items`: the gun goes, inline `UpdateClientData`, `CurWeapon 0,0,0`); while dying, advance the death sequence (`studio_frame_advance`, at the slow-motion `framerate`) until it finishes; then corpse (`deadcorpse`, 10 s) + observer after 3 s or any button. |
| `CHalfLifeMultiplay::PlayerKilled` 0x895e4 | `scoring::player_killed` | The killer is any `CBasePlayer` (`IsPlayer()` 0x896dc, `rules::has_player`) and the weapon name and log test its `FL_CLIENT` (0x8986d, 0x89da3, `rules::ts_flags`), so a killer who has left (its grenade still flying) is still scored, named and logged as the killer. TS's damage globals as `Killed` took them (`damage::glast::Snapshot`: `glast_headshot`, `glast_closecombat`, `glast_coldcock`, `glast_weaponid`), points (kung fu/thrown knife 2, stunt/slide +1), sprees 3/6/9/10+ (hp, kung fu, super jump, stored slow pause, awards 2/3/4/10), `#TS_KnifeKill`/`#TS_KatanaKill` from the killer's current gun weapon (own inflictor, not under `glast_closecombat`), `#TS_Coldcock`, double kill ×2, killed-specialist +5, suicide −1 (−5 more at spree > 8), cash `points×500+400` / victim +900, spectacular slow-motion point (`DoSpectacularNess` 0x76b48 = `scoring::do_spectacularness`, shared with explosions: `spectacularness` clamped 0..100 and rounded, 0 never fires), `slowmatch` stored slow-mo (max 3 s), TK kick/ban (`banfortk`), all `#TS_*` TSMessage lines, the log line (`"name<uid><authid><uid>"`, the `model` key instead of the second user id in teamplay; tags in TS's order: ` (victims_weapon)` when the killer's current weapon record was first owned by the victim (`s_weapon_status+0x30`: `weapons::SvState::owners`, set by a buy and carried by a dropped gun) and not a grenade or blade, killed by the killer's own hand outside `glast_closecombat`; ` (stopped_fury "victim")`, ` (killed_specialist)`, ` (double_kill)`, ` (diving)`, ` (sliding)`, ` (killed_the_one)`, then the spree tag; `scoring::{KillTags, log_kill}`), `game_playerdie` (the victim lent back to its slot, `world::lend`)/`game_playerkill`, ScoreInfo, CheckDeads in 1 s. |
| `TSGetPointsForFrag` 0x89140 | `scoring::points_for_frag` | Own inflictor: `#TS_KungFu` and 2 points outside a simulated bullet's hit, under `glast_closecombat` or with kung fu out (`scoring::kung_fu_frag`); a `knife` inflictor: the blade's `CTSKnife::GetDescription` (0x7db04: "Throwing Combat Knife", "Throwing Seal Knife", "Throwing Katana"; "Throwing" without a knife object) and 2; any other inflictor returns the base points (no stunt bonus, no message). Then `#TS_StStl`/`#TS_SlStl` +1 (not under `glast_closecombat`) or `#TS_Stl`, and the frag message naming the killer. Unit tested. |
| `DeathNotice` 0x88f7c, `CHalfLifeTeamplay::DeathNotice` 0xe509c | `scoring::death_notice` | In teamplay a kill by a teammate (attacker with `FL_CLIENT`, not the victim, `PlayerRelationship == GR_TEAMMATE`) is only `DeathMsg(killer, victim, "teammate")`, with no director event. Otherwise `DeathMsg` (killer index only for an attacker with `FL_CLIENT`, which TS keeps on a player who left: `rules::ts_flags`); weapon name rules (`scoring::weapon_name`: a thrown knife of a player is named by `GetDescription`, "world" without its object; another non-player inflictor by its netname when set at all, else its classname; strip `weapon_`/`monster_`/`func_`; then only `knife`, `shrapnel`, `grenade` are renamed (Throwing Knife, Shrapnel, M61 Grenade), and an empty name stays empty; Kung Fu); HLTV director event (victim, inflictor or killer, 0x27). |
| `TheOneIsDead` 0x7b258, `SetPlayerTheOne` 0x7b078, `SelectRandomTheOne` 0x7af28, The One part of `CTSGameRules::Think` | `scoring::{the_one_is_dead, set_player_the_one}`, `frame::{the_one_check, select_random_the_one}`, `teams::the_one_switch` | Every 10 s Think counts the non-dormant players other than The One (`+0x5876`, the regeneration rate; TS loops slots 0..maxClients-1, so the last slot never counts) and, with no One, `SelectRandomTheOne` picks a living non-observer. `SetPlayerTheOne` (living, not observing): health 150, kung fu, super jump, speed 330, PwUp ×2, a stored 3 s slow pause when nothing is held or active, award 10, "%s is The One!" (100,150,255), `the_one` log. The team switch is `SetDefaultPlayerTeam` + `ChangePlayerTeam(TeamID, 0, 0)` (keys, TeamInfo, ScoreInfo, TeamNames, no observer). The One's death: "The One is dead." (suicide or no player killer), or "The One was killed by %s!", the victim's team switch, `SetPlayerTheOne(killer)` and the killer's switch, before the killer's spree rewards. |
| `EntSelectSpawnPoint` 0x76128 / `TSSelectSpawnSpot` 0x76244 | `spawnpoint::{select, spot_score, place}` | spots whose `master` (`CBaseDMStart::KeyValue` 0xe2bb8 → `netname`, `IsTriggered` 0xe2c20) isn't triggered are skipped; up to 32 spots scored by enemy/teammate visibility and nearest player; telefrag (300, ALWAYSGIB) within 128 when the best score < 128; fires the spot's target. Unit tested. |
| `CTSGameRules::Think` 0x77204 | `frame::ts_think` | global slow expiry/apply, CheckDeads, auto power-ups (`nopowerup`/`slowmatch` compared with 1 by `fcomp`/`test ah,0x40`, so NaN counts as 1: `frame::x87_eq`), The One every 10 s, LMS restart/checks, CheckVotes, then `frame::srvsett_think`. `realbullet` (0x775b3) and `usecash` (0x77650): `fild` the stored int (`last_srvsett[0]` = `+0x578`, `[1]` = `+0x558`, -1 from `Init`) and compare it with the float cvar; unless equal, store `(int)cvar` (truncated) and broadcast `SrvSett 0` with the copy, or `SrvSett 1` with `copy != 0` (`frame::refresh_copy`). A fractional value never equals its truncation, so TS re-sends it on every Think (`usecash 0.5` sends `SrvSett 1 0` every frame); a NaN compares unordered, which TS takes as equal, so the copy keeps its value. The `realbullet` copy is what the weapons read (`state::realbullet`). Between the two, `CBulletManager::Think` (`weapons::bullet_think`, 0x77635). Last, `(int)ammocount` (truncated: `frame::ammocount`, 0x776de) against `+0x55c` (`last_srvsett[2]`, -1 from `Init`): on a change `Set_ammo_count` (0x7f2cc) and `SrvSett 2`. So a new `ammocount` reaches the max carry at the next Think (`state::ammo_column`, read by `weapons::max_carry`), and 1, 2, 3 pick the reduced columns while any other value (0, 4, 5, negative) is the default one; until the first Think of a map the default column from `W_Precache`'s `compile_ammo_info` stands. `InitHUD` sends the same truncated value. |
| `CHalfLifeMultiplay::Think` 0xcf038, `CHalfLifeTeamplay::Think` 0xe412c | `frame::{multiplay_think, teamplay_think, intermission_think}` | Voice update first (teamplay in intermission runs the Multiplay Think too, so twice). `mp_timelimit`; `mp_fraglimit`: DM every slot's frags (players who left included); teamplay `team_scores` as the last `RecountTeams` left them (kills, team changes, spawns), CTB the four `TSGameRules` teams' captures (unnamed teams still lower `mp_fragsleft`); differences rounded. `mp_fragsleft` on change; `mp_timeleft` when the last value differs from the cvar (one frame late, as TS compares `last_time` with `timeleft.value`); both `last_*` statics live for the process (two pairs). Intermission (`SVC_INTERMISSION`), `mp_chattime` kept in 10..120 (compared rounded); after it, once a player has pressed a button (`PlayerThink`) or 120 s after the intermission started, `ChangeLevel` every frame (the engines honour the first `changelevel` of a map). |
| `CHalfLifeMultiplay::ChangeLevel` 0xd0784, `ReloadMapCycleFile` 0xd032c, `ExtractCommandString` 0xd0698 | `mapcycle::change_level` | The SDK cycle: the file is reloaded when `mapcyclefile` changes (case-insensitively); lines are `map ["\key\value…"]` (`COM_Parse`: `//` comments, quotes, bytes from 0x80 are white space), maps the engine doesn't know are skipped. A persistent next-item pointer (for the process, so each frame's call advances it) and `minplayers`/`maxplayers` (0..maxClients) against `CountPlayers` (every slot with a player); the other rule keys become server commands after the `changelevel`. A chosen map vote other than `nextmap` wins and is used once; then `CTSGameRules::Init` (`GameRules::ts_init`, and `weapons::reset_bullets`) resets the TS state for the frames until the new map: default team names and models, the spawner, round, slow motion, `SrvSett` copies, vote records, weapon restriction and The One; an invalid map falls back to the current map, or `hldm1` with no cycle. `CHANGE LEVEL`, `PLAYER COUNT` and `RULES` console notes. |
| `PutPowerUp` 0x9797c, `GetRandomPosition` 0x974f4, `GetRandomPowerup` 0x97894 | `frame::{put_powerup, random_position}`, `GameRules::random_powerup` | 20 s after success, 1 s after failure; only on maps without their own `ts_powerup` (spawner byte). |
| `CheckDeads` 0x79c00, `RestartRound` 0x77920, `ResetRestartRound` | `frame::{check_deads, restart_round, reset_restart_round}` | Both loop `UTIL_PlayerByIndex` + `IsPlayer()` (`slot_players`): a player who left counts towards the total and, if it left alive, `RestartRound` puts it in observer mode too (see §6 for the maxspeed guard). LMS: last man +10 frags, "%d men standing!", "Round over", RoundTime (`(int)(end - time)`, not clamped, so an overrun round sends the low byte of a negative count, 0x79e89); restart: round time (`roundtime` clamped 30..180, NaN 30; the RoundTime byte is `(int)max(end - time, 0)` of the stored `f32` end, so it can be one short of the round time, 0x779b1), refunds (`frame::round_refund`: owned weapons' cost + attachments, and `price × rounds / per_buy` for ammo types above one purchase, into `GiveCash`; unit tested), forced observer, respawn in 3 s, then `RespawnEntities` 0x77764 (`entities::ts::respawn_entities`): broken breakables back (`breakable::respawn_all(false)`), in round modes 2/3 or LMS `Fall` on every `ts_groundweapon` (only that classname, not the HL remaps or ground ammo; a taken one shows at once), in LMS `Fall` on every `ts_powerup` (`nopowerup 1` hides them instead), every case home (`RespawnFlag`). |
| `SetGlobalSlowMotion` 0x7961c, `GlobalSlowMotion2` 0x79720 | `frame::{set_global_slow, global_slow2}` | every 0.5 s, every `UTIL_PlayerByIndex` + `IsPlayer()` player (`slot_players`, players who left included) and grenade/knife/WorldGun. |
| `PlayerAddPowerUp` 0x97bc8 | `powerup::add_powerup` | armour, kung fu, super jump (`mj`), health 150 + decay, grenade (`weapons::give_weapon(24)`), infinite clip (`weapons::multiply_clip`), stored types; then `items/gunpickup2.wav` with TS slow-motion pitch (0.5, the call at 0x97de1). |
| `UsePowerUp` 0x95980, `TSUse` 0x956e0 | `powerup::use_powerup`, +use double tap in `pre_think`, `usepowerup` command | PwUpTime, TSMBlur 1/2; refused while slow-mo is blocked. |
| `UpdatePowerUpState` 0x95748 | `powerup::update_state` | self 0.6 / others 0.35 (slow-mo), 0.8 / 0.225 (slow pause, bullet time); expiry; dead players drop held/active power-ups (DM/TDM). |
| `SlowMotionFlow` 0x96bb0 / `DeactivateSlowMotionFlow` 0x96a9c | `powerup::{slow_flow, deactivate_flow}` | every 0.5 s eye→eye traces to every `UTIL_PlayerByIndex` + `IsPlayer()` player (a player who left too: `engine.player(i)`, `has_player`); the stronger slow wins; the slowed slots are reset through the slot's object as TS does through its stored pointer; grenades/moving knives/WorldGuns in sight (max 32). |
| `GoSlow`/`InSlow` 0x929d0/0x9298c, `InterpolateSlowMotion` 0x9910c | `powerup::{go_slow_self, go_slow, in_slow, interpolate}` | fuser1 eases toward `Player::slow_target` at 7/s, snaps within 0.05 (unit tested). |
| `TakeArmor` 0x93da4 | `weapons::take_armor` | kevlar costs 10 of the shared free slots once; cap 100; speed from free slots; `-1` takes it off. |
| `ClearMultiDamage` 0xf0d0c / `AddMultiDamage` 0xf0d84 / `ApplyMultiDamage` 0xf0d38, `glast_headshot` | `damage::multi` | TraceAttacks add to one victim's sum (a new victim first applies the old one's, with the TraceAttack's attacker as inflictor and attacker; the type keeps every bit since the Clear); Apply calls the victim's `TakeDamage` once with the sum (TS's four arguments only: no hitgroup, direction, impact point or weapon is kept) and resets `glast_headshot`. `damage::apply` on a `traced` hit is Clear/TraceAttack/Apply; `ShootBulletsPlayer` traces all its pellets before one Apply. Unit tested (`damage::multi::tests`). |
| `CBasePlayer::TraceAttack` 0x901cc | `player::damage::trace_attack` | spawn protection, stunt ×0.7 (`iuser4 & 0x10c10`), dodge ×0.7 (`PlayerRules::dodge_until`), friendly fire (a teammate player, the victim itself included, without `mp_friendlyfire`), head ×2 / ×1.5 shotgun (`0x2000000`, sets `glast_headshot`), legs ×0.5, PTakeDam (0.3 s throttle, ×8 shotgun, +128 kevlar, truncated at 0x904b7), head/kevlar/body hit sounds (`EMIT_SOUND_DYN` slow-motion factor 0.5, `damage::HIT_SLOW`), pain sounds (factor 0.15, `damage::PAIN_SLOW`: the pitch only drops to `0.85 + 0.15 × fuser1`), corpse push, then `AddMultiDamage`. |
| `CBasePlayer::TakeDamage` 0x8fa94 + `CBaseMonster::TakeDamage` 0xb2430 | `player::damage::take_damage` | the bullet dance arming, `FPlayerCanTakeDamage` (a teammate other than the victim); kevlar `dmg -= dmg×min(armor_absorb,1)` on hitgroups 2–5 unless `ARMOR_EXEMPT`; armour never depleted; `last_damage` and the kevlar-scaled damage truncated to int (0x8fbd0, 0x8fc5f); refused with `takedamage` off (a direct hit still reaches the player then: `damage::apply` only skips other entities); `g_vecAttackDir` from an inflictor that isn't NULL or the world (`damage::update_attack_dir`, else the last value stays); `dmg_inflictor` (when there is one) and `dmg_take`; refused in god mode; health, death → `rules::on_killed`; no knockback (TS has none here); then, taken or not, each time-based type taken restarts its countdown (`TimeBasedDamage::on_damage`), the HLTV director event, and TSFade (150,10,10) for 1+. Unit tested. |
| `CBasePlayer::WaterMove` 0xd905c | `player::water::water_move` | skipped for noclip or health < 0. Head under (`waterlevel` 3): drown recovery stops; once `air_finished` (12 s) passes, `DMG_DROWN` from the world every second, 3, 4, then 5 per hit (`pev->dmg` starts at 2), added (truncated) to the drowned total (+0x9a8). Head out (also on dry land): `pl_wade1` if the air ran out, `pl_wade2` after more than 3 s under, air refilled, `dmg = 2`, and while drowned health is owed `DMG_DROWNRECOVER` is set and its countdown restarted. In water: `pl_swim1..4` (1/32 per frame, likelier with less air; `CHAN_BODY` 0.8), lava `10 × waterlevel` `DMG_BURN` and slime `4 × waterlevel` `DMG_ACID` every frame (SDK `dmgtime` quirks), `FL_INWATER` kept. Real time, no slow-motion scaling; sounds use the time-scaled `EMIT_SOUND_DYN` pitch (k 0.5). Unit tested. |
| `CBasePlayer::CheckTimeBasedDamage` 0xda00c | `player::water::check_time_based_damage`, `TimeBasedDamage::tick` | every 2 whole seconds (truncated `abs(time - m_tbdPrev)`), for each of the 8 time-based bits: poison deals 2 (`DMG_GENERIC`, self-inflicted) for 6 ticks (start + 5); drown recovery gives back `min(owed, 10)` through TS `TakeHealth` (`player::damage::take_health`, shared with every entity heal; no cap, refused above max health, starts the decay when it overheals), 4 ticks but restarted every frame the head is out; the other types only count down 2 ticks. Antidotes (+0x834, from `item_antidote`) cure nerve gas/poison. Unit tested. |
| `CBasePlayer::UpdateClientData` 0x91d00 | `player::hud::update_client_data` | From PreThink between `WaterMove` and `CheckTimeBasedDamage` (not in intermission), and inline from `ForceClientDllUpdate` and `RemoveAllItems`. Runs when `+0x534 <= time` and not negative (`TSPlayerDisconnect` sets -1), then waits 0.1 s; the InitHUD block is inside the throttle. On change: HideWeapon (`drive::update_client_hide_weapon`), TSHealth (`(int)max(health, 0)`), TSArmor (`(int)armorvalue`; both truncated, low byte, no cap; the stored `(int)` value is compared with the float, so a fractional health or armour is resent every update), Train (`drive::update_client_train`), TSState, TSCash, TSSpace (free slots, `+0x548`), PwUp (seconds `max(0)`, low byte), TSSlowMo, TSBTime, ActItems (ALL, only while alive; reset to 0, so 0 isn't resent), TSPAward (ALL), then the gun's `UpdateClientData` (`weapons::update_client_data`). Unit tested. |
| `Host_Say` 0xad02c | `commands::host_say` / `format_say` | `\x02name: `, `(TEAM)`, `%a`/`%h`, 0.5 s flood gate, LMS dead chat rule, `timeleft` (reply to the sender only; like a handled vote the line is not broadcast: both exits of the branch reach 0xad4dc, which clears the broadcast flag), log. Unit tested. |
| `PlayerVote` 0x7a1c8, `CheckVotes` 0x7a5c4, `CalcMiniminVotants` 0x7a1b0 | `votes::{player_vote, check_votes, min_votes}`, `Votes` in `GameRules` | `vote map <name>` (current/invalid map refused, "%s vote for map %s" to all), `vote stats` (last tally, "(n/need) votes for map"), `vote reset`; handled votes aren't broadcast. Every 10 s every player is counted and votes are grouped by map; the leader passes at `max(1, players/2)`. A passed vote sends `multiplay_think` to intermission and `change_level` loads the voted map. A joiner's vote is cleared in TS InitHUD. Unit tested. |
| `cmd_globalslow` 0xc5580, `cmd_restartround` 0xc5748, `cmd_weaponrestriction` 0xc58a0 | `commands::cmd_*` | clamps unit tested. |
| `CVoiceGameMgr::Init` 0xf7438 (`VoiceMask`/`ReqState` registered first, as the `CHalfLifeMultiplay` constructor does before `LinkUserMessages`), `Update` 0xf750c, `ClientConnected` 0xf7548 (via `CHalfLifeMultiplay::ClientConnected` 0xcf59c), `ClientCommand` 0xf770c (via `CHalfLifeMultiplay::ClientCommand` 0xceee8), `UpdateMasks` 0xf79bc | `voice::{register_messages, precache, update, client_connected, client_command, update_masks}` | Every 0.3 s (from the start of the Multiplay `Think`, intermission included) and after each `vban`/`VModEnable`: `ReqState` to clients that haven't answered `VModEnable`; for clients with voice enabled, the audible mask = players the rules allow (`sv_alltalk` ignores the rules); `VoiceMask` (`L audible, L banned`) when either mask changed; then `Voice_SetClientListening(client, other, audible && !banned)` for every pair. Every slot with a player counts, players who left included; the sender of `vban`/`VModEnable` is mid-dispatch, so its respawn time is passed in. Steam HLDS starts every stream closed, so without this nobody hears anybody there. `vban <hex>` takes one mask (32 players); `VModEnable <n>`. |
| `CMultiplayGameMgrHelper::CanPlayerHearPlayer` 0xd0ed0 | `voice::can_player_hear_player` | teamplay: teammates only; round modes 2/3 or LMS: a talker who is dead for the rest of the round (`respawn_at` −1) is heard only by listeners in the same state. |
| `AddToFullPack` 0xaf180 | `net::add_to_full_pack` | `iuser4` for all entities, `playerclass` for non-players, glow shell on the viewer's own `WorldGun`/`knife` (`weapons::glow_for_host`: the private dropper +0xd0 / thrower +0x8c, not `pev->owner`), `FL_SPECTATOR` culling. Entities are skipped for `modelindex == 0` only: TS's other model test (`pStringBase + pev->model == NULL`) can't fail, so a model index with an empty `model` string is still sent. Players' `spectator` is 0/1 where TS stores `flags & FL_SPECTATOR` (0x4000000); delta.lst gives it 1 bit and every engine clamps an over-range unsigned delta integer to its maximum (FWGS `Delta_ClampIntegerField`, HLDS/ReHLDS `MSG_WriteBits`), so both put 1 on the wire. Neither client reads it beyond copying it in `HUD_ProcessPlayerState` (pmove's `spectator` comes from the engine's HLTV flag). Hidden only when `effects == EF_NODRAW` exactly (other bits with it still send). A host with `groupinfo` sets the group trace and skips entities whose `groupinfo` doesn't share a bit (TS returns without `UTIL_UnsetGroupTrace` there, and so does the port). EFLAG_SLERP for an animating (`animtime != 0`) non-player with zero velocity (the older SDK test, not `FL_FLY`). `impacttime`/`starttime` copied. Render values truncate like TS's x87 `fistp` (the decomp's `ROUND`), colour through a short so out-of-range channels wrap. Players send `weaponmodel = pfnModelIndex(pev->weaponmodel)` (the p_ model the client's `StudioDrawPlayer` merges onto the player and in-eye spectating shows as the view model). |
| `CmdStart` 0xb067c / `CmdEnd` 0xb06f4, `UTIL_SetGroupTrace` 0xebecc / `UTIL_UnsetGroupTrace` 0xebf0c | `net::{cmd_start, cmd_end, set_group_trace, unset_group_trace}` | a player with `groupinfo` runs its usercmd with that group mask (`pfnSetGroupMask`, op AND). Nothing in TS sets `groupinfo` (it isn't a keyvalue), so this is dormant with TS content. |

### 2.1 `deathmatch 0`: `CHalfLifeRules`

`InstallGameRules` builds `CHalfLifeRules` (0xde6fc) when `deathmatch` is 0 (`state::single_player`):
`g_teamplay` 0 and no `DecideGamePlay`, so LMS and The One keep the last map's values; no
`CTSGameRules::Init`, so the TS state carries over from the last map (zero on a fresh server); no
`exec servercfgfile`. Its hooks are empty or the `CGameRules` defaults, so the port skips: the rules
`Think` (and with it `CTSGameRules::Think` and `CBulletManager::Think`), the voice manager
(`ClientConnected`, `Update`, `vban`/`VModEnable`) and `menuselect` (they are "Unknown command"),
the multiplayer `InitHUD` and `game_playerjoin` (`CTSGameRules::InitHUD` still runs),
`ClientDisconnected`, `ClientUserInfoChanged`, `PlayerSpawn` (no suit, no `game_player_equip`, no
gun, no loadout), the spawn spot's target, `PlayerKilled`/`DeathNotice` (no scoring at all), and
`motd`. Items don't come back (`ItemShouldRespawn` 6), empty chargers stay empty (recharge time 0),
`trigger_hurt` has no per-player mask (`!IsMultiplayer()`), `UTIL_BloodDrips` isn't doubled, and
`respawn` (0xace58) sends `reload` instead of spawning (neither coop nor deathmatch).
`GetGameDescription` says "The Specialists".

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
and the port's `ts_bot_quota 0`, `ts_bot_stop 0` and `ts_pm_arith msvc` (movement arithmetic,
`movement.md`). The three bot cvars are read as TS reads
them: `botlearn` (> 0 learns nodes, 2 adds debug output), `bottalk` (bot chat), `botarmor` (the
buy module's kevlar), §10.

`g_flWeaponCheat` (`cvars::cheats`) is `sv_cheats` as `CWorld::Spawn` (0xf4450) found it, so a new
value counts from the next map: the `fov`/`powerup`/`cashala` commands, `CheatImpulseCommands` and
the InitHUD cheat notice, `damagemult` and `dmgreport` (`weapons::damage_adjust`,
`weapons::dmgreport`). The port's dev aids (`ts_tp`, `ts_face`, `ts_dev_touch`, `ts_give`,
`ts_weapons_debug`; `cvars::dev_cheats`) read `sv_cheats` live.

`customweapons` is registered, as in TS, and nothing reads it (one reference in `ts_i386.so`, the
`CVarRegister` at 0xc5d97): weapons files load with `loadweapons` only.

`CTSGameRules::Init` (every map) clears the custom-weapons flag (`+0x5884`) and sets
`armor_absorb` 0.2125 and `health_absorb` 0.1 (`+0x588c`/`+0x5888`; `state::GameRules::{custom_weapons,
armor_absorb, health_absorb}`). Kevlar reads `armor_absorb` (`player::damage::after_kevlar`);
nothing reads `health_absorb`. A `loadweapons` file sets them and the flag; its weapon values
live in the weapon tables and, like TS's `global_weapon_info`, outlive the map, but from the next
map on joiners no longer get the "custom weapons" notice and `CustomWP` in `InitHUD`.
`deathmatch 0` runs no `Init`, so the values carry over (0 on a fresh server, as TS's `.bss`).

TS never registers `initial_cash`, `cashforwin`, `cashforlose`, `respawnnumber`, `dmgreport`;
they are constants (`cvars::INITIAL_CASH` 1200 etc., respawnnumber 1). Nothing registers
`dmgreport` in the port either: `weapons::dmgreport` reads it by name (0 when missing), as TS's
`CVarGetFloat` does, so its cheat output only works where the engine lets the console create it.

Server commands: `globalslow`, `restartround`, `weaponrestriction`, `addbot [model]` /
`addcustombot [name] [model] [skill]` (as TS's Windows `mp.dll`, `0x10026110`/`0x100261a0`: on a
dedicated server TS only prints "Bots only in listen servers.", which is all the Linux
`ts_i386.so` has; the port adds the bot there too, a chosen deviation below. `help` prints the
usage and anything else adds a bot:
`bots::add_bot`/`add_custom_bot`, TS's `BotCreate`/`BotCreateCustom` with TS's AI (§10): the
`addbot` argument is the model, the name comes from `bots.txt`, "<name> connected with skill <f>",
and `botlearn 1` when the node graph is empty; the quota neither counts nor kicks them. TS's
shipped `listenserver.cfg` runs four `addbot`s on every map, so "Create Server" starts with four
bots, as in TS),
`loadweapons`/`saveweapons` (the weapons package, `weapons::custom`), and test helpers `ts_status`, `ts_hurt <#> <dmg> [hitgroup] [attacker#]`,
`ts_givepowerup <#> <bits>`.

Client command names are matched case-sensitively, as typed (`strcmp`, and `strstr(..) ==
argv(0)` for the `weapon_`/`lasclr_`/`drop_`/`cutwire_` prefixes, `ClientCommand` 0xad838), so
`Say` is an unknown command; only the voice manager's `vban`/`VModEnable` use `strcasecmp`. The
voice manager sees every command that gets past `ClientCommand`'s own names, then `menuselect`;
anything else gets "Unknown command". A client without private data is ignored.

Client commands handled here: `say`, `say_team`, `respawn`, `usepowerup`, `powerup <n>` (cheat),
`cashala` (cheat), `fov`, `use <classname>` (`SelectItem` 0xdade4, `weapons::select_item`: switches
between `weapon_tsgun` and a picked-up `cycler_weapon`; with only the gun it changes nothing, since the gun is
then the active item), `changeteam` (VGUIMenu 2, in every mode: TS has no teamplay check, 0xae49c; the client only opens the team menu when `GameMode` said teamplay, `ShowVGUIMenu` 0x10064050 → 0x10064280, as the port's client does), `jointeam <n>`, `motd`, `fullupdate`,
`lasclr_<n>` (PlInfo), `menuselect`, `vban`, `VModEnable` (voice manager, §2), `cutwire_<n>` (no-op).

## 4. Map entities (`entities/ts`)

| Class | Status |
|---|---|
| `ts_powerup`, `item_longjump` | Full: keys `pwuptype`/`pwupduration`/`pwupmult`, random set from spawnflags, bodies/durations (a stored type's `base × pwupmult` truncated, as TS's `(int)` cast), fall → ground (bbox ±16×64), pickup rules (single slot, +use swap, instant set 0x1e6), message/target, respawn clamps (<1 → 60, ≤10 → 10, ≤180), hidden with `nopowerup 1`. Temporary ones (auto spawner, drops) live 60 s and fade over the last 30 s. A map power-up disables the automatic spawner; `item_longjump` re-enables it (TS). |
| `ts_groundweapon` + `weapon_crossbow/shotgun/gauss/egon/rpg/9MMAR/9mmAR/hornetgun/satchel/snark/handgrenade`, `ammo_ARgrenades` | Full: random ids per HL class (`Random<Kind>`'s `RANDOM_LONG(1, n)` order; the one-weapon classes draw nothing), `w_` model, attachments from spawnflags, flag 16 = on wall (`PutOnWall`), pickup with **+use** (`weapons::give_weapon`) of any visible weapon, falling or landed (`GetWeapon` 0xa2328 checks only `EF_NODRAW`); the shotgun, rifle, SMG, big-pistol and knife classes (`weapon_shotgun/gauss/egon/rpg/9MMAR/satchel`, `ammo_ARgrenades`) pick a new id once taken and set the model again on every use (`CTSShotgunWeapon::GetWeapon` 0xc95ac and the others); `weapon_hornetgun` (`CTSMGWeapon::Spawn` 0xc9dac) always has `wextraclip` 100; a taken item keeps its `solid` (only its touch goes), `wextraclip` (clamped to ≥ 0 for good, then that many rounds through `AddAmmo` when the reserve for that ammo type is empty after the pickup, with the time-scaled `9mmclip1` sound, as in `GetWeapon` 0xa2328), respawn (<1 → 60, 1 → 1, ≤180), message/target. `buyable`/`attackerteam` (`GetWeapon` 0xa2328) are ported behind `state::cash_based()`, which is false as in TS: the team check (round modes, `attackerteam` ≠ -1), the price (`ground::buy_price`: cost + spawnflag attachments + ammo price × `wextraclip`; unit tested), "You don't have enough money…", and the charge after the pickup. The whole spawnflags go to `PickUpWeapon` as the attachments. `Spawn` (0xa1b40) turns an akimbo pair (8, 10, 16, 30, 37) into its single and gives an id without a valid `get_weapon_info` entry (0, 29, outside 0..38) `SUB_Remove` 0.1 s later. `Use` (0xa230c) calls `GetWeapon` with any activator, which goes on for any `CBasePlayer` (`IsPlayer`, `rules::has_player`); the re-picking HL classes do nothing while hidden and otherwise set their model even for a non-player. `WaitTouch` (0xa26cc) gives only a briefcase, which `Spawn` never keeps. `PutOnWall` (0xa1cd8) skips surfaces that take damage or are hidden, and keeps the closest distance as an `(int)` cast (truncated). No precache in `Spawn` (`TSPrecache` has every `w_` model). |
| `ts_groundammo` + `ammo_9mmAR/buckshot/crossbow/gaussclip/rpgclip` | Full: `mags<t>` = `strtol` × per-buy rounds (left unhandled, as `KeyValue` 0xa2718 doesn't set `fHandled`); the HL classes put half the max carry of fixed ammo types after `Spawn` (`ground::hl_ammo_types`: crossbow .50BMG + 7.62x51, buckshot shells, gaussclip 7.62x39 + 5.56, rpgclip 9mm/.45/5.7/.32, 9mmAR .50AE + .454) in the `Set_ammo_count` column then in force (`state::ammo_column`: the default one for map entities, which spawn before the first gamerules `Think`). Picked up on touch (`WaitTouch` 0xa2f40: any `CBasePlayer`, alive, with a gun; every non-zero slot through `weapons::give_ammo`; slow-motion `9mmclip1`). |
| `ts_model` | Full. |
| `ts_mapglobals` | Full: weapon mask `0x1f ^ spawnflags` on the whole int (`get_weapon_restriction` 0x7263c; SrvSett 3 carries the higher bits too), round timer flag, super jump map flag. |
| `ts_wind` | Full: `TSPSys` type 3. |
| `ts_fog`, `ts_emitter`, `ts_particle` | The client reads them from the BSP lump. `ts_fog` and `ts_emitter` are removed (`CTSEmitter::CheckValues` 0x740ac logs an `at_error` when either emits-per-second bound is ≤ 0 or either particles-per-emit bound < 1). A `ts_particle` (`Spawn` 0x74134) takes one of the shared emitter's 512 slots and stays; only when they are all used is it removed ("Too many particles!"). `soundonemit` (any case, `strcasecmp`) is precached either way. |
| `ts_trigger`, `ts_dmhill`, `ts_timer`, `ts_customtarget`, `ts_playeramount`, `ts_slowmotion`, `ts_slowmotionpoint` | Full per §5 (TS quirks kept: `ts_trigger` team defaults to index 0 and always prints its message; `ts_timer` flag 2 ignored). `ts_customtarget`'s `targets` skip empty fields (`strtok`). Number keys are read as TS's `strtol`/`strtod` do (the leading number). Float-to-int conversions truncate, as TS's GCC `(int)` casts do (`ts_slowmotion`'s radius, `ts_dmhill`'s message hold, `ts_trigger`'s `attackerteam`/`allowedteam` from `strtod`, `ts_wind`'s yaw). `ts_slowmotionpoint` (`WaitThink` 0x71728) tests each player's line of sight with `ignore_glass`, so glass doesn't block it. |
| `ts_briefcase`, `ts_flag`, `ts_scoregiver`, `ts_wingiver` | CTB: the case's `s_flag_status` (`ctb::FlagStatus`: contents, id, team, last holder, home) lives on the case entity or, while carried, on the player (`PlayerRules::flag`); taking destroys the entity (`Destroy` 0x9de24: out of the slowed lists, `UTIL_Remove`) and a drop creates a new one (`CreateFlag` 0x9d874, spawned at the world origin, then `pev->origin` set without relinking). The zone's think is `FlagFall` or `WaitThink` (every 1 s), scheduled exactly. Case falls at home 1 s after spawn (`FlagFall` 0x8da00, at the centre of the zone's `mins`/`maxs`, TSFlag 0); a flying case (`Fall` 0x9df10 every 0.1 s: landing sound, impact damage `|v|·30·0.001` through TraceAttack on non-players with health ≤ `|v|·30`; `BounceTouch` 0x9dbbc) is taken only with +use, a landed one (32×32×48 trigger) also by touch. Taking (`TakeFlag` 0x93830): 20 free slots and speed, `items/gunpickup2.wav`, TSFlag 1, "%s picked up a briefcase containing %s!" unless the taker was the case's last holder, `aquired_briefcase` log with contents; a case taken from home closes its zone for captures until it is back. Thrown (`DropFlag` 0x9334c) on death/observer/disconnect, or with `drop` and no gun in hand (`DropWeapon` 0x94a87): from eye + forward·16, along `v_angle` at (90 − pitch)·2 while alive, random (pitch ±60, yaw ±180, speed 60..160) when dead, plus 0.9 × player velocity and speed·0.33 up, passing through the thrower until it lands; `dropped_briefcase` log, TSFlag 2. 20 s return (`RespawnFlag` 0x9de58: "<owning team> briefcase returned", none for a case any team may take; new case at home 1 s later), owner return +5 (only with 2+ players, "returned the briefcase! 5 #TS_Frg"; new case at home 1 s later), capture +10 and team score (`captured_briefcase` log), TSHVoice/TeamScore; owner return and capture send no TSFlag 0 until the case reappears. Flag logs repeat the user id in the team slot (`"name<uid><auth><uid>"`), as TS does. Score/win givers: team score, "%s Score!/Win!", TSHVoice, TeamScore (scoregiver flag 1 restarts the round). |
| `deadcorpse` | `TSCreateDeadCorpse` (0x70234), from the death think, for a player not `EF_NODRAW`: `CBaseEntity::Create` at the player, then their model/modelindex, frame, sequence, animtime, colormap, deadflag and velocity, `MOVETYPE_TOSS`, `kRenderTransColor` at 255 with `kRenderFxDeadPlayer` and their index in `rendercolor.x`. `WaitThink` (0x70460) runs every 0.5 s, fading `renderamt` over the last two thirds of the 10 s (at least 1 s), and removes it after. |
| `info_player_team1..4` | `CBaseDMStart` points like `info_player_deathmatch` (`master` key; team spawns are found by classname). TS doesn't export `info_player_teamspawn`, so the engine frees it ("No spawn function"), as with TS. |
| `ts_objective_manager`, `ts_objective_ptr`, `ts_bomb`, `ts_hack`, `ts_teamescape`, `ts_sellzone` | Stubs: hidden, non-solid (dead in 3.0). |

### 4.1 Save data

The TS classes save TS's tables (`save_fields` in `entities/ts/*`, `weapons/worldgun.rs`,
`weapons/projectiles.rs`): `CTSPowerUp` its type and `pwupduration`, `CTSGroundWeapon` id,
`wduration`, `wextraclip`, `CTSDMHill` `fragamt` / `fragamtteam` / `nextcheck`, `CTSSlowMotion`
`duration` / `slowamt`, `CTSWinGiver` cash, team and call count, `CTSObjectiveManager` its messages
and targets, `CTSObjective` / `CTSBomb` / `CTSHack` / `CTSTeamEscape` theirs (always empty in TS:
`CTSMapGlobals`, `CTSPlayerAmount`'s `CBaseDelay` part, `CTSTrigger`'s inherited tables). The
objective classes the port doesn't otherwise implement (`ts_objective_manager`, `ts_bomb`,
`ts_hack`, `ts_teamescape`, `ts_sellzone`) got their own types holding the saved keys; their
`Use` / think / touch behaviour is still unported (§7). Power-ups, the CTB zone and case, world
guns, ammo packs, grenades, knives, shrapnel, wind, the slow-motion entities, a running timer,
`ts_trigger`, an enabled `ts_dmhill` and the objectives set a member pointer, so (as in TS) their
restore stops after `"BASE"` and they come back inert. A power-up's type is a `u16` in the port, so
a restored type above 0xffff is cut.

The player (`player/save.rs`) saves `CBasePlayer::m_playerSaveData` from the subsystems that hold
each field (buttons and fall speed from `stunt`, `m_rgItems` antidotes and security cards, the
physics flags `ONTRAIN` / `DUCKING` / `USING`, items and the active / last item from `weapons`,
drowning from `tbd`, train / tank / hide-HUD from `drive`, the sound volumes from `impulse`,
`m_fInitHUD`, the HEV suit queue from `suit`), the `CBaseAnimating` part from the animation state (`ResetSequenceInfo` now keeps
`m_flLastEventCheck`), and `m_pfnThink` set after a death that didn't gib (`death_think`). Fields
TS keeps constant are written as such (`m_bitsHUDDamage` -1, the rest 0). `Restore`'s tail
(spawn spot without a landmark, view angles, hull, `hl` / `slj`, `m_flNextAttack`) and `Precache`
(`m_iTrain = TRAIN_NEW`, the armour cache, `m_bitsDamageType`, `gInitHUD` → `m_fInitHUD`) follow.

## 5. Messages sent (payloads in `shared::protocol`)

| Message | Dest | Payload | Sent from |
|---|---|---|---|
| ResetHUD | ONE | `B 0` | spawn / observer start (next think) |
| InitHUD | ONE | – | first spawn |
| GameMode | ONE | `B teamplay` | InitHUD |
| ScoreInfo | ONE/ALL | `B idx, S frags, S deaths, S 0, S team+1` (frags: `(int)` cast, low 16 bits: `lifecycle::score_frags`) | InitHUD, kills, team change, dmhill, CTB, LMS |
| ServerName, MOTD | ONE | `Str` / `B last, Str ≤60` | InitHUD, `motd` |
| TeamNames | ONE/ALL | `B n, n×Str` | teamplay InitHUD, team change |
| TeamInfo | ONE/ALL | `B idx, Str team` | teamplay, observer start, The One |
| TeamScore | ALL | `Str team, S score, S 0` | captures, wins |
| DeathMsg | ALL | `B killer, B victim, Str weapon` | PlayerKilled |
| SayText | ONE/ALL | `B sender, Str` | chat, join/leave, name change |
| TextMsg | ONE/ALL | `B dest, Str` | join notice, timeleft, TK kick |
| TSMessage | ONE/ALL | `B r,g,b, B effect, B hold, Str ≤127` | all TS HUD text |
| TSHealth / TSArmor / TSState / TSSpace | ONE | `B` | hud::update_client_data, Killed |
| TSCash | ONE | `S` | hud::update_client_data, StartObserver |
| PwUp | ONE | `S bits, B secs` | hud::update_client_data (held slot), permanent kung fu/super jump as `(4,0)`/`(0x100,0)` |
| PwUpTime | ONE | `B secs` | UsePowerUp |
| TSSlowMo | ONE | `Co factor×100` | hud::update_client_data |
| TSBTime | ONE | `B` | hud::update_client_data |
| TSMBlur | PVS | `B ent, B mode` | UsePowerUp, Killed |
| TSPAward | ALL | `B ent, B award` | hud::update_client_data |
| PTakeDam | PVS | `Co pos×3, Co dir×3, B amount` | damage |
| TSFade | ONE | `S dur, S hold, S flags, B r,g,b,a` | spawn protection, damage |
| Spectator | ALL/ONE | `B ent, B observer` | observer start/stop, TS InitHUD, spawn |
| ActItems | ALL | `B ent, B mask` | hud::update_client_data (alive, on change), 0 from Killed and StartObserver |
| SetFOV | ONE | `B 0` | Killed, StartObserver |
| SrvSett | ONE/ALL | `B key, S value` | InitHUD, cvar changes, weaponrestriction |
| RoundTime | ONE/ALL | `B secs, B state` | LMS rounds, InitHUD |
| TSFlag | ALL | `B state, B id, B team+1, B holder` | CTB |
| TSHVoice | ONE | `B lost` | captures, wins |
| TSPSys | ALL | `B 3, L seed, L yaw, Co strength` | ts_wind |
| VGUIMenu | ONE | `B 2` | changeteam (any mode) |
| PlInfo | ALL | `B ent, B 0, B colour` | lasclr_N |
| VoiceMask | ONE | `L audible, L banned` | voice `UpdateMasks` on change |
| ReqState | ONE | – | voice `UpdateMasks` until the client answers `VModEnable` |
| engine msg 50 (`SVC_HLTV`) | SPEC | `B 3 (DRC_EVENT), S primary, S secondary, L flags` | `DeathNotice` (victim, inflictor or killer, `0x27`), player `TakeDamage` (self, inflictor, `5`; after the damage, also when `takedamage` is off). TS's old director layout (no length byte, not the SDK's `svc_director` 51); only HLTV proxies receive `MSG_SPEC`. `msg::director_event`, payload `hud_wire::DirectorEvent`. |

## 6. Deviations from TS 3.0

Port additions:
* `ts_bot_quota` (TS's bots on dedicated servers too) and `ts_bot_stop`: §10.1. On a full server
  the quota retries quietly every 0.5 s.
* `ts_pm_arith` (`msvc` default, or `gcc`): the server's movement and shared weapon arithmetic, `mp.dll`'s or
  `ts_i386.so`'s (`cvars::pm_arith`, `movement.md`).
* `ClientDisconnect` clears `FL_CLIENT`/`FL_FAKECLIENT` (engine safety, still needed: FWGS's
  `pfnSetClientMaxspeed` (`sv_game.c`) looks the client up with `SV_ClientFromEdict(e, false)`,
  which still returns a dropped client's slot, and then writes `cl->edict->v.maxspeed`, but
  `SV_DropClient` (`sv_client.c`) has set `cl->edict = NULL`; the port's `players()`/`is_player()`
  liveness test is the flag). The flag is only the port's own liveness test (the bots, the dev
  commands, `rules::set_client_maxspeed`); no TS test reads it. TS keeps `FL_CLIENT`, and its
  `IsPlayer()`/`IsNetClient()` virtuals, `Classify() == CLASS_PLAYER` and classname `"player"`
  compares are true for any `CBasePlayer`, so the port tests those with `rules::has_player`
  (`Player` attached, classname `player`), its `FL_CLIENT` tests with `rules::ts_flags` (the
  flags with `FL_CLIENT` put back on a player) and `rules::entities_in_box` (`UTIL_EntitiesInBox`
  0xec1bc: doors, breakables, `env_fade`, the Xen tree), and its `UTIL_PlayerByIndex` loops with
  `slot_players`. A player who left is therefore still a player everywhere TS sees one, until
  the slot's next client spawns.

Where TS reads garbage or crashes on input it never gets in practice:
* `pfnSetClientMaxspeed` on a player who left (`CBasePlayer::SetPlayerSpeed` 0x93068 and
  `Observer_SetMode`'s roaming mode 0x99e44): FWGS writes through the dropped slot's NULL
  `cl->edict` and crashes; the Steam engines only set `pev->maxspeed`. TS reaches it: LMS
  `RestartRound` (0x77920) puts every `UTIL_PlayerByIndex` + `IsPlayer()` player with `iuser1` 0
  in observer mode, a player who left alive included, and `Observer_SetMode` falls back to
  roaming when no target is found. A delayed trigger chain whose activator left can also reach
  `SetPlayerSpeed` through a `Use` that takes any `CBasePlayer` (`CRecharge::Use` →
  `TakeArmor`, `game_player_equip` → `PickUpWeapon`). The port
  (`rules::set_client_maxspeed`) skips the engine call for an edict without `FL_CLIENT` and sets
  `pev->maxspeed` itself, as the Steam engines do.
* `m_szTeamName` is filled with `strncpy(.., 16)`; a team name of 16 bytes or more (a `team` or
  `model` key without a team limit) stays unterminated in TS, whose reads then run on into the
  animation extension at `+0xb58`. The port keeps the first 16 bytes.
* `PlayerIsCheating` with a `cl_lw` that `sscanf("%i")` can't read compares an uninitialised
  stack word with 1; the port treats it as not 1 (cheating, so `cl_lw 1` is set).
* `TeamWithFewestPlayers` with no teams gives `""` (TS copies from NULL; it can't happen, because
  the player being placed is always counted).
* `CreateFlag` failing (no free edict): TS dereferences NULL in `DropFlag`/`FlagFall`; the port
  skips the case. A `ts_briefcase` without a model isn't given `SetModel("")`.
* `Killed`'s killer TSMBlur with a NULL attacker: TS reads through it; the port skips the blur.
* `ResetSequenceInfo` on a negative sequence: TS reads before the sequence table; the port gives
  frame rate 0 (as for a sequence past the end), so such a death animation never ends, like any
  sequence with frame rate 0 in TS.
* `PlayerKilled`'s weapon name for a non-player attacker without an inflictor: TS reads the
  classname through NULL; the port says "world".
* A weapons-file line longer than 255 bytes: `adjust_weapon_value` `strcpy`s it into a 256-byte
  buffer; the port reads the first 255.
* The HEV suit queue (`player::suit`) indexes `m_rgSuitPlayList` with `m_iSuitPlayNext`, the
  no-repeat table with `RandomLong(0, 31)`'s result and `gszallsentencenames` with a queued
  sentence, all unchecked; only a corrupt save (or a `sentences.txt` changed under a restore) puts
  them out of range. The port skips the write, searches the play list from `m_iSuitPlayNext`
  modulo 4, and plays nothing for a sentence past the table. `SENTENCEG_PlayRndI` with a group
  index past the loaded groups plays nothing.

Port structure:
* `PlayerSpawn` with a `game_player_equip`: as in TS no `weapon_tsgun` (no gun at all, kung fu
  included, unless the equip lists it: `weapons::has_gun`, weapons.md §2) and no `BuyArmament`.
  The equip touches run inside the spawn, as in TS, with the player lent to its slot.
* `game_player_team` changes the activator's team inside the `Use`, as TS does, when the
  activator's own call fired it: `player_use` (+use), `game_playerdie` (`PlayerKilled`) and
  `game_playerjoin`/`game_playerspawn` (`UpdateClientData`'s InitHUD block) lend the player back to
  its slot for the targets (`world::lend`), and `world::run_now_or_defer` reaches it. (TS items
  fire their `target` with themselves as activator, so `game_player_team` ignores them.) A
  player's own weapon frame does the same: each shot and melee swing (and the stunt hook's delayed
  kung-fu hit) lends the player, gun and inventory put back into it, around the hit
  (`weapons::host::ServerHost::reach_player`), so shooting a `func_breakable` or button that
  targets `game_player_team` changes the team mid-shot, as in TS. A change with flag 2 kills the
  shooter there, holstering and emptying the gun that is firing; the attack then carries on with
  that gun, as TS's carries on with its changed `CTSGun`.
* The same lend lets every other call TS makes on a busy player happen at once: `game_player_hurt`
  (damage before its targets fire, healing), `game_score`'s `ScoreInfo`, `player_weaponstrip`'s
  `RemoveAllItems(FALSE)`, the HEV charger's `TakeArmor`, the antidote count, `func_tank`'s
  `StartControl`/`StopControl` player half (`drive` lends the player around its tank `Use`s), and
  `PlayerKilled`'s scoring of a killer whose own shot, swing or +use killed (`world::run_now` on the
  lent killer, `scoring::player_killed`). Only a player held by a dispatch that didn't lend it gets
  the work once that dispatch returns (`world::run_now_or_defer`, same frame).
* `deathmatch 0` (`CHalfLifeRules`, §2.1): `CBulletManager::Reset` still runs on every map from
  the weapons' precache (TS resets only from `CTSGameRules::Init`; no bullet moves in single
  player either way).

Not deviations, for the record: `PlayerSpawn`'s stored-cash copy without an equip
(`pl+0x53c = pl+0x540`, 0x8912d) isn't modelled because nothing reads `pl+0x53c`: the only
other instruction that addresses it is `CTSGameRules`' reset to 0 (0x794df); round modes 2/3 and objectives can't be reached in TS 3.0
(`InitObjectives` 0x785d8 only keeps a mode that is already 2/3), `CashBased()` is hard-wired off,
and `CheckVotes`' slow-match / teamplay / friendly-fire tallies can't pass (nothing sets those
votes and the minimum is `max(1, n/2)`), so only map votes are live. `+use` on TS items goes
through the one `PlayerUse` (`player::player_use`, see stock-entities.md §3.7): ground weapons and
ammo, dropped guns, knives, grenades, the CTB case and the hidden objectives all compete as
`FCAP_CONTINUOUS_USE` users, as in TS.

## 7. Missing / partial

* Observer code (audited against `ts_i386.so` for issue #7; the `Observer_*` functions,
  `StartObserver`/`StopObserver`, the PreThink observer block and `PlayerDeathThink` match;
  `jointeam`/`ChangePlayerTeam`, `fullupdate` and `CallCheckDeads` were aligned for issue #81;
  `changeteam`, the `ChangePlayerTeam` `model` key and its kill path for issue #84;
  `SetDefaultPlayerTeam`, `RecountTeams`' team list and limit, `IsValidTeam` and the `ScoreInfo`
  team index for issue #86; `ClientUserInfoChanged`, The One's team switch, `game_player_team`,
  disconnects and the deviations list for issue #87).
* Objective classes (`ts_objective_manager`, `ts_bomb`, `ts_hack`, `ts_teamescape`,
  `ts_sellzone`, `ts_objective_ptr`): only their saved fields and spawn flags are ported (§4.1).
* Save/restore is unit-tested only (byte layout, every class against TS's tables, round trips);
  no save from TS's server has been loaded in-engine yet.
* Untested in-engine: CTB (`ctb_deadheat` ships with two briefcases), `ts_dmhill`, `ts_slowmotion`,
  `ts_wind`, ground ammo pickup (weapons stubs give nothing), chat from a real client,
  `item_antidote` (no shipped map places one).

## 8. Tests

* `cargo test`: payload encoders (sizes match registration), `decide_gameplay`, team lists,
  `ChangePlayerTeam` team choice (with and without a team limit), observer rule and `model` key, `SetDefaultPlayerTeam`'s keep rule (`teams::tests`), `RecountTeams` with and without a team limit, `IsValidTeam` and `TeamWithFewestPlayers` (`state::tests`), the client's team menu only in teamplay (`hud::state::tests::team_menu_only_in_teamplay`), `GetIndexedTeamName`, `CallCheckDeads`, the
  `fullupdate` `Spectator` flag, `UpdateClientData` byte/compare rules and resets
  (`player::hud::tests`), the impulse 99 `Logo` toggle, the spray frames, `SetCustomDecalFrames`,
  the customization types and the impulse 106 text (`player::impulse::tests`), `CTSGameRules::Init`
  (`state::tests::ts_init_resets_only_the_ts_state`, with the custom-weapons flag and the absorb
  values), the dispatch lend, `run_now` and `with_lent` (`world::tests`), the kill log's tags, ids and weapon
  renames and the ` (victims_weapon)` gun test (`scoring::tests`), the player's `soundent` slot
  (`soundent::tests::player_sound_follows_speed_and_silence`), the map cycle (`mapcycle::tests`: parsing,
  rule commands, min/max players, the traversal), the restriction mask's high bits,
  spawn scoring, easing, damage multipliers/kevlar/PTakeDam byte, chat formatting, globalslow
  clamps, weapon restriction, map vote parsing/tally, round refunds, `DoSpectacularNess` rounding/clamping, power-up bodies/respawn clamps, HL remaps, drowning/time-based damage (`player::water::tests`: drown ramp, wade sounds, 2 s tick gate, poison ticks and restart, antidote, drown recovery 10/10/2 and its stop underwater), the HLTV
  director event layout (`hud_wire::director_event_layout`), in-eye spectator movement at mode 5
  (`pm::tests::in_eye_spectator_is_mode_5`), TS's `FL_CLIENT` on a player who left
  (`rules::tests::ts_flags_keep_fl_client_on_a_player_who_left`), the truncated `ammocount` and its
  `Set_ammo_count` column (`state::tests::ammocount_cvar_is_truncated_then_mapped`,
  `weapons::assets::tests::ammocount_outside_1_to_3_keeps_the_default_carry`).
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
* Implement `give_weapon`, `give_ammo`, `remove_all_items`, `multiply_clip`, `on_killed` (drop weapons),
  `ts_init` (TSInit's free slots and loadout items, from the spawn).
  Ground items, power-ups (grenade 24, infinite clip), observers and death call them.
* Read `Player::active_powerup` for double fire rate (0x10) and infinite clip (0x2),
  `Player::kungfu` for ×2 melee.
* Fill `DamageInfo` fully: `.traced()` when TS goes through Clear/`TraceAttack`/Apply (it gates
  spawn protection, the stunt ×0.7, the TraceAttack team check and `PTakeDam`; several
  TraceAttacks before one Apply drive `damage::multi` directly), `hitgroup`, `pos`, `dir` (travel direction; all three only reach `TraceAttack`),
  `inflictor` (`"knife"` entity for thrown knives), bit `0x2000000` for shotgun pellets,
  `DMG_THROWN` for thrown knives. Wrap the hit in `damage::glast::close_combat` /
  `coldcock` / `weapon_id` where TS sets `glast_closecombat` / `glast_coldcock` /
  `glast_weaponid` (`CloseCombatAttack` and the kung-fu `CheckForBreakables` hit,
  `ColdCockAttack`, `UpdateBullet`): `Killed` and `PlayerKilled` read those, not the damage.
* Register no cvars that are in `rules::cvars::CVARS` (`realbullet`, `ammocount`, `damagemult`,
  `customweapons`, `weaponstay`): read them by name.
* `free_slots` (`+0x548`) drives `TSSpace` (`player::hud`); the kevlar/flag slot costs (10/20) go through the weapons'
  `free_slots` (`weapons::take_armor`, `weapons::sync_speed`). Only TSInit (`weapons::ts_init`) resets it and the
  loadout items (`+0x718`); `RemoveAllItems`, `StartObserver`, `Killed` and a disconnect don't, so an observer or a
  corpse keeps the count (and `TSSpace`, `maxspeed`) its weapons and vest left.
* Dropped guns use classname `WorldGun` and `pev->owner` = thrower (`CreateWeapon`); the glow
  shell reads the private dropper (`WorldGun::dropper`, TS +0xd0) through `weapons::glow_for_host`.

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

## 10. Bots (`bots/**`)

TS's bot AI exists only in the Windows server `mp.dll` (the Linux `ts_i386.so` keeps only the
`addbot` stubs). The port follows the decomp's `docs/server/bots.md`; addresses in this section
are `mp.dll` ones (Ghidra = objdump), player offsets are the `.so` ones (`mp.dll` + 0x14,
bots.md §14.1) like the rest of this page.

| Rust | TS (bots.md) |
|---|---|
| `bots/mod.rs` | `addbot`/`addcustombot` (`BotCreate` 0x1006b7c0, `BotCreateCustom` 0x1006b590, `BotSetup` 0x1006b3b0; §2), `CTSGameRules::Init`'s node load and `BotLoadFiles` (`bots::ts_init`), the `pfnStartFrame` driver (0x10010e00, before the rules' `Think`), the `CBot` overrides (`Spawn` 0x1006bb40: brain reset first, random laser colour last; `TraceAttack` 0x1006bac0: look at a player attacker; `Killed` 0x1006ba00 and `OnKill` 0x1006b970: kill chat), PreThink's node learning (0x1005fed2, `bots::learn`), the node client commands, and the port's quota. |
| `bots/brain.rs` | `CBotBrain` (§5): `Reset`, `Think` (module order state, 0x10, 0x100 respawn inline, 0x40, 4, 0x80, 8, 0x200, 0x400, 1; skipped while `fuser1 <= 0.1`), `PressButtons`/`UpdateButtons` (presses show a frame late), `UpdateView` (turn rate `10 × clamp(skill, 0.5, 2)`/s, punch low-pass), `RunMove` (msec `450 / frames per 0.5 s`, 5..100), `LookAt`, `UTIL_AngleMod` (0x10076250). |
| `bots/states.rs` | `CBotStateDM` (0x1006e900, modes 0/1) with `PickRoamGoal` (hill / power-up / spawn point, `TSFoundRandomEntity` 0x10073a30), and the unreachable `CBotStateTeamRounds` (0x1006ed00) and `CBotStateObjective` (0x1006f0c0). |
| `bots/modules.rs` | vision (0x1006dc10), buy (0x1006d630, `BotRandomAttachments` 0x1006d560, `AddItemToArmament`/`AddAmmoToArmament` semantics), stunt (0x1006e130), look around (0x1006dde0/0x1006dea0), power-up (0x1006e280), chat (0x1006e330) and `BotSay` (0x1006e7a0). |
| `bots/attack.rs` | `CBotModuleAttack` (§8): tracking with the knife lead, `SelectEnemy`, `ComputeAimOffset`, `FindCoverSpot`, `FireWeapon` per weapon kind, the close-range jump / stock hit (button 0x40), the dodge roll. |
| `bots/nav.rs` | `CBotNav` (§11.7): `Navigate`, `Replan`, `RandomWanderPoint`, `GetSteerPoint`, the ladder block. |
| `bots/nodes.rs` | `CNodeGraph` (§11.1–11.6): `.ndm` I/O, `AddNode`/`LinkNewNode`/`IsReachable`/`IsGroundContinuous`, `FindPath`, `FindPathBetweenPoints`, the debug beams (`sprites/lgtning.spr`, precached first in `W_Precache`'s tail as `mp.dll` does) and printouts; behind a `NodeWorld` trait so it's unit tested. |
| `bots/files.rs` | `bots.txt` (`Name//skill`, 64 names) and `bot_say.txt` (`offense`/`freetime`/`killed`), kept in TS's `char[64]` slot layout so the overflow quirks come out the same; `BotPickName` (0x1006c150); the chat line as a `printf` format. |

Player state the bots read, added to `Player`: `bot` (the `CBot` part: `Box<Brain>`), `cbot` (the
object is a `CBot`: `IsNetClient()` false; never cleared), `is_bot` (`m_bIsBot` `pl+0x160`: set by
`BotSetup`, cleared by `TSPlayerDisconnect`, turned into `FL_FAKECLIENT` by `Spawn`; only bots
with it think), `next_node_learn` (`pl+0x788`, zeroed by `TSInit`) and `player_speed`
(`m_flPlayerSpeed` `pl+0x550`, set wherever `SetPlayerSpeed` is: `TSInit`, the slot speed, The
One). The rest is the port's existing state: the loadout (`inv.sv.armament`, `pl+0x594`), held
and active power-ups (`ts.held`, `active_powerup`), the respawn time (`respawn_at`, `pl+0x770`),
the kung-fu bar (`inv.gun.melee_bar`, `pl+0x774`), the gun's status, kind, reserve and stock damage.

What changes for every player: `IsNetClient()` is false for bots (`rules::is_net_client`), so
`Host_Say` doesn't send chat to them and map votes don't count them; `BotSetup` skips
`ClientPutInServer`'s starting cash and `EF_NOINTERP` and the rules' `ClientConnected`.

Kept on purpose (bots.md §15): the `fabs(dot) > 0.9` aim cone (a bot fires at an enemy behind
it), the dead enemy kept until the 1–3 s retarget, the cover search caching the enemy's origin,
the attack think reading the enemy before re-targeting, `FindPath`'s backtrack popping the
current node (paths may skip a node) and its other tie and limit rules, `Load` rejecting exactly
512 nodes, `addbot`'s argument being the model, the `bot_say.txt` trim at byte 63, chat lines as
`printf` formats, bots chatting to everyone, `m_bIsBot` tested `== 1` (think) and `!= 0` (kill
taunt), no access check on the node commands, no automatic save. Every random draw is in TS's
order (bots.md §13.2).

### 10.1 Deviations

Port additions:
* **`ts_bot_quota`** bots ("Bot Neo", "Bot Trinity", ...) are TS bots: created like `addbot`'s
  (random model outside teamplay, random skill 0.1..1, no "connected with skill" line, so the
  dedicated-server test can tell them from an `addbot`), with `[ts] bots: added ...` in the log.
  **They also think on dedicated servers**, where TS thinks no bot: the quota exists to test
  dedicated servers. On a listen server every `m_bIsBot == 1` player thinks, as in TS. The
  quota counts and kicks only its own bots; a quota bot the engine kept across a level change
  without its brain gets a new one.
* **`ts_bot_stop 1`** freezes living bots: no AI, buttons or movement (the view still settles).
  Dead and observing bots still buy and respawn.
* **Deviation (chosen): bots on dedicated servers.** TS's `addbot`/`addcustombot` print "Bots
  only in listen servers." on a dedicated server, and its `pfnStartFrame` thinks bots only on
  listen servers. The port adds them and thinks every bot there too. Being server commands, they
  only run from the server's console, command line, configs or `rcon`; players can't run them.

Paths: `Load` reads `<gamedir>/maps/<map>.ndm` and `BotLoadFiles` `<gamedir>/bots.txt` and
`<gamedir>/bot_say.txt` as TS does (plain file reads relative to the engine's working directory,
not the engine's search path; each map start logs `[ts] bots: <names> names, chat lines [..],
<n> nodes`). TS's `Save` writes the hard-coded
`TS\maps\<map>.ndm`, which on Windows' case-insensitive file system is the shipped mod's game
dir, i.e. the file `Load` reads; the port writes `<gamedir>/maps/<map>.ndm`, the portable
equivalent (on Linux `TS/maps` would be another, usually missing, directory).

Guards where TS corrupts memory, reads garbage or hangs (behaviour otherwise kept):
* A `bots.txt` line long enough to spill past the last name slot is cut at the end of the skill
  table (TS runs on into the count and the used flags). A `%` conversion other than the first
  `%s` in a chat line prints nothing (TS reads garbage); formatted chat is cut at 127 bytes.
* `LinkNewNode`'s trace through doors gives up after 32 doors (TS loops forever between two
  overlapping doors). A negative node count from `Load` stays (so `addbot` still turns
  `botlearn` on) but `AddNode` drops the node TS would write before the array. `FindPath`'s
  `path:` printout has no 512-byte limit.
* The brain's enemy is an edict handle; it's only used while the edict still has a player (TS
  keeps a `CBaseEntity *` that a reused slot leaves dangling). `TraceAttack` doesn't allocate a
  `CBaseEntity` for an attacker without private data (TS's `GetClassPtr` does). An out-of-range
  `RANDOM_LONG` never indexes outside a table.
* Bots are driven only while they have a live user id (HL25 drops every bot when the last human
  leaves; `pfnRunPlayerMove` on a dropped client crashes the engine).

Not modelled: TS's PreThink returns early on the frame a player lets go of a train's controls
(the SDK's train block), before the node-learning block; the port's PreThink runs its later
hooks on that frame, so `botlearn` can offer one node there that TS wouldn't. Whether the
engines keep fake clients (and their `CBot` objects) across `changelevel` is engine behaviour
(bots.md §15); the port keeps a kept client's `Player`, brain included.

### 10.2 Tests

* `cargo test -p ts-server bots`: the node graph against all 43 shipped `.ndm` files (counts,
  links and ladders as bots.md §11.4, byte-for-byte re-save), the load quirks (512 rejected, 511
  accepted, negative, truncated), `FindPath` against `scripts/ndm.py`'s replica (reference data
  in `bots/testdata`, regenerated by `testdata/gen.py`: all pairs of ts_dojo and ts_japa, 3000 /
  2000 / 2000 sampled pairs on ts_fall, ts_central and ts_cave_of_fear, 341 full paths including
  backtrack-bug cases; every one matches), `AddNode`/linking with a fake world (spacing 128 / 32
  / 8, in-air rejection, floor gaps, ladder shortcuts, walls, doors, the 512 limit),
  `find_path_between_points`' tie rules; `bots.txt`/`bot_say.txt` parsing (the shipped files:
  64 of 70 names, 25/29/42 lines, the joined freetime line), the name picker, chat formatting,
  the brain's clamps, delays, angle wrap and button timers. The replica rounds `FindPath`'s
  heuristic to a float, which `mp.dll`'s x87 code doesn't; the reference data is generated with
  that rounding removed (it changes ts_dojo's 89 → 25 and 90 → 25, and the original TS prints
  `path:89,88,25` as the port does).
* `cargo xtask bots` (also part of `play`): TS bots on a ts_dojo listen server (see
  `docs/validation.md`).
