//! TS 2.1's client bullets (issue #165): the slow-motion and `realbullet` bullets the fire events
//! queue. Frame 0x1006a070, trace step 0x1006a3f0, draw 0x10069540, models 0x10069160 (load) and
//! 0x10069450 (add), trails 0x1007d8b0 / 0x1007dc70 / 0x1007de30. f64 stands in for the x87
//! stack where TS keeps a value on it.

use goldsrc::ext::render::{Vec3, vec3 as v};
use goldsrc::sys::kRenderTransAdd;
use ts_client::effects::{self, fx, psys};
use ts_client::events::{BulletDraw, BulletModel, BulletStep, ClientBullet};

const ADD: i32 = kRenderTransAdd as i32;

/// Degrees to radians, the double at 0x10270240.
const DEG: f64 = 0.017453292519943295;

/// The bullet models 0x10069160 loads with `CL_LoadModel`; any one missing turns the model path
/// off (`+0x1d34d`).
pub const MODELS: [&str; 3] = ["models/bullet.mdl", "models/long_bullet.mdl", "models/pellet.mdl"];

/// The draw's model slots: entities 1..31 (`+0x1d308 < 0x20`, from 1 each frame).
const MAX_MODELS: usize = 31;

/// x87 `fcomp` + `test ah, 0x41` / `jp`: "greater than", with an unordered compare (a NaN) taken
/// as greater too.
#[allow(clippy::neg_cmp_op_on_partial_ord)] // the unordered case must come out true
fn gt(a: f32, b: f32) -> bool {
    !(a <= b)
}

/// The slow-motion factor the bullets scale by (0x1006a40d.., 0x10069808..): the raw `TSSlowMo`
/// factor (1 at normal speed) divided by 5 in bullet time.
pub fn speed_factor(slowmo_factor100: f32, bullet_time: bool) -> f32 {
    let f = slowmo_factor100 / 100.0;
    let s = if f == 0.0 { 1.0 } else { f / 1.0 };
    if bullet_time { s / 5.0 } else { s }
}

/// `EV_TS_FireBullets` (0x1007c5e0) queues client bullets when `SrvSett 0` (`realbullet`) is
/// exactly 1 or a `TSSlowMo` factor is set.
pub fn queues(realbullet: i32, slowmo_factor100: f32) -> bool {
    realbullet == 1 || slowmo_factor100 / 100.0 != 0.0
}

/// The frame (0x1006a0f8..) steps a bullet when its time is past and then sets the next step
/// 0.1 s on, whatever `realbullet` is.
pub fn next_update(now: f32) -> f32 {
    (now as f64 + 0.1) as f32
}

/// The trace step (0x1006a3f0): a fixed `dir × velocity / 0.25 × s` (`FUN_10069fd0` divides),
/// each vector operation rounded to floats: `velocity × 40 × s` units a second at one step per
/// 0.1 s, not scaled by the time since the last step.
pub fn step(dir: Vec3, s: &BulletStep) -> Vec3 {
    let k = speed_factor(s.slowmo_factor100, s.bullet_time);
    dir.map(|d| d * s.velocity / 0.25 * k)
}

/// The range index of a hit (0x1006a5a0..0x1006a64b): the quotient is stored to a float before
/// `ftol(x × 10)`; exit bullets use `dist / 300`, the others `dist / (unit / 0.1f)`. `dist` is
/// `Length`'s float.
pub fn range_index(dist: f64, unit: f32, exit: bool) -> i32 {
    let d = dist as f32 as f64;
    let x = if exit { (d / 300.0) as f32 } else { (d / ((unit as i32) as f64 / 0.1f32 as f64)) as f32 };
    ts_client::events::ftol(x as f64 * 10.0)
}

/// The bullet model for an ammo type (0x10069600): buckshot the pellet, 5.56, 7.62 and .50 BMG
/// the long bullet, the rest the bullet.
pub fn model_for(ammo: i32) -> &'static str {
    match ammo {
        3 => MODELS[2],
        4 | 6 | 8 => MODELS[1],
        _ => MODELS[0],
    }
}

/// The model's body (0x100967d0, from `HUD_CreateEntities`): a detail level by the distance from
/// the cull origin and `m_iFOV`; a scope (FOV below 80) keeps detail further out.
pub fn lod_body(dist: f32, fov: i32) -> i32 {
    if fov < 0x50 {
        if fov < 0xe && dist < 2048.0 {
            return 3;
        }
        if fov < 0x29 && dist < 512.0 {
            return 2;
        }
    }
    if dist <= 256.0 { if dist <= 128.0 { if dist <= 64.0 { 3 } else { 2 } } else { 1 } } else { 0 }
}

