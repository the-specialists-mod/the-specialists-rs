//! The smaller brain modules (bots.md §7, §9, §10.2): vision, buy, stunt, look around, power-up
//! and chat. The respawn module is inline in [`super::brain`], navigation is [`super::nav`] and
//! attack [`super::attack`].

use super::brain::{Brain, add, bit, btn, delay, gun_position, length, scale, slow, sub};
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

// ---- vision (bit 0x40) ---------------------------------------------------------------------------

/// `CBotModuleVision::Scan` (0x1006dc10): every `0.1 / slow / skill` s (at most 0.7 s), list the
/// living players in a ±60° cone whose eyes the bot's eyes can see; teammates (teamplay, same
/// team) go to a list nothing reads, everyone else is an enemy.
pub fn vision(b: &mut Brain, _p: &mut Player, cx: &mut Cx) {
    let now = cx.time();
    if b.vision_next > now {
        return;
    }
    b.vision_next = delay(now, 0.1, slow(cx.pev()), b.skill, 0.7);
    b.visible_enemies = [std::ptr::null_mut(); 16];
    b.visible_mates = [std::ptr::null_mut(); 16];
    b.num_visible_enemies = 0;
    let me = cx.edict;
    let eng = engine();
    for i in 1..=eng.max_clients() {
        // `UTIL_PlayerByIndex` + `IsPlayer()`: any player, one who left included.
        let o = eng.player(i);
        if !crate::rules::has_player(o) || o == me {
            continue;
        }
        let ov = o.v();
        if !crate::rules::alive(ov) || ov.effects & EF_NODRAW as i32 != 0 {
            continue;
        }
        let eye = add(ov.origin, ov.view_ofs); // EyePosition
        if !in_view_cone(cx.pev(), eye) || !visible(cx.pev(), me, eye) {
            continue;
        }
        let mate = crate::rules::teams::same_team(me, o); // PlayerRelationship == GR_TEAMMATE
        for j in 0..16 {
            if mate {
                if b.visible_mates[j].is_null() {
                    b.visible_mates[j] = o;
                    break;
                }
            } else if b.visible_enemies[j].is_null() {
                b.visible_enemies[j] = o;
                b.num_visible_enemies = b.num_visible_enemies.wrapping_add(1);
                break;
            }
        }
    }
}

/// `CBaseEntity::FInViewCone(Vector *)` (0x10014ab0): the flat direction to `point` against the
/// flat forward of `pev->angles` (the model angles: pitch is a third of the view's).
pub fn in_view_cone(v: &entvars_t, point: [f32; 3]) -> bool {
    let (f, _, _) = engine().angle_vectors(v.angles);
    let d = [point[0] - v.origin[0], point[1] - v.origin[1]];
    let l = ((d[0] as f64).powi(2) + (d[1] as f64).powi(2)).sqrt() as f32;
    let d = if l == 0.0 { [0.0, 0.0] } else { [d[0] / l, d[1] / l] };
    let dot = (d[0] as f64 * f[0] as f64 + d[1] as f64 * f[1] as f64) as f32;
    dot > PLAYER_FOV
}

/// `CBaseEntity::FVisible(const Vector &)` (0x10014cb0): one eye-to-point trace that ignores
/// monsters and glass, clear all the way.
pub fn visible(v: &entvars_t, me: *mut edict_t, point: [f32; 3]) -> bool {
    let tr = engine().trace_line(add(v.origin, v.view_ofs), point, IGNORE_MONSTERS_GLASS, me);
    tr.flFraction == 1.0
}

// ---- buy (bit 0x10) ------------------------------------------------------------------------------

/// `CBasePlayer::AddItemToArmament(id, attachments, isItem)` (0x93ea4): into the first entry
/// whose id is 0, at most 32.
fn add_item(p: &mut Player, id: i32, attachments: u32, item: bool) {
    let list = &mut p.inv.sv.armament;
    let e = ArmamentEntry { id, attachments, item };
    match list.iter().position(|a| a.id == 0) {
        Some(i) => list[i] = e,
        None if list.len() < 32 => list.push(e),
        None => {}
    }
}

/// `CBasePlayer::AddAmmoToArmament(type, count)` (0x93ef4).
fn add_ammo(p: &mut Player, ammo: i32, count: i32) {
    if (0..MAX_AMMO as i32).contains(&ammo) {
        p.inv.sv.armament_ammo[ammo as usize] = count;
    }
}

