//! TS 2.1's client rules: HUD and input (issue #100 §4), and the plugin object `client-dll`
//! registers. Addresses are TS 2.1 `client.dll` VAs (decomp `ts21/client`); see
//! `docs/implementation/ts21.md` ("Client: HUD and input").

#![deny(unsafe_code)]

use std::cmp::Ordering;
use ts_client::events::{BulletDraw, BulletStep, ClientBullet};
use ts_client::gameplay::{ClientPlugin, ClientRules, CrosshairParts, KungfuRegen, StuntCooldown};
use ts_client::menus::buy::MenuRules;
use ts_client::view::{Bob, BobInput};
use ts21_gameplay::TS21;

pub mod bullets;
pub mod view;

/// TS 2.1's client-only rules.
pub struct Ts21Client;

/// The plugin on the client: TS 2.1's shared hooks and its client hooks.
pub static PLUGIN: ClientPlugin = ClientPlugin { pm: &TS21, gun: &TS21, client: &Ts21Client };

/// 2.1's buy-menu handguns (`CTSBuyMenu` Init 0x1006aa70, table `+0x2d6c`): no single Beretta (2)
/// and no Contender (36); id 8 is the akimbo-only Berettas.
const HANDGUNS: [i32; 9] = [1, 9, 12, 14, 8, 21, 22, 28, 31];

/// `RandomConfig` (0x10070860)'s pistols, picked with `rand(1, 9)`; the MK23 and the Five-seveN may
/// be akimbo (`rand(0, 1)`).
const RANDOM_PISTOLS: [(i32, bool); 9] =
    [(1, false), (9, true), (12, false), (14, true), (8, false), (21, false), (22, false), (28, false), (31, false)];

/// `slowdiv` (0x10095800 over 0x100957c0): 1 at normal speed (`TSSlowMo` 0), else
/// `1 / max(s, 0.1f)`; a NaN factor stays NaN (the x87 compare is unordered).
pub(crate) fn slowdiv(s: f32) -> f64 {
    if s == 0.0 {
        return 1.0;
    }
    let floor = 0.1f32;
    let m = if s < floor { floor } else { s };
    1.0 / m as f64
}

/// The fire events' pitch (0x100738a0) for the slow-motion factor `s` (`TSSlowMo / 100`, 0 =
/// normal speed). `s * 300` is compared at x87 precision (`<= 150`, or unordered for a NaN,
/// gives 150) and stored as a float otherwise; `__ftol` truncates `100 + that`.
#[allow(clippy::neg_cmp_op_on_partial_ord)] // the x87 `fcomp`/`and 0x4100` test: a NaN takes 150
pub fn fire_pitch(s: f32) -> i32 {
    let x = if s == 0.0 {
        300.0f32
    } else {
        let p = s as f64 * 300.0;
        if !(p > 150.0) { 150.0 } else { p as f32 }
    };
    (100.0 + x as f64) as i64 as i32
}

impl ClientRules for Ts21Client {
    /// The weapons element's crosshair draw (0x1008b960) is one gate: nothing at all (no
    /// divesteps, no laser number) under `cl_realhud`, for kung fu, the M61, the knives and the
    /// katana, and for the Barrett unless `m_iFOV < 40` (tested before third person clears the
    /// scoped flag). Otherwise the crosshair, then the divesteps on top (0x1008c1c0).
    fn crosshair_parts(&self, weapon: i32, fov: f32) -> CrosshairParts {
        let hidden = matches!(weapon, 0 | 24 | 25 | 34 | 35) || (weapon == 18 && (fov as i32) >= 40);
        CrosshairParts { crosshair: !hidden, divesteps: !hidden, divesteps_last: true }
    }

    /// 0x1008bb67: `ftol(base + ((float)(acc + spread) − acc) / 0.1f × 16)`. Unlike 3.0, the sum is
    /// stored to a float first, and `/ 0.1f × 16` is a hair under × 160, so a spread that should
    /// give a whole number of pixels can come out one short. f64 stands in for the x87 stack.
    /// Guarded as 3.0's: the gap is clamped to ±0x10000.
    fn crosshair_gap(&self, base: i32, spread: f32, accuracy: f32) -> i32 {
        let sum = (accuracy as f64 + spread as f64) as f32;
        let gap = (sum as f64 - accuracy as f64) / 0.1f32 as f64 * 16.0 + base as f64;
        (gap as i32).clamp(-0x10000, 0x10000)
    }

