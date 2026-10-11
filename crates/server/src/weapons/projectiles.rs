//! Projectiles: the M61 grenade (`CM61Grenade` 0x7c850.., `CTSGrenade::BounceTouch`; its
//! explosion is `entities::stock::explode::ts_explosion`), thrown knives / katanas (`CTSKnife` 0x7d9c0..), plus
//! the slow-motion physics they share with `WorldGun`s (`CTSPhysicObject` 0x7ba54..).

use super::host::emit;
use super::{assets, inventory, melee};
use crate::damage::{self, DamageInfo};
use crate::entities::{self, link_entities};
use crate::player::Player;
use crate::world::{self, Cx, Entity};
use goldsrc::edict::EdictPtr;
use goldsrc::ext::entities::is_player;
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
    if p.ts.slow_others != 0.0 { p.ts.slow_others } else { give_slow_mul(pev.fuser1) }
}

/// `CBaseEntity::GiveSlowMul` (0xac5b4) → `FixedSlowAmt` (0x990bc): the entity's time scale
/// `fuser1`, at least 0.01 (a NaN gives 0.01 too, as the x87 compare does).
pub fn give_slow_mul(fuser1: f32) -> f32 {
    fuser1.max(0.01)
}

/// The ground branch of `CTSFlag::BounceTouch` (0x9dbbc) and `CTSWorldGun::BounceTouch`
/// (0xa0b0c): velocity and spin are scaled by 0.9 while the horizontal speed is above
/// `40 * slow`, and by 0.7 at or below it (floats at objdump 0xf7f44 / 0xf7f50).
/// The engine runs the touch on every frame a sliding entity scrapes the floor, so this is the
/// only ground friction a `MOVETYPE_BOUNCE` entity gets (see docs/implementation/weapons.md).
pub fn bounce_ground_damping(horizontal_speed: f32, slow: f32) -> f32 {
    if horizontal_speed > slow * 40.0 { 0.9 } else { 0.7 }
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
    /// The scale the live values were derived with (`+0x20`).
    last: f32,
}

