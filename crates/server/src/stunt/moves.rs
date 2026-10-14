//! `CTSStunt` (decomp `server/decomp/CTSStunt.c`, 0x99fbc-0x9d3ec) and the stunt parts of
//! `CBasePlayer::Jump`/`Duck`/`PostThink`. The functions keep the TS names.

//!
//! The ruleset's hooks (`r`, resolved by the entry points in `super`) decide the values
//! ([`rules::StuntRules`]), how the timers keep time ([`rules::StuntClock`]: they are only
//! stored from `start` and asked through `running`/`done`) and the slot limits (`can_dive`).

use super::rules::Dive;
use super::*;
use crate::damage::{DamageInfo, bits};
use goldsrc::edict::EdictPtr;
use goldsrc::server::{DONT_IGNORE_MONSTERS, HEAD_HULL, IGNORE_MONSTERS};

/// `CanDiveHigh/Medium/Low` (0x9a494...), the ruleset's ([`rules::can_dive`] in TS 3.0).
fn can(p: &Player, cx: &mut Cx, r: Rules, dive: Dive) -> bool {
    r.can_dive(p, cx.pev().armorvalue, dive)
}

/// A stunt timer of `secs` seconds of the player's time, starting now.
fn timer(cx: &mut Cx, r: Rules, secs: f64) -> f32 {
    r.stunt_clock().start(cx.time(), cx.pev().fuser1, secs)
}

/// `CTSStunt::NextDive`: no cooldown after dives and flips.
const NEXT_DIVE: f64 = 0.0;

fn forward(cx: &mut Cx) -> Vec3 {
    shared::pm::angle_vectors(crate::rules::cvars::pm_arith(), cx.pev().v_angle).0
}

fn hspeed(v: Vec3) -> f32 {
    (v[1] * v[1] + v[0] * v[0]).sqrt()
}

fn set_view(cx: &mut Cx, z: f32) {
    cx.pev().view_ofs = [0.0, 0.0, z];
}

/// `CTSStunt::Play` (0x9a014), every PreThink.
pub(super) fn play(p: &mut Player, cx: &mut Cx, r: Rules) {
    let (clock, rules) = (r.stunt_clock(), r.stunts());
    let dt = slow_mul(cx) * frametime();
    clock.tick(&mut p.stunt, dt);
    let now = cx.time();

    if !is_alive(cx) || cx.pev().iuser1 != 0 {
        return;
    }
    if cx.pev().iuser4 & SUPERJUMP_START != 0 {
        cx.pev().iuser4 |= SUPERJUMP_GLIDE;
        cx.pev().iuser4 &= !SUPERJUMP_START;
        anim::animate(p, cx, 0x39);
        stunt_sound(cx, c"player/airwheel.wav");
    }
    if on_ground(cx) {
        p.stunt.wall_chain = 0.0;
        p.stunt.climb_tap = 0.0;
        p.stunt.wall_climbs = 0;
    }

    if !on_ladder(cx) && cx.pev().waterlevel != 3 {
        let s = &mut p.stunt;
        if s.recover != 0.0 && (clock.done(now, s.recover) || s.anim.seq_finished) {
            let pev = cx.pev();
            pev.iuser4 &= !rules.recover_end_clears;
            s.recover = 0.0;
            if rules.recover_end_unblocks {
                s.block_timer = 0.0;
            }
            pev.friction = 1.0;
            let z = if pev.flags & FL_DUCKING as i32 != 0 { shared::pm::VIEW_DUCK } else { shared::pm::VIEW_STAND };
            pev.view_ofs = [0.0, 0.0, z];
        }
        if cx.pev().iuser4 & DIVING != 0 {
            handle_dive(p, cx, r);
        }
        if cx.pev().iuser4 & PRONE != 0 {
            handle_slide(p, cx, r);
        }
        if cx.pev().iuser4 & (FLIP_HIGH | FLIP_LOW) != 0 {
            handle_wheel(p, cx, r);
        }
        if cx.pev().iuser4 & WALLJUMPING != 0 {
            handle_wall_jump(p, cx, r);
        }
        if cx.pev().iuser4 & WALLCLIMBING != 0 {
            handle_wall_climb(p, cx, r);
        }
        if cx.pev().button & (IN_JUMP as i32 | IN_ALT1) != 0 {
            handle_stunt(p, cx, r);
        }
    } else {
        // Ladder or under water: drop every stunt.
        cx.pev().iuser4 &= !SUPERJUMP_GLIDE;
        if cx.pev().iuser4 & 0x70c31 != 0 {
            let pev = cx.pev();
            pev.iuser4 &= !(ROLLING | GO_DUCK | RECOVER | DIVING | PRONE | FLIP_HIGH | FLIP_LOW | WALLJUMPING | WALLCLIMBING);
            pev.view_ofs = [0.0, 0.0, shared::pm::VIEW_STAND];
            pev.friction = 1.0;
            pev.gravity = 1.0;
            p.stunt.recover = 0.0;
            if rules.recover_end_unblocks {
                p.stunt.block_timer = 0.0;
            }
            p.stunt.anim.current = -1;
            anim::animate(p, cx, 1);
        }
    }
}

