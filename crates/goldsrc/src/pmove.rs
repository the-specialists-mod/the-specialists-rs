//! Safe calls through the engine callbacks in `playermove_t`, for the shared player-movement
//! code (server `pfnPM_Move` and client `HUD_PlayerMove` both hand the game a `playermove_t`).
//!
//! The engine fills the callback slots and `movevars` before every call and keeps them valid for
//! its duration, which is what makes these wrappers sound to call from the movement code.

use crate::sys::*;
use std::ffi::{CStr, CString};

/// `modtype_t::mod_brush` (com_model.h).
const MOD_BRUSH: i32 = 0;

/// Engine callbacks on `playermove_t`. A missing (null) callback behaves as noted per method.
pub trait PlayerMoveExt {
    /// `movevars` (gravity, friction, speeds). Panics if the engine left it null.
    fn movevars(&self) -> &movevars_s;
    /// `PM_PlayerTrace(start, end, PM_NORMAL, -1)`. Panics if the callback is missing.
    fn player_trace(&self, start: [f32; 3], end: [f32; 3]) -> pmtrace_t;
    /// `PM_PointContents`, optionally returning the true contents. Panics if missing.
    fn point_contents(&self, p: [f32; 3], truecont: Option<&mut i32>) -> i32;
    /// `PM_TestPlayerPosition`; -1 (free) without the callback.
    fn test_player_position(&self, pos: [f32; 3], tr: Option<&mut pmtrace_t>) -> i32;
    /// `PM_PlaySound`; nothing without the callback.
    fn play_sound(&self, channel: i32, sample: &CStr, volume: f32, attn: f32, flags: i32, pitch: i32);
    /// `RandomLong`; `lo` without the callback.
    fn random_long(&self, lo: i32, hi: i32) -> i32;
    /// `PM_Info_ValueForKey(physinfo, key)`; empty without the callback.
    fn physinfo_value(&self, key: &CStr) -> String;
    /// `COM_LoadFile` + `COM_FreeFile`: the whole file, or `None`.
    fn load_file(&self, path: &CStr) -> Option<Vec<u8>>;
    /// `Con_DPrintf("%s", msg)`; nothing without the callback.
    fn dprint(&self, msg: &str);
    /// `Sys_FloatTime`; 0 without the callback.
    fn float_time(&self) -> f64;
    /// `PM_StuckTouch(hitent, trace)`; nothing without the callback.
    fn stuck_touch(&self, hitent: i32, tr: &mut pmtrace_t);
    /// `PM_TraceLine(start, end, flags, usehull, ignore_pe)`, copied out; `None` without the
    /// callback or when the engine returns no trace.
    fn trace_line(&self, start: [f32; 3], end: [f32; 3], flags: i32, usehull: i32, ignore_pe: i32) -> Option<pmtrace_t>;
    /// For `moveents[i]` with a brush model: the contents of its clip hull at `point` (offset by
    /// the hull's origin, as `PM_Ladder` does). `None` for non-brush models, a null model, or
    /// missing callbacks.
    fn moveent_hull_contents(&self, i: usize, point: [f32; 3]) -> Option<i32>;
    /// `PM_GetModelBounds` of `moveents[i]`'s model; zeros without the callback or model.
    fn moveent_model_bounds(&self, i: usize) -> ([f32; 3], [f32; 3]);
    /// `PM_TraceModel(&moveents[i], start, end, &trace)`; `None` without the callback.
    fn moveent_trace_model(&self, i: usize, start: [f32; 3], end: [f32; 3]) -> Option<trace_t>;
    /// `PM_TraceTexture(ground, start, end)`: the texture name hit, copied out; `None` without
    /// the callback or when nothing was hit.
    fn trace_texture(&self, ground: i32, start: [f32; 3], end: [f32; 3]) -> Option<Vec<u8>>;
}

impl PlayerMoveExt for playermove_t {
    fn movevars(&self) -> &movevars_s {
        assert!(!self.movevars.is_null(), "playermove_t::movevars is null");
        // SAFETY: the engine points movevars at its movevars_t for the duration of the call.
        unsafe { &*self.movevars }
    }

    fn player_trace(&self, start: [f32; 3], end: [f32; 3]) -> pmtrace_t {
        let (mut s, mut e) = (start, end);
        let f = self.PM_PlayerTrace.expect("PM_PlayerTrace");
        // SAFETY: engine callback; the vectors are local 3-float arrays.
        unsafe { f(s.as_mut_ptr(), e.as_mut_ptr(), PM_NORMAL as i32, -1) }
    }

    fn point_contents(&self, p: [f32; 3], truecont: Option<&mut i32>) -> i32 {
        let mut p = p;
        let f = self.PM_PointContents.expect("PM_PointContents");
        let tc = truecont.map_or(std::ptr::null_mut(), |r| r as *mut i32);
        // SAFETY: engine callback; `tc` is null or a live &mut i32.
        unsafe { f(p.as_mut_ptr(), tc) }
    }

    fn test_player_position(&self, pos: [f32; 3], tr: Option<&mut pmtrace_t>) -> i32 {
        let mut pos = pos;
        let Some(f) = self.PM_TestPlayerPosition else { return -1 };
        let t = tr.map_or(std::ptr::null_mut(), |r| r as *mut pmtrace_t);
        // SAFETY: engine callback; `t` is null or a live &mut pmtrace_t.
        unsafe { f(pos.as_mut_ptr(), t) }
    }

    fn play_sound(&self, channel: i32, sample: &CStr, volume: f32, attn: f32, flags: i32, pitch: i32) {
        if let Some(f) = self.PM_PlaySound {
            // SAFETY: engine callback; `sample` is NUL-terminated.
            unsafe { f(channel, sample.as_ptr(), volume, attn, flags, pitch) }
        }
    }

