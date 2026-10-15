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
use crate::weapons::Active;
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

/// How a ruleset spells two kinds of kill in `DeathMsg` and the log. TS 3.0's is
/// [`WeaponNaming::DEFAULT`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WeaponNaming {
    /// A thrown blade (a `knife` inflictor) is named by its description, `CTSKnife::GetDescription`
    /// ("Throwing Seal Knife"); otherwise by its classname, which becomes "Throwing Knife".
    pub describe_thrown_blades: bool,
    /// A stock hit (`glast_coldcock`) is "<weapon> Stock" (`strcat(name, " Stock")`); otherwise
    /// just "Stock".
    pub stock_names_weapon: bool,
}

impl WeaponNaming {
    /// TS 3.0's `PlayerKilled` (0x895e4).
    pub const DEFAULT: Self = Self { describe_thrown_blades: true, stock_names_weapon: true };
}

/// The `DeathMsg`/log weapon when the attacker is its own inflictor (`PlayerKilled` 0x895e4):
/// a simulated bullet's `glast_weaponid`; else "Kung Fu" under
/// `glast_closecombat`; else the killer's current gun weapon (a stock hit under
/// `glast_coldcock`, spelled as `naming` says); else "world" (kung fu outside
/// `CloseCombatAttack`, no gun).
fn own_weapon_name(g: &Snapshot, killer_gun: Option<u8>, naming: WeaponNaming) -> String {
    if g.weapon_id != -1 {
        weapon_info_name(g.weapon_id)
    } else if g.close_combat {
        "Kung Fu".to_owned()
    } else if let Some(cur) = killer_gun.filter(|&c| c != 0) {
        match (g.coldcock, naming.stock_names_weapon) {
            (false, _) => weapon_info_name(cur as i32),
            (true, true) => weapon_info_name(cur as i32) + " Stock",
            (true, false) => "Stock".to_owned(),
        }
    } else {
        "world".to_owned()
    }
}

/// A thrown blade's name before [`rename_weapon`]: its description (without a knife object
/// "world"), or its classname.
fn thrown_blade_name(cls: String, naming: WeaponNaming, describe: impl FnOnce() -> Option<&'static str>) -> String {
    if naming.describe_thrown_blades { describe().unwrap_or("world").to_owned() } else { cls }
}

/// Weapon name for `DeathMsg` and logs (gamerules-entities.md §2.3). `killer_gun` is the
/// attacker's [`active_gun`].
pub fn weapon_name(d: &DamageInfo, g: &Snapshot, killer_gun: Option<u8>) -> String {
    weapon_name_with(d, g, killer_gun, WeaponNaming::DEFAULT)
}

