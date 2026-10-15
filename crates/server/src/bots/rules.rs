//! The native bots' rules: the hook points where a gameplay ruleset's version of TS's bot AI
//! differs (docs/implementation/bot-plugins.md §2.2). The hooks have no default bodies: each
//! ruleset's bots implement every one (the 3.0 plugin's run the AI ported from `mp.dll`, issue
//! #200).
//!
//! A ruleset that ships its own bot behaviour implements [`NativeBotRules`] and the server
//! library registers it under the gameplay id it belongs to ([`register`]). The native backend
//! resolves the rules from the active gameplay id ([`current`]), so the bot layer (`ts_bot_backend`)
//! and the gameplay layer (`ts_gameplay`) stay independent: the gameplay plugin's `ServerRules`
//! has no bot hooks, a plugin backend (`tsmod_*`) never sees these, and this module names no
//! ruleset.
//!
//! The hooks are coarse where a ruleset replaces a whole brain module (vision, attack, buy, look
//! around, the strategy state) and fine where it changes one value inside shared code (the
//! module timers, the view turning, the buttons). A replacement module gets the same `Brain`,
//! `Player` and `Cx` the native one does, and the native helpers are public for it.

use super::brain::Brain;
#[cfg(test)]
use super::brain::btn;
#[cfg(test)]
use super::nodes;
use crate::player::Player;
use crate::world::{Cx, DamageInfo};
use goldsrc::Global;
use goldsrc::sys::*;
use std::cell::{Cell, RefCell};

/// `bots.txt` and `bot_say.txt` as bytes, when a ruleset ships its own texts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BotTexts {
    pub names: &'static [u8],
    pub say: &'static [u8],
}

/// The native bots' hooks. Every method is required: a ruleset's bots implement all of them
/// (decomp `docs/server/bots.md` for the `mp.dll` AI they are cut from).
pub trait NativeBotRules: Sync {
    // ---- creation (`BotCreate`, `BotCreateCustom`, `BotSetup`) ----

    /// `BotCreate` without a model argument: whether one of the stock models is picked in
    /// teamplay too (outside teamplay it always is). True: also in teamplay.
    fn random_model_in_teamplay(&self) -> bool;

    /// Whether `BotCreate` issues `botlearn 1` when the node graph is empty.
    fn learn_on_empty_graph(&self) -> bool;

    /// The skill `BotSetup` stores in the brain, from the requested `skill`.
    fn stored_skill(&self, skill: f32) -> f32;

    /// Whether `BotSetup` sets the `team` infokey (to the model).
    fn team_key(&self) -> bool;

    /// The bot texts. `None`: `<gamedir>/bots.txt` and `<gamedir>/bot_say.txt` (`BotLoadFiles`
    /// 0x1006bee0).
    fn texts(&self) -> Option<BotTexts>;

    // ---- the `CBot` overrides ----

    /// The tail of `CBot::Spawn`, after `CBasePlayer::Spawn`.
    fn spawn_post(&self, p: &mut Player, cx: &mut Cx);

    /// `CBot::TraceAttack`, after `CBasePlayer::TraceAttack`: a bot's reaction to a hit.
    fn trace_attack(&self, p: &mut Player, cx: &mut Cx, dmg: &DamageInfo);

    // ---- the brain (`CBotBrain::Think`, `RunMove`, the buttons) ----

    /// The modules' reschedule (bots.md §5.8): the next think time from `now`, the module's
    /// constant `k`, the bot's entvars (its time scale), `skill` and the cap `max`. Also the
    /// strategy state (skill 1) and the path follower's stuck window (`k` 3, skill 1, `max` 9).
    fn delay(&self, now: f32, k: f64, pev: &entvars_t, skill: f32, max: f64) -> f32;

    /// `Think`'s time-scale test: whether the AI runs this frame.
    fn runs_ai(&self, pev: &entvars_t) -> bool;

    /// The view turning rate `Think` sets every frame, from the bot's skill.
    fn turn_rate(&self, skill: f32) -> f32;

    /// `RunMove`'s view part: turn the view toward the brain's wanted angles.
    fn update_view(&self, b: &mut Brain, pev: &mut entvars_t);

    /// `LookAt`: the wanted view toward `target`.
    fn look_at(&self, b: &mut Brain, pev: &mut entvars_t, target: [f32; 3]);

    /// `PressButtons(mask, duration)`: hold the buttons of `mask` for `duration` s from `now`.
    fn press(&self, b: &mut Brain, mask: u32, duration: f32, now: f32);

    /// Lets go of `mask` now.
    fn release(&self, b: &mut Brain, mask: u32, now: f32);

    /// `UpdateButtons`, at the start of `Think`: the buttons held at `now` into `b.buttons`.
    fn update_buttons(&self, b: &mut Brain, now: f32);

