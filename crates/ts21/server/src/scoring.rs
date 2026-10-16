//! TS 2.1's kill scoring: `TSGetPointsForFrag` (0x1004f3c0) and `CHalfLifeMultiplay::PlayerKilled`
//! (0x1004f7e0) with the teamplay wrapper (0x100392e0). Decomp `docs/ts21/rules.md` §4.

use crate::cash;
use goldsrc::edict::EdictPtr;
use goldsrc::server::engine;
use goldsrc::sys::{FL_CLIENT, edict_t};
use shared::protocol::hud_wire::PwUp;
use shared::ts::powerup as pw;
use shared::weapons::{WeaponKind, tables};
use ts_server::damage::DamageInfo;
use ts_server::damage::glast::Snapshot;
use ts_server::messages::Msg;
use ts_server::player::Player;
use ts_server::rules::lifecycle::score_info_of;
use ts_server::rules::msg::{self, ts_message_all};
use ts_server::rules::scoring::{self as core, Frag, STUNTING};
use ts_server::rules::state::{self, Mode};
use ts_server::rules::{cvars, has_player, netname, other, teams};
use ts_server::weapons::Active;
use ts_server::world::{Cx, UseType};

const GREEN: [u8; 3] = [0, 255, 0];
/// `iuser4` 0x20: prone (a slide when moving with friction under 1).
const PRONE: i32 = 0x20;

/// What `TSGetPointsForFrag` adds to the frag message after the label (0x100989f0, " style!"),
/// and the stunt/slide labels.
const STYLE: &str = " style!";

/// 2.1's DeathNotice (`FUN_1004ede0`) names a thrown blade by its inflictor's classname (no
/// `CTSKnife::GetDescription`), so every blade is "Throwing Knife", and a stock hit is just
/// "Stock" (0x100989b0), not "<weapon> Stock".
const NAMING: core::WeaponNaming = core::WeaponNaming { describe_thrown_blades: false, stock_names_weapon: false };

/// The kill as `TSGetPointsForFrag` sees it, without the engine: the label and points before the
/// stunt test.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// A plain kill: the base points, no label.
    Plain,
    /// A projectile that isn't a thrown knife (a grenade, ...): the base points at once, no stunt
    /// bonus and no message.
    Projectile,
    /// "Throwing Knife", 2 points.
    Knife,
    /// "Kung-Fu", 2 points.
    KungFu,
    /// "Katana", 2 points.
    Katana,
}

/// The points and the frag message's label, given the kind of kill, whether the killer was
/// stunting (`iuser4 & 0x10c10`, any speed) or sliding (prone, friction < 1, faster than 80),
/// whether the killer is a player, and the base points. A stunt adds 2 and a slide 1, close
/// combat or not. `None` for the label means no message.
pub fn points(kind: Kind, base: i32, stunting: bool, sliding: bool, killer_is_player: bool) -> (i32, Option<String>) {
    let (mut pts, label) = match kind {
        Kind::Projectile => return (base & 0xff, None),
        Kind::Plain => (base & 0xff, ""),
        Kind::Knife => (2, "Throwing Knife"),
        Kind::KungFu => (2, "Kung-Fu"),
        Kind::Katana => (2, "Katana"),
    };
    let label = if stunting || sliding {
        if killer_is_player {
            pts = (pts + if stunting { 2 } else { 1 }) & 0xff;
        }
        if pts < 2 {
            return (pts, None);
        }
        let style = if stunting { "Stuntman style!" } else { "Sliding style!" };
        if label.is_empty() { style.to_owned() } else { format!("{label} & {style}") }
    } else if label.is_empty() {
        String::new()
    } else {
        format!("{label}{STYLE}")
    };
    (pts, (pts > 1).then_some(label))
}

/// 2.1's `pev->velocity.Length() > 80` (0x1004f680..0x1004f6ae): the squares `x, y, z`, the root
/// unrounded (a NaN fails).
fn slide_fast(v: [f32; 3]) -> bool {
    let [x, y, z] = v.map(f64::from);
    (x * x + y * y + z * z).sqrt() > 80.0
}

