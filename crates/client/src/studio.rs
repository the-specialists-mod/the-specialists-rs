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
//! * `StudioDrawModel` (`0x10031ae0`) slows the view model's animation with the time scale and
//!   the local power-ups ([`Renderer::latch_view_model_rate`]).
//! * renderfx 17 (`kRenderFxDeadPlayer`) is redefined (`StudioDrawModel` `0x10031ae0`):
//!   `rendercolor.r` is the player, `rendercolor.b == 1` marks a motion-blur ghost
//!   (`StudioDrawGhostPlayer` `0x100337f0`), anything else a corpse (`StudioDrawDeadPlayer`
//!   `0x10033540`).
//!
//! The ruleset picks which of these run ([`StudioStyle`], `ClientRules::studio_style`): an older
//! renderer can keep the gait changes and drop the rest.
//!
//! Engine access goes through `goldsrc::ext::studio`; the model data is parsed from bytes
//! (`studio::model`).

pub mod math;
pub mod model;

use crate::{state, view};
use goldsrc::Global;
use goldsrc::client::{Cvar, engine};
use goldsrc::ext::render as r;
use goldsrc::ext::studio::{self as es, Ent, Header, Lighting, Mat34, Model, PlayerInfo};
use goldsrc::ext::tri;
use goldsrc::sys::*;
use math::*;
use model::{Anim, Bytes, SeqDesc, Studio};
use std::cell::RefCell;

/// The renderer's ruleset choices (`ClientRules::studio_style`). With every flag set the
/// renderer is the one the module docs describe. With a flag cleared, that part is the SDK's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StudioStyle {
    /// `StudioDrawModel` slows the view model's animation with the time scale and the local
    /// power-ups ([`Renderer::latch_view_model_rate`]).
    pub view_model_slowmo: bool,
    /// `StudioSetupBones` turns the split bone by the yaw between the upper-body and gait roots
    /// ([`split_torso`]). Without it the gait bones are copied up to the split bone and the split
    /// bone keeps the upper sequence's own rotation.
    pub torso_yaw_fix: bool,
    /// `StudioDrawPlayer` sets `pplayer->renderamt = 255` first.
    pub player_renderamt: bool,
    /// renderfx 17 draws a corpse from a private copy of its player's `player_info`
    /// (`StudioDrawDeadPlayer`), and `rendercolor.b == 1` a motion-blur ghost. Without it every
    /// renderfx 17 entity is a corpse that `StudioDrawPlayer` draws on the player's own
    /// `player_info` (gait sequence 0, `renderframe` written), with no ghost branch.
    pub dead_player_copy: bool,
    /// `StudioRenderFinal_Hardware` draws the absolute bounding box at `r_drawentities 5`. The
    /// software path always does.
    pub hw_abs_bbox: bool,
    /// `HUD_StudioEvent` 5004 plays its sound on the entity (`EV_PlaySound`, `CHAN_BODY`,
    /// attenuation 0.8, the slow-motion pitch). Without it the sound plays at the attachment
    /// with `pfnPlaySoundByNameAtLocation`, at volume 1 and normal pitch
    /// (`effects::studio_event`).
    pub slowed_event_sound: bool,
    /// `V_CalcNormalRefdef` pushes the view model `(80 − m_iFOV) × 0.15` units along the view
    /// after the bob offset (`view::view_model_fov_push`).
    pub view_model_fov_push: bool,
}

impl StudioStyle {
    /// Every renderer change on.
    pub const ALL: StudioStyle = StudioStyle {
        view_model_slowmo: true,
        torso_yaw_fix: true,
        player_renderamt: true,
        dead_player_copy: true,
        hw_abs_bbox: true,
        slowed_event_sound: true,
        view_model_fov_push: true,
    };
}

/// The current ruleset's renderer choices.
pub fn style() -> StudioStyle {
    crate::gameplay::current().studio_style()
}

/// Which `client.dll` build's floating-point forms the renderer and the view kick replicate
/// (`ClientRules::float_form`). Both builds run the x87 at 53-bit precision (modelled as `f64`);
/// they differ in where a value is rounded to a float and in how a constant divisor is applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FloatForm {
    /// An optimised build (TS 3.0): a division by a constant is a multiply by its double
    /// reciprocal (`×10`, `×5`, `×(1/255)`, `×(1/3)`), and a value stays on the x87 stack until it
    /// is stored to a float.
    Optimised,
    /// An unoptimised build (TS 2.1): the source's divisions (`/0.1`, `/0.2`, `/255.0`, `/3.0`),
    /// and every statement's result stored to its float variable.
    Unoptimised,
}

/// The current ruleset's float forms.
pub fn float_form() -> FloatForm {
    crate::gameplay::current().float_form()
}

/// `0x10079e68`: the double TS 3.0 multiplies by instead of dividing by 255.
const INV255: f64 = 1.0 / 255.0;
/// `0x1007d518` (3.0), `0x10270240` (2.1): `M_PI / 180` as a double.
const DEG2RAD: f64 = std::f64::consts::PI / 180.0;

impl FloatForm {
    /// `x / 255` as the build applies it (`fmul 0x10079e68` or `fdiv 255.0`).
    fn by255(self, x: f64) -> f64 {
        match self {
            FloatForm::Optimised => x * INV255,
            FloatForm::Unoptimised => x / 255.0,
        }
    }
}

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
    /// The view model's slowed framerate (`g_vmFramerate` `0x10094cf0`, initially 1) and the frame
    /// count and `latched.prevframe` of the last view-model draw that computed it (`0x100eae6c`,
    /// `0x100eae70`).
    vm_framerate: f32,
    vm_frame_count: i32,
    vm_prevframe: f32,
    /// The ruleset's choices, read at the start of each engine draw call.
    style: StudioStyle,
    /// The ruleset's float forms, read with `style`.
    form: FloatForm,
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
            vm_framerate: 1.0,
            vm_frame_count: 0,
            vm_prevframe: 0.0,
            style: StudioStyle::ALL,
            form: FloatForm::Optimised,
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
    let (style, form) = (style(), float_form());
    with(|r| {
        r.style = style;
        r.form = form;
        r.draw_model(flags)
    })
}

/// `R_StudioDrawPlayer` (`0x1001f3b0`) -> `StudioDrawPlayer`.
pub fn draw_player(flags: i32, player: &mut entity_state_s) -> bool {
    let (style, form) = (style(), float_form());
    with(|r| {
        r.style = style;
        r.form = form;
        r.draw_player(flags, player)
    })
}

fn time_scale() -> f32 {
    state::with(|s| s.time_scale)
}

