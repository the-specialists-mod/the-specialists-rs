//! TS 3.0's spectator panel visibility.

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
}
