//! `CTSAnimation` (decomp `server/decomp/CTSAnimation.c`, 0x6b0c8-0x6ddc6): picks the torso
//! (`sequence`) and leg (`gaitsequence`) animations from the stunt state, the weapon animation
//! extensions and the player's speed. Sequence names are looked up in the player model through
//! the studio header (`goldsrc::ext::movement`).
//!
//! The animation `framerate` is the player's time scale (set every PreThink), and
//! `studio_frame_advance` (SDK `CBaseAnimating::StudioFrameAdvance`) runs in PostThink, and in
//! `PlayerDeathThink` while dying, to keep `m_fSequenceFinished`: stunts use it to end recoveries
//! early, and the death think to end the dying state.

use super::rules::{FirePose2, StuntRules};
use super::*;
use goldsrc::ext::movement::{self as studio, STUDIO_LOOPING};

/// The active ruleset's stunt values (the animation rules among them).
fn stunt_rules() -> &'static StuntRules {
    crate::gameplay::active().server.stunts()
}

/// `Animate`'s velocity direction (the in-air jump blend, the jump pose, the dive start, the
/// death pose): [`normalized`], or with [`StuntRules::anim_normalize_stores_reciprocal`] under
/// `mp.dll`'s arithmetic, [`normalize_stored_reciprocal`].
fn anim_dir(v: Vec3) -> Vec3 {
    if stunt_rules().anim_normalize_stores_reciprocal && crate::rules::cvars::pm_arith() == shared::pm::Arith::Msvc {
        normalize_stored_reciprocal(v)
    } else {
        normalized(v)
    }
}

/// An out-of-line `Vector::Normalize()` that stores the reciprocal (TS 2.1 `mp.dll` FUN_10011fc0,
/// 0x10011fc0..0x10012033): `len = sqrt((x² + y²) + z²)` unrounded; a length equal to 0 (IEEE:
/// `test ah,0x44; jp`, so a NaN goes on) gives (0, 0, 1); otherwise `1 / len` is stored as a
/// `float` and each component is that `float` times the component, stored.
fn normalize_stored_reciprocal(v: Vec3) -> Vec3 {
    let [x, y, z] = v.map(f64::from);
    let len = (x * x + y * y + z * z).sqrt();
    if len == 0.0 {
        return [0.0, 0.0, 1.0];
    }
    let inv = (1.0 / len) as f32;
    v.map(|c| inv * c)
}

/// A `CTSAnimation` timer of `secs` of the player's time: TS 3.0 stores the seconds and counts
/// them down in [`update_timers`]; with [`StuntRules::anim_end_times`] the ruleset's
/// [`StuntClock`](super::rules::StuntClock) gives the end time.
fn anim_timer(cx: &mut Cx, secs: f64) -> f32 {
    let (now, scale) = (cx.time(), cx.pev().fuser1);
    crate::gameplay::active().server.stunt_clock().start(now, scale, secs)
}

/// The "recently shot" timer for the torso's aim poses: TS 3.0 `shot >= 0.01`; end times
/// `!(shot < time)`, so a NaN aims (2.1 mp.dll 0x1003ea36: `fcomp; test ah,0x5; jnp` idle).
fn aiming(end_times: bool, now: f32, shot: f32) -> bool {
    if end_times { shot.partial_cmp(&now) != Some(std::cmp::Ordering::Less) } else { shot >= 0.01 }
}

/// The "recently shot" timer for the legs' `combat_idle`: TS 3.0 `shot >= 0.01`; end times
/// `!(shot <= time)`, so a NaN counts as running (2.1 0x1003ebce: `test ah,0x41; jnp` low).
fn shot_running(end_times: bool, now: f32, shot: f32) -> bool {
    if end_times { !matches!(shot.partial_cmp(&now), Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)) } else { shot >= 0.01 }
}

/// `Animate`'s fire case: whether the standing shot plays its `2` pose, and whether that zeroes
/// the shot timer ([`FirePose2`]). `speed` is `Animate`'s stored horizontal speed.
fn fire_pose_2(rule: FirePose2, ext2: &str, ducking: bool, hold: f32, fov: f32, speed: f32) -> (bool, bool) {
    match rule {
        FirePose2::AfterHold => {
            let two = hold > 0.001 && !ducking && fov == 0.0 && (ext2.starts_with("one") || ext2.starts_with("two"));
            (two, two)
        }
        // 2.1 mp.dll 0x1003e8f2: `fcomp 180; test ah,0x5; jp` skips at or above 180 and on a NaN.
        FirePose2::SlowOneHanded => (ext2.starts_with("one") && speed < 180.0 && !ducking, false),
    }
}

