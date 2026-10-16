use super::*;
use shared::pm::Arith;
use shared::weapons::gun::{MeleeHit, MeleeResult, Shot, Sound, Virtual};
use shared::weapons::spread::Vec3;
use shared::weapons::{EventArgs, att, id, info};

const IN_DUCK: u32 = 4;
const IN_BACK: u32 = 16;
const IN_RELOAD: u32 = 0x2000;
const STUNTING_KUNGFU: i32 = 0x50c30;

/// Records everything the gun asks of its host.
#[derive(Default)]
struct Mock {
    time: f32,
    slow: f32,
    powerups: u16,
    buttons: u32,
    pressed: u32,
    velocity: Vec3,
    flags: i32,
    iuser4: i32,
    seed: u32,
    fov: f32,
    view_model: Option<u8>,
    events: Vec<(Event, EventArgs)>,
    shots: Vec<Shot>,
    kicks: Vec<(f32, f32)>,
    shakes: Vec<f32>,
    melee: Vec<MeleeHit>,
    melee_result: Option<MeleeResult>,
    grenades: Vec<(Vec3, Vec3, bool)>,
    blades: Vec<i32>,
    slots: i32,
    anims: Vec<i32>,
    blocks: Vec<f32>,
    crouches: Vec<i32>,
    cooldowns: Vec<f32>,
    /// `IsAlive()` false (`HolsterWeapon` stops before its animation and event).
    dead: bool,
    /// `deploy_update_now` calls (`CTSGun::Deploy` zeroing the `UpdateClientData` timer).
    update_now: u32,
    /// The build reproduced (`Arith::CLIENT` unless set).
    arith: Option<Arith>,
    /// A client re-prediction (`g_runfuncs` 0).
    repredict: bool,
    /// The client's HUD kung-fu power (`0x1018a718`); `None` = the server.
    kfu_power: Option<f32>,
    /// The prediction's records, which never carry the owned flag (`tracks_owned` false).
    client_records: bool,
    /// `default_deploy`'s answer (`None`: client.dll's 0.5 / 3.0) and the ids it was asked for.
    deploy_timers: Option<(f32, f32)>,
    deployed: Vec<u8>,
    /// The body passed with each `weapon_anim`.
    anim_bodies: Vec<i32>,
    /// The hookable virtual calls, as `pre <v>` / `post <v> <ran>` (`virtual_pre` / `virtual_post`).
    virtuals: Vec<String>,
    /// `virtual_pre` skips this virtual (a plugin's SUPERCEDE).
    skip: Option<Virtual>,
    /// `melee_attack_pre`: the (time, damage) it was asked with, and its answer: `None` lets the
    /// move through unchanged, `Some((time, damage, go))` changes it.
    melee_pre: Vec<(f32, f32)>,
    melee_answer: Option<(f32, f32, bool)>,
}

impl GunHost for Mock {
    fn melee_attack_pre(&mut self, _gun: &mut Gun, time: &mut f32, damage: &mut f32) -> bool {
        self.melee_pre.push((*time, *damage));
        let Some((t, d, go)) = self.melee_answer else { return true };
        (*time, *damage) = (t, d);
        go
    }
    fn virtual_pre(&mut self, _gun: &mut Gun, v: Virtual, _arg: &mut i32) -> bool {
        self.virtuals.push(format!("pre {v:?}"));
        self.skip != Some(v)
    }
    fn virtual_post(&mut self, _gun: &mut Gun, v: Virtual, _arg: i32, ran: Option<i32>) -> i32 {
        self.virtuals.push(format!("post {v:?} {ran:?}"));
        ran.unwrap_or(0)
    }
    fn time(&self) -> f32 {
        self.time
    }
    fn slow_mul(&self) -> f32 {
        if self.slow == 0.0 { 1.0 } else { self.slow }
    }
    fn arith(&self) -> Arith {
        self.arith.unwrap_or(Arith::CLIENT)
    }
    fn rules(&self) -> &'static dyn shared::gameplay::GunRules {
        &crate::TS30
    }
    fn first_prediction(&self) -> bool {
        !self.repredict
    }
    fn kfu_power(&self) -> Option<f32> {
        self.kfu_power
    }
    fn prediction(&self) -> bool {
        self.kfu_power.is_some()
    }
    fn tracks_owned(&self) -> bool {
        !self.client_records
    }
    fn powerups(&self) -> u16 {
        self.powerups
    }
    fn kungfu(&self) -> bool {
        false
    }
    fn alive(&self) -> bool {
        !self.dead
    }
    fn buttons(&self) -> u32 {
        self.buttons
    }
    fn pressed(&self) -> u32 {
        self.pressed
    }
    fn origin(&self) -> Vec3 {
        [0.0; 3]
    }
    fn view_ofs(&self) -> Vec3 {
        [0.0, 0.0, 28.0]
    }
    fn velocity(&self) -> Vec3 {
        self.velocity
    }
    fn v_angle(&self) -> Vec3 {
        [0.0; 3]
    }
    fn flags(&self) -> i32 {
        self.flags
    }
    fn waterlevel(&self) -> i32 {
        0
    }
    fn iuser4(&self) -> i32 {
        self.iuser4
    }
    fn set_iuser4(&mut self, v: i32) {
        self.iuser4 = v;
    }
    fn seed(&self) -> u32 {
        self.seed
    }
    fn random_long(&mut self, lo: i32, _hi: i32) -> i32 {
        lo
    }
    fn fov(&self) -> f32 {
        self.fov
    }
    fn set_fov(&mut self, fov: f32) {
        self.fov = fov;
    }
    fn set_view_model(&mut self, id: Option<u8>) {
        self.view_model = id;
    }
    fn view_model_hidden(&self) -> bool {
        self.view_model.is_none()
    }
    fn set_weapon_model(&mut self, _model: Option<(u8, bool)>) {}
    fn event(&mut self, ev: Event, args: EventArgs) {
        self.events.push((ev, args));
    }
    fn deploy_update_now(&mut self) {
        self.update_now += 1;
    }
    fn default_deploy(&mut self, id: u8) -> (f32, f32) {
        self.deployed.push(id);
        self.deploy_timers.unwrap_or((0.5, 3.0))
    }
    fn weapon_anim(&mut self, anim: i32, _skip_local: bool, body: i32) {
        self.anims.push(anim);
        self.anim_bodies.push(body);
    }
    fn player_anim(&mut self, _anim: i32) {}
    fn kick(&mut self, pitch: f32, yaw: f64, shake: bool) {
        assert!(shake, "TS 3.0's VKickback shakes the view");
        self.kicks.push((pitch, yaw as f32));
    }
    fn view_shake(&mut self, amp: f32) {
        self.shakes.push(amp);
    }
    fn sound(&mut self, _s: Sound) {}
    fn shoot(&mut self, shot: &Shot, _gun: &mut Gun) {
        self.shots.push(*shot);
    }
    fn melee(&mut self, hit: &MeleeHit, _gun: &mut Gun) -> MeleeResult {
        self.melee.push(*hit);
        self.melee_result.unwrap_or(MeleeResult::Miss)
    }
    fn throw_grenade(&mut self, origin: Vec3, velocity: Vec3, roll: bool) {
        self.grenades.push((origin, velocity, roll));
    }
    fn throw_blade(&mut self, btype: i32) {
        self.blades.push(btype);
    }
    fn slots_changed(&mut self, delta_free: i32) {
        self.slots += delta_free;
    }
    fn melee_bar_changed(&mut self, _bar: f32) {}
    fn block_player(&mut self, t: f32) {
        self.blocks.push(t);
        self.iuser4 |= 1;
    }
    fn set_close_combat_cooldown(&mut self, t: f32) {
        self.cooldowns.push(t);
    }
    fn melee_crouch(&mut self, bits: i32) {
        self.crouches.push(bits);
        self.iuser4 |= bits;
    }
}

