//! Weapon and impact effects built on the sprite particle system (rendering doc §4.4-4.5,
//! `TS_MuzzleFlash`, `TS_GunSmoke`, `TS_Sparks`, `TS_ImpactParticles`, ...).
//!
//! The sprites, counts and trigger conditions follow the doc; the per-particle numbers (sizes,
//! speeds, lifetimes) are approximations of the un-annotated assembly, tuned to look like TS.

use super::psys::{self, PSys, Particle};
use goldsrc::ext::render::{self as r, Vec3, vec3 as v};
use goldsrc::sys::*;

fn rf(a: f32, b: f32) -> f32 {
    r::random_float(a, b)
}

fn rand_dir() -> Vec3 {
    v::normalize([rf(-1.0, 1.0), rf(-1.0, 1.0), rf(-1.0, 1.0)])
}

/// `TS_MuzzleFlash` (`0x100474b0`): `count` sprites of `ts_muzzleflash1h.spr` along `fwd`, each
/// x0.909 smaller, plus a `ts_muzzleflash6.spr` core. `fade` (0..1) dims distant flashes.
pub fn muzzle_flash(ps: &mut PSys, now: f64, org: Vec3, fwd: Vec3, count: i32, size: f32, fade: f32) {
    if fade <= 0.0 {
        return;
    }
    let (Some(s1), Some(s6)) = (ps.sprite("sprites/ts_muzzleflash1h.spr"), ps.sprite("sprites/ts_muzzleflash6.spr")) else { return };
    let mut radius = size * 0.25;
    let mut ofs = 0.0;
    let roll = rf(0.0, 360.0);
    for _ in 0..count.clamp(0, 24) {
        ps.add(
            Particle {
                sprite: s1,
                origin: v::ma(org, ofs, fwd),
                radius,
                life: 0.02,
                fadespeed: 0.6,
                color: fade,
                rendermode: kRenderTransAdd as i32,
                orient: psys::ORIENT_ROLL,
                angles: [0.0, 0.0, roll],
                ..Default::default()
            },
            now,
        );
        ofs += radius * 0.5;
        radius *= 0.909;
    }
    ps.add(
        Particle {
            sprite: s6,
            origin: org,
            radius: size * 0.35,
            life: 0.02,
            fadespeed: 0.8,
            color: fade,
            rendermode: kRenderTransAdd as i32,
            orient: psys::ORIENT_ROLL,
            angles: [0.0, 0.0, rf(0.0, 360.0)],
            ..Default::default()
        },
        now,
    );
}

/// `TS_MuzzleDlight` (`0x10048e60`): radius 192, colour (240, 221, 172), life 0.06/ts, at most
/// one every 0.1/ts seconds.
pub fn muzzle_dlight(ps: &mut PSys, now: f64, org: Vec3, ts: f32) {
    if !ps.cooldown_ready(0, now, 0.1 / ts.max(0.01)) {
        return;
    }
    if let Some(dl) = r::alloc_dlight(0) {
        dl.origin = org;
        dl.radius = 192.0;
        dl.color.r = 240;
        dl.color.g = 221;
        dl.color.b = 172;
        dl.die = r::client_time() + 0.06 / ts.max(0.01);
        dl.decay = 0.0;
    }
}

/// `TS_GunSmoke` (`0x10047130`): `gun_smoke_add.spr` puffs drifting out of the barrel.
/// TS passes an intensity of 45-55 (5-10 for the sawed-off); one puff per 10.
pub fn gun_smoke(ps: &mut PSys, now: f64, org: Vec3, fwd: Vec3, intensity: i32) {
    let Some(s) = ps.sprite("sprites/gun_smoke_add.spr") else { return };
    let n = (intensity / 10).max(1);
    for _ in 0..n {
        ps.add(
            Particle {
                sprite: s,
                origin: v::ma(org, rf(0.0, 4.0), fwd),
                vel: v::add(v::scale(fwd, rf(10.0, 40.0)), [rf(-4.0, 4.0), rf(-4.0, 4.0), rf(4.0, 12.0)]),
                radius: rf(1.5, 3.0),
                scalespeed: 0.06,
                life: 0.1,
                color: 0.25,
                fadespeed: 0.004,
                drag: 2.0,
                rendermode: kRenderTransAdd as i32,
                ..Default::default()
            },
            now,
        );
    }
}

