//! `CHudTSWeapons` (0x100d0ca0): weapon info, weapon list / slot menu, kung-fu bar, crosshair,
//! wire panel, slot selection and the Think sounds. Reference: architecture-hud.md §4.2.

use super::Dc;
use super::draw::{RED, half, num};
use super::state::{GRID_ROWS, Hud};
use super::text::localise;
use crate::input;
use goldsrc::client::engine;
use goldsrc::ext::hud::SND_CHANGE_VOL;
use shared::weapons::tables;
use std::cell::Cell;

// ---------------------------------------------------------------------------------------------
// Slot selection (pure)

fn slot_id(h: &Hud, row: i32, bucket: i32) -> i32 {
    h.grid.get(row as usize).and_then(|r| r.get(bucket as usize)).map_or(-1, |s| s.id)
}

/// `SelectSlot(n)` (1004f220) after the text-menu and buy-menu checks. Returns a server command.
pub fn select_slot(h: &mut Hud, n: i32, fastswitch: bool) -> Option<String> {
    if h.wires.iter().any(|&w| w != 0) {
        return (1..=3).contains(&n).then(|| format!("cutwire_{n}"));
    }
    if n == 10 {
        let cmd = (!h.menu_open).then(|| {
            h.menu_row = 0;
            h.menu_bucket = 0;
            "weapon_0".to_owned()
        });
        h.menu_open = false;
        return cmd;
    }
    if !(1..6).contains(&n) {
        return None;
    }
    let last = GRID_ROWS as i32 - 1;
    if !fastswitch {
        h.menu_open = true;
        if n == h.menu_bucket {
            // Cycle down the bucket.
            let mut row = h.menu_row + 1;
            if row > last {
                row = 1;
            }
            let found = (row..=last).find(|&r| slot_id(h, r, n) != -1);
            match found {
                Some(r) => h.menu_row = r,
                None => {
                    h.menu_row = 1;
                    if slot_id(h, 1, n) == -1
                        && let Some(r) = (2..=last).find(|&r| slot_id(h, r, n) != -1)
                    {
                        h.menu_row = r;
                    }
                }
            }
        } else {
            h.menu_row = 0;
            if slot_id(h, 0, n) == -1 {
                h.menu_row = (1..=last).find(|&r| slot_id(h, r, n) != -1).unwrap_or(1);
            }
        }
        h.menu_bucket = n;
        return None;
    }
    // hud_fastswitch: pick the next weapon in the bucket and select it at once.
    let start = if h.menu_row + 1 > last { 0 } else { h.menu_row + 1 };
    let mut row = start;
    loop {
        if slot_id(h, row, n) != -1 {
            break;
        }
        row = if row + 1 > last { 0 } else { row + 1 };
        if row == start {
            break;
        }
    }
    h.menu_row = row;
    h.menu_open = false;
    let id = slot_id(h, row, n);
    (id >= 0).then(|| format!("weapon_{id}"))
}

/// `SelectNextPrev(dir)` (1004f500): walk the grid, skipping single guns whose akimbo version
/// is owned. With fast switch the weapon is selected immediately.
pub fn select_next_prev(h: &mut Hud, next: bool, fastswitch: bool) -> Option<String> {
    let last = GRID_ROWS as i32 - 1;
    if !h.menu_open {
        if let Some((r, b)) = super::state::grid_pos(h.weapon) {
            h.menu_row = r as i32;
            h.menu_bucket = b as i32;
        }
        h.menu_open = true;
    }
    if !(1..=5).contains(&h.menu_bucket) {
        h.menu_bucket = 1;
    }
    if !(0..=last).contains(&h.menu_row) {
        h.menu_row = 0;
    }
    let step = |row: &mut i32, b: &mut i32, first_bucket: &mut i32| {
        if next {
            *row += 1;
            if *row > last {
                *row = 0;
                if *first_bucket == -1 {
                    *first_bucket = *b;
                }
                *b = if *b + 1 > 5 { 1 } else { *b + 1 };
            }
        } else {
            *row -= 1;
            if *row < 0 {
                *row = last;
                if *first_bucket == -1 {
                    *first_bucket = *b;
                }
                *b = if *b - 1 < 1 { 5 } else { *b - 1 };
            }
        }
    };
    let (orig_row, mut first_bucket) = (h.menu_row, -1);
    let mut row = if next {
        if h.menu_row + 1 > last { 0 } else { h.menu_row + 1 }
    } else if h.menu_row - 1 < 0 {
        last
    } else {
        h.menu_row - 1
    };
    let mut b = h.menu_bucket;
    loop {
        let id = slot_id(h, row, b);
        if id > 0 {
            let akimbo = tables().weapon(id as u8).map_or(-1, |w| w.akimbo);
            if akimbo == -1 {
                break;
            }
            let owned_akimbo = super::state::grid_pos(akimbo).is_some_and(|(r, c)| h.grid[r][c].id >= 0);
            if !owned_akimbo {
                break;
            }
        }
        step(&mut row, &mut b, &mut first_bucket);
        if b == first_bucket && row == orig_row {
            break;
        }
    }
    h.menu_bucket = b;
    h.menu_row = row;
    if fastswitch {
        h.menu_open = false;
        let id = slot_id(h, row, b);
        return (id > 0).then(|| format!("weapon_{id}"));
    }
    None
}

