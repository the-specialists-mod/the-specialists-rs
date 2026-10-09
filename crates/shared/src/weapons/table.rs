//! Static weapon data. `tables()` parses the embedded TOML once.

use serde::Deserialize;
use std::sync::{OnceLock, RwLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeaponKind {
    Kungfu,
    Pistol,
    Smg,
    Rifle,
    Shotgun,
    Sniper,
    Akimbo,
    AkimboOnly,
    Grenade,
    Knife,
    Katana,
}

/// TS fire modes, in the bit order of `s_weapon_info` (weapons.md §4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FireMode {
    Auto,
    Semi,
    Burst,
    Pump,
    AkSemi,
    AkAuto,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WeaponInfo {
    pub id: u8,
    pub key: String,
    pub name: String,
    pub kind: WeaponKind,
    /// HUD grid position [column, row].
    pub hud: [u8; 2],
    pub ammo: u8,
    pub clip: i32,
    pub modes: Vec<FireMode>,
    /// Seconds between shots.
    pub rate: f32,
    pub reload: f32,
    pub damage: f32,
    pub pellets: i32,
    pub bullet: i32,
    pub accuracy: f32,
    pub scale_spread: bool,
    pub acc_kick: f32,
    pub view_kick: f32,
    pub kick: bool,
    pub stock: i32,
    /// Zoom FOV steps (0 = none).
    pub zoom: [i32; 2],
    pub slots: i32,
    pub cost: i32,
    pub stack: bool,
    pub att_allowed: u8,
    pub att_default: u8,
    /// Akimbo version of this weapon, or -1.
    pub akimbo: i32,
    /// Single version of an akimbo weapon (itself otherwise), or -1.
    pub single: i32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AmmoInfo {
    pub id: u8,
    pub name: String,
    pub price: i32,
    pub per_buy: i32,
    /// Indexed by the `ammocount` cvar.
    pub max_carry: [i32; 4],
    pub pierce: f32,
    pub range_unit: f32,
    /// Metres per second (x40 units/m for simulated bullets).
    pub velocity: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Asset {
    pub id: u8,
    /// Model stem: models/v_<model>.mdl, p_<model>.mdl; world model w_<w_model>.mdl.
    pub model: String,
    pub w_model: String,
    pub p_model_sil: String,
    pub sound: String,
    pub sound2: String,
    pub shell: String,
    pub anim_ext: String,
    /// Body group additions for silencer, laser, flashlight, scope, laser+flashlight.
    pub body: [i32; 5],
    pub fx: [i32; 4],
    pub f_d4: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Silencer {
    pub damage: f32,
    pub spread: f32,
    pub kick: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SpreadPenalty {
    pub max: f32,
    pub decay_per_s: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Burst {
    pub shots: i32,
    pub gap: f32,
    pub end: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Pierce {
    pub min_left: f32,
    pub max_step: f32,
    pub player_factor: f32,
    pub unit: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Grenade {
    pub fuse: f32,
    pub damage: f32,
    pub r_full: f32,
    pub r_max: f32,
    pub max_speed: f32,
    pub per_life: i32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Thrown {
    pub speed: f32,
    pub knife: f32,
    pub seal: f32,
    pub katana: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Slots {
    pub total: i32,
    pub kevlar: i32,
    pub flag: i32,
    pub speed_min: f32,
    pub speed_max: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AttachmentPrice {
    pub silencer: i32,
    pub laser: i32,
    pub flashlight: i32,
    pub scope: i32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Combat {
    pub armor_absorb: f32,
    pub silencer: Silencer,
    pub headshot: f32,
    pub headshot_shotgun: f32,
    pub leg: f32,
    pub stunt_damage_taken: f32,
    pub spread_penalty: SpreadPenalty,
    pub semi_delay: f32,
    pub burst: Burst,
    pub pierce: Pierce,
    pub grenade: Grenade,
    pub thrown: Thrown,
    pub slots: Slots,
    pub attachment_price: AttachmentPrice,
}

#[derive(Clone)]
pub struct Tables {
    weapons: Vec<Option<WeaponInfo>>,
    ammo: Vec<Option<AmmoInfo>>,
    assets: Vec<Option<Asset>>,
    pub combat: Combat,
}

impl Tables {
    pub fn weapon(&self, id: u8) -> Option<&WeaponInfo> {
        self.weapons.get(id as usize)?.as_ref()
    }
    pub fn ammo(&self, id: u8) -> Option<&AmmoInfo> {
        self.ammo.get(id as usize)?.as_ref()
    }
    pub fn asset(&self, id: u8) -> Option<&Asset> {
        self.assets.get(id as usize)?.as_ref()
    }
    pub fn weapons(&self) -> impl Iterator<Item = &WeaponInfo> {
        self.weapons.iter().flatten()
    }
    pub fn weapon_by_key(&self, key: &str) -> Option<&WeaponInfo> {
        self.weapons().find(|w| w.key.eq_ignore_ascii_case(key))
    }
}

#[derive(Deserialize)]
struct WeaponFile {
    weapon: Vec<WeaponInfo>,
}
#[derive(Deserialize)]
struct AmmoFile {
    ammo: Vec<AmmoInfo>,
}
#[derive(Deserialize)]
struct AssetFile {
    asset: Vec<Asset>,
}

fn by_id<T>(items: Vec<T>, id: impl Fn(&T) -> u8) -> Vec<Option<T>> {
    let mut out: Vec<Option<T>> = Vec::new();
    for it in items {
        let i = id(&it) as usize;
        if out.len() <= i {
            out.resize_with(i + 1, || None);
        }
        out[i] = Some(it);
    }
    out
}

pub fn parse() -> Result<Tables, toml::de::Error> {
    let w: WeaponFile = toml::from_str(include_str!("../../data/weapons.toml"))?;
    let a: AmmoFile = toml::from_str(include_str!("../../data/ammo.toml"))?;
    let s: AssetFile = toml::from_str(include_str!("../../data/assets.toml"))?;
    let combat: Combat = toml::from_str(include_str!("../../data/combat.toml"))?;
    Ok(Tables { weapons: by_id(w.weapon, |w| w.id), ammo: by_id(a.ammo, |a| a.id), assets: by_id(s.asset, |a| a.id), combat })
}

fn base() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(|| parse().expect("embedded weapon tables"))
}

/// Custom weapon parameters (`loadweapons` / `CustomWP`) replace the base tables.
static CUSTOM: RwLock<Option<&'static Tables>> = RwLock::new(None);

/// The weapon tables: the embedded TS 3.0 values, or the custom ones when a server loaded a
/// weapons file (`CustomWP`). Parsed once; panics on a malformed embedded table (caught by
/// `cargo test`).
pub fn tables() -> &'static Tables {
    if let Some(t) = *CUSTOM.read().unwrap() {
        return t;
    }
    base()
}

impl Tables {
    /// `AdjustSpecificWeapon` / `adjust_weapon_value`: the five fields a weapons file or a
    /// `CustomWP` message can change. Accuracy is clamped to >= 0.001. False for invalid ids.
    pub fn with_weapon_params(&self, id: u8, accuracy: f32, view_kick: f32, acc_kick: f32, damage: f32, cost: i32) -> Option<Tables> {
        let mut t = self.clone();
        let w = t.weapons.get_mut(id as usize)?.as_mut()?;
        w.accuracy = accuracy.max(0.001);
        w.view_kick = view_kick;
        w.acc_kick = acc_kick;
        w.damage = damage;
        w.cost = cost;
        Some(t)
    }

    /// `armor_absorb x` line of a weapons file (clamped at 0).
    pub fn with_armor_absorb(&self, v: f32) -> Tables {
        let mut t = self.clone();
        t.combat.armor_absorb = v.max(0.0);
        t
    }
}

/// Installs custom tables (the old ones are leaked; changes are rare).
pub fn set_custom_tables(t: Tables) {
    *CUSTOM.write().unwrap() = Some(Box::leak(Box::new(t)));
}

/// Changes one weapon's custom parameters. False for invalid ids.
pub fn set_weapon_params(id: u8, accuracy: f32, view_kick: f32, acc_kick: f32, damage: f32, cost: i32) -> bool {
    match tables().with_weapon_params(id, accuracy, view_kick, acc_kick, damage, cost) {
        Some(t) => {
            set_custom_tables(t);
            true
        }
        None => false,
    }
}

/// Back to the embedded TS 3.0 values.
pub fn reset_custom_tables() {
    *CUSTOM.write().unwrap() = None;
}

/// True while custom parameters are in effect.
pub fn custom_tables_active() -> bool {
    CUSTOM.read().unwrap().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_parse() {
        let t = parse().unwrap();
        assert_eq!(t.weapons().count(), 37);
        let ak = t.weapon(13).unwrap();
        assert_eq!(ak.name, "AK47");
        assert_eq!(ak.clip, 30);
        assert_eq!(ak.modes, vec![FireMode::Auto, FireMode::Semi]);
        assert!(t.weapon(29).is_none(), "29 is the briefcase");
        assert_eq!(t.ammo(4).unwrap().name, "5.56 Nato");
        assert_eq!(t.asset(13).unwrap().model, "ak47");
        assert!((t.combat.armor_absorb - 0.2125).abs() < 1e-6);
    }

    #[test]
    fn weapon_params_override() {
        let t = parse().unwrap();
        let c = t.with_weapon_params(13, 0.0, 1.0, 2.0, 50.0, 99).unwrap();
        let ak = c.weapon(13).unwrap();
        assert_eq!((ak.accuracy, ak.view_kick, ak.acc_kick, ak.damage, ak.cost), (0.001, 1.0, 2.0, 50.0, 99));
        assert!(t.with_weapon_params(29, 0.0, 0.0, 0.0, 0.0, 0).is_none());
    }
}
