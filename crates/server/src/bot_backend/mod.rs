//! Bot backends (port addition, issue #104; docs/implementation/bot-plugins.md). TS's bots are
//! one backend among several: the server reaches them only through [`BotBackend`], and the
//! server cvar `ts_bot_backend` picks the backend when a map starts ([`map_start`]); a change
//! mid-map reloads the map ([`crate::reload`]).
//!
//! * `native`: TS's own bots, the AI ported from `mp.dll` ([`crate::bots`], the default).
//! * a name a plugin registered (`tsmod_*` libraries, [`crate::plugins`]): `ts-server-dll`
//!   implements [`BotBackend`] over the plugin's C callbacks.
//!
//! The host (this module) owns what doesn't depend on the AI: the fake client and its join
//! (`BotSetup`, [`create_player`]), the port's `ts_bot_quota` (added and kicked through the
//! backend), `addbot`/`addcustombot` (handed to the backend), the bot manager's timing, and
//! `ts_bot_stop` for plugin bots (`plugin_api`'s `run_cmd`). Bots are kicked when a map start
//! switches the backend, so a backend only ever sees its own bots.
//!
//! A bot player's backend state lives in `Player::bot` ([`BotState`]): native bots keep their
//! brain there, plugin bots a marker. Hooks that can run inside a player's dispatch take the
//! `Player`, so the native AI keeps TS's order of calls exactly.

pub mod quota;

use crate::commands::Args;
use crate::player::{Player, with_player};
use crate::rules;
use crate::world::{self, Cx, DamageInfo};
use goldsrc::Global;
use goldsrc::edict::EdictPtr;
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::any::Any;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::ffi::CString;
use std::rc::Rc;

/// A backend's per-bot state, kept on the bot's `Player` (`Player::bot`).
pub type BotState = Box<dyn Any>;

/// The name of TS's own bots, the default backend.
pub const NATIVE: &str = "native";

/// What `addbot`, `addcustombot` and the quota ask a backend for. Names and models are the
/// argument's bytes as typed: TS `strcpy`s `CMD_ARGV` into `BotCreate`'s `char model[32]` and
/// `char name[64]`, and the model becomes the `model`/`team` keys and, in teamplay, the team
/// (#190).
#[derive(Clone, Debug, PartialEq)]
pub enum AddBot {
    /// `addbot [model]`.
    Addbot { model: Vec<u8> },
    /// `addcustombot [name] [model] [skill]` (skill as typed, 0 when missing).
    Custom { name: Vec<u8>, model: Vec<u8>, skill: f32 },
    /// One of the port's `ts_bot_quota` bots, named by the host ("Bot Neo", ...).
    Quota { name: String },
}

impl AddBot {
    /// A plugin backend's `add_bot(name, model, skill)` arguments: the bytes unconverted (an
    /// embedded NUL, which no command argument has, is dropped), empty where the request has none.
    pub fn plugin_args(&self) -> (CString, CString, f32) {
        let (name, model, skill): (&[u8], &[u8], f32) = match self {
            AddBot::Addbot { model } => (b"", model, 0.0),
            AddBot::Custom { name, model, skill } => (name, model, *skill),
            AddBot::Quota { name } => (name.as_bytes(), b"", 0.0),
        };
        (c_string(name), c_string(model), skill)
    }
}

/// `bytes` as a C string, without any NUL.
fn c_string(bytes: &[u8]) -> CString {
    CString::new(bytes.iter().copied().filter(|&c| c != 0).collect::<Vec<u8>>()).unwrap_or_default()
}

