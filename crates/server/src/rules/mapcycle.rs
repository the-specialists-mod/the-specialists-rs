//! `CHalfLifeMultiplay::ChangeLevel` (0xd0784) and its map cycle: the SDK's `mapcyclefile`
//! parser (`ReloadMapCycleFile` 0xd032c with `COM_Parse` 0xd01c8 / `COM_TokenWaiting` 0xd02d8),
//! `minplayers`/`maxplayers`, the per-map server commands (`ExtractCommandString` 0xd0698), and
//! the chosen map vote. TS calls it every frame once the intermission is over; the engine only
//! honours the first `changelevel` of a map, but each call still advances the cycle.

use super::state;
use goldsrc::Global;
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::cell::RefCell;

/// `mapcycle_item_s`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Item {
    pub map: String,
    pub min_players: i32,
    pub max_players: i32,
    /// The rule buffer without `minplayers`/`maxplayers` (`\key\value...`).
    pub rules: String,
}

/// `ChangeLevel`'s statics `szPreviousMapCycleFile` and `mapcycle`: they live for the process,
/// across maps, as the `.so`'s static data does.
#[derive(Default)]
pub struct Cycle {
    pub previous_file: String,
    pub items: Vec<Item>,
    /// `mapcycle.next_item`, an index into `items`.
    pub next: usize,
}

static CYCLE: Global<RefCell<Cycle>> = Global(RefCell::new(Cycle { previous_file: String::new(), items: Vec::new(), next: 0 }));

/// `com_token` is 1500 bytes; `ReloadMapCycleFile` copies the map into 32 and the rules into 1024.
const TOKEN_MAX: usize = 1499;
const MAP_MAX: usize = 31;
const RULES_MAX: usize = 1023;

/// GoldSrc's `char` is signed: bytes from 0x80 compare below `' '` and count as white space.
fn is_white(b: u8) -> bool {
    (b as i8) <= b' ' as i8
}

/// `COM_Parse` (0xd01c8): the next token and what follows it, `None` at the end of the data.
pub fn com_parse(data: &[u8]) -> (Vec<u8>, Option<&[u8]>) {
    let mut i = 0;
    loop {
        while i < data.len() && is_white(data[i]) {
            if data[i] == 0 {
                return (Vec::new(), None);
            }
            i += 1;
        }
        if i >= data.len() {
            return (Vec::new(), None);
        }
        if data[i] == b'/' && data.get(i + 1) == Some(&b'/') {
            while i < data.len() && data[i] != 0 && data[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        break;
    }
    let mut tok = Vec::new();
    let push = |tok: &mut Vec<u8>, b: u8| {
        // `com_token` is 1500 bytes; TS writes past it for longer tokens.
        if tok.len() < TOKEN_MAX {
            tok.push(b);
        }
    };
    let c = data[i];
    if c == b'"' {
        i += 1;
        loop {
            let c = data.get(i).copied().unwrap_or(0);
            i += 1;
            if c == b'"' || c == 0 {
                return (tok, Some(data.get(i..).unwrap_or(&[])));
            }
            push(&mut tok, c);
        }
    }
    let single = |c: u8| matches!(c, b'{' | b'}' | b')' | b'(' | b'\'' | b',');
    if single(c) {
        tok.push(c);
        return (tok, Some(&data[i + 1..]));
    }
    loop {
        push(&mut tok, data[i]);
        i += 1;
        let c = data.get(i).copied().unwrap_or(0);
        if single(c) || is_white(c) {
            break;
        }
    }
    (tok, Some(&data[i..]))
}

/// `COM_TokenWaiting` (0xd02d8): something other than white space before the end of the line.
pub fn com_token_waiting(data: &[u8]) -> bool {
    for &b in data {
        if b == 0 || b == b'\n' {
            return false;
        }
        // glibc `isspace`/`isalnum`; bytes from 0x80 are neither in the C locale.
        if !b.is_ascii_whitespace() && b != 0x0b || b.is_ascii_alphanumeric() {
            return true;
        }
    }
    false
}

/// `Info_ValueForKey` over a `\key\value` buffer (`pfnInfoKeyValue`), "" when missing.
pub fn info_value<'a>(buf: &'a str, key: &str) -> &'a str {
    let mut it = buf.strip_prefix('\\').unwrap_or(buf).split('\\');
    while let Some(k) = it.next() {
        let v = it.next().unwrap_or("");
        if k == key {
            return v;
        }
    }
    ""
}

