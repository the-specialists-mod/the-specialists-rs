//! TS's client-side `PM_SpectatorMove` (`0x1002c790`). The client copy of player movement matches
//! the server's except here and in the three flags `PM_PlayerMove` (`0x1002e330`) clears first.
//! Roaming is the shared move; following a target differs from the server's:
//!
//! * it records what the view may smooth (`cl_vsmoothing` flags `0x101914ec`: bit 2 angles,
//!   bit 4 origins, bit 8 "no bob") and tells `V_CalcSpectatorRefdef` to use the camera it
//!   computed (`0x1019149c` angles, `0x101914f0` origin, kept in `0x10190b6c`/`0x101913e4`);
//! * when the target isn't among the physents (it left the PVS), the camera stays where it was;
//! * it runs the HLTV director's locked view (mode 4): looking from the primary target towards
//!   the secondary one, the side picked by the director's flags.
//!
//! The flags and saved camera are only written when `runfuncs` is set, as in TS.

use super::{Spectator, interpolate_angles, with};
use goldsrc::ext::render::{Vec3, vec3 as v};
use goldsrc::pmove::PlayerMoveExt;
use goldsrc::sys::*;
use shared::gameplay::PmRules;
use shared::ts::obs;

/// `cl_vsmoothing` flag bits (`0x101914ec`).
pub(crate) mod smooth {
    /// Set on every client spectator move.
    pub const MOVED: i32 = 1;
    pub const ANGLES: i32 = 2;
    pub const ORIGIN: i32 = 4;
    /// In-eye: `V_CalcBob` keeps bobbing (it returns 0 while any other flag is set).
    pub const BOB: i32 = 8;
}

/// Director flags (`DRC_CAMERA`'s long, `0x10192158`) the locked view reads.
const DRC_FLAG_SIDE: i32 = 0x10;
const DRC_FLAG_DRAMATIC: i32 = 0x20;

/// The start of the client's `PM_PlayerMove` (`0x1002e330`): with `runfuncs` set, the smoothing
/// flags and both camera overrides are cleared before every move.
pub fn begin_player_move(pm: &playermove_t) {
    if pm.runfuncs != 0 {
        with(|s| {
            s.smooth_flags = 0;
            s.angles_override = false;
            s.origin_override = false;
        });
    }
}

/// The client's `PM_SpectatorMove` (`0x1002c790`), as a [`shared::pm::SpectatorMove`]: false
/// hands roaming (mode 3, or no target) to the shared move. `rules` give the in-eye pitch
/// ([`PmRules::in_eye_pitch`]): TS 3.0's client copies the target's pitch, TS 2.1's
/// (`FUN_1005c820`, at `0x1005c9..`) multiplies it by -2 as its server does.
pub fn spectator_move(pm: &mut playermove_t, rules: &'static dyn PmRules) -> bool {
    let runfuncs = pm.runfuncs != 0;
    with(|s| {
        if runfuncs {
            s.smooth_flags = smooth::MOVED;
        }
        if pm.iuser1 == obs::ROAMING || pm.iuser2 == 0 {
            // After a director reset a roaming spectator is put on the map camera once
            // (angles `0x10191d2c`, never written, so zero) and the move ends there. A
            // ruleset's HLTV camera jump uses the same pin with its own angles.
            if !s.pin_pending {
                return false;
            }
            pm.origin = s.default_origin;
            pm.angles = s.pin_angles;
            pm.velocity = [0.0; 3];
            s.pin_pending = false;
            return true;
        }
        follow(s, pm, rules, runfuncs);
        true
    })
}

/// The physent whose `info` (entity index) is `entity`.
fn physent_of(pm: &playermove_t, entity: i32) -> Option<usize> {
    let n = pm.numphysent.clamp(0, pm.physents.len() as i32) as usize;
    (0..n).find(|&i| pm.physents[i].info == entity)
}

