//! `CBasePlayer::PlayerUse` (0xd9848): what +use does to the world. One search picks a single
//! object for every package: stock entities (buttons, doors, chargers, pushables, tank
//! controls), TS map items (ground weapons and ammo, the CTB case) and the weapons package's
//! dropped guns, thrown blades and grenades. TS calls it from `ImpulseCommands` (0xdb780), so
//! it runs in PostThink under the gate of `ItemPostFrame` (0xdc4d0): not while a tank is in use,
//! and not until the gun's `m_flNextAttack` has run out (`weapons::post_think`).

use super::Player;
use crate::entities::stock::{self, FCAP_CONTINUOUS_USE, FCAP_IMPULSE_USE, FCAP_ONOFF_USE, drive, explode::emit_sound_slow};
use crate::world::{self, Cx, UseType};
use goldsrc::edict::EdictPtr;
use goldsrc::ext::entities::ATTN_NORM;
use goldsrc::server::{IGNORE_MONSTERS, engine};
use goldsrc::sys::*;
use shared::weapons::spread::{self, Vec3};
use std::cell::Cell;

/// `PLAYER_SEARCH_RADIUS`.
const SEARCH_RADIUS: f32 = 64.0;
/// `VIEW_FIELD_NARROW`: the object must be within about 45° of the view.
const VIEW_FIELD_NARROW: f32 = 0.7;

/// `ObjectCaps()` for the +use search: the stock classes, else the TS entities (TS has no class
/// with both).
pub fn object_caps(e: *mut edict_t) -> i32 {
    match stock::object_caps(e) {
        0 if !e.is_null() => crate::entities::ts::object_caps(engine().string(e.v().classname)),
        caps => caps,
    }
}

/// `CBasePlayer::PlayerUse` (0xd9848). `pressed` and `released` are this frame's
/// `m_afButtonPressed` / `m_afButtonReleased`.
pub fn player_use(p: &mut Player, cx: &mut Cx, pressed: i32, released: i32) {
    let use_bit = IN_USE as i32;
    let button = cx.pev().button;
    if (button | pressed | released) & use_bit == 0 {
        return;
    }
    // A press first stops a tank or train, or takes the train's controls.
    if drive::player_use(p, cx, pressed) {
        return;
    }
    let me = cx.edict;
    let eng = engine();
    let (origin, eye, v_angle) = {
        let v = cx.pev();
        (v.origin, spread::add(v.origin, v.view_ofs), v.v_angle)
    };
    let (fwd, _, _) = eng.make_vectors(v_angle);
    let mut best = std::ptr::null_mut();
    let mut best_dot = VIEW_FIELD_NARROW;
    let mut it = std::ptr::null_mut();
    loop {
        it = eng.find_in_sphere(it, origin, SEARCH_RADIUS);
        if it.is_null() {
            break;
        }
        if object_caps(it) & (FCAP_IMPULSE_USE | FCAP_CONTINUOUS_USE | FCAP_ONOFF_USE) == 0 {
            continue;
        }
        // `VecBModelOrigin` to the eyes, moved to the box corner nearest the player.
        let (absmin, size) = {
            let v = it.v();
            (v.absmin, v.size)
        };
        let half = spread::scale(size, 0.5);
        let d = spread::dot(clamp_to_box(spread::sub(spread::add(absmin, half), eye), half), fwd);
        if d > best_dot {
            best_dot = d;
            best = it;
        }
    }
    if best.is_null() {
        return;
    }
    // The TS addition: objects that can't be used through walls need a clear line from the eyes
    // to their `Center()`, ignoring the object itself and all monsters.
    let (class, center) = {
        let v = best.v();
        (eng.string(v.classname), spread::scale(spread::add(v.absmin, v.absmax), 0.5))
    };
    if !crate::entities::ts::can_use_through_walls(class) && eng.trace_line(eye, center, IGNORE_MONSTERS, best).flFraction < 1.0 {
        return;
    }
    let caps = object_caps(best);
    if pressed & use_bit != 0 {
        emit_sound_slow(me, CHAN_ITEM, c"common/wpn_select.wav", 0.4, ATTN_NORM, 0, 100, 0.5);
    }
    let value = if (button & use_bit != 0 && caps & FCAP_CONTINUOUS_USE != 0)
        || (pressed & use_bit != 0 && caps & (FCAP_IMPULSE_USE | FCAP_ONOFF_USE) != 0)
    {
        if caps & FCAP_CONTINUOUS_USE != 0 {
            p.drive.using = true;
        }
        1.0
    } else if released & use_bit != 0 && caps & FCAP_ONOFF_USE != 0 {
        // SDK "BUGBUG": the release sends USE_SET with value 0, not USE_OFF.
        0.0
    } else {
        return;
    };
    world::dispatch(best, |ent, icx| {
        // TS items take the player, who is being dispatched, directly.
        let any = &mut *ent as &mut dyn std::any::Any;
        if !crate::weapons::use_by_player(any, icx, p, cx) && !crate::entities::ts::use_by_player(any, icx, p, cx) {
            ent.use_(icx, me, me, UseType::Set, value);
        }
    });
}