/// Torso controller value for a yaw offset: `(yaw * 0.25 + 30) * 4.25` (the x87 code's doubles),
/// truncated to a byte.
fn torso_controller(yaw: f32) -> u8 {
    torso_byte(FloatForm::Optimised, yaw as f64)
}

/// [`torso_controller`] in a build's form: 3.0 `(yaw × 0.25 + 30) × 4.25` (`0x100323be`), 2.1
/// `(yaw / 4.0 + 30.0) / 0.23529411764705882` (`0x10066b82`), `__ftol`'s low byte.
fn torso_byte(form: FloatForm, yaw: f64) -> u8 {
    (match form {
        FloatForm::Optimised => (yaw * 0.25 + 30.0) * 4.25,
        FloatForm::Unoptimised => (yaw / 4.0 + 30.0) / 0.23529411764705882,
    }) as i32 as u8
}

/// The view model's animation rate in slow motion (`StudioDrawModel` `0x10031b44..0x10031bf4`):
/// the time scale, x3.5556 with slow-pause (8), x1.71429 with slow-motion (1), x1.5 with double
/// fire rate (0x10). `None` (rate left alone) at normal speed without double fire rate.
fn view_model_rate(powerups: i32, ts: f32) -> Option<f32> {
    if powerups & 0x10 == 0 && (0.999..=1.001).contains(&ts) {
        return None;
    }
    let mut r = ts;
    if powerups & 8 != 0 {
        r *= f32::from_bits(0x4063_8e3a); // 3.5555558 (0x1007d694)
    }
    if powerups & 1 != 0 {
        r *= f32::from_bits(0x3fdb_6db8); // 1.7142859 (0x1007d690)
    }
    if powerups & 0x10 != 0 {
        r *= 1.5;
    }
    Some(r)
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
        if self.style.view_model_slowmo
            && let Some(rate) = view_model_rate(state::with(|s| s.powerups), time_scale())
            && es::is_view_model(ent)
        {
            self.latch_view_model_rate(ent.get(), rate);
        }
        let e = ent.get();
        if e.curstate.renderfx == kRenderFxDeadPlayer as i32 {
            return if !self.style.dead_player_copy {
                self.draw_corpse_as_player(flags)
            } else if e.curstate.rendercolor.b != 1 {
                self.draw_corpse(flags)
            } else {
                self.draw_ghost(flags)
            };
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

    /// TS's view-model slow motion (`0x10031bb0..0x10031bf4`). The rate is taken into
    /// `g_vmFramerate` on the first draw of a frame when the last draw advanced the animation
    /// (`latched.prevframe` more than 0.001 past the one the draw before saw), and written to
    /// `curstate.framerate` on every draw. At normal speed TS leaves the framerate alone, so the
    /// last slowed rate holds until the engine resets it to 1 on the next weapon animation.
    fn latch_view_model_rate(&mut self, e: &mut cl_entity_s, rate: f32) {
        if self.vm_frame_count != self.frame_count && self.vm_prevframe + 0.001 < e.latched.prevframe {
            self.vm_framerate = rate;
        }
        e.curstate.framerate = self.vm_framerate;
        self.vm_prevframe = e.latched.prevframe;
        self.vm_frame_count = self.frame_count;
    }

    /// renderfx 17, `rendercolor.b != 1`: a corpse draws as its player, from a copy of the player's
    /// state and `player_info` so the live player's gait isn't disturbed.
    fn draw_corpse(&mut self, flags: i32) -> bool {
        let Some((who, mut dead)) = self.corpse_state() else { return false };
        let Some(info) = es::player_info(who - 1) else { return false };
        self.local_info = info;
        let interp = std::mem::replace(&mut self.do_interp, false);
        let res = self.draw_dead_player(flags, &mut dead);
        self.do_interp = interp;
        res
    }

    /// The corpse's player (`rendercolor.r`, 1..maxclients) and the state it is drawn from: a copy
    /// of the player's state with no weapon, no gait and `MOVETYPE_NONE`, numbered as the player,
    /// at the entity's origin and angles.
    fn corpse_state(&self) -> Option<(i32, entity_state_s)> {
        let e = self.ent();
        let who = e.curstate.rendercolor.r as i32;
        if who == 0 || who > engine().max_clients() {
            return None;
        }
        // (The SDK-style renderer copies the state without a NULL check.)
        let mut dead = es::player_state(who - 1)?;
        dead.number = who;
        dead.weaponmodel = 0;
        dead.gaitsequence = 0;
        dead.movetype = MOVETYPE_NONE as i32;
        dead.angles = e.curstate.angles;
        dead.origin = e.curstate.origin;
        Some((who, dead))
    }

    /// renderfx 17 without [`StudioStyle::dead_player_copy`] (TS 2.1 `StudioDrawModel` 0x10065d50,
    /// `0x10065da0..`): every such entity is a corpse, drawn by `StudioDrawPlayer` with
    /// interpolation off. That draw works on the player's own `player_info`.
    fn draw_corpse_as_player(&mut self, flags: i32) -> bool {
        let Some((_, mut dead)) = self.corpse_state() else { return false };
        let interp = std::mem::replace(&mut self.do_interp, false);
        let res = self.draw_player(flags, &mut dead);
        self.do_interp = interp;
        res
    }

    /// renderfx 17, `rendercolor.b == 1`: a motion-blur ghost (`effects::mblur`). The gait state
    /// the ghost froze is in `iuser1` (gaitsequence), `iuser2` (renderframe), `fuser3` (gaitframe)
    /// and `fuser4` (gaityaw).
    fn draw_ghost(&mut self, flags: i32) -> bool {
        let e = self.ent();
        let owner = e.curstate.rendercolor.r as i32;
        if engine().local_player().is_some_and(|p| p.index == owner) && (!view::is_third_person() || (self.motionblur.value() as i32) < 2) {
            return false;
        }
        if time_scale() > 0.99 {
            return false;
        }
        // (TS asks the engine for any number; not every engine range-checks PlayerInfo.)
        if !(1..=engine().max_clients()).contains(&e.curstate.number) {
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
        if self.player_index < 0 || self.player_index >= engine().max_clients() {
            return None;
        }
        // Hidden while spectating this player in eye mode. TS tests g_iUser1 == 5 here (the SDK's
        // in-eye is 4); `effects::add_entity` already drops the in-eye target for it.
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
            engine().with_entity_mut(index, |g| g.attachment = attachment);
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
        if let Some(wm) = es::model_by_index(weaponmodel)
            && self.set_header(wm)
        {
            self.merge_bones(wm);
            es::setup_lighting(lighting);
            self.render();
            self.calc_attachments();
        }
        *self.ent() = save;
    }

    /// `StudioDrawPlayer` (`0x10032540`): the SDK flow; forces `renderamt = 255` on the state
    /// ([`StudioStyle::player_renderamt`]).
    fn draw_player(&mut self, flags: i32, player: &mut entity_state_s) -> bool {
        if self.style.player_renderamt {
            player.renderamt = 255;
        }
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

    /// `StudioProcessGait`'s `dt`: the frame time as a float, below 0 → 0 (3.0's `test ah,1`
    /// at `0x100322d8` also takes a NaN; 2.1's `jp` at `0x10066a3e` doesn't), above 1 → 1.
    fn dt(&self) -> f32 {
        let dt = (self.time - self.old_time) as f32;
        if dt < 0.0 || (self.form == FloatForm::Optimised && dt.is_nan()) {
            0.0
        } else if dt > 1.0 {
            1.0
        } else {
            dt
        }
    }

    /// `StudioEstimateGait` (3.0 `0x10031fa0`, 2.1 `0x10066300`), TS version: 2-D movement, and
    /// the gait movement is scaled by 0.75 below 180 units/s ([`gait_step`]).
    #[allow(clippy::neg_cmp_op_on_partial_ord)] // the x87 tests: a NaN goes the way TS's flags send it
    fn estimate_gait(&mut self, player: &entity_state_s) {
        let form = self.form;
        let dt = (self.time - self.old_time) as f32;
        let e = self.ent();
        let frame_count = self.frame_count;
        let Some(info) = self.pinfo.as_mut() else { return };
        // 3.0 (`0x10031fb8`): below 0 or NaN ends it; 2.1 (`0x1006632e`): below 0 becomes 0 (and
        // ends it as 0), a NaN goes on. Both cap at 1 and end on 0.
        if form == FloatForm::Optimised && !(dt >= 0.0) {
            self.gait_movement = 0.0;
            return;
        }
        let dt = dt.clamp(0.0, 1.0);
        if dt == 0.0 || info.renderframe == frame_count {
            self.gait_movement = 0.0;
            return;
        }
        let est = if self.gait_estimation {
            let est = [e.origin[0] - info.prevgaitorigin[0], e.origin[1] - info.prevgaitorigin[1]];
            info.prevgaitorigin = e.origin;
            Gait::Origin(est)
        } else {
            Gait::Velocity([player.velocity[0], player.velocity[1]])
        };
        let (movement, gaityaw) = gait_step(form, dt, est, e.angles[1], info.gaityaw);
        self.gait_movement = movement;
        info.gaityaw = gaityaw;
    }

    /// The side-to-side turn of `StudioProcessGait`: torso controllers from the view yaw relative
    /// to the gait yaw, then the body faces the gait yaw. Also the ghost's torso (TS
    /// `StudioDrawGhostPlayer` repeats this block).
    fn turn_torso(&mut self) {
        let form = self.form;
        let e = self.ent();
        let Some(info) = self.pinfo.as_mut() else { return };
        let (byte, gaityaw, movement) = torso_turn(form, e.angles[1], info.gaityaw, self.gait_movement);
        info.gaityaw = gaityaw;
        self.gait_movement = movement;
        e.curstate.controller[..4].fill(byte);
        e.latched.prevcontroller = e.curstate.controller;
        e.angles[1] = info.gaityaw;
        // 3.0 `test ah,1` (`0x10032458`) also takes a NaN; 2.1 (`0x10066cb6`) doesn't.
        if e.angles[1] < 0.0 || (form == FloatForm::Optimised && e.angles[1].is_nan()) {
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
            let blend = player_blend(self.form, &seq, &mut e.angles[0]);
            e.latched.prevangles[0] = e.angles[0];
            e.curstate.blending[0] = blend;
            e.latched.prevblending[0] = blend;
            e.latched.prevseqblending[0] = blend;
        }
        let dt = self.dt();
        let form = self.form;
        self.estimate_gait(player);
        self.turn_torso();

        // TS drops the SDK's range check here; an out-of-range gait sequence is treated as 0.
        if player.gaitsequence >= m.numseq() {
            player.gaitsequence = 0;
        }
        let Some(seq) = m.seq(player.gaitsequence) else { return };
        let framerate = self.ent().curstate.framerate;
        let Some(info) = self.pinfo.as_mut() else { return };
        info.gaitframe = gait_frame(form, info.gaitframe, self.gait_movement, &seq, framerate, dt);
    }

    // ---- transforms and bones -------------------------------------------------------------------

    /// `StudioSetUpTransform` (`0x10030970`).
    #[allow(clippy::needless_range_loop)] // per-component and per-column loops, as in the SDK
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

    /// `StudioEstimateInterpolant` (3.0 `0x10030c40`, 2.1 `0x10064520`): `(time - animtime) / 0.1`
    /// capped at 2 once `animtime` has moved 0.01 past `prevanimtime` (the double sum). 3.0
    /// multiplies by 10 and returns the x87 value (its callers store it to a float); 2.1 divides
    /// and stores first. A NaN test passes in 3.0 (`test ah,0x41; je`) and fails in 2.1
    /// (`test ah,0x41; jp`); a NaN result isn't capped in either.
    fn estimate_interpolant(&self) -> f32 {
        let e = self.ent();
        if !self.do_interp {
            return 1.0;
        }
        interpolant(self.form, self.time, e.curstate.animtime, e.latched.prevanimtime)
    }

    /// `StudioEstimateFrame` (3.0 `0x10030f80`, 2.1 `0x10064b00`): the SDK's, in doubles.
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
        // The `< 0` tests: 3.0's `test ah,1` (`0x1003101f`, `0x1003105e`) also takes a NaN, 2.1's
        // `test ah,5; jp` (`0x10064c0d`, `0x10064c59`) doesn't.
        let below = |f: f64| f < 0.0 || (self.form == FloatForm::Optimised && f.is_nan());
        if seq.flags & model::STUDIO_LOOPING != 0 {
            if seq.numframes > 1 {
                f -= (f / (n - 1.0)).trunc() * (n - 1.0);
            }
            if below(f) {
                f += n - 1.0;
            }
        } else {
            if f >= n - 1.001 {
                f = n - 1.001;
            }
            if below(f) {
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
            return Anim { data: m.b, base: model::at(data.max(0) as usize, seq.animindex.max(0) as usize) };
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
        let adj = bone_adj(&m, self.form, dadt, &c1, &c2, mouth);
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
        // `(cur·dadt + prev·(1 − dadt)) / 255` on the x87, stored as the float argument
        // (3.0 `0x1003115c..0x10031181`, 2.1 `0x10064e16..0x10064e4c`).
        let form = self.form;
        let s0 = seq_blend(form, blending[0], l.prevblending[0], dadt);
        let s1 = seq_blend(form, blending[1], l.prevblending[1], dadt);
        let mut pose = Pose::default();
        self.calc_blended(&mut pose, model, &seq, f as f32, [s0, s1]);

        if self.do_interp && l.sequencetime != 0.0 && l.sequencetime as f64 + 0.2 > self.time && l.prevsequence < m.numseq() {
            // blend from the last sequence
            let prev_seq = l.prevsequence.max(0);
            if let Some(pseq) = m.seq(prev_seq) {
                let mut prev = Pose::default();
                let pb = l.prevseqblending.map(|b| form.by255(b as f64) as f32);
                self.calc_blended(&mut prev, model, &pseq, l.prevframe, pb);
                let s = prev_seq_weight(form, self.time, l.sequencetime);
                self.slerp_bones(&mut pose, &prev, s);
            }
        } else {
            self.ent().latched.prevframe = f as f32;
        }

        // The legs: gait-sequence bones up to the torso split.
        if let Some(info) = self.pinfo.as_mut()
            && info.gaitsequence >= m.numseq()
        {
            info.gaitsequence = 0;
        }
        let gait = self.pinfo.filter(|i| i.gaitsequence != 0);
        if let Some(info) = gait
            && let Some(gseq) = m.seq(info.gaitsequence)
        {
            let mut legs = Pose::default();
            let anim = self.get_anim(model, &gseq);
            self.calc_rotations(&mut legs, &gseq, anim, info.gaitframe);
            split_torso(&m, &mut pose, &legs, self.style.torso_yaw_fix);
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
                for (row, p) in bm.iter_mut().zip(pose.pos[i]) {
                    row[3] = p;
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
                tri::render_mode(kRenderTransAdd as i32);
            }
            es::set_force_face_flags(es::NF_CHROME);
            es::chrome_sprite_texture();
            self.ent().curstate.renderfx = kRenderFxGlowShell as i32;
            self.render_final();
            if !hw {
                tri::render_mode(kRenderNormal as i32);
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
                // A body part the engine can't set up (a malformed model or body) isn't drawn
                // (port guard in `goldsrc`).
                if !es::setup_model(i) {
                    continue;
                }
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
            tri::render_mode(kRenderTransAdd as i32);
            es::draw_hulls();
            tri::render_mode(kRenderNormal as i32);
        }
        if de == 5.0 && (!hw || self.style.hw_abs_bbox) {
            es::draw_abs_bbox();
        }
        es::restore_renderer();
    }
}

/// `StudioProcessGait`'s torso turn (3.0 `0x10032312..0x10032469`, 2.1 `0x10066a88..0x10066ce0`;
/// also 3.0's `StudioDrawGhostPlayer`): the view yaw relative to the gait yaw, wrapped to ±180;
/// beyond ±120 the gait turns round (gait yaw ∓ 180, movement negated). Returns the torso
/// controller byte, the gait yaw and the movement. 3.0 keeps the yaw on the x87 (a NaN reads as
/// below −180 and −120); 2.1 stores every step.
#[allow(clippy::neg_cmp_op_on_partial_ord)] // the x87 tests: a NaN goes the way TS's flags send it
fn torso_turn(form: FloatForm, view_yaw: f32, gaityaw: f32, movement: f32) -> (u8, f32, f32) {
    let (mut g, mut m) = (gaityaw, movement);
    let y = if form == FloatForm::Optimised {
        let mut y = view_yaw as f64 - gaityaw as f64;
        y = wrap360_by(y, y * f32::from_bits(0x3b36_0b61) as f64);
        if !(y >= -180.0) {
            y += 360.0;
        }
        if y > 180.0 {
            y -= 360.0;
        }
        if y > 120.0 {
            g -= 180.0;
            m = -m;
            y -= 180.0;
        }
        if !(y >= -120.0) {
            g += 180.0;
            m = -m;
            y += 180.0;
        }
        y
    } else {
        let mut y = view_yaw - gaityaw;
        y = wrap360_by(y as f64, y as f64 / 360.0) as f32;
        if y < -180.0 {
            y += 360.0;
        }
        if y > 180.0 {
            y -= 360.0;
        }
        if y > 120.0 {
            g -= 180.0;
            m = -m;
            y -= 180.0;
        }
        if y < -120.0 {
            g += 180.0;
            m = -m;
            y += 180.0;
        }
        y as f64
    };
    (torso_byte(form, y), g, m)
}

/// `StudioProcessGait`'s gait frame (3.0 `0x1003249a..0x10032523`, 2.1 `0x10066d3d..0x10066e21`):
/// `movement / linearmovement.x × numframes` when the sequence moves, else 3.0's
/// `framerate × fps × dt` or 2.1's `dt × fps × framerate` (the x87 products in that order), added
/// and stored; then `− ftol(frame / numframes) × numframes` (the quotient unrounded, the `int`
/// product wrapping) and `+ numframes` below 0 (3.0 also on NaN). The port skips the wrap for a
/// sequence without frames, where TS divides by 0.
fn gait_frame(form: FloatForm, frame: f32, movement: f32, seq: &SeqDesc, framerate: f32, dt: f32) -> f32 {
    let n = seq.numframes as f64;
    let add = if seq.linearmovement[0] > 0.0 {
        movement as f64 / seq.linearmovement[0] as f64 * n
    } else {
        match form {
            FloatForm::Optimised => framerate as f64 * seq.fps as f64 * dt as f64,
            FloatForm::Unoptimised => dt as f64 * seq.fps as f64 * framerate as f64,
        }
    };
    let mut f = (add + frame as f64) as f32;
    if seq.numframes > 0 {
        f = (f as f64 - ((f as f64 / n) as i32).wrapping_mul(seq.numframes) as f64) as f32;
        if f < 0.0 || (form == FloatForm::Optimised && f.is_nan()) {
            f = (n + f as f64) as f32;
        }
    }
    f
}

/// Where `StudioEstimateGait` takes the movement from: the origin's change since the last
/// estimate (`m_fGaitEstimation`), or the player's velocity.
#[derive(Clone, Copy, Debug)]
enum Gait {
    Origin([f32; 2]),
    Velocity([f32; 2]),
}

/// `x − ftol(x · (1/360))·360` with the `int` product wrapping as C's does (the gait yaw wraps).
fn wrap360_by(x: f64, q: f64) -> f64 {
    x - (q as i32).wrapping_mul(360) as f64
}

/// The body of `StudioEstimateGait` once `dt` is in (0, 1]: the gait movement and the new gait
/// yaw. 3.0 (`0x10032005..0x1003222c`) keeps the length, speed, yaw difference and products on the
/// x87 (`fsqrt`, `fpatan`, `× 57.29577951308232`, `× (1/360)f`) and compares the unrounded speed
/// with 180; a NaN movement counts as stationary. 2.1 (`0x1006638b..0x10066802`) stores `x²` to a
/// float before adding `y²` and every statement after (CRT `sqrt`/`atan2`, `× 180 / π`,
/// `/ 360.0f`), tests the speed against 5 unrounded and against 180 stored.
#[allow(clippy::neg_cmp_op_on_partial_ord)] // the x87 tests: a NaN goes the way TS's flags send it
fn gait_step(form: FloatForm, dt: f32, est: Gait, yaw: f32, gaityaw: f32) -> (f32, f32) {
    let opt = form == FloatForm::Optimised;
    let len = |v: [f32; 2]| {
        let (x, y) = (v[0] as f64, v[1] as f64);
        if opt { (y * y + x * x).sqrt() } else { ((x * x) as f32 as f64 + y * y).sqrt() }
    };
    let dtd = dt as f64;
    let (mut movement, speed, est, mut still) = match est {
        Gait::Origin(v) => {
            let l = len(v);
            let movement = l as f32;
            let (speed, slow) = if opt {
                // `fst` keeps `l / dt` for the 180 test; the 5 test reads the stored float.
                let sp = l / dtd;
                (sp, !(dt > 0.0) || !(sp as f32 >= 5.0))
            } else {
                let sp = movement as f64 / dtd;
                ((sp as f32) as f64, !(dt > 0.0 || dt.is_nan()) || sp < 5.0)
            };
            if slow { (0.0, speed, [0.0; 2], true) } else { (movement, speed, v, false) }
        }
        Gait::Velocity(v) => {
            let p = len(v) * dtd;
            (p as f32, if opt { p } else { (p as f32) as f64 }, v, false)
        }
    };
    // 3.0 `test ah,0x40; je` (`0x100320e6`): a NaN reads as 0; 2.1 `test ah,0x44; jp`: not.
    let zero = |x: f32| x == 0.0 || (opt && x.is_nan());
    still |= zero(est[1]) && zero(est[0]);
    let mut g;
    if still {
        if opt {
            let mut d = yaw as f64 - gaityaw as f64;
            d = wrap360_by(d, d * f32::from_bits(0x3b36_0b61) as f64);
            if d > 180.0 {
                d -= 360.0;
            }
            if !(d >= -180.0) {
                d += 360.0;
            }
            d *= dtd;
            if dtd < 0.25 {
                d *= 4.0;
            }
            g = (d + gaityaw as f64) as f32;
            g = wrap360_by(g as f64, g as f64 * f32::from_bits(0x3b36_0b61) as f64) as f32;
        } else {
            let mut d = yaw - gaityaw;
            d = wrap360_by(d as f64, d as f64 / 360.0) as f32;
            if d > 180.0 {
                d -= 360.0;
            }
            if d < -180.0 {
                d += 360.0;
            }
            d = if dtd < 0.25 { (dtd * 4.0 * d as f64) as f32 } else { d * dt };
            g = d + gaityaw;
            g = wrap360_by(g as f64, g as f64 / 360.0) as f32;
        }
        movement = 0.0;
    } else {
        let a = (est[1] as f64).atan2(est[0] as f64);
        g = if opt { a * 57.29577951308232 } else { a * 180.0 / std::f64::consts::PI } as f32;
        if g > 180.0 {
            g = 180.0;
        }
        if if opt { !(g >= -180.0) } else { g < -180.0 } {
            g = -180.0;
        }
    }
    if if opt { !(speed >= 180.0) } else { speed < 180.0 } {
        movement = (movement as f64 * 0.75) as f32;
    }
    (movement, g)
}

/// The `StudioEstimateInterpolant` value with interpolation on: see
/// [`Renderer::estimate_interpolant`].
#[allow(clippy::neg_cmp_op_on_partial_ord)] // the x87 tests: a NaN goes the way TS's flags send it
fn interpolant(form: FloatForm, time: f64, animtime: f32, prevanimtime: f32) -> f32 {
    let (animtime, prev) = (animtime as f64, prevanimtime as f64 + 0.01);
    let moved = match form {
        FloatForm::Optimised => !(prev > animtime),
        FloatForm::Unoptimised => prev <= animtime,
    };
    if !moved {
        return 1.0;
    }
    let x = match form {
        FloatForm::Optimised => (time - animtime) * 10.0,
        FloatForm::Unoptimised => ((time - animtime) / 0.1) as f32 as f64,
    };
    if x > 2.0 { 2.0 } else { x as f32 }
}

/// A sequence blend for `StudioSetupBones`: `(cur·dadt + prev·(1 − dadt)) / 255` on the x87,
/// stored as the float argument (3.0 `0x1003115c..0x10031181`, 2.1 `0x10064e16..0x10064e4c`).
fn seq_blend(form: FloatForm, cur: u8, prev: u8, dadt: f32) -> f32 {
    form.by255(cur as f64 * dadt as f64 + prev as f64 * (1.0 - dadt as f64)) as f32
}

/// The weight of the previous sequence's pose: `1 − (time − sequencetime)·5` (3.0 `0x1003146d`)
/// or `1 − (time − sequencetime) / 0.2` (2.1 `0x1006526f`), stored as a float.
fn prev_seq_weight(form: FloatForm, time: f64, sequencetime: f32) -> f32 {
    let dt = time - sequencetime as f64;
    (match form {
        FloatForm::Optimised => 1.0 - dt * 5.0,
        FloatForm::Unoptimised => 1.0 - dt / 0.2,
    }) as f32
}

/// `StudioCalcBoneAdj` (3.0 `0x10030140`, 2.1 `0x10062e70`): controller values for this frame.
/// The blend runs on the x87 from the float `dadt`; 3.0 keeps the value there through the clamp,
/// the lerp and the degrees-to-radians multiply (`×(1/255)`), 2.1 stores it to a float after each
/// statement (`/255.0`, `/64.0`). Both multiply by the double `M_PI / 180`.
#[allow(clippy::neg_cmp_op_on_partial_ord)] // the x87 tests: a NaN goes the way TS's flags send it
fn bone_adj(m: &Studio, form: FloatForm, dadt: f32, c1: &[u8; 4], c2: &[u8; 4], mouth: u8) -> [f32; 8] {
    let opt = form == FloatForm::Optimised;
    // A statement's result: kept (3.0) or stored to a float (2.1).
    let st = |x: f64| if opt { x } else { x as f32 as f64 };
    let (d, d1) = (dadt as f64, 1.0 - dadt as f64);
    let lerp = |v: f64, c: &model::BoneController| st((1.0 - v) * c.start as f64 + v * c.end as f64);
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
                    st(((a as f64 * d + b as f64 * d1) - 128.0) * (360.0 / 256.0) + c.start as f64)
                } else {
                    st((c1[i] as f64 * d + c2[i] as f64 * d1) * (360.0 / 256.0) + c.start as f64)
                }
            } else {
                let v = st(form.by255(c1[i] as f64 * d + c2[i] as f64 * d1));
                // 3.0 `0x10030288`: below 0 or NaN becomes 0 (`test ah,1`); 2.1 `0x10063014`: only
                // below 0 (`test ah,5; jp`). Above 1 becomes 1 in both.
                let v = if if opt { !(v >= 0.0) } else { v < 0.0 } { 0.0 } else { v };
                let v = if v > 1.0 { 1.0 } else { v };
                lerp(v, &c)
            }
        } else {
            let v = match form {
                FloatForm::Optimised => mouth as f64 * (1.0 / 64.0),
                FloatForm::Unoptimised => (mouth as f64 / 64.0) as f32 as f64,
            };
            lerp(if v > 1.0 { 1.0 } else { v }, &c)
        };
        match c.kind & model::STUDIO_TYPES {
            model::STUDIO_XR | model::STUDIO_YR | model::STUDIO_ZR => adj[j] = (value * DEG2RAD) as f32,
            model::STUDIO_X | model::STUDIO_Y | model::STUDIO_Z => adj[j] = value as f32,
            _ => {}
        }
    }
    adj
}

/// The body of `StudioCalcRotations` (`0x10030ca0`), with `StudioCalcBoneQuaterion`
/// (`0x10030380`) and `StudioCalcBonePosition` (`0x10030550`): the pose at frame `f`.
fn sample_pose(m: &Studio, pose: &mut Pose, seq: &SeqDesc, anim: Anim, f: f32, adj: &[f32; 8]) {
    let mut f = f;
    if f > seq.numframes.saturating_sub(1) as f32 {
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

    if let Ok(mb) = usize::try_from(seq.motionbone)
        && mb < MAX_BONES
    {
        for (axis, bit) in [model::STUDIO_X, model::STUDIO_Y, model::STUDIO_Z].into_iter().enumerate() {
            if seq.motiontype & bit != 0 {
                pose.pos[mb][axis] = 0.0;
            }
        }
        // (The SDK's STUDIO_LX/LY/LZ terms are multiplied by 0.)
    }
}

/// `StudioPlayerBlend` (3.0 `0x10030890`, 2.1 `0x10063d00`): the pitch blend of the upper body
/// (0..255) and the pitch left over for the model angles. `pitch × 3.0f` is truncated unrounded;
/// the leftover pitch is `pitch − start × (1/3)` (3.0, double reciprocal) or `pitch − start / 3.0`
/// (2.1), and the blend `(blend − start)·255 / (end − start)`, all on the x87. 3.0 compares the
/// blend after a float store and takes a NaN `start` as below it (`test ah,1`).
#[allow(clippy::neg_cmp_op_on_partial_ord)] // the x87 tests: a NaN goes the way TS's flags send it
fn player_blend(form: FloatForm, seq: &SeqDesc, pitch: &mut f32) -> u8 {
    let blend = (*pitch as f64 * 3.0) as i32;
    let (start, end) = (seq.blendstart[0] as f64, seq.blendend[0] as f64);
    let b = match form {
        FloatForm::Optimised => blend as f32 as f64,
        FloatForm::Unoptimised => blend as f64,
    };
    let third = |x: f64| match form {
        FloatForm::Optimised => x * (1.0 / 3.0),
        FloatForm::Unoptimised => x / 3.0,
    };
    let below = match form {
        FloatForm::Optimised => !(b >= start),
        FloatForm::Unoptimised => b < start,
    };
    if below {
        *pitch = (*pitch as f64 - third(start)) as f32;
        0
    } else if b > end {
        *pitch = (*pitch as f64 - third(end)) as f32;
        255
    } else {
        *pitch = 0.0;
        let d = end - start;
        let narrow = match form {
            FloatForm::Optimised => !(d >= 0.1),
            FloatForm::Unoptimised => d < 0.1,
        };
        // `__ftol` into the int TS stores; the caller keeps its low byte.
        if narrow { 127 } else { ((b - start) * 255.0 / d) as i32 as u8 }
    }
}

/// The TS run+aim split (`StudioSetupBones` `0x10031080`): bones before the first `Bip01 Dorso` or
/// `Bip01 Spine` take the gait pose; that bone is then turned about its own (x) axis by the yaw
/// difference between the upper-body root and the gait root, so the torso keeps the upper-body
/// sequence's facing. (R§2.3 reads the asm as `a3[2] = a3[0] + d`; the stores are to `a3[0]`,
/// i.e. `a3[0] += d`, which Ghidra's C shows too.) Without `yaw_fix` (TS 2.1 `StudioSetupBones`
/// 0x10064ce0, the same `_strcmp` loop) the split bone keeps the upper pose untouched.
fn split_torso(m: &Studio, upper: &mut Pose, legs: &Pose, yaw_fix: bool) {
    let root_upper = upper.q[0];
    for i in 0..m.numbones() {
        let name = m.bone(i).name;
        if name == b"Bip01 Dorso" || name == b"Bip01 Spine" {
            if !yaw_fix {
                return;
            }
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
        let unfixed = {
            let mut u = upper.clone();
            split_torso(&m, &mut u, &legs, false);
            u
        };
        split_torso(&m, &mut upper, &legs, true);
        assert_eq!(upper.pos[0], [2.0, 0.0, 0.0], "root from the legs");
        assert_eq!(upper.q[1], legs.q[1], "pelvis from the legs");
        assert_eq!(upper.pos[2], [1.0, 0.0, 0.0], "split bone keeps the upper position");
        let a = quaternion_angles(upper.q[2]);
        assert!((a[0] - 0.3).abs() < 1e-4, "spine turned about x by 0.5 - 0.2: {a:?}");
        assert_eq!(upper.q[3], angle_quaternion([0.0; 3]), "bones after the split stay upper");

        // TS 2.1 (`StudioSetupBones` 0x10064ce0): the same copy, the split bone left alone.
        assert_eq!((unfixed.pos[0], unfixed.q[0], unfixed.q[1]), (legs.pos[0], legs.q[0], legs.q[1]));
        assert_eq!((unfixed.pos[2], unfixed.q[2], unfixed.q[3]), ([1.0, 0.0, 0.0], angle_quaternion([0.0; 3]), angle_quaternion([0.0; 3])));
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
        split_torso(&m, &mut upper, &legs, true);
        assert_eq!((upper.pos[0], upper.pos[1], upper.pos[2]), ([2.0; 3], [1.0; 3], [1.0; 3]));
    }

    #[test]
    fn blend_and_controllers() {
        let seq = SeqDesc { blendstart: [-90.0, 0.0], blendend: [90.0, 0.0], ..Default::default() };
        let mut pitch = 0.0;
        assert_eq!(player_blend(FloatForm::Optimised, &seq, &mut pitch), 127);
        let mut pitch = 40.0; // 120 > 90
        assert_eq!(player_blend(FloatForm::Optimised, &seq, &mut pitch), 255);
        assert_eq!(pitch, 10.0);
        let mut pitch = -10.0;
        assert_eq!(player_blend(FloatForm::Optimised, &seq, &mut pitch), 85);
        assert_eq!(pitch, 0.0);
        assert_eq!(torso_controller(0.0), 127);
        assert_eq!(torso_controller(120.0), 255);
        assert_eq!(torso_controller(-120.0), 0);
        // The gait yaw turns round past ±120 and wraps to ±180 in both builds.
        for form in [FloatForm::Optimised, FloatForm::Unoptimised] {
            assert_eq!(torso_turn(form, 370.0, 0.0, 2.0), (torso_controller(10.0), 0.0, 2.0));
            assert_eq!(torso_turn(form, 150.0, 0.0, 2.0), (torso_controller(-30.0), -180.0, -2.0));
            assert_eq!(torso_turn(form, -150.0, 0.0, 2.0), (torso_controller(30.0), 180.0, -2.0));
        }
    }

    #[test]
    fn view_model_rate_and_latch() {
        assert_eq!(view_model_rate(0, 1.0), None, "normal speed: framerate left alone");
        assert_eq!(view_model_rate(0, 1.0005), None);
        assert_eq!(view_model_rate(0x10, 1.0), Some(1.5));
        assert_eq!(view_model_rate(0, 0.5), Some(0.5));
        let r = view_model_rate(1 | 8, 0.3).unwrap();
        assert!((r - 0.3 * 3.5555558 * 1.7142859).abs() < 1e-5, "{r}");

        let mut rd = Renderer { frame_count: 1, ..Default::default() };
        let mut e = cl_entity_s::default();
        e.latched.prevframe = 0.5;
        rd.latch_view_model_rate(&mut e, 0.4);
        assert_eq!(e.curstate.framerate, 0.4, "first draw: the animation advanced from 0");
        // Same frame (a second draw): written, not re-latched.
        e.latched.prevframe = 0.9;
        rd.latch_view_model_rate(&mut e, 0.2);
        assert_eq!(e.curstate.framerate, 0.4);
        // Next frame, animation restarted (prevframe fell back): the old rate holds.
        rd.frame_count = 2;
        e.latched.prevframe = 0.1;
        rd.latch_view_model_rate(&mut e, 0.2);
        assert_eq!(e.curstate.framerate, 0.4);
        // Next frame, advancing again: latched.
        rd.frame_count = 3;
        e.latched.prevframe = 0.3;
        rd.latch_view_model_rate(&mut e, 0.2);
        assert_eq!(e.curstate.framerate, 0.2);
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
        let adj = bone_adj(&m, FloatForm::Optimised, 1.0, &[255, 0, 0, 0], &[0; 4], 0);
        assert!((adj[0] - 30f32.to_radians()).abs() < 1e-5);
        sample_pose(&m, &mut pose, &seq, anim, 0.0, &adj);
        let a = quaternion_angles(pose.q[0]);
        assert!((a[2] - 30f32.to_radians()).abs() < 1e-4, "{a:?}");
    }

    #[test]
    fn float_forms_are_the_builds() {
        use super::model::tests::run;
        use super::model::{BoneController, STUDIO_ZR};
        use FloatForm::{Optimised as O, Unoptimised as U};
        let fb = f32::from_bits;
        // `StudioEstimateInterpolant`: the 0.01 test is a double sum (3.0 `0x10030c5c`, 2.1
        // `0x10064554`): `animtime` 1.01f is short of 1.0 + 0.01. ×10 / `/0.1` capped at 2.
        assert_eq!([interpolant(O, 1.05, 1.01, 1.0), interpolant(U, 1.05, 1.01, 1.0)], [1.0, 1.0]);
        assert_eq!([interpolant(O, 1.5, 1.2, 1.0), interpolant(U, 1.5, 1.2, 1.0)], [2.0, 2.0]);
        // A NaN `prevanimtime` passes 3.0's test (`test ah,0x41; je`), not 2.1's (`jp`).
        assert_eq!(interpolant(O, 1.15, 1.1, f32::NAN), ((1.15 - 1.1f32 as f64) * 10.0) as f32);
        assert_eq!(interpolant(U, 1.15, 1.1, f32::NAN), 1.0);
        // The blend sums on the x87 before one float store (3.0 `0x1003115c`, 2.1 `0x10064e16`);
        // float products gave 0x3e994a87.
        for form in [O, U] {
            assert_eq!(seq_blend(form, 171, 68, fb(0x3da5_f368)).to_bits(), 0x3e99_4a86);
            assert_eq!(prev_seq_weight(form, 437.2898315981447, fb(0x43da_914c)).to_bits(), 0x3e67_efed);
        }
        // `StudioPlayerBlend`: `pitch × 3.0f` truncated unrounded (−84.999996 → −84, the float
        // product rounded to −85), then `(b − start)·255 / (end − start)` on the x87 (190, the
        // float form 191).
        let seq = SeqDesc { blendstart: [-90.0, 0.0], blendend: [90.0, 0.0], ..Default::default() };
        let seq2 = SeqDesc { blendstart: [fb(0xc20b_28c7), 0.0], blendend: [fb(0x4297_7b71), 0.0], ..Default::default() };
        for form in [O, U] {
            assert_eq!(player_blend(form, &seq, &mut fb(0xc1e2_aaaa)), 8);
            assert_eq!(player_blend(form, &seq2, &mut 16.0), 190);
        }
        // A NaN start is "below" in 3.0 only (`0x100308c5` `test ah,1`, the pitch becomes NaN);
        // 2.1 (`0x10063d31` `jp`) goes on to the NaN `__ftol` (low byte 0) and zeroes the pitch.
        let nan = SeqDesc { blendstart: [f32::NAN, 0.0], blendend: [90.0, 0.0], ..Default::default() };
        let (mut p3, mut p2) = (1.0, 1.0);
        assert_eq!([player_blend(O, &nan, &mut p3), player_blend(U, &nan, &mut p2)], [0, 0]);
        assert!(p3.is_nan() && p2 == 0.0);
        // `StudioCalcBoneAdj`: 3.0 keeps the value on the x87 through `×(1/255)`, the lerp and
        // `× M_PI/180` (`0x10030261..0x10030307`); 2.1 stores it after `/255.0` and after the
        // lerp (`0x1006300e`, `0x10063069`).
        let b = Builder {
            bones: vec![("Bip01", -1, [0.0; 6], [1.0; 6])],
            seqs: vec![SeqDesc { numframes: 2, ..Default::default() }],
            anims: vec![vec![[None, None, Some(run(2, 2, &[0, 0])), None, None, None]]],
            controllers: vec![BoneController { bone: 0, kind: STUDIO_ZR, start: -45.0, end: 30.0, index: 0 }],
            ..Default::default()
        };
        let data = b.build();
        let m = Studio::new(&data);
        let adj = |form| bone_adj(&m, form, fb(0x3f0a_e1c6), &[206, 0, 0, 0], &[10, 0, 0, 0], 0)[0].to_bits();
        assert_eq!([adj(O), adj(U)], [0xbe40_bfb6, 0xbe40_bfb8]);
    }

    #[test]
    fn gait_forms_are_the_builds() {
        use FloatForm::{Optimised as O, Unoptimised as U};
        let fb = f32::from_bits;
        // Velocity path: 3.0 `fsqrt(vy² + vx²) · dt` on the x87 (`0x100320bf..0x100320d7`); 2.1
        // stores `vx²` first (`0x100665ca`). Both × 0.75 below 180; the gait yaw from atan2.
        let v = Gait::Velocity([fb(0xc331_4b59), fb(0x4200_f3ef)]);
        let (m3, g3) = gait_step(O, fb(0x3d48_352f), v, 0.0, 0.0);
        let (m2, g2) = gait_step(U, fb(0x3d48_352f), v, 0.0, 0.0);
        assert_eq!([m3.to_bits(), m2.to_bits()], [0x40d3_649c, 0x40d3_649a]);
        assert_eq!([g3.to_bits(), g2.to_bits()], [0x4329_b1ba; 2]);
        // Standing: the yaw difference wraps, then `× dt × 4` below 0.25 s. 3.0 keeps it on the
        // x87 (`0x1003210b..0x1003217c`), 2.1 stores each step (`0x10066656..0x100666fb`).
        let still = Gait::Origin([0.0; 2]);
        let step = |form| gait_step(form, fb(0x3e47_f583), still, fb(0x431f_800d), fb(0x4082_7e50));
        assert_eq!([step(O), step(U)].map(|(m, g)| (m, g.to_bits())), [(0.0, 0x42fa_f412), (0.0, 0x42fa_f411)]);
        // The gait frame's `framerate × fps × dt + frame` is one x87 expression stored once
        // (3.0 `0x100324bb..0x100324ce`, 2.1 `0x10066d8a..0x10066da8`); float steps gave 0x40da7904.
        let gseq = SeqDesc { numframes: 30, fps: fb(0x4213_c69f), ..Default::default() };
        for form in [O, U] {
            let f = gait_frame(form, fb(0x4085_5e76), 0.0, &gseq, fb(0x3fb9_0932), fb(0x3d4b_f8a6));
            assert_eq!(f.to_bits(), 0x40da_7905);
        }
        // Under 5 u/s from the origin: no movement, the standing branch.
        let (m, _) = gait_step(O, 0.1, Gait::Origin([0.3, 0.0]), 10.0, 0.0);
        assert_eq!(m, 0.0);
        // A NaN velocity reads as standing in 3.0 (`test ah,0x40`) and goes to atan2 in 2.1.
        let nan = Gait::Velocity([f32::NAN, f32::NAN]);
        assert_eq!(gait_step(O, 0.1, nan, 0.0, 10.0), (0.0, 6.0));
        assert!(gait_step(U, 0.1, nan, 0.0, 10.0).1.is_nan());
    }

    /// Delta-state angles, frames and sequences and model data of any value: no overflow, no
    /// out-of-range bone or controller index.
    #[test]
    fn garbage_inputs_never_panic() {
        for x in [f32::MAX, f32::MIN, f32::INFINITY, f32::NAN, 1e12, -1e12, 0.0] {
            torso_controller(x);
            let seq = SeqDesc { blendstart: [x, 0.0], blendend: [-x, 0.0], ..Default::default() };
            for form in [FloatForm::Optimised, FloatForm::Unoptimised] {
                player_blend(form, &seq, &mut x.clone());
                torso_turn(form, x, -x, x);
                gait_step(form, 0.5, Gait::Origin([x, -x]), x, x);
                gait_step(form, 0.5, Gait::Velocity([x, 0.0]), x, -x);
                let gseq = SeqDesc { numframes: 3, fps: x, linearmovement: [x, 0.0, 0.0], ..Default::default() };
                gait_frame(form, x, x, &gseq, x, 0.5);
            }
        }
        let mut x = 0x1234_5678_u32;
        let mut bufs: Vec<Vec<u8>> = vec![vec![], vec![0xff; 1024]];
        for n in 1..30 {
            bufs.push(
                (0..n * 61)
                    .map(|_| {
                        x ^= x << 13;
                        x ^= x >> 17;
                        x ^= x << 5;
                        x as u8
                    })
                    .collect(),
            );
        }
        for b in &bufs {
            let m = Studio::new(b);
            bone_adj(&m, FloatForm::Unoptimised, f32::NAN, &[255; 4], &[0; 4], 255);
            let adj = bone_adj(&m, FloatForm::Optimised, f32::NAN, &[255; 4], &[0; 4], 255);
            for numframes in [i32::MIN, -1, 0, 1, 2, i32::MAX] {
                let seq = SeqDesc { numframes, motionbone: i32::MAX, motiontype: 7, ..Default::default() };
                for f in [f32::NAN, f32::INFINITY, -f32::INFINITY, -5.0, 0.0, 1e9] {
                    let mut pose = Pose::default();
                    sample_pose(&m, &mut pose, &seq, Anim { data: Bytes(b), base: usize::MAX }, f, &adj);
                    sample_pose(&m, &mut pose, &seq, Anim { data: Bytes(b), base: 0 }, f, &adj);
                }
            }
            let mut legs = Pose::default();
            split_torso(&m, &mut legs, &Pose::default(), true);
        }
    }
}