/// A bot backend. Every hook but [`Self::name`] and [`Self::add_bot`] defaults to nothing, which
/// is what a plugin backend needs: plugins get the game's events through `crate::plugins`, not
/// through these. The per-player hooks run inside the player's dispatch, in TS's order.
pub trait BotBackend {
    /// `ts_bot_backend`'s value that selects it.
    fn name(&self) -> &str;
    /// `GameDLLInit`.
    fn game_init(&self) {}
    /// mp.dll's part of `CTSGameRules::Init` (0x1004d910): a new game's rules (map start, and
    /// the map end's `ChangeLevel`).
    fn map_init(&self) {}
    /// The map's precache (`W_Precache`'s list).
    fn precache(&self) {}
    /// `ServerActivate`, `ServerDeactivate`.
    fn server_activate(&self) {}
    fn server_deactivate(&self) {}
    /// A map start made it the active backend / another one.
    fn activate(&self) {}
    fn deactivate(&self) {}
    /// `StartFrame`, after the host's quota: run the bots.
    fn think(&self) {}
    /// Adds one bot: the player and its backend state's skill (reported in the log), or `None`.
    fn add_bot(&self, req: &AddBot) -> Option<(*mut edict_t, Option<f32>)>;
    /// The state a bot player gets when a quota bot is picked up again without one (the engine
    /// kept the client across a level change, with a new `Player`).
    fn new_state(&self, _skill: f32) -> BotState {
        Box::new(PluginBot)
    }
    /// After [`Self::new_state`] was given to the quota bot `e`.
    fn adopted(&self, _e: *mut edict_t) {}
    /// The head and tail of a player's `Spawn` (`CBot::Spawn` around `CBasePlayer::Spawn`).
    fn spawn_pre(&self, _p: &mut Player, _cx: &mut Cx) {}
    fn spawn_post(&self, _p: &mut Player, _cx: &mut Cx) {}
    /// After a player's `TraceAttack`, `Killed`; a kill a player scored.
    fn trace_attack(&self, _p: &mut Player, _cx: &mut Cx, _dmg: &DamageInfo) {}
    fn killed(&self, _p: &mut Player, _cx: &mut Cx, _attacker: *mut edict_t) {}
    fn on_kill(&self, _killer: &mut Player, _victim: *mut edict_t, _now: f32) {}
    /// Every player's PreThink tail (mp.dll's node learning).
    fn pre_think(&self, _p: &mut Player, _cx: &mut Cx) {}
    /// A client command the game didn't handle: true if the backend did.
    fn client_command(&self, _p: &mut Player, _cx: &mut Cx, _cmd: &str, _args: &Args) -> bool {
        false
    }
}

/// The state of a plugin backend's bot: it keeps its own, so the player only carries this.
pub struct PluginBot;

struct Registry {
    /// The backends, native first, each with the plugin that registered it (`None`: native).
    backends: Vec<(Rc<dyn BotBackend>, Option<ts_plugin_api::PluginId>)>,
    active: Option<Rc<dyn BotBackend>>,
    /// `ts_bot_backend` values already warned about.
    warned: Vec<String>,
    /// Which backend made each bot still around, by user id.
    owners: BTreeMap<i32, String>,
    /// A map start chose the map's backend ([`map_start`]): a backend registered from now on may
    /// take over at once ([`register_plugin`]).
    started: bool,
}

static REGISTRY: Global<RefCell<Registry>> =
    Global(RefCell::new(Registry { backends: Vec::new(), active: None, warned: Vec::new(), owners: BTreeMap::new(), started: false }));

/// Adds `b` for `plugin`: false if the name is taken (case-insensitive, `native` included). The
/// native backend goes first whenever it registers: an attached plugin (version 7) can register
/// before `GameDLLInit` does (metamod's `Meta_Attach`), and the first backend is the default.
fn add(b: Rc<dyn BotBackend>, plugin: Option<ts_plugin_api::PluginId>) -> bool {
    let mut r = REGISTRY.0.borrow_mut();
    if r.backends.iter().any(|(x, _)| x.name().eq_ignore_ascii_case(b.name())) {
        return false;
    }
    if plugin.is_none() && b.name() == NATIVE {
        r.backends.insert(0, (b, None));
    } else {
        r.backends.push((b, plugin));
    }
    true
}

/// Registers a backend (the native one at `GameDLLInit`). False if the name is taken
/// (case-insensitive, `native` included).
pub fn register(b: Rc<dyn BotBackend>) -> bool {
    add(b, None)
}

/// Registers a plugin's backend (`PluginApi::register_bot_backend`), as [`register`]. Version 7:
/// one registered after this map's start (an AMX Mod X plugin's `plugin_init`, which runs in
/// `ServerActivate`) takes over now if `ts_bot_backend` names it and the map runs native, as the
/// map start would have chosen it ([`late_switch`]); without that the map would be reloaded for
/// it (`crate::reload`).
pub fn register_plugin(plugin: ts_plugin_api::PluginId, b: Rc<dyn BotBackend>) -> bool {
    let name = b.name().to_owned();
    if !add(b, Some(plugin)) {
        return false;
    }
    let started = REGISTRY.0.borrow().started;
    if started {
        let (wanted, running) = (wanted(), active());
        if late_switch(started, running.name(), wanted.name(), &name) {
            switch_to(wanted);
        }
    }
    true
}

/// Whether a backend `registered` just now takes over the running map: a map start chose the
/// map's backend already, the map runs `native` (the fallback for a name not registered then),
/// and `ts_bot_backend` resolves to the new one.
pub fn late_switch(started: bool, running: &str, wanted: &str, registered: &str) -> bool {
    started && running == NATIVE && wanted.eq_ignore_ascii_case(registered)
}

