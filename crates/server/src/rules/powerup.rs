//! Player power-ups and slow motion: `PlayerAddPowerUp` (0x97bc8), `UsePowerUp` (0x95980),
//! `UpdatePowerUpState` (0x95748), `SlowMotionFlow` (0x96bb0), `DeactivateSlowMotionFlow`
//! (0x96a9c), `GoSlow` (0x929d0), `InterpolateSlowMotion` (0x9910c).
//! Reference: player-movement.md §5, gamerules-entities.md §4.

use super::msg::{self, ts_message_one};
use super::{alive, eye, other, state};
use crate::messages::Msg;
use crate::player::Player;
use crate::world::Cx;
use goldsrc::edict::EdictPtr;
use goldsrc::server::{IGNORE_MONSTERS, engine};
use goldsrc::sys::*;
use shared::protocol::hud_wire::{PwUp, PwUpTime, TsMBlur};
use shared::ts::powerup as pw;

/// Slow amounts while a power-up is active: (self, others, bullet-time flag).
pub const SLOWMO: (f32, f32, bool) = (0.6, 0.35, false);
pub const SLOWPAUSE: (f32, f32, bool) = (0.8, 0.225, true);

/// `InterpolateSlowMotion`: exponential approach at ~7/s, snapping within 0.05.
pub fn interpolate(frametime: f32, cur: &mut f32, target: f32) {
    let mut d = target - *cur;
    if d.abs() > 0.05 {
        d *= (frametime * 7.0).min(1.0);
    }
    *cur += d;
}

/// `CBasePlayer::GoSlow` on the dispatched player.
pub fn go_slow_self(p: &mut Player, cx: &mut Cx, amount: f32, btime: bool) {
    p.slow_target = if cx.pev().iuser1 != 0 { 1.0 } else { amount };
    p.ts.slow_display = amount;
    p.ts.btime = btime;
}

/// `GoSlow` on any entity: players through their Rust state, others directly (`fuser1`).
pub fn go_slow(e: *mut edict_t, amount: f32, btime: bool) {
    if e.is_null() {
        return;
    }
    let observer = e.v().iuser1 != 0;
    let done = other(e, |p| {
        p.slow_target = if observer { 1.0 } else { amount };
        p.ts.slow_display = amount;
        p.ts.btime = btime;
    });
    if done.is_none() && !super::is_player(e) {
        (e.v().fuser1 = amount);
    }
}

/// `InSlow` for players: target or displayed amount differs from 1.
pub fn in_slow(p: &Player) -> bool {
    p.slow_target != 1.0 || p.ts.slow_display != 1.0
}

/// `InSlow` for any entity.
pub fn entity_in_slow(e: *mut edict_t) -> bool {
    other(e, |p| in_slow(p)).unwrap_or_else(|| e.v().fuser1 != 1.0)
}

/// `UpdatePowerUpState`: activator amounts, expiry, dropping on death.
pub fn update_state(p: &mut Player, cx: &mut Cx) {
    if p.ts.held == 0 {
        return;
    }
    let now = cx.time();
    let is_alive = alive(cx.pev());
    let continuous = matches!(state::with(|g| g.mode), state::Mode::Dm | state::Mode::TeamDm);
    let drop_worthy = |bits: u16, secs: i32| bits & pw::SLOWMO == 0 || secs > 2;
    if p.active_powerup == 0 {
        if is_alive {
            return;
        }
        if continuous && drop_worthy(p.ts.held, p.ts.held_secs) {
            drop_powerup(cx.pev().origin, p.ts.held, p.ts.held_secs);
        }
    } else {
        let active = p.active_powerup;
        if active & (pw::SLOWMO | pw::SLOWPAUSE) == 0 {
            if active & pw::SUPERJUMP != 0 {
                cx.pev().gravity = 0.15;
            }
        } else if p.powerup_until < now || !is_alive {
            deactivate_flow(p, cx);
        } else if active & pw::SLOWPAUSE != 0 {
            (p.ts.slow_self, p.ts.slow_others, p.ts.btime) = SLOWPAUSE;
        } else {
            (p.ts.slow_self, p.ts.slow_others, p.ts.btime) = SLOWMO;
        }
        if now <= p.powerup_until && is_alive {
            return;
        }
        if !is_alive && continuous && drop_worthy(active, p.ts.held_secs) {
            drop_powerup(cx.pev().origin, active, p.ts.held_secs);
        }
        if active & pw::SUPERJUMP != 0 {
            cx.pev().gravity = 1.0;
        }
    }
    p.powerup_until = 0.0;
    p.ts.held_secs = 0;
    p.active_powerup = 0;
    p.ts.held = 0;
}

