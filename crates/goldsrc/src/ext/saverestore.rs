//! The engine's save/restore state (`gpGlobals->pSaveData`, `SAVERESTOREDATA`) as far as the game
//! touches it outside `pfnSave`/`pfnRestore`: `ParmsChangeLevel`'s level connection list and the
//! transition flags it sets in the entity table (SDK `CSaveRestoreBuffer::EntityIndex` /
//! `EntityFlagsSet`), plus `pfnEntitiesInPVS`.

use crate::edict::EdictPtr;
use crate::eng;
use crate::server::Engine;
use crate::sys::*;

/// `MAX_LEVEL_CONNECTIONS`: the size of `SAVERESTOREDATA::levelList`.
pub const MAX_LEVEL_CONNECTIONS: usize = 16;
/// `FENTTABLE_GLOBAL`: an entity with a `globalname` that isn't dormant.
pub const FENTTABLE_GLOBAL: i32 = 0x1000_0000;
/// `FENTTABLE_MOVEABLE`: an entity that can cross a transition (`FCAP_ACROSS_TRANSITION`).
pub const FENTTABLE_MOVEABLE: i32 = 0x2000_0000;

/// One `LEVELLIST` entry as the game fills it (`AddTransitionToList`).
#[derive(Clone, Debug, PartialEq)]
pub struct LevelConnection {
    pub map: Vec<u8>,
    pub landmark: Vec<u8>,
    pub pent_landmark: *mut edict_t,
    pub landmark_origin: [f32; 3],
}

/// `gpGlobals->pSaveData` while it is set (the engine sets it around a save, e.g. for a
/// `changelevel2`).
pub struct SaveData {
    p: *mut SAVERESTOREDATA,
}

/// Copies `src` into a `char[32]` field: at most 31 bytes and a NUL.
fn copy_name(dst: &mut [std::ffi::c_char; 32], src: &[u8]) {
    let n = src.len().min(dst.len() - 1);
    for (d, s) in dst.iter_mut().zip(&src[..n]) {
        *d = *s as std::ffi::c_char;
    }
    dst[n] = 0;
}

fn name_of(src: &[std::ffi::c_char; 32]) -> Vec<u8> {
    src.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect()
}

impl SaveData {
    fn data(&self) -> &SAVERESTOREDATA {
        // SAFETY: `p` is the engine's non-null `pSaveData`, valid while the engine is inside the
        // callback that handed it to the game (this object doesn't outlive it).
        unsafe { &*self.p }
    }

    fn data_mut(&mut self) -> &mut SAVERESTOREDATA {
        // SAFETY: as in `data`; the engine doesn't touch it while the game runs.
        unsafe { &mut *self.p }
    }

    /// `levelList[i]` (`i < MAX_LEVEL_CONNECTIONS`).
    pub fn connection(&self, i: usize) -> LevelConnection {
        let l = &self.data().levelList[i];
        LevelConnection {
            map: name_of(&l.mapName),
            landmark: name_of(&l.landmarkName),
            pent_landmark: l.pentLandmark,
            landmark_origin: l.vecLandmarkOrigin,
        }
    }

    /// Writes `levelList[i]`; names longer than 31 bytes are cut there (TS `strcpy`s them whole
    /// into the 32-byte fields).
    pub fn set_connection(&mut self, i: usize, c: &LevelConnection) {
        let l = &mut self.data_mut().levelList[i];
        copy_name(&mut l.mapName, &c.map);
        copy_name(&mut l.landmarkName, &c.landmark);
        l.pentLandmark = c.pent_landmark;
        l.vecLandmarkOrigin = c.landmark_origin;
    }

    pub fn set_connection_count(&mut self, n: i32) {
        self.data_mut().connectionCount = n;
    }

    /// `pTable != NULL`.
    pub fn has_table(&self) -> bool {
        !self.data().pTable.is_null()
    }

    fn table(&self) -> &[ENTITYTABLE] {
        let d = self.data();
        if d.pTable.is_null() || d.tableCount <= 0 {
            return &[];
        }
        // SAFETY: the engine allocates `tableCount` entries at `pTable` for the save.
        unsafe { std::slice::from_raw_parts(d.pTable, d.tableCount as usize) }
    }

    /// `CSaveRestoreBuffer::EntityIndex(edict_t *)` (TS 0xeee2c): the table index of `e`, or -1.
    pub fn entity_index(&self, e: *mut edict_t) -> i32 {
        if e.is_null() {
            return -1;
        }
        self.table().iter().position(|t| t.pent == e).map_or(-1, |i| i as i32)
    }

    /// `CSaveRestoreBuffer::EntityFlagsSet` (TS 0xeeed0): ORs `flags` into entry `index` and
    /// returns its flags; 0 for a bad index. TS accepts `index == tableCount` (one past the end);
    /// that write is refused here.
    pub fn entity_flags_set(&mut self, index: i32, flags: i32) -> i32 {
        let d = self.data();
        if d.pTable.is_null() || index < 0 || index >= d.tableCount {
            return 0;
        }
        // SAFETY: `index` is inside the engine's `tableCount` entries at `pTable`.
        let t = unsafe { &mut *d.pTable.add(index as usize) };
        t.flags |= flags;
        t.flags
    }
}

impl Engine {
    /// `gpGlobals->pSaveData`, when the engine has set it.
    pub fn save_data(&self) -> Option<SaveData> {
        let p = self.globals().pSaveData as *mut SAVERESTOREDATA;
        (!p.is_null()).then_some(SaveData { p })
    }

    /// `pfnEntitiesInPVS(pent)`: the entities in the PVS of `pent`'s origin, in the engine's chain
    /// order (linked through `v.chain`, ending at null or the world).
    pub fn entities_in_pvs(&self, pent: *mut edict_t) -> Vec<*mut edict_t> {
        let mut out = Vec::new();
        let mut e = eng!(pfnEntitiesInPVS(pent));
        while !e.is_null() && self.index_of(e) != 0 {
            out.push(e);
            e = e.v().chain;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_cut_to_the_field() {
        let mut f = [0 as std::ffi::c_char; 32];
        copy_name(&mut f, b"ts_dojo");
        assert_eq!(name_of(&f), b"ts_dojo");
        copy_name(&mut f, &[b'a'; 40]);
        assert_eq!(name_of(&f).len(), 31);
    }

    #[test]
    fn flags_on_a_table() {
        let mut table = [ENTITYTABLE::default(), ENTITYTABLE::default()];
        let e = 0x1000 as *mut edict_t;
        table[1].pent = e;
        let mut d = SAVERESTOREDATA { pTable: table.as_mut_ptr(), tableCount: 2, ..Default::default() };
        let mut sd = SaveData { p: &mut d };
        assert_eq!(sd.entity_index(e), 1);
        assert_eq!(sd.entity_index(0x2000 as *mut edict_t), -1);
        assert_eq!(sd.entity_flags_set(1, FENTTABLE_MOVEABLE | 1), FENTTABLE_MOVEABLE | 1);
        assert_eq!(sd.entity_flags_set(1, 4), FENTTABLE_MOVEABLE | 5);
        assert_eq!(sd.entity_flags_set(2, 1), 0, "one past the end is refused");
        assert_eq!(sd.entity_flags_set(-1, 1), 0);
        assert_eq!(table[1].flags, FENTTABLE_MOVEABLE | 5);
    }
}
