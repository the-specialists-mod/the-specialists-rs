//! TS's server studio blending interface (`animation.cpp`'s neighbour in `ts_i386.so`,
//! 0xa3f80-0xa400c). `server-dll` exports `Server_GetBlendingInterface` and holds the engine
//! structs; this is the part that isn't pointer plumbing.
//!
//! TS's interface is dead: `Server_GetBlendingInterface` fills in the engine's pointers but always
//! returns 0 (`mov $0x0,%eax` at 0xa3fd1), so FWGS and (Re)HLDS reset to their built-in
//! `SV_StudioSetupBones` and never call this one. Its only body is a log line.

use goldsrc::server::engine;
use goldsrc::sys::ALERT_TYPE;

/// `Server_GetBlendingInterface`'s return value (0xa3f80): 0 for every version, including the
/// version 1 it fills the interface in for.
pub fn blending_interface_result(_version: i32) -> i32 {
    0
}

/// Whether `Server_GetBlendingInterface` fills in the interface, the studio API copy and the two
/// matrix pointers for `version` (0xa3f98: `cmpl $0x1`).
pub fn blending_interface_accepts(version: i32) -> bool {
    version == 1
}

/// The log line [`sv_studio_setup_bones`] prints.
pub const SETUP_BONES_MSG: &str = "SV_StudioSetupBones called.\n";

/// `SV_StudioSetupBones` (0xa3fdc), `svBlending`'s only function: an `at_console` alert and
/// nothing else (no bones are set up).
pub fn sv_studio_setup_bones() {
    engine().alert(ALERT_TYPE::at_console, SETUP_BONES_MSG);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blending_interface_is_always_refused() {
        // TS fills the interface in for version 1 only, and returns 0 either way.
        assert!(blending_interface_accepts(1));
        assert!(!blending_interface_accepts(0));
        assert!(!blending_interface_accepts(2));
        for v in [-1, 0, 1, 2] {
            assert_eq!(blending_interface_result(v), 0);
        }
    }
}
