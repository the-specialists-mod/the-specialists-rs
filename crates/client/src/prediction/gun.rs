//! Local weapon prediction (`HUD_WeaponsPostThink` `0x10003830`, client `CTSGun`).
//!
//! The client runs the same `shared::weapons::Gun` state machine as the server, through a client
//! [`GunHost`]: the networked state (`clientdata` `vuser*`/`ammo_*`/`m_flNextAttack` and
//! `weapon_data[16]`) is unpacked with `shared::protocol::weapons`, the gun runs one usercmd
//! (`pre_frame`, `weapon_post_frame` behind TS's client gate, `decrement_timers`) and the result
//! is packed into `to`.
//!
//! The server plays every weapon event with `FEV_NOTHOST`, so the shooter only sees and hears
//! its own shots through these predicted events (`runfuncs` only). Traces, damage, projectiles
//! and server sounds are server-only; the `fire` event re-traces the pellets with the same seed.

use super::TS_GUN_SLOT;
use crate::{state, view};
use goldsrc::Global;
use goldsrc::ext::render as r;
use goldsrc::sys::*;
use shared::protocol::weapons::{pack_clientdata, pack_weapon_data, unpack_clientdata, unpack_weapon_data};
use shared::weapons::gun::{MeleeHit, MeleeResult, Shot, Sound};
use shared::weapons::spread::Vec3;
use shared::weapons::{Event, EventArgs, Gun, GunHost, MAX_AMMO, MAX_WEAPONS, Status, att, body_by_items, info, models};
use std::cell::RefCell;

#[derive(Default)]
struct Pred {
    /// Event indexes in `Event::ALL` order (0 = not precached yet).
    events: [u16; 11],
    /// View model indexes per weapon id (0 = not looked up).
    v_models: Vec<i32>,
    /// State that isn't networked (recoil penalty, last shot time, kung-fu bar, combos, mode
    /// memory), carried from one first-time prediction to the next.
    gun: Option<Gun>,
    /// `0x100d5cb8`: the last view animation `HUD_SendWeaponAnim` (`0x1001ace0`) played.
    last_anim: i32,
    /// The client gun's `pev->body` (`0x1009f23c+0x114`), the body every predicted view animation
    /// uses. Only two places write it: each command's rebuild of a current record
    /// (`0x10003a8b..0x10003a99`, [`rebuilt_body`]) and the respawn `Deploy` (`0x100087a5`). The
    /// gun's Spawn leaves it, and it starts at 0.
    pev_body: i32,
    /// The local weapon's alternate grip (`0x10189f9d`, a byte): `(int)vuser2[2]` from
    /// `HUD_WeaponsPostThink`, or the demo's record type 1 during playback.
    alt_grip: u8,
    /// `lasthealth` (`0x100991a4`): the local health of the last `runfuncs` command, as an int.
    last_health: i32,
    /// `0x1009ff08`: the client gun has been (re)spawned since the last `HUD_VidInit`.
    gun_spawned: bool,
    /// The client player's `m_pActiveItem` (`0x1009f008`) is the TS gun: set by the first command
    /// whose `m_iId` is 16 and never cleared.
    active_gun: bool,
}

static PRED: Global<RefCell<Option<Pred>>> = Global(RefCell::new(None));

fn with<R>(f: impl FnOnce(&mut Pred) -> R) -> R {
    let mut g = PRED.0.borrow_mut();
    f(g.get_or_insert_with(Pred::default))
}

/// Alternate grip (anim set B) of the local weapon, the flag `GetActiveAnimSet` (`0x1003df60`)
/// tests. During demo playback it first takes the demo's last type 1 record, as TS does there.
pub fn alt_grip() -> bool {
    if goldsrc::client::engine().demo_playing() {
        let rec = crate::demo::recorded_alt_grip();
        if let Ok(mut g) = PRED.0.try_borrow_mut() {
            g.get_or_insert_with(Pred::default).alt_grip = rec;
        }
        return rec != 0;
    }
    PRED.0.try_borrow().ok().and_then(|g| g.as_ref().map(|p| p.alt_grip)).unwrap_or(0) != 0
}

