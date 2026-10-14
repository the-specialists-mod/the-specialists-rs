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
use ts_server::rules::{cvars, has_player, len3, netname, other, teams};
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
    let slide = !stunting && av.iuser4 & PRONE != 0 && av.friction < 1.0 && len3(av.velocity) > 80.0;
    // The port's kill log tags (3.0's) are kept for the flags; 2.1 logs no tags.
    *diving = stunting;
    *sliding = slide;
    let (pts, label) = points(kind, base, stunting, slide, true);
    if let Some(label) = label {
        ts_message_all(GREEN, 1, 30, &format!("{pts} frags for {}, {label}", netname(attacker)));
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

    // DeathNotice first (2.1 `FUN_1004ede0`), with its log line.
    let attacker_gun = if suicide { core::active_gun(p) } else { other(attacker, |k| core::active_gun(k)).flatten() };
    let weapon = core::weapon_name_with(d, &g, attacker_gun, NAMING);
    core::death_notice(attacker, d.inflictor, victim, &weapon);
    core::log_kill_untagged(attacker, victim, &weapon, suicide);
    p.deaths += 1;
    ts_server::world::lend(victim, p, || ts_server::entities::fire_targets(c"game_playerdie", victim, victim, UseType::Toggle, 0.0));
    if p.ts.is_the_one {
        core::the_one_is_dead(p, cx, killer_is_player.then_some(attacker));
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
            let kv = attacker.v();
            if (kv.iuser4 & STUNTING != 0 || g.close_combat || points > 2) && core::do_spectacularness() {
                ts_server::entities::ts::create_slow_motion_point(cx.pev().origin, 0.15, 1.5, false);
            }
            let dm = state::with(|g| matches!(g.mode, Mode::Dm | Mode::TeamDm));
            if dm && points > 0 {
                spree(p.ts.spree, &netname(victim), k, &mut points, now);
            }
            kv.frags += points as f32;
            other(k.e, |kp| kp.frags = kv.frags as i32);
            ts_server::entities::fire_targets(c"game_playerkill", k.e, k.e, UseType::Toggle, 0.0);
            if cvars::f(c"slowmatch") == 1.0 && points > 0 && (k.held == 0 || k.held & pw::SLOWMO != 0) && k.active == 0 {
                k.active = 0;
                k.held = pw::SLOWMO;
                k.held_secs = (k.held_secs + 1).min(5);
            }
            if points > 0 {
                other(k.e, |kp| ts_server::bots::on_kill(kp, victim, now));
            }
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
}

/// The DM/TDM part of `PlayerKilled` for a scoring kill: double kill (1 s, always updating the
/// last kill), stopping a spree, killing The Specialist, the killer's spree rewards, the overheal
/// timer and the kill cash.
fn spree(victim_spree: u8, vname: &str, k: &mut Killer, points: &mut i32, now: f32) {
    let kname = netname(k.e);
    if now - k.last_kill < 1.0 {
        *points *= 2;
        ts_message_all(GREEN, 1, 30, &format!("DoubleFrag for {kname}!"));
    }
    if victim_spree > 2 {
        if victim_spree < 9 {
            ts_message_all(GREEN, 1, 30, &format!("{kname} stops {vname}'s fury!"));
        } else {
            *points += 5;
            ts_message_all(GREEN, 1, 40, &format!("{} frags for {kname}, he has killed THE SPECIALIST!", *points));
            store_slow_pause(k);
        }
    }
    // `if (spree != 255) spree++`.
    k.spree = k.spree.saturating_add(1);
    let kv = k.e.v();
    match k.spree {
        3 => {
            ts_message_all(GREEN, 1, 30, &format!("{kname} is a Killer!"));
            kv.health = (kv.health + 30.0).min(150.0);
            k.award = award(k.award, 2);
        }
        6 => {
            ts_message_all(GREEN, 1, 30, &format!("{kname} is Demolition Man!"));
            kv.health = (kv.health + 50.0).min(150.0);
            k.award = award(k.award, 3);
        }
        9 => {
            ts_message_all(GREEN, 1, 30, &format!("{kname} is The Specialist!"));
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
            ts_message_all(GREEN, 1, 30, &format!("{kname} is Unstoppable! {s} consecutive frags"));
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
    fn kill_names_are_2_1s() {
        // Issue #131: "Throwing Knife" for every blade and a bare "Stock"; 3.0 keeps its own.
        assert!(!NAMING.describe_thrown_blades && !NAMING.stock_names_weapon);
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
