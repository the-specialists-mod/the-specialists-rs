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
    /// Version 5: a stunt started (`ts_plugin_api::stunt`).
    fn player_stunt(&self, _player: i32, _stunt: i32) {}
    /// Version 5: a power-up took effect (`ts_plugin_api::powerup` bits).
    fn player_powerup(&self, _player: i32, _powerup: i32) {}
    /// Version 5: a kung-fu move is about to land; may change its time and damage. True blocks it.
    fn melee_attack(&self, _player: i32, _time: &mut f32, _damage: &mut f32) -> bool {
        false
    }
    /// Whether [`Self::melee_attack`] does anything (so the gun skips lending the player).
    fn wants_melee_attack(&self) -> bool {
        false
    }
    /// Version 5: `PlayerKilled` scored a death.
    fn kill_scored(&self, _kill: &ts_plugin_api::KillInfo) {}
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

/// Listeners (version 5, `PluginApi::add_listener`): plugins the server didn't load (metamod
/// plugins) that registered a descriptor, by handle.
static LISTENERS: Global<RefCell<Listeners>> = Global(RefCell::new(Listeners { next: 1, list: Vec::new() }));

/// The listeners in registration order, with their handles.
struct Listeners {
    next: i32,
    list: Vec<(i32, Rc<dyn PluginHooks>)>,
}

impl Listeners {
    fn add(&mut self, hooks: Rc<dyn PluginHooks>) -> i32 {
        let h = self.next;
        self.next = h.checked_add(1).unwrap_or(1);
        self.list.push((h, hooks));
        h
    }

    fn remove(&mut self, handle: i32) -> bool {
        let before = self.list.len();
        self.list.retain(|(h, _)| *h != handle);
        self.list.len() != before
    }
}

/// Registers a listener: its handle (> 0). Installs the sound hook if it listens for sounds (a
/// loaded plugin's is installed at `GameDLLInit`).
pub fn add_listener(hooks: Rc<dyn PluginHooks>) -> i32 {
    if hooks.wants_sounds() {
        goldsrc::server::set_sound_hook(Some(sound));
    }
    LISTENERS.0.borrow_mut().add(hooks)
}

/// Unregisters a listener: false for a handle that isn't registered.
pub fn remove_listener(handle: i32) -> bool {
    LISTENERS.0.borrow_mut().remove(handle)
}

/// The loaded plugins' callbacks, then the listeners', copied out so a callback may load,
/// register or call back in.
fn all() -> Vec<Rc<dyn PluginHooks>> {
    let mut v: Vec<_> = PLUGINS.0.borrow().iter().filter_map(|l| l.hooks.clone()).collect();
    v.extend(LISTENERS.0.borrow().list.iter().map(|(_, h)| h.clone()));
    v
}

