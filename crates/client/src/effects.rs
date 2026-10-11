//! Entity/temp-entity hooks, TS particle system and map emitters, laser sights. Owner:
//! rendering/prediction work package.
//! Reference: decomp docs/client/rendering-effects-prediction.md §1, §3, §4.6-4.7, §5.
//!
//! Messages handled here (hooked in `init`; the HUD package should not hook them, or should
//! forward to the `msg_*` functions): `TSPSys` (wind), `PTakeDam` (blood), `TSFrags` (glass,
//! explosion, sparks), `TSMBlur` (motion-blur request), `TSBTime` (bullet time).

pub mod emitters;
pub mod fx;
mod laser;
pub mod mblur;
pub mod psys;
pub use laser::laser_color;
mod scope;

use crate::events::ClientBullet;
use crate::{state, view};
use emitters::MapFx;
use goldsrc::Global;
use goldsrc::client::{Cvar, engine};
use goldsrc::ext::render::{self as r, Vec3, vec3 as v};
use goldsrc::sys::*;
use psys::{Camera, PSys};
use shared::protocol::effects as msg;
use std::cell::RefCell;

struct Fx {
    psys: PSys,
    map: MapFx,
    level: String,
    fog: Option<emitters::Fog>,
    /// [`fog_begin`] turned fog on this frame, so [`fog_end`] turns it off.
    fog_on: bool,
    bullets: Vec<ClientBullet>,
    visible: [bool; 33],
    /// `CachePlayerAttachments` (`0x10056e70`, `0x1018caf8`): attachments 0-3 of every player
    /// entity as they stood at the last `TS_DrawEffects`.
    attachments: [[Vec3; 4]; 33],
    /// Sequence number of the newest tracked shell (see [`track_shell`]).
    shell_seq: i32,
    whizz: u32,
    time: f64,
    frametime: f32,
    /// The emitters' own clock (`0x10189478`): `HUD_Frame`'s frame times summed since the map
    /// loaded, which emission is scheduled against.
    emit_clock: f64,
    sprites: Option<laser::Sprites>,
    quality: Quality,
    blur: mblur::MotionBlur,
    draws: u64,
}

struct Cvars {
    fx: Cvar,
    gore: Cvar,
    laser: Cvar,
    lasercolor: Cvar,
    shadow: Cvar,
    motionblur: Cvar,
    paintball: Cvar,
    gl_fog: Cvar,
    /// `gl_fog_start`, `gl_fog_end`: the `gl_fog 2` user fog's linear range.
    fog_range: [Cvar; 2],
    /// `gl_fog_red`, `gl_fog_green`, `gl_fog_blue`: its colour, 0..1.
    fog_color: [Cvar; 3],
}

/// `cl_fx` quality (`FX_ApplyCvar` `0x10057d50`, `FX_AdaptQuality` `0x10057a70`).
#[derive(Clone, Copy)]
struct Quality {
    q: f32,
    samples: [f32; 20],
    n: usize,
    next: f64,
    next_adapt: f64,
    applied: f32,
    /// The particle pool exists (`g_PSys+8`, `0x1018bdf0`): allocated by the first
    /// `TS_DrawEffects` and never freed (`PSys_Init` `0x100507e0` reallocates it in place), so
    /// once set it stays set across level changes. Emitters wait for it (0x1003abef).
    ready: bool,
    /// `TS_DrawEffects` has run since the last `HUD_VidInit` (`0x1018d16a`; set by
    /// `PSys_Startup` 0x10056bd0, cleared by the TS panel's reset 0x1006600f).
    started: bool,
}

impl Default for Quality {
    fn default() -> Self {
        Quality { q: 1.0, samples: [0.0; 20], n: 0, next: 0.0, next_adapt: 0.0, applied: -1.0, ready: false, started: false }
    }
}

static FX: Global<RefCell<Option<Fx>>> = Global(RefCell::new(None));
static CVARS: Global<RefCell<Option<Cvars>>> = Global(RefCell::new(None));

fn with<R>(f: impl FnOnce(&mut Fx) -> R) -> R {
    let mut g = FX.0.borrow_mut();
    let fx = g.get_or_insert_with(|| Fx {
        psys: PSys::default(),
        map: MapFx::default(),
        level: String::new(),
        fog: None,
        fog_on: false,
        bullets: Vec::new(),
        visible: [false; 33],
        attachments: [[v::ZERO; 4]; 33],
        shell_seq: 0,
        whizz: 0,
        time: 0.0,
        frametime: 0.0,
        emit_clock: 0.0,
        sprites: None,
        quality: Quality::default(),
        blur: mblur::MotionBlur::default(),
        draws: 0,
    });
    f(fx)
}

