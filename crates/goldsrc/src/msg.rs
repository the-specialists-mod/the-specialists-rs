//! Reader for user messages, mirroring the server's `pfnWrite*` calls.

use std::ffi::CStr;

pub struct MsgReader<'a> {
    buf: &'a [u8],
    pos: usize,
    pub bad: bool,
}

impl<'a> MsgReader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0, bad: false }
    }

    /// # Safety
    /// `buf` must point to `size` readable bytes for the lifetime `'a`.
    pub unsafe fn from_raw(buf: *const std::ffi::c_void, size: i32) -> Self {
        if buf.is_null() || size <= 0 {
            return Self::new(&[]);
        }
        Self::new(unsafe { std::slice::from_raw_parts(buf.cast(), size as usize) })
    }

    fn take<const N: usize>(&mut self) -> [u8; N] {
        match self.buf.get(self.pos..self.pos + N) {
            Some(b) => {
                self.pos += N;
                b.try_into().unwrap()
            }
            None => {
                self.bad = true;
                [0; N]
            }
        }
    }

    pub fn byte(&mut self) -> u8 {
        self.take::<1>()[0]
    }

    pub fn short(&mut self) -> i16 {
        i16::from_le_bytes(self.take())
    }

    pub fn long(&mut self) -> i32 {
        i32::from_le_bytes(self.take())
    }

    pub fn string(&mut self) -> &'a CStr {
        match CStr::from_bytes_until_nul(&self.buf[self.pos.min(self.buf.len())..]) {
            Ok(s) => {
                self.pos += s.to_bytes_with_nul().len();
                s
            }
            Err(_) => {
                self.bad = true;
                c""
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_in_order() {
        let mut r = MsgReader::new(&[7, 0x34, 0x12, b'h', b'i', 0]);
        assert_eq!(r.byte(), 7);
        assert_eq!(r.short(), 0x1234);
        assert_eq!(r.string(), c"hi");
        assert!(!r.bad);
        r.byte();
        assert!(r.bad);
    }
}
