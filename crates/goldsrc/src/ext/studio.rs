//! The engine studio API (`engine_studio_api_t`, r_studioint.h) for the client's studio model
//! renderer (`ts_client::studio`), plus the custom temp entity the motion-blur ghosts use.
//!
//! The engine hands the table to `HUD_GetStudioModelInterface`; [`init`] keeps a copy. Everything
//! else here is a safe wrapper over one table slot:
//!
//! * entities and models are passed as handles ([`Ent`], [`Model`]) that wrap the engine's pointers;
//! * the studio header is exposed as a byte slice ([`Header::data`]) that the renderer parses
//!   itself, so no studio structure is dereferenced through a pointer outside this module;
//! * the bone, light, alias and rotation matrices the engine draws from are read and written by
//!   value ([`bone`], [`set_bone`], ...), so no Rust reference into them outlives a call.
//!
//! The engine runs every studio callback on its one client thread. Model data and entities stay
//! valid for the frame being drawn; don't keep [`Header`] slices across frames (models are freed
//! on map change).

use crate::Global;
use crate::sys::*;
use std::cell::{Cell, RefCell};
use std::ffi::{CStr, c_int, c_void};
use std::ptr::NonNull;

/// `MAXSTUDIOBONES` (studio.h): the engine's bone matrix arrays have this many entries.
pub const MAX_BONES: usize = 128;
/// `STUDIO_NF_CHROME` (studio.h), the glow-shell force-face flag.
pub const NF_CHROME: i32 = 2;
/// A bone matrix, `float[3][4]`.
pub type Mat34 = [[f32; 4]; 3];

/// TS 3.0 (built against the 2007 SDK) copies 43 slots: the table ends at `GL_SetRenderMode`.
/// The newer slots (`StudioSetRenderamt`, ...) may not exist on older engines and are never used.
const TS_TABLE_SLOTS: usize = 43;

#[derive(Clone, Copy)]
struct Api {
    f: engine_studio_api_s,
    bones: *mut [Mat34; MAX_BONES],
    lights: *mut [Mat34; MAX_BONES],
    alias: *mut Mat34,
    rotation: *mut Mat34,
    models_drawn: *mut c_int,
    model_count: *mut c_int,
    chrome: *mut model_s,
}

static API: Global<Cell<Option<Api>>> = Global(Cell::new(None));

fn api() -> Api {
    API.0.get().expect("studio API not initialised (HUD_GetStudioModelInterface)")
}

/// Calls a studio API slot, panicking with its name if the engine left it null.
macro_rules! st {
    ($name:ident ( $($arg:expr),* $(,)? )) => {
        // SAFETY: `init` copied the engine's table, whose slots stay valid for the process; each
        // slot is called with the argument types from r_studioint.h, and callers pass pointers
        // that are valid for the call.
        unsafe { (api().f.$name.expect(stringify!($name)))($($arg),*) }
    };
}

/// Copies the engine's studio API (the first 43 slots, as TS does) and caches the pointers the
/// renderer needs (`CStudioModelRenderer::Init` `0x10030030`). Returns false for a null table.
///
/// # Safety
/// `table` must be null or the `engine_studio_api_t` the engine passed to
/// `HUD_GetStudioModelInterface`, with at least the first 43 slots filled in.
pub unsafe fn init(table: *const engine_studio_api_s) -> bool {
    if table.is_null() {
        return false;
    }
    let mut f = engine_studio_api_s::default();
    // SAFETY: the engine's table holds at least `TS_TABLE_SLOTS` function pointers (the caller's
    // contract); the rest of `f` stays None. Both are plain arrays of optional fn pointers.
    unsafe {
        std::ptr::copy_nonoverlapping(
            table.cast::<u8>(),
            (&mut f as *mut engine_studio_api_s).cast::<u8>(),
            TS_TABLE_SLOTS * size_of::<usize>(),
        )
    };
    API.0.set(Some(Api {
        f,
        bones: std::ptr::null_mut(),
        lights: std::ptr::null_mut(),
        alias: std::ptr::null_mut(),
        rotation: std::ptr::null_mut(),
        models_drawn: std::ptr::null_mut(),
        model_count: std::ptr::null_mut(),
        chrome: std::ptr::null_mut(),
    }));
    let mut a = api();
    a.bones = st!(StudioGetBoneTransform()).cast();
    a.lights = st!(StudioGetLightTransform()).cast();
    a.alias = st!(StudioGetAliasTransform()).cast();
    a.rotation = st!(StudioGetRotationMatrix()).cast();
    a.chrome = st!(GetChromeSprite());
    let (mut s, mut d) = (std::ptr::null_mut(), std::ptr::null_mut());
    st!(GetModelCounters(&mut s, &mut d));
    a.model_count = s;
    a.models_drawn = d;
    API.0.set(Some(a));
    true
}

