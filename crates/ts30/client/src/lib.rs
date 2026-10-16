//! TS 3.0's client rules (issue #200): the HUD and input hooks, and the plugin object
//! `client-dll` registers. Addresses are TS 3.0 `client.dll` VAs.

#![deny(unsafe_code)]

use goldsrc::ext::render::{ATTN_NORM, Vec3, random_float, random_long};
use goldsrc::sys::ref_params_t;
use ts_client::effects::fx::FxStyle;
use ts_client::effects::psys::PSys;
use ts_client::events::{BulletDraw, BulletStep, ClientBullet};
use ts_client::gameplay::{ClientPlugin, ClientRules, CrosshairParts, KungfuRegen, MeleeReach, SpectatorTail, StuntCooldown, ThinkParts};
use ts_client::hud::draw::{HudOp, StatusLook, WeaponPanel};
use ts_client::menus::buy::MenuRules;
use ts_client::spectator::HltvHost;
use ts_client::view::{Bob, BobInput, EyeZ};
use ts30_gameplay::TS30;

pub mod events;
pub mod fx;
pub mod spectator;
pub mod view;

/// TS 3.0's client-only rules.
pub struct Ts30Client;

impl ClientRules for Ts30Client {
    fn ruleset_name(&self) -> &'static str {
        "TS 3.0"
    }

    /// `DrawCrosshair` (1004e9f0): the divesteps first (whatever the weapon), then the crosshair
    /// unless the weapon is kung fu, the M61 grenade, a knife or the katana (0, 24, 25, 34, 35).
    fn crosshair_parts(&self, weapon: i32, _fov: f32) -> CrosshairParts {
        CrosshairParts { crosshair: !matches!(weapon, 0 | 24 | 25 | 34 | 35), divesteps: true, divesteps_last: false }
    }

    /// `0x1004eb7f`: `base + ((acc + spread) − acc) × 160`, which TS works out on the x87 stack,
    /// where it is `base + spread × 160` (the accuracy cancels out). Nothing is rounded to float
    /// before `__ftol`, so this is `trunc(base + spread × 160)` exactly; f64 holds a 24-bit spread
    /// times 160 without rounding (issue #110: f32 rounded a spread one ulp under `k / 160` up a
    /// pixel). Guarded: a huge predicted spread would overflow `cx + gap`; at that size the lines
    /// are far off screen anyway.
    fn crosshair_gap(&self, base: i32, spread: f32, _accuracy: f32) -> i32 {
        ((base as f64 + spread as f64 * 160.0) as i32).clamp(-0x10000, 0x10000)
    }

    /// Thresholds 1004eec0/1004eef0/1004ef20: red when the slots in use (`81 − free`) reach 42, 52
    /// and 62, whatever the armour or game mode.
    fn divesteps_blocked(&self, used_slots: i32, _armor: u8, _teamplay: i32) -> [bool; 3] {
        [42, 52, 62].map(|limit| used_slots >= limit)
    }

    /// 1004ef50: the countdown is at least 0.01 s.
    fn stunt_cooldown(&self, cooldown: StuntCooldown, _now: f32) -> bool {
        cooldown.remaining >= 0.01
    }

    /// None: TS 3.0 clamps the cvar to 70..110 instead, 0 meaning 80.
    fn pinned_default_fov(&self) -> Option<f32> {
        None
    }

    /// Only while `m_iFOV` is unset (below 0.001).
    fn fov_snaps(&self, cur: i32) -> bool {
        (cur as f32) < 0.001
    }

    fn input_gate(&self) -> bool {
        true
    }

    fn attack_switch(&self) -> bool {
        true
    }

    fn attack_sends_respawn(&self) -> bool {
        true
    }

    fn map_effects(&self) -> bool {
        true
    }

    /// `DrawKungFuBar`: power at most 0.01.
    fn kungfu_exhausted(&self, power: f32) -> bool {
        power <= 0.01
    }

    /// 1004ce10: below 100 it gains 15 per step while non-negative, 1 per step while negative, up
    /// to 100.
    fn kungfu_regen(&self, r: KungfuRegen) -> f32 {
        let mut power = r.power;
        if power < 100.0 {
            let step = if power >= -0.001 { r.step * 15.0 } else { r.step };
            power = (power + step).min(100.0);
        }
        power
    }

    /// All three: the `lasclr_%i` send (`0x1004ce2e..`, on a change of `+0x3728`), the heartbeat
    /// and breathing sounds (`0x1004cf8b..`, state `+0x3708..+0x3714`) and the fire-mode
    /// `weapons/switch.wav` (`0x1004d25d..0x1004d3cd`).
    fn think_parts(&self) -> ThinkParts {
        ThinkParts { laser_color: true, firemode_click: true, sounds: true }
    }

    /// None: TS 3.0's `Reset` leaves the bar alone (only `VidInit` refills it).
    fn kungfu_power_on_reset(&self) -> Option<f32> {
        None
    }

    /// TS 3.0 hard-wires it off, whatever the game mode (`m_Teamplay`) or `SrvSett` 1
    /// (`cashbased`) say.
    fn buy_cash_based(&self, _teamplay: i32, _cashbased: i32) -> bool {
        false
    }

    /// [`MenuRules`]'s default, TS 3.0's.
    fn buy_menu(&self) -> MenuRules {
        MenuRules::default()
    }

    /// TS 3.0 has no such icon (it tints the screen instead).
    fn slowmo_icon(&self) -> bool {
        false
    }

    fn slowmo_tint(&self) -> bool {
        true
    }

    /// `100 - (int)(ts * -300)` (`0x1003eeed`), 400 at normal speed.
    fn fire_pitch(&self, time_scale: f32, _slowmo_factor100: f32) -> i32 {
        100 - (time_scale * -300.0) as i32
    }

    /// [`view::bob`] (`0x10066e70`).
    fn view_bob(&self, bob: &mut Bob, input: &BobInput) -> f32 {
        view::bob(bob, input)
    }

    /// [`view::eye`]: the view height eased at `min(15·ft, 1)` on z only, then `V_SmoothStairs`
    /// (`0x100677d0`) on the eye z (`0x100679ee..0x10067a46`).
    fn eye_origin(&self, eye: &mut EyeZ, simorg: Vec3, viewheight: Vec3, on_ground: bool, ft: f32) -> Vec3 {
        view::eye(eye, simorg, viewheight, on_ground, ft)
    }

    /// [`view::body_cam_z`]: `V_SmoothStairs` again (`0x10068313`).
    fn body_cam_z(&self, eye: &mut EyeZ, z: f32, on_ground: bool, ft: f32) -> f32 {
        view::body_cam_z(eye, z, on_ground, ft)
    }

    /// `0x10045a8b`: with `realbullet` (`SrvSett` 0) on or the eased time scale below 0.99.
    fn client_bullets_queue(&self, realbullet: i32, time_scale: f32, _slowmo_factor100: f32) -> bool {
        realbullet != 0 || time_scale < 0.99
    }

    /// [`events::next_update`] (`0x10035b97`).
    fn client_bullet_next_update(&self, now: f32, realbullet: i32) -> f32 {
        events::next_update(now, realbullet)
    }

    /// `ClientBullets_Impact` (0x10035d40): `dir·(velocity·40)·dt·GetBulletVelocityMult`.
    fn client_bullet_step(&self, dir: [f32; 3], s: &BulletStep) -> [f32; 3] {
        let mult = ts_client::events::bullet_velocity_mult(s.time_scale, s.bullet_time);
        let v40 = s.velocity * 40.0;
        dir.map(|d| d * v40 * s.dt * mult)
    }

    /// Exit bullets on a fixed 30-unit scale (`0x10035f44`), the others by the ammo's range unit
    /// (`0x10035f94`).
    fn client_bullet_range_index(&self, dist: f64, unit: f32, exit: bool) -> i32 {
        if exit {
            ts_client::events::ftol(dist * 0.003_333_333_4f32 as f64 * 10.0)
        } else {
            ts_client::events::range_index(dist as f32, unit)
        }
    }

    /// While the eased time scale is below 0.99.
    fn client_bullet_draws(&self, time_scale: f32, _slowmo_factor100: f32) -> bool {
        time_scale < 0.99
    }

    /// False: the core draw runs.
    fn client_bullet_draw(&self, _bullet: &mut ClientBullet, _d: &mut BulletDraw) -> bool {
        false
    }

    /// [`fx::FX`].
    fn fx_style(&self) -> &'static FxStyle {
        &fx::FX
    }

    /// False: [`ts_client::effects::fx::impact`]'s body (`0x10048f60`) runs.
    fn impact_particles(&self, _ps: &mut PSys, _now: f64, _pos: Vec3, _n: Vec3, _tex: u8, _bullet_type: i32) -> bool {
        false
    }

    /// `RandomLong(5, 10)` for the sawed-off (33), else `RandomLong(45, 55)`.
    fn gun_smoke_rate(&self, id: i32) -> i32 {
        if id == 33 { random_long(5, 10) } else { random_long(45, 55) }
    }

    fn pellet_spray_gravity(&self) -> f32 {
        0.0
    }

    /// `RandomLong(-40, 40)` for the 17, else `RandomLong(120, 140)`.
    fn shell_side_speed(&self, id: i32) -> i32 {
        if id == 17 { random_long(-40, 40) } else { random_long(120, 140) }
    }

    /// 2 for weapon 11, else 1.
    fn shell_sound(&self, id: i32) -> i32 {
        if id == 11 { 2 } else { 1 }
    }

    /// 20-40 and 0-20 (`0x10040e43`, `0x10040e4d`).
    fn pump_shell_speeds(&self) -> [(i32, i32); 2] {
        [(20, 40), (0, 20)]
    }

    /// [`AMMO_ATTENUATION`](ts_client::events::AMMO_ATTENUATION), 0.4 past it.
    fn ammo_attenuation(&self, ammo: usize) -> f32 {
        ts_client::events::AMMO_ATTENUATION.get(ammo).copied().unwrap_or(0.4)
    }

    fn bullet_whizz_and_push(&self) -> bool {
        true
    }

    /// 0.97 ([`events::MATERIAL_ATTN`]) and [`slow_pitch`](ts_client::events::slow_pitch).
    fn bullet_material_sound(&self) -> (f32, i32) {
        (events::MATERIAL_ATTN, ts_client::events::slow_pitch())
    }

    /// [`fire_pitch`](ts_client::events::fire_pitch) of the eased scale (`0x10044dbd`).
    fn ricochet_pitch(&self, _time_scale: f32, _slowmo_factor100: f32) -> i32 {
        ts_client::events::fire_pitch()
    }

    /// `RandomFloat(0.92, 1)`, `ATTN_NORM`, [`slow_pitch`](ts_client::events::slow_pitch), the
    /// pitch's `RandomLong(-5, 5)` rolled before the volume (`EV_TS_Empty`
    /// `0x10042507..0x1004256f`, `EV_TS_Knife` `0x10043feb..0x1004402e`).
    fn event_sound(&self) -> (f32, f32, i32) {
        let pitch = ts_client::events::slow_pitch();
        let vol = random_float(0.92, 1.0);
        (vol, ATTN_NORM, pitch)
    }

    /// Attenuation 0.8, `slow_pitch` (rolled first, `0x10046066`), volume
    /// [`events::melee_material_volume`].
    fn melee_material_sound(&self, tex: u8) -> Option<(f32, f32, i32)> {
        events::melee_material_volume(tex).map(|vol| (vol, ATTN_NORM, ts_client::events::slow_pitch()))
    }

    /// Close combat `-55`/`45`/`35` by attack (`0x10043427..`), coldcock 30, knife 35, katana
    /// `-55` for the back swing (15), else 50.
    fn melee_reach(&self, m: MeleeReach) -> f32 {
        match m {
            MeleeReach::CloseCombat(3) => -55.0,
            MeleeReach::CloseCombat(2) => 45.0,
            MeleeReach::CloseCombat(_) => 35.0,
            MeleeReach::Coldcock(_) => 30.0,
            MeleeReach::Knife => 35.0,
            MeleeReach::Katana(15) => -55.0,
            MeleeReach::Katana(_) => 50.0,
        }
    }

    fn close_combat_blocks(&self) -> bool {
        true
    }

    /// False: `TS_Scintille` and `TS_Sparks(7, 1.15, 25)` run.
    fn blade_hit_particles(&self, _ps: &mut PSys, _now: f64, _pos: Vec3, _n: Vec3, _katana: bool) -> bool {
        false
    }

    fn reload_event_phases(&self) -> bool {
        true
    }

    /// None: TS 3.0's `MsgFunc_WeaponInfo` leaves the bar to `KFuPower`.
    fn kungfu_power_from_weapon_info(&self, _clip: u8) -> Option<f32> {
        None
    }

    /// False: the `respawn` key tap alone.
    fn hltv_buttons(&self, _host: &mut dyn HltvHost, _buttons: i32, _down: bool) -> bool {
        false
    }

    /// False: the core's `SetModes` (`0x100234d0`).
    fn hltv_set_modes(&self, _host: &mut dyn HltvHost, _mode: i32, _inset: i32) -> bool {
        false
    }

    /// Nothing beyond the core's own state.
    fn hltv_reset(&self, _init: bool) {}

    /// Nothing, false.
    fn hltv_draw(&self, _host: &mut dyn HltvHost) -> bool {
        false
    }

    /// False: the core's passes (`0x10068f70`).
    fn hltv_calc_refdef(&self, _host: &mut dyn HltvHost, _p: &mut ref_params_t) -> bool {
        false
    }

    /// `0x10069bb2..0x10069bbd`: either, so a player who observes gets the spectator view.
    fn spectator_refdef(&self, spectator: bool, observer_mode: i32) -> bool {
        spectator || observer_mode != 0
    }

    /// False: the core's refresh (`0x10024160`).
    fn hltv_overview_camera(&self, _host: &dyn HltvHost, _origin: &mut Vec3, _angles: &mut Vec3) -> bool {
        false
    }

    /// `0x10024970`: always.
    fn hltv_overview_in_main_view(&self) -> bool {
        true
    }

    /// None: the observer mode (`0x10063870`).
    fn hltv_panel_mode(&self) -> Option<i32> {
        None
    }

    /// [`spectator::nohud_shows`]: an exact 0 or a NaN (issue #185).
    fn spectator_panel_shows(&self, nohud: f32) -> bool {
        spectator::nohud_shows(nohud)
    }

    /// None: TS 3.0's `DrawWeaponInfo` (1004d7e0).
    fn weapon_panel(&self, _panel: &WeaponPanel) -> Option<Vec<HudOp>> {
        None
    }

    /// False: TS 3.0's `MsgFunc_WeaponInfo` doesn't touch the selection.
    fn kungfu_weapon_info_resets_menu_selection(&self) -> bool {
        false
    }

    /// None: TS 3.0's `DrawKungFuLabel` (1004d700).
    fn kungfu_label(&self, _power: f32, _hud: [i32; 3], _w: i32, _h: i32) -> Option<Vec<HudOp>> {
        None
    }

    /// [`StatusLook`]'s default, TS 3.0's `DrawStatusPanel` (1004a3e0).
    fn status_look(&self) -> StatusLook {
        StatusLook::default()
    }

    // Audit round 2: A1

    /// `HUD_AddEntity` (`0x10044a5f..0x10044ac8`) fades an item past 492 / 206 / 824 units while
    /// `m_iFOV > 79`.
    fn ground_items_fade(&self) -> bool {
        true
    }

    /// [`spectator::add_entity_tail`] of `g_iUser1`, `g_iUser2` and the inset.
    fn add_entity_spectator(&self) -> SpectatorTail {
        let (mode, target) = ts_client::state::with(|s| (s.spec_mode, s.spec_target));
        spectator::add_entity_tail(mode, target, ts_client::spectator::in_eye(), ts_client::spectator::pip())
    }

    /// [`fx::LASER`].
    fn laser_style(&self) -> &'static ts_client::effects::LaserStyle {
        &fx::LASER
    }
}

