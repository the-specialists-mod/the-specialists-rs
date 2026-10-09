//! Bullet impact helpers: texture sounds, ricochets, decals (rendering doc §4.5).

use super::{fire_pitch, slow_pitch};
use crate::effects;
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
        b'S' => (&["player/pl_slosh1.wav", "player/pl_slosh2.wav", "player/pl_slosh3.wav", "player/pl_slosh4.wav"], 0.9),
        b'W' => (&["debris/wood1.wav", "debris/wood2.wav", "debris/wood3.wav"], 0.9),
        b'T' => (
            &[
                "debris/cement.wav",
                "debris/cement1.wav",
                "debris/cement2.wav",
                "debris/cement3.wav",
                "debris/cement4.wav",
                "debris/cement5.wav",
                "debris/cement6.wav",
            ],
            0.8,
        ),
        b'F' => (&[], 0.0),
        _ => (&["debris/cement1.wav", "debris/cement2.wav", "debris/cement3.wav", "debris/cement4.wav", "debris/cement5.wav", "debris/cement6.wav"], 0.9),
    }
}

/// Material character of what `tr` hit: `F` for players, else the texture's type.
pub fn texture_type(tr: &mut pmtrace_s, src: Vec3, end: Vec3) -> u8 {
    let ent = r::index_from_trace(tr);
    if ent > 0 && ent <= r::max_clients() {
        return b'F';
    }
    match r::trace_texture(tr.ent, src, end) {
        Some(name) => {
            let n = strip_texture_name(&name);
            let mut buf: Vec<u8> = n.bytes().take(15).collect();
            buf.push(0);
            shared::pm::find_texture_type(std::ffi::CStr::from_bytes_until_nul(&buf).unwrap_or(c""))
        }
        None => b'C',
    }
}

/// `TS_PlayTextureSound` (`0x10044bf0`): ricochet, material sound (dmg > 33) and impact
/// particles. Returns the texture type.
pub fn texture_sound(dmg: f32, tr: &mut pmtrace_s, src: Vec3, end: Vec3, bullet_type: i32) -> u8 {
    if r::point_contents(tr.endpos) == CONTENTS_SKY {
        return 0;
    }
    let tex = texture_type(tr, src, end);
    if tex != b'F' && r::random_long(0, 0x7fff) < 0x3fff {
        let snd = format!("debris/ric{}.wav", r::random_long(1, 5));
        r::play_sound(-1, tr.endpos, CHAN_AUTO as i32, &snd, 1.0, 0.99, 0, fire_pitch());
    }
    if tex == b'F' {
        return tex;
    }
    effects::impact(tr.endpos, tr.plane.normal, tex, bullet_type == 1);
    let (sounds, vol) = material_sounds(tex);
    if dmg > 33.0 && !sounds.is_empty() {
        let s = sounds[r::random_long(0, sounds.len() as i32 - 1) as usize];
        r::play_sound(0, tr.endpos, CHAN_STATIC as i32, s, vol, r::ATTN_NORM, 0, slow_pitch());
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
        assert_eq!(AMMO_ATTENUATION[6], 0.2);
    }
}
