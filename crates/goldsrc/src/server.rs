//! Server-side engine handle (`enginefuncs_t` + `globalvars_t`) and helpers.

use crate::edict::EdictPtr;
use crate::guard::{self, Kind};
use crate::sys::*;
use crate::util::{cstr, to_cstring};
use std::ffi::{CStr, c_int};
use std::sync::OnceLock;

pub struct Engine {
    pub f: enginefuncs_t,
    pub globals: *mut globalvars_t,
}
// SAFETY: the table and globals pointer are engine-owned for the process, and the engine runs
// every game-DLL callback on one thread.
unsafe impl Send for Engine {}
// SAFETY: as above.
unsafe impl Sync for Engine {}

static ENGINE: OnceLock<Engine> = OnceLock::new();

/// Called from the server library's `GiveFnptrsToDll` export.
///
/// # Safety
/// `funcs` must point to a valid engine function table.
pub unsafe fn init(funcs: *const enginefuncs_t, globals: *mut globalvars_t) {
    // SAFETY: `funcs` points at the engine's function table for the duration of this call; it's copied once.
    let _ = ENGINE.set(Engine { f: unsafe { *funcs }, globals });
}

pub fn engine() -> &'static Engine {
    ENGINE.get().expect("GiveFnptrsToDll not called yet")
}

/// Calls an engine slot, panicking with its name if the engine left it null.
#[macro_export]
macro_rules! eng {
    ($name:ident ( $($arg:expr),* $(,)? )) => {
        // SAFETY: the engine fills its function tables before the first callback and keeps them

        // for the process; slots are called with the argument types bindgen generated from the

        // engine headers, and callers pass valid pointers for those arguments.

        unsafe { ($crate::server::engine().f.$name.expect(stringify!($name)))($($arg),*) }
    };
}

impl Engine {
    pub fn globals(&self) -> &globalvars_t {
        // SAFETY: `globals` points at the engine's globalvars_t, which lives for the process.
        unsafe { &*self.globals }
    }

    /// `pfnCheckVisibility`: is `e` in the PVS `set` passed to `AddToFullPack`?
    pub fn check_visibility(&self, e: *const edict_t, set: *mut u8) -> bool {
        eng!(pfnCheckVisibility(e, set)) != 0
    }

    /// `pfnSetGroupMask`: traces and player movement skip entities whose `groupinfo` fails `op`
    /// (0 = AND, 1 = NAND) against `mask`; `(0, 0)` turns the filter off.
    pub fn set_group_mask(&self, mask: i32, op: i32) {
        eng!(pfnSetGroupMask(mask, op))
    }

