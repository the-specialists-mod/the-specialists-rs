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
//! (button edges, `CTSStunt::Play`, breakables, `Jump()`/`Duck()`, fall speed);
//! `post_think` runs the PostThink parts (frozen angles, fall damage, super-jump landing,
//! idle/move animation, frame advance).

pub mod anim;
pub(crate) mod moves;
pub mod rules;

use crate::commands::Args;
use crate::entities::stock::explode::emit_sound_slow;
use crate::messages::{self, Msg};
use crate::player::Player;
use crate::world::Cx;
use goldsrc::server::engine;
use goldsrc::sys::*;
use shared::pm::{Fpu, FpuMath, Num, ORDER_XYZ, ORDER_ZYX, Vec3, below87, eq87};
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
    /// pl+0x848 `m_afPhysicsFlags & PFLAG_DUCKING` (8). `PreThink` calls `Duck()` while it is
    /// set, but nothing in TS sets it: `Spawn` zeroes the flags and `StartObserver` (0x99634)
    /// clears the bit, so it stays false. Ported for the `PreThink` test.
    pub pflag_ducking: bool,
    /// pl+0x498: the `TSState` stance (0 stand, 1 crouch, 2 prone/diving) `UpdatePlayerState`
    /// (0x92a08) last set. Only a living player's PreThink updates it, and `TSInit` zeroes it;
    /// `player::hud` sends it when it changes.
    pub stance: u8,
    /// Last physinfo `mj` value written (mirrors `Player::superjump`).
    mj_sent: Option<bool>,

    /// Last `iuser4` seen by the debug trace.
    debug_last: i32,
    /// Debug: (takeoff z, highest z) of the current airborne phase.
    debug_air: Option<(f32, f32)>,
    /// Frames of simulated `+alt1` left (`ts_debug_alt1`, with `ts_debug_stunt 1`).
    debug_alt1: i32,

    /// `CTSAnimation` (pl+0x28c).
    pub anim: AnimState,
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
            pflag_ducking: false,
            stance: 0,
            mj_sent: None,
            debug_last: 0,
            debug_alt1: 0,
            debug_air: None,
            anim: AnimState::default(),
        }
    }
}

// ---- small helpers shared by the submodules ------------------------------------------------------

/// The active ruleset's server hooks, resolved once per entry point and passed down
/// (`ServerRules::stunts`, `stunt_clock`, `can_dive`, `motion_blur`...).
type Rules = &'static dyn crate::gameplay::ServerRules;

pub(crate) const IN_ALT1: i32 = shared::ts::IN_ALT1 as i32;
const ATTN_NORM: f32 = 0.8;

