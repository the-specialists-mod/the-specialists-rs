//! HUD: message hooks, HUD state, and one immediate-mode drawing pass in TS order. Owner: HUD/UI
//! work package. Reference: decomp docs/client/architecture-hud.md §3-4, §7.
//!
//! TS's `CHud` element list (§4.1) becomes plain functions called from [`redraw`] in the same
//! order: the SDK health/battery/ammo elements TS kept only for bookkeeping draw nothing, then
//! `CHudMessage` (HudText), death notices, text messages, `CHudTSWeapons`, `CHudTSStatus`, say
//! text and `CHudMenu`.

pub mod draw;
mod notice;
pub mod state;
mod status;
pub mod text;
mod weapons;

use crate::input::{self, Cvar};
use draw::{HudSprites, TsSprites};
use goldsrc::Global;
use goldsrc::client::engine;
use goldsrc::ext::hud::Screen;
use shared::protocol::{MESSAGES, Msg};
use state::{Ctx, Effect, Hud};
use std::cell::{Cell, RefCell};
use std::ffi::{c_char, c_int, c_void};

/// Everything the draw pass needs besides the message state.
#[derive(Default)]
pub struct Gfx {
    pub screen: Screen,
    pub sprites: HudSprites,
    pub ts: TsSprites,
    pub loaded: bool,
}

#[derive(Clone, Copy)]
pub struct HudCvars {
    pub drawhistory_time: Cvar,
    pub fastswitch: Cvar,
    pub deathnotice_time: Cvar,
    pub classautokill: Cvar,
    pub takesshots: Cvar,
    pub zoom_sensitivity_ratio: Cvar,
    pub default_fov: Cvar,
    pub capturemouse: Cvar,
    pub saytext_time: Cvar,
    pub centerid: Cvar,
    pub voice_modenable: Cvar,
    pub voice_clientdebug: Cvar,
    pub drawnames: Cvar,
}

static HUD: Global<RefCell<Option<Hud>>> = Global(RefCell::new(None));
static GFX: Global<RefCell<Option<Gfx>>> = Global(RefCell::new(None));
static CVARS: Global<Cell<Option<HudCvars>>> = Global(Cell::new(None));
/// `gHUD.m_flTime` and the last frame time.
static TIME: Global<Cell<(f32, f32)>> = Global(Cell::new((0.0, 0.0)));
/// Smoothed FOV (`m_iFOV`) and the zoom sensitivity (`m_flMouseSensitivity`).
static FOV: Global<Cell<(f32, f32)>> = Global(Cell::new((90.0, 0.0)));
static INTERMISSION: Global<Cell<bool>> = Global(Cell::new(false));
static ATTACK_CONSUMED: Global<Cell<bool>> = Global(Cell::new(false));
static PREV_BITS: Global<Cell<u32>> = Global(Cell::new(0));

/// Borrow the HUD state. Don't call into the engine in ways that re-enter the client inside `f`.
pub fn with<R>(f: impl FnOnce(&mut Hud) -> R) -> R {
    let mut h = HUD.0.borrow_mut();
    f(h.get_or_insert_with(Hud::default))
}

fn gfx<R>(f: impl FnOnce(&mut Gfx) -> R) -> R {
    let mut g = GFX.0.borrow_mut();
    f(g.get_or_insert_with(Gfx::default))
}

pub fn screen() -> Screen {
    gfx(|g| g.screen.clone())
}

pub fn now() -> f32 {
    TIME.0.get().0
}

pub fn cvars() -> HudCvars {
    CVARS.0.get().expect("hud::init")
}

pub fn intermission() -> bool {
    INTERMISSION.0.get()
}

pub fn observer() -> bool {
    with(|h| h.observer)
}

/// `gHUD.m_flMouseSensitivity`: non-zero while zoomed.
pub fn zoom_sensitivity() -> f32 {
    FOV.0.get().1
}

// ---------------------------------------------------------------------------------------------
// Message hooks

