//! Weapon and impact effects built on the sprite particle system (rendering doc §4.4-4.5).
//!
//! Every number is read from the TS client's disassembly (the decompile hides the float
//! arguments). TS keeps velocities in units per 0.1 s and angular velocities in degrees per
//! 0.1 s; they are converted to per second here (×10, the `TS` constant). Fields TS leaves at zero
//! in its scratch particle (`0x1004ffe0`) are left at zero, and `PSys::add` applies TS's drag
//! floor of 0.1.

use super::psys::{self, PSys, Particle};
use goldsrc::client::engine;
use goldsrc::ext::render::{self as r, Vec3, vec3 as v};
use goldsrc::sys::*;

/// TS velocity units (per 0.1 s) to units per second.
const TS: f32 = 10.0;

const ADD: i32 = kRenderTransAdd as i32;

fn rf(a: f32, b: f32) -> f32 {
    r::random_float(a, b)
}

fn rl(a: i32, b: i32) -> i32 {
    r::random_long(a, b)
}

/// `0x100509a0`: a random frame of `sprite`.
fn random_frame(ps: &PSys, sprite: i16) -> f32 {
    if sprite < 0 {
        return 0.0;
    }
    rl(0, ps.frames(sprite) - 1) as f32
}

/// The particle builders' parameters a ruleset can change
/// ([`ClientRules::fx_style`](crate::gameplay::ClientRules::fx_style)), each ruleset's own.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FxStyle {
    /// The first muzzle-flash sprite's colour (each later one ÷1.1).
    pub flash_color: f32,
    /// The flash sprites' fade per unit of colour.
    pub flash_fade: f32,
    /// The second flash sprite's scale speed.
    pub flash_scale_speed: f32,
    /// The flash radius factor in third person and for other players.
    pub third_person_flash: f64,
    /// The muzzle blast pushes particles (`blast_push`).
    pub muzzle_blast: bool,
    /// An unsilenced flash ends with a `scintille_metal.spr` spark at the muzzle (radius × 3,
    /// colour 0.5, fade 0.09).
    pub muzzle_spark: bool,
    /// The silenced flash's share of the sprites.
    pub silenced_count: f32,
    /// The silenced flash's colour per unit inside 512 units.
    pub silenced_color: f32,
    /// The silenced flash's first fade (÷1.1 each); `None`: `flash_fade × colour`.
    pub silenced_fade: Option<f32>,
    /// The silenced flash sprites' roll speed range (`RandomLong(-n, n)`; 0: none).
    pub silenced_spin: i32,
    /// The gun smoke's gravity.
    pub smoke_gravity: f32,
    /// The gun smoke has 10 puffs at `cl_fx 2` and none at `cl_fx 0` (else always 5).
    pub smoke_puffs_by_cl_fx: bool,
    /// The drag of the muzzle flash, gun smoke, impact smoke and small puffs.
    pub drag: f32,
    /// Impact smoke and small puffs stretch with their speed.
    pub stretch: bool,
    /// How much sprite debris reacts to bullet and blast pushes.
    pub debris_react: f32,
    /// Sprite debris fades (0.05) whatever its render mode, not only when additive.
    pub debris_fade_any_mode: bool,
    /// Sprite debris counts are scaled by the quality's particle multiplier (at most 1).
    pub debris_count_by_quality: bool,
}

/// The active ruleset's builder parameters.
fn style() -> &'static FxStyle {
    crate::gameplay::current().fx_style()
}

/// The weapon-info fields the muzzle flash reads (client `s_weapon_info`).
#[derive(Clone, Copy, Debug, Default)]
pub struct FlashInfo {
    /// `+0x350`: 1 = `ts_muzzleflash6.spr` core, else a `ts_muzzleflash1h.spr` core.
    pub kind: i32,
    /// `+0x354`: length the flash sprites are spread over.
    pub length: f32,
    /// `+0x358`: base sprite radius.
    pub radius: i32,
    /// `+0x1c`: damage, the muzzle blast's push radius.
    pub damage: f32,
}

