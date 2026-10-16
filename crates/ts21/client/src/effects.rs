//! TS 2.1's own effects outside the weapon events: the lasers' look (issue #204), the `TSFrags`
//! handler (`0x10088020`: glass `0x10081840`, explosion `0x100821e0`, sparks) and the `PTakeDam`
//! blood (`0x10087f60` → `0x10082cd0`) (issue #232).
//!
//! The particle values are those 2.1 writes into each builder's scratch particle; the slots are
//! named from 2.1's sprite-debris builder (`0x1007ef20`), whose parameters are known from its
//! calls: `+0x01` collide (3), `+0x10` origin, `+0x28` the roll orientation, `+0x2c` velocity,
//! `+0x38` angular velocity, `+0x44` gravity, `+0x48` life, `+0x50` colour, `+0x54` fade, `+0x58`
//! scale speed, `+0x5c` frame, `+0x60` frame rate, `+0xb4` radius. They run on the port's (TS
//! 3.0's) particle system, as the other 2.1 builders do; velocities are TS units per 0.1 s (×10
//! here, as `ts_client::effects::fx` does).

use goldsrc::client::engine;
use goldsrc::ext::render::{Vec3, random_float, random_long, vec3 as v};
use goldsrc::sys::{CHAN_STATIC, kRenderTransAdd, kRenderTransColor};
use ts_client::effects::LaserStyle;
use ts_client::effects::fx::{self, random_frame};
use ts_client::effects::psys::{self, PSys, Particle};
use ts_client::effects::{PTakeDam, TsFrags};

const ADD: i32 = kRenderTransAdd as i32;
const COLOR: i32 = kRenderTransColor as i32;

/// TS velocity units (per 0.1 s) to units per second.
const TS: f32 = 10.0;

/// A particle from the default (TS's zeroed scratch particle) with `f`'s fields.
fn particle(f: impl FnOnce(&mut Particle)) -> Particle {
    let mut p = Particle::default();
    f(&mut p);
    p
}

/// 2.1's lasers, laser dot, flashlights and shadows:
///
/// * the local beam (`0x10071bd0`): `laserbeam.spr` frame 0, colour `(j + 1) / 8 + 0.75` from
///   the frame's first jitter roll, half-width 1.5 (`0x1007284d..`);
/// * the HUD dot (`0x1008b410`): red, only while the hit is less than 4096 units away
///   (`fcomp 4096.0` at `0x1008b4c7`), the first gun's alpha `ftol((4096 − d) / 4096 × 255)`
///   (`0x1008b520..0x1008b53a`), the second's 255;
/// * other players' lasers (`0x100731a0`) and flashlights (`0x10071630`) from `origin + 24` with
///   the angles' pitch × −3; the lasers a white `laserdot.spr` (radius 4) and no beam;
/// * shadows (`0x10095260`, `0x10094e70`): `RenderMode(2)`, `Color4f(1, 1, 1, 1)` once, no
///   distance fade or cut-off, traces with flags 2.
pub static LASER: LaserStyle =
    LaserStyle { animated_beam: false, dot_range: Some(4096.0), others_from_origin: true, other_beams: false, shadow_fade: false };

/// `TSFrags` (`0x10088020`): type 0 [`glass`] from the centre and size alone, type 1
/// [`explosion`] with the radius `ftol`'d, type 2 a `scintille_metal.spr` burst (9) and a
/// `scintille2.spr` sprite-debris burst (7, radius 2, spread 16, life 0.5, gravity 5, colliding),
/// both with a zero direction (`0x100880f6..0x10088158`).
pub fn tsfrags(ps: &mut PSys, now: f64, f: &TsFrags) {
    match *f {
        TsFrags::Glass { origin, size, .. } => glass(ps, now, origin, size),
        TsFrags::Explosion { origin, radius } => explosion(ps, now, origin, radius as i32),
        TsFrags::Sparks { origin, .. } => {
            fx::scintille_metal(ps, now, origin, [0.0; 3], 9.0);
            fx::sprite_debris(ps, now, origin, [0.0; 3], "sprites/scintille2.spr", ADD, 7, 2.0, 16, 0.5, 5.0, false, true);
        }
    }
}

/// The explosion sound's pitch (`0x10082240..0x10082274`): the ricochet's code on the raw
/// `TSSlowMo` factor ([`ricochet_pitch`](crate::events::ricochet_pitch)), with no floor.
pub fn explosion_pitch(slowmo_factor100: f32) -> i32 {
    crate::events::ricochet_pitch(slowmo_factor100)
}