macro_rules! hooks {
    ($($v:ident),* $(,)?) => {
        /// One `pfnUserMsgHook` per message, dispatching to [`on_message`].
        fn hook_for(m: Msg) -> goldsrc::sys::pfnUserMsgHook {
            match m {
                $(Msg::$v => {
                    unsafe extern "C" fn f(_n: *const c_char, size: c_int, buf: *mut c_void) -> c_int {
                        let b: &[u8] = if buf.is_null() || size <= 0 { &[] } else { unsafe { std::slice::from_raw_parts(buf.cast(), size as usize) } };
                        on_message(Msg::$v, b);
                        1
                    }
                    Some(f)
                })*
            }
        }
    };
}

hooks!(
    SelAmmo, CurWeapon, Geiger, Health, Damage, Battery, Train, HudText, SayText, TextMsg, WeaponList, ResetHUD, InitHUD, GameTitle, DeathMsg, ScoreInfo, TeamInfo, TeamScore, GameMode, Motd,
    ServerName, AmmoPickup, WeapPickup, ItemPickup, HideWeapon, SetFov, ShowMenu, ScreenShake, ScreenFade, TsFade, AmmoX, TeamNames, VguiMenu, WeaponInfo, ClipInfo, WStatus, ActItems, KFuPower, TsHealth,
    TsArmor, TsState, TsCash, TsSpace, NextStunt, PwUp, PwUpTime, RoundTime, Objective, ShowObj, Wires, SrvSett, TsPAward, TsBTime, TsHVoice, TsFlag, CustomWp, Spectator, TsMBlur, TsMessage, TsSlowMo,
    PTakeDam, TsFrags, PlInfo, TsPSys,
);

fn local_index() -> i32 {
    engine().local_player().map_or(0, |p| p.index)
}

fn third_person() -> bool {
    crate::view::is_third_person()
}

fn on_message(m: Msg, buf: &[u8]) {
    // Effect messages: the render modules own their effect state and parse the raw payload.
    // The engine keeps a single hook per message, so the HUD hook forwards to them.
    match m {
        Msg::TsPSys => crate::effects::on_tspsys(buf),
        Msg::PTakeDam => crate::effects::on_ptakedam(buf),
        Msg::TsFrags => crate::effects::on_tsfrags(buf),
        Msg::TsMBlur => crate::effects::on_tsmblur(buf),
        Msg::TsBTime => crate::effects::on_tsbtime(buf),
        Msg::TsFade => crate::view::on_tsfade(buf),
        _ => {}
    }
    let e = engine();
    if m == Msg::SetFov && e.cvar_float(c"cl_lw") != 0.0 {
        return; // TS ignores SetFOV while client weapons predict the FOV.
    }
    let screen = screen();
    let li = local_index();
    let c = cvars();
    let ctx = Ctx {
        now: now(),
        screen: &screen,
        titles: &text::EngineTitles,
        local_index: li,
        local_name: if li > 0 { e.player_info(li).name } else { String::new() },
        third_person: third_person(),
        thirdperson_switch: input::cvars().cl_thirdpersonswitch.value() == 1.0,
        saytext_time: c.saytext_time.value(),
        deathnotice_time: c.deathnotice_time.value(),
    };
    let mut fx = Vec::new();
    let ok = with(|h| {
        let ok = h.apply(m, buf, &ctx, &mut fx);
        sync_shared(h);
        ok
    });
    if !ok {
        e.print(&format!("[ts] malformed {} message ({} bytes)\n", m.name().to_string_lossy(), buf.len()));
    }
    for f in fx {
        apply_effect(f);
    }
    if m == Msg::DeathMsg {
        if let Some(d) = shared::protocol::hud::DeathMsg::decode(buf) {
            let name = |i: u8| if i > 0 { e.player_info(i as i32).name } else { String::new() };
            let (k, v) = (name(d.killer), name(d.victim));
            let teamkill = d.killer != d.victim && with(|h| {
                h.gamemode != 0 && h.players.get(d.killer as usize).map(|p| p.team) == h.players.get(d.victim as usize).map(|p| p.team)
            });
            e.print(&notice::death_console_line(&k, &v, &d.weapon, d.killer == d.victim || d.killer == 0, teamkill));
        }
    }
}

/// Mirror the values other modules read into `crate::state`.
fn sync_shared(h: &Hud) {
    crate::state::with(|s| {
        s.health = h.health as i32;
        s.dead = h.health == 0;
        s.observer = h.observer;
        s.weapon = h.weapon;
        s.menu_open = h.menu_open || h.menu.displayed;
        for (i, p) in h.players.iter().enumerate().take(s.active_items.len()) {
            s.active_items[i] = p.active_items;
            s.laser_color[i] = p.laser_color;
            if (1..=32).contains(&i) {
                s.spectator[i - 1] = p.observer;
            }
        }
    });
}