/// The follow half of `0x1002c790`.
fn follow(s: &mut Spectator, pm: &mut playermove_t, rules: &dyn PmRules, runfuncs: bool) {
    let Some(target) = physent_of(pm, pm.iuser2) else {
        // The target isn't sent (outside the PVS): keep the last camera.
        if runfuncs {
            pm.origin = s.saved_origin;
            pm.angles = s.saved_angles;
            pm.velocity = [0.0; 3];
            s.angles_override = true;
            s.origin_override = true;
        }
        return;
    };
    let mode = pm.iuser1;
    // The locked view's secondary target: its physent, `None` when there is none (or it is the
    // world physent), `Some(None)` when it isn't sent.
    let secondary = (mode == obs::LOCKED_VIEW && pm.iuser3 != 0).then(|| physent_of(pm, pm.iuser3).filter(|&i| i >= 1));
    let pe = pm.physents[target];
    let angles = match mode {
        obs::CHASE_LOCKED => {
            if runfuncs {
                s.smooth_flags |= smooth::ORIGIN;
            }
            [10.0 - pe.angles[0], pe.angles[1], pe.angles[2]]
        }
        obs::IN_EYE => {
            if runfuncs {
                s.smooth_flags |= smooth::ANGLES | smooth::ORIGIN | smooth::BOB;
            }
            in_eye_angles(rules, pe.angles)
        }
        obs::LOCKED_VIEW => {
            let angles = match secondary {
                Some(Some(sec)) => {
                    let from = pe.origin;
                    let to = pm.physents[sec].origin;
                    let a = locked_view_angles(v::sub(to, from), s.director_flags);
                    let cam = shared::pm::chase_origin(pm, shared::pm::Arith::CLIENT, a, target);
                    let blocked = pm.trace_line(cam, to, PM_STUDIO_IGNORE as i32, 2, -1).is_some_and(|tr| line_blocked(tr.fraction));
                    if blocked { swing_round(a, s.director_flags) } else { a }
                }
                Some(None) => s.saved_angles,
                // TS halves and negates the yaw here (`0x1002cb7d`, `fmul -0.5`).
                None => [pe.angles[0], pe.angles[1] * -0.5, pe.angles[2]],
            };
            if runfuncs {
                s.smooth_flags |= smooth::ANGLES | smooth::ORIGIN;
                // `InterpolateAngles(locked, angles, angles, 0.1)` (`0x1002caf2`), then copied
                // back into the locked angles: both inputs it normalises are overwritten.
                let (mut start, mut end) = (s.locked_angles, angles);
                s.locked_angles = interpolate_angles(&mut start, &mut end, 0.1);
                s.locked_angles
            } else {
                angles
            }
        }
        _ => pm.angles,
    };
    let origin = if mode == obs::IN_EYE { pe.origin } else { shared::pm::chase_origin(pm, shared::pm::Arith::CLIENT, angles, target) };
    pm.origin = origin;
    pm.angles = angles;
    pm.velocity = [0.0; 3];
    if runfuncs {
        s.saved_angles = angles;
        s.saved_origin = origin;
        s.angles_override = true;
        s.origin_override = true;
    }
}

/// The in-eye camera angles from the target's entity angles: the pitch through the ruleset's
/// [`PmRules::in_eye_pitch`] (3.0 `0x1002c790` copies it, 2.1 `FUN_1005c820` multiplies it by
/// -2), the yaw and roll copied.
pub fn in_eye_angles(rules: &dyn PmRules, target: Vec3) -> Vec3 {
    [rules.in_eye_pitch(target[0]), target[1], target[2]]
}

/// The locked view looking along `dir` (primary → secondary): pitched 12.5° down (up with the
/// dramatic flag) and turned 22.5° to the side the director picked.
fn locked_view_angles(dir: Vec3, flags: i32) -> Vec3 {
    let mut a = v::vector_angles(dir);
    a[0] = if flags & DRC_FLAG_DRAMATIC != 0 { -a[0] - 12.5 } else { -a[0] + 12.5 };
    a[1] += if flags & DRC_FLAG_SIDE != 0 { 22.5 } else { -22.5 };
    a
}

/// The locked view's line test (`0x1002ca94`): `fraction == 1.0` with `fcomp`/`test ah, 0x40`, so
/// a NaN fraction is not blocked.
fn line_blocked(fraction: f32) -> bool {
    !(fraction == 1.0 || fraction.is_nan())
}

/// When the line from the chase camera to the secondary target is blocked, TS swings the camera
/// 45° back the other way.
fn swing_round(mut a: Vec3, flags: i32) -> Vec3 {
    a[1] += if flags & DRC_FLAG_SIDE != 0 { -45.0 } else { 45.0 };
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locked_view_sides() {
        let a = locked_view_angles([1.0, 0.0, 0.0], 0);
        assert_eq!((a[0], a[1]), (12.5, -22.5));
        let a = locked_view_angles([1.0, 0.0, 0.0], DRC_FLAG_SIDE | DRC_FLAG_DRAMATIC);
        assert_eq!((a[0], a[1]), (-12.5, 22.5));
        assert_eq!(swing_round([0.0, 10.0, 0.0], DRC_FLAG_SIDE)[1], -35.0);
        assert_eq!(swing_round([0.0, 10.0, 0.0], 0)[1], 55.0);
        // Looking straight down the axis from below: VectorAngles' pitch wraps to 270 first.
        let a = locked_view_angles([0.0, 0.0, -1.0], 0);
        assert_eq!((a[0], a[1]), (-257.5, -22.5));
    }

    #[test]
    fn blocked_trace() {
        assert!(line_blocked(0.25));
        assert!(!line_blocked(1.0));
        assert!(!line_blocked(f32::NAN), "TS treats a NaN fraction as clear");
    }
}
