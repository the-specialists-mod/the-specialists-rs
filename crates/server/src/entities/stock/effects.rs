//! Effect entities (SDK effects.cpp / explode.cpp / world.cpp / h_cycler.cpp; same in TS except
//! screen shake/fade, which go through TS's time-scaled `UTIL_ScreenShake` / `TSFade`, and
//! env_fade's brush form with spawnflag 8): `env_sprite`, `env_glow`, `env_shake`, `env_fade`,
//! `env_explosion` (+ `spark_shower`), `env_render`, `env_beam`, `env_laser`, `env_shooter`,
//! `gibshooter` (+ `gib`), `env_spark`, `env_bubbles`, `infodecal`, `cycler_sprite`, `cycler`.

use super::animating::{Animating, init_bone_controllers};
use super::base::*;
use super::blood::{BLOOD_COLOR_RED, DONT_BLEED, blood_decal_trace};
use super::breakable::{mat, material_sound_list, material_sound_precache};
use super::buttons::{do_spark, precache_sparks};
use super::explode::{emit_sound_slow, radius_damage, screen_fade, screen_fade_all, screen_shake, sparks};
use crate::entities::remove;
use crate::world::{self, Cx, DamageInfo, Entity, UseType};
use goldsrc::edict::EdictPtr;
use goldsrc::ext::entities::{
    ATTN_NORM, EHandle, MsgWriterExt, SVC_TEMPENTITY, Vec3, is_player, te, te_begin, te_explosion, te_smoke, vec,
};
use goldsrc::server::{DONT_IGNORE_MONSTERS, IGNORE_MONSTERS, MsgWriter, engine};
use goldsrc::sys::*;
use std::ffi::CStr;

// ---- env_sprite ---------------------------------------------------------------------------------------

pub const SF_SPRITE_STARTON: i32 = 1;
pub const SF_SPRITE_ONCE: i32 = 2;
pub const SF_SPRITE_TEMPORARY: i32 = 0x8000;

#[derive(Default)]
pub struct Sprite {
    max_frame: f32,
    last_time: f32,
    animating: bool,
    /// `AnimateUntilDead` think (set by [`Sprite::animate_and_die`]).
    until_dead: bool,
}

impl Sprite {
    fn animate(&mut self, cx: &mut Cx, frames: f32) {
        let v = cx.pev();
        v.frame += frames;
        if v.frame > self.max_frame {
            if v.spawnflags & SF_SPRITE_ONCE != 0 {
                self.turn_off(cx);
            } else if self.max_frame > 0.0 {
                v.frame %= self.max_frame;
            }
        }
    }

    fn turn_off(&mut self, cx: &mut Cx) {
        let v = cx.pev();
        v.effects = EF_NODRAW as i32;
        v.nextthink = 0.0;
        self.animating = false;
    }

    fn turn_on(&mut self, cx: &mut Cx) {
        let now = engine().time();
        let v = cx.pev();
        v.effects = 0;
        if (v.framerate != 0.0 && self.max_frame > 1.0) || v.spawnflags & SF_SPRITE_ONCE != 0 {
            self.animating = true;
            v.nextthink = now;
            self.last_time = now;
        }
        v.frame = 0.0;
    }

    /// `CSprite::SpriteCreate(name, origin, animate)`: a temporary env_sprite.
    pub fn create(name: &CStr, origin: Vec3, animate: bool) -> *mut edict_t {
        let eng = engine();
        let e = eng.create_named_entity(c"env_sprite");
        if e.is_null() {
            return e;
        }
        let v = e.v();
        v.model = alloc(name);
        v.origin = origin;
        world::dispatch(e, |ent, cx| ent.spawn(cx));
        let v = e.v();
        v.solid = SOLID_NOT as i32;
        v.movetype = MOVETYPE_NOCLIP as i32;
        v.spawnflags |= SF_SPRITE_TEMPORARY;
        if animate {
            world::dispatch(e, |ent, cx| ent.use_(cx, std::ptr::null_mut(), std::ptr::null_mut(), UseType::On, 0.0));
        }
        e
    }

    /// `CSprite::AnimateAndDie(framerate)`: play once at `framerate`, then remove itself
    /// (`AnimateUntilDead`, removed once `dmgtime` passes).
    pub fn animate_and_die(e: *mut edict_t, framerate: f32) {
        let now = engine().time();
        let Some(max_frame) = world::with::<Sprite, _>(e, |sp| {
            sp.until_dead = true;
            sp.max_frame
        }) else {
            return;
        };
        let v = e.v();
        v.framerate = framerate;
        v.dmgtime = now + max_frame / framerate;
        v.nextthink = now;
    }
}

/// `CSprite::SetAttachment` (0xbec08): attach sprite `e` to `owner`'s model attachment
/// `attachment` (1-based on the client).
pub fn sprite_set_attachment(e: *mut edict_t, owner: *mut edict_t, attachment: i32) {
    if owner.is_null() || e.is_null() {
        return;
    }
    let index = engine().index_of(owner);
    let v = e.v();
    v.skin = index;
    v.body = attachment;
    v.aiment = owner;
    v.movetype = MOVETYPE_FOLLOW as i32;
}

/// `CSprite::SetTransparency` (0xbec74): render mode, colour and amount (int parameters, so
/// callers passing floats truncate them).
pub fn sprite_set_transparency(e: *mut edict_t, mode: i32, color: Vec3, amount: f32, fx: i32) {
    let v = e.v();
    v.rendermode = mode;
    v.rendercolor = color.map(|c| c as i32 as f32);
    v.renderamt = amount as i32 as f32;
    v.renderfx = fx;
}

impl Entity for Sprite {
    fn spawn(&mut self, cx: &mut Cx) {
        let v = cx.pev();
        v.solid = SOLID_NOT as i32;
        v.movetype = MOVETYPE_NONE as i32;
        v.effects = 0;
        v.frame = 0.0;
        // CSprite::Precache (0xbbc10): a sprite with an `aiment` follows that entity's attachment
        // `body` (the client reads skin = entity index, body = attachment).
        precache_model_s(v.model);
        if v.aiment.is_null() {
            v.skin = 0;
            v.body = 0;
        } else {
            v.skin = engine().index_of(v.aiment);
            v.movetype = MOVETYPE_FOLLOW as i32;
        }
        set_own_model(cx);
        let v = cx.pev();
        self.max_frame = engine().model_frames(v.modelindex) as f32 - 1.0;
        if v.targetname != 0 && v.spawnflags & SF_SPRITE_STARTON == 0 {
            self.turn_off(cx);
        } else {
            self.turn_on(cx);
        }
        let v = cx.pev();
        if v.angles[1] != 0.0 && v.angles[2] == 0.0 {
            v.angles[2] = v.angles[1];
            v.angles[1] = 0.0;
        }
    }

    fn think(&mut self, cx: &mut Cx) {
        let now = engine().time();
        if self.until_dead {
            if now > cx.pev().dmgtime {
                remove(cx.edict);
                return;
            }
            // AnimateThink, then think again next frame.
            let fr = cx.pev().framerate;
            self.animate(cx, fr * (now - self.last_time));
            self.last_time = now;
            cx.pev().nextthink = now;
            return;
        }
        if !self.animating {
            return;
        }
        let fr = cx.pev().framerate;
        self.animate(cx, fr * (now - self.last_time));
        if self.animating {
            cx.pev().nextthink = now + 0.1;
        }
        self.last_time = now;
    }

    fn use_(&mut self, cx: &mut Cx, _a: *mut edict_t, _c: *mut edict_t, kind: UseType, _v: f32) {
        let on = cx.pev().effects != EF_NODRAW as i32;
        if should_toggle(kind, on) {
            if on {
                self.turn_off(cx);
            } else {
                self.turn_on(cx);
            }
        }
    }
}

// ---- env_glow ---------------------------------------------------------------------------------------------

#[derive(Default)]
pub struct Glow {
    max_frame: f32,
    last_time: f32,
}

impl Entity for Glow {
    fn spawn(&mut self, cx: &mut Cx) {
        let now = engine().time();
        let v = cx.pev();
        v.solid = SOLID_NOT as i32;
        v.movetype = MOVETYPE_NONE as i32;
        v.effects = 0;
        v.frame = 0.0;
        precache_model_s(v.model);
        set_own_model(cx);
        let v = cx.pev();
        self.max_frame = engine().model_frames(v.modelindex) as f32 - 1.0;
        if self.max_frame > 1.0 && v.framerate != 0.0 {
            v.nextthink = now + 0.1;
        }
        self.last_time = now;
    }

    fn think(&mut self, cx: &mut Cx) {
        let now = engine().time();
        let v = cx.pev();
        if self.max_frame > 0.0 {
            v.frame = (v.frame + v.framerate * (now - self.last_time)) % self.max_frame;
        }
        v.nextthink = now + 0.1;
        self.last_time = now;
    }
}

// ---- env_shake / env_fade ---------------------------------------------------------------------------------

pub const SF_SHAKE_EVERYONE: i32 = 1;

/// `env_shake`: amplitude in `scale`, frequency `dmg_save`, duration `dmg_take`, radius `dmg`.
#[derive(Default)]
pub struct Shake;

impl Entity for Shake {
    fn key_value(&mut self, cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        let v = cx.pev();
        match key.to_bytes() {
            b"amplitude" => v.scale = kv_f(value),
            b"frequency" => v.dmg_save = kv_f(value),
            b"duration" => v.dmg_take = kv_f(value),
            b"radius" => v.dmg = kv_f(value),
            _ => return false,
        }
        true
    }

    fn spawn(&mut self, cx: &mut Cx) {
        let v = cx.pev();
        v.solid = SOLID_NOT as i32;
        v.movetype = MOVETYPE_NONE as i32;
        v.effects = 0;
        v.frame = 0.0;
        if v.spawnflags & SF_SHAKE_EVERYONE != 0 {
            v.dmg = 0.0;
        }
    }

    fn use_(&mut self, cx: &mut Cx, _a: *mut edict_t, _c: *mut edict_t, _k: UseType, _v: f32) {
        let v = cx.pev();
        screen_shake(v.origin, v.scale, v.dmg_save, v.dmg_take, v.dmg);
    }
}

pub const SF_FADE_IN: i32 = 1;
pub const SF_FADE_MODULATE: i32 = 2;
pub const SF_FADE_ONLYONE: i32 = 4;
/// TS: fade only the clients inside the entity's brush.
pub const SF_FADE_INBOX: i32 = 8;

/// `env_fade` (TS): duration in `dmg_take`, hold in `dmg_save`; sent as `TSFade`.
#[derive(Default)]
pub struct Fade {
    delay: Delay,
}

