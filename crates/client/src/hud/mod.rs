//! HUD: message hooks, HUD state, and one immediate-mode drawing pass. Owner: HUD/UI work
//! package. Reference: decomp docs/client/architecture-hud.md §3-4, §7.

use goldsrc::client::engine;
use goldsrc::msg::MsgReader;
use std::ffi::{c_char, c_int, c_void};

pub fn init() {
    engine().hook_user_msg(c"TSHealth", Some(msg_ts_health));
}

pub fn vid_init() {}
pub fn reset() {}

unsafe extern "C" fn msg_ts_health(_name: *const c_char, size: c_int, buf: *mut c_void) -> c_int {
    let mut r = unsafe { MsgReader::from_raw(buf, size) };
    let hp = r.byte();
    if !r.bad {
        crate::state::with(|s| s.health = hp as i32);
    }
    1
}

pub fn redraw(_time: f32, intermission: bool) {
    if intermission {
        return;
    }
    let e = engine();
    let (w, h) = e.screen_size();
    let hp = crate::state::with(|s| s.health);
    e.draw_string(16, h - 48, [1.0, 0.7, 0.1], "The Specialists (Rust)");
    e.draw_string(16, h - 28, [1.0, 1.0, 1.0], &format!("Health: {hp}"));
    let (cx, cy) = (w / 2, h / 2);
    e.fill_rgba(cx - 6, cy, 13, 1, [255, 160, 0, 255]);
    e.fill_rgba(cx, cy - 6, 1, 13, [255, 160, 0, 255]);
}
