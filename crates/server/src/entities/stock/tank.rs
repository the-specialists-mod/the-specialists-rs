//! `func_tank`, `func_tanklaser`, `func_tankrocket`, `func_tankmortar` and `func_tankcontrols`
//! (SDK func_tank.cpp). TS 3.0 keeps the SDK code (decomp `CFuncTank*.c`, 0xc20e8..0xc5294); the
//! differences that matter come from the rest of TS:
//!
//! * TS never fills `gSkillData`'s monster bullet damage (`CHalfLifeMultiplay::RefreshSkillData`
//!   0xcef78 sets only the player values), so a `func_tank` without `bullet_damage` does no damage.
//! * TS doesn't link `rpg_rocket`, so `func_tankrocket` creates nothing; only its sprites and
//!   targets fire.
//!
//! The player's side of tank control (`m_pTank`, holstering, `PostThink`) is in `drive.rs`.

use super::base::*;
use super::drive;
use super::effects::{Laser, Sprite, decal_trace, explosion_create};
use super::explode::emit_sound_slow;
use super::tracktrain::angle_distance;
use crate::entities::fire_targets;
use crate::rules::has_player;
use crate::world::{self, Cx, DamageInfo, Entity, UseType};
use goldsrc::edict::EdictPtr;
use goldsrc::ext::entities::{ATTN_NORM, EHandle, MsgWriterExt, Vec3, snd, te, te_begin, vec};
use goldsrc::server::{DONT_IGNORE_MONSTERS, engine};
use goldsrc::sys::*;
use std::ffi::CStr;

pub const SF_TANK_ACTIVE: i32 = 0x0001;
pub const SF_TANK_LINEOFSIGHT: i32 = 0x0010;
pub const SF_TANK_CANCONTROL: i32 = 0x0020;
pub const SF_TANK_SOUNDON: i32 = 0x8000;

/// `gTankSpread`.
const TANK_SPREAD: [Vec3; 5] = [[0.0; 3], [0.025; 3], [0.05; 3], [0.1; 3], [0.25; 3]];

/// The `func_tank*` subclasses, as the const parameter of [`FuncTank`].
pub const GUN: u8 = 0;
pub const LASER: u8 = 1;
pub const ROCKET: u8 = 2;
pub const MORTAR: u8 = 3;

/// `CFuncTank` and its subclasses (`K` = [`GUN`], [`LASER`], [`ROCKET`] or [`MORTAR`]).
#[derive(Default)]
pub struct FuncTank<const K: u8> {
    controller: EHandle,
    next_attack: f32,
    controller_use_pos: Vec3,
    yaw_center: f32,
    yaw_rate: f32,
    yaw_range: f32,
    yaw_tolerance: f32,
    pitch_center: f32,
    pitch_rate: f32,
    pitch_range: f32,
    pitch_tolerance: f32,
    fire_last: f32,
    fire_rate: f32,
    last_sight_time: f32,
    persist: f32,
    min_range: f32,
    max_range: f32,
    barrel_pos: Vec3,
    sprite_scale: f32,
    sprite_smoke: string_t,
    sprite_flash: string_t,
    bullet_type: i32,
    bullet_damage: i32,
    sight_origin: Vec3,
    spread: i32,
    master: string_t,
    // CFuncTankLaser
    laser: EHandle,
    laser_time: f32,
}

/// `CFuncTank::OnControls` (0xc2970) for any tank class: `SF_TANK_CANCONTROL` and `pv` within 30
/// units of where the controller pressed +use.
pub fn on_controls(tank: *mut edict_t, pv: &entvars_t) -> bool {
    fn pos<const K: u8>(e: *mut edict_t) -> Option<Vec3> {
        world::with::<FuncTank<K>, _>(e, |t| t.controller_use_pos)
    }
    let Some(use_pos) = pos::<GUN>(tank).or_else(|| pos::<LASER>(tank)).or_else(|| pos::<ROCKET>(tank)).or_else(|| pos::<MORTAR>(tank))
    else {
        return false;
    };
    tank.v().spawnflags & SF_TANK_CANCONTROL != 0 && vec::length(vec::sub(use_pos, pv.origin)) < 30.0
}

