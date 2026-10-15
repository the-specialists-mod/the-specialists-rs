//! Weapon, ammo and combat tables, decoded from TS 3.0 (data/*.toml) and loaded identically on
//! server and client. The predicted weapon state machine lives here too (see docs/design.md):
//!
//! * [`gun::Gun`]: the per-player `CTSGun` (38 weapon records, timers, fire modes, reloads,
//!   zoom, akimbo, melee timing), driven through the [`gun::GunHost`] trait so the server and the
//!   client prediction run the same code.
//! * [`random`]: the SDK shared random generator (`UTIL_SharedRandomFloat/Long`).
//! * [`spread`]: seeded pellet directions, range falloff, recoil (`VKickback`) and math helpers.

pub mod anims;
pub mod gun;
pub mod random;
pub mod spread;
pub mod table;

pub use gun::{Event, EventArgs, Gun, GunHost, Status, Virtual};
pub use table::{AmmoInfo, Asset, Combat, FireMode, Tables, WeaponInfo, WeaponKind, tables};

/// Weapon ids are the TS ids (0 = kung fu .. 37); 29 is the briefcase and has no table entry.
pub const MAX_WEAPONS: usize = 38;
/// Ammo types (`s_ammo_info`, 16 entries; 0 and 14/15 are unused).
pub const MAX_AMMO: usize = 16;

/// TS weapon ids (`tsweapon_t`).
pub mod id {
    pub const KUNGFU: u8 = 0;
    pub const GLOCK18: u8 = 1;
    pub const BERETTA: u8 = 2;
    pub const UZI: u8 = 3;
    pub const BENELLI: u8 = 4;
    pub const M4: u8 = 5;
    pub const MP5SD: u8 = 6;
    pub const MP5K: u8 = 7;
    pub const BERETTA_AKIMBO: u8 = 8;
    pub const MK23: u8 = 9;
    pub const MK23_AKIMBO: u8 = 10;
    pub const USAS: u8 = 11;
    pub const DEAGLE: u8 = 12;
    pub const AK47: u8 = 13;
    pub const FIVESEVEN: u8 = 14;
    pub const AUG: u8 = 15;
    pub const UZI_AKIMBO: u8 = 16;
    pub const SKORPION: u8 = 17;
    pub const BARRETT: u8 = 18;
    pub const MP7: u8 = 19;
    pub const SPAS: u8 = 20;
    pub const GOLDEN_COLTS: u8 = 21;
    pub const GLOCK20: u8 = 22;
    pub const UMP: u8 = 23;
    pub const M61: u8 = 24;
    pub const KNIFE: u8 = 25;
    pub const MOSSBERG: u8 = 26;
    pub const M16: u8 = 27;
    pub const RUGER: u8 = 28;
    pub const BRIEFCASE: u8 = 29;
    pub const FIVESEVEN_AKIMBO: u8 = 30;
    pub const RAGING_BULL: u8 = 31;
    pub const M60: u8 = 32;
    pub const SAWEDOFF: u8 = 33;
    pub const KATANA: u8 = 34;
    pub const SEAL_KNIFE: u8 = 35;
    pub const CONTENDER: u8 = 36;
    pub const SKORPION_AKIMBO: u8 = 37;
}

/// TS weapon kind numbers (`s_weapon_info+0x348`, weapons.md §2).
pub mod kind {
    pub const KUNGFU: u8 = 0;
    pub const SHOTGUN: u8 = 1;
    pub const GRENADE: u8 = 2;
    pub const KNIFE: u8 = 3;
    pub const AKIMBO: u8 = 4;
    pub const AKIMBO_ONLY: u8 = 5;
    pub const RIFLE: u8 = 6;
    pub const SNIPER: u8 = 7;
    pub const PISTOL: u8 = 8;
    pub const SMG: u8 = 9;
    pub const KATANA: u8 = 10;
    pub const FLAG: u8 = 11;
}