impl Entity for Fade {
    fn key_value(&mut self, cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"duration" => cx.pev().dmg_take = kv_f(value),
            b"holdtime" => cx.pev().dmg_save = kv_f(value),
            _ => return self.delay.key_value(key, value),
        }
        true
    }

    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        // TS spawns it like a brush so spawnflag 8 has a box to test.
        let v = cx.pev();
        if v.model != 0 {
            set_own_model(cx);
        }
        let v = cx.pev();
        let (mins, maxs, org) = (v.mins, v.maxs, v.origin);
        eng.set_size(cx.edict, mins, maxs);
        eng.set_origin(cx.edict, org);
        let v = cx.pev();
        v.effects |= EF_NODRAW as i32;
        v.movetype = MOVETYPE_FLY as i32;
        v.solid = SOLID_NOT as i32;
    }

    fn use_(&mut self, cx: &mut Cx, activator: *mut edict_t, _c: *mut edict_t, _k: UseType, _v: f32) {
        let v = cx.pev();
        let mut flags = 0;
        if v.spawnflags & SF_FADE_IN == 0 {
            flags |= super::explode::FFADE_OUT;
        }
        if v.spawnflags & SF_FADE_MODULATE != 0 {
            flags |= super::explode::FFADE_MODULATE;
        }
        let (color, dur, hold, alpha) = (v.rendercolor, v.dmg_take, v.dmg_save, v.renderamt as i32);
        if v.spawnflags & SF_FADE_INBOX != 0 {
            let (lo, hi) = (v.absmin, v.absmax);
            for p in engine().entities_in_box(lo, hi, FL_CLIENT as i32, 32) {
                screen_fade(p, color, dur, hold, alpha, flags);
            }
        } else if v.spawnflags & SF_FADE_ONLYONE != 0 {
            screen_fade(activator, color, dur, hold, alpha, flags);
        } else {
            screen_fade_all(color, dur, hold, alpha, flags);
        }
        let me = cx.edict;
        self.delay.use_targets(me, me, UseType::Toggle, 0.0);
    }
}

// ---- env_explosion / spark_shower -------------------------------------------------------------------------

pub const SF_ENVEXPLOSION_NODAMAGE: i32 = 1;
pub const SF_ENVEXPLOSION_REPEATABLE: i32 = 2;
pub const SF_ENVEXPLOSION_NOFIREBALL: i32 = 4;
pub const SF_ENVEXPLOSION_NOSMOKE: i32 = 8;
pub const SF_ENVEXPLOSION_NODECAL: i32 = 16;
pub const SF_ENVEXPLOSION_NOSPARKS: i32 = 32;

/// `UTIL_DecalTrace`: world or brush-entity decal at a trace end.
pub fn decal_trace(tr: &TraceResult, decal_name: &CStr) {
    if tr.flFraction == 1.0 {
        return;
    }
    let eng = engine();
    let mut index = eng.decal_index(decal_name);
    if index < 0 {
        return;
    }
    let mut entity = 0;
    if !tr.pHit.is_null() {
        let hv = tr.pHit.v();
        if hv.solid != SOLID_BSP as i32 && hv.movetype != MOVETYPE_PUSHSTEP as i32 {
            return;
        }
        entity = eng.index_of(tr.pHit);
    }
    let msg = if entity != 0 {
        if index > 255 {
            index -= 256;
            te::DECALHIGH
        } else {
            te::DECAL
        }
    } else if index > 255 {
        index -= 256;
        te::WORLDDECALHIGH
    } else {
        te::WORLDDECAL
    };
    let w = te_begin(MSG_BROADCAST, None, msg).vec3(tr.vecEndPos).byte(index);
    let w = if entity != 0 { w.short(entity) } else { w };
    w.end();
}

#[derive(Default)]
pub struct EnvExplosion {
    magnitude: i32,
    sprite_scale: i32,
    fireball: i32,
    smoke: i32,
    smoking: bool,
    /// `CEnvExplosion` is a `CBaseMonster`: TS's vtable has `CBaseMonster::TakeDamage` and
    /// `Killed`, reachable when a map sets `takedamage`.
    monster: super::monsters::MonsterBase,
}

impl Entity for EnvExplosion {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        if key == c"iMagnitude" {
            self.magnitude = kv_i(value);
            return true;
        }
        false
    }

    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        // Precached globally by the SDK's W_Precache.
        self.fireball = eng.precache_model(c"sprites/zerogxplode.spr");
        self.smoke = eng.precache_model(c"sprites/steam1.spr");
        eng.precache_model(c"models/grenade.mdl");
        let v = cx.pev();
        v.solid = SOLID_NOT as i32;
        v.effects = EF_NODRAW as i32;
        v.movetype = MOVETYPE_NONE as i32;
        self.sprite_scale = (((self.magnitude - 50) as f32 * 0.6).max(10.0)) as i32;
    }

    fn use_(&mut self, cx: &mut Cx, _a: *mut edict_t, _c: *mut edict_t, _k: UseType, _v: f32) {
        let eng = engine();
        let me = cx.edict;
        let v = cx.pev();
        v.model = 0;
        v.solid = SOLID_NOT as i32;
        let spot = vec::add(v.origin, [0.0, 0.0, 8.0]);
        let tr = eng.trace_line(spot, vec::add(spot, [0.0, 0.0, -40.0]), IGNORE_MONSTERS, me);
        if tr.flFraction != 1.0 {
            cx.pev().origin = vec::mad(tr.vecEndPos, tr.vecPlaneNormal, (self.magnitude - 24) as f32 * 0.6);
        }
        let v = cx.pev();
        let (org, sf) = (v.origin, v.spawnflags);
        if sf & SF_ENVEXPLOSION_NODECAL == 0 {
            decal_trace(&tr, if eng.random_float(0.0, 1.0) < 0.5 { c"{scorch1" } else { c"{scorch2" });
        }
        let scale = if sf & SF_ENVEXPLOSION_NOFIREBALL == 0 { self.sprite_scale as u8 as i32 } else { 0 };
        te_explosion(org, self.fireball, scale, 15, te::EXPLFLAG_NONE);
        if sf & SF_ENVEXPLOSION_NODAMAGE == 0 {
            let m = self.magnitude as f32;
            radius_damage(org, me, me, m, m * 2.5, crate::damage::bits::BLAST);
        }
        // SetThink(&CEnvExplosion::Smoke) replaces a death think.
        self.smoking = true;
        self.monster.think = super::monsters::MonsterThink::None;
        cx.pev().nextthink = eng.time() + 0.3;
        if sf & SF_ENVEXPLOSION_NOSPARKS == 0 {
            for _ in 0..eng.random_long(0, 3) {
                create_entity(c"spark_shower", org, tr.vecPlaneNormal, std::ptr::null_mut());
            }
        }
    }

    fn think(&mut self, cx: &mut Cx) {
        if self.monster.run_think(cx) || !self.smoking {
            return;
        }
        self.smoking = false;
        let v = cx.pev();
        if v.spawnflags & SF_ENVEXPLOSION_NOSMOKE == 0 {
            te_smoke(v.origin, self.smoke, self.sprite_scale as u8 as i32, 12);
        }
        if cx.pev().spawnflags & SF_ENVEXPLOSION_REPEATABLE == 0 {
            remove(cx.edict);
        }
    }

    /// `CBaseMonster::TakeDamage` (0xb2430), with `CBaseEntity::Classify` (0) and no gibs.
    fn take_damage(&mut self, cx: &mut Cx, info: &DamageInfo) -> bool {
        self.monster.take_damage(cx, info, 0, 0)
    }
}

/// `ExplosionCreate` (0xc00e8): a one-shot `env_explosion` of `magnitude` at `center`, owned by
/// `owner`; `damage = false` sets `SF_ENVEXPLOSION_NODAMAGE`.
pub fn explosion_create(center: Vec3, angles: Vec3, owner: *mut edict_t, magnitude: i32, damage: bool) {
    let e = create_entity(c"env_explosion", center, angles, owner);
    if e.is_null() {
        return;
    }
    world::with::<EnvExplosion, _>(e, |x| x.magnitude = magnitude);
    if !damage {
        e.v().spawnflags |= SF_ENVEXPLOSION_NODAMAGE;
    }
    world::dispatch(e, |ent, cx| {
        ent.spawn(cx);
        ent.use_(cx, std::ptr::null_mut(), std::ptr::null_mut(), UseType::Toggle, 0.0);
    });
}

/// `spark_shower` (CShower): an invisible bouncing spark emitter.
#[derive(Default)]
pub struct Shower;

impl Entity for Shower {
    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        let v = cx.pev();
        v.velocity = vec::scale(v.angles, eng.random_float(200.0, 300.0));
        v.velocity[0] += eng.random_float(-100.0, 100.0);
        v.velocity[1] += eng.random_float(-100.0, 100.0);
        v.velocity[2] += if v.velocity[2] >= 0.0 { 200.0 } else { -200.0 };
        v.movetype = MOVETYPE_BOUNCE as i32;
        v.gravity = 0.5;
        v.nextthink = eng.time() + 0.1;
        v.solid = SOLID_NOT as i32;
        eng.set_model(cx.edict, c"models/grenade.mdl");
        eng.set_size(cx.edict, [0.0; 3], [0.0; 3]);
        let v = cx.pev();
        v.effects |= EF_NODRAW as i32;
        v.speed = eng.random_float(0.5, 1.5);
        v.angles = [0.0; 3];
    }

    fn think(&mut self, cx: &mut Cx) {
        let v = cx.pev();
        sparks(v.origin);
        v.speed -= 0.1;
        if v.speed > 0.0 {
            v.nextthink = engine().time() + 0.1;
        } else {
            remove(cx.edict);
        }
        cx.pev().flags &= !(FL_ONGROUND as i32);
    }

    fn touch(&mut self, cx: &mut Cx, _other: *mut edict_t) {
        let v = cx.pev();
        let k = if v.flags & FL_ONGROUND as i32 != 0 { 0.1 } else { 0.6 };
        v.velocity = vec::scale(v.velocity, k);
        if v.velocity[0] * v.velocity[0] + v.velocity[1] * v.velocity[1] < 10.0 {
            v.speed = 0.0;
        }
    }
}

// ---- env_render ---------------------------------------------------------------------------------------------

pub const SF_RENDER_MASKFX: i32 = 1;
pub const SF_RENDER_MASKAMT: i32 = 2;
pub const SF_RENDER_MASKMODE: i32 = 4;
pub const SF_RENDER_MASKCOLOR: i32 = 8;

#[derive(Default)]
pub struct RenderFxManager;

impl Entity for RenderFxManager {
    fn spawn(&mut self, cx: &mut Cx) {
        cx.pev().solid = SOLID_NOT as i32;
    }

    fn use_(&mut self, cx: &mut Cx, _a: *mut edict_t, _c: *mut edict_t, _k: UseType, _v: f32) {
        let eng = engine();
        let v = cx.pev();
        if v.target == 0 {
            return;
        }
        let (sf, fx, amt, mode, color, target) = (v.spawnflags, v.renderfx, v.renderamt, v.rendermode, v.rendercolor, v.target);
        let mut e = std::ptr::null_mut();
        loop {
            e = eng.find_by_target_name(e, s(target));
            if e.is_null() {
                break;
            }
            let tv = e.v();
            if sf & SF_RENDER_MASKFX == 0 {
                tv.renderfx = fx;
            }
            if sf & SF_RENDER_MASKAMT == 0 {
                tv.renderamt = amt;
            }
            if sf & SF_RENDER_MASKMODE == 0 {
                tv.rendermode = mode;
            }
            if sf & SF_RENDER_MASKCOLOR == 0 {
                tv.rendercolor = color;
            }
        }
    }
}

