//! TS's bots (Windows `mp.dll` only; decomp `docs/server/bots.md`). Owner: rules work package.
//!
//! Layout (bots.md §16):
//! * this module: `addbot`/`addcustombot` (`BotCreate`/`BotCreateCustom`/`BotSetup`, §2), the
//!   `CBot` hooks (`Spawn`, `TraceAttack`, `Killed`, the kill taunt; §4, §10.1), the per-frame
//!   driver (`pfnStartFrame`, §5.3), node learning and the node commands (§11.2–11.3), and the
//!   port's `ts_bot_quota`.
//! * [`brain`]: `CBotBrain` (buttons, module mask and order, view turning, usercmd; §5).
//! * [`states`] (§6), [`modules`] (vision, buy, stunt, look around, power-up, chat; §7, §9,
//!   §10), [`attack`] (§8), [`nav`] (`CBotNav`, §11.7), [`nodes`] (`CNodeGraph`, §11.1–11.6),
//!   [`files`] (`bots.txt`, `bot_say.txt`; §3).
//!
//! A bot is a `Player` with a brain (`Player::bot`, TS's `CBot` object) and `Player::is_bot`
//! (`m_bIsBot`, `pl+0x160`). TS has bots only on listen servers. Deviation (chosen): `addbot`,
//! `addcustombot` and the port's `ts_bot_quota` bots also run on dedicated servers. Port addition:
//! `ts_bot_stop 1` freezes living bots.

pub mod attack;
pub mod brain;
pub mod files;
pub mod modules;
pub mod nav;
pub mod nodes;
pub mod states;

pub use brain::Brain;

use crate::commands::Args;
use crate::player::{Player, with_player};
use crate::rules::{self, cvars};
use crate::world::{self, Cx, DamageInfo};
use goldsrc::Global;
use goldsrc::edict::EdictPtr;
use goldsrc::ext::weapons::WeaponsEngine;
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::cell::RefCell;
use std::ffi::CString;

/// Names of the port's quota bots ("Bot Neo", ...; the xtask scenes look for the `Bot ` prefix).
const QUOTA_NAMES: &[&str] = &[
    "Bot Neo",
    "Bot Trinity",
    "Bot Morpheus",
    "Bot Niobe",
    "Bot Ghost",
    "Bot Switch",
    "Bot Apoc",
    "Bot Tank",
    "Bot Dozer",
    "Bot Mouse",
    "Bot Cypher",
    "Bot Link",
    "Bot Seraph",
    "Bot Sati",
    "Bot Rama",
    "Bot Kid",
];

/// The player models `BotCreate` picks from outside teamplay (jump table 0x1006b94c).
const BOT_MODELS: [&str; 7] = ["seal", "merc", "gordon", "hitman", "castor", "agent", "laurence"];

/// `bots.txt` and `bot_say.txt` as loaded (`g_botNames`... at 0x100db1e0).
#[derive(Default)]
pub struct Files {
    pub names: files::BotNames,
    pub say: files::BotSay,
}

#[derive(Default)]
struct Manager {
    active: bool,
    start_at: f32,
    next_add: f32,
    added: u32,
    /// The last add found the server full (the quota then retries quietly).
    full: bool,
    /// User ids of the quota's bots: it counts and kicks only these.
    quota: Vec<i32>,
}

static MGR: Global<RefCell<Option<Manager>>> = Global(RefCell::new(None));
static FILES: Global<RefCell<Option<Files>>> = Global(RefCell::new(None));

fn mgr<R>(f: impl FnOnce(&mut Manager) -> R) -> R {
    let mut m = MGR.0.borrow_mut();
    f(m.get_or_insert_with(Manager::default))
}

/// The loaded text files (no engine calls inside `f`).
pub fn with_files<R>(f: impl FnOnce(&mut Files) -> R) -> R {
    let mut m = FILES.0.borrow_mut();
    f(m.get_or_insert_with(Files::default))
}

pub fn game_init() {
    // `addbot`/`addcustombot` and the bot cvars are registered with the rules commands and cvars.
}

/// The bot part of `CTSGameRules::Init` (0x1004d910, from both rules constructors and
/// `ChangeLevel`): reset and load the node graph, then `BotLoadFiles` (0x1006bee0).
/// The files are read as TS's `fopen(<gamedir>\bots.txt)` does: from the game directory itself,
/// relative to the engine's working directory (not through the engine's search path).
pub fn ts_init() {
    nodes::map_init();
    let dir = std::path::PathBuf::from(engine().game_dir());
    let names = files::BotNames::parse(std::fs::read(dir.join("bots.txt")).ok().as_deref());
    let say = files::BotSay::parse(std::fs::read(dir.join("bot_say.txt")).ok().as_deref());
    let (count, lines) = (names.count, say.count);
    engine().print(&format!("[ts] bots: {count} names, chat lines {lines:?}, {} nodes\n", nodes::with(|g| g.count())));
    with_files(|f| *f = Files { names, say });
}

