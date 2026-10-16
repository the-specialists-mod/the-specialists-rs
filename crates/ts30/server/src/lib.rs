//! TS 3.0's server rules (issue #200) and the plugin object `server-dll` registers.
//!
//! * [`PLUGIN`]: the gameplay plugin, [`Ts30Server`] with the shared [`TS30`] hooks.
//! * [`rules`]: the multiplayer install, the server browser's description and
//!   `CTSGameRules::Think`.
//! * [`commands`]: the chat's text rules and dead-chat gate, and `kill`.
//! * [`players`]: The One, teams, the respawn time, the overheal and the spawn loadout.
//! * [`observer`]: the observer part of `PreThink` and `Observer_SetMode`.
//! * [`scoring`]: `PlayerKilled` and `TSGetPointsForFrag`.
//! * [`items`]: pickups, `TakeHealth`, the battery and the wall chargers.
//! * [`entities`]: the map entities' rules (objectives, givers, `RespawnEntities`, ground weapons).
//! * [`powerup`]: power-ups and slow motion.
//! * [`stunt`]: the stunt values, clock and slot limits.
//! * [`damage`]: bullets and what a hit does to a player.
//! * [`melee`]: blocks on the victim and the stunt kicks.
//! * [`explode`]: explosions and thrown blades.
//! * [`bots::BOTS`]: the native bot backend's rules for 3.0, which `server-dll` registers with
//!   `ts_server::bots::rules` under the plugin's id (the bots are a layer of their own, so they
//!   aren't part of [`PLUGIN`]).

#![deny(unsafe_code)]

pub mod bots;
pub mod commands;
pub mod damage;
pub mod entities;
pub mod explode;
pub mod items;
pub mod melee;
pub mod observer;
pub mod players;
pub mod powerup;
pub mod rules;
pub mod scoring;
pub mod stunt;

use goldsrc::server::MsgWriter;
use goldsrc::sys::{TraceResult, edict_t, entvars_t};
use shared::pm::{Arith, Ext};
use shared::weapons::gun::{MeleeKind, MeleeResult};
use shared::weapons::spread::Vec3;
use shared::weapons::{Gun, Status, WeaponInfo};
use std::ffi::CStr;
use ts_server::commands::Args;
use ts_server::damage::DamageInfo;
use ts_server::damage::glast::Snapshot;
use ts_server::entities::ts::ctb::GiverWin;
use ts_server::entities::ts::ground::GroundWeapon;
use ts_server::entities::ts::misc::MapFlags;
use ts_server::entities::ts::objective::ObjectiveRules;
use ts_server::gameplay::server::CvarDef;
use ts_server::gameplay::{ConnectingClient, ServerPlugin, ServerRules};
use ts_server::messages::Msg;
use ts_server::player::Player;
use ts_server::player::damage::{HitDamage, PlayerHit};
use ts_server::rules::DescriptionMode;
use ts_server::rules::commands::SayRules;
use ts_server::rules::powerup::PowerupRules;
use ts_server::rules::scoring::Frag;
use ts_server::rules::state::GameRules;
use ts_server::stunt::rules::{Dive, StuntClock, StuntRules};
use ts_server::weapons::ThrownBlade;
use ts_server::world::{Cx, Entity};
use ts30_gameplay::TS30;

/// TS 3.0's server-only rules.
pub struct Ts30Server;

impl ServerRules for Ts30Server {
    /// Nothing: 3.0 keeps no state of its own.
    fn on_activate(&self) {}

