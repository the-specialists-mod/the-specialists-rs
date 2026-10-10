//! Projectiles: the M61 grenade (`CM61Grenade` 0x7c850.., `CTSGrenade::BounceTouch`), the
//! explosion (`TSExplosion` 0x7d01c), and thrown knives / katanas (`CTSKnife` 0x7d9c0..), plus
//! the slow-motion physics they share with `WorldGun`s (`CTSPhysicObject` 0x7ba54..).

use super::host::emit;
use super::{assets, inventory, melee};
use crate::damage::{self, DamageInfo};
use crate::entities::{self, link_entities};
use crate::messages::{self, Msg};
use crate::player::Player;
use crate::world::{self, Cx, Entity};
use goldsrc::edict::EdictPtr;
use goldsrc::ext::weapons::WeaponsEngine;
use goldsrc::server::{DONT_IGNORE_MONSTERS, engine};
use goldsrc::sys::*;
use shared::weapons::id;
use shared::weapons::spread::{self, Vec3};
use std::ffi::CStr;

link_entities! {
    m61_grenade => Grenade,
    knife => Knife,
}

/// Creates a game entity of class `classname` (an exported link name), lets `init` set it up and
/// spawns it. Returns the edict.
pub fn spawn_entity<T: Entity + Default>(classname: &CStr, init: impl FnOnce(&mut T, &mut Cx)) -> *mut edict_t {
    let e = engine().create_named_entity(classname);
    if e.is_null() {
        return e;
    }
    world::dispatch(e, |ent, cx| {
        if let Some(t) = (ent as &mut dyn std::any::Any).downcast_mut::<T>() {
            init(t, cx);
        }
        ent.spawn(cx);
    });
    e
}

fn speed(v: &entvars_t) -> f32 {
    spread::length(v.velocity)
}

// ---- slow motion ----------------------------------------------------------------------------

/// The time scale a thrower hands to its projectile (`CreateGrenade` 0x7c874, `CreateKnife`
/// 0x7d9e4, `CTSWorldGun::CreateWeapon` 0xa07ec): `GoSlow(pl+0x180)` when the thrower is a
/// slow-motion activator (its "others" amount), else `GoSlow(GiveSlowMul())` (`max(fuser1, 0.01)`).
pub fn thrower_time_scale(p: &Player, pev: &entvars_t) -> f32 {
    if p.ts.slow_others != 0.0 { p.ts.slow_others } else { pev.fuser1.max(0.01) }
}

/// `CTSPhysicObject` (0x7ba54..0x7bd98). The engine's physics ignores `fuser1`, so a slowed
/// projectile scales its own motion. While `fuser1 != 1`, the pre-slow velocity, angular velocity
/// and gravity are saved, and the live values become `velocity * s`, `avelocity * s` and
/// `gravity * s²`. When `s` changes they're re-derived from the saved values, and when the slow
/// ends the saved values come back.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SlowPhysics {
    velocity: Vec3,
    avelocity: Vec3,
    gravity: f32,
    /// TS uses "saved velocity != 0" as this flag (see `docs/implementation/weapons.md`).
    saved: bool,
    /// The scale the live values were derived with (`+0x20`).
    last: f32,
}

impl Default for SlowPhysics {
    /// `CTSPhysicObject::Spawn` 0x7ba54.
    fn default() -> Self {
        SlowPhysics { velocity: [0.0; 3], avelocity: [0.0; 3], gravity: 0.0, saved: false, last: 1.0 }
    }
}

impl SlowPhysics {
    /// `CTSPhysicObject::WaitThink` 0x7bd98: `InSlow()` (`fuser1 != 1`) ? `SavePhysics` :
    /// `RestorePhysycs`.
    pub fn update(&mut self, pev: &mut entvars_t) {
        if pev.fuser1 != 1.0 {
            self.save(pev);
        } else {
            self.restore(pev);
        }
    }

