//! The port's `ts_bot_quota N` (port addition, not in TS) and the bot manager's timing: keep N
//! quota bots, adding or kicking one every half second, a second after `ServerActivate`. The
//! host owns it; the active backend makes each bot ([`BotBackend::add_bot`] with
//! [`AddBot::Quota`]).

use super::{AddBot, BotBackend, live_fake_clients};
use crate::player::Player;
use crate::rules::cvars;
use crate::world;
use goldsrc::Global;
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::cell::RefCell;

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

/// The `n`th quota bot's name (from 1): the list, then numbered.
pub fn quota_name(n: u32) -> String {
    let base = QUOTA_NAMES[(n as usize - 1) % QUOTA_NAMES.len()];
    if n as usize > QUOTA_NAMES.len() { format!("{base} {n}") } else { base.to_owned() }
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

fn mgr<R>(f: impl FnOnce(&mut Manager) -> R) -> R {
    let mut m = MGR.0.borrow_mut();
    f(m.get_or_insert_with(Manager::default))
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

/// One quota bot from the backend. False when the server has no free client slot (reported once
/// per streak of failed adds: the quota retries every half second).
fn add_quota_bot(backend: &dyn BotBackend) -> bool {
    let eng = engine();
    let n = mgr(|m| m.added) + 1;
    let name = quota_name(n);
    let Some((e, skill)) = backend.add_bot(&AddBot::Quota { name: name.clone() }) else {
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
    let skill = skill.map(|s| format!(" skill {s:.2}")).unwrap_or_default();
    eng.print(&format!("[ts] bots: added \"{name}\" (#{}){skill}\n", eng.index_of(e)));
    true
}

/// The quota's bots that are still connected.
fn quota_bots() -> Vec<*mut edict_t> {
    let eng = engine();
    let ids = mgr(|m| m.quota.clone());
    live_fake_clients().into_iter().filter(|&e| ids.contains(&eng.user_id(e))).collect()
}

/// `ts_bot_quota N` (port): keep N quota bots, adding or kicking one every half second. A quota
/// bot that lost its backend state (a client the engine kept across a level change) gets a new
/// one ([`BotBackend::new_state`]: the native backend's brain of skill 1).
pub fn think(backend: &dyn BotBackend) {
    let eng = engine();
    let now = eng.time();
    let (active, start_at, next_add) = mgr(|m| (m.active, m.start_at, m.next_add));
    if !active || now < start_at {
        return;
    }
    let want = cvars::f(c"ts_bot_quota").max(0.0) as usize;
    let bots = quota_bots();
    for &e in &bots {
        let adopted = world::with::<Player, bool>(e, |p| {
            if p.bot.is_some() {
                return false;
            }
            p.bot = Some(backend.new_state(1.0));
            p.cbot = true;
            p.is_bot = 1;
            true
        });
        if adopted == Some(true) {
            backend.adopted(e);
        }
    }
    if now < next_add {
        return;
    }
    if bots.len() < want {
        mgr(|m| m.next_add = now + 0.5);
        add_quota_bot(backend);
    } else if bots.len() > want {
        mgr(|m| m.next_add = now + 0.5);
        if let Some(&e) = bots.last() {
            eng.server_command(&format!("kick #{}\n", eng.user_id(e)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quota_names() {
        assert_eq!(quota_name(1), "Bot Neo");
        assert_eq!(quota_name(16), "Bot Kid");
        assert_eq!(quota_name(17), "Bot Neo 17");
    }
}
