//! TS 3.0's observer: the observer part of `CBasePlayer::PreThink` (0x908cc, the respawn
//! countdown and the join deadline, `Observer_HandleButtons`, `Observer_Idle`) and
//! `Observer_SetMode` (0x99e44) with `Observer_FindNextPlayer`. An observer leaves with the
//! `respawn` client command (`ts_server::rules::lifecycle::respawn_command`).

use goldsrc::edict::EdictPtr;
use goldsrc::server::engine;
use goldsrc::sys::{EF_NODRAW, IN_BACK, IN_FORWARD, IN_JUMP};
use shared::ts::obs;
use ts_server::player::Player;
use ts_server::rules::lifecycle::obs_handle_valid;
use ts_server::rules::msg::ts_message_one;
use ts_server::rules::{alive, has_player, is_alive, set_client_maxspeed, state, teams, x87_trunc};
use ts_server::world::Cx;

/// `CBasePlayer::Observer_SetMode` (0x99e44). Only TS's modes 1, 2, 3 and 5 are accepted; the
/// chase and in-eye modes need a target (`m_hObserverTarget`) and fall back to roaming.
pub fn set_mode(p: &mut Player, cx: &mut Cx, mode: i32) {
    if mode == cx.pev().iuser1 {
        return;
    }
    {
        let v = cx.pev();
        v.view_ofs = [0.0; 3];
        v.viewmodel = 0;
    }
    if mode == obs::ROAMING {
        let v = cx.pev();
        v.iuser1 = obs::ROAMING;
        v.iuser2 = 0;
        set_client_maxspeed(cx.edict, 420.0);
        return;
    }
    if !matches!(mode, obs::CHASE_LOCKED | obs::CHASE_FREE | obs::IN_EYE) {
        return;
    }
    if !obs_handle_valid(p.ts.obs_target) {
        observer_find_next_player(p, cx, false);
    }
    if !obs_handle_valid(p.ts.obs_target) {
        set_mode(p, cx, obs::ROAMING);
        return;
    }
    let v = cx.pev();
    v.iuser1 = mode;
    v.iuser2 = p.ts.obs_target;
}

