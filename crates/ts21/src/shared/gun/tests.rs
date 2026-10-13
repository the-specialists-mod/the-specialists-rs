use super::*;
use shared::gameplay::CORE;
use shared::weapons::gun::{MeleeHit, MeleeResult, Shot, Sound};
use shared::weapons::spread::Vec3;
use shared::weapons::{Status, att, mode_mask, table};

/// The gun code reads the active tables: these tests run with 2.1's (and none sets another).
fn setup() {
    table::set_ruleset_tables(tables::tables());
}

#[derive(Default)]
struct Mock {
    time: f32,
    fuser1: f32,
    powerups: u16,
    buttons: u32,
    pressed: u32,
    velocity: Vec3,
    seed: u32,
    client: bool,
    repredict: bool,
    events: Vec<(Event, EventArgs)>,
    shots: Vec<Shot>,
    kicks: Vec<(f32, f32, bool)>,
}

impl GunHost for Mock {
    fn time(&self) -> f32 {
        self.time
    }
    fn slow_mul(&self) -> f32 {
        shared::weapons::spread::fixed_slow_amt(self.fuser1)
    }
    fn time_scale(&self) -> f32 {
        self.fuser1
    }
    fn prediction(&self) -> bool {
        self.client
    }
    fn first_prediction(&self) -> bool {
        !self.repredict
    }
    fn arith(&self) -> Arith {
        Arith::CLIENT
    }
    fn rules(&self) -> &'static dyn GunRules {
        &crate::TS21
    }
    fn powerups(&self) -> u16 {
        self.powerups
    }
    fn kungfu(&self) -> bool {
        false
    }
    fn alive(&self) -> bool {
        true
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
        [0.0; 3]
    }
    fn velocity(&self) -> Vec3 {
        self.velocity
    }
    fn v_angle(&self) -> Vec3 {
        [0.0; 3]
    }
    fn flags(&self) -> i32 {
        0x200
    }
    fn waterlevel(&self) -> i32 {
        0
    }
    fn iuser4(&self) -> i32 {
        0
    }
    fn set_iuser4(&mut self, _v: i32) {}
    fn seed(&self) -> u32 {
        self.seed
    }
    fn random_long(&mut self, lo: i32, _hi: i32) -> i32 {
        lo
    }
    fn fov(&self) -> f32 {
        0.0
    }
    fn set_fov(&mut self, _fov: f32) {}
    fn set_view_model(&mut self, _id: Option<u8>) {}
    fn view_model_hidden(&self) -> bool {
        false
    }
    fn set_weapon_model(&mut self, _model: Option<(u8, bool)>) {}
    fn event(&mut self, ev: Event, args: EventArgs) {
        self.events.push((ev, args));
    }
    fn weapon_anim(&mut self, _anim: i32, _skip_local: bool, _body: i32) {}
    fn player_anim(&mut self, _anim: i32) {}
    fn kick(&mut self, pitch: f32, yaw: f32, shake: bool) {
        self.kicks.push((pitch, yaw, shake));
    }
    fn sound(&mut self, _s: Sound) {}
    fn shoot(&mut self, shot: &Shot, _gun: &mut Gun) {
        self.shots.push(*shot);
    }
    fn melee(&mut self, _hit: &MeleeHit, _gun: &mut Gun) -> MeleeResult {
        MeleeResult::Miss
    }
    fn throw_grenade(&mut self, _origin: Vec3, _velocity: Vec3, _roll: bool) {}
    fn throw_blade(&mut self, _btype: i32) {}
    fn slots_changed(&mut self, _delta_free: i32) {}
    fn melee_bar_changed(&mut self, _bar: f32) {}
    fn block_player(&mut self, _t: f32) {}
}

