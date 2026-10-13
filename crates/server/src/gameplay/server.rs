//! [`ServerRules`]: the server-only hooks of a gameplay ruleset (rules, scoring, damage,
//! entities...). Bots aren't part of a ruleset: they are left to a plugin axis of their own.
//!
//! The server's own code resolves them from the active plugin ([`active`](super::active)), which
//! follows `ts_gameplay` once a frame; a switch resets the game first ([`super::reset`]).

use crate::commands::Args;
use crate::damage::DamageInfo;
use crate::damage::glast::Snapshot;
use crate::entities::ts::ground::GroundWeapon;
use crate::entities::ts::misc::MapFlags;
use crate::messages::Msg;
use crate::player::Player;
use crate::player::damage::{self as player_damage, HitDamage, PlayerHit};
use crate::rules::powerup::{self, PowerupRules};
use crate::rules::scoring::{self, Frag};
use crate::rules::state::GameRules;
use crate::rules::{cvars, frame, lifecycle, teams};
use crate::stunt::rules::{self as stunt, Dive, StuntClock, StuntRules};
use crate::weapons::{fire, melee, projectiles};
use crate::world::{Cx, Entity};
use goldsrc::server::MsgWriter;
use goldsrc::sys::{TraceResult, edict_t};
use shared::weapons::gun::{MeleeKind, MeleeResult};
use shared::weapons::spread::{self, Vec3};
use std::ffi::CStr;

/// A server cvar: (name, default, flags), as [`cvars::CVARS`] lists TS 3.0's.
pub type CvarDef = (&'static str, &'static str, i32);

/// Server rules. Every default body is TS 3.0's (`shared::gameplay::Core`); the server code calls
/// these where a ruleset can differ. A hook needing server state takes this crate's types, so
/// its 3.0 body can be the code it replaces.
pub trait ServerRules: Sync {
    /// The plugin has become active on a brand-new game: at every map start (after the rules
    /// are installed, before the map's entities spawn) and after a switch to it has reset the
    /// game ([`super::reset`]). The place to set up the plugin's own state.
    fn on_activate(&self) {}

    // ---- cvars and commands ----------------------------------------------------------------

