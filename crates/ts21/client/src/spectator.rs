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
//! The engine side is [`HltvHost`] (`ts_client::spectator`); the views the modes select are the
//! port's (TS 3.0's), see `docs/implementation/ts21.md`.

use goldsrc::ext::render::{Vec3, vec3 as v};
use goldsrc::sys::{IN_ATTACK, IN_ATTACK2, IN_BACK, IN_DUCK, IN_FORWARD, IN_JUMP, IN_MOVELEFT, IN_MOVERIGHT};
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
                        host.in_eye_camera(host.locked_target(), &mut origin, &mut angles);
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

    fn jump(&mut self, host: &mut dyn HltvHost, origin: Vec3, angles: Vec3) {
        self.jump_origin = origin;
        self.jump_angles = angles;
        host.jump_camera(origin, angles);
    }
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
        fn entity_camera(&self, index: i32) -> Option<(Vec3, Vec3)> {
            Some(([index as f32, 0.0, 0.0], [0.0, index as f32, 0.0]))
        }
        fn in_eye_camera(&self, target: i32, org: &mut Vec3, ang: &mut Vec3) {
            if target != 0 {
                *org = [0.0, 0.0, target as f32];
                *ang = [1.0, 2.0, 3.0];
            }
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
        assert_eq!(f.jumps.last().unwrap(), &([0.0, 0.0, 7.0], [1.0, 2.0, 3.0]));
        assert_eq!((f.mode, f.locked, f.target), (3, 0, (0, 0)));
    }

    /// TS 3.0 has none of this: its key tap runs, its own `SetModes` runs, `Draw` changes nothing.
    #[test]
    fn core_rules_have_no_hltv_controls() {
        let core = ts_client::gameplay::CORE_PLUGIN.client;
        let mut f = Fake::default();
        assert!(!core.hltv_buttons(&mut f, IN_JUMP as i32, true));
        assert!(!core.hltv_buttons(&mut f, IN_FORWARD as i32, false));
        assert!(!core.hltv_set_modes(&mut f, 2, 0));
        core.hltv_draw(&mut f);
        assert_eq!((f.mode, f.locked, f.inset, f.panel, f.map.0), (0, 0, 0, 0, 1.0));
        assert!(f.log.is_empty() && f.jumps.is_empty() && f.crosshair.is_none());
    }

    #[test]
    fn normalize_skips_only_zero() {
        assert_eq!(normalize([0.0; 3]), [0.0; 3]);
        assert_eq!(normalize([3.0, 4.0, 0.0]), [0.6, 0.8, 0.0]);
        assert!(normalize([f32::NAN, 0.0, 0.0])[1].is_nan(), "a NaN length still scales");
    }
}