/// Whether any plugin is loaded or a listener registered.
pub fn any() -> bool {
    PLUGINS.0.borrow().iter().any(|l| l.hooks.is_some()) || !LISTENERS.0.borrow().list.is_empty()
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

/// Version 5: player `e` started a stunt (`ts_plugin_api::stunt`), from inside its PreThink, so
/// the callback waits until it is back in its slot.
pub fn player_stunt(e: *mut edict_t, stunt: i32) {
    if any() {
        let i = index(e);
        after(&[e], move || each(|h| h.player_stunt(i, stunt)));
    }
}

/// Version 5: a power-up took effect on player `e` (deferred as [`player_stunt`]).
pub fn player_powerup(e: *mut edict_t, bits: i32) {
    if any() {
        let i = index(e);
        after(&[e], move || each(|h| h.player_powerup(i, bits)));
    }
}

/// Version 5: `PlayerKilled` scored `victim`'s death by `killer` (null for none), deferred as
/// [`on_kill`] (so after the victim's `Killed`, its `TSHealth` 0 included).
pub fn kill_scored(victim: *mut edict_t, killer: *mut edict_t, kill: ts_plugin_api::KillInfo) {
    if any() {
        after(&[killer, victim], move || each(|h| h.kill_scored(&kill)));
    }
}

/// Whether anything listens for [`melee_attack`].
pub fn wants_melee_attack() -> bool {
    all().iter().any(|h| h.wants_melee_attack())
}

/// Version 5: a kung-fu move of `player` is about to land. Each plugin may change `time` and
/// `damage` (a value that isn't finite or is below 0 is ignored); the first that blocks stops it
/// and the rest aren't asked. True: blocked. Runs at once: the caller lends the player.
pub fn melee_attack(player: i32, time: &mut f32, damage: &mut f32) -> bool {
    melee_chain(&all(), player, time, damage)
}

fn melee_chain(hooks: &[Rc<dyn PluginHooks>], player: i32, time: &mut f32, damage: &mut f32) -> bool {
    for h in hooks.iter().filter(|h| h.wants_melee_attack()) {
        let (mut t, mut d) = (*time, *damage);
        let block = h.melee_attack(player, &mut t, &mut d);
        if t.is_finite() && t >= 0.0 {
            *time = t;
        }
        if d.is_finite() && d >= 0.0 {
            *damage = d;
        }
        if block {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// A plugin that scales the time and damage and maybe blocks, counting its calls.
    struct Melee {
        time: f32,
        damage: f32,
        block: bool,
        calls: Cell<i32>,
    }

    impl PluginHooks for Melee {
        fn wants_melee_attack(&self) -> bool {
            true
        }
        fn melee_attack(&self, _player: i32, time: &mut f32, damage: &mut f32) -> bool {
            self.calls.set(self.calls.get() + 1);
            *time *= self.time;
            *damage *= self.damage;
            self.block
        }
    }

    struct Quiet;
    impl PluginHooks for Quiet {}

    fn melee(time: f32, damage: f32, block: bool) -> Rc<Melee> {
        Rc::new(Melee { time, damage, block, calls: Cell::new(0) })
    }

    #[test]
    fn melee_attack_chains_and_the_first_block_stops() {
        let (a, b, c) = (melee(2.0, 0.5, false), melee(1.0, 3.0, true), melee(1.0, 1.0, false));
        let hooks: Vec<Rc<dyn PluginHooks>> = vec![Rc::new(Quiet), a.clone(), b.clone(), c.clone()];
        let (mut t, mut d) = (0.35, 20.0);
        assert!(melee_chain(&hooks, 1, &mut t, &mut d));
        assert_eq!((t, d), (0.7, 30.0), "each sees the last one's values");
        assert_eq!((a.calls.get(), b.calls.get(), c.calls.get()), (1, 1, 0));
        // Nobody blocks: the move goes on with the changes.
        let hooks: Vec<Rc<dyn PluginHooks>> = vec![a.clone()];
        let (mut t, mut d) = (0.5, 10.0);
        assert!(!melee_chain(&hooks, 1, &mut t, &mut d));
        assert_eq!((t, d), (1.0, 5.0));
    }

    #[test]
    fn melee_attack_ignores_bad_values() {
        let hooks: Vec<Rc<dyn PluginHooks>> = vec![melee(f32::NAN, -1.0, false), melee(f32::INFINITY, 1.0, false)];
        let (mut t, mut d) = (0.35, 20.0);
        assert!(!melee_chain(&hooks, 1, &mut t, &mut d));
        assert_eq!((t, d), (0.35, 20.0));
    }

    #[test]
    fn listeners_get_handles_and_leave() {
        let mut l = Listeners { next: 1, list: Vec::new() };
        let h = l.add(Rc::new(Quiet));
        let h2 = l.add(melee(1.0, 1.0, true));
        assert!(h > 0 && h2 > 0 && h != h2);
        assert!(l.remove(h2));
        assert!(!l.remove(h2), "already gone");
        assert!(!l.remove(99));
        assert_eq!(l.list.len(), 1);
        let h3 = l.add(Rc::new(Quiet));
        assert!(h3 != h && h3 != h2, "handles aren't reused");
        // After i32::MAX the count starts over at 1, never 0 or below.
        l.next = i32::MAX;
        assert_eq!(l.add(Rc::new(Quiet)), i32::MAX);
        assert_eq!(l.next, 1);
    }
}
