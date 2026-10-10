//! `CTSAnimation` (decomp `server/decomp/CTSAnimation.c`, 0x6b0c8-0x6ddc6): picks the torso
//! (`sequence`) and leg (`gaitsequence`) animations from the stunt state, the weapon animation
//! extensions and the player's speed. Sequence names are looked up in the player model through
//! the studio header (`goldsrc::ext::movement`).
//!
//! The animation `framerate` is the player's time scale (set every PreThink), and
//! `studio_frame_advance` (SDK `CBaseAnimating::StudioFrameAdvance`) runs in PostThink, and in
//! `PlayerDeathThink` while dying, to keep `m_fSequenceFinished`: stunts use it to end recoveries
//! early, and the death think to end the dying state.

use super::*;
use goldsrc::ext::movement::{self as studio, STUDIO_LOOPING};

/// The name tables (16-byte slots in TS), indexed by `GetDirection`: left, right, forward, back.
const JUMP: [&str; 4] = ["jump_left_", "jump_right_", "jump_forward_", "jump_forward_"];
const DIVE: [&str; 4] = ["dive_l_", "dive_r_", "dive_f_", "dive_b_"];
const SLIDE: [&str; 4] = ["slide_l_", "slide_r_", "slide_f_", "slide_b_"];
const SLIDE_END: [&str; 4] = ["slide_l_end", "slide_r_end", "slide_f_end", "slide_b_end"];
const DIVE_END: [&str; 4] = ["dive_l_end", "dive_r_end", "dive_f_end", "dive_b_end"];
const ROLL: [&str; 4] = ["roll_l", "roll_r", "roll_f", "roll_b"];
const ROLL_HIGH: [&str; 4] = ["roll_l", "roll_r", "roll_f_high", "roll_b"];

/// `SetAnim` flags.
const SA_GAIT_ONLY: i32 = 0x01;
const SA_FULL_BODY: i32 = 0x04;
const SA_INTERP: i32 = 0x10;
const SA_SET_FRAME: i32 = 0x20;
const SA_FORCE: i32 = 0x80;
const SA_KEEP_FRAME: i32 = 0x100;

/// `CTSAnimation` (pl+0x28c) and the `CBaseAnimating` sequence state the player needs.
#[derive(Clone, Debug)]
pub struct AnimState {
    /// +0x00 current animation id (-1 none).
    pub current: i32,
    /// +0x04 `goduck` played for this duck.
    pub duck_latch: bool,
    /// +0x08 hold timer (3.0 after rolls/flips), counts down in slowed time.
    pub hold: f32,
    /// +0x0c "recently shot": aim poses instead of idle ones while ≥ 0.01.
    pub shot: f32,
    /// +0x10 grenade timer.
    pub grenade: f32,
    /// +0x14 last direction (avoids restarting the same blend).
    pub last_dir: i32,
    /// +0x1dc / +0x1e0 / +0x1e4 bullet dance (dead body twitching when shot).
    pub dance_next: f32,
    pub dance_time: f32,
    pub bullet_dance: bool,
    /// +0x1e8 close-combat flag.
    pub cc_flag: bool,
    /// pl+0x478 `m_szAnimExtention` (idle gait override, e.g. "knife_idle").
    pub ext: String,
    /// pl+0xb58 torso extension: "onehanded", "twohanded", "akimbo", "onehanded_case" or "".
    pub ext2: String,
    /// Death animation inputs (rules sets them before `animate(7)`): `m_lastDamageAmount`,
    /// `m_LastHitGroup`, `g_vecAttackDir`.
    pub last_damage: i32,
    pub last_hitgroup: i32,
    pub attack_dir: Vec3,
    /// `CBaseAnimating`: m_fSequenceFinished, m_flFrameRate, m_flGroundSpeed, m_fSequenceLoops.
    pub seq_finished: bool,
    pub frame_rate: f32,
    pub ground_speed: f32,
    pub seq_loops: bool,
}

