//! The impulses `CBasePlayer::ImpulseCommands` (0xdb780) handles besides +use, the `Logo` toggle
//! and `ForceClientDllUpdate` (`player_use.rs`): the spray (201, `CSprayCan`) and the SDK
//! developer impulses of `CBasePlayer::CheatImpulseCommands` (0xdb9e4), gated by
//! `g_flWeaponCheat` (`rules::cvars::cheats`). TS 3.0 dropped the SDK's give-everything 101.
//! The custom spray's frame count comes from `PlayerCustomization` (0xaeef0).

use super::Player;
use crate::entities::stock::base::{GlobalState, create_entity, global_list};
use crate::entities::stock::blood::{BLOOD_COLOR_RED, blood_decal_trace};
use crate::entities::stock::effects::{Gib, decal_trace};
use crate::entities::stock::explode::emit_sound_slow;
use crate::world::{self, Cx, Entity};
use goldsrc::edict::EdictPtr;
use goldsrc::ext::entities::{ATTN_NORM, MsgWriterExt, te, te_begin};
use goldsrc::server::{DONT_IGNORE_MONSTERS, IGNORE_MONSTERS, engine};
use goldsrc::sys::*;
use shared::weapons::spread::{self, Vec3};

/// `CBasePlayer` fields the impulses use.
#[derive(Clone, Copy, Debug)]
pub struct ImpulseState {
    /// `m_flNextDecalTime` (pl+0xb44): the next spray; 0 on every spawn.
    pub next_decal_time: f32,
    /// `m_nCustomSprayFrames` (pl+0xb40): frames of the player's uploaded spray, -1 for none
    /// (`ClientPutInServer` sets -1).
    pub custom_decal_frames: i32,
    /// `m_fNoPlayerSound` (pl+0x9d0), impulse 105: the player's `soundent` slot gets volume 0
    /// ([`update_player_sound`]). Only monsters would hear it, and TS has none.
    pub silent: bool,
    /// `m_iWeaponVolume` (pl+0x7f8): only `UpdatePlayerSound` writes it.
    pub weapon_volume: i32,
}

impl Default for ImpulseState {
    fn default() -> Self {
        ImpulseState { next_decal_time: 0.0, custom_decal_frames: -1, silent: false, weapon_volume: 0 }
    }
}

/// The `CBasePlayer::Spawn` (0x8f568) part: `m_flNextDecalTime = 0`, `m_fNoPlayerSound = 0`.
pub fn on_spawn(p: &mut Player) {
    p.impulse.next_decal_time = 0.0;
    p.impulse.silent = false;
}

/// `CBasePlayer::UpdatePlayerSound` (0xda630), near the end of a live player's `PostThink`
/// (0x91aa6): the player's reserved `soundent` slot.
pub fn update_player_sound(p: &mut Player, cx: &mut Cx) {
    use crate::entities::stock::soundent::{PlayerSoundInput, update_player_sound};
    let idx = cx.index();
    let v = cx.pev();
    let inp = PlayerSoundInput {
        origin: v.origin,
        velocity: v.velocity,
        on_ground: v.flags & FL_ONGROUND as i32 != 0,
        jumping: v.button & IN_JUMP as i32 != 0,
        silent: p.impulse.silent,
        frametime: engine().globals().frametime,
    };
    update_player_sound(idx, &mut p.impulse.weapon_volume, &inp);
}

/// `CBasePlayer::SetCustomDecalFrames` (0xdd488): 1..=7 frames are kept, anything else is -1.
pub fn custom_decal_frames(frames: i32) -> i32 {
    if (1..=7).contains(&frames) { frames } else { -1 }
}

/// `t_decal` (`resourcetype_t`).
const T_DECAL: i32 = 3;

/// What `PlayerCustomization` (0xaeef0) does with a customization of `kind` (`resource.type`):
/// `Some(true)` sets the spray frames, `Some(false)` ignores it (sounds, skins, models), `None`
/// is an unknown type.
fn customization_sets_frames(kind: i32) -> Option<bool> {
    match kind {
        0..T_DECAL => Some(false),
        T_DECAL => Some(true),
        _ => None,
    }
}

/// `PlayerCustomization` (0xaeef0): a client's uploaded logo sets its spray's frame count
/// (`nUserData2`). `custom` is `(resource.type, nUserData2)`, `None` for a NULL customization.
pub fn player_customization(e: *mut edict_t, custom: Option<(i32, i32)>) {
    let eng = engine();
    // `pvPrivateData` is the player (TS doesn't check the class).
    if e.is_null() || !e.has_private_data() {
        eng.alert(ALERT_TYPE::at_console, "PlayerCustomization:  Couldn't get player!\n");
        return;
    }
    let Some((kind, frames)) = custom else {
        eng.alert(ALERT_TYPE::at_console, "PlayerCustomization:  NULL customization!\n");
        return;
    };
    match customization_sets_frames(kind) {
        Some(true) => {
            crate::player::with_player(e, |p, _| p.impulse.custom_decal_frames = custom_decal_frames(frames));
        }
        Some(false) => {}
        None => eng.alert(ALERT_TYPE::at_console, "PlayerCustomization:  Unknown customization type!\n"),
    }
}

