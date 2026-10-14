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
//! `ts_gameplay` is latched at a map start ([`map_select`]): a change mid-map takes effect at the
//! next map (`changelevel`, `map`), whose new game is the fresh state. The map start's own
//! server config is the exception ([`exec_server_config`]). Physinfo `gp` and `SrvSett` 4 carry the id
//! to the clients.

pub mod server;

pub use server::{ConnectingClient, ServerRules};

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

/// The last unknown id warned about, so each is reported once.
static WARNED: Global<RefCell<String>> = Global(RefCell::new(String::new()));

/// The `ts_gameplay` value last reported as waiting for the next map ([`frame`]), so each change
/// is reported once.
static NOTED: Global<RefCell<String>> = Global(RefCell::new(String::new()));

/// Counts map starts ([`map_select`]), so a config check queued for an earlier map does nothing
/// ([`exec_server_config`]).
static MAP_SERIAL: Global<Cell<u32>> = Global(Cell::new(0));

/// This map's config check: `ts_gameplay` as it was when the server config began, until the check
/// ends.
static CONFIG_FROM: Global<RefCell<Option<String>>> = Global(RefCell::new(None));

/// The server command [`exec_server_config`] queues around the config (port addition, not for
/// typing).
pub const CONFIG_CHECK_CMD: &std::ffi::CStr = c"_ts_gameplay_mapcfg";

/// Registers a plugin (the server library's `GameDLLInit`). False for an id that isn't a short
/// number.
/// The plugin's own cvars ([`ServerRules::cvars`]) are registered with it, after TS 3.0's (the
/// rules' `game_init` runs first), each unless the name exists already.
pub fn register(id: &'static str, plugin: &'static ServerPlugin) -> bool {
    let added = REGISTRY.write().unwrap_or_else(PoisonError::into_inner).add(id, plugin);
    if added {
        cvars::register_extra(plugin.server.cvars());
    }
    added
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
    ts_gameplay::srvsett_value(active_id()).unwrap_or(30)
}

/// A map is installing its rules (`rules::install`, from worldspawn, after `game.cfg`): the plugin
/// `ts_gameplay` selects becomes active for the whole map, before the rules are built
/// ([`ServerRules::install`]).
pub fn map_select() {
    let (id, plugin) = resolve(&cvars::gameplay());
    set_active(id, plugin);
    NOTED.0.borrow_mut().clear();
    MAP_SERIAL.0.set(MAP_SERIAL.0.get().wrapping_add(1));
    CONFIG_FROM.0.borrow_mut().take();
    engine().print(&format!("[ts] gameplay: ruleset {id} for this map\n"));
}

/// A map has installed its rules ([`map_select`] chose the plugin): the plugin sets up its state.
pub fn map_start() {
    active().server.on_activate();
}

/// Queues the map start's server config (`rules::install`'s `exec <servercfgfile>`, the command
/// `exec`) between the two halves of a check of what it does to `ts_gameplay`
/// ([`cmd_config_check`]). The engines run the server config only once the map has loaded, after
/// [`map_select`], so a `ts_gameplay` the config sets would otherwise wait for the map after. The
/// check carries the map's serial, so that one left in the console buffer from an earlier map
/// (behind a `wait` script) does nothing; and only a change the config's own commands make (from
/// the first half to the second) counts, not one made before the config ran.
pub fn exec_server_config(exec: &str) {
    let serial = MAP_SERIAL.0.get();
    let cmd = CONFIG_CHECK_CMD.to_string_lossy();
    engine().server_command(&format!("{cmd} {serial} begin\n{exec}{cmd} {serial} end\n"));
}

/// Whether the server config's `ts_gameplay` (`from` before it ran, `to` after) moves the map to
/// another ruleset: the config changed the value, and to an id (`to_id`, resolved) other than the
/// map's (`active`).
fn config_reloads(from: &str, to: &str, to_id: &str, active: &str) -> bool {
    from.trim() != to.trim() && to_id != active
}

/// [`CONFIG_CHECK_CMD`] `<serial> begin|end`: if this map's server config chose another ruleset,
/// the map is loaded again under it (`changelevel` to the same map), before anyone has joined.
pub extern "C" fn cmd_config_check() {
    let eng = engine();
    let serial = eng.cmd_argv(1).to_str().ok().and_then(|s| s.parse::<u32>().ok());
    if serial != Some(MAP_SERIAL.0.get()) {
        return;
    }
    let value = cvars::gameplay();
    if eng.cmd_argv(2) == c"begin" {
        *CONFIG_FROM.0.borrow_mut() = Some(value);
        return;
    }
    let Some(from) = CONFIG_FROM.0.borrow_mut().take() else { return };
    let (id, _) = resolve(&value);
    if !config_reloads(&from, &value, id, active_id()) {
        return;
    }
    let map = eng.string(eng.globals().mapname).to_string_lossy().into_owned();
    *NOTED.0.borrow_mut() = value;
    eng.print(&format!("[ts] gameplay: the server config chose ruleset {id}; loading {map} again under it\n"));
    eng.server_command(&format!("changelevel {map}\n"));
}

/// Makes a plugin the active one, with its weapon tables
/// ([`GunRules::tables`](shared::gameplay::GunRules::tables)).
fn set_active(id: &'static str, plugin: &'static ServerPlugin) {
    ACTIVE.0.set(Some((id, plugin)));
    shared::weapons::table::set_ruleset_tables(plugin.gun.tables());
}

/// Every registered plugin's own precaches ([`ServerRules::precache`]), each once, at the map's
/// world precache after TS 3.0's.
pub fn precache_rulesets() {
    let r = REGISTRY.read().unwrap_or_else(PoisonError::into_inner);
    let mut done: Vec<*const ServerPlugin> = Vec::new();
    for id in r.ids() {
        let p = r.resolve(id).plugin;
        if !done.contains(&(p as *const ServerPlugin)) {
            done.push(p);
            p.server.precache();
        }
    }
}

/// The weapon tables of every registered plugin, the core plugin's first and each once: what the
/// map precaches (`weapons::precache_weapon_assets`), so the map's precache list is the same
/// whichever ruleset runs it.
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

/// Once a frame (`StartFrame`, while rules are installed): a `ts_gameplay` that no longer selects
/// the map's ruleset waits for the next map; the console says so once per value.
pub fn frame() {
    let value = cvars::gameplay();
    if *NOTED.0.borrow() == value {
        return;
    }
    let (id, _) = resolve(&value);
    let current = active_id();
    if id != current {
        engine().print(&format!(
            "[ts] gameplay: ts_gameplay is \"{}\"; ruleset {id} takes effect at the next map (this map runs {current})\n",
            value.trim()
        ));
    }
    *NOTED.0.borrow_mut() = value;
}

#[cfg(test)]
mod tests {
    use super::config_reloads;

    /// Only a change the server config itself makes, to another ruleset than the map's, reloads
    /// the map.
    #[test]
    fn the_server_config_chooses_the_maps_ruleset() {
        assert!(config_reloads("30", "21", "21", "30"), "server.cfg: ts_gameplay 21");
        assert!(!config_reloads("30", "30", "30", "30"), "no change");
        assert!(!config_reloads("21", "21", "21", "30"), "changed before the config ran: the next map");
        assert!(!config_reloads("30", " 30 ", "30", "30"));
        assert!(!config_reloads("21", "30", "30", "30"), "set back to the map's ruleset");
        assert!(!config_reloads("30", "99", "30", "30"), "an unknown id is the core plugin");
    }
}
