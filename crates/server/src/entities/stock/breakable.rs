//! `func_breakable` and `func_pushable` (SDK func_break.cpp, TS `CBreakable` / `CPushable`).
//!
//! TS changes ported here (decomp `CBreakable.c`):
//! * keys `respawntime` (int; default 0 = respawn after 30 s, < 0 = never), `respawntarget`
//!   (fired on respawn with spawnflag 8), `gibmodel`, `spawnobject`, `explodemagnitude`;
//!   `material` >= 10 becomes wood.
//! * respawn (`BreakableRespawn`): once the timer runs out, and no client is inside the box, the
//!   breakable is rebuilt; otherwise it retries every 5 s. TS keeps the timer on a helper
//!   `info_breakable` entity; here the breakable thinks on its own `ltime` (same timing).
//! * `Die`: TS pitch/volume, glass debris via the `TSFrags` user message, other materials via
//!   `TE_BREAKMODEL` with a velocity along the last hit direction, `TSExplosion` for
//!   `explodemagnitude`, and the attacker's time scale copied into `fuser1`.
//! * `func_pushable`: the original SDK 2.3 `Move` (push only with +forward/+use, always add the
//!   toucher's velocity, clamp only when pushed) plus TS's 0.3 factor when pulling with +use.

use super::base::*;
use super::explode::{emit_sound_slow, ricochet, sparks, ts_explosion};
use crate::damage::bits as dmg;
use crate::entities::{fire_targets, remove};
use crate::messages::{self, Msg};
use crate::rules::has_player;
use crate::world::{self, Cx, DamageInfo, Entity, UseType};
use goldsrc::edict::EdictPtr;
use goldsrc::ext::entities::{ATTN_NORM, MsgWriterExt, Vec3, te, te_break_model, vec};
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::ffi::{CStr, CString};

pub const SF_BREAK_TRIGGER_ONLY: i32 = 1;
pub const SF_BREAK_TOUCH: i32 = 2;
pub const SF_BREAK_PRESSURE: i32 = 4;
/// TS: fire `respawntarget` (or `target`) when the breakable respawns.
pub const SF_BREAK_RESPAWN_TARGET: i32 = 8;
pub const SF_PUSH_BREAKABLE: i32 = 128;
pub const SF_BREAK_CROWBAR: i32 = 256;

/// TS damage bit: shotgun pellets (debris magnitude x8, door break chance).
pub const DMG_TS_SHOTGUN: i32 = 0x0200_0000;
/// TS damage bit 0x4000000 (kung fu).
pub const DMG_TS_KUNGFU: i32 = 0x0400_0000;
const DMG_POISON: i32 = 1 << 17;

pub mod mat {
    pub const GLASS: i32 = 0;
    pub const WOOD: i32 = 1;
    pub const METAL: i32 = 2;
    pub const FLESH: i32 = 3;
    pub const CINDER: i32 = 4;
    pub const CEILING: i32 = 5;
    pub const COMPUTER: i32 = 6;
    pub const UNBREAKABLE_GLASS: i32 = 7;
    pub const ROCKS: i32 = 8;
    pub const NONE: i32 = 9;
}

/// `CBreakable::pSpawnObjects` (identical in TS).
const SPAWN_OBJECTS: [&CStr; 22] = [
    c"",
    c"item_battery",
    c"item_healthkit",
    c"weapon_9mmhandgun",
    c"ammo_9mmclip",
    c"weapon_9mmAR",
    c"ammo_9mmAR",
    c"ammo_ARgrenades",
    c"weapon_shotgun",
    c"ammo_buckshot",
    c"weapon_crossbow",
    c"ammo_crossbow",
    c"weapon_357",
    c"ammo_357",
    c"weapon_rpg",
    c"ammo_rpgclip",
    c"ammo_gaussclip",
    c"weapon_handgrenade",
    c"weapon_tripmine",
    c"weapon_satchel",
    c"weapon_snark",
    c"weapon_hornetgun",
];

/// `MaterialSoundList`: bullet-impact sounds per material.
pub fn material_sound_list(m: i32) -> &'static [&'static CStr] {
    match m {
        mat::GLASS | mat::COMPUTER | mat::UNBREAKABLE_GLASS => &[c"debris/glass1.wav", c"debris/glass2.wav", c"debris/glass3.wav"],
        mat::WOOD => &[c"debris/wood1.wav", c"debris/wood2.wav", c"debris/wood3.wav"],
        mat::METAL => &[c"debris/metal1.wav", c"debris/metal2.wav", c"debris/metal3.wav"],
        mat::FLESH => &[
            c"debris/flesh1.wav",
            c"debris/flesh2.wav",
            c"debris/flesh3.wav",
            c"debris/flesh5.wav",
            c"debris/flesh6.wav",
            c"debris/flesh7.wav",
        ],
        mat::CINDER | mat::ROCKS => &[c"debris/concrete1.wav", c"debris/concrete2.wav", c"debris/concrete3.wav"],
        _ => &[],
    }
}

