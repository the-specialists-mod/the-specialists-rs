//! TS 2.1's server rules (issue #100 §3) and the plugin object `server-dll` registers. Hooks it
//! doesn't override keep TS 3.0's body.
//!
//! * [`rules`]: the mode from the map name, teams, `CTSGameRules::Think` and the round modes.
//! * [`scoring`]: `TSGetPointsForFrag` and `PlayerKilled`.
//! * [`cash`]: `CashBased`, the cash floor, kill cash and buying.
//! * [`players`]: spawn, join, death, respawn timing, overheal and The One.
//! * [`cvars`] and [`commands`]: 2.1's extra cvars, the polled cvars and the client commands.
//! * [`entities`]: the map entities (§3.9).
//! * [`protocol`]: 2.1 on TS 3.0's wire (§1.2).
//! * [`powerup`]: power-ups and slow motion (§3.7).
//! * [`stunt`]: the server stunts and their slot limits (§3.8, §3.5).
//! * [`damage`]: bullets and what a hit does to a player (§3.1).
//! * [`explode`], [`blades`]: explosions, shrapnel and thrown knives (§3.2).
//! * [`melee`]: blocks and disarms on the victim (§3.2; the attacks are the shared gun hooks).

#![deny(unsafe_code)]

pub mod blades;
pub mod cash;
pub mod commands;
pub mod cvars;
pub mod damage;
pub mod entities;
pub mod explode;
pub mod melee;
pub mod players;
pub mod powerup;
pub mod protocol;
pub mod rules;
pub mod scoring;
pub mod stunt;

use goldsrc::server::MsgWriter;
use goldsrc::server::{DONT_IGNORE_MONSTERS, engine};
use goldsrc::sys::{TraceResult, edict_t};
use shared::weapons::gun::{MeleeKind, MeleeResult};
use shared::weapons::spread::Vec3;
use std::ffi::CStr;
use ts_server::commands::Args;
use ts_server::damage::DamageInfo;
use ts_server::damage::glast::Snapshot;
use ts_server::entities::ts::ground::GroundWeapon;
use ts_server::entities::ts::misc::MapFlags;
use ts_server::gameplay::server::CvarDef;
use ts_server::gameplay::{ServerPlugin, ServerRules};
use ts_server::messages::Msg;
use ts_server::player::Player;
use ts_server::player::damage::{HitDamage, PlayerHit};
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
    fn cvars(&self) -> &'static [CvarDef] {
        cvars::CVARS
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

    fn default_team(&self, p: &mut Player, cx: &mut Cx) -> String {
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

    /// `RestartRound` (0x100445d0) forces it.
    fn respawn_entities(&self) {
        entities::respawn_entities(true);
    }

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

    fn bullet_range_index(&self, scaled: f32) -> i32 {
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

    fn bullet_velocity_mult(&self, slowmul: f32, bullet_time: bool) -> f32 {
        damage::bullet_velocity_mult(slowmul, bullet_time)
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

    fn thrown_blade_damage(&self, btype: i32, flown: f32) -> f32 {
        blades::thrown_blade_damage(btype, flown)
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

    fn melee_disarm(&self, damage: f32, victim_is_the_one: bool) -> Option<f32> {
        melee::melee_disarm(damage, victim_is_the_one)
    }
}

/// The plugin on the server: TS 2.1's shared hooks and its server hooks.
pub static PLUGIN: ServerPlugin = ServerPlugin { pm: &TS21, gun: &TS21, server: &Ts21Server };

#[cfg(test)]
mod tests {
    use super::*;
    use shared::gameplay::CORE;

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
    }

    /// The 2.1 hooks differ where 2.1 does, and the core plugin keeps 3.0's.
    #[test]
    fn hooks_follow_the_plugin() {
        let core: &dyn ServerRules = &shared::gameplay::CORE;
        let v21: &dyn ServerRules = &Ts21Server;
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
    }
}
