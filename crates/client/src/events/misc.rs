//! The non-firing TS events: empty, pump, reload, draw, grenade, knife, katana, close combat
//! (kung fu) and coldcock (stock hit). Rendering doc §4.1-4.2, server weapons.md §6.1.

use super::fire::{eject_shell, weapon_body};
use super::impact::texture_type;
use super::{Args, first_person_view_model, info, slow_pitch};
use crate::{effects, state, view};
use goldsrc::ext::render::{self as r, Vec3, vec3 as v};
use goldsrc::sys::*;
use shared::weapons::tables;

fn set(id: i32) -> [i16; 29] {
    let alt = crate::prediction::alt_grip();
    info::info(id).map_or([0; 29], |i| *i.set(alt))
}

/// The local player, or the player we spectate in-eye (draw/grenade anims only play there).
fn in_eye(a: &Args) -> bool {
    state::with(|s| s.spec_mode == 4 || s.spec_mode == 5) && state::with(|s| s.spec_target) == a.entindex
}

fn sound(a: &Args, chan: u32, sample: &str, vol: f32) {
    r::play_sound(a.entindex, a.origin, chan as i32, sample, vol, r::ATTN_NORM, 0, slow_pitch());
}

/// `EV_TS_Empty` (`0x100424e0`).
pub fn empty(a: &Args) {
    sound(a, CHAN_ITEM, "weapons/pistol-empty.wav", 0.8);
}

/// `EV_TS_Pump` (`0x100419a0`): pump anim; in first person a shotgun shell and one smoke puff;
/// the pump sound (`info+0x306`).
pub fn pump(a: &Args) {
    let id = a.i1;
    if a.is_local() {
        r::weapon_animation(set(id)[13 + r::random_long(0, 1) as usize] as i32, weapon_body(id, a.i2));
        if let Some(vm) = first_person_view_model() {
            let (fwd, right, up) = a.vectors();
            let shell = tables().asset(id as u8).map(|s| s.shell.clone()).unwrap_or_default();
            if !shell.is_empty() {
                let vel = v::ma(v::ma(v::ma(a.velocity, r::random_float(100.0, 130.0), right), r::random_float(40.0, 80.0), up), 30.0, fwd);
                eject_shell(vm.attachment[0], vel, a.angles, r::model_index(&format!("models/{shell}.mdl")), 2);
            }
            // One forced wisp: radius 1, frame rate 20-40, speed 4, back along the barrel by the
            // muzzle offset (`info+0x34c`). The direction argument is unclear in the decompile;
            // `right` is used, as in the silenced shot's second smoke call.
            let now = effects::now();
            let ofs = tables().asset(id as u8).map_or(0, |s| s.fx[0]) as f32;
            let at = v::ma(vm.attachment[0], -ofs, fwd);
            let vel = a.velocity;
            effects::with_psys(|ps| effects::fx::gun_smoke(ps, now, 1.0, r::random_long(20, 40), at, vel, right, 4.0, true));
        }
    }
    if let Some(asset) = tables().asset(id as u8).filter(|s| !s.sound2.is_empty()) {
        sound(a, CHAN_ITEM, &format!("weapons/{}", asset.sound2), r::random_float(0.92, 1.0));
    }
}

