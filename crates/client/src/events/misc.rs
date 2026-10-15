//! The non-firing TS events: empty, pump, reload, draw, grenade, knife, katana, close combat
//! (kung fu) and coldcock (stock hit). Rendering doc §4.1-4.2, server weapons.md §6.1.

use super::fire::{eject_shell, weapon_body};
use super::impact::traced_texture_type;
use super::{Args, first_person_view_model, info};
use crate::gameplay::MeleeReach;
use crate::{effects, view};
use goldsrc::client::engine;
use goldsrc::ext::render::{self as r, Vec3, vec3 as v};
use goldsrc::sys::*;
use shared::weapons::tables;

/// `GetActiveAnimSet` (`0x1003df60`), as in `fire::anim_set`.
fn set(id: i32) -> [i16; 29] {
    let alt = crate::prediction::alt_grip() && !crate::spectator::in_eye();
    info::info(id).map_or([0; 29], |i| *i.set(alt))
}

/// The normal sequence set (`info+0x252`), which the melee events index directly instead of going
/// through `GetActiveAnimSet`.
fn set_a(id: i32) -> [i16; 29] {
    info::info(id).map_or([0; 29], |i| i.a)
}

/// An event sound at the shooter, with the ruleset's volume, attenuation and pitch
/// ([`ClientRules::event_sound`](crate::gameplay::ClientRules::event_sound)).
fn sound(a: &Args, chan: u32, sample: &str) {
    let (vol, attn, pitch) = crate::gameplay::current().event_sound();
    engine().ev_play_sound(a.entindex, a.origin, chan as i32, sample, vol, attn, 0, pitch);
}

/// `EV_TS_Empty` (`0x100424e0`): `CHAN_WEAPON`, volume `RandomFloat(0.92, 1.0)` (`0x10042560..`).
pub fn empty(a: &Args) {
    sound(a, CHAN_WEAPON, "weapons/pistol-empty.wav");
}

/// `EV_TS_Pump` (`0x100419a0`): pump anim; a shotgun shell and one forced smoke wisp, both
/// `muzzle - fwd·ofs` (`info+0x34c`) back along the barrel, from the view model's attachment 0 in
/// first person (`0x10041c78..0x10041f2e`) or the player's attachment 3 in third person and for
/// other players (`0x100420dc..0x100423be`); then the pump sound (`info+0x306`). Without a shell
/// model nothing is ejected and there is no smoke (`0x10041c70`, `0x100420f9`); with no view model
/// the shell leaves from the view origin, without smoke.
pub fn pump(a: &Args) {
    let id = a.i1;
    let (fwd, right, up) = a.vectors();
    // Shell speeds (0x10041bc9..0x10041be8): 60-80 forward and right, 50-100 up.
    let side = r::random_long(60, 80) as f32;
    let lift = r::random_long(50, 100) as f32;
    let shell = tables().asset(id as u8).map(|s| s.shell.clone()).unwrap_or_default();
    let model = if shell.is_empty() { 0 } else { r::model_index(&format!("models/{shell}.mdl")) };
    let ofs = tables().asset(id as u8).map_or(0, |s| s.fx[0]) as f32;
    let shell_vel = v::ma(v::ma(v::ma(a.velocity, side, right), lift, up), side, fwd);
    let wisp = |muzzle: Vec3| {
        let now = effects::now();
        let at = v::ma(muzzle, -ofs, fwd);
        eject_shell(at, shell_vel, a.angles, model, 2);
        let vel = a.velocity;
        effects::with_psys(|ps| effects::fx::gun_smoke(ps, now, 1.0, r::random_long(20, 40), at, vel, right, 4.0, true));
    };
    let owner = a.is_view_owner();
    let first_person = owner && !view::is_third_person();
    if owner {
        // `[13 + RandomLong(0, 1)]` (`0x10041c46`).
        r::weapon_animation(set(id)[13 + r::random_long(0, 1) as usize] as i32, weapon_body(id, a.i2));
        if first_person && model != 0 {
            match first_person_view_model() {
                Some(vm) => wisp(vm.attachment[0]),
                None => eject_shell(view::origin(), shell_vel, a.angles, model, 2),
            }
        }
    }
    // Third person and other players: the cached player attachment 3 (`0x100422ba`, `0x10042394`).
    if !first_person && model != 0 && effects::player_visible(a.entindex) {
        wisp(effects::player_attachment(a.entindex, 3));
    }
    // Not attached to the shooter: entity -1 at the event origin, `CHAN_WEAPON` (`0x100423c6..`).
    if let Some(asset) = tables().asset(id as u8).filter(|s| !s.sound2.is_empty()) {
        let sample = format!("weapons/{}", asset.sound2);
        let (vol, attn, pitch) = crate::gameplay::current().event_sound();
        engine().ev_play_sound(-1, a.origin, CHAN_WEAPON as i32, &sample, vol, attn, 0, pitch);
    }
}

