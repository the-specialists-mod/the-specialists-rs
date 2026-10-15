//! TS 3.0's particle builder parameters.

use ts_client::effects::fx::FxStyle;

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
};