    // ---- navigation (`CBotNav`) ----

    /// `Navigate`'s ladder buttons: on a ladder, climb down (`down`: at or above the waypoint)
    /// or up.
    fn ladder_buttons(&self, b: &mut Brain, down: bool, now: f32);

    /// `RandomWanderPoint`'s trace end: `origin + dir * dist`, rounded as the ruleset's code does.
    fn wander_end(&self, dir: [f32; 3], dist: f32, origin: [f32; 3]) -> [f32; 3];

    /// `GetSteerPoint`'s forward probe end: `origin + v_forward * 24`, rounded as the ruleset's
    /// code does.
    fn steer_probe_end(&self, forward: [f32; 3], origin: [f32; 3]) -> [f32; 3];

    /// The respawn module (bit 0x100): what a dead or observing bot does to come back.
    fn respawn(&self, b: &mut Brain, p: &mut Player, cx: &mut Cx);

    // ---- the modules ----

    /// The current strategy state's `Think` (state vtable slot 0, bots.md §6): which modules run
    /// and where to roam.
    fn state_think(&self, b: &mut Brain, p: &mut Player, cx: &mut Cx);

    /// The vision module (bit 0x40): the visible enemies and teammates.
    fn vision(&self, b: &mut Brain, p: &mut Player, cx: &mut Cx);

    /// The attack module (bit 4): aim, target, move and fire.
    fn attack(&self, b: &mut Brain, p: &mut Player, cx: &mut Cx);

    /// The buy module (bit 0x10): the loadout for the next respawn.
    fn buy(&self, b: &mut Brain, p: &mut Player, cx: &mut Cx);

    /// The look-around module (bit 0x80): where to look while roaming.
    fn look_around(&self, b: &mut Brain, p: &mut Player, cx: &mut Cx);

    /// The stunt module's choice of a new stunt: 1 a dive, 2 a flip.
    fn stunt_mode(&self) -> u8;

    // ---- the node graph (`CNodeGraph`, `botlearn` / `addnode`) ----

    /// `IsReachable`'s ladder shortcuts: the 2D distance between nodes `a` and `b`, compared with
    /// 64 and 160.
    fn ladder_link_distance(&self, a: [f32; 3], b: [f32; 3]) -> f64;

    /// `IsGroundContinuous`: the floor probes from node `a` to node `b`; `no_floor` traces below
    /// each point, and the first one without a floor makes it false.
    fn ground_probes(&self, a: [f32; 3], b: [f32; 3], no_floor: &mut dyn FnMut([f32; 3]) -> bool) -> bool;
}

/// The rules the registry falls back to in this crate's tests, where nothing is registered: the
/// node graph's probes ([`nodes::ladder_link_distance`], [`nodes::ground_probes`]) and the
/// brain's per-bit button timers, which the neutral code's tests reach through [`current`].
/// Nothing else is reached; those hooks panic.
#[cfg(test)]
pub(crate) struct Stub;