impl Default for AnimState {
    fn default() -> Self {
        Self {
            current: -1,
            duck_latch: false,
            hold: 0.0,
            shot: 0.0,
            grenade: 0.0,
            last_dir: -1,
            dance_next: 0.0,
            dance_time: 0.0,
            bullet_dance: false,
            cc_flag: false,
            ext: String::new(),
            ext2: String::new(),
            last_damage: 0,
            last_hitgroup: 0,
            attack_dir: [0.0; 3],
            seq_finished: false,
            frame_rate: 0.0,
            ground_speed: 0.0,
            seq_loops: false,
        }
    }
}

fn lookup(cx: &mut Cx, name: &str) -> i32 {
    studio::lookup_sequence(cx.edict, name)
}

/// SDK `ResetSequenceInfo`.
pub fn reset_sequence_info(p: &mut Player, cx: &mut Cx) {
    let seq = cx.pev().sequence;
    let info = studio::model_ptr(cx.edict).and_then(|h| h.sequence_info(seq));
    let a = &mut p.stunt.anim;
    match info {
        Some(i) => {
            (a.frame_rate, a.ground_speed) = i.rates();
            a.seq_loops = i.flags & STUDIO_LOOPING != 0;
        }
        None => {
            (a.frame_rate, a.ground_speed) = (0.0, 0.0);
            a.seq_loops = false;
        }
    }
    let time = cx.time();
    let pev = cx.pev();
    pev.animtime = time;
    pev.framerate = 1.0;
    a.seq_finished = false;
}

/// SDK `CBaseAnimating::StudioFrameAdvance(0)`.
pub fn studio_frame_advance(p: &mut Player, cx: &mut Cx) {
    let time = cx.time();
    let a = &mut p.stunt.anim;
    let pev = cx.pev();
    let mut interval = time - pev.animtime;
    if interval <= 0.001 {
        pev.animtime = time;
        return;
    }
    if pev.animtime == 0.0 {
        interval = 0.0;
    }
    pev.frame += interval * a.frame_rate * pev.framerate;
    pev.animtime = time;
    if pev.frame < 0.0 || pev.frame >= 256.0 {
        if a.seq_loops {
            pev.frame -= ((pev.frame / 256.0) as i32) as f32 * 256.0;
        } else {
            pev.frame = if pev.frame < 0.0 { 0.0 } else { 255.0 };
        }
        a.seq_finished = true;
    }
}

/// `CTSAnimation::SetAnim` (0x6d7e4).
fn set_anim(p: &mut Player, cx: &mut Cx, name: &str, flags: i32, frame: i32) -> bool {
    let mut seq = lookup(cx, name);
    if seq == -1 {
        seq = 0;
    }
    let pev = cx.pev();
    if pev.sequence == seq && flags & SA_FORCE == 0 {
        return false;
    }
    if flags & SA_INTERP == 0 {
        pev.effects |= EF_NOINTERP as i32;
    } else {
        pev.effects &= !(EF_NOINTERP as i32);
    }
    if flags & SA_FULL_BODY != 0 {
        pev.gaitsequence = 0;
    } else if flags & SA_GAIT_ONLY != 0 {
        pev.gaitsequence = seq;
        return true;
    }
    pev.sequence = seq;
    if flags & SA_SET_FRAME != 0 {
        pev.frame = frame as f32;
    } else if flags & SA_KEEP_FRAME == 0 {
        pev.frame = 0.0;
    }
    reset_sequence_info(p, cx);
    true
}

/// `CTSAnimation::UpdateTimers` (0x6b37c), in slowed time.
pub fn update_timers(p: &mut Player, cx: &mut Cx) {
    let dt = slow_mul(cx) * frametime();
    let a = &mut p.stunt.anim;
    a.dance_time = (a.dance_time - dt).max(0.0);
    a.dance_next = (a.dance_next - dt).max(0.0);
    a.shot = (a.shot - dt).max(0.0);
    a.grenade = (a.grenade - dt).max(0.0);
    a.hold = if a.hold >= -0.001 { (a.hold - dt).max(0.0) } else { (dt + a.hold).min(0.0) };
}

