//! The map-wide precaches `CWorld::Precache` (0xf449c) issues, in TS's order: `W_Precache`
//! (0xf12e8), `ClientPrecache` (0xaeafc), `TSPrecache` (0x9f320), then its own sounds and gib
//! models. The engine numbers models, sounds and events in precache order, so the whole
//! sequence (set and order) matches TS's; the subsystems keep the indices they need. Entity
//! classes precache the rest in their `Spawn`, as TS's do.
//!
//! `UTIL_PrecacheOther(class)` creates an entity of the class, runs its `Precache` and removes
//! it again; the freed edict is reused by the next allocation, so only the class's precaches are
//! visible. They are listed here directly.

use goldsrc::server::engine;
use std::ffi::CStr;

/// `W_Precache`'s `UTIL_PrecacheOther` classes up to `weapon_tsgun`: `item_suit`
/// (`CItemSuit::Precache` 0xcb5e4) and `ts_bomb` (`CTSBomb::Precache` 0x8c570) precache
/// something; `ts_mapglobals`, `ts_objective_manager`, `ts_objective_ptr`, `ts_wingiver`,
/// `ts_trigger`, `ts_slowmotion`, `ts_slowmotionpoint`, `ts_dmhill`, `ts_hack`, `ts_teamescape`
/// and `ts_model` don't.
const W_PRECACHE_OTHERS: &[Asset] = &[Asset::Model(c"models/w_suit.mdl"), Asset::Sound(c"misc/beep2.wav")];

/// The end of `W_Precache`, after `ts_powerup` (`CTSPowerUp::Precache` 0x98548), `ts_groundweapon`
/// and `deadcorpse` (no precaches) and the discarded `IsDeathmatch()` call.
const W_PRECACHE_TAIL: &[Asset] = &[
    // mp.dll only (`W_Precache` 0x100799c0): the bots' debug beams. The Linux build has no bots.
    Asset::Lightning,
    Asset::Model(c"sprites/zerogxplode.spr"),
    Asset::Model(c"sprites/WXplo1.spr"),
    Asset::Model(c"sprites/steam1.spr"),
    Asset::Bubbles,
    Asset::BloodSpray,
    Asset::BloodDrop,
    // g_pModelNameLaser
    Asset::Model(c"sprites/laserbeam.spr"),
    Asset::Model(c"sprites/laserdot.spr"),
    Asset::Model(c"models/grenade.mdl"),
    Asset::Model(c"sprites/explode1.spr"),
    Asset::Sound(c"weapons/debris1.wav"),
    Asset::Sound(c"weapons/debris2.wav"),
    Asset::Sound(c"weapons/debris3.wav"),
    Asset::Sound(c"weapons/grenade_hit1.wav"),
    Asset::Sound(c"weapons/grenade_hit2.wav"),
    Asset::Sound(c"weapons/grenade_hit3.wav"),
    Asset::Sound(c"weapons/bullet_hit1.wav"),
    Asset::Sound(c"weapons/bullet_hit2.wav"),
    Asset::Sound(c"items/weapondrop1.wav"),
];

