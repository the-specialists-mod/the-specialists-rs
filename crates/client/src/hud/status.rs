//! `CHudTSStatus` (0x100d43cc) drawing: round timer, TS messages, objective text, teammate
//! markers, crosshair ID, status panel, power-up, objective markers (§4.3, `Draw` 10049ea0).

use super::Dc;
use super::draw::{GREEN, RED, half, num};
use super::state::Hud;
use super::text::localise;
use crate::input;
use goldsrc::client::engine;

/// TS team colours (`0x10096974`, RGB per team number).
const TEAM_COLORS: [[i32; 3]; 5] = [[150, 175, 255], [100, 150, 255], [255, 100, 100], [255, 255, 100], [100, 255, 100]];

/// The voice helper's lookup (`GetPlayerTextColor` 1001fcd0): negative teams are 0, then modulo.
pub fn team_color(team: i32) -> [i32; 3] {
    TEAM_COLORS[team.max(0) as usize % TEAM_COLORS.len()]
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
        draw_objective_text(dc, h);
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

/// `DrawRoundTimer` (1004a130): top-right `MM:SS`, red under a minute.
fn draw_round_timer(dc: &Dc, h: &mut Hud) {
    let secs = h.tick_round_timer(dc.now);
    let rgb = if secs < 60 { RED } else { dc.hud };
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
        let base = if m.rgb == [0; 3] { dc.hud } else { m.rgb };
        let f = (m.expire - dc.now).min(1.0);
        let rgb = base.map(|c| (c as f32 * f) as i32);
        dc.string(m.x, m.y, &m.text, rgb);
    }
}

/// `DrawObjectiveText` (1004b0e0): the objective labels as wrapped text, bottom centre.
fn draw_objective_text(dc: &Dc, h: &Hud) {
    if h.gamemode < 2 {
        return;
    }
    let mut y = dc.h() - 96;
    for o in h.objectives.iter().take(h.obj_max.max(0) as usize + 1).filter(|o| o.visible && !o.label.is_empty()) {
        let text = localise(&o.label);
        let max = dc.w() - 64;
        let mut line = String::new();
        let mut lines = Vec::new();
        for word in text.split(' ') {
            let candidate = if line.is_empty() { word.to_owned() } else { format!("{line} {word}") };
            if dc.string_len(&candidate) > max && !line.is_empty() {
                lines.push(std::mem::replace(&mut line, word.to_owned()));
            } else {
                line = candidate;
            }
        }
        lines.push(line);
        for l in lines {
            dc.string((dc.w() - dc.string_len(&l)) / 2, y, &l, half(dc.hud));
            y -= 16;
        }
    }
}

/// `GetPlayerScreenPos` (1004b470): `WorldToScreen(origin + 40)`.
fn player_screen_pos(dc: &Dc, index: i32) -> Option<(i32, i32)> {
    let e = engine();
    let ent = e.entity(index)?;
    let o = ent.origin;
    e.world_to_screen([o[0], o[1], o[2] + 40.0], &dc.g.screen)
}

/// `DrawTeammateMarkers` (1004ab40): a green dot over visible teammates, flag sprite over
/// carriers (green own team, red enemy). TS's visibility flags come from `HUD_AddEntity`; the
/// port uses "updated in the last packet".
fn draw_teammate_markers(dc: &Dc, h: &Hud) {
    let e = engine();
    let Some(me) = e.local_player() else { return };
    let my_team = h.players.get(me.index as usize).map_or(0, |p| p.team);
    let carriers: Vec<i32> = h.flags.iter().filter(|f| f.state == 1).map(|f| f.carrier as i32).collect();
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
        if carriers.contains(&i) {
            let rgb = if team == my_team { [15, 65, 15] } else { [65, 15, 15] };
            dc.spr(dc.g.ts.flag, x - 24, y - 48, rgb);
        } else if team == my_team {
            dc.fill(x - 2, y - 2, 4, 4, GREEN, 0x75);
        }
    }
}

/// `DrawCrosshairID` (1004acc0): player trace 8192 units along the view. Returns
/// (index, 1 = teammate / 2 = enemy, name).
fn crosshair_trace() -> Option<(i32, u8, String)> {
    let e = engine();
    let me = e.local_player()?;
    let vh = e.local_view_height();
    let start = [me.origin[0] + vh[0], me.origin[1] + vh[1], me.origin[2] + vh[2]];
    let (f, _, _) = e.angle_vectors(e.view_angles());
    let end = [start[0] + f[0] * 8192.0, start[1] + f[1] * 8192.0, start[2] + f[2] * 8192.0];
    let (_, _, idx) = e.player_trace(start, end, me.index)?;
    let ent = e.entity(idx)?;
    if ent.player == 0 || !(1..=64).contains(&idx) {
        return None;
    }
    let name = e.player_info(idx).name;
    if name.is_empty() {
        return None;
    }
    let (gm, mine, theirs) = super::with(|h| (h.gamemode, h.players[me.index as usize].team, h.players[idx as usize].team));
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
        if h.health < 100 {
            // Red silhouette under the filled part.
            e.spr_set(stance, RED);
            e.spr_draw_additive(0, 8, y - 4, None);
        }
        let top = sh - hp * sh / 100;
        let rc = goldsrc::sys::wrect_t { left: 0, right: sw, top, bottom: sh };
        e.spr_set(stance, half(hud));
        e.spr_draw_additive(0, 8, y - 4 + top, Some(&rc));
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
    {
        let p = &h.players[me.index as usize];
        let c = TEAM_COLORS[(p.team.max(0) as usize).min(TEAM_COLORS.len() - 1)];
        dc.fill(8, y, 0x7f, 0x13, c, 0x23);
        dc.string(10, y, &p.teamname, c);
    }
    // Free slots and credits.
    let mut y = ht - 0x1c;
    let x = 0x88;
    dc.fill(x, y, 0x7f, 0x13, hud, 0xf);
    let rgb = if h.slots == 0 { RED } else { half(hud) };
    let nx = dc.number(x + 8, y + 2, num::DIGITS_3 | num::DRAW_ZERO, h.slots as i32, rgb);
    dc.string(nx + 8, y, &localise("#HUD_FreeSlots"), rgb);
    if h.gamemode > 1 || h.cashbased != 0 {
        y -= 0x14;
        dc.fill(x, y, 0x7f, 0x13, hud, 0xf);
        let rgb = if h.cash < 1000 { RED } else { half(hud) };
        let nx = dc.number(x, y + 2, num::DIGITS_5 | num::DRAW_ZERO, h.cash, rgb);
        dc.string(nx + 8, y, &localise("#HUD_Credits"), rgb);
    }
    if h.kungfu_icon {
        dc.spr(dc.g.ts.kungfu, w / 2 - 0x30, ht - 0x38, half(hud));
    }
    if h.move_icon {
        dc.spr(dc.g.ts.movement, w / 2, ht - 0x38, half(hud));
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
    let n = if h.pw_expire > dc.now { (h.pw_expire - dc.now) as i32 } else { h.pw_value };
    if n != 0 {
        dc.number(x + 4, 8, num::DIGITS_3, n, GREEN);
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
        dc.string(x + 16, y - 8, &localise(&o.label), c);
    }
}
