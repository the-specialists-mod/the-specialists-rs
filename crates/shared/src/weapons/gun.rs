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
use crate::gameplay::{GunRules, Predicted};
use crate::pm::{Arith, Fpu, FpuMath, Num};
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
    /// Use `v_angle` with roll 0 (the running back kick, `CCombatAttack` 0x76af1).
    pub zero_roll: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MeleeResult {
    Miss,
    /// The victim blocked (`Victim*Hold`). The attacker gets these values.
    Blocked {
        next_attack: f32,
        bar_cost: f32,
    },
    /// Something alive was hit; `player` if it was a player.
    Hit {
        player: bool,
    },
}

/// The `CTSGun` virtual functions TS calls through the gun's vtable that a server plugin may hook
/// (hamsandwich's `Item_*` / `Weapon_*` on `weapon_tsgun`). The gun calls
/// [`GunHost::virtual_pre`] and [`GunHost::virtual_post`] around each call TS makes through the
/// vtable; the prediction's host runs them plainly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Virtual {
    /// `CTSGun::Deploy` (0x81030), returns 1.
    Deploy,
    /// `CTSGun::Holster(skiplocal)` (0x85674): `HolsterWeapon`.
    Holster,
    /// `CBasePlayerWeapon::ItemPostFrame` (0xa03d4).
    PostFrame,
    /// `CTSGun::PrimaryAttack` (0x81a20), `SecondaryAttack` (0x817a8), `AlternateAttack`
    /// (0x81ba8), `Reload` (0x84d5c), `WeaponIdle` (0x854dc), from `ItemPostFrame` (`Reload`
    /// also from `WeaponIdle`).
    PrimaryAttack,
    SecondaryAttack,
    AlternateAttack,
    Reload,
    WeaponIdle,
    /// `CTSGun::ShouldWeaponIdle` (0x87274), returns 1.
    ShouldWeaponIdle,
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
    /// `player/closecombat.wav` (a missed stock hit, in rulesets that play it).
    CloseCombatMiss,
    /// `player/block.wav` (a kung-fu block, in rulesets that play it from the gun).
    Block,
}

/// Everything the gun needs from the player and the engine.
pub trait GunHost {
    fn time(&self) -> f32;
    /// `GiveSlowMul()`: the player's time scale (`pev->fuser1`, [`spread::fixed_slow_amt`]).
    fn slow_mul(&self) -> f32;
    /// The player's time scale as it is (`pev->fuser1`, no floor), for rules that scale by it
    /// themselves. The default is [`slow_mul`](Self::slow_mul).
    fn time_scale(&self) -> f32 {
        self.slow_mul()
    }
    /// This is the client's weapon prediction, not the server. The two copies of some TS
    /// functions differ (the view kick, rounding), and rules that follow them need to know.
    fn prediction(&self) -> bool {
        false
    }
    /// The TS build whose floating point this side reproduces ([`spread`]): the server's
    /// `ts_pm_arith` selection, `Arith::CLIENT` for the prediction.
    fn arith(&self) -> Arith;
    /// The active gameplay rules for this command, resolved once from the player's physinfo key
    /// (`crate::gameplay::PHYSINFO_KEY`) when the host is built: the server's from its own
    /// physinfo, the prediction's from `clientdata_t::physinfo`.
    fn rules(&self) -> &'static dyn crate::gameplay::GunRules;
    /// The client's `g_runfuncs` (`0x100d5cbc`): this command is predicted for the first time.
    /// `client.dll`'s `ScaleSpread` (`0x10008100`) only adds and grows the recoil penalty then.
    /// Always true on the server.
    fn first_prediction(&self) -> bool {
        true
    }
    /// The kung-fu power [`Gun::ccombat_block`] gates on, or `None` for the gun's own
    /// [`Gun::melee_bar`] (`pl+0x774`), which the block then charges: the server. `client.dll`
    /// keeps no bar: its block (`0x1000adc0`) gates on the HUD's networked power (`0x1018a718`)
    /// and charges nothing.
    fn kfu_power(&self) -> Option<f32> {
        None
    }
    /// The records carry their owned flag (`s_weapon_status+0`), as the server's do. The
    /// prediction's never do: `client.dll`'s rebuild (`0x10003965..0x10003bbe`) never writes it,
    /// so its `DropWeapon` (`0x100085d0`) and `SelectWeapon` (`0x100086b0`) find no weapon, and
    /// its `ColdCock` plays the event without testing it (`0x10008dc8`).
    fn tracks_owned(&self) -> bool {
        true
    }
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
    /// `CTSGun::Deploy` (0x81030) with a weapon zeroes the gun's `UpdateClientData` timer
    /// (+0x9e0), so the new weapon's `WeaponInfo` goes out on the next update. Server only.
    fn deploy_update_now(&mut self) {}
    /// The timer tail of `CBasePlayerWeapon::DefaultDeploy` for weapon `id`: returns
    /// `(m_flNextAttack, m_flTimeWeaponIdle)`. The default is `client.dll`'s (`0x100035f0`: 0.5,
    /// 3.0, no sound). The server's (`ts_i386.so` 0xa0178, `mp.dll` 0x10073c50) holds the attack
    /// until spawn protection (`pl+0x71c`) ends, gives the Contender a 0.75 s idle, and plays
    /// `weapons/draw.wav`.
    fn default_deploy(&mut self, id: u8) -> (f32, f32) {
        let _ = id;
        (0.5, 3.0)
    }
    /// `SendWeaponAnim(anim, skiplocal, body)`. `body` is always the gun's own `pev->body`
    /// ([`Gun::body`]): TS's `CBasePlayerWeapon::SendWeaponAnim` (0xa00d0) ignores its body
    /// argument and writes `pev->body`, and so does `client.dll`'s (0x10003680).
    fn weapon_anim(&mut self, anim: i32, skip_local: bool, body: i32);
    /// `CTSAnimation::Animate(n)` (third-person sequence; owned by the movement package).
    fn player_anim(&mut self, anim: i32);
    /// `VKickback` result. TS writes it to `v_angle` only for bots; the client applies it to the
    /// local view and, with `shake`, starts the view shake (TS 3.0's client `VKickback`).
    fn kick(&mut self, pitch: f32, yaw: f32, shake: bool);
    /// The client's melee view shake, `ViewShake::Start(amp, 0.2, 10)`: the client copies of
    /// `ColdCock` (`0x10008da1`), `KatanaFire` (`0x1000a156`), `KnifeFire` (`0x1000a327`) and
    /// `CCombatAttack` (`0x1000b5c7`) start it on the first prediction of the command. The server
    /// has no view shake.
    fn view_shake(&mut self, _amp: f32) {}
    fn sound(&mut self, s: Sound);

