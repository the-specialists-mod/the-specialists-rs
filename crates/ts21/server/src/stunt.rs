//! TS 2.1's server stunts (`CTSStunt`, issue #100 §3.8 and the slot limits of §3.5; decomp
//! `docs/ts21/movement.md` §3). Addresses are 2.1 `mp.dll` VAs.
//!
//! The port keeps TS 3.0's `iuser4` bit layout and animation ids (the 3.0 player models): 2.1's
//! glide bit 0x80000 is the port's `SUPERJUMP_GLIDE`, and its animations above 0x34 are one
//! lower than the ids the shared code plays.

use goldsrc::sys::IN_JUMP;
use shared::ts::stunt::{GO_DUCK, RECOVER, ROLLING};
use ts_server::player::Player;
use ts_server::rules::state::{self, Mode};
use ts_server::stunt::StuntState;
use ts_server::stunt::rules::{Dive, FirePose2, StuntClock, StuntRules};

/// 2.1's stunt values.
pub const STUNTS: StuntRules = StuntRules {
    // `Landing` (0x1005c4e0): roll-out recover `time + 0.75 * slowInv`.
    dive_roll_out_recover: 0.75,
    // `HandleWheel` (0x1005c040): flip landing `0.5 * slowInv`.
    flip_recover: 0.5,
    // `HandleSlide` (0x1005c1f0): get-up `time + slowInv` (1.0).
    get_up_recover: 1.0,
    // `HandleWallJump` (0x1005bd80) / `HandleWallClimb` (0x1005bec0): `iuser4 |= 1`, recover 0.5.
    wall_landing_recover: 0.5,
    wall_landing_sets_recover: true,
    // `GoRoll` (0x1005d600).
    roll_recover: 0.75,
    // `PostThink` (0x10054180): the hard landing's 0.75, and `m_flNextAttack = 0.75 * slowInv`
    // after rolling out.
    super_jump_landing_recover: 0.75,
    super_jump_roll_attack_delay: 0.75,
    // `Landing` and `HandleSlide` test `button & 0x4002`: duck doesn't count.
    get_up_buttons: shared::ts::IN_ALT1 as i32 | IN_JUMP as i32,
    // `HandleStunt` (0x1005b9f0) never calls a wall dive, nor does `HandleWallClimb`.
    wall_dive: false,
    // `HandleStunt`: `!(button & IN_JUMP) || gliding` takes the wall-jump branch, so jump while
    // gliding doesn't climb.
    wall_climb_while_gliding: false,
    // `HandleStunt` leaves the close-combat window (`pl+0x718..0x720`) alone.
    stunt_ends_close_combat: false,
    // `GoWheel` (0x1005d0e0) and `GoRoll` don't read the jump kick's cooldown (`+0x0c`).
    wheel_and_roll_wait_for_kick: false,
    // `CloseAttack` (0x1005cd00): `kickCD = time + 0.7 * slowInv` and `RandomLong(0, 2)` right
    // after the gate (0x1005cdd8..), before `CloseCombatAttack`; a miss leaves it armed.
    kick_cooldown_on_try: true,
    // `CTSAnimation::Animate` (0x1003d8b0) case 0x36 (2.1's wall-jump id, the port's 0x37): held
    // while `!m_fSequenceFinished && alive && GO_DUCK && !ladder` against everything but the two
    // wall poses: no on-ground release and no dive exception.
    wall_jump_pose_holds_until_recovered: true,
    // `Animate` case 0x37 (2.1's wall climb, the port's 0x38): no dive exception (#112).
    wall_climb_pose_lets_dive_through: false,
    // `Animate` case 5: `!m_fSequenceFinished && (button & IN_DUCK) && !FL_DUCKING`, against all
    // but 0x2f, 0x28, 4, 0x38 (the port's 0x39), 0x14 and 0x21 (`katana_duck`, the port's 0x25).
    goduck_holds: true,
    // `Animate` case 0x2a/0x2b: `button & 0x600`.
    side_roll_holds_on_strafe_keys: true,
    // `Animate`'s close-combat branch: no `GaitCloseCombat`, `cc_flag` or on-ground condition.
    melee_gait_follows: false,
    // The leg tail (0x1003ea..) reads only the torso extension (`pl+0xab0`).
    gait_idle_anim_extension: false,
    // `Animate`/`HandleCloseCombat` (0x1003f340) write `time + 2 * slowInv` / `time + slowInv`
    // (`FixedSlowMul`), the bullet dance (FUN_1003ee00, FUN_1003ee50) `time + 0.15/3 * slowInv`.
    anim_end_times: true,
    // `HandleSlide` (0x1005c1f0..0x1005c396) normalises `v_forward` inline.
    side_roll_normalizes_forward: true,
    // `Animate` (0x1003d8b0) fire case: `2` after `strncmp(ext, "one", 3) == 0`, the stored
    // horizontal speed below 180 and not `FL_DUCKING` (0x1003e8c9..0x1003e923); the shot timer is
    // set as always (0x1003e96f). Nothing in `CTSAnimation` writes the hold timer (`+0x08`).
    fire_pose_2: FirePose2::SlowOneHanded,
    // `Animate`'s idle refresh (ids 1/6, 0x1003e9f7..): only the torso extension's poses, aiming
    // on the shot timer alone (`fov` isn't read).
    idle_anim_extension: false,
    aim_pose_while_zoomed: false,
    // FUN_1003f680 (2.1's `GaitCloseCombat`): `fcomp [80.0]` (0x1003f711).
    melee_gait_speed: 80.0,
    // `GoBulletTime` (0x1005b160): no dodge window (2.1 `TraceAttack` has only the stunt ×0.7),
    // and `if (m_flNextAttack < 0) m_flNextAttack = 0.55 * slowInv`.
    bullet_time_dodge: None,
    bullet_time_attack_delay: Some(0.55),
    // `Play` (0x1005acf0): the recovery's end clears 0x40000, 0x4 and 0x1 and leaves
    // `BlockPlayer`'s lock (`pl+0x460`) to `PreThink`.
    recover_end_clears: ROLLING | GO_DUCK | RECOVER,
    recover_end_unblocks: false,
};

