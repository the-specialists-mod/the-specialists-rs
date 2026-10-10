//! Local weapon prediction (`HUD_WeaponsPostThink` `0x10003830`, client `CTSGun`).
//!
//! The client runs the same `shared::weapons::Gun` state machine as the server, through a client
//! [`GunHost`]: the networked state (`clientdata` `vuser*`/`ammo_*`/`m_flNextAttack` and
//! `weapon_data[16]`) is unpacked with `shared::protocol::weapons`, the gun runs one usercmd
//! (`pre_frame`, `item_post_frame`, `decrement_timers`, as in the server's PreThink/PostThink)
//! and the result is packed into `to`.
//!
//! The server plays every weapon event with `FEV_NOTHOST`, so the shooter only sees and hears
//! its own shots through these predicted events (`runfuncs` only). Traces, damage, projectiles
//! and server sounds are server-only; the `fire` event re-traces the pellets with the same seed.

use super::TS_GUN_SLOT;
use crate::{state, view};
use goldsrc::Global;
use goldsrc::ext::render as r;
use goldsrc::sys::*;
use shared::protocol::weapons::{pack_clientdata, pack_weapon_data, unpack_clientdata, unpack_weapon_data};
use shared::weapons::gun::{MeleeHit, MeleeResult, Shot, Sound};
use shared::weapons::spread::Vec3;
use shared::weapons::{Event, EventArgs, Gun, GunHost, MAX_AMMO, MAX_WEAPONS, Status, anim, att, body_by_items, info, models};
use std::cell::RefCell;

#[derive(Default)]
struct Pred {
    /// Event indexes in `Event::ALL` order (0 = not precached yet).
    events: [u16; 11],
    /// View model indexes per weapon id (0 = not looked up).
    v_models: Vec<i32>,
    /// State that isn't networked (recoil penalty, last shot time, kung-fu bar, combos, mode
    /// memory), carried from one first-time prediction to the next.
    gun: Option<Gun>,
    last_id: Option<u8>,
    alt_grip: bool,
    was_dead: bool,
}

static PRED: Global<RefCell<Option<Pred>>> = Global(RefCell::new(None));

fn with<R>(f: impl FnOnce(&mut Pred) -> R) -> R {
    let mut g = PRED.0.borrow_mut();
    f(g.get_or_insert_with(Pred::default))
}

/// Alternate grip (anim set B) of the local weapon. Server sends it in `vuser2[2]`.
pub fn alt_grip() -> bool {
    PRED.0.try_borrow().ok().and_then(|g| g.as_ref().map(|p| p.alt_grip)).unwrap_or(false)
}

fn event_index(ev: Event) -> u16 {
    let i = Event::ALL.iter().position(|&e| e == ev).unwrap_or(0);
    let cached = with(|p| p.events[i]);
    if cached != 0 {
        return cached;
    }
    let idx = r::precache_event(ev.path());
    with(|p| p.events[i] = idx);
    idx
}

fn view_model_index(id: u8) -> i32 {
    let id = (id as usize).min(MAX_WEAPONS - 1);
    let cached = with(|p| p.v_models.get(id).copied().unwrap_or(0));
    if cached != 0 {
        return cached;
    }
    let idx = r::model_index(&models(id as u8).0);
    with(|p| {
        p.v_models.resize(MAX_WEAPONS, 0);
        p.v_models[id] = idx;
    });
    idx
}

/// The client side of the gun's side effects for one usercmd.
struct ClientHost<'a> {
    from: &'a local_state_t,
    cmd: &'a usercmd_t,
    runfuncs: bool,
    time: f32,
    seed: u32,
    powerups: u16,
    /// Predicted results written back into `to`.
    fov: f32,
    viewmodel: i32,
    iuser4: i32,
    /// "Silenced" for the current shot's view shake (`VKickback` passes the multiplier only).
    silenced: bool,
}

