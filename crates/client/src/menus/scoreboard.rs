//! TS's VGUI1 scoreboard, `ScorePanel` (`0x26af0` bytes, constructor `0x1005ce70`, created by
//! `CreateScoreBoard` `0x10063a50`): the HL SDK 2.3 scoreboard without its close button, with
//! TS's status column and team-score rule. Drawn with engine fills ([`crate::hud::vgui`]).
//!
//! * Data: `g_TeamInfo` (`0x1018f1b0`, 65 × `0x28`) is rebuilt by `RebuildTeams` (`0x1005da20`,
//!   on `TeamInfo` and when the board opens) and `TeamScore` overrides a listed team's score
//!   (`0x10065120`). `Update` (`0x1005d5f0`: on open, on `ScoreInfo` and every 0.5 s while shown,
//!   `0x100649b8`) sorts the rows (`SortTeams` `0x1005d6a0`, `SortPlayers` `0x1005d8f0`) and fills
//!   the grid (`FillGrid` `0x1005db70`).
//! * Layout: the panel at (`XRES(104)`, `YRES(40)`) (30, 30 at 512 wide; 0, 20 at 400) with a grey
//!   `LineBorder` over black at VGUI alpha 96; the server name in "Scoreboard Title Text"; a
//!   header grid of eight columns (640-based widths 24, 140, 56, 40, 46, 46, 40 and the rest) in
//!   "Scoreboard Small Text"; one grid per row in a list box (`CListBox`, [`super::scroll`]),
//!   which scrolls by rows: `Update` sets its range to the row count, and its `ScrollBar2` shows
//!   when rows are scrolled off the top or run past the bottom.
//! * Squelch mode (voice muting): see `mouse_pressed` and `cursor_moved`.

use super::scroll::ListBox;
use super::widgets::{SC_WHITE, ViewportScheme, team_color};
use crate::hud::state::PlayerExtra;
use crate::hud::vgui::{self, Align, BitmapFont, Color, Image, Panel, Schemes, Surface, ftol, spill, xs, ys};
use std::rc::Rc;

/// `NUM_ROWS`: grids and sort slots (`0x4a`).
pub const NUM_ROWS: usize = 74;
pub const NUM_COLUMNS: usize = 8;
/// The player loops run `1..MAX_PLAYERS`.
const MAX_PLAYERS: usize = 64;

pub const TEAM_NO: i32 = 0;
pub const TEAM_YES: i32 = 1;
pub const TEAM_SPECTATORS: i32 = 2;
pub const TEAM_BLANK: i32 = 3;

/// `g_ColumnInfo` (`0x10096730`): title, 640-based width; all east-aligned headers.
const COLUMNS: [(Option<&str>, i32); NUM_COLUMNS] = [
    (None, 24),
    (None, 140),
    (None, 56),
    (Some("#SCORE"), 40),
    (Some("#DEATHS"), 46),
    (Some("#LATENCY"), 46),
    (Some("#VOICE"), 40),
    (None, 2),
];
const COL_NAME: usize = 1;
const COL_CLASS: usize = 2;
const COL_KILLS: usize = 3;
const COL_DEATHS: usize = 4;
const COL_LATENCY: usize = 5;
const COL_VOICE: usize = 6;

/// One `team_info_t`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TeamInfo {
    pub name: String,
    pub frags: i16,
    pub deaths: i16,
    pub ping: i16,
    pub packetloss: i16,
    pub ownteam: bool,
    pub players: i16,
    pub already_drawn: bool,
    pub scores_overriden: bool,
    pub teamnumber: i32,
}

/// One engine `hud_player_info_t` (`GetAllPlayersInfo` `0x10064880`): `name` is `None` for an
/// empty slot, as the engine's `NULL`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlayerSlot {
    pub name: Option<Vec<u8>>,
    pub ping: i16,
    pub packetloss: u8,
    pub this_player: bool,
    /// `GetEntityByIndex` found the client's entity.
    pub entity: bool,
}

/// Everything `Update` reads.
pub struct Inputs<'a> {
    /// Indexed by client number, at least `MAX_PLAYERS` long.
    pub players: &'a [PlayerSlot],
    pub extra: &'a [PlayerExtra],
    /// `gHUD.m_Teamplay` (`GameMode`).
    pub teamplay: i32,
    pub server_name: &'a str,
    /// `GetTeamName(n)`: the viewport's team names (`+0x8bc + n * 0x40`, from `TeamNames`,
    /// localised as the handler stores them).
    pub team_names: &'a [String],
    /// `BufferedLocaliseTextString`.
    pub lookup: &'a dyn Fn(&str) -> String,
    pub time: f32,
    pub spectate_only: bool,
    /// `IsInSquelchMode`: `FillGrid` drops the highlight when it's off.
    pub squelch: bool,
    pub screen_h: i32,
    pub screen_w: i32,
}

/// One cell's label (`CLabelHeader`) as `FillGrid` leaves it.
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub visible: bool,
    /// The text after `CLabelHeader::setText` stripped trailing spaces.
    pub text: Vec<u8>,
    pub text2: Vec<u8>,
    /// "Scoreboard Title Text" (else "Scoreboard Text").
    pub title_font: bool,
    pub fg: Color,
    pub bg: Color,
    pub align: Align,
    pub tall: i32,
    /// The voice column's player (`UpdateSpeakerImage` 0x100185d0), else 0.
    pub voice: i32,
    /// `_gap`: where `text2` starts.
    pub gap: i32,
    /// The voice column's image (`setImage`), kept between `UpdateBanButton` calls.
    pub image: Option<Rc<Image>>,
}

impl Default for Cell {
    fn default() -> Cell {
        Cell {
            visible: false,
            text: Vec::new(),
            text2: Vec::new(),
            title_font: false,
            fg: Color::new(0, 0, 0, 0),
            bg: Color::new(0, 0, 0, 255),
            align: Align::West,
            tall: 0,
            voice: 0,
            gap: 0,
            image: None,
        }
    }
}

