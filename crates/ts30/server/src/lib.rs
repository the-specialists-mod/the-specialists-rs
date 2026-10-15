//! TS 3.0's server rules (issue #200) and the plugin object `server-dll` registers.
//!
//! * [`PLUGIN`]: the gameplay plugin, [`Ts30Server`] with the shared [`TS30`] hooks.
//! * [`bots::BOTS`]: the native bot backend's rules for 3.0, which `server-dll` registers with
//!   `ts_server::bots::rules` under the plugin's id (the bots are a layer of their own, so they
//!   aren't part of [`PLUGIN`]).

#![deny(unsafe_code)]

pub mod bots;

use ts_server::gameplay::{ServerPlugin, ServerRules};
use ts30_gameplay::TS30;

/// TS 3.0's server-only rules.
pub struct Ts30Server;

impl ServerRules for Ts30Server {}

/// The plugin as the server registers it.
pub static PLUGIN: ServerPlugin = ServerPlugin { pm: &TS30, gun: &TS30, server: &Ts30Server };
