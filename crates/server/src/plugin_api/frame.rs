//! The plugin API's per-frame world (version 6): every player slot as a `PlayerSnapshot`, the
//! highest entity in use, and the items lying in the map, so a bot backend reads the world once a
//! frame instead of calling `player_info` per player and scanning entities and classnames itself.
//!
//! [`begin`] builds the frame at `StartFrame`, before the plugins' `start_frame`
//! (`plugins::start_frame`), when no player is inside its own callbacks. A query before the first
//! frame, or in a frame where no plugin or listener was there at `StartFrame` ([`invalidate`]),
//! builds it then.

use super::{Game, Host, Snapshots, inventory, snapshot};
use crate::entities::ts::{ground::GroundWeapon, powerup::PowerUp};
use crate::player::Player;
use crate::weapons::worldgun::WorldGun;
use crate::world;
use goldsrc::Global;
use goldsrc::edict::EdictPtr;
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::any::TypeId;
use std::cell::RefCell;
use std::ffi::c_int;
use ts_plugin_api::{CombatState, FrameInfo, ItemInfo, PlayerInfo, PlayerSnapshot, WeaponEntry, entity, player_flag, stunt_bits};

/// The frame's snapshot.
#[derive(Default)]
struct Frame {
    /// `gpGlobals->time` it was built at (`None`: never).
    built: Option<f32>,
    frametime: f32,
    highest_edict: c_int,
    /// Slots 1..=maxClients.
    players: Vec<PlayerSnapshot>,
}

static FRAME: Global<RefCell<Option<Frame>>> = Global(RefCell::new(None));
/// A client's last inventory and its serial (`PlayerSnapshot::inventory_serial`).
type Serial = (Vec<WeaponEntry>, c_int);

static SERIALS: Global<RefCell<Snapshots<Serial>>> = Global(RefCell::new(Snapshots(Vec::new())));

/// `StartFrame`: builds the frame.
pub fn begin() {
    let eng = engine();
    let g = eng.globals();
    let players = (1..=eng.max_clients()).map(slot).collect();
    let frame = Frame { built: Some(g.time), frametime: g.frametime, highest_edict: highest_edict(), players };
    *FRAME.0.borrow_mut() = Some(frame);
}

/// `StartFrame` without a plugin to build for: the next query (a metamod plugin's) builds the
/// frame anew rather than reading an old one.
pub fn invalidate() {
    if let Some(f) = FRAME.0.borrow_mut().as_mut() {
        f.built = None;
    }
}

/// The frame, built now if it never was (or [`invalidate`] dropped it).
fn with_frame<R>(f: impl FnOnce(&Frame) -> R) -> R {
    if FRAME.0.borrow().as_ref().is_none_or(|fr| fr.built.is_none()) {
        begin();
    }
    f(FRAME.0.borrow().as_ref().expect("built"))
}

/// The highest entity index in use (0, the world, when nothing else is).
fn highest_edict() -> c_int {
    let eng = engine();
    (1..eng.globals().maxEntities)
        .rev()
        .find(|&i| {
            let e = eng.entity_of_index(i);
            !e.is_null() && !e.is_free()
        })
        .unwrap_or(0)
}

/// The next inventory serial: the same while the list is, one up when it changed (1 for a new
/// client).
pub fn next_serial(last: Option<&Serial>, now: &[WeaponEntry]) -> c_int {
    match last {
        None => 1,
        Some((inv, serial)) if inv.as_slice() == now => *serial,
        Some((_, serial)) => serial.wrapping_add(1).max(1),
    }
}

/// What the player's entvars say, for [`player_flags`].
#[derive(Clone, Copy, Debug, Default)]
pub struct Body {
    pub fl: i32,
    pub movetype: i32,
    pub waterlevel: i32,
    pub observer: i32,
    pub iuser4: i32,
}

/// `PlayerSnapshot::flags` from the API's player state and the entvars.
pub fn player_flags(info: &PlayerInfo, b: Body) -> c_int {
    let bit = |on: bool, f: c_int| if on { f } else { 0 };
    bit(info.joined != 0, player_flag::JOINED)
        | bit(info.alive != 0, player_flag::ALIVE)
        | bit(info.is_bot != 0, player_flag::BOT)
        | bit(b.fl & FL_ONGROUND as i32 != 0, player_flag::ON_GROUND)
        | bit(b.fl & FL_DUCKING as i32 != 0, player_flag::DUCKING)
        | bit(b.movetype == MOVETYPE_FLY as i32, player_flag::ON_LADDER)
        | bit(b.waterlevel >= 2, player_flag::IN_WATER)
        | bit(info.has_flag != 0, player_flag::HAS_FLAG)
        | bit(b.observer != 0, player_flag::SPECTATOR)
        | bit(b.iuser4 & stunt_bits::PRONE != 0, player_flag::PRONE)
        | bit(b.iuser4 & stunt_bits::STUNTING != 0, player_flag::STUNTING)
}