pub fn server_activate() {
    let now = engine().time();
    mgr(|m| {
        m.active = true;
        m.start_at = now + 1.0;
        m.next_add = now + 1.0;
    });
}

pub fn server_deactivate() {
    mgr(|m| m.active = false);
}

// ---- creation ------------------------------------------------------------------------------------

/// `addbot [model]`: `BotCreate` (0x1006b7c0). The argument is the model/team
/// (despite the usage text); the name always comes from `bots.txt`. Afterwards `botlearn 1`
/// when the node graph is empty.
pub fn add_bot(model: &str) {
    let eng = engine();
    let model = pick_model(model);
    let mut skill = eng.random_float(0.1, 1.0);
    let name = with_files(|f| f.names.pick(|lo, hi| eng.random_long(lo, hi), &mut skill));
    if create(&name, &model, skill, true).is_some() && nodes::with(|g| g.count()) < 1 {
        eng.server_command("botlearn 1\n");
    }
}

/// `addcustombot [name] [model] [skill]`: `BotCreateCustom` (0x1006b590). The
/// skill is random in 0.1..1 when not positive, else kept in 0.1..3; an empty name is picked from
/// `bots.txt` (whose per-name skill then wins).
pub fn add_custom_bot(name: &[u8], model: &str, skill: f32) {
    let eng = engine();
    let model = pick_model(model);
    let mut skill = if skill <= 0.0 {
        eng.random_float(0.1, 1.0)
    } else if skill as f64 >= 3.0 {
        3.0
    } else if skill as f64 <= 0.1 {
        0.1
    } else {
        skill
    };
    let name = if name.is_empty() { with_files(|f| f.names.pick(|lo, hi| eng.random_long(lo, hi), &mut skill)) } else { name.to_vec() };
    create(&name, &model, skill, true);
}

/// The model argument, else outside teamplay one of TS's seven (`RANDOM_LONG(0, 6)`).
fn pick_model(arg: &str) -> String {
    if !arg.is_empty() {
        arg.to_owned()
    } else if !rules::state::teamplay() {
        BOT_MODELS[engine().random_long(0, 6).clamp(0, 6) as usize].to_owned()
    } else {
        String::new()
    }
}

/// `CREATE_FAKE_CLIENT` and `BotSetup` (0x1006b3b0), TS's `ClientPutInServer` for bots: the
/// `CBot` object with its skill, "<name> connected with skill <f>" (`announce`; the unclamped
/// skill), `SetCustomDecalFrames(-1)`, `Spawn` (before the keys, so the first spawn sees no
/// model), the `model`/`team`/`cl_lw` keys, the team change for a given model, the rules'
/// `PlayerThink` and the `ts_playeramount` count. No `ClientConnected`, starting cash or
/// `EF_NOINTERP`, as in TS.
fn create(name: &[u8], model: &str, skill: f32, announce: bool) -> Option<*mut edict_t> {
    let eng = engine();
    let cname = CString::new(name.iter().copied().filter(|&c| c != 0).collect::<Vec<u8>>()).unwrap_or_default();
    let e = eng.create_fake_client(&cname);
    if e.is_null() {
        if announce {
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
        p.bot = Some(Box::new(Brain::new(skill)));
        p.cbot = true;
        p.is_bot = 1;
        p.impulse.custom_decal_frames = crate::player::impulse::custom_decal_frames(-1);
    });
    if announce {
        eng.alert(ALERT_TYPE::at_console, &format!("{} connected with skill {:.6}\n", cname.to_string_lossy(), skill));
    }
    crate::ham::spawn(e);
    eng.set_client_key(e, "model", if model.is_empty() { "seal" } else { model });
    eng.set_client_key(e, "team", model);
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
    Some(e)
}

