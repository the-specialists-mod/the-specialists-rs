//! TS 2.1's bots (issue #100 §3.10, decomp `docs/ts21/bots-entities.md` (b)): the native bot
//! backend's rules ([`NativeBotRules`]) for the 2.1 ruleset. `server-dll` registers [`BOTS`] under
//! the plugin's id, so the native bots play as 2.1's while `ts_gameplay` is 21; plugin backends
//! (`ts_bot_backend <name>`) aren't affected.
//!
//! Addresses are 2.1 `mp.dll`'s (`ts21/server-win`); every value was read from the decomp and
//! checked in the disassembly. 2.1's brain is 3.0's with these changes:
//!
//! * Creation (`BotCreate` 0x1005dc30, `BotCreateCustom` 0x1005da10, `BotSetup` 0x1005d8c0): a
//!   random model also in teamplay, no `botlearn 1`, the skill stored as given, no `team` key.
//!   The texts are 2.1's `bots.txt` and `bot_say.txt` (embedded: the game directory has 3.0's).
//! * `CBot` (vtable 0x10089220) overrides only `Spawn` (Reset + `CBasePlayer::Spawn`, no laser
//!   colour), `Killed` and `IsNetClient`: no `TraceAttack` reaction.
//! * `Think` (0x100614e0): the time-scale test is `fuser1 == 0 || fuser1 >= 0.1f`, `turnSpeed`
//!   is 30 every frame, and the dead bot respawns by holding `IN_ATTACK` (0.1 s).
//! * Timers: `time + min(slowInv * k / skill, max)` (`slowInv` = `FixedSlowMul`, 0x1005a3d0).
//! * The view: `LookAt` (0x100616b0) sets `v_angle` at once ([`set_ideal_angles`]); `RunMove`
//!   (0x100603d0) turns only the yaw, at `turnSpeed * frametime * 10` degrees ([`change_yaw`]).
//! * Buttons: 32 slots of `{mask, expire}` (`PressButtons` 0x100613e0, `ReleaseButton`
//!   0x10061450, `UpdateButtons` 0x10061490), held while `time <= expire`.
//! * The modules: [`attack`], vision, look around, buy, the stunt roll and the states.

pub mod attack;

use goldsrc::edict::EdictPtr;
use goldsrc::server::engine;
use goldsrc::sys::*;
use shared::weapons::id;
use ts_server::bots::brain::{Brain, StateKind, bit, btn, diff_x87, gun_position, is_zero, len_x87, order, sub};
use ts_server::bots::modules::{add_item, buy_weapon, in_view_cone, visible};
use ts_server::bots::rules::{BotTexts, NativeBotRules};
use ts_server::bots::states::{armament_empty, center, find_by_class, random_entity};
use ts_server::player::Player;
use ts_server::rules::alive;
use ts_server::world::{Cx, DamageInfo};

/// 2.1's native bot rules.
pub struct Bots;

/// What `server-dll` registers for the 2.1 ruleset.
pub static BOTS: Bots = Bots;

/// 2.1's `bots.txt` (22 names, 8 with a `//skill`, and 9 blank lines the loader skips) and
/// `bot_say.txt` (offense 10, freetime 11, killed 20 lines), byte for byte.
pub const TEXTS: BotTexts =
    BotTexts { names: include_bytes!("../../../data/bots21.txt"), say: include_bytes!("../../../data/bot_say21.txt") };

/// `FixedSlowMul(fuser1)` (0x1005a3d0) for the port's time scale (1.0 is normal speed in both
/// encodings' meaning; see `protocol::time_scale`).
pub fn slow_inv(pev: &entvars_t) -> f64 {
    crate::stunt::slow_inv(pev.fuser1)
}

/// 2.1's module timer: `time + min(slowInv * k / skill, max)`, the product and quotient on the
/// x87 (53-bit) and the sum stored as a float. `fcomp max; jp` takes `max` unless `x < max`
/// (also for a NaN).
pub fn timer(now: f32, k: f64, pev: &entvars_t, skill: f32, max: f64) -> f32 {
    let x = slow_inv(pev) * k / skill as f64;
    #[allow(clippy::neg_cmp_op_on_partial_ord)] // the x87 compare: NaN takes `max`
    let x = if !(x < max) { max } else { x };
    (now as f64 + x) as f32
}

