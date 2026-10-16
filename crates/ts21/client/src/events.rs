//! TS 2.1's weapon events and their effects (issue #160): the fire events (0x10073b70 standard,
//! 0x10075080 akimbo, 0x10076ce0 pump), `EV_TS_FireBullets` (0x1007c5e0), the impact sound
//! (0x1007b820) and particles (0x10084240), the other events (0x10078810 empty, 0x10077c40 pump,
//! 0x10078940 reload, 0x100792e0 close combat, 0x10079aa0 coldcock, 0x1007a260 knife,
//! 0x1007ab50 katana, 0x1007d250 melee material sound) and the particle builders (muzzle flash
//! 0x10080a30, gun smoke 0x1007f970, sprite debris 0x1007ef20).

use goldsrc::ext::render::{Vec3, random_float, random_long};
use goldsrc::sys::{kRenderTransAdd, kRenderTransColor};
use ts_client::effects::fx::{self, FxStyle};
use ts_client::effects::psys::PSys;
use ts_client::gameplay::MeleeReach;

const ADD: i32 = kRenderTransAdd as i32;
const COLOR: i32 = kRenderTransColor as i32;

/// 2.1's builders: the flash starts at colour 1.0 with fade `0.15 × colour` and a 0.1 second
/// scale speed, has no third-person ×1.3, no blast push and ends with a `scintille_metal` spark;
/// silenced it keeps every sprite, colour `(512 − d) / 640`, fade 0.13 (÷1.1 each), roll
/// `RandomLong(-60, 60)`. The smoke has gravity 0.1 and 10 puffs at `cl_fx 2` (none at 0). No
/// builder writes drag, stretch or a debris react value; debris always fades and isn't scaled
/// by the quality.
pub static FX: FxStyle = FxStyle {
    flash_color: 1.0,
    flash_fade: 0.15,
    flash_scale_speed: 0.1,
    third_person_flash: 1.0,
    muzzle_blast: false,
    muzzle_spark: true,
    silenced_count: 1.0,
    silenced_color: 0.0015625,
    silenced_fade: Some(0.13),
    silenced_spin: 60,
    smoke_gravity: 0.1,
    smoke_puffs_by_cl_fx: true,
    drag: 0.0,
    stretch: false,
    debris_react: 0.0,
    debris_fade_any_mode: true,
    debris_count_by_quality: false,
    silenced_jitter: false,
    spark_moves: true,
    raw_slow_cooldowns: true,
    impact_smoke_speed: Some(16.0),
};

/// The unsilenced smoke's frame rate (0x10074125, 0x10074183): `RandomLong(5, 10)` for the
/// sawed-off (33) and weapon 11 (0x100740cb..0x100740db), else `RandomLong(30, 50)`.
pub fn gun_smoke_rate(id: i32) -> i32 {
    if id == 33 || id == 11 { random_long(5, 10) } else { random_long(30, 50) }
}

/// The fire sound's attenuation (2.1's client ammo table, field 0): 3.0's, but 2.1's .454 Casull
/// is type 12, with 0.3 (3.0's 12 is the .32 ACP's 0.5).
pub fn ammo_attenuation(ammo: usize) -> f32 {
    if ammo == 12 { 0.3 } else { ts_client::events::AMMO_ATTENUATION.get(ammo).copied().unwrap_or(0.4) }
}

/// The ricochet's pitch (0x1007ba3c..0x1007ba70): `ftol(100 + (s == 0 ? 300 : (float)(s × 300)))`
/// on the raw `TSSlowMo` factor `s`. The product is stored to a float (`fstp [ebp-0x108]`)
/// before the `+ 100`, so a product a hair under a whole number rounds up to it (issue #251).
/// The `TSFrags` explosion's pitch (0x10082240..0x10082274) is the same code.
pub fn ricochet_pitch(slowmo_factor100: f32) -> i32 {
    let s = slowmo_factor100 / 100.0;
    let x = if s == 0.0 { 300.0 } else { (s as f64 * 300.0f32 as f64) as f32 as f64 };
    ts_client::events::ftol(100.0f32 as f64 + x)
}

/// The other events' sounds (empty 0x10078894, pump 0x1007845b, reload 0x10078bdd, the melee
/// swings): pitch `98 + RandomLong(0, 3)` rolled first, then volume `RandomFloat(0.92, 1)`,
/// attenuation 0.8.
pub fn event_sound() -> (f32, f32, i32) {
    let pitch = 98 + random_long(0, 3);
    (random_float(0.92, 1.0), 0.8, pitch)
}

/// The melee material sound (0x1007d250): attenuation 0.8, pitch `96 + RandomLong(0, 15)`,
/// volume 0.9 but tile and glass 0.8 and duct 0.5; nothing for flesh.
pub fn melee_material_sound(tex: u8) -> Option<(f32, f32, i32)> {
    material_volume(tex).map(|vol| (vol, 0.8, 96 + random_long(0, 15)))
}