/// `CTSStunt::HandleStunt` (0x9b000): the stunt key (or jump) was used.
fn handle_stunt(p: &mut Player, cx: &mut Cx, r: Rules) {
    let (clock, rules, now) = (r.stunt_clock(), r.stunts(), cx.time());
    if clock.running(now, p.stunt.next_stunt) || clock.running(now, p.stunt.recover) {
        return;
    }
    if cx.pev().iuser4 & (PRONE | DIVING | FLIP_HIGH | FLIP_LOW) != 0 {
        return;
    }
    if rules.stunt_ends_close_combat {
        let g = &mut p.inv.gun;
        g.melee_window = 0.0;
        g.melee_damage = 0.0;
        g.melee_source = 0;
    }

    let alt_pressed = p.stunt.button_pressed & IN_ALT1 != 0;
    let gliding = cx.pev().iuser4 & SUPERJUMP_GLIDE != 0;
    if cx.pev().button & IN_JUMP as i32 != 0 && (rules.wall_climb_while_gliding || !gliding) {
        if check_for_wall_climb(p, cx) {
            go_wall_climb(p, cx, r);
            return;
        }
    } else if alt_pressed {
        if check_for_wall_jump(p, cx, r) {
            go_wall_jump(p, cx, r);
            return;
        }
        if rules.wall_dive && check_for_wall_dive(cx) {
            cx.pev().iuser4 |= WALLDIVE;
            go_dive(p, cx, r);
            return;
        }
    }

    if !alt_pressed {
        return;
    }
    if cx.pev().iuser4 & (SUPERJUMP_GLIDE | WALLJUMPING | WALLCLIMBING) != 0 {
        return;
    }
    if !on_ground(cx) {
        if cx.pev().velocity[2] < -680.0 {
            return;
        }
        // Coyote time: 0.7 s (slowed) after leaving the ground.
        if !clock.air_stunt_allowed(now, cx.pev().fuser1, p.stunt.last_ground_time) {
            return;
        }
    }
    if hspeed(cx.pev().velocity) < 80.0 {
        go_bullet_time(p, cx, r);
    } else {
        go_dive(p, cx, r);
    }
}

/// A wall within `dist` units along `dir` from the origin: |normal.z| < 0.5 and not falling
/// faster than 370 u/s.
fn wall_check(cx: &mut Cx, dir: Vec3, dist: f32, no_monsters: i32) -> Option<TraceResult> {
    let o = cx.pev().origin;
    let end = [o[0] + dir[0] * dist, o[1] + dir[1] * dist, o[2] + dir[2] * dist];
    let tr = engine().trace_line(o, end, no_monsters, cx.edict);
    let n = tr.vecPlaneNormal[2];
    (tr.flFraction < 1.0 && n < 0.5 && n > -0.5 && cx.pev().velocity[2] > -370.0).then_some(tr)
}

/// `CTSStunt::CheckForWallJump` (0x9ac14): facing a wall within 48 units.
fn check_for_wall_jump(p: &Player, cx: &mut Cx, r: Rules) -> bool {
    if !can(p, cx, r, Dive::High) {
        return false;
    }
    let f = forward(cx);
    wall_check(cx, f, 48.0, IGNORE_MONSTERS).is_some()
}

/// `CTSStunt::CheckForWallDive` (0x9ad44): in the air with a wall 48 units behind.
fn check_for_wall_dive(cx: &mut Cx) -> bool {
    if on_ground(cx) {
        return false;
    }
    let f = forward(cx);
    wall_check(cx, [-f[0], -f[1], -f[2]], 48.0, IGNORE_MONSTERS).is_some()
}

/// `CTSStunt::CheckForWallClimb` (0x9ae6c): jump double-tap (unless already climbing) facing a
/// non-player wall within 54 units.
fn check_for_wall_climb(p: &mut Player, cx: &mut Cx) -> bool {
    if cx.pev().iuser4 & WALLCLIMBING == 0 {
        if p.stunt.button_pressed & IN_JUMP as i32 == 0 {
            return false;
        }
        let time = cx.time();
        if p.stunt.climb_tap < time {
            p.stunt.climb_tap = time + 0.3;
            return false;
        }
    }
    let f = forward(cx);
    let o = cx.pev().origin;
    let end = [o[0] + f[0] * 54.0, o[1] + f[1] * 54.0, o[2] + f[2] * 54.0];
    let tr = engine().trace_line(o, end, DONT_IGNORE_MONSTERS, cx.edict);
    if tr.flFraction >= 1.0 {
        return false;
    }
    if is_player(tr.pHit) {
        return false;
    }
    let n = tr.vecPlaneNormal[2];
    n < 0.5 && n > -0.5 && cx.pev().velocity[2] > -370.0
}

/// `IsPlayer()` (0x9af9e, 0x9c075): any `CBasePlayer`.
fn is_player(e: *mut edict_t) -> bool {
    crate::rules::has_player(e)
}

/// Common start of every stunt: the hull drops to duck through `GO_DUCK`, the view to 0.
fn stunt_dir_back(p: &mut Player, cx: &mut Cx) {
    let f = forward(cx);
    p.stunt.dir = [-f[0], -f[1], -f[2]];
}

/// `CTSStunt::GoBulletTime` (0x9a5b4): stunt key standing still, only in slow motion.
fn go_bullet_time(p: &mut Player, cx: &mut Cx, r: Rules) {
    if !r.bullet_time_allowed(cx.pev().fuser1) {
        return;
    }
    let rules = r.stunts();
    stunt_dir_back(p, cx);
    p.stunt.dir_not_forward = true;
    p.stunt.next_stunt = timer(cx, r, 0.55);
    if let Some(d) = rules.bullet_time_dodge {
        p.stunt.dodge_until = cx.time() + d;
    }
    send_next_stunt(p, cx, r);
    cx.pev().iuser4 |= GO_DUCK | PRONE;
    p.stunt.anim.last_dir = 3;
    p.stunt.prone_lock = timer(cx, r, 0.55);
    set_view(cx, 0.0);
    if let Some(d) = rules.bullet_time_attack_delay
        && p.inv.gun.next_attack < 0.0
    {
        p.inv.gun.next_attack = r.stunt_clock().attack_delay(cx.pev().fuser1, d);
    }
    anim::animate(p, cx, 0x36);
    stunt_sound(cx, c"player/land.wav");
    if r.motion_blur() {
        motion_blur(cx, 2);
    }
}

