//! Dropped weapons (`CTSWorldGun`, link `TSWorldGun`, runtime classname `WorldGun`) and dropped
//! ammo (`CTSAmmoPack`, link `TSAmmoPack`, classname `AmmoPack`).

use super::host::emit;
use super::{assets, inventory};
use crate::damage::{self, DamageInfo};
use crate::entities::{self, link_entities};
use crate::player;
use crate::world::{Cx, Entity};
use goldsrc::ext::weapons::WeaponsEngine;
use goldsrc::server::{DONT_IGNORE_MONSTERS, engine};
use goldsrc::sys::*;
use shared::weapons::spread::{self, Vec3};
use shared::weapons::{MAX_AMMO, Status, id, info};

link_entities! {
    TSWorldGun => WorldGun,
    TSAmmoPack => AmmoPack,
}

#[derive(Default)]
pub struct WorldGun {
    pub id: u8,
    pub status: Status,
    /// Original owner (`s_weapon_status+0x30`).
    pub original_owner: Option<*mut edict_t>,
    /// The player who dropped it (glow shell for them, `+0xd0`).
    pub dropper: Option<*mut edict_t>,
    expire: f32,
    first_bounce: bool,
    pickup_after: f32,
    taken: bool,
    on_ground: bool,
}

/// `CTSWorldGun::CreateWeapon(player, id, status)` + placement; returns the edict.
pub fn create(owner: *mut edict_t, id: u8, status: Status, origin: Vec3, velocity: Vec3, original_owner: *mut edict_t) -> *mut edict_t {
    let slow = if owner.is_null() { 1.0 } else { unsafe { (*owner).v.fuser1 } };
    super::projectiles::spawn_entity::<WorldGun>(c"TSWorldGun", |w, cx| {
        w.id = id;
        w.status = status;
        w.original_owner = (!original_owner.is_null()).then_some(original_owner);
        let e = engine();
        let pev = cx.pev();
        pev.origin = origin;
        pev.velocity = velocity;
        pev.owner = owner;
        pev.fuser1 = if slow > 0.0 { slow } else { 1.0 };
        pev.angles = [0.0; 3];
        pev.avelocity = [0.0, e.random_long(-300, 300) as f32, e.random_long(-300, 300) as f32];
        if id == shared::weapons::id::BRIEFCASE {
            pev.renderamt = 5.0;
            pev.renderfx = kRenderFxGlowShell as i32;
            pev.rendercolor = [200.0, 200.0, 200.0];
        }
    })
}

/// Lifetime override for weapons dropped with `drop`/disarm: `clamp(2 * weaponstay, 10, 120)`,
/// and the dropper sees it glow.
pub fn set_drop_lifetime(gun: *mut edict_t, dropper: *mut edict_t) {
    let t = (assets::cvars().weaponstay * 2.0).clamp(10.0, 120.0);
    let now = engine().time();
    crate::world::with::<WorldGun, _>(gun, |w| {
        w.expire = now + t;
        w.dropper = Some(dropper);
    });
}

/// The dropper sees its own dropped gun glow (see `weapons::glow_for_host`).
pub fn glows_for(ent: *mut edict_t, host: *mut edict_t) -> bool {
    crate::world::with::<WorldGun, _>(ent, |w| w.dropper == Some(host)).unwrap_or(false)
}

impl Entity for WorldGun {
    fn spawn(&mut self, cx: &mut Cx) {
        let e = engine();
        let edict = cx.edict;
        let origin = cx.pev().origin;
        let model = assets::with(|a| a.w_model[self.id as usize]).flatten().unwrap_or(c"models/w_glock18.mdl");
        e.set_model(edict, model);
        let pev = cx.pev();
        pev.classname = e.alloc_string(c"WorldGun");
        pev.movetype = MOVETYPE_BOUNCE as i32;
        pev.solid = SOLID_BBOX as i32;
        pev.gravity = 0.7;
        pev.friction = 0.8;
        e.set_origin(edict, origin);
        e.set_size(edict, [0.0; 3], [0.0; 3]);
        self.expire = e.time()
            + if self.id == id::BRIEFCASE { 30.0 } else { assets::cvars().weaponstay.clamp(5.0, 60.0) };
        let pev = cx.pev();
        pev.nextthink = e.time() + 0.1;
        pev.iuser4 = 2;
        self.first_bounce = true;
        self.pickup_after = e.time() + 0.5;
        self.taken = false;
    }

