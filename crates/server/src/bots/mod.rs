//! Server-side bots (fake clients) used to exercise dedicated servers without a human client.
//! Owner: rules work package.
//!
//! `ts_bot_quota N` keeps N fake clients on the server once a map is active. Each one goes
//! through ClientConnect and ClientPutInServer like a real client and is driven every frame
//! with `pfnRunPlayerMove`: wander, turn away from walls, walk to items, aim and fire at visible
//! enemies, and respawn when dead. Robust beats smart.

use crate::commands::Args;
use crate::player::{Player, with_player};
use crate::rules::{self, cvars, lifecycle};
use crate::world::{self, Cx};
use goldsrc::Global;
use goldsrc::server::{IGNORE_MONSTERS, engine};
use goldsrc::sys::*;
use goldsrc::util::to_cstring;
use std::cell::RefCell;

const NAMES: &[&str] = &[
    "Bot Neo", "Bot Trinity", "Bot Morpheus", "Bot Niobe", "Bot Ghost", "Bot Switch", "Bot Apoc", "Bot Tank", "Bot Dozer",
    "Bot Mouse", "Bot Cypher", "Bot Link", "Bot Seraph", "Bot Sati", "Bot Rama", "Bot Kid",
];

/// Per-bot state (kept on the `Player`).
#[derive(Default)]
pub struct Bot {
    yaw: f32,
    pitch: f32,
    next_turn: f32,
    /// Stuck detection: position a second ago.
    check_at: f32,
    check_origin: [f32; 3],
    strafe: f32,
    next_strafe: f32,
    fire_toggle: bool,
    next_fire_toggle: f32,
    next_use: f32,
    goal: Option<[f32; 3]>,
    next_goal_scan: f32,
    jump: bool,
}

#[derive(Default)]
struct Manager {
    active: bool,
    start_at: f32,
    next_add: f32,
    added: u32,
}

static MGR: Global<RefCell<Manager>> = Global(RefCell::new(Manager { active: false, start_at: 0.0, next_add: 0.0, added: 0 }));

pub fn game_init() {
    // `ts_bot_quota` is registered with the rules cvars.
}

pub fn server_activate() {
    let now = engine().time();
    let mut m = MGR.0.borrow_mut();
    m.active = true;
    m.start_at = now + 1.0;
    m.next_add = now + 1.0;
}

pub fn server_deactivate() {
    MGR.0.borrow_mut().active = false;
}

/// Connected fake clients. A dropped client's edict can keep FL_FAKECLIENT and its private data
/// (HL25 drops every bot when the last human leaves); `pfnRunPlayerMove` on it crashes the engine,
/// so also require a live user id.
fn fake_clients() -> Vec<*mut edict_t> {
    let eng = engine();
    eng.clients().filter(|&e| unsafe { (*e).v.flags } & FL_FAKECLIENT as i32 != 0 && eng.user_id(e) != -1).collect()
}

/// Adds one bot (also the `addbot` server command).
pub fn add_bot() {
    let eng = engine();
    let n = {
        let mut m = MGR.0.borrow_mut();
        m.added += 1;
        m.added
    };
    let base = NAMES[(n as usize - 1) % NAMES.len()];
    let name = if n as usize > NAMES.len() { format!("{base} {}", n) } else { base.to_owned() };
    let cname = to_cstring(&name);
    let e = eng.create_fake_client(&cname);
    if e.is_null() {
        eng.print("[ts] bots: no free client slot\n");
        return;
    }
    // A fake client gets no connect/put-in-server from the engine: do what a real client gets.
    rules::client_connect(e, &cname, c"127.0.0.1");
    unsafe {
        (*e).v.flags |= FL_FAKECLIENT as i32;
        world::create::<Player>(&mut (*e).v);
    }
    world::with::<Player, ()>(e, |p| p.bot = Some(Bot::default()));
    world::dispatch(e, |ent, cx| ent.spawn(cx));
    rules::userinfo_changed(e);
    eng.print(&format!("[ts] bots: added \"{name}\" (#{})\n", eng.index_of(e)));
}

