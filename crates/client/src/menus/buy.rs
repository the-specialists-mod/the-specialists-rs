//! TS buy menu (`CTSBuyMenu`, embedded in `CHudTSWeapons` at 0x100d1460): an immediate-mode page
//! machine drawn with `FillRGBA`/`DrawHudString`, digit keys through `slotN`, mouse clicks through
//! `IN_MouseEvent`. Saved loadouts live in `<gamedir>/tsbuylist.tsa` (nine 0x408-byte configs).
//! Reference: architecture-hud.md §6.2; decomp `unnamed_100337f0.c` 10036200..100398e0.

use shared::weapons::tables;

pub const CONFIG_SIZE: usize = 0x408;
pub const SAVED_CONFIGS: usize = 9;
pub const ENTRIES: usize = 16;
/// The money a menu that isn't `CashBased()` spends (`FUN_10036640`).
pub const CASH: i32 = 30000;
/// `FUN_10036660`: inventory slots.
pub const SLOTS: i32 = 0x51;

pub const ITEM_KEVLAR: i32 = 0x10;
pub const ITEM_STEALTH: i32 = 0x20;

/// One loadout entry (0x38 bytes at config+0x80).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Entry {
    /// +0x00: item (kevlar/stealth shoes) rather than a weapon.
    pub is_item: bool,
    /// +0x04: weapon id, or item bits.
    pub id: i32,
    /// +0x08: attachment bits.
    pub att: u32,
    /// +0x0c: price of this entry.
    pub cost: i32,
    /// +0x30: bought as akimbo.
    pub akimbo: bool,
    /// +0x34: count for stackables (grenades, knives).
    pub count: i32,
}

/// One loadout (`0x408` bytes): ammo {rounds, buys} per type, 16 entries, total cost, slots used.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub ammo: [(i32, i32); 16],
    pub entries: [Entry; ENTRIES],
    pub total: i32,
    pub slots: i32,
}

impl Default for Config {
    fn default() -> Self {
        Config { ammo: [(0, 0); 16], entries: [Entry::default(); ENTRIES], total: 0, slots: 0 }
    }
}

/// A little-endian int at `o`, 0 past the end.
fn rd(b: &[u8], o: usize) -> i32 {
    b.get(o..o + 4).and_then(|s| s.try_into().ok()).map_or(0, i32::from_le_bytes)
}
fn wr(b: &mut [u8], o: usize, v: i32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}

impl Config {
    pub fn from_bytes(b: &[u8]) -> Self {
        let mut c = Config::default();
        if b.len() < CONFIG_SIZE {
            return c;
        }
        for (i, a) in c.ammo.iter_mut().enumerate() {
            *a = (rd(b, i * 8), rd(b, i * 8 + 4));
        }
        for (i, e) in c.entries.iter_mut().enumerate() {
            let o = 0x80 + i * 0x38;
            *e = Entry {
                is_item: b.get(o).is_some_and(|&v| v != 0),
                id: rd(b, o + 4),
                att: rd(b, o + 8) as u32,
                cost: rd(b, o + 0xc),
                akimbo: b.get(o + 0x30).is_some_and(|&v| v != 0),
                count: rd(b, o + 0x34),
            };
        }
        c.total = rd(b, 0x400);
        c.slots = rd(b, 0x404);
        c
    }

    pub fn to_bytes(&self) -> [u8; CONFIG_SIZE] {
        let mut b = [0u8; CONFIG_SIZE];
        for (i, a) in self.ammo.iter().enumerate() {
            wr(&mut b, i * 8, a.0);
            wr(&mut b, i * 8 + 4, a.1);
        }
        for (i, e) in self.entries.iter().enumerate() {
            let o = 0x80 + i * 0x38;
            b[o] = e.is_item as u8;
            wr(&mut b, o + 4, e.id);
            wr(&mut b, o + 8, e.att as i32);
            wr(&mut b, o + 0xc, e.cost);
            b[o + 0x30] = e.akimbo as u8;
            wr(&mut b, o + 0x34, e.count);
        }
        wr(&mut b, 0x400, self.total);
        wr(&mut b, 0x404, self.slots);
        b
    }
}

/// Parse `tsbuylist.tsa`: up to nine configs back to back.
pub fn parse_buylist(bytes: &[u8]) -> [Config; SAVED_CONFIGS] {
    std::array::from_fn(|i| bytes.get(i * CONFIG_SIZE..(i + 1) * CONFIG_SIZE).map(Config::from_bytes).unwrap_or_default())
}

pub fn write_buylist(saved: &[Config; SAVED_CONFIGS]) -> Vec<u8> {
    saved.iter().flat_map(|c| c.to_bytes()).collect()
}

/// What the gameplay ruleset decides about the menu
/// ([`ClientRules::buy_menu`](crate::gameplay::ClientRules::buy_menu)). The default is TS 3.0's.
#[derive(Debug, Clone, Copy)]
pub struct MenuRules {
    /// The handgun category (`Init` 10036200, column 0 of [`table`]).
    pub handguns: &'static [i32],
    /// `RandomConfig`'s pistols (`(id, may be akimbo)`), picked with `rand(from, from + len − 1)`.
    pub random_pistols: &'static [(i32, bool)],
    pub random_pistols_from: i32,
    /// Kevlar takes 10 slots, and items go through the slot check.
    pub items_use_slots: bool,
    /// The items page ("6 Player Accessory") only opens when `m_Teamplay > 1`.
    pub items_need_teamplay: bool,
    /// Stealth shoes are restricted when `m_Teamplay < 2`.
    pub stealth_needs_teamplay: bool,
    /// `cl_buykevlar` puts kevlar in a reset (and random) config.
    pub kevlar_cvar: bool,
    /// `add` buys a gun with no akimbo version (and not stackable) as a single one, whatever
    /// `full` says.
    pub akimbo_fixup: bool,
    /// The highest key `HandleKey` takes (11 = the `a` key).
    pub max_key: i32,
    /// Fixed menu text in place of `titles.txt`: each `#key` the menu draws or prints and its
    /// text ([`FixedTitles`]). `None` (TS 3.0) localises through the engine's titles.
    pub text: Option<&'static [(&'static str, &'static str)]>,
    /// The items page's rows are red by money only (TS 3.0 also tests the slots).
    pub items_red_by_cash_only: bool,
    /// The category column's mouse rows: `(m_Teamplay != 0) + 5` category rows before "back"
    /// (TS 3.0: always 6, the five categories and "Player Accessory").
    pub category_rows_by_teamplay: bool,
    /// Centre-printed when a click finds no row and leaves the page unset ([`Hit::Unset`]).
    /// TS 3.0 prints nothing.
    pub hole_message: Option<&'static str>,
    /// The summary writes an item's name (Kevlar, Stealth Shoes) in the halved HUD colour, never
    /// red. TS 3.0 (`0x10036af0`) uses the full HUD colour, or red when the item is restricted.
    pub summary_items_dim: bool,
    /// `Open` refuses during demo playback (TS 3.0 `0x10036670`). TS 2.1's (`0x1006b3f0`) doesn't
    /// check, and neither do its callers (`buy` `0x1008db00`, `Spectator` `0x100b28c0`), so a
    /// played-back demo opens it when the recorded player becomes an observer.
    pub refused_in_demo: bool,
}

impl Default for MenuRules {
    fn default() -> Self {
        MenuRules {
            handguns: &[1, 9, 12, 14, 2, 21, 22, 28, 31, 36],
            random_pistols: &[
                (36, false),
                (1, false),
                (9, true),
                (12, false),
                (14, true),
                (2, true),
                (21, false),
                (22, false),
                (28, false),
                (31, false),
            ],
            random_pistols_from: 0,
            items_use_slots: true,
            items_need_teamplay: false,
            stealth_needs_teamplay: true,
            kevlar_cvar: true,
            akimbo_fixup: true,
            max_key: 11,
            text: None,
            items_red_by_cash_only: false,
            category_rows_by_teamplay: false,
            hole_message: None,
            summary_items_dim: false,
            refused_in_demo: true,
        }
    }
}

