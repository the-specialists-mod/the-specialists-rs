//! `CBotBrain` (bots.md §5): the per-bot AI object TS embeds in `CBot` at `bot+0xb78`. Button
//! hold timers, the module mask and its fixed order, the view turning (`UpdateView`) and the
//! usercmd (`RunMove`).
//!
//! The brain lives in `Player::bot` (the `CBot` part of the player). [`think`] takes it out for
//! the frame and puts it back around the one call that can re-enter the bot (the respawn's
//! `StopObserver` → `Spawn` → `CBotBrain::Reset`), so every module gets the brain, the player and
//! the entity context side by side.

use super::{attack, modules, nav, states};
use crate::player::Player;
use crate::world::Cx;
use goldsrc::server::engine;
use goldsrc::sys::*;

/// Module bits (`m_activeModules`, brain+0x1924).
pub mod bit {
    pub const NAVIGATE: u32 = 0x1;
    pub const ATTACK: u32 = 0x4;
    pub const STUNT: u32 = 0x8;
    pub const BUY: u32 = 0x10;
    pub const VISION: u32 = 0x40;
    pub const LOOK_AROUND: u32 = 0x80;
    pub const RESPAWN: u32 = 0x100;
    pub const POWERUP: u32 = 0x200;
    pub const CHAT: u32 = 0x400;
}

/// The usercmd buttons the modules press (raw `usercmd` bits; bots.md §5.6).
pub mod btn {
    pub const ATTACK: u32 = 0x1;
    pub const JUMP: u32 = 0x2;
    pub const DUCK: u32 = 0x4;
    pub const FORWARD: u32 = 0x8;
    pub const BACK: u32 = 0x10;
    /// `+alt2` (`IN_CANCEL`): TS's stock hit.
    pub const ALT2: u32 = 0x40;
    pub const ATTACK2: u32 = 0x800;
    pub const RELOAD: u32 = 0x2000;
    /// The stunt key.
    pub const ALT1: u32 = 0x4000;
}

/// The strategy object `m_pState` points at (brain+0x24), chosen in `Reset` by the TS mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StateKind {
    /// `CBotStateDM` (brain+0x1888, 0x1006e900): modes 0 and 1.
    #[default]
    Dm,
    /// `CBotStateTeamRounds` (brain+0x1894, 0x1006ed00): mode 2 (unreachable in TS 3.0).
    TeamRounds,
    /// `CBotStateObjective` (brain+0x18a0, 0x1006f0c0): mode 3 (unreachable in TS 3.0).
    Objective,
}

/// A state object (`+4` next think, `+8` next roam-goal pick). One per state, as in TS.
#[derive(Clone, Copy, Debug, Default)]
pub struct StateObj {
    pub next_think: f32,
    pub next_pick: f32,
}

