//! The functions the port has, by `Ham` number: which classes call the hook (`fires`), which
//! only run it on request (`exec`), the native argument types, and how `ham_execute` runs it.
//! docs/implementation/plugin-api.md §7 lists them with the TS functions they port.

use super::*;
use crate::entities::stock::playeritem::WorldTsGun;
use crate::entities::stock::{blood, breakable, items};
use shared::weapons::Virtual;
use std::any::TypeId;

/// A set of classes, by Rust type.
pub(crate) enum Classes {
    None,
    All,
    Player,
    Is(fn(TypeId) -> bool),
}

impl Classes {
    pub(crate) fn has(&self, ty: TypeId) -> bool {
        match self {
            Classes::None => false,
            Classes::All => true,
            Classes::Player => ty == TypeId::of::<Player>(),
            Classes::Is(f) => f(ty),
        }
    }
}

/// `ham_execute`'s body: decodes the checked arguments, runs the function (with the hooks if
/// asked) and writes `call.ret`. `None` when the object couldn't be reached (`BUSY`).
type Run = fn(*mut edict_t, &mut HamCall, &mut Pointers, bool) -> Option<()>;

pub(crate) struct Def {
    pub ham: usize,
    /// The hook runs when the game calls it on these (`ham_supported` 1).
    pub fires: Classes,
    /// `ham_execute` runs it on these too, but the game never calls it as a hookable unit
    /// (`ham_supported` 0).
    pub exec: Classes,
    /// The argument kinds and return kind of the native types: hamsandwich's signature.
    pub sig: fn() -> (&'static [c_int], c_int),
    pub run: Run,
}

impl Def {
    pub(crate) fn support(&self, ty: TypeId) -> c_int {
        if self.fires.has(ty) {
            1
        } else if self.exec.has(ty) {
            0
        } else {
            UNSUPPORTED
        }
    }

    pub(crate) fn kinds(&self) -> &'static [c_int] {
        (self.sig)().0
    }
}

fn sig<A: Args, R: Ret>() -> (&'static [c_int], c_int) {
    (A::KINDS, R::RET)
}

/// The function `ham`, if the port has it.
pub(crate) fn def(ham: c_int) -> Option<&'static Def> {
    DEFS.iter().find(|d| d.ham as c_int == ham)
}

/// The checked arguments of `call`, decoded over `a`.
fn args<A: Args>(call: &HamCall, mut a: A) -> A {
    a.take(&call.args, &Game);
    a
}

fn ret<R: Ret>(call: &mut HamCall, r: Option<R>) -> Option<()> {
    call.ret = r?.put(&Game);
    Some(())
}

const NULL: *mut edict_t = std::ptr::null_mut();

fn is_player(e: *mut edict_t) -> bool {
    world::type_of(e) == Some(TypeId::of::<Player>())
}

fn is_objective(ty: TypeId) -> bool {
    crate::entities::ts::objective::is_objective(ty, true)
}

fn classname(e: *mut edict_t) -> &'static CStr {
    engine().string(e.v().classname)
}

/// `weapon_tsgun` (`CTSGun`): the class of the `Item_*` / `Weapon_*` functions.
fn is_tsgun(ty: TypeId) -> bool {
    ty == TypeId::of::<WorldTsGun>()
}

/// A query's `ham_execute`: through [`query`](super::query::query) (its hooks) with `hooks`.
fn q<R: Ret>(hooks: bool, ham: usize, e: *mut edict_t, f: impl FnOnce() -> R) -> R {
    if hooks { super::query::query(ham, e, (), |_| f()) } else { f() }
}

/// `ham_execute` of a gun virtual on `this`: run on its holder's gun (`Gun::run_virtual`). `None`
/// (`BUSY`) for a `weapon_tsgun` no player holds (TS would run it with no `m_pPlayer`) or whose
/// holder is out of reach.
fn gun_exec(this: *mut edict_t, c: &mut HamCall, hooks: bool, v: Virtual) -> Option<()> {
    let owner = super::gun::holder(this)?;
    let r = crate::player::with_player(owner, |p, cx| crate::weapons::run_gun_virtual(p, cx, v, hooks))?;
    match v {
        Virtual::Deploy | Virtual::ShouldWeaponIdle => ret(c, Some(r)),
        _ => ret(c, Some(())),
    }
}

