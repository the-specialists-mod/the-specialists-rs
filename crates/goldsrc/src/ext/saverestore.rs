//! The engine's save/restore state (`gpGlobals->pSaveData`, `SAVERESTOREDATA`) and TS's field
//! codec over it.
//!
//! * `ParmsChangeLevel`'s level connection list and the transition flags it sets in the entity
//!   table (`CSaveRestoreBuffer::EntityIndex` / `EntityFlagsSet`), plus `pfnEntitiesInPVS`.
//! * `CSave` / `CRestore` (TS 0xef0e4..0xf0ba4, the SDK's saverestore code unchanged): fields
//!   described by a `TYPEDESCRIPTION` list are written as `[short size][short token][data]`
//!   records, each class's block led by an `int` record holding the number of non-empty fields.
//!   The token is the field name's slot in the engine's string hash (`pTokens`).
//!
//! The codec works on [`Value`]s, one per field, holding what TS's object holds there; the game
//! maps its own state to and from them (`ts-server`'s `save` module), and [`raw`] reads and
//! writes the engine's own structs for `pfnSaveWriteFields` / `pfnSaveReadFields`. Engine calls
//! go through [`Host`] so the byte layout can be tested without an engine.

use crate::edict::EdictPtr;
use crate::eng;
use crate::server::Engine;
use crate::sys::*;
use std::ffi::{CStr, CString};

/// `MAX_LEVEL_CONNECTIONS`: the size of `SAVERESTOREDATA::levelList`.
pub const MAX_LEVEL_CONNECTIONS: usize = 16;
/// `FENTTABLE_GLOBAL`: an entity with a `globalname` that isn't dormant.
pub const FENTTABLE_GLOBAL: i32 = 0x1000_0000;
/// `FENTTABLE_MOVEABLE`: an entity that can cross a transition (`FCAP_ACROSS_TRANSITION`).
pub const FENTTABLE_MOVEABLE: i32 = 0x2000_0000;
/// `FTYPEDESC_GLOBAL`: the field is left alone when a global entity is restored over an existing
/// one.
pub const FTYPEDESC_GLOBAL: i16 = 1;
/// `MAX_ENTITYARRAY`: the entity index array `WriteFields` fills on its stack.
const MAX_ENTITYARRAY: usize = 64;

/// `FIELDTYPE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum FieldType {
    Float = 0,
    String,
    Entity,
    ClassPtr,
    EHandle,
    EVars,
    Edict,
    Vector,
    PositionVector,
    Pointer,
    Integer,
    Function,
    Boolean,
    Short,
    Character,
    Time,
    ModelName,
    SoundName,
}

impl FieldType {
    const ALL: [FieldType; 18] = {
        use FieldType::*;
        [
            Float,
            String,
            Entity,
            ClassPtr,
            EHandle,
            EVars,
            Edict,
            Vector,
            PositionVector,
            Pointer,
            Integer,
            Function,
            Boolean,
            Short,
            Character,
            Time,
            ModelName,
            SoundName,
        ]
    };

    pub fn from_raw(t: i32) -> Option<FieldType> {
        Self::ALL.get(usize::try_from(t).ok()?).copied()
    }

    /// `gSizes` (TS 0x122d20): the bytes per element `DataEmpty` tests and `ReadField` steps by.
    /// `FIELD_EHANDLE` counts 4 although an `EHANDLE` is 8 bytes (`ReadField` adds the rest).
    pub const fn size(self) -> usize {
        match self {
            FieldType::Vector | FieldType::PositionVector => 12,
            FieldType::Short => 2,
            FieldType::Character => 1,
            _ => 4,
        }
    }

    fn is_entity(self) -> bool {
        matches!(self, FieldType::Entity | FieldType::ClassPtr | FieldType::EHandle | FieldType::EVars | FieldType::Edict)
    }
}

/// A `TYPEDESCRIPTION` without its offset: the port's objects aren't TS's memory layout.
#[derive(Clone, Copy, Debug)]
pub struct FieldDesc {
    pub ty: FieldType,
    /// `fieldName`: also the token written for the field, so it must outlive the save (the
    /// engine keeps the pointer in `pTokens`).
    pub name: &'static CStr,
    /// `fieldSize`: the element count.
    pub count: i16,
    pub flags: i16,
}

impl FieldDesc {
    fn len(&self) -> usize {
        self.count.max(0) as usize
    }
}

/// An entity reference field (`FIELD_ENTITY` .. `FIELD_EDICT`) as the edict it names.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntRef {
    pub edict: *mut edict_t,
    /// The field's bytes aren't all zero (what `DataEmpty` tests): an `EHANDLE` whose entity went
    /// away still holds its pointer and serial number.
    pub nonzero: bool,
}

impl EntRef {
    pub const NULL: EntRef = EntRef { edict: std::ptr::null_mut(), nonzero: false };

    pub fn of(edict: *mut edict_t) -> EntRef {
        EntRef { edict, nonzero: !edict.is_null() }
    }
}

/// What TS's object holds in one field (all `count` elements).
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// `FIELD_FLOAT`, `FIELD_TIME`.
    Floats(Vec<f32>),
    /// `FIELD_INTEGER`, `FIELD_BOOLEAN`, `FIELD_POINTER`, and the `string_t`s of
    /// `FIELD_STRING` / `FIELD_MODELNAME` / `FIELD_SOUNDNAME`.
    Ints(Vec<i32>),
    /// `FIELD_VECTOR`, `FIELD_POSITION_VECTOR`.
    Vecs(Vec<[f32; 3]>),
    /// `FIELD_ENTITY`, `FIELD_CLASSPTR`, `FIELD_EHANDLE`, `FIELD_EVARS`, `FIELD_EDICT`.
    Ents(Vec<EntRef>),
    /// `FIELD_FUNCTION`: the word TS hands to `pfnNameForFunction` (only the first element is
    /// ever written).
    Func(u64),
    /// `FIELD_SHORT`.
    Shorts(Vec<i16>),
    /// `FIELD_CHARACTER`.
    Bytes(Vec<u8>),
}

impl Value {
    /// The field after `ReadFields`' `memset`.
    pub fn zero(d: &FieldDesc) -> Value {
        let n = d.len();
        use FieldType::*;
        match d.ty {
            Float | Time => Value::Floats(vec![0.0; n]),
            String | ModelName | SoundName | Pointer | Integer | Boolean => Value::Ints(vec![0; n]),
            Vector | PositionVector => Value::Vecs(vec![[0.0; 3]; n]),
            Entity | ClassPtr | EHandle | EVars | Edict => Value::Ents(vec![EntRef::NULL; n]),
            Function => Value::Func(0),
            Short => Value::Shorts(vec![0; n]),
            Character => Value::Bytes(vec![0; n]),
        }
    }

    /// `CSave::DataEmpty` (0xf0174) over the field's `count * gSizes[type]` bytes.
    pub fn is_empty(&self) -> bool {
        match self {
            Value::Floats(v) => v.iter().all(|f| f.to_bits() == 0),
            Value::Ints(v) => v.iter().all(|&i| i == 0),
            Value::Vecs(v) => v.iter().flatten().all(|f| f.to_bits() == 0),
            Value::Ents(v) => v.iter().all(|e| !e.nonzero),
            Value::Func(f) => *f as u32 == 0,
            Value::Shorts(v) => v.iter().all(|&i| i == 0),
            Value::Bytes(v) => v.iter().all(|&i| i == 0),
        }
    }
}

