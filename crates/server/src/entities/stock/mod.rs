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
pub mod impact;
pub mod items;
pub mod levelchange;
pub mod lights;
pub mod maprules;
pub mod monsters;
pub mod playeritem;
pub mod sentences;
pub mod sound;
pub mod soundent;
pub mod tank;
pub mod trackchange;
pub mod tracktrain;
pub mod trains;
pub mod triggers;
pub mod xen;

use super::link_entities;
use crate::world::{Cx, Entity};
use goldsrc::server::engine;
use goldsrc::sys::*;
use shared::pm::plus_double;
use std::ffi::CStr;

use bmodels::*;
use breakable::{FuncBreakable, FuncPushable};
use buttons::{Button, ButtonTarget, MultiSource};
use doors::Door;
use effects::*;
use goldsrc::edict::EdictPtr;
use items::{Antidote, ArmorCharger, Battery, HealthCharger, HealthKit};
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
    info_intermission         => InfoIntermission,
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
    cycler                    => GenericCycler,
    // items.cpp / healthkit.cpp / maprules.cpp
    item_healthkit            => HealthKit,
    item_battery              => Battery,
    item_antidote             => Antidote,
    game_text                 => GameText,
    game_player_equip         => GamePlayerEquip,
    game_team_master          => maprules::GameTeamMaster,
    game_team_set             => maprules::GameTeamSet,
    game_player_team          => maprules::GamePlayerTeam,
    game_score                => maprules::GameScore,
    game_end                  => maprules::GameEnd,
    game_zone_player          => maprules::GamePlayerZone,
    game_player_hurt          => maprules::GamePlayerHurt,
    game_counter              => maprules::GameCounter,
    game_counter_set          => maprules::GameCounterSet,
    func_healthcharger        => HealthCharger,
    func_recharge             => ArmorCharger,
    env_global                => EnvGlobal,
    env_message               => EnvMessage,
    // #95: effects.cpp / h_cycler.cpp / items.cpp / sound.cpp / soundent.cpp / weapons.cpp / world.cpp
    beam                      => Beam,
    env_lightning             => Lightning,
    test_effect               => TestEffect,
    env_funnel                => EnvFunnel,
    env_beverage              => EnvBeverage,
    item_sodacan              => ItemSoda,
    cycler_prdroid            => CyclerProbe,
    cycler_wreckage           => Wreckage,
    cycler_weapon             => playeritem::WeaponCycler,
    item_suit                 => items::Suit,
    item_security             => items::Security,
    world_items               => items::WorldItem,
    speaker                   => sound::Speaker,
    soundent                  => soundent::SoundEnt,
    bodyque                   => Corpse,
    weaponbox                 => playeritem::WeaponBox,
    weapon_tsgun              => playeritem::WorldTsGun,
    info_breakable            => breakable::InfoBreakable,
    // #95: triggers.cpp / player.cpp extras
    trigger                   => Trigger<K_BASE>,
    trigger_autosave          => Trigger<K_AUTOSAVE>,
    trigger_cdaudio           => Trigger<K_CDAUDIO>,
    target_cdaudio            => TargetCdAudio,
    trigger_changelevel       => levelchange::ChangeLevel,
    trigger_transition        => levelchange::TriggerVolume,
    trigger_counter           => Trigger<K_COUNTER>,
    trigger_endsection        => Trigger<K_ENDSECTION>,
    trigger_monsterjump       => Trigger<K_MONSTERJUMP>,
    player_loadsaved          => levelchange::RevertSaved,
    player_weaponstrip        => levelchange::StripWeapons,
    func_monsterclip          => FuncMonsterClip,
    // #95: plats.cpp track changes / platrot / guntarget
    func_platrot              => trains::FuncPlatRot,
    func_trackchange          => trackchange::FuncTrackChange<false>,
    func_trackautochange      => trackchange::FuncTrackChange<true>,
    func_guntarget            => trains::GunTarget,
    // #95: momentary door / rot button
    momentary_door            => doors::MomentaryDoor,
    momentary_rot_button      => buttons::MomentaryRotButton,
    // #95: subs.cpp / triggers.cpp helpers TS also exports
    DelayedUse                => base::DelayedUse,
    fireanddie                => levelchange::FireAndDie,
}

/// `worldspawn` spawnflags (SDK `SF_WORLD_*`); `CWorld::KeyValue` sets them from keys.
pub const SF_WORLD_DARK: i32 = 1;
pub const SF_WORLD_TITLE: i32 = 2;
pub const SF_WORLD_FORCETEAM: i32 = 4;

/// `CWorld::Precache` (0xf449c): `sv_zmax` when the map has no positive `MaxRange`.
pub const DEFAULT_ZMAX: f32 = 4096.0;

