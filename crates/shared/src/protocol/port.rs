//! Port additions that tell the two sides apart from TS 3.0's own libraries, on channels TS 3.0
//! tolerates (issue #100, `docs/implementation/gameplay-plugins.md` §2):
//!
//! * The port client announces itself and its version in its userinfo ([`USERINFO_KEY`], a
//!   `FCVAR_USERINFO` cvar the client registers). TS 3.0's client has no such cvar, so the key
//!   is missing from its userinfo. The server reads it in `ClientConnect`.
//! * The port server sends its version to the HUD as `SrvSett` keys [`SRVSETT_VERSION_KEYS`]
//!   (major, minor, patch) after key 4. TS 3.0's `SrvSett` handler (client.dll 0x1004b840)
//!   switches on keys 0..3 and only sets the HUD's redraw flag for any other key.

/// The version of the library this is built into: the workspace's (`Cargo.toml`), which the
/// release tags follow (`v3.0.2`).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// [`VERSION`] for the engine (the client's cvar default).
pub const VERSION_C: &std::ffi::CStr = match std::ffi::CStr::from_bytes_with_nul(concat!(env!("CARGO_PKG_VERSION"), "\0").as_bytes()) {
    Ok(c) => c,
    Err(_) => panic!("the package version has a NUL"),
};

/// The userinfo key (and client cvar) that carries the port client's version. The leading `_`
/// keeps it out of the userinfo the engines send to the other clients.
pub const USERINFO_KEY: &str = "_tsrs";
/// [`USERINFO_KEY`] for the engine's info-key calls.
pub const USERINFO_KEY_C: &std::ffi::CStr = c"_tsrs";

/// The `SrvSett` keys that carry the server's version: major, minor, patch.
pub const SRVSETT_VERSION_KEYS: [u8; 3] = [5, 6, 7];

/// `major.minor.patch` as three numbers, each fitting `SrvSett`'s `short`; anything after the
/// patch number (`-rc1`, `+dirty`) is dropped. `None` if it isn't such a version.
pub fn parse_version(v: &str) -> Option<[i16; 3]> {
    let mut parts = v.trim().splitn(3, '.');
    let mut out = [0i16; 3];
    for (i, slot) in out.iter_mut().enumerate() {
        let p = parts.next()?;
        let p = if i == 2 { p.split(|c: char| !c.is_ascii_digit()).next()? } else { p };
        if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        *slot = p.parse().ok()?;
    }
    Some(out)
}

/// This library's version as `SrvSett` values.
pub fn version_numbers() -> [i16; 3] {
    parse_version(VERSION).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(parse_version("3.0.2"), Some([3, 0, 2]));
        assert_eq!(parse_version(" 10.20.30-rc1 "), Some([10, 20, 30]));
        assert_eq!(parse_version("3.0"), None);
        assert_eq!(parse_version("3.x.1"), None);
        assert_eq!(parse_version("99999.0.0"), None, "too big for a short");
        assert_eq!(parse_version(""), None);
        assert!(parse_version(VERSION).is_some(), "the workspace version is numeric");
    }

    #[test]
    fn version_keys_follow_the_ruleset_key() {
        // TS 3.0 uses 0..3 and the gameplay ruleset 4.
        assert!(SRVSETT_VERSION_KEYS.iter().all(|&k| k > 4));
    }
}