/// `ClientPrecache` (0xaeafc). `giPrecacheGrunt` is 0 in TS (nothing sets it), so
/// `monster_human_grunt` is never precached.
const CLIENT_PRECACHE: &[Asset] = &[
    Asset::Sound(c"player/sprayer.wav"),
    Asset::Sound(c"player/pl_fallpain3.wav"),
    Asset::Sound(c"player/pl_step1.wav"),
    Asset::Sound(c"player/pl_step2.wav"),
    Asset::Sound(c"player/pl_step3.wav"),
    Asset::Sound(c"player/pl_step4.wav"),
    Asset::Sound(c"player/pl_metal1.wav"),
    Asset::Sound(c"player/pl_metal2.wav"),
    Asset::Sound(c"player/pl_metal3.wav"),
    Asset::Sound(c"player/pl_metal4.wav"),
    Asset::Sound(c"player/pl_dirt1.wav"),
    Asset::Sound(c"player/pl_dirt2.wav"),
    Asset::Sound(c"player/pl_dirt3.wav"),
    Asset::Sound(c"player/pl_dirt4.wav"),
    Asset::Sound(c"player/pl_duct1.wav"),
    Asset::Sound(c"player/pl_duct2.wav"),
    Asset::Sound(c"player/pl_duct3.wav"),
    Asset::Sound(c"player/pl_duct4.wav"),
    Asset::Sound(c"player/pl_grate1.wav"),
    Asset::Sound(c"player/pl_grate2.wav"),
    Asset::Sound(c"player/pl_grate3.wav"),
    Asset::Sound(c"player/pl_grate4.wav"),
    Asset::Sound(c"player/pl_slosh1.wav"),
    Asset::Sound(c"player/pl_slosh2.wav"),
    Asset::Sound(c"player/pl_slosh3.wav"),
    Asset::Sound(c"player/pl_slosh4.wav"),
    Asset::Sound(c"player/pl_tile1.wav"),
    Asset::Sound(c"player/pl_tile2.wav"),
    Asset::Sound(c"player/pl_tile3.wav"),
    Asset::Sound(c"player/pl_tile4.wav"),
    Asset::Sound(c"player/pl_tile5.wav"),
    Asset::Sound(c"player/pl_swim1.wav"),
    Asset::Sound(c"player/pl_swim2.wav"),
    Asset::Sound(c"player/pl_swim3.wav"),
    Asset::Sound(c"player/pl_swim4.wav"),
    Asset::Sound(c"player/pl_ladder1.wav"),
    Asset::Sound(c"player/pl_ladder2.wav"),
    Asset::Sound(c"player/pl_ladder3.wav"),
    Asset::Sound(c"player/pl_ladder4.wav"),
    Asset::Sound(c"player/pl_wade1.wav"),
    Asset::Sound(c"player/pl_wade2.wav"),
    Asset::Sound(c"player/pl_wade3.wav"),
    Asset::Sound(c"player/pl_wade4.wav"),
    Asset::Sound(c"debris/wood1.wav"),
    Asset::Sound(c"debris/wood2.wav"),
    Asset::Sound(c"debris/wood3.wav"),
    Asset::Sound(c"plats/train_use1.wav"),
    Asset::Sound(c"buttons/spark5.wav"),
    Asset::Sound(c"buttons/spark6.wav"),
    Asset::Sound(c"debris/glass1.wav"),
    Asset::Sound(c"debris/glass2.wav"),
    Asset::Sound(c"debris/glass3.wav"),
    Asset::Sound(c"common/bodysplat.wav"),
    Asset::Sound(c"player/pl_pain2.wav"),
    Asset::Sound(c"player/pl_pain4.wav"),
    Asset::Sound(c"player/pl_pain5.wav"),
    Asset::Sound(c"player/pl_pain6.wav"),
    Asset::Sound(c"player/pl_pain7.wav"),
    Asset::Model(crate::player::PLAYER_MODEL),
    Asset::Sound(c"common/wpn_hudoff.wav"),
    Asset::Sound(c"common/wpn_hudon.wav"),
    Asset::Sound(c"common/wpn_moveselect.wav"),
    Asset::Sound(c"common/wpn_select.wav"),
    Asset::Sound(c"common/wpn_denyselect.wav"),
    Asset::Sound(c"player/geiger6.wav"),
    Asset::Sound(c"player/geiger5.wav"),
    Asset::Sound(c"player/geiger4.wav"),
    Asset::Sound(c"player/geiger3.wav"),
    Asset::Sound(c"player/geiger2.wav"),
    Asset::Sound(c"player/geiger1.wav"),
];

/// `TSPrecache` (0x9f320) before its weapon loop.
const TS_PRECACHE_HEAD: &[Asset] = &[
    Asset::Sound(c"player/superjump-land.wav"),
    Asset::Sound(c"player/land.wav"),
    Asset::Sound(c"player/dive.wav"),
    Asset::Sound(c"player/airwheel.wav"),
    Asset::Sound(c"player/headshot.wav"),
    Asset::Sound(c"player/headshot2.wav"),
    Asset::Sound(c"player/kevlarhit.wav"),
    Asset::Sound(c"player/block.wav"),
    Asset::Sound(c"player/splat.wav"),
    Asset::Sound(c"player/bodyhit1.wav"),
    Asset::Sound(c"player/bodyhit2.wav"),
    Asset::Sound(c"player/bodyhit3.wav"),
    Asset::Sound(c"player/bodyhit4.wav"),
    Asset::Sound(c"player/die.wav"),
    Asset::Sound(c"player/die1.wav"),
    Asset::Sound(c"player/die2.wav"),
    Asset::Sound(c"player/die3.wav"),
    Asset::Sound(c"player/pain1.wav"),
    Asset::Sound(c"player/pain2.wav"),
    Asset::Sound(c"player/pain3.wav"),
    Asset::Sound(c"player/pain4.wav"),
    Asset::Sound(c"misc/hacking.wav"),
    Asset::Sound(c"misc/hacked.wav"),
    Asset::Sound(c"misc/failed.wav"),
    Asset::Sound(c"misc/successful.wav"),
    Asset::Sound(c"misc/gogogo.wav"),
    Asset::Sound(c"debris/ric1.wav"),
    Asset::Sound(c"debris/ric2.wav"),
    Asset::Sound(c"debris/ric3.wav"),
    Asset::Sound(c"debris/ric4.wav"),
    Asset::Sound(c"debris/ric5.wav"),
    Asset::Sound(c"debris/cement.wav"),
    Asset::Sound(c"debris/cement1.wav"),
    Asset::Sound(c"debris/cement2.wav"),
    Asset::Sound(c"debris/cement3.wav"),
    Asset::Sound(c"debris/cement4.wav"),
    Asset::Sound(c"debris/cement5.wav"),
    Asset::Sound(c"debris/cement6.wav"),
    Asset::Sound(c"debris/gravel1.wav"),
    Asset::Sound(c"debris/gravel2.wav"),
    Asset::Sound(c"debris/gravel3.wav"),
    Asset::Sound(c"debris/gravel4.wav"),
    Asset::Sound(c"debris/metal1.wav"),
    Asset::Sound(c"debris/metal2.wav"),
    Asset::Sound(c"debris/metal3.wav"),
    Asset::Sound(c"debris/metal4.wav"),
    Asset::Sound(c"debris/metal5.wav"),
    Asset::Sound(c"weapons/shell.wav"),
    Asset::Sound(c"weapons/sshell.wav"),
];

