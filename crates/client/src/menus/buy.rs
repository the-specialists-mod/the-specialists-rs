//! TS buy menu (`CTSBuyMenu`, embedded in `CHudTSWeapons` at 0x100d1460): an immediate-mode page
//! machine drawn with `FillRGBA`/`DrawHudString`, digit keys through `slotN`, mouse clicks through
//! `IN_MouseEvent`. Saved loadouts live in `<gamedir>/tsbuylist.tsa` (nine 0x408-byte configs).
//! Reference: architecture-hud.md §6.2; decomp `unnamed_100337f0.c` 10036200..100398e0.

use shared::weapons::{WeaponKind, tables};

pub const CONFIG_SIZE: usize = 0x408;
pub const SAVED_CONFIGS: usize = 9;
pub const ENTRIES: usize = 16;
/// `FUN_10036640`: money available. `CashBased()` is hard-wired off in 3.0, so always 30000.
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

fn rd(b: &[u8], o: usize) -> i32 {
    i32::from_le_bytes(b[o..o + 4].try_into().unwrap())
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
                is_item: b[o] != 0,
                id: rd(b, o + 4),
                att: rd(b, o + 8) as u32,
                cost: rd(b, o + 0xc),
                akimbo: b[o + 0x30] != 0,
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
}

impl Default for Rules {
    fn default() -> Self {
        Rules { gamemode: 0, restriction: 0x1f, ammocount: 0, buy_kevlar: true }
    }
}

/// Weapon type for the restriction system (`FUN_1000b670`): katana → knife, sawed-off →
/// shotgun, akimbo uzi/skorpion → SMG, other akimbos → pistol.
fn restriction_kind(id: i32) -> i32 {
    let Some(w) = tables().weapon(id as u8) else { return 0 };
    match (id, w.kind) {
        (34, _) => 3,
        (33, _) => 1,
        (16 | 37, _) => 9,
        (_, WeaponKind::Akimbo) => 8,
        (_, WeaponKind::Shotgun) => 1,
        (_, WeaponKind::Knife) => 3,
        (_, WeaponKind::Rifle) => 6,
        (_, WeaponKind::Sniper) => 7,
        (_, WeaponKind::Pistol) => 8,
        (_, WeaponKind::Smg) => 9,
        // Grenade, Golden Colts and kung fu are never restricted (server weapons.md §7.2).
        _ => 0,
    }
}

/// `FUN_1000b6d0`: is the weapon blocked by the restriction mask?
pub fn weapon_restricted(id: i32, mask: i32) -> bool {
    if !(1..38).contains(&id) {
        return false;
    }
    if mask == 0 {
        return true;
    }
    match restriction_kind(id) {
        1 => mask & 0x10 == 0,
        3 => mask & 0x01 == 0,
        6 | 7 => mask & 0x08 == 0,
        8 => mask & 0x02 == 0,
        9 => mask & 0x04 == 0,
        _ => false,
    }
}