/// `UTIL_MakeAimVectors`: pitch is negated for the brush model's angles.
fn aim_vectors(angles: Vec3) -> (Vec3, Vec3, Vec3) {
    engine().make_vectors([-angles[0], angles[1], angles[2]])
}

/// Circular gaussian spread (`FireBullets` / `TankTrace`): `forward` jittered along `right`/`up`.
fn spread_dir(forward: Vec3, spread: Vec3, right: Vec3, up: Vec3) -> Vec3 {
    let eng = engine();
    let (x, y) = loop {
        let x = eng.random_float(-0.5, 0.5) + eng.random_float(-0.5, 0.5);
        let y = eng.random_float(-0.5, 0.5) + eng.random_float(-0.5, 0.5);
        if x * x + y * y <= 1.0 {
            break (x, y);
        }
    };
    vec::add(vec::add(forward, vec::scale(right, x * spread[0])), vec::scale(up, y * spread[1]))
}

impl<const K: u8> FuncTank<K> {
    fn is_active(cx: &mut Cx) -> bool {
        cx.pev().spawnflags & SF_TANK_ACTIVE != 0
    }

    fn spread(&self) -> Vec3 {
        // TS keeps the SDK's off-by-one (`spread > 5` resets; 5 reads past the table): use 0.
        TANK_SPREAD.get(self.spread as usize).copied().unwrap_or([0.0; 3])
    }

    /// `CFuncTank::Precache` (0xc2390): the smoke and flash sprites, the rotate sound.
    fn precache(&mut self, cx: &mut Cx) {
        if self.sprite_smoke != 0 {
            precache_model_s(self.sprite_smoke);
        }
        if self.sprite_flash != 0 {
            precache_model_s(self.sprite_flash);
        }
        let noise = cx.pev().noise;
        if noise != 0 {
            precache_sound_s(noise);
        }
    }

    /// `CFuncTank::BarrelPosition` (0xc5294).
    fn barrel_position(&self, cx: &mut Cx) -> Vec3 {
        let v = cx.pev();
        let (f, r, u) = engine().angle_vectors(v.angles);
        let p = vec::mad(v.origin, f, self.barrel_pos[0]);
        let p = vec::mad(p, r, self.barrel_pos[1]);
        vec::mad(p, u, self.barrel_pos[2])
    }

    fn can_fire(&self) -> bool {
        engine().time() - self.last_sight_time < self.persist
    }

    fn in_range(&self, range: f32) -> bool {
        !(range < self.min_range || (self.max_range > 0.0 && range > self.max_range))
    }

    fn activate(&mut self, cx: &mut Cx) {
        let v = cx.pev();
        v.spawnflags |= SF_TANK_ACTIVE;
        v.nextthink = v.ltime + 0.1;
        self.fire_last = 0.0;
    }

    fn deactivate(&mut self, cx: &mut Cx) {
        cx.pev().spawnflags &= !SF_TANK_ACTIVE;
        self.fire_last = 0.0;
        self.stop_rot_sound(cx);
    }

    /// `CFuncTank::StartRotSound` (0xc3e60).
    fn start_rot_sound(&mut self, cx: &mut Cx) {
        let v = cx.pev();
        if v.noise == 0 || v.spawnflags & SF_TANK_SOUNDON != 0 {
            return;
        }
        v.spawnflags |= SF_TANK_SOUNDON;
        let noise = s(v.noise);
        emit_sound_slow(cx.edict, CHAN_STATIC, noise, 0.85, ATTN_NORM, 0, 100, 0.5);
    }

    /// `CFuncTank::StopRotSound` (0xc3ed8).
    fn stop_rot_sound(&mut self, cx: &mut Cx) {
        let v = cx.pev();
        if v.spawnflags & SF_TANK_SOUNDON != 0 {
            let noise = s(v.noise);
            emit_sound_slow(cx.edict, CHAN_STATIC, noise, 0.0, 0.0, snd::STOP, 100, 0.5);
        }
        cx.pev().spawnflags &= !SF_TANK_SOUNDON;
    }