fn apply_effect(f: Effect) {
    let e = engine();
    match f {
        Effect::Sound(name) => e.play_sound(name, 1.0),
        Effect::ToggleThirdPerson => crate::state::with(|s| s.thirdperson_toggles += 1),
        Effect::LeaveThirdPerson => crate::state::with(|s| s.thirdperson_leave = true),
        Effect::RestoreThirdPerson => crate::state::with(|s| s.thirdperson_restore = true),
        Effect::OpenBuyMenu => crate::menus::open_buy(),
        Effect::OpenTeamMenu => crate::menus::open_team_menu(),
        Effect::OpenMotd => crate::menus::open_motd(),
        Effect::Fx(fx) => crate::state::with(|s| {
            if s.ts_effects.len() < 256 {
                s.ts_effects.push(fx);
            }
        }),
        Effect::Fade(f) => {
            let st = crate::state::TsFadeState {
                duration: f.duration as f32 / 4096.0,
                hold: f.hold as f32 / 4096.0,
                flags: f.flags,
                rgba: [f.r, f.g, f.b, f.a],
                start: now(),
            };
            crate::state::with(|s| s.ts_fade = Some(st));
        }
        Effect::ConsolePrint(t) => e.print(&t),
        // SDK CHudTextMessage sends HUD_PRINTCENTER to the engine's centre print.
        Effect::CenterPrint(t) => e.center_print(&t),
    }
}

/// Voice manager messages (registered by the server's `CVoiceGameMgr`, not in the TS table).
unsafe extern "C" fn msg_voice_mask(_n: *const c_char, _size: c_int, _buf: *mut c_void) -> c_int {
    1
}

/// `ReqState`: the server asks for the voice state; the SDK answers `VModEnable` and `vban`.
unsafe extern "C" fn msg_req_state(_n: *const c_char, _size: c_int, _buf: *mut c_void) -> c_int {
    let e = engine();
    e.server_cmd(&format!("VModEnable {}", cvars().voice_modenable.value() as i32));
    e.server_cmd("vban 0 0 0 0");
    1
}

/// `Logo` (impulse 99): the animated logo; TS never ships `sprites/%d_logo.spr`, so ignore.
unsafe extern "C" fn msg_logo(_n: *const c_char, _size: c_int, _buf: *mut c_void) -> c_int {
    1
}

// ---------------------------------------------------------------------------------------------
// Init

fn register_cvars() -> HudCvars {
    let e = engine();
    HudCvars {
        voice_modenable: Cvar::from_raw(e.register_variable(c"voice_modenable", c"1", 1)),
        voice_clientdebug: Cvar::from_raw(e.register_variable(c"voice_clientdebug", c"0", 0)),
        drawhistory_time: Cvar::from_raw(e.register_variable(c"hud_drawhistory_time", c"5", 0)),
        fastswitch: Cvar::from_raw(e.register_variable(c"hud_fastswitch", c"0", 1)),
        deathnotice_time: Cvar::from_raw(e.register_variable(c"hud_deathnotice_time", c"6", 0)),
        classautokill: Cvar::from_raw(e.register_variable(c"hud_classautokill", c"1", 3)),
        takesshots: Cvar::from_raw(e.register_variable(c"hud_takesshots", c"0", 1)),
        zoom_sensitivity_ratio: Cvar::from_raw(e.register_variable(c"zoom_sensitivity_ratio", c"1.2", 0)),
        // TS registers "80" then forces the value to 80 (overriding configs); the port keeps the
        // user's value and defaults to 90 like ts-patcher (docs/design.md §10).
        default_fov: Cvar::from_raw(e.register_variable(c"default_fov", c"90", 1)),
        capturemouse: Cvar::from_raw(e.register_variable(c"hud_capturemouse", c"1", 1)),
        saytext_time: Cvar::from_raw(e.register_variable(c"hud_saytext_time", c"5", 0)),
        centerid: Cvar::from_raw(e.register_variable(c"hud_centerid", c"0", 1)),
        drawnames: Cvar::from_raw(e.register_variable(c"cl_drawnames", c"1", 1)),
    }
}