impl Default for SlowPhysics {
    /// `CTSPhysicObject::Spawn` 0x7ba54.
    fn default() -> Self {
        SlowPhysics { velocity: [0.0; 3], avelocity: [0.0; 3], gravity: 0.0, last: 1.0 }
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

    /// TS's "nothing saved" test: the saved velocity equals `g_vecZero` (0x6baff, 0x6bbc6).
    fn nothing_saved(&self) -> bool {
        self.velocity == [0.0; 3]
    }

    /// `CTSPhysicObject::SavePhysics` 0x7bac0. "Saved" means a non-zero saved velocity, so an
    /// object slowed while at rest saves again on every think: its gravity (and spin) shrink by
    /// the scale each time and the shrunken values are what gets restored.
    fn save(&mut self, pev: &mut entvars_t) {
        let s = pev.fuser1;
        if self.last != s && !self.nothing_saved() {
            pev.velocity = spread::scale(self.velocity, s);
            pev.avelocity = spread::scale(self.avelocity, s);
            pev.gravity = self.gravity * s * s;
            self.last = s;
            return;
        }
        if !self.nothing_saved() {
            return;
        }
        self.velocity = pev.velocity;
        self.avelocity = pev.avelocity;
        self.gravity = pev.gravity;
        pev.velocity = spread::scale(pev.velocity, s);
        pev.avelocity = spread::scale(pev.avelocity, s);
        pev.gravity = pev.gravity * s * s;
        self.last = s;
    }

    /// `CTSPhysicObject::RestorePhysycs` 0x7bcd4: back to the saved (pre-slow) motion, unless the
    /// saved velocity is zero.
    fn restore(&mut self, pev: &mut entvars_t) {
        if self.nothing_saved() {
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
    thrower: Thrower,
    fuse: f32,
    elapsed: f32,
    phys: SlowPhysics,
}

/// `CM61Grenade::CreateGrenade(player, 3.0, origin, velocity)`, plus the throw/roll spin.
/// `time_scale` comes from [`thrower_time_scale`].
pub fn create_grenade(owner: *mut edict_t, origin: Vec3, velocity: Vec3, roll: bool, time_scale: f32) {
    spawn_entity::<Grenade>(c"m61_grenade", |g, cx| {
        g.owner = Some(owner);
        g.thrower = Thrower::new(owner);
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
            impact_damage(me, self.thrower.or(me), 0.1, 0x1080, true);
            self.phys.update(cx.pev());
            return;
        }
        let pev = cx.pev();
        let origin = pev.origin;
        let attacker = self.owner.unwrap_or(std::ptr::null_mut());
        if e.point_contents(origin) != CONTENTS_SKY {
            // 0x7cacc: `TSExplosion(origin, pev, owner, 110, DMG_BLAST | DMG_NEVERGIB, 300, 1024)`.
            crate::entities::stock::explode::ts_explosion(origin, me, attacker, 110.0, 0x1040, 300.0, 1024.0);
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

impl Grenade {
    /// `CM61Grenade::Use` 0x7ce64, from the player's +use (`player::player_use`, the player
    /// is the one being dispatched): kicks a grenade that rests on the ground away along the
    /// player's yaw, 0.75 up, at 625 u/s on top of the player's own velocity.
    pub fn use_by(&mut self, cx: &mut Cx, pcx: &mut Cx) {
        if cx.pev().flags & FL_ONGROUND as i32 == 0 || !Player::is_alive(pcx) {
            return;
        }
        let (v_angle, velocity, effects) = {
            let v = pcx.pev();
            (v.v_angle, v.velocity, v.effects)
        };
        if effects & EF_NODRAW as i32 != 0 {
            return;
        }
        let (fwd, _, _) = spread::angle_vectors([0.0, v_angle[1], v_angle[2]]);
        let dir = spread::normalize(spread::add(fwd, [0.0, 0.0, 0.75]));
        cx.pev().velocity = spread::add(velocity, spread::scale(dir, 625.0));
    }
}

/// Port deviation (issue #1): the player a projectile's flying impact is credited to. TS passes
/// the projectile itself as the attacker (`CM61Grenade::WaitThink` 0x7cacc, `CTSKnife::WaitThink`
/// 0x7ea30, `CTSWorldGun::Fall` 0xa0d84), so a breakable broken in flight isn't the thrower's.
/// The port credits the thrower while it is still the same connected player: the slot must still
/// be a client (`FL_CLIENT`, cleared on disconnect) with the user id it had at the throw (a
/// reconnecting client reuses the edict with a new id). Otherwise [`Thrower::get`] is null and
/// the projectile keeps the credit, as in TS.
#[derive(Clone, Copy, Debug)]
pub struct Thrower {
    e: *mut edict_t,
    user_id: i32,
}

impl Default for Thrower {
    fn default() -> Self {
        Self::NONE
    }
}

impl Thrower {
    pub const NONE: Thrower = Thrower { e: std::ptr::null_mut(), user_id: -1 };

    /// Remembers `e` if it's a connected player.
    pub fn new(e: *mut edict_t) -> Self {
        if !is_player(e) {
            return Self::NONE;
        }
        Thrower { e, user_id: engine().user_id(e) }
    }

    /// The thrower, or null if it left (or its slot now holds someone else).
    pub fn get(&self) -> *mut edict_t {
        let same = self.user_id != -1 && is_player(self.e) && engine().user_id(self.e) == self.user_id;
        if same { self.e } else { std::ptr::null_mut() }
    }

    /// The attacker for a flying impact of `projectile`: the thrower, else the projectile.
    pub fn or(&self, projectile: *mut edict_t) -> *mut edict_t {
        let e = self.get();
        if e.is_null() { projectile } else { e }
    }
}

/// The "flying impact" of grenades, knives and world guns: if the projectile is fast, the
/// entity 64 u ahead takes `speed * k` when that is at least its health (breaks glass, doesn't
/// chip players). Not players for knives (`any_entity` false). `attacker` is
/// [`Thrower::or`] (port deviation, issue #1; TS: the projectile).
fn impact_damage(me: *mut edict_t, attacker: *mut edict_t, k: f32, bits: i32, any_entity: bool) {
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
        let mut info = DamageInfo::new(me, attacker, dmg, bits).traced();
        info.dir = spread::scale(dir, 64.0);
        info.pos = tr.vecEndPos;
        damage::apply(tr.pHit, &info);
    }
}

// ---- thrown knives --------------------------------------------------------------------------

/// `CTSKnife`: a thrown combat knife (btype 6), seal knife (7) or katana (8). Lethal to players
/// on touch while flying, then sticks in walls or lies around as a pickup for 10 s (+use).
#[derive(Default)]
pub struct Knife {
    /// The thrower (`CTSKnife+0x8c`, set by `CreateKnife` 0x7d9e4). TS never sets the blade's
    /// `pev->owner`.
    owner: Option<*mut edict_t>,
    thrower: Thrower,
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
        k.thrower = Thrower::new(owner);
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

/// A thrown knife glows for its thrower (`AddToFullPack` 0xaf180 compares the knife's +0x8c with
/// the host).
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

/// `CTSKnife::WaitTouch` 0x7dce8 on the blade's entvars: on the ground, the velocity halves
/// (0.5 at objdump 0xf71e4), and below 32 u/s (0xf6dac) the blade stops and lies flat (pitch and
/// roll 0). The spin is left alone: the engine clears `avelocity` once the blade is at rest.
/// The engine calls this touch on every frame a sliding blade is pushed into the floor, so it is
/// the blade's only ground friction (see docs/implementation/weapons.md).
fn ground_slide(pev: &mut entvars_t) {
    if pev.flags & FL_ONGROUND as i32 == 0 {
        return;
    }
    pev.velocity = spread::scale(pev.velocity, 0.5);
    if speed(pev) < 32.0 {
        pev.angles[0] = 0.0;
        pev.angles[2] = 0.0;
        pev.velocity = [0.0; 3];
    }
}

/// `CTSKnife::BounceTouch` 0x7decc, world hit: a blade at 100 u/s or more glances off when it
/// meets the surface at a shallow angle (`-normal·dir < 0.4`) and the surface isn't a floor
/// (`normal.z <= 0.98`); otherwise it sticks. The thresholds are doubles (0.4, 0.98) compared
/// on the x87 stack.
fn glances_off(speed: f32, dir: Vec3, normal: Vec3) -> bool {
    let into = -(normal[0] as f64 * dir[0] as f64 + normal[1] as f64 * dir[1] as f64 + normal[2] as f64 * dir[2] as f64);
    speed >= 100.0 && into < 0.4 && normal[2] as f64 <= 0.98
}

/// The pitch of a blade's world hit: `100 + RANDOM_FLOAT(..)`, truncated (`fistp` with the
/// rounding control set to chop).
fn hit_pitch(random: f32) -> i32 {
    (100.0 + random as f64) as i32
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
        // No `pev->owner`: TS keeps the thrower only in the private +0x8c (`self.owner`), so the
        // engine collides the blade with its thrower and `bounce_touch` lets it bounce off.
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
        // `CTSKnife::WaitThink` 0x7ea30 is the think in every state (flying, lying, stuck), so the
        // flying impact runs for a blade that bounced off a player or slides along the floor
        // too. It has no ground friction: that is `WaitTouch`, which the engine runs on every
        // frame a sliding blade is pushed into the floor (see `ground_slide`).
        impact_damage(cx.edict, self.thrower.or(cx.edict), 0.1, 0x0800_1000, false);
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
    /// `CTSKnife::BounceTouch` 0x7decc. The thrower (+0x8c) is ignored: the engine bounces the
    /// blade off them (it has no `pev->owner` to skip them) and they take no damage.
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
            let mut info = DamageInfo::new(me, attacker, self.damage(), 0x0800_1000).traced();
            info.hitgroup = gt.trace_hitgroup;
            info.dir = spread::normalize(pev.velocity);
            info.pos = gt.trace_endpos;
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
            if glances_off(s, dir, normal) {
                if e.point_contents(pev.origin) != CONTENTS_WATER {
                    melee::sparks(pev.origin);
                }
                emit(me, CHAN_BODY, c"weapons/knife/knife_hit.wav", 1.0, 0.8, hit_pitch(e.random_float(0.0, 10.0)));
                let pev = cx.pev();
                pev.velocity = spread::scale(pev.velocity, 0.5);
                return;
            }
            // Stick in the wall, 6 u back along the flight path. TS leaves it `SOLID_NOT`; +use
            // still finds it (`FindEntityInSphere` and the abs box don't depend on `solid`).
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
            if s < 100.0 {
                // A slow blade sticks too, with an extra hit sound on the same channel; the
                // `settle` sound below replaces it (0x7e4aa).
                emit(me, CHAN_BODY, c"weapons/knife/knife_hit.wav", 1.0, 0.8, hit_pitch(e.random_float(-10.0, 0.0)));
            }
        } else if cls == b"knife" {
            return;
        } else {
            let pev = cx.pev();
            pev.velocity = spread::scale(pev.velocity, -0.25);
        }
        self.settle(cx);
    }

    /// `CTSKnife::WaitTouch` 0x7dce8: a blade lying on the ground slows down on every touch
    /// ([`ground_slide`]). Picking it up is `Use` (+use, see [`Knife::use_by`]).
    fn wait_touch(&mut self, cx: &mut Cx, _other: *mut edict_t) {
        ground_slide(cx.pev());
    }

    /// `CTSKnife::Use` 0x7dda4, from the player's +use (`player::player_use`, the player is the
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

    #[test]
    fn ground_bounce_damps_slow_objects_more() {
        assert_eq!(bounce_ground_damping(41.0, 1.0), 0.9);
        assert_eq!(bounce_ground_damping(40.0, 1.0), 0.7);
        // slowed: the threshold scales with the time scale
        assert_eq!(bounce_ground_damping(21.0, 0.5), 0.9);
        assert_eq!(bounce_ground_damping(19.0, 0.5), 0.7);
        // frozen (fuser1 0 -> 0.01): 0.5 u/s is already "fast"
        assert_eq!(bounce_ground_damping(0.5, give_slow_mul(0.0)), 0.9);
    }

    #[test]
    fn slow_mul_has_a_floor() {
        assert_eq!(give_slow_mul(1.0), 1.0);
        assert_eq!(give_slow_mul(0.2), 0.2);
        assert_eq!(give_slow_mul(0.0), 0.01);
        assert_eq!(give_slow_mul(-1.0), 0.01);
        assert_eq!(give_slow_mul(f32::NAN), 0.01);
    }

    fn sliding(velocity: Vec3) -> entvars_t {
        entvars_t { velocity, avelocity: [-1300.0, 0.0, 0.0], angles: [30.0, 45.0, 90.0], flags: FL_ONGROUND as i32, ..Default::default() }
    }

    #[test]
    fn blade_on_the_ground_halves_per_touch_then_lies_flat() {
        let mut v = sliding([200.0, 0.0, 0.0]);
        ground_slide(&mut v);
        assert_eq!(v.velocity, [100.0, 0.0, 0.0]);
        assert_eq!(v.angles, [30.0, 45.0, 90.0]);
        ground_slide(&mut v);
        assert_eq!(v.velocity, [50.0, 0.0, 0.0]);
        // 25 u/s: stops and lies flat; the spin is the engine's to clear
        ground_slide(&mut v);
        assert_eq!(v.velocity, [0.0; 3]);
        assert_eq!(v.angles, [0.0, 45.0, 0.0]);
        assert_eq!(v.avelocity, [-1300.0, 0.0, 0.0]);
        // exactly 64 -> 32 is not below 32
        let mut v = sliding([64.0, 0.0, 0.0]);
        ground_slide(&mut v);
        assert_eq!(v.velocity, [32.0, 0.0, 0.0]);
    }

    #[test]
    fn blade_in_the_air_is_not_slowed_by_touches() {
        let mut v = sliding([200.0, 0.0, -50.0]);
        v.flags = 0;
        ground_slide(&mut v);
        assert_eq!(v.velocity, [200.0, 0.0, -50.0]);
        assert_eq!(v.angles, [30.0, 45.0, 90.0]);
    }

    #[test]
    fn fast_shallow_wall_hits_glance_off() {
        let wall = [-1.0, 0.0, 0.0];
        let shallow = spread::normalize([1.0, 3.0, 0.0]); // -n·d ≈ 0.32
        let steep = spread::normalize([1.0, 1.0, 0.0]); // -n·d ≈ 0.71
        assert!(glances_off(100.0, shallow, wall));
        assert!(!glances_off(99.9, shallow, wall));
        assert!(!glances_off(1900.0, steep, wall));
        // floors (normal.z > 0.98) always stick, however shallow
        let floor = [0.0, 0.0, 1.0];
        assert!(!glances_off(1900.0, spread::normalize([10.0, 0.0, -1.0]), floor));
        // 0.98 is a double: a float normal.z of 0.98f (0.98000001907) is above it
        let ramp = [(1.0f32 - 0.98 * 0.98).sqrt(), 0.0, 0.98];
        assert!(!glances_off(1900.0, [0.0, 1.0, 0.0], ramp));
        let ramp = [0.2, 0.0, 0.979];
        assert!(glances_off(1900.0, [0.0, 1.0, 0.0], ramp));
    }

    #[test]
    fn hit_pitch_truncates() {
        assert_eq!(hit_pitch(0.0), 100);
        assert_eq!(hit_pitch(9.99), 109);
        assert_eq!(hit_pitch(10.0), 110);
        assert_eq!(hit_pitch(-0.5), 99);
        assert_eq!(hit_pitch(-10.0), 90);
    }

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

    /// TS's sentinel is a zero saved velocity: an object slowed at rest saves again every think,
    /// so its gravity shrinks by `s²` each time, and the slow's end restores nothing.
    #[test]
    fn resting_object_gravity_compounds_as_in_ts() {
        let mut v = entvars_t { gravity: 0.75, avelocity: [0.0, 8.0, 0.0], fuser1: 0.5, ..Default::default() };
        let mut ph = SlowPhysics::default();
        for _ in 0..3 {
            ph.update(&mut v);
        }
        assert_eq!(v.gravity, 0.75 * 0.25 * 0.25 * 0.25);
        assert_eq!(v.avelocity, [0.0, 1.0, 0.0]);
        v.fuser1 = 1.0;
        ph.update(&mut v);
        assert_eq!(v.gravity, 0.75 * 0.25 * 0.25 * 0.25);
        assert_eq!(v.avelocity, [0.0, 1.0, 0.0]);
    }
}
