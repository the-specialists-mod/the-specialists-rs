//! Engine helpers added by the hud work package. Keep additions here to avoid merge conflicts.
//!
//! Only slots present in the 2007 `cl_enginefunc_t` that TS 3.0 was built against are used, so
//! these work on old Steam GoldSrc, HL25 and FWGS alike.
//!
//! Each engine call has one wrapper, either here or in [`render`]; the `ClEngine` table
//! accessors (`event_api`, `efx_api`, `tri_api`, `demo_api`) are the null-checked ones, and
//! the `ev!`/`efx!`/`tri!` macros go through them.
//!
//! # Client entities
//! Game code reaches a `cl_entity_t` by index only through [`ClEngine::entity`] and
//! [`ClEngine::local_player`], which hand out shared `&'static` references, and
//! [`ClEngine::with_entity_mut`] / [`ClEngine::with_local_player_mut`], which lend a `&mut`
//! for the length of a closure. The engine owns the entity array for the process (slots are
//! reused, never unmapped) and rewrites entries each frame from the network, all on the client
//! thread. So, as with edicts (`edict.rs`):
//! * read what you need and drop the shared reference within the callback; don't keep it
//!   across frames, since the slot may have been reused for another entity;
//! * never hold a shared reference to an entity across a `with_*_mut` call (or a call that
//!   makes the engine rewrite entities, such as `EV_SetUpPlayerPrediction`, which recomputes
//!   other players' origins); copy the fields out first;
//! * don't nest `with_*_mut` calls for the same entity.
//!
//! Engine callbacks that pass an entity to modify (`HUD_AddEntity`, the studio and temp-entity
//! callbacks) and [`render::view_model`] hand out their own mutable views; the same rules
//! apply between those and the accessors here.

