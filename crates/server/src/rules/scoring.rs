//! `CHalfLifeMultiplay::PlayerKilled` (0x895e4), `TSGetPointsForFrag` (0x89140) and
//! `DeathNotice` (0x88f7c): points, sprees and awards, cash, kill messages, DeathMsg, ScoreInfo.
//! Decomp: gamerules-entities.md §2.3.

use super::lifecycle::{give_cash, score_info_of};
use super::msg::{self, ts_message_all};
use super::{cvars, netname, other, state, teams};
use crate::damage::DamageInfo;
use crate::damage::glast::Snapshot;
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

pub fn classname(e: *mut edict_t) -> String {
    if e.is_null() {
        return String::new();
    }
    engine().string(e.v().classname).to_string_lossy().into_owned()
}

/// `get_weapon_info(id)` (0x7f190) `+0x11`, the weapon's name: entry 0 for an id outside 0..0x26.
fn weapon_info_name(id: i32) -> String {
    let t = tables();
    u32::try_from(id)
        .ok()
        .filter(|&i| i < 0x26)
        .and_then(|i| t.weapon(i as u8))
        .or_else(|| t.weapon(0))
        .map(|w| w.name.clone())
        .unwrap_or_default()
}

/// The killer's active `weapon_tsgun` as `PlayerKilled` reads it (`pl+0xa10`, item id 0x10):
/// `Some(gun+0xc8)`, the current weapon id (0 = kung fu, `gun+0xcc` NULL), or `None` without the
/// gun active (a `game_player_equip` map, weapons.md §2, or a `cycler_weapon` switched to).
pub fn active_gun(p: &Player) -> Option<u8> {
    crate::weapons::gun_active(p).then_some(p.inv.gun.current)
}

/// The `DeathMsg`/log weapon when the attacker is its own inflictor (`PlayerKilled` 0x895e4):
/// a simulated bullet's `glast_weaponid`; else "Kung Fu" under
/// `glast_closecombat`; else the killer's current gun weapon (with " Stock" under
/// `glast_coldcock`); else "world" (kung fu outside `CloseCombatAttack`, no gun).
fn own_weapon_name(g: &Snapshot, killer_gun: Option<u8>) -> String {
    if g.weapon_id != -1 {
        weapon_info_name(g.weapon_id)
    } else if g.close_combat {
        "Kung Fu".to_owned()
    } else if let Some(cur) = killer_gun.filter(|&c| c != 0) {
        let name = weapon_info_name(cur as i32);
        if g.coldcock { name + " Stock" } else { name }
    } else {
        "world".to_owned()
    }
}

/// Weapon name for `DeathMsg` and logs (gamerules-entities.md §2.3). `killer_gun` is the
/// attacker's [`active_gun`].
pub fn weapon_name(d: &DamageInfo, g: &Snapshot, killer_gun: Option<u8>) -> String {
    // TS's `FL_CLIENT` test (0x8986d): still true for an attacker who has left.
    let attacker_client = !d.attacker.is_null() && super::ts_flags(d.attacker) & FL_CLIENT as i32 != 0;
    let mut name = if attacker_client {
        if d.inflictor.is_null() {
            "world".to_owned()
        } else if d.inflictor == d.attacker {
            own_weapon_name(g, killer_gun)
        } else {
            let cls = classname(d.inflictor);
            if cls == "knife" {
                // `CTSKnife::GetDescription`; without a knife object the name stays "world".
                crate::weapons::thrown_knife_description(d.inflictor).unwrap_or("world").to_owned()
            } else {
                cls
            }
        }
    } else if !d.inflictor.is_null() {
        // The netname when it is set at all (`string_t` non-zero), else the classname.
        let v = d.inflictor.v();
        if v.netname == 0 { classname(d.inflictor) } else { engine().string(v.netname).to_string_lossy().into_owned() }
    } else {
        // Guard: TS reads the classname through a NULL inflictor here.
        "world".to_owned()
    };
    for prefix in ["weapon_", "monster_", "func_"] {
        if let Some(rest) = name.strip_prefix(prefix) {
            name = rest.to_owned();
            break;
        }
    }
    rename_weapon(name)
}