pub fn material_sound_precache(m: i32) {
    for s in material_sound_list(m) {
        engine().precache_sound(s);
    }
}

/// Break sounds (`Die`), debris speed multiplier and `TE_BREAKMODEL` flag per material.
pub fn break_sounds(m: i32) -> (&'static [&'static CStr], f32, i32) {
    match m {
        mat::GLASS => (&[c"debris/bustglass1.wav", c"debris/bustglass2.wav"], 1.0, te::BREAK_GLASS),
        mat::WOOD => (&[c"debris/bustcrate1.wav", c"debris/bustcrate2.wav"], 2.5, te::BREAK_WOOD),
        mat::METAL | mat::COMPUTER => (&[c"debris/bustmetal1.wav", c"debris/bustmetal2.wav"], 2.5, te::BREAK_METAL),
        mat::FLESH => (&[c"debris/bustflesh1.wav", c"debris/bustflesh2.wav"], 4.0, te::BREAK_FLESH),
        mat::CINDER | mat::ROCKS => (&[c"debris/bustconcrete1.wav", c"debris/bustconcrete2.wav"], 2.0, te::BREAK_CONCRETE),
        mat::CEILING => (&[c"debris/bustceiling.wav"], 1.0, 0),
        _ => (&[], 1.0, 0),
    }
}

/// `CBreakable::Precache` material part: break sounds and the gib model. Returns the gib index.
pub fn precache_material(m: i32, gib_override: string_t) -> i32 {
    let eng = engine();
    let gib: &'static CStr = match m {
        mat::GLASS | mat::UNBREAKABLE_GLASS => c"models/glassgibs.mdl",
        mat::WOOD => c"models/woodgibs.mdl",
        mat::METAL => c"models/metalplategibs.mdl",
        mat::FLESH => c"models/fleshgibs.mdl",
        mat::CINDER => c"models/cindergibs.mdl",
        mat::CEILING => c"models/ceilinggibs.mdl",
        mat::COMPUTER => c"models/computergibs.mdl",
        mat::ROCKS => c"models/rockgibs.mdl",
        // matNone: TS leaves the name register unset; use the wood gibs rather than garbage.
        _ => c"models/woodgibs.mdl",
    };
    if m == mat::COMPUTER {
        eng.precache_sound(c"buttons/spark5.wav");
        eng.precache_sound(c"buttons/spark6.wav");
    }
    for snd in break_sounds(if m == mat::UNBREAKABLE_GLASS { mat::GLASS } else { m }).0 {
        eng.precache_sound(snd);
    }
    material_sound_precache(m);
    if gib_override != 0 { precache_model_s(gib_override) } else { eng.precache_model(gib) }
}

/// TS break sound: random pitch 95..124 (100 for 98..102), volume `rand(0.85, 1) + extra_vol`
/// (breakables add |health|/100), capped at 1; `s` is the slow-motion pitch factor.
pub fn play_break_sound(e: *mut edict_t, m: i32, extra_vol: f32, s: f32) {
    let eng = engine();
    let r = eng.random_long(0, 29);
    let pitch = if (3..8).contains(&r) { 100 } else { 95 + r };
    let vol = (eng.random_float(0.85, 1.0) + extra_vol).min(1.0);
    let list = break_sounds(m).0;
    if !list.is_empty() {
        let i = if list.len() > 1 { eng.random_long(0, list.len() as i32 - 1) as usize } else { 0 };
        emit_sound_slow(e, CHAN_VOICE, list[i], vol, ATTN_NORM, 0, pitch, s);
    }
}

/// `TSCreateDebris`: the glass debris effect (`TSFrags`, MSG_PVS).
#[allow(clippy::too_many_arguments)]
pub fn send_glass_debris(center: Vec3, size: Vec3, hit_pos: Vec3, hit_dir: Vec3, bits: i32, magnitude: f32) {
    messages::pvs(Msg::TsFrags, center).vec3(center).vec3(size).byte(0).vec3(hit_pos).vec3(hit_dir).long(bits).coord(magnitude).end();
}

#[derive(Default)]
struct PushState {
    max_speed: f32,
    sound_time: f32,
    last_sound: usize,
}

