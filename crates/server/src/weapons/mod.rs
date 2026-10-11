//! Server half of TS weapons: the per-player CTSGun inventory, firing, hit resolution,
//! penetration, grenades, knives, kung fu. Predicted logic is shared::weapons.
//! Owner: weapons work package. Reference: decomp docs/server/weapons.md,
//! docs/implementation/weapons.md.

mod assets;
mod bullets;
mod commands;
mod custom;
mod fire;
mod host;
mod inventory;
mod melee;
mod net;
mod projectiles;
pub mod worldgun;

/// The classnames the weapons modules link (the save-table tests).
#[cfg(test)]
pub(crate) fn linked_entities() -> Vec<crate::world::Linked> {
    let mut v = worldgun::linked_entities();
    v.extend(projectiles::linked_entities());
    v
}

use crate::commands::Args;
use crate::player::Player;
use crate::world::Cx;
use goldsrc::ext::entities::EHandle;
use goldsrc::server::engine;
use goldsrc::sys::{clientdata_t, edict_t, usercmd_t, weapon_data_t};
use shared::weapons::{Gun, MAX_AMMO, MAX_WEAPONS, Status};

#[allow(unused_imports)]
pub use assets::{cvars, dmgreport, restriction, set_restriction};
pub use inventory::Drop;
pub use projectiles::{SlowPhysics, bounce_ground_damping, give_slow_mul, thrower_time_scale, thrown_knife_description};

/// Free inventory slots at spawn (`TSInit`: `pl+0x548 = 81`).
pub const SLOTS: i32 = shared::ts::SLOTS_TOTAL;

/// One loadout entry (`pl+0x594`, 32 x {id, attachments, isItem}).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ArmamentEntry {
    pub id: i32,
    pub attachments: u32,
    pub item: bool,
}

/// Per-player server state around the shared gun.
pub struct SvState {
    /// The player holds TS's one weapon item, `weapon_tsgun` (`CTSGun` in `m_rgpPlayerItems[2]`,
    /// [`holds_gun`]). It is `m_pActiveItem` (pl+0xa10) too unless a `cycler_weapon` was switched
    /// to ([`cycler_active`](Self::cycler_active)). `PlayerSpawn` gives it only on maps without a
    /// `game_player_equip`, and `RemoveAllItems` destroys it (death think, observer). Without it
    /// active there is no weapon play at all, kung fu included: see [`gun_active`].
    pub has_gun: bool,
    /// Loadout (`tkw`/`tki`), max 32 entries, and ammo purchases per type (`tka`).
    pub armament: Vec<ArmamentEntry>,
    pub armament_ammo: [i32; MAX_AMMO],
    /// Free inventory weight (`pl+0x548`); `Player::slots_used = SLOTS - free_slots`.
    pub free_slots: i32,
    /// Loadout items bought (`pl+0x718`): 0x10 kevlar, 0x20 stealth shoes.
    pub items: u32,
    /// Grenades bought this life (`pl+0x714`).
    pub grenades_bought: u8,
    /// Accumulated disarm damage (`pl+0x7b4`, decays 5/s).
    pub disarm: f32,
    /// `BlockPlayer` timer (`pl+0x4dc`): clears `iuser4 & 1` when it runs out.
    pub block_timer: f32,
    /// Previous weapon for `lastinv` (`pl+0xa1c`).
    pub last_inv: u8,
    /// usercmd random seed (`pl+0x7ec`, set in CmdStart, rerolled after each weapon frame).
    pub seed: u32,
    pub last_buttons: u32,
    pub pressed: u32,
    /// Original owner of each record (`s_weapon_status+0x30`, "killed with own weapon").
    pub owners: [*mut edict_t; MAX_WEAPONS],
    /// Player animation extension of the current weapon (for the movement package).
    pub anim_ext: String,
    /// Last victim of the delayed melee hit (`pl+0x798`).
    pub lunge_victim: *mut edict_t,
    /// The attacker's kung-fu block window at the start of the weapon frame (for blocks).
    pub atk_block: f32,
    // ---- network caches (`CTSGun::UpdateClientData` 0x858c8) ----
    // They belong to the gun entity in TS: a new gun starts with `CTSGun::Reset`'s values
    // ([`new_gun_state`]), and a gun that is still held keeps them through a spawn.
    /// `+0x9e0`: the 10 Hz throttle (0 in a new gun; `CTSGun::Deploy` with a weapon zeroes it).
    pub next_update: f32,
    /// `+0x97a`: the weapon id of the last `WeaponInfo` (`Reset`: 0). `None` only after the
    /// port-only `ts_give` ([`SvState::force_resend`]).
    pub sent_id: Option<u8>,
    /// `+0x97c`: the record the last `WeaponInfo` was built from (`Reset`: zero).
    pub sent_status: Status,
    /// `+0x9b4`: the reserve the last `WeaponInfo` sent (`Reset`: 0).
    pub sent_reserve: i32,
    /// `+0x9b8`: the `owned` flag the last `WStatus` sent, per weapon (`Reset`: zero).
    pub sent_owned: [bool; MAX_WEAPONS],
    /// Port bookkeeping: the free-slot count `pev->maxspeed` was last computed for
    /// (`inventory::sync_speed`). `TSInit`'s `SetPlayerSpeed(330)` is the value for 81.
    pub speed_for: Option<i32>,
    /// The `weapon_tsgun` entity that is the gun: TS's `CTSGun` is an entity, from
    /// `GiveNamedItem("weapon_tsgun")` (`PlayerSpawn`, a `game_player_equip`) or placed by the
    /// map, kept attached to the player (`AttachToPlayer` 0xf1d80: hidden, following it) until
    /// `RemoveAllItems` drops it ([`drop_player_items`]). The gun's state stays in [`Gun`].
    pub gun_item: EHandle,
    /// A `cycler_weapon` in the player's HL item slot 1 (`m_rgpPlayerItems[1]`, from
    /// `CBasePlayerItem::DefaultTouch` 0xf1b40), attached like `gun_item`.
    pub cycler: EHandle,
    /// The cycler's `m_iszModel` and `m_iModel` (fixed at its `Spawn`): `Deploy` shows the
    /// model, and `ItemPostFrame`'s gate reads `m_iModel` through the active item.
    pub cycler_model: goldsrc::sys::string_t,
    pub cycler_model_index: i32,
    /// The cycler is `m_pActiveItem` (`SwitchWeapon` or `SelectItem` deployed it); a held gun is
    /// holstered meanwhile.
    pub cycler_active: bool,
    /// `m_pLastItem` (pl+0xa18): the item `SelectItem` switched away from. Only `SelectLastItem`
    /// reads it, and nothing calls that; it is saved with the player.
    pub last_item: Active,
}

