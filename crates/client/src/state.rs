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

    // ---- added by the render package -------------------------------------------------------
    /// Zoom target from `SetFOV` (0 = default_fov). Written by the HUD `SetFOV` handler; the
    /// render package eases `fov` toward it every frame (TS `CHud::Think`, animated zoom).
    pub fov_target: f32,
    /// `TSBTime` bullet-time flag (render hooks `TSBTime`).
    pub bullet_time: bool,
    /// Local power-up bits from `clientdata.vuser1[1]` (1 slow-mo, 2 infinite clip, 8 slow-pause,
    /// 0x10 double fire rate). Written by prediction.
    pub powerups: i32,
    /// Local weapon state from clientdata (prediction): attachments `vuser3[2]` (1 silencer,
    /// 2 laser, 4 flashlight, 8 scope), active items `vuser1[2]`, firemode `vuser3[1]`, clip.
    pub attachments: i32,
    pub active_items: i32,
    pub firemode: i32,
    pub clip: i32,
    pub reserve_ammo: i32,
    /// `ActItems` per player (index = entindex - 1): bit 2 laser on, 4 flashlight on, 0x40 akimbo.
    /// Written by the HUD `ActItems` handler; read by the laser/flashlight drawing.
    pub player_items: [u8; 32],
    /// `PlInfo` key 0 per player (index = entindex - 1): laser colour 0 red, 1 green, 2 blue.
    pub laser_color: [u8; 32],
    /// `Spectator` message per player (index = entindex - 1): not drawn when set.
    pub spectator: [bool; 32],
    /// Screen positions of the local laser dot(s) this frame (second = akimbo), for the HUD's
    /// `DrawLaserDots`. Written by effects in the transparent pass.
    pub laser_dots: [Option<[f32; 2]>; 2],
    /// Third-person camera active (render writes; HUD may read for the crosshair).
    pub third_person: bool,
    /// Spectator globals from `HUD_ProcessPlayerState` (g_iUser1/2/3): mode, target, flags.
    pub spec_mode: i32,
    pub spec_target: i32,
    pub spec_flags: i32,
}

static SHARED: Global<RefCell<Option<Shared>>> = Global(RefCell::new(None));

/// Borrow the shared state for the duration of `f`. Don't call engine functions that can call
/// back into the client inside `f`.
pub fn with<R>(f: impl FnOnce(&mut Shared) -> R) -> R {
    let mut s = SHARED.0.borrow_mut();
    let s = s.get_or_insert_with(|| Shared { time_scale_target: 1.0, time_scale: 1.0, fov: 90.0, ..Default::default() });
    f(s)
}