fn armed(id: u8) -> (Gun, Mock) {
    let mut g = Gun::default();
    let mut h = Mock { flags: 0x200, time: 10.0, ..Default::default() };
    assert_eq!(g.take_weapon(id, None, 0), Some(id));
    let ammo = info(id).unwrap().ammo as usize;
    g.reserve[ammo] = 100;
    assert!(g.switch_to(id, &mut h));
    g.next_attack = 0.0;
    h.events.clear();
    (g, h)
}

/// Runs one usercmd: post frame with `buttons`, then timers by `dt`.
fn frame(g: &mut Gun, h: &mut Mock, buttons: u32, last: &mut u32, dt: f32) {
    h.buttons = buttons;
    h.pressed = buttons & !*last;
    *last = buttons;
    g.item_post_frame(h);
    g.decrement_timers(h.rules().timer_step(h.arith(), dt as f64, h.slow_mul(), h.powerups));
    h.time += dt;
}

#[test]
fn auto_fire_rate_and_clip() {
    crate::tables::make_default();
    let (mut g, mut h) = armed(id::AK47);
    let mut last = 0;
    for _ in 0..100 {
        frame(&mut g, &mut h, 1, &mut last, 0.01);
    }
    // 0.08 s per shot over 1 s -> 13 shots (first at t=0)
    let fired = h.events.iter().filter(|e| e.0 == Event::Fire).count();
    assert!((12..=14).contains(&fired), "{fired}");
    assert_eq!(g.status[id::AK47 as usize].clip, 30 - fired as i32);
    assert_eq!(h.shots[0].pellets, 1);
    assert_eq!(h.shots[0].damage, 39.0);
}

#[test]
fn slow_motion_scales_timers() {
    crate::tables::make_default();
    let (mut g, mut h) = armed(id::AK47);
    h.slow = 0.5;
    let mut last = 0;
    for _ in 0..100 {
        frame(&mut g, &mut h, 1, &mut last, 0.01);
    }
    let fired = h.events.iter().filter(|e| e.0 == Event::Fire).count();
    assert!((6..=8).contains(&fired), "{fired}");
}

#[test]
fn double_fire_rate_is_1_5x() {
    crate::tables::make_default();
    let (mut g, mut h) = armed(id::AK47);
    h.powerups = shared::ts::powerup::DOUBLE_FIRERATE;
    let mut last = 0;
    for _ in 0..100 {
        frame(&mut g, &mut h, 1, &mut last, 0.01);
    }
    let fired = h.events.iter().filter(|e| e.0 == Event::Fire).count();
    assert!((16..=18).contains(&fired), "{fired}"); // 0.08 s at 0.015 s steps = 6 frames
}

#[test]
fn semi_needs_a_press_and_uses_0_1s_on_auto_capable_weapons() {
    crate::tables::make_default();
    let (mut g, mut h) = armed(id::GLOCK18);
    g.switch_fire_mode();
    assert_eq!(g.status[id::GLOCK18 as usize].mode, mode::SEMI);
    let mut last = 0;
    for _ in 0..50 {
        frame(&mut g, &mut h, 1, &mut last, 0.01);
    }
    assert_eq!(h.events.iter().filter(|e| e.0 == Event::Fire).count(), 1);
    assert!((g.next_primary + 0.001).abs() < 0.0011 || g.next_primary <= 0.0);
    // tapping fires every 0.1 s at most
    h.events.clear();
    for i in 0..100 {
        let b = if i % 2 == 0 { 1 } else { 0 };
        frame(&mut g, &mut h, b, &mut last, 0.01);
    }
    let n = h.events.iter().filter(|e| e.0 == Event::Fire).count();
    assert!((9..=11).contains(&n), "{n}");
}

#[test]
fn burst_fires_three_then_waits() {
    crate::tables::make_default();
    let (mut g, mut h) = armed(id::M16);
    assert_eq!(g.status[id::M16 as usize].mode, mode::BURST);
    let mut last = 0;
    frame(&mut g, &mut h, 1, &mut last, 0.01);
    for _ in 0..30 {
        frame(&mut g, &mut h, 0, &mut last, 0.01);
    }
    assert_eq!(h.events.iter().filter(|e| e.0 == Event::Fire).count(), 3);
    // UMP bursts are two rounds
    let (mut g, mut h) = armed(id::UMP);
    g.switch_fire_mode();
    g.switch_fire_mode();
    assert_eq!(g.status[id::UMP as usize].mode, mode::BURST);
    let mut last = 0;
    frame(&mut g, &mut h, 1, &mut last, 0.01);
    for _ in 0..30 {
        frame(&mut g, &mut h, 0, &mut last, 0.01);
    }
    assert_eq!(h.events.iter().filter(|e| e.0 == Event::Fire).count(), 2);
}

#[test]
fn pump_alternates_fire_and_pump() {
    crate::tables::make_default();
    let (mut g, mut h) = armed(id::MOSSBERG);
    assert_eq!(g.status[id::MOSSBERG as usize].mode, mode::PUMP);
    let mut last = 0;
    for _ in 0..60 {
        frame(&mut g, &mut h, 1, &mut last, 0.01);
    }
    let kinds: Vec<Event> = h.events.iter().map(|e| e.0).filter(|e| *e == Event::Fire || *e == Event::Pump).collect();
    assert!(kinds.len() >= 3);
    assert_eq!(kinds[0], Event::Fire);
    assert_eq!(kinds[1], Event::Pump);
    assert_eq!(kinds[2], Event::Fire);
    assert_eq!(h.shots[0].pellets, 8);
    assert_eq!(h.shots[0].btype, 1);
}

#[test]
fn magazine_reload_moves_rounds_after_reload_time() {
    crate::tables::make_default();
    let (mut g, mut h) = armed(id::AK47);
    g.status[id::AK47 as usize].clip = 5;
    let mut last = 0;
    frame(&mut g, &mut h, IN_RELOAD, &mut last, 0.01);
    assert_eq!(g.status[id::AK47 as usize].in_reload, 1);
    assert!(h.events.iter().any(|e| e.0 == Event::Reload && e.1.f1 == 0.0));
    for _ in 0..240 {
        frame(&mut g, &mut h, 0, &mut last, 0.01);
    }
    assert_eq!(g.status[id::AK47 as usize].clip, 5);
    for _ in 0..20 {
        frame(&mut g, &mut h, 0, &mut last, 0.01);
    }
    assert_eq!(g.status[id::AK47 as usize].clip, 30);
    assert_eq!(g.reserve[6], 75);
    assert_eq!(g.status[id::AK47 as usize].in_reload, 0);
}

/// Prediction runs on networked clip/reserve values: out-of-range ones wrap like TS's C ints
/// instead of overflowing (an abort under `panic = "abort"`).
#[test]
fn extreme_networked_values_wrap() {
    crate::tables::make_default();
    let (mut g, mut h) = armed(id::AK47);
    let ammo = info(id::AK47).unwrap().ammo as usize;
    g.status[id::AK47 as usize].clip = i32::MIN;
    g.status[id::AK47 as usize].in_reload = 1;
    g.reserve[ammo] = i32::MAX;
    let mut last = 0;
    frame(&mut g, &mut h, 0, &mut last, 0.01);
    assert_eq!(g.status[id::AK47 as usize].in_reload, 0);
    // Firing with a clip of i32::MIN.
    let (mut g, mut h) = armed(id::AK47);
    g.status[id::AK47 as usize].clip = i32::MIN;
    frame(&mut g, &mut h, IN_RELOAD, &mut last, 0.01);
    g.status[id::AK47 as usize].mode = u8::MAX;
    g.switch_fire_mode();
    g.reserve[ammo] = 1;
    assert!(g.add_ammo(ammo as u8, i32::MAX, i32::MAX));
    assert_eq!(g.reserve[ammo], i32::MAX);
}