/// `CanDiveHigh/Medium/Low` (0x1005b0b0, 0x1005b110, 0x1005b130): used slots (`81 - free`)
/// below 22 / 42 / 72. Nothing bypasses them (2.1 has no The One perk for it). In the round modes
/// (`TSGameRules` mode byte not 0/1) a high stunt also needs `armorvalue <= 0`: `fcomp 0.0;
/// test ah,0x41; jp` refuses when the armour is above 0 (or a NaN). That is, high stunts are for
/// players without armour there (the decomp report reads it the other way round).
pub fn can_dive(p: &Player, armor: f32, dive: Dive) -> bool {
    let continuous = matches!(state::with(|g| g.mode), Mode::Dm | Mode::TeamDm);
    can_dive_in(p.slots_used, armor, continuous, dive)
}

fn can_dive_in(used: i32, armor: f32, continuous: bool, dive: Dive) -> bool {
    match dive {
        Dive::High => used < 22 && (continuous || armor <= 0.0),
        Dive::Medium => used < 42,
        Dive::Low => used < 72,
    }
}

/// `GoBulletTime` (0x1005b160): `if (pev->fuser1 == 0) return`, i.e. only while slowed. 2.1
/// encodes normal time as 0, the port as 1 (issue #100 §1.2), so any other time scale counts.
pub fn bullet_time_allowed(time_scale: f32) -> bool {
    time_scale != 1.0
}

/// 2.1's stunt timers: absolute end times `gpGlobals->time + d * slowInv`, fixed when the timer
/// starts (every `CTSStunt` function, `BlockPlayer` 0x10056b60). Nothing counts them down; a
/// timer runs while its end is ahead of the time and is over once the time is past it.
///
/// `slowInv` is `FixedSlowMul(fuser1)` (0x1005a3d0): `1 / FixedSlowAmt(fuser1)`, the amount at
/// least `0.1f` (0x1005a3b0). (2.1's own "normal" `fuser1` is 0, which `FixedSlowMul` maps to 1;
/// the port's is 1, which gives the same.) The x87 maths (`double` durations, `float10`
/// products, stored as `float`) is done in `f64`.
pub struct Absolute;

pub static ABSOLUTE: Absolute = Absolute;

/// `FixedSlowAmt` (0x1005a3b0): `fuser1 < 0.1f ? 0.1f : fuser1` (a NaN stays).
fn fixed_slow_amt(time_scale: f32) -> f32 {
    if time_scale < 0.1 { 0.1 } else { time_scale }
}