    /// The client's physics info string (`pfnGetPhysicsInfoString`).
    pub fn physics_info(&self, e: *const edict_t) -> &'static CStr {
        let p = eng!(pfnGetPhysicsInfoString(e));
        // SAFETY: the engine returns null or a NUL-terminated buffer owned by the client slot.
        unsafe { cstr(p) }
    }

    /// Makes the engine re-run touches for every entity next frame (`gpGlobals->force_retouch++`),
    /// e.g. after a trigger turns solid.
    pub fn force_retouch(&self) {
        // SAFETY: the engine's globals live for the whole server and are only touched from the
        // game thread.
        unsafe { (*self.globals).force_retouch += 1.0 };
    }

    /// Sets `gpGlobals->cdAudioTrack` (worldspawn `sounds`); the engine sends it to connecting
    /// clients as the CD track to loop.
    pub fn set_cd_audio_track(&self, track: i32) {
        // SAFETY: as in `force_retouch`: the globals outlive the server and only the game thread
        // writes them.
        unsafe { (*self.globals).cdAudioTrack = track };
    }

    pub fn time(&self) -> f32 {
        self.globals().time
    }

    pub fn max_clients(&self) -> i32 {
        self.globals().maxClients
    }

    /// Prints to the server console (always shown, unlike `at_console` alerts).
    pub fn print(&self, msg: &str) {
        let s = to_cstring(msg);
        eng!(pfnServerPrint(s.as_ptr()));
    }

    pub fn alert(&self, kind: ALERT_TYPE, msg: &str) {
        let s = to_cstring(msg);
        // Variadic: format in Rust, pass through "%s".
        eng!(pfnAlertMessage(kind, c"%s".as_ptr().cast_mut(), s.as_ptr()));
    }

    /// Engine string pool lookup. `string_t` is an offset into the engine's pool.
    pub fn string(&self, s: string_t) -> &'static CStr {
        let p = eng!(pfnSzFromIndex(s));
        // SAFETY: the engine returns null or a NUL-terminated string it owns; it's used or copied before the next engine call.
        unsafe { cstr(p) }
    }

    /// Copies `s` into the engine string pool.
    pub fn alloc_string(&self, s: &CStr) -> string_t {
        eng!(pfnAllocString(s.as_ptr()))
    }

    /// The engine keeps the pointer, so the name must be `'static`. Guarded (`guard`): on the
    /// Steam engines a name they would stop the server over returns 0 without the call.
    pub fn precache_model(&self, name: &'static CStr) -> i32 {
        self.precache(Kind::Model, name, |n| eng!(pfnPrecacheModel(n.as_ptr())))
    }

    /// True on FWGS (Xash3D), which has the `host_ver` cvar; false on the Steam engines.
    /// FWGS only warns where the Steam engines stop the server, so the guards pass calls through
    /// there, as TS makes them.
    pub fn is_xash(&self) -> bool {
        static XASH: OnceLock<bool> = OnceLock::new();
        *XASH.get_or_init(|| !self.cvar_string(c"host_ver").is_empty())
    }

    /// `PRECACHE_*` through the guard: the Steam engines `Host_Error` on a bad name, a sentence
    /// sound, a new name once the level is active or a full table; the port logs and returns 0
    /// (FWGS's answer for a bad name) instead.
    fn precache(&self, kind: Kind, name: &'static CStr, call: impl FnOnce(&'static CStr) -> i32) -> i32 {
        // A brush model is already in the table if the map has it, on every engine; one it lacks
        // is a `Sys_Error` on the Steam engines ("Mod_ForName: *n not found") and an empty model
        // on FWGS.
        if kind == Kind::Model && guard::is_brush_name(guard::lookup_name(kind, name).to_bytes()) {
            if self.brush_model_ok(guard::lookup_name(kind, name).to_bytes()) {
                return call(name);
            }
            let msg = format!("Precache Model \"{}\": the map has no such brush model; skipped (port guard)\n", name.to_string_lossy());
            self.alert(ALERT_TYPE::at_error, &msg);
            return 0;
        }
        if self.is_xash() {
            return call(name);
        }
        let decision = guard::with(|s| {
            let loading = s.loading;
            let t = s.table(kind);
            t.decide(kind, name, loading).map(|n| (n, t.find(guard::lookup_name(kind, n)).is_none()))
        });
        // A new model is loaded at once with `crash` set (`Mod_ForName`: `Sys_Error` "not found")
        // and a new event script must exist (`Host_Error` "file missing from server").
        let decision = decision.and_then(|(n, new)| {
            let file = guard::lookup_name(kind, n);
            let missing = new && matches!(kind, Kind::Model | Kind::Event) && self.file_size(file).is_some_and(|size| size < 0);
            if missing { Err("file not found") } else { Ok(n) }
        });
        match decision {
            Ok(n) => {
                let i = call(n);
                guard::with(|s| s.table(kind).record(kind, n, i));
                i
            }
            Err(why) => {
                let msg = format!("Precache {kind:?} \"{}\": {why}; skipped (port guard)\n", name.to_string_lossy());
                self.alert(ALERT_TYPE::at_error, &msg);
                0
            }
        }
    }

    /// `pfnGetFileSize` (`COM_FileSize`): the file's size, -1 when the engine's file system
    /// can't open it; `None` if the engine has no such slot.
    pub fn file_size(&self, name: &CStr) -> Option<i32> {
        // SAFETY: an engine slot from its function table, called with a NUL-terminated name.
        self.f.pfnGetFileSize.map(|f| unsafe { f(name.as_ptr()) })
    }

    /// The name to give a Steam engine's `SET_MODEL`/`MODEL_INDEX` for `model`, or `None` when it
    /// would stop the server ("no precache" / "not precached"): an empty name (model 0), the
    /// world, or a name precached this level (in its precached spelling). Brush models are
    /// checked by [`Engine::model_to_set`].
    fn precached_model<'a>(&self, model: &'a CStr) -> Option<&'a CStr> {
        let b = model.to_bytes();
        if b.is_empty() {
            return Some(model);
        }
        if guard::bad_string(b) {
            return None;
        }
        let map = self.string(self.globals().mapname).to_string_lossy();
        if b.eq_ignore_ascii_case(format!("maps/{map}.bsp").as_bytes()) {
            return Some(model);
        }
        guard::with(|s| s.models.resolve(model))
    }

    /// Whether the map has brush model `name` (`*n`): its BSP's models lump is read once per
    /// level (`pfnLoadFileForMe` on `maps/<mapname>.bsp`, as the engine loaded it). A BSP the
    /// port can't read or doesn't know (not version 29/30) lets every `*n` through, as before.
    pub fn brush_model_ok(&self, name: &[u8]) -> bool {
        let map = self.string(self.globals().mapname).to_bytes().to_vec();
        let cached = guard::with(|s| s.brush.as_ref().filter(|(m, _)| *m == map).map(|(_, c)| *c));
        let count = cached.unwrap_or_else(|| {
            let path = format!("maps/{}.bsp", String::from_utf8_lossy(&map));
            let count = self.file_head(&path, guard::BSP_HEADER_MODELS_END).and_then(|h| guard::bsp_model_count(&h));
            guard::with(|s| s.brush = Some((map, count)));
            count
        });
        count.is_none_or(|c| guard::brush_model_known(name, c))
    }

    /// The first `n` bytes (or fewer) of a game file, through `pfnLoadFileForMe`.
    fn file_head(&self, name: &str, n: usize) -> Option<Vec<u8>> {
        let s = to_cstring(name);
        let mut len = 0;
        let p = eng!(pfnLoadFileForMe(s.as_ptr(), &mut len));
        if p.is_null() {
            return None;
        }
        // SAFETY: the engine returned `len` bytes at non-null `p`; at most that many are copied
        // before the file is freed.
        let out = unsafe { std::slice::from_raw_parts(p as *const u8, (len.max(0) as usize).min(n)) }.to_vec();
        eng!(pfnFreeFile(p.cast()));
        Some(out)
    }

    /// What `SET_MODEL`/`MODEL_INDEX` may be given for `model`: a `*n` the map doesn't have stops
    /// the Steam engines ("no precache" `Host_Error`, `SV_ModelIndex` `Sys_Error`) and makes FWGS
    /// register a model with no data, which a `SOLID_BSP` entity then stops it with, so it is
    /// refused on every engine; other names as [`Engine::precached_model`] on the Steam engines.
    fn model_to_set<'a>(&self, model: &'a CStr) -> Result<&'a CStr, &'static str> {
        let b = model.to_bytes();
        if guard::is_brush_name(b) {
            return if self.brush_model_ok(b) { Ok(model) } else { Err("the map has no such brush model") };
        }
        if self.is_xash() {
            return Ok(model);
        }
        self.precached_model(model).ok_or("not precached")
    }

    /// `SET_MODEL`. Guarded ([`Engine::model_to_set`]): a model that would stop the server leaves
    /// the entity as it was, as FWGS does with a bad name.
    pub fn set_model(&self, e: *mut edict_t, model: &CStr) {
        match self.model_to_set(model) {
            Ok(m) => eng!(pfnSetModel(e, m.as_ptr())),
            Err(why) => {
                let msg = format!("SET_MODEL \"{}\": {why}; skipped (port guard)\n", model.to_string_lossy());
                self.alert(ALERT_TYPE::at_error, &msg);
            }
        }
    }

    /// `SET_SIZE`. Guarded on the Steam engines: backwards bounds (a `Host_Error`) leave the size
    /// and only relink, as FWGS does.
    pub fn set_size(&self, e: *mut edict_t, mins: [f32; 3], maxs: [f32; 3]) {
        if !self.is_xash() && (0..3).any(|i| mins[i] > maxs[i]) {
            self.alert(ALERT_TYPE::at_error, "SET_SIZE with backwards mins/maxs; skipped (port guard)\n");
            let org = e.v().origin;
            self.set_origin(e, org);
            return;
        }
        eng!(pfnSetSize(e, mins.as_ptr(), maxs.as_ptr()))
    }

    pub fn set_origin(&self, e: *mut edict_t, org: [f32; 3]) {
        eng!(pfnSetOrigin(e, org.as_ptr()))
    }

    pub fn edict_of_vars(&self, pev: *mut entvars_t) -> *mut edict_t {
        eng!(pfnFindEntityByVars(pev))
    }

    pub fn index_of(&self, e: *const edict_t) -> i32 {
        eng!(pfnIndexOfEdict(e))
    }

    /// Next entity after `start` whose `field` equals `value`; null when there are no more.
    pub fn find_by_string(&self, start: *mut edict_t, field: &CStr, value: &CStr) -> *mut edict_t {
        let e = eng!(pfnFindEntityByString(start, field.as_ptr(), value.as_ptr()));
        // The engine returns the world edict (index 0) for "not found".
        if e.is_null() || self.index_of(e) == 0 { std::ptr::null_mut() } else { e }
    }

    pub fn client_print(&self, e: *mut edict_t, kind: PRINT_TYPE, msg: &str) {
        let s = to_cstring(msg);
        eng!(pfnClientPrintf(e, kind, s.as_ptr()))
    }

    pub fn cmd_argv(&self, i: c_int) -> &'static CStr {
        let p = eng!(pfnCmd_Argv(i));
        // SAFETY: the engine returns null or a NUL-terminated string it owns; it's used or copied before the next engine call.
        unsafe { cstr(p) }
    }
}