#[test]
fn shotgun_reload_three_phases() {
    crate::tables::make_default();
    let (mut g, mut h) = armed(id::BENELLI);
    g.status[id::BENELLI as usize].clip = 5;
    let mut last = 0;
    frame(&mut g, &mut h, IN_RELOAD, &mut last, 0.01);
    assert_eq!(g.status[id::BENELLI as usize].state, 1);
    for _ in 0..200 {
        frame(&mut g, &mut h, 0, &mut last, 0.01);
    }
    let phases: Vec<f32> = h.events.iter().filter(|e| e.0 == Event::Reload).map(|e| e.1.f1).collect();
    assert_eq!(phases, vec![8.0, 9.0, 9.0, 10.0]);
    assert_eq!(g.status[id::BENELLI as usize].clip, 7);
    assert_eq!(g.status[id::BENELLI as usize].state, 0);
}

#[test]
fn spread_penalty_grows_caps_and_decays() {
    crate::tables::make_default();
    let (mut g, mut h) = armed(id::GLOCK18);
    let mut last = 0;
    for _ in 0..100 {
        frame(&mut g, &mut h, 1, &mut last, 0.01);
    }
    assert!(g.spread_penalty > 0.0 && g.spread_penalty <= 0.1);
    let last_spread = h.events.iter().rev().find(|e| e.0 == Event::Fire).unwrap().1.f1;
    assert!(last_spread > 0.01);
    let p = g.spread_penalty;
    decay_spread(&mut g, Arith::CLIENT, 0.5);
    assert!((g.spread_penalty - (p - 0.05).max(0.0)).abs() < 1e-6);
}

/// `client.dll`'s `ScaleSpread` (`0x10008100`) neither adds nor grows the penalty, nor moves the
/// last-shot time, when re-predicting; the silencer's 0.875 still applies.
#[test]
fn spread_penalty_only_on_first_prediction() {
    let mut g = Gun { spread_penalty: 0.05, last_shot: 1.0, ..Default::default() };
    let mut h = Mock { time: 2.0, repredict: true, ..Default::default() };
    let mut spread = 0.04;
    scale_spread(&mut g, &mut spread, 0.01, true, &h);
    assert_eq!((spread, g.spread_penalty, g.last_shot), (0.04 * 0.875, 0.05, 1.0));
    h.repredict = false;
    let mut spread = 0.04;
    scale_spread(&mut g, &mut spread, 0.01, false, &h);
    assert_eq!(spread, 0.04 + 0.05);
    assert!(g.spread_penalty > 0.05);
    assert_eq!(g.last_shot, 2.0);
}

#[test]
fn silencer_multipliers() {
    crate::tables::make_default();
    let (mut g, mut h) = armed(id::M4);
    g.status[id::M4 as usize].items |= att::SILENCER;
    let mut last = 0;
    frame(&mut g, &mut h, 1, &mut last, 0.01);
    assert!((h.shots[0].damage - 33.0 * 0.775).abs() < 1e-4);
    let (p, _) = h.kicks[0];
    assert!((p - 3.75 * 0.85 * 0.775).abs() < 1e-4);
    // spread 0.005 * 0.875 on the first shot (no penalty yet)
    assert!((h.shots[0].spread - 0.005 * 0.875).abs() < 1e-7);
    // the MP5SD's built-in silencer has no penalty
    let (mut g, mut h) = armed(id::MP5SD);
    frame(&mut g, &mut h, 1, &mut 0, 0.01);
    assert_eq!(h.shots[0].damage, 19.0);
}

#[test]
fn akimbo_pickup_merges_and_linked_fire() {
    crate::tables::make_default();
    let mut g = Gun::default();
    let mut h = Mock { flags: 0x200, time: 1.0, ..Default::default() };
    assert_eq!(g.take_weapon(id::BERETTA, None, 0), Some(id::BERETTA));
    g.status[id::BERETTA as usize].clip = 10;
    assert_eq!(g.take_weapon(id::BERETTA, None, 0), Some(id::BERETTA_AKIMBO));
    let a = g.status[id::BERETTA_AKIMBO as usize];
    assert_eq!((a.left, a.right, a.clip), (10, 15, 25));
    g.reserve[1] = 60;
    g.switch_to(id::BERETTA_AKIMBO, &mut h);
    g.next_attack = 0.0;
    // switch to akimbo-semi (mode 4) and press both triggers
    g.switch_fire_mode();
    assert_eq!(g.status[id::BERETTA_AKIMBO as usize].mode, mode::AKIMBO_SEMI);
    h.events.clear();
    let mut last = 0;
    frame(&mut g, &mut h, IN_ATTACK | IN_ATTACK2, &mut last, 0.01);
    assert_eq!(h.events.iter().filter(|e| e.0 == Event::FireLinked).count(), 1);
    assert_eq!(h.shots.len(), 2);
    assert_eq!(h.shots[1].seed, h.shots[0].seed.wrapping_add(0x21));
    assert_eq!(h.shots[0].right_offset, -h.shots[1].right_offset);
    let a = g.status[id::BERETTA_AKIMBO as usize];
    assert_eq!((a.left, a.right, a.clip), (9, 14, 23));
    assert_eq!(g.status[id::BERETTA as usize].clip, 9, "single mirrors the left clip");
    // single trigger alternation in mode 1 (semi)
    let mut g2 = g.clone();
    g2.switch_fire_mode();
    assert_eq!(g2.status[id::BERETTA_AKIMBO as usize].mode, mode::SEMI);
}

/// Free semi as TS has it (`AkimboFire` 0x829c0, `ItemPostFrame` 0xa03d4): `+attack` fires the
/// `b1 = 0` gun and `+attack2` the other, each on its own press; both pressed in one command fire a
/// linked pair. A shot sets both attack timers to 0.1 s, so a second trigger pressed within that
/// time (in a later command) is dropped, as in TS.
#[test]
fn akimbo_free_semi_triggers() {
    crate::tables::make_default();
    let mut g = Gun::default();
    let mut h = Mock { flags: 0x200, time: 1.0, ..Default::default() };
    g.take_weapon(id::BERETTA, None, 0);
    g.take_weapon(id::BERETTA, None, 0);
    g.reserve[1] = 60;
    g.switch_to(id::BERETTA_AKIMBO, &mut h);
    g.next_attack = 0.0;
    g.switch_fire_mode();
    assert_eq!(g.status[id::BERETTA_AKIMBO as usize].mode, mode::AKIMBO_SEMI);
    let fired = |h: &Mock| -> Vec<(Event, i32)> {
        h.events.iter().filter(|e| matches!(e.0, Event::Fire | Event::FireLinked)).map(|e| (e.0, e.1.b1)).collect()
    };
    let mut last = 0;
    // +attack, then +attack2 30 ms later: one shot (the second press falls in the 0.1 s).
    frame(&mut g, &mut h, IN_ATTACK, &mut last, 0.01);
    frame(&mut g, &mut h, IN_ATTACK, &mut last, 0.01);
    frame(&mut g, &mut h, IN_ATTACK, &mut last, 0.01);
    frame(&mut g, &mut h, IN_ATTACK | IN_ATTACK2, &mut last, 0.01);
    for _ in 0..20 {
        frame(&mut g, &mut h, IN_ATTACK | IN_ATTACK2, &mut last, 0.01);
    }
    assert_eq!(fired(&h), [(Event::Fire, 0)]);
    // Released; +attack2 alone: the other gun.
    frame(&mut g, &mut h, 0, &mut last, 0.01);
    frame(&mut g, &mut h, IN_ATTACK2, &mut last, 0.01);
    assert_eq!(fired(&h)[1..], [(Event::Fire, 1)]);
    for _ in 0..20 {
        frame(&mut g, &mut h, 0, &mut last, 0.01);
    }
    // Both in one command: a linked pair.
    frame(&mut g, &mut h, IN_ATTACK | IN_ATTACK2, &mut last, 0.01);
    assert_eq!(fired(&h)[2..].iter().map(|e| e.0).collect::<Vec<_>>(), [Event::FireLinked]);
}

