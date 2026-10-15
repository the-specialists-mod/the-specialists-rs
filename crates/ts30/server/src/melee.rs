//! TS 3.0's melee rules on the victim and the stunt kicks: the blocks (`VictimKungfuHold`
//! 0x76f50, `VictimKnifeHold` 0x76de4, `VictimKatanaHold` 0x76c8c), the jump kick's hit and the
//! backflip kick.

use goldsrc::edict::EdictPtr;
use goldsrc::sys::{CHAN_BODY, edict_t};
use shared::weapons::gun::{MeleeKind, MeleeResult};
use shared::weapons::spread;
use ts_server::player::Player;
use ts_server::weapons::{send_kfu_power, sparks};
use ts_server::world::Cx;

/// The melee sounds' `EMIT_SOUND_DYN` (`CHAN_BODY`, volume 1, attenuation 0.8, the weapons'
/// slow-motion factor).
fn slow_emit(e: *mut edict_t, sample: &std::ffi::CStr, pitch: i32) {
    ts_server::weapons::emit(e, CHAN_BODY, sample, 1.0, 0.8, pitch);
}

/// Applies a block by the victim. Returns what the attacker gets, or `None` if not blocked.
/// Victim-side effects (its next attack, its bar) are applied here.
/// `atk_block` is the attacker's own kung-fu block window (the attacker is the player being
/// dispatched, so the caller passes it; victims are other players, reachable with world::with).
pub fn hold(kind: MeleeKind, attacker: *mut edict_t, victim: *mut edict_t, atk_block: f32) -> Option<MeleeResult> {
    if !ts_server::rules::has_player(victim) {
        return None;
    }
    let r = ts_server::world::with::<Player, _>(victim, |v| {
        let g = &mut v.inv.gun;
        match kind {
            MeleeKind::KungFu | MeleeKind::ColdCock => {
                if g.melee_bar < 0.1 {
                    return None;
                }
                if g.block_kungfu > 0.0 {
                    // the victim blocks: attacker 0.5 s lockout and -33 power, victim 0.25 s
                    g.melee_damage = 0.0;
                    g.next_attack = 0.25;
                    Some((MeleeResult::Blocked { next_attack: 0.5, bar_cost: 33.0 }, None))
                } else if atk_block > 0.0 && g.melee_source == 0 && g.melee_window > 0.0 {
                    // the attacker was blocking while the victim was mid-attack: the victim gets
                    // 0.5 s, then `UpdateMeleeBar(33)` (an emptied bar makes that 1.5 s)
                    g.melee_damage = 0.0;
                    g.next_attack = 0.5;
                    g.charge_melee_bar(33.0);
                    let bar = g.melee_bar;
                    Some((MeleeResult::Blocked { next_attack: 0.25, bar_cost: 0.0 }, Some(bar)))
                } else {
                    None
                }
            }
            MeleeKind::Knife => {
                if g.block_knife > 0.0 || (g.block_katana > 0.0 && g.melee_bar > 0.1) {
                    g.next_attack = 0.25;
                    Some((MeleeResult::Blocked { next_attack: 0.5, bar_cost: 33.0 }, None))
                } else {
                    None
                }
            }
            MeleeKind::Katana => {
                if g.block_katana > 0.0 && g.melee_bar > 0.1 {
                    g.next_attack = 0.35;
                    Some((MeleeResult::Blocked { next_attack: 0.75, bar_cost: 33.0 }, None))
                } else {
                    None
                }
            }
        }
    })
    .flatten()?;
    let (result, victim_bar) = r;
    if let Some(bar) = victim_bar {
        send_kfu_power(victim, bar);
    }
    match kind {
        MeleeKind::KungFu | MeleeKind::ColdCock => {
            let who = if victim_bar.is_some() { victim } else { attacker };
            slow_emit(who, c"player/kungfuhit.wav", 100);
        }
        MeleeKind::Knife | MeleeKind::Katana => {
            let a = attacker.v();
            let v = victim.v();
            let mut mid = spread::scale(spread::add(a.origin, v.origin), 0.5);
            mid[2] += 20.0;
            sparks(mid);
            slow_emit(attacker, c"weapons/knife/knife_hit.wav", 100);
        }
    }
    Some(result)
}

/// The end of a jump kick that hit (`CTSStunt::CloseAttack` 0x9c630): `UTIL_WeaponTimeBase()` is
/// 0, so only a run-out `m_flNextAttack` (it rests at -0.001) is set, to 0.35; then
/// `UpdateMeleeBar(strength * 1.5)`. (TS sets the attack time before the sound; nothing reads it
/// in between.)
pub fn jump_kick_hit(p: &mut Player, cx: &mut Cx, strength: f32) {
    if p.inv.gun.next_attack < 0.0 {
        p.inv.gun.next_attack = 0.35;
    }
    ts_server::weapons::update_melee_bar(p, cx, strength * 1.5);
}

/// The backflip kick (`CTSStunt::GoWheel` backward): 22.5 kung-fu power and a delayed hit of 15
/// (30 with Kung Fu) in the gun's close-combat window.
pub fn backflip_kick(p: &mut Player, cx: &mut Cx) {
    let dmg = if p.kungfu { 30.0 } else { 15.0 };
    ts_server::weapons::arm_kick(p, cx, 22.5, dmg);
}
