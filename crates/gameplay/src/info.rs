//! Info-string lookup (`\key\value\key\value`), for reading [`PHYSINFO_KEY`](crate::PHYSINFO_KEY)
//! from a `clientdata_t::physinfo` buffer without an engine callback.

/// The value of `key` in the NUL-terminated info string `buf` (`Info_ValueForKey`), or `None` when
/// the key is absent. Bytes after the first NUL are ignored.
pub fn info_value<'a>(buf: &'a [u8], key: &[u8]) -> Option<&'a [u8]> {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    let s = buf[..end].strip_prefix(b"\\").unwrap_or(&buf[..end]);
    let mut it = s.split(|&b| b == b'\\');
    while let Some(k) = it.next() {
        let v = it.next()?;
        if k == key {
            return Some(v);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_keys_in_physinfo_strings() {
        let buf = b"\\slj\\0\\hl\\1\\gp\\30\\mj\\0\0garbage\\gp\\7";
        assert_eq!(info_value(buf, b"gp"), Some(&b"30"[..]));
        assert_eq!(info_value(buf, b"mj"), Some(&b"0"[..]));
        assert_eq!(info_value(buf, b"g"), None);
        assert_eq!(info_value(b"\0", b"gp"), None);
        assert_eq!(info_value(b"\\gp\\", b"gp"), Some(&b""[..]));
        assert_eq!(info_value(b"\\gp", b"gp"), None);
    }
}