    /// `CFuncTank::StartControl` (0xc2a24). The player's half (holster, hide the weapon HUD) is
    /// `drive::tank_started`, run from `Use` inside the call as in TS when the player lent itself
    /// (`player_use`).
    fn start_control(&mut self, cx: &mut Cx, player: *mut edict_t) -> bool {
        if self.controller.is_valid() {
            return false;
        }
        if self.master != 0 && !is_master_triggered(self.master, player) {
            return false;
        }
        engine().alert(ALERT_TYPE::at_console, "using TANK!\n");
        self.controller = EHandle::new(player);
        self.controller_use_pos = player.v().origin;
        let v = cx.pev();
        v.nextthink = v.ltime + 0.1;
        true
    }

    /// `CFuncTank::StopControl` (0xc2b1c).
    fn stop_control(&mut self, cx: &mut Cx) {
        let player = self.controller.get();
        if player.is_null() {
            return;
        }
        // `m_pController->m_pActiveItem->Deploy()` at once, as in TS: every caller holding the
        // player lends it (`drive`); a held, unlent one redeploys once its dispatch returns.
        world::run_now_or_defer(player, move || drive::tank_stopped(player));
        engine().alert(ALERT_TYPE::at_console, "stopped using TANK\n");
        self.controller = EHandle::NULL;
        let v = cx.pev();
        v.nextthink = 0.0;
        if v.spawnflags & SF_TANK_ACTIVE != 0 {
            v.nextthink = v.ltime + 1.0;
        }
    }

    /// `CFuncTank::ControllerPostFrame` (0xc2bb4): fire while the controller holds +attack.
    fn controller_post_frame(&mut self, cx: &mut Cx) {
        let now = engine().time();
        let player = self.controller.get();
        if now < self.next_attack || player.is_null() || player.v().button & IN_ATTACK as i32 == 0 {
            return;
        }
        let (forward, _, _) = engine().angle_vectors(cx.pev().angles);
        self.fire_last = now - 1.0 / self.fire_rate - 0.01;
        let barrel = self.barrel_position(cx);
        self.fire(cx, barrel, forward, player);
        // (m_iWeaponVolume: AI hearing, not ported.)
        self.next_attack = now + 1.0 / self.fire_rate;
    }

    /// `CFuncTank::TrackTarget` (0xc2fcc).
    fn track_target(&mut self, cx: &mut Cx) {
        let eng = engine();
        let me = cx.edict;
        let player = eng.find_client_in_pvs(me);
        let mut update_time = false;
        let mut direction = [0.0; 3];
        let mut barrel_end = [0.0; 3];
        let mut target = std::ptr::null_mut();
        let mut angles;
        let controller = self.controller.get();
        if !controller.is_null() {
            angles = controller.v().v_angle;
            angles[0] = -angles[0];
            let v = cx.pev();
            v.nextthink = v.ltime + 0.05;
        } else {
            let v = cx.pev();
            if v.spawnflags & SF_TANK_ACTIVE == 0 {
                return;
            }
            v.nextthink = v.ltime + 0.1;
            if player.is_null() {
                v.nextthink = v.ltime + 2.0;
                return;
            }
            target = player; // FindTarget
            barrel_end = self.barrel_position(cx);
            let tv = target.v();
            let target_pos = vec::add(tv.origin, tv.view_ofs);
            let range = vec::length(vec::sub(target_pos, barrel_end));
            if !self.in_range(range) {
                return;
            }
            let tr = eng.trace_line(barrel_end, target_pos, DONT_IGNORE_MONSTERS, me);
            if (tr.flFraction == 1.0 || tr.pHit == target) && self.in_range(range) && is_alive(target) {
                update_time = true;
                // CBasePlayer::BodyTarget (0x9736c): centre + view_ofs * random(0.5, 1.1).
                let tv = target.v();
                let center = vec::mid(tv.absmin, tv.absmax);
                self.sight_origin = vec::mad(center, tv.view_ofs, eng.random_float(0.5, 1.1));
            }
            direction = vec::sub(self.sight_origin, cx.pev().origin);
            angles = eng.vec_to_angles(direction);
            self.adjust_angles_for_barrel(&mut angles, vec::length(direction));
        }
        angles[0] = -angles[0];
        angles[1] = self.yaw_center + angle_distance(angles[1], self.yaw_center);
        angles[0] = self.pitch_center + angle_distance(angles[0], self.pitch_center);
        if angles[1] > self.yaw_center + self.yaw_range {
            angles[1] = self.yaw_center + self.yaw_range;
            update_time = false;
        } else if angles[1] < self.yaw_center - self.yaw_range {
            angles[1] = self.yaw_center - self.yaw_range;
            update_time = false;
        }
        if update_time {
            self.last_sight_time = eng.time();
        }
        let v = cx.pev();
        let dist_y = angle_distance(angles[1], v.angles[1]);
        v.avelocity[1] = dist_y * 10.0;
        if v.avelocity[1] > self.yaw_rate {
            v.avelocity[1] = self.yaw_rate;
        } else if v.avelocity[1] < -self.yaw_rate {
            v.avelocity[1] = -self.yaw_rate;
        }
        if angles[0] > self.pitch_center + self.pitch_range {
            angles[0] = self.pitch_center + self.pitch_range;
        } else if angles[0] < self.pitch_center - self.pitch_range {
            angles[0] = self.pitch_center - self.pitch_range;
        }
        let dist_x = angle_distance(angles[0], v.angles[0]);
        v.avelocity[0] = dist_x * 10.0;
        if v.avelocity[0] > self.pitch_rate {
            v.avelocity[0] = self.pitch_rate;
        } else if v.avelocity[0] < -self.pitch_rate {
            v.avelocity[0] = -self.pitch_rate;
        }
        if !controller.is_null() {
            return;
        }
        let los = v.spawnflags & SF_TANK_LINEOFSIGHT != 0;
        if self.can_fire() && ((dist_x.abs() < self.pitch_tolerance && dist_y.abs() < self.yaw_tolerance) || los) {
            let (forward, _, _) = eng.angle_vectors(cx.pev().angles);
            let fire = if los {
                let end = vec::mad(barrel_end, forward, vec::length(direction));
                eng.trace_line(barrel_end, end, DONT_IGNORE_MONSTERS, me).pHit == target
            } else {
                true
            };
            if fire {
                let barrel = self.barrel_position(cx);
                self.fire(cx, barrel, forward, me);
            } else {
                self.fire_last = 0.0;
            }
        } else {
            self.fire_last = 0.0;
        }
    }

