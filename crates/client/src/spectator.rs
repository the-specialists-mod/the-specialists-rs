//! Spectating: TS's `CHudSpectator` (HUD element 13, `gHUD+0x6d8`, vtable `1007cb00`) and the
//! spectator view (`V_CalcSpectatorRefdef` `0x10068f70`). Reference: decomp
//! client/architecture-hud.md §4 and rendering-effects-prediction.md §7.
//!
//! TS uses the pre-HLTV-rewrite observer numbering (`shared::ts::obs`): 1 chase locked, 2 chase
//! free, 3 roaming, 4 the director's locked view (HLTV only), 5 in-eye.
//!
//! For players the server drives the mode and target (`iuser1`/`iuser2`), and the camera
//! position comes from player movement (`PM_SpectatorMove` puts the chase camera into `simorg`).
//! This module adds TS's in-eye view (the target's eyes and gun), the `cl_vsmoothing` rings, and
//! the HLTV-only parts of `CHudSpectator`: director events, `specmode`, the director-reset camera.
//! TS's overview map and name tags only draw for HLTV spectators and aren't ported
//! (docs/implementation/client-hud.md §8).

use crate::hud::text::localise;
use crate::state;
use goldsrc::Global;
use goldsrc::client::engine;
use goldsrc::ext::render::{self as r, Vec3, vec3 as v};
use goldsrc::ext::spectator as spec;
use goldsrc::sys::*;
use shared::protocol::hud_wire::director;
use shared::ts::{obs, stunt};
use std::cell::RefCell;

/// `cl_vsmoothing` ring size (`ORIGIN_BACKUP`).
const RING: usize = 64;

/// One `cl_vsmoothing` ring (`0x10190f98` angles / `0x10190b98` origins, 64 entries with times).
#[derive(Clone)]
struct Ring {
    v: [Vec3; RING],
    time: [f32; RING],
    next: usize,
}

impl Default for Ring {
    fn default() -> Self {
        Self { v: [[0.0; 3]; RING], time: [0.0; RING], next: 0 }
    }
}

impl Ring {
    fn push(&mut self, x: Vec3, time: f32) {
        self.v[self.next & (RING - 1)] = x;
        self.time[self.next & (RING - 1)] = time;
        self.next = self.next.wrapping_add(1);
    }

    /// The pair of samples around `t` (SDK `V_CalcNormalRefdef` ring search) and the fraction
    /// between them, or `None` when the history doesn't reach back that far.
    fn sample(&self, t: f32) -> Option<(Vec3, Vec3, f32)> {
        let mut i = 1;
        let mut found = self.next.wrapping_sub(2);
        while i < RING - 1 {
            if self.time[found & (RING - 1)] <= t {
                break;
            }
            i += 1;
            found = found.wrapping_sub(1);
        }
        let (a, b) = (found & (RING - 1), found.wrapping_add(1) & (RING - 1));
        if i >= RING - 1 || self.time[a] == 0.0 {
            return None;
        }
        let dt = self.time[b] - self.time[a];
        (dt > 0.0).then(|| (self.v[a], self.v[b], ((t - self.time[a]) / dt).min(1.0)))
    }
}

#[derive(Default)]
struct Spectator {
    /// Picture-in-picture mode (`+0x14`; HLTV, never drawn here).
    pip: i32,
    /// Target picked for the locked view (`+0x948`).
    locked_target: i32,
    /// Last director camera: primary and secondary entity (`+0xca0`/`+0xca4`) and flags
    /// (`0x10192158`, read by TS's client `PM_SpectatorMove` for the locked view).
    last_primary: i32,
    last_secondary: i32,
    director_flags: i32,
    /// Map camera from the last director reset (`0x10191d40`); pending (`0x10192154`) until
    /// player movement has moved a roaming spectator there once.
    default_origin: Vec3,
    pin_pending: bool,
    /// `sprites/crosshairs.spr` (`+0xc8c`).
    crosshair: HSPRITE,
    // ---- view (`V_CalcSpectatorRefdef` statics) ----
    /// Angle ring and the last angles stored in it (`0x10190b44`).
    angles: Ring,
    last_angles: Vec3,
    /// Origin ring and the last (smoothed) view origin (`0x10190b60`).
    origins: Ring,
    last_origin: Vec3,
    /// Cached in-eye gun: the target's `weaponmodel` and the TS weapon it belongs to
    /// (`0x10191db0`/`0x10097218`).
    gun_weaponmodel: i32,
    gun: Option<u8>,
}

