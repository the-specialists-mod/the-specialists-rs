//! Motion blur (`cl_motionblur`): not a screen blur but afterimage ghosts, translucent frozen copies
//! of a player left behind every `0.03 / ts` seconds while the game runs slowed (R§7.6). The server
//! asks for them with `TSMBlur` (slow-motion power-up, stunts, kills, bullet time). The ghosts are
//! temp entities with renderfx 17 and `rendercolor = (player, 0, 1)`, which the TS studio renderer
//! draws as `StudioDrawGhostPlayer` (`crate::studio`).

use goldsrc::client::engine;
use goldsrc::ext::studio::{self as es, Model};
use goldsrc::sys::*;

/// One player's trail (TS: 32 entries x 24 bytes at `0x100dff48`), indexed by entity number.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Trail {
    active: bool,
    count: i32,
    max: i32,
    next: f64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct MotionBlur {
    trails: [Trail; 32],
}

/// The ghosts start at this alpha (`renderamt 120`, kept in `fuser2`).
const GHOST_ALPHA: f32 = 120.0;

impl MotionBlur {
    /// `MotionBlur_Add` (`0x10029680`, from `TSMBlur` `0x1004bf90`): start a trail on `ent`, or
    /// lengthen a running one by 5 ghosts. The level is capped by `cl_fx`: 1 -> 10 ghosts,
    /// 2 -> 15, anything else 5. `now` is `gHUD.m_flTime` (`0x100cf944`).
    pub fn add(&mut self, ent: usize, level: i8, cl_motionblur: f32, cl_fx: f32, now: f32) {
        if cl_motionblur as i32 == 0 {
            return;
        }
        // TS indexes its 32-entry table without a check: entity 32 and up would write past it,
        // so they are dropped.
        let Some(t) = self.trails.get_mut(ent) else { return };
        if t.active {
            t.max += 5;
            return;
        }
        // The handler sign-extends the level byte (`movsx edx, al` at `0x1004bfaf`).
        let lvl = (level as f32).min(cl_fx) as i32;
        t.max = match lvl {
            1 => 10,
            2 => 15,
            _ => 5,
        };
        t.next = now as f64;
        t.active = true;
        t.count = 0;
    }

    /// The scheduling half of `MotionBlur_Update` (`0x10029780`): the entities due a ghost now
    /// (`gHUD.m_flTime`; the stored double is compared as a float). Trails stop as soon as the
    /// game is back to (nearly) normal speed.
    pub fn due(&mut self, cl_motionblur: f32, ts: f32, now: f32) -> Vec<usize> {
        if cl_motionblur as i32 == 0 {
            return Vec::new();
        }
        let mut out = Vec::new();
        for (i, t) in self.trails.iter_mut().enumerate() {
            if !t.active || t.next as f32 > now {
                continue;
            }
            if ts > 0.99 {
                t.active = false;
                continue;
            }
            // `0x10056fa0`: 0.03 * (1 / max(ts, 0.01))
            t.next = (0.03 * (1.0 / ts.max(0.01)) + now) as f64;
            out.push(i);
        }
        out
    }

    /// A ghost was made for `ent`: count it, and end the trail at its maximum.
    pub fn spawned(&mut self, ent: usize) {
        if let Some(t) = self.trails.get_mut(ent) {
            t.count += 1;
            if t.count >= t.max {
                t.active = false;
            }
        }
    }
}

/// The spawning half of `MotionBlur_Update`: a temp entity copying the player's latest state,
/// angles and gait (`iuser1` gaitsequence, `iuser2` renderframe, `fuser3` gaitframe, `fuser4`
/// gaityaw), frozen (`framerate 0`), translucent (`rendermode 1`, `renderamt 120`). True when the
/// ghost was completed (TS only counts those). A trail on entity 0 allocates a temp entity with
/// the world model and stops at `GetPlayerState(-1)`, which the engine refuses, as in TS.
pub fn spawn_ghost(ent: usize) -> bool {
    let idx = ent as i32;
    let Some(e) = engine().entity(idx) else { return false };
    let Some(model) = Model::from_ptr(e.model) else { return false };
    let (origin, angles) = (e.origin, e.angles);
    let Some(t) = es::alloc_custom_tempent(origin, model, false, ghost_think) else { return false };
    let Some(state) = es::player_state(idx - 1) else { return false };
    let g = &mut t.entity;
    g.curstate = state;
    g.angles = angles;
    g.curstate.number = idx;
    g.curstate.rendercolor.r = idx as u8;
    g.curstate.framerate = 0.0;
    g.curstate.rendermode = kRenderTransColor as i32;
    g.curstate.renderamt = GHOST_ALPHA as i32;
    g.curstate.fuser2 = GHOST_ALPHA;
    g.curstate.rendercolor.b = 1;
    g.curstate.renderfx = kRenderFxDeadPlayer as i32;
    g.curstate.iuser4 = 0;
    let Some(info) = es::player_info(idx - 1) else { return false };
    g.curstate.iuser1 = info.gaitsequence;
    g.curstate.iuser2 = info.renderframe;
    g.curstate.fuser3 = info.gaitframe;
    g.curstate.fuser4 = info.gaityaw;
    g.curstate.origin = origin;
    // The temp-entity loop moves tempents by baseline.origin (their velocity): keep it still.
    g.baseline.origin = [0.0; 3];
    t.die += 1.0;
    true
}