/// `EV_TS_Reload` (`0x10042690`): magazine reload (`f1` 0) or shotgun phases 8/9/10. The view owner
/// plays `[1]` for a magazine and `[phase]` otherwise (`0x10042881..0x10042891`). Phase 9 plays
/// `insert-shell` for everyone; phase 10's pump sound and phase 0's `generic_reload` (then, for a
/// visible player, the dropped casings or magazine) are only for others and third person.
///
/// A ruleset without the view animation and phases
/// ([`ClientRules::reload_event_phases`](crate::gameplay::ClientRules::reload_event_phases))
/// treats every reload event as a magazine reload.
pub fn reload(a: &Args) {
    let id = a.i1;
    let phases = crate::gameplay::current().reload_event_phases();
    let phase = if phases { a.f1 as i32 } else { 0 };
    let owner = a.is_view_owner();
    if owner && phases {
        let s = set(id);
        let seq = if phase == 0 { s[1] } else { s.get(phase as usize).copied().unwrap_or(0) };
        r::weapon_animation(seq as i32, weapon_body(id, a.i2));
    }
    let first_person = owner && !view::is_third_person();
    match phase {
        9 => sound(a, CHAN_WEAPON, "weapons/insert-shell.wav"),
        10 if !first_person => {
            if let Some(asset) = tables().asset(id as u8).filter(|s| !s.sound2.is_empty()) {
                sound(a, CHAN_WEAPON, &format!("weapons/{}", asset.sound2));
            }
        }
        0 if !first_person => {
            sound(a, CHAN_WEAPON, "weapons/generic_reload.wav");
            if effects::player_visible(a.entindex) {
                reload_drop(a, id);
            }
        }
        _ => {}
    }
}

/// What a magazine reload drops (`0x10042a54..0x10043188`): the sawed-off, the Raging Bull and the
/// Contender eject their casings from the cached player attachment 0, everything else drops
/// `generic_clip` (two for akimbo, the second from attachment 2).
fn reload_drop(a: &Args, id: i32) {
    let (fwd, right, up) = a.vectors();
    let casings = match id {
        33 => Some(("models/shotgun_shell.mdl", 2, 2)),
        31 => Some(("models/9mm_shell.mdl", 5, 1)),
        36 => Some(("models/556_shell.mdl", 1, 1)),
        _ => None,
    };
    if let Some((model, n, snd)) = casings {
        let m = r::model_index(model);
        for _ in 0..n {
            // `right·RandomLong(-30, 30) + velocity + fwd·10 + up·RandomLong(0, 30)`.
            let side = r::random_long(-30, 30) as f32;
            let lift = r::random_long(0, 30) as f32;
            let vel = v::ma(v::ma(v::ma(a.velocity, side, right), 10.0, fwd), lift, up);
            eject_shell(effects::player_attachment(a.entindex, 0), vel, a.angles, m, snd);
        }
        return;
    }
    let clips = if matches!(info::class(id), 4 | 5) { 2 } else { 1 };
    for i in 0..clips {
        // The spin goes in as the temp model's angles, drawn y, x, z.
        let y = r::random_float(-60.0, 60.0);
        let x = r::random_float(-60.0, 60.0);
        let z = r::random_float(-60.0, 60.0);
        let m = r::model_index("models/generic_clip.mdl");
        let vel = v::ma(a.velocity, if i == 0 { 60.0 } else { -60.0 }, right);
        let at = v::ma(effects::player_attachment(a.entindex, if i == 0 { 0 } else { 2 }), -8.0, fwd);
        r::temp_model(at, vel, [x, y, z], 2.0, m, 0);
    }
}