/// The backends of `plugin`, removed from the registry (with their places).
fn take_plugin(plugin: ts_plugin_api::PluginId) -> Vec<Rc<dyn BotBackend>> {
    let mut r = REGISTRY.0.borrow_mut();
    let (gone, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut r.backends).into_iter().partition(|(_, p)| *p == Some(plugin));
    r.backends = kept;
    gone.into_iter().map(|(b, _)| b).collect()
}

/// Drops the backends of a plugin that failed to load (before any map start used them).
pub fn forget_plugin(plugin: ts_plugin_api::PluginId) {
    take_plugin(plugin);
}

/// Version 7: an attached plugin detached. Its backends are unregistered; if one was active, it
/// is deactivated and native takes over now, the bots it made kicked (as a map start switching
/// backends, [`switch_to`]). `ts_bot_backend` may still name it: it then resolves to native
/// (warned once), so the map isn't reloaded for it.
pub fn drop_plugin(plugin: ts_plugin_api::PluginId) {
    let was_active = active_plugin() == Some(plugin);
    let gone = take_plugin(plugin);
    if !was_active {
        return;
    }
    let native = REGISTRY.0.borrow().backends.first().map(|(b, _)| b.clone());
    let Some(native) = native else { return };
    let old = REGISTRY.0.borrow_mut().active.replace(native.clone());
    if let Some(o) = old.or_else(|| gone.into_iter().next()) {
        o.deactivate();
    }
    kick_all_but(native.name());
    engine().print(&format!("[ts] bots: backend {} (its plugin detached)\n", native.name()));
    native.activate();
}

/// The plugin whose backend is active, if a plugin's is.
pub fn active_plugin() -> Option<ts_plugin_api::PluginId> {
    let r = REGISTRY.0.borrow();
    let a = r.active.as_ref()?;
    r.backends.iter().find(|(b, _)| Rc::ptr_eq(b, a)).and_then(|(_, p)| *p)
}

/// Whether a backend of that name is registered.
pub fn is_registered(name: &str) -> bool {
    REGISTRY.0.borrow().backends.iter().any(|(x, _)| x.name().eq_ignore_ascii_case(name))
}

/// The active backend: the one the last map start selected (native until then).
pub fn active() -> Rc<dyn BotBackend> {
    let r = REGISTRY.0.borrow();
    if let Some(a) = &r.active {
        return a.clone();
    }
    r.backends.first().map(|(b, _)| b.clone()).unwrap_or_else(|| Rc::new(crate::bots::backend::Native))
}

/// The backend `value` (`ts_bot_backend`) names among `names` (registered, native first): its
/// index, or `None` for an unknown name. An empty value is native.
pub fn select(names: &[&str], value: &str) -> Option<usize> {
    let v = value.trim();
    if v.is_empty() {
        return names.iter().position(|n| *n == NATIVE);
    }
    names.iter().position(|n| n.eq_ignore_ascii_case(v))
}

/// The backend `ts_bot_backend` selects now: an unknown name warns once and is native. What a map
/// start makes active, and what [`crate::reload`] compares with [`active`].
pub fn wanted() -> Rc<dyn BotBackend> {
    let value = engine().cvar_string(c"ts_bot_backend").to_string_lossy().into_owned();
    let (pick, unknown) = {
        let r = REGISTRY.0.borrow();
        let names: Vec<&str> = r.backends.iter().map(|(b, _)| b.name()).collect();
        let pick = select(&names, &value);
        let native = names.iter().position(|n| *n == NATIVE).unwrap_or(0);
        (r.backends.get(pick.unwrap_or(native)).map(|(b, _)| b.clone()), pick.is_none())
    };
    if unknown && !REGISTRY.0.borrow().warned.contains(&value) {
        REGISTRY.0.borrow_mut().warned.push(value.clone());
        engine().print(&format!("[ts] bots: unknown ts_bot_backend \"{value}\", using native\n"));
    }
    pick.unwrap_or_else(active)
}

/// Map start (`rules::install`, after `game.cfg` ran and before the rules' `Init`): reads
/// `ts_bot_backend` and makes that backend active. A change deactivates the old one and kicks the
/// bots it made (the quota adds the new backend's). A change mid-map reloads the map
/// ([`crate::reload`]).
pub fn map_start() {
    if REGISTRY.0.borrow().backends.is_empty() {
        return;
    }
    REGISTRY.0.borrow_mut().started = true;
    switch_to(wanted());
}