/// The fade (`0x10029710`): `fuser2 -= 255 ft / max(ts, 0.01)`; `renderamt` follows; the ghost
/// dies at 0. `frametime` is the temp-entity loop's (already slowed by the time scale).
fn ghost_think(t: &mut TEMPENTITY, frametime: f32, client_time: f32) {
    let ts = crate::state::with(|s| s.time_scale);
    let (alpha, die) = fade(t.entity.curstate.fuser2, frametime, ts, client_time);
    t.entity.curstate.fuser2 = alpha;
    t.entity.curstate.renderamt = (alpha as i32).max(0);
    t.die = die;
}

/// (new `fuser2`, new `die`) for one frame of the ghost fade.
fn fade(alpha: f32, frametime: f32, ts: f32, client_time: f32) -> (f32, f32) {
    let alpha = alpha - frametime * 255.0 / ts.max(0.01);
    (alpha, if alpha as i32 > 0 { client_time + 1.0 } else { client_time - 1.0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trail_lengths_and_extension() {
        let mut m = MotionBlur::default();
        m.add(3, 2, 1.0, 1.0, 10.0); // cl_fx 1 caps level 2 at 1
        assert_eq!(m.trails[3].max, 10);
        m.add(3, 2, 1.0, 1.0, 11.0);
        assert_eq!(m.trails[3].max, 15, "a running trail grows by 5");
        assert_eq!(m.trails[3].next, 10.0, "and keeps its schedule");
        m.add(4, 2, 1.0, 2.0, 0.0);
        assert_eq!(m.trails[4].max, 15);
        m.add(5, 0, 1.0, 2.0, 0.0);
        assert_eq!(m.trails[5].max, 5);
        m.add(6, 1, 0.0, 2.0, 0.0);
        assert!(!m.trails[6].active, "cl_motionblur 0");
        m.add(32, 1, 1.0, 1.0, 0.0); // out of the table: ignored
        m.add(7, 200u8 as i8, 1.0, 2.0, 0.0); // the level byte is signed: -56 -> 5 ghosts
        assert_eq!(m.trails[7].max, 5);
    }

    #[test]
    fn schedule_follows_time_scale() {
        let mut m = MotionBlur::default();
        m.add(1, 1, 1.0, 1.0, 0.0);
        assert_eq!(m.due(1.0, 0.5, 0.0), vec![1]);
        assert!((m.trails[1].next - 0.06).abs() < 1e-6, "0.03 / ts");
        assert!(m.due(1.0, 0.5, 0.05).is_empty());
        assert_eq!(m.due(1.0, 0.5, 0.06), vec![1]);
        for _ in 0..10 {
            m.spawned(1);
        }
        assert!(!m.trails[1].active, "ends after max ghosts");

        m.add(2, 1, 1.0, 1.0, 0.0);
        assert!(m.due(1.0, 1.0, 0.0).is_empty());
        assert!(!m.trails[2].active, "normal speed ends the trail");
    }

    #[test]
    fn ghost_fades_in_real_time() {
        // ts 0.5: the loop's frametime is already halved, so 0.1 s real = 0.05 here.
        let (a, die) = fade(120.0, 0.05, 0.5, 3.0);
        assert!((a - (120.0 - 25.5)).abs() < 1e-3);
        assert_eq!(die, 4.0);
        let (a, die) = fade(10.0, 0.05, 0.5, 3.0);
        assert!(a < 0.0);
        assert_eq!(die, 2.0, "dies once transparent");
    }
}
