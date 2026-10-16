//! TS 3.0's map-entity rules: the objective system's values, the round's time running out, the
//! win givers, `RespawnEntities` and the ground weapons' `GetWeapon`.

use goldsrc::server::engine;
use goldsrc::sys::{CHAN_ITEM, EF_NODRAW};
use shared::weapons::tables;
use ts_server::entities::stock::explode::emit_sound_slow;
use ts_server::entities::ts::ctb::{self, GiverWin};
use ts_server::entities::ts::ground::{GroundWeapon, buy_price, weapon_valid};
use ts_server::entities::ts::objective::{ManagerTarget, ObjectiveRules, fire_manager_target};
use ts_server::entities::ts::{for_each_class, powerup};
use ts_server::player::Player;
use ts_server::rules::{self, msg, state};
use ts_server::world::{Cx, UseType};

/// TS 3.0's objective rules.
pub const OBJECTIVES: ObjectiveRules = ObjectiveRules {
    hud_effect: 8,
    team_names: None,
    indexed_list: true,
    pointer_enables_givers: true,
    nan_time_runs_out: true,
    clear_list_outside_rounds: true,
};

/// `CTSGameRules::RoundTimeLeft` (0x79024): while a round runs with its timer, the manager's
/// `targetontimeleft`, else a draw (`TeamWin(2, 0, restart)`). The bomb calls it when it
/// explodes.
pub fn round_time_left(_now: f32) {
    let (end, timer) = state::with(|g| (g.round_end, g.round_timer));
    if end == 0.0 || !timer {
        return;
    }
    if !fire_manager_target(ManagerTarget::TimeLeft) {
        ctb::team_win(2, 0, true, false);
    }
}

/// The giver's `TeamWin`: `TeamWin(winnerteam, (int)value, restart, isScore)`; the giver's cash
/// keys are unused.
pub fn giver_team_win(w: &GiverWin) {
    ctb::team_win(w.team, ts_server::entities::ts::c_int(f64::from(w.value)), w.restart, w.is_score);
}

/// `CTSGameRules::RespawnEntities` (0x77764), from `RestartRound`:
/// 1. every broken breakable (`info_breakable` → `BreakableRespawn(false)`);
/// 2. in round modes 2/3 or LMS, `CTSGroundWeapon::Fall` on every `ts_groundweapon` (only that
///    classname: not the HL remaps or ground ammo), which shows a taken one again at once;
/// 3. in LMS, `CTSPowerUp::Fall` on every `ts_powerup`;
/// 4. `CTSFlag::RespawnFlag` on every case.
///
/// TS shares one `UTIL_FindEntityByClassname` cursor between the four loops; each loop only
/// stops early on an entity without an edict (or worldspawn), which no class here can be, so
/// every loop starts from the beginning.
pub fn respawn_entities() {
    ts_server::entities::stock::breakable::respawn_all(false);
    let (round_mode, lms) = state::with(|g| (matches!(g.mode, state::Mode::RoundTdm | state::Mode::Objective), g.lms));
    if round_mode || lms {
        for_each_class(c"ts_groundweapon", |ent, cx| {
            if let Some(g) = ent.downcast_mut::<GroundWeapon>() {
                g.fall(cx);
            }
        });
    }
    if lms {
        for_each_class(c"ts_powerup", |ent, cx| {
            if let Some(p) = ent.downcast_mut::<powerup::PowerUp>() {
                p.fall(cx);
            }
        });
    }
    for_each_class(c"ts_flag", |ent, cx| {
        if let Some(f) = ent.downcast_mut::<ctb::Flag>() {
            f.round_respawn(cx);
        }
    });
}

fn round_mode() -> bool {
    matches!(state::with(|g| g.mode), state::Mode::RoundTdm | state::Mode::Objective)
}

fn hidden(cx: &mut Cx) -> bool {
    cx.pev().effects & EF_NODRAW as i32 != 0
}

/// `CTSGroundWeapon::Spawn` (0xa1b40) turns a kind-4 akimbo pair with a single (`info+0xcc` !=
/// -1) into that single, because `PickUpWeapon` (0x9314c) refuses such pairs.
pub const GROUND_WEAPON_PAIRS_TO_SINGLE: bool = true;