/// Port deviations from hamsandwich's signatures (`SIGS`): `TS_GiveSlowMul` returns its float.
pub(crate) const SIG_DEVIATIONS: &[(usize, &str)] = &[(id::TS_GIVESLOWMUL, "Float_Void")];

pub(crate) static DEFS: &[Def] = &[
    Def {
        ham: id::SPAWN,
        fires: Classes::All,
        exec: Classes::None,
        sig: sig::<(), ()>,
        run: |e, c, _, h| ret(c, exec(h, id::SPAWN, e, &mut (), |_| world::dispatch(e, |ent, cx| ent.spawn(cx)))),
    },
    Def {
        ham: id::KEYVALUE,
        fires: Classes::All,
        exec: Classes::None,
        sig: sig::<(Kvd,), ()>,
        run: |e, c, p, h| {
            let kv = p.kvd.as_mut()?;
            if !world::is_present(e) {
                return None;
            }
            let handled = if h {
                key_value(e, kv.class, kv.key, kv.value)
            } else {
                world::dispatch(e, |ent, cx| super::class_key_value(ent, cx, kv.key, kv.value))?
            };
            *kv.handled = handled as c_int;
            ret(c, Some(()))
        },
    },
    Def {
        ham: id::OBJECTCAPS,
        fires: Classes::All,
        exec: Classes::None,
        sig: sig::<(), i32>,
        // TS's `ObjectCaps` by class (`levelchange::ts_object_caps`); 0 for a name TS doesn't know.
        run: |e, c, _, h| {
            let caps =
                q(h, id::OBJECTCAPS, e, || crate::entities::stock::levelchange::ts_object_caps(classname(e), e.v()).unwrap_or(0) as i32);
            ret(c, Some(caps))
        },
    },
    Def {
        ham: id::ACTIVATE,
        fires: Classes::All,
        exec: Classes::None,
        sig: sig::<(), ()>,
        run: |e, c, _, h| ret(c, exec(h, id::ACTIVATE, e, &mut (), |_| world::dispatch(e, |ent, cx| ent.activate(cx)))),
    },
    Def {
        ham: id::CLASSIFY,
        fires: Classes::All,
        exec: Classes::None,
        sig: sig::<(), i32>,
        // TS's `Classify` overrides: `CLASS_PLAYER` (2) for a player, `CBaseEntity`'s `CLASS_NONE`
        // for most classes.
        run: |e, c, _, h| ret(c, Some(q(h, id::CLASSIFY, e, || crate::entities::classify(e)))),
    },
    Def {
        ham: id::TRACEATTACK,
        fires: Classes::All,
        exec: Classes::None,
        sig: sig::<TraceArgs, ()>,
        run: |e, c, p, h| {
            let tr = p.trace.as_deref_mut()?;
            let mut a: TraceArgs = args(c, (Ent(NULL), 0.0, [0.0; 3], *tr, 0));
            // TS's `TraceAttack` has no inflictor: `ApplyMultiDamage` names one.
            let base = DamageInfo::new(a.0.0, a.0.0, 0.0, 0).traced();
            let r = exec(h, id::TRACEATTACK, e, &mut a, |a| {
                let info = traced_info(&base, a);
                world::dispatch(e, |ent, cx| ent.trace_attack(cx, &info))
            });
            *tr = a.3;
            ret(c, r)
        },
    },
    Def {
        ham: id::TAKEDAMAGE,
        fires: Classes::All,
        exec: Classes::None,
        sig: sig::<DamageArgs, bool>,
        run: |e, c, _, h| {
            let mut a: DamageArgs = args(c, (Ent(NULL), Ent(NULL), 0.0, 0));
            let base = DamageInfo::new(NULL, NULL, 0.0, 0);
            ret(
                c,
                exec(h, id::TAKEDAMAGE, e, &mut a, |a| {
                    let info = damage_info(&base, a);
                    world::dispatch(e, |ent, cx| ent.take_damage(cx, &info))
                }),
            )
        },
    },
    Def {
        ham: id::TAKEHEALTH,
        fires: Classes::All,
        exec: Classes::None,
        sig: sig::<(f32, i32), bool>,
        run: |e, c, _, h| {
            let mut a = args(c, (0.0f32, 0i32));
            ret(c, exec_now(h, id::TAKEHEALTH, e, &mut a, |(amount, _)| Some(items::take_health_plain(e, *amount))))
        },
    },
    Def {
        ham: id::KILLED,
        fires: Classes::All,
        exec: Classes::None,
        sig: sig::<(Ent, i32), ()>,
        run: |e, c, _, h| {
            let mut a = args(c, (Ent(NULL), 0i32));
            if !is_player(e) {
                // `CBaseEntity::Killed` (0xabd4c), which the game calls from `CBaseEntity::TakeDamage`
                // (`entities::base_killed`, `ham::killed_entity`).
                return ret(c, exec(h, id::KILLED, e, &mut a, |_| world::dispatch(e, |_, cx| crate::entities::base_killed(cx))));
            }
            ret(
                c,
                exec(h, id::KILLED, e, &mut a, |(attacker, gib)| {
                    let dmg = killed_info(&DamageInfo::new(attacker.0, attacker.0, 0.0, 0), attacker.0, *gib);
                    crate::player::with_player(e, |p, cx| crate::rules::lifecycle::killed(p, cx, &dmg))
                }),
            )
        },
    },
    Def {
        ham: id::BLOODCOLOR,
        fires: Classes::All,
        exec: Classes::None,
        sig: sig::<(), i32>,
        // `CBasePlayer::Spawn` sets `m_bloodColor` red; `CBaseEntity::BloodColor` is `DONT_BLEED`.
        run: |e, c, _, h| ret(c, Some(q(h, id::BLOODCOLOR, e, || if is_player(e) { blood::BLOOD_COLOR_RED } else { blood::DONT_BLEED }))),
    },
    Def {
        ham: id::OVERRIDERESET,
        fires: Classes::All,
        exec: Classes::None,
        sig: sig::<(), ()>,
        run: |e, c, _, h| ret(c, exec(h, id::OVERRIDERESET, e, &mut (), |_| world::dispatch(e, |ent, cx| ent.override_reset(cx)))),
    },
    Def {
        ham: id::ISALIVE,
        fires: Classes::All,
        exec: Classes::None,
        sig: sig::<(), bool>,
        // `CBaseEntity::IsAlive` (and `CBaseMonster`'s): `deadflag == DEAD_NO && health > 0`.
        run: |e, c, _, h| ret(c, Some(q(h, id::ISALIVE, e, || crate::rules::alive(e.v())))),
    },
    Def {
        ham: id::ISBSPMODEL,
        fires: Classes::All,
        exec: Classes::None,
        sig: sig::<(), bool>,
        run: |e, c, _, h| {
            let v = e.v();
            let bsp = v.solid == SOLID_BSP as i32 || v.movetype == MOVETYPE_PUSHSTEP as i32;
            ret(c, Some(q(h, id::ISBSPMODEL, e, || bsp)))
        },
    },
    Def {
        ham: id::ISINWORLD,
        fires: Classes::All,
        exec: Classes::None,
        sig: sig::<(), bool>,
        // `CBaseEntity::IsInWorld` (SDK cbase.cpp): inside ±4096 and slower than 2000 per axis.
        run: |e, c, _, h| {
            let inside = crate::weapons::projectiles::is_in_world(e.v());
            ret(c, Some(q(h, id::ISINWORLD, e, || inside)))
        },
    },
    Def {
        ham: id::ISPLAYER,
        fires: Classes::All,
        exec: Classes::None,
        sig: sig::<(), bool>,
        run: |e, c, _, h| {
            let real = is_player(e);
            ret(c, Some(if h { super::query::is_player(e, real) } else { real }))
        },
    },
    Def {
        ham: id::ISNETCLIENT,
        fires: Classes::All,
        exec: Classes::None,
        sig: sig::<(), bool>,
        // `CBasePlayer::IsNetClient` is true, `CBot`'s (mp.dll) false.
        run: |e, c, _, h| {
            let real = if is_player(e) { world::with_lent::<Player, _>(e, |p| !p.cbot) } else { Some(false) };
            let real = real?;
            ret(c, Some(if h { super::query::is_net_client(e, real) } else { real }))
        },
    },
    Def {
        ham: id::THINK,
        fires: Classes::All,
        exec: Classes::None,
        sig: sig::<(), ()>,
        run: |e, c, _, h| ret(c, exec(h, id::THINK, e, &mut (), |_| world::dispatch(e, |ent, cx| ent.think(cx)))),
    },
    Def {
        ham: id::TOUCH,
        fires: Classes::All,
        exec: Classes::None,
        sig: sig::<(Ent,), ()>,
        run: |e, c, _, h| {
            let mut a = args(c, (Ent(NULL),));
            ret(c, exec(h, id::TOUCH, e, &mut a, |(o,)| world::dispatch(e, |ent, cx| super::class_touch(ent, cx, o.0))))
        },
    },
    Def {
        ham: id::USE,
        fires: Classes::All,
        exec: Classes::None,
        sig: sig::<(Ent, Ent, UseType, f32), ()>,
        run: |e, c, _, h| {
            let mut a = args(c, (Ent(NULL), Ent(NULL), UseType::Off, 0.0f32));
            ret(c, exec(h, id::USE, e, &mut a, |(a, cl, k, v)| world::dispatch(e, |ent, cx| ent.use_(cx, a.0, cl.0, *k, *v))))
        },
    },
    Def {
        ham: id::BLOCKED,
        fires: Classes::All,
        exec: Classes::None,
        sig: sig::<(Ent,), ()>,
        run: |e, c, _, h| {
            let mut a = args(c, (Ent(NULL),));
            ret(c, exec(h, id::BLOCKED, e, &mut a, |(o,)| world::dispatch(e, |ent, cx| ent.blocked(cx, o.0))))
        },
    },
    Def {
        ham: id::RESPAWN,
        fires: Classes::Is(items::is_item),
        exec: Classes::None,
        sig: sig::<(), Ent>,
        run: |e, c, _, h| {
            let r = exec(h, id::RESPAWN, e, &mut (), |_| {
                world::dispatch(e, |ent, cx| {
                    items::respawn_item(ent, cx);
                    Ent(cx.edict)
                })
            });
            ret(c, r)
        },
    },
    Def {
        ham: id::CENTER,
        fires: Classes::All,
        exec: Classes::None,
        sig: sig::<(), [f32; 3]>,
        run: |e, c, _, h| {
            let v = e.v();
            let centre = [0, 1, 2].map(|i| (v.absmin[i] + v.absmax[i]) * 0.5);
            ret(c, Some(q(h, id::CENTER, e, || centre)))
        },
    },
    Def {
        ham: id::EYEPOSITION,
        fires: Classes::All,
        exec: Classes::None,
        sig: sig::<(), [f32; 3]>,
        run: |e, c, _, h| {
            let v = e.v();
            let eye = [0, 1, 2].map(|i| v.origin[i] + v.view_ofs[i]);
            ret(c, Some(q(h, id::EYEPOSITION, e, || eye)))
        },
    },
    Def {
        ham: id::ITEM_CANDEPLOY,
        fires: Classes::None,
        exec: Classes::Is(is_tsgun),
        // `CBasePlayerWeapon::CanDeploy` (0xa0170): 1. The port's switches (`FShouldSwitchWeapon`,
        // `DefaultDeploy`) take it as a constant.
        sig: sig::<(), i32>,
        run: |_, c, _, _| ret(c, Some(1)),
    },
    Def {
        ham: id::ITEM_DEPLOY,
        fires: Classes::Is(is_tsgun),
        exec: Classes::None,
        sig: sig::<(), i32>,
        run: |e, c, _, h| gun_exec(e, c, h, Virtual::Deploy),
    },
    Def {
        ham: id::ITEM_CANHOLSTER,
        fires: Classes::None,
        exec: Classes::Is(is_tsgun),
        // `CBasePlayerItem::CanHolster` (0xf3444): 1; TS 3.0 never calls it.
        sig: sig::<(), i32>,
        run: |_, c, _, _| ret(c, Some(1)),
    },
    Def {
        ham: id::ITEM_HOLSTER,
        fires: Classes::Is(is_tsgun),
        exec: Classes::None,
        sig: sig::<(i32,), ()>,
        run: |e, c, _, h| gun_exec(e, c, h, Virtual::Holster),
    },
    Def {
        ham: id::ITEM_UPDATEITEMINFO,
        fires: Classes::None,
        exec: Classes::Is(is_tsgun),
        // `CBasePlayerWeapon::UpdateItemInfo` (0xf3670) is empty.
        sig: sig::<(), ()>,
        run: |_, c, _, _| ret(c, Some(())),
    },
    Def {
        ham: id::ITEM_PREFRAME,
        fires: Classes::Is(is_tsgun),
        exec: Classes::None,
        // `CBasePlayerItem::ItemPreFrame` (0xf3450) is empty.
        sig: sig::<(), ()>,
        run: |e, c, _, h| {
            q(h, id::ITEM_PREFRAME, e, || ());
            ret(c, Some(()))
        },
    },
    Def {
        ham: id::ITEM_POSTFRAME,
        fires: Classes::Is(is_tsgun),
        exec: Classes::None,
        sig: sig::<(), ()>,
        run: |e, c, _, h| gun_exec(e, c, h, Virtual::PostFrame),
    },
    Def {
        ham: id::ITEM_DROP,
        fires: Classes::Is(is_tsgun),
        exec: Classes::None,
        // `CBasePlayerItem::Drop` (0xf1cac): removed 0.1 s later. Guard: not a gun a player still
        // holds (TS would leave the player a dangling `m_pActiveItem`); that does nothing.
        sig: sig::<(), ()>,
        run: |e, c, _, h| {
            if super::gun::holder(e).is_none() {
                q(h, id::ITEM_DROP, e, || crate::entities::stock::playeritem::drop_item(e));
            }
            ret(c, Some(()))
        },
    },
    Def {
        ham: id::ITEM_UPDATECLIENTDATA,
        fires: Classes::Is(is_tsgun),
        exec: Classes::None,
        // `CTSGun::UpdateClientData(player)` (0x858c8): the holder's messages, whatever player is
        // passed (TS writes to the one it is given; the port's caches are the holder's).
        sig: sig::<(Ent,), i32>,
        run: |e, c, _, h| {
            let owner = super::gun::holder(e)?;
            crate::player::with_player(owner, |p, cx| {
                if h {
                    crate::weapons::update_client_data(p, cx);
                } else {
                    crate::weapons::gun_update_client_data(p, cx);
                }
            })?;
            ret(c, Some(1))
        },
    },
    Def {
        ham: id::ITEM_GETWEAPONPTR,
        fires: Classes::None,
        exec: Classes::Is(is_tsgun),
        // `CBasePlayerWeapon::GetWeaponPtr` (0xf3698): `this`. The port reaches the gun directly.
        sig: sig::<(), Ent>,
        run: |e, c, _, _| ret(c, Some(Ent(e))),
    },
    Def {
        ham: id::ITEM_ITEMSLOT,
        fires: Classes::None,
        exec: Classes::Is(is_tsgun),
        // `CTSGun::iItemSlot` (0x87264): 2 (slot list 2, where `HasPlayerItem` finds the gun).
        sig: sig::<(), i32>,
        run: |_, c, _, _| ret(c, Some(2)),
    },
    Def {
        ham: id::WEAPON_SENDWEAPONANIM,
        fires: Classes::Is(is_tsgun),
        exec: Classes::None,
        sig: sig::<(i32, i32, i32), ()>,
        run: |e, c, _, h| {
            let owner = super::gun::holder(e)?;
            let (anim, skip, body) = args(c, (0i32, 0i32, 0i32));
            let send = |a: i32, s: bool, b: i32| crate::weapons::send_weapon_anim_plain(owner, a, s, b);
            if h {
                super::gun::send_weapon_anim(e, anim, skip != 0, body, send);
            } else {
                send(anim, skip != 0, body);
            }
            ret(c, Some(()))
        },
    },
    Def {
        ham: id::WEAPON_PRIMARYATTACK,
        fires: Classes::Is(is_tsgun),
        exec: Classes::None,
        sig: sig::<(), ()>,
        run: |e, c, _, h| gun_exec(e, c, h, Virtual::PrimaryAttack),
    },
    Def {
        ham: id::WEAPON_SECONDARYATTACK,
        fires: Classes::Is(is_tsgun),
        exec: Classes::None,
        sig: sig::<(), ()>,
        run: |e, c, _, h| gun_exec(e, c, h, Virtual::SecondaryAttack),
    },
    Def {
        ham: id::WEAPON_RELOAD,
        fires: Classes::Is(is_tsgun),
        exec: Classes::None,
        sig: sig::<(), ()>,
        run: |e, c, _, h| gun_exec(e, c, h, Virtual::Reload),
    },
    Def {
        ham: id::WEAPON_WEAPONIDLE,
        fires: Classes::Is(is_tsgun),
        exec: Classes::None,
        sig: sig::<(), ()>,
        run: |e, c, _, h| gun_exec(e, c, h, Virtual::WeaponIdle),
    },
    Def {
        ham: id::WEAPON_SHOULDWEAPONIDLE,
        fires: Classes::Is(is_tsgun),
        exec: Classes::None,
        sig: sig::<(), i32>,
        run: |e, c, _, h| gun_exec(e, c, h, Virtual::ShouldWeaponIdle),
    },
    Def {
        ham: id::WEAPON_USEDECREMENT,
        fires: Classes::None,
        exec: Classes::Is(is_tsgun),
        // `CTSGun::UseDecrement` (0x8726c): 1. The port's weapon timers only count down (the form
        // it selects), so an override couldn't be honoured.
        sig: sig::<(), i32>,
        run: |_, c, _, _| ret(c, Some(1)),
    },
    Def {
        ham: id::PLAYER_JUMP,
        fires: Classes::Player,
        exec: Classes::None,
        sig: sig::<(), ()>,
        run: |e, c, _, h| ret(c, exec(h, id::PLAYER_JUMP, e, &mut (), |_| crate::player::with_player(e, crate::stunt::jump))),
    },
    Def {
        ham: id::PLAYER_DUCK,
        fires: Classes::Player,
        exec: Classes::None,
        sig: sig::<(), ()>,
        run: |e, c, _, h| ret(c, exec(h, id::PLAYER_DUCK, e, &mut (), |_| crate::player::with_player(e, crate::stunt::duck))),
    },
    Def {
        ham: id::PLAYER_PRETHINK,
        fires: Classes::Player,
        exec: Classes::None,
        sig: sig::<(), ()>,
        run: |e, c, _, h| ret(c, exec(h, id::PLAYER_PRETHINK, e, &mut (), |_| crate::player::with_player(e, |p, cx| p.pre_think(cx)))),
    },
    Def {
        ham: id::PLAYER_POSTTHINK,
        fires: Classes::Player,
        exec: Classes::None,
        sig: sig::<(), ()>,
        run: |e, c, _, h| ret(c, exec(h, id::PLAYER_POSTTHINK, e, &mut (), |_| crate::player::with_player(e, |p, cx| p.post_think(cx)))),
    },
    Def {
        ham: id::PLAYER_GETGUNPOSITION,
        fires: Classes::Player,
        exec: Classes::None,
        sig: sig::<(), [f32; 3]>,
        // `CBasePlayer::GetGunPosition`: `origin + view_ofs`.
        run: |e, c, _, h| {
            let v = e.v();
            let gun = [0, 1, 2].map(|i| v.origin[i] + v.view_ofs[i]);
            ret(c, Some(q(h, id::PLAYER_GETGUNPOSITION, e, || gun)))
        },
    },
    Def {
        ham: id::TS_BREAKABLERESPAWN,
        fires: Classes::Is(breakable::has_breakable_respawn),
        exec: Classes::All,
        sig: sig::<(i32,), i32>,
        run: |e, c, _, h| {
            let mut a = args(c, (0i32,));
            let r = exec(h, id::TS_BREAKABLERESPAWN, e, &mut a, |(force,)| {
                world::dispatch(e, |ent, cx| breakable::breakable_respawn(ent, cx, *force != 0))?;
                Some(0)
            });
            ret(c, r)
        },
    },
    Def {
        ham: id::TS_CANUSEDTHROUGHWALLS,
        fires: Classes::All,
        exec: Classes::None,
        sig: sig::<(), bool>,
        run: |e, c, _, h| {
            let r = if h { super::query::can_use_through_walls(e) } else { crate::entities::ts::can_use_through_walls(classname(e)) };
            ret(c, Some(r))
        },
    },
    Def {
        ham: id::TS_GIVESLOWMUL,
        fires: Classes::All,
        exec: Classes::None,
        // `CBaseEntity::GiveSlowMul` (0xac5b4), `CBasePlayer`'s (0x92960): `max(fuser1, 0.01)`.
        // Port deviation: returned as a float (hamsandwich declares it `Void_Void`).
        sig: sig::<(), f32>,
        run: |e, c, _, h| {
            let slow = crate::weapons::give_slow_mul(e.v().fuser1);
            ret(c, Some(q(h, id::TS_GIVESLOWMUL, e, || slow)))
        },
    },
    Def {
        ham: id::TS_GOSLOW,
        fires: Classes::All,
        exec: Classes::None,
        sig: sig::<(f32, i32), ()>,
        run: |e, c, _, h| {
            let mut a = args(c, (0.0f32, 0i32));
            ret(
                c,
                exec_now(h, id::TS_GOSLOW, e, &mut a, |(amount, flag)| {
                    crate::rules::powerup::go_slow_plain(e, *amount, *flag != 0);
                    Some(())
                }),
            )
        },
    },
    Def {
        ham: id::TS_INSLOW,
        fires: Classes::All,
        exec: Classes::None,
        sig: sig::<(), bool>,
        run: |e, c, _, h| ret(c, Some(q(h, id::TS_INSLOW, e, || crate::rules::powerup::entity_in_slow(e)))),
    },
    Def {
        ham: id::TS_ISOBJECTIVE,
        fires: Classes::All,
        exec: Classes::None,
        sig: sig::<(), bool>,
        // True for `CTSObjective` and `CTSWinGiver` (and `CTSScoreGiver`, which derives from it).
        run: |e, c, _, h| ret(c, Some(q(h, id::TS_ISOBJECTIVE, e, || world::type_of(e).is_some_and(is_objective)))),
    },
    Def {
        ham: id::TS_ENABLEOBJECTIVE,
        fires: Classes::All,
        exec: Classes::None,
        sig: sig::<(i32,), ()>,
        // The objectives' `EnableObjective` (`CTSWinGiver`'s resets its call counter and latch);
        // `CBaseEntity`'s (0xac62c) is empty (`objective::enable_objective` does nothing then).
        run: |e, c, _, h| {
            let mut a = args(c, (0i32,));
            ret(
                c,
                exec(h, id::TS_ENABLEOBJECTIVE, e, &mut a, |(on,)| {
                    world::dispatch(e, |ent, cx| crate::entities::ts::objective::enable_objective(ent, cx, *on != 0))
                }),
            )
        },
    },
    Def {
        ham: id::TS_ONFREEENTPRIVATEDATA,
        fires: Classes::All,
        exec: Classes::None,
        sig: sig::<(), ()>,
        // `CBaseEntity::OnFreeEntPrivateData` (0xac650) is empty and nothing overrides it; the
        // game calls it as the engine frees the entity (`ham::on_free_private_data`).
        run: |e, c, _, h| {
            q(h, id::TS_ONFREEENTPRIVATEDATA, e, || ());
            ret(c, Some(()))
        },
    },
    Def {
        ham: id::TS_SHOULDCOLLIDE,
        fires: Classes::All,
        exec: Classes::None,
        sig: sig::<(Ent,), bool>,
        run: |e, c, _, h| {
            let mut a = args(c, (Ent(NULL),));
            ret(c, exec(h, id::TS_SHOULDCOLLIDE, e, &mut a, |(o,)| world::dispatch(e, |ent, cx| ent.should_collide(cx, o.0))))
        },
    },
    Def {
        ham: id::TS_WEAPON_ALTERNATEATTACK,
        fires: Classes::Is(is_tsgun),
        exec: Classes::None,
        sig: sig::<(), ()>,
        run: |e, c, _, h| gun_exec(e, c, h, Virtual::AlternateAttack),
    },
];