/// `TS_MuzzleFlash(org, vel, info, fwd, n, silenced, fwdofs, thirdperson)` (`0x100474b0`).
///
/// Unsilenced: `max(3, n·fxmul)` roll sprites of `ts_muzzleflash1h.spr` spread over
/// `info.length` along `fwd`, each ×0.909 smaller and dimmer, a core sprite, and the muzzle blast
/// push. Silenced: 70% of the sprites, dim (colour ≤ 0.3), no core, gone beyond 512 units.
#[allow(clippy::too_many_arguments)]
pub fn muzzle_flash(
    ps: &mut PSys,
    now: f64,
    org: Vec3,
    vel: Vec3,
    fwd: Vec3,
    info: &FlashInfo,
    n: i32,
    silenced: bool,
    fwdofs: f32,
    third: bool,
) {
    let st = style();
    let Some(s1) = ps.sprite("sprites/ts_muzzleflash1h.spr", ADD) else { return };
    let radius = if third { (info.radius as f64 * st.third_person_flash) as i32 } else { info.radius } as f32;
    let count = ((n as f32 * ps.tuning.mult) as i32).max(3);
    let sprite = |i: i32, count: i32, size: f32, color: f32, fadespeed: f32, scalespeed: f32, spin: f32| Particle {
        sprite: s1,
        origin: v::ma(org, fwdofs + info.length * i as f32 / count as f32, fwd),
        vel: v::add(vel, [0.0, 0.0, rf(-3.0, 3.0) * TS]),
        radius: size,
        color,
        fadespeed,
        scalespeed,
        life: 0.0,
        drag: st.drag,
        orient: psys::ORIENT_ROLL,
        avel: [0.0, 0.0, spin * TS],
        ..Default::default()
    };
    if silenced {
        let d = v::dist(org, ps.cull_origin);
        if d > 512.0 {
            return;
        }
        let c = (512.0 - d) * st.silenced_color;
        let count = (count as f32 * st.silenced_count) as i32;
        let mut k = 1.0f32;
        for i in 0..count {
            let spin = if st.silenced_spin != 0 { rl(-st.silenced_spin, st.silenced_spin) as f32 } else { 0.0 };
            let fade = st.silenced_fade.map_or(st.flash_fade * (c * k), |f| f * k);
            let mut p = sprite(i, count, rf(0.6, 1.0) * radius * k, c * k, fade, if i == 1 { 0.3 } else { 0.0 }, spin);
            p.frame = random_frame(ps, s1);
            ps.add(p, now);
            k *= 0.909;
        }
        return;
    }
    if st.muzzle_blast {
        ps.blast_push(org, fwd, info.damage, rf(50.0, 60.0) * radius);
    }
    let mut k = 1.0f32;
    for i in 0..count {
        let size = rf(0.7, 1.3) * radius * k * if i == 0 { 0.7 } else { 1.0 };
        let color = st.flash_color * k;
        let mut p =
            sprite(i, count, size, color, st.flash_fade * color, if i == 1 { st.flash_scale_speed } else { 0.0 }, rl(-90, 90) as f32);
        p.frame = random_frame(ps, s1);
        ps.add(p, now);
        k *= 0.909;
    }
    // The core (`0x100477dc..0x10047a1c`). TS doesn't check the `ts_muzzleflash6.spr` lookup
    // (0x100477f3): a missing sprite still adds the particle, with no sprite (never drawn).
    let core = if info.kind == 1 { ps.sprite("sprites/ts_muzzleflash6.spr", ADD).unwrap_or(-1) } else { s1 };
    // TS rolls a spin, the size, then a second spin over the first (0x10047861, 0x100478c7).
    let _ = rl(-10, 10);
    let (size, color) = if info.kind == 1 { (rf(1.4, 1.7) * radius, 0.6) } else { (rf(1.1, 1.4) * radius, 0.7) };
    let spin = rl(-10, 10) as f32;
    let frame = random_frame(ps, core);
    ps.add(
        Particle {
            sprite: core,
            origin: v::ma(org, fwdofs - 2.0, fwd),
            vel,
            radius: size,
            color,
            fadespeed: 0.13,
            life: 0.0,
            drag: st.drag,
            frame,
            orient: psys::ORIENT_ROLL,
            avel: [0.0, 0.0, spin * TS],
            ..Default::default()
        },
        now,
    );
    if st.muzzle_spark
        && let Some(spark) = ps.sprite("sprites/scintille_metal.spr", ADD)
    {
        let frame = random_frame(ps, spark);
        ps.add(
            Particle {
                sprite: spark,
                origin: org,
                radius: radius * 3.0,
                color: 0.5,
                fadespeed: 0.09,
                life: 0.0,
                frame,
                ..Default::default()
            },
            now,
        );
    }
}

/// `TS_MuzzleDlight`'s rate limit: the client time of the last light (`0x1018a45c`), outside the
/// particle system, so `PSys_Init` doesn't reset it.
#[derive(Clone, Copy, Debug, Default)]
pub struct MuzzleTimer {
    last: f32,
}

impl MuzzleTimer {
    /// `0x10048e6d..0x10048ec4`: the time since the last light (`0x1018a460`). When it is
    /// negative the client clock went back (a level change restarts it): the timer restarts from
    /// now, with no light. Otherwise a light is due once `interval` (`SlowTime(0.1)` with this
    /// call's time scale) has passed, and the timer restarts.
    pub fn ready(&mut self, now: f32, interval: f32) -> bool {
        let elapsed = now - self.last;
        if elapsed < 0.0 {
            self.last = now;
            return false;
        }
        if interval > elapsed {
            return false;
        }
        self.last = now;
        true
    }
}