    /// `CFuncTank::AdjustAnglesForBarrel` (0xc38f4).
    fn adjust_angles_for_barrel(&self, angles: &mut Vec3, distance: f32) {
        let [_, by, bz] = self.barrel_pos;
        if by == 0.0 && bz == 0.0 {
            return;
        }
        let d2 = (distance - bz) * (distance - bz);
        if by != 0.0 {
            angles[1] += (by as f64).atan2(((d2 - by * by) as f64).sqrt()).to_degrees() as f32;
        }
        if bz != 0.0 {
            angles[0] += (-bz as f64).atan2(((d2 - bz * bz) as f64).sqrt()).to_degrees() as f32;
        }
    }

    /// `(time - m_fireLast) * m_fireRate`, truncated: shots owed since the last fire.
    fn bullet_count(&self) -> i32 {
        ((engine().time() - self.fire_last) * self.fire_rate) as i32
    }

    /// The subclass `Fire`: `CFuncTankGun::Fire` (0xc3f64), `CFuncTankLaser::Fire` (0xc44e0),
    /// `CFuncTankRocket::Fire` (0xc4824), `CFuncTankMortar::Fire` (0xc4a20). Each returns
    /// without the base effects when no shot is due.
    fn fire(&mut self, cx: &mut Cx, barrel_end: Vec3, forward: Vec3, attacker: *mut edict_t) {
        let me = cx.edict;
        if self.fire_last != 0.0 {
            match K {
                GUN => {
                    let (_, right, up) = aim_vectors(cx.pev().angles);
                    let count = self.bullet_count();
                    if count < 1 {
                        return;
                    }
                    let spread = self.spread();
                    for _ in 0..count {
                        if (1..=3).contains(&self.bullet_type) {
                            fire_bullet(me, barrel_end, forward, spread, right, up, self.bullet_damage, attacker);
                        }
                    }
                }
                LASER => {
                    if self.get_laser(cx).is_null() {
                        self.base_fire(cx, barrel_end);
                        return;
                    }
                    let (_, right, up) = aim_vectors(cx.pev().angles);
                    let count = self.bullet_count();
                    if count == 0 {
                        return;
                    }
                    let spread = self.spread();
                    for _ in 0..count.max(0) {
                        let tr = tank_trace(me, barrel_end, forward, spread, right, up);
                        self.laser_time = engine().time();
                        Laser::tank_shot(self.laser.get(), barrel_end, &tr);
                    }
                }
                ROCKET => {
                    if self.bullet_count() < 1 {
                        return;
                    }
                    // CBaseEntity::Create("rpg_rocket") fails in TS (no such class): no rockets.
                }
                _ => {
                    if self.bullet_count() < 1 {
                        return;
                    }
                    let (_, right, up) = aim_vectors(cx.pev().angles);
                    let tr = tank_trace(me, barrel_end, forward, self.spread(), right, up);
                    let v = cx.pev();
                    explosion_create(tr.vecEndPos, v.angles, me, v.impulse, true);
                }
            }
        }
        self.base_fire(cx, barrel_end);
    }

