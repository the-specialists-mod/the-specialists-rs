//! TS 2.1's HLTV spectator controls (issue #177): the SDK 2.2 `CHudSpectator` key handling
//! TS 3.0 doesn't have. 2.1's element (`0x102daaa0`) keeps a main mode (`+0x14`, 1..5) and an
//! inset mode (`+0x18`, 0..3) of its own and maps the main mode onto the observer mode
//! (`g_iUser1`, `0x10320638`): 2 the director's locked view (4), 3 roaming (3), 4 in-eye (5),
//! 5 the locked view with the map view reset to the overview file's.
//!
//! * `HandleButtonsDown` (`0x1004b200`, from the `+` commands with their `IN_*` bit): HLTV
//!   viewers only, at most every 0.3 s. Jump steps the main mode (past 5 back to 2), duck the
//!   inset mode (past 3 back to 0), attack/attack2 pick the next/previous player, forward/back
//!   and the strafes set the map view's zoom and pan deltas.
//! * `HandleButtonsUp` (`0x1004b4c0`, from `-forward`/`-back`/`-moveleft`/`-moveright`) clears
//!   the deltas, which `Draw` (`0x1004a8d0`) applies every HUD frame outside roaming.
//! * `FindNextPlayer` (`0x1004b090`) and `SetModes` (`0x1004b530`), also from `specmode` and the
//!   director reset (`DirectorEvent` `0x1004af10`).
//!
//! * `V_CalcSpectatorRefdef` (`0x100b96a0`): the views the main and inset modes select (issue
//!   #181), `V_GetInEyePos` (`0x100b9510`, issue #184) and `GetMapPosition` (`0x1004c990`).
//! * The rest of `Draw`: the map-mode name tags.
//!
//! The engine side is [`HltvHost`] (`ts_client::spectator`), see `docs/implementation/ts21.md`.

use goldsrc::ext::render::{Vec3, vec3 as v};
use goldsrc::sys::{IN_ATTACK, IN_ATTACK2, IN_BACK, IN_DUCK, IN_FORWARD, IN_JUMP, IN_MOVELEFT, IN_MOVERIGHT, ref_params_t};
use ts_client::spectator::HltvHost;

/// 2.1's `CHudSpectator` fields the port's (3.0's) spectator doesn't have, and the camera jump
/// globals.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Hltv {
    /// `m_iMainMode` (`+0x14`).
    pub main: i32,
    /// `m_iInsetMode` (`+0x18`).
    pub inset: i32,
    /// `m_flNextObserverInput` (`+0xc9c`).
    pub next_input: f32,
    /// `m_zoomDelta` (`+0xca0`) and `m_moveDelta` (`+0xca4`).
    pub zoom_delta: f32,
    pub move_delta: f32,
    /// `vJumpOrigin` (`0x10321ff0`) and `vJumpAngles` (`0x10321fe0`): kept between jumps, which
    /// the in-eye one only partly overwrites for a bad target.
    pub jump_origin: Vec3,
    pub jump_angles: Vec3,
}

impl Hltv {
    pub const NEW: Hltv =
        Hltv { main: 0, inset: 0, next_input: 0.0, zoom_delta: 0.0, move_delta: 0.0, jump_origin: [0.0; 3], jump_angles: [0.0; 3] };

    /// `CHudSpectator::Init` (`0x1004a2b0`) zeroes the input timer, the target, the deltas and
    /// both modes; the director reset (`0x1004af10`, `DRC_ACTIVE`) the timer and modes, before its
    /// `SetModes(2, 0)`.
    pub fn reset(&mut self, init: bool) {
        self.main = 0;
        self.inset = 0;
        self.next_input = 0.0;
        if init {
            self.zoom_delta = 0.0;
            self.move_delta = 0.0;
        }
    }

    /// A `+` (`down`) or `-` command's buttons.
    pub fn buttons(&mut self, host: &mut dyn HltvHost, buttons: i32, down: bool) {
        if down { self.buttons_down(host, buttons) } else { self.buttons_up(host, buttons) }
    }

    /// `HandleButtonsDown` (`0x1004b200`).
    pub fn buttons_down(&mut self, host: &mut dyn HltvHost, buttons: i32) {
        let now = host.time();
        // `fld [+0xc9c]; fcomp time; and ah, 0x41`: on only while the stored time is below,
        // equal to or unordered with now; then `IsSpectateOnly() == 1`.
        if self.next_input as f64 > now || !host.spectate_only() {
            return;
        }
        let b = buttons as u32;
        let (mut main, mut inset) = (self.main, self.inset);
        if b & IN_JUMP != 0 {
            main += 1;
            if main > 5 {
                main = 2;
            }
        }
        if b & IN_DUCK != 0 {
            inset += 1;
            if inset > 3 {
                inset = 0;
            }
        }
        if b & (IN_ATTACK | IN_ATTACK2) != 0 {
            // `0x1004b29c`: chase free, whatever the main mode.
            host.set_observer_mode(2);
            self.find_next_player(host, b & IN_ATTACK2 != 0);
            let target = host.locked_target();
            if self.main == 3
                && target != 0
                && let Some((origin, angles)) = host.entity_camera(target)
            {
                self.jump(host, origin, angles);
            }
        }
        // `0x3c23d70a` = 0.01f, `0xc1400000` = -12.
        if b & IN_FORWARD != 0 {
            self.zoom_delta = 0.01;
        }
        if b & IN_BACK != 0 {
            self.zoom_delta = -0.01;
        }
        if b & IN_MOVELEFT != 0 {
            self.move_delta = -12.0;
        }
        if b & IN_MOVERIGHT != 0 {
            self.move_delta = 12.0;
        }
        if main != self.main || inset != self.inset {
            self.set_modes(host, main, inset);
        }
        // `fadd qword 0.3` on the `double` time, stored as a float.
        self.next_input = (now + 0.3) as f32;
    }

