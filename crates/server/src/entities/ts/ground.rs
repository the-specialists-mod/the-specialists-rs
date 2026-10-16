//! `ts_groundweapon` (CTSGroundWeapon 0xa192c), `ts_groundammo` (CTSGroundAmmo 0xa26f4) and the
//! HL weapon/ammo classnames TS re-points at them (architecture.md §5.2, weapons.md §7.4).
//! Weapons are picked up with +use, ammo on touch. Giving goes through `crate::weapons`.

use crate::entities::stock::base::sub_remove;
use crate::player::{Player, with_player_now};
use crate::rules::mapcycle::atoi;
use crate::rules::{self, state};
use crate::save::{BaseFuncs, Fields};
use crate::world::{Cx, Entity, UseType};
use goldsrc::edict::EdictPtr;
use goldsrc::server::{engine, intern};
use goldsrc::sys::*;
use shared::pm::{Fpu, FpuMath, Num, ORDER_XYZ, ORDER_ZYX, below87, eq87};
use shared::weapons::{kind, kind_of, tables};
use std::ffi::CStr;

const BRIEFCASE: i32 = 29;
const ON_WALL: i32 = 16;
/// Entries of `global_weapon_info`; `get_weapon_info` (0x7f190) hands back entry 0 for any other
/// id, and `CTSGun::TakeWeapon` (0x80a04) refuses ids above 0x25.
const MAX_WEAPONS: i32 = 38;
/// Ammo slots of a `CTSGroundAmmo` (`+0x68..+0xa8`, one int per ammo type).
const AMMO_SLOTS: usize = 16;

/// `get_weapon_info(id)->valid` (`info+0x00`): set by every `Write_*` of `compile_weapons_info`
/// (0x7fe28) except `Write_melee`, so kung fu (0), the briefcase (29, `Write_flag` is never
/// called) and every id outside 0..38 (entry 0) have none.
pub fn weapon_valid(id: i32) -> bool {
    (1..MAX_WEAPONS).contains(&id) && tables().weapon(id as u8).is_some()
}

/// `CTSGroundWeapon::Spawn` (0xa1b40)'s check and remap of the weapon id: `None` for an id without
/// a valid `get_weapon_info` entry (the entity removes itself 0.1 s later); with `pairs_to_single`
/// ([`ServerRules::ground_weapon_pairs_to_single`](crate::gameplay::ServerRules::ground_weapon_pairs_to_single)),
/// an akimbo pair built from two singles (kind 4 with `info+0xcc` != -1) becomes its single, the
/// only form `PickUpWeapon` (0x9314c) accepts. The Golden Colts (kind 5, no single) stay.
pub fn spawn_weapon_id(id: i32, pairs_to_single: bool) -> Option<i32> {
    if !weapon_valid(id) {
        return None;
    }
    let w = tables().weapon(id as u8)?;
    Some(if pairs_to_single && kind_of(id as u8) == kind::AKIMBO && w.single != -1 { w.single } else { id })
}

/// HL classname → TS weapon ids it picks from, in `Random<Kind>`'s `RANDOM_LONG(1, n)` order.
pub fn hl_weapon_ids(class: &CStr) -> Option<&'static [i32]> {
    Some(match class.to_bytes() {
        b"weapon_crossbow" => &[18],
        b"weapon_shotgun" => &[4, 26, 33, 20, 11],
        b"weapon_gauss" | b"weapon_egon" => &[13, 5, 27, 15],
        b"weapon_rpg" => &[23, 7, 6, 19, 17, 3],
        // `RandomBigPistol` (0xc9d5c): 1 → 12, 2 → 31, 3 → 36.
        b"weapon_9MMAR" => &[12, 31, 36],
        b"weapon_hornetgun" => &[32],
        b"weapon_satchel" | b"ammo_ARgrenades" => &[25, 35],
        b"weapon_snark" => &[34],
        b"weapon_handgrenade" => &[24],
        _ => return None,
    })
}