/// 2.1's `UTIL_AngleMod` (0x100668f0), the SDK's: `a + 360 * ((int)(a / 360) + 1)` below 0 (so
/// it stays negative from -360 down), `a - 360 * (int)(a / 360)` from 360, with `a / 360` as
/// `a * ±(1/360)f` truncated. The result stays on the x87 stack.
pub fn angle_mod(a: f32) -> f64 {
    const INV: f32 = 1.0 / 360.0;
    if a < 0.0 {
        let m = (a as f64 * -(INV as f64)).trunc() as i32;
        (1i32.wrapping_sub(m)).wrapping_mul(360) as f64 + a as f64
    } else if a >= 360.0 {
        let m = (a as f64 * INV as f64).trunc() as i32;
        a as f64 - m.wrapping_mul(360) as f64
    } else {
        a as f64
    }
}

/// `SetIdealAngles` (0x10060090): `v_angle = a + punchangle` (each a float sum), the model pitch
/// a third of it, no roll, and `ideal_yaw` = `a.y` brought into [-180, 180] (0x10060020).
pub fn set_ideal_angles(pev: &mut entvars_t, a: [f32; 3]) {
    pev.v_angle = [pev.punchangle[0] + a[0], pev.punchangle[1] + a[1], pev.punchangle[2] + a[2]];
    pev.angles[0] = pev.v_angle[0] * -0.333_333_34;
    pev.angles[2] = 0.0;
    let mut y = a[1];
    if y > 180.0 {
        y -= 360.0;
    }
    if y < -180.0 {
        y += 360.0;
    }
    pev.ideal_yaw = y;
}

/// `LookAt` (0x100616b0): the angles from the gun position to `target`, pitch wrapped and
/// negated, then [`set_ideal_angles`].
pub fn look_at(pev: &mut entvars_t, target: [f32; 3]) {
    let mut a = engine().vec_to_angles(sub(target, gun_position(pev)));
    if 180.0 <= a[0] {
        a[0] -= 360.0;
    }
    if a[0] <= -180.0 {
        a[0] += 360.0;
    }
    a[0] = -a[0];
    set_ideal_angles(pev, a);
}

/// `ChangeYaw(pl, turnSpeed)` (0x10060120), `RunMove`'s view: move `angles.y` toward `ideal_yaw`
/// by at most `turnSpeed * frametime * 10` degrees the short way, and copy it to `v_angle.y`.
/// Nothing while dead or already there. The pitch isn't touched (`LookAt` set it).
pub fn change_yaw(pev: &mut entvars_t, turn_speed: f32, frametime: f32) {
    if pev.deadflag != 0 {
        return;
    }
    let cur = angle_mod(pev.angles[1]);
    let ideal = pev.ideal_yaw;
    if cur == ideal as f64 {
        return;
    }
    let speed = turn_speed as f64 * frametime as f64 * 10.0;
    let mut mv = (ideal as f64 - cur) as f32;
    if ideal as f64 <= cur {
        if mv <= -180.0 {
            mv += 360.0;
        }
    } else if mv >= 180.0 {
        mv -= 360.0;
    }
    if mv > 0.0 {
        if mv as f64 > speed {
            mv = speed as f32;
        }
    } else if (mv as f64) < -speed {
        mv = (-speed) as f32;
    }
    let y = angle_mod((mv as f64 + cur) as f32) as f32;
    pev.angles[1] = y;
    pev.v_angle[1] = y;
}

/// 2.1's button table (brain+0x1a08): 32 slots, one per distinct mask.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Buttons {
    pub mask: [u32; 32],
    pub expire: [f32; 32],
}

impl Buttons {
    /// `PressButtons(mask, dur)` (0x100613e0): the slot already holding `mask`, else the first
    /// free one, gets `time + dur`. With all 32 taken by other masks, nothing.
    pub fn press(&mut self, mask: u32, dur: f32, now: f32) {
        let mut free = None;
        for i in 0..32 {
            if self.mask[i] == 0 && free.is_none() {
                free = Some(i);
            }
            if self.mask[i] == mask {
                self.expire[i] = dur + now;
                return;
            }
        }
        if let Some(i) = free {
            self.mask[i] = mask;
            self.expire[i] = dur + now;
        }
    }

    /// `ReleaseButton(mask)` (0x10061450): clears the first slot holding exactly `mask`.
    pub fn release(&mut self, mask: u32) {
        if let Some(i) = self.mask.iter().position(|&m| m == mask) {
            self.mask[i] = 0;
            self.expire[i] = 0.0;
        }
    }

