//! `ts_powerup` (CTSPowerUp, 0x97df8) and `item_longjump` (CTSSuperJump): map power-ups, the
//! temporary ones the auto spawner and dying players leave, pickup and respawn.
//! Gamerules-entities.md §4.1–§4.2.

use crate::entities::stock::animating::Animating;
use crate::entities::stock::base::Delay;
use crate::player::{Player, with_player};
use crate::rules::powerup::PowerupRules;
use crate::rules::{self, cvars, msg, state};
use crate::save::{BaseFuncs, Fields};
use crate::world::{Cx, Entity, UseType};
use goldsrc::edict::EdictPtr;
use goldsrc::server::engine;
use goldsrc::sys::*;
use shared::ts::powerup as pw;
use std::ffi::CStr;

pub const MODEL: &CStr = c"models/powerup.mdl";

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
    /// The armour an armour power-up gives (`ServerRules::armor_powerup`): 100.
    pub armor: f32,
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
            armor: 100.0,
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

/// Model body per type (`SetupPowerUp` 0x98014), first matching bit wins. With no known bit TS
/// leaves `pev->body` as it was (`None`). The base durations are the ruleset's
/// (`rules::powerup::PowerupRules::base`).
pub fn body(kind: u16) -> Option<i32> {
    // slow motion, infinite clip, kung fu, slow pause, double firerate, grenade, health, armour,
    // super jump
    const BODIES: [i32; 9] = [4, 2, 0, 9, 1, 5, 6, 7, 8];
    (0..9).find(|i| kind & (1 << i) != 0).map(|i| BODIES[i])
}

/// `SetupPowerUp`'s type (0x98014) for the type key `key` (`pwuptype`) and `spawnflags`, and
/// whether it is drawn again on each respawn: a key below 1 draws from the spawnflags' types
/// (all when none), never armour; armour, and the super jump on a map that gives it to everyone,
/// are drawn from all.
pub fn kind_for(key: i32, spawnflags: i32) -> (u16, bool) {
    let flags = spawnflags as u16;
    let superjump_map = state::with(|g| g.superjump_map);
    if key < 1 {
        let mut k = state::with(|g| g.random_powerup(flags & 0x1ff));
        if k == 0 {
            k = state::with(|g| g.random_powerup(0x1ff));
        }
        (k, true)
    } else {
        let k = key as u16;
        let k = if (superjump_map && k & pw::SUPERJUMP != 0) || k & pw::ARMOR != 0 { state::with(|g| g.random_powerup(0x1ff)) } else { k };
        (k, false)
    }
}

/// `WaitTouch`'s pickup rules (single held slot, +use swaps) for a power-up of type `kind`
/// touched by a player holding `held` (`held_secs` seconds) with `active` running and +use
/// (`using`) or not: `None` leaves it, `Some(drop)` takes it and drops `drop` (0: nothing).
pub fn pickup_plan(r: &PowerupRules, kind: u16, held: u16, held_secs: i32, active: u16, using: bool) -> Option<u16> {
    let can_store = held == 0 || (held == pw::SLOWMO && kind == pw::SLOWMO && held_secs < r.slowmo_top_up_below);
    let instant = kind & r.instant != 0;
    let take = ((can_store || using) && active == 0) || instant;
    if !take {
        return None;
    }
    let mut swap = using && held != 0 && (r.swap_on_instant || !instant);
    if held == pw::SLOWMO && swap {
        swap = held_secs > r.slowmo_swap_above;
    }
    Some(if swap { held } else { 0 })
}

