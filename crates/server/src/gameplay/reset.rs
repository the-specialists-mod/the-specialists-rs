//! The reset a gameplay switch runs (issue #100 §1.3): the game goes back to a brand-new state, as
//! if the map had just started, without reloading the map. Nothing carries over, so no state is
//! ever converted between rulesets. This is core code: it knows no plugin, and the new plugin's
//! `on_activate` runs after it.
//!
//! The order replays a map start. [`begin`], on the switch frame:
//! 1. The clients: `SrvSett` 4 with the new id (a port client resets its HUD, prediction and
//!    effects on the change) and physinfo `gp`.
//! 2. The players lose their items and get brand-new `Player` objects; only what identifies
//!    the connection stays (a bot's skill, the spray settings). They are frozen and can't be hurt
//!    until they spawn.
//! 3. Every other entity is removed at once (`pfnRemoveEntity`), except the world's soundent,
//!    which [`finish`] spawns again in place.
//!
//! [`finish`], at least 0.5 s later, when the engines hand the freed edicts out again (FWGS
//! `SV_AllocEdict`, ReHLDS `ED_Alloc`), so even a map with many entities fits:
//! 4. The world's part of `CWorld::Precache` without the precaches: the per-map registries, the
//!    game rules (`rules::start_rules`, without the configs a map start runs), the soundent,
//!    the body queue and the light styles.
//! 5. The map's entities, spawned again from the BSP entity lump as the engine does, then
//!    `ServerActivate`'s `Activate` pass.
//! 6. Every player spawns as a fresh join (`ClientPutInServer`'s spawn): `InitHUD` again, the
//!    score, team and buy menu from scratch.

use crate::player::Player;
use crate::rules::msg;
use crate::{entities, messages::Msg, rules};
use goldsrc::edict::EdictPtr;
use goldsrc::server::engine;
use goldsrc::sys::*;
use shared::protocol::rules::SrvSett;
use std::ffi::CString;

/// A reset between its two halves.
#[derive(Clone, Copy)]
pub struct Pending {
    /// The game time from which [`finish`] may run.
    pub at: f32,
    old: &'static str,
    new: &'static str,
    removed: usize,
}

/// How long the map stays empty: the engines' 0.5 s before a freed edict is used again, and a
/// margin for the frame that frees them.
const EDICT_REUSE_WAIT: f32 = 0.6;

/// The first half of the reset for the switch from `old` to `new` (plugin ids) at game time
/// `now`: steps 1-3.
pub fn begin(old: &'static str, new: &'static str, now: f32) -> Pending {
    let value = super::srvsett_value();
    msg::all(Msg::SrvSett, |w| SrvSett { key: ts_gameplay::SRVSETT_KEY, value }.write(w));
    for e in rules::players() {
        super::set_physinfo(e);
        crate::player::with_player(e, fresh_player);
    }
    let removed = remove_entities();
    Pending { at: now + EDICT_REUSE_WAIT, old, new, removed }
}

/// The second half: steps 4-6. Whatever the players' think did in the empty map is dropped
/// first: every player is made brand new again, and the entities made meanwhile are removed.
pub fn finish(p: Pending) {
    let eng = engine();
    for e in rules::players() {
        crate::player::with_player(e, fresh_player);
    }
    let removed = p.removed + remove_entities();
    entities::stock::restart_world();
    // The new rules count `mp_timelimit` from now, as a map start's restarted clock would.
    rules::state::with(|g| g.time_base = eng.time());
    let spawned = spawn_map_entities();
    entities::server_activate(eng.max_entities(), eng.max_clients());
    crate::bot_backend::server_activate();
    let players = rules::players();
    for &e in &players {
        crate::ham::spawn(e);
        rejoin_bot_team(e);
    }
    eng.print(&format!(
        "[ts] gameplay: ruleset {} -> {}: game reset ({} players, {} entities removed, {spawned} spawned from the map)\n",
        p.old,
        p.new,
        players.len(),
        removed
    ));
}

/// The part of a bot's join that follows its spawn (`BotSetup`, `bot_backend::create_player`): in
/// teamplay, a bot created with a model joins that team.
fn rejoin_bot_team(e: *mut edict_t) {
    let Some((cbot, recorded)) = crate::player::with_player(e, |p, _| (p.cbot, p.bot_join_team.clone())) else {
        return;
    };
    let team_key = || engine().info_key(e, c"team").to_string_lossy().into_owned();
    if let Some(team) = bot_rejoin_team(rules::state::teamplay(), cbot, &recorded, team_key) {
        crate::player::with_player(e, |p, cx| rules::teams::change_team(p, cx, &team, false, false));
    }
}

/// The team a bot joins again after the reset's spawn: the model `BotSetup` joined with
/// (`Player::bot_join_team`), which a bot has whichever ruleset made it (a ruleset's bots may have
/// no `team` key: `NativeBotRules::team_key`). A bot the engine kept across a level change has
/// none recorded and falls back to its `team` key, as before. `None` outside teamplay, for
/// players, and for a bot with neither.
fn bot_rejoin_team(teamplay: bool, cbot: bool, recorded: &str, team_key: impl FnOnce() -> String) -> Option<String> {
    if !teamplay || !cbot {
        return None;
    }
    let team = if recorded.is_empty() { team_key() } else { recorded.to_owned() };
    (!team.is_empty()).then_some(team)
}

