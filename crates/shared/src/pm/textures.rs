//! Material types from `sound/materials.txt` (`PM_InitTextureTypes` 0xfc050,
//! `PM_SortTextures` 0xfbfc8, `PM_FindTextureType` 0xfc28c).
//!
//! TS differences from the SDK: up to 1024 entries (SDK 512) and 15 significant characters per
//! name (SDK 12). The shipped TS file has ~1140 entries, so the tail is ignored, as in TS.
//! A ruleset picks its limits ([`TextureLimits`], `PmRules::texture_limits`); the file is read
//! once and a table is built for each set of limits asked for.

use std::cmp::Ordering;
use std::sync::{Mutex, OnceLock};

/// `CHAR_TEX_CONCRETE`: the default material.
pub const CHAR_TEX_CONCRETE: u8 = b'C';

/// Stored name length (`char[16]`), the largest slot any build uses.
const NAME_LEN: usize = 16;
/// TS 3.0's `strncasecmp` length (also `TEXTURETYPE_Find`'s).
const CMP_LEN: usize = 15;

/// The movement's material table limits (`PM_InitTextureTypes`, `PM_FindTextureType`,
/// `PM_CatagorizeTextureType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextureLimits {
    /// Entries read from the file; the rest is ignored.
    pub max_entries: usize,
    /// Characters kept of a name, compared by the lookup (`strnicmp` length), and the index at
    /// which `PM_CatagorizeTextureType` terminates the traced texture name.
    pub name_chars: usize,
}

impl TextureLimits {
    /// 1024 entries in 16-byte slots, 15 characters: TS 3.0's (`.so` 0xfc050 / 0xfc28c, `mp.dll`
    /// 0x10036630 / 0x100368d0), which [`load`]'s count uses; also its game-side table's
    /// (`TEXTURETYPE_LoadStandardMaterials`).
    pub const LARGE: Self = Self { max_entries: 1024, name_chars: CMP_LEN };
    /// The SDK's (`CTEXTURESMAX` 512, `CBTEXTURENAMEMAX` 13): 512 entries in 13-byte slots, 12
    /// characters. TS 2.1's movement and game-side tables (`TEXTURETYPE_Init` FUN_10036580:
    /// `count < 0x200`, `j = min(j, i + 13)` from the type letter).
    pub const SDK: Self = Self { max_entries: 512, name_chars: 12 };
}

struct Table {
    names: Vec<[u8; NAME_LEN]>,
    types: Vec<u8>,
    limits: TextureLimits,
}

/// The contents of `sound/materials.txt`, read once per library as TS does.
static DATA: OnceLock<Vec<u8>> = OnceLock::new();
/// The tables built from [`DATA`], one per set of limits (at most a couple, kept for the
/// process).
static TABLES: Mutex<Vec<&'static Table>> = Mutex::new(Vec::new());

/// C `isspace` in the "C" locale.
fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r')
}

/// `strncasecmp` on NUL-terminated byte strings.
fn strncasecmp(a: &[u8], b: &[u8], n: usize) -> Ordering {
    for i in 0..n {
        let ca = a.get(i).copied().unwrap_or(0).to_ascii_lowercase();
        let cb = b.get(i).copied().unwrap_or(0).to_ascii_lowercase();
        if ca != cb {
            return ca.cmp(&cb);
        }
        if ca == 0 {
            break;
        }
    }
    Ordering::Equal
}

/// Splits like the engine's `memfgets`: each line keeps its `\n`; a last line without one is
/// returned without it.
fn lines(data: &[u8]) -> impl Iterator<Item = &[u8]> {
    data.split_inclusive(|&c| c == b'\n')
}

/// The line parser shared by `PM_InitTextureTypes` and `TEXTURETYPE_LoadStandardMaterials`/
/// `TEXTURETYPE_LoadMapMaterials`: appends to the table until it holds `limits.max_entries`
/// entries, each name cut to `limits.name_chars` (the SDK's `PM_InitTextureTypes`: 512,
/// `j = min(j, i + 12)`).
fn parse_into(data: &[u8], names: &mut Vec<[u8; NAME_LEN]>, types: &mut Vec<u8>, limits: TextureLimits) {
    for raw in lines(data) {
        if names.len() >= limits.max_entries {
            break;
        }
        // memfgets copies at most 511 characters.
        let line = &raw[..raw.len().min(511)];
        let at = |i: usize| line.get(i).copied().unwrap_or(0);
        let mut i = 0;
        while at(i) != 0 && is_space(at(i)) {
            i += 1;
        }
        let c = at(i);
        if c == 0 || c == b'/' || !c.is_ascii_alphabetic() {
            continue;
        }
        let kind = c.to_ascii_uppercase();
        i += 1;
        while at(i) != 0 && is_space(at(i)) {
            i += 1;
        }
        if at(i) == 0 {
            continue;
        }
        let mut j = i;
        while at(j) != 0 && !is_space(at(j)) {
            j += 1;
        }
        if at(j) == 0 {
            continue; // the name must be followed by whitespace (the line's newline)
        }
        let j = j.min(i + limits.name_chars);
        let mut name = [0u8; NAME_LEN];
        name[..j - i].copy_from_slice(&line[i..j]);
        names.push(name);
        types.push(kind);
    }
}

