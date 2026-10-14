//! Xen decoration (SDK xen.cpp; TS links the same classes unchanged, 0xf5064..0xf72ac):
//! `xen_plantlight`, `xen_hair`, `xen_tree` (+ its `xen_ttrigger`), `xen_spore_small`,
//! `xen_spore_medium`, `xen_spore_large` (+ its `xen_hull` leg boxes).

use super::animating::{Animating, act};
use super::base::*;
use super::effects::{Sprite, sprite_set_attachment, sprite_set_transparency};
use super::explode::emit_sound_slow;
use crate::damage::{self, DamageInfo, bits};
use crate::rules::has_player;
use crate::world::{Cx, Entity, UseType};
use goldsrc::edict::EdictPtr;
use goldsrc::ext::entities::{ATTN_NORM, EHandle, Vec3, vec};
use goldsrc::server::engine;
use goldsrc::sys::*;
use shared::pm::plus_double;
use std::ffi::CStr;

crate::entities::link_entities! {
    xen_plantlight   => XenPLight,
    xen_hair         => XenHair,
    xen_ttrigger     => XenTreeTrigger,
    xen_tree         => XenTree,
    xen_spore_small  => XenSpore<0>,
    xen_spore_medium => XenSpore<1>,
    xen_spore_large  => XenSpore<2>,
    xen_hull         => XenHull,
}

const GLOW_SPRITE: &CStr = c"sprites/flare3.spr";
/// `XEN_PLANT_HIDE_TIME`.
const HIDE_TIME: f32 = 5.0;

// ---- xen_plantlight -------------------------------------------------------------------------------

/// `xen_plantlight` (`CXenPLight`): a light plant that ducks (and turns its glow sprite and
/// targets off) when a player touches it, coming back up 5 s after the last touch.
#[derive(Default)]
pub struct XenPLight {
    anim: Animating,
    delay: Delay,
    glow: EHandle,
}

impl XenPLight {
    /// `CXenPLight::LightOn` (0xf56b4).
    fn light_on(&mut self, cx: &mut Cx) {
        let me = cx.edict;
        self.delay.use_targets(me, me, UseType::On, 0.0);
        if self.glow.is_valid() {
            self.glow.get().v().effects &= !(EF_NODRAW as i32);
        }
    }

    /// `CXenPLight::LightOff` (0xf56f8).
    fn light_off(&mut self, cx: &mut Cx) {
        let me = cx.edict;
        self.delay.use_targets(me, me, UseType::Off, 0.0);
        if self.glow.is_valid() {
            self.glow.get().v().effects |= EF_NODRAW as i32;
        }
    }
}

impl Entity for XenPLight {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        self.delay.key_value(key, value)
    }

    /// `CXenPLight::Save` (0xf5180): the `CActAnimating` chain, then `m_pGlow` (+0x84).
    fn save_fields(&mut self, cx: &mut Cx, f: &mut crate::save::Fields) {
        save_act_chain(cx, f, &mut self.delay, &mut self.anim);
        f.table(&crate::save::tables::CXenPLight);
        save_handle(f, &mut self.glow);
    }

    /// `CXenPLight::Precache` (0xf5504).
    fn restored(&mut self, _cx: &mut Cx) {
        plight_precache();
    }

    /// `CXenPLight::Spawn` (0xf5220).
    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        plight_precache();
        eng.set_model(cx.edict, c"models/light.mdl");
        let v = cx.pev();
        v.movetype = MOVETYPE_NONE as i32;
        v.solid = SOLID_TRIGGER as i32;
        eng.set_size(cx.edict, [-80.0, -80.0, 0.0], [80.0, 80.0, 32.0]);
        self.anim.set_activity(cx.edict, act::IDLE);
        cx.pev().nextthink = plus_double(eng.time(), 0.1);
        cx.pev().frame = eng.random_float(0.0, 255.0);
        let v = cx.pev();
        let org = vec::add(v.origin, [0.0, 0.0, (v.mins[2] + v.maxs[2]) * 0.5]);
        let (color, amount, fx) = (v.rendercolor, v.renderamt, v.renderfx);
        let glow = Sprite::create(GLOW_SPRITE, org, false);
        if !glow.is_null() {
            sprite_set_transparency(glow, kRenderGlow as i32, color, amount, fx);
            sprite_set_attachment(glow, cx.edict, 1);
            self.glow = EHandle::new(glow);
        }
    }

    /// `CXenPLight::Think` (0xf5538).
    fn think(&mut self, cx: &mut Cx) {
        let e = cx.edict;
        self.anim.frame_advance(e, 0.0);
        let now = engine().time();
        cx.pev().nextthink = plus_double(now, 0.1);
        match self.anim.activity {
            act::CROUCH if self.anim.finished => {
                self.anim.set_activity(e, act::CROUCHIDLE);
                self.light_off(cx);
            }
            act::CROUCHIDLE if now > cx.pev().dmgtime => {
                self.anim.set_activity(e, act::STAND);
                self.light_on(cx);
            }
            act::STAND if self.anim.finished => self.anim.set_activity(e, act::IDLE),
            _ => {}
        }
    }

    /// `CXenPLight::Touch` (0xf5648).
    fn touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        if has_player(other) {
            cx.pev().dmgtime = engine().time() + HIDE_TIME;
            if self.anim.activity == act::IDLE || self.anim.activity == act::STAND {
                self.anim.set_activity(cx.edict, act::CROUCH);
            }
        }
    }
}