// ---- handles ------------------------------------------------------------------------------------

/// A client entity owned by the engine (the entity being drawn, a player, a temp entity). Like
/// `EdictPtr`, [`Ent::get`] hands out a `&'static mut`: keep each borrow short and don't hold one
/// across [`client_events`] (it calls `HUD_StudioEvent` with the same entity).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Ent(NonNull<cl_entity_s>);

impl Ent {
    pub fn from_ptr(p: *mut cl_entity_s) -> Option<Ent> {
        NonNull::new(p).map(Ent)
    }
    pub fn get(self) -> &'static mut cl_entity_s {
        // SAFETY: the pointer came from the engine, which keeps its entities alive for the frame
        // and runs studio callbacks on one thread (see the type docs for the borrow rule).
        unsafe { &mut *self.0.as_ptr() }
    }
    pub fn model(self) -> Option<Model> {
        Model::from_ptr(self.get().model)
    }
}

/// A `model_t` owned by the engine.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Model(NonNull<model_s>);

impl Model {
    pub fn from_ptr(p: *mut model_s) -> Option<Model> {
        NonNull::new(p).map(Model)
    }
    pub fn as_ptr(self) -> *mut model_s {
        self.0.as_ptr()
    }
    pub fn name(self) -> String {
        // SAFETY: model handles point at engine models, which outlive the frame.
        crate::util::c_array_str(unsafe { &(*self.0.as_ptr()).name })
    }
}

/// A studio header (`studiohdr_t`) and the model data after it, as loaded by the engine.
#[derive(Clone, Copy)]
pub struct Header {
    ptr: NonNull<u8>,
    len: usize,
}

/// `sizeof(studiohdr_t)`; `length` is at byte 72.
const STUDIOHDR_SIZE: usize = 244;

impl Header {
    /// The model as the engine keeps it, from the header up to the texture pixels (or `length`).
    /// Valid for the frame.
    pub fn data(&self) -> &'static [u8] {
        // SAFETY: `extradata` checked that the engine's model block starts with a header; the
        // engine allocates (FWGS: `Mem_Calloc(length)`) and copies the file there, at least up to
        // the texture pixels, and keeps it until the model is freed.
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }
}

/// Reads a studio-style length field (`int` at `offset`) from a block the engine loaded.
///
/// # Safety
/// `p` must point at least `offset + 4` readable bytes.
unsafe fn read_len(p: *const u8, offset: usize) -> Option<usize> {
    // SAFETY: the caller guarantees `offset + 4` readable bytes.
    let n = unsafe { p.add(offset).cast::<i32>().read_unaligned() };
    usize::try_from(n).ok()
}

/// `IEngineStudio.Mod_Extradata`: the studio header of a studio model (None for other models).
pub fn extradata(m: Model) -> Option<Header> {
    let p = NonNull::new(st!(Mod_Extradata(m.as_ptr())).cast::<u8>())?;
    // SAFETY: Mod_Extradata returns the model's cache block, which starts with a `studiohdr_t`
    // (`length` at 72, `numtextures` at 180, `texturedataindex` at 188).
    let (mut len, numtextures, texdata) = unsafe { (read_len(p.as_ptr(), 72)?, read_len(p.as_ptr(), 180), read_len(p.as_ptr(), 188)) };
    // studiomdl writes the texture pixels last, and engines may drop them from the cached copy
    // (FWGS does with XASH_LOW_MEMORY, and updates `length`). The renderer never reads them, so
    // the slice stops there whatever `length` says.
    if let (Some(1..), Some(t)) = (numtextures, texdata)
        && (STUDIOHDR_SIZE..len).contains(&t)
    {
        len = t;
    }
    (len >= STUDIOHDR_SIZE).then_some(Header { ptr: p, len })
}

/// `IEngineStudio.StudioSetHeader`.
pub fn set_header(h: Header) {
    st!(StudioSetHeader(h.ptr.as_ptr().cast::<c_void>()));
}
/// `IEngineStudio.SetRenderModel`.
pub fn set_render_model(m: Model) {
    st!(SetRenderModel(m.as_ptr()));
}
/// `IEngineStudio.GetModelByIndex`.
pub fn model_by_index(index: i32) -> Option<Model> {
    Model::from_ptr(st!(GetModelByIndex(index)))
}
/// `IEngineStudio.SetupPlayerModel` (0-based player number).
pub fn setup_player_model(player: i32) -> Option<Model> {
    Model::from_ptr(st!(SetupPlayerModel(player)))
}
/// `IEngineStudio.GetCurrentEntity`.
pub fn current_entity() -> Option<Ent> {
    Ent::from_ptr(st!(GetCurrentEntity()))
}

