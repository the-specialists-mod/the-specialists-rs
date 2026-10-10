//! Stock Half-Life map entities that TS maps use (see docs/design.md §5 for the list and
//! priority, docs/implementation/stock-entities.md for what's ported and the TS deviations).
//! Owner: stock-entities work package.

pub mod animating;
pub mod base;
pub mod blood;
pub mod bmodels;
pub mod breakable;
pub mod buttons;
pub mod doors;
pub mod drive;
pub mod effects;
pub mod explode;
pub mod items;
pub mod lights;
pub mod maprules;
pub mod monsters;
pub mod sentences;
pub mod sound;
pub mod tank;
pub mod tracktrain;
pub mod trains;
pub mod triggers;
pub mod xen;

use super::link_entities;
use crate::world::{Cx, Entity};
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::ffi::CStr;

use bmodels::*;
use breakable::{FuncBreakable, FuncPushable};
use buttons::{Button, ButtonTarget, MultiSource};
use doors::Door;
use effects::*;
use goldsrc::edict::EdictPtr;
use items::{ArmorCharger, Battery, HealthCharger, HealthKit};
use lights::Light;
use maprules::{GamePlayerEquip, GameText};
use sound::AmbientGeneric;
use trains::{FuncPlat, FuncTrain, PathCorner};
use triggers::*;

link_entities! {
    worldspawn                => World,
    info_player_deathmatch    => SpawnPoint,
    info_player_start         => SpawnPoint,
    // bmodels.cpp
    func_wall                 => FuncWall,
    func_illusionary          => FuncIllusionary,
    func_wall_toggle          => FuncWallToggle,
    func_conveyor             => FuncConveyor,
    func_rotating             => FuncRotating,
    func_pendulum             => Pendulum,
    // func_break.cpp
    func_breakable            => FuncBreakable,
    func_pushable             => FuncPushable,
    // doors.cpp
    func_door                 => Door,
    func_door_rotating        => Door,
    func_water                => Door,
    // buttons.cpp
    func_button               => Button<false>,
    func_rot_button           => Button<true>,
    button_target             => ButtonTarget,
    multisource               => MultiSource,
    // plats.cpp / pathcorner.cpp
    func_train                => FuncTrain,
    func_plat                 => FuncPlat,
    func_tracktrain           => tracktrain::TrackTrain,
    path_track                => tracktrain::PathTrack,
    func_traincontrols        => tracktrain::TrainControls,
    // func_tank.cpp
    func_tank                 => tank::FuncTank<{ tank::GUN }>,
    func_tanklaser            => tank::FuncTank<{ tank::LASER }>,
    func_tankrocket           => tank::FuncTank<{ tank::ROCKET }>,
    func_tankmortar           => tank::FuncTank<{ tank::MORTAR }>,
    func_tankcontrols         => tank::TankControls,
    path_corner               => PathCorner,
    // triggers.cpp
    trigger_multiple          => Trigger<K_MULTIPLE>,
    trigger_once              => Trigger<K_ONCE>,
    trigger_hurt              => Trigger<K_HURT>,
    trigger_teleport          => Trigger<K_TELEPORT>,
    trigger_push              => Trigger<K_PUSH>,
    trigger_gravity           => Trigger<K_GRAVITY>,
    trigger_auto              => AutoTrigger,
    trigger_relay             => TriggerRelay,
    trigger_changetarget      => ChangeTarget,
    trigger_camera            => TriggerCamera,
    multi_manager             => MultiManager,
    func_ladder               => Ladder,
    func_friction             => FrictionModifier,
    // subs.cpp / world.cpp point entities
    info_target               => PointEntity,
    info_teleport_destination => PointEntity,
    info_intermission         => PointEntity,
    info_landmark             => PointEntity,
    info_null                 => NullEntity,
    // sound.cpp / lights.cpp
    ambient_generic           => AmbientGeneric,
    env_sound                 => sound::EnvSound,
    light                     => Light<false>,
    light_spot                => Light<false>,
    light_environment         => Light<true>,
    // effects.cpp / explode.cpp / world.cpp / h_cycler.cpp
    env_sprite                => Sprite,
    env_glow                  => Glow,
    env_shake                 => Shake,
    env_fade                  => Fade,
    env_explosion             => EnvExplosion,
    spark_shower              => Shower,
    env_render                => RenderFxManager,
    env_beam                  => Lightning,
    env_laser                 => Laser,
    env_shooter               => GibShooter<true>,
    gibshooter                => GibShooter<false>,
    env_spark                 => EnvSpark,
    env_debris                => EnvSpark,
    env_bubbles               => Bubbling,
    infodecal                 => Decal,
    cycler_sprite             => CyclerSprite,
    cycler                    => Cycler,
    // items.cpp / healthkit.cpp / maprules.cpp
    item_healthkit            => HealthKit,
    item_battery              => Battery,
    game_text                 => GameText,
    game_player_equip         => GamePlayerEquip,
    func_healthcharger        => HealthCharger,
    func_recharge             => ArmorCharger,
    env_global                => EnvGlobal,
    env_message               => EnvMessage,
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
        // A new map: per-map registries (masters, env_global states) start empty.
        base::reset_registries();
        tracktrain::reset();
        sound::reset_rooms();
        // CWorld::Precache (0xf449c): sentences.txt, once per process.
        sentences::init();
        blood::precache_world();
        // Style 0 is normal light; without it the whole map renders black. Styles 1-12 are the
        // SDK's animated presets (world.cpp), 63 is the testing style.
        let eng = engine();
        eng.light_style(0, c"m");
        for (i, s) in LIGHT_STYLES.iter().enumerate() {
            eng.light_style(i as i32 + 1, s);
        }
        eng.light_style(63, c"a");
        crate::precache_all();
    }
}