/// Attachment ("item") bits: `s_weapon_status+0x10` (owned) and `+0x14` (switched on).
pub mod att {
    pub const SILENCER: u32 = 0x01;
    pub const LASER: u32 = 0x02;
    pub const FLASHLIGHT: u32 = 0x04;
    pub const SCOPE: u32 = 0x08;
    /// Loadout items (`tki`): kevlar and stealth shoes.
    pub const KEVLAR: u32 = 0x10;
    pub const STEALTH_SHOES: u32 = 0x20;
    /// Synthetic flag in `GetActiveItems` (ActItems): the player holds an akimbo weapon.
    pub const AKIMBO: u32 = 0x40;
}

/// Fire modes (`s_weapon_status+2`), bit n of `s_weapon_info+0xc` = mode n.
pub mod mode {
    pub const AUTO: u8 = 0;
    pub const SEMI: u8 = 1;
    pub const BURST: u8 = 2;
    pub const PUMP: u8 = 3;
    pub const AKIMBO_SEMI: u8 = 4;
    pub const AKIMBO_AUTO: u8 = 5;
}

/// Weapon table entry, or `None` for invalid ids (29, >= 38).
pub fn info(id: u8) -> Option<&'static WeaponInfo> {
    tables().weapon(id)
}

/// TS kind number of a table kind.
pub fn ts_kind(k: WeaponKind) -> u8 {
    match k {
        WeaponKind::Kungfu => kind::KUNGFU,
        WeaponKind::Shotgun => kind::SHOTGUN,
        WeaponKind::Grenade => kind::GRENADE,
        WeaponKind::Knife => kind::KNIFE,
        WeaponKind::Akimbo => kind::AKIMBO,
        WeaponKind::AkimboOnly => kind::AKIMBO_ONLY,
        WeaponKind::Rifle => kind::RIFLE,
        WeaponKind::Sniper => kind::SNIPER,
        WeaponKind::Pistol => kind::PISTOL,
        WeaponKind::Smg => kind::SMG,
        WeaponKind::Katana => kind::KATANA,
    }
}

/// TS kind of weapon `id` (0 for invalid ids, like `get_weapon_info` clamping to entry 0).
pub fn kind_of(id: u8) -> u8 {
    info(id).map(|w| ts_kind(w.kind)).unwrap_or(kind::KUNGFU)
}

/// `s_weapon_info+0xc`: firemode bitmask.
pub fn mode_mask(w: &WeaponInfo) -> u32 {
    w.modes.iter().fold(0, |m, f| m | 1 << (*f as u32))
}

/// Starting mode on pickup (`TakeWeapon`): bit0 -> auto, else bit3 -> pump, else bit2 -> burst,
/// else semi.
pub fn default_mode(mask: u32) -> u8 {
    if mask & 1 != 0 {
        mode::AUTO
    } else if mask & 8 != 0 {
        mode::PUMP
    } else if mask & 4 != 0 {
        mode::BURST
    } else {
        mode::SEMI
    }
}

/// `IsWeaponAkimbo` (kind 4, built from two singles).
pub fn is_akimbo(id: u8) -> bool {
    kind_of(id) == kind::AKIMBO
}

/// `IsOnlyAkimbo` (kind 5, Golden Colts).
pub fn is_only_akimbo(id: u8) -> bool {
    kind_of(id) == kind::AKIMBO_ONLY
}

/// `GetWeaponType` 0x8705c: the kind used by the restriction system.
pub fn restriction_type(id: u8) -> u8 {
    match id {
        id::KATANA => kind::KNIFE,
        id::SAWEDOFF => kind::SHOTGUN,
        id::UZI_AKIMBO | id::SKORPION_AKIMBO => kind::SMG,
        id::MK23_AKIMBO | id::FIVESEVEN_AKIMBO | id::BERETTA_AKIMBO | id::GOLDEN_COLTS => kind::PISTOL,
        _ => kind_of(id),
    }
}

