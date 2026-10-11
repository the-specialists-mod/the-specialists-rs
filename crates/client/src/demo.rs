//! Client data in demos: TS's `Demo_WriteBuffer` (`0x1001b860`) and the `Demo_ReadBuffer` export
//! (`0x1001b8c0`). Each record is a little-endian `int` type followed by the data. TS writes
//! three types, and playback copies each into a static that the reader applies:
//!
//! | type | data | written | read into, applied by |
//! |---|---|---|---|
//! | 0 | `float` zoom FOV target (`0x100df1a4`) | `HUD_GetFOV` (`0x10020a11`), every `CHud::Think` while recording | `0x100d6074`; `HUD_GetFOV` (`0x10020a25`) during playback |
//! | 1 | `int` alternate grip (`clientdata.vuser2[2]`) | `HUD_WeaponsPostThink` (`0x10003bf9`) when it changes | `0x100d6078`; `GetActiveAnimSet` (`0x1003df6c`) during playback |
//! | 2 | `int` `cam_thirdperson` | the `thirdperson` toggle (`0x1004fead`) | `0x100d607c`; the toggle during playback |

use goldsrc::Global;
use goldsrc::client::engine;
use std::cell::Cell;

/// The last recorded zoom FOV target (`0x100d6074`), the alternate grip (`0x100d6078`, only its
/// low byte is used) and `cam_thirdperson` (`0x100d607c`), all zero-initialised.
static RECORDED_FOV: Global<Cell<f32>> = Global(Cell::new(0.0));
static RECORDED_ALT_GRIP: Global<Cell<i32>> = Global(Cell::new(0));
static RECORDED_THIRDPERSON: Global<Cell<i32>> = Global(Cell::new(0));

/// Record type 0: the zoom FOV target `HUD_GetFOV` returns.
pub const FOV: i32 = 0;
/// Record type 1: the local weapon's alternate grip, when it changes.
pub const ALT_GRIP: i32 = 1;
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

/// `Demo_ReadBuffer` (`0x1001b8c0`), from the engine during playback. Every known type carries
/// four bytes of data.
pub fn read_buffer(buf: &[u8]) {
    let data = buf.get(4..8).map(|b| [b[0], b[1], b[2], b[3]]);
    match (buf.get(..4).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]])), data) {
        (Some(FOV), Some(d)) => RECORDED_FOV.0.set(f32::from_le_bytes(d)),
        (Some(ALT_GRIP), Some(d)) => RECORDED_ALT_GRIP.0.set(i32::from_le_bytes(d)),
        (Some(THIRDPERSON), Some(d)) => RECORDED_THIRDPERSON.0.set(i32::from_le_bytes(d)),
        (Some(FOV | ALT_GRIP | THIRDPERSON), None) => {}
        _ => engine().print("Unknown demo buffer type, skipping.\n"),
    }
}

/// The zoom FOV target as the demo being played last recorded it.
pub fn recorded_fov() -> f32 {
    RECORDED_FOV.0.get()
}

/// The alternate grip as the demo being played last recorded it (TS reads the low byte).
pub fn recorded_alt_grip() -> u8 {
    RECORDED_ALT_GRIP.0.get() as u8
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
        assert_eq!(record(FOV, &20.0f32.to_le_bytes()), [0, 0, 0, 0, 0, 0, 0xa0, 0x41]);
        assert_eq!(record(ALT_GRIP, &1i32.to_le_bytes()), [1, 0, 0, 0, 1, 0, 0, 0]);
    }

    #[test]
    fn read_back() {
        read_buffer(&record(FOV, &35.0f32.to_le_bytes()));
        read_buffer(&record(ALT_GRIP, &0x101i32.to_le_bytes()));
        read_buffer(&record(THIRDPERSON, &1i32.to_le_bytes()));
        assert_eq!((recorded_fov(), recorded_alt_grip(), recorded_thirdperson()), (35.0, 1, 1));
    }
}