    /// `CTSPhysicObject::SavePhysics` 0x7bac0.
    fn save(&mut self, pev: &mut entvars_t) {
        let s = pev.fuser1;
        if self.saved {
            if self.last != s {
                pev.velocity = spread::scale(self.velocity, s);
                pev.avelocity = spread::scale(self.avelocity, s);
                pev.gravity = self.gravity * s * s;
                self.last = s;
            }
            return;
        }
        self.velocity = pev.velocity;
        self.avelocity = pev.avelocity;
        self.gravity = pev.gravity;
        self.saved = true;
        pev.velocity = spread::scale(pev.velocity, s);
        pev.avelocity = spread::scale(pev.avelocity, s);
        pev.gravity *= s * s;
        self.last = s;
    }

    /// `CTSPhysicObject::RestorePhysycs` 0x7bcd4: back to the saved (pre-slow) motion.
    fn restore(&mut self, pev: &mut entvars_t) {
        if !self.saved {
            return;
        }
        pev.velocity = self.velocity;
        pev.avelocity = self.avelocity;
        pev.gravity = self.gravity;
        *self = SlowPhysics { last: 0.0, ..SlowPhysics::default() };
    }
}

// ---- dmgreport ------------------------------------------------------------------------------

/// The `dmgreport > 1` cheat lines (`UTIL_ClientPrintAll(HUD_PRINTTALK, ...)`): bullet damage in
/// `ShootBulletsPlayer`/`CBulletManager::UpdateBullet` and the thrown-blade damage in
/// `CTSKnife::BounceTouch`.
pub fn report_damage(text: impl FnOnce() -> String) {
    const HUD_PRINTTALK: u8 = 3;
    if assets::dmgreport() > 1.0 {
        crate::rules::msg::text_all(HUD_PRINTTALK, &text());
    }
}

// ---- grenade --------------------------------------------------------------------------------

/// `CM61Grenade` (`w_m61.mdl`, BOUNCE, gravity 0.75, friction 0.8). The 3 s fuse runs in the
/// thrower's time: each 0.1 s think adds `0.1 * fuser1`.
#[derive(Default)]
pub struct Grenade {
    owner: Option<*mut edict_t>,
    fuse: f32,
    elapsed: f32,
    phys: SlowPhysics,
}

/// `CM61Grenade::CreateGrenade(player, 3.0, origin, velocity)`, plus the throw/roll spin.
/// `time_scale` comes from [`thrower_time_scale`].
pub fn create_grenade(owner: *mut edict_t, origin: Vec3, velocity: Vec3, roll: bool, time_scale: f32) {
    spawn_entity::<Grenade>(c"m61_grenade", |g, cx| {
        g.owner = Some(owner);
        g.fuse = 3.0;
        let pev = cx.pev();
        pev.origin = origin;
        pev.velocity = velocity;
        pev.owner = owner;
        pev.fuser1 = time_scale;
        if roll {
            pev.friction = 0.4;
            pev.avelocity = [-100.0, 600.0, 0.0];
        } else {
            pev.avelocity = [200.0, 0.0, 0.0];
        }
    });
}

impl Entity for Grenade {
    fn spawn(&mut self, cx: &mut Cx) {
        let e = engine();
        let edict = cx.edict;
        let (origin, friction, fuser1) = {
            let pev = cx.pev();
            (pev.origin, pev.friction, pev.fuser1)
        };
        e.set_model(edict, c"models/w_m61.mdl");
        let pev = cx.pev();
        pev.classname = e.alloc_string(c"grenade");
        pev.movetype = MOVETYPE_BOUNCE as i32;
        pev.solid = SOLID_BBOX as i32;
        pev.gravity = 0.75;
        pev.friction = if friction != 0.0 { friction } else { 0.8 };
        pev.iuser4 = 2;
        pev.fuser1 = if fuser1 > 0.0 { fuser1 } else { 1.0 };
        e.set_size(edict, [0.0; 3], [0.0; 3]);
        e.set_origin(edict, origin);
        pev.nextthink = e.time();
        if self.fuse == 0.0 {
            self.fuse = 3.0;
        }
    }