/// The engine services the codec uses (the real one is [`EngineHost`]).
pub trait Host {
    fn alert(&self, kind: ALERT_TYPE, msg: &str);
    /// `STRING(s)`.
    fn string(&self, s: i32) -> Vec<u8>;
    /// `ALLOC_STRING`.
    fn alloc_string(&self, s: &CStr) -> i32;
    fn precache_model(&self, s: i32);
    fn precache_sound(&self, s: i32);
    fn name_for_function(&self, f: u64) -> Option<Vec<u8>>;
    fn function_from_name(&self, name: &CStr) -> u64;
    fn has_private_data(&self, e: *mut edict_t) -> bool;
}

/// [`Host`] over the engine's function table.
pub struct EngineHost;

impl Host for EngineHost {
    fn alert(&self, kind: ALERT_TYPE, msg: &str) {
        crate::server::engine().alert(kind, msg);
    }

    fn string(&self, s: i32) -> Vec<u8> {
        crate::server::engine().string(s).to_bytes().to_vec()
    }

    fn alloc_string(&self, s: &CStr) -> i32 {
        crate::server::engine().alloc_string(s)
    }

    fn precache_model(&self, s: i32) {
        let eng = crate::server::engine();
        eng.precache_model(eng.string(s));
    }

    fn precache_sound(&self, s: i32) {
        let eng = crate::server::engine();
        eng.precache_sound(eng.string(s));
    }

    fn name_for_function(&self, f: u64) -> Option<Vec<u8>> {
        let p = eng!(pfnNameForFunction(f as _));
        // SAFETY: the engine returns null or a NUL-terminated string it owns.
        (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) }.to_bytes().to_vec())
    }

    // `c_ulong` is 32 bits on Windows and 64 elsewhere.
    #[allow(clippy::useless_conversion)]
    fn function_from_name(&self, name: &CStr) -> u64 {
        u64::from(eng!(pfnFunctionFromName(name.as_ptr())))
    }

    fn has_private_data(&self, e: *mut edict_t) -> bool {
        e.has_private_data()
    }
}

/// One `LEVELLIST` entry as the game fills it (`AddTransitionToList`).
#[derive(Clone, Debug, PartialEq)]
pub struct LevelConnection {
    pub map: Vec<u8>,
    pub landmark: Vec<u8>,
    pub pent_landmark: *mut edict_t,
    pub landmark_origin: [f32; 3],
}

/// One `ENTITYTABLE` entry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TableEntry {
    pub id: i32,
    pub pent: *mut edict_t,
    pub location: i32,
    pub size: i32,
    pub flags: i32,
    pub classname: string_t,
}

/// `pSaveData` (`gpGlobals->pSaveData`, or the one a save callback receives): what TS's
/// `CSaveRestoreBuffer` wraps.
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

fn name_of(src: &[std::ffi::c_char]) -> Vec<u8> {
    src.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect()
}

/// `CSaveRestoreBuffer::HashString` (0xeefe8): rotate right 4, xor the (signed) character.
fn hash_string(s: &CStr) -> u32 {
    s.to_bytes().iter().fold(0u32, |h, &c| h.rotate_right(4) ^ (c as i8 as i32 as u32))
}

/// `strcasecmp(a, b) == 0`.
fn eq_ignore_case(a: &CStr, b: &[u8]) -> bool {
    a.to_bytes().eq_ignore_ascii_case(b)
}

impl SaveData {
    /// Wraps an engine `SAVERESTOREDATA`.
    ///
    /// # Safety
    /// `p` must be null or point at a `SAVERESTOREDATA` the engine keeps valid (with its buffer,
    /// token table and entity table) while the returned value is used.
    pub unsafe fn from_raw(p: *mut SAVERESTOREDATA) -> Option<SaveData> {
        (!p.is_null()).then_some(SaveData { p })
    }

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

    fn table_mut(&mut self) -> &mut [ENTITYTABLE] {
        let d = self.data();
        if d.pTable.is_null() || d.tableCount <= 0 {
            return &mut [];
        }
        // SAFETY: as in `table`, borrowed mutably through `self`.
        unsafe { std::slice::from_raw_parts_mut(d.pTable, d.tableCount as usize) }
    }

    /// `CSaveRestoreBuffer::EntityIndex(edict_t *)` (TS 0xeee2c): the table index of `e`, or -1.
    pub fn entity_index(&self, e: *mut edict_t) -> i32 {
        if e.is_null() {
            return -1;
        }
        self.table().iter().position(|t| t.pent == e).map_or(-1, |i| i as i32)
    }

    /// `CSaveRestoreBuffer::EntityFromIndex` (TS 0xeee7c): the edict of the entry whose `id` is
    /// `index`, or null.
    pub fn entity_from_index(&self, index: i32) -> *mut edict_t {
        if index < 0 {
            return std::ptr::null_mut();
        }
        self.table().iter().find(|t| t.id == index).map_or(std::ptr::null_mut(), |t| t.pent)
    }

    /// `CSaveRestoreBuffer::EntityFlagsSet` (TS 0xeeed0): ORs `flags` into entry `index` and
    /// returns its flags; 0 for a bad index. TS accepts `index == tableCount` (one past the end);
    /// that write is refused here.
    pub fn entity_flags_set(&mut self, index: i32, flags: i32) -> i32 {
        match self.table_mut().get_mut(usize::try_from(index).unwrap_or(usize::MAX)) {
            Some(t) => {
                t.flags |= flags;
                t.flags
            }
            None => 0,
        }
    }

    /// `pTable[i]`.
    pub fn table_entry(&self, i: i32) -> Option<TableEntry> {
        let t = self.table().get(usize::try_from(i).ok()?)?;
        Some(TableEntry { id: t.id, pent: t.pent, location: t.location, size: t.size, flags: t.flags, classname: t.classname })
    }

    /// Writes `location`, `size` and `classname` of `pTable[i]` (what `DispatchSave` fills).
    pub fn set_table_entry(&mut self, i: i32, location: i32, size: i32, classname: string_t) {
        if let Some(t) = self.table_mut().get_mut(usize::try_from(i).unwrap_or(usize::MAX)) {
            t.location = location;
            t.size = size;
            t.classname = classname;
        }
    }

    pub fn current_index(&self) -> i32 {
        self.data().currentIndex
    }

    /// `size`: the bytes written (or read) so far.
    pub fn size(&self) -> i32 {
        self.data().size
    }

    /// `size = location; pCurrentData = pBaseData + location` (`DispatchRestore` rewinds to an
    /// entity's start this way).
    pub fn seek(&mut self, location: i32) {
        let d = self.data_mut();
        d.size = location;
        d.pCurrentData = d.pBaseData.wrapping_offset(location as isize);
    }

    pub fn time(&self) -> f32 {
        self.data().time
    }

    pub fn use_landmark(&self) -> bool {
        self.data().fUseLandmark != 0
    }

