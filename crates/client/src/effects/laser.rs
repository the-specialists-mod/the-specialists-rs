//! Laser sights, flashlights and player shadows (transparent pass; rendering doc §3.3).
//!
//! The local player's beams and flashlight read `CHudTSWeapons+0x14` (`state.hud_items`: the
//! local `ActItems` bits, 0x40 = akimbo, as the HUD last drew them), as TS does. Other players'
//! come from `ActItems`/`PlInfo` (`state.active_items`/`laser_color`, indexed by entity number).

use super::psys::{Camera, ORIENT_FREE, Particle, build_quad};
use crate::state;
use goldsrc::Global;
use goldsrc::client::engine;
use goldsrc::ext::render::{self as r, Vec3, vec3 as v};
use goldsrc::ext::tri;
use goldsrc::sys::*;
use std::cell::RefCell;

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

/// The laser code's globals.
struct LaserState {
    /// `0x101899b4`: the local beam's animation frame.
    beam_frame: f32,
    /// `0x10189960` / `0x10189980`: `V_GetInEyePos`'s output for the other lasers. TS adds the
    /// jitter to the stored angles, and a failed lookup leaves both as they were.
    eye_origin: Vec3,
    eye_angles: Vec3,
    /// `0x10189578` / `0x10189568`: the same for the other flashlights.
    light_origin: Vec3,
    light_angles: Vec3,
    /// `0x10189638`: each player's smoothed laser direction.
    dirs: [Vec3; 33],
    /// `ts_dev_fxlog` throttles (effects clock): local beams, other players' beams.
    next_log: [f64; 2],
}

static STATE: Global<RefCell<LaserState>> = Global(RefCell::new(LaserState {
    beam_frame: 0.0,
    eye_origin: [0.0; 3],
    eye_angles: [0.0; 3],
    light_origin: [0.0; 3],
    light_angles: [0.0; 3],
    dirs: [[0.0; 3]; 33],
    next_log: [0.0; 2],
}));

fn with<R>(f: impl FnOnce(&mut LaserState) -> R) -> R {
    f(&mut STATE.0.borrow_mut())
}

/// At most two `[ts fx] laser` batches a second per kind (0 local, 1 others): the beams draw
/// every frame. A clock that went back (a new map) restarts the throttle.
fn log_due(kind: usize, now: f64) -> bool {
    with(|s| {
        let next = &mut s.next_log[kind];
        if now < *next && *next - now < 1.0 {
            return false;
        }
        *next = now + 0.5;
        true
    })
}

/// A sprite through the particle system's cache (`PSys_LoadSprite`), as TS loads every effect
/// sprite each time it draws: (name, render mode key) -> handle.
fn load(name: &str, mode: u32) -> Option<HSPRITE> {
    super::with_psys(|ps| ps.sprite(name, mode as i32).map(|s| ps.sprite_handle(s)))
}

pub(super) fn set_sprite(h: HSPRITE, frame: i32) -> bool {
    if h == 0 {
        return false;
    }
    let m = r::sprite_model(h);
    !m.is_null() && tri::sprite_texture(m, frame)
}

