//! Local weapon prediction (`HUD_WeaponsPostThink` `0x10003830`, client `CTSGun`).
//!
//! **Stopgap.** The weapons package writes the shared TS gun state machine in
//! `shared::weapons` (one container, 38 records, fire modes, timers x fuser1, spread penalty);
//! once merged, `post_think` should run that instead of `predict_fire` below. What stays here is
//! the glue: decoding the networked state (`vuser*`, `weapon_data[16]`), the TS timer decrement,
//! writing `to`, local event playback and the client-side kick.
//!
//! Without local prediction the shooter would see nothing: the server plays every weapon event
//! with `FEV_NOTHOST`.

use super::TS_GUN_SLOT;
use crate::events::{fire::weapon_body, info};
use crate::{state, view};
use goldsrc::Global;
use goldsrc::ext::render::{self as r, vec3 as v};
use goldsrc::sys::*;
use shared::weapons::{FireMode, WeaponKind, tables};
use std::cell::RefCell;

/// TS weapon state as networked in `clientdata` (server `UpdateClientData`, architecture.md §7.2).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GunState {
    pub id: i32,
    /// 0 auto, 1 semi, 2 burst, 3 pump, 4 akimbo semi, 5 akimbo auto.
    pub firemode: i32,
    /// Owned attachments (1 silencer, 2 laser, 4 flashlight, 8 scope).
    pub attachments: i32,
    /// Switched-on attachments (laser/flashlight beams).
    pub active_items: i32,
    pub powerups: i32,
    /// Burst shots left / pump pending / grenade phase (`slot+8`).
    pub counter: i32,
    pub reserve: i32,
    pub alt_grip: bool,
    pub hand: i32,
    pub clip: i32,
    pub in_reload: bool,
}

impl GunState {
    pub fn from_clientdata(c: &clientdata_t) -> GunState {
        let i = |x: f32| x as i32;
        GunState {
            id: i(c.vuser3[0]),
            firemode: i(c.vuser3[1]),
            attachments: i(c.vuser3[2]),
            active_items: i(c.vuser1[2]),
            powerups: i(c.vuser1[1]),
            counter: i(c.vuser2[0]),
            reserve: i(c.vuser2[1]),
            alt_grip: c.vuser2[2] != 0.0,
            hand: i(c.vuser4[0]),
            clip: i(c.vuser4[1]),
            in_reload: c.vuser4[2] != 0.0,
        }
    }
    pub fn write(&self, c: &mut clientdata_t) {
        c.vuser3 = [self.id as f32, self.firemode as f32, self.attachments as f32];
        c.vuser1[1] = self.powerups as f32;
        c.vuser1[2] = self.active_items as f32;
        c.vuser2 = [self.counter as f32, self.reserve as f32, if self.alt_grip { 1.0 } else { 0.0 }];
        c.vuser4 = [self.hand as f32, self.clip as f32, if self.in_reload { 1.0 } else { 0.0 }];
    }
    pub fn infinite_clip(&self) -> bool {
        self.powerups & shared::ts::powerup::INFINITE_CLIP as i32 != 0
    }
    pub fn double_rate(&self) -> bool {
        self.powerups & shared::ts::powerup::DOUBLE_FIRERATE as i32 != 0
    }
}

#[derive(Default)]
struct Pred {
    events: Option<Events>,
    last_id: i32,
    alt_grip: bool,
    /// Client-only accuracy penalty (`0x1009ed8c`), decays by dt*0.1.
    spread_penalty: f32,
    last_shot: f64,
    was_dead: bool,
}