/// `BotRandomAttachments(id)` (0x1006d560): silencer and laser each 50/101, scope 25/101, if the
/// weapon allows them and doesn't have them built in. Always three draws.
fn random_attachments(id: u8) -> u32 {
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
fn buy_weapon(p: &mut Player, id: u8, akimbo: bool, ammo: i32) {
    let att = random_attachments(id);
    add_item(p, id as i32, att, false);
    if akimbo && engine().random_long(0, 1) != 0 {
        add_item(p, id as i32, 0, false);
    }
    add_ammo(p, ammo, 10);
}

/// `CBotModuleBuy::Think` (0x1006d630): one roll `f` decides the loadout tier; no money involved.
/// Kevlar 70 % (`botarmor` other than 0 always, -1 never), a katana or knives, a pistol above
/// 0.1, an SMG in (0.4, 0.7), a rifle from 0.7. `BuyArmament` applies the list at the respawn.
pub fn buy(b: &mut Brain, p: &mut Player, _cx: &mut Cx) {
    use shared::weapons::id::*;
    let eng = engine();
    let f = eng.random_float(0.0, 1.0);
    // ResetArmament (0x93e88).
    p.inv.sv.armament.clear();
    p.inv.sv.armament_ammo = [0; MAX_AMMO];
    let armor = cvars::f(c"botarmor");
    if (f > 0.3 || armor != 0.0) && armor != -1.0 {
        add_item(p, 0x10, 0, true); // kevlar
    }
    let k = eng.random_long(0, 5);
    if k == 5 {
        add_item(p, KATANA as i32, 0, false);
    } else if k != 0 {
        let n = if f > 0.75 { 10 } else { 1 };
        let t = eng.random_long(0, 2); // 0 seal knife, 1 combat knife, 2 both
        for _ in 0..n {
            if t == 0 || t == 2 {
                add_item(p, SEAL_KNIFE as i32, 0, false);
            }
            if t != 0 {
                add_item(p, KNIFE as i32, 0, false);
            }
        }
    }
    if f > 0.1 {
        let (id, akimbo, ammo) = match eng.random_long(0, 9) {
            0 => (CONTENDER, false, 13),
            1 => (GLOCK18, false, 1),
            2 => (MK23, true, 2),
            3 => (DEAGLE, false, 5),
            4 => (FIVESEVEN, true, 7),
            5 => (BERETTA, true, 1),
            6 => (GOLDEN_COLTS, false, 2),
            7 => (GLOCK20, false, 9),
            8 => (RUGER, false, 10),
            9 => (RAGING_BULL, false, 11),
            _ => (0, false, 0),
        };
        if id != 0 {
            buy_weapon(p, id, akimbo, ammo);
        }
    }
    if f > 0.4 && f < 0.7 {
        let (id, akimbo, ammo) = match eng.random_long(1, 6) {
            1 => (UZI, true, 1),
            2 => (MP5SD, false, 1),
            3 => (MP5K, false, 1),
            4 => (SKORPION, true, 12),
            5 => (MP7, false, 7),
            6 => (UMP, false, 2),
            _ => (0, false, 0),
        };
        if id != 0 {
            buy_weapon(p, id, akimbo, ammo);
        }
    } else if f >= 0.7 {
        let (id, ammo) = match eng.random_long(1, 11) {
            1 => (BENELLI, 3),
            2 => (USAS, 3),
            3 => (SPAS, 3),
            4 => (AK47, 6),
            5 => (M4, 4),
            6 => (AUG, 4),
            7 => (MOSSBERG, 3),
            8 => (M16, 4),
            9 => (SAWEDOFF, 3),
            10 => (BARRETT, 8),
            11 => (M60, 4),
            _ => (0, 0),
        };
        if id != 0 {
            buy_weapon(p, id, false, ammo);
        }
    }
    b.disable(bit::BUY);
}

// ---- stunt (bit 8) -------------------------------------------------------------------------------

/// `CBotModuleStunt::Think` (0x1006e130): a dive (75/101, the stunt key) or a flip (26/101, two
/// jump presses), then a jump to get up once prone. It only stops (mode 4) after getting up from
/// lying down, so a stunt that doesn't end prone keeps it going.
pub fn stunt(b: &mut Brain, _p: &mut Player, cx: &mut Cx) {
    use shared::ts::stunt::{DIVING, FLIP_HIGH, FLIP_LOW, PRONE};
    let now = cx.time();
    if b.stunt_next > now {
        return;
    }
    b.stunt_next = delay(now, 0.5, slow(cx.pev()), b.skill, 1.0);
    if b.stunt_mode == 0 {
        b.stunt_mode = if engine().random_long(0, 100) <= 25 { 2 } else { 1 };
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

// ---- look around (bit 0x80) ----------------------------------------------------------------------

/// `CBotModuleLookAround::Think` (0x1006dde0): look at a spot found by [`find_look_point`] every
/// `0.5 / slow / skill` s (at most 5 s).
pub fn look_around(b: &mut Brain, _p: &mut Player, cx: &mut Cx) {
    let now = cx.time();
    if b.look_next > now {
        return;
    }
    let point = find_look_point(cx);
    b.look_at(cx.pev(), point);
    b.look_next = delay(now, 0.5, slow(cx.pev()), b.skill, 5.0);
}

/// `FindLookPoint` (0x1006dea0): from a random yaw, eight directions 45° apart, each with a random
/// pitch (`sin(pitch)` as z, not normalised), 1024 units from the gun position. The first one
/// that is clear wins, else the end point farthest from the origin.
fn find_look_point(cx: &mut Cx) -> [f32; 3] {
    let eng = engine();
    let me = cx.edict;
    let mut best = -1.0f32;
    let mut best_pos = [0.0f32; 3];
    let mut yaw = eng.random_float(0.0, 360.0);
    for _ in 0..8 {
        let pitch = eng.random_float(-30.0, 30.0);
        let rad = std::f64::consts::PI / 180.0;
        let d = [(yaw as f64 * rad).cos() as f32, (yaw as f64 * rad).sin() as f32, (pitch as f64 * rad).sin() as f32];
        let v = cx.pev();
        let src = gun_position(v);
        let tr = eng.trace_line(src, add(src, scale(d, 1024.0)), IGNORE_MONSTERS, me);
        if tr.flFraction == 1.0 {
            return tr.vecEndPos;
        }
        let len = length(sub(tr.vecEndPos, cx.pev().origin));
        if len > best {
            best = len;
            best_pos = tr.vecEndPos;
        }
        yaw += 45.0;
    }
    best_pos
}

// ---- power-up (bit 0x200) ------------------------------------------------------------------------

/// `CBotModulePowerUp::Think` (0x1006e280): use the held power-up; the bit goes off once one is
/// active (a refused use retries every `0.5 / slow / skill` s, at most 3 s).
pub fn powerup(b: &mut Brain, p: &mut Player, cx: &mut Cx) {
    let now = cx.time();
    if b.powerup_next > now {
        return;
    }
    b.powerup_next = delay(now, 0.5, slow(cx.pev()), b.skill, 3.0);
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