/// Whether `Open` goes ahead: never during demo playback if the menu refuses it
/// ([`MenuRules::refused_in_demo`]).
pub fn opens(m: &MenuRules, demo_playing: bool) -> bool {
    !(demo_playing && m.refused_in_demo)
}

/// The colour of an item's name in the summary (`FUN_10036af0`'s item branch, TS 2.1
/// `0x1006b9a0`): see [`MenuRules::summary_items_dim`].
pub fn summary_item_rgb(id: i32, r: &Rules, hud: [i32; 3]) -> [i32; 3] {
    if r.menu.summary_items_dim {
        crate::hud::draw::half(hud)
    } else if restricted(id, true, r) {
        RED
    } else {
        hud
    }
}

/// `FUN_10036640`: the money the menu spends, `TSCash` when `CashBased()`
/// ([`ClientRules::buy_cash_based`](crate::gameplay::ClientRules::buy_cash_based), never in TS
/// 3.0), else [`CASH`].
pub fn cash(cash_based: bool, tscash: i32) -> i32 {
    if cash_based { tscash } else { CASH }
}

/// A ruleset's fixed menu text ([`MenuRules::text`]) as titles: a `#key` with no entry is left
/// as it is.
pub struct FixedTitles(pub &'static [(&'static str, &'static str)]);

impl crate::hud::text::Titles for FixedTitles {
    fn get(&self, key: &str) -> Option<String> {
        self.0.iter().find(|(k, _)| *k == key).map(|(_, t)| (*t).to_owned())
    }
}

/// The menu's text for `msg`, its `#key`s replaced: from the ruleset's fixed text, else from
/// `titles` (the engine's).
pub fn text(m: &MenuRules, titles: &dyn crate::hud::text::Titles, msg: &str) -> String {
    match m.text {
        Some(t) => crate::hud::text::localise_with(&FixedTitles(t), msg),
        None => crate::hud::text::localise_with(titles, msg),
    }
}

/// Server settings the menu depends on.
#[derive(Debug, Clone, Copy)]
pub struct Rules {
    /// `m_Teamplay` (GameMode): grenades and stealth shoes need > 1.
    pub gamemode: i32,
    /// `SrvSett` 3 weapon restriction mask.
    pub restriction: i32,
    /// `SrvSett` 2 `ammocount` column.
    pub ammocount: i32,
    /// `cl_buykevlar`: a reset config starts with kevlar.
    pub buy_kevlar: bool,
    /// The money the menu spends ([`cash`]).
    pub cash: i32,
    /// `CashBased()`: the summary shows the money left ([`cash`]).
    pub cash_based: bool,
    pub menu: MenuRules,
}

impl Default for Rules {
    fn default() -> Self {
        // The mask (`0x1018d174`) is zero until `SrvSett 3` arrives, so every weapon starts
        // restricted, as in TS.
        Rules { gamemode: 0, restriction: 0, ammocount: 0, buy_kevlar: true, cash: CASH, cash_based: false, menu: MenuRules::default() }
    }
}

/// `FUN_1000b6d0`: is the weapon blocked by the restriction mask? Ids 1..37 only; mask 0 blocks
/// all of them. The weapon type is `FUN_1000b670`'s: katana → knife, sawed-off → shotgun, akimbo
/// Uzi/Skorpion → SMG, the other akimbos and the Golden Colts → pistol, else the weapon table's
/// kind, which is the server's `GetWeaponType`/`IsRestrictedWeapon` too.
pub fn weapon_restricted(id: i32, mask: i32) -> bool {
    (1..38).contains(&id) && shared::weapons::is_restricted(id as u8, mask as u32)
}

/// `FUN_100365e0`: restricted for buying.
pub fn restricted(id: i32, item: bool, r: &Rules) -> bool {
    if !item {
        weapon_restricted(id, r.restriction) || (r.gamemode < 2 && id == 24)
    } else {
        r.menu.stealth_needs_teamplay && r.gamemode < 2 && id & ITEM_STEALTH != 0
    }
}

fn att_price(att: u32) -> i32 {
    let p = &tables().combat.attachment_price;
    let mut c = 0;
    if att & 1 != 0 {
        c += p.silencer;
    }
    if att & 2 != 0 {
        c += p.laser;
    }
    if att & 4 != 0 {
        c += p.flashlight;
    }
    if att & 8 != 0 {
        c += p.scope;
    }
    c
}

/// `FUN_10012ad0`: price of an entry.
pub fn cost(id: i32, att: u32, full: bool, item: bool) -> i32 {
    if item {
        return if id & ITEM_KEVLAR != 0 {
            1900
        } else if id & ITEM_STEALTH != 0 {
            700
        } else {
            full as i32
        };
    }
    let Some(w) = tables().weapon(id as u8) else { return 0 };
    if w.stack {
        return w.cost * if full { w.pellets } else { 1 };
    }
    w.cost * (full as i32 + 1) + att_price(att)
}

/// `FUN_10038dc0`: inventory slots an entry takes.
pub fn slots_for(id: i32, full: bool, item: bool, r: &Rules) -> i32 {
    if item {
        return if id & ITEM_KEVLAR != 0 && r.menu.items_use_slots { 10 } else { 0 };
    }
    let Some(w) = tables().weapon(id as u8) else { return 0 };
    if !full {
        w.slots
    } else if !w.stack {
        w.slots * 2
    } else {
        w.pellets * w.slots
    }
}

/// Ammo type limits under the current `ammocount`: `FUN_100072c0` switches to column 1, 2 or 3
/// for those values and fills column 0's for any other, 4 and above or negative included
/// (issue #108).
fn ammo_max(t: usize, r: &Rules) -> i32 {
    let column = if (1..=3).contains(&r.ammocount) { r.ammocount as usize } else { 0 };
    tables().ammo(t as u8).map_or(0, |a| a.max_carry[column])
}

/// Things the menu asks the HUD to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    ServerCmd(String),
    /// `AddTSMessage(text, rgb, 2 s, flags 0)`; rgb (0,0,0) = HUD colour.
    Message(String, [i32; 3]),
    /// Like `Message`, with the titles.txt format string's `%s`/`%i` argument.
    MessageArg(String, String, [i32; 3]),
    Close,
    SaveFile,
}

const RED: [i32; 3] = [195, 45, 45];

/// What [`BuyMenu::hit_test`] found under the mouse.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Hit {
    /// A row: `HandleKey(page, key)`.
    Row(i32, i32),
    /// No row (`-1`): TS's left click then acts as key 1 on the current page.
    Miss,
    /// A path where TS returns without writing the page, leaving an uninitialised stack value
    /// (a full sixteen-entry remove list, or a weapon column with no end marker). The port does
    /// nothing.
    Unset,
}

/// Pages (`+0x04`).
pub mod page {
    pub const MAIN: i32 = 0;
    pub const CATEGORY: i32 = 1;
    pub const WEAPONS: i32 = 2;
    pub const OPTIONS: i32 = 3;
    pub const ITEMS: i32 = 4;
    pub const REMOVE: i32 = 5;
    pub const SAVE: i32 = 6;
    pub const LOAD: i32 = 7;
    pub const TEAM: i32 = 8;
}