    /// Fires `shot` (`FireBullets` and its hits). `gun` is the calling gun: what the hits set off
    /// can reach the shooter's own `CTSGun` mid-call in TS (a shot `func_breakable` firing a
    /// `game_player_team` that kills the shooter), so the server lends it to the player meanwhile
    /// and the attack carries on with whatever the world left in it.
    fn shoot(&mut self, shot: &Shot, gun: &mut Gun);
    /// A melee swing; `gun` as for [`shoot`](Self::shoot).
    fn melee(&mut self, hit: &MeleeHit, gun: &mut Gun) -> MeleeResult;
    /// Throws the M61 (`CM61Grenade::CreateGrenade`, fuse 3 s of the thrower's time).
    fn throw_grenade(&mut self, origin: Vec3, velocity: Vec3, roll: bool);
    /// Throws a knife/seal knife/katana (`CTSKnife::CreateKnife`, `btype` 6/7/8).
    fn throw_blade(&mut self, btype: i32);
    /// Inventory weight freed (positive) or used (negative); the server updates speed.
    fn slots_changed(&mut self, delta_free: i32);
    /// `UpdateMeleeBar` sends `KFuPower`.
    fn melee_bar_changed(&mut self, bar: f32);
    /// `CBasePlayer::BlockPlayer(t)`: half speed (`iuser4 |= 1`) for `t` seconds. Server only:
    /// the client copies of `KatanaFire` and `CCombatAttack` never block.
    fn block_player(&mut self, t: f32);
    /// `pl+0x4f8` (`CTSStunt` +0x08, the stunts' close-combat cooldown): the server `KatanaFire`
    /// (0x73b54), `KnifeFire` (0x740e1) and `CCombatAttack` store their attack delay there. The
    /// client copies have no such field.
    fn set_close_combat_cooldown(&mut self, _t: f32) {}
    /// The forced crouch of the uppercut and the flying kick (`CCombatAttack` 0x764e4, 0x766d2) and
    /// the crouched katana swing (`KatanaFire` 0x73665): `iuser4 |= bits`, `view_ofs = (0, 0, 16)`
    /// and `pl+0x4f4` (the stunt recover timer) = 2. Server only: the client copies don't.
    fn melee_crouch(&mut self, _bits: i32) {}
    /// A melee move that holds the player: `iuser4 |= bits`, the crouched view (`crouch`) and the
    /// stunt recovery for `recover` seconds. The server implements it; the default does
    /// [`melee_crouch`](Self::melee_crouch) for a crouch and sets the bits otherwise (TS 3.0's
    /// moves only use `melee_crouch`, with its own 2 s recovery).
    fn melee_hold(&mut self, bits: i32, recover: f32, crouch: bool) {
        let _ = recover;
        if crouch {
            self.melee_crouch(bits);
        } else {
            let v = self.iuser4();
            self.set_iuser4(v | bits);
        }
    }
    /// The player is The One (`pl+0x731` in TS 2.1). The prediction doesn't know: false.
    fn the_one(&self) -> bool {
        false
    }
    /// The rest of leaving a wall climb for a kung-fu attack (`CCombatAttack` 0x769ec..0x76a88,
    /// after `iuser4 &= !0x20004`): `view_ofs` (0, 0, 16) when the player is ducking, else
    /// (0, 0, 28), and gravity and friction 1. The client copy (`0x1000b4ef`) writes these to its
    /// player too, but only `iuser4` leaves the command, so the client host doesn't need it.
    fn leave_wall_climb(&mut self) {}
    /// Before the gun's virtual `v` runs, with its int argument (`Holster`'s `skiplocal`, else 0;
    /// changes are used): false skips it. The server runs plugin entity hooks here (`ts_server::ham`);
    /// the default runs it.
    fn virtual_pre(&mut self, gun: &mut Gun, v: Virtual, arg: &mut i32) -> bool {
        let _ = (gun, v, arg);
        true
    }
    /// After it, also when [`virtual_pre`](Self::virtual_pre) skipped it (`ran` is then `None`;
    /// a void function returns `Some(0)`): the value the caller goes on with.
    fn virtual_post(&mut self, gun: &mut Gun, v: Virtual, arg: i32, ran: Option<i32>) -> i32 {
        let _ = (gun, v, arg);
        ran.unwrap_or(0)
    }
    /// Port addition (the server plugin API's `melee_attack`, version 5): a server plugin's say
    /// before a kung-fu move lands ([`Gun::ccombat_attack`]). It may change the move's delay
    /// (`time`, seconds) and the damage a hit deals; false blocks the move. The prediction's host
    /// lets every move through unchanged.
    fn melee_attack_pre(&mut self, _gun: &mut Gun, _time: &mut f32, _damage: &mut f32) -> bool {
        true
    }
}

