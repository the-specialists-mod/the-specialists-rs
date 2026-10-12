//! `CHudTSStatus` (0x100d43cc) drawing: round timer, TS messages, objective text, teammate
//! markers, crosshair ID, status panel, power-up, objective markers (§4.3, `Draw` 10049ea0).

use super::Dc;
use super::draw::{GREEN, RED, ftol, half, num};
use super::state::Hud;
use super::text::localise;
use crate::input;
use goldsrc::client::engine;

/// The voice helper's lookup (`GetPlayerTextColor` 1001fcd0): negative teams are 0, then modulo
/// into `iTeamColors` (`0x10096974`, the scoreboard's table: (255,170,0), (125,165,210),
/// (200,90,70), (225,205,45), (145,215,140)), which the status panel's team bar uses too.
pub fn team_color(team: i32) -> [i32; 3] {
    let t = crate::menus::widgets::TEAM_COLORS;
    t[team.max(0) as usize % t.len()].map(i32::from)
}

pub fn draw(dc: &Dc, nohud: bool) {
    if nohud {
        return;
    }
    let dead_or_obs = super::dead_or_observer();
    let id = if dead_or_obs { None } else { crosshair_trace() };
    super::with(|h| {
        h.crosshair_id = id.as_ref().map(|(i, t, _)| (*i, *t));
        if h.round_time != 0.0 {
            draw_round_timer(dc, h);
        }
        draw_ts_messages(dc, h);
        draw_flag_panel(dc, h);
        if dead_or_obs {
            return;
        }
        if h.gamemode != 0 {
            draw_teammate_markers(dc, h);
        }
        if let Some((_, team, name)) = &id {
            dc.string(dc.w() / 2, dc.h() / 2 - 40, name, if *team == 1 { GREEN } else { RED });
        }
        draw_status_panel(dc, h);
        if h.pw_flags == 0 {
            h.pw_expire = 0.0;
        } else {
            draw_powerup(dc, h);
        }
        if h.gamemode > 1 {
            draw_objective_markers(dc, h);
        }
    });
}

/// `DrawRoundTimer` (1004a130): top-right `MM:SS`, red under a minute, else the HUD colour × 0.5
/// (`0x1004a1c5`).
fn draw_round_timer(dc: &Dc, h: &mut Hud) {
    let secs = h.tick_round_timer(dc.now);
    let rgb = if secs < 60 { RED } else { half(dc.hud) };
    let mut x = dc.w() - 0x48;
    x = dc.number(x, 0xc, num::DIGITS_2 | num::DRAW_ZERO, secs / 60, rgb);
    x = dc.string(x, 12, ":", rgb);
    dc.number(x, 12, num::DIGITS_2 | num::DRAW_ZERO | num::LEADING_ZEROS, secs % 60, rgb);
}

/// `DrawTSMessages` (1004af40): timed messages, capped at 10 s, fading over the last second.
/// Black (0,0,0) means "HUD colour".
fn draw_ts_messages(dc: &Dc, h: &mut Hud) {
    for m in h.messages.iter_mut() {
        if m.expire <= dc.now {
            continue;
        }
        if m.expire - dc.now > 10.0 {
            m.expire = dc.now + 10.0;
        }
        // `0x1004afc9..0x1004b085`: each channel is `__ftol(base * min(expire - now, 1))`, the
        // base being the message colour, or for a black message the `hud_r/g/b` cvar values
        // themselves (floats, not rounded first).
        let ic = crate::input::cvars();
        let base = if m.rgb == [0; 3] { [ic.hud_r.value(), ic.hud_g.value(), ic.hud_b.value()] } else { m.rgb.map(|c| c as f32) };
        // x87 keeps the difference and the product unrounded; f64 is as close as Rust gets.
        let f = (m.expire as f64 - dc.now as f64).min(1.0);
        let rgb = base.map(|c| (c as f64 * f) as i32);
        dc.string(m.x, m.y, &m.text, rgb);
    }
}

/// `DrawHudStringLen(0, max, text)` (`0x10021ca0`): the width up to the first character that
/// would pass `max`.
fn string_len_to(dc: &Dc, max: i32, text: &str) -> i32 {
    let mut x = 0i32;
    for c in text.chars().take_while(|&c| c != '\n') {
        let w = dc.g.screen.char_widths[(c as u32 & 0xff) as usize] as i32;
        if x.wrapping_add(w) > max {
            break;
        }
        x = x.wrapping_add(w);
    }
    x
}