/// The plugin as the client registers it.
pub static PLUGIN: ClientPlugin = ClientPlugin { pm: &TS30, gun: &TS30, client: &Ts30Client };

#[cfg(test)]
mod tests {
    use super::*;

    /// Issue #110: `0x1004eb7f` truncates `base + spread × 160` off the x87 stack, so a spread one
    /// ulp under `k / 160` stays below the next pixel.
    #[test]
    fn crosshair_gap_is_not_rounded_to_float() {
        let under = |k: f32| f32::from_bits((k / 160.0).to_bits() - 1);
        assert_eq!(under(1.0), 0.0062499996);
        for (spread, want) in [(0.0062499996f32, 22), (0.049999997, 29), (0.099999994, 37), (0.19999999, 53)] {
            assert_eq!(Ts30Client.crosshair_gap(22, spread, 0.0075), want, "spread {spread}");
        }
        for k in 1..=39 {
            assert_eq!(Ts30Client.crosshair_gap(22, under(k as f32), 0.01), 22 + k - 1, "k {k}");
        }
        assert_eq!(Ts30Client.crosshair_gap(22, 0.2, 0.0075), 54);
        for s in [f32::NAN, f32::INFINITY, f32::MAX, -f32::MAX] {
            assert!(Ts30Client.crosshair_gap(22, s, 0.01).abs() <= 0x10000);
        }
    }

