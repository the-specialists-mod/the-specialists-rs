//! Immediate-mode menus drawn with the engine: TS's buy menu (which TS also draws this way), and
//! replacements for the VGUI1 viewport panels TS used: scoreboard, team select and the MOTD
//! window. Owner: HUD/UI work package. Reference: decomp docs/client/architecture-hud.md §6.
//!
//! The mouse: while a menu shows the cursor, `input` moves a virtual cursor instead of the view
//! and routes clicks here (TS: `IN_MouseEvent` → `CTSBuyMenu::MouseEvent`).

pub mod buy;
mod scoreboard;

use crate::hud::{self, Dc, draw::half, text::localise};
use buy::{Action, BuyMenu, Rng, Rules, page};
use goldsrc::Global;
use goldsrc::client::{Cvar, engine};
use std::cell::{Cell, RefCell};
use std::ffi::CStr;

#[derive(Default)]
struct Menus {
    buy: BuyMenu,
    /// Clickable rows from the last draw: (x, y, w, h, page, key).
    rows: Vec<(i32, i32, i32, i32, i32, i32)>,
    team_open: bool,
    team_rows: Vec<(i32, i32, i32, i32, i32)>,
    motd_open: bool,
    motd_ok: (i32, i32, i32, i32),
    scores: bool,
    path: String,
    /// Scoreboard player rows from the last draw: (x, y, w, h, player index).
    score_rows: Vec<(i32, i32, i32, i32, i32)>,
}

static MENUS: Global<RefCell<Option<Menus>>> = Global(RefCell::new(None));
static INTERMISSION: Global<Cell<bool>> = Global(Cell::new(false));

fn with<R>(f: impl FnOnce(&mut Menus) -> R) -> R {
    let mut m = MENUS.0.borrow_mut();
    f(m.get_or_insert_with(Menus::default))
}

struct EngineRng;
impl Rng for EngineRng {
    fn long(&mut self, lo: i32, hi: i32) -> i32 {
        goldsrc::ext::render::random_long(lo, hi)
    }
    fn float(&mut self, lo: f32, hi: f32) -> f32 {
        goldsrc::ext::render::random_float(lo, hi)
    }
}

fn rules() -> Rules {
    let (gamemode, restriction, ammocount) = hud::with(|h| (h.gamemode, h.restriction, h.ammocount));
    Rules { gamemode, restriction, ammocount, buy_kevlar: crate::input::cvars().cl_buykevlar.value() != 0.0 }
}

pub fn init() {
    // `Scheme_Init` (1005c2a0), right after CHud::Init. TS's VGUI scheme manager reads it to pick
    // bitmap fonts; the port draws its menus without VGUI, so it's registered only.
    Cvar::register(c"bitmapfonts", c"1", 0);
    // `FUN_10036480`: <gamedir>/tsbuylist.tsa (nine saved configs), created if missing.
    let path = format!("{}/tsbuylist.tsa", engine().game_directory());
    let saved = std::fs::read(&path).map(|b| buy::parse_buylist(&b)).unwrap_or_default();
    with(|m| {
        m.buy = BuyMenu { saved, ..BuyMenu::default() };
        m.path = path;
    });
}

pub fn vid_init(_first: bool) {
    with(|m| {
        m.buy.close();
        m.team_open = false;
        m.motd_open = false;
    });
}

/// `FUN_10036560`: write the nine configs. Written to a temporary file and renamed, so a
/// symlinked content file (xtask installs) is replaced rather than written through.
fn save_file() {
    let (path, bytes) = with(|m| (m.path.clone(), buy::write_buylist(&m.buy.saved)));
    let tmp = format!("{path}.tmp");
    if std::fs::write(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, &path)).is_err() {
        engine().print(&format!("[ts] couldn't write {path}\n"));
    }
}

