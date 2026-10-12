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
//!
//! `momentary_door` (`CMomentaryDoor`) is a separate `CBaseToggle` in TS too, so none of the
//! `CBaseDoor` changes above reach it; it's the SDK 2.3 class plus TS's slow-motion sound pitch.

use super::base::*;
use super::breakable::{DMG_TS_KUNGFU, DMG_TS_SHOTGUN, break_sounds, mat, play_break_sound, send_glass_debris};
use super::buttons::LockSounds;
use super::explode::emit_sound_slow;
use crate::damage::bits as dmg;
use crate::rules::has_player;
use crate::world::{self, Cx, DamageInfo, Entity, UseType};
use goldsrc::edict::EdictPtr;
use goldsrc::ext::entities::{ATTN_NORM, EHandle, Vec3, snd, te_break_model, vec};
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
    /// `m_bMoveSnd` / `m_bStopSnd` (`CBaseDoor::KeyValue` 0xb51a0: [`fistp16_low_byte`] of the
    /// `strtod`, read unsigned by `Precache` 0xb6448).
    move_snd: u8,
    stop_snd: u8,
    /// `m_bHealthValue` (+0x13d), a BYTE set like `movesnd`.
    health_value: u8,
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
    /// `CBaseDoor::Precache` (0xb6448).
    fn precache(&mut self, cx: &mut Cx) {
        let eng = engine();
        let m = move_sound(self.move_snd as i32);
        let st = stop_sound(self.stop_snd as i32);
        // Any other number is `common/null.wav`, only allocated (`CWorld::Precache` has it).
        for n in [m, st] {
            if n != c"common/null.wav" {
                eng.precache_sound(n);
            }
        }
        let v = cx.pev();
        v.noise1 = alloc(m);
        v.noise2 = alloc(st);
        self.ls.precache();
        self.gib_index = 0;
        if self.max_damage > 0.0 {
            // TS door gib table: glass, metal, cinder, ceiling, computer; everything else wood.
            // Its break sounds first, then the real material's impact sounds, then the model.
            // (The door later plays the real material's break sound, which TS doesn't precache
            // for flesh or rocks: the engine refuses those.)
            let (gib, sounds): (&CStr, &[&CStr]) = match self.material {
                mat::GLASS | mat::UNBREAKABLE_GLASS => (c"models/glassgibs.mdl", &[c"debris/bustglass1.wav", c"debris/bustglass2.wav"]),
                mat::METAL => (c"models/metalplategibs.mdl", &[c"debris/bustmetal1.wav", c"debris/bustmetal2.wav"]),
                mat::CINDER => (c"models/cindergibs.mdl", &[c"debris/bustconcrete1.wav", c"debris/bustconcrete2.wav"]),
                mat::CEILING => (c"models/ceilinggibs.mdl", &[c"debris/bustceiling.wav"]),
                mat::COMPUTER => (
                    c"models/computergibs.mdl",
                    &[c"buttons/spark5.wav", c"buttons/spark6.wav", c"debris/bustmetal1.wav", c"debris/bustmetal2.wav"],
                ),
                _ => (c"models/woodgibs.mdl", &[c"debris/bustcrate1.wav", c"debris/bustcrate2.wav"]),
            };
            for snd in sounds {
                eng.precache_sound(snd);
            }
            super::breakable::material_sound_precache(self.material);
            self.gib_index = eng.precache_model(gib);
        }
    }

    fn silent(cx: &mut Cx) -> bool {
        cx.pev().spawnflags & SF_DOOR_SILENT != 0
    }

    /// `CBaseDoor::DoorTouch` (0xb6ba0): only an entity whose classname is `"player"` (any
    /// `CBasePlayer`, connected or not: `rules::has_player`).
    fn door_touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        if !has_player(other) {
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
            if has_player(act) && self.health_value != 0 {
                super::items::player_take_health(act, self.health_value as f32);
            }
            self.ls.play(cx.edict, false, false);
            self.door_go_up(cx);
        }
        true
    }

    /// `CBaseDoor::DoorGoUp` (0xb6e54). Unlike HL's SDK, TS plays the move sound whatever the
    /// toggle state, so a door reversing after `Blocked` starts it again.
    fn door_go_up(&mut self, cx: &mut Cx) {
        if !Self::silent(cx) {
            let n = s(cx.pev().noise1);
            emit_sound_slow(cx.edict, CHAN_STATIC, n, 1.0, ATTN_NORM, 0, PITCH_NORM as i32, 0.5);
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
            emit_sound_slow(me, CHAN_STATIC, n1, 0.0, 0.0, snd::STOP, PITCH_NORM as i32, 0.5);
            emit_sound_slow(me, CHAN_STATIC, n2, 1.0, ATTN_NORM, 0, PITCH_NORM as i32, 0.5);
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

    /// `CBaseDoor::DoorGoDown` (0xb7284): the move sound whatever the toggle state, as in
    /// `door_go_up`.
    fn door_go_down(&mut self, cx: &mut Cx) {
        if !Self::silent(cx) {
            let n = s(cx.pev().noise1);
            emit_sound_slow(cx.edict, CHAN_STATIC, n, 1.0, ATTN_NORM, 0, PITCH_NORM as i32, 0.5);
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
            emit_sound_slow(me, CHAN_STATIC, n1, 0.0, 0.0, snd::STOP, PITCH_NORM as i32, 0.5);
            emit_sound_slow(me, CHAN_STATIC, n2, 1.0, ATTN_NORM, 0, PITCH_NORM as i32, 0.5);
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
        if !crate::rules::entities_in_box(absmin, absmax, (FL_CLIENT | FL_FAKECLIENT) as i32, 1).is_empty() {
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
            b"movesnd" => self.move_snd = fistp16_low_byte(kv_d(value)),
            b"stopsnd" => self.stop_snd = fistp16_low_byte(kv_d(value)),
            b"healthvalue" => self.health_value = fistp16_low_byte(kv_d(value)),
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

    /// `CBaseDoor::Save` / `Restore` (0xb4eb0 / 0xb4f00; `CRotDoor` uses them too). The door
    /// overrides `Use` and `Blocked` instead of setting `m_pfnUse` / `m_pfnBlocked`, and never
    /// animates. TS's breakable-door state isn't saved: a restored door can't break.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut crate::save::Fields) {
        self.t.save_chain(cx, f, &mut crate::save::Animating::default(), false, false);
        f.table(&crate::save::tables::CBaseDoor);
        f.byte(&mut self.health_value);
        f.byte(&mut self.move_snd);
        f.byte(&mut self.stop_snd);
        self.ls.save_bytes(f);
    }

    /// `CBaseDoor::Precache` (0xb6448). `rotating` stands for the TS class the object is
    /// (`CRotDoor` for `func_door_rotating`).
    fn restored(&mut self, cx: &mut Cx) {
        self.rotating = s(cx.pev().classname) == c"func_door_rotating";
        self.precache(cx);
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

    /// `CBaseDoor::TraceAttack` (0xb57ac): the break logic, and no `AddMultiDamage`.
    fn trace_attack(&mut self, cx: &mut Cx, info: &DamageInfo) {
        self.door_damage(cx, info);
    }
    // `TakeDamage` is `CBaseEntity`'s (the trait default).
}

/// `CBaseDoor::ObjectCaps`: +use opens use-only doors.
pub fn impulse_use(v: &entvars_t) -> bool {
    v.spawnflags & SF_DOOR_USE_ONLY != 0
}

// ---- momentary_door ----------------------------------------------------------------------------------

/// `momentary_door` (`CMomentaryDoor`): a sliding door that a `USE_SET` value (0..1, normally from
/// a `momentary_rot_button`) places between its closed and open positions. `ObjectCaps`
/// (0xb87f4) is 0: not +usable.
#[derive(Default)]
pub struct MomentaryDoor {
    t: Toggle<Self>,
    /// `m_bMoveSnd` (a BYTE).
    move_snd: u8,
}

impl Mover for MomentaryDoor {
    fn toggle(&mut self) -> &mut Toggle<Self> {
        &mut self.t
    }
}

/// `CMomentaryDoor::KeyValue` (0xb81a0) `movesnd`: `atof`, truncated into a 16-bit integer by
/// `fistp` (out of range or NaN gives 0x8000), then its low byte.
fn momentary_move_snd(v: f64) -> u8 {
    fistp16_low_byte(v)
}

/// `CMomentaryDoor::Precache` (0xb800c): `doors/doormove1..8.wav`; anything else is
/// `common/null.wav`, not precached (unlike `func_door`, 9 and 10 aren't in the table).
fn momentary_move_sound(n: u8) -> Option<&'static CStr> {
    (1..=8).contains(&n).then(|| move_sound(n as i32))
}

/// `CMomentaryDoor::Spawn` (0xb7e38): the open position, `pos1 + movedir × (|movedir · (size -
/// 2)| - lip)`. The distance is summed x, y, z on the x87 stack and rounded to float once.
fn momentary_door_pos2(pos1: Vec3, movedir: Vec3, size: Vec3, lip: f32) -> Vec3 {
    let axis = |k: usize| ((size[k] as f64 - 2.0) * movedir[k] as f64).abs();
    let dist = (axis(0) + axis(1) + axis(2) - lip as f64) as f32;
    mad_x87(pos1, movedir, dist)
}

/// `CMomentaryDoor::Use` (0xb8260): the destination for `value` (clamped to at most 1; TS has no
/// lower clamp) and the speed, ten times the distance (a 0.1 s move). The x87 code keeps
/// `pos2.x - pos1.x` and the destination unrounded for the distance; `f64` stands in for that.
fn momentary_door_move(pos1: Vec3, pos2: Vec3, origin: Vec3, value: f32) -> (Vec3, f32) {
    let value = if value > 1.0 { 1.0 } else { value };
    let v = value as f64;
    let part = [(v * (pos2[0] as f64 - pos1[0] as f64)) as f32, value * (pos2[1] - pos1[1]), value * (pos2[2] - pos1[2])];
    let dest64: [f64; 3] = std::array::from_fn(|k| pos1[k] as f64 + part[k] as f64);
    let dist = dest64.iter().zip(origin).map(|(d, o)| (d - o as f64).powi(2)).sum::<f64>().sqrt() as f32;
    (dest64.map(|c| c as f32), dist * 10.0)
}

impl MomentaryDoor {
    /// `CMomentaryDoor::Precache` (0xb800c).
    fn precache(&mut self, cx: &mut Cx) {
        let snd = match momentary_move_sound(self.move_snd) {
            Some(snd) => {
                engine().precache_sound(snd);
                snd
            }
            None => c"common/null.wav",
        };
        cx.pev().noise1 = alloc(snd);
    }
}

impl Entity for MomentaryDoor {
    /// `CMomentaryDoor::KeyValue` (0xb81a0): `stopsnd` and `healthvalue` are swallowed.
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"movesnd" => self.move_snd = momentary_move_snd(kv_d(value)),
            b"stopsnd" | b"healthvalue" => {}
            _ => return self.t.key_value(key, value),
        }
        true
    }

    /// `CMomentaryDoor::Spawn` (0xb7e38). `dmg` defaults to 2 but nothing reads it: the class has
    /// no `Blocked`.
    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        let v = cx.pev();
        set_movedir(v);
        v.solid = SOLID_BSP as i32;
        v.movetype = MOVETYPE_PUSH as i32;
        let org = v.origin;
        eng.set_origin(cx.edict, org);
        set_own_model(cx);
        let v = cx.pev();
        if v.speed == 0.0 {
            v.speed = 100.0;
        }
        if v.dmg == 0.0 {
            v.dmg = 2.0;
        }
        self.t.pos1 = v.origin;
        self.t.pos2 = momentary_door_pos2(self.t.pos1, v.movedir, v.size, self.t.lip);
        if v.spawnflags & SF_DOOR_START_OPEN != 0 {
            let p2 = self.t.pos2;
            eng.set_origin(cx.edict, p2);
            self.t.pos2 = self.t.pos1;
            self.t.pos1 = cx.pev().origin;
        }
        self.t.touch = None;
        self.precache(cx);
    }

    fn think(&mut self, cx: &mut Cx) {
        run_think(self, cx);
    }

    /// `CMomentaryDoor::Save` / `Restore` (0xb7d98 / 0xb7de8): no `m_pfnUse` (`Use` is its own).
    fn save_fields(&mut self, cx: &mut Cx, f: &mut crate::save::Fields) {
        self.t.save_chain(cx, f, &mut crate::save::Animating::default(), false, false);
        f.table(&crate::save::tables::CMomentaryDoor);
        f.byte(&mut self.move_snd);
    }

    /// `CMomentaryDoor::Precache` (0xb800c).
    fn restored(&mut self, cx: &mut Cx) {
        self.precache(cx);
    }

    /// `CMomentaryDoor::Use` (0xb8260): only `USE_SET` moves it. The move sound starts when the
    /// door isn't already moving; there is no stop sound and no move-done callback.
    fn use_(&mut self, cx: &mut Cx, _activator: *mut edict_t, _caller: *mut edict_t, kind: UseType, value: f32) {
        if kind != UseType::Set {
            return;
        }
        let v = cx.pev();
        let (dest, speed) = momentary_door_move(self.t.pos1, self.t.pos2, v.origin, value);
        if speed == 0.0 {
            return;
        }
        if v.nextthink < v.ltime || v.nextthink == 0.0 {
            let n = s(v.noise1);
            emit_sound_slow(cx.edict, CHAN_STATIC, n, 1.0, ATTN_NORM, 0, PITCH_NORM as i32, 0.5);
        }
        linear_move(self, cx, dest, speed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn momentary_keys_and_sounds() {
        assert_eq!(momentary_move_snd(3.9), 3);
        assert_eq!(momentary_move_snd(-1.0), 255);
        assert_eq!(momentary_move_snd(257.0), 1);
        assert_eq!(momentary_move_snd(40000.0), 0);
        assert_eq!(momentary_move_snd(f64::NAN), 0);
        assert_eq!(momentary_move_sound(8), Some(c"doors/doormove8.wav"));
        assert_eq!(momentary_move_sound(9), None);
        assert_eq!(momentary_move_sound(0), None);
    }

    #[test]
    fn momentary_positions() {
        // A 64-unit-tall door moving up with lip 4: 66 - 2 - 4 = 60 units.
        let p2 = momentary_door_pos2([10.0, 20.0, 30.0], [0.0, 0.0, 1.0], [32.0, 8.0, 66.0], 4.0);
        assert_eq!(p2, [10.0, 20.0, 90.0]);
        let (dest, speed) = momentary_door_move([0.0; 3], [0.0, 0.0, 60.0], [0.0; 3], 0.5);
        assert_eq!(dest, [0.0, 0.0, 30.0]);
        assert_eq!(speed, 300.0);
        // Clamped above 1 only: a negative value overshoots the closed position.
        assert_eq!(momentary_door_move([0.0; 3], [0.0, 0.0, 60.0], [0.0; 3], 2.0).0, [0.0, 0.0, 60.0]);
        assert_eq!(momentary_door_move([0.0; 3], [0.0, 0.0, 60.0], [0.0; 3], -0.5).0, [0.0, 0.0, -30.0]);
        assert_eq!(momentary_door_move([0.0; 3], [0.0, 0.0, 60.0], [0.0, 0.0, 30.0], 0.5).1, 0.0);
    }
}