// ---- beams: env_beam (CLightning) and env_laser (CLaser) ---------------------------------------------------------

pub const SF_BEAM_STARTON: i32 = 0x0001;
pub const SF_BEAM_TOGGLE: i32 = 0x0002;
pub const SF_BEAM_RANDOM: i32 = 0x0004;
pub const SF_BEAM_RING: i32 = 0x0008;
pub const SF_BEAM_SPARKSTART: i32 = 0x0010;
pub const SF_BEAM_SPARKEND: i32 = 0x0020;
pub const SF_BEAM_DECALS: i32 = 0x0040;
pub const SF_BEAM_SHADEIN: i32 = 0x0080;
pub const SF_BEAM_SHADEOUT: i32 = 0x0100;

const BEAM_POINTS: i32 = 0;
const BEAM_ENTPOINT: i32 = 1;
const BEAM_ENTS: i32 = 2;
const BEAM_FSHADEIN: i32 = 0x40;
const BEAM_FSHADEOUT: i32 = 0x80;
const DMG_ENERGYBEAM: i32 = 1 << 10;

/// `CBeam` entity-field helpers (the beam state rides in pev, see SDK effects.h).
mod beam {
    use super::*;

    pub fn set_type(v: &mut entvars_t, t: i32) {
        v.rendermode = (v.rendermode & 0xf0) | (t & 0x0f);
    }
    pub fn set_flags(v: &mut entvars_t, f: i32) {
        v.rendermode = (v.rendermode & 0x0f) | (f & 0xf0);
    }
    pub fn get_type(v: &entvars_t) -> i32 {
        v.rendermode & 0x0f
    }
    pub fn set_start_entity(v: &mut entvars_t, idx: i32) {
        v.sequence = (idx & 0x0fff) | ((v.sequence & 0xf000) << 12);
        v.owner = engine().entity_of_index(idx);
    }
    pub fn set_end_entity(v: &mut entvars_t, idx: i32) {
        v.skin = (idx & 0x0fff) | ((v.skin & 0xf000) << 12);
        v.aiment = engine().entity_of_index(idx);
    }
    pub fn start_pos(v: &entvars_t) -> Vec3 {
        if get_type(v) == BEAM_ENTS {
            let e = engine().entity_of_index(v.sequence & 0x0fff);
            if !e.is_null() {
                return e.v().origin;
            }
        }
        v.origin
    }
    pub fn end_pos(v: &entvars_t) -> Vec3 {
        let t = get_type(v);
        if t == BEAM_POINTS || t == 3 {
            return v.angles;
        }
        let e = engine().entity_of_index(v.skin & 0x0fff);
        if !e.is_null() { e.v().origin } else { v.angles }
    }
    /// `RelinkBeam`: size the entity to cover both ends (for PVS culling).
    pub fn relink(cx: &mut Cx) {
        let v = cx.pev();
        let (a, b) = (start_pos(v), end_pos(v));
        let mut mins = [0.0; 3];
        let mut maxs = [0.0; 3];
        for i in 0..3 {
            mins[i] = a[i].min(b[i]) - v.origin[i];
            maxs[i] = a[i].max(b[i]) - v.origin[i];
        }
        let org = v.origin;
        engine().set_size(cx.edict, mins, maxs);
        engine().set_origin(cx.edict, org);
    }
    /// `DoSparks`.
    pub fn do_sparks(v: &entvars_t, start: Vec3, end: Vec3) {
        if v.spawnflags & SF_BEAM_SPARKSTART != 0 {
            sparks(start);
        }
        if v.spawnflags & SF_BEAM_SPARKEND != 0 {
            sparks(end);
        }
    }
    /// `BeamDamage`: damage per second (`pev->dmg`) since the last call.
    pub fn damage(cx: &mut Cx, tr: &TraceResult) {
        relink(cx);
        let eng = engine();
        let now = eng.time();
        let me = cx.edict;
        let v = cx.pev();
        if tr.flFraction != 1.0 && !tr.pHit.is_null() {
            // `pHit->TraceAttack(pev, dmg, dir, tr, DMG_ENERGYBEAM)` (0xba2fb).
            let mut info = DamageInfo::new(me, me, v.dmg * (now - v.dmgtime), DMG_ENERGYBEAM).traced();
            info.hitgroup = tr.iHitgroup;
            info.dir = vec::normalize(vec::sub(tr.vecEndPos, v.origin));
            info.pos = tr.vecEndPos;
            let decals = v.spawnflags & SF_BEAM_DECALS != 0;
            crate::damage::apply(tr.pHit, &info);
            if decals {
                let hv = tr.pHit.v();
                if hv.solid == SOLID_BSP as i32 || hv.movetype == MOVETYPE_PUSHSTEP as i32 {
                    let n = eng.random_long(1, 5);
                    decal_trace(tr, goldsrc::server::intern(&format!("{{bigshot{n}")));
                }
            }
        }
        cx.pev().dmgtime = now;
    }
    /// `CBaseEntity::RandomTargetname`.
    pub fn random_targetname(name: &CStr) -> *mut edict_t {
        let eng = engine();
        let mut total = 0;
        let mut found = std::ptr::null_mut();
        let mut e = std::ptr::null_mut();
        loop {
            e = eng.find_by_target_name(e, name);
            if e.is_null() {
                break;
            }
            total += 1;
            if eng.random_long(0, total - 1) < 1 {
                found = e;
            }
        }
        found
    }
    pub fn is_point_entity(e: *mut edict_t) -> bool {
        let v = e.v();
        if v.modelindex == 0 {
            return true;
        }
        let c = s(v.classname);
        c == c"info_target" || c == c"info_landmark" || c == c"path_corner"
    }
}