fn run_actions(actions: Vec<Action>) {
    let e = engine();
    for a in actions {
        match a {
            Action::ServerCmd(c) => e.server_cmd(&c),
            Action::Message(t, rgb) => add_message(&localise(&t), rgb),
            Action::MessageArg(t, arg, rgb) => {
                let f = localise(&t);
                let arg_i = arg.parse::<i64>().ok();
                let s = hud::text::format(&f, &[arg_i.map_or(hud::text::Arg::S(&arg), hud::text::Arg::I)]);
                add_message(&s, rgb)
            }
            Action::Close => {}
            Action::SaveFile => save_file(),
        }
    }
}

/// `AddTSMessage(text, rgb, 20 tenths, 0)`.
fn add_message(text: &str, rgb: [i32; 3]) {
    let screen = hud::screen();
    let ctx = hud::state::Ctx {
        now: hud::now(),
        screen: &screen,
        titles: &hud::text::EngineTitles,
        local_index: 0,
        local_name: String::new(),
        third_person: false,
        thirdperson_switch: false,
        saytext_time: 5.0,
        deathnotice_time: 6.0,
    };
    hud::with(|h| h.add_ts_message(&ctx, text, rgb, 20, 0));
}

// ---- buy menu API ----------------------------------------------------------------------------

pub fn buy_open() -> bool {
    with(|m| m.buy.active)
}

/// `CTSBuyMenu::Open` (10036670): refused during demo playback; centres the cursor.
pub fn open_buy() {
    if engine().demo_playing() {
        return;
    }
    let r = rules();
    let opened = with(|m| {
        let was = m.buy.active;
        m.buy.open(&r, &mut EngineRng);
        !was && m.buy.active
    });
    if opened {
        crate::input::center_cursor();
    }
}

pub fn buy_key(key: i32) {
    let r = rules();
    let actions = with(|m| {
        let pg = m.buy.page;
        m.buy.handle_key(pg, key, &r, &mut EngineRng)
    });
    run_actions(actions);
}

// ---- team menu (VGUIMenu 2 → CTeamMenuPanel) -------------------------------------------------

pub fn open_team_menu() {
    with(|m| m.team_open = true);
    crate::input::center_cursor();
}

/// Team menu keys: 1..4 `jointeam n`, 5 `jointeam 5` (auto assign), 10 cancel.
pub fn team_menu_key(n: i32) -> bool {
    if !with(|m| m.team_open) {
        return false;
    }
    let teams = hud::with(|h| h.team_names.len().clamp(2, 4)) as i32;
    match n {
        1..=4 if n <= teams => engine().server_cmd(&format!("jointeam {n}")),
        5 => engine().server_cmd("jointeam 5"),
        10 => {}
        _ => return true,
    }
    with(|m| m.team_open = false);
    true
}

// ---- MOTD ------------------------------------------------------------------------------------

pub fn open_motd() {
    with(|m| m.motd_open = true);
}

// ---- scoreboard ------------------------------------------------------------------------------

pub fn show_scores(on: bool) {
    with(|m| m.scores = on);
    if !on {
        // `HideScoreBoard` (10063760) ends squelch mode.
        crate::voice::stop_squelch_mode();
    }
}

/// A mouse press while the scoreboard is up and has no cursor (`ScorePanel::mousePressed`
/// 1005e660): the first press enters squelch mode, which shows the cursor. Returns true if the
/// press was taken (it doesn't reach the engine as a key).
pub fn scoreboard_press() -> bool {
    if !scores_visible() || wants_cursor() || hud::intermission() {
        return false;
    }
    crate::voice::start_squelch_mode();
    crate::input::center_cursor();
    true
}

/// The scoreboard player row under the cursor that can be muted (`MouseOverCell` 1005e870: only
/// players the server lets us hear).
fn squelch_row(m: &Menus, x: i32, y: i32) -> Option<(i32, i32, i32, i32, i32)> {
    m.score_rows.iter().copied().find(|r| hit((r.0, r.1, r.2, r.3), x, y) && crate::voice::is_player_audible(r.4))
}

pub fn scores_visible() -> bool {
    with(|m| m.scores) || INTERMISSION.0.get()
}