/// What a `CWorld::KeyValue` (0xf4a18) key does, parsed without touching the engine.
#[derive(Debug, PartialEq)]
enum WorldKey<'a> {
    /// `skyname`: `sv_skyname`.
    SkyName(&'a CStr),
    /// `sounds`: `gpGlobals->cdAudioTrack`.
    CdTrack(i32),
    /// `WaveHeight`: `pev->scale` (value / 8), also copied to `sv_wateramp`.
    WaveHeight(f32),
    /// `MaxRange`: `pev->speed`, which `Precache` copies to `sv_zmax`.
    MaxRange(f32),
    /// `chaptertitle`: `pev->netname`.
    ChapterTitle(&'a CStr),
    /// `startdark`/`gametitle`/`defaultteam`: spawnflags bits to set (0 when the value is 0).
    Flags(i32),
    /// `newunit`: sets `sv_newunit 1` when non-zero.
    NewUnit(bool),
}

fn parse_world_key<'a>(key: &[u8], value: &'a CStr) -> Option<WorldKey<'a>> {
    let flag = |bit: i32| WorldKey::Flags(if base::kv_i(value) != 0 { bit } else { 0 });
    Some(match key {
        b"skyname" => WorldKey::SkyName(value),
        b"sounds" => WorldKey::CdTrack(base::kv_i(value)),
        b"WaveHeight" => WorldKey::WaveHeight(base::kv_f(value) * 0.125),
        b"MaxRange" => WorldKey::MaxRange(base::kv_f(value)),
        b"chaptertitle" => WorldKey::ChapterTitle(value),
        b"startdark" => flag(SF_WORLD_DARK),
        b"gametitle" => flag(SF_WORLD_TITLE),
        b"defaultteam" => flag(SF_WORLD_FORCETEAM),
        b"newunit" => WorldKey::NewUnit(base::kv_i(value) != 0),
        _ => return None,
    })
}

/// `CWorld::Precache` (0xf449c): the far clip distance for a `MaxRange` (`pev->speed`).
fn zmax_for(max_range: f32) -> f32 {
    if max_range <= 0.0 { DEFAULT_ZMAX } else { max_range }
}

/// Sets a cvar that only clients register (`v_dark`, `room_type`). A dedicated server has no such
/// cvar and FWGS prints "variable not found" on every set, so skip it there; the effect is the
/// same as TS's unconditional set.
fn set_client_cvar(name: &CStr, v: f32) {
    let eng = engine();
    if !eng.is_dedicated() {
        eng.set_cvar_float(name, v);
    }
}

#[derive(Default)]
pub struct World;

impl Entity for World {
    /// `CWorld::KeyValue` (0xf4a18). The game-rules keys (`gamemode`, `mapteams`, `teammodels`)
    /// go to `rules::world_key_value`.
    fn key_value(&mut self, cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        let Some(k) = parse_world_key(key.to_bytes(), value) else {
            return crate::rules::world_key_value(key, value);
        };
        let eng = engine();
        match k {
            WorldKey::SkyName(v) => eng.set_cvar_cstr(c"sv_skyname", v),
            WorldKey::CdTrack(t) => eng.set_cd_audio_track(t),
            WorldKey::WaveHeight(scale) => {
                cx.pev().scale = scale;
                eng.set_cvar_float(c"sv_wateramp", scale);
            }
            WorldKey::MaxRange(r) => cx.pev().speed = r,
            WorldKey::ChapterTitle(v) => cx.pev().netname = base::alloc(v),
            WorldKey::Flags(bits) => cx.pev().spawnflags |= bits,
            WorldKey::NewUnit(on) => {
                if on {
                    eng.set_cvar_float(c"sv_newunit", 1.0);
                }
            }
        }
        true
    }

    /// `CWorld::Spawn` (0xf4450): `Precache`, then `g_flWeaponCheat = CVAR_GET_FLOAT("sv_cheats")`.
    fn spawn(&mut self, cx: &mut Cx) {
        self.precache(cx);
        crate::rules::cvars::latch_weapon_cheat();
    }

    /// `CWorld::Precache` (0xf449c), which `DispatchRestore` runs for a restored world: the game
    /// rules, the soundent, the body queue and every precache come back as for a new map. The
    /// world saves only `CBaseEntity`'s part and sets no member pointer.
    fn restored(&mut self, cx: &mut Cx) {
        self.precache(cx);
    }
}