#[derive(Default)]
pub struct Lightning {
    active: bool,
    sprite_texture: i32,
    sprite_name: string_t,
    start_entity: string_t,
    end_entity: string_t,
    life: f32,
    bolt_width: i32,
    noise: i32,
    frame_start: i32,
    speed: i32,
    restrike: f32,
    radius: f32,
    /// Think modes (SDK SetThink targets).
    mode: BeamThink,
    usable: BeamUse,
    needs_activate: bool,
    delay: Delay,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum BeamThink {
    #[default]
    None,
    Damage,
    Strike,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum BeamUse {
    #[default]
    None,
    Toggle,
    Strike,
}

impl Lightning {
    /// `ServerSide()`: a persistent beam entity rather than repeated temp-entity strikes.
    fn server_side(cx: &mut Cx, life: f32) -> bool {
        life == 0.0 && cx.pev().spawnflags & SF_BEAM_RING == 0
    }

    fn beam_tail(&self, w: MsgWriter, cx: &mut Cx) -> MsgWriter {
        let v = cx.pev();
        w.short(self.sprite_texture)
            .byte(self.frame_start)
            .byte(v.framerate as i32)
            .byte((self.life * 10.0) as i32)
            .byte(self.bolt_width)
            .byte(self.noise)
            .byte(v.rendercolor[0] as i32)
            .byte(v.rendercolor[1] as i32)
            .byte(v.rendercolor[2] as i32)
            .byte(v.renderamt as i32)
            .byte(self.speed)
    }

    fn zap(&mut self, cx: &mut Cx, src: Vec3, dest: Vec3) {
        let w =
            MsgWriter::begin(MSG_BROADCAST as i32, SVC_TEMPENTITY, None, std::ptr::null_mut()).byte(te::BEAMPOINTS).vec3(src).vec3(dest);
        self.beam_tail(w, cx).end();
        beam::do_sparks(cx.pev(), src, dest);
    }

    fn random_area(&mut self, cx: &mut Cx) {
        let eng = engine();
        let me = cx.edict;
        for _ in 0..10 {
            let src = cx.pev().origin;
            let rnd = || [eng.random_float(-1.0, 1.0), eng.random_float(-1.0, 1.0), eng.random_float(-1.0, 1.0)];
            let d1 = vec::normalize(rnd());
            let tr1 = eng.trace_line(src, vec::mad(src, d1, self.radius), IGNORE_MONSTERS, me);
            if tr1.flFraction == 1.0 {
                continue;
            }
            let mut d2 = rnd();
            while vec::dot(d1, d2) > 0.0 {
                d2 = rnd();
            }
            let d2 = vec::normalize(d2);
            let tr2 = eng.trace_line(src, vec::mad(src, d2, self.radius), IGNORE_MONSTERS, me);
            if tr2.flFraction == 1.0 {
                continue;
            }
            if vec::length(vec::sub(tr1.vecEndPos, tr2.vecEndPos)) < self.radius * 0.1 {
                continue;
            }
            let tr3 = eng.trace_line(tr1.vecEndPos, tr2.vecEndPos, IGNORE_MONSTERS, me);
            if tr3.flFraction != 1.0 {
                continue;
            }
            self.zap(cx, tr1.vecEndPos, tr2.vecEndPos);
            break;
        }
    }

    fn random_point(&mut self, cx: &mut Cx, src: Vec3) {
        let eng = engine();
        let me = cx.edict;
        for _ in 0..10 {
            let d1 = vec::normalize([eng.random_float(-1.0, 1.0), eng.random_float(-1.0, 1.0), eng.random_float(-1.0, 1.0)]);
            let tr1 = eng.trace_line(src, vec::mad(src, d1, self.radius), IGNORE_MONSTERS, me);
            if vec::length(vec::sub(tr1.vecEndPos, src)) < self.radius * 0.1 || tr1.flFraction == 1.0 {
                continue;
            }
            self.zap(cx, src, tr1.vecEndPos);
            break;
        }
    }

    fn strike_think(&mut self, cx: &mut Cx) {
        let eng = engine();
        let now = eng.time();
        if self.life != 0.0 {
            let extra = if cx.pev().spawnflags & SF_BEAM_RANDOM != 0 { eng.random_float(0.0, self.restrike) } else { self.restrike };
            cx.pev().nextthink = now + self.life + extra;
        }
        self.active = true;
        if self.end_entity == 0 {
            if self.start_entity == 0 {
                self.random_area(cx);
            } else {
                let start = beam::random_targetname(s(self.start_entity));
                if !start.is_null() {
                    let o = start.v().origin;
                    self.random_point(cx, o);
                } else {
                    eng.alert(
                        ALERT_TYPE::at_console,
                        &format!("env_beam: unknown entity \"{}\"\n", s(self.start_entity).to_string_lossy()),
                    );
                }
            }
            return;
        }
        let mut start = beam::random_targetname(s(self.start_entity));
        let mut end = beam::random_targetname(s(self.end_entity));
        if start.is_null() || end.is_null() {
            return;
        }
        let ring = cx.pev().spawnflags & SF_BEAM_RING != 0;
        let any_point = beam::is_point_entity(start) || beam::is_point_entity(end);
        if any_point && ring {
            return;
        }
        let w = MsgWriter::begin(MSG_BROADCAST as i32, SVC_TEMPENTITY, None, std::ptr::null_mut());
        let w = if any_point {
            if !beam::is_point_entity(end) {
                std::mem::swap(&mut start, &mut end);
            }
            let eo = end.v().origin;
            if !beam::is_point_entity(start) {
                w.byte(te::BEAMENTPOINT).short(eng.index_of(start)).vec3(eo)
            } else {
                w.byte(te::BEAMPOINTS).vec3(start.v().origin).vec3(eo)
            }
        } else {
            w.byte(if ring { te::BEAMRING } else { te::BEAMENTS }).short(eng.index_of(start)).short(eng.index_of(end))
        };
        self.beam_tail(w, cx).end();
        let (so, eo) = (start.v().origin, end.v().origin);
        beam::do_sparks(cx.pev(), so, eo);
        if cx.pev().dmg > 0.0 {
            let tr = eng.trace_line(so, eo, DONT_IGNORE_MONSTERS, std::ptr::null_mut());
            let v = cx.pev();
            v.dmgtime = now - 1.0; // BeamDamageInstant
            beam::damage(cx, &tr);
        }
    }

    fn damage_think(&mut self, cx: &mut Cx) {
        let eng = engine();
        cx.pev().nextthink = eng.time() + 0.1;
        let v = cx.pev();
        let (a, b) = (beam::start_pos(v), beam::end_pos(v));
        let tr = eng.trace_line(a, b, DONT_IGNORE_MONSTERS, std::ptr::null_mut());
        beam::damage(cx, &tr);
    }

    /// `CLightning::BeamUpdateVars` (run from `Activate` in the SDK).
    fn update_vars(&mut self, cx: &mut Cx) {
        let eng = engine();
        let mut start = eng.find_by_target_name(std::ptr::null_mut(), s(self.start_entity));
        let mut end = eng.find_by_target_name(std::ptr::null_mut(), s(self.end_entity));
        if start.is_null() || end.is_null() {
            return;
        }
        let mut ps = beam::is_point_entity(start);
        let mut pe = beam::is_point_entity(end);
        let v = cx.pev();
        v.skin = 0;
        v.sequence = 0;
        v.rendermode = 0;
        v.flags |= FL_CUSTOMENTITY as i32;
        v.model = self.sprite_name;
        v.modelindex = self.sprite_texture;
        let mut t = BEAM_ENTS;
        if ps || pe {
            if !ps {
                std::mem::swap(&mut start, &mut end);
                std::mem::swap(&mut ps, &mut pe);
            }
            t = if !pe { BEAM_ENTPOINT } else { BEAM_POINTS };
        }
        beam::set_type(v, t);
        if t == BEAM_POINTS || t == BEAM_ENTPOINT {
            v.origin = start.v().origin;
            if t == BEAM_POINTS {
                v.angles = end.v().origin;
            } else {
                beam::set_end_entity(v, eng.index_of(end));
            }
        } else {
            beam::set_start_entity(v, eng.index_of(start));
            beam::set_end_entity(v, eng.index_of(end));
        }
        beam::relink(cx);
        let v = cx.pev();
        v.scale = self.bolt_width as f32;
        v.body = self.noise;
        v.frame = self.frame_start as f32;
        v.animtime = self.speed as f32;
        if v.spawnflags & SF_BEAM_SHADEIN != 0 {
            beam::set_flags(v, BEAM_FSHADEIN);
        } else if v.spawnflags & SF_BEAM_SHADEOUT != 0 {
            beam::set_flags(v, BEAM_FSHADEOUT);
        }
    }
}

impl Entity for Lightning {
    fn key_value(&mut self, cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"LightningStart" => self.start_entity = alloc(value),
            b"LightningEnd" => self.end_entity = alloc(value),
            b"life" => self.life = kv_f(value),
            b"BoltWidth" => self.bolt_width = kv_i(value),
            b"NoiseAmplitude" => self.noise = kv_i(value),
            b"TextureScroll" => self.speed = kv_i(value),
            b"StrikeTime" => self.restrike = kv_f(value),
            b"texture" => self.sprite_name = alloc(value),
            b"framestart" => self.frame_start = kv_i(value),
            b"Radius" => self.radius = kv_f(value),
            b"damage" => cx.pev().dmg = kv_f(value),
            _ => return self.delay.key_value(key, value),
        }
        true
    }

    fn spawn(&mut self, cx: &mut Cx) {
        let now = engine().time();
        if self.sprite_name == 0 {
            remove(cx.edict);
            return;
        }
        cx.pev().solid = SOLID_NOT as i32;
        self.sprite_texture = precache_model_s(self.sprite_name);
        cx.pev().dmgtime = now;
        if Self::server_side(cx, self.life) {
            self.mode = BeamThink::None;
            let v = cx.pev();
            if v.dmg > 0.0 {
                self.mode = BeamThink::Damage;
            }
            if v.targetname != 0 {
                if v.spawnflags & SF_BEAM_STARTON == 0 {
                    v.effects = EF_NODRAW as i32;
                    self.active = false;
                } else {
                    self.active = true;
                }
                self.usable = BeamUse::Toggle;
            } else {
                self.active = true;
            }
            // SDK Activate -> BeamUpdateVars once everything has spawned.
            self.needs_activate = true;
            cx.pev().nextthink = now + 0.05;
        } else {
            self.active = false;
            let v = cx.pev();
            if v.targetname != 0 {
                self.usable = BeamUse::Strike;
            }
            if v.targetname == 0 || v.spawnflags & SF_BEAM_STARTON != 0 {
                self.mode = BeamThink::Strike;
                v.nextthink = now + 1.0;
            }
        }
    }

    fn think(&mut self, cx: &mut Cx) {
        if self.needs_activate {
            self.needs_activate = false;
            self.update_vars(cx);
            let now = engine().time();
            let v = cx.pev();
            v.nextthink = if self.mode == BeamThink::Damage && self.active { now + 0.1 } else { 0.0 };
            return;
        }
        match self.mode {
            BeamThink::Damage => self.damage_think(cx),
            BeamThink::Strike => self.strike_think(cx),
            BeamThink::None => {}
        }
    }

    fn use_(&mut self, cx: &mut Cx, _a: *mut edict_t, _c: *mut edict_t, kind: UseType, _v: f32) {
        let now = engine().time();
        match self.usable {
            BeamUse::None => {}
            BeamUse::Toggle => {
                if !should_toggle(kind, self.active) {
                    return;
                }
                if self.active {
                    self.active = false;
                    let v = cx.pev();
                    v.effects |= EF_NODRAW as i32;
                    v.nextthink = 0.0;
                } else {
                    self.active = true;
                    let v = cx.pev();
                    v.effects &= !(EF_NODRAW as i32);
                    let (a, b) = (beam::start_pos(v), beam::end_pos(v));
                    beam::do_sparks(v, a, b);
                    if v.dmg > 0.0 {
                        v.nextthink = now;
                        v.dmgtime = now;
                    }
                }
            }
            BeamUse::Strike => {
                if !should_toggle(kind, self.active) {
                    return;
                }
                if self.active {
                    self.active = false;
                    self.mode = BeamThink::None;
                } else {
                    self.mode = BeamThink::Strike;
                    cx.pev().nextthink = now + 0.1;
                }
                if cx.pev().spawnflags & SF_BEAM_TOGGLE == 0 {
                    self.usable = BeamUse::None;
                }
            }
        }
    }
}

/// `env_laser` (CLaser).
#[derive(Default)]
pub struct Laser {
    sprite: EHandle,
    sprite_name: string_t,
    fire_position: Vec3,
    on: bool,
}

impl Laser {
    fn turn_off(&mut self, cx: &mut Cx) {
        let v = cx.pev();
        v.effects |= EF_NODRAW as i32;
        v.nextthink = 0.0;
        self.on = false;
        let sp = self.sprite.get();
        if !sp.is_null() {
            world::dispatch(sp, |e, c| e.use_(c, std::ptr::null_mut(), std::ptr::null_mut(), UseType::Off, 0.0));
        }
    }

    fn turn_on(&mut self, cx: &mut Cx) {
        let now = engine().time();
        let v = cx.pev();
        v.effects &= !(EF_NODRAW as i32);
        self.on = true;
        let sp = self.sprite.get();
        if !sp.is_null() {
            world::dispatch(sp, |e, c| e.use_(c, std::ptr::null_mut(), std::ptr::null_mut(), UseType::On, 0.0));
        }
        let v = cx.pev();
        v.dmgtime = now;
        v.nextthink = now;
    }

    /// `CLaser::FireAtPoint`.
    fn fire_at_point(&mut self, cx: &mut Cx, tr: &TraceResult) {
        cx.pev().angles = tr.vecEndPos;
        let sp = self.sprite.get();
        if !sp.is_null() {
            engine().set_origin(sp, tr.vecEndPos);
        }
        beam::damage(cx, tr);
        let v = cx.pev();
        let start = beam::start_pos(v);
        beam::do_sparks(v, start, tr.vecEndPos);
    }

    /// `CLaser::TurnOff` on another entity (`func_tanklaser`). False if `e` isn't an env_laser.
    pub fn turn_off_entity(e: *mut edict_t) -> bool {
        world::dispatch(e, |ent, cx| (ent as &mut dyn std::any::Any).downcast_mut::<Laser>().map(|l| l.turn_off(cx))).flatten().is_some()
    }

    /// One `func_tanklaser` shot (`CFuncTankLaser::Fire` 0xc44e0): move the beam start to the
    /// barrel, turn it on, deal a full second of `dmg`, aim it at `tr`, and stop its own think.
    pub fn tank_shot(e: *mut edict_t, barrel_end: Vec3, tr: &TraceResult) {
        world::dispatch(e, |ent, cx| {
            let Some(l) = (ent as &mut dyn std::any::Any).downcast_mut::<Laser>() else { return };
            let now = engine().time();
            cx.pev().origin = barrel_end;
            l.turn_on(cx);
            cx.pev().dmgtime = now - 1.0;
            l.fire_at_point(cx, tr);
            cx.pev().nextthink = 0.0;
        });
    }
}

impl Entity for Laser {
    fn key_value(&mut self, cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        let v = cx.pev();
        match key.to_bytes() {
            b"LaserTarget" => v.message = alloc(value),
            b"width" => v.scale = kv_f(value),
            b"NoiseAmplitude" => v.body = kv_i(value),
            b"TextureScroll" => v.animtime = kv_i(value) as f32,
            b"texture" => v.model = alloc(value),
            b"EndSprite" => self.sprite_name = alloc(value),
            b"framestart" => v.frame = kv_i(value) as f32,
            b"damage" => v.dmg = kv_f(value),
            _ => return false,
        }
        true
    }

    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        if cx.pev().model == 0 {
            remove(cx.edict);
            return;
        }
        let v = cx.pev();
        v.solid = SOLID_NOT as i32;
        v.flags |= FL_CUSTOMENTITY as i32;
        v.modelindex = precache_model_s(v.model);
        if self.sprite_name != 0 {
            precache_model_s(self.sprite_name);
        }
        // PointsInit(origin, origin)
        let v = cx.pev();
        beam::set_type(v, BEAM_POINTS);
        v.angles = v.origin;
        v.frame = 0.0;
        v.animtime = 0.0;
        beam::relink(cx);
        if !self.sprite.is_valid() && self.sprite_name != 0 {
            let org = cx.pev().origin;
            let sp = Sprite::create(s(self.sprite_name), org, true);
            if !sp.is_null() {
                let (c, a, fx) = {
                    let v = cx.pev();
                    (v.rendercolor, v.renderamt, v.renderfx)
                };
                let sv = sp.v();
                sv.rendermode = kRenderGlow as i32;
                sv.rendercolor = c;
                sv.renderamt = a;
                sv.renderfx = fx;
            }
            self.sprite = EHandle::new(sp);
        }
        let _ = eng;
        if cx.pev().targetname != 0 && cx.pev().spawnflags & SF_BEAM_STARTON == 0 {
            self.turn_off(cx);
        } else {
            self.turn_on(cx);
        }
    }

