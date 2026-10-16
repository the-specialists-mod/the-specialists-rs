//! TS 3.0's native bots (the Windows `mp.dll` AI, decomp `docs/server/bots.md`): the rules the
//! bot layer (`ts_server::bots`) runs while the 3.0 ruleset is active
//! (docs/implementation/bot-plugins.md §2.2).
//!
//! * this module: [`Ts30Bots`], the hooks, and the `CBot` overrides (the laser colour at spawn,
//!   the turn toward an attacker).
//! * [`states`]: the strategy states' think (§6).
//! * [`modules`]: the vision, buy and look-around modules (§7, §9).
//! * [`attack`]: the attack module's think (§8).
//!
//! The brain, navigation, node graph, texts and the modules both rulesets share (stunt, power-up,
//! chat) are the bot layer's, in `ts_server::bots`.

pub mod attack;
pub mod modules;
pub mod states;

use goldsrc::edict::EdictPtr;
use goldsrc::server::engine;
use goldsrc::sys::*;
use ts_server::bots::brain::{self, Brain, btn};
use ts_server::bots::rules::{BotTexts, NativeBotRules};
use ts_server::bots::{self, nodes};
use ts_server::player::Player;
use ts_server::world::{Cx, DamageInfo};

/// TS 3.0's bot rules.
pub struct Ts30Bots;

impl NativeBotRules for Ts30Bots {
    // ---- creation (`BotCreate`, `BotCreateCustom`, `BotSetup`) ----

    /// `BotCreate` without a model argument picks one of its seven models only outside teamplay
    /// (jump table 0x1006b94c).
    fn random_model_in_teamplay(&self) -> bool {
        false
    }

    /// `BotCreate` issues `botlearn 1` when the node graph is empty.
    fn learn_on_empty_graph(&self) -> bool {
        true
    }

    /// `BotSetup` stores the skill at least 0.1.
    fn stored_skill(&self, skill: f32) -> f32 {
        if skill > 0.1 { skill } else { 0.1 }
    }

    /// `BotSetup` sets the `team` infokey (to the model).
    fn team_key(&self) -> bool {
        true
    }

    /// The game directory's `bots.txt` and `bot_say.txt` (`BotLoadFiles` 0x1006bee0).
    fn texts(&self) -> Option<BotTexts> {
        None
    }

    // ---- the `CBot` overrides ----

    /// The tail of `CBot::Spawn` (0x1006bb40): a random laser-sight colour ([`laser_colour`]).
    fn spawn_post(&self, p: &mut Player, cx: &mut Cx) {
        laser_colour(p, cx);
    }

    /// `CBot::TraceAttack` (0x1006bac0): turn toward the attacker ([`turn_to_attacker`]).
    fn trace_attack(&self, p: &mut Player, cx: &mut Cx, dmg: &DamageInfo) {
        turn_to_attacker(p, cx, dmg);
    }

    // ---- the brain (`CBotBrain::Think`, `RunMove`, the buttons) ----

    /// `time + min(k / slow / skill, max)` (bots.md §5.8), with `slow` = `GiveSlowMul()`.
    fn delay(&self, now: f32, k: f64, pev: &entvars_t, skill: f32, max: f64) -> f32 {
        brain::delay(now, k, brain::slow(pev), skill, max)
    }

    /// The AI runs while `GiveSlowMul() > 0.1` (0x1006f53b).
    fn runs_ai(&self, pev: &entvars_t) -> bool {
        brain::slow(pev) > 0.1
    }

    /// The view turning rate `Think` sets every frame (brain+0x1a70): `clamp(skill, 0.5, 2) * 10`.
    fn turn_rate(&self, skill: f32) -> f32 {
        (brain::clamp_skill(skill, 2.0) as f64 * 10.0) as f32
    }

    /// `UpdateView` (0x1006e500) at the brain's turn rate.
    fn update_view(&self, b: &mut Brain, pev: &mut entvars_t) {
        let rate = b.turn_rate;
        brain::update_view(b, pev, rate);
    }

    /// `LookAt` (0x1006f740): the wanted view toward `target` from the gun position.
    fn look_at(&self, b: &mut Brain, pev: &mut entvars_t, target: [f32; 3]) {
        b.set_ideal_toward(pev, target);
    }

    /// `PressButtons(mask, duration)` (0x1006f470): a timer per bit.
    fn press(&self, b: &mut Brain, mask: u32, duration: f32, now: f32) {
        b.press_bits(mask, duration, now);
    }

    /// A press of length 0.
    fn release(&self, b: &mut Brain, mask: u32, now: f32) {
        b.press_bits(mask, 0.0, now);
    }