/// Called every server frame: keeps the quota and drives each bot with `pfnRunPlayerMove`.
pub fn start_frame() {
    let eng = engine();
    let now = eng.time();
    let (active, start_at, next_add) = {
        let m = MGR.0.borrow();
        (m.active, m.start_at, m.next_add)
    };
    if !active || now < start_at {
        return;
    }
    let quota = cvars::f(c"ts_bot_quota").max(0.0) as usize;
    let bots = fake_clients();
    if bots.len() < quota && now >= next_add {
        MGR.0.borrow_mut().next_add = now + 0.5;
        add_bot();
    } else if bots.len() > quota && now >= next_add {
        MGR.0.borrow_mut().next_add = now + 0.5;
        if let Some(&e) = bots.last() {
            eng.server_command(&format!("kick #{}\n", eng.user_id(e)));
        }
    }

    let msec = (eng.globals().frametime * 1000.0).round().clamp(1.0, 100.0) as u8;
    for e in fake_clients() {
        // Bots created by another mod or a previous map have no Rust bot: adopt them.
        if world::with::<Player, bool>(e, |p| p.bot.is_some()) == Some(false) {
            world::with::<Player, ()>(e, |p| p.bot = Some(Bot::default()));
        }
        let cmd = with_player(e, |p, cx| think(p, cx)).flatten();
        if let Some(c) = cmd {
            eng.run_player_move(e, c.angles, c.forward, c.side, 0.0, c.buttons, 0, msec);
        }
    }
}

pub fn client_command(_p: &mut Player, _cx: &mut Cx, _cmd: &str, _args: &Args) -> bool {
    false
}

struct Cmd {
    angles: [f32; 3],
    forward: f32,
    side: f32,
    buttons: u16,
}

fn angle_mod(a: f32) -> f32 {
    let mut a = a % 360.0;
    if a < 0.0 {
        a += 360.0;
    }
    a
}

/// Turns `cur` toward `target` by at most `step` degrees.
fn approach_angle(cur: f32, target: f32, step: f32) -> f32 {
    let mut d = angle_mod(target - cur);
    if d > 180.0 {
        d -= 360.0;
    }
    angle_mod(cur + d.clamp(-step, step))
}

/// One bot's decision for this frame. Returns None if it has no bot state.
fn think(p: &mut Player, cx: &mut Cx) -> Option<Cmd> {
    let eng = engine();
    let now = cx.time();
    let me = cx.edict;
    let ft = eng.globals().frametime.max(0.001);
    let mut bot = p.bot.take()?;
    let mut buttons: u16 = 0;
    let (mut forward, mut side) = (0.0, 0.0);

    let (observer, deadflag) = (cx.pev().iuser1 != 0, cx.pev().deadflag);
    if observer {
        if lifecycle::can_respawn(p, cx) && eng.random_long(0, 20) == 0 {
            lifecycle::respawn_command(p, cx);
            bot = Bot { yaw: cx.pev().angles[1], ..Bot::default() };
        }
    } else if deadflag != DEAD_NO as i32 {
        // Any button skips the death cam once the body settles.
        if eng.random_long(0, 10) == 0 {
            buttons |= IN_ATTACK as u16;
        }
    } else {
        let v = cx.pev();
        let eye = rules::eye(v);
        let origin = v.origin;

        // Enemy in sight?
        let mut enemy: Option<(f32, [f32; 3])> = None;
        for o in rules::players() {
            if o == me {
                continue;
            }
            let ov = unsafe { &(*o).v };
            if !rules::alive(ov) || ov.iuser1 != 0 || ov.effects & EF_NODRAW as i32 != 0 || rules::teams::same_team(me, o) {
                continue;
            }
            let target = [ov.origin[0], ov.origin[1], ov.origin[2] + 16.0];
            let d = rules::dist(eye, target);
            if d > 3000.0 || enemy.is_some_and(|(best, _)| d >= best) {
                continue;
            }
            let tr = eng.trace_line(eye, target, IGNORE_MONSTERS, me);
            if tr.flFraction >= 1.0 || tr.pHit == o {
                enemy = Some((d, target));
            }
        }

        if let Some((d, target)) = enemy {
            let ang = eng.vec_to_angles([target[0] - eye[0], target[1] - eye[1], target[2] - eye[2]]);
            bot.yaw = approach_angle(bot.yaw, ang[1], 540.0 * ft);
            bot.pitch = (-ang[0]).clamp(-60.0, 60.0);
            if now >= bot.next_fire_toggle {
                bot.fire_toggle = !bot.fire_toggle;
                bot.next_fire_toggle = now + if bot.fire_toggle { 0.25 } else { 0.1 };
            }
            if bot.fire_toggle {
                buttons |= IN_ATTACK as u16;
            }
            // Fight in slow motion when holding a power-up.
            if p.ts.held != 0 && p.active_powerup == 0 && d < 1500.0 {
                rules::powerup::use_powerup(p, cx);
            }
            forward = if d > 400.0 { 300.0 } else { 0.0 };
            if now >= bot.next_strafe {
                bot.strafe = [-300.0, 0.0, 300.0][eng.random_long(0, 2) as usize];
                bot.next_strafe = now + 0.6;
            }
            side = bot.strafe;
        } else {
            bot.pitch = 0.0;
            // Items worth walking to: ground weapons and power-ups in sight.
            if now >= bot.next_goal_scan {
                bot.next_goal_scan = now + 1.0;
                bot.goal = find_item(me, eye);
            }
            if let Some(g) = bot.goal {
                let dir = [g[0] - origin[0], g[1] - origin[1], 0.0];
                if rules::len2d(dir) < 24.0 {
                    bot.goal = None;
                } else {
                    let ang = eng.vec_to_angles(dir);
                    bot.yaw = approach_angle(bot.yaw, ang[1], 360.0 * ft);
                }
                if rules::len2d(dir) < 64.0 && now >= bot.next_use {
                    buttons |= IN_USE as u16;
                    bot.next_use = now + 0.5;
                }
            }
            forward = 400.0;

            // Turn away from walls.
            let (fwd, _, _) = eng.angle_vectors([0.0, bot.yaw, 0.0]);
            let from = [origin[0], origin[1], origin[2]];
            let ahead = [from[0] + fwd[0] * 72.0, from[1] + fwd[1] * 72.0, from[2]];
            let tr = eng.trace_hull(from, ahead, IGNORE_MONSTERS, goldsrc::server::HEAD_HULL, me);
            if tr.flFraction < 1.0 && now >= bot.next_turn {
                bot.goal = None;
                let turn = eng.random_float(90.0, 180.0) * if eng.random_long(0, 1) == 0 { 1.0 } else { -1.0 };
                bot.yaw = angle_mod(bot.yaw + turn);
                bot.next_turn = now + 0.3;
            } else if now >= bot.next_turn && eng.random_long(0, 200) == 0 {
                bot.yaw = angle_mod(bot.yaw + eng.random_float(-60.0, 60.0));
                bot.next_turn = now + 1.0;
            }
        }

        // Stuck: barely moved in a second → turn around and jump.
        if now >= bot.check_at {
            if rules::dist(origin, bot.check_origin) < 16.0 && forward != 0.0 {
                bot.yaw = angle_mod(bot.yaw + 180.0 + eng.random_float(-45.0, 45.0));
                bot.jump = true;
                bot.goal = None;
            }
            bot.check_origin = origin;
            bot.check_at = now + 1.0;
        }
        if bot.jump || eng.random_long(0, 400) == 0 {
            buttons |= IN_JUMP as u16;
            bot.jump = false;
        }
    }

    let angles = [bot.pitch, bot.yaw, 0.0];
    p.bot = Some(bot);
    Some(Cmd { angles, forward, side, buttons })
}

