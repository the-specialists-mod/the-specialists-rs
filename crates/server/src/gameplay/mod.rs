//! The server's gameplay plugins (`docs/implementation/gameplay-plugins.md`): the registry the
//! server library fills at `GameDLLInit`, the active plugin, and where each part of the server
//! finds its rules.
//!
//! * Movement (`pfnPM_Move`): [`pm_rules`], from the player's physinfo key `gp`, as the client
//!   predicts it.
//! * The gun ([`weapons::host`](crate::weapons::host)): [`gun_rules`], from the same physinfo key,
//!   once per command.
//! * Everything else (the frame, rules, scoring): [`active`], the plugin `ts_gameplay` selected
//!   when the map started.
//!
//! `ts_gameplay` is latched at a map start ([`map_select`]), whose new game is the fresh state.
//! A change mid-map, from any source, reloads the map ([`crate::reload`]). Physinfo `gp` and
//! `SrvSett` 4 carry the id to the clients.

pub mod server;

pub use server::{ConnectingClient, ServerRules};

use crate::rules::cvars;
use goldsrc::Global;
use goldsrc::pmove::PlayerMoveExt;
use goldsrc::server::engine;
use goldsrc::sys::{edict_t, playermove_t};
use shared::gameplay::{GunRules, PHYSINFO_KEY_C, PmRules};
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

/// Every hook's default body, for this crate's tests of the code behind them (they move to
/// the 3.0 plugin with the bodies, issue #200).
#[cfg(test)]
pub(crate) struct Defaults;

#[cfg(test)]
impl ServerRules for Defaults {}

/// [`Defaults`] as a plugin, with the 3.0 plugin's movement and gun rules.
#[cfg(test)]
pub(crate) static DEFAULTS: ServerPlugin = ServerPlugin { pm: &ts30_gameplay::TS30, gun: &ts30_gameplay::TS30, server: &Defaults };

/// The registered plugins. The first one the server library registers is the default: the
/// fallback for an empty or unknown id, and the plugin the server runs before its first map.
static REGISTRY: RwLock<Registry<ServerPlugin>> = RwLock::new(Registry::new());

/// A plugin and its id.
type Active = (&'static str, &'static ServerPlugin);

/// The active plugin: `None` before the first map installs its rules (the default stands in).
static ACTIVE: Global<Cell<Option<Active>>> = Global(Cell::new(None));

/// The last unknown id warned about, so each is reported once.
static WARNED: Global<RefCell<String>> = Global(RefCell::new(String::new()));

/// Registers a plugin (the server library's `GameDLLInit`, before the rules' `game_init`), the
/// default ruleset first. False for an id that isn't a short number. The plugins' own cvars
/// ([`ServerRules::cvars`]) are registered later, with the server's ([`register_cvars`]).
pub fn register(id: &'static str, plugin: &'static ServerPlugin) -> bool {
    REGISTRY.write().unwrap_or_else(PoisonError::into_inner).add(id, plugin)
}

/// Every registered plugin's own cvars ([`ServerRules::cvars`]), in registration order, after the
/// server's (`rules::cvars::register`), each unless the name exists already.
pub fn register_cvars() {
    let r = REGISTRY.read().unwrap_or_else(PoisonError::into_inner);
    for id in r.ids() {
        if let Some(p) = r.resolve(id) {
            cvars::register_extra(p.plugin.server.cvars());
        }
    }
}

/// The default plugin and its id (the first registered).
fn fallback() -> Active {
    REGISTRY.read().unwrap_or_else(PoisonError::into_inner).fallback().unwrap_or_else(unregistered)
}

/// The default ruleset's id: `ts_gameplay`'s default value.
pub fn default_id() -> &'static str {
    fallback().0
}

/// What stands in while nothing is registered: nothing in the library, which registers its rulesets
/// at `GameDLLInit` before any game code runs; [`Defaults`] in this crate's tests.
fn unregistered() -> Active {
    #[cfg(test)]
    return ("30", &DEFAULTS);
    #[cfg(not(test))]
    panic!("no gameplay ruleset registered (server-dll registers them at GameDLLInit)");
}

/// The registered ids, for the cvar's help and warnings.
pub fn ids() -> Vec<&'static str> {
    REGISTRY.read().unwrap_or_else(PoisonError::into_inner).ids().collect()
}

/// The plugin for `id`; an unknown one is the default plugin, with a console warning the first
/// time.
fn resolve(id: &str) -> Active {
    let Some(r) = REGISTRY.read().unwrap_or_else(PoisonError::into_inner).resolve(id) else { return unregistered() };
    if r.unknown {
        let id = id.trim();
        let mut w = WARNED.0.borrow_mut();
        if *w != id {
            *w = id.to_owned();
            engine().print(&format!("[ts] gameplay: unknown ruleset \"{id}\" (registered: {}); using \"{}\"\n", ids().join(", "), r.id));
        }
    }
    (r.id, r.plugin)
}