macro_rules! cmd {
    ($name:literal, $body:expr) => {{
        unsafe extern "C" fn f() {
            $body
        }
        engine().add_command($name, f);
    }};
}

fn register_commands() {
    cmd!(c"cancelselect", with(|h| h.menu_open = false));
    cmd!(c"special", engine().server_cmd("_special"));
    cmd!(c"slot1", select_slot(1));
    cmd!(c"slot2", select_slot(2));
    cmd!(c"slot3", select_slot(3));
    cmd!(c"slot4", select_slot(4));
    cmd!(c"slot5", select_slot(5));
    cmd!(c"slot6", select_slot(6));
    cmd!(c"slot7", select_slot(7));
    cmd!(c"slot8", select_slot(8));
    cmd!(c"slot9", select_slot(9));
    cmd!(c"slot10", select_slot(10));
    cmd!(c"invnext", select_next_prev(true));
    cmd!(c"invprev", select_next_prev(false));
    cmd!(c"buy", cmd_buy());
    cmd!(c"ts_hud_selftest", selftest());
}

/// `ts_hud_selftest`: feed a set of encoded TS messages through the real hooks so the HUD can be
/// checked on a server that doesn't send them yet (development aid; not in TS).
fn selftest() {
    use shared::protocol::hud as p;
    let li = local_index().max(1) as u8;
    // `ts_hud_selftest dead`: just report health 0 (the buy menu opens only when dead).
    if engine().cmd_argv(1) == c"dead" {
        on_message(Msg::TsHealth, &p::TsHealth { hp: 0 }.encode());
        return;
    }
    let msgs: Vec<(Msg, Vec<u8>)> = vec![
        (Msg::TsHealth, p::TsHealth { hp: 73 }.encode()),
        (Msg::TsArmor, p::TsArmor { armor: 40 }.encode()),
        (Msg::TsCash, p::TsCash { cash: 1200 }.encode()),
        (Msg::TsSpace, p::TsSpace { slots: 31 }.encode()),
        (Msg::TsState, p::TsState { stance: 1 }.encode()),
        (Msg::SrvSett, p::SrvSett { key: 1, value: 1 }.encode()),
        (Msg::WStatus, p::WStatus { id: 1, owned: 1, clip: 17 }.encode()),
        (Msg::WStatus, p::WStatus { id: 25, owned: 1, clip: 5 }.encode()),
        (Msg::WeaponInfo, p::WeaponInfo { id: 13, clip: 24, reserve: 90, mode: 0, attachments: 8 }.encode()),
        (Msg::PwUp, p::PwUp { flags: 1, value: 3 }.encode()),
        (Msg::PwUp, p::PwUp { flags: 4, value: 0 }.encode()),
        (Msg::PwUpTime, p::PwUpTime { secs: 25 }.encode()),
        (Msg::RoundTime, p::RoundTime { secs: 200, mode: 0 }.encode()),
        (Msg::KFuPower, p::KFuPower { power: 60 }.encode()),
        (Msg::TsMessage, p::TsMessage { r: 0, g: 0, b: 0, flags: 1, time: 100, text: "#HUD_Credits selftest (top)".into() }.encode()),
        (Msg::TsMessage, p::TsMessage { r: 255, g: 200, b: 0, flags: 8, time: 100, text: "TSMessage bottom".into() }.encode()),
        (Msg::ScoreInfo, p::ScoreInfo { client: li, frags: 7, deaths: 2, class: 0, team: 0 }.encode()),
        (Msg::TsPAward, p::TsPAward { client: li, award: 10 }.encode()),
        (Msg::DeathMsg, p::DeathMsg { killer: li, victim: li, weapon: "skull".into() }.encode()),
        (Msg::SayText, p::SayText { client: li, text: "\u{2}selftest: hello from SayText\n".into() }.encode()),
    ];
    for (m, b) in msgs {
        on_message(m, &b);
    }
}

