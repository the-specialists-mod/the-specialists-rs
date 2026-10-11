//! `monster_hevsuit_dead` (`CDeadHEV`, 0xdda14..0xde5ac): the only `monster_*` TS 3.0 still
//! links. It's a posed corpse using the `CBaseMonster` dead-body code: it drops to the floor and
//! can be gibbed (`DeadTakeDamage` 0xb2a10 → `Killed` 0xb1b74 → `CallGibMonster` 0xb1a4c →
//! `GibMonster` 0xb14e0).

use super::animating::{Animating, init_bone_controllers, lookup_sequence, set_sequence_box};
use super::blood::BLOOD_COLOR_RED;
use super::effects::Gib;
use super::explode::emit_sound_slow;
use crate::damage::{DamageInfo, bits};
use crate::world::{Cx, Entity};
use goldsrc::ext::entities::{ATTN_NORM, Vec3};
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::ffi::CStr;

crate::entities::link_entities! {
    monster_hevsuit_dead => DeadHev,
}

/// `CDeadHEV::m_szPoses`, chosen by the `pose` keyvalue.
const POSES: [&str; 4] = ["deadback", "deadsitting", "deadstomach", "deadtable"];

/// `DMG_GIB_CORPSE`: damage types that can gib a corpse.
const DMG_GIB_CORPSE: i32 = bits::CRUSH | bits::FALL | bits::BLAST | bits::CLUB | (1 << 9);

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum DeadThink {
    #[default]
    None,
    /// `CorpseFallThink` until it lands.
    Fall,
    /// `SUB_FadeOut` after `FadeMonster`.
    FadeOut,
    /// `SUB_Remove`.
    Remove,
}

/// `monster_hevsuit_dead`.
#[derive(Default)]
pub struct DeadHev {
    anim: Animating,
    pose: i32,
    think: DeadThink,
    /// `MEMORY_KILLED` (`Killed` ran once).
    killed: bool,
}

impl DeadHev {
    /// `CBaseMonster::MonsterInitDead` (0xce728) with `BecomeDead` (0xb19c0).
    fn init_dead(&mut self, cx: &mut Cx) {
        let eng = engine();
        init_bone_controllers(cx.edict);
        let v = cx.pev();
        v.solid = SOLID_BBOX as i32;
        v.movetype = MOVETYPE_TOSS as i32;
        v.frame = 0.0;
        self.anim.reset_sequence_info(cx.edict);
        let v = cx.pev();
        v.framerate = 0.0;
        v.max_health = v.health;
        v.deadflag = DEAD_DEAD as i32;
        eng.set_size(cx.edict, [0.0; 3], [0.0; 3]);
        let org = cx.pev().origin;
        eng.set_origin(cx.edict, org);
        // BecomeDead
        let v = cx.pev();
        v.takedamage = DAMAGE_YES as f32;
        v.health = v.max_health * 0.5;
        v.max_health = 5.0;
        v.movetype = MOVETYPE_TOSS as i32;
        self.think = DeadThink::Fall;
        v.nextthink = eng.time() + 0.5;
    }

    /// `CBaseMonster::Killed` (0xb1b74) as it runs for a corpse, which is always gibbed
    /// (`GIB_ALWAYS`).
    fn killed(&mut self, cx: &mut Cx, attack_dir: Vec3) {
        if !self.killed {
            self.killed = true;
            emit_sound_slow(cx.edict, CHAN_WEAPON, c"common/null.wav", 1.0, ATTN_NORM, 0, 100, 0.5);
        }
        self.call_gib_monster(cx, attack_dir);
    }