/// `CTSAnimation::EnableBulletDance` (0x6d314), from `CBasePlayer::TakeDamage`.
pub fn enable_bullet_dance(p: &mut Player) {
    let a = &mut p.stunt.anim;
    a.bullet_dance = true;
    a.dance_next = 0.15;
    a.dance_time = 3.0;
}

/// `CTSAnimation::DisableBulletDance` (0x6d2f0), from `CBasePlayer::TakeDamage`.
pub fn disable_bullet_dance(p: &mut Player) {
    let a = &mut p.stunt.anim;
    a.bullet_dance = false;
    a.dance_next = 0.0;
    a.dance_time = 0.0;
}

/// `CTSAnimation::BulletDance` (0x6d338): a dead body jerks through the death poses when shot.
/// Called from `CBasePlayer::TraceAttack` on a dead player; it only acts while the death animation
/// is still playing (`m_fSequenceFinished` clear), and each new pose restarts that animation.
pub fn bullet_dance(p: &mut Player, cx: &mut Cx) {
    let a = &p.stunt.anim;
    if !a.bullet_dance || a.dance_time < 0.01 || a.dance_next > 0.0 || a.seq_finished {
        return;
    }
    p.stunt.anim.dance_next = 0.15;
    let name = match engine().random_long(0, 2) {
        0 => "normal_die1",
        1 => "normal_die2",
        _ => "normal_die3",
    };
    set_anim(p, cx, name, 0x14, 0);
}

fn is_in_close_combat(id: i32) -> bool {
    id == 0x3a || (10..40).contains(&id)
}

/// `CTSAnimation::GaitCloseCombat` (0x6dd14): melee moves that keep the legs running.
fn gait_close_combat(cx: &mut Cx, id: i32) -> bool {
    let pev = cx.pev();
    let ducking = pev.flags & FL_DUCKING as i32 != 0;
    if id == 0x25 && (ducking || pev.button & IN_DUCK as i32 != 0) {
        return false;
    }
    let kinds =
        (0x16..=0x17).contains(&id) || (0x21..=0x25).contains(&id) || id == 0x19 || id == 0x26 || id == 0x27 || (0x1d..=0x20).contains(&id);
    if id == 0x24 || id == 0x23 || id == 0x3a || !kinds {
        return false;
    }
    ducking || hspeed2(pev.velocity) > 120.0
}

fn hspeed2(v: Vec3) -> f32 {
    (v[1] * v[1] + v[0] * v[0]).sqrt()
}

fn ext2_or_akimbo(p: &Player) -> String {
    if p.stunt.anim.ext2.is_empty() { "akimbo".to_owned() } else { p.stunt.anim.ext2.clone() }
}

/// `BlendAnimationsByDir`: switch to `table[dir] + ext` when the direction changes.
fn blend_by_dir(p: &mut Player, cx: &mut Cx, dir: Vec3, table: &[&str; 4], ext: &str) {
    if ext.is_empty() {
        return;
    }
    let d = get_direction(dir, cx.pev().v_angle, 0.6, 0.3);
    if d != p.stunt.anim.last_dir {
        let name = format!("{}{}", table[d as usize], ext);
        set_anim(p, cx, &name, 0x194, 0);
        p.stunt.anim.last_dir = d;
    }
}

/// `BlendAnimationsForSlide` (0x6d400).
fn blend_for_slide(p: &mut Player, cx: &mut Cx) {
    if !p.stunt.dir_not_forward {
        return;
    }
    let d = get_direction(p.stunt.dir, cx.pev().v_angle, 0.6, 0.3);
    if d != p.stunt.anim.last_dir && d != 2 {
        let name = format!("{}{}", SLIDE[d as usize], ext2_or_akimbo(p));
        set_anim(p, cx, &name, 0x194, 0);
        p.stunt.anim.last_dir = d;
    }
}

/// The slide table entry for the stunt direction (updating `last_dir` except toward forward).
fn slide_name(p: &mut Player, cx: &mut Cx) -> String {
    if !p.stunt.dir_not_forward {
        return "slide_f_".to_owned();
    }
    let d = get_direction(p.stunt.dir, cx.pev().v_angle, 0.6, 0.3);
    if d != p.stunt.anim.last_dir && d != 2 {
        p.stunt.anim.last_dir = d;
    }
    SLIDE[p.stunt.anim.last_dir.clamp(0, 3) as usize].to_owned()
}

