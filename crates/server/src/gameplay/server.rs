//! [`ServerRules`]: the server-only hooks of a gameplay ruleset (rules, scoring, damage,
//! entities...). Bots aren't part of a ruleset: they are left to a plugin axis of their own.
//!
//! The server's own code resolves them from the active plugin ([`active`](super::active)), which
//! `ts_gameplay` selects at each map start ([`super::map_select`]).

use crate::commands::Args;
use crate::damage::DamageInfo;
use crate::damage::glast::Snapshot;
use crate::entities::ts::ground::GroundWeapon;
use crate::entities::ts::misc::MapFlags;
use crate::entities::ts::{ctb, objective};
use crate::messages::Msg;
use crate::player::Player;
use crate::player::damage::{HitDamage, PlayerHit};
use crate::rules;
use crate::rules::powerup::PowerupRules;
use crate::rules::scoring::Frag;
use crate::rules::state::GameRules;
use crate::stunt::rules::{Dive, StuntClock, StuntRules};
use crate::weapons::projectiles;
use crate::world::{Cx, Entity};
use goldsrc::server::MsgWriter;
use goldsrc::sys::{TraceResult, edict_t, entvars_t};
use shared::pm::{Arith, Ext};
use shared::weapons::gun::{MeleeKind, MeleeResult};
use shared::weapons::spread::Vec3;
use shared::weapons::{Gun, Status, WeaponInfo};
use std::ffi::CStr;

/// A server cvar: (name, default, flags), as [`cvars::CVARS`](crate::rules::cvars::CVARS) lists TS 3.0's.
pub type CvarDef = (&'static str, &'static str, i32);

/// Who is connecting, as `ClientConnect` can tell (port addition, [`ServerRules::refuse_client`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectingClient<'a> {
    /// A fake client (`FL_FAKECLIENT`, or a `*bot` userinfo key): a bot plugin's player.
    Bot,
    /// An HLTV proxy (`*hltv`, FWGS `hltv`).
    Hltv,
    /// The port's client, with the version it sent (`shared::protocol::port::USERINFO_KEY`).
    Port(&'a str),
    /// Any other client: TS 3.0's own, or another without the port's key.
    Other,
}

impl<'a> ConnectingClient<'a> {
    /// Classifies a connection from its edict's `FL_FAKECLIENT` and its userinfo values: `*bot`,
    /// `*hltv` (ReHLDS, HLDS) or `hltv` (FWGS), and the port's version key.
    pub fn classify(fake: bool, bot: &str, hltv: &str, port_version: &'a str) -> Self {
        let set = |v: &str| !v.is_empty() && v != "0";
        if fake || set(bot) {
            ConnectingClient::Bot
        } else if set(hltv) {
            ConnectingClient::Hltv
        } else if !port_version.trim().is_empty() {
            ConnectingClient::Port(port_version.trim())
        } else {
            ConnectingClient::Other
        }
    }
}

/// Server rules: the server code calls these where a ruleset decides something, and every ruleset
/// answers them all (no default bodies). A hook needing server state takes this crate's types.
pub trait ServerRules: Sync {
    /// The plugin has become active on a brand-new game: at every map start (after the rules
    /// are installed, before the map's entities spawn). The place to set up the plugin's own
    /// state.
    fn on_activate(&self);

    /// `ClientConnect` (port addition): `Some(reason)` refuses the client, and the engine shows
    /// the reason to it (the engines' reject buffer holds 127 bytes). Asked on each connect,
    /// which the engines run again at every map change.
    fn refuse_client(&self, client: ConnectingClient) -> Option<&'static str>;

    /// `GetGameDescription`'s text for the mode: what the server browser's Game column shows
    /// (`crate::game_description!` builds one with the port's version).
    fn game_description(&self, mode: rules::DescriptionMode) -> &'static CStr;

    // ---- cvars and commands ----------------------------------------------------------------