impl Default for SvState {
    fn default() -> Self {
        SvState {
            has_gun: false,
            armament: Vec::new(),
            armament_ammo: [0; MAX_AMMO],
            free_slots: SLOTS,
            items: 0,
            grenades_bought: 0,
            disarm: 0.0,
            block_timer: 0.0,
            last_inv: 0,
            seed: 0,
            last_buttons: 0,
            pressed: 0,
            owners: [std::ptr::null_mut(); MAX_WEAPONS],
            anim_ext: String::new(),
            lunge_victim: std::ptr::null_mut(),
            atk_block: 0.0,
            next_update: 0.0,
            sent_id: Some(0),
            sent_status: Status::default(),
            sent_reserve: 0,
            sent_owned: [false; MAX_WEAPONS],
            speed_for: None,
            gun_item: EHandle::NULL,
            cycler: EHandle::NULL,
            cycler_model: 0,
            cycler_model_index: 0,
            cycler_active: false,
            last_item: Active::None,
        }
    }
}

impl SvState {
    /// Port addition for the `ts_give` dev command: every weapon message is resent on the next
    /// update. TS has no such reset; a spawn keeps the caches of a held gun.
    pub fn force_resend(&mut self) {
        self.next_update = 0.0;
        self.sent_id = None;
        self.sent_reserve = -1;
        self.sent_owned = [false; MAX_WEAPONS];
    }

    /// The `UpdateClientData` caches `CTSGun::Reset` (0x805e0) zeroes (+0x97a..+0x9de): last
    /// `WeaponInfo` id 0 (kung fu, so a gun left on kung fu sends no `WeaponInfo`), its record
    /// and reserve, and the `WStatus` owned flags. The 10 Hz timer (+0x9e0) is not among them.
    fn reset_send_caches(&mut self) {
        self.sent_id = Some(0);
        self.sent_status = Status::default();
        self.sent_reserve = 0;
        self.sent_owned = [false; MAX_WEAPONS];
    }
}

/// The player's weapon state: the shared `CTSGun` plus server bookkeeping.
#[derive(Default)]
pub struct Inventory {
    pub gun: Gun,
    pub sv: SvState,
}

impl Inventory {
    /// Current TS weapon id (0 = kung fu).
    pub fn current(&self) -> u8 {
        self.gun.current
    }
    /// Player animation extension of the current weapon (`onehanded`, `twohanded`, `akimbo`).
    pub fn anim_extension(&self) -> &str {
        &self.sv.anim_ext
    }
}

pub fn game_init() {
    assets::register_cvars();
}

/// `CTSGun::Precache`'s events, from `W_Precache` (`crate::precache`).
pub fn precache_events() {
    assets::precache_events();
}

/// `TSPrecache`'s weapon loop (`crate::precache`).
pub fn precache_weapon_assets() {
    assets::precache_weapon_assets();
}

/// `g_sModelIndexBubbles`, from `W_Precache` (`crate::precache`).
pub fn set_bubbles_model(index: i32) {
    assets::set_bubbles_model(index);
}

/// The weapon part of `StartFrame`: `CTSGameRules::Think` runs the simulated bullets
/// (`CBulletManager::Think`).
pub fn start_frame() {
    // `CBulletManager::Think` runs from `CTSGameRules::Think`, which single player never runs.
    if !crate::rules::state::single_player() {
        bullets::start_frame();
    }
}

/// The weapon fields of `CBasePlayer::TSInit` (0x92a4c), which runs first in
/// `CBasePlayer::Spawn` (0x8f568, its only caller): 81 free slots (`+0x548 = 0x51`), no loadout
/// items (`+0x718 = 0`) and `SetPlayerSpeed(330)` (the caller, `rules::lifecycle::on_spawn`,
/// writes the speed). Nothing else resets them: `RemoveAllItems`, `StartObserver` and `Killed`
/// leave them to the item and weapon code (pickups, drops, armour, the CTB case). A gun held
/// through the spawn keeps its weapons, whose slots are free again, as in TS. `TSSpace` is
/// resent by `UpdateClientData` (`TSInit`'s `+0x54c = -1`, `player::hud::reset`).
pub fn ts_init(p: &mut Player) {
    let sv = &mut p.inv.sv;
    sv.free_slots = SLOTS;
    sv.items = 0;
    sv.speed_for = Some(SLOTS);
}

pub fn on_spawn(p: &mut Player, cx: &mut Cx) {
    // TSInit: kung-fu power 100, recoil penalty 0, fresh timers. The records were emptied where TS
    // loses its gun (`remove_all_items`); a spawn without that keeps them, as TS keeps the gun it
    // already holds (`AddPlayerItem` kills the new duplicate), and with it the gun's
    // `UpdateClientData` caches, so nothing is resent. A new gun gets `CTSGun::Reset`'s caches
    // (`add_gun`). The slots and loadout items were reset by `ts_init`.
    p.inv.gun.reset_for_spawn();
    p.inv.gun.mode_memory = [0; MAX_WEAPONS];
    let sv = &mut p.inv.sv;
    // CBasePlayer::Spawn (0x8f568): `m_pLastItem = NULL`.
    sv.last_item = Active::None;
    sv.grenades_bought = 0;
    sv.disarm = 0.0;
    sv.block_timer = 0.0;
    sv.pressed = 0;
    sv.last_buttons = 0;
    cx.pev().fov = 0.0;
    // CHalfLifeMultiplay::PlayerSpawn (0x890ac): every game_player_equip touches the player;
    // only without one does it get `weapon_tsgun` (`GiveNamedItem`, an entity) and buy its
    // loadout. A gunless player stays without weapons (kung fu included) unless an equip lists
    // `weapon_tsgun`. The player is lent to its slot meanwhile, so the touches reach it inside
    // the spawn as in TS. `CHalfLifeRules::PlayerSpawn` (0xde7d8) is empty: single player spawns
    // with nothing.
    if !crate::rules::state::single_player() {
        let me = cx.edict;
        if !crate::world::lend(me, p, || crate::entities::stock::maprules::equip_on_spawn(me)) {
            give_named_gun(p, cx);
            inventory::buy_armament(p, cx);
        }
    }
    inventory::sync_speed(p, cx);
    net::send_kfu_power(cx.edict, p.inv.gun.melee_bar);
}