/// `EV_TS_Draw` (`0x100425b0`): only when spectating in-eye, `[2]`.
pub fn draw(a: &Args) {
    if a.is_in_eye_target() {
        r::weapon_animation(set(a.i1)[2] as i32, weapon_body(a.i1, a.i2));
    }
}

/// `EV_TS_Grenade` (`0x10042630`): in-eye only, the M61's `[i1]` (19 pull, 20 throw, 21 roll)
/// with body 1 (`0x1004267d`).
pub fn grenade(a: &Args) {
    if a.is_in_eye_target() {
        let i = (a.i1 as usize).min(28);
        r::weapon_animation(set(24)[i] as i32, 1);
    }
}

/// What a melee trace hit, as `TS_MeleeMaterialSound` (`0x10045df0`) tells it apart.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Surface {
    /// The hit point is `CONTENTS_SKY` (`0x10045e0a..0x10045e1a`).
    Sky,
    /// `pfnTraceTexture` returned NULL (`0x10045e6a`).
    Untextured,
    /// A player (`F`) or the texture's type.
    Type(u8),
}

/// `TS_MeleeMaterialSound`'s samples by texture type, in its tables' order (byte table
/// `0x100460f8`, jump table `0x100460d0`; tile and slosh list 1, 3, 2, 4 at `0x10045f89` and
/// `0x10045fbb`, duct its one sample twice at `0x10045f52`). Flesh has none (`0x10046038`);
/// anything else, including no texture (0), takes the default `pl_step` pair (`0x10046044`).
fn melee_material_samples(tex: u8) -> &'static [&'static str] {
    match tex {
        b'M' => &["player/pl_metal1.wav", "player/pl_metal2.wav"],
        b'D' => &["player/pl_dirt1.wav", "player/pl_dirt2.wav", "player/pl_dirt3.wav"],
        b'V' => &["player/pl_duct1.wav", "player/pl_duct1.wav"],
        b'G' => &["player/pl_grate1.wav", "player/pl_grate4.wav"],
        b'T' => &["player/pl_tile1.wav", "player/pl_tile3.wav", "player/pl_tile2.wav", "player/pl_tile4.wav"],
        b'S' => &["player/pl_slosh1.wav", "player/pl_slosh3.wav", "player/pl_slosh2.wav", "player/pl_slosh4.wav"],
        b'W' => &["debris/wood1.wav", "debris/wood2.wav", "debris/wood3.wav"],
        b'P' | b'Y' => &["debris/glass1.wav", "debris/glass2.wav", "debris/glass3.wav"],
        b'F' => &[],
        _ => &["player/pl_step1.wav", "player/pl_step2.wav"],
    }
}

/// `TS_MeleeMaterialSound` (`0x10045df0`): a footstep-style sound at the hit point, entity 0,
/// `CHAN_STATIC`. Returns the texture type the knife and katana test: 0 for the sky (no sound)
/// and for a surface with no texture (after the default sound). The ruleset's
/// [`melee_material_sound`](crate::gameplay::ClientRules::melee_material_sound) rolls the pitch
/// (`0x10046066`) before the sample's `RandomLong(0, n - 1)` (`0x100460a5`).
fn melee_material_sound(surface: Surface, pos: Vec3) -> u8 {
    let tex = match surface {
        Surface::Sky => return 0,
        Surface::Untextured => 0,
        Surface::Type(t) => t,
    };
    let list = melee_material_samples(tex);
    if !list.is_empty()
        && let Some((vol, attn, pitch)) = crate::gameplay::current().melee_material_sound(tex)
        && let Some(s) = list.get(r::random_long(0, list.len() as i32 - 1).max(0) as usize)
    {
        engine().ev_play_sound(0, pos, CHAN_STATIC as i32, s, vol, attn, 0, pitch);
    }
    tex
}

