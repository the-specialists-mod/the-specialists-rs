//! `EV_TS_Fire` (`0x1003de40`) and its variants, `EV_TS_FireBullets` (`0x100450c0`), shell
//! ejection (`0x10056890`) and client bullets (`ClientBullets_*`, `0x10035b50`...).

use super::impact::{self, AMMO_ATTENUATION, MAX_RANGE, SPREAD_RADIUS};
use super::random::shared_random_float;
use super::{Args, first_person_view_model, info, slow_pitch};
use crate::effects::psys::{self, Camera, heron_height};
use crate::{effects, state, view};
use goldsrc::Global;
use goldsrc::client::engine;
use goldsrc::ext::render::{self as r, Vec3, vec3 as v};
use goldsrc::ext::tri;
use goldsrc::sys::*;
use shared::weapons::spread::{self, PelletOffset, PelletSide};
use shared::weapons::tables;
use std::cell::Cell;

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

/// Current view-model sequence set (`GetActiveAnimSet` `0x1003df60`): the alternative grip set,
/// except while spectating in-eye (the view model is someone else's gun then).
fn anim_set(id: i32) -> &'static [i16; 29] {
    static EMPTY: [i16; 29] = [0; 29];
    let alt = crate::prediction::alt_grip() && !crate::spectator::in_eye();
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
    /// Weapon info the muzzle flash reads: `fx` = (`+0x34c`, `+0x350`, `+0x354`, `+0x358`).
    fn flash(&self) -> effects::fx::FlashInfo {
        let fx = tables().asset(self.id as u8).map_or([0; 4], |s| s.fx);
        let damage = tables().weapon(self.id as u8).map_or(0.0, |w| w.damage);
        effects::fx::FlashInfo { kind: fx[1], length: fx[2] as f32, radius: fx[3], damage }
    }
}

/// Where one shot's effects go: the flash(es), the smoke, and the pellet spray (`None` for
/// akimbo, which has none).
struct FxAt<'a> {
    flashes: &'a [Vec3],
    smoke: Vec3,
    spray: Option<Vec3>,
}

