//! `monster_hevsuit_dead` (`CDeadHEV`, 0xdda14..0xde5ac): the only `monster_*` TS 3.0 still
//! links. It's a posed corpse using the `CBaseMonster` dead-body code: it drops to the floor and
//! can be gibbed (`DeadTakeDamage` 0xb2a10 → `Killed` 0xb1b74 → `CallGibMonster` 0xb1a4c →
//! `GibMonster` 0xb14e0). [`MonsterBase`] is that `CBaseMonster` damage and death code, shared
//! with the other AI-less classes TS derives from `CBaseMonster` (`env_explosion`,
//! `cycler_wreckage`).

use super::animating::{Animating, init_bone_controllers, lookup_sequence, set_sequence_box};
use super::blood::BLOOD_COLOR_RED;
use super::effects::Gib;
use super::explode::emit_sound_slow;
use crate::damage::{DamageInfo, bits};
use crate::world::{Cx, Entity};
use goldsrc::ext::entities::ATTN_NORM;
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

/// `Killed`'s `iGib`: `GIB_NORMAL`, `GIB_NEVER`, `GIB_ALWAYS`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GibType {
    Normal,
    Never,
    Always,
}

impl GibType {
    /// `CBaseMonster::TakeDamage`'s choice from the damage bits: `DMG_ALWAYSGIB` first, then
    /// `DMG_NEVERGIB`.
    pub fn from_bits(b: i32) -> Self {
        if b & bits::ALWAYSGIB != 0 {
            GibType::Always
        } else if b & bits::NEVERGIB != 0 {
            GibType::Never
        } else {
            GibType::Normal
        }
    }
}

/// `CBaseMonster::ShouldGibMonster` (0xb1a0c).
fn should_gib(gib: GibType, health: f32) -> bool {
    (gib == GibType::Normal && health < -30.0) || gib == GibType::Always
}

/// `CBaseMonster::HasHumanGibs` (0xb13e0) / `HasAlienGibs` (0xb1418) by `Classify`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Gibs {
    Human,
    Alien,
    None,
}

fn gibs_of(classify: i32) -> Gibs {
    match classify {
        2 | 3 | 4 | 11 => Gibs::Human,
        5..=10 => Gibs::Alien,
        _ => Gibs::None,
    }
}

/// The think a `CBaseMonster` death sets (`SetThink`). A class whose `Think` is
/// `CBaseEntity::Think` runs it in place of its own until it sets another; `cycler_wreckage`
/// overrides `Think` itself, so it never runs there.
#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum MonsterThink {
    #[default]
    None,
    /// `SUB_FadeOut` after `SUB_StartFadeOut`.
    FadeOut,
    /// `SUB_Remove`.
    Remove,
}

/// The `CBaseMonster` damage and death code of the classes TS derives from it without AI
/// (`env_explosion`, `cycler_wreckage`, `monster_hevsuit_dead`): their vtables keep
/// `CBaseMonster::TakeDamage` (0xb2430), `Killed` (0xb1b74), `IsAlive`, the gib and fade
/// functions, the empty `PainSound`/`MakeIdealYaw`/`DeathNotice` and `ShouldFadeOnDeath` 0.
/// `classify` is the class's `Classify` (`CBaseEntity`'s 0, `CDeadHEV`'s 4) and `blood` its
/// `BloodColor` (`m_bloodColor`).
#[derive(Default)]
pub struct MonsterBase {
    /// `bits_MEMORY_KILLED` (`Killed` ran once).
    killed: bool,
    pub think: MonsterThink,
}

impl MonsterBase {
    /// `CBaseMonster::TakeDamage` (0xb2430). Refused with `takedamage` off; a dead one
    /// (`deadflag` `DEAD_DEAD`: not `IsAlive`) goes to `DeadTakeDamage`. Otherwise
    /// `g_vecAttackDir` comes from the inflictor and the health drops; at 0 or below `Killed`
    /// runs (gib type from the damage bits) and it returns 0. The rest only feeds AI these
    /// classes don't have: `m_bitsDamageType`, the conditions and `m_vecEnemyLKP` (the
    /// `FL_MONSTER` branch ends in TS's empty `MakeIdealYaw`); `m_MonsterState` is never
    /// `MONSTERSTATE_SCRIPT`, and `IsPlayer` is false. TS dropped the SDK's push of a
    /// `MOVETYPE_WALK` victim (only its `ENTINDEX` call is left).
    pub fn take_damage(&mut self, cx: &mut Cx, info: &DamageInfo, classify: i32, blood: i32) -> bool {
        if cx.pev().takedamage == 0.0 {
            return false;
        }
        if cx.pev().deadflag == DEAD_DEAD as i32 {
            return self.dead_take_damage(cx, info, classify, blood);
        }
        crate::damage::update_attack_dir(info.inflictor, cx.pev());
        let v = cx.pev();
        v.health -= info.damage;
        if v.health <= 0.0 {
            self.killed(cx, GibType::from_bits(info.bits), classify, blood);
            return false;
        }
        true
    }

