//! Guards for engine calls that stop the server (port rule: the game must never crash, hang or
//! stop the engine, even where TS does).
//!
//! The Steam engines (HLDS, ReHLDS) `Host_Error` or `Sys_Error` on several inputs that FWGS only
//! warns about: an empty precache name ("Bad string"), a sentence name given to
//! `PRECACHE_SOUND`, a precache after the level has loaded ("Precache can only be done in spawn
//! functions"), a precache table overflow, `SET_MODEL`/`MODEL_INDEX` of a model that isn't
//! precached, a sound volume, attenuation, channel or pitch out of range, backwards
//! `SET_SIZE` bounds and user messages that are too long or don't match their registered size.
//! On those engines the wrappers in `server.rs` skip the call (or clamp, as the engines do further
//! down) and log the error instead; on FWGS they make the call as TS would, because FWGS handles it
//! itself. Calls that stop every engine (light styles past 63, no free edicts, a bad
//! `WRITE_ENTITY` number, a `*n` brush model the map doesn't have) are guarded everywhere.
//!
//! This module holds the bookkeeping: the names the game precached this level (so a call the
//! engine would refuse can be recognised before it is made), the map's brush-model count and the
//! registered user-message sizes. The client's guards are in `cl_guard`.

use std::collections::{HashMap, HashSet};
use std::ffi::CStr;
use std::sync::Mutex;

/// The Steam engines' table sizes (`qlimits.h`): `MAX_MODELS`, `MAX_SOUNDS`, `MAX_GENERIC`
/// (512 each) and `MAX_EVENTS` (256). FWGS's are larger, so these never refuse a call FWGS
/// would make.
pub const MAX_MODELS: i32 = 512;
pub const MAX_SOUNDS: i32 = 512;
pub const MAX_GENERIC: usize = 512;
pub const MAX_EVENTS: i32 = 256;
/// `MAX_LIGHTSTYLES` (64 in the Steam engines and FWGS 0.21; FWGS has 256 now). FWGS
/// `Host_Error`s past it and the Steam engines write past their `lightstyles[64]` array.
pub const MAX_LIGHTSTYLES: i32 = 64;
/// `MAX_USER_MSG_DATA`: a variable-size user message over this is a `Host_Error` (ReHLDS
/// `Mesage_CheckUserMessageLength`).
pub const MAX_USER_MSG_DATA: usize = 192;
/// `gMsgData[512]`: the Steam engines' buffer for one message; overflowing it is a `Sys_Error`
/// in `PF_MessageEnd_I`.
pub const MAX_MSG_DATA: usize = 512;

/// Which precache table a name goes in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Model,
    Sound,
    Generic,
    Event,
}

/// `PR_IsEmptyString` (ReHLDS `pr_cmds.cpp`): `s[0] < ' '` with a signed `char`, so an empty
/// name, a control character or a byte of 0x80 and above is a "Bad string".
pub fn bad_string(name: &[u8]) -> bool {
    name.first().is_none_or(|&c| (c as i8) < b' ' as i8)
}

/// The names one table holds this level, as the Steam engines compare them.
#[derive(Default)]
pub struct Table {
    /// Every exact name passed to the engine (ReHLDS with `REHLDS_FIXES` compares with
    /// `strcmp`, so a case variant takes a new slot while the level loads).
    exact: HashSet<Vec<u8>>,
    /// Lower-cased name (without a model's optional `!`) → the first spelling precached, for the
    /// `stricmp` engines and for calls after loading.
    lower: HashMap<Vec<u8>, &'static CStr>,
    /// Highest index the engine returned (the tables fill in order, after the engine's own
    /// entries such as the brush models).
    high: i32,
    /// Distinct exact names (`MAX_GENERIC` counts entries, not indices).
    count: usize,
}

