//! Message payloads for the stunt area (layouts: decomp server/architecture.md §3.2,
//! client/architecture-hud.md §3). The server writes them with `MsgWriter`; these are the
//! layouts and the client-side decoders.
//!
//! | Message | Dest | Payload |
//! |---|---|---|
//! | `NextStunt` | ONE | byte: stunt cooldown in tenths of a second (`round(next * 10)`) |
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

pub fn encode_next_stunt(seconds: f32) -> u8 {
    (seconds * 10.0).round_ties_even().clamp(0.0, 255.0) as u8
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
        assert_eq!(encode_next_stunt(0.55), 6);
        assert_eq!(decode_next_stunt(&[5]), Some(0.5));
        assert_eq!(decode_state(&[2]), Some(stance::PRONE));
        assert_eq!(decode_motion_blur(&[3, 2]), Some((3, 2)));
        assert_eq!(decode_motion_blur(&[3]), None);
    }
}
