# Weapons and combat (implementation notes)

Work package: weapons. Source of truth: the decomp's `docs/server/weapons.md`,
`server/architecture.md` §3/§4/§7.2-7.3, `client/rendering-effects-prediction.md` §4/§8,
`server/player-movement.md` §4/§6, and `server/decomp/CTSGun.c`, `CBasePlayer.c`,
`CBulletManager.c`, `CM61Grenade.c`, `CTSKnife.c`, `CTSWorldGun.c`. Addresses below are Ghidra
addresses of `ts_i386.so`.

## 1. Layout

| Path | Contents |
|---|---|
| `crates/shared/src/weapons/gun.rs` | `Gun` = `CTSGun` + the player fields it uses (timers, recoil penalty, kung-fu bar, melee windows), and the `GunHost` trait. Runs on the server and in client prediction. |
| `crates/shared/src/weapons/gun/tests.rs` | State-machine tests with a mock host. |
| `crates/shared/src/weapons/random.rs` | SDK `U_Random`/`U_Srand`/`UTIL_SharedRandomLong/Float`, bit-exact. |
| `crates/shared/src/weapons/spread.rs` | Pellet directions, `VKickback`, melee multiplier, bullet velocity, vector math. |
| `crates/shared/src/weapons/anims.rs` | View-model sequence tables A/B (`s_weapon_info+0x252/+0x28c`) for all 38 ids, extracted from the `Write_*` functions. |
| `crates/shared/src/weapons/mod.rs` | Ids, kinds, attachment bits, fire modes, `PRIORITY`, `default_mode`, `is_restricted`, `body_by_items`, `models`, `range_multiplier`. |
| `crates/shared/src/weapons/table.rs` | `tables()` (unchanged API) + custom weapon parameters (`CustomWP`). |
| `crates/shared/src/protocol/weapons.rs` | Message payloads (write + read), `clientdata_t` / `weapon_data_t` packing. |
| `crates/server/src/weapons/` | `mod.rs` hooks/API, `host.rs` (server `GunHost`), `fire.rs`, `bullets.rs`, `melee.rs`, `projectiles.rs`, `worldgun.rs`, `inventory.rs`, `commands.rs`, `net.rs`, `assets.rs`, `custom.rs`. |
| `crates/goldsrc/src/ext/weapons.rs` | `WeaponsEngine`: `pfnCanSkipPlayer`, `pfnFindEntityInSphere`, `SVC_WEAPONANIM`, `pfnGetGameDir`, string precaches. |

## 2. Implemented (TS function -> Rust)