/// The scoreboard's state.
#[derive(Debug, Clone)]
pub struct ScoreBoard {
    pub teams: Vec<TeamInfo>,
    pub num_teams: usize,
    /// `m_iPlayerNum`: the local player's index.
    pub player_num: i32,
    pub last_killed_by: i32,
    /// `m_fLastKillTime` (an int in TS).
    pub last_kill_time: i32,
    /// `m_iSortedRows`, `m_iIsATeam`, `m_iRows`.
    pub sorted: [i32; NUM_ROWS],
    pub is_a_team: [i32; NUM_ROWS],
    pub rows: usize,
    has_been_sorted: [bool; NUM_ROWS],
    pub title: Vec<u8>,
    /// `m_PlayerEntries[col][row]`.
    pub cells: Vec<[Cell; NUM_COLUMNS]>,
    /// Whether column 1's label has been given the small font for its second text (persistent:
    /// `setFont2` is only called for team rows, `0x1005e155`).
    small_font2: [bool; NUM_ROWS],
    /// Row underlines (`SetRowUnderline`): offset and colour.
    pub underline: [Option<(i32, Color)>; NUM_ROWS],
    /// `m_iHighlightRow` and the highlighted label (row, column).
    pub highlight_row: i32,
    highlight_label: Option<(usize, usize)>,
    /// The last bounds of each row's labels: VGUI keeps a hidden row's old bounds, and
    /// `cursorMoved` tests all rows.
    bounds: Vec<Option<[Panel; NUM_COLUMNS]>>,
    /// Which grids the list showed at the last paint (only shown panels take input).
    shown: Vec<bool>,
    /// `m_flScoreBoardLastUpdated` (viewport `+0x20c`).
    pub next_update: f32,
    /// `m_PlayerList`'s scrolling.
    pub list: ListBox,
}

impl Default for ScoreBoard {
    fn default() -> ScoreBoard {
        ScoreBoard {
            teams: vec![TeamInfo::default(); 65],
            num_teams: 0,
            player_num: 0,
            last_killed_by: 0,
            last_kill_time: 0,
            sorted: [0; NUM_ROWS],
            is_a_team: [0; NUM_ROWS],
            rows: 0,
            has_been_sorted: [false; NUM_ROWS],
            title: Vec::new(),
            cells: vec![std::array::from_fn(|_| Cell::default()); NUM_ROWS],
            small_font2: [false; NUM_ROWS],
            underline: [None; NUM_ROWS],
            highlight_row: -1,
            highlight_label: None,
            bounds: vec![None; NUM_ROWS],
            shown: vec![false; NUM_ROWS],
            next_update: 0.0,
            list: ListBox::default(),
        }
    }
}

