//! Client data in demos: TS's `Demo_WriteBuffer` (`0x1001b860`) and the `Demo_ReadBuffer` export
//! (`0x1001b8c0`). Each record is a little-endian `int` type followed by the data. TS writes
//! three types: 0 and 1 (`0x10003bf9`, `0x10020a11`; read back into `0x100d6074`/`0x100d6078`)
//! and 2, the `thirdperson` state (`0x1004fead`, read into `0x100d607c`). Only type 2 is
//! ported; see client-rendering.md.

use goldsrc::Global;
use goldsrc::client::engine;
use std::cell::Cell;

/// `cam_thirdperson` as last recorded in the demo being played (`0x100d607c`, zero-initialised).
static RECORDED_THIRDPERSON: Global<Cell<i32>> = Global(Cell::new(0));

/// Record type 2: the new `cam_thirdperson` after a `thirdperson` toggle.
pub const THIRDPERSON: i32 = 2;

/// The record `Demo_WriteBuffer(kind, data)` hands to the engine's `WriteBuffer`.
pub fn record(kind: i32, data: &[u8]) -> Vec<u8> {
    let mut b = kind.to_le_bytes().to_vec();
    b.extend_from_slice(data);
    b
}

/// `Demo_WriteBuffer` (`0x1001b860`).
pub fn write_buffer(kind: i32, data: &[u8]) {
    engine().demo_write_buffer(&record(kind, data));
}

/// `Demo_ReadBuffer` (`0x1001b8c0`), from the engine during playback.
pub fn read_buffer(buf: &[u8]) {
    let int = |i: usize| buf.get(i..i + 4).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    match int(0) {
        Some(THIRDPERSON) => {
            if let Some(v) = int(4) {
                RECORDED_THIRDPERSON.0.set(v);
            }
        }
        // Types 0 and 1 are recognised but not ported (see the module docs).
        Some(0 | 1) => {}
        _ => engine().print("Unknown demo buffer type, skipping.\n"),
    }
}

/// `cam_thirdperson` as the demo being played last recorded it.
pub fn recorded_thirdperson() -> i32 {
    RECORDED_THIRDPERSON.0.get()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_layout() {
        assert_eq!(record(THIRDPERSON, &1i32.to_le_bytes()), [2, 0, 0, 0, 1, 0, 0, 0]);
    }
}
