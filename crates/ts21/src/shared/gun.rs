//! TS 2.1's gun (issue #100 §2.3, decomp `docs/ts21/weapons.md` §3 and `client-protocol.md`
//! §3.2): the shot functions, recoil, spread and the timer model, which the server and the
//! client's prediction both run. Server addresses are 2.1 `mp.dll`'s, client ones 2.1
//! `client.dll`'s; where the two copies round differently the code follows each
//! ([`GunHost::prediction`]).
//!
//! The timer model: 2.1 fixes each delay when it is set, `delay * T` with
//! `T = fuser1 ? 1 / max(fuser1, 0.1) : 1` (or `x 6.6666665` for the Slow Pause owner), and runs
//! the timers down by the raw frame time. TS 3.0 sets raw delays and slows the timers instead.

use crate::Ts21;
use crate::shared::tables;
use shared::gameplay::{GunRules, Predicted, SpreadDecay};
use shared::pm::Arith;
use shared::ts::powerup;
use shared::weapons::random::shared_random_float_unrounded;
use shared::weapons::spread::{PelletOffset, PelletSide, Vec3};
use shared::weapons::table::{Tables, WeaponInfo};
use shared::weapons::{Event, EventArgs, Gun, GunHost, id, info, kind, kind_of, mode};
use std::cmp::Ordering;

