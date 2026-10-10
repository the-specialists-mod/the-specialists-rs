//! Laser sights, flashlights and player shadows (transparent pass; rendering doc §3.3).
//!
//! Local state comes from prediction (`state.local_items` = `clientdata.vuser1[2]`), other
//! players' from `ActItems`/`PlInfo` (`state.active_items`/`laser_color`, indexed by entity number, HUD package).

use super::psys::Camera;
use crate::state;
use goldsrc::ext::render::{self as r, Vec3, vec3 as v};
use goldsrc::ext::tri;
use goldsrc::sys::*;

pub const ITEM_LASER: i32 = 2;
pub const ITEM_FLASHLIGHT: i32 = 4;
pub const ITEM_AKIMBO: i32 = 0x40;

/// `LaserColor(n)` (`0x100582f0`).
pub fn laser_color(n: i32) -> Vec3 {
    match n {
        1 => [0.0, 205.0 / 255.0, 0.0],
        2 => [0.0, 120.0 / 255.0, 1.0],
        _ => [1.0, 25.0 / 255.0, 25.0 / 255.0],
    }
}

pub struct Sprites {
    pub beam_anim: HSPRITE,
    pub beam: HSPRITE,
    pub dot: HSPRITE,
    pub flash: HSPRITE,
    pub shadow: HSPRITE,
    pub scope: HSPRITE,
}

impl Sprites {
    pub fn load() -> Sprites {
        Sprites {
            beam_anim: r::spr_load("sprites/laserbeamanim.spr"),
            beam: r::spr_load("sprites/laserbeam.spr"),
            dot: r::spr_load("sprites/laserdot.spr"),
            flash: r::spr_load("sprites/flash.spr"),
            shadow: r::spr_load("sprites/ombra.spr"),
            scope: r::spr_load(super::scope::SPRITE),
        }
    }
}

pub(super) fn set_sprite(h: HSPRITE, frame: i32) -> bool {
    if h == 0 {
        return false;
    }
    let m = r::sprite_model(h);
    !m.is_null() && tri::sprite_texture(m, frame)
}

/// A camera-facing strip from `a` to `b`.
fn beam_quad(cam: &Camera, a: Vec3, b: Vec3, width: f32, rgb: Vec3, alpha: f32) {
    let dir = v::sub(b, a);
    let mid = v::scale(v::add(a, b), 0.5);
    let side = v::normalize(v::cross(dir, v::sub(cam.origin, mid)));
    let w = v::scale(side, width * 0.5);
    tri::begin(TRI_QUADS as i32);
    tri::color4f(rgb[0], rgb[1], rgb[2], alpha);
    for (uv, p) in [([0.0, 0.0], v::sub(a, w)), ([1.0, 0.0], v::sub(b, w)), ([1.0, 1.0], v::add(b, w)), ([0.0, 1.0], v::add(a, w))] {
        tri::tex_coord2f(uv[0], uv[1]);
        tri::vertex3fv(&p);
    }
    tri::end();
}

fn sprite_quad(cam: &Camera, org: Vec3, size: f32, rgb: Vec3, alpha: f32) {
    let r_ = v::scale(cam.right, size);
    let u_ = v::scale(cam.up, size);
    tri::begin(TRI_QUADS as i32);
    tri::color4f(rgb[0], rgb[1], rgb[2], alpha);
    for (uv, p) in [
        ([0.0, 0.0], v::add(v::sub(org, r_), u_)),
        ([1.0, 0.0], v::add(v::add(org, r_), u_)),
        ([1.0, 1.0], v::sub(v::add(org, r_), u_)),
        ([0.0, 1.0], v::sub(v::sub(org, r_), u_)),
    ] {
        tri::tex_coord2f(uv[0], uv[1]);
        tri::vertex3fv(&p);
    }
    tri::end();
}

fn trace_world(a: Vec3, b: Vec3, ignore_player: i32) -> pmtrace_s {
    r::with_solid_players(ignore_player, || r::player_trace(a, b, 2, PM_STUDIO_BOX as i32, -1))
}

/// `GetCullOrigin` (`0x10057270`): the eye in third person, otherwise the view origin.
fn cull_origin(cam: &Camera) -> Vec3 {
    if crate::view::is_third_person() { crate::view::eye() } else { cam.origin }
}