    fn think(&mut self, cx: &mut Cx) {
        let eng = engine();
        let end = beam::random_targetname(s(cx.pev().message));
        if !end.is_null() {
            self.fire_position = end.v().origin;
        }
        let org = cx.pev().origin;
        let tr = eng.trace_line(org, self.fire_position, DONT_IGNORE_MONSTERS, std::ptr::null_mut());
        self.fire_at_point(cx, &tr);
        cx.pev().nextthink = eng.time() + 0.1;
    }

    fn use_(&mut self, cx: &mut Cx, _a: *mut edict_t, _c: *mut edict_t, kind: UseType, _v: f32) {
        let active = cx.pev().effects & EF_NODRAW as i32 == 0;
        if !should_toggle(kind, active) {
            return;
        }
        if active {
            self.turn_off(cx);
        } else {
            self.turn_on(cx);
        }
    }
}

// ---- env_shooter / gibshooter / gib ---------------------------------------------------------------------------

pub const SF_GIBSHOOTER_REPEATABLE: i32 = 1;

/// `gibshooter` (`ENV = false`, human gibs) and `env_shooter` (`ENV = true`, any model).
#[derive(Default)]
pub struct GibShooter<const ENV: bool> {
    delay: Delay,
    gibs: i32,
    capacity: i32,
    velocity: f32,
    variance: f32,
    gib_life: f32,
    model_index: i32,
    material: i32,
    shooting: bool,
    usable: bool,
}

impl<const ENV: bool> GibShooter<ENV> {
    fn create_gib(&mut self, cx: &mut Cx) -> *mut edict_t {
        let eng = engine();
        if !ENV && eng.cvar_float(c"violence_hgibs") == 0.0 {
            return std::ptr::null_mut();
        }
        let e = eng.create_entity();
        if e.is_null() {
            return e;
        }
        world::create::<Gib>(e.v());
        let model: &CStr = if ENV { s(cx.pev().model) } else { c"models/hgibs.mdl" };
        let material = if ENV { self.material } else { mat::NONE };
        // CGibShooter::CreateGib: red; CEnvShooter::CreateGib: DONT_BLEED.
        let blood_color = if ENV { DONT_BLEED } else { BLOOD_COLOR_RED };
        world::dispatch(e, |ent, gcx| {
            if let Some(g) = (ent as &mut dyn std::any::Any).downcast_mut::<Gib>() {
                g.spawn_gib(gcx, model);
                g.material = material;
                g.blood_color = blood_color;
            }
        });
        let v = cx.pev();
        let gv = e.v();
        if ENV {
            gv.body = if v.body > 1 { eng.random_long(0, v.body - 1) } else { 0 };
            gv.rendermode = v.rendermode;
            gv.renderamt = v.renderamt;
            gv.rendercolor = v.rendercolor;
            gv.renderfx = v.renderfx;
            gv.scale = v.scale;
            gv.skin = v.skin;
        } else {
            gv.body = if v.body > 1 { eng.random_long(1, v.body - 1) } else { 0 };
        }
        e
    }

    fn shoot_think(&mut self, cx: &mut Cx) {
        let eng = engine();
        let now = eng.time();
        cx.pev().nextthink = now + self.delay.delay;
        let (fwd, right, up) = eng.make_vectors(cx.pev().angles);
        let mut dir = cx.pev().movedir;
        dir = vec::mad(dir, right, eng.random_float(-1.0, 1.0) * self.variance);
        dir = vec::mad(dir, fwd, eng.random_float(-1.0, 1.0) * self.variance);
        dir = vec::mad(dir, up, eng.random_float(-1.0, 1.0) * self.variance);
        dir = vec::normalize(dir);
        let gib = self.create_gib(cx);
        if !gib.is_null() {
            let org = cx.pev().origin;
            let life = self.gib_life * eng.random_float(0.95, 1.05);
            let gv = gib.v();
            gv.origin = org;
            gv.velocity = vec::scale(dir, self.velocity);
            gv.avelocity[0] = eng.random_float(100.0, 200.0);
            gv.avelocity[1] = eng.random_float(100.0, 300.0);
            let think_time = gv.nextthink - now;
            let mut life_left = life;
            if life < think_time {
                gv.nextthink = now + life;
                life_left = 0.0;
            }
            world::with::<Gib, _>(gib, |g| g.life_time = life_left);
        }
        self.gibs -= 1;
        if self.gibs <= 0 {
            if cx.pev().spawnflags & SF_GIBSHOOTER_REPEATABLE != 0 {
                self.gibs = self.capacity;
                self.shooting = false;
                cx.pev().nextthink = now;
            } else {
                remove(cx.edict);
            }
        }
    }
}