/// The HL classnames whose TS class re-picks its weapon after each pickup (`GetWeapon` of
/// `CTSShotgunWeapon` 0xc95ac, `CTSRifleWeapon` 0xc974c, `CTSSMGWeapon` 0xc9918,
/// `CTSBigPistolWeapon` 0xc9cd0, `CTSKnifeWeapon` 0xc9e1c). The sniper, MG, katana and grenade
/// classes have one weapon and `CTSGroundWeapon`'s `GetWeapon`.
pub fn hl_weapon_repicks(class: &CStr) -> bool {
    matches!(
        class.to_bytes(),
        b"weapon_shotgun" | b"weapon_gauss" | b"weapon_egon" | b"weapon_rpg" | b"weapon_9MMAR" | b"weapon_satchel" | b"ammo_ARgrenades"
    )
}

/// `CTSMGWeapon::Spawn` (0xc9dac), `weapon_hornetgun`: `wextraclip` is 100 whatever the key said.
const MG_EXTRA_CLIP: i32 = 100;

/// HL ammo classname → the ammo types its TS class fills with half the max carry after
/// `CTSGroundAmmo::Spawn`: `CTSSniperAmmo` (0xc94a8: .50BMG and 7.62x51), `CTSShotgunAmmo`
/// (0xc9534), `CTSRifleAmmo` (0xc96b4), `CTSSMGAmmo` (0xc9848) and `CTSBigPistolAmmo` (0xc9c38:
/// .50AE and .454 only).
pub fn hl_ammo_types(class: &CStr) -> Option<&'static [u8]> {
    Some(match class.to_bytes() {
        b"ammo_crossbow" => &[8, 13],
        b"ammo_buckshot" => &[3],
        b"ammo_gaussclip" => &[6, 4],
        b"ammo_rpgclip" => &[2, 1, 7, 12],
        b"ammo_9mmAR" => &[5, 11],
        _ => return None,
    })
}