/// A gun holding weapon `id` (full clip, default mode) and the host, fuser1 1.0 (normal).
fn armed(id: u8) -> (Gun, Mock) {
    setup();
    let mut g = Gun::default();
    let w = info(id).unwrap();
    let mut s = Status { owned: true, ammo: w.ammo, mode: shared::weapons::default_mode(mode_mask(w)), clip: w.clip, ..Default::default() };
    if w.kind == shared::weapons::WeaponKind::AkimboOnly || w.kind == shared::weapons::WeaponKind::Akimbo {
        s.left = w.clip / 2;
        s.right = w.clip - s.left;
    }
    g.status[id as usize] = s;
    g.current = id;
    (g, Mock { time: 10.0, fuser1: 1.0, ..Default::default() })
}

fn fire_once(g: &mut Gun, h: &mut Mock) {
    crate::TS21.fire(g, h);
}

#[test]
fn the_2_1_table_set() {
    setup();
    let t = tables::tables();
    assert_eq!(t.weapons().count(), 34, "33 weapons and kung fu");
    for gone in [id::BERETTA, id::CONTENDER, id::SKORPION_AKIMBO, 29] {
        assert!(t.weapon(gone).is_none(), "{gone}");
    }
    let tmp = t.weapon(17).unwrap();
    assert_eq!(
        (tmp.name.as_str(), tmp.ammo, tmp.clip, tmp.rate, tmp.reload, tmp.cost, tmp.slots),
        ("STEYR-TMP", 1, 30, 0.05, 1.8, 1600, 20)
    );
    assert_eq!(t.asset(17).unwrap().model, "tmp");
    assert_eq!(t.anims(17), &tables::TMP_SEQUENCES);
    assert_eq!(t.anims(13), shared::weapons::table::base_tables().anims(13), "3.0's sequences for the 3.0 models");
    // Damage = the ammo's base damage x the weapon's multiplier.
    let dmg = |id: u8| t.weapon(id).unwrap().damage;
    assert_eq!(dmg(id::AK47), 52.5);
    assert_eq!(dmg(id::GLOCK18), 45.0 * 0.675f32);
    assert_eq!(dmg(id::BARRETT), 150.0);
    assert_eq!(dmg(id::RAGING_BULL), 120.0);
    assert_eq!(tables::ammo_damage(12), 160);
    assert_eq!(tables::damage_multiplier(id::MOSSBERG), 0.74);
    assert_eq!(t.ammo(12).unwrap().name, ".454 Casull");
    assert!(t.ammo(11).is_none() && t.ammo(13).is_none());
    assert_eq!(t.ammo(10).unwrap().pierce, 0.05);
    assert_eq!(t.ammo(2).unwrap().max_carry, [200, 48, 16, 0]);
    // The stock hit: a flag, 20 on the M4 and the Sawed-off only.
    let stocks: Vec<u8> = t.weapons().filter(|w| w.stock != 0).map(|w| w.id).collect();
    assert_eq!(stocks, vec![id::M4, id::SAWEDOFF]);
    assert_eq!(t.weapon(id::M4).unwrap().stock, 20);
    // Akimbo Berettas stand alone; the UMP has no burst.
    assert_eq!(t.weapon(8).unwrap().single, -1);
    assert_eq!(mode_mask(t.weapon(id::UMP).unwrap()), 3);
    assert_eq!(t.combat.silencer.spread, 1.2);
    assert_eq!(t.combat.headshot_shotgun, 2.0);
}

#[test]
fn delays_are_fixed_at_fire_time() {
    let delay = |powerups: u16, fuser1: f32, client: bool| {
        let (mut g, mut h) = armed(id::AK47);
        h.powerups = powerups;
        h.fuser1 = fuser1;
        h.client = client;
        fire_once(&mut g, &mut h);
        g.next_primary
    };
    assert_eq!(delay(0, 1.0, false), 0.08);
    assert_eq!(delay(0, 0.0, false), 0.08, "fuser1 0 is normal speed in 2.1");
    assert_eq!(delay(powerup::DOUBLE_FIRERATE, 1.0, false), 0.04);
    assert_eq!(delay(0, 0.5, false), (2.0f64 * 0.08f32 as f64) as f32);
    assert_eq!(delay(0, 0.05, false), (10.0f64 * 0.08f32 as f64) as f32, "the server floors fuser1 at 0.1");
    assert_eq!(delay(0, 0.05, true), (1.0 / 0.05f32 as f64 * 0.08f32 as f64) as f32, "the client doesn't");
    assert_eq!(delay(powerup::SLOWPAUSE, 0.5, false), (0.08f32 as f64 * 6.666_666_5f32 as f64) as f32);
    assert_eq!(delay(powerup::SLOWPAUSE, 0.5, true), (0.08f32 as f64 / 0.15) as f32);
}

