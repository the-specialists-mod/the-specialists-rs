//! What a stock bullet does where it lands (SDK sound.cpp / weapons.cpp / util.cpp, unchanged in
//! TS 3.0): `TEXTURETYPE_PlaySound`'s impact sound, `DecalGunshot`'s decal through the hit
//! entity's `DamageDecal`, and TS's `gDecals` table they index.

use super::breakable::{self, mat};
use super::effects::decal_trace;
use goldsrc::edict::EdictPtr;
use goldsrc::ext::entities::{MsgWriterExt, Vec3, te, te_begin};
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::ffi::CStr;

/// `BULLET_PLAYER_CROWBAR` (5): `TEXTURETYPE_PlaySound` plays nothing on flesh for it, and
/// `DecalGunshot` places a plain `DMG_CLUB` decal.
pub const BULLET_PLAYER_CROWBAR: i32 = 5;

/// TS's `gDecals` (`ts_i386.so` 0x123400, `{ name, index }` pairs, the SDK's 42 entries in the
/// SDK's order): the `DECAL_*` numbers index it. TS fills `.index` with `DECAL_INDEX(name)` in
/// `W_Precache`; the port asks the engine when it needs one, which gives the same number.
pub const DECALS: [&CStr; 42] = [
    c"{shot1",
    c"{shot2",
    c"{shot3",
    c"{shot4",
    c"{shot5",
    c"{lambda01",
    c"{lambda02",
    c"{lambda03",
    c"{lambda04",
    c"{lambda05",
    c"{lambda06",
    c"{scorch1",
    c"{scorch2",
    c"{blood1",
    c"{blood2",
    c"{blood3",
    c"{blood4",
    c"{blood5",
    c"{blood6",
    c"{yblood1",
    c"{yblood2",
    c"{yblood3",
    c"{yblood4",
    c"{yblood5",
    c"{yblood6",
    c"{break1",
    c"{break2",
    c"{break3",
    c"{bigshot1",
    c"{bigshot2",
    c"{bigshot3",
    c"{bigshot4",
    c"{bigshot5",
    c"{spit1",
    c"{spit2",
    c"{bproof1",
    c"{gargstomp",
    c"{smscorch1",
    c"{smscorch2",
    c"{smscorch3",
    c"{mommablob",
    c"{mommablob",
];

/// `DECAL_GUNSHOT1` (`{shot1`; `{shot2`..`{shot5` follow).
pub const DECAL_GUNSHOT1: i32 = 0;
/// `DECAL_GLASSBREAK1` (`{break1`; `{break2`, `{break3` follow).
pub const DECAL_GLASSBREAK1: i32 = 0x19;
/// `DECAL_BPROOF1` (`{bproof1`).
pub const DECAL_BPROOF1: i32 = 0x23;

/// `CBaseEntity::DamageDecal` (0xac428), which every TS class but `CBreakable` keeps: none
/// (-1) on a `kRenderTransAlpha` entity, a random gunshot on a `kRenderNormal` one, else the
/// bulletproof decal. `random(lo, hi)` is `RANDOM_LONG`.
pub fn base_damage_decal(rendermode: i32, random: impl FnOnce(i32, i32) -> i32) -> i32 {
    match rendermode {
        4 => -1,
        0 => DECAL_GUNSHOT1 + random(0, 4),
        _ => DECAL_BPROOF1,
    }
}

/// `CBreakable::DamageDecal` (0xc162c; `func_breakable` and `func_pushable`): glass cracks
/// with a random break decal, unbreakable glass takes the bulletproof one, the rest
/// [`base_damage_decal`].
pub fn breakable_damage_decal(material: i32, rendermode: i32, random: impl FnOnce(i32, i32) -> i32) -> i32 {
    match material {
        mat::GLASS => DECAL_GLASSBREAK1 + random(0, 2),
        mat::UNBREAKABLE_GLASS => DECAL_BPROOF1,
        _ => base_damage_decal(rendermode, random),
    }
}

/// `DamageDecal(CBaseEntity *, int)` (0xf0e30): the entity's `DamageDecal` (vtable +0x9c; no TS
/// override reads the damage bits), or a random gunshot for no entity (`DecalGunshot` passes
/// none for the world). A breakable whose object is out for a dispatch further up the stack
/// can't be read and counts as a plain entity.
pub fn damage_decal(e: Option<*mut edict_t>) -> i32 {
    let eng = engine();
    let random = |lo, hi| eng.random_long(lo, hi);
    let Some(e) = e else {
        return DECAL_GUNSHOT1 + random(0, 4);
    };
    let rendermode = e.v().rendermode;
    match breakable::material_of(e) {
        Some(m) => breakable_damage_decal(m, rendermode, random),
        None => base_damage_decal(rendermode, random),
    }
}