/// `0x1000b9d0` on the client `player` (`0x1009e628`), which the `Spectator` handler runs for the
/// local player (`0x100652fd`): the predicted spread penalty (`player+0x764`, the non-networked
/// `Gun::spread_penalty`) and the crosshair's copy of it (`0x1018d188`, `state.spread`) go to 0.
/// It also zeroes `player+0x768..0x770`, which nothing in the client reads, and `ev_punchangle`
/// (`0x10191480`), which only `V_DropPunchAngle` ever writes, so neither has port state.
pub fn reset_spread() {
    with(|p| {
        if let Some(g) = p.gun.as_mut() {
            g.spread_penalty = 0.0;
        }
    });
    state::with(|s| s.spread = 0.0);
}

/// `CHudTSWeapons::VidInit` (`0x1004cb50`) clears the alternate grip.
pub fn reset_alt_grip() {
    with(|p| p.alt_grip = 0);
}

/// `HUD_VidInit` (`0x1001aa50`) -> `VGui_Startup` (`0x1005b3e0`) -> the viewport's `Initialize`
/// (`0x10060aa0`) -> the TS panel's reset (`0x10065ff0`, vtable slot 133) clears `0x1009ff08`, so
/// the next `HUD_WeaponsPostThink` spawns the gun again (see [`spawn_gun`]).
pub fn vid_init() {
    with(|p| p.gun_spawned = false);
}

/// `CTSGun::Spawn` on the client gun (`0x100083f0`, with `CTSGun::Reset` `0x10008470`). It zeroes
/// the inventory (`+0x104`, 38 records), the reserve ammo (`+0xc4`), the current id and its record
/// and info pointers (`+0xb8..+0xc0`), the `UpdateClientData` send caches (`+0x83a`, `+0x83c`,
/// `+0x86c`, `+0x870`), `+0x89c` (written nowhere else, read nowhere), the last shot time
/// (`+0x8a0`, `Gun::last_shot`) and the last view-kick time (`+0x8c0`, `view::reset_kick_time`),
/// and sets `m_iClip` (`+0xa4`) and `m_iDefaultAmmo` (`+0xb4`) to 1 and `m_iId` to 16. The
/// records and reserve are otherwise kept from command to command: [`gun_from`] only rebuilds the
/// current record. The send caches have no port state (the client never reads them). The combo
/// counters (`+0x8b4..+0x8bc`) and the grenade-pin time (`+0x8c4`) are left alone, as are the
/// player's fields (melee bar, windows, mode memory).
fn spawn_gun(g: &mut Gun) {
    reset_gun(g);
    view::reset_kick_time();
}

/// The [`Gun`] part of [`spawn_gun`].
fn reset_gun(g: &mut Gun) {
    g.status = [Status::default(); MAX_WEAPONS];
    g.reserve = [0; MAX_AMMO];
    g.current = 0;
    g.clip_mirror = 1;
    g.last_shot = 0.0;
}

/// `CTSGun::Deploy` on the client gun (`0x10008720`, vtable `+0x120`), which only the client
/// player's Spawn (`0x100036d0`) reaches. Kung fu runs `HolsterWeapon` ([`holster`]). Otherwise it
/// resets the current record's body (`+0x18`, also `pev->body`), state (`+0x8`) and alternate grip
/// (`+0x2c`), clears `+0x898` (written nowhere else, read nowhere), runs `DefaultDeploy`
/// (`0x100035f0`: view model, `m_flNextAttack` 0.5, idle 3.0; unlike the SDK's it plays no view
/// animation), plays the `draw` event (`FEV_NOTHOST`, id, attachments), and syncs the fire-mode
/// memory (`pl+0x486+id`) as the server's Deploy does. The gun's Spawn runs straight after and
/// wipes the records, and `from` overwrites the timers, so the mode memory is what lasts. Returns
/// the draw event's id and attachments.
fn deploy(g: &mut Gun) -> Option<(u8, u32)> {
    let id = g.current;
    if id == 0 {
        holster(g);
        return None;
    }
    let s = &mut g.status[id as usize];
    s.body = body_by_items(id, s.items);
    s.state = 0;
    s.alt_grip = false;
    let items = s.items;
    g.next_attack = 0.5;
    g.idle = 3.0;
    let mem = &mut g.mode_memory[id as usize];
    if *mem < 1 {
        // A byte in TS (`pl+0x486+id`), so a networked mode of 255 wraps instead of overflowing.
        *mem = g.status[id as usize].mode.wrapping_add(1);
    } else {
        g.status[id as usize].mode = *mem - 1;
    }
    Some((id, items))
}