fn cvar(f: impl FnOnce(&Cvars) -> Cvar) -> f32 {
    CVARS.0.borrow().as_ref().map_or(0.0, |c| f(c).value())
}

/// Runs `f` with the particle system. Don't call back into `effects` inside `f`.
pub fn with_psys<R>(f: impl FnOnce(&mut PSys) -> R) -> R {
    with(|fx| f(&mut fx.psys))
}

/// `HUD_TempEntUpdate`'s `cl_gravity` (`0x1001bd25`): the particles fall with it.
pub fn set_cl_gravity(gravity: f32) {
    with(|fx| fx.psys.gravity = gravity);
}

/// Effects clock (client time, seconds).
pub fn now() -> f64 {
    with(|fx| fx.time)
}

pub fn player_visible(idx: i32) -> bool {
    with(|fx| fx.visible.get(idx as usize).copied().unwrap_or(false))
}

/// `0x10056be0(out, entindex, n)`: player `entindex`'s attachment `n` from the cache that
/// [`cache_player_attachments`] fills at the last `TS_DrawEffects`. TS reads every player
/// attachment in its weapon events from here, not from the live entity: the engine resets a
/// player's attachments to its origin when it links the frame's entities (FWGS `CL_LinkPlayers`),
/// which runs before the events fire and before the studio renderer recomputes them. TS doesn't
/// range-check either index; out-of-range ones read zero here.
pub fn player_attachment(entindex: i32, n: usize) -> Vec3 {
    with(|fx| attachment_slot(&fx.attachments, entindex, n))
}

fn attachment_slot(cache: &[[Vec3; 4]; 33], entindex: i32, n: usize) -> Vec3 {
    usize::try_from(entindex).ok().and_then(|i| cache.get(i)).and_then(|a| a.get(n)).copied().unwrap_or(v::ZERO)
}

/// `CachePlayerAttachments` (`0x10056e70`, the copy at `0x10056eb4`): entities 0..=maxclients
/// whose `player` flag is set (`+0x4`) copy their four attachments (`+0xb60`) into the cache;
/// the rest keep their old entries. The positions are whatever the engine left there this frame:
/// the studio renderer's for a player it drew, the entity origin for one it linked but didn't
/// draw. On FWGS that includes the local player in first person (`CL_AddVisibleEntity` passes it
/// to `HUD_AddEntity` but doesn't draw it), so TS's first-person muzzle flashes and pellet spray
/// start at the player's origin.
fn cache_player_attachments(fx: &mut Fx) {
    for i in 0..=engine().max_clients().min(32) {
        if let Some(e) = engine().entity(i).filter(|e| e.player != 0) {
            fx.attachments[i as usize] = e.attachment;
        }
    }
}

pub fn paintball() -> bool {
    cvar(|c| c.paintball) != 0.0
}

/// Shell life (`0x1018ca54`: 30, 5 with `cl_fx 0`, `max(30q, 2)` adaptive).
pub fn shell_life() -> f32 {
    with(|fx| fx.psys.tuning.shell_life)
}

/// TS keeps at most 64 shells: when a new one arrives, the oldest is killed (TS kept a ring of
/// pointers and set the old shell's `die`). Here each shell gets a sequence number in
/// `curstate.iuser2`, and the temp-entity update kills shells that fell out of the last 64.
pub fn track_shell(t: &mut TEMPENTITY) {
    let seq = with(|fx| {
        fx.shell_seq += 1;
        fx.shell_seq
    });
    t.entity.curstate.iuser2 = seq;
}

/// True if the shell with sequence number `seq` (from [`track_shell`]) is older than the newest 64.
pub fn shell_expired(seq: i32) -> bool {
    seq > 0 && with(|fx| fx.shell_seq - seq >= 64)
}

/// Whizz sounds alternate between channels 503 and 504.
pub fn next_whizz_channel() -> i32 {
    with(|fx| {
        let chan = 503 + (fx.whizz & 1) as i32;
        fx.whizz = fx.whizz.wrapping_add(1);
        chan
    })
}

pub fn add_client_bullet(b: ClientBullet) {
    with(|fx| {
        if fx.bullets.len() >= 256 {
            fx.bullets.remove(0);
        }
        fx.bullets.push(b)
    });
}

