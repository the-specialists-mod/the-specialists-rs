//! Message payloads for the stunt area (layouts: decomp server/architecture.md §3.2,
//! client/architecture-hud.md §3). The server writes them with `MsgWriter`; these are the
//! layouts and the client-side decoders.
//!
//! | Message | Dest | Payload |
//! |---|---|---|
//! | `NextStunt` | ONE | byte: stunt cooldown in tenths of a second (`(int)(next * 10)`, truncated) |
//! | `TSState` | ONE | byte: stance, 0 standing, 1 crouched, 2 prone/diving (`iuser4 & 0x30`) |
//! | `TSMBlur` | PVS | byte entity index, byte level (1 or 2) |

/// `TSState` stance values (index `sprites/player/{stand,crouched,prone}.spr`).
pub mod stance {
    pub const STAND: u8 = 0;
    pub const CROUCH: u8 = 1;
    pub const PRONE: u8 = 2;
}

/// `NextStunt`: cooldown in seconds.
pub fn decode_next_stunt(buf: &[u8]) -> Option<f32> {
    buf.first().map(|&b| b as f32 * 0.1)
}

/// `NextStunt`'s byte for a cooldown of `seconds`: `WriteByte((int)(next * 10.0f))`, truncated
/// in both TS 3.0 servers (`mp.dll`'s `__ftol` 0x1007d310, e.g. after `GoWallJump` 0x10068cdc;
/// `ts_i386.so` loads a truncating control word before each `fistp`, e.g. 0x8a929, which
/// Ghidra shows as `ROUND`). The `float` times `10.0f` is exact on the x87 (it fits in 53 bits),
/// so the product is taken in `f64`, not rounded to `f32` first; `WriteByte` keeps the low byte.
pub fn encode_next_stunt(seconds: f32) -> u8 {
    use crate::pm::Num;
    (f64::from(seconds) * 10.0).trunc_i32() as u8
}

/// `TSState`: the stance byte.
pub fn decode_state(buf: &[u8]) -> Option<u8> {
    buf.first().copied()
}

/// `TSMBlur`: (entity index, blur level).
pub fn decode_motion_blur(buf: &[u8]) -> Option<(u8, u8)> {
    match buf {
        [ent, level, ..] => Some((*ent, *level)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        assert_eq!(encode_next_stunt(0.5), 5);
        assert_eq!(decode_next_stunt(&[5]), Some(0.5));
        assert_eq!(decode_state(&[2]), Some(stance::PRONE));
        assert_eq!(decode_motion_blur(&[3, 2]), Some((3, 2)));
        assert_eq!(decode_motion_blur(&[3]), None);
    }

    /// Truncated, from the exact product (TS 3.0's bytes; the port used to round the `f32`
    /// product to nearest, sending 6, 7 and 10 for these).
    #[test]
    fn next_stunt_truncates_the_exact_product() {
        assert_eq!(encode_next_stunt(0.55), 5, "GoBulletTime's cooldown");
        assert_eq!(encode_next_stunt(0.7), 6, "two wall jumps");
        assert_eq!(encode_next_stunt(0.95), 9, "0.95f * 10 is 9.4999999, the f32 product 9.5");
        assert_eq!(encode_next_stunt(0.85), 8);
        assert_eq!(encode_next_stunt(0.0), 0);
        assert_eq!(encode_next_stunt(25.6), 0, "the low byte of 256");
        assert_eq!(encode_next_stunt(30.0), 44, "the low byte of 300");
    }

    #[test]
    fn decoders_survive_garbage() {
        for b in [&[][..], &[0xff], &[0xff; 2], &[0xff; 300]] {
            let _ = (decode_next_stunt(b), decode_state(b), decode_motion_blur(b));
        }
        for s in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0, 1e30] {
            let _ = encode_next_stunt(s);
        }
    }
}