impl<const ENV: bool> Entity for GibShooter<ENV> {
    fn key_value(&mut self, cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        if ENV {
            match key.to_bytes() {
                b"shootmodel" => {
                    cx.pev().model = alloc(value);
                    return true;
                }
                b"shootsounds" => {
                    self.material = match kv_i(value) {
                        0 => mat::GLASS,
                        1 => mat::WOOD,
                        2 => mat::METAL,
                        3 => mat::FLESH,
                        4 => mat::ROCKS,
                        _ => mat::NONE,
                    };
                    return true;
                }
                _ => {}
            }
        }
        match key.to_bytes() {
            b"m_iGibs" => {
                self.gibs = kv_i(value);
                self.capacity = self.gibs;
            }
            b"m_flVelocity" => self.velocity = kv_f(value),
            b"m_flVariance" => self.variance = kv_f(value),
            b"m_flGibLife" => self.gib_life = kv_f(value),
            _ => return self.delay.key_value(key, value),
        }
        true
    }

    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        self.model_index = if ENV {
            // CEnvShooter::Precache (0xbca44): the model, then the material sounds.
            let index = precache_model_s(cx.pev().model);
            material_sound_precache(self.material);
            index
        } else if super::blood::german() {
            // CGibShooter::Precache (0xbc1c4): TS's (misspelt) German gib model.
            eng.precache_model(c"models/germanygibs.mdl")
        } else {
            eng.precache_model(c"models/hgibs.mdl")
        };
        let v = cx.pev();
        v.solid = SOLID_NOT as i32;
        v.effects = EF_NODRAW as i32;
        if self.delay.delay == 0.0 {
            self.delay.delay = 0.1;
        }
        if self.gib_life == 0.0 {
            self.gib_life = 25.0;
        }
        set_movedir(cx.pev());
        cx.pev().body = eng.model_frames(self.model_index);
        self.usable = true;
    }

    fn think(&mut self, cx: &mut Cx) {
        if self.shooting {
            self.shoot_think(cx);
        }
    }

    fn use_(&mut self, cx: &mut Cx, _a: *mut edict_t, _c: *mut edict_t, _k: UseType, _v: f32) {
        self.shooting = true;
        cx.pev().nextthink = engine().time();
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum GibState {
    #[default]
    WaitTillLand,
    StartFade,
    FadeOut,
    Remove,
}

/// `gib` (CGib).
#[derive(Default)]
pub struct Gib {
    life_time: f32,
    material: i32,
    /// `m_bloodColor`: 0 until set, as in TS's zeroed private data.
    blood_color: i32,
    /// `m_cBloodDecals`: blood decals left (one per bounce).
    blood_decals: i32,
    state: GibState,
}

impl Gib {
    /// A new `gib` entity spawned with `model` (`GetClassPtr<CGib>` + `CGib::Spawn`).
    fn create(model: &CStr) -> *mut edict_t {
        let e = engine().create_entity();
        if e.is_null() {
            return e;
        }
        world::create::<Gib>(e.v());
        world::dispatch(e, |ent, gcx| {
            if let Some(g) = (ent as &mut dyn std::any::Any).downcast_mut::<Gib>() {
                g.spawn_gib(gcx, model);
            }
        });
        e
    }

    /// `CGib::LimitVelocity` (0xb089c): at most 1500 units/s.
    fn limit_velocity(e: *mut edict_t) {
        let v = e.v();
        if vec::length(v.velocity) > 1500.0 {
            v.velocity = vec::scale(vec::normalize(v.velocity), 1500.0);
        }
    }

    /// The victim-health speed factor shared by the gib spawners.
    fn health_scale(victim: &entvars_t) -> f32 {
        if victim.health > -50.0 {
            0.7
        } else if victim.health > -200.0 {
            2.0
        } else {
            4.0
        }
    }

    /// `CGib::SpawnHeadGib` (0xb0ce8): the skull, thrown up (5 % of the time at a player who can
    /// see it). `blood_color` is the victim's `BloodColor()`.
    pub fn spawn_head_gib(victim: *mut edict_t, blood_color: i32) {
        let eng = engine();
        let e = Self::create(if super::blood::german() { c"models/germangibs.mdl" } else { c"models/hgibs.mdl" });
        if e.is_null() {
            return;
        }
        e.v().body = 0;
        if !victim.is_null() {
            let vv = victim.v();
            e.v().origin = vec::add(vv.origin, vv.view_ofs);
            let player = eng.find_client_in_pvs(e);
            if eng.random_long(0, 100) <= 5 && !player.is_null() {
                let eye = vec::add(player.v().origin, player.v().view_ofs);
                let gv = e.v();
                gv.velocity = vec::scale(vec::normalize(vec::sub(eye, gv.origin)), 300.0);
                gv.velocity[2] += 100.0;
            } else {
                let x = eng.random_float(-100.0, 100.0);
                let y = eng.random_float(-100.0, 100.0);
                let z = eng.random_float(200.0, 300.0);
                e.v().velocity = [x, y, z];
            }
            let gv = e.v();
            gv.avelocity[0] = eng.random_float(100.0, 200.0);
            gv.avelocity[1] = eng.random_float(100.0, 300.0);
            world::with::<Gib, _>(e, |g| g.blood_color = blood_color);
            let k = Self::health_scale(victim.v());
            e.v().velocity = vec::scale(e.v().velocity, k);
        }
        Self::limit_velocity(e);
    }

    /// `CGib::SpawnRandomGibs` (0xb1060): `count` human (`hgibs.mdl` bodies 1..5) or alien
    /// (`agibs.mdl` bodies 0..3) gibs from inside the victim's box, flying against `attack_dir`
    /// (`g_vecAttackDir`).
    pub fn spawn_random_gibs(victim: *mut edict_t, count: i32, human: bool, blood_color: i32, attack_dir: Vec3) {
        let eng = engine();
        for _ in 0..count {
            let (model, lo, hi) = if super::blood::german() {
                (c"models/germangibs.mdl", 0, 3)
            } else if human {
                (c"models/hgibs.mdl", 1, 5)
            } else {
                (c"models/agibs.mdl", 0, 3)
            };
            let e = Self::create(model);
            if e.is_null() {
                return;
            }
            e.v().body = eng.random_long(lo, hi);
            if !victim.is_null() {
                let (absmin, size) = (victim.v().absmin, victim.v().size);
                let x = absmin[0] + size[0] * eng.random_float(0.0, 1.0);
                let y = absmin[1] + size[1] * eng.random_float(0.0, 1.0);
                let z = absmin[2] + size[2] * eng.random_float(0.0, 1.0) + 1.0;
                let mut vel = vec::scale(attack_dir, -1.0);
                vel[0] += eng.random_float(-0.25, 0.25);
                vel[1] += eng.random_float(-0.25, 0.25);
                vel[2] += eng.random_float(-0.25, 0.25);
                vel = vec::scale(vel, eng.random_float(300.0, 400.0));
                let ax = eng.random_float(100.0, 200.0);
                let ay = eng.random_float(100.0, 300.0);
                let k = Self::health_scale(victim.v());
                let gv = e.v();
                gv.origin = [x, y, z];
                gv.avelocity[0] = ax;
                gv.avelocity[1] = ay;
                gv.velocity = vec::scale(vel, k);
                gv.solid = SOLID_BBOX as i32;
                world::with::<Gib, _>(e, |g| g.blood_color = blood_color);
                eng.set_size(e, [0.0; 3], [0.0; 3]);
            }
            Self::limit_velocity(e);
        }
    }

    fn spawn_gib(&mut self, cx: &mut Cx, model: &CStr) {
        let eng = engine();
        let v = cx.pev();
        v.movetype = MOVETYPE_BOUNCE as i32;
        v.friction = 0.55;
        v.renderamt = 255.0;
        v.rendermode = kRenderNormal as i32;
        v.renderfx = kRenderFxNone as i32;
        v.solid = SOLID_SLIDEBOX as i32;
        v.classname = eng.alloc_string(c"gib");
        eng.set_model(cx.edict, model);
        eng.set_size(cx.edict, [0.0; 3], [0.0; 3]);
        cx.pev().nextthink = eng.time() + 4.0;
        self.life_time = 25.0;
        self.state = GibState::WaitTillLand;
        self.material = mat::NONE;
        self.blood_decals = 5;
    }
}

impl Entity for Gib {
    fn think(&mut self, cx: &mut Cx) {
        let now = engine().time();
        match self.state {
            GibState::WaitTillLand => {
                let v = cx.pev();
                let out = (0..3).any(|i| v.absmin[i] < -4096.0 || v.absmax[i] > 4096.0);
                if out {
                    remove(cx.edict);
                } else if v.velocity == [0.0; 3] {
                    self.state = GibState::StartFade;
                    v.nextthink = now + self.life_time;
                    // `CSoundEnt::InsertSound(bits_SOUND_MEAT, origin, 384, 25)` (0xb1e5b).
                    if self.blood_color != DONT_BLEED {
                        let origin = v.origin;
                        super::soundent::insert_sound(super::soundent::SOUND_MEAT, origin, 384, 25.0);
                    }
                } else {
                    v.nextthink = now + 0.5;
                }
            }
            GibState::StartFade => {
                let v = cx.pev();
                if v.rendermode == kRenderNormal as i32 {
                    v.renderamt = 255.0;
                    v.rendermode = kRenderTransTexture as i32;
                }
                v.solid = SOLID_NOT as i32;
                v.avelocity = [0.0; 3];
                v.nextthink = now + 0.1;
                self.state = GibState::FadeOut;
            }
            GibState::FadeOut => {
                let v = cx.pev();
                if v.renderamt > 7.0 {
                    v.renderamt -= 7.0;
                    v.nextthink = now + 0.1;
                } else {
                    v.renderamt = 0.0;
                    v.nextthink = now + 0.2;
                    self.state = GibState::Remove;
                }
            }
            GibState::Remove => remove(cx.edict),
        }
    }

    fn touch(&mut self, cx: &mut Cx, _other: *mut edict_t) {
        let eng = engine();
        let v = cx.pev();
        if v.flags & FL_ONGROUND as i32 != 0 {
            v.velocity = vec::scale(v.velocity, 0.9);
            v.angles[0] = 0.0;
            v.angles[2] = 0.0;
            v.avelocity[0] = 0.0;
            v.avelocity[2] = 0.0;
        } else {
            // CGib::BounceGibTouch (0xb1ebc): a blood decal under the gib for its first bounces.
            if !super::blood::german() && self.blood_decals > 0 && self.blood_color != DONT_BLEED {
                let spot = vec::add(v.origin, [0.0, 0.0, 8.0]);
                let tr = eng.trace_line(spot, vec::add(spot, [0.0, 0.0, -24.0]), IGNORE_MONSTERS, cx.edict);
                blood_decal_trace(&tr, self.blood_color);
                self.blood_decals -= 1;
            }
            if self.material != mat::NONE && eng.random_long(0, 2) == 0 {
                let vol = 0.8 * (cx.pev().velocity[2].abs() / 450.0).min(1.0);
                let list = material_sound_list(self.material);
                if !list.is_empty() {
                    let i = eng.random_long(0, list.len() as i32 - 1) as usize;
                    emit_sound_slow(cx.edict, CHAN_BODY, list[i], vol, 1.0, 0, 100, 0.5);
                }
            }
        }
    }
}

// ---- env_spark / env_bubbles ------------------------------------------------------------------------------------

#[derive(Default)]
pub struct EnvSpark {
    max_delay: f32,
    sparking: bool,
    usable: bool,
}

impl EnvSpark {
    fn start(&mut self, cx: &mut Cx) {
        self.sparking = true;
        cx.pev().nextthink = engine().time() + 0.1 + engine().random_float(0.0, self.max_delay);
    }
}

impl Entity for EnvSpark {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"MaxDelay" => self.max_delay = kv_f(value),
            b"style" | b"height" | b"killtarget" | b"value1" | b"value2" | b"value3" => {}
            _ => return false,
        }
        true
    }

    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        let sf = cx.pev().spawnflags;
        self.sparking = false;
        self.usable = false;
        if sf & 32 != 0 {
            self.usable = true;
            if sf & 64 != 0 {
                self.sparking = true;
            }
        } else {
            self.sparking = true;
        }
        cx.pev().nextthink = eng.time() + 0.1 + eng.random_float(0.0, 1.5);
        if self.max_delay <= 0.0 {
            self.max_delay = 1.5;
        }
        precache_sparks();
    }

    fn think(&mut self, cx: &mut Cx) {
        if !self.sparking {
            return;
        }
        let eng = engine();
        cx.pev().nextthink = eng.time() + 0.1 + eng.random_float(0.0, self.max_delay);
        let org = cx.pev().origin;
        do_spark(cx.edict, org);
    }

    fn use_(&mut self, cx: &mut Cx, _a: *mut edict_t, _c: *mut edict_t, _k: UseType, _v: f32) {
        if !self.usable {
            return;
        }
        if self.sparking {
            self.sparking = false; // SparkStop
        } else {
            self.start(cx); // SparkStart
        }
    }
}

pub const SF_BUBBLES_STARTOFF: i32 = 1;

#[derive(Default)]
pub struct Bubbling {
    density: i32,
    frequency: i32,
    bubble_model: i32,
    state: bool,
}

impl Entity for Bubbling {
    fn key_value(&mut self, cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"density" => self.density = kv_i(value),
            b"frequency" => self.frequency = kv_i(value),
            b"current" => cx.pev().speed = kv_i(value) as f32,
            _ => return false,
        }
        true
    }

    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        self.bubble_model = eng.precache_model(c"sprites/bubble.spr");
        set_own_model(cx);
        let v = cx.pev();
        v.solid = SOLID_NOT as i32;
        v.renderamt = 0.0;
        v.rendermode = kRenderTransTexture as i32;
        let speed = v.speed.abs() as i32;
        v.rendercolor = [(speed >> 8) as f32, (speed & 255) as f32, if v.speed < 0.0 { 1.0 } else { 0.0 }];
        if v.spawnflags & SF_BUBBLES_STARTOFF == 0 {
            self.state = true;
            v.nextthink = eng.time() + 2.0;
        } else {
            self.state = false;
        }
    }

    fn think(&mut self, cx: &mut Cx) {
        if !self.state {
            return;
        }
        let eng = engine();
        let center = bmodel_origin(cx.pev());
        te_begin(MSG_PAS, Some(center), te::FIZZ).short(eng.index_of(cx.edict)).short(self.bubble_model).byte(self.density).end();
        let f = self.frequency;
        cx.pev().nextthink = eng.time() + if f > 19 { 0.5 } else { 2.5 - 0.1 * f as f32 };
    }

    fn use_(&mut self, cx: &mut Cx, _a: *mut edict_t, _c: *mut edict_t, kind: UseType, _v: f32) {
        if should_toggle(kind, self.state) {
            self.state = !self.state;
        }
        cx.pev().nextthink = if self.state { engine().time() + 0.1 } else { 0.0 };
    }
}

// ---- infodecal ------------------------------------------------------------------------------------------------------

pub const SF_DECAL_NOTINDEATHMATCH: i32 = 2048;

#[derive(Default)]
pub struct Decal {
    static_pending: bool,
    removing: bool,
}

impl Decal {
    fn trace(cx: &mut Cx) -> TraceResult {
        let o = cx.pev().origin;
        engine().trace_line(vec::sub(o, [5.0; 3]), vec::add(o, [5.0; 3]), IGNORE_MONSTERS, cx.edict)
    }
}

impl Entity for Decal {
    fn key_value(&mut self, cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        if key == c"texture" {
            let idx = engine().decal_index(value);
            cx.pev().skin = idx;
            if idx < 0 {
                engine().alert(ALERT_TYPE::at_console, &format!("Can't find decal {}\n", value.to_string_lossy()));
            }
            return true;
        }
        false
    }

    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        let v = cx.pev();
        let deathmatch = eng.globals().deathmatch != 0.0;
        if v.skin < 0 || (deathmatch && v.spawnflags & SF_DECAL_NOTINDEATHMATCH != 0) {
            remove(cx.edict);
            return;
        }
        if v.targetname == 0 {
            self.static_pending = true;
            v.nextthink = eng.time();
        }
    }

    fn think(&mut self, cx: &mut Cx) {
        if self.removing {
            remove(cx.edict);
            return;
        }
        if !self.static_pending {
            return;
        }
        self.static_pending = false;
        let eng = engine();
        let tr = Self::trace(cx);
        let (entity, model) = if tr.pHit.is_null() { (0, 0) } else { (eng.index_of(tr.pHit), tr.pHit.v().modelindex) };
        let model = if entity != 0 { model } else { 0 };
        let (org, skin) = (cx.pev().origin, cx.pev().skin);
        eng.static_decal(org, skin, entity, model);
        remove(cx.edict);
    }

    fn use_(&mut self, cx: &mut Cx, _a: *mut edict_t, _c: *mut edict_t, _k: UseType, _v: f32) {
        if cx.pev().targetname == 0 || self.removing {
            return;
        }
        let eng = engine();
        let tr = Self::trace(cx);
        let entity = if tr.pHit.is_null() { 0 } else { eng.index_of(tr.pHit) };
        let model = if entity != 0 { tr.pHit.v().modelindex } else { 0 };
        let (org, skin) = (cx.pev().origin, cx.pev().skin);
        goldsrc::ext::entities::te_bsp_decal(MSG_BROADCAST, org, skin, entity, model);
        self.removing = true;
        cx.pev().nextthink = eng.time() + 0.1;
    }
}