// ---- strings the engine keeps --------------------------------------------------------------

/// Returns a `'static` C string for `s`, allocating it once. Use for names the engine stores
/// (precache paths, cvar and command names) when they aren't compile-time literals.
pub fn intern(s: &str) -> &'static CStr {
    use std::collections::HashMap;
    use std::sync::Mutex;
    static POOL: Mutex<Option<HashMap<String, &'static CStr>>> = Mutex::new(None);
    let mut pool = POOL.lock().unwrap();
    let map = pool.get_or_insert_with(HashMap::new);
    if let Some(c) = map.get(s) {
        return c;
    }
    let c: &'static CStr = Box::leak(to_cstring(s).into_boxed_c_str());
    map.insert(s.to_owned(), c);
    c
}

// ---- traces ---------------------------------------------------------------------------------

/// `fNoMonsters` values for traces.
pub const DONT_IGNORE_MONSTERS: c_int = 0;
pub const IGNORE_MONSTERS: c_int = 1;
pub const MISSILE: c_int = 2;

/// Hull numbers for `trace_hull`.
pub const POINT_HULL: c_int = 0;
pub const HUMAN_HULL: c_int = 1;
pub const LARGE_HULL: c_int = 2;
pub const HEAD_HULL: c_int = 3;

impl Engine {
    pub fn trace_line(&self, start: [f32; 3], end: [f32; 3], no_monsters: c_int, skip: *mut edict_t) -> TraceResult {
        let mut tr = TraceResult::default();
        eng!(pfnTraceLine(start.as_ptr(), end.as_ptr(), no_monsters, skip, &mut tr));
        tr
    }

