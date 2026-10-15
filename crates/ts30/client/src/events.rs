//! TS 3.0's weapon-event numbers: the client bullets' step period and the melee material sound.

/// Attenuation of the material sound (`0x10045034`, `0x3f7851ec`).
pub const MATERIAL_ATTN: f32 = 0.97;

/// When a slot's next trace step is due, set when one runs at client time `now` (`0x10035b97`):
/// every 0.1 s with `realbullet` 0, every 0.05 s with 1, otherwise every frame.
pub fn next_update(now: f32, realbullet: i32) -> f32 {
    if (0..2).contains(&realbullet) { 0.1 / (realbullet + 1) as f32 + now } else { now }
}

/// `TS_MeleeMaterialSound`'s volume by texture type (`0x10045f0b..0x10046044`): 0.8 for tile
/// and glass, 0.5 for duct, 0.9 for the rest; flesh plays nothing.
pub fn melee_material_volume(tex: u8) -> Option<f32> {
    match tex {
        b'F' => None,
        b'T' | b'P' | b'Y' => Some(0.8),
        b'V' => Some(0.5),
        _ => Some(0.9),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The trace-step throttle (0x10035b97..0x10035bc9).
    #[test]
    fn client_bullet_throttle() {
        assert_eq!(next_update(10.0, 0), 10.0 + 0.1f32);
        assert_eq!(next_update(10.0, 1), 10.0 + 0.05f32);
        assert_eq!(next_update(10.0, 2), 10.0, "2 and up: every frame");
        assert_eq!(next_update(10.0, -1), 10.0);
        for rb in [i32::MIN, -1, 0, 1, 2, i32::MAX] {
            let _ = next_update(1.0, rb);
        }
    }

    /// Issue #77: `TS_MeleeMaterialSound`'s volumes (0.9 `0x3f666666`, 0.8 `0x3f4ccccd`, 0.5
    /// `0x3f000000`).
    #[test]
    fn melee_material_volumes() {
        for (tex, want) in [(b'T', Some(0.8)), (b'P', Some(0.8)), (b'Y', Some(0.8)), (b'V', Some(0.5)), (b'F', None)] {
            assert_eq!(melee_material_volume(tex), want, "{}", tex as char);
        }
        for tex in [0, b'C', b'M', b'D', b'G', b'S', b'W', b'X'] {
            assert_eq!(melee_material_volume(tex), Some(0.9), "{}", tex as char);
        }
    }
}