    /// 0x1008c0f0/0x1008c160/0x1008c190: the top dive needs `used ≤ 21` and, when `m_Teamplay > 1`,
    /// no kevlar; the centre `used ≤ 41`, the bottom `used ≤ 71` (signed compares).
    fn divesteps_blocked(&self, used_slots: i32, armor: u8, teamplay: i32) -> [bool; 3] {
        let top = used_slots <= 21 && (teamplay < 2 || armor == 0);
        [!top, used_slots > 41, used_slots > 71]
    }

    /// `NextStunt` (0x1008d6a0) stores `m_flTime + (float)tenths / 10` and the divesteps
    /// (0x1008c1cf) are red unless that time has passed (red when it's NaN too). Real time:
    /// slow motion doesn't stretch it, unlike 3.0's countdown.
    fn stunt_cooldown(&self, c: StuntCooldown, now: f32) -> bool {
        let expiry = c.received_at + c.tenths as f32 / 10.0;
        expiry.partial_cmp(&now) != Some(Ordering::Less)
    }

    /// `CHud::Think` (0x10084aa0) writes 80.0 into `default_fov`'s value every frame, so the
    /// player's `default_fov` never takes effect.
    fn pinned_default_fov(&self) -> Option<f32> {
        Some(80.0)
    }

    /// The FOV easer (0x10084800) has 3.0's step, but jumps whenever `m_iFOV` is outside 1..80.
    fn fov_snaps(&self, cur: i32) -> bool {
        !(1..=80).contains(&cur)
    }

    /// The `+` commands (`+forward` 0x10050490, `+attack` 0x10050b40, ...; `InitInput` 0x10051b80)
    /// press their buttons unconditionally, so `alias`/`wait` scripts work.
    fn input_gate(&self) -> bool {
        false
    }

    /// No `cl_attackswitch`: `+attack2` is always `IN_ATTACK2` and `+alt2` `IN_ALT2`.
    fn attack_switch(&self) -> bool {
        false
    }

    /// The key hook (0x1004b200) is the SDK's HLTV `HandleButtonsDown`, which never sends
    /// `respawn`: a 2.1 observer respawns with `+attack` on the server.
    fn attack_sends_respawn(&self) -> bool {
        false
    }

    /// The 2.1 client reads the entity lump only for the spectator's `info_player_start`
    /// (0x1004a500): no `ts_particle`, `ts_emitter` or `ts_fog`.
    fn map_effects(&self) -> bool {
        false
    }

    /// The bar (0x1008a0f0) has no exhausted state: an empty bar is just empty.
    fn kungfu_exhausted(&self, _power: f32) -> bool {
        false
    }

    /// The weapons element's `Think` (0x100899f0): below 100, `power += (float)(now − last) /
    /// slowdiv(TSSlowMo) × 15`, at most 100, where `slowdiv` uses the raw factor (no easing);
    /// `last` is the previous `Think`'s `m_flTime`. f64 stands in for the x87 stack.
    fn kungfu_regen(&self, r: KungfuRegen) -> f32 {
        if r.power.partial_cmp(&100.0) != Some(Ordering::Less) {
            return r.power;
        }
        let dt = r.now - r.last;
        let s = r.slowmo_factor100 / 100.0;
        let power = (dt as f64 / slowdiv(s) * 15.0 + r.power as f64) as f32;
        if power > 100.0 { 100.0 } else { power }
    }

    /// The weapons element's `Reset` (0x10089890) refills the bar on every `ResetHUD`.
    fn kungfu_power_on_reset(&self) -> Option<f32> {
        Some(100.0)
    }

    /// The status `Draw` (0x10085d90) ends with `sprites/slowmotion.spr` while slowed.
    fn slowmo_icon(&self) -> bool {
        true
    }