/// The explosion (`0x100821e0`). `explo/explode{,1..4}.wav` (`RandomLong(0, 4)`) on
/// `CHAN_STATIC` at [`explosion_pitch`], volume `RandomFloat(0.92, 1)`, attenuation 0.2. Then
/// `24 × quality` `gun_smoke_add.spr` puffs (radius `0.7 r`, raised by half that, velocity
/// `(±54, ±54, 0..32)`, gravity 2, colour 0.7, scale speed 0.1, frame rate `RandomLong(2, 10)`),
/// one `scintille_metal.spr` flash (radius `2 r`, raised by `r`, colour 1, fade 0.01, scale
/// speed 1) and `64 × quality` `debris_concrete2.spr` chips (radius 8, gravity 13, colour 0.7,
/// fade 0.05, rolling at `RandomLong(−180, 180)`, life 2 + that / 1000, velocity `(±64, ±64, 0..64)`,
/// colliding). A sprite that fails to load ends the effect. No fireball, second smoke cloud,
/// particle push or light (3.0's `0x100481b0` has them).
pub fn explosion(ps: &mut PSys, now: f64, org: Vec3, r: i32) {
    const SOUNDS: [&str; 5] = ["explo/explode.wav", "explo/explode1.wav", "explo/explode2.wav", "explo/explode3.wav", "explo/explode4.wav"];
    let snd = SOUNDS[random_long(0, 4).clamp(0, 4) as usize];
    let pitch = explosion_pitch(ts_client::state::with(|s| s.slowmo_factor100));
    engine().ev_play_sound(0, org, CHAN_STATIC as i32, snd, random_float(0.92, 1.0), 0.2, 0, pitch);
    let scale = ps.tuning.scale;
    let Some(s) = ps.sprite("sprites/gun_smoke_add.spr", ADD) else { return };
    let size = r as f32 * 0.7;
    let mut i = 0;
    while (i as f32) < scale * 24.0 {
        let vel = [random_long(-54, 54) as f32, random_long(-54, 54) as f32, random_long(0, 32) as f32];
        let framerate = random_long(2, 10) as f32;
        ps.add(
            particle(|p| {
                p.sprite = s;
                p.origin = v::add(org, [0.0, 0.0, size / 2.0]);
                p.vel = v::scale(vel, TS);
                p.radius = size;
                p.gravity = 2.0;
                p.color = 0.7;
                p.scalespeed = 0.1;
                p.framerate = framerate;
                p.life = 0.0;
            }),
            now,
        );
        i += 1;
    }
    let Some(s) = ps.sprite("sprites/scintille_metal.spr", ADD) else { return };
    let size = r.wrapping_shl(1) as f32;
    let frame = random_frame(ps, s);
    ps.add(
        particle(|p| {
            p.sprite = s;
            p.origin = v::add(org, [0.0, 0.0, size / 2.0]);
            p.radius = size;
            p.color = 1.0;
            p.fadespeed = 0.01;
            p.scalespeed = 1.0;
            p.frame = frame;
            p.life = 0.0;
        }),
        now,
    );
    let Some(s) = ps.sprite("sprites/debris_concrete2.spr", ADD) else { return };
    let mut i = 0;
    while (i as f32) < scale * 64.0 {
        let spin = random_long(-180, 180) as f32;
        let life = (spin as f64 / 1000.0 + 2.0) as f32;
        let frame = random_frame(ps, s);
        let vel = [random_long(-64, 64) as f32, random_long(-64, 64) as f32, random_long(0, 64) as f32];
        ps.add(
            particle(|p| {
                p.sprite = s;
                p.origin = org;
                p.vel = v::scale(vel, TS);
                p.avel = [0.0, 0.0, spin * TS];
                p.radius = 8.0;
                p.gravity = 13.0;
                p.color = 0.7;
                p.fadespeed = 0.05;
                p.life = life;
                p.frame = frame;
                p.orient = psys::ORIENT_ROLL;
                p.collide = true;
            }),
            now,
        );
        i += 1;
    }
}

/// The box's axes with the thin one zeroed (`0x10081840`'s copy of the size, 3.0's tie rules).
fn glass_size(size: Vec3) -> Vec3 {
    let mut s = size;
    if s[0] <= s[1] {
        if s[0] <= s[2] {
            s[0] = 0.0;
        } else {
            s[2] = 0.0;
        }
    } else if s[1] <= s[2] {
        s[1] = 0.0;
    } else {
        s[2] = 0.0;
    }
    s
}

/// The shard radius: `min((s1 + s2) / 2 / 9 × 0.7, 20)` of the two kept axes.
fn glass_radius(s: Vec3) -> f32 {
    let r = (s[0] as f64 + s[1] as f64 + s[2] as f64) / 2.0 / 9.0 * 0.7;
    if 20.0 <= r { 20.0 } else { r as f32 }
}

