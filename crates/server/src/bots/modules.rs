//! The smaller brain modules (bots.md §7, §9, §10.2): stunt, power-up and chat, and the helpers
//! the rulesets' vision and buy modules share (the view cone, the line of sight, the loadout).
//! The vision, buy and look-around modules themselves are the rules' hooks
//! ([`super::rules::NativeBotRules`]); the respawn module is inline in [`super::brain`],
//! navigation is [`super::nav`].

use super::brain::{Brain, add, bit, btn};
use super::rules::delay;
use crate::player::Player;
use crate::rules::cvars;
use crate::weapons::ArmamentEntry;
use crate::world::Cx;
use goldsrc::edict::EdictPtr;
use goldsrc::server::{IGNORE_MONSTERS, engine};
use goldsrc::sys::*;
use shared::weapons::{MAX_AMMO, info};

/// `UTIL_TraceLine(.., ignore_monsters, ignore_glass, ..)`: the engine's `fNoMonsters` with the
/// glass bit (0x100) set.
const IGNORE_MONSTERS_GLASS: i32 = IGNORE_MONSTERS | 0x100;

/// `m_flFieldOfView` (`pl+0x140`): `CBasePlayer::Spawn` sets 0.5 and nothing else changes it for
/// a player.
const PLAYER_FOV: f32 = 0.5;

// ---- vision (bit 0x40): the checks -----------------------------------------------------------------

/// `CBaseEntity::FInViewCone(Vector *)` (0x10014ab0): the flat direction to `point` against the
/// flat forward of `pev->angles` (the model angles: pitch is a third of the view's).
pub fn in_view_cone(v: &entvars_t, point: [f32; 3]) -> bool {
    let (f, _, _) = engine().angle_vectors(v.angles);
    cone_dot(v.origin, point, f) > PLAYER_FOV as f64
}

/// `FInViewCone`'s dot product as mp.dll computes it (0x10014acf..0x10014b5b): the flat
/// differences and their length stay on the x87 stack, the direction (times `1 / length`) is
/// stored as floats, and the dot `y·fy + x·fx` is compared unrounded.
fn cone_dot(origin: [f32; 3], point: [f32; 3], forward: [f32; 3]) -> f64 {
    let (dx, dy) = (point[0] as f64 - origin[0] as f64, point[1] as f64 - origin[1] as f64);
    let l = (dy * dy + dx * dx).sqrt();
    let d = if l == 0.0 { [0.0f32; 2] } else { [(dx * (1.0 / l)) as f32, (dy * (1.0 / l)) as f32] };
    d[1] as f64 * forward[1] as f64 + d[0] as f64 * forward[0] as f64
}

/// `CBaseEntity::FVisible(const Vector &)` (0x10014cb0): one eye-to-point trace that ignores
/// monsters and glass, clear all the way.
pub fn visible(v: &entvars_t, me: *mut edict_t, point: [f32; 3]) -> bool {
    let tr = engine().trace_line(add(v.origin, v.view_ofs), point, IGNORE_MONSTERS_GLASS, me);
    tr.flFraction == 1.0
}

// ---- buy (bit 0x10): the loadout -------------------------------------------------------------------

/// `CBasePlayer::AddItemToArmament(id, attachments, isItem)` (0x93ea4): into the first entry
/// whose id is 0, at most 32.
pub fn add_item(p: &mut Player, id: i32, attachments: u32, item: bool) {
    let list = &mut p.inv.sv.armament;
    let e = ArmamentEntry { id, attachments, item };
    match list.iter().position(|a| a.id == 0) {
        Some(i) => list[i] = e,
        None if list.len() < 32 => list.push(e),
        None => {}
    }
}

/// `CBasePlayer::AddAmmoToArmament(type, count)` (0x93ef4).
pub fn add_ammo(p: &mut Player, ammo: i32, count: i32) {
    if (0..MAX_AMMO as i32).contains(&ammo) {
        p.inv.sv.armament_ammo[ammo as usize] = count;
    }
}

/// `CBasePlayer::ResetArmament` (0x93e88): an empty loadout.
pub fn reset_armament(p: &mut Player) {
    p.inv.sv.armament.clear();
    p.inv.sv.armament_ammo = [0; MAX_AMMO];
}

/// `BotRandomAttachments(id)` (0x1006d560): silencer and laser each 50/101, scope 25/101, if the
/// weapon allows them and doesn't have them built in. Always three draws.
pub fn random_attachments(id: u8) -> u32 {
    let eng = engine();
    let (allowed, builtin) = info(id).map_or((0, 0), |w| (w.att_allowed as u32, w.att_default as u32));
    let mut att = 0;
    for (bit, above) in [(1u32, 50), (2, 50), (8, 75)] {
        if eng.random_long(0, 100) > above && allowed & bit != 0 && builtin & bit == 0 {
            att |= bit;
        }
    }
    att
}

/// One table entry: the weapon with random attachments, a second copy without attachments half
/// of the time for `akimbo` ones (two singles make the pair), then 10 purchases of `ammo`.
pub fn buy_weapon(p: &mut Player, id: u8, akimbo: bool, ammo: i32) {
    let att = random_attachments(id);
    add_item(p, id as i32, att, false);
    if akimbo && engine().random_long(0, 1) != 0 {
        add_item(p, id as i32, 0, false);
    }
    add_ammo(p, ammo, 10);
}

// ---- stunt (bit 8) -------------------------------------------------------------------------------

