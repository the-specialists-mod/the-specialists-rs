//! TS 3.0's melee: the knife, the katana, kung fu and the stock hit, the `MeleeRules` answers
//! (whole functions, so a ruleset can run its own). Server addresses are `ts_i386.so`'s.

use crate::Ts30;
use shared::gameplay::MeleeRules;
use shared::pm::{Arith, Fpu, FpuMath, Num};
use shared::weapons::gun::{MeleeHit, MeleeKind, MeleeResult, Sound};
use shared::weapons::spread::{self, Vec3};
use shared::weapons::{Event, EventArgs, Gun, GunHost, kind};

const IN_ATTACK: u32 = 1;
const IN_DUCK: u32 = 4;
const IN_BACK: u32 = 16;
const IN_ATTACK2: u32 = 0x800;
const FL_ONGROUND: i32 = 0x200;
const FL_DUCKING: i32 = 0x4000;
/// `iuser4` bits that count as "stunting" for melee (`0x70c30`) and kung fu (`0x50c30`).
const STUNTING_MELEE: i32 = 0x70c30;
const STUNTING_KUNGFU: i32 = 0x50c30;

/// Every melee attack runs TS 3.0's function.
impl MeleeRules for Ts30 {
    /// `CTSGun::KnifeFire` (0x84044): [`knife_fire`].
    fn knife_fire(&self, gun: &mut Gun, host: &mut dyn GunHost) {
        knife_fire(gun, host);
    }

    /// `CTSGun::KatanaFire` (0x833f0): [`katana_fire`].
    fn katana_fire(&self, gun: &mut Gun, host: &mut dyn GunHost) {
        katana_fire(gun, host);
    }

    /// `CTSGun::CCombatAttack` (0x86078): [`ccombat_attack`].
    fn ccombat_attack(&self, gun: &mut Gun, host: &mut dyn GunHost) {
        ccombat_attack(gun, host);
    }

    /// `CTSGun::CCombatBlock` (0x85ebc): [`ccombat_block`].
    fn ccombat_block(&self, gun: &mut Gun, host: &mut dyn GunHost) {
        ccombat_block(gun, host);
    }

    /// `CTSGun::ColdCock` (0x81dd0): [`cold_cock`].
    fn cold_cock(&self, gun: &mut Gun, host: &mut dyn GunHost) {
        cold_cock(gun, host);
    }
}

/// `CTSGun::KnifeFire` 0x84044: slash (toggle off) or throw (toggle on).
pub fn knife_fire(gun: &mut Gun, host: &mut dyn GunHost) {
    if !gun.has_slot() {
        return;
    }
    let info = gun.info();
    let stunting = host.iuser4() & STUNTING_MELEE != 0;
    let rate = if stunting { info.rate * 1.5 } else { info.rate };
    gun.next_secondary = rate;
    gun.next_primary = rate;
    gun.idle = 3.0;
    host.set_close_combat_cooldown(rate);
    let thrown;
    if !gun.status[gun.current as usize].toggle {
        thrown = false;
        let r = host.random_long(0, 2);
        if !stunting && host.flags() & FL_ONGROUND != 0 {
            host.player_anim(r + 0x1d);
        }
        let mut dmg = if stunting { info.damage * 0.666_666_7 } else { info.damage };
        // Client: the damage before the melee-bar multiplier x 0.15 (0x1000a313..0x1000a327,
        // constants 0x1007a894 / 0x1007a898).
        host.view_shake(dmg * 0.15);
        dmg = spread::melee_damage(host.arith(), dmg, gun.melee_bar);
        let res = host.melee(
            &MeleeHit {
                kind: MeleeKind::Knife,
                weapon: gun.current,
                reach: 35.0,
                hull: 3,
                damage: dmg,
                bits: 0x1004,
                player_anim: -1,
                disarm: false,
                knockback: false,
                zero_roll: false,
            },
            gun,
        );
        match res {
            MeleeResult::Hit { player } => {
                if player {
                    gun.cur().state = 1;
                    gun.body = 1;
                    host.sound(Sound::KnifeHitBody);
                }
            }
            MeleeResult::Blocked { next_attack, bar_cost } => {
                gun.melee_damage = 0.0;
                gun.next_attack = next_attack;
                gun.update_melee_bar(bar_cost, host);
            }
            MeleeResult::Miss => {
                gun.melee_window = info.rate * 0.25;
                gun.melee_damage = dmg;
                gun.melee_source = 1;
            }
        }
        let st = gun.status[gun.current as usize].state;
        host.event(Event::Knife, EventArgs { i1: gun.current as i32, i2: st, ..Default::default() });
    } else {
        host.throw_blade(info.bullet);
        thrown = true;
        if host.iuser4() & 0x30c30 == 0 && host.flags() & FL_ONGROUND != 0 {
            host.player_anim(0x20);
        }
        host.sound(Sound::KnifeMiss);
    }
    if thrown {
        if !Gun::infinite_clip(host) {
            host.slots_changed(info.slots);
            let s = gun.cur();
            s.clip = s.clip.wrapping_sub(1);
        }
        if gun.status[gun.current as usize].clip < 1 {
            gun.remove_weapon(gun.current, true, host);
        } else {
            host.weapon_anim(gun.seq(20), true, gun.body);
            let items = gun.status[gun.current as usize].items as i32;
            host.event(Event::Draw, EventArgs { i1: gun.current as i32, i2: items, ..Default::default() });
            gun.idle = 2.0;
        }
    }
}