/// `UTIL_IsValidEntity` (0xeeacc): not null, not free, not `FL_KILLME`.
fn is_valid_entity(e: *mut edict_t) -> bool {
    !e.is_null() && !e.is_free() && e.v().flags & FL_KILLME as i32 == 0
}

/// `DecalGunshot(&tr, iBulletType)` (0xf0e7c): only on a `SOLID_BSP` or `MOVETYPE_PUSHSTEP` hit,
/// the hit entity's [`damage_decal`] (none for the world or an edict without private data), as a
/// `TE_GUNSHOTDECAL` ([`gunshot_decal_trace`]) or, for the crowbar, a plain decal.
pub fn decal_gunshot(tr: &TraceResult, bullet_type: i32) {
    let hit = tr.pHit;
    if !is_valid_entity(hit) {
        return;
    }
    let v = hit.v();
    if v.solid != SOLID_BSP as i32 && v.movetype != MOVETYPE_PUSHSTEP as i32 {
        return;
    }
    let entity = (engine().index_of(hit) != 0 && hit.has_private_data()).then_some(hit);
    let decal = damage_decal(entity);
    if bullet_type == BULLET_PLAYER_CROWBAR {
        // `UTIL_DecalTrace(&tr, DamageDecal(pEntity, DMG_CLUB))`.
        if let Some(name) = usize::try_from(decal).ok().and_then(|i| DECALS.get(i)) {
            decal_trace(tr, name);
        }
    } else {
        gunshot_decal_trace(tr, decal);
    }
}

/// `UTIL_GunshotDecalTrace` (0xee0fc): `TE_GUNSHOTDECAL` at the trace end to the PAS (position,
/// hit entity, decal), which the client draws with its ricochet. Nothing for a negative decal,
/// a decal the engine doesn't have, or a trace that hit nothing. Port guard: a number past
/// `gDecals` (TS reads past the table) places nothing.
pub fn gunshot_decal_trace(tr: &TraceResult, decal: i32) {
    let Some(name) = usize::try_from(decal).ok().and_then(|i| DECALS.get(i)) else {
        return;
    };
    let eng = engine();
    let index = eng.decal_index(name);
    if index < 0 || tr.flFraction == 1.0 {
        return;
    }
    let pos = tr.vecEndPos;
    te_begin(MSG_PAS, Some(pos), te::GUNSHOTDECAL).vec3(pos).short(eng.index_of(tr.pHit) as i16 as i32).byte(index).end();
}

/// The game's own material table as the active ruleset builds it: its size and name length
/// (`PmRules::texture_limits`) and whether it reads the map's file (`ServerRules::map_materials`).
pub fn game_textures() -> (shared::pm::textures::TextureLimits, bool) {
    let rules = crate::gameplay::active();
    (rules.pm.texture_limits(), rules.server.map_materials())
}

