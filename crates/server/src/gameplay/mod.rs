//! The server's gameplay plugins (`docs/implementation/gameplay-plugins.md`): the registry the
//! server library fills at `GameDLLInit`, the active plugin, and where each part of the server
//! finds its rules.
//!
//! * Movement (`pfnPM_Move`): [`pm_rules`], from the player's physinfo key `gp`, as the client
//!   predicts it.
//! * The gun ([`weapons::host`](crate::weapons::host)): [`gun_rules`], from the same physinfo key,
//!   once per command.
//! * Everything else (the frame, rules, scoring): [`active`], the plugin `ts_gameplay` selected,
//!   followed once a frame by [`frame`].
//!
//! Changing `ts_gameplay` mid-game resets the game to a brand-new state ([`reset`]) and then
//! activates the new plugin. Physinfo `gp` and `SrvSett` 4 carry the id to the clients.

mod reset;
pub mod server;

pub use server::ServerRules;

use crate::rules::cvars;
use goldsrc::Global;
use goldsrc::pmove::PlayerMoveExt;
use goldsrc::server::engine;
use goldsrc::sys::{edict_t, playermove_t};
use shared::gameplay::{CORE, GunRules, PHYSINFO_KEY_C, PmRules};
use std::cell::{Cell, RefCell};
use std::sync::{PoisonError, RwLock};
use ts_gameplay::Registry;

/// A gameplay plugin as the server sees it: its shared hooks and the server's. Each part is its
/// own object, so a plugin's server half can live in a package of its own (the server library
/// links it; the client library never does).
pub struct ServerPlugin {
    pub pm: &'static dyn PmRules,
    pub gun: &'static dyn GunRules,
    pub server: &'static dyn ServerRules,
}

impl ServerRules for shared::gameplay::Core {}

/// The core plugin (TS 3.0) on the server.
pub static CORE_PLUGIN: ServerPlugin = ServerPlugin { pm: &CORE, gun: &CORE, server: &CORE };

static REGISTRY: RwLock<Registry<ServerPlugin>> = RwLock::new(Registry::new(&CORE_PLUGIN));

/// A plugin and its id.
type Active = (&'static str, &'static ServerPlugin);

/// The active plugin: `None` before the first map installs its rules (the core plugin stands in).
static ACTIVE: Global<Cell<Option<Active>>> = Global(Cell::new(None));

/// A switch's reset waiting for its second half ([`reset::finish`]) from this game time on.
static PENDING: Global<Cell<Option<reset::Pending>>> = Global(Cell::new(None));

/// The last unknown id warned about, so each is reported once.
static WARNED: Global<RefCell<String>> = Global(RefCell::new(String::new()));

/// Registers a plugin (the server library's `GameDLLInit`). False for an id that isn't a short
/// number.
pub fn register(id: &'static str, plugin: &'static ServerPlugin) -> bool {
    REGISTRY.write().unwrap_or_else(PoisonError::into_inner).add(id, plugin)
}

/// The registered ids, for the cvar's help and warnings.
pub fn ids() -> Vec<&'static str> {
    REGISTRY.read().unwrap_or_else(PoisonError::into_inner).ids().collect()
}

/// The plugin for `id`; an unknown one is the core plugin, with a console warning the first
/// time.
fn resolve(id: &str) -> Active {
    let r = REGISTRY.read().unwrap_or_else(PoisonError::into_inner).resolve(id);
    if r.unknown {
        let id = id.trim();
        let mut w = WARNED.0.borrow_mut();
        if *w != id {
            *w = id.to_owned();
            engine().print(&format!(
                "[ts] gameplay: unknown ruleset \"{id}\" (registered: {}); using \"{}\"\n",
                ids().join(", "),
                ts_gameplay::DEFAULT_ID
            ));
        }
    }
    (r.id, r.plugin)
}

/// The active plugin: what the frame, the rules and scoring use.
pub fn active() -> &'static ServerPlugin {
    ACTIVE.0.get().map_or(&CORE_PLUGIN, |a| a.1)
}

