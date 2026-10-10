//! `EV_TS_Fire` (`0x1003de40`) and its variants, `EV_TS_FireBullets` (`0x100450c0`), shell
//! ejection (`0x10056890`) and slow-motion client bullets (`0x10035c20`...).

use super::impact::{self, AMMO_ATTENUATION, MAX_RANGE, SPREAD_RADIUS};
use super::random::shared_random_float;
use super::{Args, fire_pitch, first_person_view_model, info, slow_pitch, time_scale};
use crate::{effects, state, view};
use goldsrc::ext::render::{self as r, Vec3, vec3 as v};
use goldsrc::sys::*;
use shared::weapons::tables;

pub fn fire(a: &Args) {
    match info::class(a.i1) {
        4 | 5 => fire_akimbo(a, false),
        1 => fire_pump(a),
        _ => fire_standard(a),
    }
}

pub fn fire_linked(a: &Args) {
    fire_akimbo(a, true)
}

/// `GetWeaponBody` (`0x10007110`): view-model body for the attachments in `items`.
pub fn weapon_body(id: i32, items: i32) -> i32 {
    let Some(asset) = tables().asset(id as u8) else { return 0 };
    let b = asset.body;
    let mut body = 0;
    if items & 1 != 0 {
        body += b[0];
    }
    if items & 6 == 6 && b[4] != -1 {
        body += b[4];
    } else {
        if items & 2 != 0 {
            body += b[1];
        }
        if items & 4 != 0 {
            body += b[2];
        }
    }
    if items & 8 != 0 {
        body += b[3];
    } else if id == 36 {
        body += 1;
    }
    body
}

/// Current view-model sequence set (`GetActiveAnimSet` `0x1003df60`).
fn anim_set(id: i32) -> &'static [i16; 29] {
    static EMPTY: [i16; 29] = [0; 29];
    let alt = crate::prediction::alt_grip();
    info::info(id).map_or(&EMPTY, |i| i.set(alt))
}

struct Shot {
    id: i32,
    silenced: bool,
    fwd: Vec3,
    right: Vec3,
    up: Vec3,
    shell: i32,
    shell_snd: i32,
    shell_right: f32,
    shell_r: f32,
}

impl Shot {
    fn new(a: &Args) -> Shot {
        let id = a.i1;
        let (fwd, right, up) = a.vectors();
        let shell_name = tables().asset(id as u8).map(|s| s.shell.clone()).unwrap_or_default();
        let shell = if shell_name.is_empty() { 0 } else { r::model_index(&format!("models/{shell_name}.mdl")) };
        let shell_right = if id == 17 { r::random_long(-40, 40) } else { r::random_long(120, 140) } as f32;
        Shot {
            id,
            silenced: a.i2 & 1 != 0,
            fwd,
            right,
            up,
            shell,
            shell_snd: if id == 11 { 2 } else { 1 },
            shell_right,
            shell_r: r::random_long(50, 100) as f32,
        }
    }

    fn shell_velocity(&self, a: &Args) -> Vec3 {
        let mut vel = a.velocity;
        vel = v::ma(vel, self.shell_right, self.right);
        vel = v::ma(vel, self.shell_r, self.up);
        v::ma(vel, self.shell_r + 50.0, self.fwd)
    }

    fn muzzle_ofs(&self) -> f32 {
        tables().asset(self.id as u8).map_or(0, |s| s.fx[0]) as f32
    }
    fn flash(&self) -> (i32, f32) {
        tables().asset(self.id as u8).map_or((0, 0.0), |s| (s.fx[3], s.fx[2] as f32))
    }
}

/// `EV_TS_EjectShell` (`0x10056890`): `R_TempModel` with TS's own shell sound type in
/// `curstate.iuser1` (played by `temp_ent_update`), spinning, ring of 64.
pub fn eject_shell(origin: Vec3, vel: Vec3, angles: Vec3, model: i32, snd: i32) {
    if model == 0 {
        return;
    }
    let ang = [angles[0], angles[1] - 90.0, angles[2]];
    let life = effects::shell_life();
    if let Some(t) = r::temp_model(origin, vel, ang, life, model, 0) {
        t.entity.curstate.iuser1 = snd;
        t.flags |= FTENT_ROTATE as i32;
        t.entity.curstate.body = 0;
        t.entity.curstate.iuser4 = 12;
        let x = r::random_float(-60.0, 60.0);
        let y = r::random_float(-60.0, 60.0);
        t.entity.baseline.angles = [x * x, y * y, y * y * 0.5];
        effects::track_shell(t);
    }
}