use crate::cl_eng;
use crate::client::ClEngine;
use crate::ext::render;
use crate::sys::*;
use crate::util::{cstr, to_cstring};
use std::ffi::{CStr, c_int};

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
    /// `pfnSPR_Load`: the single wrapper for it.
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
        // SAFETY: the engine returned `n` (> 0) sprite entries at non-null `p`, valid until the next `SPR_GetList`; copied below.
        let list = unsafe { std::slice::from_raw_parts(p, n as usize) };
        list.iter()
            .map(|e| SpriteEntry {
                // SAFETY: the sprite list entries hold NUL-terminated names (engine-filled char arrays).
                name: unsafe { cstr(e.szName.as_ptr()) }.to_string_lossy().into_owned(),
                // SAFETY: the sprite list entries hold NUL-terminated names (engine-filled char arrays).
                sprite: unsafe { cstr(e.szSprite.as_ptr()) }.to_string_lossy().into_owned(),
                res: e.iRes,
                rc: e.rc,
            })
            .collect()
    }

    /// `pfnGetScreenInfo`: the single wrapper for it.
    pub fn screen(&self) -> Screen {
        let mut info = SCREENINFO { iSize: size_of::<SCREENINFO>() as i32, ..Default::default() };
        cl_eng!(pfnGetScreenInfo(&mut info));
        Screen { width: info.iWidth, height: info.iHeight, char_height: info.iCharHeight, char_widths: info.charWidths }
    }

    /// `pfnDrawCharacter`: returns the advance.
    pub fn draw_character(&self, x: i32, y: i32, ch: u8, rgb: [i32; 3]) -> i32 {
        cl_eng!(pfnDrawCharacter(x, y, ch as c_int, rgb[0], rgb[1], rgb[2]))
    }

    /// `pfnDrawSetTextColor` + `pfnDrawConsoleString`: the single wrapper for them. Console font
    /// string with an explicit colour; returns the x after the text.
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

    /// `pfnFillRGBA`: the single wrapper for it.
    pub fn fill(&self, x: i32, y: i32, w: i32, h: i32, rgb: [i32; 3], a: i32) {
        cl_eng!(pfnFillRGBA(x, y, w, h, rgb[0], rgb[1], rgb[2], a));
    }

    /// `pfnServerCmd`: the single wrapper for it. Every engine fills the slot; if it is null the
    /// command is dropped rather than panicking, since callers are fire-and-forget.
    pub fn server_cmd(&self, cmd: &str) {
        let Some(f) = self.f.pfnServerCmd else { return };
        let s = to_cstring(cmd);
        // SAFETY: engine callback from the client function table, called with a NUL-terminated string that outlives the call.
        unsafe { f(s.as_ptr()) };
    }
    pub fn client_cmd(&self, cmd: &str) {
        let s = to_cstring(cmd);
        cl_eng!(pfnClientCmd(s.as_ptr()));
    }
    pub fn center_print(&self, text: &str) {
        let s = to_cstring(text);
        cl_eng!(pfnCenterPrint(s.as_ptr()));
    }

    pub fn cvar_string(&self, name: &CStr) -> String {
        let p = cl_eng!(pfnGetCvarString(name.as_ptr()));
        // SAFETY: the engine returns null or a NUL-terminated string it owns; it's used or copied before the next engine call.
        unsafe { cstr(p) }.to_string_lossy().into_owned()
    }
    pub fn cvar_set(&self, name: &CStr, value: f32) {
        cl_eng!(Cvar_SetValue(name.as_ptr(), value));
    }
    pub fn cmd_argc(&self) -> i32 {
        cl_eng!(Cmd_Argc())
    }

    /// `pfnTextMessageGet` (titles.txt): the message, if the key exists.
    pub fn text_message(&self, name: &str) -> Option<TextMessage> {
        let s = to_cstring(name);
        let p = cl_eng!(pfnTextMessageGet(s.as_ptr()));
        // SAFETY: the engine's titles.txt entries live for the process.
        if p.is_null() { None } else { Some(TextMessage(unsafe { &*p })) }
    }

    pub fn player_info(&self, index: i32) -> PlayerInfo {
        let mut info = hud_player_info_t::default();
        cl_eng!(pfnGetPlayerInfo(index, &mut info));
        PlayerInfo {
            // SAFETY: the engine returns null or a NUL-terminated string it owns; it's used or copied before the next engine call.
            name: unsafe { cstr(info.name) }.to_string_lossy().into_owned(),
            ping: info.ping as i32,
            packetloss: info.packetloss as i32,
            this_player: info.thisplayer != 0,
            spectator: info.spectator != 0,
            // SAFETY: the engine returns null or a NUL-terminated string it owns; it's used or copied before the next engine call.
            model: unsafe { cstr(info.model) }.to_string_lossy().into_owned(),
            topcolor: info.topcolor as i32,
            bottomcolor: info.bottomcolor as i32,
        }
    }

    /// `GetLocalPlayer`: the single wrapper for it (the accessors below go through it).
    fn local_player_ptr(&self) -> *mut cl_entity_t {
        cl_eng!(GetLocalPlayer())
    }
    /// `GetEntityByIndex`: the single wrapper for it (the accessors below go through it).
    fn entity_ptr(&self, index: i32) -> *mut cl_entity_t {
        cl_eng!(GetEntityByIndex(index))
    }
    /// The local player's entity: a shared, read-only view. See the module docs.
    pub fn local_player(&self) -> Option<&'static cl_entity_t> {
        // SAFETY: null or an entry of the engine's entity array, which lives for the process;
        // read-only and short-lived per the module contract.
        unsafe { self.local_player_ptr().as_ref() }
    }
    /// Entity `index`: a shared, read-only view. See the module docs.
    pub fn entity(&self, index: i32) -> Option<&'static cl_entity_t> {
        // SAFETY: as in `local_player`.
        unsafe { self.entity_ptr(index).as_ref() }
    }
    /// Runs `f` on the local player's entity with a mutable view scoped to the call (the SDK's
    /// `V_CalcRefdef` edits it: render mode, model origin/angles). `None` if there is none. No
    /// reference from [`local_player`](Self::local_player)/[`entity`](Self::entity) may be
    /// held across the call.
    pub fn with_local_player_mut<R>(&self, f: impl FnOnce(&mut cl_entity_t) -> R) -> Option<R> {
        // SAFETY: as in `local_player`; the exclusive borrow ends with `f`, and the module
        // contract forbids holding another view of the entry across it.
        unsafe { self.local_player_ptr().as_mut() }.map(f)
    }
    /// Runs `f` on entity `index` with a mutable view scoped to the call. `None` if there is
    /// none. Same rule as [`with_local_player_mut`](Self::with_local_player_mut).
    pub fn with_entity_mut<R>(&self, index: i32, f: impl FnOnce(&mut cl_entity_t) -> R) -> Option<R> {
        // SAFETY: as in `with_local_player_mut`.
        unsafe { self.entity_ptr(index).as_mut() }.map(f)
    }
    /// `GetMaxClients`: the single wrapper for it.
    pub fn max_clients(&self) -> i32 {
        cl_eng!(GetMaxClients())
    }
    /// `GetClientTime`: the single wrapper for it.
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
        // SAFETY: the engine returns null or a NUL-terminated string it owns; it's used or copied before the next engine call.
        unsafe { cstr(p) }.to_string_lossy().into_owned()
    }

    /// `EV_LocalPlayerViewheight`: the local player's view offset. The single wrapper for it,
    /// used by the crosshair ID and by the events' `EV_GetGunPosition`. Every engine fills the
    /// slot; if the table or slot is null this returns the standing view offset (0, 0, 28)
    /// instead of panicking, since both callers only need a plausible eye position.
    pub fn local_view_height(&self) -> [f32; 3] {
        let mut v = [0.0, 0.0, 28.0];
        if let Some(f) = self.event_api().and_then(|a| a.EV_LocalPlayerViewheight) {
            // SAFETY: engine callback from the event API table, called with a valid 3-float buffer.
            unsafe { f(v.as_mut_ptr()) };
        }
        v
    }

    /// The event API (`EV_PlayerTrace` etc.), if the engine provides one. Only slots of the 2007
    /// SDK `event_api_s` are called; that table is a prefix of the FWGS one bindgen generates.
    pub fn event_api(&self) -> Option<&'static event_api_s> {
        // SAFETY: the engine sets `pEventAPI` (or leaves it null) before `Initialize` and keeps the table for the process.
        unsafe { self.f.pEventAPI.as_ref() }
    }
    /// The demo API (`IsPlayingback` etc.), if the engine provides one.
    pub fn demo_api(&self) -> Option<&'static demo_api_s> {
        // SAFETY: the engine sets `pDemoAPI` (or leaves it null) before `Initialize` and keeps the table for the process.
        unsafe { self.f.pDemoAPI.as_ref() }
    }
    /// `IsPlayingback`; false if the table or slot is null.
    pub fn demo_playing(&self) -> bool {
        // SAFETY: engine callback from the demo API table, no arguments.
        self.demo_api().and_then(|d| d.IsPlayingback).is_some_and(|f| unsafe { f() } != 0)
    }
    /// `IsRecording`; false if the table or slot is null.
    pub fn demo_recording(&self) -> bool {
        // SAFETY: engine callback from the demo API table, no arguments.
        self.demo_api().and_then(|d| d.IsRecording).is_some_and(|f| unsafe { f() } != 0)
    }

    /// The effects API (`R_TempModel`, `CL_AllocDlight` etc.), if the engine provides one. Only
    /// slots of the 2007 SDK `efx_api_s` are called; that table is a prefix of the FWGS one
    /// bindgen generates.
    pub fn efx_api(&self) -> Option<&'static efx_api_s> {
        // SAFETY: the engine sets `pEfxAPI` (or leaves it null) before `Initialize` and keeps the table for the process.
        unsafe { self.f.pEfxAPI.as_ref() }
    }

    /// `EV_PlayerTrace` from `start` to `end` against the world and the other players (studio
    /// boxes, hull 2), as TS's crosshair ID does (`DrawCrosshairID` 1004acc0). Returns
    /// (fraction, endpos, `EV_IndexFromTrace`), or `None` if the engine has no event API.
    /// Built on [`render::with_solid_players`] and [`render::player_trace`], the single
    /// wrappers for those calls.
    ///
    /// As TS, `ignore_pe` is the local player's entity index and `EV_IndexFromTrace` is called
    /// whether or not the trace hit. `ignore_pe` is an index into the physent list (world, then
    /// the frame's solid non-player entities, then the other players), so this skips whatever
    /// sits at that slot, not the local player, who `EV_SetSolidPlayers` already leaves out. A
    /// miss leaves `tr.ent` at -1, for which FWGS returns -1 (no entity). See
    /// docs/implementation/client-hud.md §5.
    pub fn crosshair_trace(&self, start: [f32; 3], end: [f32; 3], local_index: i32) -> Option<(f32, [f32; 3], i32)> {
        self.event_api()?;
        Some(render::with_solid_players(local_index - 1, || {
            let mut tr = render::player_trace(start, end, 2, PM_STUDIO_BOX, local_index);
            let ent = render::index_from_trace(&mut tr);
            (tr.fraction, tr.endpos, ent)
        }))
    }
}