/// `CXenPLight::Precache` (0xf5504).
fn plight_precache() {
    let eng = engine();
    eng.precache_model(c"models/light.mdl");
    eng.precache_model(GLOW_SPRITE);
}

/// A `FIELD_CLASSPTR` the port keeps as a handle: the entity it names now, both ways.
fn save_handle(f: &mut crate::save::Fields, h: &mut EHandle) {
    let mut e = h.get();
    f.ent(&mut e);
    if f.restoring() {
        *h = EHandle::new(e);
    }
}

/// The chain down to `CActAnimating` (`CActAnimating::Save` 0xf5064). No member pointer is ever
/// set: every xen class overrides `Think` and `Touch` instead.
fn save_act_chain(cx: &mut Cx, f: &mut crate::save::Fields, delay: &mut Delay, anim: &mut Animating) {
    f.base_funcs(cx, crate::save::BaseFuncs::default());
    f.delay(&mut delay.delay, &mut delay.kill_target);
    anim.save_act_fields(f);
}

// ---- xen_hair -------------------------------------------------------------------------------------

pub const SF_HAIR_SYNC: i32 = 1;

/// `xen_hair` (`CXenHair`): swaying hair, desynchronised unless spawnflag 1.
#[derive(Default)]
pub struct XenHair {
    anim: Animating,
    /// `CBaseDelay`'s keys (its `KeyValue` is `CBaseDelay::KeyValue`): saved, never used.
    delay: Delay,
}

impl Entity for XenHair {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        self.delay.key_value(key, value)
    }

    /// `CActAnimating::Save` (0xf5064).
    fn save_fields(&mut self, cx: &mut Cx, f: &mut crate::save::Fields) {
        save_act_chain(cx, f, &mut self.delay, &mut self.anim);
    }

    /// `CXenHair::Precache` (0xf58cc).
    fn restored(&mut self, _cx: &mut Cx) {
        engine().precache_model(c"models/hair.mdl");
    }

    /// `CXenHair::Spawn` (0xf5760).
    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        eng.precache_model(c"models/hair.mdl");
        eng.set_model(cx.edict, c"models/hair.mdl");
        eng.set_size(cx.edict, [-4.0, -4.0, 0.0], [4.0, 4.0, 32.0]);
        let v = cx.pev();
        v.sequence = 0;
        if v.spawnflags & SF_HAIR_SYNC == 0 {
            v.frame = eng.random_float(0.0, 255.0);
            v.framerate = eng.random_float(0.7, 1.4);
        }
        // (ResetSequenceInfo sets framerate back to 1, as in the SDK.)
        self.anim.reset_sequence_info(cx.edict);
        let v = cx.pev();
        v.solid = SOLID_NOT as i32;
        v.movetype = MOVETYPE_NONE as i32;
        v.nextthink = eng.time() + eng.random_float(0.1, 0.4);
    }

    /// `CXenHair::Think` (0xf588c).
    fn think(&mut self, cx: &mut Cx) {
        self.anim.frame_advance(cx.edict, 0.0);
        cx.pev().nextthink = engine().time() + 0.5;
    }
}

