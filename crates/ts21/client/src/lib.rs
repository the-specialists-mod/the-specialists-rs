//! TS 2.1's client rules: HUD and input (issue #100 §4), and the plugin object `client-dll`
//! registers.

#![deny(unsafe_code)]

use ts_client::gameplay::{ClientPlugin, ClientRules};
use ts21_gameplay::TS21;

/// TS 2.1's client-only rules.
pub struct Ts21Client;

impl ClientRules for Ts21Client {
    /// 2.1's server has no `ts_emitter`, `ts_particle` or `ts_fog` (issue #100 §3.9), and its
    /// client (`client.dll`, April 2004) no emitters or map fog: the classes and `CTS_CL_Emitter`
    /// came with 3.0.
    fn map_effects(&self) -> bool {
        false
    }
}

/// The plugin on the client: TS 2.1's shared hooks and its client hooks.
pub static PLUGIN: ClientPlugin = ClientPlugin { pm: &TS21, gun: &TS21, client: &Ts21Client };

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_map_effects() {
        assert!(!Ts21Client.map_effects());
        assert!(ts_client::gameplay::CORE_PLUGIN.client.map_effects());
    }
}
