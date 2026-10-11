//! Dropped weapons (`CTSWorldGun`, link `TSWorldGun`, runtime classname `WorldGun`) and dropped
//! ammo (`CTSAmmoPack`, link `TSAmmoPack`, classname `AmmoPack`).

use super::host::emit;
use super::projectiles::{self, SlowPhysics, bounce_ground_damping, give_slow_mul};
use super::{assets, inventory};
use crate::damage::{self, DamageInfo};
use crate::entities::{self, link_entities};
use crate::player::{self, Player};
use crate::world::{Cx, Entity};
use goldsrc::edict::EdictPtr;
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
    /// Who threw it, credited for its flying impact (port deviation, issue #1).
    thrower: projectiles::Thrower,
    expire: f32,
    pickup_after: f32,
    taken: bool,
    on_ground: bool,
    phys: SlowPhysics,
}

/// `CTSWorldGun::CreateWeapon(player, id, status)` + placement; returns the edict. `time_scale`
/// is the thrower's ([`projectiles::thrower_time_scale`]); without a thrower it's 1.
#[allow(clippy::too_many_arguments)]
pub fn create(
    owner: *mut edict_t,
    id: u8,
    status: Status,
    origin: Vec3,
    velocity: Vec3,
    original_owner: *mut edict_t,
    time_scale: f32,
) -> *mut edict_t {
    let slow = if owner.is_null() { 1.0 } else { time_scale };
    projectiles::spawn_entity::<WorldGun>(c"TSWorldGun", |w, cx| {
        w.id = id;
        w.status = status;
        w.original_owner = (!original_owner.is_null()).then_some(original_owner);
        w.thrower = projectiles::Thrower::new(owner);
        let e = engine();
        let pev = cx.pev();
        pev.origin = origin;
        pev.velocity = velocity;
        pev.owner = owner;
        pev.fuser1 = slow;
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

/// The `WorldGun` branch of `TSExplosion` (0x7d678..0x7d93d): the gun is put back into flight
/// (`UTIL_SetOrigin`, zero size, touch `CTSWorldGun::BounceTouch`, think `CTSWorldGun::Fall` in
/// 0.1 s, `MOVETYPE_BOUNCE`, `SOLID_BBOX`, gravity 0.7, friction 0.8), thrown by
/// [`blast_velocity`], hopped off the ground if it rests there ([`ground_hop`]), and spun at
/// random. `from` is the blast point + 50z.
pub fn blast(gun: *mut edict_t, from: Vec3, r_max: f32) {
    // The touch/think switch: the port's `WorldGun` runs `Fall`/`BounceTouch` while it isn't on
    // the ground.
    let Some(id) = crate::world::with::<WorldGun, _>(gun, |w| {
        w.on_ground = false;
        w.id
    }) else {
        return;
    };
    let e = engine();
    let origin = gun.v().origin;
    e.set_origin(gun, origin);
    e.set_size(gun, [0.0; 3], [0.0; 3]);
    let now = e.time();
    // 0x7d85c: `get_weapon_info(this+0x88)`, byte `+0xd0` (the inventory weight).
    let slots = info(id).map_or(0, |w| w.slots) as u8;
    let pev = gun.v();
    pev.nextthink = now + 0.1;
    pev.movetype = MOVETYPE_BOUNCE as i32;
    pev.solid = SOLID_BBOX as i32;
    pev.gravity = 0.7;
    pev.friction = 0.8;
    pev.velocity = blast_velocity(pev.origin, from, r_max, slots);
    if pev.flags & FL_ONGROUND as i32 != 0 {
        pev.flags &= !(FL_ONGROUND as i32);
        pev.velocity[2] = ground_hop(pev.velocity[2]);
    }
    pev.avelocity[1] = e.random_long(-300, 300) as f32;
    pev.avelocity[2] = e.random_long(-300, 300) as f32;
}

/// `TSExplosion`'s knock-back for a `WorldGun` at `gun` (0x7d743..0x7d8b6):
/// `normalize(gun − from) · (r_max − |gun − from|) / slots · 10`, so heavier guns fly less. Past
/// `r_max` the speed goes negative (towards the blast). A weightless id gives an infinite
/// velocity, as TS's x87 division does; the engine clamps it to `sv_maxvelocity`.
pub fn blast_velocity(gun: Vec3, from: Vec3, r_max: f32, slots: u8) -> Vec3 {
    let away = spread::sub(gun, from);
    let dist = spread::length(away);
    let dir = if dist == 0.0 { [0.0, 0.0, 1.0] } else { spread::scale(away, 1.0 / dist) };
    let push = spread::scale(dir, r_max - dist);
    let w = slots as f32;
    [push[0] / w * 10.0, push[1] / w * 10.0, push[2] / w * 10.0]
}

/// 0x7d8cf: a gun resting on the ground (`FL_ONGROUND`) hops up: `|v.z| + 50`.
pub fn ground_hop(vz: f32) -> f32 {
    vz.abs() + 50.0
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
        self.expire = e.time() + if self.id == id::BRIEFCASE { 30.0 } else { assets::cvars().weaponstay.clamp(5.0, 60.0) };
        let pev = cx.pev();
        pev.nextthink = e.time() + 0.1;
        pev.iuser4 = 2;
        self.pickup_after = e.time() + 0.5;
        self.taken = false;
    }

    /// `Fall` (0xa0d84) until it lands, then `WaitThink`. TS adds no friction here: a sliding
    /// gun is slowed only by `BounceTouch`'s ground damping, which the engine runs on every frame
    /// it scrapes the floor.
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
        if pev.flags & FL_ONGROUND as i32 != 0 && spread::length(pev.velocity) < give_slow_mul(pev.fuser1) * 3.0 {
            emit(me, CHAN_STATIC, c"items/weapondrop1.wav", 1.0, 0.8, 100);
            let pev = cx.pev();
            pev.angles[0] = 0.0;
            pev.angles[2] = 0.0;
            self.land(cx);
            return;
        }
        // Falling guns hurt what they hit (breakables). The attacker is the thrower (port
        // deviation, issue #1; TS `CTSWorldGun::Fall` 0xa0d84 passes the gun).
        let pev = cx.pev();
        let end = spread::ma(pev.origin, 0.1, pev.velocity);
        let tr = e.trace_line(pev.origin, end, DONT_IGNORE_MONSTERS, me);
        if tr.flFraction < 1.0 && !tr.pHit.is_null() && !e.is_player_edict(tr.pHit) {
            let slots = info(self.id).map(|w| w.slots).unwrap_or(10) as f32;
            let s = spread::length(pev.velocity) * slots;
            let target = tr.pHit.v();
            if target.takedamage != 0.0 && target.health <= s {
                let mut d = DamageInfo::new(me, self.thrower.or(me), s * 0.001, 0x1080).traced();
                d.dir = pev.velocity;
                d.pos = tr.vecEndPos;
                damage::apply(tr.pHit, &d);
            }
        }
        self.phys.update(cx.pev());
    }

    fn touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        if self.on_ground {
            // WaitTouch 0xa1250: +use (the briefcase on plain touch)
            let use_held = other.v().button & IN_USE as i32 != 0;
            if self.id == id::BRIEFCASE || use_held {
                self.get_weapon(cx, other);
            }
            return;
        }
        // BounceTouch (0xa0b0c): keep 0.3 of the vertical speed. In the air, a fast hit on a
        // floor plays the drop sound. TS gates that on a byte (+0xc8) that Spawn sets and the
        // touch sets again (never clears), so it plays on every such bounce, not just the first.
        // On the ground, damp velocity and spin and level the roll.
        let e = engine();
        let me = cx.edict;
        let pev = cx.pev();
        pev.velocity[2] = (pev.velocity[2] as f64 * 0.3) as f32;
        let slow = give_slow_mul(pev.fuser1);
        if pev.flags & FL_ONGROUND as i32 == 0 {
            if spread::length(pev.velocity) > slow * 18.0 && e.globals().trace_plane_normal[2] as f64 > 0.98 {
                emit(me, CHAN_STATIC, c"items/weapondrop1.wav", 1.0, 0.8, 100 + e.random_long(0, 10));
            }
        } else {
            let k = bounce_ground_damping(pev.velocity[0].hypot(pev.velocity[1]), slow);
            pev.velocity = spread::scale(pev.velocity, k);
            pev.avelocity = spread::scale(pev.avelocity, k);
            pev.angles[2] = 0.0;
        }
        // A gun still in the air is picked up with +use (`Use`, see `use_by`), not by touch.
    }

    fn should_collide(&mut self, _cx: &mut Cx, other: *mut edict_t) -> bool {
        if other.is_null() {
            return true;
        }
        let c = engine().string(other.v().classname).to_bytes();
        c != b"WorldGun" && c != b"knife"
    }
}