pub fn pre_think(p: &mut Player, cx: &mut Cx) {
    let pev = cx.pev();
    let buttons = pev.button as u32;
    p.inv.sv.pressed = buttons & !p.inv.sv.last_buttons;
    let dt = Gun::timer_dt(engine().globals().frametime, pev.fuser1.max(0.0), p.active_powerup);
    let alive = Player::is_alive(cx);
    p.inv.gun.pre_frame(dt, alive);
    p.inv.sv.disarm = (p.inv.sv.disarm - dt * 5.0).max(0.0);
    // BlockPlayer expiry (PreThink: clears iuser4 bit 1 when the timer runs out).
    let sv = &mut p.inv.sv;
    if sv.block_timer != 0.0 {
        sv.block_timer = (sv.block_timer - dt).max(0.001);
        if sv.block_timer < 0.01 {
            let pev = cx.pev();
            pev.iuser4 &= !1;
            sv.block_timer = 0.0;
        }
    }
    // The delayed melee hit lands from the stunt hook (`CheckForBreakables`), which calls
    // `pending_melee_hit`.
}

/// The `Use` of the weapons package's usable entities when a player's +use picks them
/// (`player::player_use`, run while the player is being dispatched, so these take the player
/// directly): dropped guns (`CTSWorldGun::Use` 0xa1158), thrown blades (`CTSKnife::Use`
/// 0x7dda4) and grenades (`CM61Grenade::Use` 0x7ce64). False if `ent` is none of them.
pub fn use_by_player(ent: &mut dyn std::any::Any, icx: &mut Cx, p: &mut Player, cx: &mut Cx) -> bool {
    if let Some(w) = ent.downcast_mut::<worldgun::WorldGun>() {
        w.use_by(icx, p, cx);
    } else if let Some(k) = ent.downcast_mut::<projectiles::Knife>() {
        k.use_by(icx, p, cx);
    } else if let Some(g) = ent.downcast_mut::<projectiles::Grenade>() {
        g.use_by(icx, cx);
    } else {
        return false;
    }
    true
}

/// The weapon parts of `CBasePlayer::PostThink` (0x91610). `active` is false during intermission
/// and while dead: PostThink then skips `ItemPostFrame` and the `m_afButtonLast` update and only
/// runs the timers down.
pub fn post_think(p: &mut Player, cx: &mut Cx, active: bool) {
    // `CBasePlayer::ItemPostFrame` (0xdc4d0) does nothing while a tank is in use. Otherwise, once
    // the gun is ready, `ImpulseCommands` (0xdb780: `PlayerUse` 0xd9848 and the impulses) and then
    // the weapon frame.
    if active {
        if !p.drive.tank.is_valid() && item_post_frame_ready(p, cx.pev().button as u32) {
            let (buttons, last) = (cx.pev().button, p.inv.sv.last_buttons as i32);
            crate::player::player_use::impulse_commands(p, cx, buttons & !last, last & !buttons);
            // `m_pActiveItem->ItemPostFrame()` and the reseed, for whichever item is active once
            // the impulses ran: `CBasePlayerWeapon::ItemPostFrame` (0xa03d4) returns at once for
            // a cycler (`m_iId` 0), but the seed is rerolled all the same.
            match active_item(p) {
                Active::None => {}
                Active::Gun => {
                    host::with_host(p, cx, |gun, h| gun.weapon_post_frame(h));
                    p.inv.sv.seed = engine().random_long(0, 32000) as u32;
                }
                Active::Cycler => p.inv.sv.seed = engine().random_long(0, 32000) as u32,
            }
        }
        p.inv.sv.last_buttons = cx.pev().button as u32;
    }
    let pev = cx.pev();
    let dt = Gun::timer_dt(engine().globals().frametime, pev.fuser1.max(0.0), p.active_powerup);
    p.inv.gun.decrement_timers(dt);
    inventory::sync_speed(p, cx);
}

/// `m_pActiveItem->UpdateClientData` (`CTSGun::UpdateClientData` 0x858c8), run at the end of
/// `CBasePlayer::UpdateClientData` (`player::hud::update_client_data`).
pub fn update_client_data(p: &mut Player, cx: &mut Cx) {
    net::update_client(p, cx);
}

/// `CFuncTank::StartControl` (0xc2a24): `m_pActiveItem->Holster()` (`CTSGun::Holster` 0x85674,
/// back to kung fu, or `CWeaponCycler::Holster`), when there is an active item.
pub fn holster_for_tank(p: &mut Player, cx: &mut Cx) {
    holster_active(p, cx);
}

/// The item part at the top of `CBasePlayer::StartObserver` (0x9971e..0x9974f): when
/// `m_pActiveItem` is the TS gun (`m_iId == 16`), `CTSGun::Reset` (0x805e0, [`gun_reset`]) and
/// `CTSGun::HolsterWeapon` (0x85698); then, for any active item, its `Holster` (`CTSGun::Holster`
/// 0x85674, which is `HolsterWeapon` again, or `CWeaponCycler::Holster` 0xc7dd4). The reset comes
/// first, so neither holster finds a record: a primed grenade is not thrown and a reload is not
/// cancelled, the weapon is simply gone. Each gun holster of a living player sends the kung-fu
/// holster animation and the draw event; `RemoveAllItems` holsters a third time later, after
/// `deadflag` is set, so that one sends neither.
pub fn observer_holster(p: &mut Player, cx: &mut Cx) {
    if matches!(active_item(p), Active::Gun) {
        gun_reset(&mut p.inv);
        host::with_host(p, cx, |gun, h| gun.holster_weapon(h));
    }
    holster_active(p, cx);
}

/// `CTSGun::Reset` (0x805e0): the records, reserve ammo, current weapon (with its cached
/// record and info pointers, +0xc8..+0xd0) and the `UpdateClientData` caches (+0x97a..+0x9de: last
/// `WeaponInfo` id 0, its status and reserve, the `WStatus` owned flags) are zeroed. Timers, the
/// kung-fu bar and the gun entity's `pev` are left alone.
fn gun_reset(inv: &mut Inventory) {
    let body = inv.gun.body;
    inv.gun.reset_inventory();
    inv.gun.body = body;
    inv.sv.reset_send_caches();
}

/// The state of a `weapon_tsgun` that has just joined the player ([`add_gun`]): a new entity
/// (zeroed private data, then `CTSGun::Spawn` 0x8053c → `Reset`), so an empty inventory, no
/// owners, `Reset`'s send caches and the `UpdateClientData` timer at 0.
fn new_gun_state(inv: &mut Inventory) {
    inv.gun.reset_inventory();
    inv.sv.owners = [std::ptr::null_mut(); MAX_WEAPONS];
    inv.sv.reset_send_caches();
    inv.sv.next_update = 0.0;
}

