//! [`ClientRules`]: the client-only hooks (HUD and input).
//!
//! The HUD resolves them from the server's `SrvSett` key [`SRVSETT_KEY`](ts_gameplay::SRVSETT_KEY)
//! ([`hud_rules`](super::hud_rules)), which `InitHUD` sends on each map. Code outside the
//! HUD's own borrow (input commands, the FOV and effects frames) uses [`current`](super::current).

use crate::effects::psys::PSys;
use goldsrc::ext::render::Vec3;

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

/// Which of TS 3.0's extra parts the weapon element's `Think` runs, besides the `+use` power-up
/// press, the kung-fu regeneration and the stunt countdown every ruleset has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThinkParts {
    /// Sends `lasclr_<n>` when `cl_lasercolor` differs from the last one sent.
    pub laser_color: bool,
    /// Plays `weapons/switch.wav` on the local player when the held weapon's fire mode changes.
    pub firemode_click: bool,
    /// The heartbeat (slow motion) and breathing (low kung-fu power) sounds.
    pub sounds: bool,
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

/// What the end of `HUD_AddEntity` (the SDK's spectator code) does with an entity that passed the
/// ruleset's tests ([`ClientRules::add_entity_spectator`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpectatorTail {
    /// Nothing: the entity is drawn.
    Skip,
    /// The entity goes on the overview (`AddOverviewEntity`), and the entity `hide` (an entity
    /// index; the in-eye target) isn't drawn.
    Overview { hide: Option<i32> },
}

/// How the weapon element draws and walks its slot menu (`DrawWeaponList` 1004e030,
/// `SelectNextPrev` 1004f500).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlotMenu {
    /// The list also draws while the menu is closed (the `cl_showequipment` equipment strip) and
    /// during demo playback, rather than only while the menu is open.
    pub closed_list: bool,
    /// The selected weapon's `"%d/%d"` clip line under its name and the clip bars.
    pub clip_info: bool,
    /// `SelectNextPrev` reloads the selection from the held weapon's HUD position on every
    /// press, not only when it opens the menu.
    pub next_prev_reloads: bool,
    /// With fast switch, `SelectNextPrev` sends `weapon_<id>` for any id from 0 (kung fu) up,
    /// not only from 1.
    pub next_prev_sends_kungfu: bool,
}

/// How `AddTSMessage` (`0x1004b520`) stores and places a `TSMessage`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TsMessageLayout {
    /// The text goes through `LookupString` and a message containing the local player's name is
    /// grey (155, 155, 155) instead of the sent colour.
    pub lookup_and_highlight: bool,
    /// The characters the slot keeps.
    pub max_chars: usize,
    /// A bottom message's (flag 8) distance from the screen's bottom edge.
    pub bottom_margin: i32,
    /// `Some(w)`: centred and right-aligned messages are placed by `strlen × w` (centred: `w/2 +
    /// (strlen >> 1) × −w`); `None`: by the text's drawn width.
    pub char_width: Option<i32>,
    /// The expiry is `(float)t / 10.0 + time` in double; false: `t × 0.1f + time` in float.
    pub expiry_double: bool,
}

/// Client rules: the HUD and input code call these where a ruleset can differ. Every method is
/// required; each ruleset's plugin answers them all.
pub trait ClientRules: Sync {
    /// The ruleset's name as the buy menu shows it (port addition, `menus::draw_versions`).
    fn ruleset_name(&self) -> &'static str;

    /// Which parts `DrawCrosshair` (1004e9f0) draws for the held `weapon`; `fov` is `m_iFOV`.
    fn crosshair_parts(&self, weapon: i32, fov: f32) -> CrosshairParts;

    /// The crosshair lines' distance from the centre in pixels (`0x1004eb7f`). `base` is 22, or 12
    /// with the laser on; `spread` the predicted spread penalty; `accuracy` the held weapon's
    /// table accuracy. The caller draws the lines at `cx ± gap`, so keep the result in a range
    /// that can't overflow that.
    fn crosshair_gap(&self, base: i32, spread: f32, accuracy: f32) -> i32;

    /// The divestep indicator (`DrawCrosshair`'s helper 1004ef50): whether sprite `i` (top,
    /// centre, bottom) is red, too heavy for that dive, with `used_slots` weight slots in use.
    /// `armor` is `TSArmor`, `teamplay` `m_Teamplay`.
    fn divesteps_blocked(&self, used_slots: i32, armor: u8, teamplay: i32) -> [bool; 3];

