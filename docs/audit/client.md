# Client audit (3.0 + 2.1)

Scope: crates/client/src/** (HUD, input, FOV, buy menu, messages, effects readability, spectator) and crates/ts21/client/src/**.
Asm: /tmp/audit/cl.asm (3.0 client.dll), /tmp/audit/cl21.asm (2.1 client.dll). Decomp via /tmp/audit/fn.sh client|ts21/client <addr>.

Issues filed: #144, #152, #153, #160, #164, #165; comment on #62. Existing issues that already cover findings: #38, #39, #41, #58, #62, #72-74, #77, #81, #108, #110, #111, #112, #115.

## Coverage
| Port function | 3.0 original | 2.1 original | Status |
|---|---|---|---|
| hud/weapons.rs draw_crosshair (gate, colours, line layout, team sprite x0.7, enemy x0.75, laser number x0.7/x0.5 and -8 scoped) | DrawCrosshair 0x1004e9f0 (+disasm) | 0x1008b960 | match, except solidcross test: TS `cl_solidcross > 0 && IEngineStudio.IsHardware()` (0x100ea860 = studio api slot 40), port `cross2 sprite loaded` (software renderer only) |
| ClientRules::crosshair_parts (3.0 default / 2.1) | 0x1004e9f0 weapon list 0/24/25/34/35, divesteps first & skipped when scoped | 0x1008b960 single gate incl. Barrett fov<40, divesteps last | match |
| crosshair_gap 3.0 | 0x1004eb7f x87 | — | existing #110 |
| crosshair_gap 2.1 | — | 0x1008bb67 (fstp float, /0.1f, x16) | match |
| divestep_layout / divesteps_blocked 3.0 | 0x1004ef50, thresholds 0x1004eec0/ef0/f20 (`81-free-0 <= 41/51/61`) | — | match (pitch band NaN edge differs: TS band 0, port 1; irrelevant) |
| divesteps_blocked / stunt_cooldown 2.1 | — | 0x1008c0f0.. | match (doc) |
| state.rs apply: TSHealth, TSArmor, TSState, TSCash(signed short), TSSpace, PwUp(0x104 split), PwUpTime, RoundTime, TSFlag, TSPAward(1..64), PlInfo(key 0), SrvSett(0..3), TSBTime, TSMessage(|t| cap 100), TSFade, TSSlowMo(goslow when scale 1), ClipInfo, WeaponInfo(3rd-person switch ==1.0), WStatus, ActItems(0..32), NextStunt, Wires, KFuPower(±100) | 0x1004b730..0x1004c1e0, 0x1004f6f0..0x1004fb00 | 0x100877b0.., 0x1008d000.. | match |
| KFuPower / WeaponInfo id 0 in 2.1 | — | 0x1008d000 clamps 0..100; 0x1008d1a0 sets bar from clip on id 0 | not hooked; documented integration point (ts21.md); bar never negative in port (charge_melee_bar clamps 0) |

| input.rs key_event (anti-script gate) | HUD_Key_Event 0x10025a80 (gate = eax 1 on any match, press or release) | 2.1: no gate (hook) | MISMATCH #144 (port opens gate on press only) |
| input.rs + handlers (+forward/back/moveleft/moveright/attack/attack2/alt2/jump/reload/duck), key_tap bits | 0x10025da0..0x10026210, FUN_10023450 | hooks: no gate/switch/respawn | match |
| attack_switched (cl_attackswitch kinds 2/3/10/11, id 0) | 0x10025f40/0x10025fc0/0x10026110/0x10026180 | n/a | match (invalid id >38 edge: TS uses entry 0) |
| button_bits | CL_ButtonBits 0x10026870 (+FUN_10026860 health<1, intermission) | — | match |
| +use usepowerup double tap | weapons Think 0x1004ce10 (`now <= last+0.3`) | — | existing #62 |
| think_fov (default_fov clamp, target, zoom sens), ease_fov | CHud::Think 0x10049760, UTIL_ApproachFOV 0x10049620 (+disasm) | 0x10084aa0, 0x10084800 | match |
| weapons think: stunt countdown, kungfu regen 3.0, heartbeat/breathe, firemode switch.wav, lasclr | 0x1004ce10 | — | match |
| buy.rs cost / slots_for / table / random_config / random_attachments | 0x10012ad0, 0x10038dc0, Init 0x10036200, RandomConfig 0x100398e0, 0x10039810 | Init 0x1006aa70, RandomConfig 0x10070860 (memset, no kevlar, copies to +0x41c) | match (3.0 tables in 2.1 = #111; ammo = #108) |
| status.rs draw_round_timer / tick_round_timer, draw_powerup label/number | DrawRoundTimer 0x1004a130, DrawPowerup 0x1004a2e0 | — (2.1 status look kept by doc) | match |
| hud/notice.rs DeathNotices::add/draw | MsgFunc_DeathMsg 0x1001b360 | — | spot-checked, match (prior audits) |
| view.rs fade_color slow tint in 2.1 mode | V_CalcRefdef tail (cl_slowfade) | no tint, no cl_slowfade | MISMATCH #152 |
| events.rs fire_pitch/slow_pitch in 2.1 mode | 0x1003eeed etc. | 0x100738a0 (100+max(300s,150)), hks fixed 94+rand(0,15) | MISMATCH #152 (per-sound 2.1 pass not done) |
| menus/scoreboard.rs sort_players / sort_teams | SortPlayers 0x1005d8f0 (slots 1..63, frags desc, deaths asc) | — | match |
| state.rs ScoreInfo / Spectator | 0x10065090 / 0x10065250 | — | match |
| codec write order (server side) | TSFlag in CBasePlayer / CTSBriefcase: state, idx, team, carrier | — | match |
| spectator in_eye_position | V_GetInEyePos 0x10068e20 (needs a player-info name) | — | MISMATCH #153 |
| hud/weapons.rs solidcross test | 0x1004ecf3 IsHardware() | 2.1 0x1008b960 same | MISMATCH #153 |
| effects/laser.rs draw_shadows | DrawPlayerShadows 0x10055760 | — | match |
| 2.1 usepowerup | — | 0x100899f0 (same double tap as 3.0) | existing #62 (commented) |
| events/fire.rs fire_standard/akimbo/pump, gun_fx, Shot (2.1 mode) | 0x1003dfa0 / 0x1003f090 / 0x10040c20 (match) | 0x10073b70 / 0x10075080 / 0x10076ce0 | MISMATCH #160 (smoke rate, low-smoke ids, spray gravity, shell speed/sound) |
| fire_sound (2.1 mode) | 0x1003de90 | 0x10073a40 (same volumes/attn/samples) | match except pitch (#152) and .454 attenuation (#160) |
| fire_bullets (2.1 mode): pellet pattern / akimbo offsets | 0x100450c0 | 0x1007c5e0 | hooked, match; whizz, push and client-bullet gate → #160 |
| impact.rs texture_sound / material_sounds | 0x10044bf0 | 0x1007b820 | lists, volumes and attenuation match; pitches → #160 |
| fx::impact (3.0) incl. type jump table, impact_smoke | 0x10048f60 (table 0x10049280), 0x10046680 | — | match |
| fx::impact (2.1 mode) | — | 0x10084240 | MISMATCH #160 (concrete/metal/glass/wood particle params) |
| misc.rs empty / reload / pump (2.1 mode) | 0x100424e0 / 0x10042690 / 0x100419a0 | 0x10078810 / 0x10078940 / 0x10077c40 | match except pitch, reload has no anim or phases (#160) |
| misc.rs knife / katana / close_combat / coldcock / melee_material_sound (2.1 mode) | 0x10043c90 / 0x10044230 / 0x10043200 / 0x10043800 / 0x10045df0 | 0x1007a260 / 0x1007ab50 / 0x100792e0 / 0x10079aa0 / 0x1007d250 | MISMATCH #160 (reaches, coldcock reach, block branch, sparks, material vol/pitch) |
| client bullets: bullet_velocity_mult / next_update / step_time / draw (3.0) | 0x10055b30 / 0x10035b50 / 0x10035d40 / 0x10035010 | — | match (constants spot-checked: 37.5, 5..25, d²·0.1, 0.15) |
| client bullets (2.1 mode) | — | 0x1006a070 / 0x1006a3f0 / 0x10069540 / 0x10069160 | MISMATCH #165 (speed, stepping, model drawing) |
| view.rs Bob::calc / bob_levels (3.0) | V_CalcBob 0x10066e70 | — | match |
| view.rs Bob (2.1 mode) | — | 0x100b5c80 | MISMATCH #164 |
| punch (ev_punchangle) | only zeroed (0x1000b9e4) | KAlign (never sent), dead 0x10096310/0x100964a0, zeroed at deploy | match (always 0 in both) |
| view.rs apply_kick | V_ApplyKick 0x10057ed0 | 0x10097de0 | match (2.1 float stores, at most one ulp) |
| status.rs draw_status_panel (stance fill, hp/kevlar <50 red, slots 0 red, cash <1000 red and its gate, team bar) | DrawStatusPanel 0x1004a3e0 | — | match |
| scoreboard status / death_msg / rebuild_teams / sort / team_score | 0x1005db70 (status switch), 0x1005e5d0, 0x1005da20, 0x1005d8f0/0x1005d6a0, 0x10065120 | — | match |
| team menu slot_input / update | 0x100665a0 / 0x10066020 | — | match |
| overview add_entity / death | 0x100249f0 / 0x10024a60 | — | match |
| state.rs menu_select | CHudMenu::SelectMenuItem 0x10028700 (callers gate on displayed, slots 1..10) | — | match |
| state.rs add_ts_message | AddTSMessage 0x1004b520 | — | match |
| 2.1 second bob vector 0x1032073c | — (3.0 adds no bob to vieworg) | V_CalcNormalRefdef 0x100b6c90 adds it to vieworg (camera bob) | MISMATCH #164 (comment) |
| fx::muzzle_flash in 2.1 mode | 0x100474b0 (match) | 0x10080a30 | MISMATCH #160 comment (colour/fade, no ×1.3 third person, no blast push, extra scintille_metal, silenced params) |
| fx::bullet_trail in 2.1 mode | 0x10046240 (match: colour alpha×0.75, fade 0.1) | 0x1007d8b0 / 0x1007dc70 / 0x1007de30 by cl_wavedetail | MISMATCH #165 comment |
| 3.0 particle builders: sparks, scintille, sprite_debris, surface_mark, gun_smoke, small_puff, impact_smoke | 0x100492b0 / 0x10046f50 / 0x10046bd0 / 0x10046a70 / 0x10047130 / 0x100464f0 / 0x10046680 | — | match (body constants; +0x120 collision mode folded into `collide`) |
| 2.1 particle builders (port uses 3.0's in 2.1 mode) | — | 0x1007f970 / 0x1007ef20 / 0x1007f3f0 / 0x1007e7f0 / 0x1007e550 / 0x1007ec80 | MISMATCH #160 comment (gun smoke gravity 0.1 and count 10 at cl_fx 2, debris has no react, no drag/stretch, no sparks builder); the others match |
| hud/notice.rs DeathNotices::add / draw (TS-modified: weapon as text, nohud gate) | 0x1001b360 / 0x1001b020 | 0x10040540 / 0x1003ff80 | match (both) |
| spectator/panel.rs texts/update | UpdateSpectatorPanel 0x10063870 | 0x100af0c0 | match, except TS also hides on IsSpectateOnly()==2 (not reachable: FWGS returns 0/1); not filed |
| state.rs Motd handling, menus/motd.rs title | MsgFunc_MOTD 0x10064f60, CreateTextWindow 0x10063c00 | 0x100b1f40 | match (both) |

## Stock SDK modules (skipped, per the decomp's architecture-hud.md §4 and the 2.1 message table "stock layouts")
CHudSayText (chat), CHudMessage (HudText/GameTitle title texts), CHudStatusBar (StatusText/StatusValue, never sent by TS), CHudMenu (ShowMenu text menu), CHudTextMessage (TextMsg), CHudGeiger, CHudTrain, the TFC leftovers (SecAmmo etc.) and the SDK VGUI1 plumbing (scroll panels, command menu). TS's changes in those areas sit outside them (the `nohud` cvar, the menu-key `slot` handling) and are covered above. The TS-modified modules (death notices, spectator panel, MOTD/briefing window, scoreboard, team menu, TS status/weapons elements) were checked for both versions.

## Not verified / gaps
- 2.1's particle *update* (whether its particle system has drag and stretch at all) was not traced; the finding is limited to the builders not writing those fields.
- The pitch-dot position in f32 vs TS's x87 sin/cos: at most one pixel off, rarely. Not filed.