/// `CFuncTank::StopControl` (0xc2b1c): `m_pActiveItem->Deploy()` (`CTSGun::Deploy` 0x81030, or
/// `CWeaponCycler::Deploy`), when there is an active item.
pub fn deploy_after_tank(p: &mut Player, cx: &mut Cx) {
    deploy_active(p, cx);
}

/// The weapon part of `CBasePlayer::Killed` (0x8ed5c): holster, the ammo pack, the CTB case,
/// then every weapon. The gun itself goes on the first death think ([`remove_all_items`]).
pub fn on_killed(p: &mut Player, cx: &mut Cx) {
    inventory::on_killed(p, cx);
}

/// `m_pActiveItem` is the TS gun (`m_iId == 16`): the player holds `weapon_tsgun` (see
/// [`SvState::has_gun`]) and hasn't switched to a `cycler_weapon`. Without it TS skips the weapon
/// frame (attacks, kung fu, reload), every weapon command (`weapon_<n>`, `firemode`,
/// `useitems`, `lastinv`, `drop`, so the CTB case can't be dropped either), pickups
/// (`PickUpWeapon` 0x9314c: dropped guns, `ts_groundweapon`, thrown knives, the grenade
/// power-up), ammo packs and ground ammo, the infinite-clip power-up, weapon and ammo buys and
/// attachments, the `ActItems` and `CTSGun::UpdateClientData` messages, and the weapon fields
/// of `UpdateClientData` / `GetWeaponData`. +use, stunts, kevlar and the other power-ups still
/// work. (The paths that only test `m_pActiveItem` for NULL before using it as the gun would
/// read a deployed cycler as a `CTSGun`, past the end of its object; the port treats a cycler
/// as no gun there too.)
pub fn gun_active(p: &Player) -> bool {
    active_item(p) == Active::Gun
}

/// `HasPlayerItem` for `weapon_tsgun` (by classname in slot list 2): the gun is held, active or
/// holstered for a `cycler_weapon`.
pub fn holds_gun(p: &Player) -> bool {
    p.inv.sv.has_gun
}

/// What `m_pActiveItem` (pl+0xa10) is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Active {
    #[default]
    None,
    Gun,
    Cycler,
}

/// The player's `m_pActiveItem`.
pub fn active_item(p: &Player) -> Active {
    if p.inv.sv.cycler_active {
        Active::Cycler
    } else if p.inv.sv.has_gun {
        Active::Gun
    } else {
        Active::None
    }
}

/// An item `AddPlayerItem` adds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Gun,
    Cycler,
}

impl Item {
    /// `ItemInfoArray[m_iId].iWeight`: the gun's is 1 (`CTSGun::GetItemInfo` 0x85dd0, id 16); the
    /// cycler's `GetItemInfo` (0xc8840) registers nothing, so its id 0 weighs 0.
    fn weight(self) -> i32 {
        match self {
            Item::Gun => 1,
            Item::Cycler => 0,
        }
    }
}

/// `g_pGameRules->FShouldSwitchWeapon(player, new)` at the end of `AddPlayerItem` (0xdc0c4).
/// `CHalfLifeMultiplay` (0xcf380): `CanDeploy` (always), then with no active item yes, else
/// `CanHolster` (always) and the active item weighing less. `CHalfLifeRules` (0xde74c): with no
/// active item or one that `CanHolster`, i.e. always.
pub fn should_switch_weapon(single_player: bool, active: Active, new: Item) -> bool {
    let active = match active {
        Active::None => return true,
        Active::Gun => Item::Gun,
        Active::Cycler => Item::Cycler,
    };
    single_player || active.weight() < new.weight()
}

/// `CBasePlayer::ItemPostFrame`'s gate (0xdc4d0), the tank aside: `m_flNextAttack` ran out, no
/// active item, or the active item's `+0xcc` is NULL (the gun's current record: kung fu,
/// [`Gun::post_frame_ready`], with its +cancel clause). For a cycler `+0xcc` is `m_iModel`; the
/// +cancel clause would then read through that model index as a pointer, which TS can't survive
/// (an out-of-bounds read): the port leaves that clause out for a cycler.
fn item_post_frame_ready(p: &Player, buttons: u32) -> bool {
    match active_item(p) {
        Active::None => true,
        Active::Gun => p.inv.gun.post_frame_ready(buttons),
        Active::Cycler => cycler_post_frame_ready(p.inv.gun.next_attack, p.inv.sv.cycler_model_index),
    }
}

fn cycler_post_frame_ready(next_attack: f32, model_index: i32) -> bool {
    next_attack <= 0.0 || model_index == 0
}

/// `InformClientsOnWeaponsParams(player)` (0x9eb14): the custom-weapons notice and every
/// weapon's parameters (`CustomWP`), from `CTSGameRules::InitHUD` while the custom-weapons flag
/// is set.
pub fn inform_custom_weapons(e: *mut edict_t) {
    custom::inform(Some(e));
}

/// `CBasePlayer::HasWeapons` (0xdb060): any item in the player's slot lists, the gun or a
/// `cycler_weapon`.
pub fn has_weapons(p: &Player) -> bool {
    p.inv.sv.has_gun || p.inv.sv.cycler.is_valid()
}

/// `CBulletManager::Reset` (0x6ec50), the part of `CTSGameRules::Init` the weapons own.
pub fn reset_bullets() {
    bullets::reset();
}

/// `CBasePlayerWeapon::AddToPlayer` (0xf1e60) sets `pev->weapons |= 1 << m_iId`; the TS gun's
/// id is 16 (`CTSGun::GetItemInfo` 0x85dd0). Only `RemoveAllItems` clears it again (TS never
/// calls `DropPlayerItem`, the other place that clears a weapon bit, and nothing reads it: the
/// TS client ignores `iWeaponBits`).
pub const TSGUN_WEAPON_BIT: i32 = 1 << shared::protocol::weapons::TSGUN_ID;

/// `CBasePlayer::AddPlayerItem` (0xdc0c4) for a `weapon_tsgun` entity (`item`), after
/// `CanHavePlayerItem` (the player holds none): the new `CTSGun` (`Spawn` → `Reset`: an empty
/// inventory, kung fu) joins slot list 2 and `CBasePlayerWeapon::AddToPlayer` sets weapon bit
/// 16. [`switch_to_gun`] is the `FShouldSwitchWeapon` / `SwitchWeapon` part that follows
/// `CheckRespawn`.
pub fn add_gun(p: &mut Player, cx: &mut Cx, item: *mut edict_t) {
    p.inv.sv.has_gun = true;
    new_gun_state(&mut p.inv);
    p.inv.sv.gun_item = EHandle::new(item);
    cx.pev().weapons |= TSGUN_WEAPON_BIT;
}