/// The last step of the weapon name (0x89b40..0x89bdf): the three classnames TS spells out. A
/// thrown knife already carries its description, so "knife" only comes from an inflictor of a
/// non-player attacker. An empty name stays empty.
fn rename_weapon(name: String) -> String {
    match name.as_str() {
        "knife" => "Throwing Knife".to_owned(),
        "shrapnel" => "Shrapnel".to_owned(),
        "grenade" => "M61 Grenade".to_owned(),
        _ => name,
    }
}

/// `IPointsForKill`: 1, or -1 for a team kill.
pub fn points_for_kill(attacker: *mut edict_t, victim: *mut edict_t) -> i32 {
    if attacker.is_null() {
        return 0;
    }
    if state::teamplay() && attacker != victim && teams::same_team(attacker, victim) {
        return -1;
    }
    1
}

/// Snapshot of the killer's Rust state we need (read through `other`).
pub struct Killer {
    pub e: *mut edict_t,
    pub idx: i32,
    pub spree: u8,
    pub last_kill: f32,
    pub is_the_one: bool,
    pub award: u8,
    pub held: u16,
    pub held_secs: i32,
    pub active: u16,
    /// [`active_gun`].
    pub gun: Option<u8>,
    /// The killer's current weapon record was first owned by the victim ([`victims_gun`]).
    pub victims_gun: bool,
}

/// A kill by a player, as `TSGetPointsForFrag` sees it
/// ([`ServerRules::points_for_frag`](crate::gameplay::ServerRules::points_for_frag)).
pub struct Frag<'a> {
    pub damage: &'a DamageInfo,
    /// TS's damage globals as the kill left them.
    pub glast: &'a Snapshot,
    pub killer: &'a Killer,
    /// [`points_for_kill`]: 1, or -1 for a team kill or a suicide.
    pub base: i32,
    pub now: f32,
}

/// The `PlayerKilled` body. `p`/`cx` are the victim; `g` holds TS's damage globals as the kill
/// left them (`Killed` takes it first: they are reset once the hit's `ApplyMultiDamage` returns,
/// and this may run later, below).
pub fn player_killed(p: &mut Player, cx: &mut Cx, d: &DamageInfo, g: Snapshot) {
    // `CHalfLifeTeamplay::PlayerKilled` (0xe51b8) skips everything, the death notice included,
    // while `ChangePlayerTeam` kills the player.
    if state::with(|g| g.death_suppressed) {
        return;
    }
    let eng = engine();
    let attacker = if d.attacker.is_null() { eng.entity_of_index(0) } else { d.attacker };
    let victim = cx.edict;
    let suicide = attacker == victim;
    // `IsPlayer()` (0x896dc): any `CBasePlayer`, so a killer who has left (its grenade still
    // flying) is scored as one.
    let killer_is_player = !suicide && super::has_player(attacker);

    // The killer's own think (its weapon frame) is usually what killed: its Player is taken
    // out further up the stack. TS scores inside the `Killed`; the weapon frame, a tank shot and
    // a +use lend the killer to its slot (`world::lend`), so it is reached now. A killer held by
    // a dispatch that didn't lend it is scored as soon as that call returns.
    if killer_is_player && crate::world::is_taken_out(attacker) {
        // Opened only if it was parked: a lent killer whose own object is dispatched again (opened
        // further up and taken out once more) stays out of reach, and waits like an unlent one.
        let scored = crate::world::run_now(attacker, || {
            let open = !crate::world::is_taken_out(attacker);
            if open {
                player_killed(p, cx, d, g);
            }
            open
        });
        if scored == Some(true) {
            return;
        }
        let d = *d;
        crate::world::defer(attacker, move || {
            crate::player::with_player(victim, |p, cx| player_killed(p, cx, &d, g));
        });
        return;
    }
    crate::gameplay::active().server.player_killed(p, cx, d, g);
}