/// Arms the hold timer (`+0x08`, 3 s) where TS 3.0 does; it exists only for
/// [`FirePose2::AfterHold`].
fn arm_hold(p: &mut Player) {
    if stunt_rules().fire_pose_2 == FirePose2::AfterHold {
        p.stunt.anim.hold = 3.0;
    }
}

/// The "recently shot" timer is over for the whole-body follow: TS 3.0 `shot < 0.01`; end times
/// `shot < time || shot == time` (a NaN is neither).
fn shot_over(end_times: bool, now: f32, shot: f32) -> bool {
    if end_times { shot <= now } else { shot < 0.01 }
}

/// `BulletDance`'s timers allow a twitch: TS 3.0 `!(time_left < 0.01) && !(next > 0)`; end times
/// `time <= until && next <= time`.
fn dance_due(end_times: bool, now: f32, next: f32, until: f32) -> bool {
    use std::cmp::Ordering::{Greater, Less};
    if end_times { now <= until && next <= now } else { until.partial_cmp(&0.01) != Some(Less) && next.partial_cmp(&0.0) != Some(Greater) }
}

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
    /// `m_flLastEventCheck` (+0x74): `ResetSequenceInfo` sets it; saved with the player.
    pub last_event_check: f32,
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
            last_event_check: 0.0,
        }
    }
}

fn lookup(cx: &mut Cx, name: &str) -> i32 {
    studio::lookup_sequence(cx.edict, name)
}

/// `CBaseAnimating::ResetSequenceInfo` (0xa37c4).
pub fn reset_sequence_info(p: &mut Player, cx: &mut Cx) {
    let seq = cx.pev().sequence;
    let hdr = studio::model_ptr(cx.edict);
    let info = hdr.as_ref().and_then(|h| h.sequence_info(seq));
    let a = &mut p.stunt.anim;
    match info {
        Some(i) => {
            (a.frame_rate, a.ground_speed) = i.rates();
            a.seq_loops = i.flags & STUDIO_LOOPING != 0;
        }
        // SDK `GetSequenceInfo`: without studio data it returns before writing the rates (they
        // keep their old values); a sequence past the model's last one gets 0. (A negative one
        // would make TS read before the sequence table; the port treats it as past the end.)
        None if hdr.is_some() => {
            (a.frame_rate, a.ground_speed) = (0.0, 0.0);
            a.seq_loops = false;
        }
        None => a.seq_loops = false,
    }
    let time = cx.time();
    let pev = cx.pev();
    pev.animtime = time;
    pev.framerate = 1.0;
    a.seq_finished = false;
    a.last_event_check = time;
}

/// SDK `CBaseAnimating::StudioFrameAdvance(0)`, in the server's arithmetic
/// ([`frame_advance`]).
pub fn studio_frame_advance(p: &mut Player, cx: &mut Cx) {
    let time = cx.time();
    let a = &mut p.stunt.anim;
    let pev = cx.pev();
    let step = FrameStep {
        interval: 0.0,
        time,
        animtime: pev.animtime,
        frame: pev.frame,
        frame_rate: a.frame_rate,
        framerate: pev.framerate,
        loops: a.seq_loops,
    };
    let (frame, finished) = frame_advance_in(crate::rules::cvars::pm_arith(), step);
    pev.animtime = time;
    if let Some(frame) = frame {
        pev.frame = frame;
        if finished {
            a.seq_finished = true;
        }
    }
}

/// [`frame_advance`]'s inputs: the `flInterval` argument (0: since `animtime`),
/// `gpGlobals->time`, `pev->animtime`, `pev->frame`, `m_flFrameRate`, `pev->framerate` and
/// `m_fSequenceLoops`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FrameStep {
    pub interval: f32,
    pub time: f32,
    pub animtime: f32,
    pub frame: f32,
    pub frame_rate: f32,
    pub framerate: f32,
    pub loops: bool,
}

/// [`frame_advance`] in `arith` (the player's and the stock entities' `StudioFrameAdvance`).
pub(crate) fn frame_advance_in(arith: shared::pm::Arith, s: FrameStep) -> (Option<f32>, bool) {
    shared::by_arith!(arith, frame_advance(s))
}