pub fn init() {
    CVARS.0.set(Some(register_cvars()));
    let e = engine();
    for (m, name, _) in MESSAGES {
        // The engine parses these two itself (FWGS cl_parse.c; GoldSrc likewise); TS doesn't
        // hook them either.
        if matches!(m, Msg::ScreenShake | Msg::ScreenFade) {
            continue;
        }
        e.hook_user_msg(name, hook_for(*m));
    }
    e.hook_user_msg(c"VoiceMask", Some(msg_voice_mask));
    e.hook_user_msg(c"ReqState", Some(msg_req_state));
    e.hook_user_msg(c"Logo", Some(msg_logo));
    register_commands();
    with(|h| *h = Hud::default());
}

pub fn vid_init() {
    let e = engine();
    let s = e.screen();
    let first = gfx(|g| !g.loaded);
    let sprites = HudSprites::load(s.width);
    let ts = TsSprites::load();
    gfx(|g| {
        g.screen = s;
        g.sprites = sprites;
        g.ts = ts;
        g.loaded = true;
    });
    crate::state::with(|st| {
        st.time_scale = 1.0;
        st.time_scale_target = 1.0;
    });
    with(|h| {
        h.reset();
        // CHudTSStatus::VidInit also clears messages, objectives and flags.
        h.messages = vec![state::TsLine::default(); state::MAX_TS_MESSAGES];
        h.objectives = vec![state::Objective::default(); state::MAX_OBJECTIVES];
        h.obj_max = 0;
        h.flags = Default::default();
        h.flag_count = 0;
        h.observer = false;
        h.kfu_power = 100.0;
        h.kfu_prev = 100.0;
    });
    crate::menus::vid_init(first);
}

/// `HUD_Reset` (TS calls `CHud::VidInit` again).
pub fn reset() {
    vid_init();
}

// ---------------------------------------------------------------------------------------------
// Per-frame logic

/// `HUD_Frame`: TS eases `g_flTimescale` toward the `TSSlowMo` target (10055ae0):
/// `cur += min(dt*7, 1) * (target - cur)` while the difference exceeds 0.05, else snap.
pub fn approach_timescale(dt: f32, cur: f32, target: f32) -> f32 {
    if (target - cur).abs() > 0.05 { cur + (dt * 7.0).min(1.0) * (target - cur) } else { target }
}

static LAST_FRAME: Global<Cell<f64>> = Global(Cell::new(0.0));

pub fn frame(time: f64) {
    // Engines pass frame time (FWGS: host.frametime) to HUD_Frame.
    let dt = if time > 0.0 && time < 1.0 { time as f32 } else { 0.0 };
    LAST_FRAME.0.set(time);
    crate::state::with(|s| s.time_scale = approach_timescale(dt, s.time_scale, s.time_scale_target));
}

/// Smooth zoom (`CHud::Think` 10049760 / `UTIL_ApproachFOV` 10049620) and zoom sensitivity.
fn think_fov(dt: f32) {
    let c = cvars();
    let df = c.default_fov.value();
    let default = if df < 0.001 { 80.0 } else { df.clamp(70.0, 110.0) };
    let target = with(|h| if h.fov == 0 { default } else { h.fov as f32 });
    let (mut cur, _) = FOV.0.get();
    if cur < 0.001 {
        cur = target;
    } else if cur != target {
        let step = ((target - cur).abs() * (dt * 12.0).min(1.0)).max(dt * 60.0);
        cur = if cur < target { (cur + step).min(target) } else { (cur - step).max(target) };
    }
    let sens = if (target - default).abs() >= 0.01 {
        target / default * input::cvars().sensitivity.value() * c.zoom_sensitivity_ratio.value()
    } else {
        0.0
    };
    FOV.0.set((cur, sens));
    crate::state::with(|s| s.fov = cur);
}

/// The TS weapon element's `Think` (1004ce10), the parts that need no key bits: laser colour,
/// kung-fu regeneration, heartbeat/breathing sounds, fire-mode switch sound.
fn think(dt: f32) {
    let e = engine();
    let ic = input::cvars();
    let laser = ic.cl_lasercolor.value() as i32;
    let ts = crate::state::with(|s| s.time_scale.max(0.01));
    let (send_laser, firemode_changed) = with(|h| {
        let send = h.last_laser_color != laser;
        h.last_laser_color = laser;
        h.next_stunt = (h.next_stunt - dt / ts).max(0.0);
        h.regen_kungfu(dt / ts);
        let fm = h.weapon > 0 && h.last_firemode != h.firemode;
        if fm {
            h.last_firemode = h.firemode;
        }
        (send, fm)
    });
    if send_laser {
        e.server_cmd(&format!("lasclr_{laser}"));
    }
    if firemode_changed && !dead_or_observer() {
        e.play_sound("weapons/switch.wav", e.client_time().fract() * 0.08 + 0.92);
    }
    weapons::think_sounds(dt);
}