/// `CTSGun::KatanaFire` 0x833f0: four swing variants (stunt, backward, airborne, ground
/// alternating, crouched). Decoded from the disassembly (0x73580..0x737a8): each swing sets a
/// strength (`esp+0x14`, the damage before the melee-bar multiplier; negative = behind), a
/// reach (`esp+0x10`, 50 units unless stunting 35 or backward 55), the event's view-animation
/// index (`esp+0xc`), the delay (`esp+0x1c`) and the `Animate` id (`esp+0x18`).
pub fn katana_fire(gun: &mut Gun, host: &mut dyn GunHost) {
    if !gun.has_slot() {
        return;
    }
    let stunting = host.iuser4() & STUNTING_MELEE != 0;
    let speed = melee_speed(host.arith(), host.velocity());
    let (fwd, _, _) = spread::angle_vectors(host.v_angle());
    let backward = if speed <= 100.0 {
        host.buttons() & IN_BACK != 0
    } else {
        let form = if host.prediction() { BackDot::Kept } else { BackDot::Stored };
        back_dot(host.arith(), form, fwd, host.velocity()).0
    };
    // (view anim index, strength, reach, delay, player anim, block after). The crouch test
    // reads `FL_DUCKING` on the gun entity's own pev (server 0x73624, client 0x1000a052),
    // which never has it, so only `+duck` counts.
    let (view, strength, reach, delay, panim, block): (i32, f32, f32, f32, i32, bool) = if stunting {
        (0x11, 35.0, 35.0, 1.0, -1, false)
    } else if backward {
        (0xf, -75.0, 55.0, 1.0, 35, true)
    } else if host.flags() & FL_ONGROUND == 0 {
        (0x11, (speed * 0.153_846_16).clamp(35.0, 55.0), 50.0, 1.1, 36, false)
    } else if host.buttons() & IN_DUCK == 0 {
        if !(1..=2).contains(&gun.last_melee_anim) {
            gun.last_melee_anim = host.random_long(1, 2);
        }
        if gun.last_melee_anim == 1 {
            gun.last_melee_anim = 2;
            (0x11, 45.0, 50.0, 0.7, 33, false)
        } else {
            gun.last_melee_anim = 1;
            (0x10, 45.0, 50.0, 0.7, 34, false)
        }
    } else {
        host.melee_crouch(0x4 | 0x1);
        (0x11, 65.0, 50.0, 0.9, 37, true)
    };
    host.player_anim(panim);
    // Client: the strength x 0.2 (0x1000a156, constant 0x1007a8f0), before the bar.
    host.view_shake(strength * 0.2);
    let dmg = spread::melee_damage(host.arith(), strength, gun.melee_bar);
    // A negative strength swings behind with the human hull (0x7386b, 0x738cf).
    let behind = dmg < 0.0;
    let res = host.melee(
        &MeleeHit {
            kind: MeleeKind::Katana,
            weapon: gun.current,
            reach: if behind { -reach } else { reach },
            hull: if behind { 1 } else { 3 },
            damage: dmg.abs(),
            bits: 0x1080,
            player_anim: -1,
            disarm: false,
            knockback: false,
            zero_roll: false,
        },
        gun,
    );
    match res {
        MeleeResult::Hit { player } => {
            if player {
                gun.cur().state = 1;
                gun.body = 1;
                host.sound(Sound::KatanaHitBody);
            }
        }
        MeleeResult::Blocked { next_attack, bar_cost } => {
            gun.melee_damage = 0.0;
            gun.next_attack = next_attack;
            gun.update_melee_bar(bar_cost, host);
        }
        MeleeResult::Miss => {
            // The signed strength: the delayed hit looks behind for a backward swing.
            gun.melee_window = 0.25;
            gun.melee_damage = dmg;
            gun.melee_source = 2;
        }
    }
    if block && host.iuser4() & 1 == 0 {
        host.block_player(delay * 0.9);
    }
    let st = gun.status[gun.current as usize].state;
    host.event(Event::Katana, EventArgs { i1: view, i2: st, ..Default::default() });
    gun.next_secondary = delay;
    gun.next_primary = delay;
    gun.idle = 3.0;
    host.set_close_combat_cooldown(delay);
}