/// [`weapon_name`] with a ruleset's spelling.
pub fn weapon_name_with(d: &DamageInfo, g: &Snapshot, killer_gun: Option<u8>, naming: WeaponNaming) -> String {
    // TS's `FL_CLIENT` test (0x8986d): still true for an attacker who has left.
    let attacker_client = !d.attacker.is_null() && super::ts_flags(d.attacker) & FL_CLIENT as i32 != 0;
    let mut name = if attacker_client {
        if d.inflictor.is_null() {
            "world".to_owned()
        } else if d.inflictor == d.attacker {
            own_weapon_name(g, killer_gun, naming)
        } else {
            let cls = classname(d.inflictor);
            if cls == "knife" { thrown_blade_name(cls, naming, || crate::weapons::thrown_knife_description(d.inflictor)) } else { cls }
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
    /// What `m_pActiveItem` is (the gun, another item such as a `cycler_weapon`, or nothing).
    pub item: Active,
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
            item: crate::weapons::active_item(k),
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
            ts_message_all(GREEN, 1, 30, &shared::bformat!("{}, #TS_SpecialistSuicide", vname));
        }
        match frag_loser(suicide) {
            FragLoser::Victim => super::add_frags(victim, Some(p), -1.0),
            FragLoser::Killer => super::add_frags(attacker, None, -1.0),
        }
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
                ts_message_all(GREEN, 1, 20, &shared::bformat!("#TS_Headshot #TS_FrgFr {}!", kname));
            }
            // The killer's current gun weapon decides (`get_weapon_info(gun+0xc8)+0xd2`), not
            // what dealt the damage.
            if d.inflictor == attacker && !close {
                match k.gun.filter(|&c| c != 0).and_then(|c| tables().weapon(c)).map(|w| w.kind) {
                    Some(WeaponKind::Knife) => ts_message_all(GREEN, 1, 20, &shared::bformat!("#TS_KnifeKill #TS_FrgFr {}!", kname)),
                    Some(WeaponKind::Katana) => ts_message_all(GREEN, 1, 20, &shared::bformat!("#TS_KatanaKill #TS_FrgFr {}!", kname)),
                    _ => {}
                }
            }
            if g.coldcock {
                ts_message_all(GREEN, 1, 20, &shared::bformat!("#TS_Coldcock #TS_FrgFr {}!", kname));
            }
            if double_kill {
                points *= 2;
                ts_message_all(GREEN, 1, 20, &shared::bformat!("#TS_Doublefrag #TS_FrgFr {}!", kname));
            }
            if stopped_fury {
                if killed_specialist {
                    points += 5;
                    ts_message_all(GREEN, 1, 30, &shared::bformat!("{} #TS_Frg #TS_FrgFr {}, #TS_KilledTheSpecialist", points, kname));
                    if k.active == 0 && k.held == 0 {
                        k.held = pw::SLOWPAUSE;
                        k.held_secs = 2;
                    }
                }
                let n = eng.random_long(1, 10);
                ts_message_all(GREEN, 1, 30, &shared::bformat!("{} #TS_SpreeKilled{}", vname, n));
            }
            let mut spree_msg = None;
            if is_unstoppable {
                ts_message_all(GREEN, 1, 30, &shared::bformat!("#TS_Unstoppable {}! {} #TS_ConsKills", kname, k.spree));
                points *= 2;
            } else if is_specialist {
                ts_message_all(GREEN, 1, 0x37, &shared::bformat!("{} #TS_Specialist", kname));
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
                ts_message_all(GREEN, 1, hold, &shared::bformat!("{} #TS_KillSpree{} {} #TS_ConsKills", kname, r, n));
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
        // ` (victims_weapon)` is also announced, for every such kill (0x7ab45 file address).
        if victims_weapon {
            ts_message_all(GREEN, 1, 30, &own_weapon_message(&kname, &vname));
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
            other(k.e, |kp| crate::bot_backend::on_kill(kp, victim, now));
        }
        // Port addition: plugins hear of every kill a player scored.
        crate::plugins::on_kill(k.e, victim, points);

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
            team_kill_punish(k.e);
        }
        // Kills by The One's victims make the killer the new One.
        let _ = k.idx;
    }

    // ScoreInfo for the victim and the killer.
    let si = score_info_of(p, cx.pev(), vidx);
    msg::all(Msg::ScoreInfo, |w| si.write(w));
    killer_score_info(p, cx, attacker, now);
    state::with(|g| g.call_check_deads(now));
    // `CHalfLifeTeamplay::PlayerKilled` (0xe51b8): `RecountTeams(false)` after every kill, so the
    // team frags the fraglimit reads are those of the last recount.
    if state::teamplay() {
        teams::recount(false);
    }
}

/// `PlayerKilled`'s announcement of a kill with the victim's own gun (string 0x108d00); the
/// client's titles.txt resolves the keys ("X killed Y with his own weapon").
fn own_weapon_message(killer: &[u8], victim: &[u8]) -> Vec<u8> {
    shared::bformat!("{} #TS_Killed {} #TS_OwnWeapon", killer, victim)
}

/// Who gets the killer's `ScoreInfo` at the end of `PlayerKilled`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum KillerScore {
    Nobody,
    /// A suicide: the victim, a second time.
    Victim,
    /// Another player.
    Killer,
}

/// The test is `pKiller`'s `Classify() == CLASS_PLAYER` with no `pVictim != pKiller` check (3.0
/// 0x7affb file address, 2.1 0x1005004b), so a suicide sends the victim's `ScoreInfo` twice.
fn killer_score(suicide: bool, attacker_is_player: bool) -> KillerScore {
    if suicide {
        KillerScore::Victim
    } else if attacker_is_player {
        KillerScore::Killer
    } else {
        KillerScore::Nobody
    }
}

