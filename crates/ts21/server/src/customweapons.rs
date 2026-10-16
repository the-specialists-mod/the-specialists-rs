//! TS 2.1's custom weapon values: the `customweapons` cvar, which `CTSGameRules::Think` polls
//! (mp.dll 0x10043cc0, 0x100442e0..0x10044320), `adjust_weapon_value` (0x10063ed0) and
//! `InformClientsOnWeaponsParams` (0x10063d90). 2.1 has no `loadweapons`/`saveweapons`
//! commands (its `save_weapon_value` 0x100640e0 is never called), so in 2.1 mode they are unknown
//! commands ([`server_command`]).
//!
//! A file line is `id accuracy viewkick acckick` (`sscanf(" %i %f %f %f ")`); damage, cost and
//! the absorb values aren't 2.1's to change. The values stay until the next map rebuilds the
//! weapon tables, as 3.0's `loadweapons` do (`ts_server::weapons::rebuild_weapon_info`). The
//! clients get 3.0's six-value `CustomWP` (the wire rule; 2.1 sends `"%i %f %f %f"`), with the
//! table's damage and cost.

use goldsrc::server::engine;
use shared::protocol::rules::hud_dest;
use shared::protocol::weapons::custom_wp_string;
use shared::weapons::MAX_WEAPONS;
use shared::weapons::table::{Tables, set_custom_tables, tables};
use ts_server::messages::{self, Msg};
use ts_server::rules::msg;
use ts_server::weapons::custom::{chunks, line_text, path, scan_float, scan_int};

/// `adjust_weapon_value`'s four `sscanf` targets. They live across the lines of one load, and
/// the count `sscanf` returns isn't checked, so a field a line doesn't have keeps the previous
/// line's value (a line with no id re-applies the previous line's). TS starts from whatever is
/// on the stack; the port starts from no id.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Scanned {
    id: i32,
    accuracy: f32,
    view_kick: f32,
    acc_kick: f32,
}

impl Scanned {
    const START: Scanned = Scanned { id: -1, accuracy: 0.0, view_kick: 0.0, acc_kick: 0.0 };

    /// `sscanf(line, " %i %f %f %f ", ...)`: reads fields until one fails, keeping the others.
    fn scan(mut self, line: &str) -> Scanned {
        let Some((id, r)) = scan_int(line) else { return self };
        self.id = id as i32;
        let Some((v, r)) = scan_float(r) else { return self };
        self.accuracy = v;
        let Some((v, r)) = scan_float(r) else { return self };
        self.view_kick = v;
        if let Some((v, _)) = scan_float(r) {
            self.acc_kick = v;
        }
        self
    }
}

/// `adjust_weapon_value`'s loop over a file's contents: every line (after
/// [`line_text`]: not empty, no `//`) is scanned, and an id in `0..64` with a weapon gets the
/// accuracy (`<= 0.001f`, or NaN, becomes 0.001: `fcomps; and ah,0x41`, 0x10064015), the view
/// kick and the accuracy kick (`info+0x28`, `+0x30`, `+0x34`).
fn apply(data: &[u8], from: &Tables) -> Tables {
    let mut t = from.clone();
    let mut s = Scanned::START;
    for raw in chunks(data) {
        let Some(line) = line_text(raw) else { continue };
        s = s.scan(&String::from_utf8_lossy(line));
        if !(0..0x40).contains(&s.id) {
            continue;
        }
        let Some(w) = t.weapon(s.id as u8) else { continue };
        let (damage, cost) = (w.damage, w.cost);
        let accuracy = if s.accuracy.is_nan() || s.accuracy <= 0.001 { 0.001 } else { s.accuracy };
        if let Some(n) = t.with_weapon_params(s.id as u8, accuracy, s.view_kick, s.acc_kick, damage, cost) {
            t = n;
        }
    }
    t
}

/// `adjust_weapon_value` (0x10063ed0): applies `<gamedir>/<file>`, then everyone reads "weapons
/// behaviour changed by <file>". False (and nothing said) when it can't be opened. The flag it
/// sets (0x100b1d98) is read by nothing: 2.1's `InitHUD` tests the rules' `+0x54d8`, which only
/// `CTSGameRules::Init` writes (0), so a joiner isn't told.
fn load(file: &str) -> bool {
    let Ok(data) = std::fs::read(path(file)) else { return false };
    set_custom_tables(apply(&data, tables()));
    msg::text_all(hud_dest::TALK, &format!("weapons behaviour changed by {file}\n"));
    true
}