    /// `HandleButtonsUp` (`0x1004b4c0`).
    pub fn buttons_up(&mut self, host: &dyn HltvHost, buttons: i32) {
        if !host.spectate_only() {
            return;
        }
        let b = buttons as u32;
        if b & (IN_FORWARD | IN_BACK) != 0 {
            self.zoom_delta = 0.0;
        }
        if b & (IN_MOVELEFT | IN_MOVERIGHT) != 0 {
            self.move_delta = 0.0;
        }
    }

    /// `FindNextPlayer` (`0x1004b090`): the next (or previous) active player after the target
    /// (from 1 without one), over the fixed 1..64, back round to the start. Outside roaming the
    /// observer target follows.
    pub fn find_next_player(&mut self, host: &mut dyn HltvHost, reverse: bool) {
        let start = match host.locked_target() {
            0 => 1,
            t => t,
        };
        host.set_locked_target(0);
        let mut i = start;
        loop {
            i += if reverse { -1 } else { 1 };
            if i > 64 {
                i = 1;
            }
            if i < 1 {
                i = 64;
            }
            if host.active_player(i) {
                host.set_locked_target(i);
                break;
            }
            if i == start {
                break;
            }
        }
        let target = host.locked_target();
        if target == 0 {
            host.print("No observer targets.\n");
            return;
        }
        if self.main != 3 {
            host.set_observer_target(target, 0);
        }
        let name = host.player_name(target);
        host.print(&format!("Now Tracking {name}\n"));
    }

    /// `SetModes` (`0x1004b530`).
    pub fn set_modes(&mut self, host: &mut dyn HltvHost, mode: i32, inset: i32) {
        if !(1..=5).contains(&mode) {
            host.print("Invalid spectator mode.\n");
            return;
        }
        if inset != self.inset {
            self.inset = inset;
            host.set_inset(inset);
            host.center_text(&format!("#Spec_Mode_Inset{inset}"));
        }
        if mode != self.main {
            match mode {
                2 => host.set_observer_mode(4),
                3 => {
                    host.set_observer_mode(3);
                    // From the locked view: jump to the current view (`v_origin`, `v_angles`).
                    if self.main == 2 {
                        let (origin, angles) = host.view_camera();
                        self.jump(host, origin, angles);
                    }
                    // From in-eye: to the target's eyes (`V_GetInEyePos` `0x100b9510`).
                    if self.main == 4 {
                        let (mut origin, mut angles) = (self.jump_origin, self.jump_angles);
                        in_eye_pos(host, host.locked_target(), &mut origin, &mut angles);
                        self.jump(host, origin, angles);
                    }
                    host.set_locked_target(0);
                    host.set_observer_target(0, 0);
                }
                4 => host.set_observer_mode(5),
                5 => {
                    host.set_observer_mode(4);
                    let (zoom, origin) = host.overview();
                    host.set_map_view(zoom, origin);
                }
                _ => {}
            }
            self.main = mode;
            host.center_text(&format!("#Spec_Mode{mode}"));
            if host.observer_mode() == 4 && host.locked_target() == 0 {
                let (primary, secondary) = host.last_director();
                if host.active_player(primary) {
                    host.set_locked_target(primary);
                    host.set_observer_target(primary, secondary);
                }
            }
            host.set_crosshair(self.main == 3 || self.main == 4);
        }
        // The map inset needs a main view that isn't the map, and the map main view only takes
        // the camera insets.
        if self.main != 5 && (self.inset == 1 || self.inset == 2) {
            self.set_modes(host, self.main, 3);
        }
        if self.main == 5 && self.inset == 3 {
            self.set_modes(host, self.main, 0);
        }
        host.update_panel();
    }

    /// The head of `CHudSpectator::Draw` (`0x1004a8d0`): outside roaming, the zoom delta moves
    /// the map zoom (`+0x954`) within 0.5..3 and the move delta pans the map origin (`+0x958`)
    /// along the map angles' right vector.
    pub fn draw(&self, host: &mut dyn HltvHost) {
        if !host.spectate_only() {
            return;
        }
        let (mut zoom, mut origin, angles) = host.map_view();
        if self.zoom_delta != 0.0 && self.main != 3 {
            // `3.0 < z` then `z < 0.5`: a NaN stays, as with `clamp`.
            zoom = (zoom + self.zoom_delta).clamp(0.5, 3.0);
        }
        if self.move_delta != 0.0 && self.main != 3 {
            let right = scale(normalize(v::angle_vectors(angles).1), self.move_delta);
            origin = [origin[0] + right[0], origin[1] + right[1], origin[2] + right[2]];
        }
        host.set_map_view(zoom, origin);
    }

    /// The rest of `Draw` (`0x1004aac7..0x1004adbc`): the name tags, only in the map main mode
    /// (5) and with `cl_drawnames` on (`test ah, 0x44`: a NaN counts as on), after
    /// `GetAllPlayersInfo` (the port's player info is the engine's, always current). A tag at
    /// `[2] < 0` is off (a NaN draws). With an inset mode, tags strictly inside the inset window
    /// are skipped, the window edges `int × (ScreenWidth / 640f)` on the x87 (a NaN position
    /// draws). The name (`sprintf("%s")`, raw bytes) goes at `ftol(x − strlen × 3)`, `ftol(y)`.
    pub fn draw_names(&self, host: &mut dyn HltvHost) {
        if !host.spectate_only() || self.main != 5 || host.draw_names() == 0.0 {
            return;
        }
        let (w, h) = host.screen_size();
        let (sx, sy) = (w as f64 / 640.0, h as f64 / 480.0);
        let [ix, iy, iw, ih] = host.overview_inset();
        for (i, t) in host.name_tags().into_iter().enumerate().take(64) {
            if t[2] < 0.0 {
                continue;
            }
            let (x, y) = (t[0] as f64, t[1] as f64);
            if self.inset != 0
                && ix as f64 * sx < x
                && iy as f64 * sy < y
                && x < ix.wrapping_add(iw) as f64 * sx
                && y < iy.wrapping_add(ih) as f64 * sy
            {
                continue;
            }
            let index = i as i32 + 1;
            let name = host.player_name_bytes(index);
            let color = match host.player_team(index) {
                1 => [0.4, 0.4, 1.0],
                2 => [1.0, 0.4, 0.4],
                _ => [0.8, 0.8, 0.8],
            };
            let n = name.len() as i32 * 3;
            host.draw_text(ftol(x - n as f64), ftol(y), color, &name);
        }
    }