#[test]
fn zoom_steps() {
    crate::tables::make_default();
    let (mut g, mut h) = armed(id::AK47);
    g.status[id::AK47 as usize].items |= att::SCOPE;
    let mut last = 0;
    frame(&mut g, &mut h, IN_ATTACK2, &mut last, 0.01);
    assert_eq!(h.fov, 32.0);
    assert!(h.view_model.is_none(), "fov < 40 hides the view model");
    for _ in 0..40 {
        frame(&mut g, &mut h, 0, &mut last, 0.01);
    }
    frame(&mut g, &mut h, IN_ATTACK2, &mut last, 0.01);
    assert_eq!(h.fov, 13.0);
    for _ in 0..40 {
        frame(&mut g, &mut h, 0, &mut last, 0.01);
    }
    frame(&mut g, &mut h, IN_ATTACK2, &mut last, 0.01);
    assert_eq!(h.fov, 0.0);
    assert_eq!(h.view_model, Some(id::AK47));
}

#[test]
fn grip_toggle_without_scope() {
    crate::tables::make_default();
    let (mut g, mut h) = armed(id::M4);
    let mut last = 0;
    frame(&mut g, &mut h, IN_ATTACK2, &mut last, 0.01);
    assert!(g.status[id::M4 as usize].alt_grip);
    assert_eq!(*h.anims.last().unwrap(), 8, "A[7] of the M4");
}

#[test]
fn grenade_pull_and_throw_on_release() {
    crate::tables::make_default();
    let (mut g, mut h) = armed(id::M61);
    let mut last = 0;
    frame(&mut g, &mut h, 1, &mut last, 0.01);
    assert_eq!(g.status[id::M61 as usize].state, 1);
    for _ in 0..100 {
        frame(&mut g, &mut h, 1, &mut last, 0.01);
    }
    assert!(h.grenades.is_empty(), "held: no throw");
    for _ in 0..200 {
        frame(&mut g, &mut h, 0, &mut last, 0.01);
    }
    assert_eq!(h.grenades.len(), 1);
    assert!(!h.grenades[0].2);
    // the only grenade is gone and kung fu is back
    assert_eq!(g.current, 0);
    assert!(!g.status[id::M61 as usize].owned);
    assert_eq!(h.slots, 7);
    let pins: Vec<i32> = h.events.iter().filter(|e| e.0 == Event::Grenade).map(|e| e.1.i1).collect();
    assert_eq!(pins, vec![0x13, 0x14]);
}

/// `CBasePlayer::StartObserver` (0x9971e..0x9974f) on a living player with a primed grenade:
/// `CTSGun::Reset`, `HolsterWeapon`, `Holster` (`HolsterWeapon` again), then `RemoveAllItems`'
/// `Holster` once `deadflag` is set.
#[test]
fn observer_reset_then_holsters_throws_nothing() {
    crate::tables::make_default();
    let (mut g, mut h) = armed(id::M61);
    let mut last = 0;
    frame(&mut g, &mut h, 1, &mut last, 0.01);
    assert_eq!(g.status[id::M61 as usize].state, 1, "pin pulled");
    h.events.clear();
    h.anims.clear();
    g.reset_inventory();
    g.holster_weapon(&mut h);
    g.holster_weapon(&mut h);
    h.dead = true;
    g.holster_weapon(&mut h);
    assert!(h.grenades.is_empty(), "the reset took the primed grenade: nothing thrown");
    assert!(!h.events.iter().any(|e| e.0 == Event::Grenade));
    // Two kung-fu holster animations and draw events (current 0), none from the dead holster.
    let draws: Vec<i32> = h.events.iter().filter(|e| e.0 == Event::Draw).map(|e| e.1.i1).collect();
    assert_eq!(draws, vec![0, 0]);
    assert_eq!(h.anims.len(), 2);
    assert_eq!(g.current, 0);

    // Without the reset (what a holster alone does) the grenade would go.
    let (mut g, mut h) = armed(id::M61);
    let mut last = 0;
    frame(&mut g, &mut h, 1, &mut last, 0.01);
    g.holster_weapon(&mut h);
    assert_eq!(h.grenades.len(), 1);
}

#[test]
fn grenade_throw_math() {
    let (src, vel) =
        Gun::grenade_throw(&crate::gun::grenade_rounding(Arith::Msvc), [0.0; 3], [0.0, 0.0, 28.0], [0.0; 3], [0.0, 0.0, 0.0], false);
    // pitch 0 -> -10 (up), speed min(750, 100*7)=700
    assert!((spread::length(vel) - 700.0).abs() < 1e-3);
    assert!(vel[2] > 0.0);
    assert!((src[2] - (28.0 + 16.0 * (10.0f32).to_radians().sin())).abs() < 1e-3);
    let (_, vel) = Gun::grenade_throw(&crate::gun::grenade_rounding(Arith::Gcc), [0.0; 3], [0.0; 3], [0.0; 3], [0.0, 0.0, 0.0], true);
    assert!((spread::length(vel) - 700.0 * 0.6666667).abs() < 1e-2);
}

/// The throw's pitch and speed use the `double`s 10/9, 8/9, -10 and 2/3, and the builds sum the
/// velocity differently (issue #129). At a pitch of -88.2 the port's old `float` arithmetic gave
/// the velocity `[0x430aa2b7, 0xc1b6a2a0, 0x443d5a16]`.
#[test]
fn grenade_throw_rounds_as_ts() {
    let p = f32::from_bits(0xc2b0_6666);
    let throw = |arith| {
        Gun::grenade_throw(
            &crate::gun::grenade_rounding(arith),
            [100.3, -20.7, 64.1],
            [0.0, 0.0, 28.0],
            [120.5, -33.3, 7.7],
            [p, 30.0, 0.0],
            false,
        )
    };
    let (src, vel) = throw(Arith::Msvc);
    assert_eq!(src.map(f32::to_bits), [0x42c9_5fb1, 0xc1a3_d022, 0x42d8_3001]);
    assert_eq!(vel.map(f32::to_bits), [0x430a_a2bc, 0xc1b6_a288, 0x443d_5a16]);
    // `ts_i386.so` adds the y product unrounded where mp.dll stores it first.
    assert_eq!(throw(Arith::Gcc).1.map(f32::to_bits), [0x430a_a2bc, 0xc1b6_a289, 0x443d_5a16]);
}

