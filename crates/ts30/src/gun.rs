//! TS 3.0's gun: `CTSGun::Fire` and the shots it fires (standard, shotgun, akimbo) with their
//! spread and recoil, the weapon timer step, the recoil penalty's decay and `GetBetterWeapon`'s
//! pick: the `GunRules` answers. Server addresses are `ts_i386.so`'s.

use crate::{Ts30, recoil};
use shared::gameplay::{GunRules, Predicted, SpreadDecay, WeaponPreThink};
use shared::pm::Arith;
use shared::weapons::spread::{self, PelletOffset, PelletSide, Vec3};
use shared::weapons::table::{Tables, WeaponInfo};
use shared::weapons::{Event, EventArgs, Gun, GunHost, MAX_WEAPONS, id, kind, kind_of, mode, mode_mask};

const IN_ATTACK: u32 = 1;
const IN_ATTACK2: u32 = 0x800;

/// `g_iWeaponPriority` (auto-switch order after a drop), `compile_weapons_info` 0x7fe28.
pub const PRIORITY: [u8; 38] = [
    18, 32, 13, 5, 15, 27, 11, 20, 4, 26, 33, 16, 37, 6, 23, 7, 3, 17, 19, 10, 8, 21, 30, 31, 12, 36, 22, 9, 2, 14, 28, 1, 34, 25, 35, 24,
    29, 0,
];