/// `CTSAnimation::HandleCloseCombat` (0x6d8d0).
fn handle_close_combat(p: &mut Player, cx: &mut Cx, id: i32) {
    let name: String = match id {
        10 => "circfront".into(),
        0xb => "circkick".into(),
        0xc => "circkicklow".into(),
        0xd => "circback".into(),
        0xe => "latback".into(),
        0xf => "circspin".into(),
        0x10 => "sidekick".into(),
        0x11 => "backkick".into(),
        0x12 => "airkick".into(),
        0x13 => "airkick2".into(),
        0x14 => "duckkick".into(),
        0x16 => "punchleft".into(),
        0x17 => "punchright".into(),
        0x18 => "punchair".into(),
        0x19 => "uppercut".into(),
        0x1d => "knife1".into(),
        0x1e => "knife2".into(),
        0x1f => "knife3".into(),
        0x20 => "knife_launch".into(),
        0x21 => "katana1".into(),
        0x22 => "katana2".into(),
        0x23 => "katana3".into(),
        0x24 => "katana_air".into(),
        0x25 => "katana_duck".into(),
        0x26 => {
            let ext = &p.stunt.anim.ext;
            let base = if ext.starts_with("kat") {
                "katana_block"
            } else if ext.starts_with("kni") {
                "knife_block"
            } else {
                "kungfu_block"
            };
            let n = if engine().random_long(0, 1) == 0 { "2" } else { "1" };
            format!("{base}{n}")
        }
        0x27 => format!("stock_{}", p.stunt.anim.ext2),
        0x3a => "superjump_land".into(),
        _ => String::new(), // uninitialised buffer in TS
    };
    let flags = if gait_close_combat(cx, id) { 0x12 } else { 0x14 };
    set_anim(p, cx, &name, flags | SA_FORCE, 0);
    let a = &mut p.stunt.anim;
    a.shot = 2.0;
    a.hold = 3.0;
    a.cc_flag = false;
}

