//! `tsmod_test`: the stub `tsmod_*` plugin (issue #104; docs/implementation/bot-plugins.md §6).
//! It registers a bot backend named after itself (`ts_bot_backend test`) whose bots walk in
//! circles and jump, and it calls the plugin API once each way, logging `[tsmod_test] check <what>
//! ok` lines that `cargo xtask plugins` looks for.
//!
//! Loaded under other names it exercises the loader's refusals: `tsmod_test_badver` reports
//! another API version, `tsmod_test_badsize` a wrong descriptor size, and `tsmod_test_refuse`
//! returns an error (after registering a command and a backend, which the server then drops).
//!
//! The plugin keeps its state behind a mutex, which it never holds while calling the API: the
//! server calls back into the plugin from inside API calls (`create_bot` spawns the bot).

#![warn(clippy::undocumented_unsafe_blocks)]
#![allow(clippy::missing_safety_doc)] // the only export is the plugin entry point, called by the server

use goldsrc_sys::{edict_t, enginefuncs_t, globalvars_t};
use std::ffi::{CStr, CString, c_char, c_int};
use std::sync::Mutex;
use std::sync::atomic::{AtomicPtr, Ordering};
use ts_plugin_api::{
    BotBackend, BotCmd, DEFERRED, EntityInfo, GameState, HamCall, INVALID, LoadInfo, NOT_PLAYER, OK, ObjectiveInfo, PlayerInfo, Plugin,
    PluginApi, PluginId, VERSION, WeaponEntry, entity, ham, item, log,
};

/// TS weapon id of the M4 (`shared::weapons::id::M4`), bought for the test bots.
const M4: c_int = 5;
/// `IN_JUMP`.
const IN_JUMP: c_int = goldsrc_sys::IN_JUMP as c_int;
/// hamsandwich's `Ham_Item_PreFrame`: the gun's `ItemPreFrame`, every frame a living player's
/// `m_flNextAttack` has run out.
const HAM_ITEM_PREFRAME: c_int = 70;

#[derive(Clone, Copy)]
struct Bot {
    idx: c_int,
    /// Where it was when first seen alive this life, and the farthest it got from there.
    start: Option<[f32; 3]>,
    moved: f32,
    yaw: f32,
    next_jump: f32,
    next_respawn: f32,
    /// The checks run once per bot (API calls on a bot of ours).
    probed: bool,
}

struct State {
    id: PluginId,
    name: String,
    active: bool,
    added: u32,
    bots: Vec<Bot>,
    last_time: f32,
    next_report: f32,
    /// The `check` lines already logged.
    done: Vec<&'static str>,
}

/// The server's table, from `TSRS_PluginLoad`.
static API: AtomicPtr<PluginApi> = AtomicPtr::new(std::ptr::null_mut());

static STATE: Mutex<State> = Mutex::new(State {
    id: 0,
    name: String::new(),
    active: false,
    added: 0,
    bots: Vec::new(),
    last_time: 0.0,
    next_report: 0.0,
    done: Vec::new(),
});

/// Runs `f` on the state, or not at all when it is busy further up the stack.
fn with<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.try_lock().ok().map(|mut s| f(&mut s))
}

fn api() -> Option<&'static PluginApi> {
    let p = API.load(Ordering::Relaxed);
    // SAFETY: a non-NULL pointer is the server's static table, which lives for the process.
    (!p.is_null()).then(|| unsafe { &*p })
}

fn id() -> PluginId {
    with(|s| s.id).unwrap_or(0)
}

fn say(level: c_int, msg: &str) {
    let (Some(a), Ok(m)) = (api(), CString::new(msg)) else { return };
    // SAFETY: `log` is the server's; the id is ours and `m` a NUL-terminated string for the call.
    unsafe { (a.log)(id(), level, m.as_ptr()) };
}

/// Logs `check <what> ok` (or FAIL) once per name.
fn check(what: &'static str, ok: bool) {
    let first = with(|s| {
        if s.done.contains(&what) {
            false
        } else {
            s.done.push(what);
            true
        }
    });
    if first == Some(true) || !ok {
        say(if ok { log::INFO } else { log::ERROR }, &format!("check {what} {}", if ok { "ok" } else { "FAIL" }));
    }
}

