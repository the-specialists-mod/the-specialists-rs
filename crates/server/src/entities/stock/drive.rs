//! The player's side of driving trains and tanks: the `CBasePlayer` members `m_iTrain`,
//! `PFLAG_ONTRAIN` (in `m_afPhysicsFlags`), `m_pTank` and `m_iHideHUD`, and the pieces of
//! `PreThink` (0x908cc), `PostThink` (0x91610), `PlayerUse` (0xd9848) and `UpdateClientData`
//! (0x91d00) that use them. TS keeps the SDK code for all of it.
//!
//! The entity side is in `tracktrain.rs` (`func_tracktrain`, `func_traincontrols`) and
//! `tank.rs` (`func_tank*`, `func_tankcontrols`).

use super::base::s;
use super::explode::emit_sound_slow;
use super::{tank, tracktrain};
use crate::player::{Player, with_player};
use crate::rules::msg;
use crate::world::{self, Cx, UseType};
use goldsrc::edict::EdictPtr;
use goldsrc::ext::entities::{ATTN_NORM, EHandle, vec};
use goldsrc::server::{IGNORE_MONSTERS, engine};
use goldsrc::sys::*;
use shared::protocol::Msg;
use shared::protocol::hud_wire::{HideWeapon, Train};

/// `m_iTrain` flags: the low nibble is the HUD notch (`tracktrain::train_speed`).
pub const TRAIN_ACTIVE: i32 = 0x80;
pub const TRAIN_NEW: i32 = 0xc0;
pub const TRAIN_OFF: i32 = 0x00;

/// `m_iHideHUD` bit set while a tank is in use.
pub const HIDEHUD_WEAPONS: i32 = 1;

/// Train and tank state kept on the player.
pub struct DriveState {
    /// `m_iTrain`.
    pub train: i32,
    /// `PFLAG_ONTRAIN`: the player is driving the train under them.
    pub on_train: bool,
    /// `PFLAG_USING`: set by `PlayerUse` while a continuous user is in use. Nothing in TS reads
    /// it and only `Spawn` clears it; kept for fidelity.
    pub using: bool,
    /// `m_pTank`.
    pub tank: EHandle,
    /// `m_iHideHUD` and the last value sent (`m_iClientHideHUD`, -1 = resend).
    pub hide_hud: i32,
    sent_hide_hud: i32,
}

impl Default for DriveState {
    /// `CBasePlayer::Precache` (0xda90c) starts with `m_iTrain = TRAIN_NEW`, so the first update
    /// clears the client's train HUD.
    fn default() -> Self {
        DriveState { train: TRAIN_NEW, on_train: false, using: false, tank: EHandle::NULL, hide_hud: 0, sent_hide_hud: -1 }
    }
}

/// `CBasePlayer::Spawn` (0x8f568): `m_afPhysicsFlags = 0`, `m_iClientHideHUD = -1`. Port: the
/// train HUD is also cleared (TS leaves the last notch up after dying on a train).
pub fn on_spawn(p: &mut Player, cx: &mut Cx) {
    let d = &mut p.drive;
    d.on_train = false;
    d.using = false;
    d.sent_hide_hud = -1;
    d.train = TRAIN_NEW | TRAIN_OFF;
    cx.pev().flags &= !(FL_ONTRAIN as i32);
}

/// The entity a player stands on, as `CBaseEntity::Instance(pev->groundentity)` finds it: the
/// world when there is none, null when the edict has no game object.
fn ground_instance(pv: &entvars_t) -> *mut edict_t {
    let g = if pv.groundentity.is_null() { engine().entity_of_index(0) } else { pv.groundentity };
    if g.is_null() || !g.has_private_data() { std::ptr::null_mut() } else { g }
}

/// `ObjectCaps() & FCAP_DIRECTIONAL_USE && OnControls(pev)`: only `func_tracktrain` has the cap.
fn train_controls(train: *mut edict_t, pv: &entvars_t) -> bool {
    s(train.v().classname) == c"func_tracktrain" && tracktrain::on_controls(train, pv)
}