// ---- cycler_sprite / cycler ------------------------------------------------------------------------------------------

#[derive(Default)]
pub struct CyclerSprite {
    animate: bool,
    last_time: f32,
    max_frame: f32,
}

impl CyclerSprite {
    fn advance(&mut self, cx: &mut Cx, frames: f32) {
        let v = cx.pev();
        if self.max_frame > 0.0 {
            v.frame = (v.frame + frames) % self.max_frame;
        }
    }
}

impl Entity for CyclerSprite {
    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        let now = eng.time();
        let v = cx.pev();
        v.solid = SOLID_SLIDEBOX as i32;
        v.movetype = MOVETYPE_NONE as i32;
        v.takedamage = DAMAGE_YES as f32;
        v.effects = 0;
        v.frame = 0.0;
        v.nextthink = now + 0.1;
        self.animate = true;
        self.last_time = now;
        precache_model_s(v.model);
        set_own_model(cx);
        self.max_frame = eng.model_frames(cx.pev().modelindex) as f32 - 1.0;
    }

    fn think(&mut self, cx: &mut Cx) {
        let now = engine().time();
        if self.animate && self.max_frame > 1.0 {
            let fr = cx.pev().framerate;
            self.advance(cx, fr * (now - self.last_time));
        }
        cx.pev().nextthink = now + 0.1;
        self.last_time = now;
    }

    fn use_(&mut self, _cx: &mut Cx, _a: *mut edict_t, _c: *mut edict_t, _k: UseType, _v: f32) {
        self.animate = !self.animate;
    }

    fn take_damage(&mut self, cx: &mut Cx, _info: &DamageInfo) -> bool {
        if self.max_frame > 1.0 {
            self.advance(cx, 1.0);
        }
        true
    }
}

/// `cycler` (`CGenericCycler::Spawn` 0xc86ac) and `cycler_prdroid` (`CCyclerProbe::Spawn`
/// 0xc752c), both through `CCycler::GenericCyclerSpawn` (0xc75d4): a studio model cycling its
/// sequence. +use toggles the animation; damage steps to the next sequence while animating, or
/// advances one 0.1 s frame step while paused. Both are renamed `cycler`.
#[derive(Default)]
pub struct Cycler<const PROBE: bool> {
    anim: Animating,
    animate: bool,
}

pub type GenericCycler = Cycler<false>;
pub type CyclerProbe = Cycler<true>;

impl<const PROBE: bool> Cycler<PROBE> {
    /// `CCycler::GenericCyclerSpawn` (0xc75d4). `model` is `None` for the map's `model` key.
    fn generic_spawn(&mut self, cx: &mut Cx, model: Option<&'static CStr>, mins: Vec3, maxs: Vec3) {
        let eng = engine();
        let v = cx.pev();
        let name = model.unwrap_or_else(|| s(v.model));
        if name.is_empty() {
            let o = v.origin;
            eng.alert(ALERT_TYPE::at_error, &format!("cycler at {:.0} {:.0} {:.0} missing modelname", o[0], o[1], o[2]));
            remove(cx.edict);
            return;
        }
        v.classname = eng.alloc_string(c"cycler");
        match model {
            Some(m) => {
                eng.precache_model(m);
                eng.set_model(cx.edict, m);
            }
            None => {
                precache_model_s(v.model);
                set_own_model(cx);
            }
        }
        // CCycler::Spawn (0xc7694)
        init_bone_controllers(cx.edict);
        let v = cx.pev();
        v.solid = SOLID_SLIDEBOX as i32;
        v.movetype = MOVETYPE_NONE as i32;
        v.takedamage = DAMAGE_YES as f32;
        v.effects = 0;
        v.health = 80000.0;
        v.yaw_speed = 5.0;
        v.ideal_yaw = v.angles[1];
        // (ChangeYaw(360) is a no-op: ideal_yaw was just set to the current yaw.)
        self.anim.frame_rate = 75.0;
        self.anim.ground_speed = 0.0;
        v.nextthink += 1.0;
        self.anim.reset_sequence_info(cx.edict);
        let v = cx.pev();
        if v.sequence != 0 || v.frame != 0.0 {
            self.animate = false;
            v.framerate = 0.0;
        } else {
            self.animate = true;
        }
        eng.set_size(cx.edict, mins, maxs);
    }
}

impl<const PROBE: bool> Entity for Cycler<PROBE> {
    fn spawn(&mut self, cx: &mut Cx) {
        if PROBE {
            // CCyclerProbe::Spawn (0xc752c): 16 units up (no relink until SET_MODEL).
            cx.pev().origin[2] += 16.0;
            self.generic_spawn(cx, Some(c"models/prdroid.mdl"), [-16.0; 3], [16.0; 3]);
        } else {
            self.generic_spawn(cx, None, [-16.0, -16.0, 0.0], [16.0, 16.0, 72.0]);
        }
    }

    /// `CCycler::Think` (0xc778c).
    fn think(&mut self, cx: &mut Cx) {
        let now = engine().time();
        cx.pev().nextthink = now + 0.1;
        if self.animate {
            self.anim.frame_advance(cx.edict, 0.0);
        }
        if self.anim.finished && !self.anim.loops {
            // A one-shot sequence restarts from frame 0.
            let v = cx.pev();
            v.animtime = now;
            v.framerate = 1.0;
            self.anim.finished = false;
            self.anim.last_event_check = now;
            v.frame = 0.0;
            if !self.animate {
                v.framerate = 0.0;
            }
        }
    }

    /// `CCycler::Use` (0xc7834).
    fn use_(&mut self, cx: &mut Cx, _a: *mut edict_t, _c: *mut edict_t, _k: UseType, _v: f32) {
        self.animate = !self.animate;
        cx.pev().framerate = if self.animate { 1.0 } else { 0.0 };
    }

    /// `CCycler::TakeDamage` (0xc7870).
    fn take_damage(&mut self, cx: &mut Cx, _info: &DamageInfo) -> bool {
        if self.animate {
            cx.pev().sequence += 1;
            self.anim.reset_sequence_info(cx.edict);
            if self.anim.frame_rate == 0.0 {
                // Past the last sequence: back to the first.
                cx.pev().sequence = 0;
                self.anim.reset_sequence_info(cx.edict);
            }
            cx.pev().frame = 0.0;
        } else {
            cx.pev().framerate = 1.0;
            self.anim.frame_advance(cx.edict, 0.1);
            let v = cx.pev();
            v.framerate = 0.0;
            engine().alert(ALERT_TYPE::at_console, &format!("sequence: {}, frame {:.0}\n", v.sequence, v.frame));
        }
        false
    }
}

// ---- cycler_wreckage ---------------------------------------------------------------------------------------------

/// `cycler_wreckage` (`CWreckage`): a smoking model. With `dmgtime` set it smokes less and less
/// until that time, then removes itself.
#[derive(Default)]
pub struct Wreckage {
    anim: Animating,
    /// `m_flStartTime`, an `int` in TS (the spawn time truncated).
    start_time: i32,
    /// `g_sModelIndexSmoke` (`sprites/steam1.spr`, precached by TS's `W_Precache`).
    smoke: i32,
    /// `CWreckage` is a `CBaseMonster` (`TakeDamage`, `Killed`); its own `Think` override means a
    /// death's `SetThink` never runs.
    monster: super::monsters::MonsterBase,
}

impl Entity for Wreckage {
    /// `CWreckage::Spawn` (0xc7fe4). It doesn't call `Precache`.
    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        let now = eng.time();
        let v = cx.pev();
        v.solid = SOLID_NOT as i32;
        v.movetype = MOVETYPE_NONE as i32;
        v.takedamage = 0.0;
        v.effects = 0;
        v.frame = 0.0;
        v.nextthink = now + 0.1;
        if v.model != 0 {
            precache_model_s(v.model);
            set_own_model(cx);
        }
        // fistp under the truncating control word (0xc80b6).
        self.start_time = now as i32;
        self.smoke = eng.precache_model(c"sprites/steam1.spr");
    }

    /// `CWreckage::Think` (0xc8118).
    fn think(&mut self, cx: &mut Cx) {
        let eng = engine();
        self.anim.frame_advance(cx.edict, 0.0);
        let now = eng.time();
        let v = cx.pev();
        v.nextthink = now + 0.2;
        if v.dmgtime != 0.0 {
            if v.dmgtime < now {
                remove(cx.edict);
                return;
            }
            let r = eng.random_float(0.0, v.dmgtime - self.start_time as f32);
            if (v.dmgtime as f64 - now as f64) < r as f64 {
                return;
            }
        }
        let v = cx.pev();
        let (mins, maxs) = (v.absmin, v.absmax);
        let x = eng.random_float(mins[0], maxs[0]);
        let y = eng.random_float(mins[1], maxs[1]);
        let z = eng.random_float(mins[2], maxs[2]);
        let pos = [x, y, z];
        let scale = eng.random_long(0, 49) + 50;
        let rate = eng.random_long(0, 3) + 8;
        te_begin(MSG_PVS, Some(pos), te::SMOKE).vec3(pos).short(self.smoke).byte(scale).byte(rate).end();
    }

    /// `CBaseMonster::TakeDamage` (0xb2430). `Spawn` clears `takedamage`, so it refuses.
    fn take_damage(&mut self, cx: &mut Cx, info: &DamageInfo) -> bool {
        self.monster.take_damage(cx, info, 0, 0)
    }
}

// ---- beam / test_effect --------------------------------------------------------------------------------------------

/// `beam` (`CBeam`). A map-placed one only clears its solidity and, like any beam, takes its end
/// entities from `owner` / `aiment` (`CBeam::Precache` 0xb8cd4). Code creates beams through
/// [`beam_create`].
#[derive(Default)]
pub struct Beam;

impl Entity for Beam {
    /// `CBeam::Spawn` (0xb8cb4).
    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        let v = cx.pev();
        v.solid = SOLID_NOT as i32;
        if !v.owner.is_null() {
            let i = eng.index_of(v.owner);
            beam::set_start_entity(v, i);
        }
        let v = cx.pev();
        if !v.aiment.is_null() {
            let i = eng.index_of(v.aiment);
            beam::set_end_entity(v, i);
        }
    }
}