/// `SetupPowerUp`'s duration of a stored type: `(int)(base * pwupmult)`, the x87 product truncated
/// (0x881f4..0x88215).
pub fn duration(base: i32, mult: f32) -> i32 {
    super::c_int(f64::from(base) * f64::from(mult))
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
        let r = crate::gameplay::active().server;
        let flags = cx.pev().spawnflags;
        (self.kind, self.random) = r.powerup_kind(kind, flags);
        if let Some(body) = body(self.kind) {
            cx.pev().body = body;
        }
        let rules = r.powerups();
        let base = rules.base(self.kind);
        self.secs = if secs >= 1 {
            secs
        } else if rules.multiplier {
            duration(base, self.mult)
        } else {
            base
        };
        self.armor = 100.0;
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
        if !rules::is_alive(pcx.edict) {
            return false;
        }
        let using = pcx.pev().button & IN_USE as i32 != 0;
        let r = crate::gameplay::active().server.powerups();
        let Some(drop) = pickup_plan(r, self.kind, p.ts.held, p.ts.held_secs, p.active_powerup, using) else {
            return false;
        };
        if !rules::powerup::add_powerup(p, pcx, self.kind, self.secs, self.armor) {
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
        // `solid` stays as `OnGround` left it (`SOLID_TRIGGER`): only the touch goes.
        v.effects |= EF_NODRAW as i32;
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
        // `CTSPowerUp::KeyValue` (0x97ebc): `strtol`, `strtol`, `strtod`.
        let strtol = || crate::rules::mapcycle::atoi(&value.to_string_lossy());
        match key.to_bytes() {
            b"pwuptype" => self.kind_key = strtol(),
            b"pwupduration" => self.respawn = strtol(),
            b"pwupmult" => self.mult = crate::entities::stock::base::kv_f(value),
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
        if !self.funcs.touch || self.phase != Phase::Waiting || !rules::has_player(other) {
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
    crate::ham::spawn(e);
    crate::world::dispatch(e, |ent, cx| {
        if let Some(pu) = (ent as &mut dyn std::any::Any).downcast_mut::<PowerUp>() {
            pu.setup(cx, kind as i32, secs, 60.0, false);
        }
        let org = cx.pev().origin;
        engine().set_origin(cx.edict, org);
    });
    e
}

/// Sets the armour a power-up gives ([`PowerUp::armor`], e.g. a dying player's armour on the one
/// a ruleset drops). Nothing for anything but a power-up.
pub fn set_armor(e: *mut edict_t, armor: f32) {
    if e.is_null() {
        return;
    }
    crate::world::dispatch(e, |ent, _cx| {
        if let Some(pu) = (ent as &mut dyn std::any::Any).downcast_mut::<PowerUp>() {
            pu.armor = armor;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bodies() {
        assert_eq!(body(pw::SLOWMO), Some(4));
        assert_eq!(body(pw::INFINITE_CLIP), Some(2));
        assert_eq!(body(pw::SLOWPAUSE), Some(9));
        assert_eq!(body(pw::DOUBLE_FIRERATE), Some(1));
        assert_eq!(body(pw::GRENADE), Some(5));
        assert_eq!(body(pw::HEALTH), Some(6));
        assert_eq!(body(pw::ARMOR), Some(7));
        assert_eq!(body(pw::SUPERJUMP), Some(8));
        assert_eq!(body(pw::KUNGFU), Some(0));
        assert_eq!(body(pw::SLOWPAUSE | pw::GRENADE), Some(9), "first matching bit");
        // No known bit: the body is left alone.
        assert_eq!(body(0x200), None);
    }

    /// `WaitTouch` (0x98600..) with TS 3.0's values.
    #[test]
    fn pickup_rules() {
        let r = &crate::rules::powerup::CORE;
        let plan = |kind, held, secs, active, using| pickup_plan(r, kind, held, secs, active, using);
        // An empty slot takes anything; a held one refuses a second stored type.
        assert_eq!(plan(pw::SLOWPAUSE, 0, 0, 0, false), Some(0));
        assert_eq!(plan(pw::SLOWPAUSE, pw::SLOWMO, 2, 0, false), None);
        // Slow motion tops up a held one below 3 s.
        assert_eq!(plan(pw::SLOWMO, pw::SLOWMO, 2, 0, false), Some(0));
        assert_eq!(plan(pw::SLOWMO, pw::SLOWMO, 3, 0, false), None);
        // +use swaps, slow motion only above 2 s; never while one runs.
        assert_eq!(plan(pw::SLOWPAUSE, pw::DOUBLE_FIRERATE, 3, 0, true), Some(pw::DOUBLE_FIRERATE));
        assert_eq!(plan(pw::SLOWPAUSE, pw::SLOWMO, 2, 0, true), Some(0));
        assert_eq!(plan(pw::SLOWPAUSE, pw::SLOWMO, 3, 0, true), Some(pw::SLOWMO));
        assert_eq!(plan(pw::SLOWPAUSE, 0, 0, pw::SLOWMO, true), None);
        // Instant types: always, and +use doesn't drop the held one. Infinite clip is one.
        for k in [pw::INFINITE_CLIP, pw::KUNGFU, pw::GRENADE, pw::HEALTH, pw::ARMOR, pw::SUPERJUMP] {
            assert_eq!(plan(k, pw::SLOWMO, 3, pw::SLOWMO, true), Some(0), "{k:#x}");
        }
    }

    #[test]
    fn durations_truncate() {
        assert_eq!(duration(3, 1.0), 3);
        assert_eq!(duration(3, 1.5), 4);
        assert_eq!(duration(3, 0.9), 2);
        assert_eq!(duration(2, 1.75), 3);
        assert_eq!(duration(3, f32::NAN), i32::MIN);
    }

    #[test]
    fn respawn_clamps() {
        assert_eq!(respawn_delay(0), 60.0);
        assert_eq!(respawn_delay(5), 10.0);
        assert_eq!(respawn_delay(30), 30.0);
        assert_eq!(respawn_delay(999), 180.0);
    }
}