| TS (address) | Rust |
|---|---|
| `CTSGun::ScaleSpread` 0x802a8 | `Gun::scale_spread` |
| `CTSGun::VKickback` 0x80414 | `spread::view_kick`, `Gun::vkickback` (server: `punchangle` for bots only) |
| `CTSGun::Precache` 0x80660 | `Event::ALL` / `assets::precache` |
| `CTSGun::AddItem` 0x80794 | `Gun::add_item` |
| `CTSGun::AddAmmo` 0x808ac | `Gun::add_ammo` |
| `CTSGun::DropWeapon` 0x8094c | `Gun::remove_weapon` |
| `CTSGun::TakeWeapon` 0x80a04 | `Gun::take_weapon` |
| `CTSGun::GetBetterWeapon` 0x80f40 | `Gun::get_better_weapon` (`PRIORITY` = `g_iWeaponPriority`) |
| `CTSGun::SelectWeapon` 0x80fb8 / `Deploy` 0x81030 | `Gun::select_weapon` / `Gun::deploy` / `Gun::switch_to` |
| `CTSGun::GetActiveItems` 0x81228 | `Gun::active_items` |
| `CTSGun::StopReload` 0x81288 | `Gun::stop_reload` |
| `CTSGun::MultiplyClips` 0x81308 | `Gun::multiply_clips`, `weapons::multiply_clip` |
| `CTSGun::SwitchItems` 0x8155c | `Gun::switch_items` (`useitems`) |
| `CTSGun::SwitchFireMode` 0x8169c | `Gun::switch_fire_mode` (`firemode`) |
| `CTSGun::Zoom`/`Unzoom` 0x81710/0x81748 | `Gun::zoom`/`unzoom` |
| `CTSGun::SecondaryAttack` 0x817a8 / `PrimaryAttack` 0x81a20 / `AlternateAttack` 0x81ba8 | same names |
| `CTSGun::EmptySound` 0x81be4 / `Fire` 0x81c78 | same names |
| `CTSGun::ColdCock` 0x81dd0 + `CBasePlayer::ColdCockAttack` 0x96610 | `Gun::cold_cock` + `melee::attack` |
| `CTSGun::ShotgunFire` 0x82108 / `StandardFire` 0x824a4 / `AkimboFire` 0x829c0 | same names |
| `CTSGun::GrenadeFire` 0x832f0 / `GrenadeIdle` 0x84f18 | `Gun::grenade_fire` / `Gun::grenade_idle` + `Gun::grenade_throw` |
| `CTSGun::KatanaFire` 0x833f0 / `KatanaThrow` 0x83b68 / `KnifeFire` 0x84044 | same names |
| `CTSGun::ReloadWeapon` 0x84a14 / `ShotgunReload` 0x84aac / `Reload` 0x84d5c | same names |
| `CTSGun::WeaponIdle` 0x854dc / `HolsterWeapon` 0x85698 | same names |
| `CTSGun::UpdateClientData` 0x858c8 | `net::update_client` (`WStatus`, `WeaponInfo`, 10 Hz) |
| `CTSGun::CCombatIdle/Block/Attack` 0x85e54/0x85ebc/0x86078 | `Gun::ccombat_idle/block/attack` |
| `CBasePlayer::ItemPostFrame` 0xdc4d0 + `CBasePlayerWeapon::ItemPostFrame` 0xa03d4 | `Gun::item_post_frame` |
| `CBasePlayer::PostThink` timer tail 0x91610 | `Gun::decrement_timers`, `Gun::timer_dt` |
| `CBasePlayer::PreThink` 0x908cc (weapon parts) | `Gun::pre_frame`, `weapons::pre_think` (BlockPlayer expiry, disarm decay) |
| `CBasePlayer::UpdateMeleeBar` 0x9650c / `GetMeleeMultiplier` 0x965d0 | `Gun::update_melee_bar` / `spread::melee_multiplier` |
| `CBasePlayer::ShootBulletsPlayer` 0x6f69c, `UTIL_TraceLine_Expanded` 0x6f4f4 | `fire::shoot_bullets`, `fire::trace_expanded` |
| `compile_range_damage` 0x7f1f4 | `range_multiplier` (pure function) |
| `CBulletManager` 0x6ebe0.. | `bullets.rs` (`ts_bulletmanager` entity) |
| `CBasePlayer::CheckHull` 0x95e28, `CloseCombatAttack` 0x95f88 | `melee::check_hull`, `melee::attack` |
| `VictimKungfuHold/KnifeHold/KatanaHold` 0x76f50/0x76de4/0x76c8c | `melee::hold` |
| `CTSStunt::CheckForBreakables` (melee-window part) | `melee::pending_hit` |
| `CM61Grenade` 0x7c850.. + `CTSGrenade::BounceTouch` 0x7be28 | `projectiles::Grenade` (link `m61_grenade`, classname `grenade`) |
| `TSExplosion` 0x7d01c, `DoSpectacularNess` 0x76b48 | `projectiles::explode` (+ `screen_shake`, `screen_fade` via `TSFade`, `TSFrags` type 1, the 0.25/1 s slow-motion point) |
| `CTSPhysicObject` 0x7ba54.. (`Spawn`, `SavePhysics`, `RestorePhysycs`, `WaitThink`) | `projectiles::SlowPhysics` (grenade, knife and `WorldGun` thinks) |
| `CreateGrenade` 0x7c874 / `CreateKnife` 0x7d9e4 / `CreateWeapon` 0xa07ec (`GoSlow` of the thrower's amount) | `projectiles::thrower_time_scale` |
| `CTSKnife` 0x7d9c0.. | `projectiles::Knife` (link + classname `knife`; `Use` 0x7dda4 = `Knife::use_by`) |
| `CTSWorldGun` 0xa07c8.. | `worldgun::WorldGun` (link `TSWorldGun`, classname `WorldGun`; `Use` 0xa1158 = `WorldGun::use_by`) |
| `CBasePlayer::PlayerUse` 0xd9848 (weapon entities) | `weapons::player_use` (from `pre_think`) |
| `UTIL_BubbleTrail` 0xee93c / `UTIL_WaterLevel` 0xee73c | `fire::bubble_trail` / `fire::water_level` |
| `CTSAmmoPack` 0xa1310.. / `CBasePlayer::DropAmmo` 0x946f0 | `worldgun::AmmoPack` (link `TSAmmoPack`, classname `AmmoPack`) / `worldgun::drop_ammo` |
| `CBasePlayer::PickUpWeapon` 0x9314c | `inventory::pick_up_weapon` |
| `CBasePlayer::DropWeapon` 0x94900 / `ThrowWeapon` 0x9529c | `inventory::drop_weapon` (`Drop::{Current, Id, Disarm}`) / `throw_weapon` |
| `CBasePlayer::BuyArmament/BuyWeapon/BuyAmmo/AddItem/TakeArmor` | `inventory::buy_armament/buy_weapon/buy_ammo/add_item/take_armor` |
| `GetSpeedBySlots` 0x76bd4 / `SetPlayerSpeed` 0x93068 | `inventory::sync_speed` |
| `GetWeaponType` 0x8705c / `IsRestrictedWeapon` 0x870c8 / `cmd_weaponrestriction` 0xc58a0 | `restriction_type` / `is_restricted` / `assets::cmd_weaponrestriction` |
| `GetBodyByItems` 0x7f0a4 | `body_by_items` |
| `ClientCommand` 0xad838 (weapon part) | `commands::client_command` |
| `UpdateClientData` 0xb0294 (weapon part) / `GetWeaponData` 0xb00ac / `CmdStart` 0xb067c | `protocol::weapons::pack_clientdata` / `pack_weapon_data` / `weapons::cmd_start` |
| `adjust_weapon_value` 0x9eca4, `save_weapon_value` 0x9f044, `SendWeaponParams` 0x9eb98, `InformClientsOnWeaponsParams` 0x9eb14 | `custom.rs` (`loadweapons`, `saveweapons`, `customweapons`, `CustomWP`) |
| `CBasePlayer::Killed` 0x8ed5c (weapon part) | `inventory::on_killed` |
| `TSPrecache` 0x9f320 (weapon part) | `assets::precache` |

### Hooks

* `game_init`: cvars `realbullet` 0, `ammocount` 0, `weaponstay` 15, `damagemult` 0,
  `customweapons` "", `ts_weapons_debug` 0 (not TS), `dmgreport` 0 (TS reads it but never
  registers it); server commands `weaponrestriction`, `loadweapons`, `saveweapons`.
* `precache`: the 11 events, every v_/p_/p_sil/w_ model, shells, fire sounds, the server-side
  weapon sounds, `w_m61`/`w_knife`/`w_sealknife`/`w_katana`/`ammopack`, `sprites/bubble.spr`
  (`g_sModelIndexBubbles`), `common/wpn_select.wav`; applies `customweapons`.
* `on_spawn`: TSInit (bar 100, penalty 0, timers), deploy (kung fu unless rules already gave a
  weapon), `BuyArmament` from the `tkw`/`tki`/`tka` lists (or the map's `game_player_equip`s
  instead), speed, `KFuPower`, `CustomWP` once per client. The inventory itself is **not**
  cleared here (see on_killed) so weapons the rules give in their `on_spawn` survive.
* `pre_think`: pressed buttons, `Gun::pre_frame` (recoil decay 0.1/s, bar regen 15/s, block and
  melee windows), `BlockPlayer` expiry, the delayed melee hit, and (alive, +use held) `Use` on
  the dropped gun or thrown blade in front of the player.
* `post_think`: `ItemPostFrame` while alive, seed reroll (`RandomLong(0, 32000)`, as TS), timer
  decrement by `frametime * fuser1 * (DFR ? 1.5 : 1)`, speed, messages.
* `on_killed`: holster, ammo pack, drop every weapon as `WorldGun`, clear inventory, slots,
  kevlar flag, loadout (TS `ResetArmament`), `ActItems 0`, `CurWeapon 0,0,0`.
* `client_command`: `weapon_<n>`, `lastinv`, `drop`, `drop_<n>`, `firemode`, `useitems`,
  `lasclr_<n>` (`PlInfo`), `rst_cfg`, `tkw id_att ...`, `tki item ...`, `tka type_count ...`;
  test helpers (need `sv_cheats`): `ts_give id_att ...` (weapon + full ammo),
  `ts_weapons_debug n` (sets the cvar when typed in a listen server console).
* `fill_client_data` / `get_weapon_data` / `cmd_start`: TS packing (below), seed.

## 3. Constants and formulas (as implemented)

* Timers: `dt = frametime * fuser1 * (DFR ? 1.5 : 1)`; NPA clamps at -1, the others at -0.001;
  `UTIL_WeaponTimeBase() = 0`.
* Spread = `accuracy` (+ `pl.spread_penalty` via `ScaleSpread`), x0.875 silenced; growth
  `(1 - min(slowmul*dt, ->0.5 if >=1)) * acc_kick * (DFR ? 1 : 2) * (silenced ? 0.775 : 1)`,
  cap 0.1, decay 0.1/s. Barrett: penalty 0.3 above 35 u/s; M60: acc kick x5 above 175 u/s.
* Silencer (not built-in): damage x0.775, spread x0.875, kick x0.775.
* Rates: semi on auto-capable weapons 0.1 s; shotgun semi 0.15 s; burst 3 x 0.03 s then 0.2 s
  (UMP 2 rounds); akimbo semi modes 0.1 s.
* Pellets: `t = SR(seed+i, -pi, pi)` (really `[-pi, -pi+6)` due to the integer range),
  `r = SR(seed+i+1, 0, 1)`, `m = 1 - r^3`, `dir = fwd + right*cos(t)*m*s + up*sin(t)*m*s`,
  8192 u. Linked akimbo: second ray seed + 0x21, offsets +-5 u along `v_right`.
* Range falloff: `min(1, 1/sqrt(max(round(dist/unit),1)*0.1))`. Bits 0x1002, shotguns
  0x2001002 (x1.5 headshot flag). Penetration: `left = dmg * pierce` (x0.5 into a player),
  `btype` not 1/2 and `left > 5`: step `min(2*left, 48)` (simulated bullets: `min(left, 48)`),
  then one 4096 u trace with a 30-unit range index.
* Simulated bullets: when `realbullet != 0` or the shooter's `fuser1 < 0.99`; step every
  `0.1/(realbullet+1)` s (every frame for >= 2) at `velocity_mps * 40 * dt * mult`, with
  `mult = GetBulletVelocityMult(s, bt)`: `s` is the shooter's slow-motion "others" amount
  (`pl+0x180`) while it's an activator, else `max(fuser1, 0.01)`; `bt` is the bullet-time flag
  (`pl+0x175`, `PlayerRules::btime`: slow pause, `ts_slowmotion` flag 4).
* Bubble trails: every hitscan pellet sends `TE_BUBBLETRAIL` (src -> end, `round(8192 *
  fraction / 64)` bubbles, the pierce shot `round(4096 * fraction / 64)`) when either end is at
  least 8 u under water.
* Slow motion of projectiles: grenades, knives (not while stuck in a wall) and falling
  `WorldGun`s start at the thrower's scale (`pl+0x180` if it's an activator, else
  `max(fuser1, 0.01)`) and run `SlowPhysics` every 0.1 s think: `velocity * s`, `avelocity * s`,
  `gravity * s²` from the saved pre-slow values, restored when `fuser1` returns to 1.
