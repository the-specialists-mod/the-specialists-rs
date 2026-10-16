//! Server-side studio animation for the stock model entities (`cycler`, `xen_*`,
//! `monster_hevsuit_dead`): TS `CBaseAnimating` (0xa35a0..0xa3f7c), `CActAnimating::SetActivity`
//! (0xf5104) and the animation.cpp helpers (`LookupActivity` 0xa4070, `GetAnimationEvent`
//! 0xa46c0, `SetController` 0xa4820). They're unchanged from the SDK.
//!
//! The client animates the model from `sequence`/`frame`/`animtime`/`framerate`; the server keeps
//! `frame` in step so it knows when a sequence ends and which model events it crosses.

use goldsrc::edict::EdictPtr;
use goldsrc::ext::movement::{BoneController, STUDIO_LOOPING, SeqInfo, StudioEvent, StudioHdr, model_ptr};
use goldsrc::server::engine;
use goldsrc::sys::*;

/// SDK `Activity` values used by the stock classes.
pub mod act {
    pub const IDLE: i32 = 1;
    pub const CROUCH: i32 = 17;
    pub const CROUCHIDLE: i32 = 18;
    pub const STAND: i32 = 19;
    pub const MELEE_ATTACK1: i32 = 30;
}

/// Events at or above this are client-side (`EVENT_CLIENT`).
const EVENT_CLIENT: i32 = 5000;
/// `STUDIO_XR | STUDIO_YR | STUDIO_ZR`.
const STUDIO_ROTATION: i32 = 0x38;

/// The `CBaseAnimating` members (`m_flFrameRate`, `m_flGroundSpeed`, `m_flLastEventCheck`,
/// `m_fSequenceFinished`, `m_fSequenceLoops`) and `CActAnimating::m_Activity`.
#[derive(Default, Clone, Copy, Debug)]
pub struct Animating {
    pub frame_rate: f32,
    pub ground_speed: f32,
    pub last_event_check: f32,
    pub finished: bool,
    pub loops: bool,
    pub activity: i32,
}

impl Animating {
    /// The `"CBaseAnimating"` table (`CBaseAnimating::Save` 0xa3500): the five members at
    /// +0x6c..+0x7c, both ways.
    pub fn save_fields(&mut self, f: &mut crate::save::Fields) {
        let mut a = crate::save::Animating {
            frame_rate: self.frame_rate,
            ground_speed: self.ground_speed,
            last_event_check: self.last_event_check,
            sequence_finished: self.finished,
            sequence_loops: self.loops,
        };
        f.animating(&mut a);
        self.frame_rate = a.frame_rate;
        self.ground_speed = a.ground_speed;
        self.last_event_check = a.last_event_check;
        self.finished = a.sequence_finished;
        self.loops = a.sequence_loops;
    }

    /// `"CBaseAnimating"`, then `"CActAnimating"` (`CActAnimating::Save` 0xf5064): `m_Activity`
    /// (+0x80).
    pub fn save_act_fields(&mut self, f: &mut crate::save::Fields) {
        self.save_fields(f);
        f.table(&crate::save::tables::CActAnimating);
        f.int(&mut self.activity);
    }

    /// `CBaseAnimating::ResetSequenceInfo` (0xa37c4).
    pub fn reset_sequence_info(&mut self, e: *mut edict_t) {
        let seq = e.v().sequence;
        let info = model_ptr(e).and_then(|h| h.sequence_info(seq));
        // GetSequenceInfo leaves 0/0 for a missing model or sequence.
        (self.frame_rate, self.ground_speed) = info.map_or((0.0, 0.0), |i| i.rates());
        self.loops = info.is_some_and(|i| i.flags & STUDIO_LOOPING != 0);
        let now = engine().time();
        let v = e.v();
        v.animtime = now;
        v.framerate = 1.0;
        self.finished = false;
        self.last_event_check = now;
    }

    /// `CBaseAnimating::StudioFrameAdvance(interval)` (0xa35a0); `interval` 0 means "since
    /// `animtime`". Returns the interval used.
    pub fn frame_advance(&mut self, e: *mut edict_t, interval: f32) -> f32 {
        use crate::stunt::anim::{FrameStep, frame_advance_in};
        let now = engine().time();
        let v = e.v();
        let step = FrameStep {
            interval,
            time: now,
            animtime: v.animtime,
            frame: v.frame,
            frame_rate: self.frame_rate,
            framerate: v.framerate,
            loops: self.loops,
        };
        // The frame step in the server's arithmetic, rounded once (issue #243).
        let (frame, finished) = frame_advance_in(crate::rules::cvars::pm_arith(), step);
        let used = if interval == 0.0 { now - v.animtime } else { interval };
        v.animtime = now;
        let Some(frame) = frame else {
            return 0.0;
        };
        v.frame = frame;
        self.finished |= finished;
        if step.animtime == 0.0 { 0.0 } else { used }
    }