// ---- xen_tree / xen_ttrigger ------------------------------------------------------------------------

/// `TREE_AE_ATTACK`.
const TREE_AE_ATTACK: i32 = 1;
const TREE_HIT_SOUNDS: [&CStr; 3] = [c"zombie/claw_strike1.wav", c"zombie/claw_strike2.wav", c"zombie/claw_strike3.wav"];
const TREE_MISS_SOUNDS: [&CStr; 2] = [c"zombie/claw_miss1.wav", c"zombie/claw_miss2.wav"];

/// `xen_ttrigger` (`CXenTreeTrigger`): the tree's reach, passing touches to its owner.
#[derive(Default)]
pub struct XenTreeTrigger;

impl Entity for XenTreeTrigger {
    /// `CXenTreeTrigger::Touch` (0xf598c).
    fn touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        let owner = cx.pev().owner;
        if !owner.is_null() {
            crate::ham::touch(owner, other);
        }
    }
}

/// `xen_tree` (`CXenTree`): swings at whatever enters the box 64 units in front of it (25 crush |
/// slash damage, a punch and a shove) when touched or shot.
#[derive(Default)]
pub struct XenTree {
    anim: Animating,
    /// `m_pTrigger`.
    trigger: EHandle,
    /// `CBaseDelay`'s keys (its `KeyValue` is `CBaseDelay::KeyValue`): saved, never used.
    delay: Delay,
}

impl XenTree {
    /// `CXenTree::Attack` (0xf5dc0).
    fn attack(&mut self, cx: &mut Cx) {
        if self.anim.activity == act::IDLE {
            let eng = engine();
            self.anim.set_activity(cx.edict, act::MELEE_ATTACK1);
            cx.pev().framerate = eng.random_float(1.0, 1.4);
            emit_array(cx.edict, &TREE_MISS_SOUNDS);
        }
    }

    /// `CXenTree::HandleAnimEvent` (0xf5e68).
    fn handle_anim_event(&mut self, cx: &mut Cx, event: i32) {
        if event != TREE_AE_ATTACK || !self.trigger.is_valid() {
            return;
        }
        let eng = engine();
        let me = cx.edict;
        let tv = self.trigger.get().v();
        let list = crate::rules::entities_in_box(tv.absmin, tv.absmax, (FL_MONSTER | FL_CLIENT) as i32, 8);
        let (forward, _, _) = eng.angle_vectors(cx.pev().angles);
        let mut hit = false;
        for e in list {
            if e == me || e.v().owner == me {
                continue;
            }
            hit = true;
            damage::apply(e, &DamageInfo::new(me, me, 25.0, bits::CRUSH | bits::SLASH));
            let v = e.v();
            v.punchangle[0] = 15.0;
            v.velocity = vec::mad(v.velocity, forward, 100.0);
        }
        if hit {
            emit_array(me, &TREE_HIT_SOUNDS);
        }
    }
}

/// `EMIT_SOUND_ARRAY_DYN(CHAN_WEAPON, array)`: pitch 95..105, TS slow-motion factor 0.5.
fn emit_array(e: *mut edict_t, sounds: &[&CStr]) {
    let eng = engine();
    let pitch = eng.random_long(95, 105);
    let s = sounds[eng.random_long(0, sounds.len() as i32 - 1) as usize];
    emit_sound_slow(e, CHAN_WEAPON, s, 1.0, ATTN_NORM, 0, pitch, 0.5);
}

/// `CXenTree::Precache` (0xf5ce0).
fn tree_precache() {
    let eng = engine();
    eng.precache_model(c"models/tree.mdl");
    eng.precache_model(GLOW_SPRITE);
    for s in TREE_HIT_SOUNDS.iter().chain(TREE_MISS_SOUNDS.iter()) {
        eng.precache_sound(s);
    }
}