/// `get_ammo_info(t)->max_carry / 2` (`+0x0c`, C division) in `Set_ammo_count` column `col`, what
/// the HL ammo classes put in each of their types.
pub fn hl_ammo_rounds(t: u8, col: usize) -> i32 {
    tables().ammo(t).map_or(0, |a| a.max_carry[col.min(3)] / 2)
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

/// `GetWeapon`'s price of a `buyable` ground weapon: `get_weapon_info(id)->cost`, the
/// attachments in its spawnflags (900/800/500/1800), and the ammo price per extra magazine.
pub fn buy_price(id: i32, spawnflags: i32, extra_clip: i32) -> i32 {
    let t = tables();
    // `get_weapon_info` falls back to entry 0 (kung fu: no cost, no ammo).
    let w = u8::try_from(id).ok().filter(|&i| i32::from(i) < MAX_WEAPONS).and_then(|id| t.weapon(id));
    let prices = &t.combat.attachment_price;
    let mut cost = w.map_or(0, |w| w.cost);
    for (bit, price) in [(1, prices.silencer), (2, prices.laser), (4, prices.flashlight), (8, prices.scope)] {
        if spawnflags & bit != 0 {
            cost += price;
        }
    }
    let ammo_price = w.and_then(|w| t.ammo(w.ammo)).map_or(0, |a| a.price);
    cost.wrapping_add(ammo_price.wrapping_mul(extra_clip))
}

fn round_mode() -> bool {
    matches!(state::with(|g| g.mode), state::Mode::RoundTdm | state::Mode::Objective)
}

fn hidden(cx: &mut Cx) -> bool {
    cx.pev().effects & EF_NODRAW as i32 != 0
}

/// Shared falling / landing behaviour.
#[derive(Default)]
struct Ground {
    landed: bool,
    /// `m_pfnThink` is `SUB_Remove`, not `Fall` (`CTSGroundWeapon::Spawn` with an invalid id).
    /// Only meaningful while `funcs.think` is set; not saved (a restore clears the think).
    removing: bool,
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
        if ammo || crate::gameplay::active().server.ground_weapon_lands_flying() {
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
        self.removing = false;
        self.landed = true;
    }

    /// `Spawn`'s pointers (touch none, think `Fall`), `iuser4` (`= 0; |= 2`) and `+0x64` (landed).
    fn spawned(&mut self, cx: &mut Cx) {
        self.funcs = BaseFuncs { think: true, ..BaseFuncs::default() };
        self.removing = false;
        self.landed = false;
        cx.pev().iuser4 = 2;
    }

    /// Hide after a pickup and schedule the respawn. `solid` stays as it was (`SOLID_TRIGGER`
    /// once landed): only the touch goes.
    fn taken(&mut self, cx: &mut Cx, respawn: i32) {
        self.funcs.touch = false;
        let v = cx.pev();
        v.effects |= EF_NODRAW as i32;
        // A null think (`nextthink` left as it was) keeps it hidden until `RespawnEntities`.
        self.funcs.think = !(respawn < 1 && round_mode());
        self.removing = false;
        if self.funcs.think {
            v.nextthink = engine().time() + respawn_delay(respawn);
        }
    }
}

/// `PutOnWall` (0xa1cd8 / 0xa28f4): trace 1024 units in 8 horizontal directions and stick to the
/// closest surface it may use ([`wall_surface`]). TS keeps the best distance as a rounded int
/// (from 9999), so a later hit must be closer than that int.
fn put_on_wall(cx: &mut Cx) {
    let eng = engine();
    let e = cx.edict;
    let org = cx.pev().origin;
    let arith = rules::cvars::pm_arith();
    let mut best: Option<(i32, [f32; 3], [f32; 3])> = None;
    for k in 0..8 {
        let dir = shared::by_arith!(arith, wall_dir(k));
        let end = [org[0] + dir[0] * 1024.0, org[1] + dir[1] * 1024.0, org[2] + dir[2] * 1024.0];
        let tr = eng.trace_line(org, end, goldsrc::server::IGNORE_MONSTERS, e);
        if tr.flFraction >= 1.0 || !wall_surface(tr.pHit) {
            continue;
        }
        if let Some(kept) = shared::by_arith!(arith, wall_hit(org, tr.vecEndPos, best.map(|b| b.0))) {
            best = Some((kept, tr.vecEndPos, tr.vecPlaneNormal));
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

/// `PutOnWall`'s direction `k`: `Vector(cos a, sin a, 0).Normalize()` for `a = k · 45 · (π /
/// 180)` (the `double` constant times the int), the cosine stored. mp.dll (0x10075436..0x100754ab)
/// keeps the sine on the FPU, sums the squares `z, y, x` and leaves the root and `1 / len`
/// unrounded; `ts_i386.so` (0xa1d9c..0xa1e3c) stores the sine and the root, summing `x, y, z`.
fn wall_dir<F: Fpu>(k: i32) -> [f32; 3] {
    let rad = F::xd(0.017_453_292_519_943_295) * F::xi(k * 45);
    let c = rad.cos().f();
    if !F::MSVC {
        return F::vec_normalize([c, rad.sin().f(), 0.0], ORDER_XYZ);
    }
    let s = rad.sin();
    let len = (F::ZERO * F::ZERO + s * s + F::x(c) * F::x(c)).sqrt();
    if eq87(len, F::ZERO) {
        return [0.0, 0.0, 1.0];
    }
    let inv = F::x(1.0) / len;
    [(inv * F::x(c)).f(), (inv * s).f(), (inv * F::ZERO).f()]
}

/// `(origin - tr.vecEndPos).Length()` in `PutOnWall`: mp.dll (0x100755d9..0x10075623) stores the
/// differences and sums `z, y, x` with the root unrounded; `ts_i386.so` (0xa1f6b..0xa1fab) squares
/// the unrounded differences `x, y, z` and stores the root.
fn wall_length<F: Fpu>(origin: [f32; 3], end: [f32; 3]) -> F::N {
    if F::MSVC {
        return F::vec_length([0, 1, 2].map(|i| origin[i] - end[i]), ORDER_ZYX);
    }
    let d = [0, 1, 2].map(|i| F::x(origin[i]) - F::x(end[i]));
    F::x((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().f())
}

/// `PutOnWall`'s distance bookkeeping: a hit at `len` wins if it is closer than the kept int (9999
/// to start with), and is kept as an `(int)` cast (truncated): mp.dll compares and `__ftol`s the
/// unrounded length (`fcompp; test ah,1`, so a NaN wins), `ts_i386.so` the stored one (`jbe`, so a
/// NaN doesn't; the `fldcw` before the `fistpl` at 0xa1fe2 selects round-toward-zero).
fn wall_distance<F: Fpu>(len: F::N, kept: Option<i32>) -> Option<i32> {
    let best = F::xi(kept.unwrap_or(9999));
    let closer = if F::MSVC { below87(len, best) } else { best > len };
    closer.then(|| len.trunc_i32())
}

/// [`wall_distance`] of the hit at `end`.
fn wall_hit<F: Fpu>(origin: [f32; 3], end: [f32; 3], kept: Option<i32>) -> Option<i32> {
    wall_distance::<F>(wall_length::<F>(origin, end), kept)
}

/// `PutOnWall`'s test of what the trace hit (a NULL `pHit` is the world): anything without a game
/// object, or an entity that doesn't take damage (`takedamage != DAMAGE_YES`) and isn't hidden.
fn wall_surface(hit: *mut edict_t) -> bool {
    let hit = if hit.is_null() { engine().entity_of_index(0) } else { hit };
    if hit.is_null() || !hit.has_private_data() {
        return true;
    }
    let v = hit.v();
    v.takedamage != DAMAGE_YES as f32 && v.effects & EF_NODRAW as i32 == 0
}

// ---- ts_groundweapon ---------------------------------------------------------------------------

#[derive(Default)]
pub struct GroundWeapon {
    pub id: i32,
    /// `wduration`: the respawn delay after a pickup ([`respawn_delay`]).
    pub respawn: i32,
    /// `wextraclip`: rounds given with it when the player's reserve is empty.
    pub extra_clip: i32,
    /// `buyable`: in cash mode, the weapon costs [`buy_price`].
    pub buyable: bool,
    /// `attackerteam`: in the round modes, the only team that may buy it (-1 any).
    pub attacker_team: i32,
    ground: Ground,
}

impl GroundWeapon {
    /// `CTSGroundWeapon::Fall` (0xa2150), its think, which `RespawnEntities` also calls directly:
    /// a weapon that has landed before (taken or not) shows again at once and drops its respawn
    /// timer.
    pub fn fall(&mut self, cx: &mut Cx) {
        self.ground.fall(cx, false);
    }

    /// The class's `GetWeapon` (vtable +0x118), which `Use` and the briefcase's `WaitTouch` call
    /// with any entity. `CTSGroundWeapon`'s gives to a player ([`give`](Self::give)). The
    /// re-picking HL classes ([`hl_weapon_repicks`]) do nothing while hidden; otherwise they run
    /// it, draw a new weapon if it was taken, and set the model again (even when nothing was
    /// given, or `other` isn't a player).
    fn get_weapon(&mut self, cx: &mut Cx, player: Option<(&mut Player, &mut Cx)>) {
        let eng = engine();
        let repicks = hl_weapon_repicks(eng.string(cx.pev().classname));
        if repicks && hidden(cx) {
            return;
        }
        if let Some((p, pcx)) = player {
            crate::gameplay::active().server.give_ground_weapon(self, cx, p, pcx);
        }
        if !repicks {
            return;
        }
        if hidden(cx) {
            self.pick(cx);
        }
        if let Some(model) = self.model() {
            eng.set_model(cx.edict, model);
        }
    }

    /// [`get_weapon`](Self::get_weapon) for an entity: `GetWeapon` goes on only for `IsPlayer()`
    /// (vtable 50), which every `CBasePlayer` answers, connected or not (`rules::has_player`).
    pub fn get_weapon_for(&mut self, cx: &mut Cx, other: *mut edict_t) {
        if rules::has_player(other) && with_player_now(other, |p, pcx| self.get_weapon(cx, Some((p, pcx)))).is_some() {
            return;
        }
        // Not a player (or one held by a dispatch further up, which the port can't reach).
        self.get_weapon(cx, None);
    }

    /// The weapon id an HL classname's TS class sets in `Spawn`: `Random<Kind>`'s
    /// `RANDOM_LONG(1, n)` pick, or the one weapon of a class without one (no random draw).
    fn pick(&mut self, cx: &mut Cx) {
        let eng = engine();
        match hl_weapon_ids(eng.string(cx.pev().classname)) {
            Some(&[id]) => self.id = id,
            Some(ids) => self.id = ids[eng.random_long(1, ids.len() as i32) as usize - 1],
            None => {}
        }
    }

    /// The world model of the weapon id (`get_weapon_info(id) + 0x1f2`), which `TSPrecache`
    /// precaches for every weapon (`weapons::precache_weapon_assets`).
    fn model(&self) -> Option<&'static CStr> {
        let asset = u8::try_from(self.id).ok().and_then(|id| tables().asset(id))?;
        Some(intern(&format!("models/w_{}.mdl", asset.w_model)))
    }

    /// Whether `m_pfnTouch` is `WaitTouch`: landed and not taken.
    pub fn touchable(&self) -> bool {
        self.ground.funcs.touch
    }

    /// The pickup's hide and respawn schedule (`GetWeapon` after `PickUpWeapon` succeeds): no
    /// touch, `EF_NODRAW`, and `Fall` again after [`respawn_delay`] unless a round mode keeps
    /// it for `RespawnEntities`.
    pub fn taken(&mut self, cx: &mut Cx) {
        self.ground.taken(cx, self.respawn);
    }

    /// +use from the player's own think (the player is the dispatched entity).
    pub fn player_use(&mut self, cx: &mut Cx, p: &mut Player, pcx: &mut Cx) {
        self.get_weapon(cx, Some((p, pcx)));
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

    /// `CTSGroundWeapon::KeyValue` (0xa19f0): `strtol(value, 0, 10)` for each key.
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        let i = || atoi(&value.to_string_lossy());
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

    /// The HL classes' `Spawn` sets the id (and the MG its `wextraclip`) before
    /// `CTSGroundWeapon::Spawn` (0xa1b40), whose empty `Precache` precaches nothing.
    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        let e = cx.edict;
        self.pick(cx);
        if eng.string(cx.pev().classname) == c"weapon_hornetgun" {
            self.extra_clip = MG_EXTRA_CLIP;
        }
        let Some(id) = spawn_weapon_id(self.id, crate::gameplay::active().server.ground_weapon_pairs_to_single()) else {
            // `SetThink(SUB_Remove)`, 0.1 s later; nothing else is set up.
            self.ground.funcs.think = true;
            self.ground.removing = true;
            cx.think_in(0.1);
            return;
        };
        self.id = id;
        if let Some(model) = self.model() {
            eng.set_model(e, model);
        }
        let wall = cx.pev().spawnflags & ON_WALL != 0;
        let v = cx.pev();
        v.movetype = if wall { MOVETYPE_FLY } else { MOVETYPE_TOSS } as i32;
        v.solid = SOLID_BBOX as i32;
        let org = v.origin;
        eng.set_origin(e, org);
        eng.set_size(e, [0.0; 3], [0.0; 3]);
        self.ground.spawned(cx);
        cx.think_in(0.1);
    }

    fn think(&mut self, cx: &mut Cx) {
        if !self.ground.funcs.think {
            return;
        }
        if self.ground.removing {
            sub_remove(cx.edict);
        } else {
            self.fall(cx);
        }
    }

    /// `CTSGroundWeapon::WaitTouch` (0xa26cc): only the briefcase (29) is taken on touch, which
    /// `Spawn` never lets through.
    fn touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        if self.ground.funcs.touch && self.id == BRIEFCASE {
            self.get_weapon_for(cx, other);
        }
    }

    /// `CTSGroundWeapon::Use` (0xa230c): `GetWeapon(activator)`.
    fn use_(&mut self, cx: &mut Cx, activator: *mut edict_t, _caller: *mut edict_t, _kind: UseType, _value: f32) {
        self.get_weapon_for(cx, activator);
    }
}

// ---- ts_groundammo -------------------------------------------------------------------------------

#[derive(Default)]
pub struct GroundAmmo {
    respawn: i32,
    /// Rounds per ammo type (index = TS ammo id).
    rounds: [i32; AMMO_SLOTS],
    ground: Ground,
}

impl Entity for GroundAmmo {
    /// `CBaseEntity::Save` / `Restore`: TS saves nothing of its own (`+0x64..` stay as the
    /// constructor left them).
    fn save_fields(&mut self, cx: &mut Cx, f: &mut Fields) {
        super::save_base(cx, f, &mut self.ground.funcs);
    }

    // `CTSGroundAmmo::Precache` (0xa28f0) is empty.

    /// `CTSGroundAmmo::KeyValue` (0xa2718): `wduration`, and `mags<t>` = `strtol(value)` ×
    /// `get_ammo_info(t)->per_buy` (`+0x10`; 0 for the empty entries 0, 14 and 15). A `mags` key
    /// leaves `fHandled` as it was (unhandled).
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        let v = atoi(&value.to_string_lossy());
        if key.to_bytes() == b"wduration" {
            self.respawn = v;
            return true;
        }
        if let Some(n) = key.to_bytes().strip_prefix(b"mags") {
            // Port guard: TS writes `+0x68 + t * 4` for any `t` (`strtol` as unsigned), past the
            // 16 slots for t >= 16; the port ignores those.
            let t = atoi(&String::from_utf8_lossy(n)) as u32 as usize;
            if t < AMMO_SLOTS {
                let per_buy = tables().ammo(t as u8).map_or(0, |a| a.per_buy);
                self.rounds[t] = v.wrapping_mul(per_buy);
            }
        }
        false
    }

    /// `CTSGroundAmmo::Spawn` (0xa27e4), then the HL ammo classes' half max carry
    /// ([`hl_ammo_types`]) in the `Set_ammo_count` column in force: the map's entities spawn
    /// before the gamerules' first `Think` applies `ammocount`, so that is the default column.
    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        let e = cx.edict;
        eng.set_model(e, c"models/ammopack.mdl");
        let wall = cx.pev().spawnflags & ON_WALL != 0;
        let v = cx.pev();
        v.movetype = if wall { MOVETYPE_FLY } else { MOVETYPE_TOSS } as i32;
        v.solid = SOLID_BBOX as i32;
        let org = v.origin;
        eng.set_origin(e, org);
        eng.set_size(e, [0.0; 3], [0.0; 3]);
        self.ground.spawned(cx);
        cx.think_in(0.1);
        if let Some(types) = hl_ammo_types(eng.string(cx.pev().classname)) {
            let col = state::ammo_column();
            for &t in types {
                self.rounds[t as usize] = hl_ammo_rounds(t, col);
            }
        }
    }

    fn think(&mut self, cx: &mut Cx) {
        if self.ground.funcs.think {
            self.ground.fall(cx, true);
        }
    }

    /// `CTSGroundAmmo::WaitTouch` (0xa2f40): a visible pack, `IsPlayer()` (any `CBasePlayer`),
    /// `IsAlive()` and a gun; `AddAmmo` for every non-zero slot, and if any took, the slow-motion
    /// pickup sound, hide and respawn.
    fn touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        if !self.ground.funcs.touch || hidden(cx) || !rules::has_player(other) {
            return;
        }
        let rounds = self.rounds;
        let took = with_player_now(other, |p, pcx| {
            if !rules::is_alive(pcx.edict) {
                return false;
            }
            let mut any = false;
            for (ty, &n) in rounds.iter().enumerate() {
                if n != 0 && crate::weapons::give_ammo(p, pcx, ty as u8, n) > 0 {
                    any = true;
                }
            }
            if any {
                crate::entities::stock::explode::emit_sound_slow(pcx.edict, CHAN_ITEM, c"items/9mmclip1.wav", 1.0, 0.8, 0, 100, 0.5);
            }
            any
        })
        .unwrap_or(false);
        if took {
            self.ground.taken(cx, self.respawn);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remaps_are_real_weapons() {
        ts30_gameplay::tables::make_default();
        for c in [c"weapon_shotgun", c"weapon_9MMAR", c"weapon_handgrenade", c"ammo_ARgrenades"] {
            for &id in hl_weapon_ids(c).unwrap() {
                assert!(tables().weapon(id as u8).is_some(), "{c:?} → {id}");
                assert!(tables().asset(id as u8).is_some());
            }
        }
        assert!(hl_weapon_ids(c"weapon_crowbar").is_none());
    }

    #[test]
    fn repicking_classes() {
        // The classes with a `Random<Kind>` re-pick after each pickup.
        for c in [c"weapon_shotgun", c"weapon_gauss", c"weapon_egon", c"weapon_rpg", c"weapon_9MMAR", c"weapon_satchel", c"ammo_ARgrenades"]
        {
            assert!(hl_weapon_repicks(c), "{c:?}");
            assert!(hl_weapon_ids(c).unwrap().len() > 1, "{c:?}");
        }
        // RANDOM_LONG(1, n) order of the Random<Kind> switches.
        assert_eq!(hl_weapon_ids(c"weapon_9MMAR"), Some(&[12, 31, 36][..]));
        assert_eq!(hl_weapon_ids(c"weapon_rpg"), Some(&[23, 7, 6, 19, 17, 3][..]));
        // One weapon each, CTSGroundWeapon's GetWeapon.
        for c in [c"weapon_crossbow", c"weapon_hornetgun", c"weapon_snark", c"weapon_handgrenade", c"ts_groundweapon"] {
            assert!(!hl_weapon_repicks(c), "{c:?}");
        }
    }

    #[test]
    fn valid_ids_follow_weapon_info() {
        ts30_gameplay::tables::make_default();
        // Kung fu and the briefcase have no `valid` byte; ids outside 0..38 read entry 0.
        for id in [0, 29, 38, 39, 255, 256 + 5, -1, i32::MIN] {
            assert!(!weapon_valid(id), "{id}");
            assert_eq!(spawn_weapon_id(id, true), None, "{id}");
        }
        for id in (1..38).filter(|&id| id != 29) {
            assert!(weapon_valid(id), "{id}");
            // Every weapon a ground weapon can show has a precached world model.
            assert!(tables().asset(id as u8).is_some(), "{id}");
        }
    }

    #[test]
    fn spawn_turns_akimbo_pairs_into_singles() {
        ts30_gameplay::tables::make_default();
        // Berettas, MK23, Mini-Uzis, Five-seveNs, Skorpions → their single.
        for (pair, single) in [(8, 2), (10, 9), (16, 3), (30, 14), (37, 17)] {
            assert_eq!(spawn_weapon_id(pair, true), Some(single), "{pair}");
        }
        // The Golden Colts (kind 5, no single) and ordinary weapons keep their id.
        assert_eq!(spawn_weapon_id(21, true), Some(21));
        for id in [1, 2, 12, 18, 24, 34] {
            assert_eq!(spawn_weapon_id(id, true), Some(id));
        }
        // Every id a ground weapon ends up with is one `PickUpWeapon` accepts.
        for id in 0..40 {
            if let Some(got) = spawn_weapon_id(id, true) {
                assert!(weapon_valid(got));
                assert_ne!(kind_of(got as u8), kind::AKIMBO, "{id}");
            }
        }
        // Without the remap (TS 2.1's `Spawn` 0x10065c70, issue #224) every valid id stays.
        for id in 0..40 {
            assert_eq!(spawn_weapon_id(id, false), weapon_valid(id).then_some(id), "{id}");
        }
    }

    #[test]
    fn hl_ammo_classes_fill_their_types() {
        ts30_gameplay::tables::make_default();
        assert_eq!(hl_ammo_types(c"ammo_crossbow"), Some(&[8, 13][..]));
        assert_eq!(hl_ammo_types(c"ammo_buckshot"), Some(&[3][..]));
        assert_eq!(hl_ammo_types(c"ammo_gaussclip"), Some(&[6, 4][..]));
        assert_eq!(hl_ammo_types(c"ammo_rpgclip"), Some(&[2, 1, 7, 12][..]));
        assert_eq!(hl_ammo_types(c"ammo_9mmAR"), Some(&[5, 11][..]));
        assert_eq!(hl_ammo_types(c"ts_groundammo"), None);
        // Half the max carry (default column: 50BMG 30, 7.62x51 35 → 17).
        assert_eq!(hl_ammo_rounds(8, 0), 15);
        assert_eq!(hl_ammo_rounds(13, 0), 17);
        assert_eq!(hl_ammo_rounds(1, 1), 45);
        assert_eq!(hl_ammo_rounds(3, 3), 0);
        assert_eq!(hl_ammo_rounds(0, 0), 0);
    }

    #[test]
    fn buyable_price() {
        ts30_gameplay::tables::make_default();
        let t = tables();
        let deagle = t.weapon(12).unwrap();
        let ammo = t.ammo(deagle.ammo).unwrap().price;
        assert_eq!(buy_price(12, 0, 0), deagle.cost);
        assert_eq!(buy_price(12, 1 | 8 | 16, 0), deagle.cost + 900 + 1800);
        assert_eq!(buy_price(12, 2 | 4, 3), deagle.cost + 800 + 500 + 3 * ammo);
        // `GetWeapon` clamps a negative `wextraclip` to 0 before pricing.
        assert_eq!(buy_price(12, 0, 0), deagle.cost);
        // An id outside the table prices as entry 0 (kung fu): attachments only.
        assert_eq!(buy_price(99, 0, 0), 0);
        assert_eq!(buy_price(99, 1, 4), 900);
    }

    #[test]
    fn wall_distance_is_kept_as_an_int() {
        use shared::pm::{Gcc, Msvc};
        fn go<F: Fpu>() {
            let d = |v: f32| wall_distance::<F>(F::x(v), None);
            assert_eq!(d(100.4), Some(100));
            assert_eq!(d(100.9), Some(100));
            // Closer than the first hit, but not than its truncated distance.
            assert_eq!(wall_distance::<F>(F::x(100.2), Some(100)), None);
            assert_eq!(wall_distance::<F>(F::x(99.9), Some(100)), Some(99));
            assert_eq!(wall_distance::<F>(F::x(99.5), Some(99)), None);
            assert_eq!(wall_distance::<F>(F::x(98.5), Some(100)), Some(98));
            assert_eq!(d(9999.0), None);
        }
        go::<Msvc>();
        go::<Gcc>();
        // A NaN length passes mp.dll's `test ah,1` but not the .so's `jbe`.
        assert!(wall_distance::<Msvc>(f64::NAN, None).is_some());
        assert_eq!(wall_distance::<Gcc>(Gcc::x(f32::NAN), None), None);
    }

    /// `PutOnWall`'s distance (issue #171): mp.dll truncates the unrounded root, 740.99998; the
    /// .so (and the port's old `float` length) rounds it to 741.0 first.
    #[test]
    fn wall_distance_rounds_as_each_build() {
        use shared::pm::{Gcc, Msvc};
        let f = |b: [u32; 3]| b.map(f32::from_bits);
        let (org, end) = (f([0xc2da_01de, 0x445a_d2ed, 0x44bd_680a]), f([0x4285_7810, 0x431b_6fc8, 0x44bd_680a]));
        assert_eq!(wall_hit::<Msvc>(org, end, None), Some(740));
        assert_eq!(wall_hit::<Gcc>(org, end, None), Some(741));
        // The eight directions: mp.dll's unrounded sine normalises to the stored cos and sin.
        assert_eq!(wall_dir::<Msvc>(1).map(f32::to_bits), [0x3f35_04f3, 0x3f35_04f3, 0]);
        assert_eq!(wall_dir::<Msvc>(0), [1.0, 0.0, 0.0]);
        assert_eq!(wall_dir::<Gcc>(3).map(f32::to_bits), [0xbf35_04f3, 0x3f35_04f3, 0]);
    }

    #[test]
    fn respawn_clamps() {
        assert_eq!(respawn_delay(0), 60.0);
        assert_eq!(respawn_delay(1), 1.0);
        assert_eq!(respawn_delay(45), 45.0);
        assert_eq!(respawn_delay(1000), 180.0);
    }
}