/// The client player's `Killed` (`0x100036b0`): `m_pActiveItem->Holster()` (vtable `+0x128` =
/// `0x1000aaf0` -> `CTSGun::HolsterWeapon` `0x1000ab00`). The client player's `IsAlive`
/// (`0x10004700`) reads `pev->health`, which nothing on the client writes, so the holster always
/// stops before its living-player part (reload/state reset, switch to kung fu, draw anim and
/// event, combo reset). What runs first: view and weapon model 0, `Unzoom`, the animation
/// extensions cleared, `m_flNextAttack = 0.5`, `m_flTimeWeaponIdle = 3.0` and, if a knife or
/// katana hit is pending (`pl+0x788` window and `pl+0x790` source both set), the melee window
/// cleared. The models, FOV, `m_flNextAttack` and the idle time are copied from `from` straight
/// after (`0x100038e7..`), and [`gun_from`] unpacks the same fields, so the melee window is the
/// only lasting effect.
fn holster(g: &mut Gun) {
    if g.melee_window != 0.0 && g.melee_source != 0 {
        g.melee_window = 0.0;
        g.melee_damage = 0.0;
        g.melee_source = 0;
    }
}

/// The start of `HUD_WeaponsPostThink`, before anything is read from `from`: the gun spawn after
/// a `HUD_VidInit` (`0x100036f0`, the call at `0x10003713`, on every call) and, for `runfuncs`
/// commands, the SDK's `lasthealth` check (`0x10003877..0x100038e2`, int `0x100991a4`). These
/// act on the state carried from the last first-time prediction, which [`gun_from`] then
/// overlays with the networked fields.
fn spawn_and_death(c: &clientdata_t, runfuncs: bool) {
    if !with(|p| std::mem::replace(&mut p.gun_spawned, true)) {
        with(|p| spawn_gun(p.gun.get_or_insert_with(Gun::default)));
    }
    if !runfuncs {
        return;
    }
    let (died, spawned, active) = with(|p| {
        let alive = c.health > 0.0;
        let r = (!alive && p.last_health > 0, alive && p.last_health <= 0, p.active_gun);
        p.last_health = c.health as i32;
        r
    });
    if died && active {
        // health > 0 -> <= 0 (0x1000389b).
        with(|p| holster(p.gun.get_or_insert_with(Gun::default)));
    }
    if spawned {
        // health <= 0 -> > 0: the client player's Spawn (0x100036d0), i.e. the active item's
        // Deploy (vtable +0x120, only with an active item) and the spread reset 0x1000b9d0, then
        // the gun's Spawn (0x100038c8). The view model's draw animation comes from the server's
        // Deploy (`DefaultDeploy` with skiplocal 0 sends svc_weaponanim).
        let deployed = active.then(|| {
            with(|p| {
                let g = p.gun.get_or_insert_with(Gun::default);
                let r = deploy(g);
                if r.is_some() {
                    // 0x100087a5: pev->body = the record's new body.
                    p.pev_body = g.status[g.current as usize].body;
                }
                r
            })
        });
        if let Some(Some((id, items))) = deployed {
            let ev = event_index(Event::Draw);
            r::playback_event(FEV_NOTHOST as i32, ev, 0.0, 0.0, id as i32, items as i32, false, 0);
        }
        reset_spread();
        with(|p| spawn_gun(p.gun.get_or_insert_with(Gun::default)));
    }
}

/// The alternate-grip update in `HUD_WeaponsPostThink` (`0x10003bc4..0x10003c0c`), on every call
/// whose `from` weapon is the TS gun (prediction or not): while recording, a change from the
/// stored byte writes demo record type 1 with the new `int`.
fn update_alt_grip(c: &clientdata_t) {
    if c.m_iId != TS_GUN_SLOT as i32 {
        return;
    }
    let alt = c.vuser2[2] as i32;
    if with(|p| p.alt_grip as i32 != alt) && goldsrc::client::engine().demo_recording() {
        crate::demo::write_buffer(crate::demo::ALT_GRIP, &alt.to_le_bytes());
    }
    with(|p| p.alt_grip = alt as u8);
}