    pub fn trace_hull(&self, start: [f32; 3], end: [f32; 3], no_monsters: c_int, hull: c_int, skip: *mut edict_t) -> TraceResult {
        let mut tr = TraceResult::default();
        eng!(pfnTraceHull(start.as_ptr(), end.as_ptr(), no_monsters, hull, skip, &mut tr));
        tr
    }

    pub fn point_contents(&self, p: [f32; 3]) -> c_int {
        eng!(pfnPointContents(p.as_ptr()))
    }

    /// Texture name hit by a trace against `ent` (usually the world), for material sounds.
    pub fn trace_texture(&self, ent: *mut edict_t, start: [f32; 3], end: [f32; 3]) -> &'static CStr {
        let p = eng!(pfnTraceTexture(ent, start.as_ptr(), end.as_ptr()));
        // SAFETY: the engine returns null or a NUL-terminated string it owns; it's used or copied before the next engine call.
        unsafe { cstr(p) }
    }

    // ---- sounds, events, precache --------------------------------------------------------------

    /// Guarded like [`Engine::precache_model`].
    pub fn precache_sound(&self, name: &'static CStr) -> i32 {
        self.precache(Kind::Sound, name, |n| eng!(pfnPrecacheSound(n.as_ptr())))
    }

    /// Guarded like [`Engine::precache_model`].
    pub fn precache_generic(&self, name: &'static CStr) -> i32 {
        self.precache(Kind::Generic, name, |n| eng!(pfnPrecacheGeneric(n.as_ptr())))
    }

    /// `pfnPrecacheEvent(1, "events/x.sc")`; the `.sc` file must exist in the mod's events/.
    /// Guarded like [`Engine::precache_model`].
    pub fn precache_event(&self, name: &'static CStr) -> u16 {
        self.precache(Kind::Event, name, |n| eng!(pfnPrecacheEvent(1, n.as_ptr())) as i32) as u16
    }

    /// `MODEL_INDEX`. Guarded ([`Engine::model_to_set`]): a model that would stop the server
    /// (ReHLDS `Sys_Error`s in `SV_ModelIndex`) gives 0, FWGS's answer for a bad name.
    pub fn model_index(&self, name: &CStr) -> i32 {
        match self.model_to_set(name) {
            Ok(n) => eng!(pfnModelIndex(n.as_ptr())),
            Err(why) => {
                let msg = format!("MODEL_INDEX \"{}\": {why}; 0 (port guard)\n", name.to_string_lossy());
                self.alert(ALERT_TYPE::at_error, &msg);
                0
            }
        }
    }

    /// `EMIT_SOUND_DYN`. Guarded on the Steam engines: `PF_sound_I` `Sys_Error`s on a volume
    /// outside 0..1 (0..255 without `REHLDS_FIXES`), an attenuation outside 0..4, a channel
    /// outside 0..7 or a pitch outside 0..255. The port clamps them, which is what
    /// `SV_BuildSoundMsg` (and FWGS) does with the values it is then given.
    #[allow(clippy::too_many_arguments)]
    pub fn emit_sound(&self, e: *mut edict_t, channel: i32, sample: &CStr, volume: f32, attn: f32, flags: i32, pitch: i32) {
        let (mut channel, mut volume, mut attn, mut pitch) = (channel, volume, attn, pitch);
        if !self.is_xash() {
            (volume, attn) = (guard::clamp_nan_kept(volume, 1.0), guard::clamp_nan_kept(attn, 4.0));
            (channel, pitch) = (channel.clamp(0, 7), pitch.clamp(0, 255));
        }
        eng!(pfnEmitSound(e, channel, sample.as_ptr(), volume, attn, flags, pitch))
    }

    #[allow(clippy::too_many_arguments)] // mirrors pfnEmitAmbientSound
    pub fn emit_ambient_sound(&self, e: *mut edict_t, pos: [f32; 3], sample: &CStr, volume: f32, attn: f32, flags: i32, pitch: i32) {
        let mut pos = pos;
        eng!(pfnEmitAmbientSound(e, pos.as_mut_ptr(), sample.as_ptr(), volume, attn, flags, pitch))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn playback_event(
        &self,
        flags: i32,
        invoker: *const edict_t,
        index: u16,
        delay: f32,
        origin: [f32; 3],
        angles: [f32; 3],
        f1: f32,
        f2: f32,
        i1: i32,
        i2: i32,
        b1: bool,
        b2: bool,
    ) {
        let (mut o, mut a) = (origin, angles);
        eng!(pfnPlaybackEvent(flags, invoker, index, delay, o.as_mut_ptr(), a.as_mut_ptr(), f1, f2, i1, i2, b1 as c_int, b2 as c_int))
    }

    // ---- entities -------------------------------------------------------------------------------

    /// `CREATE_NAMED_ENTITY`. Guarded: null when the engine has no edict to give (every engine
    /// stops the server in `ED_Alloc` then; see [`Engine::edict_available`]).
    pub fn create_named_entity(&self, classname: &CStr) -> *mut edict_t {
        if !self.edict_available() {
            return std::ptr::null_mut();
        }
        let s = self.alloc_string(classname);
        eng!(pfnCreateNamedEntity(s))
    }

    /// Whether `ED_Alloc` (ReHLDS) / `SV_AllocEdict` (FWGS) can return an edict now instead of
    /// stopping the server with "no free edicts": the engine's edict count is below
    /// `maxEntities` (the last slot was never used: slots are taken in order and start zeroed),
    /// or a non-client slot is free and reusable (freed in the first 2 s of the level or over
    /// 0.5 s ago). Logs when not.
    pub fn edict_available(&self) -> bool {
        let g = self.globals();
        let (max, clients, time) = (g.maxEntities, g.maxClients.max(0), g.time);
        let base = self.entity_of_index(0);
        if base.is_null() || max <= clients + 1 {
            return true;
        }
        // SAFETY: the engines allocate their edict array for `maxEntities` edicts up front
        // (ReHLDS `Hunk_AllocName`, FWGS `Mem_Calloc`) and edict 0 is its start, so every index
        // below `maxEntities` is inside it; only plain fields are read, on the game thread.
        let edict = |i: i32| unsafe { &*base.add(i as usize) };
        let last = edict(max - 1);
        let never_used = last.free == 0 && last.pvPrivateData.is_null() && last.v.classname == 0;
        if never_used || (clients + 1..max).any(|i| guard::reusable(edict(i).free != 0, edict(i).freetime, time)) {
            return true;
        }
        self.alert(ALERT_TYPE::at_error, &format!("No free edicts (max is {max}); entity not created (port guard)\n"));
        false
    }

    pub fn remove_entity(&self, e: *mut edict_t) {
        eng!(pfnRemoveEntity(e))
    }

    pub fn entity_of_index(&self, i: i32) -> *mut edict_t {
        eng!(pfnPEntityOfEntIndex(i))
    }

    /// Edict of player `i` (1..=maxClients), or null if the slot is free.
    pub fn player(&self, i: i32) -> *mut edict_t {
        let e = self.entity_of_index(i);
        if e.is_null() || e.is_free() || !e.has_private_data() { std::ptr::null_mut() } else { e }
    }

    pub fn drop_to_floor(&self, e: *mut edict_t) -> i32 {
        eng!(pfnDropToFloor(e))
    }

    pub fn angle_vectors(&self, angles: [f32; 3]) -> ([f32; 3], [f32; 3], [f32; 3]) {
        let (mut f, mut r, mut u) = ([0.0; 3], [0.0; 3], [0.0; 3]);
        eng!(pfnAngleVectors(angles.as_ptr(), f.as_mut_ptr(), r.as_mut_ptr(), u.as_mut_ptr()));
        (f, r, u)
    }

    pub fn vec_to_angles(&self, v: [f32; 3]) -> [f32; 3] {
        let mut out = [0.0; 3];
        eng!(pfnVecToAngles(v.as_ptr(), out.as_mut_ptr()));
        out
    }

    pub fn random_float(&self, lo: f32, hi: f32) -> f32 {
        eng!(pfnRandomFloat(lo, hi))
    }

    pub fn random_long(&self, lo: i32, hi: i32) -> i32 {
        eng!(pfnRandomLong(lo, hi))
    }

    /// `LIGHT_STYLE`. Guarded: a style outside 0..64 is skipped (FWGS `Host_Error`s past its
    /// limit; the Steam engines write outside their `lightstyles[64]`).
    pub fn light_style(&self, style: i32, value: &'static CStr) {
        if !(0..guard::MAX_LIGHTSTYLES).contains(&style) {
            self.alert(ALERT_TYPE::at_error, &format!("LIGHT_STYLE {style} out of range; skipped (port guard)\n"));
            return;
        }
        eng!(pfnLightStyle(style, value.as_ptr()))
    }

    // ---- clients --------------------------------------------------------------------------------

    pub fn set_client_maxspeed(&self, e: *mut edict_t, speed: f32) {
        eng!(pfnSetClientMaxspeed(e, speed))
    }

    pub fn set_physics_key(&self, e: *mut edict_t, key: &CStr, value: &CStr) {
        eng!(pfnSetPhysicsKeyValue(e, key.as_ptr(), value.as_ptr()))
    }

    pub fn info_key(&self, e: *mut edict_t, key: &CStr) -> &'static CStr {
        let buf = eng!(pfnGetInfoKeyBuffer(e));
        let v = eng!(pfnInfoKeyValue(buf, key.as_ptr()));
        // SAFETY: the engine returns null or a NUL-terminated string it owns; it's used or copied before the next engine call.
        unsafe { cstr(v) }
    }

    pub fn user_id(&self, e: *mut edict_t) -> i32 {
        eng!(pfnGetPlayerUserId(e))
    }

    /// Bots: returns null when no client slot is free.
    pub fn create_fake_client(&self, name: &CStr) -> *mut edict_t {
        eng!(pfnCreateFakeClient(name.as_ptr()))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn run_player_move(&self, e: *mut edict_t, angles: [f32; 3], fwd: f32, side: f32, up: f32, buttons: u16, impulse: u8, msec: u8) {
        eng!(pfnRunPlayerMove(e, angles.as_ptr(), fwd, side, up, buttons, impulse, msec))
    }

    pub fn is_dedicated(&self) -> bool {
        eng!(pfnIsDedicatedServer()) != 0
    }

    pub fn server_command(&self, cmd: &str) {
        let s = to_cstring(cmd);
        eng!(pfnServerCommand(s.as_ptr()))
    }

    pub fn cmd_argc(&self) -> i32 {
        eng!(pfnCmd_Argc())
    }

    pub fn cmd_args(&self) -> &'static CStr {
        let p = eng!(pfnCmd_Args());
        // SAFETY: the engine returns null or a NUL-terminated string it owns; it's used or copied before the next engine call.
        unsafe { cstr(p) }
    }

    // ---- cvars and commands ---------------------------------------------------------------------

    /// Registers a cvar. The engine keeps the pointer, so the `cvar_t` is leaked.
    pub fn register_cvar(&self, name: &str, value: &str, flags: i32) -> &'static cvar_t {
        let c = Box::leak(Box::new(cvar_t {
            name: intern(name).as_ptr().cast_mut(),
            string: intern(value).as_ptr().cast_mut(),
            flags: flags as _,
            value: value.parse().unwrap_or(0.0),
            next: std::ptr::null_mut(),
        }));
        eng!(pfnCVarRegister(c));
        c
    }

    pub fn cvar_float(&self, name: &CStr) -> f32 {
        eng!(pfnCVarGetFloat(name.as_ptr()))
    }

    pub fn cvar_string(&self, name: &CStr) -> &'static CStr {
        let p = eng!(pfnCVarGetString(name.as_ptr()));
        // SAFETY: the engine returns null or a NUL-terminated string it owns; it's used or copied before the next engine call.
        unsafe { cstr(p) }
    }

    pub fn set_cvar_float(&self, name: &CStr, v: f32) {
        eng!(pfnCVarSetFloat(name.as_ptr(), v))
    }

    /// Registers a server console command. `f` is a safe `extern "C" fn()`; it reads its
    /// arguments through [`Engine::cmd_argc`]/[`Engine::cmd_argv`].
    pub fn add_server_command(&self, name: &'static CStr, f: extern "C" fn()) {
        eng!(pfnAddServerCommand(name.as_ptr(), Some(f)))
    }

    /// Registers a user message; its size is kept for [`MsgWriter::end`]'s check.
    pub fn reg_user_msg(&self, name: &'static CStr, size: i32) -> i32 {
        let id = eng!(pfnRegUserMsg(name.as_ptr(), size));
        if id != 0 {
            guard::with(|s| s.user_msgs.insert(id, size));
        }
        id
    }
}