/// Muzzle flash, gun smoke and the shotgun's pellet spray, as `EV_TS_FireStandard`/`FireAkimbo`
/// (`pump` false) and `EV_TS_FirePump` (`pump` true) call them. The smoke and flash carry the
/// shooter's velocity.
fn gun_fx(a: &Args, s: &Shot, at: &FxAt, third: bool, pump: bool, pellets: i32) {
    let now = effects::now();
    let (fwd, vel, info) = (s.fwd, a.velocity, s.flash());
    effects::with_psys(|ps| {
        use effects::fx;
        for &muzzle in at.flashes {
            fx::muzzle_flash(ps, now, muzzle, vel, fwd, &info, 9, s.silenced && !pump, if s.silenced && !pump { 8.0 } else { 0.0 }, third);
        }
        let muzzle = at.smoke;
        if pump {
            fx::gun_smoke(ps, now, if third { 24.0 } else { 16.0 }, r::random_long(5, 10), muzzle, vel, fwd, 24.0, false);
        } else if !s.silenced {
            let rate = if s.id == 33 { r::random_long(5, 10) } else { r::random_long(45, 55) };
            fx::gun_smoke(ps, now, 16.0, rate, muzzle, vel, fwd, 24.0, false);
        } else {
            fx::gun_smoke(ps, now, 8.0, r::random_long(50, 60), muzzle, vel, fwd, 24.0, false);
            // The wisp sits back along the barrel: `muzzle - fwd·ofs` (`Vector::operator-`
            // 0x1003d200 at 0x1003e54b, 0x1003ecb3, 0x1003f6ec and 0x100406ac).
            let at = v::ma(muzzle, -s.muzzle_ofs(), fwd);
            fx::gun_smoke(ps, now, 1.0, r::random_long(50, 80), at, vel, s.right, 4.0, true);
        }
        if let Some(spray) = at.spray
            && (pump || pellets > 1)
        {
            let n = v::scale(fwd, 3.0);
            fx::sprite_debris(
                ps,
                now,
                spray,
                n,
                "sprites/shotgun_pellets.spr",
                kRenderTransAdd as i32,
                pellets,
                0.5,
                7,
                0.0,
                0.0,
                true,
                false,
            );
        }
    });
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

/// Effects at the shooter's cached player attachments ([`effects::player_attachment`]; third
/// person and other players): flashes at `flash_atts`, the smoke at `smoke_att`, the pellet spray
/// at `spray_att`, and a shell behind each flash.
fn world_effects(a: &Args, s: &Shot, flash_atts: &[usize], smoke_att: usize, spray_att: Option<usize>, pump: bool, pellets: i32) {
    let att = |n: usize| effects::player_attachment(a.entindex, n);
    let flashes: Vec<Vec3> = flash_atts.iter().map(|&n| att(n)).collect();
    gun_fx(a, s, &FxAt { flashes: &flashes, smoke: att(smoke_att), spray: spray_att.map(att) }, true, pump, pellets);
    for &muzzle in &flashes {
        eject_shell(v::ma(muzzle, -s.muzzle_ofs(), s.fwd), s.shell_velocity(a), a.angles, s.shell, s.shell_snd);
    }
}

/// First-person effects. As TS, the flashes and the pellet spray come from the shooter's cached
/// player attachments (`0x1003e39e`, `0x1003e5c4`, `0x1003f4b8`, `0x1003f579`, `0x10040fcd`,
/// `0x10041091`), while the smoke and each shell (behind its flash's gun) come from the view
/// model's (`0x1003e408`, `0x1003e7cc`, `0x1003f5e6`, `0x1003f8ff`, `0x1003fb1b`, `0x10041017`,
/// `0x10041286`). The spray, when there is one, is at the first flash's attachment.
///
/// For the local player the cache holds the view model's muzzle: both engines run the view
/// model's `STUDIO_EVENTS` pass before the transparent pass that fills the cache (Steam
/// `R_PreDrawViewModel`, FWGS `R_RunViewmodelEvents`), and the studio renderer copies its
/// attachments into the entity of the view model's `index`, the local player's
/// ([`crate::studio`]'s `events`). Spectating in-eye (port deviation), the cache holds the
/// target's origin: the engines reset a player's attachments to its origin every update and
/// never draw the in-eye target, so TS's flash starts at the target's hull centre, about 28 units
/// below the eye. The port takes the view model's attachments there, while the view model's
/// events run (`r_drawviewmodel`).
fn view_model_effects(a: &Args, s: &Shot, flash_atts: &[usize], smoke_att: usize, spray: bool, pump: bool, pellets: i32) {
    let Some(vm) = first_person_view_model() else {
        // No view model (scoped): shell from the view origin.
        eject_shell(view::origin(), s.shell_velocity(a), a.angles, s.shell, s.shell_snd);
        return;
    };
    let att = |n: usize| vm.attachment.get(n).copied().unwrap_or(vm.origin);
    let in_eye_vm = !a.is_local() && engine().cvar_float(c"r_drawviewmodel") != 0.0;
    let flash_at = |n: usize| if in_eye_vm { att(n) } else { effects::player_attachment(a.entindex, n) };
    let flashes: Vec<Vec3> = flash_atts.iter().map(|&n| flash_at(n)).collect();
    if let Some(&flash) = flashes.first() {
        log_flash(flash, att(flash_atts[0]), s.fwd);
    }
    let at = FxAt { flashes: &flashes, smoke: att(smoke_att), spray: spray.then(|| flashes[0]) };
    gun_fx(a, s, &at, false, pump, pellets);
    for &n in flash_atts {
        eject_shell(v::ma(att(n), -s.muzzle_ofs(), s.fwd), s.shell_velocity(a), a.angles, s.shell, s.shell_snd);
    }
}

/// `ts_dev_fxlog` line for a first-person flash: where it sits relative to the view (units ahead
/// along the view axis and off it), how far it is from the view model's muzzle, and how well the
/// direction the flash sprites spread along (the event's angles) matches the view's.
fn log_flash(flash: Vec3, muzzle: Vec3, fwd: Vec3) {
    super::fx_log(|| {
        let cam = view::camera();
        let d = v::sub(flash, cam.origin);
        let ahead = v::dot(d, cam.forward);
        let off = v::len(v::ma(d, -ahead, cam.forward));
        let (to_muzzle, dir) = (v::dist(flash, muzzle), v::dot(fwd, cam.forward));
        format!("flash at {flash:.1?} ahead {ahead:.1} off-axis {off:.1} muzzle-distance {to_muzzle:.1} direction-dot {dir:.3}")
    });
}

/// Which branch of a fire event runs (`0x1003e2b6`/`0x1003e32f`/`0x1003e9ee..0x1003ea1e` in
/// `EV_TS_FireStandard`, the same in the akimbo and pump handlers): the view owner (the local
/// player, or the one spectated in-eye) gets the view animation and, outside third person, the
/// first-person effects; everyone else, and the owner in third person, gets the player-model
/// effects if `HUD_AddEntity` saw them this frame.
fn branches(a: &Args) -> (bool, bool, bool) {
    let owner = a.is_view_owner();
    let third = view::is_third_person();
    (owner, owner && !third, (third || !owner) && effects::player_visible(a.entindex))
}

/// The muzzle dlight: unsilenced shots from a visible player or the view owner (`0x1003e21c..`).
fn muzzle_light(a: &Args, s: &Shot) {
    if s.silenced || !(effects::player_visible(a.entindex) || a.is_view_owner()) {
        return;
    }
    let pos = v::ma(v::ma(a.origin, 16.0, s.fwd), 32.0, s.up);
    let now = effects::now();
    effects::with_psys(|ps| effects::fx::muzzle_dlight(ps, now, pos));
}

/// The weapon fire sound (`0x1003de90`): the ruleset's fire pitch
/// ([`ClientRules::fire_pitch`](crate::gameplay::ClientRules::fire_pitch), 3.0's is
/// [`super::fire_pitch`]) `+ rand(-5, 5)`, rolled before the volume.
/// The silenced flag alone picks the sample (`0x1003de98`): `sound2` (`info+0x306`, volume
/// 0.84-0.92, attenuation x2) or `sound` (`info+0x2c6`, `0x1003dee5`), even when that one is empty;
/// TS then plays its empty buffer, and so does the port.
fn fire_sound(a: &Args, id: i32, silenced: bool) {
    let Some(asset) = tables().asset(id as u8) else { return };
    let ammo = tables().weapon(id as u8).map_or(0, |w| w.ammo) as usize;
    let atten = AMMO_ATTENUATION.get(ammo).copied().unwrap_or(0.4);
    let roll = r::random_long(-5, 5);
    let pitch = roll + super::weapon_fire_pitch();
    let (sample, vol, attn) = fire_sample(&asset.sound, &asset.sound2, silenced);
    let vol = r::random_float(vol.0, vol.1);
    engine().ev_play_sound(a.entindex, a.origin, CHAN_WEAPON as i32, &sample, vol, attn * atten, 0, pitch);
}

/// The fire sound's sample path, volume range and attenuation factor. TS's info buffers hold
/// `weapons/...` paths or nothing; the table holds them without the prefix.
fn fire_sample(sound: &str, sound2: &str, silenced: bool) -> (String, (f32, f32), f32) {
    let path = |s: &str| if s.is_empty() { String::new() } else { format!("weapons/{s}") };
    if silenced { (path(sound2), (0.84, 0.92), 2.0) } else { (path(sound), (0.92, 1.0), 1.0) }
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
    let (owner, first_person, world) = branches(a);
    if owner {
        // Last shot `[6]` (`0x1003e2fa`), else `[3 + RandomLong(0, 2)]` (`0x1003e313`).
        let set = anim_set(s.id);
        let seq = if a.b1 != 0 { set[6] } else { set[3 + r::random_long(0, 2) as usize] };
        r::weapon_animation(seq as i32, weapon_body(s.id, a.i2));
    }
    if first_person {
        view_model_effects(a, &s, &[0], 0, true, false, pellets);
    }
    if world {
        // Rifles and snipers flash from attachment 3, the rest from 0 (0x1003ed4e); the pellet
        // spray always comes from attachment 3 (0x1003ed21).
        let att = if matches!(info::class(s.id), 6 | 7) { 3 } else { 0 };
        world_effects(a, &s, &[att], att, Some(3), false, pellets);
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
    let (owner, first_person, world) = branches(a);
    if owner {
        // Pump mode `[12]` (`0x10040f4b`), semi `[11]` (`0x10040f57`).
        let set = anim_set(s.id);
        r::weapon_animation(set[if a.b1 != 0 { 12 } else { 11 }] as i32, weapon_body(s.id, a.i2));
    }
    let shell = if a.b1 != 0 { Shot { shell: 0, ..s } } else { s };
    if first_person {
        view_model_effects(a, &shell, &[0], 0, true, true, pellets);
    }
    if world {
        world_effects(a, &shell, &[3], 3, Some(3), true, pellets);
    }
    fire_sound(a, shell.id, false);
    bullets_for(a, &shell, a.gun_position(), a.b2, a.f1, a.f2);
}

/// `EV_TS_FireAkimbo` (`0x1003f090`): `b1` selects the gun (attachment 0 when set, 2 when not:
/// `0x1003f55c..0x1003f576`, `0x1004050e`); linked fires both, offset +-5 along right, the second
/// batch with `seed + 0x21`. A linked shot flashes at attachments 0 and 2 (`0x1003f4b8`,
/// `0x1003f579`) but smokes once, at the `b1` gun (`0x1003f5cc..0x1003f70b`). Akimbo has no
/// pellet spray.
fn fire_akimbo(a: &Args, linked: bool) {
    super::fx_log(|| {
        format!(
            "akimbo shot #{} weapon {} {}",
            a.entindex,
            a.i1,
            if linked {
                "linked"
            } else if a.b1 != 0 {
                "gun b1"
            } else {
                "gun b0"
            }
        )
    });
    let s = Shot::new(a);
    let pellets = tables().weapon(s.id as u8).map_or(1, |w| w.pellets);
    muzzle_light(a, &s);
    let sel = if a.b1 != 0 { 0 } else { 2 };
    let hands: &[usize] = if linked { &[0, 2] } else { &[sel] };
    let (owner, first_person, world) = branches(a);
    if owner {
        // Linked `[3 + RandomLong(0, 2)]` (`0x1003f3e4`), `b1` gun `[22 + RandomLong(0, 1)]`
        // (`0x1003f406`), the other `[24 + RandomLong(0, 1)]` (`0x1003f41c`).
        let set = anim_set(s.id);
        let seq = if linked {
            set[3 + r::random_long(0, 2) as usize]
        } else if a.b1 != 0 {
            set[22 + r::random_long(0, 1) as usize]
        } else {
            set[24 + r::random_long(0, 1) as usize]
        };
        r::weapon_animation(seq as i32, weapon_body(s.id, a.i2));
    }
    if first_person {
        view_model_effects(a, &s, hands, sel, false, false, pellets);
    }
    if world {
        world_effects(a, &s, hands, sel, None, false, pellets);
    }
    fire_sound(a, s.id, s.silenced);
    let src = a.gun_position();
    // The guns' offsets along `v_right` are the ruleset's (TS 3.0: -5 and +5 linked, none for one).
    let (first, second) = crate::gameplay::hud_gun_rules().akimbo_event_offsets(linked, a.b1 != 0);
    let at = |off: f32| if off == 0.0 { src } else { v::ma(src, off, s.right) };
    bullets_for(a, &s, at(first), a.b2, a.f1, a.f2);
    if let Some(off) = second {
        bullets_for(a, &s, at(off), a.b2.wrapping_add(0x21), a.f1, a.f2);
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

/// Pellet `i` direction (1-based) of TS 3.0's pattern, as the client fire event computes it.
pub fn pellet_dir(dir: Vec3, right: Vec3, up: Vec3, spread: [f32; 2], seed: i32, i: i32) -> Vec3 {
    spread::pellet_dir(PelletSide::Client, dir, right, up, spread, pellet_offset(seed, i))
}

/// TS 3.0's pellet offset, in units of the spread, as `client.dll` computes it
/// (`EV_TS_FireBullets` 0x100458f0..0x1004596e): the angle stored as a `float`, `y` unrounded.
fn pellet_offset(seed: i32, i: i32) -> PelletOffset {
    let s = (seed as u32).wrapping_add(i as u32);
    let ang = shared_random_float(s, -std::f32::consts::PI, std::f32::consts::PI);
    let r = shared_random_float(s.wrapping_add(1), 0.0, SPREAD_RADIUS);
    spread::polar_offset(ang, r)
}

/// Whizz-by sound (`EV_TS_FireBullets` `0x100453b5`): another player's shot whose line (to the
/// end of a 4096-unit trace, longer than 200 units) passes within 75 units of the view, measured
/// with TS's distance ([`heron_height`]), and only while the view is inside the sphere
/// `|mid − view| < |mid − end| − 100`. Volume `min(1, 2/(h + 0.1))`, channels 503/504 alternating.
fn whizz(b: &Bullets, end: Vec3) {
    if super::is_local_entity(b.shooter) {
        return;
    }
    let len = v::dist(b.src, end);
    if len <= 200.0 {
        return;
    }
    let eye = view::origin();
    let mid = v::scale(v::add(b.src, end), 0.5);
    if v::dist(mid, eye) >= v::dist(mid, end) - 100.0 {
        return;
    }
    let h = heron_height(v::dist(eye, b.src), v::dist(eye, end), len);
    if h < 75.0 {
        let chan = effects::next_whizz_channel();
        let snd = format!("weapons/whizz{}.wav", r::random_long(1, 5));
        engine().ev_play_sound(-1, eye, chan, &snd, (2.0 / (h + 0.1)).min(1.0), 0.8, 0, slow_pitch());
    }
}

/// `EV_TS_FireBullets` (`0x100450c0`): mirrors the server's `ShootBulletsPlayer` so decals land
/// where the server's bullets hit. With the server's `realbullet` on (`SrvSett` 0, `0x1018d16c`)
/// or in slow motion (`0x10045330`) the pellets are queued as client bullets instead of traced.
pub fn fire_bullets(b: &Bullets) {
    let (pierce, unit, _) = ammo_consts(b.ammo);
    let (ts, factor100) = state::with(|s| (s.time_scale, s.slowmo_factor100));
    let slow = crate::gameplay::current().client_bullets_queue(crate::hud::with(|h| h.realbullet), ts, factor100);

    if b.bullet_type == 2 {
        let end = v::ma(b.src, 4096.0, b.dir);
        r::with_solid_players(b.shooter.wrapping_sub(1), || {
            let mut tr = r::player_trace(b.src, end, 2, PM_STUDIO_BOX as i32, -1);
            if trace_hit(tr.fraction) {
                impact::texture_sound(&mut tr, b.src, end, 2);
                impact::decal_gunshot(&mut tr);
            }
        });
        return;
    }

    let end = r::player_trace(b.src, v::ma(b.src, 4096.0, b.dir), 2, PM_STUDIO_BOX as i32, -1).endpos;
    whizz(b, end);
    // Particle push: ×8 for buckshot (ammo 3), with a swirl for ammo 5, 8 and 13.
    let push = if b.ammo == 3 { b.dmg * 8.0 } else { b.dmg };
    let swirl = matches!(b.ammo, 5 | 8 | 13);
    effects::with_psys(|ps| {
        ps.push_radial(b.src, 512.0, push);
        ps.push_along_line(b.src, end, push, swirl);
    });

    // TS's shared trace (`0x1018a110`) last ended at the whizz trace's end; each pellet trace
    // moves it on, hit or miss.
    let mut last_end = end;
    let mut last_dir = b.dir;
    // The pellet pattern is the ruleset's ([`GunRules::pellet_offset`]).
    let rules = crate::gameplay::hud_gun_rules();
    for i in 1..=b.pellets.max(1) {
        let offset = rules.pellet_offset(b.seed as u32, i as u32, None).map_or_else(|| pellet_offset(b.seed, i), PelletOffset::from);
        let d = rules.pellet_dir(PelletSide::Client, b.dir, b.right, b.up, b.spread, offset);
        last_dir = d;
        let end = v::ma(b.src, MAX_RANGE, d);
        if slow {
            // `ClientBullets_Add` (0x10045a8b) with the pellet direction as is, not normalised.
            effects::add_client_bullet(b.shooter, b.src, d, b.dmg, b.ammo, b.bullet_type);
            continue;
        }
        r::with_solid_players(b.shooter.wrapping_sub(1), || {
            let mut tr = r::player_trace(b.src, end, 2, PM_STUDIO_BOX as i32, -1);
            last_end = tr.endpos;
            super::fx_log(|| format!("bullet from {:.1?} to {:.1?}", b.src, tr.endpos));
            if trace_hit(tr.fraction) {
                let remaining = impact::range_mult(range_index(v::dist(b.src, tr.endpos), unit)) * b.dmg;
                let bt = if b.bullet_type == 1 { 1 } else { b.bullet_type };
                impact::texture_sound(&mut tr, b.src, end, bt);
                impact::decal_gunshot(&mut tr);
                REMAINING.0.set(remaining * pierce);
            }
        });
    }

    // Penetration (bullet type 0 only on the client: TS skips buckshot) from wherever the last
    // pellet's trace ended, with whatever damage the last hit, in this event or an earlier one,
    // left.
    let remaining = REMAINING.0.get();
    if slow || b.bullet_type == 1 || !penetrates(remaining) {
        return;
    }
    let step = remaining.min(48.0);
    let p = v::ma(last_end, step, last_dir);
    if r::point_contents(p) == CONTENTS_SOLID {
        return;
    }
    let hit = last_end;
    let mut tr = r::player_trace(p, hit, 2, 0, -1);
    if trace_hit(tr.fraction) {
        impact::texture_sound(&mut tr, p, hit, 2);
        impact::decal_gunshot(&mut tr);
    }
    fire_bullets(&Bullets { pellets: 1, src: p, dir: last_dir, spread: [0.0; 2], dmg: remaining, bullet_type: 2, ..*b });
}

/// `DAT_1018a164`: the damage the last pellet hit left for penetration. Only a pellet that hits
/// writes it and nothing resets it, so a later event that misses (or only queues client bullets)
/// penetrates with an earlier event's damage, as in TS.
static REMAINING: Global<Cell<f32>> = Global(Cell::new(0.0));

/// A trace hit something: TS tests `fraction == 1.0` with `fcomp`/`test ah, 0x40`, so a NaN
/// counts as a miss (`0x1004527b`, `0x10045aec`, `0x10045d0f`).
fn trace_hit(fraction: f32) -> bool {
    !(fraction == 1.0 || fraction.is_nan())
}

/// The penetration test, `remaining > 5.0` (`0x10045bff`: `test ah, 0x41`, so a NaN fails).
fn penetrates(remaining: f32) -> bool {
    remaining > 5.0
}

/// `FUN_100071f0(ammo)`: the ammo table's (pierce `+0x18`, range unit `+0x20`, velocity in metres
/// per second `+0x24`). An ammo index outside the table reads (0, 80, 330) here; TS indexes past
/// its table.
fn ammo_consts(ammo: i32) -> (f32, f32, f32) {
    u8::try_from(ammo).ok().and_then(|a| tables().ammo(a)).map_or((0.0, 80.0, 330.0), |a| (a.pierce, a.range_unit, a.velocity))
}

/// The range-falloff index of a hit `dist` units away with range unit `unit`:
/// `__ftol(dist / (unit·10) · 10)`, truncating (`0x10045b2b`, `0x10035f94`). TS keeps the x87
/// intermediates in extended precision, approximated here in f64.
pub(crate) fn range_index(dist: f32, unit: f32) -> i32 {
    ftol(dist as f64 / (unit as f64 * 10.0) * 10.0)
}

/// MSVC's `__ftol` (`0x1006ad1c`): truncate to 64 bits, keep the low 32.
pub fn ftol(x: f64) -> i32 {
    x as i64 as i32
}

/// Client bullet slots (`0x100eb678`: 256 slots of `0x5c` bytes).
pub const CLIENT_BULLETS: usize = 256;

/// One client bullet slot. Each lives two lives: the traced bullet ([`ClientBullet::pos`]),
/// stepped and traced by `ClientBullets_Impact` (`0x10035d40`) on a throttle, and the drawn one
/// ([`ClientBullet::draw_pos`]), moved by `ClientBullets_Update` (`0x10035010`) every frame it is
/// in the view cone. They part ways: only the traced one hits, penetrates or is cleared.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ClientBullet {
    /// `+0x00`: the shooter's entity index; 0 marks a free slot.
    pub shooter: i32,
    /// `+0x04`: the traced position.
    pub pos: Vec3,
    /// `+0x10`: where the current leg started (the fire point, or the exit point after a
    /// penetration); the range falloff is measured from here.
    pub start: Vec3,
    /// `+0x1c`: the pellet direction, not normalised.
    pub dir: Vec3,
    /// `+0x28`: the drawn position.
    pub draw_pos: Vec3,
    /// `+0x34`: where the last trail segment ended.
    pub trail: Vec3,
    /// `+0x40`.
    pub dmg: f32,
    /// `+0x44`: ammo index.
    pub ammo: i32,
    /// `+0x4c`: client time the next trail segment may start.
    pub next_trail: f32,
    /// `+0x50`: client time of the next trace step.
    pub next_update: f32,
    /// `+0x54`: 0 normal, 1 buckshot, 2 exit (after a penetration).
    pub bullet_type: i32,
    /// `+0x58`: a ruleset's trail spin, in degrees (unused in TS 3.0).
    pub spin: f32,
}

/// The client bullet slots and their counter (`+0x5c00`, `0x100f1278`): every bullet added since
/// the last frame plus those the last frame found in use. `TS_DrawEffects` runs the frame only
/// while it is non-zero, so the frame after the last bullet is gone still runs once.
#[derive(Default)]
pub struct ClientBullets {
    /// Empty until the first [`ClientBullets::add`] or [`ClientBullets::reset`], then 256 slots.
    slots: Vec<ClientBullet>,
    pub count: i32,
    /// The models a ruleset's draw queued this frame (none in TS 3.0).
    pub models: BulletModels,
}

/// A studio model a ruleset's client-bullet draw queues ([`ClientRules::client_bullet_draw`]),
/// drawn by the next `HUD_CreateEntities` while bullets are in flight.
///
/// [`ClientRules::client_bullet_draw`]: crate::gameplay::ClientRules::client_bullet_draw
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BulletModel {
    pub name: &'static str,
    pub origin: Vec3,
    /// The entity's `curstate.angles`.
    pub angles: Vec3,
    pub body: i32,
}

/// The client-loaded bullet models (`CL_LoadModel`, once per name until the next `VidInit`) and
/// this frame's queue.
#[derive(Default)]
pub struct BulletModels {
    cache: Vec<(&'static str, Option<goldsrc::ext::models::LoadedModel>)>,
    queued: Vec<BulletModel>,
}

impl BulletModels {
    fn model(&mut self, name: &'static str) -> Option<goldsrc::ext::models::LoadedModel> {
        if let Some((_, m)) = self.cache.iter().find(|(n, _)| *n == name) {
            return *m;
        }
        let m = goldsrc::ext::models::load_model(name);
        self.cache.push((name, m));
        m
    }

    /// Whether every one of `names` loads (each is loaded once, all of them even when one fails).
    pub fn loaded(&mut self, names: &[&'static str]) -> bool {
        let mut ok = true;
        for n in names {
            ok &= self.model(n).is_some();
        }
        ok
    }

    /// Queues `m` for the next `HUD_CreateEntities`.
    pub fn queue(&mut self, m: BulletModel) {
        self.queued.push(m);
    }

    /// The models queued since the last bullet frame.
    pub fn queued(&self) -> &[BulletModel] {
        &self.queued
    }

    /// Adds the queued models to the frame's visible entities (`HUD_CreateEntities`).
    pub fn add_entities(&mut self) {
        for i in 0..self.queued.len().min(goldsrc::ext::models::MODEL_SLOTS) {
            let q = self.queued[i];
            if let Some(model) = self.model(q.name) {
                let e = goldsrc::ext::models::ModelEntity { model, origin: q.origin, angles: q.angles, body: q.body };
                goldsrc::ext::models::add_model_entity(i, &e);
            }
        }
    }

    /// Forgets the loaded models (`VidInit`: a new level may free them).
    pub fn forget(&mut self) {
        *self = BulletModels::default();
    }
}

impl ClientBullets {
    /// `ClientBullets_Reset` (`0x10034f60`): every slot free, the counter 0.
    pub fn reset(&mut self) {
        self.slots.clear();
        self.slots.resize(CLIENT_BULLETS, ClientBullet::default());
        self.count = 0;
    }

    /// `ClientBullets_Add` (`0x10035c20`): the first free slot. With all 256 in use the new
    /// bullet is dropped.
    pub fn add(&mut self, shooter: i32, src: Vec3, dir: Vec3, dmg: f32, ammo: i32, bullet_type: i32) {
        if self.slots.is_empty() {
            self.reset();
        }
        let Some(s) = self.slots.iter_mut().find(|s| s.shooter == 0) else { return };
        *s = ClientBullet { shooter, pos: src, start: src, dir, draw_pos: src, trail: src, dmg, ammo, bullet_type, ..Default::default() };
        self.count += 1;
    }

    /// The slots (empty before the first add or reset).
    pub fn slots(&self) -> &[ClientBullet] {
        &self.slots
    }
}

/// What a client bullet frame reads from the rest of the client, gathered once by
/// `TS_DrawEffects`.
pub struct BulletFrame {
    /// The view of this transparent pass (TS's `AngleVectors` at `0x10056f60` and the refdef
    /// origin `0x10190b28`).
    pub cam: Camera,
    /// `GetCullOrigin` (`0x10057270`).
    pub cull: Vec3,
    /// TS's copy of `ref_params_t::frametime` (`0x10191478`).
    pub frametime: f32,
}

/// `GetBulletVelocityMult(ts, bt)` (`0x10055b30`).
pub fn bullet_velocity_mult(ts: f32, bullet_time: bool) -> f32 {
    if ts >= 0.99 {
        return 1.0;
    }
    ts / ((1.0 - ts * ts) * if bullet_time { 10.0 } else { 1.0 }).max(1.0)
}

/// When a slot's next trace step is due, set when one runs at client time `now` (`0x10035b97`):
/// every 0.1 s with `realbullet` 0, every 0.05 s with 1, otherwise every frame.
pub(crate) fn next_update(now: f32, realbullet: i32) -> f32 {
    if (0..2).contains(&realbullet) { 0.1 / (realbullet + 1) as f32 + now } else { now }
}

/// The time a trace step covers (`0x10035d82..0x10035da2`): up to the next step, or the frame
/// time when that is under a millisecond away (every-frame stepping).
pub fn step_time(next_update: f32, now: f32, frametime: f32) -> f32 {
    let dt = next_update - now;
    if dt < 0.001 { frametime } else { dt }
}

/// `ClientBullets_Frame` (`0x10035b50`), at the end of `TS_DrawEffects` (`0x10034e60`): looks up
/// `sprites/shotgun_pellets.spr` (additive), then for each slot in use steps its trace when due
/// and, in slow motion (ts < 0.99), draws it, even when the step just cleared it.
///
/// A ruleset can change the step period ([`ClientRules::client_bullet_next_update`]), when it
/// draws ([`ClientRules::client_bullet_draws`]) and the draw itself
/// ([`ClientRules::client_bullet_draw`]).
///
/// [`ClientRules::client_bullet_next_update`]: crate::gameplay::ClientRules::client_bullet_next_update
/// [`ClientRules::client_bullet_draws`]: crate::gameplay::ClientRules::client_bullet_draws
/// [`ClientRules::client_bullet_draw`]: crate::gameplay::ClientRules::client_bullet_draw
pub fn client_bullets_frame(bullets: &mut ClientBullets, f: &BulletFrame) {
    let rules = crate::gameplay::current();
    let sprite = effects::with_psys(|ps| ps.sprite("sprites/shotgun_pellets.spr", kRenderTransAdd as i32));
    bullets.count = 0;
    bullets.models.queued.clear();
    for i in 0..bullets.slots.len() {
        if bullets.slots[i].shooter == 0 {
            continue;
        }
        let now = engine().client_time();
        if now > bullets.slots[i].next_update {
            bullets.slots[i].next_update = rules.client_bullet_next_update(now, crate::hud::with(|h| h.realbullet));
            client_bullet_impact(&mut bullets.slots[i], f.frametime, rules);
        }
        let (ts, factor100, bullet_time, fov) = state::with(|s| (s.time_scale, s.slowmo_factor100, s.bullet_time, s.fov));
        if rules.client_bullet_draws(ts, factor100) {
            let s = &mut bullets.slots[i];
            let mut d = BulletDraw {
                frame: f,
                now: engine().client_time(),
                velocity: ammo_consts(s.ammo).2,
                slowmo_factor100: factor100,
                bullet_time,
                wavedetail: crate::input::cvars().cl_wavedetail.value(),
                fov,
                models: &mut bullets.models,
            };
            if !rules.client_bullet_draw(s, &mut d) {
                client_bullet_draw(s, sprite, f);
            }
        }
        bullets.count += 1;
    }
}

/// What a ruleset's client-bullet draw ([`ClientRules::client_bullet_draw`]) reads.
///
/// [`ClientRules::client_bullet_draw`]: crate::gameplay::ClientRules::client_bullet_draw
pub struct BulletDraw<'a> {
    pub frame: &'a BulletFrame,
    /// The engine's client time.
    pub now: f32,
    /// The bullet's ammo velocity (the ammo table's `+0x24`).
    pub velocity: f32,
    /// The last `TSSlowMo` value as sent (the factor × 100; 0 = normal speed).
    pub slowmo_factor100: f32,
    /// `TSBTime`: bullet time is on.
    pub bullet_time: bool,
    /// `cl_wavedetail`.
    pub wavedetail: f32,
    /// `m_iFOV`.
    pub fov: f32,
    pub models: &'a mut BulletModels,
}

/// What a client bullet's trace step reads ([`ClientRules::client_bullet_step`]).
///
/// [`ClientRules::client_bullet_step`]: crate::gameplay::ClientRules::client_bullet_step
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BulletStep {
    /// The bullet's ammo velocity (the ammo table's `+0x24`).
    pub velocity: f32,
    /// The eased time scale.
    pub time_scale: f32,
    /// `TSBTime`: bullet time is on.
    pub bullet_time: bool,
    /// The last `TSSlowMo` value as sent (the factor × 100; 0 = normal speed).
    pub slowmo_factor100: f32,
    /// The time this step covers ([`step_time`]).
    pub dt: f32,
}

/// `ClientBullets_Impact` (`0x10035d40`): one trace step of `dir·(velocity·40)·dt·mult`
/// (`GetBulletVelocityMult`), with players solid except the shooter. A miss moves on. A hit runs
/// the impact code (from the hit point, toward the hit point plus the step) and decal, and clears
/// the slot, unless a normal bullet (type 0) has more than 5 damage left after the range falloff
/// and the ammo's pierce factor: then, as `EV_TS_FireBullets`, it steps `min(left, 48)` units on,
/// and unless that is inside a solid it carries on from there as an exit bullet (type 2) with the
/// damage left, after the exit point's impact (traced back to the hit).
fn client_bullet_impact(s: &mut ClientBullet, frametime: f32, rules: &dyn crate::gameplay::ClientRules) {
    let (pierce, unit, velocity) = ammo_consts(s.ammo);
    let (ts, bt, factor100) = state::with(|st| (st.time_scale, st.bullet_time, st.slowmo_factor100));
    let dt = step_time(s.next_update, engine().client_time(), frametime);
    let input = BulletStep { velocity, time_scale: ts, bullet_time: bt, slowmo_factor100: factor100, dt };
    let step = rules.client_bullet_step(s.dir, &input);
    let clear = r::with_solid_players(s.shooter.wrapping_sub(1), || {
        let end = v::add(step, s.pos);
        let mut tr = r::player_trace(s.pos, end, 2, PM_STUDIO_BOX as i32, -1);
        if tr.fraction == 1.0 {
            s.pos = end;
            return false;
        }
        let hit = tr.endpos;
        s.pos = hit;
        let d = v::sub(s.start, s.pos);
        let dist = ((d[2] as f64 * d[2] as f64 + d[1] as f64 * d[1] as f64) + d[0] as f64 * d[0] as f64).sqrt();
        let idx = rules.client_bullet_range_index(dist, unit, s.bullet_type == 2);
        let mut left = impact::range_mult(idx) * s.dmg;
        // TS passes its shooter index first, which `TS_PlayTextureSound` never reads.
        impact::texture_sound(&mut tr, s.pos, v::add(step, s.pos), s.bullet_type);
        impact::decal_gunshot(&mut tr);
        left *= pierce;
        if s.bullet_type == 2 || s.bullet_type == 1 || left <= 5.0 {
            return true;
        }
        let ahead = if left < 48.0 { left } else { 48.0 };
        let p = v::add(s.dir.map(|d| d * ahead), hit);
        if r::point_contents(p) == CONTENTS_SOLID {
            return true;
        }
        s.pos = p;
        s.start = p;
        s.dmg = left;
        s.bullet_type = 2;
        let mut tr = r::player_trace(p, hit, 2, 0, -1);
        if tr.fraction != 1.0 {
            impact::texture_sound(&mut tr, p, hit, 2);
            impact::decal_gunshot(&mut tr);
        }
        false
    });
    if clear {
        // `ClientBullets_Clear` (`0x10035d10`).
        *s = ClientBullet::default();
    }
}

/// Width of a drawn bullet and its trail: `max(1 − ts², 0.15)` (`0x1003503f`).
pub fn bullet_width(ts: f32) -> f32 {
    let w = 1.0 - ts as f64 * ts as f64;
    if 0.15f32 as f64 > w { 0.15 } else { w as f32 }
}

/// Half-length of a drawn bullet's streaks: `ts·37.5` clamped to 5..25 (`0x100352e0`).
pub fn streak_length(ts: f32) -> f32 {
    (ts as f64 * 37.5).clamp(5.0, 25.0) as f32
}

/// A drawn bullet's alpha, also its trail's: faded right at the view, `d²·0.1` when the squared
/// distance `d²` from the view origin is under 10 (`0x10035070..0x100350c9`).
pub fn near_alpha(view: Vec3, pos: Vec3) -> f32 {
    let dx = (view[0] - pos[0]) as f64;
    let dy = (view[1] - pos[1]) as f64;
    // TS multiplies the unrounded z difference by its stored copy.
    let dz = view[2] as f64 - pos[2] as f64;
    let d2 = dz * (dz as f32) as f64 + dy * dy + dx * dx;
    if d2 < 10.0 { (d2 * 0.1f32 as f64) as f32 } else { 1.0 }
}

/// A drawn bullet's quads (uv (0,0) (1,0) (1,1) (0,1) each): unless it's buckshot (ammo 3), two
/// streaks from `pos + dir·len` (head) to `pos − dir·len` (tail), `w` either side across the
/// view's up and then its right (`0x10035335..0x1003599e`); then a `w`-radius billboard at `pos`
/// (`Quad_Build` `0x10050080` as a screen sprite, `0x100359be`).
pub fn bullet_quads(pos: Vec3, dir: Vec3, buckshot: bool, len: f32, w: f32, up: Vec3, right: Vec3) -> Vec<[Vec3; 4]> {
    let mut quads = Vec::with_capacity(3);
    if !buckshot {
        let along = dir.map(|d| len * d);
        let head = v::add(along, pos);
        let tail = v::sub(pos, along);
        for axis in [up, right] {
            let a = axis.map(|x| x * w);
            quads.push([v::sub(head, a), v::add(a, head), v::add(a, tail), v::sub(tail, a)]);
        }
    }
    let u = up.map(|x| x * w);
    let r = right.map(|x| x * w);
    let top = v::add(u, pos);
    let bottom = v::sub(pos, u);
    quads.push([v::sub(top, r), v::add(top, r), v::add(bottom, r), v::sub(bottom, r)]);
    quads
}

/// `ClientBullets_Update` (`0x10035010`): nothing at all outside the view cone (`InViewCone` of
/// the drawn position). Otherwise the bullet's quads with the pellets sprite, then the drawn
/// position moves `dir·velocity·40·mult·frametime`, and with `cl_wavedetail` > 0 a
/// `bullet_trail.spr` segment (`TS_BulletTrail` `0x10046240`) runs from the last trail point to
/// it, at most every `SlowTime(0.01)` s. The bullet-model path (`0x100350df`) is dead in TS
/// (`0x1d34d` is only ever cleared, by `0x10034f90` from `HUD_CreateEntities`).
fn client_bullet_draw(s: &mut ClientBullet, sprite: Option<i16>, f: &BulletFrame) {
    if !psys::in_view_cone(&f.cam, f.cull, s.draw_pos) {
        return;
    }
    let (ts, bt) = state::with(|st| (st.time_scale, st.bullet_time));
    let w = bullet_width(ts);
    let alpha = near_alpha(f.cam.origin, s.draw_pos);
    if let Some(sprite) = sprite {
        let model = r::sprite_model(effects::with_psys(|ps| ps.sprite_handle(sprite)));
        if !model.is_null() && tri::sprite_texture(model, 0) {
            tri::render_mode(kRenderTransAdd as i32);
            tri::cull_face(TRICULLSTYLE_TRI_NONE);
            tri::begin(TRI_QUADS as i32);
            tri::color4f(1.0, 1.0, 1.0, alpha);
            for q in bullet_quads(s.draw_pos, s.dir, s.ammo == 3, streak_length(ts), w, f.cam.up, f.cam.right) {
                for (uv, vtx) in [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]].iter().zip(q.iter()) {
                    tri::tex_coord2f(uv[0], uv[1]);
                    tri::vertex3fv(vtx);
                }
            }
            tri::end();
            tri::render_mode(kRenderNormal as i32);
        }
    }
    let (_, _, velocity) = ammo_consts(s.ammo);
    let k = velocity as f64 * 40.0 * bullet_velocity_mult(ts, bt) as f64 * f.frametime as f64;
    s.draw_pos = v::add(s.dir.map(|d| (k * d as f64) as f32), s.draw_pos);
    let now = engine().client_time();
    if crate::input::cvars().cl_wavedetail.value() > 0.0 && now > s.next_trail {
        s.next_trail = (now as f64 + slow_time(ts, 0.01) as f64) as f32;
        let (p1, p2, up) = (s.draw_pos, s.trail, f.cam.up);
        let t = effects::now();
        effects::with_psys(|ps| effects::fx::bullet_trail(ps, t, p1, p2, w, alpha, up));
        s.trail = s.draw_pos;
    }
}

/// `SlowTime(x)` (`0x10056fa0`): `x · (1 / max(ts, 0.01))`, rounded to a float.
fn slow_time(ts: f32, x: f32) -> f32 {
    let m = if ts < 0.01 { 0.01f32 } else { ts };
    (1.0 / m as f64 * x as f64) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_and_penetration_tests_follow_the_x87_compares() {
        assert!(trace_hit(0.5));
        assert!(!trace_hit(1.0));
        assert!(!trace_hit(f32::NAN), "fcomp sets C3 for a NaN: a miss");
        assert!(penetrates(5.5));
        assert!(!penetrates(5.0));
        assert!(!penetrates(f32::NAN));
        assert_eq!(REMAINING.0.get(), 0.0, "TS's global starts at zero (.bss)");
    }

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

    /// `ClientBullets_Add` (0x10035c20): first free slot, every position at the source, the
    /// direction as given; a full table drops the new bullet.
    #[test]
    fn client_bullet_slots() {
        let mut b = ClientBullets::default();
        let dir = [0.9, 0.1, 0.05];
        b.add(3, [1.0, 2.0, 3.0], dir, 30.0, 2, 0);
        assert_eq!(b.count, 1);
        let s = b.slots()[0];
        assert_eq!(
            (s.shooter, s.pos, s.start, s.draw_pos, s.trail),
            (3, [1.0, 2.0, 3.0], [1.0, 2.0, 3.0], [1.0, 2.0, 3.0], [1.0, 2.0, 3.0])
        );
        assert_eq!((s.dir, s.dmg, s.ammo, s.bullet_type, s.next_update, s.next_trail), (dir, 30.0, 2, 0, 0.0, 0.0));
        assert_eq!(b.slots().len(), CLIENT_BULLETS);
        for i in 1..CLIENT_BULLETS {
            b.add(i as i32 % 32 + 1, [0.0; 3], dir, 1.0, 1, 0);
        }
        assert_eq!(b.count, 256);
        b.add(9, [5.0; 3], dir, 1.0, 1, 0);
        assert_eq!(b.count, 256, "full: dropped");
        assert!(b.slots().iter().all(|s| s.pos != [5.0; 3]));
        // A freed slot is reused first.
        b.slots[7] = ClientBullet::default();
        b.add(9, [5.0; 3], dir, 1.0, 1, 0);
        assert_eq!(b.slots()[7].pos, [5.0; 3]);
        b.reset();
        assert_eq!(b.count, 0);
        assert!(b.slots().iter().all(|s| *s == ClientBullet::default()));
    }

    /// The trace-step throttle (0x10035b97..0x10035bc9) and the step time (0x10035d82).
    #[test]
    fn client_bullet_throttle() {
        assert_eq!(next_update(10.0, 0), 10.0 + 0.1f32);
        assert_eq!(next_update(10.0, 1), 10.0 + 0.05f32);
        assert_eq!(next_update(10.0, 2), 10.0, "2 and up: every frame");
        assert_eq!(next_update(10.0, -1), 10.0);
        assert_eq!(step_time(10.1, 10.0, 0.016), 10.1f32 - 10.0);
        assert_eq!(step_time(10.0, 10.0, 0.016), 0.016, "due now: the frame time");
        assert_eq!(step_time(10.0005, 10.0, 0.016), 0.016);
    }

    #[test]
    fn client_bullet_looks() {
        assert_eq!(bullet_width(1.0), 0.15);
        assert_eq!(bullet_width(0.2), 0.96);
        assert_eq!(bullet_width(0.95), 0.15, "1 - 0.9025 is under 0.15");
        assert_eq!(streak_length(0.1), 5.0);
        assert_eq!(streak_length(0.4), 15.0);
        assert_eq!(streak_length(0.98), 25.0);
        assert_eq!(near_alpha([0.0; 3], [100.0, 0.0, 0.0]), 1.0);
        assert_eq!(near_alpha([0.0; 3], [2.0, 0.0, 0.0]), 0.4);
        assert_eq!(near_alpha([0.0; 3], [0.0, 0.0, 3.0]), 0.90000004);
        assert_eq!(slow_time(0.5, 0.01), 0.02);
        assert_eq!(slow_time(0.001, 0.01), 1.0, "ts floored at 0.01");
    }

    /// `ClientBullets_Update`'s quads (0x10035335..0x100359be): two crossed streaks and a
    /// billboard; buckshot only the billboard.
    #[test]
    fn client_bullet_quads() {
        let (up, right) = ([0.0, 0.0, 1.0], [0.0, -1.0, 0.0]);
        let q = bullet_quads([100.0, 0.0, 0.0], [1.0, 0.0, 0.0], false, 10.0, 0.5, up, right);
        assert_eq!(q.len(), 3);
        assert_eq!(q[0], [[110.0, 0.0, -0.5], [110.0, 0.0, 0.5], [90.0, 0.0, 0.5], [90.0, 0.0, -0.5]]);
        assert_eq!(q[1], [[110.0, 0.5, 0.0], [110.0, -0.5, 0.0], [90.0, -0.5, 0.0], [90.0, 0.5, 0.0]]);
        assert_eq!(q[2], [[100.0, 0.5, 0.5], [100.0, -0.5, 0.5], [100.0, -0.5, -0.5], [100.0, 0.5, -0.5]]);
        let b = bullet_quads([100.0, 0.0, 0.0], [1.0, 0.0, 0.0], true, 10.0, 0.5, up, right);
        assert_eq!(b, vec![q[2]]);
    }

    /// The range index is `dist / (unit·10) · 10`, truncated (0x10045b41, 0x10035f9f).
    #[test]
    fn range_index_truncates_tenths() {
        assert_eq!(range_index(400.0, 80.0), 5);
        assert_eq!(range_index(799.0, 80.0), 9);
        assert_eq!(range_index(4000.0, 80.0), 50);
        assert_eq!(range_index(0.0, 80.0), 0);
    }

    /// `0x1003de98`: the silenced flag alone picks the sample, even an empty one.
    #[test]
    fn fire_sound_sample() {
        assert_eq!(
            fire_sample("glock/glock-fire.wav", "glock/glock-fire-sil.wav", false),
            ("weapons/glock/glock-fire.wav".into(), (0.92, 1.0), 1.0)
        );
        assert_eq!(
            fire_sample("glock/glock-fire.wav", "glock/glock-fire-sil.wav", true),
            ("weapons/glock/glock-fire-sil.wav".into(), (0.84, 0.92), 2.0)
        );
        assert_eq!(fire_sample("ak47/fire.wav", "", true), (String::new(), (0.84, 0.92), 2.0));
        assert_eq!(fire_sample("", "x.wav", false), (String::new(), (0.92, 1.0), 1.0));
    }

    /// Event parameters are the server's: any weapon id, ammo index, seed and float.
    #[test]
    fn event_params_out_of_range() {
        for id in [i32::MIN, -1, 0, 29, 38, 255, 256, i32::MAX] {
            let _ = weapon_body(id, -1);
            let _ = super::info::info(id).map(|i| i.set(true));
            let _ = super::info::class(id);
        }
        for ammo in [i32::MIN, -1, 14, 16, 255, i32::MAX] {
            assert_eq!(ammo_consts(ammo), (0.0, 80.0, 330.0));
        }
        for x in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, f32::MAX, 0.0] {
            let _ = range_index(x, x);
            let _ = impact::range_mult(range_index(x, 80.0));
            let _ = (bullet_velocity_mult(x, true), bullet_width(x), streak_length(x), slow_time(x, x), step_time(x, x, x));
            let _ = pellet_dir([x; 3], [x; 3], [x; 3], [x, x], i32::MIN, i32::MAX);
            let _ = near_alpha([x; 3], [0.0; 3]);
        }
        for rb in [i32::MIN, -1, 0, 1, 2, i32::MAX] {
            let _ = next_update(1.0, rb);
        }
        let mut b = ClientBullets::default();
        for i in 0..1000 {
            b.add(i32::MIN + i, [f32::NAN; 3], [f32::INFINITY; 3], f32::NAN, i32::MAX, i32::MIN);
        }
        assert_eq!(b.count, CLIENT_BULLETS as i32);
    }

    #[test]
    fn velocity_mult() {
        assert_eq!(bullet_velocity_mult(1.0, false), 1.0);
        assert!((bullet_velocity_mult(0.35, false) - 0.35).abs() < 1e-6);
        let bt = bullet_velocity_mult(0.35, true);
        assert!((bt - 0.35 / ((1.0 - 0.35 * 0.35) * 10.0)).abs() < 1e-6);
    }
}
