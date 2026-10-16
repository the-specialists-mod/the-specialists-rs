# Client audit, round 2 (3.0 + 2.1)

Scope: `crates/client/src` (HUD, input, view, effects, events, spectator, menus), `crates/ts30/client/src/{lib,view,events,fx,spectator}.rs`, `crates/ts21/client/src/**`. Client weapon prediction is the weapons area. Port commit 661d918 (after #200).
Asm: objdump listings of 3.0's and 2.1's client.dll. Decomp: `scripts/round2/fn.sh client|ts21/client <addr>`, disassembly `dis.sh cl|cl21 <addr> <len>`, constants `rd.py cl|cl21 [d:|s:]<addr>`.

Round-1 issues #144, #152, #153, #160, #164, #165 are all closed. Their fixes were re-checked (rows marked "fixed").
Issues filed this round: **#203** (3.0), **#204, #207, #230, #232, #240, #244** (2.1), plus comments on #230 (goslow) and #232 (PTakeDam blood). Validation added **#251** (2.1 ricochet pitch rounding), **#252** (3.0+2.1 `hud_takesshots` timing) and a #232 comment (2.1 glass; blood correction).

Method note: since #200, every `ClientRules` hook has a 3.0 answer in `ts30-client` and a 2.1 answer in `ts21-client`. This round concentrated on **core client code with no hook**, which 2.1 mode runs as TS 3.0's. Each such path was compared against 2.1's `client.dll`, and most of the findings are there.

## Coverage
| Port function | 3.0 original | 2.1 original | Status |
|---|---|---|---|
| hud/state.rs `TsHealth` (death-cam toggle `!observer && hp==0 && !third`) | 0x1004b730 | 0x100877b0 | match (re-checked 2.1 ✓) |
| `TsArmor`, `TsState`, `TsCash` (signed short), `TsSpace` | 0x1004b790.. | 0x10087a10/a70/ad0/b30 | match (re-checked `TsCash` 2.1: `READ_SHORT` ✓) |
| `PwUp` (0x104 split) | 0x1004b9e0 | 0x10087b90 | match. Both zero the value when an icon arrives with flags 0 and the port doesn't, but that's unobservable (the draw needs flags ≠ 0) (re-checked 2.1 ✓) |
| `PwUpTime`, `RoundTime` | 0x1004b9b0.., 0x1004ba50 | 0x10087c60, 0x10087cd0 | match (re-checked 2.1 ✓) |
| `SrvSett` (key 2 = ammocount column) | 0x1004b840 → 0x100072c0 | 0x100878e0 → 0x1001d970 (rewrites the ammo maxima) | match: the 2.1 values equal `ammo21.toml` `max_carry`, and 0/≥4 → column 0 |
| `TsBTime`, `TsPAward` (1..64), `ActItems` (0..32), `Wires`, `ClipInfo`, `WStatus` (2-byte → count 1, translated by the server) | 0x1004b.., 0x1004f.. | 0x100879a0, 0x10087860, 0x1008d620, 0x1008d730, 0x1008d0c0, 0x1008d4c0 | match |
| `WeaponInfo` (grid, kung-fu hooks, `cl_thirdpersonswitch`) | 0x1004f7e0 | 0x1008d1a0 | match (hooks #111/#178) |
| `KFuPower` | 0x1004fb00 | 0x1008d000 (0..100) | match (documented: the port clamps ±100, and the bar never goes negative) (re-checked 2.1: `READ_BYTE`, clamp 0..100 ✓) |
| `Objective`/`ShowObj` (2.1 `DelObj` → `ShowObj 0`) | 0x1004bb70.. | 0x10087d50, 0x10087ed0 | match |
| `TsMessage` handler + `add_ts_message` | 0x1004c0a0, AddTSMessage 0x1004b520 | 0x100882e0, 0x100873d0 | 3.0 match. **2.1 MISMATCH #230**: 63 characters, no name highlight, bottom at h−32, 6 px per character |
| `TsSlowMo` goslow | 0x1004bef0 (eased scale == 1) | 0x100881e0 (previous raw factor == 0) | **2.1 MISMATCH #230 (comment)** |
| `Spectator` (local: buy menu, third person, spread) | 0x10065250 (`cl_thirdperson`) | 0x100b28c0 (no `cl_thirdperson`: always leaves third person) | 3.0 match; **2.1 MISMATCH #230** |
| status Reset | 0x10049e70 | 0x100854a0 | match (re-checked ✓; 2.1 also clears `+0xfc0`, the `GloSlow` icon flag, which no server sends) |
| status Draw order (`draw`) | 0x10049ea0 | 0x10085560 (no flag panel; the 2.1 server spawns no `ts_flag`) | match |
| `draw_round_timer` | 0x1004a130 (×0.5f) | 0x10085970 (/2.0) | match (re-checked both: tick 1 s / 0.1 s in mode 1, frozen in mode 2, red below 60, flags 3 / 0x23 ✓) |
| `draw_ts_messages` | 0x1004af40 | 0x10086ec0 | match. Round 4: **#279** (2.1 stores the min(1, t) factor to a float first, `0x1008700d`: one colour step) and **#280** (the 10 s cap compares the unrounded difference), fixed |
| `draw_teammate_markers` | 0x1004ab40 | 0x10086950 (teamname strcmp, alpha 235·½ = 117) | match |
| `crosshair_trace` (crosshair ID) | 0x1004acc0 | 0x10086b00 | match |
| `draw_powerup` | 0x1004a2e0 (titles) | 0x10085c00 (hard-coded "Slow-Motion"…) | 3.0 match; **2.1 MISMATCH #230** |
| `draw_objective_markers` | 0x10049fc0 | 0x10085700 | match, except both read `gHUD.m_vecOrigin` z (`0x100cf960`, 2.1 `0x102da408`): **MISMATCH #269 (both rulesets), fixed in round 4** (also the `goslow.wav` origin) |
| `draw_status_panel` / `status_look` | 0x1004a3e0 | 0x10085d90 | match (hook, #111) |
| scoreboard `status` (award titles) | 0x1005e213 | 0x100a4670 ("KILLER", "DEMOLITION") | 3.0 match (re-checked colours 125/135/145/155/175 alpha 5 ✓); **2.1 MISMATCH #230** |
| hud/weapons.rs `draw` (order, nohud, buy menu, wires) | 0x1004d3f0 | 0x10089d90 | match except the weapon list (next row) |
| `draw_weapon_list` (closed equipment list, open slot menu, clip text, clip bar) | 0x1004e030 | 0x1008af40 (open only; no clip text or bar) | 3.0 match; **2.1 MISMATCH #207** |
| `select_slot` | 0x1004f220 | 0x1008c6f0 (slot 10 → (1,1), fastswitch bucket-1 start, row −1) | 3.0 match; **2.1 MISMATCH #207** |
| `select_next_prev` (+0.1 s throttle) | 0x1004f500, 0x1004fce0 | 0x1008cc40 (always reloads the held weapon's cell), 0x1008d9c0 | 3.0 match; **2.1 MISMATCH #207**. Round 4: the throttle adds the double 0.1 (`0x1004fd1f`), **#280**, fixed |
| `draw_laser_dots` | 0x1004e6f0 | 0x1008b410 (red, alpha by distance, none ≥ 4096) | 3.0 match; **2.1 MISMATCH #204**. Round 4: the dot position is `WorldToScreen`'s unrounded `(x + 1) × w × 0.5`, **#280** (both), fixed |
| `draw_crosshair`, `crosshair_parts`, `crosshair_gap`, `divesteps` | 0x1004e9f0, 0x1004ef50 | 0x1008b960, 0x1008c1c0 | match (hooks; #110, #153 fixed: `IsHardware()`). Round 4: the divestep pitch dot is worked out in double (3.0 `× π/180`, 2.1 `/ 180 × π`), **#280** (both), fixed |
| weapon `Think` parts, kung-fu regen, Reset | 0x1004ce10 | 0x100899f0, 0x10089890 | match (hooks #193/#194) |
| `weapon_panel`, `kungfu_label` | 0x1004d7e0, 0x1004d700 | 0x1008a320, 0x1008a0f0 | match (hooks #176/#111) |
| effects/laser.rs `draw_local_laser` | 0x1003bb00 | 0x10071bd0 (`laserbeam.spr`, grey flicker, half-width 1.5) | 3.0 match; **2.1 MISMATCH #204**. Round 4: the jitter (`× 0.003` double) and 2.1's flicker round once, **#282** (both), fixed |
| `draw_other_lasers` | 0x1003d600 | 0x100731a0 (origin+24, pitch ×−3, white dot only) | 3.0 match; **2.1 MISMATCH #204**. Round 4: the angle jitter (`× 0.05` double) rounds once, **#282** (both), fixed |
| `draw_local_flashlight` | 0x1003b250 (cull origin, light only on a hit at `endpos`, flash r6/r4 colour 0.7) | 0x10070f20 (same as 3.0) | **MISMATCH #203 (both rulesets)** |
| `draw_other_flashlights` | 0x1003b750 (V_GetInEyePos) | 0x10071630 (origin+24, pitch ×−3) | 3.0 match; **2.1 MISMATCH #204** |
| `draw_shadows` | 0x10055760, 0x10055500 | 0x10095260, 0x10094e70 (no fade or 200 cut, flags 2, mode 2) | 3.0 match; **2.1 MISMATCH #204** |
| effects.rs `add_entity` (hemisphere, occlusion, item cull/fade, LOD body, spectator tail) | HUD_AddEntity 0x100447f0 | 0x1007b4c0 (no item fade; tail HLTV-only, 2.1 main mode 4) | 3.0 match; **2.1 MISMATCH #244** |
| `lod_body` | 0x10056fc0 | 0x100967d0 | match (re-checked 2.1 ✓) |
| `on_tsfrags` / `fx::explosion` / sparks | 0x1004bd50, 0x100481b0 | 0x10088020 → 0x100821e0 (smoke + flash + concrete debris, raw-factor pitch), type 2 scintille2 burst | 3.0 match; **2.1 MISMATCH #232** |
| `fx::glass` | 0x10047cb0 | 0x10081840 (own 9×9 glassfrag) | 3.0 match per round 1. **2.1 MISMATCH #232 (validation comment)**: compared field by field. 2.1 has no hit/force inputs, velocity `(±9, ±9, 0..10)` (the port, fed zeros by the 2.1 server, throws every shard toward +x+y+z at 0..15), gravity 6, life 0.5..2.5, spin `vel.x × 10` |
| `fx::blood` / `blood_count` (PTakeDam) | 0x10048840 | 0x10087f60 → 0x10082cd0 (kevlar: puff only; count clamp(a·0.2, 5, 17), no `cl_gore`; size 3) | **2.1 MISMATCH #232 (comment + validation comment)**; 3.0 spot-checked (count, kevlar puff). Validation: the comment's "size 3.0" is the gravity (`+0x44`); the radius is 1.0. All drops start at the hit point with velocity `(dir ± 0.1) × 16..48 × (30 %: −1) + z 0..8`, life 0.1..1, fade 0.01 |
| effects/scope.rs `draw` | 0x10054510 | 0x10094300 | match (gates, size `max(fov·0.1, 1)/2`, 10 units ahead) |
| view.rs `calc_refdef` dispatch / `spectator_refdef` | 0x10069b90 | 0x100ba6b0 | match (hook #183) |
| `calc_normal`: local translucency (`cl_thirdtransparency`) | 0x10067890 start | none in 0x100b6c90 | 3.0 match; **2.1 MISMATCH #240** |
| `cam_base` (`cl_camtype`) / `cam_smooth` | 0x10067f53.., 0x10068139 | 0x100b7700.. (cam_back/right/up only; int offsets ±ftol(ft·100)) | 3.0 match; **2.1 MISMATCH #240** |
| `calc_view_roll` / `view_roll_target` | 0x100675b0 | 0x100b6910 (V_CalcRoll(0.5 or 30 for a dive, 10), not eased, also on the ground) | 3.0 match; **2.1 MISMATCH #240** |
| `add_idle` | 0x10067550 | 0x100b6820 | match (re-checked: 2.1 multiplies sin × scale × level, 3.0 sin × level × scale, both on x87; same float result in practice ✓) |
| third-person trace / `body_cam` / `smooth_model_z` | 0x10068255.., 0x10068480 | 0x100b6c90 body | match |
| `view_bob` (2.1 `view::calc`) | V_CalcBob 0x10066e70 | 0x100b5c80 | match (#164 fixed; re-checked the peak branch, phase and camera bob) |
| `eye_origin` / `body_cam_z` | 0x100679ee.. | 0x100b6b80 | match (hooks #167) |
| `fade_color` slow tint | V_CalcRefdef tail | none | match (hook #152) |
| `apply_kick` | 0x10057ed0 | 0x10097de0 | match (round 3, `client-display.md`) |
| input.rs `key_event` (anti-script gate, 'a' → slot 11) | HUD_Key_Event 0x10025a80 | 0x10050170 (viewport only) | 3.0 match (#144 fixed: the gate opens on press and release). 2.1: slot 11 is a no-op in 2.1 mode (buy `max_key` 10, no weapon slot 11): match in effect |
| input `+`/`-` commands, `attack_switch`, `respawn` tap, HLTV buttons | 0x10025da0.. | 0x10050490.. | match (hooks, #177) |
| `button_bits` | CL_ButtonBits 0x10026870 | 0x10051730 (same 15 bits incl. 0x40, forced +score dead/intermission) | match (re-checked 2.1 ✓) |
| spectator `in_eye_position` | 0x10068e20 | (2.1's own in `ts21-client` spectator, #184) | match (#153 fixed: name check) |
| spectator panel / nohud | 0x10063870 | 0x100af0c0 | match (hooks #185) |
| events: fire/akimbo/pump/impact/melee/reload/empty sound and particle hooks | 0x1003dfa0.. | 0x10073b70.. | Validation re-traced 2.1: fire pitch 0x100738a0 ✓, event sound order/values (empty 0x10078894) ✓, impact particles 0x10084240 (every material: counts, radii, spread, life, gravity, smoke rates 20/30/50, scintille 48, mark 0.75/0.015/0.25) ✓, blade hits 0x1007a6ad/0x1007aff7 ✓, melee reaches (close combat −56/48/38, katana back 54 subtracted) ✓. **2.1 MISMATCH #251**: the ricochet pitch (0x1007ba4c..0x1007ba70) stores `s × 300` to a float before `+100`; the port's `events::ricochet_pitch` doesn't, so it's one lower for 110 of 255 factors |
| client bullets | 0x10035.. | 0x1006a070.. | match. Validation re-traced 2.1: queue test 0x1007c71e (`realbullet == 1` or factor ≠ 0, NaN queues) ✓, frame throttle 0x1006a0f0 (`now > next`, `next = time + 0.1`) ✓, speed factor (raw / 1, ÷5 in bullet time) ✓, step `dir × vel / 0.25 × s` ✓, trace hull 2 / `PM_STUDIO_BOX` / solid players ✓, range index (exit `/300`, else `/((int)unit / 0.1f)`, float-stored, ×10) ✓, sound → decal → pierce → `> 5` → `min(left, 48)` → `CONTENTS_SOLID` → exit bullet and back trace ✓, draw step order ✓. The step order differs (2.1 steps the bullet then sets `next`; the port sets `next` first), which has no effect because 2.1's step ignores `dt` |
| buy menu rules/text/cash | 0x10036200.. | 0x1006aa70.. | match (hooks #111, #108) |
| death notices, MOTD | 0x1001b360, 0x10064f60 | 0x10040540, 0x100b1f40 | match. Validation compared 2.1's death notice `Draw` 0x1003ff80 (nohud gate, expiry/clamp, layout, weapon text in (1, 0.5, 0.2), skull (10, 240, 10) for a team kill) and `MsgFunc_DeathMsg` 0x10040540 (entry fields, console lines, gluon/tau names) ✓ |
| scoreboard `sort_players` / `sort_teams` | 0x1005d8f0, 0x1005d6a0 | 0x100a4190, 0x100a3c70 | match (validation, new row: most frags then fewest deaths, team sums only when `m_Teamplay < 2` and unoverridden, ping/loss averaged, spectators header 2, blank row 3) |
| scoreboard `rebuild_teams` / `update` / `team_score` | 0x1005da20, 0x1005d5f0, 0x10065120 | 0x100a43d0, 0x100a3b30, (TeamScore hook 0x10045c10) | match (validation, new row: empty-slot reuse, `strncpy(.., 16)`, cleared empty slots, the 0x4a-row reset, teamplay 0 → players only) |
| scoreboard `fill_grid`, `layout` columns | 0x1005db70, `g_ColumnInfo` 0x10096730 | 0x100a4670, 0x1028a944 | match apart from the awards (#230) (validation, new row: identical column table and team colours 0x1028aee8; fonts, row heights, team header underline, local row (white on team colour 196), killer row fade, alignments 4/3/5, "(%d %s)" small font, team name `viewport + 0x8bc + n·0x40`, voice column; `CLabelHeader::setText` 0x100a67b0/0x100a6690 strips trailing whitespace and sums the char widths as 3.0's 0x1005efc0) |
| menus/team.rs `TeamMenu` (`update`, `slot_input`, `set_active_info`, `initialize`, `reset`) | 0x10066020, 0x100665a0, 0x10066690, 0x10065ff0, 0x10066730 | 0x100b3eb0, 0x100b4810, 0x100b4990, 0x100b3e40, 0x100b4b60 (vtable 0x1027844c) | match (validation, new row: buttons 1..4 from `TeamNames`, auto-assign 5, cancel, no spectate button (ctor 0x100b2ca0 leaves `+0x11c` 0), player lists `strncat` 0x43d/0x53d, "%s: %d Player(s) (%d points)" from `g_TeamInfo[n].frags`, map name / `maps/<map>.txt` briefing). Constructor layout compared by structure only |
| menus `vgui_menu_message` / `show_vgui_menu` / queue | 0x10064f00, 0x10064050, 0x10064280 | 0x100b1e30, 0x100afd80, 0x100b0140 | match (validation, new row: map name 64 bytes for menu 4, demo and intermission gates (MOTD allowed), duplicate check, team menu only in teamplay, class menu 3 pre-empted by 2) |
| menus `key_event` | KeyInput 0x10064b90 | 0x100b1760 (2.1's `HUD_Key_Event` 0x10050170 calls only this) | match (validation, new row) |
| hud/notice.rs `draw_menu` (`CHudMenu::Draw`) | 0x100285f0 | 0x10054520 | match (validation, new row) |
| HUD element list; SDK health/battery/ammo/flashlight/secondary-ammo `Draw` stubs | `CHud::Init` 0x10020170 | 0x10045d50 (same 17 elements; stubs 0x10044170, 0x1003e4d0, 0x1003c8e0, 0x10043560, 0x1003d7b0 return 1) | match (validation, new row) |
| hud/mod.rs `redraw` (intermission, `hud_takesshots`) | `CHud::Redraw` 0x10021aa0 | 0x10047b70 | **MISMATCH #252 (both rulesets)**: TS takes the snapshot once `(float)ftol(time + 1) < time`; the port takes it in the first intermission frame. The rest (menus hidden, scoreboard, element gates) matches |
| effects.rs `apply_cvar` / `adapt_quality` (`cl_fx` tuning) | FX_ApplyCvar 0x10057d50, FX_AdaptQuality 0x10057a70 | 0x10097c20, 0x100977a0 | match for the tuning values (validation, new row: 1.0/0.35 scale, 0.05/0.2 and 0.05/0.3 intervals, 1.5/0.7 count, 30/5 shell life, 4096/2048; adaptive `min(0.05/q, 0.3)`, `max(2q, 0.1)`, `max(30q, 2)`, `max(q, 0.25)`). 3.0's pool divisor and physics switch belong to the particle system, which isn't traced for 2.1 |

## Not verified / gaps
- 2.1 particle *update*: traced in round 3 (**#261**, fixed; **#267**, the follow field; `client-display.md`).
- `apply_kick` against 2.1's `0x10097de0`: compared in round 3 (match).
- ~~2.1 team menu constructor (`0x100b2ca0`) layout constants and `paint` were compared by structure only. MOTD window (`CMessageWindowPanel`) not compared for 2.1.~~ Closed in round 3 ([client-display.md](client-display.md) GUI: same constants; 2.1 divides by 640/480, #266).
- SDK HUD elements TS left alone (`CHudMessage`, `CHudSayText`, `CHudStatusBar`, `CHudStatusIcons`, geiger, train, `TextMsg`): compared against 2.1 in round 3 (`client-display.md`; **#260**, fixed).
- 2.1 client bullet draw (`0x10069540`: models, trail spin, spiral): traced in round 3 ([client-display.md](client-display.md) "Model rendering"); one mismatch, #259.
- ~~3.0 `draw_objective_markers` compares against the local entity's origin z, where TS reads `gHUD` `m_vecOrigin` (`0x100cf960`). The two differ by prediction lag only; not filed.~~ Round 4: **#269** (both rulesets; the `TSSlowMo` sound too), fixed: `Hud::vec_origin` is set from `cdata->origin` in `HUD_UpdateClientData`.

## Validation (second pass, independent)
- Checked: all seven filed issues exist, are open, are labelled `bug`, and describe the rows that cite them. Port line references spot-checked: `hud/state.rs:858-873` (#230), `effects.rs:546-593`/`571-575` (#244), `effects/fx.rs:610` (#232). Cited original addresses spot-checked against the decomps: 0x100877b0, 0x10087ad0, 0x10085970, 0x100967d0, 0x10051730, 0x100a4670, 0x10081840, 0x10082cd0, 0x100b6820, 0x1008d000, 0x100854a0. All correct.
- Re-checked 14 "match" rows against the decompile and disassembly: `TsHealth`, `TsCash`, `PwUp`, `PwUpTime`/`RoundTime`, `KFuPower`, status Reset, `draw_round_timer`, `lod_body`, `button_bits`, `add_idle`, the scoreboard status colours, death notices, and both hook rows (#160 events, #165 bullets) line by line, plus the `fx::glass` and `fx::blood` rows for 2.1 (16 in all). **Wrong: 2.** The #160 events row hid the ricochet rounding (**#251**). The `fx::glass` row ("not compared") hid 2.1 glass differences (commented on #232). Also corrected: the #232 blood comment's "size 3.0" is the gravity.
- Gap areas covered: 2.1 glass and blood (above), 2.1 scoreboard sort/grid/rebuild/update (match apart from #230), 2.1 team menu, VGUI queue and key input (match), and the #160/#165 hooks (one mismatch, #251). HUD modules TS changed, for both rulesets: element list and SDK draw stubs, `CHudTSStatus`/`CHudTSWeapons` (rows above), the death notice, `CHudMenu` and `CHud::Redraw`. That last one gave **#252** (both rulesets).
- Missing rows added: 10 (scoreboard ×3, team menu, VGUI queue, key input, `CHudMenu`, element list, `redraw`, FX tuning).
