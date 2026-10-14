# Bots audit: native TS bots (3.0 vs Windows mp.dll; 2.1 overrides vs 2.1 mp.dll)

Port at c49ee1a. 3.0 = `ts30-decomp/server-win` (mp.dll), 2.1 = `ts30-decomp/ts21/server-win`.
Method: decomp of every function, disassembly wherever rounding / summation order / constant width mattered.
Not filed (documented deviations): quota bots and addbot on dedicated servers; `ts_bot_stop`; port guards (live(), clamps on RNG indices, door-trace limit, buffer caps).

## Issues filed
- #154 Audit 3.0: FireWeapon burst timer `h+time` rounded before `+0.3`; knife `tt` rounded before `tt*v` / `tt*360` (mp.dll keeps both on x87).
- #155 Audit 3.0: PreThink delayed suicide runs before node learning (TS: learning first, suicide last) -> no node on the suicide frame; stunt PreThink also runs after death.
- #156 Audit 2.1: ladder block uses 3.0's combined masks; under 2.1's slot table the probe's +jump slot and the old FORWARD/BACK slot aren't released (2.1 does ReleaseButton). Plus 2.1-only rounding in RandomWanderPoint (x) and GetSteerPoint (forward z).
- #157 Audit 2.1: ts21 scoring calls `bots::on_kill` directly (plugin backends miss it, extra RandomLong for plugin bots) and never calls `plugins::on_kill`.
- #162 Audit 2.1: GetBetterWeapon (bots' FireWeapon, and every drop-with-switch) uses 3.0's priority list; 2.1 (0x1004a410) takes the owned id with the highest cost x (akimbo ? 2 : 1), lowest id on ties.
- #163 Audit 2.1: node graph IsReachable (unrounded x/y differences) and IsGroundContinuous (unrounded differences, ZYX length, float reciprocal 1/steps instead of a division) use 3.0's rounding in the port.

Second pass (coordinator request): the five gaps below were closed against the binaries.

## Coverage (port fn -> 3.0 original / 2.1 original -> status)