impl WorldGun {
    /// `OnGround`: becomes a trigger pickup.
    fn land(&mut self, cx: &mut Cx) {
        let e = engine();
        let me = cx.edict;
        self.on_ground = true;
        assets::debug(|| format!("WorldGun {} landed at {:?}", self.id, cx.pev().origin));
        let pev = cx.pev();
        pev.owner = std::ptr::null_mut();
        pev.solid = SOLID_TRIGGER as i32;
        let origin = pev.origin;
        e.set_origin(me, origin);
        e.set_size(me, [-4.0, -4.0, 0.0], [4.0, 4.0, 4.0]);
    }

    /// `CTSWorldGun::GetWeapon` 0xa1198 for a touching player.
    fn get_weapon(&mut self, cx: &mut Cx, other: *mut edict_t) {
        if !engine().is_player_edict(other) {
            return;
        }
        player::with_player(other, |p, pcx| self.give_to(cx, p, pcx));
    }

    /// `CTSWorldGun::Use` 0xa1158, from the player's +use (`player::player_use`, the player is
    /// the one being dispatched): works while the gun is still flying, once 0.5 s have passed.
    pub fn use_by(&mut self, cx: &mut Cx, p: &mut Player, pcx: &mut Cx) {
        if self.pickup_after < engine().time() {
            self.give_to(cx, p, pcx);
        }
    }