/// Strips a model's leading `!` (an optional model) for lookups; `name` stays `'static`.
pub fn lookup_name(kind: Kind, name: &'static CStr) -> &'static CStr {
    let b = name.to_bytes_with_nul();
    if kind == Kind::Model && b[0] == b'!' {
        // The tail of a NUL-terminated string is NUL-terminated.
        CStr::from_bytes_with_nul(&b[1..]).unwrap_or(name)
    } else {
        name
    }
}

impl Table {
    fn full(&self, kind: Kind) -> bool {
        match kind {
            Kind::Model => self.high >= MAX_MODELS - 1,
            Kind::Sound => self.high >= MAX_SOUNDS - 1,
            Kind::Event => self.high >= MAX_EVENTS - 1,
            Kind::Generic => self.count >= MAX_GENERIC,
        }
    }

    /// The precached spelling of `name` (exact first, then case-insensitive), if any.
    pub fn find(&self, name: &CStr) -> Option<&'static CStr> {
        self.lower.get(&name.to_bytes().to_ascii_lowercase()).copied()
    }

    /// `name` if it was precached as spelt, else its precached spelling, else `None`.
    pub fn resolve<'a>(&self, name: &'a CStr) -> Option<&'a CStr> {
        if self.exact.contains(name.to_bytes()) { Some(name) } else { self.find(name) }
    }

    /// What a Steam engine's `PF_precache_*_I` would do with `name`: `Ok(name to pass)` when the
    /// call is safe, `Err(reason)` when it would stop the server.
    pub fn decide(&self, kind: Kind, name: &'static CStr, loading: bool) -> Result<&'static CStr, &'static str> {
        let b = name.to_bytes();
        if bad_string(b) {
            return Err("Bad string");
        }
        if kind == Kind::Sound && b[0] == b'!' {
            return Err("do not precache sentence names");
        }
        if self.exact.contains(b) {
            return Ok(name);
        }
        match self.find(lookup_name(kind, name)) {
            // A case variant of a name in the table: the `stricmp` engines answer with that
            // entry, so it is passed in the spelling the table has (ReHLDS with `REHLDS_FIXES`
            // compares with `strcmp`, and would refuse the variant after loading, take a new slot
            // while loading, and look for the variant's file on a case-sensitive file system).
            Some(c) => Ok(c),
            None if !loading => Err("Precache can only be done in spawn functions"),
            None if self.full(kind) => Err("the item count is over the limit"),
            None => Ok(name),
        }
    }

    /// Records a name the engine accepted at `index`.
    pub fn record(&mut self, kind: Kind, name: &'static CStr, index: i32) {
        if self.exact.insert(name.to_bytes().to_vec()) {
            self.count += 1;
        }
        let key = lookup_name(kind, name);
        self.lower.entry(key.to_bytes().to_ascii_lowercase()).or_insert(key);
        self.high = self.high.max(index);
    }
}

/// The guard state for the current level.
#[derive(Default)]
pub struct State {
    /// True from the world's spawn until `ServerActivate` returns: the engine's `ss_loading`,
    /// when the Steam engines accept new precaches.
    pub loading: bool,
    pub models: Table,
    pub sounds: Table,
    pub generic: Table,
    pub events: Table,
    /// User-message type → registered size (-1 variable); kept across levels like the engine's.
    pub user_msgs: HashMap<i32, i32>,
    /// The level's map name and its BSP's brush-model count (`None`: the header couldn't be
    /// read), read once per level by `Engine::brush_model_ok`.
    pub brush: Option<(Vec<u8>, Option<i32>)>,
}

impl State {
    pub fn table(&mut self, kind: Kind) -> &mut Table {
        match kind {
            Kind::Model => &mut self.models,
            Kind::Sound => &mut self.sounds,
            Kind::Generic => &mut self.generic,
            Kind::Event => &mut self.events,
        }
    }
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

/// Runs `f` on the guard state (created loading, as the engine is before the first level).
pub fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(|| State { loading: true, ..State::default() }))
}

/// A new level starts loading (the world is spawned or restored, or the server deactivated):
/// the engine has emptied its precache tables and accepts new names again.
pub fn level_loading() {
    with(|s| {
        s.loading = true;
        s.models = Table::default();
        s.sounds = Table::default();
        s.generic = Table::default();
        s.events = Table::default();
        s.brush = None;
    });
}

/// `ServerActivate` has returned: the engine is `ss_active` and refuses new precaches.
pub fn level_active() {
    with(|s| s.loading = false);
}

/// `LUMP_MODELS` in a BSP version 29/30 header (`version`, then 15 `{fileofs, filelen}` lumps).
const LUMP_MODELS: usize = 14;
/// The bytes of a BSP header up to the end of the models lump entry.
pub const BSP_HEADER_MODELS_END: usize = 4 + (LUMP_MODELS + 1) * 8;
/// `sizeof(dmodel_t)` in BSP versions 29 and 30.
const DMODEL_SIZE: i32 = 64;

