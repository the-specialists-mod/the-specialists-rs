//! The player's save data: `CBasePlayer::Save` / `Restore` (0xda9d8 / 0xdaa2c) and `Precache`
//! (0xda90c), which `DispatchRestore` runs after it.
//!
//! The chain is `CBaseToggle`'s (TS's `CBaseMonster` has no table) and then `"PLAYER"`
//! (`m_playerSaveData`, 36 fields). The player's state is spread over the port's subsystems; each
//! field below names where it lives, or why TS's is always the same value.

use super::Player;
use crate::save::{BaseFuncs, Fields, tables};
use crate::weapons::Active;
use crate::world::Cx;
use goldsrc::edict::EdictPtr;
use goldsrc::server::engine;
use goldsrc::sys::*;

/// `m_afPhysicsFlags` bits the port keeps (`PFLAG_ONTRAIN`, `PFLAG_DUCKING`, `PFLAG_USING`).
/// TS never sets the others: there are no ladders that set `PFLAG_ONLADDER` in TS's player code,
/// no barnacles, and its `StartObserver` doesn't set `PFLAG_OBSERVER`.
const PFLAG_ONTRAIN: i32 = 2;
const PFLAG_DUCKING: i32 = 8;
const PFLAG_USING: i32 = 16;

/// `m_rgItems` indices.
const ITEM_ANTIDOTE: usize = 2;
const ITEM_SECURITY: usize = 3;

impl Player {
    /// Walks TS's chain for the player (see the module docs).
    pub fn walk_save(&mut self, cx: &mut Cx, f: &mut Fields) {
        let cleared = f.base_funcs(cx, BaseFuncs { think: self.death_think, ..Default::default() });
        if cleared.think {
            self.death_think = false;
        }
        // CBaseDelay: a player has no delay or killtarget.
        f.table(&tables::CBaseDelay);
        f.zero();
        f.zero();
        f.table(&tables::CBaseAnimating);
        let a = &mut self.stunt.anim;
        f.float(&mut a.frame_rate);
        f.float(&mut a.ground_speed);
        f.float(&mut a.last_event_check);
        f.boolean(&mut a.seq_finished);
        f.boolean(&mut a.seq_loops);
        // CBaseToggle: none of TS's player code touches +0x80..+0x104, so all 19 stay zero.
        f.table(&tables::CBaseToggle);
        for _ in 0..tables::CBaseToggle.fields.len() {
            f.zero();
        }

        f.table(&tables::CBasePlayer);
        let st = &mut self.stunt;
        f.int(&mut st.button_last);
        f.int(&mut st.button_pressed);
        f.int(&mut st.button_released);
        let mut items = [0; 5];
        items[ITEM_ANTIDOTE] = self.tbd.antidotes;
        items[ITEM_SECURITY] = self.security_cards;
        f.ints(&mut items);
        self.tbd.antidotes = items[ITEM_ANTIDOTE];
        self.security_cards = items[ITEM_SECURITY];
        let d = &mut self.drive;
        let mut pflags = (d.on_train as i32 * PFLAG_ONTRAIN) | (st.pflag_ducking as i32 * PFLAG_DUCKING) | (d.using as i32 * PFLAG_USING);
        f.int(&mut pflags);
        d.on_train = pflags & PFLAG_ONTRAIN != 0;
        st.pflag_ducking = pflags & PFLAG_DUCKING != 0;
        d.using = pflags & PFLAG_USING != 0;
        // m_flTimeStepSound (Spawn zeroes it, nothing else writes it), m_flTimeWeaponIdle,
        // m_flSwimTime, m_flDuckTime, m_flWallJumpTime (never written).
        for _ in 0..5 {
            f.zero();
        }
        // The HEV suit queue (`suit.rs`; filled only under `CHalfLifeRules`).
        let q = &mut self.suit;
        f.float(&mut q.update);
        f.ints(&mut q.play_list);
        f.int(&mut q.play_next);
        f.ints(&mut q.no_repeat);
        f.floats(&mut q.no_repeat_time);
        f.int(&mut self.stunt.anim.last_damage);
        // m_rgpPlayerItems: slot 1 the cycler (`cycler_weapon`), slot 2 the gun (`weapon_tsgun`).
        let sv = &mut self.inv.sv;
        let mut slots = [std::ptr::null_mut(); 6];
        slots[1] = sv.cycler.get();
        slots[2] = if sv.has_gun { sv.gun_item.get() } else { std::ptr::null_mut() };
        f.ents(&mut slots);
        let item_of = |a: Active, slots: &[*mut edict_t; 6]| match a {
            Active::None => std::ptr::null_mut(),
            Active::Cycler => slots[1],
            Active::Gun => slots[2],
        };
        let active = if sv.cycler_active {
            Active::Cycler
        } else if sv.has_gun {
            Active::Gun
        } else {
            Active::None
        };
        let mut active_e = item_of(active, &slots);
        f.ent(&mut active_e);
        let mut last_e = item_of(sv.last_item, &slots);
        f.ent(&mut last_e);
        if f.restoring() {
            sv.cycler = goldsrc::ext::entities::EHandle::new(slots[1]);
            sv.gun_item = goldsrc::ext::entities::EHandle::new(slots[2]);
            sv.has_gun = !slots[2].is_null();
            let which = |e: *mut edict_t| {
                if e.is_null() {
                    Active::None
                } else if e == slots[1] {
                    Active::Cycler
                } else if e == slots[2] {
                    Active::Gun
                } else {
                    Active::None
                }
            };
            sv.cycler_active = which(active_e) == Active::Cycler;
            sv.last_item = which(last_e);
        }
        // m_rgAmmo: only `GiveAmmo` fills it, and no TS item gives HL ammo.
        f.zero();
        f.int(&mut self.tbd.drowned);
        f.int(&mut self.tbd.restored);
        // m_tSneaking: only `StartSneaking` / `StopSneaking` write it, and nothing calls them.
        f.zero();
        f.int(&mut self.drive.train);
        // m_bitsHUDDamage: `Spawn`, `Precache` and `TakeDamage` all set -1.
        let _ = f.int_val(-1);
        f.float(&mut self.stunt.fall_velocity);
        if f.restoring() {
            self.ts.buttons_last = self.stunt.button_last;
            self.ts.buttons_pressed = self.stunt.button_pressed;
        }
        f.int(&mut self.impulse.target_volume);
        f.int(&mut self.impulse.weapon_volume);
        // m_iExtraSoundTypes (only zeroed) and m_iWeaponFlash (only decays from 0).
        f.zero();
        f.zero();
        // m_fLongJump: `Spawn` zeroes it and 3.0 never sets it; `Restore` turns it into "slj".
        f.boolean(&mut self.long_jump);
        let long_jump = self.long_jump;
        f.boolean(&mut self.ts.init_hud);
        f.float(&mut self.tbd.prev);
        f.ehandle(&mut self.drive.tank);
        f.int(&mut self.drive.hide_hud);
        // m_iFOV: only the `fov` cheat sets it.
        f.int(&mut self.ts.ifov);
        if f.live() {
            restore_tail(self, cx, long_jump);
        }
    }

