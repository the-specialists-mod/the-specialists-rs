//! The TS gun (`CTSGun`, weapons.md §4): one object per player holding all 38 weapon records,
//! the current weapon, the relative weapon timers and the per-player recoil penalty.
//!
//! Every method mirrors the `CTSGun` method of the same name (addresses in the docs). Side
//! effects that differ between server and client (events, traces, damage, projectiles, view
//! model, sounds) go through [`GunHost`]; the state transitions themselves are shared so the
//! client prediction and the server agree.
//!
//! Timers are relative (`UseDecrement() == 1`, `UTIL_WeaponTimeBase() == 0`) and are lowered by
//! [`Gun::decrement_timers`] with `frametime * fuser1 * (double fire rate ? 1.5 : 1)`.

use super::spread::{self, Vec3};
use super::*;
use crate::ts::powerup;
use std::ffi::CStr;

/// The eleven TS weapon events, in `CTSGun::Precache` order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Event {
    Fire,
    ColdCock,
    FireLinked,
    Empty,
    Pump,
    CloseCombat,
    Reload,
    Knife,
    Katana,
    Draw,
    Grenade,
}

impl Event {
    pub const ALL: [Event; 11] = [
        Event::Fire,
        Event::ColdCock,
        Event::FireLinked,
        Event::Empty,
        Event::Pump,
        Event::CloseCombat,
        Event::Reload,
        Event::Knife,
        Event::Katana,
        Event::Draw,
        Event::Grenade,
    ];

    /// `events/<name>.sc`, as passed to `pfnPrecacheEvent(1, ...)`.
    pub fn path(self) -> &'static CStr {
        match self {
            Event::Fire => c"events/fire.sc",
            Event::ColdCock => c"events/coldcock.sc",
            Event::FireLinked => c"events/firelinked.sc",
            Event::Empty => c"events/empty.sc",
            Event::Pump => c"events/pump.sc",
            Event::CloseCombat => c"events/closecombat.sc",
            Event::Reload => c"events/reload.sc",
            Event::Knife => c"events/knife.sc",
            Event::Katana => c"events/katana.sc",
            Event::Draw => c"events/draw.sc",
            Event::Grenade => c"events/grenade.sc",
        }
    }
}

/// `PLAYBACK_EVENT_FULL` parameters (origin and angles are always zero, flags `FEV_NOTHOST`).
/// `b1`/`b2` are ints because TS passes the random seed as `bparam2` (the event delta only
/// carries its low bit).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EventArgs {
    pub f1: f32,
    pub f2: f32,
    pub i1: i32,
    pub i2: i32,
    pub b1: i32,
    pub b2: i32,
}

/// One `ShootBulletsPlayer` call. The host traces `pellets` rays from the gun position (offset by
/// `right_offset` along `v_right` for linked akimbo shots) along the seeded directions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shot {
    pub weapon: u8,
    pub pellets: u32,
    pub spread: f32,
    pub damage: f32,
    /// `s_weapon_info+0x20`: 0 normal, 1 shotgun, 2 pierce.
    pub btype: i32,
    pub ammo: u8,
    pub seed: u32,
    pub right_offset: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeleeKind {
    KungFu,
    ColdCock,
    Knife,
    Katana,
}

/// A melee trace + hit (`CloseCombatAttack`, `ColdCockAttack`, the knife and katana slashes).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeleeHit {
    pub kind: MeleeKind,
    /// Weapon id credited for the hit (0 = kung fu).
    pub weapon: u8,
    /// Units from the gun position along `v_forward` (negative = behind, the running back kick).
    pub reach: f32,
    /// Hull for the fallback trace (`CheckHull`): 1 human, 3 head.
    pub hull: i32,
    pub damage: f32,
    pub bits: i32,
    /// `CTSAnimation` sequence for the attacker, -1 for none.
    pub player_anim: i32,
    pub disarm: bool,
    pub knockback: bool,
    /// Use `v_angle` with pitch 0 (running back kick).
    pub level: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MeleeResult {
    Miss,
    /// The victim blocked (`Victim*Hold`). The attacker gets these values.
    Blocked { next_attack: f32, bar_cost: f32 },
    /// Something alive was hit; `player` if it was a player.
    Hit { player: bool },
}

/// Sounds the server emits itself (the client plays everything else from events).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sound {
    Switch,
    KevlarHit,
    KnifeHitBody,
    KnifeMiss,
    KatanaHitBody,
    KatanaMiss,
}

/// Everything the gun needs from the player and the engine.
pub trait GunHost {
    fn time(&self) -> f32;
    /// `GiveSlowMul()`: the player's time scale (`pev->fuser1`).
    fn slow_mul(&self) -> f32;
    /// Active power-up bits (`pl+0x724`, `shared::ts::powerup`).
    fn powerups(&self) -> u16;
    /// Permanent kung-fu power-up (x2 melee damage).
    fn kungfu(&self) -> bool;
    fn alive(&self) -> bool;
    fn buttons(&self) -> u32;
    /// `m_afButtonPressed`.
    fn pressed(&self) -> u32;
    fn origin(&self) -> Vec3;
    fn view_ofs(&self) -> Vec3;
    fn velocity(&self) -> Vec3;
    fn v_angle(&self) -> Vec3;
    fn flags(&self) -> i32;
    fn waterlevel(&self) -> i32;
    fn iuser4(&self) -> i32;
    fn set_iuser4(&mut self, v: i32);
    /// The usercmd random seed (`pl+0x7ec`).
    fn seed(&self) -> u32;
    fn random_long(&mut self, lo: i32, hi: i32) -> i32;

    fn fov(&self) -> f32;
    fn set_fov(&mut self, fov: f32);
    /// `pev->viewmodel`: `Some(id)` = that weapon's v_ model, `None` = hidden.
    fn set_view_model(&mut self, id: Option<u8>);
    fn view_model_hidden(&self) -> bool;
    /// `pev->weaponmodel`: `Some((id, silenced))` or `None`.
    fn set_weapon_model(&mut self, model: Option<(u8, bool)>);
    /// Third-person animation extension (`DefaultDeploy` `szAnimExt`).
    fn set_anim_extension(&mut self, ext: &str) {
        let _ = ext;
    }

    fn event(&mut self, ev: Event, args: EventArgs);
    /// `SendWeaponAnim(anim, skiplocal, body)`.
    fn weapon_anim(&mut self, anim: i32, skip_local: bool, body: i32);
    /// `CTSAnimation::Animate(n)` (third-person sequence; owned by the movement package).
    fn player_anim(&mut self, anim: i32);
    /// `VKickback` result. TS writes it to `v_angle` only for bots; the client applies it to the
    /// local view.
    fn kick(&mut self, pitch: f32, yaw: f32);
    fn sound(&mut self, s: Sound);

    fn shoot(&mut self, shot: &Shot);
    fn melee(&mut self, hit: &MeleeHit) -> MeleeResult;
    /// Throws the M61 (`CM61Grenade::CreateGrenade`, fuse 3 s of the thrower's time).
    fn throw_grenade(&mut self, origin: Vec3, velocity: Vec3, roll: bool);
    /// Throws a knife/seal knife/katana (`CTSKnife::CreateKnife`, `btype` 6/7/8).
    fn throw_blade(&mut self, btype: i32);
    /// Inventory weight freed (positive) or used (negative); the server updates speed.
    fn slots_changed(&mut self, delta_free: i32);
    /// `UpdateMeleeBar` sends `KFuPower`.
    fn melee_bar_changed(&mut self, bar: f32);
    /// `CBasePlayer::BlockPlayer(t)`: half speed (`iuser4 |= 1`) for `t` seconds.
    fn block_player(&mut self, t: f32);
}

/// `s_weapon_status` (0x38 bytes per weapon id).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Status {
    pub owned: bool,
    pub ammo: u8,
    pub mode: u8,
    /// Clip (stackables: count).
    pub clip: i32,
    /// +8: burst shots left / shotgun reload phase / grenade phase / melee hit flag.
    pub state: i32,
    /// +0xc
    pub in_reload: i32,
    /// +0x10 owned attachments.
    pub items: u32,
    /// +0x14 attachments switched on.
    pub active: u32,
    /// +0x18 body group.
    pub body: i32,
    /// +0x1c / +0x20 akimbo left / right clip.
    pub left: i32,
    pub right: i32,
    /// +0x24 akimbo gun toggle / pump pending / grenade roll / knife throw.
    pub toggle: bool,
    /// +0x2c alternate grip (sequence table B).
    pub alt_grip: bool,
}

