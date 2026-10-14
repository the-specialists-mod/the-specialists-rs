//! Keeps the client HUD in sync: `CBasePlayer::UpdateClientData` (0x91d00), which compares
//! against "last sent" copies and runs at most every 0.1 s, the InitHUD block included. Owner:
//! rules work package. Reference: decomp docs/server/architecture.md §3.2,
//! client/architecture-hud.md §3.
//!
//! TS runs it from `PreThink` (between `WaterMove` and `CheckTimeBasedDamage`, not in
//! intermission), and inline from `ForceClientDllUpdate` (`fullupdate`, throttle reset first) and
//! `RemoveAllItems` (`StartObserver`, a disconnect; throttled as usual).

use super::Player;
use crate::messages::Msg;
use crate::rules::{msg, powerup};
use crate::world::Cx;
use goldsrc::sys::*;
use shared::protocol::hud_wire::{ActItems, PwUp, TsArmor, TsBTime, TsCash, TsHealth, TsPAward, TsSlowMo, TsSpace, TsState};
use shared::ts::stunt;

/// The "last sent" copies, with TS's reset values (`Spawn` 0x8f568 / `TSInit` 0x92a4c).
pub struct HudCache {
    /// `+0x534`: the next time `UpdateClientData` may run; negative disables it for good
    /// (`TSPlayerDisconnect`).
    pub next_update: f32,
    /// `m_iClientHealth` (`+0x9e0`): `(int)health` as last sent, `None` = -1.
    pub health: Option<i32>,
    /// `m_iClientBattery` (`+0x9e4`): `(int)armorvalue` as last sent, -1 = resend.
    pub armor: i32,
    /// `+0x499`: last `TSState`, 0xff = resend.
    pub state: u8,
    /// `+0x544`: last `TSCash`, `None` = -1.
    pub cash: Option<i32>,
    /// `+0x54c`: last `TSSpace` (free slots), -1 = resend.
    pub space: i32,
    /// `+0x738` / `+0x734`: last `PwUp` bits (0xff = resend) and seconds.
    pub pwup_bits: i32,
    pub pwup_secs: i32,
    /// `+0x16c`: last `TSSlowMo` factor, -1 = resend.
    pub slowmo: f32,
    /// `+0x174`: last `TSBTime`, 0xff = resend.
    pub btime: u8,
    /// `+0x720`: last `ActItems` mask; reset to 0, so a 0 mask isn't sent again.
    pub act_items: u8,
    /// `+0x74d`: last `TSPAward`, 0xff = resend.
    pub award: u8,
}

impl Default for HudCache {
    fn default() -> Self {
        HudCache {
            next_update: 0.0,
            health: None,
            armor: -1,
            state: 0xff,
            cash: None,
            space: -1,
            pwup_bits: 0xff,
            pwup_secs: 0,
            slowmo: -1.0,
            btime: 0xff,
            act_items: 0,
            award: 0xff,
        }
    }
}

/// The resets of a spawn (`Spawn` + `TSInit`): every value is sent again on the next update.
pub fn reset(p: &mut Player) {
    p.hud = HudCache::default();
}

/// The resets of `ForceClientDllUpdate` (0x92594): everything but `TSPAward`, and the throttle.
pub fn force_reset(p: &mut Player) {
    let award = p.hud.award;
    p.hud = HudCache { award, ..HudCache::default() };
}

/// `UpdatePlayerState` (0x92a08): 2 prone/stunt (`iuser4 & 0x30`), 1 ducked, 0 standing.
pub fn stance(v: &entvars_t) -> u8 {
    if v.iuser4 & (stunt::DIVING | stunt::PRONE) != 0 {
        2
    } else if v.flags & FL_DUCKING as i32 != 0 {
        1
    } else {
        0
    }
}

/// `fistp` with the truncating control word TS loads around these conversions (0x91fd2,
/// 0x9202a, 0x92074, 0x920c6): out of range or NaN gives the x87 "integer indefinite".
fn fist_trunc(x: f32) -> i32 {
    if x.is_finite() && x > -2_147_483_649.0 && x < 2_147_483_648.0 { x as i32 } else { i32::MIN }
}

