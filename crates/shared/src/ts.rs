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
    pub const HAS_KEVLAR: i32 = 0x200;
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

/// `usercmd_t::buttons`: TS's stunt key is `+alt1`. `+alt2` sends 0x40 (SDK `IN_CANCEL`), reused
/// by TS for its secondary action.
pub const IN_ALT1: u16 = goldsrc_sys::IN_ALT1 as u16;
pub const IN_ALT2: u16 = 0x40;

/// Time scale: `pev->fuser1` holds every entity's slow-motion factor (1.0 = normal). Player
/// movement multiplies frametime by it and freezes below 0.1. Networked via `clientdata.fuser1`.
pub const TIME_SCALE_FREEZE: f32 = 0.1;

/// Inventory: 81 slots in total; free slots set move speed.
pub const SLOTS_TOTAL: i32 = 81;

/// Move speed from free inventory slots: `210 + free * 120 / 81`, capped at 330.
pub fn speed_for_free_slots(free: i32) -> f32 {
    (210.0 + free.max(0) as f32 * 120.0 / SLOTS_TOTAL as f32).min(330.0)
}

#[cfg(test)]
mod tests {
    #[test]
    fn slot_speed() {
        assert_eq!(super::speed_for_free_slots(0), 210.0);
        assert_eq!(super::speed_for_free_slots(81), 330.0);
    }
}
