//! Client state read by more than one module. Written by message handlers (hud) and read by
//! view/effects, or the other way round. Single-threaded: engine callbacks run on one thread.

use goldsrc::Global;
use std::cell::RefCell;

/// A TS effect message the HUD parsed for the effects package (`PTakeDam`, `TSFrags`, `TSPSys`
/// type 3, `TSMBlur`). The HUD only queues them; `effects` drains `Shared::ts_effects`.
#[derive(Debug, Clone, PartialEq)]
pub enum TsEffect {
    /// `PTakeDam` → `FX_PlayerHit` (10048840): blood at `pos` along `dir`.
    PlayerHit { pos: [f32; 3], dir: [f32; 3], amount: u8 },
    /// `TSFrags`: 0 breakable gibs, 1 smoke, 2 metal sparks.
    Frags(shared::protocol::hud::TsFrags),
    /// `TSPSys` type 3: directional wind/rain emitter.
    Wind { seed: i32, yaw: i32, strength: f32 },
    /// `TSMBlur`: toggle the motion-blur trail on an entity.
    MotionBlur { entity: i32 },
}

/// `TSFade` as received (applied by `view` in `V_CalcRefdef`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TsFadeState {
    /// Seconds (the message's 4.12 fixed-point values / 4096).
    pub duration: f32,
    pub hold: f32,
    pub flags: u16,
    pub rgba: [u8; 4],
    /// Client time the message arrived.
    pub start: f32,
}

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
    // ---- added by the hud package ----
    /// Local player is an observer (`Spectator` message).
    pub observer: bool,
    /// Predicted spread penalty of the current gun (TS `0x1018d188`); the crosshair opens by
    /// `spread × 160` px. Written by `prediction`, read by the HUD.
    pub spread: f32,
    /// Last `TSFade`, if any.
    pub ts_fade: Option<TsFadeState>,
    /// Per-client active attachment bits (`ActItems`, laser/flashlight on; 0x40 = akimbo).
    pub active_items: Vec<u8>,
    /// Per-client laser colour index (`PlInfo` key 0).
    pub laser_color: Vec<u8>,
    /// Effect messages waiting for `effects`.
    pub ts_effects: Vec<TsEffect>,
    /// Third-person toggles requested by TS HUD logic (death cam on `TSHealth` 0,
    /// `cl_thirdpersonswitch`, observer changes). `view` consumes them.
    pub thirdperson_toggles: u32,
    /// Leave / restore third person on entering / leaving observer mode.
    pub thirdperson_leave: bool,
    pub thirdperson_restore: bool,
    /// The weapon menu, buy menu or another HUD menu is open (view/effects may hide things).
    pub menu_open: bool,
}

static SHARED: Global<RefCell<Option<Shared>>> = Global(RefCell::new(None));

/// Borrow the shared state for the duration of `f`. Don't call engine functions that can call
/// back into the client inside `f`.
pub fn with<R>(f: impl FnOnce(&mut Shared) -> R) -> R {
    let mut s = SHARED.0.borrow_mut();
    let s = s.get_or_insert_with(|| Shared {
        time_scale_target: 1.0,
        time_scale: 1.0,
        fov: 90.0,
        active_items: vec![0; 65],
        laser_color: vec![0; 65],
        ..Default::default()
    });
    f(s)
}
