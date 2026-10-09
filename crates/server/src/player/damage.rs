//! Player damage, armour, death. Owner: rules work package.
//! Reference: decomp docs/server/player-movement.md §6, weapons.md §5.9.

use super::Player;
use crate::world::{Cx, DamageInfo};

/// Placeholder: subtracts health and reports death to the rules.
pub fn take_damage(p: &mut Player, cx: &mut Cx, dmg: &DamageInfo) -> bool {
    let pev = cx.pev();
    if pev.takedamage == 0.0 || pev.health <= 0.0 {
        return false;
    }
    pev.health -= dmg.damage;
    if pev.health <= 0.0 {
        crate::rules::on_killed(p, cx, dmg);
    }
    true
}