impl ClEngine {
    /// The triangle API (`WorldToScreen` etc.), if the engine provides one. Only slots of the
    /// 2007 SDK `triangleapi_s` are called; that table is a prefix of the FWGS one bindgen
    /// generates.
    pub fn tri_api(&self) -> Option<&'static triangleapi_s> {
        // SAFETY: the engine sets `pTriAPI` (or leaves it null) before `Initialize` and keeps the table for the process.
        unsafe { self.f.pTriAPI.as_ref() }
    }

    /// Triangle API `WorldToScreen`: the single wrapper for it. Returns the normalised device
    /// coordinates (-1..1, y up) of a world point, or `None` when it is z-clipped (behind the
    /// view) or the engine left the table or slot null. Not bounds-checked: the laser dots
    /// (`DrawLaserDots`) are converted to pixels without a screen check.
    pub fn world_to_screen_ndc(&self, world: [f32; 3]) -> Option<[f32; 2]> {
        let f = self.tri_api()?.WorldToScreen?;
        let mut out = [0.0f32; 3];
        // SAFETY: engine callback from the triangle API table, called with a 3-float input and a 3-float output buffer.
        let clipped = unsafe { f(world.as_ptr(), out.as_mut_ptr()) };
        (clipped == 0).then_some([out[0], out[1]])
    }

    /// TS `WorldToScreen` (100579c0): screen pixel position of a world point, or `None` when it
    /// is behind the view or off screen.
    pub fn world_to_screen(&self, world: [f32; 3], screen: &Screen) -> Option<(i32, i32)> {
        let [nx, ny] = self.world_to_screen_ndc(world)?;
        let x = ((1.0 + nx) * screen.width as f32 * 0.5) as i32;
        let y = ((1.0 - ny) * screen.height as f32 * 0.5) as i32;
        (0..screen.width).contains(&x).then_some(())?;
        (0..screen.height).contains(&y).then_some((x, y))
    }
}