fn eye(v: &entvars_t) -> Vec3 {
    spread::add(v.origin, v.view_ofs)
}

/// Impulse 201 in `ImpulseCommands`: once `m_flNextDecalTime` has passed, a wall within 128
/// units of the eyes (monsters ignored) gets a spray, and the next one waits `decalfrequency`.
pub fn spray(p: &mut Player, cx: &mut Cx) {
    let eng = engine();
    let now = eng.time();
    if p.impulse.next_decal_time > now {
        return;
    }
    let me = cx.edict;
    let (start, v_angle) = {
        let v = cx.pev();
        (eye(v), v.v_angle)
    };
    let (fwd, _, _) = eng.make_vectors(v_angle);
    let tr = eng.trace_line(start, spread::add(start, spread::scale(fwd, 128.0)), IGNORE_MONSTERS, me);
    if tr.flFraction != 1.0 {
        p.impulse.next_decal_time = now + eng.cvar_float(c"decalfrequency");
        SprayCan::create(me);
    }
}

/// A class-less entity (`GetClassPtr<T>(NULL)`: a fresh edict, no classname) placed like
/// `CSprayCan::Spawn` / `CBloodSplat::Spawn`: 32 units above the player's origin, facing the
/// player's view, owned by the player, thinking in 0.1 s.
fn create_sprayer<T: Entity + Default>(player: *mut edict_t) -> *mut edict_t {
    let eng = engine();
    let e = eng.create_entity();
    if e.is_null() {
        return e;
    }
    world::create::<T>(e.v());
    let (origin, v_angle) = {
        let pv = player.v();
        (pv.origin, pv.v_angle)
    };
    let v = e.v();
    v.origin = spread::add(origin, [0.0, 0.0, 32.0]);
    v.angles = v_angle;
    v.owner = player;
    v.nextthink = eng.time() + 0.1;
    e
}

/// `CSprayCan` (0xdb0d4..0xde514): sprays the owner's logo, one frame every 0.1 s, or the
/// `{lambda06` decal when the owner has no custom spray.
#[derive(Default)]
pub struct SprayCan;

impl SprayCan {
    /// `GetClassPtr<CSprayCan>` + `CSprayCan::Spawn` (0xdb0d4).
    fn create(player: *mut edict_t) {
        let e = create_sprayer::<SprayCan>(player);
        if e.is_null() {
            return;
        }
        e.v().frame = 0.0;
        emit_sound_slow(e, CHAN_VOICE, c"player/sprayer.wav", 1.0, ATTN_NORM, 0, 100, 0.5);
    }
}

/// What one `CSprayCan::Think` does with `frames` (`GetCustomDecalFrames`) at `frame`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SprayStep {
    /// `UTIL_DecalTrace(DECAL_LAMBDA6)`, then remove.
    Lambda,
    /// `UTIL_PlayerDecalTrace` of this frame; keep thinking while frames are left.
    Frame { frame: i32, more: bool },
}

fn spray_step(frames: i32, frame: f32) -> SprayStep {
    if frames == -1 {
        SprayStep::Lambda
    } else {
        SprayStep::Frame { frame: frame.round_ties_even() as i32, more: frame < (frames - 1) as f32 }
    }
}

impl Entity for SprayCan {
    /// `CSprayCan::Think` (0xdb1c4).
    fn think(&mut self, cx: &mut Cx) {
        let eng = engine();
        let (owner, origin, angles, frame) = {
            let v = cx.pev();
            (v.owner, v.origin, v.angles, v.frame)
        };
        // An owner without a player behind it has no custom frames.
        let frames = crate::rules::other(owner, |p| p.impulse.custom_decal_frames).unwrap_or(-1);
        let playernum = eng.index_of(owner);
        let (fwd, _, _) = eng.make_vectors(angles);
        let tr = eng.trace_line(origin, spread::add(origin, spread::scale(fwd, 128.0)), IGNORE_MONSTERS, owner);
        match spray_step(frames, frame) {
            SprayStep::Lambda => {
                decal_trace(&tr, c"{lambda06");
                crate::entities::remove(cx.edict);
            }
            SprayStep::Frame { frame, more } => {
                player_decal_trace(&tr, playernum, frame);
                cx.pev().frame += 1.0;
                if !more {
                    crate::entities::remove(cx.edict);
                }
            }
        }
        cx.think_in(0.1);
    }
}

