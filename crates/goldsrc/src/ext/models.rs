//! Client-loaded studio models drawn as private visible entities (the client's own entities, not
//! the server's): `CL_LoadModel` and `CL_CreateVisibleEntity`. Used by a ruleset's client bullets.

use crate::cl_eng;
use crate::client::EngineCell;
use crate::sys::*;
use crate::util::to_cstring;

/// A model `CL_LoadModel` loaded: the engine's model and its index in the client's model list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadedModel {
    model: *mut model_s,
    pub index: i32,
}

// SAFETY: a handle to a model the engine owns for the level, only used on the client thread.
unsafe impl Send for LoadedModel {}
// SAFETY: as above; the handle is never dereferenced outside the engine's calls.
unsafe impl Sync for LoadedModel {}

/// `CL_LoadModel(name, &index)`: loads (or finds) a model on the client alone, whether or not the
/// server precached it. `None` when the engine can't load it.
pub fn load_model(name: &str) -> Option<LoadedModel> {
    let s = to_cstring(name);
    let mut index = 0;
    let model = cl_eng!(CL_LoadModel(s.as_ptr(), &mut index));
    (!model.is_null()).then_some(LoadedModel { model, index })
}

/// One studio-model entity for this frame's visible list.
#[derive(Clone, Copy, Debug)]
pub struct ModelEntity {
    pub model: LoadedModel,
    pub origin: [f32; 3],
    /// `curstate.angles`, which the studio renderer reads (the entity's own `angles` stay 0).
    pub angles: [f32; 3],
    pub body: i32,
}

/// Slots for [`add_model_entity`]: the engine keeps the pointers handed to
/// `CL_CreateVisibleEntity` until the frame is rendered, so the entities live in static storage.
pub const MODEL_SLOTS: usize = 32;

static MODELS: EngineCell<[cl_entity_t; MODEL_SLOTS]> = EngineCell::new(zeroed_models());

const fn zeroed_models() -> [cl_entity_t; MODEL_SLOTS] {
    // SAFETY: `cl_entity_t` is a C struct of numbers, arrays and raw pointers, for which all-zero
    // bytes are a valid value (the SDK `memset`s it the same way).
    unsafe { std::mem::zeroed() }
}

fn zeroed_entity() -> cl_entity_t {
    // SAFETY: as in `zeroed_models`, all-zero bytes are a valid `cl_entity_t`.
    unsafe { std::mem::zeroed() }
}

/// Fills entity `slot` (`0..MODEL_SLOTS`) from `m` and adds it to this frame's visible list
/// (`CL_CreateVisibleEntity(ET_NORMAL, ...)`). Call from `HUD_CreateEntities`.
pub fn add_model_entity(slot: usize, m: &ModelEntity) {
    if slot >= MODEL_SLOTS {
        return;
    }
    let ent = &mut MODELS.get()[slot];
    *ent = zeroed_entity();
    ent.model = m.model.model;
    ent.curstate.modelindex = m.model.index;
    ent.origin = m.origin;
    ent.curstate.origin = m.origin;
    ent.curstate.angles = m.angles;
    ent.curstate.body = m.body;
    let p: *mut cl_entity_t = ent;
    // ET_NORMAL is 0 (entity_types.h).
    cl_eng!(CL_CreateVisibleEntity(0, p));
}