/// `CBotBrain` (0x1a78 bytes). Field names follow bots.md §5.1.
pub struct Brain {
    /// brain+0x0/+0x4/+0x8: `RunMove`'s frame counter, window end and msec.
    pub msec_frames: i32,
    pub msec_next: f32,
    pub msec: f32,
    /// brain+0xc: wanted view angles (pitch already negated), written only by [`Brain::look_at`].
    pub ideal: [f32; 3],
    /// brain+0x18: `UpdateView`'s low-pass of `pev->punchangle`.
    pub punch_accum: [f32; 3],
    /// brain+0x24.
    pub state: StateKind,
    /// brain+0x1888 / 0x1894 / 0x18a0, indexed by [`StateKind`].
    pub states: [StateObj; 3],
    /// brain+0x28: the path follower.
    pub nav: nav::Nav,
    /// brain+0x18ac: `CBotModuleAttack`.
    pub attack: attack::Attack,
    /// brain+0x18f8 / 0x1900: vision and look-around next think.
    pub vision_next: f32,
    pub look_next: f32,
    /// brain+0x1908 / 0x190c / 0x190d: stunt next think, mode, jumps.
    pub stunt_next: f32,
    pub stunt_mode: u8,
    pub stunt_jumps: u8,
    /// brain+0x1914 / 0x1918: power-up next think and its unused byte.
    pub powerup_next: f32,
    pub powerup_flag: u8,
    /// brain+0x1920: the chat module's next line.
    pub next_chat: f32,
    /// brain+0x1924.
    pub modules: u32,
    /// brain+0x1928: navigation goal, zero = none.
    pub goal: [f32; 3],
    /// brain+0x1934: walk straight to the goal without nodes (set by the attack module).
    pub go_straight: bool,
    /// brain+0x1938: the enemy (a player's edict; TS keeps the `CBaseEntity *`).
    pub enemy: *mut edict_t,
    /// brain+0x193c: written by the states, never read.
    pub keep_goal: u8,
    /// brain+0x193d: the name for the next chat line's `%s` (`char[67]`).
    pub chat_name: Vec<u8>,
    /// brain+0x1980: chat section (0 offense, 1 freetime, 2 killed).
    pub chat_section: i32,
    /// brain+0x1984: read by the chat module, never set.
    pub chat_now: bool,
    /// brain+0x1988 / 0x19c8 / 0x1a08: what vision saw.
    pub visible_enemies: [*mut edict_t; 16],
    pub visible_mates: [*mut edict_t; 16],
    pub num_visible_enemies: i8,
    /// brain+0x1a0c: where an enemy was last seen, zero = none.
    pub last_seen: [f32; 3],
    /// brain+0x1a18: zeroed by `Reset`, otherwise unused.
    pub unused_1a18: [f32; 3],
    /// brain+0x1a24 / 0x1a28: `SetMove`.
    pub forward: f32,
    pub side: f32,
    /// brain+0x1a2c: rebuilt each frame from the timers.
    pub buttons: u32,
    /// brain+0x1a30: button bit `1 << i` is held while `time < button_expire[i]`.
    pub button_expire: [f32; 16],
    /// brain+0x1a70: `clamp(skill, 0.5, 2) * 10`.
    pub turn_rate: f32,
    /// brain+0x1a74: at least 0.1.
    pub skill: f32,
}

impl Brain {
    /// The zero-filled object `GetClassPtr<CBot>` (0x1006c240) leaves, with the skill
    /// `BotSetup` stores.
    pub fn new(skill: f32) -> Brain {
        Brain {
            msec_frames: 0,
            msec_next: 0.0,
            msec: 0.0,
            ideal: [0.0; 3],
            punch_accum: [0.0; 3],
            state: StateKind::Dm,
            states: [StateObj::default(); 3],
            nav: nav::Nav::default(),
            attack: attack::Attack::default(),
            vision_next: 0.0,
            look_next: 0.0,
            stunt_next: 0.0,
            stunt_mode: 0,
            stunt_jumps: 0,
            powerup_next: 0.0,
            powerup_flag: 0,
            next_chat: 0.0,
            modules: 0,
            goal: [0.0; 3],
            go_straight: false,
            enemy: std::ptr::null_mut(),
            keep_goal: 0,
            chat_name: Vec::new(),
            chat_section: 0,
            chat_now: false,
            visible_enemies: [std::ptr::null_mut(); 16],
            visible_mates: [std::ptr::null_mut(); 16],
            num_visible_enemies: 0,
            last_seen: [0.0; 3],
            unused_1a18: [0.0; 3],
            forward: 0.0,
            side: 0.0,
            buttons: 0,
            button_expire: [0.0; 16],
            turn_rate: 0.0,
            skill: if skill > 0.1 { skill } else { 0.1 },
        }
    }

    /// `CBotBrain::Reset` (0x1006f2c0), from `CBot::Spawn` before `CBasePlayer::Spawn`.
    /// Not reset: the ideal angles, the punch accumulator, the module objects (timers), the
    /// visible lists and the skill.
    pub fn reset(&mut self, now: f32, mode: crate::rules::state::Mode) {
        // memset(brain+0x1a24, 0, 0x50): move, buttons, timers, turn rate.
        self.forward = 0.0;
        self.side = 0.0;
        self.buttons = 0;
        self.button_expire = [0.0; 16];
        self.turn_rate = 0.0;
        // memset(brain+0x1924, 0, 0x64): mask, goal, go-straight, enemy, chat.
        self.modules = 0;
        self.goal = [0.0; 3];
        self.go_straight = false;
        self.enemy = std::ptr::null_mut();
        self.keep_goal = 0;
        self.chat_name.clear();
        self.chat_now = false;
        self.chat_section = super::files::FREETIME;
        let s = clamp_skill(self.skill, 1.0);
        self.next_chat = (engine().random_float(60.0, 180.0) as f64 * s as f64 + now as f64) as f32;
        self.nav.reset();
        self.last_seen = [0.0; 3];
        self.unused_1a18 = [0.0; 3];
        // ResetMsec (0x1006e4b0).
        self.msec_frames = 0;
        self.msec_next = 0.0;
        self.msec = 0.0;
        use crate::rules::state::Mode;
        self.state = match mode {
            Mode::Dm | Mode::TeamDm => StateKind::Dm,
            Mode::RoundTdm => StateKind::TeamRounds,
            Mode::Objective => StateKind::Objective,
        };
    }