// ---------------------------------------------------------------------------------------------
// Think sounds

/// Heartbeat (slow motion) / breathing (low kung-fu power) state: 0 idle, 2 playing, 1 fading.
static HEART: goldsrc::Global<Cell<(i32, f32)>> = goldsrc::Global(Cell::new((0, 0.0)));
static BREATH: goldsrc::Global<Cell<(i32, f32)>> = goldsrc::Global(Cell::new((0, 0.0)));

/// `Think` (1004ce10) sound part: `sound/player/heartbeat1.wav` while the time scale is below 1,
/// `sound/player/breathe2.wav` while kung-fu power is below 50; both fade out when the cause
/// ends (volume −0.01 / −0.005 per frame with `SND_CHANGE_VOL`).
pub fn think_sounds(_dt: f32) {
    let e = engine();
    let Some(me) = e.local_player() else { return };
    let (ts, power) = (crate::state::with(|s| s.time_scale), super::with(|h| h.kfu_power));
    let origin = me.curstate.origin;
    let run = |cell: &Cell<(i32, f32)>, active: bool, start_vol: f32, fade: f32, ch: i32, sample: &str| {
        let (mut st, mut vol) = cell.get();
        if !active {
            if st == 2 {
                st = 1;
            }
        } else if st == 0 || st == 1 {
            vol = start_vol;
            e.ev_play_sound(me.index, origin, ch, sample, vol, 0.8, if st == 1 { SND_CHANGE_VOL } else { 0 }, 100);
            st = 2;
        }
        if st == 1 {
            vol -= fade;
            if vol <= 0.0 {
                st = 0;
                e.ev_play_sound(me.index, origin, ch, sample, 0.0, 0.8, SND_CHANGE_VOL, 100);
            } else {
                e.ev_play_sound(me.index, origin, ch, sample, vol, 0.8, SND_CHANGE_VOL, 100);
            }
        }
        cell.set((st, vol));
    };
    run(&HEART.0, ts < 1.0, 1.0, 0.01, 501, "player/heartbeat1.wav");
    run(&BREATH.0, power < 50.0, 0.7, 0.005, 502, "player/breathe2.wav");
}

// ---------------------------------------------------------------------------------------------
// Drawing

/// `CHudTSWeapons::Draw` (1004d3f0) minus the buy menu (drawn by `menus`).
pub fn draw(dc: &Dc, nohud: bool) {
    let hidden = super::dead_or_observer();
    let items = super::with(|h| items_flags(h, nohud, hidden));
    draw_laser_dots(dc, items);
    if nohud || hidden {
        return;
    }
    let buy_open = crate::menus::buy_open();
    super::with(|h| {
        let demo = engine().demo_playing();
        draw_weapon_list(dc, h, !h.menu_open || demo);
        if h.weapon == 0 {
            draw_kungfu_label(dc, h);
        } else {
            draw_weapon_info(dc, h);
        }
        if buy_open {
            return;
        }
        if h.wires.iter().any(|&w| w != 0) {
            draw_wire_panel(dc, h);
        } else {
            draw_crosshair(dc, h);
        }
    });
}