fn parse(data: &[u8], limits: TextureLimits) -> Table {
    let mut names = Vec::new();
    let mut types = Vec::new();
    parse_into(data, &mut names, &mut types, limits);
    // PM_SortTextures: for each i, swap with every later j that sorts before it. (The SDK's
    // `stricmp` orders the NUL-terminated names as this does.)
    let n = names.len();
    for i in 0..n {
        for j in i + 1..n {
            if strncasecmp(&names[i], &names[j], NAME_LEN) == Ordering::Greater {
                names.swap(i, j);
                types.swap(i, j);
            }
        }
    }
    Table { names, types, limits }
}

/// The table for `limits`, built from the file on first use; `None` before [`load`].
fn table(limits: TextureLimits) -> Option<&'static Table> {
    let data = DATA.get()?;
    let mut tables = TABLES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(t) = tables.iter().find(|t| t.limits == limits) {
        return Some(t);
    }
    let t: &'static Table = Box::leak(Box::new(parse(data, limits)));
    tables.push(t);
    Some(t)
}

/// Keeps the contents of `sound/materials.txt`. Only the first call has an effect (TS loads it
/// once per library). Returns the number of entries in TS 3.0's table.
pub fn load(data: &[u8]) -> usize {
    DATA.get_or_init(|| data.to_vec());
    table(TextureLimits::LARGE).map_or(0, |t| t.names.len())
}

pub fn is_loaded() -> bool {
    DATA.get().is_some()
}

/// [`find_in`] TS 3.0's table.
pub fn find(name: &[u8]) -> u8 {
    find_in(name, TextureLimits::LARGE)
}

/// Binary search, as `PM_FindTextureType`, comparing `limits.name_chars` characters (the SDK:
/// `strnicmp(name, slot, 12)`). Unknown names (or no table) are concrete.
pub fn find_in(name: &[u8], limits: TextureLimits) -> u8 {
    let Some(t) = table(limits) else { return CHAR_TEX_CONCRETE };
    let (mut lo, mut hi) = (0i32, t.names.len() as i32 - 1);
    while lo <= hi {
        let mid = (lo + hi) / 2;
        match strncasecmp(name, &t.names[mid as usize], limits.name_chars) {
            Ordering::Equal => return t.types[mid as usize],
            Ordering::Greater => lo = mid + 1,
            Ordering::Less => hi = mid - 1,
        }
    }
    CHAR_TEX_CONCRETE
}

/// The game code's own material table (`TEXTURETYPE_*`, server 0xe0e14..0xe1330), which
/// `TEXTURETYPE_PlaySound` reads: `sound/materials.txt`, followed by `sound/<map>_materials.txt`
/// where the ruleset reads map files, unsorted, searched linearly (the first match wins). The
/// size and name length are the ruleset's [`TextureLimits`]; each set of rules has its own table.
pub mod texturetype {
    use super::*;
    use std::sync::Mutex;

    /// What a table is built with: the limits and whether the map's file is read.
    type Key = (TextureLimits, bool);

    /// A table: its key, `fTextureTypeInit`, the names and the types.
    type Built = (Key, bool, Vec<[u8; NAME_LEN]>, Vec<u8>);

    /// One table per [`Key`].
    static TABLES: Mutex<Vec<Built>> = Mutex::new(Vec::new());

    /// `TEXTURETYPE_Init` (0xe1244), from `CWorld::Precache`. Only loads while no file has been
    /// found yet, so the map file of the first map that has either file sticks for the whole
    /// process, as in TS. `load` reads a game file. With `map_files` false (the SDK's, TS 2.1's
    /// FUN_10036580) only `sound/materials.txt` is read.
    pub fn init(limits: TextureLimits, map_files: bool, mapname: &str, mut load: impl FnMut(&str) -> Option<Vec<u8>>) {
        let mut tables = TABLES.lock().unwrap_or_else(|e| e.into_inner());
        let key = (limits, map_files);
        let i = match tables.iter().position(|t| t.0 == key) {
            Some(i) => i,
            None => {
                tables.push((key, false, Vec::new(), Vec::new()));
                tables.len() - 1
            }
        };
        let (_, done, names, types) = &mut tables[i];
        if *done {
            return;
        }
        names.clear();
        types.clear();
        let standard = load("sound/materials.txt").map(|d| parse_into(&d, names, types, limits)).is_some();
        let map = map_files && load(&format!("sound/{mapname}_materials.txt")).map(|d| parse_into(&d, names, types, limits)).is_some();
        *done = standard || map;
    }

