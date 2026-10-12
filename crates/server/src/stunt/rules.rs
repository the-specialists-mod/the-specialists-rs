//! What a gameplay ruleset can change in the server's stunt code (`CTSStunt`): the values
//! ([`StuntRules`], `ServerRules::stunts`), how the stunt timers keep time ([`StuntClock`],
//! `ServerRules::stunt_clock`) and the slot limits ([`can_dive`], `ServerRules::can_dive`).
//! Every default here is TS 3.0's (decomp `server/decomp/CTSStunt.c`).

use super::StuntState;
use crate::player::Player;
use goldsrc::sys::{IN_DUCK, IN_JUMP};
use shared::ts::stunt::{DEAD_LOCK, FREEZE_ANGLES, GO_DUCK, RECOVER, ROLLING};

/// A dive height: `CanDiveHigh`, `CanDiveMedium`, `CanDiveLow` (0x9a494, 0x9a4ec, 0x9a544).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dive {
    High,
    Medium,
    Low,
}

/// `CanDiveHigh/Medium/Low` (0x9a494...): used slots (`81 - TSSpace`) must be below 42/52/62
/// ("40 for high, 50 for medium, 60 for low"); The One (pl+0x7b9, `Player::ts.is_the_one`, set
/// by `SetPlayerTheOne`) ignores the limits. `SlotBonus()` is 0.
pub fn can_dive(p: &Player, dive: Dive) -> bool {
    let limit = match dive {
        Dive::High => 42,
        Dive::Medium => 52,
        Dive::Low => 62,
    };
    p.ts.is_the_one || p.slots_used < limit
}

/// The stunt values a ruleset sets. Seconds are the player's own (slowed) time, given to the
/// [`StuntClock`] as `f64` so a ruleset whose binary used `double` constants gets them exactly;
/// TS 3.0's are `float`s, which these `f64`s convert back to unchanged.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StuntRules {
    /// `Landing` (0x9bbf4): the recovery after rolling out of a landing dive.
    pub dive_roll_out_recover: f64,
    /// `HandleWheel` (0x9b820): the recovery after landing a flip.
    pub flip_recover: f64,
    /// `HandleSlide` (0x9b9f8): the recovery after getting up from prone.
    pub get_up_recover: f64,
    /// `HandleWallJump` (0x9b4fc) / `HandleWallClimb` (0x9b644): the recovery after landing a
    /// wall jump or climb, and whether that landing also sets `RECOVER` (slow input).
    pub wall_landing_recover: f64,
    pub wall_landing_sets_recover: bool,
    /// `GoRoll` (0x9cfe8): the roll's recovery.
    pub roll_recover: f64,
    /// `PostThink`'s super-jump landing: the hard landing's recovery, and the weapon delay
    /// (`m_flNextAttack`) after rolling out of it.
    pub super_jump_landing_recover: f64,
    pub super_jump_roll_attack_delay: f64,
    /// The keys that roll out of a landing dive (`Landing`) or get up from prone
    /// (`HandleSlide`): `IN_JUMP | IN_DUCK | IN_ALT1`.
    pub get_up_buttons: i32,
    /// `HandleStunt` (0x9b000): the stunt key with a wall behind dives off it
    /// (`CheckForWallDive` 0x9ad44, `WALLDIVE`), and from a wall climb (`HandleWallClimb`).
    pub wall_dive: bool,
    /// `HandleStunt`: holding jump climbs a wall even while gliding after a super jump.
    pub wall_climb_while_gliding: bool,
    /// `HandleStunt` closes the gun's close-combat window (pl+0x79c..0x7a4).
    pub stunt_ends_close_combat: bool,
    /// `GoWheel` (0x9c9bc) and `GoRoll` wait for the jump kick's cooldown (`+0x08`).
    pub wheel_and_roll_wait_for_kick: bool,
    /// `GoBulletTime` (0x9a5b4): bullets do ×0.7 for this many seconds (absolute, unscaled
    /// time, `+0x34`); `None` sets no dodge window.
    pub bullet_time_dodge: Option<f32>,
    /// `GoBulletTime`: a weapon delay (`m_flNextAttack`, only when it has run out) in the
    /// player's time; `None` leaves the gun alone.
    pub bullet_time_attack_delay: Option<f64>,
    /// `Play` (0x9a014): the `iuser4` bits cleared when the recovery runs out, and whether that
    /// also ends a `BlockPlayer` lock (`pl+0x4dc`).
    pub recover_end_clears: i32,
    pub recover_end_unblocks: bool,
}