/// The end of `AddPlayerItem` for the gun: [`should_switch_weapon`] always says yes (no active
/// item, or a cycler, which weighs less; single player switches anyway), so `SwitchWeapon`
/// (0xdd98c) holsters an active cycler (`CWeaponCycler::Holster`) and deploys the gun
/// (`CTSGun::Deploy`: kung fu, unless the port-only bots already got a weapon).
pub fn switch_to_gun(p: &mut Player, cx: &mut Cx) {
    if !p.inv.sv.has_gun || !should_switch_weapon(crate::rules::state::single_player(), active_item(p), Item::Gun) {
        return;
    }
    if p.inv.sv.cycler_active {
        cycler_holster(p);
        p.inv.sv.cycler_active = false;
    }
    host::with_host(p, cx, |gun, h| {
        let cur = gun.current;
        gun.current = 0;
        if cur != 0 && gun.status[cur as usize].owned {
            gun.current = cur;
            gun.deploy(h);
        } else {
            gun.holster_weapon(h);
        }
    });
    inventory::sync_speed(p, cx);
}

/// `CBasePlayer::GiveNamedItem("weapon_tsgun")` (0xdb524) for the player being dispatched
/// (`PlayerSpawn`): the player is lent to its slot (`world::lend`) while the new `weapon_tsgun`
/// spawns at its feet and touches it, so the touch reaches it as in TS
/// (`maprules::give_named_item`, `playeritem::WorldTsGun`).
pub fn give_named_gun(p: &mut Player, cx: &mut Cx) {
    let me = cx.edict;
    crate::world::lend(me, p, || crate::entities::stock::maprules::give_named_item(me, c"weapon_tsgun"));
}

/// `CBasePlayer::RemoveAllItems(TRUE)` (0xd8cec), from `PlayerDeathThink` (0x8eaec) while the
/// player still holds the gun (`HasWeapons`), `StartObserver` (0x99986) and
/// `ClientDisconnected` (0xcf960): the active item is holstered and every item destroyed, the
/// gun with everything in it (the inventory and reserve ammo), then [`remove_all_items_tail`]
/// (`UpdateClientData`, then `CurWeapon 0,0,0`). The free slots, loadout items and move speed
/// are left alone: only `TSInit` ([`ts_init`]) and the item code change them.
pub fn remove_all_items(p: &mut Player, cx: &mut Cx) {
    holster_active(p, cx);
    drop_player_items(p);
    clear_held_items(&mut p.inv);
    remove_all_items_tail(p, cx);
}

/// What destroying the player's items takes with it: the gun (its records and reserve, kept in
/// the port's [`Inventory`]) and the records' owners. Nothing of the player's own state.
fn clear_held_items(inv: &mut Inventory) {
    inv.sv.has_gun = false;
    inv.gun.reset_inventory();
    inv.sv.owners = [std::ptr::null_mut(); MAX_WEAPONS];
}

/// `CBasePlayer::RemoveAllItems(FALSE)` (0xd8cec), from `player_weaponstrip`
/// (`CStripWeapons::Use` 0xddbb0): [`remove_all_items`] except that `pev->weapons` keeps the HEV
/// suit bit (`&= 0x80000000`, set before `UpdateClientData` and `CurWeapon 0,0,0`).
pub fn remove_all_items_keep_suit(p: &mut Player, cx: &mut Cx) {
    holster_active(p, cx);
    drop_player_items(p);
    clear_held_items(&mut p.inv);
    let v = cx.pev();
    v.viewmodel = 0;
    v.weaponmodel = 0;
    v.weapons &= i32::MIN;
    crate::player::hud::update_client_data(p, cx);
    net::send_cur_weapon_reset(cx.edict);
}

pub fn client_command(p: &mut Player, cx: &mut Cx, cmd: &str, args: &Args) -> bool {
    commands::client_command(p, cx, cmd, args)
}

pub fn fill_client_data(p: &mut Player, cx: &mut Cx, cd: &mut clientdata_t) {
    net::fill_client_data(p, cx, cd);
}

/// `pfnGetWeaponData`: fill up to 32 `weapon_data_t`. Return 1.
pub fn get_weapon_data(p: &mut Player, cx: &mut Cx, info: &mut [weapon_data_t; 32]) -> i32 {
    *info = [weapon_data_t::default(); 32];
    net::get_weapon_data(p, cx, info);
    1
}

/// `pfnCmdStart`: remember the shared random seed for this usercmd.
pub fn cmd_start(p: &mut Player, _cx: &mut Cx, _cmd: &usercmd_t, seed: u32) {
    p.inv.sv.seed = seed;
}

// ---- API for other subsystems (rules: loadouts, ground weapons, power-ups) -------------------

/// Gives weapon `id` with attachment bits. Returns false if it doesn't fit.
/// (`CBasePlayer::PickUpWeapon(id, NULL, attachments, select = id != 29, own = false)`, what a
/// `ts_groundweapon` does with its whole spawnflags.)
pub fn give_weapon(p: &mut Player, cx: &mut Cx, id: u8, attachments: u32) -> bool {
    if !gun_active(p) {
        return false; // `PickUpWeapon` needs the gun
    }
    let ok = inventory::pick_up_weapon(p, cx, id, None, attachments, id != shared::weapons::id::BRIEFCASE, false);
    inventory::sync_speed(p, cx);
    ok
}

/// Gives `rounds` of ammo type `ammo`. Returns rounds actually added (`CTSGun::AddAmmo`,
/// clamped to the `ammocount` max carry); none without the gun.
pub fn give_ammo(p: &mut Player, _cx: &mut Cx, ammo: u8, rounds: i32) -> i32 {
    if !gun_active(p) || ammo as usize >= MAX_AMMO {
        return 0;
    }
    let before = p.inv.gun.reserve[ammo as usize];
    p.inv.gun.add_ammo(ammo, rounds, assets::max_carry(ammo));
    p.inv.gun.reserve[ammo as usize] - before
}

/// Selects weapon `id` like the `weapon_<id>` client command (0 = kung fu).
pub fn select(p: &mut Player, cx: &mut Cx, id: u8) {
    if !gun_active(p) {
        return;
    }
    p.inv.sv.last_inv = p.inv.gun.current;
    host::with_host(p, cx, |gun, h| gun.switch_to(id, h));
}

