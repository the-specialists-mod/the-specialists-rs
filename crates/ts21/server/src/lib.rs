//! TS 2.1's server rules (issue #100 §3) and the plugin object `server-dll` registers. Hooks it
//! doesn't override keep TS 3.0's body.
//!
//! * [`rules`]: the mode from the map name, teams, `CTSGameRules::Think` and the round modes.
//! * [`scoring`]: `TSGetPointsForFrag` and `PlayerKilled`.
//! * [`cash`]: `CashBased`, the cash floor, kill cash and buying.
//! * [`players`]: spawn, join, death, respawn timing, overheal and The One.
//! * [`observer`]: the observer part of `PreThink`, where fire respawns (issue #113).
//! * [`cvars`] and [`commands`]: 2.1's extra cvars, the polled cvars and the client commands.
//! * [`entities`]: the map entities (§3.9).
//! * [`protocol`]: 2.1 on TS 3.0's wire (§1.2).
//! * [`powerup`]: power-ups and slow motion (§3.7).
//! * [`stunt`]: the server stunts and their slot limits (§3.8, §3.5).
//! * [`damage`]: bullets and what a hit does to a player (§3.1).
//! * [`explode`], [`blades`]: explosions, shrapnel and thrown knives (§3.2).
//! * [`melee`]: blocks and disarms on the victim (§3.2; the attacks are the shared gun hooks).
//! * [`worldgun`]: dropped guns and ammo packs.
//! * [`items`]: the SDK's `TakeHealth`, battery and wall chargers.
//! * [`bots`]: the native bot backend's rules for 2.1 (§3.10): `server-dll` registers [`bots::BOTS`]
//!   with `ts_server::bots::rules` under the plugin's id (the bots are a layer of their own, so
//!   they aren't part of [`PLUGIN`]).

#![deny(unsafe_code)]

pub mod blades;
pub mod bots;
pub mod cash;
pub mod commands;
pub mod cvars;
pub mod damage;
pub mod entities;
pub mod explode;
pub mod items;
pub mod melee;
pub mod observer;
pub mod players;
pub mod powerup;
pub mod protocol;
pub mod rules;
pub mod scoring;
pub mod stunt;
pub mod worldgun;

use goldsrc::server::MsgWriter;
use goldsrc::server::{DONT_IGNORE_MONSTERS, engine};
use goldsrc::sys::{TraceResult, edict_t, entvars_t};
use shared::weapons::gun::{MeleeKind, MeleeResult};
use shared::weapons::spread::Vec3;
use shared::weapons::{Gun, Status, WeaponInfo};
use std::ffi::CStr;
use ts_server::commands::Args;
use ts_server::damage::DamageInfo;
use ts_server::damage::glast::Snapshot;
use ts_server::entities::ts::ground::GroundWeapon;
use ts_server::entities::ts::misc::MapFlags;
use ts_server::gameplay::server::CvarDef;
use ts_server::gameplay::{ConnectingClient, ServerPlugin, ServerRules};
use ts_server::messages::Msg;
use ts_server::player::Player;
use ts_server::player::damage::{HitDamage, PlayerHit};
use ts_server::rules::DescriptionMode;
use ts_server::rules::powerup::PowerupRules;
use ts_server::rules::scoring::Frag;
use ts_server::rules::state::{self, GameRules, Mode};
use ts_server::stunt::rules::{Dive, StuntClock, StuntRules};
use ts_server::weapons::ThrownBlade;
use ts_server::world::{Cx, Entity};
use ts21_gameplay::TS21;

/// TS 2.1's server-only rules.
pub struct Ts21Server;

impl ServerRules for Ts21Server {
    /// Nothing: 2.1's state is set up by its `install`.
    fn on_activate(&self) {}

