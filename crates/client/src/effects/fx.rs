//! Weapon and impact effects built on the sprite particle system (rendering doc §4.4-4.5).
//!
//! Every number is read from the TS client's disassembly (the decompile hides the float
//! arguments). Velocities are TS's own units per 0.1 s and angular velocities degrees per 0.1 s,
//! as the particles hold them ([`Particle::vel`]). Fields TS leaves at zero in its scratch
//! particle (`0x1004ffe0`) are left at zero, and `PSys::add` applies TS's drag floor of 0.1.

use super::psys::{self, PSys, Particle};
use goldsrc::client::engine;
use goldsrc::ext::render::{self as r, Vec3, vec3 as v};
use goldsrc::sys::*;

const ADD: i32 = kRenderTransAdd as i32;

fn rf(a: f32, b: f32) -> f32 {
    r::random_float(a, b)
}

fn rl(a: i32, b: i32) -> i32 {
    r::random_long(a, b)
}

/// `0x100509a0`: a random frame of `sprite` (`RandomLong(0, frames − 1)`).
pub fn random_frame(ps: &PSys, sprite: i16) -> f32 {
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
    /// The flash sprites' fade per unit of colour (a double in both builds).
    pub flash_fade: f64,
    /// The second flash sprite's scale speed.
    pub flash_scale_speed: f32,
    /// The flash radius factor in third person and for other players.
    pub third_person_flash: f64,
    /// The muzzle blast pushes particles (`blast_push`).
    pub muzzle_blast: bool,
    /// An unsilenced flash ends with a `scintille_metal.spr` spark at the muzzle (radius × 3,
    /// colour 0.5, fade 0.09).
    pub muzzle_spark: bool,
    /// The silenced flash's share of the sprites (a double: `ftol(count × 0.7)`).
    pub silenced_count: f64,
    /// The silenced flash's colour per unit inside 512 units (a double).
    pub silenced_color: f64,
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
    // Audit round 4: hud
    /// The silenced flash sprites get the unsilenced ones' `RandomFloat(-3, 3)` rise.
    pub silenced_jitter: bool,
    /// The muzzle spark moves with the gun's velocity.
    pub spark_moves: bool,
    /// The cooldowns of the gun smoke, sprite debris and small puffs stretch by the raw
    /// `TSSlowMo` factor (`1 / max(s, 0.1f)`, 1 at normal speed) instead of `SlowTime`.
    pub raw_slow_cooldowns: bool,
    /// The impact smoke's speed along the normal (TS units per 0.1 s) before `/ (count − j)`:
    /// `None` rolls `RandomFloat(8, 24)` per puff.
    pub impact_smoke_speed: Option<f32>,
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

/// `TS_MuzzleFlash(org, vel, info, fwd, n, silenced, fwdofs, thirdperson)` (`0x100474b0`; 2.1
/// `0x10080a30`). `vel` is in TS units: the events pass the shooter's velocity `/ 10.0f`
/// ([`psys::ts_units`]).
///
/// Unsilenced: `max(3, n·fxmul)` roll sprites of `ts_muzzleflash1h.spr` spread over
/// `info.length` along `fwd`, each 1.1 times smaller and dimmer, a core sprite, and the muzzle
/// blast push. Silenced: 70% of the sprites, dim (colour ≤ 0.3), no core, gone beyond 512 units.
/// TS rolls each sprite's size, its rise (`RandomFloat(-3, 3)`), the frame, then its spin; a
/// failed `PSys_Add` ends the flash.
///
/// The float forms follow the build ([`DllBuild`](crate::gameplay::DllBuild)): 3.0 multiplies
/// the running colour and radius by the double `1/1.1` (`0x1004779f`) and works each origin out
/// in one x87 expression (`0x10047673`); 2.1 divides by 1.1 and stores every value
/// (`0x10080d8c`, `0x10080b91`).
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
    use crate::gameplay::DllBuild;
    let st = style();
    let build = crate::gameplay::current().dll_build();
    let radius = if third { (info.radius as f64 * st.third_person_flash) as i32 } else { info.radius } as f32;
    // The blast push comes before the sprite lookup (`0x10047508..0x1004754a`).
    if !silenced && st.muzzle_blast {
        ps.blast_push(org, fwd, info.damage, rf(50.0, 60.0) * radius);
    }
    // `ftol(n × fxmul)` off the x87 (`0x1004755b`, 2.1 `0x10080a73`), at least 3.
    let count = ((n as f64 * ps.tuning.mult as f64) as i32).max(3);
    let Some(s1) = ps.sprite("sprites/ts_muzzleflash1h.spr", ADD) else { return };
    let decay = |x: f64| match build {
        DllBuild::Optimised => x * (1.0 / 1.1),
        DllBuild::Debug => x / 1.1,
    };
    // Sprite `i` of `count` along `fwd`.
    let origin = |i: i32, count: i32| -> Vec3 {
        let (len, fo) = (info.length as f64, fwdofs as f64);
        [0, 1, 2].map(|k| {
            let (f, o) = (fwd[k] as f64, org[k] as f64);
            match build {
                DllBuild::Optimised => (len * i as f64 * f / count as f64 + fo * f + o) as f32,
                DllBuild::Debug => {
                    let a = (i as f32 as f64 * f * len / count as f64 + o) as f32;
                    (fo * f + a as f64) as f32
                }
            }
        })
    };
    let sprite = |origin: Vec3, size: f32, color: f32, fadespeed: f32, scalespeed: f32| Particle {
        sprite: s1,
        origin,
        vel,
        radius: size,
        color,
        fadespeed,
        scalespeed,
        life: 0.0,
        drag: st.drag,
        orient: psys::ORIENT_ROLL,
        ..Default::default()
    };
    let rise = |p: &mut Particle| p.vel[2] += rf(-3.0, 3.0);
    if silenced {
        // 3.0: the x87 length of the stored offset (`0x10047a67`); 2.1: its own float length
        // (`0x10097fc0`).
        let off = v::sub(org, ps.cull_origin);
        let d = match build {
            DllBuild::Optimised => len_x87(off),
            DllBuild::Debug => len_stored(off) as f64,
        };
        if d > 512.0 {
            return;
        }
        let mut c = ((512.0 - d) * st.silenced_color) as f32;
        let mut fade = st.silenced_fade.unwrap_or((c as f64 * st.flash_fade) as f32);
        let mut rr = radius;
        let count = (count as f64 * st.silenced_count) as i32;
        for i in 0..count {
            let size = (rf(0.6, 1.0) as f64 * rr as f64) as f32;
            let mut p = sprite(origin(i, count), size, c, fade, if i == 1 { 0.3 } else { 0.0 });
            if st.silenced_jitter {
                rise(&mut p);
            }
            if st.silenced_spin != 0 {
                p.avel[2] = rl(-st.silenced_spin, st.silenced_spin) as f32;
            }
            p.frame = random_frame(ps, s1);
            if !ps.add(p, now) {
                return;
            }
            let next = decay(c as f64);
            c = next as f32;
            fade = match (build, st.silenced_fade) {
                (DllBuild::Debug, Some(_)) => decay(fade as f64) as f32,
                (DllBuild::Debug, None) => (c as f64 * st.flash_fade) as f32,
                (DllBuild::Optimised, _) => (next * st.flash_fade) as f32,
            };
            rr = decay(rr as f64) as f32;
        }
        return;
    }
    let mut color = st.flash_color;
    let mut fade = (color as f64 * st.flash_fade) as f32;
    let mut rr = radius;
    for i in 0..count {
        let t = rf(0.7, 1.3) as f64 * rr as f64;
        let size = match (build, i) {
            (DllBuild::Optimised, 0) => (t * 0.7) as f32,
            (DllBuild::Debug, 0) => (t as f32 as f64 * 0.7) as f32,
            _ => t as f32,
        };
        let mut p = sprite(origin(i, count), size, color, fade, if i == 1 { st.flash_scale_speed } else { 0.0 });
        rise(&mut p);
        p.frame = random_frame(ps, s1);
        p.avel[2] = rl(-90, 90) as f32;
        if !ps.add(p, now) {
            return;
        }
        let next = decay(color as f64);
        color = next as f32;
        fade = match build {
            DllBuild::Optimised => (next * st.flash_fade) as f32,
            DllBuild::Debug => (color as f64 * st.flash_fade) as f32,
        };
        rr = decay(rr as f64) as f32;
    }
    // The core (`0x100477dc..0x10047a1c`). TS doesn't check the `ts_muzzleflash6.spr` lookup
    // (0x100477f3): a missing sprite still adds the particle, with no sprite (never drawn).
    let core = if info.kind == 1 { ps.sprite("sprites/ts_muzzleflash6.spr", ADD).unwrap_or(-1) } else { s1 };
    // TS rolls a spin, the size, then a second spin over the first (0x10047861, 0x100478c7).
    let _ = rl(-10, 10);
    let (size, color) = if info.kind == 1 { (rf(1.4, 1.7) * radius, 0.6) } else { (rf(1.1, 1.4) * radius, 0.7) };
    let spin = rl(-10, 10) as f32;
    let frame = random_frame(ps, core);
    // `(fwdofs − 2) × fwd + org`, rounded once.
    let core_origin = [0, 1, 2].map(|k| ((fwdofs as f64 - 2.0) * fwd[k] as f64 + org[k] as f64) as f32);
    ps.add(
        Particle {
            sprite: core,
            origin: core_origin,
            vel,
            radius: size,
            color,
            fadespeed: 0.13,
            life: 0.0,
            drag: st.drag,
            frame,
            orient: psys::ORIENT_ROLL,
            avel: [0.0, 0.0, spin],
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
                vel: if st.spark_moves { vel } else { v::ZERO },
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

/// `0x10058390`: a vector's length on the x87, the squares summed and the root taken unrounded.
fn len_x87(a: Vec3) -> f64 {
    a.iter().fold(0.0f64, |s, &x| s + x as f64 * x as f64).sqrt()
}

/// 2.1's length (`0x10097fc0`): each square added to a stored float sum, the root stored.
fn len_stored(a: Vec3) -> f32 {
    let s = a.iter().fold(0.0f32, |s, &x| (x as f64 * x as f64 + s as f64) as f32);
    (s as f64).sqrt() as f32
}

/// `Vector::Normalize` (`0x10029e40`): the length on the x87, then each component times its
/// reciprocal, stored. A zero vector is left as it is.
fn normalize_x87(a: Vec3) -> Vec3 {
    let len = len_x87(a);
    if len == 0.0 {
        return a;
    }
    let inv = 1.0 / len;
    a.map(|c| (inv * c as f64) as f32)
}

/// `p + n × k` as TS's optimised vector code leaves it: the products for x and y stored as
/// floats, z's kept on the x87, then each sum stored (`0x1004660f`, `0x10047095`).
fn ma_z_unrounded(p: Vec3, k: f64, n: Vec3) -> Vec3 {
    [
        (p[0] as f64 + (n[0] as f64 * k) as f32 as f64) as f32,
        (p[1] as f64 + (n[1] as f64 * k) as f32 as f64) as f32,
        (p[2] as f64 + n[2] as f64 * k) as f32,
    ]
}

/// The effect cooldowns' interval for `x` seconds: `SlowTime(x)`, or with
/// [`FxStyle::raw_slow_cooldowns`] `x × slowdiv(s)` on the raw `TSSlowMo` factor `s`
/// (`0x100962e0`: 1 at normal speed, else `1 / max(s, 0.1f)`).
fn cooldown(ps: &PSys, x: f32) -> f32 {
    if !style().raw_slow_cooldowns {
        return ps.slow(x);
    }
    let s = ps.slowmo;
    let div = if s == 0.0 { 1.0 } else { 1.0 / (if s < 0.1 { 0.1f32 } else { s }) as f64 };
    (div * x as f64) as f32
}

/// `ftol(n × min(fxmul, 1))` on the x87 (`0x10046ded`, `0x10049406`).
fn quality_count(ps: &PSys, n: i32) -> i32 {
    (n as f64 * ps.tuning.mult.min(1.0) as f64) as i32
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
    /// `interval` is compared as the x87 leaves it (`0x10048e9e`).
    pub fn ready(&mut self, now: f32, interval: f64) -> bool {
        let elapsed = now - self.last;
        if elapsed < 0.0 {
            self.last = now;
            return false;
        }
        if interval > elapsed as f64 {
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
    let interval = ps.slow_x87(0.1);
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
/// `cl_fx 0` and when the muzzle is out of view. `vel` is in TS units ([`psys::ts_units`]).
#[allow(clippy::too_many_arguments)]
pub fn gun_smoke(ps: &mut PSys, now: f64, radius: f32, framerate: i32, org: Vec3, vel: Vec3, dir: Vec3, speed: f32, force: bool) {
    if !force && !ps.cooldown_ready(1, now, cooldown(ps, 0.1)) {
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
        // The frame rate is rolled before the jitter (`0x1004730c`, 2.1 `0x1007fbc0`), whose
        // bound is `r × 0.2f` (2.1: `r / 5.0f`, `0x1007fc33`).
        let half = framerate / 2;
        let framerate = (framerate + rl(-half, half)) as f32;
        let b = match crate::gameplay::current().dll_build() {
            crate::gameplay::DllBuild::Optimised => 0.2 * r,
            crate::gameplay::DllBuild::Debug => r / 5.0,
        };
        let jitter = [rf(-b, b), rf(-b, b), 0.0];
        let added = ps.add(
            Particle {
                sprite: s,
                origin,
                vel: v::add(v::ma(vel, speed / k as f32, dir), jitter),
                radius: r,
                gravity: st.smoke_gravity,
                color: 0.3,
                life: 0.0,
                scalespeed: 0.1,
                drag: st.drag,
                framerate,
                ..Default::default()
            },
            now,
        );
        if !added {
            return;
        }
        // The double 0.8 (`0x10047409`, 2.1 `0x1007fce2`).
        r = (r as f64 * 0.8) as f32;
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
        let iv = cooldown(ps, ps.tuning.interval);
        if !(2..=5).any(|slot| ps.cooldown_ready(slot, now, iv)) {
            return;
        }
    }
    let Some(s) = ps.sprite(sprite, rendermode) else { return };
    let st = style();
    let count = if st.debris_count_by_quality { quality_count(ps, rl(cnt / 2, cnt)) } else { rl(cnt / 2, cnt) };
    let build = crate::gameplay::current().dll_build();
    for _ in 0..count {
        let spin = rl(-180, 180) as f32;
        let jitter = [rl(-spread, spread) as f32, rl(-spread, spread) as f32, rl(-spread / 2, spread / 2) as f32];
        let frame = random_frame(ps, s);
        // `spin × 0.001 + life` with the double 0.001 (`0x10046ee9`); 2.1 `spin / 1000.0`
        // (`0x1007f28a`).
        let life = match build {
            crate::gameplay::DllBuild::Optimised => spin as f64 * 0.001 + life as f64,
            crate::gameplay::DllBuild::Debug => spin as f64 / 1000.0 + life as f64,
        } as f32;
        let added = ps.add(
            Particle {
                sprite: s,
                origin: v::ma(pos, 2.0, n),
                vel: v::add(v::scale(n, 16.0), jitter),
                gravity,
                radius,
                color: 1.0,
                fadespeed: if rendermode == ADD || st.debris_fade_any_mode { 0.05 } else { 0.0 },
                react: st.debris_react,
                life,
                frame,
                orient: psys::ORIENT_ROLL,
                avel: [0.0, 0.0, spin],
                collide,
                ..Default::default()
            },
            now,
        );
        if !added {
            return;
        }
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
    if !ps.in_view(pos) {
        return;
    }
    let Some(s) = ps.sprite(sprite, rendermode) else { return };
    let a = vec_to_angles(n);
    let frame = random_frame(ps, s);
    ps.add(
        Particle {
            sprite: s,
            origin: mark_origin(pos, n),
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
///
/// 3.0 takes the horizontal length on the x87 and multiplies by the double `180/π`
/// (`0x100461be`); 2.1 (`0x1007d6c0`) stores the length as a float and takes `× 180 / π`.
pub fn vec_to_angles(f: Vec3) -> Vec3 {
    vec_to_angles_as(crate::gameplay::current().dll_build(), f)
}

fn vec_to_angles_as(build: crate::gameplay::DllBuild, f: Vec3) -> Vec3 {
    use crate::gameplay::DllBuild;
    if f[0] == 0.0 && f[1] == 0.0 {
        return [if f[2] > 0.0 { -90.0 } else { -270.0 }, 0.0, 0.0];
    }
    let deg = |y: f64, x: f64| {
        let a = y.atan2(x);
        (match build {
            DllBuild::Optimised => a * 57.295_779_513_082_32,
            DllBuild::Debug => a * 180.0 / std::f64::consts::PI,
        }) as i32 as f32
    };
    let wrap = |a: f32| if a < 0.0 { a + 360.0 } else { a };
    let yaw = if f[0] != 0.0 {
        wrap(deg(f[1] as f64, f[0] as f64))
    } else if f[1] > 0.0 {
        90.0
    } else {
        270.0
    };
    let h = (f[0] as f64 * f[0] as f64 + f[1] as f64 * f[1] as f64).sqrt();
    let h = match build {
        DllBuild::Optimised => h,
        DllBuild::Debug => h as f32 as f64,
    };
    let pitch = wrap(deg(f[2] as f64, h));
    [-pitch, yaw, 0.0]
}

/// The surface mark's origin, `pos + n × 0.1f`: x's product kept on the x87, y's and z's
/// stored (`0x10046b2b..0x10046b65`).
fn mark_origin(pos: Vec3, n: Vec3) -> Vec3 {
    let k = 0.1f32 as f64;
    [
        (n[0] as f64 * k + pos[0] as f64) as f32,
        (pos[1] as f64 + (n[1] as f64 * k) as f32 as f64) as f32,
        (pos[2] as f64 + (n[2] as f64 * k) as f32 as f64) as f32,
    ]
}

/// `TS_Sparks(org, dir, n, scale, spread)` (`0x100492b0`): `sprites/sparks.spr`, `n/2..n`
/// (× `min(fxmul, 1)`) stretched sparks thrown along `dir` with gravity. Not drawn beyond 512
/// units (1024 zoomed). The long-lived ones collide when the quality is high.
pub fn sparks(ps: &mut PSys, now: f64, org: Vec3, dir: Vec3, n: i32, scale: f32, spread: f32) {
    let max = if (ps.cam.fov as i32) < 40 { 1024.0 } else { 512.0 };
    if v::dist(org, ps.cull_origin) >= max || !ps.in_view(org) {
        return;
    }
    let Some(s) = ps.sprite("sprites/sparks.spr", ADD) else { return };
    let count = quality_count(ps, rl(n / 2, n));
    for _ in 0..count {
        // TS's order (`0x10049472..0x1004951d`): radius, jitter, life, frame.
        let radius = rf(0.5, 1.15) * scale;
        let jitter = [rf(-spread, spread), rf(-spread, spread), rf(0.0, spread)];
        let life = rf(0.1, 1.0);
        let frame = random_frame(ps, s);
        let added = ps.add(
            Particle {
                sprite: s,
                origin: org,
                vel: v::add(v::scale(dir, 17.0), jitter),
                radius,
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
        if !added {
            return;
        }
    }
}

/// `TS_Scintille(org, dir, sprite, rendermode, size)` (`0x10046f50`): a flash off a surface, two
/// billboards (the second one growing) for sizes up to 24, else one, `size/2` apart along `dir`.
pub fn scintille(ps: &mut PSys, now: f64, org: Vec3, dir: Vec3, sprite: &str, rendermode: i32, size: f32) {
    let Some(s) = ps.sprite(sprite, rendermode) else { return };
    let count = if size <= 24.0 { 2 } else { 1 };
    let build = crate::gameplay::current().dll_build();
    for i in 0..count {
        let frame = random_frame(ps, s);
        // 3.0: `(i + 1) × (double)(size × 0.5)` with z's product kept (`0x10047059..0x10047095`).
        let origin = match build {
            crate::gameplay::DllBuild::Optimised => ma_z_unrounded(org, (i + 1) as f64 * (size as f64 * 0.5), dir),
            crate::gameplay::DllBuild::Debug => v::ma(org, (i + 1) as f32 * size * 0.5, dir),
        };
        let added = ps.add(
            Particle {
                sprite: s,
                origin,
                vel: v::scale(dir, 2.0),
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
        if !added {
            return;
        }
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
    if !nocd && !ps.cooldown_ready(0, now, cooldown(ps, ps.tuning.puff_interval)) {
        return;
    }
    if !ps.visible(pos) {
        return;
    }
    let Some(s) = ps.sprite(sprite, rendermode) else { return };
    let build = crate::gameplay::current().dll_build();
    let half = (framerate as f32 * 0.5) as i32;
    let mut r = size;
    let mut origin = v::ma(pos, size * 0.5, n);
    for j in 0..count {
        // The frame rate first (`0x100468b7`), then the speed, `f32(f32(n × speed) / k)` per
        // component, then the x and y jitter added to it (2.1 `0x1007e7f0`: the speed is 16).
        let framerate = (framerate + rl(-half, half)) as f32;
        let speed = style().impact_smoke_speed.unwrap_or_else(|| rf(8.0, 24.0));
        let k = (count - j) as f32 as f64;
        let b = n.map(|c| ((c as f64 * speed as f64) as f32 as f64 / k) as f32);
        let vel = [rf(-2.0, 2.0) + b[0], rf(-2.0, 2.0) + b[1], b[2]];
        let added = ps.add(
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
                framerate,
                ..Default::default()
            },
            now,
        );
        if !added {
            return;
        }
        // The double 0.8; the next origin is `pos + n × (r × 0.8 × j)`: 3.0 keeps the product
        // (`0x100469b4`, z's sum rounded once), 2.1 stores the radius and the product.
        let next = r as f64 * 0.8;
        r = next as f32;
        origin = match build {
            crate::gameplay::DllBuild::Optimised => ma_z_unrounded(pos, next * j as f64, n),
            crate::gameplay::DllBuild::Debug => v::ma(pos, r * j as f32, n),
        };
    }
}

/// `TS_SmallPuff(pos, n, sprite, rendermode, radius, framerate)` (`0x100464f0`): one sinking,
/// growing puff (melee hits, kevlar hits). Slot 0 rate-limits it.
#[allow(clippy::too_many_arguments)]
pub fn small_puff(ps: &mut PSys, now: f64, pos: Vec3, n: Vec3, sprite: &str, rendermode: i32, radius: f32, framerate: f32) {
    if !ps.cooldown_ready(0, now, cooldown(ps, ps.tuning.puff_interval)) || !ps.visible(pos) {
        return;
    }
    let Some(s) = ps.sprite(sprite, rendermode) else { return };
    // 3.0: `n × (radius × 0.5)` with z's product kept (`0x100465d2..0x1004661b`).
    let origin = match crate::gameplay::current().dll_build() {
        crate::gameplay::DllBuild::Optimised => ma_z_unrounded(pos, radius as f64 * 0.5, n),
        crate::gameplay::DllBuild::Debug => v::ma(pos, radius * 0.5, n),
    };
    ps.add(
        Particle {
            sprite: s,
            origin,
            vel: [0.0, 0.0, -2.0],
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
        // `ftol(n × min(gore, 33))` on the x87 (`0x10048aec`).
        return (n as f64 * gore.min(33.0) as f64) as i32;
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
        d = normalize_x87([d[0] + rf(-0.3, 0.3), d[1] + rf(-0.3, 0.3), d[2] + rf(-0.1, 0.1)]);
        small_puff(ps, now, pos, v::scale(d, 6.0), "sprites/gun_smoke_add.spr", ADD, 4.0, 20.0);
        amount
    } else {
        amount * 2
    };
    let radius = (a as f32 * 0.075).clamp(1.0, 5.0);
    let n = blood_count(a, kevlar, gore);
    let Some(s) = ps.sprite("sprites/bloodfrag2.spr", kRenderTransColor as i32) else { return };
    d[2] += rf(0.5, 1.0);
    let dn = normalize_x87(d);
    let (mut at, mut acc) = (pos, 0.0f32);
    for i in 0..n {
        // Per drop (`0x10048be8..0x10048d3f`): the frame, the life, then (after the first) the
        // speed and the x and y jitter, then the contents test. Velocities are `f32(dn × s)`
        // plus the jitter, in TS units.
        let frame = random_frame(ps, s);
        let life = rf(1.0, 2.0);
        let (origin, vel) = if i == 0 {
            (pos, dir.map(|c| (c as f64 * 32.0) as f32))
        } else {
            let speed = rf(8.0, 24.0);
            let b = dn.map(|c| (c as f64 * speed as f64) as f32);
            (at, [rf(-1.0, 1.0) + b[0], rf(-1.0, 1.0) + b[1], b[2]])
        };
        if r::point_contents(origin) == CONTENTS_SOLID {
            break;
        }
        let added = ps.add(
            Particle {
                sprite: s,
                origin,
                vel,
                radius,
                gravity: 0.5,
                color: 1.0,
                stretch: 2.0,
                scalespeed: 0.075,
                life,
                frame,
                collide: true,
                oncollide: psys::ONCOLLIDE_BLOOD,
                ..Default::default()
            },
            now,
        );
        if !added {
            return;
        }
        if i >= 1 {
            at = blood_step(at, &mut acc, radius, dn, n);
        }
    }
}

/// The blood chain's next drop position (`0x10048d42..0x10048df6`): `w = dn × r × 0.3f` (x's
/// and y's `dn × r` stored, z's kept), each `at + w` stored, `acc = r × 0.1f + acc` stored, then
/// `at.z −= acc / n` on the x87.
fn blood_step(at: Vec3, acc: &mut f32, r: f32, dn: Vec3, n: i32) -> Vec3 {
    let k = 0.3f32 as f64;
    let w = [
        ((dn[0] as f64 * r as f64) as f32 as f64 * k) as f32,
        ((dn[1] as f64 * r as f64) as f32 as f64 * k) as f32,
        (dn[2] as f64 * r as f64 * k) as f32,
    ];
    let next = [0, 1, 2].map(|i| (at[i] as f64 + w[i] as f64) as f32);
    *acc = (r as f64 * 0.1f32 as f64 + *acc as f64) as f32;
    [next[0], next[1], (next[2] as f64 - *acc as f64 / n as f64) as f32]
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
    // `(s1 + s2) × (1/18)f × 0.7` on the x87, capped at 20 and stored (`0x10047d43..0x10047dca`).
    let r = (size[a1] as f64 + size[a2] as f64) * (1.0f32 / 18.0) as f64 * 0.7;
    let radius = if r < 20.0 { r as f32 } else { 20.0 };
    // `(org − s × 0.5f) + s × (1/9)f × k`, rounded once (`0x10047e82..0x10047ea0`).
    let cell = |a: usize, k: i32| (org[a] as f64 - size[a] as f64 * 0.5 + size[a] as f64 * (1.0f32 / 9.0) as f64 * k as f64) as f32;
    for k1 in 0..9 {
        for k2 in 0..9 {
            let mut p = org;
            p[a1] = cell(a1, k1);
            p[a2] = cell(a2, k2);
            // The frame first (`0x10047f5c`), then the velocity, spin and life.
            let frame = random_frame(ps, s);
            let vel = if dmg_bits & 0x0800_0002 != 0 {
                let b = [rf(-9.0, 9.0), rf(-9.0, 9.0), rl(0, 10) as f32];
                // `force / (|p − hit| + 1)` on the x87; x's and y's `dir × push` and `× 10f`
                // stored, z's kept, each plus its roll stored (`0x10047fcd..0x10048079`).
                let push = force as f64 / (len_x87(v::sub(p, hit)) + 1.0);
                let ten = |c: f64| c * 10.0;
                [
                    (ten((dir[0] as f64 * push) as f32 as f64) as f32 as f64 + b[0] as f64) as f32,
                    (ten((dir[1] as f64 * push) as f32 as f64) as f32 as f64 + b[1] as f64) as f32,
                    (ten(dir[2] as f64 * push) + b[2] as f64) as f32,
                ]
            } else {
                // `RandomFloat(c × 0.5, c + 15)` per axis, x then y then z (0x10048093..0x1004812f):
                // x's bounds from the unrounded `dir.x × force` (0x100480c6), y's and z's from the
                // stored products (`[esp+0x40]`, `[esp+0x44]`).
                let x = dir[0] as f64 * force as f64;
                let vx = rf((x * 0.5) as f32, (x + 15.0) as f32);
                let c = [dir[1] * force, dir[2] * force];
                let vy = rf(c[0] * 0.5, c[0] + 15.0);
                [vx, vy, rf(c[1] * 0.5, c[1] + 15.0)]
            };
            let spin = rl(-90, 90) as f32;
            let life = rf(2.0, 5.0);
            let added = ps.add(
                Particle {
                    sprite: s,
                    origin: p,
                    vel,
                    gravity: 1.0,
                    radius,
                    color: 0.7,
                    fadespeed: 0.05,
                    react: 0.5,
                    life,
                    frame,
                    orient: psys::ORIENT_ROLL,
                    avel: [0.0, 0.0, spin],
                    collide: true,
                    ..Default::default()
                },
                now,
            );
            if !added {
                return;
            }
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
    // So does a failed `PSys_Add` (`0x10048491`, `0x100485c5`). The count is `ftol(scale × n)`
    // on the x87 (`0x1004841c`, `0x10048550`); the size `radius × k` with the double k and the
    // rise half of it (`0x100484ca`).
    let cloud = |ps: &mut PSys, sprite: &str, count: f32, size: f64, speed: f32, fr: (i32, i32)| -> bool {
        let Some(s) = ps.sprite(sprite, ADD) else { return false };
        let r = radius as f64 * size;
        let origin = [org[0], org[1], (org[2] as f64 + r * 0.5) as f32];
        for _ in 0..(scale as f64 * count as f64) as i32 {
            let added = ps.add(
                Particle {
                    sprite: s,
                    origin,
                    vel: [rf(-speed, speed), rf(-speed, speed), rf(0.0, speed)],
                    radius: r as f32,
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
            if !added {
                return false;
            }
        }
        true
    };
    if !cloud(ps, "sprites/grenade_asplode.spr", 24.0, 0.5, 70.0, (25, 30))
        || !cloud(ps, "sprites/gun_smoke_add.spr", 48.0, 0.7, 54.0, (2, 10))
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
    // The first corner's x subtracts the unrounded `up.x × w` (`0x100462f3`).
    let mut c0 = v::sub(p2, up);
    c0[0] = (p2[0] as f64 - view_up[0] as f64 * w as f64) as f32;
    ps.add(
        Particle {
            sprite: s,
            origin: p1,
            radius: w,
            color: alpha * 0.75,
            fadespeed: 0.1,
            life: 0.0,
            corners: Some([c0, v::add(p2, up), v::add(p1, up), v::sub(p1, up)]),
            ..Default::default()
        },
        now,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `VecToAngles`: 3.0 keeps the horizontal length on the x87 (`0x100461be`), 2.1 stores it
    /// and takes `× 180 / π` (`0x1007d7d3`); a float length gave 3.0 the wrong whole degree.
    #[test]
    fn vec_to_angles_per_build() {
        use crate::gameplay::DllBuild;
        let f = [f32::from_bits(0xbe3c_185b), f32::from_bits(0x3e6f_d8e6), f32::from_bits(0x3eea_ae06)];
        assert_eq!(vec_to_angles_as(DllBuild::Optimised, f)[0], -57.0);
        assert_eq!(vec_to_angles_as(DllBuild::Debug, f)[0], -56.0);
    }

    /// `0x10048d42..0x10048df6`: the blood chain's step keeps TS's stores.
    #[test]
    fn blood_step_forms() {
        let mut acc = 0.0;
        let at = blood_step([0.0; 3], &mut acc, 2.0, [0.6, 0.0, 0.8], 10);
        assert_eq!(acc, (2.0f64 * 0.1f32 as f64) as f32);
        assert_eq!(at[0], ((0.6f32 as f64 * 2.0) as f32 as f64 * 0.3f32 as f64) as f32);
        assert_eq!(at[2], ((0.8f32 as f64 * 2.0 * 0.3f32 as f64) as f32 as f64 - acc as f64 / 10.0) as f32);
    }

    /// TS's `Vector::Normalize` (`0x10029e40`) multiplies by the x87 reciprocal of the x87 length.
    #[test]
    fn normalize_on_the_x87() {
        assert_eq!(normalize_x87([3.0, 0.0, 4.0]), [0.6, 0.0, 0.8]);
        assert_eq!(normalize_x87([0.0; 3]), [0.0; 3]);
        let a = [0.1f32, 0.2, 0.3];
        let l = (0.1f32 as f64 * 0.1f32 as f64 + 0.2f32 as f64 * 0.2f32 as f64 + 0.3f32 as f64 * 0.3f32 as f64).sqrt();
        assert_eq!(normalize_x87(a)[2], (1.0 / l * 0.3f32 as f64) as f32);
    }

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