/// `CHudTSWeapons+0x14` as `Draw` sets it: the local player's `ActItems` bits while a weapon is
/// held and the HUD draws it (with `nohud`, whenever a weapon is held), otherwise 0. TS sets it
/// after drawing the dots, so its dots use last frame's value; this uses the current one.
fn items_flags(h: &Hud, nohud: bool, hidden: bool) -> i32 {
    if h.weapon == 0 || (!nohud && hidden) {
        return 0;
    }
    crate::state::with(|s| s.active_items.get(engine().local_player().map_or(0, |p| p.index as usize)).copied().unwrap_or(0)) as i32
}

/// The crosshair centre: the screen centre, or in third person the projection of the view
/// trace's end (`DrawCrosshairID` 1004acc0 stores it at `+0x38`), (−1, −1) when off screen.
fn crosshair_centre(dc: &Dc) -> (i32, i32) {
    if !crate::view::is_third_person() {
        return (dc.w() / 2, dc.h() / 2);
    }
    let e = engine();
    let Some(me) = e.local_player() else { return (-1, -1) };
    let start = crate::view::eye();
    let end = goldsrc::ext::render::vec3::ma(start, 8192.0, crate::view::camera().forward);
    e.crosshair_trace(start, end, me.index).and_then(|(_, hit, _)| e.world_to_screen(hit, &dc.g.screen)).unwrap_or((-1, -1))
}

/// `DrawLaserDots` (1004e6f0): 2×2 dots in the local laser colour (`PlInfo`). While the gun may
/// fire (`state.items_ready`) and `cl_laser` is below 1, the dot sits on the crosshair (akimbo:
/// 4 px right and left of it) with a rare ±1 px jitter; otherwise it is drawn where the trace
/// hit (`state.laser_dots`). A dot whose hit point is off screen is not drawn.
fn draw_laser_dots(dc: &Dc, items: i32) {
    use goldsrc::ext::render::random_long;
    let local = engine().local_player().map_or(0, |p| p.index as usize);
    let (dots, ready, color) = crate::state::with(|s| (s.laser_dots, s.items_ready, s.laser_color.get(local).copied().unwrap_or(0)));
    let rgb = crate::effects::laser_color(color as i32).map(|c| (c * 255.0).round() as i32);
    // C integer division: ±1 only for ±10.
    let (jx, jy) = (random_long(-10, 10) / 10, random_long(-10, 10) / 10);
    let centre = crosshair_centre(dc);
    let centred = ready != 0 && input::cvars().cl_laser.value() < 1.0;
    for (x, y) in laser_dot_positions(items, centred, centre, (jx, jy), dots, (dc.w(), dc.h())) {
        dc.fill(x, y, 2, 2, rgb, 255);
    }
}

/// Top-left corners of the `DrawLaserDots` fills (pure, for tests). `dots` are `WorldToScreen`
/// coordinates; `centred` pins them to `centre` (plus `jitter`, swapped for the second gun).
fn laser_dot_positions(
    items: i32,
    centred: bool,
    (cx, cy): (i32, i32),
    (jx, jy): (i32, i32),
    dots: [Option<[f32; 2]>; 2],
    (w, h): (i32, i32),
) -> Vec<(i32, i32)> {
    if items & 2 == 0 {
        return Vec::new();
    }
    let to_px = |d: [f32; 2]| (((1.0 + d[0]) * w as f32 * 0.5) as i32, ((1.0 - d[1]) * h as f32 * 0.5) as i32);
    let pinned: &[(i32, i32)] = if items & 0x40 == 0 { &[(cx + jx, cy + jy)] } else { &[(cx + 4 + jx, cy + jy), (cx - 4 + jy, cy + jx)] };
    pinned.iter().zip(dots).filter_map(|(&p, d)| d.map(|d| if centred { p } else { to_px(d) })).map(|(x, y)| (x - 1, y - 1)).collect()
}

fn realhud() -> bool {
    input::cvars().cl_realhud.value() != 0.0
}

/// `DrawKungFuBar` (1004d5f0): 128×19 frame plus a fill proportional to `KFuPower`.
fn draw_kungfu_bar(dc: &Dc, h: &Hud, x: i32, y: i32) {
    let mut rgb = dc.hud;
    if h.kfu_power <= 0.01 {
        rgb = RED;
    } else {
        let p = h.kfu_power as i32;
        let w = p * 124 / 100;
        dc.fill(x + 2, y + 2, w.max(0), 16, [rgb[0] * 128 / 255, p * rgb[1] / 255, p * rgb[2] / 255], 255 - p);
    }
    dc.fill(x, y, 128, 19, rgb, 30);
}