/// Impact particles at a bullet hit (`TS_ImpactParticles`); `bullet_type` 1 buckshot, 2 exit.
pub fn impact(pos: Vec3, normal: Vec3, tex: u8, bullet_type: i32) {
    with(|fx| {
        let t = fx.time;
        fx::impact(&mut fx.psys, t, pos, normal, tex, bullet_type)
    });
}

/// The effects cvars: `cl_fx`, `cl_gore`, `cl_laser*`, `cl_shadow` and `cl_motionblur` are
/// `InitInput`'s, so this shares those handles rather than registering the names again (see
/// [`Cvar`]); `r_paintball`, `gl_fog` and `gl_fog_start/end/red/green/blue` are registered here,
/// as `CHud::Init` (10020170) does, since no other module reads them. Runs after `input::init`.
pub fn init() {
    let i = crate::input::cvars();
    *CVARS.0.borrow_mut() = Some(Cvars {
        fx: i.cl_fx,
        gore: i.cl_gore,
        laser: i.cl_laser,
        lasercolor: i.cl_lasercolor,
        shadow: i.cl_shadow,
        motionblur: i.cl_motionblur,
        paintball: Cvar::register(c"r_paintball", c"0", r::FCVAR_ARCHIVE),
        gl_fog: Cvar::register(c"gl_fog", c"0", r::FCVAR_ARCHIVE),
        fog_range: [Cvar::register(c"gl_fog_start", c"0", 0), Cvar::register(c"gl_fog_end", c"2048", 0)],
        fog_color: [
            Cvar::register(c"gl_fog_red", c".5", 0),
            Cvar::register(c"gl_fog_green", c".5", 0),
            Cvar::register(c"gl_fog_blue", c".5", 0),
        ],
    });
    let e = goldsrc::client::engine();
    e.add_command(c"ts_test_fx", cmd_test_fx);
}

/// `ts_test_fx slowmo <scale> | blood | glass | explosion | sparks | wind <speed> | bullettime <0|1>
/// | mblur [entindex] [level]`: feeds the message handlers locally (developer test; the server
/// package sends the real ones). `mblur` defaults to the local player at level 2 (its ghosts show
/// in third person with `cl_motionblur 2`, and only while slowed).
extern "C" fn cmd_test_fx() {
    let e = goldsrc::client::engine();
    let what = e.cmd_argv(1).to_string_lossy().into_owned();
    let num = e.cmd_argv(2).to_str().unwrap_or("").parse::<f32>().unwrap_or(0.0);
    let Some(p) = engine().local_player() else { return };
    let (f, _, _) = v::angle_vectors(e.view_angles());
    let at = v::ma(v::add(p.origin, [0.0, 0.0, 16.0]), 96.0, f);
    let mut b = Vec::new();
    match what.as_str() {
        "stats" => {
            let line = with(|fx| {
                let live = fx.map.emitters.iter().filter(|e| !e.dead).count();
                let first = fx
                    .map
                    .emitters
                    .first()
                    .map(|e| format!("{:?} next {:.2} tmpl {:?}", e.origin, e.next_emit, e.template))
                    .unwrap_or_default();
                format!(
                    "draws {} ft {:.4} particles {} / max {} div {} emitters {} live {} first {} time {:.2} wind {:.1}\n",
                    fx.draws,
                    fx.frametime,
                    fx.psys.count(),
                    fx.psys.max,
                    fx.psys.divisor,
                    fx.map.emitters.len(),
                    live,
                    first,
                    fx.time,
                    fx.psys.wind.speed(fx.time)
                )
            });
            e.print(&line);
            let cam = view::camera();
            let d = with(|fx| fx.psys.debug(&cam));
            e.print(&format!("{d}\n"));
        }
        "slowmo" => state::with(|s| s.time_scale_target = if num > 0.0 { num } else { 1.0 }),
        "bullettime" => on_tsbtime(&[num as u8]),
        "blood" => {
            msg::PTakeDam { pos: at, dir: f, amount: 40 }.encode(&mut b);
            on_ptakedam(&b);
        }
        "glass" => {
            msg::TsFrags::Glass { origin: at, size: [32.0, 32.0, 48.0], hit: at, dir: f, dmg_bits: 2, force: 200.0 }.encode(&mut b);
            on_tsfrags(&b);
        }
        "explosion" => {
            msg::TsFrags::Explosion { origin: at, radius: 300.0 }.encode(&mut b);
            on_tsfrags(&b);
        }
        "sparks" => {
            msg::TsFrags::Sparks { origin: at, dir: v::scale(f, -1.0) }.encode(&mut b);
            on_tsfrags(&b);
        }
        "wind" => {
            msg::TsWind { seed: 1234, yaw: 30, speed: num }.encode(&mut b);
            on_tspsys(&b);
        }
        "mblur" => {
            let ent = if num >= 1.0 { num as u8 } else { p.index as u8 };
            let level = e.cmd_argv(3).to_str().unwrap_or("").parse::<u8>().unwrap_or(2);
            msg::TsMBlur { entindex: ent, level }.encode(&mut b);
            on_tsmblur(&b);
        }
        _ => e.print("ts_test_fx slowmo <s>|bullettime <0|1>|blood|glass|explosion|sparks|wind <speed>|mblur [ent] [level]\n"),
    }
}

