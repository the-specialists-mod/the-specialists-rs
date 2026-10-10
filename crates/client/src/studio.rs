//! The TS studio model renderer: `CStudioModelRenderer` (ctor `0x100300b0`, vtable `0x1007d5f8`),
//! reached through `HUD_GetStudioModelInterface` (`0x1001f3f0`). Reference: decomp
//! `docs/client/rendering-effects-prediction.md` §2 (**R§2**) and the SDK `StudioModelRenderer.cpp`
//! it was built from.
//!
//! What TS changed from the SDK, all ported here:
//!
//! * `StudioSetupBones` (`0x10031080`): the gait (leg) sequence replaces every bone before the first
//!   one named `Bip01 Dorso` or `Bip01 Spine`, and that bone is turned about its own axis by the
//!   yaw between the upper-body and gait roots, so the torso keeps aiming while the legs run.
//! * `StudioEstimateGait` (`0x10031fa0`): 2-D speed; gait movement x0.75 below 180 units/s.
//! * `StudioProcessGait` (`0x10032250`): the fps-driven gait advance is scaled by `curstate.framerate`.
//! * renderfx 17 (`kRenderFxDeadPlayer`) is redefined (`StudioDrawModel` `0x10031ae0`):
//!   `rendercolor.r` is the player, `rendercolor.b == 1` marks a motion-blur ghost
//!   (`StudioDrawGhostPlayer` `0x100337f0`), anything else a corpse (`StudioDrawDeadPlayer`
//!   `0x10033540`).
//!
//! Engine access goes through `goldsrc::ext::studio`; the model data is parsed from bytes
//! (`studio::model`).

pub mod math;
pub mod model;

use crate::{state, view};
use goldsrc::Global;
use goldsrc::ext::render::{self as r, Cvar};
use goldsrc::ext::studio::{self as es, Ent, Header, Lighting, Mat34, Model, PlayerInfo};
use goldsrc::sys::*;
use math::*;
use model::{Anim, Bytes, SeqDesc, Studio};
use std::cell::RefCell;

const STUDIO_RENDER: i32 = 1;
const STUDIO_EVENTS: i32 = 2;
const MAX_BONES: usize = es::MAX_BONES;

/// Bone positions and rotations of one evaluated sequence (the SDK's static `pos[]`/`q[]`).
#[derive(Clone)]
struct Pose {
    pos: [Vec3; MAX_BONES],
    q: [Quat; MAX_BONES],
}

impl Default for Pose {
    fn default() -> Self {
        Pose { pos: [[0.0; 3]; MAX_BONES], q: [[0.0, 0.0, 0.0, 1.0]; MAX_BONES] }
    }
}

/// Where the current `m_pPlayerInfo` lives: the engine's (copied in, written back on release) or
/// a private copy (corpses and ghosts).
#[derive(Clone, Copy, PartialEq)]
enum InfoSrc {
    Engine(i32),
    Local,
}

/// A bone kept by `StudioSaveBones` for `StudioMergeBones` (weapon models, followers).
#[derive(Clone)]
struct CachedBone {
    /// `m_nCachedBoneNames[i]` (`char[32]`), without the NUL.
    name: [u8; 32],
    len: usize,
    bone: Mat34,
    light: Mat34,
}

/// `CStudioModelRenderer` (layout R§2.2). One object, `g_StudioRenderer` in TS.
struct Renderer {
    do_interp: bool,
    gait_estimation: bool,
    frame_count: i32,
    time: f64,
    old_time: f64,
    view: es::ViewInfo,
    soft_scale: (f32, f32),
    ent: Option<Ent>,
    render_model: Option<Model>,
    hdr: Option<Header>,
    /// `m_pPlayerInfo` (None = NULL) and where it came from.
    pinfo: Option<PlayerInfo>,
    pinfo_src: InfoSrc,
    /// The private `player_info_t` copy a corpse or ghost draw works on.
    local_info: PlayerInfo,
    player_index: i32,
    gait_movement: f32,
    cached: Vec<CachedBone>,
    draw_entities: Cvar,
    motionblur: Cvar,
}

impl Default for Renderer {
    fn default() -> Self {
        Renderer {
            // ctor `0x100300b0`
            do_interp: true,
            gait_estimation: true,
            frame_count: 0,
            time: 0.0,
            old_time: 0.0,
            view: es::ViewInfo::default(),
            soft_scale: (1.0, 1.0),
            ent: None,
            render_model: None,
            hdr: None,
            pinfo: None,
            pinfo_src: InfoSrc::Local,
            local_info: PlayerInfo::default(),
            player_index: 0,
            gait_movement: 0.0,
            cached: Vec::new(),
            draw_entities: Cvar::NULL,
            motionblur: Cvar::NULL,
        }
    }
}

static RENDERER: Global<RefCell<Option<Box<Renderer>>>> = Global(RefCell::new(None));

/// Runs `f` on the renderer. The renderer is taken out while it runs, so an (unexpected) nested
/// draw from an engine callback gets a fresh one instead of a double borrow.
fn with<R>(f: impl FnOnce(&mut Renderer) -> R) -> R {
    let mut r = RENDERER.0.borrow_mut().take().unwrap_or_default();
    let out = f(&mut r);
    *RENDERER.0.borrow_mut() = Some(r);
    out
}

/// `CStudioModelRenderer::Init` (`0x10030030`), after `goldsrc::ext::studio::init` cached the
/// engine's matrices. `cl_himodels` and `developer` are looked up by TS but never used.
pub fn init() {
    with(|r| {
        r.draw_entities = Cvar::find(c"r_drawentities");
        r.motionblur = Cvar::find(c"cl_motionblur");
    });
}

/// `R_StudioDrawModel` (`0x1001f3d0`) -> `StudioDrawModel`.
pub fn draw_model(flags: i32) -> bool {
    with(|r| r.draw_model(flags))
}

/// `R_StudioDrawPlayer` (`0x1001f3b0`) -> `StudioDrawPlayer`.
pub fn draw_player(flags: i32, player: &mut entity_state_s) -> bool {
    with(|r| r.draw_player(flags, player))
}

fn time_scale() -> f32 {
    state::with(|s| s.time_scale)
}

/// Torso controller value for a yaw offset: `((yaw / 4) + 30) / (60 / 255)`, truncated to a byte.
fn torso_controller(yaw: f32) -> u8 {
    (((yaw / 4.0 + 30.0) as f64 * (255.0 / 60.0)) as i32) as u8
}

