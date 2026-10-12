//! Classes for the entity hooks: one id per Rust type, the port's stand-in for a C++ vtable.
//! Classnames linked to the same type (`func_door`, `func_water`) share it, as hamsandwich's
//! hooks on one vtable cover every classname that creates that class.
//!
//! Ids are handed out once per server process: first to the linked types, in link order
//! ([`linked`]), then to any other type the first time an entity of it is asked about (entities
//! the game creates under a classname nothing links). They never change while the server runs.

use goldsrc::Global;
use std::any::TypeId;
use std::cell::RefCell;
use std::ffi::{CStr, c_int};

/// Every linked classname with its type, from the modules' `link_entities!`.
pub(crate) fn linked() -> Vec<(&'static str, TypeId)> {
    let mut v = crate::entities::stock::linked_classes();
    v.extend(crate::entities::ts::linked_classes());
    v.extend(crate::entities::stock::blood::linked_classes());
    v.extend(crate::entities::stock::monsters::linked_classes());
    v.extend(crate::entities::stock::xen::linked_classes());
    v.extend(crate::player::linked_classes());
    v.extend(crate::weapons::linked_classes());
    v
}

/// The ids: `types[id - 1]`.
pub(crate) struct Registry {
    types: Vec<TypeId>,
    names: Vec<(&'static str, c_int)>,
}

impl Registry {
    pub(crate) fn new(linked: Vec<(&'static str, TypeId)>) -> Self {
        let mut r = Registry { types: Vec::new(), names: Vec::with_capacity(linked.len()) };
        for (name, ty) in linked {
            let id = r.id_of(ty);
            r.names.push((name, id));
        }
        r
    }

    /// The id of `ty`, given one now if it has none yet.
    pub(crate) fn id_of(&mut self, ty: TypeId) -> c_int {
        let i = self.types.iter().position(|&t| t == ty).unwrap_or_else(|| {
            self.types.push(ty);
            self.types.len() - 1
        });
        i as c_int + 1
    }

    /// The class of a linked classname (case-sensitive, as the engine's export lookup is).
    pub(crate) fn class_id(&self, name: &CStr) -> Option<c_int> {
        let name = name.to_str().ok()?;
        self.names.iter().find(|(n, _)| *n == name).map(|&(_, id)| id)
    }

    pub(crate) fn type_of(&self, class: c_int) -> Option<TypeId> {
        self.types.get(usize::try_from(class).ok()?.checked_sub(1)?).copied()
    }
}

static REGISTRY: Global<RefCell<Option<Registry>>> = Global(RefCell::new(None));

fn with<R>(f: impl FnOnce(&mut Registry) -> R) -> R {
    let mut r = REGISTRY.0.borrow_mut();
    f(r.get_or_insert_with(|| Registry::new(linked())))
}

pub(crate) fn class_id(name: &CStr) -> Option<c_int> {
    with(|r| r.class_id(name))
}

pub(crate) fn class_of_type(ty: TypeId) -> c_int {
    with(|r| r.id_of(ty))
}

pub(crate) fn type_of_class(class: c_int) -> Option<TypeId> {
    with(|r| r.type_of(class))
}