/// A player's part: its items destroyed (the gun entity with them), then a brand-new `Player`
/// that keeps only the connection's identity, and a zero score. The spawn comes later, once
/// the map is back ([`finish`]); until then the player is frozen and can't be hurt (the spawn
/// clears both).
fn fresh_player(p: &mut Player, cx: &mut crate::world::Cx) {
    crate::weapons::remove_all_items(p, cx);
    let old = std::mem::take(p);
    p.cbot = old.cbot;
    p.is_bot = old.is_bot;
    p.bot_join_team = old.bot_join_team;
    // The bot's backend state for the new game (native: a fresh brain with the bot's skill).
    p.bot = crate::bot_backend::reset_state(old.bot);
    p.impulse.custom_decal_frames = old.impulse.custom_decal_frames;
    let v = cx.pev();
    v.frags = 0.0;
    v.iuser1 = 0;
    v.iuser2 = 0;
    // Alive to the think until the spawn: a dead player's `PlayerDeathThink` would make a corpse
    // and start observing in the empty map.
    v.deadflag = DEAD_NO as i32;
    v.flags |= FL_FROZEN as i32;
    v.takedamage = DAMAGE_NO as f32;
    v.velocity = [0.0; 3];
}

/// Removes every entity past the clients but the world's soundent, at once, and returns how many.
fn remove_entities() -> usize {
    let eng = engine();
    let soundent = entities::stock::soundent::world_soundent();
    let mut n = 0;
    for i in eng.max_clients() + 1..eng.max_entities() {
        let e = eng.entity_of_index(i);
        if e.is_null() || e.is_free() || e == soundent {
            continue;
        }
        // `UTIL_Remove`'s `UpdateOnRemove` (globals, sounds), then freed now rather than at the end
        // of the frame, so the map's entities can reuse the edicts.
        if e.has_private_data() {
            entities::stock::base::update_on_remove(e);
        }
        eng.remove_entity(e);
        n += 1;
    }
    n
}

/// Spawns the map's entities from its BSP entity lump, skipping worldspawn (which stays), the
/// way FWGS's `SV_LoadFromFile`/`SV_ParseEdict` do: the class's private data, then every other
/// keyvalue in order (empty keys or values and `wad` skipped, trailing spaces trimmed from keys,
/// `angle` given as `angles`), then `Spawn`; an entity whose spawn refuses (-1) is freed unless
/// it removed itself. Returns how many spawned.
fn spawn_map_entities() -> usize {
    let eng = engine();
    let map = eng.string(eng.globals().mapname).to_string_lossy().into_owned();
    let Some(bsp) = eng.load_file(&format!("maps/{map}.bsp")) else {
        eng.print(&format!("[ts] gameplay: can't read maps/{map}.bsp; the map's entities aren't respawned\n"));
        return 0;
    };
    let blocks = shared::bsp::entity_lump(&bsp).map(shared::bsp::parse_entities).unwrap_or_default();
    blocks.iter().skip(1).filter(|kv| spawn_entity(kv)).count()
}

fn spawn_entity(kv: &[(String, String)]) -> bool {
    let eng = engine();
    let Some((_, class)) = kv.iter().find(|(k, v)| k == "classname" && !v.is_empty()) else { return false };
    let Ok(class) = CString::new(class.as_str()) else { return false };
    let e = eng.create_named_entity(&class);
    if e.is_null() || e.is_free() || e.v().flags & FL_KILLME as i32 != 0 {
        return false;
    }
    for (k, v) in kv {
        let k = k.trim_end_matches(' ');
        if k.is_empty() || v.is_empty() || k == "wad" || k == "classname" {
            continue;
        }
        let (k, v) = if k == "angle" { ("angles", angle_to_angles(e, v)) } else { (k, v.clone()) };
        let (Ok(k), Ok(v)) = (CString::new(k), CString::new(v)) else { continue };
        entities::dispatch_key_value(e, Some(&class), &k, &v);
    }
    if entities::dispatch_spawn(e) == -1 {
        if e.v().flags & FL_KILLME as i32 == 0 {
            eng.remove_entity(e);
        }
        return false;
    }
    true
}

/// `SV_ParseEdict`'s `angle` key: a yaw, or -1 (up) / -2 (down).
fn angle_to_angles(e: *mut edict_t, v: &str) -> String {
    let yaw: f32 = v.split_whitespace().next().and_then(|t| t.parse().ok()).unwrap_or(0.0);
    let a = e.v().angles;
    if yaw >= 0.0 {
        format!("{} {} {}", a[0], yaw, a[2])
    } else if yaw == -1.0 {
        "-90 0 0".to_owned()
    } else if yaw == -2.0 {
        "90 0 0".to_owned()
    } else {
        "0 0 0".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::bot_rejoin_team;

    #[test]
    fn a_bot_rejoins_the_team_it_was_made_with() {
        let key = || "merc".to_owned();
        assert_eq!(bot_rejoin_team(true, true, "seal", key), Some("seal".to_owned()), "the recorded model wins");
        // A ruleset whose BotSetup sets no `team` key: the recorded model still puts it back.
        assert_eq!(bot_rejoin_team(true, true, "seal", String::new), Some("seal".to_owned()));
        // Kept across a level change (nothing recorded): its `team` key, as before.
        assert_eq!(bot_rejoin_team(true, true, "", key), Some("merc".to_owned()));
        assert_eq!(bot_rejoin_team(true, true, "", String::new), None);
    }

    #[test]
    fn only_bots_in_teamplay_rejoin() {
        assert_eq!(bot_rejoin_team(false, true, "seal", || "seal".to_owned()), None);
        assert_eq!(bot_rejoin_team(true, false, "seal", || "seal".to_owned()), None);
    }
}
