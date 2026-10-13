//! The native backend: TS's own bots behind [`BotBackend`]. Every hook calls the function that
//! ran at that point before the backend split (issue #104), so the bots behave exactly as
//! before. The TS 2.1 bot rules (#100 §3.10) can hook these functions later without touching the
//! host.

use super::{Brain, add_bot, add_custom_bot, add_quota_bot, nodes};
use crate::bot_backend::{AddBot, BotBackend, BotState, NATIVE};
use crate::commands::Args;
use crate::player::Player;
use crate::world::{Cx, DamageInfo};
use goldsrc::sys::*;

/// TS's bots (`crates/server/src/bots`).
pub struct Native;

impl BotBackend for Native {
    fn name(&self) -> &str {
        NATIVE
    }

    /// `addbot`/`addcustombot` are registered with the rules' server commands, and `botlearn`
    /// with TS's cvars (`rules::cvars`): TS has them without bots too (`botlearn` learns from
    /// every player).
    fn game_init(&self) {}

    /// The node graph and the bot texts.
    fn map_init(&self) {
        super::ts_init();
    }

    /// `g_sModelIndexLightning`, the node display's beam (`sprites/lgtning.spr`).
    fn precache(&self) {
        let eng = goldsrc::server::engine();
        nodes::set_lightning_sprite(eng.precache_model(c"sprites/lgtning.spr"));
    }

    fn think(&self) {
        super::start_frame();
    }

    fn add_bot(&self, req: &AddBot) -> Option<(*mut edict_t, Option<f32>)> {
        match req {
            AddBot::Addbot { model } => add_bot(model).map(|e| (e, None)),
            AddBot::Custom { name, model, skill } => add_custom_bot(name, model, *skill).map(|e| (e, None)),
            AddBot::Quota { name } => add_quota_bot(name).map(|(e, skill)| (e, Some(skill))),
        }
    }

    fn new_state(&self, skill: f32) -> BotState {
        Box::new(Brain::new(skill))
    }

    /// A fresh brain with the bot's skill: `BotCreate` made it, and no state of the old game
    /// stays.
    fn reset_state(&self, old: BotState) -> Option<BotState> {
        old.downcast::<Brain>().ok().map(|b| Box::new(Brain::new(b.skill)) as BotState)
    }

    fn spawn_pre(&self, p: &mut Player, cx: &mut Cx) {
        super::spawn_pre(p, cx);
    }

    fn spawn_post(&self, p: &mut Player, cx: &mut Cx) {
        super::spawn_post(p, cx);
    }

    fn trace_attack(&self, p: &mut Player, cx: &mut Cx, dmg: &DamageInfo) {
        super::trace_attack(p, cx, dmg);
    }

    fn killed(&self, p: &mut Player, cx: &mut Cx, attacker: *mut edict_t) {
        super::killed(p, cx, attacker);
    }

    fn on_kill(&self, killer: &mut Player, victim: *mut edict_t, now: f32) {
        super::on_kill(killer, victim, now);
    }

    fn pre_think(&self, p: &mut Player, cx: &mut Cx) {
        super::learn(p, cx);
    }

    fn client_command(&self, p: &mut Player, cx: &mut Cx, cmd: &str, args: &Args) -> bool {
        super::client_command(p, cx, cmd, args)
    }
}