/// `CTSGun::CCombatAttack` 0x86078, decoded from the disassembly (0x76078..0x76e19). Each move
/// sets, in stack slots: the attack type for the event (`esp+0x28`), the move id (`esp+0x2c`:
/// the `Animate` id `CloseCombatAttack` plays and the `gun+0xa04` the kick combos chain on),
/// the event's kung-fu anim index (`esp+0x8`), the damage (`esp+0x1c`), the reach in units
/// (`esp+0x18`), the delay (`esp+0x24`: next attack, `pl+0x4f8` and, x0.75, the `BlockPlayer`
/// time) and the delayed-hit window (`esp+0x20`).
pub fn ccombat_attack(gun: &mut Gun, host: &mut dyn GunHost) {
    if gun.next_attack > 0.0 || gun.next_primary > 0.0 || gun.next_secondary > 0.0 {
        return;
    }
    let iuser4 = host.iuser4();
    let stunting = iuser4 & STUNTING_KUNGFU != 0;
    let vel = host.velocity();
    let speed = melee_speed(host.arith(), vel);
    let on_ground = host.flags() & FL_ONGROUND != 0;
    let buttons = host.buttons();
    let pressed = host.pressed();
    let mut dot = 0.0;
    let backward = if speed <= 100.0 {
        buttons & IN_BACK != 0
    } else {
        let a = host.v_angle();
        let (f, _, _) = spread::angle_vectors([0.0, a[1], a[2]]);
        let (back, stored) = back_dot(host.arith(), BackDot::Kept, f, vel);
        dot = stored;
        back
    };

    // `esp+0x14` == 3: the running back kick (reach behind, human hull, roll 0).
    let mut back = backward;
    // attack type (event iparam1): 0 none, 1 punch, 2 kick, 3 running back kick
    let mut atype = 0;
    let mut move_id = 0;
    // Event iparam2: an index into the kung-fu anim set A, which the client plays (pushed at
    // 0x76daa; the client copy picks the same, 0x1000b0b0..0x1000b4ea).
    let mut view_anim = 0;
    let mut dmg = 0.0f32;
    let mut reach = 0.0f32;
    let mut delay = 0.6f32;
    let mut window = 0.0f32;
    let mut block_after = false;
    let mut stand_fix = true;
    // The uppercut's and flying kick's crouch, made once the move goes ahead (the plugin hook
    // may block it).
    let mut crouch = None;

    let pick = |host: &mut dyn FnMut(i32, i32) -> i32, last: i32, table: &[(i32, i32)], choices: &[i32]| -> i32 {
        for (prev, next) in table {
            if last == *prev {
                return *next;
            }
        }
        choices[(host(0, choices.len() as i32 - 1)) as usize]
    };

    if stunting {
        (back, atype, move_id, view_anim, delay, window, dmg, reach) = (false, 1, -1, 4, 0.55, 0.15, 15.0, 35.0);
        gun.punch_combo = 0;
    } else if buttons & IN_ATTACK != 0 {
        (back, atype, reach) = (false, 1, 35.0);
        if !on_ground {
            (move_id, view_anim, delay, window, dmg) = (24, 5, 0.5, 0.15, 25.0);
            gun.punch_combo = 0;
        } else if gun.punch_combo > 2 || (buttons & IN_DUCK != 0 && host.flags() & FL_DUCKING == 0) {
            (move_id, view_anim, delay, window, dmg) = (25, 11, 0.65, 0.15, 30.0);
            gun.punch_combo = 0;
        } else if pressed & IN_ATTACK != 0 {
            // a fresh press: straight punch
            (move_id, view_anim, delay, window, dmg) = (23, 4, 0.35, 0.15, 20.0);
            gun.punch_combo = 0;
        } else {
            // held: fast jabs (the combo counter leads to the heavy punch)
            gun.punch_combo += 1;
            (move_id, view_anim, delay, window, dmg) = (22, 3, 0.25, 0.1, 10.0);
        }
    } else if buttons & IN_ATTACK2 != 0 {
        // The crouch test reads `FL_DUCKING` on the gun entity's own pev (server 0x76487,
        // client 0x1000b1c1), which never has it, so only `+duck` counts.
        if on_ground && buttons & IN_DUCK != 0 {
            // uppercut kick
            (back, atype, block_after, move_id, view_anim, reach, delay, window, dmg) = (false, 2, true, 20, 6, 45.0, 1.0, 0.35, 35.0);
            crouch = Some(0x4 | 0x1);
        } else if backward {
            (atype, move_id, view_anim, reach, window) = (3, 17, 7, 55.0, 0.3);
            dmg = (speed * 0.303_030_3).clamp(15.0, 80.0);
            if on_ground {
                block_after = true;
                delay = 1.0;
            } else {
                delay = 1.35;
            }
        } else {
            atype = 2;
            if !on_ground && speed > 270.0 && buttons & IN_DUCK != 0 && dot > 0.5 {
                // flying kick
                gun.kick_combo = 0;
                dmg = (speed * 0.153_846_16).clamp(15.0, 45.0);
                (reach, delay, window, move_id, view_anim) = (45.0, 1.0, 0.25, 16, 8);
                stand_fix = false;
                crouch = Some(0x4);
            } else if on_ground || speed <= 200.0 {
                block_after = true;
                reach = 45.0;
                if pressed & IN_ATTACK2 != 0 || gun.kick_combo > 2 {
                    (delay, window, dmg) = (0.5, 0.3, 25.0);
                    gun.kick_combo = 0;
                    let mut r = |lo, hi| host.random_long(lo, hi);
                    move_id = pick(&mut r, gun.last_melee_anim, &[(15, 10), (10, 14), (14, 13), (13, 15)], &[14, 13, 10, 15]);
                    view_anim = 10;
                } else {
                    gun.kick_combo += 1;
                    (delay, window, dmg) = (0.3, 0.2, 15.0);
                    let mut r = |lo, hi| host.random_long(lo, hi);
                    move_id = pick(&mut r, gun.last_melee_anim, &[(11, 12), (12, 11)], &[11, 12]);
                    view_anim = 9;
                }
            } else {
                // airborne speed kick
                gun.kick_combo = 0;
                dmg = (speed as f64 * 0.1).clamp(15.0, 30.0) as f32;
                (reach, delay, window) = (45.0, 0.75, 0.25);
                let mut r = |lo, hi| host.random_long(lo, hi);
                move_id = pick(&mut r, gun.last_melee_anim, &[(15, 19), (19, 18), (18, 15)], &[19, 18, 15]);
                view_anim = 10;
            }
        }
    }

    // Port addition (plugin API version 5): plugins see the move's delay and the damage it
    // deals (`melee_damage` and the doubling are pure, so computing them first changes nothing
    // TS does) and may change or block it.
    let base_dmg = dmg;
    let mut go = atype != 0;
    if go {
        dmg = spread::melee_damage(host.arith(), dmg, gun.melee_bar);
        if host.kungfu() {
            dmg += dmg;
        }
        go = host.melee_attack_pre(gun, &mut delay, &mut dmg);
    }
    if go {
        if let Some(bits) = crouch {
            host.melee_crouch(bits);
        }
        let iuser4 = host.iuser4();
        if stand_fix && iuser4 & 0x20000 != 0 {
            // leave a wall climb
            host.set_iuser4(iuser4 & !0x20000 & !0x4);
            host.leave_wall_climb();
        }
        gun.last_melee_anim = move_id;
        // Client: the base damage x 0.35 (0x1000b5c7, constant 0x1007a8a8); the client copy
        // picks the same per-attack values (0x1000b09f..0x1000b4e4).
        host.view_shake(base_dmg * 0.35);
        let res = host.melee(
            &MeleeHit {
                kind: MeleeKind::KungFu,
                weapon: 0,
                reach: if back { -reach } else { reach },
                hull: if back { 1 } else { 3 },
                damage: dmg,
                bits: 0x0400_1080,
                player_anim: move_id,
                disarm: true,
                knockback: true,
                zero_roll: back,
            },
            gun,
        );
        match res {
            MeleeResult::Hit { .. } => {}
            MeleeResult::Blocked { next_attack, bar_cost } => {
                gun.next_attack = next_attack;
                if bar_cost > 0.0 {
                    gun.update_melee_bar(bar_cost, host);
                }
                arm_kungfu_window(gun, dmg, window, back);
            }
            MeleeResult::Miss => arm_kungfu_window(gun, dmg, window, back),
        }
        if block_after && host.iuser4() & 1 == 0 {
            host.block_player(delay * 0.75);
        }
        host.event(Event::CloseCombat, EventArgs { f1: 0.0, f2: 0.0, i1: atype, i2: view_anim, b1: 0, b2: 0 });
    }
    gun.next_secondary = delay;
    gun.next_primary = delay;
    gun.idle = 2.0;
    gun.block_kungfu = 0.0;
    host.set_close_combat_cooldown(delay);
}