fn eq_ignore_case(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn fmt_d(v: i32) -> Vec<u8> {
    v.to_string().into_bytes()
}

impl ScoreBoard {
    /// `Initialize` (`0x1005d5a0`, from the constructor and `ReInitialize` on every `VidInit`).
    pub fn initialize(&mut self) {
        self.last_killed_by = 0;
        self.last_kill_time = 0;
        self.player_num = 0;
        self.num_teams = 0;
        self.teams = vec![TeamInfo::default(); 65];
    }

    /// `GetAllPlayersInfo`'s side effect: the local player's index.
    fn players_info(&mut self, inp: &Inputs) {
        for i in 1..MAX_PLAYERS {
            if inp.players[i].this_player {
                self.player_num = i as i32;
            }
        }
    }

    fn push(&mut self, kind: i32, index: i32) {
        // TS writes past its 74 slots with more rows than that; the port stops at the last.
        if self.rows < NUM_ROWS {
            self.is_a_team[self.rows] = kind;
            self.sorted[self.rows] = index;
            self.rows += 1;
        }
    }

    /// `SortPlayers` (`0x1005d8f0`): the unsorted players (of team `team`), most frags first,
    /// then fewest deaths, under a `kind` header row when `kind` isn't 0; a blank row after a team.
    fn sort_players(&mut self, inp: &Inputs, kind: i32, team: Option<&str>) {
        let mut created = false;
        loop {
            let (mut highest, mut lowest, mut best) = (-99999, 99999, 0);
            for i in 1..MAX_PLAYERS {
                let (p, x) = (&inp.players[i], &inp.extra[i]);
                if !self.has_been_sorted[i]
                    && p.name.is_some()
                    && x.frags >= highest
                    && p.entity
                    && team.is_none_or(|t| eq_ignore_case(&x.teamname, t))
                    && (x.frags > highest || x.deaths < lowest)
                {
                    best = i;
                    lowest = x.deaths;
                    highest = x.frags;
                }
            }
            if best == 0 {
                break;
            }
            if !created && kind != 0 {
                self.push(kind, 0);
                created = true;
            }
            self.push(TEAM_NO, best as i32);
            self.has_been_sorted[best] = true;
        }
        if team.is_some() {
            self.push(TEAM_BLANK, 0);
        }
    }

    /// `SortTeams` (`0x1005d6a0`). TS sums the players' frags into an unoverridden team only in
    /// game mode 0 or 1, and its deaths, ping and loss always.
    fn sort_teams(&mut self, inp: &Inputs) {
        for t in &mut self.teams[1..=self.num_teams] {
            if !t.scores_overriden {
                t.frags = 0;
            }
            t.deaths = 0;
            t.ping = 0;
            t.packetloss = 0;
        }
        for i in 1..MAX_PLAYERS {
            let (p, x) = (&inp.players[i], &inp.extra[i]);
            if p.name.is_none() || x.teamname.is_empty() {
                continue;
            }
            let Some(j) = (1..=self.num_teams).find(|&j| eq_ignore_case(&x.teamname, &self.teams[j].name)) else { continue };
            let t = &mut self.teams[j];
            if !t.scores_overriden && inp.teamplay < 2 {
                t.frags = t.frags.wrapping_add(x.frags as i16);
            }
            t.deaths = t.deaths.wrapping_add(x.deaths as i16);
            t.ping = t.ping.wrapping_add(p.ping);
            t.packetloss = t.packetloss.wrapping_add(p.packetloss as i16);
            t.ownteam = p.this_player;
            t.teamnumber = x.team.max(0);
        }
        for t in &mut self.teams[1..=self.num_teams] {
            t.already_drawn = false;
            if t.players > 0 {
                t.ping /= t.players;
                t.packetloss /= t.players;
            }
        }
        loop {
            let (mut highest, mut lowest, mut best) = (-99999, 99999, 0);
            for i in 1..=self.num_teams {
                let t = &self.teams[i];
                let frags = t.frags as i32;
                if t.players > 0 && !t.already_drawn && frags >= highest && (frags > highest || (t.deaths as i32) < lowest) {
                    best = i;
                    lowest = t.deaths as i32;
                    highest = frags;
                }
            }
            if best == 0 {
                break;
            }
            self.push(TEAM_YES, best as i32);
            self.teams[best].already_drawn = true;
            let name = self.teams[best].name.clone();
            self.sort_players(inp, 0, Some(&name));
        }
        self.sort_players(inp, TEAM_SPECTATORS, None);
    }

    /// `RebuildTeams` (`0x1005da20`): teams in order of their first listed player, each new one in
    /// the next slot. A slot keeps its other fields (score override included) when a different
    /// team takes it over, and a slot left without players is cleared, as in TS.
    pub fn rebuild_teams(&mut self, inp: &Inputs, sb: &Schemes) {
        for t in &mut self.teams[1..=self.num_teams] {
            t.players = 0;
        }
        self.players_info(inp);
        self.num_teams = 0;
        for i in 1..MAX_PLAYERS {
            let (p, x) = (&inp.players[i], &inp.extra[i]);
            if p.name.is_none() || x.teamname.is_empty() {
                continue;
            }
            let mut j = 1;
            while j <= self.num_teams {
                if self.teams[j].name.is_empty() || eq_ignore_case(&x.teamname, &self.teams[j].name) {
                    break;
                }
                j += 1;
            }
            if j > self.num_teams {
                let mut k = 1;
                while k <= self.num_teams && !self.teams[k].name.is_empty() {
                    k += 1;
                }
                self.num_teams = self.num_teams.max(k);
                // `strncpy(name, teamname, 16)` (0x1005da20); see `TeamInfo` for the 16th byte.
                self.teams[k].name = x.teamname.chars().take(16).collect();
                self.teams[k].players = 0;
            }
            self.teams[j].players += 1;
        }
        for t in &mut self.teams[1..=self.num_teams] {
            if t.players < 1 {
                *t = TeamInfo::default();
            }
        }
        self.update(inp, sb);
    }

    /// `TeamScore` (`0x10065120`): a listed team's score, from now on not summed from players.
    pub fn team_score(&mut self, name: &str, frags: i16, deaths: i16) {
        if let Some(j) = (1..=self.num_teams).find(|&j| eq_ignore_case(name, &self.teams[j].name)) {
            let t = &mut self.teams[j];
            t.scores_overriden = true;
            t.frags = frags;
            t.deaths = deaths;
        }
    }

    /// `DeathMsg` (`0x1005e5d0`): when the local player died (or the world killed someone), the
    /// killer's row is lit for 10 seconds.
    pub fn death_msg(&mut self, killer: i32, victim: i32, time: f32) {
        let me = self.player_num;
        if victim != me && killer != 0 {
            return;
        }
        self.last_killed_by = if victim == me && killer != 0 { killer } else { me };
        self.last_kill_time = ftol((time + 10.0) as f64);
        if killer == me {
            self.last_killed_by = me;
        }
    }

    /// `Update` (`0x1005d5f0`).
    pub fn update(&mut self, inp: &Inputs, sb: &Schemes) {
        // `sprintf("%s")` of the viewport's 32-byte server name.
        self.title = crate::hud::saytext::bytes(inp.server_name).into_iter().take(31).collect();
        self.rows = 0;
        self.players_info(inp);
        self.sorted = [0; NUM_ROWS];
        self.is_a_team = [0; NUM_ROWS];
        self.has_been_sorted = [false; NUM_ROWS];
        if inp.teamplay == 0 {
            self.sort_players(inp, 0, None);
        } else {
            self.sort_teams(inp);
        }
        // `m_PlayerList.SetScrollRange(m_iRows)` (`0x1005b860`).
        self.list.set_scroll_range(self.rows as i32);
        self.fill_grid(inp, sb);
    }

    /// The list's panel on a `w`×`h` screen.
    fn list_panel(w: i32, h: i32) -> Panel {
        layout(w, h).list
    }

    /// The cursor moved: the list's scroll bar slider follows it while dragged.
    pub fn scroll_cursor(&mut self, w: i32, h: i32, x: i32, y: i32) {
        self.list.cursor_moved(&Self::list_panel(w, h), x, y);
    }

    /// A press on the list's scroll bar (true if it took it).
    pub fn scroll_pressed(&mut self, w: i32, h: i32, x: i32, y: i32) -> bool {
        self.list.mouse_pressed(&Self::list_panel(w, h), x, y)
    }

    /// A release on the list's scroll bar (true if it took it).
    pub fn scroll_released(&mut self, w: i32, h: i32, x: i32, y: i32, left: bool) -> bool {
        self.list.mouse_released(&Self::list_panel(w, h), x, y, left)
    }

    /// `FillGrid` (`0x1005db70`).
    pub fn fill_grid(&mut self, inp: &Inputs, sb: &Schemes) {
        let h = inp.screen_h;
        let sfont = sb.font_of("Scoreboard Text");
        let tfont = sb.font_of("Scoreboard Title Text");
        let gap5 = xs(inp.screen_w) * 5.0;
        if !inp.squelch {
            self.highlight_row = -1;
        }
        for row in 0..NUM_ROWS {
            self.underline[row] = None;
            if row >= self.rows {
                for c in &mut self.cells[row] {
                    c.visible = false;
                }
                continue;
            }
            let kind = self.is_a_team[row];
            let index = self.sorted[row];
            for col in 0..NUM_COLUMNS {
                let mut cell = std::mem::take(&mut self.cells[row][col]);
                cell.visible = true;
                cell.text2.clear();
                cell.voice = 0;
                cell.image = None;
                cell.title_font = false;
                cell.tall = if h > 480 { ftol(ys(h) * 13.0) } else { 15 };
                cell.bg = Color::new(0, 0, 0, 255);
                let header_tall = if h >= 480 { ftol(ys(h) * 20.0) } else { 20 };
                let mut text: Vec<u8> = Vec::new();
                if kind == TEAM_BLANK {
                    text.push(b' ');
                } else {
                    match kind {
                        TEAM_YES => {
                            let t = self.teams[index as usize].teamnumber;
                            cell.fg = team_color(t, 0);
                            cell.tall = header_tall;
                            cell.title_font = true;
                            self.underline[row] = Some((ftol(ys(h) * 3.0), team_color(t, 0)));
                        }
                        TEAM_SPECTATORS => {
                            cell.fg = Color::new(100, 100, 100, 0);
                            cell.tall = header_tall;
                            cell.title_font = true;
                            self.underline[row] = Some((ftol(ys(h) * 3.0), Color::new(100, 100, 100, 0)));
                        }
                        _ => {
                            let x = &inp.extra[index as usize];
                            cell.fg = team_color(x.team, 0);
                            if inp.players[index as usize].this_player {
                                cell.fg = SC_WHITE;
                                cell.bg = team_color(x.team, 196);
                            } else if index == self.last_killed_by && self.last_kill_time != 0 && self.last_kill_time as f32 > inp.time {
                                let a = ftol(255.0 - (self.last_kill_time as f32 - inp.time) as f64 * 15.0);
                                cell.bg = Color::new(255, 0, 0, a as u8);
                            }
                        }
                    }
                    cell.align = match col {
                        COL_NAME | COL_CLASS => Align::West,
                        0 => Align::Center,
                        _ => Align::East,
                    };
                    if kind != TEAM_NO {
                        let team = &self.teams[index as usize];
                        match col {
                            COL_NAME => {
                                text = if kind == TEAM_SPECTATORS {
                                    crate::hud::saytext::bytes(&(inp.lookup)("#Spectators"))
                                } else {
                                    team_name(inp.team_names, team.teamnumber)
                                };
                                if kind == TEAM_YES {
                                    let word = if team.players == 1 { "#Player" } else { "#Player_plural" };
                                    let s = format!("({} {})", team.players, (inp.lookup)(word));
                                    cell.text2 = crate::hud::saytext::bytes(&s);
                                    self.small_font2[row] = true;
                                }
                            }
                            COL_KILLS if kind == TEAM_YES => text = fmt_d(team.frags as i32),
                            COL_DEATHS if kind == TEAM_YES => text = fmt_d(team.deaths as i32),
                            COL_LATENCY if kind == TEAM_YES => text = fmt_d(team.ping as i32),
                            _ => {}
                        }
                    } else {
                        let (p, x) = (&inp.players[index as usize], &inp.extra[index as usize]);
                        match col {
                            COL_NAME => {
                                text = p.name.clone().unwrap_or_default();
                                text.extend_from_slice(b"  ");
                            }
                            COL_CLASS => {
                                let (s, fg) = status(x);
                                text = s.as_bytes().to_vec();
                                if let Some(fg) = fg {
                                    cell.fg = fg;
                                }
                            }
                            COL_KILLS => text = fmt_d(x.frags as i16 as i32),
                            COL_DEATHS => text = fmt_d(x.deaths as i16 as i32),
                            COL_LATENCY => text = fmt_d(p.ping as i32),
                            COL_VOICE if !p.this_player || inp.spectate_only => cell.voice = index,
                            _ => {}
                        }
                    }
                }
                // `CLabelHeader::setText`: trailing spaces stripped, then the gap after the text.
                while text.last().is_some_and(|c| c.is_ascii_whitespace()) {
                    text.pop();
                }
                text.truncate(127);
                let font = if cell.title_font { tfont } else { sfont };
                cell.gap = ftol(font.map_or(0, |f| f.chars_wide(&text)) as f64 + gap5);
                cell.text = text;
                self.cells[row][col] = cell;
            }
        }
    }

    /// `CLabelHeader::paintBackground` / `paint` colours with the squelch highlight.
    fn cell_colors(&self, row: usize, cell: &Cell) -> (Color, Color) {
        if self.highlight_row == row as i32 { (Color::new(255, 255, 255, 0), Color::new(134, 91, 19, 0)) } else { (cell.fg, cell.bg) }
    }

    /// `cursorMoved` (`0x1005e7f0`, every frame from the viewport's `paintBackground`): in squelch
    /// mode, the first row whose label is under the cursor (by the bounds it last had).
    pub fn cursor_moved(&mut self, x: i32, y: i32, squelch: bool, ok: &dyn Fn(i32) -> (bool, bool, bool)) {
        if !squelch {
            return;
        }
        for row in 0..NUM_ROWS {
            if let Some(b) = self.bounds[row]
                && let Some(col) = b.iter().position(|p| p.is_within(x, y))
            {
                self.mouse_over_cell(row, col, ok);
                return;
            }
        }
    }

    /// `MouseOverCell` (`0x1005e870`): a player row lights up, unless it's the local player (and
    /// not spectate-only) or a player the server won't let us hear (TS's addition).
    /// `ok(player)` gives (connected, this player, audible); `spectate_only` folds into it.
    fn mouse_over_cell(&mut self, row: usize, col: usize, ok: &dyn Fn(i32) -> (bool, bool, bool)) {
        if self.highlight_label != Some((row, col)) {
            self.highlight_label = None;
            self.highlight_row = -1;
        }
        if self.is_a_team[row] != TEAM_NO {
            return;
        }
        let (connected, blocked_self, audible) = ok(self.sorted[row]);
        if !connected || blocked_self {
            return;
        }
        if audible {
            self.highlight_label = Some((row, col));
            self.highlight_row = row as i32;
        }
    }

    /// The player to mute or unmute for a press in squelch mode (`mousePressed` `0x1005e660`).
    pub fn highlighted_player(&self) -> Option<i32> {
        (self.highlight_row >= 0).then(|| self.sorted[self.highlight_row as usize]).filter(|&p| p > 0)
    }

    /// Whether (`x`, `y`) is on a visible player-entry label (they carry the panel's input signal;
    /// the title and header labels don't).
    pub fn on_entry(&self, x: i32, y: i32) -> bool {
        (0..self.rows).filter(|&r| self.shown[r]).any(|r| {
            self.cells[r]
                .iter()
                .zip(self.bounds[r].iter().flatten())
                .any(|(c, p)| c.visible && p.clip.x0 <= x && x < p.clip.x1 && p.clip.y0 <= y && y < p.clip.y1)
        })
    }
}

/// `GetTeamName(n)`: an empty name outside the four the viewport stores.
fn team_name(names: &[String], n: i32) -> Vec<u8> {
    if (1..=4).contains(&n) { names.get(n as usize - 1).map(|s| crate::hud::saytext::bytes(s)).unwrap_or_default() } else { Vec::new() }
}

/// TS's status column (`0x1005e213`): the `TSPAward` byte's title and colour (VGUI alpha 5),
/// else "WAITING" for a spectator (`0x1005e3e8`).
pub fn status(x: &PlayerExtra) -> (&'static str, Option<Color>) {
    match x.award {
        1 => ("DEAD", None),
        2 => ("3 KILLS", Some(Color::new(125, 255, 125, 5))),
        3 => ("6 KILLS", Some(Color::new(135, 255, 135, 5))),
        4 => ("SPECIALIST", Some(Color::new(145, 255, 145, 5))),
        5 => ("UNSTOPPABLE", Some(Color::new(155, 255, 155, 5))),
        10 => ("THE ONE", Some(Color::new(175, 255, 175, 5))),
        11 => ("WAITING", None),
        _ if x.observer => ("WAITING", None),
        _ => ("", None),
    }
}

/// `UpdateBanButton`'s result: the image and, when it sets one, the label's foreground.
pub type VoiceIcon = (Option<Rc<Image>>, Option<Color>);

/// The scoreboard panel's geometry for a screen (`CreateScoreBoard` and the constructor).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    pub panel: Panel,
    pub title: Panel,
    pub header: Panel,
    pub widths: [i32; NUM_COLUMNS],
    pub header_tall: i32,
    pub list: Panel,
    /// The list's items panel (15 pixels narrower: the scroll bar's room).
    pub items: Panel,
}