* `dmgreport` (only with `sv_cheats`): `> 1` prints `damage:%f`, `damage(piercing):%f` (bullets)
  and `(debug)%f` (thrown blades) to everyone as `HUD_PRINTTALK`; any non-zero value logs
  `<attacker> damaged <victim> (%f hp)` to the console from the player's TraceAttack.
* Melee: stock 35 u ahead hull 3, rate 0.725 (1.0875 stunting), damage `info.stock`
  (x2/3 stunting), bits 0x4001080. Knife slash 35 u, `damage * meleemult` (x2/3 stunting), bits
  0x1004, rate x1.5 stunting. Katana: reach 15-17 u, 50 (35 stunting, 55 backward) x meleemult,
  bits 0x1080, delays 1.0 / 1.0 / 1.1 / 0.7 / 0.9. Thrown: 1900 u/s, 100 / 50 / 150, bits
  0x8001000. Block windows 0.5 s, cost 33.
* Kung fu (decoded `CCombatAttack`): punches 10 (held jab, 0.25 s) / 20 (press, 0.35 s) /
  30 (combo end or duck, 0.65 s) / air 25 (0.5 s); kicks: uppercut 35 (1.0 s), back kick
  `clamp(speed*0.303,15,80)` (1.35 s airborne / 1.0 s ground), flying kick
  `clamp(speed*0.1538,15,45)` (1.0 s), combo kick 25 (0.5 s), quick kick 15 (0.3 s), speed kick
  `clamp(speed*0.1,15,30)` (0.75 s); stunting 15 (0.55 s). x meleemult, x2 with the kung-fu
  power-up. Bits 0x4001080, disarm + knockback (`v.vel += |a.vel|*dir`, `z = 235` above 30).