#[test]
fn semi_waits_the_rate_and_shotgun_semi_is_0_1() {
    let (mut g, mut h) = armed(id::M4);
    g.status[id::M4 as usize].mode = shared::weapons::mode::SEMI;
    h.powerups = powerup::DOUBLE_FIRERATE;
    fire_once(&mut g, &mut h);
    assert_eq!(g.next_primary, 0.07, "semi: the rate, no 3.0 0.1, no double fire rate");

    let (mut g, mut h) = armed(id::BENELLI);
    g.status[id::BENELLI as usize].mode = shared::weapons::mode::SEMI;
    fire_once(&mut g, &mut h);
    assert_eq!(g.next_primary, 0.1);
    assert_eq!(h.shots.len(), 1);
    // Kick x1.3 in semi: the server applies the kick itself as the pitch.
    assert_eq!(h.kicks[0].0, 1.3f32 * 11.5);
    assert!(g.status[id::BENELLI as usize].toggle, "the pump toggles after every shot");
}

#[test]
fn silencer_widens_the_spread_and_cuts_damage() {
    let (mut g, mut h) = armed(id::MP5K);
    g.status[id::MP5K as usize].items = att::SILENCER;
    fire_once(&mut g, &mut h);
    let shot = h.shots[0];
    // ScaleSpread adds the (empty) penalty: the spread is accuracy x 1.2.
    assert_eq!(shot.spread, (0.029f32 as f64 * 1.2) as f32);
    assert_eq!(shot.damage, 45.0 * (0.7f32 as f64 * 0.8) as f32);
}

#[test]
fn moving_barrett_and_m60_set_the_penalty() {
    for (wid, speed, want) in [(id::BARRETT, 81.0, 0.1), (id::BARRETT, 79.0, 0.0), (id::M60, 100.0, 0.1), (id::AK47, 300.0, 0.0)] {
        let (mut g, mut h) = armed(wid);
        h.velocity = [speed, 0.0, 0.0];
        h.repredict = true; // keep ScaleSpread from growing the penalty further
        fire_once(&mut g, &mut h);
        assert_eq!(g.spread_penalty, want, "weapon {wid} at {speed}");
    }
}

#[test]
fn scale_spread_grows_by_elapsed_over_t() {
    let (mut g, mut h) = armed(id::AK47);
    g.last_shot = 9.9;
    g.spread_penalty = 0.01;
    let mut spread = 0.003f32;
    scale_spread(&mut g, &mut spread, 0.02, &h);
    assert_eq!(spread, 0.003 + 0.01);
    // k = 2 without double fire rate, no silencer factor: (1 - 0.1) * 2 * 0.02 + 0.01.
    let dt = 10.0f32 - 9.9f32;
    assert_eq!(g.spread_penalty, ((1.0 - dt as f64) * 2.0 * 0.02f32 as f64 + 0.01f32 as f64) as f32);
    assert_eq!(g.last_shot, 10.0);
    // A shot at the same instant still grows it (2.1 needs dt >= 0, 3.0 dt >= 0.001).
    let before = g.spread_penalty;
    scale_spread(&mut g, &mut spread, 0.02, &h);
    assert!(g.spread_penalty > before);
    // akick 0 leaves everything alone; so does a client re-prediction.
    h.repredict = true;
    let p = g.spread_penalty;
    scale_spread(&mut g, &mut spread, 0.02, &h);
    assert_eq!(g.spread_penalty, p);
}