/// The wall chain's `m_flWallChain += add`: every TS build adds a `double` constant to the
/// `float` on the x87 and stores the sum as a `float` (`ts_i386.so` GoWallJump 0x8a8ca, mp.dll
/// 0x10068c9b / GoWallClimb 0x10068ee6.., 2.1 mp.dll 0x1005b42a / 0x1005b67b..), under the
/// server's arithmetic (`arith`, [`crate::rules::cvars::pm_arith`]). Returns the sum as the x87 holds it
/// before the store (53 bits under `Msvc`); `GoWallJump` hands that on to the cooldown in the
/// MSVC builds, `ts_i386.so` reloads the stored `float`.
fn wall_chain_add(arith: shared::pm::Arith, chain: &mut f32, add: f64) -> f64 {
    use shared::pm::{Arith, Fpu, Gcc, Msvc, Num};
    match arith {
        Arith::Msvc => {
            let sum = Msvc::xd(add) + Msvc::x(*chain);
            *chain = sum.f();
            sum
        }
        Arith::Gcc => {
            *chain = (Gcc::xd(add) + Gcc::x(*chain)).f();
            f64::from(*chain)
        }
    }
}

/// `CTSStunt::GoWallJump` (0x9a7d4): kick off the wall in front.
fn go_wall_jump(p: &mut Player, cx: &mut Cx, r: Rules) {
    stunt_dir_back(p, cx);
    let pev = cx.pev();
    pev.iuser4 &= !SUPERJUMP_GLIDE;
    pev.iuser4 |= GO_DUCK | WALLJUMP_IMPULSE | WALLJUMPING;
    pev.iuser4 &= !WALLCLIMBING;
    pev.view_ofs = [0.0; 3];
    anim::animate(p, cx, 0x37);
    cx.pev().gravity = 0.75;
    let sum = wall_chain_add(crate::rules::cvars::pm_arith(), &mut p.stunt.wall_chain, 0.35);
    p.stunt.next_stunt = r.stunt_clock().wall_chain_cooldown(cx.time(), cx.pev().fuser1, sum);
    send_next_stunt(p, cx, r);
    stunt_sound(cx, c"player/airwheel.wav");
    maybe_blur(cx, r, 5.0);
}

/// `CTSStunt::GoWallClimb` (0x9aa1c): run up the wall in front.
fn go_wall_climb(p: &mut Player, cx: &mut Cx, r: Rules) {
    stunt_dir_back(p, cx);
    let pev = cx.pev();
    pev.iuser4 &= !SUPERJUMP_GLIDE;
    pev.iuser4 |= GO_DUCK | WALLCLIMB_IMPULSE | WALLCLIMBING;
    pev.iuser4 &= !WALLJUMPING;
    pev.view_ofs = [0.0; 3];
    anim::animate(p, cx, 0x38);
    cx.pev().gravity = 1.0;
    let add = if can(p, cx, r, Dive::High) {
        0.3
    } else if can(p, cx, r, Dive::Medium) {
        0.5
    } else {
        0.75
    };
    // Every build stores the sum and loads the float back for the cooldown.
    wall_chain_add(crate::rules::cvars::pm_arith(), &mut p.stunt.wall_chain, add);
    p.stunt.next_stunt = r.stunt_clock().wall_chain_cooldown(cx.time(), cx.pev().fuser1, f64::from(p.stunt.wall_chain));
    send_next_stunt(p, cx, r);
    stunt_sound(cx, c"player/airwheel.wav");
    p.stunt.wall_climbs += 1;
}

/// `CTSStunt::GoDive` (0x9b1a0): dive height by pitch and slot space.
fn go_dive(p: &mut Player, cx: &mut Cx, r: Rules) {
    let pitch = cx.pev().v_angle[0];
    if pitch < -10.0 && can(p, cx, r, Dive::High) {
        cx.pev().iuser4 |= DIVE_HIGH;
        cx.pev().gravity = 0.75;
    } else if pitch <= 10.0 && can(p, cx, r, Dive::Medium) {
        cx.pev().iuser4 |= DIVE_MED;
        cx.pev().gravity = 0.75;
    } else {
        cx.pev().iuser4 |= DIVE_LOW;
        if can(p, cx, r, Dive::Low) {
            cx.pev().friction = 0.25;
            cx.pev().gravity = 0.75;
        } else {
            // Too heavy: a stumble.
            cx.pev().friction = 0.9;
            cx.pev().gravity = 1.0;
        }
    }
    let pev = cx.pev();
    pev.iuser4 &= !(SUPERJUMP_GLIDE | WALLCLIMB_IMPULSE | WALLCLIMBING | WALLJUMPING);
    pev.iuser4 |= GO_DUCK | DIVING;
    pev.view_ofs = [0.0; 3];
    anim::animate(p, cx, 0x28);
    p.stunt.next_stunt = timer(cx, r, NEXT_DIVE);
    send_next_stunt(p, cx, r);
    maybe_blur(cx, r, 8.0);
    p.stunt.dir = if cx.pev().iuser4 & WALLDIVE == 0 { normalized(cx.pev().velocity) } else { forward(cx) };
    stunt_sound(cx, c"player/dive.wav");
}

/// Landing from a wall jump/climb: `CTSStunt::HandleWallJump` (0x9b4fc) and the ground half of
/// `HandleWallClimb` (0x9b644).
fn land_wall_stunt(p: &mut Player, cx: &mut Cx, r: Rules, bit: i32) {
    let rules = r.stunts();
    if cx.pev().iuser4 & bit != 0 {
        cx.pev().iuser4 &= !bit;
        if rules.wall_landing_sets_recover {
            cx.pev().iuser4 |= RECOVER;
        }
        cx.pev().friction = 0.5;
        set_view(cx, 0.0);
        p.stunt.recover = timer(cx, r, rules.wall_landing_recover);
        if r.stunt_clock().done(cx.time(), p.stunt.next_stunt) {
            p.stunt.next_stunt = timer(cx, r, 0.0);
            send_next_stunt(p, cx, r);
        }
    }
    cx.pev().gravity = 1.0;
    stunt_sound(cx, c"player/land.wav");
}

fn handle_wall_jump(p: &mut Player, cx: &mut Cx, r: Rules) {
    if on_ground(cx) {
        land_wall_stunt(p, cx, r, WALLJUMPING);
    }
}

