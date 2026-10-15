//! TS 3.0's client rules (issue #200): the HUD and input hooks, and the plugin object
//! `client-dll` registers.

#![deny(unsafe_code)]

use ts_client::gameplay::{ClientPlugin, ClientRules};
use ts30_gameplay::TS30;

/// TS 3.0's client-only rules.
pub struct Ts30Client;

impl ClientRules for Ts30Client {}

/// The plugin as the client registers it.
pub static PLUGIN: ClientPlugin = ClientPlugin { pm: &TS30, gun: &TS30, client: &Ts30Client };
