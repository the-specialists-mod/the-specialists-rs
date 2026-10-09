//! The player entity. Subsystems keep their state in their own structs (stunt, weapons, rules)
//! and are called through fixed hooks in a fixed order, so each work package owns its files.
//! Fields several subsystems read live directly on `Player` (see docs/design.md §4.3).

pub mod damage;
pub mod hud;

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
    pub team: u8,
    /// Kills and deaths for the scoreboard.
    pub frags: i32,
    pub deaths: i32,
    /// Next allowed respawn time while dead.
    pub respawn_at: f32,
    /// Joined the game (past the MOTD/team menu); spectators have this false.
    pub joined: bool,
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

        // Leave any trigger_camera view and un-freeze (TS resets these on spawn).
        pev.flags &= !(FL_FROZEN as i32);
        e.set_view(edict, edict);
        e.set_model(edict, PLAYER_MODEL);
        e.set_size(edict, VEC_HULL_MIN, VEC_HULL_MAX);

        // Order matters: rules picks the spawn point and loadout, weapons equips, stunt resets.
        crate::rules::on_spawn(self, cx);
        crate::weapons::on_spawn(self, cx);
        crate::stunt::on_spawn(self, cx);
        hud::reset(self);
    }

    fn take_damage(&mut self, cx: &mut Cx, dmg: &DamageInfo) -> bool {
        damage::take_damage(self, cx, dmg)
    }
}

impl Player {
    pub fn pre_think(&mut self, cx: &mut Cx) {
        // +use on stock entities (buttons, doors, pushables, chargers); rules handles ground
        // items and power-ups. Read last frame's buttons before rules updates them.
        let (buttons, last) = (cx.pev().button, self.ts.buttons_last);
        if Player::is_alive(cx) {
            crate::entities::stock::player_use(cx.edict, buttons, last);
        }
        crate::rules::pre_think(self, cx);
        crate::stunt::pre_think(self, cx);
        crate::weapons::pre_think(self, cx);
    }

    pub fn post_think(&mut self, cx: &mut Cx) {
        crate::weapons::post_think(self, cx);
        crate::stunt::post_think(self, cx);
        crate::rules::post_think(self, cx);
        hud::update(self, cx);
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