/// `x - (int)(x / 360) * 360`.
fn wrap360(x: f32) -> f32 {
    x - ((x / 360.0) as i32 * 360) as f32
}

impl Renderer {
    fn ent(&self) -> &'static mut cl_entity_s {
        self.ent.expect("no current entity").get()
    }

    fn studio(&self) -> Studio<'static> {
        Studio::new(self.hdr.map_or(&[][..], |h| h.data()))
    }

    /// Start of every draw: current entity, times and view (the first lines of every `StudioDraw*`).
    fn begin(&mut self) -> Option<Ent> {
        self.ent = es::current_entity();
        let (n, t, o) = es::times();
        self.frame_count = n;
        self.time = t;
        self.old_time = o;
        self.view = es::view_info();
        self.soft_scale = es::alias_scale();
        self.ent
    }

    /// Sets the model to draw (`m_pStudioHeader = Mod_Extradata(..)`, `StudioSetHeader`).
    fn set_header(&mut self, m: Model) -> bool {
        self.hdr = es::extradata(m);
        match self.hdr {
            Some(h) => {
                es::set_header(h);
                true
            }
            None => false,
        }
    }

    // ---- player_info ------------------------------------------------------------------------

    fn acquire_engine_info(&mut self, player: i32) {
        self.pinfo = es::player_info(player);
        self.pinfo_src = InfoSrc::Engine(player);
    }

    fn acquire_local_info(&mut self) {
        self.pinfo = Some(self.local_info);
        self.pinfo_src = InfoSrc::Local;
    }

    /// `m_pPlayerInfo = NULL`: writes the copy back where it came from.
    fn release_info(&mut self) {
        if let Some(p) = self.pinfo.take() {
            match self.pinfo_src {
                InfoSrc::Engine(i) => es::set_player_info(i, &p),
                InfoSrc::Local => self.local_info = p,
            }
        }
    }

    // ---- StudioDrawModel ----------------------------------------------------------------------

    /// `StudioDrawModel` (`0x10031ae0`).
    fn draw_model(&mut self, flags: i32) -> bool {
        let Some(ent) = self.begin() else { return false };
        self.pinfo = None;
        // TS also rescales the view model's framerate here (slow motion); `view` does that once per
        // frame instead (`view::view_model_framerate`).
        let e = ent.get();
        if e.curstate.renderfx == kRenderFxDeadPlayer as i32 {
            return if e.curstate.rendercolor.b != 1 { self.draw_corpse(flags) } else { self.draw_ghost(flags) };
        }

        let Some(model) = ent.model() else { return false };
        self.render_model = Some(model);
        if !self.set_header(model) {
            return false;
        }
        es::set_render_model(model);
        self.set_up_transform(false);

        if flags & STUDIO_RENDER != 0 {
            if !es::check_bbox() {
                return false;
            }
            es::count_model();
            if self.studio().numbodyparts() == 0 {
                return true;
            }
        }

        if self.ent().curstate.movetype == MOVETYPE_FOLLOW as i32 {
            self.merge_bones(model);
        } else {
            self.setup_bones();
        }
        self.save_bones();

        if flags & STUDIO_EVENTS != 0 {
            self.events();
        }

        if flags & STUDIO_RENDER != 0 {
            let mut lighting = Lighting::default();
            es::dynamic_light(ent, &mut lighting);
            es::entity_light(&mut lighting);
            es::setup_lighting(&mut lighting);
            let colormap = self.ent().curstate.colormap;
            es::set_remap_colors(colormap & 0xff, (colormap >> 8) & 0xff);
            self.render();
        }
        true
    }

    /// renderfx 17, `rendercolor.b != 1`: a corpse draws as its player, from a copy of the player's
    /// state and `player_info` so the live player's gait isn't disturbed.
    fn draw_corpse(&mut self, flags: i32) -> bool {
        let e = self.ent();
        let who = e.curstate.rendercolor.r as i32;
        if who == 0 || who > r::max_clients() {
            return false;
        }
        let Some(mut dead) = es::player_state(who - 1) else { return false };
        dead.number = who;
        dead.weaponmodel = 0;
        dead.gaitsequence = 0;
        dead.movetype = MOVETYPE_NONE as i32;
        dead.angles = e.curstate.angles;
        dead.origin = e.curstate.origin;
        let Some(info) = es::player_info(who - 1) else { return false };
        self.local_info = info;
        let interp = std::mem::replace(&mut self.do_interp, false);
        let res = self.draw_dead_player(flags, &mut dead);
        self.do_interp = interp;
        res
    }

    /// renderfx 17, `rendercolor.b == 1`: a motion-blur ghost (`effects::mblur`). The gait state
    /// the ghost froze is in `iuser1` (gaitsequence), `iuser2` (renderframe), `fuser3` (gaitframe)
    /// and `fuser4` (gaityaw).
    fn draw_ghost(&mut self, flags: i32) -> bool {
        let e = self.ent();
        let owner = e.curstate.rendercolor.r as i32;
        if r::local_player().is_some_and(|p| p.index == owner) && (!view::is_third_person() || (self.motionblur.value() as i32) < 2) {
            return false;
        }
        if time_scale() > 0.99 {
            return false;
        }
        // (TS asks the engine for any number; not every engine range-checks PlayerInfo.)
        if !(1..=r::max_clients()).contains(&e.curstate.number) {
            return false;
        }
        let Some(mut info) = es::player_info(e.curstate.number - 1) else { return false };
        info.gaitsequence = e.curstate.iuser1;
        info.renderframe = e.curstate.iuser2;
        info.gaitframe = e.curstate.fuser3;
        info.gaityaw = e.curstate.fuser4;
        self.local_info = info;
        // TS passes the ghost's own curstate as the player state; its renderamt = 255 lands on the
        // entity (harmless: the engine took the blend already and the ghost's callback resets it).
        e.curstate.renderamt = 255;
        let st = e.curstate;
        let interp = std::mem::replace(&mut self.do_interp, false);
        let res = self.draw_ghost_player(flags, &st);
        self.do_interp = interp;
        res
    }

    /// The common start of the three player draws: player index, spectator hide, player model.
    fn player_model(&mut self, number: i32) -> Option<Model> {
        self.player_index = number - 1;
        if self.player_index < 0 || self.player_index >= r::max_clients() {
            return None;
        }
        // Hidden while spectating this player in eye mode. TS tests g_iUser1 == 5 here (the SDK's
        // in-eye is 4); `effects::add_entity` already drops the in-eye target for mode 4.
        if state::with(|s| s.spec_mode == 5 && s.spec_target == number) {
            return None;
        }
        let model = es::setup_player_model(self.player_index)?;
        self.render_model = Some(model);
        if !self.set_header(model) {
            return None;
        }
        es::set_render_model(model);
        Some(model)
    }

    fn reset_torso(&self) {
        let e = self.ent();
        e.curstate.controller[..4].fill(127);
        e.latched.prevcontroller = e.curstate.controller;
    }

    /// `if (flags & STUDIO_RENDER)`: frustum test and counters. Some(result) ends the draw.
    fn check_render(&self, flags: i32) -> Option<bool> {
        if flags & STUDIO_RENDER == 0 {
            return None;
        }
        if !es::check_bbox() {
            return Some(false);
        }
        es::count_model();
        (self.studio().numbodyparts() == 0).then_some(true)
    }

    /// `StudioCalcAttachments`, `StudioClientEvents`, then the attachments copied to the entity list.
    fn events(&mut self) {
        self.calc_attachments();
        es::client_events();
        // Copied out first: the list entry is often the current entity itself.
        let (index, attachment) = {
            let e = self.ent();
            (e.index, e.attachment)
        };
        if index > 0 {
            if let Some(g) = r::entity(index) {
                g.attachment = attachment;
            }
        }
    }

    /// Lighting, the player's colours (clamped to 0..360) and the body.
    fn light_and_render(&mut self, ent: Ent, lighting: &mut Lighting) {
        es::dynamic_light(ent, lighting);
        es::entity_light(lighting);
        es::setup_lighting(lighting);
        let info = self.pinfo.unwrap_or_default();
        es::set_remap_colors(info.topcolor.clamp(0, 360), info.bottomcolor.clamp(0, 360));
        self.render();
    }

    /// The `p_` weapon model, merged onto the player's cached bones.
    fn draw_weapon(&mut self, weaponmodel: i32, lighting: &mut Lighting) {
        let save = *self.ent();
        if let Some(wm) = es::model_by_index(weaponmodel) {
            if self.set_header(wm) {
                self.merge_bones(wm);
                es::setup_lighting(lighting);
                self.render();
                self.calc_attachments();
            }
        }
        *self.ent() = save;
    }

    /// `StudioDrawPlayer` (`0x10032540`): the SDK flow; forces `renderamt = 255` on the state.
    fn draw_player(&mut self, flags: i32, player: &mut entity_state_s) -> bool {
        player.renderamt = 255;
        let Some(ent) = self.begin() else { return false };
        if self.player_model(player.number).is_none() {
            return false;
        }
        let idx = self.player_index;
        if player.gaitsequence != 0 {
            self.acquire_engine_info(idx);
            let orig = self.ent().angles;
            self.process_gait(player);
            if let Some(p) = self.pinfo.as_mut() {
                p.gaitsequence = player.gaitsequence;
            }
            self.release_info();
            self.set_up_transform(false);
            self.ent().angles = orig;
        } else {
            self.reset_torso();
            self.acquire_engine_info(idx);
            if let Some(p) = self.pinfo.as_mut() {
                p.gaitsequence = 0;
            }
            self.release_info();
            self.set_up_transform(false);
        }

        if let Some(res) = self.check_render(flags) {
            return res;
        }

        self.acquire_engine_info(idx);
        self.setup_bones();
        self.save_bones();
        if let Some(p) = self.pinfo.as_mut() {
            p.renderframe = self.frame_count;
        }
        self.release_info();

        if flags & STUDIO_EVENTS != 0 {
            self.events();
        }

        if flags & STUDIO_RENDER != 0 {
            let mut lighting = Lighting::default();
            self.acquire_engine_info(idx);
            self.light_and_render(ent, &mut lighting);
            self.release_info();
            if player.weaponmodel != 0 {
                self.draw_weapon(player.weaponmodel, &mut lighting);
            }
        }
        true
    }

    /// TS `StudioDrawDeadPlayer` (`0x10033540`): no gait, no weapon, private `player_info`.
    fn draw_dead_player(&mut self, flags: i32, player: &mut entity_state_s) -> bool {
        player.renderamt = 255;
        let Some(ent) = self.begin() else { return false };
        if self.player_model(player.number).is_none() {
            return false;
        }
        self.reset_torso();
        self.local_info.gaitsequence = 0;
        self.set_up_transform(false);

        if let Some(res) = self.check_render(flags) {
            return res;
        }

        self.acquire_local_info();
        self.setup_bones();
        self.save_bones();
        if let Some(p) = self.pinfo.as_mut() {
            p.renderframe = self.frame_count;
        }
        self.release_info();

        if flags & STUDIO_EVENTS != 0 {
            self.events();
        }

        if flags & STUDIO_RENDER != 0 {
            let mut lighting = Lighting::default();
            self.acquire_local_info();
            self.light_and_render(ent, &mut lighting);
            self.release_info();
        }
        true
    }

    /// TS `StudioDrawGhostPlayer` (`0x100337f0`): the frozen pose. The stored gait yaw turns the
    /// torso controllers as `StudioProcessGait` would, without estimating or advancing the gait;
    /// no animation events; the weapon model is drawn.
    fn draw_ghost_player(&mut self, flags: i32, player: &entity_state_s) -> bool {
        let Some(ent) = self.begin() else { return false };
        if self.player_model(player.number).is_none() {
            return false;
        }
        self.acquire_local_info();
        if player.gaitsequence == 0 {
            self.reset_torso();
        } else {
            self.turn_torso();
        }
        self.release_info();
        self.set_up_transform(false);

        if let Some(res) = self.check_render(flags) {
            return res;
        }

        self.acquire_local_info();
        self.setup_bones();
        self.save_bones();
        self.release_info();

        if flags & STUDIO_RENDER != 0 {
            let mut lighting = Lighting::default();
            self.acquire_local_info();
            self.light_and_render(ent, &mut lighting);
            self.release_info();
            if player.weaponmodel != 0 {
                self.draw_weapon(player.weaponmodel, &mut lighting);
            }
        }
        true
    }

    // ---- gait -----------------------------------------------------------------------------------

    fn dt(&self) -> f32 {
        ((self.time - self.old_time) as f32).clamp(0.0, 1.0)
    }

    /// `StudioEstimateGait` (`0x10031fa0`), TS version: 2-D movement, and the gait movement is
    /// scaled by 0.75 below 180 units/s.
    fn estimate_gait(&mut self, player: &entity_state_s) {
        let dt = (self.time - self.old_time) as f32;
        let e = self.ent();
        let frame_count = self.frame_count;
        let Some(info) = self.pinfo.as_mut() else { return };
        if dt <= 0.0 || info.renderframe == frame_count {
            self.gait_movement = 0.0;
            return;
        }
        let dt = dt.min(1.0);
        let mut est: [f32; 2];
        // The value TS compares with 180: the estimated speed (units/s), or with the velocity
        // path the movement itself.
        let speed;
        if self.gait_estimation {
            est = [e.origin[0] - info.prevgaitorigin[0], e.origin[1] - info.prevgaitorigin[1]];
            info.prevgaitorigin = e.origin;
            self.gait_movement = (est[0] * est[0] + est[1] * est[1]).sqrt();
            speed = self.gait_movement / dt;
            if speed < 5.0 {
                self.gait_movement = 0.0;
                est = [0.0; 2];
            }
        } else {
            est = [player.velocity[0], player.velocity[1]];
            self.gait_movement = (est[0] * est[0] + est[1] * est[1]).sqrt() * dt;
            speed = self.gait_movement;
        }

        if est[0] == 0.0 && est[1] == 0.0 {
            let mut d = wrap360(e.angles[1] - info.gaityaw);
            if d > 180.0 {
                d -= 360.0;
            }
            if d < -180.0 {
                d += 360.0;
            }
            d *= if dt < 0.25 { dt * 4.0 } else { dt };
            info.gaityaw = wrap360(info.gaityaw + d);
            self.gait_movement = 0.0;
        } else {
            info.gaityaw = (est[1].atan2(est[0]).to_degrees()).clamp(-180.0, 180.0);
        }
        if speed < 180.0 {
            self.gait_movement *= 0.75;
        }
    }

    /// The side-to-side turn of `StudioProcessGait`: torso controllers from the view yaw relative
    /// to the gait yaw, then the body faces the gait yaw. Also the ghost's torso (TS
    /// `StudioDrawGhostPlayer` repeats this block).
    fn turn_torso(&mut self) {
        let e = self.ent();
        let Some(info) = self.pinfo.as_mut() else { return };
        let mut yaw = wrap360(e.angles[1] - info.gaityaw);
        if yaw < -180.0 {
            yaw += 360.0;
        }
        if yaw > 180.0 {
            yaw -= 360.0;
        }
        if yaw > 120.0 {
            info.gaityaw -= 180.0;
            self.gait_movement = -self.gait_movement;
            yaw -= 180.0;
        } else if yaw < -120.0 {
            info.gaityaw += 180.0;
            self.gait_movement = -self.gait_movement;
            yaw += 180.0;
        }
        e.curstate.controller[..4].fill(torso_controller(yaw));
        e.latched.prevcontroller = e.curstate.controller;
        e.angles[1] = info.gaityaw;
        if e.angles[1] < 0.0 {
            e.angles[1] += 360.0;
        }
        e.latched.prevangles[1] = e.angles[1];
    }

    /// `StudioProcessGait` (`0x10032250`), TS version: the fps-driven gait advance is multiplied
    /// by `curstate.framerate` (the server slows it for slowed players).
    fn process_gait(&mut self, player: &mut entity_state_s) {
        let m = self.studio();
        let e = self.ent();
        if e.curstate.sequence >= m.numseq() {
            e.curstate.sequence = 0;
        }
        if let Some(seq) = m.seq(e.curstate.sequence) {
            let blend = player_blend(&seq, &mut e.angles[0]);
            e.latched.prevangles[0] = e.angles[0];
            e.curstate.blending[0] = blend;
            e.latched.prevblending[0] = blend;
            e.latched.prevseqblending[0] = blend;
        }
        let dt = self.dt();
        self.estimate_gait(player);
        self.turn_torso();

        // TS drops the SDK's range check here; an out-of-range gait sequence is treated as 0.
        if player.gaitsequence >= m.numseq() {
            player.gaitsequence = 0;
        }
        let Some(seq) = m.seq(player.gaitsequence) else { return };
        let framerate = self.ent().curstate.framerate;
        let Some(info) = self.pinfo.as_mut() else { return };
        let numframes = seq.numframes as f32;
        if seq.linearmovement[0] > 0.0 {
            info.gaitframe += self.gait_movement / seq.linearmovement[0] * numframes;
        } else {
            info.gaitframe += framerate * seq.fps * dt;
        }
        if seq.numframes > 0 {
            info.gaitframe -= ((info.gaitframe / numframes) as i32 * seq.numframes) as f32;
            if info.gaitframe < 0.0 {
                info.gaitframe += numframes;
            }
        }
    }

    // ---- transforms and bones -------------------------------------------------------------------

    /// `StudioSetUpTransform` (`0x10030970`).
    fn set_up_transform(&mut self, trivial_accept: bool) {
        let e = self.ent();
        let mut modelpos = e.origin;
        let mut angles = e.curstate.angles;
        if e.curstate.movetype == MOVETYPE_STEP as i32 {
            let mut f = 0.0f32;
            let (animtime, prev) = (e.curstate.animtime, e.latched.prevanimtime);
            if self.time < (animtime + 1.0) as f64 && animtime != prev {
                f = ((self.time - animtime as f64) / (animtime - prev) as f64) as f32;
            }
            f = if self.do_interp { f - 1.0 } else { 0.0 };
            for i in 0..3 {
                modelpos[i] += (e.origin[i] - e.latched.prevorigin[i]) * f;
            }
            for i in 0..3 {
                let mut d = e.angles[i] - e.latched.prevangles[i];
                if d > 180.0 {
                    d -= 360.0;
                } else if d < -180.0 {
                    d += 360.0;
                }
                angles[i] += d * f;
            }
        } else if e.curstate.movetype != MOVETYPE_NONE as i32 {
            angles = e.angles;
        }
        angles[0] = -angles[0];
        let mut rot = angle_matrix(angles);

        if !es::is_hardware() {
            let v = &self.view;
            let up = v.up.map(|x| -x);
            let viewmatrix =
                [[v.right[0], v.right[1], v.right[2], 0.0], [up[0], up[1], up[2], 0.0], [v.forward[0], v.forward[1], v.forward[2], 0.0]];
            for i in 0..3 {
                rot[i][3] = modelpos[i] - v.origin[i];
            }
            let mut alias = concat(&viewmatrix, &rot);
            if trivial_accept {
                // ZISCALE * 0x10000 = 2^31
                let k = 1.0 / 2147483648.0f32;
                for i in 0..4 {
                    alias[0][i] *= self.soft_scale.0 * k;
                    alias[1][i] *= self.soft_scale.1 * k;
                    alias[2][i] *= k;
                }
            }
            es::set_alias(&alias);
        }
        for i in 0..3 {
            rot[i][3] = modelpos[i];
        }
        es::set_rotation(&rot);
    }

    /// `StudioEstimateInterpolant` (`0x10030c40`).
    fn estimate_interpolant(&self) -> f32 {
        let e = self.ent();
        let mut dadt = 1.0;
        if self.do_interp && e.curstate.animtime >= e.latched.prevanimtime + 0.01 {
            dadt = (((self.time - e.curstate.animtime as f64) / 0.1) as f32).min(2.0);
        }
        dadt
    }

    /// `StudioEstimateFrame` (`0x10030f80`).
    fn estimate_frame(&self, seq: &SeqDesc) -> f64 {
        let e = self.ent();
        let dfdt = if self.do_interp && self.time >= e.curstate.animtime as f64 {
            (self.time - e.curstate.animtime as f64) * e.curstate.framerate as f64 * seq.fps as f64
        } else {
            0.0
        };
        let n = seq.numframes as f64;
        let mut f = if seq.numframes <= 1 { 0.0 } else { e.curstate.frame as f64 * (n - 1.0) / 256.0 };
        f += dfdt;
        if seq.flags & model::STUDIO_LOOPING != 0 {
            if seq.numframes > 1 {
                f -= (f / (n - 1.0)).trunc() * (n - 1.0);
            }
            if f < 0.0 {
                f += n - 1.0;
            }
        } else {
            if f >= n - 1.001 {
                f = n - 1.001;
            }
            if f < 0.0 {
                f = 0.0;
            }
        }
        f
    }

    /// `StudioGetAnim` (`0x100307e0`). A missing sequence group gives an empty block, which reads
    /// as the bind pose.
    fn get_anim(&self, model: Model, seq: &SeqDesc) -> Anim<'static> {
        let m = self.studio();
        let empty = Anim { data: Bytes(&[]), base: 0 };
        let Some((file, data)) = m.seqgroup(seq.seqgroup) else { return empty };
        if seq.seqgroup == 0 {
            return Anim { data: m.b, base: (data.max(0) + seq.animindex.max(0)) as usize };
        }
        let Ok(name) = std::ffi::CString::new(file) else { return empty };
        match es::seqgroup_data(model, seq.seqgroup as usize, &name) {
            Some(d) => Anim { data: Bytes(d), base: seq.animindex.max(0) as usize },
            None => empty,
        }
    }

    /// `StudioCalcRotations` (`0x10030ca0`): the entity's controllers, then [`sample_pose`].
    fn calc_rotations(&self, pose: &mut Pose, seq: &SeqDesc, anim: Anim, f: f32) {
        let m = self.studio();
        let dadt = self.estimate_interpolant();
        let (c1, c2, mouth) = {
            let e = self.ent();
            (e.curstate.controller, e.latched.prevcontroller, e.mouth.mouthopen)
        };
        let adj = bone_adj(&m, dadt, &c1, &c2, mouth);
        sample_pose(&m, pose, seq, anim, f, &adj);
    }

    /// `StudioSlerpBones` (`0x100306a0`): `a = a * (1 - s) + b * s`.
    fn slerp_bones(&self, a: &mut Pose, b: &Pose, s: f32) {
        let s = s.clamp(0.0, 1.0);
        let s1 = 1.0 - s;
        for i in 0..self.studio().numbones() {
            a.q[i] = quaternion_slerp(a.q[i], b.q[i], s);
            for j in 0..3 {
                a.pos[i][j] = a.pos[i][j] * s1 + b.pos[i][j] * s;
            }
        }
    }

    /// Evaluates a sequence with its 2 or 4 blends (`numblends > 1`), as `StudioSetupBones` does.
    fn calc_blended(&self, pose: &mut Pose, model: Model, seq: &SeqDesc, f: f32, blend: [f32; 2]) {
        let nb = self.studio().numbones();
        let anim = self.get_anim(model, seq);
        self.calc_rotations(pose, seq, anim, f);
        if seq.numblends > 1 {
            let mut p2 = Pose::default();
            self.calc_rotations(&mut p2, seq, anim.blend(1, nb), f);
            self.slerp_bones(pose, &p2, blend[0]);
            if seq.numblends == 4 {
                let (mut p3, mut p4) = (Pose::default(), Pose::default());
                self.calc_rotations(&mut p3, seq, anim.blend(2, nb), f);
                self.calc_rotations(&mut p4, seq, anim.blend(3, nb), f);
                self.slerp_bones(&mut p3, &p4, blend[0]);
                self.slerp_bones(pose, &p3, blend[1]);
            }
        }
    }

    /// `if (curstate.sequence >= numseq) curstate.sequence = 0`, returning the sequence.
    fn clamp_sequence(&self, m: &Studio) -> i32 {
        let e = self.ent();
        if e.curstate.sequence >= m.numseq() || e.curstate.sequence < 0 {
            e.curstate.sequence = 0;
        }
        e.curstate.sequence
    }

    /// `StudioSetupBones` (`0x10031080`), TS version (see the module docs).
    fn setup_bones(&mut self) {
        let m = self.studio();
        let Some(model) = self.render_model else { return };
        let Some(seq) = m.seq(self.clamp_sequence(&m)) else { return };
        let f = self.estimate_frame(&seq);
        let dadt = self.estimate_interpolant();
        let (blending, l) = {
            let e = self.ent();
            (e.curstate.blending, e.latched)
        };
        let blend = |cur: u8, prev: u8| (cur as f32 * dadt + prev as f32 * (1.0 - dadt)) / 255.0;
        let s0 = blend(blending[0], l.prevblending[0]);
        let s1 = blend(blending[1], l.prevblending[1]);
        let mut pose = Pose::default();
        self.calc_blended(&mut pose, model, &seq, f as f32, [s0, s1]);

        if self.do_interp && l.sequencetime != 0.0 && (l.sequencetime + 0.2) as f64 > self.time && l.prevsequence < m.numseq() {
            // blend from the last sequence
            let prev_seq = l.prevsequence.max(0);
            if let Some(pseq) = m.seq(prev_seq) {
                let mut prev = Pose::default();
                let pb = [l.prevseqblending[0] as f32 / 255.0, l.prevseqblending[1] as f32 / 255.0];
                self.calc_blended(&mut prev, model, &pseq, l.prevframe, pb);
                let s = 1.0 - ((self.time - l.sequencetime as f64) / 0.2) as f32;
                self.slerp_bones(&mut pose, &prev, s);
            }
        } else {
            self.ent().latched.prevframe = f as f32;
        }

        // The legs: gait-sequence bones up to the torso split.
        if let Some(info) = self.pinfo.as_mut() {
            if info.gaitsequence >= m.numseq() {
                info.gaitsequence = 0;
            }
        }
        let gait = self.pinfo.filter(|i| i.gaitsequence != 0);
        if let Some(info) = gait {
            if let Some(gseq) = m.seq(info.gaitsequence) {
                let mut legs = Pose::default();
                let anim = self.get_anim(model, &gseq);
                self.calc_rotations(&mut legs, &gseq, anim, info.gaitframe);
                split_torso(&m, &mut pose, &legs);
            }
        }

        self.write_bones(&m, &pose, |_| None);
    }

    /// The bone matrices from a pose (the end of `StudioSetupBones`/`StudioMergeBones`); `cached`
    /// supplies merged bones.
    fn write_bones(&self, m: &Studio, pose: &Pose, cached: impl Fn(usize) -> Option<(Mat34, Mat34)>) {
        let hw = es::is_hardware();
        let rot = es::rotation();
        let root = if hw { rot } else { es::alias() };
        let n = m.numbones();
        let mut bones = [[[0.0f32; 4]; 3]; MAX_BONES];
        let mut lights = bones;
        for i in 0..n {
            if let Some((b, l)) = cached(i) {
                bones[i] = b;
                lights[i] = l;
            } else {
                let mut bm = quaternion_matrix(pose.q[i]);
                for j in 0..3 {
                    bm[j][3] = pose.pos[i][j];
                }
                let parent = m.bone(i).parent;
                match usize::try_from(parent).ok().filter(|&p| p < i) {
                    None => {
                        bones[i] = concat(&root, &bm);
                        lights[i] = concat(&rot, &bm);
                        self.fx_transform(&mut bones[i]);
                    }
                    Some(p) => {
                        bones[i] = concat(&bones[p], &bm);
                        lights[i] = concat(&lights[p], &bm);
                    }
                }
            }
            es::set_bone(i, &bones[i]);
            es::set_light(i, &lights[i]);
        }
    }

    /// `StudioFxTransform` (`0x10030e60`).
    fn fx_transform(&self, t: &mut Mat34) {
        let e = self.ent();
        let fx = e.curstate.renderfx;
        if fx == kRenderFxDistort as i32 || fx == kRenderFxHologram as i32 {
            if r::random_long(0, 49) == 0 {
                let axis = if r::random_long(0, 1) == 1 { 2 } else { 0 };
                let k = r::random_float(1.0, 1.484);
                for x in t[axis].iter_mut().take(3) {
                    *x *= k;
                }
            } else if r::random_long(0, 49) == 0 {
                let _axis = r::random_long(0, 1);
                let offset = r::random_float(-10.0, 10.0);
                t[r::random_long(0, 2).clamp(0, 2) as usize][3] += offset;
            }
        } else if fx == kRenderFxExplode as i32 {
            let scale = ((1.0 + (self.time - e.curstate.animtime as f64) * 10.0) as f32).min(2.0);
            for row in t.iter_mut() {
                row[1] *= scale;
            }
        }
    }

    /// `StudioSaveBones` (`0x100317b0`).
    fn save_bones(&mut self) {
        let m = self.studio();
        self.cached.clear();
        for i in 0..m.numbones() {
            let src = m.bone(i).name;
            let mut name = [0u8; 32];
            name[..src.len()].copy_from_slice(src);
            self.cached.push(CachedBone { name, len: src.len(), bone: es::bone(i), light: es::light(i) });
        }
    }

    /// `StudioMergeBones` (`0x10031890`): bones named like a cached one copy its matrices (case
    /// insensitive), the rest animate normally.
    fn merge_bones(&mut self, sub: Model) {
        let m = self.studio();
        let Some(seq) = m.seq(self.clamp_sequence(&m)) else { return };
        let f = self.estimate_frame(&seq);
        let mut pose = Pose::default();
        let anim = self.get_anim(sub, &seq);
        self.calc_rotations(&mut pose, &seq, anim, f as f32);
        let cached = &self.cached;
        self.write_bones(&m, &pose, |i| {
            let name = m.bone(i).name;
            cached.iter().find(|c| c.name[..c.len].eq_ignore_ascii_case(name)).map(|c| (c.bone, c.light))
        });
    }

    /// `StudioCalcAttachments` (`0x10034560`).
    fn calc_attachments(&mut self) {
        let m = self.studio();
        let e = self.ent();
        // TS prints "Too many attachments on %s" and calls exit(-1) above 4; this draws the first 4.
        for i in 0..m.numattachments().min(4) {
            let a = m.attachment(i);
            let bone = usize::try_from(a.bone).unwrap_or(0).min(MAX_BONES - 1);
            e.attachment[i] = transform(a.org, &es::light(bone));
        }
    }

    // ---- rendering ------------------------------------------------------------------------------

    /// `StudioRenderModel` (`0x100345f0`), with the glow shell's second (chrome) pass.
    fn render(&mut self) {
        es::set_chrome_origin();
        es::set_force_face_flags(0);
        if self.ent().curstate.renderfx == kRenderFxGlowShell as i32 {
            self.ent().curstate.renderfx = kRenderFxNone as i32;
            self.render_final();
            let hw = es::is_hardware();
            if !hw {
                es::tri_render_mode(kRenderTransAdd as i32);
            }
            es::set_force_face_flags(es::NF_CHROME);
            es::chrome_sprite_texture();
            self.ent().curstate.renderfx = kRenderFxGlowShell as i32;
            self.render_final();
            if !hw {
                es::tri_render_mode(kRenderNormal as i32);
            }
        } else {
            self.render_final();
        }
    }

    /// `StudioRenderFinal` (`0x10034860`) -> `_Hardware` (`0x10034750`) / `_Software` (`0x10034690`).
    fn render_final(&mut self) {
        let hw = es::is_hardware();
        let rendermode = if !hw {
            0
        } else if es::force_face_flags() != 0 {
            kRenderTransAdd as i32
        } else {
            self.ent().curstate.rendermode
        };
        es::setup_renderer(rendermode);
        let de = self.draw_entities.value();
        if de == 2.0 {
            es::draw_bones();
        } else if de == 3.0 {
            es::draw_hulls();
        } else {
            for i in 0..self.studio().numbodyparts() {
                es::setup_model(i);
                if hw {
                    if self.do_interp {
                        // interpolation messes up bounding boxes (SDK)
                        self.ent().trivial_accept = 0;
                    }
                    es::gl_set_render_mode(rendermode);
                }
                es::draw_points();
                if hw {
                    es::gl_draw_shadow();
                }
            }
        }
        if de == 4.0 {
            es::tri_render_mode(kRenderTransAdd as i32);
            es::draw_hulls();
            es::tri_render_mode(kRenderNormal as i32);
        }
        if de == 5.0 {
            es::draw_abs_bbox();
        }
        es::restore_renderer();
    }
}

