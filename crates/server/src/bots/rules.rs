//! The native bots' rules: the hook points where a gameplay ruleset's version of TS's bot AI
//! differs (docs/implementation/bot-plugins.md §2.2). Every hook's default body is TS 3.0's
//! behaviour, so [`CORE`] is the AI ported from `mp.dll` unchanged.
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

use super::brain::{self, Brain};
use super::{attack, modules, states};
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

/// The native bots' hooks. All defaults are TS 3.0 (`mp.dll`, decomp `docs/server/bots.md`).
pub trait NativeBotRules: Sync {
    // ---- creation (`BotCreate`, `BotCreateCustom`, `BotSetup`) ----

    /// `BotCreate` without a model argument: TS 3.0 picks one of its seven models only outside
    /// teamplay (jump table 0x1006b94c). True: also in teamplay.
    fn random_model_in_teamplay(&self) -> bool {
        false
    }

    /// `BotCreate` issues `botlearn 1` when the node graph is empty.
    fn learn_on_empty_graph(&self) -> bool {
        true
    }

    /// The skill `BotSetup` stores in the brain: at least 0.1.
    fn stored_skill(&self, skill: f32) -> f32 {
        if skill > 0.1 { skill } else { 0.1 }
    }

    /// `BotSetup` sets the `team` infokey (to the model).
    fn team_key(&self) -> bool {
        true
    }

    /// The bot texts. `None`: `<gamedir>/bots.txt` and `<gamedir>/bot_say.txt` (`BotLoadFiles`
    /// 0x1006bee0).
    fn texts(&self) -> Option<BotTexts> {
        None
    }

    // ---- the `CBot` overrides ----

    /// The tail of `CBot::Spawn` (0x1006bb40): a random laser-sight colour.
    fn spawn_post(&self, p: &mut Player, cx: &mut Cx) {
        super::laser_colour(p, cx);
    }

    /// `CBot::TraceAttack` (0x1006bac0): turn toward the attacker.
    fn trace_attack(&self, p: &mut Player, cx: &mut Cx, dmg: &DamageInfo) {
        super::turn_to_attacker(p, cx, dmg);
    }

    // ---- the brain (`CBotBrain::Think`, `RunMove`, the buttons) ----

    /// The modules' reschedule (bots.md §5.8): `time + min(k / slow / skill, max)`. Also the
    /// strategy state (skill 1) and the path follower's stuck window (`k` 3, skill 1, `max` 9).
    fn delay(&self, now: f32, k: f64, pev: &entvars_t, skill: f32, max: f64) -> f32 {
        brain::delay(now, k, brain::slow(pev), skill, max)
    }

    /// `Think`'s time-scale test: the AI runs while `GiveSlowMul() > 0.1` (0x1006f53b).
    fn runs_ai(&self, pev: &entvars_t) -> bool {
        brain::slow(pev) > 0.1
    }

    /// The view turning rate `Think` sets every frame (brain+0x1a70): `clamp(skill, 0.5, 2) * 10`.
    fn turn_rate(&self, skill: f32) -> f32 {
        (brain::clamp_skill(skill, 2.0) as f64 * 10.0) as f32
    }

    /// `RunMove`'s view part: `UpdateView` (0x1006e500).
    fn update_view(&self, b: &mut Brain, pev: &mut entvars_t) {
        let rate = b.turn_rate;
        brain::update_view(b, pev, rate);
    }

    /// `LookAt` (0x1006f740): the wanted view toward `target`.
    fn look_at(&self, b: &mut Brain, pev: &mut entvars_t, target: [f32; 3]) {
        b.set_ideal_toward(pev, target);
    }

    /// `PressButtons(mask, duration)` (0x1006f470).
    fn press(&self, b: &mut Brain, mask: u32, duration: f32, now: f32) {
        b.press_bits(mask, duration, now);
    }

    /// Lets go of `mask` now (3.0: a press of length 0).
    fn release(&self, b: &mut Brain, mask: u32, now: f32) {
        b.press_bits(mask, 0.0, now);
    }

    /// `UpdateButtons` (0x1006f4b0), at the start of `Think`.
    fn update_buttons(&self, b: &mut Brain, now: f32) {
        b.update_bits(now);
    }

    /// The respawn module (bit 0x100, inline at 0x1006f5b8): the `respawn` command's
    /// `StopObserver` once the respawn time has passed.
    fn respawn(&self, b: &mut Brain, p: &mut Player, cx: &mut Cx) {
        brain::respawn_now(b, p, cx);
    }

    // ---- the modules ----

    /// The current strategy state's `Think` (state vtable slot 0, bots.md §6).
    fn state_think(&self, b: &mut Brain, p: &mut Player, cx: &mut Cx) {
        states::think(b, p, cx);
    }

    /// `CBotModuleVision::Scan` (0x1006dc10).
    fn vision(&self, b: &mut Brain, p: &mut Player, cx: &mut Cx) {
        modules::vision(b, p, cx);
    }

