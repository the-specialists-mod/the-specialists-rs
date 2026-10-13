//! The functions the port has, by `Ham` number: which classes call the hook (`fires`), which
//! only run it on request (`exec`), the native argument types, and how `ham_execute` runs it.
//! docs/implementation/plugin-api.md §7 lists them with the TS functions they port.

use super::*;
use crate::entities::stock::{blood, breakable, items};
use crate::entities::ts::ctb::WinGiver;
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
    ty == TypeId::of::<WinGiver<false>>() || ty == TypeId::of::<WinGiver<true>>()
}

fn classname(e: *mut edict_t) -> &'static CStr {
    engine().string(e.v().classname)
}

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
        fires: Classes::None,
        exec: Classes::All,
        sig: sig::<(), i32>,
        // TS's `ObjectCaps` by class (`levelchange::ts_object_caps`); 0 for a name TS doesn't know.
        run: |e, c, _, _| {
            let caps = crate::entities::stock::levelchange::ts_object_caps(classname(e), e.v()).unwrap_or(0);
            ret(c, Some(caps as i32))
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
        fires: Classes::None,
        exec: Classes::All,
        sig: sig::<(), i32>,
        // `CBasePlayer::Classify` is `CLASS_PLAYER` (2); `CBaseEntity`'s `CLASS_NONE`.
        run: |e, c, _, _| ret(c, Some(if is_player(e) { 2 } else { 0 })),
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
        fires: Classes::Player,
        exec: Classes::All,
        sig: sig::<(Ent, i32), ()>,
        run: |e, c, _, h| {
            let mut a = args(c, (Ent(NULL), 0i32));
            if !is_player(e) {
                // `CBaseEntity::Killed` (0xabd4c): the other classes' deaths aren't separate calls
                // in the port (`entities::base_killed` inside their `TakeDamage`).
                return ret(c, world::dispatch(e, |_, cx| crate::entities::base_killed(cx)));
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
        fires: Classes::None,
        exec: Classes::All,
        sig: sig::<(), i32>,
        // `CBasePlayer::Spawn` sets `m_bloodColor` red; `CBaseEntity::BloodColor` is `DONT_BLEED`.
        run: |e, c, _, _| ret(c, Some(if is_player(e) { blood::BLOOD_COLOR_RED } else { blood::DONT_BLEED })),
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
        fires: Classes::None,
        exec: Classes::All,
        sig: sig::<(), bool>,
        // `CBaseEntity::IsAlive` (and `CBaseMonster`'s): `deadflag == DEAD_NO && health > 0`.
        run: |e, c, _, _| {
            let v = e.v();
            ret(c, Some(v.deadflag == DEAD_NO as i32 && v.health > 0.0))
        },
    },
    Def {
        ham: id::ISBSPMODEL,
        fires: Classes::None,
        exec: Classes::All,
        sig: sig::<(), bool>,
        run: |e, c, _, _| {
            let v = e.v();
            ret(c, Some(v.solid == SOLID_BSP as i32 || v.movetype == MOVETYPE_PUSHSTEP as i32))
        },
    },
    Def {
        ham: id::ISINWORLD,
        fires: Classes::None,
        exec: Classes::All,
        sig: sig::<(), bool>,
        // `CBaseEntity::IsInWorld` (SDK cbase.cpp): inside ±4096 and slower than 2000 per axis.
        run: |e, c, _, _| {
            let v = e.v();
            let inside = v.origin.iter().all(|o| o.abs() < 4096.0) && v.velocity.iter().all(|s| s.abs() < 2000.0);
            ret(c, Some(inside))
        },
    },
    Def { ham: id::ISPLAYER, fires: Classes::None, exec: Classes::All, sig: sig::<(), bool>, run: |e, c, _, _| ret(c, Some(is_player(e))) },
    Def {
        ham: id::ISNETCLIENT,
        fires: Classes::None,
        exec: Classes::All,
        sig: sig::<(), bool>,
        // `CBasePlayer::IsNetClient` is true, `CBot`'s (mp.dll) false.
        run: |e, c, _, _| {
            if !is_player(e) {
                return ret(c, Some(false));
            }
            ret(c, world::with_lent::<Player, _>(e, |p| !p.cbot))
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
        fires: Classes::None,
        exec: Classes::All,
        sig: sig::<(), [f32; 3]>,
        run: |e, c, _, _| {
            let v = e.v();
            ret(c, Some([0, 1, 2].map(|i| (v.absmin[i] + v.absmax[i]) * 0.5)))
        },
    },
    Def {
        ham: id::EYEPOSITION,
        fires: Classes::None,
        exec: Classes::All,
        sig: sig::<(), [f32; 3]>,
        run: |e, c, _, _| {
            let v = e.v();
            ret(c, Some([0, 1, 2].map(|i| v.origin[i] + v.view_ofs[i])))
        },
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
        fires: Classes::None,
        exec: Classes::Player,
        sig: sig::<(), [f32; 3]>,
        // `CBasePlayer::GetGunPosition`: `origin + view_ofs`.
        run: |e, c, _, _| {
            let v = e.v();
            ret(c, Some([0, 1, 2].map(|i| v.origin[i] + v.view_ofs[i])))
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
        fires: Classes::None,
        exec: Classes::All,
        sig: sig::<(), bool>,
        run: |e, c, _, _| ret(c, Some(crate::entities::ts::can_use_through_walls(classname(e)))),
    },
    Def {
        ham: id::TS_GIVESLOWMUL,
        fires: Classes::None,
        exec: Classes::All,
        // hamsandwich declares it `Void_Void`, so the value (`CBaseEntity::GiveSlowMul` 0xac5b4,
        // `CBasePlayer`'s 0x92960: `max(fuser1, 0.01)`) can't be returned; it computes nothing.
        sig: sig::<(), ()>,
        run: |_, c, _, _| ret(c, Some(())),
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
        fires: Classes::None,
        exec: Classes::All,
        sig: sig::<(), bool>,
        run: |e, c, _, _| ret(c, Some(crate::rules::powerup::entity_in_slow(e))),
    },
    Def {
        ham: id::TS_ISOBJECTIVE,
        fires: Classes::None,
        exec: Classes::All,
        sig: sig::<(), bool>,
        // True for `CTSObjective` and `CTSWinGiver` (and `CTSScoreGiver`, which derives from it).
        run: |e, c, _, _| ret(c, Some(world::type_of(e).is_some_and(is_objective))),
    },
    Def {
        ham: id::TS_ENABLEOBJECTIVE,
        fires: Classes::None,
        exec: Classes::All,
        sig: sig::<(i32,), ()>,
        // `CTSWinGiver::EnableObjective` resets its call counter and latch; `CBaseEntity`'s
        // (0xac62c) is empty.
        run: |e, c, _, _| {
            if !world::type_of(e).is_some_and(is_objective) {
                return ret(c, Some(()));
            }
            ret(c, world::dispatch(e, |ent, _| crate::entities::ts::ctb::enable_objective(ent)))
        },
    },
    Def {
        ham: id::TS_ONFREEENTPRIVATEDATA,
        fires: Classes::None,
        exec: Classes::All,
        sig: sig::<(), ()>,
        // `CBaseEntity::OnFreeEntPrivateData` (0xac650) is empty and nothing overrides it.
        run: |_, c, _, _| ret(c, Some(())),
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
];
