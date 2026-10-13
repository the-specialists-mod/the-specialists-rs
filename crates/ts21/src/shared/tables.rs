//! TS 2.1's weapon tables (issue #100 §2.3): `data/weapons21.toml`, `ammo21.toml`,
//! `assets21.toml` and `combat21.toml`, decoded from 2.1's `mp.dll` (each file cites its
//! writers). [`tables`] is the plugin's [`GunRules::tables`](shared::gameplay::GunRules::tables).
//!
//! 2.1 stores a damage *multiplier* per weapon (`info+0x1c`) and an integer base damage per ammo
//! type (`+0x1c`); `ShootBulletsPlayer` (0x1003fd40) multiplies the two. The shared
//! `WeaponInfo::damage` holds their product, `(float)base * mult`; [`damage_multiplier`] and
//! [`ammo_damage`] give the two factors for code that needs 2.1's exact order of operations.

use serde::Deserialize;
use shared::weapons::anims::ANIMS;
use shared::weapons::table::{AmmoInfo, Asset, Combat, FireMode, SeqTables, Tables, WeaponInfo, WeaponKind};
use shared::weapons::{MAX_AMMO, MAX_WEAPONS, id};
use std::sync::OnceLock;

/// The damage of 2.1's stock hit (+alt2), on every weapon that has one (`CTSGun::AlternateAttack`
/// 0x1004ab40 passes 20.0 to `FUN_100572a0`).
pub const STOCK_DAMAGE: i32 = 20;

/// The STEYR-TMP's view-model sequence tables (`Write_skorpion` 0x100056b0, `info+0x26a` and
/// `+0x2a4`). TS 3.0 ships the same `v_tmp.mdl`. Every other weapon keeps 3.0's tables, because
/// they index the 3.0 view models the port plays with (3.0 added stock, throw and block
/// sequences, and replaced `v_ump.mdl`).
pub const TMP_SEQUENCES: SeqTables = (
    [0, 1, 2, 3, 4, 3, 4, 5, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [7, 10, 2, 8, 9, 8, 9, 6, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
);

/// One `weapons21.toml` entry: the shared fields, with 2.1's damage multiplier and stock flag.
#[derive(Deserialize)]
struct Weapon21 {
    id: u8,
    key: String,
    name: String,
    kind: WeaponKind,
    hud: [u8; 2],
    ammo: u8,
    clip: i32,
    modes: Vec<FireMode>,
    rate: f32,
    reload: f32,
    mult: f32,
    pellets: i32,
    bullet: i32,
    accuracy: f32,
    scale_spread: bool,
    acc_kick: f32,
    view_kick: f32,
    kick: bool,
    stock: bool,
    zoom: [i32; 2],
    slots: i32,
    cost: i32,
    stack: bool,
    att_allowed: u8,
    att_default: u8,
    akimbo: i32,
    single: i32,
}

#[derive(Deserialize)]
struct Ammo21 {
    id: u8,
    name: String,
    price: i32,
    per_buy: i32,
    max_carry: [i32; 4],
    pierce: f32,
    range_unit: f32,
    velocity: f32,
    damage: i32,
}

#[derive(Deserialize)]
struct WeaponFile {
    weapon: Vec<Weapon21>,
}
#[derive(Deserialize)]
struct AmmoFile {
    ammo: Vec<Ammo21>,
}
#[derive(Deserialize)]
struct AssetFile {
    asset: Vec<Asset>,
}

/// The tables and the two damage factors.
struct Data {
    tables: Tables,
    mult: [f32; MAX_WEAPONS],
    ammo_damage: [i32; MAX_AMMO],
}

fn parse() -> Result<Data, toml::de::Error> {
    let w: WeaponFile = toml::from_str(include_str!("../../data/weapons21.toml"))?;
    let a: AmmoFile = toml::from_str(include_str!("../../data/ammo21.toml"))?;
    let s: AssetFile = toml::from_str(include_str!("../../data/assets21.toml"))?;
    let combat: Combat = toml::from_str(include_str!("../../data/combat21.toml"))?;
    let mut ammo_damage = [0; MAX_AMMO];
    for x in &a.ammo {
        ammo_damage[x.id as usize] = x.damage;
    }
    let mut mult = [0.0; MAX_WEAPONS];
    let weapons = w
        .weapon
        .into_iter()
        .map(|x| {
            mult[x.id as usize] = x.mult;
            let base = ammo_damage.get(x.ammo as usize).copied().unwrap_or(0);
            WeaponInfo {
                id: x.id,
                key: x.key,
                name: x.name,
                kind: x.kind,
                hud: x.hud,
                ammo: x.ammo,
                clip: x.clip,
                modes: x.modes,
                rate: x.rate,
                reload: x.reload,
                damage: base as f32 * x.mult,
                pellets: x.pellets,
                bullet: x.bullet,
                accuracy: x.accuracy,
                scale_spread: x.scale_spread,
                acc_kick: x.acc_kick,
                view_kick: x.view_kick,
                kick: x.kick,
                stock: if x.stock { STOCK_DAMAGE } else { 0 },
                zoom: x.zoom,
                slots: x.slots,
                cost: x.cost,
                stack: x.stack,
                att_allowed: x.att_allowed,
                att_default: x.att_default,
                akimbo: x.akimbo,
                single: x.single,
            }
        })
        .collect();
    let ammo = a
        .ammo
        .into_iter()
        .map(|x| AmmoInfo {
            id: x.id,
            name: x.name,
            price: x.price,
            per_buy: x.per_buy,
            max_carry: x.max_carry,
            pierce: x.pierce,
            range_unit: x.range_unit,
            velocity: x.velocity,
        })
        .collect();
    let mut anims = ANIMS.to_vec();
    anims[id::SKORPION as usize] = TMP_SEQUENCES;
    Ok(Data { tables: Tables::new(weapons, ammo, s.asset, anims, combat), mult, ammo_damage })
}

fn data() -> &'static Data {
    static D: OnceLock<Data> = OnceLock::new();
    D.get_or_init(|| parse().expect("embedded TS 2.1 weapon tables"))
}

/// TS 2.1's tables.
pub fn tables() -> &'static Tables {
    &data().tables
}

/// Weapon `id`'s damage multiplier (`s_weapon_info+0x1c`), 0 for an id 2.1 doesn't have.
pub fn damage_multiplier(id: u8) -> f32 {
    data().mult.get(id as usize).copied().unwrap_or(0.0)
}

/// Ammo type `ammo`'s integer base damage (`s_ammo_info+0x1c`), 0 for a type 2.1 doesn't have.
pub fn ammo_damage(ammo: u8) -> i32 {
    data().ammo_damage.get(ammo as usize).copied().unwrap_or(0)
}
