//! Bullet impact helpers: texture sounds, ricochets, decals (rendering doc §4.5).

use crate::effects;
use goldsrc::client::engine;
use goldsrc::ext::render::{self as r, Vec3};
use goldsrc::sys::*;

/// Client ammo table fields not in `shared::weapons` (`GetAmmoInfo` `0x100071f0`, 16 x 0x38):
/// `[0]` fire-sound attenuation, indexed by ammo id. Decoded from `FUN_10007480`.
pub const AMMO_ATTENUATION: [f32; 14] = [0.0, 0.4, 0.4, 0.3, 0.3, 0.3, 0.2, 0.5, 0.1, 0.3, 0.5, 0.3, 0.5, 0.2];
/// `ammo[5]`: maximum bullet distance (8192 for every type).
pub const MAX_RANGE: f32 = 8192.0;
/// `ammo[7]`: spread-cone radius (1.0 for every type).
pub const SPREAD_RADIUS: f32 = 1.0;

/// Range falloff table (`FUN_10007220`): `min(1, 1/sqrt(max(idx, 1) * 0.1))`, 10000 entries.
pub fn range_mult(idx: i32) -> f32 {
    let i = idx.clamp(1, 9999) as f32;
    (1.0 / (i * 0.1).sqrt()).min(1.0)
}

/// Strips the texture prefixes TS skips (`-`/`+` animation pairs, then `{`, `!`, `~`, space).
pub fn strip_texture_name(name: &str) -> &str {
    let mut s = name;
    if s.starts_with('-') || s.starts_with('+') {
        s = s.get(2..).unwrap_or("");
    }
    if s.starts_with(['{', '!', '~', ' ']) {
        s = &s[1..];
    }
    s
}

/// Material sounds (`TS_PlayTextureSound` table).
pub fn material_sounds(tex: u8) -> (&'static [&'static str], f32) {
    match tex {
        b'D' => (&["debris/gravel1.wav", "debris/gravel2.wav", "debris/gravel3.wav", "debris/gravel4.wav"], 0.9),
        b'G' | b'M' => (&["debris/metal1.wav", "debris/metal2.wav", "debris/metal3.wav", "debris/metal4.wav", "debris/metal5.wav"], 0.9),
        b'V' => (&["debris/metal1.wav", "debris/metal2.wav", "debris/metal3.wav", "debris/metal4.wav", "debris/metal5.wav"], 0.5),
        b'P' | b'Y' => (&["debris/glass1.wav", "debris/glass2.wav", "debris/glass3.wav"], 0.8),
        // TS's order (`0x10044ef5`): 1, 3, 2, 4.
        b'S' => (&["player/pl_slosh1.wav", "player/pl_slosh3.wav", "player/pl_slosh2.wav", "player/pl_slosh4.wav"], 0.9),
        b'W' => (&["debris/wood1.wav", "debris/wood2.wav", "debris/wood3.wav"], 0.9),
        b'T' => (&CEMENT, 0.8),
        b'F' => (&[], 0.0),
        // Every other type shares `T`'s seven sounds (`0x10044f8d` falls into `0x10044f9d`).
        _ => (&CEMENT, 0.9),
    }
}

const CEMENT: [&str; 7] = [
    "debris/cement.wav",
    "debris/cement1.wav",
    "debris/cement2.wav",
    "debris/cement3.wav",
    "debris/cement4.wav",
    "debris/cement5.wav",
    "debris/cement6.wav",
];

/// Attenuation of the material sound (`0x10045034`, `0x3f7851ec`).
pub(crate) const MATERIAL_ATTN: f32 = 0.97;

/// `debris/ric1..5.wav`, picked by the ricochet roll itself: `roll % 5` (`0x10044d2e`).
fn ricochet_sound(roll: i32) -> String {
    format!("debris/ric{}.wav", roll % 5 + 1)
}

/// The material-sound roll's test (`0x10044ff8`: `cmp eax, 0x21; jle`).
fn material_roll_plays(roll: i32) -> bool {
    roll > 33
}

/// Material character of what `tr` hit: `F` for players, else the texture's type (`C` when
/// `pfnTraceTexture` finds no texture).
pub fn texture_type(tr: &mut pmtrace_s, src: Vec3, end: Vec3) -> u8 {
    traced_texture_type(tr, src, end).unwrap_or(b'C')
}

/// [`texture_type`], but `None` when `pfnTraceTexture` returns NULL, which
/// `TS_MeleeMaterialSound` (`0x10045e6a`) tells apart from a concrete texture.
pub fn traced_texture_type(tr: &mut pmtrace_s, src: Vec3, end: Vec3) -> Option<u8> {
    let ent = r::index_from_trace(tr);
    if ent > 0 && ent <= engine().max_clients() {
        return Some(b'F');
    }
    r::trace_texture(tr.ent, src, end).map(|name| {
        let n = strip_texture_name(&name);
        let mut buf: Vec<u8> = n.bytes().take(15).collect();
        buf.push(0);
        // The ruleset's material table (`PM_FindTextureType`, [`PmRules::texture_limits`]).
        //
        // [`PmRules::texture_limits`]: shared::gameplay::PmRules::texture_limits
        let name = std::ffi::CStr::from_bytes_until_nul(&buf).unwrap_or(c"");
        shared::pm::textures::find_in(name.to_bytes(), crate::gameplay::hud_pm_rules().texture_limits())
    })
}