pub fn vid_init() {
    with(|fx| {
        fx.psys.reset_sprites();
        fx.sprites = None;
        fx.level.clear();
        fx.bullets.clear();
        fx.shell_seq = 0;
        fx.blur.reset();
        // HUD_VidInit -> VGui_Startup -> the TS panel's reset (0x1006600f). The pool pointer that
        // gates emitters is untouched, so they keep emitting until the next `TS_DrawEffects`.
        fx.quality.started = false;
    });
}

// ---- messages ------------------------------------------------------------------------------

/// `TSPSys` (`0x1004bfd0`): subtype 3 = wind.
pub fn on_tspsys(b: &[u8]) {
    if let Some(w) = msg::TsWind::decode(b) {
        with(|fx| {
            let t = fx.time;
            fx.psys.wind.set(w.seed, w.yaw, w.speed, t)
        });
    }
}

/// `PTakeDam` (`0x1004bcd0`) -> `TS_BloodImpact`.
pub fn on_ptakedam(b: &[u8]) {
    let Some(d) = msg::PTakeDam::decode(b) else { return };
    let gore = cvar(|c| c.gore);
    with(|fx| {
        let t = fx.time;
        fx::blood(&mut fx.psys, t, d.pos, d.dir, d.strength(), d.kevlar(), gore)
    });
}

/// `TSFrags` (`0x1004bd50`).
pub fn on_tsfrags(b: &[u8]) {
    let Some(f) = msg::TsFrags::decode(b) else { return };
    with(|fx| {
        let t = fx.time;
        match f {
            msg::TsFrags::Glass { origin, size, hit, dir, dmg_bits, force } => {
                fx::glass(&mut fx.psys, t, origin, size, dir, hit, dmg_bits, force)
            }
            // TS passes the radius coord truncated to an int.
            msg::TsFrags::Explosion { origin, radius } => fx::explosion(&mut fx.psys, t, origin, radius.trunc()),
            // TS passes an uninitialised direction here (0x1004be0c); the message's is used.
            msg::TsFrags::Sparks { origin, dir } => {
                fx::scintille_metal(&mut fx.psys, t, origin, dir, 9.0);
                fx::sparks(&mut fx.psys, t, origin, dir, 7, 1.15, 25.0);
            }
        }
    });
}

/// `TSMBlur` (`0x1004bf90` -> `MotionBlur_Add` `0x10029680`): start or lengthen a ghost trail.
pub fn on_tsmblur(b: &[u8]) {
    if let Some(m) = msg::TsMBlur::decode(b) {
        let (blur, fxq) = (cvar(|c| c.motionblur), cvar(|c| c.fx));
        let now = crate::hud::now();
        with(|fx| fx.blur.add(m.entindex as usize, m.level, blur, fxq, now));
    }
}

/// `TSBTime` (`0x1004b8e0`).
pub fn on_tsbtime(b: &[u8]) {
    if let Some(bt) = msg::decode_btime(b) {
        state::with(|s| s.bullet_time = bt);
    }
}

// ---- per frame -------------------------------------------------------------------------------

