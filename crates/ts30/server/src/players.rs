//! TS 3.0's player hooks: The One, the default team and the team-change tail, the respawn time,
//! the overheal and The One's regeneration, and the spawn loadout (`BuyArmament`).

use goldsrc::server::engine;
use goldsrc::sys::CHAN_ITEM;
use shared::protocol::hud_wire::PwUp;
use shared::ts::powerup as pw;
use shared::weapons::{MAX_AMMO, MAX_WEAPONS, att, id, info, is_restricted, tables};
use ts_server::messages::Msg;
use ts_server::player::Player;
use ts_server::rules::msg::{self, ts_message_all};
use ts_server::rules::scoring::THE_ONE_RGB;
use ts_server::rules::{alive, cvars, netname, set_client_maxspeed, state};
use ts_server::weapons;
use ts_server::world::Cx;

/// `CTSGameRules::SetPlayerTheOne` (0x7b078): only a living, non-observing player. Health 150,
/// kung fu and super jump, speed 330, a stored 3 s slow pause when nothing is held or active,
/// award 10, the announcement and the log line. The team switch is the caller's.
pub fn set_player_the_one(p: &mut Player, cx: &mut Cx) {
    if !alive(cx.pev()) || cx.pev().iuser1 != 0 {
        return;
    }
    let e = cx.edict;
    let eng = engine();
    let now = cx.time();
    p.ts.is_the_one = true;
    cx.pev().health = 150.0;
    p.ts.health_timer = now + 1.0;
    p.kungfu = true;
    p.superjump = true;
    eng.set_physics_key(e, c"mj", c"1");
    // SetPlayerSpeed(330).
    p.player_speed = 330.0;
    cx.pev().maxspeed = 330.0;
    set_client_maxspeed(e, 330.0);
    msg::one(Msg::PwUp, e, |w| PwUp { bits: pw::KUNGFU as i16, secs: 0 }.write(w));
    msg::one(Msg::PwUp, e, |w| PwUp { bits: pw::SUPERJUMP as i16, secs: 0 }.write(w));
    if p.active_powerup == 0 && p.ts.held == 0 {
        p.active_powerup = 0;
        p.ts.held = pw::SLOWPAUSE;
        p.powerup_until = 0.0;
        p.ts.held_secs = 3;
    }
    p.ts.award = 10;
    state::with(|g| g.the_one_exists = true);
    let name = netname(e);
    ts_message_all(THE_ONE_RGB, 1, 30, &shared::bformat!("{} is The One!", name));
    msg::log(&shared::bformat!("Player \"{}\" triggered \"the_one\"\n", name));
}

/// The team `CHalfLifeTeamplay::SetDefaultPlayerTeam` (0xe44ac) reads first: in The One mode
/// The One's team or the other one, otherwise the `team` key.
pub fn default_team(p: &Player, cx: &mut Cx) -> Vec<u8> {
    let (the_one, one) = state::with(|g| (g.the_one, g.the_one_team_names()));
    if the_one { one[if p.ts.is_the_one { 0 } else { 1 }].clone() } else { engine().info_key(cx.edict, c"team").to_bytes().to_vec() }
}

/// Whether `ChangePlayerTeam` sends the player to observer with a 5 s respawn (0xe4c38..0xe4ca1):
/// every change that isn't a kill, outside The One mode, even to the same team or while already
/// observing.
fn change_team_observes(kill: bool, the_one: bool) -> bool {
    !kill && !the_one
}

/// The tail of `ChangePlayerTeam` (0xe4c38..0xe4ca1): a change that isn't a kill, outside The One
/// mode, puts the player in observer at its origin and view angles with a 5 s respawn.
pub fn change_team_tail(p: &mut Player, cx: &mut Cx, kill: bool) {
    if change_team_observes(kill, state::with(|g| g.the_one)) {
        let (o, a) = (cx.pev().origin, cx.pev().v_angle);
        ts_server::rules::lifecycle::start_observer(p, cx, o, a);
        ts_server::rules::lifecycle::set_respawn_time(p, cx, 5.0);
    }
}