/// `CBeam::BeamCreate` (0xb8eb8) + `BeamInit` (0xb8f0c): a new `beam` entity (not spawned) with
/// the sprite and width.
fn beam_create(sprite: &'static CStr, width: i32) -> *mut edict_t {
    let eng = engine();
    let e = eng.create_entity();
    if e.is_null() {
        return e;
    }
    world::create::<Beam>(e.v());
    let index = eng.precache_model(sprite);
    let v = e.v();
    v.classname = eng.alloc_string(c"beam");
    v.flags |= FL_CUSTOMENTITY as i32;
    v.rendercolor = [255.0; 3];
    v.renderamt = 255.0;
    v.body = 0;
    v.frame = 0.0;
    v.animtime = 0.0;
    v.model = eng.alloc_string(sprite);
    v.modelindex = index;
    v.scale = width as f32;
    v.skin = 0;
    v.sequence = 0;
    v.rendermode = 0;
    e
}

/// `CBeam::PointsInit` (0xb8ff0).
fn beam_points_init(e: *mut edict_t, start: Vec3, end: Vec3) {
    let v = e.v();
    beam::set_type(v, BEAM_POINTS);
    v.origin = start;
    v.angles = end;
    v.sequence &= 0x0fff;
    v.skin &= 0x0fff;
    beam::relink(&mut Cx { edict: e });
}

const TEST_EFFECT_BEAMS: usize = 24;

/// `test_effect` (`CTestEffect`): on use, up to 24 lightning beams grow out of it, one every
/// 0.1 s, brightening, until they all vanish 3 s after the use.
#[derive(Default)]
pub struct TestEffect {
    /// `SetThink(TestThink)` (cleared when the beams go).
    thinking: bool,
    /// `m_iLoop`.
    count: usize,
    beams: [Option<EHandle>; TEST_EFFECT_BEAMS],
    beam_time: [f32; TEST_EFFECT_BEAMS],
    /// `m_flStartTime`.
    start_time: f32,
}

impl TestEffect {
    /// `CTestEffect::TestThink` (0xbcc04).
    fn test_think(&mut self, cx: &mut Cx) {
        let eng = engine();
        let now = eng.time();
        let elapsed = now - self.start_time;
        if self.count < TEST_EFFECT_BEAMS {
            let b = beam_create(c"sprites/lgtning.spr", 100);
            let start = cx.pev().origin;
            let d = [eng.random_float(-1.0, 1.0), eng.random_float(-1.0, 1.0), eng.random_float(-1.0, 1.0)];
            let dir = normalize_or_up(d);
            let tr = eng.trace_line(start, vec::mad(start, dir, 128.0), IGNORE_MONSTERS, cx.edict);
            if !b.is_null() {
                beam_points_init(b, start, tr.vecEndPos);
                let bv = b.v();
                bv.rendercolor = [255.0, 180.0, 100.0];
                bv.scale = 100.0;
                bv.animtime = 12.0;
            }
            self.beam_time[self.count] = now;
            self.beams[self.count] = (!b.is_null()).then(|| EHandle::new(b));
            self.count += 1;
        }
        if elapsed >= 3.0 {
            for h in self.beams[..self.count].iter().flatten() {
                remove(h.get());
            }
            self.start_time = now;
            self.count = 0;
            self.thinking = false;
        } else {
            for i in 0..self.count {
                let amt = beam_brightness(now, self.beam_time[i], self.start_time);
                if let Some(b) = self.beams[i].map(|h| h.get()).filter(|b| !b.is_null()) {
                    b.v().renderamt = amt as f32;
                }
            }
            cx.pev().nextthink = now + 0.1;
        }
    }
}

/// `Vector::Normalize` as TS inlines it: the length on the x87 stack, rounded to a float, then
/// scaled by its reciprocal; `(0, 0, 1)` for a zero vector.
fn normalize_or_up(d: Vec3) -> Vec3 {
    let len = (d[0] as f64 * d[0] as f64 + d[1] as f64 * d[1] as f64 + d[2] as f64 * d[2] as f64).sqrt() as f32;
    if len == 0.0 {
        return [0.0, 0.0, 1.0];
    }
    let k = 1.0 / len;
    [k * d[0], k * d[1], k * d[2]]
}

/// `CTestEffect::TestThink`'s beam brightness: the share of the beam's 3 s life so far × 255,
/// truncated (fistp under the truncating control word, 0xbceb5).
fn beam_brightness(now: f32, born: f32, start: f32) -> i32 {
    let born = born as f64;
    ((now as f64 - born) / ((start as f64 + 3.0) - born) * 255.0) as i32
}

impl Entity for TestEffect {
    /// `CTestEffect::Spawn` (0xbcbc8) / `Precache` (0xbcbdc).
    fn spawn(&mut self, _cx: &mut Cx) {
        engine().precache_model(c"sprites/lgtning.spr");
    }

    fn think(&mut self, cx: &mut Cx) {
        if self.thinking {
            self.test_think(cx);
        }
    }

    /// `CTestEffect::Use` (0xbcf4c).
    fn use_(&mut self, cx: &mut Cx, _a: *mut edict_t, _c: *mut edict_t, _k: UseType, _v: f32) {
        let now = engine().time();
        self.thinking = true;
        cx.pev().nextthink = now + 0.1;
        self.start_time = now;
    }
}

// ---- env_funnel / env_beverage / item_sodacan ---------------------------------------------------------------------

pub const SF_FUNNEL_REVERSE: i32 = 1;

/// `env_funnel` (`CEnvFunnel`): one `TE_LARGEFUNNEL` on use, then it removes itself.
#[derive(Default)]
pub struct EnvFunnel {
    sprite: i32,
    /// `SetThink(SUB_Remove)`.
    removing: bool,
}

impl Entity for EnvFunnel {
    /// `CEnvFunnel::Spawn` (0xbde24) / `Precache` (0xbdcfc).
    fn spawn(&mut self, cx: &mut Cx) {
        self.sprite = engine().precache_model(c"sprites/flare6.spr");
        let v = cx.pev();
        v.solid = SOLID_NOT as i32;
        v.effects = EF_NODRAW as i32;
    }

    fn think(&mut self, cx: &mut Cx) {
        if self.removing {
            sub_remove(cx.edict);
        }
    }

    /// `CEnvFunnel::Use` (0xbdd50).
    fn use_(&mut self, cx: &mut Cx, _a: *mut edict_t, _c: *mut edict_t, _k: UseType, _v: f32) {
        let v = cx.pev();
        let reverse = if v.spawnflags & SF_FUNNEL_REVERSE != 0 { 1 } else { 0 };
        te_begin(MSG_BROADCAST, None, te::LARGEFUNNEL).vec3(v.origin).short(self.sprite).short(reverse).end();
        self.removing = true;
        cx.pev().nextthink = engine().time();
    }
}

/// `env_beverage` (`CEnvBeverage`): each use drops an `item_sodacan` (skin 6 = random 0..5),
/// `health` cans in all (10 by default), the next only once the last one was drunk (`frags`).
#[derive(Default)]
pub struct EnvBeverage;

impl Entity for EnvBeverage {
    /// `CEnvBeverage::Spawn` (0xbdf70) / `Precache` (0xbde50).
    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        eng.precache_model(c"models/can.mdl");
        eng.precache_sound(c"weapons/g_bounce3.wav");
        let v = cx.pev();
        v.solid = SOLID_NOT as i32;
        v.effects = EF_NODRAW as i32;
        v.frags = 0.0;
        if v.health == 0.0 {
            v.health = 10.0;
        }
    }

    /// `CEnvBeverage::Use` (0xbdeac).
    fn use_(&mut self, cx: &mut Cx, _a: *mut edict_t, _c: *mut edict_t, _k: UseType, _v: f32) {
        let me = cx.edict;
        let v = cx.pev();
        if v.frags != 0.0 || v.health <= 0.0 {
            return;
        }
        let (origin, angles, skin) = (v.origin, v.angles, v.skin);
        let can = create_entity(c"item_sodacan", origin, angles, me);
        if !can.is_null() {
            can.v().skin = if skin == 6 { engine().random_long(0, 5) } else { skin };
        }
        let v = cx.pev();
        v.frags = 1.0;
        v.health -= 1.0;
    }
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum SodaState {
    #[default]
    None,
    /// `CanThink`, half a second after spawning.
    Land,
    /// `CanTouch`.
    Touchable,
    /// `SUB_Remove`.
    Remove,
}

/// `item_sodacan` (`CItemSoda`): falls, then a player touching it drinks it (1 health) and lets
/// its `env_beverage` drop the next one.
#[derive(Default)]
pub struct ItemSoda {
    state: SodaState,
}

impl Entity for ItemSoda {
    /// `CItemSoda::Spawn` (0xbe004); `Precache` is empty (the beverage precaches the model).
    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        let v = cx.pev();
        v.solid = SOLID_NOT as i32;
        v.movetype = MOVETYPE_TOSS as i32;
        eng.set_model(cx.edict, c"models/can.mdl");
        eng.set_size(cx.edict, [0.0; 3], [0.0; 3]);
        self.state = SodaState::Land;
        cx.pev().nextthink = eng.time() + 0.5;
    }

    fn think(&mut self, cx: &mut Cx) {
        match self.state {
            // CItemSoda::CanThink (0xbe0c0)
            SodaState::Land => {
                emit_sound_slow(cx.edict, CHAN_WEAPON, c"weapons/g_bounce3.wav", 1.0, ATTN_NORM, 0, PITCH_NORM as i32, 0.5);
                cx.pev().solid = SOLID_TRIGGER as i32;
                engine().set_size(cx.edict, [-8.0, -8.0, 0.0], [8.0, 8.0, 8.0]);
                self.state = SodaState::Touchable;
            }
            SodaState::Remove => sub_remove(cx.edict),
            _ => {}
        }
    }

    /// `CItemSoda::CanTouch` (0xbe178).
    fn touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        if self.state != SodaState::Touchable || !is_player(other) {
            return;
        }
        // TakeHealth(1, DMG_GENERIC); the result isn't checked.
        super::items::player_take_health(other, 1.0);
        let owner = cx.pev().owner;
        if !owner.is_null() && engine().index_of(owner) != 0 {
            owner.v().frags = 0.0;
        }
        let v = cx.pev();
        v.solid = SOLID_NOT as i32;
        v.movetype = MOVETYPE_NONE as i32;
        v.effects = EF_NODRAW as i32;
        self.state = SodaState::Remove;
        cx.pev().nextthink = engine().time();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_effect_brightness() {
        // Used at t = 10: a beam made at 10.5 is half bright at 11.75 (1.25 / 2.5), and full at 13.
        assert_eq!(beam_brightness(11.75, 10.5, 10.0), 127);
        assert_eq!(beam_brightness(13.0, 10.5, 10.0), 255);
        assert_eq!(beam_brightness(10.5, 10.5, 10.0), 0);
    }

    #[test]
    fn normalize_zero_is_up() {
        assert_eq!(normalize_or_up([0.0; 3]), [0.0, 0.0, 1.0]);
        assert_eq!(normalize_or_up([0.0, 3.0, 4.0]), [0.0, 0.6, 0.8]);
    }
}
