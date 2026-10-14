//! Port addition (issues #100, #104): a change to `ts_gameplay` or `ts_bot_backend` reloads the
//! map. Both are read only at a map start ([`crate::gameplay::map_select`],
//! [`crate::bot_backend::map_start`]), so when either selects something other than what the map
//! runs, from any source (the console, rcon, the server config, a script), the server changes
//! level to the same map and the new map start reads them as always.
//!
//! * **Debounced.** A difference seen at one frame must still be there at the next before the map
//!   is reloaded, so a config that sets both cvars, or sets one and then back, reloads once or not
//!   at all; the reload reads the values of that later frame.
//! * **Resolved values.** What is compared is the ruleset id and the backend the values resolve
//!   to, with their fallbacks (an unknown id is TS 3.0, an unknown backend native), so a value that
//!   falls back to what runs does nothing, and one that can't be loaded can't reload in a loop.
//! * **One reload per map.** After the change of level is asked for, nothing more until the next
//!   map start.
//! * **No loop through a map start.** If the map that a reload started runs something other than
//!   what was asked for (`game.cfg`, which runs inside the map start, set the value back), that
//!   choice isn't reloaded for again until the values change; the console says why.
//!
//! The server config needs nothing of its own: the engines run it once the map has loaded, so a
//! value it sets is a change like any other, and when it runs again on the reloaded map it sets
//! what that map already runs.

use crate::{bot_backend, gameplay};
use goldsrc::Global;
use goldsrc::server::engine;
use std::cell::RefCell;

/// What a map runs, or what the cvars select: the resolved ruleset id and bot backend name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    pub gameplay: String,
    pub backend: String,
}

/// The watcher's state across frames and map starts.
#[derive(Debug, Default)]
struct Watcher {
    /// A difference was seen at the last frame.
    pending: bool,
    /// The reload this map asked for (until the next map start).
    requested: Option<Selection>,
    /// A selection a map start set back: not reloaded for while the cvars still select it.
    blocked: Option<Selection>,
}

impl Watcher {
    /// A map started, running `running`. Returns the reload's target if the map start didn't
    /// honour it (it becomes blocked).
    fn map_start(&mut self, running: &Selection) -> Option<Selection> {
        self.pending = false;
        let req = self.requested.take()?;
        if req == *running {
            return None;
        }
        self.blocked = Some(req.clone());
        Some(req)
    }

    /// One frame: true if the map is to be reloaded now for `wanted`.
    fn frame(&mut self, running: &Selection, wanted: &Selection) -> bool {
        if wanted == running {
            self.pending = false;
            self.blocked = None;
            return false;
        }
        if self.requested.is_some() || self.blocked.as_ref() == Some(wanted) {
            self.pending = false;
            return false;
        }
        if !self.pending {
            self.pending = true;
            return false;
        }
        self.pending = false;
        self.blocked = None;
        self.requested = Some(wanted.clone());
        true
    }
}

static WATCHER: Global<RefCell<Watcher>> = Global(RefCell::new(Watcher { pending: false, requested: None, blocked: None }));

/// What the map runs: the active ruleset and backend.
fn running() -> Selection {
    Selection { gameplay: gameplay::active_id().to_owned(), backend: bot_backend::active().name().to_owned() }
}

/// What the cvars select now (unknown values warn once and fall back, as at a map start).
fn wanted() -> Selection {
    Selection { gameplay: gameplay::wanted_id().to_owned(), backend: bot_backend::wanted().name().to_owned() }
}

/// A map start, after the ruleset and the backend were chosen (`rules::install`).
pub fn map_start() {
    let Some(asked) = WATCHER.0.borrow_mut().map_start(&running()) else { return };
    let now = running();
    engine().print(&format!(
        "[ts] the map start set ruleset {} / bot backend {} (game.cfg?), not {} / {}; not reloading for that again\n",
        now.gameplay, now.backend, asked.gameplay, asked.backend
    ));
}

/// Once a frame (`StartFrame`, while rules are installed): reloads the map if `ts_gameplay` or
/// `ts_bot_backend` selected something else at this frame and the last.
pub fn frame() {
    let (running, wanted) = (running(), wanted());
    if !WATCHER.0.borrow_mut().frame(&running, &wanted) {
        return;
    }
    let eng = engine();
    let map = eng.string(eng.globals().mapname).to_string_lossy().into_owned();
    if wanted.gameplay != running.gameplay {
        eng.print(&format!("[ts] gameplay: ruleset {}: reloading {map}\n", wanted.gameplay));
    }
    if wanted.backend != running.backend {
        eng.print(&format!("[ts] bots: backend {}: reloading {map}\n", wanted.backend));
    }
    eng.change_level(&map);
}

#[cfg(test)]
mod tests {
    use super::{Selection, Watcher};

    fn sel(gameplay: &str, backend: &str) -> Selection {
        Selection { gameplay: gameplay.into(), backend: backend.into() }
    }

    #[test]
    fn a_change_reloads_once_at_the_next_frame() {
        let mut w = Watcher::default();
        let run = sel("30", "native");
        assert!(!w.frame(&run, &run));
        assert!(!w.frame(&run, &sel("21", "native")), "seen once: wait a frame");
        assert!(w.frame(&run, &sel("21", "native")), "still there: reload");
        assert!(!w.frame(&run, &sel("21", "native")), "asked for already");
        assert!(!w.frame(&run, &sel("21", "test")), "one reload per map");
        assert_eq!(w.map_start(&sel("21", "native")), None, "the new map runs it");
        assert!(!w.frame(&sel("21", "native"), &sel("21", "native")));
    }

    #[test]
    fn both_cvars_or_a_change_and_back() {
        let run = sel("30", "native");
        let mut w = Watcher::default();
        assert!(!w.frame(&run, &sel("21", "test")));
        assert!(w.frame(&run, &sel("21", "test")), "both at once: one reload");

        let mut w = Watcher::default();
        assert!(!w.frame(&run, &sel("21", "native")));
        assert!(!w.frame(&run, &run), "set back before the next frame: no reload");
        assert!(!w.frame(&run, &run));
        assert!(!w.frame(&run, &sel("30", "test")), "a new change waits a frame again");
        assert!(w.frame(&run, &sel("21", "test")), "and reloads for the latest values");
    }

    #[test]
    fn a_map_start_that_sets_it_back_doesnt_loop() {
        let run = sel("30", "native");
        let mut w = Watcher::default();
        assert!(!w.frame(&run, &sel("21", "native")));
        assert!(w.frame(&run, &sel("21", "native")));
        // game.cfg set ts_gameplay 30 at the map start; the server config sets 21 again.
        assert_eq!(w.map_start(&run), Some(sel("21", "native")));
        for _ in 0..5 {
            assert!(!w.frame(&run, &sel("21", "native")), "blocked");
        }
        assert_eq!(w.map_start(&run), None, "a later map start keeps the block");
        assert!(!w.frame(&run, &sel("21", "native")));
        assert!(!w.frame(&run, &sel("21", "test")), "another choice reloads again");
        assert!(w.frame(&run, &sel("21", "test")));
    }
}
