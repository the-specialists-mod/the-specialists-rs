//! HUD: message hooks, HUD state, and one immediate-mode drawing pass in TS order. Owner: HUD/UI
//! work package. Reference: decomp docs/client/architecture-hud.md §3-4, §7.
//!
//! TS's `CHud` element list (§4.1) becomes plain functions called from [`redraw`] in the same
//! order: the SDK health/battery/ammo elements TS kept only for bookkeeping draw nothing, then
//! `CHudMessage` (HudText), death notices, text messages, `CHudTSWeapons`, `CHudTSStatus`, say
//! text and `CHudMenu`.

pub mod draw;
mod icons;
mod notice;
pub mod saytext;
pub mod state;
mod status;
pub mod text;
pub mod vgui;
mod weapons;

use crate::input::{self, Cvar};
use draw::{HudSprites, TsSprites};
use goldsrc::Global;
use goldsrc::client::engine;
use goldsrc::ext::hud::Screen;
use shared::protocol::{MESSAGES, Msg};
use state::{Ctx, Effect, Hud};
use std::cell::{Cell, RefCell};

/// Everything the draw pass needs besides the message state.
#[derive(Default)]
pub struct Gfx {
    pub screen: Screen,
    pub sprites: HudSprites,
    pub ts: TsSprites,
    /// `CHudTrain::m_hSprite`: `sprites/%d_train.spr` (the stock Half-Life sprite).
    pub train: goldsrc::sys::HSPRITE,
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
    pub drawnames: Cvar,
}

static HUD: Global<RefCell<Option<Hud>>> = Global(RefCell::new(None));
static GFX: Global<RefCell<Option<Gfx>>> = Global(RefCell::new(None));
static CVARS: Global<Cell<Option<HudCvars>>> = Global(Cell::new(None));
/// `gHUD.m_flTime` and the last frame time.
static TIME: Global<Cell<(f32, f32)>> = Global(Cell::new((0.0, 0.0)));
/// The zoom sensitivity (`m_flMouseSensitivity`, `CHud+0x18`). The eased FOV (`m_iFOV`) is
/// `state.fov`.
static ZOOM_SENS: Global<Cell<f32>> = Global(Cell::new(0.0));
static INTERMISSION: Global<Cell<bool>> = Global(Cell::new(false));
static ATTACK_CONSUMED: Global<Cell<bool>> = Global(Cell::new(false));
static PREV_BITS: Global<Cell<u32>> = Global(Cell::new(0));

/// Borrow the HUD state. Don't call into the engine in ways that re-enter the client inside `f`.
pub fn with<R>(f: impl FnOnce(&mut Hud) -> R) -> R {
    let r = {
        let mut h = HUD.0.borrow_mut();
        f(h.get_or_insert_with(Hud::default))
    };
    let pending = std::mem::take(&mut *PENDING_SAY.0.borrow_mut());
    for (text, client) in pending {
        say_text_print(&text, client);
    }
    r
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
    ZOOM_SENS.0.get()
}

// ---------------------------------------------------------------------------------------------
// Message hooks

macro_rules! hooks {
    ($($v:ident),* $(,)?) => {
        /// One message handler per message, dispatching to [`on_message`].
        fn hook_for(m: Msg) -> fn(&[u8]) {
            match m {
                $(Msg::$v => {
                    fn f(b: &[u8]) {
                        on_message(Msg::$v, b);
                    }
                    f
                })*
            }
        }
    };
}

hooks!(
    SelAmmo,
    CurWeapon,
    Geiger,
    Health,
    Damage,
    Battery,
    Train,
    HudText,
    SayText,
    TextMsg,
    WeaponList,
    ResetHUD,
    InitHUD,
    GameTitle,
    DeathMsg,
    ScoreInfo,
    TeamInfo,
    TeamScore,
    GameMode,
    Motd,
    ServerName,
    AmmoPickup,
    WeapPickup,
    ItemPickup,
    HideWeapon,
    SetFov,
    ShowMenu,
    ScreenShake,
    ScreenFade,
    TsFade,
    AmmoX,
    TeamNames,
    VguiMenu,
    WeaponInfo,
    ClipInfo,
    WStatus,
    ActItems,
    KFuPower,
    TsHealth,
    TsArmor,
    TsState,
    TsCash,
    TsSpace,
    NextStunt,
    PwUp,
    PwUpTime,
    RoundTime,
    Objective,
    ShowObj,
    Wires,
    SrvSett,
    TsPAward,
    TsBTime,
    TsHVoice,
    TsFlag,
    CustomWp,
    Spectator,
    TsMBlur,
    TsMessage,
    TsSlowMo,
    PTakeDam,
    TsFrags,
    PlInfo,
    TsPSys,
);

