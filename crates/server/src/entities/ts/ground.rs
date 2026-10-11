//! `ts_groundweapon` (CTSGroundWeapon 0xa192c), `ts_groundammo` (CTSGroundAmmo 0xa26f4) and the
//! HL weapon/ammo classnames TS re-points at them (architecture.md §5.2, weapons.md §7.4).
//! Weapons are picked up with +use, ammo on touch. Giving goes through `crate::weapons`.

use crate::player::{Player, with_player};
use crate::rules::{self, msg, state};
use crate::save::{BaseFuncs, Fields};
use crate::world::{Cx, Entity, UseType};
use goldsrc::server::{engine, intern};
use goldsrc::sys::*;
use shared::weapons::tables;
use std::ffi::CStr;

const BRIEFCASE: i32 = 29;
const ON_WALL: i32 = 16;

/// HL classname → TS weapon ids it picks from (`Random<Kind>`).
pub fn hl_weapon_ids(class: &CStr) -> Option<&'static [i32]> {
    Some(match class.to_bytes() {
        b"weapon_crossbow" => &[18],
        b"weapon_shotgun" => &[4, 26, 33, 20, 11],
        b"weapon_gauss" | b"weapon_egon" => &[13, 5, 27, 15],
        b"weapon_rpg" => &[23, 7, 6, 19, 17, 3],
        b"weapon_9MMAR" => &[31, 12, 36],
        b"weapon_hornetgun" => &[32],
        b"weapon_satchel" | b"ammo_ARgrenades" => &[25, 35],
        b"weapon_snark" => &[34],
        b"weapon_handgrenade" => &[24],
        _ => return None,
    })
}

/// HL ammo classname → the weapon class whose ammo types it carries.
pub fn hl_ammo_weapons(class: &CStr) -> Option<&'static [i32]> {
    Some(match class.to_bytes() {
        b"ammo_9mmAR" => &[31, 12, 36],
        b"ammo_buckshot" => &[4, 26, 33, 20, 11],
        b"ammo_crossbow" => &[18],
        b"ammo_gaussclip" => &[13, 5, 27, 15],
        b"ammo_rpgclip" => &[23, 7, 6, 19, 17, 3],
        _ => return None,
    })
}

/// Respawn delay of ground items: < 1 → 60 s, 1 → 1 s, else min(v, 180).
pub fn respawn_delay(v: i32) -> f32 {
    if v < 1 {
        60.0
    } else if v < 2 {
        1.0
    } else {
        v.min(180) as f32
    }
}

/// `GetWeapon`'s price of a `buyable` ground weapon: the weapon, the attachments in its
/// spawnflags, and the ammo price per extra magazine.
pub fn buy_price(id: i32, spawnflags: i32, extra_clip: i32) -> i32 {
    let t = tables();
    let Some(w) = u8::try_from(id).ok().and_then(|id| t.weapon(id)) else { return 0 };
    let prices = &t.combat.attachment_price;
    let mut cost = w.cost;
    for (bit, price) in [(1, prices.silencer), (2, prices.laser), (4, prices.flashlight), (8, prices.scope)] {
        if spawnflags & bit != 0 {
            cost += price;
        }
    }
    cost + t.ammo(w.ammo).map_or(0, |a| a.price) * extra_clip.max(0)
}

fn round_mode() -> bool {
    matches!(state::with(|g| g.mode), state::Mode::RoundTdm | state::Mode::Objective)
}

/// Shared falling / landing behaviour.
#[derive(Default)]
struct Ground {
    landed: bool,
    waiting: bool,
    /// TS's member pointers: `m_pfnThink` `Fall` from `Spawn` until `OnGround`, and again after a
    /// pickup unless the item stays hidden; `m_pfnTouch` `WaitTouch` from `OnGround` until a pickup.
    funcs: BaseFuncs,
}

impl Ground {
    /// `Fall` (every 0.5 s): land, snap to a wall if flagged, become pickable.
    fn fall(&mut self, cx: &mut Cx, ammo: bool) {
        cx.think_in(0.5);
        let on_wall = cx.pev().spawnflags & ON_WALL != 0;
        if !on_wall {
            if cx.pev().flags & FL_ONGROUND as i32 == 0 && !self.landed {
                return;
            }
            let v = cx.pev();
            v.angles[0] = 0.0;
            v.angles[2] = 0.0;
        } else if !self.landed {
            put_on_wall(cx);
        }
        // TS's slow-motion `EMIT_SOUND_DYN` (factor 0.5), as in `CTSGroundWeapon::Fall` (0xa2150)
        // and `CTSGroundAmmo::Fall` (0xa2d6c).
        crate::entities::stock::explode::emit_sound_slow(cx.edict, CHAN_STATIC, c"items/weapondrop1.wav", 1.0, 0.8, 0, 100, 0.5);
        // OnGround (0xa226c / 0xa2e88): touch `WaitTouch`, no think (`nextthink` keeps Fall's).
        let e = cx.edict;
        let v = cx.pev();
        if ammo {
            v.movetype = MOVETYPE_FLY as i32;
            v.effects |= 2;
        }
        v.solid = SOLID_TRIGGER as i32;
        v.effects &= !(EF_NODRAW as i32);
        let org = v.origin;
        engine().set_origin(e, org);
        engine().set_size(e, [-4.0; 3], [4.0; 3]);
        self.funcs.touch = true;
        self.funcs.think = false;
        self.landed = true;
        self.waiting = true;
    }