/// Short melee trace from the gun position; returns (hit position, normal, surface) on a hit.
/// All four melee events make one hull-2 `PM_STUDIO_BOX` trace (`0x10043200`, `0x10043800`,
/// `0x10043c90`, `0x10044230`); a fraction of 1 is a miss. The surface is looked up as
/// `TS_MeleeMaterialSound` does, before the events pop the player states: the sky test, then
/// players, then the texture.
fn melee_trace(a: &Args, reach: f32) -> Option<(Vec3, Vec3, Surface)> {
    let (fwd, _, _) = a.vectors();
    let src = a.gun_position();
    let end = v::ma(src, reach, fwd);
    r::with_solid_players(a.entindex.wrapping_sub(1), || {
        let mut tr = r::player_trace(src, end, 2, PM_STUDIO_BOX as i32, -1);
        (tr.fraction < 1.0).then(|| {
            let surface = if r::point_contents(tr.endpos) == CONTENTS_SKY {
                Surface::Sky
            } else {
                traced_texture_type(&mut tr, src, end).map_or(Surface::Untextured, Surface::Type)
            };
            (tr.endpos, tr.plane.normal, surface)
        })
    })
}

/// The knife's and katana's byte tables (`0x10044140` / `0x10044708`, indexed by texture type up
/// to `'W'`, anything above takes the spark branch): 0 (the sky, no texture), `'D'` and `'W'` are
/// 0 and `'F'` is 1, which both skip the sparks and the hit sound; every other type, slosh
/// included, sparks.
fn hard(tex: u8) -> bool {
    !matches!(tex, 0 | b'F' | b'D' | b'W')
}

/// Melee hit puff: `TS_SmallPuff(pos, n, sprite, 5, 12, 30)`, wood smoke on dirt and wood.
fn puff(pos: Vec3, n: Vec3, tex: u8) {
    let now = effects::now();
    let sprite = if matches!(tex, b'D' | b'W') { "sprites/wood_smoke_add.spr" } else { "sprites/gun_smoke_add.spr" };
    effects::with_psys(|ps| effects::fx::small_puff(ps, now, pos, n, sprite, kRenderTransAdd as i32, 12.0, 30.0));
}

/// Metal hit: `TS_Scintille(size)` and `TS_Sparks(7, 1.15, 25)`, unless the ruleset makes its own
/// ([`ClientRules::blade_hit_particles`](crate::gameplay::ClientRules::blade_hit_particles)).
fn metal_hit(pos: Vec3, n: Vec3, size: f32, katana: bool) {
    let now = effects::now();
    let rules = crate::gameplay::current();
    effects::with_psys(|ps| {
        if rules.blade_hit_particles(ps, now, pos, n, katana) {
            return;
        }
        effects::fx::scintille_metal(ps, now, pos, n, size);
        effects::fx::sparks(ps, now, pos, n, 7, 1.15, 25.0);
    });
}

/// The melee events' trace length under the active ruleset.
fn reach(m: MeleeReach) -> f32 {
    crate::gameplay::current().melee_reach(m)
}

/// The knife's and katana's view animation, played after the trace (`0x100440b0`, `0x100446b4`):
/// only for the view owner in first person, from set A (`info+0x252`, not `GetActiveAnimSet`),
/// with body `i2`, or 1 when the blade hit flesh (`'F'`: `0x10043f82`, `0x100445a4`).
fn blade_anim(a: &Args, flesh: bool, seq: impl FnOnce() -> i16) {
    if a.is_view_owner() && !view::is_third_person() {
        r::weapon_animation(seq() as i32, if flesh { 1 } else { a.i2 });
    }
}

/// `EV_TS_Knife` (`0x10043c90`): trace 35 ahead. A hit plays the material sound
/// (`TS_MeleeMaterialSound` `0x10043f63`), plus metal sparks and `knife_hit` on hard surfaces
/// (`0x10043f8d`); only a miss plays `knife_miss` (`0x10044042`). Then set A
/// `[15 + RandomLong(0, 2)]` (`0x100440c8..0x1004411b`).
pub fn knife(a: &Args) {
    let mut flesh = false;
    match melee_trace(a, reach(MeleeReach::Knife)) {
        Some((pos, n, surface)) => {
            let tex = melee_material_sound(surface, pos);
            flesh = tex == b'F';
            if hard(tex) {
                metal_hit(pos, n, 6.0, false);
                sound(a, CHAN_WEAPON, "weapons/knife/knife_hit.wav");
            }
        }
        None => sound(a, CHAN_WEAPON, "weapons/knife/knife_miss.wav"),
    }
    blade_anim(a, flesh, || set_a(a.i1)[15 + r::random_long(0, 2) as usize]);
}