    /// The divestep thresholds (1004eec0/1004eef0/1004ef20): `81 − free` slots reach 42, 52, 62.
    #[test]
    fn divesteps_blocked_thresholds() {
        let blocked = |free: i32| Ts30Client.divesteps_blocked(81 - free, 0, 0);
        assert_eq!(blocked(70), [false; 3]);
        assert_eq!(blocked(25), [true, true, false], "56 used: top and centre too heavy");
        assert_eq!(blocked(19), [true; 3]);
        assert_eq!([blocked(39), blocked(40)], [[true, false, false], [false; 3]]);
    }

    /// `UTIL_ApproachFOV` (10049620) snaps only while `m_iFOV` is unset.
    #[test]
    fn fov_snaps_when_unset() {
        assert!(Ts30Client.fov_snaps(0) && Ts30Client.fov_snaps(-1) && Ts30Client.fov_snaps(i32::MIN));
        assert!(!Ts30Client.fov_snaps(1) && !Ts30Client.fov_snaps(90));
    }

    /// `EV_TS_FirePump`'s shell rolls (`0x10040e43`, `0x10040e4d`): 20-40 and 0-20.
    #[test]
    fn pump_shell_speeds() {
        assert_eq!(Ts30Client.pump_shell_speeds(), [(20, 40), (0, 20)]);
    }

    /// TS 3.0's reaches (`0x10043427..0x100434ab`: -55.0 at `0x1007d6fc`, 45 at `0x1007a900`,
    /// 35 at `0x1007a8d8`; coldcock 30 at `0x10043b69`).
    #[test]
    fn melee_reach_by_event() {
        let core = &Ts30Client;
        assert_eq!(core.melee_reach(MeleeReach::CloseCombat(1)), 35.0); // punch
        assert_eq!(core.melee_reach(MeleeReach::CloseCombat(2)), 45.0); // kick
        assert_eq!(core.melee_reach(MeleeReach::CloseCombat(3)), -55.0); // back kick: behind the player
        assert_eq!(core.melee_reach(MeleeReach::Coldcock(3)), 30.0);
        assert_eq!(core.melee_reach(MeleeReach::Knife), 35.0);
        assert_eq!(core.melee_reach(MeleeReach::Katana(15)), -55.0);
        assert_eq!(core.melee_reach(MeleeReach::Katana(16)), 50.0);
        assert!(core.close_combat_blocks() && core.reload_event_phases() && core.bullet_whizz_and_push());
    }
}