static SPEC: Global<RefCell<Option<Spectator>>> = Global(RefCell::new(None));

fn with<R>(f: impl FnOnce(&mut Spectator) -> R) -> R {
    let mut g = SPEC.0.borrow_mut();
    f(g.get_or_insert_with(Spectator::default))
}

/// `CHudSpectator::Init` (`0x10022c70`): the `specmode` command (`cl_drawnames` is registered
/// with the HUD cvars).
pub fn init() {
    with(|s| *s = Spectator::default());
    engine().add_command(c"specmode", cmd_specmode);
}

/// `CHudSpectator::VidInit` (`0x10022fe0`). TS also loads the overview sprites (iplayer*,
/// tile, camera, laserbeam); without the overview only the crosshair is used.
pub fn vid_init() {
    let h = r::spr_load("sprites/crosshairs.spr");
    with(|s| s.crosshair = h);
}

/// `specmode <n>` (`0x10022c10`): HLTV spectators pick a camera mode (and turn the inset off).
extern "C" fn cmd_specmode() {
    let e = engine();
    if !spec::is_spectate_only() {
        e.print("specmode only in HLTV mode available.\n");
        return;
    }
    if e.cmd_argc() < 2 {
        e.print("usage:  specmode < modenumber >\n");
        return;
    }
    let mode = e.cmd_argv(1).to_str().ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
    // TS passes inset mode 0 (the push before the `Cmd_Argv` call stays on the stack).
    set_modes(mode, 0);
}

/// What `set_modes` decided, applied by the caller (pure, so it can be tested).
#[derive(Debug, Default, PartialEq)]
struct ModeChange {
    /// Centre texts to show (`%c#Spec_Mode_Inset%d`, `%c#Spec_Mode%d` through `TextMsg`).
    texts: Vec<String>,
    /// Crosshair: `Some(true)` = the spectator crosshair, `Some(false)` = none.
    crosshair: Option<bool>,
}

/// `CHudSpectator::SetModes` (`0x100234d0`). Requests use the `specmode` numbers: 2 selects the
/// director's locked view (4), 3 roaming and 5 in-eye; 1 and 4 change nothing but the text.
fn set_modes_in(s: &mut Spectator, st: &mut state::Shared, mode: i32, pip: i32, valid_target: impl Fn(i32) -> bool) -> Option<ModeChange> {
    if !(1..=5).contains(&mode) {
        return None;
    }
    let mut out = ModeChange::default();
    if pip != s.pip {
        s.pip = pip;
        out.texts.push(format!("#Spec_Mode_Inset{pip}"));
    }
    if mode == st.spec_mode {
        return Some(out);
    }
    match mode {
        2 => st.spec_mode = obs::LOCKED_VIEW,
        3 => {
            st.spec_mode = obs::ROAMING;
            s.locked_target = 0;
            st.spec_target = 0;
            st.spec_flags = 0;
        }
        5 => st.spec_mode = obs::IN_EYE,
        _ => {}
    }
    out.texts.push(format!("#Spec_Mode{mode}"));
    if st.spec_mode == obs::LOCKED_VIEW && s.locked_target == 0 && valid_target(s.last_primary) {
        st.spec_target = s.last_primary;
        s.locked_target = s.last_primary;
        st.spec_flags = s.last_secondary;
    }
    out.crosshair = Some(matches!(st.spec_mode, obs::ROAMING | obs::IN_EYE));
    Some(out)
}

/// A director target must be another, present player (`0x100236d0`).
fn valid_player(idx: i32) -> bool {
    let local = r::local_player().map_or(0, |p| p.index);
    idx > 0 && idx != local && r::entity(idx).is_some_and(|e| e.player != 0 && !e.model.is_null())
}