    fn refuse_client(&self, client: ConnectingClient) -> Option<&'static str> {
        refuse_client(client)
    }

    fn cvars(&self) -> &'static [CvarDef] {
        cvars::CVARS
    }

    /// 3.0's text with `-21` after the mode, so the server browser tells 2.1 servers apart
    /// ("The Specialists v3.0.2 (DM-21)"; port addition).
    fn game_description(&self, mode: DescriptionMode) -> &'static CStr {
        match mode {
            DescriptionMode::Single => ts_server::game_description!("21"),
            DescriptionMode::Dm => ts_server::game_description!("DM-21"),
            DescriptionMode::Lms => ts_server::game_description!("LMS-21"),
            DescriptionMode::Teamplay => ts_server::game_description!("Teamplay-21"),
            DescriptionMode::TheOne => ts_server::game_description!("The One-21"),
        }
    }

    fn server_command(&self, name: &str) -> bool {
        commands::server_command(name)
    }

    fn client_command(&self, p: &mut Player, cx: &mut Cx, cmd: &str, args: &Args) -> Option<bool> {
        commands::client_command(p, cx, cmd, args)
    }

    fn install(&self, g: &mut GameRules) {
        rules::install(g);
    }

    fn think(&self, now: f32) {
        rules::think(now);
    }

    fn set_player_the_one(&self, p: &mut Player, cx: &mut Cx) {
        players::set_player_the_one(p, cx);
    }

    fn default_team(&self, p: &mut Player, cx: &mut Cx) -> Vec<u8> {
        players::default_team(p, cx)
    }

    fn on_spawn(&self, p: &mut Player, _cx: &mut Cx) {
        players::on_spawn(p);
    }

    /// 2.1's `ClientPutInServer` gives no cash; `InitHUD` sets it ([`players::on_init_hud`]).
    fn put_in_server_cash(&self) -> i32 {
        0
    }

    fn on_start_observer(&self, p: &mut Player, _cx: &mut Cx) {
        cash::cash_floor(p);
    }

    /// 2.1's observer block of `PreThink` (0x100537e0): fire respawns once the time has passed.
    fn observer_think(&self, p: &mut Player, cx: &mut Cx, now: f32) {
        observer::observer_think(p, cx, now);
    }

    /// 2.1's `Observer_SetMode` (FUN_1005aad0): a waiting roaming observer is held still.
    fn observer_set_mode(&self, p: &mut Player, cx: &mut Cx, mode: i32) {
        observer::set_mode(p, cx, mode);
    }

    fn on_init_hud(&self, p: &mut Player, _cx: &mut Cx) {
        players::on_init_hud(p);
    }

    fn set_respawn_time(&self, p: &mut Player, cx: &mut Cx, secs: f32) {
        players::set_respawn_time(p, cx, secs);
    }

    /// 2.1's `Killed` (0x10052390): the cash floor and, in DM/TDM, an armour power-up holding the
    /// victim's armour where it died.
    fn on_killed(&self, p: &mut Player, cx: &mut Cx) {
        cash::cash_floor(p);
        let armor = cx.pev().armorvalue;
        if state::with(|g| matches!(g.mode, Mode::Dm | Mode::TeamDm)) && armor > 0.0 {
            // `CreatePowerUp(0x80, -1)`, then `pev->origin` and `+0x7c = (int)armorvalue`: the
            // armour it gives (`PlayerAddPowerUp`, `powerup::armor`), not its seconds.
            let e = ts_server::entities::ts::powerup::create(shared::ts::powerup::ARMOR, -1, cx.pev().origin);
            ts_server::entities::ts::powerup::set_armor(e, cvars::trunc(armor) as f32);
        }
    }

    fn change_team_tail(&self, p: &mut Player, cx: &mut Cx, kill: bool) {
        players::change_team_tail(p, cx, kill);
    }

    fn health_regen(&self, p: &mut Player, cx: &mut Cx, now: f32) {
        players::health_regen(p, cx, now);
    }

    fn points_for_frag(&self, frag: &Frag<'_>, sliding: &mut bool, diving: &mut bool) -> i32 {
        scoring::points_for_frag(frag, sliding, diving)
    }

    fn player_killed(&self, p: &mut Player, cx: &mut Cx, d: &DamageInfo, g: Snapshot) {
        scoring::player_killed(p, cx, d, g);
    }

    fn cash_based(&self) -> bool {
        cash::cash_based()
    }

    fn buy_armament(&self, p: &mut Player, cx: &mut Cx) {
        cash::buy_armament(p, cx);
    }

    fn pick_up_cost(&self, _id: u8, w: &WeaponInfo, _from: Option<&Status>) -> Option<f32> {
        cash::pick_up_cost(w)
    }

    /// 2.1's `DropWeapon` (FUN_10056080) has no dead-player branch: `Killed`'s drop throws one
    /// M61 as a world gun (with the drop sound), whatever the mode.
    fn dead_grenades_become_powerups(&self) -> bool {
        false
    }

    /// `DropWeapon` (FUN_10056080) leaves every thrown gun `CTSWorldGun::Spawn`'s
    /// `clamp(weaponstay, 5, 60)` (0x100650f0), and nothing marks the dropper.
    fn drop_lifetime_and_glow(&self) -> bool {
        false
    }

    /// `DropWeapon`'s stack branch (FUN_10056080, `info+0xd4`: the two knives) copies the status
    /// with `clip = 1` and throws it: one knife per drop, disarm or not, so `Killed`'s per-id drop
    /// leaves one knife by a corpse (issue #166).
    fn stack_drop_count(&self, _held: i32, _disarm: bool) -> i32 {
        1
    }

    /// `CTSWorldGun::Fall` (0x10065550) has no impact trace ([`worldgun::touch`] instead).
    fn world_gun_fall_impact(&self) -> bool {
        false
    }

    fn world_gun_touch(&self, gun: *mut edict_t, attacker: *mut edict_t, id: u8, other: *mut edict_t) {
        worldgun::touch(gun, attacker, id, other);
    }

    /// `CTSAmmoPack::WaitTouch` (0x10065a90): `9mmclip1.wav` and `UTIL_Remove` only if something
    /// was added; a full player leaves the pack for the next.
    fn ammo_pack_stays_when_nothing_fits(&self) -> bool {
        true
    }

    fn player_add_health(&self, v: &mut entvars_t, amount: f32) -> Option<bool> {
        items::add_health(v, amount)
    }

    fn battery_armor(&self, p: *mut edict_t, amount: f32) {
        items::battery_armor(p, amount);
    }

    fn charger_capacity(&self, armor: bool) -> i32 {
        items::charger_capacity(armor)
    }

    fn charger_charge(&self, armor: bool, juice: &mut i32, activator: *mut edict_t) {
        items::charger_charge(armor, juice, activator);
    }

    fn entity_object(&self, class: &CStr) -> Option<Box<dyn Entity>> {
        items::entity_object(class)
    }

    fn precache(&self) {
        items::precache();
    }

    fn spawns_entity(&self, class: &CStr) -> bool {
        entities::spawns(class)
    }

    fn entity_key_value(&self, ent: &mut dyn Entity, cx: &mut Cx, key: &CStr, value: &CStr) -> Option<bool> {
        entities::key_value(ent, cx, key, value)
    }

    fn entity_touch(&self, ent: &mut dyn Entity, cx: &mut Cx, other: *mut edict_t) -> bool {
        entities::touch(ent, cx, other)
    }

    /// 2.1's `CBreakable::Die` (0x10042540) and `CBaseDoor::TraceAttack` (0x10013c50) leave the
    /// entity broken: their think (FUN_10015e10) only pushes `nextthink` on.
    fn broken_respawn_delay(&self, _respawn_time: f32) -> Option<f32> {
        None
    }

    fn breakable_respawn_guarded(&self) -> bool {
        false
    }

    fn objectives(&self) -> &'static ts_server::entities::ts::objective::ObjectiveRules {
        &rules::OBJECTIVES
    }

    /// 2.1's `RoundTimeLeft` (0x100450f0), from the exploding bomb.
    fn round_time_left(&self, now: f32) {
        rules::round_time_left(now);
    }

    /// 2.1's win giver pays the team cash (`CTSWinGiver::Use` 0x10050d10).
    fn giver_team_win(&self, w: &ts_server::entities::ts::ctb::GiverWin) {
        rules::giver_team_win(w, goldsrc::server::engine().time());
    }

    /// 2.1's `GoSlow` (0x10055220) clamps the weapon timers on the way back to normal speed.
    fn on_go_slow(&self, p: &mut Player, amount: f32) {
        powerup::on_go_slow(p, amount);
    }

    /// 2.1's `CM61Grenade::WaitThink` (0x100477e0): the 0.1 floor and the `double` 0.1.
    fn grenade_fuse_step(&self, elapsed: f32, time_scale: f32) -> f32 {
        stunt::grenade_fuse_step(elapsed, time_scale)
    }

    /// 2.1's `PreThink` stops the animation below a tenth of normal speed.
    fn anim_framerate(&self, time_scale: f32) -> f32 {
        stunt::anim_framerate(time_scale)
    }

    /// 2.1's `CBreakable::Die` (0x10042540) and `CBaseDoor::TraceAttack` (0x10013c50) throw the
    /// SDK's debris (issue #109).
    fn break_debris_from_hit(&self) -> bool {
        false
    }

    /// 2.1's `CBreakable::TakeDamage` (0x1001b960) normalises inline.
    fn breakable_attack_dir(&self, inflictor_origin: [f32; 3], absmin: [f32; 3], size: [f32; 3]) -> [f32; 3] {
        entities::breakable_attack_dir(inflictor_origin, absmin, size)
    }

    /// `RestartRound` (0x100445d0) forces it.
    fn respawn_entities(&self) {
        entities::respawn_entities(true);
    }

    /// Nothing: 2.1's `CTSGameRules::Think` ([`rules`], the `think` hook) does its own entity
    /// upkeep (`RespawnEntities(false)` every 60 s).
    fn entities_think(&self, _now: f32) {}

    fn map_globals(&self, e: *mut edict_t, spawnflags: i32) -> Option<MapFlags> {
        entities::map_globals(e, spawnflags)
    }

    /// 2.1's `OnGround` (FUN_10066190) makes the weapon `MOVETYPE_FLY` with `effects |= 2`.
    fn ground_weapon_lands_flying(&self) -> bool {
        true
    }

    fn give_ground_weapon(&self, g: &mut GroundWeapon, cx: &mut Cx, p: &mut Player, pcx: &mut Cx) {
        entities::give_ground_weapon(g, cx, p, pcx);
    }

    fn outgoing_message(&self, m: Msg, w: MsgWriter, send: &mut dyn FnMut(MsgWriter)) {
        protocol::outgoing_message(m, w, send);
    }

    fn powerups(&self) -> &'static PowerupRules {
        &powerup::POWERUPS
    }

    fn powerup_kind(&self, key: i32, spawnflags: i32) -> (u16, bool) {
        powerup::kind(key, spawnflags)
    }

    fn armor_powerup(&self, _p: &mut Player, cx: &mut Cx, armor: f32) {
        powerup::armor(cx, armor)
    }

    /// `GoSlow` (0x10055220) writes `pev->fuser1` directly; 2.1 has no `InterpolateSlowMotion`.
    fn time_scale_eases(&self) -> bool {
        false
    }

    /// 2.1 has no `TSMBlur` message.
    fn motion_blur(&self) -> bool {
        false
    }

    /// 2.1's `EMIT_SOUND_DYN` (0x10036300) is the SDK's: no time-scaled pitch.
    fn sound_pitch(&self, pitch: i32, time_scale: f32, s: f32) -> i32 {
        let _ = (time_scale, s);
        pitch
    }

    fn stunts(&self) -> &'static StuntRules {
        &stunt::STUNTS
    }

    fn stunt_clock(&self) -> &'static dyn StuntClock {
        &stunt::ABSOLUTE
    }

    fn can_dive(&self, p: &Player, armor: f32, dive: Dive) -> bool {
        stunt::can_dive(p, armor, dive)
    }

    fn bullet_time_allowed(&self, time_scale: f32) -> bool {
        stunt::bullet_time_allowed(time_scale)
    }

    /// `ShootBulletsPlayer` (0x1003ff56, 0x10040319) and the bullet update (0x1003fa21) trace
    /// with the plain `UTIL_TraceLine` (`FUN_10067850`, ignoring the shooter): no 32-unit growth
    /// of the players' boxes, so hitboxes outside a player's box can't be hit.
    fn bullet_trace(&self, start: Vec3, end: Vec3, shooter: *mut edict_t) -> TraceResult {
        engine().trace_line(start, end, DONT_IGNORE_MONSTERS, shooter)
    }

    fn bullet_damage(&self, ammo: u8, weapon: u8, damage: f32, falloff: f32, simulated: bool) -> f32 {
        damage::bullet_damage(ammo, weapon, damage, falloff, simulated)
    }

    fn bullet_range_index(&self, scaled: shared::pm::Ext) -> i32 {
        damage::range_index(scaled)
    }

    fn bullet_damage_multiplier(&self, acc: &mut f32, cheat: Option<f32>) -> f32 {
        damage::accumulate(acc, cheat, engine().random_float(0.0, 0.05))
    }

    fn pierce_step(&self, left: f32) -> f32 {
        damage::pierce_step(left)
    }

    /// `ShootBulletsPlayer` (0x1003fe17) reads a global (`DAT_100aca80`) that nothing writes:
    /// only the shooter's slow motion fires simulated bullets, and they always step every 0.1 s.
    fn weapon_realbullet(&self, _realbullet: i32) -> i32 {
        0
    }

    /// 2.1 shipped only `mp.dll`, whose update stores the factor as a `float` in every build.
    fn bullet_velocity_mult(&self, _arith: shared::pm::Arith, slowmul: f32, bullet_time: bool) -> shared::pm::Ext {
        shared::pm::Ext::from_f32(damage::bullet_velocity_mult(slowmul, bullet_time))
    }

    fn player_hit(&self, hit: &PlayerHit) -> HitDamage {
        damage::player_hit(hit)
    }

    fn armor_damage(&self, armor: &mut f32, amount: f32, hitgroup: i32, bits: i32) -> f32 {
        damage::armor_damage(armor, amount, hitgroup, bits)
    }

    fn damage_punch(&self) -> f32 {
        damage::DAMAGE_PUNCH
    }

    fn grenade_explosion(&self) -> (f32, f32, f32) {
        explode::GRENADE_EXPLOSION
    }

    /// `TSExplosion` (0x10047aa0) has no `WorldGun` knock-around.
    fn explosion_pushes_world_guns(&self) -> bool {
        false
    }

    fn explosion_aftermath(&self, org: Vec3, _inflictor: *mut edict_t, attacker: *mut edict_t) {
        explode::aftermath(org, attacker);
    }

    fn thrown_blade(&self, btype: i32) -> ThrownBlade {
        blades::thrown_blade(btype)
    }

    fn thrown_blade_damage(&self, btype: i32, from: Vec3, at: Vec3) -> f32 {
        blades::thrown_blade_damage(btype, blades::flown(from, at))
    }

    fn melee_block(
        &self,
        kind: MeleeKind,
        attacker: *mut edict_t,
        victim: *mut edict_t,
        damage: f32,
        atk_block: f32,
    ) -> Option<MeleeResult> {
        melee::melee_block(kind, attacker, victim, damage, atk_block)
    }

    fn jump_kick_hit(&self, p: &mut Player, cx: &mut Cx, _strength: f32) {
        melee::jump_kick_hit(p, melee::player_t(cx.pev().fuser1));
    }

    fn backflip_kick(&self, p: &mut Player, cx: &mut Cx) {
        melee::backflip_kick(p, ts21_gameplay::shared::gun::slow_div(cx.pev().fuser1), cx.time());
    }

    /// 2.1's jump kick (`FUN_1005cd00`) leaves the kung-fu block (an end time) alone, and its
    /// `CloseCombatAttack` lets the attacker block with it.
    fn jump_kick_attacker_block(&self, gun: &mut Gun) -> f32 {
        gun.block_kungfu
    }

    fn melee_disarm(&self, damage: f32, victim_is_the_one: bool) -> Option<f32> {
        melee::melee_disarm(damage, victim_is_the_one)
    }

    /// `FUN_1005c680` has no `dmg <= 0` test: a spent delayed hit still runs the team check, the
    /// block (`FUN_100439e0` / the blade blocks) and the hit sound (0x1005c9f0..0x1005ca99).
    fn zero_delayed_hit_reaches_player(&self) -> bool {
        true
    }
}