/// `EV_TS_Katana` (`0x10044230`): `i1` is the swing's index into the katana's set A (15 = back
/// swing, trace -55), played after the trace. Only a played hit or miss sound rolls
/// `RandomLong(0, 1)`, after the trace, material sound and sparks (`0x10044611`, `0x1004462e`):
/// non-zero picks `hit.wav`/`miss.wav`, zero `hit1.wav`/`miss1.wav`.
pub fn katana(a: &Args) {
    let variant = || katana_variant(r::random_long(0, 1));
    let mut flesh = false;
    match melee_trace(a, reach(MeleeReach::Katana(a.i1))) {
        // As the knife (`0x10044585`..`0x100446ad`): the material sound on every hit, sparks and
        // `hit`/`hit1` only on hard surfaces.
        Some((pos, nrm, surface)) => {
            let tex = melee_material_sound(surface, pos);
            flesh = tex == b'F';
            if hard(tex) {
                metal_hit(pos, nrm, 12.0, true);
                sound(a, CHAN_WEAPON, &format!("weapons/katana/hit{}.wav", variant()));
            }
        }
        None => sound(a, CHAN_WEAPON, &format!("weapons/katana/miss{}.wav", variant())),
    }
    // TS reads `set[i1]` unchecked; the server only sends 15..18.
    blade_anim(a, flesh, || set_a(34).get(a.i1 as usize).copied().unwrap_or(0));
}

/// The katana's sample suffix for its `RandomLong(0, 1)`: non-zero `hit.wav`/`miss.wav`
/// (`0x100954c8`, `0x10095498`), zero `hit1.wav`/`miss1.wav` (`0x1004461e..0x10044644`).
fn katana_variant(roll: i32) -> &'static str {
    if roll != 0 { "" } else { "1" }
}

/// `EV_TS_CloseCombat` (`0x10043200`): block (`i1` 0, `f1` the weapon id) or kung-fu attack (`i1`
/// 1 punch, 2 kick, 3 back kick; `f1` 0). For both, the view owner plays set A of weapon `f1` at
/// index `i2`, body 0 (`0x100433bb..0x10043401`). A block then plays `player/block.wav`
/// (`0x100436fa`). An attack plays `player/closecombat.wav` first (`0x100435b6..0x10043619`), then
/// traces from the gun position along `forward` by [`close_combat_reach`] (`0x1004342c..0x100435b0`)
/// and on a hit plays the material sound and, except on flesh, the puff (`0x10043667..0x100436e3`;
/// the sky and an untextured surface, type 0, take the gun-smoke puff too: `0x1004369e..0x100436ad`).
pub fn close_combat(a: &Args) {
    if a.is_view_owner() {
        r::weapon_animation(close_combat_seq(&set_a(a.f1 as i32), a.i2), 0);
    }
    if a.i1 == 0 && crate::gameplay::current().close_combat_blocks() {
        sound(a, CHAN_WEAPON, "player/block.wav");
        return;
    }
    sound(a, CHAN_WEAPON, "player/closecombat.wav");
    if let Some((pos, n, surface)) = melee_trace(a, reach(MeleeReach::CloseCombat(a.i1))) {
        let tex = melee_material_sound(surface, pos);
        if tex != b'F' {
            puff(pos, n, tex);
        }
    }
}

/// The close-combat view sequence: `set[i2]`. TS reads it unchecked (`movswl 0x252(%ebx,%edi,2)`);
/// the servers send 0..11 for attacks and 1 or 0x12 for blocks, so out of range plays sequence 0.
fn close_combat_seq(set: &[i16; 29], i2: i32) -> i32 {
    usize::try_from(i2).ok().and_then(|i| set.get(i)).copied().unwrap_or(0) as i32
}