/// `UTIL_PlayerDecalTrace` (0xee028) with `bIsCustom`: `TE_PLAYERDECAL` of the player's own logo
/// frame `decal` where the trace hit.
fn player_decal_trace(tr: &TraceResult, playernum: i32, decal: i32) {
    if tr.flFraction == 1.0 {
        return;
    }
    let entity = engine().index_of(tr.pHit);
    te_begin(MSG_BROADCAST, None, te::PLAYERDECAL).byte(playernum).vec3(tr.vecEndPos).short(entity as i16 as i32).byte(decal).end();
}

/// `CBloodSplat` (0xdb360, 0xdb428): impulse 202's red blood decal, sprayed one think after it
/// spawns (not with `g_Language` 1, German), then removed one think later.
#[derive(Default)]
pub struct BloodSplat {
    sprayed: bool,
}

impl Entity for BloodSplat {
    fn think(&mut self, cx: &mut Cx) {
        if self.sprayed {
            // `SUB_Remove`: the edict is freed at once.
            crate::entities::stock::base::sub_remove(cx.edict);
            return;
        }
        // `CBloodSplat::Spray` (0xdb428).
        if splat_sprays(crate::entities::stock::blood::german()) {
            let eng = engine();
            let (owner, origin, angles) = {
                let v = cx.pev();
                (v.owner, v.origin, v.angles)
            };
            let (fwd, _, _) = eng.make_vectors(angles);
            let tr = eng.trace_line(origin, spread::add(origin, spread::scale(fwd, 128.0)), IGNORE_MONSTERS, owner);
            blood_decal_trace(&tr, BLOOD_COLOR_RED);
        }
        self.sprayed = true;
        cx.think_in(0.1);
    }
}

/// `CBloodSplat::Spray`'s `g_Language != 1` test: the German build (`sv_language 1` at map
/// start) draws no blood splat, but the entity still removes itself 0.1 s later.
fn splat_sprays(german: bool) -> bool {
    !german
}

/// `FindEntityForward` (0xdb5f4): what the player looks at within 8192 units (monsters
/// included), if it is a game entity other than the world.
fn find_entity_forward(me: *mut edict_t) -> *mut edict_t {
    let eng = engine();
    let (start, v_angle) = {
        let v = me.v();
        (eye(v), v.v_angle)
    };
    let (fwd, _, _) = eng.make_vectors(v_angle);
    let tr = eng.trace_line(start, spread::add(start, spread::scale(fwd, 8192.0)), DONT_IGNORE_MONSTERS, me);
    if tr.flFraction != 1.0 && !tr.pHit.is_null() && eng.index_of(tr.pHit) != 0 && tr.pHit.has_private_data() {
        tr.pHit
    } else {
        std::ptr::null_mut()
    }
}

/// `estates` for `DumpGlobals`.
fn global_state_name(st: GlobalState) -> &'static str {
    match st {
        GlobalState::Off => "Off",
        GlobalState::On => "On",
        GlobalState::Dead => "Dead",
    }
}

/// Impulse 106: `Classname: %s`, the target name, the model and any global name.
fn entity_info(classname: &str, targetname: Option<&str>, model: &str, globalname: Option<&str>) -> String {
    let mut s = format!("Classname: {classname}");
    match targetname {
        Some(t) => s += &format!(" - Targetname: {t}\n"),
        None => s += " - TargetName: No Targetname\n",
    }
    s += &format!("Model: {model}\n");
    if let Some(g) = globalname {
        s += &format!("Globalname: {g}\n");
    }
    s
}

