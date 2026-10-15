use std::ffi::{CStr, CString, c_char};

/// Borrows a C string from the engine, treating null as empty.
///
/// # Safety
/// `p` must be null or point to a NUL-terminated string that outlives `'a`.
pub unsafe fn cstr<'a>(p: *const c_char) -> &'a CStr {
    // SAFETY: non-null `p` is a NUL-terminated string that outlives `'a` (the caller's contract above).
    if p.is_null() { c"" } else { unsafe { CStr::from_ptr(p) } }
}

/// Reads a NUL-terminated string from a fixed-size C `char` array (struct fields such as
/// `mstudioevent_t::options`). Stops at the array end if there is no NUL; never reads past it.
pub fn c_array_str(a: &[std::ffi::c_char]) -> String {
    let bytes: Vec<u8> = a.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Converts a Rust string to a C string, dropping interior NULs instead of failing.
pub fn to_cstring(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap()
}

/// Converts bytes to a C string unchanged (no UTF-8 conversion: player text is whatever the
/// client sent), dropping interior NULs instead of failing.
pub fn bytes_to_cstring(b: &[u8]) -> CString {
    CString::new(b.iter().copied().filter(|&c| c != 0).collect::<Vec<u8>>()).unwrap_or_default()
}

/// Parses a keyvalue vector such as `"128 -64 32"`. Missing components are zero.
pub fn parse_vec3(s: &str) -> [f32; 3] {
    let mut out = [0.0; 3];
    for (o, part) in out.iter_mut().zip(s.split_whitespace()) {
        *o = part.parse().unwrap_or(0.0);
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn parse_vec3() {
        assert_eq!(super::parse_vec3("1 -2.5 3"), [1.0, -2.5, 3.0]);
        assert_eq!(super::parse_vec3("7"), [7.0, 0.0, 0.0]);
    }
}