/// The two grid axes (outer, inner) from the zeroed size, as 2.1 tests it: y then z without x,
/// else x then (z without y, else y).
fn glass_axes(s: Vec3) -> (usize, usize) {
    if s[0] == 0.0 {
        (1, 2)
    } else if s[1] == 0.0 {
        (0, 2)
    } else {
        (0, 1)
    }
}

/// A grid coordinate: `size / 9 × k + (org − size / 2)` on the x87, stored as a float.
fn glass_coord(org: f32, size: f32, k: i32) -> f32 {
    (size as f64 / 9.0 * k as f64 + (org as f64 - size as f64 / 2.0)) as f32
}

/// The glass (`0x10081840`, `TSFrags` type 0, from the centre `org` and the box `size` only):
/// 81 `glassfrag.spr` shards on a 9×9 grid across the two large axes ([`glass_size`],
/// [`glass_axes`]), radius [`glass_radius`], each thrown `(RandomFloat(−9, 9), RandomFloat(−9, 9),
/// RandomLong(0, 10))` and spinning at 10 × its x speed, gravity 6, colour 0.7, fade 0.05, life
/// `(RandomLong(30, 50) − 25) × 0.1`, rolling and colliding. 3.0's (`0x10047cb0`) throws them
/// from the hit point by the damage and force, with gravity 1, a 2..5 s life, a random spin and a
/// react value.
pub fn glass(ps: &mut PSys, now: f64, org: Vec3, size: Vec3) {
    let Some(s) = ps.sprite("sprites/glassfrag.spr", ADD) else { return };
    let thin = glass_size(size);
    let radius = glass_radius(thin);
    let (a1, a2) = glass_axes(thin);
    let mut at = org;
    for a in 0..3 {
        if thin[a] != 0.0 {
            at[a] = (org[a] as f64 - size[a] as f64 / 2.0) as f32;
        }
    }
    for k1 in 0..9 {
        at[a1] = glass_coord(org[a1], size[a1], k1);
        for k2 in 0..9 {
            at[a2] = glass_coord(org[a2], size[a2], k2);
            let frame = random_frame(ps, s);
            let vx = random_float(-9.0, 9.0);
            let vy = random_float(-9.0, 9.0);
            let vz = random_long(0, 10) as f32;
            let life = ((random_long(30, 50) - 25) as f64 * 0.1) as f32;
            ps.add(
                particle(|p| {
                    p.sprite = s;
                    p.origin = at;
                    p.vel = v::scale([vx, vy, vz], TS);
                    p.avel = [0.0, 0.0, vx * 10.0 * TS];
                    p.radius = radius;
                    p.gravity = 6.0;
                    p.color = 0.7;
                    p.fadespeed = 0.05;
                    p.life = life;
                    p.frame = frame;
                    p.orient = psys::ORIENT_ROLL;
                    p.collide = true;
                }),
                now,
            );
        }
    }
}

/// The 2.1 amount from the port's `PTakeDam` byte, which keeps 3.0's encoding in 2.1 mode (half
/// the damage, ×8 for pellets, +128 for kevlar; ts21.md Deviations): 2.1's server sends 0 for a
/// kevlar hit, else `min(dmg, 255)`. The half damage is doubled back (at least 1, so a hit that
/// isn't kevlar still bleeds); a pellet's ×8 can't be told apart.
pub fn blood_amount(d: &PTakeDam) -> i32 {
    if d.kevlar() { 0 } else { (d.strength() * 2).max(1) }
}

/// The drop count (`0x10082e69..0x10082f2a`): `ftol` of `amount × 0.2` clamped to 5..17, with
/// no `cl_gore` (3.0: 5..15, halved on kevlar, × the `cl_gore` factor).
pub fn blood_count(amount: i32) -> i32 {
    let x = amount as f64 * 0.2;
    let x = if x <= 5.0 { 5.0 } else { x };
    let x = if x >= 17.0 { 17.0 } else { x };
    x as i32
}