/// Category table (`+0x2d6c`, 12 rows × 5 categories, set in `Init` 10036200). The handguns
/// are the ruleset's ([`MenuRules::handguns`]).
pub fn table(m: &MenuRules) -> [[i32; 5]; 12] {
    let mut t = [[-1; 5]; 12];
    let cols: [&[i32]; 5] = [m.handguns, &[3, 6, 7, 17, 19, 23], &[5, 13, 15, 27, 18], &[4, 11, 20, 26, 33], &[24, 25, 32, 34, 35]];
    for (c, col) in cols.iter().enumerate() {
        for (row, &id) in col.iter().take(12).enumerate() {
            t[row][c] = id;
        }
    }
    t
}

/// Category names (`Init` 10036200; `#Buy_SMGs` is at `0x10094dd0`).
pub const CATEGORIES: [&str; 5] = ["#Buy_Handguns", "#Buy_SMGs", "#Buy_Rifles", "#Buy_Shotguns", "#Buy_SpecialPurpose"];

pub struct BuyMenu {
    pub active: bool,
    pub page: i32,
    pub category: i32,
    pub pending_att: u32,
    pub pending_weapon: i32,
    pub current: Config,
    pub last: Config,
    pub saved: [Config; SAVED_CONFIGS],
    /// `+0x2f08`: the config changed since the last submit ("Use New Config").
    pub new_config: bool,
    /// `+0x2f0c`: the money the ammo fill spent ([`BuyMenu::recompute_ammo`]), which the summary
    /// adds to the total.
    pub ammo_spend: i32,
    /// `+0x2f10`: random loadout on the first open.
    pub random_first: bool,
}

impl Default for BuyMenu {
    fn default() -> Self {
        BuyMenu {
            active: false,
            page: 0,
            category: 0,
            pending_att: 0,
            pending_weapon: -1,
            current: Config::default(),
            last: Config::default(),
            saved: Default::default(),
            new_config: true,
            ammo_spend: 0,
            random_first: true,
        }
    }
}

/// Random source: `rand(lo, hi)` inclusive, and a float in [lo, hi).
pub trait Rng {
    fn long(&mut self, lo: i32, hi: i32) -> i32;
    fn float(&mut self, lo: f32, hi: f32) -> f32;
}

impl BuyMenu {
    /// `Init` (`0x10036200`), run by every `CHudTSWeapons::VidInit` (`0x1004cb50`, its only
    /// caller), given the configs `FUN_10036480` read from `tsbuylist.tsa`: closed (`+0`), page,
    /// category and pending attachments 0 (`+4/+8/+0xc`), no pending weapon (`+0x10` = −1), the
    /// current, last and saved configs zeroed (`+0x14`, `+0x41c`, `+0x824`) before the file is
    /// read, "new config" set (`+0x2f08`) and the random first loadout armed again (`+0x2f10`).
    /// So each connect, map change or video-mode change forgets the last config, and the next
    /// open picks a random one. The category table (`+0x2d6c`) and names (`+0x2e5c`) it rebuilds
    /// are [`table`] and [`MenuRules`]; `+0x2f14` is never read and `+0x2f1c` (the mouse's
    /// pressed bits) is rewritten by every mouse event. `+0x2f0c` (the ammo spend) is left as it
    /// is in TS; here it is 0, which nothing sees: only the open menu shows it, and `Open` zeroes it.
    pub fn new(saved: [Config; SAVED_CONFIGS]) -> BuyMenu {
        BuyMenu { saved, ..BuyMenu::default() }
    }

    fn back_to_main(&mut self) {
        self.page = page::MAIN;
        self.category = 0;
        self.pending_weapon = -1;
        self.pending_att = 0;
    }

    /// `Close` (10036720).
    pub fn close(&mut self) {
        self.pending_weapon = -1;
        self.active = false;
        self.page = 0;
        self.category = 0;
        self.pending_att = 0;
        self.ammo_spend = 0;
    }

    /// `Open` (10036670), minus the demo check and cursor centring (caller).
    pub fn open(&mut self, r: &Rules, rng: &mut dyn Rng) {
        if self.active {
            return;
        }
        self.current = self.last.clone();
        self.ammo_spend = 0;
        self.active = true;
        self.page = 0;
        self.new_config = false;
        if self.random_first {
            self.random_first = false;
            let p = rng.float(0.5, 1.0);
            self.random_config(p, r, rng);
        }
        self.recompute_ammo(r);
    }

    /// `FUN_10038800`: empty config (plus kevlar with `cl_buykevlar`).
    pub fn reset_config(&mut self, r: &Rules) {
        self.current = Config::default();
        self.new_config = true;
        if r.buy_kevlar && r.menu.kevlar_cvar {
            self.add(ITEM_KEVLAR, 0, false, true, r);
        }
    }

    /// `FUN_10038d70`: can the pending weapon take this attachment?
    pub fn attachment_allowed(&self, bit: u32) -> bool {
        if self.pending_att & bit != 0 {
            return false;
        }
        let Some(w) = tables().weapon(self.pending_weapon.max(0) as u8) else { return false };
        bit & w.att_default as u32 == 0 && bit & w.att_allowed as u32 != 0
    }

    /// `FUN_10039380`: one more ammo purchase of type `t`.
    fn add_ammo_buy(&mut self, t: usize, r: &Rules) -> bool {
        if t >= 16 {
            return false;
        }
        let max = ammo_max(t, r);
        let a = &mut self.current.ammo[t];
        if a.0 >= max {
            return false;
        }
        let per = tables().ammo(t as u8).map_or(0, |x| x.per_buy);
        a.0 = (a.0 + per).min(max);
        a.1 += 1;
        true
    }

    /// `FUN_100392b0`: fill ammo round-robin over the guns with the money left. With money to
    /// spend it records what the ammo cost, `(initial - remaining) & ((initial - remaining) < 1) - 1`
    /// (`0x10039354..0x10039365`), so at least 0; otherwise the spend keeps its value (issue #108).
    pub fn recompute_ammo(&mut self, r: &Rules) {
        let initial = r.cash.wrapping_sub(self.current.total);
        let mut budget = initial;
        self.current.ammo = [(0, 0); 16];
        if budget > 0 {
            loop {
                let mut changed = false;
                for i in 0..ENTRIES {
                    let e = self.current.entries[i];
                    if e.id == 0 || e.is_item {
                        continue;
                    }
                    let t = tables().weapon(e.id as u8).map_or(0, |w| w.ammo as usize);
                    let price = tables().ammo(t as u8).map_or(0, |a| a.price);
                    if budget - price >= 0 && self.add_ammo_buy(t, r) {
                        budget -= price;
                        changed = true;
                    }
                }
                if !changed {
                    break;
                }
            }
            self.ammo_spend = initial.wrapping_sub(budget).max(0);
        }
    }

    /// `FUN_10038e40`: remove entry `i` and compact the list.
    pub fn remove(&mut self, i: usize, r: &Rules) {
        let Some(&e) = self.current.entries.get(i) else { return };
        // A loaded `tsbuylist.tsa` can hold any numbers: C int arithmetic, which wraps.
        self.current.total = self.current.total.wrapping_sub(e.cost);
        if !e.is_item {
            let w = tables().weapon(e.id as u8).map_or(0, |w| w.slots);
            let n = if e.count == 0 { w.wrapping_mul(e.akimbo as i32 + 1) } else { w.wrapping_mul(e.count) };
            self.current.slots = self.current.slots.wrapping_sub(n);
        } else if e.id & ITEM_KEVLAR != 0 && r.menu.items_use_slots {
            self.current.slots = self.current.slots.wrapping_sub(10);
        }
        self.current.entries[i] = Entry::default();
        // Bubble empty entries to the end.
        let mut moved = true;
        while moved {
            moved = false;
            for j in 0..ENTRIES - 1 {
                if self.current.entries[j].id < 1 && self.current.entries[j + 1].id > 0 {
                    self.current.entries.swap(j, j + 1);
                    moved = true;
                }
            }
        }
        self.new_config = true;
    }

