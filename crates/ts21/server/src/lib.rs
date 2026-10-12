//! TS 2.1's server rules (issue #100 §3) and the plugin object `server-dll` registers. Nothing is
//! overridden yet: every hook keeps TS 3.0's body.

#![deny(unsafe_code)]

use ts_server::gameplay::{ServerPlugin, ServerRules};
use ts21_gameplay::TS21;

/// TS 2.1's server-only rules.
pub struct Ts21Server;

impl ServerRules for Ts21Server {}

/// The plugin on the server: TS 2.1's shared hooks and its server hooks.
pub static PLUGIN: ServerPlugin = ServerPlugin { pm: &TS21, gun: &TS21, server: &Ts21Server };
