//! The `Item_*` / `Weapon_*` functions on `weapon_tsgun` (version 4, issue #99). TS's weapons are
//! one `CTSGun` per player holding up to six guns; the port keeps its state in the player's
//! inventory (`weapons::Inventory`) and the `weapon_tsgun` entity (`WorldTsGun`) only follows the
//! player. So the hooks fire with `this` that entity, around the calls TS makes through the gun's
//! vtable, with the gun in hand as the context (the API's `entity_info` and `player_info`):
//! * from the player's own code (`ItemPreFrame`, `UpdateClientData`, `Drop`), the player being
//!   dispatched is lent to its slot for the hook ([`gun_call`]);
//! * from inside the gun (`ItemPostFrame` and the attacks, `Deploy`, `Holster`), through
//!   `shared::weapons::GunHost::virtual_pre` / `virtual_post`: the server's host lends the player,
//!   gun and all, for the hook ([`gun_pre`], [`gun_post`]).

use super::query::query;
use super::*;
use shared::weapons::Virtual;

/// The `Ham` number of a gun virtual.
pub fn ham_of(v: Virtual) -> usize {
    match v {
        Virtual::Deploy => id::ITEM_DEPLOY,
        Virtual::Holster => id::ITEM_HOLSTER,
        Virtual::PostFrame => id::ITEM_POSTFRAME,
        Virtual::PrimaryAttack => id::WEAPON_PRIMARYATTACK,
        Virtual::SecondaryAttack => id::WEAPON_SECONDARYATTACK,
        Virtual::AlternateAttack => id::TS_WEAPON_ALTERNATEATTACK,
        Virtual::Reload => id::WEAPON_RELOAD,
        Virtual::WeaponIdle => id::WEAPON_WEAPONIDLE,
        Virtual::ShouldWeaponIdle => id::WEAPON_SHOULDWEAPONIDLE,
    }
}

/// The player's `weapon_tsgun` entity (`SvState::gun_item`), null without one.
pub fn gun_item(p: &Player) -> *mut edict_t {
    p.inv.sv.gun_item.get()
}

/// A call of function `ham` on `this`, a gun entity of the player the caller's dispatch holds
/// (`p`, `cx`): the hooks run with the player lent to its slot ([`inner_on`]).
pub fn gun_call<A: Args, R: Ret>(
    ham: usize,
    this: *mut edict_t,
    p: &mut Player,
    cx: &mut Cx,
    args: A,
    run: impl FnOnce(&mut Player, &mut Cx, &mut A) -> R,
) -> R {
    if !HOOKS.0.armed(ham) || this.is_null() || world::type_of(this).is_none() {
        let mut args = args;
        return run(p, cx, &mut args);
    }
    inner_on(ham, this, p, cx, args, run)
}

/// `m_pActiveItem->ItemPreFrame()` (`CBasePlayer::ItemPreFrame` 0xdc484): the gun's is
/// `CBasePlayerItem::ItemPreFrame` (0xf3450), empty.
pub fn item_pre_frame(p: &mut Player, cx: &mut Cx) {
    let this = gun_item(p);
    gun_call(id::ITEM_PREFRAME, this, p, cx, (), |_, _, _| ());
}

/// `m_pActiveItem->UpdateClientData(this)` (`CTSGun::UpdateClientData` 0x858c8, returns 1):
/// `run` sends the gun's messages.
pub fn item_update_client_data(p: &mut Player, cx: &mut Cx, run: impl FnOnce(&mut Player, &mut Cx)) {
    let this = gun_item(p);
    let me = Ent(cx.edict);
    gun_call(id::ITEM_UPDATECLIENTDATA, this, p, cx, (me,), |p, cx, _| {
        run(p, cx);
        1
    });
}

/// `pItem->Drop()` (`CBasePlayerItem::Drop` 0xf1cac) of the player's gun entity `this` in
/// `RemoveAllItems`: `run` drops it.
pub fn item_drop(this: *mut edict_t, p: &mut Player, cx: &mut Cx, run: impl FnOnce()) {
    gun_call(id::ITEM_DROP, this, p, cx, (), |_, _, _| run());
}

/// `SendWeaponAnim(iAnim, skiplocal, body)` from the gun (`CTSGun::WeaponIdle`, `DefaultDeploy`,
/// ...): `run` sends it with the arguments the hook leaves. The player is inside its own weapon
/// frame, out of reach for the hook (the gun entity isn't).
pub fn send_weapon_anim(this: *mut edict_t, anim: i32, skip_local: bool, body: i32, run: impl FnOnce(i32, bool, i32)) {
    query(id::WEAPON_SENDWEAPONANIM, this, (anim, skip_local as i32, body), |(a, s, b)| run(*a, *s != 0, *b));
}

/// The hook before gun virtual `v` ran: what [`gun_post`] needs.
pub struct GunFrame(Option<Frame>);

impl GunFrame {
    /// The pre hook superseded the function.
    pub fn superseded(&self) -> bool {
        self.0.as_ref().is_some_and(Frame::superseded)
    }
}

/// The hook before gun virtual `v` on `this` (`GunHost::virtual_pre`); `arg` is `Holster`'s
/// `skiplocal` (its change is taken). `reach` runs the hook with the player reachable.
pub fn gun_pre(v: Virtual, this: *mut edict_t, arg: &mut i32, reach: &mut dyn FnMut(&mut dyn FnMut())) -> GunFrame {
    let ham = ham_of(v);
    if !HOOKS.0.armed(ham) || this.is_null() || world::type_of(this).is_none() {
        return GunFrame(None);
    }
    let h = &HOOKS.0;
    GunFrame(match v {
        Virtual::Holster => {
            let mut a = (*arg,);
            let f = h.pre(&Game, ham, this, kind::VOID, &mut a, reach);
            *arg = a.0;
            f
        }
        Virtual::Deploy | Virtual::ShouldWeaponIdle => h.pre(&Game, ham, this, kind::INT, &mut (), reach),
        _ => h.pre(&Game, ham, this, kind::VOID, &mut (), reach),
    })
}

/// The hook after it (`GunHost::virtual_post`): the value the gun goes on with.
pub fn gun_post(v: Virtual, frame: GunFrame, arg: i32, ran: Option<i32>, reach: &mut dyn FnMut(&mut dyn FnMut())) -> i32 {
    let Some(frame) = frame.0 else { return ran.unwrap_or(0) };
    let h = &HOOKS.0;
    match v {
        Virtual::Holster => {
            h.post::<_, ()>(&Game, frame, ran.map(|_| ()), &mut (arg,), reach);
            0
        }
        Virtual::Deploy | Virtual::ShouldWeaponIdle => h.post(&Game, frame, ran, &mut (), reach).unwrap_or(0),
        _ => {
            h.post::<_, ()>(&Game, frame, ran.map(|_| ()), &mut (), reach);
            0
        }
    }
}

/// The player holding gun entity `this` as its `weapon_tsgun`, for `ham_execute`.
pub(crate) fn holder(this: *mut edict_t) -> Option<*mut edict_t> {
    let o = this.v().owner;
    (crate::rules::is_player(o) && world::with_lent::<Player, _>(o, |p| gun_item(p) == this).unwrap_or(false)).then_some(o)
}
