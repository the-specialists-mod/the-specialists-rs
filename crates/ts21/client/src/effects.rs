//! TS 2.1's own effects outside the weapon events: the lasers' look (issue #204).

use ts_client::effects::LaserStyle;

/// 2.1's lasers, laser dot, flashlights and shadows:
///
/// * the local beam (`0x10071bd0`): `laserbeam.spr` frame 0, colour `(j + 1) / 8 + 0.75` from
///   the frame's first jitter roll, half-width 1.5 (`0x1007284d..`);
/// * the HUD dot (`0x1008b410`): red, only while the hit is less than 4096 units away
///   (`fcomp 4096.0` at `0x1008b4c7`), the first gun's alpha `ftol((4096 − d) / 4096 × 255)`
///   (`0x1008b520..0x1008b53a`), the second's 255;
/// * other players' lasers (`0x100731a0`) and flashlights (`0x10071630`) from `origin + 24` with
///   the angles' pitch × −3; the lasers a white `laserdot.spr` (radius 4) and no beam;
/// * shadows (`0x10095260`, `0x10094e70`): `RenderMode(2)`, `Color4f(1, 1, 1, 1)` once, no
///   distance fade or cut-off, traces with flags 2.
pub static LASER: LaserStyle =
    LaserStyle { animated_beam: false, dot_range: Some(4096.0), others_from_origin: true, other_beams: false, shadow_fade: false };
