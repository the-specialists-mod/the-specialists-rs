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

use crate::commands::Args;
use crate::player::Player;
use crate::world::Cx;
use goldsrc::edict::EdictPtr;
use goldsrc::server::engine;
use goldsrc::sys::{clientdata_t, edict_t, usercmd_t, weapon_data_t};
use shared::weapons::{Gun, MAX_AMMO, MAX_WEAPONS, Status};

#[allow(unused_imports)]
pub use assets::{cvars, dmgreport, restriction, set_restriction};
pub use inventory::Drop;
pub use projectiles::{SlowPhysics, bounce_ground_damping, give_slow_mul, thrower_time_scale};

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
    // ---- network caches (CTSGun::UpdateClientData) ----
    pub next_update: f32,
    pub sent_id: Option<u8>,
    pub sent_status: Status,
    pub sent_reserve: i32,
    pub sent_owned: [bool; MAX_WEAPONS],
    pub sent_act_items: Option<u8>,
    pub sent_space: Option<i32>,
    /// Free-slot count the move speed was last computed for.
    pub speed_for: Option<i32>,
    /// `CustomWP` sent to this client.
    pub custom_informed: bool,
}

impl Default for SvState {
    fn default() -> Self {
        SvState {
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
            sent_id: None,
            sent_status: Status::default(),
            sent_reserve: -1,
            sent_owned: [false; MAX_WEAPONS],
            sent_act_items: None,
            sent_space: None,
            speed_for: None,
            custom_informed: false,
        }
    }
}

