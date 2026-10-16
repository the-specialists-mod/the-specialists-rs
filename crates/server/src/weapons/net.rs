//! What the client learns about weapons: `CTSGun::UpdateClientData` 0x858c8 (`WStatus`,
//! `WeaponInfo`, 10 Hz, on change), `KFuPower` (`UpdateMeleeBar`), and the `clientdata_t` /
//! `weapon_data_t` packing. `ActItems` and `TSSpace` are `CBasePlayer::UpdateClientData`'s
//! (`player::hud`).

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

/// Port addition (`ts_give`): resend everything on the next update.
pub fn force_update(p: &mut Player) {
    p.inv.sv.force_resend();
}

/// What `CTSGun::UpdateClientData` sends after its `WStatus` loop
/// ([`ServerRules::gun_hud_update`](crate::gameplay::ServerRules::gun_hud_update)).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GunHud {
    /// Nothing changed.
    Nothing,
    /// `WeaponInfo`; with `twins`, a `WStatus` for each owned akimbo or single twin of the
    /// current weapon follows.
    WeaponInfo { msg: WeaponInfo, twins: bool },
    /// `ClipInfo` with the current clip.
    ClipInfo(u8),
}

/// `CTSGun::UpdateClientData` (0x858c8), from the end of `CBasePlayer::UpdateClientData`
/// (`player::hud::update_client_data`): `WStatus` on change, then what the ruleset's
/// [`GunHud`] says, 10 Hz on the gun's own timer. `ActItems` and `TSSpace` are the player's
/// part, sent from there.
pub fn update_client(p: &mut Player, cx: &mut Cx) {
    let me = cx.edict;
    if bot(me) {
        return;
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
    match crate::gameplay::active().server.gun_hud_update(sv, gun) {
        GunHud::Nothing => {}
        GunHud::ClipInfo(clip) => {
            let mut s = Sink::new(messages::one(Msg::ClipInfo, me));
            s.byte(clip as i32);
            s.end();
        }
        GunHud::WeaponInfo { msg, twins } => {
            let mut s = Sink::new(messages::one(Msg::WeaponInfo, me));
            msg.write(&mut s);
            s.end();
            let cur = gun.current;
            if twins
                && gun.slot().is_some()
                && let Some(w) = info(cur)
            {
                for twin in [w.akimbo, w.single] {
                    if twin != -1 && twin as u8 != cur && gun.status[twin as usize].owned {
                        let t = &gun.status[twin as usize];
                        send_wstatus(me, twin as u8, t.owned, t.clip);
                    }
                }
            }
        }
    }
}

/// TS `UpdateClientData` weapon fields + the view model index.
pub fn fill_client_data(p: &mut Player, cx: &mut Cx, cd: &mut clientdata_t) {
    if super::gun_active(p) {
        proto::pack_clientdata(&p.inv.gun, p.active_powerup, cd);
    } else {
        pack_clientdata_no_gun(p.inv.gun.next_attack, p.active_powerup, cd);
    }
    let vm = cx.pev().viewmodel;
    cd.viewmodel = if vm != 0 { engine().model_index(engine().string(vm)) } else { 0 };
    let _ = SLOTS;
}

/// `UpdateClientData` (0xb0294) for a player without the gun: `m_flNextAttack`, the power-ups,
/// the ammo fields and weapon id 0 (`vuser3[0]`), then it returns before the weapon state;
/// `m_iId` is left as the engine passed it.
fn pack_clientdata_no_gun(next_attack: f32, powerups: u16, cd: &mut clientdata_t) {
    cd.m_flNextAttack = next_attack;
    cd.vuser1[1] = powerups as f32;
    cd.ammo_shells = 0;
    cd.ammo_rockets = 0;
    cd.ammo_cells = 0;
    cd.vuser3[0] = 0.0;
}

/// TS `GetWeaponData`: only the TS gun, under HL weapon id 16; all zero without it.
pub fn get_weapon_data(p: &mut Player, _cx: &mut Cx, info: &mut [weapon_data_t; 32]) {
    if super::gun_active(p) {
        proto::pack_weapon_data(&p.inv.gun, &mut info[proto::TSGUN_ID as usize]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clientdata_without_the_gun_stops_after_the_weapon_id() {
        let mut cd = clientdata_t { m_iId: 7, ..Default::default() };
        cd.vuser3 = [5.0, 6.0, 7.0];
        cd.vuser4[1] = 9.0;
        cd.ammo_shells = 3;
        pack_clientdata_no_gun(0.25, 0x10, &mut cd);
        assert_eq!((cd.m_flNextAttack, cd.vuser1[1], cd.ammo_shells), (0.25, 16.0, 0));
        assert_eq!(cd.vuser3, [0.0, 6.0, 7.0], "only the weapon id is cleared");
        assert_eq!((cd.m_iId, cd.vuser4[1]), (7, 9.0), "the weapon fields are left alone");
    }
}