/// `CTSGroundWeapon::GetWeapon` (0xa2328) past its `IsPlayer` test: a live, visible player and
/// a visible weapon (falling or landed). In cash mode a `buyable` one checks `attackerteam`
/// (round modes) and the price first; then `PickUpWeapon(id, NULL, spawnflags, id != 29,
/// false)`, and on success the cash, the hide/respawn, the extra magazines, `message` and
/// `target`, in TS's order.
pub fn give_ground_weapon(g: &mut GroundWeapon, cx: &mut Cx, p: &mut Player, pcx: &mut Cx) {
    if !rules::is_alive(pcx.edict) || hidden(pcx) || hidden(cx) {
        return;
    }
    let flags = cx.pev().spawnflags;
    // A negative `wextraclip` is clamped to 0 for good.
    g.extra_clip = g.extra_clip.max(0);
    let mut cost = 0;
    if state::cash_based() && g.buyable {
        if round_mode() && g.attacker_team != -1 {
            let team = state::with(|r| r.team_index(&p.ts.team_name));
            if team != g.attacker_team {
                msg::ts_message_one(pcx.edict, [0, 0, 0], 16, 30, "Your team cannot buy this weapon.");
                return;
            }
        }
        cost = buy_price(g.id, flags, g.extra_clip);
        if p.cash < cost {
            msg::ts_message_one(pcx.edict, [0, 0, 0], 16, 30, "You don't have enough money for that weapon.");
            return;
        }
    }
    // `TakeWeapon` refuses an id without a valid entry (only a restored id can be one).
    if !weapon_valid(g.id) {
        return;
    }
    // The whole spawnflags go in as the attachments: `TakeWeapon` masks a new record with the
    // allowed attachments, but ORs them unmasked into the single of a new akimbo pair.
    if !ts_server::weapons::give_weapon(p, pcx, g.id as u8, flags as u32) {
        return;
    }
    rules::lifecycle::give_cash(p, -cost);
    g.taken(cx);
    // TS tops up only an empty reserve, checked after the pickup (`reserve[ammo] < 1`), with
    // `CTSGun::AddAmmo(ammo, wextraclip)`: the key's raw number of rounds.
    let extra = g.extra_clip;
    if extra != 0
        && let Some(w) = tables().weapon(g.id as u8)
    {
        let empty = p.inv.gun.reserve.get(w.ammo as usize).is_some_and(|&r| r < 1);
        if empty && ts_server::weapons::give_ammo(p, pcx, w.ammo, extra) > 0 {
            emit_sound_slow(pcx.edict, CHAN_ITEM, c"items/9mmclip1.wav", 1.0, 0.8, 0, 100, 0.5);
        }
    }
    let (message, target) = (cx.pev().message, cx.pev().target);
    if message != 0 {
        msg::ts_message_one(pcx.edict, [0, 0, 0], 8, 30, engine().string(message).to_bytes());
    }
    if target != 0 {
        ts_server::entities::fire_targets(engine().string(target), cx.edict, cx.edict, UseType::Toggle, 0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Ts30Server;
    use ts_server::gameplay::ServerRules;

    #[test]
    fn ground_weapons_turn_pairs_into_singles() {
        assert!(Ts30Server.ground_weapon_pairs_to_single());
    }

    /// The world hooks that need no engine keep TS 3.0's values.
    #[test]
    fn world_hooks_are_ts_3_0() {
        let core: &dyn ServerRules = &Ts30Server;
        assert!(core.break_debris_from_hit(), "3.0 throws the debris along the last hit");
        let fuse = |slow| crate::powerup::fuse_step(ts_server::rules::cvars::pm_arith(), 0.0, slow);
        assert_eq!(core.grenade_fuse_step(0.0, 0.05), fuse(0.05));
        assert_eq!(core.grenade_fuse_step(0.0, 0.001), fuse(0.01), "floored at 0.01");
        assert_eq!(core.grenade_fuse_step(0.0, f32::NAN), fuse(0.01));
        assert_eq!(core.anim_framerate(0.05), 0.05);
        assert_eq!(core.anim_framerate(0.0), 0.01);
        assert_eq!(core.anim_framerate(1.0), 1.0);
        assert_eq!(core.breakable_attack_dir([10.0, 0.0, 0.0], [-1.0; 3], [2.0; 3]), [1.0, 0.0, 0.0]);
    }

    #[test]
    fn objective_rules_are_3_0() {
        let c = OBJECTIVES;
        // Also `CTSSlowMotion::Use` (0x71404) and `CTSDMHill::WaitTouch` (0x71be8): issue #221.
        assert_eq!(c.hud_effect, 8);
        assert!(c.team_names.is_none() && c.indexed_list && c.pointer_enables_givers);
        assert!(c.nan_time_runs_out && c.clear_list_outside_rounds);
    }
}