    /// `Spawn`'s pointers (touch none, think `Fall`) and `iuser4` (`= 0; |= 2`).
    fn spawned(&mut self, cx: &mut Cx) {
        self.funcs = BaseFuncs { think: true, ..BaseFuncs::default() };
        cx.pev().iuser4 = 2;
    }

    /// Hide after a pickup and schedule the respawn.
    fn taken(&mut self, cx: &mut Cx, respawn: i32) {
        self.waiting = false;
        self.funcs.touch = false;
        let v = cx.pev();
        v.effects |= EF_NODRAW as i32;
        v.solid = SOLID_NOT as i32;
        // A null think (`nextthink` left as it was) keeps it hidden until `RespawnEntities`.
        self.funcs.think = !(respawn < 1 && round_mode());
        if self.funcs.think {
            v.nextthink = engine().time() + respawn_delay(respawn);
        }
    }
}

/// `PutOnWall`: find the closest surface in 8 horizontal directions and stick to it.
fn put_on_wall(cx: &mut Cx) {
    let eng = engine();
    let e = cx.edict;
    let org = cx.pev().origin;
    let mut best: Option<(f32, [f32; 3], [f32; 3])> = None;
    for k in 0..8 {
        let a = (k as f32 * 45.0).to_radians();
        let dir = [a.cos(), a.sin(), 0.0];
        let end = [org[0] + dir[0] * 1024.0, org[1] + dir[1] * 1024.0, org[2]];
        let tr = eng.trace_line(org, end, goldsrc::server::IGNORE_MONSTERS, e);
        if tr.flFraction < 1.0 {
            let d = rules::dist(org, tr.vecEndPos);
            if best.is_none_or(|b| d < b.0) {
                best = Some((d, tr.vecEndPos, tr.vecPlaneNormal));
            }
        }
    }
    let v = cx.pev();
    match best {
        Some((_, pos, n)) => {
            v.origin = [pos[0] + n[0] * 2.0, pos[1] + n[1] * 2.0, pos[2] + n[2] * 2.0];
            let ang = eng.vec_to_angles(n);
            v.angles = [ang[0] + 90.0, ang[1], ang[2]];
        }
        None => v.angles[2] = 90.0,
    }
}

// ---- ts_groundweapon ---------------------------------------------------------------------------

#[derive(Default)]
pub struct GroundWeapon {
    pub id: i32,
    respawn: i32,
    extra_clip: i32,
    buyable: bool,
    attacker_team: i32,
    ground: Ground,
}

impl GroundWeapon {
    /// `GetWeapon` (0xa2328): in cash mode a `buyable` one checks `attackerteam` (round modes) and
    /// the price first; then give the weapon (attachments from spawnflags) and extra magazines.
    fn give(&mut self, cx: &mut Cx, p: &mut Player, pcx: &mut Cx) -> bool {
        if !self.ground.waiting || !rules::alive(pcx.pev()) || pcx.pev().effects & EF_NODRAW as i32 != 0 {
            return false;
        }
        if cx.pev().effects & EF_NODRAW as i32 != 0 {
            return false;
        }
        let flags = cx.pev().spawnflags;
        // A negative `wextraclip` is clamped to 0 for good.
        self.extra_clip = self.extra_clip.max(0);
        let mut cost = 0;
        if state::cash_based() && self.buyable {
            if round_mode() && self.attacker_team != -1 {
                let team = state::with(|g| g.team_index(&p.ts.team_name));
                if team != self.attacker_team {
                    msg::ts_message_one(pcx.edict, [0, 0, 0], 16, 30, "Your team cannot buy this weapon.");
                    return false;
                }
            }
            cost = buy_price(self.id, flags, self.extra_clip);
            if p.cash < cost {
                msg::ts_message_one(pcx.edict, [0, 0, 0], 16, 30, "You don't have enough money for that weapon.");
                return false;
            }
        }
        let attachments = (flags & 0xf) as u8;
        if !crate::weapons::give_weapon(p, pcx, self.id as u8, attachments) {
            return false;
        }
        rules::lifecycle::give_cash(p, -cost);
        // TS tops up only an empty reserve, checked after the pickup (0xa2328: `reserve[ammo] < 1`),
        // with `CTSGun::AddAmmo(ammo, wextraclip)`: the key's raw number of rounds.
        let extra = self.extra_clip;
        if extra != 0
            && let Some(w) = tables().weapon(self.id as u8)
        {
            let empty = p.inv.gun.reserve.get(w.ammo as usize).is_some_and(|&r| r < 1);
            if empty && crate::weapons::give_ammo(p, pcx, w.ammo, extra) > 0 {
                crate::entities::stock::explode::emit_sound_slow(pcx.edict, CHAN_ITEM, c"items/9mmclip1.wav", 1.0, 0.8, 0, 100, 0.5);
            }
        }
        true
    }