/// The number of models in a BSP's models lump (the world and its brush models), as
/// `Mod_LoadSubmodels` counts them (`filelen / sizeof(dmodel_t)`). `None` for a header that isn't
/// version 29 or 30 (FWGS also loads BSP2, whose header differs), so the caller doesn't guard.
pub fn bsp_model_count(header: &[u8]) -> Option<i32> {
    let rd = |o: usize| header.get(o..o + 4).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let version = rd(0)?;
    if version != 29 && version != 30 {
        return None;
    }
    let len = rd(4 + LUMP_MODELS * 8 + 4)?;
    (len >= 0).then_some(len / DMODEL_SIZE)
}

/// A brush-model name (`*n`), which the engines look up in the map's model table, not on disk.
pub fn is_brush_name(name: &[u8]) -> bool {
    name.first() == Some(&b'*')
}

/// Whether `name` is in the model table a map with `count` models gets: `SV_SpawnServer` (ReHLDS
/// and FWGS) adds `"*%i"` for 1..count, so any other number or spelling (`*0`, `*01`, `*+1`)
/// isn't there.
pub fn brush_model_known(name: &[u8], count: i32) -> bool {
    let Some(digits) = name.strip_prefix(b"*").and_then(|d| std::str::from_utf8(d).ok()) else { return false };
    digits.parse::<i32>().is_ok_and(|n| n >= 1 && n < count && digits == n.to_string())
}

/// The engines' `ED_Alloc` reuse test for a free edict: freed in the level's first 2 s, or more
/// than 0.5 s ago.
pub fn reusable(free: bool, freetime: f32, time: f32) -> bool {
    free && (freetime < 2.0 || time - freetime > 0.5)
}

/// Clamps `v` to `0..=hi` as `SV_BuildSoundMsg` does; NaN passes the engines' range checks, so
/// it is kept.
pub fn clamp_nan_kept(v: f32, hi: f32) -> f32 {
    if v < 0.0 {
        0.0
    } else if v > hi {
        hi
    } else {
        v
    }
}

