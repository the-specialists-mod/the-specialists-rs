//! `CHalfLifeMultiplay::PlayerKilled` (0x895e4), `TSGetPointsForFrag` (0x89140) and
//! `DeathNotice` (0x88f7c): points, sprees and awards, cash, kill messages, DeathMsg, ScoreInfo.
//! Decomp: gamerules-entities.md §2.3.

use super::lifecycle::{give_cash, score_info_of};
use super::msg::{self, ts_message_all};
use super::{cvars, is_player, netname, other, state, teams};
use crate::damage::{DamageInfo, bits, hitgroup};
use crate::messages::Msg;
use crate::player::Player;
use crate::world::{Cx, UseType};
use goldsrc::edict::EdictPtr;
use goldsrc::server::engine;
use goldsrc::sys::*;
use shared::protocol::hud_wire::{PwUp, director};
use shared::protocol::rules::DeathMsg;
use shared::ts::{powerup as pw, stunt};
use shared::weapons::{WeaponKind, tables};

const GREEN: [u8; 3] = [0, 255, 0];
/// `iuser4` bits that count as "stunting" for scoring and damage (dive, flips, wall jump).
pub const STUNTING: i32 = 0x10c10;

/// Close combat: kung fu / melee strike (no projectile).
pub fn is_close_combat(d: &DamageInfo) -> bool {
    d.weapon == 0 || (d.inflictor == d.attacker && d.bits & (bits::CLUB | bits::SLASH) != 0 && d.bits & bits::THROWN == 0)
}

fn classname(e: *mut edict_t) -> String {
    if e.is_null() {
        return String::new();
    }
    engine().string(e.v().classname).to_string_lossy().into_owned()
}

/// Weapon name for `DeathMsg` and logs (gamerules-entities.md §2.3).
pub fn weapon_name(d: &DamageInfo) -> String {
    let attacker_client = !d.attacker.is_null() && d.attacker.v().flags & FL_CLIENT as i32 != 0;
    let mut name = if attacker_client {
        if d.inflictor == d.attacker || d.inflictor.is_null() {
            if d.weapon == 0 || (d.weapon < 0 && is_close_combat(d)) {
                "Kung Fu".to_owned()
            } else if let Some(w) = u8::try_from(d.weapon).ok().and_then(|id| tables().weapon(id)) {
                w.name.clone()
            } else {
                "world".to_owned()
            }
        } else {
            classname(d.inflictor)
        }
    } else if !d.inflictor.is_null() {
        let v = d.inflictor.v();
        let n = engine().string(v.netname).to_string_lossy().into_owned();
        if n.is_empty() { classname(d.inflictor) } else { n }
    } else {
        "world".to_owned()
    };
    for prefix in ["weapon_", "monster_", "func_"] {
        if let Some(rest) = name.strip_prefix(prefix) {
            name = rest.to_owned();
            break;
        }
    }
    match name.as_str() {
        "knife" => "Throwing Knife".to_owned(),
        "shrapnel" => "Shrapnel".to_owned(),
        "grenade" | "m61_grenade" => "M61 Grenade".to_owned(),
        "" => "world".to_owned(),
        _ => name,
    }
}

/// `IPointsForKill`: 1, or -1 for a team kill.
fn points_for_kill(attacker: *mut edict_t, victim: *mut edict_t) -> i32 {
    if attacker.is_null() {
        return 0;
    }
    if state::teamplay() && attacker != victim && teams::same_team(attacker, victim) {
        return -1;
    }
    1
}

/// Snapshot of the killer's Rust state we need (read through `other`).
struct Killer {
    e: *mut edict_t,
    idx: i32,
    spree: u8,
    last_kill: f32,
    is_the_one: bool,
    award: u8,
    held: u16,
    held_secs: i32,
    active: u16,
}

