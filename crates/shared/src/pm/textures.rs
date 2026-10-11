//! Material types from `sound/materials.txt` (`PM_InitTextureTypes` 0xfc050,
//! `PM_SortTextures` 0xfbfc8, `PM_FindTextureType` 0xfc28c).
//!
//! TS differences from the SDK: up to 1024 entries (SDK 512) and 15 significant characters per
//! name (SDK 12). The shipped TS file has ~1140 entries, so the tail is ignored, as in TS.

use std::cmp::Ordering;
use std::sync::OnceLock;

/// `CHAR_TEX_CONCRETE`: the default material.
pub const CHAR_TEX_CONCRETE: u8 = b'C';

const MAX_TEXTURES: usize = 1024;
/// Stored name length (`char[16]`), and the `strncasecmp` length used by the lookup.
const NAME_LEN: usize = 16;
const CMP_LEN: usize = 15;

struct Table {
    names: Vec<[u8; NAME_LEN]>,
    types: Vec<u8>,
}

static TABLE: OnceLock<Table> = OnceLock::new();

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
/// `TEXTURETYPE_LoadMapMaterials`: appends to the table until it holds 1024 entries.
fn parse_into(data: &[u8], names: &mut Vec<[u8; NAME_LEN]>, types: &mut Vec<u8>) {
    for raw in lines(data) {
        if names.len() >= MAX_TEXTURES {
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
        let j = j.min(i + CMP_LEN);
        let mut name = [0u8; NAME_LEN];
        name[..j - i].copy_from_slice(&line[i..j]);
        names.push(name);
        types.push(kind);
    }
}

fn parse(data: &[u8]) -> Table {
    let mut names = Vec::new();
    let mut types = Vec::new();
    parse_into(data, &mut names, &mut types);
    // PM_SortTextures: for each i, swap with every later j that sorts before it.
    let n = names.len();
    for i in 0..n {
        for j in i + 1..n {
            if strncasecmp(&names[i], &names[j], NAME_LEN) == Ordering::Greater {
                names.swap(i, j);
                types.swap(i, j);
            }
        }
    }
    Table { names, types }
}

/// Loads the table from the contents of `sound/materials.txt`. Only the first call has an
/// effect (TS loads it once per library). Returns the number of entries in the table.
pub fn load(data: &[u8]) -> usize {
    TABLE.get_or_init(|| parse(data)).names.len()
}

pub fn is_loaded() -> bool {
    TABLE.get().is_some()
}

/// Binary search, as `PM_FindTextureType`. Unknown names (or no table) are concrete.
pub fn find(name: &[u8]) -> u8 {
    let Some(t) = TABLE.get() else { return CHAR_TEX_CONCRETE };
    let (mut lo, mut hi) = (0i32, t.names.len() as i32 - 1);
    while lo <= hi {
        let mid = (lo + hi) / 2;
        match strncasecmp(name, &t.names[mid as usize], CMP_LEN) {
            Ordering::Equal => return t.types[mid as usize],
            Ordering::Greater => lo = mid + 1,
            Ordering::Less => hi = mid - 1,
        }
    }
    CHAR_TEX_CONCRETE
}

/// The game code's own material table (`TEXTURETYPE_*`, server 0xe0e14..0xe1330), which
/// `TEXTURETYPE_PlaySound` reads: `sound/materials.txt` followed by `sound/<map>_materials.txt`,
/// unsorted, searched linearly (the first match wins).
pub mod texturetype {
    use super::*;
    use std::sync::Mutex;

    /// The table and `fTextureTypeInit`.
    static TABLE: Mutex<(bool, Vec<[u8; NAME_LEN]>, Vec<u8>)> = Mutex::new((false, Vec::new(), Vec::new()));

    /// `TEXTURETYPE_Init` (0xe1244), from `CWorld::Precache`. Only loads while no file has been
    /// found yet, so the map file of the first map that has either file sticks for the whole
    /// process, as in TS. `load` reads a game file.
    pub fn init(mapname: &str, mut load: impl FnMut(&str) -> Option<Vec<u8>>) {
        let mut t = TABLE.lock().unwrap_or_else(|e| e.into_inner());
        if t.0 {
            return;
        }
        let (_, names, types) = &mut *t;
        names.clear();
        types.clear();
        let standard = load("sound/materials.txt").map(|d| parse_into(&d, names, types)).is_some();
        let map = load(&format!("sound/{mapname}_materials.txt")).map(|d| parse_into(&d, names, types)).is_some();
        t.0 = standard || map;
    }

    /// `TEXTURETYPE_Find` (0xe12c4): `CHAR_TEX_CONCRETE` when the name isn't listed.
    pub fn find(name: &[u8]) -> u8 {
        let t = TABLE.lock().unwrap_or_else(|e| e.into_inner());
        t.1.iter().position(|n| strncasecmp(name, n, CMP_LEN) == Ordering::Equal).map_or(CHAR_TEX_CONCRETE, |i| t.2[i])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_lookup() {
        let t = parse(b"// comment\n  m ae_roof04\nw BOARD03_128X128\r\nd thisnameiswaytoolongforit\n1 bad\ny last_no_newline");
        assert_eq!(t.names.len(), 3);
        // Sorted case-insensitively.
        assert_eq!(&t.names[0][..9], b"ae_roof04");
        assert_eq!(&t.names[1][..15], b"BOARD03_128X128");
        assert_eq!(&t.names[2][..15], b"thisnameiswayto");
        assert_eq!(t.names[2][15], 0);
    }

    #[test]
    fn texturetype_table_is_unsorted_first_match() {
        texturetype::init("dojo", |path| match path {
            "sound/materials.txt" => Some(b"m crate\nw crate\n".to_vec()),
            "sound/dojo_materials.txt" => Some(b"g grid\n".to_vec()),
            _ => None,
        });
        assert_eq!(texturetype::find(b"CRATE"), b'M');
        assert_eq!(texturetype::find(b"grid"), b'G');
        assert_eq!(texturetype::find(b"other"), b'C');
    }

    #[test]
    fn global_table() {
        load(b"m METALFLOOR\nd grass_64\nt tile1\n");
        assert_eq!(find(b"metalfloor"), b'M');
        assert_eq!(find(b"GRASS_64"), b'D');
        assert_eq!(find(b"tile1"), b'T');
        assert_eq!(find(b"nothing"), b'C');
    }
}
