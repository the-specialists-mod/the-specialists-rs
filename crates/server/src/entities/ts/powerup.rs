//! `ts_powerup` (CTSPowerUp, 0x97df8) and `item_longjump` (CTSSuperJump): map power-ups, the
//! temporary ones the auto spawner and dying players leave, pickup and respawn.
//! Gamerules-entities.md §4.1–§4.2.

use crate::entities::stock::animating::Animating;
use crate::entities::stock::base::Delay;
use crate::player::{Player, with_player};
use crate::rules::{self, cvars, msg, state};
use crate::save::{BaseFuncs, Fields};
use crate::world::{Cx, Entity, UseType};
use goldsrc::edict::EdictPtr;
use goldsrc::server::engine;
use goldsrc::sys::*;
use shared::ts::powerup as pw;
use std::ffi::CStr;

pub const MODEL: &CStr = c"models/powerup.mdl";

/// Instant power-ups (always picked up): infinite clip, kung fu, grenade, health, armour, super jump.
pub const INSTANT: u16 = 0x1e6;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    Falling,
    Waiting,
    Hidden,
}

pub struct PowerUp {
    /// `pwuptype` (≥ 1 fixed) and the current type.
    pub kind_key: i32,
    pub kind: u16,
    /// Re-randomise on respawn (`pwuptype < 1`).
    random: bool,
    /// Map power-up (respawns) vs temporary (expires).
    permanent: bool,
    /// Duration given to the player (stored types).
    pub secs: i32,
    /// Temporary power-ups vanish at this time.
    expire: f32,
    /// `pwupduration` (respawn delay key) and `pwupmult`.
    respawn: i32,
    mult: f32,
    phase: Phase,
    /// TS's member pointers: `m_pfnThink` (`Fall` / `WaitThink`) from `Spawn` on, `m_pfnTouch`
    /// (`WaitTouch`) while it waits on the ground.
    funcs: BaseFuncs,
    /// `CBaseDelay` (`delay` / `killtarget`: `KeyValue` falls back to `CBaseDelay::KeyValue`).
    /// Saved, otherwise unused.
    delay: Delay,
    /// `CBaseAnimating`: `Spawn` resets the sequence, `WaitThink` advances it.
    anim: Animating,
}

impl Default for PowerUp {
    fn default() -> Self {
        Self {
            kind_key: 0,
            kind: 0,
            random: false,
            permanent: true,
            secs: 0,
            expire: 0.0,
            respawn: 0,
            mult: 1.0,
            phase: Phase::Falling,
            funcs: BaseFuncs::default(),
            delay: Delay::default(),
            anim: Animating::default(),
        }
    }
}

/// Model body and base duration per type (`SetupPowerUp` 0x98014), first matching bit wins.
pub fn body_and_secs(kind: u16) -> (i32, i32) {
    if kind & pw::SLOWMO != 0 {
        (4, 3)
    } else if kind & pw::INFINITE_CLIP != 0 {
        (2, 3)
    } else if kind & pw::KUNGFU != 0 {
        (0, 0)
    } else if kind & pw::SLOWPAUSE != 0 {
        (9, 2)
    } else if kind & pw::DOUBLE_FIRERATE != 0 {
        (1, 3)
    } else if kind & pw::GRENADE != 0 {
        (5, 0)
    } else if kind & pw::HEALTH != 0 {
        (6, 0)
    } else if kind & pw::ARMOR != 0 {
        (7, 0)
    } else if kind & pw::SUPERJUMP != 0 {
        (8, 0)
    } else {
        (0, 0)
    }
}

/// Respawn delay from `pwupduration`: < 1 → 60 s, 1..10 → 10 s, else min(v, 180).
pub fn respawn_delay(v: i32) -> f32 {
    if v < 1 {
        60.0
    } else if v < 11 {
        10.0
    } else {
        v.min(180) as f32
    }
}

impl PowerUp {
    /// `SetupPowerUp(type, secs, life, permanent)`.
    fn setup(&mut self, cx: &mut Cx, kind: i32, secs: i32, life: f32, permanent: bool) {
        let flags = cx.pev().spawnflags as u16;
        let superjump_map = state::with(|g| g.superjump_map);
        if kind < 1 {
            let mut k = state::with(|g| g.random_powerup(flags & 0x1ff));
            if k == 0 {
                k = state::with(|g| g.random_powerup(0x1ff));
            }
            self.kind = k;
            self.random = true;
        } else {
            let k = kind as u16;
            self.kind =
                if (superjump_map && k & pw::SUPERJUMP != 0) || k & pw::ARMOR != 0 { state::with(|g| g.random_powerup(0x1ff)) } else { k };
            self.random = false;
        }
        let (body, base) = body_and_secs(self.kind);
        cx.pev().body = body;
        self.secs = if secs < 1 { (base as f32 * self.mult).round() as i32 } else { secs };
        self.expire = life + cx.time();
        self.permanent = permanent;
    }