/// `StudioCalcBoneAdj` (`0x10030140`): controller values for this frame.
fn bone_adj(m: &Studio, dadt: f32, c1: &[u8; 4], c2: &[u8; 4], mouth: u8) -> [f32; 8] {
    let mut adj = [0.0f32; 8];
    for j in 0..m.numbonecontrollers().min(adj.len()) {
        let c = m.bone_controller(j);
        let i = c.index;
        let value = if (0..=3).contains(&i) {
            let i = i as usize;
            if c.kind & model::STUDIO_RLOOP != 0 {
                if (c1[i] as i32 - c2[i] as i32).abs() > 128 {
                    // SDK quirk kept by TS: the wrapped case indexes by controller number `j`.
                    let a = (c1.get(j).copied().unwrap_or(0) as i32 + 128) % 256;
                    let b = (c2.get(j).copied().unwrap_or(0) as i32 + 128) % 256;
                    ((a as f32 * dadt) + (b as f32 * (1.0 - dadt)) - 128.0) * (360.0 / 256.0) + c.start
                } else {
                    (c1[i] as f32 * dadt + c2[i] as f32 * (1.0 - dadt)) * (360.0 / 256.0) + c.start
                }
            } else {
                let v = ((c1[i] as f32 * dadt + c2[i] as f32 * (1.0 - dadt)) / 255.0).clamp(0.0, 1.0);
                (1.0 - v) * c.start + v * c.end
            }
        } else {
            let v = (mouth as f32 / 64.0).min(1.0);
            (1.0 - v) * c.start + v * c.end
        };
        match c.kind & model::STUDIO_TYPES {
            model::STUDIO_XR | model::STUDIO_YR | model::STUDIO_ZR => adj[j] = value.to_radians(),
            model::STUDIO_X | model::STUDIO_Y | model::STUDIO_Z => adj[j] = value,
            _ => {}
        }
    }
    adj
}