/// Intermission: TS hides VGUI menus and shows the scoreboard; leaving hides it.
pub fn intermission(on: bool) {
    INTERMISSION.0.set(on);
    if on {
        with(|m| {
            m.buy.close();
            m.team_open = false;
            m.motd_open = false;
        });
    }
}

// ---- input -----------------------------------------------------------------------------------

/// A menu owns the mouse (virtual cursor, clicks).
pub fn wants_cursor() -> bool {
    with(|m| m.buy.active || m.team_open || m.motd_open) || squelching()
}

/// Squelch mode with the scoreboard showing: the scoreboard owns the cursor.
fn squelching() -> bool {
    crate::voice::in_squelch_mode() && scores_visible()
}

const K_ENTER: i32 = 13;
const K_ESCAPE: i32 = 27;
const K_SPACE: i32 = 32;
const K_KP_ENTER: i32 = 172;

/// Returns true if a menu consumed the key.
pub fn key_event(down: bool, key: i32, _binding: &CStr) -> bool {
    if !down {
        return false;
    }
    let (motd, buy_open, team) = with(|m| (m.motd_open, m.buy.active, m.team_open));
    if motd && matches!(key, K_ENTER | K_ESCAPE | K_SPACE | K_KP_ENTER) {
        with(|m| m.motd_open = false);
        return true;
    }
    if key == K_ESCAPE && buy_open {
        with(|m| m.buy.close());
        return true;
    }
    if key == K_ESCAPE && team {
        with(|m| m.team_open = false);
        return true;
    }
    false
}

/// Mouse buttons while a menu has the cursor (`CTSBuyMenu::MouseEvent` 10038360): left release
/// selects the row under the cursor, right release is key 10 (back).
pub fn mouse_buttons(state: i32, old: i32, x: i32, y: i32) {
    let pressed = state & !old;
    if squelching() && !with(|m| m.buy.active || m.team_open || m.motd_open) {
        // `ScorePanel::mousePressed`: any button toggles the highlighted player's mute.
        if pressed != 0
            && !hud::intermission()
            && let Some(r) = with(|m| squelch_row(m, x, y))
        {
            crate::voice::scoreboard_toggle(r.4);
        }
        return;
    }
    let released = old & !state;
    if released & 1 != 0 {
        click(x, y);
    } else if released & 2 != 0 {
        if buy_open() {
            buy_key(10);
        } else {
            with(|m| {
                m.team_open = false;
                m.motd_open = false;
            });
        }
    }
}

fn hit(r: (i32, i32, i32, i32), x: i32, y: i32) -> bool {
    x >= r.0 && x < r.0 + r.2 && y >= r.1 && y < r.1 + r.3
}

fn click(x: i32, y: i32) {
    let (motd_ok, motd) = with(|m| (m.motd_ok, m.motd_open));
    if motd {
        if hit(motd_ok, x, y) {
            with(|m| m.motd_open = false);
        }
        return;
    }
    if let Some(key) = with(|m| m.team_rows.iter().find(|r| m.team_open && hit((r.0, r.1, r.2, r.3), x, y)).map(|r| r.4)) {
        team_menu_key(key);
        return;
    }
    let row = with(|m| m.rows.iter().rev().find(|r| hit((r.0, r.1, r.2, r.3), x, y)).copied());
    if let Some((_, _, _, _, pg, key)) = row {
        let r = rules();
        let actions = with(|m| m.buy.handle_key(pg, key, &r, &mut EngineRng));
        run_actions(actions);
    }
}

// ---- drawing ---------------------------------------------------------------------------------

/// A TS menu column (`FUN_1004fed0` / `FUN_1004ff10`): rows of 128×20, a translucent box per row.
struct Column<'a> {
    dc: &'a Dc<'a>,
    x: i32,
    y: i32,
    page: i32,
    dim: bool,
    rows: &'a mut Vec<(i32, i32, i32, i32, i32, i32)>,
}