/// Drops a power-up as a temporary `ts_powerup` (60 s life).
pub fn drop_powerup(origin: [f32; 3], bits: u16, secs: i32) {
    crate::entities::ts::create_powerup(bits, secs, origin);
}

/// `SlowMotionFlow`: every 0.5 s an activator slows every player and projectile it can see.
pub fn slow_flow(p: &mut Player, cx: &mut Cx) {
    let now = cx.time();
    if p.ts.slow_self == 0.0 || p.ts.next_flow > now {
        return;
    }
    p.ts.next_flow = now + 0.5;
    let eng = engine();
    let me = cx.edict;
    let my_eye = eye(cx.pev());
    let mine = p.ts.slow_others;
    let mut abort = false;
    for i in 1..=eng.max_clients().min(32) {
        let o = eng.client(i);
        let i = i as usize;
        if p.ts.slowed & (1 << i) != 0 {
            // Someone we slowed became an activator: re-evaluate.
            let activator = !o.is_null() && other(o, |q| q.ts.slow_self != 0.0).unwrap_or(false);
            if !activator {
                continue;
            }
            p.ts.slowed &= !(1 << i);
        }
        if o.is_null() || o == me || !super::is_player(o) {
            continue;
        }
        let Some((oself, oothers)) = other(o, |q| (q.ts.slow_self, q.ts.slow_others)) else { continue };
        if oself != 0.0 && oothers == mine {
            continue;
        }
        let tr = eng.trace_line(my_eye, eye(o.v()), IGNORE_MONSTERS, me);
        if tr.flFraction >= 1.0 {
            if oself == 0.0 || mine <= oothers {
                p.ts.slowed |= 1 << i;
            } else {
                // A stronger slow wins.
                p.ts.slowed &= !(1 << i);
                abort = true;
                break;
            }
        }
    }
    if abort {
        return;
    }
    let (self_amt, btime) = (p.ts.slow_self, p.ts.btime);
    go_slow_self(p, cx, self_amt, btime);
    for i in 1..=eng.max_clients().min(32) {
        if p.ts.slowed & (1 << i) == 0 {
            continue;
        }
        let o = eng.client(i);
        let keep = !o.is_null() && other(o, |q| q.ts.slow_self == 0.0 || q.ts.slow_others != mine).unwrap_or(false);
        if keep {
            go_slow(o, mine, btime);
        } else {
            p.ts.slowed &= !(1 << i);
        }
    }
    // Projectiles in sight: grenades, thrown knives and dropped guns that are moving.
    for class in [c"grenade", c"knife", c"WorldGun"] {
        let mut it = std::ptr::null_mut();
        loop {
            if p.ts.slowed_ents.len() >= 32 {
                return;
            }
            it = eng.find_by_string(it, c"classname", class);
            if it.is_null() {
                break;
            }
            let v = it.v();
            if class != c"grenade" && v.velocity == [0.0; 3] {
                continue;
            }
            if p.ts.slowed_ents.contains(&it) {
                continue;
            }
            let tr = eng.trace_line(my_eye, v.origin, IGNORE_MONSTERS, me);
            if tr.flFraction >= 1.0 {
                go_slow(it, mine, false);
                p.ts.slowed_ents.push(it);
            }
        }
    }
}

/// `DeactivateSlowMotionFlow`: back to normal time for us and everything we slowed.
pub fn deactivate_flow(p: &mut Player, cx: &mut Cx) {
    go_slow_self(p, cx, 1.0, false);
    p.ts.slow_self = 0.0;
    p.ts.slow_others = 0.0;
    p.ts.btime = false;
    let eng = engine();
    for i in 1..=eng.max_clients().min(32) {
        if p.ts.slowed & (1 << i) == 0 {
            continue;
        }
        p.ts.slowed &= !(1 << i);
        let o = eng.client(i);
        if o.is_null() {
            continue;
        }
        other(o, |q| {
            if q.ts.slow_self == 0.0 {
                q.slow_target = 1.0;
                q.ts.slow_display = 1.0;
                q.ts.btime = false;
            }
        });
    }
    for e in std::mem::take(&mut p.ts.slowed_ents) {
        // The entity may be gone; only touch edicts that are still in use.
        if !e.is_free() {
            go_slow(e, 1.0, false);
        }
    }
}

