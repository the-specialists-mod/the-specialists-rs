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
| `CTSGun::Precache` 0x80660 | `Event::ALL` / `assets::precache_events` (from `crate::precache`'s `W_Precache`) |
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
| `CTSGun::UpdateClientData` 0x858c8 | `weapons::update_client_data` → `net::update_client` (`WStatus`, `WeaponInfo`, 10 Hz), called at the end of the player's `UpdateClientData` (`player::hud::update_client_data`, from PreThink). The caches (`SvState::next_update`, `sent_*`) are the gun entity's in TS: a new gun starts with `CTSGun::Reset`'s values (`new_gun_state`: last id 0, so a gun on kung fu sends no `WeaponInfo`; timer 0), `CTSGun::Deploy` with a weapon zeroes the timer (`GunHost::deploy_update_now`), and a spawn keeps the caches of a gun still held. `WeaponInfo`'s decision is `net::weapon_info_update`. |
| `CBasePlayer::TSInit` 0x92a4c (weapon fields) | `weapons::ts_init`, from `rules::lifecycle::on_spawn`'s TSInit block | 81 free slots, no loadout items, the speed (330) recorded for 81. The only reset of these: `RemoveAllItems`, `StartObserver` and `Killed` leave them to the item code. |
| `CTSGun::CCombatIdle/Block/Attack` 0x85e54/0x85ebc/0x86078 | `Gun::ccombat_idle/block/attack` |
| `CBasePlayer::ItemPostFrame` 0xdc4d0 + `CBasePlayerWeapon::ItemPostFrame` 0xa03d4 | `Gun::item_post_frame` |
| `CBasePlayer::PostThink` timer tail 0x91610 | `Gun::decrement_timers`, `Gun::timer_dt` |
| `CBasePlayer::PreThink` 0x908cc (weapon parts) | `Gun::pre_frame`, `weapons::pre_think` (BlockPlayer expiry, disarm decay) |
| `CBasePlayer::UpdateMeleeBar` 0x9650c / `GetMeleeMultiplier` 0x965d0 | `Gun::update_melee_bar` / `spread::melee_multiplier` |
| `CBasePlayer::ShootBulletsPlayer` 0x6f69c, `UTIL_TraceLine_Expanded` 0x6f4f4 | `fire::shoot_bullets`, `fire::trace_expanded` |
| `compile_range_damage` 0x7f1f4 | `range_multiplier` (pure function) |
| `CBulletManager` 0x6ebe0.. | `bullets.rs` (256 slots, run from `CTSGameRules::Think` between its `realbullet` and `usecash` updates) |
| `CBasePlayer::CheckHull` 0x95e28, `CloseCombatAttack` 0x95f88 | `melee::check_hull`, `melee::attack` |
| `VictimKungfuHold/KnifeHold/KatanaHold` 0x76f50/0x76de4/0x76c8c | `melee::hold` |
| `CTSStunt::CheckForBreakables` (melee-window part) | `melee::pending_hit` |
| `CM61Grenade` 0x7c850.. + `CTSGrenade::BounceTouch` 0x7be28 | `projectiles::Grenade` (link `m61_grenade`, classname `grenade`) |
| `TSExplosion` 0x7d01c, `DoSpectacularNess` 0x76b48 | `entities::stock::explode::ts_explosion`, shared with breakables (stock-entities.md §3.4) |
| `TSExplosion`'s `WorldGun` branch 0x7d678 | `worldgun::blast` (`blast_velocity`, `ground_hop`) |
| `CTSPhysicObject` 0x7ba54.. (`Spawn`, `SavePhysics`, `RestorePhysycs`, `WaitThink`) | `projectiles::SlowPhysics` (grenade, knife and `WorldGun` thinks) |
| `CreateGrenade` 0x7c874 / `CreateKnife` 0x7d9e4 / `CreateWeapon` 0xa07ec (`GoSlow` of the thrower's amount) | `projectiles::thrower_time_scale` |
| `CTSKnife` 0x7d9c0.. | `projectiles::Knife` (link + classname `knife`; `Use` 0x7dda4 = `Knife::use_by`) |
| `CTSWorldGun` 0xa07c8.. | `worldgun::WorldGun` (link `TSWorldGun`, classname `WorldGun`; `Use` 0xa1158 = `WorldGun::use_by`) |
| `CBasePlayer::PlayerUse` 0xd9848 | `player::player_use` (all classes; from `post_think`), with `weapons::use_by_player` for the weapon entities |
| `CM61Grenade::Use` 0x7ce64 | `Grenade::use_by` (kicks a grenade resting on the ground) |
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
| `adjust_weapon_value` 0x9eca4, `save_weapon_value` 0x9f044, `SendWeaponParams` 0x9eb98, `InformClientsOnWeaponsParams` 0x9eb14 | `custom.rs` (`loadweapons`, `saveweapons`, `CustomWP`; TS's file format and messages, the absorb values and the custom-weapons flag in the rules state, reset by `CTSGameRules::Init`: rules.md §2) |
| `CBasePlayer::Killed` 0x8ed5c (weapon part) | `inventory::on_killed` |
| `TSPrecache` 0x9f320 (weapon loop) | `assets::precache_weapon_assets` (from `crate::precache::ts_precache`) |
| `CBasePlayerWeapon::AddToPlayer` 0xf1e60 (weapon bit) | `add_gun` (`TSGUN_WEAPON_BIT`, bit 16), `add_cycler` (bit 0) |
| `CBasePlayer::HasWeapons` 0xdb060 | `has_weapons` (the gun or a `cycler_weapon`) |
| `CBasePlayer::GiveNamedItem` 0xdb524 (`weapon_tsgun`) | `give_named_gun` (`PlayerSpawn`) / `maprules::give_named_item`: a `weapon_tsgun` entity (`playeritem::WorldTsGun`) |
| `CBasePlayer::AddPlayerItem` 0xdc0c4 / `SwitchWeapon` 0xdd98c | `add_gun` + `switch_to_gun`, `add_cycler` + `deploy_cycler` (`should_switch_weapon`) |
| `FShouldSwitchWeapon` 0xcf380 (multiplayer) / 0xde74c (single player) | `should_switch_weapon` |
| `CBasePlayer::SelectItem` 0xdade4 (`use <classname>`) | `select_item` |
| `CBasePlayer::ItemPostFrame` 0xdc4d0 (gate) | `item_post_frame_ready` (gun: `Gun::post_frame_ready`; cycler: `cycler_post_frame_ready`) |

### Hooks

* `game_init`: cvars `realbullet` 0, `ammocount` 0, `weaponstay` 15, `damagemult` 0,
  `customweapons` "" (registered and, as in TS, never read), `ts_weapons_debug` 0 (not TS); server commands `weaponrestriction`,
  `loadweapons`, `saveweapons`. `dmgreport` is read but, as in TS, never registered.
* `bullet_think` (from `rules::frame`'s `CTSGameRules::Think`, 0x77635, right after the
  `realbullet` update and before `usecash`): `CBulletManager::Think` while it counts any bullet. Single
  player and an intermission don't run that Think, so the bullets freeze then, as in TS.
* Precaches: `crate::precache` issues TS's map-wide sequence (stock-entities.md §1.1) and calls
  `precache_events` (the 11 events, in `W_Precache` where `UTIL_PrecacheOtherWeapon
  ("weapon_tsgun")` runs `CTSGun::Precache`), `set_bubbles_model` (`g_sModelIndexBubbles`) and
  `precache_weapon_assets` (`TSPrecache`'s loop: per weapon id its shell, `v_`, `p_`, silenced
  `p_` and `w_` models and its two sounds, non-empty ones only).
  The other weapon sounds and models are in `TSPrecache`'s own lists.
* `on_spawn`: TSInit (bar 100, penalty 0, timers); then, as `PlayerSpawn` (0x890ac), with the
  player lent to its slot (`world::lend`), either the map's `game_player_equip`s touch the player,
  or (no equip) `give_named_gun` (`GiveNamedItem("weapon_tsgun")`: a `weapon_tsgun` entity at the
  player's feet with `SF_NORESPAWN`, spawned and touched: `add_gun`, a new empty gun, then
  `switch_to_gun`, deploy, kung fu unless the port-only bots already got a weapon; a player still
  holding a gun leaves the new one lying there) and `BuyArmament` from the `tkw`/`tki`/`tka` lists;
  speed, `KFuPower`. (`CustomWP` goes to a joiner from `CTSGameRules::InitHUD` while the
  custom-weapons flag is set: `weapons::inform_custom_weapons`.) The gun's records are emptied where TS loses
  its `weapon_tsgun` (`remove_all_items`), not here, and the `UpdateClientData` caches are not
  touched: a held gun keeps them, a new one gets `CTSGun::Reset`'s from `add_gun`. The free
  slots and loadout items were reset just before, by `weapons::ts_init` (TSInit).
* No gun (`SvState::has_gun` false: on an equip map whose equips don't list `weapon_tsgun`, and
  from the first death think until the next gun): as in TS no weapon frame (no attack, kung fu,
  reload; `ImpulseCommands`' +use still runs), `weapon_<n>`/`firemode`/`useitems`/`lastinv`
  ignored, `drop`/`drop_<n>` do nothing (the CTB case can't be dropped either), no pickups
  (`PickUpWeapon`: dropped guns, `ts_groundweapon`, knives, the grenade power-up), ammo packs and
  ground ammo left alone, the infinite-clip power-up refused, no weapon/ammo buys or
  attachments (kevlar still works), no `ActItems`/`WStatus`/`WeaponInfo`, `clientdata_t` stops
  after the weapon id (0), `GetWeaponData` all zero. Stunts and the other power-ups work. An
  equip listing `weapon_tsgun` gives a new, empty gun (`maprules::give_named_item`). TS tests
  `m_pActiveItem` (and `m_iId == 16`) on all these paths, so a gun holstered for a
  `cycler_weapon` counts as no gun there too (`gun_active`); holding it (`holds_gun`) matters only
  to `CanHavePlayerItem`, `HasWeapons`, `SelectItem` and `RemoveAllItems`.
* `pre_think` (skipped during intermission, where TS `PreThink` returns after `PlayerThink`): pressed buttons, `Gun::pre_frame` (recoil decay 0.1/s, bar regen 15/s, block and
  melee windows), `BlockPlayer` expiry, the delayed melee hit.
* `post_think`: `ItemPostFrame` while alive, not in intermission (`game_over`) and not on a
  tank (dead or game over, `PostThink` skips to the timer tail): once the active item is ready
  (`item_post_frame_ready`: the gun's `Gun::post_frame_ready`; a cycler's `m_flNextAttack` run
  out or its `m_iModel` 0; always with no active item), `ImpulseCommands` (`PlayerUse` and the
  impulses, `player::player_use::impulse_commands`) and then, with the gun active, the weapon
  frame (`Gun::weapon_post_frame`), and with any active item (a cycler's `ItemPostFrame` returns
  at once) the seed reroll (`RandomLong(0, 32000)`, as TS); timer decrement
  by `frametime * fuser1 * (DFR ? 1.5 : 1)`, speed, messages. Each shot and melee swing of the
  frame lends the player to its slot (`host::ServerHost::reach_player`: the inventory is moved out
  for the gun call and put back for the hit), so a `game_player_team` the hit fires changes the
  player at once (rules.md §7).
* `on_killed` (`Killed` 0x8ed5c): the active item's holster (gun or cycler), the ammo pack with
  the gun active; the CTB case
  (`DropFlag(false)`, gun or not); drop every weapon as `WorldGun` (a corpse's grenades become
  grenade power-ups, their record staying in the gun); the loadout lists (TS `ResetArmament`
  0x93e88). The free slots, the kevlar flag and the speed are what the drops leave (a worn vest
  keeps its 10 slots taken) until the next spawn's TSInit, as in TS. `ActItems 0` is the rules' (sent once, later in `Killed`); the player's
  `UpdateClientData` sends `ActItems` only while alive and holding the gun (`player::hud`).
* `remove_all_items` (`RemoveAllItems(TRUE)` 0xd8cec, from the first `PlayerDeathThink` while
  the player `HasWeapons`: the gun or a `cycler_weapon`): the active item holstered, the item
  entities (a picked-up `cycler_weapon`, then a map-placed `weapon_tsgun` that became the gun)
  `Drop`ped (removed 0.1 s later), the gun destroyed with what's left in it (reserve, records),
  then `remove_all_items_tail`: no view/weapon model, `pev->weapons = 0`, `UpdateClientData`
  inline (0xd8eb5), `CurWeapon 0,0,0`. Free slots, loadout items and speed are left alone
  (only TSInit and the item code change them), so `TSSpace` is not resent and `maxspeed` keeps its
  value. `StartObserver` and a disconnect call the same `remove_all_items`, `StartObserver` after
  its own holsters (`observer_holster`, 0x9971e..0x9974f): with
  the gun active, `CTSGun::Reset` (`gun_reset`: records, reserve, current weapon and the
  `UpdateClientData` caches +0x97a..+0x9de zeroed), `HolsterWeapon`, then `Holster`
  (`HolsterWeapon` again); a cycler gets its `Holster`. So the gun holsters three times in all,
  the third from `RemoveAllItems` after `deadflag` is set; a living player gets the kung-fu
  holster animation and draw event (iparam1 0) from each of the first two, a dead one from none,
  and no holster finds a record (a primed grenade is not thrown). `player_weaponstrip` uses
  `remove_all_items_keep_suit`, inside its `Use` even for a player whose own +use fired it
  (lent, `world::run_now_or_defer`).
* Weapon bits (`pev->weapons`, sent as `clientdata_t::weapons`): `AddToPlayer` sets bit 16 when
  the gun is added (`give_gun` without a gun: `PlayerSpawn`, an equip, a map `weapon_tsgun`) and
  bit 0 for a `cycler_weapon`; bit 31 is the HEV suit (`PlayerSpawn`). Only `RemoveAllItems`
  clears them (`= 0`, or `&= 0x80000000` from `player_weaponstrip`); `DropPlayerItem`, the
  other SDK place, is never called in TS, and the TS client never reads `iWeaponBits`.
* `cycler_weapon` (`CWeaponCycler`, stock-entities.md §2): `add_cycler` puts it in the HL item
  list and `deploy_cycler` makes it the active item when `FShouldSwitchWeapon` says so
  (`should_switch_weapon`: under the multiplayer rules only with no active item, the gun weighing
  1 to the cycler's 0; in single player always, holstering a held gun to kung fu first): its
  model as the view model, `m_flNextAttack` (`gun.next_attack`) 1, weapon anim 0 with the
  cycler's `body`. A gun given later holsters it (`m_flNextAttack` 0.5) and deploys.
  `use cycler_weapon` / `use weapon_tsgun` (`select_item`, `SelectItem`) switch between the held
  two the same way (the old item's `Holster`, the new one's `Deploy`), with no alive check. Tanks
  holster and redeploy whichever item is active. Nothing else runs for it: TS's `ItemPostFrame`,
  `UpdateClientData` and every gun path test `m_iId == 16` first (or read it as a `CTSGun`, see
  `gun_active`).
* `client_command`: `weapon_<n>`, `lastinv`, `drop`, `drop_<n>`, `firemode`, `useitems`,
  `lasclr_<n>` (`PlInfo`), `rst_cfg`, `tkw id_att ...`, `tki item ...`, `tka type_count ...`;
  test helpers (need `sv_cheats`): `ts_give id_att ...` (weapon + full ammo),
  `ts_weapons_debug n` (sets the cvar when typed in a listen server console).
* `fill_client_data` / `get_weapon_data` / `cmd_start`: TS packing (below), seed.

### Save data

`weapon_tsgun` (`CTSGun`) saves `CBasePlayerItem` (`m_pPlayer`, `m_pNext` always 0, `m_iId` 16)
and `CBasePlayerWeapon` (`stock::playeritem`): for a held gun the timers and clip come from the
holder's `Inventory::gun`, and a restore writes them back while that player's `gun_item` is this
entity. Pickups set the ammo types to -1 as `AddToPlayer` does. TS saves nothing else of the
gun's inventory (the 38 weapon records, reserve ammo, loadout), so a restored player keeps only
those. The player's item slots, `m_pActiveItem` and `m_pLastItem` (now kept: `SvState::last_item`,
set by `SelectItem`, cleared by `Spawn` and `RemoveAllItems`) are in the player's save data
(`player/save.rs`); `m_rgAmmo` is always empty in TS.

## 3. Constants and formulas (as implemented)

* Max carry (`get_ammo_info(t)+0x0c`, `assets::max_carry`): the column `Set_ammo_count`
  (0x7f2cc) last wrote for types 1..13 (`rules::state::ammo_column`). `W_Precache`'s
  `compile_ammo_info` writes the default column at every map; the end of `CTSGameRules::Think`
  writes `(int)ammocount` (truncated toward zero, not rounded) when it differs from the last
  value, so a change counts from the next Think. 1, 2 and 3 are the reduced columns (9 x 19mm:
  90, 30, 0; column 3 is 0 for every type); 0, 4, 5, negative or out-of-range values are the
  switch's default, the full carry (210). `ammocount 5` therefore gives the full carry, not 0.
* Timers: `dt = frametime * GiveSlowMul() * (DFR ? 1.5 : 1)`, `GiveSlowMul` = `fuser1` with a
  floor of 0.01 (`FixedSlowAmt`, `spread::fixed_slow_amt`); NPA clamps at -1, the others at
  -0.001; `UTIL_WeaponTimeBase() = 0`. The frame time is the engine's `(float)(msec * 0.001)` on
  the server and the same expression in client prediction (§5, floating point).
* Spread = `accuracy` (+ `pl.spread_penalty` via `ScaleSpread`), x0.875 silenced; growth
  `(1 - (slowmul*dt >= 1 ? 0.5 : slowmul*dt)) * acc_kick * (DFR ? 1 : 2) * (silenced ? 0.775 : 1)`
  for shots at least 1 ms apart, cap 0.1 (`double`), decay 0.1/s. Barrett: penalty 0.3 above
  35 u/s; M60: acc kick x5 above 175 u/s.
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
* Simulated bullets: when `TSGameRules+0x578` is non-zero (`assets::realbullet` =
  `rules::state::realbullet`: the `(int)realbullet` copy `CTSGameRules::Think` refreshes, truncated,
  -1 from `Init` until the first Think; never the live cvar) or the shooter's
  `GiveSlowMul() < 0.99`; 256 slots filled first-free and updated in slot order; step every
  `0.1/(realbullet+1)` s (every frame for >= 2) at `velocity_mps * 40 * dt * mult`, with
  `mult = GetBulletVelocityMult(s, bt)`: `s` is the shooter's slow-motion "others" amount
  (`pl+0x180`) while it's an activator, else `max(fuser1, 0.01)`; `bt` is the bullet-time flag
  (`pl+0x175`, `PlayerRules::btime`: slow pause, `ts_slowmotion` flag 4).
* Bubble trails: every pellet sends `TE_BUBBLETRAIL` (src -> end, `round(8192 * fraction / 64)`
  bubbles, the pierce shot `round(4096 * fraction / 64)`) when either end is at least 8 u under
  water. End and fraction come from `ShootBulletsPlayer`'s static trace: a simulated bullet's
  pellet reuses the last hitscan trace of anyone, as in TS (`fire::LAST_TRACE`).
* Slow motion of projectiles: grenades, knives (not while stuck in a wall) and falling
  `WorldGun`s start at the thrower's scale (`pl+0x180` if it's an activator, else
  `max(fuser1, 0.01)`) and run `SlowPhysics` every 0.1 s think: `velocity * s`, `avelocity * s`,
  `gravity * s²` from the saved pre-slow values, restored when `fuser1` returns to 1. As in TS,
  "saved" means a non-zero saved velocity (`CTSPhysicObject::SavePhysics` 0x7bac0): an object
  slowed at rest re-saves every think (its gravity and spin shrink by `s` each time) and is never
  restored, and a restore puts back the pre-slow velocity, so a slowed grenade or knife lying on
  the ground jumps off with its throw velocity when the slow ends.
* `dmgreport` (only with `g_flWeaponCheat`, `sv_cheats` as the map started; `damagemult` too): `> 1` prints `damage:%f`, `damage(piercing):%f` (bullets)
  and `(debug)%f` (thrown blades) to everyone as `HUD_PRINTTALK`; any non-zero value logs
  `<attacker> damaged <victim> (%f hp)` to the console from the player's TraceAttack.
* Melee: stock 35 u ahead hull 3, rate 0.725 (1.0875 stunting), damage `info.stock`
  (x2/3 stunting), bits 0x4001080; a blocked stock hit still plays `kevlarhit` (`ColdCockAttack`
  returns the target). Knife slash 35 u, `damage * meleemult` (x2/3 stunting), bits 0x1004, rate
  x1.5 stunting. Katana (disassembly 0x73580..0x737a8): reach 50 u (35 stunting, 55 backward,
  behind with hull 1), damage = the swing's strength x meleemult: stunting 35, backward 75,
  airborne `clamp(speed*0.1538, 35, 55)`, ground 45 (alternating view index 0x11/0x10), crouched
  65; bits 0x1080, delays 1.0 / 1.0 / 1.1 / 0.7 / 0.9. A knife or katana slash that hits a player
  sets the gun's `pev->body` to 1 (`Gun::body`, sent with the grip-switch and idle animations).
  Knife, katana and kung fu store their delay in `pl+0x4f8` (the stunts' close-combat cooldown).
  Targets: knife/katana need `IsAlive` and no `EF_NODRAW`; kung fu and the stock need
  `takedamage`, no `FL_WORLDBRUSH`/`EF_NODRAW`, and not a teammate under `mp_friendlyfire 0`.
  Thrown: knives 1900 u/s, the katana 1000 u/s (`KatanaThrow` 0x73dc1), 100 / 50 / 150, bits
  0x8001000. Block windows 0.5 s, cost 33.
* Kung fu (`CCombatAttack`, decoded from the disassembly 0x76078..0x76e19; damage, delay,
  delayed-hit window, reach): punches 10 / 0.25 s / 0.1 / 35 u (held jab), 20 / 0.35 / 0.15 / 35
  (press), 30 / 0.65 / 0.15 / 35 (combo end or duck), air 25 / 0.5 / 0.15 / 35; kicks: uppercut
  35 / 1.0 / 0.35 / 45, back kick `clamp(speed*0.303,15,80)` / 1.35 airborne, 1.0 ground / 0.3 /
  55 behind (hull 1, `v_angle` with roll 0), flying kick `clamp(speed*0.1538,15,45)` / 1.0 / 0.25
  / 45, combo kick 25 / 0.5 / 0.3 / 45, quick kick 15 / 0.3 / 0.2 / 45, speed kick
  `clamp(speed*0.1,15,30)` / 0.75 / 0.25 / 45; stunting 15 / 0.55 / 0.15 / 35. x meleemult, x2
  with the kung-fu power-up. Ground kicks (and the back kick on the ground) `BlockPlayer(delay *
  0.75)` unless `iuser4 & 1`. The uppercut and the flying kick force a crouch (`iuser4 |= 4|1` /
  `|= 4`, `view_ofs` 16, stunt recover timer 2 s), as does the crouched katana swing; the
  uppercut's bit 1 means it never blocks. Leaving a wall climb sets `view_ofs` 28 (16 ducking)
  and gravity/friction 1. The crouch tests of the uppercut and the katana read `FL_DUCKING` on
  the gun entity's own pev, so only `+duck` counts. Bits 0x4001080, disarm (not during the
  victim's spawn protection, `pl+0x71c`) + knockback (`v.vel += |a.vel|*dir`, `z = 235` above
  30). Each attack picks two numbers: the event's `i2`, an index into kung fu's set A that the
  client plays (stack slot `0x8(%esp)`, pushed at `0x76daa`; the client copy
  `0x1000b0b0..0x1000b4ea` picks the same), and a move id kept in `gun+0xa04`
  (`Gun::last_melee_anim`): the kick combos chain on it and `CloseCombatAttack` plays it as the
  `CTSAnimation` id. Jab 3 / move 22, press 4 / 23, combo end 11 / 25, air punch 5 / 24,
  stunting 4 / -1, uppercut 6 / 20, back kick 7 / 17, flying kick 8 / 16, quick kick 9 / 11-12,
  combo kick 10 / 10, 13-15, speed kick 10 / 15, 18, 19. The client copies never block, crouch
  or touch the stunt timers.
* Grenade: pitch `p*(p>=0 ? 10/9 : 8/9) - 10`, speed `min(750, (90-p')*7)` (roll x2/3, from the
  feet, friction 0.4, spin (-100, 600, 0)); fuse 3 s of `max(fuser1, 0.01)` time;
  `TSExplosion(110, 0x1040, 300, 1024)` only while `IsInWorld()` (origin inside ±4096 and
  velocity inside ±2000 on each axis; no sky test, so a grenade in the sky explodes, one outside
  the box is removed silently); flying impact `speed*0.1` if it would kill the target,
  credited to the thrower (deviation, §5).
* Explosion slow-motion point: after the damage, `DoSpectacularNess()` (`rules::scoring::do_spectacularness`; `spectacularness`
  clamped 0..100 and rounded, 0 = never, else `RandomLong(0,100) <= it`) creates a
  `ts_slowmotionpoint` at the blast (amount 0.25, 1 s, no bullet time).
* Ammo pack touch (alive player holding the gun): every type goes through `AddAmmo` (whole
  amount clamped to the max carry, if the reserve is below it; no weapon needed), then the pack
  is removed whether or not anything fitted. Without the gun nothing happens and it stays.
* Slots: 81 free at spawn, speed `round(210 + free*120/81)` (330 with nothing carried).
* Disarm threshold: katana 30, knives 25, weapons with < 21 slots 20, else `min(slots, 50)`.
* Dropped guns live `clamp(weaponstay, 5, 60)` s; `drop`/disarm drops `clamp(2*weaponstay, 10,
  120)` s and glow for the dropper (`WorldGun` +0xc4 / +0xd0, `DropWeapon` 0x94ed9, 0x94fad,
  0x95265): both halves of a split akimbo pair, never the CTB briefcase.
* A thrown blade keeps its thrower only in the private +0x8c (`CreateKnife` 0x7d9e4); its
  `pev->owner` stays null. The engine collides it with the thrower, and `BounceTouch` returns
  at once for them, so it bounces off without hurting them. The thrower is not sent as
  `entity_state.owner`.
* `AddToFullPack` (0xaf180) glow shell (`rendermode 0, renderamt 2, renderfx 19, colour
  100,100,100`): a `WorldGun` whose dropper (+0xd0) is the viewer, or a `knife` whose thrower
  (+0x8c) is (`weapons::glow_for_host`). Guns dropped on death or by `drop_<id>` have no dropper.

## 4. Wire format

Events (all `FEV_NOTHOST`, invoker = player, zero origin/angles), exactly weapons.md §6.1:
`fire` (f1=f2=spread, i1 id, i2 attachments, b1 clip==0 / akimbo toggle / pump mode, b2 seed),
`firelinked`, `pump`, `empty` (none for the Contender), `reload` (f1 0 or 8/9/10), `draw`,
`coldcock` (f1/f2 last spread offset), `knife` (i2 hit flag), `katana` (i1 swing = anim index
15/16/17, i2 hit flag), `grenade` (i1 0x13/0x14/0x15), `closecombat` (attack: i1 type 1 punch /
2 kick / 3 back kick, f1 0, i2 index into kung fu's anim set A; block: f1 weapon id, i1 0, i2 1
kung fu / 0x12, an index into that weapon's set A).
`bparam2` is passed as the raw seed int (the engine only transmits its low bit, like TS).

Messages: `WeaponInfo` (B id, B clip, S reserve, B mode, B items; kung fu: B 0, B bar, 0,0,0),
`WStatus` (B id, B owned, B clip; when the owned flag changes and for the akimbo/single twin
with each `WeaponInfo`), `ActItems` (ALL: B entindex, B active | 0x40 akimbo) and `TSSpace`
(B free; both sent by `player::hud`, from `CBasePlayer::UpdateClientData`), `KFuPower` (C round(bar)), `CurWeapon` (0,0,0), `PlInfo` (ALL: B entindex,
B 0, B colour), `CustomWP` (string), `TSMBlur` (PVS, kung fu), `TSFrags` (explosion),
`ScreenShake`, `TSFade`, `TSMessage` (custom weapons notice). `CurWeapon 0,0,0` goes out from
`RemoveAllItems`: the first death think (not `Killed`) and `StartObserver`.

`clientdata_t` (sendweapons): `m_flNextAttack`, `vuser1[1]` power-ups, `m_iId 16`,
`vuser3 = (id, mode, owned attachments)` (kung fu: `(0, kick combo, punch combo)`),
`vuser4 = (akimbo toggle, clip, in reload)`, `ammo_nails` left akimbo clip, `ammo_shells` twin
record clip, `vuser2 = (state counter, reserve, alt grip)`, `vuser1[2]` active attachments,
`viewmodel` = model index of `pev->viewmodel`. `weapon_data_t[16]`: `m_iClip` (clip mirror),
NPA/NSA/idle clamped at -0.001, `fuser1` = the gun's own timer, the rest 0.
`shared::protocol::weapons::{unpack_clientdata, unpack_weapon_data}` are the client side (only the
client calls them). The client rebuilds the akimbo right clip as `clip - left`, 0 when below 1
(`0x10003ae1..0x10003aed`). The client prediction runs `Gun::weapon_post_frame` behind TS's client
gate (`deadflag != 5`, last `TSHealth` > 0, `m_flNextAttack <= 0`), not `Gun::post_frame_ready`.
`HUD_WeaponsPostThink` (`0x10003830`) has no `PreThink` part, so the prediction runs no
`Gun::pre_frame`: the melee windows are never decremented (the client never arms `pl+0x788` and
never reads `+0x794..+0x79c`; the death holster's check `0x1000abae` is the only reader) and
`client.dll` keeps no kung-fu bar. Its kung-fu block (`0x1000adc0`) gates on the HUD's power
instead, `0.01 <= 0x1018a718`, the copy the TS weapon element's `Think` (`0x1004cf5e`) makes of
the networked, locally regenerated `KFuPower` value on each `Think` (`state::Shared::kfu_power`), and
charges nothing (no `UpdateMeleeBar`, so no 1.5 s `m_flNextAttack` for an empty bar):
`GunHost::kfu_power`, `None` on the server, which gates on and charges its own bar.

## 5. Deviations and decisions

**Floating point, per TS build.** `ts_i386.so` (GCC, x87 extended precision), `mp.dll` and
`client.dll` (MSVC, x87 at 53 bits) compile the weapon code separately, as they do movement
(`movement.md`). Audited against the three binaries' machine code, for every value the server and
the prediction both compute:

| Value (TS function) | `ts_i386.so` | `mp.dll` / `client.dll` | Port |
|---|---|---|---|
| Frame time (`SV_RunCmd`; client `HUD_WeaponsPostThink` 0x10003eb7) | engine `(float)(msec * 0.001)` | client: `fild msec; fmul qword 0.001; fstp` (the same) | `frame_time` = `(msec as f64 * 0.001) as f32`. **Was** `msec as f32 * 0.001`, a different float for 141 of the 256 msec values (10 ms too): predicted timers ran out a frame before the server's (an AK-47's 0.08 s at 10 ms: 8 commands instead of 9), so the client predicted shots the server fired a command later with another seed. Fixed |
| Timer step `slow * ft * 1.5` (PostThink 0x91ae7, PreThink 0x909c5; mp 0x1006053c; client 0x10003ec5) | the unrounded product × 1.5, stored once | the product stored as a `float`, then × 1.5 | `spread::timer_step(arith)`: both. Differs only in slow motion with double fire rate, by one float ulp; no frame-count difference found |
| `GiveSlowMul` (`FixedSlowAmt` 0x990bc, client 0x10055aa0) | floor 0.01 (`double`) | floor 0.01f | `fixed_slow_amt` (0.01f). **Was** `max(fuser1, 0)`: fixed |
| Timer decrements and clamps (PostThink tail; mp 0x10060592; client 0x10003f40) | the difference of two floats is exact, one rounding | the same | f32: equal |
| `ScaleSpread` elapsed test (0x802a8; mp 0x10053ee0; client 0x10008100) | stored float `>= 0.001f` | the unrounded difference | `spread::shot_interval(arith)`: equal in practice (nearby times subtract exactly) |
| `ScaleSpread` growth | `x = (float)(slow * dt)`, the test `slow*dt >= 1` on the unrounded product, `pen + (1-x)*akick*k` in extended precision, one store | `x` unrounded, `(1-x)*k*akick + pen` at 53 bits | `spread::grow_spread_penalty(arith)`: both. **Was** f32 throughout: a product just below 1 rounded to 1.0 and added 0.5·akick·k instead of almost nothing, and about one penalty in ten was an ulp off. GCC ≠ MSVC only in slow motion (an ulp) |
| Penalty decay (PreThink 0x80d88; mp 0x1005fa29; client 0x100040a5) | `pen - dt*0.1f`, one store, the unrounded result `< 0` clears | the same at 53 bits | `spread::decay_spread_penalty(arith)`. **Was** two roundings (an ulp off in ~6 % of frames, never a frame's difference in reaching 0) |
| Melee bar regeneration (PreThink 0x80ddc; mp 0x1005fa65) | `bar + dt*15`, one store | the same | `spread::regen_melee_bar(arith)` (was two roundings) |
| `VKickback` (0x80414; mp 0x10054000; client 0x10008230) | yaw from the unrounded `0.85*kick`; silenced pitch from the stored pitch | yaw the same; silenced pitch from the unrounded product | `spread::view_kick(arith)`: both (was an ulp off in ~20 % of kicks). The server applies it only to bots |
| `GetMeleeMultiplier` × damage (0x965d0; mp 0x10063e60) | `(max(bar,0)*0.5f + 50f) * 0.01f` left on the FPU, product stored once | the same | `spread::melee_damage(arith)`. **Was** a `double` 0.01 and two roundings (1-2 ulp in ~30 % of hits). Server only |
| `UTIL_SharedRandomFloat` (0xebc54; mp 0x10076190; client 0x1001aea0) | range truncated by a 64-bit `fistp` | `mp.dll`: `__ftol`, the same; `client.dll`: the float range, untruncated (TS's client bug) | server: `weapons::random` (exact for both servers); the client's fire events use `events::random`, `client.dll`'s, as TS's client did |
| Pellet offset/direction (`ShootBulletsPlayer` 0x6f69c; client `EV_TS_FireBullets`) | `1 - r³`, `fcos`/`fsin` in extended precision, components rounded once | 53 bits, different order | `f64` trig, f32 vector steps: an ulp off at most (a few 1e-4 units at 8192). Not changed |
| Speed gates (Barrett 35, M60 175) | the stored `sqrt` | unrounded | f32: differs only exactly at the boundary. Not changed |

Where the builds differ the function takes a `shared::pm::Arith` (`GunHost::arith`): the server
passes its `ts_pm_arith` selection (default `msvc`, `mp.dll`, which is what every client
predicts with; `gcc` for `ts_i386.so`), the prediction `Arith::CLIENT`. With the default the
server and the prediction compute these values bit for bit alike. As in `client.dll`, the
prediction adds, grows and decays the penalty, and moves the last-shot time, only on a command's
first prediction (`g_runfuncs`, `GunHost::first_prediction`; a re-predicted shot's spread has no
penalty), and decays it after the timers, dead or alive (`0x100040a5`), where the server's
`PreThink` decays it before the weapon frame while alive. TS's server and client also differ in
the frame time that decay uses: `PreThink` sees the previous command's (the engine sets
`frametime` before `PostThink`), the client the current one; both sides of the port keep their
build's. NaN inputs take each build's compare direction.

Remaining deviations from TS:

* **Deviation: the flying impact credits the thrower** (issue #1, a deliberate choice). TS passes
  `this->pev` as the TraceAttack attacker and as both `ApplyMultiDamage` arguments in
  `CM61Grenade::WaitThink` (0x7cacc; see the disassembly at 0x7ce3c/0x7ce46), `CTSKnife::WaitThink`
  (0x7ea30) and `CTSWorldGun::Fall` (0xa0d84), not `pev->owner`, so in TS a breakable broken, or a
  player killed, by a flying projectile isn't credited to the thrower. The port keeps the
  projectile as the inflictor but makes the thrower the attacker (`projectiles::Thrower`, used by
  `impact_damage` and the `WorldGun` fall damage): breakables take the thrower's time scale and
  name it in the `broken by` log, kills are the thrower's, and the team / friendly-fire checks see
  the thrower. The thrower is checked at impact time: it must still be a connected client
  (`FL_CLIENT`) with the user id it had at the throw, so a player who left, or a new client in the
  same slot, is never credited; then the projectile is the attacker, as in TS.

Port additions and defensive checks (TS has no equivalent, or would read invalid memory):

* `ts_weapons_debug` (cvar) and `ts_give` (cheat command) are port-only test aids.
* Bots (port-only) get their loadout from the rules' `on_spawn`, before `weapons::on_spawn`,
  which therefore deploys a weapon the player already holds instead of kung fu.
* `CBulletManager::UpdateBullet` reads the shooter without a check; the port drops a bullet
  whose shooter edict is null or freed.
* `BuyArmament` buys each `tka` ammo type as many times as the client asked (any `int`); the
  port stops at the first purchase that adds nothing (the reserve is full or the type invalid),
  since every later one would add nothing too, instead of looping billions of times at spawn.
* `tki`/`tkw`/`tka` and `weapon_<n>` parse with `sscanf("%i")`; `-2147483648` negates with C
  wrapping rather than overflowing. Gun arithmetic on clips, reserves and fire modes (which the
  client also runs on networked values for prediction) wraps like TS's C `int`s; `AddAmmo`'s sum
  saturates before the `max_carry` clamp. `UTIL_SharedRandomLong` over the full 32-bit range
  (`range` wraps to 0, a division by zero in TS) returns `low + rnum`.
* `DropWeapon` on an owned id without a table entry returns instead of reading TS's entry 0.
* TS precaches `models/p_pdw_sil.mdl`, shipped as `p_pdw_Sil.mdl`. Engines that look files up
  case-insensitively (Windows, FWGS) load it under TS's name; on a case-sensitive server, where TS
  stops at map load, the port precaches the shipped spelling instead
  (`shared::weapons::MP7_SIL_ON_DISK`).

How the port maps TS where the structure differs (not deviations):

* **The gun's lifetime.** TS gives a fresh `weapon_tsgun` at `PlayerSpawn` (only on maps without
  a `game_player_equip`, or from an equip listing it) and loses it in `RemoveAllItems` (the first
  death think after `Killed`, and `StartObserver`); a spawn while a gun is still held keeps it
  (`AddPlayerItem` kills the new duplicate). The port's `Gun` per player stays allocated;
  `SvState::has_gun` says whether TS's gun exists, `add_gun` sets it with an emptied inventory
  and fresh send caches (`new_gun_state`), and `remove_all_items` (death think, `StartObserver`,
  disconnect) clears it and empties the records. `reset_for_spawn` gives the fields a fresh gun starts with. TS's gun is a
  `weapon_tsgun` entity attached to the player, and so is the port's (`SvState::gun_item`, from
  `GiveNamedItem` or a map-placed one, dropped with the gun), so edicts are allocated and numbered
  as in TS; the gun's state lives in the player's `Gun`, not in the entity. A `GiveNamedItem` gun
  stays on its `FallThink` while held (it never lands), thinking every 0.1 s, as TS's does.
* **A gun holstered for a `cycler_weapon`.** `SvState::has_gun` (held) and
  `SvState::cycler_active` (`m_pActiveItem` is the cycler) together give TS's active item
  (`active_item`); gun paths use `gun_active`.
* **Multi-damage.** Hits go through TS's `ClearMultiDamage` / `TraceAttack` / `ApplyMultiDamage`
  (`damage::multi`): `ShootBulletsPlayer` clears once (not with simulated bullets), sets the type
  0x1002, traces every pellet and applies after the loop, so the pellets that hit one victim in a
  row are one `TakeDamage` with their sum (a pellet on another victim first applies the previous
  one's sum); the pierce shot, each simulated bullet (`UpdateBullet`) and every other caller
  (melee, explosions, thrown and falling objects, `func_tank`, beams) is its own
  Clear/TraceAttack/Apply. `glast_headshot` (any head hit since the last Apply) picks the
  `#TS_Headshot` message. The buffer holds only TS's entity, amount and type, and `TakeDamage`
  gets TS's four arguments (`DamageInfo::new(inflictor, attacker, sum, type)`): hitgroup,
  direction and impact point only reach `TraceAttack`, and there is no weapon id in `DamageInfo`.
  What a kill reads besides those are TS's globals (`damage::glast`): `glast_closecombat` around
  the hit of `CloseCombatAttack` (kung fu, `melee::attack`) and the stored kung-fu hit of
  `CheckForBreakables` (`melee::pending_hit`), `glast_coldcock` around an unblocked
  `ColdCockAttack`, `glast_weaponid` around a simulated bullet's hit (`bullets.rs`); knife and
  katana slashes set none of them. `g_vecAttackDir` (`damage::attack_dir`) is set by the
  victim's `TakeDamage` (rules.md; also `CBaseEntity::TakeDamage`, breakables and dead monsters:
  stock-entities.md). `multi::add` takes TS's four `AddMultiDamage` values only.
* A gun's original owner (`s_weapon_status+0x30`, `SvState::owners`): a buy sets it (and the
  akimbo pair's, when owned); a dropped gun carries it, and picking it up copies it into the
  record `TakeWeapon` fills (`inventory::copied_record`: the id, or its akimbo pair; a NULL owner
  too). A fresh pick-up without a record (a thrown knife, a power-up grenade) leaves the old one,
  as TS never clears it. `PlayerKilled` logs ` (victims_weapon)` from it (rules.md).
* Thrown blades lend themselves back to their slot while their touch or think deals damage
  (`world::lend`), so the kill they score names them (`weapons::thrown_knife_description`,
  `CTSKnife::GetDescription` 0x7db04).
* The loadout is cleared at death (TS `Killed` -> `ResetArmament`); the TS client resends
  `rst_cfg` + `tkw/tki/tka` from its buy menu.
* M61 buys and stealth shoes are refused, as in TS 3.0 (only allowed when
  `CTSGameRules::m_mode > 1`, unreachable). `weapons::set_round_mode_items(true)` enables them,
  and then a corpse's grenades drop as `WorldGun`s instead of grenade power-ups.
* `VKickback` writes `punchangle` (`pev+0x68`) only for bots (`FL_FAKECLIENT`); humans get the
  kick from their client's fire event.
* `WorldGun` and thrown-blade ground friction are TS's alone (issues #64, #66): no GoldSrc engine
  slows a sliding `MOVETYPE_BOUNCE` entity, and the engine runs `BounceTouch` (0xa0b0c: x0.9
  above `40 * GiveSlowMul()`, x0.7 at or below) / `CTSKnife::WaitTouch` (0x7dce8: x0.5, stop
  below 32 u/s) on every frame the object is pushed into the floor, so the slide depends on the
  server frame rate as in TS. The gun's drop sound plays on every fast floor hit. A blade stuck
  in a wall is `SOLID_NOT` and skips the slow-motion physics.
* `WorldGun` `WaitTouch` doesn't check the 0.5 s pickup delay (as TS: only `Use` does).
* `CloseCombatAttack`, `CCombatAttack`, `KatanaFire` and `ThrowWeapon` are garbled in the
  decompile (stack slots shared between unrelated values); their numbers come from the
  disassembly (addresses in §3 and in the code).

## 6. Missing / partial

* `SVC_WEAPONANIM` is sent only to clients that don't predict weapons (TS behaviour); the client
  package plays view animations from events.
* The briefcase/flag (rules).
* `DropArmor` 0x93a3c armour packs (`CTSAmmoPack` with `armorvalue`) aren't ported. They're dead
  in TS: nothing calls `DropArmor`, so ammo packs never carry armour there either.
* `CTSShrapnel::Create` 0x7bf9c isn't ported: nothing in the decomp calls it. A map-placed
  `shrapnel` is (`entities::ts::shrapnel`, #95; see stock-entities.md §2).

## 7. Tests

* Floating point per build (`shared::weapons::spread` tests, reference bits from exact models of
  the x87 at 64 and 53 bits): `timer_step_per_build` (DFR in slow motion: GCC 0x3a7f9725, MSVC
  0x3a7f9724), `spread_penalty_growth` (incl. a `slowmul*dt` of 0.99999998 adding nothing),
  `penalty_decay_and_melee_bar`, `kick` (silenced pitch per build), `melee_and_bullets`
  (`20 × multiplier(95.6)` = 0x419c7ae1). `cargo test -p ts-client`:
  `frame_time_is_the_servers` (every msec equals FWGS's frame time; an AK-47's 0.08 s takes the
  server's 9 commands at 10 ms, 8 with the old `float` 0.001).

* `cargo test -p shared`: weapon tests pass (x86_64 and `--target i686-unknown-linux-gnu`). Includes `random::matches_sdk_c_bit_for_bit` (43 seeds
  compared against the SDK C `util.cpp` compiled with `gcc -m32 -O0 -mfpmath=387`), fire rates
  (auto, semi, DFR x1.5, slow motion), burst (3 / UMP 2), pump alternation, magazine and
  3-phase shotgun reloads, recoil penalty growth/cap/decay, silencer multipliers, akimbo merge +
  linked shot (seed + 0x21, +-5 offsets, clip split, single mirror), zoom steps (AK 32/13 and the
  hidden view model), grip toggle, grenade pull/throw/slots, throw math, knife stacks, kung-fu
  punch + block + bar regen, Infinite Clip, priority switching, pickup limits, mode memory,
  message layouts, `CustomWP` round trip, clientdata/weapon_data packing, the kung-fu and
  katana reach / damage / window / block / forced crouch per move (`kungfu_reach_anim_window_and_block`,
  `katana_swings`).
* `cargo test -p ts-server`: command argument parsing (`%i_%i`), `SlowPhysics` (scale, rescale,
  restore, TS's gravity compounding at rest), ammo pack contents (`take_ammo`), the explosion's
  `WorldGun` knock-back (`blast_velocity`, `ground_hop`) and damage falloff (`falloff_damage`),
  the thrown blade's ground slide (`ground_slide`), glance-or-stick test (`glances_off`) and
  hit pitch (`hit_pitch`), the gunless `clientdata_t` packing
  (`clientdata_without_the_gun_stops_after_the_weapon_id`), and the multi-damage
  (`damage::multi::tests`: pellets summed into one `TakeDamage`, a new victim flushing the old one
  with the accumulated bits, no victim no apply, `glast_headshot` reset, `TakeDamage` getting only
  the four arguments) and the `glast_*` set-then-reset (`damage::glast::tests`), the blade
  names (`knife_descriptions_follow_the_blade_type`), the record a dropped gun's status lands in
  (`dropped_gun_status_lands_in_takeweapons_record`), and the weapons file: line rules, `%i`
  hex/octal, `saveweapons` text reading back (`custom::tests`). Issue #106: TSInit's slots and
  items with a held gun's caches kept (`ts_init_frees_the_slots_and_drops_the_loadout_items`),
  `RemoveAllItems` leaving slots, items and speed (`destroying_the_items_leaves_slots_items_and_speed`),
  a new gun's `CTSGun::Reset` caches (`a_new_gun_starts_with_reset_send_caches`), no kung-fu
  `WeaponInfo` from a fresh gun and the on-change rules (`net::tests`), and `Deploy` zeroing the
  update timer only with a weapon (shared `deploy_with_a_weapon_updates_the_client_at_once`).
* **Not yet tested in engine**: a thrown knife's kill naming the blade in `DeathMsg` and the log,
  ` (victims_weapon)` after picking up a victim's dropped gun, `loadweapons` then a map change
  (no notice for new joiners, kevlar back at 0.2125).
* **Not yet tested in engine**: slowed projectiles (power-up slow motion / `globalslow` with a
  grenade, thrown knife and dropped gun in sight), bullet-time simulated bullets (slow pause),
  bubble trails under water, `dmgreport 2` with `sv_cheats 1`, +use on a flying/landed gun, on a
  grenade resting on the ground (the kick) and on a stuck knife/katana, the explosion slow-motion point (`spectacularness 100`),
  a grenade (or an exploding breakable) next to a dropped gun: it flies off, lighter guns
  further, and a landed gun hops. The slide of a thrown blade on the floor without the old
  think friction (issue #66): `WaitTouch` halves it every frame, so it should still stop within
  a few frames of landing.
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
  selects it; `attachments` is a `u32` because a ground weapon passes its whole spawnflags, as
  `GetWeapon` 0xa2328 does: `TakeWeapon` masks a new record with the allowed attachments but ORs
  them unmasked into the single of a new akimbo pair), ammo: `weapons::give_ammo(p, cx,
  ammo_type, rounds)` (returns rounds added; a ground weapon's `wextraclip` gives that raw number
  of rounds, only if the reserve is empty after the pickup).
* Ground weapon ids (`entities::ts::ground`, `CTSGroundWeapon::Spawn` 0xa1b40): an id without a
  valid `get_weapon_info` entry (kung fu 0, the briefcase 29, anything outside 0..38, which reads
  entry 0) gets `SUB_Remove` 0.1 s later; an akimbo pair built from two singles (kind 4,
  `info+0xcc` != -1: 8, 10, 16, 30, 37) is turned into its single, since `PickUpWeapon` refuses
  those pairs. The Golden Colts (21, kind 5) stay. Both unit tested (`ground::spawn_weapon_id`).
* Power-ups: Infinite Clip -> `weapons::multiply_clip`; grenade -> `give_weapon(p, cx, 24, 0)`;
  kung fu sets `Player::kungfu` (x2 melee), DFR/slow-mo are read from `Player::active_powerup`
  and `pev->fuser1`.
* `rules::on_killed` calls `weapons::on_killed(p, cx)` (holster, ammo pack, CTB case, drops,
  loadout reset) and sends `ActItems 0` itself; the first death think calls
  `weapons::remove_all_items` (the gun, then `remove_all_items_tail`). `StartObserver` /
  disconnect call `weapons::remove_all_items` too (TS `RemoveAllItems(TRUE)`, with its inline
  `UpdateClientData`, then `CurWeapon 0,0,0`); a spawn without either keeps the gun, as in TS.
  The spawn's TSInit block calls `weapons::ts_init` (slots, items).
* `CTSGameRules::Think` (`rules::frame`) calls `weapons::bullet_think` (the simulated bullets).
* Restrictions: `weapons::set_restriction(mask)` (`ts_mapglobals` spawnflags: `0x1f ^ flags`),
  `weapons::restriction()` (the whole int, as `BuyWeapon` 0x945bc passes it to
  `IsRestrictedWeapon`; not masked to 0x1f); broadcast it as `SrvSett 3`; the rules broadcast `realbullet` as `SrvSett 0` and
  `ammocount` as `SrvSett 2` (`rules::frame::srvsett_think`; cvars registered by the rules).
* Armour absorb: `rules::state::armor_absorb()` (`TSGameRules+0x588c`: 0.2125 from every
  `Init`, or a weapons file's value).
* `Player::slots_used` and `maxspeed` are written by weapons only when the slot count changes
  (`SvState::speed_for`, port bookkeeping; `ts_init` records TSInit's 330 as the speed for 81).
* Kevlar: every armour source (loadout `tki 16`, the armour power-up, `item_battery`,
  `func_recharge`) goes through `weapons::take_armor` (`TakeArmor` 0x93da4). The first vest needs
  10 of the shared `free_slots` (`pl+0x548`, the same count weapons and the CTB case use) and sets
  `SvState::items & 0x10`; `take_armor(-1)` takes it off and gives the slots back. Death,
  `StartObserver` and a disconnect leave `free_slots` and `items` alone; the spawn's TSInit
  (`weapons::ts_init`: `pl+0x548 = 81`, `pl+0x718 = 0`) resets both. Stealth shoes are `items & 0x20` plus `iuser4` bit 0x200 (`stunt::STEALTH_SHOES`, which
  the decomp docs call `HAS_KEVLAR`).

**Damage package**: `DamageInfo.hitgroup` = trace hitgroup, bits as listed in §3 (0x2000000 =
shotgun headshot x1.5, 0x8000000 = thrown blade, 0x4000000 = close combat, which breakable
doors test; kills read `glast_closecombat` instead, see the multi-damage note above).
`PTakeDam` is yours.

**net.rs (AddToFullPack)**: `weapons::glow_for_host(ent, host)` says when TS forces
`rendermode 0, renderamt 2, renderfx 19, rendercolor 100,100,100` (own dropped gun / thrown
knife). Players' `weaponmodel` (p_ model) is set in `pev->weaponmodel`; `AddToFullPack`
sends its model index, which the client draws merged onto the player (third person, other
players, motion-blur ghosts) and shows in-eye as the spectated player's view model.

**Client / prediction package**: drive `shared::weapons::Gun` with a client `GunHost`:
`unpack_clientdata` + `unpack_weapon_data` before the command (no `Gun::pre_frame`, §4),
`item_post_frame`, `decrement_timers(Gun::timer_dt(Arith::CLIENT, frame_time(msec), fixed_slow_amt(fuser1), powerups))`; host events
go to the local event handlers, `shoot` can be a no-op (the `fire` event re-traces with
`spread::pellet_dir` and the same seed), `kick` feeds the view kick (timed with the prediction
time, as TS's client `VKickback`). The client keeps the weapon records between commands (only the
current one is rebuilt from `from`) until its gun's Spawn on a respawn or after `HUD_VidInit`, and
never deploys on a networked weapon change: the draw animation is the server's `svc_weaponanim`
(`Deploy` -> `DefaultDeploy` skiplocal 0), as in TS (see `client-rendering.md`). Messages decode with
`shared::protocol::weapons::{WeaponInfo, WStatus, ActItems}::read`, `read_kfu_power`,
`parse_custom_wp` (apply with `shared::weapons::table::set_weapon_params`). View-model sequences:
`shared::weapons::anim(id, alt, index)`.

**Movement package**: `Inventory::anim_extension()`, `Inventory::current()`; the weapons code
sets `iuser4 & 1` through `BlockPlayer` (and clears it when its timer runs out) and clears
`0x20000|0x4` when a kung-fu attack ends a wall climb.