    /// `CTSWorldGun::GetWeapon` 0xa1198.
    fn give_to(&mut self, cx: &mut Cx, p: &mut Player, pcx: &mut Cx) {
        if self.taken || !Player::is_alive(pcx) || pcx.pev().effects & EF_NODRAW as i32 != 0 {
            return;
        }
        if cx.pev().effects & EF_NODRAW as i32 != 0 {
            return;
        }
        let id = self.id;
        let ok = inventory::pick_up_weapon(p, pcx, id, Some(&self.status), 0, id != shared::weapons::id::BRIEFCASE, false);
        if ok && let Some(o) = self.original_owner {
            p.inv.sv.owners[id as usize] = o;
        }
        inventory::sync_speed(p, pcx);
        if ok {
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
    let v = owner.v();
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
        let ov = other.v();
        if ov.deadflag != DEAD_NO as i32 || ov.health <= 0.0 {
            return;
        }
        let reserve = &mut self.reserve;
        // Without the gun the whole touch is skipped: nothing taken, the pack stays.
        let took = player::with_player(other, |p, _| {
            p.inv.sv.has_gun.then(|| take_ammo(reserve, |t, n| p.inv.gun.add_ammo(t, n, assets::max_carry(t))))
        });
        let Some(Some(took)) = took else {
            return;
        };
        if took {
            emit(other, CHAN_ITEM, c"items/9mmclip1.wav", 1.0, 0.8, 100);
        }
        // TS removes the pack on the first touch whether or not anything fitted (the armour a
        // `DropArmor` pack carries is the only thing that keeps it; not ported).
        entities::remove(cx.edict);
    }
}

/// The ammo half of `CTSAmmoPack::WaitTouch` 0xa16c0: every type the pack holds goes through
/// `CTSGun::AddAmmo` 0x808ac, which takes the whole amount (clamped to the max carry) if the
/// player is below the max carry for that type. TS doesn't check for a weapon that uses it.
/// Taken types are emptied from the pack. Returns true if anything was taken.
fn take_ammo(pack: &mut [i32; MAX_AMMO], mut add_ammo: impl FnMut(u8, i32) -> bool) -> bool {
    let mut any = false;
    for (t, n) in pack.iter_mut().enumerate() {
        if *n != 0 && add_ammo(t as u8, *n) {
            *n = 0;
            any = true;
        }
    }
    any
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::weapons::Gun;

    fn close(a: Vec3, b: Vec3) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-3)
    }

    #[test]
    fn blast_velocity_pushes_away_scaled_by_weight() {
        // 100 u straight above the push point, grenade r_max 1024, a 10-slot pistol.
        let v = blast_velocity([0.0, 0.0, 151.0], [0.0, 0.0, 51.0], 1024.0, 10);
        assert!(close(v, [0.0, 0.0, 924.0]), "{v:?}");
        // Sideways, a 40-slot shotgun: a quarter of the speed.
        let v = blast_velocity([300.0, 0.0, 51.0], [0.0, 0.0, 51.0], 1024.0, 40);
        assert!(close(v, [181.0, 0.0, 0.0]), "{v:?}");
        // Diagonal: the direction is normalised, the magnitude is (r_max - dist) / slots * 10.
        let v = blast_velocity([30.0, 40.0, 0.0], [0.0; 3], 1024.0, 20);
        assert!(close(v, [292.2, 389.6, 0.0]), "{v:?}");
    }

    #[test]
    fn blast_velocity_edge_cases() {
        // On the push point: straight up at full strength.
        assert!(close(blast_velocity([5.0; 3], [5.0; 3], 1024.0, 10), [0.0, 0.0, 1024.0]));
        // Beyond r_max (the sphere test is against the gun's box and the blast point, not the
        // push point 50 u higher): pulled back towards the blast.
        assert!(close(blast_velocity([0.0, 0.0, 1100.0], [0.0; 3], 1024.0, 10), [0.0, 0.0, -76.0]));
        // No weight: infinite, as in TS.
        assert!(blast_velocity([0.0, 0.0, 10.0], [0.0; 3], 1024.0, 0)[2].is_infinite());
    }

    #[test]
    fn grounded_gun_hops() {
        assert_eq!(ground_hop(-120.0), 170.0);
        assert_eq!(ground_hop(30.0), 80.0);
        assert_eq!(ground_hop(0.0), 50.0);
    }

    #[test]
    fn ammo_pack_takes_every_type_below_max_carry() {
        let mut gun = Gun::default();
        gun.reserve[1] = 200;
        gun.reserve[3] = 210;
        let mut pack = [0; MAX_AMMO];
        pack[1] = 60;
        pack[3] = 30;
        pack[4] = 5;
        assert!(take_ammo(&mut pack, |t, n| gun.add_ammo(t, n, 210)));
        // below the max carry: everything, clamped
        assert_eq!(gun.reserve[1], 210);
        assert_eq!(pack[1], 0);
        // no weapon for type 4 needed
        assert_eq!(gun.reserve[4], 5);
        // at the max carry: stays in the pack
        assert_eq!(gun.reserve[3], 210);
        assert_eq!(pack[3], 30);
        assert!(!take_ammo(&mut pack, |t, n| gun.add_ammo(t, n, 210)));
    }
}
