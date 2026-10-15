//! TS map entities (ts_powerup, ts_groundweapon, ts_model, ...). Owner: rules work package.
//! Reference: decomp docs/server/gamerules-entities.md §3-6 (reconstructed FGD in §6),
//! architecture.md §5 (classname table, HL remaps).

#![allow(non_snake_case)] // weapon_9MMAR & co. are case-sensitive export names

pub mod ctb;
pub mod ground;
pub mod misc;
pub mod objective;
pub mod powerup;
pub mod shrapnel;
pub mod triggers;

use super::link_entities;
use crate::player::Player;
use crate::world::Cx;
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::ffi::CStr;

link_entities! {
    // power-ups
    ts_powerup => powerup::PowerUp,
    item_longjump => powerup::PowerUp,
    // ground items and the HL classnames TS re-points at them
    ts_groundweapon => ground::GroundWeapon,
    weapon_crossbow => ground::GroundWeapon,
    weapon_shotgun => ground::GroundWeapon,
    weapon_gauss => ground::GroundWeapon,
    weapon_egon => ground::GroundWeapon,
    weapon_rpg => ground::GroundWeapon,
    weapon_9MMAR => ground::GroundWeapon,
    weapon_hornetgun => ground::GroundWeapon,
    weapon_satchel => ground::GroundWeapon,
    weapon_snark => ground::GroundWeapon,
    weapon_handgrenade => ground::GroundWeapon,
    ammo_ARgrenades => ground::GroundWeapon,
    ts_groundammo => ground::GroundAmmo,
    ammo_9mmAR => ground::GroundAmmo,
    ammo_buckshot => ground::GroundAmmo,
    ammo_crossbow => ground::GroundAmmo,
    ammo_gaussclip => ground::GroundAmmo,
    ammo_rpgclip => ground::GroundAmmo,
    // map setup and effects
    ts_model => misc::Model,
    ts_mapglobals => misc::MapGlobals,
    ts_wind => misc::Wind,
    ts_fog => misc::Fog,
    ts_emitter => misc::Emitter,
    ts_particle => misc::Particle,
    deadcorpse => misc::DeadCorpse,
    info_player_team1 => crate::entities::stock::SpawnPoint,
    info_player_team2 => crate::entities::stock::SpawnPoint,
    info_player_team3 => crate::entities::stock::SpawnPoint,
    info_player_team4 => crate::entities::stock::SpawnPoint,
    // logic
    ts_trigger => triggers::Trigger,
    ts_dmhill => triggers::DmHill,
    ts_timer => triggers::Timer,
    ts_customtarget => triggers::CustomTarget,
    ts_playeramount => triggers::PlayerAmount,
    ts_slowmotion => triggers::SlowMotion,
    ts_slowmotionpoint => triggers::SlowMotionPoint,
    // capture the briefcase
    ts_briefcase => ctb::Briefcase,
    ts_flag => ctb::Flag,
    ts_wingiver => ctb::WinGiver<false>,
    ts_scoregiver => ctb::WinGiver<true>,
    // objectives
    ts_objective_manager => objective::ObjectiveManager,
    ts_objective_ptr => objective::ObjectivePointer,
    ts_bomb => objective::Bomb,
    ts_hack => objective::Hack,
    ts_teamescape => objective::TeamEscape,
    ts_sellzone => misc::SellZone,
    // #95: CTSShrapnel (only a map can place one: nothing calls CTSShrapnel::Create)
    shrapnel => shrapnel::Shrapnel,
}

/// `CBaseEntity::Save` / `Restore` (0xabe40 / 0xabe9c) for a class with no goal entity:
/// `"ENTVARS"` and `"BASE"` with TS's member pointers `funcs` (which of `m_pfnThink` /
/// `m_pfnTouch` / `m_pfnUse` / `m_pfnBlocked` TS's object has set for the port's state). A restore
/// leaves them null (`ReadFields` zeroes every field first, and a set one is never written), so
/// they are cleared here and the class's think / touch, which check them, then do nothing, as
/// TS's `CBaseEntity::Think` / `Touch` do with a null pointer.
pub fn save_base(cx: &mut Cx, f: &mut crate::save::Fields, funcs: &mut crate::save::BaseFuncs) {
    let cleared = f.base_funcs(cx, *funcs);
    funcs.think &= !cleared.think;
    funcs.touch &= !cleared.touch;
    funcs.use_ &= !cleared.use_;
    funcs.blocked &= !cleared.blocked;
}

