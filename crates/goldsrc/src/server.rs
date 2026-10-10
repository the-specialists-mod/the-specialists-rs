//! Server-side engine handle (`enginefuncs_t` + `globalvars_t`) and helpers.

use crate::edict::EdictPtr;
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

    /// The engine keeps the pointer, so the name must be `'static`.
    pub fn precache_model(&self, name: &'static CStr) -> i32 {
        eng!(pfnPrecacheModel(name.as_ptr()))
    }

    pub fn set_model(&self, e: *mut edict_t, model: &CStr) {
        eng!(pfnSetModel(e, model.as_ptr()))
    }

    pub fn set_size(&self, e: *mut edict_t, mins: [f32; 3], maxs: [f32; 3]) {
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

    pub fn precache_sound(&self, name: &'static CStr) -> i32 {
        eng!(pfnPrecacheSound(name.as_ptr()))
    }

    pub fn precache_generic(&self, name: &'static CStr) -> i32 {
        eng!(pfnPrecacheGeneric(name.as_ptr()))
    }

    /// `pfnPrecacheEvent(1, "events/x.sc")`; the `.sc` file must exist in the mod's events/.
    pub fn precache_event(&self, name: &'static CStr) -> u16 {
        eng!(pfnPrecacheEvent(1, name.as_ptr()))
    }

    pub fn model_index(&self, name: &CStr) -> i32 {
        eng!(pfnModelIndex(name.as_ptr()))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn emit_sound(&self, e: *mut edict_t, channel: i32, sample: &CStr, volume: f32, attn: f32, flags: i32, pitch: i32) {
        eng!(pfnEmitSound(e, channel, sample.as_ptr(), volume, attn, flags, pitch))
    }

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

    pub fn create_named_entity(&self, classname: &CStr) -> *mut edict_t {
        let s = self.alloc_string(classname);
        eng!(pfnCreateNamedEntity(s))
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

    pub fn light_style(&self, style: i32, value: &'static CStr) {
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

    pub fn reg_user_msg(&self, name: &'static CStr, size: i32) -> i32 {
        eng!(pfnRegUserMsg(name.as_ptr(), size))
    }
}

/// Reads a registered cvar's current value.
pub fn cvar_value(c: &cvar_t) -> f32 {
    // The engine updates `value` in place; read it volatilely so the compiler doesn't cache it.
    // SAFETY: `c` is a live reference; the volatile read only stops the compiler caching a value the engine changes.
    unsafe { std::ptr::read_volatile(&c.value) }
}

// ---- user messages ---------------------------------------------------------------------------------

/// Builder for `pfnMessageBegin` ... `pfnMessageEnd`. Call `end()` to send.
#[must_use = "call .end() to send the message"]
pub struct MsgWriter(());

impl MsgWriter {
    pub fn begin(dest: c_int, msg_type: c_int, origin: Option<[f32; 3]>, ed: *mut edict_t) -> Self {
        let o = origin.unwrap_or_default();
        let po = if origin.is_some() { o.as_ptr() } else { std::ptr::null() };
        eng!(pfnMessageBegin(dest, msg_type, po, ed));
        MsgWriter(())
    }
    pub fn byte(self, v: i32) -> Self {
        eng!(pfnWriteByte(v));
        self
    }
    pub fn char(self, v: i32) -> Self {
        eng!(pfnWriteChar(v));
        self
    }
    pub fn short(self, v: i32) -> Self {
        eng!(pfnWriteShort(v));
        self
    }
    pub fn long(self, v: i32) -> Self {
        eng!(pfnWriteLong(v));
        self
    }
    pub fn angle(self, v: f32) -> Self {
        eng!(pfnWriteAngle(v));
        self
    }
    pub fn coord(self, v: f32) -> Self {
        eng!(pfnWriteCoord(v));
        self
    }
    pub fn string(self, s: &CStr) -> Self {
        eng!(pfnWriteString(s.as_ptr()));
        self
    }
    pub fn entity(self, v: i32) -> Self {
        eng!(pfnWriteEntity(v));
        self
    }
    pub fn end(self) {
        eng!(pfnMessageEnd());
    }
}