/// `IsRestrictedWeapon(id, allowed_mask)` 0x870c8. Mask bits: 1 knives, 2 pistols, 4 SMGs,
/// 8 rifles/sniper, 0x10 shotguns. Mask 0 blocks every weapon id 1..37.
pub fn is_restricted(id: u8, allowed: u32) -> bool {
    if !(1..=37).contains(&id) {
        return false;
    }
    if allowed == 0 {
        return true;
    }
    match restriction_type(id) {
        kind::SHOTGUN => allowed & 0x10 == 0,
        kind::KNIFE => allowed & 0x01 == 0,
        kind::RIFLE | kind::SNIPER => allowed & 0x08 == 0,
        kind::PISTOL => allowed & 0x02 == 0,
        kind::SMG => allowed & 0x04 == 0,
        _ => false,
    }
}

/// `GetBodyByItems(id, items)` 0x7f0a4: view/world model body group for the attachments.
pub fn body_by_items(id: u8, items: u32) -> i32 {
    let body = tables().asset(id).map(|a| a.body).unwrap_or([0, 0, 0, 0, -1]);
    let mut b = if items & att::SILENCER != 0 { body[0] } else { 0 };
    if items & 6 == 6 && body[4] != -1 {
        b += body[4];
    } else {
        if items & att::LASER != 0 {
            b += body[1];
        }
        if items & att::FLASHLIGHT != 0 {
            b += body[2];
        }
    }
    if items & att::SCOPE != 0 {
        b += body[3];
    } else if id == id::CONTENDER {
        b += 1;
    }
    b
}

/// Model paths for weapon `id`: (v_, p_, p_ silenced, w_), the names TS uses. Kung fu only has
/// `v_melee.mdl`. TS's `models/p_pdw_sil.mdl` is shipped as `p_pdw_Sil.mdl` (see
/// [`MP7_SIL_ON_DISK`]).
pub fn models(id: u8) -> (String, Option<String>, Option<String>, Option<String>) {
    models_in(tables(), id)
}

/// [`models`] from the tables `t` (a ruleset's, active or not).
pub fn models_in(t: &Tables, id: u8) -> (String, Option<String>, Option<String>, Option<String>) {
    if id == id::KUNGFU {
        return ("models/v_melee.mdl".into(), None, None, None);
    }
    match t.asset(id) {
        Some(a) => {
            let sil = (!a.p_model_sil.is_empty()).then(|| format!("models/{}", a.p_model_sil));
            (
                format!("models/v_{}.mdl", a.model),
                Some(format!("models/p_{}.mdl", a.model)),
                sil,
                Some(format!("models/w_{}.mdl", a.w_model)),
            )
        }
        None => ("models/v_melee.mdl".into(), None, None, None),
    }
}

/// The file name TS 3.0's content has for the MP7's silenced p_ model (TS asks for
/// `models/p_pdw_sil.mdl`). Engines that look files up case-insensitively (all of them on
/// Windows; FWGS everywhere) find it under TS's name; a case-sensitive server would fail the
/// map's precache, so the server falls back to this spelling there.
pub const MP7_SIL_ON_DISK: &str = "models/p_pdw_Sil.mdl";

/// `range_damage_multipler[idx]` (`compile_range_damage` 0x7f1f4) as a pure function:
/// `min(1, 1/sqrt(max(idx, 1) * 0.1))`.
pub fn range_multiplier(idx: i32) -> f32 {
    let i = idx.clamp(1, 9999);
    (1.0 / ((i as f64) * 0.1).sqrt()).min(1.0) as f32
}

/// Player view model sequence for weapon `id`, table A or B, index `i` (0 when out of range).
pub fn anim(id: u8, alt: bool, i: usize) -> i32 {
    let (a, b) = tables().anims(id.min(37));
    *(if alt { b } else { a }).get(i).unwrap_or(&0) as i32
}

/// The weapon has an alternate grip (sequence table B exists).
pub fn has_alt_grip(id: u8) -> bool {
    tables().anims(id.min(37)).1[0] != -1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_falloff() {
        assert_eq!(range_multiplier(0), 1.0);
        assert_eq!(range_multiplier(10), 1.0);
        assert!((range_multiplier(40) - 0.5).abs() < 1e-6);
    }
}
