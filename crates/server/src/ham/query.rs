//! The query and helper virtuals (`ObjectCaps`, `Classify`, `IsPlayer`, `Center`, `GiveSlowMul`,
//! ...) as the game asks them (version 4, issue #99): each port function that stands for one of
//! TS's virtual calls goes through [`query`], so a plugin's hook sees the call and may override the
//! answer. docs/implementation/plugin-api.md §7.5 lists the call sites.
//!
//! These don't take the object out of its slot (they read entvars or a constant), so the hook
//! fires for any game entity: with `this` reachable when it is in its slot or lent to it
//! (`world::run_now`), and out of reach when the game asks about an entity inside its own call
//! (a grenade's think asking its own `GiveSlowMul`), as the API treats a player inside its own
//! callback.

use super::*;

/// The query `ham` on `e`: the hooks around `run`, which computes the port's answer.
#[inline(always)]
pub(crate) fn query<A: Args, R: Ret>(ham: usize, e: *mut edict_t, mut args: A, run: impl FnOnce(&mut A) -> R) -> R {
    if !HOOKS.0.armed(ham) {
        return run(&mut args);
    }
    query_hooked(ham, e, args, run)
}

#[inline(never)]
fn query_hooked<A: Args, R: Ret>(ham: usize, e: *mut edict_t, mut args: A, run: impl FnOnce(&mut A) -> R) -> R {
    if e.is_null() || world::type_of(e).is_none() {
        return run(&mut args);
    }
    let mut reach = |f: &mut dyn FnMut()| {
        if world::run_now(e, &mut *f).is_none() {
            f();
        }
    };
    HOOKS.0.call(&Game, ham, e, &mut args, &mut reach, |a| Some(run(a))).unwrap_or_else(R::zero)
}

/// `pEntity->ObjectCaps()`: `run` is the class's caps (`PlayerUse`'s search, the master test).
pub fn object_caps(e: *mut edict_t, run: impl FnOnce() -> u32) -> u32 {
    query(id::OBJECTCAPS, e, (), |_| run() as i32) as u32
}

/// `pEntity->Classify()` (`entities::classify`): `TEXTURETYPE_PlaySound`'s test.
pub fn classify(e: *mut edict_t) -> i32 {
    query(id::CLASSIFY, e, (), |_| crate::entities::classify(e))
}

/// `pEntity->BloodColor()`: `run` is the class's.
pub fn blood_color(e: *mut edict_t, run: impl FnOnce() -> i32) -> i32 {
    query(id::BLOODCOLOR, e, (), |_| run())
}

/// `pEntity->IsAlive()` (`deadflag == DEAD_NO && health > 0`).
pub fn is_alive(e: *mut edict_t) -> bool {
    query(id::ISALIVE, e, (), |_| crate::rules::alive(e.v()))
}

/// `pEntity->IsBSPModel()`.
pub fn is_bsp_model(e: *mut edict_t) -> bool {
    query(id::ISBSPMODEL, e, (), |_| {
        let v = e.v();
        v.solid == SOLID_BSP as i32 || v.movetype == MOVETYPE_PUSHSTEP as i32
    })
}

/// `pEntity->IsInWorld()`: `run` is the test (`weapons::projectiles::is_in_world`).
pub fn is_in_world(e: *mut edict_t, run: impl FnOnce() -> bool) -> bool {
    query(id::ISINWORLD, e, (), |_| run())
}

/// `pEntity->IsPlayer()`, whose port answer is `real` (`rules::has_player`). Guard (port choice):
/// a hook can make a player not count as one, but not the reverse: the port has no player object
/// for anything else, where TS would treat the entity's memory as a `CBasePlayer`.
pub fn is_player(e: *mut edict_t, real: bool) -> bool {
    query(id::ISPLAYER, e, (), |_| real) && real
}

/// `pEntity->IsNetClient()`, whose port answer is `real` (`rules::is_net_client`); guarded as
/// [`is_player`].
pub fn is_net_client(e: *mut edict_t, real: bool) -> bool {
    query(id::ISNETCLIENT, e, (), |_| real) && real
}