/// The body of `StudioCalcRotations` (`0x10030ca0`), with `StudioCalcBoneQuaterion`
/// (`0x10030380`) and `StudioCalcBonePosition` (`0x10030550`): the pose at frame `f`.
fn sample_pose(m: &Studio, pose: &mut Pose, seq: &SeqDesc, anim: Anim, f: f32, adj: &[f32; 8]) {
    let mut f = f;
    if f > (seq.numframes - 1) as f32 {
        f = 0.0; // bah, fix this bug with changing sequences too fast (SDK)
    } else if f < -0.01 {
        f = -0.01;
    }
    let frame = f as i32;
    let s = f - frame as f32;
    let ctl = |c: i32| usize::try_from(c).ok().and_then(|c| adj.get(c).copied());

    for i in 0..m.numbones() {
        let bone = m.bone(i);
        // rotation
        let (mut a1, mut a2) = ([0.0f32; 3], [0.0f32; 3]);
        for j in 0..3 {
            match anim.rotation_samples(i, j + 3, frame) {
                None => {
                    a1[j] = bone.value[j + 3];
                    a2[j] = a1[j];
                }
                Some((x1, x2)) => {
                    a1[j] = bone.value[j + 3] + x1 * bone.scale[j + 3];
                    a2[j] = bone.value[j + 3] + x2 * bone.scale[j + 3];
                }
            }
            if let Some(v) = ctl(bone.bonecontroller[j + 3]) {
                a1[j] += v;
                a2[j] += v;
            }
        }
        pose.q[i] = if a1 != a2 { quaternion_slerp(angle_quaternion(a1), angle_quaternion(a2), s) } else { angle_quaternion(a1) };
        // position
        for j in 0..3 {
            let mut p = bone.value[j];
            if let Some(x) = anim.position_sample(i, j, frame, s) {
                p += x * bone.scale[j];
            }
            if let Some(v) = ctl(bone.bonecontroller[j]) {
                p += v;
            }
            pose.pos[i][j] = p;
        }
    }

    if let Ok(mb) = usize::try_from(seq.motionbone) {
        if mb < MAX_BONES {
            for (axis, bit) in [model::STUDIO_X, model::STUDIO_Y, model::STUDIO_Z].into_iter().enumerate() {
                if seq.motiontype & bit != 0 {
                    pose.pos[mb][axis] = 0.0;
                }
            }
            // (The SDK's STUDIO_LX/LY/LZ terms are multiplied by 0.)
        }
    }
}

