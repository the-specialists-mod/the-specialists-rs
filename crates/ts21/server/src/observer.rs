//! The observer part of TS 2.1's `CBasePlayer::PreThink` (0x100537e0, the `pev->iuser1 != 0`
//! block 0x10053b25..0x10053d63; decomp `docs/ts21/rules.md` §3), issue #113.
//!
//! 2.1 has no `respawn` client command: an observer respawns by holding fire once its respawn
//! time has passed. That check comes first, ahead of the countdown text, and returns before the
//! observer buttons. 2.1's bots respawn the same way (`bots`, `Think` bit 0x100). The rest is
//! 3.0's but for the join deadline's text, which 2.1 shows at the top (effect 0x11).
//!
//! The observer camera (`Observer_HandleButtons` FUN_1005aa00, `Observer_Idle` FUN_1005a930) is
//! still TS 3.0's (see `docs/implementation/ts21.md`, Deviations).

use goldsrc::sys::IN_ATTACK;
use ts_server::player::Player;
use ts_server::rules::lifecycle;
use ts_server::rules::msg::ts_message_one;
use ts_server::rules::state;
use ts_server::world::Cx;

/// "You have to wait until the next round." in `0xc3, 0x2d, 0x2d` (0x10099360).
const WAIT_TEXT: &str = "You have to wait until the next round.";
const WAIT_RGB: [u8; 3] = [0xc3, 0x2d, 0x2d];

/// What holding fire does to an observer (0x10053b33..0x10053bd8).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Fire {
    /// No respawn this round (`m_flRespawnTime < 0`) and the HUD line is due: "You have to wait
    /// until the next round.".
    pub wait_text: bool,
    /// The respawn time has passed: respawn now.
    pub respawn: bool,
}

/// [`Fire`] for the respawn time `respawn_at` (`pl+0x6e8`), the next HUD line time `next_msg`
/// (`pl+0x6cc`) and `now`. Every compare is ordered (a NaN does nothing), as the `fcomp`s test it.
pub fn fire(respawn_at: f32, next_msg: f32, now: f32) -> Fire {
    Fire { wait_text: respawn_at < 0.0 && next_msg < now, respawn: 0.0 < respawn_at && respawn_at < now }
}

/// MSVC's `__ftol` on the FPU's `double`: truncation toward zero; NaN and out of range give the
/// integer indefinite.
fn ftol(v: f64) -> i32 {
    if v > -2147483649.0 && v < 2147483648.0 { v as i32 } else { i32::MIN }
}

/// The countdown line while the respawn time is ahead (0x10053bd9..0x10053c8a): the hold time
/// and text. `respawn_at - now` stays on the FPU (`fsub`, not stored): "Press Fire To Play!"
/// (hold 30) once it truncates below 1, else the whole seconds, capped at 30 (hold 20).
pub fn countdown(respawn_at: f32, now: f32) -> (u8, String) {
    let left = respawn_at as f64 - now as f64;
    if ftol(left) < 1 {
        return (30, "Press Fire To Play!".to_owned());
    }
    // `fcom 30.0f; test ah,5; jnp`: below 30 kept, anything else (a NaN too) is 30.
    let capped = if left < 30.0 { left } else { 30.0 };
    (20, format!("{}", ftol(capped)))
}

/// The join deadline (`pl+0x734`, 0x10053c8d..0x10053d43): `None` while there is none, else
/// whether it ran out. A NaN deadline counts as one that is still running (`fcomp time;
/// test ah,5; jp`), as does the moment it is reached.
pub fn deadline_over(deadline: f32, now: f32) -> Option<bool> {
    if deadline == 0.0 { None } else { Some(deadline < now) }
}

/// The join deadline's line: `"%i seconds left for respawn, press Fire to Play!"` (0x10099318),
/// the seconds truncated on the FPU.
pub fn deadline_text(deadline: f32, now: f32) -> String {
    format!("{} seconds left for respawn, press Fire to Play!", ftol(deadline as f64 - now as f64))
}

