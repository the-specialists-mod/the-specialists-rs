//! TS 3.0's scoring: the body of `CHalfLifeMultiplay::PlayerKilled` (0x895e4) and
//! `TSGetPointsForFrag` (0x89140): points, sprees and awards, cash, kill messages, `DeathMsg`,
//! logs and `ScoreInfo`. Decomp: gamerules-entities.md §2.3. The kill mechanics both rulesets
//! use (the weapon name, `DeathNotice`, the log line, The One's death, team-kill punishment) are
//! `ts_server::rules::scoring`'s.

use goldsrc::edict::EdictPtr;
use goldsrc::server::engine;
use goldsrc::sys::edict_t;
use shared::protocol::hud_wire::PwUp;
use shared::ts::{powerup as pw, stunt};
use shared::weapons::{WeaponKind, tables};
use ts_server::damage::DamageInfo;
use ts_server::damage::glast::Snapshot;
use ts_server::messages::Msg;
use ts_server::player::Player;
use ts_server::rules::lifecycle::{give_cash, score_info_of};
use ts_server::rules::msg::{self, ts_message_all};
use ts_server::rules::scoring::{
    Frag, KillFlags, Killer, STUNTING, active_gun, active_item_kung_fu, classname, death_notice, do_spectacularness, kill_report,
    killer_score_info, log_kill, own_hand, points_for_kill, report_kill, team_kill_punish, the_one_is_dead, weapon_name,
};
use ts_server::rules::{add_frags, cvars, has_player, netname, other, state, teams};
use ts_server::weapons::Active;
use ts_server::world::{Cx, UseType};

const GREEN: [u8; 3] = [0, 255, 0];