/// The tail of `CBasePlayer::RemoveAllItems(TRUE)` (0xd8cec), once the items are gone:
/// no view or weapon model, `pev->weapons = 0` (the HEV suit bit too, so the geiger counter
/// stops), `UpdateClientData` inline (0xd8eb5, throttled as usual), and `CurWeapon 0,0,0` to the
/// player. Callers: `StartObserver` (0x99986), `ClientDisconnected` (0xcf960) and
/// `PlayerDeathThink` (0x8eaec, [`remove_all_items`]).
pub fn remove_all_items_tail(p: &mut Player, cx: &mut Cx) {
    let v = cx.pev();
    v.viewmodel = 0;
    v.weaponmodel = 0;
    v.weapons = 0;
    crate::player::hud::update_client_data(p, cx);
    net::send_cur_weapon_reset(cx.edict);
}

/// `m_pActiveItem->Holster()` (`RemoveAllItems` 0xd8cec, `Killed`, `StartObserver`,
/// `StartControl`, `SwitchWeapon`, `SelectItem`): the gun's (`CTSGun::Holster` 0x85674 →
/// `HolsterWeapon`, kung fu) or a deployed cycler's (`CWeaponCycler::Holster` 0xc7dd4). The item
/// stays `m_pActiveItem`: the switches replace it, `RemoveAllItems` clears it.
pub fn holster_active(p: &mut Player, cx: &mut Cx) {
    match active_item(p) {
        Active::None => {}
        Active::Gun => host::with_host(p, cx, |gun, h| gun.holster_weapon(h)),
        Active::Cycler => cycler_holster(p),
    }
}

/// `m_pActiveItem->Deploy()` (`StopControl`): `CTSGun::Deploy` (0x81030) or
/// `CWeaponCycler::Deploy` (0xc7d6c).
fn deploy_active(p: &mut Player, cx: &mut Cx) {
    match active_item(p) {
        Active::None => {}
        Active::Gun => host::with_host(p, cx, |gun, h| gun.deploy(h)),
        Active::Cycler => cycler_deploy(p, cx),
    }
}

/// The item loop of `RemoveAllItems` (0xd8cec): every item entity the player holds is
/// `Drop`ped (`CBasePlayerItem::Drop` 0xf1cac, removed 0.1 s later). Slot list 1 (the cycler)
/// comes before slot list 2 (the gun). `m_pActiveItem` is cleared.
fn drop_player_items(p: &mut Player) {
    let sv = &mut p.inv.sv;
    sv.cycler_active = false;
    sv.last_item = Active::None;
    for h in [std::mem::take(&mut sv.cycler), std::mem::take(&mut sv.gun_item)] {
        crate::entities::stock::playeritem::drop_item(h.get());
    }
}

/// `CWeaponCycler::Holster` (0xc7dd4): `m_flNextAttack = UTIL_WeaponTimeBase() + 0.5` (the time
/// base is 0).
fn cycler_holster(p: &mut Player) {
    p.inv.gun.next_attack = 0.5;
}

/// `CBasePlayer::AddPlayerItem` (0xdc0c4) for a `cycler_weapon` (`item`, its `m_iszModel` and
/// `m_iModel`), after `CanHavePlayerItem`: `CBasePlayerWeapon::AddToPlayer` (0xf1e60) sets
/// weapon bit 0 (the cycler's `m_iId` is never set) and the item goes into slot list 1.
/// [`deploy_cycler`] is the `FShouldSwitchWeapon` / `SwitchWeapon` part that follows
/// `CheckRespawn`.
pub fn add_cycler(p: &mut Player, cx: &mut Cx, item: *mut edict_t, model: goldsrc::sys::string_t, model_index: i32) {
    cx.pev().weapons |= 1;
    let sv = &mut p.inv.sv;
    sv.cycler = EHandle::new(item);
    sv.cycler_model = model;
    sv.cycler_model_index = model_index;
}

/// The end of `AddPlayerItem` for a `cycler_weapon`: [`should_switch_weapon`] (under the
/// multiplayer rules only with no active item, the gun's weight 1 beating the cycler's 0; in
/// single player always), then `SwitchWeapon` (0xdd98c): the active gun is holstered
/// (`CTSGun::Holster`, kung fu) and the cycler deployed ([`cycler_deploy`]).
pub fn deploy_cycler(p: &mut Player, cx: &mut Cx) {
    if !p.inv.sv.cycler.is_valid() || !should_switch_weapon(crate::rules::state::single_player(), active_item(p), Item::Cycler) {
        return;
    }
    holster_active(p, cx);
    p.inv.sv.cycler_active = true;
    cycler_deploy(p, cx);
}

/// `CWeaponCycler::Deploy` (0xc7d6c): the cycler's `m_iszModel` as the view model,
/// `m_flNextAttack = 1` (time base 0) and `SendWeaponAnim(0, skiplocal)`, which sends the
/// cycler's own `pev->body`.
fn cycler_deploy(p: &mut Player, cx: &mut Cx) {
    cx.pev().viewmodel = p.inv.sv.cycler_model;
    p.inv.gun.next_attack = 1.0;
    // CBasePlayerWeapon::SendWeaponAnim (0xa00d0), as `host`'s weapon_anim sends it.
    cx.pev().weaponanim = 0;
    if cx.pev().flags & goldsrc::sys::FL_FAKECLIENT as i32 != 0 {
        return;
    }
    let item = p.inv.sv.cycler.get();
    let body = if item.is_null() { 0 } else { goldsrc::edict::EdictPtr::v(item).body };
    use goldsrc::ext::weapons::WeaponsEngine;
    let eng = engine();
    if !eng.can_skip_player(cx.edict) {
        eng.send_weapon_anim(cx.edict, 0, body);
    }
}

/// `CBasePlayer::SelectItem(name)` (0xdade4, the `use <classname>` command): the first held item
/// of that classname (slot list 1, the cycler, before slot list 2, the gun: `strcmp`), unless it
/// is already `m_pActiveItem`, becomes the active item: the old one's `Holster`, then the new
/// one's `Deploy` (and `UpdateItemInfo`, empty). No `CanHolster`/`CanDeploy` or alive test.
/// (`m_pLastItem`, which it also sets, is read only by `SelectLastItem`, which nothing calls.)
pub fn select_item(p: &mut Player, cx: &mut Cx, name: &std::ffi::CStr) {
    let cycler = p.inv.sv.cycler.get();
    let cycler_class = (!cycler.is_null()).then(|| engine().string(goldsrc::edict::EdictPtr::v(cycler).classname));
    let Some(item) = select_item_target(name, cycler_class, p.inv.sv.has_gun) else { return };
    if matches!((item, active_item(p)), (Item::Gun, Active::Gun) | (Item::Cycler, Active::Cycler)) {
        return;
    }
    p.inv.sv.last_item = active_item(p);
    holster_active(p, cx);
    match item {
        Item::Cycler => {
            p.inv.sv.cycler_active = true;
            cycler_deploy(p, cx);
        }
        Item::Gun => {
            p.inv.sv.cycler_active = false;
            host::with_host(p, cx, |gun, h| gun.deploy(h));
        }
    }
    inventory::sync_speed(p, cx);
}

