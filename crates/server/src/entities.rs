//! Map entity classes. Every placeable classname is exported by `link_entities!`.

use crate::world::{Cx, Entity, create};
use goldsrc::server::engine;
use goldsrc::sys::*;
use goldsrc::eng;
use shared::{VEC_HULL_MAX, VEC_HULL_MIN, VEC_VIEW};
use std::cell::Cell;
use std::ffi::CStr;

macro_rules! link_entities {
    ($($class:ident => $ty:ty),* $(,)?) => {$(
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $class(pev: *mut entvars_t) {
            create::<$ty>(pev);
        }
    )*};
}

link_entities! {
    worldspawn             => World,
    info_player_deathmatch => SpawnPoint,
    info_player_start      => SpawnPoint,
    func_wall              => Brush<true>,
    func_illusionary       => Brush<false>,
    player                 => Player,
}

pub const PLAYER_MODEL: &CStr = c"models/player.mdl";

#[derive(Default)]
pub struct World;

impl Entity for World {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        if key == c"skyname" {
            eng!(pfnCVarSetString(c"sv_skyname".as_ptr(), value.as_ptr()));
            return true;
        }
        false
    }

    fn spawn(&mut self, _cx: &mut Cx) {
        let e = engine();
        e.precache_model(PLAYER_MODEL);
        // Style 0 is normal light; without it the whole map renders black.
        eng!(pfnLightStyle(0, c"m".as_ptr()));
        e.print("[mymod] worldspawn: hello from Rust\n");
    }
}

#[derive(Default)]
pub struct SpawnPoint;
impl Entity for SpawnPoint {}

/// `func_wall` (solid) and `func_illusionary` (non-solid): static brush models.
#[derive(Default)]
pub struct Brush<const SOLID: bool>;

impl<const SOLID: bool> Entity for Brush<SOLID> {
    fn spawn(&mut self, cx: &mut Cx) {
        let edict = cx.edict;
        let pev = cx.pev();
        pev.angles = [0.0; 3];
        pev.movetype = MOVETYPE_PUSH as i32;
        pev.solid = if SOLID { SOLID_BSP } else { SOLID_NOT } as i32;
        let model = engine().string(pev.model);
        engine().set_model(edict, model);
    }
}

#[derive(Default)]
pub struct Player {
    /// Health last sent in the `Health` message; `None` forces a resend.
    pub sent_health: Option<u8>,
}

thread_local! {
    static LAST_SPAWN: Cell<*mut edict_t> = const { Cell::new(std::ptr::null_mut()) };
}

/// Round-robin through deathmatch spawns, falling back to the single-player start.
fn next_spawn_point() -> *mut edict_t {
    let e = engine();
    for class in [c"info_player_deathmatch", c"info_player_start"] {
        let mut spot = e.find_by_string(LAST_SPAWN.get(), c"classname", class);
        if spot.is_null() {
            spot = e.find_by_string(std::ptr::null_mut(), c"classname", class);
        }
        if !spot.is_null() {
            LAST_SPAWN.set(spot);
            return spot;
        }
    }
    std::ptr::null_mut()
}

impl Entity for Player {
    fn spawn(&mut self, cx: &mut Cx) {
        let e = engine();
        let edict = cx.edict;
        let pev = cx.pev();
        pev.classname = e.alloc_string(c"player");
        pev.health = 100.0;
        pev.max_health = 100.0;
        pev.armorvalue = 0.0;
        pev.takedamage = DAMAGE_AIM as f32;
        pev.solid = SOLID_SLIDEBOX as i32;
        pev.movetype = MOVETYPE_WALK as i32;
        pev.flags = FL_CLIENT as i32;
        pev.deadflag = DEAD_NO as i32;
        pev.friction = 1.0;
        pev.gravity = 1.0;
        pev.maxspeed = 320.0;
        pev.fov = 90.0;
        pev.view_ofs = VEC_VIEW;
        pev.effects = 0;
        pev.v_angle = [0.0; 3];
        pev.fixangle = 1;

        let spot = next_spawn_point();
        let (origin, angles) = if spot.is_null() {
            ([0.0, 0.0, 64.0], [0.0; 3])
        } else {
            let s = unsafe { &(*spot).v };
            ([s.origin[0], s.origin[1], s.origin[2] + 1.0], s.angles)
        };
        pev.angles = angles;

        e.set_model(edict, PLAYER_MODEL);
        e.set_size(edict, VEC_HULL_MIN, VEC_HULL_MAX);
        e.set_origin(edict, origin);
        eng!(pfnSetClientMaxspeed(edict, 320.0));
        self.sent_health = None;
        e.print(&format!("[mymod] player spawned at {origin:?}\n"));
    }
}