fn handle_wall_climb(p: &mut Player, cx: &mut Cx, r: Rules) {
    if !on_ground(cx) {
        // Climbing and the stunt key: kick off, or dive away from the wall.
        if p.stunt.button_pressed & IN_ALT1 != 0 {
            if check_for_wall_jump(p, cx, r) {
                go_wall_jump(p, cx, r);
            } else if r.stunts().wall_dive && check_for_wall_dive(cx) {
                cx.pev().iuser4 |= WALLDIVE;
                go_dive(p, cx, r);
            }
        }
    } else {
        land_wall_stunt(p, cx, r, WALLCLIMBING);
    }
}

/// `CTSStunt::HandleDive` (0x9b7e4).
fn handle_dive(p: &mut Player, cx: &mut Cx, r: Rules) {
    landing(p, cx, r);
    anim::animate(p, cx, 2);
}

/// `CTSStunt::Landing` (0x9bbf4): land a dive into a slide, or roll out if a key is held.
fn landing(p: &mut Player, cx: &mut Cx, r: Rules) {
    if !on_ground(cx) || cx.pev().iuser4 & DIVING == 0 {
        return;
    }
    let rules = r.stunts();
    if cx.pev().button & rules.get_up_buttons == 0 {
        cx.pev().iuser4 &= !DIVING;
        cx.pev().iuser4 |= PRONE;
        set_view(cx, 0.0);
        let d = get_direction(p.stunt.dir, cx.pev().v_angle, 0.6, 0.3);
        p.stunt.dir_not_forward = d != 2;
        p.stunt.anim.last_dir = d;
    } else {
        cx.pev().iuser4 &= !DIVING;
        cx.pev().iuser4 |= ROLLING | RECOVER;
        cx.pev().friction = 0.5;
        set_view(cx, 0.0);
        p.stunt.recover = timer(cx, r, rules.dive_roll_out_recover);
        anim::animate(p, cx, 0x2e);
    }
    cx.pev().gravity = 1.0;
    stunt_sound(cx, c"player/land.wav");
}

/// `CTSStunt::HandleWheel` (0x9b820): cartwheels end prone, flips end in a short recovery.
fn handle_wheel(p: &mut Player, cx: &mut Cx, r: Rules) {
    if !on_ground(cx) {
        return;
    }
    if cx.pev().iuser4 & FLIP_HIGH == 0 {
        cx.pev().iuser4 &= !FLIP_LOW;
        cx.pev().iuser4 |= PRONE;
        set_view(cx, 0.0);
        let d0 = p.stunt.dir;
        p.stunt.dir = [-d0[0], -d0[1], -d0[2]];
        let d = get_direction(p.stunt.dir, cx.pev().v_angle, 0.6, 0.3);
        p.stunt.dir_not_forward = d != 2;
        p.stunt.anim.last_dir = d;
    } else {
        cx.pev().iuser4 &= !FLIP_HIGH;
        cx.pev().iuser4 |= RECOVER;
        cx.pev().friction = 0.25;
        set_view(cx, 0.0);
        p.stunt.recover = timer(cx, r, r.stunts().flip_recover);
    }
    cx.pev().gravity = 1.0;
    stunt_sound(cx, c"player/land.wav");
}

/// `CTSStunt::HandleSlide` (0x9b9f8): lying after a dive. Back brakes, jump/duck/stunt gets up,
/// strafe rolls sideways.
fn handle_slide(p: &mut Player, cx: &mut Cx, r: Rules) {
    if !on_ground(cx) {
        return;
    }
    let rules = r.stunts();
    let button = cx.pev().button;
    if button & IN_BACK as i32 != 0 {
        cx.pev().friction = 1.0;
    }
    let free = r.stunt_clock().done(cx.time(), p.stunt.prone_lock);
    if button & rules.get_up_buttons != 0 && free {
        cx.pev().iuser4 |= RECOVER;
        cx.pev().iuser4 &= !PRONE;
        set_view(cx, shared::pm::VIEW_STAND);
        p.stunt.recover = timer(cx, r, rules.get_up_recover);
        p.stunt.anim.seq_finished = true;
        anim::animate(p, cx, 0x29);
        cx.pev().friction = 1.0;
    } else if button & IN_MOVELEFT as i32 != 0 && free {
        side_roll(p, cx, 0x2a);
    } else if button & IN_MOVERIGHT as i32 != 0 && free {
        side_roll(p, cx, 0x2b);
    } else {
        anim::animate(p, cx, 3);
    }
}

fn side_roll(p: &mut Player, cx: &mut Cx, id: i32) {
    anim::animate(p, cx, id);
    p.stunt.dir = forward(cx);
    cx.pev().friction = 1.0;
    p.stunt.dir_not_forward = false;
    p.stunt.anim.last_dir = 2;
}

/// `CTSStunt::GoWheel` (0x9c9bc): second jump press within 0.3 s. A back flip (or any flip
/// looking up) needs high-dive space; otherwise a cartwheel.
fn go_wheel(p: &mut Player, cx: &mut Cx, r: Rules) -> bool {
    let time = cx.time();
    let clock = r.stunt_clock();
    let s = &p.stunt;
    let mut ok = (!r.stunts().wheel_and_roll_wait_for_kick || !clock.running(time, s.close_attack_cd))
        && !on_ladder(cx)
        && !clock.running(time, s.recover)
        && cx.pev().iuser4 & 0x130c30 == 0
        && time <= s.wheel_tap
        && !clock.running(time, s.next_stunt);
    if !on_ground(cx) && !clock.air_stunt_allowed(time, cx.pev().fuser1, p.stunt.last_ground_time) {
        ok = false;
    }
    if ok {
        let v = cx.pev().velocity;
        let dirn = if hspeed(v) >= 80.0 {
            p.stunt.dir = normalized(v);
            get_direction(p.stunt.dir, cx.pev().v_angle, 0.6, 0.3)
        } else {
            stunt_dir_back(p, cx);
            3
        };
        maybe_blur(cx, r, 7.0);
        if can(p, cx, r, Dive::High) && (dirn == 3 || cx.pev().v_angle[0] < -10.0) {
            cx.pev().iuser4 |= GO_DUCK | FLIP_HIGH;
            set_view(cx, 0.0);
            anim::animate(p, cx, 0x31);
            cx.pev().iuser4 |= DIVE_MED;
            cx.pev().gravity = 0.75;
            p.stunt.next_stunt = timer(cx, r, NEXT_DIVE);
            send_next_stunt(p, cx, r);
            stunt_sound(cx, c"player/airwheel.wav");
            p.stunt.wheel_tap = 0.0;
            if dirn == 3 {
                // Backflip kick: costs 22.5 kung-fu power and arms a delayed hit (15, 30 with
                // Kung Fu) in the gun's close-combat window.
                crate::gameplay::active().server.backflip_kick(p, cx);
            }
            return true;
        }
        if can(p, cx, r, Dive::Low) && dirn != 3 {
            cx.pev().iuser4 |= GO_DUCK | FLIP_LOW;
            set_view(cx, 0.0);
            anim::animate(p, cx, 0x32);
            cx.pev().iuser4 |= DIVE_LOW;
            cx.pev().gravity = 0.75;
            if dirn == 2 {
                cx.pev().friction = 0.25;
            }
            p.stunt.next_stunt = timer(cx, r, NEXT_DIVE);
            send_next_stunt(p, cx, r);
            stunt_sound(cx, c"player/airwheel.wav");
            p.stunt.wheel_tap = 0.0;
            return true;
        }
    }
    p.stunt.wheel_tap = time + 0.3;
    false
}