/// `TSPrecache` after its weapon loop, with the `UTIL_PrecacheOther`s of `m61_grenade`
/// (`CTSGrenade::Precache` 0x7bdd8), `shrapnel` (`CTSShrapnel::Precache` 0x7c1c8) and `ts_flag`
/// (`CTSFlag::Precache` 0x9db94).
const TS_PRECACHE_TAIL: &[Asset] = &[
    Asset::Sound(c"weapons/pistol-empty.wav"),
    Asset::Sound(c"weapons/insert-shell.wav"),
    Asset::Sound(c"goslow.wav"),
    Asset::Sound(c"weapons/whizz1.wav"),
    Asset::Sound(c"weapons/whizz2.wav"),
    Asset::Sound(c"weapons/whizz3.wav"),
    Asset::Sound(c"weapons/whizz4.wav"),
    Asset::Sound(c"weapons/whizz5.wav"),
    Asset::Sound(c"player/closecombat.wav"),
    Asset::Sound(c"player/kungfuhit.wav"),
    Asset::Sound(c"weapons/weapondrop.wav"),
    Asset::Sound(c"weapons/items.wav"),
    Asset::Sound(c"weapons/draw.wav"),
    Asset::Sound(c"weapons/switch.wav"),
    Asset::Sound(c"items/9mmclip1.wav"),
    Asset::Sound(c"weapons/generic_reload.wav"),
    Asset::Sound(c"explo/explode.wav"),
    Asset::Sound(c"explo/explode1.wav"),
    Asset::Sound(c"explo/explode2.wav"),
    Asset::Sound(c"explo/explode3.wav"),
    Asset::Sound(c"explo/explode4.wav"),
    Asset::Model(c"models/generic_clip.mdl"),
    Asset::Model(c"models/ammopack.mdl"),
    Asset::Model(c"models/bullet.mdl"),
    Asset::Model(c"models/long_bullet.mdl"),
    Asset::Model(c"models/pellet.mdl"),
    // m61_grenade
    Asset::Sound(c"weapons/grenbounce1.wav"),
    Asset::Sound(c"weapons/grenbounce2.wav"),
    Asset::Sound(c"weapons/grenbounce3.wav"),
    Asset::Sound(c"weapons/grenbounce4.wav"),
    // shrapnel
    Asset::Model(c"models/shrapnel.mdl"),
    Asset::Sound(c"weapons/knife/knife_hit.wav"),
    // ts_flag
    Asset::Model(c"models/briefcase.mdl"),
    Asset::Sound(c"weapons/knife/knife_miss.wav"),
    Asset::Sound(c"weapons/knife/knife_hit.wav"),
    Asset::Sound(c"weapons/knife/knife_hitbody.wav"),
    Asset::Sound(c"weapons/katana/hit-body.wav"),
    Asset::Sound(c"weapons/katana/miss1.wav"),
    Asset::Sound(c"weapons/katana/miss.wav"),
    Asset::Sound(c"weapons/katana/hit.wav"),
    Asset::Sound(c"weapons/katana/hit1.wav"),
];

/// `CWorld::Precache`'s own sounds, after `TSPrecache`.
const WORLD_SOUNDS: &[Asset] = &[
    Asset::Sound(c"common/null.wav"),
    Asset::Sound(c"items/suitchargeok1.wav"),
    Asset::Sound(c"items/gunpickup2.wav"),
    Asset::Sound(c"common/bodydrop3.wav"),
    Asset::Sound(c"common/bodydrop4.wav"),
];

