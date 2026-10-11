//! Blood effects (TS util.cpp, unchanged from the SDK: `UTIL_ShouldShowBlood` 0xedad0,
//! `UTIL_BloodStream` 0xedb68, `UTIL_BloodDrips` 0xedc64, `UTIL_RandomBloodVector` 0xeddb8,
//! `UTIL_BloodDecalTrace` 0xede38) and `env_blood` (`CBlood`, 0xbcfc4..0xbd26c).

use super::base::*;
use super::effects::decal_trace;
use crate::world::{Cx, Entity, UseType};
use goldsrc::Global;
use goldsrc::edict::EdictPtr;
use goldsrc::ext::entities::{MsgWriterExt, Vec3, is_player, te, te_begin, vec};
use goldsrc::server::{IGNORE_MONSTERS, engine};
use goldsrc::sys::*;
use std::cell::Cell;
use std::ffi::CStr;

crate::entities::link_entities! {
    env_blood => EnvBlood,
}

pub const DONT_BLEED: i32 = -1;
pub const BLOOD_COLOR_RED: i32 = 247;
pub const BLOOD_COLOR_YELLOW: i32 = 195;

/// `g_sModelIndexBloodSpray` / `g_sModelIndexBloodDrop` (precached by TS's `W_Precache`).
static BLOOD_SPRITES: Global<Cell<(i32, i32)>> = Global(Cell::new((0, 0)));

/// `g_sModelIndexBloodSpray`, precached by `W_Precache` (`crate::precache`).
pub fn set_spray_sprite(index: i32) {
    let (_, drop) = BLOOD_SPRITES.0.get();
    BLOOD_SPRITES.0.set((index, drop));
}

/// `g_sModelIndexBloodDrop`, precached by `W_Precache` (`crate::precache`).
pub fn set_drop_sprite(index: i32) {
    let (spray, _) = BLOOD_SPRITES.0.get();
    BLOOD_SPRITES.0.set((spray, index));
}

/// `g_Language`: `sv_language` as `CWorld::Precache` read it (1 = German: no blood decals from
/// gibs and `models/germangibs.mdl` for every gib).
static LANGUAGE: Global<Cell<i32>> = Global(Cell::new(0));

pub fn set_language(language: i32) {
    LANGUAGE.0.set(language);
}

/// `g_Language == 1`.
pub fn german() -> bool {
    LANGUAGE.0.get() == 1
}

/// `UTIL_ShouldShowBlood`: red blood needs `violence_hblood`, any other colour `violence_ablood`.
pub fn should_show_blood(color: i32) -> bool {
    if color == DONT_BLEED {
        return false;
    }
    let cvar = if color == BLOOD_COLOR_RED { c"violence_hblood" } else { c"violence_ablood" };
    engine().cvar_float(cvar) != 0.0
}

/// `g_Language == 1` turns red blood into colour 0 in `UTIL_BloodStream` / `UTIL_BloodDrips`.
fn language_color(color: i32) -> i32 {
    if german() && color == BLOOD_COLOR_RED { 0 } else { color }
}

/// `UTIL_BloodStream` (0xedb68): a `TE_BLOODSTREAM` arc.
pub fn blood_stream(origin: Vec3, dir: Vec3, color: i32, amount: i32) {
    if !should_show_blood(color) {
        return;
    }
    let color = language_color(color);
    te_begin(MSG_PVS, Some(origin), te::BLOODSTREAM).vec3(origin).vec3(dir).byte(color).byte(amount.min(255)).end();
}

/// `UTIL_BloodDrips` (0xedc64): a `TE_BLOODSPRITE` puff; the amount is doubled in multiplayer
/// (`IsMultiplayer()`: every rules but single player's).
pub fn blood_drips(origin: Vec3, _dir: Vec3, color: i32, amount: i32) {
    if !should_show_blood(color) || color == DONT_BLEED || amount == 0 {
        return;
    }
    let color = language_color(color);
    let amount = if crate::rules::state::single_player() { amount } else { amount * 2 }.min(255);
    let (spray, drop) = BLOOD_SPRITES.0.get();
    te_begin(MSG_PVS, Some(origin), te::BLOODSPRITE)
        .vec3(origin)
        .short(spray)
        .short(drop)
        .byte(color)
        .byte((amount / 10).clamp(3, 16))
        .end();
}