/// A C `(int)` of a double as GCC 2.95 compiles it: truncation, 0x80000000 for NaN or out of range.
fn x87_int_f64(d: f64) -> i32 {
    if (-2_147_483_648.0..2_147_483_648.0).contains(&d) { d as i32 } else { i32::MIN }
}

/// `CPushable::Spawn`'s `(int)((maxs.y - mins.y) * (maxs.x - mins.x) * (float)skin * 0.0005)`.
fn pushable_buoyancy(skin: i32, dx: f32, dy: f32) -> i32 {
    x87_int_f64(dy as f64 * dx as f64 * (skin as f32) as f64 * 0.0005)
}

/// `func_breakable` (`PUSH = false`) and `func_pushable` (`PUSH = true`).
pub struct Breakable<const PUSH: bool> {
    delay: Delay,
    material: i32,
    explosion_directed: bool,
    gib_index: i32,
    angle: f32,
    gib_model: string_t,
    /// `m_iszSpawnObject`: TS stores the `pSpawnObjects` entry as a `string_t`.
    spawn_object: string_t,
    respawn_time: f32,
    respawn_target: CString,
    max_health: f32,
    // Last hit, for debris (TS +0xf8..+0x114).
    hit_pos: Vec3,
    hit_dir: Vec3,
    hit_damage: f32,
    hit_bits: i32,
    think: ThinkFn<Self>,
    touch: TouchFn<Self>,
    /// TS's `m_pfnThink` (`Die`, from a pressure touch). The port's own respawn timer stands for
    /// the `info_breakable` helper's think, which isn't the breakable's.
    die_think: bool,
    /// TS's `info_breakable` helper handle (+0x70) is set: `Spawn` makes one when `respawntime`
    /// is not negative. It isn't saved, so a restored breakable has none: it is removed when it
    /// breaks and `RespawnEntities` doesn't reach it.
    helper: bool,
    push: PushState,
}

impl<const PUSH: bool> Default for Breakable<PUSH> {
    fn default() -> Self {
        Self {
            delay: Delay::default(),
            material: mat::GLASS,
            explosion_directed: false,
            gib_index: 0,
            angle: 0.0,
            gib_model: 0,
            spawn_object: 0,
            respawn_time: 0.0,
            respawn_target: CString::default(),
            max_health: 0.0,
            hit_pos: [0.0; 3],
            hit_dir: [0.0; 3],
            hit_damage: 0.0,
            hit_bits: 0,
            think: None,
            touch: None,
            die_think: false,
            helper: false,
            push: PushState::default(),
        }
    }
}

pub type FuncBreakable = Breakable<false>;
pub type FuncPushable = Breakable<true>;

impl<const PUSH: bool> Breakable<PUSH> {
    fn is_breakable(&self) -> bool {
        self.material != mat::UNBREAKABLE_GLASS
    }

    fn breakable_flag(cx: &mut Cx) -> bool {
        !PUSH || cx.pev().spawnflags & SF_PUSH_BREAKABLE != 0
    }

    /// `CBreakable::Precache` (0xc0820), or `CPushable::Precache` (0xc1894): the push sounds,
    /// then the breakable's only with `SF_PUSH_BREAKABLE`.
    fn precache(&mut self, cx: &mut Cx) {
        if PUSH {
            for s in PUSH_SOUNDS {
                engine().precache_sound(s);
            }
        }
        if !Self::breakable_flag(cx) {
            return;
        }
        self.gib_index = precache_material(self.material, self.gib_model);
        if self.spawn_object != 0 {
            precache_other(s(self.spawn_object));
        }
    }

    /// `CBreakable::Spawn` (TS).
    fn breakable_spawn(&mut self, cx: &mut Cx) {
        self.precache(cx);
        let v = cx.pev();
        self.max_health = v.health;
        v.fuser1 = 1.0;
        v.takedamage = if v.spawnflags & SF_BREAK_TRIGGER_ONLY != 0 { DAMAGE_NO } else { DAMAGE_YES } as f32;
        v.solid = SOLID_BSP as i32;
        v.movetype = MOVETYPE_PUSH as i32;
        self.angle = v.angles[1];
        v.angles[1] = 0.0;
        if self.material == mat::GLASS {
            v.playerclass = 1; // lets the client put decals on glass
        }
        set_own_model(cx);
        let v = cx.pev();
        self.touch = if v.spawnflags & SF_BREAK_TRIGGER_ONLY != 0 { None } else { Some(Self::break_touch) };
        if !self.is_breakable() && v.rendermode != kRenderNormal as i32 {
            v.flags |= FL_WORLDBRUSH as i32;
        }
        if self.respawn_time >= 0.0 {
            self.helper = true;
        }
    }