/// TS 3.0's stunt values.
pub const CORE: StuntRules = StuntRules {
    dive_roll_out_recover: 0.5625,
    flip_recover: 0.375,
    get_up_recover: 0.75,
    wall_landing_recover: 0.2625,
    wall_landing_sets_recover: false,
    roll_recover: 0.75,
    super_jump_landing_recover: 0.75,
    super_jump_roll_attack_delay: 0.75,
    get_up_buttons: super::IN_ALT1 | (IN_DUCK | IN_JUMP) as i32,
    wall_dive: true,
    wall_climb_while_gliding: true,
    stunt_ends_close_combat: true,
    wheel_and_roll_wait_for_kick: true,
    bullet_time_dodge: Some(0.55),
    bullet_time_attack_delay: None,
    recover_end_clears: ROLLING | FREEZE_ANGLES | GO_DUCK | RECOVER | DEAD_LOCK,
    recover_end_unblocks: true,
};

/// How the stunt timers keep time: the cooldown (`next_stunt`), the recovery (`recover`), the
/// jump kick's cooldown (`close_attack_cd`), the prone lock (`prone_lock`) and `BlockPlayer`'s
/// lock (`block_timer`). The stunt code only stores what [`start`](Self::start) returns and asks
/// [`running`](Self::running) / [`done`](Self::done), so a ruleset can keep them in another form.
///
/// TS 3.0's ([`RELATIVE`]) holds the seconds left of the player's own time: they count down by
/// `slowmul * frametime` each frame (`CTSStunt::Play` 0x9a014), rest at 0.001 (the recovery,
/// `BlockPlayer`) or 0, and count as over below 0.01.
pub trait StuntClock: Sync {
    /// The value to store for a timer of `secs` seconds of the player's time, started at `now`
    /// while the player's time scale (`pev->fuser1`, 1 = normal) is `time_scale`.
    fn start(&self, now: f32, time_scale: f32, secs: f64) -> f32 {
        let _ = (now, time_scale);
        secs as f32
    }

    /// The timer `t` is still running (stunts wait for it).
    fn running(&self, now: f32, t: f32) -> bool {
        let _ = now;
        t > 0.0
    }

    /// The timer `t` has run out: the recovery and the locks end, a wall landing may stunt again.
    fn done(&self, now: f32, t: f32) -> bool {
        let _ = now;
        t < 0.01
    }

    /// `CTSStunt::Play`'s countdown, `dt` being the frame in the player's time.
    fn tick(&self, s: &mut StuntState, dt: f32) {
        if s.recover != 0.0 {
            s.recover = (s.recover - dt).max(0.001);
        }
        s.next_stunt = (s.next_stunt - dt).max(0.0);
        s.close_attack_cd = (s.close_attack_cd - dt).max(0.0);
        s.prone_lock = (s.prone_lock - dt).max(0.0);
    }

    /// `PreThink`'s countdown of `BlockPlayer`'s lock (`dt` is ×1.5 under double firerate).
    fn tick_block(&self, s: &mut StuntState, dt: f32) {
        if s.block_timer != 0.0 {
            s.block_timer = (s.block_timer - dt).max(0.001);
        }
    }

    /// `NextStunt`'s byte for the cooldown `t`: tenths of a second, `ROUND(t * 10)`.
    fn next_stunt_byte(&self, now: f32, t: f32) -> u8 {
        let _ = now;
        shared::protocol::stunt::encode_next_stunt(t)
    }

    /// `GoWallJump` (0x9a7d4) / `GoWallClimb` (0x9aa1c): the cooldown after a wall stunt with
    /// the chain at `chain`: `max(0.5, chain)`.
    fn wall_chain_cooldown(&self, now: f32, time_scale: f32, chain: f32) -> f32 {
        let _ = (now, time_scale);
        chain.max(0.5)
    }

    /// `HandleStunt` / `GoWheel`: the "coyote time" an air stunt is still allowed after leaving
    /// the ground at `last_ground`: 0.7 s of the player's time (`GiveSlowMul`, at least 0.01).
    fn air_stunt_allowed(&self, now: f32, time_scale: f32, last_ground: f32) -> bool {
        // TS refuses when `0.7 / slowmul < now - last_ground`; the floor keeps a NaN out.
        0.7 / time_scale.max(0.01) >= now - last_ground
    }

