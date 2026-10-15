//! [`ClientRules`]: the client-only hooks (HUD and input).
//!
//! The HUD resolves them from the server's `SrvSett` key [`SRVSETT_KEY`](ts_gameplay::SRVSETT_KEY)
//! ([`hud_rules`](super::hud_rules)), which `InitHUD` sends on each map. Code outside the
//! HUD's own borrow (input commands, the FOV and effects frames) uses [`current`](super::current).

use crate::effects::psys::PSys;
use goldsrc::ext::render::{Vec3, random_float, random_long};

/// Which melee event's trace [`ClientRules::melee_reach`] sizes, with the event's `iparam1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeleeReach {
    /// `EV_TS_CloseCombat`: 1 punch, 2 kick, 3 back kick.
    CloseCombat(i32),
    /// `EV_TS_ColdCock`: the weapon id.
    Coldcock(i32),
    Knife,
    /// `EV_TS_Katana`: the swing's sequence index (15 the back swing).
    Katana(i32),
}

/// Which parts `DrawCrosshair` draws for the held weapon (before the scoped test, which the
/// caller applies: the divesteps never show while scoped).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CrosshairParts {
    /// The crosshair lines or sprite and the laser range number.
    pub crosshair: bool,
    /// The divestep indicator.
    pub divesteps: bool,
    /// The divesteps are drawn after the crosshair (on top of it) rather than before.
    pub divesteps_last: bool,
}

/// What the stunt cooldown (`NextStunt`) state is when the divesteps are drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StuntCooldown {
    /// The countdown the weapon element's `Think` runs down (`Hud::next_stunt`).
    pub remaining: f32,
    /// `gHUD.m_flTime` when the last `NextStunt` arrived (0 after a reset).
    pub received_at: f32,
    /// That message's value, tenths of a second (0 after a reset).
    pub tenths: u8,
}

/// The kung-fu bar's client-side regeneration inputs for one weapon-element `Think`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct KungfuRegen {
    /// The bar before this `Think`.
    pub power: f32,
    /// TS 3.0's step: `m_flTimeDelta × max(timescale, 0.01)` (`0x1004ce58`).
    pub step: f32,
    /// `gHUD.m_flTime` now.
    pub now: f32,
    /// `gHUD.m_flTime` at the previous `Think` (0 after a reset).
    pub last: f32,
    /// The last `TSSlowMo` value as sent (the factor × 100; 0 = normal speed, and before any).
    pub slowmo_factor100: f32,
}

