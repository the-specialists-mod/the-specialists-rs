//! The plugin API table (`ts_plugin_api::PluginApi`) that `CreateInterface` hands out: a port
//! addition. Each function converts its arguments and calls `ts_server::plugin_api` or, for the
//! entity hooks, `ts_server::ham`, where the logic is (docs/implementation/plugin-api.md). Calling
//! the plugin's hook is FFI, so it is here too ([`call_hook`]).

use goldsrc::Global;
use goldsrc::sys::{KeyValueData, TraceResult};
use std::cell::Cell;
use std::ffi::{CStr, c_char, c_int};
use ts_plugin_api::{HamCall, INVALID, NOT_PLAYER, OK, PlayerInfo, PluginApi, VERSION, ham};
use ts_server::ham as hooks;
use ts_server::plugin_api as api;

/// The one table. Plugins only read it.
pub static PLUGIN_API: PluginApi = PluginApi {
    size: size_of::<PluginApi>() as u32,
    version: VERSION,
    player_info,
    set_cash,
    set_free_slots,
    set_award,
    run_powerup,
    set_speed,
    set_physics_speed,
    set_bullet_trail,
    set_weapon_ammo,
    projectile_owner,
    ham_set_hook,
    ham_watch,
    ham_supported,
    class_id,
    entity_class_id,
    ham_execute,
};

/// Guard: a NULL `out` gives `NOT_PLAYER` (the contract asks for a valid pointer); nothing is
/// written on failure.
unsafe extern "C" fn player_info(player: c_int, out: *mut PlayerInfo) -> c_int {
    if out.is_null() {
        return NOT_PLAYER;
    }
    match api::player_info(player) {
        Ok(info) => {
            // SAFETY: the caller passes a `PlayerInfo` valid for writes (the table's contract);
            // `write_unaligned` doesn't rely on its alignment.
            unsafe { out.write_unaligned(info) };
            OK
        }
        Err(code) => code,
    }
}

extern "C" fn set_cash(player: c_int, cash: c_int) -> c_int {
    api::set_cash(player, cash)
}

extern "C" fn set_free_slots(player: c_int, slots: c_int) -> c_int {
    api::set_free_slots(player, slots)
}

extern "C" fn set_award(player: c_int, award: c_int) -> c_int {
    api::set_award(player, award)
}

extern "C" fn run_powerup(player: c_int, powerup: c_int, duration: f32) -> c_int {
    api::run_powerup(player, powerup, duration)
}

extern "C" fn set_speed(player: c_int, speed: f32, aura: f32, duration: f32) -> c_int {
    api::set_speed(player, speed, aura, duration)
}

extern "C" fn set_physics_speed(player: c_int, speed: f32) -> c_int {
    api::set_physics_speed(player, speed)
}

extern "C" fn set_bullet_trail(player: c_int, on: c_int) -> c_int {
    api::set_bullet_trail(player, on)
}

extern "C" fn set_weapon_ammo(player: c_int, weapon: c_int, clip: c_int, ammo: c_int) -> c_int {
    api::set_weapon_ammo(player, weapon, clip, ammo)
}

extern "C" fn projectile_owner(entity: c_int) -> c_int {
    api::projectile_owner(entity)
}

// ---- version 2: entity hooks --------------------------------------------------------------------

/// The plugin's hook, as `ham_set_hook` installed it.
static HOOK: Global<Cell<Option<ham::Hook>>> = Global(Cell::new(None));

extern "C" fn ham_set_hook(hook: Option<ham::Hook>) {
    HOOK.0.set(hook);
    hooks::set_hook(hook.map(|_| call_hook as hooks::HookFn));
}

/// The plugin's hook as `ts_server::ham` calls it.
fn call_hook(call: &mut HamCall, post: bool) -> c_int {
    let Some(hook) = HOOK.0.get() else { return ham::action::CONTINUE };
    // SAFETY: `hook` is what the plugin installed with `ham_set_hook`, callable on the engine's
    // thread until it removes it (the table's contract); `call` is an exclusive `HamCall` that
    // lives for the call. The game holds no borrow of entity state across it (`ts_server::ham`'s
    // module docs), so the hook may call back into the table.
    unsafe { hook(call, post as c_int) }
}