/// `TS_MuzzleDlight` (`0x10048e60`): radius 192, colour (240, 221, 172), life 0.06/ts, at most
/// one every 0.1/ts seconds by its own timer ([`MuzzleTimer`]). Off while the adaptive quality is
/// below 0.3 or with `cl_fx 0` (`0x1018ca58`). `_now` is unused: TS asks `GetClientTime`.
pub fn muzzle_dlight(ps: &mut PSys, _now: f64, org: Vec3) {
    if !ps.tuning.dlights {
        return;
    }
    let interval = ps.slow(0.1);
    if !ps.muzzle_timer.ready(engine().client_time(), interval) {
        return;
    }
    if let Some(dl) = r::alloc_dlight(0) {
        dl.origin = org;
        dl.radius = 192.0;
        dl.color.r = 240;
        dl.color.g = 221;
        dl.color.b = 172;
        dl.die = engine().client_time() + ps.slow(0.06);
        dl.decay = 0.0;
    }
}

/// `TS_GunSmoke(radius, framerate, org, vel, dir, speed, force)` (`0x10047130`): five
/// `gun_smoke_add.spr` puffs along `dir`, each ×0.8 smaller, living for one run of the animation
/// at `framerate ± 50%`. Unless `force`, at most one burst per 0.1/ts s (slot 1). Skipped with
/// `cl_fx 0` and when the muzzle is out of view.
#[allow(clippy::too_many_arguments)]
pub fn gun_smoke(ps: &mut PSys, now: f64, radius: f32, framerate: i32, org: Vec3, vel: Vec3, dir: Vec3, speed: f32, force: bool) {
    if !force && !ps.cooldown_ready(1, now, ps.slow(0.1)) {
        return;
    }
    let st = style();
    let cl_fx = super::cvar(|c| c.fx);
    if !ps.tuning.smoke || (st.smoke_puffs_by_cl_fx && cl_fx == 0.0) || !ps.visible(org) {
        return;
    }
    let Some(s) = ps.sprite("sprites/gun_smoke_add.spr", ADD) else { return };
    let puffs = if st.smoke_puffs_by_cl_fx && cl_fx == 2.0 { 10 } else { 5 };
    let mut r = radius;
    for (j, k) in (1..=puffs).rev().enumerate() {
        let origin = if j == 0 { v::ma(org, r * 0.5, dir) } else { v::ma(org, r * (j - 1) as f32, dir) };
        let jitter = [rf(-0.2 * r, 0.2 * r) * TS, rf(-0.2 * r, 0.2 * r) * TS, 0.0];
        let half = framerate / 2;
        ps.add(
            Particle {
                sprite: s,
                origin,
                vel: v::add(v::ma(vel, speed / k as f32 * TS, dir), jitter),
                radius: r,
                gravity: st.smoke_gravity,
                color: 0.3,
                life: 0.0,
                scalespeed: 0.1,
                drag: st.drag,
                framerate: (framerate + rl(-half, half)) as f32,
                ..Default::default()
            },
            now,
        );
        r *= 0.8;
    }
}

/// `TS_SpriteDebris(pos, n, sprite, rendermode, cnt, radius, spread, life, gravity, nocd,
/// collide)` (`0x10046bd0`): `cnt/2..cnt` (× `min(fxmul, 1)`) spinning chips thrown along `n`
/// with integer spread. Also the shotgun's muzzle pellet spray. Unless `nocd`, it needs one of
/// the cooldown slots 2-5.
#[allow(clippy::too_many_arguments)]
pub fn sprite_debris(
    ps: &mut PSys,
    now: f64,
    pos: Vec3,
    n: Vec3,
    sprite: &str,
    rendermode: i32,
    cnt: i32,
    radius: f32,
    spread: i32,
    life: f32,
    gravity: f32,
    nocd: bool,
    collide: bool,
) {
    if !ps.visible(pos) {
        return;
    }
    if !nocd {
        let iv = ps.slow(ps.tuning.interval);
        if !(2..=5).any(|slot| ps.cooldown_ready(slot, now, iv)) {
            return;
        }
    }
    let Some(s) = ps.sprite(sprite, rendermode) else { return };
    let st = style();
    let count = if st.debris_count_by_quality { (rl(cnt / 2, cnt) as f32 * ps.tuning.mult.min(1.0)) as i32 } else { rl(cnt / 2, cnt) };
    for _ in 0..count {
        let spin = rl(-180, 180) as f32;
        let jitter = [rl(-spread, spread) as f32, rl(-spread, spread) as f32, rl(-spread / 2, spread / 2) as f32];
        let frame = random_frame(ps, s);
        ps.add(
            Particle {
                sprite: s,
                origin: v::ma(pos, 2.0, n),
                vel: v::scale(v::add(v::scale(n, 16.0), jitter), TS),
                gravity,
                radius,
                color: 1.0,
                fadespeed: if rendermode == ADD || st.debris_fade_any_mode { 0.05 } else { 0.0 },
                react: st.debris_react,
                life: life + spin * 0.001,
                frame,
                orient: psys::ORIENT_ROLL,
                avel: [0.0, 0.0, spin * TS],
                collide,
                ..Default::default()
            },
            now,
        );
    }
}