    fn damage_sound(&mut self, cx: &mut Cx) {
        let eng = engine();
        let mut m = self.material;
        let pitch = if eng.random_long(0, 2) == 0 { 95 + eng.random_long(0, 34) } else { 100 };
        let vol = eng.random_float(0.75, 1.0);
        if m == mat::COMPUTER && eng.random_long(0, 1) != 0 {
            m = mat::METAL;
        }
        let list: &[&CStr] = match m {
            mat::GLASS | mat::COMPUTER | mat::UNBREAKABLE_GLASS => &[c"debris/glass1.wav", c"debris/glass2.wav", c"debris/glass3.wav"],
            mat::WOOD => &[c"debris/wood1.wav", c"debris/wood2.wav", c"debris/wood3.wav"],
            mat::METAL => &[c"debris/metal1.wav", c"debris/metal3.wav"],
            mat::FLESH => &[
                c"debris/flesh1.wav",
                c"debris/flesh2.wav",
                c"debris/flesh3.wav",
                c"debris/flesh5.wav",
                c"debris/flesh6.wav",
                c"debris/flesh7.wav",
            ],
            mat::CINDER | mat::ROCKS => &[c"debris/concrete1.wav", c"debris/concrete2.wav", c"debris/concrete3.wav"],
            _ => &[],
        };
        if !list.is_empty() {
            let i = eng.random_long(0, list.len() as i32 - 1) as usize;
            emit_sound_slow(cx.edict, CHAN_VOICE, list[i], vol, ATTN_NORM, 0, pitch, 0.5);
        }
    }