    fn after_give(&mut self, cx: &mut Cx, player: *mut edict_t) {
        self.ground.taken(cx, self.respawn);
        let (message, target) = (cx.pev().message, cx.pev().target);
        if message != 0 {
            msg::ts_message_one(player, [0, 0, 0], 8, 30, &engine().string(message).to_string_lossy());
        }
        if target != 0 {
            crate::entities::fire_targets(engine().string(target), cx.edict, cx.edict, UseType::Toggle, 0.0);
        }
    }

    /// `CTSGroundWeapon::Fall` (0xa2150), its think, which `RespawnEntities` also calls directly:
    /// a weapon that has landed before (taken or not) shows again at once and drops its respawn
    /// timer.
    pub fn fall(&mut self, cx: &mut Cx) {
        self.ground.fall(cx, false);
    }

    /// +use from the player's own think (the player is the dispatched entity).
    pub fn player_use(&mut self, cx: &mut Cx, p: &mut Player, pcx: &mut Cx) {
        if self.give(cx, p, pcx) {
            self.after_give(cx, pcx.edict);
        }
    }
}

impl Entity for GroundWeapon {
    /// `CTSGroundWeapon::Save` / `Restore` (0xa1950 / 0xa19a0): the weapon id (`+0xa4`, the
    /// `tsweaponid` key, or the HL classname's random pick), `wduration` (`+0xa0`) and
    /// `wextraclip` (`+0xa8`). `Use` is a virtual, not `m_pfnUse`.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut Fields) {
        super::save_base(cx, f, &mut self.ground.funcs);
        f.table(&crate::save::tables::CTSGroundWeapon);
        f.int(&mut self.id);
        f.int(&mut self.respawn);
        f.int(&mut self.extra_clip);
    }

    // `CTSGroundWeapon::Precache` (0xa1cd4) is empty: nothing to do after a restore.

    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        let v = value.to_str().unwrap_or("").trim();
        let i = || v.parse::<i32>().unwrap_or(0);
        match key.to_bytes() {
            b"tsweaponid" => self.id = i(),
            b"wduration" => self.respawn = i(),
            b"wextraclip" => self.extra_clip = i(),
            b"buyable" => self.buyable = i() != 0,
            b"attackerteam" => self.attacker_team = i(),
            _ => return false,
        }
        true
    }

    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        let e = cx.edict;
        if let Some(ids) = hl_weapon_ids(eng.string(cx.pev().classname)) {
            self.id = ids[eng.random_long(0, ids.len() as i32 - 1) as usize];
        }
        let Some(asset) = u8::try_from(self.id)
            .ok()
            .and_then(|id| tables().asset(id))
            .filter(|_| self.id != BRIEFCASE && tables().weapon(self.id as u8).is_some())
        else {
            eng.print(&format!("[ts] ts_groundweapon: unknown weapon id {}, removed\n", self.id));
            crate::entities::remove(e);
            return;
        };
        let model = intern(&format!("models/w_{}.mdl", asset.w_model));
        eng.precache_model(model);
        eng.set_model(e, model);
        let wall = cx.pev().spawnflags & ON_WALL != 0;
        let v = cx.pev();
        v.movetype = if wall { MOVETYPE_FLY } else { MOVETYPE_TOSS } as i32;
        v.solid = SOLID_BBOX as i32;
        let org = v.origin;
        eng.set_origin(e, org);
        eng.set_size(e, [0.0; 3], [0.0; 3]);
        cx.think_in(0.1);
        self.ground.spawned(cx);
    }

    fn think(&mut self, cx: &mut Cx) {
        if self.ground.funcs.think {
            self.fall(cx);
        }
    }

    fn use_(&mut self, cx: &mut Cx, activator: *mut edict_t, _caller: *mut edict_t, _kind: UseType, _value: f32) {
        if !rules::is_player(activator) {
            return;
        }
        let took = with_player(activator, |p, pcx| self.give(cx, p, pcx)).unwrap_or(false);
        if took {
            self.after_give(cx, activator);
        }
    }
}