/// Client rules. Every default body is TS 3.0's (`shared::gameplay::Core`); the HUD and input code
/// call these where a ruleset can differ.
pub trait ClientRules: Sync {
    /// The ruleset's name as the buy menu shows it (port addition, `menus::draw_versions`).
    fn ruleset_name(&self) -> &'static str {
        "TS 3.0"
    }

    /// `DrawCrosshair` (1004e9f0): the divesteps first (whatever the weapon), then the crosshair
    /// unless the weapon is kung fu, the M61 grenade, a knife or the katana (0, 24, 25, 34, 35).
    /// `fov` is `m_iFOV`.
    fn crosshair_parts(&self, weapon: i32, fov: f32) -> CrosshairParts {
        let _ = fov;
        CrosshairParts { crosshair: !matches!(weapon, 0 | 24 | 25 | 34 | 35), divesteps: true, divesteps_last: false }
    }

    /// The crosshair lines' distance from the centre (`0x1004eb7f`): `base + ((acc + spread) −
    /// acc) × 160`, which TS works out on the x87 stack, where it is `base + spread × 160`.
    /// `base` is 22, or 12 with the laser on; `spread` the predicted spread penalty; `accuracy`
    /// the held weapon's table accuracy (which cancels out here). Nothing is rounded to float
    /// before `__ftol`, so this is `trunc(base + spread × 160)` exactly; f64 holds a 24-bit spread
    /// times 160 without rounding (issue #110: f32 rounded a spread one ulp under `k / 160` up a
    /// pixel). Guarded: a huge predicted spread would overflow `cx + gap`; at that size the lines
    /// are far off screen anyway.
    fn crosshair_gap(&self, base: i32, spread: f32, accuracy: f32) -> i32 {
        let _ = accuracy;
        ((base as f64 + spread as f64 * 160.0) as i32).clamp(-0x10000, 0x10000)
    }

    /// The divestep indicator (`DrawCrosshair`'s helper 1004ef50, thresholds 1004eec0/1004eef0/
    /// 1004ef20): sprite `i` (top, centre, bottom) is red, too heavy for that dive, when the slots
    /// in use (`81 − free`) reach 42, 52 and 62. `armor` is `TSArmor`, `teamplay` `m_Teamplay`.
    fn divesteps_blocked(&self, used_slots: i32, armor: u8, teamplay: i32) -> [bool; 3] {
        let _ = (armor, teamplay);
        [42, 52, 62].map(|limit| used_slots >= limit)
    }

    /// Whether the stunt cooldown colours the divesteps red (1004ef50: the countdown is at least
    /// 0.01 s).
    fn stunt_cooldown(&self, cooldown: StuntCooldown, now: f32) -> bool {
        let _ = now;
        cooldown.remaining >= 0.01
    }

    /// A value `CHud::Think` writes into `default_fov` every frame before reading it. TS 3.0
    /// writes none (it clamps the cvar to 70..110 instead, 0 meaning 80).
    fn pinned_default_fov(&self) -> Option<f32> {
        None
    }

    /// Whether `UTIL_ApproachFOV` (10049620) jumps straight to the target instead of easing:
    /// only while `m_iFOV` is unset (below 0.001).
    fn fov_snaps(&self, cur: i32) -> bool {
        (cur as f32) < 0.001
    }

    /// The anti-script gate (§5.3): the nine gated `+` commands need a physical key that frame.
    fn input_gate(&self) -> bool {
        true
    }

    /// Whether `cl_attackswitch` may swap `+attack2` and `+alt2`.
    fn attack_switch(&self) -> bool {
        true
    }

    /// Whether `+attack` sends `respawn` (`FUN_10023450`, throttled to one press in 0.2 s), which
    /// is how a click spawns an observer whose wait is over.
    fn attack_sends_respawn(&self) -> bool {
        true
    }

    /// Whether the client builds the map's `ts_particle` templates, `ts_emitter`s and `ts_fog`
    /// from the BSP entity lump (`ProcessEnts` 0x1003a5d0).
    fn map_effects(&self) -> bool {
        true
    }

    /// Whether the kung-fu bar shows as exhausted, an empty red frame (`DrawKungFuBar`, power at
    /// most 0.01).
    fn kungfu_exhausted(&self, power: f32) -> bool {
        power <= 0.01
    }

    /// The kung-fu bar after one weapon-element `Think` (1004ce10): below 100 it gains 15 per
    /// step while non-negative, 1 per step while negative, up to 100.
    fn kungfu_regen(&self, r: KungfuRegen) -> f32 {
        let mut power = r.power;
        if power < 100.0 {
            let step = if power >= -0.001 { r.step * 15.0 } else { r.step };
            power = (power + step).min(100.0);
        }
        power
    }

    /// The kung-fu bar the weapon element's `Reset` (1004cd60, from every `ResetHUD`) sets, if
    /// any. TS 3.0's leaves it alone (only `VidInit` refills it).
    fn kungfu_power_on_reset(&self) -> Option<f32> {
        None
    }

    /// The buy menu's `CashBased()` (`FUN_100365d0`): whether it spends the player's `TSCash`
    /// rather than 30000 (`FUN_10036640`, [`Rules::cash`](crate::menus::buy::Rules::cash)). TS 3.0
    /// hard-wires it off, whatever the game mode (`m_Teamplay`) or `SrvSett` 1 (`cashbased`) say.
    fn buy_cash_based(&self, teamplay: i32, cashbased: i32) -> bool {
        let _ = (teamplay, cashbased);
        false
    }

    /// The buy menu's category lists, random pistols, item and key rules
    /// ([`MenuRules`](crate::menus::buy::MenuRules); its default is TS 3.0's).
    fn buy_menu(&self) -> crate::menus::buy::MenuRules {
        crate::menus::buy::MenuRules::default()
    }

    /// Whether the status panel shows `sprites/slowmotion.spr` at the top centre while the
    /// player is slowed. TS 3.0 has no such icon (it tints the screen instead).
    fn slowmo_icon(&self) -> bool {
        false
    }

    /// Whether the end of `V_CalcRefdef` tints the screen (0, 100, 125) with alpha
    /// `(1 - ts) * 50` while slowed and `cl_slowfade` is set. TS 3.0 does; the `TSFade` blend
    /// doesn't depend on this.
    fn slowmo_tint(&self) -> bool {
        true
    }

    /// The weapon fire sound's pitch before its `RandomLong(-5, 5)` (`0x1003de90`). TS 3.0:
    /// `100 - (int)(ts * -300)` (`0x1003eeed`), `ts` the eased time scale clamped to 0.01..1
    /// (400 at normal speed). `slowmo_factor100` is the last `TSSlowMo` value as sent.
    fn fire_pitch(&self, time_scale: f32, slowmo_factor100: f32) -> i32 {
        let _ = slowmo_factor100;
        100 - (time_scale * -300.0) as i32
    }

    /// `V_CalcBob` for one `V_CalcNormalRefdef`: advances `bob` (the gun offset, its pitch and,
    /// if the ruleset has one, the camera bob [`Bob::eye`](crate::view::Bob::eye)) and returns the
    /// amplitude `V_CalcGunAngle` adds to the gun's pitch. TS 3.0: [`Bob::calc`](crate::view::Bob::calc)
    /// (`0x10066e70`).
    fn view_bob(&self, bob: &mut crate::view::Bob, input: &crate::view::BobInput) -> f32 {
        bob.calc(input)
    }

    /// `V_CalcNormalRefdef`'s eye before the camera bob: `simorg` plus the view height, smoothed.
    /// TS 3.0: [`EyeZ::eye`](crate::view::EyeZ::eye), the view height eased at `min(15·ft, 1)` on
    /// z only, then `V_SmoothStairs` (`0x100677d0`) on the eye z (`0x100679ee..0x10067a46`).
    fn eye_origin(&self, eye: &mut crate::view::EyeZ, simorg: Vec3, viewheight: Vec3, on_ground: bool, ft: f32) -> Vec3 {
        eye.eye(simorg, viewheight, on_ground, ft)
    }

    /// The third-person body cam's z smoothing, on the same state as [`eye_origin`](Self::eye_origin).
    /// TS 3.0: [`EyeZ::body_cam_z`](crate::view::EyeZ::body_cam_z), `V_SmoothStairs` again
    /// (`0x10068313`).
    fn body_cam_z(&self, eye: &mut crate::view::EyeZ, z: f32, on_ground: bool, ft: f32) -> f32 {
        eye.body_cam_z(z, on_ground, ft)
    }

    /// Whether `EV_TS_FireBullets` queues client bullets (`ClientBullets_Add`) instead of tracing
    /// at once. TS 3.0 (`0x10045a8b`): with `realbullet` (`SrvSett` 0) on or the eased time scale
    /// below 0.99.
    fn client_bullets_queue(&self, realbullet: i32, time_scale: f32, slowmo_factor100: f32) -> bool {
        let _ = slowmo_factor100;
        realbullet != 0 || time_scale < 0.99
    }

    /// When a client bullet's next trace step is due, set when one runs at `now`. TS 3.0:
    /// [`next_update`](crate::events::fire::next_update) (`0x10035b97`).
    fn client_bullet_next_update(&self, now: f32, realbullet: i32) -> f32 {
        crate::events::fire::next_update(now, realbullet)
    }

    /// One client-bullet trace step along the (unnormalised) pellet direction `dir`. TS 3.0
    /// (`ClientBullets_Impact` 0x10035d40): `dir·(velocity·40)·dt·GetBulletVelocityMult`.
    fn client_bullet_step(&self, dir: [f32; 3], s: &crate::events::BulletStep) -> [f32; 3] {
        let mult = crate::events::fire::bullet_velocity_mult(s.time_scale, s.bullet_time);
        let v40 = s.velocity * 40.0;
        dir.map(|d| d * v40 * s.dt * mult)
    }

    /// The range-falloff index of a client bullet's hit `dist` units from where its leg started,
    /// `exit` for a bullet past a penetration. TS 3.0: exit bullets on a fixed 30-unit scale
    /// (`0x10035f44`), the others by the ammo's range unit (`0x10035f94`).
    fn client_bullet_range_index(&self, dist: f64, unit: f32, exit: bool) -> i32 {
        if exit {
            crate::events::ftol(dist * 0.003_333_333_4f32 as f64 * 10.0)
        } else {
            crate::events::fire::range_index(dist as f32, unit)
        }
    }

    /// Whether the client bullets are drawn this frame. TS 3.0: while the eased time scale is
    /// below 0.99.
    fn client_bullet_draws(&self, time_scale: f32, slowmo_factor100: f32) -> bool {
        let _ = slowmo_factor100;
        time_scale < 0.99
    }

    /// A ruleset's own client-bullet draw (`ClientBullets_Update`): true when it drew (and moved)
    /// the bullet. TS 3.0: false, so the core draw (the `shotgun_pellets.spr` streaks and the
    /// `bullet_trail.spr` trail) runs.
    fn client_bullet_draw(&self, bullet: &mut crate::events::ClientBullet, d: &mut crate::events::BulletDraw) -> bool {
        let _ = (bullet, d);
        false
    }

    /// The particle builders' parameters (muzzle flash, gun smoke, impact smoke, puffs, debris).
    /// TS 3.0: [`FxStyle::TS30`](crate::effects::fx::FxStyle::TS30).
    fn fx_style(&self) -> &'static crate::effects::fx::FxStyle {
        &crate::effects::fx::FxStyle::TS30
    }

    /// A ruleset's own bullet impact particles (`TS_ImpactParticles`): true when it made them.
    /// TS 3.0: false, so [`fx::impact`](crate::effects::fx::impact)'s body (`0x10048f60`) runs.
    #[allow(clippy::too_many_arguments)]
    fn impact_particles(&self, ps: &mut PSys, now: f64, pos: Vec3, n: Vec3, tex: u8, bullet_type: i32) -> bool {
        let _ = (ps, now, pos, n, tex, bullet_type);
        false
    }

    /// The unsilenced gun smoke's frame rate in `EV_TS_FireStandard`/`FireAkimbo` (`0x1003e5..`):
    /// TS 3.0 `RandomLong(5, 10)` for the sawed-off (33), else `RandomLong(45, 55)`.
    fn gun_smoke_rate(&self, id: i32) -> i32 {
        if id == 33 { random_long(5, 10) } else { random_long(45, 55) }
    }

    /// The shotgun pellet spray's gravity (`TS_SpriteDebris` from the standard and pump fire).
    /// TS 3.0: 0.
    fn pellet_spray_gravity(&self) -> f32 {
        0.0
    }

    /// A fired shell's sideways speed. TS 3.0: `RandomLong(-40, 40)` for the 17, else
    /// `RandomLong(120, 140)`.
    fn shell_side_speed(&self, id: i32) -> i32 {
        if id == 17 { random_long(-40, 40) } else { random_long(120, 140) }
    }

    /// The fire events' shell sound type (`EV_TS_EjectShell`). TS 3.0: 2 for weapon 11, else 1.
    fn shell_sound(&self, id: i32) -> i32 {
        if id == 11 { 2 } else { 1 }
    }

    /// `EV_TS_FirePump`'s shell speed ranges, `RandomLong`ed in this order: the first both
    /// sideways and forward, the second up. TS 3.0: 20-40 and 0-20 (`0x10040e43`, `0x10040e4d`).
    fn pump_shell_speeds(&self) -> [(i32, i32); 2] {
        [(20, 40), (0, 20)]
    }

    /// The fire sound's attenuation factor for ammo type `ammo` (the client ammo table's field
    /// 0). TS 3.0: [`AMMO_ATTENUATION`](crate::events::AMMO_ATTENUATION), 0.4 past it.
    fn ammo_attenuation(&self, ammo: usize) -> f32 {
        crate::events::AMMO_ATTENUATION.get(ammo).copied().unwrap_or(0.4)
    }

    /// Whether `EV_TS_FireBullets` plays the whizz-by and pushes particles along the shot.
    /// TS 3.0: true.
    fn bullet_whizz_and_push(&self) -> bool {
        true
    }

    /// The bullet material sound's attenuation and pitch (`TS_PlayTextureSound` `0x10044bf0`).
    /// TS 3.0: 0.97 and [`slow_pitch`](crate::events::slow_pitch).
    fn bullet_material_sound(&self) -> (f32, i32) {
        (crate::events::impact::MATERIAL_ATTN, crate::events::slow_pitch())
    }

    /// The ricochet's pitch. TS 3.0: [`fire_pitch`](crate::events::fire_pitch) of the eased
    /// scale (`0x10044dbd`).
    fn ricochet_pitch(&self, time_scale: f32, slowmo_factor100: f32) -> i32 {
        let _ = (time_scale, slowmo_factor100);
        crate::events::fire_pitch()
    }

    /// The other events' sounds (empty, pump, reload, the melee swings): volume, attenuation and
    /// pitch. TS 3.0: `RandomFloat(0.92, 1)`, `ATTN_NORM`, [`slow_pitch`](crate::events::slow_pitch),
    /// the pitch's `RandomLong(-5, 5)` rolled before the volume (`EV_TS_Empty`
    /// `0x10042507..0x1004256f`, `EV_TS_Knife` `0x10043feb..0x1004402e`).
    fn event_sound(&self) -> (f32, f32, i32) {
        let pitch = crate::events::slow_pitch();
        let vol = random_float(0.92, 1.0);
        (vol, goldsrc::ext::render::ATTN_NORM, pitch)
    }

    /// A melee hit's material sound (`TS_MeleeMaterialSound` `0x10045df0`) on texture type `tex`
    /// (0: no texture): volume, attenuation and pitch, or `None` for none. Called before the
    /// sample roll, so the pitch is rolled first (`0x10046066`). TS 3.0: attenuation 0.8,
    /// `slow_pitch`, volume [`melee_material_volume`].
    fn melee_material_sound(&self, tex: u8) -> Option<(f32, f32, i32)> {
        melee_material_volume(tex).map(|vol| (vol, goldsrc::ext::render::ATTN_NORM, crate::events::slow_pitch()))
    }

    /// The melee events' trace length (negative: behind). TS 3.0: close combat
    /// `-55`/`45`/`35` by attack (`0x10043427..`), coldcock 30, knife 35, katana `-55` for the
    /// back swing (15), else 50.
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

    /// Whether `EV_TS_CloseCombat` with `i1` 0 is a block (`player/block.wav`, no trace).
    /// TS 3.0: true.
    fn close_combat_blocks(&self) -> bool {
        true
    }

    /// A knife or katana hit on a hard surface: true when the ruleset made the particles. TS 3.0:
    /// false, so `TS_Scintille` and `TS_Sparks(7, 1.15, 25)` run.
    fn blade_hit_particles(&self, ps: &mut PSys, now: f64, pos: Vec3, n: Vec3, katana: bool) -> bool {
        let _ = (ps, now, pos, n, katana);
        false
    }

    /// Whether `EV_TS_Reload` plays the view animation and the shotgun phases (`fparam1` 8/9/10).
    /// TS 3.0: true.
    fn reload_event_phases(&self) -> bool {
        true
    }

    /// The kung-fu bar a `WeaponInfo` for kung fu (id 0) sets from its clip byte, if any. TS
    /// 3.0's `MsgFunc_WeaponInfo` leaves the bar to `KFuPower`.
    fn kungfu_power_from_weapon_info(&self, clip: u8) -> Option<f32> {
        let _ = clip;
        None
    }

    /// A ruleset's own key hook for the `+`/`-` commands that call TS 3.0's `respawn` key tap
    /// (`+forward`, `+back`, `+moveleft`, `+moveright`, `+attack`, `+attack2`, `+jump`, `+duck`,
    /// and the four movement releases), with the command's `IN_*` bit. Returning true replaces
    /// 3.0's key tap for a press. TS 3.0: false (the tap alone).
    fn hltv_buttons(&self, host: &mut dyn crate::spectator::HltvHost, buttons: i32, down: bool) -> bool {
        let _ = (host, buttons, down);
        false
    }

    /// A ruleset's own spectator `SetModes`, for `specmode` and the director reset. Returning
    /// false runs TS 3.0's (`0x100234d0`).
    fn hltv_set_modes(&self, host: &mut dyn crate::spectator::HltvHost, mode: i32, inset: i32) -> bool {
        let _ = (host, mode, inset);
        false
    }

    /// Resets a ruleset's own spectator state: `init` from `CHudSpectator::Init`, else from the
    /// director reset (`DRC_ACTIVE`). TS 3.0: nothing beyond its own.
    fn hltv_reset(&self, init: bool) {
        let _ = init;
    }

    /// Runs at the start of `CHudSpectator::Draw` each HUD frame. Returning true replaces the
    /// rest of 3.0's `Draw` (`0x10023080`, the HLTV name tags). TS 3.0: nothing, false.
    fn hltv_draw(&self, host: &mut dyn crate::spectator::HltvHost) -> bool {
        let _ = host;
        false
    }

    /// A ruleset's own mode passes of `V_CalcSpectatorRefdef`, after the camera overrides and
    /// the `cl_vsmoothing` rings: the main view (`nextView == 0`) and the inset window. Returning
    /// true replaces 3.0's (`0x10068f70`: in-eye, the inset modes and the in-eye gun).
    fn hltv_calc_refdef(&self, host: &mut dyn crate::spectator::HltvHost, p: &mut goldsrc::sys::ref_params_t) -> bool {
        let _ = (host, p);
        false
    }

    /// Whether `V_CalcRefdef` runs the spectator view, from `pparams->spectator` (an HLTV viewer)
    /// and the observer mode (`g_iUser1`); otherwise the normal view runs (unless paused). TS 3.0
    /// (`0x10069bb2..0x10069bbd`): either, so a player who observes gets the spectator view.
    fn spectator_refdef(&self, spectator: bool, observer_mode: i32) -> bool {
        spectator || observer_mode != 0
    }

    /// A ruleset's own refresh of the spectator camera (`origin`, `angles`) that
    /// `DrawOverviewEntities` marks on the overview while an inset mode shows. Returning true
    /// replaces 3.0's (`0x10024160`: `V_GetInEyePos` `0x10068e20` of the locked target with the
    /// in-eye inset or observer mode).
    fn hltv_overview_camera(&self, host: &dyn crate::spectator::HltvHost, origin: &mut Vec3, angles: &mut Vec3) -> bool {
        let _ = (host, origin, angles);
        false
    }

    /// Whether `DrawOverview` draws the overview map in the main view (it draws in the inset
    /// window with inset mode 3 either way). TS 3.0 (`0x10024970`): always.
    fn hltv_overview_in_main_view(&self) -> bool {
        true
    }

    /// The mode number `UpdateSpectatorPanel` shows an HLTV viewer (`#Spec_Mode%d`, and hidden
    /// at 0). `None`: the observer mode, as TS 3.0 (`0x10063870`).
    fn hltv_panel_mode(&self) -> Option<i32> {
        None
    }

    /// Whether `nohud` lets the spectator panel show. TS 3.0: an exact 0 or a NaN (issue #185).
    fn spectator_panel_shows(&self, nohud: f32) -> bool {
        crate::spectator::panel_nohud_shows(nohud)
    }

    /// The weapon panel the weapon element draws for a held weapon (not kung fu) while
    /// `cl_realhud` is 0, as a layout. `None` is TS 3.0's `DrawWeaponInfo` (1004d7e0).
    fn weapon_panel(&self, panel: &crate::hud::draw::WeaponPanel) -> Option<Vec<crate::hud::draw::HudOp>> {
        let _ = panel;
        None
    }

    /// Whether a `WeaponInfo` for kung fu (id 0) puts the weapon menu's selection back on its
    /// first slot (`menu_row` and `menu_bucket` 0; the menu stays open). TS 3.0's
    /// `MsgFunc_WeaponInfo` doesn't touch the selection.
    fn kungfu_weapon_info_resets_menu_selection(&self) -> bool {
        false
    }

    /// The kung-fu bar and its label the weapon element draws while kung fu is the weapon, as a
    /// layout for a `w` × `h` screen with the bar at `power` in the HUD colour `hud`. `None` is
    /// TS 3.0's `DrawKungFuLabel` (1004d700).
    fn kungfu_label(&self, power: f32, hud: [i32; 3], w: i32, h: i32) -> Option<Vec<crate::hud::draw::HudOp>> {
        let _ = (power, hud, w, h);
        None
    }

    /// The status panel's look ([`StatusLook`](crate::hud::draw::StatusLook)). TS 3.0's
    /// `DrawStatusPanel` (1004a3e0) by default.
    fn status_look(&self) -> crate::hud::draw::StatusLook {
        crate::hud::draw::StatusLook::default()
    }
}