/// One precache of a list. The sprites whose indices TS keeps in globals are named.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Asset {
    Model(&'static CStr),
    Sound(&'static CStr),
    /// `g_sModelIndexBubbles` (`sprites/bubble.spr`).
    Bubbles,
    /// `g_sModelIndexBloodSpray` (`sprites/bloodspray.spr`).
    BloodSpray,
    /// `g_sModelIndexBloodDrop` (`sprites/blood.spr`).
    BloodDrop,
    /// `g_sModelIndexLightning` (`sprites/lgtning.spr`, mp.dll).
    Lightning,
}

impl Asset {
    fn precache(self) {
        let eng = engine();
        match self {
            Asset::Model(m) => {
                eng.precache_model(m);
            }
            Asset::Sound(s) => {
                eng.precache_sound(s);
            }
            Asset::Bubbles => crate::weapons::set_bubbles_model(eng.precache_model(c"sprites/bubble.spr")),
            Asset::BloodSpray => crate::entities::stock::blood::set_spray_sprite(eng.precache_model(c"sprites/bloodspray.spr")),
            Asset::BloodDrop => crate::entities::stock::blood::set_drop_sprite(eng.precache_model(c"sprites/blood.spr")),
            Asset::Lightning => crate::bots::nodes::set_lightning_sprite(eng.precache_model(c"sprites/lgtning.spr")),
        }
    }
}

fn precache_list(list: &[Asset]) {
    for a in list {
        a.precache();
    }
}

/// `W_Precache` (0xf12e8): the `UTIL_PrecacheOther`s, `CTSGun::Precache`'s events (through
/// `UTIL_PrecacheOtherWeapon("weapon_tsgun")`), `ts_powerup`'s model, then the shared sprites,
/// models and sounds.
pub fn w_precache() {
    precache_list(W_PRECACHE_OTHERS);
    crate::weapons::precache_events();
    engine().precache_model(crate::entities::ts::powerup::MODEL);
    precache_list(W_PRECACHE_TAIL);
}

/// `ClientPrecache` (0xaeafc).
pub fn client_precache() {
    precache_list(CLIENT_PRECACHE);
}

/// `TSPrecache` (0x9f320): player and debris sounds, every weapon's models and sounds
/// (`weapons::precache_weapon_assets`), then the shared weapon sounds and models.
pub fn ts_precache() {
    precache_list(TS_PRECACHE_HEAD);
    crate::weapons::precache_weapon_assets();
    precache_list(TS_PRECACHE_TAIL);
}

/// The rest of `CWorld::Precache`'s precaches: its sounds, then the gib models for
/// `sv_language` (`g_Language`, `(int)`, truncated: 0xf45f7; 1 = German).
pub fn world_sounds_and_gibs() {
    precache_list(WORLD_SOUNDS);
    let eng = engine();
    let language = crate::rules::x87_trunc(eng.cvar_float(c"sv_language") as f64);
    crate::entities::stock::blood::set_language(language);
    for m in gib_models(language) {
        eng.precache_model(m);
    }
}

/// `CWorld::Precache`'s gib models: `models/germangibs.mdl` with `g_Language == 1`, else the
/// human and alien gibs.
fn gib_models(language: i32) -> &'static [&'static CStr] {
    if language == 1 { &[c"models/germangibs.mdl"] } else { &[c"models/hgibs.mdl", c"models/agibs.mdl"] }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[Asset]) -> Vec<&'static CStr> {
        list.iter()
            .filter_map(|a| match a {
                Asset::Model(n) | Asset::Sound(n) => Some(*n),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn lists_keep_ts_order() {
        let c = names(CLIENT_PRECACHE);
        assert_eq!(c.len(), 70);
        assert_eq!(c[0], c"player/sprayer.wav");
        assert_eq!(c[58], c"models/player.mdl");
        assert_eq!(c[69], c"player/geiger1.wav");
        let h = names(TS_PRECACHE_HEAD);
        assert_eq!(h.len(), 49);
        assert_eq!((h[0], h[48]), (c"player/superjump-land.wav", c"weapons/sshell.wav"));
        let t = names(TS_PRECACHE_TAIL);
        assert_eq!(t.len(), 41);
        // knife_hit.wav twice: shrapnel's Precache, then TSPrecache's own list.
        assert_eq!(t.iter().filter(|n| **n == c"weapons/knife/knife_hit.wav").count(), 2);
        assert_eq!(W_PRECACHE_TAIL[4..7], [Asset::Bubbles, Asset::BloodSpray, Asset::BloodDrop]);
        assert_eq!(names(WORLD_SOUNDS)[0], c"common/null.wav");
    }

    #[test]
    fn gibs_by_language() {
        assert_eq!(gib_models(0), &[c"models/hgibs.mdl", c"models/agibs.mdl"]);
        assert_eq!(gib_models(1), &[c"models/germangibs.mdl"]);
        assert_eq!(gib_models(2), &[c"models/hgibs.mdl", c"models/agibs.mdl"]);
    }
}