fn event_index(ev: Event) -> u16 {
    let i = Event::ALL.iter().position(|&e| e == ev).unwrap_or(0);
    let cached = with(|p| p.events[i]);
    if cached != 0 {
        return cached;
    }
    let idx = r::precache_event(ev.path());
    with(|p| p.events[i] = idx);
    idx
}

fn view_model_index(id: u8) -> i32 {
    let id = (id as usize).min(MAX_WEAPONS - 1);
    let cached = with(|p| p.v_models.get(id).copied().unwrap_or(0));
    if cached != 0 {
        return cached;
    }
    let idx = r::model_index(&models(id as u8).0);
    with(|p| {
        p.v_models.resize(MAX_WEAPONS, 0);
        p.v_models[id] = idx;
    });
    idx
}

/// `HUD_SendWeaponAnim` (`0x1001ace0`): the local view model, on a command's first prediction or
/// when forced. Remembers the animation for the replay at the end of [`post_think`].
fn send_weapon_anim(anim: i32, body: i32, runfuncs: bool, force: bool) {
    if runfuncs || force {
        with(|p| p.last_anim = anim);
        r::weapon_animation(anim, body);
    }
}

/// The client side of the gun's side effects for one usercmd.
struct ClientHost<'a> {
    from: &'a local_state_t,
    cmd: &'a usercmd_t,
    runfuncs: bool,
    time: f32,
    seed: u32,
    powerups: u16,
    /// Predicted results written back into `to`.
    fov: f32,
    viewmodel: i32,
    iuser4: i32,
    /// The player's `pev->weaponanim` (`+0x218`), copied to `to` after the gun's frame.
    weaponanim: i32,
    /// The gun's `pev->body` ([`Pred::pev_body`]); nothing in the gun's frame changes it.
    body: i32,
    /// "Silenced" for the current shot's view shake (`VKickback` passes the multiplier only).
    silenced: bool,
}