/// One fuse think of 2.1's `CM61Grenade::WaitThink` (0x10047887..0x100478c3), issue #112:
/// `FixedSlowAmt(fuser1) * 0.1 + elapsed` with the `double` 0.1 (`fmul qword`) and the 0.1 floor,
/// rounded once. (2.1's normal `fuser1 == 0` takes `elapsed + 0.1`, the same sum as the port's
/// normal 1.) The x87 sum is done in `f64`.
pub fn grenade_fuse_step(elapsed: f32, time_scale: f32) -> f32 {
    (f64::from(fixed_slow_amt(time_scale)) * 0.1 + f64::from(elapsed)) as f32
}

/// 2.1's `PreThink` animation rate (0x100537e0), issue #112: `fuser1 == 0 || fuser1 >= 0.1` (normal
/// speed, or at least a tenth) gives `1 / FixedSlowMul(fuser1)`, anything slower (or a NaN) 0, so
/// the model stops. The port's normal time scale is 1.
pub fn anim_framerate(time_scale: f32) -> f32 {
    if time_scale >= 0.1 { (1.0 / slow_inv(time_scale)) as f32 } else { 0.0 }
}

/// `FixedSlowMul` (0x1005a3d0) for the port's time scale.
pub fn slow_inv(time_scale: f32) -> f64 {
    1.0 / f64::from(fixed_slow_amt(time_scale))
}

impl StuntClock for Absolute {
    /// `time + d * slowInv`. `NextDive()` (0x1005b150) is 0 and its timer is `0 + time`, not
    /// scaled.
    fn start(&self, now: f32, time_scale: f32, secs: f64) -> f32 {
        if secs == 0.0 {
            return now;
        }
        (slow_inv(time_scale) * secs + f64::from(now)) as f32
    }

    /// `nextStunt <= time` / `!(time < recover)` let a stunt through.
    fn running(&self, now: f32, t: f32) -> bool {
        t > now
    }

    /// `recover < time` (`Play`), `proneLock < time` (`HandleSlide`), `nextStunt < time`
    /// (the wall landings), `pl+0x460 < time` (`PreThink`).
    fn done(&self, now: f32, t: f32) -> bool {
        t < now
    }

    fn tick(&self, s: &mut StuntState, dt: f32) {
        let _ = (s, dt);
    }

    fn tick_block(&self, s: &mut StuntState, dt: f32) {
        let _ = (s, dt);
    }

    /// `BlockPlayer` (`FUN_10056b60`): `pl+0x460 = t + gpGlobals->time`, a `float` sum. Its one
    /// caller, the kung-fu attack, passes the slowed duration already (`ts21_gameplay`'s
    /// `ccombat_attack`, issue #242).
    fn block_end(&self, now: f32, time_scale: f32, t: f32) -> f32 {
        let _ = time_scale;
        (f64::from(t) + f64::from(now)) as f32
    }

    /// `WriteByte(__ftol((next - time) * 10))`: truncated, the low byte.
    fn next_stunt_byte(&self, now: f32, t: f32) -> u8 {
        ((f64::from(t) - f64::from(now)) * 10.0) as i32 as u8
    }

    /// `GoWallJump` (0x1005b320) / `GoWallClimb` (0x1005b550):
    /// `time + min(10, slowInv * max(0.5, chain))`. `GoWallJump` passes the unrounded sum on as
    /// a `double` (`fst dword [esi+0x3c]` then `fstp qword`, 0x1005b436..0x1005b442);
    /// `GoWallClimb` reloads the stored `float` (0x1005b68c).
    fn wall_chain_cooldown(&self, now: f32, time_scale: f32, chain: f64) -> f32 {
        let m = if chain <= 0.5 { 0.5 } else { chain };
        let v = slow_inv(time_scale) * m;
        let v = if 10.0 <= v { 10.0 } else { v };
        (v + f64::from(now)) as f32
    }

    /// `HandleStunt` / `GoWheel`: refused when `slowInv * 0.7 < time - lastGroundTime`.
    // The negated compare is the x87's: a NaN `slowInv` (a NaN `fuser1`) is unordered, so it
    // doesn't refuse.
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    fn air_stunt_allowed(&self, now: f32, time_scale: f32, last_ground: f32) -> bool {
        !(slow_inv(time_scale) * 0.7 < f64::from(now) - f64::from(last_ground))
    }

