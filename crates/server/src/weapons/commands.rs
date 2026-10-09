//! Weapon client commands (`ClientCommand` 0xad838; weapons.md §7.1, architecture.md §4).

use super::host::with_host;
use super::inventory::{self, Drop};
use super::{ArmamentEntry, net};
use crate::commands::Args;
use crate::messages::{self, Msg};
use crate::player::Player;
use crate::world::Cx;
use goldsrc::server::engine;
use shared::weapons::{MAX_AMMO, info};

/// `sscanf("%i")`-like: optional sign, decimal / 0x hex / leading-0 octal, stops at the first
/// non-digit. `None` if no digits.
fn scan_int(s: &str) -> Option<(i32, &str)> {
    let s = s.trim_start();
    let (neg, rest) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let (radix, digits) = if rest.starts_with("0x") || rest.starts_with("0X") {
        (16, &rest[2..])
    } else if rest.len() > 1 && rest.starts_with('0') {
        (8, &rest[1..])
    } else {
        (10, rest)
    };
    let end = digits.find(|c: char| !c.is_digit(radix)).unwrap_or(digits.len());
    if end == 0 {
        return if radix == 8 { Some((0, &rest[1..])) } else { None };
    }
    let v = i64::from_str_radix(&digits[..end], radix).ok()? as i32;
    Some((if neg { -v } else { v }, &digits[end..]))
}

/// `sscanf("%i_%i")`. The second value defaults to 0 when missing.
fn scan_pair(s: &str) -> Option<(i32, i32)> {
    let (a, rest) = scan_int(s)?;
    let b = rest.strip_prefix('_').and_then(scan_int).map(|(b, _)| b).unwrap_or(0);
    Some((a, b))
}

fn add_armament(p: &mut Player, e: ArmamentEntry) {
    if p.inv.sv.armament.len() < 32 {
        p.inv.sv.armament.push(e);
    }
}

pub fn client_command(p: &mut Player, cx: &mut Cx, cmd: &str, args: &Args) -> bool {
    if let Some(n) = cmd.strip_prefix("weapon_") {
        // weapon_<id>: select (remembers lastinv)
        let Some((id, _)) = scan_int(n) else { return true };
        p.inv.sv.last_inv = p.inv.gun.current;
        if (0..38).contains(&id) {
            with_host(p, cx, |gun, h| gun.switch_to(id as u8, h));
        }
        return true;
    }
    if let Some(n) = cmd.strip_prefix("lasclr_") {
        let v = scan_int(n).map(|(v, _)| v).unwrap_or(0);
        let mut w = messages::all(Msg::PlInfo);
        w = w.byte(cx.index()).byte(0).byte(v);
        w.end();
        return true;
    }
    if let Some(n) = cmd.strip_prefix("drop_") {
        let id = scan_int(n).map(|(v, _)| v).unwrap_or(-1);
        if (0..38).contains(&id) {
            inventory::drop_weapon(p, cx, Drop::Id(id as u8));
        } else {
            inventory::drop_weapon(p, cx, Drop::Current);
        }
        return true;
    }
    match cmd {
        "drop" => {
            inventory::drop_weapon(p, cx, Drop::Current);
            true
        }
        "rst_cfg" => {
            p.inv.sv.armament.clear();
            p.inv.sv.armament_ammo = [0; MAX_AMMO];
            true
        }
        "tkw" => {
            for i in 1..args.argc() {
                let Some((id, n)) = scan_pair(args.str(i)) else { continue };
                let stack = (0..38).contains(&id) && info(id as u8).map(|w| w.stack).unwrap_or(false);
                if !stack {
                    add_armament(p, ArmamentEntry { id, attachments: n as u32, item: false });
                } else {
                    let max = info(id as u8).map(|w| w.pellets).unwrap_or(1);
                    let count = if max < 2 { 1 } else { n.clamp(1, max) };
                    for _ in 0..count {
                        add_armament(p, ArmamentEntry { id, attachments: 0, item: false });
                    }
                }
            }
            true
        }
        "tki" => {
            for i in 1..args.argc() {
                if let Some((id, _)) = scan_int(args.str(i)) {
                    add_armament(p, ArmamentEntry { id, attachments: 0, item: true });
                }
            }
            true
        }
        "tka" => {
            for i in 1..args.argc() {
                if let Some((t, n)) = scan_pair(args.str(i)) {
                    if (0..MAX_AMMO as i32).contains(&t) {
                        p.inv.sv.armament_ammo[t as usize] = n;
                    }
                }
            }
            true
        }
        "firemode" => {
            p.inv.gun.switch_fire_mode();
            true
        }
        "useitems" => {
            with_host(p, cx, |gun, h| gun.switch_items(h));
            true
        }
        "lastinv" => {
            if p.inv.gun.current == 0 {
                return true;
            }
            let target = p.inv.sv.last_inv;
            p.inv.sv.last_inv = p.inv.gun.current;
            with_host(p, cx, |gun, h| gun.switch_to(target, h));
            true
        }
        // Not TS: console helper to arm yourself on servers with sv_cheats (testing).
        "ts_give" => {
            if engine().cvar_float(c"sv_cheats") == 0.0 {
                return true;
            }
            for i in 1..args.argc() {
                if let Some((id, att)) = scan_pair(args.str(i)) {
                    if (1..38).contains(&id) {
                        super::give_weapon(p, cx, id as u8, att as u8);
                        if let Some(w) = info(id as u8) {
                            super::give_ammo(p, cx, w.ammo, 999);
                        }
                    }
                }
            }
            net::force_update(p);
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scanf_like_parsing() {
        assert_eq!(scan_pair("13_8"), Some((13, 8)));
        assert_eq!(scan_pair("24"), Some((24, 0)));
        assert_eq!(scan_pair("0x10_1"), Some((16, 1)));
        assert_eq!(scan_pair("x"), None);
        assert_eq!(scan_int("12abc"), Some((12, "abc")));
        assert_eq!(scan_int("0"), Some((0, "")));
    }
}
