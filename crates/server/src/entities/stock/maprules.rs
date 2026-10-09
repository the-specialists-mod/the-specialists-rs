//! `game_text` (SDK maprules.cpp; same in TS): a `TE_TEXTMESSAGE` HUD message to the activator
//! or (spawnflag 1) every player, gated by an optional `master`.

use super::base::*;
use crate::world::{Cx, Entity, UseType};
use goldsrc::ext::entities::{HudTextParms, is_player, te_text_message};
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::ffi::CStr;

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
                if is_player(p) && unsafe { (*p).v.flags } & FL_FAKECLIENT as i32 == 0 {
                    te_text_message(p, &self.parms, msg);
                }
            }
        } else if is_player(activator) && unsafe { (*activator).v.flags } & FL_FAKECLIENT as i32 == 0 {
            te_text_message(activator, &self.parms, msg);
        }
        let me = cx.edict;
        self.delay.use_targets(me, me, UseType::Toggle, 0.0);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn colors() {
        assert_eq!(super::int_array4("255 128 0"), [255, 128, 0, 0]);
        assert_eq!(super::int_array4("1 2 3 4 5"), [1, 2, 3, 4]);
    }
}