    fn think(&mut self, cx: &mut Cx) {
        let e = engine();
        let me = cx.edict;
        cx.pev().nextthink = e.time() + 0.1;
        if self.elapsed < self.fuse {
            // The fuse runs in the grenade's own time (`GiveSlowMul`).
            self.elapsed += 0.1 * cx.pev().fuser1.max(0.01);
            impact_damage(me, 0.1, 0x1080, true);
            self.phys.update(cx.pev());
            return;
        }
        let pev = cx.pev();
        let origin = pev.origin;
        let attacker = self.owner.unwrap_or(std::ptr::null_mut());
        if e.point_contents(origin) != CONTENTS_SKY {
            explode(origin, me, attacker, 110.0, 0x1040, 300.0, 1024.0);
        }
        entities::remove(me);
    }

    fn touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        if Some(other) == self.owner {
            return;
        }
        let e = engine();
        let me = cx.edict;
        let pev = cx.pev();
        if pev.flags & FL_ONGROUND as i32 == 0 {
            let s: &CStr = match e.random_long(0, 3) {
                0 => c"weapons/grenbounce1.wav",
                1 => c"weapons/grenbounce2.wav",
                2 => c"weapons/grenbounce3.wav",
                _ => c"weapons/grenbounce4.wav",
            };
            emit(me, CHAN_VOICE, s, 0.25, 0.8, 100);
        } else {
            pev.velocity = spread::scale(pev.velocity, 0.8);
            if speed(pev) < 32.0 {
                pev.angles[0] = 0.0;
                pev.angles[2] = 0.0;
                pev.velocity = [0.0; 3];
            }
        }
    }
}

/// The "flying impact" of grenades, knives and world guns: if the projectile is fast, the
/// entity 64 u ahead takes `speed * k` when that is at least its health (breaks glass, doesn't
/// chip players). `skip_players` for knives.
fn impact_damage(me: *mut edict_t, k: f32, bits: i32, any_entity: bool) {
    let e = engine();
    let v = me.v();
    let s = speed(v);
    if s <= 64.0 {
        return;
    }
    let dir = spread::normalize(v.velocity);
    let tr = e.trace_line(v.origin, spread::ma(v.origin, 64.0, dir), DONT_IGNORE_MONSTERS, me);
    if tr.flFraction >= 1.0 || tr.pHit.is_null() {
        return;
    }
    if !any_entity && e.is_player_edict(tr.pHit) {
        return;
    }
    let target = tr.pHit.v();
    let dmg = s * k;
    if target.takedamage != 0.0 && target.health <= dmg {
        let mut info = DamageInfo::new(me, me, dmg, bits);
        info.dir = spread::scale(dir, 64.0);
        info.pos = tr.vecEndPos;
        damage::apply(tr.pHit, &info);
    }
}

// ---- explosion ------------------------------------------------------------------------------

/// `UTIL_ScreenShake`: every player on the ground within `radius` (SDK falloff disabled).
pub fn screen_shake(center: Vec3, amplitude: f32, frequency: f32, duration: f32, radius: f32) {
    let e = engine();
    let fixed = |v: f32, scale: f32| ((v * scale) as i32).clamp(0, 0xffff);
    for i in 1..=e.max_clients() {
        let p = e.player(i);
        if p.is_null() {
            continue;
        }
        let v = p.v();
        if v.flags & FL_ONGROUND as i32 == 0 {
            continue;
        }
        if radius > 0.0 && spread::length(spread::sub(center, v.origin)) >= radius {
            continue;
        }
        if v.flags & FL_FAKECLIENT as i32 != 0 {
            continue;
        }
        messages::one(Msg::ScreenShake, p)
            .short(fixed(amplitude, 4096.0))
            .short(fixed(duration, 4096.0))
            .short(fixed(frequency, 256.0))
            .end();
    }
}

/// `UTIL_ScreenFade` through TS's `TSFade` message (4.12 fixed-point times).
pub fn screen_fade(p: *mut edict_t, color: [u8; 3], fade: f32, hold: f32, alpha: u8, flags: i32) {
    if p.v().flags & FL_FAKECLIENT as i32 != 0 {
        return;
    }
    let fixed = |v: f32| ((v * 4096.0) as i32).clamp(0, 0xffff);
    messages::one(Msg::TsFade, p)
        .short(fixed(fade))
        .short(fixed(hold))
        .short(flags)
        .byte(color[0] as i32)
        .byte(color[1] as i32)
        .byte(color[2] as i32)
        .byte(alpha as i32)
        .end();
}