/// `DrawKungFuLabel` (1004d700).
fn draw_kungfu_label(dc: &Dc, h: &Hud) {
    let (w, ht) = (dc.w(), dc.h());
    let x = w - 136;
    draw_kungfu_bar(dc, h, x, ht - 28);
    dc.fill(x, ht - 48, 128, 19, dc.hud, 35);
    dc.string(w - 134, ht - 48, "Kung-Fu", dc.hud);
}

const FIREMODE_LABELS: [&str; 6] = ["#HUD_FullAuto", "#HUD_SemiAuto", "#HUD_Burst", "#HUD_PumpAction", "#HUD_FreeSemi", "#HUD_FreeFull"];
const ATTACHMENT_LABELS: [&str; 4] = ["#HUD_Silencer", "#HUD_Lasersight", "#HUD_Flashlight", "#HUD_Scope"];

/// `DrawWeaponInfo` (1004d7e0): bottom-right stack of 128×19 rows, bottom-up: owned attachments,
/// clip/reserve (or the kung-fu bar for melee), fire mode, ammo name, weapon name.
fn draw_weapon_info(dc: &Dc, h: &Hud) {
    if realhud() {
        return;
    }
    let Some(info) = tables().weapon(h.weapon as u8) else { return };
    let x = dc.w() - 136;
    let mut y = dc.h() - 8;
    let row = 20;
    let active =
        crate::state::with(|s| s.active_items.get(engine().local_player().map_or(0, |p| p.index as usize)).copied().unwrap_or(0)) as u32;
    for (bit, label) in ATTACHMENT_LABELS.iter().enumerate() {
        let m = 1u32 << bit;
        if h.attachments & m == 0 {
            continue;
        }
        y -= row;
        let on = active & m != 0;
        dc.fill(x, y, 128, row - 1, dc.hud, if on { 45 } else { 15 });
        dc.string(x + 2, y, &localise(label), half(dc.hud));
        dc.spr(dc.g.ts.item_installed, x + 0x70, y, if on { dc.hud } else { half(dc.hud) });
    }
    y -= row;
    dc.fill(x, y, 128, row - 1, dc.hud, 15);
    // The server sends WeaponInfo only when the weapon or the reserve changes (CTSGun::
    // UpdateClientData); the clip, reserve and fire mode in between come from the predicted
    // weapon state (prediction writes them every command).
    let (clip, reserve, firemode) = crate::state::with(|s| {
        if s.predicted_weapon == Some(h.weapon) {
            (s.clip, s.reserve_ammo, s.firemode)
        } else {
            (h.clip, h.reserve[info.ammo as usize & 15], h.firemode as i32)
        }
    });
    if info.ammo == 0 {
        draw_kungfu_bar(dc, h, x, y);
        y -= row;
        if info.stack {
            dc.number(x + 2, y, num::DIGITS_3 | num::DRAW_ZERO, clip, dc.hud);
        }
    } else {
        dc.number(x + 2, y, num::DIGITS_3 | num::DRAW_ZERO, clip, dc.hud);
        dc.string(x + 0x40, y, "|", [dc.hud[0] / 4, dc.hud[1] / 4, dc.hud[2] / 4]);
        dc.number(x + 0x48, y, num::DIGITS_3 | num::DRAW_ZERO, reserve, half(dc.hud));
        y -= row;
        dc.fill(x, y, 128, row - 1, dc.hud, 15);
        let label = FIREMODE_LABELS.get(firemode as usize).copied().unwrap_or(FIREMODE_LABELS[0]);
        dc.string(x + 2, y, &localise(label), dc.hud);
        y -= row;
        dc.fill(x, y, 128, row - 1, dc.hud, 15);
        let ammo = tables().ammo(info.ammo).map_or("", |a| a.name.as_str());
        dc.string(x + 2, y, ammo, half(dc.hud));
    }
    y -= row;
    dc.fill(x, y, 128, row - 1, dc.hud, 35);
    dc.string(x + 2, y, &info.name, dc.hud);
    // Zoom factor while scoped: `sprintf("%ix", 80 / m_iFOV)` at (24, 24) (1004dfbe).
    let fov = crate::state::with(|s| s.fov) as i32;
    if (1..40).contains(&fov) {
        dc.string(24, 24, &format!("{}x", 80 / fov), dc.hud);
    }
}