    /// 2.1 has no `cl_slowfade` and writes no screen fade for slow motion: the icon is its only
    /// slow-motion cue (issue #152).
    fn slowmo_tint(&self) -> bool {
        false
    }

    /// The fire events' pitch helper (0x100738a0, through thunk 0x1000367f from 0x10073b70,
    /// 0x10075080 and 0x10076ce0): `s = TSSlowMo / 100` as the handler stores it (0x1031bee4,
    /// not eased), `ftol(100 + (s == 0 ? 300 : (s * 300 <= 150 ? 150 : (float)(s * 300))))`.
    /// 400 at normal speed, never below 250. The fire sound (0x10073a40) adds
    /// `RandomLong(-5, 5)`, as 3.0's.
    fn fire_pitch(&self, _time_scale: f32, slowmo_factor100: f32) -> i32 {
        fire_pitch(slowmo_factor100 / 100.0)
    }

    /// `V_CalcBob` (0x100b5c80) with its camera bob ([`view::calc`]).
    fn view_bob(&self, bob: &mut Bob, input: &BobInput) -> f32 {
        view::calc(bob, input)
    }

    /// `EV_TS_FireBullets` (0x1007c5e0): `SrvSett 0 == 1 || TSSlowMo != 0` ([`bullets::queues`]).
    fn client_bullets_queue(&self, realbullet: i32, _time_scale: f32, slowmo_factor100: f32) -> bool {
        bullets::queues(realbullet, slowmo_factor100)
    }

    /// Every 0.1 s ([`bullets::next_update`]).
    fn client_bullet_next_update(&self, now: f32, _realbullet: i32) -> f32 {
        bullets::next_update(now)
    }

    /// A fixed `dir × velocity / 0.25 × s` per step ([`bullets::step`]).
    fn client_bullet_step(&self, dir: [f32; 3], s: &BulletStep) -> [f32; 3] {
        bullets::step(dir, s)
    }

    /// The float-stored quotient ([`bullets::range_index`]).
    fn client_bullet_range_index(&self, dist: f64, unit: f32, exit: bool) -> i32 {
        bullets::range_index(dist, unit, exit)
    }

    /// Drawn while a `TSSlowMo` factor is set (0x1006a13b), on the raw factor.
    fn client_bullet_draws(&self, _time_scale: f32, slowmo_factor100: f32) -> bool {
        slowmo_factor100 / 100.0 != 0.0
    }

    /// 2.1's models and trails ([`bullets::draw`]).
    fn client_bullet_draw(&self, bullet: &mut ClientBullet, d: &mut BulletDraw) -> bool {
        bullets::draw(bullet, d);
        true
    }

    /// `CashBased()` (0x1006b2f0) is `m_Teamplay ≥ 2 || SrvSett 1`, so the menu spends the live
    /// `TSCash` in round modes and with `usecash` (0x1006b390).
    fn buy_cash_based(&self, teamplay: i32, cashbased: i32) -> bool {
        teamplay >= 2 || cashbased != 0
    }

    /// The 2.1 menu (`CTSBuyMenu` at 0x102dc084, 0x1006aa70..0x10071000): its handguns and random
    /// pistols; kevlar and stealth shoes cost no slots and skip the slot check (add 0x1006f5b0,
    /// remove 0x1006f340), and are offered only when `m_Teamplay > 1` (0x1006cde0, 0x1006e830),
    /// with no restriction of their own; no `cl_buykevlar` (a reset or random config starts
    /// empty); no akimbo fix-up, so key 6 buys any gun "akimbo" at twice the price and slots;
    /// keys 1..10 only.
    fn buy_menu(&self) -> MenuRules {
        MenuRules {
            handguns: &HANDGUNS,
            random_pistols: &RANDOM_PISTOLS,
            random_pistols_from: 1,
            items_use_slots: false,
            items_need_teamplay: true,
            stealth_needs_teamplay: false,
            kevlar_cvar: false,
            akimbo_fixup: false,
            max_key: 10,
        }
    }
}

#[cfg(test)]
mod tests;