impl GunRules for Ts30 {
    /// TS 3.0's embedded tables ([`crate::tables::tables`]).
    fn tables(&self) -> &'static Tables {
        crate::tables::tables()
    }

    /// `CTSGun::Fire` (0x81c78): [`fire`].
    fn fire(&self, gun: &mut Gun, host: &mut dyn GunHost) {
        fire(gun, host);
    }

    /// `CTSGun::StandardFire` (0x824a4): 0.1 s, whatever the weapon's own rate.
    fn semi_auto_delay(&self, rate: f32) -> f32 {
        let _ = rate;
        0.1
    }

    /// `frametime * GiveSlowMul() * (double fire rate ? 1.5 : 1)` (`CBasePlayer::PostThink`
    /// 0x91ae7, client `0x10003ec5`; [`Gun::timer_dt`]).
    fn timer_step(&self, arith: Arith, frametime: f64, fuser1: f32, powerups: u16) -> f64 {
        Gun::timer_dt(arith, frametime as f32, spread::fixed_slow_amt(fuser1), powerups) as f64
    }

    /// The client's `HUD_PostRunCmd`: `(float)(msec * 0.001)` with the `double` constant
    /// (`0x10003eb7`), which is also the engine's `frametime` for the server's `PostThink`.
    fn command_frame_time(&self, msec: u8) -> f64 {
        (msec as f64 * 0.001) as f32 as f64
    }

    /// `PreThink` (0x80d88) while alive, and the client's `HUD_WeaponsPostThink` (`0x100040a5`) on
    /// a first prediction: `pen -= step * 0.1` by the frame's timer step ([`decay_spread`]). The
    /// client copies the penalty to its crosshair on every first prediction (`0x100040dc..`), so
    /// always true.
    fn decay_spread(&self, gun: &mut Gun, d: &SpreadDecay) -> bool {
        if d.alive {
            decay_spread(gun, d.arith, d.step);
        }
        true
    }

    /// The weapon parts of `CBasePlayer::PreThink` (0x908cc) after `CTSStunt::Play`, by the
    /// frame's timer step ([`pre_think_dt`]): the recoil decay while alive
    /// ([`GunRules::decay_spread`]), the bar and the window's closure ([`pre_frame`]).
    fn pre_think(&self, gun: &mut Gun, disarm: &mut f32, t: &WeaponPreThink) {
        let _ = disarm;
        let dt = pre_think_dt(t);
        pre_frame(gun, t.arith, dt, t.alive);
        let decay = SpreadDecay { arith: t.arith, step: dt, time: t.time, time_scale: t.fuser1, alive: t.alive };
        self.decay_spread(gun, &decay);
    }

    /// TS 3.0's polar pattern, which each side computes as its build does
    /// ([`spread::polar_offset`] of each side's random draws): `None`.
    fn pellet_offset(&self, seed: u32, i: u32, bot_random: Option<&mut dyn FnMut(f32, f32) -> f32>) -> Option<(f32, f32)> {
        let _ = (seed, i, bot_random);
        None
    }

    /// [`spread::pellet_dir`], rounded as each of TS 3.0's builds does (issue #161).
    fn pellet_dir(&self, side: PelletSide, aim: Vec3, right: Vec3, up: Vec3, spread: [f32; 2], offset: PelletOffset) -> Vec3 {
        spread::pellet_dir(side, aim, right, up, spread, offset)
    }

    /// +5 then -5 for a linked shot (`0x100409be..0x10040a01`, 5.0 at `0x1007a8d0` with `b2`;
    /// `0x10040a66..0x10040aa5`, -5.0 at `0x1007d6f8` with `b2 + 0x21`; `0x10189e50` is
    /// `AngleVectors`' right at `0x1003f1f8`), none for one gun.
    fn akimbo_event_offsets(&self, linked: bool, toggle: bool) -> (f32, Option<f32>) {
        let _ = toggle;
        if linked { (5.0, Some(-5.0)) } else { (0.0, None) }
    }

    /// `ItemPostFrame` (`0x1000b7b0`) runs them all.
    fn predicted(&self) -> Predicted {
        Predicted::ALL
    }

    /// `CBasePlayer::ItemPostFrame` (0xdc4d0): a weapon with a stock hit.
    fn cancel_runs_frame_in_reload(&self, info: &WeaponInfo) -> bool {
        info.stock != 0
    }

    /// `CTSGun::GetBetterWeapon(excluded)` (0x80f40): [`better_weapon`], the first owned one in
    /// `g_iWeaponPriority`.
    fn better_weapon(&self, gun: &Gun, excluded: u8) -> u8 {
        better_weapon(gun, excluded)
    }

    /// `PrimaryAttack` (0x81a20) and `SecondaryAttack` (0x817a8) attack while the button is held.
    fn blades_need_press(&self) -> bool {
        false
    }

    /// `SecondaryAttack` (0x817a8) → `KatanaThrow`.
    fn katana_throws(&self) -> bool {
        true
    }

    /// `WeaponIdle` (0x854dc): `!InfiniteClips(pl) && clip < 1`.
    fn infinite_clip_skips_idle_reload(&self) -> bool {
        true
    }

    /// `HolsterWeapon` (0x85698) unzooms and draws kung fu.
    fn holster_draws_kungfu(&self) -> bool {
        true
    }

    // Audit round 2: C

    /// The weapon parts of `CBasePlayer::PreThink` (0x908cc) before `CTSStunt::Play`
    /// (0x909c5..0x90b0b): the melee window and the block timers ([`pre_frame_timers`]) and the
    /// disarm count's `- dt × 5`, floored at 0.
    fn pre_think_timers(&self, gun: &mut Gun, disarm: &mut f32, t: &WeaponPreThink) {
        let dt = pre_think_dt(t);
        pre_frame_timers(gun, dt);
        *disarm = (*disarm - dt * 5.0).max(0.0);
    }
}

/// `PreThink`'s timer step `local_90` (0x909c5): [`Gun::timer_dt`], `frametime × GiveSlowMul() ×
/// (double fire rate ? 1.5 : 1)`, worked out once before `Play` and used after it too.
fn pre_think_dt(t: &WeaponPreThink) -> f32 {
    Gun::timer_dt(t.arith, t.frametime, spread::fixed_slow_amt(t.fuser1), t.powerups)
}