/// `DrawWeaponList` (1004e030): the slot menu (header row per bucket, weapon sprites, name and
/// `clip/reserve` of the selection, clip bar) or, with the menu closed, the equipment list when
/// `cl_showequipment` is 1..3.
fn draw_weapon_list(dc: &Dc, h: &Hud, closed: bool) {
    if closed {
        let v = input::cvars().cl_showequipment.value() as i32;
        if !(1..=3).contains(&v) {
            return;
        }
    }
    let t = tables();
    for bucket in 1..=5i32 {
        let x = bucket * 128 - 120;
        if !closed {
            let sel = bucket == h.menu_bucket;
            dc.fill(x, 8, 128, 20, dc.hud, if sel { 100 } else { 50 });
            dc.number(x, 8, num::DIGITS_2, bucket, half(dc.hud));
        }
        let mut y = 28;
        for row in 0..GRID_ROWS {
            let slot = h.grid[row][bucket as usize];
            if slot.id == -1 {
                continue;
            }
            if closed && slot.id == 0 {
                continue;
            }
            let id = slot.id;
            if !closed
                && row as i32 == h.menu_row
                && bucket == h.menu_bucket
                && let Some(info) = t.weapon(id as u8)
            {
                dc.string(x + 128, y, &info.name, dc.hud);
                let ammo = match id {
                    24 | 25 | 35 => Some(format!("{}", slot.clip)),
                    0 | 29 | 34 => None,
                    _ => Some(format!("{}/{}", slot.clip, h.reserve[info.ammo as usize & 15])),
                };
                if let Some(a) = ammo {
                    dc.string(x + 128, y + 16, &a, half(dc.hud));
                }
            }
            let rgb = if slot.clip < 1 && id != 0 { RED } else { dc.hud };
            if let Some(&spr) = dc.g.ts.weapons.get(id as usize) {
                dc.spr(spr, x, y, if !closed && row as i32 == h.menu_row && bucket == h.menu_bucket { rgb } else { half(rgb) });
            }
            // Clip bar: 5 px wide at x+123, 44 px tall.
            if !matches!(id, 0 | 29 | 34)
                && let Some(info) = t.weapon(id as u8)
            {
                let max = if info.stack { info.pellets } else { info.clip };
                if max > 0 {
                    let filled = (slot.clip.max(0) * 44 / max).clamp(0, 44);
                    dc.fill(x + 0x7b, y + 46 - filled, 5, filled, dc.hud, 100);
                    if slot.clip < max {
                        dc.fill(x + 0x7b, y + 2, 5, 44 - filled, RED, 50);
                    }
                }
            }
            y += 48;
        }
    }
}

/// `DrawWirePanel` (1004d550): three wire sprites 32 px apart above the crosshair.
fn draw_wire_panel(dc: &Dc, h: &Hud) {
    let e = engine();
    let (mut x, y) = (dc.w() / 2 - 48, dc.h() / 2 - 128);
    for i in 0..3 {
        let spr = if h.wires[i] != 0 { dc.g.ts.wires[i] } else { dc.g.ts.wires_cut[i] };
        if spr != 0 {
            e.spr_set(spr, [255; 3]);
            e.spr_draw_holes(0, x, y, None);
        }
        x += 32;
    }
}

/// The scoped flag `DrawCrosshair` sets (`0x1018a704`): FOV below 40, cleared in third person, so
/// the third-person crosshair keeps its lines, divesteps and unraised laser number while zoomed.
fn crosshair_scoped(fov: f32, third_person: bool) -> bool {
    (fov as i32) < 40 && !third_person
}