    /// `TEXTURETYPE_Find` (0xe12c4; TS 2.1 0x10036760): the first entry whose name matches
    /// `limits.name_chars` characters, `CHAR_TEX_CONCRETE` when the name isn't listed.
    pub fn find(name: &[u8], limits: TextureLimits, map_files: bool) -> u8 {
        let tables = TABLES.lock().unwrap_or_else(|e| e.into_inner());
        let Some((_, _, names, types)) = tables.iter().find(|t| t.0 == (limits, map_files)) else { return CHAR_TEX_CONCRETE };
        names.iter().position(|n| strncasecmp(name, n, limits.name_chars) == Ordering::Equal).map_or(CHAR_TEX_CONCRETE, |i| types[i])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_lookup() {
        let data = b"// comment\n  m ae_roof04\nw BOARD03_128X128\r\nd thisnameiswaytoolongforit\n1 bad\ny last_no_newline";
        let t = parse(data, TextureLimits::LARGE);
        assert_eq!(t.names.len(), 3);
        // Sorted case-insensitively.
        assert_eq!(&t.names[0][..9], b"ae_roof04");
        assert_eq!(&t.names[1][..15], b"BOARD03_128X128");
        assert_eq!(&t.names[2][..15], b"thisnameiswayto");
        assert_eq!(t.names[2][15], 0);
        // The SDK's limits: 12 characters.
        let t = parse(data, TextureLimits::SDK);
        assert_eq!(&t.names[1][..13], b"BOARD03_128X\0");
        assert_eq!(&t.names[2][..13], b"thisnameiswa\0");
    }

    #[test]
    fn sdk_limits_keep_512_entries() {
        let data: Vec<u8> = (0..1100).flat_map(|i| format!("m tex{i:04}\n").into_bytes()).collect();
        assert_eq!(parse(&data, TextureLimits::SDK).names.len(), 512);
        assert_eq!(parse(&data, TextureLimits::LARGE).names.len(), 1024);
    }

    #[test]
    fn texturetype_table_is_unsorted_first_match() {
        let files = |path: &str| match path {
            "sound/materials.txt" => Some(b"m crate\nw crate\nt tile_longname\n".to_vec()),
            "sound/dojo_materials.txt" => Some(b"g grid\n".to_vec()),
            _ => None,
        };
        let large = TextureLimits::LARGE;
        texturetype::init(large, true, "dojo", files);
        assert_eq!(texturetype::find(b"CRATE", large, true), b'M');
        assert_eq!(texturetype::find(b"grid", large, true), b'G');
        assert_eq!(texturetype::find(b"other", large, true), b'C');
        assert_eq!(texturetype::find(b"tile_longnamX", large, true), b'C', "the 13th character differs");
        // The SDK's table (TS 2.1): no map file, 12 characters.
        let sdk = TextureLimits::SDK;
        texturetype::init(sdk, false, "dojo", files);
        assert_eq!(texturetype::find(b"crate", sdk, false), b'M');
        assert_eq!(texturetype::find(b"grid", sdk, false), b'C', "no map file");
        assert_eq!(texturetype::find(b"tile_longnamX", sdk, false), b'T', "12 characters match");
    }

    #[test]
    fn texturetype_sdk_table_keeps_512() {
        let data: Vec<u8> = (0..600).flat_map(|i| format!("m tex{i:04}\n").into_bytes()).collect();
        // (A key of its own: the tables are per process.)
        let key = TextureLimits::SDK;
        texturetype::init(key, true, "x", |p| (p == "sound/materials.txt").then(|| data.clone()));
        assert_eq!(texturetype::find(b"tex0511", key, true), b'M');
        assert_eq!(texturetype::find(b"tex0512", key, true), b'C');
    }

    #[test]
    fn global_table() {
        load(b"m METALFLOOR\nd grass_64\nt tile1\nw woodplanks_long_a\n");
        assert_eq!(find(b"metalfloor"), b'M');
        assert_eq!(find(b"GRASS_64"), b'D');
        assert_eq!(find(b"tile1"), b'T');
        assert_eq!(find(b"nothing"), b'C');
        // 15 characters (3.0) against the SDK's 12 (`strnicmp(.., 12)`): a name that differs only
        // after the 12th character matches with the SDK's limits.
        assert_eq!(find(b"woodplanks_long_b"), b'W', "same first 15");
        assert_eq!(find(b"woodplanks_lXXX"), b'C');
        assert_eq!(find_in(b"woodplanks_lXXX", TextureLimits::SDK), b'W');
        assert_eq!(find_in(b"metalfloor", TextureLimits::SDK), b'M');
    }
}