/// `GiveSlowMul`: the player's time scale, at least 0.01.
pub fn slow_mul(cx: &mut Cx) -> f32 {
    let e = cx.edict;
    crate::ham::give_slow_mul(e, || goldsrc::edict::EdictPtr::v(e).fuser1.max(0.01))
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

/// The stunt sounds: body channel, pitch 95..126, half time-scaled (TS `EMIT_SOUND_DYN`).
fn stunt_sound(cx: &mut Cx, sample: &CStr) {
    let pitch = engine().random_long(0, 31) + 95;
    emit_sound_slow(cx.edict, CHAN_BODY, sample, 1.0, ATTN_NORM, 0, pitch, 0.5);
}

/// `gmsgTSNextStunt`: the cooldown in tenths of a second ([`rules::StuntClock::next_stunt_byte`]).
fn send_next_stunt(p: &mut Player, cx: &mut Cx, r: Rules) {
    let v = r.stunt_clock().next_stunt_byte(cx.time(), p.stunt.next_stunt);
    messages::one(Msg::NextStunt, cx.edict).byte(v as i32).end();
}

/// `TSMBlur` to everyone in the PVS: (entindex, level).
pub fn motion_blur(cx: &mut Cx, level: i32) {
    let origin = cx.pev().origin;
    let idx = cx.index();
    messages::pvs(Msg::TsMBlur, origin).byte(idx).byte(level).end();
}

/// Blur when `RandomLong(0,10) + 10*slowmul > threshold` (stunt starts). A ruleset without
/// motion blur draws no number either.
fn maybe_blur(cx: &mut Cx, rules: Rules, threshold: f32) {
    if !rules.motion_blur() {
        return;
    }
    let r = engine().random_long(0, 10);
    if slow_mul(cx) * 10.0 + r as f32 > threshold {
        motion_blur(cx, 2);
    }
}

/// `GetDirection(dir, angles, 0.6, 0.3)` (0x6b008) on the yaw-only basis:
/// 0 left, 1 right, 2 forward, 3 back.
pub fn get_direction(dir: Vec3, angles: Vec3, side: f32, back: f32) -> i32 {
    let arith = crate::rules::cvars::pm_arith();
    let (forward, right, _) = shared::pm::angle_vectors(arith, [0.0, angles[1], angles[2]]);
    shared::by_arith!(arith, direction_in(forward, right, dir, side, back))
}

/// [`get_direction`]'s tests on the basis. Both dot products stay on the FPU against the `float`
/// thresholds: `ts_i386.so` sums `x, y, z` (0x6b04f..0x6b07a), mp.dll `z, y, x` (0x1004453e..,
/// 0x10044584.., the same in 2.1's 0x1003d5d3). A NaN right-dot is "left" in mp.dll (`test
/// ah,0x1`) and passes on in the .so (`jbe`); a NaN forward-dot is "back" only in the .so.
fn direction_in<F: Fpu>(forward: Vec3, right: Vec3, dir: Vec3, side: f32, back: f32) -> i32 {
    let dot = |b: Vec3| {
        let [i, j, k] = if F::MSVC { ORDER_ZYX } else { ORDER_XYZ };
        F::x(b[i]) * F::x(dir[i]) + F::x(b[j]) * F::x(dir[j]) + F::x(b[k]) * F::x(dir[k])
    };
    let r = dot(right);
    let left = if F::MSVC { below87(r, F::x(-side)) } else { r < F::x(-side) };
    if left {
        return 0;
    }
    if r > F::x(side) {
        return 1;
    }
    let f = dot(forward);
    let behind = if F::MSVC { f < F::x(-back) } else { below87(f, F::x(-back)) };
    if behind { 3 } else { 2 }
}

/// The inline `Vector::Normalize()` of the stunt and animation code (`x² + y² + z²` in both
/// builds: GoDive 0x9b434 / mp.dll 0x10069727, GoWheel 0x9cb44 / 0x1006ab70.., GoRoll 0x9d0fa /
/// 0x1006b0d8.., Animate's jump, dive and death directions 0x6b524, 0x6c6b2.. / mp.dll's
/// out-of-line 0x100148f0): unit vector, or (0, 0, 1) for a zero (or NaN) length.
fn normalized(v: Vec3) -> Vec3 {
    fn go<F: Fpu>(v: Vec3) -> Vec3 {
        F::vec_normalize(v, ORDER_XYZ)
    }
    shared::by_arith!(crate::rules::cvars::pm_arith(), go(v))
}

/// The inline `Length2D()` of the velocity, `x² + y²` on the FPU in both builds.
fn hspeed_n<F: Fpu>(v: Vec3) -> F::N {
    (F::x(v[0]) * F::x(v[0]) + F::x(v[1]) * F::x(v[1])).sqrt()
}

/// The horizontal speed where both builds store it as a `float` (`Animate`'s `speed` 0x6b4ac /
/// mp.dll 0x10044b93, `GoWheel`'s 0x9c9e9 / 0x1006aa53).
fn hspeed_stored(v: Vec3) -> f32 {
    fn go<F: Fpu>(v: Vec3) -> f32 {
        hspeed_n::<F>(v).f()
    }
    shared::by_arith!(crate::rules::cvars::pm_arith(), go(v))
}

/// `Length2D() < limit` where mp.dll tests the root unrounded and `ts_i386.so` stores it first
/// (`HandleStunt` 0x9b16a / mp.dll 0x10069457, `GoRoll` 0x9d064 / 0x1006b046,
/// `CBasePlayer::TakeDamage`'s bullet dance 0x8fb2e / 0x1005ea49), or with `above`
/// `Length2D() > limit` (`GaitCloseCombat` 0x6dda9 / 0x1004705b, `Killed`'s hop 0x8f3a2 /
/// 0x1005e479).
pub(crate) fn hspeed_cmp(v: Vec3, limit: f32, above: bool) -> bool {
    fn go<F: Fpu>(v: Vec3, limit: f32, above: bool) -> bool {
        let s = hspeed_n::<F>(v);
        let s = if F::MSVC { s } else { F::x(s.f()) };
        if above { s > F::x(limit) } else { s < F::x(limit) }
    }
    shared::by_arith!(crate::rules::cvars::pm_arith(), go(v, limit, above))
}

/// TS `UTIL_ScreenShake` (0xec978); one helper shared with the stock entities and `TSExplosion`.
pub use crate::entities::stock::explode::screen_shake;

// ---- API for other packages -------------------------------------------------------------------------

/// `CBasePlayer::BlockPlayer(t)` (0x95b58): half speed (`RECOVER`) for `t` seconds of the
/// player's time (kept by the ruleset's [`rules::StuntClock`]).
pub fn block_player(p: &mut Player, cx: &mut Cx, t: f32) {
    let clock = crate::gameplay::active().server.stunt_clock();
    p.stunt.block_timer = clock.start(cx.time(), cx.pev().fuser1, f64::from(t));
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

static DEBUG: goldsrc::Global<std::cell::Cell<Option<&'static cvar_t>>> = goldsrc::Global(std::cell::Cell::new(None));

/// `ts_debug_stunt 1` logs every `iuser4` change (with origin, velocity and sequences) to the
/// server console. Development aid; not in TS.
pub fn game_init() {
    DEBUG.0.set(Some(engine().register_cvar("ts_debug_stunt", "0", 0)));
}

fn debug_enabled() -> bool {
    DEBUG.0.get().is_some_and(|c| goldsrc::server::cvar_value(c) != 0.0)
}

/// Debug trace of stunt state changes (see `ts_debug_stunt`).
fn debug_log(p: &mut Player, cx: &mut Cx, last: &mut i32) {
    let edict = cx.edict;
    let time = cx.time();
    let pev = cx.pev();
    if pev.iuser4 == *last || !debug_enabled() {
        *last = pev.iuser4;
        return;
    }
    let name = |seq: i32| {
        goldsrc::ext::movement::model_ptr(edict)
            .and_then(|h| h.sequence_name(seq).map(|n| String::from_utf8_lossy(n).into_owned()))
            .unwrap_or_default()
    };
    let msg = format!(
        "[stunt] t={:.2} iuser4 {:#x} -> {:#x} org=({:.0} {:.0} {:.0}) vel=({:.0} {:.0} {:.0}) view={} hull_duck={} seq={} gait={} next={:.2} recover={:.2}\n",
        time,
        *last,
        pev.iuser4,
        pev.origin[0],
        pev.origin[1],
        pev.origin[2],
        pev.velocity[0],
        pev.velocity[1],
        pev.velocity[2],
        pev.view_ofs[2],
        pev.flags & FL_DUCKING as i32 != 0,
        name(pev.sequence),
        name(pev.gaitsequence),
        p.stunt.next_stunt,
        p.stunt.recover,
    );
    *last = pev.iuser4;
    engine().print(&msg);
}

/// Debug trace of airborne phases: rise from takeoff to apex, logged on landing.
fn debug_air(p: &mut Player, cx: &mut Cx) {
    if !debug_enabled() {
        return;
    }
    let grounded = on_ground(cx);
    let z = cx.pev().origin[2];
    match (&mut p.stunt.debug_air, grounded) {
        (None, false) => p.stunt.debug_air = Some((z, z)),
        (Some((_, top)), false) => *top = top.max(z),
        (Some((start, top)), true) => {
            let msg =
                format!("[stunt] landed: rise {:.2} (takeoff z {:.2}, apex z {:.2}, landing z {:.2})\n", *top - *start, *start, *top, z);
            p.stunt.debug_air = None;
            engine().print(&msg);
        }
        (None, true) => {}
    }
}

/// `TEXTURETYPE_Init`, from `CWorld::Precache` (0xf449c): the texture table `GoRoll`'s impact
/// sound reads, with the active ruleset's limits and map file
/// ([`crate::entities::stock::impact::game_textures`]). (The stunt and footstep sounds are in
/// `ClientPrecache` / `TSPrecache`, `crate::precache`.)
pub fn texture_type_init() {
    let e = engine();
    let map = e.string(e.globals().mapname).to_string_lossy().into_owned();
    let (limits, map_files) = crate::entities::stock::impact::game_textures();
    shared::pm::textures::texturetype::init(limits, map_files, &map, |path| e.load_file(path));
}

/// `TSInit` (0x92a4c), the stunt and animation parts.
pub fn on_spawn(p: &mut Player, cx: &mut Cx) {
    // Keep what weapons set earlier in the spawn order: the animation extensions of the spawn
    // loadout.
    let (ext, ext2) = (std::mem::take(&mut p.stunt.anim.ext), std::mem::take(&mut p.stunt.anim.ext2));
    p.stunt = StuntState::default();
    p.stunt.anim.ext = ext;
    p.stunt.anim.ext2 = ext2;
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
    crate::gameplay::set_physinfo(cx.edict);
    sync_superjump(p, cx);
}

fn sync_superjump(p: &mut Player, cx: &mut Cx) {
    if p.stunt.mj_sent != Some(p.superjump) {
        p.stunt.mj_sent = Some(p.superjump);
        engine().set_physics_key(cx.edict, c"mj", if p.superjump { c"1" } else { c"0" });
    }
}

/// `CBasePlayer::Jump` (vtable slot 84), which `PreThink` calls while +jump is held (through
/// `crate::ham` for plugins' hooks).
pub fn jump(p: &mut Player, cx: &mut Cx) {
    moves::jump(p, cx, crate::gameplay::active().server);
}

/// `CBasePlayer::Duck` (vtable slot 85), as [`jump`].
pub fn duck(p: &mut Player, cx: &mut Cx) {
    moves::duck(p, cx, crate::gameplay::active().server);
}

/// The stunt parts of `CBasePlayer::PreThink` (0x908cc), in order. `tail` is false on the frame
/// a train is lost: TS's PreThink returns before `Jump`/`Duck` then (issue #60).
pub fn pre_think(p: &mut Player, cx: &mut Cx, tail: bool) {
    let mut last = p.stunt.debug_last;
    debug_log(p, cx, &mut last);
    p.stunt.debug_last = last;
    debug_air(p, cx);
    if p.stunt.debug_alt1 > 0 {
        p.stunt.debug_alt1 -= 1;
        cx.pev().button |= IN_ALT1;
    }
    let time = cx.time();
    let r: Rules = crate::gameplay::active().server;
    let clock = r.stunt_clock();
    {
        let button = cx.pev().button;
        let changed = p.stunt.button_last ^ button;
        p.stunt.button_pressed = button & changed;
        p.stunt.button_released = changed & !button;
    }
    sync_superjump(p, cx);

    // Animation runs in the player's slowed time.
    let slow = slow_mul(cx);
    cx.pev().framerate = r.anim_framerate(cx.pev().fuser1);
    if on_ground(cx) {
        p.stunt.last_ground_time = time;
    }
    let mut dt = slow * frametime();
    if p.active_powerup & shared::ts::powerup::DOUBLE_FIRERATE != 0 {
        dt *= 1.5;
    }
    // The close-combat window and block timers (pl+0x79c..0x7b0) live in the gun
    // (`GunRules::pre_think`, weapons pre_think).
    clock.tick_block(&mut p.stunt, dt);

    moves::play(p, cx, r);
    anim::update_timers(p, cx);

    let pev = cx.pev();
    if pev.view_ofs == [0.0; 3] {
        pev.iuser4 |= NO_VIEW;
    } else {
        pev.iuser4 &= !NO_VIEW;
    }

    // BlockPlayer expiry.
    if p.stunt.block_timer != 0.0 && cx.pev().iuser4 & RECOVER != 0 && clock.done(time, p.stunt.block_timer) {
        cx.pev().iuser4 &= !RECOVER;
        p.stunt.block_timer = 0.0;
    }
    if cx.pev().iuser1 != 0 {
        return; // observer
    }
    moves::check_for_breakables(p, cx);
    if cx.pev().deadflag > 0 || !tail {
        return;
    }

    let button = cx.pev().button;
    if button & IN_JUMP as i32 != 0 {
        crate::ham::jump(p, cx);
    }
    if button & IN_DUCK as i32 != 0 || cx.pev().flags & FL_DUCKING as i32 != 0 || p.stunt.pflag_ducking {
        crate::ham::duck(p, cx);
    }
    if !on_ground(cx) {
        p.stunt.fall_velocity = -cx.pev().velocity[2];
    }
    // `UpdatePlayerState` (0x92a08). TS sends the stance only from `UpdateClientData`
    // (`player::hud`), never from here (issue #48).
    p.stunt.stance = crate::player::hud::stance(cx.pev());
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
                moves::super_jump_landing(p, cx, crate::gameplay::active().server);
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

/// TS has no stunt client commands. Development aids, only with `ts_debug_stunt 1`:
/// `ts_debug_alt1` holds the stunt key for 3 frames (scripted tests before the client has
/// `+alt1`), `ts_debug_face <pitch> <yaw>` turns the player.
pub fn client_command(p: &mut Player, cx: &mut Cx, cmd: &str, args: &Args) -> bool {
    if !debug_enabled() {
        return false;
    }
    match cmd {
        "ts_debug_alt1" => p.stunt.debug_alt1 = 3,
        "ts_debug_face" => {
            let pev = cx.pev();
            pev.v_angle = [args.float(1), args.float(2), 0.0];
            // SVC_SETANGLE sends pev->angles as the client view angles.
            pev.angles = pev.v_angle;
            pev.fixangle = 1;
        }
        _ => return false,
    }
    true
}

/// Prediction inputs: the stunt bits and the time scale (TS's `delta.lst` carries both).
/// Everything else player movement reads (maxspeed, view_ofs, bInDuck, flDuckTime, physinfo
/// `mj`, and gravity/friction through the player's entity_state) is already networked.
pub fn fill_client_data(_p: &mut Player, cx: &mut Cx, cd: &mut clientdata_t) {
    let pev = cx.pev();
    cd.iuser4 = pev.iuser4;
    cd.fuser1 = pev.fuser1;
}

#[cfg(test)]
mod precision_tests {
    use super::*;
    use shared::pm::{Gcc, Msvc};

    fn v(b: [u32; 3]) -> Vec3 {
        b.map(f32::from_bits)
    }

    /// `GetDirection`'s right-dot stays on the FPU: here the old `float` dot was exactly -0.6f
    /// (not left), mp.dll's `z, y, x` sum is -0.60000003 (left).
    #[test]
    fn direction_dots_are_unrounded() {
        let right = v([0xbd2b_9c9d, 0xbf2e_36b3, 0x3ef0_34fe]);
        let dir = v([0xbf45_ccc8, 0xbe5e_c3c1, 0xbfd4_ebfd]);
        let old = right[0] * dir[0] + right[1] * dir[1] + right[2] * dir[2];
        assert_eq!(old, -0.6f32);
        assert_eq!(direction_in::<Msvc>([1.0, 0.0, 0.0], right, dir, 0.6, 0.3), 0);
        // An exact basis gives the same answer in both builds.
        let (fwd, rt) = ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        for (d, want) in [([1.0, 0.0, 0.0], 2), ([-1.0, 0.0, 0.0], 3), ([0.0, -1.0, 0.0], 0), ([0.0, 1.0, 0.0], 1)] {
            assert_eq!(direction_in::<Msvc>(fwd, rt, d, 0.6, 0.3), want);
            assert_eq!(direction_in::<Gcc>(fwd, rt, d, 0.6, 0.3), want);
        }
    }

    /// The 80 u/s stunt gates: the old `y² + x²` in `float` gave exactly 80 for both velocities.
    /// mp.dll's unrounded root is below 80 for the first (HandleStunt and GoRoll take it as slow)
    /// while its stored root is 80; the second's stored root is 79.999992 in both builds.
    #[test]
    fn horizontal_speed_per_build() {
        let a = v([0x41aa_e8b2, 0xc29a_307f, 0]);
        assert_eq!((a[1] * a[1] + a[0] * a[0]).sqrt(), 80.0, "the old float speed");
        assert!(hspeed_n::<Msvc>(a) < 80.0);
        assert_eq!(hspeed_n::<Msvc>(a).f(), 80.0);
        let b = v([0xc195_f122, 0xc29b_8c01, 0]);
        assert_eq!((b[1] * b[1] + b[0] * b[0]).sqrt(), 80.0, "the old float speed");
        assert_eq!(hspeed_n::<Msvc>(b).f().to_bits(), 0x429f_ffff);
        assert_eq!(hspeed_n::<Gcc>(b).f().to_bits(), 0x429f_ffff);
    }

    /// The wall-jump pose's square test: this velocity is square to `right` in the old all-`float`
    /// maths but not in mp.dll's; zero speed always passes.
    #[test]
    fn wall_jump_square_test() {
        let vel = v([0xc322_71e9, 0x431b_50d1, 0]);
        let right = v([0x3f30_e9d9, 0x3f39_08bc, 0]);
        assert!(anim::moving_off_square::<Msvc>(vel, right));
        assert!(anim::moving_off_square::<Msvc>([0.0; 3], right));
        assert!(anim::moving_off_square::<Gcc>([0.0; 3], right));
        assert!(!anim::moving_off_square::<Msvc>([0.0, 5.0, 0.0], [1.0, 0.0, 0.0]));
        assert!(!anim::moving_off_square::<Gcc>([0.0, 5.0, 0.0], [1.0, 0.0, 0.0]));
    }
}