    fn break_touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        if !has_player(other) || !self.is_breakable() {
            return;
        }
        let ov = other.v();
        let (ovel, oabsmin_z) = (ov.velocity, ov.absmin[2]);
        let sf = cx.pev().spawnflags;
        if sf & SF_BREAK_TOUCH != 0 {
            let damage = vec::length(ovel) * 0.01;
            if cx.pev().health <= damage {
                self.touch = None;
                let info = DamageInfo::new(other, other, damage, dmg::CRUSH);
                self.take_damage_impl(cx, &info);
                let me = cx.edict;
                crate::damage::apply(other, &DamageInfo::new(me, me, damage * 0.25, dmg::SLASH));
            }
        }
        if sf & SF_BREAK_PRESSURE != 0 && oabsmin_z >= cx.pev().maxs[2] - 2.0 {
            self.damage_sound(cx);
            self.touch = None;
            if self.delay.delay == 0.0 {
                self.delay.delay = 0.1;
            }
            // When the touch branch above just broke it, TS still sets `Die` as the think, which
            // returns at once (already dead); its respawn timer runs on the `info_breakable`
            // helper, so the port keeps its own respawn think here.
            if cx.pev().deadflag != DEAD_DEAD as i32 {
                self.think = Some(Self::die_think);
                self.die_think = true;
                let d = self.delay.delay;
                next_think(cx, d);
            }
        }
    }

    fn die_think(&mut self, cx: &mut Cx) {
        self.die(cx, std::ptr::null_mut());
    }

    /// `CBreakable::TraceAttack` part: remember the hit, sparks/ricochets.
    fn trace_attack(&mut self, cx: &mut Cx, info: &DamageInfo) {
        let eng = engine();
        self.hit_pos = info.pos;
        self.hit_dir = if vec::is_zero(info.dir) {
            let av = if info.attacker.is_null() { [0.0; 3] } else { info.attacker.v().velocity };
            if vec::is_zero(av) { [0.0; 3] } else { vec::normalize(av) }
        } else {
            vec::normalize(info.dir)
        };
        self.hit_bits = info.bits;
        self.hit_damage = if info.bits & DMG_TS_SHOTGUN != 0 { info.damage * 8.0 } else { info.damage };
        if eng.random_long(0, 1) != 0 {
            if self.material == mat::COMPUTER {
                sparks(info.pos);
                let vol = eng.random_float(0.7, 1.0);
                let s = if eng.random_long(0, 1) == 0 { c"buttons/spark5.wav" } else { c"buttons/spark6.wav" };
                emit_sound_slow(cx.edict, CHAN_VOICE, s, vol, ATTN_NORM, 0, 100, 0.5);
            } else if self.material == mat::UNBREAKABLE_GLASS {
                ricochet(info.pos, eng.random_float(0.5, 1.5));
            }
        }
    }

    /// `CBreakable::TakeDamage` (TS). Returns true while the breakable is still standing.
    fn take_damage_impl(&mut self, cx: &mut Cx, info: &DamageInfo) -> bool {
        let mut damage = info.damage;
        // From our box centre to the inflictor's origin (the attacker's when they are the same).
        // Guard: TS reads through a NULL inflictor; the port leaves the direction alone then.
        let dir = (!info.inflictor.is_null()).then(|| {
            let v = cx.pev();
            vec::sub(info.inflictor.v().origin, vec::mad(v.absmin, v.size, 0.5))
        });
        if info.attacker == info.inflictor && has_player(info.attacker) {
            let sf = cx.pev().spawnflags;
            if sf & SF_BREAK_CROWBAR != 0 && info.bits & dmg::CLUB != 0 {
                damage = cx.pev().health;
            }
        }
        if !self.is_breakable() {
            return false;
        }
        if info.bits & dmg::CLUB != 0 {
            damage *= 2.0;
        }
        if info.bits & DMG_POISON != 0 {
            damage *= 0.1;
        }
        // `g_vecAttackDir` (0xc1532), only for a breakable one.
        if let Some(d) = dir {
            crate::damage::set_attack_dir(crate::damage::normalize(d));
        }
        let v = cx.pev();
        v.health -= damage;
        if v.health > 0.0 {
            self.damage_sound(cx);
            true
        } else {
            let eng = engine();
            if eng.cvar_float(c"developer") != 0.0 {
                let name = |e: *mut edict_t, f: fn(&entvars_t) -> string_t| {
                    if e.is_null() { "world".to_owned() } else { eng.string(f(e.v())).to_string_lossy().into_owned() }
                };
                eng.print(&format!(
                    "[ts] func_breakable #{} broken by {} ({}, damage bits {:#x})\n",
                    eng.index_of(cx.edict),
                    name(info.attacker, |v| v.netname),
                    name(info.inflictor, |v| v.classname),
                    info.bits
                ));
            }
            self.die(cx, info.attacker);
            false
        }
    }

    /// `CBreakable::Die` (TS).
    fn die(&mut self, cx: &mut Cx, attacker: *mut edict_t) {
        let eng = engine();
        let me = cx.edict;
        if cx.pev().deadflag == DEAD_DEAD as i32 {
            return;
        }
        let health = cx.pev().health;
        if !attacker.is_null() {
            cx.pev().fuser1 = attacker.v().fuser1;
        }

        play_break_sound(me, self.material, (health as i32).abs() as f32 * 0.01, 0.9);
        let (_, mult, flags) = break_sounds(self.material);

        let v = cx.pev();
        let center = vec::add(v.origin, vec::mid(v.mins, v.maxs));
        let size = v.size;
        if self.material == mat::GLASS {
            send_glass_debris(center, size, self.hit_pos, self.hit_dir, self.hit_bits, mult * self.hit_damage);
        } else {
            let vel = vec::scale(self.hit_dir, self.hit_damage * mult);
            te_break_model(MSG_PVS, center, size, vel, 10, self.gib_index, 0, 25, flags);
        }

        // Anything resting on top falls.
        let (absmin, mut absmax) = (v.absmin, v.absmax);
        absmax[2] += 8.0;
        for e in eng.entities_in_box(absmin, absmax, FL_ONGROUND as i32, 256) {
            let ov = e.v();
            ov.flags &= !(FL_ONGROUND as i32);
            ov.groundentity = std::ptr::null_mut();
        }

        cx.pev().solid = SOLID_NOT as i32;
        self.delay.use_targets(me, std::ptr::null_mut(), UseType::Toggle, 0.0);

        let v = cx.pev();
        if self.spawn_object != 0 {
            let (org, ang) = (bmodel_origin(v), v.angles);
            create_entity(s(self.spawn_object), org, ang, me);
        }
        let v = cx.pev();
        if v.impulse > 0 {
            let c = vec::mid(v.absmin, v.absmax);
            let mag = v.impulse as f32;
            let att = if attacker.is_null() { me } else { attacker };
            ts_explosion(c, me, att, mag, dmg::BLAST | dmg::NEVERGIB, mag, mag * 2.5);
        }
        let v = cx.pev();
        v.deadflag = DEAD_DEAD as i32;
        v.effects = EF_NODRAW as i32;
        v.takedamage = DAMAGE_NO as f32;
        if self.respawn_time >= 0.0 && self.helper {
            let t = if self.respawn_time == 0.0 { 30.0 } else { self.respawn_time };
            self.schedule_respawn(cx, t);
        } else {
            remove(me);
        }
    }

    fn schedule_respawn(&mut self, cx: &mut Cx, delay: f32) {
        self.think = Some(Self::respawn_think);
        next_think(cx, delay);
    }

    fn respawn_think(&mut self, cx: &mut Cx) {
        self.breakable_respawn(cx, false);
    }

    /// TS `CBreakable::BreakableRespawn(bool force)`.
    pub fn breakable_respawn(&mut self, cx: &mut Cx, force: bool) {
        let v = cx.pev();
        if !(v.deadflag == DEAD_DEAD as i32 || v.effects == EF_NODRAW as i32 || force) {
            return;
        }
        if self.respawn_time < 0.0 {
            remove(cx.edict);
            return;
        }
        let (absmin, absmax) = (v.absmin, v.absmax);
        if !crate::rules::entities_in_box(absmin, absmax, (FL_CLIENT | FL_FAKECLIENT) as i32, 1).is_empty() {
            self.schedule_respawn(cx, 5.0);
            return;
        }
        let sf = v.spawnflags;
        let me = cx.edict;
        if sf & SF_BREAK_RESPAWN_TARGET != 0 {
            if self.respawn_target.is_empty() {
                self.delay.use_targets(me, std::ptr::null_mut(), UseType::Toggle, 0.0);
            } else if self.delay.delay == 0.0 {
                fire_targets(&self.respawn_target, std::ptr::null_mut(), me, UseType::Toggle, 0.0);
            } else {
                let t = alloc(&self.respawn_target);
                sub_use_targets(me, t, self.delay.kill_target, self.delay.delay, std::ptr::null_mut(), UseType::Toggle, 0.0);
            }
        }
        let v = cx.pev();
        v.deadflag = DEAD_NO as i32;
        v.health = self.max_health;
        v.effects = 0;
        self.think = None;
        self.die_think = false;
        self.touch = None;
        self.spawn(cx);
    }

    // ---- func_pushable ----------------------------------------------------------------------------

    fn push_move(&mut self, cx: &mut Cx, other: *mut edict_t, push: bool) {
        let me = cx.edict;
        let tv = other.v();
        let v = cx.pev();
        let player = has_player(other);
        // `CPushable::Move` (0xc1c50): pulling (+use) takes 30% of the player's velocity, before
        // every use of it, the standing-on-top case included.
        let input = if player && tv.button & IN_USE as i32 != 0 { vec::scale(tv.velocity, 0.3) } else { tv.velocity };
        if tv.flags & FL_ONGROUND as i32 != 0 && tv.groundentity == me {
            if v.waterlevel > 0 {
                v.velocity[2] += input[2] * 0.1;
            }
            return;
        }
        if player && push && tv.button & (IN_FORWARD | IN_USE) as i32 == 0 {
            return;
        }
        let factor = if player {
            if tv.flags & FL_ONGROUND as i32 == 0 {
                if v.waterlevel < 1 {
                    return;
                }
                0.1
            } else {
                1.0
            }
        } else {
            0.25
        };
        v.velocity[0] += input[0] * factor;
        v.velocity[1] += input[1] * factor;
        let length = vec::length2d(v.velocity);
        let max = self.push.max_speed;
        if push && length > max {
            v.velocity[0] = v.velocity[0] * max / length;
            v.velocity[1] = v.velocity[1] * max / length;
        }
        if player {
            tv.velocity[0] = v.velocity[0];
            tv.velocity[1] = v.velocity[1];
            let now = engine().time();
            if now - self.push.sound_time > 0.7 {
                self.push.sound_time = now;
                if length > 0.0 && v.flags & FL_ONGROUND as i32 != 0 {
                    self.push.last_sound = engine().random_long(0, 2) as usize;
                    emit_sound_slow(me, CHAN_WEAPON, PUSH_SOUNDS[self.push.last_sound], 0.5, ATTN_NORM, 0, PITCH_NORM as i32, 0.5);
                } else {
                    emit_sound_slow(
                        me,
                        CHAN_WEAPON,
                        PUSH_SOUNDS[self.push.last_sound],
                        0.0,
                        0.0,
                        goldsrc::ext::entities::snd::STOP,
                        PITCH_NORM as i32,
                        0.5,
                    );
                }
            }
        }
    }
}

