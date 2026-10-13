//! [`GunRules`]: the weapon hooks (`crate::weapons::gun`).
//!
//! The server's gun and the client's weapon prediction run the same code and must agree, so each
//! resolves the rules once per command from the player's physinfo key
//! ([`PHYSINFO_KEY`](super::PHYSINFO_KEY)) and hands them to the gun through
//! [`GunHost::rules`](crate::weapons::GunHost::rules).

use crate::pm::Arith;
use crate::weapons::gun::{Gun, GunHost};
use crate::weapons::spread;
use crate::weapons::table::{Tables, WeaponInfo, base_tables};

/// Gun rules. Every default body is TS 3.0's (`Core`); the gun code calls these where a ruleset
/// can differ. The melee attacks are in the supertrait [`MeleeRules`].
pub trait GunRules: MeleeRules + Sync {
    /// The ruleset's weapon, ammo, asset, sequence and combat tables. Both sides make them the
    /// tables `weapons::tables()` returns while the ruleset is active
    /// ([`set_ruleset_tables`](crate::weapons::table::set_ruleset_tables)), so everything that
    /// reads weapon data (the gun, inventory, buying, the HUD, damage) follows the ruleset.
    fn tables(&self) -> &'static Tables {
        base_tables()
    }

    /// `CTSGun::Fire` (0x81c78) and the shot it fires: the standard, shotgun or akimbo shot, with
    /// its spread, recoil, ammo use and next-attack delays. Called by
    /// [`Gun::fire`](crate::weapons::Gun::fire) for every trigger pull and burst round.
    fn fire(&self, gun: &mut Gun, host: &mut dyn GunHost) {
        gun.fire_core(host);
    }

    /// `CTSGun::StandardFire` (0x824a4): the delay before the next primary shot of a gun in semi-auto
    /// mode that can also fire full auto. `rate` is the weapon's own fire rate (`info.rate`),
    /// which full auto uses.
    fn semi_auto_delay(&self, rate: f32) -> f32 {
        let _ = rate;
        0.1
    }

    /// The step the weapon timers run down by in one player think or predicted command
    /// ([`Gun::decrement_timers`](crate::weapons::Gun::decrement_timers)): TS 3.0's
    /// `frametime * GiveSlowMul() * (double fire rate ? 1.5 : 1)` (`CBasePlayer::PostThink`
    /// 0x91ae7, client `0x10003ec5`). `frametime` is the engine's (`gpGlobals->frametime`) or
    /// [`command_frame_time`](Self::command_frame_time); `fuser1` the player's time scale.
    fn timer_step(&self, arith: Arith, frametime: f64, fuser1: f32, powerups: u16) -> f64 {
        Gun::timer_dt(arith, frametime as f32, spread::fixed_slow_amt(fuser1), powerups) as f64
    }

    /// The frame time of one predicted command of `msec` milliseconds, as the client's
    /// `HUD_PostRunCmd` computes it: TS 3.0's `(float)(msec * 0.001)` with the `double` constant
    /// (`0x10003eb7`), which is also the engine's `frametime` for the server's `PostThink`.
    fn command_frame_time(&self, msec: u8) -> f64 {
        (msec as f64 * 0.001) as f32 as f64
    }

    /// The recoil penalty's decay in one think ([`SpreadDecay`]): TS 3.0's `PreThink` (0x80d88)
    /// while alive, and the client's `HUD_WeaponsPostThink` (`0x100040a5`) on a first prediction,
    /// both `pen -= step * 0.1` by the frame's timer step.
    fn decay_spread(&self, gun: &mut Gun, d: &SpreadDecay) {
        if d.alive {
            gun.decay_spread(d.arith, d.step);
        }
    }

    /// The offset of pellet `i` (1-based) of a shot with random seed `seed`, in units of its
    /// spread, `(right, up)`, for the server's `ShootBulletsPlayer` and the client fire event's
    /// re-trace (`EV_TS_FireBullets`). `None` is TS 3.0's polar pattern, which each side
    /// computes as its build does (the server [`spread::pellet_offset`], the client event in
    /// single precision). `bot_random` is the engine's `RandomFloat(lo, hi)` when the server fires
    /// for a bot, for rules that don't seed a bot's pellets.
    fn pellet_offset(&self, seed: u32, i: u32, bot_random: Option<&mut dyn FnMut(f32, f32) -> f32>) -> Option<(f32, f32)> {
        let _ = (seed, i, bot_random);
        None
    }

    /// The client fire event's akimbo bullet origins (`EV_TS_FireAkimbo`), units along `v_right`
    /// from the gun position: the first traced with the event's seed, the second (linked shots
    /// only) with seed + 0x21. TS 3.0: -5 and +5 for a linked shot, none for one gun. `toggle` is
    /// the event's `bparam1`, the gun that fired.
    fn akimbo_event_offsets(&self, linked: bool, toggle: bool) -> (f32, Option<f32>) {
        let _ = toggle;
        if linked { (-5.0, Some(5.0)) } else { (0.0, None) }
    }

    /// Which parts of `ItemPostFrame` the client's weapon prediction runs. TS 3.0's
    /// (`0x1000b7b0`) runs them all.
    fn predicted(&self) -> Predicted {
        Predicted::ALL
    }

    /// Whether +cancel (+alt2) lets the weapon frame run during a magazine reload of weapon
    /// `info` (`CBasePlayer::ItemPostFrame` 0xdc4d0): in TS 3.0, a weapon with a stock hit.
    fn cancel_runs_frame_in_reload(&self, info: &WeaponInfo) -> bool {
        info.stock != 0
    }
}