    /// `Fall` until it lands, then `WaitThink`.
    fn think(&mut self, cx: &mut Cx) {
        let e = engine();
        let me = cx.edict;
        let now = e.time();
        cx.pev().nextthink = now + 0.1;
        if now > self.expire {
            entities::remove(me);
            return;
        }
        if self.on_ground {
            return;
        }
        let pev = cx.pev();
        let slow = if pev.fuser1 > 0.0 { pev.fuser1 } else { 1.0 };
        if pev.flags & FL_ONGROUND as i32 != 0 && spread::length(pev.velocity) < slow * 3.0 {
            emit(me, CHAN_STATIC, c"items/weapondrop1.wav", 1.0, 0.8, 100);
            let pev = cx.pev();
            pev.angles[0] = 0.0;
            pev.angles[2] = 0.0;
            self.land(cx);
            return;
        }
        // falling guns hurt what they hit (breakables)
        let pev = cx.pev();
        let end = spread::ma(pev.origin, 0.1, pev.velocity);
        let tr = e.trace_line(pev.origin, end, DONT_IGNORE_MONSTERS, me);
        if tr.flFraction < 1.0 && !tr.pHit.is_null() && !e.is_player_edict(tr.pHit) {
            let slots = info(self.id).map(|w| w.slots).unwrap_or(10) as f32;
            let s = spread::length(pev.velocity) * slots;
            let target = unsafe { &(*tr.pHit).v };
            if target.takedamage != 0.0 && target.health <= s {
                let mut d = DamageInfo::new(me, me, s * 0.001, 0x1080);
                d.dir = pev.velocity;
                d.pos = tr.vecEndPos;
                damage::apply(tr.pHit, &d);
            }
        }
    }

    fn touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        if self.on_ground {
            // WaitTouch: +use (the briefcase on plain touch)
            let use_held = unsafe { (*other).v.button } & IN_USE as i32 != 0;
            if self.id == id::BRIEFCASE || use_held {
                self.get_weapon(cx, other);
            }
            return;
        }
        // BounceTouch
        let e = engine();
        let me = cx.edict;
        let pev = cx.pev();
        pev.velocity[2] *= 0.3;
        let slow = if pev.fuser1 > 0.0 { pev.fuser1 } else { 1.0 };
        if pev.flags & FL_ONGROUND as i32 == 0 {
            if spread::length(pev.velocity) > slow * 18.0 && self.first_bounce && e.globals().trace_plane_normal[2] > 0.98 {
                self.first_bounce = false;
                emit(me, CHAN_STATIC, c"items/weapondrop1.wav", 1.0, 0.8, 100 + e.random_long(0, 10));
            }
        } else {
            let h = (pev.velocity[0] * pev.velocity[0] + pev.velocity[1] * pev.velocity[1]).sqrt();
            if h <= slow * 40.0 {
                pev.velocity = spread::scale(pev.velocity, 0.7);
                pev.avelocity = spread::scale(pev.avelocity, 0.7);
            }
            pev.angles[2] = 0.0;
        }
        // Players can also grab it while it's still sliding.
        if e.is_player_edict(other) && unsafe { (*other).v.button } & IN_USE as i32 != 0 {
            self.get_weapon(cx, other);
        }
    }

    fn should_collide(&mut self, _cx: &mut Cx, other: *mut edict_t) -> bool {
        if other.is_null() {
            return true;
        }
        let c = engine().string(unsafe { (*other).v.classname }).to_bytes();
        c != b"WorldGun" && c != b"knife"
    }
}

impl WorldGun {
    /// `OnGround`: becomes a trigger pickup.
    fn land(&mut self, cx: &mut Cx) {
        let e = engine();
        let me = cx.edict;
        self.on_ground = true;
        let pev = cx.pev();
        pev.owner = std::ptr::null_mut();
        pev.solid = SOLID_TRIGGER as i32;
        let origin = pev.origin;
        e.set_origin(me, origin);
        e.set_size(me, [-4.0, -4.0, 0.0], [4.0, 4.0, 4.0]);
    }