fn player_attachment(entindex: i32, n: usize) -> Option<Vec3> {
    r::entity(entindex).map(|e| e.attachment[n])
}

/// Muzzle/smoke/shell effects at a world position (third person and other players). `fade` is
/// 1 near the viewer and 0 by 512 units.
fn world_effects(a: &Args, s: &Shot, muzzle: Vec3, shell_from: Vec3, pellets: i32) {
    let now = effects::now();
    let fade = 1.0 - (v::dist(muzzle, view::origin()) / 512.0).min(1.0);
    effects::with_psys(|ps| {
        if !s.silenced {
            let (count, size) = s.flash();
            effects::fx::muzzle_flash(ps, now, muzzle, s.fwd, count, size, fade);
        }
        effects::fx::gun_smoke(ps, now, muzzle, s.fwd, if s.id == 33 { r::random_long(5, 10) } else { r::random_long(45, 55) });
        if pellets > 1 {
            effects::fx::pellet_spray(ps, now, muzzle, s.fwd, pellets);
        }
    });
    eject_shell(shell_from, s.shell_velocity(a), a.angles, s.shell, s.shell_snd);
}

/// First-person effects at the view model's attachment `att`.
fn view_model_effects(a: &Args, s: &Shot, att: usize, pellets: i32) {
    let Some(vm) = first_person_view_model() else {
        // No view model (scoped): shell from the view origin.
        eject_shell(view::origin(), s.shell_velocity(a), a.angles, s.shell, s.shell_snd);
        return;
    };
    let muzzle = vm.attachment[att];
    let now = effects::now();
    effects::with_psys(|ps| {
        if !s.silenced {
            let (count, size) = s.flash();
            effects::fx::muzzle_flash(ps, now, muzzle, s.fwd, count, size, 1.0);
            effects::fx::gun_smoke(ps, now, muzzle, s.fwd, if s.id == 33 { r::random_long(5, 10) } else { r::random_long(45, 55) });
        } else {
            effects::fx::gun_smoke(ps, now, muzzle, s.fwd, r::random_long(50, 60));
        }
        if pellets > 1 {
            effects::fx::pellet_spray(ps, now, v::ma(muzzle, 3.0, s.fwd), s.fwd, pellets);
        }
    });
    let shell_from = v::ma(muzzle, -s.muzzle_ofs(), s.fwd);
    eject_shell(shell_from, s.shell_velocity(a), a.angles, s.shell, s.shell_snd);
}

fn muzzle_light(a: &Args, s: &Shot) {
    if s.silenced || !(a.is_local() || effects::player_visible(a.entindex)) {
        return;
    }
    let pos = v::ma(v::ma(a.origin, 16.0, s.fwd), 32.0, s.up);
    let now = effects::now();
    effects::with_psys(|ps| effects::fx::muzzle_dlight(ps, now, pos, time_scale()));
}

/// The weapon fire sound (`0x1003de90`).
fn fire_sound(a: &Args, id: i32, silenced: bool) {
    let Some(asset) = tables().asset(id as u8) else { return };
    let ammo = tables().weapon(id as u8).map_or(0, |w| w.ammo) as usize;
    let atten = AMMO_ATTENUATION.get(ammo).copied().unwrap_or(0.4);
    let (sample, vol, attn) = if silenced && !asset.sound2.is_empty() {
        (&asset.sound2, r::random_float(0.84, 0.92), atten * 2.0)
    } else {
        (&asset.sound, r::random_float(0.92, 1.0), atten)
    };
    if sample.is_empty() {
        return;
    }
    r::play_sound(a.entindex, a.origin, CHAN_WEAPON as i32, &format!("weapons/{sample}"), vol, attn, 0, fire_pitch());
}

fn bullets_for(a: &Args, s: &Shot, src: Vec3, seed: i32, f1: f32, f2: f32) {
    let Some(w) = tables().weapon(s.id as u8) else { return };
    let dmg = w.damage * if s.silenced { 0.775 } else { 1.0 };
    fire_bullets(&Bullets {
        shooter: a.entindex,
        right: s.right,
        up: s.up,
        pellets: w.pellets.max(1),
        src,
        dir: s.fwd,
        spread: [f1, f2],
        dmg,
        bullet_type: w.bullet,
        ammo: w.ammo as i32,
        seed,
    });
}