    /// `TestModules` (0x1006f410).
    pub fn test(&self, bits: u32) -> bool {
        self.modules & bits != 0
    }
    /// `SetModules` (0x1006f420): replaces the mask.
    pub fn set_modules(&mut self, bits: u32) {
        self.modules = bits;
    }
    /// `EnableModules` (0x1006f430).
    pub fn enable(&mut self, bits: u32) {
        self.modules |= bits;
    }
    /// `DisableModules` (0x1006f450).
    pub fn disable(&mut self, bits: u32) {
        self.modules &= !bits;
    }

    /// `PressButtons(mask, duration)` (0x1006f470): hold each bit until `time + duration`
    /// (0 releases it now).
    pub fn press(&mut self, mask: u32, duration: f32, now: f32) {
        for i in 0..16 {
            if mask & (1 << i) != 0 {
                self.button_expire[i] = (duration as f64 + now as f64) as f32;
            }
        }
    }

    /// `UpdateButtons` (0x1006f4b0), at the start of `Think`: a press made in frame N shows in
    /// frame N+1.
    fn update_buttons(&mut self, now: f32) {
        self.buttons = 0;
        for i in 0..16 {
            if now < self.button_expire[i] {
                self.buttons |= 1 << i;
            }
        }
    }

    /// `SetMove(fwd, side)` (0x1006f7f0).
    pub fn set_move(&mut self, forward: f32, side: f32) {
        self.forward = forward;
        self.side = side;
    }

    /// `NumVisibleEnemies` (0x1006f730).
    pub fn num_enemies(&self) -> i8 {
        self.num_visible_enemies
    }

    /// `LookAt(pl, target)` (0x1006f740): the wanted view toward `target` from the gun position.
    pub fn look_at(&mut self, pev: &entvars_t, target: [f32; 3]) {
        let src = gun_position(pev);
        let mut a = engine().vec_to_angles(sub(target, src));
        if a[0] >= 180.0 {
            a[0] -= 360.0;
        }
        if a[0] <= -180.0 {
            a[0] += 360.0;
        }
        a[0] = -a[0];
        self.ideal = a; // SetIdealAngles (0x1006e4e0)
    }
}

/// `clamp(skill, 0.5, hi)` as `Reset` (hi 1) and `Think` (hi 2) write it.
pub fn clamp_skill(skill: f32, hi: f32) -> f32 {
    if skill > 0.5 && skill >= hi {
        hi
    } else if skill <= 0.5 {
        0.5
    } else {
        skill
    }
}

/// `CBasePlayer::GiveSlowMul` (0x10061080): the TS time scale `max(fuser1, 0.01)`.
pub fn slow(pev: &entvars_t) -> f32 {
    crate::weapons::give_slow_mul(pev.fuser1)
}

/// The modules' reschedule formula (bots.md §5.8): `time + min(k / slow / skill, max)`, in the
/// x87's precision and stored as a float.
pub fn delay(now: f32, k: f64, slow: f32, skill: f32, max: f64) -> f32 {
    let x = k / slow as f64 / skill as f64;
    (now as f64 + if x >= max { max } else { x }) as f32
}

/// `CBasePlayer::GetGunPosition`: origin + view_ofs.
pub fn gun_position(pev: &entvars_t) -> [f32; 3] {
    add(pev.origin, pev.view_ofs)
}

/// `UTIL_AngleMod` (0x10076250): into [0, 360) by whole turns (`a + 360 * (1 - (int)(a / 360))`
/// below 0, `a - 360 * (int)(a / 360)` from 360).
pub fn angle_mod(a: f32) -> f32 {
    let inv = (1.0f32 / 360.0) as f64;
    if a < 0.0 {
        let n = (a as f64 * inv).trunc() as i32;
        ((1 - n) as f64 * 360.0 + a as f64) as f32
    } else if a >= 360.0 {
        let n = (a as f64 * inv).trunc() as i32;
        (a as f64 - (n as f64 * 360.0)) as f32
    } else {
        a
    }
}