    /// `UpdateButtons` (0x1006f4b0).
    fn update_buttons(&self, b: &mut Brain, now: f32) {
        b.update_bits(now);
    }

    // ---- navigation (`CBotNav`) ----

    /// `Navigate`'s ladder buttons (0x1006fc00): 3.0 presses the combined masks `DUCK|BACK` (or
    /// `DUCK|FORWARD`) for 1 s and `JUMP|FORWARD` (or `JUMP|BACK`) for 0 s, which with per-bit
    /// timers lets go of the jump and the other direction.
    fn ladder_buttons(&self, b: &mut Brain, down: bool, now: f32) {
        let (hold, other) = if down { (btn::BACK, btn::FORWARD) } else { (btn::FORWARD, btn::BACK) };
        b.press(btn::DUCK | hold, 1.0, now);
        b.press(btn::JUMP | other, 0.0, now);
    }

    /// `RandomWanderPoint`'s trace end (0x1006f8c0): `origin + dir * dist`, the products
    /// rounded to float first (3.0's z product stays on the x87 stack, but `dir.z` is 0).
    fn wander_end(&self, dir: [f32; 3], dist: f32, origin: [f32; 3]) -> [f32; 3] {
        brain::add(brain::scale(dir, dist), origin)
    }

    /// `GetSteerPoint`'s forward probe end (0x10070400): `origin + v_forward * 24`, the whole
    /// product vector stored as floats first.
    fn steer_probe_end(&self, forward: [f32; 3], origin: [f32; 3]) -> [f32; 3] {
        brain::add(brain::scale(forward, 24.0), origin)
    }

    /// The respawn module (bit 0x100, inline at 0x1006f5b8): the `respawn` command's
    /// `StopObserver` once the respawn time has passed ([`brain::respawn_now`]).
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
        if engine().random_long(0, 100) <= 25 { 2 } else { 1 }
    }

    // ---- the node graph (`CNodeGraph`, `botlearn` / `addnode`) ----

    /// `IsReachable`'s ladder shortcuts (0x100711e0): [`nodes::ladder_link_distance`].
    fn ladder_link_distance(&self, a: [f32; 3], b: [f32; 3]) -> f64 {
        nodes::ladder_link_distance(a, b)
    }

    /// `IsGroundContinuous` (0x10070fe0): [`nodes::ground_probes`].
    fn ground_probes(&self, a: [f32; 3], b: [f32; 3], no_floor: &mut dyn FnMut([f32; 3]) -> bool) -> bool {
        nodes::ground_probes(a, b, no_floor)
    }

    // Audit round 2: E

    /// `Navigate` (0x1006fd29.., 0x1006fdbd.., 0x1006ff50..) sums `(x² + z²) + y²`.
    fn nav_distance_order(&self) -> [usize; 3] {
        brain::order::XZY
    }

    /// `GetSteerPoint` (0x10070617..0x10070676): the products `v_forward * 48` are stored as
    /// floats (the `Vector` constructor's arguments), then subtracted.
    fn forward_detour(&self, endpos: [f32; 3], forward: [f32; 3]) -> [f32; 3] {
        brain::sub(endpos, brain::scale(forward, 48.0))
    }
}

/// The rules `server-dll` registers under [`ts30_gameplay::ID`].
pub static BOTS: Ts30Bots = Ts30Bots;

// ---- the `CBot` overrides ------------------------------------------------------------------------

/// The tail of `CBot::Spawn` (0x1006bb40): a random laser-sight colour (`PlInfo` key 0: 0 for 76/101, 1
/// for 15/101, 2 for 10/101), as the `lasclr_<n>` command sends it.
pub fn laser_colour(p: &mut Player, cx: &mut Cx) {
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
    ts_server::messages::all(shared::protocol::Msg::PlInfo).byte(cx.index()).byte(0).byte(laser).end();
}

/// `CBot::TraceAttack` (0x1006bac0): a player's hit turns the wanted view toward the attacker's
/// origin (no enemy or goal is set). TS's `GetClassPtr` would allocate a `CBaseEntity` for an
/// attacker without private data; the port only looks.
pub fn turn_to_attacker(p: &mut Player, cx: &mut Cx, dmg: &DamageInfo) {
    let a = dmg.attacker;
    if !p.cbot || a.is_null() || !ts_server::rules::has_player(a) {
        return;
    }
    let target = a.v().origin;
    if let Some(b) = bots::brain_of(p) {
        b.look_at(cx.pev(), target);
    }
}

#[cfg(test)]
mod tests;