extern "C" fn ham_watch(ham: c_int, on: c_int) -> c_int {
    hooks::watch(ham, on != 0)
}

extern "C" fn ham_supported(ham: c_int, class: c_int) -> c_int {
    hooks::supported(ham, class)
}

/// Guard: a NULL name is `INVALID`.
unsafe extern "C" fn class_id(classname: *const c_char) -> c_int {
    if classname.is_null() {
        return INVALID;
    }
    // SAFETY: a non-NULL name is the caller's NUL-terminated string, read for this call only.
    hooks::class_id(unsafe { CStr::from_ptr(classname) })
}

extern "C" fn entity_class_id(entity: c_int) -> c_int {
    hooks::entity_class_id(entity)
}

/// The call is copied in and only `ret` is written back, so a plugin passing the `HamCall` of the
/// hook it is in (or nesting another `ham_execute`) never aliases a live reference. The pointer
/// arguments are read here: a `TRACE` argument's `TraceResult` (copied in and back) and
/// `Ham_Keyvalue`'s `KeyValueData` (its strings read, `fHandled` written back). Guard: a NULL
/// `call` is `INVALID`, and so is a NULL `TraceResult` / `KeyValueData` / key / value.
unsafe extern "C" fn ham_execute(call: *mut HamCall, run_hooks: c_int) -> c_int {
    if call.is_null() {
        return INVALID;
    }
    // SAFETY: the caller passes a `HamCall` valid for reads and writes for this call (the table's
    // contract); `read_unaligned` doesn't rely on its alignment.
    let mut c = unsafe { call.read_unaligned() };
    let argc = usize::try_from(c.argc).unwrap_or(0).min(ham::MAX_ARGS);
    let trace_ptr =
        c.args[..argc].iter().find(|a| a.kind == ham::kind::TRACE).map(|a| a.p.cast_mut().cast::<TraceResult>()).filter(|p| !p.is_null());
    // SAFETY: a `TRACE` argument points to a `TraceResult` valid for the call (contract).
    let mut trace = trace_ptr.map(|p| unsafe { p.read_unaligned() });
    let kvd_ptr = (c.ham == hooks::id::KEYVALUE as c_int && argc >= 1)
        .then(|| c.args[0].p.cast_mut().cast::<KeyValueData>())
        .filter(|p| !p.is_null());
    // SAFETY: `Ham_Keyvalue`'s argument points to a `KeyValueData` valid for the call (contract).
    let kvd = kvd_ptr.map(|p| unsafe { p.read_unaligned() });
    let mut handled = kvd.map_or(0, |k| k.fHandled);
    let strings = kvd.and_then(|k| {
        let ok = !k.szKeyName.is_null() && !k.szValue.is_null();
        // SAFETY: its non-NULL strings are NUL-terminated and live for the call (contract).
        ok.then(|| unsafe {
            let class = (!k.szClassName.is_null()).then(|| CStr::from_ptr(k.szClassName));
            (class, CStr::from_ptr(k.szKeyName), CStr::from_ptr(k.szValue))
        })
    });
    let r = {
        let mut ptrs = hooks::Pointers {
            trace: trace.as_mut(),
            kvd: strings.map(|(class, key, value)| hooks::KeyValue { class, key, value, handled: &mut handled }),
        };
        hooks::execute(&mut c, &mut ptrs, run_hooks != 0)
    };
    if let (Some(p), Some(t)) = (trace_ptr, trace) {
        // SAFETY: as when it was read.
        unsafe { p.write_unaligned(t) };
    }
    if let Some(p) = kvd_ptr.filter(|_| strings.is_some()) {
        // SAFETY: as when it was read; only `fHandled` is written.
        unsafe { (&raw mut (*p).fHandled).write_unaligned(handled) };
    }
    // SAFETY: as when it was read.
    unsafe { (&raw mut (*call).ret).write_unaligned(c.ret) };
    r
}