/// `EV_TS_ColdCock` (`0x10043800`): stock hit, set A `[28]` for the view owner (`0x10043a30`),
/// `player/closecombat.wav` (`0x10043b1f`), then a trace 30 ahead (`0x10043b69`).
pub fn coldcock(a: &Args) {
    if a.is_view_owner() {
        r::weapon_animation(set_a(a.i1)[28] as i32, weapon_body(a.i1, a.i2));
    }
    sound(a, CHAN_WEAPON, "player/closecombat.wav");
    if let Some((pos, n, surface)) = melee_trace(a, reach(MeleeReach::Coldcock(a.i1))) {
        let tex = melee_material_sound(surface, pos);
        if tex != b'F' {
            puff(pos, n, tex);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_combat_plays_set_a_at_the_index() {
        // Kung-fu attacks (f1 0): the server's indices 3..11 into set A.
        let kf = set_a(0);
        assert_eq!(close_combat_seq(&kf, 3), 2); // jab
        assert_eq!(close_combat_seq(&kf, 4), 10); // straight punch, stunt punch
        assert_eq!(close_combat_seq(&kf, 11), 7); // heavy punch
        // Blocks: kung fu sends 1, a weapon 0x12 with f1 = its id.
        assert_eq!(close_combat_seq(&kf, 1), 4);
        assert_eq!(close_combat_seq(&set_a(34), 0x12), set_a(34)[0x12] as i32);
        // Out of range (the port's old 10..25 move ids, -1) falls back to sequence 0.
        assert_eq!(close_combat_seq(&kf, -1), 0);
        assert_eq!(close_combat_seq(&kf, 29), 0);
    }

    /// Issue #77: `TS_MeleeMaterialSound`'s lists in table order (`0x10045f0b..0x10046054`).
    #[test]
    fn melee_material_lists() {
        let tile = ["player/pl_tile1.wav", "player/pl_tile3.wav", "player/pl_tile2.wav", "player/pl_tile4.wav"];
        assert_eq!(melee_material_samples(b'T'), tile);
        assert_eq!(melee_material_samples(b'S')[1..3], ["player/pl_slosh3.wav", "player/pl_slosh2.wav"]);
        assert_eq!(melee_material_samples(b'V'), ["player/pl_duct1.wav"; 2], "two entries: RandomLong(0, 1)");
        assert!(melee_material_samples(b'F').is_empty());
        let step = ["player/pl_step1.wav", "player/pl_step2.wav"];
        for tex in [0, b'C', b'X', b'E'] {
            assert_eq!(melee_material_samples(tex), step, "{tex}");
        }
        assert_eq!(melee_material_samples(b'Y'), melee_material_samples(b'P'));
    }

    /// The sky and an untextured hit (type 0) neither spark nor play the blades' hit sound.
    #[test]
    fn blades_skip_type_zero() {
        assert!(!hard(0) && !hard(b'F') && !hard(b'D') && !hard(b'W'));
        assert!(hard(b'C') && hard(b'S') && hard(b'M') && hard(b'X'));
        assert_eq!((katana_variant(1), katana_variant(0)), ("", "1"));
    }

    /// TS 3.0's reaches (`0x10043427..0x100434ab`: -55.0 at `0x1007d6fc`, 45 at `0x1007a900`,
    /// 35 at `0x1007a8d8`; coldcock 30 at `0x10043b69`).
    #[test]
    fn melee_reach_by_event() {
        let core = crate::gameplay::DEFAULTS.client;
        assert_eq!(core.melee_reach(MeleeReach::CloseCombat(1)), 35.0); // punch
        assert_eq!(core.melee_reach(MeleeReach::CloseCombat(2)), 45.0); // kick
        assert_eq!(core.melee_reach(MeleeReach::CloseCombat(3)), -55.0); // back kick: behind the player
        assert_eq!(core.melee_reach(MeleeReach::Coldcock(3)), 30.0);
        assert_eq!(core.melee_reach(MeleeReach::Knife), 35.0);
        assert_eq!(core.melee_reach(MeleeReach::Katana(15)), -55.0);
        assert_eq!(core.melee_reach(MeleeReach::Katana(16)), 50.0);
        assert!(core.close_combat_blocks() && core.reload_event_phases() && core.bullet_whizz_and_push());
    }
}
