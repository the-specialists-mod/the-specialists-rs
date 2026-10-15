//! TS 2.1's movement and gun rules, which the server and the client's prediction both run
//! (issue #100 §2): movement and the move speed in [`pm`]; the gun (§2.3) is [`gun`] with its
//! tables in [`tables`]; the melee attacks (§3.2) are [`melee`].

pub mod gun;
pub mod melee;
pub mod pm;
pub mod tables;

use crate::Ts21;
use ::shared::gameplay::MeleeRules;
use ::shared::weapons::{Gun, GunHost};

/// Every melee attack runs 2.1's own function ([`melee`]).
impl MeleeRules for Ts21 {
    fn knife_fire(&self, gun: &mut Gun, host: &mut dyn GunHost) {
        melee::knife_fire(gun, host);
    }

    fn katana_fire(&self, gun: &mut Gun, host: &mut dyn GunHost) {
        melee::katana_fire(gun, host);
    }

    fn ccombat_attack(&self, gun: &mut Gun, host: &mut dyn GunHost) {
        melee::ccombat_attack(gun, host);
    }

    fn ccombat_block(&self, gun: &mut Gun, host: &mut dyn GunHost) {
        melee::ccombat_block(gun, host);
    }

    fn cold_cock(&self, gun: &mut Gun, host: &mut dyn GunHost) {
        melee::cold_cock(gun, host);
    }
}
