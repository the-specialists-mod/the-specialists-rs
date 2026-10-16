//! Custom weapon parameters: `loadweapons <file>` / `saveweapons <file>` (`cmd_loadweapons`
//! 0xc59ac, `adjust_weapon_value` 0x9eca4, `cmd_saveweapons` 0xc5a2c, `save_weapon_value`
//! 0x9f044) and `CustomWP` (`InformClientsOnWeaponsParams` 0x9eb14, `SendWeaponParams` 0x9eb98).
//!
//! File format, one line each: `id accuracy viewkick acckick damage cost`, plus
//! `armor_absorb <f>` and `health_absorb <f>`; lines with `//` are comments. The weapon values
//! live in the custom weapon tables until the next map: TS's `W_Precache` rebuilds
//! `global_weapon_info` on every map (`compile_weapons_info` 0x7fe28,
//! [`rebuild_weapon_info`](super::rebuild_weapon_info)). The absorb values and the "custom weapons" flag are game-rules state that
//! `CTSGameRules::Init` resets at the same time (`rules::state`), so the flag and the values
//! agree. TS 3.0 registers the `customweapons` cvar but never reads it (TS 2.1's Think does:
//! the 2.1 plugin), and a ruleset may refuse the two commands (`ServerRules::server_command`).

use crate::messages::{self, Msg};
use crate::rules::{msg, state};
use goldsrc::ext::weapons::WeaponsEngine;
use goldsrc::server::engine;
use goldsrc::sys::{ALERT_TYPE, edict_t};
use goldsrc::util::to_cstring;
use shared::protocol::rules::hud_dest;
use shared::protocol::weapons::custom_wp_string;
use shared::weapons::MAX_WEAPONS;
use shared::weapons::table::{set_custom_tables, tables};
use std::fmt::Write;

/// `"%s%s%s"` of the game directory, `/` and the file name.
pub fn path(file: &str) -> String {
    format!("{}/{}", engine().game_dir(), file)
}

/// One line of a weapons file.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Line {
    /// Empty after trimming, or a `//` comment.
    Skip,
    /// `armor_absorb` / `health_absorb` and the value `sscanf(" %f ")` read after the key, if any.
    ArmorAbsorb(Option<f32>),
    HealthAbsorb(Option<f32>),
    /// `sscanf(" %i %f %f %f %f %i ")` read all six.
    Weapon {
        id: u32,
        accuracy: f32,
        view_kick: f32,
        acc_kick: f32,
        damage: f32,
        cost: i32,
    },
    /// Fewer than six: "You need to run saveweapons again." and the load stops.
    Bad,
}

/// `adjust_weapon_value`'s view of one `fgets` chunk: copied into a 256-byte buffer (the port
/// truncates where TS's `strcpy` would overflow it), up to a NUL, trailing non-graphic
/// characters cut. `None` when nothing is left or the line contains `//` (a comment).
pub fn line_text(raw: &[u8]) -> Option<&[u8]> {
    let mut line = &raw[..raw.len().min(255)];
    if let Some(nul) = line.iter().position(|&b| b == 0) {
        line = &line[..nul];
    }
    while let Some((&last, rest)) = line.split_last()
        && !last.is_ascii_graphic()
    {
        line = rest;
    }
    (!line.is_empty() && find(line, b"//").is_none()).then_some(line)
}

/// The `fgets(buf, 0x400)` chunks of a file: lines, and pieces of 1023 bytes of a longer one.
pub fn chunks(data: &[u8]) -> impl Iterator<Item = &[u8]> {
    data.split_inclusive(|&b| b == b'\n').flat_map(|l| l.chunks(0x3ff))
}