/// The plugin on the server: TS 2.1's shared hooks and its server hooks.
pub static PLUGIN: ServerPlugin = ServerPlugin { pm: &TS21, gun: &TS21, server: &Ts21Server };

/// What a client that can't play 2.1 is told (port addition, issue #100: TS 3.0's own client
/// predicts 3.0's movement and weapons, so it can't play this ruleset).
pub const REFUSED_CLIENT: &str = "Invalid Version: Running 2.1 mode. Upgrade to the rust port at tsmod.org";

/// `ClientConnect` in 2.1 mode: only the port's client may join; bots and HLTV always may.
fn refuse_client(client: ConnectingClient) -> Option<&'static str> {
    matches!(client, ConnectingClient::Other).then_some(REFUSED_CLIENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ts30_server::Ts30Server as CORE;

    #[test]
    fn descriptions_mark_the_mode_as_2_1() {
        let v = env!("CARGO_PKG_VERSION");
        let d = |m| Ts21Server.game_description(m).to_str().unwrap().to_owned();
        assert_eq!(d(DescriptionMode::Dm), format!("The Specialists v{v} (DM-21)"));
        assert_eq!(d(DescriptionMode::TheOne), format!("The Specialists v{v} (The One-21)"));
        assert_eq!(CORE.game_description(DescriptionMode::Dm).to_str().unwrap(), format!("The Specialists v{v} (DM)"));
    }

    #[test]
    fn only_the_port_client_joins() {
        use ConnectingClient::*;
        let s: &dyn ServerRules = &Ts21Server;
        assert_eq!(s.refuse_client(Other), Some("Invalid Version: Running 2.1 mode. Upgrade to the rust port at tsmod.org"));
        for ok in [Bot, Hltv, Port("3.0.2"), Port("0.0.1")] {
            assert_eq!(s.refuse_client(ok), None, "{ok:?}");
        }
        assert!(REFUSED_CLIENT.len() < 128, "fits the engines' reject buffer");
        // From the engine's view: TS 3.0's client sends no `_tsrs`; a bot with none still joins.
        assert_eq!(s.refuse_client(ConnectingClient::classify(false, "", "", "")), Some(REFUSED_CLIENT));
        assert_eq!(s.refuse_client(ConnectingClient::classify(true, "", "", "")), None);
        assert_eq!(s.refuse_client(ConnectingClient::classify(false, "", "1", "")), None);
    }

    #[test]
    fn overrides_without_the_engine() {
        let s: &dyn ServerRules = &Ts21Server;
        assert_eq!(s.cvars(), cvars::CVARS);
        assert_eq!(s.put_in_server_cash(), 0, "2.1 sets the cash in InitHUD instead");
        assert!(!s.server_command("addbot"), "only the polled cvars are 2.1's");
    }

    /// The 3.0 defaults the 2.1 plugin overrides, unchanged by the hooks.
    #[test]
    fn core_keeps_3_0s_entities() {
        let core: &dyn ServerRules = &CORE;
        assert!(core.spawns_entity(c"ts_flag"));
        assert!(core.spawns_entity(c"weapon_shotgun"));
        assert_eq!(core.broken_respawn_delay(0.0), Some(30.0));
        assert_eq!(core.broken_respawn_delay(12.0), Some(12.0));
        assert!(core.breakable_respawn_guarded());
        assert!(!core.ground_weapon_lands_flying());
        assert_eq!(core.map_globals(std::ptr::null_mut(), 32 | 64), Some(MapFlags::from_spawnflags(32 | 64)));
        assert_eq!(MapFlags::from_spawnflags(32 | 64), MapFlags { round_timer: false, superjump: true, weapons: 0x1f ^ 0x60 });
    }

    #[test]
    fn plugin_overrides() {
        let p: &dyn ServerRules = &Ts21Server;
        assert!(!p.spawns_entity(c"ts_flag"));
        assert!(p.spawns_entity(c"func_breakable"));
        assert_eq!(p.broken_respawn_delay(0.0), None);
        assert_eq!(p.broken_respawn_delay(12.0), None);
        assert!(!p.breakable_respawn_guarded());
        assert!(p.ground_weapon_lands_flying());
        assert!(!p.dead_grenades_become_powerups(), "a corpse drops one M61");
        assert!(CORE.dead_grenades_become_powerups(), "3.0's m_mode < 2: power-ups");
        assert!(!p.drop_lifetime_and_glow() && CORE.drop_lifetime_and_glow());
        assert_eq!([p.stack_drop_count(3, false), p.stack_drop_count(3, true), p.stack_drop_count(1, false)], [1, 1, 1]);
        assert_eq!([CORE.stack_drop_count(3, false), CORE.stack_drop_count(3, true), CORE.stack_drop_count(1, false)], [3, 1, 1]);
        assert!(!p.world_gun_fall_impact() && CORE.world_gun_fall_impact());
        assert!(p.ammo_pack_stays_when_nothing_fits() && !CORE.ammo_pack_stays_when_nothing_fits());
    }

    /// The 2.1 hooks differ where 2.1 does, and the 3.0 plugin keeps 3.0's.
    #[test]
    fn hooks_follow_the_plugin() {
        let core: &dyn ServerRules = &ts30_server::Ts30Server;
        let v21: &dyn ServerRules = &Ts21Server;
        assert!(core.break_debris_from_hit() && !v21.break_debris_from_hit());
        assert_eq!(core.objectives(), &ts30_server::entities::OBJECTIVES);
        assert_eq!(v21.objectives(), &rules::OBJECTIVES);
        assert_eq!(v21.anim_framerate(0.05), 0.0, "frozen below a tenth");
        assert_eq!(core.anim_framerate(0.05), 0.05);
        assert_eq!(v21.anim_framerate(1.0), 1.0);
        assert_eq!(v21.grenade_fuse_step(0.0, 0.05), 0.01, "floored at 0.1");
        assert_eq!(v21.grenade_fuse_step(0.0, 1.0), 0.1);
        assert_ne!(core.grenade_fuse_step(3e-6, 1.0), v21.grenade_fuse_step(3e-6, 1.0), "0.1f against the double 0.1");
        let (org, absmin, size) = ([1.5, 16_777_218.0, 0.0], [0.0, 16_777_216.0, 0.0], [0.0, 1.0, 0.0]);
        assert_eq!(core.breakable_attack_dir(org, absmin, size), [0.6, 0.8, 0.0], "3.0 rounds the centre: d = (1.5, 2, 0)");
        assert_ne!(v21.breakable_attack_dir(org, absmin, size), core.breakable_attack_dir(org, absmin, size));
        assert!(core.time_scale_eases() && !v21.time_scale_eases());
        assert!(core.motion_blur() && !v21.motion_blur());
        assert_eq!(core.sound_pitch(100, 0.5, 0.5), 75);
        assert_eq!(v21.sound_pitch(100, 0.5, 0.5), 100);
        assert_eq!(core.powerups().base(shared::ts::powerup::SLOWMO), 3);
        assert_eq!(v21.powerups().base(shared::ts::powerup::SLOWMO), 5);
        assert!(core.stunts().wall_dive && !v21.stunts().wall_dive);
        assert_eq!(core.stunt_clock().start(10.0, 1.0, 0.5), 0.5);
        assert_eq!(v21.stunt_clock().start(10.0, 1.0, 0.5), 10.5);
        assert!(core.bullet_time_allowed(0.99) && !core.bullet_time_allowed(0.995));
        assert!(v21.bullet_time_allowed(0.995) && !v21.bullet_time_allowed(1.0));
        assert!(!core.zero_delayed_hit_reaches_player() && v21.zero_delayed_hit_reaches_player());
    }
}
