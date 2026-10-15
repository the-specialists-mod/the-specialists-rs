//! Projectiles: the M61 grenade (`CM61Grenade` 0x7c850.., `CTSGrenade::BounceTouch`; its
//! explosion is `entities::stock::explode::ts_explosion`), thrown knives / katanas (`CTSKnife` 0x7d9c0..), plus
//! the slow-motion physics they share with `WorldGun`s (`CTSPhysicObject` 0x7ba54..).

use super::host::emit;
use super::{assets, inventory, melee};
use crate::damage::{self, DamageInfo};
use crate::entities::{self, link_entities};
use crate::player::Player;
use crate::save::{BaseFuncs, Fields};
use crate::world::{self, Cx, Entity};
use goldsrc::edict::EdictPtr;
use goldsrc::ext::entities::is_player;
use goldsrc::server::{DONT_IGNORE_MONSTERS, engine};
use goldsrc::sys::*;
use shared::pm::{Arith, Ext, Fpu, FpuMath, Num, ORDER_XYZ, below87, eq87, not_above87, plus_double};
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
    });
    crate::ham::spawn(e);
    e
}

/// The ground stop of `CTSGrenade::BounceTouch` and `CTSKnife::WaitTouch`: `Length() < 32`,
/// summed `x² + y² + z²`; `ts_i386.so` stores the root (0x7be9e, 0x7dd5e), mp.dll tests it
/// unrounded (0x100504d3, 0x10051f6f).
fn below_32(v: &entvars_t) -> bool {
    shared::by_arith!(crate::rules::cvars::pm_arith(), below_32_in(v.velocity))
}