#[cfg(test)]
impl NativeBotRules for Stub {
    fn random_model_in_teamplay(&self) -> bool {
        unreachable!("not used by the core tests")
    }
    fn learn_on_empty_graph(&self) -> bool {
        unreachable!("not used by the core tests")
    }
    fn stored_skill(&self, _skill: f32) -> f32 {
        unreachable!("not used by the core tests")
    }
    fn team_key(&self) -> bool {
        unreachable!("not used by the core tests")
    }
    fn texts(&self) -> Option<BotTexts> {
        unreachable!("not used by the core tests")
    }
    fn spawn_post(&self, _p: &mut Player, _cx: &mut Cx) {
        unreachable!("not used by the core tests")
    }
    fn trace_attack(&self, _p: &mut Player, _cx: &mut Cx, _dmg: &DamageInfo) {
        unreachable!("not used by the core tests")
    }
    fn delay(&self, _now: f32, _k: f64, _pev: &entvars_t, _skill: f32, _max: f64) -> f32 {
        unreachable!("not used by the core tests")
    }
    fn runs_ai(&self, _pev: &entvars_t) -> bool {
        unreachable!("not used by the core tests")
    }
    fn turn_rate(&self, _skill: f32) -> f32 {
        unreachable!("not used by the core tests")
    }
    fn update_view(&self, _b: &mut Brain, _pev: &mut entvars_t) {
        unreachable!("not used by the core tests")
    }
    fn look_at(&self, _b: &mut Brain, _pev: &mut entvars_t, _target: [f32; 3]) {
        unreachable!("not used by the core tests")
    }
    fn press(&self, b: &mut Brain, mask: u32, duration: f32, now: f32) {
        b.press_bits(mask, duration, now);
    }
    fn release(&self, b: &mut Brain, mask: u32, now: f32) {
        b.press_bits(mask, 0.0, now);
    }
    fn update_buttons(&self, b: &mut Brain, now: f32) {
        b.update_bits(now);
    }
    fn ladder_buttons(&self, _b: &mut Brain, _down: bool, _now: f32) {
        unreachable!("not used by the core tests")
    }
    fn wander_end(&self, _dir: [f32; 3], _dist: f32, _origin: [f32; 3]) -> [f32; 3] {
        unreachable!("not used by the core tests")
    }
    fn steer_probe_end(&self, _forward: [f32; 3], _origin: [f32; 3]) -> [f32; 3] {
        unreachable!("not used by the core tests")
    }
    fn respawn(&self, _b: &mut Brain, _p: &mut Player, _cx: &mut Cx) {
        unreachable!("not used by the core tests")
    }
    fn state_think(&self, _b: &mut Brain, _p: &mut Player, _cx: &mut Cx) {
        unreachable!("not used by the core tests")
    }
    fn vision(&self, _b: &mut Brain, _p: &mut Player, _cx: &mut Cx) {
        unreachable!("not used by the core tests")
    }
    fn attack(&self, _b: &mut Brain, _p: &mut Player, _cx: &mut Cx) {
        unreachable!("not used by the core tests")
    }
    fn buy(&self, _b: &mut Brain, _p: &mut Player, _cx: &mut Cx) {
        unreachable!("not used by the core tests")
    }
    fn look_around(&self, _b: &mut Brain, _p: &mut Player, _cx: &mut Cx) {
        unreachable!("not used by the core tests")
    }
    fn stunt_mode(&self) -> u8 {
        unreachable!("not used by the core tests")
    }
    fn ladder_link_distance(&self, a: [f32; 3], b: [f32; 3]) -> f64 {
        nodes::ladder_link_distance(a, b)
    }
    fn ground_probes(&self, a: [f32; 3], b: [f32; 3], no_floor: &mut dyn FnMut([f32; 3]) -> bool) -> bool {
        nodes::ground_probes(a, b, no_floor)
    }
}

#[cfg(test)]
pub(crate) static STUB: Stub = Stub;

/// A gameplay id and the rules registered for it.
type Entry = (&'static str, &'static dyn NativeBotRules);

/// The registered rules, by gameplay id.
static REGISTRY: Global<RefCell<Vec<Entry>>> = Global(RefCell::new(Vec::new()));

/// The last resolution: the gameplay id it was for and its rules.
static CACHE: Global<Cell<Option<Entry>>> = Global(Cell::new(None));

/// Registers the native bots' rules for gameplay id `id` (the server library's `GameDLLInit`,
/// next to the gameplay plugin). An id without rules runs the first registered. A second
/// registration of an id replaces the first, in its place.
pub fn register(id: &'static str, rules: &'static dyn NativeBotRules) {
    let mut r = REGISTRY.0.borrow_mut();
    match r.iter_mut().find(|(i, _)| *i == id) {
        Some(e) => e.1 = rules,
        None => r.push((id, rules)),
    }
    CACHE.0.set(None);
}

/// The rules for gameplay id `id` (the first registered when that id has none).
pub fn for_id(id: &str) -> &'static dyn NativeBotRules {
    let r = REGISTRY.0.borrow();
    let found = r.iter().find(|(i, _)| *i == id).or_else(|| r.first());
    match found {
        Some(f) => f.1,
        #[cfg(test)]
        None => &STUB,
        #[cfg(not(test))]
        None => panic!("no native bot rules registered (server-dll registers them at GameDLLInit)"),
    }
}

/// The rules of the active gameplay ruleset (`ts_gameplay`).
pub fn current() -> &'static dyn NativeBotRules {
    let id = crate::gameplay::active_id();
    if let Some((cached, r)) = CACHE.0.get()
        && std::ptr::eq(cached, id)
    {
        return r;
    }
    let r = for_id(id);
    CACHE.0.set(Some((id, r)));
    r
}

/// [`NativeBotRules::delay`] of the active rules.
pub fn delay(now: f32, k: f64, pev: &entvars_t, skill: f32, max: f64) -> f32 {
    current().delay(now, k, pev, skill, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unregistered_ids_run_the_first_registered() {
        // (Registering would touch the process-wide registry other tests read; this crate's
        // tests register nothing, so the test fallback answers.)
        assert!(std::ptr::addr_eq(for_id("nope"), &STUB));
        let mut b = Brain::new(1.0);
        for_id("nope").press(&mut b, btn::ATTACK, 0.5, 10.0);
        for_id("nope").update_buttons(&mut b, 10.0);
        assert_eq!(b.buttons, btn::ATTACK);
    }
}