/// The `PlayerKilled` body. `p`/`cx` are the victim.
pub fn player_killed(p: &mut Player, cx: &mut Cx, d: &DamageInfo) {
    let eng = engine();
    let now = cx.time();
    let victim = cx.edict;
    let vidx = cx.index();
    let vname = netname(victim);
    let attacker = if d.attacker.is_null() { eng.entity_of_index(0) } else { d.attacker };
    let suicide = attacker == victim;
    let killer_is_player = !suicide && is_player(attacker);
    let close = is_close_combat(d);

    // The killer's own think (its weapon frame) is usually what killed: its Player is taken
    // out further up the stack. Score as soon as that call returns.
    if killer_is_player && crate::world::is_taken_out(attacker) {
        let d = *d;
        crate::world::defer(attacker, move || {
            crate::player::with_player(victim, |p, cx| player_killed(p, cx, &d));
        });
        return;
    }

    let mut killer = if killer_is_player {
        other(attacker, |k| Killer {
            e: attacker,
            idx: 0,
            spree: k.ts.spree,
            last_kill: k.ts.last_kill,
            is_the_one: k.ts.is_the_one,
            award: k.ts.award,
            held: k.ts.held,
            held_secs: k.ts.held_secs,
            active: k.active_powerup,
        })
    } else {
        None
    };
    if let Some(k) = killer.as_mut() {
        k.idx = eng.index_of(attacker);
    }

    let mut points = points_for_kill(attacker, victim);
    let (mut double_kill, mut stopped_fury, mut killed_specialist) = (false, false, false);
    let (mut sliding, mut diving) = (false, false);
    let (mut is_killer, mut is_demolition, mut is_specialist, mut is_unstoppable) = (false, false, false, false);

    if let Some(k) = killer.as_mut() {
        points = points_for_frag(d, attacker, victim, points, k.is_the_one, k.spree, k.last_kill, &mut sliding, &mut diving, &vname, now);
        double_kill = now - k.last_kill < 2.0;
        if p.ts.spree > 2 {
            stopped_fury = true;
            killed_specialist = p.ts.spree > 8;
        }
        if k.spree != 255 {
            k.spree += 1;
        }
        match k.spree {
            3 => is_killer = true,
            6 => is_demolition = true,
            9 => is_specialist = true,
            s if s >= 10 => is_unstoppable = true,
            _ => {}
        }
    }

    let weapon = weapon_name(d);
    death_notice(attacker, d.inflictor, victim, &weapon);
    log_kill(
        attacker,
        victim,
        &weapon,
        suicide,
        sliding,
        diving,
        double_kill,
        killed_specialist,
        stopped_fury.then_some(&vname),
        is_killer,
        is_demolition,
        is_specialist,
        is_unstoppable,
    );

    p.deaths += 1;
    crate::entities::fire_targets(c"game_playerdie", victim, victim, UseType::Toggle, 0.0);
    if p.ts.is_the_one {
        the_one_is_dead(p, cx, killer.as_ref().map(|k| k.e));
        p.ts.spree = 0;
    }

    if suicide || !killer_is_player {
        if suicide && p.ts.spree > 8 {
            super::add_frags(victim, Some(p), -5.0);
            ts_message_all(GREEN, 1, 30, &format!("{vname}, #TS_SpecialistSuicide"));
        }
        super::add_frags(victim, Some(p), -1.0);
    } else if let Some(mut k) = killer {
        let kname = netname(k.e);
        let kv = k.e.v();
        if (kv.iuser4 & STUNTING != 0 || close || points > 2) && spectacular() {
            crate::entities::ts::create_slow_motion_point(cx.pev().origin, 0.25, 1.0, false);
        }
        let mut health_delta: Option<f32> = None; // (+hp, cap 150) for awards
        let mut give_specialist = false;
        if points > 0 {
            if p.ts.last_hitgroup == hitgroup::HEAD && d.bits & (bits::BULLET | bits::THROWN) != 0 {
                ts_message_all(GREEN, 1, 20, &format!("#TS_Headshot #TS_FrgFr {kname}!"));
            }
            if (d.inflictor == d.attacker || d.inflictor.is_null()) && !close {
                match u8::try_from(d.weapon).ok().and_then(|id| tables().weapon(id)).map(|w| w.kind) {
                    Some(WeaponKind::Knife) => ts_message_all(GREEN, 1, 20, &format!("#TS_KnifeKill #TS_FrgFr {kname}!")),
                    Some(WeaponKind::Katana) => ts_message_all(GREEN, 1, 20, &format!("#TS_KatanaKill #TS_FrgFr {kname}!")),
                    _ => {}
                }
            }
            if double_kill {
                points *= 2;
                ts_message_all(GREEN, 1, 20, &format!("#TS_Doublefrag #TS_FrgFr {kname}!"));
            }
            if stopped_fury {
                if killed_specialist {
                    points += 5;
                    ts_message_all(GREEN, 1, 30, &format!("{points} #TS_Frg #TS_FrgFr {kname}, #TS_KilledTheSpecialist"));
                    if k.active == 0 && k.held == 0 {
                        k.held = pw::SLOWPAUSE;
                        k.held_secs = 2;
                    }
                }
                let n = eng.random_long(1, 10);
                ts_message_all(GREEN, 1, 30, &format!("{vname} #TS_SpreeKilled{n}"));
            }
            let mut spree_msg = None;
            if is_unstoppable {
                ts_message_all(GREEN, 1, 30, &format!("#TS_Unstoppable {kname}! {} #TS_ConsKills", k.spree));
                points *= 2;
            } else if is_specialist {
                ts_message_all(GREEN, 1, 0x37, &format!("{kname} #TS_Specialist"));
                give_specialist = true;
                if k.active == 0 && k.held == 0 {
                    k.held = pw::SLOWPAUSE;
                    k.held_secs = 2;
                }
                k.award = if k.award == 10 { 10 } else { 4 };
            } else if is_demolition {
                health_delta = Some(60.0);
                k.award = if k.award == 10 { 10 } else { 3 };
                spree_msg = Some((6, 0x2d));
            } else if is_killer {
                health_delta = Some(30.0);
                k.award = if k.award == 10 { 10 } else { 2 };
                spree_msg = Some((3, 0x23));
            }
            if let Some((n, hold)) = spree_msg {
                let r = eng.random_long(1, 16);
                ts_message_all(GREEN, 1, hold, &format!("{kname} #TS_KillSpree{r} {n} #TS_ConsKills"));
            }
            let kvm = k.e.v();
            if give_specialist {
                kvm.health = kvm.health.max(150.0);
                eng.set_physics_key(k.e, c"mj", c"1");
                msg::one(Msg::PwUp, k.e, |w| PwUp { bits: pw::KUNGFU as i16, secs: 0 }.write(w));
                msg::one(Msg::PwUp, k.e, |w| PwUp { bits: pw::SUPERJUMP as i16, secs: 0 }.write(w));
            }
            if let Some(h) = health_delta {
                kvm.health = (kvm.health + h).min(150.0);
            }
            let regen = kvm.health > 100.0;
            let cash = if points == -1 { -1000 } else { points * 500 + 400 };
            let last_kill = if double_kill { 0.0 } else { now };
            other(k.e, |kp| {
                if give_specialist {
                    kp.kungfu = true;
                    kp.superjump = true;
                }
                if regen {
                    kp.ts.health_timer = now + 1.0;
                }
                kp.ts.last_kill = last_kill;
                give_cash(kp, cash);
            });
            give_cash(p, 900);
        }

        // Frags (the killer's pev->frags) and team scores.
        super::add_frags(k.e, None, points as f32);
        teams::update_team_scores();
        crate::entities::fire_targets(c"game_playerkill", k.e, k.e, UseType::Toggle, 0.0);

        // slowmatch: each kill stores a second of slow motion (max 3).
        if cvars::f(c"slowmatch") == 1.0 && points > 0 && (k.held == 0 || k.held & pw::SLOWMO != 0) && k.active == 0 {
            k.held = pw::SLOWMO;
            k.held_secs = (k.held_secs + 1).min(3);
        }

        // Team kills: 3 within rolling 30 s windows → kick or ban.
        let mut kick = false;
        if points == -1 {
            other(k.e, |kp| {
                if kp.ts.tk_until == 0.0 || kp.ts.tk_until < now {
                    kp.ts.tk_count = 1;
                } else {
                    kp.ts.tk_count += 1;
                }
                kp.ts.tk_until = now + 30.0;
                kick = kp.ts.tk_count > 2;
            });
        }
        let (spree, award, held, held_secs) = (k.spree, k.award, k.held, k.held_secs);
        other(k.e, |kp| {
            kp.ts.spree = spree;
            kp.ts.award = award;
            kp.ts.held = held;
            kp.ts.held_secs = held_secs;
        });
        if kick {
            team_kill_punish(k.e, victim);
        }
        // Kills by The One's victims make the killer the new One.
        let _ = k.idx;
    }

    // ScoreInfo for the victim and the killer.
    let si = score_info_of(p, cx.pev(), vidx);
    msg::all(Msg::ScoreInfo, |w| si.write(w));
    if killer_is_player {
        let kidx = eng.index_of(attacker);
        if let Some(s) = other(attacker, |k| score_info_of(k, attacker.v(), kidx)) {
            msg::all(Msg::ScoreInfo, |w| s.write(w));
        }
    }
    state::with(|g| g.check_deads_at = now + 1.0);
}

