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
pub use laser::{LaserStyle, laser_color};
mod scope;

use crate::events::ClientBullets;
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
    /// Whether [`Fx::map`] was built with the map's effects: the ruleset's answer when the map
    /// was last read (true before any, as TS 3.0's).
    map_effects: bool,
    fog: Option<emitters::Fog>,
    /// `0x100eb678`.
    bullets: ClientBullets,
    visible: [bool; 33],
    /// `CachePlayerAttachments` (`0x10056e70`, `0x1018caf8`): attachments 0-3 of every player
    /// entity as they stood at the last `TS_DrawEffects`.
    attachments: [[Vec3; 4]; 33],
    whizz: u32,
    time: f64,
    frametime: f32,
    /// The emitters' own clock (`0x10189478`): `HUD_Frame`'s frame times summed since the map
    /// loaded, which emission is scheduled against.
    emit_clock: f64,
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
        map_effects: true,
        fog: None,
        bullets: ClientBullets::default(),
        visible: [false; 33],
        attachments: [[v::ZERO; 4]; 33],
        whizz: 0,
        time: 0.0,
        frametime: 0.0,
        emit_clock: 0.0,
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

/// Shell life (`0x1018ca54`: 30, 5 with `cl_fx 0`, `max(30q, 2)` adaptive). Every shell lives
/// this long: `EV_TS_EjectShell` (`0x10056890`) zeroes the `die` of the shell in its 64-slot ring
/// (`0x1018bbc8`, counter `0x1018bcc8`), but nothing ever stores a shell there, so the ring stays
/// empty and TS never removes an old shell early.
pub fn shell_life() -> f32 {
    with(|fx| fx.psys.tuning.shell_life)
}

/// Whizz sounds alternate between channels 503 and 504.
pub fn next_whizz_channel() -> i32 {
    with(|fx| {
        let chan = 503 + (fx.whizz & 1) as i32;
        fx.whizz = fx.whizz.wrapping_add(1);
        chan
    })
}

