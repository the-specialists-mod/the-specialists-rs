//! Footsteps: `PM_UpdateStepSound` (0xfcc20), `PM_PlayStepSound` (0xfc344),
//! `PM_MapTextureTypeStepType` (0xfca90).
//!
//! TS changes: the step interval is divided by the time scale (sparser steps in slow motion),
//! steps are silent while `iuser4 & (RECOVER | 0x200)`, attenuation is 1.25 (SDK `ATTN_NORM`),
//! and `mp_footsteps` (movevars) is not checked.

use super::{CHAN_BODY, Pm, round_int};
use goldsrc_sys::{CONTENTS_WATER, MOVETYPE_FLY};
use std::sync::atomic::{AtomicI32, Ordering::Relaxed};

pub const STEP_CONCRETE: i32 = 0;
pub const STEP_METAL: i32 = 1;
pub const STEP_DIRT: i32 = 2;
pub const STEP_VENT: i32 = 3;
pub const STEP_GRATE: i32 = 4;
pub const STEP_TILE: i32 = 5;
pub const STEP_SLOSH: i32 = 6;
pub const STEP_WADE: i32 = 7;
pub const STEP_LADDER: i32 = 8;

const STEP_ATTN: f32 = 1.25;

/// `iSkipStep` (static in `PM_PlayStepSound`).
static SKIP_STEP: AtomicI32 = AtomicI32::new(0);

pub fn map_texture_type_step_type(t: u8) -> i32 {
    match t {
        b'D' => STEP_DIRT,
        b'G' => STEP_GRATE,
        b'M' => STEP_METAL,
        b'S' => STEP_SLOSH,
        b'T' => STEP_TILE,
        b'V' => STEP_VENT,
        _ => STEP_CONCRETE,
    }
}