/// SDK world.cpp `CWorld::Precache` light styles 1..=12.
const LIGHT_STYLES: [&CStr; 12] = [
    c"mmnmmommommnonmmonqnmmo",
    c"abcdefghijklmnopqrstuvwxyzyxwvutsrqponmlkjihgfedcba",
    c"mmmmmaaaaammmmmaaaaaabcdefgabcdefg",
    c"mamamamamama",
    c"jklmnopqrstuvwxyzyxwvutsrqponmlkj",
    c"nmonqnmomnmomomno",
    c"mmmaaaabcdefgmmmmaaaammmaamm",
    c"mmmaaammmaaammmabcdefaaaammmmabcdefmmmaaaa",
    c"aaaaaaaazzzzzzzz",
    c"mmamammmmammamamaaamammma",
    c"abcdefghijklmnopqrrqponmlkjihgfedcba",
    c"mmnnmmnnnmmnn",
];

/// Spawn points keep their origin/angles in pev; the rules pick among them.
#[derive(Default)]
pub struct SpawnPoint;
impl Entity for SpawnPoint {}

/// SDK `FCAP_*` object caps for +use (the `Entity` trait has no `ObjectCaps`).
pub const FCAP_IMPULSE_USE: i32 = 0x0000_0008;
pub const FCAP_CONTINUOUS_USE: i32 = 0x0000_0010;
pub const FCAP_ONOFF_USE: i32 = 0x0000_0020;
pub const FCAP_DIRECTIONAL_USE: i32 = 0x0000_0040;

/// `ObjectCaps()` use bits of the stock classes, for the player's +use search
/// (`CBasePlayer::PlayerUse`). 0 = not usable.
pub fn object_caps(e: *mut edict_t) -> i32 {
    if e.is_null() {
        return 0;
    }
    let v = e.v();
    match engine().string(v.classname).to_bytes() {
        b"func_button" | b"func_rot_button" => {
            if v.takedamage == 0.0 {
                FCAP_IMPULSE_USE
            } else {
                0
            }
        }
        b"func_door" | b"func_door_rotating" | b"func_water" => {
            if doors::impulse_use(v) {
                FCAP_IMPULSE_USE
            } else {
                0
            }
        }
        b"button_target" => {
            if v.spawnflags & buttons::SF_BTARGET_USE != 0 {
                FCAP_IMPULSE_USE
            } else {
                0
            }
        }
        b"func_pushable" | b"func_healthcharger" | b"func_recharge" => FCAP_CONTINUOUS_USE,
        // CFuncTankControls::ObjectCaps (0xc4bc8); trains are driven through `drive`.
        b"func_tankcontrols" => FCAP_IMPULSE_USE,
        b"func_tracktrain" => FCAP_DIRECTIONAL_USE,
        // CCycler::ObjectCaps (0xc8654) / CCyclerSprite::ObjectCaps (0xc87b0): TS's impulse bit.
        b"cycler" | b"cycler_sprite" => FCAP_IMPULSE_USE,
        _ => 0,
    }
}

/// SDK `CBasePlayer::PlayerUse` (TS 0xd9848) for the stock classes: a +use press first starts
/// or stops driving a train or tank (`drive::player_use`); otherwise, on +use (pressed, or held
/// for continuous users) find the closest usable entity within 64 units in front of the player
/// and `Use` it. Called from the player's PreThink with this frame's and last frame's buttons.
pub fn player_use(p: &mut crate::player::Player, cx: &mut Cx, buttons: i32, old_buttons: i32) {
    let use_bit = IN_USE as i32;
    let pressed = buttons & use_bit != 0 && old_buttons & use_bit == 0;
    let held = buttons & use_bit != 0;
    if !pressed && !held {
        return;
    }
    if drive::player_use(p, cx, buttons & !old_buttons) {
        return;
    }
    let player = cx.edict;
    let eng = engine();
    let pv = player.v();
    let (fwd, _, _) = eng.make_vectors(pv.v_angle);
    let src = goldsrc::ext::entities::vec::add(pv.origin, pv.view_ofs);
    let mut best = std::ptr::null_mut();
    let mut best_dot = 0.7; // VIEW_FIELD_NARROW
    let mut e = std::ptr::null_mut();
    loop {
        e = eng.find_in_sphere(e, pv.origin, 64.0);
        if e.is_null() {
            break;
        }
        let caps = object_caps(e);
        if caps & (FCAP_IMPULSE_USE | FCAP_CONTINUOUS_USE | FCAP_ONOFF_USE) == 0 {
            continue;
        }
        let ev = e.v();
        let center = goldsrc::ext::entities::vec::mad(ev.absmin, ev.size, 0.5);
        let to = goldsrc::ext::entities::vec::normalize(goldsrc::ext::entities::vec::sub(center, src));
        let d = goldsrc::ext::entities::vec::dot(to, fwd);
        if d > best_dot {
            best = e;
            best_dot = d;
        }
    }
    if best.is_null() {
        return;
    }
    let caps = object_caps(best);
    if (held && caps & FCAP_CONTINUOUS_USE != 0) || (pressed && caps & (FCAP_IMPULSE_USE | FCAP_ONOFF_USE) != 0) {
        let kind = if caps & FCAP_CONTINUOUS_USE != 0 { crate::world::UseType::Set } else { crate::world::UseType::Toggle };
        crate::world::dispatch(best, |ent, cx| ent.use_(cx, player, player, kind, 1.0));
    }
}