/// `InformClientsOnWeaponsParams(NULL)` (0x10063d90): "This server is using custom weapons."
/// (`TSHudMessageAll(0x2d, 0xc3, 0x2d, 1, 100, ..)`) and a `CustomWP` per weapon
/// (`SendWeaponParams` 0x10063df0).
fn inform() {
    msg::ts_message_all([0x2d, 0xc3, 0x2d], 1, 100, "This server is using custom weapons.");
    let t = tables();
    for id in 0..MAX_WEAPONS as u8 {
        let Some(w) = t.weapon(id) else { continue };
        let s = goldsrc::util::to_cstring(&custom_wp_string(id, w.accuracy, w.view_kick, w.acc_kick, w.damage, w.cost));
        messages::all(Msg::CustomWp).string(&s).end();
    }
}

/// `CTSGameRules::Think`'s poll (0x100442e0..0x10044320): a non-empty `customweapons` (its first
/// 64 bytes, `strncpy(buf, .., 0x40)`) is loaded, the clients informed if it loaded, and the
/// cvar emptied either way.
pub fn think() {
    let e = engine();
    let value = e.cvar_string(c"customweapons").to_bytes();
    if value.is_empty() {
        return;
    }
    let file = String::from_utf8_lossy(&value[..value.len().min(0x40)]).into_owned();
    if load(&file) {
        inform();
    }
    e.set_cvar_string(c"customweapons", "");
}

/// 3.0's `loadweapons`/`saveweapons` don't exist in 2.1: the engine's answer to an unknown
/// command (FWGS's text; the command is registered for every ruleset, so the port prints it).
pub fn server_command(name: &str) -> bool {
    if !matches!(name, "loadweapons" | "saveweapons") {
        return false;
    }
    engine().print(&format!("Unknown command \"{name}\"\n"));
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::weapons::id;

    #[test]
    fn three_values_and_stale_fields() {
        let s = Scanned::START.scan(" 13 0.5 1.5 2.5 40 2000");
        assert_eq!(s, Scanned { id: 13, accuracy: 0.5, view_kick: 1.5, acc_kick: 2.5 });
        // Missing fields keep the previous line's.
        assert_eq!(s.scan("14 0.25"), Scanned { id: 14, accuracy: 0.25, view_kick: 1.5, acc_kick: 2.5 });
        assert_eq!(s.scan("armor_absorb 0.5"), s, "no id: the previous line again");
        assert_eq!(s.scan("0x0e 1"), Scanned { id: 14, accuracy: 1.0, ..s });
    }

    #[test]
    fn applies_accuracy_and_kicks_only() {
        ts30_gameplay::tables::make_default();
        let base = tables().clone();
        let ak = base.weapon(id::AK47).unwrap().clone();
        let file = format!("// header\n{} 0.0005 3 4\n\n{} 0.2 5 // comment\n99 1 1 1\n", id::AK47, id::M4);
        let t = apply(file.as_bytes(), &base);
        let w = t.weapon(id::AK47).unwrap();
        assert_eq!((w.accuracy, w.view_kick, w.acc_kick), (0.001, 3.0, 4.0), "0.001 floor");
        assert_eq!((w.damage, w.cost), (ak.damage, ak.cost), "damage and cost unchanged");
        assert_eq!(t.weapon(id::M4).unwrap().accuracy, base.weapon(id::M4).unwrap().accuracy, "a `//` line is skipped");
        // Exactly 0.001f is floored too (`<=`), to the same value.
        let t = apply(format!("{} 0.001 0 0\n", id::AK47).as_bytes(), &base);
        assert_eq!(t.weapon(id::AK47).unwrap().accuracy, 0.001);
    }

    #[test]
    fn only_the_two_commands_are_unknown() {
        assert!(!server_command("globalslow"));
        assert!(!server_command("weaponrestriction"));
    }
}