    /// `UpdateButtons` (0x10061490): the masks still held (`time <= expire`); expired slots are
    /// cleared.
    pub fn update(&mut self, now: f32) -> u32 {
        let mut held = 0;
        for i in 0..32 {
            if self.mask[i] != 0 {
                if now <= self.expire[i] {
                    held |= self.mask[i];
                } else {
                    self.mask[i] = 0;
                    self.expire[i] = 0.0;
                }
            }
        }
        held
    }
}

/// The bot's button table, kept in the brain's ruleset slot (`Reset` drops it with the memset).
pub fn buttons(b: &mut Brain) -> &mut Buttons {
    if !b.ext.as_ref().is_some_and(|e| e.is::<Buttons>()) {
        b.ext = Some(Box::<Buttons>::default());
    }
    b.ext.as_mut().and_then(|e| e.downcast_mut::<Buttons>()).expect("just set")
}

/// 2.1's `RandomWanderPoint` trace end: x at 53-bit precision (`dir.x * dist` is exact there,
/// `dist` being an integer below 2^13), rounded once; y and z rounded after the product too.
pub fn wander_end(dir: [f32; 3], dist: f32, origin: [f32; 3]) -> [f32; 3] {
    [(dir[0] as f64 * dist as f64 + origin[0] as f64) as f32, dir[1] * dist + origin[1], dir[2] * dist + origin[2]]
}

/// 2.1's `GetSteerPoint` forward probe end: z at 53-bit precision, rounded once.
pub fn steer_probe_end(f: [f32; 3], origin: [f32; 3]) -> [f32; 3] {
    [f[0] * 24.0 + origin[0], f[1] * 24.0 + origin[1], (f[2] as f64 * 24.0 + origin[2] as f64) as f32]
}

impl NativeBotRules for Bots {
    fn random_model_in_teamplay(&self) -> bool {
        true
    }

    /// `BotCreate` (0x1005dc30) has no `botlearn 1`.
    fn learn_on_empty_graph(&self) -> bool {
        false
    }

    /// `BotSetup` (0x1005d8c0): `this[0x97f] = skill`, no floor.
    fn stored_skill(&self, skill: f32) -> f32 {
        skill
    }

    /// `BotSetup` sets `model` and `cl_lw` only (0x1005d98c, 0x1005d9b8).
    fn team_key(&self) -> bool {
        false
    }

    fn texts(&self) -> Option<BotTexts> {
        Some(TEXTS)
    }

    /// `CBot::Spawn` (0x1005def0) is Reset + `CBasePlayer::Spawn`: no laser colour.
    fn spawn_post(&self, _p: &mut Player, _cx: &mut Cx) {}

    /// `CBot` keeps `CBasePlayer::TraceAttack` (vtable 0x10089220 slot 13).
    fn trace_attack(&self, _p: &mut Player, _cx: &mut Cx, _dmg: &DamageInfo) {}

    fn delay(&self, now: f32, k: f64, pev: &entvars_t, skill: f32, max: f64) -> f32 {
        timer(now, k, pev, skill, max)
    }

    /// `fuser1 == 0.0f || !(fuser1 < 0.1f)` (0x100614fb..0x1006151f; a NaN runs).
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    fn runs_ai(&self, pev: &entvars_t) -> bool {
        pev.fuser1 == 0.0 || !(pev.fuser1 < 0.1)
    }

    /// `turnSpeed` (brain+0x1b14) = 30 at the start of every `Think`.
    fn turn_rate(&self, _skill: f32) -> f32 {
        30.0
    }

    fn update_view(&self, b: &mut Brain, pev: &mut entvars_t) {
        change_yaw(pev, b.turn_rate, engine().globals().frametime);
    }

    fn look_at(&self, _b: &mut Brain, pev: &mut entvars_t, target: [f32; 3]) {
        look_at(pev, target);
    }

    fn press(&self, b: &mut Brain, mask: u32, duration: f32, now: f32) {
        buttons(b).press(mask, duration, now);
    }

    fn release(&self, b: &mut Brain, mask: u32, _now: f32) {
        buttons(b).release(mask);
    }

    fn update_buttons(&self, b: &mut Brain, now: f32) {
        b.buttons = buttons(b).update(now);
    }