fn center(v: &entvars_t) -> Vec3 {
    spread::scale(spread::add(v.absmin, v.absmax), 0.5)
}

/// `TSExplosion(origin, inflictor, attacker, dmg, bits, r_full, r_max)`: full damage inside
/// `r_full`, linear to 0 at `r_max`; occluded targets lose the obstacle's thickness.
pub fn explode(origin: Vec3, inflictor: *mut edict_t, attacker: *mut edict_t, dmg: f32, bits: i32, r_full: f32, r_max: f32) {
    let e = engine();
    let mut org = origin;
    org[2] += 1.0;
    screen_shake(org, dmg, 80.0, 1.0, r_max + r_max);
    messages::all(Msg::TsFrags).coord(org[0]).coord(org[1]).coord(org[2]).coord(dmg.round_ties_even()).coord(0.0).coord(0.0).byte(1).end();
    let falloff = dmg / (r_max - r_full);
    let mut ent = std::ptr::null_mut();
    loop {
        ent = e.find_in_sphere(ent, org, r_max);
        if ent.is_null() {
            break;
        }
        if ent == inflictor {
            continue;
        }
        let v = ent.v();
        let classname = e.string(v.classname);
        if classname.to_bytes() == b"WorldGun" {
            // knocked around by the blast
            let away = spread::normalize(spread::sub(v.origin, org));
            v.velocity = spread::ma(v.velocity, 300.0, away);
            v.velocity[2] += 150.0;
            continue;
        }
        if v.takedamage == 0.0 || v.deadflag == DEAD_DEAD as i32 {
            continue;
        }
        super::assets::debug(|| format!("explosion reaches {} #{}", e.string(v.classname).to_string_lossy(), e.index_of(ent)));
        let is_player = e.is_player_edict(ent);
        if is_player && v.deadflag == DEAD_NO as i32 && v.health > 0.0 {
            screen_fade(ent, [222, 227, 189], 0.1, 0.05, 75, 0);
        }
        let target = center(v);
        let tr = e.trace_line(org, target, DONT_IGNORE_MONSTERS, inflictor);
        let dist = spread::length(spread::sub(org, target));
        let mut d = dmg;
        if dist > r_full {
            d = (dmg - (dist - r_full) * falloff).max(0.0);
        }
        let dir = spread::normalize(spread::sub(target, org));
        if tr.flFraction == 1.0 || tr.pHit == ent {
            hit(ent, inflictor, attacker, d, bits, dir, target);
        } else {
            let first = if tr.fStartSolid != 0 { org } else { tr.vecEndPos };
            let back = e.trace_line(target, org, DONT_IGNORE_MONSTERS, ent);
            if back.flFraction != 1.0 {
                let thickness = spread::length(spread::sub(back.vecEndPos, first));
                if thickness < d {
                    hit(ent, inflictor, attacker, d - thickness, bits, dir, target);
                }
            }
        }
    }
    // The "spectacular" slow-motion point at the blast: 0.25 for 1 s, no bullet time.
    if crate::rules::scoring::do_spectacularness() {
        crate::entities::ts::create_slow_motion_point(org, 0.25, 1.0, false);
    }
}

fn hit(victim: *mut edict_t, inflictor: *mut edict_t, attacker: *mut edict_t, d: f32, bits: i32, dir: Vec3, pos: Vec3) {
    if d <= 0.0 {
        return;
    }
    let attacker = if attacker.is_null() { inflictor } else { attacker };
    let mut info = DamageInfo::new(inflictor, attacker, d, bits);
    info.dir = dir;
    info.pos = pos;
    info.weapon = id::M61 as i32;
    let ok = damage::apply(victim, &info);
    super::assets::debug(|| format!("explosion damage {d:.1} applied={ok} health={}", victim.v().health));
}

// ---- thrown knives --------------------------------------------------------------------------