impl Column<'_> {
    fn row(&mut self, text: &str, rgb: [i32; 3], key: i32) {
        let (w, h) = (128, 20);
        let a = if self.dim { 25 } else { 50 };
        self.dc.fill(self.x, self.y, w - 1, h - 1, rgb, a);
        let t = rgb.map(|c| (c * 2).min(255));
        let t = if self.dim { half(t) } else { t };
        self.dc.string(self.x, self.y, &localise(text), t);
        if key > 0 {
            self.rows.push((self.x, self.y, w, h, self.page, key));
        }
        self.y += h;
    }
}

const RED: [i32; 3] = [195, 45, 45];
const GREEN: [i32; 3] = [45, 195, 45];
const BACK: [i32; 3] = [100, 100, 205];

fn draw_buy(dc: &Dc, b: &BuyMenu, r: &Rules, rows: &mut Vec<(i32, i32, i32, i32, i32, i32)>) {
    let hud2 = half(dc.hud);
    let y0 = dc.h() / 2;
    let fmt = hud::text::format;
    use hud::text::Arg::I;
    if b.page == page::TEAM {
        let mut c = Column { dc, x: 8, y: y0, page: page::TEAM, dim: false, rows };
        c.row("#Chg_ChangeTeam", RED, 0);
        c.row("1 #Chg_Join Specialists", hud2, 1);
        c.row("2 #Chg_Join Mercenaries", hud2, 2);
        c.row("0 #Chg_Cancel", BACK, 10);
        return;
    }
    draw_summary(dc, b, r);
    let cur = &b.current;
    let affordable = |cost: i32, slots: i32| cost + cur.total <= buy::CASH && slots + cur.slots <= buy::SLOTS;

    // Columns are drawn child-first, parents after (each TS page function calls its parent).
    if b.page == page::OPTIONS {
        let id = b.pending_weapon;
        let base = buy::cost(id, b.pending_att, false, false);
        let mut c = Column { dc, x: 0x188, y: y0, page: page::OPTIONS, dim: false, rows };
        c.row("1 #Buy_Buy", if affordable(base, buy::slots_for(id, false, false)) { GREEN } else { RED }, 1);
        let labels = ["#HUD_Silencer", "#HUD_Lasersight", "#HUD_Flashlight", "#HUD_Scope"];
        let prices = [900, 800, 500, 1800];
        for bit in 0..4 {
            if b.attachment_allowed(1 << bit) {
                let price = base + prices[bit];
                let text = fmt(&format!("%i {}  %i", labels[bit]), &[I(bit as i64 + 2), I(price as i64)]);
                c.row(&text, if price + cur.total <= buy::CASH { hud2 } else { RED }, bit as i32 + 2);
            }
        }
        if let Some(w) = shared::weapons::tables().weapon(id.max(0) as u8) {
            if w.akimbo != -1 {
                let ok = affordable(buy::cost(id, b.pending_att, true, false), w.slots * 2);
                c.row("6 #Buy_Akimbo", if ok { GREEN } else { RED }, 6);
            } else if w.stack {
                let ok = affordable(buy::cost(id, 0, true, false), w.slots * w.pellets);
                c.row("6 #Buy_FullLoad", if ok { GREEN } else { RED }, 6);
            }
        }
        c.row("0 #Buy_GoBack", BACK, 10);
    }
    if matches!(b.page, page::WEAPONS | page::OPTIONS) {
        let mut c = Column { dc, x: 0x108, y: y0, page: page::WEAPONS, dim: b.page != page::WEAPONS, rows };
        for row in 0..12 {
            let id = buy::TABLE[row][b.category as usize];
            if id == -1 || buy::restricted(id, false, r) {
                continue;
            }
            let Some(w) = shared::weapons::tables().weapon(id as u8) else { continue };
            let (key, ch) = if row < 9 { (row as i32 + 1, (b'1' + row as u8) as char) } else { (11, 'a') };
            let ok = affordable(w.cost, w.slots);
            c.row(&format!("{ch} {}", w.name), if ok { hud2 } else { RED }, key);
        }
        if b.page == page::WEAPONS {
            c.row("0 #Buy_GoBack", BACK, 10);
        }
        if b.pending_weapon >= 0 {
            draw_weapon_box(dc, b.pending_weapon);
        }
    }
    if b.page == page::ITEMS {
        let mut c = Column { dc, x: 0x108, y: y0, page: page::ITEMS, dim: false, rows };
        for (key, id, label, price, slots) in
            [(1i32, buy::ITEM_KEVLAR, "Kevlar", 1900, 10), (2, buy::ITEM_STEALTH, "Stealth Shoes", 700, 0)]
        {
            if buy::restricted(id, true, r) {
                continue;
            }
            let text = fmt(&format!("%i {label}  %i"), &[I(key as i64), I(price as i64)]);
            c.row(&text, if affordable(price, slots) { hud2 } else { RED }, key);
        }
        c.row("0 #Buy_GoBack", BACK, 10);
    }
    if matches!(b.page, page::CATEGORY | page::WEAPONS | page::OPTIONS | page::ITEMS) {
        let mut c = Column { dc, x: 0x88, y: y0, page: page::CATEGORY, dim: b.page != page::CATEGORY, rows };
        for (i, name) in buy::CATEGORIES.iter().enumerate() {
            c.row(&format!("{} {name}", i + 1), hud2, i as i32 + 1);
        }
        c.row("6 #Buy_PlayerAccessory", hud2, 6);
        if b.page == page::CATEGORY {
            c.row("0 #Buy_GoBack", BACK, 10);
        }
    }
    if b.page == page::REMOVE {
        let mut c = Column { dc, x: 0x88, y: y0, page: page::REMOVE, dim: false, rows };
        for (i, e) in cur.entries.iter().enumerate().filter(|(_, e)| e.id != 0) {
            c.row(&format!("{} {}", i + 1, BuyMenu::entry_name(e)), hud2, i as i32 + 1);
        }
        c.row("0 #Buy_GoBack", BACK, 10);
    }
    if matches!(b.page, page::SAVE | page::LOAD) {
        let save = b.page == page::SAVE;
        let mut c = Column { dc, x: 0x88, y: y0, page: b.page, dim: false, rows };
        c.row(if save { "#Buy_SaveEquipment" } else { "#Buy_LoadEquipment" }, RED, 0);
        for (i, s) in b.saved.iter().enumerate() {
            let key = if s.total < 1 { "#Buy_SlotEmpty" } else { "#Buy_SlotAvailable" };
            let text = fmt(&localise(key), &[I(i as i64 + 1)]);
            let red = !save && s.total > buy::CASH;
            c.row(&text, if red { RED } else { hud2 }, i as i32 + 1);
        }
        c.row("0 #Buy_GoBack", BACK, 10);
    }
    // Main page.
    let mut c = Column { dc, x: 8, y: y0, page: page::MAIN, dim: b.page != page::MAIN, rows };
    if cur.total < 1 {
        c.row("1 #Buy_EmptyConfig", RED, 1);
    } else {
        let t = if b.new_config { "1 #Buy_UseNewConfig" } else { "1 #Buy_UseLastConfig" };
        c.row(t, if buy::CASH - cur.total < 0 { RED } else { GREEN }, 1);
    }
    c.row("2 #Buy_Buy", hud2, 2);
    c.row("3 #Buy_RemoveItem", hud2, 3);
    c.row("4 #Buy_ResetConfig", hud2, 4);
    c.row("5 #Buy_SaveConfig", hud2, 5);
    c.row("6 #Buy_LoadConfig", hud2, 6);
    c.row("7 #Buy_Random", GREEN, 7);
    if b.page == page::MAIN {
        c.row("0 #Buy_Cancel", BACK, 10);
    }
}

