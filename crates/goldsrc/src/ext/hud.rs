//! Engine helpers added by the hud work package. Keep additions here to avoid merge conflicts.
//!
//! Only slots present in the 2007 `cl_enginefunc_t` that TS 3.0 was built against are used, so
//! these work on old Steam GoldSrc, HL25 and FWGS alike.

use crate::cl_eng;
use crate::client::ClEngine;
use crate::sys::*;
use crate::util::{cstr, to_cstring};
use std::ffi::{CStr, c_char, c_int};

/// Prefix of the SDK `event_api_s` (common/event_api.h). bindgen only sees it as opaque, and the
/// layout has been frozen since 1999, so the HUD declares the slots it needs.
#[repr(C)]
pub struct EventApi {
    pub version: c_int,
    pub ev_play_sound: Option<unsafe extern "C" fn(c_int, *mut f32, c_int, *const c_char, f32, f32, c_int, c_int)>,
    pub ev_stop_sound: Option<unsafe extern "C" fn(c_int, c_int, *const c_char)>,
    pub ev_find_model_index: Option<unsafe extern "C" fn(*const c_char) -> c_int>,
    pub ev_is_local: Option<unsafe extern "C" fn(c_int) -> c_int>,
    pub ev_local_player_ducking: Option<unsafe extern "C" fn() -> c_int>,
    pub ev_local_player_viewheight: Option<unsafe extern "C" fn(*mut f32)>,
    pub ev_local_player_bounds: Option<unsafe extern "C" fn(c_int, *mut f32, *mut f32)>,
    pub ev_index_from_trace: Option<unsafe extern "C" fn(*mut pmtrace_s) -> c_int>,
    pub ev_get_physent: Option<unsafe extern "C" fn(c_int) -> *mut physent_s>,
    pub ev_set_up_player_prediction: Option<unsafe extern "C" fn(c_int, c_int)>,
    pub ev_push_pm_states: Option<unsafe extern "C" fn()>,
    pub ev_pop_pm_states: Option<unsafe extern "C" fn()>,
    pub ev_set_solid_players: Option<unsafe extern "C" fn(c_int)>,
    pub ev_set_trace_hull: Option<unsafe extern "C" fn(c_int)>,
    pub ev_player_trace: Option<unsafe extern "C" fn(*mut f32, *mut f32, c_int, c_int, *mut pmtrace_s)>,
}

/// SDK `demo_api_s`.
#[repr(C)]
pub struct DemoApi {
    pub is_recording: Option<unsafe extern "C" fn() -> c_int>,
    pub is_playingback: Option<unsafe extern "C" fn() -> c_int>,
    pub is_time_demo: Option<unsafe extern "C" fn() -> c_int>,
    pub write_buffer: Option<unsafe extern "C" fn(c_int, *mut u8)>,
}

/// `PM_STUDIO_BOX` trace flag for `EV_PlayerTrace` (SDK pm_defs.h: 0 normal, 1 studio ignore,
/// 2 studio box).
pub const PM_STUDIO_BOX: c_int = 2;

/// One entry of `SPR_GetList` (hud.txt), copied out of engine memory.
#[derive(Debug, Clone)]
pub struct SpriteEntry {
    pub name: String,
    pub sprite: String,
    pub res: i32,
    pub rc: wrect_t,
}

/// Screen size and font metrics from `pfnGetScreenInfo`.
#[derive(Clone)]
pub struct Screen {
    pub width: i32,
    pub height: i32,
    pub char_height: i32,
    pub char_widths: [i16; 256],
}

impl Default for Screen {
    fn default() -> Self {
        Screen { width: 640, height: 480, char_height: 12, char_widths: [8; 256] }
    }
}

/// Player info from `pfnGetPlayerInfo`.
#[derive(Debug, Clone, Default)]
pub struct PlayerInfo {
    pub name: String,
    pub ping: i32,
    pub packetloss: i32,
    pub this_player: bool,
    pub spectator: bool,
    pub model: String,
    pub topcolor: i32,
    pub bottomcolor: i32,
}

