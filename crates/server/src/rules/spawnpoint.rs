//! Spawn point selection: `EntSelectSpawnPoint` (0x76128) and `TSSelectSpawnSpot` (0x76244),
//! then the SDK `GetPlayerSpawnSpot` placement. Gamerules-entities.md §2.1.

use super::{alive, eye, state, teams};
use crate::damage::{DamageInfo, bits};
use crate::player::Player;
use crate::world::{Cx, UseType};
use goldsrc::edict::EdictPtr;
use goldsrc::server::{IGNORE_MONSTERS, engine};
use goldsrc::sys::*;
use goldsrc::util::to_cstring;

/// Score of one candidate spot given what the players around it can see.
/// `enemy_sees`: an enemy (or a teammate closer than 128) has line of sight.
pub fn spot_score(enemy_sees: bool, teammate_sees: bool, min_dist: i32, rand: impl FnOnce(i32, i32) -> i32) -> i32 {
    if !enemy_sees && teammate_sees {
        rand(6145, 8192)
    } else if !enemy_sees {
        rand(4097, 6144)
    } else {
        min_dist
    }
}

/// Picks the spot for `self_e` and returns it (null if the map has none).
/// `TSSelectSpawnSpot`'s two `(spot - player).Length()`s: whether it is at least 128 (the
/// teammate test) and its `(int)` (truncated). mp.dll (`FUN_1004c4a0`, 0x1004c66e..0x1004c6e0)
/// sums the unrounded differences `z, y, x` and keeps the root (`__ftol` truncates it unrounded);
/// `ts_i386.so` (0x76409..0x764b8) sums `x, y, z` and stores the root before both.
fn spot_distance<F: shared::pm::Fpu>(spot: [f32; 3], player: [f32; 3]) -> (bool, i32) {
    use shared::pm::Num;
    let d = [0, 1, 2].map(|i| F::x(spot[i]) - F::x(player[i]));
    let len =
        if F::MSVC { (d[2] * d[2] + d[1] * d[1] + d[0] * d[0]).sqrt() } else { F::x((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().f()) };
    (len >= F::x(128.0), len.trunc_i32())
}

pub fn select(self_e: *mut edict_t) -> *mut edict_t {
    let eng = engine();
    let (class, fallback) = if state::teamplay() {
        let team = teams::team_name_of(self_e);
        state::with(|g| match g.team_index(&team) {
            i if i >= 0 => (g.teams[i as usize].spawn_class.clone(), Some(g.teams[i as usize].fallback.clone())),
            _ => ("info_player_deathmatch".to_owned(), None),
        })
    } else {
        ("info_player_deathmatch".to_owned(), None)
    };
    let mut class_c = to_cstring(&class);
    if let Some(f) = fallback
        && eng.find_by_string(std::ptr::null_mut(), c"classname", &class_c).is_null()
    {
        class_c = to_cstring(&f);
    }

    // TS walks the classname "player" with `IsPlayer()` (0x76324): players who left are on the
    // list, and the `IsAlive`/`EF_NODRAW` tests below skip them.
    let players = super::slot_players();
    let mut spots: Vec<(*mut edict_t, i32)> = Vec::new();
    let mut it = std::ptr::null_mut();
    while spots.len() < 32 {
        it = eng.find_by_string(it, c"classname", &class_c);
        if it.is_null() {
            break;
        }
        // `CBaseDMStart::IsTriggered` (0xe2c20): a spot whose `master` (in `netname`) isn't
        // triggered for this player is skipped without taking one of the 32 places.
        if !crate::entities::stock::base::is_master_triggered(it.v().netname, self_e) {
            continue;
        }
        let sv = it.v();
        let mut enemy_sees = false;
        let mut teammate_sees = false;
        let mut min_dist = 4096;
        for &o in &players {
            if o == self_e {
                continue;
            }
            let ov = o.v();
            if !alive(ov) || ov.effects & EF_NODRAW as i32 != 0 {
                continue;
            }
            let (far, d) = shared::by_arith!(super::cvars::pm_arith(), spot_distance(sv.origin, ov.origin));
            if !enemy_sees {
                let tr = eng.trace_line(sv.origin, eye(ov), IGNORE_MONSTERS, it);
                if tr.flFraction == 1.0 {
                    if state::teamplay() && teams::same_team(self_e, o) && far {
                        teammate_sees = true;
                    } else {
                        enemy_sees = true;
                    }
                }
            }
            min_dist = min_dist.min(d);
        }
        let score = spot_score(enemy_sees, teammate_sees, min_dist, |a, b| eng.random_long(a, b));
        spots.push((it, score));
    }

    if spots.is_empty() {
        let mut s = eng.find_by_string(std::ptr::null_mut(), c"classname", c"info_player_deathmatch");
        if s.is_null() {
            s = eng.find_by_string(std::ptr::null_mut(), c"classname", c"info_player_start");
        }
        return s;
    }
    let (spot, best) = spots.iter().copied().fold((spots[0].0, i32::MIN), |acc, (s, sc)| if sc > acc.1 { (s, sc) } else { acc });
    if best < 128 {
        telefrag(spot, self_e);
    }
    spot
}

/// The spawn telefrag's damage bits: `DMG_NEVERGIB` (0x1000 at 0x6675a file address in 3.0,
/// 0x1004335e in 2.1), so the body is never gibbed.
const TELEFRAG_BITS: i32 = bits::NEVERGIB;

/// Kills every living, visible player within 128 units of the spot (300 damage, never gib).
fn telefrag(spot: *mut edict_t, self_e: *mut edict_t) {
    let eng = engine();
    let org = spot.v().origin;
    let world = eng.entity_of_index(0);
    let mut it = std::ptr::null_mut();
    loop {
        it = eng.find_in_sphere(it, org, 128.0);
        if it.is_null() {
            break;
        }
        if it == self_e || !super::has_player(it) || !alive(it.v()) || it.v().effects & EF_NODRAW as i32 != 0 {
            continue;
        }
        crate::damage::apply(it, &DamageInfo::new(world, world, 300.0, TELEFRAG_BITS));
    }
}

/// `GetPlayerSpawnSpot`: select, place at origin + 1 z, copy angles, then
/// (`CHalfLifeMultiplay::GetPlayerSpawnSpot` only, not `CGameRules`'s 0xc5e2c that single player
/// keeps) fire the spot's target.
pub fn place(_p: &mut Player, cx: &mut Cx) {
    let eng = engine();
    let spot = select(cx.edict);
    let (origin, angles) = if spot.is_null() {
        ([0.0, 0.0, 64.0], [0.0; 3])
    } else {
        let s = spot.v();
        ([s.origin[0], s.origin[1], s.origin[2] + 1.0], s.angles)
    };
    {
        let v = cx.pev();
        v.velocity = [0.0; 3];
        v.angles = angles;
        v.v_angle = angles;
        v.punchangle = [0.0; 3];
        v.fixangle = 1;
    }
    eng.set_origin(cx.edict, origin);
    if !spot.is_null() && !super::state::single_player() {
        let sv = spot.v();
        if sv.target != 0 {
            let t = eng.string(sv.target);
            crate::entities::fire_targets(t, cx.edict, cx.edict, UseType::Toggle, 0.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{TELEFRAG_BITS, spot_distance, spot_score};
    use crate::damage::bits;

    /// The spot's distance to a player (issue #171): mp.dll truncates the unrounded 740.99998 to
    /// 740; the `.so` stores 741.0, as the port's old `float` length did.
    #[test]
    fn spot_distance_rounds_as_each_build() {
        use shared::pm::{Gcc, Msvc};
        let f = |b: [u32; 3]| b.map(f32::from_bits);
        let (a, b) = (f([0xc2da_01de, 0x445a_d2ed, 0x44bd_680a]), f([0x4285_7810, 0x431b_6fc8, 0x44bd_680a]));
        assert_eq!(spot_distance::<Msvc>(a, b), (true, 740));
        assert_eq!(spot_distance::<Gcc>(a, b), (true, 741));
        assert_eq!(spot_distance::<Msvc>([0.0; 3], [127.0, 0.0, 0.0]), (false, 127));
    }

    #[test]
    fn spawn_telefrags_never_gib() {
        // Issue #128: TS passes 0x1000 to TakeDamage.
        assert_eq!(TELEFRAG_BITS, 0x1000);
        assert_eq!(TELEFRAG_BITS, bits::NEVERGIB);
    }

    #[test]
    fn scoring_bands() {
        // Hidden from everyone but watched by a distant teammate: top band.
        assert_eq!(spot_score(false, true, 50, |a, _| a), 6145);
        // Hidden from everyone: second band.
        assert_eq!(spot_score(false, false, 50, |_, b| b), 6144);
        // Seen by an enemy: the distance to the closest player.
        assert_eq!(spot_score(true, true, 300, |_, _| unreachable!()), 300);
        // Any hidden spot beats any seen one (distances are capped at 4096).
        assert!(spot_score(false, false, 0, |a, _| a) > spot_score(true, false, 4096, |a, _| a));
    }
}