/// Which terms of `CTSGun::GrenadeIdle`'s throw sums a build stores as `float`s before the add
/// (the rest stay on the x87 unrounded), per component ([`Gun::grenade_throw`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GrenadeRounding {
    /// The arithmetic of the roll's sums (`ts_i386.so`'s extended precision or MSVC's 53 bits).
    pub arith: Arith,
    /// A throw's eye, `origin + view_ofs`, before `16·forward` is added.
    pub eye_stored: [bool; 3],
    /// A throw's `speed·forward` before the player's velocity is added.
    pub throw_stored: [bool; 3],
    /// A roll's `speed·2/3`.
    pub roll_speed_stored: bool,
    /// A roll's `speed·2/3·forward` before the velocity is added.
    pub roll_stored: [bool; 3],
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
    /// The game time the recoil penalty last decayed, for rules whose decay runs on the time
    /// elapsed since ([`GunRules::decay_spread`](crate::gameplay::GunRules::decay_spread)).
    /// TS 3.0 decays by the frame's timer step and never reads it.
    pub decay_time: f32,
    /// `gun+0x9fc` / `+0xa00` / `+0xa04`: punch combo, kick combo, last melee move (the kung-fu
    /// move id 10..25 or -1 that chains the kick combos, not the event's anim index).
    pub punch_combo: i32,
    pub kick_combo: i32,
    pub last_melee_anim: i32,
    /// `pl+0x774`: kung-fu power (0..100).
    pub melee_bar: f32,
    /// `pl+0x79c` / `+0x7a0` / `+0x7a4`: delayed melee hit window, its damage and source
    /// (0 kung fu, 1 knife, 2 katana). The window and the block windows below are the rules'
    /// timers: the time left (TS 3.0) or an end time against the game time (TS 2.1); 0 is none
    /// either way, and only the rules read them beyond that.
    pub melee_window: f32,
    pub melee_damage: f32,
    pub melee_source: u8,
    /// `pl+0x7a8` / `+0x7ac` / `+0x7b0`: block windows (kung fu, knife, katana).
    pub block_kungfu: f32,
    pub block_knife: f32,
    pub block_katana: f32,
    /// `pl+0x49a + id`: remembered fire mode + 1.
    pub mode_memory: [u8; MAX_WEAPONS],
    /// The gun entity's own `pev->body`: `Deploy` sets it to the record's body (+0x18), and a
    /// knife or katana slash that hits a player sets it to 1 (0x73a17, `KnifeFire`). The
    /// alternate-grip switch (`SecondaryAttack`) and `WeaponIdle` send it with their view
    /// animation.
    pub body: i32,
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
            decay_time: 0.0,
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
            body: 0,
        }
    }
}

