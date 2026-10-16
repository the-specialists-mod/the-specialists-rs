# Weapons audit (3.0 + 2.1), round 2

Round 2 (Oct 16 2026, after the #200 split, port at 661d918 + 14240b0; validated against main 5032209, which adds #201's `GunRules::pre_think` and #198's 2.1 plugin callbacks): weapons, weapon tables, client prediction, buying and weapon selection. Melee, explosives and bullet damage/penetration are the damage area; the 2.1 melee end-time representation is #202 (being fixed elsewhere).

Issues filed this round: #208 (3.0+2.1 fire-mode memory cleared on every spawn), #211 (3.0 corpse ammo pack owner), #212 (2.1 `customweapons`), #214 (2.1 `CTSGun::UpdateClientData`), #215 (2.1 `lasclr_`, #200 phase 2), #225 (2.1 grenade throw velocity sums). Validation added #247 (spawn resets: grenade-buy count, `m_afButtonLast`, last delayed-hit victim, a kept gun's timers), #248 (client keeps `CustomWP` values across `HUD_VidInit`), #249 (PreThink: `CheckForBreakables` before the window step/closure).
Round 1 issues (#129 #130 #134 #141 #146 #158 #159 #161 #166) are all closed. Their fixes were re-read where the code moved. #201 is fixed on main (14240b0); its new code is checked below.

Scripts: round 1's `scripts/cmp_weapons.py` (pointed at `crates/ts30/data/weapons.toml`) and `scripts/cmp21.py` (run from `~/dev/hl`), rerun against the new locations: no differences.

"R2v" marks a row the validation re-checked against the decompile/disassembly itself.

## Moved code (#200)
| Port (now) | Was | Check | Status |
|---|---|---|---|
| ts30/src/gun.rs `fire`, `standard_fire`, `shotgun_fire`, `akimbo_fire`, `scale_spread`, `vkickback`, `decay_spread`, `better_weapon`, `PRIORITY` | shared/src/weapons/gun.rs `fire_core` etc. (d91088e^) | text diff, `self.`→`gun.` only | verbatim move |
| ts30/src/gun.rs `GunRules` bodies (tables, semi_auto_delay 0.1, timer_step, command_frame_time, decay_spread → true, pellet_offset None, pellet_dir, akimbo ±5, predicted ALL, cancel needs stock, blades/katana/infinite clip/holster) | shared/src/gameplay/gun.rs default bodies | text diff | verbatim move |
| ts30/src/gun.rs `pre_think` / `pre_frame` (14240b0, #201) | shared `Gun::pre_frame` + server `weapons::pre_think` | text diff | moved; checked below (PreThink rows) |
| ts30/src/recoil.rs `shot_interval`, `grow_spread_penalty`, `view_kick`, `decay_spread_penalty` | shared/src/weapons/spread.rs | text diff | verbatim move |
| ts30/data/{weapons,ammo,assets,combat}.toml | shared/data | `git diff -M`: renames, 0 lines | match (cmp_weapons.py clean) |
| ts21/data/*.toml | — | comment lines only since round 1 | match (cmp21.py clean) |

## Tables
| Port data | 3.0 original | 2.1 original | Status |
|---|---|---|---|
| ts30/data/weapons.toml (36 ids x 25 fields, modes, zoom, hud, kind), anims.rs | Write_* + Sparta() | — | match (script) |
| ts30/data/ammo.toml | compile_ammo_info, Set_ammo_count | — | match (round 1, unchanged) |
| ts21/data/weapons21.toml, ammo21, assets21, combat21 | — | FUN_10049890 writers, 0x10049420, 0x100492a0 | match (script) |
| table.rs `with_weapon_params` (`accuracy.max(0.001)`) | adjust_weapon_value 0x9eca4 (`acc < 0.001 ? 0.001`) | — | match except NaN (TS keeps NaN, port 0.001) and `%f` "nan"/"inf" (glibc reads them, port's `scan_float` makes the line Bad): edge only, not filed |
| table.rs custom-table lifetime on the client (`set_ruleset_tables` drops customs only on a ruleset change; `reset_custom_tables` never called on the client; hud/state.rs:738 `Msg::CustomWp`) | client `FUN_10007de0` (zero + rebuild the table) from `CHudTSWeapons::VidInit` 0x1004cb50 and the gun init `FUN_100036f0` after each `HUD_VidInit` (flag `0x1009ff08`, cleared by 0x10065ff0) | client `FUN_10017d30` → `FUN_1001e270` (memset + `Write_*`), flag `0x10297b20` cleared by `FUN_100b3e40` | **R2v: mismatch #248** (both): the port's client keeps `CustomWP` values for the session |
| mod.rs helpers `mode_mask` / `default_mode` / `is_restricted` / `restriction_type` / `body_by_items` / `range_multiplier` | info+0xc, TakeWeapon, IsRestrictedWeapon, GetBodyByItems | 2.1 equivalents | match (round 1); not re-checked |
| random.rs `shared_random_long` / `shared_random_float(_unrounded)` | SDK UTIL_SharedRandom* | FUN_10066640 | match (round 1, range ≥ 0); not re-checked |

## Shot functions and recoil
| Port function | 3.0 | 2.1 | Status |
|---|---|---|---|
| ts30 `standard_fire` | StandardFire 0x824a4 (re-read) | — | match |
| ts30 `shotgun_fire` / `akimbo_fire` / `scale_spread` / `vkickback` / recoil.rs | 0x82108 / 0x829c0 / 0x802a8 / 0x80414 | — | match (round 1; moved verbatim) |
| ts21 `fire` | — | FUN_1004ac20 (re-read: kind byte 4/5 akimbo, 1 shotgun; anim only when DAT_100b1df8 >= 0) | match |
| ts21 `shotgun_fire` | — | FUN_1004b050 (re-read) | match |
| ts21 `standard_fire` / `akimbo_fire` / `empty_sound` / `scale_spread` / `vkickback` / `slow_div` / `scale_delay` / `rate_delay` / `burst_delay` / pellets | — | 0x1004b400 / 0x1004b8c0 / 0x1004ab60 / 0x10049a50 / 0x10049b80 / 0x100551c0.. | match (round 1; code unchanged except the #111 bool return) |
| ts21 `decay_spread` (server + client, crosshair copy) | — | PreThink 0x100537e0; client FUN_10017f70 0x10018d1a..0x10018dae, divisor FUN_10095800→FUN_100957c0 (`flds 0.1f`, NaN kept) | match; **R2v** (server: `0 < pen && last <= time`, `pen − elapsed/slowdiv·0.1` on the FPU, `< 0` tested unrounded, `last = time` every think after the closure; client: same, crosshair copy only inside the branch) |
| ts21 `better_weapon` | — | FUN_1004a410 | match (#162) |

## PreThink weapon step (since #201: `GunRules::pre_think`)
| Port function | 3.0 | 2.1 | Status |
|---|---|---|---|
| ts30 `pre_think` / `pre_frame` / spread.rs `timer_step`, `regen_melee_bar` | PreThink 0x908cc (decomp ~1340..1420; mp.dll 0x1005f5c0): step `GiveSlowMul·frametime (×1.5 DFR)`; window `−step` floored 0.001; blocks `−step` floored 0; disarm `−5·step`; alive: penalty `−0.1·step`, bar `+15·step` (`+step` below −0.001), clamp 100; closure `< 0.01 || dmg == 0`, `+0x7a8 = 0` while open | — | **R2v**: values match. Order: **mismatch #249**: TS closes the window before `CTSStunt::CheckForBreakables`; the port's `PRE_THINK` (player/mod.rs:259) runs the stunt step (check_for_breakables, stunt/mod.rs:465) before the weapon step, so the delayed hit gets one more frame |
| ts21 `pre_think_step` / `regen_melee_bar` / `slow_div` | — | PreThink 0x100537e0 (decomp 58..113): alive: penalty, bar `elapsed/slowdiv·15 + bar` on the FPU; closure `end < time || dmg == 0`; `+0x6f0 = time` | **R2v**: match for the bar and penalty; window/blocks as countdowns = #202 (known); closure order = #249 (2.1 closes at line 105, `CheckForBreakables` FUN_1005c680 at 172); no disarm in 2.1 (port ignores the count) |
| server mod.rs `pre_think` (pressed edge, `WeaponPreThink`) | PreThink head 0x908cc (`m_afButtonPressed` from `+0x810`) | 0x100537e0 (`+0x76c`) | match per frame; spawn reset of the last buttons = #247 |

## Core gun state machine (shared/src/weapons/gun.rs, both rulesets)
| Port function | 3.0 | 2.1 | Status |
|---|---|---|---|
| `post_frame_ready` / `item_post_frame` gate + reseed | 0xdc4d0 | FUN_1002d840 (re-read: tank, next_attack, no item, +alt2 && in_reload==1, reseed for any item) | match |
| `weapon_post_frame` | 0xa03d4 | 0x10064ce0 | match (round 1) |
| `primary_attack` / `secondary_attack` / `alternate_attack` | 0x81a20 / 0x817a8 / 0x81ba8 | 0x1004aa50 / 0x1004a800 / 0x1004ab40 | match (#141 fixed) |
| `zoom` / `unzoom` | 0x81710 / 0x81748 (re-read) | inlined | match (Unzoom tests TS's info pointer, port `has_slot`: differs only for a dead player's holster, whose view model RemoveAllItems clears); **R2v** (`info+0x3c < 0x28` hides the view model; Unzoom restores `info+0x132`) |
| `switch_fire_mode` | 0x8169c (re-read; TS loops forever on a gun with no mode bits, port guard) | FUN_1004a790 | match; **R2v** (TS tests bit `mode+1` before wrapping at > 5, so a mode-5 gun tests bit 6 first: no table has mode bits above 5, unobservable) |
| `switch_items` (+ switch.wav vol RF(0.92,1), attn 1.25, pitch 98+RL(0,3)) | 0x8155c | FUN_1004a6c0 | match; **R2v** both (all four owned/active cases; 2.1's `EMIT_SOUND_DYN` has no slow-motion argument, port's `sound_pitch`) |
| `select_weapon` / `switch_to` | 0x80fb8 | FUN_1004a4c0 (ids < 64, name byte) | match; **R2v** 3.0 (`id != current && id < 0x26 && ((owned && name) || id == 0)`, then HolsterWeapon) |
| `deploy_weapon` | 0x81030 | 0x1004a530 (re-read: mode memory pl+0x41e, no draw event, anim 0x33) | match except the draw event and anim 0x33/0x34 (round 1: kept by design, docs/audit round 1 row "Gun::deploy in 2.1 mode"); **mode memory reset: #208**; **R2v** 2.1 server Deploy |
| `holster_weapon` | 0x85698 | FUN_1004d7a0 | match (#130 fixed) |
| `add_item` (CTSGun::AddItem) | 0x80794 (re-read) | FUN_10049e50 (re-read) | match |
| `add_ammo` | 0x808ac (re-read) | FUN_10049f40 | match (saturating vs wrapping add unreachable); **R2v** 3.0 |
| `take_weapon` / `remove_weapon` / `get_better_weapon` | 0x80a04 / 0x8094c / 0x80f40 | FUN_1004a040 / FUN_10049fb0 / FUN_1004a410 | match (round 1) |
| `multiply_clips` | 0x81308 | — (2.1: no MultiplyClips) | match (round 1 + 2327216) |
| `grenade_fire` / `grenade_idle` | 0x832f0 / 0x84f18 | FUN_1004c020 / 0x1004d260 (re-read + disasm) | flow match |
| `grenade_throw` (now gun.rs:921-955) | 0x84f18 / mp.dll 0x10057d80 (disasm re-checked: throw x on FPU, y/z stored; roll k·f.z on FPU) | 0x1004d260 disasm 0x1004d2ac..0x1004d439: same doubles; throw keeps x **and y** on the FPU, roll keeps **x** | 3.0 match; **2.1 mismatch #225** (its file:line 945-978 is now 921-955) |
| `reload` / `reload_weapon` / `shotgun_reload` / `stop_reload` / `weapon_idle` / `ccombat_idle` | 0x84d5c / 0x84a14 / 0x84aac / 0x81288 / 0x854dc / 0x85e54 | 0x1004d0e0 / FUN_1004ce20 / FUN_1004cea0 / — / 0x1004d660 | match (round 1, #141); **R2v** `stop_reload` (record+info+in_reload, class 1 clears state/toggle, idle and `m_flNextAttack` = WeaponTimeBase 0) |
| `decrement_timers` | PostThink tail 0x91ae0.. | PostThink 0x10054180 (decomp 137..183) | match; **R2v** both (primary −1, secondary/alt/idle/fuser1 −0.001, idle and fuser1 skipped at 1000, `m_flNextAttack` −0.001; 2.1 `<=` clamps and raw `frametime`, same results) |
| `charge_melee_bar` / `update_melee_bar` | UpdateMeleeBar 0x9650c | 2.1 equivalent | match (round 1 / damage area); not re-checked |
| `reset_inventory` / server `gun_reset` / `new_gun_state` | CTSGun::Reset 0x805e0, Spawn 0x8053c (clip mirror 1, `+0x9e4`/`+0x9e8` 0) | Spawn 0x10049cb0 (`+0xa4`/`+0xb4` 1, `+0xd90`/`+0xd94` 0) | **R2v** match for a new gun (zeroed entity + Spawn = `Gun::default` at spawn) |
| `reset_for_spawn` | TSInit 0x92a4c (bar 100 `+0x774`, penalty `+0x778`, window/dmg/source `+0x79c..+0x7a4`, blocks `+0x7a8..+0x7b0`, disarm `+0x7b4`, last victim `+0x798`, `m_flNextAttack` 1+base then Spawn 0x8f568 sets it to base) | TSInit 0x100552f0 (bar `+0x6ec`, `+0x6f0 = time`, penalty `+0x6f4`, last victim `+0x714`, window/dmg/source `+0x718..+0x720`, blocks `+0x724..+0x72c`) | **R2v**: player fields match except the mode memory (#208); the gun entity's timers/combos/last shot are zeroed for a gun kept through a spawn (TS keeps them; plugin `Ham_Spawn` of a living player only) and the last victim isn't reset: **#247**. 2.1 `+0x6f0 = time` vs port `decay_time = 0`: no effect (everything is 0 at spawn) |

## Server (crates/server/src/weapons)
| Port function | 3.0 | 2.1 | Status |
|---|---|---|---|
| commands.rs `weapon_`, `lastinv`, `drop`, `drop_`, `tkw`, `tki`, `tka`, `rst_cfg`, `firemode`, `useitems` | ClientCommand 0xad838 | pfnClientCommand 0x1000d4b0 (re-read), FUN_10055950/10055970/100559b0 | match |
| commands.rs `lasclr_` | 0xad838 | absent ("Unknown command") | **2.1 mismatch #215** (#200 phase 2); file:line re-checked (commands.rs:67-73) |
| inventory.rs `pick_up_weapon` | 0x9314c | 0x10055830 | match (round 1, #159 fixed; hook `pick_up_cost`) |
| inventory.rs `drop_weapon` | 0x94900 | FUN_10056080 (re-read: -2 no better, M61 primed/empty no sound, stack 1, akimbo split ±4, pair keep, plain) | match (hooks `dead_grenades_become_powerups`, `stack_drop_count`) |
| inventory.rs `on_killed` | Killed 0x8ed5c | Killed 0x10052390 (holster, DropAmmo FUN_10055ef0, ids 1..63 twice with akimbo, ResetArmament) | match |
| inventory.rs `throw_weapon` | 0x9529c | 0x100564d0 | match (round 1) |
| inventory.rs `take_armor` | 0x93da4 | — (2.1 kevlar sets 100, ts21 cash.rs) | match; **R2v** (10 slots, items.wav, `(float)n + armour` capped 100; NaN armour: TS keeps NaN, port `min` gives 100, unreachable; TS calls `SetPlayerSpeed` every time, port `sync_speed` only on a slot change, same value) |
| inventory.rs `sync_speed` | GetSpeedBySlots + SetPlayerSpeed | 2.1 `speed_by_slots` hook | match (round 1 / movement area); not re-checked |
| ts30 players.rs `buy_armament` / `buy_weapon` / `buy_ammo` / `add_item` | 0x93f10 / 0x945bc / 0x941e8 / 0x9440c (re-read; CashBased 0x76abc returns 0) | — | match (port loop guard on a no-op purchase; kevlar with NaN armour skipped by the port, TS buys: unreachable); **grenade count reset: #247** (TS: RestartRound 0x77920 / InitHUD 0x79134 only; port: every spawn, and 3.0's `restart_round` doesn't) |
| ts21 cash.rs `buy_*`, `add_item` | — | 0x100559d0 / 0x10055e00 / 0x10055a40 / 0x10055c30 (re-read) | match; **grenade count reset: #247** (2.1 resets `+0x68c` in RestartRound 0x100445d0 and InitHUD FUN_100451c0 only) |
| worldgun.rs `drop_ammo` | DropAmmo 0x946f0, CreateAmmoPack 0xa1334 (owner set after Spawn), OnGround 0xa1610 | FUN_10055ef0, CreateAmmoPack 0x100657e0 (no owner) | **3.0 mismatch #211** (owner; position sum order noted there); 2.1 match; file:line re-checked (worldgun.rs:384, owner NULL at :427) |
| worldgun.rs `WorldGun` create/spawn/think/touch/land/give_to/use_by, `blast`, `fall_impact`, `ground_hop`, `set_drop_lifetime`, `glows_for`; `AmmoPack` spawn/think/touch, `take_ammo`; ts21 worldgun.rs `touch`/`touch_breaks` | CreateWeapon/Spawn/Fall 0xa0d84/BounceTouch 0xa0b0c/GetWeapon/Use; CTSAmmoPack Spawn 0xa13f4/WaitThink/OnGround/WaitTouch 0xa16c0 | 0x10065040.., Fall 0x10065550, BounceTouch 0x10065270, WaitTouch 0x10065a90 | match (round 1, #158 fixed; not in the round-2 table); **R2v** `AmmoPack::touch`/`take_ammo` vs WaitTouch 0xa16c0 (IsPlayer, IsAlive, active gun, `AddAmmo` per type, 9mmclip1.wav) |
| net.rs `update_client` / `weapon_info_update` | CTSGun::UpdateClientData 0x858c8 (re-read) | 0x1004d8c0 (re-read; disasm 0x1004d9ca `flds pl+0x6ec; __ftol`) | 3.0 match (clip/reserve clamp vs WriteByte truncation unreachable; bots skipped, unobservable); **2.1 mismatch #214**; file:line re-checked (net.rs:83, :132) |
| net.rs `get_weapon_data` / protocol `pack_weapon_data` | GetWeaponData 0xb00ac (re-read) | pfnGetWeaponData 0x1000f540 (wire = 3.0's) | match (fields CTSGun never sets are 0 in both) |
| net.rs `fill_client_data` / protocol `pack_clientdata` | UpdateClientData 0xb0294 (re-read) | 0x1000f710 (wire = 3.0's) | match, except `sendweapons` is ignored (server-dll `_sendweapons`): TS stops before `m_flNextAttack`/weapon fields when the client has `cl_lw 0`; unobservable (nothing reads them without weapon prediction), not filed |
| host.rs `default_deploy` | DefaultDeploy 0xa0178 | FUN_10064ba0 | match (round 1) |
| host.rs `kick`, `sound`, `alive`, models, events | VKickback/SwitchItems/... | 2.1 equivalents | match (round 1); sounds through `ServerRules::sound_pitch` |
| custom.rs `load` / `save` / `inform` / `cmd_loadweapons` / `cmd_saveweapons` | 0x9eca4 (re-read) / 0x9f044 / 0x9eb14 / 0xc59ac / 0xc5a2c | 2.1 has no commands; `customweapons` polled in Think 0x10043cc0 → adjust_weapon_value 0x10063ed0 | 3.0 match (NaN/inf edge above); **2.1 mismatch #212**; file:lines re-checked (cvars.rs:52, ts21 rules.rs:330, assets.rs:27) |
| mod.rs `post_think` / `item_post_frame_ready` | PostThink 0x91610 | PostThink 0x10054180 | match (dead/intermission skip the frame and the `m_afButtonLast` update, both: **R2v**) |
| mod.rs `ts_init` | TSInit 0x92a4c (`+0x548 = 0x51`, `+0x718 = 0`, SetPlayerSpeed 330) | TSInit 0x100552f0 (`+0x4c0 = 0x51`, SetPlayerSpeed 310 via the hook) | match (**R2v** 3.0 slots/items) |
| mod.rs `on_spawn` | TSInit / Spawn 0x8f568 / PlayerSpawn | same | **R2v**: `m_pLastItem`, disarm, fov match; **#208** (mode memory), **#247** (grenade count, `last_buttons`/`pressed` and the stunt `button_last`: TS keeps `m_afButtonLast` from the death; `lunge_victim` not reset) |

## Client prediction (crates/client/src/prediction/gun.rs)
| Port function | 3.0 | 2.1 | Status |
|---|---|---|---|
| `post_think`, `post_frame_gate`, `write_weapon_data`, `rebuild`, `spawn_and_death`, `deploy`, `holster` | HUD_WeaponsPostThink 0x10003830.. | FUN_10017f70 (whole function re-read, see below) | match (round 1; 2.1 `predicted()` hook); mode memory never reset on the client in both TS clients; **R2v** 2.1 (details below) and 3.0 `write_weapon_data` (disasm 0x10003eb0..0x1000407e: step `fild msec; fmul 0.001; ×GiveSlowMul; ×1.5`, clamps −1 for primary and aim bonus, −0.001 for the rest, no 1000 test) |
| `ClientHost` (fov 90 = 0, arith CLIENT, first_prediction = runfuncs) | client.dll | client FUN_100261b0 (no floor), FUN_10095800 (floor) | match |
| 2.1 client `HUD_WeaponsPostThink` FUN_10017f70 in full | — | gun spawn after VidInit `FUN_10017d30` (flag `0x10297b20`) → `FUN_1001ead0` (Reset `FUN_1001ebb0` + clip mirror/`+0xb4` 1, current 0, `+0xd90`/`+0xd94` 0); `lasthealth` branch: Spawn `FUN_10017900` = active item's Deploy (`FUN_1001fae0`: body, state, alt grip, mode memory `+0x41e`, **no draw event**) + `FUN_10026210` (penalty `+0x6f4`, crosshair copy, `+0x6f8..+0x700`, punch) then gun Spawn; Killed `FUN_100178b0` = Holster(0) `FUN_10024230` (models 0, fov 0, ext "", next attack 0.5, idle 3.0, blade window cleared, `IsAlive` false); rebuild of record/clip/reserve/mode/attachments from vuser*/ammo_*; gate `deadflag != 5 && FUN_10051700 (HUD health ≥ 1) && m_flNextAttack <= 0`; weapon-change check `FUN_1001fa30` rejects the current id (no Deploy); `to` timers `x − (float)msec/1000.0` with −1/−0.001 clamps; decay on runfuncs; `to.m_flNextAttack`, `to.fuser2/fuser3` run down | **R2v** match, with notes: (1) the port's spawn plays `events/draw.sc` in 2.1 mode too (2.1's client Deploy plays none; same cosmetic difference as the server's, round 1 kept it); (2) 2.1 never loads `fuser4` into the gun (`+0x94`) and doesn't predict the stock hit; the port loads it but `predicted().alt_attack` is false, so nothing reads it; (3) 2.1 runs `to.client.fuser2/fuser3` down to −0.001; the port leaves them (the server sends 0 and nothing on the client reads them), unobservable; (4) the gun-spawn path also rebuilds the weapon table: **#248** |

## Not verified / gaps
- worldgun.rs `WorldGun` think/touch/land/give_to/blast/fall_impact (round 1 + #158) were not re-walked this round, only `AmmoPack::touch`.
- random.rs and the weapons/mod.rs helpers: round 1 only.
- projectiles.rs (grenade/knife entities), fire.rs/bullets.rs (ShootBulletsPlayer, falloff, penetration, simulated bullets) and melee.rs: damage area.
- Client weapon events (fire/pump/reload/empty) beyond the pellet offsets: client area (#160 for 2.1).
- 2.1 `pfnUpdateClientData`/`pfnGetWeaponData` field layout: 3.0's wire format by design (#100 §1.2), not compared.
- Cosmetic 2.1 differences kept by design (round 1): no draw.sc/grenade.sc in 2.1 (server Deploy and the client's spawn Deploy), animation ids 0x33/0x34, 3.0 event argument encoding.
- server mod.rs item glue (`add_gun`, `give_named_gun`, `remove_all_items`, `select_item`, the cycler functions, `set_weapon_ammo`, `multiply_clip`): rules/entities glue, round 1 (powerups-entities.md) only.

## Validation (round 2, independent re-check)
- Rows: 48 before, 60 after (added: the client custom-table lifetime, the mod.rs helpers, random.rs, the moved #201 code, the PreThink section's 3 rows (the old mod.rs `pre_think` part moved there), `charge_melee_bar`, `reset_inventory`/`new_gun_state`, `sync_speed`, the worldgun/AmmoPack row, `ts_init`, the 2.1 client function row).
- Missing from the auditor's table: the new #201 code (`ts30::pre_think`/`pre_frame`, `ts21::pre_think_step`/`regen_melee_bar`), `ts_init`, `reset_inventory`/`new_gun_state`, the whole worldgun.rs entity code except `drop_ammo`, random.rs, the mod.rs helpers, `sync_speed`, `charge_melee_bar`, and the client's `CustomWP` table handling. The gameplay-relevant ones were compared (PreThink step both rulesets, spawn resets, `AmmoPack` touch, client table lifetime); the rest are round-1 rows, listed above.
- Re-checked "match" rows (20, against the decompile and, for the timers, the disassembly): `ts30 pre_think/pre_frame` (values match, order wrong → #249), `ts21 pre_think_step` (match; #202, #249), `ts21 decay_spread` server+client, `decrement_timers` 3.0+2.1, `zoom`/`unzoom`, `switch_fire_mode`, `switch_items` 3.0+2.1, `select_weapon`, `deploy_weapon` 2.1 server+client, `add_ammo`, `stop_reload`, `take_armor`, `reset_inventory`/`new_gun_state`, `reset_for_spawn` (→ #247), `on_spawn` (→ #247), `ts_init`, `post_think` (dead skip), client `write_weapon_data` (3.0 disasm), the 2.1 client `HUD_WeaponsPostThink` in full (→ #248), `AmmoPack::touch`. Wrong: 3 (`on_spawn`/`reset_for_spawn`, which the auditor had only for #208; the PreThink step's order; the client post-think's table rebuild).
- Mismatch rows: #208, #211, #212, #214, #215, #225 exist, are open and describe the same thing; their file:line references hold except #208's `gun.rs:1599` (now 1575) and #225's `gun.rs:945-978` (now 921-955), moved by later commits.
- Addresses spot-checked: 0x92a4c, 0x8f568, 0x8053c, 0x805e0, 0x8169c, 0x808ac, 0x80fb8, 0x81288, 0x81710, 0x81748, 0x8155c, 0x93da4, 0x908cc, 0x91610, 0x945bc, 0x77920, 0x79134, 0xa16c0, mp.dll 0x1005f5c0; 2.1 0x100552f0, 0x10049cb0, 0x1004a530, 0x1004a6c0, 0x10055e00, 0x100445d0, 0x100451c0, 0x100537e0, 0x10054180, 0x1005c680; client 0x100036f0, 0x10007de0, 0x10003eb0.., 2.1 client 0x10017f70, 0x10017d30, 0x1001e270, 0x1001ead0, 0x1001ebb0, 0x1001fae0, 0x10024230, 0x1001fa30, 0x10051700, 0x100b3e40: all exist and do what the rows say. Port paths all exist.
- Not filed (edge only, noted in rows): 2.1 `+0x6f0 = time` at spawn, the client's 1000 tests in `decrement_timers`, the 2.1 client's `fuser2/fuser3` run-down and unloaded `fuser4`, the `take_armor` NaN case, a mode-5 gun testing bit 6.