    /// `m_flNextAttack = UTIL_WeaponTimeBase() + d * slowInv`: 2.1's gun counts its timers down
    /// in real time (`PostThink` 0x10054180).
    fn attack_delay(&self, time_scale: f32, secs: f64) -> f32 {
        (slow_inv(time_scale) * secs) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grenade_fuse_and_animation_rate_floor_at_a_tenth() {
        assert_eq!(grenade_fuse_step(0.0, 0.01), 0.01);
        assert_eq!(grenade_fuse_step(0.0, 0.5), 0.05);
        assert_eq!(grenade_fuse_step(3e-6, 1.0), (3e-6f32 as f64 + 0.1) as f32, "the double 0.1");
        assert_ne!(grenade_fuse_step(3e-6, 1.0), 3e-6 + 0.1f32);
        assert_eq!(anim_framerate(0.1), 0.1);
        assert_eq!(anim_framerate(0.55), 0.55);
        assert_eq!(anim_framerate(0.099), 0.0);
        assert_eq!(anim_framerate(f32::NAN), 0.0);
    }
    use ts30_server::stunt::{RELATIVE, STUNTS as CORE};

    #[test]
    fn slot_limits() {
        // used < 22 / 42 / 72 (3.0: 42 / 52 / 62).
        for (used, high, medium, low) in [
            (21, true, true, true),
            (22, false, true, true),
            (41, false, true, true),
            (42, false, false, true),
            (71, false, false, true),
            (72, false, false, false),
        ] {
            assert_eq!([Dive::High, Dive::Medium, Dive::Low].map(|d| can_dive_in(used, 0.0, true, d)), [high, medium, low], "{used}");
        }
    }

    /// The round modes refuse high stunts to players wearing armour.
    #[test]
    fn high_stunts_in_round_modes() {
        assert!(can_dive_in(10, 0.0, false, Dive::High));
        assert!(can_dive_in(10, -5.0, false, Dive::High));
        assert!(!can_dive_in(10, 1.0, false, Dive::High));
        assert!(!can_dive_in(10, f32::NAN, false, Dive::High));
        assert!(can_dive_in(10, 100.0, true, Dive::High), "DM/TDM don't look at the armour");
        assert!(can_dive_in(10, 100.0, false, Dive::Medium));
    }

    /// No The One bypass.
    #[test]
    fn the_one_has_no_bypass() {
        let mut p = Player { slots_used: 60, ..Player::default() };
        p.ts.is_the_one = true;
        assert!(!can_dive_in(p.slots_used, 0.0, true, Dive::High));
        assert!(ts30_server::stunt::can_dive(&p, Dive::High), "3.0's does");
    }

    #[test]
    fn bullet_time_needs_slow_motion() {
        assert!(!bullet_time_allowed(1.0));
        assert!(bullet_time_allowed(0.35) && bullet_time_allowed(0.05) && bullet_time_allowed(2.0));
    }

    #[test]
    fn values() {
        let s = STUNTS;
        assert_eq!((s.dive_roll_out_recover, s.flip_recover, s.get_up_recover, s.wall_landing_recover), (0.75, 0.5, 1.0, 0.5));
        assert_eq!(s.get_up_buttons, 0x4002);
        assert_eq!(s.recover_end_clears, 0x40005);
        assert!(s.wall_landing_sets_recover && !s.wall_dive && !s.wall_climb_while_gliding);
        assert!(!s.stunt_ends_close_combat && !s.wheel_and_roll_wait_for_kick && !s.recover_end_unblocks);
        assert!(s.kick_cooldown_on_try, "2.1 arms the jump kick's cooldown on every try");
        assert!(s.wall_jump_pose_holds_until_recovered);
        assert!(!s.wall_climb_pose_lets_dive_through && !s.melee_gait_follows && !s.gait_idle_anim_extension);
        assert!(s.goduck_holds && s.side_roll_holds_on_strafe_keys && s.anim_end_times && s.side_roll_normalizes_forward);
        assert_eq!((s.bullet_time_dodge, s.bullet_time_attack_delay), (None, Some(0.55)));
        // #174: the torso's choices and the melee legs' speed.
        assert_eq!(s.fire_pose_2, FirePose2::SlowOneHanded);
        assert!(!s.idle_anim_extension && !s.aim_pose_while_zoomed);
        assert_eq!(s.melee_gait_speed, 80.0);
        // 3.0's stay.
        assert_eq!((CORE.get_up_buttons, CORE.wall_landing_recover), (0x4006, 0.2625));
        let core = CORE;
        assert_eq!((core.fire_pose_2, core.melee_gait_speed), (FirePose2::AfterHold, 120.0));
        assert!(core.idle_anim_extension && core.aim_pose_while_zoomed);
    }

    #[test]
    fn slow_inv_floor() {
        assert_eq!(slow_inv(1.0), 1.0);
        assert_eq!(slow_inv(0.5), 2.0);
        assert_eq!(slow_inv(0.05), 1.0 / f64::from(0.1f32));
        assert_eq!(slow_inv(0.0), 1.0 / f64::from(0.1f32));
    }

    /// #242: `BlockPlayer`'s end is the passed (already slowed) duration plus the time, a `float`
    /// sum, whatever the time scale.
    #[test]
    fn block_end_adds_the_time() {
        let t = f32::from_bits(0x3f2c_cccc);
        assert_eq!(Absolute.block_end(1000.25, 0.35, t), 1000.25 + t);
        assert_eq!(Absolute.block_end(1000.25, 1.0, t), 1000.25 + t);
    }

    /// Timers are end times fixed at the start: a slow motion that starts or ends later doesn't
    /// stretch them.
    #[test]
    fn absolute_timers() {
        let c = &ABSOLUTE;
        assert_eq!(c.start(100.0, 1.0, 0.75), 100.75);
        assert_eq!(c.start(100.0, 0.5, 0.75), 101.5);
        assert_eq!(c.start(100.0, 0.5, 0.0), 100.0, "NextDive: time + 0");
        let t = c.start(100.0, 0.5, 0.5);
        assert!(c.running(100.9, t) && !c.done(100.9, t));
        assert!(!c.running(101.0, t) && !c.done(101.0, t), "at the end: neither");
        assert!(c.done(101.01, t));
        // An unset timer (0) never runs and is over.
        assert!(!c.running(5.0, 0.0) && c.done(5.0, 0.0));
        // Nothing counts down.
        let mut s = StuntState::default();
        (s.recover, s.next_stunt, s.block_timer) = (t, t, t);
        c.tick(&mut s, 0.1);
        c.tick_block(&mut s, 0.1);
        assert_eq!((s.recover, s.next_stunt, s.block_timer), (t, t, t));
        // The relative 3.0 clock is untouched.
        assert_eq!(RELATIVE.start(100.0, 0.5, 0.75), 0.75);
    }

    #[test]
    fn next_stunt_message() {
        let c = &ABSOLUTE;
        assert_eq!(c.next_stunt_byte(100.0, 100.0), 0);
        assert_eq!(c.next_stunt_byte(100.0, 100.55), 5, "truncated (3.0 rounds: 6)");
        assert_eq!(c.next_stunt_byte(100.0, 110.0), 100);
    }

    /// `time + min(10, slowInv * max(0.5, chain))`.
    #[test]
    fn wall_chain_cap() {
        let c = &ABSOLUTE;
        assert_eq!(c.wall_chain_cooldown(100.0, 1.0, 0.35), 100.5);
        assert_eq!(c.wall_chain_cooldown(100.0, 1.0, 2.0), 102.0);
        assert_eq!(c.wall_chain_cooldown(100.0, 1.0, 30.0), 110.0, "capped at 10");
        assert_eq!(c.wall_chain_cooldown(100.0, 0.35, 4.0), 110.0, "the cap is after slowInv");
        assert_eq!(c.wall_chain_cooldown(100.0, 0.5, 2.0), 104.0);
        // GoWallJump's unrounded sum (0.3f + 0.35) against the float GoWallClimb would reload.
        let sum = f64::from(0.3f32) + 0.35;
        assert_eq!(c.wall_chain_cooldown(1.0, 1.0, sum).to_bits(), 0x3fd3_3333, "1.65 - 1 ulp");
        assert_eq!(c.wall_chain_cooldown(1.0, 1.0, f64::from(sum as f32)).to_bits(), 0x3fd3_3334);
    }

    #[test]
    fn coyote_time_and_attack_delay() {
        let c = &ABSOLUTE;
        assert!(c.air_stunt_allowed(10.0, 1.0, 9.4) && !c.air_stunt_allowed(10.0, 1.0, 9.2));
        assert!(c.air_stunt_allowed(10.0, 0.5, 8.7));
        assert_eq!(c.attack_delay(1.0, 0.75), 0.75);
        assert_eq!(c.attack_delay(0.5, 0.55), 1.1);
    }
}