#[test]
fn knife_throw_consumes_stack() {
    crate::tables::make_default();
    let (mut g, mut h) = armed(id::KNIFE);
    g.take_weapon(id::KNIFE, None, 0);
    assert_eq!(g.status[id::KNIFE as usize].clip, 2);
    let mut last = 0;
    frame(&mut g, &mut h, IN_ATTACK2, &mut last, 0.01);
    assert_eq!(h.blades, vec![6]);
    assert_eq!(g.status[id::KNIFE as usize].clip, 1);
    assert_eq!(h.slots, 1);
}

#[test]
fn kungfu_punch_and_bar() {
    let mut g = Gun::default();
    let mut h = Mock { flags: 0x200, ..Default::default() };
    let mut last = 0;
    frame(&mut g, &mut h, IN_ATTACK, &mut last, 0.01);
    assert_eq!(h.melee.len(), 1);
    assert_eq!(h.melee[0].damage, 20.0);
    // Client view shake: the base damage x 0.35 (0x1000b5c7).
    assert_eq!(h.shakes, vec![20.0 * 0.35]);
    let cc: Vec<_> = h.events.iter().filter(|e| e.0 == Event::CloseCombat).collect();
    assert_eq!(cc[0].1.i1, 1);
    // A fresh punch: set A index 4 on the wire (0x7643e), move 23 kept for the combos.
    assert_eq!(cc[0].1.i2, 4);
    assert_eq!(g.last_melee_anim, 23);
    assert!((g.next_primary - 0.35 + 0.01).abs() < 0.02);
    // block costs 33 power
    for _ in 0..60 {
        frame(&mut g, &mut h, 0, &mut last, 0.01);
    }
    frame(&mut g, &mut h, IN_RELOAD, &mut last, 0.01);
    assert!((g.melee_bar - 67.0).abs() < 1e-4);
    assert!(g.block_kungfu > 0.0);
    // the bar regenerates at 15/s
    pre_frame(&mut g, Arith::CLIENT, 1.0, true);
    assert!((g.melee_bar - 82.0).abs() < 1e-4);
}

/// Port addition (plugin API version 5, `melee_attack`): the hook sees the move's delay and the
/// damage it deals (bar and Kung Fu power-up applied), its changes reach the hit and the timers,
/// and a block leaves only the delay.
#[test]
fn kungfu_melee_attack_hook() {
    let mut g = Gun::default();
    let mut h = Mock { flags: 0x200, ..Default::default() };
    kungfu_move(&mut g, &mut h, IN_ATTACK, IN_ATTACK);
    assert_eq!(h.melee_pre, vec![(0.35, 20.0)]);
    assert_eq!(h.melee[0].damage, 20.0);
    // Changed: the hit deals the new damage, the delay is the new time.
    h.melee_answer = Some((1.25, 55.0, true));
    kungfu_move(&mut g, &mut h, IN_ATTACK, IN_ATTACK);
    assert_eq!(h.melee.last().unwrap().damage, 55.0);
    assert_eq!((g.next_primary, g.next_secondary), (1.25, 1.25));
    assert_eq!(h.cooldowns.last(), Some(&1.25));
    // Blocked uppercut: no hit, event, crouch or BlockPlayer; the delay still applies.
    let (hits, crouches) = (h.melee.len(), h.crouches.len());
    h.melee_answer = Some((0.8, 35.0, false));
    (g.next_attack, g.next_primary, g.next_secondary) = (0.0, 0.0, 0.0);
    (h.buttons, h.pressed) = (IN_ATTACK2 | IN_DUCK, IN_ATTACK2);
    h.events.clear();
    g.ccombat_attack(&mut h);
    assert_eq!(h.melee_pre.last(), Some(&(1.0, 35.0)), "the uppercut's own values");
    assert_eq!((h.melee.len(), h.crouches.len()), (hits, crouches));
    assert!(h.events.is_empty() && h.blocks.is_empty());
    assert_eq!(g.next_primary, 0.8);
}

/// `client.dll`'s block (`0x1000adc0`) gates on the HUD's power (`0.01 <= 0x1018a718`), not the
/// gun's bar, and charges nothing: no bar cost, so no 1.5 s `m_flNextAttack` for an empty bar.
#[test]
fn client_block_gates_on_hud_power() {
    crate::tables::make_default();
    let try_block = |bar: f32, hud: Option<f32>| {
        let mut g = Gun { melee_bar: bar, ..Default::default() };
        let mut h = Mock { kfu_power: hud, ..Default::default() };
        let mut last = 0;
        frame(&mut g, &mut h, IN_RELOAD, &mut last, 0.0);
        (g, h.events.iter().any(|e| e.0 == Event::CloseCombat))
    };
    // A drained local bar doesn't stop the client while the HUD has power.
    let (g, blocked) = try_block(0.0, Some(50.0));
    assert!(blocked);
    assert_eq!((g.block_kungfu, g.melee_bar, g.next_attack), (0.5, 0.0, 0.0));
    // A full local bar doesn't let it through once the HUD's power is gone.
    let (g, blocked) = try_block(100.0, Some(0.005));
    assert!(!blocked);
    assert_eq!(g.block_kungfu, 0.0);
    assert!(try_block(100.0, Some(0.01)).1);
    // A block that empties the client's bar leaves the next attack alone.
    let (g, _) = try_block(20.0, Some(20.0));
    assert_eq!((g.melee_bar, g.next_attack), (20.0, 0.0));
    // The server gates on and charges its own bar (`UpdateMeleeBar` 0x9650c).
    let (g, blocked) = try_block(20.0, None);
    assert!(blocked);
    assert_eq!((g.melee_bar, g.next_attack), (0.0, 1.5));
    assert!(!try_block(0.005, None).1);
}

/// One kung-fu attack with the timers cleared; returns the close-combat event's (i1, i2).
fn kungfu_move(g: &mut Gun, h: &mut Mock, buttons: u32, pressed: u32) -> (i32, i32) {
    g.next_attack = 0.0;
    g.next_primary = 0.0;
    g.next_secondary = 0.0;
    h.buttons = buttons;
    h.pressed = pressed;
    h.events.clear();
    g.ccombat_attack(h);
    let cc: Vec<_> = h.events.iter().filter(|e| e.0 == Event::CloseCombat).collect();
    assert_eq!(cc.len(), 1);
    (cc[0].1.i1, cc[0].1.i2)
}