/// `StudioPlayerBlend` (`0x10030890`): the pitch blend of the upper body (0..255) and the pitch
/// left over for the model angles.
fn player_blend(seq: &SeqDesc, pitch: &mut f32) -> u8 {
    let blend = (*pitch * 3.0) as i32;
    let (start, end) = (seq.blendstart[0], seq.blendend[0]);
    if (blend as f32) < start {
        *pitch -= start / 3.0;
        0
    } else if (blend as f32) > end {
        *pitch -= end / 3.0;
        255
    } else {
        *pitch = 0.0;
        if end - start < 0.1 { 127 } else { (255.0 * (blend as f32 - start) / (end - start)) as i32 as u8 }
    }
}

/// The TS run+aim split (`StudioSetupBones` `0x10031080`): bones before the first `Bip01 Dorso` or
/// `Bip01 Spine` take the gait pose; that bone is then turned about its own (x) axis by the yaw
/// difference between the upper-body root and the gait root, so the torso keeps the upper-body
/// sequence's facing. (R§2.3 reads the asm as `a3[2] = a3[0] + d`; the stores are to `a3[0]`,
/// i.e. `a3[0] += d`, which Ghidra's C shows too.)
fn split_torso(m: &Studio, upper: &mut Pose, legs: &Pose) {
    let root_upper = upper.q[0];
    for i in 0..m.numbones() {
        let name = m.bone(i).name;
        if name == b"Bip01 Dorso" || name == b"Bip01 Spine" {
            let a1 = quaternion_angles(root_upper);
            let a2 = quaternion_angles(upper.q[0]);
            let d = a1[2] - a2[2];
            let mut a3 = quaternion_angles(upper.q[i]);
            a3[0] += d;
            upper.q[i] = angle_quaternion(a3);
            return;
        }
        upper.pos[i] = legs.pos[i];
        upper.q[i] = legs.q[i];
    }
}