/// Whether `e` is the engine's view model (`gEngfuncs.GetViewModel() == e`).
pub fn is_view_model(e: Ent) -> bool {
    std::ptr::eq(crate::cl_eng!(GetViewModel()), e.0.as_ptr())
}

/// Animation data of a demand-loaded sequence group (`StudioGetAnim` `0x100307e0`, group > 0):
/// the engine caches group files in a `cache_user_t[16]` hung off the model's `submodels` field.
/// Returns the group file's bytes (its `studioseqhdr_t` and animation data).
pub fn seqgroup_data(m: Model, group: usize, file: &CStr) -> Option<&'static [u8]> {
    if group >= 16 {
        return None;
    }
    let model = m.as_ptr();
    // SAFETY: `m` is an engine model. A studio model's `submodels` field is unused by the engine
    // except as this cache list (SDK `StudioGetAnim`), allocated below with 16 entries.
    let mut list = unsafe { (*model).submodels.cast::<cache_user_s>() };
    if list.is_null() {
        list = st!(Mem_Calloc(16, size_of::<cache_user_s>())).cast::<cache_user_s>();
        if list.is_null() {
            return None;
        }
        // SAFETY: as above; the engine zeroed the 16 entries.
        unsafe { (*model).submodels = list.cast() };
    }
    // SAFETY: `group < 16`, inside the list.
    let slot = unsafe { list.add(group) };
    if st!(Cache_Check(slot)).is_null() {
        st!(LoadCacheFile(file.as_ptr(), slot));
    }
    // SAFETY: `slot` is a valid cache_user_t; after LoadCacheFile its data is null or the whole
    // group file, which starts with a `studioseqhdr_t` (id, version, name[64], length) and is
    // `length` bytes long.
    unsafe {
        let data = (*slot).data.cast::<u8>();
        if data.is_null() {
            return None;
        }
        let len = read_len(data, 72)?;
        (len >= 76).then(|| std::slice::from_raw_parts(data, len))
    }
}

// ---- players --------------------------------------------------------------------------------------

/// The `player_info_t` fields the renderer reads and writes (gait state and colours).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerInfo {
    pub topcolor: i32,
    pub bottomcolor: i32,
    pub renderframe: i32,
    pub gaitsequence: i32,
    pub gaitframe: f32,
    pub gaityaw: f32,
    pub prevgaitorigin: [f32; 3],
}

/// `IEngineStudio.PlayerInfo` (0-based player number), copied out.
pub fn player_info(player: i32) -> Option<PlayerInfo> {
    let p = st!(PlayerInfo(player));
    // SAFETY: the engine returns null or its player_info_t for this player, valid for the frame.
    let p = unsafe { p.as_ref() }?;
    Some(PlayerInfo {
        topcolor: p.topcolor,
        bottomcolor: p.bottomcolor,
        renderframe: p.renderframe,
        gaitsequence: p.gaitsequence,
        gaitframe: p.gaitframe,
        gaityaw: p.gaityaw,
        prevgaitorigin: p.prevgaitorigin,
    })
}

/// Writes the gait fields back to the engine's `player_info_t` (the colours are read-only).
pub fn set_player_info(player: i32, i: &PlayerInfo) {
    let p = st!(PlayerInfo(player));
    // SAFETY: as in `player_info`; the reference doesn't outlive this block.
    if let Some(p) = unsafe { p.as_mut() } {
        p.renderframe = i.renderframe;
        p.gaitsequence = i.gaitsequence;
        p.gaitframe = i.gaitframe;
        p.gaityaw = i.gaityaw;
        p.prevgaitorigin = i.prevgaitorigin;
    }
}

/// `IEngineStudio.GetPlayerState` (0-based), copied out.
pub fn player_state(player: i32) -> Option<entity_state_s> {
    let p = st!(GetPlayerState(player));
    // SAFETY: the engine returns null or the latest state it received for this player.
    unsafe { p.as_ref() }.copied()
}

// ---- frame info -----------------------------------------------------------------------------------

/// `IEngineStudio.GetTimes`: (frame count, current time, previous time).
pub fn times() -> (i32, f64, f64) {
    let (mut n, mut t, mut o) = (0, 0.0, 0.0);
    st!(GetTimes(&mut n, &mut t, &mut o));
    (n, t, o)
}