/// `fildl last; fcomps value`: the value is resent unless it compares equal. An unordered
/// compare (NaN) sets ZF too, so NaN counts as unchanged.
fn changed(last: i32, value: f32) -> bool {
    !value.is_nan() && last as f64 != value as f64
}

/// `TSHealth` byte: `(int)max(health, 0)`, truncated, and `WRITE_BYTE` keeps the low 8 bits, so
/// there is no cap at 255.
fn health_byte(health: f32) -> u8 {
    fist_trunc(if health < 0.0 { 0.0 } else { health }) as u8
}

/// `TSArmor` byte: `(int)armorvalue`, truncated, no clamp either side.
fn armor_byte(armor: f32) -> u8 {
    fist_trunc(armor) as u8
}

/// `PwUp` seconds: `secs & ~(secs >> 31)` (negative → 0), then the low byte.
fn pwup_secs_byte(secs: i32) -> u8 {
    (secs & !(secs >> 31)) as u8
}

/// `CBasePlayer::UpdateClientData` (0x91d00). Runs when `next_update <= time` and isn't
/// negative, then waits 0.1 s. The InitHUD block (`m_fInitHUD`) is inside the throttle: ResetHUD
/// every spawn and observer start, InitHUD and both rules InitHUDs once. Then, on change:
/// HideWeapon, TSHealth, TSArmor, Train, TSState, TSCash, TSSpace, PwUp, TSSlowMo, TSBTime,
/// ActItems (alive only), TSPAward, and the gun's own (also 10 Hz) `UpdateClientData`.
pub fn update_client_data(p: &mut Player, cx: &mut Cx) {
    let now = cx.time();
    if !(p.hud.next_update <= now && p.hud.next_update >= 0.0) {
        return;
    }
    p.hud.next_update = shared::pm::plus_double(now, 0.1);
    if p.ts.init_hud {
        crate::rules::lifecycle::init_hud_block(p, cx);
    }
    let e = cx.edict;
    crate::entities::stock::drive::update_client_hide_weapon(p, cx);
    let (health, armor, alive) = {
        let v = cx.pev();
        (v.health, v.armorvalue, crate::rules::alive(v))
    };
    let state = p.stunt.stance;
    if changed(p.hud.health.unwrap_or(-1), health) {
        let hp = health_byte(health);
        msg::one(Msg::TsHealth, e, |w| TsHealth { hp }.write(w));
        p.hud.health = Some(fist_trunc(health));
    }
    if changed(p.hud.armor, armor) {
        p.hud.armor = fist_trunc(armor);
        let armor = armor_byte(armor);
        msg::one(Msg::TsArmor, e, |w| TsArmor { armor }.write(w));
    }
    crate::entities::stock::drive::update_client_train(p, cx);
    if p.hud.state != state {
        p.hud.state = state;
        msg::one(Msg::TsState, e, |w| TsState { state }.write(w));
    }
    if p.hud.cash != Some(p.cash) {
        p.hud.cash = Some(p.cash);
        let cash = p.cash as i16;
        msg::one(Msg::TsCash, e, |w| TsCash { cash }.write(w));
    }
    // `+0x548`, the free inventory weight.
    let space = p.inv.sv.free_slots;
    if p.hud.space != space {
        p.hud.space = space;
        msg::one(Msg::TsSpace, e, |w| TsSpace { space: space as u8 }.write(w));
    }
    let (bits, secs) = (p.ts.held as i32, p.ts.held_secs);
    if p.hud.pwup_bits != bits || p.hud.pwup_secs != secs {
        p.hud.pwup_bits = bits;
        p.hud.pwup_secs = secs;
        let m = PwUp { bits: bits as i16, secs: pwup_secs_byte(secs) };
        msg::one(Msg::PwUp, e, |w| m.write(w));
    }
    let slow = powerup::hud_slow_amount(p, crate::gameplay::active().server.powerups());
    if slow != p.hud.slowmo {
        p.hud.slowmo = slow;
        msg::one(Msg::TsSlowMo, e, |w| TsSlowMo { factor_x100: slow * 100.0 }.write(w));
    }
    let btime = p.ts.btime as u8;
    if p.hud.btime != btime {
        p.hud.btime = btime;
        msg::one(Msg::TsBTime, e, |w| TsBTime { on: btime }.write(w));
    }
    // `m_pActiveItem` is the TS gun and the player is alive.
    let has_gun = crate::weapons::gun_active(p);
    if has_gun && alive {
        let mask = p.inv.gun.active_items() as u8;
        if p.hud.act_items != mask {
            p.hud.act_items = mask;
            let m = ActItems { entindex: cx.index() as u8, mask };
            msg::all(Msg::ActItems, |w| m.write(w));
        }
    }
    if p.hud.award != p.ts.award {
        p.hud.award = p.ts.award;
        let m = TsPAward { entindex: cx.index() as u8, award: p.ts.award };
        msg::all(Msg::TsPAward, |w| m.write(w));
    }
    // `m_pActiveItem->UpdateClientData(this)`: `CTSGun::UpdateClientData` (0x858c8), only while
    // the gun is the active item.
    if has_gun {
        crate::weapons::update_client_data(p, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_player_state_stance() {
        let mut v = entvars_t::default();
        assert_eq!(stance(&v), 0);
        v.flags = FL_DUCKING as i32;
        assert_eq!(stance(&v), 1);
        v.iuser4 = stunt::DIVING;
        assert_eq!(stance(&v), 2, "a dive or prone wins over ducking");
        v.iuser4 = stunt::PRONE;
        v.flags = 0;
        assert_eq!(stance(&v), 2);
    }

    /// Issue #48: `TSState` goes out only from `UpdateClientData`, never from the stunt code.
    #[test]
    fn tsstate_is_sent_only_from_update_client_data() {
        let src = include_str!("../stunt/mod.rs");
        assert!(!src.contains(concat!("Msg::", "TsState")), "stunt::pre_think must not send TSState");
    }

    #[test]
    fn health_and_armor_bytes_truncate_like_ts() {
        // The One's regen leaves fractional health; TS shows the floor, not the rounded value.
        assert_eq!(health_byte(99.84), 99);
        assert_eq!(health_byte(0.9), 0);
        assert_eq!(health_byte(-12.0), 0);
        // WRITE_BYTE of an int keeps the low byte: no cap at 255.
        assert_eq!(health_byte(300.0), 44);
        assert_eq!(armor_byte(49.99), 49);
        assert_eq!(armor_byte(256.0), 0);
        assert_eq!(fist_trunc(f32::NAN), i32::MIN);
        assert_eq!(fist_trunc(3e9), i32::MIN);
        assert_eq!(fist_trunc(-7.9), -7);
    }

    #[test]
    fn fractional_health_is_resent_every_update() {
        // TS stores `(int)health` and compares it with the float, so 99.84 never matches 99.
        let stored = fist_trunc(99.84);
        assert!(changed(stored, 99.84));
        assert!(!changed(100, 100.0));
        // -1 is the reset value: a health of exactly -1 isn't sent again.
        assert!(!changed(-1, -1.0));
        assert!(!changed(5, f32::NAN));
    }

    #[test]
    fn pwup_seconds_byte() {
        assert_eq!(pwup_secs_byte(-3), 0);
        assert_eq!(pwup_secs_byte(5), 5);
        assert_eq!(pwup_secs_byte(260), 4);
    }

    #[test]
    fn force_reset_keeps_the_award() {
        let hud = HudCache { award: 11, next_update: 5.0, health: Some(100), cash: Some(3), act_items: 0x40, ..HudCache::default() };
        let mut p = Player { hud, ..Player::default() };
        force_reset(&mut p);
        assert_eq!(p.hud.award, 11);
        assert_eq!(p.hud.next_update, 0.0);
        assert_eq!(p.hud.health, None);
        assert_eq!(p.hud.act_items, 0);
        assert_eq!(p.hud.state, 0xff);
        reset(&mut p);
        assert_eq!(p.hud.award, 0xff);
    }
}