/// `TS_PlayTextureSound` (`0x10044bf0`): ricochet, impact particles and material sound. Returns
/// the texture type. TS's damage argument is never read, so it isn't taken here: the material
/// sound plays on its own roll, `RandomLong(0, 100) > 33` (rolled at `0x10044c2a`, right after
/// the sky test and before the ricochet's roll; tested at `0x10044ff8`).
pub fn texture_sound(tr: &mut pmtrace_s, src: Vec3, end: Vec3, bullet_type: i32) -> u8 {
    if r::point_contents(tr.endpos) == CONTENTS_SKY {
        return 0;
    }
    let roll = r::random_long(0, 100);
    let tex = texture_type(tr, src, end);
    if tex == b'F' {
        return tex;
    }
    let ric = r::random_long(0, 0x7fff);
    if ric < 0x3fff {
        let factor100 = crate::state::with(|s| s.slowmo_factor100);
        let pitch = crate::gameplay::current().ricochet_pitch(super::time_scale(), factor100);
        engine().ev_play_sound(-1, tr.endpos, CHAN_AUTO as i32, &ricochet_sound(ric), 1.0, 0.99, 0, pitch);
    }
    super::fx_log(|| format!("impact on texture type {} at {:.1?}", tex as char, tr.endpos));
    effects::impact(tr.endpos, tr.plane.normal, tex, bullet_type);
    let (sounds, vol) = material_sounds(tex);
    if material_roll_plays(roll) {
        // The pitch's `RandomLong(-5, 5)` comes before the sample's (`0x10044ffd`, `0x10045040`).
        let (attn, pitch) = crate::gameplay::current().bullet_material_sound();
        // `get`: the engine's RandomLong is trusted to stay in range, but not to the point of a crash.
        if let Some(s) = usize::try_from(r::random_long(0, sounds.len() as i32 - 1)).ok().and_then(|i| sounds.get(i)) {
            engine().ev_play_sound(0, tr.endpos, CHAN_STATIC as i32, s, vol, attn, 0, pitch);
        }
    }
    tex
}

/// Decal name for a bullet hit on physent `pe` (`TS_DamageDecal` `0x10001000`).
pub fn damage_decal(pe: &physent_s, paintball: bool) -> String {
    if paintball {
        return format!("{{splat{}", r::random_long(0, 17));
    }
    if pe.classnumber == 1 {
        format!("{{break{}", r::random_long(0, 2) + 1)
    } else if pe.rendermode == 0 {
        format!("{{shot{}", r::random_long(0, 4) + 1)
    } else {
        "{bproof1".to_string()
    }
}

/// `TS_DecalGunshot` (`0x10001180`): only on `SOLID_BSP` physents, and only with `r_decals`.
pub fn decal_gunshot(tr: &mut pmtrace_s) {
    let Some(pe) = r::physent(tr.ent) else { return };
    if pe.solid != SOLID_BSP as i32 {
        return;
    }
    if goldsrc::client::engine().cvar_float(c"r_decals") == 0.0 {
        return;
    }
    let name = damage_decal(pe, effects::paintball());
    let idx = r::decal_index(&name);
    let ent = r::index_from_trace(tr);
    super::fx_log(|| format!("decal {name} on entity {ent} at {:.1?}", tr.endpos));
    r::decal_shoot(idx, ent, 0, tr.endpos, 0);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn helpers() {
        assert_eq!(strip_texture_name("-0metal"), "metal");
        assert_eq!(strip_texture_name("{grate"), "grate");
        assert_eq!(strip_texture_name("+a~lamp"), "lamp");
        assert_eq!(range_mult(0), 1.0);
        assert_eq!(range_mult(10), 1.0);
        assert!((range_mult(40) - 0.5).abs() < 1e-6);
        assert_eq!(material_sounds(b'F').0.len(), 0);
        assert_eq!(material_sounds(b'C'), (&CEMENT[..], 0.9));
        assert_eq!(material_sounds(b'T'), (&CEMENT[..], 0.8));
        assert_eq!(material_sounds(b'S').0[1], "player/pl_slosh3.wav");
        assert!(!material_roll_plays(33) && material_roll_plays(34));
        assert_eq!(ricochet_sound(0), "debris/ric1.wav");
        assert_eq!(ricochet_sound(0x3ffe), "debris/ric3.wav");
        assert_eq!(ricochet_sound(9), "debris/ric5.wav");
        assert_eq!(AMMO_ATTENUATION[6], 0.2);
    }
}