#[cfg(test)]
mod tests {
    use super::model::tests::Builder;
    use super::*;

    fn pose_of(n: usize, f: impl Fn(usize) -> (Vec3, Quat)) -> Pose {
        let mut p = Pose::default();
        for i in 0..n {
            (p.pos[i], p.q[i]) = f(i);
        }
        p
    }

    #[test]
    fn torso_split_copies_legs_and_turns_the_spine() {
        let b = Builder {
            bones: vec![
                ("Bip01", -1, [0.0; 6], [1.0; 6]),
                ("Bip01 Pelvis", 0, [0.0; 6], [1.0; 6]),
                ("Bip01 Dorso", 1, [0.0; 6], [1.0; 6]),
                ("Bip01 Head", 2, [0.0; 6], [1.0; 6]),
            ],
            ..Default::default()
        };
        let data = b.build();
        let m = Studio::new(&data);
        // upper body faces yaw 0.5 rad at the root, legs face yaw 0.2
        let mut upper = pose_of(4, |i| ([1.0, 0.0, 0.0], angle_quaternion([0.0, 0.0, if i == 0 { 0.5 } else { 0.0 }])));
        let legs = pose_of(4, |i| ([2.0, 0.0, 0.0], angle_quaternion([0.0, 0.0, if i == 0 { 0.2 } else { 0.1 }])));
        split_torso(&m, &mut upper, &legs);
        assert_eq!(upper.pos[0], [2.0, 0.0, 0.0], "root from the legs");
        assert_eq!(upper.q[1], legs.q[1], "pelvis from the legs");
        assert_eq!(upper.pos[2], [1.0, 0.0, 0.0], "split bone keeps the upper position");
        let a = quaternion_angles(upper.q[2]);
        assert!((a[0] - 0.3).abs() < 1e-4, "spine turned about x by 0.5 - 0.2: {a:?}");
        assert_eq!(upper.q[3], angle_quaternion([0.0; 3]), "bones after the split stay upper");
    }