* Grenade: pitch `p*(p>=0 ? 10/9 : 8/9) - 10`, speed `min(750, (90-p')*7)` (roll x2/3, from the
  feet, friction 0.4, spin (-100, 600, 0)); fuse 3 s of `max(fuser1, 0.01)` time;
  `TSExplosion(110, 0x1040, 300, 1024)`; flying impact `speed*0.1` if it would kill the target.
* Explosion slow-motion point: after the damage, `DoSpectacularNess()` (`rules::scoring::do_spectacularness`; `spectacularness`
  clamped 0..100 and rounded, 0 = never, else `RandomLong(0,100) <= it`) creates a
  `ts_slowmotionpoint` at the blast (amount 0.25, 1 s, no bullet time).
* Ammo pack touch (alive player): every type goes through `AddAmmo` (whole amount clamped to the
  max carry, if the reserve is below it; no weapon needed), then the pack is removed whether or
  not anything fitted.
* Slots: 81 free at spawn, speed `round(210 + free*120/81)` (330 with nothing carried).
* Disarm threshold: katana 30, knives 25, weapons with < 21 slots 20, else `min(slots, 50)`.
* Dropped guns live `clamp(weaponstay, 5, 60)` s; `drop`/disarm drops `clamp(2*weaponstay, 10,
  120)` s and glow for the dropper.

