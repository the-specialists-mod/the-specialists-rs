//! `PluginApi::entity_key` (version 6): the map keys an entity was given that aren't `entvars_t`
//! fields. The port's classes parse such keys (`master`, `wait`, `tsweaponid`, ...) into their Rust
//! objects, or drop them, so nothing else can read them back; RCBot reads `master` to know which
//! buttons and doors a `multisource` locks. `entities::dispatch_key_value` records every key the
//! entvars table didn't take, by entity index and serial number, and freeing the entity forgets
//! them.

use goldsrc::Global;
use goldsrc::edict::EdictPtr;
use goldsrc::server::engine;
use goldsrc::sys::edict_t;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::ffi::{CStr, CString, c_int};

/// The keys of one entity, in the order given.
#[derive(Default)]
pub struct Keys {
    /// Entity index → (serial number, keys).
    map: BTreeMap<c_int, (c_int, Vec<(CString, CString)>)>,
}

impl Keys {
    /// Records `key` = `value` for the entity at `index` with `serial`; a new serial starts over.
    pub fn put(&mut self, index: c_int, serial: c_int, key: &CStr, value: &CStr) {
        let e = self.map.entry(index).or_insert_with(|| (serial, Vec::new()));
        if e.0 != serial {
            *e = (serial, Vec::new());
        }
        e.1.push((key.to_owned(), value.to_owned()));
    }

    /// The last value given for `key` (map keys are case-sensitive in the SDK's `KeyValue`s, but
    /// mappers' tools vary: an exact match wins, else one ignoring ASCII case).
    pub fn get(&self, index: c_int, serial: c_int, key: &CStr) -> Option<&CStr> {
        let (s, keys) = self.map.get(&index)?;
        if *s != serial {
            return None;
        }
        let exact = keys.iter().rev().find(|(k, _)| k.as_c_str() == key);
        let any = || keys.iter().rev().find(|(k, _)| k.as_bytes().eq_ignore_ascii_case(key.to_bytes()));
        exact.or_else(any).map(|(_, v)| v.as_c_str())
    }

    /// Drops the keys of the entity at `index`.
    pub fn forget(&mut self, index: c_int) {
        self.map.remove(&index);
    }
}

static KEYS: Global<RefCell<Keys>> = Global(RefCell::new(Keys { map: BTreeMap::new() }));

/// `DispatchKeyValue` gave `e` a key the entvars table didn't take.
pub fn record(e: *mut edict_t, key: &CStr, value: &CStr) {
    if e.is_null() {
        return;
    }
    KEYS.0.borrow_mut().put(engine().index_of(e), e.serial_number(), key, value);
}

/// The entity's private data is being freed: its keys go.
pub fn forget(e: *mut edict_t) {
    if !e.is_null() {
        KEYS.0.borrow_mut().forget(engine().index_of(e));
    }
}

/// `PluginApi::entity_key`: the value, `NOT_PLAYER` for an index that isn't a game entity, or
/// `UNSUPPORTED` when it wasn't given the key.
pub fn entity_key(entity: c_int, key: &CStr) -> Result<CString, c_int> {
    let eng = engine();
    if entity < 0 || entity >= eng.globals().maxEntities {
        return Err(ts_plugin_api::NOT_PLAYER);
    }
    let e = eng.entity_of_index(entity);
    if e.is_null() || e.is_free() || !e.has_private_data() {
        return Err(ts_plugin_api::NOT_PLAYER);
    }
    KEYS.0.borrow().get(entity, e.serial_number(), key).map(CStr::to_owned).ok_or(ts_plugin_api::UNSUPPORTED)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_by_entity_and_serial() {
        let mut k = Keys::default();
        k.put(40, 7, c"master", c"ms_door");
        k.put(40, 7, c"wait", c"3");
        k.put(40, 7, c"master", c"ms_door2");
        assert_eq!(k.get(40, 7, c"master"), Some(c"ms_door2"), "the last one given");
        assert_eq!(k.get(40, 7, c"MASTER"), Some(c"ms_door2"), "any case when nothing matches exactly");
        assert_eq!(k.get(40, 8, c"master"), None, "another entity in the slot");
        assert_eq!(k.get(41, 7, c"master"), None);
        assert_eq!(k.get(40, 7, c"target"), None);
        k.put(40, 8, c"wait", c"1");
        assert_eq!(k.get(40, 8, c"master"), None, "a new serial starts over");
        k.forget(40);
        assert_eq!(k.get(40, 8, c"wait"), None);
    }
}