/// `FUN_10038180`: sprite, price and slots of the highlighted weapon.
fn draw_weapon_box(dc: &Dc, id: i32) {
    let Some(w) = shared::weapons::tables().weapon(id as u8) else { return };
    let (x, y) = (0x108, dc.h() / 2 - 0x30);
    dc.fill(x, y, 0xff, 0x2f, dc.hud, 0x19);
    if let Some(&s) = dc.g.ts.weapons.get(id as usize) {
        dc.spr(s, x, y, dc.hud);
    }
    let f = hud::text::format;
    dc.string(x + 128, y, &f(&localise("#Buy_Credits"), &[hud::text::Arg::I(w.cost as i64)]), half(dc.hud));
    dc.string(x + 128, y + 20, &f(&localise("#Buy_Slots"), &[hud::text::Arg::I(w.slots as i64)]), half(dc.hud));
}

/// `FUN_10036af0`: totals bar and the loadout per bucket with sprites, ammo and attachments.
fn draw_summary(dc: &Dc, b: &BuyMenu, r: &Rules) {
    let hud2 = half(dc.hud);
    let cur = &b.current;
    use hud::text::Arg::I;
    dc.fill(8, 8, 0x280, 0x13, hud2, 0x32);
    let total = hud::text::format(&localise("#Buy_Total"), &[I(cur.total as i64)]);
    dc.string(8, 8, &total, if buy::CASH - cur.total <= 0 { RED } else { hud2 });
    let free = hud::text::format(&localise("#Buy_FreeSlots"), &[I((buy::SLOTS - cur.slots) as i64)]);
    dc.string(0x108, 8, &free, if buy::SLOTS - cur.slots <= 0 { RED } else { hud2 });
    let t = shared::weapons::tables();
    for bucket in 1..=5 {
        let x = bucket * 0x80 - 0x78;
        dc.fill(x, 0x1c, 0x80, 0x14, hud2, 0x32);
        dc.number(x, 0x1c, hud::draw::num::DIGITS_2, bucket, hud2);
        let mut y = 0x30;
        for e in cur.entries.iter().filter(|e| e.id != 0) {
            if e.is_item {
                if bucket != 1 {
                    continue;
                }
                let rgb = if buy::restricted(e.id, true, r) { RED } else { dc.hud };
                dc.string(x, y + 5, &BuyMenu::entry_name(e), rgb);
                y += 0x30;
                continue;
            }
            let mut ids = vec![e.id];
            if e.akimbo
                && let Some(a) = t.weapon(e.id as u8).map(|w| w.akimbo).filter(|&a| a >= 0)
            {
                ids = vec![a];
            }
            for id in ids {
                let Some(w) = t.weapon(id as u8) else { continue };
                if w.hud[0] as i32 != bucket {
                    continue;
                }
                let rgb = if buy::restricted(id, false, r) { RED } else { dc.hud };
                if let Some(&s) = dc.g.ts.weapons.get(id as usize) {
                    dc.spr(s, x, y, rgb);
                }
                if e.count != 0 {
                    dc.number(x, y + 5, hud::draw::num::DIGITS_2, e.count, dc.hud);
                } else if w.ammo != 0 {
                    dc.number(x, y + 5, hud::draw::num::DIGITS_3 | hud::draw::num::DRAW_ZERO, cur.ammo[w.ammo as usize & 15].0, dc.hud);
                }
                let s = buy::attachment_summary(e.att, &|k| localise(k));
                if !s.is_empty() {
                    dc.string(x, y + 0x14, &s, hud2);
                }
                y += 0x30;
            }
        }
    }
}