/// The observer block of 2.1's `PreThink` ([`ServerRules::observer_think`](ts_server::gameplay::ServerRules::observer_think)).
pub fn observer_think(p: &mut Player, cx: &mut Cx, now: f32) {
    let e = cx.edict;
    if cx.pev().button & IN_ATTACK as i32 != 0 {
        let f = fire(p.respawn_at, p.ts.next_hud_msg, now);
        if f.wait_text {
            p.ts.next_hud_msg = now + 1.0;
            ts_message_one(e, WAIT_RGB, 0x10, 0x14, WAIT_TEXT);
        }
        if f.respawn {
            // `m_flRespawnTime = 0; StopObserver()` (FUN_1005a700, the same as 3.0's); PreThink
            // then zeroes `impulse` and returns.
            p.respawn_at = 0.0;
            lifecycle::stop_observer(p, cx);
            return;
        }
    }

    if 0.0 < p.respawn_at && now < p.respawn_at && p.ts.next_hud_msg < now {
        p.ts.next_hud_msg = now + 1.0;
        let (hold, text) = countdown(p.respawn_at, now);
        ts_message_one(e, [0, 0, 0], 0x10, hold, &text);
    }

    match deadline_over(p.ts.join_deadline, now) {
        Some(true) => {
            p.ts.join_deadline = 0.0;
            p.respawn_at = -1.0;
            p.ts.award = 1;
            ts_message_one(e, WAIT_RGB, 0x10, 0x14, WAIT_TEXT);
            // FUN_10045910 is `CallCheckDeads`.
            state::with(|g| g.call_check_deads(now));
        }
        Some(false) if p.ts.next_hud_msg < now => {
            p.ts.next_hud_msg = now + 1.0;
            ts_message_one(e, [0, 0, 0], 0x11, 0x14, &deadline_text(p.ts.join_deadline, now));
        }
        _ => {}
    }

    lifecycle::observer_handle_buttons(p, cx, now);
    lifecycle::observer_idle(p, cx, now);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fire_respawns_once_the_time_has_passed() {
        assert_eq!(fire(10.0, 0.0, 10.5), Fire { wait_text: false, respawn: true });
        assert_eq!(fire(10.0, 0.0, 10.0), Fire { wait_text: false, respawn: false }, "strictly after");
        assert_eq!(fire(10.0, 0.0, 9.0), Fire { wait_text: false, respawn: false }, "still counting down");
        assert_eq!(fire(0.0, 0.0, 9.0), Fire { wait_text: false, respawn: false }, "0: just respawned");
    }

    #[test]
    fn fire_without_a_respawn_says_wait_once_a_second() {
        assert_eq!(fire(-1.0, 5.0, 6.0), Fire { wait_text: true, respawn: false });
        assert_eq!(fire(-1.0, 6.5, 6.0), Fire { wait_text: false, respawn: false }, "line not due yet");
        assert_eq!(fire(f32::NAN, 0.0, 6.0), Fire { wait_text: false, respawn: false }, "a NaN does nothing");
        assert_eq!(fire(-1.0, f32::NAN, 6.0), Fire { wait_text: false, respawn: false });
    }

    #[test]
    fn countdown_truncates_and_caps() {
        assert_eq!(countdown(14.7, 10.0), (20, "4".to_owned()));
        assert_eq!(countdown(10.9, 10.0), (30, "Press Fire To Play!".to_owned()));
        assert_eq!(countdown(100.0, 10.0), (20, "30".to_owned()), "capped at 30");
        assert_eq!(countdown(40.0, 10.0), (20, "30".to_owned()));
    }

    /// `respawn_at - now` isn't rounded to a `float` first: 2.1 truncates the exact difference.
    #[test]
    fn countdown_uses_the_unrounded_difference() {
        // 2 - 2^-25 is 2.0 as a `float`, which would show "2".
        let now = 2f32.powi(-25);
        assert_eq!(2.0f32 - now, 2.0);
        assert_eq!(countdown(2.0, now), (20, "1".to_owned()));
    }

    #[test]
    fn join_deadline() {
        assert_eq!(deadline_over(0.0, 5.0), None);
        assert_eq!(deadline_over(5.0, 5.0), Some(false), "reached, not passed");
        assert_eq!(deadline_over(4.0, 5.0), Some(true));
        assert_eq!(deadline_over(f32::NAN, 5.0), Some(false), "a NaN shows the line");
        assert_eq!(deadline_text(30.9, 10.0), "20 seconds left for respawn, press Fire to Play!");
    }

    #[test]
    fn ftol_is_msvc_s() {
        assert_eq!(ftol(-1.5), -1);
        assert_eq!(ftol(2.9), 2);
        assert_eq!(ftol(f64::NAN), i32::MIN);
        assert_eq!(ftol(3e9), i32::MIN);
    }
}