/// `TEXTURETYPE_PlaySound(ptr, vecSrc, vecEnd, iBulletType)` (0xe1330): the impact sound of what
/// `vecSrc -> vecEnd` hits. A hit entity whose `Classify()` ([`crate::entities::classify`]) isn't
/// `CLASS_NONE` or `CLASS_MACHINE` sounds as flesh (silent for the crowbar); anything else (or
/// no entity) by the material of the texture it hits, from the game's own table
/// (`shared::pm::textures::texturetype`, [`game_textures`]). Returns the decal volume (`fvolbar`).
pub fn texture_type_play_sound(tr: &TraceResult, src: Vec3, end: Vec3, bullet_type: i32) -> f32 {
    use shared::pm::textures::texturetype;
    let eng = engine();
    let world = eng.entity_of_index(0);
    let hit = if tr.pHit.is_null() { world } else { tr.pHit };
    let entity = (!hit.is_null() && hit.has_private_data()).then_some(hit);
    let chtype = if entity.is_none_or(|e| matches!(crate::ham::classify(e), 0 | 1)) {
        let name = eng.trace_texture(entity.unwrap_or(world), src, end).to_bytes();
        let mut name = name;
        if matches!(name.first(), Some(b'-' | b'+')) {
            name = name.get(2..).unwrap_or(&[]);
        }
        if matches!(name.first(), Some(b'{' | b'!' | b'~' | b' ')) {
            name = &name[1..];
        }
        // `strcpy` into a buffer whose last byte is zeroed: 15 characters (TS 3.0), the SDK's 12.
        let (limits, map_files) = game_textures();
        texturetype::find(&name[..name.len().min(limits.name_chars)], limits, map_files)
    } else {
        b'F'
    };
    let mut attn = 0.8f32;
    let (mut vol, mut volbar, samples): (f32, f32, &[&CStr]) = match chtype {
        b'D' => (0.9, 0.1, &[c"player/pl_dirt1.wav", c"player/pl_dirt2.wav", c"player/pl_dirt3.wav"]),
        b'F' => {
            if bullet_type == BULLET_PLAYER_CROWBAR {
                return 0.0;
            }
            attn = 1.0;
            (1.0, 0.2, &[c"weapons/bullet_hit1.wav", c"weapons/bullet_hit2.wav"])
        }
        b'G' => (0.9, 0.5, &[c"player/pl_grate1.wav", c"player/pl_grate4.wav"]),
        b'M' => (0.9, 0.3, &[c"player/pl_metal1.wav", c"player/pl_metal2.wav"]),
        b'P' | b'Y' => (0.8, 0.2, &[c"debris/glass1.wav", c"debris/glass2.wav", c"debris/glass3.wav"]),
        b'S' => (0.9, 0.0, &[c"player/pl_slosh1.wav", c"player/pl_slosh3.wav", c"player/pl_slosh2.wav", c"player/pl_slosh4.wav"]),
        b'T' => (0.8, 0.2, &[c"player/pl_tile1.wav", c"player/pl_tile3.wav", c"player/pl_tile2.wav", c"player/pl_tile4.wav"]),
        b'V' => (0.5, 0.3, &[c"player/pl_duct1.wav", c"player/pl_duct1.wav"]),
        b'W' => (0.9, 0.2, &[c"debris/wood1.wav", c"debris/wood2.wav", c"debris/wood3.wav"]),
        _ => (0.9, 0.6, &[c"player/pl_step1.wav", c"player/pl_step2.wav"]),
    };
    let breakable = entity.is_some_and(|e| eng.string(e.v().classname).to_bytes() == b"func_breakable");
    if breakable {
        // `fvol /= 1.5` as `fvol * (2.0 / 3)`, a `double`; `fvolbar /= 2`.
        vol = (vol as f64 * (2.0 / 3.0)) as f32;
        volbar *= 0.5;
    } else if chtype == b'P' && tr.flFraction != 1.0 && eng.random_long(0, 1) != 0 {
        super::explode::sparks(tr.vecEndPos);
        let flvolume = eng.random_float(0.7, 1.0);
        let spark = match eng.random_long(0, 1) {
            0 => Some(c"buttons/spark5.wav"),
            1 => Some(c"buttons/spark6.wav"),
            _ => None,
        };
        if let Some(spark) = spark {
            super::sentences::emit_ambient(world, tr.vecEndPos, spark, flvolume, 0.8, 0, 100);
        }
    }
    let pitch = 96 + eng.random_long(0, 0xf);
    let sample = samples[eng.random_long(0, samples.len() as i32 - 1).clamp(0, samples.len() as i32 - 1) as usize];
    super::sentences::emit_ambient(world, tr.vecEndPos, sample, vol, attn, 0, pitch);
    volbar
}

#[cfg(test)]
mod tests {
    use super::*;

    /// TS's `gDecals` order: the numbers `DamageDecal` returns name these decals.
    #[test]
    fn decal_table_numbers() {
        assert_eq!(DECALS[DECAL_GUNSHOT1 as usize], c"{shot1");
        assert_eq!(DECALS[4], c"{shot5");
        assert_eq!(DECALS[DECAL_GLASSBREAK1 as usize], c"{break1");
        assert_eq!(DECALS[DECAL_GLASSBREAK1 as usize + 2], c"{break3");
        assert_eq!(DECALS[DECAL_BPROOF1 as usize], c"{bproof1");
        assert_eq!(DECALS[41], c"{mommablob");
    }

    /// `CBaseEntity::DamageDecal` and `CBreakable::DamageDecal` by render mode and material.
    #[test]
    fn damage_decals() {
        let r = |lo: i32, hi: i32| {
            assert_eq!(lo, 0);
            hi
        };
        assert_eq!(base_damage_decal(0, r), 4);
        assert_eq!(base_damage_decal(4, r), -1);
        for mode in [1, 2, 3, 5] {
            assert_eq!(base_damage_decal(mode, r), DECAL_BPROOF1);
        }
        assert_eq!(breakable_damage_decal(mat::GLASS, 4, r), DECAL_GLASSBREAK1 + 2);
        assert_eq!(breakable_damage_decal(mat::UNBREAKABLE_GLASS, 0, r), DECAL_BPROOF1);
        assert_eq!(breakable_damage_decal(mat::WOOD, 0, r), 4);
        assert_eq!(breakable_damage_decal(mat::METAL, 4, r), -1);
    }
}
