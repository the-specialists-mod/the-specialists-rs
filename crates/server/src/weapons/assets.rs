//! Precache (`CTSGun::Precache` events, `TSPrecache` weapon models and sounds), per-map model
//! strings, cvars and the weapon restriction mask.

use goldsrc::Global;
use goldsrc::ext::weapons::WeaponsEngine;
use goldsrc::server::{cvar_value, engine, intern};
use goldsrc::sys::*;
use shared::weapons::{Event, MAX_WEAPONS, models, tables};
use std::cell::{Cell, RefCell};
use std::ffi::CStr;

pub struct Cvars {
    /// `realbullet`: simulated bullets (`CBulletManager`) for everyone; 2+ = per-frame steps.
    pub realbullet: &'static cvar_t,
    /// `ammocount`: max-carry column (0 default, 1..3 reduced).
    pub ammocount: &'static cvar_t,
    /// `weaponstay`: dropped weapon lifetime (clamped 5..60 s; drops of the held gun 10..120 s
    /// at twice the value).
    pub weaponstay: &'static cvar_t,
    /// `damagemult`: weapon damage multiplier, only with `sv_cheats`.
    pub damagemult: &'static cvar_t,
    /// `customweapons`: weapon parameter file (not implemented, see docs).
    pub customweapons: &'static cvar_t,
    /// Not TS: `ts_weapons_debug 1` logs fires, hits, explosions, drops to the server console.
    pub debug: &'static cvar_t,
}

static CVARS: Global<RefCell<Option<Cvars>>> = Global(RefCell::new(None));
/// `TSGameRules+0x5870` weapon restriction mask (default 0x1f = everything allowed).
static RESTRICTION: Global<Cell<u32>> = Global(Cell::new(0x1f));
static GRENADE_BUYS: Global<Cell<bool>> = Global(Cell::new(false));

pub fn register_cvars() {
    let e = engine();
    const FCVAR_SERVER: i32 = 4;
    let c = Cvars {
        realbullet: e.register_cvar("realbullet", "0", FCVAR_SERVER),
        ammocount: e.register_cvar("ammocount", "0", FCVAR_SERVER),
        weaponstay: e.register_cvar("weaponstay", "15", FCVAR_SERVER),
        damagemult: e.register_cvar("damagemult", "0", FCVAR_SERVER),
        customweapons: e.register_cvar("customweapons", "", FCVAR_SERVER),
        debug: e.register_cvar("ts_weapons_debug", "0", 0),
    };
    *CVARS.0.borrow_mut() = Some(c);
    e.add_server_command(c"weaponrestriction", cmd_weaponrestriction);
    e.add_server_command(c"loadweapons", super::custom::cmd_loadweapons);
    e.add_server_command(c"saveweapons", super::custom::cmd_saveweapons);
}

/// Current cvar values.
pub struct CvarValues {
    pub realbullet: f32,
    pub ammocount: f32,
    pub weaponstay: f32,
    pub damagemult: f32,
}

pub fn cvars() -> CvarValues {
    let c = CVARS.0.borrow();
    match c.as_ref() {
        Some(c) => CvarValues {
            realbullet: cvar_value(c.realbullet),
            ammocount: cvar_value(c.ammocount),
            weaponstay: cvar_value(c.weaponstay),
            damagemult: cvar_value(c.damagemult),
        },
        None => CvarValues { realbullet: 0.0, ammocount: 0.0, weaponstay: 15.0, damagemult: 0.0 },
    }
}

/// `damagemult` applies only when weapon cheats (`sv_cheats`) are on.
pub fn damage_adjust() -> f32 {
    let m = cvars().damagemult;
    if m > 0.0 && engine().cvar_float(c"sv_cheats") != 0.0 { m } else { 1.0 }
}

/// Max carry of ammo type `t` for the current `ammocount` (`Set_ammo_count`).
pub fn max_carry(t: u8) -> i32 {
    let col = (cvars().ammocount as i32).clamp(0, 3) as usize;
    tables().ammo(t).map(|a| a.max_carry[col]).unwrap_or(0)
}

/// Logs `f()` when `ts_weapons_debug` is set.
pub fn debug(f: impl FnOnce() -> String) {
    let on = CVARS.0.borrow().as_ref().map(|c| cvar_value(c.debug) != 0.0).unwrap_or(false);
    if on {
        engine().print(&format!("[ts weapons] {}\n", f()));
    }
}

pub fn restriction() -> u32 {
    RESTRICTION.0.get()
}

/// Sets the restriction mask (`ts_mapglobals` spawnflags, `weaponrestriction`). The rules
/// broadcast it as `SrvSett 3`.
pub fn set_restriction(mask: u32) {
    RESTRICTION.0.set(mask & 0x1f);
}

pub fn grenade_buys() -> bool {
    GRENADE_BUYS.0.get()
}

pub fn set_grenade_buys(v: bool) {
    GRENADE_BUYS.0.set(v);
}