## 4. Wire format

Events (all `FEV_NOTHOST`, invoker = player, zero origin/angles), exactly weapons.md §6.1:
`fire` (f1=f2=spread, i1 id, i2 attachments, b1 clip==0 / akimbo toggle / pump mode, b2 seed),
`firelinked`, `pump`, `empty` (none for the Contender), `reload` (f1 0 or 8/9/10), `draw`,
`coldcock` (f1/f2 last spread offset), `knife` (i2 hit flag), `katana` (i1 swing = anim index
15/16/17, i2 hit flag), `grenade` (i1 0x13/0x14/0x15), `closecombat` (attack: i1 type 1 punch /
2 kick / 3 back kick, i2 view anim index; block: f1 weapon id, i1 0, i2 1 kung fu / 0x12).
`bparam2` is passed as the raw seed int (the engine only transmits its low bit, like TS).

Messages: `WeaponInfo` (B id, B clip, S reserve, B mode, B items; kung fu: B 0, B bar, 0,0,0),
`WStatus` (B id, B owned, B clip; when the owned flag changes and for the akimbo/single twin
with each `WeaponInfo`), `ActItems` (ALL: B entindex, B active | 0x40 akimbo), `TSSpace`
(B free), `KFuPower` (C round(bar)), `CurWeapon` (0,0,0 on death), `PlInfo` (ALL: B entindex,
B 0, B colour), `CustomWP` (string), `TSMBlur` (PVS, kung fu), `TSFrags` (explosion),
`ScreenShake`, `TSFade`, `TSMessage` (custom weapons notice).