/// The held item `SelectItem` finds for `name`: the cycler (slot list 1, by its classname)
/// first, then the gun (slot list 2, `weapon_tsgun`).
fn select_item_target(name: &std::ffi::CStr, cycler_class: Option<&std::ffi::CStr>, holds_gun: bool) -> Option<Item> {
    if cycler_class == Some(name) {
        Some(Item::Cycler)
    } else if holds_gun && name == c"weapon_tsgun" {
        Some(Item::Gun)
    } else {
        None
    }
}

/// `CHalfLifeMultiplay::CanHavePlayerItem` (0xcfd38) / `CGameRules::CanHavePlayerItem`
/// (0xc5f48) for a `cycler_weapon`: a living player (`deadflag`) without one already (by id
/// with `weaponstay` on, otherwise `HasPlayerItem` by classname: the same test, since the gun's
/// id and classname differ from the cycler's).
pub fn can_have_cycler(p: &Player, cx: &mut Cx) -> bool {
    cx.pev().deadflag == goldsrc::sys::DEAD_NO as i32 && !p.inv.sv.cycler.is_valid()
}

/// Infinite Clip power-up: double the current gun's clip. False if no gun.
pub fn multiply_clip(p: &mut Player, cx: &mut Cx) -> bool {
    if !gun_active(p) {
        return false; // `PlayerAddPowerUp` 0x97bc8: `if (!m_pActiveItem) return 0`
    }
    let free = p.inv.sv.free_slots;
    match p.inv.gun.multiply_clips(free) {
        Some(used) => {
            p.inv.sv.free_slots -= used;
            inventory::sync_speed(p, cx);
            true
        }
        None => false,
    }
}

/// The close-combat window part of `CTSStunt::CheckForBreakables`: lands the delayed hit armed
/// by a melee attack or the backflip kick (called by the stunt hook while the window is open).
pub fn pending_melee_hit(p: &mut Player, cx: &mut Cx) {
    melee::pending_hit(p, cx);
}

/// The backflip kick (`CTSStunt::GoWheel`): `UpdateMeleeBar(cost)`, then a 0.3 s kung-fu hit
/// window with `damage`.
pub fn arm_kick(p: &mut Player, cx: &mut Cx, cost: f32, damage: f32) {
    host::with_host(p, cx, |gun, h| gun.update_melee_bar(cost, h));
    let g = &mut p.inv.gun;
    g.melee_window = 0.3;
    g.melee_damage = damage;
    g.melee_source = 0;
    g.block_kungfu = 0.0;
}

/// `GetSpeedBySlots` + `SetPlayerSpeed` after something outside the weapons changed
/// `free_slots` (the CTB case, `CBasePlayer::TakeFlag` 0x93830 / `DropFlag` 0x9334c).
pub fn sync_speed(p: &mut Player, cx: &mut Cx) {
    inventory::sync_speed(p, cx);
}

/// `CBasePlayer::TakeArmor(n)` (0x93da4): kevlar weighs [`inventory::KEVLAR_SLOTS`] of the
/// shared free slots; `n < 0` takes it off. False when there's no room for the vest.
pub fn take_armor(p: &mut Player, cx: &mut Cx, n: i32) -> bool {
    inventory::take_armor(p, cx, n)
}

/// Drops a weapon like the `drop` command / a disarm (rules: flag/briefcase handling stays theirs).
pub fn drop_weapon(p: &mut Player, cx: &mut Cx, which: Drop) {
    inventory::drop_weapon(p, cx, which);
}

/// `AddToFullPack` glow (TS: `WorldGun` dropped by `host`, or a `knife` it threw): returns
/// true if `ent` should get `rendermode 0, renderamt 2, renderfx kRenderFxGlowShell (19),
/// rendercolor (100,100,100)` for this client.
pub fn glow_for_host(ent: *mut edict_t, host: *mut edict_t) -> bool {
    worldgun::glows_for(ent, host) || projectiles::knife_glows_for(ent, host)
}