/// `0x10046a70`: one static mark lying on the surface (metal impacts: `shotgun_pellets.spr`).
/// Culled by the view cone only.
#[allow(clippy::too_many_arguments)]
pub fn surface_mark(
    ps: &mut PSys,
    now: f64,
    pos: Vec3,
    n: Vec3,
    sprite: &str,
    rendermode: i32,
    radius: f32,
    color: f32,
    fadespeed: f32,
    scalespeed: f32,
) {
    if !psys::in_view_cone(&ps.cam, ps.cull_origin, pos) {
        return;
    }
    let Some(s) = ps.sprite(sprite, rendermode) else { return };
    let a = vec_to_angles(n);
    let frame = random_frame(ps, s);
    ps.add(
        Particle {
            sprite: s,
            origin: v::ma(pos, 0.1, n),
            radius,
            color,
            fadespeed: if rendermode == ADD { fadespeed } else { 0.0 },
            scalespeed,
            life: 0.0,
            frame,
            orient: psys::ORIENT_FREE,
            angles: a,
            ..Default::default()
        },
        now,
    );
}

/// TS's `VecToAngles` (`0x10046110`): the SDK `VectorAngles` with each angle truncated to whole
/// degrees *before* a negative one gets 360 added, and the pitch negated (AngleVectors'
/// convention). Straight up/down is (-90 / -270, 0, 0).
pub fn vec_to_angles(f: Vec3) -> Vec3 {
    if f[0] == 0.0 && f[1] == 0.0 {
        return [if f[2] > 0.0 { -90.0 } else { -270.0 }, 0.0, 0.0];
    }
    let deg = |y: f32, x: f32| ((y as f64).atan2(x as f64) * 57.295_779_513_082_32) as i32 as f32;
    let wrap = |a: f32| if a < 0.0 { a + 360.0 } else { a };
    let yaw = if f[0] != 0.0 {
        wrap(deg(f[1], f[0]))
    } else if f[1] > 0.0 {
        90.0
    } else {
        270.0
    };
    let pitch = wrap(deg(f[2], (f[0] * f[0] + f[1] * f[1]).sqrt()));
    [-pitch, yaw, 0.0]
}

/// `TS_Sparks(org, dir, n, scale, spread)` (`0x100492b0`): `sprites/sparks.spr`, `n/2..n`
/// (× `min(fxmul, 1)`) stretched sparks thrown along `dir` with gravity. Not drawn beyond 512
/// units (1024 zoomed). The long-lived ones collide when the quality is high.
pub fn sparks(ps: &mut PSys, now: f64, org: Vec3, dir: Vec3, n: i32, scale: f32, spread: f32) {
    let max = if (ps.cam.fov as i32) < 40 { 1024.0 } else { 512.0 };
    if v::dist(org, ps.cull_origin) >= max || !psys::in_view_cone(&ps.cam, ps.cull_origin, org) {
        return;
    }
    let Some(s) = ps.sprite("sprites/sparks.spr", ADD) else { return };
    let count = (rl(n / 2, n) as f32 * ps.tuning.mult.min(1.0)) as i32;
    for _ in 0..count {
        let life = rf(0.1, 1.0);
        let jitter = [rf(-spread, spread), rf(-spread, spread), rf(0.0, spread)];
        let frame = random_frame(ps, s);
        ps.add(
            Particle {
                sprite: s,
                origin: org,
                vel: v::scale(v::add(v::scale(dir, 17.0), jitter), TS),
                radius: rf(0.5, 1.15) * scale,
                life,
                gravity: 1.0,
                color: 0.9,
                fadespeed: 0.05,
                stretch: 0.35,
                drag: 0.1,
                frame,
                collide: life > 0.7 && ps.tuning.mult > 0.9,
                ..Default::default()
            },
            now,
        );
    }
}

