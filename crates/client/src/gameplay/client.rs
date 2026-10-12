//! [`ClientRules`]: the client-only hooks (HUD and input).
//!
//! The HUD resolves them from the server's `SrvSett` key [`SRVSETT_KEY`](ts_gameplay::SRVSETT_KEY)
//! ([`hud_rules`](super::hud_rules)), which `InitHUD` sends and a switch re-sends.

/// Client rules. Every default body is TS 3.0's (`shared::gameplay::Core`); the HUD and input code
/// call these where a ruleset can differ.
pub trait ClientRules: Sync {
    /// The divestep indicator (`DrawCrosshair`'s helper 1004ef50, thresholds 1004eec0/1004eef0/
    /// 1004ef20): sprite `i` turns red when the slots in use (`81 − free`) reach `limits[i]`.
    fn divestep_limits(&self) -> [i32; 3] {
        [42, 52, 62]
    }
}