/// The ruleset id `ts_gameplay` selects now (an unknown one is the default plugin's, with the
/// warning): what [`crate::reload`] compares with [`active_id`].
pub fn wanted_id() -> &'static str {
    resolve(&cvars::gameplay()).0
}

/// The active plugin: what the frame, the rules and scoring use.
pub fn active() -> &'static ServerPlugin {
    ACTIVE.0.get().unwrap_or_else(fallback).1
}

/// The active plugin's id: what physinfo `gp` and `SrvSett` 4 carry.
pub fn active_id() -> &'static str {
    ACTIVE.0.get().unwrap_or_else(fallback).0
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

/// Sets a client's physinfo `gp` to the active plugin's id: at every spawn, next to `mj`/`hl`.
pub fn set_physinfo(e: *mut edict_t) {
    let id = goldsrc::util::to_cstring(active_id());
    engine().set_physics_key(e, PHYSINFO_KEY_C, &id);
}

/// `ClientConnect`: the active ruleset's verdict on the connecting client
/// ([`ServerRules::refuse_client`]), from its edict and userinfo. `Some(reason)` refuses it.
pub fn refuse_client(e: *mut edict_t) -> Option<&'static str> {
    use goldsrc::edict::EdictPtr;
    let eng = engine();
    let fake = !e.is_null() && e.v().flags & goldsrc::sys::FL_FAKECLIENT as i32 != 0;
    let key = |k: &std::ffi::CStr| if e.is_null() { String::new() } else { eng.info_key(e, k).to_string_lossy().into_owned() };
    let (bot, version) = (key(c"*bot"), key(shared::protocol::port::USERINFO_KEY_C));
    let hltv = [key(c"*hltv"), key(c"hltv")].into_iter().find(|v| !v.is_empty() && v != "0").unwrap_or_default();
    let client = ConnectingClient::classify(fake, &bot, &hltv, &version);
    let reason = active().server.refuse_client(client)?;
    eng.print(&format!("[ts] gameplay: refused a client in ruleset {} ({client:?}): {reason}\n", active_id()));
    Some(reason)
}

/// The `SrvSett` 4 value for the active plugin.
pub fn srvsett_value() -> i16 {
    // Registered ids are numbers (`Registry::add`).
    ts_gameplay::srvsett_value(active_id()).unwrap_or_default()
}

/// A map is installing its rules (`rules::install`, from worldspawn, after `game.cfg`): the plugin
/// `ts_gameplay` selects becomes active for the whole map, before the rules are built
/// ([`ServerRules::install`]).
pub fn map_select() {
    let (id, plugin) = resolve(&cvars::gameplay());
    set_active(id, plugin);
    engine().print(&format!("[ts] gameplay: ruleset {id} for this map\n"));
}

/// A map has installed its rules ([`map_select`] chose the plugin): the plugin sets up its state.
pub fn map_start() {
    active().server.on_activate();
}

/// Makes a plugin the active one, with its weapon tables
/// ([`GunRules::tables`](shared::gameplay::GunRules::tables)).
fn set_active(id: &'static str, plugin: &'static ServerPlugin) {
    ACTIVE.0.set(Some((id, plugin)));
    shared::weapons::table::set_ruleset_tables(plugin.gun.tables());
}

/// Every registered plugin's own precaches ([`ServerRules::precache`]), each once, at the map's
/// world precache after the server's own.
pub fn precache_rulesets() {
    let r = REGISTRY.read().unwrap_or_else(PoisonError::into_inner);
    let mut done: Vec<*const ServerPlugin> = Vec::new();
    for id in r.ids() {
        let Some(p) = r.resolve(id).map(|r| r.plugin) else { continue };
        if !done.contains(&(p as *const ServerPlugin)) {
            done.push(p);
            p.server.precache();
        }
    }
}

/// The weapon tables of every registered plugin, in registration order (the default's first)
/// and each once: what the map precaches (`weapons::precache_weapon_assets`), so the map's
/// precache list is the same whichever ruleset runs it.
pub fn ruleset_tables() -> Vec<&'static shared::weapons::Tables> {
    let r = REGISTRY.read().unwrap_or_else(PoisonError::into_inner);
    let mut out: Vec<&'static shared::weapons::Tables> = Vec::new();
    for id in r.ids() {
        let Some(t) = r.resolve(id).map(|r| r.plugin.gun.tables()) else { continue };
        if !out.iter().any(|o| std::ptr::eq(*o, t)) {
            out.push(t);
        }
    }
    out
}
