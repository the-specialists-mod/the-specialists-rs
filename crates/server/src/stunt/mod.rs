//! Server half of TS stunts (CTSStunt) and animation selection (CTSAnimation). Owner: movement
//! work package. Movement physics itself is shared::pm (runs on both sides).
//! Reference: decomp docs/server/player-movement.md §2-3, §5, §6.6, §7.
//!
//! As in TS, the stunt *triggers* run here on the server and communicate with player movement
//! through `pev->iuser4` (`shared::ts::stunt`), which the client predicts from `clientdata`.
//! Keeping the triggers server-side keeps the wire behaviour identical to TS 3.0, so the
//! original client predicts our server correctly and vice versa.
//!
//! Hooks (fixed order, player/mod.rs): `pre_think` runs the CBasePlayer::PreThink stunt parts
//! (button edges, `CTSStunt::Play`, breakables, `Jump()`/`Duck()`, fall speed, `TSState`);
//! `post_think` runs the PostThink parts (frozen angles, fall damage, super-jump landing,
//! idle/move animation, frame advance).

pub mod anim;
mod moves;

use crate::commands::Args;
use crate::messages::{self, Msg};
use crate::player::Player;
use crate::world::Cx;
use goldsrc::server::engine;
use goldsrc::sys::*;
use shared::pm::Vec3;
use shared::ts::stunt::*;
use std::ffi::CStr;

pub use anim::AnimState;

/// `CTSStunt` (pl+0x4f0) plus the `CBasePlayer` fields only the stunt code uses.
pub struct StuntState {
    // ---- CTSStunt ------------------------------------------------------------------------------
    /// +0x00 stunt cooldown (s, slowed time). Sent ×10 in `NextStunt`.
    pub next_stunt: f32,
    /// +0x04 recovery lock (s). Clamped to ≥ 0.001 while active; on expiry the recover bits clear.
    pub recover: f32,
    /// +0x08 close-combat cooldown (s). Jump kicks (weapons) write it.
    pub close_attack_cd: f32,
    /// +0x0c / +0x10 / +0x14 double-tap windows (absolute times): duck (roll), jump (flip),
    /// jump at a wall (climb).
    pub roll_tap: f32,
    pub wheel_tap: f32,
    pub climb_tap: f32,
    /// +0x18 `CheckForBreakables` gate: -1 disables it.
    pub breakable_check: f32,
    /// +0x20 stunt direction (read by the animation code).
    pub dir: Vec3,
    /// +0x2c `GetDirection(dir) != forward`.
    pub dir_not_forward: bool,
    /// +0x30 prone lock (s): can't get up yet (0.55 after bullet time).
    pub prone_lock: f32,
    /// +0x34 bullet-time dodge until (absolute). Player damage scales bullets ×0.7 before it.
    pub dodge_until: f32,
    /// +0x38 / +0x3c wall-stunt chain cooldown and climb count (reset on the ground).
    pub wall_chain: f32,
    pub wall_climbs: i32,

    // ---- CBasePlayer fields owned here ---------------------------------------------------------
    /// pl+0x4dc `BlockPlayer` timer: `RECOVER` (half speed) until it runs out.
    pub block_timer: f32,
    /// pl+0x4e0 angles kept while `FREEZE_ANGLES`.
    pub frozen_angles: Vec3,
    /// pl+0x810/0x814/0x818 `m_afButtonLast/Pressed/Released`.
    pub button_last: i32,
    pub button_pressed: i32,
    pub button_released: i32,
    /// pl+0xb88 last time on the ground ("coyote time" for air stunts).
    pub last_ground_time: f32,
    /// pl+0x828 `m_flFallVelocity`.
    pub fall_velocity: f32,
    /// pl+0x498/0x499 `TSState` stance (0 stand, 1 crouch, 2 prone/diving) and last sent.
    pub stance: u8,
    stance_sent: Option<u8>,
    /// pl+0x7b9 "is The One" (rules sets it): ignores slot limits.
    pub the_one: bool,
    /// Last physinfo `mj` value written (mirrors `Player::superjump`).
    mj_sent: Option<bool>,