impl GunRules for Ts21 {
    fn tables(&self) -> &'static Tables {
        tables::tables()
    }

    fn fire(&self, gun: &mut Gun, host: &mut dyn GunHost) {
        fire(self, gun, host);
    }

    /// `StandardFire` (0x1004b400): semi-auto waits the weapon's own rate.
    fn semi_auto_delay(&self, rate: f32) -> f32 {
        rate
    }

    /// `CBasePlayer::PostThink` (0x10054180) runs the weapon timers down by
    /// `gpGlobals->frametime`, the client (`FUN_10017f70`) by `msec / 1000.0`: no time scale and
    /// no double-fire-rate speed-up.
    fn timer_step(&self, _arith: Arith, frametime: f64, _fuser1: f32, _powerups: u16) -> f64 {
        frametime
    }

    /// `FUN_10017f70` (`0x10018921..`): `t - (double)msec / 1000.0`, the quotient unrounded.
    fn command_frame_time(&self, msec: u8) -> f64 {
        msec as f64 / 1000.0
    }

    /// The decay runs on game time: the server's `PreThink` (0x100537e0, 0x10053a40..) while
    /// alive, the client's `FUN_10017f70` on a first prediction:
    /// `pen -= (now - last) / T * 0.1` when `pen > 0` and `last <= now`, clamped at 0; `last = now`
    /// every think. `T` is the server player's `fuser1`'s, or the HUD's `TSSlowMo` on the client.
    /// There is no double-fire-rate factor.
    fn decay_spread(&self, gun: &mut Gun, d: &SpreadDecay) {
        if d.alive && gun.spread_penalty > 0.0 && gun.decay_time <= d.time {
            let elapsed = d.time - gun.decay_time;
            let v = gun.spread_penalty as f64 - elapsed as f64 / slow_div(d.time_scale) * 0.1;
            gun.spread_penalty = if v < 0.0 { 0.0 } else { v as f32 };
        }
        gun.decay_time = d.time;
    }

    /// HL's triangle pattern (`ShootBulletsPlayer` 0x1003fd40, client `FUN_1007c190`): pellet `i`
    /// is `x = U(seed+i) + U(seed+i+1)`, `y = U(seed+i+2) + U(seed+i+3)`, U the shared random
    /// float in [-0.5, 0.5). A bot's pellets (server, `FL_FAKECLIENT`) take the engine's
    /// `RandomFloat` the same way, drawn again until `x² + y² <= 1`.
    fn pellet_offset(&self, seed: u32, i: u32, bot_random: Option<&mut dyn FnMut(f32, f32) -> f32>) -> Option<(f32, f32)> {
        if let Some(rf) = bot_random {
            loop {
                let x = (rf(-0.5, 0.5) as f64 + rf(-0.5, 0.5) as f64) as f32;
                let y = (rf(-0.5, 0.5) as f64 + rf(-0.5, 0.5) as f64) as f32;
                if x as f64 * x as f64 + y as f64 * y as f64 <= 1.0 {
                    return Some((x, y));
                }
            }
        }
        Some(triangle(seed, i))
    }

    fn pellet_dir(&self, side: PelletSide, aim: Vec3, right: Vec3, up: Vec3, spread: [f32; 2], offset: PelletOffset) -> Vec3 {
        pellet_dir(side, aim, right, up, spread, offset)
    }

    /// `EV_TS_FireAkimbo` (client `FUN_10075080`): a linked shot at +4 and -4, one gun at
    /// `bparam1 ? +4 : -4`, matching the server's `AkimboFire` offsets.
    fn akimbo_event_offsets(&self, linked: bool, toggle: bool) -> (f32, Option<f32>) {
        if linked { (4.0, Some(-4.0)) } else { (if toggle { 4.0 } else { -4.0 }, None) }
    }

    /// The client's `ItemPostFrame` (`FUN_10025f40`) doesn't refill a finished magazine reload
    /// (it calls a stub, `FUN_1003fb80`), has no +alt2 branch and no reload key without a weapon.
    fn predicted(&self) -> Predicted {
        Predicted { reload_finish: false, alt_attack: false, kungfu_reload: false }
    }

    /// `FUN_1002d840` (the player's `ItemPostFrame` gate): +alt2 lets the frame run during any
    /// magazine reload; 2.1 doesn't ask whether the weapon has a stock hit.
    fn cancel_runs_frame_in_reload(&self, _info: &WeaponInfo) -> bool {
        true
    }

    fn better_weapon(&self, gun: &Gun, excluded: u8) -> u8 {
        better_weapon(gun, excluded)
    }

    /// `PrimaryAttack` (0x1004aa50) calls `KnifeFire`/`KatanaFire` and `SecondaryAttack`
    /// (0x1004a800) `KnifeFire` only when the button went down this frame (`pl+0x770`).
    fn blades_need_press(&self) -> bool {
        true
    }

    /// `SecondaryAttack` (0x1004a800) has no katana branch: the katana falls through to the
    /// scope / alt-grip code, has neither, and only sets the 0.3 / 1.0 timers.
    fn katana_throws(&self) -> bool {
        false
    }

    /// `WeaponIdle` (0x1004d660): `clip < 1` reloads, with no `InfiniteClips` test.
    fn infinite_clip_skips_idle_reload(&self) -> bool {
        false
    }

    /// 2.1's HolsterWeapon (`FUN_1004d7a0`, from `Holster`, `Deploy` of id 0 and every weapon
    /// switch): `viewmodel = weaponmodel = fov = 0` and nothing else; 2.1 has no kung-fu view
    /// model, draw animation or `events/draw.sc`.
    fn holster_draws_kungfu(&self) -> bool {
        false
    }
}

/// 2.1's `GetBetterWeapon` (FUN_1004a410): the owned weapon other than `excluded` with the
/// highest `cost × (akimbo ? 2 : 1)` (`info+0xd0`, the kind byte `FUN_10049a10` tests for 4),
/// strictly greater so the lowest id wins a tie; kung fu when none is worth more than 0.
pub fn better_weapon(gun: &Gun, excluded: u8) -> u8 {
    let (mut best, mut pick) = (0i32, 0u8);
    for id in 1..gun.status.len() as u8 {
        if !gun.status[id as usize].owned || id == excluded {
            continue;
        }
        let Some(w) = info(id) else { continue };
        let worth = w.cost.wrapping_mul(if kind_of(id) == kind::AKIMBO { 2 } else { 1 });
        if worth > best {
            best = worth;
            pick = id;
        }
    }
    pick
}