/// `DrawCrosshair` (1004e9f0) + divestep helper (1004ef50). Everything is drawn around
/// [`crosshair_centre`]: the screen centre, or the projected view-trace end in third person.
fn draw_crosshair(dc: &Dc, h: &Hud) {
    if realhud() {
        return;
    }
    let e = engine();
    let scoped = crosshair_scoped(crate::state::with(|s| s.fov), crate::view::is_third_person());
    let (cx, cy) = crosshair_centre(dc);
    if !scoped {
        draw_divesteps(dc, h, cx, cy);
    }
    if matches!(h.weapon, 0 | 24 | 25 | 34 | 35) {
        return;
    }
    let id_team = h.crosshair_id.map(|(_, t)| t);
    // 1 = teammate under the crosshair (green, "don't shoot" sprite), 2 = enemy (red).
    let rgb = match id_team {
        Some(1) => [45, 195, 45],
        Some(2) => RED,
        _ => dc.hud,
    };
    let active = crate::state::with(|s| s.active_items.get(e.local_player().map_or(0, |p| p.index as usize)).copied().unwrap_or(0));
    if !scoped || id_team == Some(1) {
        let base = if active & 2 != 0 { 12 } else { 22 };
        let spread = crate::state::with(|s| s.spread);
        let gap = (base as f32 + spread * 160.0) as i32;
        if id_team == Some(1) {
            dc.spr(dc.g.ts.cross_t, cx - 32, cy - 32, rgb);
        } else {
            dc.fill(cx - gap, cy - 1, 5, 2, rgb, 75);
            dc.fill(cx + gap - 5, cy - 1, 5, 2, rgb, 75);
            dc.fill(cx - 1, cy - gap, 2, 5, rgb, 75);
            dc.fill(cx - 1, cy + gap - 5, 2, 5, rgb, 75);
            let solid = input::cvars().cl_solidcross.value();
            if solid > 0.0 && dc.g.ts.cross2 != 0 {
                e.spr_set(dc.g.ts.cross2, if solid > 1.0 { [0; 3] } else { rgb });
                e.spr_draw_holes(0, cx - 32, cy - 32, None);
            } else {
                dc.spr(dc.g.ts.cross_e, cx - 32, cy - 32, rgb);
            }
        }
    }
    // Laser on: its range (`state.laser_range`) right of the crosshair, in the crosshair colour
    // ×0.7 with a player under it, else ×0.5; 8 px higher while scoped (1004ee02).
    if active & 2 != 0 {
        let k = if id_team.is_some() { 0.7 } else { 0.5 };
        let y = cy - if scoped { 8 } else { 0 } - dc.g.screen.char_height / 2;
        let range = crate::state::with(|s| s.laser_range);
        dc.number(cx + 28, y, num::DRAW_ZERO, range, rgb.map(|c| (c as f32 * k) as i32));
    }
}

/// Divestep sprites around the crosshair (1004ef50): top/centre/bottom dive indicators plus a
/// pitch dot. See [`divestep_layout`].
fn draw_divesteps(dc: &Dc, h: &Hud, cx: i32, cy: i32) {
    let (sprites, (dx, dy), dot) = divestep_layout(h.next_stunt >= 0.01, h.slots, engine().view_angles()[0], dc.hud, (cx, cy));
    for (i, rgb) in sprites.into_iter().enumerate() {
        dc.spr(dc.g.ts.divestep[i], cx - 40, cy - 40, rgb);
    }
    dc.fill(dx, dy, 2, 2, dot, 100);
}

/// Colours of the three divestep sprites, and the pitch dot's position and colour (pure, from the
/// TS binary at 1004ef50 and the threshold helpers 1004eec0/1004eef0/1004ef20).
///
/// Sprite `i` is red when the load is too heavy for that dive (`81 − free slots` ≥ 42, 52, 62).
/// Otherwise it has the base colour (red while the stunt cooldown runs, else the HUD colour), at
/// full strength for the band the view pitch points at (< −10°, −10..10°, > 10°) and halved for
/// the other two. The dot sits 32 px from the centre at angle `clamp(3·pitch, ±90°)`, on the left:
/// `(cx − 32·cos a, cy + 32·sin a)`, in the base colour.
fn divestep_layout(
    cooldown: bool,
    free_slots: u8,
    pitch: f32,
    hud: [i32; 3],
    (cx, cy): (i32, i32),
) -> ([[i32; 3]; 3], (i32, i32), [i32; 3]) {
    let base = if cooldown { RED } else { hud };
    let used = 81 - free_slots as i32;
    let band = if pitch < -10.0 {
        0
    } else if pitch > 10.0 {
        2
    } else {
        1
    };
    let limits = [42, 52, 62];
    let sprites = [0, 1, 2].map(|i| {
        if used >= limits[i] {
            RED
        } else if i == band {
            base
        } else {
            half(base)
        }
    });
    let a = (pitch * 3.0).clamp(-90.0, 90.0).to_radians();
    let dot = ((cx as f32 - a.cos() * 32.0) as i32, (a.sin() * 32.0 + cy as f32) as i32);
    (sprites, dot, base)
}

#[cfg(test)]
mod tests {
    use super::super::state::{Slot, grid_pos};
    use super::*;