/// `UTIL_ClampVectorToBox` (0xee5d8): the part of `v` outside a box of half-size `half`,
/// normalised; (0, 0, 1) when `v` is inside.
pub fn clamp_to_box(v: Vec3, half: Vec3) -> Vec3 {
    let mut out = v;
    for k in 0..3 {
        out[k] = if v[k] > half[k] {
            v[k] - half[k]
        } else if v[k] < -half[k] {
            v[k] + half[k]
        } else {
            0.0
        };
    }
    spread::normalize(out)
}

/// `CBasePlayer::ImpulseCommands` (0xdb780): `PlayerUse`, then `pev->impulse`: 99 toggles the
/// `Logo` message, 204 is `ForceClientDllUpdate`; the impulse is cleared whatever it was.
pub fn impulse_commands(p: &mut Player, cx: &mut Cx, pressed: i32, released: i32) {
    player_use(p, cx, pressed, released);
    match cx.pev().impulse {
        99 => logo(cx.edict),
        204 => crate::rules::commands::force_client_dll_update(p, cx),
        // 201 (spray, `CSprayCan`) and `CheatImpulseCommands` aren't ported.
        _ => {}
    }
    cx.pev().impulse = 0;
}

/// `gmsgLogo`: 0 until impulse 99 registers the message, and 0 again after the next one. It lives
/// for the process, as the engine's message table does.
static LOGO_MSG: goldsrc::Global<Cell<i32>> = goldsrc::Global(Cell::new(0));

/// Impulse 99: when `gmsgLogo` is 0, `REG_USER_MSG("Logo", 1)` (the engine returns the old
/// number if it was registered before) and `Logo 1`; otherwise `Logo 0` and `gmsgLogo = 0`.
fn logo(e: *mut edict_t) {
    let (id, on) = logo_toggle(LOGO_MSG.0.get(), || engine().reg_user_msg(c"Logo", 1));
    goldsrc::server::MsgWriter::begin(MSG_ONE as i32, id, None, e).byte(on as i32).end();
    LOGO_MSG.0.set(if on { id } else { 0 });
}

/// The message number and byte `logo` sends for the current `gmsgLogo`.
fn logo_toggle(current: i32, register: impl FnOnce() -> i32) -> (i32, bool) {
    if current == 0 { (register(), true) } else { (current, false) }
}

#[cfg(test)]
mod tests {
    use super::{clamp_to_box, logo_toggle};

    #[test]
    fn logo_impulse_toggles() {
        // First press registers the message and sends 1; the next sends 0 and forgets the number.
        assert_eq!(logo_toggle(0, || 140), (140, true));
        assert_eq!(logo_toggle(140, || panic!("registered twice")), (140, false));
    }

    #[test]
    fn clamp_vector_to_box() {
        assert_eq!(clamp_to_box([10.0, 0.0, 0.0], [4.0, 4.0, 4.0]), [1.0, 0.0, 0.0]);
        assert_eq!(clamp_to_box([0.0, -10.0, 3.0], [4.0, 4.0, 4.0]), [0.0, -1.0, 0.0]);
        assert_eq!(clamp_to_box([1.0, 1.0, 1.0], [4.0, 4.0, 4.0]), [0.0, 0.0, 1.0], "inside the box");
        let v = clamp_to_box([7.0, 8.0, 0.0], [4.0, 4.0, 0.0]);
        assert!((v[0] - 0.6).abs() < 1e-6 && (v[1] - 0.8).abs() < 1e-6);
    }
}