/// `CTSAnimation::Animate(player, id)` (0x6b470).
pub fn animate(p: &mut Player, cx: &mut Cx, id: i32) {
    if id < 0 {
        return;
    }
    let v = cx.pev().velocity;
    let speed = hspeed2(v);
    let alive = is_alive(cx);
    let grounded = on_ground(cx);
    let ladder = on_ladder(cx);
    let fin = p.stunt.anim.seq_finished;
    let iuser4 = cx.pev().iuser4;
    let cur = p.stunt.anim.current;

    // In the air: keep the jump and dive blends facing the right way.
    if !grounded && alive {
        if cur == 4 && speed > 96.0 && !is_in_close_combat(id) {
            let ext2 = p.stunt.anim.ext2.clone();
            blend_by_dir(p, cx, normalized(v), &JUMP, &ext2);
        }
        if id == 2 {
            let dir = p.stunt.dir;
            let ext = ext2_or_akimbo(p);
            blend_by_dir(p, cx, dir, &DIVE, &ext);
        }
    }

    // Priority: a playing stunt animation refuses lower-priority ids until it finishes.
    let mut cc_gait = false;
    if !is_in_close_combat(cur) {
        let stunt_ids = |id: i32| matches!(id, 4 | 0x28 | 0x2f | 0x30 | 0x31 | 0x37 | 0x38 | 0x36 | 0x32);
        #[allow(clippy::collapsible_match)] // one arm per TS switch case, each shaped like its siblings
        match cur {
            3 => {
                if alive && !matches!(id, 4 | 0x39 | 0x2a | 0x2b | 0x29 | 0 | 8 | 9) && !ladder {
                    blend_for_slide(p, cx);
                    return;
                }
            }
            4 => {
                if !fin && !stunt_ids(id) && alive && !grounded && !is_in_close_combat(id) {
                    return;
                }
            }
            7 => return,
            0x28 => {
                if !fin && alive && id != 0x29 && id != 0x2e && id != 3 && !ladder {
                    return;
                }
            }
            0x29 | 0x2e | 0x2f => {
                if !fin && alive && iuser4 & GO_DUCK != 0 {
                    return;
                }
            }
            0x2a | 0x2b => {
                let (_, right, _) = shared::pm::angle_vectors(cx.pev().v_angle);
                let len = (v[2] * v[2] + v[1] * v[1] + v[0] * v[0]).sqrt();
                let check = len == 0.0 || {
                    let a = normalized(v);
                    let b = normalized(right);
                    a[2] * b[2] + a[1] * b[1] + a[0] * b[0] != 0.0
                };
                if check && alive && !matches!(id, 0x2a | 0x2b | 3 | 0x29) && !ladder {
                    return;
                }
            }
            0x30 => {
                if !fin && alive {
                    return;
                }
            }
            0x31 => {
                if !fin && alive && iuser4 & GO_DUCK != 0 && !ladder {
                    return;
                }
            }
            0x32 => {
                if !fin && alive && !ladder && id != 0x29 && id != 0x2e {
                    return;
                }
            }
            0x36 => {
                if !fin && alive {
                    return;
                }
                p.stunt.prone_lock = 0.0;
            }
            0x37 => {
                if !fin && alive && iuser4 & GO_DUCK != 0 && !ladder && !matches!(id, 0x37 | 0x38 | 0x28) && !grounded {
                    return;
                }
            }
            0x38 => {
                if !fin
                    && alive
                    && iuser4 & GO_DUCK != 0
                    && !ladder
                    && !matches!(id, 0x37 | 0x38 | 0x28)
                    && !is_in_close_combat(id)
                    && !grounded
                {
                    return;
                }
            }
            0x39 => {
                // (unlike case 4, a jump id doesn't replace the super-jump pose)
                if !fin
                    && !matches!(id, 0x39 | 0x28 | 0x2f | 0x30 | 0x31 | 0x37 | 0x38 | 0x36 | 0x32)
                    && alive
                    && !grounded
                    && !is_in_close_combat(id)
                {
                    return;
                }
            }
            _ => {}
        }
    } else if !fin
        && !is_in_close_combat(id)
        && alive
        && !matches!(id, 0x28 | 0x2f | 0x30 | 0x36 | 0x32 | 0x31 | 0x37 | 0x38 | 0x39)
        && (!p.stunt.anim.cc_flag || !grounded)
    {
        // A melee move is playing: keep it, but let the legs follow if it allows that.
        if !gait_close_combat(cx, cur) {
            return;
        }
        cc_gait = true;
    }

    let mut id = id;
    if !cc_gait {
        if is_in_close_combat(id) {
            handle_close_combat(p, cx, id);
        } else if !torso(p, cx, &mut id) {
            return;
        }
    }

    if !cc_gait {
        p.stunt.anim.current = id;
    }
    gait(p, cx, id, speed);
}