    /// Whether the stunt cooldown colours the divesteps red, at `gHUD.m_flTime` `now`.
    fn stunt_cooldown(&self, cooldown: StuntCooldown, now: f32) -> bool;

    /// A value `CHud::Think` writes into `default_fov` every frame before reading it, if any.
    fn pinned_default_fov(&self) -> Option<f32>;

    /// Whether `UTIL_ApproachFOV` (10049620) jumps straight to the target instead of easing, with
    /// `m_iFOV` at `cur`.
    fn fov_snaps(&self, cur: i32) -> bool;

    /// Whether the anti-script gate (§5.3) runs: the nine gated `+` commands need a physical key
    /// that frame.
    fn input_gate(&self) -> bool;

    /// Whether `cl_attackswitch` may swap `+attack2` and `+alt2`.
    fn attack_switch(&self) -> bool;

    /// Whether `+attack` sends `respawn` (`FUN_10023450`, throttled to one press in 0.2 s), which
    /// is how a click spawns an observer whose wait is over.
    fn attack_sends_respawn(&self) -> bool;

    /// Whether the client builds the map's `ts_particle` templates, `ts_emitter`s and `ts_fog`
    /// from the BSP entity lump (`ProcessEnts` 0x1003a5d0).
    fn map_effects(&self) -> bool;

    /// Whether the kung-fu bar at `power` shows as exhausted, an empty red frame (`DrawKungFuBar`).
    fn kungfu_exhausted(&self, power: f32) -> bool;

    /// The kung-fu bar after one weapon-element `Think` (1004ce10).
    fn kungfu_regen(&self, r: KungfuRegen) -> f32;

    /// Which of the weapon element's `Think` (1004ce10) optional parts run.
    fn think_parts(&self) -> ThinkParts;

    /// The kung-fu bar the weapon element's `Reset` (1004cd60, from every `ResetHUD`) sets, if
    /// any (`None` leaves it alone).
    fn kungfu_power_on_reset(&self) -> Option<f32>;

    /// The buy menu's `CashBased()` (`FUN_100365d0`): whether it spends the player's `TSCash`
    /// rather than 30000 (`FUN_10036640`, [`Rules::cash`](crate::menus::buy::Rules::cash)), for the
    /// game mode `teamplay` (`m_Teamplay`) and `SrvSett` 1 (`cashbased`).
    fn buy_cash_based(&self, teamplay: i32, cashbased: i32) -> bool;

    /// The buy menu's category lists, random pistols, item and key rules
    /// ([`MenuRules`](crate::menus::buy::MenuRules)).
    fn buy_menu(&self) -> crate::menus::buy::MenuRules;

    /// Whether the status panel shows `sprites/slowmotion.spr` at the top centre while the
    /// player is slowed.
    fn slowmo_icon(&self) -> bool;

    /// Whether the end of `V_CalcRefdef` tints the screen (0, 100, 125) with alpha
    /// `(1 - ts) * 50` while slowed and `cl_slowfade` is set. The `TSFade` blend doesn't depend
    /// on this.
    fn slowmo_tint(&self) -> bool;

    /// The weapon fire sound's pitch before its `RandomLong(-5, 5)` (`0x1003de90`), from `ts`, the
    /// eased time scale clamped to 0.01..1, and `slowmo_factor100`, the last `TSSlowMo` value as
    /// sent.
    fn fire_pitch(&self, time_scale: f32, slowmo_factor100: f32) -> i32;

    /// `V_CalcBob` for one `V_CalcNormalRefdef`: advances `bob` (the gun offset, its pitch and,
    /// if the ruleset has one, the camera bob [`Bob::eye`](crate::view::Bob::eye)) and returns the
    /// amplitude `V_CalcGunAngle` adds to the gun's pitch.
    fn view_bob(&self, bob: &mut crate::view::Bob, input: &crate::view::BobInput) -> f32;

    /// `V_CalcNormalRefdef`'s eye before the camera bob: `simorg` plus the view height, smoothed
    /// on the state `eye` keeps across frames.
    fn eye_origin(&self, eye: &mut crate::view::EyeZ, simorg: Vec3, viewheight: Vec3, on_ground: bool, ft: f32) -> Vec3;

    /// The third-person body cam's z smoothing, on the same state as [`eye_origin`](Self::eye_origin).
    fn body_cam_z(&self, eye: &mut crate::view::EyeZ, z: f32, on_ground: bool, ft: f32) -> f32;

    /// Whether `EV_TS_FireBullets` queues client bullets (`ClientBullets_Add`) instead of tracing
    /// at once, from `realbullet` (`SrvSett` 0), the eased time scale and the last `TSSlowMo`.
    fn client_bullets_queue(&self, realbullet: i32, time_scale: f32, slowmo_factor100: f32) -> bool;