/// One think's recoil-penalty decay ([`GunRules::decay_spread`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpreadDecay {
    pub arith: Arith,
    /// The frame's weapon timer step (TS 3.0 decays by it).
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

/// Whole melee attacks a ruleset may run its own way (issue #100 §1.0: hook a whole function
/// where a ruleset runs a different algorithm). Each returns true when it handled the attack;
/// the default returns false and TS 3.0's [`Gun`] method runs.
pub trait MeleeRules {
    /// `CTSGun::KnifeFire` (0x84044): the knife's slash or throw.
    fn knife_fire(&self, gun: &mut Gun, host: &mut dyn GunHost) -> bool {
        let _ = (gun, host);
        false
    }

    /// `CTSGun::KatanaFire` (0x833f0): a katana swing.
    fn katana_fire(&self, gun: &mut Gun, host: &mut dyn GunHost) -> bool {
        let _ = (gun, host);
        false
    }

    /// `CTSGun::CCombatAttack` (0x86078): a kung-fu punch or kick.
    fn ccombat_attack(&self, gun: &mut Gun, host: &mut dyn GunHost) -> bool {
        let _ = (gun, host);
        false
    }

    /// `CTSGun::CCombatBlock` (0x85ebc): the reload key with kung fu or a blade.
    fn ccombat_block(&self, gun: &mut Gun, host: &mut dyn GunHost) -> bool {
        let _ = (gun, host);
        false
    }

    /// `CTSGun::ColdCock` (0x81dd0): the stock hit (`+alt2`).
    fn cold_cock(&self, gun: &mut Gun, host: &mut dyn GunHost) -> bool {
        let _ = (gun, host);
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gameplay::CORE;
    use crate::ts::powerup;

    /// The core plugin's gun hooks are TS 3.0's code, unchanged.
    #[test]
    fn core_gun_hooks_are_ts30() {
        assert!(std::ptr::eq(CORE.tables(), base_tables()));
        for arith in [Arith::Gcc, Arith::Msvc] {
            for (ft, fuser1, pw) in [(0.01f32, 1.0f32, 0u16), (0.001, 0.65, powerup::DOUBLE_FIRERATE), (0.02, 0.0, 0)] {
                let want = Gun::timer_dt(arith, ft, spread::fixed_slow_amt(fuser1), pw);
                assert_eq!(CORE.timer_step(arith, ft as f64, fuser1, pw), want as f64);
            }
        }
        for msec in 0..=255u8 {
            assert_eq!(CORE.command_frame_time(msec), (msec as f64 * 0.001) as f32 as f64);
        }
        assert_eq!(CORE.pellet_offset(5, 1, None), None);
        assert_eq!(CORE.akimbo_event_offsets(true, true), (-5.0, Some(5.0)));
        assert_eq!(CORE.akimbo_event_offsets(false, true), (0.0, None));
        assert_eq!(CORE.predicted(), Predicted::ALL);
        assert_eq!(CORE.semi_auto_delay(0.07), 0.1);
        let ak = base_tables().weapon(crate::weapons::id::AK47).unwrap();
        let m4 = base_tables().weapon(crate::weapons::id::M4).unwrap();
        assert!(!CORE.cancel_runs_frame_in_reload(ak) && CORE.cancel_runs_frame_in_reload(m4));
    }

    /// TS 3.0's decay: by the step, only while alive; the time isn't recorded.
    #[test]
    fn core_decay_is_the_step() {
        let mut g = Gun { spread_penalty: 0.05, ..Gun::default() };
        let mut want = g.clone();
        want.decay_spread(Arith::Msvc, 0.01);
        let d = SpreadDecay { arith: Arith::Msvc, step: 0.01, time: 99.0, time_scale: 0.5, alive: true };
        CORE.decay_spread(&mut g, &d);
        assert_eq!(g, want);
        CORE.decay_spread(&mut g, &SpreadDecay { alive: false, ..d });
        assert_eq!(g, want);
    }
}
