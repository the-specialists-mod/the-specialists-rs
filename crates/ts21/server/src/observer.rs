//! The observer part of TS 2.1's `CBasePlayer::PreThink` (0x100537e0, the `pev->iuser1 != 0`
//! block 0x10053b25..0x10053d63; decomp `docs/ts21/rules.md` §3), issue #113.
//!
//! 2.1 has no `respawn` client command: an observer respawns by holding fire once its respawn
//! time has passed. That check comes first, ahead of the countdown text, and returns before the
//! observer buttons. 2.1's bots respawn the same way (`bots`, `Think` bit 0x100). The rest is
//! 3.0's but for the join deadline's text, which 2.1 shows at the top (effect 0x11).
//!
//! The observer camera is 2.1's own (issue #115): `Observer_HandleButtons` (FUN_1005aa00),
//! `Observer_Idle` (FUN_1005a930), `Observer_SetMode` (FUN_1005aad0, also reached from
//! `StartObserver` through [`ServerRules::observer_set_mode`](ts_server::gameplay::ServerRules::observer_set_mode))
//! and `Observer_FindNextPlayer` (FUN_1005a7d0).

use goldsrc::edict::EdictPtr;
use goldsrc::server::engine;
use goldsrc::sys::{EF_NODRAW, IN_ATTACK, IN_BACK, IN_FORWARD, IN_JUMP};
use shared::ts::obs;
use ts_server::player::Player;
use ts_server::rules::lifecycle::{self, obs_handle_valid};
use ts_server::rules::msg::ts_message_one;
use ts_server::rules::state::{self, Mode};
use ts_server::rules::{alive, cvars, has_player, set_client_maxspeed, teams};
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

    handle_buttons(p, cx, now);
    idle(p, cx, now);
}

/// `+jump`'s next mode (FUN_1005aa00): roaming → chase free → chase locked → roaming. 2.1 has no
/// in-eye step (3.0 goes chase locked → in-eye → roaming).
pub fn next_mode(mode: i32) -> i32 {
    match mode {
        obs::ROAMING => obs::CHASE_FREE,
        obs::CHASE_FREE => obs::CHASE_LOCKED,
        _ => obs::ROAMING,
    }
}

/// Whether the round modes or LMS hold a waiting observer back (FUN_1005a7d0, FUN_1005aad0): mode
/// 2 or 3, or `lastmanstanding` on in any mode.
fn waiting_rules() -> bool {
    matches!(state::with(|g| g.mode), Mode::RoundTdm | Mode::Objective) || cvars::on(c"lastmanstanding")
}

/// Roaming's maxspeed (FUN_1005aad0): 420, but 1 for an observer still waiting to respawn
/// (`0.0 <= m_flRespawnTime`, so not a NaN) under [`waiting_rules`], which can't fly around.
pub fn roaming_maxspeed(waiting_rules: bool, respawn_at: f32) -> f32 {
    if waiting_rules && 0.0 <= respawn_at { 1.0 } else { 420.0 }
}

/// Whether a waiting observer may only watch teammates (FUN_1005a7d0): under [`waiting_rules`]
/// unless `m_flRespawnTime < 0` (a NaN counts as waiting).
pub fn teammates_only(waiting_rules: bool, respawn_at: f32) -> bool {
    waiting_rules && !(respawn_at < 0.0)
}

/// `Observer_SetMode` (FUN_1005aad0). As 3.0's (the view offset and view model cleared, the chase
/// modes and in-eye need a target and fall back to roaming) but for roaming's maxspeed
/// ([`roaming_maxspeed`]).
pub fn set_mode(p: &mut Player, cx: &mut Cx, mut mode: i32) {
    if mode == cx.pev().iuser1 {
        return;
    }
    loop {
        {
            let v = cx.pev();
            v.view_ofs = [0.0; 3];
            v.viewmodel = 0;
        }
        if mode == obs::ROAMING {
            {
                let v = cx.pev();
                v.iuser1 = obs::ROAMING;
                v.iuser2 = 0;
            }
            set_client_maxspeed(cx.edict, 420.0);
            let speed = roaming_maxspeed(waiting_rules(), p.respawn_at);
            if speed != 420.0 {
                set_client_maxspeed(cx.edict, speed);
            }
            return;
        }
        if !matches!(mode, obs::CHASE_LOCKED | obs::CHASE_FREE | obs::IN_EYE) {
            return;
        }
        if !obs_handle_valid(p.ts.obs_target) {
            find_next_player(p, cx, false);
        }
        if obs_handle_valid(p.ts.obs_target) {
            let v = cx.pev();
            v.iuser1 = mode;
            v.iuser2 = p.ts.obs_target;
            return;
        }
        mode = obs::ROAMING;
        if cx.pev().iuser1 == obs::ROAMING {
            return;
        }
    }
}

