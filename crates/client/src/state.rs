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
    /// Slow-motion target from `TSSlowMo` (1.0 = normal; `0x1009ff04`). It starts at 0, not 1:
    /// TS keeps it past the initialised part of `.data` (which ends at `0x10099000`) and only the
    /// `TSSlowMo` handler (`0x1004bf10`) writes it, so until the first message `HUD_Frame` eases
    /// the scale toward 0.
    pub time_scale_target: f32,
    /// The last `TSSlowMo` value as sent (the factor × 100, 0 = normal speed), before the
    /// handler maps 0 to 1.0; 0 until the first message. For a ruleset's own slow-motion rules
    /// ([`crate::gameplay::KungfuRegen`], the slow-motion icon).
    pub slowmo_factor100: f32,
    /// The eased scale, `g_flTimescale` (`0x100963e8`, initialised to 1.0).
    pub time_scale: f32,
    /// Local player is dead / spectating.
    pub dead: bool,
    /// The last `TSHealth` value (`0x100cf9d8`, [`crate::hud::state::Hud::last_hp`]): `<= 0` is
    /// `0x10026860`, which forces +score and stops the predicted `ItemPostFrame`.
    pub health: i32,
    /// Current weapon id (TS ids).
    pub weapon: i32,
    /// The eased zoom FOV, `CHud::m_iFOV` (whole degrees; 0 until `CHud::Think` first runs).
    pub fov: f32,

    // ---- added by the render package -------------------------------------------------------
    /// Zoom target (`0x100df1a4`, 0 = `default_fov`): `to.client.fov` from `HUD_PostRunCmd`, or
    /// the `SetFOV` message while `cl_lw` is off. `CHud::Think` reads it through `HUD_GetFOV`
    /// (`hud::get_fov`, which also records and replays it in demos) and eases `fov` toward it.
    pub fov_target: f32,
    /// `TSBTime` bullet-time flag (render hooks `TSBTime`).
    pub bullet_time: bool,
    /// Local power-up bits from `clientdata.vuser1[1]` (1 slow-mo, 2 infinite clip, 8 slow-pause,
    /// 0x10 double fire rate). Written by prediction.
    pub powerups: i32,
    /// `CHudTSWeapons+0x14` (`0x100d0cb4`) as the last HUD draw left it: the local player's
    /// `ActItems` bits (0x40 = akimbo) while a weapon is held, else 0. TS's local laser and
    /// flashlight read this, not the predicted active items (which have no akimbo bit). Written by
    /// `hud::weapons::draw`, read by `effects::laser` in the next frame's transparent pass.
    pub hud_items: i32,
    /// `Spectator` flag per player (index = entindex - 1): not drawn when set. Filled by the HUD.
    pub spectator: [bool; 32],
    /// Local laser dot(s) this frame (second = akimbo) as TriAPI `WorldToScreen` output (−1..1,
    /// y up), for the HUD's `DrawLaserDots`. Written by effects in the transparent pass.
    pub laser_dots: [Option<[f32; 2]>; 2],
    /// Laser range shown next to the crosshair (TS `0x100d0cb8`): distance to the first dot ×0.025.
    pub laser_range: i32,
    /// Active attachment bits while the gun may fire (TS `0x1018a628`: `from` weapon state's
    /// switched-on items, 0 while `m_flNextAttack > 0`). Written by prediction, read by
    /// `DrawLaserDots` to pin the dot to the crosshair.
    pub items_ready: i32,
    /// Predicted local velocity (`from.client.velocity`, TS's `simvel`), for particle react.
    pub local_velocity: [f32; 3],
    /// Spectator globals from `HUD_ProcessPlayerState` (g_iUser1/2/3): mode, target, flags.
    pub spec_mode: i32,
    pub spec_target: i32,
    pub spec_flags: i32,
    // ---- added by the hud package ----
    /// Local player is an observer (`Spectator` message).
    pub observer: bool,
    /// Predicted spread penalty of the current gun (TS `0x1018d188`); the crosshair opens by
    /// `spread × 160` px. Written by `prediction`, read by the HUD.
    pub spread: f32,
    /// `0x1018a718`: the HUD's kung-fu power ([`crate::hud::state::Hud::kfu_power`]) as the TS
    /// weapon element's `Think` (`0x1004cf5e`) last left it, after its regeneration. Written by
    /// the HUD, read by the predicted kung-fu block (`0x1000ae35`). 0 until the first `Think`
    /// (TS keeps it past `.data`).
    pub kfu_power: f32,
    /// Last `TSFade`, if any.
    pub ts_fade: Option<TsFadeState>,
    /// Per-client active attachment bits (`ActItems`, laser/flashlight on; 0x40 = akimbo),
    /// indexed by entity number (0 unused). Read by the laser/flashlight drawing.
    pub active_items: Vec<u8>,
    /// Per-client laser colour index (`PlInfo` key 0).
    pub laser_color: Vec<u8>,
    /// Effect messages waiting for `effects`.
    pub ts_effects: Vec<TsEffect>,
    /// The weapon menu, buy menu or another HUD menu is open (view/effects may hide things).
    pub menu_open: bool,
}

static SHARED: Global<RefCell<Option<Shared>>> = Global(RefCell::new(None));

/// Borrow the shared state for the duration of `f`. Don't call engine functions that can call
/// back into the client inside `f`.
pub fn with<R>(f: impl FnOnce(&mut Shared) -> R) -> R {
    let mut s = SHARED.0.borrow_mut();
    let s = s.get_or_insert_with(|| Shared {
        time_scale_target: 0.0,
        slowmo_factor100: 0.0,
        time_scale: 1.0,
        fov: 90.0,
        active_items: vec![0; 65],
        laser_color: vec![0; 65],
        ..Default::default()
    });
    f(s)
}
