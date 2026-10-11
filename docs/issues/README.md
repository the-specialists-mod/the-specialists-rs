# Issues from the original repository

The port started in `CrunchyMonkies/rust-hlmod-the-specialists` (private). Its 112 issues, with their comments, are kept here
as `<number>.md`. Issue numbers in the code, commits and docs (`#23`, `issue #62`, ...) refer to
these files, not to issues in this repository. The issues that were still open were re-opened
in this repository; each new issue names its old number.

| # | State | Title | Labels | Re-opened as |
|---|---|---|---|---|
| [1](1.md) | closed | Grenade in-flight impact damage is credited to the grenade, not the thrower | bug, integration |  |
| [2](2.md) | closed | TS studio model renderer (split torso/legs bones) | enhancement, integration |  |
| [3](3.md) | closed | Motion-blur ghosts (TSMBlur) | enhancement, integration |  |
| [4](4.md) | closed | Scope overlay is not drawn | enhancement, integration |  |
| [5](5.md) | closed | Laser sight dots are computed but never drawn | bug, integration |  |
| [6](6.md) | closed | View polish not ported (drift pitch, idle sway, smoothing, local translucency, third-person smoothing) | enhancement, integration |  |
| [7](7.md) | closed | Spectator: HUD/overview and observer modes beyond the basics | enhancement, integration |  |
| [8](8.md) | closed | Voice status icons and voice ban UI | enhancement, integration |  |
| [9](9.md) | closed | HLTV director messages | enhancement, integration |  |
| [10](10.md) | closed | Joystick input on Steam GoldSrc | enhancement, integration |  |
| [11](11.md) | closed | Tune effect parameters and recoil against TS | enhancement, integration |  |
| [12](12.md) | closed | Corpse "bullet dance" is not wired | enhancement, integration |  |
| [13](13.md) | closed | Death animation wait is a fixed 1.2 s | enhancement, integration |  |
| [14](14.md) | closed | Map votes (vote map / vote stats / vote reset) | enhancement, integration |  |
| [15](15.md) | closed | Refunds on LMS round restart | enhancement, integration |  |
| [16](16.md) | closed | game_player_equip override in PlayerSpawn | enhancement, integration |  |
| [17](17.md) | closed | ts_groundweapon buyable and team checks | enhancement, integration |  |
| [18](18.md) | closed | Slow-motion scaling of projectiles and the bullet-time flag | enhancement, integration |  |
| [19](19.md) | closed | Weapons: dmgreport, underwater bubble trails, +use on weapons, ammo pack contents | enhancement, integration |  |
| [20](20.md) | closed | Stock entities: sentences (sentences.txt) | enhancement, integration |  |
| [21](21.md) | closed | Stock entities: train driving and func_tank | enhancement, integration |  |
| [22](22.md) | closed | Stock entities: remaining simplified or missing entities | enhancement, integration |  |
| [23](23.md) | closed | Validate on a Steam GoldSrc client (pre- and post-anniversary) | integration, validation |  |
| [24](24.md) | open | Validate with real keyboard and mouse | integration, validation | [#1](https://github.com/the-specialists-mod/the-specialists-rs/issues/1) |
| [25](25.md) | open | Validate on native Windows | integration, validation | [#2](https://github.com/the-specialists-mod/the-specialists-rs/issues/2) |
| [26](26.md) | open | Validate long sessions, many players and real network latency | integration, validation | [#3](https://github.com/the-specialists-mod/the-specialists-rs/issues/3) |
| [27](27.md) | open | Test remaining map features in-engine (CTB, ts_dmhill, ts_slowmotion, ts_wind, ground ammo, chat) | integration, validation | [#4](https://github.com/the-specialists-mod/the-specialists-rs/issues/4) |
| [28](28.md) | closed | "timeleft" chat line is not broadcast | bug, integration |  |
| [29](29.md) | closed | ts_groundweapon gives extra magazines even when the player already has ammo | bug, integration |  |
| [30](30.md) | closed | Game crates call the engine macros directly, breaking the unsafe boundary | bug, integration |  |
| [31](31.md) | closed | V_SmoothStairs port differs from TS (offset sign, 32-unit snap) | bug, integration |  |
| [32](32.md) | closed | spectacularness 0 still triggers slow-motion points | bug, integration |  |
| [33](33.md) | closed | Breakable explosions don't create the spectacular slow-motion point | enhancement, integration |  |
| [34](34.md) | closed | +use on stock entities skips TS's wall check, use sound and release handling | bug, integration |  |
| [35](35.md) | closed | Windows mouse code in goldsrc/platform.rs has unsafe blocks without SAFETY comments | bug, integration |  |
| [36](36.md) | closed | Observer sees "Press Fire To Play!" again every second after the respawn timer runs out | bug, integration |  |
| [37](37.md) | closed | V_CalcRefdef runs the normal view while paused | bug, integration |  |
| [38](38.md) | closed | Player WaterMove and time-based damage are not ported (drowning, poison, drown recovery) | enhancement, integration |  |
| [39](39.md) | closed | Entity keyvalues reach the class before the entvars fields (the SDK parses entvars first) | bug, integration |  |
| [40](40.md) | closed | Crosshair ignores the third-person aim point and hides when zoomed in third person | bug, integration |  |
| [41](41.md) | closed | TSExplosion's WorldGun knock-back is guessed, and the two TSExplosion ports have drifted apart | bug, integration |  |
| [42](42.md) | closed | View-height smoothing starts from the first frame's view height; TS starts at 0 | bug, integration |  |
| [43](43.md) | closed | Workspace is not clippy-clean | integration |  |
| [44](44.md) | closed | goldsrc ext/hud.rs keeps a hand-written EventApi mirror of event_api_s | integration |  |
| [45](45.md) | closed | Player TakeDamage rounds damage; TS truncates | bug, integration |  |
| [46](46.md) | closed | stunt::emit_sound_dyn rounds the pitch and skips sentences; TS's EMIT_SOUND_DYN truncates | bug, integration |  |
| [47](47.md) | closed | player_take_health doesn't start the overheal decay timer | bug, integration |  |
| [48](48.md) | closed | item_antidote is not ported | enhancement, integration |  |
| [49](49.md) | closed | worldspawn ignores most CWorld::KeyValue/Precache keys (MaxRange→sv_zmax, WaveHeight→sv_wateramp, …) | bug, integration |  |
| [50](50.md) | closed | PostThink runs during intermission; TS skips it when the game is over | bug, integration |  |
| [51](51.md) | closed | +use doesn't take the CTB case (CTSFlag::Use not ported) | bug, integration |  |
| [52](52.md) | closed | goldsrc ext/hud.rs still keeps hand-written mirrors of triangleapi_s and demo_api_s | integration |  |
| [53](53.md) | closed | Player HUD health/armour bytes are rounded and capped; TS truncates | bug, integration |  |
| [54](54.md) | closed | UTIL_ScreenShake: two port versions, neither matches TS's divide-then-truncate | bug, integration |  |
| [55](55.md) | closed | Explosion damage through walls skips the player TraceAttack step (spawn protection, stunt ×0.7, PTakeDam) | bug, integration |  |
| [56](56.md) | closed | PreThink keeps running player code during intermission (TS returns early when the game is over) | bug, integration |  |
| [57](57.md) | closed | CTB timed return: wrong FlagFall delay and message text | bug, integration |  |
| [58](58.md) | closed | goldsrc client: duplicate engine wrappers in ext/render.rs and ext/hud.rs, and an unchecked pEfxAPI | integration |  |
| [59](59.md) | closed | CTB pickup, drop and capture details differ from TS (messages, sound, log, TSFlag, throw) | bug, integration |  |
| [60](60.md) | closed | goldsrc client: remaining duplicate wrappers (Cvar types, screen info, console string, fill, level name, sprite pointer, TriAPI) | integration |  |
| [61](61.md) | closed | Crosshair-ID trace passes ignore_pe -1 where TS passes the local player index | integration |  |
| [62](62.md) | closed | Steam HL25 client: world stays black and the loading dialog never closes | bug, integration, validation |  |
| [63](63.md) | closed | Kevlar slot weight bypasses the shared free-slot count and is lost on the next weapon change | bug, integration |  |
| [64](64.md) | closed | WorldGun BounceTouch skips the ×0.9 ground damping and uses the wrong slow-motion floor | bug, integration |  |
| [65](65.md) | closed | Client doesn't register TS's CAM_Init, gl_fog_*, scr_ofs* and bitmapfonts cvars; gl_fog 2 user fog not ported | integration |  |
| [66](66.md) | closed | Stuck knives get a port-only ×0.5 ground friction in think | bug, integration |  |
| [67](67.md) | open | xtask: scripted Steam GoldSrc client checks (listen and dedicated) | enhancement, integration, validation | [#5](https://github.com/the-specialists-mod/the-specialists-rs/issues/5) |
| [68](68.md) | closed | Thrown knives set pev->owner, which TS doesn't; the thrower's glow is keyed off it | bug, integration |  |
| [69](69.md) | closed | Third-person camera controls: TS's +cam* commands and the thirdperson toggle aren't ported | integration |  |
| [70](70.md) | closed | AddToFullPack differs from TS: NODRAW test, groupinfo filtering, eflags, render rounding, weaponmodel | bug |  |
| [71](71.md) | closed | Client third-person state: is_third_person includes the death cam; demo record types 0 and 1 unused | bug |  |
| [72](72.md) | closed | Client camera/timescale: cl_thirdperson and cl_thirdpersonswitch acted on in view::frame, PwUp switch condition, timescale eased twice | bug |  |
| [73](73.md) | closed | AddToFullPack: spectator field and empty-model skip differ from TS | bug |  |
| [74](74.md) | closed | Client: VidInit time-scale reset, Spectator prediction reset, observer flag, emitter frame-time clamp, WeaponInfo 0xff | bug |  |
| [75](75.md) | closed | Client: initial slow-motion target, respawn spread reset, emitter behind camera, FX_AdaptQuality timing | bug |  |
| [76](76.md) | closed | Client prediction: gun re-init on respawn, holster on death, emitter count/sprite order | bug |  |
| [77](77.md) | closed | Client emitters: 2D camera test, strict inside test, raw count range, sprite-miss caching, ready flag on VidInit | bug |  |
| [78](78.md) | closed | Client weapon prediction: weapon switches not predicted (SelectWeapon/Deploy), m_iId gate, records rebuilt every command, VKickback clock | bug |  |
| [79](79.md) | closed | Client prediction: view-anim body, akimbo right-clip clamp, ItemPostFrame gate; rain doesn't lead the player | bug |  |
| [80](80.md) | closed | Client view: V_CalcBob and V_CalcGunAngle differ from TS; normal view ignores spectator camera overrides | bug |  |
| [81](81.md) | closed | Server: jointeam/ChangePlayerTeam, fullupdate and CallCheckDeads differ from TS | bug |  |
| [82](82.md) | closed | Client: akimbo view animations use the wrong anim-set indices | bug |  |
| [83](83.md) | closed | Close-combat view animation: server sends wrong i2 range, client plays it as a raw sequence | bug |  |
| [84](84.md) | closed | Server: changeteam VGUIMenu outside teamplay, ChangePlayerTeam model key, kill path alive check | bug |  |
| [85](85.md) | closed | Client close-combat trace length; stale 'kung fu not predicted' doc line | bug |  |
| [86](86.md) | closed | Server teams: SetDefaultPlayerTeam sets keys, single-team list keeps the limit | bug |  |
| [87](87.md) | closed | Server teams: ClientUserInfoChanged (teamplay and multiplay), The One team switch, game_player_team | bug |  |
| [88](88.md) | closed | Server: game_* map-rule entities and RespawnEntities incomplete | bug |  |
| [89](89.md) | closed | Server rules: UpdateClientData throttle, case-sensitive commands, impulse 99, CTB team_win, restriction mask, intermission/deathmatch details | bug |  |
| [90](90.md) | closed | Server weapons/damage: pellets summed per victim, game_player_equip without weapon_tsgun, death order and CurWeapon | bug |  |
| [91](91.md) | closed | Client effects: client bullets update/draw timing and slow-motion sprite, fire sound sample, fog via raw GL | bug |  |
| [92](92.md) | closed | Client HUD: HUD_Shutdown input shutdown, TFC hooks and status bar, death notices/HudText audit | bug |  |
| [93](93.md) | closed | Client VGUI1 panels: team menu, MOTD, scoreboard and voice labels in TS's layouts and fonts | enhancement |  |
| [94](94.md) | closed | Client-side movement arithmetic: audit shared pm against client.dll's compiled code | bug |  |
| [95](95.md) | closed | Server: entity classes TS exports that the port doesn't | bug |  |
| [96](96.md) | closed | Server combat/scoring: close-combat and coldcock flags, death knockback direction, damage to dead players, score clamping, spawn cash copy | bug |  |
| [97](97.md) | closed | Server: impulse 201 and cheat impulses, 'use <item>', Train message order, ChangeLevel Init, game_player_team timing, deathmatch 0 single-player rules | bug |  |
| [98](98.md) | closed | Client events: impact material-sound roll, FireBullets penetration uses global remaining damage | bug |  |
| [99](99.md) | closed | Client spectator maths: blocked-trace NaN, VectorAngles, InterpolateAngles, NormalizeAngles | bug |  |
| [100](100.md) | closed | Client HUD: Geiger sounds, scoreboard last killer, TS localisation rules, non-UTF-8 player names | bug |  |
| [101](101.md) | closed | Server: thrown-knife names, victims_weapon log suffix, ScoreInfo truncation in entities, g_vecAttackDir writers, Init resets, latched sv_cheats | bug |  |
| [102](102.md) | closed | Server entities: dead-player pickups, gun weapon bit, global precaches, LinearMove/AngularMove, crushers, func_train sounds, cycler_weapon, ParmsChangeLevel | bug |  |
| [103](103.md) | closed | Client: ShowMenu key-name substitution; overview name tags use decoded names | bug |  |
| [104](104.md) | closed | Server: UpdateOnRemove globalname, monster TakeDamage classes, GiveNamedItem gun entity, per-class precache audit, blood splat language, save data | bug |  |
| [105](105.md) | closed | Server: game_player_hurt and player_weaponstrip defer, weapon_tsgun precache, StartObserver holster count | bug |  |
| [106](106.md) | closed | Server: free slots and items reset on StartObserver/disconnect; weapon net caches at spawn | bug |  |
| [107](107.md) | closed | Server: Server_GetBlendingInterface not exported (server-side bone setup / hitboxes) | bug |  |
| [108](108.md) | closed | Server player: single-player HEV suit sentence queue, PlayerDeathThink as a real think | bug |  |
| [109](109.md) | closed | Server entities: pendulum, chargers, game_text, TEAM_MASTERS, effects removal, ambient_generic, env_fade, trains, trigger_camera, info_intermission, TS weapon remaps, ts_particle/emitter, deadcorpse | bug |  |
| [110](110.md) | closed | Server: ts_groundweapon unknown id and akimbo remap; func_recharge player check | bug |  |
| [111](111.md) | closed | Server: IsPlayer() vs FL_CLIENT across is_player call sites; ammocount mapping | bug |  |
| [112](112.md) | closed | Server: realbullet and usecash SrvSett sends (truncation, re-send every Think) | bug |  |