`clientdata_t` (sendweapons): `m_flNextAttack`, `vuser1[1]` power-ups, `m_iId 16`,
`vuser3 = (id, mode, owned attachments)` (kung fu: `(0, kick combo, punch combo)`),
`vuser4 = (akimbo toggle, clip, in reload)`, `ammo_nails` left akimbo clip, `ammo_shells` twin
record clip, `vuser2 = (state counter, reserve, alt grip)`, `vuser1[2]` active attachments,
`viewmodel` = model index of `pev->viewmodel`. `weapon_data_t[16]`: `m_iClip` (clip mirror),
NPA/NSA/idle clamped at -0.001, `fuser1` = the gun's own timer, the rest 0.
`shared::protocol::weapons::{unpack_clientdata, unpack_weapon_data}` are the client side.

## 5. Deviations and decisions

* **Inventory reset at death, not at spawn**: TS gets a fresh `weapon_tsgun` every spawn. Here
  `on_killed` clears it, so weapons the rules give in `rules::on_spawn` (which runs before
  `weapons::on_spawn`) are kept. Anyone respawning a player without killing them must call
  `weapons::strip_all` first.
* **The loadout is cleared at death** (TS `Killed` -> `ResetArmament`); the TS client resends
  `rst_cfg` + `tkw/tki/tka` from its buy menu. Bots must be given weapons again (rules).
* **M61 buys and stealth shoes are refused**, as in TS 3.0 (only allowed when
  `CTSGameRules::m_mode > 1`, unreachable). `weapons::set_round_mode_items(true)` enables them.
  Grenades still come from power-ups via `give_weapon(24)`. On death TS turns carried grenades
  into grenade power-ups; here they drop as `WorldGun`s.
* `CTSAnimation::Animate` calls (third-person sequences) are not made: `GunHost::player_anim`
  is a no-op on the server (movement package). `Inventory::anim_extension()` exposes the
  `DefaultDeploy` extension.
* The kung-fu uppercut / flying kick / katana duck swing force a crouch through `iuser4` and the
  stunt recover timer in TS; those stunt-state writes are left to the movement package (only
  `BlockPlayer` is applied).
* `ShootBulletsPlayer` applies each pellet's damage directly (TS sums them in
  `Apply/AddMultiDamage`); hitgroup multipliers and armour are the damage package's.
* Spawn protection (`pl+0x71c`) isn't checked before a disarm (rules field).
* `ThrowWeapon`'s velocity is garbled in the decompile: alive drops fly at 250 u/s (+125 up)
  plus the player's velocity, corpses drop at 60..160 u/s in a random direction.
* `WorldGun`s and stuck knives get ground friction (x0.5 per 0.1 s): FWGS doesn't slow a
  sliding `MOVETYPE_BOUNCE` entity, so they would slide forever.
* **+use on weapons** (`weapons::player_use`) searches only `WorldGun`s and thrown knives. TS's
  `PlayerUse` picks the single best usable entity of any class; here buttons, doors and ground
  weapons have their own searches (`entities::stock::player_use`, `rules::lifecycle`), so one
  +use can reach both a gun and, say, a button in front of the player. Stuck blades are
  `SOLID_TRIGGER` instead of `SOLID_NOT` (from before `Use` existed; harmless).
* **`SlowPhysics` keeps an explicit "saved" flag.** TS treats "saved velocity == 0" as "nothing
  saved". With that sentinel, a resting slowed object saves again on every think (its gravity
  shrinks by `s²` each time) and is never restored. The port saves once and restores the saved
  values. Kept as TS: the restore puts back the *pre-slow* velocity, so a slowed grenade or
  knife lying on the ground jumps off with its throw velocity when the slow ends.
* `UTIL_BubbleTrail` is not sent for simulated bullets (`realbullet` / slow motion): TS sends it
  with the stale static trace result of the previous hitscan shot.
* `dmgreport` is registered as a cvar (port addition, default 0). TS reads it with
  `CVarGetFloat` but never registers it, so TS output needs the cvar created elsewhere.