/// The melee material sound's volume by texture type (0x1007d2f0..): none for flesh.
pub fn material_volume(tex: u8) -> Option<f32> {
    match tex {
        b'F' => None,
        b'T' | b'P' | b'Y' => Some(0.8),
        b'V' => Some(0.5),
        _ => Some(0.9),
    }
}

/// The melee traces: close combat (0x100792e0) a back kick (3) 56 behind, a kick (2) 48 ahead,
/// else 38; the coldcock (0x10079aa0) is a copy of that code, so it reaches by its `i1` (the
/// weapon id) the same way; the knife 38; the katana's back swing (15) 54 behind, else 48.
pub fn melee_reach(m: MeleeReach) -> f32 {
    match m {
        MeleeReach::CloseCombat(3) | MeleeReach::Coldcock(3) => -56.0,
        MeleeReach::CloseCombat(2) | MeleeReach::Coldcock(2) => 48.0,
        MeleeReach::CloseCombat(_) | MeleeReach::Coldcock(_) => 38.0,
        MeleeReach::Knife => 38.0,
        MeleeReach::Katana(15) => -54.0,
        MeleeReach::Katana(_) => 48.0,
    }
}

/// `slowdiv` of the raw factor times `x` (0x100962e0), the cooldown interval.
fn slow(x: f32) -> f32 {
    let factor100 = ts_client::state::with(|s| s.slowmo_factor100);
    (crate::slowdiv(factor100 / 100.0) * x as f64) as f32
}

/// The bullet impact particles (0x10084240). Slot 7's cooldown (on 2.1's raw-factor slow time)
/// as 3.0's, culled the same way, with 3.0's puff counts and smoke. Unlike 3.0: concrete throws
/// `RandomLong(1, 3)` `debris_concrete3` chips (radius 4, gravity 9) and has no spark roll; metal
/// throws a `scintille2` burst (`RandomLong(1, 3)`, radius 4, life 0.2, gravity 9) instead of
/// sparks; glass has no sparks; wood throws `RandomLong(1, 3)` `woodfrag` chips (radius 6,
/// gravity 5).
pub fn impact(ps: &mut PSys, now: f64, pos: Vec3, n: Vec3, tex: u8, bullet_type: i32) {
    let iv = slow(ps.tuning.interval);
    if !ps.cooldown_ready(7, now, iv) && !matches!(bullet_type, 1 | 2) {
        return;
    }
    if !ps.visible(pos) {
        return;
    }
    let puffs = match bullet_type {
        1 => 1,
        2 => 2,
        _ => 4,
    };
    let roll = random_long(0, 100);
    let (wood, smoke) = ("sprites/wood_smoke_add.spr", "sprites/gun_smoke_add.spr");
    match tex {
        b'D' => fx::impact_smoke(ps, now, pos, n, wood, ADD, puffs, 16.0, 30, true),
        b'W' => {
            fx::sprite_debris(ps, now, pos, n, "sprites/woodfrag.spr", ADD, random_long(1, 3), 6.0, 32, 1.0, 5.0, false, true);
            fx::impact_smoke(ps, now, pos, n, wood, ADD, puffs, 16.0, 30, true);
        }
        b'G' | b'M' | b'V' => {
            if roll > 33 {
                fx::scintille_metal(ps, now, pos, n, 48.0);
            }
            fx::sprite_debris(ps, now, pos, n, "sprites/scintille2.spr", ADD, random_long(1, 3), 4.0, 32, 0.2, 9.0, false, true);
            if roll > 70 {
                fx::surface_mark(ps, now, pos, n, "sprites/shotgun_pellets.spr", ADD, 5.0, 0.75, 0.015, 0.25);
            }
            fx::impact_smoke(ps, now, pos, n, smoke, ADD, puffs, 16.0, 50, true);
        }
        b'P' | b'Y' => {
            fx::scintille_metal(ps, now, pos, n, 48.0);
            fx::impact_smoke(ps, now, pos, n, smoke, ADD, puffs, 16.0, 50, true);
        }
        b'F' | b'S' => {}
        _ => {
            fx::sprite_debris(ps, now, pos, n, "sprites/debris_concrete3.spr", COLOR, random_long(1, 3), 4.0, 32, 1.0, 9.0, false, true);
            fx::impact_smoke(ps, now, pos, n, smoke, ADD, puffs, 16.0, 20, true);
        }
    }
}

/// A knife or katana hit on a hard surface (0x1007a6ad, 0x1007aff7): `scintille_metal` (6, the
/// katana 12) and a `scintille2` burst of 5 (7) chips, radius 2, spread 16, life 0.5, gravity 5,
/// colliding, instead of sparks.
pub fn blade_hit(ps: &mut PSys, now: f64, pos: Vec3, n: Vec3, katana: bool) {
    fx::scintille_metal(ps, now, pos, n, if katana { 12.0 } else { 6.0 });
    let count = if katana { 7 } else { 5 };
    fx::sprite_debris(ps, now, pos, n, "sprites/scintille2.spr", ADD, count, 2.0, 16, 0.5, 5.0, false, true);
}