    fn give(h: &mut Hud, id: i32) {
        let (r, b) = grid_pos(id).unwrap();
        h.grid[r][b] = Slot { id, clip: 5 };
    }

    #[test]
    fn crosshair_scoped_flag() {
        assert!(crosshair_scoped(20.0, false));
        assert!(!crosshair_scoped(40.0, false), "40 is not scoped");
        assert!(crosshair_scoped(39.9, false), "the FOV is truncated, as TS's int");
        assert!(!crosshair_scoped(20.0, true), "third person clears it");
        assert!(!crosshair_scoped(90.0, false));
    }

    #[test]
    fn divesteps_follow_ts_binary() {
        let hud = [100, 200, 60];
        let c = (320, 240);
        // Level view, light load: centre band full, the others halved; dot 32 px left.
        let (s, dot, base) = divestep_layout(false, 70, 0.0, hud, c);
        assert_eq!(s, [half(hud), hud, half(hud)]);
        assert_eq!((dot, base), ((288, 240), hud));
        // Looking down 30° (clamped to 90°): bottom band, dot straight below.
        let (s, dot, _) = divestep_layout(false, 70, 30.0, hud, c);
        assert_eq!((s[2], dot), (hud, (320, 272)));
        // 81 - 25 = 56 used: top and centre dives too heavy; cooldown turns the rest red.
        let (s, _, base) = divestep_layout(true, 25, -20.0, hud, c);
        assert_eq!((s, base), ([RED, RED, half(RED)], RED));
    }

    #[test]
    fn laser_dots_follow_ts_layout() {
        let dots = [Some([0.5, 0.5]), Some([-0.5, 0.0])];
        let (c, j, sz) = ((320, 240), (1, 0), (640, 480));
        assert_eq!(laser_dot_positions(0, true, c, j, dots, sz), [], "laser off");
        assert_eq!(laser_dot_positions(2, true, c, j, dots, sz), [(320, 239)], "pinned to the crosshair, jitter x");
        assert_eq!(laser_dot_positions(2, false, c, j, dots, sz), [(479, 119)], "at the projected hit");
        assert_eq!(laser_dot_positions(0x42, true, c, j, dots, sz), [(324, 239), (315, 240)], "akimbo: +3/-5 px, jitter swapped");
        assert_eq!(laser_dot_positions(0x42, true, c, j, [None, dots[1]], sz), [(315, 240)], "off-screen hit: no dot");
    }

    #[test]
    fn slot_menu_cycles() {
        let mut h = Hud::default();
        give(&mut h, 1); // glock [2,1]
        give(&mut h, 12); // deagle [2,3]
        assert_eq!(select_slot(&mut h, 2, false), None);
        assert!(h.menu_open);
        assert_eq!((h.menu_bucket, h.menu_row), (2, 1));
        select_slot(&mut h, 2, false);
        assert_eq!(h.menu_row, 3);
        select_slot(&mut h, 2, false);
        assert_eq!(h.menu_row, 1, "wraps");
        assert_eq!(select_slot(&mut h, 10, false), None, "slot 10 closes an open menu");
        assert!(!h.menu_open);
        assert_eq!(select_slot(&mut h, 10, false).as_deref(), Some("weapon_0"));
    }

    #[test]
    fn fast_switch_and_wires() {
        let mut h = Hud::default();
        give(&mut h, 13);
        assert_eq!(select_slot(&mut h, 4, true).as_deref(), Some("weapon_13"));
        assert!(!h.menu_open);
        h.wires = [1, 1, 0];
        assert_eq!(select_slot(&mut h, 2, false).as_deref(), Some("cutwire_2"));
        assert_eq!(select_slot(&mut h, 5, false), None);
    }

    #[test]
    fn next_prev_skips_single_when_akimbo_owned() {
        let mut h = Hud::default();
        give(&mut h, 9); // mk23 [2,2]
        give(&mut h, 10); // mk23 akimbo [5,2]
        give(&mut h, 13); // ak [4,3]
        h.weapon = 13;
        let c = select_next_prev(&mut h, true, true);
        assert_eq!(c.as_deref(), Some("weapon_10"), "AK -> akimbo MK23, single MK23 skipped");
        h.weapon = 10;
        let c = select_next_prev(&mut h, true, true);
        assert_eq!(c.as_deref(), Some("weapon_13"), "wraps through bucket 1..3 to the AK");
    }
}