/// `CTSStunt::GoRoll` (0x9cfe8): duck double-tap while moving at 80+ u/s.
fn go_roll(p: &mut Player, cx: &mut Cx, r: Rules) -> bool {
    let time = cx.time();
    let clock = r.stunt_clock();
    let mut roll = true;
    if !on_ground(cx) {
        if p.stunt.roll_tap < time {
            return false;
        }
    } else if p.stunt.roll_tap < time {
        roll = false;
    }
    if hspeed(cx.pev().velocity) < 80.0
        || cx.pev().iuser4 & 0x130c30 != 0
        || clock.running(time, p.stunt.recover)
        || on_ladder(cx)
        || (r.stunts().wheel_and_roll_wait_for_kick && clock.running(time, p.stunt.close_attack_cd))
    {
        roll = false;
    }
    if !roll {
        p.stunt.roll_tap = time + 0.3;
        return false;
    }
    p.stunt.dir = normalized(cx.pev().velocity);
    let pev = cx.pev();
    pev.friction = 0.3;
    pev.iuser4 &= !SUPERJUMP_GLIDE;
    pev.iuser4 |= ROLLING | GO_DUCK | RECOVER;
    pev.view_ofs = [0.0; 3];
    p.stunt.recover = timer(cx, r, r.stunts().roll_recover);
    anim::animate(p, cx, 0x2f);
    if cx.pev().iuser4 & FREEZE_ANGLES == 0 {
        stunt_sound(cx, c"player/land.wav");
    } else {
        // Dead in TS (nothing sets FREEZE_ANGLES), ported as is: the ground's impact sound.
        let pev = cx.pev();
        let src = pev.origin;
        let end = [src[0], src[1], src[2] - 4000.0];
        let tr = engine().trace_line(src, end, IGNORE_MONSTERS | 0x100, cx.edict);
        texture_type_play_sound(&tr, src, end);
    }
    p.stunt.roll_tap = 0.0;
    maybe_blur(cx, r, 9.0);
    true
}

/// `TEXTURETYPE_PlaySound(ptr, vecSrc, vecEnd, BULLET_NONE)` (0xe1330): the impact sound of
/// the material `vecSrc -> vecEnd` hits, from the game's own table
/// (`shared::pm::textures::texturetype`). Returns the decal volume, which `GoRoll` ignores.
///
/// TS first asks the hit entity's `Classify()` and plays flesh sounds for anything but
/// `CLASS_NONE`/`CLASS_MACHINE`. The port has no `Classify`; the only caller traces with
/// `ignore_monsters`, which only hits `SOLID_BSP` entities, and every TS class that can be
/// one classifies as `CLASS_NONE` (0) or `CLASS_MACHINE` (`func_guntarget`), so the
/// texture branch is the one TS takes.
fn texture_type_play_sound(tr: &TraceResult, src: Vec3, end: Vec3) -> f32 {
    use shared::pm::textures::texturetype;
    let eng = engine();
    let world = eng.entity_of_index(0);
    let hit = if tr.pHit.is_null() { world } else { tr.pHit };
    let entity = (!hit.is_null() && hit.has_private_data()).then_some(hit);
    let name = eng.trace_texture(entity.unwrap_or(world), src, end).to_bytes();
    let mut name = name;
    if matches!(name.first(), Some(b'-' | b'+')) {
        name = name.get(2..).unwrap_or(&[]);
    }
    if matches!(name.first(), Some(b'{' | b'!' | b'~' | b' ')) {
        name = &name[1..];
    }
    let chtype = texturetype::find(&name[..name.len().min(15)]);
    let mut attn = 0.8f32;
    let (mut vol, mut volbar, samples): (f32, f32, &[&CStr]) = match chtype {
        b'D' => (0.9, 0.1, &[c"player/pl_dirt1.wav", c"player/pl_dirt2.wav", c"player/pl_dirt3.wav"]),
        b'F' => {
            attn = 1.0;
            (1.0, 0.2, &[c"weapons/bullet_hit1.wav", c"weapons/bullet_hit2.wav"])
        }
        b'G' => (0.9, 0.5, &[c"player/pl_grate1.wav", c"player/pl_grate4.wav"]),
        b'M' => (0.9, 0.3, &[c"player/pl_metal1.wav", c"player/pl_metal2.wav"]),
        b'P' | b'Y' => (0.8, 0.2, &[c"debris/glass1.wav", c"debris/glass2.wav", c"debris/glass3.wav"]),
        b'S' => (0.9, 0.0, &[c"player/pl_slosh1.wav", c"player/pl_slosh3.wav", c"player/pl_slosh2.wav", c"player/pl_slosh4.wav"]),
        b'T' => (0.8, 0.2, &[c"player/pl_tile1.wav", c"player/pl_tile3.wav", c"player/pl_tile2.wav", c"player/pl_tile4.wav"]),
        b'V' => (0.5, 0.3, &[c"player/pl_duct1.wav", c"player/pl_duct1.wav"]),
        b'W' => (0.9, 0.2, &[c"debris/wood1.wav", c"debris/wood2.wav", c"debris/wood3.wav"]),
        _ => (0.9, 0.6, &[c"player/pl_step1.wav", c"player/pl_step2.wav"]),
    };
    let breakable = entity.is_some_and(|e| eng.string(e.v().classname).to_bytes() == b"func_breakable");
    if breakable {
        // `fvol /= 1.5` as `fvol * (2.0 / 3)`, a `double`; `fvolbar /= 2`.
        vol = (vol as f64 * (2.0 / 3.0)) as f32;
        volbar *= 0.5;
    } else if chtype == b'P' && tr.flFraction != 1.0 && eng.random_long(0, 1) != 0 {
        crate::entities::stock::explode::sparks(tr.vecEndPos);
        let flvolume = eng.random_float(0.7, 1.0);
        let spark = match eng.random_long(0, 1) {
            0 => Some(c"buttons/spark5.wav"),
            1 => Some(c"buttons/spark6.wav"),
            _ => None,
        };
        if let Some(spark) = spark {
            crate::entities::stock::sentences::emit_ambient(world, tr.vecEndPos, spark, flvolume, 0.8, 0, 100);
        }
    }
    let pitch = 96 + eng.random_long(0, 0xf);
    let sample = samples[eng.random_long(0, samples.len() as i32 - 1).clamp(0, samples.len() as i32 - 1) as usize];
    crate::entities::stock::sentences::emit_ambient(world, tr.vecEndPos, sample, vol, attn, 0, pitch);
    volbar
}