/// Render origin and view vectors (`IEngineStudio.GetViewInfo`).
#[derive(Clone, Copy, Debug, Default)]
pub struct ViewInfo {
    pub origin: [f32; 3],
    pub up: [f32; 3],
    pub right: [f32; 3],
    pub forward: [f32; 3],
}

pub fn view_info() -> ViewInfo {
    let mut v = ViewInfo::default();
    st!(GetViewInfo(v.origin.as_mut_ptr(), v.up.as_mut_ptr(), v.right.as_mut_ptr(), v.forward.as_mut_ptr()));
    v
}

/// `IEngineStudio.GetAliasScale` (software renderer x/y scale).
pub fn alias_scale() -> (f32, f32) {
    let (mut x, mut y) = (0.0, 0.0);
    st!(GetAliasScale(&mut x, &mut y));
    (x, y)
}

// ---- matrices -------------------------------------------------------------------------------------

fn bone_slot(arr: *mut [Mat34; MAX_BONES], i: usize) -> *mut Mat34 {
    assert!(i < MAX_BONES && !arr.is_null(), "bone index {i} out of range");
    // SAFETY: `arr` is the engine's `float[MAXSTUDIOBONES][3][4]` and `i` is in range.
    unsafe { (*arr).as_mut_ptr().add(i) }
}

/// Bone `i` of the engine's bone transform array (what the engine skins with).
pub fn bone(i: usize) -> Mat34 {
    // SAFETY: `bone_slot` returns an in-range element of the engine's static array.
    unsafe { *bone_slot(api().bones, i) }
}
pub fn set_bone(i: usize, m: &Mat34) {
    // SAFETY: as in `bone`; written by value, no reference kept.
    unsafe { *bone_slot(api().bones, i) = *m }
}
/// Bone `i` of the engine's light transform array (lighting and attachments).
pub fn light(i: usize) -> Mat34 {
    // SAFETY: as in `bone`.
    unsafe { *bone_slot(api().lights, i) }
}
pub fn set_light(i: usize, m: &Mat34) {
    // SAFETY: as in `set_bone`.
    unsafe { *bone_slot(api().lights, i) = *m }
}

fn mat(p: *mut Mat34) -> *mut Mat34 {
    assert!(!p.is_null(), "studio matrix not available");
    p
}
/// The model's rotation matrix (`m_protationmatrix`).
pub fn rotation() -> Mat34 {
    // SAFETY: the engine's static `float[3][4]`, non-null (checked).
    unsafe { *mat(api().rotation) }
}
pub fn set_rotation(m: &Mat34) {
    // SAFETY: as in `rotation`.
    unsafe { *mat(api().rotation) = *m }
}
/// The software renderer's alias transform (`m_paliastransform`).
pub fn alias() -> Mat34 {
    // SAFETY: as in `rotation`.
    unsafe { *mat(api().alias) }
}
pub fn set_alias(m: &Mat34) {
    // SAFETY: as in `rotation`.
    unsafe { *mat(api().alias) = *m }
}

// ---- drawing --------------------------------------------------------------------------------------

/// `IEngineStudio.StudioCheckBBox`: false when the model is outside the view.
pub fn check_bbox() -> bool {
    st!(StudioCheckBBox()) != 0
}

/// Bumps the engine's model counters (`(*m_pModelsDrawn)++; (*m_pStudioModelCount)++`).
pub fn count_model() {
    let a = api();
    // SAFETY: the counters are engine ints returned by GetModelCounters (null-checked).
    unsafe {
        if let Some(n) = a.models_drawn.as_mut() {
            *n += 1;
        }
        if let Some(n) = a.model_count.as_mut() {
            *n += 1;
        }
    }
}

/// `alight_t` with its own light-vector storage (`lighting.plightvec = dir`).
pub struct Lighting {
    light: alight_s,
    dir: Box<[f32; 3]>,
}

impl Default for Lighting {
    fn default() -> Self {
        Lighting { light: alight_s::default(), dir: Box::new([0.0; 3]) }
    }
}

impl Lighting {
    fn raw(&mut self) -> *mut alight_s {
        // The vector lives in a Box, so the pointer stays valid however `self` moves.
        self.light.plightvec = self.dir.as_mut_ptr();
        &mut self.light
    }
}