/// The TS gun of one player.
#[derive(Clone, Debug, PartialEq)]
pub struct Gun {
    pub status: [Status; MAX_WEAPONS],
    /// Reserve ammo per ammo type (`gun+0xd4`).
    pub reserve: [i32; MAX_AMMO],
    /// Current weapon id, 0 = kung fu (`gun+0xc8`).
    pub current: u8,
    pub next_primary: f32,
    pub next_secondary: f32,
    /// `gun+0xa4`: next alternate (stock) attack.
    pub next_alt: f32,
    pub idle: f32,
    /// `m_flNextAttack` (`pl+0x12c`).
    pub next_attack: f32,
    /// `m_iClip` mirror (`gun+0xb4`).
    pub clip_mirror: i32,
    /// The gun entity's `pev->fuser1`, a timer TS decrements and reports in `weapon_data_t`.
    pub fuser1: f32,
    /// `gun+0x9e8`: time of the last shot (absolute).
    pub last_shot: f32,
    /// `pl+0x778`: recoil spread penalty (max 0.1).
    pub spread_penalty: f32,
    /// `gun+0x9fc` / `+0xa00` / `+0xa04`: punch combo, kick combo, last melee view anim.
    pub punch_combo: i32,
    pub kick_combo: i32,
    pub last_melee_anim: i32,
    /// `pl+0x774`: kung-fu power (0..100).
    pub melee_bar: f32,
    /// `pl+0x79c` / `+0x7a0` / `+0x7a4`: delayed melee hit window, its damage and source
    /// (0 kung fu, 1 knife, 2 katana).
    pub melee_window: f32,
    pub melee_damage: f32,
    pub melee_source: u8,
    /// `pl+0x7a8` / `+0x7ac` / `+0x7b0`: block windows (kung fu, knife, katana).
    pub block_kungfu: f32,
    pub block_knife: f32,
    pub block_katana: f32,
    /// `pl+0x49a + id`: remembered fire mode + 1.
    pub mode_memory: [u8; MAX_WEAPONS],
    /// Last spread offset returned by `ShootBulletsPlayer` (stale, sent with `coldcock.sc`).
    pub last_dir: Vec3,
}

impl Default for Gun {
    fn default() -> Self {
        Gun {
            status: [Status::default(); MAX_WEAPONS],
            reserve: [0; MAX_AMMO],
            current: 0,
            next_primary: 0.0,
            next_secondary: 0.0,
            next_alt: 0.0,
            idle: 0.0,
            next_attack: 0.0,
            clip_mirror: 1,
            fuser1: 0.0,
            last_shot: 0.0,
            spread_penalty: 0.0,
            punch_combo: 0,
            kick_combo: 0,
            last_melee_anim: 0,
            melee_bar: 100.0,
            melee_window: 0.0,
            melee_damage: 0.0,
            melee_source: 0,
            block_kungfu: 0.0,
            block_knife: 0.0,
            block_katana: 0.0,
            mode_memory: [0; MAX_WEAPONS],
            last_dir: [0.0; 3],
        }
    }
}

const IN_ATTACK: u32 = 1;
const IN_DUCK: u32 = 4;
const IN_BACK: u32 = 16;
const IN_CANCEL: u32 = 0x40; // TS +alt2: stock attack
const IN_ATTACK2: u32 = 0x800;
const IN_RELOAD: u32 = 0x2000;
const FL_ONGROUND: i32 = 0x200;
const FL_DUCKING: i32 = 0x4000;
/// `iuser4` bits that count as "stunting" for melee (`0x70c30`) and kung fu (`0x50c30`).
const STUNTING_MELEE: i32 = 0x70c30;
const STUNTING_KUNGFU: i32 = 0x50c30;

fn wi(id: u8) -> &'static WeaponInfo {
    // Entry 0 for invalid ids, like get_weapon_info.
    info(id).or_else(|| info(0)).expect("weapon table entry 0")
}

impl Gun {
    // ---- helpers ------------------------------------------------------------------------------