    /// `Navigate`'s ladder block (0x100621b9): `PressButtons(dir, 1)`, `PressButtons(IN_DUCK, 1)`,
    /// `ReleaseButton(IN_JUMP)`, `ReleaseButton(other)`, one mask per slot. 3.0's combined
    /// masks would sit in slots of their own and leave the probe's jump and the old direction
    /// held (issue #156).
    fn ladder_buttons(&self, b: &mut Brain, down: bool, now: f32) {
        let (hold, other) = if down { (btn::BACK, btn::FORWARD) } else { (btn::FORWARD, btn::BACK) };
        let t = buttons(b);
        t.press(hold, 1.0, now);
        t.press(btn::DUCK, 1.0, now);
        t.release(btn::JUMP);
        t.release(other);
    }

    /// `RandomWanderPoint` (0x10061942..0x10061969): `dir.x * dist` stays on the x87 stack for
    /// `+ origin.x`; the y and z products are stored first.
    fn wander_end(&self, dir: [f32; 3], dist: f32, origin: [f32; 3]) -> [f32; 3] {
        wander_end(dir, dist, origin)
    }

    /// `GetSteerPoint` (0x10062418..0x10062421): `v_forward.z * 24` stays on the x87 stack for
    /// `+ origin.z`; the x and y products are stored first.
    fn steer_probe_end(&self, forward: [f32; 3], origin: [f32; 3]) -> [f32; 3] {
        steer_probe_end(forward, origin)
    }

    /// Bit 0x100 in `Think` (0x1006155c): `PressButtons(IN_ATTACK, 0.1)`, `SetMove(0, 0)`. The
    /// player's `PreThink` respawns an observer that holds fire (issue #113).
    fn respawn(&self, b: &mut Brain, _p: &mut Player, cx: &mut Cx) {
        b.press(btn::ATTACK, 0.1, cx.time());
        b.set_move(0.0, 0.0);
    }

    fn state_think(&self, b: &mut Brain, p: &mut Player, cx: &mut Cx) {
        state_think(b, p, cx);
    }

    fn vision(&self, b: &mut Brain, p: &mut Player, cx: &mut Cx) {
        vision(b, p, cx);
    }

    fn attack(&self, b: &mut Brain, p: &mut Player, cx: &mut Cx) {
        attack::think(b, p, cx);
    }

    fn buy(&self, b: &mut Brain, p: &mut Player, cx: &mut Cx) {
        buy(b, p, cx);
    }

    fn look_around(&self, b: &mut Brain, p: &mut Player, cx: &mut Cx) {
        look_around(b, p, cx);
    }

    /// `Stunt` (0x1005fc40): `RANDOM_LONG(1, 2)`, a dive or a flip.
    fn stunt_mode(&self) -> u8 {
        engine().random_long(1, 2) as u8
    }
}

// ---- states (DM 0x100605f0, TeamRounds 0x10060950, Objective 0x10061020) -------------------------

/// The current state's think. As 3.0's, except: the DM state's roam goal is always an
/// `info_player_deathmatch`, and it leaves `keepGoal` alone while it has a goal (3.0 sets 1);
/// TeamRounds picks a spawn class by the bot's team name and the side swap.
pub fn state_think(b: &mut Brain, p: &mut Player, cx: &mut Cx) {
    let i = b.state as usize;
    let now = cx.time();
    if now < b.states[i].next_think {
        return;
    }
    b.states[i].next_think = timer(now, 0.1, cx.pev(), 1.0, 1.0);
    let v = cx.pev();
    if !alive(v) || v.iuser1 != 0 {
        b.set_modules(if armament_empty(p) { bit::BUY } else { bit::RESPAWN });
        b.enable(bit::CHAT);
        return;
    }
    b.enable(bit::VISION);
    b.enable(bit::CHAT);
    if b.num_enemies() != 0 {
        b.enable(bit::ATTACK);
        b.disable(bit::LOOK_AROUND);
        return;
    }
    if !is_zero(b.last_seen) {
        b.goal = b.last_seen;
        b.last_seen = [0.0; 3];
        b.enable(bit::NAVIGATE);
        if b.state == StateKind::Dm {
            b.keep_goal = 1;
        }
        b.enable(bit::LOOK_AROUND);
        return;
    }
    if !is_zero(b.goal) || now <= b.states[i].next_pick {
        if b.state != StateKind::Dm {
            b.keep_goal = 1;
        }
        b.enable(bit::LOOK_AROUND);
        return;
    }
    match b.state {
        StateKind::Dm => pick_dm(b, cx),
        StateKind::TeamRounds => pick_team_rounds(b, cx),
        StateKind::Objective => {}
    }
    b.states[i].next_pick = (now as f64 + 3.0) as f32;
    b.keep_goal = 0;
    b.enable(bit::LOOK_AROUND);
}

