//! Byte strings for player-provided text. TS handles chat, names, the MOTD and its log lines as C
//! strings: whatever bytes the client sent (a Windows client's CP1252 `é` is the single byte
//! 0xE9) are copied, `sprintf`ed and written to the wire unchanged (`Host_Say` 0xad02c,
//! `ClientUserInfoChanged` 0xae638). A Rust `String` can't hold those bytes, so the port carries
//! such text as `Vec<u8>`/`&[u8]` and builds lines with [`bformat!`](crate::bformat) instead of
//! `format!` (which would need a lossy conversion that turns 0xE9 into `EF BF BD`).

use std::ffi::{CStr, CString};

/// A piece of a [`bformat!`](crate::bformat) line, appended as its raw bytes. Text types add
/// their bytes unchanged; numbers add their decimal `Display` form (`%i`).
pub trait Piece {
    fn put(&self, out: &mut Vec<u8>);
}

impl Piece for [u8] {
    fn put(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(self);
    }
}
impl<const N: usize> Piece for [u8; N] {
    fn put(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(self);
    }
}
impl Piece for Vec<u8> {
    fn put(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(self);
    }
}
impl Piece for str {
    fn put(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(self.as_bytes());
    }
}
impl Piece for String {
    fn put(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(self.as_bytes());
    }
}
impl Piece for CStr {
    fn put(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(self.to_bytes());
    }
}
impl Piece for CString {
    fn put(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(self.to_bytes());
    }
}
impl<T: Piece + ?Sized> Piece for &T {
    fn put(&self, out: &mut Vec<u8>) {
        (**self).put(out);
    }
}
macro_rules! display_piece {
    ($($t:ty),*) => {$(
        impl Piece for $t {
            fn put(&self, out: &mut Vec<u8>) {
                out.extend_from_slice(self.to_string().as_bytes());
            }
        }
    )*};
}
display_piece!(i8, u8, i16, u16, i32, u32, i64, u64, usize, isize, char);

/// The engine of [`bformat!`](crate::bformat): each `{}` in `fmt` is replaced by the next piece;
/// `{{` and `}}` are literal braces. Only plain `{}` is supported (format numbers with a width
/// or precision with `format!` first and pass the `String`).
pub fn format(fmt: &str, args: &[&dyn Piece]) -> Vec<u8> {
    let mut out = Vec::with_capacity(fmt.len() + 16 * args.len());
    let mut args = args.iter();
    let b = fmt.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match (b[i], b.get(i + 1)) {
            (b'{', Some(b'}')) => {
                let a = args.next();
                debug_assert!(a.is_some(), "bformat!: more {{}} than arguments in {fmt:?}");
                if let Some(a) = a {
                    a.put(&mut out);
                }
                i += 2;
            }
            (b'{', Some(b'{')) | (b'}', Some(b'}')) => {
                out.push(b[i]);
                i += 2;
            }
            (c, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    debug_assert!(args.next().is_none(), "bformat!: more arguments than {{}} in {fmt:?}");
    out
}

/// `format!` for byte strings: `bformat!("* {} changed name to {}\n", old, new)` gives a
/// `Vec<u8>` with each argument's raw bytes (see [`Piece`]). Arguments are positional `{}` only.
#[macro_export]
macro_rules! bformat {
    ($fmt:literal $(, $a:expr)* $(,)?) => {
        $crate::text::format($fmt, &[$(&$a as &dyn $crate::text::Piece),*])
    };
}

/// The first `max` bytes of `text` with NULs dropped: a C `char[max + 1]` copy (`strncpy` and a
/// terminator), and what `WRITE_STRING` can carry.
pub fn clip(text: &[u8], max: usize) -> Vec<u8> {
    let mut out: Vec<u8> = text.iter().copied().filter(|&b| b != 0).collect();
    out.truncate(max);
    out
}

/// The text as one `char` per byte (Latin-1), for code that wants a `String` but must not lose
/// bytes: the client's HUD keeps network strings like this, and [`latin1_bytes`] turns them back.
pub fn latin1(b: &[u8]) -> String {
    b.iter().map(|&c| c as char).collect()
}

/// The inverse of [`latin1`]: a char above U+00FF (which no network string decodes to) becomes `?`.
pub fn latin1_bytes(s: &str) -> Vec<u8> {
    s.chars().map(|c| if (c as u32) < 256 { c as u8 } else { b'?' }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bformat_keeps_bytes() {
        let name: &[u8] = b"Ren\xe9";
        let line = crate::bformat!("\"{}<{}><{}>\" say \"{}\"\n", name, 3, c"STEAM_0:1", b"caf\xe9".to_vec());
        assert_eq!(line, b"\"Ren\xe9<3><STEAM_0:1>\" say \"caf\xe9\"\n");
        assert_eq!(crate::bformat!("{{}} {}", "x"), b"{} x");
        assert_eq!(crate::bformat!("plain"), b"plain");
    }

    #[test]
    fn clip_and_latin1() {
        assert_eq!(clip(b"a\0b\xe9cd", 3), b"ab\xe9");
        let all: Vec<u8> = (1..=255).collect();
        assert_eq!(latin1_bytes(&latin1(&all)), all);
        assert_eq!(latin1_bytes("\u{20ac}"), b"?");
    }
}