    /// The active weapon record (`gun+0xcc`), `None` for kung fu.
    pub fn slot(&self) -> Option<&Status> {
        (self.current != 0).then(|| &self.status[self.current as usize])
    }
    fn has_slot(&self) -> bool {
        self.current != 0
    }
    fn cur(&mut self) -> &mut Status {
        &mut self.status[self.current as usize]
    }
    fn info(&self) -> &'static WeaponInfo {
        wi(self.current)
    }
    fn kind(&self) -> u8 {
        kind_of(self.current)
    }
    pub fn double_fire_rate(host: &impl GunHost) -> bool {
        host.powerups() & powerup::DOUBLE_FIRERATE != 0
    }
    pub fn infinite_clip(host: &impl GunHost) -> bool {
        host.powerups() & powerup::INFINITE_CLIP != 0
    }
    /// "silenced" = silencer owned and not built in.
    fn silenced(&self) -> bool {
        self.has_slot() && self.status[self.current as usize].items & att::SILENCER != 0 && self.info().att_default & 1 == 0
    }
    /// Current sequence table entry `i` (table B when the alternate grip is on).
    fn seq(&self, i: usize) -> i32 {
        let alt = self.slot().map(|s| s.alt_grip).unwrap_or(false);
        anim(self.current, alt, i)
    }
    fn speed(host: &impl GunHost) -> f32 {
        spread::length(host.velocity())
    }

    /// Reserve ammo of the current weapon's ammo type.
    pub fn current_reserve(&self) -> i32 {
        self.reserve[(self.info().ammo as usize).min(MAX_AMMO - 1)]
    }

    /// `CTSGun::GetActiveItems`: attachments on, plus 0x40 for akimbo weapons.
    pub fn active_items(&self) -> u32 {
        match self.slot() {
            Some(s) => s.active | if is_akimbo(self.current) || is_only_akimbo(self.current) { att::AKIMBO } else { 0 },
            None => 0,
        }
    }

    /// Lowers every weapon timer by `dt` (`CBasePlayer::PostThink` 0x91610 tail).
    pub fn decrement_timers(&mut self, dt: f32) {
        self.next_primary = (self.next_primary - dt).max(-1.0);
        self.next_secondary = (self.next_secondary - dt).max(-0.001);
        self.next_alt = (self.next_alt - dt).max(-0.001);
        if self.idle != 1000.0 {
            self.idle = (self.idle - dt).max(-0.001);
        }
        if self.fuser1 != 1000.0 {
            self.fuser1 = (self.fuser1 - dt).max(-0.001);
        }
        self.next_attack -= dt;
        if self.next_attack < -0.001 {
            self.next_attack = -0.001;
        }
    }

    /// Weapon timer step: `frametime * slowmul * (double fire rate ? 1.5 : 1)`.
    pub fn timer_dt(frametime: f32, slowmul: f32, powerups: u16) -> f32 {
        let dt = slowmul * frametime;
        if powerups & powerup::DOUBLE_FIRERATE != 0 { dt * 1.5 } else { dt }
    }

    /// The weapon parts of `CBasePlayer::PreThink` 0x908cc: melee windows, recoil penalty decay
    /// (0.1/s) and kung-fu power regeneration (15/s). `dt` as for [`Gun::timer_dt`].
    pub fn pre_frame(&mut self, dt: f32, alive: bool) {
        if self.melee_window != 0.0 {
            self.melee_window = (self.melee_window - dt).max(0.001);
        }
        self.block_kungfu = (self.block_kungfu - dt).max(0.0);
        self.block_katana = (self.block_katana - dt).max(0.0);
        self.block_knife = (self.block_knife - dt).max(0.0);
        if alive {
            if self.spread_penalty > 0.0 {
                self.spread_penalty -= dt * 0.1;
                if self.spread_penalty < 0.0 {
                    self.spread_penalty = 0.0;
                }
            }
            if self.melee_bar < 100.0 {
                let step = if self.melee_bar >= -0.001 { dt * 15.0 } else { dt };
                self.melee_bar = (self.melee_bar + step).min(100.0);
            }
        }
        if self.melee_window != 0.0 {
            if self.melee_window < 0.01 || self.melee_damage == 0.0 {
                self.melee_window = 0.0;
                self.melee_damage = 0.0;
                self.melee_source = 0;
            }
            self.block_kungfu = 0.0;
        }
    }

    /// `CBasePlayer::UpdateMeleeBar(cost)` 0x9650c.
    pub fn update_melee_bar(&mut self, cost: f32, host: &mut impl GunHost) {
        let v = self.melee_bar - cost;
        if v < 0.01 {
            self.melee_bar = 0.0;
            self.next_attack = 1.5;
        } else {
            self.melee_bar = v;
        }
        host.melee_bar_changed(self.melee_bar);
    }

    // ---- frame entry --------------------------------------------------------------------------

    /// `CBasePlayer::ItemPostFrame` 0xdc4d0 + `CBasePlayerWeapon::ItemPostFrame` 0xa03d4.
    /// Returns true if the weapon frame ran (the server then rerolls the seed).
    pub fn item_post_frame(&mut self, host: &mut impl GunHost) -> bool {
        let buttons = host.buttons();
        let run = self.next_attack <= 0.0
            || !self.has_slot()
            || (buttons & IN_CANCEL != 0 && self.status[self.current as usize].in_reload == 1 && self.info().stock != 0);
        if !run {
            return false;
        }
        self.weapon_post_frame(host);
        true
    }

    fn weapon_post_frame(&mut self, host: &mut impl GunHost) {
        let buttons = host.buttons();
        if !self.has_slot() {
            if buttons & IN_ATTACK2 != 0 && self.next_secondary <= 0.0 {
                self.secondary_attack(host);
            } else if buttons & IN_ATTACK != 0 && self.next_primary <= 0.0 {
                self.primary_attack(host);
            } else if buttons & IN_RELOAD != 0 {
                self.reload(host);
            }
            self.weapon_idle(host);
            return;
        }
        // Finish a magazine reload.
        if self.status[self.current as usize].in_reload != 0 && self.next_attack <= 0.0 {
            let info = self.info();
            let ammo = (info.ammo as usize).min(MAX_AMMO - 1);
            let s = self.status[self.current as usize];
            let j = (info.clip - s.clip).min(self.reserve[ammo]);
            let s = self.cur();
            s.clip += j;
            s.left = s.clip / 2;
            s.right = s.clip - s.left;
            let (clip, left) = (s.clip, s.left);
            if kind_of(self.current) == kind::AKIMBO {
                if info.single >= 0 {
                    self.status[info.single as usize].clip = left;
                }
            } else if info.akimbo != -1 {
                let tw = &mut self.status[info.akimbo as usize];
                tw.clip += j;
                tw.left = clip;
            }
            self.clip_mirror = clip;
            self.reserve[ammo] -= j;
            self.cur().in_reload = 0;
        }
        let contender_empty = self.current == id::CONTENDER && self.status[self.current as usize].clip == 0;
        if buttons & IN_ATTACK2 != 0 && self.next_secondary <= 0.0 {
            self.secondary_attack(host);
        } else if buttons & IN_ATTACK != 0 && self.next_primary <= 0.0 && !contender_empty {
            self.primary_attack(host);
        } else if buttons & IN_CANCEL != 0 && self.next_alt <= 0.0 {
            self.alternate_attack(host);
        } else if buttons & IN_RELOAD != 0
            && self.has_slot()
            && self.status[self.current as usize].in_reload == 0
            && self.current != id::CONTENDER
        {
            self.reload(host);
        }
        self.weapon_idle(host);
    }

    // ---- attacks ------------------------------------------------------------------------------

    /// `CTSGun::PrimaryAttack` 0x81a20.
    pub fn primary_attack(&mut self, host: &mut impl GunHost) {
        if !self.has_slot() {
            self.ccombat_attack(host);
            return;
        }
        match self.kind() {
            kind::GRENADE => {
                self.cur().toggle = false;
                self.grenade_fire(host);
                return;
            }
            kind::KNIFE => {
                self.cur().toggle = false;
                self.knife_fire(host);
                return;
            }
            kind::KATANA => {
                self.katana_fire(host);
                return;
            }
            kind::FLAG => {
                if host.pressed() & IN_ATTACK != 0 {
                    self.cur().toggle = false;
                    self.knife_fire(host);
                }
                return;
            }
            _ => {}
        }
        let m = self.status[self.current as usize].mode;
        let fire = match m {
            mode::SEMI | mode::AKIMBO_SEMI => host.pressed() & IN_ATTACK != 0 || Self::double_fire_rate(host),
            mode::AUTO | mode::AKIMBO_AUTO => true,
            mode::BURST => {
                let uump = self.current == id::UMP;
                let s = self.cur();
                if host.pressed() & IN_ATTACK != 0 && s.state == 0 {
                    s.state = if uump { 2 } else { 3 };
                }
                false
            }
            mode::PUMP => host.buttons() & IN_ATTACK != 0,
            _ => false,
        };
        if fire {
            self.fire(host);
        }
    }

    /// `CTSGun::SecondaryAttack` 0x817a8.
    pub fn secondary_attack(&mut self, host: &mut impl GunHost) {
        if !self.has_slot() {
            self.ccombat_attack(host);
            return;
        }
        match self.kind() {
            kind::GRENADE => {
                self.cur().toggle = true;
                self.grenade_fire(host);
                return;
            }
            kind::KNIFE => {
                self.cur().toggle = true;
                self.knife_fire(host);
                return;
            }
            kind::KATANA => {
                self.katana_throw(host);
                return;
            }
            kind::FLAG => return,
            _ => {}
        }
        let m = self.status[self.current as usize].mode;
        if m == mode::AKIMBO_SEMI {
            if host.pressed() & IN_ATTACK2 == 0 && !Self::double_fire_rate(host) {
                return;
            }
            self.fire(host);
            return;
        }
        if m == mode::AKIMBO_AUTO {
            self.fire(host);
            return;
        }
        let info = self.info();
        let s = self.status[self.current as usize];
        if s.items & att::SCOPE == 0 {
            if has_alt_grip(self.current) {
                // anim [7] of the table in use before the switch
                let a = self.seq(7);
                let body = s.body;
                self.cur().alt_grip = !s.alt_grip;
                host.weapon_anim(a, true, body);
            }
        } else if host.fov() == 0.0 {
            if info.zoom[0] != 0 {
                self.zoom(info.zoom[0], host);
            }
        } else if info.zoom[1] == 0 || info.zoom[1] as f32 == host.fov() {
            self.unzoom(host);
        } else {
            self.zoom(info.zoom[1], host);
        }
        self.next_secondary = 0.3;
        self.idle = 1.0;
    }

    /// `CTSGun::AlternateAttack` 0x81ba8: the stock hit (+alt2).
    pub fn alternate_attack(&mut self, host: &mut impl GunHost) {
        if self.has_slot() && self.info().stock > 0 {
            self.cold_cock(host);
        }
    }

    /// `CTSGun::EmptySound` 0x81be4 (silent for the Contender).
    pub fn empty_sound(&mut self, host: &mut impl GunHost) {
        if self.current != id::CONTENDER {
            host.event(Event::Empty, EventArgs::default());
            self.next_secondary = 0.1;
            self.next_primary = 0.1;
            self.idle = 0.5;
        }
    }

    /// `CTSGun::Fire` 0x81c78.
    pub fn fire(&mut self, host: &mut impl GunHost) {
        if !self.has_slot() {
            return;
        }
        if host.waterlevel() == 3 {
            self.empty_sound(host);
            return;
        }
        let s = self.status[self.current as usize];
        let k = self.kind();
        if !Self::infinite_clip(host) && s.clip < 1 && (!s.toggle || s.mode != mode::PUMP || k != kind::SHOTGUN) {
            self.empty_sound(host);
            return;
        }
        let shoot_anim = if k == kind::AKIMBO || k == kind::AKIMBO_ONLY {
            self.akimbo_fire(host).then(|| if self.status[self.current as usize].toggle { 9 } else { 8 })
        } else {
            if k == kind::SHOTGUN {
                self.shotgun_fire(host);
            } else {
                self.standard_fire(host);
            }
            Some(0)
        };
        if let Some(a) = shoot_anim {
            host.player_anim(a);
        }
    }

    /// `CTSGun::ScaleSpread` 0x802a8: adds the recoil penalty to `spread` and grows it.
    pub fn scale_spread(&mut self, spread: &mut f32, akick: f32, silenced: bool, host: &impl GunHost) {
        if silenced {
            *spread *= 0.875;
        }
        if akick > 0.0 {
            let mut k = if Self::double_fire_rate(host) { 1.0 } else { 2.0 };
            if silenced {
                k *= 0.775;
            }
            let now = host.time();
            let dt = now - self.last_shot;
            if dt >= 0.001 {
                *spread += self.spread_penalty;
                let mut x = host.slow_mul() * dt;
                if x >= 1.0 {
                    x = 0.5;
                }
                self.spread_penalty = ((1.0 - x) * akick * k + self.spread_penalty).min(0.1);
            }
            self.last_shot = now;
        }
    }

    /// `CTSGun::VKickback` 0x80414.
    pub fn vkickback(&self, view_kick: f32, k: u8, silenced: bool, host: &mut impl GunHost) {
        let (p, y) = spread::view_kick(view_kick, k, silenced, Self::double_fire_rate(host));
        host.kick(p, y);
    }

    fn shoot(&mut self, host: &mut impl GunHost, spread: f32, damage: f32, seed: u32, right_offset: f32) {
        let info = self.info();
        let shot = Shot {
            weapon: self.current,
            pellets: info.pellets.max(0) as u32,
            spread,
            damage,
            btype: info.bullet,
            ammo: info.ammo,
            seed,
            right_offset,
        };
        host.shoot(&shot);
        self.last_dir = spread::last_pellet_spread(seed, shot.pellets, [spread, spread], spread::RADIAL_SPREAD);
    }

    /// `CTSGun::StandardFire` 0x824a4.
    pub fn standard_fire(&mut self, host: &mut impl GunHost) {
        let info = self.info();
        let silenced = self.silenced();
        let mut spread = info.accuracy;
        if self.current == id::BARRETT && Self::speed(host) > 35.0 {
            self.spread_penalty = 0.3;
        }
        if self.current == id::M60 && Self::speed(host) > 175.0 {
            self.scale_spread(&mut spread, info.acc_kick * 5.0, silenced, host);
        } else if info.scale_spread {
            self.scale_spread(&mut spread, info.acc_kick, silenced, host);
        }
        let damage = if silenced { info.damage * 0.775 } else { info.damage };
        let seed = host.seed();
        self.shoot(host, spread, damage, seed, 0.0);
        let k = self.kind();
        if k == kind::SNIPER || self.current == id::RAGING_BULL || self.current == id::SAWEDOFF || info.kick {
            self.vkickback(info.view_kick, k, silenced, host);
        }
        if !Self::infinite_clip(host) {
            self.cur().clip -= 1;
            if info.akimbo != -1 {
                let clip = self.status[self.current as usize].clip;
                let tw = &mut self.status[info.akimbo as usize];
                tw.clip -= 1;
                tw.left = clip;
                if tw.clip < 0 {
                    tw.clip = 0;
                }
            }
        }
        let s = self.status[self.current as usize];
        host.event(
            Event::Fire,
            EventArgs { f1: spread, f2: spread, i1: self.current as i32, i2: s.items as i32, b1: (s.clip == 0) as i32, b2: seed as i32 },
        );
        let rate = if s.mode == mode::BURST {
            if s.state == 1 { 0.2 } else { 0.03 }
        } else if s.mode == mode::SEMI && mode_mask(info) & 1 != 0 {
            0.1
        } else {
            info.rate
        };
        self.next_primary = rate;
        self.idle = if self.current == id::CONTENDER && s.clip < 1 && !Self::infinite_clip(host) { 0.5 } else { 2.0 };
    }

    /// `CTSGun::ShotgunFire` 0x82108 (fire / pump alternation in pump mode).
    pub fn shotgun_fire(&mut self, host: &mut impl GunHost) {
        let s = self.status[self.current as usize];
        if s.state != 0 {
            // firing interrupts a shotgun reload
            self.cur().state = 2;
            return;
        }
        let info = self.info();
        let dopump = s.mode == mode::PUMP && s.toggle;
        self.next_primary = if s.mode == mode::SEMI { 0.15 } else { info.rate };
        self.idle = 2.0;
        if !(Self::infinite_clip(host) || s.clip > 0 || dopump) {
            return;
        }
        if !dopump && !Self::infinite_clip(host) {
            self.cur().clip -= 1;
        }
        let seed = host.seed();
        if !dopump {
            self.shoot(host, info.accuracy, info.damage, seed, 0.0);
            if info.kick {
                let k = if mode_mask(info) & 3 != 0 && s.mode != mode::SEMI { 0.35 } else { 1.0 };
                self.vkickback(k * info.view_kick, self.kind(), false, host);
            }
        }
        let items = self.status[self.current as usize].items as i32;
        let pump = (s.mode == mode::PUMP) as i32;
        if dopump {
            host.event(Event::Pump, EventArgs { f1: 0.0, f2: 0.0, i1: self.current as i32, i2: items, b1: pump, b2: 0 });
        } else {
            host.event(
                Event::Fire,
                EventArgs { f1: info.accuracy, f2: info.accuracy, i1: self.current as i32, i2: items, b1: pump, b2: seed as i32 },
            );
        }
        let st = self.cur();
        st.toggle = !st.toggle;
    }

    /// `CTSGun::AkimboFire` 0x829c0: one timer pair, the toggle picks the gun; modes 4/5 with
    /// both triggers (and akimbo auto mode 0) fire a linked double shot.
    pub fn akimbo_fire(&mut self, host: &mut impl GunHost) -> bool {
        let info = self.info();
        let s = self.status[self.current as usize];
        let free_mode = s.mode == mode::AKIMBO_SEMI || s.mode == mode::AKIMBO_AUTO;
        let both = if free_mode {
            let b = if s.mode == mode::AKIMBO_AUTO { host.buttons() } else { host.pressed() };
            b & IN_ATTACK != 0 && b & IN_ATTACK2 != 0
        } else {
            false
        };
        let mut linked = both || s.mode == mode::AUTO;
        if !free_mode || linked {
            let st = self.cur();
            st.toggle = !st.toggle;
            if st.right < 1 {
                linked = false;
                st.toggle = true;
            }
            if st.left < 1 {
                linked = false;
                st.toggle = false;
            }
        } else {
            let buttons = host.buttons();
            if buttons & IN_ATTACK != 0 {
                if s.right < 1 {
                    self.empty_sound(host);
                    return false;
                }
                self.cur().toggle = false;
            }
            if buttons & IN_ATTACK2 != 0 {
                if s.left < 1 {
                    self.empty_sound(host);
                    return false;
                }
                self.cur().toggle = true;
            }
        }
        if !Self::infinite_clip(host) {
            let st = self.cur();
            if linked {
                st.clip -= 2;
                st.left -= 1;
                st.right -= 1;
            } else {
                st.clip -= 1;
                if st.toggle {
                    st.left -= 1;
                } else {
                    st.right -= 1;
                }
            }
            st.clip = st.clip.max(0);
            st.left = st.left.max(0);
            st.right = st.right.max(0);
            let left = st.left;
            if kind_of(self.current) == kind::AKIMBO && info.single >= 0 {
                self.status[info.single as usize].clip = left;
            }
        }
        let silenced = self.silenced();
        let mut spread = info.accuracy;
        if info.scale_spread {
            let akick = if linked { info.acc_kick * 2.0 } else { info.acc_kick };
            self.scale_spread(&mut spread, akick, silenced, host);
        }
        let damage = if silenced { info.damage * 0.775 } else { info.damage };
        let seed = host.seed();
        let toggle = self.status[self.current as usize].toggle;
        if !linked {
            self.shoot(host, spread, damage, seed, 0.0);
        } else {
            let off = if toggle { -5.0 } else { 5.0 };
            self.shoot(host, spread, damage, seed, off);
            self.shoot(host, spread, damage, seed.wrapping_add(0x21), -off);
        }
        if info.kick {
            let vk = if linked { info.view_kick * 2.0 } else { info.view_kick };
            self.vkickback(vk, self.kind(), silenced, host);
        }
        let items = self.status[self.current as usize].items as i32;
        host.event(
            if linked { Event::FireLinked } else { Event::Fire },
            EventArgs { f1: spread, f2: spread, i1: self.current as i32, i2: items, b1: toggle as i32, b2: seed as i32 },
        );
        let s = self.status[self.current as usize];
        let rate = if s.mode == mode::BURST {
            if s.state == 1 { 0.2 } else { 0.03 }
        } else if s.mode == mode::SEMI || s.mode == mode::AKIMBO_SEMI {
            0.1
        } else {
            info.rate
        };
        self.next_secondary = rate;
        self.next_primary = rate;
        self.idle = 2.0;
        true
    }

    /// `CTSGun::ColdCock` 0x81dd0: stock hit, 35 u ahead.
    pub fn cold_cock(&mut self, host: &mut impl GunHost) {
        if !self.has_slot() || self.info().stock == 0 {
            return;
        }
        let stunting = host.iuser4() & STUNTING_MELEE != 0;
        if self.kind() == kind::GRENADE {
            self.grenade_idle(true, host);
        }
        self.stop_reload();
        let rate = if stunting { 1.087_500_1 } else { 0.725 };
        self.unzoom(host);
        self.next_alt = rate;
        self.next_secondary = rate;
        self.next_primary = rate;
        self.next_attack = rate;
        self.idle = 2.0;
        if !self.has_slot() {
            return;
        }
        let mut dmg = self.info().stock as f32;
        if stunting {
            dmg *= 0.666_666_7;
        }
        let r = host.melee(&MeleeHit {
            kind: MeleeKind::ColdCock,
            weapon: self.current,
            reach: 35.0,
            hull: 3,
            damage: dmg,
            bits: 0x0400_1080,
            player_anim: if stunting { -1 } else { 0x27 },
            disarm: false,
            knockback: false,
            level: false,
        });
        match r {
            MeleeResult::Hit { .. } => host.sound(Sound::KevlarHit),
            MeleeResult::Blocked { next_attack, bar_cost } => {
                self.melee_damage = 0.0;
                self.next_attack = next_attack;
                if bar_cost > 0.0 {
                    self.update_melee_bar(bar_cost, host);
                }
            }
            MeleeResult::Miss => {}
        }
        if self.has_slot() && self.status[self.current as usize].owned {
            let items = self.status[self.current as usize].items as i32;
            host.event(
                Event::ColdCock,
                EventArgs { f1: self.last_dir[0], f2: self.last_dir[1], i1: self.current as i32, i2: items, b1: 0, b2: 0 },
            );
        }
    }

    // ---- grenades, blades -----------------------------------------------------------------------

    /// `CTSGun::GrenadeFire` 0x832f0: pull the pin. The throw happens in `grenade_idle`.
    pub fn grenade_fire(&mut self, host: &mut impl GunHost) {
        let s = self.status[self.current as usize];
        if !self.has_slot() || s.state != 0 || s.clip <= 0 {
            return;
        }
        host.weapon_anim(self.seq(19), true, 0);
        host.event(Event::Grenade, EventArgs { i1: 0x13, ..Default::default() });
        self.cur().state = 1;
        self.next_secondary = 0.7;
        self.next_primary = 0.7;
        self.idle = 0.7;
    }

    /// Throw origin and velocity for the M61 (`GrenadeIdle`): the pitch is remapped to
    /// `p * (p >= 0 ? 10/9 : 8/9) - 10`, speed `min(750, (90 - p') * 7)`; a roll starts at the
    /// feet with 2/3 speed.
    pub fn grenade_throw(origin: Vec3, view_ofs: Vec3, velocity: Vec3, v_angle: Vec3, roll: bool) -> (Vec3, Vec3) {
        let p = v_angle[0];
        let pitch = (if p >= 0.0 { 1.111_111_2 } else { 0.888_888_9 }) * p - 10.0;
        let speed = ((90.0 - pitch) * 7.0).min(750.0);
        let (f, _, _) = spread::angle_vectors([pitch, v_angle[1], v_angle[2]]);
        if !roll {
            let src = spread::ma(spread::add(origin, view_ofs), 16.0, f);
            (src, spread::ma(velocity, speed, f))
        } else {
            let src = spread::ma(origin, 16.0, f);
            (src, spread::ma(velocity, speed * 0.666_666_7, f))
        }
    }

    /// `CTSGun::GrenadeIdle(force)` 0x84f18: throws when the button is released (or `force`).
    pub fn grenade_idle(&mut self, force: bool, host: &mut impl GunHost) {
        if !self.has_slot() {
            return;
        }
        let mut done = false;
        let s = self.status[self.current as usize];
        if s.state == 1 {
            if host.buttons() & (IN_ATTACK | IN_ATTACK2) != 0 && !force {
                return;
            }
            let (src, vel) = Self::grenade_throw(host.origin(), host.view_ofs(), host.velocity(), host.v_angle(), s.toggle);
            if self.info().bullet == 4 {
                host.throw_grenade(src, vel, s.toggle);
            }
            host.weapon_anim(self.seq(if s.toggle { 21 } else { 20 }), true, 0);
            host.event(Event::Grenade, EventArgs { i1: 0x14 + s.toggle as i32, ..Default::default() });
            host.player_anim(0x35);
            self.next_secondary = 0.5;
            self.next_primary = 0.5;
            self.idle = 0.5;
            if s.clip == 1 {
                host.set_weapon_model(None);
            }
            let st = self.cur();
            st.state = if force { 0 } else { 2 };
            st.clip -= 1;
            done = force;
        } else if s.state == 2 {
            self.cur().state = 0;
            done = true;
        } else {
            self.idle = 3.0;
        }
        if done {
            host.slots_changed(self.info().slots);
            if self.status[self.current as usize].clip < 1 {
                self.remove_weapon(self.current, true, host);
            } else {
                host.weapon_anim(self.seq(2), true, 0);
                self.idle = 2.0;
            }
        }
    }

    /// `CTSGun::KnifeFire` 0x84044: slash (toggle off) or throw (toggle on).
    pub fn knife_fire(&mut self, host: &mut impl GunHost) {
        if !self.has_slot() {
            return;
        }
        let info = self.info();
        let stunting = host.iuser4() & STUNTING_MELEE != 0;
        let rate = if stunting { info.rate * 1.5 } else { info.rate };
        self.next_secondary = rate;
        self.next_primary = rate;
        self.idle = 3.0;
        let thrown;
        if !self.status[self.current as usize].toggle {
            thrown = false;
            let r = host.random_long(0, 2);
            if !stunting && host.flags() & FL_ONGROUND != 0 {
                host.player_anim(r + 0x1d);
            }
            let mut dmg = if stunting { info.damage * 0.666_666_7 } else { info.damage };
            dmg *= spread::melee_multiplier(self.melee_bar);
            let res = host.melee(&MeleeHit {
                kind: MeleeKind::Knife,
                weapon: self.current,
                reach: 35.0,
                hull: 3,
                damage: dmg,
                bits: 0x1004,
                player_anim: -1,
                disarm: false,
                knockback: false,
                level: false,
            });
            match res {
                MeleeResult::Hit { player } => {
                    if player {
                        self.cur().state = 1;
                        host.sound(Sound::KnifeHitBody);
                    }
                }
                MeleeResult::Blocked { next_attack, bar_cost } => {
                    self.melee_damage = 0.0;
                    self.next_attack = next_attack;
                    self.update_melee_bar(bar_cost, host);
                }
                MeleeResult::Miss => {
                    self.melee_window = info.rate * 0.25;
                    self.melee_damage = dmg;
                    self.melee_source = 1;
                }
            }
            let st = self.status[self.current as usize].state;
            host.event(Event::Knife, EventArgs { i1: self.current as i32, i2: st, ..Default::default() });
        } else {
            host.throw_blade(info.bullet);
            thrown = true;
            if host.iuser4() & 0x30c30 == 0 && host.flags() & FL_ONGROUND != 0 {
                host.player_anim(0x20);
            }
            host.sound(Sound::KnifeMiss);
        }
        if thrown {
            if !Self::infinite_clip(host) {
                host.slots_changed(info.slots);
                self.cur().clip -= 1;
            }
            if self.status[self.current as usize].clip < 1 {
                self.remove_weapon(self.current, true, host);
            } else {
                host.weapon_anim(self.seq(20), true, 0);
                let items = self.status[self.current as usize].items as i32;
                host.event(Event::Draw, EventArgs { i1: self.current as i32, i2: items, ..Default::default() });
                self.idle = 2.0;
            }
        }
    }

    /// `CTSGun::KatanaFire` 0x833f0: four swing variants (stunt, backward, airborne, ground
    /// alternating, crouched).
    pub fn katana_fire(&mut self, host: &mut impl GunHost) {
        if !self.has_slot() {
            return;
        }
        let stunting = host.iuser4() & STUNTING_MELEE != 0;
        let speed = Self::speed(host);
        let (fwd, _, _) = spread::angle_vectors(host.v_angle());
        let backward = if speed <= 100.0 {
            host.buttons() & IN_BACK != 0
        } else {
            spread::dot(fwd, spread::normalize(host.velocity())) <= -0.5
        };
        // (reach / view anim index, damage, delay, player anim, block after)
        let (reach, mut dmg, delay, panim, block): (i32, f32, f32, i32, bool) = if stunting {
            (0x11, 35.0, 1.0, -1, false)
        } else if backward {
            (0xf, 55.0, 1.0, 35, true)
        } else if host.flags() & FL_ONGROUND == 0 {
            (0x11, 50.0, 1.1, 36, false)
        } else if host.buttons() & IN_DUCK == 0 && host.flags() & FL_DUCKING == 0 {
            if !(1..=2).contains(&self.last_melee_anim) {
                self.last_melee_anim = host.random_long(1, 2);
            }
            if self.last_melee_anim == 1 {
                self.last_melee_anim = 2;
                (0x11, 50.0, 0.7, 33, false)
            } else {
                self.last_melee_anim = 1;
                (0x10, 50.0, 0.7, 34, false)
            }
        } else {
            (0x11, 50.0, 0.9, 37, true)
        };
        host.player_anim(panim);
        dmg *= spread::melee_multiplier(self.melee_bar);
        let res = host.melee(&MeleeHit {
            kind: MeleeKind::Katana,
            weapon: self.current,
            reach: reach as f32,
            hull: 3,
            damage: dmg,
            bits: 0x1080,
            player_anim: -1,
            disarm: false,
            knockback: false,
            level: false,
        });
        match res {
            MeleeResult::Hit { player } => {
                if player {
                    self.cur().state = 1;
                    host.sound(Sound::KatanaHitBody);
                }
            }
            MeleeResult::Blocked { next_attack, bar_cost } => {
                self.melee_damage = 0.0;
                self.next_attack = next_attack;
                self.update_melee_bar(bar_cost, host);
            }
            MeleeResult::Miss => {
                self.melee_window = 0.25;
                self.melee_damage = dmg;
                self.melee_source = 2;
            }
        }
        if block && host.iuser4() & 1 == 0 {
            host.block_player(delay * 0.9);
        }
        let st = self.status[self.current as usize].state;
        host.event(Event::Katana, EventArgs { i1: reach, i2: st, ..Default::default() });
        self.next_secondary = delay;
        self.next_primary = delay;
        self.idle = 3.0;
    }

    /// `CTSGun::KatanaThrow` 0x83b68 (cooldown `info.accuracy` = 0.7 s).
    pub fn katana_throw(&mut self, host: &mut impl GunHost) {
        if !self.has_slot() {
            return;
        }
        let info = self.info();
        self.next_secondary = info.accuracy;
        self.next_primary = info.accuracy;
        self.idle = 3.0;
        host.throw_blade(info.bullet);
        if host.iuser4() & 0x30c30 == 0 && host.flags() & FL_ONGROUND != 0 {
            host.player_anim(0x20);
        }
        host.sound(Sound::KatanaMiss);
        if !Self::infinite_clip(host) {
            host.slots_changed(info.slots);
            self.remove_weapon(self.current, true, host);
        }
    }

    // ---- kung fu ------------------------------------------------------------------------------

    /// `CTSGun::CCombatAttack` 0x86078. Decoded from the decompile with the stack slots
    /// re-associated (docs/implementation/weapons.md, "Kung fu"): the next-attack delay is the
    /// value TS also stores in `pl+0x4f8`, the shorter value is the `BlockPlayer` time.
    pub fn ccombat_attack(&mut self, host: &mut impl GunHost) {
        if self.next_attack > 0.0 || self.next_primary > 0.0 || self.next_secondary > 0.0 {
            return;
        }
        let iuser4 = host.iuser4();
        let stunting = iuser4 & STUNTING_KUNGFU != 0;
        let vel = host.velocity();
        let speed = spread::length(vel);
        let on_ground = host.flags() & FL_ONGROUND != 0;
        let buttons = host.buttons();
        let pressed = host.pressed();
        let mut dot = 0.0;
        let backward = if speed <= 100.0 {
            buttons & IN_BACK != 0
        } else {
            let a = host.v_angle();
            let (f, _, _) = spread::angle_vectors([0.0, a[1], a[2]]);
            dot = spread::dot(f, spread::normalize(vel));
            dot <= -0.5
        };

        let mut reach = if backward { 3 } else { 2 };
        // attack type (event iparam1): 0 none, 1 punch, 2 kick, 3 running back kick
        let mut atype = 0;
        let mut view_anim = 0; // event iparam2, index into the kung-fu sequence table
        let mut dmg = 0.0f32;
        let mut delay = 0.6f32; // next attack
        let mut block_time = 0.0f32; // BlockPlayer(block_time * 0.75) for ground kicks
        let mut block_after = false;
        let mut panim = 0; // CTSAnimation sequence
        let mut stand_fix = true;

        let pick = |host: &mut dyn FnMut(i32, i32) -> i32, last: i32, table: &[(i32, i32)], choices: &[i32]| -> i32 {
            for (prev, next) in table {
                if last == *prev {
                    return *next;
                }
            }
            choices[(host(0, choices.len() as i32 - 1)) as usize]
        };

        if stunting {
            view_anim = -1;
            delay = 0.55;
            block_time = 0.15;
            dmg = 15.0;
            self.punch_combo = 0;
            atype = 1;
            reach = 2;
            panim = 35;
        } else if buttons & IN_ATTACK == 0 {
            if buttons & IN_ATTACK2 != 0 {
                if on_ground && (buttons & IN_DUCK != 0 || host.flags() & FL_DUCKING != 0) {
                    // uppercut
                    dmg = 35.0;
                    atype = 2;
                    block_after = true;
                    view_anim = 20;
                    panim = 45;
                    delay = 1.0;
                    block_time = 0.35;
                    reach = 2;
                } else if backward {
                    atype = 3;
                    view_anim = 17;
                    dmg = (speed * 0.303_030_3).clamp(15.0, 80.0);
                    panim = 55;
                    if on_ground {
                        block_after = true;
                        delay = 1.0;
                    } else {
                        delay = 1.35;
                    }
                    block_time = 0.3;
                } else {
                    atype = 2;
                    if !on_ground && speed > 270.0 && buttons & IN_DUCK != 0 && dot > 0.5 {
                        // flying kick
                        self.kick_combo = 0;
                        dmg = (speed * 0.153_846_16).clamp(15.0, 45.0);
                        panim = 45;
                        delay = 1.0;
                        block_time = 0.25;
                        view_anim = 16;
                        stand_fix = false;
                    } else if on_ground || speed <= 200.0 {
                        block_after = true;
                        panim = 45;
                        if pressed & IN_ATTACK2 != 0 || self.kick_combo > 2 {
                            delay = 0.5;
                            block_time = 0.3;
                            self.kick_combo = 0;
                            let mut r = |lo, hi| host.random_long(lo, hi);
                            view_anim = pick(&mut r, self.last_melee_anim, &[(15, 10), (10, 14), (14, 13), (13, 15)], &[14, 13, 10, 15]);
                            dmg = 25.0;
                        } else {
                            self.kick_combo += 1;
                            delay = 0.3;
                            block_time = 0.2;
                            dmg = 15.0;
                            let mut r = |lo, hi| host.random_long(lo, hi);
                            view_anim = pick(&mut r, self.last_melee_anim, &[(11, 12), (12, 11)], &[11, 12]);
                        }
                    } else {
                        // airborne speed kick
                        self.kick_combo = 0;
                        dmg = (speed * 0.1).clamp(15.0, 30.0);
                        panim = 45;
                        delay = 0.75;
                        block_time = 0.25;
                        let mut r = |lo, hi| host.random_long(lo, hi);
                        view_anim = pick(&mut r, self.last_melee_anim, &[(15, 19), (19, 18), (18, 15)], &[19, 18, 15]);
                    }
                }
            }
        } else {
            // punches
            if !on_ground {
                view_anim = 24;
                delay = 0.5;
                block_time = 0.15;
                dmg = 25.0;
                self.punch_combo = 0;
            } else if self.punch_combo > 2 || (buttons & IN_DUCK != 0 && host.flags() & FL_DUCKING == 0) {
                view_anim = 25;
                delay = 0.65;
                block_time = 0.15;
                dmg = 30.0;
                self.punch_combo = 0;
            } else if pressed & IN_ATTACK != 0 {
                // a fresh press: straight punch
                delay = 0.35;
                block_time = 0.15;
                self.punch_combo = 0;
                view_anim = 23;
                dmg = 20.0;
            } else {
                // held: fast jabs (the combo counter leads to the heavy punch)
                self.punch_combo += 1;
                view_anim = 22;
                delay = 0.25;
                block_time = 0.1;
                dmg = 10.0;
            }
            atype = 1;
            reach = 2;
            panim = 35;
        }

        if atype != 0 {
            if stand_fix && iuser4 & 0x20000 != 0 {
                // leave a wall climb
                host.set_iuser4(iuser4 & !0x20000 & !0x4);
            }
            self.last_melee_anim = view_anim;
            dmg *= spread::melee_multiplier(self.melee_bar);
            if host.kungfu() {
                dmg += dmg;
            }
            let back = reach == 3;
            let res = host.melee(&MeleeHit {
                kind: MeleeKind::KungFu,
                weapon: 0,
                reach: if back { -(reach as f32) } else { reach as f32 },
                hull: if back { 1 } else { 3 },
                damage: dmg,
                bits: 0x0400_1080,
                player_anim: panim,
                disarm: true,
                knockback: true,
                level: back,
            });
            match res {
                MeleeResult::Hit { .. } => {}
                MeleeResult::Blocked { next_attack, bar_cost } => {
                    self.next_attack = next_attack;
                    if bar_cost > 0.0 {
                        self.update_melee_bar(bar_cost, host);
                    }
                    self.arm_kungfu_window(dmg, delay, back);
                }
                MeleeResult::Miss => self.arm_kungfu_window(dmg, delay, back),
            }
            if block_after && host.iuser4() & 1 == 0 {
                host.block_player(block_time * 0.75);
            }
            host.event(Event::CloseCombat, EventArgs { f1: 0.0, f2: 0.0, i1: atype, i2: view_anim, b1: 0, b2: 0 });
        }
        self.next_secondary = delay;
        self.next_primary = delay;
        self.idle = 2.0;
        self.block_kungfu = 0.0;
    }

    fn arm_kungfu_window(&mut self, dmg: f32, window: f32, back: bool) {
        if dmg > 0.001 {
            self.melee_window = window;
            self.melee_damage = if back { -dmg } else { dmg };
            self.melee_source = 0;
        }
    }

    /// `CTSGun::CCombatBlock` 0x85ebc (reload key with kung fu or a blade).
    pub fn ccombat_block(&mut self, host: &mut impl GunHost) {
        if self.next_attack > 0.0 || self.next_primary > 0.0 || self.next_secondary > 0.0 || self.next_alt > 0.0 {
            return;
        }
        if host.iuser4() & STUNTING_MELEE != 0 || self.melee_bar < 0.01 {
            return;
        }
        let i2 = if !self.has_slot() {
            self.block_kungfu = 0.5;
            1
        } else {
            match self.kind() {
                kind::KATANA => self.block_katana = 0.5,
                kind::KNIFE => self.block_knife = 0.5,
                _ => {}
            }
            0x12
        };
        host.event(Event::CloseCombat, EventArgs { f1: self.current as f32, f2: 0.0, i1: 0, i2, b1: 0, b2: 0 });
        host.player_anim(0x26);
        self.update_melee_bar(33.0, host);
        self.next_secondary = 0.5;
        self.next_primary = 0.5;
        self.idle = 3.0;
    }

    /// `CTSGun::CCombatIdle` 0x85e54.
    pub fn ccombat_idle(&mut self, host: &mut impl GunHost) {
        if self.idle <= 0.0 {
            self.idle = 3.0;
            host.weapon_anim(anim(0, false, 0), false, 0);
        }
    }

    // ---- reload, idle ---------------------------------------------------------------------------

    /// `CTSGun::Reload` 0x84d5c.
    pub fn reload(&mut self, host: &mut impl GunHost) {
        if !self.has_slot() {
            self.ccombat_block(host);
            return;
        }
        match self.kind() {
            kind::GRENADE => return,
            kind::KNIFE | kind::KATANA => {
                self.ccombat_block(host);
                return;
            }
            kind::SHOTGUN => {
                self.shotgun_reload(host);
                return;
            }
            _ => {}
        }
        if self.current_reserve() < 1 || self.status[self.current as usize].state != 0 {
            return;
        }
        let info = self.info();
        if self.reload_weapon(info.clip, info.reload) {
            self.idle = 5.0;
            self.unzoom(host);
            let items = self.status[self.current as usize].items as i32;
            host.event(Event::Reload, EventArgs { i1: self.current as i32, i2: items, ..Default::default() });
            host.player_anim(0x33);
        }
    }

    /// `CTSGun::ReloadWeapon(clip_size, anim, time)` 0x84a14.
    fn reload_weapon(&mut self, clip_size: i32, time: f32) -> bool {
        if !self.has_slot() || self.current_reserve() <= 0 {
            return false;
        }
        let j = (clip_size - self.status[self.current as usize].clip).min(self.current_reserve());
        if j > 0 {
            self.cur().in_reload = 1;
            self.next_attack = time;
            return true;
        }
        false
    }

    /// `CTSGun::ShotgunReload` 0x84aac: phase 1 start (0.3 s), phase 2 one shell per
    /// `info.reload`, phase 3 end.
    pub fn shotgun_reload(&mut self, host: &mut impl GunHost) {
        let info = self.info();
        let ammo = (info.ammo as usize).min(MAX_AMMO - 1);
        let items = self.status[self.current as usize].items as i32;
        let id = self.current as i32;
        match self.status[self.current as usize].state {
            0 => {
                if self.reserve[ammo] > 0 && self.status[self.current as usize].clip < info.clip {
                    self.unzoom(host);
                    if host.view_model_hidden() {
                        host.set_view_model(Some(self.current));
                    }
                    self.cur().state = 1;
                    host.event(Event::Reload, EventArgs { f1: 8.0, i1: id, i2: items, ..Default::default() });
                    self.next_attack = 0.3;
                    self.idle = 0.3;
                    self.next_primary = 0.3;
                    self.next_secondary = 0.3;
                }
            }
            1 => {
                if self.idle <= 0.0 {
                    if self.reserve[ammo] < 1 || info.clip <= self.status[self.current as usize].clip {
                        self.cur().state = 2;
                    } else {
                        self.reserve[ammo] -= 1;
                        self.cur().clip += 1;
                        host.event(Event::Reload, EventArgs { f1: 9.0, i1: id, i2: items, ..Default::default() });
                        self.idle = info.reload;
                    }
                }
            }
            2 => {
                let st = self.cur();
                st.state = 0;
                st.toggle = false;
                host.event(Event::Reload, EventArgs { f1: 10.0, i1: id, i2: items, ..Default::default() });
                self.next_attack = 0.3;
                self.idle = 3.0;
            }
            _ => {}
        }
    }

    /// `CTSGun::StopReload` 0x81288.
    pub fn stop_reload(&mut self) {
        if self.has_slot() && self.status[self.current as usize].in_reload != 0 {
            let shotgun = self.kind() == kind::SHOTGUN;
            let st = self.cur();
            st.in_reload = 0;
            if shotgun {
                st.state = 0;
                st.toggle = false;
            }
            self.idle = 0.0;
            self.next_attack = 0.0;
        }
    }

    /// `CTSGun::WeaponIdle` 0x854dc (also runs bursts and auto-reloads an empty gun).
    pub fn weapon_idle(&mut self, host: &mut impl GunHost) {
        if !self.has_slot() {
            self.ccombat_idle(host);
            return;
        }
        self.clip_mirror = self.status[self.current as usize].clip;
        if self.kind() == kind::SHOTGUN && self.status[self.current as usize].state != 0 {
            self.reload(host);
            return;
        }
        if self.status[self.current as usize].mode == mode::BURST && self.next_primary <= 0.0 {
            if self.status[self.current as usize].state < 1 {
                self.cur().state = 0;
            } else {
                self.fire(host);
                if self.has_slot() {
                    self.cur().state -= 1;
                }
            }
        }
        if !self.has_slot() || self.idle > 0.0 {
            return;
        }
        if self.kind() == kind::GRENADE {
            self.grenade_idle(false, host);
        } else if !Self::infinite_clip(host) && self.status[self.current as usize].clip < 1 {
            self.reload(host);
        } else if self.status[self.current as usize].clip != 0 {
            self.idle = 3.0;
            let body = self.status[self.current as usize].body;
            host.weapon_anim(self.seq(0), false, body);
        }
    }

    // ---- zoom, modes, attachments ---------------------------------------------------------------

    /// `CTSGun::Zoom(fov)` 0x81710: FOV below 40 hides the view model.
    pub fn zoom(&mut self, fov: i32, host: &mut impl GunHost) {
        host.set_fov(fov as f32);
        if self.info().zoom[0] < 40 {
            host.set_view_model(None);
        }
    }

    /// `CTSGun::Unzoom` 0x81748.
    pub fn unzoom(&mut self, host: &mut impl GunHost) {
        host.set_fov(0.0);
        if host.view_model_hidden() && self.has_slot() {
            host.set_view_model(Some(self.current));
        }
    }

    /// `CTSGun::SwitchFireMode` 0x8169c (`firemode` command).
    pub fn switch_fire_mode(&mut self) {
        if !self.has_slot() {
            return;
        }
        let mask = mode_mask(self.info());
        if mask & 0x3f == 0 {
            return;
        }
        let mut m = self.status[self.current as usize].mode as u32 + 1;
        loop {
            if m > 5 {
                m = 0;
            }
            if (mask >> m) & 1 != 0 {
                self.cur().mode = m as u8;
                break;
            }
            m += 1;
        }
        self.mode_memory[self.current as usize] = self.status[self.current as usize].mode + 1;
    }

    /// `CTSGun::SwitchItems` 0x8155c (`useitems`): laser -> flashlight -> both -> off.
    pub fn switch_items(&mut self, host: &mut impl GunHost) {
        if !self.has_slot() {
            return;
        }
        let st = self.cur();
        let (owned, on) = (st.items, st.active);
        if owned & 6 == 6 {
            st.active = if on & 6 == 6 {
                0
            } else if on & 2 != 0 {
                (on | 4) & !2
            } else {
                on | 2
            };
        } else if owned & 2 == 0 {
            if owned & 4 == 0 {
                st.active = 0;
                return;
            }
            st.active = if on & 4 == 0 { on | 4 } else { 0 };
        } else {
            st.active = if on & 2 == 0 { on | 2 } else { 0 };
        }
        host.sound(Sound::Switch);
    }

    // ---- inventory ----------------------------------------------------------------------------

    /// `CTSGun::SelectWeapon(id)` 0x80fb8. The caller deploys on success.
    pub fn select_weapon(&mut self, id: u8, host: &mut impl GunHost) -> bool {
        if id == self.current || id as usize >= MAX_WEAPONS {
            return false;
        }
        if !((self.status[id as usize].owned && info(id).is_some()) || id == 0) {
            return false;
        }
        self.holster_weapon(host);
        self.current = id;
        true
    }

    /// `SelectWeapon` + `Deploy`.
    pub fn switch_to(&mut self, id: u8, host: &mut impl GunHost) -> bool {
        if self.select_weapon(id, host) {
            self.deploy(host);
            true
        } else {
            false
        }
    }

    /// `CTSGun::Deploy` 0x81030.
    pub fn deploy(&mut self, host: &mut impl GunHost) {
        if !self.has_slot() {
            self.holster_weapon(host);
            return;
        }
        let id = self.current;
        let items = self.status[id as usize].items;
        let body = body_by_items(id, items);
        {
            let st = self.cur();
            st.body = body;
            st.state = 0;
            st.alt_grip = false;
        }
        let draw = anim(id, false, 2);
        // CBasePlayerWeapon::DefaultDeploy(v, p, anim, ext, skiplocal 0, body)
        host.set_view_model(Some(id));
        host.set_weapon_model(Some((id, items & att::SILENCER != 0)));
        host.set_anim_extension(tables().asset(id).map(|a| a.anim_ext.as_str()).unwrap_or(""));
        host.weapon_anim(draw, false, body);
        self.next_attack = 0.5;
        self.idle = 1.0;
        host.event(Event::Draw, EventArgs { i1: id as i32, i2: items as i32, ..Default::default() });
        host.player_anim(0x34);
        let mem = &mut self.mode_memory[id as usize];
        if *mem < 1 {
            *mem = self.status[id as usize].mode + 1;
        } else {
            self.status[id as usize].mode = *mem - 1;
        }
    }

    /// `CTSGun::HolsterWeapon` 0x85698: back to kung fu.
    pub fn holster_weapon(&mut self, host: &mut impl GunHost) {
        host.set_view_model(None);
        host.set_weapon_model(None);
        // pl+0xb58 / pl+0x478 = "" (no weapon animation extension).
        host.set_anim_extension("");
        self.unzoom(host);
        self.next_attack = 0.5;
        self.idle = 3.0;
        if self.melee_window != 0.0 && self.melee_source != 0 {
            self.melee_window = 0.0;
            self.melee_damage = 0.0;
            self.melee_source = 0;
        }
        if !host.alive() {
            return;
        }
        if self.has_slot() {
            if self.kind() == kind::GRENADE {
                self.grenade_idle(true, host);
            } else {
                let st = self.cur();
                st.in_reload = 0;
                st.state = 0;
            }
        }
        self.current = 0;
        host.set_view_model(Some(0));
        host.weapon_anim(anim(0, false, 2), false, 0);
        host.event(Event::Draw, EventArgs::default());
        self.kick_combo = 0;
        self.punch_combo = 0;
        self.last_melee_anim = 0;
    }

    /// `CTSGun::GetBetterWeapon(excluded)` 0x80f40: switch by `g_iWeaponPriority`.
    pub fn get_better_weapon(&mut self, excluded: u8, host: &mut impl GunHost) {
        let mut pick = PRIORITY[MAX_WEAPONS - 1];
        for &id in PRIORITY.iter() {
            if self.status[id as usize].owned && id != excluded {
                pick = id;
                break;
            }
        }
        self.switch_to(pick, host);
    }

    /// `CTSGun::DropWeapon(id, better)` 0x8094c: removes the record. Returns true if owned.
    pub fn remove_weapon(&mut self, id: u8, better: bool, host: &mut impl GunHost) -> bool {
        if !(1..=37).contains(&id) || !self.status[id as usize].owned {
            return false;
        }
        self.status[id as usize] = Status::default();
        if host.alive() && id == self.current {
            if better {
                self.get_better_weapon(id, host);
            } else {
                self.holster_weapon(host);
            }
        }
        true
    }

    /// `CTSGun::TakeWeapon(id, status, attachments)` 0x80a04. Returns the record that changed
    /// (the akimbo id when a second single pistol turns into the pair), or `None`.
    pub fn take_weapon(&mut self, id: u8, from: Option<&Status>, attachments: u32) -> Option<u8> {
        let w = info(id)?;
        let mut target = id;
        let owned = self.status[id as usize].owned;
        let akimbo = w.akimbo;
        let ok = if !owned {
            akimbo == -1 || (info(akimbo as u8).is_some() && !self.status[akimbo as usize].owned)
        } else if !w.stack {
            if akimbo != -1 && info(akimbo as u8).is_some() && !self.status[akimbo as usize].owned {
                target = akimbo as u8;
                true
            } else {
                false
            }
        } else {
            self.status[id as usize].clip < w.pellets
        };
        if !ok {
            return None;
        }
        let t = target as usize;
        let tw = info(target)?;
        if tw.stack && self.status[t].owned {
            self.status[t].clip += 1;
            return Some(target);
        }
        let mask = mode_mask(tw);
        match from {
            None => {
                let mut s = Status { owned: true, ammo: tw.ammo, mode: default_mode(mask), ..Default::default() };
                if !is_akimbo(target) {
                    s.clip = tw.clip;
                    if is_only_akimbo(target) {
                        s.left = tw.clip / 2;
                        s.right = tw.clip / 2;
                    }
                    s.items = attachments | tw.att_default as u32;
                    s.active = 0;
                } else {
                    let single = tw.single.max(0) as usize;
                    s.left = self.status[single].clip;
                    s.right = info(single as u8).map(|i| i.clip).unwrap_or(0);
                    s.clip = s.left + s.right;
                    let items = attachments | self.status[single].items | self.status[t].items;
                    s.items = items;
                    self.status[single].items = items;
                    let active = self.status[t].active | self.status[single].active;
                    s.active = active;
                    self.status[single].active = active;
                }
                s.items &= tw.att_allowed as u32;
                if s.items & att::LASER != 0 {
                    s.active |= att::LASER;
                }
                if s.items & att::FLASHLIGHT != 0 {
                    s.active |= att::FLASHLIGHT;
                }
                self.status[t] = s;
            }
            Some(from) => {
                let mut s = *from;
                s.owned = true;
                s.state = 0;
                s.in_reload = 0;
                if is_akimbo(target) {
                    let single = tw.single.max(0) as usize;
                    s.clip += self.status[single].clip;
                    s.left = self.status[single].clip;
                    s.right = s.clip - s.left;
                    let items = attachments | self.status[single].items | s.items;
                    s.items = items;
                    self.status[single].items = items;
                    let active = s.active | self.status[single].active;
                    s.active = active;
                    self.status[single].active = active;
                    s.mode = default_mode(mask);
                }
                s.items &= tw.att_allowed as u32;
                self.status[t] = s;
            }
        }
        Some(target)
    }

    /// `CTSGun::AddItem(bits)` 0x80794: buys an attachment for the current weapon (redeploys).
    pub fn add_item(&mut self, bits: u32, host: &mut impl GunHost) -> bool {
        if !self.has_slot() {
            return false;
        }
        let info = self.info();
        let s = self.status[self.current as usize];
        if bits & info.att_allowed as u32 == 0 || bits & info.att_default as u32 != 0 || bits & s.items != 0 {
            return false;
        }
        self.cur().items |= bits;
        if !is_akimbo(self.current) {
            if info.akimbo != -1 && self.status[info.akimbo as usize].owned {
                self.status[info.akimbo as usize].items |= bits;
            }
        } else if info.single >= 0 {
            self.status[info.single as usize].items |= bits;
        }
        let cur = self.current;
        self.current = 0;
        if self.select_weapon(cur, host) {
            self.deploy(host);
        }
        true
    }

    /// `CTSGun::AddAmmo(type, n)` 0x808ac, clamped to `max_carry`.
    pub fn add_ammo(&mut self, ammo: u8, n: i32, max_carry: i32) -> bool {
        let a = ammo as usize;
        if a >= MAX_AMMO || n <= 0 || self.reserve[a] >= max_carry {
            return false;
        }
        self.reserve[a] = (self.reserve[a] + n).min(max_carry);
        true
    }

    /// `CTSGun::MultiplyClips` 0x81308 (Infinite Clip pickup). Stackables refill from the free
    /// slots: returns the slots consumed.
    pub fn multiply_clips(&mut self, free_slots: i32) -> Option<i32> {
        if !self.has_slot() || self.current == id::M61 {
            return None;
        }
        self.stop_reload();
        let info = self.info();
        if !info.stack {
            let clip = info.clip * 2;
            self.cur().clip = clip;
            if info.akimbo == -1 {
                let k = self.kind();
                if k == kind::AKIMBO || k == kind::AKIMBO_ONLY {
                    let half = (clip as f32 * 0.5).round_ties_even() as i32;
                    let st = self.cur();
                    st.left = half;
                    st.right = half;
                    if info.single >= 0 && info.single as u8 != self.current {
                        self.status[info.single as usize].clip = half;
                    }
                }
            } else {
                let tw = &mut self.status[info.akimbo as usize];
                tw.clip = (clip as f32 * 0.5 + tw.clip as f32).round_ties_even() as i32;
                tw.left = clip;
            }
            Some(0)
        } else {
            let want = (info.pellets * 2 - self.status[self.current as usize].clip) * info.slots;
            let used = want.min(free_slots);
            if info.slots > 0 {
                self.cur().clip += used / info.slots;
            }
            Some(used)
        }
    }

    /// Clears the whole inventory (`CTSGun::Reset` 0x805e0); timers and the bar stay.
    pub fn reset_inventory(&mut self) {
        self.status = [Status::default(); MAX_WEAPONS];
        self.reserve = [0; MAX_AMMO];
        self.current = 0;
    }

    /// Per-spawn reset (`TSInit` + `CTSGun::Spawn`).
    pub fn reset_for_spawn(&mut self) {
        let status = self.status;
        let reserve = self.reserve;
        let current = self.current;
        *self = Gun::default();
        self.status = status;
        self.reserve = reserve;
        self.current = current;
    }
}

#[cfg(test)]
mod tests;