/// [`line_text`], then the `armor_absorb`, `health_absorb` tests and the weapon line, in that
/// order.
fn parse_line(raw: &[u8]) -> Line {
    let Some(line) = line_text(raw) else { return Line::Skip };
    let text = String::from_utf8_lossy(line);
    if let Some(i) = find(line, b"armor_absorb") {
        return Line::ArmorAbsorb(scan_float(&String::from_utf8_lossy(&line[i + 12..])).map(|(v, _)| v));
    }
    if let Some(i) = find(line, b"health_absorb") {
        return Line::HealthAbsorb(scan_float(&String::from_utf8_lossy(&line[i + 13..])).map(|(v, _)| v));
    }
    let scanned = (|| {
        let (id, r) = scan_int(&text)?;
        let (accuracy, r) = scan_float(r)?;
        let (view_kick, r) = scan_float(r)?;
        let (acc_kick, r) = scan_float(r)?;
        let (damage, r) = scan_float(r)?;
        let (cost, _) = scan_int(r)?;
        Some(Line::Weapon { id: id as u32, accuracy, view_kick, acc_kick, damage, cost: cost as i32 })
    })();
    scanned.unwrap_or(Line::Bad)
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// `sscanf`'s `%i`: whitespace, a sign, then hexadecimal after `0x`, octal after `0`, else
/// decimal.
pub fn scan_int(s: &str) -> Option<(i64, &str)> {
    let s = s.trim_start();
    let (neg, body) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let b = body.as_bytes();
    let (radix, start) = if b.len() > 2 && b[0] == b'0' && (b[1] | 0x20) == b'x' && b[2].is_ascii_hexdigit() {
        (16, 2)
    } else if b.first() == Some(&b'0') {
        (8, 0)
    } else {
        (10, 0)
    };
    let digits = body[start..].bytes().take_while(|c| (*c as char).is_digit(radix)).count();
    if digits == 0 {
        return None;
    }
    let end = start + digits;
    let v =
        body[start..end].chars().fold(0i64, |acc, c| acc.wrapping_mul(radix as i64).wrapping_add(c.to_digit(radix).unwrap_or(0) as i64));
    Some((if neg { v.wrapping_neg() } else { v }, &body[end..]))
}

/// `sscanf`'s `%f`: whitespace, then the longest prefix that reads as a number.
pub fn scan_float(s: &str) -> Option<(f32, &str)> {
    let s = s.trim_start();
    let len = s.bytes().take_while(|c| c.is_ascii_digit() || matches!(c, b'+' | b'-' | b'.' | b'e' | b'E')).count();
    (1..=len).rev().find_map(|n| s[..n].parse::<f32>().ok().map(|v| (v, &s[n..])))
}

/// A negative value read for `armor_absorb`/`health_absorb` becomes 0.
fn absorb_value(v: f32) -> f32 {
    if v < 0.0 { 0.0 } else { v }
}

/// `adjust_weapon_value` (0x9eca4): applies a weapons file line by line. False when it can't be
/// opened. A malformed weapon line stops the load (true, but no notice and no flag); otherwise
/// everyone reads "weapons behaviour changed by <file>" and the custom-weapons flag is set.
pub fn load(file: &str) -> bool {
    let Ok(data) = std::fs::read(path(file)) else { return false };
    let mut t = tables().clone();
    for raw in chunks(&data) {
        match parse_line(raw) {
            Line::Skip | Line::ArmorAbsorb(None) | Line::HealthAbsorb(None) => {}
            Line::ArmorAbsorb(Some(v)) => state::with(|g| g.armor_absorb = absorb_value(v)),
            Line::HealthAbsorb(Some(v)) => state::with(|g| g.health_absorb = absorb_value(v)),
            Line::Weapon { id, accuracy, view_kick, acc_kick, damage, cost } => {
                if id < 0x26
                    && let Some(n) = t.with_weapon_params(id as u8, accuracy, view_kick, acc_kick, damage, cost)
                {
                    t = n;
                }
            }
            Line::Bad => {
                // The lines before it already changed the weapons.
                set_custom_tables(t);
                engine().alert(ALERT_TYPE::at_console, "You need to run saveweapons again.\n");
                return true;
            }
        }
    }
    set_custom_tables(t);
    msg::text_all(hud_dest::TALK, &format!("weapons behaviour changed by {file}\n"));
    state::with(|g| g.custom_weapons = true);
    true
}

/// The text `save_weapon_value` writes: TS's header, each weapon's name as a comment and its
/// five values, then `armor_absorb`.
fn saved_text(armor_absorb: f32) -> String {
    let t = tables();
    let mut out = String::from(
        "//===========================================\n//values are : |weapon id|accuracy|view kick|accuracy kick|damage|cost|\n\n//IMPORTANT:\n//-dont change weapon id!\n//-accuracy cant go under 0.001\n//-if you put an accuracy kick of 0 accuracy will not be scaled\n\n//If you want use different weapon settings use the 'customweapons' cvar.\n//Example: 'customweapons weapons_custom.txt'\n//===========================================\n\n",
    );
    for id in 0..MAX_WEAPONS as u8 {
        if let Some(w) = t.weapon(id) {
            let _ =
                write!(out, "//{}\n{} {:.6} {:.6} {:.6} {:.6} {}\n\n", w.name, id, w.accuracy, w.view_kick, w.acc_kick, w.damage, w.cost);
        }
    }
    let _ = write!(out, "//===========================================\n//Misc game variables\narmor_absorb {armor_absorb:.6}\n");
    out
}

/// `save_weapon_value` (0x9f044): writes the current values, then tells everyone. False when the
/// file can't be written.
pub fn save(file: &str) -> bool {
    if std::fs::write(path(file), saved_text(state::armor_absorb())).is_err() {
        return false;
    }
    msg::text_all(hud_dest::TALK, &format!("weapons behaviour saved to {file}\n"));
    true
}

/// `InformClientsOnWeaponsParams` (0x9eb14): the notice and `SendWeaponParams` for every weapon,
/// to `to` or everyone. `loadweapons` sends it to everyone; `InitHUD` to a joiner while the
/// custom-weapons flag is set.
pub fn inform(to: Option<*mut edict_t>) {
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

/// `cmd_loadweapons` (0xc59ac).
pub extern "C" fn cmd_loadweapons() {
    if crate::gameplay::active().server.server_command("loadweapons") {
        return;
    }
    let e = engine();
    if e.cmd_argc() < 2 {
        e.alert(ALERT_TYPE::at_console, "ERROR: specify a file!; example: \"loadweapons weapons.txt\"\n");
        return;
    }
    let f = e.cmd_argv(1).to_string_lossy().into_owned();
    if load(&f) {
        inform(None);
    } else {
        e.alert(ALERT_TYPE::at_console, "ERROR: file doesn't exist!\n");
    }
}

/// `cmd_saveweapons` (0xc5a2c).
pub extern "C" fn cmd_saveweapons() {
    if crate::gameplay::active().server.server_command("saveweapons") {
        return;
    }
    let e = engine();
    if e.cmd_argc() < 2 {
        e.alert(ALERT_TYPE::at_console, "ERROR: specify a file!; example: \"saveweapons weapons.txt\"\n");
        return;
    }
    let f = e.cmd_argv(1).to_string_lossy().into_owned();
    if !save(&f) {
        e.alert(ALERT_TYPE::at_console, "ERROR: unable to save!\n");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weapon_file_lines() {
        assert_eq!(parse_line(b"  \r\n"), Line::Skip);
        assert_eq!(parse_line(b"//AK47\n"), Line::Skip);
        assert_eq!(parse_line(b"13 0.1 0.2 0.3 40 2000 // note\n"), Line::Skip, "any `//` skips the line");
        assert_eq!(parse_line(b"armor_absorb 0.500000\n"), Line::ArmorAbsorb(Some(0.5)));
        assert_eq!(parse_line(b"armor_absorb\n"), Line::ArmorAbsorb(None));
        assert_eq!(parse_line(b"x health_absorb -2\n"), Line::HealthAbsorb(Some(-2.0)));
        assert_eq!(
            parse_line(b"13 0.1 0.2 0.3 40 2000\r\n"),
            Line::Weapon { id: 13, accuracy: 0.1, view_kick: 0.2, acc_kick: 0.3, damage: 40.0, cost: 2000 }
        );
        // `%i` reads hex and octal.
        assert_eq!(
            parse_line(b"0x0d 1 2 3 4 010"),
            Line::Weapon { id: 13, accuracy: 1.0, view_kick: 2.0, acc_kick: 3.0, damage: 4.0, cost: 8 }
        );
        assert_eq!(parse_line(b"13 0.1 0.2"), Line::Bad);
        assert_eq!(absorb_value(-1.0), 0.0);
    }

    #[test]
    fn scanf_prefixes() {
        assert_eq!(scan_int("  -12abc"), Some((-12, "abc")));
        assert_eq!(scan_int("0"), Some((0, "")));
        assert_eq!(scan_int("x"), None);
        assert_eq!(scan_float(" 1.5e1x"), Some((15.0, "x")));
        assert_eq!(scan_float("2."), Some((2.0, "")));
        assert_eq!(scan_float("-"), None);
    }

    #[test]
    fn saved_file_reads_back() {
        ts30_gameplay::tables::make_default();
        let text = saved_text(0.2125);
        assert!(text.ends_with("//Misc game variables\narmor_absorb 0.212500\n"));
        let t = tables();
        let mut weapons = 0;
        for line in text.split_inclusive('\n') {
            match parse_line(line.as_bytes()) {
                Line::Weapon { id, damage, .. } => {
                    weapons += 1;
                    assert_eq!(t.weapon(id as u8).unwrap().damage, damage);
                }
                Line::ArmorAbsorb(v) => assert_eq!(v, Some(0.2125)),
                Line::Skip => {}
                other => panic!("{other:?} in {line:?}"),
            }
        }
        assert_eq!(weapons, t.weapons().count());
    }
}