/// The local laser beams' directions (`DrawLocalLaser` `0x1003bb00`): view-model attachment
/// 0 → 1 (2 → 3 for the akimbo second gun) unless there is no view model or `cl_laser` is 1, then
/// the view forward. TS jitters x and y only, by one random pair per frame (swapped for the second
/// gun), without renormalising; a degenerate attachment pair gives (0, 0, 1).
fn beam_dirs(vm: Option<&cl_entity_s>, cam: &Camera, cl_laser: f32, jitter: [f32; 2]) -> [(Option<Vec3>, Vec3); 2] {
    let Some(vm) = vm.filter(|_| cl_laser != 1.0) else { return [(None, cam.forward); 2] };
    let dir = |a: usize, j: [f32; 2]| {
        let d = v::sub(vm.attachment[a + 1], vm.attachment[a]);
        let d = if v::len(d) == 0.0 { [0.0, 0.0, 1.0] } else { v::normalize(d) };
        (Some(vm.attachment[a]), [d[0] + j[0] * 0.003, d[1] + j[1] * 0.003, d[2]])
    };
    [dir(0, jitter), dir(2, [jitter[1], jitter[0]])]
}

/// `DrawLocalLaser` (`0x1003bb00`) and the local flashlight (`0x1003b250`).
///
/// The laser is traced from the cull origin (offset 4 units right/left per gun with akimbo), not
/// from the view model. The hit point minus `2·v_forward` is projected for the HUD dot, and the
/// first gun's range from the cull origin ×0.025 is the number the crosshair shows.
pub fn draw_local(s: &Sprites, cam: &Camera, now: f64, cl_laser: f32, color: i32) {
    let (items, dead, ts) = state::with(|st| (st.local_items, st.dead, st.time_scale));
    state::with(|st| st.laser_dots = [None, None]);
    if dead || items & (ITEM_LASER | ITEM_FLASHLIGHT) == 0 {
        return;
    }
    let local = r::local_player().map_or(0, |e| e.index);
    let vm = crate::events::first_person_view_model();
    let akimbo = items & ITEM_AKIMBO != 0;
    if items & ITEM_LASER != 0 {
        let rgb = laser_color(color);
        let jitter = [r::random_float(-1.0, 1.0), r::random_float(-1.0, 1.0)];
        let beams = beam_dirs(vm.as_deref(), cam, cl_laser, jitter);
        let cull = cull_origin(cam);
        let guns: &[(usize, f32)] = if akimbo { &[(0, 4.0), (1, -4.0)] } else { &[(0, 0.0)] };
        for &(i, side) in guns {
            let (beam_origin, d) = beams[i];
            let start = v::ma(cull, side, cam.right);
            let tr = trace_world(start, v::ma(start, 8192.0, d), local - 1);
            let hit = tr.endpos;
            let dot = r::world_to_screen(v::ma(hit, -2.0, cam.forward));
            state::with(|st| {
                st.laser_dots[i] = dot;
                if i == 0 {
                    st.laser_range = (v::dist(hit, cull) * 0.025) as i32;
                }
            });
            let frames = r::spr_frames(s.beam_anim).max(1);
            let frame = ((now * 20.0 * ts as f64) as i32).rem_euclid(frames);
            if let Some(o) = beam_origin
                && set_sprite(s.beam_anim, frame)
            {
                tri::render_mode(kRenderTransAdd as i32);
                tri::cull_face(TRICULLSTYLE_TRI_NONE);
                let end = v::ma(o, v::dist(o, hit).min(96.0), v::normalize(d));
                beam_quad(cam, o, end, 0.6, rgb, 0.6);
            }
        }
    }
    if items & ITEM_FLASHLIGHT != 0 {
        let tr = trace_world(cam.origin, v::ma(cam.origin, 1024.0, cam.forward), local - 1);
        let d = v::dist(cam.origin, tr.endpos);
        if let Some(dl) = r::alloc_dlight(2) {
            dl.origin = v::ma(tr.endpos, -8.0, cam.forward);
            dl.radius = (1024.0 - d).clamp(0.0, 120.0);
            dl.color.r = 255;
            dl.color.g = 255;
            dl.color.b = 255;
            dl.die = r::client_time() + 0.1;
        }
        if let Some(vm) = &vm
            && set_sprite(s.flash, 0)
        {
            tri::render_mode(kRenderTransAdd as i32);
            sprite_quad(cam, vm.attachment[0], 2.0, [1.0, 1.0, 1.0], 0.5);
            if akimbo {
                sprite_quad(cam, vm.attachment[2], 2.0, [1.0, 1.0, 1.0], 0.5);
            }
        }
    }
    tri::render_mode(kRenderNormal as i32);
}