    /// `FUN_10038f60(id, attachments, full, is_item)`: add to the config. `full` = akimbo for
    /// guns with an akimbo version, full load for stackables.
    pub fn add(&mut self, id: i32, att: u32, full: bool, item: bool, r: &Rules) -> bool {
        if restricted(id, item, r) {
            return false;
        }
        let info = if item { None } else { tables().weapon(id as u8) };
        if !item && info.is_none() {
            return false;
        }
        let (mut att, mut full) = (att, full);
        if r.menu.akimbo_fixup
            && let Some(w) = info
            && w.akimbo == -1
            && !w.stack
        {
            full = false;
        }
        // `wrapping_add`: a loaded config's totals can be anything (C int arithmetic).
        if cost(id, att, full, item).wrapping_add(self.current.total) > r.cash {
            return false;
        }
        if (!item || r.menu.items_use_slots) && slots_for(id, full, item, r).wrapping_add(self.current.slots) > SLOTS {
            return false;
        }
        for i in 0..ENTRIES {
            let e = self.current.entries[i];
            if e.id != id || e.id == 0 {
                continue;
            }
            if item {
                if e.is_item {
                    return false;
                }
                continue;
            }
            let Some(w) = info else { continue };
            if w.stack {
                if e.count >= w.pellets {
                    return false;
                }
                let n = if full { w.pellets.wrapping_sub(e.count) } else { 1 };
                let en = &mut self.current.entries[i];
                en.count = en.count.wrapping_add(n);
                en.cost = en.cost.wrapping_add(w.cost.wrapping_mul(n));
                self.current.total = self.current.total.wrapping_add(w.cost.wrapping_mul(n));
                self.current.slots = self.current.slots.wrapping_add(w.slots.wrapping_mul(n));
                self.recompute_ammo(r);
                self.new_config = true;
                return true;
            }
            // Buying the same gun again: merge attachments and make it akimbo.
            if w.akimbo != -1 && !e.akimbo {
                att |= e.att;
                full = true;
            }
            self.remove(i, r);
            break;
        }
        let Some(j) = self.current.entries.iter().position(|e| e.id == 0) else { return false };
        let stack = info.is_some_and(|w| w.stack);
        let mut e = Entry { is_item: item, id, att: if item { 0 } else { att }, ..Default::default() };
        if !stack {
            e.akimbo = full;
        }
        e.cost = cost(id, att, full, item);
        if !item && stack {
            e.count = if full { info.map_or(1, |w| w.pellets) } else { 1 };
        }
        self.current.entries[j] = e;
        self.current.total = self.current.total.wrapping_add(e.cost);
        self.current.slots = self.current.slots.wrapping_add(slots_for(id, full, item, r));
        self.recompute_ammo(r);
        self.new_config = true;
        true
    }

    /// `FUN_10039810`: random attachments (50% silencer, 50% laser, 25% scope; allowed and not
    /// built in).
    fn random_attachments(id: i32, rng: &mut dyn Rng) -> u32 {
        let Some(w) = tables().weapon(id as u8) else { return 0 };
        let (allowed, builtin) = (w.att_allowed as u32, w.att_default as u32);
        let mut a = 0;
        for (bit, threshold) in [(1u32, 50), (2, 50), (8, 75)] {
            if rng.long(0, 100) > threshold && allowed & bit != 0 && builtin & bit == 0 {
                a |= bit;
            }
        }
        a
    }

    /// `RandomConfig` (100398e0): a melee weapon, then by `p`: a pistol (0.1 < p < 0.9), an SMG
    /// (0.3 < p < 0.7) or a rifle/shotgun (p ≥ 0.7). `p <= 0` picks `p` at random. The result
    /// also becomes the last config (`+0x14` copied to `+0x41c` at the end), so the next open
    /// brings it back even if it was never bought.
    pub fn random_config(&mut self, p: f32, r: &Rules, rng: &mut dyn Rng) {
        self.random_config_entries(p, r, rng);
        self.last = self.current.clone();
    }

    fn random_config_entries(&mut self, p: f32, r: &Rules, rng: &mut dyn Rng) {
        self.reset_config(r);
        let (melee, full) = match rng.long(0, 5) {
            2 => (35, rng.long(0, 2) == 0),
            3 => (34, false),
            _ => (25, rng.long(0, 2) == 0),
        };
        self.add(melee, 0, full, false, r);
        let p = if p <= 0.0 { rng.float(0.0, 1.0) } else { p };
        if 0.1 < p && p < 0.9 {
            let (pistols, from) = (r.menu.random_pistols, r.menu.random_pistols_from);
            let last = pistols.len() as i32 - 1;
            // Guarded: the RNG stays in the list.
            let k = rng.long(from, from + last).wrapping_sub(from).clamp(0, last);
            let (id, can_akimbo) = pistols[k as usize];
            let ak = can_akimbo && rng.long(0, 1) != 0;
            let att = Self::random_attachments(id, rng);
            self.add(id, att, ak, false, r);
        }
        if p > 0.3 && p < 0.7 {
            const SMGS: [i32; 6] = [3, 6, 7, 17, 19, 23];
            let k = rng.long(1, 6).clamp(1, 6) as usize - 1;
            let id = SMGS[k];
            let ak = k == 0 && rng.long(0, 1) != 0;
            let att = Self::random_attachments(id, rng);
            self.add(id, att, ak, false, r);
        } else if p >= 0.7 {
            const RIFLES: [i32; 11] = [4, 11, 20, 13, 5, 15, 26, 27, 33, 18, 32];
            let k = rng.long(1, 12);
            if (1..=11).contains(&k) {
                let id = RIFLES[k as usize - 1];
                let att = Self::random_attachments(id, rng);
                self.add(id, att, false, false, r);
            }
        }
    }

    /// `SubmitConfig` (10039400): `rst_cfg`, then the `tkw`, `tki` and `tka` lists.
    pub fn submit(&mut self, actions: &mut Vec<Action>, r: &Rules) -> bool {
        if self.current.total < 1 {
            actions.push(Action::Message("#Buy_ConfigEmpty".into(), RED));
        }
        if r.cash < self.current.total {
            actions.push(Action::Message("#Buy_NoCash".into(), RED));
            return false;
        }
        if SLOTS < self.current.slots {
            return false;
        }
        // Sort (bubble, pairs 0..15): ascending price, melee/grenades (no ammo type) first.
        let ammo_of = |id: i32| tables().weapon(id as u8).map_or(0, |w| w.ammo);
        // TS loops until a pass swaps nothing. Its order isn't transitive once items mix with
        // ammo-less guns, so the passes are capped (a consistent order needs fewer than
        // ENTRIES) to never hang on a loaded config.
        for _ in 0..ENTRIES * ENTRIES {
            let mut swapped = false;
            for i in 0..ENTRIES - 1 {
                let (a, b) = (self.current.entries[i], self.current.entries[i + 1]);
                if a.id <= 0 || b.id <= 0 {
                    continue;
                }
                let swap = if !a.is_item && !b.is_item {
                    let (aa, ab) = (ammo_of(a.id), ammo_of(b.id));
                    if aa == 0 || ab == 0 { if aa == 0 && ab == 0 { b.cost < a.cost } else { aa != 0 } } else { b.cost < a.cost }
                } else {
                    b.cost < a.cost
                };
                if swap {
                    self.current.entries.swap(i, i + 1);
                    swapped = true;
                }
            }
            if !swapped {
                break;
            }
        }
        let (mut tkw, mut tki, mut tka) = ("tkw ".to_owned(), "tki ".to_owned(), "tka ".to_owned());
        for e in self.current.entries.iter().filter(|e| e.id != 0) {
            if e.is_item {
                tki.push_str(&format!("{} ", e.id));
                continue;
            }
            let stack = tables().weapon(e.id as u8).is_some_and(|w| w.stack);
            if stack {
                tkw.push_str(&format!("{}_{} ", e.id, e.count));
            } else if e.akimbo {
                // Akimbo: the gun twice, the second copy without attachments.
                tkw.push_str(&format!("{}_{} {}_{} ", e.id, e.att, e.id, 0));
            } else {
                tkw.push_str(&format!("{}_{} ", e.id, e.att));
            }
        }
        for (t, a) in self.current.ammo.iter().enumerate() {
            if a.0 != 0 {
                tka.push_str(&format!("{}_{} ", t, a.1));
            }
        }
        actions.push(Action::ServerCmd("rst_cfg".into()));
        actions.push(Action::ServerCmd(tkw));
        actions.push(Action::ServerCmd(tki));
        actions.push(Action::ServerCmd(tka));
        let msg = if self.new_config { "#Buy_UsingNewConfig" } else { "#Buy_UsingLastConfig" };
        actions.push(Action::Message(msg.into(), [0; 3]));
        self.new_config = false;
        self.last = self.current.clone();
        true
    }