    /// Server cvars the ruleset needs besides the server's own ([`cvars::CVARS`](crate::rules::cvars::CVARS), TS's). They are
    /// registered when the plugin is ([`super::register`], at `GameDLLInit`), skipping any name
    /// that already exists, so the server has the union of every ruleset's cvars and each ruleset
    /// reads the ones it knows.
    fn cvars(&self) -> &'static [CvarDef];

    /// A TS server command (`globalslow`, `restartround`, `weaponrestriction`) before the
    /// server's handler runs it; true when the ruleset has handled it. The arguments are the
    /// engine's (`Cmd_Argv`).
    fn server_command(&self, name: &str) -> bool;

    /// A player's client command (`ClientCommand` 0xad838) before the server's handlers (weapons,
    /// stunts, rules, bots): `Some(handled)` decides it (false prints "Unknown command"), `None`
    /// lets the server's code run.
    fn client_command(&self, p: &mut Player, cx: &mut Cx, cmd: &str, args: &Args) -> Option<bool>;

    // ---- game rules ------------------------------------------------------------------------

    /// `InstallGameRules` and the rules' constructors for a multiplayer map (`deathmatch` set):
    /// the game mode, teams and `CTSGameRules::Init` ([`GameRules::ts_init`]), rebuilding `g`.
    /// Runs inside the rules state's borrow, so it must not call code that reads the rules state.
    fn install(&self, g: &mut GameRules);

    /// `CTSGameRules::Think` (0x77204), from the multiplay or teamplay `Think` each frame, at the
    /// game time `now`.
    fn think(&self, now: f32);

    /// `CTSGameRules::SetPlayerTheOne` (0x7b078): makes player `p` The One (`scoring::set_player_the_one`,
    /// from The One's death and the random pick). The team switch is the caller's.
    fn set_player_the_one(&self, p: &mut Player, cx: &mut Cx);

    /// The team `CHalfLifeTeamplay::SetDefaultPlayerTeam` (0xe44ac) reads before it checks it
    /// against the team list ([`teams::set_default_team`]).
    fn default_team(&self, p: &mut Player, cx: &mut Cx) -> Vec<u8>;

    // ---- players ---------------------------------------------------------------------------

    /// End of the rules part of `TSInit` at every spawn (`lifecycle::on_spawn`), before the
    /// team and the spawn point.
    fn on_spawn(&self, p: &mut Player, cx: &mut Cx);

    /// The cash a player joins with (`ClientPutInServer`: `GiveCash(atoi(initial_cash.string))`).
    fn put_in_server_cash(&self) -> i32;

    /// Start of `CBasePlayer::StartObserver` (0x99634), before the cash goes to the client.
    fn on_start_observer(&self, p: &mut Player, cx: &mut Cx);

    /// The observer part of `CBasePlayer::PreThink` (0x908cc), run every frame while
    /// `pev->iuser1` is set: the respawn countdown and the join deadline, then
    /// `Observer_HandleButtons` and `Observer_Idle`. PreThink zeroes `impulse` and returns after
    /// it, so a ruleset that respawns the observer here (`lifecycle::stop_observer`) just returns.
    fn observer_think(&self, p: &mut Player, cx: &mut Cx, now: f32);

    /// `CBasePlayer::Observer_SetMode` (0x99e44) as `StartObserver` and the port's `ts_observe`
    /// call it: puts the observer in `mode`. The ruleset's observer camera
    /// ([`observer_think`](Self::observer_think)) calls its own.
    fn observer_set_mode(&self, p: &mut Player, cx: &mut Cx, mode: i32);

    /// End of `CTSGameRules::InitHUD` (0x79134), once a joiner is observing with its respawn
    /// time set.
    fn on_init_hud(&self, p: &mut Player, cx: &mut Cx);

