//! The player entity. Subsystems keep their state in their own structs (stunt, weapons, rules)
//! and are called through fixed hooks in a fixed order, so each work package owns its files.
//! Fields several subsystems read live directly on `Player` (see docs/design.md §4.3).

pub mod damage;
pub mod hud;
pub mod impulse;
pub mod player_use;
pub mod save;
pub mod suit;
pub mod water;

use crate::entities::link_entities;
use crate::world::{Cx, DamageInfo, Entity};
use goldsrc::server::engine;
use goldsrc::sys::*;
use shared::{VEC_HULL_MAX, VEC_HULL_MIN, VEC_VIEW};

link_entities! {
    player => Player,
}

pub const PLAYER_MODEL: &std::ffi::CStr = c"models/player.mdl";

#[derive(Default)]
pub struct Player {
    // ---- subsystem state (owned by the module named) -----------------------------------------
    pub stunt: crate::stunt::StuntState,
    pub inv: crate::weapons::Inventory,
    pub ts: crate::rules::PlayerRules,
    pub hud: hud::HudCache,
    /// Damage types taken, time-based damage and drown recovery (`water.rs`).
    pub tbd: water::TimeBasedDamage,
    /// Train and tank driving (stock entities package).
    pub drive: crate::entities::stock::drive::DriveState,
    /// Spray and developer impulses (`impulse.rs`).
    pub impulse: impulse::ImpulseState,
    /// The HEV suit's sentence queue (`suit.rs`).
    pub suit: suit::SuitQueue,
    pub bot: Option<crate::bots::Bot>,

    // ---- shared fields -------------------------------------------------------------------------
    /// Slow-motion target; `pev->fuser1` eases toward it (rules writes, everyone reads fuser1).
    pub slow_target: f32,
    /// Active power-up bits (`shared::ts::powerup`) and when they expire.
    pub active_powerup: u16,
    pub powerup_until: f32,
    /// Permanent power-ups from pickups this life.
    pub kungfu: bool,
    pub superjump: bool,
    /// Inventory slots in use (weapons writes; speed = shared::ts::speed_for_free_slots).
    pub slots_used: i32,
    pub cash: i32,
    /// Kills and deaths for the scoreboard.
    pub frags: i32,
    pub deaths: i32,
    /// Next allowed respawn time while dead.
    pub respawn_at: f32,
    /// Joined the game (past the MOTD/team menu); spectators have this false.
    pub joined: bool,
    /// `m_pfnThink` is `PlayerDeathThink`: `Killed` (0x8ed5c) sets it (with `nextthink` 0.1 s
    /// on) for a death that doesn't gib, and nothing clears it; [`Entity::think`] runs it.
    pub death_think: bool,
    /// `m_rgItems[ITEM_SECURITY]` (+0x838): `item_security` pickups. Nothing reads it but the save.
    pub security_cards: i32,
}

impl Entity for Player {
    fn spawn(&mut self, cx: &mut Cx) {
        let e = engine();
        let edict = cx.edict;
        let pev = cx.pev();
        pev.classname = e.alloc_string(c"player");
        pev.health = 100.0;
        pev.max_health = 100.0;
        pev.armorvalue = 0.0;
        pev.takedamage = DAMAGE_AIM as f32;
        pev.solid = SOLID_SLIDEBOX as i32;
        pev.movetype = MOVETYPE_WALK as i32;
        pev.flags = FL_CLIENT as i32 | (pev.flags & FL_FAKECLIENT as i32);
        pev.deadflag = DEAD_NO as i32;
        pev.friction = 1.0;
        pev.gravity = 1.0;
        pev.fov = 90.0;
        pev.view_ofs = VEC_VIEW;
        pev.effects = 0;
        pev.iuser4 = 0;
        pev.fuser1 = 1.0; // time scale
        pev.v_angle = [0.0; 3];
        pev.fixangle = 1;
        self.slow_target = 1.0;
        self.active_powerup = 0;
        self.kungfu = false;
        self.superjump = false;
        // TS Spawn clears m_bitsDamageType only; the drown counters carry over.
        self.tbd.bits = 0;

        // Leave any trigger_camera view and un-freeze (TS resets these on spawn).
        pev.flags &= !(FL_FROZEN as i32);
        e.set_view(edict, edict);
        e.set_model(edict, PLAYER_MODEL);
        e.set_size(edict, VEC_HULL_MIN, VEC_HULL_MAX);

        // Order matters: rules picks the spawn point and loadout, weapons equips, stunt resets.
        crate::rules::on_spawn(self, cx);
        crate::weapons::on_spawn(self, cx);
        crate::stunt::on_spawn(self, cx);
        crate::entities::stock::drive::on_spawn(self, cx);
        impulse::on_spawn(self);
        hud::reset(self);
    }