/// The close-combat event's `i2` is TS's set A index (`CCombatAttack` 0x762d7..0x76997, pushed at
/// 0x76daa), not the move id the combos chain on.
#[test]
fn kungfu_view_anim_indices() {
    let mut g = Gun::default();
    let mut h = Mock { flags: 0x200, ..Default::default() };
    // Punches on the ground: fresh 4, held jabs 3, the fourth in a row the heavy punch 11.
    assert_eq!(kungfu_move(&mut g, &mut h, IN_ATTACK, IN_ATTACK), (1, 4));
    for _ in 0..3 {
        assert_eq!(kungfu_move(&mut g, &mut h, IN_ATTACK, 0), (1, 3));
        assert_eq!(g.last_melee_anim, 22);
    }
    assert_eq!(kungfu_move(&mut g, &mut h, IN_ATTACK, 0), (1, 11));
    assert_eq!(g.last_melee_anim, 25);
    // Kicks on the ground: held 9 (moves 11/12), pressed 10 (moves 10/13/14/15).
    assert_eq!(kungfu_move(&mut g, &mut h, IN_ATTACK2, 0), (2, 9));
    assert!(matches!(g.last_melee_anim, 11 | 12));
    assert_eq!(kungfu_move(&mut g, &mut h, IN_ATTACK2, IN_ATTACK2), (2, 10));
    assert!(matches!(g.last_melee_anim, 10 | 13 | 14 | 15));
    // Uppercut 6, back kick 7.
    assert_eq!(kungfu_move(&mut g, &mut h, IN_ATTACK2 | IN_DUCK, IN_ATTACK2), (2, 6));
    assert_eq!(g.last_melee_anim, 20);
    assert_eq!(kungfu_move(&mut g, &mut h, IN_ATTACK2 | IN_BACK, IN_ATTACK2), (3, 7));
    assert_eq!(g.last_melee_anim, 17);
    // Airborne: punch 5, fast kick 10, flying kick 8.
    h.flags = 0;
    assert_eq!(kungfu_move(&mut g, &mut h, IN_ATTACK, IN_ATTACK), (1, 5));
    assert_eq!(g.last_melee_anim, 24);
    h.velocity = [250.0, 0.0, 0.0];
    assert_eq!(kungfu_move(&mut g, &mut h, IN_ATTACK2, IN_ATTACK2), (2, 10));
    assert!(matches!(g.last_melee_anim, 15 | 18 | 19));
    h.velocity = [300.0, 0.0, 0.0];
    assert_eq!(kungfu_move(&mut g, &mut h, IN_ATTACK2 | IN_DUCK, IN_ATTACK2), (2, 8));
    assert_eq!(g.last_melee_anim, 16);
    // A stunt punch: index 4, move -1.
    h.flags = 0x200;
    h.velocity = [0.0; 3];
    h.iuser4 = STUNTING_KUNGFU;
    assert_eq!(kungfu_move(&mut g, &mut h, IN_ATTACK, IN_ATTACK), (1, 4));
    assert_eq!(g.last_melee_anim, -1);
}

#[test]
fn infinite_clip_keeps_clip_and_multiply_doubles() {
    crate::tables::make_default();
    let (mut g, mut h) = armed(id::AK47);
    h.powerups = shared::ts::powerup::INFINITE_CLIP;
    let mut last = 0;
    for _ in 0..50 {
        frame(&mut g, &mut h, 1, &mut last, 0.01);
    }
    assert_eq!(g.status[id::AK47 as usize].clip, 30);
    assert_eq!(g.multiply_clips(81), Some(0));
    assert_eq!(g.status[id::AK47 as usize].clip, 60);
}

#[test]
fn drop_switches_by_priority() {
    crate::tables::make_default();
    let mut g = Gun::default();
    let mut h = Mock { flags: 0x200, ..Default::default() };
    g.take_weapon(id::GLOCK18, None, 0);
    g.take_weapon(id::AK47, None, 0);
    g.take_weapon(id::BARRETT, None, 0);
    g.switch_to(id::BARRETT, &mut h);
    assert!(g.remove_weapon(id::BARRETT, true, &mut h));
    assert_eq!(g.current, id::AK47);
}

#[test]
fn restricted_pickups() {
    crate::tables::make_default();
    let mut g = Gun::default();
    // akimbo ids can't be taken directly when the akimbo version exists... but TakeWeapon allows
    // it (PickUpWeapon rejects them); owning the pair blocks another single.
    g.take_weapon(id::BERETTA, None, 0);
    g.take_weapon(id::BERETTA, None, 0);
    assert_eq!(g.take_weapon(id::BERETTA, None, 0), None);
    // stackables stop at their max count
    for _ in 0..10 {
        g.take_weapon(id::KNIFE, None, 0);
    }
    assert_eq!(g.status[id::KNIFE as usize].clip, 5);
}

/// `CTSGun::Deploy` (0x81030) zeroes the `UpdateClientData` timer (+0x9e0) only when it deploys
/// a weapon; with none it is `HolsterWeapon` (kung fu), which leaves the timer.
#[test]
fn deploy_with_a_weapon_updates_the_client_at_once() {
    crate::tables::make_default();
    let (mut g, mut h) = armed(id::AK47);
    h.update_now = 0;
    g.switch_to(0, &mut h);
    assert_eq!(h.update_now, 0, "kung fu");
    g.switch_to(id::AK47, &mut h);
    assert_eq!(h.update_now, 1);
    g.current = 0;
    g.deploy(&mut h);
    assert_eq!(h.update_now, 1, "Deploy without a weapon holsters");
}

#[test]
fn deploy_remembers_fire_mode() {
    crate::tables::make_default();
    let (mut g, mut h) = armed(id::AK47);
    g.switch_fire_mode();
    assert_eq!(g.status[id::AK47 as usize].mode, mode::SEMI);
    g.status[id::AK47 as usize].mode = 0;
    g.switch_to(0, &mut h);
    g.switch_to(id::AK47, &mut h);
    assert_eq!(g.status[id::AK47 as usize].mode, mode::SEMI);
}

/// The client melee view shakes (`ViewShake::Start(amp, 0.2, 10)`): knife slash damage x 0.15
/// (0x1000a327) and the katana's own ground-swing strength 45 x 0.2 (0x1000a156).
#[test]
fn melee_view_shakes() {
    crate::tables::make_default();
    let (mut g, mut h) = armed(id::KNIFE);
    let mut last = 0;
    frame(&mut g, &mut h, IN_ATTACK, &mut last, 0.01);
    assert_eq!(h.shakes, vec![info(id::KNIFE).unwrap().damage * 0.15]);
    let (mut g, mut h) = armed(id::KATANA);
    let mut last = 0;
    frame(&mut g, &mut h, IN_ATTACK, &mut last, 0.01);
    assert_eq!(h.shakes, vec![45.0 * 0.2]);
    // Airborne: clamp(speed / 6.5, 35, 55).
    let (mut g, mut h) = armed(id::KATANA);
    h.flags = 0;
    h.velocity = [600.0, 0.0, 0.0];
    let mut last = 0;
    frame(&mut g, &mut h, IN_ATTACK, &mut last, 0.01);
    assert_eq!(h.shakes, vec![55.0 * 0.2]);
}

/// One kung-fu attack (timers cleared, nothing hit): the trace it asked for, the delayed-hit
/// window it armed and the `BlockPlayer` time, if any.
fn kungfu_hit(g: &mut Gun, h: &mut Mock, buttons: u32, pressed: u32) -> (MeleeHit, f32, Option<f32>) {
    kungfu_move(g, h, buttons, pressed);
    let hit = *h.melee.last().unwrap();
    let block = h.blocks.pop();
    h.iuser4 = 0;
    (hit, g.melee_window, block)
}

/// `CCombatAttack`'s stack slots as the disassembly uses them (0x76078..0x76e19): reach in units
/// (`esp+0x18`), the `Animate` id (`esp+0x2c`), the delayed-hit window (`esp+0x20`) and the
/// delay (`esp+0x24`), which is also the `BlockPlayer` time x 0.75 and `pl+0x4f8`.
#[test]
fn kungfu_reach_anim_window_and_block() {
    let mut g = Gun::default();
    let mut h = Mock { flags: 0x200, ..Default::default() };
    let (hit, window, block) = kungfu_hit(&mut g, &mut h, IN_ATTACK, IN_ATTACK);
    assert_eq!((hit.reach, hit.hull, hit.player_anim, hit.damage), (35.0, 3, 23, 20.0));
    assert_eq!((window, block), (0.15, None));
    assert_eq!(h.cooldowns.last(), Some(&0.35));
    // A held kick on the ground: 45 units, window 0.2, blocked for 0.3 x 0.75.
    let (hit, window, block) = kungfu_hit(&mut g, &mut h, IN_ATTACK2, 0);
    assert_eq!((hit.reach, hit.damage), (45.0, 15.0));
    assert!(matches!(hit.player_anim, 11 | 12));
    assert_eq!((window, block), (0.2, Some(0.3 * 0.75)));
    // The uppercut forces a crouch (iuser4 4|1), so it never blocks.
    let (hit, window, block) = kungfu_hit(&mut g, &mut h, IN_ATTACK2 | IN_DUCK, IN_ATTACK2);
    assert_eq!((hit.reach, hit.player_anim, hit.damage), (45.0, 20, 35.0));
    assert_eq!((window, block), (0.35, None));
    assert_eq!(h.crouches, vec![5]);
    // The back kick: 55 units behind, human hull, roll 0.
    let (hit, window, block) = kungfu_hit(&mut g, &mut h, IN_ATTACK2 | IN_BACK, IN_ATTACK2);
    assert_eq!((hit.reach, hit.hull, hit.zero_roll, hit.player_anim), (-55.0, 1, true, 17));
    assert_eq!((window, block), (0.3, Some(0.75)));
    assert!(g.melee_damage < 0.0, "the delayed hit looks behind");
    // A stunt punch plays no animation.
    h.iuser4 = STUNTING_KUNGFU;
    let (hit, _, _) = kungfu_hit(&mut g, &mut h, IN_ATTACK, IN_ATTACK);
    assert_eq!((hit.reach, hit.player_anim, hit.damage), (35.0, -1, 15.0));
}