/// `CTSKnife`: a thrown combat knife (btype 6), seal knife (7) or katana (8). Lethal to players
/// on touch while flying, then sticks in walls or lies around as a pickup for 10 s (+use).
#[derive(Default)]
pub struct Knife {
    owner: Option<*mut edict_t>,
    btype: i32,
    expire: f32,
    flying: bool,
    needs_kungfu: bool,
    stuck: bool,
    phys: SlowPhysics,
}

/// `KnifeFire`/`KatanaThrow` throw: from the eyes + 32 forward + 1 right (traced), 1900 u/s.
/// `time_scale` comes from [`thrower_time_scale`].
pub fn throw_blade(owner: *mut edict_t, btype: i32, time_scale: f32) {
    let e = engine();
    let v = owner.v();
    let (fwd, right, _) = spread::angle_vectors(v.v_angle);
    let eye = spread::add(v.origin, v.view_ofs);
    let from = spread::add(eye, right);
    let to = spread::add(spread::ma(eye, 32.0, fwd), right);
    let tr = e.trace_line(from, to, DONT_IGNORE_MONSTERS, owner);
    let velocity = spread::scale(fwd, 1900.0);
    let src = tr.vecEndPos;
    spawn_entity::<Knife>(c"knife", |k, cx| {
        k.owner = Some(owner);
        k.btype = btype;
        let pev = cx.pev();
        pev.origin = src;
        pev.velocity = velocity;
        let mut ang = engine().vec_to_angles(spread::normalize(velocity));
        ang[2] = 90.0;
        pev.angles = ang;
        pev.avelocity = [if btype == 8 { -700.0 } else { -1300.0 }, 0.0, 0.0];
        pev.fuser1 = time_scale;
    });
}

/// A thrown knife glows for its thrower.
pub fn knife_glows_for(ent: *mut edict_t, host: *mut edict_t) -> bool {
    world::with::<Knife, _>(ent, |k| k.owner == Some(host)).unwrap_or(false)
}

impl Knife {
    fn weapon_id(&self) -> u8 {
        match self.btype {
            8 => id::KATANA,
            6 => id::KNIFE,
            _ => id::SEAL_KNIFE,
        }
    }

    fn damage(&self) -> f32 {
        match self.btype {
            8 => 150.0,
            6 => 100.0,
            _ => 50.0,
        }
    }

    /// Becomes a pickup (`WaitTouch`) for 10 s.
    fn settle(&mut self, cx: &mut Cx) {
        let e = engine();
        let me = cx.edict;
        if e.point_contents(cx.pev().origin) != CONTENTS_WATER {
            melee::sparks(cx.pev().origin);
        }
        emit(me, CHAN_BODY, c"weapons/knife/knife_hit.wav", 1.0, 0.8, 100);
        self.flying = false;
        self.stuck = true;
        let pev = cx.pev();
        pev.nextthink = e.time() + 0.1;
        pev.gravity = 0.75;
        self.expire = e.time() + 10.0;
    }
}

impl Entity for Knife {
    fn spawn(&mut self, cx: &mut Cx) {
        let e = engine();
        let edict = cx.edict;
        let origin = cx.pev().origin;
        let (model, gravity): (&CStr, f32) = match self.btype {
            8 => (c"models/w_katana.mdl", 0.85),
            6 => (c"models/w_knife.mdl", 0.75),
            _ => (c"models/w_sealknife.mdl", 0.3),
        };
        e.set_model(edict, model);
        let pev = cx.pev();
        pev.classname = e.alloc_string(c"knife");
        pev.gravity = gravity;
        pev.friction = 0.8;
        pev.movetype = MOVETYPE_BOUNCE as i32;
        pev.solid = SOLID_BBOX as i32;
        pev.owner = self.owner.unwrap_or(std::ptr::null_mut());
        if pev.fuser1 <= 0.0 {
            pev.fuser1 = 1.0;
        }
        pev.iuser4 = 1;
        e.set_size(edict, [0.0; 3], [0.0; 3]);
        e.set_origin(edict, origin);
        cx.pev().nextthink = e.time();
        self.expire = e.time() + 30.0;
        self.flying = true;
        self.needs_kungfu = false;
    }