/// Why a Steam engine would stop on a message of `size` bytes of type `msg_type`, if it would:
/// a user message registered with a fixed size must match it, a variable one may hold 192 bytes,
/// and nothing may exceed the 512-byte message buffer.
pub fn message_size_error(user_size: Option<i32>, size: usize) -> Option<String> {
    if size > MAX_MSG_DATA {
        return Some(format!("{size} bytes overflow the {MAX_MSG_DATA}-byte message buffer"));
    }
    match user_size {
        Some(-1) if size > MAX_USER_MSG_DATA => Some(format!("{size} bytes, user message size limit is {MAX_USER_MSG_DATA} bytes")),
        Some(n) if n >= 0 && n as usize != size => Some(format!("{size} bytes written, expected {n}")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bad_strings_are_what_the_steam_engines_refuse() {
        assert!(bad_string(b""));
        assert!(bad_string(b"\x01a"));
        assert!(bad_string("\u{e9}.wav".as_bytes()));
        assert!(!bad_string(b" a"));
        assert!(!bad_string(b"models/a.mdl"));
    }

    #[test]
    fn precache_while_loading_then_only_known_names() {
        let mut t = Table::default();
        assert_eq!(t.decide(Kind::Model, c"", true), Err("Bad string"));
        assert_eq!(t.decide(Kind::Model, c"models/a.mdl", true), Ok(c"models/a.mdl"));
        t.record(Kind::Model, c"models/a.mdl", 40);
        // A case variant goes to the engine in the table's spelling.
        assert_eq!(t.decide(Kind::Model, c"Models/A.mdl", true), Ok(c"models/a.mdl"));
        // After loading: known names (in the precached spelling) only.
        assert_eq!(t.decide(Kind::Model, c"models/a.mdl", false), Ok(c"models/a.mdl"));
        assert_eq!(t.decide(Kind::Model, c"MODELS/A.MDL", false), Ok(c"models/a.mdl"));
        assert_eq!(t.decide(Kind::Model, c"models/b.mdl", false), Err("Precache can only be done in spawn functions"));
        assert_eq!(t.decide(Kind::Model, c"", false), Err("Bad string"));
    }

    #[test]
    fn optional_models_and_sentence_names() {
        let mut t = Table::default();
        assert_eq!(t.decide(Kind::Model, c"!models/opt.mdl", true), Ok(c"!models/opt.mdl"));
        t.record(Kind::Model, c"!models/opt.mdl", 5);
        assert_eq!(t.find(c"models/opt.mdl"), Some(c"models/opt.mdl"));
        assert_eq!(t.decide(Kind::Model, c"models/opt.mdl", false), Ok(c"models/opt.mdl"));
        assert_eq!(t.decide(Kind::Sound, c"!HEV_A0", true), Err("do not precache sentence names"));
    }

    #[test]
    fn full_tables_refuse_new_names_only() {
        let mut t = Table::default();
        t.record(Kind::Sound, c"a.wav", MAX_SOUNDS - 1);
        assert_eq!(t.decide(Kind::Sound, c"b.wav", true), Err("the item count is over the limit"));
        assert_eq!(t.decide(Kind::Sound, c"a.wav", true), Ok(c"a.wav"));
        assert_eq!(t.decide(Kind::Sound, c"A.WAV", true), Ok(c"a.wav"));
        let mut g = Table::default();
        for i in 0..MAX_GENERIC {
            g.record(Kind::Generic, crate::server::intern(&format!("g{i}.txt")), 0);
        }
        assert_eq!(g.decide(Kind::Generic, c"new.txt", true), Err("the item count is over the limit"));
        let mut e = Table::default();
        e.record(Kind::Event, c"events/a.sc", MAX_EVENTS - 2);
        assert_eq!(e.decide(Kind::Event, c"events/b.sc", true), Ok(c"events/b.sc"));
        e.record(Kind::Event, c"events/b.sc", MAX_EVENTS - 1);
        assert!(e.decide(Kind::Event, c"events/c.sc", true).is_err());
    }

    #[test]
    fn message_sizes() {
        assert_eq!(message_size_error(Some(-1), 192), None);
        assert!(message_size_error(Some(-1), 193).is_some());
        assert_eq!(message_size_error(Some(4), 4), None);
        assert!(message_size_error(Some(4), 3).is_some());
        assert_eq!(message_size_error(None, 512), None);
        assert!(message_size_error(None, 513).is_some());
    }

    #[test]
    fn resolve_prefers_the_exact_spelling() {
        let mut t = Table::default();
        t.record(Kind::Model, c"models/a.mdl", 3);
        assert_eq!(t.resolve(c"models/a.mdl"), Some(c"models/a.mdl"));
        assert_eq!(t.resolve(c"MODELS/a.mdl"), Some(c"models/a.mdl"));
        assert_eq!(t.resolve(c"models/b.mdl"), None);
    }

    #[test]
    fn brush_model_names() {
        let mut h = vec![0u8; BSP_HEADER_MODELS_END];
        h[..4].copy_from_slice(&30i32.to_le_bytes());
        h[4 + 14 * 8 + 4..][..4].copy_from_slice(&(5 * 64i32).to_le_bytes());
        assert_eq!(bsp_model_count(&h), Some(5));
        assert_eq!(bsp_model_count(&h[..BSP_HEADER_MODELS_END - 1]), None);
        h[..4].copy_from_slice(&31i32.to_le_bytes());
        assert_eq!(bsp_model_count(&h), None);
        h[..4].copy_from_slice(&29i32.to_le_bytes());
        h[4 + 14 * 8 + 4..][..4].copy_from_slice(&(-64i32).to_le_bytes());
        assert_eq!(bsp_model_count(&h), None);
        assert!(is_brush_name(b"*3") && !is_brush_name(b"models/a.mdl") && !is_brush_name(b""));
        // A map with 5 models has the world and *1..*4.
        assert!(brush_model_known(b"*1", 5) && brush_model_known(b"*4", 5));
        for bad in [&b"*0"[..], b"*5", b"*99", b"*", b"*01", b"*+1", b"*-1", b"*1x", b"* 1", b"1"] {
            assert!(!brush_model_known(bad, 5), "{}", String::from_utf8_lossy(bad));
        }
    }

    #[test]
    fn edict_reuse_and_sound_clamps() {
        assert!(reusable(true, 1.0, 100.0));
        assert!(reusable(true, 50.0, 50.6));
        assert!(!reusable(true, 50.0, 50.4));
        assert!(!reusable(false, 0.0, 100.0));
        assert_eq!(clamp_nan_kept(1.5, 1.0), 1.0);
        assert_eq!(clamp_nan_kept(-1.0, 4.0), 0.0);
        assert_eq!(clamp_nan_kept(0.5, 1.0), 0.5);
        assert!(clamp_nan_kept(f32::NAN, 1.0).is_nan());
    }
}
