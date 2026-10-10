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

use crate::events::ClientBullet;
use crate::{state, view};
use emitters::MapFx;
use goldsrc::Global;
use goldsrc::ext::render::{self as r, Cvar, Vec3, vec3 as v};
use goldsrc::sys::*;
use psys::{Camera, PSys};
use shared::protocol::effects as msg;
use std::cell::RefCell;

struct Fx {
    psys: PSys,
    map: MapFx,
    level: String,
    fog: Option<emitters::Fog>,
    bullets: Vec<ClientBullet>,
    visible: [bool; 33],
    /// Sequence number of the newest tracked shell (see [`track_shell`]).
    shell_seq: i32,
    whizz: u32,
    time: f64,
    frametime: f32,
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
}

/// `cl_fx` quality (`FX_ApplyCvar` `0x10057d50`, `FX_AdaptQuality` `0x10057a70`).
#[derive(Clone, Copy)]
struct Quality {
    q: f32,
    samples: [f32; 20],
    n: usize,
    next: f64,
    applied: f32,
}

impl Default for Quality {
    fn default() -> Self {
        Quality { q: 1.0, samples: [0.0; 20], n: 0, next: 0.0, applied: -1.0 }
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
        bullets: Vec::new(),
        visible: [false; 33],
        shell_seq: 0,
        whizz: 0,
        time: 0.0,
        frametime: 0.0,
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

/// Effects clock (client time, seconds).
pub fn now() -> f64 {
    with(|fx| fx.time)
}

pub fn player_visible(idx: i32) -> bool {
    with(|fx| fx.visible.get(idx as usize).copied().unwrap_or(false))
}

pub fn paintball() -> bool {
    cvar(|c| c.paintball) != 0.0
}

/// Shell life `max(2, 30 q)`.
pub fn shell_life() -> f32 {
    with(|fx| (30.0 * fx.quality.q).max(2.0))
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
        fx.whizz = fx.whizz.wrapping_add(1);
        503 + (fx.whizz & 1) as i32
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

/// Impact particles at a bullet hit (`TS_ImpactParticles`).
pub fn impact(pos: Vec3, normal: Vec3, tex: u8, pellets: bool) {
    let cam = view::origin();
    with(|fx| {
        let t = fx.time;
        fx::impact(&mut fx.psys, t, cam, pos, normal, tex, pellets)
    });
}

pub fn init() {
    *CVARS.0.borrow_mut() = Some(Cvars {
        fx: Cvar::register(c"cl_fx", c"1", r::FCVAR_ARCHIVE),
        gore: Cvar::register(c"cl_gore", c"2", r::FCVAR_ARCHIVE),
        laser: Cvar::register(c"cl_laser", c"0", r::FCVAR_ARCHIVE),
        lasercolor: Cvar::register(c"cl_lasercolor", c"0", r::FCVAR_ARCHIVE),
        shadow: Cvar::register(c"cl_shadow", c"2", r::FCVAR_ARCHIVE),
        motionblur: Cvar::register(c"cl_motionblur", c"1", r::FCVAR_ARCHIVE),
        paintball: Cvar::register(c"r_paintball", c"0", 0),
        gl_fog: Cvar::register(c"gl_fog", c"0", r::FCVAR_ARCHIVE),
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
    let Some(p) = r::local_player() else { return };
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
    let ts = state::with(|s| s.time_scale);
    with(|fx| {
        let t = fx.time;
        match f {
            msg::TsFrags::Glass { origin, size, dir, force, .. } => fx::glass(&mut fx.psys, t, origin, size, dir, force),
            msg::TsFrags::Explosion { origin, radius } => fx::explosion(&mut fx.psys, t, origin, radius, ts),
            msg::TsFrags::Sparks { origin, dir } => {
                fx::scintille(&mut fx.psys, t, origin, 6.0);
                fx::sparks(&mut fx.psys, t, origin, dir, 12, 1.0, 300.0);
            }
        }
    });
}

/// `TSMBlur` (`0x1004bf90` -> `MotionBlur_Add` `0x10029680`): start or lengthen a ghost trail.
pub fn on_tsmblur(b: &[u8]) {
    if let Some(m) = msg::TsMBlur::decode(b) {
        let (blur, fxq) = (cvar(|c| c.motionblur), cvar(|c| c.fx));
        let now = r::client_time() as f64;
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
/// (`CTS_CL_Emitter::Frame` `0x1003ab40`), quality adaptation.
pub fn frame(time: f64, frametime: f32) {
    with(|fx| {
        fx.time = time;
        fx.frametime = frametime;
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
        });
    }
    let fxq = cvar(|c| c.fx);
    let ts = state::with(|s| s.time_scale);
    let cam = view::camera();
    with(|fx| {
        apply_quality(fx, fxq, frametime);
        let t = fx.time;
        fx.map.emit(&mut fx.psys, &cam, t, ts);
    });
}

fn apply_quality(fx: &mut Fx, cl_fx: f32, frametime: f32) {
    let q = &mut fx.quality;
    if cl_fx != q.applied {
        q.applied = cl_fx;
        fx.psys.max = if cl_fx <= 0.0 { 2048 } else { 4096 };
        match cl_fx as i32 {
            0 => {
                q.q = 0.0;
                fx.psys.divisor = 8.0;
                fx.psys.physics = false;
            }
            2.. => {
                q.q = 1.0;
                fx.psys.divisor = 1.0;
                fx.psys.physics = true;
            }
            _ => {}
        }
    }
    if cl_fx as i32 != 1 {
        return;
    }
    q.samples[q.n % 20] = frametime;
    q.n += 1;
    if fx.time >= q.next {
        q.next = fx.time + 0.1;
        let cnt = q.n.min(20).max(1);
        let avg10 = q.samples[..cnt].iter().sum::<f32>() / cnt as f32 * 100.0;
        q.q = 1.0 - (avg10.clamp(1.11, 3.33) - 1.11) / 2.22;
        fx.psys.divisor = if q.q > 0.75 {
            1.0
        } else if q.q > 0.5 {
            2.0
        } else if q.q > 0.25 {
            4.0
        } else {
            6.0
        };
        fx.psys.physics = q.q > 0.374;
    }
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
        let visible = hemisphere && !spectator && !in_eye_target && occlusion_visible(e, &cam);
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
    if d >= max || !psys::in_view_cone(&cam, e.origin) {
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

    let (blur, ts, now) = (cvar(|c| c.motionblur), state::with(|s| s.time_scale), r::client_time() as f64);
    for ent in with(|fx| fx.blur.due(blur, ts, now)) {
        if mblur::spawn_ghost(ent) {
            with(|fx| fx.blur.spawned(ent));
        }
    }
}

pub fn draw_normal() {}

/// `HUD_DrawTransparentTriangles` -> `TS_DrawEffects` (`0x10056dd0`).
pub fn draw_transparent() {
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
    with(|fx| fx.sprites = Some(sprites));

    // Players for particle reaction.
    let mut players = Vec::new();
    for i in 1..=r::max_clients().min(32) {
        if visible[i as usize] {
            if let Some(e) = r::entity(i) {
                players.push((e.origin, e.curstate.velocity));
            }
        }
    }
    let (dt, requests) = with(|fx| {
        let dt = fx.frametime * ts;
        fx.draws += 1;
        let t = fx.time;
        fx.psys.update(t, dt, &cam, &players);
        (dt, std::mem::take(&mut fx.psys.spawn_requests))
    });
    with(|fx| {
        let t = fx.time;
        for (tmpl, pos) in requests {
            fx.map.spawn_one(&mut fx.psys, tmpl, pos, t);
        }
        fx.psys.draw(t, dt, &cam);
        fx.visible = [false; 33];
    });
    fog_end();
}

fn cam_hardware() -> bool {
    view::hardware()
}

/// Fog from the map's `ts_fog` (`V_SetFog` `0x10056c20`), through TriAPI instead of raw OpenGL.
/// Only with `gl_fog` set, as in TS (default 0).
pub fn fog_begin() {
    let fog = with(|fx| fx.fog);
    let on = cvar(|c| c.gl_fog) != 0.0;
    if let (Some(f), true) = (fog, on) {
        // ts_fog colours are 0..1 (TS feeds glFogfv); TriAPI Fog takes 0..255.
        let mut c = f.color.map(|x| if x <= 1.0 { x * 255.0 } else { x });
        goldsrc::tri!(Fog(c.as_mut_ptr(), f.near, f.far, 1));
    }
}

fn fog_end() {
    if with(|fx| fx.fog.is_some()) && cvar(|c| c.gl_fog) != 0.0 {
        let mut c = [0.0f32; 3];
        goldsrc::tri!(Fog(c.as_mut_ptr(), 0.0, 0.0, 0));
    }
}

/// `HUD_StudioEvent` (`0x10044b50`): only 5002 (sparks) and 5004 (sound) remain.
pub fn studio_event(ev: &mstudioevent_s, ent: &cl_entity_t) {
    let opts = goldsrc::util::c_array_str(&ev.options);
    match ev.event {
        5002 => r::spark_effect(ent.attachment[0], opts.trim().parse().unwrap_or(0), -100, 100),
        5004 => r::play_sound(ent.index, ent.attachment[0], CHAN_BODY as i32, &opts, 1.0, 0.8, 0, crate::events::slow_pitch()),
        _ => {}
    }
}