#[test]
fn view_kick_is_random_and_sided() {
    for seed in [0u32, 7, 123, 31999] {
        let (g, mut h) = armed(id::AK47);
        let _ = g;
        h.seed = seed;
        vkickback(6.0, kind::RIFLE, false, &mut h);
        h.client = true;
        vkickback(6.0, kind::RIFLE, false, &mut h);
        let (sp, sy, sshake) = h.kicks[0];
        let (cp, cy, cshake) = h.kicks[1];
        assert!(!sshake && !cshake, "no view shake in 2.1");
        assert_eq!(sp, 6.0, "the server (bots) kicks by the full kick");
        // p = SR(seed+1, 3, 6): the range truncates to 3.
        assert!((3.0..6.0).contains(&cp), "seed {seed}: pitch {cp}");
        // r2 = SR(seed+2, 0, 0.75) truncates to 0: yaw = (u ± 0.5) * 6 * 0.65, |u ± 0.5| in [0.5, 1).
        let u = shared::weapons::random::shared_random_float(seed, -0.5, 0.5);
        let side = if u < 0.0 { -0.5 } else { 0.5 };
        assert_eq!(sy, ((u as f64 + side) * 6.0 * 0.65) as f32);
        assert!((cy - sy).abs() < 1e-5);
        assert!(sy.abs() >= 0.5 * 6.0 * 0.65 - 1e-5 && sy.abs() < 6.0 * 0.65);
    }
    // The client applies no kick for a kick of 0.
    let (_, mut h) = armed(id::AK47);
    h.client = true;
    vkickback(0.0, kind::RIFLE, false, &mut h);
    assert!(h.kicks.is_empty());
}

#[test]
fn akimbo_shots_are_4_units_off_centre() {
    let (mut g, mut h) = armed(id::MK23_AKIMBO);
    g.status[id::MK23_AKIMBO as usize].mode = shared::weapons::mode::AKIMBO_SEMI;
    h.buttons = IN_ATTACK | IN_ATTACK2;
    h.pressed = IN_ATTACK | IN_ATTACK2;
    h.seed = 5;
    fire_once(&mut g, &mut h);
    let toggle = g.status[id::MK23_AKIMBO as usize].toggle;
    let first = if toggle { -4.0 } else { 4.0 };
    assert_eq!(h.shots.iter().map(|s| (s.right_offset, s.seed)).collect::<Vec<_>>(), vec![(first, 5), (-first, 5 + 0x21)]);
    assert_eq!(g.next_primary, 0.1);
    assert_eq!(g.next_secondary, 0.1);

    let (mut g, mut h) = armed(id::MK23_AKIMBO);
    g.status[id::MK23_AKIMBO as usize].mode = shared::weapons::mode::AKIMBO_SEMI;
    h.buttons = IN_ATTACK2;
    h.pressed = IN_ATTACK2;
    fire_once(&mut g, &mut h);
    assert!(g.status[id::MK23_AKIMBO as usize].toggle);
    assert_eq!(h.shots[0].right_offset, 4.0);
    assert_eq!(crate::TS21.akimbo_event_offsets(false, true), (4.0, None));
    assert_eq!(crate::TS21.akimbo_event_offsets(true, false), (4.0, Some(-4.0)));
}

#[test]
fn infinite_clip_saves_the_round_but_an_empty_clip_clicks() {
    let (mut g, mut h) = armed(id::AK47);
    h.powerups = powerup::INFINITE_CLIP;
    fire_once(&mut g, &mut h);
    assert_eq!(g.status[id::AK47 as usize].clip, 30);
    g.status[id::AK47 as usize].clip = 0;
    g.next_primary = 0.0;
    h.shots.clear();
    fire_once(&mut g, &mut h);
    assert!(h.shots.is_empty());
    assert_eq!(h.events.last().unwrap().0, Event::Empty);
    assert_eq!(g.next_primary, 0.1f64 as f32);
    assert_eq!(g.idle, 0.5);
}