    /// `CBaseAnimating::DispatchAnimEvents` (0xa3884): the server-side model events crossed in
    /// the next 0.1 s (the SDK ignores the interval it's given), for the class's
    /// `HandleAnimEvent`.
    pub fn anim_events(&mut self, e: *mut edict_t) -> Vec<i32> {
        let Some(h) = model_ptr(e) else {
            engine().alert(ALERT_TYPE::at_aiconsole, "Gibbed monster is thinking!\n");
            return Vec::new();
        };
        let interval = 0.1;
        let v = e.v();
        let start = v.frame + (self.last_event_check - v.animtime) * self.frame_rate * v.framerate;
        let end = v.frame + interval * self.frame_rate * v.framerate;
        self.last_event_check = v.animtime + interval;
        self.finished = end >= 256.0 || end <= 0.0;
        animation_events(&h, v.sequence, start, end)
    }

    /// `CActAnimating::SetActivity` (0xf5104): a sequence for `activity`, from frame 0.
    pub fn set_activity(&mut self, e: *mut edict_t, activity: i32) {
        let seq = lookup_activity(e, activity);
        if seq != -1 {
            let v = e.v();
            v.sequence = seq;
            v.frame = 0.0;
            self.activity = activity;
            self.reset_sequence_info(e);
        }
    }
}

/// `GetAnimationEvent` (0xa46c0), looped until it reports none: events of `seq` whose frame lies
/// in `start..end` (0..256 units), wrapping for looping sequences.
fn animation_events(h: &StudioHdr, seq: i32, start: f32, end: f32) -> Vec<i32> {
    let Some(info) = h.sequence_info(seq) else { return Vec::new() };
    events_in_range(&info, &h.sequence_events(seq), start, end)
}

fn events_in_range(info: &SeqInfo, events: &[StudioEvent], start: f32, end: f32) -> Vec<i32> {
    let (start, end) = if info.numframes > 1 {
        let k = (info.numframes - 1) as f32 / 256.0;
        (start * k, end * k)
    } else {
        (0.0, 1.0)
    };
    let n = info.numframes as f32;
    let looping = info.flags & STUDIO_LOOPING != 0;
    events
        .iter()
        .filter(|ev| ev.event < EVENT_CLIENT)
        .filter(|ev| {
            let f = ev.frame as f32;
            (f >= start && f < end) || (looping && end >= n - 1.0 && f < end - n + 1.0)
        })
        .map(|ev| ev.event)
        .collect()
}

/// `LookupActivity` (0xa4070): a sequence for `activity`, chosen at random by `actweight`; -1 if
/// none (0 without a model).
pub fn lookup_activity(e: *mut edict_t, activity: i32) -> i32 {
    let Some(h) = model_ptr(e) else { return 0 };
    let eng = engine();
    let mut total = 0i32;
    let mut seq = -1;
    for i in 0..h.num_sequences() {
        let Some((a, weight)) = h.sequence_activity(i) else { continue };
        if a == activity {
            // The weights come from the model file: C int arithmetic (wrapping).
            total = total.wrapping_add(weight);
            if total == 0 || eng.random_long(0, total.wrapping_sub(1)) < weight {
                seq = i;
            }
        }
    }
    seq
}

/// `LookupSequence`: -1 if the model or sequence is missing.
pub fn lookup_sequence(e: *mut edict_t, name: &str) -> i32 {
    model_ptr(e).map_or(-1, |h| h.lookup_sequence(name))
}

/// `SetController` (0xa4820): sets `pev->controller[index]` from an angle/value.
fn set_controller(e: *mut edict_t, h: &StudioHdr, index: i32, value: f32) {
    let Some(c) = h.bone_controllers().into_iter().find(|c| c.index == index) else { return };
    if let Some(b) = e.v().controller.get_mut(index as usize) {
        *b = controller_setting(&c, value);
    }
}