    /// `CBaseMonster::CallGibMonster` (0xb1a4c) + `GibMonster` (0xb14e0); the HEV corpse has
    /// human gibs (`Classify` 4 = `CLASS_HUMAN_PASSIVE`).
    fn call_gib_monster(&mut self, cx: &mut Cx, attack_dir: Vec3) {
        let eng = engine();
        let gib = eng.cvar_float(c"violence_hgibs") != 0.0;
        let v = cx.pev();
        v.takedamage = DAMAGE_NO as f32;
        v.solid = SOLID_NOT as i32;
        if gib {
            v.effects = EF_NODRAW as i32;
            emit_sound_slow(cx.edict, CHAN_WEAPON, c"common/bodysplat.wav", 1.0, ATTN_NORM, 0, 100, 0.5);
            Gib::spawn_head_gib(cx.edict, BLOOD_COLOR_RED);
            Gib::spawn_random_gibs(cx.edict, 4, true, BLOOD_COLOR_RED, attack_dir);
        }
        let v = cx.pev();
        v.deadflag = DEAD_DEAD as i32;
        if v.health < -99.0 {
            v.health = 0.0;
        }
        let now = eng.time();
        if gib {
            // SetThink(SUB_Remove), nextthink = now.
            self.think = DeadThink::Remove;
            cx.pev().nextthink = now;
        } else {
            // FadeMonster (0xb145c) + SUB_StartFadeOut.
            let v = cx.pev();
            v.framerate = 0.0;
            v.velocity = [0.0; 3];
            v.movetype = MOVETYPE_NONE as i32;
            v.avelocity = [0.0; 3];
            v.animtime = now;
            v.effects |= EF_NOINTERP as i32;
            if v.rendermode == kRenderNormal as i32 {
                v.renderamt = 255.0;
                v.rendermode = kRenderTransTexture as i32;
            }
            v.solid = SOLID_NOT as i32;
            v.nextthink = now + 0.1;
            self.think = DeadThink::FadeOut;
        }
    }
}

impl Entity for DeadHev {
    /// `CDeadHEV::KeyValue` (0xdda14).
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"pose" => self.pose = super::base::kv_i(value),
            _ => return false,
        }
        true
    }

    /// `CDeadHEV::Spawn` (0xdda9c).
    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        eng.precache_model(c"models/player.mdl");
        eng.set_model(cx.edict, c"models/player.mdl");
        let v = cx.pev();
        v.effects = 0;
        v.yaw_speed = 8.0;
        v.sequence = 0;
        v.body = 1;
        let pose = POSES.get(self.pose as usize).copied().unwrap_or("");
        let seq = lookup_sequence(cx.edict, pose);
        let v = cx.pev();
        v.sequence = seq;
        if seq == -1 {
            eng.alert(ALERT_TYPE::at_console, "Dead hevsuit with bad pose\n");
            v.sequence = 0;
            v.effects = EF_BRIGHTFIELD as i32;
        }
        v.health = 8.0;
        self.init_dead(cx);
    }

    fn think(&mut self, cx: &mut Cx) {
        let now = engine().time();
        match self.think {
            DeadThink::None => {}
            // CBaseMonster::CorpseFallThink (0xce6bc)
            DeadThink::Fall => {
                if cx.pev().flags & FL_ONGROUND as i32 != 0 {
                    self.think = DeadThink::None;
                    set_sequence_box(cx.edict);
                    let org = cx.pev().origin;
                    engine().set_origin(cx.edict, org);
                } else {
                    cx.pev().nextthink = now + 0.1;
                }
            }
            // CBaseEntity::SUB_FadeOut
            DeadThink::FadeOut => {
                let v = cx.pev();
                if v.renderamt > 7.0 {
                    v.renderamt -= 7.0;
                    v.nextthink = now + 0.1;
                } else {
                    v.renderamt = 0.0;
                    v.nextthink = now + 0.2;
                    self.think = DeadThink::Remove;
                }
            }
            DeadThink::Remove => super::base::sub_remove(cx.edict),
        }
    }

    /// `CBaseMonster::TakeDamage` (0xb2430) on a dead monster → `DeadTakeDamage` (0xb2a10):
    /// gib-capable damage wears the corpse down (10 % per hit) and gibs it once a hit is at least
    /// its remaining health.
    fn take_damage(&mut self, cx: &mut Cx, info: &DamageInfo) -> bool {
        if cx.pev().takedamage == 0.0 {
            return false;
        }
        // g_vecAttackDir, from our centre towards just below the inflictor's (0xb2a6c..0xb2b9c),
        // the same as a live monster's; without a real inflictor it keeps the last hit's.
        crate::damage::update_attack_dir(info.inflictor, cx.pev());
        if info.bits & DMG_GIB_CORPSE != 0 {
            let v = cx.pev();
            if v.health <= info.damage {
                v.health = -50.0;
                // The gibs fly against `g_vecAttackDir` (`CGib::SpawnRandomGibs` reads it).
                self.killed(cx, crate::damage::attack_dir());
                return false;
            }
            v.health -= info.damage * 0.1;
        }
        true
    }
}