fn set_goal(b: &mut Brain, goal: [f32; 3]) {
    b.goal = goal;
    b.enable(bit::NAVIGATE);
}

/// The DM pick (0x100607a3..): a `ts_dmhill` (1 in 5; not within 64 units), a `ts_powerup`
/// (1 in 5) or an `info_player_deathmatch`, also in teamplay.
fn pick_dm(b: &mut Brain, cx: &mut Cx) {
    let r = engine().random_long(0, 4);
    if r == 0 {
        let e = random_entity("ts_dmhill");
        if !e.is_null() {
            let c = center(e);
            if len_x87(diff_x87(cx.pev().origin, c), order::ZXY) > 64.0 {
                set_goal(b, c);
            }
            return;
        }
    } else if r == 2 {
        let e = random_entity("ts_powerup");
        if !e.is_null() {
            set_goal(b, e.v().origin);
            return;
        }
    }
    let e = random_entity("info_player_deathmatch");
    if !e.is_null() {
        set_goal(b, e.v().origin);
    }
}

/// The class TeamRounds roams to: the other side's spawns. `info_player_deathmatch` for a
/// "Specialists" bot (`_stricmp` with `TeamID()`), else `info_player_start`; the other way round
/// once the sides are swapped (`0x100acb90`, the rules object's `+0x2d0`).
pub fn team_rounds_class(team: &str, swapped: bool) -> &'static str {
    if team.eq_ignore_ascii_case(crate::rules::SPECIALISTS) != swapped { "info_player_deathmatch" } else { "info_player_start" }
}

/// The TeamRounds pick (0x10060a6b..): up to 32 entities of [`team_rounds_class`], one at random.
fn pick_team_rounds(b: &mut Brain, cx: &mut Cx) {
    let team = ts_server::rules::teams::team_name_of(cx.edict);
    let class = team_rounds_class(&team, crate::rules::with(|r| r.swapped));
    let mut list = Vec::new();
    let mut e = std::ptr::null_mut();
    loop {
        e = find_by_class(e, class);
        if e.is_null() || list.len() > 31 {
            break;
        }
        list.push(e);
    }
    if !list.is_empty() {
        let n = list.len() as i32;
        let e = list[engine().random_long(0, n - 1).clamp(0, n - 1) as usize];
        set_goal(b, e.v().origin);
    }
}

// ---- vision (0x1005f890) -------------------------------------------------------------------------

/// `Vision`: every `min(slowInv * 0.1 / skill, 0.7)` s, the players in the view cone whose eyes
/// the bot sees. 2.1 clears only the enemy list, skips players already in either list and
/// observers, and fills **every** empty slot of a list with the player (there's no `break`), so
/// one scan lists one enemy (the first by index) and sets the count to 1. The teammate list is
/// never cleared.
pub fn vision(b: &mut Brain, _p: &mut Player, cx: &mut Cx) {
    let now = cx.time();
    if now < b.vision_next {
        return;
    }
    b.vision_next = timer(now, 0.1, cx.pev(), b.skill, 0.7);
    b.visible_enemies = [std::ptr::null_mut(); 16];
    b.num_visible_enemies = 0;
    let me = cx.edict;
    let eng = engine();
    for i in 1..=eng.max_clients() {
        let o = eng.player(i);
        if !ts_server::rules::has_player(o) {
            continue;
        }
        if b.visible_enemies.contains(&o) || b.visible_mates.contains(&o) || o == me {
            continue;
        }
        let ov = o.v();
        if !alive(ov) || ov.iuser1 != 0 || (ov.effects as u8 as i8) < 0 {
            continue;
        }
        let eye = [ov.origin[0] + ov.view_ofs[0], ov.origin[1] + ov.view_ofs[1], ov.origin[2] + ov.view_ofs[2]];
        if !in_view_cone(cx.pev(), eye) || !visible(cx.pev(), me, eye) {
            continue;
        }
        for j in 0..16 {
            if ts_server::rules::teams::same_team(me, o) {
                if b.visible_mates[j].is_null() {
                    b.visible_mates[j] = o;
                }
            } else if b.visible_enemies[j].is_null() {
                b.visible_enemies[j] = o;
                b.num_visible_enemies = 1;
            }
        }
    }
}

// ---- look around (0x1005fa90) --------------------------------------------------------------------