    #[test]
    fn spine_name_also_splits() {
        let b = Builder {
            bones: vec![("Bip01", -1, [0.0; 6], [1.0; 6]), ("Bip01 Spine", 0, [0.0; 6], [1.0; 6]), ("x", 1, [0.0; 6], [1.0; 6])],
            ..Default::default()
        };
        let data = b.build();
        let m = Studio::new(&data);
        let mut upper = pose_of(3, |_| ([1.0; 3], [0.0, 0.0, 0.0, 1.0]));
        let legs = pose_of(3, |_| ([2.0; 3], [0.0, 0.0, 0.0, 1.0]));
        split_torso(&m, &mut upper, &legs);
        assert_eq!((upper.pos[0], upper.pos[1], upper.pos[2]), ([2.0; 3], [1.0; 3], [1.0; 3]));
    }

    #[test]
    fn blend_and_controllers() {
        let seq = SeqDesc { blendstart: [-90.0, 0.0], blendend: [90.0, 0.0], ..Default::default() };
        let mut pitch = 0.0;
        assert_eq!(player_blend(&seq, &mut pitch), 127);
        let mut pitch = 40.0; // 120 > 90
        assert_eq!(player_blend(&seq, &mut pitch), 255);
        assert_eq!(pitch, 10.0);
        let mut pitch = -10.0;
        assert_eq!(player_blend(&seq, &mut pitch), 85);
        assert_eq!(pitch, 0.0);
        assert_eq!(torso_controller(0.0), 127);
        assert_eq!(torso_controller(120.0), 255);
        assert_eq!(torso_controller(-120.0), 0);
        assert_eq!(wrap360(370.0), 10.0);
        assert_eq!(wrap360(-370.0), -10.0);
    }