impl GunHost for ClientHost<'_> {
    fn time(&self) -> f32 {
        self.time
    }
    fn slow_mul(&self) -> f32 {
        self.from.client.fuser1.max(0.0)
    }
    fn powerups(&self) -> u16 {
        self.powerups
    }
    fn kungfu(&self) -> bool {
        false // only scales melee damage (server)
    }
    fn alive(&self) -> bool {
        let c = &self.from.client;
        c.deadflag == 0 && c.health > 0.0
    }
    fn buttons(&self) -> u32 {
        self.cmd.buttons as u32
    }
    fn pressed(&self) -> u32 {
        self.cmd.buttons as u32 & !(self.from.playerstate.oldbuttons as u32)
    }
    fn origin(&self) -> Vec3 {
        self.from.client.origin
    }
    fn view_ofs(&self) -> Vec3 {
        self.from.client.view_ofs
    }
    fn velocity(&self) -> Vec3 {
        self.from.client.velocity
    }
    fn v_angle(&self) -> Vec3 {
        self.cmd.viewangles
    }
    fn flags(&self) -> i32 {
        self.from.client.flags
    }
    fn waterlevel(&self) -> i32 {
        self.from.client.waterlevel
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
    fn random_long(&mut self, lo: i32, hi: i32) -> i32 {
        r::random_long(lo, hi)
    }
    fn fov(&self) -> f32 {
        // As the server: 90 (the spawn default) means "not zoomed".
        if self.fov >= 90.0 { 0.0 } else { self.fov }
    }
    fn set_fov(&mut self, fov: f32) {
        self.fov = fov;
    }
    fn set_view_model(&mut self, id: Option<u8>) {
        self.viewmodel = id.map(view_model_index).unwrap_or(0);
    }
    fn view_model_hidden(&self) -> bool {
        self.viewmodel == 0
    }
    fn set_weapon_model(&mut self, _model: Option<(u8, bool)>) {}
    fn event(&mut self, ev: Event, a: EventArgs) {
        if self.runfuncs {
            r::playback_event(FEV_NOTHOST as i32, event_index(ev), a.f1, a.f2, a.i1, a.i2, a.b1 != 0, a.b2);
        }
    }
    fn weapon_anim(&mut self, anim: i32, _skip_local: bool, _body: i32) {
        // The client `CBasePlayerWeapon::SendWeaponAnim` (0x10003680): pev->weaponanim, then
        // HUD_SendWeaponAnim (first prediction only) with the gun's pev->body (0x10003695), not
        // the body argument.
        self.weaponanim = anim;
        send_weapon_anim(anim, self.body, self.runfuncs, false);
    }
    fn player_anim(&mut self, _anim: i32) {}
    fn kick(&mut self, pitch: f32, yaw: f32) {
        // VKickback: once per predicted shot.
        if self.runfuncs {
            view::kick_angles(pitch, yaw, self.silenced, self.time);
        }
    }
    fn view_shake(&mut self, amp: f32) {
        // Gated on runfuncs (`*0x100d5cbc`) like the kick.
        if self.runfuncs {
            view::melee_shake(amp);
        }
    }
    fn sound(&mut self, _s: Sound) {}
    fn shoot(&mut self, _shot: &Shot, _gun: &mut Gun) {}
    fn melee(&mut self, _hit: &MeleeHit, _gun: &mut Gun) -> MeleeResult {
        MeleeResult::Miss // hits are resolved by the server
    }
    fn throw_grenade(&mut self, _origin: Vec3, _velocity: Vec3, _roll: bool) {}
    fn throw_blade(&mut self, _btype: i32) {}
    fn slots_changed(&mut self, _delta_free: i32) {}
    fn melee_bar_changed(&mut self, _bar: f32) {}
    fn block_player(&mut self, _t: f32) {
        // The client copies of `KatanaFire` (0x10009ee0) and `CCombatAttack` (0x1000af10) never
        // block: the only `iuser4` write in the client gun is leaving a wall climb (0x1000b4ef).
    }
}

/// Builds the gun for this command: the state carried from the last first-time prediction, with
/// the networked fields from `from`. As in TS (`0x10003965..0x10003bbe`), only the current record,
/// its akimbo/single twin's clip and its reserve ammo are rebuilt; the other records keep their
/// values until the gun's Spawn ([`spawn_gun`]).
fn gun_from(from: &local_state_t) -> (Gun, u16) {
    let mut g = with(|p| p.gun.clone()).unwrap_or_default();
    let powerups = rebuild(&mut g, from);
    (g, powerups)
}

/// The `pev->body` the rebuild writes (`0x10003a8b`: `GetBodyByItems(id, record items)`), or
/// `None` for kung fu, whose rebuild (`0x10003a0a` -> `0x10003a0c`) leaves it.
fn rebuilt_body(g: &Gun) -> Option<i32> {
    g.slot().map(|s| body_by_items(g.current, s.items))
}

/// The networked part of [`gun_from`]. Returns the power-up bits.
fn rebuild(g: &mut Gun, from: &local_state_t) -> u16 {
    let powerups = unpack_clientdata(&from.client, g);
    // TS's rebuild never writes a record's owned byte, so its client DropWeapon (0x100085d0) and
    // GetBetterWeapon (0x10008660) never find a weapon. The port marks the current record owned
    // (`unpack_clientdata`) and only that one, so a kept record can't become a predicted switch.
    let cur = g.current as usize;
    for (i, s) in g.status.iter_mut().enumerate() {
        if i != cur {
            s.owned = false;
        }
    }
    unpack_weapon_data(&from.weapondata[TS_GUN_SLOT], g);
    g.next_alt = from.weapondata[TS_GUN_SLOT].fuser4;
    powerups
}

/// The gate on the gun's own `ItemPostFrame` (vtable `+0x134` = `0x1000b7b0`) in
/// `HUD_WeaponsPostThink` (`0x10003d4b..0x10003d76`): the player's `deadflag` (from `from`) isn't 5,
/// `0x10026860` is false (the last `TSHealth`, `0x100cf9d8`, is above 0) and `from`'s
/// `m_flNextAttack` <= 0. Unlike the server's `CBasePlayer::ItemPostFrame`
/// ([`Gun::post_frame_ready`]) there is no pass for kung fu or for +cancel during a reload.
fn post_frame_gate(c: &clientdata_t, hud_health: i32) -> bool {
    c.deadflag != 5 && hud_health > 0 && c.m_flNextAttack <= 0.0
}