pub fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub fn scale(a: [f32; 3], k: f32) -> [f32; 3] {
    [a[0] * k, a[1] * k, a[2] * k]
}
/// `Vector::Length()`: the sum of squares in the x87's precision, stored as a float.
pub fn length(a: [f32; 3]) -> f32 {
    let (x, y, z) = (a[0] as f64, a[1] as f64, a[2] as f64);
    (x * x + y * y + z * z).sqrt() as f32
}
pub fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] as f64 * b[0] as f64 + a[1] as f64 * b[1] as f64 + a[2] as f64 * b[2] as f64) as f32
}
/// `Vector::Normalize()`: (0, 0, 1) for a zero vector.
pub fn normalize(a: [f32; 3]) -> [f32; 3] {
    let l = length(a);
    if l == 0.0 { [0.0, 0.0, 1.0] } else { [a[0] / l, a[1] / l, a[2] / l] }
}
pub fn is_zero(a: [f32; 3]) -> bool {
    a == [0.0; 3]
}

/// What `RunMove` hands to `pfnRunPlayerMove` (sent by the caller, outside the player's
/// dispatch, since the engine runs the player's think from it).
pub struct Usercmd {
    pub angles: [f32; 3],
    pub forward: f32,
    pub side: f32,
    pub buttons: u16,
    pub msec: u8,
}

/// `CBotBrain::Think` (0x1006f500) for one bot, up to the `pfnRunPlayerMove` call it ends with,
/// which is returned. `frozen` is the port's `ts_bot_stop 1`: a living bot then skips the AI
/// and sends no movement or buttons (dead and observing bots still buy and respawn).
pub fn think(p: &mut Player, cx: &mut Cx, frozen: bool) -> Option<Usercmd> {
    let mut b = p.bot.take()?;
    let cmd = think_with(&mut b, p, cx, frozen);
    p.bot = Some(b);
    Some(cmd)
}

fn think_with(b: &mut Box<Brain>, p: &mut Player, cx: &mut Cx, frozen: bool) -> Usercmd {
    let now = cx.time();
    b.update_buttons(now);
    b.turn_rate = (clamp_skill(b.skill, 2.0) as f64 * 10.0) as f32;
    let playing = {
        let v = cx.pev();
        crate::rules::alive(v) && v.iuser1 == 0
    };
    let frozen = frozen && playing;
    if frozen {
        b.buttons = 0;
        b.set_move(0.0, 0.0);
    } else if slow(cx.pev()) > 0.1 {
        states::think(b, p, cx);
        if b.test(bit::BUY) {
            modules::buy(b, p, cx);
        }
        if b.test(bit::RESPAWN) {
            // The respawn module, inline (0x1006f5b8): what the `respawn` command does. Its
            // `StopObserver` respawns the bot, and `CBot::Spawn` resets this brain.
            if p.respawn_at > 0.0 && p.respawn_at < now {
                p.respawn_at = 0.0;
                p.bot = Some(std::mem::replace(b, Box::new(Brain::new(0.1))));
                crate::rules::lifecycle::stop_observer(p, cx);
                if let Some(back) = p.bot.take() {
                    *b = back;
                }
                cx.pev().impulse = 0;
            }
            b.set_move(0.0, 0.0);
        }
        if b.test(bit::VISION) {
            modules::vision(b, p, cx);
        }
        if b.test(bit::ATTACK) {
            attack::think(b, p, cx);
        }
        if b.test(bit::LOOK_AROUND) {
            modules::look_around(b, p, cx);
        }
        if b.test(bit::STUNT) {
            modules::stunt(b, p, cx);
        }
        if b.test(bit::POWERUP) {
            modules::powerup(b, p, cx);
        }
        if b.test(bit::CHAT) {
            modules::chat(b, p, cx);
        }
        if b.test(bit::NAVIGATE) {
            let goal = b.goal;
            let r = nav::navigate(b, p, cx, goal);
            if r == nav::ARRIVED || r == nav::FAILED {
                b.disable(bit::NAVIGATE);
                b.goal = [0.0; 3];
            }
        }
    }
    cx.pev().button = b.buttons as i32;
    run_move(b, p, cx)
}

