//! `tsmod_*` plugins (port addition, issue #104; docs/implementation/plugin-api.md §1,
//! bot-plugins.md): libraries the server loads from its `dlls/` directory at `GameDLLInit` and
//! calls back on the game's events. `ts-server-dll` does the loading and the C calls
//! ([`PluginHooks`] over a plugin's `ts_plugin_api::Plugin`); this module decides which files to
//! load ([`loader`]), keeps the loaded plugins, and fires their callbacks at the right points.
//!
//! # When callbacks run
//! Plugins call the plugin API from their callbacks, and the API reaches a player only while it
//! is in its slot (`plugin_api`'s module docs). So the player callbacks that the game reaches
//! inside a player's own dispatch (spawn, damage, death, a kill) are deferred until every player
//! they name is back ([`after`]); the rest (frame, map, connect, commands, sounds) run at once.

pub mod host;
pub mod loader;

use crate::commands::Args;
use crate::world::{self, DamageInfo};
use goldsrc::Global;
use goldsrc::edict::EdictPtr;
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::cell::RefCell;
use std::ffi::CStr;
use std::rc::Rc;
use ts_plugin_api::PluginId;

/// A loaded plugin's callbacks. `ts-server-dll` implements it over the plugin's C descriptor;
/// every method does nothing unless the plugin gave that callback. Players and entities are
/// indices (0 the world, -1 none), as the API's are.
pub trait PluginHooks {
    fn game_init(&self) {}
    fn precache(&self) {}
    fn server_activate(&self, _map: &CStr) {}
    fn server_deactivate(&self) {}
    fn start_frame(&self) {}
    fn shutdown(&self) {}
    fn client_connect(&self, _player: i32, _bot: bool) {}
    fn client_put_in_server(&self, _player: i32, _bot: bool) {}
    fn client_disconnect(&self, _player: i32, _bot: bool) {}
    /// True if the plugin swallowed the command.
    fn client_command(&self, _player: i32, _argv: &[&CStr]) -> bool {
        false
    }
    /// Whether [`Self::client_command`] does anything (so the server skips building `argv`).
    fn wants_client_commands(&self) -> bool {
        false
    }
    fn player_spawned(&self, _player: i32) {}
    fn player_damaged(&self, _victim: i32, _attacker: i32, _inflictor: i32, _damage: f32, _bits: i32, _health: f32) {}
    fn player_killed(&self, _victim: i32, _attacker: i32, _inflictor: i32) {}
    fn on_kill(&self, _killer: i32, _victim: i32, _points: i32) {}
    fn round_start(&self) {}
    fn round_end(&self) {}
    fn sound(&self, _entity: i32, _origin: [f32; 3], _sample: &CStr, _volume: f32, _attenuation: f32) {}
    /// Whether [`Self::sound`] does anything (the sound hook is installed only then).
    fn wants_sounds(&self) -> bool {
        false
    }
    fn weapon_changed(&self, _player: i32, _weapon: i32) {}
    fn ammo_changed(&self, _player: i32, _weapon: i32, _clip: i32, _ammo: i32) {}
}

struct Loaded {
    id: PluginId,
    name: String,
    hooks: Option<Rc<dyn PluginHooks>>,
}

static PLUGINS: Global<RefCell<Vec<Loaded>>> = Global(RefCell::new(Vec::new()));

/// Reserves an id for the plugin `name` about to be loaded (its load function may already call
/// the API with it). [`attach`] completes it, [`forget`] drops it if the load fails.
pub fn reserve(name: &str) -> PluginId {
    let mut p = PLUGINS.0.borrow_mut();
    // Ids are never reused, also after a failed load.
    let id = p.iter().map(|l| l.id).max().unwrap_or(0) + 1;
    p.push(Loaded { id, name: name.to_owned(), hooks: None });
    id
}

/// The plugin loaded: its callbacks from now on.
pub fn attach(id: PluginId, hooks: Rc<dyn PluginHooks>) {
    if let Some(l) = PLUGINS.0.borrow_mut().iter_mut().find(|l| l.id == id) {
        l.hooks = Some(hooks);
    }
}

/// The load failed: whatever the plugin registered goes (its commands then do nothing).
pub fn forget(id: PluginId) {
    PLUGINS.0.borrow_mut().retain(|l| l.id != id);
    host::forget(id);
    crate::bot_backend::forget_plugin(id);
}

/// The name (`<name>` of `tsmod_<name>`) of a plugin being loaded or loaded.
pub fn name_of(id: PluginId) -> Option<String> {
    PLUGINS.0.borrow().iter().find(|l| l.id == id).map(|l| l.name.clone())
}

/// The loaded plugins' names, in load order.
pub fn names() -> Vec<String> {
    PLUGINS.0.borrow().iter().filter(|l| l.hooks.is_some()).map(|l| l.name.clone()).collect()
}

/// The loaded plugins' callbacks, copied out so a callback may load, register or call back in.
fn all() -> Vec<Rc<dyn PluginHooks>> {
    PLUGINS.0.borrow().iter().filter_map(|l| l.hooks.clone()).collect()
}