/// `slowdiv` of the raw factor times `x` (0x100962e0).
fn slow_time(slowmo_factor100: f32, x: f32) -> f64 {
    crate::slowdiv(slowmo_factor100 / 100.0) * x as f64
}

/// The trail spin's step (0x100699a4..0x10069b3b), in degrees: `max(m × 90, 22.5)` with `m` the
/// raw factor, at least 0.1 (1 at normal speed).
pub fn spin_step(slowmo_factor100: f32) -> f32 {
    let f = slowmo_factor100 / 100.0;
    let m = if f == 0.0 {
        1.0
    } else if f as f64 <= 0.1 {
        0.1
    } else {
        f as f64
    };
    (if m * 90.0 <= 22.5 { 22.5 } else { m * 90.0 }) as f32
}

/// The trails a non-buckshot bullet leaves (0x10069b5c..): by `cl_wavedetail`, the spiral needs
/// bullet time and more than 2, the wave bullet time and more than 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trails {
    None,
    Trail,
    TrailAndWave,
    Spiral,
}

pub fn trails(wavedetail: f32, bullet_time: bool) -> Trails {
    if !gt(wavedetail, 0.0) {
        Trails::None
    } else if !(gt(wavedetail, 1.0) && bullet_time) {
        Trails::Trail
    } else if gt(wavedetail, 2.0) {
        Trails::Spiral
    } else {
        Trails::TrailAndWave
    }
}

/// `bullet_trail.spr` (0x1007d8b0): a corner quad from `p2` (`b` either side along the view's up)
/// to `p1` (`a` either side), colour 0.5, fade 0.02.
pub fn trail_particle(sprite: i16, p1: Vec3, p2: Vec3, a: f32, b: f32, up: Vec3) -> psys::Particle {
    let (ua, ub) = (v::scale(up, a), v::scale(up, b));
    let mut p = particle(sprite, p1, a, 0.5, 0.02);
    p.corners = Some([v::sub(p2, ub), v::add(p2, ub), v::add(p1, ua), v::sub(p1, ua)]);
    p
}

/// 2.1's particle templates are zeroed statics the builders fill: life 0 and nothing else set.
fn particle(sprite: i16, origin: Vec3, radius: f32, color: f32, fadespeed: f32) -> psys::Particle {
    let mut p = psys::Particle::default();
    p.sprite = sprite;
    p.origin = origin;
    p.radius = radius;
    p.color = color;
    p.fadespeed = fadespeed;
    p.life = 0.0;
    p
}

/// `bullet_wave.spr` (0x1007dc70): a billboard of radius `w` at `p`, colour 0.6, fade 0.035. TS
/// passes the direction too, but doesn't use it.
pub fn wave_particle(sprite: i16, p: Vec3, w: f32) -> psys::Particle {
    particle(sprite, p, w, 0.6, 0.035)
}

/// `bullet_trail3.spr` (0x1007de30): 3 segments (5 above `cl_wavedetail` 3) round the bullet's
/// axis (`AngleVectors(angles)`'s right and up), each a corner quad from `start` (radius `a`) to
/// `end` (radius `b`) spanning `360/n` degrees, with the sprite's u split between them; colour
/// 0.7, fade 0.035.
pub fn spiral_particles(sprite: i16, start: Vec3, end: Vec3, angles: Vec3, a: f32, b: f32, wavedetail: f32) -> Vec<psys::Particle> {
    let (_, right, up) = v::angle_vectors(angles);
    let n: f32 = if wavedetail <= 3.0 { 3.0 } else { 5.0 };
    let c = ((360.0f32 / n) as f64 * std::f64::consts::PI / 180.0) as f32;
    let inv = 1.0f32 / n;
    let at = |base: Vec3, r: f32, t: f32| {
        let side = v::scale(v::scale(right, (t as f64).cos() as f32), r);
        let lift = v::scale(v::scale(up, (t as f64).sin() as f32), r);
        v::add(v::add(base, side), lift)
    };
    let mut out = Vec::new();
    let mut t = 0.0f32;
    let mut i = 0;
    while (i as f32) < n {
        let u0 = (i as f64 * inv as f64) as f32;
        let u1 = (i as f64 * inv as f64 + inv as f64) as f32;
        let mut p = particle(sprite, start, a, 0.7, 0.035);
        p.corners = Some([at(start, a, t), at(start, a, t + c), at(end, b, t + c), at(end, b, t)]);
        p.uv = Some([[u0, 0.0], [u1, 0.0], [u1, 1.0], [u0, 1.0]]);
        out.push(p);
        t += c;
        i += 1;
    }
    out
}

