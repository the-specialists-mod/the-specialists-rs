//! TS 3.0's particle builder parameters and its lasers' look.

use ts_client::effects::LaserStyle;
use ts_client::effects::fx::FxStyle;

/// TS 3.0's lasers, laser dot, flashlights and shadows: the animated, coloured local beam
/// (`0x1003c5f9`), the `PlInfo`-coloured dot at any range (`0x1004e6f0`), other players' lasers
/// and flashlights from `V_GetInEyePos` with the hand-line beam (`0x1003d600`, `0x1003b750`), the
/// fading TransAlpha shadows (`0x10055760`). Its sprites are grey: no tint.
pub static LASER: LaserStyle = LaserStyle {
    animated_beam: true,
    dot_range: None,
    others_from_origin: false,
    other_beams: true,
    shadow_fade: true,
    sprite_tint: [1.0; 3],
};

/// TS 3.0's particle builder parameters (muzzle flash, gun smoke, impact smoke, puffs, debris).
pub static FX: FxStyle = FxStyle {
    flash_color: 0.9,
    flash_fade: 0.2,
    flash_scale_speed: 0.2,
    third_person_flash: 1.3,
    muzzle_blast: true,
    muzzle_spark: false,
    silenced_count: 0.7,
    silenced_color: 0.000_585_937_5,
    silenced_fade: None,
    silenced_spin: 0,
    smoke_gravity: 0.01,
    smoke_puffs_by_cl_fx: false,
    drag: 3.3,
    stretch: true,
    debris_react: 0.3,
    debris_fade_any_mode: false,
    debris_count_by_quality: true,
    silenced_jitter: true,
    spark_moves: false,
    raw_slow_cooldowns: false,
    impact_smoke_speed: None,
};