fn local_index() -> i32 {
    engine().local_player().map_or(0, |p| p.index)
}

fn third_person() -> bool {
    crate::view::is_third_person()
}

/// Feed a locally built `TextMsg` through the normal handler (`CHudTextMessage::MsgFunc_TextMsg`
/// with no message, as the scoreboard's mute notices do).
pub fn print_text_message(dest: u8, text: &str) {
    let t = shared::protocol::hud::TextMsg { dest, strings: vec![text.to_owned()] };
    on_message(Msg::TextMsg, &t.encode());
}

/// TS team colour by team number (`GetPlayerTextColor` 1001fcd0: `iTeamColors[team % count]`).
pub fn team_color(team: i32) -> [i32; 3] {
    status::team_color(team)
}

fn on_message(m: Msg, buf: &[u8]) {
    // `MsgFunc_TextMsg` (`0x10034a60`) prints nothing while `AllowedToPrintText` says no.
    if m == Msg::TextMsg && !crate::menus::allowed_to_print_text() {
        return;
    }
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
    if m == Msg::SetFov {
        // TS ignores SetFOV while client weapons predict the FOV (`cl_lw`).
        if e.cvar_float(c"cl_lw") == 0.0 {
            set_fov(shared::protocol::hud::Reader::new(buf).byte());
        }
        return;
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
        cl_thirdperson: input::cvars().cl_thirdperson.value(),
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
    // The viewport's handlers for the same messages (scoreboard teams, team menu).
    crate::menus::on_message(m, buf);
    if m == Msg::ResetHUD {
        // `0x100219c7`: `m_flMouseSensitivity` (`+0x18`) = 0 after the elements' `Reset`.
        ZOOM_SENS.0.set(0.0);
    }
    if m == Msg::DeathMsg
        && let Some(d) = shared::protocol::hud::DeathMsg::decode(buf)
    {
        let name = |i: u8| if i > 0 { e.player_info(i as i32).name } else { String::new() };
        let (k, v) = (name(d.killer), name(d.victim));
        let teamkill = d.killer != d.victim
            && with(|h| {
                h.gamemode != 0 && h.players.get(d.killer as usize).map(|p| p.team) == h.players.get(d.victim as usize).map(|p| p.team)
            });
        e.print(&notice::death_console_line(&k, &v, &d.weapon, d.killer == d.victim || d.killer == 0, teamkill));
        // TS's handler ends with `CHudSpectator::DeathMessage` (the overview's dead icon).
        crate::spectator::death_notice(d.victim as i32);
    }
}

/// Mirror the values other modules read into `crate::state`.
fn sync_shared(h: &Hud) {
    crate::state::with(|s| {
        s.health = h.last_hp;
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
        Effect::ToggleThirdPerson => crate::view::toggle_third_person(),
        Effect::OpenBuyMenu => crate::menus::open_buy(),
        Effect::OpenTeamMenu => crate::menus::open_team_menu(),
        Effect::OpenMotd => crate::menus::open_motd(),
        Effect::ResetSpread => crate::prediction::reset_spread(),
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
        Effect::SayText(t, client) => say_text_print(&saytext::bytes(&t), client),
        // SDK CHudTextMessage sends HUD_PRINTCENTER to the engine's centre print.
        Effect::CenterPrint(t) => e.center_print(&t),
    }
}

/// `CHud::MsgFunc_Logo` (`0x100209c0`): `m_iLogo` (`+0x8`) = the byte. While it is set,
/// `CHud::Redraw` draws the animated `sprites/%d_logo.spr` ([`draw_logo`]).
fn msg_logo(buf: &[u8]) {
    let v = shared::protocol::hud::Reader::new(buf).byte();
    with(|h| h.logo = v as i32);
}

/// `CHud::MsgFunc_Concuss` (`0x10021a40`, hooked at `0x1001fe40`): `m_iConcuss` (`+0x1c`) = the
/// byte, which `CHud::UpdateClientData` copies to `v_idlescale`; non-zero shows the `dmg_concuss`
/// status icon in (255, 160, 0), zero hides it. TS servers never send it (a TFC message).
fn msg_concuss(buf: &[u8]) {
    let v = shared::protocol::hud::Reader::new(buf).byte() as i32;
    let icon = with(|h| {
        h.concuss = v;
        if v != 0 {
            return true;
        }
        h.icons.disable(b"dmg_concuss");
        false
    });
    if icon {
        let grenade = gfx(|g| with(|h| h.icons.enable(b"dmg_concuss", [255, 160, 0], &g.sprites)));
        if grenade {
            play_timer_sound();
        }
    }
}

/// `CHudStatusIcons::MsgFunc_StatusIcon` (`0x1002f1a0`): byte on/off, string sprite name, and
/// when on the colour bytes; turning one on also sets the element's `HUD_ACTIVE`.
fn msg_status_icon(buf: &[u8]) {
    let mut r = shared::protocol::hud::Reader::new(buf);
    let on = r.byte();
    let name = saytext::bytes(&r.string());
    if on == 0 {
        with(|h| h.icons.disable(&name));
        return;
    }
    let rgb = [r.byte(), r.byte(), r.byte()];
    let grenade = gfx(|g| {
        with(|h| {
            let gren = h.icons.enable(&name, rgb, &g.sprites);
            h.icons.active = true;
            gren
        })
    });
    if grenade {
        play_timer_sound();
    }
}

/// `EnableIcon`'s grenade-timer cue: `weapons/timer.wav` on the local player, `CHAN_STATIC`,
/// volume 1, attenuation 0.8, pitch 100.
fn play_timer_sound() {
    let e = engine();
    if let Some(p) = e.local_player() {
        let (index, origin) = (p.index, p.origin);
        e.ev_play_sound(index, origin, 6, "weapons/timer.wav", 1.0, 0.8, 0, 100);
    }
}

/// `CHud::m_iConcuss` (`+0x1c`): `CHud::UpdateClientData` (`0x10024b63`) sets `v_idlescale` from
/// it.
pub fn concussion() -> f32 {
    with(|h| h.concuss) as f32
}

// ---------------------------------------------------------------------------------------------
// Init

fn register_cvars() -> HudCvars {
    HudCvars {
        drawhistory_time: Cvar::register(c"hud_drawhistory_time", c"5", 0),
        fastswitch: Cvar::register(c"hud_fastswitch", c"0", 1),
        deathnotice_time: Cvar::register(c"hud_deathnotice_time", c"6", 0),
        classautokill: Cvar::register(c"hud_classautokill", c"1", 3),
        takesshots: Cvar::register(c"hud_takesshots", c"0", 1),
        zoom_sensitivity_ratio: Cvar::register(c"zoom_sensitivity_ratio", c"1.2", 0),
        // `0x100203de`: registered as `sprintf("%i", 80)`, flags 0, then `value = 80` written
        // directly (the string is left as it is).
        default_fov: {
            let c = Cvar::register(c"default_fov", c"80", 0);
            c.set_value(80.0);
            c
        },
        capturemouse: Cvar::register(c"hud_capturemouse", c"1", 1),
        saytext_time: Cvar::register(c"hud_saytext_time", c"5", 0),
        centerid: Cvar::register(c"hud_centerid", c"0", 1),
        drawnames: Cvar::register(c"cl_drawnames", c"1", 1),
    }
}

macro_rules! cmd {
    ($name:literal, $body:expr) => {{
        extern "C" fn f() {
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
        // The engine parses ScreenShake and ScreenFade itself (FWGS cl_parse.c; GoldSrc
        // likewise); TS hooks neither, nor SelAmmo.
        if matches!(m, Msg::ScreenShake | Msg::ScreenFade | Msg::SelAmmo) {
            continue;
        }
        e.hook_user_msg(name, hook_for(*m));
    }
    e.hook_user_msg(c"Logo", msg_logo);
    e.hook_user_msg(c"Concuss", msg_concuss);
    e.hook_user_msg(c"StatusIcon", msg_status_icon);
    register_commands();
    with(|h| *h = Hud::default());
}

pub fn vid_init() {
    let e = engine();
    let s = e.screen();
    let first = gfx(|g| !g.loaded);
    let sprites = HudSprites::load(s.width);
    let ts = TsSprites::load();
    let train = e.spr_load(&format!("sprites/{}_train.spr", if s.width < 640 { 320 } else { 640 }));
    gfx(|g| {
        g.screen = s;
        g.sprites = sprites;
        g.ts = ts;
        g.train = train;
        g.loaded = true;
    });
    LOGO.0.set(0);
    vgui::vid_init();
    // `CHud::VidInit` (`0x100208a9`) resets only the current scale (`0x100963e8`). The `TSSlowMo`
    // target (`0x1009ff04`) is left alone, so slow motion still active eases back in.
    crate::state::with(|st| st.time_scale = 1.0);
    crate::prediction::reset_alt_grip();
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

/// `InterpolateSlowMotion` (`0x10055ae0`): `cur += min(dt*7, 1) * (target - cur)` while the
/// difference exceeds 0.05, else `cur = target`. TS works on the x87 stack and rounds once when
/// storing; f64 is as close as Rust gets.
pub fn approach_timescale(dt: f32, cur: f32, target: f32) -> f32 {
    let d = target as f64 - cur as f64;
    let d = if d.abs() > 0.05f32 as f64 { (dt as f64 * 7.0).min(1.0) * d } else { d };
    (d + cur as f64) as f32
}

/// The slow-motion part of `HUD_Frame` (`0x1001ab09..0x1001ab24`), TS's only time-scale easer:
/// `g_flTimescale` (`0x100963e8`, `state.time_scale`) moves toward the `TSSlowMo` target
/// (`0x1009ff04`, `state.time_scale_target`) by `HUD_Frame`'s argument, the frame time (FWGS
/// `host.frametime`), narrowed to a float and not clamped.
pub fn frame(time: f64) {
    crate::state::with(|s| s.time_scale = approach_timescale(time as f32, s.time_scale, s.time_scale_target));
}

/// `HUD_GetFOV` (`0x100209f0`): the zoom target `CHud::Think` eases toward (`0x100df1a4`,
/// `state.fov_target`). While recording it writes the value to the demo on every call (record
/// type 0); during playback the last recorded value replaces it.
fn get_fov() -> f32 {
    let e = engine();
    let fov = crate::state::with(|s| s.fov_target);
    if e.demo_recording() {
        crate::demo::write_buffer(crate::demo::FOV, &fov.to_le_bytes());
    }
    if e.demo_playing() {
        let fov = crate::demo::recorded_fov();
        crate::state::with(|s| s.fov_target = fov);
        return fov;
    }
    fov
}

/// `CHud::Think` (`0x10049760`), from `CHud::UpdateClientData` (`0x10024b00`, i.e.
/// `HUD_UpdateClientData`): eases `m_iFOV` (`state.fov`) toward `HUD_GetFOV` (or `default_fov`
/// while that is 0) with `UTIL_ApproachFOV` ([`crate::view::ease_fov`]) and sets the zoom
/// sensitivity. `ft` is the last refdef's frame time. Returns the eased FOV.
pub fn think_fov(ft: f32) -> f32 {
    let c = cvars();
    let df = c.default_fov.value();
    let default = if df < 0.001 { 80.0 } else { df.clamp(70.0, 110.0) };
    let want = get_fov() as i32;
    let target = if (want as f32) < 0.001 { default as i32 } else { want };
    let mut cur = crate::state::with(|s| crate::view::ease_fov(s.fov, target as f32, ft));
    let sens = if (cur - default).abs() >= 0.01 {
        want as f32 / default * input::cvars().sensitivity.value() * c.zoom_sensitivity_ratio.value()
    } else {
        0.0
    };
    ZOOM_SENS.0.set(sens);
    if cur < 0.001 {
        cur = default as i32 as f32;
    }
    crate::state::with(|s| s.fov = cur);
    cur
}

/// The tail of `CHud::MsgFunc_SetFOV` (`0x10049570`), reached only while `cl_lw` is off: the
/// zoom target takes the new FOV, `m_iFOV` jumps to it (0 means 80), and the zoom sensitivity
/// becomes `fov * 0.0125 * sensitivity * zoom_sensitivity_ratio` (0 at 80).
fn set_fov(fov: u8) {
    let fov = fov as i32;
    let cur = if fov == 0 { 80 } else { fov };
    crate::state::with(|s| {
        s.fov_target = fov as f32;
        s.fov = cur as f32;
    });
    let sens =
        if cur == 80 { 0.0 } else { fov as f32 * 0.0125 * input::cvars().sensitivity.value() * cvars().zoom_sensitivity_ratio.value() };
    ZOOM_SENS.0.set(sens);
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
    if (1..=10).contains(&n)
        && let Some(cmd) = with(|h| h.menu_select(n))
    {
        engine().server_cmd(cmd.trim_end());
        return;
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

/// `gHUD.m_flTimeDelta` (`0x100cf950`): the time between the last two `HUD_Redraw`s, never
/// negative (the port also caps it at 1 s, which only matters for the HUD's own timers).
pub fn time_delta() -> f32 {
    TIME.0.get().1
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
    let hide_all = with(|h| h.hide_hud & 4 != 0);

    let s = e.screen();
    gfx(|g| {
        if s.width != g.screen.width || s.height != g.screen.height {
            g.sprites = HudSprites::load(s.width);
        }
        g.screen = s;
    });
    think(dt);
    with(|h| sync_shared(h));
    let g = GFX.0.borrow();
    let Some(g) = g.as_ref() else { return };
    let dc = Dc { g, now: time, hud: input::hud_rgb() };
    if intermission {
        // Only `HUD_INTERMISSION` elements draw: TS sets it on the say text alone.
        draw_say_text(&dc);
    } else if !hide_all {
        let nohud = input::cvars().nohud.value() != 0.0;
        // TS element order (§4.1), each needing `HUD_ACTIVE` and no `HIDEHUD_ALL`: CHudTrain,
        // CHudMessage, CHudDeathNotice, CHudStatusIcons, CHudSpectator (HLTV name tags),
        // CHudTSWeapons, CHudTSStatus, CHudSayText, CHudMenu.
        draw_train(&dc);
        notice::draw_hud_text(&dc);
        if !nohud {
            notice::draw_death_notices(&dc);
        }
        let icons = with(|h| h.icons.active.then(|| h.icons.clone()));
        if let Some(icons) = icons {
            icons.draw(dc.h());
        }
        crate::spectator::draw_hltv_names();
        weapons::draw(&dc, nohud);
        status::draw(&dc, nohud);
        draw_say_text(&dc);
        notice::draw_menu(&dc);
    }
    draw_logo(g, time);
}

/// The end of `CHud::Redraw` (`0x10021bcd`): while `m_iLogo` is set, frame
/// `grgLogoFrame[(int)(time * 20) % 56] - 1` of `sprites/%d_logo.spr` (loaded on first use, and
/// tried again every frame while missing) in (250, 250, 250) at the top right, half its height
/// down. Drawn in intermission and under `HIDEHUD_ALL` too.
fn draw_logo(g: &Gfx, time: f32) {
    /// `grgLogoFrame` (`0x10093990`).
    const FRAMES: [i32; 56] = [
        1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 13, 13, 13, 13, 13, 12, 11, 10, 9, 8, 14, 15, 16, 17, 18, 19, 20, 20, 20, 20, 20, 20,
        21, 22, 23, 24, 25, 26, 27, 28, 29, 29, 29, 29, 29, 29, 28, 27, 26, 25, 24, 30, 31,
    ];
    if with(|h| h.logo) == 0 {
        return;
    }
    let e = engine();
    let mut spr = LOGO.0.get();
    if spr == 0 {
        // `0x10058450`: `sprites/%d_logo.spr` with 640 from 640 px wide, else 320.
        spr = e.spr_load(&format!("sprites/{}_logo.spr", if g.screen.width >= 640 { 640 } else { 320 }));
        LOGO.0.set(spr);
    }
    e.spr_set(spr, [250, 250, 250]);
    let x = g.screen.width - e.spr_width(spr, 0);
    let y = e.spr_height(spr, 0) / 2;
    let i = ((time * 20.0) as i32 % 56).unsigned_abs() as usize;
    e.spr_draw_additive(FRAMES[i] - 1, x, y, None);
}

/// `CHud::m_hsprLogo` (`+0x4`), zeroed by `VidInit`.
static LOGO: Global<Cell<goldsrc::sys::HSPRITE>> = Global(Cell::new(0));

/// The say text element, when active (`HUD_ACTIVE`).
fn draw_say_text(dc: &Dc) {
    if !with(|h| h.say_text.active) {
        return;
    }
    // `Draw` (`0x1002eb40`) draws nothing while `AllowedToPrintText` says no.
    if !crate::menus::allowed_to_print_text() {
        return;
    }
    let plan = with(|h| h.say_text.draw_plan(dc.now, &mut EngineSay));
    let e = engine();
    for l in plan {
        match l {
            saytext::DrawLine::Name { y, name, color, rest } => {
                let x = e.draw_console_bytes(10, y, Some(color), &name);
                e.draw_console_bytes(x, y, None, &rest);
            }
            saytext::DrawLine::Plain { y, text } => {
                e.draw_console_bytes(10, y, Some(saytext::SAY_COLOR), &text);
            }
        }
    }
}

/// The engine half of [`saytext::SayEnv`].
struct EngineSay;
impl saytext::SayEnv for EngineSay {
    fn string_size(&self, text: &[u8]) -> (i32, i32) {
        engine().console_bytes_size(text)
    }
    fn console_print(&mut self, text: &[u8]) {
        engine().print_bytes(text);
    }
}

/// Lines [`say_text_print`] couldn't add because the HUD state was borrowed (an effect that
/// warns from inside a HUD callback); [`with`] adds them when the borrow ends.
static PENDING_SAY: Global<RefCell<Vec<PendingSay>>> = Global(RefCell::new(Vec::new()));
/// A deferred `SayTextPrint`: (text, client).
type PendingSay = (Vec<u8>, i32);

/// `CHudSayText::SayTextPrint` (`0x1002ed00`): drops text from a muted player; while
/// `AllowedToPrintText` says no it goes to the console only; otherwise it becomes a chat line
/// (the speaker's name coloured when the text starts with `\x02`) and `misc/talk.wav` plays.
pub fn say_text_print(text: &[u8], client: i32) {
    if crate::voice::is_player_blocked(client) {
        return;
    }
    let e = engine();
    if !crate::menus::allowed_to_print_text() {
        e.print_bytes(text);
        return;
    }
    let Ok(mut h) = HUD.0.try_borrow_mut() else {
        PENDING_SAY.0.borrow_mut().push((text.to_vec(), client));
        return;
    };
    let h = h.get_or_insert_with(Hud::default);
    let name = (text.first() == Some(&2) && client > 0).then(|| saytext::bytes(&e.player_info(client).name));
    // `GetClientColor` reads the scoreboard teams, which live in the state borrowed here.
    let color = name.as_ref().and_then(|_| {
        if client == local_index() {
            return Some([1.0, 1.0, 1.0]);
        }
        match h.players.get(client as usize).map_or(0, |p| p.team) {
            1 => Some([0.6, 0.8, 1.0]),
            2 => Some([1.0, 0.25, 0.25]),
            3 => Some(saytext::SAY_COLOR),
            4 => Some([0.6, 1.0, 0.6]),
            _ => None,
        }
    });
    // `ScreenWidth`/`ScreenHeight` (the HUD's screen info); the draw pass may hold it.
    let screen = match GFX.0.try_borrow() {
        Ok(g) => g.as_ref().map(|g| (g.screen.width, g.screen.height)),
        Err(_) => None,
    }
    .unwrap_or_else(|| {
        let s = e.screen();
        (s.width, s.height)
    });
    let speaker = name.as_deref().filter(|n| !n.is_empty()).map(|n| (n, color));
    let saytime = || cvars().saytext_time.value();
    h.say_text.print(text, speaker, now(), saytime, screen, &mut EngineSay);
    e.play_sound("misc/talk.wav", 1.0);
}

/// `CHudTrain::Draw` (10034d40): the speed notch frame of the train sprite, yellowish, a third of
/// the way across and just above the bottom text line. Hidden at notch 0 (`HUD_ACTIVE` off) and,
/// like every element in `CHud::Redraw`, under `HIDEHUD_ALL`.
fn draw_train(dc: &Dc) {
    let (pos, hide) = with(|h| (h.train_pos, h.hide_hud));
    let spr = dc.g.train;
    if pos == 0 || hide & 4 != 0 || spr == 0 {
        return;
    }
    let e = engine();
    e.spr_set(spr, [255, 160, 0]);
    let y = dc.h() - e.spr_height(spr, 0) - dc.g.screen.char_height;
    let x = dc.w() / 3 + e.spr_width(spr, 0) / 4;
    e.spr_draw_additive(pos as i32 - 1, x, y, None);
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
        let mut ts = 1.0;
        for _ in 0..200 {
            ts = approach_timescale(0.01, ts, 0.35);
        }
        assert_eq!(ts, 0.35, "settles exactly on the target");
    }

    #[test]
    fn every_message_has_a_hook() {
        for (m, _, _) in MESSAGES {
            let _: fn(&[u8]) = hook_for(*m); // every message maps to a handler (exhaustive match)
        }
    }
}
