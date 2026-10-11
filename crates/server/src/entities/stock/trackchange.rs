//! `func_trackchange` (`CFuncTrackChange`) and `func_trackautochange` (`CFuncTrackAuto`): toggle
//! `func_platrot`s that would carry a `func_tracktrain` between two `path_track`s.
//!
//! In TS they never do. `CFuncTrackChange::Spawn` (0xd5d84) installs `CFuncTrainControls::Find`
//! as the think (the GOT slot it loads at 0xc5e75 objdump relocates to `Find__18CFuncTrainControls`),
//! not `CFuncTrackChange::Find` (0xd5ff4). `CFuncTrackChange::Find`, the only code that looks up
//! the `toptrack` / `bottomtrack` paths and the `train`, is then reached only through
//! `CFuncTrackChange::OverrideReset` (0xd5fb0), which only save-game restore calls. So
//! `m_trackTop`, `m_trackBottom` and `m_train` (+0x12c..+0x134) stay NULL in multiplayer, and with
//! them:
//!
//! * `EvaluateTrain` (0xd6338) always returns `TRAIN_SAFE` (no path), so `m_code` stays 0: no
//!   `buttons/button11.wav` alarm, no `UpdateTrain` (0xd6458), no `SetTrack` on arrival.
//! * `UpdateAutoTargets` (0xd68b4, `CFuncTrackAuto`'s 0xd6b60) returns at once (no tracks).
//! * `CFuncTrackAuto::Use` (0xd6c48) evaluates a NULL path (from a train) or toggles nothing and
//!   updates nothing (otherwise): it never moves the platform.
//!
//! What's left is ported: a toggle plat that moves (or, with `SF_TRACK_DONT_MOVE`, only turns)
//! between its top and bottom on use, and a 2 s think that runs `CFuncTrainControls::Find` on
//! itself (a `target` naming a `func_tracktrain` hands that train this box as its control volume
//! and removes the track change; otherwise "No train" is logged).

use super::base::*;
use super::trains::*;
use crate::world::{Cx, Entity, UseType};
use goldsrc::ext::entities::Vec3;
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::ffi::CStr;

/// Start at the bottom (`m_vecPosition2`, `m_start` angles).
pub const SF_TRACK_STARTBOTTOM: i32 = 8;
/// Don't move: only turn (`AngularMove`), with the bottom at the spawn height.
pub const SF_TRACK_DONT_MOVE: i32 = 16;

/// `func_trackchange` (`AUTO = false`) and `func_trackautochange` (`AUTO = true`).
#[derive(Default)]
pub struct FuncTrackChange<const AUTO: bool> {
    p: PlatBase<Self>,
    /// `m_end` (+0x114): the angles at the top.
    end: Vec3,
    /// `m_start` (+0x120): the angles at the bottom.
    start: Vec3,
}

impl<const AUTO: bool> Mover for FuncTrackChange<AUTO> {
    fn toggle(&mut self) -> &mut Toggle<Self> {
        &mut self.p.t
    }
}

impl<const AUTO: bool> FuncTrackChange<AUTO> {
    /// The `Spawn` think: `CFuncTrainControls::Find` (see the module docs).
    fn find(&mut self, cx: &mut Cx) {
        super::tracktrain::train_controls_find(cx);
    }

    /// The `HitTop` / `HitBottom` tail: no `SetTrack` (no train), `SetThink(NULL)`,
    /// `nextthink = -1`; `UpdateAutoTargets` and `EnableUse` have nothing to act on.
    fn arrived(&mut self, cx: &mut Cx) {
        self.p.t.think = None;
        cx.pev().nextthink = -1.0;
    }
}

impl<const AUTO: bool> Plat for FuncTrackChange<AUTO> {
    fn plat(&mut self) -> &mut PlatBase<Self> {
        &mut self.p
    }

    /// `CFuncTrackChange::IsTogglePlat` (0xd7cb0): always, so no trigger volume and no automatic
    /// return from the top.
    fn is_toggle_plat(&self, _cx: &mut Cx) -> bool {
        true
    }