    /// `CFuncTank::Fire` (0xc39d4): smoke and muzzle-flash sprites, fire `target`.
    fn base_fire(&mut self, cx: &mut Cx, barrel_end: Vec3) {
        let eng = engine();
        if self.fire_last != 0.0 {
            if self.sprite_smoke != 0 {
                let sp = Sprite::create(s(self.sprite_smoke), barrel_end, true);
                if !sp.is_null() {
                    Sprite::animate_and_die(sp, eng.random_float(15.0, 20.0));
                    let c = cx.pev().rendercolor;
                    let sv = sp.v();
                    sv.rendermode = kRenderTransAlpha as i32;
                    sv.rendercolor = [c[0] as i32 as f32, c[1] as i32 as f32, c[2] as i32 as f32];
                    sv.renderamt = 255.0;
                    sv.renderfx = kRenderFxNone as i32;
                    sv.velocity[2] = eng.random_float(40.0, 80.0);
                    sv.scale = self.sprite_scale;
                }
            }
            if self.sprite_flash != 0 {
                let sp = Sprite::create(s(self.sprite_flash), barrel_end, true);
                if !sp.is_null() {
                    Sprite::animate_and_die(sp, 60.0);
                    let sv = sp.v();
                    sv.rendermode = kRenderTransAdd as i32;
                    sv.rendercolor = [255.0; 3];
                    sv.renderamt = 255.0;
                    sv.renderfx = kRenderFxNoDissipation as i32;
                    sv.scale = self.sprite_scale;
                    // "Hack Hack, make it stick around for at least 100 ms."
                    sv.nextthink += 0.1;
                }
            }
            let target = cx.pev().target;
            if target != 0 {
                let me = cx.edict;
                fire_targets(s(target), me, me, UseType::Toggle, 0.0);
            }
        }
        self.fire_last = eng.time();
    }

    /// `CFuncTankLaser::GetLaser` (0xc4354): the first `env_laser` named by `laserentity`.
    fn get_laser(&mut self, cx: &mut Cx) -> *mut edict_t {
        let l = self.laser.get();
        if !l.is_null() {
            return l;
        }
        let eng = engine();
        let name = s(cx.pev().message);
        let mut e = std::ptr::null_mut();
        loop {
            e = eng.find_by_target_name(e, name);
            if e.is_null() {
                return e;
            }
            if s(e.v().classname) == c"env_laser" {
                self.laser = EHandle::new(e);
                return e;
            }
        }
    }
}