/// Whether any plugin is loaded.
pub fn any() -> bool {
    PLUGINS.0.borrow().iter().any(|l| l.hooks.is_some())
}

fn each(f: impl Fn(&dyn PluginHooks)) {
    for h in all() {
        f(&*h);
    }
}

/// Runs `job` once every player in `players` is back in its slot (`world::defer` on each in
/// turn): at once outside their dispatches.
fn after(players: &[*mut edict_t], job: impl FnOnce() + 'static) {
    match players.split_first() {
        None => job(),
        Some((&first, rest)) => {
            let rest = rest.to_vec();
            if first.is_null() || !world::is_taken_out(first) {
                after(&rest, job);
            } else {
                world::defer(first, move || after(&rest, job));
            }
        }
    }
}

/// An entity's index for a callback: -1 for none.
fn index(e: *mut edict_t) -> i32 {
    if e.is_null() { -1 } else { engine().index_of(e) }
}

fn is_fake(e: *mut edict_t) -> bool {
    !e.is_null() && e.v().flags & FL_FAKECLIENT as i32 != 0
}

// ---- events -------------------------------------------------------------------------------------

/// `GameDLLInit`, after every plugin loaded. Installs the sound hook if a plugin listens.
pub fn game_init() {
    if all().iter().any(|h| h.wants_sounds()) {
        goldsrc::server::set_sound_hook(Some(sound));
    }
    each(|h| h.game_init());
}

pub fn precache() {
    each(|h| h.precache());
}

pub fn server_activate() {
    if !any() {
        return;
    }
    let map = engine().string(engine().globals().mapname).to_owned();
    each(|h| h.server_activate(&map));
}

pub fn server_deactivate() {
    each(|h| h.server_deactivate());
}

/// `StartFrame`, after the bots and before the rules: the plugins' frame, then the weapon and ammo
/// changes since the last frame (`plugin_api::start_frame`).
pub fn start_frame() {
    each(|h| h.start_frame());
}

pub fn shutdown() {
    each(|h| h.shutdown());
}

pub fn client_connect(e: *mut edict_t) {
    let (i, bot) = (index(e), is_fake(e));
    each(|h| h.client_connect(i, bot));
}

pub fn client_put_in_server(e: *mut edict_t, bot: bool) {
    let i = index(e);
    each(|h| h.client_put_in_server(i, bot));
}

pub fn client_disconnect(e: *mut edict_t) {
    let (i, bot) = (index(e), is_fake(e));
    each(|h| h.client_disconnect(i, bot));
}

/// A client command, before the game's: true if a plugin swallowed it.
pub fn client_command(e: *mut edict_t, args: &Args) -> bool {
    let hooks: Vec<_> = all().into_iter().filter(|h| h.wants_client_commands()).collect();
    if hooks.is_empty() {
        return false;
    }
    let argv: Vec<&CStr> = (0..args.argc()).map(|i| args.argv(i)).collect();
    let i = index(e);
    hooks.iter().any(|h| h.client_command(i, &argv))
}

pub fn player_spawned(e: *mut edict_t) {
    if any() {
        let i = index(e);
        after(&[e], move || each(|h| h.player_spawned(i)));
    }
}

pub fn player_damaged(e: *mut edict_t, dmg: &DamageInfo, health: f32) {
    if any() {
        let (v, a, inf) = (index(e), index(dmg.attacker), index(dmg.inflictor));
        let (amount, bits) = (dmg.damage, dmg.bits);
        after(&[e, dmg.attacker], move || each(|h| h.player_damaged(v, a, inf, amount, bits, health)));
    }
}

pub fn player_killed(e: *mut edict_t, dmg: &DamageInfo) {
    if any() {
        let (v, a, inf) = (index(e), index(dmg.attacker), index(dmg.inflictor));
        after(&[e, dmg.attacker], move || each(|h| h.player_killed(v, a, inf)));
    }
}

pub fn on_kill(killer: *mut edict_t, victim: *mut edict_t, points: i32) {
    if any() {
        let (k, v) = (index(killer), index(victim));
        after(&[killer, victim], move || each(|h| h.on_kill(k, v, points)));
    }
}

pub fn round_start() {
    each(|h| h.round_start());
}

pub fn round_end() {
    each(|h| h.round_end());
}

/// The sound hook (`goldsrc::server::set_sound_hook`): every sound the game plays. `pos` is the
/// ambient sound's position, else the entity's origin.
fn sound(e: *mut edict_t, pos: Option<[f32; 3]>, sample: &CStr, volume: f32, attenuation: f32) {
    let i = if e.is_null() { 0 } else { engine().index_of(e) };
    let origin = pos.unwrap_or_else(|| if e.is_null() { [0.0; 3] } else { e.v().origin });
    for h in all().iter().filter(|h| h.wants_sounds()) {
        h.sound(i, origin, sample, volume, attenuation);
    }
}

pub fn weapon_changed(player: i32, weapon: i32) {
    each(|h| h.weapon_changed(player, weapon));
}

pub fn ammo_changed(player: i32, weapon: i32, clip: i32, ammo: i32) {
    each(|h| h.ammo_changed(player, weapon, clip, ammo));
}