/// `TSGetPointsForFrag` (0x1004f3c0). Round modes score the plain points; a team kill keeps its
/// -1; in The One mode only The One scores. The frag message is `"%i frags for %s, %s"` in
/// English (3.0 uses `#TS_` keys and doubles the shown value on sprees and double kills).
pub fn points_for_frag(frag: &Frag<'_>, sliding: &mut bool, diving: &mut bool) -> i32 {
    let Frag { damage: d, glast: g, killer: k, base, .. } = *frag;
    if state::with(|g| matches!(g.mode, Mode::RoundTdm | Mode::Objective)) || base < 0 {
        return base;
    }
    if state::the_one_mode() && !k.is_the_one {
        return 0;
    }
    let attacker = k.e;
    if ts_server::rules::ts_flags(attacker) & FL_CLIENT as i32 == 0 {
        return base & 0xff;
    }
    let kind = kill_kind(d, g, k.item, k.gun, attacker);
    let av = attacker.v();
    let stunting = av.iuser4 & STUNTING != 0;
    let slide = !stunting && av.iuser4 & PRONE != 0 && av.friction < 1.0 && slide_fast(av.velocity);
    // The port's kill log tags (3.0's) are kept for the flags; 2.1 logs no tags.
    *diving = stunting;
    *sliding = slide;
    let (pts, label) = points(kind, base, stunting, slide, true);
    if let Some(label) = label {
        ts_message_all(GREEN, 1, 30, &shared::bformat!("{} frags for {}, {}", pts, netname(attacker), label));
    }
    pts
}

/// The label test of `TSGetPointsForFrag`: an inflictor that isn't the killer is a projectile
/// (a thrown knife by classname); the killer's own hit outside a simulated bullet's
/// (`DAT_100ac81c == 0`, the port's -1) is kung fu by the active item
/// ([`core::active_item_kung_fu`]: no item under `glast_closecombat`, or the gun with kung fu out
/// or under `glast_closecombat`), or the katana (weapon category 10) in the gun. Any other
/// active item (a `cycler_weapon`) is a plain kill.
fn kill_kind(d: &DamageInfo, g: &Snapshot, item: Active, gun: Option<u8>, attacker: *mut edict_t) -> Kind {
    if d.inflictor.is_null() {
        return Kind::Plain;
    }
    if d.inflictor != attacker {
        return if core::classname(d.inflictor) == "knife" { Kind::Knife } else { Kind::Projectile };
    }
    if g.weapon_id != -1 {
        return Kind::Plain;
    }
    own_kind(item, gun, g.close_combat)
}

/// [`kill_kind`] for the killer's own hit.
fn own_kind(item: Active, gun: Option<u8>, close_combat: bool) -> Kind {
    if core::active_item_kung_fu(item, gun, close_combat) {
        Kind::KungFu
    } else if item == Active::Gun && gun.and_then(|cur| tables().weapon(cur)).is_some_and(|w| w.kind == WeaponKind::Katana) {
        Kind::Katana
    } else {
        Kind::Plain
    }
}

/// The spree award (`TSPAward`): The One keeps 10.
fn award(current: u8, spree_award: u8) -> u8 {
    if current == 10 { 10 } else { spree_award }
}

/// The killer's state `PlayerKilled` reads and writes.
struct Killer {
    e: *mut edict_t,
    spree: u8,
    last_kill: f32,
    is_the_one: bool,
    award: u8,
    held: u16,
    held_secs: i32,
    active: u16,
    gun: Option<u8>,
    item: Active,
}

/// A stored slow pause of 3 s when nothing is active or held (`+0x69c..+0x6a8`).
fn store_slow_pause(k: &mut Killer) {
    if k.active == 0 && k.held == 0 {
        k.active = 0;
        k.held = pw::SLOWPAUSE;
        k.held_secs = 3;
    }
}