    /// When a client bullet's next trace step is due, set when one runs at `now`.
    fn client_bullet_next_update(&self, now: f32, realbullet: i32) -> f32;

    /// One client-bullet trace step along the (unnormalised) pellet direction `dir`
    /// (`ClientBullets_Impact` 0x10035d40).
    fn client_bullet_step(&self, dir: [f32; 3], s: &crate::events::BulletStep) -> [f32; 3];

    /// The range-falloff index of a client bullet's hit `dist` units from where its leg started,
    /// `exit` for a bullet past a penetration; `unit` is the ammo's range unit.
    fn client_bullet_range_index(&self, dist: f64, unit: f32, exit: bool) -> i32;

    /// Whether the client bullets are drawn this frame.
    fn client_bullet_draws(&self, time_scale: f32, slowmo_factor100: f32) -> bool;

    /// A ruleset's own client-bullet draw (`ClientBullets_Update`): true when it drew (and moved)
    /// the bullet; false runs the core draw (the `shotgun_pellets.spr` streaks and the
    /// `bullet_trail.spr` trail).
    fn client_bullet_draw(&self, bullet: &mut crate::events::ClientBullet, d: &mut crate::events::BulletDraw) -> bool;

    /// The particle builders' parameters (muzzle flash, gun smoke, impact smoke, puffs, debris).
    fn fx_style(&self) -> &'static crate::effects::fx::FxStyle;

    /// A ruleset's own bullet impact particles (`TS_ImpactParticles`): true when it made them;
    /// false runs [`fx::impact`](crate::effects::fx::impact)'s body (`0x10048f60`).
    #[allow(clippy::too_many_arguments)]
    fn impact_particles(&self, ps: &mut PSys, now: f64, pos: Vec3, n: Vec3, tex: u8, bullet_type: i32) -> bool;

    /// The unsilenced gun smoke's frame rate in `EV_TS_FireStandard`/`FireAkimbo` (`0x1003e5..`)
    /// for weapon `id`.
    fn gun_smoke_rate(&self, id: i32) -> i32;

    /// The shotgun pellet spray's gravity (`TS_SpriteDebris` from the standard and pump fire).
    fn pellet_spray_gravity(&self) -> f32;

    /// A fired shell's sideways speed for weapon `id`.
    fn shell_side_speed(&self, id: i32) -> i32;

    /// The fire events' shell sound type (`EV_TS_EjectShell`) for weapon `id`.
    fn shell_sound(&self, id: i32) -> i32;

    /// `EV_TS_FirePump`'s shell speed ranges, `RandomLong`ed in this order: the first both
    /// sideways and forward, the second up.
    fn pump_shell_speeds(&self) -> [(i32, i32); 2];

    /// The fire sound's attenuation factor for ammo type `ammo` (the client ammo table's field
    /// 0).
    fn ammo_attenuation(&self, ammo: usize) -> f32;

    /// Whether `EV_TS_FireBullets` plays the whizz-by and pushes particles along the shot.
    fn bullet_whizz_and_push(&self) -> bool;

    /// The bullet material sound's attenuation and pitch (`TS_PlayTextureSound` `0x10044bf0`).
    fn bullet_material_sound(&self) -> (f32, i32);

    /// The ricochet's pitch, from the eased time scale and the last `TSSlowMo` value as sent.
    fn ricochet_pitch(&self, time_scale: f32, slowmo_factor100: f32) -> i32;

    /// The other events' sounds (empty, pump, reload, the melee swings): volume, attenuation and
    /// pitch. Any random rolls happen here, in the ruleset's order.
    fn event_sound(&self) -> (f32, f32, i32);

    /// A melee hit's material sound (`TS_MeleeMaterialSound` `0x10045df0`) on texture type `tex`
    /// (0: no texture): volume, attenuation and pitch, or `None` for none. Called before the
    /// sample roll.
    fn melee_material_sound(&self, tex: u8) -> Option<(f32, f32, i32)>;

    /// The melee events' trace length (negative: behind).
    fn melee_reach(&self, m: MeleeReach) -> f32;

    /// Whether `EV_TS_CloseCombat` with `i1` 0 is a block (`player/block.wav`, no trace).
    fn close_combat_blocks(&self) -> bool;

    /// A knife or katana hit on a hard surface: true when the ruleset made the particles; false
    /// runs `TS_Scintille` and `TS_Sparks(7, 1.15, 25)`.
    fn blade_hit_particles(&self, ps: &mut PSys, now: f64, pos: Vec3, n: Vec3, katana: bool) -> bool;