/// `CBotModuleStunt::Think` (0x1006e130): a dive (75/101, the stunt key) or a flip (26/101, two
/// jump presses; the odds are [`NativeBotRules::stunt_mode`](super::rules::NativeBotRules::stunt_mode)), then a jump to get up once prone. It only stops (mode 4) after getting up from
/// lying down, so a stunt that doesn't end prone keeps it going.
pub fn stunt(b: &mut Brain, _p: &mut Player, cx: &mut Cx) {
    use shared::ts::stunt::{DIVING, FLIP_HIGH, FLIP_LOW, PRONE};
    let now = cx.time();
    if b.stunt_next > now {
        return;
    }
    b.stunt_next = delay(now, 0.5, cx.pev(), b.skill, 1.0);
    if b.stunt_mode == 0 {
        b.stunt_mode = super::rules::current().stunt_mode();
        b.stunt_jumps = 0;
    }
    let (s, flags) = (cx.pev().iuser4, cx.pev().flags);
    if s & (DIVING | PRONE | FLIP_HIGH | FLIP_LOW) == 0 {
        match b.stunt_mode {
            1 => b.press(btn::ALT1, 0.1, now),
            2 => {
                if flags & FL_ONGROUND as i32 == 0 && b.stunt_jumps != 0 {
                    b.press(btn::JUMP, 0.1, now); // the second press, airborne: flip
                } else {
                    b.press(btn::JUMP, 0.1, now);
                    b.stunt_jumps = b.stunt_jumps.wrapping_add(1);
                    b.stunt_next = (now as f64 + 0.2) as f32;
                }
            }
            4 => {
                b.stunt_mode = 0;
                b.disable(bit::STUNT);
            }
            _ => {}
        }
        return;
    }
    if s & DIVING == 0 && s & PRONE != 0 {
        b.press(btn::JUMP, 0.1, now); // get up
        b.stunt_mode = 4;
    }
}

// ---- power-up (bit 0x200) ------------------------------------------------------------------------

/// `CBotModulePowerUp::Think` (0x1006e280): use the held power-up; the bit goes off once one is
/// active (a refused use retries every `0.5 / slow / skill` s, at most 3 s).
pub fn powerup(b: &mut Brain, p: &mut Player, cx: &mut Cx) {
    let now = cx.time();
    if b.powerup_next > now {
        return;
    }
    b.powerup_next = delay(now, 0.5, cx.pev(), b.skill, 3.0);
    if p.ts.held != 0 && p.active_powerup == 0 {
        crate::rules::powerup::use_powerup(p, cx);
    }
    if p.active_powerup != 0 {
        b.disable(bit::POWERUP);
        b.powerup_flag = 0;
    }
}

// ---- chat (bit 0x400) ----------------------------------------------------------------------------

/// `CBotModuleChat::Think` (0x1006e330): when the chat timer is due (and `bottalk` is on), a
/// random line of the pending section with the stored name for `%s`, then the next line in
/// 60..180 s (scaled by `clamp(skill, 0.5, 1)`) and back to freetime.
pub fn chat(b: &mut Brain, _p: &mut Player, cx: &mut Cx) {
    if cvars::f(c"bottalk") == 0.0 {
        return;
    }
    let now = cx.time();
    let due = b.next_chat <= now;
    if !due && !b.chat_now {
        return;
    }
    let s = super::brain::clamp_skill(b.skill, 1.0);
    let eng = engine();
    b.next_chat = (eng.random_float(60.0, 180.0) as f64 * s as f64 + now as f64) as f32;
    let line = super::with_files(|f| f.say.pick(b.chat_section, |lo, hi| eng.random_long(lo, hi)));
    let text = super::files::format_line(&line, &b.chat_name);
    if !text.is_empty() {
        bot_say(cx.edict, &text);
    }
    b.disable(bit::CHAT);
    b.chat_section = super::files::FREETIME;
    b.chat_name.clear();
    b.chat_now = false;
}

/// `BotSay` (0x1006e7a0): `SayText` to every player but the speaker that is a net client (not a
/// bot), in entity order, players who left included. Always all-chat; not logged or echoed.
pub fn bot_say(me: *mut edict_t, text: &[u8]) {
    let eng = engine();
    let name = eng.string(me.v().netname).to_bytes();
    let msg = super::files::say_text(name, text);
    let Ok(msg) = std::ffi::CString::new(msg.into_iter().filter(|&c| c != 0).collect::<Vec<u8>>()) else { return };
    let idx = eng.index_of(me);
    for o in crate::rules::slot_players() {
        if o == me || !crate::rules::is_net_client(o) {
            continue;
        }
        crate::messages::one(shared::protocol::Msg::SayText, o).byte(idx).string(&msg).end();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_cone_dot() {
        let fwd = [1.0f32, 0.0, 0.0];
        assert_eq!(cone_dot([0.0; 3], [100.0, 0.0, 50.0], fwd), 1.0);
        // 60 degrees off is the edge of the 0.5 cone; the height doesn't count.
        let d = cone_dot([10.0, 10.0, 0.0], [10.5, 10.0 + 0.75f32.sqrt(), -40.0], fwd);
        assert!((d - 0.5).abs() < 1e-6, "{d}");
        // Directly on top: no direction.
        assert_eq!(cone_dot([5.0, 5.0, 0.0], [5.0, 5.0, 90.0], fwd), 0.0);
        // The direction is stored as floats and the dot is `y·fy + x·fx`, unrounded.
        let (o, p, f) = ([1000.1f32, -3.3, 0.0], [1777.7f32, 555.5, 0.0], [0.6f32, 0.8, 0.0]);
        let (dx, dy) = (p[0] as f64 - o[0] as f64, p[1] as f64 - o[1] as f64);
        let inv = 1.0 / (dy * dy + dx * dx).sqrt();
        let want = ((dy * inv) as f32) as f64 * f[1] as f64 + ((dx * inv) as f32) as f64 * f[0] as f64;
        assert_eq!(cone_dot(o, p, f), want);
    }
}