impl World {
    /// `CWorld::Precache` (0xf449c).
    fn precache(&mut self, cx: &mut Cx) {
        let eng = engine();
        // Precache forces the movement defaults first. The server config that the game rules
        // queue (`rules::install`, the `CHalfLifeMultiplay` constructor) runs after the map has
        // loaded, so a `sv_gravity` in server.cfg still wins, as in TS.
        eng.set_cvar_cstr(c"sv_gravity", c"800");
        eng.set_cvar_cstr(c"sv_stepsize", c"18");
        set_client_cvar(c"room_type", 0.0);
        // A new map: per-map registries (masters, env_global states) start empty.
        base::reset_registries();
        maprules::reset();
        sound::reset_rooms();
        // CWorld::Precache (0xf449c) in TS's order: InstallGameRules (the multiplayer rules
        // precache the voice icon), then the soundent and the body queue, the first edicts after
        // the clients, so map entities get TS's edict numbers.
        crate::rules::install();
        soundent::create_world_soundent();
        init_body_que();
        // SENTENCEG_Init (sentences.txt, once per process) and TEXTURETYPE_Init.
        sentences::init();
        crate::stunt::texture_type_init();
        // W_Precache, ClientPrecache, TSPrecache, the world's sounds and the gib models: the
        // engine numbers precaches in this order (`crate::precache`).
        crate::precache::w_precache();
        crate::precache::client_precache();
        crate::precache::ts_precache();
        crate::precache::world_sounds_and_gibs();
        // Port addition: the gameplay rulesets' own precaches, so any can become active mid-map.
        crate::gameplay::precache_rulesets();
        // Port addition: the `tsmod_*` plugins' precaches, after the game's.
        crate::plugins::precache();
        light_styles();

        // The rest of CWorld::Precache.
        let (speed, netname, flags) = {
            let v = cx.pev();
            (v.speed, std::mem::take(&mut v.netname), v.spawnflags)
        };
        // A movevar: the engine sends it to clients, which use it as the far clip plane.
        eng.set_cvar_float(c"sv_zmax", zmax_for(speed));
        if netname != 0 {
            eng.alert(ALERT_TYPE::at_aiconsole, &format!("Chapter title: {}\n", base::s(netname).to_string_lossy()));
            // An env_message that shows the title once, 0.3 s in (its think is SUB_CallUseToggle).
            let m = base::create_entity(c"env_message", [0.0; 3], [0.0; 3], std::ptr::null_mut());
            if !m.is_null() {
                let v = m.v();
                v.message = netname;
                v.spawnflags = triggers::SF_MESSAGE_ONCE;
                v.nextthink = plus_double(eng.time(), 0.3);
            }
        }
        set_client_cvar(c"v_dark", if flags & SF_WORLD_DARK != 0 { 1.0 } else { 0.0 });
        // TS also sets gDisplayTitle from SF_WORLD_TITLE, but nothing reads it (the GameTitle
        // message is registered and never sent), so the port keeps only the spawnflag.
        eng.set_cvar_float(c"mp_defaultteam", if flags & SF_WORLD_FORCETEAM != 0 { 1.0 } else { 0.0 });
    }
}

/// The light styles `CWorld::Precache` sets. Style 0 is normal light; without it the whole map
/// renders black. Styles 1-12 are the SDK's animated presets (world.cpp), 63 is the testing style.
fn light_styles() {
    let eng = engine();
    eng.light_style(0, c"m");
    for (i, s) in LIGHT_STYLES.iter().enumerate() {
        eng.light_style(i as i32 + 1, s);
    }
    eng.light_style(63, c"a");
}