/// The flag panel (`0x1004b0e0`, run while `TSFlag` has set the count): two blink timers tick
/// by frame time × time scale (0.1 s and 0.5 s, each toggling a phase); each of the first
/// `count` (≤ 16) flags with a state and a team 1..4 draws the flag sprite at
/// `(w − 56, team·48 + 40)` in the team colour (`iTeamColors`) × (1 or 0.5 by the fast phase
/// while carried, else × 0.75 × (1 or 0.5 by the slow phase)). A carrier 1..64 gets their name
/// (24 bytes) right-aligned against the flag, red (175, 40, 40) for the other team, green
/// (40, 175, 40) for yours; if the carrier is you, "Take the Briefcase to the base!" at
/// y 320, right-aligned 32 px from the edge, in the HUD colour with 35 more red. The count
/// becomes one past the last flag drawn.
fn draw_flag_panel(dc: &Dc, h: &mut Hud) {
    if h.flag_count < 1 {
        return;
    }
    let hud = dc.hud;
    let e = engine();
    let local = e.local_player().map_or(0, |p| p.index);
    let team_of = |h: &Hud, i: i32| usize::try_from(i).ok().and_then(|i| h.players.get(i)).map_or(0, |p| p.team);
    let my_team = team_of(h, local);
    // `m_flTimeDelta / (1 / max(ts, 0.01))`.
    let ts = crate::state::with(|s| s.time_scale).max(0.01);
    let step = super::time_delta() as f64 * ts as f64;
    for (blink, reset) in h.flag_blink.iter_mut().zip([0.1f32, 0.5]) {
        blink.0 = (blink.0 as f64 - step) as f32;
        if blink.0 < 0.0 {
            blink.0 = reset;
            blink.1 = !blink.1;
        }
    }
    let (fast, slow) = (h.flag_blink[0].1, h.flag_blink[1].1);
    let n = h.flag_count.min(16) as usize;
    h.flag_count = 0;
    let (w, x) = (dc.w(), dc.w() - 0x38);
    for i in 0..n {
        let f = h.flags[i];
        if f.state == 0 || !(1..=4).contains(&f.team) {
            continue;
        }
        let y = f.team as i32 * 0x30 + 0x28;
        let k: f64 = if f.state == 1 { if fast { 1.0 } else { 0.5 } } else { (if slow { 1.0 } else { 0.5 }) * 0.75 };
        let c = crate::menus::widgets::TEAM_COLORS[f.team as usize].map(|v| ftol(v as f64 * k));
        dc.spr(dc.g.ts.flag, x, y, c);
        let carrier = f.carrier as i32;
        if (1..=64).contains(&carrier) {
            if carrier == local {
                let msg = "Take the Briefcase to the base!";
                let rgb = [(hud[0] + 0x23).min(0xff), hud[1], hud[2]];
                super::text::draw_string(&dc.g.screen, w - string_len_to(dc, w, msg) - 0x20, 0x140, w, msg, rgb);
            }
            let mut name = e.player_info(carrier).name_bytes;
            name.truncate(0x18);
            let name = super::saytext::string(&name);
            // `0x1004b3b6..`: (195, 45) or (45, 195) × 0.9f, blue 40.
            let rgb = if team_of(h, carrier) != my_team { [175, 40, 40] } else { [40, 175, 40] };
            super::text::draw_string(&dc.g.screen, x - string_len_to(dc, w, &name), y, w, &name, rgb);
        }
        h.flag_count = i as i32 + 1;
    }
}

/// `GetPlayerScreenPos` (1004b470): `WorldToScreen(origin + 40)`.
fn player_screen_pos(dc: &Dc, index: i32) -> Option<(i32, i32)> {
    let e = engine();
    let ent = e.entity(index)?;
    let o = ent.origin;
    e.world_to_screen([o[0], o[1], o[2] + 40.0], &dc.g.screen)
}