impl Entity for XenTree {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        self.delay.key_value(key, value)
    }

    /// `CXenTree::Save` (0xf5a10): the `CActAnimating` chain, then `m_pTrigger` (+0x84).
    fn save_fields(&mut self, cx: &mut Cx, f: &mut crate::save::Fields) {
        save_act_chain(cx, f, &mut self.delay, &mut self.anim);
        f.table(&crate::save::tables::CXenTree);
        save_handle(f, &mut self.trigger);
    }

    /// `CXenTree::Precache` (0xf5ce0).
    fn restored(&mut self, _cx: &mut Cx) {
        tree_precache();
    }

    /// `CXenTree::Spawn` (0xf5ab0).
    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        tree_precache();
        eng.set_model(cx.edict, c"models/tree.mdl");
        let v = cx.pev();
        v.movetype = MOVETYPE_NONE as i32;
        v.solid = SOLID_BBOX as i32;
        v.takedamage = DAMAGE_YES as f32;
        eng.set_size(cx.edict, [-30.0, -30.0, 0.0], [30.0, 30.0, 188.0]);
        self.anim.set_activity(cx.edict, act::IDLE);
        cx.pev().nextthink = plus_double(eng.time(), 0.1);
        cx.pev().frame = eng.random_float(0.0, 255.0);
        cx.pev().framerate = eng.random_float(0.7, 1.4);
        let (forward, _, _) = eng.angle_vectors(cx.pev().angles);
        let pos = vec::mad(cx.pev().origin, forward, 64.0);
        // CXenTreeTrigger::TriggerCreate (0xf5918)
        let t = eng.create_named_entity(c"xen_ttrigger");
        if !t.is_null() {
            let tv = t.v();
            tv.origin = pos;
            tv.solid = SOLID_TRIGGER as i32;
            tv.movetype = MOVETYPE_NONE as i32;
            tv.owner = cx.edict;
            eng.set_size(t, [-24.0, -24.0, 0.0], [24.0, 24.0, 128.0]);
            self.trigger = EHandle::new(t);
        }
    }

    /// `CXenTree::Think` (0xf6228).
    fn think(&mut self, cx: &mut Cx) {
        let eng = engine();
        self.anim.frame_advance(cx.edict, 0.0);
        cx.pev().nextthink = plus_double(eng.time(), 0.1);
        for event in self.anim.anim_events(cx.edict) {
            self.handle_anim_event(cx, event);
        }
        if self.anim.activity == act::MELEE_ATTACK1 && self.anim.finished {
            self.anim.set_activity(cx.edict, act::IDLE);
            cx.pev().framerate = eng.random_float(0.6, 1.4);
        }
    }

    /// `CXenTree::Touch` (0xf5d58): anything but a big momma.
    fn touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        if !has_player(other) && s(other.v().classname) == c"monster_bigmomma" {
            return;
        }
        self.attack(cx);
    }

    /// `CXenTree::TakeDamage` (0xf7108).
    fn take_damage(&mut self, cx: &mut Cx, _info: &DamageInfo) -> bool {
        self.attack(cx);
        false
    }
}

// ---- xen_spore_* / xen_hull -----------------------------------------------------------------------

/// `CXenSpore::pModelNames`, by skin.
const SPORE_MODELS: [&CStr; 3] = [c"models/fungus(small).mdl", c"models/fungus.mdl", c"models/fungus(large).mdl"];

/// `CXenSporeLarge::m_hullSizes`: the leg boxes, as (forward, right) offsets.
const LARGE_HULLS: [[f32; 2]; 5] = [[90.0, -25.0], [25.0, 75.0], [-15.0, -100.0], [-90.0, -35.0], [-90.0, 60.0]];

/// `xen_spore_small` (`SKIN` 0), `xen_spore_medium` (1) and `xen_spore_large` (2) (`CXenSpore`
/// subclasses): animated fungus. Damage is absorbed. The large one adds five `xen_hull` boxes for
/// its legs.
#[derive(Default)]
pub struct XenSpore<const SKIN: i32> {
    anim: Animating,
    /// `CBaseDelay`'s keys (its `KeyValue` is `CBaseDelay::KeyValue`): saved, never used.
    delay: Delay,
}