/// No target (or a blocked one): the delayed hit (0x76d02), armed while the move's window is
/// open.
fn arm_kungfu_window(gun: &mut Gun, dmg: f32, window: f32, back: bool) {
    if window > 0.001 {
        gun.melee_window = window;
        gun.melee_damage = if back { -dmg } else { dmg };
        gun.melee_source = 0;
    }
}

/// `CTSGun::CCombatBlock` 0x85ebc (reload key with kung fu or a blade). The client's
/// (`0x1000adc0`) gates on [`GunHost::kfu_power`] and charges no power.
pub fn ccombat_block(gun: &mut Gun, host: &mut dyn GunHost) {
    if gun.next_attack > 0.0 || gun.next_primary > 0.0 || gun.next_secondary > 0.0 || gun.next_alt > 0.0 {
        return;
    }
    let hud_power = host.kfu_power();
    if host.iuser4() & STUNTING_MELEE != 0 || hud_power.unwrap_or(gun.melee_bar) < 0.01 {
        return;
    }
    let i2 = if !gun.has_slot() {
        gun.block_kungfu = 0.5;
        1
    } else {
        match gun.kind() {
            kind::KATANA => gun.block_katana = 0.5,
            kind::KNIFE => gun.block_knife = 0.5,
            _ => {}
        }
        0x12
    };
    host.event(Event::CloseCombat, EventArgs { f1: gun.current as f32, f2: 0.0, i1: 0, i2, b1: 0, b2: 0 });
    host.player_anim(0x26);
    if hud_power.is_none() {
        gun.update_melee_bar(33.0, host);
    }
    gun.next_secondary = 0.5;
    gun.next_primary = 0.5;
    gun.idle = 3.0;
}