    /// `CTSWorldGun::GetWeapon` 0xa1198.
    fn get_weapon(&mut self, cx: &mut Cx, other: *mut edict_t) {
        let e = engine();
        if self.taken || e.time() < self.pickup_after || !e.is_player_edict(other) {
            return;
        }
        let ov = unsafe { &(*other).v };
        if ov.deadflag != DEAD_NO as i32 || ov.health <= 0.0 || ov.effects & EF_NODRAW as i32 != 0 {
            return;
        }
        if cx.pev().effects & EF_NODRAW as i32 != 0 {
            return;
        }
        let (id, status, orig) = (self.id, self.status, self.original_owner);
        let took = player::with_player(other, |p, pcx| {
            let ok = inventory::pick_up_weapon(p, pcx, id, Some(&status), 0, id != shared::weapons::id::BRIEFCASE, false);
            if ok {
                if let Some(o) = orig {
                    p.inv.sv.owners[id as usize] = o;
                }
            }
            inventory::sync_speed(p, pcx);
            ok
        })
        .unwrap_or(false);
        if took {
            self.taken = true;
            entities::remove(cx.edict);
        }
    }
}

// ---- ammo pack --------------------------------------------------------------------------------

#[derive(Default)]
pub struct AmmoPack {
    pub reserve: [i32; MAX_AMMO],
    expire: f32,
    on_ground: bool,
}

/// `CBasePlayer::DropAmmo` 0x946f0 (on death): all reserve ammo in one pack.
pub fn drop_ammo(owner: *mut edict_t, reserve: [i32; MAX_AMMO]) {
    if reserve.iter().all(|&r| r <= 0) {
        return;
    }
    let e = engine();
    let v = unsafe { &(*owner).v };
    let ang = [e.random_long(-100, 100) as f32, e.random_long(-100, 100) as f32, 0.0];
    let (f, r, _) = spread::angle_vectors(ang);
    let origin = spread::add(v.origin, spread::add(spread::scale(f, 8.0), spread::scale(r, 8.0)));
    super::projectiles::spawn_entity::<AmmoPack>(c"TSAmmoPack", |a, cx| {
        a.reserve = reserve;
        let pev = cx.pev();
        pev.origin = origin;
        pev.angles = [0.0; 3];
        pev.avelocity = [0.0, e.random_long(-300, 300) as f32, e.random_long(-300, 300) as f32];
    });
}

impl Entity for AmmoPack {
    fn spawn(&mut self, cx: &mut Cx) {
        let e = engine();
        let edict = cx.edict;
        e.set_model(edict, c"models/ammopack.mdl");
        let pev = cx.pev();
        pev.classname = e.alloc_string(c"AmmoPack");
        pev.movetype = MOVETYPE_TOSS as i32;
        pev.solid = SOLID_BBOX as i32;
        let origin = pev.origin;
        e.set_origin(edict, origin);
        e.set_size(edict, [0.0; 3], [0.0; 3]);
        self.expire = e.time() + assets::cvars().weaponstay.clamp(5.0, 60.0);
        let pev = cx.pev();
        pev.nextthink = e.time() + 0.1;
        pev.iuser4 = 0;
        pev.armorvalue = 0.0;
        pev.owner = std::ptr::null_mut();
    }

    fn think(&mut self, cx: &mut Cx) {
        let e = engine();
        let me = cx.edict;
        cx.pev().nextthink = e.time() + 0.1;
        if !self.on_ground && cx.pev().flags & FL_ONGROUND as i32 != 0 {
            self.on_ground = true;
            let pev = cx.pev();
            pev.angles[0] = 0.0;
            pev.angles[2] = 0.0;
            pev.solid = SOLID_TRIGGER as i32;
            let o = pev.origin;
            e.set_origin(me, o);
            e.set_size(me, [-16.0, -16.0, 0.0], [16.0, 16.0, 16.0]);
        }
        if self.expire < e.time() {
            entities::remove(me);
        }
    }

    /// `CTSAmmoPack::WaitTouch`: an alive player takes what fits.
    fn touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        let e = engine();
        if !self.on_ground || !e.is_player_edict(other) {
            return;
        }
        let ov = unsafe { &(*other).v };
        if ov.deadflag != DEAD_NO as i32 || ov.health <= 0.0 {
            return;
        }
        let reserve = self.reserve;
        let took = player::with_player(other, |p, _| {
            let mut any = false;
            for (t, &n) in reserve.iter().enumerate() {
                if n > 0 && p.inv.gun.add_ammo(t as u8, n, assets::max_carry(t as u8)) {
                    any = true;
                }
            }
            any
        })
        .unwrap_or(false);
        if took {
            emit(other, CHAN_ITEM, c"items/9mmclip1.wav", 1.0, 0.8, 100);
            entities::remove(cx.edict);
        }
    }
}