    /// `CBaseMonster::DeadTakeDamage` (0xb2a10): `g_vecAttackDir` as for a live monster (without
    /// a real inflictor it keeps the last hit's); gib-capable damage wears the corpse down (10 %
    /// per hit) and gibs it once a hit is at least its remaining health.
    fn dead_take_damage(&mut self, cx: &mut Cx, info: &DamageInfo, classify: i32, blood: i32) -> bool {
        crate::damage::update_attack_dir(info.inflictor, cx.pev());
        if info.bits & DMG_GIB_CORPSE != 0 {
            let v = cx.pev();
            if v.health <= info.damage {
                v.health = -50.0;
                self.killed(cx, GibType::Always, classify, blood);
                return false;
            }
            v.health -= info.damage * 0.1;
        }
        true
    }

    /// `CBaseMonster::Killed` (0xb1b74). The first time: `common/null.wav`, the owner's (or the
    /// world's) `DeathNotice` (every TS class has `CBaseEntity`'s empty one), then without a gib
    /// an `FL_MONSTER` entity `BecomeDead`s (0xb19c0) and health below -99 becomes 0. Later
    /// calls only gib.
    pub fn killed(&mut self, cx: &mut Cx, gib: GibType, classify: i32, blood: i32) {
        if !self.killed {
            self.killed = true;
            emit_sound_slow(cx.edict, CHAN_WEAPON, c"common/null.wav", 1.0, ATTN_NORM, 0, 100, 0.5);
            if !should_gib(gib, cx.pev().health) {
                let v = cx.pev();
                if v.flags & FL_MONSTER as i32 != 0 {
                    // SetTouch(NULL) (none of these classes has a touch), BecomeDead.
                    v.takedamage = DAMAGE_YES as f32;
                    v.health = v.max_health * 0.5;
                    v.max_health = 5.0;
                    v.movetype = MOVETYPE_TOSS as i32;
                }
                if v.health < -99.0 {
                    v.health = 0.0;
                }
                return;
            }
        } else if !should_gib(gib, cx.pev().health) {
            return;
        }
        self.call_gib_monster(cx, classify, blood);
    }

    /// `CBaseMonster::CallGibMonster` (0xb1a4c): human or alien gibs fade instead when their
    /// `violence_*gibs` is 0; otherwise `EF_NODRAW` and `GibMonster`.
    fn call_gib_monster(&mut self, cx: &mut Cx, classify: i32, blood: i32) {
        let eng = engine();
        let gibs = gibs_of(classify);
        let fade = match gibs {
            Gibs::Human => eng.cvar_float(c"violence_hgibs") == 0.0,
            Gibs::Alien => eng.cvar_float(c"violence_agibs") == 0.0,
            Gibs::None => false,
        };
        let v = cx.pev();
        v.takedamage = DAMAGE_NO as f32;
        v.solid = SOLID_NOT as i32;
        if fade {
            self.fade_monster(cx);
        } else {
            cx.pev().effects = EF_NODRAW as i32;
            self.gib_monster(cx, gibs, blood);
        }
        let v = cx.pev();
        v.deadflag = DEAD_DEAD as i32;
        if v.health < -99.0 {
            v.health = 0.0;
        }
    }

    /// `CBaseMonster::GibMonster` (0xb14e0): the splat, the gibs its `violence_*gibs` allows, then
    /// `SUB_Remove` at once for a class with gibs, else `FadeMonster` (`Classify` 0).
    fn gib_monster(&mut self, cx: &mut Cx, gibs: Gibs, blood: i32) {
        let eng = engine();
        emit_sound_slow(cx.edict, CHAN_WEAPON, c"common/bodysplat.wav", 1.0, ATTN_NORM, 0, 100, 0.5);
        match gibs {
            Gibs::Human => {
                if eng.cvar_float(c"violence_hgibs") != 0.0 {
                    Gib::spawn_head_gib(cx.edict, blood);
                    // The gibs fly against `g_vecAttackDir` (`CGib::SpawnRandomGibs` reads it).
                    Gib::spawn_random_gibs(cx.edict, 4, true, blood, crate::damage::attack_dir());
                }
            }
            Gibs::Alien => {
                if eng.cvar_float(c"violence_agibs") != 0.0 {
                    Gib::spawn_random_gibs(cx.edict, 4, false, blood, crate::damage::attack_dir());
                }
            }
            Gibs::None => {
                self.fade_monster(cx);
                return;
            }
        }
        self.think = MonsterThink::Remove;
        cx.pev().nextthink = eng.time();
    }

    /// `CBaseMonster::FadeMonster` (0xb145c) and `CBaseEntity::SUB_StartFadeOut` (0xb1cb0).
    fn fade_monster(&mut self, cx: &mut Cx) {
        let now = engine().time();
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
        self.think = MonsterThink::FadeOut;
    }

