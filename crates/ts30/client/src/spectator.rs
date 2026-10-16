//! TS 3.0's spectator panel visibility and `HUD_AddEntity`'s spectator tail.

use ts_client::gameplay::SpectatorTail;

/// `HUD_AddEntity`'s tail (`0x10044af9..0x10044b2a`): while observing (`g_iUser1 != 0`), the
/// overview icon, and the observer target (`g_iUser2`) hidden in-eye (`g_iUser1 == 5`) or with
/// the in-eye inset (2).
pub fn add_entity_tail(observer_mode: i32, observer_target: i32, in_eye: bool, inset: i32) -> SpectatorTail {
    if observer_mode == 0 {
        return SpectatorTail::Skip;
    }
    SpectatorTail::Overview { hide: (in_eye || inset == 2).then_some(observer_target) }
}

/// Whether `nohud` lets the panel show (`0x10063912..0x10063925`): `fcomp 0.0` on the float with
/// `test ah, 0x40`, so only an exact 0, or a NaN (unordered sets C3), shows it (issue #185).
pub fn nohud_shows(nohud: f32) -> bool {
    nohud == 0.0 || nohud.is_nan()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `nohud` is compared as a float (issue #185): 0.5 hides the bar, a NaN shows it.
    #[test]
    fn nohud_is_a_float_compare() {
        assert!(nohud_shows(0.0) && nohud_shows(-0.0) && nohud_shows(f32::NAN));
        assert!(!nohud_shows(0.5) && !nohud_shows(-0.25) && !nohud_shows(1.0));
    }

    /// `0x10044af9`: the tail runs while observing and hides the target in-eye or with inset 2.
    #[test]
    fn add_entity_tail_while_observing() {
        assert_eq!(add_entity_tail(0, 3, true, 2), SpectatorTail::Skip);
        assert_eq!(add_entity_tail(4, 3, false, 0), SpectatorTail::Overview { hide: None });
        assert_eq!(add_entity_tail(5, 3, true, 0), SpectatorTail::Overview { hide: Some(3) });
        assert_eq!(add_entity_tail(4, 3, false, 2), SpectatorTail::Overview { hide: Some(3) });
    }
}