/// The port's quota bot ("Bot Neo", ...): created like `addbot`'s (a random model outside
/// teamplay, a random skill in 0.1..1), but without TS's announcement, so the dedicated-server
/// tests can tell it from an `addbot`. False when the server has no free client slot (reported
/// once per streak of failed adds: the quota retries every half second).
fn add_quota_bot() -> bool {
    let eng = engine();
    let n = mgr(|m| m.added) + 1;
    let base = QUOTA_NAMES[(n as usize - 1) % QUOTA_NAMES.len()];
    let name = if n as usize > QUOTA_NAMES.len() { format!("{base} {n}") } else { base.to_owned() };
    let model = pick_model("");
    let skill = eng.random_float(0.1, 1.0);
    let Some(e) = create(name.as_bytes(), &model, skill, false) else {
        if !mgr(|m| std::mem::replace(&mut m.full, true)) {
            eng.print("[ts] bots: no free client slot\n");
        }
        return false;
    };
    let uid = eng.user_id(e);
    mgr(|m| {
        m.added = n;
        m.full = false;
        m.quota.push(uid);
    });
    eng.print(&format!("[ts] bots: added \"{name}\" (#{}) skill {skill:.2}\n", eng.index_of(e)));
    true
}

// ---- per frame -----------------------------------------------------------------------------------

/// Connected fake clients with a live user id. Guard: HL25 drops every bot when the last human
/// leaves, and driving a dropped client with `pfnRunPlayerMove` crashes the engine.
fn live_fake_clients() -> Vec<*mut edict_t> {
    let eng = engine();
    eng.clients().filter(|&e| e.v().flags & FL_FAKECLIENT as i32 != 0 && eng.user_id(e) != -1).collect()
}

/// The quota's bots that are still connected.
fn quota_bots() -> Vec<*mut edict_t> {
    let eng = engine();
    let ids = mgr(|m| m.quota.clone());
    live_fake_clients().into_iter().filter(|&e| ids.contains(&eng.user_id(e))).collect()
}

/// The bot part of `pfnStartFrame` (0x10010e00), which runs before the rules' `Think`: on a
/// listen server every player whose `m_bIsBot == 1` thinks, in entity order (`BotPlayerByIndex`
/// 0x1006b360: `FL_FAKECLIENT` or `FL_CLIENT`, with private data). Port: the quota first, and
/// bots think on dedicated servers too.
pub fn start_frame() {
    let eng = engine();
    let now = eng.time();
    quota_think(now);
    let frozen = cvars::f(c"ts_bot_stop") != 0.0;
    for i in 1..=eng.max_clients() {
        let e = eng.player(i);
        if e.is_null() || e.is_free() || !e.has_private_data() || e.v().flags & (FL_FAKECLIENT | FL_CLIENT) as i32 == 0 {
            continue;
        }
        if eng.user_id(e) == -1 {
            continue;
        }
        let Some(Some(cmd)) = with_player(e, |p, cx| if p.is_bot == 1 { brain::think(p, cx, frozen) } else { None }) else {
            continue;
        };
        // `pfnRunPlayerMove`, outside the player's dispatch: the engine runs its PreThink/PostThink.
        eng.run_player_move(e, cmd.angles, cmd.forward, cmd.side, 0.0, cmd.buttons, 0, cmd.msec);
    }
}

/// `ts_bot_quota N` (port): keep N quota bots, adding or kicking one every half second. A quota
/// bot that lost its brain (a client the engine kept across a level change) gets a new one.
fn quota_think(now: f32) {
    let eng = engine();
    let (active, start_at, next_add) = mgr(|m| (m.active, m.start_at, m.next_add));
    if !active || now < start_at {
        return;
    }
    let want = cvars::f(c"ts_bot_quota").max(0.0) as usize;
    let bots = quota_bots();
    for &e in &bots {
        world::with::<Player, ()>(e, |p| {
            if p.bot.is_none() {
                p.bot = Some(Box::new(Brain::new(1.0)));
                p.cbot = true;
                p.is_bot = 1;
            }
        });
    }
    if now < next_add {
        return;
    }
    if bots.len() < want {
        mgr(|m| m.next_add = now + 0.5);
        add_quota_bot();
    } else if bots.len() > want {
        mgr(|m| m.next_add = now + 0.5);
        if let Some(&e) = bots.last() {
            eng.server_command(&format!("kick #{}\n", eng.user_id(e)));
        }
    }
}

// ---- CBot overrides and hooks -------------------------------------------------------------------

/// The head of `CBot::Spawn` (0x1006bb40): `CBotBrain::Reset`, before `CBasePlayer::Spawn`.
pub fn spawn_pre(p: &mut Player, cx: &mut Cx) {
    let now = cx.time();
    let mode = rules::state::with(|g| g.mode);
    if let Some(b) = p.bot.as_mut() {
        b.reset(now, mode);
    }
}

/// The tail of `CBot::Spawn`: a random laser-sight colour (`PlInfo` key 0: 0 for 76/101, 1 for
/// 15/101, 2 for 10/101), as the `lasclr_<n>` command sends it.
pub fn spawn_post(p: &mut Player, cx: &mut Cx) {
    if !p.cbot {
        return;
    }
    let r = engine().random_long(0, 100);
    let laser = if r > 90 {
        2
    } else if r > 75 {
        1
    } else {
        0
    };
    crate::messages::all(shared::protocol::Msg::PlInfo).byte(cx.index()).byte(0).byte(laser).end();
}