/// `StudioFrameAdvance(flInterval)` as each TS build compiles it (3.0 `mp.dll` 0x10008290, 2.1
/// `mp.dll` 0x10006090 the same; `ts_i386.so` 0x935a0): the new `pev->frame` (`None`: the
/// interval is at most 0.001, only `animtime` moves on) and whether the sequence finished. An
/// `flInterval` of 0 (C3: a NaN too) is `time - animtime`, kept on the x87 (compared with the
/// `double` 0.001; a NaN returns too); the interval is 0 when `animtime == 0` (C3 again). `frame += interval * m_flFrameRate * framerate` is
/// rounded once (`mp.dll` multiplies `framerate * m_flFrameRate * interval`, 0x100082f6..
/// 0x10008308; the .so `interval * m_flFrameRate * framerate`, 0x93621..0x93632). Outside
/// `[0, 256)` (C0 tests, so a NaN frame too) the sequence finishes: a looping one subtracts
/// `(int)(frame * (1/256)) * 256` (`_ftol` / a truncating `fistp`), another clamps to 0 or 255.
fn frame_advance<F: Fpu>(s: FrameStep) -> (Option<f32>, bool) {
    let mut interval = F::x(s.interval);
    if shared::pm::eq87(s.interval, 0.0) {
        interval = F::x(s.time) - F::x(s.animtime);
        if !(interval > F::xd(0.001)) {
            return (None, false);
        }
    }
    let interval = if shared::pm::eq87(s.animtime, 0.0) { F::ZERO } else { interval };
    let step = if F::MSVC { F::x(s.framerate) * F::x(s.frame_rate) * interval } else { interval * F::x(s.frame_rate) * F::x(s.framerate) };
    let frame = (step + F::x(s.frame)).f();
    if !below87(frame, 0.0) && below87(frame, 256.0) {
        return (Some(frame), false);
    }
    let frame = if s.loops {
        let n = (F::x(frame) * F::xd(1.0 / 256.0)).trunc_i32();
        (F::x(frame) - F::xi(n) * F::xd(256.0)).f()
    } else if below87(frame, 0.0) {
        0.0
    } else {
        255.0
    };
    (Some(frame), true)
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

/// `CTSAnimation::UpdateTimers` (0x6b37c), in slowed time. With
/// [`StuntRules::anim_end_times`] only the hold timer counts down.
pub fn update_timers(p: &mut Player, cx: &mut Cx) {
    let dt = slow_mul(cx) * frametime();
    let a = &mut p.stunt.anim;
    if stunt_rules().anim_end_times {
        a.hold = if a.hold >= -0.001 { (a.hold - dt).max(0.0) } else { (dt + a.hold).min(0.0) };
        return;
    }
    a.dance_time = (a.dance_time - dt).max(0.0);
    a.dance_next = (a.dance_next - dt).max(0.0);
    a.shot = (a.shot - dt).max(0.0);
    a.grenade = (a.grenade - dt).max(0.0);
    a.hold = if a.hold >= -0.001 { (a.hold - dt).max(0.0) } else { (dt + a.hold).min(0.0) };
}

/// `CTSAnimation::EnableBulletDance` (0x6d314), from `CBasePlayer::TakeDamage`.
pub fn enable_bullet_dance(p: &mut Player, cx: &mut Cx) {
    let (next, until) = (anim_timer(cx, 0.15), anim_timer(cx, 3.0));
    let a = &mut p.stunt.anim;
    a.bullet_dance = true;
    a.dance_next = next;
    a.dance_time = until;
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
    if !a.bullet_dance || !dance_due(stunt_rules().anim_end_times, cx.time(), a.dance_next, a.dance_time) || a.seq_finished {
        return;
    }
    p.stunt.anim.dance_next = anim_timer(cx, 0.15);
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
    let (ducking, duck_held) = (pev.flags & FL_DUCKING as i32 != 0, pev.button & IN_DUCK as i32 != 0);
    melee_legs_follow(id, ducking, duck_held, pev.velocity, stunt_rules().melee_gait_speed)
}

/// [`gait_close_combat`] on its inputs: the moves that may, then ducking or the unrounded
/// horizontal speed above `speed` ([`StuntRules::melee_gait_speed`]).
fn melee_legs_follow(id: i32, ducking: bool, duck_held: bool, velocity: Vec3, speed: f32) -> bool {
    if id == 0x25 && (ducking || duck_held) {
        return false;
    }
    let kinds =
        (0x16..=0x17).contains(&id) || (0x21..=0x25).contains(&id) || id == 0x19 || id == 0x26 || id == 0x27 || (0x1d..=0x20).contains(&id);
    if id == 0x24 || id == 0x23 || id == 0x3a || !kinds {
        return false;
    }
    ducking || hspeed_cmp(velocity, speed, true)
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
    let shot = anim_timer(cx, 2.0);
    p.stunt.anim.shot = shot;
    arm_hold(p);
    p.stunt.anim.cc_flag = false;
}

/// `Animate`'s case 0x37 (the wall-jump pose): whether it refuses `id`. TS 3.0 holds it while
/// unfinished, alive, `GO_DUCK`, off ladders and in the air, against everything but the wall poses
/// and the dive (0x28). With
/// [`StuntRules::wall_jump_pose_holds_until_recovered`](super::rules::StuntRules::wall_jump_pose_holds_until_recovered)
/// it holds on the ground too and against a dive, so after a wall-jump landing the pose plays on
/// (and its end, `m_fSequenceFinished`, can end the recovery) until `GO_DUCK` goes with the
/// recovery.
fn wall_jump_pose_holds(until_recovered: bool, fin: bool, alive: bool, go_duck: bool, ladder: bool, grounded: bool, id: i32) -> bool {
    let let_through = if until_recovered { matches!(id, 0x37 | 0x38) } else { matches!(id, 0x37 | 0x38 | 0x28) || grounded };
    !fin && alive && go_duck && !ladder && !let_through
}

/// `Animate`'s `goduck` case ([`StuntRules::goduck_holds`]): the pose holds while unfinished,
/// alive, duck held and not yet ducking, against every id but a roll (0x2f), a dive (0x28), a jump
/// (4), the super jump (0x39), the duck kick (0x14) and `katana_duck` (0x25).
fn goduck_holds(fin: bool, alive: bool, button: i32, flags: i32, id: i32) -> bool {
    !fin && button & IN_DUCK as i32 != 0 && flags & FL_DUCKING as i32 == 0 && !matches!(id, 0x2f | 0x28 | 4 | 0x39 | 0x14 | 0x25) && alive
}

/// The wall-jump pose's test in `Animate` (0x6bb15..0x6bc57; mp.dll 0x10044faa..0x100450dd): the
/// velocity is zero, or its direction isn't square to `right` (`v.Normalize() · right.Normalize()
/// != 0`). The .so stores both lengths and every component and sums `x, y, z`. mp.dll tests the
/// unrounded length, then normalises `right` with x left on the FPU, the velocity by its *stored*
/// length with x left on the FPU, and sums `(x + z) + y`. A NaN dot counts as zero (`test ah,0x40`).
pub(super) fn moving_off_square<F: Fpu>(v: Vec3, right: Vec3) -> bool {
    if eq87(F::vec_length(v, ORDER_XYZ), F::ZERO) {
        return true;
    }
    let dot = if F::MSVC {
        let [bx, by, bz] = F::vec_normalize_n(right, ORDER_XYZ);
        let len = F::sq_in(v, ORDER_XYZ).sqrt().f();
        let [ax, ay, az] = if eq87(len, 0.0) {
            [F::ZERO, F::ZERO, F::x(1.0)]
        } else {
            let inv = F::x(1.0) / F::x(len);
            [inv * F::x(v[0]), F::x((inv * F::x(v[1])).f()), F::x((inv * F::x(v[2])).f())]
        };
        (ax * bx + az * F::x(bz.f())) + ay * F::x(by.f())
    } else {
        let (a, b) = (F::vec_normalize(v, ORDER_XYZ), F::vec_normalize(right, ORDER_XYZ));
        F::x(a[0]) * F::x(b[0]) + F::x(a[1]) * F::x(b[1]) + F::x(a[2]) * F::x(b[2])
    };
    !eq87(dot, F::ZERO)
}

/// `CTSAnimation::Animate(player, id)` (0x6b470).
pub fn animate(p: &mut Player, cx: &mut Cx, id: i32) {
    if id < 0 {
        return;
    }
    let v = cx.pev().velocity;
    let speed = hspeed_stored(v);
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
            blend_by_dir(p, cx, anim_dir(v), &JUMP, &ext2);
        }
        if id == 2 {
            let dir = p.stunt.dir;
            let ext = ext2_or_akimbo(p);
            blend_by_dir(p, cx, dir, &DIVE, &ext);
        }
    }

    // Priority: a playing stunt animation refuses lower-priority ids until it finishes.
    let rules = stunt_rules();
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
            5 if rules.goduck_holds => {
                if goduck_holds(fin, alive, cx.pev().button, cx.pev().flags, id) {
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
                let check = if rules.side_roll_holds_on_strafe_keys {
                    cx.pev().button & (IN_MOVELEFT | IN_MOVERIGHT) as i32 != 0
                } else {
                    let (_, right, _) = shared::pm::angle_vectors(crate::rules::cvars::pm_arith(), cx.pev().v_angle);
                    shared::by_arith!(crate::rules::cvars::pm_arith(), moving_off_square(v, right))
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
                let until_recovered = crate::gameplay::active().server.stunts().wall_jump_pose_holds_until_recovered;
                if wall_jump_pose_holds(until_recovered, fin, alive, iuser4 & GO_DUCK != 0, ladder, grounded, id) {
                    return;
                }
            }
            0x38 => {
                let dive_through = rules.wall_climb_pose_lets_dive_through && id == 0x28;
                if !fin
                    && alive
                    && iuser4 & GO_DUCK != 0
                    && !ladder
                    && !matches!(id, 0x37 | 0x38)
                    && !dive_through
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
    } else if !fin && !is_in_close_combat(id) && alive && !matches!(id, 0x28 | 0x2f | 0x30 | 0x36 | 0x32 | 0x31 | 0x37 | 0x38 | 0x39) {
        // A melee move is playing: keep it, but let the legs follow if it allows that.
        if !rules.melee_gait_follows {
            return;
        }
        if !p.stunt.anim.cc_flag || !grounded {
            if !gait_close_combat(cx, cur) {
                return;
            }
            cc_gait = true;
        }
    }

    let mut id = id;
    if !cc_gait {
        if is_in_close_combat(id) {
            handle_close_combat(p, cx, id);
        } else if !torso(p, cx, &mut id, speed) {
            return;
        }
    }

    if !cc_gait {
        p.stunt.anim.current = id;
    }
    gait(p, cx, id, speed);
}

/// The torso part of `Animate` (the big switch). Returns false where TS returns early.
fn torso(p: &mut Player, cx: &mut Cx, id: &mut i32, speed: f32) -> bool {
    let rules = stunt_rules();
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
                p.stunt.anim.shot = anim_timer(cx, 2.0);
                let mut name = format!("{}{}", if ducking { "crouch_shoot_" } else { "shoot_" }, ext2);
                let (two, zero_shot) = fire_pose_2(rules.fire_pose_2, &ext2, ducking, p.stunt.anim.hold, fov, speed);
                if two {
                    name.push('2');
                }
                if zero_shot {
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
            if rules.idle_anim_extension && !p.stunt.anim.ext.is_empty() {
                let ext = p.stunt.anim.ext.clone();
                set_anim(p, cx, &ext, 0x12, 0);
            } else if !ext2.is_empty() {
                let cur = p.stunt.anim.current;
                if matches!(cur, 0 | 8 | 9 | 0x34 | 0x33) && !p.stunt.anim.seq_finished {
                    *id = cur;
                } else {
                    let aim = aiming(rules.anim_end_times, cx.time(), p.stunt.anim.shot) || (rules.aim_pose_while_zoomed && fov != 0.0);
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
                let d = get_direction(anim_dir(cx.pev().velocity), cx.pev().v_angle, 0.6, 0.3);
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
            let d = get_direction(anim_dir(cx.pev().velocity), cx.pev().v_angle, 0.6, 0.3);
            p.stunt.anim.last_dir = d;
            let name = format!("{}{}", DIVE[d as usize], ext2_or_akimbo(p));
            set_anim(p, cx, &name, 0x94, 0);
        }
        // Get up.
        0x29 => {
            let name = SLIDE_END[p.stunt.anim.last_dir.clamp(0, 3) as usize];
            set_anim(p, cx, name, 0x94, 0);
            arm_hold(p);
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
                arm_hold(p);
            }
        }
        // Reload / draw (torso only, standing).
        0x33 | 0x34 => {
            if !grounded || iuser4 & (DIVING | PRONE) != 0 || ext2.is_empty() {
                return false;
            }
            let name = format!("{}{}", if *id == 0x33 { "relo_" } else { "draw_" }, ext2);
            set_anim(p, cx, &name, 0x92, 0);
            p.stunt.anim.shot = anim_timer(cx, 2.0);
        }
        0x35 => {
            if !grounded || iuser4 & (DIVING | PRONE) != 0 {
                return false;
            }
            set_anim(p, cx, "grenade", 0x92, 0);
            p.stunt.anim.shot = anim_timer(cx, 2.0);
            p.stunt.anim.grenade = anim_timer(cx, 1.0);
        }
        0x36 => {
            set_anim(p, cx, "bullettime", 0x94, 0);
        }
        0x37 | 0x38 => {
            set_anim(p, cx, if *id == 0x37 { "walljump_end" } else { "walljump_climb" }, 0x94, 0);
            arm_hold(p);
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
    let name = if hspeed_stored(v) >= 80.0 || a.last_damage > 50 {
        let dir = if a.last_damage <= 50 { anim_dir(v) } else { [-a.attack_dir[0], -a.attack_dir[1], -a.attack_dir[2]] };
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
    let r = stunt_rules();
    let now = cx.time();
    let a = &mut p.stunt.anim;
    let name: String = if cx.pev().flags & FL_DUCKING as i32 != 0 {
        a.duck_latch = false;
        (if speed == 0.0 { "crouch_idle" } else { "crawl" }).into()
    } else if speed > 180.0 {
        "run2".into()
    } else if speed > 0.0 {
        "walk2".into()
    } else if r.gait_idle_anim_extension && !a.ext.is_empty() {
        a.ext.clone()
    } else if !a.ext2.is_empty() {
        format!("idle_{}", a.ext2)
    } else if shot_running(r.anim_end_times, now, a.shot) {
        "combat_idle".into()
    } else {
        "combat_idle_low".into()
    };
    let gaitseq = lookup(cx, &name);
    cx.pev().gaitsequence = gaitseq;
    // No weapon: the whole body follows the legs.
    let a = &p.stunt.anim;
    let ext_free = a.ext.is_empty() || !r.gait_idle_anim_extension;
    if ext_free && a.ext2.is_empty() && !gait_close_combat(cx, id) && (shot_over(r.anim_end_times, now, a.shot) || a.seq_finished) {
        let pev = cx.pev();
        if pev.sequence != pev.gaitsequence {
            pev.sequence = pev.gaitsequence;
            pev.frame = 0.0;
            reset_sequence_info(p, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{aiming, dance_due, fire_pose_2, goduck_holds, melee_legs_follow, shot_over, shot_running, wall_jump_pose_holds};
    use crate::stunt::rules::FirePose2;
    use goldsrc::sys::{FL_DUCKING, IN_DUCK};

    /// #222: TS 2.1's `Animate` normalises with FUN_10011fc0, which stores `1 / len` as a float
    /// (0x10012014) before the products; 3.0's (0x100148f0, `normalized`) keeps it unrounded.
    #[test]
    fn normalize_stored_reciprocal_rounds_the_reciprocal() {
        use super::normalize_stored_reciprocal;
        let v = [0x4427_f24a, 0x431b_b090, 0xc43d_9372].map(f32::from_bits);
        assert_eq!(normalize_stored_reciprocal(v).map(f32::to_bits), [0x3f27_c9ac, 0x3e1b_8ae9, 0xbf3d_6599]);
        let three: [f32; 3] = <shared::pm::Msvc as shared::pm::FpuMath>::vec_normalize(v, shared::pm::ORDER_XYZ);
        assert_eq!(three.map(f32::to_bits), [0x3f27_c9ac, 0x3e1b_8ae8, 0xbf3d_6598], "3.0's");
        assert_eq!(normalize_stored_reciprocal([0.0; 3]), [0.0, 0.0, 1.0]);
        assert!(normalize_stored_reciprocal([f32::NAN, 0.0, 0.0]).iter().all(|c| c.is_nan()), "IEEE zero test");
    }

    /// #243: `frame += interval * m_flFrameRate * framerate` rounded once (mp.dll 0x100082f6..
    /// 0x10008308, ts_i386.so 0x93621..0x93632); the port's old `float` steps gave 0x4316a7fe.
    #[test]
    fn frame_advance_rounds_once() {
        use super::{FrameStep, frame_advance};
        use shared::pm::{Gcc, Msvc};
        let b = f32::from_bits;
        let s = FrameStep {
            interval: 0.0,
            time: b(0x4345_163b),
            animtime: b(0x4345_0aaa),
            frame: b(0x4316_2e8b),
            frame_rate: 30.0,
            framerate: b(0x3eb3_3333),
            loops: true,
        };
        assert_eq!(frame_advance::<Msvc>(s).0.map(f32::to_bits), Some(0x4316_a7fd));
        assert_eq!(frame_advance::<Gcc>(s).0.map(f32::to_bits), Some(0x4316_a7fd));
        // The interval is compared unrounded with the double 0.001: 10.001f - 10 = 0.0010004.
        let s = FrameStep { time: 10.001, animtime: 10.0, ..s };
        assert!(frame_advance::<Msvc>(s).0.is_some());
        assert_eq!(frame_advance::<Msvc>(FrameStep { time: 10.0009985, ..s }), (None, false));
        // animtime 0: no step.
        assert_eq!(frame_advance::<Msvc>(FrameStep { animtime: 0.0, ..s }), (Some(s.frame), false));
        // Wrapping: a loop subtracts whole 256s, another clamps; a NaN frame finishes too.
        let s = FrameStep { interval: 0.0, time: 11.0, animtime: 10.0, frame: 250.0, frame_rate: 10.0, framerate: 1.0, loops: true };
        assert_eq!(frame_advance::<Msvc>(s), (Some(4.0), true));
        assert_eq!(frame_advance::<Gcc>(s), (Some(4.0), true));
        assert_eq!(frame_advance::<Msvc>(FrameStep { loops: false, ..s }), (Some(255.0), true));
        assert_eq!(frame_advance::<Msvc>(FrameStep { frame: -20.0, loops: false, ..s }), (Some(0.0), true));
        let (f, done) = frame_advance::<Msvc>(FrameStep { frame: f32::NAN, ..s });
        assert!(f.unwrap().is_nan() && done);
        // A given interval: no 0.001 test.
        assert_eq!(frame_advance::<Msvc>(FrameStep { interval: 0.0005, frame: 1.0, ..s }), (Some(1.005), false));
    }

    /// #174: TS 3.0's `2` shot pose follows the hold timer (unzoomed, one- or two-handed) and zeroes
    /// the shot timer; the slow one-handed rule reads the speed instead and keeps the shot timer.
    #[test]
    fn fire_pose_two() {
        let after = |ext, ducking, hold, fov, speed| fire_pose_2(FirePose2::AfterHold, ext, ducking, hold, fov, speed);
        assert_eq!(after("onehanded", false, 2.0, 0.0, 300.0), (true, true));
        assert_eq!(after("twohanded", false, 0.0011, 0.0, 0.0), (true, true));
        assert_eq!(after("onehanded", false, 0.001, 0.0, 0.0), (false, false), "hold over");
        assert_eq!(after("onehanded", false, 2.0, 40.0, 0.0), (false, false), "zoomed");
        assert_eq!(after("onehanded", true, 2.0, 0.0, 0.0), (false, false), "ducking");
        assert_eq!(after("akimbo", false, 2.0, 0.0, 0.0), (false, false));

        let slow = |ext, ducking, hold, fov, speed| fire_pose_2(FirePose2::SlowOneHanded, ext, ducking, hold, fov, speed);
        assert_eq!(slow("onehanded", false, 0.0, 40.0, 179.9), (true, false), "no hold, zoom ignored");
        assert_eq!(slow("onehanded_case", false, 0.0, 0.0, 0.0), (true, false));
        assert_eq!(slow("onehanded", false, 3.0, 0.0, 180.0), (false, false), "at 180");
        assert_eq!(slow("onehanded", false, 0.0, 0.0, f32::NAN), (false, false), "NaN speed");
        assert_eq!(slow("twohanded", false, 3.0, 0.0, 0.0), (false, false), "one-handed only");
        assert_eq!(slow("onehanded", true, 0.0, 0.0, 0.0), (false, false), "ducking");
    }

    /// #174 (sweep): a melee move's legs follow above 120 u/s in TS 3.0; the speed is the ruleset's
    /// (80 in the other), strictly above and on the unrounded length.
    #[test]
    fn melee_legs_speed() {
        let punch = |v: [f32; 3], limit| melee_legs_follow(0x16, false, false, v, limit);
        assert!(!punch([100.0, 0.0, 0.0], 120.0) && punch([100.0, 0.0, 0.0], 80.0));
        assert!(!punch([80.0, 0.0, 0.0], 80.0) && punch([80.0, 0.01, 0.0], 80.0));
        assert!(!punch([120.0, 0.0, 0.0], 120.0) && punch([0.0, 121.0, 0.0], 120.0));
        assert!(!punch([f32::NAN, 0.0, 0.0], 80.0), "a NaN isn't above");
        assert!(melee_legs_follow(0x16, true, false, [0.0; 3], 120.0), "ducking");
        assert!(!melee_legs_follow(0x25, false, true, [200.0, 0.0, 0.0], 80.0), "katana_duck with duck held");
        assert!(!melee_legs_follow(0x24, false, false, [200.0, 0.0, 0.0], 80.0), "katana_air");
        assert!(!melee_legs_follow(0x18, false, false, [200.0, 0.0, 0.0], 80.0), "punchair");
    }

    /// #112: the `goduck` hold (a ruleset's), against everything but its exceptions.
    #[test]
    fn goduck_hold() {
        let duck = IN_DUCK as i32;
        assert!(goduck_holds(false, true, duck, 0, 1));
        assert!(goduck_holds(false, true, duck, 0, 0x2a));
        for id in [0x2f, 0x28, 4, 0x39, 0x14, 0x25] {
            assert!(!goduck_holds(false, true, duck, 0, id), "{id:#x}");
        }
        assert!(!goduck_holds(true, true, duck, 0, 1), "finished");
        assert!(!goduck_holds(false, true, 0, 0, 1), "duck released");
        assert!(!goduck_holds(false, true, duck, FL_DUCKING as i32, 1), "already ducking");
        assert!(!goduck_holds(false, false, duck, 0, 1), "dead");
    }

    /// #112: 3.0's countdown thresholds and the end-time tests.
    #[test]
    fn anim_timers() {
        // 3.0: seconds left.
        assert!(aiming(false, 50.0, 0.01) && !aiming(false, 50.0, 0.009));
        assert!(shot_running(false, 50.0, 0.01) && shot_over(false, 50.0, 0.009));
        assert!(dance_due(false, 50.0, 0.0, 0.01) && !dance_due(false, 50.0, 0.001, 3.0) && !dance_due(false, 50.0, 0.0, 0.009));
        // End times: aiming includes the end, the legs' timer doesn't.
        assert!(aiming(true, 50.0, 50.0) && !aiming(true, 50.0, 49.9));
        assert!(!shot_running(true, 50.0, 50.0) && shot_running(true, 50.0, 50.1));
        assert!(shot_over(true, 50.0, 50.0) && !shot_over(true, 50.0, 50.1));
        assert!(!shot_over(true, 50.0, f32::NAN), "a NaN is neither");
        // #174 (sweep): the x87 tests take a NaN as aiming and as running.
        assert!(aiming(true, 50.0, f32::NAN) && shot_running(true, 50.0, f32::NAN));
        assert!(!aiming(false, 50.0, f32::NAN) && !shot_running(false, 50.0, f32::NAN), "3.0's countdown unchanged");
        assert!(dance_due(true, 50.0, 50.0, 50.0) && !dance_due(true, 50.0, 50.1, 53.0) && !dance_due(true, 50.0, 49.0, 49.9));
    }

    /// #112 (the wall-landing part): 3.0 drops the wall-jump pose on landing; a ruleset that holds
    /// it until recovered keeps it on the ground while `GO_DUCK` lasts, and refuses a dive.
    #[test]
    fn wall_jump_pose_hold() {
        let core = |fin, go_duck, grounded, id| wall_jump_pose_holds(false, fin, true, go_duck, false, grounded, id);
        let held = |fin, go_duck, grounded, id| wall_jump_pose_holds(true, fin, true, go_duck, false, grounded, id);
        // In the air, both hold against the idle pose and let the wall poses through.
        assert!(core(false, true, false, 0) && held(false, true, false, 0));
        assert!(!core(false, true, false, 0x38) && !held(false, true, false, 0x37));
        // Landed: 3.0 lets the ground poses in, the other keeps the pose.
        assert!(!core(false, true, true, 0));
        assert!(held(false, true, true, 0));
        // The dive only gets through in 3.0.
        assert!(!core(false, true, false, 0x28) && held(false, true, false, 0x28));
        // A finished pose, or the recovery's end clearing GO_DUCK, lets anything in.
        assert!(!held(true, true, true, 0) && !held(false, false, true, 0));
        assert!(!wall_jump_pose_holds(true, false, false, true, false, true, 0), "dead");
        assert!(!wall_jump_pose_holds(true, false, true, true, true, true, 0), "on a ladder");
    }
}