    pub fn landmark_offset(&self) -> [f32; 3] {
        self.data().vecLandmarkOffset
    }

    pub fn set_landmark_offset(&mut self, v: [f32; 3]) {
        self.data_mut().vecLandmarkOffset = v;
    }

    pub fn landmark_name(&self) -> Vec<u8> {
        name_of(&self.data().szLandmarkName)
    }

    pub fn current_map_name(&self) -> Vec<u8> {
        name_of(&self.data().szCurrentMapName)
    }

    // ---- the buffer -------------------------------------------------------------------------

    fn offset(&self) -> isize {
        let d = self.data();
        (d.pCurrentData as isize).wrapping_sub(d.pBaseData as isize)
    }

    /// The buffer's `bufferSize` bytes.
    fn buffer(&self) -> &[u8] {
        let d = self.data();
        if d.pBaseData.is_null() || d.bufferSize <= 0 {
            return &[];
        }
        // SAFETY: the engine allocates `bufferSize` bytes at `pBaseData` for the save/restore.
        unsafe { std::slice::from_raw_parts(d.pBaseData.cast::<u8>(), d.bufferSize as usize) }
    }

    /// `n` bytes of the buffer from `at`; bytes past its end read as 0.
    fn input(&self, at: isize, n: usize) -> Vec<u8> {
        let b = self.buffer();
        (0..n).map(|i| usize::try_from(at + i as isize).ok().and_then(|j| b.get(j)).copied().unwrap_or(0)).collect()
    }

    /// The NUL-terminated string at `at`, after skipping `skip` strings; the buffer's end ends it.
    fn input_string(&self, at: isize, skip: usize) -> Vec<u8> {
        let b = self.buffer();
        let mut i = usize::try_from(at).unwrap_or(usize::MAX);
        for _ in 0..skip {
            while b.get(i).is_some_and(|&c| c != 0) {
                i += 1;
            }
            i = i.saturating_add(1);
        }
        b.get(i..).unwrap_or(&[]).iter().take_while(|&&c| c != 0).copied().collect()
    }

    /// `CSaveRestoreBuffer::BufferRewind` (0xeef10).
    fn rewind(&mut self, n: i32) {
        let d = self.data_mut();
        let n = n.min(d.size);
        d.pCurrentData = d.pCurrentData.wrapping_offset(-(n as isize));
        d.size -= n;
    }

    /// `CSaveRestoreBuffer::TokenHash` (0xef028): the slot holding `name` in `pTokens`, putting it
    /// in the first free slot of its probe sequence. The engine keeps the pointer.
    pub fn token_hash(&mut self, host: &dyn Host, name: &'static CStr) -> i32 {
        let hash = hash_string(name);
        let d = self.data_mut();
        let count = d.tokenCount;
        if count > 0 && !d.pTokens.is_null() {
            for i in 0..count {
                let mut index = i + (hash % count as u32) as u16 as i32;
                if index >= count {
                    index -= count;
                }
                // SAFETY: `index < tokenCount`, the size of the engine's `pTokens` array.
                let slot = unsafe { &mut *d.pTokens.add(index as usize) };
                // SAFETY: a used slot holds a NUL-terminated token string.
                if slot.is_null() || unsafe { CStr::from_ptr(*slot) } == name {
                    *slot = name.as_ptr().cast_mut();
                    return index;
                }
            }
        }
        host.alert(ALERT_TYPE::at_error, "CSaveRestoreBuffer :: TokenHash() is COMPLETELY FULL!");
        0
    }