/// `SND_CHANGE_VOL` for [`ClEngine::ev_play_sound`].
pub const SND_CHANGE_VOL: i32 = 1 << 6;

impl ClEngine {
    /// `EV_PlaySound(ent, origin, channel, sample, volume, attenuation, flags, pitch)`: the
    /// single wrapper for it. `ent` 0 plays at `origin`, -1 is a positional sound with no
    /// entity. Every engine fills the slot; if the table or slot is null the sound is dropped
    /// rather than panicking, since callers are fire-and-forget.
    #[allow(clippy::too_many_arguments)]
    pub fn ev_play_sound(&self, ent: i32, origin: [f32; 3], channel: i32, sample: &str, volume: f32, attn: f32, flags: i32, pitch: i32) {
        let Some(f) = self.event_api().and_then(|a| a.EV_PlaySound) else { return };
        let s = to_cstring(sample);
        let mut o = origin;
        // SAFETY: engine callback from the event API table, called with a 3-float origin and a NUL-terminated sample that outlive the call.
        unsafe { f(ent, o.as_mut_ptr(), channel, s.as_ptr(), volume, attn, flags, pitch) };
    }
}

/// A `titles.txt` entry from `ClEngine::text_message`. Derefs to the engine struct; the text
/// is read through [`TextMessage::message`].
#[derive(Clone, Copy)]
pub struct TextMessage(&'static client_textmessage_t);

impl std::ops::Deref for TextMessage {
    type Target = client_textmessage_t;
    fn deref(&self) -> &client_textmessage_t {
        self.0
    }
}

impl TextMessage {
    /// The message text (empty if the engine left it null).
    pub fn message(&self) -> &'static CStr {
        // SAFETY: the engine fills pMessage with a NUL-terminated string that lives with the entry.
        unsafe { crate::util::cstr(self.0.pMessage) }
    }
}