/// The controller byte for `value` (the body of `SetController`).
fn controller_setting(c: &BoneController, value: f32) -> u8 {
    let mut value = value;
    if c.kind & STUDIO_ROTATION != 0 {
        // Inverted ranges flip the value.
        if c.end < c.start {
            value = -value;
        }
        if c.start + 359.0 >= c.end {
            let mid = (c.start + c.end) * 0.5;
            if value > mid + 180.0 {
                value -= 360.0;
            }
            if value < mid - 180.0 {
                value += 360.0;
            }
        } else if value > 360.0 {
            value -= ((value / 360.0) as i32) as f32 * 360.0;
        } else if value < 0.0 {
            value += (((value / -360.0) + 1.0) as i32) as f32 * 360.0;
        }
    }
    ((255.0 * (value - c.start) / (c.end - c.start)) as i32).clamp(0, 255) as u8
}

/// `CBaseAnimating::InitBoneControllers` (0xa39d8): controllers 0..3 at their zero position.
pub fn init_bone_controllers(e: *mut edict_t) {
    if let Some(h) = model_ptr(e) {
        for i in 0..4 {
            set_controller(e, &h, i, 0.0);
        }
    }
}

/// `CBaseAnimating::SetSequenceBox` (0xa3c64): the sequence's box rotated by yaw, flattened to
/// 0..1 in z.
pub fn set_sequence_box(e: *mut edict_t) {
    let seq = e.v().sequence;
    let Some((mins, maxs)) = model_ptr(e).and_then(|h| h.sequence_bbox(seq)) else { return };
    let (rmin, rmax) = rotated_box(mins, maxs, e.v().angles[1]);
    engine().set_size(e, rmin, rmax);
}

/// The yaw-rotated bounds of a box, flattened to z 0..1.
fn rotated_box(mins: [f32; 3], maxs: [f32; 3], yaw_degrees: f32) -> ([f32; 3], [f32; 3]) {
    let yaw = yaw_degrees * (std::f32::consts::PI / 180.0);
    let (x, y) = ([yaw.cos(), yaw.sin()], [-yaw.sin(), yaw.cos()]);
    let bounds = [mins, maxs];
    let (mut rmin, mut rmax) = ([9999.0f32; 3], [-9999.0f32; 3]);
    for bi in &bounds {
        for bj in &bounds {
            for bk in &bounds {
                let base = [bi[0], bj[1], bk[2]];
                let t = [x[0] * base[0] + y[0] * base[1], x[1] * base[0] + y[1] * base[1], base[2]];
                for k in 0..3 {
                    rmin[k] = rmin[k].min(t[k]);
                    rmax[k] = rmax[k].max(t[k]);
                }
            }
        }
    }
    rmin[2] = 0.0;
    rmax[2] = 1.0;
    (rmin, rmax)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_in_window() {
        let info = SeqInfo { fps: 10.0, flags: 0, numframes: 11, linear_movement: [0.0; 3] };
        let evs = [StudioEvent { frame: 2, event: 1 }, StudioEvent { frame: 5, event: 5001 }, StudioEvent { frame: 8, event: 2 }];
        // 0..256 maps onto frames 0..10.
        assert_eq!(events_in_range(&info, &evs, 0.0, 64.0), [1]);
        assert_eq!(events_in_range(&info, &evs, 64.0, 230.0), [2]);
        // Client events (>= 5000) are skipped.
        assert_eq!(events_in_range(&info, &evs, 100.0, 150.0), Vec::<i32>::new());
        // Looping sequences wrap past the last frame.
        let looping = SeqInfo { flags: STUDIO_LOOPING, ..info };
        assert_eq!(events_in_range(&looping, &evs, 240.0, 320.0), [1]);
    }

    #[test]
    fn controller_bytes() {
        let c = BoneController { kind: 0x10, start: -30.0, end: 30.0, index: 0 };
        assert_eq!(controller_setting(&c, 0.0), 127);
        assert_eq!(controller_setting(&c, -90.0), 0);
        assert_eq!(controller_setting(&c, 90.0), 255);
        // Non-rotational controllers aren't wrapped.
        let m = BoneController { kind: 0x1, start: 0.0, end: 64.0, index: 0 };
        assert_eq!(controller_setting(&m, 32.0), 127);
    }

    #[test]
    fn sequence_box_rotation() {
        let (lo, hi) = rotated_box([-10.0, -20.0, 5.0], [10.0, 20.0, 70.0], 90.0);
        assert!((lo[0] + 20.0).abs() < 1e-4 && (hi[0] - 20.0).abs() < 1e-4);
        assert!((lo[1] + 10.0).abs() < 1e-4 && (hi[1] - 10.0).abs() < 1e-4);
        assert_eq!((lo[2], hi[2]), (0.0, 1.0));
    }
}