/// Makes `new` the active backend if it isn't: the old one is deactivated and the bots it made
/// kicked (the quota adds the new backend's), then `new` is activated.
fn switch_to(new: Rc<dyn BotBackend>) {
    let old = REGISTRY.0.borrow().active.clone();
    let changed = old.as_ref().is_none_or(|o| !Rc::ptr_eq(o, &new));
    if !changed {
        return;
    }
    if let Some(o) = &old {
        o.deactivate();
        kick_all_but(new.name());
    }
    REGISTRY.0.borrow_mut().active = Some(new.clone());
    engine().print(&format!("[ts] bots: backend {}\n", new.name()));
    new.activate();
}

/// Kicks every bot still connected that another backend than `keep` made.
fn kick_all_but(keep: &str) {
    let eng = engine();
    let owners = REGISTRY.0.borrow().owners.clone();
    for e in rules::players() {
        let uid = eng.user_id(e);
        if owners.get(&uid).is_some_and(|o| !o.eq_ignore_ascii_case(keep)) {
            eng.server_command(&format!("kick #{uid}\n"));
        }
    }
    REGISTRY.0.borrow_mut().owners.retain(|_, o| o.eq_ignore_ascii_case(keep));
}

/// The backend that made the bot with user id `uid`, if it is still around.
pub fn owner_of(uid: i32) -> Option<String> {
    REGISTRY.0.borrow().owners.get(&uid).cloned()
}

// ---- the coupling points -----------------------------------------------------------------------

/// `GameDLLInit`: the native backend is registered first (the default), then its init.
pub fn game_init() {
    register(Rc::new(crate::bots::backend::Native));
    active().game_init();
}

pub fn map_init() {
    active().map_init();
}

pub fn precache() {
    active().precache();
}

/// `ServerActivate`'s bot manager: the quota starts a second later; then the backend's.
pub fn server_activate() {
    quota::server_activate();
    active().server_activate();
}

pub fn server_deactivate() {
    quota::server_deactivate();
    active().server_deactivate();
}

/// `StartFrame`'s bot part, before the rules' Think: the quota, then the backend's bots.
pub fn start_frame() {
    let b = active();
    quota::think(&*b);
    b.think();
}

/// `addbot` / `addcustombot` (server commands).
pub fn add_bot(req: AddBot) {
    active().add_bot(&req);
}

pub fn spawn_pre(p: &mut Player, cx: &mut Cx) {
    active().spawn_pre(p, cx);
}

pub fn spawn_post(p: &mut Player, cx: &mut Cx) {
    active().spawn_post(p, cx);
}

pub fn trace_attack(p: &mut Player, cx: &mut Cx, dmg: &DamageInfo) {
    active().trace_attack(p, cx, dmg);
}

pub fn killed(p: &mut Player, cx: &mut Cx, attacker: *mut edict_t) {
    active().killed(p, cx, attacker);
}

pub fn on_kill(killer: &mut Player, victim: *mut edict_t, now: f32) {
    active().on_kill(killer, victim, now);
}

pub fn pre_think(p: &mut Player, cx: &mut Cx) {
    active().pre_think(p, cx);
}

pub fn client_command(p: &mut Player, cx: &mut Cx, cmd: &str, args: &Args) -> bool {
    active().client_command(p, cx, cmd, args)
}

// ---- the bot player ----------------------------------------------------------------------------