    /// A weapon delay (`m_flNextAttack`, a relative timer of the gun) a stunt sets, of `secs`
    /// seconds of the player's time: the gun counts it down in the player's time.
    fn attack_delay(&self, time_scale: f32, secs: f64) -> f32 {
        let _ = time_scale;
        secs as f32
    }
}

/// TS 3.0's [`StuntClock`].
pub struct Relative;

impl StuntClock for Relative {}

pub static RELATIVE: Relative = Relative;

#[cfg(test)]
mod tests {
    use super::*;

    /// The `f64` values convert back to the `float`s TS 3.0 (and the port before the hooks)
    /// used.
    #[test]
    fn core_values_are_the_floats() {
        let c = CORE;
        assert_eq!(c.dive_roll_out_recover as f32, 0.5625f32);
        assert_eq!(c.flip_recover as f32, 0.375f32);
        assert_eq!(c.get_up_recover as f32, 0.75f32);
        assert_eq!(c.wall_landing_recover as f32, 0.2625f32);
        assert_eq!(c.roll_recover as f32, 0.75f32);
        assert_eq!(c.super_jump_landing_recover as f32, 0.75f32);
        assert_eq!(c.super_jump_roll_attack_delay as f32, 0.75f32);
        assert_eq!(0.55f64 as f32, 0.55f32);
        assert_eq!(0.7f64 as f32, 0.7f32);
        assert_eq!(c.get_up_buttons, 0x4006);
        assert_eq!(c.recover_end_clears, 0x40000 | 0x80000 | 0x4 | 0x1 | 0x2);
        assert!(c.wall_dive && c.wall_climb_while_gliding && c.stunt_ends_close_combat && c.wheel_and_roll_wait_for_kick);
        assert!(!c.wall_landing_sets_recover && c.recover_end_unblocks);
        assert_eq!((c.bullet_time_dodge, c.bullet_time_attack_delay), (Some(0.55), None));
    }

    /// The 3.0 clock is the relative countdown the stunt code had before the hooks.
    #[test]
    fn relative_clock() {
        let c = &RELATIVE;
        assert_eq!(c.start(100.0, 0.35, 0.75), 0.75);
        assert!(c.running(100.0, 0.001) && !c.running(100.0, 0.0));
        assert!(c.done(100.0, 0.001) && !c.done(100.0, 0.01));
        let mut s = StuntState { recover: 0.1, next_stunt: 0.05, close_attack_cd: 0.2, prone_lock: 0.0, ..StuntState::default() };
        c.tick(&mut s, 0.1);
        assert_eq!((s.recover, s.next_stunt, s.close_attack_cd, s.prone_lock), (0.001, 0.0, 0.1, 0.0));
        s.block_timer = 0.0;
        c.tick_block(&mut s, 0.1);
        assert_eq!(s.block_timer, 0.0, "a lock that isn't set stays unset");
        s.block_timer = 0.05;
        c.tick_block(&mut s, 0.1);
        assert_eq!(s.block_timer, 0.001);
        assert_eq!(c.next_stunt_byte(100.0, 0.55), 6);
        assert_eq!(c.wall_chain_cooldown(0.0, 1.0, 0.35), 0.5);
        assert_eq!(c.wall_chain_cooldown(0.0, 1.0, 30.0), 30.0, "no cap");
        assert!(c.air_stunt_allowed(10.0, 1.0, 9.4) && !c.air_stunt_allowed(10.0, 1.0, 9.2));
        // Slowed to 0.5, the window is 1.4 s of real time.
        assert!(c.air_stunt_allowed(10.0, 0.5, 8.7));
        assert_eq!(c.attack_delay(0.35, 0.75), 0.75);
    }

    #[test]
    fn can_dive_limits_and_the_one() {
        let mut p = Player { slots_used: 60, ..Player::default() };
        assert!(!can_dive(&p, Dive::High) && !can_dive(&p, Dive::Medium) && can_dive(&p, Dive::Low));
        p.slots_used = 41;
        assert!(can_dive(&p, Dive::High));
        p.slots_used = 81;
        p.ts.is_the_one = true;
        assert!(can_dive(&p, Dive::High) && can_dive(&p, Dive::Medium) && can_dive(&p, Dive::Low));
    }
}