/// `LookAround`: every `min(2 * slowInv / skill, 5)` s, without traces: look behind (±179° from
/// the view yaw) while `keepGoal` is 0 or there's no goal, else toward the goal ±60°; pitch 0.
/// The view snaps ([`set_ideal_angles`]) and the yaw turns at `RF(2, 4) * 30` this frame.
pub fn look_around(b: &mut Brain, _p: &mut Player, cx: &mut Cx) {
    let now = cx.time();
    if now < b.look_next {
        return;
    }
    b.look_next = timer(now, 2.0, cx.pev(), b.skill, 5.0);
    let eng = engine();
    let v = cx.pev();
    let angles = if b.keep_goal == 0 || is_zero(b.goal) {
        let r = eng.random_long(0, 1);
        Some([0.0, (if r != 0 { 179 } else { -179 }) as f32 + v.v_angle[1], v.v_angle[2]])
    } else if b.keep_goal == 1 {
        let a = eng.vec_to_angles(sub(b.goal, v.origin));
        let r = eng.random_long(-60, 60);
        Some([0.0, r as f32 + a[1], a[2]])
    } else {
        // 2.1 passes uninitialised angles here; `keepGoal` is only ever 0 or 1.
        None
    };
    if let Some(a) = angles {
        set_ideal_angles(cx.pev(), a);
    }
    b.turn_rate = (eng.random_float(2.0, 4.0) as f64 * 30.0) as f32;
}

// ---- buy (0x1005f320) ----------------------------------------------------------------------------

/// `Buy`: one roll `f` for the tier, no money and no kevlar. Knives always (a seal knife ×1/×10,
/// a katana, or combat knives ×1/×5), nine more combat knives instead of a pistol at `f <= 0.1`,
/// an SMG in (0.4, 0.7), a rifle from 0.7. Ids are 2.1's (`weapons21.toml`); the compares are
/// against `double`s.
pub fn buy(b: &mut Brain, p: &mut Player, _cx: &mut Cx) {
    let eng = engine();
    let f = eng.random_float(0.0, 1.0);
    let f64_ = f as f64;
    ts_server::bots::modules::reset_armament(p);
    match eng.random_long(0, 5) {
        2 => (0..if f64_ <= 0.75 { 1 } else { 10 }).for_each(|_| add_item(p, id::SEAL_KNIFE as i32, 0, false)),
        3 => add_item(p, id::KATANA as i32, 0, false),
        _ => (0..if f64_ <= 0.75 { 1 } else { 5 }).for_each(|_| add_item(p, id::KNIFE as i32, 0, false)),
    }
    if f64_ <= 0.1 {
        (0..9).for_each(|_| add_item(p, id::KNIFE as i32, 0, false));
    } else {
        // (id, a second copy half the time, ammo type)
        let (w, akimbo, ammo) = match eng.random_long(1, 9) {
            1 => (1, false, 1),   // Glock-18
            2 => (9, true, 2),    // SOCOM-MK23
            3 => (12, false, 5),  // Desert Eagle
            4 => (14, true, 7),   // Five-seveN
            5 => (8, false, 1),   // Akimbo Berettas
            6 => (21, false, 2),  // Golden Colts
            7 => (22, false, 9),  // Glock-20C
            8 => (28, false, 10), // Ruger
            9 => (31, false, 12), // Raging Bull
            _ => (0, false, 0),
        };
        if w != 0 {
            buy_weapon(p, w, akimbo, ammo);
        }
    }
    if 0.4 < f64_ && f64_ < 0.7 {
        let (w, akimbo, ammo) = match eng.random_long(1, 6) {
            1 => (3, true, 1),   // Mini-Uzi
            2 => (6, false, 1),  // MP5SD
            3 => (7, false, 1),  // MP5K
            4 => (17, false, 1), // STEYR-TMP
            5 => (19, false, 7), // MP7-PDW
            6 => (23, false, 2), // UMP (MAC10's slot)
            _ => (0, false, 0),
        };
        if w != 0 {
            buy_weapon(p, w, akimbo, ammo);
        }
    } else if f64_ >= 0.7 {
        let (w, ammo) = match eng.random_long(1, 11) {
            1 => (4, 3),
            2 => (11, 3),
            3 => (20, 3),
            4 => (13, 6),
            5 => (5, 4),
            6 => (15, 4),
            7 => (26, 3),
            8 => (27, 4),
            9 => (33, 3),
            10 => (18, 8),
            11 => (32, 4),
            _ => (0, 0),
        };
        if w != 0 {
            buy_weapon(p, w, false, ammo);
        }
    }
    b.disable(bit::BUY);
}

#[cfg(test)]
mod tests;