/// `CXenSpore::Precache` (0xf6958): the model of `pev->skin`.
///
/// Guard: TS indexes `pModelNames` with any (restored) skin; the port skips one outside it.
fn spore_precache(skin: i32) {
    if let Some(m) = usize::try_from(skin).ok().and_then(|i| SPORE_MODELS.get(i)) {
        engine().precache_model(m);
    }
}

impl<const SKIN: i32> Entity for XenSpore<SKIN> {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        self.delay.key_value(key, value)
    }

    /// `CActAnimating::Save` (0xf5064).
    fn save_fields(&mut self, cx: &mut Cx, f: &mut crate::save::Fields) {
        save_act_chain(cx, f, &mut self.delay, &mut self.anim);
    }

    /// `CXenSpore::Precache` (0xf6958).
    fn restored(&mut self, cx: &mut Cx) {
        spore_precache(cx.pev().skin);
    }

    /// `CXenSporeSmall/Med/Large::Spawn` (0xf6450, 0xf64c8, 0xf6540) around `CXenSpore::Spawn`
    /// (0xf6880).
    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        cx.pev().skin = SKIN;
        spore_precache(SKIN);
        eng.set_model(cx.edict, SPORE_MODELS[SKIN as usize]);
        let v = cx.pev();
        v.movetype = MOVETYPE_NONE as i32;
        v.solid = SOLID_BBOX as i32;
        v.takedamage = DAMAGE_YES as f32;
        v.sequence = 0;
        v.frame = eng.random_float(0.0, 255.0);
        v.framerate = eng.random_float(0.7, 1.4);
        self.anim.reset_sequence_info(cx.edict);
        cx.pev().nextthink = eng.time() + eng.random_float(0.1, 0.4);
        match SKIN {
            0 => eng.set_size(cx.edict, [-16.0, -16.0, 0.0], [16.0, 16.0, 64.0]),
            1 => eng.set_size(cx.edict, [-40.0, -40.0, 0.0], [40.0, 40.0, 120.0]),
            _ => {
                eng.set_size(cx.edict, [-48.0, -48.0, 110.0], [48.0, 48.0, 240.0]);
                let (forward, right, _) = eng.angle_vectors(cx.pev().angles);
                for [f, r] in LARGE_HULLS {
                    let offset = vec::add(vec::scale(forward, f), vec::scale(right, r));
                    create_hull(cx.edict, [-12.0, -12.0, 0.0], [12.0, 12.0, 120.0], offset);
                }
            }
        }
    }

    /// `CXenSpore::Think` (0xf6994).
    fn think(&mut self, cx: &mut Cx) {
        self.anim.frame_advance(cx.edict, 0.0);
        cx.pev().nextthink = plus_double(engine().time(), 0.1);
    }

    /// `CXenSpore::TakeDamage` (0xf7180): `Attack()` is empty.
    fn take_damage(&mut self, _cx: &mut Cx, _info: &DamageInfo) -> bool {
        false
    }
}

/// `xen_hull` (`CXenHull`): an invisible solid box (a large spore's leg). Placed in a map it's a
/// plain point entity.
#[derive(Default)]
pub struct XenHull;

impl Entity for XenHull {
    /// `CPointEntity::Spawn`.
    fn spawn(&mut self, cx: &mut Cx) {
        cx.pev().solid = SOLID_NOT as i32;
    }
}

/// `CXenHull::CreateHull` (0xf62bc).
fn create_hull(source: *mut edict_t, mins: Vec3, maxs: Vec3, offset: Vec3) {
    let eng = engine();
    let h = eng.create_named_entity(c"xen_hull");
    if h.is_null() {
        return;
    }
    let sv = source.v();
    eng.set_origin(h, vec::add(sv.origin, offset));
    eng.set_model(h, s(sv.model));
    let v = h.v();
    v.solid = SOLID_BBOX as i32;
    v.movetype = MOVETYPE_NONE as i32;
    v.owner = source;
    eng.set_size(h, mins, maxs);
    let v = h.v();
    v.renderamt = 0.0;
    v.rendermode = kRenderTransTexture as i32;
}
