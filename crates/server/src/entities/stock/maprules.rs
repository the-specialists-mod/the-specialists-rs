//! SDK maprules.cpp classes (same in TS): `game_text`, a `TE_TEXTMESSAGE` HUD message to the
//! activator or (spawnflag 1) every player, and `game_score`, points for the activating player or
//! their teammates. Both are gated by an optional `master`.

use super::base::*;
use crate::world::{Cx, Entity, UseType};
use goldsrc::edict::EdictPtr;
use goldsrc::ext::entities::{HudTextParms, is_player, te_text_message};
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::ffi::CStr;

crate::entities::link_entities! {
    game_score => GameScore,
}

pub const SF_ENVTEXT_ALLPLAYERS: i32 = 1;

#[derive(Default)]
pub struct GameText {
    parms: HudTextParms,
    master: string_t,
    delay: Delay,
}

/// `UTIL_StringToIntArray` for up to 4 values (missing ones are 0).
pub fn int_array4(v: &str) -> [i32; 4] {
    let mut out = [0; 4];
    for (o, t) in out.iter_mut().zip(v.split_whitespace()) {
        *o = t.parse::<f32>().map(|f| f as i32).unwrap_or(0);
    }
    out
}

impl Entity for GameText {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        let p = &mut self.parms;
        match key.to_bytes() {
            b"channel" => p.channel = kv_i(value),
            b"x" => p.x = kv_f(value),
            b"y" => p.y = kv_f(value),
            b"effect" => p.effect = kv_i(value),
            b"color" => {
                let c = int_array4(kv_str(value));
                (p.r1, p.g1, p.b1, p.a1) = (c[0] as u8, c[1] as u8, c[2] as u8, c[3] as u8);
            }
            b"color2" => {
                let c = int_array4(kv_str(value));
                (p.r2, p.g2, p.b2, p.a2) = (c[0] as u8, c[1] as u8, c[2] as u8, c[3] as u8);
            }
            b"fadein" => p.fade_in = kv_f(value),
            b"fadeout" => p.fade_out = kv_f(value),
            b"holdtime" => p.hold = kv_f(value),
            b"fxtime" => p.fx_time = kv_f(value),
            b"master" => self.master = alloc(value),
            _ => return self.delay.key_value(key, value),
        }
        true
    }

    fn spawn(&mut self, cx: &mut Cx) {
        let v = cx.pev();
        v.solid = SOLID_NOT as i32;
        v.movetype = MOVETYPE_NONE as i32;
        v.effects = EF_NODRAW as i32;
        v.frame = 0.0;
    }

    fn use_(&mut self, cx: &mut Cx, activator: *mut edict_t, _caller: *mut edict_t, _kind: UseType, _value: f32) {
        if self.master != 0 && !is_master_triggered(self.master, activator) {
            return;
        }
        let msg = s(cx.pev().message);
        if cx.pev().spawnflags & SF_ENVTEXT_ALLPLAYERS != 0 {
            let eng = engine();
            for i in 1..=eng.max_clients() {
                let p = eng.player(i);
                if is_player(p) && p.v().flags & FL_FAKECLIENT as i32 == 0 {
                    te_text_message(p, &self.parms, msg);
                }
            }
        } else if is_player(activator) && activator.v().flags & FL_FAKECLIENT as i32 == 0 {
            te_text_message(activator, &self.parms, msg);
        }
        let me = cx.edict;
        self.delay.use_targets(me, me, UseType::Toggle, 0.0);
    }
}

pub const SF_SCORE_NEGATIVE: i32 = 1;
pub const SF_SCORE_TEAM: i32 = 2;

/// `game_score` (`CGameScore`, 0xcc4d8..0xcc568): `points` (`frags`) to the activator, or with
/// spawnflag 2 to every teammate of the activator (not the activator). Without spawnflag 1 the
/// points can't take a score below zero.
#[derive(Default)]
pub struct GameScore {
    master: string_t,
}

impl Entity for GameScore {
    fn key_value(&mut self, cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"points" => cx.pev().frags = kv_i(value) as f32,
            // CRuleEntity::KeyValue (0xcc380)
            b"master" => self.master = alloc(value),
            _ => return false,
        }
        true
    }

    /// `CRulePointEntity::Spawn` (0xcc42c).
    fn spawn(&mut self, cx: &mut Cx) {
        let v = cx.pev();
        v.solid = SOLID_NOT as i32;
        v.movetype = MOVETYPE_NONE as i32;
        v.effects = EF_NODRAW as i32;
        v.frame = 0.0;
        v.model = 0;
    }

    /// `CGameScore::Use` (0xcc568).
    fn use_(&mut self, cx: &mut Cx, activator: *mut edict_t, _caller: *mut edict_t, _kind: UseType, _value: f32) {
        // CRuleEntity::CanFireForActivator (0xcc3e4)
        if self.master != 0 && !is_master_triggered(self.master, activator) {
            return;
        }
        if !is_player(activator) {
            return;
        }
        let v = cx.pev();
        let (points, allow_negative) = (v.frags as i32, v.spawnflags & SF_SCORE_NEGATIVE != 0);
        if v.spawnflags & SF_SCORE_TEAM != 0 {
            add_points_to_team(activator, points, allow_negative);
        } else {
            add_points(activator, points, allow_negative);
        }
    }
}

/// `CBasePlayer::AddPoints` (0xd9e18): frags plus `ScoreInfo` to everyone.
pub fn add_points(player: *mut edict_t, points: i32, allow_negative: bool) {
    let v = player.v();
    let Some(points) = clamp_points(v.frags, points, allow_negative) else { return };
    v.frags += points as f32;
    // (When the player itself is mid-dispatch the message waits for the next score update.)
    crate::rules::scoring::broadcast_score(player);
}

/// `AddPoints`' floor: without `allow_negative` a score can't be taken below zero (and an
/// already negative one isn't lowered further).
fn clamp_points(frags: f32, points: i32, allow_negative: bool) -> Option<i32> {
    if points < 0 && !allow_negative {
        if frags < 0.0 {
            return None;
        }
        if frags < -points as f32 {
            return Some((-frags) as i32);
        }
    }
    Some(points)
}

/// `CBasePlayer::AddPointsToTeam` (0xd9f68): `AddPoints` for every other player on the same team.
pub fn add_points_to_team(player: *mut edict_t, points: i32, allow_negative: bool) {
    for other in crate::rules::players() {
        if other != player && crate::rules::teams::same_team(player, other) {
            add_points(other, points, allow_negative);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn score_floor() {
        use super::clamp_points;
        assert_eq!(clamp_points(5.0, 3, false), Some(3));
        assert_eq!(clamp_points(5.0, -3, false), Some(-3));
        assert_eq!(clamp_points(2.0, -3, false), Some(-2));
        assert_eq!(clamp_points(-1.0, -3, false), None);
        assert_eq!(clamp_points(2.0, -3, true), Some(-3));
    }

    #[test]
    fn colors() {
        assert_eq!(super::int_array4("255 128 0"), [255, 128, 0, 0]);
        assert_eq!(super::int_array4("1 2 3 4 5"), [1, 2, 3, 4]);
    }
}