/// Shotgun pellet spray (`0x10046bd0` with `shotgun_pellets.spr`).
pub fn pellet_spray(ps: &mut PSys, now: f64, org: Vec3, fwd: Vec3, pellets: i32) {
    let Some(s) = ps.sprite("sprites/shotgun_pellets.spr") else { return };
    for _ in 0..pellets.clamp(1, 16) {
        let d = v::normalize(v::add(fwd, v::scale(rand_dir(), 0.08)));
        ps.add(
            Particle {
                sprite: s,
                origin: org,
                vel: v::scale(d, rf(900.0, 1400.0)),
                radius: 0.6,
                stretch: 0.02,
                life: 0.08,
                fadespeed: 0.2,
                rendermode: kRenderTransAdd as i32,
                ..Default::default()
            },
            now,
        );
    }
}

/// `TS_Sparks` (`0x100492b0`): `sprites/sparks.spr`, count in `[n/2, n]`, stretched by speed.
pub fn sparks(ps: &mut PSys, now: f64, pos: Vec3, dir: Vec3, n: i32, scale: f32, speed: f32) {
    let Some(s) = ps.sprite("sprites/sparks.spr") else { return };
    let count = r::random_long(n / 2, n.max(1));
    for _ in 0..count {
        let d = v::normalize(v::add(dir, v::scale(rand_dir(), 0.7)));
        ps.add(
            Particle {
                sprite: s,
                origin: pos,
                vel: v::scale(d, speed * rf(0.4, 1.0)),
                gravity: 0.5,
                radius: 0.4 * scale,
                stretch: 0.015,
                life: rf(0.1, 0.35),
                fadespeed: 0.03,
                rendermode: kRenderTransAdd as i32,
                ..Default::default()
            },
            now,
        );
    }
}

/// `TS_Scintille` (`0x10046f50`): a short bright flash at metal impacts.
pub fn scintille(ps: &mut PSys, now: f64, pos: Vec3, size: f32) {
    let Some(s) = ps.sprite("sprites/scintille_metal.spr") else { return };
    let frames = ps.frames(s);
    ps.add(
        Particle {
            sprite: s,
            origin: pos,
            radius: size,
            life: 0.0,
            framerate: frames as f32 * 20.0,
            fadespeed: 0.05,
            rendermode: kRenderTransAdd as i32,
            orient: psys::ORIENT_ROLL,
            angles: [0.0, 0.0, rf(0.0, 360.0)],
            ..Default::default()
        },
        now,
    );
}

/// `TS_ImpactSmoke` / `TS_SmallPuff`: a dust puff off a surface.
pub fn impact_smoke(ps: &mut PSys, now: f64, pos: Vec3, normal: Vec3, sprite: &str, color: f32, n: i32) {
    let Some(s) = ps.sprite(sprite) else { return };
    for _ in 0..n {
        ps.add(
            Particle {
                sprite: s,
                origin: v::ma(pos, 2.0, normal),
                vel: v::add(v::scale(normal, rf(20.0, 60.0)), v::scale(rand_dir(), 10.0)),
                radius: rf(2.0, 4.0),
                scalespeed: 0.08,
                life: 0.1,
                color,
                fadespeed: 0.006,
                drag: 3.0,
                gravity: -0.01,
                rendermode: kRenderTransAdd as i32,
                orient: psys::ORIENT_ROLL,
                angles: [0.0, 0.0, rf(0.0, 360.0)],
                avel: [0.0, 0.0, rf(-60.0, 60.0)],
                ..Default::default()
            },
            now,
        );
    }
}

/// `TS_SpriteDebris`: small chunks thrown off a surface (wood, concrete, glass, blood).
#[allow(clippy::too_many_arguments)]
pub fn debris(ps: &mut PSys, now: f64, pos: Vec3, normal: Vec3, sprite: &str, n: i32, size: f32, speed: f32, rendermode: i32) {
    let Some(s) = ps.sprite(sprite) else { return };
    let frames = ps.frames(s);
    for _ in 0..n {
        let d = v::normalize(v::add(normal, v::scale(rand_dir(), 0.8)));
        ps.add(
            Particle {
                sprite: s,
                origin: v::ma(pos, 1.0, normal),
                vel: v::scale(d, speed * rf(0.5, 1.0)),
                gravity: 1.0,
                radius: size * rf(0.6, 1.0),
                frame: r::random_long(0, frames - 1) as f32,
                life: rf(0.4, 0.8),
                fadespeed: 0.02,
                rendermode,
                orient: psys::ORIENT_FREE,
                angles: [rf(0.0, 360.0), rf(0.0, 360.0), 0.0],
                avel: [rf(-400.0, 400.0), rf(-400.0, 400.0), 0.0],
                collide: true,
                ..Default::default()
            },
            now,
        );
    }
}

