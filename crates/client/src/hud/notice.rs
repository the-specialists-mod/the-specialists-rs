//! The SDK elements TS kept as they were: `CHudMessage` (HudText), `CHudDeathNotice` (with TS's
//! `nohud` gate), `CHudSayText` and `CHudMenu` (ShowMenu).

use super::Dc;
use super::text::localise;
use goldsrc::client::engine;

const CON_COLOR: [f32; 3] = [1.0, 180.0 / 255.0, 30.0 / 255.0];

fn team_color(team: i32) -> [f32; 3] {
    match team {
        1 => [0.6, 0.8, 1.0],
        2 => [1.0, 0.25, 0.25],
        3 => [1.0, 1.0, 0.4],
        4 => [0.4, 1.0, 0.4],
        _ => CON_COLOR,
    }
}

/// `CHudMessage` (100290b0) for `HudText`: a titles.txt message at its position with its colour,
/// shown for its hold time.
pub fn draw_hud_text(dc: &Dc) {
    let (text, expire) = super::with(|h| (h.hud_text.clone(), h.hud_text_expire));
    if text.is_empty() || expire <= dc.now {
        return;
    }
    let e = engine();
    let key = text.trim_start_matches('#');
    let (body, x, y, rgb, hold) = match e.text_message(key) {
        Some(m) if !m.pMessage.is_null() => {
            let s: String = m.message().to_bytes().iter().map(|&b| b as char).collect();
            (s, m.x, m.y, [m.r1 as i32, m.g1 as i32, m.b1 as i32], m.holdtime + m.fadein + m.fadeout)
        }
        _ => (localise(&text), -1.0, 0.35, [255, 255, 255], 5.0),
    };
    // The message's own hold time replaces the default 5 s set when it arrived.
    if expire - 5.0 + hold <= dc.now {
        return;
    }
    let lines: Vec<&str> = body.split('\n').collect();
    let lh = dc.g.screen.char_height.max(12);
    let mut yy = if y < 0.0 { (dc.h() - lh * lines.len() as i32) / 2 } else { (y * dc.h() as f32) as i32 };
    for l in lines {
        let w = dc.string_len(l);
        let xx = if x < 0.0 { (dc.w() - w) / 2 } else { (x * dc.w() as f32) as i32 };
        dc.string(xx, yy, l, rgb);
        yy += lh;
    }
}

/// `CHudDeathNotice::Draw` (1001b020): top-right kill feed with `d_<weapon>` sprites.
pub fn draw_death_notices(dc: &Dc) {
    let e = engine();
    let deaths = super::with(|h| {
        h.deaths.retain(|d| d.expire > dc.now);
        h.deaths.clone()
    });
    let lh = 20;
    for (i, d) in deaths.iter().enumerate() {
        let y = 32 + 2 + lh * i as i32;
        let victim = if d.victim > 0 { e.player_info(d.victim).name } else { String::new() };
        let killer = if d.killer > 0 { e.player_info(d.killer).name } else { String::new() };
        let (vteam, kteam) = super::with(|h| {
            (h.players.get(d.victim as usize).map_or(0, |p| p.team), h.players.get(d.killer as usize).map_or(0, |p| p.team))
        });
        let s = &dc.g.sprites;
        let spr = s.index(&format!("d_{}", d.weapon)).or_else(|| s.index("d_skull"));
        let sw = spr.map_or_else(|| e.console_string_size(&d.weapon).0 + 8, |i| s.rects[i].right - s.rects[i].left);
        let mut x = dc.w() - e.console_string_size(&victim).0 - sw - 8;
        let suicide = d.killer == d.victim || d.killer == 0;
        if !suicide {
            x -= 5 + e.console_string_size(&killer).0;
            x = e.draw_console_string(x, y, team_color(kteam), &killer) + 5;
        }
        match spr {
            Some(i) => {
                e.spr_set(s.handles[i], [255, 80, 0]);
                e.spr_draw_additive(0, x, y, Some(&s.rects[i]));
            }
            None => {
                e.draw_console_string(x + 4, y, [1.0, 80.0 / 255.0, 0.0], &d.weapon);
            }
        }
        x += sw;
        if d.victim > 0 {
            e.draw_console_string(x, y, team_color(vteam), &victim);
        }
    }
}