#[derive(Clone, Copy)]
struct Events {
    fire: u16,
    empty: u16,
    reload: u16,
    pump: u16,
    knife: u16,
    katana: u16,
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

fn events() -> Events {
    if let Some(e) = with(|p| p.events) {
        return e;
    }
    let e = Events {
        fire: r::precache_event(c"events/fire.sc"),
        empty: r::precache_event(c"events/empty.sc"),
        reload: r::precache_event(c"events/reload.sc"),
        pump: r::precache_event(c"events/pump.sc"),
        knife: r::precache_event(c"events/knife.sc"),
        katana: r::precache_event(c"events/katana.sc"),
    };
    with(|p| p.events = Some(e));
    e
}

/// The `weapon_data_t` timers TS uses (slot 16).
#[derive(Clone, Copy, Debug, Default)]
struct Timers {
    next_primary: f32,
    next_secondary: f32,
    idle: f32,
    next_alt: f32,
}

pub fn post_think(from: &local_state_t, to: &mut local_state_t, cmd: &usercmd_t, runfuncs: bool, time: f64, seed: u32) {
    let c = &from.client;
    let mut g = GunState::from_clientdata(c);
    let wd = &from.weapondata[TS_GUN_SLOT];
    let mut t = Timers { next_primary: wd.m_flNextPrimaryAttack, next_secondary: wd.m_flNextSecondaryAttack, idle: wd.m_flTimeWeaponIdle, next_alt: wd.fuser4 };
    let mut next_attack = c.m_flNextAttack;
    let dead = c.deadflag != 0 || c.health <= 0.0;
    let ts = c.fuser1;
    let ts = if ts <= 0.0 { 1.0 } else { ts };
    let dt = cmd.msec as f32 * 0.001 * ts.max(0.01) * if g.double_rate() { 1.5 } else { 1.0 };

    if runfuncs {
        state::with(|s| {
            s.powerups = g.powerups;
            s.attachments = g.attachments;
            s.active_items = g.active_items;
            s.firemode = g.firemode;
            s.clip = g.clip;
            s.reserve_ammo = g.reserve;
        });
        let changed = with(|p| {
            p.alt_grip = g.alt_grip;
            let ch = p.last_id != g.id || (p.was_dead && !dead);
            p.last_id = g.id;
            p.was_dead = dead;
            ch
        });
        if changed && !dead {
            // SelectWeapon + Deploy: draw anim.
            if let Some(i) = info::info(g.id) {
                r::weapon_animation(i.set(g.alt_grip)[2] as i32, weapon_body(g.id, g.attachments));
            }
        }
    }

    if !dead && next_attack <= 0.0 && c.m_iId == TS_GUN_SLOT as i32 {
        let pressed = cmd.buttons & IN_ATTACK as u16 != 0;
        let just_pressed = pressed && from.playerstate.oldbuttons & IN_ATTACK as i32 == 0;
        if g.in_reload {
            // ItemPostFrame: complete a magazine reload once m_flNextAttack expired.
            if let Some(w) = tables().weapon(g.id as u8) {
                let n = (w.clip - g.clip).min(g.reserve).max(0);
                g.clip += n;
                g.reserve -= n;
            }
            g.in_reload = false;
        }
        if pressed && t.next_primary <= 0.0 {
            predict_fire(&mut g, &mut t, c, cmd, just_pressed, runfuncs, seed);
        } else if g.firemode == 2 && g.counter > 0 && t.next_primary <= 0.0 {
            // A running burst keeps firing without the button.
            predict_fire(&mut g, &mut t, c, cmd, false, runfuncs, seed);
        } else if cmd.buttons & IN_RELOAD as u16 != 0 && !g.in_reload {
            predict_reload(&mut g, &mut next_attack, runfuncs);
        }
    }

    // Timer decrement: msec * 0.001 * max(fuser1, 0.01), x1.5 with Double Fire Rate.
    t.next_primary = (t.next_primary - dt).max(-1.0);
    t.next_secondary = (t.next_secondary - dt).max(-0.001);
    t.idle = (t.idle - dt).max(-0.001);
    t.next_alt = (t.next_alt - dt).max(-0.001);
    next_attack = (next_attack - dt).max(-0.001);
    with(|p| p.spread_penalty = (p.spread_penalty - dt * 0.1).max(0.0));
    let _ = time;

    g.write(&mut to.client);
    to.client.m_flNextAttack = next_attack;
    to.client.fov = from.client.fov;
    to.client.weaponanim = from.client.weaponanim;
    to.client.viewmodel = from.client.viewmodel;
    to.client.maxspeed = from.client.maxspeed;
    to.client.iuser4 = from.client.iuser4;
    to.client.fuser1 = from.client.fuser1;
    let tw = &mut to.weapondata[TS_GUN_SLOT];
    tw.m_flNextPrimaryAttack = t.next_primary;
    tw.m_flNextSecondaryAttack = t.next_secondary;
    tw.m_flTimeWeaponIdle = t.idle;
    tw.fuser4 = t.next_alt;
    tw.m_iClip = g.clip;
    tw.m_fInReload = g.in_reload as i32;
}

/// Spread for a shot (`ScaleSpread` client half): accuracy + accumulated penalty.
fn spread_for(w: &shared::weapons::WeaponInfo, g: &GunState, c: &clientdata_t, silenced: bool) -> f32 {
    let speed = v::len(c.velocity);
    let combat = &tables().combat;
    let mut spread = w.accuracy;
    if g.id == 18 && speed > 35.0 {
        with(|p| p.spread_penalty = 0.3);
    }
    let akick = if g.id == 32 && speed > 175.0 { w.acc_kick * 5.0 } else { w.acc_kick };
    if w.scale_spread || akick != w.acc_kick {
        if silenced {
            spread *= combat.silencer.spread;
        }
        if akick > 0.0 {
            let k = if g.double_rate() { 1.0 } else { 2.0 } * if silenced { 0.775 } else { 1.0 };
            let now = r::client_time() as f64;
            with(|p| {
                let dtshot = (now - p.last_shot) as f32;
                if dtshot >= 0.001 {
                    spread += p.spread_penalty;
                    let mut x = dtshot * c.fuser1.max(0.01);
                    if x >= 1.0 {
                        x = 0.5;
                    }
                    p.spread_penalty = ((1.0 - x) * akick * k + p.spread_penalty).min(combat.spread_penalty.max);
                }
                p.last_shot = now;
            });
        }
    }
    spread
}

fn predict_fire(g: &mut GunState, t: &mut Timers, c: &clientdata_t, cmd: &usercmd_t, just_pressed: bool, runfuncs: bool, seed: u32) {
    let ev = events();
    let Some(w) = tables().weapon(g.id as u8) else { return };
    let play = |idx: u16, f: f32, i1: i32, i2: i32, b1: bool, b2: i32| {
        if runfuncs {
            r::playback_event(FEV_NOTHOST as i32, idx, f, f, i1, i2, b1, b2);
        }
    };
    match w.kind {
        WeaponKind::Kungfu | WeaponKind::Grenade => return, // server-driven (not predicted here)
        WeaponKind::Knife => {
            play(ev.knife, 0.0, g.id, 0, false, 0);
            t.next_primary = w.rate;
            return;
        }
        WeaponKind::Katana => {
            play(ev.katana, 0.0, 16, 0, false, 0);
            t.next_primary = w.rate;
            return;
        }
        _ => {}
    }
    let semi = matches!(g.firemode, 1 | 4);
    if semi && !just_pressed && !g.double_rate() {
        return;
    }
    if g.firemode == 2 && g.counter == 0 {
        if !just_pressed {
            return;
        }
        g.counter = if g.id == 23 { 2 } else { tables().combat.burst.shots };
    }
    if c.waterlevel == 3 || (g.clip <= 0 && !g.infinite_clip()) {
        if just_pressed || !semi {
            play(ev.empty, 0.0, 0, 0, false, 0);
        }
        g.counter = 0;
        t.next_primary = 0.1;
        return;
    }
    if g.firemode == 3 {
        // Pump: every other click pumps instead of firing.
        if g.counter != 0 {
            g.counter = 0;
            play(ev.pump, 0.0, g.id, g.attachments, true, 0);
            t.next_primary = w.rate;
            return;
        }
        g.counter = 1;
    }
    // "Silenced" = silencer on and not integral (MP5SD, Ruger: `info+0xe4` & 1).
    let silenced = g.attachments & 1 != 0 && !matches!(g.id, 6 | 28);
    let spread = spread_for(w, g, c, silenced);
    if !g.infinite_clip() {
        g.clip -= 1;
    }
    let class = info::class(g.id);
    let b1 = if matches!(class, 4 | 5) {
        g.hand ^= 1;
        g.hand != 0
    } else if class == 1 {
        g.firemode == 3
    } else {
        g.clip == 0
    };
    play(ev.fire, spread, g.id, g.attachments, b1, seed as i32);
    if runfuncs && (w.kick || matches!(class, 7) || g.id == 31 || g.id == 33) {
        view::kick(w.view_kick, class, silenced, g.double_rate());
    }
    let combat = &tables().combat;
    t.next_primary = if g.firemode == 2 {
        g.counter -= 1;
        if g.counter > 0 { combat.burst.gap } else { combat.burst.end }
    } else if semi && w.modes.contains(&FireMode::Auto) {
        combat.semi_delay
    } else {
        w.rate
    };
    t.idle = 2.0;
    let _ = cmd;
}

fn predict_reload(g: &mut GunState, next_attack: &mut f32, runfuncs: bool) {
    let Some(w) = tables().weapon(g.id as u8) else { return };
    if g.reserve <= 0 || g.clip >= w.clip || g.counter != 0 || w.kind == WeaponKind::Shotgun || g.id == 36 {
        // Shotgun reloads are a 3-phase server state machine; left to the server (see doc).
        return;
    }
    if !matches!(w.kind, WeaponKind::Pistol | WeaponKind::Smg | WeaponKind::Rifle | WeaponKind::Sniper | WeaponKind::Akimbo | WeaponKind::AkimboOnly) {
        return;
    }
    g.in_reload = true;
    *next_attack = w.reload;
    if runfuncs {
        let ev = events();
        r::playback_event(FEV_NOTHOST as i32, ev.reload, 0.0, 0.0, g.id, g.attachments, false, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clientdata_roundtrip() {
        let mut c = clientdata_t::default();
        let g = GunState { id: 13, firemode: 1, attachments: 9, active_items: 2, powerups: 0x10, counter: 2, reserve: 90, alt_grip: true, hand: 1, clip: 30, in_reload: true };
        g.write(&mut c);
        assert_eq!(GunState::from_clientdata(&c), g);
        assert!(g.double_rate());
        assert!(!g.infinite_clip());
    }
}
