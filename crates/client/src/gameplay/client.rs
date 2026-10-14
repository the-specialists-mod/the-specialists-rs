//! [`ClientRules`]: the client-only hooks (HUD and input).
//!
//! The HUD resolves them from the server's `SrvSett` key [`SRVSETT_KEY`](ts_gameplay::SRVSETT_KEY)
//! ([`hud_rules`](super::hud_rules)), which `InitHUD` sends and a switch re-sends. Code outside the
//! HUD's own borrow (input commands, the FOV and effects frames) uses [`current`](super::current).

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
    /// the held weapon's table accuracy (which cancels out here). Guarded: a huge predicted spread
    /// would overflow `cx + gap`; at that size the lines are far off screen anyway.
    fn crosshair_gap(&self, base: i32, spread: f32, accuracy: f32) -> i32 {
        let _ = accuracy;
        ((base as f32 + spread * 160.0) as i32).clamp(-0x10000, 0x10000)
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
}