/// `CTSGun::Fire` 0x81c78: the empty and underwater
/// clicks, then the akimbo, shotgun or standard shot and the third-person animation.
pub fn fire(gun: &mut Gun, host: &mut dyn GunHost) {
    if !gun.has_slot() {
        return;
    }
    if host.waterlevel() == 3 {
        gun.empty_sound(host);
        return;
    }
    let s = gun.status[gun.current as usize];
    let k = gun.kind();
    if !Gun::infinite_clip(host) && s.clip < 1 && (!s.toggle || s.mode != mode::PUMP || k != kind::SHOTGUN) {
        gun.empty_sound(host);
        return;
    }
    let shoot_anim = if k == kind::AKIMBO || k == kind::AKIMBO_ONLY {
        akimbo_fire(gun, host).then(|| if gun.status[gun.current as usize].toggle { 9 } else { 8 })
    } else {
        if k == kind::SHOTGUN {
            shotgun_fire(gun, host);
        } else {
            standard_fire(gun, host);
        }
        Some(0)
    };
    if let Some(a) = shoot_anim {
        host.player_anim(a);
    }
}

/// `CTSGun::ScaleSpread` 0x802a8: adds the recoil penalty to `spread` and grows it. The
/// client's (`0x10008100`) does both, and the last-shot update, on a first prediction only.
pub fn scale_spread(gun: &mut Gun, spread: &mut f32, akick: f32, silenced: bool, host: &dyn GunHost) {
    if silenced {
        *spread *= 0.875;
    }
    if akick > 0.0 && host.first_prediction() {
        let mut k = if Gun::double_fire_rate(host) { 1.0 } else { 2.0 };
        if silenced {
            k *= 0.775;
        }
        let now = host.time();
        let arith = host.arith();
        if let Some(elapsed) = recoil::shot_interval(arith, now, gun.last_shot) {
            *spread += gun.spread_penalty;
            gun.spread_penalty = recoil::grow_spread_penalty(arith, gun.spread_penalty, host.slow_mul(), elapsed, akick, k);
        }
        gun.last_shot = now;
    }
}

/// `CTSGun::VKickback` 0x80414.
pub fn vkickback(view_kick: f32, k: u8, silenced: bool, host: &mut dyn GunHost) {
    let (p, y) = recoil::view_kick(host.arith(), view_kick, k, silenced, Gun::double_fire_rate(host));
    host.kick(p, y, true);
}

/// `CTSGun::StandardFire` 0x824a4.
pub fn standard_fire(gun: &mut Gun, host: &mut dyn GunHost) {
    let info = gun.info();
    let silenced = gun.silenced();
    let mut spread = info.accuracy;
    if gun.current == id::BARRETT && Gun::faster_than(host, 35.0) {
        gun.spread_penalty = 0.3;
    }
    if gun.current == id::M60 && Gun::faster_than(host, 175.0) {
        scale_spread(gun, &mut spread, info.acc_kick * 5.0, silenced, host);
    } else if info.scale_spread {
        scale_spread(gun, &mut spread, info.acc_kick, silenced, host);
    }
    let damage = if silenced { info.damage * 0.775 } else { info.damage };
    let seed = host.seed();
    gun.shoot(host, spread, damage, seed, 0.0);
    let k = gun.kind();
    if k == kind::SNIPER || gun.current == id::RAGING_BULL || gun.current == id::SAWEDOFF || info.kick {
        vkickback(info.view_kick, k, silenced, host);
    }
    if !Gun::infinite_clip(host) {
        let s = gun.cur();
        s.clip = s.clip.wrapping_sub(1);
        if info.akimbo != -1 {
            let clip = gun.status[gun.current as usize].clip;
            let tw = &mut gun.status[info.akimbo as usize];
            tw.clip = tw.clip.wrapping_sub(1);
            tw.left = clip;
            if tw.clip < 0 {
                tw.clip = 0;
            }
        }
    }
    let s = gun.status[gun.current as usize];
    host.event(
        Event::Fire,
        EventArgs { f1: spread, f2: spread, i1: gun.current as i32, i2: s.items as i32, b1: (s.clip == 0) as i32, b2: seed as i32 },
    );
    let rate = if s.mode == mode::BURST {
        if s.state == 1 { 0.2 } else { 0.03 }
    } else if s.mode == mode::SEMI && mode_mask(info) & 1 != 0 {
        host.rules().semi_auto_delay(info.rate)
    } else {
        info.rate
    };
    gun.next_primary = rate;
    gun.idle = if gun.current == id::CONTENDER && s.clip < 1 && !Gun::infinite_clip(host) { 0.5 } else { 2.0 };
}