/// `IEngineStudio.StudioDynamicLight`.
pub fn dynamic_light(e: Ent, l: &mut Lighting) {
    st!(StudioDynamicLight(e.0.as_ptr(), l.raw()));
}
/// `IEngineStudio.StudioEntityLight`.
pub fn entity_light(l: &mut Lighting) {
    st!(StudioEntityLight(l.raw()));
}
/// `IEngineStudio.StudioSetupLighting`.
pub fn setup_lighting(l: &mut Lighting) {
    st!(StudioSetupLighting(l.raw()));
}

pub fn set_remap_colors(top: i32, bottom: i32) {
    st!(StudioSetRemapColors(top, bottom));
}
pub fn set_chrome_origin() {
    st!(SetChromeOrigin());
}
pub fn force_face_flags() -> i32 {
    st!(GetForceFaceFlags())
}
pub fn set_force_face_flags(flags: i32) {
    st!(SetForceFaceFlags(flags));
}
pub fn setup_renderer(rendermode: i32) {
    st!(SetupRenderer(rendermode));
}
pub fn restore_renderer() {
    st!(RestoreRenderer());
}
pub fn is_hardware() -> bool {
    st!(IsHardware()) != 0
}
/// `IEngineStudio.StudioSetupModel` (the body part and submodel pointers aren't needed here).
pub fn setup_model(bodypart: i32) {
    let (mut bp, mut sm) = (std::ptr::null_mut(), std::ptr::null_mut());
    st!(StudioSetupModel(bodypart, &mut bp, &mut sm));
}
pub fn draw_points() {
    st!(StudioDrawPoints());
}
pub fn draw_bones() {
    st!(StudioDrawBones());
}
pub fn draw_hulls() {
    st!(StudioDrawHulls());
}
pub fn draw_abs_bbox() {
    st!(StudioDrawAbsBBox());
}
pub fn gl_set_render_mode(mode: i32) {
    st!(GL_SetRenderMode(mode));
}
pub fn gl_draw_shadow() {
    st!(GL_StudioDrawShadow());
}
/// `IEngineStudio.StudioClientEvents`: plays the frame's animation events, calling
/// `HUD_StudioEvent` with the current entity.
pub fn client_events() {
    st!(StudioClientEvents());
}

/// TriAPI `SpriteTexture(chrome sprite, 0)`, the glow shell's second pass.
pub fn chrome_sprite_texture() {
    let chrome = api().chrome;
    if !chrome.is_null() {
        crate::ext::tri::sprite_texture(chrome, 0);
    }
}

// ---- custom temp entities -------------------------------------------------------------------------

/// A temp entity's per-frame callback (`FTENT_CLIENTCUSTOM`): (entity, frametime, client time).
pub type TempEntCallback = fn(&mut TEMPENTITY, f32, f32);

const MAX_TENT_CALLBACKS: usize = 4;
static TENT_CALLBACKS: Global<RefCell<Vec<TempEntCallback>>> = Global(RefCell::new(Vec::new()));

unsafe extern "C" fn tent_trampoline<const N: usize>(ent: *mut TEMPENTITY, frametime: f32, client_time: f32) {
    // SAFETY: the temp-entity loop passes the live temp entity it is updating (or null).
    let Some(ent) = (unsafe { ent.as_mut() }) else { return };
    let f = TENT_CALLBACKS.0.borrow().get(N).copied();
    if let Some(f) = f {
        f(ent, frametime, client_time);
    }
}

type RawTentCallback = unsafe extern "C" fn(*mut TEMPENTITY, f32, f32);
static TENT_TRAMPOLINES: [RawTentCallback; MAX_TENT_CALLBACKS] =
    [tent_trampoline::<0>, tent_trampoline::<1>, tent_trampoline::<2>, tent_trampoline::<3>];

/// `CL_TempEntAllocCustom(origin, model, high, callback)`. Each distinct callback gets its own
/// trampoline slot (at most four).
pub fn alloc_custom_tempent(origin: [f32; 3], model: Model, high: bool, cb: TempEntCallback) -> Option<&'static mut TEMPENTITY> {
    let slot = {
        let mut cbs = TENT_CALLBACKS.0.borrow_mut();
        match cbs.iter().position(|&f| f as usize == cb as usize) {
            Some(i) => i,
            None => {
                assert!(cbs.len() < MAX_TENT_CALLBACKS, "too many temp-entity callbacks");
                cbs.push(cb);
                cbs.len() - 1
            }
        }
    };
    let p = crate::efx!(CL_TentEntAllocCustom(origin.as_ptr(), model.as_ptr(), high as c_int, Some(TENT_TRAMPOLINES[slot])));
    // SAFETY: the engine returns null or a temp entity it owns, alive until it dies.
    unsafe { p.as_mut() }
}