/// `Observer_FindNextPlayer` (FUN_1005a7d0): from the target (or from us) to the next player who
/// is visible (`EF_NODRAW` clear), not observing and alive, wrapping around; a waiting observer
/// under the round modes or LMS takes teammates only ([`teammates_only`]). Without one it roams;
/// with one it moves to the target. The target slot keeps its player as 3.0's handle test has it
/// (2.1 holds a raw `CBasePlayer *`, which a slot keeps).
fn find_next_player(p: &mut Player, cx: &mut Cx, reverse: bool) {
    let eng = engine();
    let me = cx.index();
    let start = if obs_handle_valid(p.ts.obs_target) { p.ts.obs_target } else { me };
    p.ts.obs_target = 0;
    let n = eng.max_clients();
    let mates = teammates_only(waiting_rules(), p.respawn_at);
    let mut i = start;
    loop {
        i += if reverse { -1 } else { 1 };
        if i > n {
            i = 1;
        } else if i < 1 {
            i = n;
        }
        let o = eng.player(i);
        if i != me
            && has_player(o)
            && {
                let v = o.v();
                v.iuser1 == 0 && v.effects & EF_NODRAW as i32 == 0 && alive(v)
            }
            && (!mates || teams::same_team(cx.edict, o))
        {
            p.ts.obs_target = i;
            break;
        }
        if i == start {
            break;
        }
    }
    if p.ts.obs_target == 0 {
        set_mode(p, cx, obs::ROAMING);
    } else {
        cx.pev().iuser2 = p.ts.obs_target;
        let org = eng.player(p.ts.obs_target).v().origin;
        eng.set_origin(cx.edict, org);
    }
}

/// `Observer_HandleButtons` (FUN_1005aa00): `+jump` steps [`next_mode`], forward/back pick the
/// next/previous target outside roaming; 0.2 s between actions.
fn handle_buttons(p: &mut Player, cx: &mut Cx, now: f32) {
    if p.ts.obs_next_input > now {
        return;
    }
    if cx.pev().button & IN_JUMP as i32 != 0 {
        let mode = next_mode(cx.pev().iuser1);
        set_mode(p, cx, mode);
        p.ts.obs_next_input = shared::pm::plus_double(now, 0.2);
    }
    if cx.pev().button & IN_FORWARD as i32 != 0 && cx.pev().iuser1 != obs::ROAMING {
        find_next_player(p, cx, false);
        p.ts.obs_next_input = shared::pm::plus_double(now, 0.2);
    }
    if cx.pev().button & IN_BACK as i32 != 0 && cx.pev().iuser1 != obs::ROAMING {
        find_next_player(p, cx, true);
        p.ts.obs_next_input = shared::pm::plus_double(now, 0.2);
    }
}

/// `Observer_Idle` (FUN_1005a930): in-eye (mode 5) copies the target's `view_ofs`, `viewmodel`
/// and `weaponanim` (`pev+0x174`, `+0xbc`, `+0x218`) every frame, then 3.0's twice-a-second check
/// moves on from a dead or observing target.
fn idle(p: &mut Player, cx: &mut Cx, now: f32) {
    if cx.pev().iuser1 == obs::IN_EYE {
        let t = engine().player(cx.pev().iuser2);
        if has_player(t) {
            let (ofs, model, anim) = {
                let v = t.v();
                (v.view_ofs, v.viewmodel, v.weaponanim)
            };
            let v = cx.pev();
            v.view_ofs = ofs;
            v.viewmodel = model;
            v.weaponanim = anim;
        }
    }
    if p.ts.obs_next_idle > now {
        return;
    }
    p.ts.obs_next_idle = now + 0.5;
    if cx.pev().iuser1 == obs::ROAMING || p.ts.obs_target == 0 {
        return;
    }
    let t = engine().player(p.ts.obs_target);
    if !obs_handle_valid(p.ts.obs_target) || !alive(t.v()) || t.v().iuser1 != 0 {
        find_next_player(p, cx, false);
    }
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
    fn jump_cycle_has_no_in_eye() {
        assert_eq!(next_mode(obs::ROAMING), obs::CHASE_FREE);
        assert_eq!(next_mode(obs::CHASE_FREE), obs::CHASE_LOCKED);
        assert_eq!(next_mode(obs::CHASE_LOCKED), obs::ROAMING, "3.0 goes to in-eye here");
        assert_eq!(next_mode(obs::IN_EYE), obs::ROAMING);
    }

    #[test]
    fn a_waiting_roaming_observer_is_held_still() {
        assert_eq!(roaming_maxspeed(true, 5.0), 1.0);
        assert_eq!(roaming_maxspeed(true, 0.0), 1.0, "0 <= respawn");
        assert_eq!(roaming_maxspeed(true, -1.0), 420.0, "out for the round: may fly");
        assert_eq!(roaming_maxspeed(true, f32::NAN), 420.0);
        assert_eq!(roaming_maxspeed(false, 5.0), 420.0, "DM/TDM without LMS");
    }

    #[test]
    fn a_waiting_observer_watches_teammates() {
        assert!(teammates_only(true, 5.0));
        assert!(teammates_only(true, f32::NAN), "only `respawn < 0` frees it");
        assert!(!teammates_only(true, -1.0));
        assert!(!teammates_only(false, 5.0));
    }

    #[test]
    fn ftol_is_msvc_s() {
        assert_eq!(ftol(-1.5), -1);
        assert_eq!(ftol(2.9), 2);
        assert_eq!(ftol(f64::NAN), i32::MIN);
        assert_eq!(ftol(3e9), i32::MIN);
    }
}