pub fn layout(w: i32, h: i32) -> Layout {
    let (xsc, ysc) = (xs(w), ys(h));
    let (mut xdent, mut ydent) = (ftol(xsc * 104.0), ftol(ysc * 40.0));
    if w == 512 {
        (xdent, ydent) = (30, 30);
    } else if w == 400 {
        (xdent, ydent) = (0, 20);
    }
    let (wide, tall) = (w - xdent * 2, h - ydent * 2);
    let panel = Panel::root(w, h).child(xdent, ydent, wide, tall).bordered();
    let xpos = if w >= 640 { ftol(xsc * 27.0) } else { 27 };
    let title = panel.child(xpos, 4, wide, ftol(ysc * 22.0));
    let mut widths = [0; NUM_COLUMNS];
    for (i, &(_, cw)) in COLUMNS.iter().enumerate() {
        widths[i] = if w >= 640 {
            ftol(cw as f64 * xsc)
        } else if w == 400 && i == 1 {
            cw - 28
        } else if w == 400 && i == 0 {
            cw - 8
        } else {
            cw
        };
    }
    let end6: i32 = widths[..7].iter().sum();
    widths[7] = ftol(wide as f64 - xsc * 4.0 - end6 as f64);
    let header_tall = if h >= 480 { ftol(ysc * 12.0) } else { 12 };
    let x4 = spill(xsc * 4.0);
    let (hx, hy) = (ftol(x4), ftol(ysc * 22.0));
    let header = panel.child(hx, hy, ftol(wide as f64 - x4 * 2.0), header_tall);
    let list = panel.child(hx, hy + header_tall, header.w, tall - hy - header_tall - 6);
    let items = list.child(0, 0, list.w - 15, list.h);
    Layout { panel, title, header, widths, header_tall, list, items }
}