/// `CTSGun::ColdCock` 0x81dd0: stock hit, 35 u ahead.
pub fn cold_cock(gun: &mut Gun, host: &mut dyn GunHost) {
    if !gun.has_slot() || gun.info().stock == 0 {
        return;
    }
    let stunting = host.iuser4() & STUNTING_MELEE != 0;
    if gun.kind() == kind::GRENADE {
        gun.grenade_idle(true, host);
    }
    gun.stop_reload();
    let rate = if stunting { 1.087_500_1 } else { 0.725 };
    gun.unzoom(host);
    gun.next_alt = rate;
    gun.next_secondary = rate;
    gun.next_primary = rate;
    gun.next_attack = rate;
    gun.idle = 2.0;
    if !gun.has_slot() {
        return;
    }
    // Client: stock damage x 0.3 (0x10008db2, constant 0x1007a8b0), stunting or not.
    host.view_shake(gun.info().stock as f32 * 0.3);
    let mut dmg = gun.info().stock as f32;
    if stunting {
        dmg *= 0.666_666_7;
    }
    let r = host.melee(
        &MeleeHit {
            kind: MeleeKind::ColdCock,
            weapon: gun.current,
            reach: 35.0,
            hull: 3,
            damage: dmg,
            bits: 0x0400_1080,
            player_anim: if stunting { -1 } else { 0x27 },
            disarm: false,
            knockback: false,
            zero_roll: false,
        },
        gun,
    );
    // `ColdCockAttack` returns the target even when it blocks (0x9678c is only reached for no
    // target), so the hit sound (0x7204a) plays for a block too.
    match r {
        MeleeResult::Hit { .. } => host.sound(Sound::KevlarHit),
        MeleeResult::Blocked { next_attack, bar_cost } => {
            gun.melee_damage = 0.0;
            gun.next_attack = next_attack;
            if bar_cost > 0.0 {
                gun.update_melee_bar(bar_cost, host);
            }
            host.sound(Sound::KevlarHit);
        }
        MeleeResult::Miss => {}
    }
    // The server tests the record's owned flag (0x81dd0); the client copy doesn't.
    if gun.has_slot() && (gun.status[gun.current as usize].owned || !host.tracks_owned()) {
        // f1/f2 are ColdCock's own static `vecDir` (ts_i386.so 0x17da6c, mp.dll 0x100c6358,
        // client.dll 0x1009ff90), which nothing writes: always 0.
        let items = gun.status[gun.current as usize].items as i32;
        host.event(Event::ColdCock, EventArgs { i1: gun.current as i32, i2: items, ..Default::default() });
    }
}

