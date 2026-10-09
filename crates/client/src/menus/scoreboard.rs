//! Scoreboard (TS `ScorePanel` 1005ce70 / fill 1005db70), immediate mode. Team sections in
//! teamplay, the TS status column ("THE ONE", "UNSTOPPABLE", "SPECIALIST", " KILLS", "DEAD",
//! "WAITING"), spectators last. Rows are built in a `Vec`, so TS's 74-vs-72 entry overrun
//! (`ScorePanel::Update` 1005d5f0) can't happen; at most 72 rows are drawn like TS's arrays.

use crate::hud::{self, Dc, draw::half, state::Hud, text::localise};
use goldsrc::client::engine;

pub const MAX_ROWS: usize = 72;

/// TS award byte (`TSPAward`) to the status column.
pub fn award_text(award: u8) -> &'static str {
    match award {
        1 => "DEAD",
        2 => "3 KILLS",
        3 => "6 KILLS",
        4 => "SPECIALIST",
        5 => "UNSTOPPABLE",
        10 => "THE ONE",
        _ => "",
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    Team { name: String, frags: i32, deaths: i32, players: usize },
    Player { index: i32, name: String, frags: i32, deaths: i32, ping: i32, status: &'static str, local: bool },
    Spectators,
}

/// Sort and group players like the TS panel: by team (teamplay), frags desc, deaths asc.
pub fn build_rows(h: &Hud, names: &dyn Fn(i32) -> Option<(String, i32, bool)>) -> Vec<Row> {
    let mut players: Vec<(i32, String, i32, bool)> = (1..=64).filter_map(|i| names(i).map(|(n, ping, local)| (i, n, ping, local))).collect();
    players.sort_by(|a, b| {
        let (pa, pb) = (&h.players[a.0 as usize], &h.players[b.0 as usize]);
        pb.frags.cmp(&pa.frags).then(pa.deaths.cmp(&pb.deaths)).then(a.0.cmp(&b.0))
    });
    let mk = |(i, n, ping, local): &(i32, String, i32, bool)| {
        let p = &h.players[*i as usize];
        let status = if p.observer { "WAITING" } else { award_text(p.award) };
        Row::Player { index: *i, name: n.clone(), frags: p.frags, deaths: p.deaths, ping: *ping, status, local: *local }
    };
    let mut rows = Vec::new();
    let spectators: Vec<_> = players.iter().filter(|p| h.players[p.0 as usize].observer && h.gamemode != 0).collect();
    if h.gamemode != 0 {
        let mut teams = h.team_scores.clone();
        // Teams that appear only in TeamInfo.
        for (i, _, _, _) in &players {
            let t = &h.players[*i as usize].teamname;
            if !t.is_empty() && !teams.iter().any(|x| x.name.eq_ignore_ascii_case(t)) {
                teams.push(crate::hud::state::TeamScore { name: t.clone(), ..Default::default() });
            }
        }
        teams.sort_by(|a, b| b.frags.cmp(&a.frags).then(a.deaths.cmp(&b.deaths)));
        for t in teams {
            let members: Vec<_> = players.iter().filter(|p| !h.players[p.0 as usize].observer && h.players[p.0 as usize].teamname.eq_ignore_ascii_case(&t.name)).collect();
            if members.is_empty() {
                continue;
            }
            rows.push(Row::Team { name: t.name.clone(), frags: t.frags, deaths: t.deaths, players: members.len() });
            rows.extend(members.into_iter().map(mk));
        }
        let unassigned: Vec<_> = players.iter().filter(|p| !h.players[p.0 as usize].observer && h.players[p.0 as usize].teamname.is_empty()).collect();
        rows.extend(unassigned.into_iter().map(mk));
        if !spectators.is_empty() {
            rows.push(Row::Spectators);
            rows.extend(spectators.into_iter().map(mk));
        }
    } else {
        rows.extend(players.iter().map(mk));
    }
    rows.truncate(MAX_ROWS);
    rows
}

pub fn draw(dc: &Dc) {
    let e = engine();
    let max = e.max_clients().min(64);
    let names = |i: i32| {
        if i > max {
            return None;
        }
        let info = e.player_info(i);
        (!info.name.is_empty()).then_some((info.name, info.ping, info.this_player))
    };
    let (rows, server) = hud::with(|h| (build_rows(h, &names), h.server_name.clone()));
    let w = (dc.w() - 40).min(640);
    let x = (dc.w() - w) / 2;
    let lh = 16;
    let hgt = (rows.len() as i32 + 3) * lh + 16;
    let y0 = ((dc.h() - hgt) / 2).max(8);
    dc.fill(x, y0, w, hgt, [0; 3], 150);
    let col = |c: [f32; 3]| c;
    let white = [1.0, 1.0, 1.0];
    let hudf = dc.hud.map(|c| c as f32 / 255.0);
    let title = if server.is_empty() { engine().game_directory() } else { server };
    e.draw_console_string(x + 8, y0 + 4, col(hudf), &title);
    let (cs, ck, cd, cp) = (x + w - 300, x + w - 160, x + w - 100, x + w - 50);
    let mut y = y0 + 4 + lh + 4;
    e.draw_console_string(x + 8, y, white, &localise("#Player"));
    e.draw_console_string(ck, y, white, &localise("#SCORE"));
    e.draw_console_string(cd, y, white, &localise("#DEATHS"));
    e.draw_console_string(cp, y, white, &localise("#LATENCY"));
    y += lh + 2;
    dc.fill(x + 4, y - 2, w - 8, 1, half(dc.hud), 200);
    for r in rows {
        match r {
            Row::Team { name, frags, deaths, players } => {
                let n = localise(&name);
                let label = format!("{n} ({players} {})", localise(if players == 1 { "#Player" } else { "#Player_plural" }));
                dc.fill(x + 4, y, w - 8, lh, half(dc.hud), 60);
                e.draw_console_string(x + 8, y, hudf, &label);
                e.draw_console_string(ck, y, hudf, &frags.to_string());
                e.draw_console_string(cd, y, hudf, &deaths.to_string());
            }
            Row::Spectators => {
                e.draw_console_string(x + 8, y, [0.7, 0.7, 0.7], &localise("#Spectators"));
            }
            Row::Player { name, frags, deaths, ping, status, local, .. } => {
                if local {
                    dc.fill(x + 4, y, w - 8, lh, dc.hud, 40);
                }
                let c = if local { white } else { [0.85, 0.85, 0.85] };
                e.draw_console_string(x + 16, y, c, &name);
                if !status.is_empty() {
                    e.draw_console_string(cs, y, [1.0, 0.8, 0.2], status);
                }
                e.draw_console_string(ck, y, c, &frags.to_string());
                e.draw_console_string(cd, y, c, &deaths.to_string());
                e.draw_console_string(cp, y, c, &ping.to_string());
            }
        }
        y += lh;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_sorted_and_grouped() {
        let mut h = Hud::default();
        h.gamemode = 1;
        for (i, (f, d, t)) in [(1, (5, 1, "A")), (2, (9, 0, "B")), (3, (5, 0, "A")), (4, (0, 0, ""))] {
            h.players[i].frags = f;
            h.players[i].deaths = d;
            h.players[i].teamname = t.into();
        }
        h.players[4].observer = true;
        h.team_scores = vec![
            crate::hud::state::TeamScore { name: "A".into(), frags: 10, deaths: 1 },
            crate::hud::state::TeamScore { name: "B".into(), frags: 9, deaths: 0 },
        ];
        let names = |i: i32| (i <= 4).then(|| (format!("p{i}"), 10, i == 1));
        let rows = build_rows(&h, &names);
        let idx: Vec<String> = rows
            .iter()
            .map(|r| match r {
                Row::Team { name, .. } => format!("T{name}"),
                Row::Player { index, .. } => index.to_string(),
                Row::Spectators => "S".into(),
            })
            .collect();
        assert_eq!(idx, ["TA", "3", "1", "TB", "2", "S", "4"]);
        assert!(matches!(&rows[6], Row::Player { status: "WAITING", .. }));
    }

    #[test]
    fn caps_rows() {
        let h = Hud::default();
        let names = |i: i32| Some((format!("p{i}"), 0, false));
        assert_eq!(build_rows(&h, &names).len(), 64);
        assert_eq!(award_text(10), "THE ONE");
    }
}