pub fn set_modes(mode: i32, pip: i32) {
    let change = with(|s| state::with(|st| set_modes_in(s, st, mode, pip, valid_player)));
    let Some(change) = change else {
        engine().print("Invalid spectator mode.\n");
        return;
    };
    for t in &change.texts {
        engine().center_print(&localise(t));
    }
    if let Some(on) = change.crosshair {
        let h = with(|s| s.crosshair);
        if on && h != 0 {
            spec::set_crosshair(h, [24, 48, 0, 24], [255, 255, 255]);
        } else {
            spec::set_crosshair(0, [0; 4], [0; 3]);
        }
    }
}

/// `HUD_DirectorEvent` (`0x1001ab60`) → `CHudSpectator::DirectorMessage` (`0x10023380`): TS's
/// pre-SDK-2.3 director interface. `DRC_ACTIVE` resets spectating (locked view, no inset, camera
/// on the map's first `info_player_start`); `DRC_CAMERA` records the director's targets and, in
/// the locked view, follows them.
pub fn director_event(command: u8, primary: i32, secondary: i32, flags: i32) {
    match command {
        director::DRC_ACTIVE => {
            with(|s| {
                s.pip = 0;
                s.locked_target = 0;
                s.last_primary = 0;
                s.last_secondary = 0;
                s.pin_pending = false;
            });
            state::with(|st| st.spec_mode = obs::NONE);
            set_modes(2, 0);
            let origin = map_camera();
            with(|s| {
                s.default_origin = origin;
                s.pin_pending = true;
            });
        }
        director::DRC_CAMERA => {
            let locked = state::with(|st| {
                let locked = st.spec_mode == obs::LOCKED_VIEW;
                if locked {
                    st.spec_target = primary;
                    st.spec_flags = secondary;
                }
                locked
            });
            with(|s| {
                if locked {
                    s.locked_target = primary;
                }
                s.last_primary = primary;
                s.last_secondary = secondary;
                s.director_flags = flags;
            });
        }
        _ => {
            if engine().cvar_float(c"developer") != 0.0 {
                engine().print("CHudSpectator::DirectorEvent: unknown director command.\n");
            }
        }
    }
}

/// `HUD_DirectorMessage` (port addition): the engine export SDK 2.3+ engines call with a
/// `svc_director` payload. TS only exports the older `HUD_DirectorEvent`, which these engines
/// never call. The two commands TS handles map onto the new numbering: `DRC_CMD_START` (1) is
/// TS's `DRC_ACTIVE`, and `DRC_CMD_EVENT` (2) carries the same `short, short, long` targets as
/// TS's `DRC_CAMERA` (2).
pub fn director_message(buf: &[u8]) {
    if let Some((cmd, primary, secondary, flags)) = parse_director_message(buf) {
        director_event(cmd, primary, secondary, flags);
    }
}