const IN_ATTACK: u32 = 1;
const IN_CANCEL: u32 = 0x40; // TS +alt2: stock attack
const IN_ATTACK2: u32 = 0x800;
const IN_RELOAD: u32 = 0x2000;
const FL_ONGROUND: i32 = 0x200;

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
    pub fn has_slot(&self) -> bool {
        self.current != 0
    }
    /// The active weapon record, mutable (kung fu's is entry 0).
    pub fn cur(&mut self) -> &mut Status {
        &mut self.status[self.current as usize]
    }
    pub fn info(&self) -> &'static WeaponInfo {
        wi(self.current)
    }
    pub fn kind(&self) -> u8 {
        kind_of(self.current)
    }
    pub fn double_fire_rate(host: &(impl GunHost + ?Sized)) -> bool {
        host.powerups() & powerup::DOUBLE_FIRERATE != 0
    }
    pub fn infinite_clip(host: &(impl GunHost + ?Sized)) -> bool {
        host.powerups() & powerup::INFINITE_CLIP != 0
    }
    /// "silenced" = silencer owned and not built in.
    pub fn silenced(&self) -> bool {
        self.has_slot() && self.status[self.current as usize].items & att::SILENCER != 0 && self.info().att_default & 1 == 0
    }
    /// Current sequence table entry `i` (table B when the alternate grip is on).
    pub fn seq(&self, i: usize) -> i32 {
        let alt = self.slot().map(|s| s.alt_grip).unwrap_or(false);
        anim(self.current, alt, i)
    }
    /// `pev->velocity.Length() > limit` (`StandardFire`'s Barrett and M60 gates): the squares `x, y,
    /// z`, the root unrounded in mp.dll and client.dll (0x10055b11..0x10055b3c, 0x100092e1), stored
    /// in `ts_i386.so` (0x8257b..0x825a3). A NaN is not faster.
    pub fn faster_than(host: &(impl GunHost + ?Sized), limit: f32) -> bool {
        crate::by_arith!(host.arith(), speed_over(host.velocity(), limit))
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

    /// Lowers every weapon timer by `dt` (`CBasePlayer::PostThink` 0x91610 tail). `dt` is the
    /// step as the build subtracts it: each timer is `(float)(t - dt)`, which for a `float` step
    /// is the plain `float` subtraction; some builds subtract an unrounded `double`
    /// ([`GunRules::timer_step`](crate::gameplay::GunRules::timer_step)).
    pub fn decrement_timers(&mut self, dt: f64) {
        let sub = |t: f32| (t as f64 - dt) as f32;
        self.next_primary = sub(self.next_primary).max(-1.0);
        self.next_secondary = sub(self.next_secondary).max(-0.001);
        self.next_alt = sub(self.next_alt).max(-0.001);
        if self.idle != 1000.0 {
            self.idle = sub(self.idle).max(-0.001);
        }
        if self.fuser1 != 1000.0 {
            self.fuser1 = sub(self.fuser1).max(-0.001);
        }
        self.next_attack = sub(self.next_attack);
        if self.next_attack < -0.001 {
            self.next_attack = -0.001;
        }
    }

    /// Weapon timer step: `frametime * slowmul * (double fire rate ? 1.5 : 1)` in `arith`'s
    /// rounding ([`spread::timer_step`]). `slowmul` is `GiveSlowMul()`
    /// ([`spread::fixed_slow_amt`]).
    pub fn timer_dt(arith: Arith, frametime: f32, slowmul: f32, powerups: u16) -> f32 {
        spread::timer_step(arith, frametime, slowmul, powerups & powerup::DOUBLE_FIRERATE != 0)
    }

    /// `CBasePlayer::UpdateMeleeBar(cost)` 0x9650c.
    pub fn update_melee_bar(&mut self, cost: f32, host: &mut (impl GunHost + ?Sized)) {
        self.charge_melee_bar(cost);
        host.melee_bar_changed(self.melee_bar);
    }

    /// The state part of `UpdateMeleeBar(cost)` (0x9650c), without the `KFuPower` message: an
    /// empty bar (below 0.01, or NaN: TS tests `0.01 <= v`) becomes 0 with a 1.5 s
    /// `m_flNextAttack`.
    pub fn charge_melee_bar(&mut self, cost: f32) {
        let v = self.melee_bar - cost;
        if v >= 0.01 {
            self.melee_bar = v;
        } else {
            self.melee_bar = 0.0;
            self.next_attack = 1.5;
        }
    }

    // ---- frame entry --------------------------------------------------------------------------

    /// `CBasePlayer::ItemPostFrame` 0xdc4d0 + `CBasePlayerWeapon::ItemPostFrame` 0xa03d4.
    /// Returns true if the weapon frame ran (the server then rerolls the seed).
    pub fn item_post_frame(&mut self, host: &mut impl GunHost) -> bool {
        if !self.post_frame_ready(host.buttons(), host.rules()) {
            return false;
        }
        self.active_item_post_frame(host);
        true
    }

    /// The gate of `CBasePlayer::ItemPostFrame` 0xdc4d0 (the tank check aside): `m_flNextAttack`
    /// has run out, there's no weapon, or +cancel interrupts a reload (for the weapons `rules`
    /// let it, [`GunRules::cancel_runs_frame_in_reload`]). TS runs `ImpulseCommands` (and with
    /// it `PlayerUse`) and then the weapon frame only when this holds.
    pub fn post_frame_ready(&self, buttons: u32, rules: &dyn GunRules) -> bool {
        self.next_attack <= 0.0
            || !self.has_slot()
            || (buttons & IN_CANCEL != 0
                && self.status[self.current as usize].in_reload == 1
                && rules.cancel_runs_frame_in_reload(self.info()))
    }

    /// `CBasePlayerWeapon::ItemPostFrame` 0xa03d4: the weapon frame, once
    /// [`post_frame_ready`](Self::post_frame_ready) passed.
    pub fn weapon_post_frame(&mut self, host: &mut impl GunHost) {
        let buttons = host.buttons();
        // What the client's copy runs (all of it in TS 3.0, `GunRules::predicted`).
        let predicted = if host.prediction() { host.rules().predicted() } else { Predicted::ALL };
        if !self.has_slot() {
            // Kung fu goes to `WeaponIdle` without asking `ShouldWeaponIdle` (0xa04b0..0xa0538).
            if buttons & IN_ATTACK2 != 0 && self.next_secondary <= 0.0 {
                self.virt(host, Virtual::SecondaryAttack, 0, Self::secondary_attack);
            } else if buttons & IN_ATTACK != 0 && self.next_primary <= 0.0 {
                self.virt(host, Virtual::PrimaryAttack, 0, Self::primary_attack);
            } else if buttons & IN_RELOAD != 0 && predicted.kungfu_reload {
                self.virt(host, Virtual::Reload, 0, Self::reload);
            }
            self.virt(host, Virtual::WeaponIdle, 0, Self::weapon_idle);
            return;
        }
        // Finish a magazine reload.
        if self.status[self.current as usize].in_reload != 0 && self.next_attack <= 0.0 && predicted.reload_finish {
            let info = self.info();
            let ammo = (info.ammo as usize).min(MAX_AMMO - 1);
            let s = self.status[self.current as usize];
            // C int arithmetic throughout (wrapping): the client predicts on networked values.
            let j = info.clip.wrapping_sub(s.clip).min(self.reserve[ammo]);
            let s = self.cur();
            s.clip = s.clip.wrapping_add(j);
            s.left = s.clip / 2;
            s.right = s.clip.wrapping_sub(s.left);
            let (clip, left) = (s.clip, s.left);
            if kind_of(self.current) == kind::AKIMBO {
                if info.single >= 0 {
                    self.status[info.single as usize].clip = left;
                }
            } else if info.akimbo != -1 {
                let tw = &mut self.status[info.akimbo as usize];
                tw.clip = tw.clip.wrapping_add(j);
                tw.left = clip;
            }
            self.clip_mirror = clip;
            self.reserve[ammo] = self.reserve[ammo].wrapping_sub(j);
            self.cur().in_reload = 0;
        }
        let contender_empty = self.current == id::CONTENDER && self.status[self.current as usize].clip == 0;
        let acted = if buttons & IN_ATTACK2 != 0 && self.next_secondary <= 0.0 {
            self.virt(host, Virtual::SecondaryAttack, 0, Self::secondary_attack);
            true
        } else if buttons & IN_ATTACK != 0 && self.next_primary <= 0.0 && !contender_empty {
            self.virt(host, Virtual::PrimaryAttack, 0, Self::primary_attack);
            true
        } else if buttons & IN_CANCEL != 0 && self.next_alt <= 0.0 && predicted.alt_attack {
            self.virt(host, Virtual::AlternateAttack, 0, Self::alternate_attack);
            true
        } else if buttons & IN_RELOAD != 0
            && self.has_slot()
            && self.status[self.current as usize].in_reload == 0
            && self.current != id::CONTENDER
        {
            self.virt(host, Virtual::Reload, 0, Self::reload);
            true
        } else {
            false
        };
        // After an attack or reload, or with an attack button held, `ShouldWeaponIdle` decides
        // (0xa07a0); otherwise straight to `WeaponIdle` (0xa07b4).
        if (acted || buttons & (IN_ATTACK | IN_ATTACK2) != 0) && self.virt(host, Virtual::ShouldWeaponIdle, 1, |_, _| ()) == 0 {
            return;
        }
        self.virt(host, Virtual::WeaponIdle, 0, Self::weapon_idle);
    }

    /// `m_pActiveItem->ItemPostFrame()` (`CBasePlayer::ItemPostFrame` 0xdc4d0) on the gun:
    /// [`weapon_post_frame`](Self::weapon_post_frame) as a hookable virtual call.
    pub fn active_item_post_frame(&mut self, host: &mut impl GunHost) {
        self.virt(host, Virtual::PostFrame, 0, Self::weapon_post_frame);
    }

    /// `m_pActiveItem->Holster(skiplocal)` (`CTSGun::Holster` 0x85674, which is
    /// [`holster_weapon`](Self::holster_weapon)) as a hookable virtual call.
    pub fn holster(&mut self, host: &mut (impl GunHost + ?Sized)) {
        let mut arg = 0;
        let ran = host.virtual_pre(self, Virtual::Holster, &mut arg).then(|| {
            self.holster_weapon(host);
            0
        });
        host.virtual_post(self, Virtual::Holster, arg, ran);
    }

    /// Runs virtual `v` on request (a server plugin's `ham_execute`): with `hooked`, through the
    /// host's [`virtual_pre`](GunHost::virtual_pre) / [`virtual_post`](GunHost::virtual_post) as
    /// when the gun calls it, else plainly. Returns its value (1 for `Deploy` and
    /// `ShouldWeaponIdle`, else 0).
    pub fn run_virtual(&mut self, host: &mut impl GunHost, v: Virtual, hooked: bool) -> i32 {
        fn plain<H: GunHost>(g: &mut Gun, h: &mut H, v: Virtual) -> i32 {
            match v {
                Virtual::Deploy => {
                    g.deploy_weapon(h);
                    return 1;
                }
                Virtual::Holster => g.holster_weapon(h),
                Virtual::PostFrame => g.weapon_post_frame(h),
                Virtual::PrimaryAttack => g.primary_attack(h),
                Virtual::SecondaryAttack => g.secondary_attack(h),
                Virtual::AlternateAttack => g.alternate_attack(h),
                Virtual::Reload => g.reload(h),
                Virtual::WeaponIdle => g.weapon_idle(h),
                Virtual::ShouldWeaponIdle => return 1,
            }
            0
        }
        if !hooked {
            return plain(self, host, v);
        }
        let mut arg = 0;
        let ran = host.virtual_pre(self, v, &mut arg).then(|| plain(self, host, v));
        host.virtual_post(self, v, arg, ran)
    }

    /// One of the gun's hookable virtual calls ([`Virtual`]): `f` runs unless the host's
    /// [`virtual_pre`](GunHost::virtual_pre) skips it. `ret` is what `f` returns (`ShouldWeaponIdle`'s
    /// 1, `Deploy`'s 1; 0 for void functions); the result is the host's.
    fn virt<H: GunHost>(&mut self, host: &mut H, v: Virtual, ret: i32, f: impl FnOnce(&mut Self, &mut H)) -> i32 {
        let mut arg = 0;
        let ran = host.virtual_pre(self, v, &mut arg).then(|| {
            f(self, host);
            ret
        });
        host.virtual_post(self, v, arg, ran)
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
                if host.rules().blades_need_press() && host.pressed() & IN_ATTACK == 0 {
                    return;
                }
                self.cur().toggle = false;
                self.knife_fire(host);
                return;
            }
            kind::KATANA => {
                if host.rules().blades_need_press() && host.pressed() & IN_ATTACK == 0 {
                    return;
                }
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
                if host.rules().blades_need_press() && host.pressed() & IN_ATTACK2 == 0 {
                    return;
                }
                self.cur().toggle = true;
                self.knife_fire(host);
                return;
            }
            kind::KATANA if host.rules().katana_throws() => {
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
                self.cur().alt_grip = !s.alt_grip;
                host.weapon_anim(a, true, self.body);
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
    pub fn empty_sound(&mut self, host: &mut (impl GunHost + ?Sized)) {
        if self.current != id::CONTENDER {
            host.event(Event::Empty, EventArgs::default());
            self.next_secondary = 0.1;
            self.next_primary = 0.1;
            self.idle = 0.5;
        }
    }

    /// `CTSGun::Fire` 0x81c78, as the active rules have it ([`GunRules::fire`]).
    pub fn fire(&mut self, host: &mut impl GunHost) {
        host.rules().fire(self, host);
    }

    /// One `ShootBulletsPlayer` call for the current weapon ([`GunHost::shoot`]): its pellets,
    /// bullet type and ammo with this spread, damage, seed and offset along `v_right`.
    pub fn shoot(&mut self, host: &mut (impl GunHost + ?Sized), spread: f32, damage: f32, seed: u32, right_offset: f32) {
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
        host.shoot(&shot, self);
    }

    /// `CTSGun::ColdCock` 0x81dd0: the stock hit, as the rules run it
    /// ([`MeleeRules::cold_cock`](crate::gameplay::MeleeRules::cold_cock)).
    pub fn cold_cock(&mut self, host: &mut impl GunHost) {
        host.rules().cold_cock(self, host);
    }

    // ---- grenades, blades -----------------------------------------------------------------------

    /// `CTSGun::GrenadeFire` 0x832f0: pull the pin. The throw happens in `grenade_idle`.
    pub fn grenade_fire(&mut self, host: &mut impl GunHost) {
        let s = self.status[self.current as usize];
        if !self.has_slot() || s.state != 0 || s.clip <= 0 {
            return;
        }
        host.weapon_anim(self.seq(19), true, self.body);
        host.event(Event::Grenade, EventArgs { i1: 0x13, ..Default::default() });
        self.cur().state = 1;
        self.next_secondary = 0.7;
        self.next_primary = 0.7;
        self.idle = 0.7;
    }

    /// Throw origin and velocity for the M61 (`GrenadeIdle` 0x84f18, mp.dll 0x10057d80): the
    /// pitch is remapped to `p * (p >= 0 ? 10/9 : 8/9) - 10`, speed `min(750, (90 - p') * 7)`; a
    /// roll starts at the feet with 2/3 speed. The factors 10/9, 8/9, -10 and 2/3 are `double`s
    /// on the x87 in every build (issue #129), so the pitch and speed are each rounded once.
    /// Which of the vector sums' terms are stored as `float`s before the add is the build's
    /// ([`GrenadeRounding`], `GunRules::grenade_rounding`).
    pub fn grenade_throw(r: &GrenadeRounding, origin: Vec3, view_ofs: Vec3, velocity: Vec3, v_angle: Vec3, roll: bool) -> (Vec3, Vec3) {
        let p = v_angle[0] as f64;
        // `fcom 0; test ah,1`: a NaN takes the 8/9 branch.
        let pitch = (p * if p >= 0.0 { 10.0 / 9.0 } else { 8.0 / 9.0 } - 10.0) as f32;
        let s = ((90.0 - pitch as f64) * 7.0) as f32;
        let speed = if s > 750.0 { 750.0 } else { s };
        let (f, _, _) = spread::angle_vectors([pitch, v_angle[1], v_angle[2]]);
        let d = |v: f32| v as f64;
        if !roll {
            // `16·forward` is exact; the eye `origin + view_ofs` is what the builds round apart.
            let eye = |i: usize| d(origin[i]) + d(view_ofs[i]);
            let eye = |i: usize| if r.eye_stored[i] { d(eye(i) as f32) } else { eye(i) };
            let src = [0, 1, 2].map(|i| (eye(i) + d(f[i] * 16.0)) as f32);
            let vel = [0, 1, 2].map(|i| {
                let p = d(speed) * d(f[i]);
                let p = if r.throw_stored[i] { d(p as f32) } else { p };
                (p + d(velocity[i])) as f32
            });
            (src, vel)
        } else {
            let src = spread::ma(origin, 16.0, f);
            fn go<F: Fpu>(r: &GrenadeRounding, speed: f32, f: Vec3, velocity: Vec3) -> Vec3 {
                let k = F::x(speed) * F::xd(2.0 / 3.0);
                let k = if r.roll_speed_stored { F::x(k.f()) } else { k };
                [0, 1, 2].map(|i| {
                    let p = k * F::x(f[i]);
                    let p = if r.roll_stored[i] { F::x(p.f()) } else { p };
                    (p + F::x(velocity[i])).f()
                })
            }
            (src, crate::by_arith!(r.arith, go(r, speed, f, velocity)))
        }
    }

    /// `CTSGun::GrenadeIdle(force)` 0x84f18: throws when the button is released (or `force`).
    pub fn grenade_idle(&mut self, force: bool, host: &mut (impl GunHost + ?Sized)) {
        if !self.has_slot() {
            return;
        }
        let mut done = false;
        let s = self.status[self.current as usize];
        if s.state == 1 {
            if host.buttons() & (IN_ATTACK | IN_ATTACK2) != 0 && !force {
                return;
            }
            let rounding = host.rules().grenade_rounding(host.arith());
            let (src, vel) = Self::grenade_throw(&rounding, host.origin(), host.view_ofs(), host.velocity(), host.v_angle(), s.toggle);
            if self.info().bullet == 4 {
                host.throw_grenade(src, vel, s.toggle);
            }
            host.weapon_anim(self.seq(if s.toggle { 21 } else { 20 }), true, self.body);
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
            st.clip = st.clip.wrapping_sub(1);
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
                host.weapon_anim(self.seq(2), true, self.body);
                self.idle = 2.0;
            }
        }
    }

    /// `CTSGun::KnifeFire` 0x84044: the knife's slash or throw, as the rules run it
    /// ([`MeleeRules::knife_fire`](crate::gameplay::MeleeRules::knife_fire)).
    pub fn knife_fire(&mut self, host: &mut impl GunHost) {
        host.rules().knife_fire(self, host);
    }

    /// `CTSGun::KatanaFire` 0x833f0: a katana swing, as the rules run it
    /// ([`MeleeRules::katana_fire`](crate::gameplay::MeleeRules::katana_fire)).
    pub fn katana_fire(&mut self, host: &mut impl GunHost) {
        host.rules().katana_fire(self, host);
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

    /// `CTSGun::CCombatAttack` 0x86078: a kung-fu punch or kick, as the rules run it
    /// ([`MeleeRules::ccombat_attack`](crate::gameplay::MeleeRules::ccombat_attack)).
    pub fn ccombat_attack(&mut self, host: &mut impl GunHost) {
        host.rules().ccombat_attack(self, host);
    }

    /// `CTSGun::CCombatBlock` 0x85ebc: the reload key with kung fu or a blade, as the rules run it
    /// ([`MeleeRules::ccombat_block`](crate::gameplay::MeleeRules::ccombat_block)).
    pub fn ccombat_block(&mut self, host: &mut impl GunHost) {
        host.rules().ccombat_block(self, host);
    }

    /// `CTSGun::CCombatIdle` 0x85e54.
    pub fn ccombat_idle(&mut self, host: &mut impl GunHost) {
        if self.idle <= 0.0 {
            self.idle = 3.0;
            host.weapon_anim(anim(0, false, 0), false, self.body);
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
        let j = clip_size.wrapping_sub(self.status[self.current as usize].clip).min(self.current_reserve());
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
            self.virt(host, Virtual::Reload, 0, Self::reload);
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
        } else if self.status[self.current as usize].clip < 1
            && !(host.rules().infinite_clip_skips_idle_reload() && Self::infinite_clip(host))
        {
            self.virt(host, Virtual::Reload, 0, Self::reload);
        } else if self.status[self.current as usize].clip != 0 {
            self.idle = 3.0;
            host.weapon_anim(self.seq(0), false, self.body);
        }
    }

    // ---- zoom, modes, attachments ---------------------------------------------------------------

    /// `CTSGun::Zoom(fov)` 0x81710: FOV below 40 hides the view model.
    pub fn zoom(&mut self, fov: i32, host: &mut (impl GunHost + ?Sized)) {
        host.set_fov(fov as f32);
        if self.info().zoom[0] < 40 {
            host.set_view_model(None);
        }
    }

    /// `CTSGun::Unzoom` 0x81748.
    pub fn unzoom(&mut self, host: &mut (impl GunHost + ?Sized)) {
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
        self.mode_memory[self.current as usize] = self.status[self.current as usize].mode.wrapping_add(1);
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
    pub fn select_weapon(&mut self, id: u8, host: &mut (impl GunHost + ?Sized)) -> bool {
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
    pub fn switch_to(&mut self, id: u8, host: &mut (impl GunHost + ?Sized)) -> bool {
        if self.select_weapon(id, host) {
            self.deploy(host);
            true
        } else {
            false
        }
    }

    /// `m_pActiveItem->Deploy()` / `this->Deploy()`: [`deploy_weapon`](Self::deploy_weapon) as a
    /// hookable virtual call. Every TS caller goes through the vtable.
    pub fn deploy(&mut self, host: &mut (impl GunHost + ?Sized)) {
        let mut arg = 0;
        let ran = host.virtual_pre(self, Virtual::Deploy, &mut arg).then(|| {
            self.deploy_weapon(host);
            1
        });
        host.virtual_post(self, Virtual::Deploy, arg, ran);
    }

    /// `CTSGun::Deploy` 0x81030.
    pub fn deploy_weapon(&mut self, host: &mut (impl GunHost + ?Sized)) {
        if !self.has_slot() {
            self.holster_weapon(host);
            return;
        }
        host.deploy_update_now();
        let id = self.current;
        let items = self.status[id as usize].items;
        let body = body_by_items(id, items);
        {
            let st = self.cur();
            st.body = body;
            st.state = 0;
            st.alt_grip = false;
        }
        self.body = body;
        let draw = anim(id, false, 2);
        // CBasePlayerWeapon::DefaultDeploy(v, p, anim, ext, skiplocal 0, body)
        host.set_view_model(Some(id));
        host.set_weapon_model(Some((id, items & att::SILENCER != 0)));
        host.set_anim_extension(tables().asset(id).map(|a| a.anim_ext.as_str()).unwrap_or(""));
        host.weapon_anim(draw, false, body);
        (self.next_attack, self.idle) = host.default_deploy(id);
        host.event(Event::Draw, EventArgs { i1: id as i32, i2: items as i32, ..Default::default() });
        host.player_anim(0x34);
        let mem = &mut self.mode_memory[id as usize];
        if *mem < 1 {
            *mem = self.status[id as usize].mode.wrapping_add(1);
        } else {
            self.status[id as usize].mode = *mem - 1;
        }
    }

    /// `CTSGun::HolsterWeapon` 0x85698: back to kung fu (whose view the rules decide,
    /// [`GunRules::holster_draws_kungfu`](crate::gameplay::GunRules::holster_draws_kungfu)).
    pub fn holster_weapon(&mut self, host: &mut (impl GunHost + ?Sized)) {
        let draws_kungfu = host.rules().holster_draws_kungfu();
        host.set_view_model(None);
        host.set_weapon_model(None);
        // pl+0xb58 / pl+0x478 = "" (no weapon animation extension).
        host.set_anim_extension("");
        if draws_kungfu {
            self.unzoom(host);
        } else {
            host.set_fov(0.0);
        }
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
        if draws_kungfu {
            host.set_view_model(Some(0));
            host.weapon_anim(anim(0, false, 2), false, self.body);
            host.event(Event::Draw, EventArgs::default());
        }
        self.kick_combo = 0;
        self.punch_combo = 0;
        self.last_melee_anim = 0;
    }

    /// `CTSGun::GetBetterWeapon(excluded)` 0x80f40: switch to the weapon the rules pick
    /// ([`GunRules::better_weapon`](crate::gameplay::GunRules::better_weapon)).
    pub fn get_better_weapon(&mut self, excluded: u8, host: &mut (impl GunHost + ?Sized)) {
        let pick = host.rules().better_weapon(self, excluded);
        self.switch_to(pick, host);
    }

    /// `CTSGun::DropWeapon(id, better)` 0x8094c: removes the record. Returns true if owned.
    pub fn remove_weapon(&mut self, id: u8, better: bool, host: &mut (impl GunHost + ?Sized)) -> bool {
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
            self.status[t].clip = self.status[t].clip.wrapping_add(1);
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
                    s.clip = s.clip.wrapping_add(self.status[single].clip);
                    s.left = self.status[single].clip;
                    s.right = s.clip.wrapping_sub(s.left);
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
        self.reserve[a] = self.reserve[a].saturating_add(n).min(max_carry);
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
                    // `(int)(clip * 0.5)`, truncated (0x8144b, 0x814c1, 0x814f0); `clip` is even.
                    let half = (f64::from(clip) * 0.5).trunc_i32();
                    let st = self.cur();
                    st.left = half;
                    st.right = half;
                    if info.single >= 0 && info.single as u8 != self.current {
                        self.status[info.single as usize].clip = half;
                    }
                }
            } else {
                let tw = &mut self.status[info.akimbo as usize];
                // `(int)(clip * 0.5 + pair.clip)`, truncated (0x8153b).
                tw.clip = (f64::from(clip) * 0.5 + f64::from(tw.clip)).trunc_i32();
                tw.left = clip;
            }
            Some(0)
        } else {
            let want = info.pellets.wrapping_mul(2).wrapping_sub(self.status[self.current as usize].clip).wrapping_mul(info.slots);
            let used = want.min(free_slots);
            if info.slots > 0 {
                let s = self.cur();
                s.clip = s.clip.wrapping_add(used / info.slots);
            }
            Some(used)
        }
    }

    /// Clears the whole inventory (`CTSGun::Reset` 0x805e0); timers and the bar stay.
    pub fn reset_inventory(&mut self) {
        self.status = [Status::default(); MAX_WEAPONS];
        self.reserve = [0; MAX_AMMO];
        self.current = 0;
        self.body = 0;
    }

    /// The player fields `CBasePlayer::TSInit` (3.0 0x92a4c, 2.1 0x100552f0) resets at every
    /// spawn: `m_flNextAttack`, the kung-fu bar (100), the recoil penalty, the delayed hit
    /// (window, damage, source) and the block windows. The gun entity's own fields (records,
    /// attack timers, clip mirror, `fuser1`, last shot, combos, body) belong to the gun, which
    /// keeps them through a spawn ([`reset_gun_entity`](Self::reset_gun_entity) is a new one's),
    /// and the fire-mode memory lasts until `InitHUD`.
    pub fn reset_for_spawn(&mut self) {
        let d = Gun::default();
        self.next_attack = d.next_attack;
        self.spread_penalty = d.spread_penalty;
        self.decay_time = d.decay_time;
        self.melee_bar = d.melee_bar;
        self.melee_window = d.melee_window;
        self.melee_damage = d.melee_damage;
        self.melee_source = d.melee_source;
        self.block_kungfu = d.block_kungfu;
        self.block_knife = d.block_knife;
        self.block_katana = d.block_katana;
    }

    /// A new gun entity's own fields (`CTSGun::Spawn` 0x8053c on a zeroed entity: clip mirror
    /// `+0xb4 = 1`, last shot `+0x9e8 = 0`, the attack and idle timers, `fuser1` and the combos
    /// `+0x9fc..+0xa04` zero). The records are [`reset_inventory`](Self::reset_inventory)'s.
    pub fn reset_gun_entity(&mut self) {
        let d = Gun::default();
        self.next_primary = d.next_primary;
        self.next_secondary = d.next_secondary;
        self.next_alt = d.next_alt;
        self.idle = d.idle;
        self.clip_mirror = d.clip_mirror;
        self.fuser1 = d.fuser1;
        self.last_shot = d.last_shot;
        self.punch_combo = d.punch_combo;
        self.kick_combo = d.kick_combo;
        self.last_melee_anim = d.last_melee_anim;
    }
}