/// TS 3.0's `PlayerKilled` body once the killer can be reached
/// ([`ServerRules::player_killed`](crate::gameplay::ServerRules::player_killed)).
pub fn player_killed_core(p: &mut Player, cx: &mut Cx, d: &DamageInfo, g: Snapshot) {
    let eng = engine();
    let now = cx.time();
    let victim = cx.edict;
    let vidx = cx.index();
    let vname = netname(victim);
    let attacker = if d.attacker.is_null() { eng.entity_of_index(0) } else { d.attacker };
    let suicide = attacker == victim;
    let killer_is_player = !suicide && super::has_player(attacker);
    let close = g.close_combat;

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
            gun: active_gun(k),
            victims_gun: victims_gun(active_gun(k), |cur| k.inv.sv.owners[cur as usize], victim),
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
        let frag = Frag { damage: d, glast: &g, killer: k, base: points, now };
        points = crate::gameplay::active().server.points_for_frag(&frag, &mut sliding, &mut diving);
        double_kill = now - k.last_kill < 2.0;
        if p.ts.spree > 2 {
            stopped_fury = true;
            killed_specialist = p.ts.spree > 8;
        }
        k.spree = k.spree.saturating_add(1);
        match k.spree {
            3 => is_killer = true,
            6 => is_demolition = true,
            9 => is_specialist = true,
            s if s >= 10 => is_unstoppable = true,
            _ => {}
        }
    }

    // ` (victims_weapon)`: also only for a kill by the killer's own hand, outside
    // `glast_closecombat`.
    let victims_weapon = killer.as_ref().is_some_and(|k| k.victims_gun) && !close && d.inflictor == attacker;
    let attacker_gun =
        if suicide { active_gun(p) } else { killer.as_ref().map_or_else(|| other(attacker, |k| active_gun(k)).flatten(), |k| k.gun) };
    let weapon = weapon_name(d, &g, attacker_gun);
    death_notice(attacker, d.inflictor, victim, &weapon);
    let spree = if is_unstoppable {
        " (is_unstoppable)"
    } else if is_specialist {
        " (is_specialist)"
    } else if is_demolition {
        " (is_demolition)"
    } else if is_killer {
        " (is_killer)"
    } else {
        ""
    };
    let tags = KillTags {
        victims_weapon,
        stopped_fury: stopped_fury.then(|| vname.clone()),
        killed_specialist,
        double_kill,
        diving,
        sliding,
        killed_the_one: p.ts.is_the_one,
        spree,
    };
    log_kill(attacker, victim, &weapon, suicide, &tags);

    p.deaths += 1;
    // Lent back to its slot, so a `game_player_team` it reaches changes our team inside the
    // `Use`, as in TS.
    crate::world::lend(victim, p, || crate::entities::fire_targets(c"game_playerdie", victim, victim, UseType::Toggle, 0.0));
    if p.ts.is_the_one {
        the_one_is_dead(p, cx, killer.as_ref().map(|k| k.e));
        p.ts.spree = 0;
        // `SetPlayerTheOne` may have just changed the killer (award, stored slow pause): the spree
        // rewards below read and write those fields after it, as TS does.
        if let Some(k) = killer.as_mut()
            && let Some((award, held, held_secs, active)) = other(k.e, |kp| (kp.ts.award, kp.ts.held, kp.ts.held_secs, kp.active_powerup))
        {
            (k.award, k.held, k.held_secs, k.active) = (award, held, held_secs, active);
        }
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
        if (kv.iuser4 & STUNTING != 0 || close || points > 2) && do_spectacularness() {
            crate::entities::ts::create_slow_motion_point(cx.pev().origin, 0.25, 1.0, false);
        }
        let mut health_delta: Option<f32> = None; // (+hp, cap 150) for awards
        let mut give_specialist = false;
        if points > 0 {
            // `glast_headshot`: any bullet or thrown head hit since the last ApplyMultiDamage, so
            // also another victim's from the same shotgun shot.
            if g.headshot {
                ts_message_all(GREEN, 1, 20, &format!("#TS_Headshot #TS_FrgFr {kname}!"));
            }
            // The killer's current gun weapon decides (`get_weapon_info(gun+0xc8)+0xd2`), not
            // what dealt the damage.
            if d.inflictor == attacker && !close {
                match k.gun.filter(|&c| c != 0).and_then(|c| tables().weapon(c)).map(|w| w.kind) {
                    Some(WeaponKind::Knife) => ts_message_all(GREEN, 1, 20, &format!("#TS_KnifeKill #TS_FrgFr {kname}!")),
                    Some(WeaponKind::Katana) => ts_message_all(GREEN, 1, 20, &format!("#TS_KatanaKill #TS_FrgFr {kname}!")),
                    _ => {}
                }
            }
            if g.coldcock {
                ts_message_all(GREEN, 1, 20, &format!("#TS_Coldcock #TS_FrgFr {kname}!"));
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

        // Frags (the killer's pev->frags).
        super::add_frags(k.e, None, points as f32);
        crate::entities::fire_targets(c"game_playerkill", k.e, k.e, UseType::Toggle, 0.0);

        // slowmatch: each kill stores a second of slow motion (max 3).
        if cvars::f(c"slowmatch") == 1.0 && points > 0 && (k.held == 0 || k.held & pw::SLOWMO != 0) && k.active == 0 {
            k.held = pw::SLOWMO;
            k.held_secs = (k.held_secs + 1).min(3);
        }
        // mp.dll (0x1005b18a): a bot that scored the kill may taunt the victim (`CBot::OnKill`).
        if points > 0 {
            other(k.e, |kp| crate::bots::on_kill(kp, victim, now));
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
    state::with(|g| g.call_check_deads(now));
    // `CHalfLifeTeamplay::PlayerKilled` (0xe51b8): `RecountTeams(false)` after every kill, so the
    // team frags the fraglimit reads are those of the last recount.
    if state::teamplay() {
        teams::recount(false);
    }
}

/// `DoSpectacularNess` (0x76b48): whether a frag or an explosion creates the "spectacular"
/// slow-motion point. The roll is drawn first, as in TS, so the random stream is the same.
pub fn do_spectacularness() -> bool {
    let roll = engine().random_long(0, 100);
    spectacular_roll(roll, cvars::f(c"spectacularness"))
}

/// The `DoSpectacularNess` test: `spectacularness` is clamped to 0..100 and truncated (`(int)`,
/// 0x76bb6). 0 never fires; otherwise it fires when `roll <= spectacularness`.
fn spectacular_roll(roll: i32, spectacularness: f32) -> bool {
    let pct = super::x87_trunc(spectacularness.clamp(0.0, 100.0) as f64);
    pct != 0 && roll <= pct
}

/// `TSGetPointsForFrag`'s `#TS_KungFu` test for a killer that is its own inflictor:
/// `glast_weaponid == -1`, then without an active item `glast_closecombat`, with the gun
/// kung fu out (`gun+0xcc` NULL) or `glast_closecombat`.
fn kung_fu_frag(g: &Snapshot, killer_gun: Option<u8>) -> bool {
    g.weapon_id == -1
        && match killer_gun {
            None => g.close_combat,
            Some(cur) => cur == 0 || g.close_combat,
        }
}

/// `TSGetPointsForFrag` (0x89140) for a killing player `k`: 2 points and `#TS_KungFu` when the
/// attacker is its own inflictor, outside a simulated bullet's hit, and either holds no gun under
/// `glast_closecombat` or has kung fu out (or is under `glast_closecombat`); 2 points and the
/// knife's name for a thrown knife. Any other inflictor returns the base points at once (no
/// stunt bonus, no message). Then +1 for a stunt or a slide above 80 u/s (not under
/// `glast_closecombat`), and the frag message, which names the killer.
pub fn points_for_frag(frag: &Frag<'_>, sliding: &mut bool, diving: &mut bool) -> i32 {
    let Frag { damage: d, glast: g, killer: k, base, now } = *frag;
    if base < 0 {
        return base;
    }
    if state::the_one_mode() && !k.is_the_one {
        return 0;
    }
    let attacker = k.e;
    let close = g.close_combat;
    let mut points = base;
    let mut desc = String::new();
    if !d.inflictor.is_null() {
        if d.inflictor == attacker {
            if kung_fu_frag(g, k.gun) {
                desc = "#TS_KungFu".to_owned();
                points = 2;
            }
        } else {
            if classname(d.inflictor) != "knife" {
                return points;
            }
            // `CTSKnife::GetDescription` (0x7db04), "Throwing" without a knife object.
            desc = crate::weapons::thrown_knife_description(d.inflictor).unwrap_or("Throwing").to_owned();
            points = 2;
        }
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
        if k.spree > 8 {
            shown *= 2;
        }
        if now - k.last_kill < 2.0 {
            shown *= 2;
        }
        ts_message_all(GREEN, 1, 30, &format!("{shown} #TS_Frg #TS_FrgFr {}, {desc}", netname(attacker)));
    }
    points
}

/// `CHalfLifeMultiplay::DeathNotice` (0x88f7c): `DeathMsg` to all, then the HLTV director event
/// (victim, then the inflictor or else the killer, priority 0x27). In teamplay
/// `CHalfLifeTeamplay::DeathNotice` (0xe509c) comes first: a kill by a teammate (`FL_CLIENT`,
/// `PlayerRelationship == GR_TEAMMATE`) is only `DeathMsg(killer, victim, "teammate")`.
pub fn death_notice(attacker: *mut edict_t, inflictor: *mut edict_t, victim: *mut edict_t, weapon: &str) {
    let eng = engine();
    let attacker_client = !attacker.is_null() && super::ts_flags(attacker) & FL_CLIENT as i32 != 0;
    if state::teamplay() && attacker_client && attacker != victim && teams::same_team(victim, attacker) {
        let m = DeathMsg { killer: eng.index_of(attacker) as u8, victim: eng.index_of(victim) as u8, weapon: "teammate".to_owned() };
        msg::all(Msg::DeathMsg, |w| m.write(w));
        return;
    }
    let killer = if attacker_client { eng.index_of(attacker) } else { 0 };
    let m = DeathMsg { killer: killer as u8, victim: eng.index_of(victim) as u8, weapon: weapon.to_owned() };
    msg::all(Msg::DeathMsg, |w| m.write(w));
    let secondary = if inflictor.is_null() { attacker } else { inflictor };
    msg::director_event(eng.index_of(victim), eng.index_of(secondary), director::FLAGS_DEATH);
}

/// The kill's log tags, in `PlayerKilled`'s order (0x89f0d..0x89fa4).
#[derive(Default)]
struct KillTags {
    victims_weapon: bool,
    /// The victim's name when the kill ended a spree of three or more.
    stopped_fury: Option<String>,
    killed_specialist: bool,
    double_kill: bool,
    diving: bool,
    sliding: bool,
    /// The victim was The One (`pl+0x7b9`, read before `TheOneIsDead`).
    killed_the_one: bool,
    /// " (is_killer)", " (is_demolition)", " (is_specialist)", " (is_unstoppable)" or "".
    spree: &'static str,
}

impl KillTags {
    fn suffix(&self) -> String {
        let flag = |on: bool, s: &str| if on { s.to_owned() } else { String::new() };
        let fury = self.stopped_fury.as_ref().map(|n| format!(" (stopped_fury \"{n}\")")).unwrap_or_default();
        [
            flag(self.victims_weapon, " (victims_weapon)"),
            fury,
            flag(self.killed_specialist, " (killed_specialist)"),
            flag(self.double_kill, " (double_kill)"),
            flag(self.diving, " (diving)"),
            flag(self.sliding, " (sliding)"),
            flag(self.killed_the_one, " (killed_the_one)"),
            self.spree.to_owned(),
        ]
        .concat()
    }
}

/// `"name<userid><authid><x>"` of a log line: `x` is the user id again outside teamplay, the
/// `model` key in teamplay.
fn log_id(name: &str, uid: i32, auth: &str, model: Option<&str>) -> String {
    match model {
        Some(m) => format!("\"{name}<{uid}><{auth}><{m}>\""),
        None => format!("\"{name}<{uid}><{auth}><{uid}>\""),
    }
}

/// The log weapon: A-Z lowered and spaces made underscores (0x89c28..0x89c5f).
fn log_weapon(weapon: &str) -> String {
    weapon.to_ascii_lowercase().replace(' ', "_")
}

/// `PlayerKilled`'s log line (0x89c61..0x8a2bb): a suicide, a death by something that isn't a
/// client (" (world)"), or the kill with its tags.
/// [`log_kill`] without any tags.
pub fn log_kill_untagged(attacker: *mut edict_t, victim: *mut edict_t, weapon: &str, suicide: bool) {
    log_kill(attacker, victim, weapon, suicide, &KillTags::default());
}

fn log_kill(attacker: *mut edict_t, victim: *mut edict_t, weapon: &str, suicide: bool, tags: &KillTags) {
    let eng = engine();
    let teamplay = state::teamplay();
    let id = |e: *mut edict_t| {
        let model = teamplay.then(|| eng.info_key(e, c"model").to_string_lossy().into_owned());
        log_id(&netname(e), eng.user_id(e), &eng.auth_id(e).to_string_lossy(), model.as_deref())
    };
    let w = log_weapon(weapon);
    if suicide {
        msg::log(&format!("{} committed suicide with \"{w}\"\n", id(victim)));
        return;
    }
    if super::ts_flags(attacker) & FL_CLIENT as i32 == 0 {
        msg::log(&format!("{} committed suicide with \"{w}\" (world)\n", id(victim)));
        return;
    }
    msg::log(&format!("{} killed {} with \"{w}\"{}\n", id(attacker), id(victim), tags.suffix()));
}

/// `PlayerKilled`'s ` (victims_weapon)` gun test (0x89703..0x8975c): the killer holds a weapon
/// record (`gun+0xcc`) whose original owner (`s_weapon_status+0x30`, `owner_of`) is the victim,
/// and `GetWeaponType` isn't 2, 3 or 11 (grenades, blades; 11 has no weapon). `cur` is
/// [`active_gun`].
fn victims_gun(cur: Option<u8>, owner_of: impl FnOnce(u8) -> *mut edict_t, victim: *mut edict_t) -> bool {
    use shared::weapons::{kind, restriction_type};
    let Some(cur) = cur.filter(|&c| c != 0 && (c as usize) < shared::weapons::MAX_WEAPONS) else { return false };
    owner_of(cur) == victim && !matches!(restriction_type(cur), kind::GRENADE | kind::KNIFE | kind::FLAG)
}

pub fn team_kill_punish(killer: *mut edict_t, victim: *mut edict_t) {
    let eng = engine();
    let name = netname(killer);
    let ban = cvars::f(c"banfortk");
    let uid = eng.user_id(killer);
    if ban <= 0.0 {
        msg::text_one(victim, shared::protocol::rules::hud_dest::CONSOLE, "You have been kicked for teamkilling...\n");
        eng.server_command(&format!("kick #{uid}\n"));
        msg::text_all(shared::protocol::rules::hud_dest::TALK, &format!("{name} kicked for teamkilling\n"));
    } else {
        // `(int)banfortk.value`, truncated (0x8ad5a, 0x8adbc, 0x8ae04).
        let m = super::x87_trunc(ban as f64);
        msg::text_one(
            victim,
            shared::protocol::rules::hud_dest::CONSOLE,
            &format!("You have been banned for teamkilling ({m} minutes)...\n"),
        );
        eng.server_command(&format!("banid {m} #{uid} kick\n"));
        msg::text_all(shared::protocol::rules::hud_dest::TALK, &format!("{name} banned for teamkilling ({m} minutes)\n"));
    }
}

/// `CTSGameRules::TheOneIsDead` (0x7b258) for the victim `p`; `killer` is the killing player
/// (`Classify() == CLASS_PLAYER`, `None` otherwise or for a suicide). In The One mode a kill by
/// another player makes the killer the new One: the victim's team switch, `SetPlayerTheOne` on
/// the killer, then the killer's team switch.
pub fn the_one_is_dead(p: &mut Player, cx: &mut Cx, killer: Option<*mut edict_t>) {
    state::with(|g| g.the_one_exists = false);
    p.ts.is_the_one = false;
    if !state::the_one_mode() {
        return;
    }
    match killer.filter(|&k| k != cx.edict) {
        None => ts_message_all(THE_ONE_RGB, 1, 30, "The One is dead."),
        Some(k) => {
            ts_message_all(THE_ONE_RGB, 1, 30, &format!("The One was killed by {}!", netname(k)));
            teams::the_one_switch(p, cx);
            set_player_the_one_other(k);
            teams::the_one_switch_other(k);
        }
    }
}

const THE_ONE_RGB: [u8; 3] = [100, 150, 255];

/// `CTSGameRules::SetPlayerTheOne` (0x7b078) on a player other than the one being dispatched.
pub fn set_player_the_one_other(e: *mut edict_t) {
    crate::player::with_player(e, set_player_the_one);
}

/// `CTSGameRules::SetPlayerTheOne` as the ruleset has it
/// ([`ServerRules::set_player_the_one`](crate::gameplay::ServerRules::set_player_the_one)).
pub fn set_player_the_one(p: &mut Player, cx: &mut Cx) {
    crate::gameplay::active().server.set_player_the_one(p, cx);
}

/// TS 3.0's `CTSGameRules::SetPlayerTheOne` (0x7b078): only a living, non-observing player. Health 150,
/// kung fu and super jump, speed 330, a stored 3 s slow pause when nothing is held or active,
/// award 10, the announcement and the log line. The team switch is the caller's.
pub fn set_player_the_one_core(p: &mut Player, cx: &mut Cx) {
    if !super::alive(cx.pev()) || cx.pev().iuser1 != 0 {
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
    super::set_client_maxspeed(e, 330.0);
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
    ts_message_all(THE_ONE_RGB, 1, 30, &format!("{name} is The One!"));
    msg::log(&format!("Player \"{name}\" triggered \"the_one\"\n"));
}

/// ScoreInfo for any player edict (dispatched or not is handled by the caller).
pub fn broadcast_score(e: *mut edict_t) {
    let idx = engine().index_of(e);
    if let Some(s) = other(e, |p| score_info_of(p, e.v(), idx)) {
        msg::all(Msg::ScoreInfo, |w| s.write(w));
    }
}

#[cfg(test)]
mod tests {
    use super::{
        KillTags, Snapshot, kung_fu_frag, log_id, log_weapon, own_weapon_name, rename_weapon, spectacular_roll, victims_gun,
        weapon_info_name,
    };
    use goldsrc::sys::edict_t;
    use shared::weapons::{id, tables};

    #[test]
    fn weapon_names_ts_spells_out() {
        assert_eq!(rename_weapon("knife".into()), "Throwing Knife");
        assert_eq!(rename_weapon("shrapnel".into()), "Shrapnel");
        assert_eq!(rename_weapon("grenade".into()), "M61 Grenade");
        // Only those three: the grenade's spawn name and an empty name pass through.
        assert_eq!(rename_weapon("m61_grenade".into()), "m61_grenade");
        assert_eq!(rename_weapon(String::new()), "");
        assert_eq!(rename_weapon("Throwing Katana".into()), "Throwing Katana");
    }

    #[test]
    fn kill_log_tags_keep_ts_order() {
        let all = KillTags {
            victims_weapon: true,
            stopped_fury: Some("Bob".into()),
            killed_specialist: true,
            double_kill: true,
            diving: true,
            sliding: true,
            killed_the_one: true,
            spree: " (is_killer)",
        };
        assert_eq!(
            all.suffix(),
            " (victims_weapon) (stopped_fury \"Bob\") (killed_specialist) (double_kill) (diving) (sliding) (killed_the_one) (is_killer)"
        );
        assert_eq!(KillTags::default().suffix(), "");
        assert_eq!(KillTags { victims_weapon: true, ..Default::default() }.suffix(), " (victims_weapon)");
    }

    #[test]
    fn log_ids_repeat_the_user_id_outside_teamplay() {
        assert_eq!(log_id("Bob", 7, "STEAM_0:1:2", None), "\"Bob<7><STEAM_0:1:2><7>\"");
        assert_eq!(log_id("Bob", 7, "BOT", Some("merc")), "\"Bob<7><BOT><merc>\"");
        assert_eq!(log_weapon("Throwing Combat Knife"), "throwing_combat_knife");
    }

    #[test]
    fn victims_weapon_needs_the_victims_gun_in_hand() {
        let victim: *mut edict_t = std::ptr::without_provenance_mut(0x100);
        let other: *mut edict_t = std::ptr::without_provenance_mut(0x200);
        assert!(victims_gun(Some(id::BERETTA), |_| victim, victim));
        assert!(!victims_gun(Some(id::BERETTA), |_| other, victim));
        // Kung fu, no gun, grenades and blades never count. (The CTB case has no weapon info in
        // TS, so its type is 0 and it would.)
        assert!(!victims_gun(Some(0), |_| victim, victim));
        assert!(!victims_gun(None, |_| victim, victim));
        assert!(victims_gun(Some(id::BRIEFCASE), |_| victim, victim));
        for w in [id::M61, id::KNIFE, id::SEAL_KNIFE, id::KATANA] {
            assert!(!victims_gun(Some(w), |_| victim, victim), "weapon {w}");
        }
    }

    fn name(id: u8) -> String {
        tables().weapon(id).unwrap().name.clone()
    }

    #[test]
    fn own_weapon_follows_the_ts_globals() {
        let none = Snapshot::default();
        let close = Snapshot { close_combat: true, ..none };
        let coldcock = Snapshot { coldcock: true, ..none };
        // A simulated bullet names its own weapon, whatever is held now.
        assert_eq!(own_weapon_name(&Snapshot { weapon_id: 12, ..close }, Some(0)), name(12));
        // CloseCombatAttack: kung fu even with a gun out (a delayed kick after switching).
        assert_eq!(own_weapon_name(&close, Some(12)), "Kung Fu");
        assert_eq!(own_weapon_name(&close, None), "Kung Fu");
        // A slash or hitscan shot: the weapon in hand; the stock hit says so.
        assert_eq!(own_weapon_name(&none, Some(12)), name(12));
        assert_eq!(own_weapon_name(&coldcock, Some(12)), name(12) + " Stock");
        // Kung fu outside CloseCombatAttack (a dive through glass), or no gun: "world".
        assert_eq!(own_weapon_name(&none, Some(0)), "world");
        assert_eq!(own_weapon_name(&coldcock, None), "world");
    }

    #[test]
    fn weapon_info_falls_back_to_entry_zero() {
        assert_eq!(weapon_info_name(-1), name(0));
        assert_eq!(weapon_info_name(0x26), name(0));
        assert_eq!(weapon_info_name(1), name(1));
    }

    #[test]
    fn kung_fu_frag_needs_close_combat_or_bare_hands() {
        let none = Snapshot::default();
        let close = Snapshot { close_combat: true, ..none };
        assert!(kung_fu_frag(&close, None));
        assert!(!kung_fu_frag(&none, None), "no gun, no close combat");
        assert!(kung_fu_frag(&none, Some(0)), "kung fu out");
        assert!(kung_fu_frag(&close, Some(12)));
        assert!(!kung_fu_frag(&none, Some(12)), "a knife or katana slash is no kung-fu frag");
        assert!(!kung_fu_frag(&Snapshot { weapon_id: 12, ..close }, Some(0)), "a simulated bullet");
    }

    #[test]
    fn spectacularness_zero_never_fires() {
        for v in [0.0, 0.4, 0.5, 0.99, -3.0] {
            assert!((0..=100).all(|roll| !spectacular_roll(roll, v)), "spectacularness {v}");
        }
    }

    #[test]
    fn spectacularness_truncates_and_clamps() {
        // `(int)` truncates: 1.9 is 1 (rolls 0 and 1 fire), 2.5 is 2, 3.5 is 3.
        assert!(spectacular_roll(1, 1.9) && !spectacular_roll(2, 1.9));
        assert!(spectacular_roll(2, 2.5) && !spectacular_roll(3, 2.5));
        assert!(spectacular_roll(3, 3.5) && !spectacular_roll(4, 3.5));
        // Above 100 behaves as 100: every roll fires.
        assert!((0..=100).all(|roll| spectacular_roll(roll, 250.0)));
    }
}