impl<const K: u8> Entity for FuncTank<K> {
    fn key_value(&mut self, cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"yawrate" => self.yaw_rate = kv_f(value),
            b"yawrange" => self.yaw_range = kv_f(value),
            b"yawtolerance" => self.yaw_tolerance = kv_f(value),
            b"pitchrange" => self.pitch_range = kv_f(value),
            b"pitchrate" => self.pitch_rate = kv_f(value),
            b"pitchtolerance" => self.pitch_tolerance = kv_f(value),
            b"firerate" => self.fire_rate = kv_f(value),
            b"barrel" => self.barrel_pos[0] = kv_f(value),
            b"barrely" => self.barrel_pos[1] = kv_f(value),
            b"barrelz" => self.barrel_pos[2] = kv_f(value),
            b"spritescale" => self.sprite_scale = kv_f(value),
            b"spritesmoke" => self.sprite_smoke = alloc(value),
            b"spriteflash" => self.sprite_flash = alloc(value),
            b"rotatesound" => cx.pev().noise = alloc(value),
            b"persistence" => self.persist = kv_f(value),
            b"bullet" => self.bullet_type = kv_i(value),
            b"bullet_damage" => self.bullet_damage = kv_i(value),
            b"firespread" => self.spread = kv_i(value),
            b"minRange" => self.min_range = kv_f(value),
            b"maxRange" => self.max_range = kv_f(value),
            b"master" => self.master = alloc(value),
            // CFuncTankLaser::KeyValue (0xc42ec) / CFuncTankMortar::KeyValue (0xc49b8).
            b"laserentity" if K == LASER => cx.pev().message = alloc(value),
            b"iMagnitude" if K == MORTAR => cx.pev().impulse = kv_i(value),
            _ => return false,
        }
        true
    }

    /// `CFuncTank::Spawn` (0xc2188) and `Precache` (0xc2390).
    fn spawn(&mut self, cx: &mut Cx) {
        self.precache(cx);
        let v = cx.pev();
        v.movetype = MOVETYPE_PUSH as i32;
        v.solid = SOLID_BSP as i32;
        set_own_model(cx);
        let v = cx.pev();
        self.yaw_center = v.angles[1];
        self.pitch_center = v.angles[0];
        if v.spawnflags & SF_TANK_ACTIVE != 0 {
            v.nextthink = v.ltime + 1.0;
        }
        self.sight_origin = self.barrel_position(cx);
        if self.fire_rate <= 0.0 {
            self.fire_rate = 1.0;
        }
        if self.spread as u32 > TANK_SPREAD.len() as u32 {
            self.spread = 0;
        }
        let v = cx.pev();
        v.oldorigin = v.origin;
    }

    /// `CFuncTank::Save` / `Restore` (0xc20e8 / 0xc2138), then for `func_tanklaser`
    /// `CFuncTankLaser::Save` / `Restore` (0xc41ec / 0xc423c). `Think` and `Use` are virtuals:
    /// no member pointer is set.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut crate::save::Fields) {
        f.base_funcs(cx, crate::save::BaseFuncs::default());
        f.table(&crate::save::tables::CFuncTank);
        f.float(&mut self.yaw_center);
        f.float(&mut self.yaw_rate);
        f.float(&mut self.yaw_range);
        f.float(&mut self.yaw_tolerance);
        f.float(&mut self.pitch_center);
        f.float(&mut self.pitch_rate);
        f.float(&mut self.pitch_range);
        f.float(&mut self.pitch_tolerance);
        f.float(&mut self.fire_last);
        f.float(&mut self.fire_rate);
        f.float(&mut self.last_sight_time);
        f.float(&mut self.persist);
        f.float(&mut self.min_range);
        f.float(&mut self.max_range);
        f.vector(&mut self.barrel_pos);
        f.float(&mut self.sprite_scale);
        f.string(&mut self.sprite_smoke);
        f.string(&mut self.sprite_flash);
        f.int(&mut self.bullet_type);
        f.vector(&mut self.sight_origin);
        f.int(&mut self.spread);
        class_ptr(f, &mut self.controller);
        f.vector(&mut self.controller_use_pos);
        f.float(&mut self.next_attack);
        f.int(&mut self.bullet_damage);
        f.string(&mut self.master);
        if K == LASER {
            f.table(&crate::save::tables::CFuncTankLaser);
            class_ptr(f, &mut self.laser);
            f.float(&mut self.laser_time);
        }
    }

    /// `CFuncTank::Precache` (0xc2390; `func_tankrocket`'s `CFuncTankRocket::Precache` 0xc47f0
    /// first precaches `rpg_rocket`, a class TS doesn't link, which the port skips).
    fn restored(&mut self, cx: &mut Cx) {
        self.precache(cx);
    }

    /// `CFuncTankLaser::Activate` (0xc428c), from `ServerActivate` on a new map and after a
    /// restore: turn the laser off, or remove a tank that has none. The other tanks have
    /// `CBaseEntity`'s no-op.
    fn activate(&mut self, cx: &mut Cx) {
        if K != LASER {
            return;
        }
        if self.get_laser(cx).is_null() {
            crate::entities::remove(cx.edict);
            engine().alert(ALERT_TYPE::at_error, "Laser tank with no env_laser!\n");
        } else {
            Laser::turn_off_entity(self.laser.get());
        }
    }

    fn think(&mut self, cx: &mut Cx) {
        if K == LASER {
            // CFuncTankLaser::Think (0xc4488).
            if self.laser.is_valid() && engine().time() > self.laser_time {
                Laser::turn_off_entity(self.laser.get());
            }
        }
        // Port addition: a controller whose edict no longer holds a player lets go. A controller
        // who left keeps the tank, as `m_pController` does in TS.
        let c = self.controller.get();
        if self.controller.is_valid() && !has_player(c) {
            self.stop_control(cx);
        }
        // CFuncTank::Think (0xc2f48).
        cx.pev().avelocity = [0.0; 3];
        self.track_target(cx);
        let av = cx.pev().avelocity;
        if av[0].abs() > 1.0 || av[1].abs() > 1.0 {
            self.start_rot_sound(cx);
        } else {
            self.stop_rot_sound(cx);
        }
    }

    /// `CFuncTank::Use` (0xc2db8).
    fn use_(&mut self, cx: &mut Cx, activator: *mut edict_t, _caller: *mut edict_t, kind: UseType, value: f32) {
        if cx.pev().spawnflags & SF_TANK_CANCONTROL != 0 {
            if !has_player(activator) {
                return;
            }
            if value == 2.0 && kind == UseType::Set {
                self.controller_post_frame(cx);
            } else if !self.controller.is_valid() && kind != UseType::Off {
                // `pActivator->m_pTank = this` happens even when StartControl refuses.
                let started = self.start_control(cx, activator);
                let tank = cx.edict;
                world::run_now_or_defer(activator, move || drive::tank_started(activator, tank, started));
            } else {
                self.stop_control(cx);
            }
        } else {
            let active = Self::is_active(cx);
            if !should_toggle(kind, active) {
                return;
            }
            if active {
                self.deactivate(cx);
            } else {
                self.activate(cx);
            }
        }
    }
}