    /// `CBaseEntity::Think` (0xe2c50; `CBasePlayer` doesn't override it): `m_pfnThink`, if set.
    /// The engine calls it from the player's usercmd, after PreThink, once `nextthink` is due
    /// (`SV_PlayerRunThink`), so the `PlayerDeathThink` that `Killed` schedules runs once 0.1 s
    /// after the death, on top of the one every PreThink of a dead player runs.
    fn think(&mut self, cx: &mut Cx) {
        if self.death_think {
            crate::rules::lifecycle::death_think(self, cx);
        }
    }

    fn trace_attack(&mut self, cx: &mut Cx, dmg: &DamageInfo) {
        damage::trace_attack(self, cx, dmg);
    }

    fn take_damage(&mut self, cx: &mut Cx, dmg: &DamageInfo) -> bool {
        damage::take_damage(self, cx, dmg)
    }

    /// `CBasePlayer::Save` / `Restore` (0xda9d8 / 0xdaa2c): `save.rs`.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut crate::save::Fields) {
        self.walk_save(cx, f);
    }

    /// `CBasePlayer::Precache` (0xda90c).
    fn restored(&mut self, _cx: &mut Cx) {
        self.restore_precache();
    }
}

impl Player {
    /// `CBasePlayer::PreThink` (0x908cc). During intermission TS stops after the button edges
    /// and `PlayerThink`, before `ItemPreFrame`, `WaterMove`, `CheckTimeBasedDamage`, the
    /// stunt/weapon timers and the train controls, so only `rules::player_think` runs then.
    pub fn pre_think(&mut self, cx: &mut Cx) {
        // Read last frame's buttons before rules updates them. +use on objects (`player_use`)
        // runs from the weapons' PostThink, inside `ItemPostFrame` as in TS.
        let (buttons, last) = (cx.pev().button, self.ts.buttons_last);
        // TS returns after the button edges and `PlayerThink` when the game is over.
        if crate::rules::player_think(self, cx) {
            return;
        }
        crate::entities::stock::drive::pre_think(self, cx, buttons & !last);
        crate::rules::pre_think(self, cx);
        crate::stunt::pre_think(self, cx);
        crate::weapons::pre_think(self, cx);
    }

    /// `CBasePlayer::PostThink` (0x91610). When the game is over (intermission) or the player is
    /// dead, TS jumps straight to the end, which only runs the weapon timers down: no tank
    /// control, `ItemPostFrame` (firing, `PlayerUse`), landing, animation or `m_afButtonLast`.
    pub fn post_think(&mut self, cx: &mut Cx) {
        let active = !crate::rules::state::with(|g| g.game_over) && Self::is_alive(cx);
        if active {
            crate::entities::stock::drive::post_think(self, cx);
        }
        crate::weapons::post_think(self, cx, active);
        if active {
            crate::stunt::post_think(self, cx);
            impulse::update_player_sound(self, cx);
        }
        crate::rules::post_think(self, cx);
    }

    pub fn is_alive(cx: &mut Cx) -> bool {
        let v = cx.pev();
        v.deadflag == DEAD_NO as i32 && v.health > 0.0
    }

    /// Fills `clientdata_t` for prediction (called from net::update_client_data).
    pub fn fill_client_data(&mut self, cx: &mut Cx, cd: &mut clientdata_t) {
        crate::stunt::fill_client_data(self, cx, cd);
        crate::weapons::fill_client_data(self, cx, cd);
        crate::rules::fill_client_data(self, cx, cd);
    }
}

/// Runs `f` on the `Player` attached to `e`, if any (take-out dispatch).
pub fn with_player<R>(e: *mut edict_t, f: impl FnOnce(&mut Player, &mut Cx) -> R) -> Option<R> {
    crate::world::dispatch(e, |ent, cx| (ent as &mut dyn std::any::Any).downcast_mut::<Player>().map(|p| f(p, cx))).flatten()
}

/// [`with_player`] that also reaches a player lent to its slot by a call further up the stack
/// (`world::run_now`): its own +use, spawn or shot set off what calls this, and TS reaches the
/// player at once. `None` when the player is held by a dispatch that didn't lend it.
pub fn with_player_now<R>(e: *mut edict_t, f: impl FnOnce(&mut Player, &mut Cx) -> R) -> Option<R> {
    crate::world::run_now(e, || with_player(e, f)).flatten()
}

/// `ClientPutInServer` (0xacf44): a `Player` for the edict (the engine may hand back one from the
/// last map, which keeps its object), `SetCustomDecalFrames(-1)`, then `Spawn`.
pub fn client_put_in_server(e: *mut edict_t) {
    crate::world::create::<Player>(goldsrc::edict::EdictPtr::v(e));
    crate::world::dispatch(e, |ent, cx| {
        if let Some(p) = (&mut *ent as &mut dyn std::any::Any).downcast_mut::<Player>() {
            p.impulse.custom_decal_frames = impulse::custom_decal_frames(-1);
        }
        ent.spawn(cx);
    });
}
