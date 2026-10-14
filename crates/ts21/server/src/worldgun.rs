//! TS 2.1's dropped guns and ammo packs (`CTSWorldGun`, `CTSAmmoPack`, 2.1 `mp.dll`): what a
//! flying gun hurts, and when a pack goes away. The rest is TS 3.0's
//! (`ts_server::weapons::worldgun`).

use goldsrc::edict::EdictPtr;
use goldsrc::server::engine;
use goldsrc::sys::edict_t;
use shared::weapons::info;
use ts_server::damage::{self, DamageInfo};

/// `DMG_CLUB`, the bits `BounceTouch` passes (0x100654fc: `push 0x80`).
const DMG_CLUB: i32 = 0x80;

/// The damage `BounceTouch` deals (0x1006551f: `push 0x42480000`).
pub const TOUCH_DAMAGE: f32 = 50.0;

/// `CTSWorldGun::BounceTouch` (0x10065270)'s test: the touched entity's classname is exactly
/// `func_breakable` (`repz cmpsb` over 15 bytes) and `(float)(slots + 50) >= health`
/// (`fild; fcomp health; and ah, 1`: a NaN health fails).
pub fn touch_breaks(class: &[u8], slots: i32, health: f32) -> bool {
    class == b"func_breakable" && (slots + 50) as f32 >= health
}

/// The end of `CTSWorldGun::BounceTouch` (0x1006549b..0x10065534), on every touch while the gun
/// flies, at any speed: `ClearMultiDamage`, `other->TraceAttack(gun, 50, gun velocity,
/// &gGlobalTrace, DMG_CLUB)`, `ApplyMultiDamage(gun, gun)`. 2.1's `Fall` (0x10065550) has no
/// impact trace. The attacker is the thrower, as in the port's 3.0 `Fall` (port deviation,
/// issue #1).
pub fn touch(gun: *mut edict_t, attacker: *mut edict_t, id: u8, other: *mut edict_t) {
    if other.is_null() || other.is_free() {
        return;
    }
    let e = engine();
    let slots = info(id).map_or(0, |w| w.slots);
    let (class, health) = {
        let v = other.v();
        (e.string(v.classname).to_bytes().to_vec(), v.health)
    };
    if !touch_breaks(&class, slots, health) {
        return;
    }
    let gt = e.globals();
    let mut d = DamageInfo::new(gun, attacker, TOUCH_DAMAGE, DMG_CLUB).traced();
    d.hitgroup = gt.trace_hitgroup;
    d.dir = gun.v().velocity;
    d.pos = gt.trace_endpos;
    damage::apply(other, &d);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Up to `slots + 50` health breaks: 60 for a pistol, 120 for the Barrett; nothing else is
    /// hurt, and a NaN health isn't.
    #[test]
    fn breakables_up_to_slots_plus_50() {
        assert!(touch_breaks(b"func_breakable", 10, 60.0));
        assert!(!touch_breaks(b"func_breakable", 10, 60.5));
        assert!(touch_breaks(b"func_breakable", 70, 120.0));
        assert!(touch_breaks(b"func_breakable", 10, -5.0));
        assert!(!touch_breaks(b"func_breakable", 10, f32::NAN));
        assert!(!touch_breaks(b"func_pushable", 10, 1.0));
        assert!(!touch_breaks(b"func_breakable2", 10, 1.0));
        assert!(!touch_breaks(b"func_door", 10, 1.0));
    }
}