/// `CloseAttack`'s gate (0x9c630..0x9c6b5): no recovery lock (`+0x04`) or close-combat cooldown
/// (`+0x08`) running, no stunt bits `0x30c30`, not ducking (`FL_DUCKING`), and not bare-handed:
/// with the TS gun active (`m_pActiveItem`, `m_iId == 16`) its weapon record (`gun+0xcc`) must be
/// set, which it isn't on kung fu (or holstered, both id 0). No active item at all (a
/// `cycler_weapon` has id 0 too) passes.
/// `recovering` / `cooling` are the two timers running ([`rules::StuntClock::running`]).
fn close_attack_allowed(recovering: bool, cooling: bool, iuser4: i32, flags: i32, gun_active: bool, weapon: u8) -> bool {
    !(recovering || iuser4 & 0x30c30 != 0 || flags & FL_DUCKING as i32 != 0 || cooling || (gun_active && weapon == 0))
}

/// The jump kick's third-person move for `RandomLong(0, 2)` (0x8c8df): 14, 13 or 10.
fn close_attack_anim(r: i32) -> i32 {
    match r {
        1 => 14,
        2 => 13,
        _ => 10,
    }
}

/// A step of the jump kick after its gate, in the order a ruleset takes them ([`kick_steps`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum KickStep {
    /// `CloseCombatAttack`: a miss ends the kick there.
    Hit,
    /// `RandomLong(0, 2)` for the move ([`close_attack_anim`]).
    Draw,
    /// The 0.7 s cooldown (`+0x08`).
    Arm,
}

/// TS 3.0 tries the kick first and only draws the move and arms the cooldown on a hit (0x9c630).
/// With [`StuntRules::kick_cooldown_on_try`](super::rules::StuntRules::kick_cooldown_on_try) the
/// cooldown is armed and the move drawn before the try, so a miss also waits out the cooldown.
fn kick_steps(cooldown_on_try: bool) -> [KickStep; 3] {
    if cooldown_on_try { [KickStep::Arm, KickStep::Draw, KickStep::Hit] } else { [KickStep::Hit, KickStep::Draw, KickStep::Arm] }
}

/// `CTSStunt::CloseAttack` (0x9c630; `mp.dll` 0x1006a740), the jump kick: a kung-fu kick 48 u
/// ahead of the eyes ([`crate::weapons::jump_kick`]) as strong as the speed allows
/// ([`shared::weapons::spread::jump_kick_strength`], 10..20, doubled with Kung Fu). On a hit:
/// move 10/13/14, a 0.7 s close-combat cooldown, `m_flNextAttack` 0.35 unless it is still
/// running, `closecombat.wav` and `UpdateMeleeBar(strength * 1.5)`; then `Jump` stops there.
fn close_attack(p: &mut Player, cx: &mut Cx, r: Rules) -> bool {
    let (clock, now) = (r.stunt_clock(), cx.time());
    let pev = cx.pev();
    let (iuser4, flags) = (pev.iuser4, pev.flags);
    let gun_active = crate::weapons::gun_active(p);
    let (recovering, cooling) = (clock.running(now, p.stunt.recover), clock.running(now, p.stunt.close_attack_cd));
    if !close_attack_allowed(recovering, cooling, iuser4, flags, gun_active, p.inv.gun.current) {
        return false;
    }
    let strength = shared::weapons::spread::jump_kick_strength(crate::rules::cvars::pm_arith(), cx.pev().velocity);
    let damage = if p.kungfu { strength + strength } else { strength };
    let mut anim = 0;
    for step in kick_steps(r.stunts().kick_cooldown_on_try) {
        match step {
            KickStep::Hit => {
                if !crate::weapons::jump_kick(p, cx, damage) {
                    return false;
                }
            }
            KickStep::Draw => anim = close_attack_anim(engine().random_long(0, 2)),
            KickStep::Arm => p.stunt.close_attack_cd = timer(cx, r, 0.7),
        }
    }
    anim::animate(p, cx, anim);
    stunt_sound(cx, c"player/closecombat.wav");
    crate::gameplay::active().server.jump_kick_hit(p, cx, strength);
    true
}