/// The column offsets (`CGrid::CalcColOffsets` with no spacing).
fn col_x(widths: &[i32; NUM_COLUMNS], col: usize) -> i32 {
    widths[..col].iter().sum()
}

/// `CLabelHeader::calcAlignment`: the content `iw`×`ih` in a `w`×`h` label (x never negative).
pub fn header_align(align: Align, w: i32, h: i32, iw: i32, ih: i32) -> (i32, i32) {
    let a = align as i32;
    let x = match a % 3 {
        0 => 0,
        1 => (w - iw) / 2,
        _ => w - iw,
    };
    let y = match a / 3 {
        0 => 0,
        1 => (h - ih) / 2,
        _ => h - ih,
    };
    (x.max(0), y)
}

/// `CLabelHeader::paint`'s two texts (`CTextImage2`): the first at the alignment of both together,
/// the second at the gap, its bottom at 90% of the first's height. The second text's font is the
/// label's (the small font on team rows) or VGUI's default `sf_primary1` when never set, and it
/// counts for the height either way.
#[allow(clippy::too_many_arguments)] // the label's panel, fonts, texts and colour
fn paint_dual(
    s: &mut dyn Surface,
    p: &Panel,
    f1: &BitmapFont,
    f2: Option<&BitmapFont>,
    text: &[u8],
    text2: &[u8],
    gap: i32,
    align: Align,
    fg: Color,
) {
    let (w1, t1) = f1.text_size(text);
    let (w2, t2) = f2.map_or((0, 0), |f| f.text_size(text2));
    let (x, y) = header_align(align, p.w, p.h, w1 + w2, t1.max(t2));
    p.text(s, f1, x, y, fg, text, w1);
    if let Some(f2) = f2 {
        let y2 = (y as f64 + t1 as f64 * 0.9 - t2 as f64) as i32;
        p.text(s, f2, gap, y2, fg, text2, w2);
    }
}