    /// Close-combat hit window shared with weapons (pl+0x798..0x7a8): a delayed hit that lands
    /// through `CheckForBreakables` while the player keeps moving (flying kicks, backflip kick).
    pub melee: MeleeWindow,

    /// `CTSAnimation` (pl+0x28c).
    pub anim: AnimState,
}

/// pl+0x798..0x7a8.
#[derive(Clone, Copy, Debug)]
pub struct MeleeWindow {
    /// pl+0x798: last entity hit by a stunt/kick (not hit twice in a row).
    pub last_victim: *mut edict_t,
    /// pl+0x79c: hit window (s); 0 = none.
    pub window: f32,
    /// pl+0x7a0: pending damage (sign = forward/back).
    pub damage: f32,
    /// pl+0x7a4: 0 kung fu, 1 knife, 2 katana.
    pub kind: u8,
    /// pl+0x7a8: kung-fu block window (s).
    pub block: f32,
}

impl Default for MeleeWindow {
    fn default() -> Self {
        Self { last_victim: std::ptr::null_mut(), window: 0.0, damage: 0.0, kind: 0, block: 0.0 }
    }
}

impl Default for StuntState {
    fn default() -> Self {
        Self {
            next_stunt: 0.0,
            recover: 0.0,
            close_attack_cd: 0.0,
            roll_tap: 0.0,
            wheel_tap: 0.0,
            climb_tap: 0.0,
            breakable_check: 0.0,
            dir: [0.0; 3],
            dir_not_forward: false,
            prone_lock: 0.0,
            dodge_until: 0.0,
            wall_chain: 0.0,
            wall_climbs: 0,
            block_timer: 0.0,
            frozen_angles: [0.0; 3],
            button_last: 0,
            button_pressed: 0,
            button_released: 0,
            last_ground_time: 0.0,
            fall_velocity: 0.0,
            stance: 0,
            stance_sent: None,
            the_one: false,
            mj_sent: None,
            melee: MeleeWindow::default(),
            anim: AnimState::default(),
        }
    }
}

// ---- small helpers shared by the submodules ------------------------------------------------------

pub(crate) const IN_ALT1: i32 = shared::ts::IN_ALT1 as i32;
const CHAN_BODY_: i32 = CHAN_BODY as i32;
const ATTN_NORM: f32 = 0.8;

/// `GiveSlowMul`: the player's time scale, at least 0.01.
pub fn slow_mul(cx: &mut Cx) -> f32 {
    cx.pev().fuser1.max(0.01)
}

fn frametime() -> f32 {
    engine().globals().frametime
}

fn on_ground(cx: &mut Cx) -> bool {
    cx.pev().flags & FL_ONGROUND as i32 != 0
}

fn on_ladder(cx: &mut Cx) -> bool {
    cx.pev().movetype == MOVETYPE_FLY as i32
}

fn is_alive(cx: &mut Cx) -> bool {
    Player::is_alive(cx)
}

/// TS `EMIT_SOUND_DYN` (0xe0a8c): the pitch follows the emitter's time scale,
/// `pitch * (k * fuser1 + 1 - k)`.
pub fn emit_sound_dyn(e: *mut edict_t, channel: i32, sample: &CStr, volume: f32, attn: f32, pitch: i32, k: f32) {
    let ts = unsafe { (*e).v.fuser1 };
    let rest = 1.0 - k;
    let k = if ts != 0.0 { k * ts } else { k };
    let pitch = ((k + rest) * pitch as f32).round_ties_even() as i32;
    engine().emit_sound(e, channel, sample, volume, attn, 0, pitch);
}

/// The stunt sounds: body channel, pitch 95..126, half time-scaled.
fn stunt_sound(cx: &mut Cx, sample: &CStr) {
    let pitch = engine().random_long(0, 31) + 95;
    emit_sound_dyn(cx.edict, CHAN_BODY_, sample, 1.0, ATTN_NORM, pitch, 0.5);
}

/// `gmsgTSNextStunt`: the cooldown in tenths of a second.
fn send_next_stunt(p: &mut Player, cx: &mut Cx) {
    let v = (p.stunt.next_stunt * 10.0).round_ties_even() as i32;
    messages::one(Msg::NextStunt, cx.edict).byte(v).end();
}