impl ClEngine {
    pub fn spr_load(&self, name: &str) -> HSPRITE {
        let s = to_cstring(name);
        cl_eng!(pfnSPR_Load(s.as_ptr()))
    }
    pub fn spr_width(&self, h: HSPRITE, frame: i32) -> i32 {
        if h == 0 { 0 } else { cl_eng!(pfnSPR_Width(h, frame)) }
    }
    pub fn spr_height(&self, h: HSPRITE, frame: i32) -> i32 {
        if h == 0 { 0 } else { cl_eng!(pfnSPR_Height(h, frame)) }
    }
    pub fn spr_set(&self, h: HSPRITE, rgb: [i32; 3]) {
        cl_eng!(pfnSPR_Set(h, rgb[0], rgb[1], rgb[2]));
    }
    fn rc_ptr(rc: Option<&wrect_t>) -> *const wrect_t {
        rc.map_or(std::ptr::null(), |r| r as *const wrect_t)
    }
    pub fn spr_draw(&self, frame: i32, x: i32, y: i32, rc: Option<&wrect_t>) {
        cl_eng!(pfnSPR_Draw(frame, x, y, Self::rc_ptr(rc)));
    }
    pub fn spr_draw_additive(&self, frame: i32, x: i32, y: i32, rc: Option<&wrect_t>) {
        cl_eng!(pfnSPR_DrawAdditive(frame, x, y, Self::rc_ptr(rc)));
    }
    pub fn spr_draw_holes(&self, frame: i32, x: i32, y: i32, rc: Option<&wrect_t>) {
        cl_eng!(pfnSPR_DrawHoles(frame, x, y, Self::rc_ptr(rc)));
    }

    /// `SPR_GetList`: parses e.g. `sprites/hud.txt`. Entries are copied; the engine keeps the list.
    pub fn spr_get_list(&self, file: &str) -> Vec<SpriteEntry> {
        let s = to_cstring(file);
        let mut n: c_int = 0;
        let p = cl_eng!(pfnSPR_GetList(s.as_ptr().cast_mut(), &mut n));
        if p.is_null() || n <= 0 {
            return Vec::new();
        }
        let list = unsafe { std::slice::from_raw_parts(p, n as usize) };
        list.iter()
            .map(|e| SpriteEntry {
                name: unsafe { cstr(e.szName.as_ptr()) }.to_string_lossy().into_owned(),
                sprite: unsafe { cstr(e.szSprite.as_ptr()) }.to_string_lossy().into_owned(),
                res: e.iRes,
                rc: e.rc,
            })
            .collect()
    }

    pub fn screen(&self) -> Screen {
        let mut info = SCREENINFO { iSize: size_of::<SCREENINFO>() as i32, ..Default::default() };
        cl_eng!(pfnGetScreenInfo(&mut info));
        Screen { width: info.iWidth, height: info.iHeight, char_height: info.iCharHeight, char_widths: info.charWidths }
    }

    /// `pfnDrawCharacter`: returns the advance.
    pub fn draw_character(&self, x: i32, y: i32, ch: u8, rgb: [i32; 3]) -> i32 {
        cl_eng!(pfnDrawCharacter(x, y, ch as c_int, rgb[0], rgb[1], rgb[2]))
    }

    /// Console font string with an explicit colour; returns the x after the text.
    pub fn draw_console_string(&self, x: i32, y: i32, rgb: [f32; 3], text: &str) -> i32 {
        let s = to_cstring(text);
        cl_eng!(pfnDrawSetTextColor(rgb[0], rgb[1], rgb[2]));
        cl_eng!(pfnDrawConsoleString(x, y, s.as_ptr().cast_mut()))
    }

    pub fn console_string_size(&self, text: &str) -> (i32, i32) {
        let s = to_cstring(text);
        let (mut w, mut h) = (0, 0);
        cl_eng!(pfnDrawConsoleStringLen(s.as_ptr(), &mut w, &mut h));
        (w, h)
    }

    pub fn fill(&self, x: i32, y: i32, w: i32, h: i32, rgb: [i32; 3], a: i32) {
        cl_eng!(pfnFillRGBA(x, y, w, h, rgb[0], rgb[1], rgb[2], a));
    }

    pub fn server_cmd(&self, cmd: &str) {
        let s = to_cstring(cmd);
        cl_eng!(pfnServerCmd(s.as_ptr()));
    }
    pub fn client_cmd(&self, cmd: &str) {
        let s = to_cstring(cmd);
        cl_eng!(pfnClientCmd(s.as_ptr()));
    }
    pub fn center_print(&self, text: &str) {
        let s = to_cstring(text);
        cl_eng!(pfnCenterPrint(s.as_ptr()));
    }