/// `UsePowerUp`: activate the held power-up (`usepowerup`, or +use double tap).
pub fn use_powerup(p: &mut Player, cx: &mut Cx) -> bool {
    if cx.pev().iuser1 != 0 || !alive(cx.pev()) || p.ts.held == 0 || p.active_powerup != 0 {
        return false;
    }
    let now = cx.time();
    if state::with(|g| g.slow_blocked) && p.ts.held & (pw::SLOWMO | pw::SLOWPAUSE) != 0 {
        ts_message_one(cx.edict, [195, 45, 45], 16, 20, "You can't use Slow-Motion now!");
        return false;
    }
    p.active_powerup = p.ts.held;
    p.powerup_until = now + p.ts.held_secs as f32;
    let secs = (p.powerup_until - now).round().clamp(0.0, 255.0) as u8;
    msg::one(Msg::PwUpTime, cx.edict, |w| PwUpTime { secs }.write(w));
    let mode = if p.ts.held & pw::SLOWMO != 0 {
        1
    } else if p.ts.held & pw::SLOWPAUSE != 0 {
        2
    } else {
        0
    };
    if mode != 0 {
        let b = TsMBlur { entindex: cx.index() as u8, mode };
        msg::pvs(Msg::TsMBlur, cx.pev().origin, |w| b.write(w));
    }
    true
}

/// `PlayerAddPowerUp`: apply a pickup. `secs` is the power-up entity's duration. Returns false
/// when the pickup failed (no gun to multiply, grenade didn't fit).
pub fn add_powerup(p: &mut Player, cx: &mut Cx, bits: u16, secs: i32) -> bool {
    let e = cx.edict;
    if bits & pw::ARMOR != 0 {
        take_armor(p, cx, 100);
    } else if bits & pw::KUNGFU != 0 {
        if !p.kungfu {
            p.kungfu = true;
            msg::one(Msg::PwUp, e, |w| PwUp { bits: pw::KUNGFU as i16, secs: 0 }.write(w));
        }
    } else if bits & pw::SUPERJUMP != 0 {
        if !p.superjump {
            p.superjump = true;
            engine().set_physics_key(e, c"mj", c"1");
            msg::one(Msg::PwUp, e, |w| PwUp { bits: pw::SUPERJUMP as i16, secs: 0 }.write(w));
        }
    } else if bits & pw::HEALTH != 0 {
        let v = cx.pev();
        v.health = v.health.max(150.0);
        p.ts.health_timer = cx.time() + 1.0;
    } else if bits & pw::GRENADE != 0 {
        if !crate::weapons::give_weapon(p, cx, 24, 0) {
            return false;
        }
    } else if bits & pw::INFINITE_CLIP != 0 {
        if !crate::weapons::multiply_clip(p, cx) {
            return false;
        }
    } else {
        p.active_powerup = 0;
        p.ts.held = bits;
        p.powerup_until = 0.0;
        p.ts.held_secs = secs;
    }
    engine().emit_sound(e, CHAN_ITEM as i32, c"items/gunpickup2.wav", 1.0, 0.8, 0, 100);
    true
}

/// `CBasePlayer::TakeArmor(n)`: the kevlar item costs 10 slots the first time; armour caps at 100.
pub fn take_armor(p: &mut Player, cx: &mut Cx, n: i32) -> bool {
    if n < 0 {
        if p.ts.kevlar {
            p.slots_used -= 10;
            p.ts.kevlar = false;
            cx.pev().armorvalue = 0.0;
        }
    } else {
        if !p.ts.kevlar {
            if shared::ts::SLOTS_TOTAL - p.slots_used < 10 {
                return false;
            }
            p.slots_used += 10;
            p.ts.kevlar = true;
            engine().emit_sound(cx.edict, CHAN_ITEM as i32, c"weapons/items.wav", 1.0, 0.8, 0, 100);
        }
        let v = cx.pev();
        v.armorvalue = (v.armorvalue + n as f32).min(100.0);
    }
    if !p.ts.is_the_one {
        let speed = shared::ts::speed_for_free_slots(shared::ts::SLOTS_TOTAL - p.slots_used);
        cx.pev().maxspeed = speed;
        engine().set_client_maxspeed(cx.edict, speed);
    }
    true
}

/// The `TSSlowMo` value: 0.225 under slow pause, 0.35 under slow motion, else the display amount.
pub fn hud_slow_amount(p: &Player) -> f32 {
    if p.active_powerup & pw::SLOWPAUSE != 0 {
        0.225
    } else if p.active_powerup & pw::SLOWMO != 0 {
        0.35
    } else {
        p.ts.slow_display
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn easing() {
        let mut v = 1.0;
        super::interpolate(0.1, &mut v, 0.35);
        // 70 % of the way in 0.1 s
        assert!((v - (1.0 - 0.65 * 0.7)).abs() < 1e-5);
        let mut v = 0.37;
        super::interpolate(0.01, &mut v, 0.35);
        assert_eq!(v, 0.35, "snaps within 0.05");
        let mut v = 1.0;
        super::interpolate(1.0, &mut v, 0.5);
        assert_eq!(v, 0.5, "factor capped at 1");
    }
}