/// The closest visible ground weapon or power-up within 600 units.
fn find_item(me: *mut edict_t, eye: [f32; 3]) -> Option<[f32; 3]> {
    let eng = engine();
    let mut best: Option<(f32, [f32; 3])> = None;
    let mut it = std::ptr::null_mut();
    loop {
        it = eng.find_in_sphere(it, eye, 600.0);
        if it.is_null() {
            break;
        }
        let v = unsafe { &(*it).v };
        if v.effects & EF_NODRAW as i32 != 0 || v.solid != SOLID_TRIGGER as i32 {
            continue;
        }
        let cls = eng.string(v.classname);
        if cls != c"ts_powerup" && !crate::entities::ts::is_usable(cls) && cls != c"ts_groundammo" {
            continue;
        }
        let c = [(v.absmin[0] + v.absmax[0]) * 0.5, (v.absmin[1] + v.absmax[1]) * 0.5, (v.absmin[2] + v.absmax[2]) * 0.5];
        let d = rules::dist(eye, c);
        if best.is_some_and(|(b, _)| d >= b) {
            continue;
        }
        let tr = eng.trace_line(eye, c, IGNORE_MONSTERS, me);
        if tr.flFraction >= 1.0 || tr.pHit == it {
            best = Some((d, c));
        }
    }
    best.map(|(_, c)| c)
}

#[cfg(test)]
mod tests {
    use super::approach_angle;

    #[test]
    fn turning() {
        assert_eq!(approach_angle(10.0, 20.0, 5.0), 15.0);
        assert_eq!(approach_angle(350.0, 10.0, 30.0), 10.0, "wraps the short way");
        assert_eq!(approach_angle(10.0, 350.0, 5.0), 5.0);
    }
}