    /// `pfnRegisterVariable`; the engine keeps both strings.
    pub fn register_variable(&self, name: &'static CStr, value: &'static CStr, flags: i32) -> *mut cvar_s {
        cl_eng!(pfnRegisterVariable(name.as_ptr(), value.as_ptr(), flags))
    }
    pub fn cvar_pointer(&self, name: &CStr) -> *mut cvar_s {
        cl_eng!(pfnGetCvarPointer(name.as_ptr()))
    }
    pub fn cvar_string(&self, name: &CStr) -> String {
        let p = cl_eng!(pfnGetCvarString(name.as_ptr()));
        unsafe { cstr(p) }.to_string_lossy().into_owned()
    }
    pub fn cvar_set(&self, name: &CStr, value: f32) {
        cl_eng!(Cvar_SetValue(name.as_ptr(), value));
    }
    pub fn cmd_argc(&self) -> i32 {
        cl_eng!(Cmd_Argc())
    }

    /// `pfnTextMessageGet` (titles.txt): the message, if the key exists.
    pub fn text_message(&self, name: &str) -> Option<&'static client_textmessage_t> {
        let s = to_cstring(name);
        let p = cl_eng!(pfnTextMessageGet(s.as_ptr()));
        if p.is_null() { None } else { Some(unsafe { &*p }) }
    }

    pub fn player_info(&self, index: i32) -> PlayerInfo {
        let mut info: hud_player_info_t = unsafe { std::mem::zeroed() };
        cl_eng!(pfnGetPlayerInfo(index, &mut info));
        PlayerInfo {
            name: unsafe { cstr(info.name) }.to_string_lossy().into_owned(),
            ping: info.ping as i32,
            packetloss: info.packetloss as i32,
            this_player: info.thisplayer != 0,
            spectator: info.spectator != 0,
            model: unsafe { cstr(info.model) }.to_string_lossy().into_owned(),
            topcolor: info.topcolor as i32,
            bottomcolor: info.bottomcolor as i32,
        }
    }

    pub fn local_player(&self) -> Option<&'static cl_entity_t> {
        let p = cl_eng!(GetLocalPlayer());
        if p.is_null() { None } else { Some(unsafe { &*p }) }
    }
    pub fn entity(&self, index: i32) -> Option<&'static cl_entity_t> {
        let p = cl_eng!(GetEntityByIndex(index));
        if p.is_null() { None } else { Some(unsafe { &*p }) }
    }
    pub fn max_clients(&self) -> i32 {
        cl_eng!(GetMaxClients())
    }
    pub fn client_time(&self) -> f32 {
        cl_eng!(GetClientTime())
    }
    pub fn mouse_position(&self) -> (i32, i32) {
        let (mut x, mut y) = (0, 0);
        cl_eng!(GetMousePosition(&mut x, &mut y));
        (x, y)
    }
    pub fn set_view_angles(&self, mut a: [f32; 3]) {
        cl_eng!(SetViewAngles(a.as_mut_ptr()));
    }
    pub fn key_event(&self, key: i32, down: bool) {
        cl_eng!(Key_Event(key, down as c_int));
    }
    pub fn play_sound(&self, name: &str, volume: f32) {
        let s = to_cstring(name);
        cl_eng!(pfnPlaySoundByName(s.as_ptr().cast_mut(), volume));
    }
    pub fn game_directory(&self) -> String {
        let p = cl_eng!(pfnGetGameDirectory());
        unsafe { cstr(p) }.to_string_lossy().into_owned()
    }

    /// `EV_LocalPlayerViewheight`.
    pub fn local_view_height(&self) -> [f32; 3] {
        let mut v = [0.0, 0.0, 28.0];
        if let Some(f) = self.event_api().and_then(|a| a.ev_local_player_viewheight) {
            unsafe { f(v.as_mut_ptr()) };
        }
        v
    }

    /// The event API (`EV_PlayerTrace` etc.), if the engine provides one.
    pub fn event_api(&self) -> Option<&'static EventApi> {
        let p = self.f.pEventAPI as *const EventApi;
        if p.is_null() { None } else { Some(unsafe { &*p }) }
    }
    pub fn demo_api(&self) -> Option<&'static DemoApi> {
        let p = self.f.pDemoAPI as *const DemoApi;
        if p.is_null() { None } else { Some(unsafe { &*p }) }
    }
    pub fn demo_playing(&self) -> bool {
        self.demo_api().and_then(|d| d.is_playingback).is_some_and(|f| unsafe { f() } != 0)
    }
    pub fn demo_recording(&self) -> bool {
        self.demo_api().and_then(|d| d.is_recording).is_some_and(|f| unsafe { f() } != 0)
    }

    /// `EV_PlayerTrace` from `start` to `end` against the world and the other players (studio
    /// boxes), as TS's crosshair ID does. Returns (fraction, endpos, hit entity index or 0).
    pub fn player_trace(&self, start: [f32; 3], end: [f32; 3], local_index: i32) -> Option<(f32, [f32; 3], i32)> {
        let api = self.event_api()?;
        let mut tr: pmtrace_s = unsafe { std::mem::zeroed() };
        let (mut s, mut e) = (start, end);
        unsafe {
            (api.ev_push_pm_states?)();
            (api.ev_set_up_player_prediction?)(0, 1);
            (api.ev_set_solid_players?)(local_index - 1);
            (api.ev_set_trace_hull?)(2);
            (api.ev_player_trace?)(s.as_mut_ptr(), e.as_mut_ptr(), PM_STUDIO_BOX, -1, &mut tr);
            let ent = if tr.fraction < 1.0 { (api.ev_index_from_trace?)(&mut tr) } else { 0 };
            (api.ev_pop_pm_states?)();
            Some((tr.fraction, tr.endpos, ent))
        }
    }
}