/// The raw timers [`combat`] turns into end times.
#[derive(Clone, Copy, Debug, Default)]
pub struct Timers {
    pub melee_bar: f32,
    /// The gun's (`m_flNextAttack`, primary delay): seconds left in both rulesets.
    pub next_attack: f32,
    pub next_primary: f32,
    /// The ruleset's clock (`StuntClock`): seconds left (TS 3.0) or end times (TS 2.1).
    pub next_stunt: f32,
    pub recover: f32,
    pub block_timer: f32,
    pub prone_lock: f32,
    pub block_kungfu: f32,
    pub block_knife: f32,
    pub block_katana: f32,
    pub melee_window: f32,
    /// An end time in both.
    pub dodge_until: f32,
}

/// `CombatState` at `now`; `ends_at` is the ruleset clock's (`StuntClock::ends_at`).
pub fn combat(t: &Timers, now: f32, ends_at: impl Fn(f32) -> f32) -> CombatState {
    let left = |secs: f32| if secs > 0.0 { now + secs } else { 0.0 };
    CombatState {
        melee_power: t.melee_bar,
        next_attack_at: left(t.next_attack.max(t.next_primary)),
        next_stunt_at: ends_at(t.next_stunt),
        recover_until: ends_at(t.recover).max(ends_at(t.block_timer)),
        prone_lock: ends_at(t.prone_lock),
        dodge_until: if t.dodge_until > now { t.dodge_until } else { 0.0 },
        block_kungfu: ends_at(t.block_kungfu),
        block_knife: ends_at(t.block_knife),
        block_katana: ends_at(t.block_katana),
        melee_window: ends_at(t.melee_window),
    }
}

fn timers(p: &Player) -> Timers {
    let (g, s) = (&p.inv.gun, &p.stunt);
    Timers {
        melee_bar: g.melee_bar,
        next_attack: g.next_attack,
        next_primary: g.next_primary,
        next_stunt: s.next_stunt,
        recover: s.recover,
        block_timer: s.block_timer,
        prone_lock: s.prone_lock,
        block_kungfu: g.block_kungfu,
        block_knife: g.block_knife,
        block_katana: g.block_katana,
        melee_window: g.melee_window,
        dodge_until: s.dodge_until,
    }
}

/// One slot's snapshot: zeroes for an empty slot. The player's own state comes from its object
/// when it can be reached, else from its last `player_info` snapshot.
fn slot(player: c_int) -> PlayerSnapshot {
    let Some(uid) = Game.user_id(player) else { return PlayerSnapshot::default() };
    let e = Game::edict(player);
    let now = engine().time();
    let clock = crate::gameplay::active().server.stunt_clock();
    let live = world::with_lent::<Player, _>(e, |p| {
        (snapshot(p, e.v()), inventory(p), combat(&timers(p), now, |t| clock.ends_at(now, t)), p.deaths)
    });
    let (info, combat_state, deaths) = match live {
        Some((info, inv, c, d)) => {
            let mut serials = SERIALS.0.borrow_mut();
            let serial = next_serial(serials.get(player, uid).as_ref(), &inv);
            serials.put(player, uid, (inv, serial));
            (info, c, d)
        }
        None => (super::player_info(player).unwrap_or_default(), CombatState::default(), 0),
    };
    let serial = SERIALS.0.borrow().get(player, uid).map_or(1, |(_, s)| s);
    let v = e.v();
    let body = Body { fl: v.flags, movetype: v.movetype, waterlevel: v.waterlevel, observer: v.iuser1, iuser4: v.iuser4 };
    PlayerSnapshot {
        user_id: uid,
        flags: player_flags(&info, body),
        team: info.team,
        origin: v.origin,
        velocity: v.velocity,
        v_angle: v.v_angle,
        view_ofs: v.view_ofs,
        absmin: v.absmin,
        absmax: v.absmax,
        health: v.health,
        armor: v.armorvalue,
        buttons: v.button,
        weapon: info.weapon,
        clip: info.clip,
        ammo: info.ammo,
        fire_mode: info.fire_mode,
        attachments: info.attachments,
        stance: info.stunt_state,
        stunt_bits: v.iuser4,
        active_powerup: info.active_powerup,
        stored_powerup: info.stored_powerup,
        powerup_until: info.powerup_until,
        time_scale: info.time_scale,
        frags: v.frags as c_int,
        deaths,
        free_slots: info.free_slots,
        inventory_serial: serial,
        combat: combat_state,
    }
}

/// `PluginApi::players`: slots 1..=maxClients.
pub fn players() -> Vec<PlayerSnapshot> {
    with_frame(|f| f.players.clone())
}

/// `PluginApi::frame_info`.
pub fn frame_info() -> FrameInfo {
    let (time, frametime, highest) = with_frame(|f| (f.built.unwrap_or(0.0), f.frametime, f.highest_edict));
    let s = super::world_info::game_state();
    FrameInfo { time, frametime, highest_edict: highest, teamplay: s.teamplay, ctb: s.ctb, mode: s.mode, round: s.round }
}