    /// `pTokens[i]`. TS indexes without a bound and `strcasecmp`s whatever is there; the port
    /// gives `None` past the table or for an empty slot.
    fn token(&self, i: u16) -> Option<Vec<u8>> {
        let d = self.data();
        if d.pTokens.is_null() || i as i32 >= d.tokenCount {
            return None;
        }
        // SAFETY: `i < tokenCount`.
        let p = unsafe { *d.pTokens.add(i as usize) };
        // SAFETY: a used slot holds a NUL-terminated token string.
        (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) }.to_bytes().to_vec())
    }

    // ---- CSave ------------------------------------------------------------------------------

    /// `CSave::BufferData` (0xf02a4).
    fn buffer_data(&mut self, host: &dyn Host, bytes: &[u8]) {
        let n = bytes.len() as i32;
        let d = self.data_mut();
        if d.bufferSize < d.size + n {
            host.alert(ALERT_TYPE::at_error, "Save/Restore overflow!");
            d.size = d.bufferSize;
            return;
        }
        // SAFETY: `size + n <= bufferSize`, and `pCurrentData` is `pBaseData + size` (the engine
        // and this code only ever advance them together), so the copy stays in the buffer.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), d.pCurrentData.cast::<u8>(), bytes.len()) };
        d.pCurrentData = d.pCurrentData.wrapping_add(bytes.len());
        d.size += n;
    }

    /// `CSave::BufferHeader` (0xf022c): the size's low 16 bits, then the token.
    fn buffer_header(&mut self, host: &dyn Host, name: &'static CStr, size: i32) {
        let token = self.token_hash(host, name) as i16;
        if size > 0x10000 {
            host.alert(ALERT_TYPE::at_error, "CSave :: BufferHeader() size parameter exceeds 'short'!");
        }
        self.buffer_data(host, &(size as i16).to_le_bytes());
        self.buffer_data(host, &token.to_le_bytes());
    }

    /// `CSave::BufferField` (0xf01ec).
    fn buffer_field(&mut self, host: &dyn Host, name: &'static CStr, bytes: &[u8]) {
        self.buffer_header(host, name, bytes.len() as i32);
        self.buffer_data(host, bytes);
    }

    fn write_ints(&mut self, host: &dyn Host, name: &'static CStr, v: &[i32]) {
        let bytes: Vec<u8> = v.iter().flat_map(|i| i.to_le_bytes()).collect();
        self.buffer_field(host, name, &bytes);
    }

    /// `CSave::WriteString(char const *, int const *, int)` (0xef39c): the strings with their
    /// NULs, one header for all.
    fn write_strings(&mut self, host: &dyn Host, name: &'static CStr, v: &[i32]) {
        let strings: Vec<Vec<u8>> = v.iter().map(|&s| host.string(s)).collect();
        let total: usize = strings.iter().map(|s| s.len() + 1).sum();
        self.buffer_header(host, name, total as i32);
        for s in strings {
            self.buffer_data(host, &s);
            self.buffer_data(host, &[0]);
        }
    }

    /// `CSave::WriteFunction` (0xef980): the engine's name for the word, with its NUL. TS writes
    /// nothing (the field was still counted) when the engine has no name for it.
    fn write_function(&mut self, host: &dyn Host, name: &'static CStr, f: u64) {
        match host.name_for_function(f) {
            Some(mut s) => {
                s.push(0);
                self.buffer_field(host, name, &s);
            }
            None => host.alert(ALERT_TYPE::at_error, "Invalid function pointer in entity!"),
        }
    }

    /// `CSave::WriteFields` (0xefb78): the count of non-empty fields as an `int` record named
    /// `name`, then a record per non-empty field. `vals` holds one value per descriptor.
    pub fn write_fields(&mut self, host: &dyn Host, name: &'static CStr, descs: &[FieldDesc], vals: &[Value]) -> bool {
        debug_assert_eq!(descs.len(), vals.len(), "{name:?}");
        let empty = vals.iter().filter(|v| v.is_empty()).count();
        self.write_ints(host, name, &[(descs.len() - empty) as i32]);
        for (d, v) in descs.iter().zip(vals) {
            if v.is_empty() {
                continue;
            }
            self.write_field(host, d, v);
        }
        true
    }

    fn write_field(&mut self, host: &dyn Host, d: &FieldDesc, v: &Value) {
        let n = d.len();
        use FieldType::*;
        match (d.ty, v) {
            (Float, Value::Floats(f)) => {
                let bytes: Vec<u8> = f.iter().take(n).flat_map(|x| x.to_le_bytes()).collect();
                self.buffer_field(host, d.name, &bytes);
            }
            // CSave::WriteTime (0xef1a0): relative to the save's time.
            (Time, Value::Floats(f)) => {
                let t = self.time();
                self.buffer_header(host, d.name, (n * 4) as i32);
                for x in f.iter().take(n) {
                    self.buffer_data(host, &(x - t).to_le_bytes());
                }
            }
            (String | ModelName | SoundName, Value::Ints(s)) => self.write_strings(host, d.name, &s[..n.min(s.len())]),
            (Entity | ClassPtr | EHandle | EVars | Edict, Value::Ents(e)) => {
                if n > MAX_ENTITYARRAY {
                    host.alert(ALERT_TYPE::at_error, &format!("Can't save more than {MAX_ENTITYARRAY} entities in an array!!!\n"));
                }
                // TS fills a 64-entry stack array and writes past it for a longer array; no TS
                // table has one (the longest is 32).
                let idx: Vec<i32> = e.iter().take(n).map(|r| self.entity_index(r.edict)).collect();
                self.write_ints(host, d.name, &idx);
            }
            (Vector, Value::Vecs(vs)) => {
                let bytes: Vec<u8> = vs.iter().take(n).flatten().flat_map(|x| x.to_le_bytes()).collect();
                self.buffer_field(host, d.name, &bytes);
            }
            // CSave::WritePositionVector (0xef774): relative to the landmark in a transition.
            (PositionVector, Value::Vecs(vs)) => {
                let off = if self.use_landmark() { self.landmark_offset() } else { [0.0; 3] };
                self.buffer_header(host, d.name, (n * 12) as i32);
                for p in vs.iter().take(n) {
                    let p = [p[0] - off[0], p[1] - off[1], p[2] - off[2]];
                    let bytes: Vec<u8> = p.iter().flat_map(|x| x.to_le_bytes()).collect();
                    self.buffer_data(host, &bytes);
                }
            }
            (Pointer | Integer | Boolean, Value::Ints(i)) => self.write_ints(host, d.name, &i[..n.min(i.len())]),
            (Function, Value::Func(f)) => self.write_function(host, d.name, *f),
            (Short, Value::Shorts(s)) => {
                let bytes: Vec<u8> = s.iter().take(n).flat_map(|x| x.to_le_bytes()).collect();
                self.buffer_field(host, d.name, &bytes);
            }
            (Character, Value::Bytes(b)) => self.buffer_field(host, d.name, &b[..n.min(b.len())]),
            _ => host.alert(ALERT_TYPE::at_error, "Bad field type\n"),
        }
    }

    // ---- CRestore ---------------------------------------------------------------------------

    /// `CRestore::BufferReadBytes` (0xf0a38): nothing at all once the read position is past the
    /// buffer.
    fn read_bytes(&mut self, host: &dyn Host, out: Option<&mut [u8]>, n: i32) {
        if self.offset() >= self.data().bufferSize as isize {
            return;
        }
        if self.data().bufferSize < self.data().size + n {
            host.alert(ALERT_TYPE::at_error, "Restore overflow!");
            let d = self.data_mut();
            d.size = d.bufferSize;
            return;
        }
        if let Some(out) = out {
            let bytes = self.input(self.offset(), n.max(0) as usize);
            out.copy_from_slice(&bytes[..out.len()]);
        }
        let d = self.data_mut();
        d.pCurrentData = d.pCurrentData.wrapping_offset(n as isize);
        d.size += n;
    }

    fn read_short(&mut self, host: &dyn Host) -> i16 {
        let mut b = [0u8; 2];
        self.read_bytes(host, Some(&mut b), 2);
        i16::from_le_bytes(b)
    }

    fn read_int(&mut self, host: &dyn Host) -> i32 {
        let mut b = [0u8; 4];
        self.read_bytes(host, Some(&mut b), 4);
        i32::from_le_bytes(b)
    }

    /// `CRestore::BufferReadHeader` (0xf0908): size, token, and where the data starts; skips it.
    fn read_header(&mut self, host: &dyn Host) -> (u16, u16, isize) {
        let size = self.read_short(host) as u16;
        let token = self.read_short(host) as u16;
        let at = self.offset();
        self.read_bytes(host, None, size as i32);
        (size, token, at)
    }

    /// `CRestore::ReadFields` (0xf07a4): reads the block `write_fields` wrote under `name`. `None`
    /// (with the 4 header bytes given back) when the next record isn't that block. Otherwise one
    /// entry per descriptor: the field's new contents, or `None` for a `FTYPEDESC_GLOBAL` field
    /// of a `global` restore, which TS leaves untouched. Fields not in the save read as zero (TS
    /// `memset`s them first).
    pub fn read_fields(
        &mut self,
        host: &dyn Host,
        name: &'static CStr,
        descs: &[FieldDesc],
        global: bool,
        precache: bool,
    ) -> Option<Vec<Option<Value>>> {
        self.read_short(host);
        let token = self.read_short(host);
        if token != self.token_hash(host, name) as i16 {
            self.rewind(4);
            return None;
        }
        let count = self.read_int(host);
        let mut out: Vec<Option<Value>> =
            descs.iter().map(|d| (!global || d.flags & FTYPEDESC_GLOBAL == 0).then(|| Value::zero(d))).collect();
        // TS counts the records in a 16-bit counter compared with the int count, so a count past
        // 65535 never ends; the port stops there.
        let mut last = 0;
        for _ in 0..count.clamp(0, 0x10000) {
            let (_, token, at) = self.read_header(host);
            let field = self.token(token);
            last = self.read_field(host, descs, &mut out, last, field.as_deref(), at, global, precache) + 1;
        }
        Some(out)
    }

    /// `CRestore::ReadField` (0xf0314): finds the descriptor named `field` (case-insensitively,
    /// searching from `start` and wrapping) and decodes its `count` elements from `at`. Returns
    /// its index, or -1.
    #[allow(clippy::too_many_arguments)] // TS's ReadField takes the same set.
    fn read_field(
        &mut self,
        host: &dyn Host,
        descs: &[FieldDesc],
        out: &mut [Option<Value>],
        start: i32,
        field: Option<&[u8]>,
        at: isize,
        global: bool,
        precache: bool,
    ) -> i32 {
        let n = descs.len() as i32;
        let time = self.time();
        let landmark = if self.use_landmark() { self.landmark_offset() } else { [0.0; 3] };
        for i in 0..n {
            let idx = ((start + i) % n) as usize;
            let d = &descs[idx];
            // A missing token is a NULL `strcasecmp` argument in TS (a crash); it matches nothing.
            if !field.is_some_and(|f| eq_ignore_case(d.name, f)) {
                continue;
            }
            if global && d.flags & FTYPEDESC_GLOBAL != 0 {
                return idx as i32;
            }
            let mut v = out[idx].take().unwrap_or_else(|| Value::zero(d));
            for j in 0..d.len() {
                let el = at + (j * d.ty.size()) as isize;
                self.decode(host, d, &mut v, j, el, at, time, landmark, precache);
            }
            out[idx] = Some(v);
            return idx as i32;
        }
        -1
    }

    #[allow(clippy::too_many_arguments)]
    fn decode(
        &self,
        host: &dyn Host,
        d: &FieldDesc,
        v: &mut Value,
        j: usize,
        el: isize,
        at: isize,
        time: f32,
        landmark: [f32; 3],
        precache: bool,
    ) {
        let f32_at = |o: isize| f32::from_le_bytes(self.input(o, 4).try_into().unwrap());
        let i32_at = |o: isize| i32::from_le_bytes(self.input(o, 4).try_into().unwrap());
        use FieldType::*;
        match (d.ty, v) {
            (Float, Value::Floats(f)) => f[j] = f32_at(el),
            (Time, Value::Floats(f)) => f[j] = time + f32_at(el),
            (String | ModelName | SoundName, Value::Ints(s)) => {
                // Element j is the string after j NULs from the field's start.
                let text = self.input_string(at, j);
                s[j] = 0;
                if !text.is_empty() {
                    let text = CString::new(text).unwrap_or_default();
                    s[j] = host.alloc_string(&text);
                    if s[j] != 0 && precache {
                        match d.ty {
                            ModelName => host.precache_model(s[j]),
                            SoundName => host.precache_sound(s[j]),
                            _ => {}
                        }
                    }
                }
            }
            (ty, Value::Ents(e)) if ty.is_entity() => {
                let edict = self.entity_from_index(i32_at(el));
                // FIELD_CLASSPTR / FIELD_EHANDLE take the edict's object: none without private data.
                let edict = if matches!(ty, ClassPtr | EHandle) && !host.has_private_data(edict) { std::ptr::null_mut() } else { edict };
                e[j] = EntRef::of(edict);
            }
            (Vector, Value::Vecs(vs)) => vs[j] = [f32_at(el), f32_at(el + 4), f32_at(el + 8)],
            (PositionVector, Value::Vecs(vs)) => {
                vs[j] = [f32_at(el) + landmark[0], f32_at(el + 4) + landmark[1], f32_at(el + 8) + landmark[2]]
            }
            (Pointer | Integer | Boolean, Value::Ints(i)) => i[j] = i32_at(el),
            (Function, Value::Func(f)) => {
                let text = self.input_string(el, 0);
                *f = if text.is_empty() { 0 } else { host.function_from_name(&CString::new(text).unwrap_or_default()) };
            }
            (Short, Value::Shorts(s)) => s[j] = i16::from_le_bytes(self.input(el, 2).try_into().unwrap()),
            (Character, Value::Bytes(b)) => b[j] = self.input(el, 1)[0],
            _ => host.alert(ALERT_TYPE::at_error, "Bad field type\n"),
        }
    }
}