/// `KatanaFire` (0x73580..0x737a8): the reach is 50 units (35 stunting, 55 backward), the damage
/// is the swing's strength (45 ground, 65 crouched, 75 backward) and the event carries the view
/// index.
#[test]
fn katana_swings() {
    crate::tables::make_default();
    let (mut g, mut h) = armed(id::KATANA);
    let mut last = 0;
    frame(&mut g, &mut h, IN_ATTACK, &mut last, 0.01);
    let hit = h.melee[0];
    assert_eq!((hit.reach, hit.hull, hit.damage), (50.0, 3, 45.0));
    let ev = h.events.iter().find(|e| e.0 == Event::Katana).unwrap();
    assert!(matches!(ev.1.i1, 0x10 | 0x11));
    assert_eq!(h.cooldowns, vec![0.7]);
    assert!(h.blocks.is_empty());

    let (mut g, mut h) = armed(id::KATANA);
    let mut last = 0;
    frame(&mut g, &mut h, IN_ATTACK | IN_BACK, &mut last, 0.01);
    let hit = h.melee[0];
    assert_eq!((hit.reach, hit.hull, hit.damage), (-55.0, 1, 75.0));
    assert_eq!(h.blocks, vec![0.9]);
    assert!(g.melee_damage < 0.0);

    // Crouched: the forced crouch sets iuser4 bit 1, so no BlockPlayer.
    let (mut g, mut h) = armed(id::KATANA);
    let mut last = 0;
    frame(&mut g, &mut h, IN_ATTACK | IN_DUCK, &mut last, 0.01);
    assert_eq!((h.melee[0].reach, h.melee[0].damage), (50.0, 65.0));
    assert_eq!(h.crouches, vec![5]);
    assert!(h.blocks.is_empty());
}

#[test]
fn charging_the_melee_bar_empties_it_with_a_lockout() {
    let mut g = Gun { melee_bar: 50.0, next_attack: 0.5, ..Gun::default() };
    g.charge_melee_bar(33.0);
    assert_eq!((g.melee_bar, g.next_attack), (17.0, 0.5));
    // 17 - 33 < 0.01: empty, and `UpdateMeleeBar` overrides the caller's 0.5 s with 1.5 s
    g.charge_melee_bar(33.0);
    assert_eq!((g.melee_bar, g.next_attack), (0.0, 1.5));
    // a remainder below 0.01 empties it too
    let mut g = Gun { melee_bar: 33.005, ..Gun::default() };
    g.charge_melee_bar(33.0);
    assert_eq!((g.melee_bar, g.next_attack), (0.0, 1.5));
    // TS tests `0.01 <= v`, so a NaN bar empties
    let mut g = Gun { melee_bar: f32::NAN, ..Gun::default() };
    g.charge_melee_bar(33.0);
    assert_eq!((g.melee_bar, g.next_attack), (0.0, 1.5));
}

/// `ColdCock`'s event: the server's (0x81dd0) needs the record's owned flag; the prediction's
/// records never have it and its copy (0x10008dc8) plays the event regardless.
#[test]
fn cold_cock_event_owned_gate() {
    crate::tables::make_default();
    let gun = (1..MAX_WEAPONS as u8).find(|&i| info(i).is_some_and(|w| w.stock > 0 && kind_of(i) != kind::GRENADE)).unwrap();
    for (client, owned, plays) in [(false, true, true), (false, false, false), (true, false, true)] {
        let (mut g, mut h) = armed(gun);
        g.status[gun as usize].owned = owned;
        h.client_records = client;
        g.cold_cock(&mut h);
        assert_eq!(h.events.iter().any(|e| e.0 == Event::ColdCock), plays, "client {client}, owned {owned}");
    }
}

#[test]
fn stock_hit_event_sends_no_spread() {
    crate::tables::make_default();
    // Fire a spread shot first: TS's coldcock f1/f2 are ColdCock's never-written static vecDir.
    let (mut g, mut h) = armed(id::M4);
    h.seed = 1234;
    let mut last = 0;
    frame(&mut g, &mut h, IN_ATTACK, &mut last, 0.01);
    assert_eq!(h.shots.len(), 1);
    g.cold_cock(&mut h);
    let (ev, a) = *h.events.last().unwrap();
    assert_eq!(ev, Event::ColdCock);
    assert_eq!((a.f1, a.f2, a.i1, a.b1, a.b2), (0.0, 0.0, id::M4 as i32, 0, 0));
}

#[test]
fn weapon_anims_carry_the_guns_body() {
    crate::tables::make_default();
    // A knife slash that hits a player sets pev->body 1 (the bloody blade); SendWeaponAnim sends
    // pev->body whatever the caller passed, so the draw after a throw keeps it.
    let (mut g, mut h) = armed(id::KNIFE);
    g.status[id::KNIFE as usize].clip = 2;
    h.melee_result = Some(MeleeResult::Hit { player: true });
    let mut last = 0;
    frame(&mut g, &mut h, IN_ATTACK, &mut last, 0.01);
    assert_eq!(g.body, 1);
    g.next_primary = 0.0;
    g.next_secondary = 0.0;
    h.anim_bodies.clear();
    frame(&mut g, &mut h, IN_ATTACK2, &mut last, 0.01);
    assert_eq!(h.blades.len(), 1);
    assert!(!h.anim_bodies.is_empty() && h.anim_bodies.iter().all(|&b| b == 1), "the draw after the throw: {:?}", h.anim_bodies);
    // Holstering to kung fu doesn't touch pev->body either.
    h.anim_bodies.clear();
    assert!(g.switch_to(0, &mut h));
    // (SelectWeapon's holster, then Deploy's on kung fu.)
    assert_eq!(h.anim_bodies, [1, 1]);
}

#[test]
fn deploy_takes_its_timers_from_default_deploy() {
    crate::tables::make_default();
    // client.dll's DefaultDeploy (0x100035f0): 0.5 s before attacking, 3 s idle (not the SDK's 1).
    let mut g = Gun::default();
    let mut h = Mock::default();
    g.take_weapon(id::AK47, None, 0).unwrap();
    assert!(g.switch_to(id::AK47, &mut h));
    assert_eq!((g.next_attack, g.idle), (0.5, 3.0));
    assert_eq!(h.deployed, [id::AK47]);

    // The server's: what the host says (spawn protection left, the Contender's 0.75 s idle).
    let mut g = Gun::default();
    let mut h = Mock { deploy_timers: Some((0.8, 0.75)), ..Default::default() };
    g.take_weapon(id::CONTENDER, None, 0).unwrap();
    assert!(g.switch_to(id::CONTENDER, &mut h));
    assert_eq!((g.next_attack, g.idle), (0.8, 0.75));

    // Kung fu is HolsterWeapon, not DefaultDeploy.
    h.deployed.clear();
    assert!(g.switch_to(0, &mut h));
    assert!(h.deployed.is_empty());
    assert_eq!((g.next_attack, g.idle), (0.5, 3.0));
}