/// `CTSGun::ShotgunFire` 0x82108 (fire / pump alternation in pump mode).
pub fn shotgun_fire(gun: &mut Gun, host: &mut dyn GunHost) {
    let s = gun.status[gun.current as usize];
    if s.state != 0 {
        // firing interrupts a shotgun reload
        gun.cur().state = 2;
        return;
    }
    let info = gun.info();
    let dopump = s.mode == mode::PUMP && s.toggle;
    gun.next_primary = if s.mode == mode::SEMI { 0.15 } else { info.rate };
    gun.idle = 2.0;
    if !(Gun::infinite_clip(host) || s.clip > 0 || dopump) {
        return;
    }
    if !dopump && !Gun::infinite_clip(host) {
        gun.cur().clip -= 1;
    }
    let seed = host.seed();
    if !dopump {
        gun.shoot(host, info.accuracy, info.damage, seed, 0.0);
        if info.kick {
            let k = if mode_mask(info) & 3 != 0 && s.mode != mode::SEMI { 0.35 } else { 1.0 };
            vkickback(k * info.view_kick, gun.kind(), false, host);
        }
    }
    let items = gun.status[gun.current as usize].items as i32;
    let pump = (s.mode == mode::PUMP) as i32;
    if dopump {
        host.event(Event::Pump, EventArgs { f1: 0.0, f2: 0.0, i1: gun.current as i32, i2: items, b1: pump, b2: 0 });
    } else {
        host.event(
            Event::Fire,
            EventArgs { f1: info.accuracy, f2: info.accuracy, i1: gun.current as i32, i2: items, b1: pump, b2: seed as i32 },
        );
    }
    let st = gun.cur();
    st.toggle = !st.toggle;
}

/// `CTSGun::AkimboFire` 0x829c0: one timer pair, the toggle picks the gun; modes 4/5 with
/// both triggers (and akimbo auto mode 0) fire a linked double shot.
pub fn akimbo_fire(gun: &mut Gun, host: &mut dyn GunHost) -> bool {
    let info = gun.info();
    let s = gun.status[gun.current as usize];
    let free_mode = s.mode == mode::AKIMBO_SEMI || s.mode == mode::AKIMBO_AUTO;
    let both = if free_mode {
        let b = if s.mode == mode::AKIMBO_AUTO { host.buttons() } else { host.pressed() };
        b & IN_ATTACK != 0 && b & IN_ATTACK2 != 0
    } else {
        false
    };
    let mut linked = both || s.mode == mode::AUTO;
    if !free_mode || linked {
        let st = gun.cur();
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
                gun.empty_sound(host);
                return false;
            }
            gun.cur().toggle = false;
        }
        if buttons & IN_ATTACK2 != 0 {
            if s.left < 1 {
                gun.empty_sound(host);
                return false;
            }
            gun.cur().toggle = true;
        }
    }
    if !Gun::infinite_clip(host) {
        let st = gun.cur();
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
        if kind_of(gun.current) == kind::AKIMBO && info.single >= 0 {
            gun.status[info.single as usize].clip = left;
        }
    }
    let silenced = gun.silenced();
    let mut spread = info.accuracy;
    if info.scale_spread {
        let akick = if linked { info.acc_kick * 2.0 } else { info.acc_kick };
        scale_spread(gun, &mut spread, akick, silenced, host);
    }
    let damage = if silenced { info.damage * 0.775 } else { info.damage };
    let seed = host.seed();
    let toggle = gun.status[gun.current as usize].toggle;
    if !linked {
        gun.shoot(host, spread, damage, seed, 0.0);
    } else {
        let off = if toggle { -5.0 } else { 5.0 };
        gun.shoot(host, spread, damage, seed, off);
        gun.shoot(host, spread, damage, seed.wrapping_add(0x21), -off);
    }
    if info.kick {
        let vk = if linked { info.view_kick * 2.0 } else { info.view_kick };
        vkickback(vk, gun.kind(), silenced, host);
    }
    let items = gun.status[gun.current as usize].items as i32;
    host.event(
        if linked { Event::FireLinked } else { Event::Fire },
        EventArgs { f1: spread, f2: spread, i1: gun.current as i32, i2: items, b1: toggle as i32, b2: seed as i32 },
    );
    let s = gun.status[gun.current as usize];
    let rate = if s.mode == mode::BURST {
        if s.state == 1 { 0.2 } else { 0.03 }
    } else if s.mode == mode::SEMI || s.mode == mode::AKIMBO_SEMI {
        0.1
    } else {
        info.rate
    };
    gun.next_secondary = rate;
    gun.next_primary = rate;
    gun.idle = 2.0;
    true
}