fn draw_team_menu(dc: &Dc, rows: &mut Vec<(i32, i32, i32, i32, i32)>) {
    let (names, brief) = hud::with(|h| (h.team_names.clone(), h.map_briefing.clone()));
    let names = if names.is_empty() { vec!["Specialists".to_owned(), "Mercenaries".to_owned()] } else { names };
    let (w, h) = (300, 40 + 24 * (names.len() as i32 + 2));
    let (x, mut y) = ((dc.w() - w) / 2, (dc.h() - h) / 2);
    dc.fill(x, y, w, h, [0; 3], 160);
    dc.string(x + 10, y + 8, "Select Team", dc.hud);
    if !brief.is_empty() {
        dc.string(x + 10, y + h + 4, &brief, half(dc.hud));
    }
    y += 36;
    let mut entries: Vec<(String, i32)> =
        names.iter().take(4).enumerate().map(|(i, n)| (format!("{} {}", i + 1, localise(n)), i as i32 + 1)).collect();
    entries.push(("5 Auto assign".into(), 5));
    entries.push(("0 Cancel".into(), 10));
    for (text, key) in entries {
        dc.fill(x + 10, y, w - 20, 20, half(dc.hud), 60);
        dc.string(x + 14, y + 2, &text, dc.hud);
        rows.push((x + 10, y, w - 20, 20, key));
        y += 24;
    }
}