/// The blood (`0x10082cd0`, from `PTakeDam` `0x10087f60`), after the view-cone and visibility
/// tests. A kevlar hit (amount 0) makes only a `gun_smoke_add.spr` puff along `−dir × 6`
/// (`0x1007e550`, radius 8, frame rate 20) and no blood. Otherwise [`blood_count`]
/// `bloodfrag2.spr` (TransColor) drops, all from the hit point: velocity `(dir.x +
/// RandomFloat(−0.1, 0.1), dir.y + RandomFloat(−0.1, 0.1), dir.z) × RandomLong(16, 48) × k`, then
/// `z += RandomLong(0, 8)`, where `k` is −1 for the whole hit when `RandomLong(0, 100) > 70`
/// (rolled once), else 1; radius 1, gravity 3, colour 1, fade 0.01, scale speed 0.075, life
/// `RandomFloat(0.1, 1)`, colliding with a decal and no splat sound (`0x10082ba0`). 3.0's
/// (`0x10048840`) throws a falling chain back along `dir` and adds the puff and half the blood on
/// kevlar.
pub fn blood(ps: &mut PSys, now: f64, pos: Vec3, dir: Vec3, amount: i32) {
    if !ps.visible(pos) {
        return;
    }
    if amount == 0 {
        fx::small_puff(ps, now, pos, v::scale(dir, -6.0), "sprites/gun_smoke_add.spr", ADD, 8.0, 20.0);
        return;
    }
    let Some(s) = ps.sprite("sprites/bloodfrag2.spr", COLOR) else { return };
    let k = if random_long(0, 100) > 70 { -1.0 } else { 1.0 };
    for _ in 0..blood_count(amount) {
        let d = [dir[0] + random_float(-0.1, 0.1), dir[1] + random_float(-0.1, 0.1), dir[2]];
        let frame = random_frame(ps, s);
        let life = random_float(0.1, 1.0);
        let speed = random_long(16, 48) as f32;
        let mut vel = d.map(|c| c * speed * k);
        vel[2] += random_long(0, 8) as f32;
        ps.add(
            particle(|p| {
                p.sprite = s;
                p.origin = pos;
                p.vel = v::scale(vel, TS);
                p.radius = 1.0;
                p.gravity = 3.0;
                p.color = 1.0;
                p.fadespeed = 0.01;
                p.scalespeed = 0.075;
                p.life = life;
                p.frame = frame;
                p.collide = true;
                p.oncollide = psys::ONCOLLIDE_BLOOD_DECAL;
            }),
            now,
        );
    }
}

/// [`blood`] of a `PTakeDam` ([`blood_amount`]).
pub fn blood_impact(ps: &mut PSys, now: f64, d: &PTakeDam) {
    blood(ps, now, d.pos, d.dir, blood_amount(d));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 0x10082e69..0x10082f2a: `amount × 0.2` clamped to 5..17, no `cl_gore`.
    #[test]
    fn blood_count_is_5_to_17() {
        assert_eq!([1, 25, 26, 50, 85, 89, 255].map(blood_count), [5, 5, 5, 10, 17, 17, 17]);
        assert_eq!(blood_count(84), 16);
    }

    /// The port's 3.0-encoded byte back to 2.1's amount: kevlar is 0; the half damage doubled.
    #[test]
    fn blood_amount_from_3_0_encoding() {
        let d = |amount| PTakeDam { pos: [0.0; 3], dir: [0.0; 3], amount };
        assert_eq!([0x80 | 20, 20, 0, 127].map(|a| blood_amount(&d(a))), [0, 40, 1, 254]);
    }

    /// 0x10081840: the thin axis zeroed with 3.0's ties, the grid on the other two (y, z without
    /// x; else x then z without y, else y), radius `(s1 + s2) / 2 / 9 × 0.7` up to 20.
    #[test]
    fn glass_grid() {
        assert_eq!(glass_size([64.0, 2.0, 48.0]), [64.0, 0.0, 48.0]);
        assert_eq!(glass_size([2.0, 64.0, 48.0]), [0.0, 64.0, 48.0]);
        assert_eq!(glass_size([64.0, 48.0, 2.0]), [64.0, 48.0, 0.0]);
        assert_eq!(glass_size([8.0, 8.0, 8.0]), [0.0, 8.0, 8.0], "ties drop x");
        assert_eq!(glass_axes([0.0, 64.0, 48.0]), (1, 2));
        assert_eq!(glass_axes([64.0, 0.0, 48.0]), (0, 2));
        assert_eq!(glass_axes([64.0, 48.0, 0.0]), (0, 1));
        assert_eq!(glass_radius([0.0, 90.0, 90.0]), 7.0);
        assert_eq!(glass_radius([0.0, 500.0, 200.0]), 20.0);
        assert_eq!([0, 4, 8].map(|k| glass_coord(100.0, 90.0, k)), [55.0, 95.0, 135.0]);
    }

    /// 0x10082240..0x10082274: `ftol(100 + (float)(s × 300))` on the raw factor, 400 at normal
    /// speed, no 150 floor (205 at a 0.35 factor, 145 at 0.15).
    #[test]
    fn explosion_pitch_on_the_raw_factor() {
        assert_eq!([0.0, 35.0, 15.0, 29.0].map(explosion_pitch), [400, 205, 145, 187]);
    }
}