/// Reads a registered cvar's current value.
pub fn cvar_value(c: &cvar_t) -> f32 {
    // The engine updates `value` in place; read it volatilely so the compiler doesn't cache it.
    // SAFETY: `c` is a live reference; the volatile read only stops the compiler caching a value the engine changes.
    unsafe { std::ptr::read_volatile(&c.value) }
}

/// A registered cvar's current string. The engine replaces `string` when the cvar is set, so
/// copy or compare it before the next engine call.
pub fn cvar_string(c: &cvar_t) -> &CStr {
    // SAFETY: `c` is a live reference; the volatile read only stops the compiler caching a pointer the engine replaces.
    let p = unsafe { std::ptr::read_volatile(&c.string) };
    // SAFETY: the engine keeps `string` null or a NUL-terminated string it owns until the cvar is next set.
    unsafe { cstr(p) }
}

// ---- user messages ---------------------------------------------------------------------------------

/// One `WRITE_*` call held by [`MsgWriter`].
enum Op {
    Byte(i32),
    Char(i32),
    Short(i32),
    Long(i32),
    Angle(f32),
    Coord(f32),
    Str(std::ffi::CString),
    Entity(i32),
}

impl Op {
    /// Size the Steam engines give the write (`MSG_WriteCoord` and `WRITE_ENTITY` are shorts).
    fn size(&self) -> usize {
        match self {
            Op::Byte(_) | Op::Char(_) | Op::Angle(_) => 1,
            Op::Short(_) | Op::Coord(_) | Op::Entity(_) => 2,
            Op::Long(_) => 4,
            Op::Str(s) => s.as_bytes().len() + 1,
        }
    }
}