/// The end of a jump kick that hit (`CTSStunt::CloseAttack` 0x9c630): `UTIL_WeaponTimeBase()` is
/// 0, so only a run-out `m_flNextAttack` (it rests at -0.001) is set, to 0.35; then
/// `UpdateMeleeBar(strength * 1.5)`. (TS sets the attack time before the sound; nothing reads it
/// in between.)
pub fn jump_kick_hit_core(p: &mut Player, cx: &mut Cx, strength: f32) {
    if p.inv.gun.next_attack < 0.0 {
        p.inv.gun.next_attack = 0.35;
    }
    crate::weapons::update_melee_bar(p, cx, strength * 1.5);
}

/// The backflip kick (`CTSStunt::GoWheel` backward): 22.5 kung-fu power and a delayed hit of 15
/// (30 with Kung Fu) in the gun's close-combat window.
pub fn backflip_kick_core(p: &mut Player, cx: &mut Cx) {
    let dmg = if p.kungfu { 30.0 } else { 15.0 };
    crate::weapons::arm_kick(p, cx, 22.5, dmg);
}

/// `CBasePlayer::Jump` (0x92864), every frame jump is held: out of deep water and not water
/// jumping, the jump kick ([`close_attack`]) first; if it hit, nothing else. Else a fresh press
/// tries `GoWheel`, then plays the jump animation on the ground (conveyors add their speed).
pub(super) fn jump(p: &mut Player, cx: &mut Cx, r: Rules) {
    let pev = cx.pev();
    if pev.flags & FL_WATERJUMP as i32 != 0 || pev.waterlevel >= 2 {
        return;
    }
    // Every frame jump is held, pressed or not, as TS.
    if close_attack(p, cx, r) {
        return;
    }
    let pressed = p.stunt.button_pressed & IN_JUMP as i32 != 0;
    if pressed && go_wheel(p, cx, r) {
        return;
    }
    let pev = cx.pev();
    if pressed && pev.flags & FL_ONGROUND as i32 != 0 && !pev.groundentity.is_null() {
        anim::animate(p, cx, 4);
        let pev = cx.pev();
        let ground = pev.groundentity.v();
        if ground.flags & FL_CONVEYOR as i32 != 0 {
            pev.velocity =
                [pev.velocity[0] + pev.basevelocity[0], pev.velocity[1] + pev.basevelocity[1], pev.velocity[2] + pev.basevelocity[2]];
        }
    }
}

/// `CBasePlayer::Duck` (0x927fc).
pub(super) fn duck(p: &mut Player, cx: &mut Cx, r: Rules) {
    if p.stunt.button_pressed & IN_DUCK as i32 != 0 && go_roll(p, cx, r) {
        return;
    }
    if cx.pev().button & IN_DUCK as i32 != 0 && cx.pev().flags & FL_DUCKING as i32 == 0 {
        anim::animate(p, cx, 5);
    }
}

/// The fatal-fall splat's channel: `CHAN_ITEM` (the push of 3 at 0x917b2), so it doesn't cut
/// pmove's `CHAN_VOICE` fall pain of the same landing (issue #40).
const FALL_SPLAT_CHANNEL: u32 = CHAN_ITEM;

/// Fall damage from `CBasePlayer::PostThink` (decomp §6.6): above 680 u/s,
/// `(v - 680) * 100 / 444` (fatal at 1124). A fatal fall splats on `CHAN_ITEM` (channel 3), so it
/// doesn't cut off pmove's `pl_fallpain3.wav` on `CHAN_VOICE`.
pub(super) fn fall_damage(p: &mut Player, cx: &mut Cx) {
    let pev = cx.pev();
    let fallvel = p.stunt.fall_velocity;
    if pev.flags & FL_ONGROUND as i32 == 0 || pev.health <= 0.0 || fallvel < 350.0 {
        return;
    }
    if pev.watertype != CONTENTS_WATER && fallvel > shared::pm::PLAYER_MAX_SAFE_FALL_SPEED {
        p.stunt.fall_velocity = fallvel - 680.0;
        let dmg = (fallvel - 680.0) * 100.0 * 0.002_252_252_3;
        if pev.health < dmg {
            emit_sound_slow(cx.edict, FALL_SPLAT_CHANNEL, c"common/bodysplat.wav", 1.0, ATTN_NORM, 0, 100, 0.5);
        }
        if dmg > 0.0 {
            let world = engine().entity_of_index(0);
            let info = DamageInfo::new(world, world, dmg, bits::FALL | bits::NEVERGIB);
            crate::player::damage::take_damage(p, cx, &info);
        }
    }
    if is_alive(cx) {
        anim::animate(p, cx, 1);
    }
}

/// Super-jump landing (PostThink): shake, then a forced roll if duck is held, else a hard
/// landing (`superjump-land.wav`, short recovery).
pub(super) fn super_jump_landing(p: &mut Player, cx: &mut Cx, r: Rules) {
    let rules = r.stunts();
    let origin = cx.pev().origin;
    screen_shake(origin, 20.0, 30.0, 0.3, 512.0);
    cx.pev().iuser4 &= !SUPERJUMP_GLIDE;
    let mut rolled = false;
    if cx.pev().button & IN_DUCK as i32 != 0 {
        p.stunt.roll_tap = cx.time() + 0.3;
        rolled = go_roll(p, cx, r);
        if rolled {
            // m_flNextAttack = UTIL_WeaponTimeBase() + 0.75 (relative weapon timer).
            p.inv.gun.next_attack = r.stunt_clock().attack_delay(cx.pev().fuser1, rules.super_jump_roll_attack_delay);
        }
    }
    if !rolled {
        stunt_sound(cx, c"player/superjump-land.wav");
        cx.pev().iuser4 |= GO_DUCK | RECOVER;
        set_view(cx, shared::pm::VIEW_DUCK);
        p.stunt.recover = timer(cx, r, rules.super_jump_landing_recover);
        anim::animate(p, cx, 0x3a);
    }
}

