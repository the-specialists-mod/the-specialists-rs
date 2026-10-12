//! Gameplay plugins: rulesets selected at runtime (`ts_gameplay`), each a set of hook objects the
//! game code calls instead of hard-coding its rules. TS 3.0's own rules are the core plugin
//! (`"30"`) and the default behaviour of every hook. See `docs/implementation/gameplay-plugins.md`.
//!
//! This crate holds only what every side shares and nothing that names a game type: the ids, the
//! channels that carry the active one ([`CVAR`], [`PHYSINFO_KEY`], [`SRVSETT_KEY`]) and the
//! [`Registry`]. The hook traits live with the types they take, one module each:
//! `shared::gameplay::{PmRules, GunRules}`, `ts_server::gameplay::ServerRules` and
//! `ts_client::gameplay::ClientRules`.

#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

mod info;
mod registry;

pub use info::info_value;
pub use registry::{Registry, Resolved};

/// The core plugin's id: TS 3.0's rules. Also the fallback for an unknown or empty id.
pub const DEFAULT_ID: &str = "30";

/// The server cvar that selects the active plugin by id (port addition; TS has no such cvar).
pub const CVAR: &str = "ts_gameplay";

/// The physinfo key that carries the active plugin's id to both sides' shared code, per command:
/// the server's `pfnPM_Move` and the client's `HUD_PlayerMove` (`pmove->physinfo`), the server's
/// gun and the client's weapon prediction (`clientdata_t::physinfo`).
pub const PHYSINFO_KEY: &str = "gp";
/// [`PHYSINFO_KEY`] for the engine's physics-key calls.
pub const PHYSINFO_KEY_C: &core::ffi::CStr = c"gp";

/// The `SrvSett` key that carries the active plugin's id to the client HUD (TS 3.0 sends keys
/// 0..3, and its client ignores the others). The value is the id as a number ([`srvsett_value`]).
pub const SRVSETT_KEY: u8 = 4;

/// The `SrvSett` value for a plugin id. Ids are short decimal numbers, so they fit the message's
/// `short`; `None` for an id that isn't one (it can't be registered, see [`Registry::add`]).
pub fn srvsett_value(id: &str) -> Option<i16> {
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    id.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srvsett_values_are_the_numeric_ids() {
        assert_eq!(srvsett_value(DEFAULT_ID), Some(30));
        assert_eq!(srvsett_value("7"), Some(7));
        assert_eq!(srvsett_value(""), None);
        assert_eq!(srvsett_value("+3"), None);
        assert_eq!(srvsett_value("99999"), None);
    }
}