    /// Display name of an entry (remove page, messages).
    pub fn entry_name(e: &Entry) -> String {
        if e.is_item {
            if e.id & ITEM_KEVLAR != 0 {
                "Kevlar".into()
            } else if e.id & ITEM_STEALTH != 0 {
                "Stealth Shoes".into()
            } else {
                String::new()
            }
        } else {
            tables().weapon(e.id as u8).map_or_else(String::new, |w| w.name.clone())
        }
    }

    /// `FUN_10038410`: the menu row under the mouse. Columns are 128 px wide from x 8, rows 20 px
    /// high from mid-screen; each column answers for its page whatever page is showing.
    pub fn hit_test(&self, x: i32, y: i32, screen_h: i32, r: &Rules) -> Hit {
        if x < 8 || y < screen_h / 2 {
            return Hit::Miss;
        }
        let row = (y - screen_h / 2) / 20;
        let cur = self.page;
        match (x - 8) / 128 {
            0 => {
                if row > 7 || (cur != page::MAIN && row == 7) {
                    return Hit::Miss;
                }
                Hit::Row(page::MAIN, if row == 7 { 10 } else { row + 1 })
            }
            1 => match cur {
                page::CATEGORY..=page::ITEMS => {
                    // The rows before "back": TS 3.0 always 6; with `category_rows_by_teamplay`
                    // `(m_Teamplay != 0) + 5`, so in mode 1 the drawn "back" row (the sixth, with
                    // no "Player Accessory" above it) is key 6.
                    let n = if r.menu.category_rows_by_teamplay { (r.gamemode != 0) as i32 + 5 } else { 6 };
                    if row > n || (cur != page::CATEGORY && row == n) {
                        return Hit::Miss;
                    }
                    Hit::Row(page::CATEGORY, if row == n { 10 } else { row + 1 })
                }
                page::REMOVE => {
                    let mut i = 0;
                    while self.current.entries[i].id != 0 {
                        if row == i as i32 {
                            return Hit::Row(page::REMOVE, row + 1);
                        }
                        i += 1;
                        if i > 15 {
                            return Hit::Unset;
                        }
                    }
                    if row == i as i32 { Hit::Row(page::REMOVE, 10) } else { Hit::Miss }
                }
                page::SAVE | page::LOAD if row < 11 => Hit::Row(cur, row),
                _ => Hit::Miss,
            },
            2 => match cur {
                page::WEAPONS | page::OPTIONS => {
                    let (mut i, mut skipped) = (0i32, 0i32);
                    loop {
                        let Some(&id) = table(&r.menu).get(i as usize).map(|t| &t[self.category.clamp(0, 4) as usize]) else {
                            return Hit::Unset;
                        };
                        if id == -1 {
                            break;
                        }
                        if !restricted(id, false, r) {
                            if row == i - skipped {
                                let key = skipped + 1 + row;
                                return Hit::Row(page::WEAPONS, if key < 10 { key } else { key + 1 });
                            }
                        } else {
                            skipped += 1;
                        }
                        i += 1;
                    }
                    if row == i - skipped { Hit::Row(page::WEAPONS, 10) } else { Hit::Miss }
                }
                page::ITEMS if row < 3 => {
                    let item = match row {
                        0 => ITEM_KEVLAR,
                        1 => ITEM_STEALTH,
                        _ => 0,
                    };
                    let row = if restricted(item, true, r) { row + 1 } else { row };
                    Hit::Row(page::ITEMS, if row == 2 { 10 } else { row + 1 })
                }
                _ => Hit::Miss,
            },
            3 if cur == page::OPTIONS => {
                if row == 0 {
                    return Hit::Row(page::OPTIONS, 1);
                }
                let mut skipped = 0;
                for i in 0..4 {
                    if self.attachment_allowed(1 << i) {
                        if row == i - skipped + 1 {
                            return Hit::Row(page::OPTIONS, skipped + 1 + row);
                        }
                    } else {
                        skipped += 1;
                    }
                }
                // The akimbo / full-load row (key 6) exists for guns with an akimbo version and
                // for stackables; "back" follows the last row.
                let w = tables().weapon(self.pending_weapon.max(0) as u8);
                let full_row = w.is_some_and(|w| w.akimbo != -1 || w.stack);
                let mut n = 4;
                if full_row {
                    n = 5;
                    if row == 4 - skipped + 1 {
                        return Hit::Row(page::OPTIONS, 6);
                    }
                }
                if row == n - skipped + 1 { Hit::Row(page::OPTIONS, 10) } else { Hit::Miss }
            }
            _ => Hit::Miss,
        }
    }

