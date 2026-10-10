//! Weapon event handlers (`pfnHookEvent`: fire.sc, empty.sc, reload.sc, ...): muzzle flash,
//! sounds, shells, impacts, decals. Owner: rendering/prediction work package.
//! Reference: decomp docs/client/rendering-effects-prediction.md §4, server/weapons.md §6.1.
//!
//! All eleven TS events are hooked (`HookEvents` `FUN_10003240`). Events are played by the server
//! with `FEV_NOTHOST`; the shooter's own copy comes from client weapon prediction
//! (`prediction::gun`), which plays the same events locally.

pub(crate) mod fire;
mod impact;
pub mod info;
mod misc;
pub mod random;

use crate::state;
use goldsrc::ext::render::{self as r, Vec3, vec3 as v};
use goldsrc::sys::*;
use std::ffi::CStr;

pub use fire::{ClientBullet, client_bullets_frame};

/// Events in TS's precache order (`CTSGun::Precache`), with their handlers.
pub const EVENTS: [(&CStr, fn(&event_args_s)); 11] = [
    (c"events/fire.sc", ev_fire),
    (c"events/coldcock.sc", ev_coldcock),
    (c"events/empty.sc", ev_empty),
    (c"events/pump.sc", ev_pump),
    (c"events/closecombat.sc", ev_closecombat),
    (c"events/firelinked.sc", ev_firelinked),
    (c"events/reload.sc", ev_reload),
    (c"events/knife.sc", ev_knife),
    (c"events/katana.sc", ev_katana),
    (c"events/draw.sc", ev_draw),
    (c"events/grenade.sc", ev_grenade),
];

pub fn init() {
    for (name, f) in EVENTS {
        r::hook_event(name, f);
    }
    register_test_commands();
}

/// Decoded `event_args_t`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Args {
    pub entindex: i32,
    pub origin: Vec3,
    pub angles: Vec3,
    pub velocity: Vec3,
    pub ducking: bool,
    pub f1: f32,
    pub f2: f32,
    pub i1: i32,
    pub i2: i32,
    pub b1: i32,
    pub b2: i32,
}

impl Args {
    fn from(a: &event_args_s) -> Args {
        Args {
            entindex: a.entindex,
            origin: a.origin,
            angles: a.angles,
            velocity: a.velocity,
            ducking: a.ducking != 0,
            f1: a.fparam1,
            f2: a.fparam2,
            i1: a.iparam1,
            i2: a.iparam2,
            b1: a.bparam1,
            b2: a.bparam2,
        }
    }
    pub fn is_local(&self) -> bool {
        r::is_local(self.entindex)
    }
    /// `EV_GetGunPosition`: origin + view offset (TS duck view 16).
    pub fn gun_position(&self) -> Vec3 {
        let ofs = if self.is_local() {
            r::local_player_viewheight()
        } else if self.ducking {
            shared::VEC_DUCK_VIEW
        } else {
            shared::VEC_VIEW
        };
        v::add(self.origin, ofs)
    }
    pub fn vectors(&self) -> (Vec3, Vec3, Vec3) {
        v::angle_vectors(self.angles)
    }
}

/// Eased cosmetic time scale (`g_flTimeScale`).
pub fn time_scale() -> f32 {
    state::with(|s| s.time_scale).clamp(0.01, 1.0)
}

/// TS slow-motion pitch: `25 + 75*ts + rand(-5, 5)` (~100 at normal speed). `(int)` truncation
/// as `__ftol` does.
pub fn slow_pitch() -> i32 {
    let ts = time_scale();
    25 - (ts * -75.0) as i32 + r::random_long(-5, 5)
}

/// Weapon fire, ricochet and explosion pitch. TS computes `100 - (int)(ts * -300)` here
/// (`0x1003eeed`, constant `0x1007d6f4` = -300.0), i.e. ~400 at normal speed, which neither engine
/// clamps (FWGS `S_StartSound` passes any pitch > 1 through). That can't be what players heard,
/// so this port uses the same curve as every other TS event sound. See the implementation doc.
pub fn fire_pitch() -> i32 {
    slow_pitch()
}

/// Whether the local view currently shows a first-person view model (`g_pViewModel`: not in
/// third person, fov >= 40).
pub fn first_person_view_model() -> Option<&'static mut cl_entity_s> {
    if crate::view::is_third_person() || state::with(|s| s.fov) < 40.0 {
        return None;
    }
    r::view_model().filter(|vm| !vm.model.is_null())
}

macro_rules! handler {
    ($name:ident => $f:path) => {
        fn $name(a: &event_args_s) {
            $f(&Args::from(a));
        }
    };
}

handler!(ev_fire => fire::fire);
handler!(ev_firelinked => fire::fire_linked);
handler!(ev_empty => misc::empty);
handler!(ev_pump => misc::pump);
handler!(ev_closecombat => misc::close_combat);
handler!(ev_coldcock => misc::coldcock);
handler!(ev_reload => misc::reload);
handler!(ev_knife => misc::knife);
handler!(ev_katana => misc::katana);
handler!(ev_draw => misc::draw);
handler!(ev_grenade => misc::grenade);

#[cfg(test)]
mod tests {
    #[test]
    fn event_names_fit() {
        for (n, _) in super::EVENTS {
            assert!(n.to_bytes().starts_with(b"events/") && n.to_bytes().ends_with(b".sc"));
        }
    }
}

// ---- developer test commands (exercise the handlers without a server sending events) ---------

/// `ts_test_fire <weapon id> [attachments] [seed]`: runs the fire handler as if the local player
/// fired (origin and angles from the local view).
extern "C" fn cmd_test_fire() {
    let e = goldsrc::client::engine();
    let arg = |i: i32| e.cmd_argv(i).to_str().unwrap_or("").parse::<i32>().unwrap_or(0);
    let Some(p) = r::local_player() else { return };
    let mut a = Args { entindex: p.index, origin: p.origin, angles: e.view_angles(), velocity: v::ZERO, ..Default::default() };
    a.i1 = if arg(1) == 0 { 13 } else { arg(1) };
    a.i2 = arg(2);
    a.b2 = arg(3);
    let w = shared::weapons::tables().weapon(a.i1 as u8);
    a.f1 = w.map_or(0.01, |w| w.accuracy);
    a.f2 = a.f1;
    fire::fire(&a);
}

/// `ts_test_event <name> <i1> <i2> <f1>`: runs one of the other handlers for the local player.
extern "C" fn cmd_test_event() {
    let e = goldsrc::client::engine();
    let name = e.cmd_argv(1).to_string_lossy().into_owned();
    let arg = |i: i32| e.cmd_argv(i).to_str().unwrap_or("").parse::<f32>().unwrap_or(0.0);
    let Some(p) = r::local_player() else { return };
    let a = Args {
        entindex: p.index,
        origin: p.origin,
        angles: e.view_angles(),
        i1: arg(2) as i32,
        i2: arg(3) as i32,
        f1: arg(4),
        ..Default::default()
    };
    match name.as_str() {
        "empty" => misc::empty(&a),
        "pump" => misc::pump(&a),
        "reload" => misc::reload(&a),
        "knife" => misc::knife(&a),
        "katana" => misc::katana(&a),
        "closecombat" => misc::close_combat(&a),
        "coldcock" => misc::coldcock(&a),
        _ => e.print("ts_test_event empty|pump|reload|knife|katana|closecombat|coldcock <i1> <i2> <f1>\n"),
    }
}

pub fn register_test_commands() {
    let e = goldsrc::client::engine();
    e.add_command(c"ts_test_fire", cmd_test_fire);
    e.add_command(c"ts_test_event", cmd_test_event);
}
