//! TS 3.0's weapon network and drop rules: the death ammo pack (`CBasePlayer::DropAmmo`,
//! `CTSAmmoPack::CreateAmmoPack`) and the `WeaponInfo` half of `CTSGun::UpdateClientData`.

use shared::pm::Arith;
use shared::protocol::weapons::WeaponInfo;
use shared::weapons::Gun;
use ts_server::weapons::SvState;
use ts_server::weapons::net::GunHud;
use ts_server::weapons::worldgun::AmmoPackDrop;

/// `CBasePlayer::DropAmmo` / `CTSAmmoPack::CreateAmmoPack` (`ts_i386.so` 0x946f0 / 0xa1334,
/// mp.dll 0x10062560 / 0x10074c30).
///
/// * The position: `ts_i386.so` keeps `origin + forward·8` on the FPU and adds `right·8` to it
///   (0x94826..0x94891, the products stored exact), so each component is rounded
///   once. mp.dll does that for x (`faddp` at 0x1006266c) but stores y's and z's
///   `origin + forward·8` (`fstps 0x48`/`0x4c`, 0x1006265d, 0x10062668) before adding.
/// * `CreateAmmoPack` sets `pev->owner` to the dying player after the pack's `Spawn` (which
///   cleared it): the pack doesn't collide with the body until `OnGround` (0xa1610) clears it.
pub fn ammo_pack_drop(arith: Arith) -> AmmoPackDrop {
    let stored = match arith {
        Arith::Gcc => [false; 3],
        Arith::Msvc => [false, true, true],
    };
    AmmoPackDrop { stored, owned: true }
}

/// The `WeaponInfo` half of `CTSGun::UpdateClientData` (0x858c8): with a weapon record, the
/// message goes out when the current id (+0xc8) differs from the last one sent (+0x97a) or the
/// reserve from +0x9b4, with the record copied at that moment, followed by the twins' `WStatus`;
/// on kung fu only when the id differs, carrying the kung-fu bar rounded (`ROUND(pl+0x774)`).
/// 3.0 registers `ClipInfo` but never sends it.
pub fn gun_hud_update(sv: &mut SvState, gun: &Gun) -> GunHud {
    weapon_info_update(sv, gun).map_or(GunHud::Nothing, |msg| GunHud::WeaponInfo { msg, twins: true })
}

fn weapon_info_update(sv: &mut SvState, gun: &Gun) -> Option<WeaponInfo> {
    let cur = gun.current;
    let Some(st) = gun.slot() else {
        if sv.sent_id == Some(cur) {
            return None;
        }
        sv.sent_id = Some(cur);
        return Some(WeaponInfo { id: cur, clip: gun.melee_bar.round_ties_even().clamp(0.0, 255.0) as u8, reserve: 0, mode: 0, items: 0 });
    };
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
    changed.then(|| WeaponInfo {
        id: cur,
        clip: sv.sent_status.clip.clamp(0, 255) as u8,
        reserve: sv.sent_reserve.clamp(-32768, 32767) as i16,
        mode: sv.sent_status.mode,
        items: sv.sent_status.items as u8,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::weapons::id;
    use ts_server::weapons::worldgun::ammo_pack_origin;

    #[test]
    fn a_new_gun_on_kung_fu_sends_no_weapon_info() {
        // CTSGun::Reset leaves +0x97a at 0, the kung-fu id: UpdateClientData has nothing to say.
        let mut sv = SvState::default();
        let gun = Gun::default();
        assert_eq!(gun.current, 0);
        assert_eq!(gun_hud_update(&mut sv, &gun), GunHud::Nothing);
        // The port-only `ts_give` reset does send it.
        sv.force_resend();
        let GunHud::WeaponInfo { msg: m, twins: true } = gun_hud_update(&mut sv, &gun) else { panic!("forced") };
        assert_eq!((m.id, m.clip, m.reserve), (0, gun.melee_bar.round_ties_even() as u8, 0));
        assert_eq!(gun_hud_update(&mut sv, &gun), GunHud::Nothing, "sent once");
    }

    /// `ROUND(pl+0x774)`: the kung-fu bar is rounded to nearest, ties to even.
    #[test]
    fn kung_fu_bar_is_rounded() {
        let mut sv = SvState::default();
        let mut gun = Gun { melee_bar: 66.7, ..Gun::default() };
        sv.force_resend();
        let GunHud::WeaponInfo { msg, .. } = gun_hud_update(&mut sv, &gun) else { panic!() };
        assert_eq!(msg.clip, 67);
        gun.melee_bar = 66.5;
        sv.force_resend();
        let GunHud::WeaponInfo { msg, .. } = gun_hud_update(&mut sv, &gun) else { panic!() };
        assert_eq!(msg.clip, 66);
    }

    #[test]
    fn weapon_info_on_a_new_weapon_and_on_a_reserve_change_only() {
        ts30_gameplay::tables::make_default();
        let mut sv = SvState::default();
        let mut gun = Gun::default();
        gun.take_weapon(id::AK47, None, 0).unwrap();
        gun.current = id::AK47;
        let GunHud::WeaponInfo { msg: m, twins: true } = gun_hud_update(&mut sv, &gun) else { panic!("new id") };
        assert_eq!(m.id, id::AK47);
        assert_eq!(gun_hud_update(&mut sv, &gun), GunHud::Nothing, "nothing changed");
        // 3.0 ignores the fire mode, the attachments and the clip.
        let s = &mut gun.status[id::AK47 as usize];
        s.mode ^= 1;
        s.items ^= 1;
        s.clip -= 1;
        assert_eq!(gun_hud_update(&mut sv, &gun), GunHud::Nothing);
        let t = shared::weapons::info(id::AK47).unwrap().ammo as usize;
        gun.reserve[t] += 30;
        let GunHud::WeaponInfo { msg: m, .. } = gun_hud_update(&mut sv, &gun) else { panic!("reserve changed") };
        assert_eq!(m.reserve as i32, gun.current_reserve());
    }

    /// The pack's start: mp.dll rounds y's and z's `origin + forward·8` before adding `right·8`,
    /// `ts_i386.so` none.
    #[test]
    fn ammo_pack_position_per_build() {
        let origin = [1.0, 1.0, 1.0];
        // forward·8 and right·8 are each half an ulp of 1.
        let half = [2f32.powi(-27); 3];
        let msvc = ammo_pack_drop(Arith::Msvc);
        assert!(msvc.owned && ammo_pack_drop(Arith::Gcc).owned);
        let up = 1.0f32 + 2f32.powi(-23);
        assert_eq!(ammo_pack_origin(Arith::Gcc, origin, half, half, ammo_pack_drop(Arith::Gcc).stored), [up; 3]);
        // x keeps the unrounded sum; y's and z's first half ulp is lost to the store (ties to
        // even), and so is the second.
        assert_eq!(ammo_pack_origin(Arith::Msvc, origin, half, half, msvc.stored), [up, 1.0, 1.0]);
    }
}
