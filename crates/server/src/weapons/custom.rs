//! Custom weapon parameters: `loadweapons <file>` / `saveweapons <file>` (`adjust_weapon_value`
//! 0x9eca4, `save_weapon_value` 0x9f044), the `customweapons` cvar, and `CustomWP`
//! (`SendWeaponParams` 0x9eb98, `InformClientsOnWeaponsParams` 0x9eb14).
//!
//! File format, one line each: `id accuracy viewkick acckick damage cost`, plus
//! `armor_absorb <f>` and `health_absorb <f>` (parsed, unused, as in TS). `//` lines are
//! comments.

use crate::messages::{self, Msg};
use goldsrc::ext::weapons::WeaponsEngine;
use goldsrc::server::engine;
use goldsrc::sys::edict_t;
use goldsrc::util::to_cstring;
use shared::protocol::weapons::custom_wp_string;
use shared::weapons::MAX_WEAPONS;
use shared::weapons::table::{custom_tables_active, set_custom_tables, tables};
use std::fmt::Write;

fn path(file: &str) -> String {
    format!("{}/{}", engine().game_dir(), file)
}

/// Applies a weapons file. Returns false if it can't be read.
pub fn load(file: &str) -> bool {
    let Ok(text) = std::fs::read_to_string(path(file)) else { return false };
    let mut t = tables().clone();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.contains("//") {
            continue;
        }
        if let Some(rest) = line.split("armor_absorb").nth(1).filter(|_| line.contains("armor_absorb")) {
            if let Ok(v) = rest.trim().parse::<f32>() {
                t = t.with_armor_absorb(v);
            }
            continue;
        }
        if line.contains("health_absorb") {
            continue;
        }
        let f: Vec<&str> = line.split_whitespace().collect();
        let parsed = (|| {
            Some((
                f.first()?.parse::<u8>().ok()?,
                f.get(1)?.parse::<f32>().ok()?,
                f.get(2)?.parse::<f32>().ok()?,
                f.get(3)?.parse::<f32>().ok()?,
                f.get(4)?.parse::<f32>().ok()?,
                f.get(5)?.parse::<i32>().ok()?,
            ))
        })();
        let Some((id, acc, vk, ak, dmg, cost)) = parsed else {
            engine().print("You need to run saveweapons again.\n");
            break;
        };
        if let Some(n) = t.with_weapon_params(id, acc, vk, ak, dmg, cost) {
            t = n;
        }
    }
    set_custom_tables(t);
    inform(None);
    true
}

/// Writes the current values in the same format.
pub fn save(file: &str) -> bool {
    let t = tables();
    let mut out = String::from("// id accuracy viewkick acckick damage cost\n");
    for id in 0..MAX_WEAPONS as u8 {
        if let Some(w) = t.weapon(id) {
            let _ = writeln!(out, "{} {:.6} {:.6} {:.6} {:.6} {}", id, w.accuracy, w.view_kick, w.acc_kick, w.damage, w.cost);
        }
    }
    let _ = writeln!(out, "armor_absorb {:.6}", t.combat.armor_absorb);
    let _ = writeln!(out, "health_absorb 0.100000");
    std::fs::write(path(file), out).is_ok()
}

/// `InformClientsOnWeaponsParams`: the notice and `SendWeaponParams` for every weapon, to `to`
/// or everyone.
pub fn inform(to: Option<*mut edict_t>) {
    if !custom_tables_active() {
        return;
    }
    let text = c"This server is using custom weapons.";
    let w = match to {
        Some(e) => messages::one(Msg::TsMessage, e),
        None => messages::all(Msg::TsMessage),
    };
    w.byte(0x2d).byte(0xc3).byte(0x2d).byte(8).byte(100).string(text).end();
    let t = tables();
    for id in 0..MAX_WEAPONS as u8 {
        let Some(w) = t.weapon(id) else { continue };
        let s = to_cstring(&custom_wp_string(id, w.accuracy, w.view_kick, w.acc_kick, w.damage, w.cost));
        match to {
            Some(e) => messages::one(Msg::CustomWp, e).string(&s).end(),
            None => messages::all(Msg::CustomWp).string(&s).end(),
        }
    }
}

pub unsafe extern "C" fn cmd_loadweapons() {
    let e = engine();
    if e.cmd_argc() < 2 {
        e.print("usage: loadweapons <file>\n");
        return;
    }
    let f = e.cmd_argv(1).to_string_lossy().into_owned();
    if !load(&f) {
        e.print(&format!("loadweapons: can't read {f}\n"));
    }
}

pub unsafe extern "C" fn cmd_saveweapons() {
    let e = engine();
    if e.cmd_argc() < 2 {
        e.print("usage: saveweapons <file>\n");
        return;
    }
    let f = e.cmd_argv(1).to_string_lossy().into_owned();
    if !save(&f) {
        e.print(&format!("saveweapons: can't write {f}\n"));
    }
}
