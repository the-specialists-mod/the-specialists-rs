//! The plugin registry: ids mapped to `&'static` plugin objects, with the first one registered as
//! the default (the fallback for an empty or unknown id). Each side keeps one for its own plugin
//! type (`ts_server::gameplay`, `ts_client::gameplay`), filled by its library at load
//! (`GameDLLInit`, `Initialize`), before anything resolves an id.

use alloc::vec::Vec;

/// Plugins by id. `P` is a side's plugin trait object (e.g. `dyn ServerPlugin`).
pub struct Registry<P: ?Sized + 'static> {
    /// In registration order; the first is the default.
    entries: Vec<(&'static str, &'static P)>,
}

/// What an id resolved to.
pub struct Resolved<P: ?Sized + 'static> {
    /// The id of the plugin returned: the one asked for, or the default's.
    pub id: &'static str,
    pub plugin: &'static P,
    /// The id asked for was neither empty nor registered, so the default plugin stands in. The
    /// caller warns on the console (once per value).
    pub unknown: bool,
}

impl<P: ?Sized + 'static> Default for Registry<P> {
    fn default() -> Self {
        Self::new()
    }
}

impl<P: ?Sized + 'static> Registry<P> {
    /// An empty registry: lookups find nothing until the first [`add`](Self::add).
    pub const fn new() -> Self {
        Registry { entries: Vec::new() }
    }

    /// Registers `plugin` under `id`, replacing an earlier plugin with that id (in its place, so
    /// the default stays the first id registered). Ids must be short decimal numbers
    /// ([`srvsett_value`](crate::srvsett_value)) so the HUD channel can carry them: returns false,
    /// registering nothing, for any other id.
    pub fn add(&mut self, id: &'static str, plugin: &'static P) -> bool {
        if crate::srvsett_value(id).is_none() {
            return false;
        }
        match self.entries.iter_mut().find(|e| e.0 == id) {
            Some(e) => e.1 = plugin,
            None => self.entries.push((id, plugin)),
        }
        true
    }

    /// The default plugin and its id: the first registered. `None` while the registry is empty.
    pub fn fallback(&self) -> Option<(&'static str, &'static P)> {
        self.entries.first().copied()
    }

    /// The registered ids, in registration order.
    pub fn ids(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.entries.iter().map(|e| e.0)
    }

    /// The plugin for `id` (surrounding whitespace ignored). An empty id (a server or a player
    /// that never set one) is the default plugin, silently; an unknown one is the default plugin
    /// with [`Resolved::unknown`] set. `None` only while the registry is empty.
    pub fn resolve(&self, id: &str) -> Option<Resolved<P>> {
        let id = id.trim();
        if let Some(&(id, plugin)) = self.entries.iter().find(|e| e.0 == id) {
            return Some(Resolved { id, plugin, unknown: false });
        }
        let (default, plugin) = self.fallback()?;
        Some(Resolved { id: default, plugin, unknown: !id.is_empty() })
    }

    /// [`resolve`](Self::resolve) for raw bytes (a physinfo value); non-UTF-8 is unknown.
    pub fn resolve_bytes(&self, id: &[u8]) -> Option<Resolved<P>> {
        match core::str::from_utf8(id) {
            Ok(s) => self.resolve(s),
            Err(_) => self.fallback().map(|(id, plugin)| Resolved { id, plugin, unknown: true }),
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
    fn resolves_ids_and_falls_back_to_the_first_registered() {
        let mut r: Registry<dyn Named> = Registry::new();
        assert!(r.resolve("30").is_none() && r.fallback().is_none(), "nothing registered");
        assert!(r.add("30", &AA));
        assert!(r.add("12", &BB));
        assert!(!r.add("x1", &BB), "ids are numbers");
        let got = r.resolve(" 12 ").unwrap();
        assert_eq!((got.id, got.plugin.name(), got.unknown), ("12", "b", false));
        let got = r.resolve("").unwrap();
        assert_eq!((got.id, got.plugin.name(), got.unknown), ("30", "a", false));
        let got = r.resolve("99").unwrap();
        assert_eq!((got.id, got.plugin.name(), got.unknown), ("30", "a", true));
        assert!(r.resolve_bytes(&[0xff]).unwrap().unknown);
        assert_eq!(r.ids().collect::<Vec<_>>(), ["30", "12"]);
        assert!(r.add("30", &BB), "a re-registration keeps its place");
        assert_eq!(r.fallback().map(|d| (d.0, d.1.name())), Some(("30", "b")));
    }

    #[test]
    fn the_default_is_whichever_ruleset_registers_first() {
        let mut r: Registry<dyn Named> = Registry::new();
        assert!(r.add("21", &BB));
        assert!(r.add("30", &AA));
        let got = r.resolve("").unwrap();
        assert_eq!((got.id, got.plugin.name()), ("21", "b"));
        assert_eq!(r.resolve("7").unwrap().id, "21");
    }
}