    /// Whether `EV_TS_Reload` plays the view animation and the shotgun phases (`fparam1` 8/9/10).
    fn reload_event_phases(&self) -> bool;

    /// The kung-fu bar a `WeaponInfo` for kung fu (id 0) sets from its `clip` byte, if any
    /// (`None` leaves the bar to `KFuPower`).
    fn kungfu_power_from_weapon_info(&self, clip: u8) -> Option<f32>;

    /// A ruleset's own key hook for the `+`/`-` commands that call the core's `respawn` key tap
    /// (`+forward`, `+back`, `+moveleft`, `+moveright`, `+attack`, `+attack2`, `+jump`, `+duck`,
    /// and the four movement releases), with the command's `IN_*` bit. Returning true replaces
    /// the key tap for a press.
    fn hltv_buttons(&self, host: &mut dyn crate::spectator::HltvHost, buttons: i32, down: bool) -> bool;

    /// A ruleset's own spectator `SetModes`, for `specmode` and the director reset. Returning
    /// false runs the core's (`0x100234d0`).
    fn hltv_set_modes(&self, host: &mut dyn crate::spectator::HltvHost, mode: i32, inset: i32) -> bool;

    /// Resets a ruleset's own spectator state: `init` from `CHudSpectator::Init`, else from the
    /// director reset (`DRC_ACTIVE`).
    fn hltv_reset(&self, init: bool);

    /// Runs at the start of `CHudSpectator::Draw` each HUD frame. Returning true replaces the
    /// rest of the core's `Draw` (`0x10023080`, the HLTV name tags).
    fn hltv_draw(&self, host: &mut dyn crate::spectator::HltvHost) -> bool;

    /// A ruleset's own mode passes of `V_CalcSpectatorRefdef`, after the camera overrides and
    /// the `cl_vsmoothing` rings: the main view (`nextView == 0`) and the inset window. Returning
    /// true replaces the core's (`0x10068f70`: in-eye, the inset modes and the in-eye gun).
    fn hltv_calc_refdef(&self, host: &mut dyn crate::spectator::HltvHost, p: &mut goldsrc::sys::ref_params_t) -> bool;

    /// Whether `V_CalcRefdef` runs the spectator view, from `pparams->spectator` (an HLTV viewer)
    /// and the observer mode (`g_iUser1`); otherwise the normal view runs (unless paused).
    fn spectator_refdef(&self, spectator: bool, observer_mode: i32) -> bool;

    /// A ruleset's own refresh of the spectator camera (`origin`, `angles`) that
    /// `DrawOverviewEntities` marks on the overview while an inset mode shows. Returning true
    /// replaces the core's (`0x10024160`: `V_GetInEyePos` `0x10068e20` of the locked target with
    /// the in-eye inset or observer mode).
    fn hltv_overview_camera(&self, host: &dyn crate::spectator::HltvHost, origin: &mut Vec3, angles: &mut Vec3) -> bool;

    /// Whether `DrawOverview` draws the overview map in the main view (it draws in the inset
    /// window with inset mode 3 either way).
    fn hltv_overview_in_main_view(&self) -> bool;

    /// The mode number `UpdateSpectatorPanel` shows an HLTV viewer (`#Spec_Mode%d`, and hidden
    /// at 0). `None`: the observer mode.
    fn hltv_panel_mode(&self) -> Option<i32>;

    /// Whether the cvar value `nohud` lets the spectator panel show.
    fn spectator_panel_shows(&self, nohud: f32) -> bool;

    /// The weapon panel the weapon element draws for a held weapon (not kung fu) while
    /// `cl_realhud` is 0, as a layout. `None` draws the core's `DrawWeaponInfo` (1004d7e0).
    fn weapon_panel(&self, panel: &crate::hud::draw::WeaponPanel) -> Option<Vec<crate::hud::draw::HudOp>>;

    /// Whether a `WeaponInfo` for kung fu (id 0) puts the weapon menu's selection back on its
    /// first slot (`menu_row` and `menu_bucket` 0; the menu stays open).
    fn kungfu_weapon_info_resets_menu_selection(&self) -> bool;

    /// The kung-fu bar and its label the weapon element draws while kung fu is the weapon, as a
    /// layout for a `w` × `h` screen with the bar at `power` in the HUD colour `hud`. `None`
    /// draws the core's `DrawKungFuLabel` (1004d700).
    fn kungfu_label(&self, power: f32, hud: [i32; 3], w: i32, h: i32) -> Option<Vec<crate::hud::draw::HudOp>>;