/// `TS_ImpactParticles` (`0x10048f60`): the material decides the effect. Rate-limited per
/// material group and culled by distance from the view.
pub fn impact(ps: &mut PSys, now: f64, cam_origin: Vec3, pos: Vec3, normal: Vec3, tex: u8, pellets: bool) {
    if v::dist(pos, cam_origin) > 2048.0 {
        return;
    }
    let group = match tex {
        b'D' | b'W' => 1,
        b'M' | b'G' | b'V' => 2,
        b'P' | b'Y' => 3,
        b'F' | b'S' => return,
        _ => 4,
    };
    if !ps.cooldown_ready(8 + group, now, if pellets { 0.05 } else { 0.0 }) {
        return;
    }
    let solid = kRenderTransAlpha as i32;
    match tex {
        b'D' => impact_smoke(ps, now, pos, normal, "sprites/wood_smoke_add.spr", 0.35, 2),
        b'W' => {
            debris(ps, now, pos, normal, "sprites/woodfrag.spr", 4, 1.2, 150.0, solid);
            impact_smoke(ps, now, pos, normal, "sprites/wood_smoke_add.spr", 0.3, 1);
        }
        b'M' | b'G' | b'V' => {
            scintille(ps, now, pos, 4.0);
            sparks(ps, now, pos, normal, 8, 1.0, 250.0);
            impact_smoke(ps, now, pos, normal, "sprites/gun_smoke_add.spr", 0.2, 1);
        }
        b'P' | b'Y' => {
            scintille(ps, now, pos, 3.0);
            sparks(ps, now, pos, normal, 6, 0.8, 200.0);
        }
        _ => {
            debris(ps, now, pos, normal, "sprites/debris_concrete3.spr", 4, 1.0, 180.0, solid);
            impact_smoke(ps, now, pos, normal, "sprites/gun_smoke_add.spr", 0.3, 2);
            if r::random_long(0, 3) == 0 {
                sparks(ps, now, pos, normal, 4, 0.8, 200.0);
            }
        }
    }
}

/// `TS_BloodImpact` (`0x10048840`) for `PTakeDam`: `bloodfrag2.spr` spray (scaled by `cl_gore`)
/// plus a `{bloodN` decal on the wall behind; kevlar hits add a smoke puff.
pub fn blood(ps: &mut PSys, now: f64, pos: Vec3, dir: Vec3, amount: i32, kevlar: bool, gore: f32) {
    if kevlar {
        impact_smoke(ps, now, pos, v::scale(dir, -1.0), "sprites/gun_smoke_add.spr", 0.3, 2);
    }
    if gore <= 0.0 {
        return;
    }
    let n = ((amount as f32 / 4.0).clamp(2.0, 12.0) * gore.min(2.0)) as i32;
    debris(ps, now, pos, v::normalize(v::add(dir, [0.0, 0.0, 0.3])), "sprites/bloodfrag2.spr", n, 1.6, 120.0, kRenderTransAlpha as i32);
    // Decal on whatever is behind the victim (TS sprays decals from the particle collision
    // callback; one trace here is a close stand-in).
    let end = v::ma(pos, 96.0, v::normalize(dir));
    let tr = r::player_trace(pos, end, 2, (PM_STUDIO_IGNORE | PM_WORLD_ONLY) as i32, -1);
    if tr.fraction < 1.0 {
        let name = format!("{{blood{}", r::random_long(1, 8));
        let idx = r::decal_index(&name);
        if idx != 0 {
            r::decal_shoot(idx, 0, 0, tr.endpos, 0);
        }
        if r::random_long(0, 2) == 0 {
            r::play_sound(0, tr.endpos, CHAN_STATIC as i32, "player/splat.wav", 0.5, r::ATTN_NORM, 0, crate::events::slow_pitch());
        }
    }
}