* `WorldGun` `WaitTouch` doesn't check the 0.5 s pickup delay (as TS: only `Use` does).
* **The flying impact credits the projectile, as in TS** (`impact_damage`: inflictor and
  attacker are both the grenade, knife or gun). TS passes `this->pev` as the TraceAttack attacker
  and as both `ApplyMultiDamage` arguments in `CM61Grenade::WaitThink` (0x7cacc; see the
  disassembly at 0x7ce3c/0x7ce46), `CTSKnife::WaitThink` (0x7ea30) and `CTSWorldGun::Fall`
  (0xa0d84), not `pev->owner`. So a breakable broken, or a player killed, by a flying projectile
  isn't credited to the thrower (issue #1). Only the explosion (`TSExplosion(..., owner)`) and a
  thrown blade's direct hit on a player (`BounceTouch`) credit the thrower.
* The bullet manager runs on a hidden `ts_bulletmanager` entity (TS: `CTSGameRules::Think`);
  bullets are also dropped after 16384 u without a hit.
* The kick for bots goes to `punchangle` (TS `pev+0x68`), humans get it client-side.
* `p_pdw_sil.mdl` is missing from the content (`p_pdw_Sil.mdl`): the MP7 uses `p_pdw.mdl`.
* The `CloseCombatAttack` / `CCombatAttack` / `KatanaFire` stack slots are mangled in the
  decompile; values were re-associated by usage (the delay stored in `pl+0x4f8` is the next
  attack, the smaller `BlockPlayer` value is separate; the katana event's `iparam1` is the swing
  index the client indexes its sequence table with). The weapons.md §4.4 "Delay" column is the
  `BlockPlayer` time, not the attack delay.

## 6. Missing / partial

* `SVC_WEAPONANIM` is sent only to clients that don't predict weapons (TS behaviour); the client
  package plays view animations from events.
* The briefcase/flag (rules).
* Breakable explosions (`entities::stock::explode::ts_explosion`, the same `TSExplosion` in TS)
  don't create the spectacular slow-motion point or knock `WorldGun`s around; only the
  weapons' `projectiles::explode` does (#33).
* `DropArmor` 0x93a3c armour packs (`CTSAmmoPack` with `armorvalue`) aren't ported. They're dead
  in TS: nothing calls `DropArmor`, so ammo packs never carry armour there either.
* Shrapnel (`CTSShrapnel`) and the flag (`CTSFlag`), the other `CTSPhysicObject`s, aren't
  ported. Nothing in the decomp calls `CTSShrapnel::Create` 0x7bf9c directly.

## 7. Tests

* `cargo test -p shared`: 44 tests pass (x86_64 and `--target i686-unknown-linux-gnu`). Includes `random::matches_sdk_c_bit_for_bit` (43 seeds
  compared against the SDK C `util.cpp` compiled with `gcc -m32 -O0 -mfpmath=387`), fire rates
  (auto, semi, DFR x1.5, slow motion), burst (3 / UMP 2), pump alternation, magazine and
  3-phase shotgun reloads, recoil penalty growth/cap/decay, silencer multipliers, akimbo merge +
  linked shot (seed + 0x21, +-5 offsets, clip split, single mirror), zoom steps (AK 32/13 and the
  hidden view model), grip toggle, grenade pull/throw/slots, throw math, knife stacks, kung-fu
  punch + block + bar regen, Infinite Clip, priority switching, pickup limits, mode memory,
  message layouts, `CustomWP` round trip, clientdata/weapon_data packing.
* `cargo test -p ts-server`: command argument parsing (`%i_%i`), `SlowPhysics` (scale, rescale,
  restore, no gravity compounding at rest), ammo pack contents (`take_ammo`).
* **Not yet tested in engine**: slowed projectiles (power-up slow motion / `globalslow` with a
  grenade, thrown knife and dropped gun in sight), bullet-time simulated bullets (slow pause),
  bubble trails under water, `dmgreport 2` with `sv_cheats 1`, +use on a flying/landed gun and on
  a stuck knife/katana, the explosion slow-motion point (`spectacularness 100`).
* Builds: `cargo build -p ts-server -p ts-client` (x86_64), `--target i686-unknown-linux-gnu`,
  `--target i686-pc-windows-gnu`: OK (only the pre-existing stdcall linker note).