/// `CBasePlayer::Observer_FindNextPlayer` (0x99b0c): steps from the current target (or from us)
/// to the next living, visible, non-observing player, wrapping around. In LMS (and the
/// unreachable round modes) an observer still waiting to respawn may only watch teammates.
fn observer_find_next_player(p: &mut Player, cx: &mut Cx, reverse: bool) {
    let eng = engine();
    let me = cx.index();
    let start = if obs_handle_valid(p.ts.obs_target) { p.ts.obs_target } else { me };
    p.ts.obs_target = 0;
    let n = eng.max_clients();
    let teammates_only = p.respawn_at >= 0.0 && state::with(|g| g.lms || matches!(g.mode, state::Mode::RoundTdm | state::Mode::Objective));
    let mut i = start;
    loop {
        i += if reverse { -1 } else { 1 };
        if i > n {
            i = 1;
        }
        if i < 1 {
            i = n;
        }
        // `UTIL_PlayerByIndex`: a player who left is still a candidate (only during intermission
        // is it alive and visible).
        let o = eng.player(i);
        if i != me
            && has_player(o)
            && {
                let v = o.v();
                v.iuser1 == 0 && v.effects & EF_NODRAW as i32 == 0 && alive(v)
            }
            && (!teammates_only || teams::same_team(cx.edict, o))
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

/// The observer part of `CBasePlayer::PreThink` (0x908cc): respawn countdown text,
/// `Observer_HandleButtons`, `Observer_Idle`. An observer leaves with the `respawn` client command
/// (`ts_server::rules::lifecycle::respawn_command`), not from here.
///
/// The countdown runs only while the respawn time is ahead. "Press Fire To Play!" (30 s hold) is
/// sent by the tick that lands in its last second, and isn't repeated after that.
pub fn observer_think(p: &mut Player, cx: &mut Cx, now: f32) {
    let e = cx.edict;
    if p.respawn_at > 0.0 && now < p.respawn_at && p.ts.next_hud_msg < now {
        p.ts.next_hud_msg = now + 1.0;
        let (hold, text) = respawn_countdown_text(f64::from(p.respawn_at) - f64::from(now));
        ts_message_one(e, [0, 0, 0], 16, hold, &text);
    }
    if p.ts.join_deadline != 0.0 {
        if now <= p.ts.join_deadline {
            if p.ts.next_hud_msg < now {
                p.ts.next_hud_msg = now + 1.0;
                // `(int)(deadline - time)`: truncation of the unrounded difference (mp.dll
                // 0x1005fc49..0x1005fc51, like the countdown).
                let left = x87_trunc(f64::from(p.ts.join_deadline) - f64::from(now));
                ts_message_one(e, [0, 0, 0], 16, 20, &format!("{left} seconds left for respawn, press Fire to Play!"));
            }
        } else {
            p.ts.join_deadline = 0.0;
            p.respawn_at = -1.0;
            p.ts.award = 1;
            ts_message_one(e, [195, 45, 45], 16, 20, "You have to wait until the next round.");
            state::with(|g| g.call_check_deads(now));
        }
    }

    observer_handle_buttons(p, cx, now);
    observer_idle(p, cx, now);
}

/// The observer respawn countdown text and hold time for `left` seconds to go (PreThink
/// 0x90ec4..0x90fab, mp.dll 0x1005fb5e..0x1005fb93). TS converts with a plain `(int)` cast
/// (truncation: mp.dll's `__ftol`, GCC's `fistp` under RC = 11), so 4.7 s shows "4" and the
/// last second shows "Press Fire To Play!". The number is capped at 30. `left` is
/// `respawn - time` as the x87 holds it, never stored to a `float`: just under a whole second
/// it doesn't round up to it.
fn respawn_countdown_text(left: f64) -> (u8, String) {
    if x87_trunc(left) < 1 {
        (30, "Press Fire To Play!".to_owned())
    } else {
        (20, format!("{}", x87_trunc(if left < 30.0 { left } else { 30.0 })))
    }
}

/// `CBasePlayer::Observer_HandleButtons` (0x99d34): jump cycles roaming → chase free → chase
/// locked → in-eye → roaming; forward/back pick the next/previous target. 0.2 s between actions.
pub fn observer_handle_buttons(p: &mut Player, cx: &mut Cx, now: f32) {
    if p.ts.obs_next_input > now {
        return;
    }
    let buttons = cx.pev().button;
    if buttons & IN_JUMP as i32 != 0 {
        let mode = match cx.pev().iuser1 {
            obs::ROAMING => obs::CHASE_FREE,
            obs::CHASE_FREE => obs::CHASE_LOCKED,
            obs::CHASE_LOCKED => obs::IN_EYE,
            _ => obs::ROAMING,
        };
        set_mode(p, cx, mode);
        p.ts.obs_next_input = shared::pm::plus_double(now, 0.2);
    }
    if buttons & IN_FORWARD as i32 != 0 && cx.pev().iuser1 != obs::ROAMING {
        observer_find_next_player(p, cx, false);
        p.ts.obs_next_input = shared::pm::plus_double(now, 0.2);
    }
    if buttons & IN_BACK as i32 != 0 && cx.pev().iuser1 != obs::ROAMING {
        observer_find_next_player(p, cx, true);
        p.ts.obs_next_input = shared::pm::plus_double(now, 0.2);
    }
}

/// `CBasePlayer::Observer_Idle` (0x99ca4): twice a second, move on when the target died or
/// became an observer. The camera itself follows `iuser2` in `PM_SpectatorMove`.
pub fn observer_idle(p: &mut Player, cx: &mut Cx, now: f32) {
    if p.ts.obs_next_idle > now {
        return;
    }
    p.ts.obs_next_idle = now + 0.5;
    if cx.pev().iuser1 == obs::ROAMING || p.ts.obs_target == 0 {
        return;
    }
    let t = engine().player(p.ts.obs_target);
    if !obs_handle_valid(p.ts.obs_target) || !is_alive(t) || t.v().iuser1 != 0 {
        observer_find_next_player(p, cx, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The difference isn't stored to a `float` first (#147): 2^-25 s before a respawn at 2.0 is
    /// "1" (the `f32` difference rounds to 2.0).
    #[test]
    fn respawn_countdown_subtracts_unrounded() {
        let now = 2f32.powi(-25);
        assert_eq!(2.0 - now, 2.0, "the f32 difference");
        assert_eq!(respawn_countdown_text(f64::from(2.0f32) - f64::from(now)), (20, "1".to_owned()));
        assert_eq!(respawn_countdown_text(f64::from(1.0f32) - f64::from(now)), (30, "Press Fire To Play!".to_owned()));
        assert_eq!(respawn_countdown_text(f64::from(31.0f32) - f64::from(now)), (20, "30".to_owned()));
    }

    #[test]
    fn respawn_countdown_truncates_like_ts() {
        // PreThink 0x90ec4: `(int)(m_flRespawnTime - time)`, truncated, capped at 30.
        assert_eq!(respawn_countdown_text(4.7), (20, "4".to_owned()));
        assert_eq!(respawn_countdown_text(1.0), (20, "1".to_owned()));
        assert_eq!(respawn_countdown_text(45.0), (20, "30".to_owned()));
        assert_eq!(respawn_countdown_text(30.9), (20, "30".to_owned()));
        // The whole last second (not only the last half) says "Press Fire To Play!".
        assert_eq!(respawn_countdown_text(0.9), (30, "Press Fire To Play!".to_owned()));
        assert_eq!(respawn_countdown_text(0.4), (30, "Press Fire To Play!".to_owned()));
    }
}