/// `CREATE_FAKE_CLIENT` and `BotSetup` (0x1006b3b0), TS's `ClientPutInServer` for bots: the
/// `CBot` object with its backend `state`, "<name> connected with skill <f>" (`announce`, the
/// unclamped skill), `SetCustomDecalFrames(-1)`, `Spawn` (before the keys, so the first spawn
/// sees no model), the `model`/`team`/`cl_lw` keys (`team` only with `team_key`, which the
/// backend chooses), the team change for a given model, the rules'
/// `PlayerThink` and the `ts_playeramount` count. No `ClientConnected`, starting cash or
/// `EF_NOINTERP`, as in TS. The bot is recorded as the active backend's, and plugins hear of it
/// (`client_put_in_server`).
pub fn create_player(name: &[u8], model: &[u8], state: BotState, announce: Option<f32>, team_key: bool) -> Option<*mut edict_t> {
    let eng = engine();
    let cname = c_string(name);
    let e = eng.create_fake_client(&cname);
    if e.is_null() {
        if announce.is_some() {
            eng.alert(ALERT_TYPE::at_console, "Could not find any free client slots. Try setting maxplayers to a higher value.\n");
        }
        return None;
    }
    // `g_botNetnames[ENTINDEX(ed)]`: the netname stays this string for the bot's life.
    e.v().netname = eng.alloc_string(&cname);
    {
        e.v().flags |= FL_FAKECLIENT as i32;
        world::create::<Player>(e.v());
    }
    world::with::<Player, ()>(e, |p| {
        p.bot = Some(state);
        p.cbot = true;
        p.is_bot = 1;
        p.impulse.custom_decal_frames = crate::player::impulse::custom_decal_frames(-1);
    });
    if let Some(skill) = announce {
        eng.alert(ALERT_TYPE::at_console, &shared::bformat!("{} connected with skill {}\n", cname, format!("{skill:.6}")));
    }
    crate::ham::spawn(e);
    // The model's bytes as given (`model[0] ? model : defModel`), as the team below (#190).
    eng.set_client_key(e, "model", if model.is_empty() { &b"seal"[..] } else { model });
    if team_key {
        eng.set_client_key(e, "team", model);
    }
    eng.set_client_key(e, "cl_lw", "1");
    if !model.is_empty() && rules::state::teamplay() {
        // `CHalfLifeMultiplay::ChangePlayerTeam` is empty; teamplay's changes the team.
        with_player(e, |p, cx| rules::teams::change_team(p, cx, model, false, false));
    }
    // `g_pGameRules->PlayerThink(pl)` (0x1002c370): in intermission the new bot's button state is
    // cleared (it has pressed nothing, so the intermission isn't ended).
    if rules::state::with(|g| g.game_over) {
        with_player(e, |p, cx| {
            p.ts.buttons_pressed = 0;
            p.stunt.button_pressed = 0;
            p.stunt.button_released = 0;
            cx.pev().button = 0;
        });
    }
    crate::entities::ts::player_amount_changed(true);
    let owner = active().name().to_owned();
    REGISTRY.0.borrow_mut().owners.insert(eng.user_id(e), owner);
    crate::plugins::client_put_in_server(e, true);
    Some(e)
}

/// The client left: forget which backend made it.
pub fn client_disconnect(e: *mut edict_t) {
    let uid = engine().user_id(e);
    REGISTRY.0.borrow_mut().owners.remove(&uid);
}

/// Connected fake clients with a live user id. Guard: HL25 drops every bot when the last human
/// leaves, and driving a dropped client with `pfnRunPlayerMove` crashes the engine.
pub fn live_fake_clients() -> Vec<*mut edict_t> {
    let eng = engine();
    eng.clients().filter(|&e| e.v().flags & FL_FAKECLIENT as i32 != 0 && eng.user_id(e) != -1).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_late_backend_takes_over_native_only() {
        assert!(late_switch(true, NATIVE, "rcbot_amxx", "rcbot_amxx"), "plugin_init registered what the cvar names");
        assert!(late_switch(true, NATIVE, "RCBot_AMXX", "rcbot_amxx"), "case doesn't matter");
        assert!(!late_switch(false, NATIVE, "rcbot_amxx", "rcbot_amxx"), "before the first map start: the map start picks");
        assert!(!late_switch(true, NATIVE, NATIVE, "rcbot_amxx"), "the cvar names another");
        assert!(!late_switch(true, "test", "rcbot_amxx", "rcbot_amxx"), "a plugin backend runs: the map reload decides");
    }

    #[test]
    fn selection_by_name() {
        let names = ["native", "rcbot", "test"];
        assert_eq!(select(&names, "native"), Some(0));
        assert_eq!(select(&names, ""), Some(0), "empty is native");
        assert_eq!(select(&names, "  RCBot "), Some(1), "case and spaces don't matter");
        assert_eq!(select(&names, "test"), Some(2));
        assert_eq!(select(&names, "rcbot2"), None, "unknown");
    }

    /// A plugin backend gets `addbot`/`addcustombot`'s name and model bytes as typed, a CP1252
    /// 0xE9 included (#190).
    #[test]
    fn plugin_args_keep_bytes() {
        let custom = AddBot::Custom { name: b"Jos\xe9".to_vec(), model: b"caf\xe9".to_vec(), skill: 2.5 };
        let (n, m, s) = custom.plugin_args();
        assert_eq!((n.as_bytes(), m.as_bytes(), s), (&b"Jos\xe9"[..], &b"caf\xe9"[..], 2.5));
        let (n, m, s) = AddBot::Addbot { model: b"\xe9quipe\0x".to_vec() }.plugin_args();
        assert_eq!((n.as_bytes(), m.as_bytes(), s), (&b""[..], &b"\xe9quipex"[..], 0.0), "a NUL is dropped");
        let (n, m, _) = AddBot::Quota { name: "Bot Neo".into() }.plugin_args();
        assert_eq!((n.as_bytes(), m.as_bytes()), (&b"Bot Neo"[..], &b""[..]));
    }
}