/// `TSMBlur` to everyone in the PVS: (entindex, level).
pub fn motion_blur(cx: &mut Cx, level: i32) {
    let origin = cx.pev().origin;
    let idx = cx.index();
    messages::pvs(Msg::TsMBlur, origin).byte(idx).byte(level).end();
}

/// Blur when `RandomLong(0,10) + 10*slowmul > threshold` (stunt starts).
fn maybe_blur(cx: &mut Cx, threshold: f32) {
    let r = engine().random_long(0, 10);
    if slow_mul(cx) * 10.0 + r as f32 > threshold {
        motion_blur(cx, 2);
    }
}

/// `GetDirection(dir, angles, 0.6, 0.3)` (0x6b008) on the yaw-only basis:
/// 0 left, 1 right, 2 forward, 3 back.
pub fn get_direction(dir: Vec3, angles: Vec3, side: f32, back: f32) -> i32 {
    let (forward, right, _) = shared::pm::angle_vectors([0.0, angles[1], angles[2]]);
    let d = right[0] * dir[0] + right[1] * dir[1] + right[2] * dir[2];
    if d < -side {
        0
    } else if d > side {
        1
    } else if forward[0] * dir[0] + forward[1] * dir[1] + forward[2] * dir[2] < -back {
        3
    } else {
        2
    }
}

/// `Vector::Normalize()`: unit vector, or (0,0,1) for a zero vector.
fn normalized(v: Vec3) -> Vec3 {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len == 0.0 {
        [0.0, 0.0, 1.0]
    } else {
        let il = 1.0 / len;
        [il * v[0], il * v[1], il * v[2]]
    }
}

/// `FixedUnsigned16`.
fn fixed_u16(value: f32, scale: f32) -> i32 {
    ((value * scale) as i32).clamp(0, 0xffff)
}