/// The torso part of `Animate` (the big switch). Returns false where TS returns early.
fn torso(p: &mut Player, cx: &mut Cx, id: &mut i32) -> bool {
    let grounded = on_ground(cx);
    let iuser4 = cx.pev().iuser4;
    let ducking = cx.pev().flags & FL_DUCKING as i32 != 0;
    let fov = cx.pev().fov;
    let ext2 = p.stunt.anim.ext2.clone();
    match *id {
        // Fire (8/9: akimbo left/right).
        0 | 8 | 9 => {
            if !grounded {
                return false;
            }
            let side = match *id {
                0 => "",
                9 => "_right",
                _ => "_left",
            };
            if iuser4 & PRONE == 0 {
                p.stunt.anim.shot = 2.0;
                let mut name = format!("{}{}", if ducking { "crouch_shoot_" } else { "shoot_" }, ext2);
                if p.stunt.anim.hold > 0.001 && !ducking && fov == 0.0 && (ext2.starts_with("one") || ext2.starts_with("two")) {
                    name.push('2');
                    p.stunt.anim.shot = 0.0;
                }
                name.push_str(side);
                set_anim(p, cx, &name, 0x92, 0);
            } else {
                let name = format!("{}{}_shoot{}", slide_name(p, cx), ext2, side);
                set_anim(p, cx, &name, 0x94, 0);
            }
        }
        // Idle / moving refresh.
        1 | 6 => {
            if !grounded && !on_ladder(cx) {
                return false;
            }
            if !p.stunt.anim.ext.is_empty() {
                let ext = p.stunt.anim.ext.clone();
                set_anim(p, cx, &ext, 0x12, 0);
            } else if !ext2.is_empty() {
                let cur = p.stunt.anim.current;
                if matches!(cur, 0 | 8 | 9 | 0x34 | 0x33) && !p.stunt.anim.seq_finished {
                    *id = cur;
                } else {
                    let aim = p.stunt.anim.shot >= 0.01 || fov != 0.0;
                    let base = match (ducking, aim) {
                        (true, true) => "crouch_aim_",
                        (true, false) => "duck_idle_",
                        (false, true) => "aim_",
                        (false, false) => "idle_",
                    };
                    set_anim(p, cx, &format!("{base}{ext2}"), 0x12, 0);
                }
            }
        }
        // Lying.
        3 => {
            let cur = p.stunt.anim.current;
            if !matches!(cur, 0 | 8 | 9) || p.stunt.anim.seq_finished {
                let name = format!("{}{}", slide_name(p, cx), ext2_or_akimbo(p));
                set_anim(p, cx, &name, 0x94, 0);
            }
        }
        // Jump.
        4 => {
            if iuser4 & (PRONE | GO_DUCK) != 0 {
                return false;
            }
            let name = if ext2.is_empty() {
                "standard_jump".to_owned()
            } else {
                let d = get_direction(normalized(cx.pev().velocity), cx.pev().v_angle, 0.6, 0.3);
                p.stunt.anim.last_dir = d;
                format!("{}{}", JUMP[d as usize], ext2)
            };
            set_anim(p, cx, &name, 0x94, 0);
        }
        5 => {
            if !p.stunt.anim.duck_latch {
                set_anim(p, cx, "goduck", 0x94, 0);
                p.stunt.anim.duck_latch = true;
            }
        }
        7 => death(p, cx),
        // Dive start.
        0x28 => {
            let d = get_direction(normalized(cx.pev().velocity), cx.pev().v_angle, 0.6, 0.3);
            p.stunt.anim.last_dir = d;
            let name = format!("{}{}", DIVE[d as usize], ext2_or_akimbo(p));
            set_anim(p, cx, &name, 0x94, 0);
        }
        // Get up.
        0x29 => {
            let name = SLIDE_END[p.stunt.anim.last_dir.clamp(0, 3) as usize];
            set_anim(p, cx, name, 0x94, 0);
            p.stunt.anim.hold = 3.0;
        }
        // Prone side roll.
        0x2a | 0x2b => {
            let left = *id == 0x2a;
            let name = if !p.stunt.dir_not_forward {
                format!("slide_f_{}{}", ext2_or_akimbo(p), if left { "_left" } else { "_right" })
            } else {
                (if left { "slide_b_left" } else { "slide_b_right" }).to_owned()
            };
            set_anim(p, cx, &name, 0x14, 0);
        }
        0x2e..=0x32 => {
            let d = get_direction(p.stunt.dir, cx.pev().v_angle, 0.6, 0.3) as usize;
            let (name, hold) = match *id {
                0x2e => (DIVE_END[d], true),
                0x2f => (ROLL[d], true),
                0x30 => (ROLL_HIGH[d], false),
                0x31 => (["wheel_l", "wheel_r", "wheel_f", "wheel_b"][d], true),
                _ => (["lowwheel_l", "lowwheel_r", "lowwheel_f", "lowwheel_r"][d], true),
            };
            set_anim(p, cx, name, 0x94, 0);
            if hold {
                p.stunt.anim.hold = 3.0;
            }
        }
        // Reload / draw (torso only, standing).
        0x33 | 0x34 => {
            if !grounded || iuser4 & (DIVING | PRONE) != 0 || ext2.is_empty() {
                return false;
            }
            let name = format!("{}{}", if *id == 0x33 { "relo_" } else { "draw_" }, ext2);
            set_anim(p, cx, &name, 0x92, 0);
            p.stunt.anim.shot = 2.0;
        }
        0x35 => {
            if !grounded || iuser4 & (DIVING | PRONE) != 0 {
                return false;
            }
            set_anim(p, cx, "grenade", 0x92, 0);
            p.stunt.anim.shot = 2.0;
            p.stunt.anim.grenade = 1.0;
        }
        0x36 => {
            set_anim(p, cx, "bullettime", 0x94, 0);
        }
        0x37 | 0x38 => {
            set_anim(p, cx, if *id == 0x37 { "walljump_end" } else { "walljump_climb" }, 0x94, 0);
            p.stunt.anim.hold = 3.0;
        }
        // Super-jump air pose.
        0x39 => {
            if iuser4 & (PRONE | GO_DUCK) != 0 {
                return false;
            }
            set_anim(p, cx, if ext2.is_empty() { "morfy_jump" } else { "morfy_jump_guns" }, 0x94, 0);
        }
        _ => {}
    }
    true
}