/// The drawn position's move this frame (0x10069808..0x100698b6): `dir × velocity / 0.25 × s ×
/// frametime × 10`, each vector operation rounded to floats.
pub fn draw_step(dir: Vec3, velocity: f32, s: f32, frametime: f32) -> Vec3 {
    dir.map(|d| d * velocity / 0.25 * s * frametime * 10.0)
}

/// `VectorAngles` (0x1007d6c0) of the direction with the pitch negated back: (pitch, yaw, 0).
fn dir_angles(dir: Vec3) -> Vec3 {
    let mut a = fx::vec_to_angles(dir);
    a[0] = -a[0];
    a
}

/// `ClientBullets` draw (0x10069540), from the frame while a `TSSlowMo` factor is set. Outside
/// the view cone nothing happens. With bullet time on or the factor at most 0.1 (and the three
/// models loaded) it queues the bullet's model, at most 31 a frame. Then the drawn position moves
/// on and, when its trail is due, the trails go from the last trail point to it.
pub fn draw(b: &mut ClientBullet, d: &mut BulletDraw) {
    let f = d.frame;
    if !psys::in_view_cone(&f.cam, f.cull, b.draw_pos) {
        return;
    }
    let factor = d.slowmo_factor100 / 100.0;
    if (d.bullet_time || factor as f64 <= 0.1) && d.models.queued().len() < MAX_MODELS && d.models.loaded(&MODELS) {
        let body = lod_body(v::dist(b.draw_pos, f.cull), d.fov as i32);
        d.models.queue(BulletModel { name: model_for(b.ammo), origin: b.draw_pos, angles: dir_angles(b.dir), body });
    }
    let s = speed_factor(d.slowmo_factor100, d.bullet_time);
    b.draw_pos = v::add(b.draw_pos, draw_step(b.dir, d.velocity, s, f.frametime));
    if d.now > b.next_trail {
        let up = f.cam.up;
        let t = effects::now();
        if b.ammo == 3 {
            // Buckshot: no spin, and the trail time isn't set, so a segment every frame.
            if gt(d.wavedetail, 0.0) {
                let (p1, p2) = (b.draw_pos, b.trail);
                effects::with_psys(|ps| {
                    if let Some(spr) = ps.sprite("sprites/bullet_trail.spr", ADD) {
                        ps.add(trail_particle(spr, p1, p2, 0.5, 0.5, up), t);
                    }
                });
            }
        } else {
            let x = if gt(d.wavedetail, 1.0) { 0.003 } else { 0.01 };
            b.next_trail = (slow_time(d.slowmo_factor100, x) + d.now as f64) as f32;
            let w_old = ((b.spin as f64 * DEG * 1.0).sin() + 1.5) as f32;
            b.spin += spin_step(d.slowmo_factor100);
            if b.spin > 360.0 {
                b.spin -= 360.0;
            }
            let w_new = ((b.spin as f64 * DEG * 1.0).sin() + 1.5) as f32;
            let (p1, p2, dir, wd) = (b.draw_pos, b.trail, b.dir, d.wavedetail);
            effects::with_psys(|ps| match trails(wd, d.bullet_time) {
                Trails::None => {}
                Trails::Trail | Trails::TrailAndWave => {
                    if let Some(spr) = ps.sprite("sprites/bullet_trail.spr", ADD) {
                        ps.add(trail_particle(spr, p1, p2, 0.5, 0.5, up), t);
                    }
                    if trails(wd, d.bullet_time) == Trails::TrailAndWave
                        && let Some(spr) = ps.sprite("sprites/bullet_wave.spr", ADD)
                    {
                        ps.add(wave_particle(spr, p1, w_new), t);
                    }
                }
                Trails::Spiral => {
                    if let Some(spr) = ps.sprite("sprites/bullet_trail3.spr", ADD) {
                        for p in spiral_particles(spr, p2, p1, dir_angles(dir), w_old, w_new, wd) {
                            ps.add(p, t);
                        }
                    }
                }
            });
        }
        b.trail = b.draw_pos;
    }
}