    /// `CFuncTrackChange::GoUp` (0xd67a4).
    fn go_up(&mut self, cx: &mut Cx) {
        if cx.pev().spawnflags & SF_TRACK_DONT_MOVE == 0 {
            plat_go_up(self, cx);
            self.p.t.move_done = Some(call_hit_top::<Self>);
            let t = move_time(cx);
            rot_move(cx, self.end, t);
        } else {
            self.p.t.state = ToggleState::GoingUp;
            self.p.t.move_done = Some(call_hit_top::<Self>);
            let (end, speed) = (self.end, cx.pev().speed);
            angular_move(self, cx, end, speed);
        }
    }

    /// `CFuncTrackChange::GoDown` (0xd6694).
    fn go_down(&mut self, cx: &mut Cx) {
        if cx.pev().spawnflags & SF_TRACK_DONT_MOVE == 0 {
            plat_go_down(self, cx);
            self.p.t.move_done = Some(call_hit_bottom::<Self>);
            let t = move_time(cx);
            rot_move(cx, self.start, t);
        } else {
            self.p.t.move_done = Some(call_hit_bottom::<Self>);
            self.p.t.state = ToggleState::GoingDown;
            let (start, speed) = (self.start, cx.pev().speed);
            angular_move(self, cx, start, speed);
        }
    }

    /// `CFuncTrackChange::HitTop` (0xd6a98): `CFuncPlatRot::HitTop`, then the tail.
    fn hit_top(&mut self, cx: &mut Cx) {
        plat_hit_top(self, cx);
        rot_settle(cx, self.end);
        self.arrived(cx);
    }

    /// `CFuncTrackChange::HitBottom` (0xd69f4): `CFuncPlatRot::HitBottom`, then the tail.
    fn hit_bottom(&mut self, cx: &mut Cx) {
        plat_hit_bottom(self, cx);
        rot_settle(cx, self.start);
        self.arrived(cx);
    }
}

impl<const AUTO: bool> Entity for FuncTrackChange<AUTO> {
    /// `CFuncTrackChange::KeyValue` (0xd5ed0). `train`, `toptrack` and `bottomtrack` are only read
    /// by the unreachable `CFuncTrackChange::Find`, so they're claimed and dropped.
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"train" | b"toptrack" | b"bottomtrack" => true,
            _ => plat_key_value(&mut self.p, key, value),
        }
    }

    /// `CFuncTrackChange::Spawn` (0xd5d84) and `Precache` (0xd5e94).
    fn spawn(&mut self, cx: &mut Cx) {
        plat_setup(self, cx);
        if cx.pev().spawnflags & SF_TRACK_DONT_MOVE != 0 {
            self.p.t.pos2[2] = cx.pev().origin[2];
        }
        let rotation = self.p.t.final_angle[0];
        (self.end, self.start) = setup_rotation(cx, rotation);
        let (pos, state, angles) = if cx.pev().spawnflags & SF_TRACK_STARTBOTTOM == 0 {
            (self.p.t.pos1, ToggleState::AtTop, self.end)
        } else {
            (self.p.t.pos2, ToggleState::AtBottom, self.start)
        };
        engine().set_origin(cx.edict, pos);
        self.p.t.state = state;
        cx.pev().angles = angles;
        think_ltime(cx, 2.0);
        self.p.t.think = Some(Self::find);
        engine().precache_sound(c"buttons/button11.wav");
        plat_precache(self, cx);
    }

    fn think(&mut self, cx: &mut Cx) {
        run_think(self, cx);
    }

    /// `CFuncTrackChange::Use` (0xd6918): at the top go down, at the bottom go up, moving ignore.
    /// (`EvaluateTrain` is always `TRAIN_SAFE`, see the module docs.)
    ///
    /// `CFuncTrackAuto::Use` (0xd6c48) does nothing reachable (module docs). TS reads the
    /// activator's classname first and so crashes on a NULL activator; the port doesn't.
    fn use_(&mut self, cx: &mut Cx, _a: *mut edict_t, _c: *mut edict_t, _kind: UseType, _v: f32) {
        if AUTO {
            return;
        }
        match self.p.t.state {
            ToggleState::AtTop => self.go_down(cx),
            ToggleState::AtBottom => self.go_up(cx),
            _ => {}
        }
    }

    /// `CFuncPlat::Blocked` (0xd2e64); `CFuncTrackChange::Touch` (0xd5ecc) is empty.
    fn blocked(&mut self, cx: &mut Cx, other: *mut edict_t) {
        plat_blocked(self, cx, other);
    }
}