/// A `FIELD_CLASSPTR` the port keeps as an [`EHandle`].
fn class_ptr(f: &mut crate::save::Fields, h: &mut EHandle) {
    let cur = h.get();
    let mut e = cur;
    f.ent(&mut e);
    if e != cur {
        *h = EHandle::new(e);
    }
}

fn is_alive(e: *mut edict_t) -> bool {
    let v = e.v();
    v.deadflag == DEAD_NO as i32 && v.health > 0.0
}

/// `CFuncTank::TankTrace` (0xc3c80): a 4096-unit trace along `forward` with spread.
fn tank_trace(me: *mut edict_t, start: Vec3, forward: Vec3, spread: Vec3, right: Vec3, up: Vec3) -> TraceResult {
    let dir = spread_dir(forward, spread, right, up);
    engine().trace_line(start, vec::mad(start, dir, 4096.0), DONT_IGNORE_MONSTERS, me)
}

/// One shot of TS `CBaseEntity::FireBullets` (0xb39a4) as `func_tank` calls it: monster bullet
/// type, a tracer on every shot, `bullet_damage` (or TS's zero monster damage) to what it hits.
#[allow(clippy::too_many_arguments)]
fn fire_bullet(me: *mut edict_t, src: Vec3, forward: Vec3, spread: Vec3, right: Vec3, up: Vec3, damage: i32, attacker: *mut edict_t) {
    use crate::damage::bits;
    use crate::damage::multi;
    let eng = engine();
    // FireBullets(1, …): ClearMultiDamage, the type starts at DMG_BULLET | DMG_NEVERGIB, the shot's
    // TraceAttack adds to it, and ApplyMultiDamage(tank, attacker) after the shot.
    multi::clear();
    multi::set_type(0x1002);
    let dir = spread_dir(forward, spread, right, up);
    let tr = eng.trace_line(src, vec::mad(src, dir, 4096.0), DONT_IGNORE_MONSTERS, me);
    te_begin(MSG_PAS, Some(src), te::TRACER).vec3(src).vec3(tr.vecEndPos).end();
    if tr.flFraction != 1.0 {
        let hit = if tr.pHit.is_null() { eng.entity_of_index(0) } else { tr.pHit };
        // `TraceAttack(attacker, dmg, dir, &tr, bits)`, so the hitgroup counts. Without
        // `bullet_damage` TS still traces the monster bullet type's (zero) skill damage as
        // DMG_BULLET (0xb4012..0xb40fa).
        let (dmg, b) = if damage != 0 {
            (damage as f32, bits::BULLET | if damage > 16 { bits::ALWAYSGIB } else { bits::NEVERGIB })
        } else {
            (0.0, bits::BULLET)
        };
        let mut info = DamageInfo::new(me, attacker, dmg, b).traced();
        info.hitgroup = tr.iHitgroup;
        info.dir = dir;
        info.pos = tr.vecEndPos;
        crate::damage::trace_attack(hit, &info);
        // DecalGunshot: `{shot1`..`{shot5` on brush entities and the world.
        let n = eng.random_long(1, 5);
        decal_trace(&tr, goldsrc::server::intern(&format!("{{shot{n}")));
    }
    multi::apply(me, attacker);
}