    /// The status panel's look ([`StatusLook`](crate::hud::draw::StatusLook)), which
    /// `DrawStatusPanel` (1004a3e0) draws with.
    fn status_look(&self) -> crate::hud::draw::StatusLook;

    // Audit round 2: A1

    /// Whether `HUD_AddEntity` makes a ground item (`iuser4` bit 1, 4 or 2) translucent with
    /// distance inside its cull distance (`effects::ItemCull::fade`).
    fn ground_items_fade(&self) -> bool;

    /// The spectator tail at the end of `HUD_AddEntity`, for every entity that got there.
    fn add_entity_spectator(&self) -> SpectatorTail;

    /// The lasers', laser dot's, flashlights' and shadows' look
    /// ([`LaserStyle`](crate::effects::LaserStyle)).
    fn laser_style(&self) -> &'static crate::effects::LaserStyle;

    /// A ruleset's own `TSFrags` effects (glass, explosion, sparks): true when it made them; false
    /// runs the core's (`0x1004bd50`: `TS_GlassShatter`, `TS_Explosion`, `TS_Scintille` and
    /// `TS_Sparks`).
    fn tsfrags_effects(&self, ps: &mut PSys, now: f64, frags: &crate::effects::TsFrags) -> bool;

    /// A ruleset's own `PTakeDam` blood: true when it made it; false runs `TS_BloodImpact`
    /// ([`fx::blood`](crate::effects::fx::blood)).
    fn blood_impact(&self, ps: &mut PSys, now: f64, d: &crate::effects::PTakeDam) -> bool;

    // Audit round 2: A2

    /// The weapon slot menu's drawing and `invnext`/`invprev` walk ([`SlotMenu`]).
    fn slot_menu(&self) -> SlotMenu;

    /// A ruleset's own `SelectSlot(n)` after the wire-panel test (the text-menu and buy-menu
    /// keys are the caller's): `Some(cmd)` replaces the core's (`1004f220`), with the server
    /// command to send, if any; `None` runs the core's.
    fn select_slot(&self, h: &mut crate::hud::state::Hud, n: i32, fastswitch: bool) -> Option<Option<String>>;

    /// Whether a `Spectator` message for the local player toggles the camera, entering
    /// (`observer`) or leaving observer mode, with the camera now `third_person` and the
    /// `cl_thirdperson` cvar's value.
    fn spectator_toggles_third_person(&self, observer: bool, third_person: bool, cl_thirdperson: f32) -> bool;

    /// `AddTSMessage`'s text and placement ([`TsMessageLayout`]).
    fn ts_message_layout(&self) -> TsMessageLayout;

    /// The power-up name `DrawPowerup` shows for the `PwUp` flag bit `flag` (1, 2, 8 or 0x10):
    /// `Some(text)` as written, `None` for the core's `titles.txt` key (`#Pwr_SlowMotion`, ..).
    fn powerup_label(&self, flag: u32) -> Option<&'static str>;

    /// The scoreboard's status title for `TSPAward` `award`: `Some(text)` replaces the core's
    /// title (the colour stays), `None` keeps it.
    fn award_title(&self, award: u8) -> Option<&'static str>;

    /// Whether a `TSSlowMo` plays `goslow.wav`: `time_scale` is the eased scale before the
    /// message, `target` the new target scale (1 for a 0), `prev_factor100` and `factor100` the
    /// last and the new value as sent.
    fn go_slow_sound(&self, time_scale: f32, target: f32, prev_factor100: f32, factor100: f32) -> bool;

    /// `V_CalcNormalRefdef`'s third-person camera offsets (back, right, up), after the duck and
    /// stunt/death smoothing on the state `ofs` keeps across frames.
    fn camera_offsets(&self, ofs: &mut crate::view::CamOffsets, c: &crate::view::CamInput) -> (f32, f32, f32);

    /// `V_CalcViewRoll`: sets or adds to `p.viewangles[2]`, with the eased roll `roll` if the
    /// ruleset eases one; `view` is the view entity (`pparams->viewentity`), `local` the local
    /// player.
    fn view_roll(
        &self,
        roll: &mut f32,
        p: &mut goldsrc::sys::ref_params_t,
        view: Option<crate::view::RollEntity>,
        local: Option<crate::view::RollEntity>,
    );

    /// Whether `V_CalcNormalRefdef` sets the local entity's render mode and eases its
    /// `renderamt` (the third-person translucency, `cl_thirdtransparency`).
    fn local_translucency(&self) -> bool;

    // Audit round 3: render

