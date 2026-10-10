//! Keeps the client HUD in sync: sends TS HUD messages when values change. Owner: rules work
//! package. Reference: decomp docs/server/architecture.md §3.2, client/architecture-hud.md §3;
//! `CBasePlayer::UpdateClientData` (0x91d00), which compares against "last sent" copies and runs
//! at most every 0.1 s.

use super::Player;
use crate::messages::Msg;
use crate::rules::{msg, powerup};
use crate::world::Cx;
use goldsrc::sys::*;
use shared::protocol::hud_wire::{PwUp, TsArmor, TsBTime, TsCash, TsHealth, TsPAward, TsSlowMo, TsSpace, TsState};
use shared::ts::stunt;

/// Last values sent (`None` = send on the next update).
#[derive(Default)]
pub struct HudCache {
    pub next_update: f32,
    pub health: Option<u8>,
    pub armor: Option<u8>,
    pub state: Option<u8>,
    pub cash: Option<i32>,
    pub space: Option<i32>,
    pub pwup: Option<(u16, i32)>,
    pub slowmo: Option<f32>,
    pub btime: Option<bool>,
    pub award: Option<u8>,
}

pub fn reset(p: &mut Player) {
    p.hud = HudCache::default();
}

/// `UpdatePlayerState`: 2 prone/stunt (`iuser4 & 0x30`), 1 ducked, 0 standing.
pub fn stance(v: &entvars_t) -> u8 {
    if v.iuser4 & (stunt::DIVING | stunt::PRONE) != 0 {
        2
    } else if v.flags & FL_DUCKING as i32 != 0 {
        1
    } else {
        0
    }
}

/// `TSHealth` byte: `UpdateClientData` (0x91d00) sends `(int)max(health, 0)`, truncated toward
/// zero by the `fistp` with the rounding mode forced to chop (0x91fd2), and `WRITE_BYTE` keeps
/// the low 8 bits, so there is no cap at 255.
fn health_byte(health: f32) -> u8 {
    health.max(0.0) as i32 as u8
}

/// `TSArmor` byte: `(int)armorvalue`, truncated (0x9206e / 0x920c0), no clamp either side.
fn armor_byte(armor: f32) -> u8 {
    armor as i32 as u8
}

pub fn update(p: &mut Player, cx: &mut Cx) {
    let now = cx.time();
    if p.hud.next_update > now && p.hud.next_update - now <= 0.1 {
        return;
    }
    p.hud.next_update = now + 0.1;
    let e = cx.edict;
    let (health, armor, state) = {
        let v = cx.pev();
        (health_byte(v.health), armor_byte(v.armorvalue), stance(v))
    };
    if p.hud.health != Some(health) {
        p.hud.health = Some(health);
        msg::one(Msg::TsHealth, e, |w| TsHealth { hp: health }.write(w));
    }
    if p.hud.armor != Some(armor) {
        p.hud.armor = Some(armor);
        msg::one(Msg::TsArmor, e, |w| TsArmor { armor }.write(w));
    }
    if p.hud.state != Some(state) {
        p.hud.state = Some(state);
        msg::one(Msg::TsState, e, |w| TsState { state }.write(w));
    }
    if p.hud.cash != Some(p.cash) {
        p.hud.cash = Some(p.cash);
        let cash = p.cash.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        msg::one(Msg::TsCash, e, |w| TsCash { cash }.write(w));
    }
    let space = shared::ts::SLOTS_TOTAL - p.slots_used;
    if p.hud.space != Some(space) {
        p.hud.space = Some(space);
        msg::one(Msg::TsSpace, e, |w| TsSpace { space: space.clamp(0, 255) as u8 }.write(w));
    }
    let pw = (p.ts.held, p.ts.held_secs);
    if p.hud.pwup != Some(pw) {
        p.hud.pwup = Some(pw);
        let m = PwUp { bits: pw.0 as i16, secs: pw.1.clamp(0, 255) as u8 };
        msg::one(Msg::PwUp, e, |w| m.write(w));
    }
    let slow = powerup::hud_slow_amount(p);
    if p.hud.slowmo != Some(slow) {
        p.hud.slowmo = Some(slow);
        msg::one(Msg::TsSlowMo, e, |w| TsSlowMo { factor_x100: slow * 100.0 }.write(w));
    }
    if p.hud.btime != Some(p.ts.btime) {
        p.hud.btime = Some(p.ts.btime);
        msg::one(Msg::TsBTime, e, |w| TsBTime { on: p.ts.btime as u8 }.write(w));
    }
    crate::entities::stock::drive::update_client(p, cx);
    if p.hud.award != Some(p.ts.award) {
        p.hud.award = Some(p.ts.award);
        let m = TsPAward { entindex: cx.index() as u8, award: p.ts.award };
        msg::all(Msg::TsPAward, |w| m.write(w));
    }
}

#[cfg(test)]
mod tests {
    use super::{armor_byte, health_byte};

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
    }
}