/// `CBasePlayer::CheatImpulseCommands` (0xdb9e4), with `g_flWeaponCheat` on.
pub fn cheat_impulse_commands(p: &mut Player, cx: &mut Cx, impulse: i32) {
    if !crate::rules::cvars::cheats() {
        return;
    }
    let eng = engine();
    let me = cx.edict;
    let con = |s: &str| eng.alert(ALERT_TYPE::at_console, s);
    match impulse {
        // `CGib::SpawnRandomGibs(pev, 1, 1)`: one human gib (the player's `BloodColor` is red),
        // thrown against `g_vecAttackDir`, the last damage anywhere.
        102 => Gib::spawn_random_gibs(me, 1, true, BLOOD_COLOR_RED, crate::damage::attack_dir()),
        // `FindEntityForward(this)->MyMonsterPointer()->ReportAIState()`: TS's `ReportAIState`
        // (0xce69c) is empty, so only the trace remains.
        103 => {
            find_entity_forward(me);
        }
        // `gGlobalState.DumpGlobals()`.
        104 => {
            con("-- Globals --\n");
            for (name, level, st) in global_list() {
                con(&format!("{name}: {level} ({})\n", global_state_name(st)));
            }
        }
        105 => {
            p.impulse.silent = !p.impulse.silent;
            con(if p.impulse.silent { "Player is silent\n" } else { "Player is audible\n" });
        }
        106 => {
            let e = find_entity_forward(me);
            if !e.is_null() {
                let v = e.v();
                let str_of = |s: string_t| eng.string(s).to_string_lossy().into_owned();
                let target = (v.targetname != 0).then(|| str_of(v.targetname));
                let global = (v.globalname != 0).then(|| str_of(v.globalname));
                con(&entity_info(&str_of(v.classname), target.as_deref(), &str_of(v.model), global.as_deref()));
            }
        }
        // The texture 1024 units ahead, along whatever `v_forward` the last `MakeVectors` left
        // (TS doesn't make its own), traced against the hit entity or the world.
        107 => {
            let start = eye(me.v());
            let end = spread::add(start, spread::scale(eng.globals().v_forward, 1024.0));
            let tr = eng.trace_line(start, end, IGNORE_MONSTERS, me);
            let ent = if tr.pHit.is_null() { eng.entity_of_index(0) } else { tr.pHit };
            let name = eng.trace_texture(ent, start, end);
            if !name.is_empty() {
                con(&format!("Texture: {}\n", name.to_string_lossy()));
            }
        }
        // `CBaseEntity::Create` of the node viewers. Neither TS nor the port has these classes,
        // so the engine refuses and "NULL Ent in Create!" is printed.
        195..=197 => {
            let class = [c"node_viewer_fly", c"node_viewer_large", c"node_viewer_human"][(impulse - 195) as usize];
            let (origin, angles) = {
                let v = cx.pev();
                (v.origin, v.angles)
            };
            create_entity(class, origin, angles, std::ptr::null_mut());
        }
        // `CGraph::FindNearestNode` is a stub returning 0 in TS, `ShowNodeConnections` is empty.
        199 => con("0\n"),
        202 => {
            let (start, v_angle) = {
                let v = cx.pev();
                (eye(v), v.v_angle)
            };
            let (fwd, _, _) = eng.make_vectors(v_angle);
            let tr = eng.trace_line(start, spread::add(start, spread::scale(fwd, 128.0)), IGNORE_MONSTERS, me);
            if tr.flFraction != 1.0 {
                create_sprayer::<BloodSplat>(me);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blood_splat_skips_the_german_build() {
        assert!(splat_sprays(false));
        assert!(!splat_sprays(true));
    }

    #[test]
    fn set_custom_decal_frames_keeps_one_to_seven() {
        assert_eq!(custom_decal_frames(1), 1);
        assert_eq!(custom_decal_frames(7), 7);
        assert_eq!(custom_decal_frames(0), -1);
        assert_eq!(custom_decal_frames(8), -1);
        assert_eq!(custom_decal_frames(-5), -1);
        assert_eq!(custom_decal_frames(i32::MIN), -1);
        assert_eq!(ImpulseState::default().custom_decal_frames, -1, "ClientPutInServer's -1");
    }

    #[test]
    fn customization_types() {
        assert_eq!(customization_sets_frames(0), Some(false), "t_sound");
        assert_eq!(customization_sets_frames(2), Some(false), "t_model");
        assert_eq!(customization_sets_frames(3), Some(true), "t_decal");
        assert_eq!(customization_sets_frames(4), None, "t_generic is unknown");
        assert_eq!(customization_sets_frames(-1), None);
    }

    #[test]
    fn spray_frames() {
        assert_eq!(spray_step(-1, 0.0), SprayStep::Lambda);
        // Three frames: 0 and 1 keep going, 2 is the last.
        assert_eq!(spray_step(3, 0.0), SprayStep::Frame { frame: 0, more: true });
        assert_eq!(spray_step(3, 1.0), SprayStep::Frame { frame: 1, more: true });
        assert_eq!(spray_step(3, 2.0), SprayStep::Frame { frame: 2, more: false });
        assert_eq!(spray_step(1, 0.0), SprayStep::Frame { frame: 0, more: false });
        // A custom spray of 0 frames can't happen (`SetCustomDecalFrames`), but TS would spray
        // frame 0 once.
        assert_eq!(spray_step(0, 0.0), SprayStep::Frame { frame: 0, more: false });
    }

    #[test]
    fn entity_info_lines() {
        assert_eq!(entity_info("func_door", None, "*3", None), "Classname: func_door - TargetName: No Targetname\nModel: *3\n");
        assert_eq!(
            entity_info("func_button", Some("b1"), "*4", Some("g")),
            "Classname: func_button - Targetname: b1\nModel: *4\nGlobalname: g\n"
        );
    }

    #[test]
    fn global_state_names() {
        assert_eq!(global_state_name(GlobalState::Off), "Off");
        assert_eq!(global_state_name(GlobalState::On), "On");
        assert_eq!(global_state_name(GlobalState::Dead), "Dead");
    }
}