/// The server's engine table and globals, from the API.
fn engine() -> Option<(&'static enginefuncs_t, &'static globalvars_t)> {
    let a = api()?;
    let (f, g) = (a.engine_funcs.cast::<enginefuncs_t>(), a.globals.cast::<globalvars_t>().cast_const());
    // SAFETY: the API hands out the engine's table and globals, which live for the process; the
    // plugin only reads them, on the engine's thread.
    (!f.is_null() && !g.is_null()).then(|| unsafe { (&*f, &*g) })
}

/// The origin of entity `idx`, through the engine's own functions.
fn origin(idx: c_int) -> Option<[f32; 3]> {
    let (f, _) = engine()?;
    // SAFETY: `pfnPEntityOfEntIndex` takes any index and returns NULL or a live edict; its
    // `entvars_t` is the engine's, read here and not kept.
    unsafe {
        let e: *mut edict_t = (f.pfnPEntityOfEntIndex?)(idx);
        (!e.is_null()).then(|| (*e).v.origin)
    }
}

fn info(idx: c_int) -> Option<PlayerInfo> {
    let a = api()?;
    let mut i = PlayerInfo::default();
    // SAFETY: `i` is a `PlayerInfo` valid for writes.
    (unsafe { (a.player_info)(idx, &mut i) } == OK).then_some(i)
}

// ---- the entry point -----------------------------------------------------------------------------

/// `TSRS_PluginLoad`: checks the server's table, registers the backend and a status command, and
/// fills the descriptor (or misbehaves on purpose under the test names, module docs).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn TSRS_PluginLoad(
    api: *const PluginApi,
    api_size: u32,
    info: *const LoadInfo,
    out: *mut Plugin,
    out_size: u32,
) -> c_int {
    if api.is_null() || info.is_null() || out.is_null() || (out_size as usize) < size_of::<Plugin>() {
        return INVALID;
    }
    // SAFETY: the server passes its static table, valid for the process.
    let a: &'static PluginApi = unsafe { &*api };
    if (api_size as usize) < size_of::<PluginApi>() || a.version != VERSION {
        return INVALID;
    }
    // SAFETY: `info` and its strings live for the call.
    let (pid, name) = unsafe { ((*info).id, CStr::from_ptr((*info).name).to_string_lossy().into_owned()) };
    API.store(api.cast_mut(), Ordering::Relaxed);
    with(|s| {
        s.id = pid;
        s.name = name.clone();
    });
    if let Ok(cname) = CString::new(name.as_str()) {
        // SAFETY: the server copies the descriptor and reads the name during the call.
        let rc = unsafe { (a.register_bot_backend)(pid, cname.as_ptr(), &BACKEND) };
        say(log::INFO, &format!("loaded as plugin {pid}; bot backend \"{name}\": {rc}"));
    }
    let cmd = CString::new(format!("tsmod_{name}_status")).unwrap_or_default();
    // SAFETY: as above; `status` lives for the process.
    unsafe { (a.register_command)(pid, cmd.as_ptr(), status) };
    let mut desc = DESCRIPTOR;
    match name.as_str() {
        "test_refuse" => return INVALID,
        "test_badver" => desc.api_version = VERSION + 1,
        "test_badsize" => desc.size = 8,
        _ => {}
    }
    // SAFETY: `out` is a `Plugin` the server owns, writable (size checked above).
    unsafe { out.write(desc) };
    OK
}

const DESCRIPTOR: Plugin = Plugin {
    name: c"tsmod_test".as_ptr(),
    version: c"1".as_ptr(),
    game_init: Some(game_init),
    server_activate: Some(server_activate),
    start_frame: Some(start_frame),
    client_command: Some(client_command),
    player_spawned: Some(player_spawned),
    player_killed: Some(player_killed),
    sound: Some(sound),
    weapon_changed: Some(weapon_changed),
    ..Plugin::EMPTY
};

const BACKEND: BotBackend = BotBackend {
    size: size_of::<BotBackend>() as u32,
    add_bot: Some(add_bot),
    activate: Some(activate),
    deactivate: Some(deactivate),
    adopt_bot: Some(adopt_bot),
};

// ---- plugin callbacks ------------------------------------------------------------------------------