impl GunHost for ClientHost<'_> {
    fn time(&self) -> f32 {
        self.time
    }
    fn slow_mul(&self) -> f32 {
        self.from.client.fuser1.max(0.0)
    }
    fn powerups(&self) -> u16 {
        self.powerups
    }
    fn kungfu(&self) -> bool {
        false // only scales melee damage (server)
    }
    fn alive(&self) -> bool {
        let c = &self.from.client;
        c.deadflag == 0 && c.health > 0.0
    }
    fn buttons(&self) -> u32 {
        self.cmd.buttons as u32
    }
    fn pressed(&self) -> u32 {
        self.cmd.buttons as u32 & !(self.from.playerstate.oldbuttons as u32)
    }
    fn origin(&self) -> Vec3 {
        self.from.client.origin
    }
    fn view_ofs(&self) -> Vec3 {
        self.from.client.view_ofs
    }
    fn velocity(&self) -> Vec3 {
        self.from.client.velocity
    }
    fn v_angle(&self) -> Vec3 {
        self.cmd.viewangles
    }
    fn flags(&self) -> i32 {
        self.from.client.flags
    }
    fn waterlevel(&self) -> i32 {
        self.from.client.waterlevel
    }
    fn iuser4(&self) -> i32 {
        self.iuser4
    }
    fn set_iuser4(&mut self, v: i32) {
        self.iuser4 = v;
    }
    fn seed(&self) -> u32 {
        self.seed
    }
    fn random_long(&mut self, lo: i32, hi: i32) -> i32 {
        r::random_long(lo, hi)
    }
    fn fov(&self) -> f32 {
        // As the server: 90 (the spawn default) means "not zoomed".
        if self.fov >= 90.0 { 0.0 } else { self.fov }
    }
    fn set_fov(&mut self, fov: f32) {
        self.fov = fov;
    }
    fn set_view_model(&mut self, id: Option<u8>) {
        self.viewmodel = id.map(view_model_index).unwrap_or(0);
    }
    fn view_model_hidden(&self) -> bool {
        self.viewmodel == 0
    }
    fn set_weapon_model(&mut self, _model: Option<(u8, bool)>) {}
    fn event(&mut self, ev: Event, a: EventArgs) {
        if self.runfuncs {
            r::playback_event(FEV_NOTHOST as i32, event_index(ev), a.f1, a.f2, a.i1, a.i2, a.b1 != 0, a.b2);
        }
    }
    fn weapon_anim(&mut self, anim: i32, _skip_local: bool, body: i32) {
        // HUD_SendWeaponAnim: the local view model, first prediction only.
        if self.runfuncs {
            r::weapon_animation(anim, body);
        }
    }
    fn player_anim(&mut self, _anim: i32) {}
    fn kick(&mut self, pitch: f32, yaw: f32) {
        // VKickback: once per predicted shot.
        if self.runfuncs {
            view::kick_angles(pitch, yaw, self.silenced);
        }
    }
    fn sound(&mut self, _s: Sound) {}
    fn shoot(&mut self, _shot: &Shot) {}
    fn melee(&mut self, _hit: &MeleeHit) -> MeleeResult {
        MeleeResult::Miss // hits are resolved by the server
    }
    fn throw_grenade(&mut self, _origin: Vec3, _velocity: Vec3, _roll: bool) {}
    fn throw_blade(&mut self, _btype: i32) {}
    fn slots_changed(&mut self, _delta_free: i32) {}
    fn melee_bar_changed(&mut self, _bar: f32) {}
    fn block_player(&mut self, _t: f32) {
        self.iuser4 |= 1;
    }
}

/// Builds the gun for this command: non-networked state from the last first-time prediction,
/// everything networked from `from`. Only the current weapon's record is known on the client.
fn gun_from(from: &local_state_t) -> (Gun, u16) {
    let mut g = with(|p| p.gun.clone()).unwrap_or_default();
    g.status = [Status::default(); MAX_WEAPONS];
    g.reserve = [0; MAX_AMMO];
    let powerups = unpack_clientdata(&from.client, &mut g);
    unpack_weapon_data(&from.weapondata[TS_GUN_SLOT], &mut g);
    g.next_alt = from.weapondata[TS_GUN_SLOT].fuser4;
    (g, powerups)
}