    /// `SetPlayerRespawnTime` (0x75ff4): when the player may respawn (`respawn_at`, -1 for not
    /// this round), the join deadline and the award, `secs` from now.
    fn set_respawn_time(&self, p: &mut Player, cx: &mut Cx, secs: f32);

    /// `CBasePlayer::Killed` (0x8ed5c) right after the respawn time is set.
    fn on_killed(&self, p: &mut Player, cx: &mut Cx);

    /// The tail of `CHalfLifeTeamplay::ChangePlayerTeam` (0xe488c), after `TeamInfo`/`ScoreInfo`
    /// and before `TeamNames`; `kill` is the call's `bKill`.
    fn change_team_tail(&self, p: &mut Player, cx: &mut Cx, kill: bool);

    /// The overheal and The One's regeneration in `CBasePlayer::PreThink` (0x908cc, `+0x73c`),
    /// every frame at the game time `now`.
    fn health_regen(&self, p: &mut Player, cx: &mut Cx, now: f32);

    // ---- scoring and cash ------------------------------------------------------------------

    /// `TSGetPointsForFrag` (0x89140): the points a killing player scores for `frag`, which also
    /// reports whether the kill was a slide (`sliding`) or a stunt (`diving`).
    fn points_for_frag(&self, frag: &Frag<'_>, sliding: &mut bool, diving: &mut bool) -> i32;

    /// The body of `CHalfLifeMultiplay::PlayerKilled` (0x895e4) for victim `p`, once the killer
    /// can be reached (`scoring::player_killed` defers it until then): points, sprees, awards,
    /// cash, messages, `DeathMsg`, logs and `ScoreInfo`. `g` holds TS's damage globals as the
    /// kill left them.
    fn player_killed(&self, p: &mut Player, cx: &mut Cx, d: &DamageInfo, g: Snapshot);

    /// `CashBased()` (0x76abc): whether buying costs money.
    fn cash_based(&self) -> bool;

    /// `CBasePlayer::BuyArmament` (from `PlayerSpawn` when the map has no `game_player_equip`):
    /// buys the loadout the client sent (`tkw`/`tki`/`tka`, `p.inv.sv.armament`).
    fn buy_armament(&self, p: &mut Player, cx: &mut Cx);

    // ---- inventory, world guns and items ------------------------------------------------------

    /// `CBasePlayer::PickUpWeapon` (0x9314c): the slots weapon `id` (info `w`, copied from
    /// `from` when it is a dropped one) costs, or `None` when it is refused before that.
    fn pick_up_cost(&self, id: u8, w: &WeaponInfo, from: Option<&Status>) -> Option<f32>;

    /// `CBasePlayer::DropWeapon` (0x94aaf): whether a dead player's grenades become grenade
    /// power-ups (`CreatePowerUp(0x20, -1)` each, 5 units apart) instead of being dropped like a
    /// living player's.
    fn dead_grenades_become_powerups(&self) -> bool;

    /// `DropWeapon` (0x94ed9, 0x94fad, 0x95265): whether a gun thrown by `drop` or a disarm lasts
    /// `clamp(2 × weaponstay, 10, 120)` s and glows for its dropper
    /// ([`worldgun::set_drop_lifetime`](crate::weapons::worldgun::set_drop_lifetime)), instead of
    /// keeping `CTSWorldGun::Spawn`'s `clamp(weaponstay, 5, 60)`.
    fn drop_lifetime_and_glow(&self) -> bool;

    /// `DropWeapon`'s stack branch (0x94900, the knives' `stack` flag): how many of the `held`
    /// stacked weapons one drop throws (as one world gun, charging back `slots` each); `disarm`
    /// for a disarm's drop.
    fn stack_drop_count(&self, held: i32, disarm: bool) -> i32;

    /// `CTSWorldGun::Fall` (0xa0d84): whether a flying dropped gun traces 0.1 s ahead and hurts a
    /// non-player it hits for `speed × slots × 0.001` (`DMG_NEVERGIB | DMG_CLUB`) when its health
    /// is at most `speed × slots`.
    fn world_gun_fall_impact(&self) -> bool;

    /// A flying dropped gun `gun` (weapon `id`, credited to `attacker`) touched `other`
    /// (`CTSWorldGun::BounceTouch` 0xa0b0c, after its bounce).
    fn world_gun_touch(&self, gun: *mut edict_t, attacker: *mut edict_t, id: u8, other: *mut edict_t);

    /// `CTSAmmoPack::WaitTouch` (0xa16c0): whether a pack a live player touches stays when none
    /// of its ammo fitted (false: the touch removes it).
    fn ammo_pack_stays_when_nothing_fits(&self) -> bool;

    /// `CBasePlayer::TakeHealth` on a player's entvars (healthkits, chargers, doors, negative
    /// `trigger_hurt`, time-based healing): `None` when refused, else whether the player is now
    /// above `max_health` (which starts the 1/s overheal decay).
    fn player_add_health(&self, v: &mut entvars_t, amount: f32) -> Option<bool>;

    /// `CItemBattery::MyTouch` (0xcb74c), once it has accepted player `p`: gives the battery's
    /// `amount` of armour.
    fn battery_armor(&self, p: *mut edict_t, amount: f32);

    /// `m_iJuice` a wall charger (`armor`: `func_recharge`, else `func_healthcharger`) gets from
    /// `Spawn` and `Recharge`.
    fn charger_capacity(&self, armor: bool) -> i32;

    /// One charge of a wall charger (`armor`: `func_recharge`, else `func_healthcharger`) for
    /// `activator` (every 0.1 s while used), taking from `juice`.
    fn charger_charge(&self, armor: bool, juice: &mut i32, activator: *mut edict_t);

    // ---- map entities -------------------------------------------------------------------------

    /// The ruleset's own game object for an entity of `class`, made when the engine creates the
    /// edict's private data (the classname's export), instead of the type the server links to it
    /// (`None`). For a class that is something else in the ruleset (e.g. an HL item where TS has a
    /// power-up). Port addition.
    fn entity_object(&self, class: &CStr) -> Option<Box<dyn Entity>>;

    /// The ruleset's own precaches, made at every map start for every registered ruleset (after
    /// the server's, `gameplay::precache_rulesets`), so the precache list doesn't depend on the
    /// ruleset. Port addition.
    fn precache(&self);

    /// Whether an entity of `class` may spawn (`DispatchSpawn`, for the map's entities and the
    /// game's own): a refused one fails its spawn (-1, the engine frees it) and its class parses
    /// no keyvalue.
    fn spawns_entity(&self, class: &CStr) -> bool;

    /// A keyvalue for an entity's class (`DispatchKeyValue`'s class part, after the entvars
    /// table), before the class's own `KeyValue`. `None` lets the class parse it; `Some(handled)`
    /// means the plugin parsed or refused it, and is `fHandled`.
    fn entity_key_value(&self, ent: &mut dyn Entity, cx: &mut Cx, key: &CStr, value: &CStr) -> Option<bool>;

    /// An entity's `Touch` (`DispatchTouch` and the game's own touches): true when the plugin ran
    /// it instead of the class.
    fn entity_touch(&self, ent: &mut dyn Entity, cx: &mut Cx, other: *mut edict_t) -> bool;

    /// The objective system's rules (`ts_objective_ptr`, `ts_bomb`, `ts_hack`, `ts_teamescape`, the
    /// HUD list, the wires).
    fn objectives(&self) -> &'static objective::ObjectiveRules;

    /// `CTSGameRules::RoundTimeLeft`, which an exploding `ts_bomb` calls (`now` is the game time).
    fn round_time_left(&self, now: f32);

    /// The `TeamWin` of a `ts_wingiver` / `ts_scoregiver` that reached its `callsnumber`.
    fn giver_team_win(&self, w: &ctb::GiverWin);

    /// How a break throws its debris (`CBreakable::Die` 0x752e0, `CBaseDoor::TraceAttack`
    /// 0xb57ac). True: from the last hit, the breakable's debris velocity along the hit direction
    /// × damage × the material's multiplier, glass as the long `TSFrags` with the hit, the
    /// attacker's `fuser1` copied first; a door's along the attacker's direction × damage × 2 (× 8
    /// for a shotgun). False: the SDK's, `g_vecAttackDir × 200` for an `explosion directed`
    /// breakable and none otherwise, a door's none, glass as the 13-byte `TSFrags` (centre, size,
    /// type), no `fuser1` copy.
    fn break_debris_from_hit(&self) -> bool;

    /// The `g_vecAttackDir` a breakable's `TakeDamage` leaves (0x75480..), from the box centre
    /// `absmin + size × 0.5` to the inflictor's origin, normalised.
    fn breakable_attack_dir(&self, inflictor_origin: Vec3, absmin: Vec3, size: Vec3) -> Vec3;

    /// How long a broken `func_breakable` or breakable door waits before it rebuilds itself
    /// (`CBreakable::Die` 0x752e0, `CBaseDoor::TraceAttack` 0xb57ac), given its `respawntime` key (≥ 0); `None`
    /// leaves it broken, with no think, until `RespawnEntities` restores it.
    fn broken_respawn_delay(&self, respawn_time: f32) -> Option<f32>;

    /// Whether `BreakableRespawn` (`CBreakable` 0x74e84, `CBaseDoor`/`CRotDoor` 0xb5604/0xb7838)
    /// waits while a client is inside the box (retrying in 5 s) and fires a breakable's
    /// `respawntarget` (spawnflag 8). False restores the entity at once and fires nothing.
    fn breakable_respawn_guarded(&self) -> bool;

    /// `CTSGameRules::RespawnEntities` as `RestartRound` (0x77920) calls it.
    fn respawn_entities(&self);

    /// The map-entity upkeep of `CTSGameRules::Think` (0x77204) in deathmatch and teamplay, after The One's
    /// check (`now` is the game time).
    fn entities_think(&self, now: f32);

    /// What a `ts_mapglobals` (`e`) sets with its spawnflags (`CTSMapGlobals::Spawn` 0x72534),
    /// or `None` to set nothing.
    fn map_globals(&self, e: *mut edict_t, spawnflags: i32) -> Option<MapFlags>;

    /// Whether a landing `ts_groundweapon` (`OnGround`) also becomes `MOVETYPE_FLY` with
    /// `effects |= 2`, as `ts_groundammo`'s does.
    fn ground_weapon_lands_flying(&self) -> bool;

    /// `CTSGroundWeapon::GetWeapon` (0xa2328) for a player, past its `IsPlayer` test (the
    /// ground weapon `g` is `cx`, the player `p` is `pcx`).
    fn give_ground_weapon(&self, g: &mut GroundWeapon, cx: &mut Cx, p: &mut Player, pcx: &mut Cx);

    // ---- protocol -----------------------------------------------------------------------------

    /// A user message on its way out (`msg`, written into `w`): hands `send` what goes on the
    /// wire in its place (`w` itself to send it as it is).
    fn outgoing_message(&self, msg: Msg, w: MsgWriter, send: &mut dyn FnMut(MsgWriter));

    // ---- power-ups and slow motion ----------------------------------------------------------

    /// The power-up values: durations, the instant types, the pickup thresholds, the slow-motion
    /// amounts (`SetupPowerUp` 0x98014, `WaitTouch`, `UpdatePowerUpState` 0x95748).
    fn powerups(&self) -> &'static PowerupRules;

    /// `SetupPowerUp`'s type for a `ts_powerup` with the type `key` (`pwuptype`, < 1 random)
    /// and `spawnflags`: the type, and whether it is drawn again on each respawn.
    fn powerup_kind(&self, key: i32, spawnflags: i32) -> (u16, bool);

    /// `PlayerAddPowerUp` (0x97bc8) for an armour power-up worth `armor` (100 unless a ruleset
    /// makes it otherwise): gives player `p` the armour.
    fn armor_powerup(&self, p: &mut Player, cx: &mut Cx, armor: f32);

    /// `GoSlow` (0x929d0) sets a target time scale that `InterpolateSlowMotion` (0x9910c) eases
    /// `pev->fuser1` toward each PreThink. False: `GoSlow` writes `fuser1` at once.
    fn time_scale_eases(&self) -> bool;

    /// The end of a player's `GoSlow` (0x929d0) for the time scale `amount` (1 = normal): a
    /// ruleset whose release touches the weapon timers does it here.
    fn on_go_slow(&self, p: &mut Player, amount: f32);

    /// One 0.1 s fuse think of a thrown grenade (`CM61Grenade::WaitThink` 0x7cb68): the fuse's
    /// elapsed time after it, the grenade's time scale being `time_scale`.
    fn grenade_fuse_step(&self, elapsed: f32, time_scale: f32) -> f32;

    /// A player's animation `framerate`, set from the time scale every `PreThink` (0x908cc).
    fn anim_framerate(&self, time_scale: f32) -> f32;

    /// The ruleset sends `TSMBlur` (stunts, power-ups, kills, kung fu), and draws the random
    /// numbers deciding it.
    fn motion_blur(&self) -> bool;

    /// `EMIT_SOUND_DYN`'s pitch (0xe0aa7) for an entity whose time scale is `time_scale`, with
    /// the slow factor `s`.
    fn sound_pitch(&self, pitch: i32, time_scale: f32, s: f32) -> i32;

    // ---- stunts (CTSStunt) ------------------------------------------------------------------

    /// The stunt values (recoveries, keys, which moves exist).
    fn stunts(&self) -> &'static StuntRules;

    /// How the stunt timers keep time.
    fn stunt_clock(&self) -> &'static dyn StuntClock;

    /// `CanDiveHigh/Medium/Low`: the player can do a stunt of this height. `armor` is
    /// `pev->armorvalue`.
    fn can_dive(&self, p: &Player, armor: f32, dive: Dive) -> bool;

    /// `GoBulletTime` (0x9a5b4): bullet time is possible at the time scale `time_scale`
    /// (`pev->fuser1`).
    fn bullet_time_allowed(&self, time_scale: f32) -> bool;

    // ---- bullets (`weapons::fire`, `weapons::bullets`) ------------------------------------------

    /// The trace a bullet flies along from `start` to `end`, fired by `shooter`
    /// (`ShootBulletsPlayer` 0x6f69c and its penetration, `CBulletManager::UpdateBullet`
    /// 0x6ef5c).
    fn bullet_trace(&self, start: Vec3, end: Vec3, shooter: *mut edict_t) -> TraceResult;

    /// A bullet's range-falloff index from `scaled`, the distance in tenths of the ammo's range
    /// unit (`dist / (unit·10)·10`, or `dist·0.0033333334·10` after a penetration) exactly as the
    /// x87 holds it ([`fire::range_scaled`](crate::weapons::fire::range_scaled): nothing in it is rounded to a `float`).
    fn bullet_range_index(&self, scaled: Ext) -> i32;

    /// A bullet's damage where it hits, before [`bullet_damage_multiplier`](Self::bullet_damage_multiplier):
    /// the gun's `damage` (`WeaponInfo::damage` of `weapon`, firing ammo type `ammo`) times the
    /// range falloff `falloff`, for a hitscan pellet or a `simulated` bullet's first hit (a
    /// penetration's carried damage is scaled by the falloff alone).
    fn bullet_damage(&self, ammo: u8, weapon: u8, damage: f32, falloff: f32, simulated: bool) -> f32;

    /// The multiplier of a bullet's damage where it hits. `acc` starts at 1.0 for each
    /// `ShootBulletsPlayer` call and each simulated-bullet update and persists over the
    /// pellets of the call; `cheat` is `damagemult` when it applies (`sv_cheats`). The value
    /// also scales the damage a penetration carries on.
    fn bullet_damage_multiplier(&self, acc: &mut f32, cheat: Option<f32>) -> f32;

    /// How far past the hit point `ShootBulletsPlayer` (0x6f69c) looks for the far side of a
    /// wall, from the damage `left` to carry through. (The simulated bullets use `min(left, 48)`
    /// in every ruleset.)
    fn pierce_step(&self, left: f32) -> f32;

    /// The weapons' `realbullet` (`TSGameRules + 0x578`, `rules::state::realbullet`): non-zero
    /// fires simulated bullets for everyone (2+: stepped every frame). The shooter's slow motion
    /// fires them too, whatever this says.
    fn weapon_realbullet(&self, realbullet: i32) -> i32;

    /// `GetBulletVelocityMult` (`CBulletManager::UpdateBullet` 0x6ef5c): a simulated bullet's
    /// speed factor from the shooter's time scale (`slowmul`: its slow-motion "others" amount,
    /// else its own time scale) and bullet time, as the x87 holds it for the caller in `arith`'s
    /// build.
    fn bullet_velocity_mult(&self, arith: Arith, slowmul: f32, bullet_time: bool) -> Ext;

    // ---- the victim (`player::damage`, `rules::lifecycle`) --------------------------------------

    /// `CBasePlayer::TraceAttack` (0x901cc): the damage a traced hit adds to the multi-damage
    /// after the stunt and dodge ×0.7 and the hitgroup multipliers, and whether it is a
    /// headshot (the hit sound; `glast_headshot` when [`HitDamage::record_headshot`]).
    fn player_hit(&self, hit: &PlayerHit) -> HitDamage;

    /// `CBasePlayer::TakeDamage` (0x8fa94): the damage that reaches `CBaseMonster::TakeDamage`
    /// after armour, truncated to an int (`player::damage::after_kevlar`), changing `armor` if
    /// the ruleset uses it up.
    fn armor_damage(&self, armor: &mut f32, amount: f32, hitgroup: i32, bits: i32) -> f32;

    /// `CBasePlayer::TakeDamage` (0x8fa94): how far a hit the player takes kicks its view up
    /// (`punchangle.x -= ...`).
    fn damage_punch(&self) -> f32;

    // ---- melee (`weapons::melee`) -----------------------------------------------------------------

    /// The victim's block of a melee hit (`VictimKungFuHold`, `VictimKnifeHold`,
    /// `VictimKatanaHold`): `None` if `victim` doesn't block, else what the attacker gets
    /// ([`MeleeResult::Blocked`]), the victim's side applied here. `damage` is the hit's;
    /// `atk_block` the attacker's own kung-fu block window (the attacker is the player being
    /// dispatched). Called for every hit on a player, the delayed ones too.
    fn melee_block(
        &self,
        kind: MeleeKind,
        attacker: *mut edict_t,
        victim: *mut edict_t,
        damage: f32,
        atk_block: f32,
    ) -> Option<MeleeResult>;

    /// A jump kick (`CTSStunt::CloseAttack` 0x9c630) has hit; `strength` is its damage before
    /// Kung Fu's doubling.
    fn jump_kick_hit(&self, p: &mut Player, cx: &mut Cx, strength: f32);

    /// Before a jump kick's hit (`CTSStunt::CloseAttack` 0x9c630): the attacker's kung-fu block
    /// as the hit's block test sees it (`melee_block`'s `atk_block`), after whatever the kick does
    /// to it in `gun`.
    fn jump_kick_attacker_block(&self, gun: &mut Gun) -> f32;

    /// The backflip kick (`CTSStunt::GoWheel` backward): its kung-fu power and delayed hit.
    fn backflip_kick(&self, p: &mut Player, cx: &mut Cx);

    /// `CTSStunt::CheckForBreakables` (0x9bd9c): whether a delayed hit whose stored damage is
    /// spent (0) still runs the player part on a player (the team check, the victim's block,
    /// disarm, knockback and the hit sound) before its 0-damage `TraceAttack`.
    fn zero_delayed_hit_reaches_player(&self) -> bool;

    /// A melee hit that may disarm (`MeleeHit::disarm`, and every delayed kung-fu hit): the
    /// damage it adds to the victim's disarm count (`DropWeapon(-2, dmg)`, dropped at a
    /// per-weapon threshold), or `None` for no disarm. `damage` is the hit's,
    /// `victim_is_the_one` whether the victim is The One.
    fn melee_disarm(&self, damage: f32, victim_is_the_one: bool) -> Option<f32>;

    // ---- explosions and thrown blades ------------------------------------------------------------

    /// `CM61Grenade::WaitThink` (0x7cacc): the grenade's `TSExplosion` damage, full-damage
    /// radius and maximum radius.
    fn grenade_explosion(&self) -> (f32, f32, f32);

    /// `TSExplosion` (0x7d01c): whether the blast knocks the dropped guns (`WorldGun`) in its
    /// sphere away.
    fn explosion_pushes_world_guns(&self) -> bool;

    /// The end of `TSExplosion` (0x7d964), after the damage: `org` is the blast point (the
    /// origin, 1 unit up), `attacker` the attacker or else the inflictor.
    fn explosion_aftermath(&self, org: Vec3, inflictor: *mut edict_t, attacker: *mut edict_t);

    /// A thrown blade of type `btype` (6 combat knife, 7 seal knife, 8 katana): its speed
    /// (`KnifeFire` 0x84044 / `KatanaThrow` 0x83b68), its gravity (`CTSKnife::Spawn` 0x7db54) and
    /// its damage bits (`CTSKnife::BounceTouch` 0x7decc and `WaitThink` 0x7ea30).
    fn thrown_blade(&self, btype: i32) -> projectiles::ThrownBlade;

    /// `CTSKnife::BounceTouch` (0x7decc): what a thrown blade of type `btype` deals the player it
    /// hits at `at`, thrown from `from` (a ruleset whose damage falls off measures the distance
    /// in its own arithmetic).
    fn thrown_blade_damage(&self, btype: i32, from: Vec3, at: Vec3) -> f32;

    // Audit round 2: D2

    /// `CTSGroundWeapon::Spawn` (0xa1b40) turns an akimbo pair built from two singles into its
    /// single, the only form the ruleset's `PickUpWeapon` accepts. False keeps the id, so the
    /// ground weapon gives the pair.
    fn ground_weapon_pairs_to_single(&self) -> bool;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `ClientConnect`'s classification: bots and HLTV whatever their userinfo, then the port's
    /// key.
    #[test]
    fn connecting_clients() {
        use ConnectingClient::*;
        assert_eq!(ConnectingClient::classify(true, "", "", ""), Bot, "FL_FAKECLIENT");
        assert_eq!(ConnectingClient::classify(false, "1", "", ""), Bot, "*bot");
        assert_eq!(ConnectingClient::classify(true, "", "1", "3.0.2"), Bot);
        assert_eq!(ConnectingClient::classify(false, "", "1", ""), Hltv);
        assert_eq!(ConnectingClient::classify(false, "", "2", "3.0.2"), Hltv);
        assert_eq!(ConnectingClient::classify(false, "0", "0", " 3.0.2 "), Port("3.0.2"));
        assert_eq!(ConnectingClient::classify(false, "", "", ""), Other, "TS 3.0's client");
        assert_eq!(ConnectingClient::classify(false, "", "", "  "), Other);
    }
}