/// Prefix of the SDK `triangleapi_s` up to `WorldToScreen` (common/triangleapi.h).
#[repr(C)]
struct TriApiPrefix {
    version: c_int,
    render_mode: *const std::ffi::c_void,
    begin: *const std::ffi::c_void,
    end: *const std::ffi::c_void,
    color4f: *const std::ffi::c_void,
    color4ub: *const std::ffi::c_void,
    tex_coord2f: *const std::ffi::c_void,
    vertex3fv: *const std::ffi::c_void,
    vertex3f: *const std::ffi::c_void,
    brightness: *const std::ffi::c_void,
    cull_face: *const std::ffi::c_void,
    sprite_texture: *const std::ffi::c_void,
    world_to_screen: Option<unsafe extern "C" fn(*const f32, *mut f32) -> c_int>,
}

impl ClEngine {
    /// TS `WorldToScreen` (100579c0): screen pixel position of a world point, or `None` when it
    /// is behind the view or off screen.
    pub fn world_to_screen(&self, world: [f32; 3], screen: &Screen) -> Option<(i32, i32)> {
        let api = self.f.pTriAPI as *const TriApiPrefix;
        if api.is_null() {
            return None;
        }
        let f = unsafe { (*api).world_to_screen }?;
        let mut out = [0.0f32; 3];
        if unsafe { f(world.as_ptr(), out.as_mut_ptr()) } != 0 {
            return None;
        }
        let x = ((1.0 + out[0]) * screen.width as f32 * 0.5) as i32;
        let y = ((1.0 - out[1]) * screen.height as f32 * 0.5) as i32;
        (0..screen.width).contains(&x).then_some(())?;
        (0..screen.height).contains(&y).then_some((x, y))
    }
}

/// `SND_CHANGE_VOL` for [`ClEngine::ev_play_sound`].
pub const SND_CHANGE_VOL: i32 = 1 << 6;

impl ClEngine {
    /// `EV_PlaySound(ent, origin, channel, sample, volume, attenuation, flags, pitch)`.
    #[allow(clippy::too_many_arguments)]
    pub fn ev_play_sound(&self, ent: i32, origin: [f32; 3], channel: i32, sample: &CStr, volume: f32, attn: f32, flags: i32, pitch: i32) {
        let Some(f) = self.event_api().and_then(|a| a.ev_play_sound) else { return };
        let mut o = origin;
        unsafe { f(ent, o.as_mut_ptr(), channel, sample.as_ptr(), volume, attn, flags, pitch) };
    }
}