    /// Which of TS's studio-renderer changes run (`crate::studio`): the view-model slow motion,
    /// the torso yaw fix, the forced player `renderamt`, the renderfx 17 corpse and ghost draws and
    /// the hardware abs-bbox debug view.
    fn studio_style(&self) -> crate::studio::StudioStyle;

    // Audit round 3: hud

    /// The say-text element's mute check and plain-line colour ([`SayTextStyle`](crate::hud::saytext::SayTextStyle)).
    fn say_text_style(&self) -> crate::hud::saytext::SayTextStyle;

    /// The particle update ([`ParticleModel`](crate::effects::psys::ParticleModel)).
    fn particle_model(&self) -> crate::effects::psys::ParticleModel;

    // Audit round 3: gui

    /// How the VGUI1 panels (team menu, MOTD, scoreboard, spectator bar) scale TS's 640×480
    /// layout: an optimised `client.dll`'s float reciprocals and spills, or an unoptimised one's
    /// division.
    fn vgui_scale(&self) -> crate::hud::vgui::VguiScale;

    // Audit round 4: render

    /// Which `client.dll` build's float forms the studio renderer (blends, controllers,
    /// interpolant, player blend) and the view kick (`V_ApplyKick`) replicate.
    fn float_form(&self) -> crate::studio::FloatForm;

    /// Whether the client drives OpenGL fog: `V_SetFog`'s map and single-player user fog
    /// (`gl_fog`, `gl_fog_*`) and the `glDisable(GL_FOG)` that ends every transparent pass.
    fn gl_fog(&self) -> bool;

    // Audit round 4: gui

    /// How the HLTV overview (layer, icons, name tags) rounds its floats: an optimised
    /// `client.dll`'s x87 stack or an unoptimised one's float stores.
    fn hltv_overview_floats(&self) -> crate::spectator::OverviewFloats;
}

/// The test plugin's client hooks (`gameplay::Defaults`): inert answers (no, none, zero, the
/// core's defaults, the input unchanged), so the core's tests exercise the core's own code. The
/// ruleset's numbers are tested with its plugin.
#[cfg(test)]
mod stub {
    use super::*;
    use crate::effects::fx::FxStyle;
    use crate::events::{BulletDraw, BulletStep, ClientBullet};
    use crate::hud::draw::{HudOp, StatusLook, WeaponPanel};
    use crate::menus::buy::MenuRules;
    use crate::spectator::HltvHost;
    use crate::view::{Bob, BobInput, EyeZ};

    /// Builder parameters with every effect off.
    static FX: FxStyle = FxStyle {
        flash_color: 0.0,
        flash_fade: 0.0,
        flash_scale_speed: 0.0,
        third_person_flash: 0.0,
        muzzle_blast: false,
        muzzle_spark: false,
        silenced_count: 0.0,
        silenced_color: 0.0,
        silenced_fade: None,
        silenced_spin: 0,
        smoke_gravity: 0.0,
        smoke_puffs_by_cl_fx: false,
        drag: 0.0,
        stretch: false,
        debris_react: 0.0,
        debris_fade_any_mode: false,
        debris_count_by_quality: false,
    };

    /// TS 3.0's look (the core's own code paths).
    static LASER: crate::effects::LaserStyle = crate::effects::LaserStyle {
        animated_beam: true,
        dot_range: None,
        others_from_origin: false,
        other_beams: true,
        shadow_fade: true,
    };

    const NONE: CrosshairParts = CrosshairParts { crosshair: false, divesteps: false, divesteps_last: false };