/// Called from `view::frame` (HUD_Frame): map change detection, emitter loading and emission
/// (`CTS_CL_Emitter::Frame` `0x1003ab40`). `frametime` is `HUD_Frame`'s argument, unclamped: the
/// emitter clock adds it as is, and the particle system, client bullets and `FX_AdaptQuality`
/// read TS's copy of `ref_params_t::frametime` (`0x10191478`), which the engine fills from the
/// same `host.frametime` (FWGS `V_SetupRefDef`). `cl_fx` is applied and adapted later in the
/// frame, from [`draw_transparent`], as TS does.
pub fn frame(time: f64, frametime: f64) {
    with(|fx| {
        fx.time = time;
        fx.frametime = frametime as f32;
    });
    let level = r::level_name();
    let changed = with(|fx| fx.level != level);
    if changed {
        let map = if level.is_empty() { None } else { r::load_file(&level) };
        let fxmap = map.as_deref().and_then(emitters::entity_lump).map(MapFx::from_lump).unwrap_or_default();
        if !level.is_empty() {
            goldsrc::client::engine().print(&format!(
                "[ts] effects: {} emitters, {} particle templates{} from {level}\n",
                fxmap.emitters.len(),
                fxmap.templates.len(),
                if fxmap.fog.is_some() { ", fog" } else { "" }
            ));
        }
        with(|fx| {
            fx.psys.clear();
            fx.fog = fxmap.fog;
            fx.map = fxmap;
            fx.level = level;
            fx.bullets.clear();
            fx.emit_clock = 0.0;
        });
    }
    let ts = state::with(|s| s.time_scale);
    let cam = view::camera();
    let cull_origin = if view::is_third_person() { view::eye() } else { cam.origin };
    with(|fx| {
        fx.psys.cam = cam;
        fx.psys.cull_origin = cull_origin;
        fx.psys.time_scale = ts;
        // 0x1003abef: no emission, and no emitter clock, until the first `TS_DrawEffects` has
        // allocated the particle pool (0x10056bc0). TS tests the pool pointer, not 0x1018d16a,
        // so this does not wait again after a `HUD_VidInit`.
        if fx.level.is_empty() || !fx.quality.ready {
            return;
        }
        fx.emit_clock += frametime;
        let (clock, t) = (fx.emit_clock, fx.time);
        fx.map.emit(&mut fx.psys, &cam, clock, t, ts);
    });
}

/// The start of `TS_DrawEffects` (`0x10056dd0..0x10056e04`). On the first call after every
/// `HUD_VidInit` (`0x1018d16a` clear): `FX_ApplyCvar` (`0x10057d50`), `PSys_Startup`
/// (`0x10056bc0` -> `PSys_Init` `0x100507e0`: a fresh pool, sprite cache and cooldowns) and the
/// client bullet reset (`0x10034eb0`). Otherwise `FX_ApplyCvar` whenever `cl_fx` differs from the
/// value last applied (`0x1018cac4`).
fn apply_cvar(fx: &mut Fx, cl_fx: f32) {
    let startup = !fx.quality.started;
    if startup {
        fx.psys.reset_sprites();
        fx.psys.clear();
        fx.bullets.clear();
    }
    let q = &mut fx.quality;
    q.ready = true;
    q.started = true;
    if startup || cl_fx != q.applied {
        // cl_fx 1 and 2 start at full quality, anything else is low.
        q.applied = cl_fx;
        let full = cl_fx == 1.0 || cl_fx == 2.0;
        let ps = &mut fx.psys;
        ps.max = if full { 4096 } else { 2048 };
        ps.divisor = if full { 1.0 } else { 8.0 };
        ps.physics = full;
        ps.tuning = if full {
            psys::FxTuning::default()
        } else {
            psys::FxTuning { mult: 0.7, interval: 0.2, puff_interval: 0.3, dlights: false, shell_life: 5.0, scale: 0.35, smoke: true }
        };
        q.samples = [0.0; 20];
        q.n = 0;
        q.next = 0.0;
        q.next_adapt = 0.0;
    }
    fx.psys.tuning.smoke = cl_fx != 0.0;
}