    /// `CBasePlayer::Precache` (0xda90c) as `DispatchRestore` runs it. The node-graph check
    /// has nothing to check (no graph); the geiger range and `m_iUpdateTime` aren't kept by the
    /// port across a restore (a fresh object holds TS's values).
    pub fn restore_precache(&mut self) {
        self.tbd.bits = 0;
        self.hud.armor = -1;
        self.drive.train = crate::entities::stock::drive::TRAIN_NEW;
        if crate::save::init_hud() {
            self.ts.init_hud = true;
        }
    }
}

/// The rest of `CBasePlayer::Restore` (0xdaa2c), after the fields: without a landmark (a saved
/// game, not a transition) the player goes to a spawn spot; the view angles become the body
/// angles; the hull follows `FL_DUCKING`; the physics keys; `m_flNextAttack =
/// UTIL_WeaponTimeBase()` (0).
fn restore_tail(p: &mut Player, cx: &mut Cx, long_jump: bool) {
    let eng = engine();
    if let Some(sd) = eng.save_data()
        && !sd.use_landmark()
    {
        eng.alert(ALERT_TYPE::at_console, &format!("No Landmark:{}\n", String::from_utf8_lossy(&sd.landmark_name())));
        // Guard: TS reads the spot through NULL when the map has none.
        let spot = crate::rules::spawnpoint::select(cx.edict);
        if !spot.is_null() {
            let (o, a) = (spot.v().origin, spot.v().angles);
            let v = cx.pev();
            v.origin = [o[0] + 0.0, o[1] + 0.0, o[2] + 1.0];
            v.angles = a;
        }
    }
    let v = cx.pev();
    v.v_angle[2] = 0.0;
    v.angles = v.v_angle;
    v.fixangle = 1;
    let (mins, maxs) = if v.flags & FL_DUCKING as i32 != 0 {
        ([-16.0, -16.0, -18.0], [16.0, 16.0, 18.0])
    } else {
        ([-16.0, -16.0, -36.0], [16.0, 16.0, 36.0])
    };
    eng.set_size(cx.edict, mins, maxs);
    eng.set_physics_key(cx.edict, c"hl", c"1");
    eng.set_physics_key(cx.edict, c"slj", if long_jump { c"1" } else { c"0" });
    crate::gameplay::set_physinfo(cx.edict);
    p.inv.gun.next_attack = 0.0;
}