extern "C" fn game_init() {
    let Some(a) = api() else { return };
    let mut buf = [0 as c_char; 512];
    // SAFETY: `buf` is writable for its length.
    let n = unsafe { (a.data_dir)(id(), buf.as_mut_ptr(), buf.len() as c_int) };
    // SAFETY: `data_dir` wrote a NUL-terminated string into `buf`.
    let dir = unsafe { CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().into_owned();
    let name = with(|s| s.name.clone()).unwrap_or_default();
    let tail = |sep: char| format!("tsmod{sep}{name}{sep}");
    check("data_dir", n > 0 && (dir.ends_with(&tail('/')) || dir.ends_with(&tail('\\'))));
    // The gun's entity hooks: `Ham_Item_PreFrame` on our bots' `weapon_tsgun`.
    (a.ham_set_hook)(Some(ham_hook));
    let rc = (a.ham_watch)(HAM_ITEM_PREFRAME, 1);
    say(log::INFO, &format!("ham_watch(Item_PreFrame): {rc}"));
}

/// The entity hook: `Item_PreFrame` must come on a `weapon_tsgun` (class `weapon_tsgun`, kind
/// `GUN`) held by one of our bots.
unsafe extern "C" fn ham_hook(call: *mut HamCall, post: c_int) -> c_int {
    let Some(a) = api() else { return ham::action::CONTINUE };
    // SAFETY: the server passes a `HamCall` valid for the call.
    let (ham, this, class) = unsafe { ((*call).ham, (*call).this, (*call).class) };
    if ham != HAM_ITEM_PREFRAME || post == 0 {
        return ham::action::CONTINUE;
    }
    let mut e = EntityInfo::default();
    // SAFETY: `e` is writable.
    let rc = unsafe { (a.entity_info)(this, &mut e) };
    let ours = with(|s| s.bots.iter().any(|b| b.idx == e.owner)).unwrap_or(false);
    // SAFETY: a NUL-terminated name for the call.
    let gun_class = unsafe { (a.class_id)(c"weapon_tsgun".as_ptr()) };
    if ours {
        check("gun_hook", rc == OK && e.kind == entity::GUN && class == gun_class && (a.ham_supported)(ham, class) == 1);
    }
    ham::action::CONTINUE
}

unsafe extern "C" fn server_activate(map: *const c_char) {
    let Some(a) = api() else { return };
    let mut buf = [0 as c_char; 64];
    // SAFETY: `buf` is writable for its length; `map` is the server's string for the call.
    let ok = unsafe { (a.map_name)(buf.as_mut_ptr(), buf.len() as c_int) > 0 && CStr::from_ptr(buf.as_ptr()) == CStr::from_ptr(map) };
    check("map_name", ok);
}

/// Every frame: each of our bots walks forward, turning, and jumps now and then; a dead one asks
/// to respawn. Every 3 s a line per bot.
extern "C" fn start_frame() {
    let Some((a, (_, g))) = api().zip(engine()) else { return };
    let now = g.time;
    let Some((active, bots, dt, report)) = with(|s| {
        let dt = (now - s.last_time).clamp(0.001, 0.1);
        s.last_time = now;
        let report = now >= s.next_report;
        if report {
            s.next_report = now + 3.0;
        }
        (s.active, s.bots.clone(), dt, report)
    }) else {
        return;
    };
    if !active {
        return;
    }
    let mut kept = Vec::new();
    for mut b in bots {
        let Some(i) = info(b.idx) else { continue };
        if b.probed || probe(a, b.idx, &i) {
            b.probed = true;
        }
        if i.alive == 0 {
            b.start = None;
            if now >= b.next_respawn {
                b.next_respawn = now + 1.0;
                // SAFETY: a NUL-terminated command for the call.
                unsafe { (a.bot_command)(b.idx, c"respawn".as_ptr()) };
            }
            kept.push(b);
            continue;
        }
        if let Some(o) = origin(b.idx) {
            let s = *b.start.get_or_insert(o);
            let d = ((o[0] - s[0]).powi(2) + (o[1] - s[1]).powi(2)).sqrt();
            b.moved = b.moved.max(d);
            if b.moved > 64.0 {
                check("moved", true);
            }
        }
        b.yaw = (b.yaw + 45.0 * dt) % 360.0;
        let jump = now >= b.next_jump;
        if jump {
            b.next_jump = now + 1.5;
        }
        let cmd = BotCmd {
            angles: [0.0, b.yaw, 0.0],
            forward: 250.0,
            buttons: if jump { IN_JUMP } else { 0 },
            msec: (dt * 1000.0).round().clamp(1.0, 100.0) as c_int,
            ..BotCmd::default()
        };
        // SAFETY: `cmd` is a `BotCmd` valid for the call.
        let rc = unsafe { (a.run_cmd)(b.idx, &cmd) };
        check("run_cmd", rc == OK);
        if report {
            say(log::INFO, &format!("bot #{} weapon={} moved={:.0} alive={}", b.idx, i.weapon, b.moved, i.alive));
        }
        kept.push(b);
    }
    with(|s| {
        // Bots added during the loop (none in practice) are kept too.
        let extra: Vec<Bot> = s.bots.iter().filter(|x| !kept.iter().any(|k| k.idx == x.idx)).copied().collect();
        s.bots = kept.into_iter().chain(extra).collect();
    });
}

/// The API calls on one of our living bots, once: true when done.
fn probe(a: &PluginApi, idx: c_int, i: &PlayerInfo) -> bool {
    if i.alive == 0 {
        return false;
    }
    check("player_info", i.is_bot == 1);
    let mut e = EntityInfo::default();
    // SAFETY: `e` is writable.
    let rc = unsafe { (a.entity_info)(idx, &mut e) };
    check("entity_info", rc == OK && e.kind == entity::PLAYER);
    let mut g = GameState::default();
    // SAFETY: `g` is writable.
    let rc = unsafe { (a.game_state)(&mut g) };
    check("game_state", rc == OK && g.max_clients > 0 && g.time > 0.0);
    check("team_scores", g.team_scores.iter().all(|&t| t >= 0));
    let mut objs = [ObjectiveInfo::default(); 8];
    // SAFETY: `objs` is writable for 8 entries; a NULL buffer only counts.
    let (n_bot, n_none, bad) = unsafe {
        (
            (a.objectives)(idx, objs.as_mut_ptr(), objs.len() as c_int),
            (a.objectives)(0, std::ptr::null_mut(), 0),
            (a.objectives)(-1, std::ptr::null_mut(), 0),
        )
    };
    check("objectives", n_bot >= 0 && n_bot == n_none && bad == NOT_PLAYER);
    check("create_entity", create_entity(a));
    let mut inv = [WeaponEntry::default(); 40];
    // SAFETY: `inv` is writable for 40 entries.
    let n = unsafe { (a.player_inventory)(idx, inv.as_mut_ptr(), inv.len() as c_int) };
    check("inventory", n >= 0);
    // Through the client command path, to our own `client_command`.
    // SAFETY: a NUL-terminated command for the call.
    let rc = unsafe { (a.bot_command)(idx, c"tsmod_test_ping 1".as_ptr()) };
    check("bot_command", rc == OK || rc == DEFERRED);
    true
}

/// Creates an `info_target` from keys (as RCBot's `givetsweapon` makes a `ts_groundweapon`),
/// spawns it, checks it, and removes it again. An unknown class is refused.
fn create_entity(a: &PluginApi) -> bool {
    // SAFETY: NUL-terminated strings for the API calls; `i` is writable; the engine's
    // `pfnPEntityOfEntIndex` returns the live edict we created, which `pfnRemoveEntity` frees.
    unsafe {
        if (a.create_entity)(c"tsmod_test_no_such_class".as_ptr()) != INVALID {
            return false;
        }
        let e = (a.create_entity)(c"info_target".as_ptr());
        if e <= 0 {
            return false;
        }
        let kv = (a.entity_key_value)(e, c"targetname".as_ptr(), c"tsmod_test_target".as_ptr());
        let placed = (a.entity_key_value)(e, c"origin".as_ptr(), c"1 2 3".as_ptr());
        let spawned = (a.spawn_entity)(e);
        let mut i = EntityInfo::default();
        let ok = kv == 1 && placed == 1 && spawned == OK && (a.entity_info)(e, &mut i) == OK && origin(e) == Some([1.0, 2.0, 3.0]);
        if let Some((f, _)) = engine()
            && let (Some(get), Some(remove)) = (f.pfnPEntityOfEntIndex, f.pfnRemoveEntity)
        {
            remove(get(e));
        }
        ok
    }
}

unsafe extern "C" fn client_command(_player: c_int, argc: c_int, argv: *const *const c_char) -> c_int {
    if argc < 1 || argv.is_null() {
        return 0;
    }
    // SAFETY: the server passes `argc` NUL-terminated strings for the call.
    let cmd = unsafe { CStr::from_ptr(*argv) };
    if cmd != c"tsmod_test_ping" {
        return 0;
    }
    check("client_command", argc == 2);
    1
}

fn ours(idx: c_int) -> bool {
    with(|s| s.bots.iter().any(|b| b.idx == idx)).unwrap_or(false)
}

/// A spawn of one of our bots: an M4 and kevlar for the next life, and the M4 in hand once it
/// has one.
extern "C" fn player_spawned(idx: c_int) {
    let Some(a) = api() else { return };
    if !ours(idx) {
        return;
    }
    check("spawned", true);
    let rc = (a.buy)(idx, M4, 0, 0, 4);
    check("buy", rc == OK || rc == DEFERRED);
    let rc = (a.buy_item)(idx, item::KEVLAR);
    check("buy_item", rc == OK || rc == DEFERRED);
    let mut inv = [WeaponEntry::default(); 40];
    // SAFETY: `inv` is writable for 40 entries.
    let n = unsafe { (a.player_inventory)(idx, inv.as_mut_ptr(), inv.len() as c_int) };
    if inv.iter().take(n.max(0) as usize).any(|w| w.weapon == M4) {
        let rc = (a.select_weapon)(idx, M4);
        check("select_weapon", rc == OK || rc == DEFERRED);
    }
}

extern "C" fn player_killed(victim: c_int, _attacker: c_int, _inflictor: c_int) {
    if ours(victim) {
        check("killed", true);
    }
}

unsafe extern "C" fn sound(_entity: c_int, origin: *const f32, sample: *const c_char, volume: f32, _attenuation: f32) {
    check("sound", !origin.is_null() && !sample.is_null() && (0.0..=1.0).contains(&volume));
}

extern "C" fn weapon_changed(idx: c_int, _weapon: c_int) {
    if ours(idx) {
        check("weapon_changed", true);
    }
}

/// `tsmod_<name>_status`: our bots.
unsafe extern "C" fn status(_argc: c_int, _argv: *const *const c_char) {
    let bots = with(|s| s.bots.clone()).unwrap_or_default();
    say(log::INFO, &format!("{} bots", bots.len()));
    for b in bots {
        let i = info(b.idx).unwrap_or_default();
        say(log::INFO, &format!("  #{} alive={} weapon={} team={} moved={:.0}", b.idx, i.alive, i.weapon, i.team, b.moved));
    }
}

// ---- the bot backend -------------------------------------------------------------------------------

/// `addbot`, `addcustombot`, the quota: a bot named as asked, else "Test Bot <n>".
unsafe extern "C" fn add_bot(name: *const c_char, model: *const c_char, skill: f32) -> c_int {
    let Some(a) = api() else { return 0 };
    let n = with(|s| {
        s.added += 1;
        s.added
    })
    .unwrap_or(1);
    // SAFETY: the server passes NUL-terminated strings for the call.
    let given = unsafe { CStr::from_ptr(name) };
    // A given name goes back to `create_bot` as its bytes (the API's strings are bytes, #190).
    let cname = if given.is_empty() { CString::new(format!("Test Bot {n}")).unwrap_or_default() } else { given.to_owned() };
    let name = cname.to_string_lossy().into_owned();
    // SAFETY: as above; `create_bot` reads the strings during the call.
    let idx = unsafe { (a.create_bot)(id(), cname.as_ptr(), model, skill) };
    if idx > 0 {
        with(|s| {
            s.bots.push(Bot { idx, start: None, moved: 0.0, yaw: (n * 97 % 360) as f32, next_jump: 0.0, next_respawn: 0.0, probed: false })
        });
        say(log::INFO, &format!("added \"{name}\" (#{idx})"));
        check("create_bot", true);
    } else {
        say(log::WARNING, &format!("create_bot \"{name}\": {idx}"));
    }
    idx
}

extern "C" fn activate() {
    with(|s| s.active = true);
    say(log::INFO, "bot backend active");
}

extern "C" fn deactivate() {
    with(|s| {
        s.active = false;
        s.bots.clear();
    });
    say(log::INFO, "bot backend inactive");
}

extern "C" fn adopt_bot(idx: c_int) {
    with(|s| s.bots.push(Bot { idx, start: None, moved: 0.0, yaw: 0.0, next_jump: 0.0, next_respawn: 0.0, probed: false }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_descriptor_matches_this_api() {
        assert_eq!(DESCRIPTOR.size as usize, size_of::<Plugin>());
        assert_eq!(DESCRIPTOR.api_version, VERSION);
        assert_eq!(BACKEND.size as usize, size_of::<BotBackend>());
    }

    #[test]
    fn a_null_or_short_call_is_refused() {
        let mut out = Plugin::EMPTY;
        // SAFETY: NULL arguments are refused before anything is read.
        let rc = unsafe { TSRS_PluginLoad(std::ptr::null(), 0, std::ptr::null(), &mut out, size_of::<Plugin>() as u32) };
        assert_eq!(rc, INVALID);
    }
}
