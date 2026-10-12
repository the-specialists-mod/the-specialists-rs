//! The TS 2.1 gameplay plugin (issue #100): TS 2.1's rules (April 2004 `mp.dll`/`client.dll`,
//! decomp `docs/ts21/`) as a ruleset the server selects at runtime with `ts_gameplay 21`.
//!
//! It implements the hook traits, overriding only what 2.1 changes; every hook it doesn't
//! override keeps TS 3.0's body. The 3.0 crates never name it: `server-dll` and `client-dll`
//! register it under [`ID`] behind their `ts21` feature. See `docs/implementation/ts21.md`.
//!
//! * This package, [`shared`]: the movement and gun hooks both sides run (`PmRules`, `GunRules`,
//!   on [`Ts21`]).
//! * `ts21-server` (`crates/ts21/server`): the server's hooks (`ServerRules`) and the server's
//!   plugin object.
//! * `ts21-client` (`crates/ts21/client`): the client's hooks (`ClientRules`) and the client's
//!   plugin object.
//!
//! Done so far: movement and the move speed (`shared::pm`). Every other hook keeps TS 3.0's body.

#![deny(unsafe_code)]

pub mod shared;

/// The plugin's id: `ts_gameplay 21`, physinfo `gp` "21", `SrvSett` 4 value 21.
pub const ID: &str = "21";

/// TS 2.1's shared rules: the movement and gun hooks.
pub struct Ts21;

/// The shared hook object both sides' plugin objects point at.
pub static TS21: Ts21 = Ts21;

#[cfg(test)]
mod tests {
    #[test]
    fn id_fits_the_hud_channel() {
        assert_eq!(ts_gameplay::srvsett_value(super::ID), Some(21));
    }
}