/// The server's slow-motion divisor (`FUN_100551c0` -> `FUN_1005a3d0`, `FUN_1005a3b0`):
/// `fuser1 == 0 ? 1 : 1 / max(fuser1, 0.1f)`, left on the FPU.
pub fn slow_div(fuser1: f32) -> f64 {
    if fuser1 == 0.0 { 1.0 } else { 1.0 / (if fuser1 < 0.1 { 0.1f32 } else { fuser1 }) as f64 }
}

/// The client's divisor for fire delays and `ScaleSpread` (`FUN_100261b0`): no 0.1 floor.
pub fn client_slow_div(fuser1: f32) -> f64 {
    if fuser1 == 0.0 { 1.0 } else { 1.0 / fuser1 as f64 }
}

/// The time scale divisor of this side for the fire delays and `ScaleSpread`.
fn side_slow_div(host: &dyn GunHost) -> f64 {
    if host.prediction() { client_slow_div(host.time_scale()) } else { slow_div(host.time_scale()) }
}

/// The direction of a pellet at offset `o` ([`GunRules::pellet_dir`]), as 2.1's builds round it
/// (issue #161). `o.y` is a `float` here (the triangle's sums are stored). With `a = x·sx` and
/// `b = y·sy` unrounded:
///
/// * the server (`ShootBulletsPlayer` 0x100401b7..0x10040260): `(aim + b·up) + a·right`, the x
///   component wholly on the FPU; for y and z the products `a·right` and `b·up` and the sum
///   `aim + b·up` are stored. 2.1 shipped no Linux server, so `ts_pm_arith gcc` computes the same.
/// * the client (`FUN_1007c5e0` 0x1007c97f..0x1007ca0d, unoptimised but these stay on the FPU):
///   `(a·right + aim) + b·up`, stored once.
pub fn pellet_dir(side: PelletSide, aim: Vec3, right: Vec3, up: Vec3, spread: [f32; 2], o: PelletOffset) -> Vec3 {
    let (a, b) = (o.x as f64 * spread[0] as f64, o.y * spread[1] as f64);
    let (r, u, aim) = (right.map(f64::from), up.map(f64::from), aim.map(f64::from));
    match side {
        PelletSide::Server(_) => {
            let f = |v: f64| v as f32 as f64;
            [
                ((aim[0] + b * u[0]) + a * r[0]) as f32,
                (f(aim[1] + f(b * u[1])) + f(a * r[1])) as f32,
                (f(aim[2] + f(b * u[2])) + f(a * r[2])) as f32,
            ]
        }
        PelletSide::Client => [0, 1, 2].map(|i| ((a * r[i] + aim[i]) + b * u[i]) as f32),
    }
}

/// Pellet `i`'s triangle offset ([`GunRules::pellet_offset`]): each sum is stored as a `float`.
pub fn triangle(seed: u32, i: u32) -> (f32, f32) {
    let u = |k: u32| shared_random_float_unrounded(seed.wrapping_add(i).wrapping_add(k), -0.5, 0.5);
    ((u(0) + u(1)) as f32, (u(2) + u(3)) as f32)
}

/// The delay tail of the three shot functions: `rate * (double fire rate ? 0.5 : 1)` for the
/// weapon's own rate (stored as a `float`; burst and semi delays don't get it).
fn rate_delay(rate: f32, host: &dyn GunHost) -> f32 {
    if host.powerups() & powerup::DOUBLE_FIRERATE != 0 { (rate as f64 * 0.5) as f32 } else { rate }
}

/// The rest of the delay tail: the Slow Pause owner's `x 6.6666665f` (the client divides by the
/// `double` 0.15), else `x T`, stored once.
pub fn scale_delay(delay: f32, host: &dyn GunHost) -> f32 {
    if host.powerups() & powerup::SLOWPAUSE != 0 {
        if host.prediction() { (delay as f64 / 0.15) as f32 } else { (delay as f64 * 6.666_666_5f32 as f64) as f32 }
    } else {
        (side_slow_div(host) * delay as f64) as f32
    }
}

