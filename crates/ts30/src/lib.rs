//! The TS 3.0 gameplay plugin (issue #200): TS 3.0's rules, the port's own game, as a ruleset the
//! server selects with `ts_gameplay 30`. The core crates call hooks where a ruleset decides
//! something; this plugin answers them as TS 3.0 does. `server-dll` and `client-dll` register it
//! under [`ID`] behind their `ts30` feature, first, so it is the default ruleset when linked.
//! See `docs/implementation/gameplay-plugins.md`.
//!
//! * This package: the movement and gun hooks both sides run (`PmRules`, `GunRules`,
//!   `MeleeRules`, on [`Ts30`]).
//! * `ts30-server` (`crates/ts30/server`): the server's hooks (`ServerRules`, the native bots'
//!   `NativeBotRules`) and the server's plugin object.
//! * `ts30-client` (`crates/ts30/client`): the client's hooks (`ClientRules`) and the client's
//!   plugin object.

#![deny(unsafe_code)]

use shared::gameplay::{GunRules, MeleeRules, PmRules};

/// The plugin's id: the `ts_gameplay` value, physinfo `gp` and `SrvSett` 4.
pub const ID: &str = "30";

/// TS 3.0's shared rules.
pub struct Ts30;

/// The plugin's shared hooks object.
pub static TS30: Ts30 = Ts30;

impl PmRules for Ts30 {}
impl GunRules for Ts30 {}
impl MeleeRules for Ts30 {}