/// `DrawOtherLasers` (`0x1003d600`) / `DrawOtherFlashlights` (`0x1003b750`).
pub fn draw_others(s: &Sprites, cam: &Camera, visible: &[bool; 33]) {
    let local = r::local_player().map_or(0, |e| e.index);
    let max = r::max_clients().min(32);
    let (items, colors) = state::with(|st| (st.active_items.clone(), st.laser_color.clone()));
    for idx in 1..=max {
        if idx == local || !visible[idx as usize] {
            continue;
        }
        let it = items.get(idx as usize).copied().unwrap_or(0) as i32;
        if it & (ITEM_LASER | ITEM_FLASHLIGHT) == 0 {
            continue;
        }
        let Some(e) = r::entity(idx) else { continue };
        if e.curstate.messagenum != r::local_player().map_or(0, |l| l.curstate.messagenum) {
            continue; // not in this packet
        }
        let a0 = e.attachment[0];
        let a1 = e.attachment[1];
        let dir = v::normalize(v::sub(a1, a0));
        if v::len(dir) < 0.5 {
            continue;
        }
        let tr = trace_world(a0, v::ma(a0, 8192.0, dir), idx - 1);
        if it & ITEM_LASER != 0 {
            let rgb = laser_color(colors.get(idx as usize).copied().unwrap_or(0) as i32);
            if set_sprite(s.dot, 0) {
                tri::render_mode(kRenderTransAdd as i32);
                tri::cull_face(TRICULLSTYLE_TRI_NONE);
                sprite_quad(cam, v::ma(tr.endpos, -1.0, dir), 4.0, rgb, 1.0);
            }
            if set_sprite(s.beam, 0) {
                beam_quad(cam, a0, a1, 0.6, rgb, 0.6);
            }
        }
        if it & ITEM_FLASHLIGHT != 0 {
            let d = v::dist(a0, tr.endpos);
            if let Some(dl) = r::alloc_dlight(idx + 3) {
                dl.origin = v::ma(tr.endpos, -8.0, dir);
                dl.radius = (1024.0 - d).clamp(0.0, 120.0);
                dl.color.r = 255;
                dl.color.g = 255;
                dl.color.b = 255;
                dl.die = r::client_time() + 0.1;
            }
        }
    }
    tri::render_mode(kRenderNormal as i32);
}

/// `DrawPlayerShadows` (`0x10055760`): `ombra.spr` quad on the floor below visible players.
pub fn draw_shadows(s: &Sprites, cam: &Camera, visible: &[bool; 33], cl_shadow: f32, third_person: bool) {
    if cl_shadow <= 0.0 || !set_sprite(s.shadow, 0) {
        return;
    }
    let local = r::local_player().map_or(0, |e| e.index);
    tri::render_mode(kRenderTransAlpha as i32);
    tri::cull_face(TRICULLSTYLE_TRI_NONE);
    for idx in 1..=r::max_clients().min(32) {
        let is_local = idx == local;
        if is_local {
            let dead = state::with(|st| st.dead);
            if dead || !(third_person || cl_shadow == 1.0) {
                continue;
            }
        } else if !visible[idx as usize] {
            continue;
        }
        let Some(e) = r::entity(idx) else { continue };
        if e.model.is_null() {
            continue;
        }
        let org = e.origin;
        let tr = r::player_trace(org, v::add(org, [0.0, 0.0, -8192.0]), 2, (PM_STUDIO_IGNORE | PM_WORLD_ONLY) as i32, -1);
        let d = org[2] - tr.endpos[2] - 36.0;
        if !(0.0..200.0).contains(&d) {
            continue;
        }
        let alpha = 1.0 - d.max(0.0) * 0.005;
        let n = tr.plane.normal;
        let tangent = v::normalize(v::cross(n, if n[2].abs() < 0.9 { [0.0, 0.0, 1.0] } else { [1.0, 0.0, 0.0] }));
        let bit = v::cross(n, tangent);
        let c = v::ma(tr.endpos, 0.5, n);
        let (a, b) = (v::scale(tangent, 24.0), v::scale(bit, 24.0));
        let _ = cam;
        tri::begin(TRI_QUADS as i32);
        tri::color4f(1.0, 1.0, 1.0, alpha);
        for (uv, p) in [
            ([0.0, 0.0], v::sub(v::sub(c, a), b)),
            ([1.0, 0.0], v::sub(v::add(c, a), b)),
            ([1.0, 1.0], v::add(v::add(c, a), b)),
            ([0.0, 1.0], v::add(v::sub(c, a), b)),
        ] {
            tri::tex_coord2f(uv[0], uv[1]);
            tri::vertex3fv(&p);
        }
        tri::end();
    }
    tri::render_mode(kRenderNormal as i32);
}