/// `CTSStunt::CheckForBreakables` (0x9bd9c): diving through glass and delayed kicks.
///
/// With a close-combat window open (pl+0x79c, armed by weapons or the backflip kick) the
/// delayed hit is resolved by `weapons::pending_melee_hit` (blocks, disarm, knockback), alive or
/// not. Without
/// one, the head hull is traced 64 units along the velocity and non-player victims take `|vel|`
/// damage (`|vel| * 0.1` while the diver is alive, as TS does).
pub(super) fn check_for_breakables(p: &mut Player, cx: &mut Cx) {
    let time = cx.time();
    if p.stunt.breakable_check == -1.0 || time < p.stunt.breakable_check {
        return;
    }
    p.stunt.breakable_check = time;
    if p.inv.gun.melee_window != 0.0 {
        // Alive or not: `PreThink` runs this before its `deadflag` return and death doesn't
        // close the window, so a hit armed just before dying still lands (`IsAlive` only picks
        // the stunt impact's damage below).
        crate::weapons::pending_melee_hit(p, cx);
        return;
    }
    if on_ground(cx) {
        p.inv.sv.lunge_victim = std::ptr::null_mut();
        return;
    }
    let v = cx.pev().velocity;
    let speed = (v[2] * v[2] + v[1] * v[1] + v[0] * v[0]).sqrt();
    if speed < 64.0 {
        return;
    }
    let o = cx.pev().origin;
    let d = normalized(v);
    let end = [o[0] + d[0] * 64.0, o[1] + d[1] * 64.0, o[2] + d[2] * 64.0];
    let tr = engine().trace_hull(o, end, DONT_IGNORE_MONSTERS, HEAD_HULL, cx.edict);
    if tr.flFraction >= 1.0 {
        return;
    }
    let victim = if tr.pHit.is_null() { engine().entity_of_index(0) } else { tr.pHit };
    if victim.is_null() || !victim.has_private_data() || is_player(victim) {
        return;
    }
    let vv = victim.v();
    if vv.takedamage == 0.0 || vv.flags & FL_WORLDBRUSH as i32 != 0 || vv.effects & EF_NODRAW as i32 != 0 {
        return;
    }
    if p.inv.sv.lunge_victim == victim {
        return;
    }
    let dmg = if is_alive(cx) { speed * 0.1 } else { speed };
    let mut info = DamageInfo::new(cx.edict, cx.edict, dmg, bits::CLUB | bits::NEVERGIB).traced();
    info.hitgroup = tr.iHitgroup;
    info.pos = tr.vecEndPos;
    crate::damage::apply(victim, &info);
    p.inv.sv.lunge_victim = victim;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `CTSStunt::CloseAttack`'s gate (0x9c630..0x9c6b5).
    #[test]
    fn fall_splat_is_on_the_item_channel() {
        assert_eq!(FALL_SPLAT_CHANNEL, 3);
    }

    #[test]
    fn jump_kick_gate() {
        use super::super::rules::StuntClock;
        let clock = &super::super::rules::RELATIVE;
        let ok = |recover: f32, cd: f32, iuser4, flags, gun, weapon| {
            close_attack_allowed(clock.running(0.0, recover), clock.running(0.0, cd), iuser4, flags, gun, weapon)
        };
        // a weapon in hand
        assert!(ok(0.0, 0.0, 0, 0, true, shared::weapons::id::AK47));
        // bare-handed (kung fu or holstered) with the gun active: no kick
        assert!(!ok(0.0, 0.0, 0, 0, true, 0));
        // no active item (or a cycler_weapon): the kick is allowed
        assert!(ok(0.0, 0.0, 0, 0, false, 0));
        assert!(!ok(0.001, 0.0, 0, 0, false, 0));
        assert!(!ok(0.0, 0.7, 0, 0, false, 0));
        assert!(!ok(0.0, 0.0, 0, FL_DUCKING as i32, false, 0));
        for bit in [0x10, 0x20, 0x400, 0x800, 0x10000, 0x20000] {
            assert!(!ok(0.0, 0.0, bit, 0, false, 0), "{bit:#x}");
        }
        // the other stunt bits don't stop it (0x100000 is GoWheel's extra bit)
        assert!(ok(0.0, 0.0, 0x100001, FL_ONGROUND as i32, false, 0));
        // a negative timer has run out
        assert!(ok(-0.5, -0.5, 0, 0, false, 0));
    }

    /// 3.0 arms the cooldown and draws the move only after a hit; a ruleset that arms it on
    /// every try does both before the try.
    #[test]
    fn jump_kick_cooldown_order() {
        use KickStep::*;
        assert_eq!(kick_steps(super::super::rules::CORE.kick_cooldown_on_try), [Hit, Draw, Arm]);
        assert_eq!(kick_steps(true), [Arm, Draw, Hit]);
    }

    /// The chain adds a `double` constant on the x87 (#124): a wall climb with high-dive space
    /// (+0.3) then a wall jump (+0.35) is TS's 0.65000004, not the `f32` sum 0.64999998.
    #[test]
    fn wall_chain_adds_doubles() {
        use shared::pm::Arith;
        for arith in [Arith::Msvc, Arith::Gcc] {
            let mut chain = 0.0f32;
            let first = wall_chain_add(arith, &mut chain, 0.3);
            assert_eq!(first, if arith == Arith::Msvc { 0.3 } else { f64::from(0.3f32) }, "{arith:?}");
            assert_eq!(chain.to_bits(), 0.3f32.to_bits());
            let sum = wall_chain_add(arith, &mut chain, 0.35);
            assert_eq!(chain.to_bits(), 0x3f26_6667, "{arith:?}: 0.65000004");
            assert_ne!(chain, 0.3f32 + 0.35f32, "the old f32 sum");
            // mp.dll's GoWallJump hands the 53-bit sum on; ts_i386.so reloads the float.
            let held = if arith == Arith::Msvc { f64::from(0.3f32) + 0.35 } else { f64::from(chain) };
            assert_eq!(sum, held, "{arith:?}");
        }
        // 0.5 and 0.75 are exact either way.
        let mut chain = 0.35f32;
        wall_chain_add(Arith::Msvc, &mut chain, 0.75);
        assert_eq!(chain, 0.35f32 + 0.75);
    }

    #[test]
    fn jump_kick_anims() {
        assert_eq!([0, 1, 2].map(close_attack_anim), [10, 14, 13]);
    }
}