/// The recoil penalty's decay (0.1/s, [`recoil::decay_spread_penalty`]): the server's
/// `PreThink` (0x80d88) while alive; the client's `HUD_WeaponsPostThink` after the timers, on
/// a first prediction only, dead or alive (`0x100040a5`).
pub fn decay_spread(gun: &mut Gun, arith: Arith, dt: f32) {
    gun.spread_penalty = recoil::decay_spread_penalty(arith, gun.spread_penalty, dt);
}

/// The melee timers of `CBasePlayer::PreThink` 0x908cc before `CTSStunt::Play`, by the timer step
/// `dt` ([`Gun::timer_dt`]): the delayed-hit window (floored at 0.001) and the block timers
/// (`pl+0x79c`, `+0x7a8/+0x7b0/+0x7ac`). Server only, as [`pre_frame`].
pub fn pre_frame_timers(gun: &mut Gun, dt: f32) {
    if gun.melee_window != 0.0 {
        gun.melee_window = (gun.melee_window - dt).max(0.001);
    }
    gun.block_kungfu = (gun.block_kungfu - dt).max(0.0);
    gun.block_katana = (gun.block_katana - dt).max(0.0);
    gun.block_knife = (gun.block_knife - dt).max(0.0);
}

/// The melee parts of `CBasePlayer::PreThink` 0x908cc after `CTSStunt::Play` and before
/// `CheckForBreakables`, by the timer step `dt` ([`Gun::timer_dt`]): while alive, the kung-fu bar's
/// regeneration (15/s, [`spread::regen_melee_bar`]), then the delayed-hit window's closure (below
/// 0.01 or once its damage is spent; while it is open the kung-fu block is off). Server only:
/// `client.dll`'s `HUD_WeaponsPostThink` (`0x10003830`) has no PreThink part.
pub fn pre_frame(gun: &mut Gun, arith: Arith, dt: f32, alive: bool) {
    if alive {
        gun.melee_bar = spread::regen_melee_bar(arith, gun.melee_bar, dt);
    }
    if gun.melee_window != 0.0 {
        if gun.melee_window < 0.01 || gun.melee_damage == 0.0 {
            gun.melee_window = 0.0;
            gun.melee_damage = 0.0;
            gun.melee_source = 0;
        }
        gun.block_kungfu = 0.0;
    }
}

/// `GetBetterWeapon`'s pick (0x80f40): the first owned weapon in
/// `g_iWeaponPriority` other than `excluded`, else the list's last entry (kung fu).
pub fn better_weapon(gun: &Gun, excluded: u8) -> u8 {
    PRIORITY.iter().copied().find(|&id| gun.status[id as usize].owned && id != excluded).unwrap_or(PRIORITY[MAX_WEAPONS - 1])
}

#[cfg(test)]
mod tests;