/// `UTIL_RandomBloodVector`: random x/y, upward z.
pub fn random_blood_vector() -> Vec3 {
    let eng = engine();
    let x = eng.random_float(-1.0, 1.0);
    let y = eng.random_float(-1.0, 1.0);
    let z = eng.random_float(0.0, 1.0);
    [x, y, z]
}

/// `UTIL_BloodDecalTrace`: one of the six red (`{blood1..6`) or yellow (`{yblood1..6`) decals.
pub fn blood_decal_trace(tr: &TraceResult, color: i32) {
    if !should_show_blood(color) {
        return;
    }
    const RED: [&CStr; 6] = [c"{blood1", c"{blood2", c"{blood3", c"{blood4", c"{blood5", c"{blood6"];
    const YELLOW: [&CStr; 6] = [c"{yblood1", c"{yblood2", c"{yblood3", c"{yblood4", c"{yblood5", c"{yblood6"];
    let i = engine().random_long(0, 5) as usize;
    decal_trace(tr, if color == BLOOD_COLOR_RED { RED[i] } else { YELLOW[i] });
}

pub const SF_BLOOD_RANDOM: i32 = 1;
pub const SF_BLOOD_STREAM: i32 = 2;
pub const SF_BLOOD_PLAYER: i32 = 4;
pub const SF_BLOOD_DECAL: i32 = 8;

/// `env_blood` (`CBlood`): blood (`impulse` = colour, `dmg` = amount) when used.
#[derive(Default)]
pub struct EnvBlood;

impl EnvBlood {
    /// `CBlood::Direction` (0xbd0cc).
    fn direction(cx: &mut Cx) -> Vec3 {
        if cx.pev().spawnflags & SF_BLOOD_RANDOM != 0 { random_blood_vector() } else { cx.pev().movedir }
    }

    /// `CBlood::BloodPosition` (0xbd120): with spawnflag 4, near the eyes of the activating
    /// player (or player 1).
    fn position(cx: &mut Cx, activator: *mut edict_t) -> Vec3 {
        if cx.pev().spawnflags & SF_BLOOD_PLAYER != 0 {
            let eng = engine();
            let p = if is_player(activator) { activator } else { eng.entity_of_index(1) };
            if !p.is_null() {
                let eye = vec::add(p.v().origin, p.v().view_ofs);
                let x = eng.random_float(-10.0, 10.0);
                let y = eng.random_float(-10.0, 10.0);
                let z = eng.random_float(-10.0, 10.0);
                return vec::add(eye, [x, y, z]);
            }
        }
        cx.pev().origin
    }
}

impl Entity for EnvBlood {
    /// `CBlood::KeyValue` (0xbd018).
    fn key_value(&mut self, cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"color" => cx.pev().impulse = if kv_i(value) == 1 { BLOOD_COLOR_YELLOW } else { BLOOD_COLOR_RED },
            b"amount" => cx.pev().dmg = kv_f(value),
            _ => return false,
        }
        true
    }

    /// `CBlood::Spawn` (0xbcfc4).
    fn spawn(&mut self, cx: &mut Cx) {
        let v = cx.pev();
        v.solid = SOLID_NOT as i32;
        v.movetype = MOVETYPE_NONE as i32;
        v.effects = 0;
        v.frame = 0.0;
        set_movedir(v);
    }

    /// `CBlood::Use` (0xbd26c).
    fn use_(&mut self, cx: &mut Cx, activator: *mut edict_t, _caller: *mut edict_t, _kind: UseType, _value: f32) {
        let (color, amount) = (cx.pev().impulse, cx.pev().dmg as i32);
        if cx.pev().spawnflags & SF_BLOOD_STREAM != 0 {
            let dir = Self::direction(cx);
            let pos = Self::position(cx, activator);
            blood_stream(pos, dir, if color == BLOOD_COLOR_RED { 70 } else { color }, amount);
        } else {
            let dir = Self::direction(cx);
            let pos = Self::position(cx, activator);
            blood_drips(pos, dir, color, amount);
        }
        if cx.pev().spawnflags & SF_BLOOD_DECAL != 0 {
            let forward = Self::direction(cx);
            let start = Self::position(cx, activator);
            let end = vec::mad(start, forward, cx.pev().dmg * 2.0);
            let tr = engine().trace_line(start, end, IGNORE_MONSTERS, std::ptr::null_mut());
            if tr.flFraction != 1.0 {
                blood_decal_trace(&tr, color);
            }
        }
    }
}
