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

pub fn update(p: &mut Player, cx: &mut Cx) {
    let now = cx.time();
    if p.hud.next_update > now && p.hud.next_update - now <= 0.1 {
        return;
    }
    p.hud.next_update = now + 0.1;
    let e = cx.edict;
    let (health, armor, state) = {
        let v = cx.pev();
        (v.health.max(0.0).round().min(255.0) as u8, v.armorvalue.max(0.0).round().min(255.0) as u8, stance(v))
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