/// `CBotBrain::RunMove` (0x1006e6d0): `UpdateView`, then the msec estimate, `450 / frames` over
/// half-second windows (0.9 of the frame time) kept in 5..100.
fn run_move(b: &mut Brain, _p: &mut Player, cx: &mut Cx) -> Usercmd {
    let turn_rate = b.turn_rate;
    update_view(b, cx.pev(), turn_rate);
    let now = cx.time();
    if now < b.msec_next {
        b.msec_frames += 1;
    } else {
        b.msec_next = (now as f64 + 0.5) as f32;
        if b.msec_frames > 0 {
            b.msec = (450.0 / b.msec_frames as f64) as f32;
        }
        b.msec_frames = 0;
    }
    b.msec = b.msec.clamp(5.0, 100.0);
    let v = cx.pev();
    Usercmd { angles: v.v_angle, forward: b.forward, side: b.side, buttons: v.button as u16, msec: b.msec as u8 }
}

/// `CBotBrain::UpdateView` (0x1006e500): turn toward the wanted angles by `frametime *
/// turnRate` of the way, with the punch angle through a low-pass. Nothing while dead.
pub fn update_view(b: &mut Brain, v: &mut entvars_t, turn_rate: f32) {
    if v.deadflag != 0 {
        return;
    }
    let ft = engine().globals().frametime;
    let mut k = ft * 15.0;
    if 1.0 <= k {
        k = 1.0;
    }
    let mut d = [0.0f32; 3];
    for ((d, acc), punch) in d.iter_mut().zip(b.punch_accum.iter_mut()).zip(v.punchangle) {
        *d = (punch as f64 - k as f64 * *acc as f64) as f32;
        *acc = (*d as f64 + *acc as f64) as f32;
    }
    let cur = angle_mod(v.angles[1]);
    let ideal = angle_mod(b.ideal[1]);
    let mut dy = ideal as f64 - cur as f64;
    if ideal <= cur {
        if dy <= -180.0 {
            dy += 360.0;
        }
    } else if 180.0 <= dy {
        dy -= 360.0;
    }
    let mut t = turn_rate as f64 * ft as f64;
    if 1.0 <= t {
        t = 1.0;
    }
    v.angles[1] = angle_mod((t * dy + d[1] as f64 + cur as f64) as f32);
    v.v_angle[1] = v.angles[1];
    let mut t = turn_rate * ft;
    if 1.0 <= t {
        t = 1.0;
    }
    let pitch = v.v_angle[0];
    v.v_angle[0] = (t as f64 * (b.ideal[0] as f64 - pitch as f64) + d[0] as f64 + pitch as f64) as f32;
    v.angles[0] = v.v_angle[0] * -0.333_333_34;
    v.angles[2] = 0.0;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn angle_mod_whole_turns() {
        assert_eq!(angle_mod(10.0), 10.0);
        assert_eq!(angle_mod(-10.0), 350.0);
        assert_eq!(angle_mod(-370.0), 350.0);
        assert_eq!(angle_mod(360.0), 0.0);
        assert_eq!(angle_mod(725.0), 5.0);
    }

    #[test]
    fn skill_clamps() {
        assert_eq!(clamp_skill(0.1, 2.0), 0.5);
        assert_eq!(clamp_skill(1.3, 2.0), 1.3);
        assert_eq!(clamp_skill(3.0, 2.0), 2.0);
        assert_eq!(clamp_skill(3.0, 1.0), 1.0);
        assert_eq!(clamp_skill(0.7, 1.0), 0.7);
    }

    #[test]
    fn delays() {
        // min(0.1 / slow / skill, 1.0)
        assert_eq!(delay(10.0, 0.1, 1.0, 1.0, 1.0), 10.1);
        assert_eq!(delay(10.0, 0.1, 0.05, 1.0, 1.0), 11.0);
        assert_eq!(delay(0.0, 3.0, 1.0, 2.0, 7.0), 1.5);
    }

    #[test]
    fn buttons_hold_and_show_a_frame_late() {
        let mut b = Brain::new(1.0);
        b.press(btn::ATTACK | btn::DUCK, 0.5, 10.0);
        b.update_buttons(10.0);
        assert_eq!(b.buttons, btn::ATTACK | btn::DUCK);
        b.press(btn::DUCK, 0.0, 10.2);
        b.update_buttons(10.2);
        assert_eq!(b.buttons, btn::ATTACK);
        b.update_buttons(10.5);
        assert_eq!(b.buttons, 0);
        assert_eq!(Brain::new(0.01).skill, 0.1);
    }
}