    /// Server cvars the ruleset needs besides TS 3.0's ([`cvars::CVARS`]). They are registered
    /// when the plugin is ([`super::register`], at `GameDLLInit`), skipping any name that already
    /// exists, so the server has the union of every ruleset's cvars and each ruleset reads the
    /// ones it knows. 3.0: none.
    fn cvars(&self) -> &'static [CvarDef] {
        &[]
    }

    /// A TS server command (`globalslow`, `restartround`, `weaponrestriction`) before TS 3.0's
    /// handler runs it; true when the ruleset has handled it. The arguments are the engine's
    /// (`Cmd_Argv`). 3.0: false, the command runs.
    fn server_command(&self, _name: &str) -> bool {
        false
    }

    /// A player's client command (`ClientCommand` 0xad838) before TS 3.0's handlers (weapons,
    /// stunts, rules, bots): `Some(handled)` decides it (false prints "Unknown command"), `None`
    /// lets 3.0's code run.
    fn client_command(&self, _p: &mut Player, _cx: &mut Cx, _cmd: &str, _args: &Args) -> Option<bool> {
        None
    }

    // ---- game rules ------------------------------------------------------------------------

    /// `InstallGameRules` and the rules' constructors for a multiplayer map (`deathmatch` set):
    /// the game mode, teams and `CTSGameRules::Init` ([`GameRules::install_multiplayer`]). Runs
    /// inside the rules state's borrow, so it must not call code that reads the rules state.
    fn install(&self, g: &mut GameRules) {
        g.install_multiplayer();
    }

    /// `CTSGameRules::Think` (0x77204), from the multiplay or teamplay `Think` each frame.
    fn think(&self, now: f32) {
        frame::ts_think(now);
    }

    /// `CTSGameRules::SetPlayerTheOne` (0x7b078): makes a living, non-observing player The One.
    fn set_player_the_one(&self, p: &mut Player, cx: &mut Cx) {
        scoring::set_player_the_one_core(p, cx);
    }

    /// The team `CHalfLifeTeamplay::SetDefaultPlayerTeam` (0xe44ac) reads before it checks it
    /// against the team list ([`teams::default_team_core`]).
    fn default_team(&self, p: &mut Player, cx: &mut Cx) -> String {
        teams::default_team_core(p, cx)
    }

    // ---- players ---------------------------------------------------------------------------

    /// End of the rules part of `TSInit` at every spawn (`lifecycle::on_spawn`), before the
    /// team and the spawn point. 3.0: nothing.
    fn on_spawn(&self, _p: &mut Player, _cx: &mut Cx) {}

    /// The cash a player joins with (`ClientPutInServer`: `GiveCash(atoi(initial_cash.string))`).
    /// 3.0: [`cvars::INITIAL_CASH`].
    fn put_in_server_cash(&self) -> i32 {
        cvars::INITIAL_CASH
    }

    /// Start of `CBasePlayer::StartObserver` (0x99634), before the cash goes to the client.
    /// 3.0: nothing.
    fn on_start_observer(&self, _p: &mut Player, _cx: &mut Cx) {}

    /// The observer part of `CBasePlayer::PreThink` (0x908cc), run every frame while
    /// `pev->iuser1` is set: the respawn countdown and the join deadline, then
    /// `Observer_HandleButtons` and `Observer_Idle`. PreThink zeroes `impulse` and returns after
    /// it, so a ruleset that respawns the observer here (`lifecycle::stop_observer`) just returns.
    /// 3.0: [`lifecycle::observer_think_core`] (3.0 respawns with the `respawn` command instead).
    fn observer_think(&self, p: &mut Player, cx: &mut Cx, now: f32) {
        lifecycle::observer_think_core(p, cx, now);
    }

    /// End of `CTSGameRules::InitHUD` (0x79134), once a joiner is observing with its respawn
    /// time set. 3.0: nothing.
    fn on_init_hud(&self, _p: &mut Player, _cx: &mut Cx) {}

    /// `SetPlayerRespawnTime` (0x75ff4): when the player may respawn (`respawn_at`, -1 for not
    /// this round), the join deadline and the award.
    fn set_respawn_time(&self, p: &mut Player, cx: &mut Cx, secs: f32) {
        lifecycle::set_respawn_time_core(p, cx, secs);
    }

    /// `CBasePlayer::Killed` (0x8ed5c) right after the respawn time is set. 3.0: nothing.
    fn on_killed(&self, _p: &mut Player, _cx: &mut Cx) {}

    /// The overheal and The One's regeneration in `CBasePlayer::PreThink` (0x908cc, `+0x73c`).
    fn health_regen(&self, p: &mut Player, cx: &mut Cx, now: f32) {
        lifecycle::health_regen_core(p, cx, now);
    }

    // ---- scoring and cash ------------------------------------------------------------------

    /// `TSGetPointsForFrag` (0x89140): the points a killing player scores for `frag`, which also
    /// reports whether the kill was a slide or a stunt.
    fn points_for_frag(&self, frag: &Frag<'_>, sliding: &mut bool, diving: &mut bool) -> i32 {
        scoring::points_for_frag(frag, sliding, diving)
    }

    /// The body of `CHalfLifeMultiplay::PlayerKilled` (0x895e4) for victim `p`, once the killer
    /// can be reached (`scoring::player_killed` defers it until then): points, sprees, awards,
    /// cash, messages, `DeathMsg`, logs and `ScoreInfo`.
    fn player_killed(&self, p: &mut Player, cx: &mut Cx, d: &DamageInfo, g: Snapshot) {
        scoring::player_killed_core(p, cx, d, g);
    }

    /// `CashBased()` (0x76abc): whether buying costs money. 3.0: hard-wired off.
    fn cash_based(&self) -> bool {
        false
    }

    /// `CBasePlayer::BuyArmament` (from `PlayerSpawn` when the map has no `game_player_equip`):
    /// the loadout the client sent (`tkw`/`tki`/`tka`).
    fn buy_armament(&self, p: &mut Player, cx: &mut Cx) {
        crate::weapons::buy_armament(p, cx);
    }

    // ---- map entities -------------------------------------------------------------------------

    /// Whether an entity of `class` may spawn (`DispatchSpawn`, for the map's entities and the
    /// game's own): a refused one fails its spawn (-1, the engine frees it) and its class parses
    /// no keyvalue. TS 3.0 has an export for every class the port links: all spawn.
    fn spawns_entity(&self, _class: &CStr) -> bool {
        true
    }

    /// A keyvalue for an entity's class (`DispatchKeyValue`'s class part, after the entvars
    /// table), before the class's own `KeyValue`. `None` lets the class parse it (TS 3.0);
    /// `Some(handled)` means the plugin parsed or refused it, and is `fHandled`.
    fn entity_key_value(&self, _ent: &mut dyn Entity, _cx: &mut Cx, _key: &CStr, _value: &CStr) -> Option<bool> {
        None
    }

    /// An entity's `Touch` (`DispatchTouch` and the game's own touches): true when the plugin ran
    /// it instead of the class. TS 3.0: the class's.
    fn entity_touch(&self, _ent: &mut dyn Entity, _cx: &mut Cx, _other: *mut edict_t) -> bool {
        false
    }

    /// How long a broken `func_breakable` or breakable door waits before it rebuilds itself
    /// (`CBreakable::Die` 0x752e0, `CBaseDoor::TraceAttack` 0xb57ac), given its `respawntime` key (≥ 0); `None`
    /// leaves it broken, with no think, until `RespawnEntities` restores it. TS 3.0: the key, 0
    /// meaning 30 s on the `info_breakable` helper (`RespawnThink` 0x75da0).
    fn broken_respawn_delay(&self, respawn_time: f32) -> Option<f32> {
        Some(if respawn_time == 0.0 { 30.0 } else { respawn_time })
    }

    /// Whether `BreakableRespawn` (`CBreakable` 0x74e84, `CBaseDoor`/`CRotDoor` 0xb5604/0xb7838)
    /// waits while a client is inside the box (retrying in 5 s) and fires a breakable's
    /// `respawntarget` (spawnflag 8). TS 3.0: yes. False restores the entity at once and fires
    /// nothing.
    fn breakable_respawn_guarded(&self) -> bool {
        true
    }

    /// `CTSGameRules::RespawnEntities` as `RestartRound` (0x77920) calls it. TS 3.0's
    /// ([`crate::entities::ts::respawn_entities`]).
    fn respawn_entities(&self) {
        crate::entities::ts::respawn_entities();
    }

    /// The map-entity upkeep of `CTSGameRules::Think` (0x77204) in deathmatch and teamplay, after The One's
    /// check (`now` is the game time). TS 3.0 has none.
    fn entities_think(&self, _now: f32) {}

    /// What a `ts_mapglobals` (`e`) sets with its spawnflags (`CTSMapGlobals::Spawn` 0x72534),
    /// or `None` to set nothing. TS 3.0: [`MapFlags::from_spawnflags`].
    fn map_globals(&self, _e: *mut edict_t, spawnflags: i32) -> Option<MapFlags> {
        Some(MapFlags::from_spawnflags(spawnflags))
    }

    /// Whether a landing `ts_groundweapon` (`OnGround`) also becomes `MOVETYPE_FLY` with
    /// `effects |= 2`, as `ts_groundammo`'s does. TS 3.0: no (`CTSGroundWeapon::OnGround`
    /// 0xa226c).
    fn ground_weapon_lands_flying(&self) -> bool {
        false
    }

    /// `CTSGroundWeapon::GetWeapon` (0xa2328) for a player, past its `IsPlayer` test (the
    /// ground weapon `g` is `cx`, the player `p` is `pcx`). TS 3.0's
    /// ([`GroundWeapon::give`]).
    fn give_ground_weapon(&self, g: &mut GroundWeapon, cx: &mut Cx, p: &mut Player, pcx: &mut Cx) {
        g.give(cx, p, pcx);
    }

    // ---- protocol -----------------------------------------------------------------------------

    /// A user message on its way out (`msg`, written into `w`): hands `send` what goes on the
    /// wire in its place. TS 3.0 sends it as it is.
    fn outgoing_message(&self, _msg: Msg, w: MsgWriter, send: &mut dyn FnMut(MsgWriter)) {
        send(w);
    }

    // ---- power-ups and slow motion ----------------------------------------------------------

    /// The power-up values: durations, the instant types, the pickup thresholds, the slow-motion
    /// amounts (`SetupPowerUp` 0x98014, `WaitTouch`, `UpdatePowerUpState` 0x95748).
    fn powerups(&self) -> &'static PowerupRules {
        &powerup::CORE
    }

    /// `SetupPowerUp`'s type for a `ts_powerup` with the type `key` (`pwuptype`, < 1 random)
    /// and `spawnflags`: the type, and whether it is drawn again on each respawn.
    fn powerup_kind(&self, key: i32, spawnflags: i32) -> (u16, bool) {
        crate::entities::ts::powerup::kind_for(key, spawnflags)
    }

    /// `PlayerAddPowerUp` (0x97bc8) for an armour power-up worth `armor` (100 unless a ruleset
    /// makes it otherwise): `TakeArmor(100)`, which costs slots.
    fn armor_powerup(&self, p: &mut Player, cx: &mut Cx, armor: f32) {
        let _ = armor;
        crate::weapons::take_armor(p, cx, 100);
    }

    /// `GoSlow` (0x929d0) sets a target time scale that `InterpolateSlowMotion` (0x9910c) eases
    /// `pev->fuser1` toward each PreThink. False: `GoSlow` writes `fuser1` at once.
    fn time_scale_eases(&self) -> bool {
        true
    }

    /// The ruleset sends `TSMBlur` (stunts, power-ups, kills, kung fu), and draws the random
    /// numbers deciding it.
    fn motion_blur(&self) -> bool {
        true
    }

    /// `EMIT_SOUND_DYN`'s pitch (0xe0aa7) for an entity whose time scale is `time_scale`, with
    /// the slow factor `s` ([`slow_pitch`](crate::entities::stock::explode::slow_pitch)).
    fn sound_pitch(&self, pitch: i32, time_scale: f32, s: f32) -> i32 {
        crate::entities::stock::explode::slow_pitch(pitch, time_scale, s)
    }

    // ---- stunts (CTSStunt) ------------------------------------------------------------------

    /// The stunt values (recoveries, keys, which moves exist).
    fn stunts(&self) -> &'static StuntRules {
        &stunt::CORE
    }

    /// How the stunt timers keep time.
    fn stunt_clock(&self) -> &'static dyn StuntClock {
        &stunt::RELATIVE
    }

    /// `CanDiveHigh/Medium/Low`: the player can do a stunt of this height. `armor` is
    /// `pev->armorvalue`.
    fn can_dive(&self, p: &Player, armor: f32, dive: Dive) -> bool {
        let _ = armor;
        stunt::can_dive(p, dive)
    }

    /// `GoBulletTime` (0x9a5b4): bullet time is possible at the time scale `time_scale`
    /// (`pev->fuser1`): `GiveSlowMul() <= 0.99`.
    fn bullet_time_allowed(&self, time_scale: f32) -> bool {
        time_scale.max(0.01) <= 0.99
    }

    // ---- bullets (`weapons::fire`, `weapons::bullets`) ------------------------------------------

    /// The trace a bullet flies along (`ShootBulletsPlayer` 0x6f69c and its penetration,
    /// `CBulletManager::UpdateBullet` 0x6ef5c): `UTIL_TraceLine_Expanded` (0x6f4f4), which grows
    /// every player's box by 32 units, ignoring the shooter.
    fn bullet_trace(&self, start: Vec3, end: Vec3, shooter: *mut edict_t) -> TraceResult {
        fire::trace_expanded(start, end, shooter)
    }

    /// A bullet's range-falloff index from `scaled`, the distance in tenths of the ammo's range
    /// unit (`dist / (unit·10)·10`, or `dist·0.0033333334·10` after a penetration). The port's
    /// 3.0 rounds it to the nearest (`spread::range_index`); issue #80 has TS 3.0 truncating
    /// too, and fixing that here fixes every bullet path.
    fn bullet_range_index(&self, scaled: f32) -> i32 {
        scaled.round_ties_even() as i32
    }

    /// A bullet's damage where it hits, before [`bullet_damage_multiplier`](Self::bullet_damage_multiplier):
    /// the gun's `damage` (`WeaponInfo::damage` of `weapon`, firing ammo type `ammo`) times the
    /// range falloff `falloff`, for a hitscan pellet or a `simulated` bullet's first hit (a
    /// penetration's carried damage is scaled by the falloff alone). TS 3.0: `falloff × damage`.
    fn bullet_damage(&self, ammo: u8, weapon: u8, damage: f32, falloff: f32, simulated: bool) -> f32 {
        let _ = (ammo, weapon, simulated);
        falloff * damage
    }

    /// The multiplier of a bullet's damage where it hits. `acc` starts at 1.0 for each
    /// `ShootBulletsPlayer` call and each simulated-bullet update and persists over the
    /// pellets of the call; `cheat` is `damagemult` when it applies (`sv_cheats`). The value
    /// also scales the damage a penetration carries on. TS 3.0: `cheat` or 1
    /// (`assets::damage_adjust`).
    fn bullet_damage_multiplier(&self, acc: &mut f32, cheat: Option<f32>) -> f32 {
        let _ = acc;
        cheat.unwrap_or(1.0)
    }

    /// How far past the hit point `ShootBulletsPlayer` (0x6f69c) looks for the far side of a
    /// wall, from the damage `left` to carry through: `min(2·left, 48)`. (The simulated bullets
    /// use `min(left, 48)` in every ruleset.)
    fn pierce_step(&self, left: f32) -> f32 {
        (left + left).min(48.0)
    }

    /// The weapons' `realbullet` (`TSGameRules + 0x578`, `rules::state::realbullet`): non-zero
    /// fires simulated bullets for everyone (2+: stepped every frame). The shooter's slow motion
    /// fires them too, whatever this says.
    fn weapon_realbullet(&self, realbullet: i32) -> i32 {
        realbullet
    }

    /// `GetBulletVelocityMult` (`CBulletManager::UpdateBullet` 0x6ef5c): a simulated bullet's
    /// speed factor from the shooter's time scale (`slowmul`: its slow-motion "others" amount,
    /// else its own time scale) and bullet time.
    fn bullet_velocity_mult(&self, slowmul: f32, bullet_time: bool) -> f32 {
        spread::bullet_velocity_mult(slowmul, bullet_time)
    }

    // ---- the victim (`player::damage`, `rules::lifecycle`) --------------------------------------

    /// `CBasePlayer::TraceAttack` (0x901cc): the damage a traced hit adds to the multi-damage
    /// after the stunt and dodge ×0.7 and the hitgroup multipliers, and whether it is a
    /// headshot (the hit sound; `glast_headshot` when [`HitDamage::record_headshot`]).
    fn player_hit(&self, hit: &PlayerHit) -> HitDamage {
        player_damage::hit_damage(hit)
    }

    /// `CBasePlayer::TakeDamage` (0x8fa94): the damage that reaches `CBaseMonster::TakeDamage`
    /// after armour, truncated to an int (`player::damage::after_kevlar`), changing `armor` if
    /// the ruleset uses it up. TS 3.0's kevlar absorbs `armor_absorb` and is never depleted.
    fn armor_damage(&self, armor: &mut f32, amount: f32, hitgroup: i32, bits: i32) -> f32 {
        player_damage::after_kevlar(amount, *armor, hitgroup, bits, crate::rules::state::armor_absorb())
    }

    /// `CBasePlayer::TakeDamage` (0x8fa94): how far a hit the player takes kicks its view up
    /// (`punchangle.x -= ...`). TS 3.0: none.
    fn damage_punch(&self) -> f32 {
        0.0
    }

    // ---- melee (`weapons::melee`) -----------------------------------------------------------------

    /// The victim's block of a melee hit (`VictimKungFuHold`, `VictimKnifeHold`,
    /// `VictimKatanaHold`; `weapons::melee::hold`): `None` if `victim` doesn't block, else what the
    /// attacker gets ([`MeleeResult::Blocked`]), the victim's side applied here. `damage` is the
    /// hit's; `atk_block` the attacker's own kung-fu block window (the attacker is the player
    /// being dispatched). Called for every hit on a player, the delayed ones too.
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

    /// A jump kick (`CTSStunt::CloseAttack` 0x9c630) has hit; `strength` is its damage before
    /// Kung Fu's doubling. TS 3.0: `m_flNextAttack` 0.35 if it had run out, and
    /// `UpdateMeleeBar(strength × 1.5)` (`stunt::moves::jump_kick_hit_core`).
    fn jump_kick_hit(&self, p: &mut Player, cx: &mut Cx, strength: f32) {
        crate::stunt::moves::jump_kick_hit_core(p, cx, strength);
    }

    /// The backflip kick (`CTSStunt::GoWheel` backward): TS 3.0 charges 22.5 kung-fu power and
    /// arms a delayed hit of 15 (30 with Kung Fu) (`stunt::moves::backflip_kick_core`).
    fn backflip_kick(&self, p: &mut Player, cx: &mut Cx) {
        crate::stunt::moves::backflip_kick_core(p, cx);
    }

    /// A melee hit that may disarm (`MeleeHit::disarm`, and every delayed kung-fu hit): the
    /// damage it adds to the victim's disarm count (`DropWeapon(-2, dmg)`, dropped at a
    /// per-weapon threshold), or `None` for no disarm. TS 3.0: the damage, whoever the victim.
    fn melee_disarm(&self, damage: f32, victim_is_the_one: bool) -> Option<f32> {
        let _ = victim_is_the_one;
        Some(damage)
    }

    // ---- explosions and thrown blades ------------------------------------------------------------

    /// `CM61Grenade::WaitThink` (0x7cacc): the grenade's `TSExplosion` damage, full-damage
    /// radius and maximum radius: 110, 300, 1024.
    fn grenade_explosion(&self) -> (f32, f32, f32) {
        (110.0, 300.0, 1024.0)
    }

    /// `TSExplosion` (0x7d01c): whether the blast knocks the dropped guns (`WorldGun`) in its
    /// sphere away. TS 3.0: yes.
    fn explosion_pushes_world_guns(&self) -> bool {
        true
    }

    /// The end of `TSExplosion` (0x7d964), after the damage: `org` is the blast point (the
    /// origin, 1 unit up), `attacker` the attacker or else the inflictor. TS 3.0: the
    /// "spectacular" slow-motion point (0.25 for 1 s).
    fn explosion_aftermath(&self, org: Vec3, inflictor: *mut edict_t, attacker: *mut edict_t) {
        let _ = (inflictor, attacker);
        crate::entities::stock::explode::explosion_slow_point(org);
    }

    /// A thrown blade of type `btype` (6 combat knife, 7 seal knife, 8 katana): its speed
    /// (`KnifeFire` 0x84044 / `KatanaThrow` 0x83b68), its gravity (`CTSKnife::Spawn` 0x7db54) and
    /// its damage bits (`CTSKnife::BounceTouch` 0x7decc and `WaitThink` 0x7ea30).
    fn thrown_blade(&self, btype: i32) -> projectiles::ThrownBlade {
        projectiles::thrown_blade(btype)
    }

    /// `CTSKnife::BounceTouch` (0x7decc): what a thrown blade deals the player it hits, `flown`
    /// units from where it was thrown. TS 3.0: 150 katana, 100 combat knife, 50 seal knife.
    fn thrown_blade_damage(&self, btype: i32, flown: f32) -> f32 {
        let _ = flown;
        projectiles::thrown_blade_damage(btype)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::damage::{bits, hitgroup};
    use crate::rules::cvars;
    use shared::gameplay::CORE;

    /// The core plugin's hooks that need no engine keep TS 3.0's values.
    #[test]
    fn core_hooks_are_ts_3_0() {
        let core: &dyn ServerRules = &shared::gameplay::CORE;
        assert!(core.cvars().is_empty(), "3.0 registers only its own cvars");
        assert!(!core.cash_based(), "3.0's CashBased is hard-wired off");
        assert_eq!(core.put_in_server_cash(), cvars::INITIAL_CASH);
        assert!(!core.server_command("globalslow"), "3.0 runs its server commands");
        assert!(!core.server_command("restartround"));
        assert!(!core.server_command("weaponrestriction"));
    }

    fn hit(damage: f32, bits: i32, hitgroup: i32) -> PlayerHit {
        PlayerHit { damage, bits, hitgroup, stunting: false, dodging: false }
    }

    /// The damage hooks' defaults are TS 3.0's.
    #[test]
    fn damage_defaults_are_3_0() {
        let r: &dyn ServerRules = &CORE;
        assert_eq!(r.bullet_range_index(9.5), 10, "rounded");
        assert_eq!(r.bullet_range_index(9.4), 9);
        let mut acc = 1.0;
        assert_eq!(r.bullet_damage_multiplier(&mut acc, None), 1.0);
        assert_eq!(r.bullet_damage_multiplier(&mut acc, None), 1.0, "nothing accumulates");
        assert_eq!(r.bullet_damage_multiplier(&mut acc, Some(3.0)), 3.0);
        assert_eq!(r.bullet_damage(1, 5, 33.0, 0.5, false), 16.5);
        assert_eq!(r.pierce_step(10.0), 20.0);
        assert_eq!(r.pierce_step(30.0), 48.0);
        assert_eq!(r.weapon_realbullet(2), 2);
        assert_eq!(r.bullet_velocity_mult(0.35, true), spread::bullet_velocity_mult(0.35, true));
        assert_eq!(r.damage_punch(), 0.0);
        assert_eq!(r.grenade_explosion(), (110.0, 300.0, 1024.0));
        assert!(r.explosion_pushes_world_guns());
        assert_eq!(r.thrown_blade(6), projectiles::ThrownBlade { speed: 1900.0, gravity: 0.75, bits: 0x0800_1000 });
        assert_eq!(r.thrown_blade(7), projectiles::ThrownBlade { speed: 1900.0, gravity: 0.3, bits: 0x0800_1000 });
        assert_eq!(r.thrown_blade(8), projectiles::ThrownBlade { speed: 1000.0, gravity: 0.85, bits: 0x0800_1000 });
        assert_eq!([6, 7, 8].map(|t| r.thrown_blade_damage(t, 2000.0)), [100.0, 50.0, 150.0]);
    }

    #[test]
    fn hit_defaults_are_3_0() {
        let r: &dyn ServerRules = &CORE;
        let h = r.player_hit(&hit(30.0, bits::BULLET, hitgroup::HEAD));
        assert_eq!(h, HitDamage { amount: 60.0, headshot: true, record_headshot: true });
        assert_eq!(r.player_hit(&hit(10.0, 0x0200_1002, hitgroup::HEAD)).amount, 15.0, "shotgun ×1.5");
        assert!(r.player_hit(&hit(100.0, 0x0800_1000, hitgroup::HEAD)).headshot, "thrown");
        assert_eq!(r.player_hit(&hit(30.0, bits::BULLET, hitgroup::RIGHTLEG)).amount, 15.0);
        let mut d = hit(100.0, bits::BULLET, hitgroup::CHEST);
        d.dodging = true;
        assert!((r.player_hit(&d).amount - 70.0).abs() < 1e-4, "dodge ×0.7 for bullets");
        d.bits = bits::CLUB;
        assert_eq!(r.player_hit(&d).amount, 100.0, "not for melee");
        d.stunting = true;
        assert!((r.player_hit(&d).amount - 70.0).abs() < 1e-4);
    }
}