/// `TS_Scintille(org, dir, sprite, rendermode, size)` (`0x10046f50`): a flash off a surface, two
/// billboards (the second one growing) for sizes up to 24, else one, `size/2` apart along `dir`.
pub fn scintille(ps: &mut PSys, now: f64, org: Vec3, dir: Vec3, sprite: &str, rendermode: i32, size: f32) {
    let Some(s) = ps.sprite(sprite, rendermode) else { return };
    let count = if size <= 24.0 { 2 } else { 1 };
    for i in 0..count {
        let frame = random_frame(ps, s);
        ps.add(
            Particle {
                sprite: s,
                origin: v::ma(org, (i + 1) as f32 * size * 0.5, dir),
                vel: v::scale(dir, 2.0 * TS),
                radius: size - i as f32,
                color: 0.7,
                fadespeed: 0.05,
                scalespeed: if i == 1 { 1.0 } else { 0.0 },
                life: 0.0,
                frame,
                ..Default::default()
            },
            now,
        );
    }
}

/// `scintille_metal.spr`, additive: the usual [`scintille`] call.
pub fn scintille_metal(ps: &mut PSys, now: f64, org: Vec3, dir: Vec3, size: f32) {
    scintille(ps, now, org, dir, "sprites/scintille_metal.spr", ADD, size);
}

/// `TS_ImpactSmoke(pos, n, sprite, rendermode, count, size, framerate, nocd)` (`0x10046680`):
/// `count` stretched billboards along `n`, each ×0.8 smaller, living for one run of the
/// animation. Unless `nocd`, slot 0 rate-limits it.
#[allow(clippy::too_many_arguments)]
pub fn impact_smoke(
    ps: &mut PSys,
    now: f64,
    pos: Vec3,
    n: Vec3,
    sprite: &str,
    rendermode: i32,
    count: i32,
    size: f32,
    framerate: i32,
    nocd: bool,
) {
    if !nocd && !ps.cooldown_ready(0, now, ps.slow(ps.tuning.puff_interval)) {
        return;
    }
    if !ps.visible(pos) {
        return;
    }
    let Some(s) = ps.sprite(sprite, rendermode) else { return };
    let half = (framerate as f32 * 0.5) as i32;
    let mut r = size;
    for j in 0..count {
        let origin = if j == 0 { v::ma(pos, size * 0.5, n) } else { v::ma(pos, r * (j - 1) as f32, n) };
        let vel = v::add(v::scale(n, rf(8.0, 24.0) / (count - j) as f32 * TS), [rf(-2.0, 2.0) * TS, rf(-2.0, 2.0) * TS, 0.0]);
        ps.add(
            Particle {
                sprite: s,
                origin,
                vel,
                radius: r,
                color: 0.5,
                gravity: 0.1,
                drag: style().drag,
                scalespeed: 0.1,
                stretch: if style().stretch { 7.0 } else { 0.0 },
                life: 0.0,
                framerate: (framerate + rl(-half, half)) as f32,
                ..Default::default()
            },
            now,
        );
        r *= 0.8;
    }
}

/// `TS_SmallPuff(pos, n, sprite, rendermode, radius, framerate)` (`0x100464f0`): one sinking,
/// growing puff (melee hits, kevlar hits). Slot 0 rate-limits it.
#[allow(clippy::too_many_arguments)]
pub fn small_puff(ps: &mut PSys, now: f64, pos: Vec3, n: Vec3, sprite: &str, rendermode: i32, radius: f32, framerate: f32) {
    if !ps.cooldown_ready(0, now, ps.slow(ps.tuning.puff_interval)) || !ps.visible(pos) {
        return;
    }
    let Some(s) = ps.sprite(sprite, rendermode) else { return };
    ps.add(
        Particle {
            sprite: s,
            origin: v::ma(pos, radius * 0.5, n),
            vel: [0.0, 0.0, -2.0 * TS],
            radius,
            color: 1.0,
            framerate,
            scalespeed: 0.5,
            drag: style().drag,
            stretch: if style().stretch { 7.0 } else { 0.0 },
            life: 0.0,
            ..Default::default()
        },
        now,
    );
}