// ---- ts_groundammo -------------------------------------------------------------------------------

#[derive(Default)]
pub struct GroundAmmo {
    respawn: i32,
    /// Rounds per ammo type (index = TS ammo id).
    rounds: [i32; 16],
    ground: Ground,
}

impl Entity for GroundAmmo {
    /// `CBaseEntity::Save` / `Restore`: TS saves nothing of its own (`+0x64..` stay as the
    /// constructor left them).
    fn save_fields(&mut self, cx: &mut Cx, f: &mut Fields) {
        super::save_base(cx, f, &mut self.ground.funcs);
    }

    // `CTSGroundAmmo::Precache` (0xa28f0) is empty.

    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        let k = key.to_str().unwrap_or("");
        let v: i32 = value.to_str().unwrap_or("").trim().parse().unwrap_or(0);
        if k == "wduration" {
            self.respawn = v;
            return true;
        }
        if let Some(n) = k.strip_prefix("mags") {
            let ty: usize = n.trim().parse().unwrap_or(0);
            if let Some(a) = tables().ammo(ty as u8) {
                self.rounds[ty.min(15)] = v * a.per_buy;
            }
            return true;
        }
        false
    }

    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        let e = cx.edict;
        if let Some(ids) = hl_ammo_weapons(eng.string(cx.pev().classname)) {
            // HL ammo: half the max carry of each ammo type the class uses.
            let count = cvars_ammocount();
            for &id in ids {
                if let Some(w) = tables().weapon(id as u8)
                    && let Some(a) = tables().ammo(w.ammo)
                {
                    self.rounds[w.ammo as usize % 16] = a.max_carry[count] / 2;
                }
            }
        }
        eng.precache_model(c"models/ammopack.mdl");
        eng.set_model(e, c"models/ammopack.mdl");
        let wall = cx.pev().spawnflags & ON_WALL != 0;
        let v = cx.pev();
        v.movetype = if wall { MOVETYPE_FLY } else { MOVETYPE_TOSS } as i32;
        v.solid = SOLID_BBOX as i32;
        let org = v.origin;
        eng.set_origin(e, org);
        eng.set_size(e, [0.0; 3], [0.0; 3]);
        cx.think_in(0.1);
        self.ground.spawned(cx);
    }

    fn think(&mut self, cx: &mut Cx) {
        if self.ground.funcs.think {
            self.ground.fall(cx, true);
        }
    }

    fn touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        if !self.ground.funcs.touch || !self.ground.waiting || cx.pev().effects & EF_NODRAW as i32 != 0 || !rules::is_player(other) {
            return;
        }
        let rounds = self.rounds;
        let took = with_player(other, |p, pcx| {
            if !rules::alive(pcx.pev()) {
                return false;
            }
            let mut any = false;
            for (ty, &n) in rounds.iter().enumerate() {
                if n > 0 && crate::weapons::give_ammo(p, pcx, ty as u8, n) > 0 {
                    any = true;
                }
            }
            if any {
                engine().emit_sound(pcx.edict, CHAN_ITEM as i32, c"items/9mmclip1.wav", 1.0, 0.8, 0, 100);
            }
            any
        })
        .unwrap_or(false);
        if took {
            self.ground.taken(cx, self.respawn);
        }
    }
}

fn cvars_ammocount() -> usize {
    (rules::cvars::f(c"ammocount").round() as i32).clamp(0, 3) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remaps_are_real_weapons() {
        for c in [c"weapon_shotgun", c"weapon_9MMAR", c"weapon_handgrenade", c"ammo_ARgrenades"] {
            for &id in hl_weapon_ids(c).unwrap() {
                assert!(tables().weapon(id as u8).is_some(), "{c:?} → {id}");
                assert!(tables().asset(id as u8).is_some());
            }
        }
        assert!(hl_weapon_ids(c"weapon_crowbar").is_none());
    }

    #[test]
    fn buyable_price() {
        let t = tables();
        let deagle = t.weapon(12).unwrap();
        let ammo = t.ammo(deagle.ammo).unwrap().price;
        assert_eq!(buy_price(12, 0, 0), deagle.cost);
        assert_eq!(buy_price(12, 1 | 8 | 16, 0), deagle.cost + 900 + 1800);
        assert_eq!(buy_price(12, 2 | 4, 3), deagle.cost + 800 + 500 + 3 * ammo);
        assert_eq!(buy_price(12, 0, -2), deagle.cost);
        assert_eq!(buy_price(99, 0, 0), 0);
    }

    #[test]
    fn respawn_clamps() {
        assert_eq!(respawn_delay(0), 60.0);
        assert_eq!(respawn_delay(1), 1.0);
        assert_eq!(respawn_delay(45), 45.0);
        assert_eq!(respawn_delay(1000), 180.0);
    }
}