/// The `CBaseAnimating` part of the port's animation state, as the save visitor takes it.
pub fn anim_to_save(a: &crate::entities::stock::animating::Animating) -> crate::save::Animating {
    crate::save::Animating {
        frame_rate: a.frame_rate,
        ground_speed: a.ground_speed,
        last_event_check: a.last_event_check,
        sequence_finished: a.finished,
        sequence_loops: a.loops,
    }
}

/// Back from [`anim_to_save`] (the activity isn't part of `CBaseAnimating`'s table).
pub fn anim_from_save(a: &mut crate::entities::stock::animating::Animating, s: &crate::save::Animating) {
    a.frame_rate = s.frame_rate;
    a.ground_speed = s.ground_speed;
    a.last_event_check = s.last_event_check;
    a.finished = s.sequence_finished;
    a.loops = s.sequence_loops;
}

/// `"CBaseAnimating"` over the port's animation state.
pub fn save_animating(f: &mut crate::save::Fields, a: &mut crate::entities::stock::animating::Animating) {
    let mut s = anim_to_save(a);
    f.animating(&mut s);
    anim_from_save(a, &s);
}

/// `"CBaseDelay"`, `"CBaseAnimating"` and `"CBaseToggle"` of a TS class built on `CBaseToggle`
/// that never sets any of their fields (its `KeyValue` doesn't fall back to
/// `CBaseToggle::KeyValue`, and nothing else writes them): all empty, as in TS's zero-filled
/// object.
pub fn save_unused_toggle(f: &mut crate::save::Fields) {
    for t in [&crate::save::tables::CBaseDelay, &crate::save::tables::CBaseAnimating, &crate::save::tables::CBaseToggle] {
        f.table(t);
        for _ in 0..t.fields.len() {
            f.zero();
        }
    }
}

/// A C `(int)` cast as GCC 2.95 compiles it (`fldcw` round-toward-zero, `fistpl`) on the x87's
/// extended value: truncation, and the integer indefinite (`i32::MIN`) for NaN or out of range.
pub fn c_int(v: f64) -> i32 {
    if (-2_147_483_648.0..2_147_483_648.0).contains(&v) { v as i32 } else { i32::MIN }
}

/// Per-map state of the TS entities (the `ts_particle` count). Their map-wide models
/// (`ts_powerup`, `ts_flag`, the ammo pack) are precached by `crate::precache` where TS's
/// `W_Precache` / `TSPrecache` do.
pub fn map_init() {
    misc::PARTICLES.0.set(0);
}

/// `ts_playeramount` hooks from ClientConnect/ClientDisconnect.
pub fn player_amount_changed(connected: bool) {
    let eng = engine();
    let mut it = std::ptr::null_mut();
    let mut list = Vec::new();
    loop {
        it = eng.find_by_string(it, c"classname", c"ts_playeramount");
        if it.is_null() {
            break;
        }
        list.push(it);
    }
    for e in list {
        crate::world::dispatch(e, |ent, cx| {
            if let Some(p) = (ent as &mut dyn std::any::Any).downcast_mut::<triggers::PlayerAmount>() {
                p.changed(cx, connected);
            }
        });
    }
}

pub fn drop_flag(p: &mut Player, cx: &mut Cx, captured: bool) {
    ctb::drop_flag(p, cx, captured)
}

pub fn has_flag(e: *mut edict_t) -> bool {
    ctb::has_flag(e)
}

pub fn create_dead_corpse(player: *mut edict_t, life: f32) {
    misc::create_dead_corpse(player, life)
}

