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

fn parse(data: &[u8]) -> Table {
    let mut names = Vec::new();
    let mut types = Vec::new();
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
    fn global_table() {
        load(b"m METALFLOOR\nd grass_64\nt tile1\n");
        assert_eq!(find(b"metalfloor"), b'M');
        assert_eq!(find(b"GRASS_64"), b'D');
        assert_eq!(find(b"tile1"), b'T');
        assert_eq!(find(b"nothing"), b'C');
    }
}