/// Round-mode loadout items: M61 buys (2 per life) and stealth shoes. TS allows them only when
/// `CTSGameRules::m_mode > 1`, which 3.0 DM/TDM never reach, so they default to off.
pub fn set_round_mode_items(allowed: bool) {
    assets::set_grenade_buys(allowed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gun_weapon_bit_is_id_16() {
        assert_eq!(TSGUN_WEAPON_BIT, 0x1_0000);
        // RemoveAllItems(FALSE) keeps only the suit bit; (TRUE) clears everything.
        let weapons = i32::MIN | TSGUN_WEAPON_BIT | 1;
        assert_eq!(weapons & i32::MIN, i32::MIN);
    }

    #[test]
    fn switch_weapon_rules() {
        // Nothing active: both rules deploy the new item.
        for sp in [false, true] {
            assert!(should_switch_weapon(sp, Active::None, Item::Gun));
            assert!(should_switch_weapon(sp, Active::None, Item::Cycler));
        }
        // Multiplayer weights: the gun (1) replaces a cycler (0), never the other way round.
        assert!(should_switch_weapon(false, Active::Cycler, Item::Gun));
        assert!(!should_switch_weapon(false, Active::Gun, Item::Cycler));
        // Single player switches whenever the active item can holster (always).
        assert!(should_switch_weapon(true, Active::Gun, Item::Cycler));
        assert!(should_switch_weapon(true, Active::Cycler, Item::Gun));
    }

    #[test]
    fn active_item_follows_the_held_items() {
        let mut p = Player::default();
        assert_eq!(active_item(&p), Active::None);
        p.inv.sv.has_gun = true;
        assert_eq!(active_item(&p), Active::Gun);
        assert!(gun_active(&p) && holds_gun(&p));
        // Holstered for a cycler: still held, no longer the active item.
        p.inv.sv.cycler_active = true;
        assert_eq!(active_item(&p), Active::Cycler);
        assert!(!gun_active(&p) && holds_gun(&p));
        p.inv.sv.has_gun = false;
        assert_eq!(active_item(&p), Active::Cycler);
    }

    #[test]
    fn tsgun_reset_empties_the_gun_and_its_send_caches() {
        use shared::weapons::id;
        let mut inv = Inventory::default();
        assert_eq!(inv.gun.take_weapon(id::AK47, None, 0), Some(id::AK47));
        inv.gun.current = id::AK47;
        inv.gun.reserve[3] = 90;
        inv.gun.body = 1;
        inv.gun.melee_bar = 42.0;
        inv.sv.sent_id = Some(id::AK47);
        inv.sv.sent_reserve = 90;
        inv.sv.sent_owned[id::AK47 as usize] = true;
        gun_reset(&mut inv);
        // CTSGun::Reset (0x805e0): records, reserve, current (so no record for the holsters that
        // follow it in StartObserver), and +0x97a..+0x9de zeroed.
        assert!(inv.gun.status.iter().all(|s| !s.owned));
        assert_eq!(inv.gun.reserve, [0; MAX_AMMO]);
        assert_eq!(inv.gun.current, 0);
        assert!(inv.gun.slot().is_none());
        assert_eq!(inv.sv.sent_id, Some(0));
        assert_eq!(inv.sv.sent_reserve, 0);
        assert_eq!(inv.sv.sent_owned, [false; MAX_WEAPONS]);
        // The gun entity's pev (body) and the player-side fields stay.
        assert_eq!(inv.gun.body, 1);
        assert_eq!(inv.gun.melee_bar, 42.0);
    }

    #[test]
    fn ts_init_frees_the_slots_and_drops_the_loadout_items() {
        use shared::weapons::{att, id};
        let mut p = Player::default();
        p.inv.sv.has_gun = true;
        assert_eq!(p.inv.gun.take_weapon(id::AK47, None, 0), Some(id::AK47));
        p.inv.sv.free_slots = 30;
        p.inv.sv.items = att::KEVLAR | att::STEALTH_SHOES;
        p.inv.sv.speed_for = Some(30);
        p.inv.sv.sent_id = Some(id::AK47);
        p.inv.sv.sent_owned[id::AK47 as usize] = true;
        p.inv.sv.next_update = 7.5;
        ts_init(&mut p);
        // TSInit (0x92a4c): +0x548 = 0x51, +0x718 = 0, SetPlayerSpeed(330) (the value for 81).
        assert_eq!((p.inv.sv.free_slots, p.inv.sv.items, p.inv.sv.speed_for), (SLOTS, 0, Some(SLOTS)));
        // A gun still held keeps its weapons and its UpdateClientData caches.
        assert!(p.inv.gun.status[id::AK47 as usize].owned);
        assert_eq!(p.inv.sv.sent_id, Some(id::AK47));
        assert!(p.inv.sv.sent_owned[id::AK47 as usize]);
        assert_eq!(p.inv.sv.next_update, 7.5);
    }

    #[test]
    fn destroying_the_items_leaves_slots_items_and_speed() {
        use shared::weapons::{att, id};
        let mut inv = Inventory::default();
        inv.sv.has_gun = true;
        assert_eq!(inv.gun.take_weapon(id::AK47, None, 0), Some(id::AK47));
        inv.sv.free_slots = 40;
        inv.sv.items = att::KEVLAR;
        inv.sv.speed_for = Some(40);
        clear_held_items(&mut inv);
        // RemoveAllItems (0xd8cec) / StartObserver (0x99634) never write +0x548, +0x718 or the
        // speed: the gun and its records go, the rest stays until TSInit.
        assert!(!inv.sv.has_gun);
        assert!(inv.gun.status.iter().all(|s| !s.owned));
        assert_eq!((inv.sv.free_slots, inv.sv.items, inv.sv.speed_for), (40, att::KEVLAR, Some(40)));
    }

    #[test]
    fn a_new_gun_starts_with_reset_send_caches() {
        use shared::weapons::id;
        let mut inv = Inventory::default();
        // What an earlier gun left behind (the port keeps the caches with the player).
        inv.sv.sent_id = Some(id::AK47);
        inv.sv.sent_reserve = 90;
        inv.sv.sent_status.clip = 30;
        inv.sv.sent_owned[id::AK47 as usize] = true;
        inv.sv.next_update = 12.0;
        inv.sv.owners[1] = std::ptr::dangling_mut();
        inv.gun.reserve[3] = 5;
        new_gun_state(&mut inv);
        // A fresh entity: CTSGun::Spawn -> Reset (0x805e0), the +0x9e0 timer zeroed memory.
        assert_eq!(inv.sv.sent_id, Some(0));
        assert_eq!(inv.sv.sent_reserve, 0);
        assert_eq!(inv.sv.sent_status, Status::default());
        assert_eq!(inv.sv.sent_owned, [false; MAX_WEAPONS]);
        assert_eq!(inv.sv.next_update, 0.0);
        assert!(inv.sv.owners.iter().all(|o| o.is_null()));
        assert_eq!(inv.gun.reserve, [0; shared::weapons::MAX_AMMO]);
        // Kung fu (id 0) is what +0x97a says was sent: no WeaponInfo, as in TS.
        assert_eq!(inv.gun.current, 0);
        let fresh = SvState::default();
        assert_eq!((fresh.sent_id, fresh.sent_reserve), (inv.sv.sent_id, inv.sv.sent_reserve));
    }

    #[test]
    fn cycler_item_post_frame_gate() {
        // `m_flNextAttack` run out, or the cycler's `m_iModel` (read as the gun's record) zero.
        assert!(cycler_post_frame_ready(0.0, 5));
        assert!(cycler_post_frame_ready(-0.1, 5));
        assert!(!cycler_post_frame_ready(0.5, 5));
        assert!(cycler_post_frame_ready(0.5, 0));
    }

    #[test]
    fn select_item_by_classname() {
        let c = Some(c"cycler_weapon");
        assert_eq!(select_item_target(c"cycler_weapon", c, true), Some(Item::Cycler));
        assert_eq!(select_item_target(c"weapon_tsgun", c, true), Some(Item::Gun));
        assert_eq!(select_item_target(c"weapon_tsgun", c, false), None);
        assert_eq!(select_item_target(c"cycler_weapon", None, true), None);
        // `strcmp`: exact and case-sensitive.
        assert_eq!(select_item_target(c"WEAPON_TSGUN", c, true), None);
        assert_eq!(select_item_target(c"", c, true), None);
    }
}