/// `m_iTrain` for a train's current speed (`TrainSpeed(pev->speed, pev->impulse)`).
fn train_notch(train: *mut edict_t) -> i32 {
    let tv = train.v();
    tracktrain::train_speed(tv.speed as i32, tv.impulse)
}

/// The train and tank part at the top of `CBasePlayer::PlayerUse` (0xd9848). Returns true when
/// it handled this +use press and the usual search for a usable object must not run.
pub fn player_use(p: &mut Player, cx: &mut Cx, pressed: i32) -> bool {
    if pressed & IN_USE as i32 == 0 {
        return false;
    }
    let me = cx.edict;
    let tank = p.drive.tank.get();
    if !tank.is_null() {
        // Stop controlling the tank. The player is lent to its slot so `StopControl` redeploys
        // its gun inside the call, as in TS (`world::run_now_or_defer`).
        world::lend(me, p, || world::dispatch(tank, |ent, tcx| ent.use_(tcx, me, me, UseType::Off, 0.0)));
        p.drive.tank = EHandle::NULL;
        return true;
    }
    if p.drive.on_train {
        p.drive.on_train = false;
        p.drive.train = TRAIN_NEW | TRAIN_OFF;
        return true;
    }
    // Start controlling the train.
    let pv = cx.pev();
    let train = ground_instance(pv);
    if !train.is_null() && pv.button & IN_JUMP as i32 == 0 && pv.flags & FL_ONGROUND as i32 != 0 && train_controls(train, pv) {
        p.drive.on_train = true;
        p.drive.train = train_notch(train) | TRAIN_NEW;
        emit_sound_slow(me, CHAN_ITEM, c"plats/train_use1.wav", 0.8, ATTN_NORM, 0, 100, 0.5);
        return true;
    }
    false
}

/// The train block of `CBasePlayer::PreThink` (0x908cc): keep `FL_ONTRAIN` in step, drop the
/// controls when the player leaves them, jumps or strafes, and turn +forward / +back presses into
/// speed changes. `pressed` is this frame's newly pressed buttons.
pub fn pre_think(p: &mut Player, cx: &mut Cx, pressed: i32) {
    let me = cx.edict;
    let pv = cx.pev();
    // PreThink returns before this for observers and the dying.
    if pv.iuser1 != 0 || pv.deadflag > DEAD_NO as i32 {
        return;
    }
    let d = &mut p.drive;
    if d.on_train {
        pv.flags |= FL_ONTRAIN as i32;
    } else {
        pv.flags &= !(FL_ONTRAIN as i32);
    }
    if !d.on_train {
        if d.train & TRAIN_ACTIVE != 0 {
            d.train = TRAIN_NEW; // turn off train
        }
        return;
    }
    let mut train = ground_instance(pv);
    let keep = if train.is_null() {
        // Maybe this is on the other side of a level transition.
        let end = vec::add(pv.origin, [0.0, 0.0, -38.0]);
        let tr = engine().trace_line(pv.origin, end, IGNORE_MONSTERS, me);
        train = if tr.flFraction != 1.0 && !tr.pHit.is_null() && tr.pHit.has_private_data() { tr.pHit } else { std::ptr::null_mut() };
        !train.is_null() && train_controls(train, pv)
    } else {
        // Turn off the train if you jump, strafe, or the train controls go dead.
        pv.flags & FL_ONGROUND as i32 != 0
            && train.v().spawnflags & tracktrain::SF_TRACKTRAIN_NOCONTROL == 0
            && pv.button & (IN_MOVELEFT | IN_MOVERIGHT) as i32 == 0
    };
    if !keep {
        d.on_train = false;
        d.train = TRAIN_NEW | TRAIN_OFF;
        return;
    }
    pv.velocity = [0.0; 3];
    let vel = if pressed & IN_FORWARD as i32 != 0 {
        1.0
    } else if pressed & IN_BACK as i32 != 0 {
        -1.0
    } else {
        0.0
    };
    if vel != 0.0 {
        world::dispatch(train, |ent, tcx| ent.use_(tcx, me, me, UseType::Set, vel));
        p.drive.train = train_notch(train) | TRAIN_ACTIVE | TRAIN_NEW;
    }
}