/// `FX_AdaptQuality` (`0x10057a70`), near the end of `TS_DrawEffects` (`0x10056e45`) and only
/// while `cl_fx` is exactly 1. It runs after the particles were updated and drawn, so what it
/// decides takes effect from the next frame's emission and particle pass. It samples the frame
/// time (`0x10191478`) every 0.1 s into a ring of 20; every second it derives the quality from
/// the ring's mean (in 1/100 s, 1.11..3.33 maps to 1..0).
fn adapt_quality(fx: &mut Fx, cl_fx: f32) {
    if cl_fx != 1.0 {
        return;
    }
    let frametime = fx.frametime;
    let q = &mut fx.quality;
    if fx.time < q.next {
        return;
    }
    q.next = fx.time + 0.1;
    q.samples[q.n % 20] = frametime * 100.0;
    q.n += 1;
    if fx.time < q.next_adapt {
        return;
    }
    q.next_adapt = fx.time + 1.0;
    let avg = q.samples.iter().sum::<f32>() * 0.05;
    q.q = 1.0 - (avg.clamp(1.11, 3.33) - 1.11) * 0.450_450_45;
    if q.q == 0.0 {
        return;
    }
    let qq = q.q;
    let ps = &mut fx.psys;
    let interval = (0.05 / qq).min(0.3);
    let smoke = ps.tuning.smoke;
    ps.tuning = psys::FxTuning {
        mult: (qq * 2.0).max(0.1),
        interval,
        puff_interval: interval,
        dlights: qq >= 0.3,
        shell_life: (qq * 30.0).max(2.0),
        scale: qq.max(0.25),
        smoke,
    };
    ps.divisor = if qq < 0.374 {
        6.0
    } else if qq < 0.599 {
        4.0
    } else if qq < 0.749 {
        2.0
    } else {
        1.0
    };
    ps.physics = qq > 0.374;
}

/// `HUD_AddEntity` (`0x100447f0`).
pub fn add_entity(kind: i32, e: &mut cl_entity_t) -> bool {
    let cam = view::camera();
    if kind == 1 {
        let idx = e.index;
        let spectator = (1..=32).contains(&idx) && state::with(|s| s.spectator[(idx - 1) as usize]);
        // TS hides the in-eye target with mode 5 (`g_iUser1 == 5`), its numbering for in-eye.
        let in_eye_target = state::with(|s| s.spec_mode == shared::ts::obs::IN_EYE && s.spec_target == idx);
        let d = v::dist(e.origin, cam.origin);
        let hemisphere = d < 256.0 || v::dot(v::sub(e.origin, cam.origin), cam.forward) >= 0.0;
        let seen = hemisphere && !spectator && occlusion_visible(e, &cam);
        // TS adds the overview icon after its visibility tests, before hiding the in-eye target.
        if seen {
            crate::spectator::add_overview_entity(kind, e);
        }
        let visible = seen && !in_eye_target;
        with(|fx| {
            if let Some(v) = fx.visible.get_mut(idx as usize) {
                *v = visible;
            }
        });
        return visible;
    }
    // Item fade / cull by distance, `curstate.iuser4` bits (server sets them).
    let bits = e.curstate.iuser4;
    let fov = state::with(|s| s.fov);
    let (max, start, k) = if bits & 1 != 0 {
        (592.0, 492.0, 2.55)
    } else if bits & 4 != 0 {
        (256.0, 206.0, 5.1)
    } else if bits & 2 != 0 {
        (1024.0, 824.0, 1.275)
    } else {
        return true;
    };
    let max = if fov < 40.0 { max * 2.0 } else { max };
    let d = v::dist(e.origin, cam.origin);
    // `InViewCone` measures from `GetCullOrigin` (this frame's, set by `frame`).
    let cull = with(|fx| fx.psys.cull_origin);
    if d >= max || !psys::in_view_cone(&cam, cull, e.origin) {
        return false;
    }
    if fov >= 80.0 && d > start {
        e.curstate.rendermode = kRenderTransColor as i32;
        e.curstate.renderfx = 0;
        e.curstate.renderamt = ((max - d) * k).clamp(0.0, 255.0) as i32;
    }
    true
}

/// `ThirdPersonOcclusionVisible` (`0x10057330`): in third person the camera may not reveal
/// players the eye can't see.
fn occlusion_visible(e: &cl_entity_t, cam: &Camera) -> bool {
    if !view::is_third_person() {
        return true;
    }
    let eye = view::eye();
    let (_, right, up) = v::angle_vectors(cam.angles);
    for ofs in [[0.0f32, 0.0], [32.0, 32.0], [-32.0, 32.0], [32.0, -32.0], [-32.0, -32.0]] {
        let p = v::ma(v::ma(e.origin, ofs[0], right), ofs[1], up);
        let tr = r::player_trace(eye, p, 2, (PM_GLASS_IGNORE | PM_WORLD_ONLY) as i32, -1);
        if tr.fraction >= 1.0 {
            return true;
        }
    }
    false
}

