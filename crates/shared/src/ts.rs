//! TS 3.0 constants shared by server and client. Values and names come from the decompilation
//! (the-specialists-decomp: docs/server/player-movement.md §2, gamerules-entities.md §4).

/// `pev->iuser4` / `clientdata.iuser4`: the stunt state bitfield. Set by the server's stunt code,
/// read by player movement on both sides.
pub mod stunt {
    pub const RECOVER: i32 = 0x1; // cmd x0.5 while getting up
    pub const DEAD_LOCK: i32 = 0x2; // set on death; PM keeps the cached yaw
    pub const GO_DUCK: i32 = 0x4; // stunt in progress: duck hull, no jump/duck
    pub const STAND_UP: i32 = 0x8;
    pub const DIVING: i32 = 0x10;
    pub const PRONE: i32 = 0x20; // lying after a dive/slide
    pub const DIVE_HIGH: i32 = 0x40; // one-shot impulses consumed by TSDive
    pub const DIVE_LOW: i32 = 0x80;
    pub const DIVE_MED: i32 = 0x100;
    pub const DIVE_IMPULSE_MASK: i32 = 0x1c0;
    /// Stealth shoes (`AddItem` item 0x20, 0x9440c): silent footsteps. The decomp docs call this
    /// `HAS_KEVLAR`, but only the shoes set it; kevlar lives in the loadout items (`pl+0x718`).
    pub const STEALTH_SHOES: i32 = 0x200;
    pub const FLIP_HIGH: i32 = 0x400;
    pub const FLIP_LOW: i32 = 0x800;
    pub const WALLJUMP_IMPULSE: i32 = 0x1000;
    pub const WALLJUMP_MASK: i32 = 0x7000;
    pub const WALLCLIMB_IMPULSE: i32 = 0x8000;
    pub const WALLJUMPING: i32 = 0x10000;
    pub const WALLCLIMBING: i32 = 0x20000;
    pub const ROLLING: i32 = 0x40000;
    pub const FREEZE_ANGLES: i32 = 0x80000;
    pub const SUPERJUMP_GLIDE: i32 = 0x100000;
    pub const SUPERJUMP_START: i32 = 0x200000;
    pub const NO_VIEW: i32 = 0x400000;
    pub const WALLDIVE: i32 = 0x800000;
}

/// Power-up bitmask (`ts_powerup` "pwuptype", `PwUp` message).
pub mod powerup {
    pub const SLOWMO: u16 = 0x001;
    pub const INFINITE_CLIP: u16 = 0x002;
    pub const KUNGFU: u16 = 0x004;
    pub const SLOWPAUSE: u16 = 0x008;
    pub const DOUBLE_FIRERATE: u16 = 0x010;
    pub const GRENADE: u16 = 0x020;
    pub const HEALTH: u16 = 0x040;
    pub const ARMOR: u16 = 0x080;
    pub const SUPERJUMP: u16 = 0x100;
    /// Stored in the player's single slot and activated later with +use.
    pub const STORED: u16 = SLOWMO | SLOWPAUSE | DOUBLE_FIRERATE;
}

/// Observer modes (`pev->iuser1`, client `g_iUser1`). TS 3.0 uses the pre-HLTV-rewrite SDK
/// numbering, not the SDK 2.3 `OBS_*` values: in-eye is **5**, and there are no map modes.
/// Evidence: `Observer_HandleButtons` (0x99d34) cycles 3 → 2 → 1 → 5 → 3, `Observer_SetMode`
/// (0x99e44) accepts only 1, 2, 3 and 5, `PM_SpectatorMove` (0xfef3c) puts mode 5 at the
/// target's origin with its angles, and the client's `V_CalcSpectatorRefdef` (0x10068f70),
/// `HUD_AddEntity` (0x100447f0) and `StudioDrawPlayer` treat 5 as in-eye.
pub mod obs {
    pub const NONE: i32 = 0;
    pub const CHASE_LOCKED: i32 = 1;
    pub const CHASE_FREE: i32 = 2;
    pub const ROAMING: i32 = 3;
    /// Director-driven view of HLTV spectators: the client's `specmode 2` (`CHudSpectator::SetMode`
    /// 0x100234d0) and director camera events set it, and `PM_SpectatorMove` looks for the
    /// secondary target in `iuser3`. The server never sets it for players.
    pub const LOCKED_VIEW: i32 = 4;
    pub const IN_EYE: i32 = 5;
}

/// `usercmd_t::buttons`: TS's stunt key is `+alt1`. `+alt2` sends 0x40 (SDK `IN_CANCEL`), reused
/// by TS for its secondary action.
pub const IN_ALT1: u16 = goldsrc_sys::IN_ALT1 as u16;
pub const IN_ALT2: u16 = 0x40;

/// Time scale: `pev->fuser1` holds every entity's slow-motion factor (1.0 = normal). Player
/// movement multiplies frametime by it and freezes below 0.1. Networked via `clientdata.fuser1`.
pub const TIME_SCALE_FREEZE: f32 = 0.1;

/// Inventory: 81 slots in total; free slots set move speed.
pub const SLOTS_TOTAL: i32 = 81;