/// `CHalfLifeMultiplay::PlayerKilled` (0x1004f7e0) and, in teamplay, the wrapper
/// (`CHalfLifeTeamplay::PlayerKilled` 0x100392e0), for victim `p`.
pub fn player_killed(p: &mut Player, cx: &mut Cx, d: &DamageInfo, g: Snapshot) {
    let eng = engine();
    let now = cx.time();
    let victim = cx.edict;
    let vidx = cx.index();
    // Guard: TS reads the killer's pev without a test; a NULL attacker is the world.
    let attacker = if d.attacker.is_null() { eng.entity_of_index(0) } else { d.attacker };
    let suicide = attacker == victim;
    let killer_is_player = !suicide && has_player(attacker);
    // Port addition (plugin API version 5, `kill_scored`): the victim's spree and The One before
    // the death changes them.
    let (victim_spree, killed_the_one) = (p.ts.spree, p.ts.is_the_one);
    let (mut scored_points, mut killer_spree) = (0, 0);
    let (mut stunt, mut kung_fu, mut spree_outcome) = ((false, false), false, None);

    // DeathNotice first (2.1 `FUN_1004ede0`), with its log line.
    let attacker_gun = if suicide { core::active_gun(p) } else { other(attacker, |k| core::active_gun(k)).flatten() };
    let weapon = core::weapon_name_with(d, &g, attacker_gun, NAMING);
    core::death_notice(attacker, d.inflictor, victim, &weapon);
    core::log_kill_untagged(attacker, victim, &weapon, suicide);
    p.deaths += 1;
    ts_server::world::lend(victim, p, || ts_server::entities::fire_targets(c"game_playerdie", victim, victim, UseType::Toggle, 0.0));
    if p.ts.is_the_one {
        crate::players::the_one_is_dead(p, cx, killer_is_player.then_some(attacker));
        p.ts.spree = 0;
    }

    let killer = if killer_is_player {
        other(attacker, |k| Killer {
            e: attacker,
            spree: k.ts.spree,
            last_kill: k.ts.last_kill,
            is_the_one: k.ts.is_the_one,
            award: k.ts.award,
            held: k.ts.held,
            held_secs: k.ts.held_secs,
            active: k.active_powerup,
            gun: core::active_gun(k),
            item: ts_server::weapons::active_item(k),
        })
    } else {
        None
    };

    match killer {
        None => {
            // A suicide or a death by something that isn't a player: the *attacker* loses a frag
            // (0x1004f87f: `pKiller->frags -= 1` on all three paths), so the world or a trigger
            // takes it and the victim keeps its frags.
            attacker.v().frags -= 1.0;
        }
        Some(mut k) => {
            let k = &mut k;
            let base = core::points_for_kill(attacker, victim);
            let shim = core::Killer {
                e: k.e,
                idx: 0,
                spree: k.spree,
                last_kill: k.last_kill,
                is_the_one: k.is_the_one,
                award: k.award,
                held: k.held,
                held_secs: k.held_secs,
                active: k.active,
                gun: k.gun,
                item: k.item,
                victims_gun: false,
            };
            let frag = Frag { damage: d, glast: &g, killer: &shim, base, now };
            let (mut sliding, mut diving) = (false, false);
            let mut points = ts_server::gameplay::active().server.points_for_frag(&frag, &mut sliding, &mut diving);
            stunt = (diving, sliding);
            kung_fu = core::own_hand(d, attacker) && kill_kind(d, &g, k.item, k.gun, attacker) == Kind::KungFu;
            let kv = attacker.v();
            if (kv.iuser4 & STUNTING != 0 || g.close_combat || points > 2) && core::do_spectacularness() {
                ts_server::entities::ts::create_slow_motion_point(cx.pev().origin, 0.15, 1.5, false);
            }
            let dm = state::with(|g| matches!(g.mode, Mode::Dm | Mode::TeamDm));
            if dm && points > 0 {
                spree_outcome = Some(spree(p.ts.spree, &netname(victim), k, &mut points, now));
            }
            kv.frags += points as f32;
            other(k.e, |kp| kp.frags = kv.frags as i32);
            ts_server::entities::fire_targets(c"game_playerkill", k.e, k.e, UseType::Toggle, 0.0);
            if cvars::f(c"slowmatch") == 1.0 && points > 0 && (k.held == 0 || k.held & pw::SLOWMO != 0) && k.active == 0 {
                k.active = 0;
                k.held = pw::SLOWMO;
                k.held_secs = (k.held_secs + 1).min(5);
            }
            // `CBot::OnKill` (0x1005ddd0, from 0x1004fe02) through the active bot backend, so a
            // plugin backend's bot taunts with its own AI, as on the 3.0 path.
            if points > 0 {
                other(k.e, |kp| ts_server::bot_backend::on_kill(kp, victim, now));
            }
            // Port addition: plugins hear of every kill a player scored, as on the 3.0 path.
            ts_server::plugins::on_kill(k.e, victim, points);
            (scored_points, killer_spree) = (points, k.spree as i32);
            let kick = points == -1 && other(k.e, |kp| team_kill(kp, now)).unwrap_or(false);
            let (spree, award, held, held_secs, active, last_kill) = (k.spree, k.award, k.held, k.held_secs, k.active, k.last_kill);
            other(k.e, |kp| {
                kp.ts.spree = spree;
                kp.ts.award = award;
                kp.ts.held = held;
                kp.ts.held_secs = held_secs;
                kp.active_powerup = active;
                kp.ts.last_kill = last_kill;
            });
            if kick {
                core::team_kill_punish(k.e);
            }
        }
    }

    let si = score_info_of(p, cx.pev(), vidx);
    msg::all(Msg::ScoreInfo, |w| si.write(w));
    // The killer's, suicides included, and its decal timer (0x1005004b, 0x100500d8).
    core::killer_score_info(p, cx, attacker, now);
    state::with(|g| g.call_check_deads(now));

    if state::teamplay() {
        // The teamplay wrapper: The One's killer bonus, then `RecountTeams(false)`.
        the_one_kill_bonus(attacker);
        teams::recount(false);
    }
    // Port addition (plugin API version 5): `kill_scored`, with 2.1's values.
    let mut report = core::kill_report(d, &g, victim, attacker, killer_is_player, attacker_gun, &weapon);
    report.points = scored_points;
    report.tags = kill_flags(killed_the_one, stunt, kung_fu, spree_outcome).bits(&g);
    (report.killer_spree, report.victim_spree) = (killer_spree, victim_spree as i32);
    core::report_kill(victim, attacker, killer_is_player, report);
}