/// `HUD_CreateEntities` (`0x1001bce0`): client bullets, then the motion-blur ghosts
/// (`MotionBlur_Update` `0x10029780`).
pub fn create_entities() {
    let mut bullets = with(|fx| std::mem::take(&mut fx.bullets));
    if !bullets.is_empty() {
        let ft = with(|fx| fx.frametime);
        crate::events::client_bullets_frame(&mut bullets, ft);
    }
    with(|fx| {
        let mut b = bullets;
        b.append(&mut fx.bullets);
        fx.bullets = b;
    });

    let (blur, ts, now) = (cvar(|c| c.motionblur), state::with(|s| s.time_scale), crate::hud::now());
    for ent in with(|fx| fx.blur.due(blur, ts, now)) {
        if mblur::spawn_ghost(ent) {
            with(|fx| fx.blur.spawned(ent));
        }
    }
}

pub fn draw_normal() {}

/// `HUD_DrawTransparentTriangles` -> `TS_DrawEffects` (`0x10056dd0`).
pub fn draw_transparent() {
    let fxq = cvar(|c| c.fx);
    with(|fx| {
        apply_cvar(fx, fxq);
        cache_player_attachments(fx);
    });
    let cam = view::camera();
    let ts = state::with(|s| s.time_scale);
    let third = view::is_third_person();
    let (laser_mode, color, shadow) = (cvar(|c| c.laser), cvar(|c| c.lasercolor) as i32, cvar(|c| c.shadow));
    let (visible, has_sprites) = with(|fx| (fx.visible, fx.sprites.is_some()));
    if !has_sprites {
        let s = laser::Sprites::load();
        with(|fx| fx.sprites = Some(s));
    }
    let sprites = with(|fx| fx.sprites.take()).unwrap();
    if cam_hardware() {
        laser::draw_shadows(&sprites, &cam, &visible, shadow, third);
    }
    let now = now();
    laser::draw_local(&sprites, &cam, now, laser_mode, color);
    laser::draw_others(&sprites, &cam, &visible);
    let scope_sprite = sprites.scope;
    with(|fx| fx.sprites = Some(sprites));

    let players = react_players();
    let (dt, requests, blood_hits) = with(|fx| {
        let dt = fx.frametime * ts;
        fx.draws += 1;
        let t = fx.time;
        fx.psys.update(t, dt, &cam, &players);
        (dt, std::mem::take(&mut fx.psys.spawn_requests), std::mem::take(&mut fx.psys.blood_hits))
    });
    blood_decals(&blood_hits);
    with(|fx| {
        let t = fx.time;
        for (tmpl, pos) in requests {
            fx.map.spawn_one(&mut fx.psys, tmpl, pos, t);
        }
        fx.psys.draw(t, dt, &cam);
    });
    if cam_hardware() {
        scope::draw(scope_sprite, &cam, third);
    }
    let fxq = cvar(|c| c.fx);
    with(|fx| {
        fx.visible = [false; 33];
        adapt_quality(fx, fxq);
    });
    fog_end();
}

/// Players that push reacting particles (`Particle_Think` 0x10051270): every solid player, with
/// the local player's predicted velocity (`simvel`) and the others' velocity between their last
/// two updates.
fn react_players() -> Vec<(Vec3, Vec3)> {
    let local = engine().local_player().map_or(0, |e| e.index);
    let local_vel = state::with(|s| s.local_velocity);
    let mut players = Vec::new();
    for i in 1..=engine().max_clients().min(32) {
        let Some(e) = engine().entity(i) else { continue };
        if e.player == 0 || e.curstate.solid == 0 {
            continue;
        }
        let vel = if i == local {
            local_vel
        } else {
            let dt = e.curstate.msg_time - e.prevstate.msg_time;
            if dt > 0.0 { v::scale(v::sub(e.curstate.origin, e.prevstate.origin), 1.0 / dt) } else { v::ZERO }
        };
        players.push((e.origin, vel));
    }
    players
}

/// `TS_BloodImpact`'s collision callback (`0x10048710`): a `{blood1..8}` decal where a drop hit
/// (with `r_decals`), and `player/splat.wav` one time in ten.
fn blood_decals(hits: &[Vec3]) {
    for &pos in hits {
        let name = format!("{{blood{}", r::random_long(1, 8));
        if goldsrc::client::engine().cvar_float(c"r_decals") != 0.0 {
            let idx = r::decal_index(&name);
            if idx != 0 {
                r::decal_shoot(idx, 0, 0, pos, 0);
            }
        }
        if r::random_long(0, 100) > 90 {
            engine().ev_play_sound(-1, pos, CHAN_AUTO as i32, "player/splat.wav", 1.0, 0.99, 0, crate::events::slow_pitch());
        }
    }
}

