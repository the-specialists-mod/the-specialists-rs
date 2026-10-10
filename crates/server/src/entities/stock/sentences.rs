//! Sentences (SDK sound.cpp `SENTENCEG_*`; TS 0xe00c8..0xe0d64, unchanged from the SDK).
//!
//! `sound/sentences.txt` is read once per process from `CWorld::Precache` (0xf449c). Every line
//! `NAME words...` is a sentence; its index is its position in the file, which is what the engine
//! sends (`"!<index>"`), because the client numbers the same file the same way. Names ending in
//! digits form groups (`NA0`, `NA1` → group `NA`), which doors, buttons and the HEV suit pick
//! from. `EMIT_SOUND_DYN` (0xe0a8c) and `UTIL_EmitAmbientSound` (0xec7e8) turn a `!NAME` sample
//! into `!<index>`; a name that isn't in the file isn't played.

use super::explode::emit_sound_slow;
use goldsrc::Global;
use goldsrc::edict::EdictPtr;
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::borrow::Cow;
use std::cell::RefCell;
use std::ffi::{CStr, CString};

/// `CBSENTENCENAME_MAX`.
const NAME_MAX: usize = 16;
/// `CVOXFILESENTENCEMAX` in TS (its name table is `0x6000` = 1536 × 16 bytes).
const SENTENCES_MAX: usize = 1536;
/// `CSENTENCEG_MAX`.
const GROUPS_MAX: usize = 200;
/// `CSENTENCE_LRU_MAX`.
const LRU_MAX: usize = 32;

/// `SENTENCEG`.
#[derive(Clone, Debug, PartialEq)]
struct Group {
    name: String,
    count: i32,
    lru: [u8; LRU_MAX],
}

#[derive(Default, Debug)]
struct Table {
    /// `gszallsentencenames`, in file order.
    names: Vec<String>,
    /// `rgsentenceg`.
    groups: Vec<Group>,
}

/// `fSentencesInit` and the tables. `None` until `init` has run (even when the file is missing,
/// TS leaves `fSentencesInit` false, so a later map retries).
static TABLE: Global<RefCell<Option<Table>>> = Global(RefCell::new(None));

/// `memfgets`: the next line including its `\n`, at most `max - 1` bytes.
fn memfgets<'a>(buf: &'a [u8], pos: &mut usize, max: usize) -> Option<&'a [u8]> {
    if *pos >= buf.len() {
        return None;
    }
    let stop = buf.len().min(*pos + max - 1);
    let mut i = *pos;
    while i < stop {
        i += 1;
        if buf[i - 1] == b'\n' {
            break;
        }
    }
    if i == *pos {
        return None;
    }
    let line = &buf[*pos..i];
    *pos = i;
    Some(line)
}

/// The parse half of `SENTENCEG_Init` (0xe067c). Alerts are returned so tests can run without an
/// engine.
fn parse(file: &[u8]) -> (Table, Vec<(ALERT_TYPE, String)>) {
    let mut t = Table::default();
    let mut alerts = Vec::new();
    let mut prev_group: &[u8] = b"";
    let mut pos = 0;
    while let Some(line) = memfgets(file, &mut pos, 511) {
        // The C code works on a NUL-terminated copy; an embedded NUL ends the line there.
        let line = &line[..line.iter().position(|&c| c == 0).unwrap_or(line.len())];
        let at = |k: usize| line.get(k).copied().unwrap_or(0);
        let mut i = 0;
        while at(i) == b' ' {
            i += 1;
        }
        if at(i) == 0 || at(i) == b'/' || !at(i).is_ascii_alphabetic() {
            continue;
        }
        let mut j = i;
        while at(j) != 0 && at(j) != b' ' {
            j += 1;
        }
        if at(j) == 0 {
            continue;
        }
        if t.names.len() > SENTENCES_MAX {
            alerts.push((ALERT_TYPE::at_error, "Too many sentences in sentences.txt!\n".to_owned()));
            break;
        }
        let name = String::from_utf8_lossy(&line[i..j]).into_owned();
        if name.len() >= NAME_MAX {
            alerts.push((ALERT_TYPE::at_warning, format!("Sentence {name} longer than {} letters\n", NAME_MAX - 1)));
        }
        t.names.push(name);

        // A trailing number makes it part of the group named by the rest.
        j -= 1;
        if j <= i || !at(j).is_ascii_digit() {
            continue;
        }
        while j > i && at(j).is_ascii_digit() {
            j -= 1;
        }
        if j <= i {
            continue;
        }
        let group = &line[i..=j];
        if group != prev_group {
            if t.groups.len() >= GROUPS_MAX {
                alerts.push((ALERT_TYPE::at_error, "Too many sentence groups in sentences.txt!\n".to_owned()));
                break;
            }
            t.groups.push(Group { name: String::from_utf8_lossy(group).into_owned(), count: 1, lru: [0; LRU_MAX] });
            prev_group = group;
        } else if let Some(g) = t.groups.last_mut() {
            g.count += 1;
        }
    }
    (t, alerts)
}