/// `TS_MeleeMaterialSound`'s volume by texture type (`0x10045f0b..0x10046044`): 0.8 for tile
/// and glass, 0.5 for duct, 0.9 for the rest; flesh plays nothing.
pub fn melee_material_volume(tex: u8) -> Option<f32> {
    match tex {
        b'F' => None,
        b'T' | b'P' | b'Y' => Some(0.8),
        b'V' => Some(0.5),
        _ => Some(0.9),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Core;
    impl ClientRules for Core {}

    /// Issue #110: `0x1004eb7f` truncates `base + spread × 160` off the x87 stack, so a spread one
    /// ulp under `k / 160` stays below the next pixel.
    #[test]
    fn crosshair_gap_is_not_rounded_to_float() {
        let under = |k: f32| f32::from_bits((k / 160.0).to_bits() - 1);
        assert_eq!(under(1.0), 0.0062499996);
        for (spread, want) in [(0.0062499996f32, 22), (0.049999997, 29), (0.099999994, 37), (0.19999999, 53)] {
            assert_eq!(Core.crosshair_gap(22, spread, 0.0075), want, "spread {spread}");
        }
        for k in 1..=39 {
            assert_eq!(Core.crosshair_gap(22, under(k as f32), 0.01), 22 + k - 1, "k {k}");
        }
        assert_eq!(Core.crosshair_gap(22, 0.2, 0.0075), 54);
        for s in [f32::NAN, f32::INFINITY, f32::MAX, -f32::MAX] {
            assert!(Core.crosshair_gap(22, s, 0.01).abs() <= 0x10000);
        }
    }

    /// Issue #77: `TS_MeleeMaterialSound`'s volumes (0.9 `0x3f666666`, 0.8 `0x3f4ccccd`, 0.5
    /// `0x3f000000`).
    #[test]
    fn melee_material_volumes() {
        for (tex, want) in [(b'T', Some(0.8)), (b'P', Some(0.8)), (b'Y', Some(0.8)), (b'V', Some(0.5)), (b'F', None)] {
            assert_eq!(melee_material_volume(tex), want, "{}", tex as char);
        }
        for tex in [0, b'C', b'M', b'D', b'G', b'S', b'W', b'X'] {
            assert_eq!(melee_material_volume(tex), Some(0.9), "{}", tex as char);
        }
    }
}