pub fn post_think(from: &local_state_t, to: &mut local_state_t, cmd: &usercmd_t, runfuncs: bool, time: f64, seed: u32) {
    let c = &from.client;
    let dead = c.deadflag != 0 || c.health <= 0.0;
    let (mut gun, powerups) = gun_from(from);
    // HUD_PostRunCmd (0x10003830) keeps the active items for DrawLaserDots, cleared while the
    // next attack is pending (TS `0x1018a628`).
    let ready = if c.m_flNextAttack > 0.0 { 0 } else { gun.slot().map_or(0, |s| s.active as i32) };
    state::with(|st| st.items_ready = ready);

    // Weapon switches come from the server (`weapon_<n>`); show the draw animation when the
    // networked weapon changes (Deploy / HolsterWeapon view animation).
    if runfuncs {
        let id = gun.current;
        let items = gun.slot().map(|s| s.items).unwrap_or(0);
        let changed = with(|p| {
            p.alt_grip = gun.slot().is_some_and(|s| s.alt_grip);
            let ch = p.last_id != Some(id) || (p.was_dead && !dead);
            p.last_id = Some(id);
            p.was_dead = dead;
            ch
        });
        if changed && !dead && c.m_iId == TS_GUN_SLOT as i32 {
            r::weapon_animation(anim(id, false, 2), if id == 0 { 0 } else { body_by_items(id, items) });
        }
    }

    let silenced = gun.slot().is_some_and(|s| s.items & att::SILENCER != 0) && info(gun.current).is_some_and(|w| w.att_default & 1 == 0);
    let mut host = ClientHost {
        from,
        cmd,
        runfuncs,
        time: time as f32,
        seed,
        powerups,
        fov: c.fov,
        viewmodel: c.viewmodel,
        iuser4: c.iuser4,
        silenced,
    };
    // Same order as the server: PreThink (windows, penalty decay, bar), PostThink
    // (ItemPostFrame while alive, then the timers).
    let dt = Gun::timer_dt(cmd.msec as f32 * 0.001, c.fuser1.max(0.0), powerups);
    gun.pre_frame(dt, !dead);
    if !dead && c.m_iId == TS_GUN_SLOT as i32 {
        gun.item_post_frame(&mut host);
    }
    gun.decrement_timers(dt);

    let (fov, viewmodel, iuser4) = (host.fov, host.viewmodel, host.iuser4);
    pack_clientdata(&gun, powerups, &mut to.client);
    pack_weapon_data(&gun, &mut to.weapondata[TS_GUN_SLOT]);
    to.weapondata[TS_GUN_SLOT].fuser4 = gun.next_alt;
    to.weapondata[TS_GUN_SLOT].m_fInReload = gun.slot().map(|s| s.in_reload).unwrap_or(0);
    to.client.fov = fov;
    to.client.viewmodel = viewmodel;
    to.client.iuser4 = iuser4;
    to.client.weaponanim = from.client.weaponanim;
    to.client.maxspeed = from.client.maxspeed;
    to.client.fuser1 = from.client.fuser1;

    if runfuncs {
        let s = gun.slot().copied().unwrap_or_default();
        state::with(|st| {
            st.powerups = powerups as i32;
            st.attachments = s.items as i32;
            st.local_items = s.active as i32;
            st.firemode = s.mode as i32;
            st.clip = s.clip;
            st.reserve_ammo = if gun.current == 0 { 0 } else { gun.current_reserve() };
            st.spread = gun.spread_penalty;
            st.predicted_weapon = (!dead).then_some(gun.current as i32);
        });
        with(|p| p.gun = Some(gun));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::weapons::id;

    /// Server pack -> client unpack keeps what the predictor needs.
    #[test]
    fn networked_round_trip() {
        let mut server = Gun::default();
        server.current = id::AK47;
        let s = &mut server.status[id::AK47 as usize];
        s.owned = true;
        s.clip = 21;
        s.mode = 1;
        s.items = att::SCOPE;
        s.alt_grip = true;
        server.reserve[info(id::AK47).unwrap().ammo as usize] = 60;
        server.next_primary = 0.05;
        server.next_attack = 0.3;
        let mut ls = local_state_t::default();
        pack_clientdata(&server, 0x10, &mut ls.client);
        pack_weapon_data(&server, &mut ls.weapondata[TS_GUN_SLOT]);
        let (g, powerups) = gun_from(&ls);
        assert_eq!(powerups, 0x10);
        assert_eq!(g.current, id::AK47);
        let c = g.slot().unwrap();
        assert_eq!((c.clip, c.mode, c.items, c.alt_grip), (21, 1, att::SCOPE, true));
        assert_eq!(g.current_reserve(), 60);
        assert_eq!((g.next_primary, g.next_attack), (0.05, 0.3));
    }
}