/// `DoSpectacularNess`: `RandomLong(0,100) <= spectacularness`.
fn spectacular() -> bool {
    engine().random_long(0, 100) as f32 <= cvars::f(c"spectacularness")
}

/// `TSGetPointsForFrag` (0x89140).
#[allow(clippy::too_many_arguments)]
fn points_for_frag(
    d: &DamageInfo,
    attacker: *mut edict_t,
    victim: *mut edict_t,
    base: i32,
    killer_is_the_one: bool,
    spree: u8,
    last_kill: f32,
    sliding: &mut bool,
    diving: &mut bool,
    vname: &str,
    now: f32,
) -> i32 {
    if base < 0 {
        return base;
    }
    if state::the_one_mode() && !killer_is_the_one {
        return 0;
    }
    let _ = victim;
    let mut points = base;
    let mut desc = String::new();
    let close = is_close_combat(d);
    if close {
        desc = "#TS_KungFu".to_owned();
        points = 2;
    } else if !d.inflictor.is_null() && d.inflictor != attacker && classname(d.inflictor) == "knife" {
        desc = "Throwing".to_owned();
        points = 2;
    }
    let av = attacker.v();
    let speed = super::len3(av.velocity);
    if av.iuser4 & STUNTING != 0 && speed > 80.0 {
        if !close {
            points += 1;
        }
        if points > 1 {
            desc = if desc.is_empty() { "#TS_StStl".into() } else { desc + " & #TS_StStl" };
        }
        *diving = true;
    } else if av.iuser4 & stunt::PRONE != 0 && av.friction < 1.0 && speed > 80.0 {
        if !close {
            points += 1;
        }
        if points > 1 {
            desc = if desc.is_empty() { "#TS_SlStl".into() } else { desc + " & #TS_SlStl" };
        }
        *sliding = true;
    } else if !desc.is_empty() {
        desc += " #TS_Stl";
    }
    if points > 1 {
        let mut shown = points;
        if spree > 8 {
            shown *= 2;
        }
        if now - last_kill < 2.0 {
            shown *= 2;
        }
        ts_message_all(GREEN, 1, 30, &format!("{shown} #TS_Frg #TS_FrgFr {vname}, {desc}"));
    }
    points
}