/// The overheal: health above max decays 1/s; The One regenerates (`PreThink` 0x908cc, `+0x73c`).
pub fn health_regen(p: &mut Player, cx: &mut Cx, now: f32) {
    if p.ts.health_timer == 0.0 {
        return;
    }
    if p.ts.is_the_one {
        if p.ts.health_timer < now {
            let n = state::with(|g| g.the_one_others);
            let v = cx.pev();
            (p.ts.health_timer, v.health) = the_one_regen(n, now, v.health);
        }
        return;
    }
    let v = cx.pev();
    if v.health <= v.max_health {
        p.ts.health_timer = 0.0;
    } else if p.ts.health_timer < now {
        p.ts.health_timer = now + 1.0;
        v.health -= 1.0;
    }
}

/// The One's regeneration tick (mp.dll `PreThink` 0x1005f8bc..0x1005f92d) with `n` other players:
/// the next tick at `time + 1 / max(n * 0.32f, 1)` and `health + n * 0.16f` capped at 150. Each
/// is computed on the x87 (53 bits) from `fild n` and stored to a `float` once; the cap keeps a
/// NaN health (`fcom 150; test ah,1`).
fn the_one_regen(n: i32, now: f32, health: f32) -> (f32, f32) {
    let k = f64::from(n) * f64::from(0.32f32);
    let timer = 1.0 / if 1.0 < k { k } else { 1.0 } + f64::from(now);
    let h = f64::from(n) * f64::from(0.16f32) + f64::from(health);
    let h = if h < 150.0 || h.is_nan() { h } else { 150.0 };
    (timer as f32, h as f32)
}

/// `SetPlayerRespawnTime` (0x75ff4).
pub fn set_respawn_time(p: &mut Player, cx: &mut Cx, secs: f32) {
    let now = cx.time();
    let (mode, _, lms, _) = state::mode();
    let round_end = state::with(|g| g.round_end);
    if lms && mode == state::Mode::Dm {
        if p.ts.respawns_used as f32 >= cvars::RESPAWN_NUMBER.max(1.0) {
            p.respawn_at = -1.0;
        } else {
            p.respawn_at = secs.min(round_end - now) + now;
            p.ts.join_deadline = (round_end - now).min(10.0) + now;
        }
    } else {
        p.respawn_at = now + secs;
    }
    p.ts.award = if p.respawn_at < 0.0 { 1 } else { 11 };
}

/// The loadout part of `TSInit` (0x92a4c): `pl+0x718 = 0`. `Player::spawn` has already zeroed
/// `armorvalue`, so a vest still worn (a spawn that didn't go through death) gives its slots back.
/// `CBasePlayer::AddItem(bits)` (`tki`): 0x10 kevlar, 0x20 stealth shoes, else attachments for
/// the held weapon. Prices are never charged (`CashBased() == 0`).
fn add_item(p: &mut Player, cx: &mut Cx, bits: u32) {
    if bits & att::KEVLAR != 0 {
        if cx.pev().armorvalue < 100.0 {
            weapons::take_armor(p, cx, 100);
        }
        return;
    }
    if bits & att::STEALTH_SHOES != 0 {
        // TS: only in round modes (m_mode >= 2), never in 3.0 DM/TDM.
        if !weapons::grenade_buys() {
            return;
        }
        let pev = cx.pev();
        if pev.iuser4 & shared::ts::stunt::STEALTH_SHOES != 0 {
            return;
        }
        pev.iuser4 |= shared::ts::stunt::STEALTH_SHOES;
        p.inv.sv.items |= att::STEALTH_SHOES;
        weapons::emit(cx.edict, CHAN_ITEM, c"weapons/items.wav", 1.0, 0.8, 100);
        return;
    }
    if !weapons::gun_active(p) {
        return; // attachments go on the gun's weapon
    }
    weapons::add_gun_item(p, cx, bits);
}