const PUSH_SOUNDS: [&CStr; 3] = [c"debris/pushbox1.wav", c"debris/pushbox2.wav", c"debris/pushbox3.wav"];

impl<const PUSH: bool> Entity for Breakable<PUSH> {
    fn key_value(&mut self, cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        // `CPushable::KeyValue` (0xc18f4) also has a `size` branch (hull presets), but `size` is an
        // entvars field that `DispatchKeyValue` parses first, so it never runs; the model sets the size.
        if PUSH && key == c"buoyancy" {
            // `pev->skin = (int)strtod(...)`: the same field the `skin` key sets.
            cx.pev().skin = x87_int_f64(kv_d(value));
            return true;
        }
        match key.to_bytes() {
            b"explosion" => self.explosion_directed = kv_str(value).eq_ignore_ascii_case("directed"),
            b"material" => {
                let m = kv_i(value);
                self.material = if (0..10).contains(&m) { m } else { mat::WOOD };
            }
            b"respawntime" => self.respawn_time = kv_i(value) as f32,
            b"respawntarget" => self.respawn_target = value.to_owned(),
            b"deadmodel" | b"shards" | b"lip" => {}
            b"gibmodel" => self.gib_model = alloc(value),
            b"spawnobject" => {
                let i = kv_i(value);
                if (1..SPAWN_OBJECTS.len() as i32).contains(&i) {
                    self.spawn_object = alloc(SPAWN_OBJECTS[i as usize]);
                }
            }
            b"explodemagnitude" => cx.pev().impulse = kv_i(value),
            _ => return self.delay.key_value(key, value),
        }
        true
    }

