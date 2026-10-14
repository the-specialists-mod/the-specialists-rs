# Weapons audit (3.0 + 2.1)

Issues filed: #129 (3.0+2.1 grenade throw f32), #130 (2.1 holster view model), #134 (3.0 loadweapons persistence + switch), #141 (2.1 knife/katana press, katana throw, idle reload under Infinite Clip), #146 (2.1 corpse grenades), #158 (2.1 world gun breakable damage + ammo pack removal), #159 (2.1 hand-typed tkw akimbo id), #161 (pellet direction arithmetic, all builds). Comment on #74 (2.1 client events use the raw damage too).
Earlier 3.0 firearm audit (Oct 12): #20-#28, all closed; re-checked tables and StandardFire/Deploy/Holster paths against current code.

Scripts: /tmp/audit/cmp_weapons.py (3.0 table), /tmp/audit/scripts/{dump21,cmp21,assets21}.py (2.1).

## Tables
| Port data | 3.0 original | 2.1 original | Status |
|---|---|---|---|
| shared/data/weapons.toml (all 36 ids x 25 fields, modes, zoom, hud, kind) | Write_* via compile_weapons_info + Sparta() | — | match |
| shared/data/ammo.toml (13 types, max_carry 4 cols) | compile_ammo_info, Set_ammo_count (ts_i386.so) | — | match |
| shared/src/weapons/anims.rs (seq tables A/B, ids 1..37) | Write_* +0x252/+0x28c | — | match |
| ts21/data/weapons21.toml (33 ids x 20 fields + modes/zoom/hud/kind) | — | FUN_10049890 writers (stride 0x3f4 @0x100bbea8) | match |
| ts21/data/ammo21.toml (11 types) | — | compile_ammo_info 0x10049420, Set_ammo_count 0x100492a0 | match |
| ts21/data/assets21.toml (models, sounds, shell, anim ext, body, fx, f_d4) | — | writers +0x86..+0x31e, +0x120, +0x364, +0xcc | match |
| ts21 TMP_SEQUENCES | — | Write_skorpion +0x26a/+0x2a4 | match |
| server max_carry column (ammo_column_of) | Set_ammo_count switch default | 2.1 same | match (client buy clamp = existing #108) |

## Functions (port -> 3.0 original -> 2.1 original -> status)
| Port function | 3.0 | 2.1 | Status |
|---|---|---|---|
| ts21 gun.rs `fire` | — | FUN_1004ac20 | match |
| ts21 `empty_sound` | — | FUN_1004ab60 | match (timers T*0.1, idle 0.5, empty event) |
| ts21 `standard_fire` | — | FUN_1004b400 (+disasm consts) | match |
| ts21 `shotgun_fire` | — | FUN_1004b050 | match (pump event args are 3.0-encoded by design) |
| ts21 `akimbo_fire` | — | FUN_1004b8c0 (disasm: src+off*v_right correct, decompile scrambled) | match |
| ts21 `scale_spread` (server) | — | FUN_10049a50 | match (gun+0xd98 store is dead: never read) |
| ts21 `vkickback` (server) | — | FUN_10049b80 | match (bot-only punchangle) |
| ts21 `slow_div`/`scale_delay`/`rate_delay`/`burst_delay` | — | FUN_100551c0, FUN_10042c10/20/40 bits | match |
| shared_random_float_unrounded | SDK | FUN_10066640 | match for range>=0 (negative range unreachable) |
| server `playback` (event) in 2.1 mode | pfnPlaybackEvent direct (3.0) | FUN_10048ff0 replays to iuser1==5 spectators | match in effect: 2.1 never sets iuser1=5 (FUN_1005aad0 cycle 3/2/1), dead code |
| client 2.1 scale_spread / vkickback (prediction branch) | — | client FUN_1001e660 / FUN_1001e8e0 | match (FUN_1001e850 = discarded |v|) |
| client_slow_div | — | client FUN_100261b0 | match |
| client decay_spread / timer write-back clamps | client.dll 3.0 | client FUN_10017f70 | match (crosshair copy = existing #111) |
| ts21 decay_spread (server) | — | PreThink 0x100537e0 (+0x6f0 set every think) | match |
| Gun::deploy in 2.1 mode | CTSGun::Deploy 0x81030 | CTSGun::Deploy 0x1004a530 | match except no draw.sc in 2.1 (cosmetic; decomp doc recommends keeping) and anim 0x33 vs 0x34 (documented deviation) |
| Gun::holster_weapon in 2.1 mode | HolsterWeapon 0x85698 | FUN_1004d7a0 | MISMATCH #130 (kung-fu view model, draw anim, draw event) |
| Gun::grenade_throw (3.0 and 2.1) | GrenadeIdle 0x84f18 / mp.dll 0x10057d80 | GrenadeIdle 2.1 | MISMATCH #129 (f32 vs double 10/9, 8/9, 2/3) |
| Gun::grenade_idle flow in 2.1 mode | — | CTSGun::GrenadeIdle 2.1 | match except no grenade.sc (cosmetic) and anim 0x34 vs 0x35 (documented) |
| default_deploy (server host) in 2.1 mode | DefaultDeploy 0xa0178 | FUN_10064ba0 | match (protect 0x694 = time+1 at spawn; idle 3.0; draw.wav CHAN_VOICE) |
| Gun::grenade_fire in 2.1 mode | GrenadeFire 0x832f0 | FUN_1004c020 | match (no grenade.sc in 2.1, cosmetic) |
| Gun::reload / reload_weapon / shotgun_reload in 2.1 mode | Reload 0x84d5c / ReloadWeapon 0x84a14 / ShotgunReload 0x84aac | Reload 0x1004d0e0 / FUN_1004ce20 / FUN_1004cea0 | match (anims via 3.0 events by policy; 2.1 rejects only need==0 vs 3.0 need>0: only reachable with clip>max, not found) |
| ts21 ccombat_block with blade | — | Reload kind 3 returns | match (hook returns when has_slot) |
| custom.rs load/save/inform | adjust_weapon_value 0x9eca4, save 0x9f044, Inform 0x9eb14 | (2.1 customweapons: #100 known gap) | parse match; persistence MISMATCH #134 (TS rebuilds per map in W_Precache) |
| Gun::primary_attack / secondary_attack in 2.1 mode | PrimaryAttack 0x81a20 / SecondaryAttack 0x817a8 | 0x1004aa50 / 0x1004a800 | MISMATCH #141 (knife/katana fresh press; katana +attack2 throw); guns/grenade/zoom/alt-grip match |
| Gun::weapon_idle in 2.1 mode | WeaponIdle 0x854dc | 0x1004d660 | MISMATCH #141 (Infinite Clip auto-reload); burst/idle anim match |
| Gun::weapon_post_frame (+reload finish) in 2.1 mode | CBasePlayerWeapon::ItemPostFrame 0xa03d4 | 0x10064ce0 (ShouldWeaponIdle=1) | match |
| Gun::decrement_timers in 2.1 mode | PostThink | PostThink 0x10054180 tail | match (clamps -1/-0.001, 1000 sentinels) |
| Gun::switch_fire_mode / switch_items in 2.1 mode | 0x8169c / 0x8155c | FUN_1004a790 / FUN_1004a6c0 | match |
| ts21 cash.rs buy_armament / buy_ammo / add_item / buy_weapon | — | 0x100559d0 / 0x10055a40 / 0x10055c30 / 0x10055e00 | match |
| pick_up_weapon in 2.1 mode | PickUpWeapon 0x9314c (akimbo refusal, stack slots x count, ROUND) | 0x10055830 (flat byte slots, no refusal) | differs only for hand-typed `tkw <akimbo id>` (2.1 drops stacks one by one, splits akimbos): not filed |
| Gun::take_weapon in 2.1 mode | TakeWeapon 0x80a04 | GiveWeapon FUN_1004a040 | match |
| drop_weapon / on_killed in 2.1 mode | DropWeapon 0x94900 | FUN_10056080, Killed loop | MISMATCH #146 (corpse grenades -> power-ups); disarm via plugin hook match; akimbo split match |
| combat21.toml gun values (silencer, penalty, semi, burst, fuse, max speed, per life, attachment prices) | — | functions above | match |
| combat.toml silencer 0.775 (float) | StandardFire 0x824a4 (.so fmuls), mp.dll 0x10093e90 float | — | match |
| Gun::standard_fire (3.0) structure | 0x824a4 | — | match (spot check) |
| client 2.1 predicted() | — | client FUN_10025f40 | match |

## Gap closure (second pass)
| Port function | 3.0 | 2.1 | Status |
|---|---|---|---|
| Gun::post_frame_ready + ts21 cancel_runs_frame_in_reload | CBasePlayer::ItemPostFrame 0xdc4d0 (+alt2 needs in_reload==1 && info.stock) | FUN_1002d840 = 2.1 **server** player ItemPostFrame (disasm 0x1002d840..0x1002d8c0: +alt2 && in_reload==1, no stock test); both reseed +0x7ec/+0x748 = RANDOM_LONG(0,32000) after the item frame | match (port citation is to mp.dll, correct) |
| client post_frame_gate in 2.1 mode | client 0x10003d4b | 2.1 client gate inside FUN_10017f70 0x1001860c..0x1001863e: deadflag!=5, !FUN_10002694, next_attack <= 0.0; no +alt2 pass; ItemPostFrame = vtable slot 70 (thunk 0x1000191f -> 0x10025f40) | match |
| Gun::shotgun_fire (3.0) | ShotgunFire 0x82108 (.so disasm: 0.35 flds float, 0.15 float static) | — | match |
| Gun::akimbo_fire (3.0) | AkimboFire 0x829c0 (±5 offsets, seed+0x21, kick x2 linked, rates) | — | match |
| Gun::scale_spread / spread::shot_interval / grow_spread_penalty (3.0) | .so 0x702a8 (GCC: x stored float, 1-x extended, *akick*k, fcoml double 0.1, NaN kept); mp.dll 0x10053ee0 (MSVC: unrounded d test, x unrounded, *k*akick) | — | match (gun+0x9ec/0x9dc zeroing is a dead store) |
| Gun::vkickback / spread::view_kick (3.0) | .so 0x70414, mp.dll 0x10054000; constants 0.85, 0.35, 0.15, -0.25, -0.3, -0.35, 0.775, 0.6666667 all float | — | match |
| spread::timer_step / decay_spread_penalty (3.0) | .so PostThink 0x81af2, PreThink objdump 0x80d88; mp.dll 0x1006053c, 0x1005fa16 | — | match (port's .so citation 0x80d88 is an objdump address, not Ghidra numbering; doc nit) |
| client events fire/firelinked/pump in 2.1 mode | — | HookEvent FUN_10016470; FUN_10073980 -> 0x10073b70 / 0x10076ce0 / 0x10075080 -> bullets FUN_1007c5e0 | raw info+0x1c damage, no silencer factor: covered by #74 (comment added); falloff/penetration structure, akimbo ±4 offsets match |
| throw_weapon in 2.1 mode | ThrowWeapon 0x9529c | 0x100564d0 | match |
| worldgun create/spawn/pickup in 2.1 mode | CreateWeapon/Spawn/GetWeapon/Use/WaitTouch | 0x10065040.. | match (iuser4 bit 1 vs 2 = documented 3.0-bits deviation) |
| worldgun think (Fall) / touch (BounceTouch) in 2.1 mode | Fall 0xa0d84 / BounceTouch 0xa0b0c | 0x10065550 / 0x10065270 | MISMATCH #158 (breakable damage) |
| drop lifetime/glow (set_drop_lifetime) in 2.1 mode | DropWeapon 0x94ed9 | not in 2.1 DropWeapon | MISMATCH #158 (comment: 2.1 keeps clamp(weaponstay,5,60), no dropper glow) |
| AmmoPack touch in 2.1 mode | WaitTouch 0xa16c0 | 0x10065a90 | MISMATCH #158 (removal only after a take) |
| CTSKnife::Spawn / CTSGrenade::BounceTouch in 2.1 mode | — | 0x10048100 / 0x10046d70 | match (gravity via ts21 hook; fuser1 0 vs 1 = documented time-scale encoding) |
| pick_up_weapon akimbo refusal in 2.1 mode | 0x9314c | 0x10055830 | MISMATCH #159 (hand-typed tkw only) |
| 2.1 DefaultReload need<0 | ReloadWeapon need>0 | FUN_1004ce20 need!=0 | no observable difference: clip>table clip unreachable in 2.1 and in port 2.1 mode (no MultiplyClips: infinite_clip_instant false; akimbo tkw refused) |

## spread.rs pellet/vector helpers (third pass)
| Port function | 3.0 | 2.1 | Status |
|---|---|---|---|
| spread::pellet_offset (3.0 cos/sin, 1-r^3) | .so 0x5fb00..0x5fb5d, mp.dll 0x10047d1f..0x10047d6f, client 0x10045901..0x1004596e | — | match (t stored float, r exact 16-bit fraction; only x87-vs-f64 double rounding of cos/sin*m) |
| ts21 triangle (pellet_offset hook) | — | mp.dll 0x1004012f..0x1004019d (SR(seed+i..+3, -0.5, 0.5) sums) | match (sums exact in f32) |
| spread::pellet_dir_at (server + client event) | .so 0x5fb63..0x5fc12 / mp.dll 0x10047d75..0x10047e02 / client 0x10045970..0x100459de | mp.dll 0x100401b7..0x10040260 / client FUN_1007c5e0 | MISMATCH #161 (all f32, port order; each build keeps parts on the FPU in its own order) |
| spread::angle_vectors | engine MakeVectors(v_angle) (3.0 Fire: `UTIL_MakeVectors(pev->v_angle)`, no punchangle) | same | angles match; engine-vs-port trig bits not compared (noted in #161) |
| spread::add/sub/scale/ma/dot | used per call site (grenade throw #129) | — | no standalone TS counterpart; length/normalize covered by #150 |

## Not verified / gaps
- Client weapon events: reload/coldcock/closecombat/knife/katana/empty differ only in 3.0 arg encoding (port rule); muzzle/shell cosmetics are covered by #72/#73/#77. Not every client effect (shell models, muzzle sprite offsets) was compared byte by byte.
- spread.rs pellet/angle_vectors/range helpers were not re-walked (covered by the damage auditor's bullet path and #80/#150).
- 2.1 `customweapons` cvar (adjust_weapon_value 0x10063ed0): not implemented, tracked in #100.
- 2.1 `realbullet`/bullet damage/penetration and blades: other auditors (damage, melee).
- Cosmetic 2.1 differences kept by design: no draw.sc/grenade.sc in 2.1 (decomp doc recommends keeping 3.0's), anim ids 0x33/0x34 vs 3.0's (documented deviation), event arg encoding 3.0 (port rule).
- FUN_10048ff0's in-eye replay: dead code in 2.1 (iuser1 never 5).