/// The burst delay (0.03 s between rounds, 0.2 s after the last), `double`s stored as `float`s.
fn burst_delay(state: i32) -> f32 {
    if state == 1 { 0.2f64 as f32 } else { 0.03f64 as f32 }
}

/// The damage of one bullet of weapon `id` with multiplier `mult`: the ammo's integer base damage
/// times it ([`tables`]).
fn shot_damage(w: &WeaponInfo, mult: f32) -> f32 {
    tables::ammo_damage(w.ammo) as f32 * mult
}

/// `info+0x1c` with the silencer's `x 0.8` (a `double`), stored as a `float`.
fn damage_param(id: u8, silenced: bool) -> f32 {
    let m = tables::damage_multiplier(id);
    if silenced { (m as f64 * 0.8) as f32 } else { m }
}

/// The silencer's `x 1.2` (a `double`) on the spread, stored as a `float`.
fn silenced_spread(spread: f32) -> f32 {
    (spread as f64 * 1.2) as f32
}

/// `CTSGun::Fire` (`FUN_1004ac20`): the underwater and empty clicks (no Infinite Clip bypass: the
/// power-up only saves the round), then the shot and the third-person animation.
pub fn fire(rules: &Ts21, gun: &mut Gun, host: &mut dyn GunHost) {
    let cur = gun.current;
    if cur == 0 {
        return;
    }
    if host.waterlevel() == 3 {
        empty_sound(gun, host);
        return;
    }
    let s = gun.status[cur as usize];
    let k = kind_of(cur);
    if s.clip < 1 && !(s.toggle && s.mode == mode::PUMP && k == kind::SHOTGUN) {
        empty_sound(gun, host);
        return;
    }
    match k {
        kind::AKIMBO | kind::AKIMBO_ONLY => {
            if akimbo_fire(gun, host) {
                let toggle = gun.status[gun.current as usize].toggle;
                host.player_anim(if toggle { 9 } else { 8 });
            }
        }
        kind::SHOTGUN => {
            shotgun_fire(gun, host);
            host.player_anim(0);
        }
        _ => {
            standard_fire(rules, gun, host);
            host.player_anim(0);
        }
    }
}

/// `EmptySound` (`FUN_1004ab60`): the click, and both attacks wait `T * 0.1` (no Slow Pause rule,
/// no Contender exception), idle 0.5.
pub fn empty_sound(gun: &mut Gun, host: &mut dyn GunHost) {
    host.event(Event::Empty, EventArgs::default());
    let d = (side_slow_div(host) * 0.1) as f32;
    gun.next_primary = d;
    gun.next_secondary = d;
    gun.idle = 0.5;
}

/// `ScaleSpread` (`FUN_10049a50`, client `FUN_1001e660`): adds the recoil penalty to `spread` and
/// grows it. `k = double fire rate ? 1 : 2`, no silencer factor; `x = dt / T` from the last shot
/// (0.5 from 1 up), any `dt >= 0`. The client's runs on a first prediction only and stores `x` as
/// a `float`.
pub fn scale_spread(gun: &mut Gun, spread: &mut f32, akick: f32, host: &dyn GunHost) {
    if akick <= 0.0 || !host.first_prediction() {
        return;
    }
    let k = if host.powerups() & powerup::DOUBLE_FIRERATE != 0 { 1.0 } else { 2.0 };
    let now = host.time();
    let dt = now - gun.last_shot;
    if dt >= 0.0 {
        *spread += gun.spread_penalty;
        let pen = gun.spread_penalty as f64;
        if host.prediction() {
            let mut x = (dt as f64 / side_slow_div(host)) as f32;
            if x >= 1.0 {
                x = 0.5;
            }
            if x < 1.0 {
                let v = (1.0 - x as f64) * akick as f64 * k + pen;
                gun.spread_penalty = if v >= 0.1 { 0.1 } else { v as f32 };
            }
        } else {
            let mut x = dt as f64 / side_slow_div(host);
            // `fcom 1.0`: a NaN `x` takes the "1 or more" branch.
            if x.partial_cmp(&1.0) != Some(Ordering::Less) {
                x = 0.5;
            }
            let v = (1.0 - x) * k * akick as f64 + pen;
            gun.spread_penalty = if v >= 0.1 { 0.1 } else { v as f32 };
        }
    }
    gun.last_shot = now;
}