/// `EV_TS_Reload` (`0x10042690`): magazine reload (`f1` 0) or shotgun phases 8/9/10.
pub fn reload(a: &Args) {
    let id = a.i1;
    let phase = a.f1 as i32;
    if a.is_local() || in_eye(a) {
        let s = set(id);
        let seq = match phase {
            8..=10 => s[phase as usize],
            _ if id == 33 && state::with(|st| st.clip) > 0 => s[8],
            _ => s[1],
        };
        r::weapon_animation(seq as i32, weapon_body(id, a.i2));
    }
    let first_person = a.is_local() && !view::is_third_person();
    match phase {
        9 => sound(a, CHAN_WEAPON, "weapons/insert-shell.wav", r::random_float(0.92, 1.0)),
        10 if !first_person => {
            if let Some(asset) = tables().asset(id as u8).filter(|s| !s.sound2.is_empty()) {
                sound(a, CHAN_ITEM, &format!("weapons/{}", asset.sound2), 0.9);
            }
        }
        0 => {
            if !first_person {
                sound(a, CHAN_WEAPON, "weapons/generic_reload.wav", 0.9);
            }
            // Revolver, sawed-off and Contender dump their casings.
            let dump = match id {
                33 => Some(("models/shotgun_shell.mdl", 2, 2)),
                31 => Some(("models/9mm_shell.mdl", 5, 1)),
                36 => Some(("models/556_shell.mdl", 1, 1)),
                _ => None,
            };
            if let Some((model, n, snd)) = dump {
                if a.is_local() || effects::player_visible(a.entindex) {
                    let m = r::model_index(model);
                    let pos = r::entity(a.entindex).map_or(a.origin, |e| e.attachment[0]);
                    for _ in 0..n {
                        let vel =
                            v::add(a.velocity, [r::random_float(-30.0, 30.0), r::random_float(-30.0, 30.0), -r::random_float(0.0, 30.0)]);
                        eject_shell(pos, vel, a.angles, m, snd);
                    }
                }
            }
        }
        _ => {}
    }
}

/// `EV_TS_Draw` (`0x100425b0`): only when spectating in-eye.
pub fn draw(a: &Args) {
    if in_eye(a) && !a.is_local() {
        r::weapon_animation(set(a.i1)[2] as i32, weapon_body(a.i1, a.i2));
    }
}

/// `EV_TS_Grenade` (`0x10042630`): in-eye only, M61 anims 19 pull / 20 throw / 21 roll.
pub fn grenade(a: &Args) {
    if in_eye(a) && !a.is_local() {
        let i = (a.i1 as usize).min(28);
        r::weapon_animation(set(24)[i] as i32, 0);
    }
}

/// `TS_MeleeMaterialSound` (`0x10045df0`): footstep-style sounds for a melee hit.
fn melee_material_sound(tex: u8, pos: Vec3) {
    let list: &[&str] = match tex {
        b'M' => &["player/pl_metal1.wav", "player/pl_metal2.wav"],
        b'D' => &["player/pl_dirt1.wav", "player/pl_dirt2.wav", "player/pl_dirt3.wav"],
        b'V' => &["player/pl_duct1.wav"],
        b'G' => &["player/pl_grate1.wav", "player/pl_grate4.wav"],
        b'T' => &["player/pl_tile1.wav", "player/pl_tile2.wav", "player/pl_tile3.wav", "player/pl_tile4.wav"],
        b'S' => &["player/pl_slosh1.wav", "player/pl_slosh2.wav", "player/pl_slosh3.wav", "player/pl_slosh4.wav"],
        b'W' => &["debris/wood1.wav", "debris/wood2.wav", "debris/wood3.wav"],
        b'P' | b'Y' => &["debris/glass1.wav", "debris/glass2.wav", "debris/glass3.wav"],
        b'F' => &[],
        _ => &["player/pl_step1.wav", "player/pl_step2.wav"],
    };
    if let Some(s) = list.get(r::random_long(0, list.len() as i32 - 1).max(0) as usize) {
        r::play_sound(0, pos, CHAN_STATIC as i32, s, 0.9, r::ATTN_NORM, 0, slow_pitch());
    }
}

/// Short melee trace from the gun position; returns (hit position, normal, texture) on a hit.
fn melee_trace(a: &Args, reach: f32) -> Option<(Vec3, Vec3, u8)> {
    let (fwd, _, _) = a.vectors();
    let src = a.gun_position();
    let end = v::ma(src, reach, fwd);
    r::with_solid_players(a.entindex - 1, || {
        let mut tr = r::player_trace(src, end, 2, PM_STUDIO_BOX as i32, -1);
        if tr.fraction >= 1.0 {
            // TS uses a short hull trace when the line misses.
            tr = r::player_trace(src, end, 1, PM_STUDIO_BOX as i32, -1);
        }
        (tr.fraction < 1.0).then(|| {
            let tex = texture_type(&mut tr, src, end);
            (tr.endpos, tr.plane.normal, tex)
        })
    })
}

fn hard(tex: u8) -> bool {
    !matches!(tex, b'F' | b'S' | b'D' | b'W')
}