/// The world's part of a new game on the loaded map, for a gameplay switch's reset
/// (`crate::gameplay`): what [`World`]'s `CWorld::Precache` resets and creates, in its order, without
/// the precaches (the engine has them already), the forced movement cvars and the configs (a
/// switch keeps the server's settings), or the chapter title. Worldspawn itself stays, with its
/// keyvalues, and so does the world's soundent, spawned again in place.
pub fn restart_world() {
    set_client_cvar(c"room_type", 0.0);
    base::reset_registries();
    maprules::reset();
    sound::reset_rooms();
    crate::rules::start_rules();
    // `W_Precache`'s `compile_weapons_info`: no custom weapon parameters carry over.
    crate::weapons::rebuild_weapon_info();
    soundent::restart_world_soundent();
    init_body_que();
    light_styles();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The stock entities' `time + 0.1` (0.2, 0.3, 0.05, 0.62, 0.56) timers add the SDK's `double`
    /// literal and round once (issue #169; e.g. `CSprite::AnimateThink` mp.dll 0x1001b79c, `.so`
    /// 0xbbdef). Each input rounds one ulp higher with the `float` constant.
    #[test]
    fn stock_timers_add_the_double_constant() {
        for (t, c, new) in [
            (0x3c19_2fb5, 0.1, 0x3ddf_f2c3),
            (0x3e51_b1d6, 0.2, 0x3ecf_3f51),
            (0x3f91_a0a1, 0.3, 0x3fb8_0707),
            (0x3d28_faae, 0.05, 0x3dba_e3bd),
            (0x4043_429b, 0.62, 0x406a_f0af),
            (0x3eb1_888d, 0.56, 0x3f68_206f),
        ] {
            let t = f32::from_bits(t);
            assert_eq!(plus_double(t, c).to_bits(), new, "{c}");
            assert_eq!((t + c as f32).to_bits(), new + 1, "{c}");
        }
    }

    #[test]
    fn world_keys() {
        assert_eq!(parse_world_key(b"skyname", c"des"), Some(WorldKey::SkyName(c"des")));
        assert_eq!(parse_world_key(b"sounds", c"3"), Some(WorldKey::CdTrack(3)));
        assert_eq!(parse_world_key(b"WaveHeight", c"3.2"), Some(WorldKey::WaveHeight(0.4)));
        assert_eq!(parse_world_key(b"WaveHeight", c".2"), Some(WorldKey::WaveHeight(0.025)));
        assert_eq!(parse_world_key(b"MaxRange", c"16384"), Some(WorldKey::MaxRange(16384.0)));
        assert_eq!(parse_world_key(b"chaptertitle", c"T0A0TITLE"), Some(WorldKey::ChapterTitle(c"T0A0TITLE")));
        assert_eq!(parse_world_key(b"startdark", c"1"), Some(WorldKey::Flags(SF_WORLD_DARK)));
        assert_eq!(parse_world_key(b"startdark", c"0"), Some(WorldKey::Flags(0)));
        assert_eq!(parse_world_key(b"gametitle", c"1"), Some(WorldKey::Flags(SF_WORLD_TITLE)));
        assert_eq!(parse_world_key(b"defaultteam", c"2"), Some(WorldKey::Flags(SF_WORLD_FORCETEAM)));
        assert_eq!(parse_world_key(b"newunit", c"1"), Some(WorldKey::NewUnit(true)));
        assert_eq!(parse_world_key(b"newunit", c"0"), Some(WorldKey::NewUnit(false)));
        // Keys are case-sensitive (strcmp); the rules keys are left to rules::world_key_value.
        assert_eq!(parse_world_key(b"maxrange", c"8192"), None);
        assert_eq!(parse_world_key(b"gamemode", c"1"), None);
    }

    #[test]
    fn zmax_default() {
        assert_eq!(zmax_for(0.0), DEFAULT_ZMAX);
        assert_eq!(zmax_for(-1.0), DEFAULT_ZMAX);
        assert_eq!(zmax_for(17096.0), 17096.0);
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

/// `InitBodyQue` (0xf3ba4): four `bodyque` corpses linked into a ring through `owner`. TS's
/// only user, `CopyToBodyQue` (0xf3c70, from `CBasePlayer::StartDeathCam`), is never called,
/// so they stay empty.
fn init_body_que() {
    let eng = engine();
    let head = eng.create_named_entity(c"bodyque");
    let mut prev = head;
    for _ in 0..3 {
        let e = eng.create_named_entity(c"bodyque");
        if !prev.is_null() {
            prev.v().owner = e;
        }
        prev = e;
    }
    if !prev.is_null() {
        prev.v().owner = head;
    }
}

/// `bodyque` (`CCorpse`): no behaviour of its own (`CBaseEntity::Spawn`), only `ObjectCaps`.
#[derive(Default)]
pub struct Corpse;
impl Entity for Corpse {}

/// Spawn points (`CBaseDMStart`: `info_player_deathmatch`, `info_player_start`,
/// `info_player_team1..4`) keep their origin/angles in pev; the rules pick among them.
#[derive(Default)]
pub struct SpawnPoint;
impl Entity for SpawnPoint {
    /// `CBaseDMStart::KeyValue` (0xe2bb8): `master` goes to `pev->netname`, which
    /// `CBaseDMStart::IsTriggered` (0xe2c20) hands to `UTIL_IsMasterTriggered`.
    fn key_value(&mut self, cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        if key != c"master" {
            return false;
        }
        cx.pev().netname = base::alloc(value);
        true
    }
}

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
        // CMomentaryRotButton::ObjectCaps (0xaa9b4); momentary_door's (0xb87f4) is 0.
        b"momentary_rot_button" => {
            if v.spawnflags & buttons::SF_MOMENTARY_DOOR != 0 {
                0
            } else {
                FCAP_CONTINUOUS_USE
            }
        }
        // CFuncTankControls::ObjectCaps (0xc4bc8); trains are driven through `drive`.
        b"func_tankcontrols" => FCAP_IMPULSE_USE,
        b"func_tracktrain" => FCAP_DIRECTIONAL_USE,
        // CCycler::ObjectCaps (0xc8654) / CCyclerSprite::ObjectCaps (0xc87b0): TS's impulse bit.
        b"cycler" | b"cycler_sprite" => FCAP_IMPULSE_USE,
        _ => 0,
    }
}