    /// `HandleKey(page, key)` (10038840). Keys 1..11 (10 = "0"/back, 11 = the `a` key).
    pub fn handle_key(&mut self, pg: i32, key: i32, r: &Rules, rng: &mut dyn Rng) -> Vec<Action> {
        let mut out = Vec::new();
        if !(1..=r.menu.max_key).contains(&key) {
            return out;
        }
        match pg {
            page::MAIN => match key {
                1 => {
                    if self.submit(&mut out, r) {
                        self.close();
                        out.push(Action::Close);
                    }
                }
                2 => self.page = page::CATEGORY,
                3 => self.page = page::REMOVE,
                4 => {
                    self.reset_config(r);
                    out.push(Action::Message("#Buy_ConfigReset".into(), [0; 3]));
                    self.ammo_spend = 0;
                }
                5 => self.page = page::SAVE,
                6 => self.page = page::LOAD,
                7 => self.random_config(0.0, r, rng),
                10 => {
                    self.close();
                    out.push(Action::Close);
                }
                _ => {}
            },
            page::CATEGORY => match key {
                6 if !r.menu.items_need_teamplay || r.gamemode > 1 => self.page = page::ITEMS,
                1..=5 if table(&r.menu)[0][key as usize - 1] != -1 => {
                    self.page = page::WEAPONS;
                    self.category = key - 1;
                }
                10 => self.back_to_main(),
                _ => {}
            },
            page::WEAPONS => {
                if key == 10 {
                    self.page = page::CATEGORY;
                    self.pending_weapon = -1;
                    self.pending_att = 0;
                    return out;
                }
                let row = if key > 9 { key - 2 } else { key - 1 } as usize;
                let Some(&id) = table(&r.menu).get(row).and_then(|t| t.get(self.category as usize)) else { return out };
                if id == -1 || restricted(id, false, r) {
                    return out;
                }
                self.page = page::OPTIONS;
                self.pending_weapon = id;
            }
            page::OPTIONS => match key {
                1 | 6 => {
                    let (id, att) = (self.pending_weapon, self.pending_att);
                    self.add(id, att, key == 6, false, r);
                    self.page = page::CATEGORY;
                    self.pending_weapon = -1;
                    self.pending_att = 0;
                }
                2..=5 => {
                    let bit = 1u32 << (key - 2);
                    if self.attachment_allowed(bit) {
                        self.pending_att |= bit;
                    }
                }
                10 => {
                    self.page = page::WEAPONS;
                    self.pending_weapon = -1;
                    self.pending_att = 0;
                }
                _ => {}
            },
            page::ITEMS => match key {
                1 | 2 => {
                    let id = if key == 1 { ITEM_KEVLAR } else { ITEM_STEALTH };
                    if restricted(id, true, r) {
                        return out;
                    }
                    self.add(id, 0, false, true, r);
                    self.page = page::CATEGORY;
                    self.pending_weapon = -1;
                    self.pending_att = 0;
                }
                10 => {
                    self.page = page::CATEGORY;
                    self.pending_weapon = -1;
                    self.pending_att = 0;
                }
                _ => {}
            },
            page::REMOVE => {
                if key == 10 {
                    self.back_to_main();
                    return out;
                }
                let i = key as usize - 1;
                if i < ENTRIES && self.current.entries[i].id != 0 {
                    let name = Self::entry_name(&self.current.entries[i]);
                    out.push(Action::MessageArg("#Buy_Removed".into(), name, [0; 3]));
                    self.remove(i, r);
                    self.recompute_ammo(r);
                }
            }
            page::SAVE => {
                if key == 10 {
                    self.back_to_main();
                    return out;
                }
                if (1..=SAVED_CONFIGS as i32).contains(&key) {
                    self.saved[key as usize - 1] = self.current.clone();
                    out.push(Action::SaveFile);
                    self.back_to_main();
                    out.push(Action::MessageArg("#Buy_SavedConfig".into(), key.to_string(), [0; 3]));
                }
            }
            page::LOAD => {
                if key == 10 {
                    self.back_to_main();
                    return out;
                }
                if (1..=SAVED_CONFIGS as i32).contains(&key) && self.saved[key as usize - 1].total > 0 {
                    self.current = self.saved[key as usize - 1].clone();
                    self.recompute_ammo(r);
                    out.push(Action::MessageArg("#Buy_ConfigLoaded".into(), key.to_string(), [0; 3]));
                    self.back_to_main();
                    self.new_config = true;
                }
            }
            page::TEAM => {
                if key < 3 {
                    // `0x10038cf9`: `chgteam key - 1`.
                    out.push(Action::ServerCmd(format!("chgteam {}", key - 1)));
                    self.close();
                    out.push(Action::Close);
                } else if key == 10 {
                    self.close();
                    out.push(Action::Close);
                }
            }
            _ => {}
        }
        out
    }
}

/// `FUN_10036af0`'s header (`0x10036b64..0x10036bc1`): the total includes the ammo
/// ([`BuyMenu::ammo_spend`]), and a `CashBased()` menu also shows the money left after it,
/// `(cash - total - spend, total + spend)` (issue #108). The title key and its arguments.
pub fn summary_header(b: &BuyMenu, r: &Rules) -> (&'static str, Vec<i32>) {
    let with_ammo = b.current.total.wrapping_add(b.ammo_spend);
    if r.cash_based {
        ("#Buy_Total_CashBased", vec![r.cash.wrapping_sub(b.current.total).wrapping_sub(b.ammo_spend), with_ammo])
    } else {
        ("#Buy_Total", vec![with_ammo])
    }
}