/// What 2.1's spree part of `PlayerKilled` found (port addition, for `kill_scored`'s tags).
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
struct SpreeOutcome {
    /// Under a second after the killer's last kill.
    double_kill: bool,
    /// The victim's spree was over 2 ("stops X's fury!") / over 8 ("has killed THE SPECIALIST!").
    stopped_fury: bool,
    killed_specialist: bool,
    /// The award the killer's new spree reached: Killer (3), Demolition Man (6), The Specialist
    /// (9), Unstoppable (10 on).
    award: [bool; 4],
}

/// [`spree`]'s tests, given the victim's spree, the killer's before this kill and the seconds
/// since its last kill.
fn spree_outcome(victim_spree: u8, killer_spree: u8, since_last_kill: f32) -> SpreeOutcome {
    // `if (spree != 255) spree++`.
    let s = killer_spree.saturating_add(1);
    SpreeOutcome {
        double_kill: since_last_kill < 1.0,
        stopped_fury: victim_spree > 2,
        killed_specialist: victim_spree > 8,
        award: [s == 3, s == 6, s == 9, s >= 10],
    }
}

/// Port addition (plugin API version 5): `kill_scored`'s tags for a 2.1 kill: The One killed,
/// `TSGetPointsForFrag`'s stunt test (`(diving, sliding)`: 2.1's "Stuntman" and "Sliding" styles)
/// and kung-fu frag, and what the spree part found (`None` when it didn't run: no killing
/// player, a round mode, or no points). 2.1 has no ` (victims_weapon)` test, so that bit is
/// never set.
fn kill_flags(killed_the_one: bool, (diving, sliding): (bool, bool), kung_fu: bool, spree: Option<SpreeOutcome>) -> core::KillFlags {
    let o = spree.unwrap_or_default();
    core::KillFlags {
        victims_weapon: false,
        stopped_fury: o.stopped_fury,
        killed_specialist: o.killed_specialist,
        double_kill: o.double_kill,
        diving,
        sliding,
        killed_the_one,
        award: o.award,
        kung_fu,
    }
}