/// One TS beam quad inside a `TRI_QUADS` batch: `far ∓ w` (texture s 0 and 1 at t 0), then
/// `near ± w` (t 1), `w` being the half width along the view's up vector (`0x1003c8c0`,
/// `0x1003d280`).
fn beam_verts(far: Vec3, near: Vec3, w: Vec3) {
    for (uv, p) in
        [([0.0, 0.0], v::sub(far, w)), ([1.0, 0.0], v::add(far, w)), ([1.0, 1.0], v::add(near, w)), ([0.0, 1.0], v::sub(near, w))]
    {
        tri::tex_coord2f(uv[0], uv[1]);
        tri::vertex3fv(&p);
    }
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

/// TS's laser and flashlight traces: players solid except `player` (entity index), the
/// `PM_STUDIO_BOX` flag, hull 2, ignoring physent `player`.
fn trace_from(a: Vec3, b: Vec3, player: i32) -> pmtrace_s {
    r::with_solid_players(player - 1, || r::player_trace(a, b, 2, PM_STUDIO_BOX as i32, player))
}

/// The local beam's animation frame (`0x101899b4`) one frame on (`0x1003c628..0x1003c672`):
/// `g_flTimeScale · frametime · 20` added, so it slows down with slow motion; back to 0 once its
/// whole part is negative or past the sprite's last frame.
fn beam_frame_step(frame: f32, time_scale: f32, frametime: f32, frames: i32) -> f32 {
    let next = time_scale * frametime * 20.0 + frame;
    let f = next as i32;
    if f < 0 || f > frames - 1 { 0.0 } else { next }
}

/// `GetCullOrigin` (`0x10057270`): the eye in third person, otherwise the view origin.
fn cull_origin(cam: &Camera) -> Vec3 {
    if crate::view::is_third_person() { crate::view::eye() } else { cam.origin }
}

/// The local laser directions (`DrawLocalLaser` `0x1003bb00`): view-model attachment 0 → 1
/// (2 → 3 for the akimbo second gun) unless there is no view model or `cl_laser` is 1, then the
/// view forward. TS jitters x and y only, by one random pair per frame (swapped for the second
/// gun), without renormalising; a degenerate attachment pair gives (0, 0, 1).
fn beam_dirs(vm: Option<&cl_entity_s>, cam: &Camera, cl_laser: f32, jitter: [f32; 2]) -> [Vec3; 2] {
    let Some(vm) = vm.filter(|_| cl_laser != 1.0) else { return [cam.forward; 2] };
    let dir = |a: usize, j: [f32; 2]| {
        let d = v::sub(vm.attachment[a + 1], vm.attachment[a]);
        let d = if v::len(d) == 0.0 { [0.0, 0.0, 1.0] } else { v::normalize(d) };
        [d[0] + j[0] * 0.003, d[1] + j[1] * 0.003, d[2]]
    };
    [dir(0, jitter), dir(2, [jitter[1], jitter[0]])]
}

/// The local beams' brightness and width factor (`0x1003c741`): `1 − max(128 − d, 0) / 128` for
/// the first gun's hit distance `d` from the cull origin, so a beam fades in over the first 128
/// units.
fn local_beam_fade(d: f32) -> f32 {
    1.0 - (128.0 - d).max(0.0) * 0.0078125
}

/// `DrawLocalLaser` (`0x1003bb00`), gated on the HUD's item bits (laser `& 2`, second gun
/// `& 0x40`).
///
/// The laser is traced from the cull origin (offset 4 units right/left per gun with akimbo), not
/// from the view model, ignoring the local player. The hit point minus `2·v_forward` is projected
/// for the HUD dot, and the first gun's range from the cull origin ×0.025 is the number the
/// crosshair shows. With a view model (first person, not zoomed in) and `laserbeamanim.spr`, each
/// gun also gets a beam from its view-model muzzle (attachment 0, or 2 for the second gun, one
/// unit down) to its hit point less twice its direction: one additive quad batch, colour and
/// width scaled by [`local_beam_fade`], the sprite animated at 20 frames a second of slowed time
/// ([`beam_frame_step`]).
pub fn draw_local_laser(cam: &Camera, now: f64, frametime: f32, time_scale: f32, cl_laser: f32) {
    let local = engine().local_player().map_or(0, |e| e.index);
    let (items, color) =
        state::with(|st| (st.hud_items, st.laser_color.get(usize::try_from(local).unwrap_or(0)).copied().unwrap_or(0) as i32));
    state::with(|st| st.laser_dots = [None, None]);
    if items & ITEM_LASER == 0 {
        return;
    }
    let vm = crate::events::first_person_view_model();
    let akimbo = items & ITEM_AKIMBO != 0;
    let jitter = [r::random_float(-1.0, 1.0), r::random_float(-1.0, 1.0)];
    let dirs = beam_dirs(vm.as_deref(), cam, cl_laser, jitter);
    let cull = cull_origin(cam);
    let guns: &[(usize, f32)] = if akimbo { &[(0, 4.0), (1, -4.0)] } else { &[(0, 0.0)] };
    let mut hits = [[0.0; 3]; 2];
    for &(i, side) in guns {
        let start = v::ma(cull, side, cam.right);
        let hit = trace_from(start, v::ma(start, 8192.0, dirs[i]), local).endpos;
        hits[i] = hit;
        let dot = engine().world_to_screen_ndc(v::ma(hit, -2.0, cam.forward));
        state::with(|st| {
            st.laser_dots[i] = dot;
            if i == 0 {
                st.laser_range = (v::dist(hit, cull) * 0.025) as i32;
            }
        });
    }
    let Some(vm) = vm else { return };
    let Some(beam_anim) = load("sprites/laserbeamanim.spr", kRenderTransAdd) else { return };
    let frames = r::spr_frames(beam_anim);
    let frame = with(|s| {
        s.beam_frame = beam_frame_step(s.beam_frame, time_scale, frametime, frames);
        s.beam_frame as i32
    });
    if !set_sprite(beam_anim, frame) {
        return;
    }
    let fade = local_beam_fade(v::dist(hits[0], cull));
    let rgb = laser_color(color);
    let w = v::scale(cam.up, 0.5 * fade);
    let log = log_due(0, now);
    tri::render_mode(kRenderTransAdd as i32);
    tri::cull_face(TRICULLSTYLE_TRI_NONE);
    tri::begin(TRI_QUADS as i32);
    tri::color4f(rgb[0] * fade, rgb[1] * fade, rgb[2] * fade, 1.0);
    for &(i, _) in guns {
        let muzzle = vm.attachment[i * 2];
        let start = v::sub(muzzle, cam.up);
        let end = v::ma(hits[i], -2.0, dirs[i]);
        beam_verts(end, start, w);
        if log {
            let off = v::sub(start, cam.origin);
            crate::events::fx_log(|| {
                format!(
                    "laser local gun {i} beam {} to {} muzzle {} ahead {:.1} right {:.1} up {:.1}",
                    fmt(start),
                    fmt(end),
                    fmt(muzzle),
                    v::dot(off, cam.forward),
                    v::dot(off, cam.right),
                    v::dot(off, cam.up)
                )
            });
        }
    }
    tri::end();
    tri::render_mode(kRenderNormal as i32);
}

fn fmt(p: Vec3) -> String {
    format!("[{:.1}, {:.1}, {:.1}]", p[0], p[1], p[2])
}

/// The local flashlight's light radius (`0x1003b3f1..`): `1024 − d` for the hit `d` units from
/// the cull origin, at most 120.
fn local_flashlight_radius(d: f32) -> f32 {
    let r = 1024.0 - d;
    if 120.0 <= r { 120.0 } else { r }
}

/// The local flashlight (`DrawLocalFlashlight` `0x1003b250`; 2.1's `0x10070f20` is the same),
/// gated on the HUD's item bits (`& 4`, the second sprite `& 0x40`). A 1024-unit trace from the
/// cull origin along the view forward (hull 2, `PM_STUDIO_BOX`, ignoring the local player); when
/// it hits (`fraction != 1`), a dynamic light (key 2) at the hit with radius
/// [`local_flashlight_radius`]. With a view model, `flash.spr` billboards one unit below the
/// muzzle (attachment 0, radius 6; the second gun's attachment 2, radius 4), colour 0.7, alpha 1
/// (issue #203).
pub fn draw_local_flashlight(cam: &Camera) {
    let items = state::with(|st| st.hud_items);
    if items & ITEM_FLASHLIGHT == 0 {
        return;
    }
    let cull = cull_origin(cam);
    let tr = trace_from(cull, v::ma(cull, 1024.0, cam.forward), engine().local_player().map_or(0, |e| e.index));
    if tr.fraction != 1.0 {
        let d = v::dist(tr.endpos, cull);
        if let Some(dl) = r::alloc_dlight(2) {
            dl.origin = tr.endpos;
            dl.radius = local_flashlight_radius(d);
            dl.color.r = 255;
            dl.color.g = 255;
            dl.color.b = 255;
            dl.die = engine().client_time() + 0.1;
        }
    }
    if let Some(vm) = crate::events::first_person_view_model()
        && let Some(flash) = load("sprites/flash.spr", kRenderTransAdd)
        && set_sprite(flash, 0)
    {
        tri::render_mode(kRenderTransAdd as i32);
        tri::cull_face(TRICULLSTYLE_TRI_NONE);
        sprite_quad(cam, v::sub(vm.attachment[0], cam.up), 6.0, [0.7; 3], 1.0);
        if items & ITEM_AKIMBO != 0 {
            sprite_quad(cam, v::sub(vm.attachment[2], cam.up), 4.0, [0.7; 3], 1.0);
        }
        tri::render_mode(kRenderNormal as i32);
    }
}

/// The other players with `item` on (`ActItems`) whose entity is a player the last
/// `HUD_AddEntity` pass found visible: entity indices 0..=maxclients but the local player.
fn others_with(item: i32, visible: &[bool; 33]) -> Vec<i32> {
    let local = engine().local_player().map_or(0, |e| e.index);
    let items = state::with(|st| st.active_items.clone());
    (0..=engine().max_clients().min(32))
        .filter(|&idx| {
            idx != local
                && items.get(idx as usize).copied().unwrap_or(0) as i32 & item != 0
                && engine().entity(idx).is_some_and(|e| e.player != 0)
                && visible[idx as usize]
        })
        .collect()
}

/// `DrawOtherLasers`' direction (`0x1003d80e`): the in-eye view (jittered by up to 0.05° in pitch
/// and yaw), unless it is more than ~45° off the line from the player's cached attachment 0 to 1
/// (the `p_` model's right hand), then that line; eased toward at `15·frametime` per frame and
/// renormalised.
fn other_laser_dir(aim: Vec3, hand: Vec3, prev: Vec3, frametime: f32) -> Vec3 {
    let hand = v::normalize(hand);
    let f = if v::dot(aim, hand) < 0.7 { hand } else { aim };
    let k = (frametime * 15.0).min(1.0);
    v::normalize(v::add(prev, v::scale(v::sub(f, prev), k)))
}

/// `DrawOtherLasers` (`0x1003d600`): nothing at all unless both `laserdot.spr` and
/// `laserbeam.spr` load (`0x1003d6e6`, `0x1003d705`). For each other visible player with the
/// laser on: the direction from [`other_laser_dir`], traced 8192 units from the player's in-eye
/// position (`V_GetInEyePos`); `laserdot.spr` (size 4) two units short of the hit; and, if
/// `laserbeam.spr` binds, a beam from 16 units ahead of the player's cached attachment 0 (the
/// gun hand) to 16 units short of the dot. One beam per player, akimbo or not: TS has no second
/// world beam.
pub fn draw_other_lasers(cam: &Camera, visible: &[bool; 33], now: f64, frametime: f32) {
    let Some(dot) = load("sprites/laserdot.spr", kRenderTransAdd) else { return };
    let Some(beam) = load("sprites/laserbeam.spr", kRenderTransAdd) else { return };
    let jitter = [r::random_float(-1.0, 1.0), r::random_float(-1.0, 1.0)];
    let colors = state::with(|st| st.laser_color.clone());
    let mut log = None;
    for idx in others_with(ITEM_LASER, visible) {
        if !set_sprite(dot, 0) {
            continue;
        }
        let (eye, angles) = with(|s| {
            crate::spectator::in_eye_position(idx, &mut s.eye_origin, &mut s.eye_angles);
            s.eye_angles[0] += jitter[0] * 0.05;
            s.eye_angles[1] += jitter[1] * 0.05;
            (s.eye_origin, s.eye_angles)
        });
        let aim = v::angle_vectors(angles).0;
        let hand = super::player_attachment(idx, 0);
        let hand_dir = v::sub(super::player_attachment(idx, 1), hand);
        let dir = with(|s| {
            let d = other_laser_dir(aim, hand_dir, s.dirs[idx as usize], frametime);
            s.dirs[idx as usize] = d;
            d
        });
        let tr = trace_from(eye, v::ma(eye, 8192.0, dir), idx);
        let dot_at = v::ma(tr.endpos, -2.0, dir);
        let rgb = laser_color(colors.get(idx as usize).copied().unwrap_or(0) as i32);
        tri::render_mode(kRenderTransAdd as i32);
        tri::cull_face(TRICULLSTYLE_TRI_NONE);
        sprite_quad(cam, dot_at, 4.0, rgb, 1.0);
        tri::render_mode(kRenderNormal as i32);
        if set_sprite(beam, 0) {
            let (near, far) = (v::ma(hand, 16.0, dir), v::ma(dot_at, -16.0, dir));
            tri::render_mode(kRenderTransAdd as i32);
            tri::cull_face(TRICULLSTYLE_TRI_NONE);
            tri::begin(TRI_QUADS as i32);
            tri::color4f(rgb[0], rgb[1], rgb[2], 1.0);
            beam_verts(far, near, v::scale(cam.up, 0.5));
            tri::end();
            tri::render_mode(kRenderNormal as i32);
            if *log.get_or_insert_with(|| log_due(1, now)) {
                let origin = engine().entity(idx).map_or([0.0; 3], |e| e.origin);
                crate::events::fx_log(|| {
                    format!(
                        "laser other #{idx} beam {} to {} hand {} origin {} eye {} aim {} hand-line {} dir {}",
                        fmt(near),
                        fmt(far),
                        fmt(hand),
                        fmt(origin),
                        fmt(eye),
                        fmt(aim),
                        fmt(v::normalize(hand_dir)),
                        fmt(dir)
                    )
                });
            }
        }
    }
}

/// `DrawOtherFlashlights` (`0x1003b750`): for each other visible player with the flashlight on,
/// a 1024-unit trace along the in-eye view (`V_GetInEyePos`, no jitter); when it hits, a dynamic
/// light (key `index + 3`) at the hit, radius `1024 − d` capped at 120, `d` the hit's distance from
/// the cull origin.
pub fn draw_other_flashlights(cam: &Camera, visible: &[bool; 33]) {
    let cull = cull_origin(cam);
    for idx in others_with(ITEM_FLASHLIGHT, visible) {
        let (org, angles) = with(|s| {
            crate::spectator::in_eye_position(idx, &mut s.light_origin, &mut s.light_angles);
            (s.light_origin, s.light_angles)
        });
        let fwd = v::angle_vectors(angles).0;
        let tr = trace_from(org, v::ma(org, 1024.0, fwd), idx);
        if tr.fraction == 1.0 {
            continue;
        }
        let d = v::dist(tr.endpos, cull);
        if let Some(dl) = r::alloc_dlight(idx + 3) {
            dl.origin = tr.endpos;
            // TS lets the radius go negative far away (no light); the port stops at 0.
            dl.radius = (1024.0 - d).clamp(0.0, 120.0);
            dl.color.r = 255;
            dl.color.g = 255;
            dl.color.b = 255;
            dl.die = engine().client_time() + 0.1;
        }
    }
}

/// `DrawPlayerShadows` (`0x10055760`): one TransAlpha batch of `ombra.spr` quads (any non-zero
/// `cl_shadow`, `0x100557f6`). In first person, not observing and with `cl_shadow` exactly 1, one
/// under the view (`v_origin`), its distance less 12; then one under every player the last
/// `HUD_AddEntity` pass found visible (`0x100579a0`, slots 0..=maxclients; `DrawPlayerShadow`
/// `0x10055500`). Each traces 8192 units straight down with the point hull and `PM_NORMAL`,
/// ignoring physent number = the player's entity index, against whatever physents the engine has
/// set up, and lays a 24-unit quad facing `VecToAngles(normal)` one unit off the floor, at alpha
/// `1 − d·0.005` while the floor is less than 200 units down.
pub fn draw_shadows(cam: &Camera, visible: &[bool; 33], cl_shadow: f32, third_person: bool) {
    if cl_shadow == 0.0 {
        return;
    }
    let Some(shadow) = load("sprites/ombra.spr", kRenderTransTexture) else { return };
    if !set_sprite(shadow, 0) {
        return;
    }
    tri::render_mode(kRenderTransAlpha as i32);
    tri::cull_face(TRICULLSTYLE_TRI_NONE);
    tri::begin(TRI_QUADS as i32);
    let local = engine().local_player().map_or(0, |e| e.index);
    let shadow_quad = |from: Vec3, ignore: i32, less: f32| {
        let tr = r::player_trace(from, v::add(from, [0.0, 0.0, -8192.0]), 2, PM_NORMAL as i32, ignore);
        let n = tr.plane.normal;
        let angles = super::fx::vec_to_angles(n);
        let d = v::dist(tr.endpos, from) - less;
        if d < 200.0 {
            tri::color4f(1.0, 1.0, 1.0, 1.0 - d * 0.005);
            let p = Particle { origin: v::add(tr.endpos, n), angles, radius: 24.0, orient: ORIENT_FREE, ..Default::default() };
            for (uv, vtx) in [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]].iter().zip(build_quad(&p, cam).iter()) {
                tri::tex_coord2f(uv[0], uv[1]);
                tri::vertex3fv(vtx);
            }
        }
    };
    if !third_person && !crate::hud::observer() && cl_shadow == 1.0 {
        shadow_quad(cam.origin, local, 12.0);
    }
    for idx in 0..=engine().max_clients() {
        if !usize::try_from(idx).ok().and_then(|i| visible.get(i)).copied().unwrap_or(false) {
            continue;
        }
        let Some(e) = engine().entity(idx).filter(|e| e.player != 0) else { continue };
        shadow_quad(e.origin, idx, 0.0);
    }
    tri::end();
    tri::render_mode(kRenderNormal as i32);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_beam_animates_with_the_time_scale() {
        // 20 frames a second at normal speed, 5 at a quarter.
        assert_eq!(beam_frame_step(0.0, 1.0, 0.1, 10), 2.0);
        assert_eq!(beam_frame_step(0.0, 0.25, 0.1, 10), 0.5);
        assert_eq!(beam_frame_step(8.5, 1.0, 0.025, 10), 9.0, "the last frame is kept");
        assert_eq!(beam_frame_step(9.5, 1.0, 0.05, 10), 0.0, "past the last frame: back to 0");
        assert_eq!(beam_frame_step(0.0, 1.0, 0.1, 1), 0.0, "a one-frame sprite stays on 0");
    }

    /// `0x1003b3f1..`: `1024 − d`, capped at 120 (issue #203).
    #[test]
    fn local_flashlight_radius_is_capped_at_120() {
        assert_eq!(local_flashlight_radius(0.0), 120.0);
        assert_eq!(local_flashlight_radius(904.0), 120.0);
        assert_eq!(local_flashlight_radius(1000.0), 24.0);
        assert_eq!(local_flashlight_radius(1024.0), 0.0);
    }

    #[test]
    fn local_beam_fades_in_over_128_units() {
        assert_eq!(local_beam_fade(0.0), 0.0);
        assert_eq!(local_beam_fade(64.0), 0.5);
        assert_eq!(local_beam_fade(128.0), 1.0);
        assert_eq!(local_beam_fade(4000.0), 1.0);
    }

    #[test]
    fn other_laser_follows_the_aim_unless_the_hand_points_away() {
        let aim = [1.0, 0.0, 0.0];
        // Hand line within ~45° of the aim: the aim (no easing: frametime·15 >= 1).
        let d = other_laser_dir(aim, [1.0, 0.5, 0.0], [0.0; 3], 0.1);
        assert!(v::dist(d, aim) < 1e-6);
        // Hand pointing up (TS's akimbo idle pose holds the guns up): the hand line.
        let d = other_laser_dir(aim, [0.0, 0.0, 3.0], [0.0; 3], 0.1);
        assert!(v::dist(d, [0.0, 0.0, 1.0]) < 1e-6);
        // Eased at 15·frametime per frame, renormalised.
        let d = other_laser_dir(aim, aim, [0.0, 1.0, 0.0], 1.0 / 60.0);
        assert!((v::len(d) - 1.0).abs() < 1e-6 && d[0] > 0.0 && d[1] > d[0]);
    }
}