    /// `CBotModuleAttack::Think` (0x1006c520).
    fn attack(&self, b: &mut Brain, p: &mut Player, cx: &mut Cx) {
        attack::think(b, p, cx);
    }

    /// `CBotModuleBuy::Think` (0x1006d630).
    fn buy(&self, b: &mut Brain, p: &mut Player, cx: &mut Cx) {
        modules::buy(b, p, cx);
    }

    /// `CBotModuleLookAround::Think` (0x1006dde0).
    fn look_around(&self, b: &mut Brain, p: &mut Player, cx: &mut Cx) {
        modules::look_around(b, p, cx);
    }

    /// The stunt module's choice (0x1006e130): 2 (a flip) for 26/101, else 1 (a dive).
    fn stunt_mode(&self) -> u8 {
        if goldsrc::server::engine().random_long(0, 100) <= 25 { 2 } else { 1 }
    }
}

/// TS 3.0's bots: every hook's default.
pub struct Core;

impl NativeBotRules for Core {}

pub static CORE: Core = Core;

/// A gameplay id and the rules registered for it.
type Entry = (&'static str, &'static dyn NativeBotRules);

/// The registered rules, by gameplay id.
static REGISTRY: Global<RefCell<Vec<Entry>>> = Global(RefCell::new(Vec::new()));

/// The last resolution: the gameplay id it was for and its rules.
static CACHE: Global<Cell<Option<Entry>>> = Global(Cell::new(None));

/// Registers the native bots' rules for gameplay id `id` (the server library's `GameDLLInit`,
/// next to the gameplay plugin). An id without rules runs [`CORE`]. A second registration of an
/// id replaces the first.
pub fn register(id: &'static str, rules: &'static dyn NativeBotRules) {
    let mut r = REGISTRY.0.borrow_mut();
    r.retain(|(i, _)| *i != id);
    r.push((id, rules));
    CACHE.0.set(None);
}

/// The rules for gameplay id `id` ([`CORE`] when none are registered).
pub fn for_id(id: &str) -> &'static dyn NativeBotRules {
    REGISTRY.0.borrow().iter().find(|(i, _)| *i == id).map_or(&CORE as &dyn NativeBotRules, |r| r.1)
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
    fn core_defaults_are_ts_3_0() {
        let c: &dyn NativeBotRules = &CORE;
        assert!(!c.random_model_in_teamplay());
        assert!(c.learn_on_empty_graph());
        assert!(c.team_key());
        assert_eq!(c.texts(), None);
        // `BotSetup`'s floor.
        assert_eq!(c.stored_skill(0.01), 0.1);
        assert_eq!(c.stored_skill(-1.0), 0.1);
        assert_eq!(c.stored_skill(0.1), 0.1);
        assert_eq!(c.stored_skill(1.7), 1.7);
        // `clamp(skill, 0.5, 2) * 10`.
        assert_eq!(c.turn_rate(0.1), 5.0);
        assert_eq!(c.turn_rate(1.3), 13.0);
        assert_eq!(c.turn_rate(3.0), 20.0);
        // `min(k / slow / skill, max)` with slow = max(fuser1, 0.01).
        let mut v = entvars_t { fuser1: 1.0, ..Default::default() };
        assert_eq!(c.delay(10.0, 0.1, &v, 1.0, 1.0), brain::delay(10.0, 0.1, 1.0, 1.0, 1.0));
        v.fuser1 = 0.05;
        assert_eq!(c.delay(10.0, 0.1, &v, 1.0, 1.0), 11.0);
        assert!(!c.runs_ai(&v));
        v.fuser1 = 0.1;
        assert!(!c.runs_ai(&v));
        v.fuser1 = 0.0;
        assert!(!c.runs_ai(&v));
        v.fuser1 = 0.5;
        assert!(c.runs_ai(&v));
    }

    #[test]
    fn core_buttons_are_per_bit_timers() {
        let c: &dyn NativeBotRules = &CORE;
        let mut b = Brain::new(1.0);
        c.press(&mut b, brain::btn::ATTACK | brain::btn::DUCK, 0.5, 10.0);
        c.update_buttons(&mut b, 10.0);
        assert_eq!(b.buttons, brain::btn::ATTACK | brain::btn::DUCK);
        c.release(&mut b, brain::btn::DUCK, 10.2);
        c.update_buttons(&mut b, 10.2);
        assert_eq!(b.buttons, brain::btn::ATTACK);
        // Held while `time < expire`: not at the expiry itself.
        c.update_buttons(&mut b, 10.5);
        assert_eq!(b.buttons, 0);
    }

    #[test]
    fn unregistered_ids_run_core() {
        // (Registering would touch the process-wide registry other tests read.)
        assert!(for_id("nope").team_key());
        assert_eq!(for_id("nope").stored_skill(0.01), 0.1);
    }
}