impl SvState {
    /// Forces every weapon message to be resent.
    pub fn reset_net(&mut self) {
        self.next_update = 0.0;
        self.sent_id = None;
        self.sent_reserve = -1;
        self.sent_owned = [false; MAX_WEAPONS];
        self.sent_act_items = None;
        self.sent_space = None;
        self.speed_for = None;
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

pub fn precache() {
    assets::precache();
}

/// The weapon part of `StartFrame`: `CTSGameRules::Think` runs the simulated bullets
/// (`CBulletManager::Think`).
pub fn start_frame() {
    bullets::start_frame();
}

pub fn on_spawn(p: &mut Player, cx: &mut Cx) {
    // TSInit: kung-fu power 100, recoil penalty 0, fresh timers. The records were emptied where TS
    // loses its gun (`on_killed`, `strip_all`); a spawn without that keeps them, as TS keeps the
    // gun it already holds (`AddPlayerItem` kills the new duplicate).
    p.inv.gun.reset_for_spawn();
    inventory::reset_items(p);
    p.inv.gun.mode_memory = [0; MAX_WEAPONS];
    let sv = &mut p.inv.sv;
    sv.grenades_bought = 0;
    sv.disarm = 0.0;
    sv.block_timer = 0.0;
    sv.pressed = 0;
    sv.last_buttons = 0;
    sv.reset_net();
    cx.pev().fov = 0.0;
    // CHalfLifeMultiplay::PlayerSpawn: the gun deploys (kung fu), then BuyArmament unless the map
    // has a game_player_equip.
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
    // A map's game_player_equip replaces the loadout (rules/stock-entity hook).
    if !crate::entities::stock::maprules::equip_on_spawn(cx.edict) {
        inventory::buy_armament(p, cx);
    }
    inventory::sync_speed(p, cx);
    net::send_kfu_power(cx.edict, p.inv.gun.melee_bar);
    if !p.inv.sv.custom_informed && cx.edict.v().flags & goldsrc::sys::FL_FAKECLIENT as i32 == 0 {
        p.inv.sv.custom_informed = true;
        custom::inform(Some(cx.edict));
    }
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
    // the gun is ready, `ImpulseCommands` runs `PlayerUse` (0xd9848) and then the weapon frame.
    if active {
        if !p.drive.tank.is_valid() && p.inv.gun.post_frame_ready(cx.pev().button as u32) {
            let (buttons, last) = (cx.pev().button, p.inv.sv.last_buttons as i32);
            crate::player::player_use::player_use(p, cx, buttons & !last, last & !buttons);
            host::with_host(p, cx, |gun, h| gun.weapon_post_frame(h));
            p.inv.sv.seed = engine().random_long(0, 32000) as u32;
        }
        p.inv.sv.last_buttons = cx.pev().button as u32;
    }
    let pev = cx.pev();
    let dt = Gun::timer_dt(engine().globals().frametime, pev.fuser1.max(0.0), p.active_powerup);
    p.inv.gun.decrement_timers(dt);
    inventory::sync_speed(p, cx);
    net::update_client(p, cx);
}

/// `CFuncTank::StartControl` (0xc2a24): `m_pActiveItem->Holster()` (`CTSGun::Holster` 0x85674,
/// back to kung fu).
pub fn holster_for_tank(p: &mut Player, cx: &mut Cx) {
    host::with_host(p, cx, |gun, h| gun.holster_weapon(h));
}

/// `CFuncTank::StopControl` (0xc2b1c): `m_pActiveItem->Deploy()` (`CTSGun::Deploy` 0x81030).
pub fn deploy_after_tank(p: &mut Player, cx: &mut Cx) {
    host::with_host(p, cx, |gun, h| gun.deploy(h));
}

pub fn on_killed(p: &mut Player, cx: &mut Cx) {
    inventory::on_killed(p, cx);
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
/// `ts_groundweapon` does.)
pub fn give_weapon(p: &mut Player, cx: &mut Cx, id: u8, attachments: u8) -> bool {
    let ok = inventory::pick_up_weapon(p, cx, id, None, attachments as u32, id != shared::weapons::id::BRIEFCASE, false);
    inventory::sync_speed(p, cx);
    ok
}

/// Gives `rounds` of ammo type `ammo`. Returns rounds actually added (`CTSGun::AddAmmo`,
/// clamped to the `ammocount` max carry).
pub fn give_ammo(p: &mut Player, _cx: &mut Cx, ammo: u8, rounds: i32) -> i32 {
    if ammo as usize >= MAX_AMMO {
        return 0;
    }
    let before = p.inv.gun.reserve[ammo as usize];
    p.inv.gun.add_ammo(ammo, rounds, assets::max_carry(ammo));
    p.inv.gun.reserve[ammo as usize] - before
}

/// Selects weapon `id` like the `weapon_<id>` client command (0 = kung fu).
pub fn select(p: &mut Player, cx: &mut Cx, id: u8) {
    p.inv.sv.last_inv = p.inv.gun.current;
    host::with_host(p, cx, |gun, h| gun.switch_to(id, h));
}

/// Removes every weapon (death, spectate).
pub fn strip_all(p: &mut Player, cx: &mut Cx) {
    host::with_host(p, cx, |gun, h| gun.holster_weapon(h));
    p.inv.gun.reset_inventory();
    p.inv.sv.free_slots = SLOTS;
    p.inv.sv.items = 0;
    p.inv.sv.owners = [std::ptr::null_mut(); MAX_WEAPONS];
    inventory::sync_speed(p, cx);
}

/// The tail of `CBasePlayer::RemoveAllItems(TRUE)` (0xd8cec) that [`strip_all`] leaves out:
/// no view or weapon model, `pev->weapons = 0` (the HEV suit bit too, so the geiger counter
/// stops), and `CurWeapon 0,0,0` to the player. `StartObserver` (0x99986) is TS's only
/// `RemoveAllItems(TRUE)` caller.
pub fn remove_all_items_tail(cx: &mut Cx) {
    let v = cx.pev();
    v.viewmodel = 0;
    v.weaponmodel = 0;
    v.weapons = 0;
    net::send_cur_weapon_reset(cx.edict);
}

/// Infinite Clip power-up: double the current gun's clip. False if no gun.
pub fn multiply_clip(p: &mut Player, cx: &mut Cx) -> bool {
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