/// `pEntity->Center()`: `run` is the port's (`(absmin + absmax) / 2`) from the entvars.
pub fn center(e: *mut edict_t, run: impl FnOnce(&entvars_t) -> [f32; 3]) -> [f32; 3] {
    query(id::CENTER, e, (), |_| run(e.v()))
}

/// `pEntity->EyePosition()`: `origin + view_ofs`.
pub fn eye_position(e: *mut edict_t) -> [f32; 3] {
    query(id::EYEPOSITION, e, (), |_| {
        let v = e.v();
        [0, 1, 2].map(|i| v.origin[i] + v.view_ofs[i])
    })
}

/// `m_pPlayer->GetGunPosition()`: `run` is the port's (`origin + view_ofs`).
pub fn gun_position(e: *mut edict_t, run: impl FnOnce() -> [f32; 3]) -> [f32; 3] {
    query(id::PLAYER_GETGUNPOSITION, e, (), |_| run())
}

/// `pEntity->CanUsedTroughWalls()` (`PlayerUse`).
pub fn can_use_through_walls(e: *mut edict_t) -> bool {
    query(id::TS_CANUSEDTHROUGHWALLS, e, (), |_| crate::entities::ts::can_use_through_walls(engine().string(e.v().classname)))
}

/// `this->GiveSlowMul()`: `run` is the port's value (`max(fuser1, 0.01)` or the gun's
/// `FixedSlowAmt`). Port deviation: the hook gets and may override the float (hamsandwich's
/// signature is `Void_Void`).
pub fn give_slow_mul(e: *mut edict_t, run: impl FnOnce() -> f32) -> f32 {
    query(id::TS_GIVESLOWMUL, e, (), |_| run())
}

/// `pEntity->InSlow()` (`rules::powerup::entity_in_slow`).
pub fn in_slow(e: *mut edict_t) -> bool {
    query(id::TS_INSLOW, e, (), |_| crate::rules::powerup::entity_in_slow(e))
}

/// `pEntity->IsObjective()`: `run` is the class test (`InitObjectives`, `CTSObjectivePointer::Use`).
pub fn is_objective(e: *mut edict_t, run: impl FnOnce() -> bool) -> bool {
    query(id::TS_ISOBJECTIVE, e, (), |_| run())
}

/// `pEntity->EnableObjective(on)` (`InitObjectives`, `CTSObjectivePointer::Use`): an outer call
/// on the objective (`objective::enable_objective` through `world::dispatch`).
pub fn enable_objective(e: *mut edict_t, on: bool) {
    outer(id::TS_ENABLEOBJECTIVE, e, &mut (on as i32,), |(on,)| {
        world::dispatch(e, |ent, cx| crate::entities::ts::objective::enable_objective(ent, cx, *on != 0))
    });
}

/// `pfnOnFreeEntPrivateData` (`OnFreeEntPrivateData` 0xab788): the entity's own
/// `OnFreeEntPrivateData` (empty in TS, 0xac650) with the hooks, then the object goes.
pub fn on_free_private_data(e: *mut edict_t) {
    query(id::TS_ONFREEENTPRIVATEDATA, e, (), |_| ());
    world::free(e);
}

/// `Killed(pevAttacker, iGib)` of a non-player from `CBaseEntity::TakeDamage` (0xab994), inside
/// the entity's own `TakeDamage`: `run` is `CBaseEntity::Killed`. The entity is out of reach for
/// the hook (its `TakeDamage` holds it; entvars are readable); the attacker and gib changes are
/// used.
pub fn killed_entity(cx: &mut Cx, dmg: &DamageInfo, run: impl FnOnce(&mut Cx)) {
    let e = cx.edict;
    let mut args = (Ent(dmg.attacker), gib_of(dmg.bits));
    if !HOOKS.0.armed(id::KILLED) {
        run(cx);
        return;
    }
    HOOKS.0.call(&Game, id::KILLED, e, &mut args, &mut plain, |_| {
        run(cx);
        Some(())
    });
}
