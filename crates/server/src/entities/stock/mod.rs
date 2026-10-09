//! Stock Half-Life map entities that TS maps use (see docs/design.md §5 for the list and
//! priority). Owner: stock-entities work package.

use super::link_entities;
use crate::world::{Cx, Entity};
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::ffi::CStr;

link_entities! {
    worldspawn             => World,
    info_player_deathmatch => SpawnPoint,
    info_player_start      => SpawnPoint,
    func_wall              => Brush<true>,
    func_illusionary       => Brush<false>,
}

#[derive(Default)]
pub struct World;

impl Entity for World {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"skyname" => {
                goldsrc::eng!(pfnCVarSetString(c"sv_skyname".as_ptr(), value.as_ptr()));
                true
            }
            _ => crate::rules::world_key_value(key, value),
        }
    }

    fn spawn(&mut self, _cx: &mut Cx) {
        // Style 0 is normal light; without it the whole map renders black.
        engine().light_style(0, c"m");
        crate::precache_all();
    }
}

/// Spawn points keep their origin/angles in pev; the rules pick among them.
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