    /// `CTSPowerUp::Fall` (0x983cc): the falling think, and what `RespawnEntities` calls on
    /// every power-up in LMS. Hidden (every 5 s) with `nopowerup 1` or in round modes 2/3;
    /// otherwise it becomes pickable once on the ground, and a temporary one past its life goes.
    /// It leaves the think (`phase`) alone, as TS does, so a waiting power-up hidden from
    /// `RespawnEntities` keeps its `WaitThink` and `WaitTouch`.
    pub fn fall(&mut self, cx: &mut Cx) {
        let hidden_by_rules =
            cvars::f(c"nopowerup") == 1.0 || matches!(state::with(|g| g.mode), state::Mode::RoundTdm | state::Mode::Objective);
        if hidden_by_rules {
            cx.think_in(5.0);
            cx.pev().effects = EF_NODRAW as i32;
            return;
        }
        cx.think_in(0.1);
        if cx.pev().flags & FL_ONGROUND as i32 != 0 {
            self.on_ground(cx);
        }
        if !self.permanent && self.expire < cx.time() {
            crate::entities::remove(cx.edict);
        }
    }

    fn on_ground(&mut self, cx: &mut Cx) {
        let e = cx.edict;
        let v = cx.pev();
        v.solid = SOLID_TRIGGER as i32;
        v.effects &= !(EF_NODRAW as i32);
        v.effects |= 2;
        let org = v.origin;
        engine().set_origin(e, org);
        engine().set_size(e, [-16.0, -16.0, 0.0], [16.0, 16.0, 64.0]);
        self.phase = Phase::Waiting;
        self.funcs.touch = true;
        self.funcs.think = true;
    }

    /// `WaitTouch`: the pickup rules (single held slot, +use swaps).
    fn pickup(&mut self, cx: &mut Cx, p: &mut Player, pcx: &mut Cx) -> bool {
        if !rules::alive(pcx.pev()) {
            return false;
        }
        let using = pcx.pev().button & IN_USE as i32 != 0;
        let (held, held_secs, active) = (p.ts.held, p.ts.held_secs, p.active_powerup);
        let can_store = held == 0 || (held == pw::SLOWMO && self.kind == pw::SLOWMO && held_secs < 3);
        let instant = self.kind & INSTANT != 0;
        let take = ((can_store || using) && active == 0) || instant;
        if !take {
            return false;
        }
        let mut swap = using && held != 0 && !instant;
        if held == pw::SLOWMO && swap {
            swap = held_secs > 2;
        }
        let drop = if swap { held } else { 0 };
        if !rules::powerup::add_powerup(p, pcx, self.kind, self.secs) {
            return false;
        }
        if drop != 0 {
            create(drop, -1, pcx.pev().origin);
        }
        let _ = cx;
        true
    }

    fn after_pickup(&mut self, cx: &mut Cx, player: *mut edict_t) {
        if !self.permanent {
            crate::entities::remove(cx.edict);
            return;
        }
        if self.random {
            self.setup(cx, 0, -1, 0.0, true);
        }
        self.phase = Phase::Hidden;
        self.funcs.touch = false;
        self.funcs.think = true;
        let v = cx.pev();
        v.effects |= EF_NODRAW as i32;
        v.solid = SOLID_NOT as i32;
        v.nextthink = engine().time() + respawn_delay(self.respawn);
        let (message, target) = (v.message, v.target);
        if message != 0 {
            msg::ts_message_one(player, [0, 0, 0], 1, 30, &engine().string(message).to_string_lossy());
        }
        if target != 0 {
            crate::entities::fire_targets(engine().string(target), cx.edict, cx.edict, UseType::Toggle, 0.0);
        }
    }
}