/// `USENTENCEG_InitLRU` (0xe00c8): the group's first `count` ordinals, shuffled.
fn init_lru(lru: &mut [u8; LRU_MAX], count: i32, mut random: impl FnMut(i32, i32) -> i32) {
    let count = count.min(LRU_MAX as i32);
    if count <= 0 {
        return;
    }
    for (i, l) in lru.iter_mut().take(count as usize).enumerate() {
        *l = i as u8;
    }
    for _ in 0..count * 4 {
        let j = random(0, count - 1) as usize;
        let k = random(0, count - 1) as usize;
        lru.swap(j, k);
    }
}

/// `SENTENCEG_Init` (0xe067c): loads `sound/sentences.txt` once. Called from the world's
/// precache.
pub fn init() {
    if TABLE.0.borrow().is_some() {
        return;
    }
    let eng = engine();
    let Some(file) = eng.load_file("sound/sentences.txt") else { return };
    let (mut t, alerts) = parse(&file);
    for (kind, msg) in alerts {
        eng.alert(kind, &msg);
    }
    for g in &mut t.groups {
        init_lru(&mut g.lru, g.count, |lo, hi| eng.random_long(lo, hi));
    }
    *TABLE.0.borrow_mut() = Some(t);
}

/// `SENTENCEG_Lookup` (0xe09e0): the index of sentence `!NAME` (case-insensitive), if loaded.
pub fn lookup(sample: &CStr) -> Option<usize> {
    let name = sample.to_bytes().strip_prefix(b"!")?;
    TABLE.0.borrow().as_ref()?.names.iter().position(|n| n.as_bytes().eq_ignore_ascii_case(name))
}

/// The sample to hand the engine: `!NAME` becomes `!<index>`, anything else is passed through.
/// `None` means a sentence that isn't in sentences.txt, which TS doesn't play.
pub fn resolve(sample: &CStr) -> Option<Cow<'_, CStr>> {
    if sample.to_bytes().first() != Some(&b'!') {
        return Some(Cow::Borrowed(sample));
    }
    let i = lookup(sample)?;
    CString::new(format!("!{i}")).ok().map(Cow::Owned)
}

/// `USENTENCEG_PickSequential` (0xe01d0): sentence `ipick` of the group (clamped to the last)
/// and the ordinal to ask for next time. Past the end it keeps repeating the last sentence; the
/// callers notice because the returned ordinal stops changing.
fn pick_sequential(t: &Table, group: usize, ipick: i32, reset: bool) -> Option<(String, i32)> {
    let g = t.groups.get(group)?;
    let count = (g.count as u8) as i32;
    if count == 0 {
        return None;
    }
    let ipick = ipick.min(count - 1);
    let name = format!("!{}{}", g.name, ipick);
    // (The SDK's `ipick >= count` branch is unreachable after the clamp.)
    let _ = reset;
    Some((name, ipick + 1))
}

/// `USENTENCEG_Pick` (0xe0298): a random sentence of the group, least recently used first.
///
/// Deviation: for groups over 32 sentences the C code reads past the 32-entry LRU array into the
/// next group's record; here only the 32 LRU entries are used.
fn pick(t: &mut Table, group: usize, mut random: impl FnMut(i32, i32) -> i32) -> Option<(String, i32)> {
    let g = t.groups.get_mut(group)?;
    let count = ((g.count as u8) as usize).min(LRU_MAX);
    if count == 0 {
        return None;
    }
    loop {
        if let Some(slot) = g.lru[..count].iter_mut().find(|l| **l != 0xff) {
            let ipick = *slot;
            *slot = 0xff;
            return Some((format!("!{}{}", g.name, ipick), ipick as i32));
        }
        init_lru(&mut g.lru, g.count, &mut random);
    }
}

/// `SENTENCEG_PlaySequentialSz` (0xe0554): plays the next sentence of group `name` on
/// `CHAN_VOICE` and returns the ordinal to pass next time (-1 if the group doesn't exist).
#[allow(clippy::too_many_arguments)]
pub fn play_sequential(e: *mut edict_t, name: &CStr, vol: f32, attn: f32, flags: i32, pitch: i32, ipick: i32, reset: bool) -> i32 {
    let picked = {
        let table = TABLE.0.borrow();
        let Some(t) = table.as_ref() else { return -1 };
        // SENTENCEG_GetIndex (0xe03b8): exact match.
        let Some(gi) = t.groups.iter().position(|g| g.name.as_bytes() == name.to_bytes()) else { return -1 };
        pick_sequential(t, gi, ipick, reset)
    };
    let Some((sentence, next)) = picked else { return -1 };
    if let Ok(s) = CString::new(sentence) {
        emit_sound_slow(e, CHAN_VOICE, &s, vol, attn, flags, pitch, 0.5);
    }
    next
}