* In engine (FWGS, `~/games/xash_weapons`, `ts_bay107`): dedicated server on port 27210 PASS
  (all precaches resolve). Listen-server autotests (`ts/autotest_weapons*.cfg`, debug log on):
  AK fire + reload + scope; akimbo Berettas from two singles, alternating shots and a linked
  `firelinked` double shot; Mossberg fire/pump alternation and 8-pellet hits with the shotgun
  bits; shotgun reload phases 8, 9 x5, 10; kung-fu punch/kick combos with `closecombat` events;
  knife slash/block/throw; grenade pin, throw and roll, explosion after ~3 s that hit the player
  for 110 (self-damage through `damage::apply` from the grenade's think); `drop` -> `WorldGun`
  lands -> picked up again with +use; `realbullet 1` routes pellets through the bullet manager;
  `loadweapons` file changes the AK damage and sends `CustomWP`.
* **Not tested in engine**: hits on other players (no bots/second client available: hitgroups,
  penetration through a player, disarm, blocks between two players, knockback), the death drop
  path (the placeholder `rules::on_killed` doesn't call `weapons::on_killed` yet), client-side
  prediction against these packers, Steam/ReHLDS engines.

## 8. Integration notes

**Rules package**
* Give weapons: `weapons::give_weapon(p, cx, id, attachments)` (= `ts_groundweapon` pickup,
  selects it), ammo: `weapons::give_ammo(p, cx, ammo_type, rounds)` (returns rounds added; a
  ground weapon's `wextraclip` gives `mags * per_buy` only if the reserve is empty).
* Power-ups: Infinite Clip -> `weapons::multiply_clip`; grenade -> `give_weapon(p, cx, 24, 0)`;
  kung fu sets `Player::kungfu` (x2 melee), DFR/slow-mo are read from `Player::active_powerup`
  and `pev->fuser1`.
* `rules::on_killed` must call `weapons::on_killed(p, cx)` (drops, ActItems reset, loadout
  reset). Spectate/team change without death: `weapons::strip_all`.
* Restrictions: `weapons::set_restriction(mask)` (`ts_mapglobals` spawnflags: `0x1f ^ flags`),
  `weapons::restriction()`; broadcast it as `SrvSett 3`, `realbullet` as `SrvSett 0`,
  `ammocount` as `SrvSett 2` (cvars registered here).
* Armour absorb: read `shared::weapons::tables().combat.armor_absorb` (a weapons file may
  change it).
* `Player::slots_used` and `maxspeed` are written by weapons only when the slot count changes.
* Kevlar from the loadout (`tki 16`) sets `armorvalue = 100` and uses 10 slots
  (`SvState::items & 0x10`).

**Damage package**: `DamageInfo.hitgroup` = trace hitgroup, `weapon` = TS id, bits as listed in
§3 (0x2000000 = shotgun headshot x1.5, 0x8000000 = thrown blade, 0x4000000 = close combat).
`PTakeDam` is yours.

**net.rs (AddToFullPack)**: `weapons::glow_for_host(ent, host)` says when TS forces
`rendermode 0, renderamt 2, renderfx 19, rendercolor 100,100,100` (own dropped gun / thrown
knife). Players' `weaponmodel` (p_ model) is set in `pev->weaponmodel`.

**Client / prediction package**: drive `shared::weapons::Gun` with a client `GunHost`:
`unpack_clientdata` + `unpack_weapon_data` before the command, `Gun::pre_frame`,
`item_post_frame`, `decrement_timers(Gun::timer_dt(msec/1000, fuser1, powerups))`; host events
go to the local event handlers, `shoot` can be a no-op (the `fire` event re-traces with
`spread::pellet_dir` and the same seed), `kick` feeds the view kick. Messages decode with
`shared::protocol::weapons::{WeaponInfo, WStatus, ActItems}::read`, `read_kfu_power`,
`parse_custom_wp` (apply with `shared::weapons::table::set_weapon_params`). View-model sequences:
`shared::weapons::anim(id, alt, index)`.

**Movement package**: `Inventory::anim_extension()`, `Inventory::current()`; the weapons code
sets `iuser4 & 1` through `BlockPlayer` (and clears it when its timer runs out) and clears
`0x20000|0x4` when a kung-fu attack ends a wall climb.