fn cam_hardware() -> bool {
    view::hardware()
}

/// `V_SetFog` (`0x10056c20`), through TriAPI instead of raw OpenGL (linear fog either way).
pub fn fog_begin() {
    let user = CVARS
        .0
        .borrow()
        .as_ref()
        .map_or(([0.0; 3], 0.0, 0.0), |c| (c.fog_color.map(|v| v.value()), c.fog_range[0].value(), c.fog_range[1].value()));
    let single = engine().max_clients() == 1;
    if let Some((color, start, end)) = fog_params(single, cvar(|c| c.gl_fog), with(|fx| fx.fog), user) {
        goldsrc::ext::tri::fog(color, start, end, true);
        with(|fx| fx.fog_on = true);
    }
}

/// `V_SetFog`'s choice: in single player with `gl_fog 2` the user's `gl_fog_red/green/blue`
/// (`user.0`) and `gl_fog_start/end`; otherwise the map's `ts_fog` while `gl_fog` is on.
/// Returns (colour 0..255 for TriAPI, start, end). Both colour sources are 0..1, as TS feeds
/// them to `glFogfv`; a `ts_fog` colour above 1 is taken as already 0..255.
fn fog_params(single: bool, gl_fog: f32, map: Option<emitters::Fog>, user: (Vec3, f32, f32)) -> Option<(Vec3, f32, f32)> {
    if single && gl_fog == 2.0 {
        Some((user.0.map(|x| x * 255.0), user.1, user.2))
    } else if gl_fog != 0.0 {
        map.map(|f| (f.color.map(|x| if x <= 1.0 { x * 255.0 } else { x }), f.near, f.far))
    } else {
        None
    }
}

/// TS disables `GL_FOG` at the end of the transparent pass (`0x10056e5e`).
fn fog_end() {
    if with(|fx| std::mem::take(&mut fx.fog_on)) {
        goldsrc::ext::tri::fog([0.0; 3], 0.0, 0.0, false);
    }
}

/// `HUD_StudioEvent` (`0x10044b50`): only 5002 (sparks) and 5004 (sound) remain.
pub fn studio_event(ev: &mstudioevent_s, ent: &cl_entity_t) {
    let opts = goldsrc::util::c_array_str(&ev.options);
    match ev.event {
        5002 => r::spark_effect(ent.attachment[0], opts.trim().parse().unwrap_or(0), -100, 100),
        5004 => engine().ev_play_sound(ent.index, ent.attachment[0], CHAN_BODY as i32, &opts, 1.0, 0.8, 0, crate::events::slow_pitch()),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fog_sources() {
        let map = Some(emitters::Fog { near: 100.0, far: 900.0, color: [0.2, 0.4, 1.0] });
        let user = ([0.5, 0.5, 0.5], 0.0, 2048.0);
        assert_eq!(fog_params(true, 0.0, map, user), None, "gl_fog 0: no fog");
        assert_eq!(fog_params(true, 1.0, map, user), Some(([51.0, 102.0, 255.0], 100.0, 900.0)));
        assert_eq!(fog_params(true, 2.0, map, user), Some(([127.5; 3], 0.0, 2048.0)), "gl_fog 2 in single player: the user's");
        assert_eq!(fog_params(true, 2.0, None, user), Some(([127.5; 3], 0.0, 2048.0)), "...even without ts_fog");
        assert_eq!(fog_params(false, 2.0, map, user), Some(([51.0, 102.0, 255.0], 100.0, 900.0)), "multiplayer: the map's");
        assert_eq!(fog_params(false, 2.0, None, user), None);
    }

    #[test]
    fn attachment_cache_slots() {
        let mut cache = [[v::ZERO; 4]; 33];
        cache[3][2] = [1.0, 2.0, 3.0];
        cache[32][3] = [4.0, 5.0, 6.0];
        assert_eq!(attachment_slot(&cache, 3, 2), [1.0, 2.0, 3.0]);
        assert_eq!(attachment_slot(&cache, 32, 3), [4.0, 5.0, 6.0]);
        assert_eq!(attachment_slot(&cache, 3, 0), v::ZERO);
        assert_eq!(attachment_slot(&cache, -1, 0), v::ZERO, "out of range reads zero");
        assert_eq!(attachment_slot(&cache, 33, 0), v::ZERO);
        assert_eq!(attachment_slot(&cache, 3, 4), v::ZERO);
    }
}
