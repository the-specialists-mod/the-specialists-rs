//! `func_door`, `func_door_rotating`, `func_water` (SDK doors.cpp + TS `CBaseDoor`/`CRotDoor`).
//!
//! TS changes ported here:
//! * Breakable doors: keys `maxdamage` (> 0 makes the door take damage), `doormaterial`,
//!   `respawntime` (negative clamps to 0; 0 = 30 s). Only blast and the TS shotgun/kung-fu damage
//!   bits count, accumulated until `maxdamage`. A closed door may first only "loosen" (50 %, or 70 %
//!   for shotgun hits: it stops opening and the next qualifying hit breaks it). Breaking plays the
//!   material sound, sends debris (`TSFrags` for glass, `TE_BREAKMODEL` otherwise), hides the door,
//!   and respawns it at its spawn position once no client is in the way.
//! * `DoorTouch` opens at `max(speed, toucher speed)`.
//! * Rotating doors pick their opening direction from the activator's *velocity* (not its view).
//! * The SDK 2.3 `LinearMoveDone` (no re-move on small errors) is used, see base.rs.

use super::base::*;
use super::breakable::{DMG_TS_KUNGFU, DMG_TS_SHOTGUN, break_sounds, mat, play_break_sound, precache_material, send_glass_debris};
use super::buttons::LockSounds;
use crate::damage::bits as dmg;
use crate::world::{self, Cx, DamageInfo, Entity, UseType};
use goldsrc::edict::EdictPtr;
use goldsrc::ext::entities::{ATTN_NORM, EHandle, Vec3, is_player, snd, te_break_model, vec};
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::ffi::CStr;

pub const SF_DOOR_START_OPEN: i32 = 1;
pub const SF_DOOR_ROTATE_BACKWARDS: i32 = 2;
pub const SF_DOOR_PASSABLE: i32 = 8;
pub const SF_DOOR_ONEWAY: i32 = 16;
pub const SF_DOOR_NO_AUTO_RETURN: i32 = 32;
pub const SF_DOOR_USE_ONLY: i32 = 256;
pub const SF_DOOR_SILENT: i32 = i32::MIN; // 0x80000000

#[derive(Default)]
pub struct Door {
    t: Toggle<Self>,
    rotating: bool,
    move_snd: i32,
    stop_snd: i32,
    health_value: i32,
    ls: LockSounds,
    // TS breakable door
    max_damage: f32,
    material: i32,
    respawn_time: f32,
    gib_index: i32,
    spawn_origin: Vec3,
    spawn_angles: Vec3,
    loosened: bool,
    damage_taken: f32,
    damage_cooldown: f32,
}

impl Mover for Door {
    fn toggle(&mut self) -> &mut Toggle<Self> {
        &mut self.t
    }
}

fn move_sound(n: i32) -> &'static CStr {
    match n {
        1 => c"doors/doormove1.wav",
        2 => c"doors/doormove2.wav",
        3 => c"doors/doormove3.wav",
        4 => c"doors/doormove4.wav",
        5 => c"doors/doormove5.wav",
        6 => c"doors/doormove6.wav",
        7 => c"doors/doormove7.wav",
        8 => c"doors/doormove8.wav",
        9 => c"doors/doormove9.wav",
        10 => c"doors/doormove10.wav",
        _ => c"common/null.wav",
    }
}

fn stop_sound(n: i32) -> &'static CStr {
    match n {
        1 => c"doors/doorstop1.wav",
        2 => c"doors/doorstop2.wav",
        3 => c"doors/doorstop3.wav",
        4 => c"doors/doorstop4.wav",
        5 => c"doors/doorstop5.wav",
        6 => c"doors/doorstop6.wav",
        7 => c"doors/doorstop7.wav",
        8 => c"doors/doorstop8.wav",
        _ => c"common/null.wav",
    }
}

impl Door {
    fn precache(&mut self, cx: &mut Cx) {
        let eng = engine();
        let m = move_sound(self.move_snd);
        let st = stop_sound(self.stop_snd);
        eng.precache_sound(m);
        eng.precache_sound(st);
        let v = cx.pev();
        v.noise1 = alloc(m);
        v.noise2 = alloc(st);
        self.ls.precache();
        self.gib_index = 0;
        if self.max_damage > 0.0 {
            // TS door gib table: glass, metal, cinder, ceiling, computer; everything else wood.
            let m = match self.material {
                mat::GLASS | mat::UNBREAKABLE_GLASS => mat::GLASS,
                mat::METAL | mat::CINDER | mat::CEILING | mat::COMPUTER => self.material,
                _ => mat::WOOD,
            };
            self.gib_index = precache_material(m, 0);
            super::breakable::material_sound_precache(self.material);
            // TS plays the real material's break sound but precaches by the table above; precache
            // both so e.g. flesh doors don't play an unprecached sound.
            for snd in break_sounds(self.material).0 {
                eng.precache_sound(snd);
            }
        }
    }

