//! TS 3.0's HUD message rules that a ruleset can change. Addresses are TS 3.0 `client.dll` VAs.

use ts_client::hud::saytext::{SAY_COLOR, SayTextStyle};

/// `CHudSayText`: `SayTextPrint` (`0x1002ed00`) drops a muted player's line
/// (`GetClientVoiceMgr()->IsPlayerBlocked`, `0x10018c80`), and `Draw` (`0x1002eb40`) sets
/// `g_ColorYellow` (`0x1009359c`: 1, 0.7, 0) before a line without a coloured name
/// (`0x1002ec72..0x1002ec86`).
pub const SAY_TEXT: SayTextStyle = SayTextStyle { mute_blocked: true, plain_color: Some(SAY_COLOR) };

/// `Spectator` (`0x10065250`, `10065294..100652f8`) for the local player: entering observer
/// mode leaves third person; leaving it, `cl_thirdperson` 1 or 0 picks the camera (other values
/// keep it), so the death cam doesn't outlive the respawn.
pub fn spectator_toggles_third_person(observer: bool, third_person: bool, cl_thirdperson: f32) -> bool {
    if observer {
        third_person
    } else if cl_thirdperson == 1.0 {
        !third_person
    } else {
        cl_thirdperson == 0.0 && third_person
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Leaving observer mode (`0x10065250`): `cl_thirdperson` 0 leaves the death cam, 1 goes back
    /// to third person, other values keep the camera; entering always leaves third person.
    #[test]
    fn spectator_applies_cl_thirdperson() {
        let toggles = |third, cl| spectator_toggles_third_person(false, third, cl);
        assert!(toggles(true, 0.0), "the death cam doesn't survive the respawn");
        assert!(toggles(false, 1.0));
        assert!(!toggles(true, 1.0) && !toggles(false, 0.0));
        assert!(!toggles(true, 2.0) && !toggles(false, 2.0), "other values keep the camera");
        assert!(spectator_toggles_third_person(true, true, 1.0) && !spectator_toggles_third_person(true, false, 1.0));
    }
}
