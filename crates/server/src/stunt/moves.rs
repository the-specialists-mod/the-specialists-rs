//! `CTSStunt` (decomp `server/decomp/CTSStunt.c`, 0x99fbc-0x9d3ec) and the stunt parts of
//! `CBasePlayer::Jump`/`Duck`/`PostThink`. The functions keep the TS names.

use super::*;
use crate::damage::{DamageInfo, bits};
use goldsrc::edict::EdictPtr;
use goldsrc::server::{DONT_IGNORE_MONSTERS, HEAD_HULL, IGNORE_MONSTERS};

/// `CanDiveHigh/Medium/Low` (0x9a494...): used slots (`81 - TSSpace`) must be below 42/52/62
/// ("40 for high, 50 for medium, 60 for low"); The One ignores the limits. `SlotBonus()` is 0.
fn can_dive(p: &Player, limit: i32) -> bool {
    p.stunt.the_one || p.slots_used < limit
}
fn can_dive_high(p: &Player) -> bool {
    can_dive(p, 42)
}
fn can_dive_medium(p: &Player) -> bool {
    can_dive(p, 52)
}
fn can_dive_low(p: &Player) -> bool {
    can_dive(p, 62)
}

/// `CTSStunt::NextDive`: no cooldown after dives and flips.
const NEXT_DIVE: f32 = 0.0;

fn forward(cx: &mut Cx) -> Vec3 {
    shared::pm::angle_vectors(cx.pev().v_angle).0
}

fn hspeed(v: Vec3) -> f32 {
    (v[1] * v[1] + v[0] * v[0]).sqrt()
}

fn set_view(cx: &mut Cx, z: f32) {
    cx.pev().view_ofs = [0.0, 0.0, z];
}

/// `CTSStunt::Play` (0x9a014), every PreThink.
pub(super) fn play(p: &mut Player, cx: &mut Cx) {
    let dt = slow_mul(cx) * frametime();
    let s = &mut p.stunt;
    if s.recover != 0.0 {
        s.recover = (s.recover - dt).max(0.001);
    }
    s.next_stunt = (s.next_stunt - dt).max(0.0);
    s.close_attack_cd = (s.close_attack_cd - dt).max(0.0);
    s.prone_lock = (s.prone_lock - dt).max(0.0);

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
        if s.recover != 0.0 && (s.recover < 0.01 || s.anim.seq_finished) {
            let pev = cx.pev();
            pev.iuser4 &= !(ROLLING | FREEZE_ANGLES | GO_DUCK | RECOVER | DEAD_LOCK);
            s.recover = 0.0;
            s.block_timer = 0.0;
            pev.friction = 1.0;
            let z = if pev.flags & FL_DUCKING as i32 != 0 { shared::pm::VIEW_DUCK } else { shared::pm::VIEW_STAND };
            pev.view_ofs = [0.0, 0.0, z];
        }
        if cx.pev().iuser4 & DIVING != 0 {
            handle_dive(p, cx);
        }
        if cx.pev().iuser4 & PRONE != 0 {
            handle_slide(p, cx);
        }
        if cx.pev().iuser4 & (FLIP_HIGH | FLIP_LOW) != 0 {
            handle_wheel(p, cx);
        }
        if cx.pev().iuser4 & WALLJUMPING != 0 {
            handle_wall_jump(p, cx);
        }
        if cx.pev().iuser4 & WALLCLIMBING != 0 {
            handle_wall_climb(p, cx);
        }
        if cx.pev().button & (IN_JUMP as i32 | IN_ALT1) != 0 {
            handle_stunt(p, cx);
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
            p.stunt.block_timer = 0.0;
            p.stunt.anim.current = -1;
            anim::animate(p, cx, 1);
        }
    }
}