| Port function | 3.0 (mp.dll) | 2.1 (mp.dll) | Status |
|---|---|---|---|
| rules/commands.rs cmd_addbot / cmd_addcustombot | 0x10026110 / 0x100261a0 | 0x1001e660 / 0x1001e6c0 (doc) | match (arg order argv3,2,1; help) |
| bots::add_bot / add_custom_bot / pick_model | BotCreate 0x1006b7c0 / BotCreateCustom 0x1006b590 | 0x1005dc30 / 0x1005da10 | match (draw order RL(0,6), RF, name RLs; 2.1 model also in teamplay, no botlearn 1) |
| bot_backend::create_player | BotSetup 0x1006b3b0 | 0x1005d8c0 | match (print, decal -1, Spawn before keys, team key 3.0 only, ChangePlayerTeam, PlayerThink) |
| rules::stored_skill | BotSetup floor 0.1 | no floor | match |
| files::BotNames::parse / pick | BotLoadFiles 0x1006bee0 / BotPickName 0x1006c150 | 0x1005e180 / 0x1005e380 | match both (2.1 decomp+disasm: same table layout, "rb", sscanf "%f" into skill[count], same trim/64 cap, same outward search) |
| files::BotSay::parse / pick, format_line, say_text | BotLoadSay 0x1006bc00, BotSayLine 0x1006bbc0, BotSay 0x1006e7a0 | 0x1005df50/0x1005df10/0x100604a0 | match both (2.1 BotLoadSay decomp: same headers/word reuse/64 stop; sprintf(buf,line,name) in disasm); ts21/data bots21.txt + bot_say21.txt byte-identical to ~/dev/hl/ts21 files |
| bots::start_frame | pfnStartFrame 0x10010e00, BotPlayerByIndex 0x1006b360 | | match (+ documented deviations) |
| bots::spawn_pre / Brain::reset | CBot::Spawn 0x1006bb40, Reset 0x1006f2c0 | 0x1005def0, 0x10061220 | match (both memsets, chat timer RF(60,180)*clamp, state by mode) |
| bots::laser_colour | CBot::Spawn tail | (none in 2.1) | match |
| bots::turn_to_attacker | CBot::TraceAttack 0x1006bac0 | none (no override) | match |
| bots::killed | CBot::Killed 0x1006ba00 | 0x1005de40 | match (has_player vs Classify()==2 equivalent) |
| bots::on_kill + scoring call site | CBot::OnKill 0x1006b970, PlayerKilled 0x1005b190 | 0x1005ddd0, 0x1004fe02 | 3.0 match; 2.1 call site MISMATCH #157 |
| brain::think_with | CBotBrain::Think 0x1006f500 | 0x100614e0 | match (module order, 0.1f slow gate / 2.1 fuser1 test, turnSpeed 30) |
| brain::respawn_now / ts21 respawn | inline 0x1006f5b8 | press ATTACK 0.1 | match |
| brain::run_move | RunMove 0x1006e6d0 | 0x100603d0 | match |
| brain::update_view / ts21 change_yaw | UpdateView 0x1006e500 (doc + #45) | ChangeYaw 0x10060120 | match (2.1 disasm: step unrounded) |
| brain::set_ideal_toward / ts21 look_at, set_ideal_angles | LookAt 0x1006f740 | 0x100616b0, 0x10060090, 0x10060020 | match |
| brain press/update bits / ts21 Buttons | 0x1006f470 / 0x1006f4b0 | 0x100613e0 / 0x10061450 / 0x10061490 | match |
| rules::delay / ts21 timer, turn_rate, runs_ai | §5.8 formula, 0x1006f510.. | FUN_100551c0 timers | match (constant widths checked) |
| states::think / pick_dm / random_entity | 0x1006e900, TSFoundRandomEntity 0x10073a30, GetTeamsNum 0x1004df10 | 0x100605f0, 0x100649c0 | match (hill dist ZXY both) |
| states::pick_team_rounds / ts21 pick_team_rounds | CBotStateTeamRounds::Think 0x1006ed00 (unreachable in 3.0) | 0x10060950 | match both (3.0 decomp: GetTeamIndex==1 -> team0 else team1 spawn/fallback, 32 cap, keepGoal writes, timers) |
| states::think Objective branch / ts21 state Objective | CBotStateObjective::Think 0x1006f0c0 (unreachable in 3.0) | 0x10061020 | match both (3.0 decomp) |
| modules::vision / ts21 vision | 0x1006dc10 | 0x1005f890 | match |
| modules::buy / random_attachments / buy_weapon / ts21 buy | 0x1006d630 / 0x1006d560 | 0x1005f320 / 0x1005f250 | match (ids, ammo, draws); 2.1 SMG/rifle tail per doc |
| modules::stunt (+ stunt_mode) | 0x1006e130 | 0x1005fc40 | match |
| modules::look_around / find_look_point / ts21 look_around | 0x1006dde0 / 0x1006dea0 | 0x1005fa90 | match (disasm: unnormalized dir, ZYX) |
| modules::powerup | 0x1006e280 | 0x1005fd90 | match |
| modules::chat | 0x1006e330 | 0x1005fe40 | match |
| attack::think / ts21 attack::think | 0x1006c520 | 0x1005e5e0 | match (dist XZY 3.0 / ZYX 2.1; 2.1 aim offset disasm) |
| attack::select_enemy / ts21 | 0x1006cec0 | 0x1005ed60 | match (ZYX 3.0 / ZXY 2.1) |
| attack::compute_aim_offset | 0x1006c3c0 | inline | match |
| attack::find_cover_spot / ts21 | 0x1006cbc0 | 0x1005e9a0 | match (2.1 far fan: anglemod-44, ftol) |
| attack::fire_weapon / ts21 fire_weapon | FireWeapon 0x1006d010 | 0x1005ee60 | 3.0 MISMATCH #154 (rounding); 2.1 match |
| attack::holding_melee / can_stock_hit | 0x1006cfc0 / 0x1006cf90 | 0x1005ee10 | match |
| bot weapon helpers in 2.1: kind_of, info() attachment masks, cost | info+0x348/0xe0/0xe4 | info+0x360/0xd8/0xdc/0xd0 | match (ruleset tables; weapons21.toml fields checked vs 2.1 binary by the weapons audit's cmp21.py) |
| weapons::get_better_weapon (Gun::get_better_weapon) | CTSGun::GetBetterWeapon 0x100548d0 | 0x1004a410 | 3.0 match; 2.1 MISMATCH #162 |
| switch_to / remove_weapon (better switch) | SelectWeapon 0x10054930 | 0x1004a4c0 / 0x10049fb0 | match |
| weapons::drop_weapon(Drop::Current) from bots | DropWeapon(-1, 0) 0x10062720 | DropWeapon(-1) 0x10056080 | body = weapons area (#146, #158 there); call semantics match |
| nav::navigate | Navigate 0x1006fc00 | 0x10061b70 | 3.0 match; 2.1 ladder block MISMATCH #156 |
| nav::replan | 0x1006fa40 | 0x100619b0 | match |
| nav::random_wander_point | 0x1006f8c0 | 0x10061840 | 3.0 match; 2.1 x rounding #156 |
| nav::steer_point | GetSteerPoint 0x10070400 | 0x10062330 | 3.0 match; 2.1 forward-z rounding #156 |
| nav::Nav::reset | 0x1006f810 | 0x10061790 | match |
| nodes::add_node / is_too_close / is_in_air | 0x10070e30 / 0x10070f50 / 0x10070ed0 | 0x10062bb0 / 0x10062cc0 / 0x10062c50 | match |
| nodes::is_reachable / is_ground_continuous | 0x100711e0 / 0x10070fe0 | 0x10062ed0 / 0x10062d50 | 3.0 match (disasm); 2.1 MISMATCH #163 (rounding) |
| nodes::link_new_node | 0x10071410 | 0x10062fc0 | match both (disasm: j==i skipped, door loop restart at hit point ignoring the door, ZXY ftol, 8-byte copy keeps pad, beam(i,j)) |
| nodes::find_path, push/pop, ClearScratch | 0x100717f0, 0x100715f0/0x10071620/0x10071650 | 0x10063360, 0x10063150/0x10063180/0x100631b0 | match both line by line (Dijkstra ties/relax, walk g<bestG&&f<bestF, Pop-top quirk, 1000 limits, h ZXY, output order, debug strings, fail returns 0) |
| nodes::find_path_between_points | 0x100716a0 | 0x10063200 | match both (ZYX ftol, strict <, ignore = bot edict) |
| nodes::load_bytes / save_bytes / ndm_path | Load 0x10071cb0 / Save 0x10071e60 | 0x100639d0 / 0x10063b50 | match (n<512, unchecked inner freads, count<=0 no save; TS\\maps save path is the port's documented substitution) |
| nodes::client_command | pfnClientCommand 0x1000f8a0 | 2.1 pfnClientCommand | match both (testpath argc>2, addnode IsAlive && iuser1==0, manual) |
| bots::learn (PreThink block) | 0x1005fed2 | 2.1 PreThink 0x100537e0 | 3.0 ordering MISMATCH #155; 2.1 match |

Counted: ~62 port functions checked (both passes), 56 match, 6 issues (#154-#157, #162, #163).

## Not verified / remaining limits
- Engine RandomFloat return precision (x87 st0 vs float) can't be modelled; assumed float everywhere.
- 2.1 DropWeapon body (0x10056080) beyond the bot call: owned by the weapons audit (#146, #158).
