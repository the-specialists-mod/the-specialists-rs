//! [`GunRules`]: the weapon hooks (`crate::weapons::gun`).
//!
//! The server's gun and the client's weapon prediction run the same code and must agree, so each
//! resolves the rules once per command from the player's physinfo key
//! ([`PHYSINFO_KEY`](super::PHYSINFO_KEY)) and hands them to the gun through
//! [`GunHost::rules`](crate::weapons::GunHost::rules).

use crate::pm::Arith;
use crate::weapons::gun::{Gun, GunHost};
use crate::weapons::spread;
use crate::weapons::table::{Tables, WeaponInfo};

/// Gun rules: the gun code calls these where a ruleset can differ. The melee attacks are in the
/// supertrait [`MeleeRules`].
pub trait GunRules: MeleeRules + Sync {
    /// The ruleset's weapon, ammo, asset, sequence and combat tables. Both sides make them the
    /// tables `weapons::tables()` returns while the ruleset is active
    /// ([`set_ruleset_tables`](crate::weapons::table::set_ruleset_tables)), so everything that
    /// reads weapon data (the gun, inventory, buying, the HUD, damage) follows the ruleset.
    fn tables(&self) -> &'static Tables;

    /// `CTSGun::Fire` and the shot it fires: the standard, shotgun or akimbo shot, with its spread,
    /// recoil, ammo use and next-attack delays. Called by [`Gun::fire`](crate::weapons::Gun::fire)
    /// for every trigger pull and burst round.
    fn fire(&self, gun: &mut Gun, host: &mut dyn GunHost);

    /// `CTSGun::StandardFire`: the delay before the next primary shot of a gun in semi-auto mode
    /// that can also fire full auto. `rate` is the weapon's own fire rate (`info.rate`), which full
    /// auto uses.
    fn semi_auto_delay(&self, rate: f32) -> f32;

    /// The step the weapon timers run down by in one player think or predicted command
    /// ([`Gun::decrement_timers`](crate::weapons::Gun::decrement_timers)). `frametime` is the
    /// engine's (`gpGlobals->frametime`) or [`command_frame_time`](Self::command_frame_time);
    /// `fuser1` the player's time scale, `powerups` the player's power-ups.
    fn timer_step(&self, arith: Arith, frametime: f64, fuser1: f32, powerups: u16) -> f64;

    /// The frame time of one predicted command of `msec` milliseconds, as the client's
    /// `HUD_PostRunCmd` computes it.
    fn command_frame_time(&self, msec: u8) -> f64;

    /// The recoil penalty's decay in one think ([`SpreadDecay`]): the server's `PreThink`, and the
    /// client's `HUD_WeaponsPostThink` on a first prediction.
    ///
    /// Returns whether the client's crosshair copy of the penalty takes the gun's penalty at the
    /// end of this prediction (the server ignores it).
    fn decay_spread(&self, gun: &mut Gun, d: &SpreadDecay) -> bool;

    /// The offset of pellet `i` (1-based) of a shot with random seed `seed`, in units of its
    /// spread, `(right, up)`, for the server's `ShootBulletsPlayer` and the client fire event's
    /// re-trace (`EV_TS_FireBullets`). `None` is the polar pattern, which each side computes as
    /// its build does ([`spread::polar_offset`] of each side's random draws). `bot_random` is the
    /// engine's `RandomFloat(lo, hi)` when the server fires for a bot, for rules that don't seed a
    /// bot's pellets.
    fn pellet_offset(&self, seed: u32, i: u32, bot_random: Option<&mut dyn FnMut(f32, f32) -> f32>) -> Option<(f32, f32)>;

    /// The direction of a pellet at offset `offset` from the aim `aim` (not normalised), for the
    /// server's `ShootBulletsPlayer` or the client fire event (`side`).
    fn pellet_dir(
        &self,
        side: spread::PelletSide,
        aim: spread::Vec3,
        right: spread::Vec3,
        up: spread::Vec3,
        spread: [f32; 2],
        offset: spread::PelletOffset,
    ) -> spread::Vec3;