/// `CHalfLifeMultiplay::DeathNotice` (0x88f7c): `DeathMsg` to all, then the HLTV director event
/// (victim, then the inflictor or else the killer, priority 0x27).
fn death_notice(attacker: *mut edict_t, inflictor: *mut edict_t, victim: *mut edict_t, weapon: &str) {
    let eng = engine();
    let killer = if !attacker.is_null() && attacker.v().flags & FL_CLIENT as i32 != 0 { eng.index_of(attacker) } else { 0 };
    let m = DeathMsg { killer: killer as u8, victim: eng.index_of(victim) as u8, weapon: weapon.to_owned() };
    msg::all(Msg::DeathMsg, |w| m.write(w));
    let secondary = if inflictor.is_null() { attacker } else { inflictor };
    msg::director_event(eng.index_of(victim), eng.index_of(secondary), director::FLAGS_DEATH);
}

#[allow(clippy::too_many_arguments)]
fn log_kill(
    attacker: *mut edict_t,
    victim: *mut edict_t,
    weapon: &str,
    suicide: bool,
    sliding: bool,
    diving: bool,
    double_kill: bool,
    killed_specialist: bool,
    fury: Option<&String>,
    killer: bool,
    demolition: bool,
    specialist: bool,
    unstoppable: bool,
) {
    let eng = engine();
    let id = |e: *mut edict_t| format!("\"{}<{}><{}><>\"", netname(e), eng.user_id(e), eng.auth_id(e).to_string_lossy());
    let w = weapon.to_ascii_lowercase().replace(' ', "_");
    if suicide {
        msg::log(&format!("{} committed suicide with \"{w}\"\n", id(victim)));
        return;
    }
    if attacker.v().flags & FL_CLIENT as i32 == 0 {
        msg::log(&format!("{} committed suicide with \"{w}\" (world)\n", id(victim)));
        return;
    }
    let spree = if unstoppable {
        " (is_unstoppable)"
    } else if specialist {
        " (is_specialist)"
    } else if demolition {
        " (is_demolition)"
    } else if killer {
        " (is_killer)"
    } else {
        ""
    };
    let fury = fury.map(|n| format!(" (stopped_fury \"{n}\")")).unwrap_or_default();
    msg::log(&format!(
        "{} killed {} with \"{w}\"{spree}{fury}{}{}{}{}\n",
        id(attacker),
        id(victim),
        if killed_specialist { " (killed_specialist)" } else { "" },
        if double_kill { " (double_kill)" } else { "" },
        if diving { " (diving)" } else { "" },
        if sliding { " (sliding)" } else { "" },
    ));
}