/// The virtual calls TS makes through the gun's vtable reach the host's hook points in TS's order
/// (`CBasePlayerWeapon::ItemPostFrame` 0xa03d4), and a skipped one doesn't run.
#[test]
fn virtual_calls_reach_the_host_in_ts_order() {
    crate::tables::make_default();
    let (mut g, mut h) = armed(id::AK47);
    h.virtuals.clear();
    h.buttons = IN_ATTACK;
    g.item_post_frame(&mut h);
    assert_eq!(
        h.virtuals,
        [
            "pre PostFrame",
            "pre PrimaryAttack",
            "post PrimaryAttack Some(0)",
            "pre ShouldWeaponIdle",
            "post ShouldWeaponIdle Some(1)",
            "pre WeaponIdle",
            "post WeaponIdle Some(0)",
            "post PostFrame Some(0)"
        ]
    );
    assert_eq!(h.shots.len(), 1);
    // No button: straight to WeaponIdle, without ShouldWeaponIdle (0xa07b4).
    let (mut g, mut h) = armed(id::AK47);
    h.virtuals.clear();
    g.weapon_post_frame(&mut h);
    assert_eq!(h.virtuals, ["pre WeaponIdle", "post WeaponIdle Some(0)"]);
    // A skipped attack doesn't fire; the post call still follows.
    let (mut g, mut h) = armed(id::AK47);
    h.skip = Some(Virtual::PrimaryAttack);
    h.buttons = IN_ATTACK;
    g.weapon_post_frame(&mut h);
    assert!(h.shots.is_empty());
    assert!(h.virtuals.contains(&"post PrimaryAttack None".to_owned()));
    // Deploy and Holster are hookable too; switching weapons deploys through the vtable.
    let (mut g, mut h) = armed(id::AK47);
    h.virtuals.clear();
    g.holster(&mut h);
    assert_eq!(h.virtuals, ["pre Holster", "post Holster Some(0)"]);
    assert_eq!(g.run_virtual(&mut h, Virtual::Deploy, true), 1);
    assert_eq!(g.run_virtual(&mut h, Virtual::ShouldWeaponIdle, false), 1);
}

/// The gun hooks are TS 3.0's code.
#[test]
fn gun_hooks_are_3_0s() {
    use shared::gameplay::Predicted;
    use shared::ts::powerup;
    let t30 = &crate::TS30;
    assert!(std::ptr::eq(t30.tables(), crate::tables::tables()));
    for arith in [Arith::Gcc, Arith::Msvc] {
        for (ft, fuser1, pw) in [(0.01f32, 1.0f32, 0u16), (0.001, 0.65, powerup::DOUBLE_FIRERATE), (0.02, 0.0, 0)] {
            let want = Gun::timer_dt(arith, ft, spread::fixed_slow_amt(fuser1), pw);
            assert_eq!(t30.timer_step(arith, ft as f64, fuser1, pw), want as f64);
        }
    }
    for msec in 0..=255u8 {
        assert_eq!(t30.command_frame_time(msec), (msec as f64 * 0.001) as f32 as f64);
    }
    assert_eq!(t30.pellet_offset(5, 1, None), None);
    assert_eq!(t30.akimbo_event_offsets(true, true), (5.0, Some(-5.0)));
    assert_eq!(t30.akimbo_event_offsets(true, false), (5.0, Some(-5.0)));
    assert_eq!(t30.akimbo_event_offsets(false, true), (0.0, None));
    assert_eq!(t30.predicted(), Predicted::ALL);
    assert_eq!(t30.semi_auto_delay(0.07), 0.1);
    let ak = crate::tables::tables().weapon(id::AK47).unwrap();
    let m4 = crate::tables::tables().weapon(id::M4).unwrap();
    assert!(!t30.cancel_runs_frame_in_reload(ak) && t30.cancel_runs_frame_in_reload(m4));
    assert!(!t30.blades_need_press() && t30.katana_throws() && t30.infinite_clip_skips_idle_reload());
    assert!(t30.holster_draws_kungfu());
    let mut g = Gun::default();
    assert_eq!(t30.better_weapon(&g, 0), 0, "nothing owned: kung fu");
    for id in [id::AK47, id::BENELLI] {
        g.status[id as usize].owned = true;
    }
    assert_eq!(t30.better_weapon(&g, 0), id::AK47, "13 comes before 4 in the list");
    assert_eq!(t30.better_weapon(&g, id::AK47), id::BENELLI);
    assert_eq!(PRIORITY[37], 0);
}

/// TS 3.0's decay: by the step, only while alive; the time isn't recorded.
#[test]
fn decay_is_the_step() {
    let mut g = Gun { spread_penalty: 0.05, ..Gun::default() };
    let mut want = g.clone();
    decay_spread(&mut want, Arith::Msvc, 0.01);
    let d = SpreadDecay { arith: Arith::Msvc, step: 0.01, time: 99.0, time_scale: 0.5, alive: true };
    // The crosshair's copy follows the penalty on every first prediction.
    assert!(crate::TS30.decay_spread(&mut g, &d));
    assert_eq!(g, want);
    crate::TS30.decay_spread(&mut g, &SpreadDecay { alive: false, ..d });
    assert_eq!(g, want);
}

/// #249: `PreThink` runs the window and the blocks down before `CTSStunt::Play` (0x909c5..) and
/// closes the window after it, before `CheckForBreakables` (0x91163..): a window that falls
/// below 0.01 this frame is still open between the two and closed before the delayed hit.
#[test]
fn pre_think_splits_around_play() {
    let t = WeaponPreThink { arith: Arith::Msvc, frametime: 0.01, time: 10.0, fuser1: 1.0, powerups: 0, alive: true };
    let mut g = Gun { melee_window: 0.015, melee_damage: 15.0, block_kungfu: 0.5, melee_bar: 50.0, ..Gun::default() };
    let mut disarm = 7.0;
    crate::TS30.pre_think_timers(&mut g, &mut disarm, &t);
    assert!(g.melee_window > 0.0 && g.melee_window < 0.01, "run down, not closed: {}", g.melee_window);
    assert_eq!((g.melee_damage, g.melee_bar), (15.0, 50.0), "no bar before Play");
    assert!(g.block_kungfu < 0.5 && disarm < 7.0);
    let d = disarm;
    crate::TS30.pre_think(&mut g, &mut disarm, &t);
    assert_eq!((g.melee_window, g.melee_damage, g.block_kungfu), (0.0, 0.0, 0.0), "closed before the delayed hit");
    assert!(g.melee_bar > 50.0);
    assert_eq!(disarm, d, "the disarm count runs down before Play only");
    // A window `Play` opens after the run-down isn't run down this frame.
    let mut g = Gun::default();
    crate::TS30.pre_think_timers(&mut g, &mut 0.0, &t);
    g.melee_window = 0.3;
    g.melee_damage = 15.0;
    crate::TS30.pre_think(&mut g, &mut 0.0, &t);
    assert_eq!(g.melee_window, 0.3);
}