/// Melee hit puff: `TS_SmallPuff(pos, n, sprite, 5, 12, 30)`, wood smoke on dirt and wood.
fn puff(pos: Vec3, n: Vec3, tex: u8) {
    let now = effects::now();
    let sprite = if matches!(tex, b'D' | b'W') { "sprites/wood_smoke_add.spr" } else { "sprites/gun_smoke_add.spr" };
    effects::with_psys(|ps| effects::fx::small_puff(ps, now, pos, n, sprite, kRenderTransAdd as i32, 12.0, 30.0));
}

/// Metal hit: `TS_Scintille(size)` and `TS_Sparks(7, 1.15, 25)`.
fn metal_hit(pos: Vec3, n: Vec3, size: f32) {
    let now = effects::now();
    effects::with_psys(|ps| {
        effects::fx::scintille_metal(ps, now, pos, n, size);
        effects::fx::sparks(ps, now, pos, n, 7, 1.15, 25.0);
    });
}

/// `EV_TS_Knife` (`0x10043c90`): trace 35 ahead; metal sparks + `knife_hit` on hard surfaces,
/// `knife_miss` otherwise.
pub fn knife(a: &Args) {
    if a.is_local() {
        let s = set(a.i1);
        r::weapon_animation(s[15 + r::random_long(0, 2) as usize] as i32, 0);
    }
    match melee_trace(a, 35.0) {
        Some((pos, n, tex)) if hard(tex) => {
            metal_hit(pos, n, 6.0);
            sound(a, CHAN_WEAPON, "weapons/knife/knife_hit.wav", 1.0);
        }
        Some((pos, _, tex)) => {
            melee_material_sound(tex, pos);
            sound(a, CHAN_WEAPON, "weapons/knife/knife_miss.wav", 1.0);
        }
        None => sound(a, CHAN_WEAPON, "weapons/knife/knife_miss.wav", 1.0),
    }
}

/// `EV_TS_Katana` (`0x10044230`): `i1` is the swing's anim index (15 = back swing, trace -55).
pub fn katana(a: &Args) {
    if a.is_local() {
        let s = set(34);
        let i = (a.i1 as usize).clamp(15, 18);
        r::weapon_animation(s[i] as i32, 0);
    }
    let reach = if a.i1 == 15 { -55.0 } else { 50.0 };
    let n = if r::random_long(0, 1) == 0 { "" } else { "1" };
    match melee_trace(a, reach) {
        Some((pos, nrm, tex)) => {
            if hard(tex) {
                metal_hit(pos, nrm, 12.0);
            }
            sound(a, CHAN_WEAPON, &format!("weapons/katana/hit{n}.wav"), 1.0);
        }
        None => sound(a, CHAN_WEAPON, &format!("weapons/katana/miss{n}.wav"), 1.0),
    }
}

/// `EV_TS_CloseCombat` (`0x10043200`): kung-fu block (`i1` 0) or attack (`i1` 1..3, `i2` anim).
pub fn close_combat(a: &Args) {
    if a.i1 == 0 {
        sound(a, CHAN_WEAPON, "player/block.wav", 1.0);
        return;
    }
    if a.is_local() && a.i2 > 0 {
        r::weapon_animation(a.i2, 0);
    }
    if let Some((pos, n, tex)) = melee_trace(a, 48.0) {
        melee_material_sound(tex, pos);
        if tex != b'F' {
            puff(pos, n, tex);
        }
    }
    sound(a, CHAN_WEAPON, "player/closecombat.wav", 1.0);
}

/// `EV_TS_ColdCock` (`0x10043800`): stock hit, anim `[28]`, trace 30 ahead.
pub fn coldcock(a: &Args) {
    if a.is_local() {
        r::weapon_animation(set(a.i1)[28] as i32, weapon_body(a.i1, a.i2));
    }
    if let Some((pos, n, tex)) = melee_trace(a, 30.0) {
        melee_material_sound(tex, pos);
        if tex != b'F' {
            puff(pos, n, tex);
        }
    }
    sound(a, CHAN_WEAPON, "player/closecombat.wav", 0.8);
}
