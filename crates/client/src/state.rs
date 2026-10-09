//! Client state read by more than one module. Written by message handlers (hud) and read by
//! view/effects, or the other way round. Single-threaded: engine callbacks run on one thread.

use goldsrc::Global;
use std::cell::RefCell;

#[derive(Default)]
pub struct Shared {
    /// Slow-motion target from `TSSlowMo` (1.0 = normal) and the eased cosmetic clock.
    pub time_scale_target: f32,
    pub time_scale: f32,
    /// Local player is dead / spectating.
    pub dead: bool,
    pub health: i32,
    /// Current weapon id (TS ids) and zoom FOV (0 = default).
    pub weapon: i32,
    pub fov: f32,
}

static SHARED: Global<RefCell<Option<Shared>>> = Global(RefCell::new(None));

/// Borrow the shared state for the duration of `f`. Don't call engine functions that can call
/// back into the client inside `f`.
pub fn with<R>(f: impl FnOnce(&mut Shared) -> R) -> R {
    let mut s = SHARED.0.borrow_mut();
    let s = s.get_or_insert_with(|| Shared { time_scale_target: 1.0, time_scale: 1.0, fov: 90.0, ..Default::default() });
    f(s)
}