pub fn create_slow_motion_point(origin: [f32; 3], amount: f32, duration: f32, btime: bool) {
    triggers::create_slow_motion_point(origin, amount, duration, btime)
}

pub fn create_powerup(kind: u16, secs: i32, origin: [f32; 3]) {
    powerup::create(kind, secs, origin);
}

/// Ground weapon classnames (`ts_groundweapon` and its HL remaps); bots walk to these.
pub fn is_usable(class: &CStr) -> bool {
    class == c"ts_groundweapon" || ground::hl_weapon_ids(class).is_some()
}

/// `ObjectCaps()` of the TS entities, for the player's +use search (`player::player_use`):
/// all of them are `FCAP_CONTINUOUS_USE` (0x10). Ground weapons (`CTSGroundWeapon::ObjectCaps`
/// 0xa34a0), ground ammo (0xa34f4, whose `Use` 0xa2f3c is empty: it is picked up on touch, but
/// still takes the +use search), dropped guns (`CTSWorldGun` 0xa3404), thrown blades (`CTSKnife`
/// 0x7f04c), grenades (`CM61Grenade` 0x7effc; classname `grenade`), shrapnel (`CTSShrapnel`
/// 0x7f09c, no `Use`), the CTB case (`CTSFlag` 0x9e704) and the objectives (`CTSBomb` 0x8e738, `CTSHack` 0x8e788,
/// `CTSTeamEscape` 0x8e820). 0 for anything else.
pub fn object_caps(class: &CStr) -> i32 {
    let usable = matches!(
        class.to_bytes(),
        b"ts_groundweapon"
            | b"ts_groundammo"
            | b"WorldGun"
            | b"knife"
            | b"grenade"
            | b"shrapnel"
            | b"ts_flag"
            | b"ts_bomb"
            | b"ts_hack"
            | b"ts_teamescape"
    ) || ground::hl_weapon_ids(class).is_some()
        || ground::hl_ammo_types(class).is_some();
    if usable { crate::entities::stock::FCAP_CONTINUOUS_USE } else { 0 }
}

/// `CanUsedTroughWalls()` (vtable slot 1; `CBaseEntity` 0xac5b0 returns 1): false for ground
/// weapons (0xa34a8), ground ammo (0xa34fc), dropped guns (0xa340c) and the CTB case (0x9e70c),
/// which `PlayerUse` only reaches with a clear line from the eyes.
pub fn can_use_through_walls(class: &CStr) -> bool {
    !(matches!(class.to_bytes(), b"ts_groundweapon" | b"ts_groundammo" | b"WorldGun" | b"ts_flag")
        || ground::hl_weapon_ids(class).is_some()
        || ground::hl_ammo_types(class).is_some())
}

/// The `Use` of a ground weapon (`CTSGroundWeapon::Use` 0xa230c → `GetWeapon`) or the CTB case
/// (`CTSFlag::Use` 0x9e5dc → `PlayerTakeFlag`) when a player's +use picks it; the player is being
/// dispatched, so it is passed directly. False if `ent` is neither.
pub fn use_by_player(ent: &mut dyn std::any::Any, icx: &mut Cx, p: &mut Player, cx: &mut Cx) -> bool {
    if let Some(g) = ent.downcast_mut::<ground::GroundWeapon>() {
        g.player_use(icx, p, cx);
    } else if let Some(f) = ent.downcast_mut::<ctb::Flag>() {
        f.use_by_player(icx, p, cx);
    } else {
        return false;
    }
    true
}

/// `UTIL_FindEntityByClassname` over `class`, dispatching each match (the next one is looked up
/// after the call, as TS's loops do, so an entity removed by the call is still stepped past).
pub fn for_each_class(class: &CStr, mut f: impl FnMut(&mut dyn std::any::Any, &mut Cx)) {
    let eng = engine();
    let mut it = std::ptr::null_mut();
    loop {
        it = eng.find_by_string(it, c"classname", class);
        if it.is_null() {
            return;
        }
        crate::world::dispatch(it, |ent, cx| f(ent as &mut dyn std::any::Any, cx));
    }
}