/// `TS_ImpactParticles(pos, n, type, tex)` (`0x10048f60`): the material decides the effect.
/// `bullet_type` is `TS_PlayTextureSound`'s: 1 buckshot, 2 penetration exit. One shared cooldown
/// (slot 7) that types 1 and 2 ignore; culled by the view cone and a visibility trace.
pub fn impact(ps: &mut PSys, now: f64, pos: Vec3, n: Vec3, tex: u8, bullet_type: i32) {
    if crate::gameplay::current().impact_particles(ps, now, pos, n, tex, bullet_type) {
        return;
    }
    let ready = ps.cooldown_ready(7, now, ps.slow(ps.tuning.interval));
    if !ready && !matches!(bullet_type, 1 | 2) {
        return;
    }
    if !ps.visible(pos) {
        return;
    }
    let puffs = match bullet_type {
        1 => 1,
        2 => 2,
        _ => 4,
    };
    let roll = rl(0, 100);
    let (wood, smoke) = ("sprites/wood_smoke_add.spr", "sprites/gun_smoke_add.spr");
    match tex {
        b'D' => impact_smoke(ps, now, pos, n, wood, ADD, puffs, 16.0, 30, true),
        b'W' => {
            sprite_debris(ps, now, pos, n, "sprites/woodfrag.spr", ADD, 5, 5.0, 32, 1.0, 0.75, false, true);
            impact_smoke(ps, now, pos, n, wood, ADD, puffs, 16.0, 30, true);
        }
        b'G' | b'M' | b'V' => {
            if roll > 33 {
                scintille_metal(ps, now, pos, n, 48.0);
            }
            sparks(ps, now, pos, n, 23, 1.15, 25.0);
            if roll > 70 {
                surface_mark(ps, now, pos, n, "sprites/shotgun_pellets.spr", ADD, 5.0, 0.75, 0.015, 0.25);
            }
            impact_smoke(ps, now, pos, n, smoke, ADD, puffs, 16.0, 50, true);
        }
        b'P' | b'Y' => {
            scintille_metal(ps, now, pos, n, 48.0);
            if roll > 50 {
                sparks(ps, now, pos, n, 3, 1.15, 25.0);
            }
            impact_smoke(ps, now, pos, n, smoke, ADD, puffs, 16.0, 50, true);
        }
        b'F' | b'S' => {}
        _ => {
            sprite_debris(ps, now, pos, n, "sprites/debris_concrete3.spr", kRenderTransColor as i32, 5, 4.0, 32, 1.0, 1.0, false, true);
            impact_smoke(ps, now, pos, n, smoke, ADD, puffs, 16.0, 20, true);
            if roll > 50 {
                scintille_metal(ps, now, pos, n, 3.0);
                sparks(ps, now, pos, n, 5, 1.15, 25.0);
            }
        }
    }
}

/// Blood particle count for `TS_BloodImpact`: `clamp(a·0.2, 5, 15)` (halved on kevlar), scaled by
/// `cl_gore` (above 10: × `min(g, 33)`; else 1 ×0.5, 2 ×1, 3 ×2.5, 4 ×5, 5+ ×10, ≤ 0 none).
pub fn blood_count(a: i32, kevlar: bool, gore: f32) -> i32 {
    let mut n = (a as f32 * 0.2).clamp(5.0, 15.0) as i32;
    if kevlar {
        n /= 2;
    }
    if gore > 10.0 {
        return (n as f32 * gore.min(33.0)) as i32;
    }
    let k = match gore.min(5.0) as i32 {
        1 => 0.5,
        2 => 1.0,
        3 => 2.5,
        4 => 5.0,
        5 => 10.0,
        _ => 0.0,
    };
    (n as f32 * k) as i32
}

/// `TS_BloodImpact(pos, dir, amount)` (`0x10048840`, `PTakeDam`): `bloodfrag2.spr` (TransColor)
/// thrown back toward the shooter in a falling chain, the first drop forward along `dir`; each
/// drop that hits the world leaves a `{bloodN` decal ([`psys::ONCOLLIDE_BLOOD`], handled by
/// `effects`). Kevlar hits add a smoke puff and halve the spray. `amount` is the low 7 bits.
pub fn blood(ps: &mut PSys, now: f64, pos: Vec3, dir: Vec3, amount: i32, kevlar: bool, gore: f32) {
    if !ps.visible(pos) {
        return;
    }
    let mut d = v::scale(dir, -1.0);
    let a = if kevlar {
        d = v::normalize([d[0] + rf(-0.3, 0.3), d[1] + rf(-0.3, 0.3), d[2] + rf(-0.1, 0.1)]);
        small_puff(ps, now, pos, v::scale(d, 6.0), "sprites/gun_smoke_add.spr", ADD, 4.0, 20.0);
        amount
    } else {
        amount * 2
    };
    let radius = (a as f32 * 0.075).clamp(1.0, 5.0);
    let n = blood_count(a, kevlar, gore);
    let Some(s) = ps.sprite("sprites/bloodfrag2.spr", kRenderTransColor as i32) else { return };
    d[2] += rf(0.5, 1.0);
    let dn = v::normalize(d);
    let (mut at, mut acc) = (pos, 0.0f32);
    for i in 0..n {
        let (origin, vel) = if i == 0 {
            (pos, v::scale(dir, 32.0 * TS))
        } else {
            (at, v::add(v::scale(dn, rf(8.0, 24.0) * TS), [rf(-1.0, 1.0) * TS, rf(-1.0, 1.0) * TS, 0.0]))
        };
        if r::point_contents(origin) == CONTENTS_SOLID {
            break;
        }
        let frame = random_frame(ps, s);
        ps.add(
            Particle {
                sprite: s,
                origin,
                vel,
                radius,
                gravity: 0.5,
                color: 1.0,
                stretch: 2.0,
                scalespeed: 0.075,
                life: rf(1.0, 2.0),
                frame,
                collide: true,
                oncollide: psys::ONCOLLIDE_BLOOD,
                ..Default::default()
            },
            now,
        );
        if i >= 1 {
            at = v::ma(at, radius * 0.3, dn);
            acc += radius * 0.1;
            at[2] -= acc / n as f32;
        }
    }
}

