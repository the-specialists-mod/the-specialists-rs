# Client display audit, round 3 (3.0 + 2.1)

Round 3 covers the client's graphical display and GUI, checked function by function against TS 3.0's and TS 2.1's `client.dll`. It compares draw order, positions, sizes, fonts, colours, alpha, timers, rounding and gating, and the messages and commands each part handles or sends. Each group has its own section, in the format of [client.md](client.md): port function | 3.0 original | 2.1 original | status. The scripts are in `scripts/round2/` (`fn.sh`, `dis.sh`, `rd21.py`). Issues filed this round are linked from the rows that cite them, and each was fixed in the same round.

## GUI

Scope: `crates/client/src/menus/{mod,buy,motd,team,scoreboard,scroll,widgets}.rs`, `hud/vgui.rs`, `spectator/{panel,overview}.rs`, and the GUI hooks in `ts30-client`/`ts21-client`. Issues: **#262, #263, #266** (2.1), **#265** (3.0), **#264, #268** (both). All are fixed in this round.

| Port function | 3.0 original | 2.1 original | Status |
|---|---|---|---|
| menus/mod.rs `draw_buy` dispatch (team page alone, else summary + page, children before parents) | 0x10036750 | 0x1006b5b0 | match |
| `Column::row`/`row_scaled` (127×19 box at `ftol(50·scale)`, text `ftol(c·scale)`, `DrawHudString` at the row, y += 20) | 0x1004fed0, 0x1004ff10 | 0x1008f200, 0x1008f270 (unlocalised text) | match |
| team page ("Change Team:" green, two `chgteam` rows at HUD/2, cancel (100,100,205)) | 0x100367e0 | 0x1006b6a0 | match |
| main page (Empty/Use Last/Use New config red/green by `cash − total < 0`, rows 2..7, cancel only on page 0; scale 1.0/0.5) | 0x10037540 | 0x1006ca30 ("7 Buy Random") | match |
| category page (`table[0][i] != −1`, chosen category full brightness `0x1003788c`, "6 Player Accessory" 3.0 always / 2.1 `m_Teamplay > 1`) | 0x100377d0 | 0x1006cde0 (32×32 table, stride 0x80) | match |
| weapons page (`%c` row ≥ 9 → 'A' / 2.1 `%i`; affordable = cost and slots; chosen weapon full brightness `0x10037ad4`; weapon box at the chosen row) | 0x10037960 | 0x1006d070 | match |
| options page (Buy, attachments 900/800/500/1800 with own price, akimbo / full load, back) | 0x10037ba0 | 0x1006d470 (pending weapon at `+0xc`) | match |
| items page (Kevlar 1900 / Stealth Shoes 700; 3.0 money+slots, 2.1 money only) | 0x10037fe0 | 0x1006db50 | match |
| remove / save / load pages | 0x100373c0, 0x10037260 | 0x1006c780, 0x1006c540 | match |
| `draw_weapon_box` (fill 0xff×0x2f alpha 0x19 at (0x108, h/2−0x30), sprite, credits/slots at +128) | 0x10038180 | 0x1006ddd0 | match |
| `draw_summary` (bar 0x280×0x13, header red at `cash − total ≤ 0`, free slots, 5 buckets, sprites, numbers flags 2/5, attachments, akimbo second gun) | 0x10036af0 | 0x1006b9a0 | 3.0 match; **2.1 MISMATCH #262** (item names HUD/2, never red), fixed (`MenuRules::summary_items_dim`) |
| `attachment_summary` | 0x100368f0 | 0x1006b830 | match |
| buy.rs `handle_key` (all pages) | 0x10038840 | 0x1006e830 (keys 1..10; pending weapon reset to 0, not −1: unobservable) | match |
| `hit_test` | 0x10038410 | 0x1006e1b0 (hole message) | match |
| `mouse_buttons`/`click` (left release: hit or key 1; right release: key 10) | 0x10038360 | 0x1006e080 | match |
| `add`, `remove`, `recompute_ammo`, `add_ammo_buy` | 0x10038f60, 0x10038e40, 0x100392b0, 0x10039380 | 0x1006f5b0, 0x1006f340, 0x1006fcb0, 0x1006fe30 | match |
| `submit` (sort, `rst_cfg`/`tkw`/`tki`/`tka` order, messages) | 0x10039400 | 0x1006ff20 (strings 0x1026b144..) | match |
| `random_config` (melee, pistol/SMG/rifle branches, RNG call order), `random_attachments` | 0x100398e0, 0x10039810 | 0x10070860, 0x10070720 | **MISMATCH #264 (both)**: `p` against `double` bounds; fixed |
| `open_buy`/`BuyMenu::open` | 0x10036670 (demo refused) | 0x1006b3f0 (no demo check) | 3.0 match; **2.1 MISMATCH #263**, fixed (`MenuRules::refused_in_demo`) |
| `BuyMenu::close`, `reset_config` | 0x10036720, 0x10038800 | 0x1006b530, inline `memset` | match |
| `restricted`, `cash`, `SLOTS` | 0x100365e0, 0x10036640, 0x10036660 | 0x1006b330 (ids 1..63, no Golden Colts case: no menu id is affected), 0x1006b390, 0x1006b3d0 | match |
| team.rs `TeamMenu::new` (constants 40/32, 176/80/424/312, 16/16, scroll 16/48/424−32/178, briefing 424−16×178, buttons 40/80+32i/124×24, info 16/234/424−16/424−234, colours, fonts, "Auto assign", `#Menu_Cancel`) | 0x100655d0 | 0x100b2ca0 | match (same constants and structure). 2.1 divides by 640/480 and spills nothing: **2.1 MISMATCH #266**, fixed (`ClientRules::vgui_scale`) |
| team.rs `update`, `paint` (`paintBackground` → `Update` while the map name is unset, black at alpha 100) | 0x10066020, 0x10066640 | 0x100b3eb0, 0x100b4940 → 0x1003a480 | match (round 2 compared `Update`'s logic; this round its geometry) |
| `Open`, `Reset` | 0x10066610, 0x10066730 | 0x100b4900, 0x100b4b60 | match |
| motd.rs `layout`, `Motd::new` (window 112/80/424/312, title at +16/+16 "Title Font" then `sf_primary1`, scroll +16/+48, −32, −(48+60), OK +16 / bottom−16−30, 160×30; shade 100/255) | 0x1005bb90 | 0x100a0ad0 | match in structure; **2.1 MISMATCH #266** (at 640×480 the scroll panel is 204 high and the OK button at y 346 in 2.1, where 3.0 has 203 and 345), fixed |
| `map_briefing`, `for_motd` (`CreateTextWindow` 1/3: titles to 31 characters, `maps/<m_sMapName>.txt` or the level's, `m_sMapName` from the level) | 0x10063c00 | 0x100af6c0 | match |
| scoreboard.rs `layout` (insets 104/40, 512/400 cases, title 27, columns `cw·XRES`, last column `wide − XRES(4) − end`, header `wide − 2·XRES(4)`) | 0x10063a50, 0x1005ce70 | 0x100a2d60 | match in structure; **2.1 MISMATCH #266** (last column 1 wider), fixed |
| scoreboard update timer (`m_fNextUpdateTime = time + 0.5` as a float) | 0x100649aa | 0x100b1210 | match |
| menus `paint_vgui` (viewport `paintBackground`: scoreboard cursor/update, team check, spectator panel key) | 0x100648c0 | 0x100b1210 | **MISMATCH #268 (both)**: no `Update` on a local team change; fixed |
| spectator/panel.rs `draw` (bar `YRES(40)` at `h − YRES(40)`, black alpha 100, "Primary Button Text" `YRES(25)` and "Briefing Text" `YRES(15)` labels, `a_north`) | 0x10064410 | 0x100b04d0 | **3.0 MISMATCH #265** (computed in `f32`, 440 at 480; TS 439), fixed; 2.1 divides (440): #266 |
| panel `update`/`texts`/`think` (`#Spec_Mode%d [: name]`, `#Directed `, `#Spec_TSHelp%d`/`#Spec_Only_Help`, key mode/target/nohud) | 0x10063870 | 0x100af0c0 | match (the `IsSpectateOnly() != 2` test can't fail on these engines) |
| hud/vgui.rs `xs`/`ys`/`spill` | `* 0x1007d43c`, `* 0x1007d438`, spills | `/ 0x1026f548`, `/ 0x1026f544`, no spills | was 3.0 for both rulesets; now per ruleset (#266) |
| vgui.rs `Schemes::load` (resolution table, `gfx\vgui\fonts\%d_%s.tga`, font sharing by name/size/weight, defaults) | 0x1005c300 | 0x100a16e0 | match in structure (SDK code; both clients load the same Half-Life fonts and schemes) |
| spectator/overview.rs `draw_entities` name-tag projection (`(x+1)·w·0.5`, `(1−y)·h·0.5`, offset (32, 32), z of the offset −0, tag y + length) | 0x10024160 | 2.1's own (`ts21-client` spectator, #181) | match. The second projection's off-screen result: TS uses it anyway, while the port takes offset 0 (a tag at the icon's own y in that rare case; not filed, HLTV only) |

### Not verified
- Overview layer tiling (`DrawOverviewLayer` 0x10023c90) and the entity quads/beam: structure only. Ghidra's output for 0x10024160 is garbled, and only the name-tag store was traced in the disassembly. The port computes the tag projection in `f32` where TS keeps 53-bit before storing a `float`, which can differ only in the last bit.
- Scroll bars (`menus/scroll.rs`, `CTFScrollPanel`) and `CommandButton` painting (`widgets.rs`): not compared against 2.1 (SDK code in both).
- Voice labels (`voice::redraw`): not in this group's comparison.
- `jointeam` setting `g_iTeamNumber` locally (`0x10064a82`) is still not emulated (documented in client-hud.md). #268's team check reads the player state.
- 2.1's VGUI scaling was brute-forced over every width 320..3840 and height 240..2400 for the MOTD expressions, and at the common resolutions for the rest. Expressions not compared operation by operation (the scoreboard's `FillGrid` row heights) take the same `vgui_scale`.
