//! TS 2.1's client rules: HUD and input (issue #100 §4), and the plugin object `client-dll`
//! registers. Nothing is overridden yet: every hook keeps TS 3.0's body.

#![deny(unsafe_code)]

use ts_client::gameplay::{ClientPlugin, ClientRules};
use ts21_gameplay::TS21;

/// TS 2.1's client-only rules.
pub struct Ts21Client;

impl ClientRules for Ts21Client {}

/// The plugin on the client: TS 2.1's shared hooks and its client hooks.
pub static PLUGIN: ClientPlugin = ClientPlugin { pm: &TS21, gun: &TS21, client: &Ts21Client };
