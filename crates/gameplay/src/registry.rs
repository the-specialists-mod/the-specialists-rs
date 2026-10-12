//! The plugin registry: ids mapped to `&'static` plugin objects, with the core plugin as the
//! fallback. Each side keeps one for its own plugin type (`ts_server::gameplay`,
//! `ts_client::gameplay`), filled by its library at load (`GameDLLInit`, `Initialize`).

use alloc::vec::Vec;

/// Plugins by id. `P` is a side's plugin trait object (e.g. `dyn ServerPlugin`).
pub struct Registry<P: ?Sized + 'static> {
    core: &'static P,
    entries: Vec<(&'static str, &'static P)>,
}

/// What an id resolved to.
pub struct Resolved<P: ?Sized + 'static> {
    /// The id of the plugin returned: the one asked for, or [`DEFAULT_ID`](crate::DEFAULT_ID).
    pub id: &'static str,
    pub plugin: &'static P,
    /// The id asked for was neither empty nor registered, so the core plugin stands in. The
    /// caller warns on the console (once per value).
    pub unknown: bool,
}

impl<P: ?Sized + 'static> Registry<P> {
    /// A registry whose fallback is `core`. Lookups never fail, even before anything is
    /// registered.
    pub const fn new(core: &'static P) -> Self {
        Registry { core, entries: Vec::new() }
    }

    /// Registers `plugin` under `id`, replacing an earlier plugin with that id; registering
    /// [`DEFAULT_ID`](crate::DEFAULT_ID) replaces the fallback too. Ids must be short decimal
    /// numbers ([`srvsett_value`](crate::srvsett_value)) so the HUD channel can carry them:
    /// returns false, registering nothing, for any other id.
    pub fn add(&mut self, id: &'static str, plugin: &'static P) -> bool {
        if crate::srvsett_value(id).is_none() {
            return false;
        }
        if id == crate::DEFAULT_ID {
            self.core = plugin;
        }
        match self.entries.iter_mut().find(|e| e.0 == id) {
            Some(e) => e.1 = plugin,
            None => self.entries.push((id, plugin)),
        }
        true
    }

    /// The registered ids, in registration order.
    pub fn ids(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.entries.iter().map(|e| e.0)
    }

    /// The plugin for `id` (surrounding whitespace ignored). An empty id (a server or a player
    /// that never set one) is the core plugin, silently; an unknown one is the core plugin with
    /// [`Resolved::unknown`] set.
    pub fn resolve(&self, id: &str) -> Resolved<P> {
        let id = id.trim();
        if let Some(&(id, plugin)) = self.entries.iter().find(|e| e.0 == id) {
            return Resolved { id, plugin, unknown: false };
        }
        Resolved { id: crate::DEFAULT_ID, plugin: self.core, unknown: !id.is_empty() && id != crate::DEFAULT_ID }
    }

    /// [`resolve`](Self::resolve) for raw bytes (a physinfo value); non-UTF-8 is unknown.
    pub fn resolve_bytes(&self, id: &[u8]) -> Resolved<P> {
        match core::str::from_utf8(id) {
            Ok(s) => self.resolve(s),
            Err(_) => Resolved { id: crate::DEFAULT_ID, plugin: self.core, unknown: true },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    trait Named: Sync {
        fn name(&self) -> &'static str;
    }
    struct A;
    struct B;
    impl Named for A {
        fn name(&self) -> &'static str {
            "a"
        }
    }
    impl Named for B {
        fn name(&self) -> &'static str {
            "b"
        }
    }
    static AA: A = A;
    static BB: B = B;

    #[test]
    fn resolves_ids_and_falls_back_to_the_core() {
        let mut r: Registry<dyn Named> = Registry::new(&AA);
        assert_eq!(r.resolve("30").plugin.name(), "a", "nothing registered: the core");
        assert!(r.add("30", &AA));
        assert!(r.add("12", &BB));
        assert!(!r.add("x1", &BB), "ids are numbers");
        let got = r.resolve(" 12 ");
        assert_eq!((got.id, got.plugin.name(), got.unknown), ("12", "b", false));
        let got = r.resolve("");
        assert_eq!((got.id, got.plugin.name(), got.unknown), ("30", "a", false));
        let got = r.resolve("99");
        assert_eq!((got.id, got.plugin.name(), got.unknown), ("30", "a", true));
        assert!(r.resolve_bytes(&[0xff]).unknown);
        assert_eq!(r.ids().collect::<Vec<_>>(), ["30", "12"]);
    }
}
