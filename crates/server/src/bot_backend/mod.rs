//! Bot backends (port addition, issue #104; docs/implementation/bot-plugins.md). TS's bots are
//! one backend among several: the server reaches them only through [`BotBackend`], and the
//! server cvar `ts_bot_backend` picks the backend when a map starts ([`map_start`]).
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

/// What `addbot`, `addcustombot` and the quota ask a backend for.
#[derive(Clone, Debug, PartialEq)]
pub enum AddBot {
    /// `addbot [model]`.
    Addbot { model: String },
    /// `addcustombot [name] [model] [skill]` (skill as typed, 0 when missing).
    Custom { name: Vec<u8>, model: String, skill: f32 },
    /// One of the port's `ts_bot_quota` bots, named by the host ("Bot Neo", ...).
    Quota { name: String },
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
    backends: Vec<Rc<dyn BotBackend>>,
    active: Option<Rc<dyn BotBackend>>,
    /// `ts_bot_backend` values already warned about.
    warned: Vec<String>,
    /// Which backend made each bot still around, by user id.
    owners: BTreeMap<i32, String>,
    /// The plugin that registered each plugin backend (by its index in `backends`).
    plugins: BTreeMap<usize, ts_plugin_api::PluginId>,
}

static REGISTRY: Global<RefCell<Registry>> = Global(RefCell::new(Registry {
    backends: Vec::new(),
    active: None,
    warned: Vec::new(),
    owners: BTreeMap::new(),
    plugins: BTreeMap::new(),
}));

/// Registers a backend (the native one at `GameDLLInit`). False if the name is taken
/// (case-insensitive, `native` included).
pub fn register(b: Rc<dyn BotBackend>) -> bool {
    let mut r = REGISTRY.0.borrow_mut();
    if r.backends.iter().any(|x| x.name().eq_ignore_ascii_case(b.name())) {
        return false;
    }
    r.backends.push(b);
    true
}

/// Registers a plugin's backend (`PluginApi::register_bot_backend`), as [`register`].
pub fn register_plugin(plugin: ts_plugin_api::PluginId, b: Rc<dyn BotBackend>) -> bool {
    if !register(b) {
        return false;
    }
    let mut r = REGISTRY.0.borrow_mut();
    let i = r.backends.len() - 1;
    r.plugins.insert(i, plugin);
    true
}

/// Drops the backends of a plugin that failed to load (before any map start used them).
pub fn forget_plugin(plugin: ts_plugin_api::PluginId) {
    let mut r = REGISTRY.0.borrow_mut();
    let gone: Vec<usize> = r.plugins.iter().filter(|(_, p)| **p == plugin).map(|(i, _)| *i).collect();
    for &i in gone.iter().rev() {
        r.backends.remove(i);
    }
    let kept: BTreeMap<usize, ts_plugin_api::PluginId> =
        r.plugins.iter().filter(|(_, p)| **p != plugin).map(|(&i, &p)| (i - gone.iter().filter(|&&g| g < i).count(), p)).collect();
    r.plugins = kept;
}

/// The plugin whose backend is active, if a plugin's is.
pub fn active_plugin() -> Option<ts_plugin_api::PluginId> {
    let r = REGISTRY.0.borrow();
    let a = r.active.as_ref()?;
    let i = r.backends.iter().position(|b| Rc::ptr_eq(b, a))?;
    r.plugins.get(&i).copied()
}

/// Whether a backend of that name is registered.
pub fn is_registered(name: &str) -> bool {
    REGISTRY.0.borrow().backends.iter().any(|x| x.name().eq_ignore_ascii_case(name))
}

/// The active backend: the one the last map start selected (native until then).
pub fn active() -> Rc<dyn BotBackend> {
    let r = REGISTRY.0.borrow();
    if let Some(a) = &r.active {
        return a.clone();
    }
    r.backends.first().cloned().unwrap_or_else(|| Rc::new(crate::bots::backend::Native))
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

/// Map start (`rules::install`, after `game.cfg` ran and before the rules' `Init`): reads
/// `ts_bot_backend` and makes that backend active. A change deactivates the old one and kicks the
/// bots it made (the quota adds the new backend's). An unknown name warns once and keeps native.
pub fn map_start() {
    let eng = engine();
    let value = eng.cvar_string(c"ts_bot_backend").to_string_lossy().into_owned();
    let (old, new, unknown) = {
        let r = REGISTRY.0.borrow();
        let names: Vec<&str> = r.backends.iter().map(|b| b.name()).collect();
        let pick = select(&names, &value);
        let native = names.iter().position(|n| *n == NATIVE).unwrap_or(0);
        let new = r.backends.get(pick.unwrap_or(native)).cloned();
        (r.active.clone(), new, pick.is_none())
    };
    let Some(new) = new else { return };
    if unknown && !REGISTRY.0.borrow().warned.contains(&value) {
        REGISTRY.0.borrow_mut().warned.push(value.clone());
        eng.print(&format!("[ts] bots: unknown ts_bot_backend \"{value}\", using native\n"));
    }
    let changed = old.as_ref().is_none_or(|o| !Rc::ptr_eq(o, &new));
    if !changed {
        return;
    }
    if let Some(o) = &old {
        o.deactivate();
        kick_all_but(new.name());
    }
    REGISTRY.0.borrow_mut().active = Some(new.clone());
    eng.print(&format!("[ts] bots: backend {}\n", new.name()));
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
pub fn create_player(name: &[u8], model: &str, state: BotState, announce: Option<f32>, team_key: bool) -> Option<*mut edict_t> {
    let eng = engine();
    let cname = CString::new(name.iter().copied().filter(|&c| c != 0).collect::<Vec<u8>>()).unwrap_or_default();
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
        eng.alert(ALERT_TYPE::at_console, &format!("{} connected with skill {:.6}\n", cname.to_string_lossy(), skill));
    }
    crate::ham::spawn(e);
    eng.set_client_key(e, "model", if model.is_empty() { "seal" } else { model });
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
    fn selection_by_name() {
        let names = ["native", "rcbot", "test"];
        assert_eq!(select(&names, "native"), Some(0));
        assert_eq!(select(&names, ""), Some(0), "empty is native");
        assert_eq!(select(&names, "  RCBot "), Some(1), "case and spaces don't matter");
        assert_eq!(select(&names, "test"), Some(2));
        assert_eq!(select(&names, "rcbot2"), None, "unknown");
    }
}