    fn spawn(&mut self, cx: &mut Cx) {
        if !PUSH {
            self.breakable_spawn(cx);
            return;
        }
        // CPushable::Spawn
        if cx.pev().spawnflags & SF_PUSH_BREAKABLE != 0 {
            self.breakable_spawn(cx);
        } else {
            self.precache(cx);
        }
        let v = cx.pev();
        v.movetype = MOVETYPE_PUSHSTEP as i32;
        v.solid = SOLID_BBOX as i32;
        set_own_model(cx);
        let v = cx.pev();
        if v.friction > 399.0 {
            v.friction = 399.0;
        }
        self.push.max_speed = 400.0 - v.friction;
        v.flags |= FL_FLOAT as i32;
        v.friction = 0.0;
        v.origin[2] += 1.0;
        let org = v.origin;
        engine().set_origin(cx.edict, org);
        let v = cx.pev();
        // `CPushable::Spawn` (0xc174c) scales `skin` (the buoyancy) in place, so each respawn of
        // a breakable pushable (which re-runs `Spawn`) scales it again, as in TS.
        v.skin = pushable_buoyancy(v.skin, v.maxs[0] - v.mins[0], v.maxs[1] - v.mins[1]);
        self.push.sound_time = 0.0;
    }

    fn think(&mut self, cx: &mut Cx) {
        if let Some(f) = self.think {
            f(self, cx);
        }
    }

    fn touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        if PUSH {
            if engine().index_of(other) != 0 {
                self.push_move(cx, other, true);
            }
            return;
        }
        if let Some(f) = self.touch {
            f(self, cx, other);
        }
    }

    fn use_(&mut self, cx: &mut Cx, activator: *mut edict_t, caller: *mut edict_t, _kind: UseType, _value: f32) {
        if PUSH {
            if !has_player(activator) {
                if Self::breakable_flag(cx) {
                    self.breakable_use(cx, activator, caller);
                }
                return;
            }
            if !vec::is_zero(activator.v().velocity) {
                self.push_move(cx, activator, false);
            }
            return;
        }
        self.breakable_use(cx, activator, caller);
    }

    /// `CBreakable::TraceAttack` (0xc1060, pushables too): the hit effects, then
    /// `CBaseEntity::TraceAttack` adds the damage.
    fn trace_attack(&mut self, cx: &mut Cx, info: &DamageInfo) {
        Breakable::<PUSH>::trace_attack(self, cx, info);
        if cx.pev().takedamage != 0.0 {
            crate::damage::multi::add(info.attacker, cx.edict, info.damage, info.bits);
        }
    }

    fn take_damage(&mut self, cx: &mut Cx, info: &DamageInfo) -> bool {
        if PUSH && !Self::breakable_flag(cx) {
            return true;
        }
        self.take_damage_impl(cx, info)
    }

    /// `CBreakable::Save` / `Restore` (0xc05ec / 0xc063c), then for `func_pushable`
    /// `CPushable::Save` / `Restore` (0xc1688 / 0xc16d8). TS's `CBreakable` skips
    /// `CBaseDelay::Save`, so the delay, kill target, `respawntime`, `respawntarget`, the spawn
    /// health and the helper link aren't saved: they come back zero. `Use` and (pushables)
    /// `Touch` are virtuals; `m_pfnTouch` is `BreakTouch` unless trigger-only (a breakable
    /// `func_pushable` sets it too, unused).
    fn save_fields(&mut self, cx: &mut Cx, f: &mut crate::save::Fields) {
        let funcs = crate::save::BaseFuncs { think: self.die_think, touch: self.touch.is_some(), use_: false, blocked: false };
        let cleared = f.base_funcs(cx, funcs);
        if cleared.think {
            self.die_think = false;
            self.think = None;
        }
        if cleared.touch {
            self.touch = None;
        }
        f.table(&crate::save::tables::CBreakable);
        f.int(&mut self.material);
        f.boolean(&mut self.explosion_directed);
        f.float(&mut self.angle);
        f.string(&mut self.gib_model);
        f.string(&mut self.spawn_object);
        if PUSH {
            f.table(&crate::save::tables::CPushable);
            f.float(&mut self.push.max_speed);
            f.float(&mut self.push.sound_time);
        }
    }

    /// `CBreakable::Precache` (0xc0820) / `CPushable::Precache` (0xc1894): the gib model index
    /// comes back from the restored material and `gibmodel`.
    fn restored(&mut self, cx: &mut Cx) {
        self.precache(cx);
    }
}