pub fn dead_or_observer() -> bool {
    let demo = engine().demo_playing() || engine().demo_recording();
    with(|h| (!demo && h.health == 0) || h.observer)
}

/// Called from `CL_CreateMove` with this frame's button bits: the weapon menu takes `+attack` as
/// select (`weapon_%i`) and `+attack2` as drop (`drop_%i`); `+use` triggers `usepowerup` when
/// `cl_usepowerup` is on. Returns the bits the HUD consumed.
pub fn weapon_menu_buttons(bits: u32) -> u32 {
    use goldsrc::sys::{IN_ATTACK, IN_ATTACK2, IN_USE};
    let prev = PREV_BITS.0.replace(bits);
    let pressed = bits & !prev;
    let e = engine();
    let t = now();
    // cl_usepowerup: +use with a stored power-up sends `usepowerup` (0.3 s debounce).
    if input::cvars().cl_usepowerup.value() != 0.0 && pressed & IN_USE != 0 {
        let send = with(|h| {
            if h.pw_flags == 0 {
                return false;
            }
            let ok = h.usepowerup_time <= t;
            h.usepowerup_time = t + 0.3;
            ok
        });
        if send {
            e.server_cmd("usepowerup");
        }
    }
    let mut consumed = 0;
    ATTACK_CONSUMED.0.set(false);
    if dead_or_observer() || crate::menus::buy_open() {
        return 0;
    }
    let cmd = with(|h| {
        if !h.menu_open {
            return None;
        }
        let slot = h.grid.get(h.menu_row as usize).and_then(|r| r.get(h.menu_bucket as usize)).copied();
        let id = slot.map_or(-1, |s| s.id);
        if bits & IN_ATTACK != 0 {
            consumed |= IN_ATTACK;
            h.menu_open = false;
            return (id >= 0).then(|| format!("weapon_{id}"));
        }
        if bits & IN_ATTACK2 != 0 {
            consumed |= IN_ATTACK2;
            h.menu_open = false;
            return (id >= 0).then(|| format!("drop_{id}"));
        }
        None
    });
    if let Some(c) = cmd {
        e.server_cmd(&c);
    }
    if consumed & IN_ATTACK != 0 {
        ATTACK_CONSUMED.0.set(true);
    }
    consumed
}

pub fn weapon_menu_consumed_attack() -> bool {
    ATTACK_CONSUMED.0.get()
}

/// `slot1..10` → `TSWeapons::SelectSlot` (1004f220).
pub fn select_slot(n: i32) {
    // An open CHudMenu takes the key first.
    if (1..=10).contains(&n) {
        if let Some(cmd) = with(|h| h.menu_select(n)) {
            engine().server_cmd(cmd.trim_end());
            return;
        }
    }
    if crate::menus::buy_open() {
        crate::menus::buy_key(n);
        return;
    }
    if crate::menus::team_menu_key(n) {
        return;
    }
    if dead_or_observer() {
        return;
    }
    let fast = cvars().fastswitch.value() != 0.0;
    if let Some(cmd) = with(|h| weapons::select_slot(h, n, fast)) {
        engine().server_cmd(&cmd);
    }
}

/// `invnext`/`invprev` (1004fce0/1004fd40): 0.1 s debounce.
fn select_next_prev(next: bool) {
    if dead_or_observer() {
        return;
    }
    let t = now();
    let fast = cvars().fastswitch.value() != 0.0;
    let cmd = with(|h| {
        if h.invnext_time > t {
            return None;
        }
        h.invnext_time = t + 0.1;
        weapons::select_next_prev(h, next, fast)
    });
    if let Some(c) = cmd {
        engine().server_cmd(&c);
    }
}

/// `buy` (1004fda0): only when dead or observing, and not while the weapon menu is open.
fn cmd_buy() {
    if !dead_or_observer() {
        return;
    }
    if with(|h| h.menu_open) {
        return;
    }
    crate::menus::open_buy();
}