/// `CBot::TraceAttack` (0x1006bac0), after `CBasePlayer::TraceAttack`: a player's hit turns the
/// wanted view toward the attacker's origin (no enemy or goal is set). TS's `GetClassPtr` would
/// allocate a `CBaseEntity` for an attacker without private data; the port only looks.
pub fn trace_attack(p: &mut Player, cx: &mut Cx, dmg: &DamageInfo) {
    let a = dmg.attacker;
    if !p.cbot || a.is_null() || !rules::has_player(a) {
        return;
    }
    let target = a.v().origin;
    if let Some(b) = p.bot.as_mut() {
        b.look_at(cx.pev(), target);
    }
}

/// `CBot::Killed` (0x1006ba00), after `CBasePlayer::Killed` (whose `PlayerKilled` may already
/// have queued the killer bot's taunt): killed by a player, 10/101 to answer from the `killed`
/// section with the killer's name in 0.5..2 s (a player-class suicide uses the bot's own).
pub fn killed(p: &mut Player, cx: &mut Cx, attacker: *mut edict_t) {
    if !p.cbot {
        return;
    }
    let a = if attacker.is_null() { engine().entity_of_index(0) } else { attacker };
    if a.is_null() || !rules::has_player(a) {
        return;
    }
    let eng = engine();
    if eng.random_long(0, 100) > 90 {
        let name = eng.string(a.v().netname).to_bytes().to_vec();
        let now = cx.time();
        if let Some(b) = p.bot.as_mut() {
            b.chat_name = name;
            b.chat_section = files::KILLED;
            b.next_chat = (eng.random_float(0.5, 2.0) as f64 + now as f64) as f32;
        }
    }
}

/// `CBot::OnKill(pevVictim)` (0x1006b970), from `CHalfLifeMultiplay::PlayerKilled` when the killer
/// is a bot (`m_bIsBot != 0`) and the kill scored points: 10/101 to taunt the victim from the
/// `offense` section in 0.5..2 s. `killer` is the killer's `Player`.
pub fn on_kill(killer: &mut Player, victim: *mut edict_t, now: f32) {
    if killer.is_bot == 0 {
        return;
    }
    let eng = engine();
    if eng.random_long(0, 100) > 90 {
        let name = eng.string(victim.v().netname).to_bytes().to_vec();
        // Guard: the `CBot` members exist only on a bot; TS's flag and class always agree.
        if let Some(b) = killer.bot.as_mut() {
            b.chat_name = name;
            b.chat_section = files::OFFENSE;
            b.next_chat = (eng.random_float(0.5, 2.0) as f64 + now as f64) as f32;
        }
    }
}

/// The node-learning block of `CBasePlayer::PreThink` (0x1005fed2), for every living player that
/// isn't observing: while `botlearn > 0`, a node at the player's origin every 0.1 s.
pub fn learn(p: &mut Player, cx: &mut Cx) {
    let v = cx.pev();
    if v.iuser1 != 0 || v.deadflag > DEAD_NO as i32 {
        return;
    }
    let (origin, ladder) = (v.origin, nav::on_ladder(v));
    nodes::learn(&mut p.next_node_learn, cx.time(), origin, ladder);
}

/// The node commands of `pfnClientCommand` (0x1000f8a0): `testpath`, `resetnodes`, `loadnodes`,
/// `savenodes`, `addnode`. Any client may use them (TS has no access check).
pub fn client_command(_p: &mut Player, cx: &mut Cx, cmd: &str, args: &Args) -> bool {
    if !matches!(cmd, "testpath" | "resetnodes" | "loadnodes" | "savenodes" | "addnode") {
        return false;
    }
    let argv: Vec<String> = (0..args.argc()).map(|i| args.argv(i).to_string_lossy().into_owned()).collect();
    let v = cx.pev();
    let playing = rules::alive(v) && v.iuser1 == 0;
    let (origin, ladder) = (v.origin, nav::on_ladder(v));
    nodes::client_command(cmd, &argv, playing, origin, ladder)
}

/// Finds a TS content directory (`ts_original`, `ts_custom`) next to one of the checkout's
/// parents, as xtask does (tests only).
#[cfg(test)]
pub fn testdata_content_dir(name: &str) -> Option<std::path::PathBuf> {
    let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    here.ancestors().map(|a| a.join(name)).find(|d| d.join("maps").is_dir())
}
