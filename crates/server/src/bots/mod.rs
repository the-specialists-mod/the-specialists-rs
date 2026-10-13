//! TS's bots (Windows `mp.dll` only; decomp `docs/server/bots.md`). Owner: rules work package.
//!
//! These are the **native** bot backend (`ts_bot_backend native`, the default): the server
//! reaches them only through [`crate::bot_backend::BotBackend`], which [`backend::Native`]
//! implements (docs/implementation/bot-plugins.md). The fake client itself (`BotSetup`), the
//! port's `ts_bot_quota` and the bot manager's timing are the host's, in [`crate::bot_backend`].
//!
//! Layout (bots.md §16):
//! * this module: `addbot`/`addcustombot` (`BotCreate`/`BotCreateCustom`, §2), the
//!   `CBot` hooks (`Spawn`, `TraceAttack`, `Killed`, the kill taunt; §4, §10.1), the per-frame
//!   driver (`pfnStartFrame`, §5.3), node learning and the node commands (§11.2–11.3).
//! * [`backend`]: the [`BotBackend`](crate::bot_backend::BotBackend) implementation.
//! * [`brain`]: `CBotBrain` (buttons, module mask and order, view turning, usercmd; §5).
//! * [`states`] (§6), [`modules`] (vision, buy, stunt, look around, power-up, chat; §7, §9,
//!   §10), [`attack`] (§8), [`nav`] (`CBotNav`, §11.7), [`nodes`] (`CNodeGraph`, §11.1–11.6),
//!   [`files`] (`bots.txt`, `bot_say.txt`; §3).
//!
//! A bot is a `Player` with a brain (the backend state in `Player::bot`, TS's `CBot` object: see
//! [`brain_of`]) and `Player::is_bot` (`m_bIsBot`, `pl+0x160`). TS has bots only on listen servers. Deviation (chosen): `addbot`,
//! `addcustombot` and the port's `ts_bot_quota` bots also run on dedicated servers. Port addition:
//! `ts_bot_stop 1` freezes living bots.

pub mod attack;
pub mod backend;
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
use crate::world::{Cx, DamageInfo};
use goldsrc::Global;
use goldsrc::edict::EdictPtr;
use goldsrc::ext::weapons::WeaponsEngine;
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::cell::RefCell;

/// The player models `BotCreate` picks from outside teamplay (jump table 0x1006b94c).
const BOT_MODELS: [&str; 7] = ["seal", "merc", "gordon", "hitman", "castor", "agent", "laurence"];

/// `bots.txt` and `bot_say.txt` as loaded (`g_botNames`... at 0x100db1e0).
#[derive(Default)]
pub struct Files {
    pub names: files::BotNames,
    pub say: files::BotSay,
}

static FILES: Global<RefCell<Option<Files>>> = Global(RefCell::new(None));

/// The native backend's state of a bot player: its brain. `None` for a player that isn't a native
/// bot (a human, or another backend's bot).
pub fn brain_of(p: &mut Player) -> Option<&mut Brain> {
    p.bot.as_mut()?.downcast_mut::<Brain>()
}

/// The loaded text files (no engine calls inside `f`).
pub fn with_files<R>(f: impl FnOnce(&mut Files) -> R) -> R {
    let mut m = FILES.0.borrow_mut();
    f(m.get_or_insert_with(Files::default))
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

// ---- creation ------------------------------------------------------------------------------------

/// `addbot [model]`: `BotCreate` (0x1006b7c0). The argument is the model/team
/// (despite the usage text); the name always comes from `bots.txt`. Afterwards `botlearn 1`
/// when the node graph is empty.
pub fn add_bot(model: &str) -> Option<*mut edict_t> {
    let eng = engine();
    let model = pick_model(model);
    let mut skill = eng.random_float(0.1, 1.0);
    let name = with_files(|f| f.names.pick(|lo, hi| eng.random_long(lo, hi), &mut skill));
    let e = create(&name, &model, skill, true);
    if e.is_some() && nodes::with(|g| g.count()) < 1 {
        eng.server_command("botlearn 1\n");
    }
    e
}

/// `addcustombot [name] [model] [skill]`: `BotCreateCustom` (0x1006b590). The
/// skill is random in 0.1..1 when not positive, else kept in 0.1..3; an empty name is picked from
/// `bots.txt` (whose per-name skill then wins).
pub fn add_custom_bot(name: &[u8], model: &str, skill: f32) -> Option<*mut edict_t> {
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
    create(&name, &model, skill, true)
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

/// `BotCreate`'s tail: the bot player (`BotSetup`, the host's
/// [`crate::bot_backend::create_player`]) with a fresh brain of `skill`, announced as TS does
/// ("<name> connected with skill <f>", the unclamped skill) when `announce` is set.
fn create(name: &[u8], model: &str, skill: f32, announce: bool) -> Option<*mut edict_t> {
    crate::bot_backend::create_player(name, model, Box::new(Brain::new(skill)), announce.then_some(skill))
}

/// The native backend's quota bot ("Bot Neo", ...; the host picks the name): created like
/// `addbot`'s (a random model outside teamplay, a random skill in 0.1..1), but without TS's
/// announcement, so the dedicated-server tests can tell it from an `addbot`. Returns the bot and
/// its skill.
pub fn add_quota_bot(name: &str) -> Option<(*mut edict_t, f32)> {
    let model = pick_model("");
    let skill = engine().random_float(0.1, 1.0);
    create(name.as_bytes(), &model, skill, false).map(|e| (e, skill))
}

// ---- per frame -----------------------------------------------------------------------------------

/// The bot part of `pfnStartFrame` (0x10010e00), which runs before the rules' `Think`: on a
/// listen server every player whose `m_bIsBot == 1` thinks, in entity order (`BotPlayerByIndex`
/// 0x1006b360: `FL_FAKECLIENT` or `FL_CLIENT`, with private data). Port: bots think on dedicated
/// servers too, after the host's quota (`bot_backend::start_frame`).
pub fn start_frame() {
    let eng = engine();
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

// ---- CBot overrides and hooks -------------------------------------------------------------------

/// The head of `CBot::Spawn` (0x1006bb40): `CBotBrain::Reset`, before `CBasePlayer::Spawn`.
pub fn spawn_pre(p: &mut Player, cx: &mut Cx) {
    let now = cx.time();
    let mode = rules::state::with(|g| g.mode);
    if let Some(b) = brain_of(p) {
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
    if let Some(b) = brain_of(p) {
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
        if let Some(b) = brain_of(p) {
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
        if let Some(b) = brain_of(killer) {
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