/// The thin axis of a breakable's box and the other two (`TS_GlassShatter`'s tie rules).
fn glass_axes(size: Vec3) -> (usize, usize, usize) {
    let thin = if size[0] > size[1] {
        if size[1] > size[2] { 2 } else { 1 }
    } else if size[0] > size[2] {
        2
    } else {
        0
    };
    let mut others = (0..3).filter(|&i| i != thin);
    (thin, others.next().unwrap_or(0), others.next().unwrap_or(1))
}

/// `TS_GlassShatter(org, size, dir, hitpoint, dmgbits, force)` (`0x10047cb0`, `TSFrags` type 0):
/// 81 `glassfrag.spr` shards on a 9×9 grid across the two large axes of the box. Bullet-like
/// damage (`dmgbits & 0x08000002`) blows them away from the hit point, scaled by distance;
/// otherwise they scatter around `dir·force`.
#[allow(clippy::too_many_arguments)]
pub fn glass(ps: &mut PSys, now: f64, org: Vec3, size: Vec3, dir: Vec3, hit: Vec3, dmg_bits: i32, force: f32) {
    let Some(s) = ps.sprite("sprites/glassfrag.spr", ADD) else { return };
    let (_, a1, a2) = glass_axes(size);
    let radius = ((size[a1] + size[a2]) * (1.0 / 18.0) * 0.7).min(20.0);
    for k1 in 0..9 {
        for k2 in 0..9 {
            let mut p = org;
            p[a1] = org[a1] - size[a1] * 0.5 + size[a1] * (1.0 / 9.0) * k1 as f32;
            p[a2] = org[a2] - size[a2] * 0.5 + size[a2] * (1.0 / 9.0) * k2 as f32;
            let vel = if dmg_bits & 0x0800_0002 != 0 {
                let push = force / (v::dist(p, hit) + 1.0) * 10.0;
                v::ma([rf(-9.0, 9.0), rf(-9.0, 9.0), rl(0, 10) as f32], push, dir)
            } else {
                v::scale(dir, force).map(|c| rf(c * 0.5, c + 15.0))
            };
            let frame = random_frame(ps, s);
            ps.add(
                Particle {
                    sprite: s,
                    origin: p,
                    vel: v::scale(vel, TS),
                    gravity: 1.0,
                    radius,
                    color: 0.7,
                    fadespeed: 0.05,
                    react: 0.5,
                    life: rf(2.0, 5.0),
                    frame,
                    orient: psys::ORIENT_ROLL,
                    avel: [0.0, 0.0, rl(-90, 90) as f32 * TS],
                    collide: true,
                    ..Default::default()
                },
                now,
            );
        }
    }
}

/// `TS_Explosion(org, radius)` (`0x100481b0`, `TSFrags` type 1): sound, particle push, a
/// `grenade_asplode.spr` fireball and `gun_smoke_add.spr` smoke (24 and 48 particles × the
/// quality scale, each living for one run of its animation), the muzzle light and a growing flash.
/// The sound plays at TS's fire pitch (`events::fire_pitch`, 400 at normal speed).
pub fn explosion(ps: &mut PSys, now: f64, org: Vec3, radius: f32) {
    const SOUNDS: [&str; 5] = ["explo/explode.wav", "explo/explode1.wav", "explo/explode2.wav", "explo/explode3.wav", "explo/explode4.wav"];
    let snd = SOUNDS[rl(0, 4).clamp(0, 4) as usize];
    engine().ev_play_sound(0, org, CHAN_STATIC as i32, snd, rf(0.92, 1.0), 0.2, 0, crate::events::fire_pitch());
    ps.push_radial(org, 512.0, radius * 10.0);
    let scale = ps.tuning.scale;
    // A sprite that fails to load ends the whole effect (`je 0x100486e7`): no smoke, light or flash.
    let cloud = |ps: &mut PSys, sprite: &str, count: f32, size: f32, rise: f32, speed: f32, fr: (i32, i32)| -> bool {
        let Some(s) = ps.sprite(sprite, ADD) else { return false };
        for _ in 0..(scale * count) as i32 {
            ps.add(
                Particle {
                    sprite: s,
                    origin: v::add(org, [0.0, 0.0, radius * rise]),
                    vel: v::scale([rf(-speed, speed), rf(-speed, speed), rf(0.0, speed)], TS),
                    radius: radius * size,
                    gravity: 0.01,
                    color: 0.3,
                    life: 0.0,
                    scalespeed: 0.1,
                    drag: 3.3,
                    framerate: rl(fr.0, fr.1) as f32,
                    ..Default::default()
                },
                now,
            );
        }
        true
    };
    if !cloud(ps, "sprites/grenade_asplode.spr", 24.0, 0.5, 0.25, 70.0, (25, 30))
        || !cloud(ps, "sprites/gun_smoke_add.spr", 48.0, 0.7, 0.35, 54.0, (2, 10))
    {
        return;
    }
    muzzle_dlight(ps, now, v::add(org, [0.0, 0.0, 32.0]));
    if let Some(s) = ps.sprite("sprites/scintille_metal.spr", ADD) {
        let frame = random_frame(ps, s);
        ps.add(
            Particle {
                sprite: s,
                origin: v::add(org, [0.0, 0.0, radius]),
                radius: radius * 2.0,
                color: 1.0,
                fadespeed: 0.05,
                scalespeed: 2.0,
                life: 0.0,
                frame,
                // TS reuses the smoke's scratch particle, so the flash keeps its drag (0x10048540).
                drag: 3.3,
                ..Default::default()
            },
            now,
        );
    }
}