/// Builder for `pfnMessageBegin` ... `pfnMessageEnd`. Call `end()` to send.
///
/// The writes are held until `end()`, which checks the message first (port guard): one that would
/// stop the server (a `MSG_ONE` to a non-client or type 0 anywhere; on the Steam engines also a
/// message too long or of the wrong size for its registration) is dropped and logged, as FWGS
/// drops it. On the Steam engines a broadcast loses its target entity (a `Sys_Error` there), and
/// everywhere a `WRITE_ENTITY` number outside the edict range is written as the short it is (FWGS
/// `Host_Error`s on it).
#[must_use = "call .end() to send the message"]
pub struct MsgWriter {
    dest: c_int,
    msg_type: c_int,
    origin: Option<[f32; 3]>,
    ed: *mut edict_t,
    ops: Vec<Op>,
}

impl MsgWriter {
    pub fn begin(dest: c_int, msg_type: c_int, origin: Option<[f32; 3]>, ed: *mut edict_t) -> Self {
        MsgWriter { dest, msg_type, origin, ed, ops: Vec::new() }
    }
    fn op(mut self, op: Op) -> Self {
        self.ops.push(op);
        self
    }
    pub fn byte(self, v: i32) -> Self {
        self.op(Op::Byte(v))
    }
    pub fn char(self, v: i32) -> Self {
        self.op(Op::Char(v))
    }
    pub fn short(self, v: i32) -> Self {
        self.op(Op::Short(v))
    }
    pub fn long(self, v: i32) -> Self {
        self.op(Op::Long(v))
    }
    pub fn angle(self, v: f32) -> Self {
        self.op(Op::Angle(v))
    }
    pub fn coord(self, v: f32) -> Self {
        self.op(Op::Coord(v))
    }
    pub fn string(self, s: &CStr) -> Self {
        self.op(Op::Str(s.to_owned()))
    }
    pub fn entity(self, v: i32) -> Self {
        self.op(Op::Entity(v))
    }