/// The tank block of `CBasePlayer::PostThink` (0x91610): fire the tank while still on its
/// controls with no weapon model; otherwise let go of it.
pub fn post_think(p: &mut Player, cx: &mut Cx) {
    let tank = p.drive.tank.get();
    if tank.is_null() {
        return;
    }
    let me = cx.edict;
    let pv = cx.pev();
    // Lent to its slot meanwhile, as in `player_use`: `StopControl` redeploys the gun inside the
    // call, as in TS.
    if tank::on_controls(tank, pv) && pv.weaponmodel == 0 {
        world::lend(me, p, || world::dispatch(tank, |ent, tcx| ent.use_(tcx, me, me, UseType::Set, 2.0)));
    } else {
        world::lend(me, p, || world::dispatch(tank, |ent, tcx| ent.use_(tcx, me, me, UseType::Off, 0.0)));
        p.drive.tank = EHandle::NULL;
    }
}

/// `Killed` (0x8ed5c) and `StartObserver` (0x99634): let go of the tank.
pub fn release_tank(p: &mut Player, cx: &mut Cx) {
    let tank = p.drive.tank.get();
    if tank.is_null() {
        return;
    }
    let me = cx.edict;
    // Lent, so `StopControl` redeploys the gun here (before `StartObserver`'s `RemoveAllItems`), as
    // in TS.
    world::lend(me, p, || world::dispatch(tank, |ent, tcx| ent.use_(tcx, me, me, UseType::Off, 0.0)));
    p.drive.tank = EHandle::NULL;
}

/// The player half of `CFuncTank::Use` + `StartControl` (0xc2db8, 0xc2a24), run once the
/// player is back in its slot: `m_pTank = tank` (even when the tank refused), and on success
/// holster the gun, clear the weapon and view models and hide the weapon HUD.
pub fn tank_started(player: *mut edict_t, tank: *mut edict_t, started: bool) {
    with_player(player, |p, cx| {
        p.drive.tank = EHandle::new(tank);
        if started {
            crate::weapons::holster_for_tank(p, cx);
            let v = cx.pev();
            v.weaponmodel = 0;
            v.viewmodel = 0;
            p.drive.hide_hud |= HIDEHUD_WEAPONS;
        }
    });
}

/// The player half of `CFuncTank::StopControl` (0xc2b1c): redeploy the gun and show the weapon
/// HUD again.
pub fn tank_stopped(player: *mut edict_t) {
    if !goldsrc::ext::entities::is_player(player) {
        return; // disconnected while in control
    }
    with_player(player, |p, cx| {
        crate::weapons::deploy_after_tank(p, cx);
        p.drive.hide_hud &= !HIDEHUD_WEAPONS;
    });
}

/// The `HideWeapon` part of `CBasePlayer::UpdateClientData` (0x91d00), before `TSHealth`.
pub fn update_client_hide_weapon(p: &mut Player, cx: &mut Cx) {
    let d = &mut p.drive;
    if d.hide_hud != d.sent_hide_hud {
        let flags = d.hide_hud as u8;
        msg::one(Msg::HideWeapon, cx.edict, |w| HideWeapon { flags }.write(w));
        d.sent_hide_hud = d.hide_hud;
    }
}

/// The `Train` part of `CBasePlayer::UpdateClientData` (0x91d00), after `TSArmor`.
pub fn update_client_train(p: &mut Player, cx: &mut Cx) {
    let e = cx.edict;
    let d = &mut p.drive;
    if d.train & TRAIN_NEW != 0 {
        let pos = (d.train & 0xf) as u8;
        msg::one(Msg::Train, e, |w| Train { pos }.write(w));
        d.train &= !TRAIN_NEW;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn train_flags() {
        let d = DriveState::default();
        assert_eq!(d.train & TRAIN_NEW, TRAIN_NEW, "first update clears the client HUD");
        assert_eq!(d.sent_hide_hud, -1);
        // TRAIN_NEW includes TRAIN_ACTIVE, and sending clears both (UpdateClientData `& ~0xc0`).
        assert_eq!(TRAIN_NEW & TRAIN_ACTIVE, TRAIN_ACTIVE);
        assert_eq!((tracktrain::train_speed(10, 100) | TRAIN_NEW) & !TRAIN_NEW, 2);
    }
}