/// `EV_TS_FireStandard` (`0x1003dfa0`).
fn fire_standard(a: &Args) {
    let s = Shot::new(a);
    let pellets = tables().weapon(s.id as u8).map_or(1, |w| w.pellets);
    muzzle_light(a, &s);
    if a.is_local() {
        let set = anim_set(s.id);
        let seq = if a.b1 != 0 { set[6] } else { set[3 + r::random_long(0, 2) as usize] };
        r::weapon_animation(seq as i32, weapon_body(s.id, a.i2));
    }
    if a.is_local() && !view::is_third_person() {
        view_model_effects(a, &s, 0, pellets);
    } else if let Some(att) = player_attachment(a.entindex, if matches!(info::class(s.id), 6 | 7) { 3 } else { 0 }) {
        if a.is_local() || effects::player_visible(a.entindex) {
            world_effects(a, &s, att, v::ma(att, -s.muzzle_ofs(), s.fwd), pellets);
        }
    }
    fire_sound(a, s.id, s.silenced);
    bullets_for(a, &s, a.gun_position(), a.b2, a.f1, a.f2);
}

/// `EV_TS_FirePump` (`0x10040c20`): pump shotguns. `b1` = firemode is pump; the shell is ejected
/// here only in semi mode (pump mode ejects it from `pump.sc`).
fn fire_pump(a: &Args) {
    let s = Shot::new(a);
    let pellets = tables().weapon(s.id as u8).map_or(8, |w| w.pellets);
    muzzle_light(a, &s);
    if a.is_local() {
        let set = anim_set(s.id);
        r::weapon_animation(set[if a.b1 != 0 { 11 } else { 12 }] as i32, weapon_body(s.id, a.i2));
    }
    let shell = if a.b1 != 0 { Shot { shell: 0, ..s } } else { s };
    if a.is_local() && !view::is_third_person() {
        view_model_effects(a, &shell, 0, pellets);
    } else if let Some(att) = player_attachment(a.entindex, 3) {
        if a.is_local() || effects::player_visible(a.entindex) {
            world_effects(a, &shell, att, v::ma(att, -shell.muzzle_ofs(), shell.fwd), pellets);
        }
    }
    fire_sound(a, shell.id, false);
    bullets_for(a, &shell, a.gun_position(), a.b2, a.f1, a.f2);
}

/// `EV_TS_FireAkimbo` (`0x1003f090`): `b1` selects the gun; linked fires both, offset +-5 along
/// right, the second batch with `seed + 0x21`.
fn fire_akimbo(a: &Args, linked: bool) {
    let s = Shot::new(a);
    let pellets = tables().weapon(s.id as u8).map_or(1, |w| w.pellets);
    muzzle_light(a, &s);
    let hands: &[usize] = if linked {
        &[0, 1]
    } else if a.b1 != 0 {
        &[1]
    } else {
        &[0]
    };
    if a.is_local() {
        let set = anim_set(s.id);
        let seq = if linked {
            set[26 + r::random_long(0, 1) as usize]
        } else if a.b1 != 0 {
            set[24 + r::random_long(0, 1) as usize]
        } else {
            set[22 + r::random_long(0, 1) as usize]
        };
        r::weapon_animation(seq as i32, weapon_body(s.id, a.i2));
    }
    for &h in hands {
        if a.is_local() && !view::is_third_person() {
            view_model_effects(a, &s, h, pellets);
        } else if let Some(att) = player_attachment(a.entindex, h) {
            if a.is_local() || effects::player_visible(a.entindex) {
                world_effects(a, &s, att, v::ma(att, -s.muzzle_ofs(), s.fwd), pellets);
            }
        }
    }
    fire_sound(a, s.id, s.silenced);
    let src = a.gun_position();
    if linked {
        bullets_for(a, &s, v::ma(src, -5.0, s.right), a.b2, a.f1, a.f2);
        bullets_for(a, &s, v::ma(src, 5.0, s.right), a.b2.wrapping_add(0x21), a.f1, a.f2);
    } else {
        bullets_for(a, &s, src, a.b2, a.f1, a.f2);
    }
}

/// Parameters of `EV_TS_FireBullets`.
#[derive(Clone, Copy, Debug)]
pub struct Bullets {
    pub shooter: i32,
    pub right: Vec3,
    pub up: Vec3,
    pub pellets: i32,
    pub src: Vec3,
    pub dir: Vec3,
    pub spread: [f32; 2],
    pub dmg: f32,
    pub bullet_type: i32,
    pub ammo: i32,
    pub seed: i32,
}