/// The player's speed as `KatanaFire` and `CCombatAttack` keep it: `x² + y² + z²` on the build's
/// FPU, the root stored as a `float` in every build (`ts_i386.so` 0x83469 / 0x860cc, mp.dll
/// 0x1005679a / 0x10058b74, client.dll 0x10009f32 / 0x1000af4f).
fn melee_speed(arith: Arith, v: Vec3) -> f32 {
    fn go<F: Fpu>(v: Vec3) -> f32 {
        F::vec_length(v, shared::pm::ORDER_XYZ).f()
    }
    shared::by_arith!(arith, go(v))
}

/// How an MSVC build forms the melee back test's `DotProduct(forward, velocity.Normalize())`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum BackDot {
    /// mp.dll's `KatanaFire` (0x100567c0..0x10056858): every component stored, summed `z, y, x`.
    Stored,
    /// `CCombatAttack` (mp.dll 0x10058c59..0x10058c94, client.dll 0x1000b02f..0x1000b06a) and the
    /// client's `KatanaFire` (0x10009f97..0x10009fd2): the normalised x left on the FPU,
    /// `(z + y) + x`.
    Kept,
}

/// The melee back test (`<= -0.5`, the `double`, on the unrounded dot) and the dot stored as a
/// `float` (`CCombatAttack` reads it again). `ts_i386.so` normalises as usual and sums `x, y, z`
/// (0x83527.., 0x86255..); MSVC as `form` says.
fn back_dot(arith: Arith, form: BackDot, fwd: Vec3, vel: Vec3) -> (bool, f32) {
    fn go<F: Fpu>(form: BackDot, f: Vec3, vel: Vec3) -> (bool, f32) {
        use shared::pm::{ORDER_XYZ, not_above87};
        let d = if !F::MSVC {
            let n = F::vec_normalize(vel, ORDER_XYZ);
            F::x(f[0]) * F::x(n[0]) + F::x(f[1]) * F::x(n[1]) + F::x(f[2]) * F::x(n[2])
        } else if form == BackDot::Stored {
            let n = F::vec_normalize(vel, ORDER_XYZ);
            F::x(f[2]) * F::x(n[2]) + F::x(f[1]) * F::x(n[1]) + F::x(f[0]) * F::x(n[0])
        } else {
            let [nx, ny, nz] = F::vec_normalize_n(vel, ORDER_XYZ);
            (F::x(f[2]) * F::x(nz.f()) + F::x(f[1]) * F::x(ny.f())) + F::x(f[0]) * nx
        };
        (not_above87(d, F::xd(-0.5)), d.f())
    }
    shared::by_arith!(arith, go(form, fwd, vel))
}

#[cfg(test)]
mod tests;