/// Console line TS prints for a kill (" killed ", " with ", " killed self").
pub fn death_console_line(killer: &str, victim: &str, weapon: &str, suicide: bool, teamkill: bool) -> String {
    if suicide {
        format!("{victim} killed self\n")
    } else if teamkill {
        format!("{killer} killed his teammate {victim}\n")
    } else {
        format!("{killer} killed {victim} with {weapon}\n")
    }
}

/// Parsed span of a `ShowMenu` line (SDK menu.cpp escapes `\w \d \y \r \R`).
#[derive(Debug, PartialEq)]
pub struct Span {
    pub text: String,
    pub rgb: [i32; 3],
    pub right: bool,
    pub line: usize,
}

pub fn parse_menu(text: &str) -> Vec<Span> {
    let mut out = Vec::new();
    let (mut rgb, mut right, mut line) = ([255, 255, 255], false, 0);
    let mut cur = String::new();
    let mut it = text.chars().peekable();
    let flush = |cur: &mut String, out: &mut Vec<Span>, rgb, right, line| {
        if !cur.is_empty() {
            out.push(Span { text: std::mem::take(cur), rgb, right, line });
        }
    };
    while let Some(c) = it.next() {
        match c {
            '\\' => {
                flush(&mut cur, &mut out, rgb, right, line);
                match it.next() {
                    Some('w') => rgb = [255, 255, 255],
                    Some('d') => rgb = [100, 100, 100],
                    Some('y') => rgb = [255, 210, 64],
                    Some('r') => rgb = [210, 24, 0],
                    Some('R') => right = true,
                    _ => {}
                }
            }
            '\n' => {
                flush(&mut cur, &mut out, rgb, right, line);
                right = false;
                line += 1;
            }
            c => cur.push(c),
        }
    }
    flush(&mut cur, &mut out, rgb, right, line);
    out
}

/// `CHudMenu::Draw` (100285f0): left side, vertically centred above the say text.
pub fn draw_menu(dc: &Dc) {
    if crate::menus::scores_visible() {
        return;
    }
    let menu = super::with(|h| {
        if h.menu.displayed && h.menu.shutoff > 0.0 && h.menu.shutoff <= dc.now {
            h.menu.displayed = false;
        }
        h.menu.displayed.then(|| h.menu.text.clone())
    });
    let Some(text) = menu else { return };
    let fh = dc.g.screen.char_height.max(12);
    let nlc = text.matches('\n').count() as i32;
    let y0 = dc.h() / 2 - (nlc / 2) * fh - (3 * fh + fh / 3);
    let mut x_line = (usize::MAX, 20);
    for s in parse_menu(&text) {
        let y = y0 + s.line as i32 * fh;
        if x_line.0 != s.line {
            x_line = (s.line, 20);
        }
        let x = if s.right { dc.w() / 2 - dc.string_len(&s.text) } else { x_line.1 };
        let end = dc.string(x, y, &s.text, s.rgb);
        if !s.right {
            x_line.1 = end;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_escapes() {
        let s = parse_menu("\\yTitle\n\\w1. A\\R\\d$5");
        assert_eq!(s[0], Span { text: "Title".into(), rgb: [255, 210, 64], right: false, line: 0 });
        assert_eq!(s[1].text, "1. A");
        assert_eq!(s[2], Span { text: "$5".into(), rgb: [100, 100, 100], right: true, line: 1 });
    }

    #[test]
    fn kill_lines() {
        assert_eq!(death_console_line("a", "b", "glock18", false, false), "a killed b with glock18\n");
        assert_eq!(death_console_line("", "b", "", true, false), "b killed self\n");
    }
}