/// `HUD_WeaponsPostThink` (`0x10003830`) for one usercmd.
pub fn post_think(from: &local_state_t, to: &mut local_state_t, cmd: &usercmd_t, runfuncs: bool, time: f64, seed: u32) {
    let c = &from.client;
    spawn_and_death(c, runfuncs);
    // `local_68` (set at 0x1000385c, tested at 0x10003965, which jumps to the epilogue): everything
    // else, including writing `to`, needs `from`'s weapon to be the TS gun. Otherwise `to` keeps
    // the engine's copy of `from` (FWGS `CL_RunUsercmd`: `*to = *from`, then pmove).
    if c.m_iId != TS_GUN_SLOT as i32 {
        return;
    }
    let dead = c.deadflag != 0 || c.health <= 0.0;
    // 0x10003d40: the client player's m_pActiveItem is the gun from now on.
    with(|p| p.active_gun = true);
    update_alt_grip(c);
    let (mut gun, powerups) = gun_from(from);
    let body = with(|p| {
        if let Some(b) = rebuilt_body(&gun) {
            p.pev_body = b;
        }
        p.pev_body
    });
    // The rebuild's record pointer (`0x1009f2f4`), null for kung fu.
    let has_record = gun.current != 0;
    // HUD_PostRunCmd (0x10003830) keeps the active items for DrawLaserDots, cleared while the
    // next attack is pending (TS `0x1018a628`).
    let ready = if c.m_flNextAttack > 0.0 { 0 } else { gun.slot().map_or(0, |s| s.active as i32) };
    state::with(|st| {
        st.items_ready = ready;
        st.local_velocity = c.velocity;
    });

    // A networked weapon change plays nothing here: the server's Deploy/HolsterWeapon send the
    // view animation with svc_weaponanim (`DefaultDeploy` skiplocal 0). TS's own check after
    // ItemPostFrame (0x10003d8d..0x10003dc1) passes `SelectWeapon` (0x100086b0) the gun's current
    // id, which it rejects, so its Deploy there never runs.
    let silenced = gun.slot().is_some_and(|s| s.items & att::SILENCER != 0) && info(gun.current).is_some_and(|w| w.att_default & 1 == 0);
    let mut host = ClientHost {
        from,
        cmd,
        runfuncs,
        time: time as f32,
        seed,
        powerups,
        fov: c.fov,
        viewmodel: c.viewmodel,
        iuser4: c.iuser4,
        weaponanim: c.weaponanim,
        body,
        silenced,
    };
    // Same order as the server: PreThink (windows, penalty decay, bar), the weapon frame, then
    // the timers.
    let dt = Gun::timer_dt(cmd.msec as f32 * 0.001, c.fuser1.max(0.0), powerups);
    gun.pre_frame(dt, !dead);
    if post_frame_gate(c, state::with(|s| s.health)) {
        gun.weapon_post_frame(&mut host);
    }
    gun.decrement_timers(dt);

    let (fov, viewmodel, iuser4, weaponanim) = (host.fov, host.viewmodel, host.iuser4, host.weaponanim);
    pack_clientdata(&gun, powerups, &mut to.client);
    pack_weapon_data(&gun, &mut to.weapondata[TS_GUN_SLOT]);
    to.weapondata[TS_GUN_SLOT].fuser4 = gun.next_alt;
    to.weapondata[TS_GUN_SLOT].m_fInReload = gun.slot().map(|s| s.in_reload).unwrap_or(0);
    to.client.fov = fov;
    to.client.viewmodel = viewmodel;
    to.client.iuser4 = iuser4;
    to.client.weaponanim = weaponanim;
    to.client.maxspeed = from.client.maxspeed;
    to.client.fuser1 = from.client.fuser1;
    // 0x10003e5f: a first prediction whose predicted animation isn't the one last played (it was
    // set by a re-predicted command) forces it, with the gun's body or 2 without a record.
    if runfuncs && with(|p| p.last_anim) != weaponanim {
        send_weapon_anim(weaponanim, if has_record { body } else { 2 }, runfuncs, true);
    }

    if runfuncs {
        let s = gun.slot().copied().unwrap_or_default();
        state::with(|st| {
            st.powerups = powerups as i32;
            st.attachments = s.items as i32;
            st.local_items = s.active as i32;
            st.firemode = s.mode as i32;
            st.clip = s.clip;
            st.reserve_ammo = if gun.current == 0 { 0 } else { gun.current_reserve() };
            st.spread = gun.spread_penalty;
            st.predicted_weapon = (!dead).then_some(gun.current as i32);
        });
        with(|p| p.gun = Some(gun));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::weapons::id;

    /// Server pack -> client unpack keeps what the predictor needs.
    #[test]
    fn networked_round_trip() {
        let mut server = Gun { current: id::AK47, ..Default::default() };
        let s = &mut server.status[id::AK47 as usize];
        s.owned = true;
        s.clip = 21;
        s.mode = 1;
        s.items = att::SCOPE;
        s.alt_grip = true;
        server.reserve[info(id::AK47).unwrap().ammo as usize] = 60;
        server.next_primary = 0.05;
        server.next_attack = 0.3;
        let mut ls = local_state_t::default();
        pack_clientdata(&server, 0x10, &mut ls.client);
        pack_weapon_data(&server, &mut ls.weapondata[TS_GUN_SLOT]);
        let mut g = Gun::default();
        let powerups = rebuild(&mut g, &ls);
        assert_eq!(powerups, 0x10);
        assert_eq!(g.current, id::AK47);
        let c = g.slot().unwrap();
        assert_eq!((c.clip, c.mode, c.items, c.alt_grip), (21, 1, att::SCOPE, true));
        assert_eq!(g.current_reserve(), 60);
        assert_eq!((g.next_primary, g.next_attack), (0.05, 0.3));
    }

    /// The death holster clears a pending knife/katana hit, not a kung-fu one (source 0).
    #[test]
    fn holster_clears_blade_window() {
        let mut g = Gun { melee_window: 0.2, melee_damage: 40.0, melee_source: 1, ..Default::default() };
        holster(&mut g);
        assert_eq!((g.melee_window, g.melee_damage, g.melee_source), (0.0, 0.0, 0));
        let mut g = Gun { melee_window: 0.2, melee_damage: 40.0, melee_source: 0, ..Default::default() };
        holster(&mut g);
        assert_eq!((g.melee_window, g.melee_damage, g.melee_source), (0.2, 40.0, 0));
    }

    /// Records other than the current one, and other ammo types' reserve, survive a command's
    /// rebuild (TS keeps them until the gun's Spawn); only the current record counts as owned.
    #[test]
    fn rebuild_keeps_other_records() {
        let mut server = Gun { current: id::AK47, ..Default::default() };
        server.status[id::AK47 as usize] = Status { owned: true, clip: 30, ..Default::default() };
        let mut ls = local_state_t::default();
        pack_clientdata(&server, 0, &mut ls.client);
        pack_weapon_data(&server, &mut ls.weapondata[TS_GUN_SLOT]);

        let mut g = Gun { current: id::BERETTA, ..Default::default() };
        g.status[id::BERETTA as usize] = Status { owned: true, clip: 7, mode: 1, items: att::SILENCER, ..Default::default() };
        let other_ammo = info(id::BERETTA).unwrap().ammo as usize;
        assert_ne!(other_ammo, info(id::AK47).unwrap().ammo as usize);
        g.reserve[other_ammo] = 45;
        rebuild(&mut g, &ls);
        assert_eq!(g.current, id::AK47);
        assert_eq!(g.slot().unwrap().clip, 30);
        assert!(g.status[id::AK47 as usize].owned);
        let kept = g.status[id::BERETTA as usize];
        assert_eq!((kept.owned, kept.clip, kept.mode, kept.items), (false, 7, 1, att::SILENCER));
        assert_eq!(g.reserve[other_ammo], 45);

        reset_gun(&mut g);
        assert_eq!(g.status, [Status::default(); MAX_WEAPONS]);
        assert_eq!((g.reserve, g.current, g.clip_mirror, g.last_shot), ([0; MAX_AMMO], 0, 1, 0.0));
    }

    /// TS's client gate: deadflag 5, the HUD's last health and `m_flNextAttack` only.
    #[test]
    fn post_frame_gate_matches_ts() {
        let c = clientdata_t { m_flNextAttack: 0.0, ..Default::default() };
        assert!(post_frame_gate(&c, 100));
        assert!(!post_frame_gate(&c, 0), "0x10026860: TSHealth <= 0");
        assert!(post_frame_gate(&clientdata_t { deadflag: 2, ..c }, 1), "only deadflag 5 stops it");
        assert!(!post_frame_gate(&clientdata_t { deadflag: 5, ..c }, 1));
        let busy = clientdata_t { m_flNextAttack: 0.2, ..c };
        assert!(!post_frame_gate(&busy, 100), "no kung-fu or +cancel pass");
    }

    /// The rebuild writes pev->body from the current record; kung fu leaves it.
    #[test]
    fn rebuild_body() {
        let mut g = Gun { current: id::AK47, ..Default::default() };
        g.status[id::AK47 as usize] = Status { owned: true, items: att::SCOPE, body: 0, ..Default::default() };
        assert_eq!(rebuilt_body(&g), Some(body_by_items(id::AK47, att::SCOPE)));
        g.current = 0;
        assert_eq!(rebuilt_body(&g), None);
    }

    /// The respawn Deploy resets the record's state and grip, plays no animation and syncs the
    /// fire-mode memory both ways; kung fu runs the (early) holster instead.
    #[test]
    fn respawn_deploy() {
        let mut g = Gun { current: id::AK47, ..Default::default() };
        g.status[id::AK47 as usize] = Status { owned: true, mode: 2, state: 3, alt_grip: true, items: att::SCOPE, ..Default::default() };
        assert_eq!(deploy(&mut g), Some((id::AK47, att::SCOPE)));
        let s = g.status[id::AK47 as usize];
        assert_eq!((s.state, s.alt_grip, s.body), (0, false, body_by_items(id::AK47, att::SCOPE)));
        assert_eq!((g.next_attack, g.idle), (0.5, 3.0));
        assert_eq!(g.mode_memory[id::AK47 as usize], 3, "unset memory takes the record's mode");

        g.status[id::AK47 as usize].mode = 0;
        deploy(&mut g);
        assert_eq!(g.status[id::AK47 as usize].mode, 2, "set memory restores the mode");

        let mut g = Gun { melee_window: 0.2, melee_damage: 40.0, melee_source: 1, ..Default::default() };
        assert_eq!(deploy(&mut g), None);
        assert_eq!(g.melee_window, 0.0);
    }

    /// Networked weapon state of any value (a hostile server's `vuser*`/`ammo_*` floats), then
    /// the respawn Deploy: a mode of 255 wraps the byte mode memory instead of overflowing.
    #[test]
    fn garbage_clientdata_rebuild_and_deploy() {
        for v in [f32::NAN, f32::INFINITY, -f32::INFINITY, 1e20, -1e20, 255.0, 256.0, -1.0, 0.0, 16.0, 38.0] {
            let mut ls = local_state_t::default();
            let c = &mut ls.client;
            (c.vuser1, c.vuser2, c.vuser3, c.vuser4) = ([v; 3], [v; 3], [v; 3], [v; 3]);
            (c.ammo_shells, c.ammo_nails, c.m_flNextAttack) = (v as i32 / 2, v as i32 / 2, v);
            ls.weapondata[TS_GUN_SLOT].m_iClip = v as i32;
            for current in [None, Some(id::AK47)] {
                let mut g = Gun::default();
                rebuild(&mut g, &ls);
                if let Some(id) = current {
                    g.current = id;
                }
                rebuilt_body(&g);
                deploy(&mut g);
            }
        }
        let mut g = Gun { current: id::AK47, ..Default::default() };
        g.status[id::AK47 as usize].mode = 255;
        deploy(&mut g);
        assert_eq!(g.mode_memory[id::AK47 as usize], 0);
    }
}
