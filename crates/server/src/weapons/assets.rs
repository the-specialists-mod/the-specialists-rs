//! Precache (`CTSGun::Precache` events, `TSPrecache` weapon models and sounds), per-map model
//! strings, cvars and the weapon restriction mask.

use goldsrc::Global;
use goldsrc::ext::weapons::WeaponsEngine;
use goldsrc::server::{cvar_value, engine, intern};
use goldsrc::sys::*;
use shared::weapons::{Event, MAX_WEAPONS, models, tables};
use std::cell::{Cell, RefCell};
use std::ffi::CStr;

/// `ts_weapons_debug` (not TS): logs fires, hits, explosions, drops to the server console.
static DEBUG: Global<Cell<Option<&'static cvar_t>>> = Global(Cell::new(None));
static GRENADE_BUYS: Global<Cell<bool>> = Global(Cell::new(false));
/// `g_sModelIndexBubbles` (`sprites/bubble.spr`, precached by TS's `W_Precache`).
static BUBBLES: Global<Cell<i32>> = Global(Cell::new(0));

/// Registers the weapons package's own cvars. The TS weapon cvars (`realbullet`, `ammocount`,
/// `weaponstay`, `damagemult`, `customweapons`) and `weaponrestriction` belong to the rules
/// (`rules::cvars`, `rules::commands`) and are read by name; `loadweapons`/`saveweapons` are
/// registered here.
pub fn register_cvars() {
    let e = engine();
    DEBUG.0.set(Some(e.register_cvar("ts_weapons_debug", "0", 0)));
    e.add_server_command(c"loadweapons", super::custom::cmd_loadweapons);
    e.add_server_command(c"saveweapons", super::custom::cmd_saveweapons);
}

/// Current values of the TS weapon cvars (registered by the rules).
pub struct CvarValues {
    /// `realbullet`: simulated bullets (`CBulletManager`) for everyone; 2+ = per-frame steps.
    pub realbullet: f32,
    /// `ammocount`: max-carry column (0 default, 1..3 reduced).
    pub ammocount: f32,
    /// `weaponstay`: dropped weapon lifetime (clamped 5..60 s; drops of the held gun 10..120 s
    /// at twice the value).
    pub weaponstay: f32,
    /// `damagemult`: weapon damage multiplier, only with `sv_cheats`.
    pub damagemult: f32,
}

/// `TSGameRules + 0x578`: `realbullet` rounded to an int (`CTSGameRules::Think` refreshes it every
/// frame), what `ShootBulletsPlayer` and `CBulletManager::Think` test.
pub fn realbullet() -> i32 {
    cvars().realbullet.round_ties_even() as i32
}

pub fn cvars() -> CvarValues {
    let e = engine();
    CvarValues {
        realbullet: e.cvar_float(c"realbullet"),
        ammocount: e.cvar_float(c"ammocount"),
        weaponstay: e.cvar_float(c"weaponstay"),
        damagemult: e.cvar_float(c"damagemult"),
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

/// `dmgreport`, read only with weapon cheats on (`g_flWeaponCheat`, i.e. `sv_cheats`), else 0.
/// Values above 1 print bullet and thrown-blade damage to everyone. Any non-zero value logs
/// player damage to the server console. As in TS, nothing registers it (`CVarGetFloat` of a
/// missing cvar is 0), so it only works where the engine lets the console create it.
pub fn dmgreport() -> f32 {
    let e = engine();
    if e.cvar_float(c"sv_cheats") == 0.0 {
        return 0.0;
    }
    e.cvar_float(c"dmgreport")
}

/// `g_sModelIndexBubbles` for `UTIL_BubbleTrail`.
pub fn bubbles_model() -> i32 {
    BUBBLES.0.get()
}

/// Logs `f()` when `ts_weapons_debug` is set.
pub fn debug(f: impl FnOnce() -> String) {
    let on = DEBUG.0.get().map(|c| cvar_value(c) != 0.0).unwrap_or(false);
    if on {
        engine().print(&format!("[ts weapons] {}\n", f()));
    }
}

/// The weapon restriction mask (1 knives, 2 pistols, 4 SMGs, 8 rifles, 16 shotguns): the rules'
/// `allowed_weapons` (`TSGameRules+0x5870`, set by `ts_mapglobals` and `weaponrestriction`, sent
/// as `SrvSett 3`). `BuyWeapon` (0x945bc) passes the whole int to `IsRestrictedWeapon`, so higher
/// bits (a `ts_mapglobals` with spawnflags above 0x1f) count: the mask isn't 0 then, and weapon
/// types outside the five categories stay buyable.
pub fn restriction() -> u32 {
    crate::rules::state::with(|g| g.allowed_weapons) as u32
}

/// Sets the restriction mask (the rules broadcast it as `SrvSett 3` on change).
pub fn set_restriction(mask: u32) {
    crate::rules::state::with(|g| g.allowed_weapons = mask as i32);
}

pub fn grenade_buys() -> bool {
    GRENADE_BUYS.0.get()
}

pub fn set_grenade_buys(v: bool) {
    GRENADE_BUYS.0.set(v);
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
    // +use on a dropped weapon (`PlayerUse`; TS precaches it in `ClientPrecache`).
    c"common/wpn_select.wav",
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
    BUBBLES.0.set(e.precache_model(c"sprites/bubble.spr"));
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
        if let Some(mut ps) = psil {
            // Port addition (defensive): TS precaches `p_pdw_sil.mdl` whatever the file's case,
            // which kills a case-sensitive server at map load; use the shipped spelling there.
            if id == shared::weapons::id::MP7 && e.load_file(&ps).is_none() && e.load_file(shared::weapons::MP7_SIL_ON_DISK).is_some() {
                ps = shared::weapons::MP7_SIL_ON_DISK.to_owned();
            }
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

#[cfg(test)]
mod tests {
    use shared::weapons::{id, is_restricted};

    #[test]
    fn restriction_mask_keeps_high_bits() {
        // `ts_mapglobals` spawnflags 0x3f leave 0x1f ^ 0x3f = 0x20. TS passes it whole: every
        // category is closed, but the mask isn't 0, so a grenade is still allowed. Masked with
        // 0x1f it would be 0 and close everything.
        let mask = 0x1f ^ 0x3f;
        assert!(is_restricted(id::BERETTA, mask));
        assert!(!is_restricted(id::M61, mask));
        assert!(is_restricted(id::M61, mask & 0x1f));
    }
}