impl Engine {
    /// `gpGlobals->pSaveData`, when the engine has set it.
    pub fn save_data(&self) -> Option<SaveData> {
        let p = self.globals().pSaveData as *mut SAVERESTOREDATA;
        // SAFETY: the engine sets `pSaveData` to its live save/restore state or null.
        unsafe { SaveData::from_raw(p) }
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

/// The engine's own structs, which it hands to `pfnSaveWriteFields` / `pfnSaveReadFields` with
/// their `TYPEDESCRIPTION`s (the save header, entity table, adjacency list, light styles...).
pub mod raw {
    use super::*;
    use std::mem::size_of;

    /// The bytes between elements in memory. TS's are `gSizes`; pointers and handles take their
    /// real size here (they're 8 bytes on the 64-bit FWGS build, which TS never had).
    fn stride(ty: FieldType) -> usize {
        use FieldType::*;
        match ty {
            ClassPtr | EVars | Edict | Pointer | Function => size_of::<usize>(),
            EHandle => 2 * size_of::<usize>(),
            _ => ty.size(),
        }
    }

    /// The engine's descriptor list as [`FieldDesc`]s with their offsets. A field type TS doesn't
    /// know (it would size it from past the end of `gSizes`) becomes an empty character array.
    ///
    /// # Safety
    /// `descs` must point at `n` descriptors whose names are NUL-terminated strings the engine
    /// keeps for the process (they become tokens).
    pub unsafe fn descs(descs: *const TYPEDESCRIPTION, n: i32) -> Vec<(FieldDesc, usize)> {
        if descs.is_null() || n <= 0 {
            return Vec::new();
        }
        // SAFETY: per the contract.
        let raw = unsafe { std::slice::from_raw_parts(descs, n as usize) };
        raw.iter()
            .map(|d| {
                // SAFETY: per the contract, `fieldName` is a static NUL-terminated string.
                let name: &'static CStr = if d.fieldName.is_null() { c"" } else { unsafe { CStr::from_ptr(d.fieldName) } };
                let (ty, count) = match FieldType::from_raw(d.fieldType as i32) {
                    Some(t) => (t, d.fieldSize),
                    None => (FieldType::Character, 0),
                };
                (FieldDesc { ty, name, count, flags: d.flags }, d.fieldOffset.max(0) as usize)
            })
            .collect()
    }

    /// Reads field `d` at `base + offset` as a [`Value`].
    ///
    /// # Safety
    /// `base + offset` must hold `d.count` elements of the field's type.
    pub unsafe fn load(host: &dyn Host, base: *const u8, d: &FieldDesc, offset: usize) -> Value {
        let n = d.len();
        let st = stride(d.ty);
        // SAFETY: per the contract (unaligned: the engine's structs are packed as it likes).
        let at = |j: usize| unsafe { base.add(offset + j * st) };
        // SAFETY: as above, for each reader.
        let rd_i32 = |j: usize| unsafe { at(j).cast::<i32>().read_unaligned() };
        // SAFETY: as above.
        let rd_ptr = |j: usize| unsafe { at(j).cast::<usize>().read_unaligned() };
        use FieldType::*;
        match d.ty {
            Float | Time => Value::Floats((0..n).map(|j| f32::from_bits(rd_i32(j) as u32)).collect()),
            String | ModelName | SoundName | Integer | Boolean => Value::Ints((0..n).map(rd_i32).collect()),
            Pointer => Value::Ints((0..n).map(|j| rd_ptr(j) as i32).collect()),
            Vector | PositionVector => Value::Vecs(
                (0..n)
                    .map(|j| {
                        // SAFETY: as above.
                        unsafe { at(j).cast::<[f32; 3]>().read_unaligned() }
                    })
                    .collect(),
            ),
            Entity => Value::Ents(
                (0..n)
                    .map(|j| {
                        let off = rd_i32(j);
                        EntRef { edict: eng!(pfnPEntityOfEntOffset(off)), nonzero: off != 0 }
                    })
                    .collect(),
            ),
            Edict => Value::Ents((0..n).map(|j| EntRef::of(rd_ptr(j) as *mut edict_t)).collect()),
            EVars => Value::Ents(
                (0..n)
                    .map(|j| {
                        let pev = rd_ptr(j) as *mut entvars_t;
                        let e = if pev.is_null() { std::ptr::null_mut() } else { eng!(pfnFindEntityByVars(pev)) };
                        EntRef { edict: e, nonzero: !pev.is_null() }
                    })
                    .collect(),
            ),
            ClassPtr => Value::Ents(
                (0..n)
                    .map(|j| {
                        let p = rd_ptr(j);
                        EntRef { edict: edict_of_private(p), nonzero: p != 0 }
                    })
                    .collect(),
            ),
            EHandle => Value::Ents(
                (0..n)
                    .map(|j| {
                        let e = rd_ptr(j) as *mut edict_t;
                        // SAFETY: as above; the serial follows the pointer.
                        let serial = unsafe { at(j).add(size_of::<usize>()).cast::<i32>().read_unaligned() };
                        let live = !e.is_null() && e.serial_number() == serial && host.has_private_data(e);
                        EntRef { edict: if live { e } else { std::ptr::null_mut() }, nonzero: !e.is_null() || serial != 0 }
                    })
                    .collect(),
            ),
            Function => Value::Func(if n > 0 { rd_ptr(0) as u64 } else { 0 }),
            Short => Value::Shorts(
                (0..n)
                    .map(|j| {
                        // SAFETY: as above.
                        unsafe { at(j).cast::<i16>().read_unaligned() }
                    })
                    .collect(),
            ),
            Character => Value::Bytes(
                (0..n)
                    .map(|j| {
                        // SAFETY: as above.
                        unsafe { at(j).read() }
                    })
                    .collect(),
            ),
        }
    }

    /// The edict whose private data is `p` (a `CBaseEntity *` in TS), or null.
    fn edict_of_private(p: usize) -> *mut edict_t {
        if p == 0 {
            return std::ptr::null_mut();
        }
        let eng = crate::server::engine();
        let max = eng.globals().maxEntities;
        for i in 0..max {
            let e = eng.entity_of_index(i);
            // SAFETY: a non-null edict from the engine's array.
            if !e.is_null() && unsafe { (*e).pvPrivateData } as usize == p {
                return e;
            }
        }
        std::ptr::null_mut()
    }

    /// Stores `v` into field `d` at `base + offset`.
    ///
    /// # Safety
    /// As [`load`], writable.
    pub unsafe fn store(base: *mut u8, d: &FieldDesc, offset: usize, v: &Value) {
        let st = stride(d.ty);
        // SAFETY: per the contract.
        let at = |j: usize| unsafe { base.add(offset + j * st) };
        // SAFETY: as above, for each writer.
        let wr_i32 = |j: usize, x: i32| unsafe { at(j).cast::<i32>().write_unaligned(x) };
        // SAFETY: as above.
        let wr_ptr = |j: usize, x: usize| unsafe { at(j).cast::<usize>().write_unaligned(x) };
        use FieldType::*;
        match (d.ty, v) {
            (Float | Time, Value::Floats(f)) => f.iter().enumerate().for_each(|(j, x)| wr_i32(j, x.to_bits() as i32)),
            (String | ModelName | SoundName | Integer | Boolean, Value::Ints(i)) => i.iter().enumerate().for_each(|(j, &x)| wr_i32(j, x)),
            (Pointer, Value::Ints(i)) => i.iter().enumerate().for_each(|(j, &x)| wr_ptr(j, x as isize as usize)),
            (Vector | PositionVector, Value::Vecs(vs)) => vs.iter().enumerate().for_each(|(j, p)| {
                // SAFETY: as above.
                unsafe { at(j).cast::<[f32; 3]>().write_unaligned(*p) }
            }),
            (Entity, Value::Ents(e)) => e.iter().enumerate().for_each(|(j, r)| {
                wr_i32(j, if r.edict.is_null() { 0 } else { eng!(pfnEntOffsetOfPEntity(r.edict)) });
            }),
            (Edict, Value::Ents(e)) => e.iter().enumerate().for_each(|(j, r)| wr_ptr(j, r.edict as usize)),
            (EVars, Value::Ents(e)) => e.iter().enumerate().for_each(|(j, r)| {
                // SAFETY: a non-null edict from the engine's array.
                wr_ptr(j, if r.edict.is_null() { 0 } else { (unsafe { &raw mut (*r.edict).v }) as usize });
            }),
            (ClassPtr, Value::Ents(e)) => e.iter().enumerate().for_each(|(j, r)| {
                // SAFETY: as above.
                wr_ptr(j, if r.edict.is_null() { 0 } else { (unsafe { (*r.edict).pvPrivateData }) as usize });
            }),
            // `EHANDLE::operator=`: a null entity leaves the serial number as it was.
            (EHandle, Value::Ents(e)) => e.iter().enumerate().for_each(|(j, r)| {
                wr_ptr(j, r.edict as usize);
                if !r.edict.is_null() {
                    // SAFETY: as above; the serial follows the pointer.
                    unsafe { at(j).add(size_of::<usize>()).cast::<i32>().write_unaligned(r.edict.serial_number()) };
                }
            }),
            (Function, Value::Func(f)) => {
                if d.count > 0 {
                    wr_ptr(0, *f as usize)
                }
            }
            (Short, Value::Shorts(s)) => s.iter().enumerate().for_each(|(j, &x)| {
                // SAFETY: as above.
                unsafe { at(j).cast::<i16>().write_unaligned(x) }
            }),
            (Character, Value::Bytes(b)) => b.iter().enumerate().for_each(|(j, &x)| {
                // SAFETY: as above.
                unsafe { at(j).write(x) }
            }),
            _ => {}
        }
    }
}

/// An in-memory save buffer and a fake [`Host`], for tests of the codec and of what the game
/// maps onto it.
#[doc(hidden)]
pub mod testing {
    use super::*;
    use std::cell::RefCell;

    /// A [`Host`] with a fake string pool and function table, logging what the codec asks for.
    #[derive(Default)]
    pub struct FakeHost {
        pub strings: RefCell<Vec<Vec<u8>>>,
        pub log: RefCell<Vec<String>>,
        pub functions: Vec<(u64, &'static str)>,
    }

    impl Host for FakeHost {
        fn alert(&self, _: ALERT_TYPE, msg: &str) {
            self.log.borrow_mut().push(format!("alert {msg}"));
        }
        fn string(&self, s: i32) -> Vec<u8> {
            self.strings.borrow().get(s as usize).cloned().unwrap_or_default()
        }
        fn alloc_string(&self, s: &CStr) -> i32 {
            let mut v = self.strings.borrow_mut();
            if v.is_empty() {
                v.push(Vec::new());
            }
            v.push(s.to_bytes().to_vec());
            v.len() as i32 - 1
        }
        fn precache_model(&self, s: i32) {
            let name = String::from_utf8(self.string(s)).unwrap();
            self.log.borrow_mut().push(format!("model {name}"));
        }
        fn precache_sound(&self, s: i32) {
            let name = String::from_utf8(self.string(s)).unwrap();
            self.log.borrow_mut().push(format!("sound {name}"));
        }
        fn name_for_function(&self, f: u64) -> Option<Vec<u8>> {
            self.functions.iter().find(|(a, _)| *a == f).map(|(_, n)| n.as_bytes().to_vec())
        }
        fn function_from_name(&self, name: &CStr) -> u64 {
            self.functions.iter().find(|(_, n)| n.as_bytes() == name.to_bytes()).map_or(0, |(a, _)| *a)
        }
        fn has_private_data(&self, e: *mut edict_t) -> bool {
            !e.is_null()
        }
    }

    /// An in-memory `SAVERESTOREDATA` with its buffer, token table and entity table.
    pub struct Buf {
        pub data: Box<SAVERESTOREDATA>,
        pub bytes: Vec<u8>,
        pub tokens: Vec<*mut std::ffi::c_char>,
        pub table: Vec<ENTITYTABLE>,
    }

    impl Buf {
        pub fn new(size: usize, tokens: usize, table: Vec<ENTITYTABLE>) -> Buf {
            let mut b = Buf { data: Box::default(), bytes: vec![0; size], tokens: vec![std::ptr::null_mut(); tokens], table };
            b.data.pBaseData = b.bytes.as_mut_ptr().cast();
            b.data.pCurrentData = b.data.pBaseData;
            b.data.bufferSize = size as i32;
            b.data.tokenCount = tokens as i32;
            b.data.pTokens = b.tokens.as_mut_ptr();
            b.data.pTable = b.table.as_mut_ptr();
            b.data.tableCount = b.table.len() as i32;
            b
        }

        pub fn sd(&mut self) -> SaveData {
            SaveData { p: &mut *self.data }
        }

        /// The bytes written so far.
        pub fn written(&self) -> &[u8] {
            &self.bytes[..self.data.size as usize]
        }

        /// Back to the start for reading what was written.
        pub fn rewind_all(&mut self) {
            self.data.pCurrentData = self.data.pBaseData;
            self.data.size = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    /// The slot `name` took in the token table (or would take first, if not there yet).
    fn token_of(b: &Buf, name: &CStr) -> u16 {
        b.tokens
            .iter()
            // SAFETY: test tokens are static C strings.
            .position(|&p| !p.is_null() && unsafe { CStr::from_ptr(p) } == name)
            .unwrap_or((hash_string(name) % b.tokens.len() as u32) as usize) as u16
    }

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
        table[1].id = 7;
        let mut d = SAVERESTOREDATA { pTable: table.as_mut_ptr(), tableCount: 2, ..Default::default() };
        let mut sd = SaveData { p: &mut d };
        assert_eq!(sd.entity_index(e), 1);
        assert_eq!(sd.entity_index(0x2000 as *mut edict_t), -1);
        assert_eq!(sd.entity_from_index(7), e, "EntityFromIndex matches the id, not the position");
        assert!(sd.entity_from_index(1).is_null());
        assert_eq!(sd.entity_flags_set(1, FENTTABLE_MOVEABLE | 1), FENTTABLE_MOVEABLE | 1);
        assert_eq!(sd.entity_flags_set(1, 4), FENTTABLE_MOVEABLE | 5);
        assert_eq!(sd.entity_flags_set(2, 1), 0, "one past the end is refused");
        assert_eq!(sd.entity_flags_set(-1, 1), 0);
        assert_eq!(table[1].flags, FENTTABLE_MOVEABLE | 5);
    }

    #[test]
    fn hash_matches_ts() {
        // rotr(h, 4) ^ c over "BASE".
        let mut h = 0u32;
        for c in b"BASE" {
            h = h.rotate_right(4) ^ *c as u32;
        }
        assert_eq!(hash_string(c"BASE"), h);
        // Characters are signed.
        assert_eq!(hash_string(c"\xff"), 0xffff_ffff);
    }

    #[test]
    fn token_hash_probes_linearly() {
        let mut b = Buf::new(64, 8, vec![]);
        let host = FakeHost::default();
        let t = b.sd().token_hash(&host, c"BASE");
        assert_eq!(t as u16, token_of(&b, c"BASE"));
        assert_eq!(b.sd().token_hash(&host, c"BASE"), t, "found again");
        // Fill the slot with another name: the next one probes on.
        b.tokens.iter_mut().for_each(|p| *p = std::ptr::null_mut());
        b.tokens[t as usize] = c"other".as_ptr().cast_mut();
        assert_eq!(b.sd().token_hash(&host, c"BASE"), (t + 1) % 8);
        let mut full = Buf::new(64, 1, vec![]);
        full.tokens[0] = c"x".as_ptr().cast_mut();
        assert_eq!(full.sd().token_hash(&host, c"BASE"), 0);
        assert_eq!(host.log.borrow().last().unwrap(), "alert CSaveRestoreBuffer :: TokenHash() is COMPLETELY FULL!");
    }

    const DESCS: [FieldDesc; 6] = [
        FieldDesc { ty: FieldType::Float, name: c"m_a", count: 1, flags: 0 },
        FieldDesc { ty: FieldType::Time, name: c"m_t", count: 1, flags: 0 },
        FieldDesc { ty: FieldType::String, name: c"m_s", count: 2, flags: FTYPEDESC_GLOBAL },
        FieldDesc { ty: FieldType::ClassPtr, name: c"m_p", count: 1, flags: 0 },
        FieldDesc { ty: FieldType::PositionVector, name: c"m_v", count: 1, flags: 0 },
        FieldDesc { ty: FieldType::Character, name: c"m_c", count: 3, flags: 0 },
    ];

    #[test]
    fn write_fields_byte_layout() {
        let e = 0x1000 as *mut edict_t;
        let mut table = vec![ENTITYTABLE::default(); 3];
        table[2].pent = e;
        table[2].id = 2;
        let mut b = Buf::new(256, 16, table);
        b.data.time = 10.0;
        b.data.fUseLandmark = 1;
        b.data.vecLandmarkOffset = [1.0, 2.0, 3.0];
        let host = FakeHost::default();
        let s1 = host.alloc_string(c"ab");
        let vals = vec![
            Value::Floats(vec![0.0]), // empty: skipped
            Value::Floats(vec![12.5]),
            Value::Ints(vec![s1, 0]),
            Value::Ents(vec![EntRef::of(e)]),
            Value::Vecs(vec![[11.0, 12.0, 13.0]]),
            Value::Bytes(vec![1, 0, 2]),
        ];
        assert!(b.sd().write_fields(&host, c"CTest", &DESCS, &vals));
        let tok = |n: &CStr| token_of(&b, n).to_le_bytes();
        let mut want = Vec::new();
        // count record: 5 non-empty fields
        want.extend(4i16.to_le_bytes());
        want.extend(tok(c"CTest"));
        want.extend(5i32.to_le_bytes());
        // time relative to the save
        want.extend(4i16.to_le_bytes());
        want.extend(tok(c"m_t"));
        want.extend(2.5f32.to_le_bytes());
        // strings with their NULs ("" for 0)
        want.extend(4i16.to_le_bytes());
        want.extend(tok(c"m_s"));
        want.extend(b"ab\0\0");
        // entity as its table index
        want.extend(4i16.to_le_bytes());
        want.extend(tok(c"m_p"));
        want.extend(2i32.to_le_bytes());
        // position relative to the landmark
        want.extend(12i16.to_le_bytes());
        want.extend(tok(c"m_v"));
        for x in [10.0f32, 10.0, 10.0] {
            want.extend(x.to_le_bytes());
        }
        want.extend(3i16.to_le_bytes());
        want.extend(tok(c"m_c"));
        want.extend([1, 0, 2]);
        assert_eq!(b.written(), &want[..]);

        // And back: time and landmark added, strings reallocated, the entity found by id.
        b.rewind_all();
        let out = b.sd().read_fields(&host, c"CTest", &DESCS, false, true).unwrap();
        assert_eq!(out[0], Some(Value::Floats(vec![0.0])));
        assert_eq!(out[1], Some(Value::Floats(vec![12.5])));
        let Some(Value::Ints(s)) = &out[2] else { panic!() };
        assert_eq!(host.string(s[0]), b"ab");
        assert_eq!(s[1], 0, "an empty string is string_t 0");
        assert_eq!(out[3], Some(Value::Ents(vec![EntRef::of(e)])));
        assert_eq!(out[4], Some(Value::Vecs(vec![[11.0, 12.0, 13.0]])));
        assert_eq!(out[5], Some(Value::Bytes(vec![1, 0, 2])));
        assert_eq!(b.data.size as usize, want.len());

        // A global restore leaves FTYPEDESC_GLOBAL fields alone.
        b.rewind_all();
        let out = b.sd().read_fields(&host, c"CTest", &DESCS, true, false).unwrap();
        assert_eq!(out[2], None);
        assert_eq!(out[1], Some(Value::Floats(vec![12.5])));
    }

    #[test]
    fn read_fields_wrong_block_gives_back_the_header() {
        let mut b = Buf::new(64, 16, vec![]);
        let host = FakeHost::default();
        b.sd().write_fields(&host, c"CTest", &DESCS[..1], &[Value::Floats(vec![1.0])]);
        b.rewind_all();
        assert!(b.sd().read_fields(&host, c"COther", &DESCS[..1], false, false).is_none());
        assert_eq!(b.data.size, 0, "rewound by 4");
        assert!(b.sd().read_fields(&host, c"CTest", &DESCS[..1], false, false).is_some());
    }

    #[test]
    fn unknown_function_is_counted_but_not_written() {
        // TS on Linux hands the engine the first word of a g++ 2.95 member pointer (0xffff0000),
        // which has no name: the count says 1, no record follows.
        let d = [FieldDesc { ty: FieldType::Function, name: c"m_pfnThink", count: 1, flags: 0 }];
        let mut b = Buf::new(64, 16, vec![]);
        let host = FakeHost { functions: vec![(0x1234, "Think__7CThing")], ..Default::default() };
        b.sd().write_fields(&host, c"BASE", &d, &[Value::Func(0xffff_0000)]);
        assert_eq!(b.data.size, 8);
        assert_eq!(&b.written()[4..], &1i32.to_le_bytes());
        assert_eq!(host.log.borrow().last().unwrap(), "alert Invalid function pointer in entity!");
        // A named one is written with its NUL and read back through FunctionFromName.
        let mut b = Buf::new(64, 16, vec![]);
        b.sd().write_fields(&host, c"BASE", &d, &[Value::Func(0x1234)]);
        assert_eq!(&b.written()[12..], b"Think__7CThing\0");
        b.rewind_all();
        assert_eq!(b.sd().read_fields(&host, c"BASE", &d, false, false).unwrap()[0], Some(Value::Func(0x1234)));
    }

    #[test]
    fn short_count_reads_on_into_the_next_record() {
        // Saved as 1 element, read with 2: element 1 comes from the bytes after the record, as in
        // TS (here the next record's header).
        let one = [FieldDesc { ty: FieldType::Integer, name: c"m_i", count: 1, flags: 0 }];
        let two = [FieldDesc { ty: FieldType::Integer, name: c"m_i", count: 2, flags: 0 }];
        let mut b = Buf::new(64, 16, vec![]);
        let host = FakeHost::default();
        b.sd().write_fields(&host, c"C", &one, &[Value::Ints(vec![7])]);
        b.sd().write_fields(&host, c"D", &one, &[Value::Ints(vec![0])]);
        b.rewind_all();
        let out = b.sd().read_fields(&host, c"C", &two, false, false).unwrap();
        let next = i32::from_le_bytes([4, 0, token_of(&b, c"D") as u8, (token_of(&b, c"D") >> 8) as u8]);
        assert_eq!(out[0], Some(Value::Ints(vec![7, next])));
    }

    #[test]
    fn fields_are_matched_by_name_from_the_last_one() {
        // Records in another order than the table still land; the search wraps.
        let d = [
            FieldDesc { ty: FieldType::Integer, name: c"a", count: 1, flags: 0 },
            FieldDesc { ty: FieldType::Integer, name: c"b", count: 1, flags: 0 },
        ];
        let rev = [d[1], d[0]];
        let mut b = Buf::new(64, 16, vec![]);
        let host = FakeHost::default();
        b.sd().write_fields(&host, c"C", &rev, &[Value::Ints(vec![2]), Value::Ints(vec![1])]);
        b.rewind_all();
        let out = b.sd().read_fields(&host, c"C", &d, false, false).unwrap();
        assert_eq!(out, vec![Some(Value::Ints(vec![1])), Some(Value::Ints(vec![2]))]);
    }

    #[test]
    fn overflow_stops_writing() {
        let mut b = Buf::new(10, 16, vec![]);
        let host = FakeHost::default();
        let d = [FieldDesc { ty: FieldType::Integer, name: c"m_i", count: 1, flags: 0 }];
        b.sd().write_fields(&host, c"C", &d, &[Value::Ints(vec![1])]);
        assert_eq!(b.data.size, 10);
        assert!(host.log.borrow().iter().any(|l| l == "alert Save/Restore overflow!"));
    }

    #[test]
    fn data_empty_tests_bytes() {
        assert!(!Value::Floats(vec![-0.0]).is_empty(), "-0.0 has a sign bit");
        assert!(Value::Ents(vec![EntRef::NULL]).is_empty());
        assert!(!Value::Ents(vec![EntRef { edict: std::ptr::null_mut(), nonzero: true }]).is_empty());
        assert!(Value::Func(0).is_empty());
        assert!(!Value::Func(0xffff_0000).is_empty());
    }

    #[test]
    fn gsizes_match_ts() {
        let want = [4, 4, 4, 4, 4, 4, 4, 12, 12, 4, 4, 4, 4, 2, 1, 4, 4, 4];
        for (i, w) in want.iter().enumerate() {
            assert_eq!(FieldType::from_raw(i as i32).unwrap().size(), *w);
        }
        assert!(FieldType::from_raw(18).is_none());
    }
}