    fn think(&mut self, cx: &mut Cx) {
        let e = engine();
        cx.pev().nextthink = e.time() + 0.1;
        if self.expire != 0.0 && e.time() > self.expire {
            entities::remove(cx.edict);
            return;
        }
        if !self.stuck {
            impact_damage(cx.edict, 0.1, 0x0800_1000, false);
        } else {
            let pev = cx.pev();
            if pev.flags & FL_ONGROUND as i32 != 0 {
                pev.velocity = spread::scale(pev.velocity, 0.5);
                pev.avelocity = spread::scale(pev.avelocity, 0.5);
            }
        }
        // A blade stuck in a wall (MOVETYPE_FLY) stays put.
        let pev = cx.pev();
        if pev.movetype != MOVETYPE_FLY as i32 {
            self.phys.update(pev);
        }
    }

    fn touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        if self.stuck {
            self.wait_touch(cx, other);
        } else {
            self.bounce_touch(cx, other);
        }
    }

    fn should_collide(&mut self, _cx: &mut Cx, _other: *mut edict_t) -> bool {
        true
    }
}

impl Knife {
    /// `CTSKnife::BounceTouch` 0x7decc.
    fn bounce_touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        let e = engine();
        let me = cx.edict;
        cx.pev().nextthink = e.time() + 0.1;
        if Some(other) == self.owner || other.is_null() {
            return;
        }
        let ov = other.v();
        if ov.effects & EF_NODRAW as i32 != 0 {
            return;
        }
        if speed(cx.pev()) < 700.0 {
            self.flying = false;
        }
        let dmg = self.damage();
        report_damage(|| format!("(debug){dmg:.6}\n"));
        self.needs_kungfu = false;
        if ov.takedamage != 0.0 && e.is_player_edict(other) {
            let gt = e.globals();
            let pev = cx.pev();
            let attacker = self.owner.unwrap_or(me);
            let mut info = DamageInfo::new(me, attacker, self.damage(), 0x0800_1000);
            info.hitgroup = gt.trace_hitgroup;
            info.dir = spread::normalize(pev.velocity);
            info.pos = gt.trace_endpos;
            info.weapon = self.weapon_id() as i32;
            damage::apply(other, &info);
            let pev = cx.pev();
            pev.velocity = spread::scale(pev.velocity, -0.25);
            emit(me, CHAN_BODY, c"weapons/knife/knife_hitbody.wav", 1.0, 0.8, 100);
            self.flying = false;
            self.stuck = true;
            let pev = cx.pev();
            pev.gravity = 0.75;
            self.expire = e.time() + 10.0;
            return;
        }
        if self.btype == 8 {
            self.needs_kungfu = true;
        }
        let cls = e.string(ov.classname).to_bytes();
        if cls == b"worldspawn" {
            let normal = e.globals().trace_plane_normal;
            let pev = cx.pev();
            let s = speed(pev);
            let dir = spread::normalize(pev.velocity);
            if s >= 100.0 && -spread::dot(normal, dir) < 0.4 && normal[2] <= 0.98 {
                // glancing hit: bounce on
                if e.point_contents(pev.origin) != CONTENTS_WATER {
                    melee::sparks(pev.origin);
                }
                emit(me, CHAN_BODY, c"weapons/knife/knife_hit.wav", 1.0, 0.8, 100 + e.random_long(0, 10));
                let pev = cx.pev();
                pev.velocity = spread::scale(pev.velocity, 0.5);
                return;
            }
            // stick in the wall
            let back = spread::scale(dir, 6.0);
            let org = spread::sub(pev.origin, back);
            e.set_origin(me, org);
            let mut ang = e.vec_to_angles(back);
            ang[2] = 90.0;
            let pev = cx.pev();
            pev.angles = ang;
            pev.solid = SOLID_NOT as i32;
            pev.movetype = MOVETYPE_FLY as i32;
            pev.velocity = [0.0; 3];
            pev.avelocity = [0.0; 3];
            pev.solid = SOLID_TRIGGER as i32;
            e.set_origin(me, org);
        } else if cls == b"knife" {
            return;
        } else {
            let pev = cx.pev();
            pev.velocity = spread::scale(pev.velocity, -0.25);
        }
        self.settle(cx);
    }

    /// `CTSKnife::WaitTouch` 0x7dce8: a blade lying on the ground slows down on every touch.
    /// Picking it up is `Use` (+use, see [`Knife::use_by`]).
    fn wait_touch(&mut self, cx: &mut Cx, _other: *mut edict_t) {
        let pev = cx.pev();
        if pev.flags & FL_ONGROUND as i32 != 0 {
            pev.velocity = spread::scale(pev.velocity, 0.5);
            if speed(pev) < 32.0 {
                pev.angles[0] = 0.0;
                pev.angles[2] = 0.0;
                pev.velocity = [0.0; 3];
                pev.avelocity = [0.0; 3];
            }
        }
    }

    /// `CTSKnife::Use` 0x7dda4, from the player's +use (`weapons::player_use`, the player is the
    /// one being dispatched). A katana that hit something other than a player needs the kung-fu
    /// power-up. An expired blade is just removed.
    pub fn use_by(&mut self, cx: &mut Cx, p: &mut Player, pcx: &mut Cx) {
        let e = engine();
        if self.flying || !Player::is_alive(pcx) || pcx.pev().effects & EF_NODRAW as i32 != 0 {
            return;
        }
        if self.needs_kungfu && !p.kungfu {
            return;
        }
        let now = e.time();
        let mut took = true;
        if now < self.expire {
            took = inventory::pick_up_weapon(p, pcx, self.weapon_id(), None, 0, false, false);
            inventory::sync_speed(p, pcx);
        }
        if took {
            self.expire = now;
            entities::remove(cx.edict);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flying() -> entvars_t {
        entvars_t { velocity: [1000.0, 0.0, 200.0], avelocity: [-1300.0, 0.0, 0.0], gravity: 0.75, fuser1: 1.0, ..Default::default() }
    }

    #[test]
    fn unslowed_projectile_is_untouched() {
        let mut v = flying();
        let mut ph = SlowPhysics::default();
        ph.update(&mut v);
        let f = flying();
        assert_eq!((v.velocity, v.avelocity, v.gravity), (f.velocity, f.avelocity, f.gravity));
        assert_eq!(ph, SlowPhysics::default());
    }

    #[test]
    fn slow_scales_velocity_and_gravity_squared_then_restores() {
        let mut v = flying();
        let mut ph = SlowPhysics::default();
        v.fuser1 = 0.5;
        ph.update(&mut v);
        assert_eq!(v.velocity, [500.0, 0.0, 100.0]);
        assert_eq!(v.avelocity, [-650.0, 0.0, 0.0]);
        assert_eq!(v.gravity, 0.75 * 0.25);
        // Same scale: the engine's motion is left alone.
        v.velocity = [480.0, 0.0, 90.0];
        ph.update(&mut v);
        assert_eq!(v.velocity, [480.0, 0.0, 90.0]);
        // New scale: re-derived from the pre-slow values.
        v.fuser1 = 0.25;
        ph.update(&mut v);
        assert_eq!(v.velocity, [250.0, 0.0, 50.0]);
        assert_eq!(v.gravity, 0.75 * 0.0625);
        // Back to normal time: the pre-slow motion returns.
        v.fuser1 = 1.0;
        ph.update(&mut v);
        assert_eq!(v.velocity, flying().velocity);
        assert_eq!(v.avelocity, flying().avelocity);
        assert_eq!(v.gravity, 0.75);
        // Slowing again saves afresh (the restore leaves `last` at 0).
        v.fuser1 = 0.5;
        ph.update(&mut v);
        assert_eq!(v.velocity, [500.0, 0.0, 100.0]);
    }

    #[test]
    fn resting_object_gravity_does_not_compound() {
        let mut v = entvars_t { gravity: 0.75, fuser1: 0.5, ..Default::default() };
        let mut ph = SlowPhysics::default();
        for _ in 0..5 {
            ph.update(&mut v);
        }
        assert_eq!(v.gravity, 0.75 * 0.25);
        v.fuser1 = 1.0;
        ph.update(&mut v);
        assert_eq!(v.gravity, 0.75);
    }
}