impl ScoreBoard {
    /// Paints the board (the viewport's paint pass): the panel, title, header grid and rows.
    /// `voice(player)` is `UpdateBanButton` (run every frame, `0x10017ef6`): the voice column's
    /// new image and the label foreground it sets, or `None` to leave both.
    pub fn paint(
        &mut self,
        s: &mut dyn Surface,
        sb: &Schemes,
        w: i32,
        h: i32,
        voice: &mut dyn FnMut(i32) -> Option<VoiceIcon>,
        arrows: &[Option<Rc<Image>>; 2],
    ) {
        let l = layout(w, h);
        let vs = ViewportScheme::from(sb);
        // `ScorePanel`: grey line border, black background at VGUI alpha 96.
        l.panel.line_border(s, Color::new(60, 60, 60, 128));
        l.panel.paint_background(s, Color::new(0, 0, 0, 96));
        let (Some(sfont), Some(tfont), Some(small)) =
            (sb.font_of("Scoreboard Text"), sb.font_of("Scoreboard Title Text"), sb.font_of("Scoreboard Small Text"))
        else {
            return;
        };
        let default_font = sb.font_of("Primary Button Text");
        // Title: west-aligned in its own bounds, `sc_primary1`.
        let (tw, th) = tfont.text_size(&self.title);
        let (tx, ty) = vgui::label_text_pos(Align::West, l.title.w, l.title.h, tw, th);
        l.title.text(s, tfont, tx, ty, vs.primary1, &self.title, tw);
        // Header labels.
        for (col, &(title, _)) in COLUMNS.iter().enumerate() {
            let p = l.header.child(col_x(&l.widths, col), 0, l.widths[col], l.header_tall);
            let text = title.map(|t| crate::hud::saytext::bytes(&crate::hud::text::lookup(t))).unwrap_or_default();
            paint_dual(s, &p, small, default_font, &text, b"", 0, Align::East, vs.primary1);
        }
        // Rows: `CListBox::InternalLayout` stacks the shown grids (from the scroll offset up to
        // the row count) from the top of the items panel; hidden ones keep their old bounds.
        let heights: Vec<i32> = self.cells.iter().map(|r| r.iter().map(|c| c.tall).max().unwrap_or(0)).collect();
        let ys = self.list.layout(l.list.w, l.list.h, &heights);
        self.shown = ys.iter().map(Option::is_some).collect();
        for row in 0..NUM_ROWS {
            let Some(y) = ys[row] else { continue };
            let tall = heights[row];
            let grid = l.items.child(0, y, l.list.w, tall);
            if let Some((off, c)) = self.underline[row] {
                // `CGrid::paint`, before the labels: one line `off` pixels above the row's bottom.
                grid.fill_rect(s, 0, tall - (off + 1), grid.w, tall - off, c);
            }
            let b: [Panel; NUM_COLUMNS] = std::array::from_fn(|col| grid.child(col_x(&l.widths, col), 0, l.widths[col], tall));
            for (col, p) in b.iter().enumerate() {
                if !self.cells[row][col].visible {
                    continue;
                }
                let player = self.cells[row][col].voice;
                if player > 0
                    && let Some((img, fg)) = voice(player)
                {
                    let c = &mut self.cells[row][col];
                    c.image = img;
                    if let Some(fg) = fg {
                        c.fg = fg;
                    }
                }
                let cell = &self.cells[row][col];
                let (fg, bg) = self.cell_colors(row, cell);
                p.paint_background(s, bg);
                let f1 = if cell.title_font { tfont } else { sfont };
                let f2 = if col == COL_NAME && self.small_font2[row] { Some(small) } else { default_font };
                paint_dual(s, p, f1, f2, &cell.text, &cell.text2, cell.gap, cell.align, fg);
                if let Some(img) = &cell.image {
                    let (ix, iy) = header_align(cell.align, p.w, p.h, img.wide, img.tall);
                    p.image(s, img, ix, iy, fg);
                }
            }
            self.bounds[row] = Some(b);
        }
        self.list.paint(s, &l.list, arrows);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny xorshift for the fuzz tests.
    fn xorshift(s: &mut u64) -> u64 {
        *s ^= *s << 13;
        *s ^= *s >> 7;
        *s ^= *s << 17;
        *s
    }

    /// Players with any team numbers, frags, names and awards; kills with any indices; team
    /// scores for names nobody has.
    #[test]
    fn garbage_scoreboard_input() {
        let mut s = 0x5eed_5eed_5eed_5eedu64;
        let sb = Schemes::load(640, false, &|_| None);
        let mut b = ScoreBoard::default();
        let long = "\u{e9}".repeat(20);
        for round in 0..30 {
            let mut players = vec![PlayerSlot::default(); 65];
            let mut extra = vec![PlayerExtra::default(); 65];
            for i in 1..65 {
                let r = xorshift(&mut s);
                if r & 1 != 0 {
                    players[i] = PlayerSlot {
                        name: Some(vec![0xff; (r % 50) as usize]),
                        ping: r as i16,
                        packetloss: (r >> 8) as u8,
                        this_player: r.is_multiple_of(7),
                        entity: r & 2 != 0,
                    };
                }
                extra[i].team = [i32::MIN, -1, 0, 1, 2, 5, i32::MAX][(r % 7) as usize];
                extra[i].frags = (r >> 16) as i32;
                extra[i].deaths = (r >> 32) as i32;
                extra[i].award = (r >> 40) as u8;
                extra[i].teamname = ["", "A", "b", long.as_str()][(r >> 48) as usize % 4].into();
                if r.is_multiple_of(11) {
                    extra[i].teamname = format!("team{i}");
                }
            }
            let names: Vec<String> = (0..10).map(|i| format!("n{i}")).collect();
            let mut inp = inputs(&players, &extra, round % 3);
            inp.team_names = &names;
            inp.squelch = round & 1 == 0;
            b.rebuild_teams(&inp, &sb);
            b.team_score("nobody", i16::MIN, i16::MAX);
            b.team_score("A", i16::MIN, i16::MAX);
            b.death_msg(xorshift(&mut s) as i32, xorshift(&mut s) as i32, f32::MAX);
            b.death_msg(0, b.player_num, f32::NAN);
            b.update(&inp, &sb);
            b.cursor_moved(xorshift(&mut s) as i32 % 700, xorshift(&mut s) as i32 % 500, true, &|_| (true, false, true));
            let _ = b.highlighted_player();
            let _ = b.on_entry(300, 200);
            let mut voice = |_: i32| Some((None, Some(Color::new(1, 2, 3, 4))));
            b.paint(&mut crate::hud::vgui::tests::Record::default(), &sb, 640, 480, &mut voice, &[None, None]);
            b.scroll_pressed(640, 480, 530, 200);
            b.scroll_cursor(640, 480, 530, 400);
            b.scroll_released(640, 480, 530, 470, true);
        }
    }

    fn slot(name: &str, this: bool) -> PlayerSlot {
        PlayerSlot { name: Some(name.as_bytes().to_vec()), ping: 10, packetloss: 0, this_player: this, entity: true }
    }

    fn inputs<'a>(players: &'a [PlayerSlot], extra: &'a [PlayerExtra], teamplay: i32) -> Inputs<'a> {
        Inputs {
            players,
            extra,
            teamplay,
            server_name: "Server",
            team_names: &[],
            lookup: &|s| s.to_owned(),
            time: 0.0,
            spectate_only: false,
            squelch: false,
            screen_h: 480,
            screen_w: 640,
        }
    }

    fn rows(b: &ScoreBoard) -> Vec<String> {
        (0..b.rows)
            .map(|r| match b.is_a_team[r] {
                TEAM_YES => format!("T{}", b.teams[b.sorted[r] as usize].name),
                TEAM_SPECTATORS => "S".into(),
                TEAM_BLANK => "-".into(),
                _ => b.sorted[r].to_string(),
            })
            .collect()
    }

    #[test]
    fn sorts_teams_and_players_like_ts() {
        let mut players = vec![PlayerSlot::default(); 65];
        let mut extra = vec![PlayerExtra::default(); 65];
        for (i, (f, d, t)) in [(1, (5, 1, "A")), (2, (9, 0, "B")), (3, (5, 0, "a")), (4, (0, 0, ""))] {
            players[i] = slot(&format!("p{i}"), i == 1);
            extra[i].frags = f;
            extra[i].deaths = d;
            extra[i].teamname = t.into();
            extra[i].team = if t == "B" { 2 } else { 1 };
        }
        let sb = Schemes::load(640, false, &|_| None);
        let mut b = ScoreBoard::default();
        b.rebuild_teams(&inputs(&players, &extra, 1), &sb);
        assert_eq!((b.num_teams, b.teams[1].name.as_str(), b.teams[1].players, b.teams[2].players), (2, "A", 2, 1));
        assert_eq!(b.player_num, 1);
        assert_eq!((b.teams[1].frags, b.teams[2].frags), (10, 9));
        assert_eq!(rows(&b), ["TA", "3", "1", "-", "TB", "2", "-", "S", "4"], "the teamless player goes under the spectators");
        // A TeamScore override sticks; game modes above 1 don't sum frags.
        b.team_score("b", 20, 1);
        b.update(&inputs(&players, &extra, 1), &sb);
        assert_eq!(rows(&b)[0], "TB");
        assert_eq!(b.teams[2].deaths, 0, "deaths are summed even with an override");
        b.update(&inputs(&players, &extra, 2), &sb);
        assert_eq!(b.teams[1].frags, 0);
        b.team_score("nobody", 1, 1);
        // Deathmatch: one list, no headers.
        b.update(&inputs(&players, &extra, 0), &sb);
        assert_eq!(rows(&b), ["2", "3", "1", "4"]);
    }

    #[test]
    fn rebuild_keeps_slot_data() {
        let mut players = vec![PlayerSlot::default(); 65];
        let mut extra = vec![PlayerExtra::default(); 65];
        players[1] = slot("a", false);
        extra[1].teamname = "Red".into();
        players[2] = slot("b", false);
        extra[2].teamname = "Blue".into();
        let sb = Schemes::load(640, false, &|_| None);
        let mut b = ScoreBoard::default();
        b.rebuild_teams(&inputs(&players, &extra, 1), &sb);
        b.team_score("Red", 7, 0);
        // Red's only player leaves: Blue moves to slot 1 and inherits the override.
        players[1].name = None;
        b.rebuild_teams(&inputs(&players, &extra, 1), &sb);
        assert_eq!((b.num_teams, b.teams[1].name.as_str(), b.teams[1].scores_overriden), (1, "Blue", true));
    }

    #[test]
    fn grid_cells() {
        let mut players = vec![PlayerSlot::default(); 65];
        let mut extra = vec![PlayerExtra::default(); 65];
        players[1] = slot("me", true);
        players[2] = slot("you", false);
        extra[1].team = 1;
        extra[2].team = 1;
        extra[2].award = 10;
        extra[1].teamname = "A".into();
        extra[2].teamname = "A".into();
        let sb = Schemes::load(640, false, &|_| None);
        let mut b = ScoreBoard::default();
        let mut inp = inputs(&players, &extra, 1);
        let names = ["Alpha".to_owned()];
        inp.team_names = &names;
        b.rebuild_teams(&inp, &sb);
        // Team row: team-coloured, the title font, the team name and "(2 …)", underlined.
        let team = &b.cells[0];
        assert!(team[1].title_font && team[1].tall == 20);
        assert_eq!(team[1].text, b"Alpha");
        assert!(team[1].text2.starts_with(b"(2 "));
        assert_eq!(team[1].fg, team_color(1, 0));
        assert_eq!(b.underline[0], Some((3, team_color(1, 0))));
        // Rows: "you" first (more frags? equal: fewer deaths, then index order).
        let me = (1..b.rows).find(|&r| b.sorted[r] == 1).unwrap();
        let you = (1..b.rows).find(|&r| b.sorted[r] == 2).unwrap();
        assert_eq!(b.cells[me][1].text, b"me", "trailing spaces stripped");
        assert_eq!((b.cells[me][1].fg, b.cells[me][1].bg), (SC_WHITE, team_color(1, 196)));
        assert_eq!(b.cells[me][6].voice, 0, "no voice icon on the local row");
        assert_eq!(b.cells[you][6].voice, 2);
        assert_eq!((b.cells[you][2].text.as_slice(), b.cells[you][2].fg), (&b"THE ONE"[..], Color::new(175, 255, 175, 5)));
        assert_eq!((b.cells[you][0].align, b.cells[you][1].align, b.cells[you][3].align), (Align::Center, Align::West, Align::East));
        assert_eq!(b.cells[you][5].text, b"10");
        assert!(!b.cells[b.rows][0].visible);
        // The killer's row fades over 10 seconds.
        b.death_msg(2, 1, 100.0);
        assert_eq!((b.last_killed_by, b.last_kill_time), (2, 110));
        inp.time = 104.0;
        b.update(&inp, &sb);
        let you = (1..b.rows).find(|&r| b.sorted[r] == 2).unwrap();
        assert_eq!(b.cells[you][1].bg, Color::new(255, 0, 0, 165));
        b.death_msg(2, 3, 0.0);
        assert_eq!(b.last_killed_by, 2, "someone else's death doesn't count");
        b.death_msg(0, 3, 0.0);
        assert_eq!(b.last_killed_by, 1, "a world kill marks the local player");
    }

    #[test]
    fn status_column() {
        let mut x = PlayerExtra { award: 1, observer: true, ..PlayerExtra::default() };
        assert_eq!(status(&x).0, "DEAD", "the award wins over the spectator flag");
        x.award = 0;
        assert_eq!(status(&x).0, "WAITING");
        x.observer = false;
        x.award = 7;
        assert_eq!(status(&x).0, "");
    }

    #[test]
    fn layout_maths() {
        let l = layout(640, 480);
        assert_eq!((l.panel.x, l.panel.y, l.panel.w, l.panel.h), (104, 40, 432, 400));
        assert_eq!(l.widths, [24, 140, 56, 40, 46, 46, 40, 35], "TS's 1/640 is a float a hair above it");
        assert_eq!((l.title.x, l.title.y, l.title.h), (104 + 1 + 27, 40 + 1 + 4, 22));
        assert_eq!((l.header.x, l.header.y, l.header.w, l.header.h), (109, 63, 424, 12));
        assert_eq!((l.list.y, l.list.h, l.items.w), (75, 400 - 22 - 12 - 6, 409));
        let l = layout(1024, 768);
        assert_eq!((l.panel.x, l.panel.y), (166, 64));
        assert_eq!(l.widths[..7], [38, 224, 89, 64, 73, 73, 64]);
        assert_eq!(layout(512, 384).panel.x, 30);
        assert_eq!(layout(400, 300).widths[..2], [16, 112]);
        assert_eq!(header_align(Align::East, 40, 12, 50, 16), (0, -2), "x clamps, y doesn't");
    }

    #[test]
    fn squelch_highlight() {
        let mut b = ScoreBoard { rows: 2, ..ScoreBoard::default() };
        b.is_a_team[0] = TEAM_YES;
        b.sorted[1] = 5;
        let p = Panel::root(640, 480).child(0, 0, 10, 10);
        b.bounds[0] = Some([p; NUM_COLUMNS]);
        b.bounds[1] = Some([Panel::root(640, 480).child(0, 10, 10, 10); NUM_COLUMNS]);
        let ok = |pl: i32| (pl == 5, false, true);
        b.cursor_moved(5, 15, true, &ok);
        assert_eq!((b.highlight_row, b.highlighted_player()), (1, Some(5)));
        b.cursor_moved(5, 5, true, &ok);
        assert_eq!(b.highlight_row, -1, "a team row clears it");
        b.cursor_moved(5, 15, false, &ok);
        assert_eq!(b.highlight_row, -1, "only in squelch mode");
        let deaf = |_: i32| (true, false, false);
        b.cursor_moved(5, 15, true, &deaf);
        assert_eq!(b.highlight_row, -1, "inaudible players don't light up");
    }
}