/// `VKickback` (server `FUN_10049b80`, client `FUN_1001e8e0`): a random kick. With
/// `s = silenced ? 0.8 : 1`, `u = SR(seed, -0.5, 0.5)`, `p = SR(seed+1, kick/2, kick)` and
/// `r2 = SR(seed+2, 0, 0.75)`: yaw = `(s·u ± 0.5 ∓ r2) · kick · 0.65` (the ± by the sign of
/// `s·u`; rifles, shotguns and snipers subtract `r2`, pistols add it). The SDK's shared random
/// float truncates the range to an integer, so `r2` is always 0 and `p` spans
/// `trunc(kick / 2)` from `kick / 2`.
///
/// The server applies `kick` itself as the pitch (bots only, [`GunHost::kick`]) and discards
/// `p`; the client's view takes `p · s` when `kick > 0`. Neither shakes the view.
pub fn vkickback(kick: f32, k: u8, silenced: bool, host: &mut dyn GunHost) {
    let seed = host.seed();
    let u = shared_random_float_unrounded(seed, -0.5, 0.5);
    let low = if host.prediction() { (kick as f64 / 2.0) as f32 } else { (kick as f64 * 0.5) as f32 };
    let p = shared_random_float_unrounded(seed.wrapping_add(1), low, kick);
    let r2 = shared_random_float_unrounded(seed.wrapping_add(2), 0.0, 0.75) as f32;
    let side = |y: f32| if y < 0.0 { -0.5 } else { 0.5 };
    let adjust = |y: f64| match k {
        kind::RIFLE | kind::SHOTGUN | kind::SNIPER => y - r2 as f64,
        kind::PISTOL => y + r2 as f64,
        _ => y,
    };
    if host.prediction() {
        // Every step is stored as a `float` (`0x1001e93e..0x1001e9d6`).
        let s = if silenced { 0.8f64 as f32 } else { 1.0 };
        let y0 = (u * s as f64) as f32;
        let pitch = (p * s as f64) as f32;
        let y1 = (y0 as f64 + side(y0)) as f32;
        let y2 = adjust(y1 as f64) as f32;
        // `test ah,0x41; jp` (`0x1001e9e4`): any kick that isn't 0 or below, NaN included.
        if matches!(kick.partial_cmp(&0.0), Some(Ordering::Greater) | None) {
            host.kick(pitch, (y2 as f64 * kick as f64 * 0.65) as f32, false);
        }
    } else {
        let s = if silenced { 0.8 } else { 1.0 };
        let y0 = (s * u) as f32;
        let y = adjust(y0 as f64 + side(y0));
        host.kick(kick, (y * kick as f64 * 0.65) as f32, false);
    }
}