/// `SENTENCEG_PlayRndSz` (0xe04a8): plays a random sentence of group `name` on `CHAN_VOICE`;
/// returns its ordinal (-1 if the group doesn't exist).
pub fn play_random(e: *mut edict_t, name: &CStr, vol: f32, attn: f32, flags: i32, pitch: i32) -> i32 {
    let eng = engine();
    let picked = {
        let mut table = TABLE.0.borrow_mut();
        let Some(t) = table.as_mut() else { return -1 };
        let Some(gi) = t.groups.iter().position(|g| g.name.as_bytes() == name.to_bytes()) else {
            drop(table);
            eng.alert(ALERT_TYPE::at_console, &format!("No such sentence group {}\n", name.to_string_lossy()));
            return -1;
        };
        pick(t, gi, |lo, hi| eng.random_long(lo, hi))
    };
    let Some((sentence, ipick)) = picked else { return -1 };
    if let Ok(s) = CString::new(sentence) {
        emit_sound_slow(e, CHAN_VOICE, &s, vol, attn, flags, pitch, 0.5);
    }
    ipick
}

/// `UTIL_EmitAmbientSound` (0xec7e8): `pfnEmitAmbientSound` with `!NAME` resolved.
#[allow(clippy::too_many_arguments)]
pub fn emit_ambient(e: *mut edict_t, origin: [f32; 3], sample: &CStr, vol: f32, attn: f32, flags: i32, pitch: i32) {
    if let Some(s) = resolve(sample) {
        engine().emit_ambient_sound(e, origin, &s, vol, attn, flags, pitch);
    }
}

/// `CBasePlayer::SetSuitUpdate` (0xda410) as TS reaches it: it ignores players without the HEV
/// suit bit, and then returns because `g_pGameRules->IsMultiplayer()` is always true in TS
/// (`CHalfLifeMultiplay::IsMultiplayer`, 0xcf330). Suit sentences (the battery's `!HEV_xP`, the
/// damage reports) are therefore never queued, so the suit queue and `CheckSuitUpdate` aren't
/// ported.
#[allow(clippy::needless_return)] // keeps TS's suit-bit early return ahead of the IsMultiplayer() one
pub fn set_suit_update(player: *mut edict_t, _name: &CStr, _group: bool, _no_repeat: i32) {
    if player.is_null() || player.v().weapons >= 0 {
        return;
    }
    // IsMultiplayer() == 1: "play no suit sounds in multiplayer".
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &[u8] = b"// comment line\r\n\
        \r\n\
        HG_GREN0 hgrunt/clik grenade!\r\n\
        HG_GREN1 hgrunt/clik take! cover!\r\n\
        HG_GREN2 hgrunt/clik oh!\n\
        \x20\x20NA0 vox/access denied\n\
        NA1 vox/access is denied\n\
        NONUMBER vox/hello\n\
        HEV_DMG4 fvox/minor_fracture\n\
        LONELY\n\
        1BAD starts with a digit\n\
        HEV_DMG5 fvox/major_fracture";

    fn table() -> Table {
        parse(FILE).0
    }

    #[test]
    fn parses_names_in_file_order() {
        let t = table();
        assert_eq!(t.names, ["HG_GREN0", "HG_GREN1", "HG_GREN2", "NA0", "NA1", "NONUMBER", "HEV_DMG4", "HEV_DMG5"]);
    }

    #[test]
    fn groups_consecutive_numbered_names() {
        let t = table();
        let g: Vec<(&str, i32)> = t.groups.iter().map(|g| (g.name.as_str(), g.count)).collect();
        assert_eq!(g, [("HG_GREN", 3), ("NA", 2), ("HEV_DMG", 2)]);
    }

    #[test]
    fn warns_about_long_names() {
        let (t, alerts) = parse(b"ABCDEFGHIJKLMNOPQ1 x\n");
        assert_eq!(t.names.len(), 1);
        assert_eq!(alerts.len(), 1);
    }

    #[test]
    fn memfgets_splits_long_lines() {
        let buf = [b'A'; 600];
        let mut pos = 0;
        assert_eq!(memfgets(&buf, &mut pos, 511).map(<[u8]>::len), Some(510));
        assert_eq!(memfgets(&buf, &mut pos, 511).map(<[u8]>::len), Some(90));
        assert_eq!(memfgets(&buf, &mut pos, 511), None);
    }

    #[test]
    fn sequential_pick_clamps_to_last() {
        let t = table();
        assert_eq!(pick_sequential(&t, 1, 0, false), Some(("!NA0".to_owned(), 1)));
        assert_eq!(pick_sequential(&t, 1, 1, false), Some(("!NA1".to_owned(), 2)));
        // Past the end: the last sentence again, and the same "next" (PlayLockSounds' EOF test).
        assert_eq!(pick_sequential(&t, 1, 2, false), Some(("!NA1".to_owned(), 2)));
    }

    #[test]
    fn random_pick_uses_each_once_per_round() {
        let mut t = table();
        let mut seq = 0;
        let mut rng = |lo: i32, hi: i32| {
            seq += 1;
            lo + seq % (hi - lo + 1)
        };
        let g = &mut t.groups[0];
        init_lru(&mut g.lru, g.count, &mut rng);
        let mut seen: Vec<i32> = (0..3).map(|_| pick(&mut t, 0, &mut rng).unwrap().1).collect();
        seen.sort();
        assert_eq!(seen, [0, 1, 2]);
        // The LRU refills once exhausted.
        assert!(pick(&mut t, 0, &mut rng).is_some());
    }
}