/// `TS_GlassShatter` (`0x10047cb0`, `TSFrags` type 0): shards fill the box, velocity dir*force.
pub fn glass(ps: &mut PSys, now: f64, origin: Vec3, size: Vec3, dir: Vec3, force: f32) {
    let Some(s) = ps.sprite("sprites/glassfrag.spr") else { return };
    let frames = ps.frames(s);
    let vol = (size[0].abs() * size[1].abs() * size[2].abs()).max(1.0);
    let n = (vol / 2000.0).clamp(8.0, 64.0) as i32;
    for _ in 0..n {
        let p = [0, 1, 2].map(|i| origin[i] + rf(-0.5, 0.5) * size[i]);
        ps.add(
            Particle {
                sprite: s,
                origin: p,
                vel: v::add(v::scale(dir, force * rf(0.3, 1.0)), v::scale(rand_dir(), 60.0)),
                gravity: 1.0,
                radius: rf(1.0, 3.0),
                frame: r::random_long(0, frames - 1) as f32,
                life: rf(1.0, 2.0),
                fadespeed: 0.01,
                rendermode: kRenderTransAdd as i32,
                orient: psys::ORIENT_FREE,
                angles: [rf(0.0, 360.0), rf(0.0, 360.0), 0.0],
                avel: [rf(-300.0, 300.0), rf(-300.0, 300.0), 0.0],
                collide: true,
                ..Default::default()
            },
            now,
        );
    }
}

/// `TS_Explosion` (`0x100481b0`, `TSFrags` type 1).
pub fn explosion(ps: &mut PSys, now: f64, origin: Vec3, radius: f32, ts: f32) {
    let snd = format!("explo/explode{}.wav", r::random_long(1, 3));
    r::play_sound(0, origin, CHAN_STATIC as i32, &snd, 1.0, 0.3, 0, crate::events::fire_pitch());
    ps.push_radial(origin, radius.max(256.0), 400.0);
    if let Some(s) = ps.sprite("sprites/grenade_asplode.spr") {
        let frames = ps.frames(s);
        ps.add(
            Particle {
                sprite: s,
                origin: v::add(origin, [0.0, 0.0, 24.0]),
                radius: 40.0,
                life: 0.0,
                framerate: frames as f32 * 2.0,
                fadespeed: 0.04,
                rendermode: kRenderTransAdd as i32,
                ..Default::default()
            },
            now,
        );
    }
    if let Some(s) = ps.sprite("sprites/gun_smoke_add.spr") {
        for _ in 0..8 {
            ps.add(
                Particle {
                    sprite: s,
                    origin: v::add(origin, [rf(-24.0, 24.0), rf(-24.0, 24.0), rf(0.0, 40.0)]),
                    vel: v::scale(rand_dir(), rf(20.0, 80.0)),
                    radius: rf(12.0, 24.0),
                    scalespeed: 0.15,
                    life: rf(0.3, 0.8),
                    color: 0.25,
                    fadespeed: 0.004,
                    drag: 1.0,
                    gravity: -0.02,
                    rendermode: kRenderTransAdd as i32,
                    orient: psys::ORIENT_ROLL,
                    angles: [0.0, 0.0, rf(0.0, 360.0)],
                    ..Default::default()
                },
                now,
            );
        }
    }
    sparks(ps, now, origin, [0.0, 0.0, 1.0], 30, 2.0, 600.0);
    if let Some(dl) = r::alloc_dlight(0) {
        dl.origin = origin;
        dl.radius = radius.clamp(200.0, 400.0);
        dl.color.r = 255;
        dl.color.g = 180;
        dl.color.b = 100;
        dl.die = r::client_time() + 0.3 / ts.max(0.01);
        dl.decay = 600.0;
    }
}

/// `TS_BulletTrail` (`0x10046240`): the slow-motion bullet's streak.
pub fn bullet_trail(ps: &mut PSys, now: f64, from: Vec3, to: Vec3) {
    let Some(s) = ps.sprite("sprites/bullet_trail.spr") else { return };
    let d = v::sub(to, from);
    let len = v::len(d);
    if len < 1.0 {
        return;
    }
    ps.add(
        Particle {
            sprite: s,
            origin: v::ma(from, 0.5, d),
            vel: v::scale(d, 1.0 / len),
            stretch: len,
            radius: 0.8,
            life: 0.0,
            color: 0.6,
            fadespeed: 0.02,
            rendermode: kRenderTransAdd as i32,
            ..Default::default()
        },
        now,
    );
}