/// When a taken item shows again: its think, while hidden and scheduled; 0 when it is available
/// or waits for no think (a round mode's weapon kept for `RespawnEntities`).
pub fn respawn_at(hidden: bool, nextthink: f32, now: f32) -> f32 {
    if hidden && nextthink > now { nextthink } else { 0.0 }
}

/// `PluginApi::items`: ground weapons, power-ups and dropped guns, by index.
pub fn items() -> Vec<ItemInfo> {
    let eng = engine();
    let now = eng.time();
    let top = with_frame(|f| f.highest_edict).max(highest_edict());
    let mut out = Vec::new();
    for i in eng.max_clients() + 1..=top {
        let e = eng.entity_of_index(i);
        if e.is_null() || e.is_free() || !e.has_private_data() {
            continue;
        }
        let ty = world::type_of(e);
        let kinds = [TypeId::of::<GroundWeapon>(), TypeId::of::<PowerUp>(), TypeId::of::<WorldGun>()];
        if !ty.is_some_and(|t| kinds.contains(&t)) {
            continue;
        }
        let Ok(info) = super::world_info::entity_info(i) else { continue };
        let v = e.v();
        let hidden = v.effects & EF_NODRAW as i32 != 0;
        out.push(ItemInfo {
            index: i,
            serial: e.serial_number(),
            kind: info.kind,
            weapon: info.weapon,
            powerup: info.powerup,
            origin: v.origin,
            available: info.available,
            respawn_at: if info.kind == entity::GUN { 0.0 } else { respawn_at(hidden, v.nextthink, now) },
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(weapon: c_int, clip: c_int) -> WeaponEntry {
        WeaponEntry { weapon, clip, ..WeaponEntry::default() }
    }

    #[test]
    fn inventory_serials_move_on_change_only() {
        let a = vec![w(5, 30)];
        assert_eq!(next_serial(None, &a), 1, "a new client starts at 1");
        let last = (a.clone(), 1);
        assert_eq!(next_serial(Some(&last), &a), 1);
        assert_eq!(next_serial(Some(&last), &[w(5, 29)]), 2, "a shot changes the clip");
        assert_eq!(next_serial(Some(&last), &[]), 2, "dropped");
        assert_eq!(next_serial(Some(&(a.clone(), c_int::MAX)), &[]), 1, "wraps past 0");
    }

    #[test]
    fn flags_from_state_and_entvars() {
        let info = PlayerInfo { joined: 1, alive: 1, is_bot: 1, has_flag: 1, ..PlayerInfo::default() };
        let b = Body {
            fl: (FL_ONGROUND | FL_DUCKING) as i32,
            movetype: MOVETYPE_FLY as i32,
            waterlevel: 2,
            observer: 0,
            iuser4: stunt_bits::PRONE | stunt_bits::DIVING,
        };
        let f = player_flags(&info, b);
        use player_flag::*;
        assert_eq!(f, JOINED | ALIVE | BOT | ON_GROUND | DUCKING | ON_LADDER | IN_WATER | HAS_FLAG | PRONE | STUNTING);
        let none =
            player_flags(&PlayerInfo::default(), Body { waterlevel: 1, observer: 3, iuser4: stunt_bits::RECOVER, ..Body::default() });
        assert_eq!(none, SPECTATOR, "knee deep isn't in water; getting up isn't a stunt");
    }

    #[test]
    fn combat_timers_become_end_times() {
        let t = Timers {
            melee_bar: 40.0,
            next_attack: 0.2,
            next_primary: 0.5,
            next_stunt: 1.0,
            recover: 0.001,
            block_timer: 0.3,
            prone_lock: 0.0,
            block_kungfu: 0.25,
            dodge_until: 101.0,
            ..Timers::default()
        };
        // TS 3.0's countdown: seconds left, over below 0.01.
        let c = combat(&t, 100.0, |s| if s >= 0.01 { 100.0 + s } else { 0.0 });
        assert_eq!(c.melee_power, 40.0);
        assert_eq!(c.next_attack_at, 100.5, "the later of the two gun delays");
        assert_eq!((c.next_stunt_at, c.recover_until, c.prone_lock), (101.0, 100.3, 0.0), "a resting 0.001 is over");
        assert_eq!((c.block_kungfu, c.block_knife, c.dodge_until), (100.25, 0.0, 101.0));
        assert_eq!(combat(&t, 102.0, |_| 0.0).dodge_until, 0.0, "a past dodge is none");
    }

    #[test]
    fn respawn_times() {
        assert_eq!(respawn_at(true, 130.0, 100.0), 130.0);
        assert_eq!(respawn_at(false, 130.0, 100.0), 0.0, "available");
        assert_eq!(respawn_at(true, 90.0, 100.0), 0.0, "kept for the next round");
    }
}