/// The `PlayerKilled` body once the killer can be reached (`ts_server::rules::scoring::player_killed`
/// defers it until then). `p`/`cx` are the victim; `g` holds TS's damage globals as the kill left
/// them.
pub fn player_killed(p: &mut Player, cx: &mut Cx, d: &DamageInfo, g: Snapshot) {
    let eng = engine();
    let now = cx.time();
    let victim = cx.edict;
    let vidx = cx.index();
    let vname = netname(victim);
    let attacker = if d.attacker.is_null() { eng.entity_of_index(0) } else { d.attacker };
    let suicide = attacker == victim;
    let killer_is_player = !suicide && has_player(attacker);
    let close = g.close_combat;
    // Port addition (plugin API version 5, `kill_scored`): the victim's spree before the death.
    let victim_spree = p.ts.spree as i32;
    let mut scored_points = 0;

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
            item: ts_server::weapons::active_item(k),
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
        points = ts_server::gameplay::active().server.points_for_frag(&frag, &mut sliding, &mut diving);
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
    log_kill(attacker, victim, &weapon, suicide, &tags.suffix());
    // Port addition (plugin API version 5): what `kill_scored` reports, from the values above.
    let kung_fu = own_hand(d, attacker) && killer.as_ref().is_some_and(|k| kung_fu_frag(&g, k.item, k.gun));
    let mut report = kill_report(d, &g, victim, attacker, killer_is_player, attacker_gun, &weapon);
    report.tags = KillFlags { award: [is_killer, is_demolition, is_specialist, is_unstoppable], kung_fu, ..tags.flags() }.bits(&g);
    report.victim_spree = victim_spree;

    p.deaths += 1;
    // Lent back to its slot, so a `game_player_team` it reaches changes our team inside the
    // `Use`, as in TS.
    ts_server::world::lend(victim, p, || ts_server::entities::fire_targets(c"game_playerdie", victim, victim, UseType::Toggle, 0.0));
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
            add_frags(victim, Some(p), -5.0);
            ts_message_all(GREEN, 1, 30, &shared::bformat!("{}, #TS_SpecialistSuicide", vname));
        }
        match frag_loser(suicide) {
            FragLoser::Victim => add_frags(victim, Some(p), -1.0),
            FragLoser::Killer => add_frags(attacker, None, -1.0),
        }
    } else if let Some(mut k) = killer {
        let kname = netname(k.e);
        let kv = k.e.v();
        if (kv.iuser4 & STUNTING != 0 || close || points > 2) && do_spectacularness() {
            ts_server::entities::ts::create_slow_motion_point(cx.pev().origin, 0.25, 1.0, false);
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
        add_frags(k.e, None, points as f32);
        scored_points = points;
        report.killer_spree = k.spree as i32;
        ts_server::entities::fire_targets(c"game_playerkill", k.e, k.e, UseType::Toggle, 0.0);

        // slowmatch: each kill stores a second of slow motion (max 3).
        if cvars::f(c"slowmatch") == 1.0 && points > 0 && (k.held == 0 || k.held & pw::SLOWMO != 0) && k.active == 0 {
            k.held = pw::SLOWMO;
            k.held_secs = (k.held_secs + 1).min(3);
        }
        // mp.dll (0x1005b18a): a bot that scored the kill may taunt the victim (`CBot::OnKill`).
        if points > 0 {
            other(k.e, |kp| ts_server::bot_backend::on_kill(kp, victim, now));
        }
        // Port addition: plugins hear of every kill a player scored.
        ts_server::plugins::on_kill(k.e, victim, points);

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
    report.points = scored_points;
    report_kill(victim, attacker, killer_is_player, report);
}

/// `PlayerKilled`'s announcement of a kill with the victim's own gun (string 0x108d00); the
/// client's titles.txt resolves the keys ("X killed Y with his own weapon").
fn own_weapon_message(killer: &[u8], victim: &[u8]) -> Vec<u8> {
    shared::bformat!("{} #TS_Killed {} #TS_OwnWeapon", killer, victim)
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
            desc = ts_server::weapons::thrown_knife_description(d.inflictor).unwrap_or("Throwing").to_owned();
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
    /// Port addition (plugin API version 5): the tags as `kill_scored` reports them (the spree
    /// award and the kung-fu frag apart).
    fn flags(&self) -> KillFlags {
        KillFlags {
            victims_weapon: self.victims_weapon,
            stopped_fury: self.stopped_fury.is_some(),
            killed_specialist: self.killed_specialist,
            double_kill: self.double_kill,
            diving: self.diving,
            sliding: self.sliding,
            killed_the_one: self.killed_the_one,
            ..Default::default()
        }
    }

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

/// `PlayerKilled`'s ` (victims_weapon)` gun test (0x89703..0x8975c): the killer holds a weapon
/// record (`gun+0xcc`) whose original owner (`s_weapon_status+0x30`, `owner_of`) is the victim,
/// and `GetWeaponType` isn't 2, 3 or 11 (grenades, blades; 11 has no weapon). `cur` is
/// [`active_gun`].
fn victims_gun(cur: Option<u8>, owner_of: impl FnOnce(u8) -> *mut edict_t, victim: *mut edict_t) -> bool {
    use shared::weapons::{kind, restriction_type};
    let Some(cur) = cur.filter(|&c| c != 0 && (c as usize) < shared::weapons::MAX_WEAPONS) else { return false };
    owner_of(cur) == victim && !matches!(restriction_type(cur), kind::GRENADE | kind::KNIFE | kind::FLAG)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::weapons::id;

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
    fn victims_weapon_needs_the_victims_gun_in_hand() {
        ts30_gameplay::tables::make_default();
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
    fn a_cycler_in_hand_is_never_kung_fu() {
        // Issue #148: an active item that isn't the gun scores the base points, even under
        // `glast_closecombat` (a stunt's hit).
        let close = Snapshot { close_combat: true, ..Snapshot::default() };
        assert!(!kung_fu_frag(&close, Active::Cycler, None));
    }
}

/// Port addition (plugin API version 5): what `kill_scored` reports (the shared part is
/// `ts_server::rules::scoring::kill_report_tests`').
#[cfg(test)]
mod kill_report_tests {
    use super::{KillTags, Snapshot};
    use ts_plugin_api::kill;

    #[test]
    fn tags_become_bits() {
        let tags = KillTags { stopped_fury: Some(b"x".to_vec()), double_kill: true, sliding: true, ..Default::default() };
        assert_eq!(tags.flags().bits(&Snapshot::default()), kill::STOPPED_FURY | kill::DOUBLE_KILL | kill::SLIDING);
        assert_eq!(KillTags::default().flags().bits(&Snapshot::default()), 0);
        let all = KillTags {
            victims_weapon: true,
            stopped_fury: Some(b"x".to_vec()),
            killed_specialist: true,
            double_kill: true,
            diving: true,
            sliding: true,
            killed_the_one: true,
            spree: " (is_unstoppable)",
        };
        assert_eq!(all.flags().bits(&Snapshot::default()), 0x7f, "the log tags; the spree award is the caller's");
    }
}