impl Entity for PowerUp {
    /// `CTSPowerUp::Save` / `Restore` (0x97e1c / 0x97e6c): `poweruptype` is the current type
    /// (`+0x84`, the `pwuptype` key until `SetupPowerUp` replaces it), `pwupduration` the respawn
    /// key (`+0x94`). `m_pfnThink` is set from `Spawn` on, so TS's restore never gets past
    /// `"BASE"`.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut Fields) {
        super::save_base(cx, f, &mut self.funcs);
        f.delay(&mut self.delay.delay, &mut self.delay.kill_target);
        super::save_animating(f, &mut self.anim);
        f.table(&crate::save::tables::CTSPowerUp);
        // The port keeps the type in 16 bits (`SetupPowerUp`'s bit tests only read 9).
        let mut kind = self.kind as i32;
        f.int(&mut kind);
        self.kind = kind as u16;
        if f.restoring() {
            self.kind_key = kind;
        }
        f.int(&mut self.respawn);
    }

    /// `CTSPowerUp::Precache` (0x98548).
    fn restored(&mut self, _cx: &mut Cx) {
        engine().precache_model(MODEL);
    }

    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        let v = value.to_str().unwrap_or("").trim();
        match key.to_bytes() {
            b"pwuptype" => self.kind_key = v.parse().unwrap_or(0),
            b"pwupduration" => self.respawn = v.parse().unwrap_or(0),
            b"pwupmult" => self.mult = v.parse().unwrap_or(0.0),
            _ => return self.delay.key_value(key, value),
        }
        true
    }

    fn spawn(&mut self, cx: &mut Cx) {
        let e = cx.edict;
        let eng = engine();
        eng.precache_model(MODEL);
        if eng.string(cx.pev().classname) == c"item_longjump" {
            // CTSSuperJump: a super jump power-up; re-enables the automatic spawner.
            self.kind_key = pw::SUPERJUMP as i32;
            state::with(|g| g.powerup_spawner = 0xff);
        } else {
            // A map with its own power-ups gets no automatic ones.
            state::with(|g| {
                if g.powerup_spawner == 0xff {
                    g.powerup_spawner = 0;
                }
            });
        }
        cx.pev().classname = eng.alloc_string(c"ts_powerup");
        eng.set_model(e, MODEL);
        let v = cx.pev();
        v.movetype = MOVETYPE_TOSS as i32;
        v.solid = SOLID_BBOX as i32;
        v.renderamt = 255.0;
        v.rendermode = kRenderTransColor as i32;
        v.rendercolor = [0.0; 3];
        v.renderfx = kRenderFxNoDissipation as i32;
        let org = v.origin;
        eng.set_origin(e, org);
        eng.set_size(e, [0.0; 3], [0.0; 3]);
        self.funcs = BaseFuncs { think: true, ..BaseFuncs::default() };
        cx.think_in(0.1);
        self.anim.reset_sequence_info(e);
        cx.pev().framerate = 1.0;
        self.phase = Phase::Falling;
        let k = self.kind_key;
        self.setup(cx, k, -1, 0.0, true);
    }

    fn think(&mut self, cx: &mut Cx) {
        if !self.funcs.think {
            return;
        }
        let now = cx.time();
        match self.phase {
            Phase::Falling | Phase::Hidden => self.fall(cx),
            Phase::Waiting => {
                // `WaitThink` (0x98570): temporary power-ups fade over their last 30 s.
                self.anim.frame_advance(cx.edict, 0.0);
                cx.think_in(1.0);
                if !self.permanent {
                    if now <= self.expire {
                        cx.pev().renderamt = ((self.expire - now) / 30.0 * 255.0).min(255.0);
                    } else {
                        crate::entities::remove(cx.edict);
                    }
                }
            }
        }
    }

    fn touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        if !self.funcs.touch || self.phase != Phase::Waiting || !rules::is_player(other) {
            return;
        }
        let took = with_player(other, |p, pcx| self.pickup(cx, p, pcx)).unwrap_or(false);
        if took {
            self.after_pickup(cx, other);
        }
    }
}

/// `CTSPowerUp::CreatePowerUp(type, secs)`: a temporary power-up (60 s) at `origin`.
pub fn create(kind: u16, secs: i32, origin: [f32; 3]) -> *mut edict_t {
    let eng = engine();
    let e = eng.create_named_entity(c"ts_powerup");
    if e.is_null() {
        msg::text_all(shared::protocol::rules::hud_dest::TALK, "WARNING:cannot create powerup!\n");
        return e;
    }
    (e.v().origin = origin);
    crate::world::dispatch(e, |ent, cx| {
        ent.spawn(cx);
        if let Some(pu) = (ent as &mut dyn std::any::Any).downcast_mut::<PowerUp>() {
            pu.setup(cx, kind as i32, secs, 60.0, false);
        }
        let org = cx.pev().origin;
        engine().set_origin(cx.edict, org);
    });
    e
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bodies() {
        assert_eq!(body_and_secs(pw::SLOWMO), (4, 3));
        assert_eq!(body_and_secs(pw::SLOWPAUSE), (9, 2));
        assert_eq!(body_and_secs(pw::DOUBLE_FIRERATE), (1, 3));
        assert_eq!(body_and_secs(pw::SUPERJUMP), (8, 0));
        assert_eq!(body_and_secs(pw::KUNGFU), (0, 0));
    }

    #[test]
    fn respawn_clamps() {
        assert_eq!(respawn_delay(0), 60.0);
        assert_eq!(respawn_delay(5), 10.0);
        assert_eq!(respawn_delay(30), 30.0);
        assert_eq!(respawn_delay(999), 180.0);
    }
}