/// The SDK tail of `PlayerKilled` ("killers score, if it's a player"), the same in 3.0 and 2.1:
/// the killer's `ScoreInfo`, and `m_flNextDecalTime = time` (3.0 `pl+0xb44`, 2.1 `pl+0xa9c`), so
/// a player may spray at once after every kill or suicide. `p`/`cx` are the victim.
pub fn killer_score_info(p: &mut Player, cx: &mut Cx, attacker: *mut edict_t, now: f32) {
    match killer_score(attacker == cx.edict, super::has_player(attacker)) {
        KillerScore::Nobody => {}
        KillerScore::Victim => {
            let idx = cx.index();
            let s = score_info_of(p, cx.pev(), idx);
            msg::all(Msg::ScoreInfo, |w| s.write(w));
            p.impulse.next_decal_time = now;
        }
        KillerScore::Killer => {
            let kidx = engine().index_of(attacker);
            let s = other(attacker, |k| {
                k.impulse.next_decal_time = now;
                score_info_of(k, attacker.v(), kidx)
            });
            if let Some(s) = s {
                msg::all(Msg::ScoreInfo, |w| s.write(w));
            }
        }
    }
}

/// Whose `frags` a death no player scored lowers by one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FragLoser {
    Victim,
    Killer,
}

/// TS takes the frag off `pKiller` (0x8aedc: `pKiller->frags -= 1`, the HL SDK's "killed by the
/// world" branch), which is the victim only for a suicide: a fall, a `trigger_hurt`, a door or a
/// barrel lowers the world's or the trigger's `frags`, and the victim keeps its own.
fn frag_loser(suicide: bool) -> FragLoser {
    if suicide { FragLoser::Victim } else { FragLoser::Killer }
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

/// `TSGetPointsForFrag`'s kung fu test on the killer's `m_pActiveItem` (3.0 `pl+0xa10`, 2.1
/// `pl+0x968`), the same in both versions: without an active item, `glast_closecombat`; with the
/// gun (`m_iId == 0x10`), kung fu out (`gun+0xcc` NULL, `gun_current` 0) or `glast_closecombat`;
/// any other item (a `cycler_weapon`) never, so its kills score the base points.
/// `glast_closecombat` is also set by a stunt's hit (`CTSStunt::CheckForBreakables` 0x9bd9c).
pub fn active_item_kung_fu(item: Active, gun_current: Option<u8>, close_combat: bool) -> bool {
    match item {
        Active::None => close_combat,
        Active::Gun => gun_current.is_none_or(|c| c == 0) || close_combat,
        Active::Cycler => false,
    }
}

/// `TSGetPointsForFrag`'s `#TS_KungFu` test for a killer that is its own inflictor:
/// `TSGetPointsForFrag`'s `pev->velocity.Length() > 80` (both tests): the squares `x, y, z`, the
/// root unrounded in mp.dll (0x100599f3..0x10059a1c), stored in `ts_i386.so` (0x893b9..0x893de).
fn frag_fast<F: shared::pm::Fpu>(velocity: [f32; 3]) -> bool {
    use shared::pm::FpuMath;
    F::vec_length(velocity, shared::pm::ORDER_XYZ) > F::x(80.0)
}

/// `glast_weaponid == -1`, then [`active_item_kung_fu`].
fn kung_fu_frag(g: &Snapshot, item: Active, killer_gun: Option<u8>) -> bool {
    g.weapon_id == -1 && active_item_kung_fu(item, killer_gun, g.close_combat)
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
            if kung_fu_frag(g, k.item, k.gun) {
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
    let fast = shared::by_arith!(cvars::pm_arith(), frag_fast(av.velocity));
    if av.iuser4 & STUNTING != 0 && fast {
        if !close {
            points += 1;
        }
        if points > 1 {
            desc = if desc.is_empty() { "#TS_StStl".into() } else { desc + " & #TS_StStl" };
        }
        *diving = true;
    } else if av.iuser4 & stunt::PRONE != 0 && av.friction < 1.0 && fast {
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
        ts_message_all(GREEN, 1, 30, &shared::bformat!("{} #TS_Frg #TS_FrgFr {}, {}", shown, netname(attacker), desc));
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
    stopped_fury: Option<Vec<u8>>,
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
    fn suffix(&self) -> Vec<u8> {
        let flag = |on: bool, s: &str| if on { s.as_bytes().to_vec() } else { Vec::new() };
        let fury = self.stopped_fury.as_ref().map(|n| shared::bformat!(" (stopped_fury \"{}\")", n)).unwrap_or_default();
        [
            flag(self.victims_weapon, " (victims_weapon)"),
            fury,
            flag(self.killed_specialist, " (killed_specialist)"),
            flag(self.double_kill, " (double_kill)"),
            flag(self.diving, " (diving)"),
            flag(self.sliding, " (sliding)"),
            flag(self.killed_the_one, " (killed_the_one)"),
            self.spree.as_bytes().to_vec(),
        ]
        .concat()
    }
}

/// `"name<userid><authid><x>"` of a log line: `x` is the user id again outside teamplay, the
/// `model` key in teamplay.
fn log_id(name: &[u8], uid: i32, auth: &[u8], model: Option<&[u8]>) -> Vec<u8> {
    match model {
        Some(m) => shared::bformat!("\"{}<{}><{}><{}>\"", name, uid, auth, m),
        None => shared::bformat!("\"{}<{}><{}><{}>\"", name, uid, auth, uid),
    }
}

/// [`log_id`] of a player now: its `netname`, user id, auth id and (teamplay) `model` key, all
/// as the engine has them (bytes, issue #81).
pub fn log_player(e: *mut edict_t) -> Vec<u8> {
    let eng = engine();
    let model = state::teamplay().then(|| eng.info_key(e, c"model").to_bytes());
    log_id(&netname(e), eng.user_id(e), eng.auth_id(e).to_bytes(), model)
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
    let id = log_player;
    let w = log_weapon(weapon);
    if suicide {
        msg::log(&shared::bformat!("{} committed suicide with \"{}\"\n", id(victim), w));
        return;
    }
    if super::ts_flags(attacker) & FL_CLIENT as i32 == 0 {
        msg::log(&shared::bformat!("{} committed suicide with \"{}\" (world)\n", id(victim), w));
        return;
    }
    msg::log(&shared::bformat!("{} killed {} with \"{}\"{}\n", id(attacker), id(victim), w, tags.suffix()));
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

/// The three texts of a team-kill kick or ban.
#[derive(Debug, PartialEq, Eq)]
struct TeamKillPunish {
    /// The `HUD_PRINTCONSOLE` line to the team killer.
    to_killer: String,
    /// The server command.
    command: String,
    /// The `HUD_PRINTTALK` line to everyone.
    to_all: Vec<u8>,
}

/// The kick (`banfortk` ≤ 0) or the ban of `(int)banfortk` minutes (truncated: 0x8ad5a, 0x8adbc,
/// 0x8ae04) for the team killer `name` with user id `uid`.
fn team_kill_punishment(name: &[u8], uid: i32, ban: f32) -> TeamKillPunish {
    if ban <= 0.0 {
        return TeamKillPunish {
            to_killer: "You have been kicked for teamkilling...\n".to_owned(),
            command: format!("kick #{uid}\n"),
            to_all: shared::bformat!("{} kicked for teamkilling\n", name),
        };
    }
    let m = super::x87_trunc(ban as f64);
    TeamKillPunish {
        to_killer: format!("You have been banned for teamkilling ({m} minutes)...\n"),
        command: format!("banid {m} #{uid} kick\n"),
        to_all: shared::bformat!("{} banned for teamkilling ({} minutes)\n", name, m),
    }
}

/// The team-kill kick or ban in `PlayerKilled`. The console notice goes to the team killer:
/// `ClientPrint(pKiller, HUD_PRINTCONSOLE, ...)` (3.0 0x7ae58 and 0x7ad79 file addresses, 2.1
/// 0x1004f7e0), before the kick takes it off the server.
pub fn team_kill_punish(killer: *mut edict_t) {
    let eng = engine();
    let t = team_kill_punishment(&netname(killer), eng.user_id(killer), cvars::f(c"banfortk"));
    msg::text_one(killer, shared::protocol::rules::hud_dest::CONSOLE, &t.to_killer);
    eng.server_command(&t.command);
    msg::text_all(shared::protocol::rules::hud_dest::TALK, &t.to_all);
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
            ts_message_all(THE_ONE_RGB, 1, 30, &shared::bformat!("The One was killed by {}!", netname(k)));
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
    ts_message_all(THE_ONE_RGB, 1, 30, &shared::bformat!("{} is The One!", name));
    msg::log(&shared::bformat!("Player \"{}\" triggered \"the_one\"\n", name));
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
        Active, FragLoser, KillTags, KillerScore, Snapshot, TeamKillPunish, WeaponNaming, active_item_kung_fu, frag_loser, killer_score,
        kung_fu_frag, log_id, log_weapon, own_weapon_message, own_weapon_name, rename_weapon, spectacular_roll, team_kill_punishment,
        thrown_blade_name, victims_gun, weapon_info_name,
    };
    use goldsrc::sys::edict_t;
    use shared::weapons::{id, tables};

    /// The stunt-kill speed (issue #171): mp.dll's unrounded root of `80² + 0.0001²` is just over
    /// 80; the `.so` stores 80.0, as the port's old `float` length did.
    #[test]
    fn frag_speed_rounds_as_each_build() {
        use shared::pm::{Gcc, Msvc};
        let v = [80.0, 0.0001, 0.0];
        assert!(super::frag_fast::<Msvc>(v));
        assert!(!super::frag_fast::<Gcc>(v));
        assert!(!super::frag_fast::<Msvc>([80.0, 0.0, 0.0]));
    }

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
            stopped_fury: Some(b"Bob".to_vec()),
            killed_specialist: true,
            double_kill: true,
            diving: true,
            sliding: true,
            killed_the_one: true,
            spree: " (is_killer)",
        };
        assert_eq!(
            all.suffix(),
            b" (victims_weapon) (stopped_fury \"Bob\") (killed_specialist) (double_kill) (diving) (sliding) (killed_the_one) (is_killer)"
        );
        assert_eq!(KillTags::default().suffix(), b"");
        assert_eq!(KillTags { victims_weapon: true, ..Default::default() }.suffix(), b" (victims_weapon)");
    }

    #[test]
    fn log_ids_repeat_the_user_id_outside_teamplay() {
        assert_eq!(log_id(b"Bob", 7, b"STEAM_0:1:2", None), b"\"Bob<7><STEAM_0:1:2><7>\"");
        assert_eq!(log_id(b"Bob", 7, b"BOT", Some(b"merc")), b"\"Bob<7><BOT><merc>\"");
        assert_eq!(log_id(b"Ren\xe9", 7, b"BOT", None), b"\"Ren\xe9<7><BOT><7>\"", "issue #81: the name's bytes as they are");
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

    const D: WeaponNaming = WeaponNaming::DEFAULT;

    #[test]
    fn a_ruleset_may_name_stocks_and_thrown_blades_plainly() {
        // Issue #131: a ruleset may say "Stock" and "Throwing Knife" (the classname, renamed).
        let plain = WeaponNaming { describe_thrown_blades: false, stock_names_weapon: false };
        let coldcock = Snapshot { coldcock: true, ..Snapshot::default() };
        assert_eq!(own_weapon_name(&coldcock, Some(12), plain), "Stock");
        assert_eq!(own_weapon_name(&coldcock, Some(12), D), name(12) + " Stock");
        // Only a stock hit changes: a plain hit and no gun read the same.
        assert_eq!(own_weapon_name(&Snapshot::default(), Some(12), plain), name(12));
        assert_eq!(own_weapon_name(&coldcock, None, plain), "world");
        let seal = || Some("Throwing Seal Knife");
        assert_eq!(rename_weapon(thrown_blade_name("knife".into(), plain, seal)), "Throwing Knife");
        assert_eq!(rename_weapon(thrown_blade_name("knife".into(), D, seal)), "Throwing Seal Knife");
        assert_eq!(rename_weapon(thrown_blade_name("knife".into(), D, || None)), "world");
    }

    #[test]
    fn own_weapon_follows_the_ts_globals() {
        let none = Snapshot::default();
        let close = Snapshot { close_combat: true, ..none };
        let coldcock = Snapshot { coldcock: true, ..none };
        // A simulated bullet names its own weapon, whatever is held now.
        assert_eq!(own_weapon_name(&Snapshot { weapon_id: 12, ..close }, Some(0), D), name(12));
        // CloseCombatAttack: kung fu even with a gun out (a delayed kick after switching).
        assert_eq!(own_weapon_name(&close, Some(12), D), "Kung Fu");
        assert_eq!(own_weapon_name(&close, None, D), "Kung Fu");
        // A slash or hitscan shot: the weapon in hand; the stock hit says so.
        assert_eq!(own_weapon_name(&none, Some(12), D), name(12));
        assert_eq!(own_weapon_name(&coldcock, Some(12), D), name(12) + " Stock");
        // Kung fu outside CloseCombatAttack (a dive through glass), or no gun: "world".
        assert_eq!(own_weapon_name(&none, Some(0), D), "world");
        assert_eq!(own_weapon_name(&coldcock, None, D), "world");
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
        assert!(kung_fu_frag(&close, Active::None, None));
        assert!(!kung_fu_frag(&none, Active::None, None), "no gun, no close combat");
        assert!(kung_fu_frag(&none, Active::Gun, Some(0)), "kung fu out");
        assert!(kung_fu_frag(&close, Active::Gun, Some(12)));
        assert!(!kung_fu_frag(&none, Active::Gun, Some(12)), "a knife or katana slash is no kung-fu frag");
        assert!(!kung_fu_frag(&Snapshot { weapon_id: 12, ..close }, Active::Gun, Some(0)), "a simulated bullet");
    }

    #[test]
    fn a_cycler_in_hand_is_never_kung_fu() {
        // Issue #148: an active item that isn't the gun scores the base points, even under
        // `glast_closecombat` (a stunt's hit).
        let close = Snapshot { close_combat: true, ..Snapshot::default() };
        assert!(!kung_fu_frag(&close, Active::Cycler, None));
        assert!(!active_item_kung_fu(Active::Cycler, None, true));
        assert!(!active_item_kung_fu(Active::Cycler, None, false));
        assert!(active_item_kung_fu(Active::None, None, true));
        assert!(active_item_kung_fu(Active::Gun, Some(0), false));
    }

    #[test]
    fn a_world_death_costs_the_killer_not_the_victim() {
        // Issue #118: a fall or a trigger lowers the world's frags; only a suicide the victim's.
        assert_eq!(frag_loser(false), FragLoser::Killer);
        assert_eq!(frag_loser(true), FragLoser::Victim);
    }

    #[test]
    fn own_weapon_kills_are_announced() {
        // Issue #121: "%s #TS_Killed %s #TS_OwnWeapon", killer then victim.
        assert_eq!(own_weapon_message(b"Bob", b"Al"), b"Bob #TS_Killed Al #TS_OwnWeapon");
    }

    #[test]
    fn the_killer_score_info_includes_suicides() {
        // Issue #120: `Classify() == CLASS_PLAYER` on pKiller, with no suicide test.
        assert_eq!(killer_score(true, true), KillerScore::Victim);
        assert_eq!(killer_score(false, true), KillerScore::Killer);
        assert_eq!(killer_score(false, false), KillerScore::Nobody);
    }

    #[test]
    fn team_kill_punishment_tells_the_killer() {
        // Issue #119: the console line is the team killer's (`ClientPrint(pKiller, ...)`).
        assert_eq!(
            team_kill_punishment(b"Bob", 7, 0.0),
            TeamKillPunish {
                to_killer: "You have been kicked for teamkilling...\n".into(),
                command: "kick #7\n".into(),
                to_all: b"Bob kicked for teamkilling\n".to_vec(),
            }
        );
        let ban = team_kill_punishment(b"Bob", 7, 5.9);
        assert_eq!(ban.to_killer, "You have been banned for teamkilling (5 minutes)...\n");
        assert_eq!(ban.command, "banid 5 #7 kick\n");
        assert_eq!(ban.to_all, b"Bob banned for teamkilling (5 minutes)\n");
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