    /// The mode passes of `V_CalcSpectatorRefdef` (`0x100b9f95..0x100ba30a`). The view model is
    /// always hidden (no in-eye gun). The main view follows the main mode, not the observer
    /// mode: 4 is in-eye of the spectator's own target (`+0x950`), 5 the map (`onlyClientDraw`),
    /// anything else clears `onlyClientDraw`; any inset mode asks for the inset pass. The inset
    /// pass takes the overview file's window, each value `ftol(int × (screen / 640f))` on the
    /// x87, and shows in-eye (2) or the map (3).
    pub fn calc_refdef(&self, host: &mut dyn HltvHost, p: &mut ref_params_t) {
        host.clear_view_model();
        if p.nextView == 0 {
            match self.main {
                4 => in_eye_pos(host, host.locked_target(), &mut p.vieworg, &mut p.viewangles),
                5 => {
                    p.onlyClientDraw = 1;
                    map_camera(host, p);
                }
                _ => p.onlyClientDraw = 0,
            }
            if self.inset != 0 {
                p.nextView = 1;
            }
            host.set_inset_pass(false);
            return;
        }
        let (w, h) = host.screen_size();
        let (sx, sy) = (w as f64 / 640.0, h as f64 / 480.0);
        let [x, y, iw, ih] = host.overview_inset();
        p.viewport = [ftol(x as f64 * sx), ftol(y as f64 * sy), ftol(iw as f64 * sx), ftol(ih as f64 * sy)];
        p.nextView = 0;
        p.onlyClientDraw = 0;
        match self.inset {
            2 => in_eye_pos(host, host.locked_target(), &mut p.vieworg, &mut p.viewangles),
            3 => {
                p.onlyClientDraw = 1;
                map_camera(host, p);
            }
            _ => {}
        }
        host.set_inset_pass(true);
    }

    /// The camera `DrawOverviewEntities` (`0x1004cb50`) marks while an inset mode shows: with
    /// the in-eye inset (2) or main mode (4) it is first moved to `V_GetInEyePos` (`0x100b9510`)
    /// of the spectator's target (`+0x950`; `0x1004d6b3..0x1004d6ca`). 3.0 tests its observer
    /// mode instead and uses its own `V_GetInEyePos`.
    pub fn overview_camera(&self, host: &dyn HltvHost, origin: &mut Vec3, angles: &mut Vec3) {
        if self.inset == 2 || self.main == 4 {
            in_eye_pos(host, host.locked_target(), origin, angles);
        }
    }

    fn jump(&mut self, host: &mut dyn HltvHost, origin: Vec3, angles: Vec3) {
        self.jump_origin = origin;
        self.jump_angles = angles;
        host.jump_camera(origin, angles);
    }
}

/// `V_GetInEyePos` (`0x100b9510`): the target's origin and angles, the pitch × the `double` −π
/// (`0x10278b28`); a non-solid target rolls 80 and drops 8, a solid one rises 16 ducked
/// (`usehull == 1`) or 28. Unlike 3.0's there is no `NO_VIEW` stunt test. A target that fails
/// the entity tests leaves `org` and `ang` alone.
pub fn in_eye_pos(host: &dyn HltvHost, target: i32, org: &mut Vec3, ang: &mut Vec3) {
    let Some(t) = host.in_eye_target(target) else { return };
    *org = t.origin;
    *ang = t.angles;
    ang[0] = (ang[0] as f64 * -std::f64::consts::PI) as f32;
    if t.solid == 0 {
        ang[2] = 80.0;
        org[2] -= 8.0;
    } else if t.usehull == 1 {
        org[2] += 16.0;
    } else {
        org[2] += 28.0;
    }
}

/// The map camera of both passes (`0x100b9fd8..0x100ba0d2`): `cl_viewangles` with the pitch
/// `pitch / 90f × 38.75f + 51.25f` on the x87, which become the map angles (`+0x964`) and the
/// view angles, and `GetMapPosition`'s origin.
fn map_camera(host: &mut dyn HltvHost, p: &mut ref_params_t) {
    let mut angles = p.cl_viewangles;
    angles[0] = (angles[0] as f64 / 90.0 * 38.75 + 51.25) as f32;
    host.set_map_angles(angles);
    let (zoom, origin, _) = host.map_view();
    p.vieworg = map_position(zoom, origin, angles);
    p.viewangles = angles;
}

/// `GetMapPosition` (`0x1004c990`): the map origin with its height `(90 − pitch) / 90 × z` on the
/// x87, plus the normalised forward of the map angles scaled by the float `−(4096f / zoom)`.
pub fn map_position(zoom: f32, origin: Vec3, angles: Vec3) -> Vec3 {
    let z = ((90.0 - angles[0] as f64) / 90.0 * origin[2] as f64) as f32;
    let back = scale(normalize(v::angle_vectors(angles).0), -(4096.0 / zoom as f64) as f32);
    [origin[0] + back[0], origin[1] + back[1], z + back[2]]
}