/// [`Gun::faster_than`] in one build's arithmetic.
pub fn speed_over<F: Fpu>(v: Vec3, limit: f32) -> bool {
    F::vec_length(v, crate::pm::ORDER_XYZ) > F::x(limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `StandardFire`'s Barrett and M60 gates (issue #171): mp.dll and client.dll compare the unrounded
    /// root, so a speed a hair over the limit counts; `ts_i386.so` stores the root (exactly the
    /// limit), as the port's old `float` length did.
    #[test]
    fn speed_gates_per_build() {
        use crate::pm::{Gcc, Msvc};
        for (v, limit) in [([35.0, 0.0001, 0.0], 35.0), ([175.0, 0.0003, 0.0], 175.0)] {
            assert!(speed_over::<Msvc>(v, limit), "{limit}");
            assert!(!speed_over::<Gcc>(v, limit), "{limit}");
            assert!(!speed_over::<Msvc>([limit, 0.0, 0.0], limit));
        }
        assert!(!speed_over::<Msvc>([f32::NAN, 0.0, 0.0], 35.0));
    }

    // Audit round 2: B2

    /// #208, #247: a spawn resets `TSInit`'s player fields (3.0 0x92a4c) and keeps the gun
    /// entity's own and the fire-mode memory (`InitHUD` 0x79134 clears that); a new gun
    /// (`CTSGun::Spawn` 0x8053c) starts with clip mirror 1 and every timer and combo at 0.
    #[test]
    fn spawn_keeps_the_gun_entity_and_the_mode_memory() {
        let mut g = Gun {
            next_primary: 0.3,
            next_secondary: 0.4,
            next_alt: 0.5,
            idle: 2.0,
            next_attack: 0.7,
            clip_mirror: 9,
            fuser1: 1.5,
            last_shot: 12.0,
            spread_penalty: 0.1,
            punch_combo: 2,
            kick_combo: 1,
            last_melee_anim: 14,
            melee_bar: 20.0,
            melee_window: 0.2,
            melee_damage: 30.0,
            melee_source: 2,
            block_kungfu: 0.1,
            block_knife: 0.2,
            block_katana: 0.3,
            body: 3,
            ..Gun::default()
        };
        g.mode_memory[5] = 2;
        g.reset_for_spawn();
        let d = Gun::default();
        assert_eq!((g.next_attack, g.spread_penalty, g.melee_bar), (0.0, 0.0, 100.0));
        assert_eq!((g.melee_window, g.melee_damage, g.melee_source), (0.0, 0.0, 0));
        assert_eq!((g.block_kungfu, g.block_knife, g.block_katana), (0.0, 0.0, 0.0));
        assert_eq!((g.next_primary, g.next_secondary, g.next_alt, g.idle), (0.3, 0.4, 0.5, 2.0));
        assert_eq!((g.clip_mirror, g.fuser1, g.last_shot, g.body), (9, 1.5, 12.0, 3));
        assert_eq!((g.punch_combo, g.kick_combo, g.last_melee_anim), (2, 1, 14));
        assert_eq!(g.mode_memory[5], 2, "only InitHUD clears it");
        g.reset_gun_entity();
        assert_eq!((g.next_primary, g.next_secondary, g.next_alt, g.idle), (0.0, 0.0, 0.0, 0.0));
        assert_eq!((g.clip_mirror, g.fuser1, g.last_shot), (1, 0.0, 0.0));
        assert_eq!((g.punch_combo, g.kick_combo, g.last_melee_anim), (d.punch_combo, d.kick_combo, d.last_melee_anim));
        assert_eq!(g.mode_memory[5], 2);
    }
}