/// The DM/TDM part of `PlayerKilled` for a scoring kill: double kill (1 s, always updating the
/// last kill), stopping a spree, killing The Specialist, the killer's spree rewards, the overheal
/// timer and the kill cash.
fn spree(victim_spree: u8, vname: &[u8], k: &mut Killer, points: &mut i32, now: f32) -> SpreeOutcome {
    let kname = netname(k.e);
    let o = spree_outcome(victim_spree, k.spree, now - k.last_kill);
    if o.double_kill {
        *points *= 2;
        ts_message_all(GREEN, 1, 30, &shared::bformat!("DoubleFrag for {}!", kname));
    }
    if o.stopped_fury {
        if !o.killed_specialist {
            ts_message_all(GREEN, 1, 30, &shared::bformat!("{} stops {}'s fury!", kname, vname));
        } else {
            *points += 5;
            ts_message_all(GREEN, 1, 40, &shared::bformat!("{} frags for {}, he has killed THE SPECIALIST!", *points, kname));
            store_slow_pause(k);
        }
    }
    k.spree = k.spree.saturating_add(1);
    let kv = k.e.v();
    match k.spree {
        3 => {
            ts_message_all(GREEN, 1, 30, &shared::bformat!("{} is a Killer!", kname));
            kv.health = (kv.health + 30.0).min(150.0);
            k.award = award(k.award, 2);
        }
        6 => {
            ts_message_all(GREEN, 1, 30, &shared::bformat!("{} is Demolition Man!", kname));
            kv.health = (kv.health + 50.0).min(150.0);
            k.award = award(k.award, 3);
        }
        9 => {
            ts_message_all(GREEN, 1, 30, &shared::bformat!("{} is The Specialist!", kname));
            // Set, not raised: an overhealed killer can lose health.
            kv.health = 150.0;
            other(k.e, |kp| {
                kp.kungfu = true;
                kp.superjump = true;
            });
            engine().set_physics_key(k.e, c"mj", c"1");
            msg::one(Msg::PwUp, k.e, |w| PwUp { bits: pw::KUNGFU as i16, secs: 0 }.write(w));
            msg::one(Msg::PwUp, k.e, |w| PwUp { bits: pw::SUPERJUMP as i16, secs: 0 }.write(w));
            store_slow_pause(k);
            k.award = award(k.award, 4);
        }
        s if s >= 10 => {
            ts_message_all(GREEN, 1, 30, &shared::bformat!("{} is Unstoppable! {} consecutive frags", kname, s));
            *points *= 2;
            k.award = award(k.award, 5);
        }
        _ => {}
    }
    if kv.health > 100.0 {
        other(k.e, |kp| kp.ts.health_timer = now + 1.0);
    }
    k.last_kill = now;
    let pts = *points;
    other(k.e, |kp| cash::give_kill_cash(kp, if pts == -1 { -1000 } else { pts.wrapping_mul(900) }));
    o
}

/// The team-kill window (identical to 3.0): 3 team kills within rolling 30 s windows.
fn team_kill(kp: &mut Player, now: f32) -> bool {
    if kp.ts.tk_until == 0.0 || kp.ts.tk_until < now {
        kp.ts.tk_count = 1;
    } else {
        kp.ts.tk_count += 1;
    }
    kp.ts.tk_until = now + 30.0;
    kp.ts.tk_count > 2
}

