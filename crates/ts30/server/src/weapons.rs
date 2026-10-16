//! TS 3.0's weapon drop rules: the death ammo pack (`CBasePlayer::DropAmmo`,
//! `CTSAmmoPack::CreateAmmoPack`).

use shared::pm::Arith;
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

#[cfg(test)]
mod tests {
    use super::*;
    use ts_server::weapons::worldgun::ammo_pack_origin;

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