/// `TS_BulletTrail(p1, p2, w1, w2, alpha)` (`0x10046240`): a `bullet_trail.spr` quad from the
/// last trail point `p2` to the bullet at `p1`, `w` wide along `view_up` (this pass's view up,
/// `0x1018bda8`), colour `alpha·0.75`. The client bullets pass `w` for both widths.
pub fn bullet_trail(ps: &mut PSys, now: f64, p1: Vec3, p2: Vec3, w: f32, alpha: f32, view_up: Vec3) {
    let Some(s) = ps.sprite("sprites/bullet_trail.spr", ADD) else { return };
    let up = v::scale(view_up, w);
    ps.add(
        Particle {
            sprite: s,
            origin: p1,
            radius: w,
            color: alpha * 0.75,
            fadespeed: 0.1,
            life: 0.0,
            corners: Some([v::sub(p2, up), v::add(p2, up), v::add(p1, up), v::sub(p1, up)]),
            ..Default::default()
        },
        now,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn muzzle_timer() {
        let mut t = MuzzleTimer::default();
        assert!(t.ready(500.0, 0.1), "first light");
        assert!(!t.ready(500.05, 0.1), "within 0.1 s");
        assert!(t.ready(500.125, 0.1), "0.1 s later");
        // The interval is this call's: in slow motion (ts 0.25) 0.4 s, at normal speed 0.1 s again.
        assert!(!t.ready(500.25, 0.4));
        assert!(t.ready(500.25, 0.1), "a refusal doesn't restart the timer");
        // A level change restarts the client clock: no light this call, then the new clock rules.
        assert!(!t.ready(2.0, 0.1), "the clock went back");
        assert!(!t.ready(2.05, 0.1));
        assert!(t.ready(2.125, 0.1));
        assert!(t.ready(2.125, 0.0), "a zero interval allows the same instant");
    }

    /// `VecToAngles` (`0x10046110`) truncates before wrapping: -45.7° yaw is 315, not 314.
    #[test]
    fn vec_to_angles_truncates_first() {
        let a = vec_to_angles([1.0, (-45.7f32).to_radians().tan(), 0.0]);
        assert_eq!(a, [-0.0, 315.0, 0.0]);
        assert_eq!(vec_to_angles([0.0, 0.0, 1.0]), [-90.0, 0.0, 0.0]);
        assert_eq!(vec_to_angles([0.0, -1.0, 0.0])[1], 270.0);
        // Pitch up 30°: stored negated.
        assert_eq!(vec_to_angles([3f32.sqrt(), 0.0, 1.0])[0], -30.0);
    }

    #[test]
    fn blood_count_follows_cl_gore() {
        assert_eq!(blood_count(10, false, 2.0), 5, "clamped up to 5");
        assert_eq!(blood_count(100, false, 2.0), 15, "clamped down to 15");
        assert_eq!(blood_count(50, true, 2.0), 5, "kevlar halves");
        assert_eq!(blood_count(50, false, 1.0), 5);
        assert_eq!(blood_count(50, false, 3.0), 25);
        assert_eq!(blood_count(50, false, 9.0), 100, "5 and up: x10");
        assert_eq!(blood_count(50, false, 20.0), 200);
        assert_eq!(blood_count(50, false, 0.0), 0);
    }

    #[test]
    fn glass_grid_uses_the_two_large_axes() {
        assert_eq!(glass_axes([64.0, 2.0, 48.0]), (1, 0, 2));
        assert_eq!(glass_axes([2.0, 64.0, 48.0]), (0, 1, 2));
        assert_eq!(glass_axes([64.0, 48.0, 2.0]), (2, 0, 1));
    }
}