/// Death pose (`Animate(7)`).
fn death(p: &mut Player, cx: &mut Cx) {
    if p.stunt.anim.bullet_dance {
        let name = match engine().random_long(0, 2) {
            0 => "normal_die1",
            1 => "normal_die2",
            _ => "normal_die3",
        };
        set_anim(p, cx, name, 0x14, 0);
        return;
    }
    let v = cx.pev().velocity;
    let a = &p.stunt.anim;
    let name = if hspeed2(v) >= 80.0 || a.last_damage > 50 {
        let dir = if a.last_damage <= 50 { normalized(v) } else { [-a.attack_dir[0], -a.attack_dir[1], -a.attack_dir[2]] };
        match get_direction(dir, cx.pev().v_angle, 0.6, 0.3) {
            2 => "forward_die",
            3 => {
                if a.last_hitgroup == 1 && engine().random_long(0, 100) > 30 {
                    "die_spinback"
                } else if engine().random_long(0, 1) == 0 {
                    "leg_die"
                } else {
                    "die_back"
                }
            }
            _ => "die_spin",
        }
    } else {
        "die_spin"
    };
    set_anim(p, cx, name, 0x14, 0);
}

/// The leg animation at the end of `Animate`.
fn gait(p: &mut Player, cx: &mut Cx, id: i32, speed: f32) {
    if matches!(id, 4 | 0x39 | 5 | 7 | 0x29 | 0x2e | 0x2f | 0x30 | 0x36) {
        return;
    }
    if is_in_close_combat(id) && !gait_close_combat(cx, id) {
        return;
    }
    if cx.pev().iuser4 & 0x30c30 != 0 {
        return;
    }
    let a = &mut p.stunt.anim;
    let name: String = if cx.pev().flags & FL_DUCKING as i32 != 0 {
        a.duck_latch = false;
        (if speed == 0.0 { "crouch_idle" } else { "crawl" }).into()
    } else if speed > 180.0 {
        "run2".into()
    } else if speed > 0.0 {
        "walk2".into()
    } else if !a.ext.is_empty() {
        a.ext.clone()
    } else if !a.ext2.is_empty() {
        format!("idle_{}", a.ext2)
    } else if a.shot >= 0.01 {
        "combat_idle".into()
    } else {
        "combat_idle_low".into()
    };
    let gaitseq = lookup(cx, &name);
    cx.pev().gaitsequence = gaitseq;
    // No weapon: the whole body follows the legs.
    let a = &p.stunt.anim;
    if a.ext.is_empty() && a.ext2.is_empty() && !gait_close_combat(cx, id) && (a.shot < 0.01 || a.seq_finished) {
        let pev = cx.pev();
        if pev.sequence != pev.gaitsequence {
            pev.sequence = pev.gaitsequence;
            pev.frame = 0.0;
            reset_sequence_info(p, cx);
        }
    }
}