/// TS `UTIL_ScreenShake` (0xec978): SDK shake to grounded players within `radius`, with the
/// duration and frequency divided by each receiver's time scale.
pub fn screen_shake(center: Vec3, amplitude: f32, frequency: f32, duration: f32, radius: f32) {
    let e = engine();
    let dur = fixed_u16(duration, 4096.0);
    let freq = fixed_u16(frequency, 256.0);
    for i in 1..=e.max_clients() {
        let pl = e.player(i);
        if pl.is_null() {
            continue;
        }
        let v = unsafe { &(*pl).v };
        if v.flags & FL_ONGROUND as i32 == 0 {
            continue;
        }
        let d = [center[0] - v.origin[0], center[1] - v.origin[1], center[2] - v.origin[2]];
        let local = if radius <= 0.0 || (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() < radius { amplitude } else { 0.0 };
        if local == 0.0 {
            continue;
        }
        let slow = v.fuser1.max(0.01);
        messages::one(Msg::ScreenShake, pl)
            .short(fixed_u16(local, 4096.0) & 0xffff)
            .short(((dur & 0xffff) as f32 / slow).round_ties_even() as i32)
            .short(((freq & 0xffff) as f32 / slow).round_ties_even() as i32)
            .end();
    }
}

// ---- API for other packages -------------------------------------------------------------------------

/// `CBasePlayer::BlockPlayer(t)` (0x95b58): half speed (`RECOVER`) for `t` seconds.
pub fn block_player(p: &mut Player, cx: &mut Cx, t: f32) {
    p.stunt.block_timer = t;
    cx.pev().iuser4 |= RECOVER;
}

/// `CTSAnimation::Animate(id)`: weapons call it with 0/8/9 (fire), 0x33/0x34 (reload/draw),
/// 0x35 (grenade) and the close-combat ids; rules with 7 (death).
pub fn animate(p: &mut Player, cx: &mut Cx, id: i32) {
    anim::animate(p, cx, id);
}

/// Sets the weapon animation extensions (`m_szAnimExtention` pl+0x478 for the idle gait, and
/// pl+0xb58 for torso sequences: "onehanded", "twohanded", "akimbo", "onehanded_case", or "").
pub fn set_anim_ext(p: &mut Player, ext: &str, ext2: &str) {
    p.stunt.anim.ext = ext.to_owned();
    p.stunt.anim.ext2 = ext2.to_owned();
}

/// True while an airborne stunt is running (dive, flips, wall jump): ×0.7 damage taken, and a
/// kill during it counts as spectacular (`iuser4 & 0x10c10`).
pub fn in_airborne_stunt(cx: &mut Cx) -> bool {
    cx.pev().iuser4 & shared::pm::ts::AIRBORNE_MASK != 0
}

// ---- hooks ---------------------------------------------------------------------------------------------

pub fn game_init() {}

/// Stunt and player-movement sounds (PM plays footsteps through the engine).
pub fn precache() {
    const SOUNDS: &[&CStr] = &[
        c"player/airwheel.wav",
        c"player/dive.wav",
        c"player/land.wav",
        c"player/superjump-land.wav",
        c"player/kungfuhit.wav",
        c"player/closecombat.wav",
        c"common/bodysplat.wav",
        c"weapons/knife/knife_hitbody.wav",
        c"weapons/katana/hit-body.wav",
        c"player/pl_fallpain3.wav",
        c"player/pl_step1.wav",
        c"player/pl_step2.wav",
        c"player/pl_step3.wav",
        c"player/pl_step4.wav",
        c"player/pl_metal1.wav",
        c"player/pl_metal2.wav",
        c"player/pl_metal3.wav",
        c"player/pl_metal4.wav",
        c"player/pl_dirt1.wav",
        c"player/pl_dirt2.wav",
        c"player/pl_dirt3.wav",
        c"player/pl_dirt4.wav",
        c"player/pl_duct1.wav",
        c"player/pl_duct2.wav",
        c"player/pl_duct3.wav",
        c"player/pl_duct4.wav",
        c"player/pl_grate1.wav",
        c"player/pl_grate2.wav",
        c"player/pl_grate3.wav",
        c"player/pl_grate4.wav",
        c"player/pl_tile1.wav",
        c"player/pl_tile2.wav",
        c"player/pl_tile3.wav",
        c"player/pl_tile4.wav",
        c"player/pl_tile5.wav",
        c"player/pl_slosh1.wav",
        c"player/pl_slosh2.wav",
        c"player/pl_slosh3.wav",
        c"player/pl_slosh4.wav",
        c"player/pl_wade1.wav",
        c"player/pl_wade2.wav",
        c"player/pl_wade3.wav",
        c"player/pl_wade4.wav",
        c"player/pl_ladder1.wav",
        c"player/pl_ladder2.wav",
        c"player/pl_ladder3.wav",
        c"player/pl_ladder4.wav",
    ];
    let e = engine();
    for s in SOUNDS {
        e.precache_sound(s);
    }
}

/// `TSInit` (0x92a4c), the stunt and animation parts.
pub fn on_spawn(p: &mut Player, cx: &mut Cx) {
    let the_one = p.stunt.the_one;
    p.stunt = StuntState { the_one, ..StuntState::default() };
    let pev = cx.pev();
    pev.iuser4 = 0;
    pev.gaitsequence = 0;
    pev.sequence = 0;
    pev.frame = 0.0;
    anim::reset_sequence_info(p, cx);
    // Spawn: physinfo keys. `mj` enables the super jump (power-up or map flag).
    let e = engine();
    e.set_physics_key(cx.edict, c"slj", c"0");
    e.set_physics_key(cx.edict, c"hl", c"1");
    sync_superjump(p, cx);
}

fn sync_superjump(p: &mut Player, cx: &mut Cx) {
    if p.stunt.mj_sent != Some(p.superjump) {
        p.stunt.mj_sent = Some(p.superjump);
        engine().set_physics_key(cx.edict, c"mj", if p.superjump { c"1" } else { c"0" });
    }
}

/// The stunt parts of `CBasePlayer::PreThink` (0x908cc), in order.
pub fn pre_think(p: &mut Player, cx: &mut Cx) {
    let time = cx.time();
    {
        let button = cx.pev().button;
        let changed = p.stunt.button_last ^ button;
        p.stunt.button_pressed = button & changed;
        p.stunt.button_released = changed & !button;
    }
    sync_superjump(p, cx);

    // Animation runs in the player's slowed time.
    let slow = slow_mul(cx);
    cx.pev().framerate = slow;
    if on_ground(cx) {
        p.stunt.last_ground_time = time;
    }
    let mut dt = slow * frametime();
    if p.active_powerup & shared::ts::powerup::DOUBLE_FIRERATE != 0 {
        dt *= 1.5;
    }
    let m = &mut p.stunt.melee;
    if m.window != 0.0 {
        m.window = (m.window - dt).max(0.001);
    }
    if p.stunt.block_timer != 0.0 {
        p.stunt.block_timer = (p.stunt.block_timer - dt).max(0.001);
    }
    m.block = (m.block - dt).max(0.0);

    moves::play(p, cx);
    anim::update_timers(p, cx);

    let pev = cx.pev();
    if pev.view_ofs == [0.0; 3] {
        pev.iuser4 |= NO_VIEW;
    } else {
        pev.iuser4 &= !NO_VIEW;
    }

    // BlockPlayer expiry.
    if p.stunt.block_timer != 0.0 && cx.pev().iuser4 & RECOVER != 0 && p.stunt.block_timer < 0.01 {
        cx.pev().iuser4 &= !RECOVER;
        p.stunt.block_timer = 0.0;
    }
    // Close-combat window expiry.
    let m = &mut p.stunt.melee;
    if m.window != 0.0 {
        if m.window < 0.01 || m.damage == 0.0 {
            m.window = 0.0;
            m.damage = 0.0;
            m.kind = 0;
        }
        m.block = 0.0;
    }

    if cx.pev().iuser1 != 0 {
        return; // observer
    }
    moves::check_for_breakables(p, cx);
    if cx.pev().deadflag > 0 {
        return;
    }

    let button = cx.pev().button;
    if button & IN_JUMP as i32 != 0 {
        moves::jump(p, cx);
    }
    if button & IN_DUCK as i32 != 0 || cx.pev().flags & FL_DUCKING as i32 != 0 {
        moves::duck(p, cx);
    }
    if !on_ground(cx) {
        p.stunt.fall_velocity = -cx.pev().velocity[2];
    }

    // UpdatePlayerState; TS sends TSState from UpdateClientData when it changes.
    let pev = cx.pev();
    p.stunt.stance = if pev.iuser4 & (DIVING | PRONE) != 0 {
        2
    } else if pev.flags & FL_DUCKING as i32 != 0 {
        1
    } else {
        0
    };
    if p.stunt.stance_sent != Some(p.stunt.stance) {
        p.stunt.stance_sent = Some(p.stunt.stance);
        messages::one(Msg::TsState, cx.edict).byte(p.stunt.stance as i32).end();
    }
}

/// The stunt parts of `CBasePlayer::PostThink` (0x91610), in order.
pub fn post_think(p: &mut Player, cx: &mut Cx) {
    if is_alive(cx) {
        if cx.pev().iuser4 & FREEZE_ANGLES != 0 {
            cx.pev().angles = p.stunt.frozen_angles;
        }
        moves::fall_damage(p, cx);
        if on_ground(cx) {
            cx.pev().gravity = 1.0;
            p.stunt.fall_velocity = 0.0;
            if cx.pev().iuser4 & SUPERJUMP_GLIDE != 0 {
                moves::super_jump_landing(p, cx);
            }
        }
        if is_alive(cx) {
            let v = cx.pev().velocity;
            let id = if v[0] == 0.0 && v[1] == 0.0 { 6 } else { 1 };
            anim::animate(p, cx, id);
        }
        anim::studio_frame_advance(p, cx);
        // TS updates m_afButtonLast only while alive.
        p.stunt.button_last = cx.pev().button;
    }
}

pub fn client_command(_p: &mut Player, _cx: &mut Cx, _cmd: &str, _args: &Args) -> bool {
    false
}

/// Prediction inputs: the stunt bits and the time scale (TS's `delta.lst` carries both).
/// Everything else player movement reads (maxspeed, view_ofs, bInDuck, flDuckTime, physinfo
/// `mj`, and gravity/friction through the player's entity_state) is already networked.
pub fn fill_client_data(_p: &mut Player, cx: &mut Cx, cd: &mut clientdata_t) {
    let pev = cx.pev();
    cd.iuser4 = pev.iuser4;
    cd.fuser1 = pev.fuser1;
}
