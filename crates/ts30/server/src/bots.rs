//! TS 3.0's native bots (`ts_server::bots`, the Windows `mp.dll` AI): the rules the bot layer
//! runs while the 3.0 ruleset is active (docs/implementation/bot-plugins.md §2.2).

use ts_server::bots::rules::NativeBotRules;

/// TS 3.0's bot rules.
pub struct Ts30Bots;

impl NativeBotRules for Ts30Bots {}

/// The rules `server-dll` registers under [`ts30_gameplay::ID`].
pub static BOTS: Ts30Bots = Ts30Bots;