    /// Runs the death think if one is set (`SUB_FadeOut` 0xb1d44, `SUB_Remove`); false when the
    /// class's own think should run.
    pub fn run_think(&mut self, cx: &mut Cx) -> bool {
        match self.think {
            MonsterThink::None => return false,
            MonsterThink::FadeOut => {
                let now = engine().time();
                let v = cx.pev();
                if v.renderamt <= 7.0 {
                    v.renderamt = 0.0;
                    v.nextthink = now + 0.2;
                    self.think = MonsterThink::Remove;
                } else {
                    v.renderamt -= 7.0;
                    v.nextthink = now + 0.1;
                }
            }
            MonsterThink::Remove => super::base::sub_remove(cx.edict),
        }
        true
    }
}

/// `CDeadHEV::Classify` (0xde5ac): `CLASS_HUMAN_PASSIVE`.
const DEAD_HEV_CLASSIFY: i32 = 4;

/// `monster_hevsuit_dead`.
#[derive(Default)]
pub struct DeadHev {
    anim: Animating,
    pose: i32,
    /// `CorpseFallThink` until it lands.
    falling: bool,
    monster: MonsterBase,
    /// The `CBaseDelay` / `CBaseToggle` members: only their keys (`CDeadHEV::KeyValue` passes the
    /// rest to `CBaseMonster::KeyValue` 0xce80c, which is `CBaseToggle::KeyValue`) ever set them,
    /// and nothing reads them back, but TS saves them.
    toggle: super::base::Toggle<DeadHev>,
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
        self.falling = true;
        v.nextthink = eng.time() + 0.5;
    }
}

impl Entity for DeadHev {
    /// `CDeadHEV::KeyValue` (0xdda14).
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"pose" => self.pose = super::base::kv_i(value),
            _ => return self.toggle.key_value(key, value),
        }
        true
    }

    /// `CBaseToggle::Save` (0xe3494); `CBaseMonster` saves nothing in TS. The think pointer is
    /// set while it falls (`MonsterInitDead` 0xce728 → `CorpseFallThink`, cleared on landing
    /// 0xce6bc) and from a fade or gib (`SUB_StartFadeOut`/`SUB_FadeOut`/`SUB_Remove`);
    /// `Killed`'s `SetTouch(NULL)` leaves the touch unset. A restored think never runs.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut crate::save::Fields) {
        let think = self.falling || self.monster.think != MonsterThink::None;
        if f.base_funcs(cx, crate::save::BaseFuncs { think, ..Default::default() }).think {
            self.falling = false;
            self.monster.think = MonsterThink::None;
        }
        f.delay(&mut self.toggle.delay.delay, &mut self.toggle.delay.kill_target);
        self.anim.save_fields(f);
        self.toggle.save_toggle(f);
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
        if self.monster.run_think(cx) || !self.falling {
            return;
        }
        // CBaseMonster::CorpseFallThink (0xce6bc)
        if cx.pev().flags & FL_ONGROUND as i32 != 0 {
            self.falling = false;
            set_sequence_box(cx.edict);
            let org = cx.pev().origin;
            engine().set_origin(cx.edict, org);
        } else {
            cx.pev().nextthink = engine().time() + 0.1;
        }
    }

    /// `CBaseMonster::TakeDamage` (0xb2430): the corpse is dead, so `DeadTakeDamage`.
    fn take_damage(&mut self, cx: &mut Cx, info: &DamageInfo) -> bool {
        self.monster.take_damage(cx, info, DEAD_HEV_CLASSIFY, BLOOD_COLOR_RED)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gib_type_from_damage_bits() {
        assert_eq!(GibType::from_bits(bits::BULLET), GibType::Normal);
        assert_eq!(GibType::from_bits(bits::NEVERGIB), GibType::Never);
        assert_eq!(GibType::from_bits(bits::ALWAYSGIB), GibType::Always);
        // DMG_ALWAYSGIB is tested first.
        assert_eq!(GibType::from_bits(bits::ALWAYSGIB | bits::NEVERGIB), GibType::Always);
    }

    #[test]
    fn should_gib_monster() {
        assert!(!should_gib(GibType::Normal, -30.0));
        assert!(should_gib(GibType::Normal, -30.5));
        assert!(!should_gib(GibType::Never, -100.0));
        assert!(should_gib(GibType::Always, 50.0));
    }

    #[test]
    fn gibs_by_classify() {
        assert_eq!(gibs_of(0), Gibs::None);
        assert_eq!(gibs_of(1), Gibs::None);
        for c in [2, 3, 4, 11] {
            assert_eq!(gibs_of(c), Gibs::Human);
        }
        for c in 5..=10 {
            assert_eq!(gibs_of(c), Gibs::Alien);
        }
        assert_eq!(gibs_of(12), Gibs::None);
        assert_eq!(gibs_of(DEAD_HEV_CLASSIFY), Gibs::Human);
    }
}