/// `weaponrestriction [n]` (`cmd_weaponrestriction` 0xc58a0): no argument prints the mask, 5
/// resets, otherwise `WeaponRestrictionByID(n) & allowed`.
unsafe extern "C" fn cmd_weaponrestriction() {
    let e = engine();
    if e.cmd_argc() < 2 {
        e.print(&format!("weapon restriction mask: 0x{:02x} (1 knives, 2 pistols, 4 SMGs, 8 rifles, 16 shotguns)\n", restriction()));
        return;
    }
    let n: i32 = e.cmd_argv(1).to_str().unwrap_or("").trim().parse().unwrap_or(0);
    let mask = match n {
        5 => 0x1f,
        1 => 0,
        2 => 1,
        3 => 3,
        4 => 0x13,
        _ => 0x1f,
    };
    set_restriction(if n == 5 { 0x1f } else { mask & restriction() });
    e.print(&format!("weapon restriction mask: 0x{:02x}\n", restriction()));
}

/// Per-map engine handles (strings and indices are reset on every level change).
pub struct Assets {
    pub events: [u16; 11],
    pub v_model: [string_t; MAX_WEAPONS],
    pub p_model: [string_t; MAX_WEAPONS],
    pub p_model_sil: [string_t; MAX_WEAPONS],
    pub w_model: [Option<&'static CStr>; MAX_WEAPONS],
}

static ASSETS: Global<RefCell<Option<Assets>>> = Global(RefCell::new(None));

pub fn with<R>(f: impl FnOnce(&Assets) -> R) -> Option<R> {
    ASSETS.0.borrow().as_ref().map(f)
}

pub fn event_index(ev: Event) -> u16 {
    with(|a| a.events[ev as usize]).unwrap_or(0)
}

/// Sounds the server itself emits or that TSPrecache lists for weapons.
const SOUNDS: &[&CStr] = &[
    c"weapons/switch.wav",
    c"player/kevlarhit.wav",
    c"player/kungfuhit.wav",
    c"player/closecombat.wav",
    c"player/block.wav",
    c"weapons/knife/knife_miss.wav",
    c"weapons/knife/knife_hit.wav",
    c"weapons/knife/knife_hitbody.wav",
    c"weapons/katana/hit-body.wav",
    c"weapons/katana/miss.wav",
    c"weapons/katana/miss1.wav",
    c"weapons/katana/hit.wav",
    c"weapons/katana/hit1.wav",
    c"items/gunpickup2.wav",
    c"items/9mmclip1.wav",
    c"items/weapondrop1.wav",
    c"weapons/items.wav",
    c"weapons/weapondrop.wav",
    c"weapons/draw.wav",
    c"weapons/pistol-empty.wav",
    c"weapons/insert-shell.wav",
    c"weapons/generic_reload.wav",
    c"weapons/shell.wav",
    c"weapons/sshell.wav",
    c"weapons/whizz1.wav",
    c"weapons/whizz2.wav",
    c"weapons/whizz3.wav",
    c"weapons/whizz4.wav",
    c"weapons/whizz5.wav",
    c"weapons/grenbounce1.wav",
    c"weapons/grenbounce2.wav",
    c"weapons/grenbounce3.wav",
    c"weapons/grenbounce4.wav",
    c"explo/explode.wav",
    c"explo/explode1.wav",
    c"explo/explode2.wav",
    c"explo/explode3.wav",
    c"explo/explode4.wav",
];

const MODELS: &[&CStr] = &[
    c"models/generic_clip.mdl",
    c"models/ammopack.mdl",
    c"models/bullet.mdl",
    c"models/long_bullet.mdl",
    c"models/pellet.mdl",
    c"models/w_m61.mdl",
    c"models/w_knife.mdl",
    c"models/w_sealknife.mdl",
    c"models/w_katana.mdl",
];

/// Called from worldspawn's precache, once per map.
pub fn precache() {
    super::bullets::reset();
    let e = engine();
    let mut events = [0u16; 11];
    for ev in Event::ALL {
        events[ev as usize] = e.precache_event(ev.path());
    }
    for s in SOUNDS {
        e.precache_sound(s);
    }
    for m in MODELS {
        e.precache_model(m);
    }
    let mut a = Assets {
        events,
        v_model: [0; MAX_WEAPONS],
        p_model: [0; MAX_WEAPONS],
        p_model_sil: [0; MAX_WEAPONS],
        w_model: [None; MAX_WEAPONS],
    };
    let t = tables();
    for id in 0..MAX_WEAPONS as u8 {
        if id != 0 && t.weapon(id).is_none() {
            continue;
        }
        let (v, p, psil, w) = models(id);
        e.precache_model_str(&v);
        a.v_model[id as usize] = e.alloc_string(intern(&v));
        if let Some(p) = p {
            e.precache_model_str(&p);
            a.p_model[id as usize] = e.alloc_string(intern(&p));
        }
        if let Some(ps) = psil {
            e.precache_model_str(&ps);
            a.p_model_sil[id as usize] = e.alloc_string(intern(&ps));
        }
        if let Some(w) = w {
            e.precache_model_str(&w);
            a.w_model[id as usize] = Some(intern(&w));
        }
        if let Some(asset) = t.asset(id) {
            if !asset.shell.is_empty() {
                e.precache_model_str(&format!("models/{}.mdl", asset.shell));
            }
            for s in [&asset.sound, &asset.sound2] {
                if !s.is_empty() {
                    e.precache_sound_str(&format!("weapons/{s}"));
                }
            }
        }
    }
    *ASSETS.0.borrow_mut() = Some(a);
    // customweapons: weapon parameter file applied at every map start
    let file = e.cvar_string(c"customweapons").to_string_lossy().into_owned();
    if !file.is_empty() {
        let f = file;
        if !super::custom::load(&f) {
            e.print(&format!("customweapons: can't read {f}\n"));
        }
    }
}