    fn silent(cx: &mut Cx) -> bool {
        cx.pev().spawnflags & SF_DOOR_SILENT != 0
    }

    fn door_touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        if !is_player(other) {
            return;
        }
        if self.t.master != 0 && !is_master_triggered(self.t.master, other) {
            self.ls.play(cx.edict, true, false);
        }
        if cx.pev().targetname != 0 {
            self.ls.play(cx.edict, true, false);
            return;
        }
        self.t.activator = EHandle::new(other);
        // TS: open at least as fast as the toucher is moving.
        let saved = cx.pev().speed;
        let ospeed = vec::length(other.v().velocity);
        cx.pev().speed = saved.max(ospeed);
        if self.door_activate(cx) {
            self.t.touch = None;
        }
        cx.pev().speed = saved;
    }

    fn door_use(&mut self, cx: &mut Cx, activator: *mut edict_t) {
        self.t.activator = EHandle::new(activator);
        let st = self.t.state;
        if st == ToggleState::AtBottom || (cx.pev().spawnflags & SF_DOOR_NO_AUTO_RETURN != 0 && st == ToggleState::AtTop) {
            self.door_activate(cx);
        }
    }

    fn door_activate(&mut self, cx: &mut Cx) -> bool {
        if self.loosened || cx.pev().effects & EF_NODRAW as i32 != 0 {
            return false;
        }
        if !is_master_triggered(self.t.master, self.t.activator.get()) {
            return false;
        }
        if cx.pev().spawnflags & SF_DOOR_NO_AUTO_RETURN != 0 && self.t.state == ToggleState::AtTop {
            self.door_go_down(cx);
        } else {
            let act = self.t.activator.get();
            if is_player(act) && self.health_value > 0 {
                super::items::player_take_health(act, self.health_value as f32);
            }
            self.ls.play(cx.edict, false, false);
            self.door_go_up(cx);
        }
        true
    }

    fn door_go_up(&mut self, cx: &mut Cx) {
        if !Self::silent(cx) && self.t.state != ToggleState::GoingUp && self.t.state != ToggleState::GoingDown {
            let n = s(cx.pev().noise1);
            emit(cx.edict, CHAN_STATIC, n, 1.0, ATTN_NORM, 0, PITCH_NORM as i32);
        }
        self.t.state = ToggleState::GoingUp;
        self.t.move_done = Some(Self::door_hit_top);
        let speed = cx.pev().speed;
        if self.rotating {
            let mut sign = 1.0;
            let act = self.t.activator.get();
            let v = cx.pev();
            if !act.is_null() && v.spawnflags & SF_DOOR_ONEWAY == 0 && v.movedir[1] != 0.0 {
                let av = act.v();
                let to = vec::sub(av.origin, v.origin);
                // TS: where the activator is heading, not where it's looking.
                let next = vec::sub(vec::mad(av.origin, vec::normalize(av.velocity), 10.0), v.origin);
                if to[0] * next[1] - to[1] * next[0] < 0.0 {
                    sign = -1.0;
                }
            }
            let a = vec::scale(self.t.angle2, sign);
            angular_move(self, cx, a, speed);
        } else {
            let p = self.t.pos2;
            linear_move(self, cx, p, speed);
        }
    }

    fn door_hit_top(&mut self, cx: &mut Cx) {
        let me = cx.edict;
        if !Self::silent(cx) {
            let v = cx.pev();
            let (n1, n2) = (s(v.noise1), s(v.noise2));
            emit(me, CHAN_STATIC, n1, 0.0, 0.0, snd::STOP, PITCH_NORM as i32);
            emit(me, CHAN_STATIC, n2, 1.0, ATTN_NORM, 0, PITCH_NORM as i32);
        }
        self.t.state = ToggleState::AtTop;
        let sf = cx.pev().spawnflags;
        if sf & SF_DOOR_NO_AUTO_RETURN != 0 {
            if sf & SF_DOOR_USE_ONLY == 0 {
                self.t.touch = Some(Self::door_touch);
            }
        } else {
            let w = self.t.wait;
            think_ltime(cx, w);
            self.t.think = Some(Self::door_go_down);
            if w == -1.0 {
                cx.pev().nextthink = -1.0;
            }
        }
        let act = self.t.activator.get();
        let netname = cx.pev().netname;
        if netname != 0 && sf & SF_DOOR_START_OPEN != 0 {
            crate::entities::fire_targets(s(netname), act, me, UseType::Toggle, 0.0);
        }
        self.t.use_targets(me, act, UseType::Toggle, 0.0);
    }

    fn door_go_down(&mut self, cx: &mut Cx) {
        if !Self::silent(cx) && self.t.state != ToggleState::GoingUp && self.t.state != ToggleState::GoingDown {
            let n = s(cx.pev().noise1);
            emit(cx.edict, CHAN_STATIC, n, 1.0, ATTN_NORM, 0, PITCH_NORM as i32);
        }
        self.t.state = ToggleState::GoingDown;
        self.t.move_done = Some(Self::door_hit_bottom);
        let speed = cx.pev().speed;
        if self.rotating {
            let a = self.t.angle1;
            angular_move(self, cx, a, speed);
        } else {
            let p = self.t.pos1;
            linear_move(self, cx, p, speed);
        }
    }

    fn door_hit_bottom(&mut self, cx: &mut Cx) {
        let me = cx.edict;
        if !Self::silent(cx) {
            let v = cx.pev();
            let (n1, n2) = (s(v.noise1), s(v.noise2));
            emit(me, CHAN_STATIC, n1, 0.0, 0.0, snd::STOP, PITCH_NORM as i32);
            emit(me, CHAN_STATIC, n2, 1.0, ATTN_NORM, 0, PITCH_NORM as i32);
        }
        self.t.state = ToggleState::AtBottom;
        let sf = cx.pev().spawnflags;
        self.t.touch = if sf & SF_DOOR_USE_ONLY == 0 { Some(Self::door_touch) } else { None };
        let act = self.t.activator.get();
        self.t.use_targets(me, act, UseType::Toggle, 0.0);
        let netname = cx.pev().netname;
        if netname != 0 && sf & SF_DOOR_START_OPEN == 0 {
            crate::entities::fire_targets(s(netname), act, me, UseType::Toggle, 0.0);
        }
    }

    /// Called by a sibling door's `Blocked` (doors sharing a targetname move together).
    fn sibling_reverse(&mut self, cx: &mut Cx, origin: Vec3, angles: Vec3, velocity: Vec3) {
        if self.t.wait < 0.0 {
            return;
        }
        let v = cx.pev();
        // SDK bug kept: avelocity is compared against the blocked door's *velocity*.
        if v.velocity == velocity && v.avelocity == velocity {
            if self.rotating {
                v.angles = angles;
                v.avelocity = [0.0; 3];
            } else {
                v.origin = origin;
                v.velocity = [0.0; 3];
            }
        }
        if self.t.state == ToggleState::GoingDown {
            self.door_go_up(cx);
        } else {
            self.door_go_down(cx);
        }
    }

    fn door_spawn(&mut self, cx: &mut Cx) {
        // TS breakable-door setup.
        let v = cx.pev();
        self.spawn_origin = v.origin;
        self.spawn_angles = v.angles;
        self.loosened = false;
        self.damage_taken = 0.0;
        self.damage_cooldown = 0.0;
        v.takedamage = if self.max_damage > 0.0 { DAMAGE_YES } else { DAMAGE_NO } as f32;
        self.rotating = s(v.classname) == c"func_door_rotating";

        self.precache(cx);
        if self.rotating {
            self.rot_spawn(cx);
        } else {
            self.linear_spawn(cx);
        }
        let sf = cx.pev().spawnflags;
        self.t.state = ToggleState::AtBottom;
        self.t.touch = if sf & SF_DOOR_USE_ONLY != 0 { None } else { Some(Self::door_touch) };
    }

    fn linear_spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        let v = cx.pev();
        set_movedir(v);
        if v.skin == 0 {
            v.solid = if v.spawnflags & SF_DOOR_PASSABLE != 0 { SOLID_NOT } else { SOLID_BSP } as i32;
        } else {
            // func_water and friends: special contents, silent.
            v.solid = SOLID_NOT as i32;
            v.spawnflags |= SF_DOOR_SILENT;
        }
        v.movetype = MOVETYPE_PUSH as i32;
        let org = v.origin;
        eng.set_origin(cx.edict, org);
        set_own_model(cx);
        let v = cx.pev();
        if v.speed == 0.0 {
            v.speed = 100.0;
        }
        self.t.pos1 = v.origin;
        let md = v.movedir;
        let dist = (md[0] * (v.size[0] - 2.0)).abs() + (md[1] * (v.size[1] - 2.0)).abs() + (md[2] * (v.size[2] - 2.0)).abs() - self.t.lip;
        self.t.pos2 = vec::mad(self.t.pos1, md, dist);
        if v.spawnflags & SF_DOOR_START_OPEN != 0 {
            let p2 = self.t.pos2;
            eng.set_origin(cx.edict, p2);
            self.t.pos2 = self.t.pos1;
            self.t.pos1 = cx.pev().origin;
        }
    }

    fn rot_spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        let v = cx.pev();
        axis_dir(v);
        if v.spawnflags & SF_DOOR_ROTATE_BACKWARDS != 0 {
            v.movedir = vec::scale(v.movedir, -1.0);
        }
        self.t.angle1 = v.angles;
        self.t.angle2 = vec::mad(v.angles, v.movedir, self.t.move_distance);
        v.solid = if v.spawnflags & SF_DOOR_PASSABLE != 0 { SOLID_NOT } else { SOLID_BSP } as i32;
        v.movetype = MOVETYPE_PUSH as i32;
        let org = v.origin;
        eng.set_origin(cx.edict, org);
        set_own_model(cx);
        let v = cx.pev();
        if v.speed == 0.0 {
            v.speed = 100.0;
        }
        if v.spawnflags & SF_DOOR_START_OPEN != 0 {
            // SDK bug kept for parity: both angles end up as angle1.
            v.angles = self.t.angle2;
            let sav = self.t.angle1;
            self.t.angle2 = self.t.angle1;
            self.t.angle1 = sav;
            v.movedir = vec::scale(v.movedir, -1.0);
        }
    }

    /// TS `CBaseDoor::TraceAttack`: the breakable-door damage path.
    fn door_damage(&mut self, cx: &mut Cx, info: &DamageInfo) {
        let eng = engine();
        let now = eng.time();
        if now < self.damage_cooldown || cx.pev().takedamage == 0.0 {
            return;
        }
        if info.bits & (DMG_TS_SHOTGUN | DMG_TS_KUNGFU | dmg::BLAST) == 0 {
            return;
        }
        if s(cx.pev().classname) == c"func_water" {
            return;
        }
        let d = if info.bits & dmg::BLAST != 0 { info.damage * 2.0 } else { info.damage };
        self.damage_taken += d;
        if self.damage_taken < self.max_damage {
            return;
        }
        if !self.loosened {
            let r = eng.random_long(0, 100);
            let threshold = if info.bits & DMG_TS_SHOTGUN != 0 { 30 } else { 50 };
            if r > threshold && self.t.state == ToggleState::AtBottom {
                self.loosened = true;
                self.damage_cooldown = now + 0.1;
                return;
            }
        }
        let me = cx.edict;
        play_break_sound(me, self.material, 0.0, 0.5);
        let (_, _, flags) = break_sounds(self.material);
        let v = cx.pev();
        let center = vec::add(v.origin, vec::mid(v.mins, v.maxs));
        let from = if info.attacker.is_null() { center } else { info.attacker.v().origin };
        let dir = vec::normalize(vec::sub(center, from));
        let scale = if info.bits & DMG_TS_SHOTGUN != 0 { 8.0 } else { 1.0 };
        let vel = vec::scale(dir, info.damage * 2.0 * scale);
        if self.material == mat::GLASS {
            send_glass_debris(center, v.size, [0.0; 3], [0.0; 3], 0, 0.0);
        } else {
            te_break_model(MSG_PVS, center, v.size, vel, 10, self.gib_index, 0, 25, flags);
        }
        let v = cx.pev();
        v.solid = SOLID_NOT as i32;
        v.effects = EF_NODRAW as i32;
        v.takedamage = DAMAGE_NO as f32;
        v.deadflag = DEAD_DEAD as i32;
        // The hidden door must not keep sliding while its respawn timer runs.
        v.velocity = [0.0; 3];
        v.avelocity = [0.0; 3];
        self.t.touch = None;
        let t = if self.respawn_time == 0.0 { 30.0 } else { self.respawn_time };
        self.t.think = Some(Self::respawn_think);
        think_ltime(cx, t);
    }

    fn respawn_think(&mut self, cx: &mut Cx) {
        self.breakable_respawn(cx, false);
    }

    /// TS `CBaseDoor::BreakableRespawn` / `CRotDoor::BreakableRespawn`.
    pub fn breakable_respawn(&mut self, cx: &mut Cx, force: bool) {
        let v = cx.pev();
        if !(v.deadflag == DEAD_DEAD as i32 || v.effects == EF_NODRAW as i32 || force) {
            return;
        }
        let (absmin, absmax) = (v.absmin, v.absmax);
        if !engine().entities_in_box(absmin, absmax, (FL_CLIENT | FL_FAKECLIENT) as i32, 1).is_empty() {
            self.t.think = Some(Self::respawn_think);
            think_ltime(cx, 5.0);
            return;
        }
        let v = cx.pev();
        v.deadflag = DEAD_NO as i32;
        v.effects = 0;
        v.origin = self.spawn_origin;
        v.angles = self.spawn_angles;
        v.avelocity = [0.0; 3];
        v.velocity = [0.0; 3];
        let org = self.spawn_origin;
        engine().set_origin(cx.edict, org);
        self.t.touch = None;
        self.t.think = None;
        self.door_spawn(cx);
    }
}