/// `Info_RemoveKey` (`pfnInfo_RemoveKey`).
pub fn info_remove(buf: &str, key: &str) -> String {
    let mut out = String::new();
    let mut it = buf.strip_prefix('\\').unwrap_or(buf).split('\\');
    while let Some(k) = it.next() {
        let v = it.next();
        if k == key || (k.is_empty() && v.is_none()) {
            continue;
        }
        out.push('\\');
        out.push_str(k);
        out.push('\\');
        out.push_str(v.unwrap_or(""));
    }
    out
}

/// `atoi` (`__strtol_internal(s, 0, 10)`): white space, a sign, digits; saturates.
pub fn atoi(s: &str) -> i32 {
    let s = s.trim_start();
    let (neg, digits) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let mut v: i64 = 0;
    for b in digits.bytes().take_while(u8::is_ascii_digit) {
        v = (v * 10 + (b - b'0') as i64).min(i32::MAX as i64 + 1);
    }
    let v = if neg { -v } else { v };
    v.clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

fn cut(b: &[u8], n: usize) -> String {
    String::from_utf8_lossy(&b[..b.len().min(n)]).into_owned()
}

/// `ReloadMapCycleFile` (0xd032c): one map per line, optionally followed by a `\key\value` rule
/// buffer; maps the engine doesn't know are skipped with a console note. `minplayers` and
/// `maxplayers` are kept in 0..=maxClients and taken out of the rules.
pub fn parse_cycle(data: &[u8], max_clients: i32, valid: impl Fn(&str) -> bool, mut skipped: impl FnMut(&str)) -> Vec<Item> {
    let data = &data[..data.iter().position(|&b| b == 0).unwrap_or(data.len())];
    let mut items = Vec::new();
    let mut rest = data;
    loop {
        let (tok, after) = com_parse(rest);
        if tok.is_empty() {
            break;
        }
        rest = after.unwrap_or(&[]);
        let map = cut(&tok, MAP_MAX);
        let mut buffer = None;
        if com_token_waiting(rest) {
            let (tok, after) = com_parse(rest);
            rest = after.unwrap_or(&[]);
            if !tok.is_empty() {
                buffer = Some(cut(&tok, RULES_MAX));
            }
        }
        if !valid(&map) {
            skipped(&map);
            continue;
        }
        let mut item = Item { map, ..Item::default() };
        if let Some(buf) = buffer {
            let limit = |s: &str| atoi(s).max(0).min(max_clients);
            let min = info_value(&buf, "minplayers");
            if !min.is_empty() {
                item.min_players = limit(min);
            }
            let max = info_value(&buf, "maxplayers");
            if !max.is_empty() {
                item.max_players = limit(max);
            }
            item.rules = info_remove(&info_remove(&buf, "minplayers"), "maxplayers");
        }
        items.push(item);
    }
    items
}

/// The traversal of `ChangeLevel`: from `next`, the first map whose player limits fit `players`.
/// The loop stops before the entry that wraps back to `next`, and any limit that fit on the way
/// counts as found, so when nothing qualifies that last entry is taken unchecked; with no limit
/// met anywhere it is `next` itself. Returns the item index and the last limits that fit.
pub fn pick(items: &[Item], next: usize, players: i32) -> (usize, i32, i32) {
    let n = items.len();
    let (mut min, mut max) = (0, 0);
    let mut found = false;
    let mut i = next;
    while (i + 1) % n != next {
        let it = &items[i];
        let mut keep_looking = false;
        if it.min_players != 0 {
            if players >= it.min_players {
                found = true;
                min = it.min_players;
            } else {
                keep_looking = true;
            }
        }
        if it.max_players != 0 {
            if players <= it.max_players {
                found = true;
                max = it.max_players;
            } else {
                keep_looking = true;
            }
        }
        if !keep_looking {
            found = true;
            break;
        }
        i = (i + 1) % n;
    }
    if !found {
        i = next;
    }
    (i, min, max)
}

/// `ExtractCommandString` (0xd0698): `\key\value\...` into `key value\n` lines (a key with an
/// empty value is just `key\n`). TS's 512-byte key/value buffers and 1500-byte command buffer are
/// kept as limits.
pub fn extract_commands(rules: &str) -> String {
    let mut out = String::new();
    let s = rules.strip_prefix('\\').unwrap_or(rules).as_bytes();
    let mut i = 0;
    loop {
        let start = i;
        while i < s.len() && s[i] != b'\\' {
            i += 1;
        }
        if i >= s.len() {
            return out;
        }
        let key = &s[start..i];
        i += 1;
        let vstart = i;
        while i < s.len() && s[i] != b'\\' {
            i += 1;
        }
        let value = &s[vstart..i];
        let mut line = cut(key, 511);
        if !value.is_empty() {
            line.push(' ');
            line.push_str(&cut(value, 511));
        }
        line.push('\n');
        if out.len() + line.len() <= TOKEN_MAX {
            out.push_str(&line);
        }
        if i >= s.len() {
            return out;
        }
        i += 1;
    }
}

/// `CHalfLifeMultiplay::ChangeLevel` (0xd0784).
pub fn change_level() {
    let eng = engine();
    let file = eng.cvar_string(c"mapcyclefile").to_string_lossy().into_owned();
    let players = super::slot_players().len() as i32;
    let current = eng.string(eng.globals().mapname).to_string_lossy().into_owned();
    let max_clients = eng.max_clients();
    // `szFirstMapInList` is "hldm1" until a cycle is in use. Without a cycle `szNextMap` is
    // uninitialised stack in TS; it is taken as an invalid map, which gives "hldm1" too.
    let mut first = "hldm1".to_owned();
    let mut next_map = String::new();
    let mut commands = String::new();
    let mut rules = String::new();
    let (mut min, mut max) = (0, 0);
    {
        let mut c = CYCLE.0.borrow_mut();
        let mut do_cycle = true;
        if !file.eq_ignore_ascii_case(&c.previous_file) {
            c.previous_file = file.clone();
            c.items.clear();
            c.next = 0;
            let data = eng.load_file(&file).unwrap_or_default();
            c.items = parse_cycle(
                &data,
                max_clients,
                |m| eng.is_map_valid(m),
                |m| eng.alert(ALERT_TYPE::at_console, &format!("Skipping {m} from mapcycle, not a valid map\n")),
            );
            if c.items.is_empty() {
                eng.alert(ALERT_TYPE::at_console, &format!("Unable to load map cycle file {file}\n"));
                do_cycle = false;
            }
        }
        if do_cycle && !c.items.is_empty() {
            first = current.clone();
            let (i, lo, hi) = pick(&c.items, c.next, players);
            (min, max) = (lo, hi);
            c.next = (i + 1) % c.items.len();
            let item = &c.items[i];
            next_map = item.map.clone();
            commands = extract_commands(&item.rules);
            rules = item.rules.clone();
        }
    }
    // The chosen map vote (`TSGameRules+0x1824`) wins unless it is "nextmap"; it's used once.
    if let Some(voted) = state::with(|g| g.votes.override_map().map(str::to_owned)) {
        next_map = voted;
    }
    state::with(|g| g.votes.chosen.clear());
    // `CTSGameRules::Init`: the TS state is reset for the frames until the next map loads (it
    // clears the chosen map too).
    let now = eng.time();
    state::with(|g| g.ts_init(now));
    crate::bot_backend::map_init(); // mp.dll: nodes and bot texts (of the map that is ending)
    crate::weapons::reset_bullets();
    if next_map.is_empty() || !eng.is_map_valid(&next_map) {
        next_map = first;
    }
    eng.alert(ALERT_TYPE::at_console, &format!("CHANGE LEVEL: {next_map}\n"));
    if min != 0 || max != 0 {
        eng.alert(ALERT_TYPE::at_console, &format!("PLAYER COUNT:  min {min} max {max} current {players}\n"));
    }
    if !rules.is_empty() {
        eng.alert(ALERT_TYPE::at_console, &format!("RULES:  {rules}\n"));
    }
    eng.change_level(&next_map);
    if !commands.is_empty() {
        eng.server_command(&commands);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(spec: &[(&str, i32, i32)]) -> Vec<Item> {
        spec.iter().map(|&(m, lo, hi)| Item { map: m.into(), min_players: lo, max_players: hi, rules: String::new() }).collect()
    }

    #[test]
    fn parse_lines_rules_and_comments() {
        let text = b"// cycle\nts_a\nts_b \"\\minplayers\\4\\maxplayers\\40\\mp_timelimit\\20\"\nbogus\nts_c\n";
        let mut skipped = Vec::new();
        let c = parse_cycle(text, 16, |m| m != "bogus", |m| skipped.push(m.to_owned()));
        assert_eq!(skipped, ["bogus"]);
        assert_eq!(c.len(), 3);
        assert_eq!(c[0], Item { map: "ts_a".into(), ..Item::default() });
        // maxplayers is capped at maxClients, and both keys leave the rules.
        assert_eq!(c[1], Item { map: "ts_b".into(), min_players: 4, max_players: 16, rules: "\\mp_timelimit\\20".into() });
        assert_eq!(extract_commands(&c[1].rules), "mp_timelimit 20\n");
        assert_eq!(c[2].map, "ts_c");
    }

    #[test]
    fn parse_token_rules() {
        assert_eq!(com_parse(b"  abc def"), (b"abc".to_vec(), Some(&b" def"[..])));
        assert_eq!(com_parse(b"a(b"), (b"a".to_vec(), Some(&b"(b"[..])));
        assert_eq!(com_parse(b"\"x y\"z"), (b"x y".to_vec(), Some(&b"z"[..])));
        assert_eq!(com_parse(b"   "), (Vec::new(), None));
        // Bytes from 0x80 are white space to a signed `char`.
        assert_eq!(com_parse(b"ab\xe9cd").0, b"ab".to_vec());
        assert!(com_token_waiting(b"  x\n"));
        assert!(!com_token_waiting(b"   \nx"));
    }

    #[test]
    fn map_names_are_cut_to_31_bytes() {
        let long = "m".repeat(40);
        let c = parse_cycle(long.as_bytes(), 8, |_| true, |_| {});
        assert_eq!(c[0].map.len(), 31);
    }

    #[test]
    fn pick_walks_the_cycle_and_respects_limits() {
        let c = items(&[("a", 0, 0), ("b", 0, 0), ("c", 0, 0)]);
        assert_eq!(pick(&c, 0, 1), (0, 0, 0));
        assert_eq!(pick(&c, 2, 1), (2, 0, 0));
        // "b" wants 4+, so with 2 players "c" is next.
        let c = items(&[("a", 0, 0), ("b", 4, 0), ("c", 0, 0)]);
        assert_eq!(pick(&c, 1, 2), (2, 0, 0));
        assert_eq!(pick(&c, 1, 5), (1, 4, 0));
        // Nothing fits and no limit was met: the start item.
        let c = items(&[("a", 8, 0), ("b", 8, 0), ("c", 8, 0)]);
        assert_eq!(pick(&c, 0, 1), (0, 0, 0));
        // A met limit counts as found, so the unchecked last entry is taken.
        let c = items(&[("a", 1, 2), ("b", 8, 0), ("c", 8, 0)]);
        assert_eq!(pick(&c, 0, 3), (2, 1, 0));
        assert_eq!(pick(&items(&[("solo", 9, 0)]), 0, 1), (0, 0, 0));
    }

    #[test]
    fn info_buffers() {
        assert_eq!(info_value("\\a\\1\\b\\2", "b"), "2");
        assert_eq!(info_value("\\a\\1", "c"), "");
        assert_eq!(info_remove("\\a\\1\\b\\2", "a"), "\\b\\2");
        assert_eq!(extract_commands("\\sv_gravity\\400\\restart\\"), "sv_gravity 400\nrestart\n");
        assert_eq!(extract_commands(""), "");
        assert_eq!(atoi(" -12x"), -12);
        assert_eq!(atoi("99999999999"), i32::MAX);
    }
}