    /// `None`: everyone may join.
    fn refuse_client(&self, _client: ConnectingClient) -> Option<&'static str> {
        None
    }

    /// "The Specialists v3.0.2 (DM)" and so on ([`rules::description`]).
    fn game_description(&self, mode: DescriptionMode) -> &'static CStr {
        rules::description(mode)
    }

    /// None: TS 3.0's own (`ts_server::rules::cvars::CVARS`) are the server's.
    fn cvars(&self) -> &'static [CvarDef] {
        &[]
    }

    /// False: the command runs.
    fn server_command(&self, _name: &str) -> bool {
        false
    }

    /// `None`: 3.0's handlers (weapons, stunts, rules, bots) run.
    fn client_command(&self, _p: &mut Player, _cx: &mut Cx, _cmd: &str, _args: &Args) -> Option<bool> {
        None
    }

    /// `DecideGamePlay`, the rules' constructors and `CTSGameRules::Init` ([`rules::install`]).
    fn install(&self, g: &mut GameRules) {
        rules::install(g);
    }

    /// `CTSGameRules::Think` (0x77204, [`rules::think`]).
    fn think(&self, now: f32) {
        rules::think(now);
    }

    /// `CTSGameRules::SetPlayerTheOne` (0x7b078, [`players::set_player_the_one`]).
    fn set_player_the_one(&self, p: &mut Player, cx: &mut Cx) {
        players::set_player_the_one(p, cx);
    }

    /// In The One mode The One's team or the other one, otherwise the `team` key
    /// ([`players::default_team`]).
    fn default_team(&self, p: &mut Player, cx: &mut Cx) -> Vec<u8> {
        players::default_team(p, cx)
    }

    /// Nothing.
    fn on_spawn(&self, _p: &mut Player, _cx: &mut Cx) {}

    /// `initial_cash`, which TS never registers: [`INITIAL_CASH`](ts_server::rules::cvars::INITIAL_CASH).
    fn put_in_server_cash(&self) -> i32 {
        ts_server::rules::cvars::INITIAL_CASH
    }

    /// Nothing.
    fn on_start_observer(&self, _p: &mut Player, _cx: &mut Cx) {}

    /// The countdown, the join deadline, `Observer_HandleButtons` and `Observer_Idle`
    /// ([`observer::observer_think`]); 3.0 respawns with the `respawn` command instead.
    fn observer_think(&self, p: &mut Player, cx: &mut Cx, now: f32) {
        observer::observer_think(p, cx, now);
    }

    /// `CBasePlayer::Observer_SetMode` (0x99e44, [`observer::set_mode`]).
    fn observer_set_mode(&self, p: &mut Player, cx: &mut Cx, mode: i32) {
        observer::set_mode(p, cx, mode);
    }

    /// Nothing.
    fn on_init_hud(&self, _p: &mut Player, _cx: &mut Cx) {}

    /// `SetPlayerRespawnTime` (0x75ff4, [`players::set_respawn_time`]).
    fn set_respawn_time(&self, p: &mut Player, cx: &mut Cx, secs: f32) {
        players::set_respawn_time(p, cx, secs);
    }

    /// Nothing.
    fn on_killed(&self, _p: &mut Player, _cx: &mut Cx) {}

    /// A change that isn't a kill, outside The One mode, observes with a 5 s respawn
    /// ([`players::change_team_tail`]).
    fn change_team_tail(&self, p: &mut Player, cx: &mut Cx, kill: bool) {
        players::change_team_tail(p, cx, kill);
    }

    /// The 1/s overheal decay and The One's regeneration ([`players::health_regen`]).
    fn health_regen(&self, p: &mut Player, cx: &mut Cx, now: f32) {
        players::health_regen(p, cx, now);
    }

    /// `TSGetPointsForFrag` (0x89140, [`scoring::points_for_frag`]).
    fn points_for_frag(&self, frag: &Frag<'_>, sliding: &mut bool, diving: &mut bool) -> i32 {
        scoring::points_for_frag(frag, sliding, diving)
    }

    /// The body of `CHalfLifeMultiplay::PlayerKilled` (0x895e4, [`scoring::player_killed`]).
    fn player_killed(&self, p: &mut Player, cx: &mut Cx, d: &DamageInfo, g: Snapshot) {
        scoring::player_killed(p, cx, d, g);
    }

    /// `CashBased()` (0x76abc): hard-wired off.
    fn cash_based(&self) -> bool {
        false
    }

    /// `CBasePlayer::BuyArmament` ([`players::buy_armament`]).
    fn buy_armament(&self, p: &mut Player, cx: &mut Cx) {
        players::buy_armament(p, cx);
    }

    /// [`items::pick_up_cost`]: refuses the akimbo pair of a single gun and charges a stack
    /// `slots × count`.
    fn pick_up_cost(&self, id: u8, w: &WeaponInfo, from: Option<&Status>) -> Option<f32> {
        items::pick_up_cost(id, w, from)
    }

    /// While `TSGameRules.m_mode < 2` (always in 3.0's DM/TDM: no grenade buys,
    /// [`weapons::grenade_buys`](ts_server::weapons::grenade_buys)).
    fn dead_grenades_become_powerups(&self) -> bool {
        !ts_server::weapons::grenade_buys()
    }

    fn drop_lifetime_and_glow(&self) -> bool {
        true
    }

    /// The whole stack, but one on a disarm.
    fn stack_drop_count(&self, held: i32, disarm: bool) -> i32 {
        if disarm { 1 } else { held }
    }

    fn world_gun_fall_impact(&self) -> bool {
        true
    }

    /// Nothing.
    fn world_gun_touch(&self, _gun: *mut edict_t, _attacker: *mut edict_t, _id: u8, _other: *mut edict_t) {}

    /// False: the touch removes it.
    fn ammo_pack_stays_when_nothing_fits(&self) -> bool {
        false
    }

    /// [`items::add_health`] (0xd8760), uncapped.
    fn player_add_health(&self, v: &mut entvars_t, amount: f32) -> Option<bool> {
        items::add_health(v, amount)
    }

    /// `CBasePlayer::TakeArmor` ([`items::player_take_armor`]: the first kevlar costs its slots,
    /// the move speed is recomputed; the result is ignored).
    fn battery_armor(&self, p: *mut edict_t, amount: f32) {
        items::player_take_armor(p, amount);
    }

    /// 1 (`CRecharge::Spawn` 0xc6ee4, `CWallHealth::Spawn`).
    fn charger_capacity(&self, _armor: bool) -> i32 {
        1
    }

    /// [`items::charge_once`]: `CRecharge::Use` (0xc6fd0) empties the charger into
    /// `TakeArmor(100)` below 100 armour; `CWallHealth::Use` (0xc8d08) gives
    /// `TakeHealth(max_health - health)` and empties it if that was taken.
    fn charger_charge(&self, armor: bool, juice: &mut i32, activator: *mut edict_t) {
        items::charge_once(armor, juice, activator);
    }

    /// `None`: every class is TS 3.0's own.
    fn entity_object(&self, _class: &CStr) -> Option<Box<dyn Entity>> {
        None
    }

    /// None.
    fn precache(&self) {}

    /// TS 3.0 has an export for every class the port links: all spawn.
    fn spawns_entity(&self, _class: &CStr) -> bool {
        true
    }

    /// `None`: the class parses it.
    fn entity_key_value(&self, _ent: &mut dyn Entity, _cx: &mut Cx, _key: &CStr, _value: &CStr) -> Option<bool> {
        None
    }

    /// The class's.
    fn entity_touch(&self, _ent: &mut dyn Entity, _cx: &mut Cx, _other: *mut edict_t) -> bool {
        false
    }

    /// [`entities::OBJECTIVES`].
    fn objectives(&self) -> &'static ObjectiveRules {
        &entities::OBJECTIVES
    }

    /// [`entities::round_time_left`] (only while a timed round runs: the manager's
    /// `targetontimeleft`, else a draw).
    fn round_time_left(&self, now: f32) {
        entities::round_time_left(now);
    }

    /// [`entities::giver_team_win`] (`TeamWin(winnerteam, (int)value, restart, isScore)`).
    fn giver_team_win(&self, w: &GiverWin) {
        entities::giver_team_win(w);
    }

    /// True: from the last hit.
    fn break_debris_from_hit(&self) -> bool {
        true
    }

    /// The stock entities' `Vector::Normalize` ([`ts_server::damage::normalize`]).
    fn breakable_attack_dir(&self, inflictor_origin: Vec3, absmin: Vec3, size: Vec3) -> Vec3 {
        let centre: Vec3 = std::array::from_fn(|i| absmin[i] + size[i] * 0.5);
        ts_server::damage::normalize(std::array::from_fn(|i| inflictor_origin[i] - centre[i]))
    }

    /// The key, 0 meaning 30 s on the `info_breakable` helper (`RespawnThink` 0x75da0).
    fn broken_respawn_delay(&self, respawn_time: f32) -> Option<f32> {
        Some(if respawn_time == 0.0 { 30.0 } else { respawn_time })
    }

    fn breakable_respawn_guarded(&self) -> bool {
        true
    }

    /// [`entities::respawn_entities`].
    fn respawn_entities(&self) {
        entities::respawn_entities();
    }

    /// TS 3.0 has none.
    fn entities_think(&self, _now: f32) {}

    /// [`MapFlags::from_spawnflags`].
    fn map_globals(&self, _e: *mut edict_t, spawnflags: i32) -> Option<MapFlags> {
        Some(MapFlags::from_spawnflags(spawnflags))
    }

    /// No (`CTSGroundWeapon::OnGround` 0xa226c).
    fn ground_weapon_lands_flying(&self) -> bool {
        false
    }

    /// [`entities::give_ground_weapon`].
    fn give_ground_weapon(&self, g: &mut GroundWeapon, cx: &mut Cx, p: &mut Player, pcx: &mut Cx) {
        entities::give_ground_weapon(g, cx, p, pcx);
    }

    /// The message goes out as it is.
    fn outgoing_message(&self, _msg: Msg, w: MsgWriter, send: &mut dyn FnMut(MsgWriter)) {
        send(w);
    }

    /// [`powerup::POWERUPS`].
    fn powerups(&self) -> &'static PowerupRules {
        &powerup::POWERUPS
    }

    /// [`powerup::kind_for`].
    fn powerup_kind(&self, key: i32, spawnflags: i32) -> (u16, bool) {
        powerup::kind_for(key, spawnflags)
    }

    /// `TakeArmor(100)`, which costs slots, whatever the power-up is worth.
    fn armor_powerup(&self, p: &mut Player, cx: &mut Cx, armor: f32) {
        let _ = armor;
        ts_server::weapons::take_armor(p, cx, 100);
    }

    fn time_scale_eases(&self) -> bool {
        true
    }

    /// Nothing.
    fn on_go_slow(&self, _p: &mut Player, _amount: f32) {}

    /// `GiveSlowMul() * 0.1f + elapsed` with `GiveSlowMul` at least 0.01 ([`powerup::fuse_step`]).
    fn grenade_fuse_step(&self, elapsed: f32, time_scale: f32) -> f32 {
        powerup::fuse_step(ts_server::rules::cvars::pm_arith(), elapsed, time_scale.max(0.01))
    }

    /// `max(fuser1, 0.01)`.
    fn anim_framerate(&self, time_scale: f32) -> f32 {
        time_scale.max(0.01)
    }

    fn motion_blur(&self) -> bool {
        true
    }

    /// [`powerup::slow_pitch`].
    fn sound_pitch(&self, pitch: i32, time_scale: f32, s: f32) -> i32 {
        powerup::slow_pitch(pitch, time_scale, s)
    }

    /// [`stunt::STUNTS`].
    fn stunts(&self) -> &'static StuntRules {
        &stunt::STUNTS
    }

    /// [`stunt::RELATIVE`].
    fn stunt_clock(&self) -> &'static dyn StuntClock {
        &stunt::RELATIVE
    }

    /// [`stunt::can_dive`]: the armour doesn't count.
    fn can_dive(&self, p: &Player, armor: f32, dive: Dive) -> bool {
        let _ = armor;
        stunt::can_dive(p, dive)
    }

    /// `GiveSlowMul() <= 0.99`.
    fn bullet_time_allowed(&self, time_scale: f32) -> bool {
        time_scale.max(0.01) <= 0.99
    }

    /// `UTIL_TraceLine_Expanded` (0x6f4f4, [`damage::trace_expanded`]), which grows every player's
    /// box by 32 units, ignoring the shooter.
    fn bullet_trace(&self, start: Vec3, end: Vec3, shooter: *mut edict_t) -> TraceResult {
        damage::trace_expanded(start, end, shooter)
    }

    /// Truncated (`ts_i386.so` `fistpl` under the chop control word, 0x6fdbb; mp.dll `__ftol`,
    /// 0x1004801c), issue #80.
    fn bullet_range_index(&self, scaled: Ext) -> i32 {
        scaled.trunc_i32()
    }

    /// `falloff × damage`.
    fn bullet_damage(&self, ammo: u8, weapon: u8, damage: f32, falloff: f32, simulated: bool) -> f32 {
        let _ = (ammo, weapon, simulated);
        falloff * damage
    }

    /// `cheat` or 1 (`assets::damage_adjust`).
    fn bullet_damage_multiplier(&self, acc: &mut f32, cheat: Option<f32>) -> f32 {
        let _ = acc;
        cheat.unwrap_or(1.0)
    }

    /// `min(2·left, 48)`.
    fn pierce_step(&self, left: f32) -> f32 {
        (left + left).min(48.0)
    }

    /// The rules' value.
    fn weapon_realbullet(&self, realbullet: i32) -> i32 {
        realbullet
    }

    /// [`spread::bullet_velocity_mult`](shared::weapons::spread::bullet_velocity_mult): mp.dll's
    /// stored `float`, `ts_i386.so`'s unrounded quotient.
    fn bullet_velocity_mult(&self, arith: Arith, slowmul: f32, bullet_time: bool) -> Ext {
        shared::weapons::spread::bullet_velocity_mult(arith, slowmul, bullet_time)
    }

    /// [`damage::hit_damage`].
    fn player_hit(&self, hit: &PlayerHit) -> HitDamage {
        damage::hit_damage(hit)
    }

    /// The kevlar absorbs `armor_absorb` and is never depleted ([`damage::after_kevlar`]).
    fn armor_damage(&self, armor: &mut f32, amount: f32, hitgroup: i32, bits: i32) -> f32 {
        damage::after_kevlar(amount, *armor, hitgroup, bits, ts_server::rules::state::armor_absorb())
    }

    /// None.
    fn damage_punch(&self) -> f32 {
        0.0
    }

    /// [`melee::hold`], whatever the damage.
    fn melee_block(
        &self,
        kind: MeleeKind,
        attacker: *mut edict_t,
        victim: *mut edict_t,
        damage: f32,
        atk_block: f32,
    ) -> Option<MeleeResult> {
        let _ = damage;
        melee::hold(kind, attacker, victim, atk_block)
    }
    /// `m_flNextAttack` 0.35 if it had run out, and `UpdateMeleeBar(strength × 1.5)`
    /// ([`melee::jump_kick_hit`]).
    fn jump_kick_hit(&self, p: &mut Player, cx: &mut Cx, strength: f32) {
        melee::jump_kick_hit(p, cx, strength);
    }

    /// The jump kick closes the attacker's kung-fu block first (`CTSStunt::CloseAttack`), so the
    /// hit's own block test sees none.
    fn jump_kick_attacker_block(&self, gun: &mut Gun) -> f32 {
        gun.block_kungfu = 0.0;
        0.0
    }

    /// 22.5 kung-fu power and a delayed hit of 15 (30 with Kung Fu) ([`melee::backflip_kick`]).
    fn backflip_kick(&self, p: &mut Player, cx: &mut Cx) {
        melee::backflip_kick(p, cx);
    }

    /// No (0x8c1f1, `dmg <= 0` goes straight to the hit).
    fn zero_delayed_hit_reaches_player(&self) -> bool {
        false
    }

    /// The damage, whoever the victim.
    fn melee_disarm(&self, damage: f32, victim_is_the_one: bool) -> Option<f32> {
        let _ = victim_is_the_one;
        Some(damage)
    }

    /// 110, 300, 1024.
    fn grenade_explosion(&self) -> (f32, f32, f32) {
        (110.0, 300.0, 1024.0)
    }

    fn explosion_pushes_world_guns(&self) -> bool {
        true
    }

    /// The "spectacular" slow-motion point (0.25 for 1 s, [`explode::explosion_slow_point`]).
    fn explosion_aftermath(&self, org: Vec3, inflictor: *mut edict_t, attacker: *mut edict_t) {
        let _ = (inflictor, attacker);
        explode::explosion_slow_point(org);
    }

    /// [`explode::thrown_blade`].
    fn thrown_blade(&self, btype: i32) -> ThrownBlade {
        explode::thrown_blade(btype)
    }

    /// 150 katana, 100 combat knife, 50 seal knife, however far it flew
    /// ([`explode::thrown_blade_damage`]).
    fn thrown_blade_damage(&self, btype: i32, from: Vec3, at: Vec3) -> f32 {
        let _ = (from, at);
        explode::thrown_blade_damage(btype)
    }

    // Audit round 2: B1

    /// [`commands::SAY_RULES`]: `%a`/`%h`, a vote's text lowered, no `ff` reply.
    fn say_rules(&self) -> &'static SayRules {
        &commands::SAY_RULES
    }

    /// Round modes 2/3 or LMS ([`commands::say_dead_chat_gated`]).
    fn say_dead_chat_gated(&self) -> bool {
        commands::say_dead_chat_gated()
    }

    /// The suicide a second later ([`commands::client_kill`]).
    fn client_kill(&self, p: &mut Player, cx: &mut Cx) {
        commands::client_kill(p, cx);
    }

    /// `SET_VIEW(edict, edict)` and `EnableControl(TRUE)` (0x8eee0).
    fn killed_resets_view(&self) -> bool {
        true
    }
}

/// The plugin as the server registers it.
pub static PLUGIN: ServerPlugin = ServerPlugin { pm: &TS30, gun: &TS30, server: &Ts30Server };

#[cfg(test)]
mod tests {
    use super::*;

    /// The hooks that need no engine.
    #[test]
    fn hooks_without_the_engine() {
        use ConnectingClient::*;
        let s: &dyn ServerRules = &Ts30Server;
        for c in [Bot, Hltv, Port("3.0.2"), Other] {
            assert_eq!(s.refuse_client(c), None, "3.0 refuses no one");
        }
        assert!(s.cvars().is_empty(), "3.0 registers only its own cvars");
        assert!(!s.cash_based(), "3.0's CashBased is hard-wired off");
        assert_eq!(s.put_in_server_cash(), ts_server::rules::cvars::INITIAL_CASH);
        assert!(!s.server_command("globalslow"), "3.0 runs its server commands");
        assert!(!s.server_command("restartround"));
        assert!(!s.server_command("weaponrestriction"));
        let v = env!("CARGO_PKG_VERSION");
        assert_eq!(s.game_description(DescriptionMode::Lms).to_str().unwrap(), format!("The Specialists v{v} (LMS)"));
    }
}