/// `StandardFire` (`FUN_1004b400`, client `FUN_100213a0`). In 2.1's order: the round is taken
/// first, the silencer widens the spread x1.2, a Barrett or M60 moving faster than 80 u/s sets
/// the penalty to 0.1, `ScaleSpread`, the shot (damage x0.8 silenced), the kick, the event, and
/// the delay: burst 0.03/0.2, semi the weapon's rate ([`GunRules::semi_auto_delay`]), auto the
/// rate halved by double fire rate; then [`scale_delay`].
pub fn standard_fire(rules: &Ts21, gun: &mut Gun, host: &mut dyn GunHost) {
    let cur = gun.current;
    let Some(w) = info(cur) else { return };
    if !Gun::infinite_clip(host) {
        let st = &mut gun.status[cur as usize];
        st.clip = st.clip.wrapping_sub(1);
        if w.akimbo != -1 {
            let clip = st.clip;
            let tw = &mut gun.status[w.akimbo as usize];
            tw.clip = tw.clip.wrapping_sub(1);
            tw.left = clip;
            if tw.clip < 0 {
                tw.clip = 0;
            }
        }
    }
    let silenced = gun.silenced();
    let mut spread = w.accuracy;
    if silenced {
        spread = silenced_spread(spread);
    }
    // 2.1's `Length() > 80` (mp.dll 0x1004b5ca..0x1004b5f2: `x, y, z`, the root unrounded; its
    // client.dll's out-of-line `Length` 0x1001e850 returns the `double` `sqrt` unrounded too).
    if (cur == id::M60 || cur == id::BARRETT) && shared::weapons::gun::speed_over::<shared::pm::Msvc>(host.velocity(), 80.0) {
        gun.spread_penalty = 0.1;
    }
    if w.scale_spread {
        scale_spread(gun, &mut spread, w.acc_kick, host);
    }
    let seed = host.seed();
    gun.shoot(host, spread, shot_damage(w, damage_param(cur, silenced)), seed, 0.0);
    let k = kind_of(cur);
    if k == kind::SNIPER || cur == id::RAGING_BULL || cur == id::SAWEDOFF || w.kick {
        vkickback(w.view_kick, k, silenced, host);
    }
    let s = gun.status[cur as usize];
    host.event(
        Event::Fire,
        EventArgs { f1: spread, f2: spread, i1: cur as i32, i2: s.items as i32, b1: (s.clip == 0) as i32, b2: seed as i32 },
    );
    let delay = match s.mode {
        mode::BURST => burst_delay(s.state),
        mode::SEMI => rules.semi_auto_delay(w.rate),
        _ => rate_delay(w.rate, host),
    };
    gun.next_primary = scale_delay(delay, host);
    gun.idle = 2.0;
}

/// `ShotgunFire` (`FUN_1004b050`): firing during a shell reload ends it; the delay (semi 0.1,
/// else the rate halved by double fire rate, then [`scale_delay`]) is set before the ammo check;
/// the shot has no silencer and no `ScaleSpread`; the kick is x1.3 in semi; the pump toggles.
pub fn shotgun_fire(gun: &mut Gun, host: &mut dyn GunHost) {
    let cur = gun.current;
    let Some(w) = info(cur) else { return };
    let s = gun.status[cur as usize];
    if s.state != 0 {
        gun.status[cur as usize].state = 2;
        return;
    }
    let pumping = s.mode == mode::PUMP && s.toggle;
    let delay = if s.mode == mode::SEMI { 0.1 } else { rate_delay(w.rate, host) };
    gun.next_primary = scale_delay(delay, host);
    gun.idle = 2.0;
    if s.clip < 1 {
        if !pumping {
            return;
        }
    } else if !pumping && !Gun::infinite_clip(host) {
        gun.status[cur as usize].clip -= 1;
    }
    let seed = host.seed();
    let items = s.items as i32;
    let pump = (s.mode == mode::PUMP) as i32;
    if !pumping {
        gun.shoot(host, w.accuracy, shot_damage(w, tables::damage_multiplier(cur)), seed, 0.0);
        if w.kick {
            let f = if s.mode == mode::SEMI { 1.3f32 } else { 1.0 };
            vkickback(f * w.view_kick, kind_of(cur), false, host);
        }
        host.event(Event::Fire, EventArgs { f1: w.accuracy, f2: w.accuracy, i1: cur as i32, i2: items, b1: pump, b2: seed as i32 });
    } else {
        host.event(Event::Pump, EventArgs { f1: 0.0, f2: 0.0, i1: cur as i32, i2: items, b1: pump, b2: 0 });
    }
    let st = &mut gun.status[cur as usize];
    st.toggle = !st.toggle;
}