/// `FUN_100368f0`: `(Sil,Las,Fla,Sco)` attachment summary.
pub fn attachment_summary(att: u32, loc: &dyn Fn(&str) -> String) -> String {
    if att == 0 {
        return String::new();
    }
    let parts: Vec<String> = [(1, "#Buy_Sil"), (2, "#Buy_Las"), (4, "#Buy_Fla"), (8, "#Buy_Sco")]
        .iter()
        .filter(|(b, _)| att & b != 0)
        .map(|(_, k)| loc(k))
        .collect();
    format!("({})", parts.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixedRng(Vec<i32>, usize);
    impl Rng for FixedRng {
        fn long(&mut self, lo: i32, hi: i32) -> i32 {
            let v = self.0.get(self.1).copied().unwrap_or(lo);
            self.1 += 1;
            v.clamp(lo, hi)
        }
        fn float(&mut self, lo: f32, _hi: f32) -> f32 {
            lo
        }
    }

    fn rules() -> Rules {
        Rules { restriction: 0x1f, ..Rules::default() }
    }

    #[test]
    fn parses_shipped_buylist() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../../ts/tsbuylist.tsa");
        let Ok(bytes) = std::fs::read(path) else { return }; // content not present in CI
        assert_eq!(bytes.len(), SAVED_CONFIGS * CONFIG_SIZE);
        let saved = parse_buylist(&bytes);
        let c = &saved[0];
        assert_eq!(c.total, 8300);
        assert_eq!(c.slots, 70);
        assert_eq!(c.entries[0], Entry { is_item: true, id: 0x10, att: 0, cost: 1900, akimbo: false, count: 0 });
        assert_eq!((c.entries[1].id, c.entries[1].count, c.entries[1].cost), (25, 5, 500));
        assert_eq!((c.entries[3].id, c.entries[3].att, c.entries[3].cost), (4, 2, 3800));
        assert_eq!(c.ammo[3], (60, 8));
        assert_eq!(write_buylist(&saved), bytes, "round trip is byte-exact");
    }

    /// Issue #189: `Init` (`0x10036200`), from every `VidInit`, forgets the current and last
    /// configs and arms the random first loadout again; the saved ones come from the file.
    #[test]
    fn init_forgets_the_last_config_and_rerolls() {
        ts30_gameplay::tables::make_default();
        let r = rules();
        let mut m = BuyMenu::default();
        m.open(&r, &mut FixedRng(vec![], 0));
        assert!(!m.random_first, "the first open used up the random loadout");
        let rolled = m.current.clone();
        assert!(rolled.entries.iter().any(|e| e.id != 0 || e.count != 0), "a random loadout was picked");
        m.last = rolled.clone();
        m.close();
        let mut saved: [Config; SAVED_CONFIGS] = Default::default();
        saved[2].total = 1234;
        let n = BuyMenu::new(saved.clone());
        assert!(!n.active);
        assert_eq!((n.page, n.category, n.pending_att, n.pending_weapon), (0, 0, 0, -1));
        assert_eq!(n.current, Config::default());
        assert_eq!(n.last, Config::default(), "the previous map's last config is gone");
        assert_eq!(n.saved, saved, "saved configs as read from tsbuylist.tsa");
        assert!(n.new_config && n.random_first);
        // So the next open rolls again instead of reopening the last config.
        let mut n = n;
        n.open(&r, &mut FixedRng(vec![], 0));
        assert!(!n.random_first);
        assert_eq!(n.current, rolled, "the same rng rolls the same loadout, not an empty last config");
    }

    #[test]
    fn add_and_costs() {
        ts30_gameplay::tables::make_default();
        let r = rules();
        let mut m = BuyMenu::default();
        m.reset_config(&r);
        assert_eq!((m.current.total, m.current.slots), (1900, 10), "kevlar from cl_buykevlar");
        assert!(m.add(13, 8, false, false, &r));
        assert_eq!(m.current.total, 1900 + 3000 + 1800);
        assert_eq!(m.current.slots, 50);
        assert_eq!(m.current.ammo[6].0, 90, "AK ammo filled to the ammocount 0 max");
        assert!(m.add(25, 0, true, false, &r), "full load of knives");
        let k = m.current.entries.iter().find(|e| e.id == 25).unwrap();
        assert_eq!((k.count, k.cost), (5, 500));
        assert!(!m.add(25, 0, false, false, &r), "already at max count");
        assert!(!m.add(24, 0, false, false, &r), "grenades need gamemode > 1");
        assert!(!m.add(ITEM_KEVLAR, 0, false, true, &r), "one kevlar");
        // Beretta then Beretta again = akimbo.
        assert!(m.add(2, 1, false, false, &r));
        assert!(m.add(2, 2, false, false, &r));
        let b = m.current.entries.iter().find(|e| e.id == 2).unwrap();
        assert!(b.akimbo);
        assert_eq!(b.att, 3);
        assert_eq!(b.cost, 900 * 2 + 900 + 800);
    }

    /// Issue #108: the ammo fill records what the ammo cost, and only when there was money to
    /// spend; the summary's total includes it, and a `CashBased()` menu shows the money left
    /// after it; `ammocount` outside 1..3 takes column 0.
    #[test]
    fn ammo_spend_and_summary() {
        ts30_gameplay::tables::make_default();
        let r = Rules { cash: 10000, ..rules() };
        let mut m = BuyMenu::default();
        assert!(m.add(13, 0, false, false, &r));
        let price = tables().ammo(6).unwrap().price;
        let (rounds, buys) = m.current.ammo[6];
        assert_eq!((rounds, m.ammo_spend), (90, buys * price), "AK47 ammo: {buys} buys");
        assert!(m.ammo_spend > 0);
        assert_eq!(summary_header(&m, &r), ("#Buy_Total", vec![3000 + m.ammo_spend]));
        let cash_based = Rules { cash_based: true, ..r };
        assert_eq!(summary_header(&m, &cash_based), ("#Buy_Total_CashBased", vec![10000 - 3000 - m.ammo_spend, 3000 + m.ammo_spend]));
        // No money to spend: the ammo is cleared and the spend keeps its value.
        let spend = m.ammo_spend;
        m.recompute_ammo(&Rules { cash: 3000, ..r });
        assert_eq!((m.current.ammo[6], m.ammo_spend), ((0, 0), spend));
        // `ammocount` 4 (or negative) is column 0's 90, not column 3's 0.
        for ammocount in [4, -1] {
            m.recompute_ammo(&Rules { ammocount, ..r });
            assert_eq!(m.current.ammo[6].0, 90, "ammocount {ammocount}");
        }
        m.recompute_ammo(&Rules { ammocount: 3, ..r });
        assert_eq!(m.current.ammo[6].0, 0, "column 3");
    }

    /// The category column's mouse rows: TS 3.0's six, or `(m_Teamplay != 0) + 5`
    /// ([`MenuRules::category_rows_by_teamplay`]).
    #[test]
    fn category_mouse_rows() {
        let m = BuyMenu { page: page::CATEGORY, ..BuyMenu::default() };
        let at = |row: i32, r: &Rules| m.hit_test(8 + 128 + 1, 300 + row * 20 + 1, 600, r);
        for gamemode in [0, 1, 2] {
            let r = Rules { gamemode, ..rules() };
            assert_eq!((at(5, &r), at(6, &r), at(7, &r)), (Hit::Row(page::CATEGORY, 6), Hit::Row(page::CATEGORY, 10), Hit::Miss));
        }
        let by_teamplay =
            |gamemode| Rules { gamemode, menu: MenuRules { category_rows_by_teamplay: true, ..MenuRules::default() }, ..rules() };
        assert_eq!((at(5, &by_teamplay(0)), at(6, &by_teamplay(0))), (Hit::Row(page::CATEGORY, 10), Hit::Miss));
        // Mode 1: the sixth row is drawn as "back" (no accessory row) but is key 6.
        assert_eq!((at(5, &by_teamplay(1)), at(6, &by_teamplay(1))), (Hit::Row(page::CATEGORY, 6), Hit::Row(page::CATEGORY, 10)));
        assert_eq!(at(6, &by_teamplay(2)), Hit::Row(page::CATEGORY, 10));
    }

    #[test]
    fn slot_limit() {
        ts30_gameplay::tables::make_default();
        let r = rules();
        let mut m = BuyMenu::default();
        assert!(m.add(18, 0, false, false, &r)); // barrett, 70 slots
        assert!(!m.add(5, 0, false, false, &r), "M4 (40) doesn't fit in 81");
        assert!(m.add(ITEM_KEVLAR, 0, false, true, &r));
        assert_eq!(m.current.slots, 80);
    }

    #[test]
    fn restrictions() {
        ts30_gameplay::tables::make_default();
        assert!(!weapon_restricted(13, 0x1f));
        assert!(weapon_restricted(13, 0x17), "rifles need 0x08");
        assert!(weapon_restricted(33, 0x0f), "sawed-off counts as a shotgun");
        assert!(weapon_restricted(34, 0x1e), "katana counts as a knife");
        assert!(weapon_restricted(16, 0x1b), "akimbo uzi counts as an SMG");
        assert!(weapon_restricted(24, 0), "mask 0 blocks everything in the client check");
        assert!(!weapon_restricted(24, 0x1f));
    }

    #[test]
    fn submit_protocol() {
        ts30_gameplay::tables::make_default();
        let r = rules();
        let mut m = BuyMenu::default();
        m.reset_config(&r);
        m.add(13, 8, false, false, &r);
        m.add(25, 0, true, false, &r);
        m.add(9, 1, true, false, &r); // akimbo MK23 with silencer
        let mut a = Vec::new();
        assert!(m.submit(&mut a, &rules()));
        assert_eq!(a[0], Action::ServerCmd("rst_cfg".into()));
        // Sorted: knives (no ammo) first, then by price: kevlar 1900 (item), MK23 akimbo 2300, AK 4800.
        assert_eq!(a[1], Action::ServerCmd("tkw 25_5 9_1 9_0 13_8 ".into()));
        assert_eq!(a[2], Action::ServerCmd("tki 16 ".into()));
        assert_eq!(a[3], Action::ServerCmd("tka 2_15 6_3 ".into()));
        assert_eq!(a[4], Action::Message("#Buy_UsingNewConfig".into(), [0; 3]));
        assert_eq!(m.last, m.current);
    }

    #[test]
    fn mouse_hit_test() {
        ts30_gameplay::tables::make_default();
        let r = rules();
        let mut b = BuyMenu::default();
        let at = |col: i32, row: i32| (8 + col * 128 + 5, 240 + row * 20 + 3);
        let hit = |b: &BuyMenu, (x, y): (i32, i32)| b.hit_test(x, y, 480, &r);
        assert_eq!(hit(&b, at(0, 0)), Hit::Row(page::MAIN, 1));
        assert_eq!(hit(&b, at(0, 7)), Hit::Row(page::MAIN, 10), "the last main row is back");
        assert_eq!(hit(&b, at(0, 8)), Hit::Miss);
        assert_eq!(hit(&b, (4, 300)), Hit::Miss);
        assert_eq!(hit(&b, (100, 200)), Hit::Miss, "above mid-screen");
        assert_eq!(hit(&b, at(1, 0)), Hit::Miss, "no category column on the main page");
        b.page = page::CATEGORY;
        assert_eq!(hit(&b, at(1, 6)), Hit::Row(page::CATEGORY, 10));
        assert_eq!(hit(&b, at(0, 7)), Hit::Miss, "a parent column has no back row");
        assert_eq!(hit(&b, at(0, 2)), Hit::Row(page::MAIN, 3), "parent columns stay clickable");
        b.page = page::WEAPONS;
        b.category = 0;
        // Handguns: ten weapons, none restricted under 0x1f; the 11th row is back.
        assert_eq!(hit(&b, at(2, 9)), Hit::Row(page::WEAPONS, 11), "keys skip 10");
        assert_eq!(hit(&b, at(2, 10)), Hit::Row(page::WEAPONS, 10));
        b.page = page::SAVE;
        assert_eq!(hit(&b, at(1, 3)), Hit::Row(page::SAVE, 3), "save/load rows are the key itself");
        b.page = page::REMOVE;
        b.current.entries[0].id = 13;
        assert_eq!(hit(&b, at(1, 0)), Hit::Row(page::REMOVE, 1));
        assert_eq!(hit(&b, at(1, 1)), Hit::Row(page::REMOVE, 10));
        for e in b.current.entries.iter_mut() {
            e.id = 13;
        }
        assert_eq!(hit(&b, at(1, 16)), Hit::Unset, "TS leaves the page unwritten");
    }

    #[test]
    fn page_machine() {
        ts30_gameplay::tables::make_default();
        let r = rules();
        let mut rng = FixedRng(vec![], 0);
        let mut m = BuyMenu { random_first: false, ..Default::default() };
        m.open(&r, &mut rng);
        assert!(m.active);
        m.handle_key(page::MAIN, 2, &r, &mut rng);
        assert_eq!(m.page, page::CATEGORY);
        m.handle_key(page::CATEGORY, 3, &r, &mut rng); // rifles
        assert_eq!((m.page, m.category), (page::WEAPONS, 2));
        m.handle_key(page::WEAPONS, 2, &r, &mut rng); // AK47
        assert_eq!((m.page, m.pending_weapon), (page::OPTIONS, 13));
        m.handle_key(page::OPTIONS, 5, &r, &mut rng); // scope (bit 3)
        assert_eq!(m.pending_att, 8);
        m.handle_key(page::OPTIONS, 2, &r, &mut rng); // silencer not allowed on the AK
        assert_eq!(m.pending_att, 8);
        m.handle_key(page::OPTIONS, 1, &r, &mut rng);
        assert_eq!(m.page, page::CATEGORY);
        assert!(m.current.entries.iter().any(|e| e.id == 13 && e.att == 8));
        m.handle_key(page::CATEGORY, 10, &r, &mut rng);
        assert_eq!(m.page, page::MAIN);
        let a = m.handle_key(page::MAIN, 1, &r, &mut rng);
        assert!(a.contains(&Action::Close));
        assert!(!m.active);
        // Remove page.
        m.open(&r, &mut rng);
        m.handle_key(page::MAIN, 3, &r, &mut rng);
        let a = m.handle_key(page::REMOVE, 1, &r, &mut rng);
        assert_eq!(a[0], Action::MessageArg("#Buy_Removed".into(), "AK47".into(), [0; 3]));
        assert!(m.current.entries.iter().all(|e| e.id == 0));
    }

    #[test]
    fn random_config_is_valid() {
        ts30_gameplay::tables::make_default();
        let r = rules();
        let mut m = BuyMenu::default();
        // melee=knife full, pistol idx 3 (deagle), rifle k=4 (AK)
        let mut rng = FixedRng(vec![0, 0, 3, 0, 0, 0, 4, 0, 0, 0], 0);
        m.random_config(0.8, &r, &mut rng);
        let ids: Vec<i32> = m.current.entries.iter().filter(|e| e.id != 0).map(|e| e.id).collect();
        assert_eq!(ids, [ITEM_KEVLAR, 25, 12, 13]);
        assert!(m.current.slots <= SLOTS);
        assert_eq!(m.last, m.current, "RandomConfig ends by copying the config to the last one");
    }

    /// A tiny xorshift for the fuzz tests.
    struct XorShift(u64);
    impl XorShift {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn bytes(&mut self, n: usize) -> Vec<u8> {
            (0..n).map(|_| self.next() as u8).collect()
        }
    }
    impl Rng for XorShift {
        fn long(&mut self, lo: i32, hi: i32) -> i32 {
            lo.wrapping_add((self.next() % (hi.wrapping_sub(lo) as u32 as u64 + 1)) as i32)
        }
        fn float(&mut self, lo: f32, hi: f32) -> f32 {
            lo + (self.next() % 1000) as f32 / 1000.0 * (hi - lo)
        }
    }

    /// Garbage, short, oversized and all-0xff buy lists parse, and every page and key on the
    /// loaded configs (any totals, ids, counts and costs) neither panics nor hangs.
    #[test]
    fn garbage_buylist_and_keys() {
        ts30_gameplay::tables::make_default();
        let mut rng = XorShift(0x9e37_79b9_7f4a_7c15);
        let mut files = vec![Vec::new(), vec![0xff; 3], vec![0xff; CONFIG_SIZE * SAVED_CONFIGS + 7], vec![0; CONFIG_SIZE - 1]];
        for _ in 0..40 {
            let n = (rng.next() % (CONFIG_SIZE as u64 * 10)) as usize;
            files.push(rng.bytes(n));
        }
        for file in files {
            let saved = parse_buylist(&file);
            assert_eq!(write_buylist(&saved).len(), CONFIG_SIZE * SAVED_CONFIGS);
            for gamemode in [0, 2] {
                let r = Rules { gamemode, restriction: rng.next() as i32, ammocount: rng.next() as i32, ..Rules::default() };
                let mut m = BuyMenu { saved: saved.clone(), ..BuyMenu::default() };
                m.open(&r, &mut rng);
                for _ in 0..200 {
                    let pg = (rng.next() % 10) as i32;
                    let key = (rng.next() % 13) as i32;
                    m.page = pg;
                    m.handle_key(pg, key, &r, &mut rng);
                    let (x, y) = ((rng.next() % 2000) as i32 - 100, (rng.next() % 2000) as i32 - 100);
                    m.hit_test(x, y, 480, &r);
                }
                for c in &saved {
                    m.current = c.clone();
                    m.recompute_ammo(&r);
                    m.submit(&mut Vec::new(), &r);
                    m.remove(0, &r);
                    m.remove(ENTRIES + 3, &r);
                    m.add(25, 0, true, false, &r);
                    m.add(13, 0xffff_ffff, true, false, &r);
                }
            }
        }
        // Extreme numbers in a config.
        let mut c = Config { total: i32::MAX, slots: i32::MIN, ..Config::default() };
        c.entries[0] = Entry { is_item: false, id: 25, att: 0, cost: i32::MIN, akimbo: true, count: i32::MIN };
        c.entries[1] = Entry { is_item: true, id: -1, att: u32::MAX, cost: i32::MAX, akimbo: true, count: i32::MAX };
        let r = rules();
        let mut m = BuyMenu { current: c, ..BuyMenu::default() };
        m.add(25, 0, true, false, &r);
        m.recompute_ammo(&r);
        m.remove(0, &r);
        m.remove(0, &r);
        m.submit(&mut Vec::new(), &r);
    }

    #[test]
    fn short_config_reads_zero() {
        assert_eq!(Config::from_bytes(&[0xff; CONFIG_SIZE - 1]), Config::default());
        assert_eq!(rd(&[1, 0, 0], 0), 0, "past the end");
    }

    #[test]
    fn summary_text() {
        let loc = |k: &str| k.trim_start_matches("#Buy_").to_owned();
        assert_eq!(attachment_summary(0b1011, &loc), "(Sil,Las,Sco)");
        assert_eq!(attachment_summary(0, &loc), "");
    }
}