/// MOTD window (`CMessageWindowPanel`): server name, the concatenated `MOTD` text, OK.
fn draw_motd(dc: &Dc) -> (i32, i32, i32, i32) {
    let e = engine();
    let (title, text) = hud::with(|h| (h.server_name.clone(), h.motd.clone()));
    let (w, h) = ((dc.w() * 3 / 4).min(640), (dc.h() * 3 / 4).min(480));
    let (x, y) = ((dc.w() - w) / 2, (dc.h() - h) / 2);
    dc.fill(x, y, w, h, [0; 3], 180);
    dc.fill(x, y, w, 22, half(dc.hud), 80);
    e.draw_console_string(x + 8, y + 4, [1.0, 1.0, 1.0], if title.is_empty() { "Message of the day" } else { &title });
    let lh = e.console_string_size("M").1.max(12);
    let mut yy = y + 30;
    'outer: for para in text.replace('\r', "").split('\n') {
        let mut line = String::new();
        for word in para.split(' ') {
            let cand = if line.is_empty() { word.to_owned() } else { format!("{line} {word}") };
            if e.console_string_size(&cand).0 > w - 16 && !line.is_empty() {
                e.draw_console_string(x + 8, yy, [0.9, 0.9, 0.9], &line);
                yy += lh;
                line = word.to_owned();
                if yy > y + h - 40 {
                    break 'outer;
                }
            } else {
                line = cand;
            }
        }
        e.draw_console_string(x + 8, yy, [0.9, 0.9, 0.9], &line);
        yy += lh;
        if yy > y + h - 40 {
            break;
        }
    }
    let ok = (x + w / 2 - 40, y + h - 30, 80, 22);
    dc.fill(ok.0, ok.1, ok.2, ok.3, half(dc.hud), 120);
    let tw = e.console_string_size("OK").0;
    e.draw_console_string(ok.0 + (80 - tw) / 2, ok.1 + 4, [1.0, 1.0, 1.0], "OK");
    ok
}

pub fn redraw(time: f32) {
    let _ = time;
    // CHudTSWeapons::Draw closes the buy menu once you're alive again (except the team page).
    if buy_open() && !hud::dead_or_observer() && with(|m| m.buy.page != page::TEAM) {
        with(|m| m.buy.close());
    }
    let r = rules();
    hud::with_dc(|dc| {
        let nohud = crate::input::cvars().nohud.value() != 0.0;
        with(|m| {
            m.rows.clear();
            m.team_rows.clear();
            if m.buy.active && !nohud {
                draw_buy(dc, &m.buy, &r, &mut m.rows);
            }
            if m.team_open {
                draw_team_menu(dc, &mut m.team_rows);
            }
            if m.motd_open {
                m.motd_ok = draw_motd(dc);
            }
        });
        if scores_visible() {
            let hover = if squelching() {
                let (cx, cy) = crate::input::cursor();
                with(|m| squelch_row(m, cx, cy)).map(|r| r.4)
            } else {
                None
            };
            let rows = scoreboard::draw(dc, hover);
            with(|m| m.score_rows = rows);
        } else {
            with(|m| m.score_rows.clear());
            // Squelch mode ends with the scoreboard (also when intermission hid it).
            crate::voice::stop_squelch_mode();
        }
        if wants_cursor() {
            let (cx, cy) = crate::input::cursor();
            if dc.g.ts.cursor != 0 {
                dc.spr(dc.g.ts.cursor, cx - 16, cy - 16, dc.hud);
            } else {
                dc.fill(cx - 1, cy - 1, 3, 3, [255; 3], 255);
            }
        }
    });
}