// ---------------------------------------------------------------------------------------------
// Drawing

/// Per-frame draw context.
pub struct Dc<'a> {
    pub g: &'a Gfx,
    pub now: f32,
    pub hud: [i32; 3],
}

impl Dc<'_> {
    pub fn w(&self) -> i32 {
        self.g.screen.width
    }
    pub fn h(&self) -> i32 {
        self.g.screen.height
    }
    pub fn string(&self, x: i32, y: i32, t: &str, rgb: [i32; 3]) -> i32 {
        text::draw_string(&self.g.screen, x, y, self.g.screen.width, t, rgb)
    }
    pub fn string_len(&self, t: &str) -> i32 {
        text::string_len(&self.g.screen, t)
    }
    pub fn number(&self, x: i32, y: i32, flags: u8, n: i32, rgb: [i32; 3]) -> i32 {
        draw::draw_number(&self.g.sprites, x, y, flags, n, rgb)
    }
    pub fn fill(&self, x: i32, y: i32, w: i32, h: i32, rgb: [i32; 3], a: i32) {
        engine().fill(x, y, w, h, rgb, a);
    }
    pub fn spr(&self, h: goldsrc::sys::HSPRITE, x: i32, y: i32, rgb: [i32; 3]) {
        if h != 0 {
            let e = engine();
            e.spr_set(h, rgb);
            e.spr_draw_additive(0, x, y, None);
        }
    }
}

/// Run `f` with a draw context (for `menus`, which draws after the HUD).
pub fn with_dc(f: impl FnOnce(&Dc)) {
    let g = GFX.0.borrow();
    let Some(g) = g.as_ref() else { return };
    if !g.loaded {
        return;
    }
    let dc = Dc { g, now: now(), hud: input::hud_rgb() };
    f(&dc);
}

pub fn redraw(time: f32, intermission: bool) {
    let e = engine();
    let (old, _) = TIME.0.get();
    let dt = (time - old).clamp(0.0, 1.0);
    TIME.0.set((time, dt));

    // Intermission: TS hides VGUI menus and shows the scoreboard (and `snapshot` with
    // hud_takesshots); on leaving it hides the scoreboard.
    let was = INTERMISSION.0.replace(intermission);
    if intermission != was {
        crate::menus::intermission(intermission);
        if intermission && cvars().takesshots.value() != 0.0 {
            e.client_cmd("snapshot\n");
        }
    }
    with(|h| h.intermission = intermission);

    let s = e.screen();
    gfx(|g| {
        if s.width != g.screen.width || s.height != g.screen.height {
            g.sprites = HudSprites::load(s.width);
        }
        g.screen = s;
    });
    think_fov(dt);
    think(dt);
    with(|h| sync_shared(h));
    if intermission {
        return;
    }
    let g = GFX.0.borrow();
    let Some(g) = g.as_ref() else { return };
    let dc = Dc { g, now: time, hud: input::hud_rgb() };
    let nohud = input::cvars().nohud.value() != 0.0;

    // TS element order (§4.1): CHudMessage, death notices, CHudTSWeapons, CHudTSStatus,
    // say text, CHudMenu. Centre text (TextMsg HUD_PRINTCENTER) is engine-drawn in TS; drawn
    // with the HudText pass here.
    notice::draw_hud_text(&dc);
    if !nohud {
        notice::draw_death_notices(&dc);
    }
    weapons::draw(&dc, nohud);
    status::draw(&dc, nohud);
    notice::draw_say_text(&dc);
    notice::draw_menu(&dc);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timescale_easing() {
        assert_eq!(approach_timescale(0.016, 1.0, 1.0), 1.0);
        let v = approach_timescale(0.1, 1.0, 0.35);
        assert!((v - (1.0 + 0.7 * (0.35 - 1.0))).abs() < 1e-6);
        assert_eq!(approach_timescale(0.1, 0.38, 0.35), 0.35, "snaps within 0.05");
        assert!((approach_timescale(1.0, 1.0, 0.35) - 0.35).abs() < 1e-6, "rate capped at 1");
    }

    #[test]
    fn every_message_has_a_hook() {
        for (m, _, _) in MESSAGES {
            assert!(hook_for(*m).is_some());
        }
    }
}