    fn random_long(&self, lo: i32, hi: i32) -> i32 {
        match self.RandomLong {
            // SAFETY: engine callback.
            Some(f) => unsafe { f(lo, hi) },
            None => lo,
        }
    }

    fn physinfo_value(&self, key: &CStr) -> String {
        let Some(f) = self.PM_Info_ValueForKey else { return String::new() };
        // SAFETY: engine callback on the NUL-terminated physinfo buffer; it returns null or a
        // string valid until the next info call, which is copied out immediately.
        unsafe {
            let v = f(self.physinfo.as_ptr(), key.as_ptr());
            crate::util::cstr(v).to_string_lossy().into_owned()
        }
    }

    fn load_file(&self, path: &CStr) -> Option<Vec<u8>> {
        let (Some(load), Some(free)) = (self.COM_LoadFile, self.COM_FreeFile) else { return None };
        let mut len: i32 = 0;
        // SAFETY: engine callbacks; the buffer is copied before it's freed.
        unsafe {
            let data = load(path.as_ptr(), 5, &mut len);
            if data.is_null() {
                return None;
            }
            if len <= 0 {
                if let Some(size) = self.COM_FileSize {
                    len = size(path.as_ptr());
                }
            }
            let bytes = std::slice::from_raw_parts(data.cast::<u8>(), len.max(0) as usize).to_vec();
            free(data.cast());
            Some(bytes)
        }
    }

    fn dprint(&self, msg: &str) {
        if let Some(print) = self.Con_DPrintf {
            let m = CString::new(msg.replace('\0', "")).unwrap_or_default();
            // SAFETY: engine printf with a "%s" format and one NUL-terminated argument.
            unsafe { print(c"%s".as_ptr(), m.as_ptr()) };
        }
    }

    fn float_time(&self) -> f64 {
        // SAFETY: engine callback.
        self.Sys_FloatTime.map_or(0.0, |f| unsafe { f() })
    }

    fn stuck_touch(&self, hitent: i32, tr: &mut pmtrace_t) {
        if let Some(f) = self.PM_StuckTouch {
            // SAFETY: engine callback; `tr` is a live &mut pmtrace_t.
            unsafe { f(hitent, tr) };
        }
    }

    fn trace_line(&self, start: [f32; 3], end: [f32; 3], flags: i32, usehull: i32, ignore_pe: i32) -> Option<pmtrace_t> {
        let (mut s, mut e) = (start, end);
        let f = self.PM_TraceLine?;
        // SAFETY: engine callback; it returns null or a pointer to a static trace, copied at once.
        unsafe {
            let tr = f(s.as_mut_ptr(), e.as_mut_ptr(), flags, usehull, ignore_pe);
            tr.as_ref().copied()
        }
    }

    fn moveent_hull_contents(&self, i: usize, point: [f32; 3]) -> Option<i32> {
        let pe = self.moveents.get(i)?;
        let (model_type, hull_for_bsp, hull_contents) = (self.PM_GetModelType?, self.PM_HullForBsp?, self.PM_HullPointContents?);
        if pe.model.is_null() {
            return None;
        }
        // SAFETY: engine callbacks on one of the engine's own physents; `hull_t` starts with
        // two pointers (clipnodes, planes) followed by `int firstclipnode` (SDK `hull_t`).
        unsafe {
            if model_type(pe.model) != MOD_BRUSH {
                return None;
            }
            let mut offset = [0.0f32; 3];
            let hull = hull_for_bsp((pe as *const physent_t).cast_mut(), offset.as_mut_ptr());
            if hull.is_null() {
                return None;
            }
            let firstclipnode = hull.cast::<u8>().add(2 * size_of::<*const ()>()).cast::<i32>().read_unaligned();
            let mut test = [point[0] - offset[0], point[1] - offset[1], point[2] - offset[2]];
            Some(hull_contents(hull.cast(), firstclipnode, test.as_mut_ptr()))
        }
    }

    fn moveent_model_bounds(&self, i: usize) -> ([f32; 3], [f32; 3]) {
        let (mut mins, mut maxs) = ([0.0f32; 3], [0.0f32; 3]);
        if let (Some(pe), Some(f)) = (self.moveents.get(i), self.PM_GetModelBounds) {
            if !pe.model.is_null() {
                // SAFETY: engine callback on the engine's own model pointer.
                unsafe { f(pe.model, mins.as_mut_ptr(), maxs.as_mut_ptr()) };
            }
        }
        (mins, maxs)
    }

    fn moveent_trace_model(&self, i: usize, start: [f32; 3], end: [f32; 3]) -> Option<trace_t> {
        let (pe, f) = (self.moveents.get(i)?, self.PM_TraceModel?);
        let (mut s, mut e) = (start, end);
        let mut tr = trace_t::default();
        // SAFETY: engine callback on one of the engine's own physents, writing into a local trace.
        unsafe { f((pe as *const physent_t).cast_mut(), s.as_mut_ptr(), e.as_mut_ptr(), &mut tr) };
        Some(tr)
    }

    fn trace_texture(&self, ground: i32, start: [f32; 3], end: [f32; 3]) -> Option<Vec<u8>> {
        let f = self.PM_TraceTexture?;
        let (mut s, mut e) = (start, end);
        // SAFETY: engine callback; it returns null or a NUL-terminated texture name, copied at once.
        unsafe {
            let name = f(ground, s.as_mut_ptr(), e.as_mut_ptr());
            (!name.is_null()).then(|| CStr::from_ptr(name).to_bytes().to_vec())
        }
    }
}