    #[test]
    fn sample_pose_interpolates_frames_and_applies_controllers() {
        use super::model::tests::run;
        use super::model::{BoneController, STUDIO_X, STUDIO_ZR};
        let b = Builder {
            bones: vec![("Bip01", -1, [5.0, 6.0, 7.0, 0.0, 0.0, 0.0], [1.0, 1.0, 1.0, 0.01, 0.01, 0.01])],
            seqs: vec![SeqDesc { numframes: 2, motiontype: STUDIO_X, motionbone: 0, ..Default::default() }],
            anims: vec![vec![[None, None, Some(run(2, 2, &[0, 10])), None, None, Some(run(2, 2, &[0, 100]))]]],
            controllers: vec![BoneController { bone: 0, kind: STUDIO_ZR, start: -30.0, end: 30.0, index: 0 }],
            ..Default::default()
        };
        let data = b.build();
        let m = Studio::new(&data);
        let seq = m.seq(0).unwrap();
        let anim = Anim { data: m.b, base: seq.animindex as usize };

        let mut pose = Pose::default();
        sample_pose(&m, &mut pose, &seq, anim, 0.5, &[0.0; 8]);
        let a = quaternion_angles(pose.q[0]);
        assert!((a[2] - 0.5).abs() < 1e-4, "half way between 0 and 1 rad: {a:?}");
        assert_eq!(pose.pos[0], [0.0, 6.0, 12.0], "x zeroed by the motion type, z = 7 + lerp(0, 10)");

        // Controller 0 fully on (255): +30 degrees of yaw.
        let adj = bone_adj(&m, 1.0, &[255, 0, 0, 0], &[0; 4], 0);
        assert!((adj[0] - 30f32.to_radians()).abs() < 1e-5);
        sample_pose(&m, &mut pose, &seq, anim, 0.0, &adj);
        let a = quaternion_angles(pose.q[0]);
        assert!((a[2] - 30f32.to_radians()).abs() < 1e-4, "{a:?}");
    }
}