/// Pellet `i` direction (1-based), as both server and client compute it.
pub fn pellet_dir(dir: Vec3, right: Vec3, up: Vec3, spread: [f32; 2], seed: i32, i: i32) -> Vec3 {
    let s = (seed as u32).wrapping_add(i as u32);
    let ang = shared_random_float(s, -std::f32::consts::PI, std::f32::consts::PI);
    let r = shared_random_float(s.wrapping_add(1), 0.0, SPREAD_RADIUS);
    let m = 1.0 - r * r * r;
    let (x, y) = (ang.cos() * m, ang.sin() * m);
    v::add(v::add(dir, v::scale(right, x * spread[0])), v::scale(up, y * spread[1]))
}

/// Whizz-by sound for other players' shots passing near the view (`< 75` units).
fn whizz(b: &Bullets) {
    if r::is_local(b.shooter) {
        return;
    }
    let end = v::ma(b.src, 4096.0, b.dir);
    let tr = r::player_trace(b.src, end, 2, PM_STUDIO_BOX as i32, -1);
    let line = v::sub(tr.endpos, b.src);
    let len = v::len(line);
    let eye = view::origin();
    if len <= 200.0 || v::dist(tr.endpos, eye) >= len - 100.0 {
        return;
    }
    let to_eye = v::sub(eye, b.src);
    let t = (v::dot(to_eye, line) / (len * len)).clamp(0.0, 1.0);
    let h = v::dist(eye, v::ma(b.src, t, line));
    if h < 75.0 {
        let chan = effects::next_whizz_channel();
        let snd = format!("weapons/whizz{}.wav", r::random_long(1, 5));
        let vol = (2.0 / (h * 0.1 + 0.1)).min(1.0);
        r::play_sound(-1, eye, chan, &snd, vol, 0.8, 0, slow_pitch());
    }
}

/// `EV_TS_FireBullets` (`0x100450c0`): mirrors the server's `ShootBulletsPlayer` so decals land
/// where the server's bullets hit. In slow motion (or bullet time) the bullets are queued as
/// visible client bullets instead of traced.
pub fn fire_bullets(b: &Bullets) {
    let ammo = tables().ammo(b.ammo as u8);
    let (pierce, unit, velocity) = ammo.map_or((0.0, 80.0, 330.0), |a| (a.pierce, a.range_unit, a.velocity));
    let slow = state::with(|s| s.bullet_time) || time_scale() < 0.99;

    if b.bullet_type == 2 {
        let end = v::ma(b.src, 4096.0, b.dir);
        r::with_solid_players(b.shooter - 1, || {
            let mut tr = r::player_trace(b.src, end, 2, PM_STUDIO_BOX as i32, -1);
            if tr.fraction < 1.0 {
                impact::texture_sound(b.dmg, &mut tr, b.src, end, 2);
                impact::decal_gunshot(&mut tr);
            }
        });
        return;
    }

    whizz(b);
    let push = if b.bullet_type == 1 { b.dmg * 8.0 } else { b.dmg };
    effects::with_psys(|ps| {
        ps.push_radial(b.src, 512.0, push);
        ps.push_along_line(b.src, v::ma(b.src, 4096.0, b.dir), push);
    });

    let mut remaining = 0.0f32;
    let mut last_dir = b.dir;
    let mut last_hit = None;
    for i in 1..=b.pellets.max(1) {
        let d = pellet_dir(b.dir, b.right, b.up, b.spread, b.seed, i);
        last_dir = d;
        let end = v::ma(b.src, MAX_RANGE, d);
        if slow {
            effects::add_client_bullet(ClientBullet::new(b.shooter, b.src, d, b.dmg, b.bullet_type, b.ammo, velocity));
            continue;
        }
        r::with_solid_players(b.shooter - 1, || {
            let mut tr = r::player_trace(b.src, end, 2, PM_STUDIO_BOX as i32, -1);
            if tr.fraction < 1.0 {
                let dist = v::dist(b.src, tr.endpos);
                let idx = (dist / (unit * 10.0)) as i32;
                remaining = impact::range_mult(idx) * b.dmg;
                let bt = if b.bullet_type == 1 { 1 } else { b.bullet_type };
                impact::texture_sound(b.dmg, &mut tr, b.src, end, bt);
                impact::decal_gunshot(&mut tr);
                remaining *= pierce;
                last_hit = Some(tr.endpos);
            } else {
                last_hit = None;
            }
        });
    }

    // Penetration of the last pellet's hit (bullet type 0 only on the client: TS skips buckshot).
    if slow || b.bullet_type == 1 || remaining <= 5.0 {
        return;
    }
    let Some(hit) = last_hit else { return };
    let step = remaining.min(48.0);
    let p = v::ma(hit, step, last_dir);
    if r::point_contents(p) == CONTENTS_SOLID {
        return;
    }
    let mut tr = r::player_trace(p, hit, 2, 0, -1);
    if tr.fraction < 1.0 {
        impact::texture_sound(b.dmg, &mut tr, p, hit, 2);
        impact::decal_gunshot(&mut tr);
    }
    fire_bullets(&Bullets { pellets: 1, src: p, dir: last_dir, spread: [0.0; 2], dmg: remaining, bullet_type: 2, ..*b });
}