/// `FUN_100365e0`: restricted for buying.
pub fn restricted(id: i32, item: bool, r: &Rules) -> bool {
    if !item { weapon_restricted(id, r.restriction) || (r.gamemode < 2 && id == 24) } else { r.gamemode < 2 && id & ITEM_STEALTH != 0 }
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
pub fn slots_for(id: i32, full: bool, item: bool) -> i32 {
    if item {
        return if id & ITEM_KEVLAR != 0 { 10 } else { 0 };
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

/// Ammo type limits under the current `ammocount`.
fn ammo_max(t: usize, r: &Rules) -> i32 {
    tables().ammo(t as u8).map_or(0, |a| a.max_carry[(r.ammocount.clamp(0, 3)) as usize])
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

/// Category table (`+0x2d6c`, 12 rows × 5 categories, set in `Init` 10036200).
pub const TABLE: [[i32; 5]; 12] = {
    let mut t = [[-1; 5]; 12];
    let cols: [&[i32]; 5] =
        [&[1, 9, 12, 14, 2, 21, 22, 28, 31, 36], &[3, 6, 7, 17, 19, 23], &[5, 13, 15, 27, 18], &[4, 11, 20, 26, 33], &[24, 25, 32, 34, 35]];
    let mut c = 0;
    while c < 5 {
        let mut r = 0;
        while r < cols[c].len() {
            t[r][c] = cols[c][r];
            r += 1;
        }
        c += 1;
    }
    t
};

/// Category names. TS stores `Buy_SMGs` without the `#` so it shows raw; the port adds it.
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
    /// `+0x2f0c`: money left after ammo.
    pub leftover: i32,
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
            leftover: 0,
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
        self.leftover = 0;
    }

    /// `Open` (10036670), minus the demo check and cursor centring (caller).
    pub fn open(&mut self, r: &Rules, rng: &mut dyn Rng) {
        if self.active {
            return;
        }
        self.current = self.last.clone();
        self.leftover = 0;
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
        if r.buy_kevlar {
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

    /// `FUN_100392b0`: fill ammo round-robin over the guns with the money left.
    pub fn recompute_ammo(&mut self, r: &Rules) {
        let mut budget = CASH - self.current.total;
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
        }
        self.leftover = budget.max(0);
    }

    /// `FUN_10038e40`: remove entry `i` and compact the list.
    pub fn remove(&mut self, i: usize) {
        let e = self.current.entries[i];
        self.current.total -= e.cost;
        if !e.is_item {
            let w = tables().weapon(e.id as u8).map_or(0, |w| w.slots);
            self.current.slots -= if e.count == 0 { w * (e.akimbo as i32 + 1) } else { w * e.count };
        } else if e.id & ITEM_KEVLAR != 0 {
            self.current.slots -= 10;
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
        if let Some(w) = info {
            if w.akimbo == -1 && !w.stack {
                full = false;
            }
        }
        if cost(id, att, full, item) + self.current.total > CASH {
            return false;
        }
        if slots_for(id, full, item) + self.current.slots > SLOTS {
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
            let w = info.unwrap();
            if w.stack {
                if e.count >= w.pellets {
                    return false;
                }
                let n = if full { w.pellets - e.count } else { 1 };
                let en = &mut self.current.entries[i];
                en.count += n;
                en.cost += w.cost * n;
                self.current.total += w.cost * n;
                self.current.slots += w.slots * n;
                self.recompute_ammo(r);
                self.new_config = true;
                return true;
            }
            // Buying the same gun again: merge attachments and make it akimbo.
            if w.akimbo != -1 && !e.akimbo {
                att |= e.att;
                full = true;
            }
            self.remove(i);
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
            e.count = if full { info.unwrap().pellets } else { 1 };
        }
        self.current.entries[j] = e;
        self.current.total += e.cost;
        self.current.slots += slots_for(id, full, item);
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
    /// (0.3 < p < 0.7) or a rifle/shotgun (p ≥ 0.7). `p <= 0` picks `p` at random.
    pub fn random_config(&mut self, p: f32, r: &Rules, rng: &mut dyn Rng) {
        self.reset_config(r);
        let (melee, full) = match rng.long(0, 5) {
            2 => (35, rng.long(0, 2) == 0),
            3 => (34, false),
            _ => (25, rng.long(0, 2) == 0),
        };
        self.add(melee, 0, full, false, r);
        let p = if p <= 0.0 { rng.float(0.0, 1.0) } else { p };
        if 0.1 < p && p < 0.9 {
            const PISTOLS: [(i32, bool); 10] = [
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
            ];
            let (id, can_akimbo) = PISTOLS[rng.long(0, 9).clamp(0, 9) as usize];
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
    pub fn submit(&mut self, actions: &mut Vec<Action>) -> bool {
        if self.current.total < 1 {
            actions.push(Action::Message("#Buy_ConfigEmpty".into(), RED));
        }
        if CASH < self.current.total {
            actions.push(Action::Message("#Buy_NoCash".into(), RED));
            return false;
        }
        if SLOTS < self.current.slots {
            return false;
        }
        // Sort (bubble, pairs 0..15): ascending price, melee/grenades (no ammo type) first.
        let ammo_of = |id: i32| tables().weapon(id as u8).map_or(0, |w| w.ammo);
        loop {
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

    /// `HandleKey(page, key)` (10038840). Keys 1..11 (10 = "0"/back, 11 = the `a` key).
    pub fn handle_key(&mut self, pg: i32, key: i32, r: &Rules, rng: &mut dyn Rng) -> Vec<Action> {
        let mut out = Vec::new();
        if !(1..=11).contains(&key) {
            return out;
        }
        match pg {
            page::MAIN => match key {
                1 => {
                    if self.submit(&mut out) {
                        self.close();
                        out.push(Action::Close);
                    }
                }
                2 => self.page = page::CATEGORY,
                3 => self.page = page::REMOVE,
                4 => {
                    self.reset_config(r);
                    out.push(Action::Message("#Buy_ConfigReset".into(), [0; 3]));
                    self.leftover = 0;
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
                6 => self.page = page::ITEMS,
                1..=5 if TABLE[0][key as usize - 1] != -1 => {
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
                let Some(&id) = TABLE.get(row).and_then(|r| r.get(self.category as usize)) else { return out };
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
                    self.remove(i);
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
                    out.push(Action::ServerCmd(format!("chgteam {key}")));
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
        Rules { gamemode: 0, restriction: 0x1f, ammocount: 0, buy_kevlar: true }
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

    #[test]
    fn add_and_costs() {
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

    #[test]
    fn slot_limit() {
        let r = rules();
        let mut m = BuyMenu::default();
        assert!(m.add(18, 0, false, false, &r)); // barrett, 70 slots
        assert!(!m.add(5, 0, false, false, &r), "M4 (40) doesn't fit in 81");
        assert!(m.add(ITEM_KEVLAR, 0, false, true, &r));
        assert_eq!(m.current.slots, 80);
    }

    #[test]
    fn restrictions() {
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
        let r = rules();
        let mut m = BuyMenu::default();
        m.reset_config(&r);
        m.add(13, 8, false, false, &r);
        m.add(25, 0, true, false, &r);
        m.add(9, 1, true, false, &r); // akimbo MK23 with silencer
        let mut a = Vec::new();
        assert!(m.submit(&mut a));
        assert_eq!(a[0], Action::ServerCmd("rst_cfg".into()));
        // Sorted: knives (no ammo) first, then by price: kevlar 1900 (item), MK23 akimbo 2300, AK 4800.
        assert_eq!(a[1], Action::ServerCmd("tkw 25_5 9_1 9_0 13_8 ".into()));
        assert_eq!(a[2], Action::ServerCmd("tki 16 ".into()));
        assert_eq!(a[3], Action::ServerCmd("tka 2_15 6_3 ".into()));
        assert_eq!(a[4], Action::Message("#Buy_UsingNewConfig".into(), [0; 3]));
        assert_eq!(m.last, m.current);
    }

    #[test]
    fn page_machine() {
        let r = rules();
        let mut rng = FixedRng(vec![], 0);
        let mut m = BuyMenu::default();
        m.random_first = false;
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
        let r = rules();
        let mut m = BuyMenu::default();
        // melee=knife full, pistol idx 3 (deagle), rifle k=4 (AK)
        let mut rng = FixedRng(vec![0, 0, 3, 0, 0, 0, 4, 0, 0, 0], 0);
        m.random_config(0.8, &r, &mut rng);
        let ids: Vec<i32> = m.current.entries.iter().filter(|e| e.id != 0).map(|e| e.id).collect();
        assert_eq!(ids, [ITEM_KEVLAR, 25, 12, 13]);
        assert!(m.current.slots <= SLOTS);
    }

    #[test]
    fn summary_text() {
        let loc = |k: &str| k.trim_start_matches("#Buy_").to_owned();
        assert_eq!(attachment_summary(0b1011, &loc), "(Sil,Las,Sco)");
        assert_eq!(attachment_summary(0, &loc), "");
    }
}