/// `DrawTeammateMarkers` (1004ab40): a green dot over visible teammates. TS also has a branch
/// that draws the flag sprite over carriers, but its test reads a `g_PlayerExtraInfo` field
/// nothing writes (`0x101901f0`, only zeroed at `0x1005d5c0`), so it never runs. TS's
/// visibility flags come from `HUD_AddEntity`; the port uses "updated in the last packet".
fn draw_teammate_markers(dc: &Dc, h: &Hud) {
    let e = engine();
    let Some(me) = e.local_player() else { return };
    let my_team = h.players.get(me.index as usize).map_or(0, |p| p.team);
    for i in 1..=e.max_clients().min(64) {
        if i == me.index {
            continue;
        }
        let Some(ent) = e.entity(i) else { continue };
        if ent.player == 0 || ent.curstate.messagenum != me.curstate.messagenum {
            continue;
        }
        let team = h.players[i as usize].team;
        let Some((x, y)) = player_screen_pos(dc, i) else { continue };
        if team == my_team {
            dc.fill(x - 2, y - 2, 4, 4, GREEN, 0x75);
        }
    }
}

/// `DrawCrosshairID` (1004acc0): player trace 8192 units along the view. Returns
/// (index, 1 = teammate / 2 = enemy, name).
fn crosshair_trace() -> Option<(i32, u8, String)> {
    let e = engine();
    let me = e.local_player()?;
    // `GetCullOrigin` (`0x10057270`: the eye in third person, else `v_origin`) along the last
    // refdef's forward (`0x1018bd90`, punch and bob included).
    let start = if crate::view::is_third_person() { crate::view::eye() } else { crate::view::origin() };
    let f = crate::view::camera().forward;
    let end = [start[0] + f[0] * 8192.0, start[1] + f[1] * 8192.0, start[2] + f[2] * 8192.0];
    let (_, _, idx) = e.crosshair_trace(start, end, me.index)?;
    let ent = e.entity(idx)?;
    if ent.player == 0 || !(1..=64).contains(&idx) {
        return None;
    }
    let name = super::saytext::string(&e.player_info(idx).name_bytes);
    if name.is_empty() {
        return None;
    }
    let team = |h: &Hud, i: i32| usize::try_from(i).ok().and_then(|i| h.players.get(i)).map_or(0, |p| p.team);
    let (gm, mine, theirs) = super::with(|h| (h.gamemode, team(h, me.index), team(h, idx)));
    let team = if gm == 0 || mine != theirs { 2 } else { 1 };
    Some((idx, team, name))
}