    fn to_one(&self) -> bool {
        self.dest == MSG_ONE as c_int || self.dest == MSG_ONE_UNRELIABLE as c_int
    }

    /// Why sending the message would stop the server, if it would.
    fn error(&self, eng: &Engine) -> Option<String> {
        if self.msg_type == 0 {
            return Some("message type 0".into());
        }
        if self.to_one() {
            let n = if self.ed.is_null() { 0 } else { eng.index_of(self.ed) };
            if n < 1 || n > eng.max_clients() {
                return Some(format!("MSG_ONE to entity {n}, not a client"));
            }
        }
        if eng.is_xash() {
            return None;
        }
        let user_size = guard::with(|s| s.user_msgs.get(&self.msg_type).copied());
        guard::message_size_error(user_size, self.ops.iter().map(Op::size).sum())
    }

    pub fn end(self) {
        let eng = engine();
        if let Some(why) = self.error(eng) {
            eng.alert(ALERT_TYPE::at_error, &format!("Message {} dropped: {why} (port guard)\n", self.msg_type));
            return;
        }
        let ed = if !self.to_one() && !eng.is_xash() { std::ptr::null_mut() } else { self.ed };
        let o = self.origin.unwrap_or_default();
        let po = if self.origin.is_some() { o.as_ptr() } else { std::ptr::null() };
        let max_entities = eng.globals().maxEntities;
        eng!(pfnMessageBegin(self.dest, self.msg_type, po, ed));
        for op in &self.ops {
            match *op {
                Op::Byte(v) => eng!(pfnWriteByte(v)),
                Op::Char(v) => eng!(pfnWriteChar(v)),
                Op::Short(v) => eng!(pfnWriteShort(v)),
                Op::Long(v) => eng!(pfnWriteLong(v)),
                Op::Angle(v) => eng!(pfnWriteAngle(v)),
                Op::Coord(v) => eng!(pfnWriteCoord(v)),
                Op::Str(ref s) => eng!(pfnWriteString(s.as_ptr())),
                Op::Entity(v) if (0..max_entities).contains(&v) => eng!(pfnWriteEntity(v)),
                // `WRITE_ENTITY` is `MSG_WriteShort` on every engine; FWGS stops on a bad number.
                Op::Entity(v) => eng!(pfnWriteShort(v)),
            }
        }
        eng!(pfnMessageEnd());
    }
}