/// 2.1's `__ftol` (`0x100bb9e0`): `fistp qword` with truncation, keeping the low dword; NaN and
/// values beyond the 64-bit range give the indefinite `0x8000000000000000`, so 0.
pub fn ftol(v: f64) -> i32 {
    let t = v.trunc();
    if (-9.223372036854776e18..9.223372036854776e18).contains(&t) { t as i64 as i32 } else { 0 }
}

/// `VectorNormalize` (`0x100981d0`): the squared length and its root stored as floats, the
/// components scaled by the float `1 / length` unless the length is exactly 0 (a NaN scales).
pub fn normalize(a: Vec3) -> Vec3 {
    let (x, y, z) = (a[0] as f64, a[1] as f64, a[2] as f64);
    let sq = ((x * x + y * y) + z * z) as f32;
    let len = (sq as f64).sqrt() as f32;
    if len == 0.0 {
        return a;
    }
    let inv = (1.0 / len as f64) as f32;
    a.map(|c| inv * c)
}

/// `VectorScale` (`0x100982e0`).
fn scale(a: Vec3, s: f32) -> Vec3 {
    a.map(|c| s * c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ts_client::spectator::InEyeTarget;

    /// An HLTV viewer with players 3 and 7 active, recording what the controls did.
    struct Fake {
        time: f64,
        hltv: bool,
        mode: i32,
        target: (i32, i32),
        locked: i32,
        director: (i32, i32),
        inset: i32,
        active: Vec<i32>,
        jumps: Vec<(Vec3, Vec3)>,
        log: Vec<String>,
        crosshair: Option<bool>,
        panel: u32,
        map: (f32, Vec3, Vec3),
        eye: Option<InEyeTarget>,
        inset_window: [i32; 4],
        screen: (i32, i32),
        inset_pass: Option<bool>,
        view_model_cleared: bool,
        tags: Vec<[f32; 3]>,
        drawnames: f32,
        drawn: Vec<(i32, i32, [f32; 3], Vec<u8>)>,
    }

    impl Default for Fake {
        fn default() -> Self {
            Fake {
                time: 10.0,
                hltv: true,
                mode: 0,
                target: (0, 0),
                locked: 0,
                director: (0, 0),
                inset: 0,
                active: vec![3, 7],
                jumps: Vec::new(),
                log: Vec::new(),
                crosshair: None,
                panel: 0,
                map: (1.0, [0.0; 3], [90.0, 0.0, 0.0]),
                eye: None,
                inset_window: [4, 4, 240, 180],
                screen: (640, 480),
                inset_pass: None,
                view_model_cleared: false,
                tags: Vec::new(),
                drawnames: 1.0,
                drawn: Vec::new(),
            }
        }
    }

    impl HltvHost for Fake {
        fn time(&self) -> f64 {
            self.time
        }
        fn spectate_only(&self) -> bool {
            self.hltv
        }
        fn observer_mode(&self) -> i32 {
            self.mode
        }
        fn set_observer_mode(&mut self, mode: i32) {
            self.mode = mode;
        }
        fn set_observer_target(&mut self, target: i32, flags: i32) {
            self.target = (target, flags);
        }
        fn locked_target(&self) -> i32 {
            self.locked
        }
        fn set_locked_target(&mut self, target: i32) {
            self.locked = target;
        }
        fn last_director(&self) -> (i32, i32) {
            self.director
        }
        fn set_inset(&mut self, mode: i32) {
            self.inset = mode;
        }
        fn active_player(&self, index: i32) -> bool {
            self.active.contains(&index)
        }
        fn player_name(&self, index: i32) -> String {
            format!("p{index}")
        }
        fn player_name_bytes(&self, index: i32) -> Vec<u8> {
            format!("p{index}").into_bytes()
        }
        fn player_team(&self, index: i32) -> i32 {
            index % 3
        }
        fn entity_camera(&self, index: i32) -> Option<(Vec3, Vec3)> {
            Some(([index as f32, 0.0, 0.0], [0.0, index as f32, 0.0]))
        }
        fn in_eye_target(&self, target: i32) -> Option<InEyeTarget> {
            let solid = InEyeTarget { origin: [0.0, 0.0, target as f32], angles: [1.0, 2.0, 3.0], solid: 3, usehull: 0 };
            self.eye.or((target != 0).then_some(solid))
        }
        fn view_camera(&self) -> (Vec3, Vec3) {
            ([5.0; 3], [6.0; 3])
        }
        fn jump_camera(&mut self, origin: Vec3, angles: Vec3) {
            self.jumps.push((origin, angles));
        }
        fn print(&mut self, text: &str) {
            self.log.push(text.to_string());
        }
        fn center_text(&mut self, key: &str) {
            self.log.push(key.to_string());
        }
        fn set_crosshair(&mut self, on: bool) {
            self.crosshair = Some(on);
        }
        fn update_panel(&mut self) {
            self.panel += 1;
        }
        fn overview(&self) -> (f32, Vec3) {
            (2.5, [100.0, 200.0, 0.0])
        }
        fn map_view(&self) -> (f32, Vec3, Vec3) {
            self.map
        }
        fn set_map_view(&mut self, zoom: f32, origin: Vec3) {
            self.map.0 = zoom;
            self.map.1 = origin;
        }
        fn set_map_angles(&mut self, angles: Vec3) {
            self.map.2 = angles;
        }
        fn overview_inset(&self) -> [i32; 4] {
            self.inset_window
        }
        fn screen_size(&self) -> (i32, i32) {
            self.screen
        }
        fn set_inset_pass(&mut self, inset: bool) {
            self.inset_pass = Some(inset);
        }
        fn clear_view_model(&mut self) {
            self.view_model_cleared = true;
        }
        fn name_tags(&self) -> Vec<[f32; 3]> {
            self.tags.clone()
        }
        fn draw_names(&self) -> f32 {
            self.drawnames
        }
        fn draw_text(&mut self, x: i32, y: i32, color: [f32; 3], text: &[u8]) {
            self.drawn.push((x, y, color, text.to_vec()));
        }
    }

    fn press(h: &mut Hltv, f: &mut Fake, b: u32) {
        f.time += 0.5;
        h.buttons_down(f, b as i32);
    }

    /// The director reset (`0x1004af10`): `SetModes(2, 0)` from main mode 0 is the locked view on
    /// the director's target when it is active.
    #[test]
    fn director_reset_selects_the_locked_view() {
        let (mut h, mut f) = (Hltv::NEW, Fake { director: (7, 9), ..Fake::default() });
        h.reset(false);
        h.set_modes(&mut f, 2, 0);
        assert_eq!((h.main, f.mode, f.locked, f.target), (2, 4, 7, (7, 9)));
        assert_eq!(f.log, ["#Spec_Mode2"]);
        assert_eq!((f.crosshair, f.panel), (Some(false), 1));
        f.log.clear();
        h.set_modes(&mut f, 6, 0);
        assert_eq!(f.log, ["Invalid spectator mode.\n"]);
        assert_eq!(f.panel, 1, "an invalid mode returns before the panel update");
    }

    /// `0x1004b24c..0x1004b285`, `0x1004b40f`: jump steps 2..5 and wraps to 2, duck 0..3; one
    /// press per 0.3 s, HLTV viewers only.
    #[test]
    fn jump_and_duck_step_the_modes() {
        let (mut h, mut f) = (Hltv::NEW, Fake::default());
        h.set_modes(&mut f, 2, 0);
        press(&mut h, &mut f, IN_JUMP);
        assert_eq!((h.main, f.mode, f.crosshair), (3, 3, Some(true)), "roaming");
        assert_eq!(f.jumps, [([5.0; 3], [6.0; 3])], "from the locked view to the current view");
        press(&mut h, &mut f, IN_JUMP);
        assert_eq!((h.main, f.mode), (4, 5), "in-eye");
        press(&mut h, &mut f, IN_JUMP);
        assert_eq!((h.main, f.mode), (5, 4), "the map");
        assert_eq!((f.map.0, f.map.1), (2.5, [100.0, 200.0, 0.0]), "the map view starts at the overview file's");
        press(&mut h, &mut f, IN_JUMP);
        assert_eq!(h.main, 2, "past 5 back to 2");
        // Outside the map main view, insets 1 and 2 become the map inset (3).
        press(&mut h, &mut f, IN_DUCK);
        assert_eq!((h.inset, f.inset), (3, 3));
        press(&mut h, &mut f, IN_DUCK);
        assert_eq!((h.inset, f.inset), (0, 0), "past 3 back to 0");
        // The 0.3 s gate and the HLTV test.
        press(&mut h, &mut f, IN_JUMP);
        assert_eq!(h.main, 3);
        assert_eq!(h.next_input, (f.time + 0.3) as f32);
        f.time += 0.2;
        h.buttons_down(&mut f, IN_JUMP as i32);
        assert_eq!(h.main, 3, "within 0.3 s");
        f.time = h.next_input as f64;
        h.buttons_down(&mut f, IN_JUMP as i32);
        assert_eq!(h.main, 4, "the stored time itself passes");
        f.hltv = false;
        press(&mut h, &mut f, IN_JUMP);
        assert_eq!(h.main, 4, "players: nothing");
    }

    /// The map main view takes the camera insets and drops the map inset.
    #[test]
    fn map_main_view_insets() {
        let (mut h, mut f) = (Hltv::NEW, Fake::default());
        h.set_modes(&mut f, 5, 0);
        h.set_modes(&mut f, 5, 1);
        assert_eq!(h.inset, 1);
        h.set_modes(&mut f, 5, 3);
        assert_eq!((h.inset, f.inset), (0, 0));
        assert_eq!(f.log.iter().filter(|t| t.starts_with("#Spec_Mode_Inset")).count(), 3, "1, 3, then 0");
    }

    /// `FindNextPlayer` (`0x1004b090`) from attack/attack2 (`0x1004b28c`): chase free, the
    /// next/previous active player, the observer target outside roaming; roaming jumps the
    /// camera to the target instead.
    #[test]
    fn attack_picks_the_next_player() {
        let (mut h, mut f) = (Hltv::NEW, Fake::default());
        h.set_modes(&mut f, 2, 0);
        press(&mut h, &mut f, IN_ATTACK);
        assert_eq!((f.mode, f.locked, f.target), (2, 3, (3, 0)));
        assert_eq!(f.log.last().unwrap(), "Now Tracking p3\n");
        press(&mut h, &mut f, IN_ATTACK);
        assert_eq!(f.locked, 7);
        press(&mut h, &mut f, IN_ATTACK);
        assert_eq!(f.locked, 3, "past 64 round to 1");
        press(&mut h, &mut f, IN_ATTACK2);
        assert_eq!(f.locked, 7, "attack2 goes back, below 1 round to 64");
        // Roaming: the target is picked but not followed; the camera jumps to it.
        h.set_modes(&mut f, 3, 0);
        assert_eq!((f.locked, f.target), (0, (0, 0)));
        press(&mut h, &mut f, IN_ATTACK);
        assert_eq!((f.locked, f.target, f.mode), (3, (0, 0), 2));
        assert_eq!(f.jumps.last().unwrap(), &([3.0, 0.0, 0.0], [0.0, 3.0, 0.0]));
        // Nobody to watch.
        f.active.clear();
        press(&mut h, &mut f, IN_ATTACK);
        assert_eq!(f.locked, 0);
        assert_eq!(f.log.last().unwrap(), "No observer targets.\n");
    }

    /// `0x1004b389..0x1004b3df`, `HandleButtonsUp` and `Draw` (`0x1004a8d0`): forward/back zoom
    /// the map by ±0.01 a frame within 0.5..3, the strafes pan it 12 units along the map's
    /// right; releases stop them; roaming leaves the map alone.
    #[test]
    fn map_zoom_and_pan() {
        let (mut h, mut f) = (Hltv::NEW, Fake::default());
        h.set_modes(&mut f, 5, 0);
        f.map = (2.995, [0.0; 3], [90.0, 0.0, 0.0]);
        press(&mut h, &mut f, IN_FORWARD | IN_MOVERIGHT);
        assert_eq!((h.zoom_delta, h.move_delta), (0.01, 12.0));
        h.draw(&mut f);
        assert_eq!(f.map.0, 3.0, "clamped at 3");
        let n = normalize(v::angle_vectors([90.0, 0.0, 0.0]).1);
        assert_eq!(f.map.1, [12.0 * n[0], 12.0 * n[1], 12.0 * n[2]]);
        h.buttons_up(&f, IN_FORWARD as i32);
        assert_eq!((h.zoom_delta, h.move_delta), (0.0, 12.0));
        h.buttons_up(&f, IN_MOVELEFT as i32);
        assert_eq!(h.move_delta, 0.0);
        press(&mut h, &mut f, IN_BACK);
        f.map.0 = 0.505;
        h.draw(&mut f);
        assert_eq!(f.map.0, 0.5, "clamped at 0.5");
        h.set_modes(&mut f, 3, 0);
        f.map.0 = 1.0;
        h.draw(&mut f);
        assert_eq!(f.map.0, 1.0, "roaming: no zoom");
    }

    /// `SetModes` case 3 from in-eye (`0x100b9510`): the jump goes to the target's eyes.
    #[test]
    fn roaming_from_in_eye_jumps_to_the_eyes() {
        let (mut h, mut f) = (Hltv::NEW, Fake { director: (7, 0), ..Fake::default() });
        h.set_modes(&mut f, 2, 0);
        h.set_modes(&mut f, 4, 0);
        assert_eq!((f.mode, f.locked), (5, 7));
        h.set_modes(&mut f, 3, 0);
        let pitch = (1.0f64 * -std::f64::consts::PI) as f32;
        assert_eq!(f.jumps.last().unwrap(), &([0.0, 0.0, 7.0 + 28.0], [pitch, 2.0, 3.0]));
        assert_eq!((f.mode, f.locked, f.target), (3, 0, (0, 0)));
    }

    /// 2.1's `V_GetInEyePos` (`0x100b9510`, issue #184): no `NO_VIEW` stunt test, the pitch ×
    /// the `double` −π, a bad target leaves the camera alone.
    #[test]
    fn in_eye_pos_is_21() {
        let mut f = Fake::default();
        let (mut org, mut ang) = ([1.0; 3], [2.0; 3]);
        in_eye_pos(&f, 0, &mut org, &mut ang);
        assert_eq!((org, ang), ([1.0; 3], [2.0; 3]), "no target: untouched");
        let t = InEyeTarget { origin: [10.0, 20.0, 30.0], angles: [0.3, 90.0, 5.0], solid: 3, usehull: 0 };
        f.eye = Some(t);
        in_eye_pos(&f, 5, &mut org, &mut ang);
        let pitch = (0.3f32 as f64 * -std::f64::consts::PI) as f32;
        assert_eq!((org, ang), ([10.0, 20.0, 58.0], [pitch, 90.0, 5.0]), "standing: +28, whatever the stunt bits");
        assert_ne!(pitch, 0.3 * -std::f32::consts::PI, "the double −π rounds once");
        f.eye = Some(InEyeTarget { usehull: 1, ..t });
        in_eye_pos(&f, 5, &mut org, &mut ang);
        assert_eq!(org[2], 46.0, "ducked: +16");
        f.eye = Some(InEyeTarget { solid: 0, ..t });
        in_eye_pos(&f, 5, &mut org, &mut ang);
        assert_eq!((org[2], ang[2]), (22.0, 80.0), "not solid: −8, roll 80");
    }

    /// `V_CalcSpectatorRefdef` (`0x100b96a0`, issue #181): the main view follows 2.1's main
    /// mode, not the observer mode, and the view model is always hidden.
    #[test]
    fn refdef_main_pass_follows_the_main_mode() {
        let (mut h, mut f) = (Hltv::NEW, Fake { director: (7, 0), ..Fake::default() });
        let pass = |h: &Hltv, f: &mut Fake| {
            let mut p = ref_params_t { vieworg: [1.0; 3], viewangles: [2.0; 3], cl_viewangles: [45.0, 30.0, 0.0], ..Default::default() };
            p.onlyClientDraw = 7;
            h.calc_refdef(f, &mut p);
            p
        };
        // The locked view (main 2, observer mode 4): the camera stays, onlyClientDraw is cleared.
        h.set_modes(&mut f, 2, 0);
        let p = pass(&h, &mut f);
        assert_eq!((p.vieworg, p.viewangles, p.onlyClientDraw, p.nextView), ([1.0; 3], [2.0; 3], 0, 0));
        assert!(f.view_model_cleared && f.inset_pass == Some(false));
        // In-eye (main 4) on the spectator's own target, not `g_iUser2`.
        h.set_modes(&mut f, 4, 0);
        f.target = (3, 0);
        let p = pass(&h, &mut f);
        assert_eq!((p.vieworg, p.onlyClientDraw), ([0.0, 0.0, 7.0 + 28.0], 7), "in-eye leaves onlyClientDraw");
        // The map (main 5, observer mode 4 like the locked view): the map camera, client draw only.
        h.set_modes(&mut f, 5, 0);
        let p = pass(&h, &mut f);
        let pitch = (45.0f64 / 90.0 * 38.75 + 51.25) as f32;
        assert_eq!(f.mode, 4);
        assert_eq!((p.viewangles, f.map.2, p.onlyClientDraw), ([pitch, 30.0, 0.0], [pitch, 30.0, 0.0], 1));
        assert_eq!(p.vieworg, map_position(2.5, [100.0, 200.0, 0.0], [pitch, 30.0, 0.0]));
        // An inset mode asks for the second pass.
        h.set_modes(&mut f, 5, 1);
        assert_eq!(pass(&h, &mut f).nextView, 1);
    }

    /// The inset pass (`0x100ba122..0x100ba300`): the overview window scaled by
    /// `screen / 640f` on the x87, truncated by 2.1's `__ftol`; 2 in-eye, 3 the map.
    #[test]
    fn refdef_inset_pass() {
        let (mut h, mut f) = (Hltv::NEW, Fake { director: (7, 0), screen: (1366, 768), ..Fake::default() });
        h.set_modes(&mut f, 2, 0);
        h.set_modes(&mut f, 2, 3);
        assert_eq!(h.inset, 3);
        let mut p = ref_params_t { nextView: 1, onlyClientDraw: 0, cl_viewangles: [0.0; 3], ..Default::default() };
        f.inset_window = [5, 7, 241, 181];
        h.calc_refdef(&mut f, &mut p);
        let (sx, sy) = (1366.0f64 / 640.0, 768.0f64 / 480.0);
        let want = [(5.0 * sx) as i32, (7.0 * sy) as i32, (241.0 * sx) as i32, (181.0 * sy) as i32];
        assert_eq!(p.viewport, want);
        assert_eq!((p.nextView, p.onlyClientDraw, f.inset_pass), (0, 1, Some(true)), "inset 3: the map");
        assert_eq!(p.viewangles, [51.25, 0.0, 0.0]);
        // Inset 2: in-eye of the spectator's target (7, from the director).
        h.set_modes(&mut f, 5, 2);
        let mut p = ref_params_t { nextView: 1, onlyClientDraw: 1, ..Default::default() };
        h.calc_refdef(&mut f, &mut p);
        assert_eq!((p.vieworg, p.onlyClientDraw), ([0.0, 0.0, 35.0], 0));
        assert_eq!(ftol(f64::NAN), 0);
        assert_eq!(ftol(3e9), 3_000_000_000u32 as i32, "beyond int: the low dword");
        assert_eq!(ftol(1e20), 0, "beyond 64 bits: the indefinite's low dword");
        assert_eq!(ftol(-2.9), -2);
    }

    /// `GetMapPosition` (`0x1004c990`): `4096 / zoom` back along the map angles, the height
    /// scaled by the pitch; straight down (90) puts the camera at z 0 over the origin.
    #[test]
    fn map_position_is_21() {
        let p = map_position(2.0, [100.0, 200.0, 50.0], [90.0, 0.0, 0.0]);
        assert!((p[0] - 100.0).abs() < 1e-3 && (p[1] - 200.0).abs() < 1e-3);
        assert!((p[2] - 2048.0).abs() < 1e-3);
        let p = map_position(1.0, [0.0, 0.0, 90.0], [45.0, 0.0, 0.0]);
        assert!((p[2] - (45.0 + 4096.0 * std::f32::consts::FRAC_1_SQRT_2)).abs() < 1e-2);
    }

    /// `Draw`'s name tags (`0x1004aac7..`): only in the map main mode with `cl_drawnames`, team
    /// colours, `x − 3 × strlen`, and none strictly inside the inset window while one shows.
    #[test]
    fn name_tags_only_on_the_map() {
        let (mut h, mut f) = (Hltv::NEW, Fake::default());
        f.tags = vec![[300.0, 300.0, 1.0], [100.0, 100.0, 1.0], [f32::NAN, 100.0, 0.0], [50.0, 50.0, -1.0]];
        h.set_modes(&mut f, 2, 0);
        h.draw_names(&mut f);
        assert!(f.drawn.is_empty(), "not the map: no tags (3.0 draws them in every mode)");
        h.set_modes(&mut f, 5, 0);
        f.drawnames = 0.0;
        h.draw_names(&mut f);
        assert!(f.drawn.is_empty(), "cl_drawnames 0");
        f.drawnames = f32::NAN;
        h.draw_names(&mut f);
        let got: Vec<_> = f.drawn.iter().map(|d| (d.0, d.1, d.2)).collect();
        assert_eq!(got, [(294, 300, [0.4, 0.4, 1.0]), (94, 100, [1.0, 0.4, 0.4]), (0, 100, [0.8; 3])]);
        assert_eq!(f.drawn[0].3, b"p1");
        // With an inset the tag at (100, 100) is under the window (4, 4, 240, 180); edges aren't.
        h.set_modes(&mut f, 5, 1);
        f.drawn.clear();
        f.tags = vec![[300.0, 300.0, 1.0], [100.0, 100.0, 1.0], [244.0, 100.0, 1.0], [4.0, 100.0, 1.0]];
        h.draw_names(&mut f);
        assert_eq!(f.drawn.iter().map(|d| d.0).collect::<Vec<_>>(), [294, 238, -2]);
        f.hltv = false;
        f.drawn.clear();
        h.draw_names(&mut f);
        assert!(f.drawn.is_empty(), "players: nothing");
    }

    /// TS 3.0 has none of this: its key tap runs, its own `SetModes` runs, `Draw` changes nothing.
    #[test]
    fn core_rules_have_no_hltv_controls() {
        let core = ts30_client::PLUGIN.client;
        let mut f = Fake::default();
        assert!(!core.hltv_buttons(&mut f, IN_JUMP as i32, true));
        assert!(!core.hltv_buttons(&mut f, IN_FORWARD as i32, false));
        assert!(!core.hltv_set_modes(&mut f, 2, 0));
        f.tags = vec![[300.0, 300.0, 1.0]];
        assert!(!core.hltv_draw(&mut f), "3.0's own name tags run");
        assert_eq!((f.mode, f.locked, f.inset, f.panel, f.map.0), (0, 0, 0, 0, 1.0));
        assert!(f.log.is_empty() && f.jumps.is_empty() && f.crosshair.is_none() && f.drawn.is_empty());
        // 3.0's spectator view runs its own passes, and the overview draws in every main view.
        let mut p = ref_params_t { spectator: 1, onlyClientDraw: 5, ..Default::default() };
        assert!(!core.hltv_calc_refdef(&mut f, &mut p));
        assert_eq!((p.onlyClientDraw, p.nextView, f.view_model_cleared, f.inset_pass), (5, 0, false, None));
        assert!(core.hltv_overview_in_main_view());
        // `V_CalcRefdef` (0x10069b90): HLTV viewers and observing players get the spectator view.
        assert!(core.spectator_refdef(true, 0) && core.spectator_refdef(false, 4) && core.spectator_refdef(true, 5));
        assert!(!core.spectator_refdef(false, 0));
        // The overview's camera icon: 3.0's own in-eye refresh.
        f.locked = 3;
        let (mut org, mut ang) = ([1.0; 3], [2.0; 3]);
        assert!(!core.hltv_overview_camera(&f, &mut org, &mut ang));
        assert_eq!((org, ang), ([1.0; 3], [2.0; 3]));
        // The panel: the observer mode's number; `nohud` 0 or NaN shows it.
        assert_eq!(core.hltv_panel_mode(), None);
        assert!(core.spectator_panel_shows(0.0) && core.spectator_panel_shows(f32::NAN) && !core.spectator_panel_shows(0.5));
    }

    /// 2.1's rules run the spectator view only for HLTV (`pparams->spectator`) and 2.1's passes
    /// there, and draw the overview in the main view only on the map main mode.
    #[test]
    fn ts21_rules_hltv_view() {
        let rules = crate::PLUGIN.client;
        let mut f = Fake::default();
        // `V_CalcRefdef` (0x100ba6b0) has no `g_iUser1` test: observers get the normal view
        // (issue #183).
        assert!(rules.spectator_refdef(true, 0) && rules.spectator_refdef(true, 4));
        assert!(!rules.spectator_refdef(false, 1) && !rules.spectator_refdef(false, 4) && !rules.spectator_refdef(false, 5));
        let mut p = ref_params_t { spectator: 1, onlyClientDraw: 5, ..Default::default() };
        assert!(rules.hltv_calc_refdef(&mut f, &mut p));
        assert!(f.view_model_cleared);
        assert!(!rules.hltv_overview_in_main_view(), "main mode 0");
        // `UpdateSpectatorPanel` (0x100af0c0): the main mode's number; a NaN `nohud` hides it.
        assert_eq!(rules.hltv_panel_mode(), Some(0));
        assert!(rules.spectator_panel_shows(0.0) && !rules.spectator_panel_shows(f32::NAN) && !rules.spectator_panel_shows(0.5));
    }

    /// `DrawOverviewEntities` (0x1004cb50, `0x1004d6b3..0x1004d6ca`): the camera moves to 2.1's
    /// `V_GetInEyePos` of the spectator's target only with the in-eye inset (2) or main mode
    /// (4), whatever the observer mode.
    #[test]
    fn overview_camera_follows_the_21_modes() {
        let mut f = Fake { locked: 5, mode: 5, ..Default::default() };
        let start = ([1.0; 3], [2.0; 3]);
        let pitch = |p: f64| (p * -std::f64::consts::PI) as f32;
        for (main, inset, moves) in [(2, 3, false), (5, 1, false), (5, 2, true), (4, 3, true), (3, 0, false)] {
            let h = Hltv { main, inset, ..Hltv::NEW };
            let (mut org, mut ang) = start;
            h.overview_camera(&f, &mut org, &mut ang);
            let want = if moves { ([0.0, 0.0, 5.0 + 28.0], [pitch(1.0), 2.0, 3.0]) } else { start };
            assert_eq!((org, ang), want, "main {main} inset {inset}");
        }
        // 2.1's in-eye position has no `NO_VIEW` test, and a non-solid target drops and rolls.
        f.eye = Some(InEyeTarget { origin: [1.0, 2.0, 3.0], angles: [10.0, 20.0, 0.0], solid: 0, usehull: 0 });
        let (mut org, mut ang) = start;
        Hltv { main: 4, ..Hltv::NEW }.overview_camera(&f, &mut org, &mut ang);
        assert_eq!((org, ang), ([1.0, 2.0, -5.0], [pitch(10.0), 20.0, 80.0]));
        // The plugin hook replaces 3.0's refresh.
        let (mut org, mut ang) = start;
        assert!(crate::PLUGIN.client.hltv_overview_camera(&f, &mut org, &mut ang));
    }

    #[test]
    fn normalize_skips_only_zero() {
        assert_eq!(normalize([0.0; 3]), [0.0; 3]);
        assert_eq!(normalize([3.0, 4.0, 0.0]), [0.6, 0.8, 0.0]);
        assert!(normalize([f32::NAN, 0.0, 0.0])[1].is_nan(), "a NaN length still scales");
    }
}