/// `DrawStatusPanel` (1004a3e0): bottom-left box with the stance sprite filled bottom-up by
/// health, health number, health/kevlar icons, team bar, free slots, credits, kung-fu and
/// movement icons. Hidden by `cl_realhud 1`.
fn draw_status_panel(dc: &Dc, h: &Hud) {
    if input::cvars().cl_realhud.value() != 0.0 {
        return;
    }
    let e = engine();
    let hud = dc.hud;
    let (w, ht) = (dc.w(), dc.h());
    let mut y = ht - 0x30;
    dc.fill(8, y, 0x7f, 0x27, hud, 0xf);
    let stance = dc.g.ts.stance[(h.stance as usize).min(2)];
    if stance != 0 {
        let sh = e.spr_height(stance, 0);
        let sw = e.spr_width(stance, 0);
        let hp = (h.health as i32).min(100);
        // C int arithmetic: the sprite's height is file data.
        let top = sh.wrapping_sub(hp.wrapping_mul(sh) / 100);
        if h.health < 100 {
            // Red over the part health doesn't fill (`0x1004a4d5..0x1004a518`: rows 0..top).
            e.spr_set(stance, RED);
            e.spr_draw_additive(0, 8, y - 4, Some(&goldsrc::sys::wrect_t { left: 0, right: sw, top: 0, bottom: top }));
        }
        let rc = goldsrc::sys::wrect_t { left: 0, right: sw, top, bottom: sh };
        e.spr_set(stance, half(hud));
        e.spr_draw_additive(0, 8, (y - 4).wrapping_add(top), Some(&rc));
    }
    let hp_rgb = if h.health < 50 { RED } else { half(hud) };
    dc.number(0x48, y + 4, num::DIGITS_3 | num::DRAW_ZERO, h.health as i32, hp_rgb);
    dc.spr(dc.g.ts.health, 0x68, y, hp_rgb);
    if h.armor != 0 {
        let rgb = if h.armor < 50 { RED } else { half(hud) };
        dc.spr(dc.g.ts.kevlar, 0x48, y + 0x14, rgb);
    }
    // Team bar (teamplay).
    y -= 0x14;
    if h.gamemode != 0
        && let Some(me) = e.local_player()
        && let Some(p) = usize::try_from(me.index).ok().and_then(|i| h.players.get(i))
    {
        // `iTeamColors[team]` (`0x1004a748`); guarded: TS indexes past the five entries.
        let t = crate::menus::widgets::TEAM_COLORS;
        let c = t[(p.team.max(0) as usize).min(t.len() - 1)].map(i32::from);
        dc.fill(8, y, 0x7f, 0x13, c, 0x23);
        dc.string(10, y, &p.teamname, c);
    }
    // Free slots and credits.
    let mut y = ht - 0x1c;
    let x = 0x88;
    dc.fill(x, y, 0x7f, 0x13, hud, 0xf);
    let rgb = if h.slots == 0 { RED } else { half(hud) };
    let nx = dc.number(x + 8, y + 2, num::DIGITS_2 | num::DRAW_ZERO, h.slots as i32, rgb);
    dc.string(nx + 8, y, &localise("#HUD_FreeSlots"), rgb);
    if h.gamemode > 1 || h.cashbased != 0 {
        y -= 0x14;
        dc.fill(x, y, 0x7f, 0x13, hud, 0xf);
        let rgb = if h.cash < 1000 { RED } else { half(hud) };
        let nx = dc.number(x, y + 2, num::DIGITS_5 | num::DRAW_ZERO, h.cash, rgb);
        dc.string(nx + 8, y, &localise("#HUD_Credits"), rgb);
    }
    // `0x1004aa4e`, `0x1004aad2`: the HUD colour × 2/3.
    let icon = hud.map(|c| ftol(c as f64 * 0.666_666_666_666_666_6));
    if h.kungfu_icon {
        dc.spr(dc.g.ts.kungfu, w / 2 - 0x30, ht - 0x38, icon);
    }
    if h.move_icon {
        dc.spr(dc.g.ts.movement, w / 2, ht - 0x38, icon);
    }
}

/// `DrawPowerup` (1004a2e0): top-left power-up name and seconds left (green).
fn draw_powerup(dc: &Dc, h: &Hud) {
    let f = h.pw_flags;
    let label = if f & 1 != 0 {
        "#Pwr_SlowMotion"
    } else if f & 2 != 0 {
        "#Pwr_InfiniteAmmo"
    } else if f & 8 != 0 {
        "#Pwr_SlowPause"
    } else if f & 0x10 != 0 {
        "#Pwr_DoubleFireRate"
    } else {
        ""
    };
    let x = dc.string(8, 8, &localise(label), GREEN);
    // `0x1004a38b..0x1004a3d3`: the seconds left while timed (0 included), else the stored
    // value when it isn't 0; flags 3.
    let n = if h.pw_expire > dc.now { Some(ftol(h.pw_expire as f64 - dc.now as f64)) } else { (h.pw_value != 0).then_some(h.pw_value) };
    if let Some(n) = n {
        dc.number(x + 4, 8, num::DIGITS_2 | num::DRAW_ZERO, n, GREEN);
    }
}

/// `DrawObjectiveMarkers` (10049fc0): compass sprite at each visible objective, up/down arrow
/// when it's more than 72 units above/below, and the label.
fn draw_objective_markers(dc: &Dc, h: &Hud) {
    let e = engine();
    let my_z = e.local_player().map_or(0.0, |p| p.origin[2]);
    let c = [100, 100, 205];
    for o in h.objectives.iter().take(h.obj_max.max(0) as usize + 1) {
        if !o.visible || o.label.is_empty() {
            continue;
        }
        let Some((x, y)) = e.world_to_screen(o.pos, &dc.g.screen) else { continue };
        dc.spr(dc.g.ts.compass, x - 16, y - 16, c);
        if o.pos[2] > my_z + 72.0 {
            dc.spr(dc.g.ts.uparrow, x - 16, y - 0x30, c);
        } else if o.pos[2] < my_z - 72.0 {
            dc.spr(dc.g.ts.downarrow, x - 16, y + 16, c);
        }
        // Drawn as stored: TS doesn't localise the label (`0x10049fc0`).
        dc.string(x + 16, y - 8, &o.label, c);
    }
}
