//! TS 2.1's HUD differences from 3.0's that the core asks for: the weapon slot selection, the
//! power-up names and the award titles.
//! Addresses are TS 2.1 `client.dll` VAs.

use ts_client::hud::state::Hud;

/// The slot grid's rows (`< 0x11`).
const ROWS: i32 = 0x11;

/// `SelectSlot(n)` (0x1008c6f0) after the wire panel's keys; returns the server command.
///
/// * Slot 10: `weapon_0` if the menu was closed; the menu closes and the selection is reset to
///   (row 1, bucket 1) every time (3.0: (0, 0), only when it was closed).
/// * Slots 1..5 without `hud_fastswitch` open the menu. The same bucket again walks down from
///   the next row (wrapping to 1), else from row 1 until a weapon (stopping past the grid, at
///   17, when the bucket is empty). A new bucket searches from row 1 (row 0 is never tried) and
///   ends on -1 when the bucket is empty.
/// * With `hud_fastswitch` the search starts at `row + 1`, or at row 0 when switching to bucket 1
///   from another bucket, and isn't wrapped before the first test; it then wraps at 16 to 0 and
///   stops back at the start. The bucket isn't changed, and the menu closes.
///
/// TS reads a row past the grid (17 or 18) from the fields after it and would spin forever if
/// the start is outside 0..=16 and the bucket is empty; the port treats those rows as empty and
/// stops after one round.
pub fn select_slot(h: &mut Hud, n: i32, fastswitch: bool) -> Option<String> {
    if n == 10 {
        let cmd = (!h.menu_open).then(|| "weapon_0".to_owned());
        h.menu_open = false;
        h.menu_row = 1;
        h.menu_bucket = 1;
        return cmd;
    }
    if n >= 6 {
        return None;
    }
    if !fastswitch {
        h.menu_open = true;
        if n == h.menu_bucket {
            let start = if h.menu_row + 1 < ROWS { h.menu_row + 1 } else { 1 };
            match (start..ROWS).find(|&r| h.slot_id(r, h.menu_bucket) != -1) {
                Some(r) => h.menu_row = r,
                None => {
                    h.menu_row = 1;
                    while h.slot_id(h.menu_row, n) == -1 && h.menu_row < ROWS {
                        h.menu_row += 1;
                    }
                }
            }
        } else {
            h.menu_row = (1..ROWS).find(|&r| h.slot_id(r, n) != -1).unwrap_or(-1);
        }
        h.menu_bucket = n;
        return None;
    }
    let start = if n == h.menu_bucket || n != 1 { h.menu_row + 1 } else { 0 };
    let mut row = start;
    for _ in 0..=ROWS {
        if h.slot_id(row, n) != -1 {
            break;
        }
        row += 1;
        if row > ROWS - 1 {
            row = 0;
        }
        if row == start {
            break;
        }
    }
    h.menu_row = row;
    h.menu_open = false;
    let id = h.slot_id(row, n);
    (id > -1).then(|| format!("weapon_{id}"))
}

/// `DrawPowerup` (0x10085c00): the name for `PwUp` flag bit `flag`.
pub fn powerup_label(flag: u32) -> Option<&'static str> {
    match flag {
        1 => Some("Slow-Motion"),
        2 => Some("Infinite-Ammo"),
        8 => Some("Slow-Pause"),
        0x10 => Some("Double-FireRate"),
        _ => None,
    }
}

/// The scoreboard's status title for the awards 2.1 names differently (0x100a4670).
pub fn award_title(award: u8) -> Option<&'static str> {
    match award {
        2 => Some("KILLER"),
        3 => Some("DEMOLITION"),
        _ => None,
    }
}