/// `CTSStunt::HandleStunt` (0x9b000): the stunt key (or jump) was used.
fn handle_stunt(p: &mut Player, cx: &mut Cx) {
    if p.stunt.next_stunt > 0.0 || p.stunt.recover > 0.0 {
        return;
    }
    if cx.pev().iuser4 & (PRONE | DIVING | FLIP_HIGH | FLIP_LOW) != 0 {
        return;
    }
    let g = &mut p.inv.gun;
    g.melee_window = 0.0;
    g.melee_damage = 0.0;
    g.melee_source = 0;

    let alt_pressed = p.stunt.button_pressed & IN_ALT1 != 0;
    if cx.pev().button & IN_JUMP as i32 != 0 {
        if check_for_wall_climb(p, cx) {
            go_wall_climb(p, cx);
            return;
        }
    } else if alt_pressed {
        if check_for_wall_jump(p, cx) {
            go_wall_jump(p, cx);
            return;
        }
        if check_for_wall_dive(cx) {
            cx.pev().iuser4 |= WALLDIVE;
            go_dive(p, cx);
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
        if 0.7 / slow_mul(cx) < cx.time() - p.stunt.last_ground_time {
            return;
        }
    }
    if hspeed(cx.pev().velocity) < 80.0 {
        go_bullet_time(p, cx);
    } else {
        go_dive(p, cx);
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
fn check_for_wall_jump(p: &Player, cx: &mut Cx) -> bool {
    if !can_dive_high(p) {
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
fn go_bullet_time(p: &mut Player, cx: &mut Cx) {
    if slow_mul(cx) > 0.99 {
        return;
    }
    stunt_dir_back(p, cx);
    p.stunt.dir_not_forward = true;
    p.stunt.next_stunt = 0.55;
    p.stunt.dodge_until = cx.time() + 0.55;
    send_next_stunt(p, cx);
    cx.pev().iuser4 |= GO_DUCK | PRONE;
    p.stunt.anim.last_dir = 3;
    p.stunt.prone_lock = 0.55;
    set_view(cx, 0.0);
    anim::animate(p, cx, 0x36);
    stunt_sound(cx, c"player/land.wav");
    motion_blur(cx, 2);
}

/// `CTSStunt::GoWallJump` (0x9a7d4): kick off the wall in front.
fn go_wall_jump(p: &mut Player, cx: &mut Cx) {
    stunt_dir_back(p, cx);
    let pev = cx.pev();
    pev.iuser4 &= !SUPERJUMP_GLIDE;
    pev.iuser4 |= GO_DUCK | WALLJUMP_IMPULSE | WALLJUMPING;
    pev.iuser4 &= !WALLCLIMBING;
    pev.view_ofs = [0.0; 3];
    anim::animate(p, cx, 0x37);
    cx.pev().gravity = 0.75;
    p.stunt.wall_chain += 0.35;
    p.stunt.next_stunt = p.stunt.wall_chain.max(0.5);
    send_next_stunt(p, cx);
    stunt_sound(cx, c"player/airwheel.wav");
    maybe_blur(cx, 5.0);
}

/// `CTSStunt::GoWallClimb` (0x9aa1c): run up the wall in front.
fn go_wall_climb(p: &mut Player, cx: &mut Cx) {
    stunt_dir_back(p, cx);
    let pev = cx.pev();
    pev.iuser4 &= !SUPERJUMP_GLIDE;
    pev.iuser4 |= GO_DUCK | WALLCLIMB_IMPULSE | WALLCLIMBING;
    pev.iuser4 &= !WALLJUMPING;
    pev.view_ofs = [0.0; 3];
    anim::animate(p, cx, 0x38);
    cx.pev().gravity = 1.0;
    let add = if can_dive_high(p) {
        0.3
    } else if can_dive_medium(p) {
        0.5
    } else {
        0.75
    };
    p.stunt.wall_chain += add;
    p.stunt.next_stunt = p.stunt.wall_chain.max(0.5);
    send_next_stunt(p, cx);
    stunt_sound(cx, c"player/airwheel.wav");
    p.stunt.wall_climbs += 1;
}

/// `CTSStunt::GoDive` (0x9b1a0): dive height by pitch and slot space.
fn go_dive(p: &mut Player, cx: &mut Cx) {
    let pitch = cx.pev().v_angle[0];
    if pitch < -10.0 && can_dive_high(p) {
        cx.pev().iuser4 |= DIVE_HIGH;
        cx.pev().gravity = 0.75;
    } else if pitch <= 10.0 && can_dive_medium(p) {
        cx.pev().iuser4 |= DIVE_MED;
        cx.pev().gravity = 0.75;
    } else {
        cx.pev().iuser4 |= DIVE_LOW;
        if can_dive_low(p) {
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
    p.stunt.next_stunt = NEXT_DIVE;
    send_next_stunt(p, cx);
    maybe_blur(cx, 8.0);
    p.stunt.dir = if cx.pev().iuser4 & WALLDIVE == 0 { normalized(cx.pev().velocity) } else { forward(cx) };
    stunt_sound(cx, c"player/dive.wav");
}

/// Landing from a wall jump/climb: `CTSStunt::HandleWallJump` (0x9b4fc) and the ground half of
/// `HandleWallClimb` (0x9b644).
fn land_wall_stunt(p: &mut Player, cx: &mut Cx, bit: i32) {
    if cx.pev().iuser4 & bit != 0 {
        cx.pev().iuser4 &= !bit;
        cx.pev().friction = 0.5;
        set_view(cx, 0.0);
        p.stunt.recover = 0.2625;
        if p.stunt.next_stunt < 0.01 {
            p.stunt.next_stunt = 0.0;
            send_next_stunt(p, cx);
        }
    }
    cx.pev().gravity = 1.0;
    stunt_sound(cx, c"player/land.wav");
}

fn handle_wall_jump(p: &mut Player, cx: &mut Cx) {
    if on_ground(cx) {
        land_wall_stunt(p, cx, WALLJUMPING);
    }
}

fn handle_wall_climb(p: &mut Player, cx: &mut Cx) {
    if !on_ground(cx) {
        // Climbing and the stunt key: kick off, or dive away from the wall.
        if p.stunt.button_pressed & IN_ALT1 != 0 {
            if check_for_wall_jump(p, cx) {
                go_wall_jump(p, cx);
            } else if check_for_wall_dive(cx) {
                cx.pev().iuser4 |= WALLDIVE;
                go_dive(p, cx);
            }
        }
    } else {
        land_wall_stunt(p, cx, WALLCLIMBING);
    }
}

/// `CTSStunt::HandleDive` (0x9b7e4).
fn handle_dive(p: &mut Player, cx: &mut Cx) {
    landing(p, cx);
    anim::animate(p, cx, 2);
}

/// `CTSStunt::Landing` (0x9bbf4): land a dive into a slide, or roll out if a key is held.
fn landing(p: &mut Player, cx: &mut Cx) {
    if !on_ground(cx) || cx.pev().iuser4 & DIVING == 0 {
        return;
    }
    if cx.pev().button & (IN_ALT1 | (IN_DUCK | IN_JUMP) as i32) == 0 {
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
        p.stunt.recover = 0.5625;
        anim::animate(p, cx, 0x2e);
    }
    cx.pev().gravity = 1.0;
    stunt_sound(cx, c"player/land.wav");
}

/// `CTSStunt::HandleWheel` (0x9b820): cartwheels end prone, flips end in a short recovery.
fn handle_wheel(p: &mut Player, cx: &mut Cx) {
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
        p.stunt.recover = 0.375;
    }
    cx.pev().gravity = 1.0;
    stunt_sound(cx, c"player/land.wav");
}

/// `CTSStunt::HandleSlide` (0x9b9f8): lying after a dive. Back brakes, jump/duck/stunt gets up,
/// strafe rolls sideways.
fn handle_slide(p: &mut Player, cx: &mut Cx) {
    if !on_ground(cx) {
        return;
    }
    let button = cx.pev().button;
    if button & IN_BACK as i32 != 0 {
        cx.pev().friction = 1.0;
    }
    let free = p.stunt.prone_lock < 0.01;
    if button & (IN_ALT1 | (IN_DUCK | IN_JUMP) as i32) != 0 && free {
        cx.pev().iuser4 |= RECOVER;
        cx.pev().iuser4 &= !PRONE;
        set_view(cx, shared::pm::VIEW_STAND);
        p.stunt.recover = 0.75;
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
fn go_wheel(p: &mut Player, cx: &mut Cx) -> bool {
    let time = cx.time();
    let s = &p.stunt;
    let mut ok = s.close_attack_cd <= 0.0
        && !on_ladder(cx)
        && s.recover <= 0.0
        && cx.pev().iuser4 & 0x130c30 == 0
        && time <= s.wheel_tap
        && s.next_stunt <= 0.0;
    if !on_ground(cx) && 0.7 / slow_mul(cx) < time - p.stunt.last_ground_time {
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
        maybe_blur(cx, 7.0);
        if can_dive_high(p) && (dirn == 3 || cx.pev().v_angle[0] < -10.0) {
            cx.pev().iuser4 |= GO_DUCK | FLIP_HIGH;
            set_view(cx, 0.0);
            anim::animate(p, cx, 0x31);
            cx.pev().iuser4 |= DIVE_MED;
            cx.pev().gravity = 0.75;
            p.stunt.next_stunt = NEXT_DIVE;
            send_next_stunt(p, cx);
            stunt_sound(cx, c"player/airwheel.wav");
            p.stunt.wheel_tap = 0.0;
            if dirn == 3 {
                // Backflip kick: costs 22.5 kung-fu power and arms a delayed hit (15, 30 with
                // Kung Fu) in the gun's close-combat window.
                let dmg = if p.kungfu { 30.0 } else { 15.0 };
                crate::weapons::arm_kick(p, cx, 22.5, dmg);
            }
            return true;
        }
        if can_dive_low(p) && dirn != 3 {
            cx.pev().iuser4 |= GO_DUCK | FLIP_LOW;
            set_view(cx, 0.0);
            anim::animate(p, cx, 0x32);
            cx.pev().iuser4 |= DIVE_LOW;
            cx.pev().gravity = 0.75;
            if dirn == 2 {
                cx.pev().friction = 0.25;
            }
            p.stunt.next_stunt = NEXT_DIVE;
            send_next_stunt(p, cx);
            stunt_sound(cx, c"player/airwheel.wav");
            p.stunt.wheel_tap = 0.0;
            return true;
        }
    }
    p.stunt.wheel_tap = time + 0.3;
    false
}

/// `CTSStunt::GoRoll` (0x9cfe8): duck double-tap while moving at 80+ u/s.
fn go_roll(p: &mut Player, cx: &mut Cx) -> bool {
    let time = cx.time();
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
        || p.stunt.recover > 0.0
        || on_ladder(cx)
        || p.stunt.close_attack_cd > 0.0
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
    p.stunt.recover = 0.75;
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
    maybe_blur(cx, 9.0);
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

/// The stunt parts of `CBasePlayer::Jump` (0x92864). TS first tries `CTSStunt::CloseAttack`
/// (jump kick), which belongs to the weapons package (close combat) and isn't wired here.
pub(super) fn jump(p: &mut Player, cx: &mut Cx) {
    let pev = cx.pev();
    if pev.flags & FL_WATERJUMP as i32 != 0 || pev.waterlevel >= 2 {
        return;
    }
    let pressed = p.stunt.button_pressed & IN_JUMP as i32 != 0;
    if pressed && go_wheel(p, cx) {
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
pub(super) fn duck(p: &mut Player, cx: &mut Cx) {
    if p.stunt.button_pressed & IN_DUCK as i32 != 0 && go_roll(p, cx) {
        return;
    }
    if cx.pev().button & IN_DUCK as i32 != 0 && cx.pev().flags & FL_DUCKING as i32 == 0 {
        anim::animate(p, cx, 5);
    }
}

/// Fall damage from `CBasePlayer::PostThink` (decomp §6.6): above 680 u/s,
/// `(v - 680) * 100 / 444` (fatal at 1124).
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
            emit_sound_slow(cx.edict, CHAN_VOICE, c"common/bodysplat.wav", 1.0, ATTN_NORM, 0, 100, 0.5);
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
pub(super) fn super_jump_landing(p: &mut Player, cx: &mut Cx) {
    let origin = cx.pev().origin;
    screen_shake(origin, 20.0, 30.0, 0.3, 512.0);
    cx.pev().iuser4 &= !SUPERJUMP_GLIDE;
    let mut rolled = false;
    if cx.pev().button & IN_DUCK as i32 != 0 {
        p.stunt.roll_tap = cx.time() + 0.3;
        rolled = go_roll(p, cx);
        if rolled {
            // m_flNextAttack = UTIL_WeaponTimeBase() + 0.75 (relative weapon timer).
            p.inv.gun.next_attack = 0.75;
        }
    }
    if !rolled {
        stunt_sound(cx, c"player/superjump-land.wav");
        cx.pev().iuser4 |= GO_DUCK | RECOVER;
        set_view(cx, shared::pm::VIEW_DUCK);
        p.stunt.recover = 0.75;
        anim::animate(p, cx, 0x3a);
    }
}

/// `CTSStunt::CheckForBreakables` (0x9bd9c): diving through glass and delayed kicks.
///
/// With a close-combat window open (pl+0x79c, armed by weapons or the backflip kick) the
/// delayed hit is resolved by `weapons::pending_melee_hit` (blocks, disarm, knockback). Without
/// one, the head hull is traced 64 units along the velocity and non-player victims take `|vel|`
/// damage (`|vel| * 0.1` while the diver is alive, as TS does).
pub(super) fn check_for_breakables(p: &mut Player, cx: &mut Cx) {
    let time = cx.time();
    if p.stunt.breakable_check == -1.0 || time < p.stunt.breakable_check {
        return;
    }
    p.stunt.breakable_check = time;
    if p.inv.gun.melee_window != 0.0 {
        if is_alive(cx) {
            crate::weapons::pending_melee_hit(p, cx);
        }
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