/// `ClientBullets_Add` (`0x10035c20`) from `EV_TS_FireBullets`.
pub fn add_client_bullet(shooter: i32, src: Vec3, dir: Vec3, dmg: f32, ammo: i32, bullet_type: i32) {
    with(|fx| fx.bullets.add(shooter, src, dir, dmg, ammo, bullet_type));
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

/// The effects side of `HUD_VidInit`: only the TS panel's reset (`HUD_VidInit -> VGui_Startup`,
/// `0x1006600f`) clears the "started" flag, so the next `TS_DrawEffects` rebuilds the pool and
/// sprite cache. Until then the old pool, sprite cache, client bullets, map emitters (the level
/// name is kept, so reloading the same map doesn't reparse them) and motion-blur trails stay, as
/// in TS (`MotionBlur_Reset` `0x10029660` has no caller).
pub fn vid_init() {
    with(|fx| {
        fx.quality.started = false;
        fx.bullets.models.forget();
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
/// The handler reads two bytes without checking the length; a missing byte reads as -1
/// (`READ_BYTE` `0x10029a10`), so a one-byte message has level -1 (5 ghosts). An empty message
/// (entity -1) is dropped: TS would write before its trail table.
pub fn on_tsmblur(b: &[u8]) {
    let Some((ent, level)) = tsmblur_args(b) else { return };
    let (blur, fxq) = (cvar(|c| c.motionblur), cvar(|c| c.fx));
    let now = crate::hud::now();
    with(|fx| fx.blur.add(ent, level, blur, fxq, now));
}

/// `TSMBlur`'s (entity, signed level) as TS reads them, `None` for an empty message.
fn tsmblur_args(b: &[u8]) -> Option<(usize, i8)> {
    let &ent = b.first()?;
    Some((ent as usize, b.get(1).map_or(-1, |&l| l as i8)))
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
    // A server on another ruleset rebuilds them too: with or without the map's effects
    // ([`ClientRules::map_effects`](crate::gameplay::ClientRules::map_effects)).
    let map_effects = crate::gameplay::current().map_effects();
    let changed = with(|fx| fx.level != level || fx.map_effects != map_effects);
    if changed {
        let map = if level.is_empty() || !map_effects { None } else { r::load_file(&level) };
        let fxmap = map.as_deref().and_then(emitters::entity_lump).map(MapFx::from_lump).unwrap_or_default();
        if !level.is_empty() && map_effects {
            goldsrc::client::engine().print(&format!(
                "[ts] effects: {} emitters, {} particle templates{} from {level}\n",
                fxmap.emitters.len(),
                fxmap.templates.len(),
                if fxmap.fog.is_some() { ", fog" } else { "" }
            ));
        }
        // TS resets the emitters, templates, fog and the emitter clock here (0x1003ab8e..0x1003abc3), not
        // the particle pool or the client bullets.
        with(|fx| {
            fx.fog = fxmap.fog;
            fx.map = fxmap;
            fx.level = level;
            fx.map_effects = map_effects;
            fx.emit_clock = 0.0;
        });
    }
    let ts = state::with(|s| s.time_scale);
    let cam = view::camera();
    let cull_origin = cull_origin(&cam);
    with(|fx| {
        fx.psys.cam = cam;
        fx.psys.cull_origin = cull_origin;
        fx.psys.time_scale = ts;
        fx.psys.hardware = view::hardware();
        // 0x1003abef: no emission, and no emitter clock, until the first `TS_DrawEffects` has
        // allocated the particle pool (0x10056bc0). TS tests the pool pointer, not 0x1018d16a,
        // so this does not wait again after a `HUD_VidInit`.
        if fx.level.is_empty() || !fx.quality.ready || !map_effects {
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
            psys::FxTuning {
                mult: 0.7,
                interval: 0.2,
                puff_interval: 0.3,
                dlights: false,
                low_detail: true,
                shell_life: 5.0,
                scale: 0.35,
                smoke: true,
            }
        };
        q.samples = [0.0; 20];
        q.n = 0;
        q.next = 0.0;
        q.next_adapt = 0.0;
    }
    // `PSys_Startup` with the pool size `FX_ApplyCvar` just chose, then the client bullet reset.
    if startup {
        fx.psys.init();
        fx.bullets.reset();
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
        low_detail: qq < 0.3,
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

/// `GetCullOrigin` (`0x10057270`): the eye in third person, otherwise the view origin. TS's
/// culls measure distances and directions from here.
fn cull_origin(cam: &Camera) -> Vec3 {
    if view::is_third_person() { view::eye() } else { cam.origin }
}

/// `HUD_AddEntity` (`0x100447f0`). Players: the hemisphere, anti-peek and spectator tests, then
/// the visible-this-frame flag. Other entities: the `curstate.iuser4` distance cull and, if the
/// ruleset has it ([`ground_items_fade`](crate::gameplay::ClientRules::ground_items_fade)), fade
/// (the server's items; client shells have 12) and the bit 8 LOD body. Then the ruleset's
/// spectator tail ([`add_entity_spectator`](crate::gameplay::ClientRules::add_entity_spectator)):
/// the overview icon and the in-eye target hidden.
pub fn add_entity(kind: i32, e: &mut cl_entity_t) -> bool {
    let cam = view::camera();
    let cull = cull_origin(&cam);
    let fov = state::with(|s| s.fov);
    if kind == 1 {
        if !in_view_hemisphere(e.origin, cull, cam.forward) || !occlusion_visible(e, &cam) {
            return false;
        }
        let idx = e.index;
        if (1..=32).contains(&idx) && state::with(|s| s.spectator[(idx - 1) as usize]) {
            return false;
        }
        // `g_bPlayerVisible` (`0x1018bd44`, set at `0x1004485a`): also for the in-eye target hidden
        // below, whose shadow, laser and flashlight still draw.
        with(|fx| {
            if let Some(v) = usize::try_from(idx).ok().and_then(|i| fx.visible.get_mut(i)) {
                *v = true;
            }
        });
    } else {
        let bits = e.curstate.iuser4;
        if let Some(cull_dist) = ItemCull::for_bits(bits) {
            if beyond_distance(e.origin, cull, cull_dist.max, fov) || !psys::in_view_cone(&cam, cull, e.origin) {
                return false;
            }
            // 2.1 (`0x1007b4c0`) has no fade: the item is solid up to the cull distance.
            if crate::gameplay::current().ground_items_fade()
                && let Some(amt) = cull_dist.fade(v::dist(e.origin, cull), fov)
            {
                e.curstate.renderfx = 0;
                e.curstate.rendermode = kRenderTransColor as i32;
                e.curstate.renderamt = amt;
            }
        }
        // 0x10044ace: the LOD body, 0 at low detail.
        if bits & 8 != 0 {
            let low = with(|fx| fx.psys.tuning.low_detail);
            e.curstate.body = if low { 0 } else { lod_body(v::dist(e.origin, cull), fov) };
        }
    }
    // 0x10044af9: the SDK spectator code, for every entity that got this far; when it runs and
    // which entity it hides are the ruleset's.
    match crate::gameplay::current().add_entity_spectator() {
        crate::gameplay::SpectatorTail::Skip => true,
        crate::gameplay::SpectatorTail::Overview { hide } => {
            crate::spectator::add_overview_entity(kind, e);
            hide != Some(e.index)
        }
    }
}

/// `InViewHemisphere` (`0x10057790`): within 256 units of the cull origin, or not behind it
/// (the unit direction's dot with the view forward ≥ 0).
fn in_view_hemisphere(org: Vec3, cull: Vec3, forward: Vec3) -> bool {
    let d = v::sub(org, cull);
    let len = v::len(d);
    if len <= 256.0 {
        return true;
    }
    v::dot(v::scale(d, 1.0 / len), forward) >= 0.0
}

/// `BeyondDistance(org, max)` (`0x10057910`): `max` doubles while zoomed in (`m_iFOV < 40`).
fn beyond_distance(org: Vec3, cull: Vec3, max: f32, fov: f32) -> bool {
    let max = if fov < 40.0 { max * 2.0 } else { max };
    v::dist(org, cull) >= max
}

/// `HUD_AddEntity`'s item distances by `curstate.iuser4` bit: the first of 1, 4 and 2 set.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ItemCull {
    /// Cull distance (`0x250`, `0x100`, `0x400`).
    max: f32,
    /// The fade starts beyond this.
    start: f32,
    /// `renderamt` per unit short of `max`.
    k: f32,
}

impl ItemCull {
    fn for_bits(bits: i32) -> Option<ItemCull> {
        let (max, start, k) = if bits & 1 != 0 {
            (592.0, 492.0, 2.55)
        } else if bits & 4 != 0 {
            (256.0, 206.0, 5.1)
        } else if bits & 2 != 0 {
            (1024.0, 824.0, 1.275)
        } else {
            return None;
        };
        Some(ItemCull { max, start, k })
    }

    /// The fade (`0x100448b4..0x10044ac8` and its copies for bits 4 and 2): beyond `start` with
    /// `m_iFOV ≥ 80`, `renderamt = (max − d)·k` truncated. Unzoomed, so `max` isn't doubled
    /// and `d < max` keeps it within 0..=255.
    fn fade(&self, d: f32, fov: f32) -> Option<i32> {
        (d > self.start && fov >= 80.0).then_some(((self.max - d) * self.k) as i32)
    }
}

/// `GetLODBody(org)` (`0x10056fc0`) for distance `d` from the cull origin: zoomed in
/// (`m_iFOV < 80`) a FOV under 14 within 2048 units gives 3 and under 41 within 512 gives 2;
/// otherwise beyond 256 units 0, beyond 128 1, beyond 64 2, else 3.
fn lod_body(d: f32, fov: f32) -> i32 {
    if fov < 80.0 {
        if fov < 14.0 && d < 2048.0 {
            return 3;
        }
        if fov < 41.0 && d < 512.0 {
            return 2;
        }
    }
    if d > 256.0 {
        0
    } else if d > 128.0 {
        1
    } else if d > 64.0 {
        2
    } else {
        3
    }
}

/// `ThirdPersonOcclusionVisible` (`0x10057330`): in third person the camera may not reveal
/// players the eye can't see: `PointVisible` from the eye to the origin and four points 32 units
/// off it.
fn occlusion_visible(e: &cl_entity_t, cam: &Camera) -> bool {
    if !view::is_third_person() {
        return true;
    }
    let eye = view::eye();
    let (_, right, up) = v::angle_vectors(cam.angles);
    for ofs in [[0.0f32, 0.0], [32.0, 32.0], [-32.0, 32.0], [32.0, -32.0], [-32.0, -32.0]] {
        let p = v::ma(v::ma(e.origin, ofs[0], right), ofs[1], up);
        if psys::point_visible(eye, p) {
            return true;
        }
    }
    false
}

/// `HUD_CreateEntities` (`0x1001bce0`): the motion-blur ghosts (`MotionBlur_Update`
/// `0x10029780`). TS first calls `ClientBullets_AddEntities` (`0x10034e80`), whose bullet models
/// are never enabled (`0x1d34d` stays 0), so it adds nothing; the bullets are stepped and drawn in
/// [`draw_transparent`].
pub fn create_entities() {
    // A ruleset's bullet models (none in TS 3.0), while the client bullets run.
    with(|fx| {
        if fx.bullets.count != 0 {
            fx.bullets.models.add_entities();
        }
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
    let (laser_mode, shadow) = (cvar(|c| c.laser), cvar(|c| c.shadow));
    let visible = with(|fx| fx.visible);
    if cam_hardware() {
        laser::draw_shadows(&cam, &visible, shadow, third);
    }
    // `0x1003de00` (local laser, other lasers), then `0x1003bad0` (local flashlight, others').
    let (now, frametime) = (now(), with(|fx| fx.frametime));
    laser::draw_local_laser(&cam, now, frametime, ts, laser_mode);
    laser::draw_other_lasers(&cam, &visible, now, frametime);
    laser::draw_local_flashlight(&cam);
    laser::draw_other_flashlights(&cam, &visible);

    let players = react_players();
    with(|fx| {
        let dt = fx.frametime * ts;
        fx.draws += 1;
        let t = fx.time;
        let Fx { psys, map, .. } = fx;
        psys.update_draw(dt, &cam, &players, &mut |ps, c| match c {
            psys::Collision::Spawn { target, origin, parent_vel } => map.spawn_one(ps, target, origin, parent_vel, t),
            psys::Collision::Blood(tr) => blood_hit(tr),
        });
    });
    if cam_hardware() {
        scope::draw(&cam, third);
    }
    let fxq = cvar(|c| c.fx);
    with(|fx| {
        fx.visible = [false; 33];
        adapt_quality(fx, fxq);
    });
    // `ClientBullets_Draw` (`0x10034e60`): the client bullet frame while the slot counter is set.
    // The bullets are taken out for it: their impacts and trails call back into `effects`.
    if let Some(mut bullets) = with(|fx| (fx.bullets.count != 0).then(|| std::mem::take(&mut fx.bullets))) {
        let cull = cull_origin(&cam);
        let f = crate::events::BulletFrame { cam, cull, frametime: with(|fx| fx.frametime) };
        crate::events::client_bullets_frame(&mut bullets, &f);
        with(|fx| fx.bullets = bullets);
    }
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

/// `TS_BloodImpact`'s collision callback (`0x10048710`), for the trace that hit: a `{bloodN`
/// decal (N rolled first, 1..8) on the entity the trace hit when that is a BSP model or a
/// `MOVETYPE_PUSHSTEP` entity and `r_decals` is on, then `player/splat.wav` one time in ten at
/// the slow-motion pitch. The particle pass then clears the drop's in-use byte.
fn blood_hit(mut tr: pmtrace_s) {
    let name = format!("{{blood{}", r::random_long(1, 8));
    let pe = r::physent(tr.ent);
    if pe.is_some_and(|pe| pe.solid == SOLID_BSP as i32 || pe.movetype == MOVETYPE_PUSHSTEP as i32)
        && goldsrc::client::engine().cvar_float(c"r_decals") != 0.0
    {
        let entity = r::index_from_trace(&mut tr);
        r::decal_shoot(r::decal_index(&name), entity, 0, tr.endpos, 0);
    }
    if r::random_long(0, 100) > 90 {
        engine().ev_play_sound(-1, tr.endpos, CHAN_AUTO as i32, "player/splat.wav", 1.0, 0.99, 0, crate::events::slow_pitch());
    }
}

fn cam_hardware() -> bool {
    view::hardware()
}

/// Whether the server's ruleset runs the map's emitters and fog (`ClientRules::map_effects`).
/// Asked each frame, so a server on another ruleset takes effect at once; the map is parsed
/// either way.
fn map_effects() -> bool {
    crate::gameplay::hud_rules(crate::hud::with(|h| h.gameplay)).map_effects()
}

/// `V_SetFog` (`0x10056c20`, from `V_CalcRefdef`): linear fog through raw OpenGL, as TS calls
/// `opengl32.dll` itself ([`goldsrc::ext::gl`], which does nothing unless the engine draws with
/// OpenGL). The map's fog also needs `IEngineStudio.IsHardware()` to be exactly 1
/// (`0x10056d20`); the single-player user fog doesn't ask.
pub fn fog_begin() {
    let user = CVARS
        .0
        .borrow()
        .as_ref()
        .map_or(([0.0; 3], 0.0, 0.0), |c| (c.fog_color.map(|v| v.value()), c.fog_range[0].value(), c.fog_range[1].value()));
    let single = engine().max_clients() == 1;
    let hardware = goldsrc::ext::studio::hardware_kind;
    let map = with(|fx| fx.fog).filter(|_| map_effects());
    if let Some((color, start, end)) = fog_params(single, cvar(|c| c.gl_fog), map, user, hardware) {
        use goldsrc::ext::gl;
        gl::fogi(gl::GL_FOG_MODE, gl::GL_LINEAR);
        gl::fogfv(gl::GL_FOG_COLOR, &color);
        gl::fogf(gl::GL_FOG_DENSITY, 1.0);
        gl::hint(gl::GL_FOG_HINT, gl::GL_NICEST);
        gl::fogf(gl::GL_FOG_START, start);
        gl::fogf(gl::GL_FOG_END, end);
        gl::enable(gl::GL_FOG);
    }
}

/// `V_SetFog`'s choice: in single player with `gl_fog 2` the user's `gl_fog_red/green/blue`
/// (`user.0`) and `gl_fog_start/end`; otherwise the map's `ts_fog` while `gl_fog` is on and
/// `hardware()` (asked last, as TS does) is 1. Returns the `glFogfv` colour (alpha 1; GL clamps it
/// to 0..1), start and end.
fn fog_params(
    single: bool,
    gl_fog: f32,
    map: Option<emitters::Fog>,
    user: (Vec3, f32, f32),
    hardware: impl FnOnce() -> i32,
) -> Option<([f32; 4], f32, f32)> {
    let rgba = |c: Vec3| [c[0], c[1], c[2], 1.0];
    if single && gl_fog == 2.0 {
        Some((rgba(user.0), user.1, user.2))
    } else {
        let f = map?;
        (gl_fog != 0.0 && hardware() == 1).then(|| (rgba(f.color), f.near, f.far))
    }
}

/// TS disables `GL_FOG` at the end of every transparent pass (`glDisable` at `0x10056e5e`),
/// whatever was drawn. Being raw GL, it also switches off the engine's own fog (FWGS's underwater
/// fog, enabled before the transparent pass) for the rest of the frame, as in TS.
fn fog_end() {
    goldsrc::ext::gl::disable(goldsrc::ext::gl::GL_FOG);
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
        let gl = || 1;
        assert_eq!(fog_params(true, 0.0, map, user, gl), None, "gl_fog 0: no fog");
        assert_eq!(fog_params(true, 1.0, map, user, gl), Some(([0.2, 0.4, 1.0, 1.0], 100.0, 900.0)));
        assert_eq!(
            fog_params(true, 2.0, map, user, gl),
            Some(([0.5, 0.5, 0.5, 1.0], 0.0, 2048.0)),
            "gl_fog 2 in single player: the user's"
        );
        assert_eq!(fog_params(true, 2.0, None, user, gl), Some(([0.5, 0.5, 0.5, 1.0], 0.0, 2048.0)), "...even without ts_fog");
        assert_eq!(fog_params(true, 2.0, map, user, || 0), Some(([0.5, 0.5, 0.5, 1.0], 0.0, 2048.0)), "...and in software");
        assert_eq!(fog_params(false, 2.0, map, user, gl), Some(([0.2, 0.4, 1.0, 1.0], 100.0, 900.0)), "multiplayer: the map's");
        assert_eq!(fog_params(false, 2.0, None, user, gl), None);
        // The map's fog needs IsHardware() == 1 exactly (0x10056d26).
        assert_eq!(fog_params(false, 1.0, map, user, || 0), None, "software");
        assert_eq!(fog_params(false, 1.0, map, user, || 2), None, "Direct3D");
        // The colour goes to glFogfv as is (GL clamps it).
        let bright = Some(emitters::Fog { near: 0.0, far: 1.0, color: [128.0, -1.0, 0.5] });
        assert_eq!(fog_params(false, 1.0, bright, user, gl), Some(([128.0, -1.0, 0.5, 1.0], 0.0, 1.0)));
    }

    #[test]
    fn hemisphere_from_the_cull_origin() {
        let fwd = [1.0, 0.0, 0.0];
        assert!(in_view_hemisphere([-256.0, 0.0, 0.0], v::ZERO, fwd), "256 units behind: within reach");
        assert!(!in_view_hemisphere([-256.5, 0.0, 0.0], v::ZERO, fwd), "further behind");
        assert!(in_view_hemisphere([-1000.0, 1000.0, 0.0], [-1000.0, 0.0, 0.0], fwd), "level with it");
        // Third person, the camera 128 units behind the eye at the origin: a player 300 units
        // behind the eye is 172 from the camera, but TS measures from the eye.
        assert!(!in_view_hemisphere([-300.0, 0.0, 0.0], v::ZERO, fwd));
        assert!(in_view_hemisphere([-300.0, 0.0, 0.0], [-128.0, 0.0, 0.0], fwd));
    }

    #[test]
    fn item_cull_and_fade() {
        assert_eq!(ItemCull::for_bits(0), None);
        assert_eq!(ItemCull::for_bits(8), None, "bit 8 alone has no cull");
        assert_eq!(ItemCull::for_bits(1 | 2 | 4).map(|c| c.max), Some(592.0), "bit 1 first");
        assert_eq!(ItemCull::for_bits(12).map(|c| c.max), Some(256.0), "a shell: bit 4");
        assert_eq!(ItemCull::for_bits(2).map(|c| c.max), Some(1024.0));
        let gun = ItemCull::for_bits(2).unwrap();
        assert_eq!(gun.fade(824.0, 90.0), None, "the fade starts beyond 824");
        assert_eq!(gun.fade(924.0, 90.0), Some(127), "(1024 - 924) * 1.275 truncated");
        assert_eq!(gun.fade(1000.0, 79.0), None, "zoomed in: no fade");
        assert_eq!(ItemCull::for_bits(1).unwrap().fade(492.5, 80.0), Some(253));
        assert!(beyond_distance([1024.0, 0.0, 0.0], v::ZERO, 1024.0, 90.0));
        assert!(!beyond_distance([1023.0, 0.0, 0.0], v::ZERO, 1024.0, 90.0));
        assert!(!beyond_distance([2047.0, 0.0, 0.0], v::ZERO, 1024.0, 39.0), "zoomed in: twice as far");
        assert!(beyond_distance([2048.0, 0.0, 0.0], v::ZERO, 1024.0, 39.0));
    }

    #[test]
    fn lod_bodies() {
        assert_eq!(lod_body(300.0, 90.0), 0);
        assert_eq!(lod_body(256.0, 90.0), 1);
        assert_eq!(lod_body(129.0, 90.0), 1);
        assert_eq!(lod_body(128.0, 90.0), 2);
        assert_eq!(lod_body(65.0, 90.0), 2);
        assert_eq!(lod_body(64.0, 90.0), 3);
        assert_eq!(lod_body(2000.0, 13.0), 3, "zoomed far in");
        assert_eq!(lod_body(2048.0, 13.0), 0);
        assert_eq!(lod_body(500.0, 14.0), 2, "FOV 14..40 within 512");
        assert_eq!(lod_body(500.0, 40.0), 2);
        assert_eq!(lod_body(500.0, 41.0), 0);
        assert_eq!(lod_body(100.0, 79.0), 2, "zoomed in, but neither zoom case: by distance");
    }

    #[test]
    fn tsmblur_bytes() {
        assert_eq!(tsmblur_args(&[]), None);
        assert_eq!(tsmblur_args(&[5]), Some((5, -1)), "a missing level byte reads as -1");
        assert_eq!(tsmblur_args(&[255, 0xff, 9, 9]), Some((255, -1)));
        let mut m = mblur::MotionBlur::default();
        for e in 0..=255u8 {
            for l in 0..=255u8 {
                let (ent, level) = tsmblur_args(&[e, l]).unwrap();
                m.add(ent, level, 1.0, 2.0, 0.0);
            }
        }
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