    impl ClientRules for crate::gameplay::Defaults {
        fn ruleset_name(&self) -> &'static str {
            "test"
        }
        fn crosshair_parts(&self, _weapon: i32, _fov: f32) -> CrosshairParts {
            NONE
        }
        fn crosshair_gap(&self, base: i32, _spread: f32, _accuracy: f32) -> i32 {
            base
        }
        fn divesteps_blocked(&self, _used_slots: i32, _armor: u8, _teamplay: i32) -> [bool; 3] {
            [false; 3]
        }
        fn stunt_cooldown(&self, _cooldown: StuntCooldown, _now: f32) -> bool {
            false
        }
        fn pinned_default_fov(&self) -> Option<f32> {
            None
        }
        fn fov_snaps(&self, _cur: i32) -> bool {
            false
        }
        fn input_gate(&self) -> bool {
            false
        }
        fn attack_switch(&self) -> bool {
            false
        }
        fn attack_sends_respawn(&self) -> bool {
            false
        }
        fn map_effects(&self) -> bool {
            false
        }
        fn kungfu_exhausted(&self, _power: f32) -> bool {
            false
        }
        fn kungfu_regen(&self, r: KungfuRegen) -> f32 {
            r.power
        }
        fn think_parts(&self) -> ThinkParts {
            ThinkParts { laser_color: false, firemode_click: false, sounds: false }
        }
        fn kungfu_power_on_reset(&self) -> Option<f32> {
            None
        }
        fn buy_cash_based(&self, _teamplay: i32, _cashbased: i32) -> bool {
            false
        }
        fn buy_menu(&self) -> MenuRules {
            MenuRules::default()
        }
        fn slowmo_icon(&self) -> bool {
            false
        }
        fn slowmo_tint(&self) -> bool {
            false
        }
        fn fire_pitch(&self, _time_scale: f32, _slowmo_factor100: f32) -> i32 {
            100
        }
        fn view_bob(&self, _bob: &mut Bob, _input: &BobInput) -> f32 {
            0.0
        }
        fn eye_origin(&self, _eye: &mut EyeZ, simorg: Vec3, viewheight: Vec3, _on_ground: bool, _ft: f32) -> Vec3 {
            [0, 1, 2].map(|a| simorg[a] + viewheight[a])
        }
        fn body_cam_z(&self, _eye: &mut EyeZ, z: f32, _on_ground: bool, _ft: f32) -> f32 {
            z
        }
        fn client_bullets_queue(&self, _realbullet: i32, _time_scale: f32, _slowmo_factor100: f32) -> bool {
            false
        }
        fn client_bullet_next_update(&self, now: f32, _realbullet: i32) -> f32 {
            now
        }
        fn client_bullet_step(&self, _dir: [f32; 3], _s: &BulletStep) -> [f32; 3] {
            [0.0; 3]
        }
        fn client_bullet_range_index(&self, _dist: f64, _unit: f32, _exit: bool) -> i32 {
            0
        }
        fn client_bullet_draws(&self, _time_scale: f32, _slowmo_factor100: f32) -> bool {
            false
        }
        fn client_bullet_draw(&self, _bullet: &mut ClientBullet, _d: &mut BulletDraw) -> bool {
            false
        }
        fn fx_style(&self) -> &'static FxStyle {
            &FX
        }
        fn impact_particles(&self, _ps: &mut PSys, _now: f64, _pos: Vec3, _n: Vec3, _tex: u8, _bullet_type: i32) -> bool {
            false
        }
        fn gun_smoke_rate(&self, _id: i32) -> i32 {
            0
        }
        fn pellet_spray_gravity(&self) -> f32 {
            0.0
        }
        fn shell_side_speed(&self, _id: i32) -> i32 {
            0
        }
        fn shell_sound(&self, _id: i32) -> i32 {
            0
        }
        fn pump_shell_speeds(&self) -> [(i32, i32); 2] {
            [(0, 0); 2]
        }
        fn ammo_attenuation(&self, _ammo: usize) -> f32 {
            0.0
        }
        fn bullet_whizz_and_push(&self) -> bool {
            false
        }
        fn bullet_material_sound(&self) -> (f32, i32) {
            (0.0, 100)
        }
        fn ricochet_pitch(&self, _time_scale: f32, _slowmo_factor100: f32) -> i32 {
            100
        }
        fn event_sound(&self) -> (f32, f32, i32) {
            (1.0, 0.0, 100)
        }
        fn melee_material_sound(&self, _tex: u8) -> Option<(f32, f32, i32)> {
            None
        }
        fn melee_reach(&self, _m: MeleeReach) -> f32 {
            0.0
        }
        fn close_combat_blocks(&self) -> bool {
            false
        }
        fn blade_hit_particles(&self, _ps: &mut PSys, _now: f64, _pos: Vec3, _n: Vec3, _katana: bool) -> bool {
            false
        }
        fn reload_event_phases(&self) -> bool {
            false
        }
        fn kungfu_power_from_weapon_info(&self, _clip: u8) -> Option<f32> {
            None
        }
        fn hltv_buttons(&self, _host: &mut dyn HltvHost, _buttons: i32, _down: bool) -> bool {
            false
        }
        fn hltv_set_modes(&self, _host: &mut dyn HltvHost, _mode: i32, _inset: i32) -> bool {
            false
        }
        fn hltv_reset(&self, _init: bool) {}
        fn hltv_draw(&self, _host: &mut dyn HltvHost) -> bool {
            false
        }
        fn hltv_calc_refdef(&self, _host: &mut dyn HltvHost, _p: &mut goldsrc::sys::ref_params_t) -> bool {
            false
        }
        fn spectator_refdef(&self, _spectator: bool, _observer_mode: i32) -> bool {
            false
        }
        fn hltv_overview_camera(&self, _host: &dyn HltvHost, _origin: &mut Vec3, _angles: &mut Vec3) -> bool {
            false
        }
        fn hltv_overview_in_main_view(&self) -> bool {
            false
        }
        fn hltv_panel_mode(&self) -> Option<i32> {
            None
        }
        fn spectator_panel_shows(&self, _nohud: f32) -> bool {
            false
        }
        fn weapon_panel(&self, _panel: &WeaponPanel) -> Option<Vec<HudOp>> {
            None
        }
        fn kungfu_weapon_info_resets_menu_selection(&self) -> bool {
            false
        }
        fn kungfu_label(&self, _power: f32, _hud: [i32; 3], _w: i32, _h: i32) -> Option<Vec<HudOp>> {
            None
        }
        fn status_look(&self) -> StatusLook {
            StatusLook::default()
        }

        // Audit round 2: A1

        fn ground_items_fade(&self) -> bool {
            false
        }
        fn add_entity_spectator(&self) -> SpectatorTail {
            SpectatorTail::Skip
        }
        fn laser_style(&self) -> &'static crate::effects::LaserStyle {
            &LASER
        }
        fn tsfrags_effects(&self, _ps: &mut PSys, _now: f64, _frags: &crate::effects::TsFrags) -> bool {
            false
        }
        fn blood_impact(&self, _ps: &mut PSys, _now: f64, _d: &crate::effects::PTakeDam) -> bool {
            false
        }

        // Audit round 2: A2

        fn slot_menu(&self) -> SlotMenu {
            // The core's own (TS 3.0's) menu, which its tests exercise.
            SlotMenu { closed_list: true, clip_info: true, next_prev_reloads: false, next_prev_sends_kungfu: false }
        }
        fn select_slot(&self, _h: &mut crate::hud::state::Hud, _n: i32, _fastswitch: bool) -> Option<Option<String>> {
            None
        }
        fn spectator_toggles_third_person(&self, _observer: bool, third_person: bool, _cl_thirdperson: f32) -> bool {
            third_person
        }
        fn ts_message_layout(&self) -> TsMessageLayout {
            // The core's own (TS 3.0's), which its tests exercise.
            TsMessageLayout { lookup_and_highlight: true, max_chars: 127, bottom_margin: 64, char_width: None, expiry_double: false }
        }
        fn powerup_label(&self, _flag: u32) -> Option<&'static str> {
            None
        }
        fn award_title(&self, _award: u8) -> Option<&'static str> {
            None
        }
        fn go_slow_sound(&self, _time_scale: f32, _target: f32, _prev_factor100: f32, _factor100: f32) -> bool {
            false
        }
        fn camera_offsets(&self, _ofs: &mut crate::view::CamOffsets, c: &crate::view::CamInput) -> (f32, f32, f32) {
            (-c.cam_back, c.cam_right, c.cam_up)
        }
        fn view_roll(
            &self,
            _roll: &mut f32,
            _p: &mut goldsrc::sys::ref_params_t,
            _view: Option<crate::view::RollEntity>,
            _local: Option<crate::view::RollEntity>,
        ) {
        }
        fn local_translucency(&self) -> bool {
            false
        }
        fn studio_style(&self) -> crate::studio::StudioStyle {
            // The core's own code paths, which its tests exercise.
            crate::studio::StudioStyle::ALL
        }

        // Audit round 3: hud

        fn say_text_style(&self) -> crate::hud::saytext::SayTextStyle {
            // The core's own (TS 3.0's), which its tests exercise.
            crate::hud::saytext::SayTextStyle { mute_blocked: true, plain_color: Some(crate::hud::saytext::SAY_COLOR) }
        }
        fn particle_model(&self) -> crate::effects::psys::ParticleModel {
            crate::effects::psys::ParticleModel::Drag
        }

        // Audit round 3: gui

        fn vgui_scale(&self) -> crate::hud::vgui::VguiScale {
            crate::hud::vgui::VguiScale::Reciprocal
        }

        // Audit round 4: render

        fn float_form(&self) -> crate::studio::FloatForm {
            crate::studio::FloatForm::Optimised
        }
        fn gl_fog(&self) -> bool {
            true
        }

        // Audit round 4: gui

        fn hltv_overview_floats(&self) -> crate::spectator::OverviewFloats {
            crate::spectator::OverviewFloats::Stack
        }
    }
}