/// `svc_director` payload → TS's `(command, primary, secondary, flags)`.
fn parse_director_message(buf: &[u8]) -> Option<(u8, i32, i32, i32)> {
    let rd16 = |o: usize| buf.get(o..o + 2).map_or(0, |b| u16::from_le_bytes([b[0], b[1]]) as i32);
    let rd32 = |o: usize| buf.get(o..o + 4).map_or(0, |b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    match buf.first() {
        Some(1) => Some((director::DRC_ACTIVE, 0, 0, 0)),
        Some(2) => Some((director::DRC_CAMERA, rd16(1), rd16(3), rd32(5))),
        _ => None,
    }
}

/// `0x10022da0`: the camera position after a director reset. TS walks the BSP entity lump and
/// keeps the last `origin` read before the first `info_player_start` classname.
fn map_camera() -> Vec3 {
    let bsp = r::load_file(&r::level_name()).unwrap_or_default();
    let ents = crate::effects::emitters::entity_lump(&bsp).map(crate::effects::emitters::parse_entities).unwrap_or_default();
    camera_from_entities(&ents)
}

fn camera_from_entities(ents: &[Vec<(String, String)>]) -> Vec3 {
    let mut origin = [0.0; 3];
    for e in ents {
        for (k, val) in e {
            if k == "origin" {
                origin = parse_vec(val);
            }
            if k == "classname" && val == "info_player_start" {
                return origin;
            }
        }
    }
    origin
}

/// `0x10022d00`: up to three numbers separated by spaces; missing ones are 0.
fn parse_vec(s: &str) -> Vec3 {
    let mut out = [0.0; 3];
    for (o, t) in out.iter_mut().zip(s.split(' ')) {
        *o = t.trim().parse().unwrap_or(0.0);
    }
    out
}

/// The client's `PM_SpectatorMove` (`0x1002c790`) start: after a director reset, a roaming
/// spectator is moved to the map camera once (angles and velocity cleared) and the move ends
/// there. Returns true when it did; `HUD_PlayerMove` then skips the shared player movement.
pub fn pin_camera(pm: &mut playermove_t) -> bool {
    if pm.spectator == 0 && pm.iuser1 <= 0 {
        return false;
    }
    let roaming = pm.iuser1 == obs::ROAMING || pm.iuser2 == 0;
    let origin = with(|s| {
        let o = (s.pin_pending && roaming).then_some(s.default_origin);
        if o.is_some() {
            s.pin_pending = false;
        }
        o
    });
    let Some(origin) = origin else { return false };
    pm.origin = origin;
    pm.angles = [0.0; 3];
    pm.velocity = [0.0; 3];
    true
}

/// Spectating someone in-eye (`g_iUser1 == 5`).
pub fn in_eye() -> bool {
    state::with(|s| s.spec_mode == obs::IN_EYE)
}

// ---- view ---------------------------------------------------------------------------------------

/// `V_CalcSpectatorRefdef` (`0x10068f70`). The camera starts at `simorg` with the mouse angles:
/// player movement has already put a chasing spectator behind its target, so the chase and
/// roaming modes use them as they are (TS's chase-position code there only writes globals the
/// function overwrites at its end). In-eye moves to the target's eyes and shows its gun.
pub fn calc_refdef(p: &mut ref_params_t) {
    let (mode, target) = state::with(|s| (s.spec_mode, s.spec_target));
    p.vieworg = p.simorg;
    p.viewangles = p.cl_viewangles;
    p.onlyClientDraw = 0;
    smooth(p, mode, target);
    if mode == obs::IN_EYE {
        in_eye_position(target, &mut p.vieworg, &mut p.viewangles);
    }
    let weaponmodel = (target != 0).then(|| r::entity(target)).flatten().map(|t| t.curstate.weaponmodel);
    match weaponmodel {
        Some(wm) if mode == obs::IN_EYE => in_eye_gun(p, target, wm),
        _ => {
            if let Some(vm) = r::view_model() {
                vm.model = std::ptr::null_mut();
            }
            with(|s| {
                s.gun_weaponmodel = 0;
                s.gun = None;
            });
        }
    }
}

/// `cl_vsmoothing` (the angle ring `0x10190f98` and origin ring `0x10190b98`). TS's client
/// `PM_SpectatorMove` flags what to smooth when it follows a target: origins in chase locked,
/// both in the locked view and in-eye (where the in-eye position replaces them afterwards).
fn smooth(p: &mut ref_params_t, mode: i32, target: i32) {
    let following = target != 0 && r::entity(target).is_some_and(|e| e.player != 0);
    let (smooth_angles, smooth_origin) = match mode {
        _ if !following => (false, false),
        obs::CHASE_LOCKED => (false, true),
        obs::LOCKED_VIEW | obs::IN_EYE => (true, true),
        _ => (false, false),
    };
    let amount = r::Cvar::find(c"cl_vsmoothing").value();
    let time = p.time;
    with(|s| {
        if v::len(v::sub(p.viewangles, s.last_angles)) != 0.0 {
            s.angles.push(p.viewangles, time);
            s.last_angles = p.viewangles;
        }
        if amount != 0.0
            && smooth_angles
            && let Some((a, b, frac)) = s.angles.sample(time - amount)
            && angle_between(v::angle_vectors(a).0, v::angle_vectors(b).0) < 22.5
        {
            p.viewangles = interpolate_angles(a, b, frac);
        }
        // TS compares against the last *smoothed* origin.
        if v::len(v::sub(p.vieworg, s.last_origin)) != 0.0 {
            s.origins.push(p.vieworg, time);
        }
        if amount != 0.0
            && smooth_origin
            && let Some((a, b, frac)) = s.origins.sample(time - amount)
            && v::len(v::sub(b, a)) < 64.0
        {
            let org = v::ma(a, frac, v::sub(b, a));
            p.vieworg = v::add(p.vieworg, v::sub(org, p.simorg));
        }
        s.last_origin = p.vieworg;
    });
}

/// `0x10029d70`: angle between two vectors in degrees (0 if either is zero).
fn angle_between(a: Vec3, b: Vec3) -> f32 {
    let (la, lb) = (v::len(a), v::len(b));
    if la == 0.0 || lb == 0.0 {
        return 0.0;
    }
    (v::dot(a, b) / (la * lb)).clamp(-1.0, 1.0).acos().to_degrees()
}

/// `InterpolateAngles` (`0x10029cf0`): per-axis shortest-way blend of normalised angles.
fn interpolate_angles(a: Vec3, b: Vec3, frac: f32) -> Vec3 {
    let mut out = [0.0; 3];
    for i in 0..3 {
        let (x, y) = (v::norm_angle(a[i]), v::norm_angle(b[i]));
        let mut d = y - x;
        if d > 180.0 {
            d -= 360.0;
        } else if d < -180.0 {
            d += 360.0;
        }
        out[i] = v::norm_angle(x + d * frac);
    }
    out
}

/// `V_GetInEyePos` (`0x10068e20`): the target's origin raised to eye height (16 ducked, 28
/// standing, none while `NO_VIEW` stunts are on) and its angles with the model pitch undone. TS
/// scales the pitch by -π where the SDK uses -3. A dead target (not solid) rolls 80° and drops 8.
fn in_eye_position(target: i32, org: &mut Vec3, ang: &mut Vec3) {
    let Some(e) = r::entity(target).filter(|e| e.player != 0) else { return };
    *org = e.origin;
    *ang = e.angles;
    ang[0] *= -std::f32::consts::PI;
    if e.curstate.solid == SOLID_NOT as i16 {
        ang[2] = 80.0;
        org[2] -= 8.0;
    } else if e.curstate.iuser4 & stunt::NO_VIEW == 0 {
        org[2] += if e.curstate.usehull == 1 { 16.0 } else { 28.0 };
    }
}

/// TS weapon id whose third-person model is `weaponmodel` (`0x10068ef0` compares the model name
/// with each weapon's p_ and silenced p_ model).
fn weapon_for_model(weaponmodel: i32) -> Option<u8> {
    if weaponmodel == 0 {
        return None;
    }
    (0..shared::weapons::MAX_WEAPONS as u8).find(|&id| {
        let (_, p, sil, _) = shared::weapons::models(id);
        [p, sil].into_iter().flatten().any(|m| r::model_index(&m) == weaponmodel)
    })
}

/// The in-eye half of `V_CalcSpectatorRefdef`: the view model shows the target's gun (with its
/// attachments' body groups), placed at the in-eye view. TS adds its bob and gun offset globals
/// here, which keep the last values `V_CalcNormalRefdef` left; the port uses none.
fn in_eye_gun(p: &ref_params_t, target: i32, weaponmodel: i32) {
    let Some(vm) = r::view_model() else { return };
    let gun = with(|s| {
        if s.gun_weaponmodel != weaponmodel {
            s.gun_weaponmodel = weaponmodel;
            s.gun = weapon_for_model(weaponmodel);
        }
        s.gun
    });
    let Some(id) = gun else {
        vm.model = std::ptr::null_mut();
        return;
    };
    let index = r::model_index(&shared::weapons::models(id).0);
    vm.model = spec::model_by_index(index);
    if vm.model.is_null() {
        return;
    }
    let items = state::with(|s| s.active_items.get(target as usize).copied().unwrap_or(0));
    vm.curstate.modelindex = index;
    vm.curstate.frame = 0.0;
    vm.curstate.colormap = 0;
    vm.curstate.body = crate::events::fire::weapon_body(id as i32, items as i32);
    vm.index = target;
    let mut angles = [-p.viewangles[0], p.viewangles[1], p.viewangles[2]];
    let mut origin = p.vieworg;
    spec::apply_shake(&mut origin, &mut angles, 0.9);
    origin[2] -= 1.0;
    vm.origin = origin;
    vm.angles = angles;
    vm.curstate.angles = angles;
    vm.latched.prevangles = angles;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specmode_numbers() {
        let mut s = Spectator { last_primary: 7, last_secondary: 9, ..Default::default() };
        let mut st = state::Shared::default();
        let c = set_modes_in(&mut s, &mut st, 2, 0, |i| i == 7).unwrap();
        assert_eq!(st.spec_mode, obs::LOCKED_VIEW, "specmode 2 is the locked view (4)");
        assert_eq!((st.spec_target, st.spec_flags, s.locked_target), (7, 9, 7));
        assert_eq!(c.texts, ["#Spec_Mode2"]);
        assert_eq!(c.crosshair, Some(false));

        let c = set_modes_in(&mut s, &mut st, 5, 1, |_| true).unwrap();
        assert_eq!(st.spec_mode, obs::IN_EYE);
        assert_eq!(c.texts, ["#Spec_Mode_Inset1", "#Spec_Mode5"]);
        assert_eq!(c.crosshair, Some(true));

        let c = set_modes_in(&mut s, &mut st, 3, 1, |_| true).unwrap();
        assert_eq!((st.spec_mode, st.spec_target, s.locked_target), (obs::ROAMING, 0, 0));
        assert_eq!(c.crosshair, Some(true));

        let c = set_modes_in(&mut s, &mut st, 1, 1, |_| true).unwrap();
        assert_eq!(st.spec_mode, obs::ROAMING, "1 only prints");
        assert_eq!(c.texts, ["#Spec_Mode1"]);
        assert!(set_modes_in(&mut s, &mut st, 6, 1, |_| true).is_none());
    }

    #[test]
    fn map_camera_uses_origin_before_classname() {
        let kv = |v: &[(&str, &str)]| v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect::<Vec<_>>();
        let ents = vec![
            kv(&[("classname", "worldspawn")]),
            kv(&[("origin", "1 2 3"), ("classname", "info_player_deathmatch")]),
            kv(&[("origin", "10 20"), ("classname", "info_player_start")]),
            kv(&[("origin", "7 7 7"), ("classname", "info_player_start")]),
        ];
        assert_eq!(camera_from_entities(&ents), [10.0, 20.0, 0.0]);
    }

    #[test]
    fn rings() {
        let mut r = Ring::default();
        for i in 0..10 {
            r.push([i as f32, 0.0, 0.0], 1.0 + i as f32 * 0.1);
        }
        let (a, b, f) = r.sample(1.55).unwrap();
        assert_eq!((a[0], b[0]), (5.0, 6.0));
        assert!((f - 0.5).abs() < 1e-3);
        assert!(r.sample(0.5).is_none(), "history doesn't reach back");
        let i = interpolate_angles([0.0, 170.0, 0.0], [0.0, -170.0, 0.0], 0.5);
        assert!((i[1].abs() - 180.0).abs() < 1e-3, "blends across ±180");
        assert!((angle_between([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]) - 90.0).abs() < 1e-3);
    }

    #[test]
    fn director_message_maps_new_commands() {
        assert_eq!(parse_director_message(&[2, 5, 0, 6, 0, 0x27, 0, 0, 0]), Some((director::DRC_CAMERA, 5, 6, 0x27)));
        assert_eq!(parse_director_message(&[1]), Some((director::DRC_ACTIVE, 0, 0, 0)));
        assert_eq!(parse_director_message(&[3, 1]), None, "DRC_CMD_MODE isn't handled by TS");
        assert_eq!(parse_director_message(&[]), None);
    }
}