/// A slow-motion bullet (`ClientBullets_*`, 256 slots in TS): flies at
/// `velocity * 40 u/s * GetBulletVelocityMult`, leaves a `bullet_trail.spr` streak and runs the
/// impact code when it hits.
#[derive(Clone, Copy, Debug)]
pub struct ClientBullet {
    pub shooter: i32,
    pub pos: Vec3,
    pub start: Vec3,
    pub dir: Vec3,
    pub dmg: f32,
    pub bullet_type: i32,
    pub ammo: i32,
    pub speed: f32,
    pub travelled: f32,
}

impl ClientBullet {
    pub fn new(shooter: i32, src: Vec3, dir: Vec3, dmg: f32, bullet_type: i32, ammo: i32, velocity_mps: f32) -> Self {
        ClientBullet {
            shooter,
            pos: src,
            start: src,
            dir: v::normalize(dir),
            dmg,
            bullet_type,
            ammo,
            speed: velocity_mps * 40.0,
            travelled: 0.0,
        }
    }
}

/// `GetBulletVelocityMult(ts, bt)` (`0x10055b30`).
pub fn bullet_velocity_mult(ts: f32, bullet_time: bool) -> f32 {
    if ts >= 0.99 {
        return 1.0;
    }
    ts / ((1.0 - ts * ts) * if bullet_time { 10.0 } else { 1.0 }).max(1.0)
}

/// Steps the client bullets (`ClientBullets_Frame`/`_Update`/`_Impact`). Returns the survivors.
pub fn client_bullets_frame(bullets: &mut Vec<ClientBullet>, frametime: f32) {
    let ts = time_scale();
    let bt = state::with(|s| s.bullet_time);
    let mult = bullet_velocity_mult(ts, bt);
    let now = effects::now();
    bullets.retain_mut(|b| {
        let step = b.speed * frametime * mult;
        let next = v::ma(b.pos, step, b.dir);
        let hit = r::with_solid_players(b.shooter - 1, || {
            let mut tr = r::player_trace(b.pos, next, 2, PM_STUDIO_BOX as i32, -1);
            if tr.fraction < 1.0 {
                impact::texture_sound(b.dmg, &mut tr, b.pos, next, b.bullet_type);
                impact::decal_gunshot(&mut tr);
                Some(tr.endpos)
            } else {
                None
            }
        });
        let to = hit.unwrap_or(next);
        effects::with_psys(|ps| effects::fx::bullet_trail(ps, now, b.pos, to));
        b.travelled += v::dist(b.pos, to);
        b.pos = to;
        hit.is_none() && b.travelled < MAX_RANGE
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pellet_spread_is_seeded_and_bounded() {
        let (dir, right, up) = ([1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, 1.0]);
        let a = pellet_dir(dir, right, up, [0.05, 0.05], 77, 1);
        let b = pellet_dir(dir, right, up, [0.05, 0.05], 77, 1);
        assert_eq!(a, b);
        for i in 1..=8 {
            let d = pellet_dir(dir, right, up, [0.05, 0.05], 1000, i);
            assert_eq!(d[0], 1.0);
            assert!(d[1].hypot(d[2]) <= 0.05 + 1e-6);
        }
        assert_eq!(pellet_dir(dir, right, up, [0.0, 0.0], 5, 1), dir);
    }

    #[test]
    fn velocity_mult() {
        assert_eq!(bullet_velocity_mult(1.0, false), 1.0);
        assert!((bullet_velocity_mult(0.35, false) - 0.35).abs() < 1e-6);
        let bt = bullet_velocity_mult(0.35, true);
        assert!((bt - 0.35 / ((1.0 - 0.35 * 0.35) * 10.0)).abs() < 1e-6);
    }
}