/// `CBasePlayer::BuyWeapon(id, attachments)`.
fn buy_weapon(p: &mut Player, cx: &mut Cx, wid: i32, attachments: u32) {
    if !weapons::gun_active(p) || !(0..MAX_WEAPONS as i32).contains(&wid) {
        return;
    }
    let wid = wid as u8;
    if is_restricted(wid, weapons::restriction()) {
        return;
    }
    if wid == id::M61 {
        if !weapons::grenade_buys() || p.inv.sv.grenades_bought > 1 {
            return;
        }
        p.inv.sv.grenades_bought += 1;
    }
    weapons::pick_up_weapon(p, cx, wid, None, attachments, true, true);
}

/// `CBasePlayer::BuyAmmo(type)`: one purchase (`per_buy` rounds); -1 = held weapon's type,
/// -2 = every owned weapon's type until full. Returns whether any ammo was added.
pub fn buy_ammo(p: &mut Player, cx: &mut Cx, t: i32) -> bool {
    if !weapons::gun_active(p) {
        return false;
    }
    let per_buy = |t: u8| tables().ammo(t).map(|a| a.per_buy).unwrap_or(0);
    let mut sound = false;
    if t == -2 {
        for wid in (0..MAX_WEAPONS as u8).rev() {
            if !p.inv.gun.status[wid as usize].owned {
                continue;
            }
            let ammo = info(wid).map(|w| w.ammo).unwrap_or(0);
            if ammo == 0 {
                continue;
            }
            while p.inv.gun.add_ammo(ammo, per_buy(ammo), weapons::max_carry(ammo)) {
                sound = true;
            }
        }
    } else {
        if p.inv.gun.current == 0 {
            return false;
        }
        let ammo = if t == -1 { info(p.inv.gun.current).map(|w| w.ammo).unwrap_or(0) } else { t as u8 };
        if ammo == 0 || ammo as usize >= MAX_AMMO {
            return false;
        }
        sound = p.inv.gun.add_ammo(ammo, per_buy(ammo), weapons::max_carry(ammo));
    }
    if sound {
        weapons::emit(cx.edict, CHAN_ITEM, c"items/9mmclip1.wav", 1.0, 0.8, 100);
    }
    sound
}

/// `CBasePlayer::BuyArmament` (from `PlayerSpawn` when the map has no `game_player_equip`).
pub fn buy_armament(p: &mut Player, cx: &mut Cx) {
    let list = p.inv.sv.armament.clone();
    for a in list {
        if a.id == 0 {
            continue;
        }
        if a.item {
            add_item(p, cx, a.id as u32);
        } else {
            buy_weapon(p, cx, a.id, a.attachments);
        }
    }
    // Guard: `tka` takes any purchase count from the client, and TS loops that many times. A
    // purchase that adds nothing leaves everything as it was, so every later one would too: stop
    // there instead of spinning through billions of no-ops at spawn.
    let ammo = p.inv.sv.armament_ammo;
    for (t, &n) in ammo.iter().enumerate() {
        for _ in 0..n.max(0) {
            if !buy_ammo(p, cx, t as i32) {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The One's tick (mp.dll 0x1005f8bc..): computed unrounded, stored once (#147).
    #[test]
    fn the_one_regen_rounds_once() {
        // 1.85 + 1/1.28: the f32 steps give 2.6312499.
        let (timer, _) = the_one_regen(4, 1.85, 100.0);
        assert_eq!(timer.to_bits(), 0x4028_6667, "2.6312501");
        assert_ne!(timer, 1.85f32 + 1.0 / (4.0f32 * 0.32));
        // 125.92 + 27 * 0.16: the f32 steps give 130.23999.
        let (_, h) = the_one_regen(27, 0.0, 125.92);
        assert_eq!(h.to_bits(), 0x4302_3d71, "130.24001");
        assert_ne!(h, 125.92f32 + 27.0 * 0.16f32);
        // n * 0.32 below 1 is raised to 1; the cap is 150; a NaN health stays NaN.
        assert_eq!(the_one_regen(1, 10.0, 149.9), (11.0, 150.0));
        assert!(the_one_regen(1, 10.0, f32::NAN).1.is_nan());
    }

    #[test]
    fn change_team_observes_unless_kill_or_the_one() {
        assert!(change_team_observes(false, false));
        assert!(!change_team_observes(true, false));
        assert!(!change_team_observes(false, true));
    }
}