impl Entity for Door {
    /// `CBaseDoor::KeyValue` (0xb51a0). Its `skin` branch is unreachable: `DispatchKeyValue` sets
    /// the entvars field first.
    fn key_value(&mut self, cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"movesnd" => self.move_snd = kv_f(value) as i32,
            b"stopsnd" => self.stop_snd = kv_f(value) as i32,
            b"healthvalue" => self.health_value = kv_f(value) as i32,
            b"WaveHeight" => cx.pev().scale = kv_f(value) * (1.0 / 8.0),
            b"maxdamage" => self.max_damage = kv_i(value) as f32,
            b"doormaterial" => {
                let m = kv_i(value);
                self.material = if (0..10).contains(&m) { m } else { mat::WOOD };
            }
            b"respawntime" => self.respawn_time = (kv_i(value) as f32).max(0.0),
            _ => return self.ls.key_value(key, value) || self.t.key_value(key, value),
        }
        true
    }

    fn spawn(&mut self, cx: &mut Cx) {
        self.door_spawn(cx);
    }

    fn think(&mut self, cx: &mut Cx) {
        run_think(self, cx);
    }

    fn touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        run_touch(self, cx, other);
    }

    fn use_(&mut self, cx: &mut Cx, activator: *mut edict_t, _caller: *mut edict_t, _kind: UseType, _value: f32) {
        self.door_use(cx, activator);
    }

    fn blocked(&mut self, cx: &mut Cx, other: *mut edict_t) {
        let me = cx.edict;
        let d = cx.pev().dmg;
        if d != 0.0 {
            crate::damage::apply(other, &DamageInfo::new(me, me, d, dmg::CRUSH));
        }
        if self.t.wait >= 0.0 {
            if self.t.state == ToggleState::GoingDown {
                self.door_go_up(cx);
            } else {
                self.door_go_down(cx);
            }
        }
        let name = cx.pev().targetname;
        if name == 0 {
            return;
        }
        let (origin, angles, velocity) = {
            let v = cx.pev();
            (v.origin, v.angles, v.velocity)
        };
        let eng = engine();
        let mut e = std::ptr::null_mut();
        loop {
            e = eng.find_by_target_name(e, s(name));
            if e.is_null() {
                break;
            }
            if e == me {
                continue;
            }
            let cls = s(e.v().classname);
            if cls != c"func_door" && cls != c"func_door_rotating" {
                continue;
            }
            world::dispatch(e, |ent, ecx| {
                if let Some(door) = (ent as &mut dyn std::any::Any).downcast_mut::<Door>() {
                    door.sibling_reverse(ecx, origin, angles, velocity);
                }
            });
        }
    }

    fn take_damage(&mut self, cx: &mut Cx, info: &DamageInfo) -> bool {
        self.door_damage(cx, info);
        false
    }
}

/// `CBaseDoor::ObjectCaps`: +use opens use-only doors.
pub fn impulse_use(v: &entvars_t) -> bool {
    v.spawnflags & SF_DOOR_USE_ONLY != 0
}
