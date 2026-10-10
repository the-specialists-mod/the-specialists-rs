//! What the client learns about weapons: `CTSGun::UpdateClientData` 0x858c8 (`WStatus`,
//! `WeaponInfo`, 10 Hz, on change), `ActItems` and `TSSpace` (`CBasePlayer::UpdateClientData`),
//! `KFuPower` (`UpdateMeleeBar`), and the `clientdata_t` / `weapon_data_t` packing.

use super::SLOTS;
use crate::messages::{self, Msg};
use crate::player::Player;
use crate::world::Cx;
use goldsrc::edict::EdictPtr;
use goldsrc::server::{MsgWriter, engine};
use goldsrc::sys::*;
use shared::protocol::weapons::{self as proto, MsgSink, WStatus, WeaponInfo};
use shared::weapons::{MAX_WEAPONS, info};
use std::ffi::CStr;

/// `MsgSink` over the engine's message writer.
struct Sink(Option<MsgWriter>);

impl Sink {
    fn new(w: MsgWriter) -> Self {
        Sink(Some(w))
    }
    fn end(mut self) {
        if let Some(w) = self.0.take() {
            w.end();
        }
    }
}

impl MsgSink for Sink {
    fn byte(&mut self, v: i32) {
        self.0 = self.0.take().map(|w| w.byte(v));
    }
    fn char(&mut self, v: i32) {
        self.0 = self.0.take().map(|w| w.char(v));
    }
    fn short(&mut self, v: i32) {
        self.0 = self.0.take().map(|w| w.short(v));
    }
    fn string(&mut self, s: &CStr) {
        self.0 = self.0.take().map(|w| w.string(s));
    }
}

fn bot(e: *mut edict_t) -> bool {
    (e.v().flags & FL_FAKECLIENT as i32) != 0
}

pub fn send_kfu_power(to: *mut edict_t, bar: f32) {
    if bot(to) {
        return;
    }
    let mut s = Sink::new(messages::one(Msg::KFuPower, to));
    proto::write_kfu_power(&mut s, bar);
    s.end();
}

pub fn send_act_items(player: *mut edict_t, items: u8) {
    let idx = engine().index_of(player);
    let mut s = Sink::new(messages::all(Msg::ActItems));
    proto::ActItems { entindex: idx as u8, items }.write(&mut s);
    s.end();
}

/// `CurWeapon` 0,0,0 (TS only sends this reset, from `RemoveAllItems`).
pub fn send_cur_weapon_reset(to: *mut edict_t) {
    if bot(to) {
        return;
    }
    let mut s = Sink::new(messages::one(Msg::CurWeapon, to));
    proto::write_cur_weapon_reset(&mut s);
    s.end();
}

fn send_wstatus(to: *mut edict_t, id: u8, owned: bool, clip: i32) {
    let mut s = Sink::new(messages::one(Msg::WStatus, to));
    WStatus { id, owned: owned as u8, clip: clip as u8 }.write(&mut s);
    s.end();
}

/// Resend everything on the next update.
pub fn force_update(p: &mut Player) {
    p.inv.sv.reset_net();
}

/// Called every post-think.
pub fn update_client(p: &mut Player, cx: &mut Cx) {
    let me = cx.edict;
    // ActItems (to everyone) and TSSpace, on change.
    let act = p.inv.gun.active_items() as u8;
    if p.inv.sv.sent_act_items != Some(act) {
        p.inv.sv.sent_act_items = Some(act);
        send_act_items(me, act);
    }
    if bot(me) {
        return;
    }
    let free = p.inv.sv.free_slots;
    if p.inv.sv.sent_space != Some(free) {
        p.inv.sv.sent_space = Some(free);
        let mut s = Sink::new(messages::one(Msg::TsSpace, me));
        proto::write_ts_space(&mut s, free.clamp(0, 255));
        s.end();
    }

    // CTSGun::UpdateClientData, 10 Hz.
    let now = engine().time();
    let sv = &mut p.inv.sv;
    if sv.next_update > now {
        return;
    }
    sv.next_update = now + 0.1;
    let gun = &p.inv.gun;
    for id in (0..MAX_WEAPONS).rev() {
        let st = &gun.status[id];
        if sv.sent_owned[id] != st.owned {
            sv.sent_owned[id] = st.owned;
            send_wstatus(me, id as u8, st.owned, st.clip);
        }
    }
    let cur = gun.current;
    if let Some(st) = gun.slot() {
        let reserve = gun.current_reserve();
        let mut changed = false;
        if sv.sent_id != Some(cur) {
            changed = true;
            sv.sent_id = Some(cur);
            sv.sent_status = *st;
            sv.sent_reserve = reserve;
        }
        if sv.sent_reserve != reserve {
            changed = true;
            sv.sent_status = *st;
            sv.sent_reserve = reserve;
        }
        if changed {
            let m = WeaponInfo {
                id: cur,
                clip: sv.sent_status.clip.clamp(0, 255) as u8,
                reserve: sv.sent_reserve.clamp(-32768, 32767) as i16,
                mode: sv.sent_status.mode,
                items: sv.sent_status.items as u8,
            };
            let mut s = Sink::new(messages::one(Msg::WeaponInfo, me));
            m.write(&mut s);
            s.end();
            if let Some(w) = info(cur) {
                for twin in [w.akimbo, w.single] {
                    if twin != -1 && twin as u8 != cur && gun.status[twin as usize].owned {
                        let t = &gun.status[twin as usize];
                        send_wstatus(me, twin as u8, t.owned, t.clip);
                    }
                }
            }
        }
    } else if sv.sent_id != Some(cur) {
        sv.sent_id = Some(cur);
        let m = WeaponInfo { id: cur, clip: gun.melee_bar.round_ties_even().clamp(0.0, 255.0) as u8, reserve: 0, mode: 0, items: 0 };
        let mut s = Sink::new(messages::one(Msg::WeaponInfo, me));
        m.write(&mut s);
        s.end();
    }
}

/// TS `UpdateClientData` weapon fields + the view model index.
pub fn fill_client_data(p: &mut Player, cx: &mut Cx, cd: &mut clientdata_t) {
    proto::pack_clientdata(&p.inv.gun, p.active_powerup, cd);
    let vm = cx.pev().viewmodel;
    cd.viewmodel = if vm != 0 { engine().model_index(engine().string(vm)) } else { 0 };
    let _ = SLOTS;
}

/// TS `GetWeaponData`: only the TS gun, under HL weapon id 16.
pub fn get_weapon_data(p: &mut Player, _cx: &mut Cx, info: &mut [weapon_data_t; 32]) {
    proto::pack_weapon_data(&p.inv.gun, &mut info[proto::TSGUN_ID as usize]);
}