fn team_kill_punish(killer: *mut edict_t, victim: *mut edict_t) {
    let eng = engine();
    let name = netname(killer);
    let ban = cvars::f(c"banfortk");
    let uid = eng.user_id(killer);
    if ban <= 0.0 {
        msg::text_one(victim, shared::protocol::rules::hud_dest::CONSOLE, "You have been kicked for teamkilling...\n");
        eng.server_command(&format!("kick #{uid}\n"));
        msg::text_all(shared::protocol::rules::hud_dest::TALK, &format!("{name} kicked for teamkilling\n"));
    } else {
        let m = ban.round() as i32;
        msg::text_one(
            victim,
            shared::protocol::rules::hud_dest::CONSOLE,
            &format!("You have been banned for teamkilling ({m} minutes)...\n"),
        );
        eng.server_command(&format!("banid {m} #{uid} kick\n"));
        msg::text_all(shared::protocol::rules::hud_dest::TALK, &format!("{name} banned for teamkilling ({m} minutes)\n"));
    }
}

/// `CTSGameRules::TheOneIsDead` (0x7b258): whoever killed The One becomes the new One.
fn the_one_is_dead(p: &mut Player, cx: &mut Cx, killer: Option<*mut edict_t>) {
    p.ts.is_the_one = false;
    p.ts.award = 0;
    state::with(|g| g.the_one_exists = false);
    let _ = cx;
    if let Some(k) = killer {
        set_the_one_other(k);
    }
}

/// `SetPlayerTheOne` (0x7b078) on another (not dispatched) player.
pub fn set_the_one_other(e: *mut edict_t) {
    let v = e.v();
    if !super::alive(v) || v.iuser1 != 0 {
        return;
    }
    let ok = other(e, |p| {
        apply_the_one(p);
    })
    .is_some();
    if ok {
        announce_the_one(e);
    }
}

/// The state part of `SetPlayerTheOne`.
pub fn apply_the_one(p: &mut Player) {
    p.ts.is_the_one = true;
    p.kungfu = true;
    p.superjump = true;
    if p.ts.held == 0 && p.active_powerup == 0 {
        p.ts.held = pw::SLOWPAUSE;
        p.ts.held_secs = 3;
    }
    p.ts.award = 10;
    p.ts.health_timer = engine().time() + 1.0;
    state::with(|g| g.the_one_exists = true);
}

/// The engine part of `SetPlayerTheOne`: hp 150, super jump, speed 330, PwUp, announcement,
/// team switch to team 0.
pub fn announce_the_one(e: *mut edict_t) {
    let eng = engine();
    let v = e.v();
    v.health = v.health.max(150.0);
    v.maxspeed = 330.0;
    eng.set_client_maxspeed(e, 330.0);
    eng.set_physics_key(e, c"mj", c"1");
    msg::one(Msg::PwUp, e, |w| PwUp { bits: pw::KUNGFU as i16, secs: 0 }.write(w));
    msg::one(Msg::PwUp, e, |w| PwUp { bits: pw::SUPERJUMP as i16, secs: 0 }.write(w));
    ts_message_all([100, 150, 255], 1, 30, &format!("{} is The One!", netname(e)));
    teams::the_one_teams();
}

/// ScoreInfo for any player edict (dispatched or not is handled by the caller).
pub fn broadcast_score(e: *mut edict_t) {
    let idx = engine().index_of(e);
    if let Some(s) = other(e, |p| score_info_of(p, e.v(), idx)) {
        msg::all(Msg::ScoreInfo, |w| s.write(w));
    }
}