/// `AkimboFire` (`FUN_1004b8c0`): TS 3.0's gun choice and ammo, with 2.1's silencer (spread
/// x1.2, damage x0.8), `ScaleSpread` with the kick doubled for a linked shot, and every bullet 4
/// units off-centre: a linked shot from `toggle ? -4 : +4` along `v_right` (seed) and the other
/// side (seed + 0x21), one gun from `toggle ? +4 : -4`. Delay: burst 0.03/0.2, semi modes 0.1,
/// else the rate halved by double fire rate; then [`scale_delay`] on both attacks.
pub fn akimbo_fire(gun: &mut Gun, host: &mut dyn GunHost) -> bool {
    let cur = gun.current;
    let Some(w) = info(cur) else { return false };
    let s = gun.status[cur as usize];
    let free_mode = s.mode == mode::AKIMBO_SEMI || s.mode == mode::AKIMBO_AUTO;
    let both = free_mode && {
        let b = if s.mode == mode::AKIMBO_AUTO { host.buttons() } else { host.pressed() };
        b & IN_ATTACK != 0 && b & IN_ATTACK2 != 0
    };
    let mut linked = both || s.mode == mode::AUTO;
    if !free_mode || linked {
        let st = &mut gun.status[cur as usize];
        st.toggle = !st.toggle;
        if st.right < 1 {
            linked = false;
            st.toggle = true;
        }
        if st.left < 1 {
            linked = false;
            st.toggle = false;
        }
    } else {
        let buttons = host.buttons();
        if buttons & IN_ATTACK != 0 {
            if s.right < 1 {
                empty_sound(gun, host);
                return false;
            }
            gun.status[cur as usize].toggle = false;
        }
        if buttons & IN_ATTACK2 != 0 {
            if s.left < 1 {
                empty_sound(gun, host);
                return false;
            }
            gun.status[cur as usize].toggle = true;
        }
    }
    if !Gun::infinite_clip(host) {
        let st = &mut gun.status[cur as usize];
        if linked {
            st.clip -= 2;
            st.left -= 1;
            st.right -= 1;
        } else {
            st.clip -= 1;
            if st.toggle {
                st.left -= 1;
            } else {
                st.right -= 1;
            }
        }
        st.clip = st.clip.max(0);
        st.left = st.left.max(0);
        st.right = st.right.max(0);
        let left = st.left;
        if kind_of(cur) == kind::AKIMBO && w.single >= 0 {
            gun.status[w.single as usize].clip = left;
        }
    }
    let silenced = gun.silenced();
    let mut spread = w.accuracy;
    if silenced {
        spread = silenced_spread(spread);
    }
    if w.scale_spread {
        let akick = if linked { w.acc_kick * 2.0 } else { w.acc_kick };
        scale_spread(gun, &mut spread, akick, host);
    }
    let damage = shot_damage(w, damage_param(cur, silenced));
    let seed = host.seed();
    let toggle = gun.status[cur as usize].toggle;
    if linked {
        let off = if toggle { -4.0 } else { 4.0 };
        gun.shoot(host, spread, damage, seed, off);
        gun.shoot(host, spread, damage, seed.wrapping_add(0x21), -off);
    } else {
        gun.shoot(host, spread, damage, seed, if toggle { 4.0 } else { -4.0 });
    }
    if w.kick {
        let vk = if linked { w.view_kick * 2.0 } else { w.view_kick };
        vkickback(vk, kind_of(cur), silenced, host);
    }
    let items = gun.status[cur as usize].items as i32;
    host.event(
        if linked { Event::FireLinked } else { Event::Fire },
        EventArgs { f1: spread, f2: spread, i1: cur as i32, i2: items, b1: toggle as i32, b2: seed as i32 },
    );
    let s = gun.status[cur as usize];
    let delay = match s.mode {
        mode::BURST => burst_delay(s.state),
        mode::SEMI | mode::AKIMBO_SEMI => 0.1,
        _ => rate_delay(w.rate, host),
    };
    let d = scale_delay(delay, host);
    gun.next_secondary = d;
    gun.next_primary = d;
    gun.idle = 2.0;
    true
}

const IN_ATTACK: u32 = 1;
const IN_ATTACK2: u32 = 0x800;

#[cfg(test)]
mod tests;