    /// The client fire event's akimbo bullet origins (`EV_TS_FireAkimbo`), units along `v_right`
    /// from the gun position: the first traced with the event's seed, the second (linked shots
    /// only) with seed + 0x21. `toggle` is the event's `bparam1`, the gun that fired.
    fn akimbo_event_offsets(&self, linked: bool, toggle: bool) -> (f32, Option<f32>);

    /// Which parts of `ItemPostFrame` the client's weapon prediction runs.
    fn predicted(&self) -> Predicted;

    /// Whether +cancel (+alt2) lets the weapon frame run during a magazine reload of weapon
    /// `info` (`CBasePlayer::ItemPostFrame`).
    fn cancel_runs_frame_in_reload(&self, info: &WeaponInfo) -> bool;

    /// `CTSGun::GetBetterWeapon(excluded)`: the weapon to draw instead (0 for kung fu), for a gun
    /// that loses its current weapon or a bot that wants another.
    fn better_weapon(&self, gun: &Gun, excluded: u8) -> u8;

    /// Whether the knife (+attack, +attack2) and the katana (+attack) attack only on the frame
    /// the button goes down (`PrimaryAttack`, `SecondaryAttack`); otherwise while it is held.
    fn blades_need_press(&self) -> bool;

    /// Whether +attack2 throws the katana (`SecondaryAttack` → `KatanaThrow`); otherwise the
    /// katana takes the generic scope / alt-grip path.
    fn katana_throws(&self) -> bool;

    /// Whether Infinite Clip stops `WeaponIdle` reloading an empty gun.
    fn infinite_clip_skips_idle_reload(&self) -> bool;

    /// Whether `HolsterWeapon` unzooms (`Unzoom`, which may put the gun's view model back) and
    /// then draws kung fu: weapon 0's view model, its draw animation and the draw event. Otherwise
    /// it only sets the FOV to 0 and leaves no view model.
    fn holster_draws_kungfu(&self) -> bool;
}

/// One think's recoil-penalty decay ([`GunRules::decay_spread`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpreadDecay {
    pub arith: Arith,
    /// The frame's weapon timer step.
    pub step: f32,
    /// The game time of the think (the server's `gpGlobals->time`, the command's time on the
    /// client).
    pub time: f32,
    /// The time scale: the server player's `pev->fuser1`, the client HUD's slow-motion scale.
    pub time_scale: f32,
    /// The player is alive (the client passes true: its decay runs dead or alive).
    pub alive: bool,
}

/// The parts of `ItemPostFrame` a client prediction runs ([`GunRules::predicted`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Predicted {
    /// Refilling the clip when a magazine reload's `m_flNextAttack` runs out.
    pub reload_finish: bool,
    /// The +alt2 stock hit (`AlternateAttack`).
    pub alt_attack: bool,
    /// The reload key without a weapon (the kung-fu block).
    pub kungfu_reload: bool,
}

impl Predicted {
    /// Everything, as the server runs it.
    pub const ALL: Predicted = Predicted { reload_finish: true, alt_attack: true, kungfu_reload: true };
}

/// Whole melee attacks, which a ruleset runs its own way (issue #100 §1.0: hook a whole function
/// where a ruleset runs a different algorithm). The gun's [`Gun`] methods of the same names call
/// these.
pub trait MeleeRules {
    /// `CTSGun::KnifeFire`: the knife's slash or throw.
    fn knife_fire(&self, gun: &mut Gun, host: &mut dyn GunHost);

    /// `CTSGun::KatanaFire`: a katana swing.
    fn katana_fire(&self, gun: &mut Gun, host: &mut dyn GunHost);

    /// `CTSGun::CCombatAttack`: a kung-fu punch or kick.
    fn ccombat_attack(&self, gun: &mut Gun, host: &mut dyn GunHost);

    /// `CTSGun::CCombatBlock`: the reload key with kung fu or a blade.
    fn ccombat_block(&self, gun: &mut Gun, host: &mut dyn GunHost);

    /// `CTSGun::ColdCock`: the stock hit (`+alt2`).
    fn cold_cock(&self, gun: &mut Gun, host: &mut dyn GunHost);
}