fn below_32_in<F: Fpu>(v: Vec3) -> bool {
    F::vec_length(v, ORDER_XYZ) < F::x(32.0)
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
///
/// The stored speed is tested against `GiveSlowMul() * 40.0f` unrounded (mp.dll 0x1007234b /
/// 0x100744bb, `ts_i386.so` 0xa0b96), which is the exact `float` product.
pub fn bounce_ground_damping(horizontal_speed: f32, slow: f32) -> f32 {
    if f64::from(horizontal_speed) > f64::from(slow) * 40.0 { 0.9 } else { 0.7 }
}

/// `BounceTouch`'s drop-sound speed test: the stored speed above the unrounded (exact)
/// `GiveSlowMul() * 18.0f` (mp.dll 0x10074673, `ts_i386.so` 0xa0cc1).
pub fn bounce_drops_loudly(speed: f32, slow: f32) -> bool {
    f64::from(speed) > f64::from(slow) * 18.0
}

/// The speeds `CTSFlag::BounceTouch` and `CTSWorldGun::BounceTouch` test, both stored as `float`s
/// in both builds: the horizontal `x² + y²` (0xa0b84 / mp.dll 0x100744ae) and the full
/// `x² + y² + z²` (0xa0cb1 / 0x10074664), each summed on the build's FPU.
pub fn bounce_speeds(arith: Arith, v: Vec3) -> (f32, f32) {
    fn go<F: Fpu>(v: Vec3) -> (f32, f32) {
        let h = (F::x(v[0]) * F::x(v[0]) + F::x(v[1]) * F::x(v[1])).sqrt().f();
        (h, F::vec_length(v, ORDER_XYZ).f())
    }
    shared::by_arith!(arith, go(v))
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

/// `CTSGrenade::Precache` (0x7bdd8), the `Precache` of both `CM61Grenade` and `CTSKnife`.
fn grenade_precache() {
    let e = engine();
    for s in [c"weapons/grenbounce1.wav", c"weapons/grenbounce2.wav", c"weapons/grenbounce3.wav", c"weapons/grenbounce4.wav"] {
        e.precache_sound(s);
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
    /// TS's member pointers: `m_pfnThink` (`WaitThink`) and `m_pfnTouch` (`BounceTouch`), set by
    /// `Spawn` (0x7c974).
    funcs: BaseFuncs,
}

/// One fuse think: `elapsed = GiveSlowMul() * 0.1f + elapsed`, rounded once (`ts_i386.so`
/// 0x7cbbb..0x7cbca, mp.dll 0x10051096..0x100510a5; issue #150).
pub fn fuse_step(arith: Arith, elapsed: f32, slow: f32) -> f32 {
    fn go<F: Fpu>(elapsed: f32, slow: f32) -> f32 {
        (F::x(slow) * F::x(0.1) + F::x(elapsed)).f()
    }
    shared::by_arith!(arith, go(elapsed, slow))
}

/// `CM61Grenade::CreateGrenade(player, 3.0, origin, velocity)`, plus the throw/roll spin.
/// `time_scale` comes from [`thrower_time_scale`].
pub fn create_grenade(owner: *mut edict_t, origin: Vec3, velocity: Vec3, roll: bool, time_scale: f32) {
    spawn_entity::<Grenade>(c"m61_grenade", |g, cx| {
        g.owner = Some(owner);
        g.thrower = Thrower::new(owner);
        note_thrower(cx.edict, g.thrower);
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
    /// `CBaseEntity::Save` / `Restore`: the fuse and thrower aren't saved; both pointers are set,
    /// so TS's restore stops after `"BASE"` and the restored grenade never goes off.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut Fields) {
        crate::entities::ts::save_base(cx, f, &mut self.funcs);
    }

    fn restored(&mut self, _cx: &mut Cx) {
        grenade_precache();
    }

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
        self.funcs = BaseFuncs { think: true, touch: true, ..BaseFuncs::default() };
    }

    fn think(&mut self, cx: &mut Cx) {
        if !self.funcs.think {
            return;
        }
        let e = engine();
        let me = cx.edict;
        cx.pev().nextthink = plus_double(e.time(), 0.1);
        if self.elapsed < self.fuse {
            // The fuse runs in the grenade's own time (`GiveSlowMul`).
            self.elapsed = crate::gameplay::active().server.grenade_fuse_step(self.elapsed, cx.pev().fuser1);
            impact_damage(me, self.thrower.or(me), 0x1080, true);
            self.phys.update(cx.pev());
            return;
        }
        let pev = cx.pev();
        let origin = pev.origin;
        let attacker = self.owner.unwrap_or(std::ptr::null_mut());
        // 0x7cacc: only `IsInWorld()` (vtable slot 49); no sky test, so a grenade in the sky
        // explodes too.
        if crate::ham::is_in_world(me, || is_in_world(me.v())) {
            // `TSExplosion(origin, pev, owner, 110, DMG_BLAST | DMG_NEVERGIB, 300, 1024)`
            // (`ServerRules::grenade_explosion`).
            let (dmg, r_full, r_max) = crate::gameplay::active().server.grenade_explosion();
            crate::entities::stock::explode::ts_explosion(origin, me, attacker, dmg, 0x1040, r_full, r_max);
        }
        entities::remove(me);
    }

    fn touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        if !self.funcs.touch || Some(other) == self.owner {
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
            if below_32(pev) {
                pev.angles[0] = 0.0;
                pev.angles[2] = 0.0;
                pev.velocity = [0.0; 3];
            }
        }
    }
}

/// `CBaseEntity::IsInWorld` (0xac2ec): the origin strictly inside ±4096 and the velocity strictly
/// inside ±2000 on every axis. The tests are TS's, in its order, so NaNs fall through the same way
/// (only the last one, `-2000 < velocity.z`, rejects a NaN).
pub(crate) fn is_in_world(v: &entvars_t) -> bool {
    let (o, s) = (v.origin, v.velocity);
    let out = 4096.0 <= o[0]
        || 4096.0 <= o[1]
        || 4096.0 <= o[2]
        || o[0] <= -4096.0
        || o[1] <= -4096.0
        || o[2] <= -4096.0
        || 2000.0 <= s[0]
        || 2000.0 <= s[1]
        || 2000.0 <= s[2]
        || s[0] <= -2000.0
        || s[1] <= -2000.0;
    !out && -2000.0 < s[2]
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
        let dir = kick_dir(crate::rules::cvars::pm_arith(), fwd);
        cx.pev().velocity = spread::add(velocity, spread::scale(dir, 625.0));
    }
}

/// `CM61Grenade::Use`'s `(v_forward + Vector(0, 0, 0.75)).Normalize()`. z (`fz + 0.75`) stays
/// unrounded in its square: `ts_i386.so` (0x7cf07..0x7cfb8) squares x and y by their stored
/// copies and z by itself, sums `x, y, z`, stores the length and normalises the stored
/// components; mp.dll (0x10051340..0x100513bb) squares z by its stored copy, sums `z, y, x`, keeps
/// the length and `1 / length` unrounded and normalises the stored z.
fn kick_dir(arith: Arith, fwd: Vec3) -> Vec3 {
    fn go<F: Fpu>(fwd: Vec3) -> Vec3 {
        let (x, y) = (F::x(fwd[0]), F::x(fwd[1]));
        let zu = F::x(fwd[2]) + F::x(0.75);
        let zs = zu.f();
        let sum = if F::MSVC { (zu * F::x(zs) + y * y) + x * x } else { (x * x + y * y) + zu * zu };
        let len = sum.sqrt();
        let len = if F::MSVC { len } else { F::x(len.f()) };
        if eq87(len, F::ZERO) {
            return [0.0, 0.0, 1.0];
        }
        let inv = F::x(1.0) / len;
        [(inv * x).f(), (inv * y).f(), (inv * F::x(zs)).f()]
    }
    shared::by_arith!(arith, go(fwd))
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

/// Port addition (plugin API `projectile_owner`, `crate::plugin_api`): the [`Thrower`] of every
/// grenade, thrown knife and thrown gun, by edict index with the edict's serial number. It
/// answers while the projectile's own call is on the stack (its object out of the slab), and a
/// reused slot doesn't match the old serial. Never cleared: entries go stale with their edict.
#[derive(Default)]
pub(crate) struct Throwers(Vec<Option<(i32, Thrower)>>);

impl Throwers {
    fn note(&mut self, index: usize, serial: i32, t: Thrower) {
        if self.0.len() <= index {
            self.0.resize(index + 1, None);
        }
        self.0[index] = Some((serial, t));
    }

    fn get(&self, index: usize, serial: i32) -> Option<Thrower> {
        self.0.get(index).copied().flatten().filter(|&(s, _)| s == serial).map(|(_, t)| t)
    }
}

static THROWERS: goldsrc::Global<std::cell::RefCell<Throwers>> = goldsrc::Global(std::cell::RefCell::new(Throwers(Vec::new())));

/// Records the thrower of a new projectile (see [`Throwers`]).
pub(crate) fn note_thrower(e: *mut edict_t, t: Thrower) {
    let (index, serial) = (engine().index_of(e), e.serial_number());
    THROWERS.0.borrow_mut().note(index as usize, serial, t);
}

/// The player who threw the grenade, knife or gun `e`: `Some(null)` once they left (as
/// [`Thrower::get`]), `None` when `e` isn't one of those.
pub fn thrower_of(e: *mut edict_t) -> Option<*mut edict_t> {
    let (index, serial) = (engine().index_of(e), e.serial_number());
    let t = THROWERS.0.borrow().get(index as usize, serial);
    t.map(|t| t.get())
}

/// Whether a flying impact of `dmg` calls the target's `TraceAttack`: when it is at least the
/// target's health. TS tests nothing else (`CM61Grenade::WaitThink` 0x7cacc, `CTSKnife::WaitThink`
/// 0x7ea30, `CTSWorldGun::Fall` 0xa0d84), not `takedamage` either: a target that takes no damage
/// still gets the TraceAttack's hit effects (a breakable's sparks and ricochets), which leaves the
/// damage out itself (issues #25, #26). `dmg` is the value TS compares, which mp.dll leaves
/// unrounded and `ts_i386.so` stores first ([`flying_impact`]); a NaN never hits.
pub(crate) fn impact_hits(target: &entvars_t, dmg: Ext) -> bool {
    Ext::from_f32(target.health) <= dmg
}

/// A grenade's or knife's flying impact at `velocity` (issue #150): `None` unless the speed is
/// over 64 (a NaN isn't), else the direction it traces along and the damage (`speed` times the
/// `double` 0.1) as dealt and as tested against the target's health. The speed and direction are
/// the inline `Length` and `Normalize` (`x² + y² + z²`); mp.dll (`CM61Grenade::WaitThink`
/// 0x100510a8..0x10051245, `CTSKnife::WaitThink` 0x10052b6f..0x10052d1e) keeps the root
/// unrounded and tests the unrounded product, `ts_i386.so` (0x7cbd8..0x7cd69, 0x7ec3b) stores
/// the root and the product.
pub(crate) fn flying_impact(arith: Arith, velocity: Vec3) -> Option<(Vec3, f32, Ext)> {
    fn go<F: Fpu>(v: Vec3) -> Option<(Vec3, f32, Ext)> {
        let s = F::vec_length(v, ORDER_XYZ);
        // `test ah,0x41; jne`: 64 or less, or NaN, skips.
        if not_above87(s, F::x(64.0)) {
            return None;
        }
        let p = s * F::xd(0.1);
        let dmg = p.f();
        Some((F::vec_normalize(v, ORDER_XYZ), dmg, if F::MSVC { p.ext() } else { Ext::from_f32(dmg) }))
    }
    shared::by_arith!(arith, go(velocity))
}

/// The landing test of a falling `WorldGun` or flag (`CTSWorldGun::Fall` 0xa0d84, `CTSFlag::Fall`
/// 0x9df10): `|velocity| < GiveSlowMul()·3`, the speed (`x² + y² + z²`) stored as a `float` and
/// the product left unrounded (mp.dll 0x1007478e..0x100747c6, 0x1007269e..0x100726d6;
/// `ts_i386.so` 0xa0dcd..0xa0dfd).
pub(crate) fn fall_landed(arith: Arith, velocity: Vec3, slow: f32) -> bool {
    fn go<F: Fpu>(v: Vec3, slow: f32) -> bool {
        F::x(F::sq_in(v, ORDER_XYZ).sqrt().f()) < F::x(slow) * F::x(3.0)
    }
    shared::by_arith!(arith, go(velocity, slow))
}

/// The impact trace's end of a falling `WorldGun` or flag, `origin + velocity·0.1f`: the x
/// product stays on the FPU for its add, y and z are stored first (mp.dll 0x10074837..0x1007488f,
/// 0x1007274c..0x100727a4; `ts_i386.so` 0xa0e8f..0xa0ed5).
pub(crate) fn fall_trace_end(arith: Arith, origin: Vec3, velocity: Vec3) -> Vec3 {
    fn go<F: Fpu>(o: Vec3, v: Vec3) -> Vec3 {
        let p = v.map(|c| F::x(c) * F::x(0.1));
        [(p[0] + F::x(o[0])).f(), (F::x(p[1].f()) + F::x(o[1])).f(), (F::x(p[2].f()) + F::x(o[2])).f()]
    }
    shared::by_arith!(arith, go(origin, velocity))
}

/// The "flying impact" of grenades and knives (`CM61Grenade::WaitThink` 0x7cacc,
/// `CTSKnife::WaitThink` 0x7ea30): if the projectile is faster than 64 u/s, the entity 64 u
/// ahead gets its `TraceAttack` for `speed * 0.1` when that is at least its health (breaks
/// glass, doesn't chip players; see [`impact_hits`] and [`flying_impact`]). Not players for
/// knives (`any_entity` false). `attacker` is [`Thrower::or`] (port deviation, issue #1; TS: the
/// projectile).
fn impact_damage(me: *mut edict_t, attacker: *mut edict_t, bits: i32, any_entity: bool) {
    let e = engine();
    let v = me.v();
    let Some((dir, dmg, test)) = flying_impact(crate::rules::cvars::pm_arith(), v.velocity) else {
        return;
    };
    // `64·dir` is exact, so the end is rounded once in either build.
    let tr = e.trace_line(v.origin, spread::ma(v.origin, 64.0, dir), DONT_IGNORE_MONSTERS, me);
    if tr.flFraction >= 1.0 || tr.pHit.is_null() {
        return;
    }
    if !any_entity && crate::rules::has_player(tr.pHit) {
        return;
    }
    if impact_hits(tr.pHit.v(), test) {
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
    /// Where it was thrown from (`CreateKnife`'s origin), for a ruleset whose damage falls off
    /// with the distance flown.
    thrown_from: Vec3,
    expire: f32,
    flying: bool,
    needs_kungfu: bool,
    stuck: bool,
    phys: SlowPhysics,
    /// TS's member pointers: `m_pfnThink` (`WaitThink`) and `m_pfnTouch` (`BounceTouch`, then
    /// `WaitTouch`), set from `Spawn` (0x7db54) on.
    funcs: BaseFuncs,
}

/// The speed of a thrown blade: `KnifeFire` throws knives at 1900 u/s (objdump 0x7465b /
/// 0x746a7), `KatanaThrow` the katana at 1000 u/s (0x73dc1, the float at 0xf7f64).
fn throw_speed(btype: i32) -> f32 {
    if btype == 8 { 1000.0 } else { 1900.0 }
}

/// What a ruleset decides about a thrown blade
/// ([`ServerRules::thrown_blade`](crate::gameplay::ServerRules::thrown_blade)).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThrownBlade {
    /// Throw speed in u/s.
    pub speed: f32,
    /// `pev->gravity` from `Spawn` (it becomes 0.75 once the blade has hit something).
    pub gravity: f32,
    /// Damage bits of its hit on a player and of its flying impact.
    pub bits: i32,
}

/// TS 3.0's thrown blades: [`throw_speed`]; gravity 0.85 katana, 0.75 combat knife, 0.3 seal
/// knife (`CTSKnife::Spawn` 0x7db54); bits `0x08001000` (thrown: the ×2 headshot).
pub fn thrown_blade(btype: i32) -> ThrownBlade {
    let gravity = match btype {
        8 => 0.85,
        6 => 0.75,
        _ => 0.3,
    };
    ThrownBlade { speed: throw_speed(btype), gravity, bits: 0x0800_1000 }
}

/// TS 3.0's thrown-blade damage (`CTSKnife::BounceTouch` 0x7decc): 150 katana, 100 combat
/// knife, 50 seal knife, however far it flew.
pub fn thrown_blade_damage(btype: i32) -> f32 {
    match btype {
        8 => 150.0,
        6 => 100.0,
        _ => 50.0,
    }
}

fn blade_dir(velocity: Vec3) -> Vec3 {
    fn go<F: Fpu>(v: Vec3) -> Vec3 {
        F::vec_normalize(v, ORDER_XYZ)
    }
    shared::by_arith!(crate::rules::cvars::pm_arith(), go(velocity))
}

/// `KnifeFire`/`KatanaThrow` throw: from the eyes + 32 forward + 1 right (traced), at
/// [`throw_speed`]. `time_scale` comes from [`thrower_time_scale`].
pub fn throw_blade(owner: *mut edict_t, btype: i32, time_scale: f32) {
    let e = engine();
    let v = owner.v();
    let (fwd, right, _) = spread::angle_vectors(v.v_angle);
    let eye = spread::add(v.origin, v.view_ofs);
    let from = spread::add(eye, right);
    let to = spread::add(spread::ma(eye, 32.0, fwd), right);
    let tr = e.trace_line(from, to, DONT_IGNORE_MONSTERS, owner);
    let velocity = spread::scale(fwd, crate::gameplay::active().server.thrown_blade(btype).speed);
    let src = tr.vecEndPos;
    spawn_entity::<Knife>(c"knife", |k, cx| {
        k.owner = Some(owner);
        k.thrower = Thrower::new(owner);
        note_thrower(cx.edict, k.thrower);
        k.btype = btype;
        k.thrown_from = src;
        let pev = cx.pev();
        pev.origin = src;
        pev.velocity = velocity;
        // `KnifeFire` / `KatanaThrow` normalise the velocity inline, `x² + y² + z²` (0x84790,
        // mp.dll's out-of-line `Length` at 0x100570b3 and inline 0x1005775d).
        let mut ang = engine().vec_to_angles(blade_dir(velocity));
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

/// `CTSKnife::GetDescription` (0x7db04) on the blade's type (`+0xa0`).
fn knife_description(btype: i32) -> &'static str {
    match btype {
        6 => "Throwing Combat Knife",
        7 => "Throwing Seal Knife",
        8 => "Throwing Katana",
        _ => "",
    }
}

/// `CTSKnife::GetDescription` of the thrown knife `e`, as `PlayerKilled` and
/// `TSGetPointsForFrag` read it. The blade lends itself back to its slot while its own touch or
/// think deals the damage (`world::lend`), so the kill it scores can read it. `None` when `e`
/// has no knife object (TS: NULL private data).
pub fn thrown_knife_description(e: *mut edict_t) -> Option<&'static str> {
    world::with_lent::<Knife, _>(e, |k| knife_description(k.btype))
}

impl Knife {
    fn weapon_id(&self) -> u8 {
        match self.btype {
            8 => id::KATANA,
            6 => id::KNIFE,
            _ => id::SEAL_KNIFE,
        }
    }

    /// What the blade deals the player it hits from where it is now
    /// ([`ServerRules::thrown_blade_damage`](crate::gameplay::ServerRules::thrown_blade_damage)).
    fn damage(&self, origin: Vec3) -> f32 {
        crate::gameplay::active().server.thrown_blade_damage(self.btype, self.thrown_from, origin)
    }

    fn bits(&self) -> i32 {
        crate::gameplay::active().server.thrown_blade(self.btype).bits
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
        pev.nextthink = plus_double(e.time(), 0.1);
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
    if below_32(pev) {
        pev.angles[0] = 0.0;
        pev.angles[2] = 0.0;
        pev.velocity = [0.0; 3];
    }
}

/// `CTSKnife::BounceTouch` 0x7decc, world hit: a blade at 100 u/s or more glances off when it
/// meets the surface at a shallow angle (`-normal·dir < 0.4`) and the surface isn't a floor
/// (`normal.z <= 0.98`); otherwise it sticks. The thresholds are doubles (0.4, 0.98) compared
/// on the x87 stack, and so is the dot product of the stored direction ([`knife_dir`]): mp.dll
/// sums it `z, y, x` (0x100526f9..0x10052717), `ts_i386.so` `x, y, z` (0x7e5c5..0x7e5e8). A NaN
/// dot glances (both branch on C0). The speed is [`knife_slower`]'s.
fn glances_off(arith: Arith, velocity: Vec3, normal: Vec3) -> bool {
    fn go<F: Fpu>(v: Vec3, n: Vec3) -> bool {
        let d = F::vec_normalize(v, ORDER_XYZ);
        let x = F::x;
        let dot = if F::MSVC {
            x(d[2]) * x(n[2]) + x(d[1]) * x(n[1]) + x(d[0]) * x(n[0])
        } else {
            x(n[0]) * x(d[0]) + x(n[1]) * x(d[1]) + x(n[2]) * x(d[2])
        };
        below87(-dot, F::xd(0.4)) && x(n[2]) <= F::xd(0.98)
    }
    !knife_slower(arith, velocity, 100.0) && shared::by_arith!(arith, go(velocity, normal))
}

/// `CTSKnife::BounceTouch`'s speed tests, `|velocity| < k` (700 for the flight, 100 for a world
/// hit): mp.dll compares the unrounded root (0x100520f6, 0x1005246c), `ts_i386.so` the stored
/// `float` (0x7df4b, 0x7e2d2); a NaN speed is below in both.
fn knife_slower(arith: Arith, velocity: Vec3, k: f32) -> bool {
    fn go<F: Fpu>(v: Vec3, k: f32) -> bool {
        below87(F::vec_length(v, ORDER_XYZ), F::x(k))
    }
    shared::by_arith!(arith, go(velocity, k))
}

/// `BounceTouch`'s blade direction, the inline `Normalize` of the velocity (`x, y, z`; mp.dll
/// 0x10052485..0x100524e4, 0x10052688..0x100526e7; `ts_i386.so` 0x7e0f9..0x7e144).
fn knife_dir(arith: Arith, velocity: Vec3) -> Vec3 {
    fn go<F: Fpu>(v: Vec3) -> Vec3 {
        F::vec_normalize(v, ORDER_XYZ)
    }
    shared::by_arith!(arith, go(velocity))
}

/// The pitch of a blade's world hit: `100 + RANDOM_FLOAT(..)`, truncated (`fistp` with the
/// rounding control set to chop).
fn hit_pitch(random: f32) -> i32 {
    (100.0 + random as f64) as i32
}

impl Entity for Knife {
    /// `CBaseEntity::Save` / `Restore`: `CTSKnife`'s own state isn't saved; both pointers set.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut Fields) {
        crate::entities::ts::save_base(cx, f, &mut self.funcs);
    }

    /// `CTSKnife`'s `Precache` slot is `CTSGrenade::Precache`.
    fn restored(&mut self, _cx: &mut Cx) {
        grenade_precache();
    }

    fn spawn(&mut self, cx: &mut Cx) {
        let e = engine();
        let edict = cx.edict;
        let origin = cx.pev().origin;
        let model = match self.btype {
            8 => c"models/w_katana.mdl",
            6 => c"models/w_knife.mdl",
            _ => c"models/w_sealknife.mdl",
        };
        let gravity = crate::gameplay::active().server.thrown_blade(self.btype).gravity;
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
        self.funcs = BaseFuncs { think: true, touch: true, ..BaseFuncs::default() };
    }

    fn think(&mut self, cx: &mut Cx) {
        if !self.funcs.think {
            return;
        }
        let e = engine();
        cx.pev().nextthink = plus_double(e.time(), 0.1);
        if self.expire != 0.0 && e.time() > self.expire {
            entities::remove(cx.edict);
            return;
        }
        // `CTSKnife::WaitThink` 0x7ea30 is the think in every state (flying, lying, stuck), so the
        // flying impact runs for a blade that bounced off a player or slides along the floor
        // too. It has no ground friction: that is `WaitTouch`, which the engine runs on every
        // frame a sliding blade is pushed into the floor (see `ground_slide`).
        // Lent back for the hit, so the kill it scores can name the blade.
        let (me, thrower, bits) = (cx.edict, self.thrower.or(cx.edict), self.bits());
        world::lend(me, self, || impact_damage(me, thrower, bits, false));
        // A blade stuck in a wall (MOVETYPE_FLY) stays put.
        let pev = cx.pev();
        if pev.movetype != MOVETYPE_FLY as i32 {
            self.phys.update(pev);
        }
    }

    fn touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        if !self.funcs.touch {
            return;
        }
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
        let arith = crate::rules::cvars::pm_arith();
        cx.pev().nextthink = plus_double(e.time(), 0.1);
        if Some(other) == self.owner || other.is_null() {
            return;
        }
        let ov = other.v();
        if ov.effects & EF_NODRAW as i32 != 0 {
            return;
        }
        if knife_slower(arith, cx.pev().velocity, 700.0) {
            self.flying = false;
        }
        let dmg = self.damage(cx.pev().origin);
        report_damage(|| format!("(debug){dmg:.6}\n"));
        self.needs_kungfu = false;
        if ov.takedamage != 0.0 && crate::rules::has_player(other) {
            let gt = e.globals();
            let pev = cx.pev();
            let attacker = self.owner.unwrap_or(me);
            let mut info = DamageInfo::new(me, attacker, dmg, self.bits()).traced();
            info.hitgroup = gt.trace_hitgroup;
            info.dir = knife_dir(arith, pev.velocity);
            info.pos = gt.trace_endpos;
            // Lent back for the hit, so the kill it scores can name the blade.
            world::lend(me, self, || damage::apply(other, &info));
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
            let slow = knife_slower(arith, pev.velocity, 100.0);
            let dir = knife_dir(arith, pev.velocity);
            if glances_off(arith, pev.velocity, normal) {
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
            if slow {
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
    fn throwers_match_the_edict_serial() {
        let mut t = Throwers::default();
        assert!(t.get(40, 1).is_none());
        t.note(40, 3, Thrower::NONE);
        assert!(t.get(40, 3).is_some(), "a projectile whose thrower is gone still is one");
        assert!(t.get(40, 4).is_none(), "the slot's next entity");
        assert!(t.get(39, 3).is_none());
        t.note(2, 1, Thrower::NONE);
        assert!(t.get(40, 3).is_some(), "growing keeps the others");
    }

    #[test]
    fn knife_descriptions_follow_the_blade_type() {
        assert_eq!(knife_description(6), "Throwing Combat Knife");
        assert_eq!(knife_description(7), "Throwing Seal Knife");
        assert_eq!(knife_description(8), "Throwing Katana");
        assert_eq!(knife_description(5), "");
    }

    #[test]
    fn flying_impact_ignores_takedamage() {
        // TS compares health alone (issue #26): an unbreakable target still gets the TraceAttack.
        let target = |takedamage: f32, health: f32| entvars_t { takedamage, health, ..Default::default() };
        assert!(impact_hits(&target(0.0, 0.0), Ext::from_f32(190.0)));
        assert!(impact_hits(&target(0.0, 190.0), Ext::from_f32(190.0)));
        assert!(impact_hits(&target(1.0, 100.0), Ext::from_f32(190.0)));
        assert!(!impact_hits(&target(1.0, 190.5), Ext::from_f32(190.0)));
        assert!(!impact_hits(&target(0.0, 200.0), Ext::from_f32(190.0)));
        assert!(!impact_hits(&target(0.0, f32::NAN), Ext::from_f32(190.0)));
    }

    /// `BounceTouch`'s speeds are stored roots of the build's sums and are tested against the
    /// exact `slow · 18` / `slow · 40`: at slow 0.55 the `float` product 9.9000006 equals this
    /// speed, but TS's 9.9000002 is below it. The old `float` length here was 0x43989094.
    #[test]
    fn bounce_speeds_and_thresholds() {
        let slow = f32::from_bits(0x3f0c_cccd);
        let speed = f32::from_bits(0x411e_6667);
        assert_eq!(slow * 18.0, speed, "the float product");
        assert!(bounce_drops_loudly(speed, slow));
        assert!(!bounce_drops_loudly(9.9, 0.55));
        let v = [0xc214_ce87, 0xc392_6221, 0xc29b_04ec].map(f32::from_bits);
        assert_eq!(bounce_speeds(Arith::Msvc, v).1.to_bits(), 0x4398_9093);
        assert_eq!(bounce_speeds(Arith::Gcc, [3.0, 4.0, 12.0]), (5.0, 13.0));
    }

    /// The grenade kick and the ground stop in each build: the old all-`float` arithmetic gave
    /// `[.., 0xbcd92851, 0x3f7be667]` for this kick, and 32.0 (not slower) for this velocity, whose
    /// root mp.dll tests unrounded (31.9999992) and the .so stores (32).
    #[test]
    fn grenade_kick_and_ground_stop_per_build() {
        use shared::pm::{Gcc, Msvc};
        let f = |b: [u32; 3]| b.map(f32::from_bits);
        let fwd = f([0x3e9c_0bb5, 0xbd3b_bbe1, 0x3f73_89bb]);
        assert_eq!(kick_dir(Arith::Msvc, fwd).map(f32::to_bits), [0x3e34_80b9, 0xbcd9_2852, 0x3f7b_e668]);
        assert_eq!(kick_dir(Arith::Gcc, [0.0, 0.0, -0.75]), [0.0, 0.0, 1.0], "a zero length is straight up");
        let v = f([0x41b7_8f1b, 0x419c_758f, 0x412b_9c79]);
        assert!(below_32_in::<Msvc>(v));
        assert!(!below_32_in::<Gcc>(v));
    }

    #[test]
    fn katana_is_thrown_slower_than_knives() {
        assert_eq!(throw_speed(6), 1900.0);
        assert_eq!(throw_speed(7), 1900.0);
        assert_eq!(throw_speed(8), 1000.0);
    }

    #[test]
    fn grenade_explodes_only_in_the_world_box() {
        let at = |origin: Vec3, velocity: Vec3| is_in_world(&entvars_t { origin, velocity, ..Default::default() });
        assert!(at([4095.9, -4095.9, 0.0], [1999.9, -1999.9, 0.0]));
        assert!(!at([4096.0, 0.0, 0.0], [0.0; 3]));
        assert!(!at([0.0, 0.0, -4096.0], [0.0; 3]));
        assert!(!at([0.0; 3], [0.0, 2000.0, 0.0]));
        assert!(!at([0.0; 3], [0.0, 0.0, -2000.0]));
        // NaNs pass every test but the last one, as in TS
        assert!(at([f32::NAN, 0.0, 0.0], [f32::NAN, 0.0, 0.0]));
        assert!(!at([0.0; 3], [0.0, 0.0, f32::NAN]));
    }

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
        for arith in [Arith::Gcc, Arith::Msvc] {
            let at = |speed: f32, d: Vec3| spread::scale(spread::normalize(d), speed);
            let wall = [-1.0, 0.0, 0.0];
            let shallow = [1.0, 3.0, 0.0]; // -n·d ≈ 0.32
            let steep = [1.0, 1.0, 0.0]; // -n·d ≈ 0.71
            assert!(glances_off(arith, at(100.5, shallow), wall));
            assert!(!glances_off(arith, at(99.9, shallow), wall));
            assert!(!glances_off(arith, at(1900.0, steep), wall));
            // floors (normal.z > 0.98) always stick, however shallow
            let floor = [0.0, 0.0, 1.0];
            assert!(!glances_off(arith, at(1900.0, [10.0, 0.0, -1.0]), floor));
            // 0.98 is a double: a float normal.z of 0.98f (0.98000001907) is above it
            let ramp = [(1.0f32 - 0.98 * 0.98).sqrt(), 0.0, 0.98];
            assert!(!glances_off(arith, [0.0, 1900.0, 0.0], ramp));
            let ramp = [0.2, 0.0, 0.979];
            assert!(glances_off(arith, [0.0, 1900.0, 0.0], ramp));
        }
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

#[cfg(test)]
mod precision_tests {
    use super::*;
    use shared::pm::Arith;

    fn f(b: [u32; 3]) -> [f32; 3] {
        b.map(f32::from_bits)
    }

    fn b(v: [f32; 3]) -> [u32; 3] {
        v.map(f32::to_bits)
    }

    /// The flying impact (issue #150): mp.dll's unrounded speed times the `double` 0.1, tested
    /// unrounded against the health; the .so stores both (the port's old `float` maths gave the
    /// .so's damage here).
    #[test]
    fn flying_impact_per_build() {
        let v = f([0x4427_f24a, 0x431b_b090, 0xc43d_9372]);
        let (dir, dmg, test) = flying_impact(Arith::Msvc, v).unwrap();
        assert_eq!((b(dir), dmg.to_bits()), ([0x3f27_c9ac, 0x3e1b_8ae8, 0xbf3d_6598], 0x42cc_fe61));
        assert!(test > Ext::from_f32(dmg), "a target with exactly the dealt health is still hit");
        let (dir, dmg, test) = flying_impact(Arith::Gcc, v).unwrap();
        assert_eq!((b(dir), dmg.to_bits()), ([0x3f27_c9ab, 0x3e1b_8ae8, 0xbf3d_6598], 0x42cc_fe62));
        assert_eq!(test, Ext::from_f32(dmg));
        for arith in [Arith::Gcc, Arith::Msvc] {
            assert!(flying_impact(arith, [64.0, 0.0, 0.0]).is_none());
            assert!(flying_impact(arith, [f32::NAN, 0.0, 0.0]).is_none());
        }
    }

    /// The fuse step rounds once (issue #150); `elapsed + 0.1 * slow` in `float`s gave 0x3fa64a5a.
    #[test]
    fn fuse_step_rounds_once() {
        let (e, s) = (f32::from_bits(0x3f9d_1368), f32::from_bits(0x3f38_4af2));
        for arith in [Arith::Gcc, Arith::Msvc] {
            assert_eq!(fuse_step(arith, e, s).to_bits(), 0x3fa6_4a5b);
        }
    }

    /// The falling objects' landing test and trace end keep the x product unrounded.
    #[test]
    fn fall_helpers() {
        for arith in [Arith::Gcc, Arith::Msvc] {
            assert!(fall_landed(arith, [0.0, 2.9, 0.0], 1.0));
            assert!(!fall_landed(arith, [0.0, 3.0, 0.0], 1.0));
            // 0.1f·v.x + o.x rounds once: 1e8·0.1f = 10000000.149..., which the old `float`
            // product lost before the add.
            let end = fall_trace_end(arith, [0.5, 0.5, 0.5], [1e8, 1e8, 1e8]);
            assert_eq!(end.map(f32::to_bits), [0x4b18_9681, 0x4b18_9680, 0x4b18_9680]);
        }
    }
}