/// The active plugin's id: what physinfo `gp` and `SrvSett` 4 carry.
pub fn active_id() -> &'static str {
    ACTIVE.0.get().map_or(ts_gameplay::DEFAULT_ID, |a| a.0)
}

/// The movement rules for one `pfnPM_Move`, from the moving player's physinfo (the key the
/// client predicts with).
pub fn pm_rules(pm: &playermove_t) -> &'static dyn PmRules {
    resolve(&pm.physinfo_value(PHYSINFO_KEY_C)).1.pm
}

/// The gun rules for one player's command, from its physinfo
/// ([`GunHost::rules`](shared::weapons::GunHost::rules)).
pub fn gun_rules(e: *mut edict_t) -> &'static dyn GunRules {
    resolve(&engine().physics_key(e, PHYSINFO_KEY_C)).1.gun
}

/// Sets a client's physinfo `gp` to the active plugin's id: at every spawn, next to `mj`/`hl`,
/// and for every client on a switch.
pub fn set_physinfo(e: *mut edict_t) {
    let id = goldsrc::util::to_cstring(active_id());
    engine().set_physics_key(e, PHYSINFO_KEY_C, &id);
}

/// The `SrvSett` 4 value for the active plugin.
pub fn srvsett_value() -> i16 {
    ts_gameplay::srvsett_value(active_id()).unwrap_or(30)
}

/// A map has installed its rules (`rules::install`, from worldspawn): the plugin `ts_gameplay`
/// selects becomes active on the brand-new game, with no reset.
pub fn map_start() {
    PENDING.0.set(None);
    let (id, plugin) = resolve(&cvars::gameplay());
    set_active(id, plugin);
    plugin.server.on_activate();
}

/// Makes a plugin the active one, with its weapon tables
/// ([`GunRules::tables`](shared::gameplay::GunRules::tables)).
fn set_active(id: &'static str, plugin: &'static ServerPlugin) {
    ACTIVE.0.set(Some((id, plugin)));
    shared::weapons::table::set_ruleset_tables(plugin.gun.tables());
}

/// The weapon tables of every registered plugin, the core plugin's first and each once: what the
/// map precaches (`weapons::precache_weapon_assets`), so any of them can become active mid-map.
pub fn ruleset_tables() -> Vec<&'static shared::weapons::Tables> {
    let r = REGISTRY.read().unwrap_or_else(PoisonError::into_inner);
    let mut out: Vec<&'static shared::weapons::Tables> = vec![CORE_PLUGIN.gun.tables()];
    for id in r.ids() {
        let t = r.resolve(id).plugin.gun.tables();
        if !out.iter().any(|o| std::ptr::eq(*o, t)) {
            out.push(t);
        }
    }
    out
}

/// Once a frame (`StartFrame`, while rules are installed): `ts_gameplay` is read, and a change of
/// plugin resets the game to brand new under the new one ([`reset`]), then activates it.
///
/// The reset takes two steps at least 0.5 s apart, because the engines don't hand out an edict
/// again for 0.5 s after freeing it (FWGS `SV_AllocEdict`, ReHLDS `ED_Alloc`): the switch frame
/// clears the players and removes the entities ([`reset::begin`]), and a later frame spawns the
/// map again and the players with it ([`reset::finish`]). Returns false in between, when the
/// rules' Think must not run: there is no game.
pub fn frame() -> bool {
    let now = engine().time();
    if let Some(p) = PENDING.0.get() {
        if now < p.at {
            return false;
        }
        PENDING.0.set(None);
        reset::finish(p);
        active().server.on_activate();
        return true;
    }
    let (id, plugin) = resolve(&cvars::gameplay());
    let Some((old, _)) = ACTIVE.0.get() else {
        map_start();
        return true;
    };
    if old == id {
        return true;
    }
    set_active(id, plugin);
    PENDING.0.set(Some(reset::begin(old, id, now)));
    false
}