impl Pm<'_> {
    pub(crate) fn play_step_sound(&mut self, step: i32, fvol: f32) {
        self.p.iStepLeft = (self.p.iStepLeft == 0) as i32;
        if self.p.runfuncs == 0 {
            return;
        }
        let mut irand = self.random_long(0, 1) + self.p.iStepLeft * 2;
        if self.p.iuser4 & (crate::ts::stunt::RECOVER | crate::ts::stunt::HAS_KEVLAR) != 0 {
            return;
        }
        let hvel = [self.p.velocity[0], self.p.velocity[1], 0.0];
        if self.p.multiplayer != 0 && !self.onladder && super::length(hvel) <= 220.0 {
            return;
        }
        let sample = match step {
            STEP_METAL => [c"player/pl_metal1.wav", c"player/pl_metal3.wav", c"player/pl_metal2.wav", c"player/pl_metal4.wav"]
                .get(irand as usize)
                .copied(),
            STEP_DIRT => [c"player/pl_dirt1.wav", c"player/pl_dirt3.wav", c"player/pl_dirt2.wav", c"player/pl_dirt4.wav"]
                .get(irand as usize)
                .copied(),
            STEP_VENT => [c"player/pl_duct1.wav", c"player/pl_duct3.wav", c"player/pl_duct2.wav", c"player/pl_duct4.wav"]
                .get(irand as usize)
                .copied(),
            STEP_GRATE => [c"player/pl_grate1.wav", c"player/pl_grate3.wav", c"player/pl_grate2.wav", c"player/pl_grate4.wav"]
                .get(irand as usize)
                .copied(),
            STEP_TILE => {
                if self.random_long(0, 4) == 0 {
                    irand = 4;
                }
                [c"player/pl_tile1.wav", c"player/pl_tile3.wav", c"player/pl_tile2.wav", c"player/pl_tile4.wav", c"player/pl_tile5.wav"]
                    .get(irand as usize)
                    .copied()
            }
            STEP_SLOSH => [c"player/pl_slosh1.wav", c"player/pl_slosh3.wav", c"player/pl_slosh2.wav", c"player/pl_slosh4.wav"]
                .get(irand as usize)
                .copied(),
            STEP_WADE => {
                let skip = SKIP_STEP.load(Relaxed);
                if skip == 0 {
                    SKIP_STEP.store(1, Relaxed);
                    return;
                }
                SKIP_STEP.store(if skip + 1 == 4 { 0 } else { skip + 1 }, Relaxed);
                [c"player/pl_wade1.wav", c"player/pl_wade2.wav", c"player/pl_wade3.wav", c"player/pl_wade4.wav"]
                    .get(irand as usize)
                    .copied()
            }
            STEP_LADDER => [c"player/pl_ladder1.wav", c"player/pl_ladder3.wav", c"player/pl_ladder2.wav", c"player/pl_ladder4.wav"]
                .get(irand as usize)
                .copied(),
            _ => [c"player/pl_step1.wav", c"player/pl_step3.wav", c"player/pl_step2.wav", c"player/pl_step4.wav"]
                .get(irand as usize)
                .copied(),
        };
        if let Some(s) = sample {
            self.play_sound(CHAN_BODY, s, fvol, STEP_ATTN);
        }
    }

    pub(crate) fn update_step_sound(&mut self) {
        if self.p.flTimeStepSound > 0 {
            return;
        }
        if self.p.flags & goldsrc_sys::FL_FROZEN as i32 != 0 {
            return;
        }
        self.categorize_texture_type();
        let speed = super::length(self.p.velocity);
        let ladder = self.p.movetype == MOVETYPE_FLY as i32;
        let (velwalk, velrun, flduck) =
            if self.p.flags & goldsrc_sys::FL_DUCKING as i32 != 0 || ladder { (60.0f32, 80.0f32, 100.0f32) } else { (120.0, 210.0, 0.0) };
        if !((self.p.onground != -1 || ladder) && super::length(self.p.velocity) > 0.0 && (speed >= velwalk || self.p.flTimeStepSound == 0))
        {
            return;
        }
        let walking = speed < velrun;
        let usehull = self.p.usehull as usize & 3;
        let height = self.p.player_maxs[usehull][2] - self.p.player_mins[usehull][2];
        let knee = [self.p.origin[0], self.p.origin[1], self.p.origin[2] - 0.3 * height];
        let feet = [self.p.origin[0], self.p.origin[1], self.p.origin[2] - 0.5 * height];

        let (step, mut fvol);
        if ladder {
            step = STEP_LADDER;
            fvol = 0.35;
            self.p.flTimeStepSound = 350;
        } else if self.point_contents(knee, None) == CONTENTS_WATER {
            step = STEP_WADE;
            fvol = 0.65;
            self.p.flTimeStepSound = 600;
        } else if self.point_contents(feet, None) == CONTENTS_WATER {
            step = STEP_SLOSH;
            fvol = if walking { 0.2 } else { 0.5 };
            self.p.flTimeStepSound = if walking { 400 } else { 300 };
        } else {
            let t = self.p.chtexturetype as u8;
            step = map_texture_type_step_type(t);
            fvol = match t {
                b'D' => {
                    if walking {
                        0.25
                    } else {
                        0.55
                    }
                }
                b'V' => {
                    if walking {
                        0.4
                    } else {
                        0.7
                    }
                }
                _ => {
                    if walking {
                        0.2
                    } else {
                        0.5
                    }
                }
            };
            self.p.flTimeStepSound = if walking { 400 } else { 300 };
        }
        self.p.flTimeStepSound = round_int(flduck + self.p.flTimeStepSound as f32);
        // TS: footsteps get sparser in slow motion.
        let ts = self.p.fuser1.max(0.1);
        self.p.flTimeStepSound = round_int((1.0 / ts) * self.p.flTimeStepSound as f32);
        if self.p.flags & goldsrc_sys::FL_DUCKING as i32 != 0 {
            fvol *= 0.35;
        }
        self.play_step_sound(step, fvol);
    }
}