#[test]
fn triangle_pellets() {
    for seed in [0u32, 1, 77, 31999] {
        for i in 1..=8 {
            let (x, y) = triangle(seed, i);
            assert!((-1.0..1.0).contains(&x) && (-1.0..1.0).contains(&y));
            let u = |k: u32| shared::weapons::random::shared_random_float(seed + i + k, -0.5, 0.5);
            assert_eq!((x, y), (u(0) + u(1), u(2) + u(3)));
            assert_eq!(crate::TS21.pellet_offset(seed, i, None), Some((x, y)));
        }
    }
    // A bot's pellets: engine random floats, redrawn outside the unit circle.
    let mut draws = [0.5f32, 0.49, 0.5, 0.49, 0.1, 0.2, -0.3, 0.0].into_iter();
    let mut rf = |_lo: f32, _hi: f32| draws.next().unwrap();
    assert_eq!(crate::TS21.pellet_offset(3, 1, Some(&mut rf)), Some((0.3, -0.3)));
}

#[test]
fn timers_run_on_raw_time() {
    setup();
    let r = &crate::TS21;
    assert_eq!(r.timer_step(Arith::CLIENT, 0.01, 0.35, powerup::DOUBLE_FIRERATE), 0.01);
    assert_eq!(r.command_frame_time(7), 7.0 / 1000.0);
    // The client subtracts the unrounded quotient.
    let mut g = Gun { next_secondary: 0.3, ..Gun::default() };
    g.decrement_timers(r.command_frame_time(7));
    assert_eq!(g.next_secondary, (0.3f32 as f64 - 0.007) as f32);
}

#[test]
fn spread_decays_by_elapsed_time() {
    setup();
    let r = &crate::TS21;
    let mut g = Gun { spread_penalty: 0.05, decay_time: 10.0, ..Gun::default() };
    let d = |time: f32, time_scale: f32, alive: bool| SpreadDecay { arith: Arith::CLIENT, step: 123.0, time, time_scale, alive };
    r.decay_spread(&mut g, &d(10.1, 1.0, true));
    assert_eq!(g.spread_penalty, (0.05f32 as f64 - (10.1f32 - 10.0f32) as f64 * 0.1) as f32);
    assert_eq!(g.decay_time, 10.1);
    // In slow motion it decays by game time x fuser1.
    let p = g.spread_penalty;
    r.decay_spread(&mut g, &d(10.2, 0.5, true));
    assert_eq!(g.spread_penalty, (p as f64 - (10.2f32 - 10.1f32) as f64 / 2.0 * 0.1) as f32);
    // Dead: no decay, but the time moves on; and it stops at 0.
    let p = g.spread_penalty;
    r.decay_spread(&mut g, &d(20.0, 1.0, false));
    assert_eq!((g.spread_penalty, g.decay_time), (p, 20.0));
    r.decay_spread(&mut g, &d(30.0, 1.0, true));
    assert_eq!(g.spread_penalty, 0.0);
}

#[test]
fn prediction_and_gate() {
    setup();
    let r = &crate::TS21;
    assert_eq!(r.predicted(), Predicted { reload_finish: false, alt_attack: false, kungfu_reload: false });
    // The client's frame doesn't run the stock hit; +alt2 still lets the server's frame run in a
    // reload of a gun without one.
    let ak = info(id::AK47).unwrap();
    assert!(r.cancel_runs_frame_in_reload(ak));
    assert!(!CORE.cancel_runs_frame_in_reload(ak));
    let mut g = Gun { current: id::AK47, next_attack: 1.0, ..Gun::default() };
    g.status[id::AK47 as usize] = Status { owned: true, in_reload: 1, ..Default::default() };
    assert!(g.post_frame_ready(0x40, r));
    assert!(!g.post_frame_ready(0x40, &CORE));
}
