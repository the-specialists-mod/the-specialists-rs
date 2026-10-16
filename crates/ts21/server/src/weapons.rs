//! TS 2.1's `WeaponInfo` / `ClipInfo` half of `CTSGun::UpdateClientData` (mp.dll 0x1004d8c0),
//! on TS 3.0's wire.

use shared::protocol::weapons::WeaponInfo;
use shared::weapons::Gun;
use ts_server::weapons::SvState;
use ts_server::weapons::net::GunHud;

/// `CTSGun::UpdateClientData` (0x1004d8c0) after its `WStatus` loop:
///
/// * Kung fu (no record, `+0xbc` NULL): `WeaponInfo` when the id differs from the last one sent
///   (`+0xd16`), the clip byte being the kung-fu bar truncated (`flds [pl+0x6ec]; __ftol`,
///   0x1004d9ca..0x1004d9d0).
/// * A weapon: the record is copied (`+0xd18`, the reserve to `+0xd48`) when the id or the
///   reserve changed, and also when the fire mode (`record+2` vs `+0xd1a`) or the attachments
///   (`record+0x10` vs `+0xd28`) did; any of the four sends `WeaponInfo` from the copy. No twin
///   `WStatus` follows (3.0 resends the akimbo/single twins).
/// * Otherwise `ClipInfo(clip)` when the record's clip differs from the copy's (`+0xd1c`), which
///   then takes it: the HUD's clip follows every shot and reload at 10 Hz.
///
/// The bytes are `WriteByte`/`WriteShort` of `int`s: the low bits.
pub fn gun_hud_update(sv: &mut SvState, gun: &Gun) -> GunHud {
    let cur = gun.current;
    let Some(st) = gun.slot() else {
        if sv.sent_id == Some(cur) {
            return GunHud::Nothing;
        }
        sv.sent_id = Some(cur);
        let msg = WeaponInfo { id: cur, clip: ftol(gun.melee_bar) as u8, reserve: 0, mode: 0, items: 0 };
        return GunHud::WeaponInfo { msg, twins: false };
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
    if sv.sent_status.mode != st.mode || sv.sent_status.items != st.items {
        changed = true;
        sv.sent_status = *st;
    }
    if changed {
        let c = &sv.sent_status;
        let msg = WeaponInfo { id: cur, clip: c.clip as u8, reserve: sv.sent_reserve as i16, mode: c.mode, items: c.items as u8 };
        return GunHud::WeaponInfo { msg, twins: false };
    }
    if sv.sent_status.clip != st.clip {
        sv.sent_status.clip = st.clip;
        return GunHud::ClipInfo(st.clip as u8);
    }
    GunHud::Nothing
}

/// MSVC's `__ftol`: truncation to a 64-bit integer (a NaN or out-of-range value is
/// 0x8000000000000000), of which the caller keeps the low bits.
fn ftol(v: f32) -> i64 {
    if v.is_nan() || v.abs() >= 9.223_372e18 { i64::MIN } else { v as i64 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::weapons::id;

    fn weapon_info(h: GunHud) -> WeaponInfo {
        match h {
            GunHud::WeaponInfo { msg, twins: false } => msg,
            other => panic!("{other:?}"),
        }
    }

    /// The kung-fu bar is truncated: 66.7 goes out as 66 (3.0 rounds it to 67).
    #[test]
    fn kung_fu_bar_is_truncated() {
        let mut sv = SvState::default();
        let gun = Gun { melee_bar: 66.7, ..Gun::default() };
        sv.force_resend();
        assert_eq!(weapon_info(gun_hud_update(&mut sv, &gun)).clip, 66);
        let mut sv3 = SvState::default();
        sv3.force_resend();
        let GunHud::WeaponInfo { msg, .. } = ts30_server::weapons::gun_hud_update(&mut sv3, &gun) else { panic!() };
        assert_eq!(msg.clip, 67);
        assert_eq!(ftol(-3.5) as u8, 253);
        assert_eq!(ftol(f32::NAN) as u8, 0);
    }

    /// A fire-mode or attachment change resends `WeaponInfo`; a clip change alone sends
    /// `ClipInfo`, once.
    #[test]
    fn mode_attachments_and_clip() {
        ts30_gameplay::tables::make_default();
        let mut sv = SvState::default();
        let mut gun = Gun::default();
        gun.take_weapon(id::AK47, None, 0).unwrap();
        gun.current = id::AK47;
        let m = weapon_info(gun_hud_update(&mut sv, &gun));
        assert_eq!(m.id, id::AK47);
        assert_eq!(gun_hud_update(&mut sv, &gun), GunHud::Nothing);

        gun.status[id::AK47 as usize].mode ^= 1;
        let m = weapon_info(gun_hud_update(&mut sv, &gun));
        assert_eq!(m.mode, gun.status[id::AK47 as usize].mode);
        assert_eq!(gun_hud_update(&mut sv, &gun), GunHud::Nothing);

        gun.status[id::AK47 as usize].items ^= 4;
        let m = weapon_info(gun_hud_update(&mut sv, &gun));
        assert_eq!(m.items as u32, gun.status[id::AK47 as usize].items);

        let clip = gun.status[id::AK47 as usize].clip - 1;
        gun.status[id::AK47 as usize].clip = clip;
        assert_eq!(gun_hud_update(&mut sv, &gun), GunHud::ClipInfo(clip as u8));
        assert_eq!(gun_hud_update(&mut sv, &gun), GunHud::Nothing, "the copy took the clip");

        // A mode change and a shot in the same tick: `WeaponInfo` carries the new clip, no
        // `ClipInfo`.
        gun.status[id::AK47 as usize].mode ^= 1;
        gun.status[id::AK47 as usize].clip = clip - 1;
        assert_eq!(weapon_info(gun_hud_update(&mut sv, &gun)).clip as i32, clip - 1);
        assert_eq!(gun_hud_update(&mut sv, &gun), GunHud::Nothing);
    }
}