impl<const PUSH: bool> Breakable<PUSH> {
    /// `CBreakable::Use` (TS): break, with debris flying away from the activator.
    fn breakable_use(&mut self, cx: &mut Cx, activator: *mut edict_t, caller: *mut edict_t) {
        if !self.is_breakable() || cx.pev().deadflag != DEAD_NO as i32 {
            return;
        }
        let v = cx.pev();
        let mid = vec::mid(v.absmin, v.absmax);
        let from = [activator, caller].into_iter().find(|e| !e.is_null()).map(|e| vec::mid(e.v().absmin, e.v().absmax)).unwrap_or(mid);
        self.hit_pos = mid;
        let d = vec::sub(mid, from);
        self.hit_dir = vec::normalize(d);
        self.hit_bits = 0;
        self.hit_damage = vec::length(d);
        v.angles[1] = self.angle;
        // `g_vecAttackDir = v_forward` of the restored angles (0xc0fe9..0xc1047).
        let angles = v.angles;
        crate::damage::set_attack_dir(engine().make_vectors(angles).0);
        self.die(cx, std::ptr::null_mut());
    }
}

/// Respawns every broken breakable / breakable door: TS `CTSGameRules::RespawnEntities`
/// (0x77764) calls `CInfoBreakable::RespawnThink` (0x75da0) on each `info_breakable` helper,
/// which drops the helper's pending timer and calls its owner's `BreakableRespawn(false)`.
/// Helpers exist for every `func_breakable` and door with `respawntime` ≥ 0 and, since
/// `CPushable::Spawn` (0xc174c) only runs `CBreakable::Spawn` then, for breakable
/// `func_pushable`s (flag 128). The port has no helpers, so it walks the owners in index order.
pub fn respawn_all(force: bool) {
    let eng = engine();
    for i in 1..eng.max_entities() {
        let e = eng.entity_of_index(i);
        if e.is_null() || e.is_free() || !e.has_private_data() {
            continue;
        }
        world::dispatch(e, |ent, cx| {
            let any = ent as &mut dyn std::any::Any;
            if let Some(b) = any.downcast_mut::<FuncBreakable>() {
                if b.respawn_time >= 0.0 && b.helper {
                    b.breakable_respawn(cx, force);
                }
            } else if let Some(b) = any.downcast_mut::<FuncPushable>() {
                if FuncPushable::breakable_flag(cx) && b.respawn_time >= 0.0 && b.helper {
                    b.breakable_respawn(cx, force);
                }
            } else if let Some(d) = any.downcast_mut::<super::doors::Door>() {
                d.breakable_respawn(cx, force);
            }
        });
    }
}

/// `info_breakable` (`CInfoBreakable`) placed by a map. TS's breakables create these as their
/// respawn timer (`RespawnThink` 0x75da0, the port keeps that timer on the breakable itself); a
/// map-placed one has no think set and `CBaseEntity::Spawn`, so it is an inert point entity.
#[derive(Default)]
pub struct InfoBreakable;
impl Entity for InfoBreakable {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pushable_buoyancy_scales_skin_in_place() {
        // 64 x 64 box, buoyancy 20: 20 * 4096 * 0.0005 = 40.96, truncated.
        assert_eq!(pushable_buoyancy(20, 64.0, 64.0), 40);
        // A respawn re-runs `Spawn` on the scaled value.
        assert_eq!(pushable_buoyancy(40, 64.0, 64.0), 81);
        assert_eq!(x87_int_f64(-2.9), -2);
        assert_eq!(x87_int_f64(f64::NAN), i32::MIN);
        assert_eq!(x87_int_f64(1e12), i32::MIN);
    }
}
