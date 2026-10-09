//! Weapon, ammo and combat tables, decoded from TS 3.0 (data/*.toml) and loaded identically on
//! server and client. The predicted weapon state machine lives here too (see docs/design.md).

pub mod table;

pub use table::{AmmoInfo, Asset, Combat, FireMode, Tables, WeaponInfo, WeaponKind, tables};

/// Weapon ids are the TS ids (0 = kung fu .. 37); 29 is the briefcase and has no table entry.
pub const MAX_WEAPONS: usize = 38;