// ---- func_tankcontrols ----------------------------------------------------------------------------------

/// `func_tankcontrols` (`CFuncTankControls`, 0xc4b28..0xc4d44): an invisible +use trigger that
/// passes `Use` on to the first `func_tank*` named by `target`.
#[derive(Default)]
pub struct TankControls {
    tank: EHandle,
}

impl Entity for TankControls {
    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        let v = cx.pev();
        v.solid = SOLID_TRIGGER as i32;
        v.movetype = MOVETYPE_NONE as i32;
        v.effects |= EF_NODRAW as i32;
        set_own_model(cx);
        let v = cx.pev();
        let (mins, maxs, org) = (v.mins, v.maxs, v.origin);
        eng.set_size(cx.edict, mins, maxs);
        eng.set_origin(cx.edict, org);
        // After all the func_tanks have spawned.
        cx.pev().nextthink = eng.time() + 0.3;
    }

    /// `CFuncTankControls::Think` (0xc4c08).
    fn think(&mut self, cx: &mut Cx) {
        let eng = engine();
        let target = s(cx.pev().target);
        let mut e = std::ptr::null_mut();
        loop {
            e = eng.find_by_target_name(e, target);
            if e.is_null() || s(e.v().classname).to_bytes().starts_with(b"func_tank") {
                break;
            }
        }
        if e.is_null() {
            eng.alert(ALERT_TYPE::at_console, &format!("No tank {}\n", target.to_string_lossy()));
            return;
        }
        self.tank = EHandle::new(e);
    }

    /// `CFuncTankControls::Save` / `Restore` (0xc4b28 / 0xc4b78): `m_pTank`. `Think` is a
    /// virtual; its `Precache` is `CBaseEntity`'s.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut crate::save::Fields) {
        f.base_funcs(cx, crate::save::BaseFuncs::default());
        f.table(&crate::save::tables::CFuncTankControls);
        class_ptr(f, &mut self.tank);
    }

    /// `CFuncTankControls::Use` (0xc4bd0).
    fn use_(&mut self, _cx: &mut Cx, activator: *mut edict_t, caller: *mut edict_t, kind: UseType, value: f32) {
        let t = self.tank.get();
        if !t.is_null() {
            crate::ham::use_(t, activator, caller, kind, value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn barrel_adjust() {
        let t = FuncTank::<GUN> { barrel_pos: [92.0, 0.0, 0.0], ..Default::default() };
        let mut a = [10.0, 20.0, 0.0];
        t.adjust_angles_for_barrel(&mut a, 500.0);
        assert_eq!(a, [10.0, 20.0, 0.0], "no side/vertical offset: unchanged");
        let t = FuncTank::<GUN> { barrel_pos: [0.0, 10.0, 0.0], ..Default::default() };
        let mut a = [0.0; 3];
        t.adjust_angles_for_barrel(&mut a, 10.0 * 2f32.sqrt());
        assert!((a[1] - 45.0).abs() < 1e-3);
    }

    #[test]
    fn spread_table() {
        let t = FuncTank::<GUN> { spread: 5, ..Default::default() };
        assert_eq!(t.spread(), [0.0; 3]);
        let t = FuncTank::<GUN> { spread: 4, ..Default::default() };
        assert_eq!(t.spread(), [0.25; 3]);
    }
}