/// The teamplay `PlayerKilled` wrapper (0x100392e0): when the attacker is The One, it gains
/// 10 health per player, up to 200. TS counts `UTIL_PlayerByIndex(i)` for `i` in
/// `0..maxClients`, so slot 0 (the world, never a player) is tried and the last slot never
/// counts.
fn the_one_kill_bonus(attacker: *mut edict_t) {
    if !other(attacker, |k| k.ts.is_the_one).unwrap_or(false) {
        return;
    }
    let eng = engine();
    let n = (0..eng.max_clients()).filter(|&i| i > 0 && has_player(eng.player(i))).count() as f32;
    let v = attacker.v();
    v.health = the_one_health(v.health, n);
}

/// The One's health after a kill: `+10 × players`, capped at 200.
pub fn the_one_health(health: f32, players: f32) -> f32 {
    let h = (players * 10.0) + health;
    if h > 200.0 { 200.0 } else { h }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The slide's speed (issue #171): 2.1's unrounded root of `80² + 0.0001²` is over 80; the
    /// port's old `float` length was 80.0.
    #[test]
    fn slide_speed_keeps_the_root_unrounded() {
        assert!(slide_fast([80.0, 0.0001, 0.0]));
        assert!(!slide_fast([80.0, 0.0, 0.0]));
        assert!(!slide_fast([f32::NAN, 0.0, 0.0]));
    }

    #[test]
    fn stunt_kills_score_three_and_stack_on_close_combat() {
        assert_eq!(points(Kind::Plain, 1, false, false, true), (1, None));
        assert_eq!(points(Kind::Plain, 1, true, false, true), (3, Some("Stuntman style!".into())));
        assert_eq!(points(Kind::Plain, 1, false, true, true), (2, Some("Sliding style!".into())));
        assert_eq!(points(Kind::KungFu, 1, false, false, true), (2, Some("Kung-Fu style!".into())));
        assert_eq!(points(Kind::KungFu, 1, true, false, true), (4, Some("Kung-Fu & Stuntman style!".into())));
        assert_eq!(points(Kind::Katana, 1, false, true, true), (3, Some("Katana & Sliding style!".into())));
        assert_eq!(points(Kind::Knife, 1, false, false, true), (2, Some("Throwing Knife style!".into())));
        // Any other projectile: the base, no bonus, no message.
        assert_eq!(points(Kind::Projectile, 1, true, false, true), (1, None));
        // A stunt by a killer that isn't a player adds nothing.
        assert_eq!(points(Kind::Plain, 1, true, false, false), (1, None));
    }

    #[test]
    fn a_cycler_kill_is_plain() {
        ts30_gameplay::tables::make_default();
        // Issue #148: an active item that isn't the gun is no kung fu, even under close combat.
        use shared::weapons::id;
        assert_eq!(own_kind(Active::Cycler, None, true), Kind::Plain);
        assert_eq!(own_kind(Active::None, None, true), Kind::KungFu);
        assert_eq!(own_kind(Active::None, None, false), Kind::Plain);
        assert_eq!(own_kind(Active::Gun, Some(0), false), Kind::KungFu);
        assert_eq!(own_kind(Active::Gun, Some(id::KATANA), false), Kind::Katana);
        assert_eq!(own_kind(Active::Gun, Some(id::KATANA), true), Kind::KungFu);
        assert_eq!(own_kind(Active::Gun, Some(id::BERETTA), false), Kind::Plain);
    }

    #[test]
    fn kills_reach_the_bot_backend_and_the_plugins() {
        // Issue #157: the kill taunt goes through the active bot backend (a plugin backend's bots
        // included), never straight to the native bots, and plugins hear of the kill.
        let src = include_str!("scoring.rs");
        assert!(!src.contains(concat!("ts_server::bots", "::on_kill")));
        assert!(src.contains(concat!("ts_server::bot_backend", "::on_kill(kp, victim, now)")));
        assert!(src.contains(concat!("ts_server::plugins", "::on_kill(k.e, victim, points)")));
    }

    /// #198: 2.1's spree part, as `kill_scored` reports it: a double kill within a second, the
    /// fury stopped from a victim's spree of 3 (the Specialist from 9), and 2.1's awards at 3, 6,
    /// 9 and 10 on (the killer's spree after this kill, capped at 255).
    #[test]
    fn spree_outcomes_are_2_1s() {
        let o = spree_outcome(0, 0, 5.0);
        assert_eq!(o, SpreeOutcome::default());
        assert!(spree_outcome(0, 0, 0.99).double_kill);
        assert!(!spree_outcome(0, 0, 1.0).double_kill, "1 s, not 3.0's 2");
        assert_eq!(spree_outcome(2, 0, 5.0), SpreeOutcome::default());
        let o = spree_outcome(3, 0, 5.0);
        assert!(o.stopped_fury && !o.killed_specialist);
        let o = spree_outcome(9, 0, 5.0);
        assert!(o.stopped_fury && o.killed_specialist);
        let award = |k| spree_outcome(0, k, 5.0).award;
        assert_eq!(
            [award(1), award(2), award(5), award(8)],
            [[false; 4], [true, false, false, false], [false, true, false, false], [false, false, true, false]]
        );
        assert_eq!([award(9), award(254), award(255)], [[false, false, false, true]; 3]);
        assert_eq!(award(3), [false; 4], "only the kill that reaches it");
    }

    /// #198: a 2.1 kill's `kill_scored` tags. No ` (victims_weapon)` in 2.1; no `glast_headshot`
    /// either, so a head kill reports no `HEADSHOT` (the damage never records one: `damage.rs`).
    #[test]
    fn kill_report_tags_are_2_1s() {
        use ts_plugin_api::kill;
        let none = Snapshot::default();
        assert_eq!(kill_flags(false, (false, false), false, None).bits(&none), 0);
        let specialist = spree_outcome(3, 8, 0.5);
        assert_eq!(
            kill_flags(true, (true, false), true, Some(specialist)).bits(&Snapshot { close_combat: true, ..none }),
            kill::KILLED_THE_ONE
                | kill::DIVING
                | kill::KUNG_FU
                | kill::STOPPED_FURY
                | kill::DOUBLE_KILL
                | kill::IS_SPECIALIST
                | kill::CLOSE_COMBAT
        );
        assert_eq!(
            kill_flags(false, (false, true), false, Some(spree_outcome(12, 9, 5.0))).bits(&none),
            kill::SLIDING | kill::STOPPED_FURY | kill::KILLED_SPECIALIST | kill::IS_UNSTOPPABLE
        );
        // The stock hit's global, as 2.1's "Stock" names it.
        assert_eq!(kill_flags(false, (false, false), false, None).bits(&Snapshot { coldcock: true, ..none }), kill::COLDCOCK);
        assert!(!kill_flags(false, (false, false), false, None).victims_weapon);
    }

    #[test]
    fn kills_are_reported_to_the_plugins() {
        // #198: 2.1's `PlayerKilled` sends `kill_scored` through the shared report.
        let src = include_str!("scoring.rs");
        assert!(src.contains(concat!("core::kill_report", "(d, &g, victim, attacker, killer_is_player, attacker_gun, &weapon)")));
        assert!(src.contains(concat!("core::report_kill", "(victim, attacker, killer_is_player, report)")));
    }

    #[test]
    fn kill_names_are_2_1s() {
        // Issue #131: "Throwing Knife" for every blade and a bare "Stock"; 3.0 keeps its own.
        assert_eq!(NAMING, core::WeaponNaming { describe_thrown_blades: false, stock_names_weapon: false });
        assert_ne!(NAMING, core::WeaponNaming::DEFAULT);
    }

    #[test]
    fn the_ones_kill_bonus_is_health() {
        assert_eq!(the_one_health(150.0, 4.0), 190.0);
        assert_eq!(the_one_health(150.0, 8.0), 200.0);
        assert_eq!(the_one_health(60.0, 0.0), 60.0);
    }

    #[test]
    fn the_one_keeps_its_award() {
        assert_eq!(award(10, 2), 10);
        assert_eq!(award(0, 4), 4);
        assert_eq!(award(11, 5), 5);
    }
}
