//! The server implementation of `shared::weapons::GunHost`: engine calls for events, view
//! models, sounds, traces and damage on behalf of one player.

use super::{Inventory, SvState, assets, fire, melee, net, projectiles};
use crate::player::Player;
use crate::world::Cx;
use goldsrc::edict::EdictPtr;
use goldsrc::ext::weapons::WeaponsEngine;
use goldsrc::server::engine;
use goldsrc::sys::*;
use shared::weapons::gun::{MeleeHit, MeleeResult, Shot, Sound};
use shared::weapons::spread::Vec3;
use shared::weapons::{Event, EventArgs, Gun, GunHost};
use std::ffi::CStr;

pub struct ServerHost<'a> {
    pub edict: *mut edict_t,
    pub sv: &'a mut SvState,
    /// The player, its [`Inventory`] moved out for the call (the gun and `sv` are borrowed from
    /// it): [`reach_player`](Self::reach_player) puts them back and lends the player around the
    /// calls that reach the world.
    player: &'a mut Player,
    pub powerups: u16,
    pub kungfu: bool,
    /// The time scale thrown grenades and blades start with (`projectiles::thrower_time_scale`).
    pub throw_scale: f32,
    /// `CTSAnimation::Animate` ids requested during the call; played (in order) once the gun is
    /// released, because the animation state lives in `Player::stunt`.
    anims: Vec<i32>,
    /// The deploy/holster changed the animation extension.
    ext_changed: bool,
    /// Writes to the stunt state (`pl+0x4f4` recover timer, `pl+0x4f8` close-combat cooldown),
    /// applied with the animations once the gun is released.
    stunt_recover: Option<f32>,
    close_attack_cd: Option<f32>,
}

/// Runs `f` with the player's gun and a host for it, then applies the third-person animation
/// requests (`set_anim_ext`, `Animate`) to the movement package's state.
///
/// The inventory is moved out of `p` for the call, so the host can hold the rest of the player
/// and lend it, inventory and all, to its slot around a shot or swing
/// ([`ServerHost::reach_player`]).
pub fn with_host<R>(p: &mut Player, cx: &mut Cx, f: impl FnOnce(&mut Gun, &mut ServerHost) -> R) -> R {
    let throw_scale = projectiles::thrower_time_scale(p, cx.pev());
    let (powerups, kungfu) = (p.active_powerup, p.kungfu);
    let mut inv = std::mem::take(&mut p.inv);
    let (r, anims, ext_changed, recover, close_cd) = {
        let Inventory { gun, sv } = &mut inv;
        sv.atk_block = gun.block_kungfu;
        let mut h = ServerHost {
            edict: cx.edict,
            sv,
            player: &mut *p,
            powerups,
            kungfu,
            throw_scale,
            anims: Vec::new(),
            ext_changed: false,
            stunt_recover: None,
            close_attack_cd: None,
        };
        let r = f(gun, &mut h);
        (r, h.anims, h.ext_changed, h.stunt_recover, h.close_attack_cd)
    };
    p.inv = inv;
    // TS writes these before its `Animate` call (`KatanaFire` 0x736bf, `CCombatAttack` 0x7653e).
    if let Some(t) = recover {
        p.stunt.recover = t;
    }
    if let Some(t) = close_cd {
        p.stunt.close_attack_cd = t;
    }
    if ext_changed {
        let ext2 = p.inv.sv.anim_ext.clone();
        crate::stunt::set_anim_ext(p, idle_override(p.inv.gun.current), &ext2);
    }
    for a in anims {
        crate::stunt::animate(p, cx, a);
    }
    r
}

/// `CBasePlayerWeapon::DefaultDeploy` 0xa0178: the idle-gait override (`m_szAnimExtention`,
/// pl+0x478) for the blades and the briefcase; empty for everything else.
fn idle_override(id: u8) -> &'static str {
    use shared::weapons::id;
    match id {
        id::KNIFE | id::SEAL_KNIFE => "knife_idle",
        id::BRIEFCASE => "briefcase_idle",
        id::KATANA => "katana_idle",
        _ => "",
    }
}

impl ServerHost<'_> {
    fn pev(&self) -> &mut entvars_t {
        self.edict.v()
    }

    /// Runs `f`, a call that reaches other entities (a shot's or swing's `TakeDamage`), with the
    /// player whole again (`gun` and `sv` back in its inventory) and lent to its slot
    /// (`world::lend`): TS's weapon frame runs inside the player, so whatever the hit sets off can
    /// change that player at once, `game_player_team` included (a shot `func_breakable` or
    /// button targeting it), even killing it, which holsters and empties the very gun that is
    /// firing. The gun and `sv` come back with those changes and the attack carries on with them.
    fn reach_player<R>(&mut self, gun: &mut Gun, f: impl FnOnce() -> R) -> R {
        std::mem::swap(&mut self.player.inv.gun, gun);
        std::mem::swap(&mut self.player.inv.sv, self.sv);
        let r = crate::world::lend(self.edict, &mut *self.player, f);
        std::mem::swap(&mut self.player.inv.gun, gun);
        std::mem::swap(&mut self.player.inv.sv, self.sv);
        r
    }
}

/// `EMIT_SOUND_DYN` with TS's usual parameters.
pub fn emit(e: *mut edict_t, chan: u32, sample: &CStr, vol: f32, attn: f32, pitch: i32) {
    engine().emit_sound(e, chan as i32, sample, vol, attn, 0, pitch);
}

/// `PLAYBACK_EVENT_FULL(FEV_NOTHOST, player, ev, 0, zero, zero, ...)`. `bparam`s are passed as
/// ints (TS sends the seed in `bparam2`).
pub fn playback(invoker: *mut edict_t, ev: Event, a: EventArgs) {
    let idx = assets::event_index(ev);
    if idx == 0 {
        return;
    }
    engine().playback_weapon_event(1, invoker, idx, a.f1, a.f2, a.i1, a.i2, a.b1, a.b2);
}

impl GunHost for ServerHost<'_> {
    fn time(&self) -> f32 {
        engine().time()
    }
    fn slow_mul(&self) -> f32 {
        shared::weapons::spread::fixed_slow_amt(self.pev().fuser1)
    }
    fn arith(&self) -> shared::pm::Arith {
        crate::rules::cvars::pm_arith()
    }
    fn powerups(&self) -> u16 {
        self.powerups
    }
    fn kungfu(&self) -> bool {
        self.kungfu
    }
    fn alive(&self) -> bool {
        let v = self.pev();
        v.deadflag == DEAD_NO as i32 && v.health > 0.0
    }
    fn buttons(&self) -> u32 {
        self.pev().button as u32
    }
    fn pressed(&self) -> u32 {
        self.sv.pressed
    }
    fn origin(&self) -> Vec3 {
        self.pev().origin
    }
    fn view_ofs(&self) -> Vec3 {
        self.pev().view_ofs
    }
    fn velocity(&self) -> Vec3 {
        self.pev().velocity
    }
    fn v_angle(&self) -> Vec3 {
        self.pev().v_angle
    }
    fn flags(&self) -> i32 {
        self.pev().flags
    }
    fn waterlevel(&self) -> i32 {
        self.pev().waterlevel
    }
    fn iuser4(&self) -> i32 {
        self.pev().iuser4
    }
    fn set_iuser4(&mut self, v: i32) {
        self.pev().iuser4 = v;
    }
    fn seed(&self) -> u32 {
        self.sv.seed
    }
    fn random_long(&mut self, lo: i32, hi: i32) -> i32 {
        engine().random_long(lo, hi)
    }

    fn fov(&self) -> f32 {
        // The player spawns with fov 90 (= default); TS keeps 0 for "not zoomed".
        let f = self.pev().fov;
        if f >= 90.0 { 0.0 } else { f }
    }
    fn set_fov(&mut self, fov: f32) {
        self.pev().fov = fov;
    }
    fn set_view_model(&mut self, id: Option<u8>) {
        let s = id.and_then(|id| assets::with(|a| a.v_model[id as usize])).unwrap_or(0);
        self.pev().viewmodel = s;
    }
    fn view_model_hidden(&self) -> bool {
        self.pev().viewmodel == 0
    }
    fn set_weapon_model(&mut self, model: Option<(u8, bool)>) {
        let s = model
            .and_then(|(id, sil)| {
                assets::with(|a| {
                    let ps = a.p_model_sil[id as usize];
                    if sil && ps != 0 { ps } else { a.p_model[id as usize] }
                })
            })
            .unwrap_or(0);
        self.pev().weaponmodel = s;
    }
    fn set_anim_extension(&mut self, ext: &str) {
        self.sv.anim_ext = ext.to_owned();
        self.ext_changed = true;
    }

    fn event(&mut self, ev: Event, args: EventArgs) {
        assets::debug(|| {
            format!("event {:?} f1={} f2={} i1={} i2={} b1={} b2={}", ev, args.f1, args.f2, args.i1, args.i2, args.b1, args.b2)
        });
        playback(self.edict, ev, args);
    }
    fn weapon_anim(&mut self, anim: i32, skip_local: bool, body: i32) {
        let pev = self.pev();
        pev.weaponanim = anim;
        if pev.flags & FL_FAKECLIENT as i32 != 0 {
            return;
        }
        let e = engine();
        if skip_local && e.can_skip_player(self.edict) {
            return;
        }
        e.send_weapon_anim(self.edict, anim, body);
    }
    fn player_anim(&mut self, anim: i32) {
        // CTSAnimation (movement package), applied by `with_host` after the gun call.
        if anim >= 0 {
            self.anims.push(anim);
        }
    }
    fn kick(&mut self, pitch: f32, yaw: f32) {
        // VKickback: the server writes pev->punchangle (pev+0x68) only for bots; humans get the
        // same kick from their client's fire-event handler.
        let pev = self.pev();
        if pev.flags & FL_FAKECLIENT as i32 != 0 {
            pev.punchangle[0] -= pitch;
            pev.punchangle[1] += yaw;
        }
    }
    fn sound(&mut self, s: Sound) {
        let e = engine();
        let ed = self.edict;
        match s {
            Sound::Switch => {
                let vol = e.random_float(0.92, 1.0);
                emit(ed, CHAN_ITEM, c"weapons/switch.wav", vol, 1.25, 98 + e.random_long(0, 3));
            }
            Sound::KevlarHit => emit(ed, CHAN_BODY, c"player/kevlarhit.wav", 1.0, 0.8, 60 + e.random_long(0, 31)),
            Sound::KnifeHitBody => emit(ed, CHAN_BODY, c"weapons/knife/knife_hitbody.wav", 1.0, 0.8, 95 + e.random_long(0, 31)),
            Sound::KnifeMiss => emit(ed, CHAN_BODY, c"weapons/knife/knife_miss.wav", 1.0, 0.8, 95 + e.random_long(0, 31)),
            Sound::KatanaHitBody => emit(ed, CHAN_BODY, c"weapons/katana/hit-body.wav", 1.0, 0.8, 95 + e.random_long(0, 31)),
            Sound::KatanaMiss => emit(ed, CHAN_BODY, c"weapons/katana/miss.wav", 1.0, 0.8, 95 + e.random_long(0, 31)),
        }
    }

    fn shoot(&mut self, shot: &Shot, gun: &mut Gun) {
        let me = self.edict;
        self.reach_player(gun, || fire::shoot_bullets(me, shot));
    }
    fn melee(&mut self, hit: &MeleeHit, gun: &mut Gun) -> MeleeResult {
        // CloseCombatAttack / ColdCockAttack start with Animate(anim) (-1 = none).
        self.player_anim(hit.player_anim);
        let (me, block) = (self.edict, self.sv.atk_block);
        self.reach_player(gun, || melee::attack(me, hit, block))
    }
    fn throw_grenade(&mut self, origin: Vec3, velocity: Vec3, roll: bool) {
        assets::debug(|| format!("throw grenade roll={roll} vel={velocity:?}"));
        projectiles::create_grenade(self.edict, origin, velocity, roll, self.throw_scale);
    }
    fn throw_blade(&mut self, btype: i32) {
        projectiles::throw_blade(self.edict, btype, self.throw_scale);
    }
    fn deploy_update_now(&mut self) {
        self.sv.next_update = 0.0;
    }
    fn slots_changed(&mut self, delta_free: i32) {
        self.sv.free_slots += delta_free;
    }
    fn melee_bar_changed(&mut self, bar: f32) {
        net::send_kfu_power(self.edict, bar);
    }
    fn block_player(&mut self, t: f32) {
        self.sv.block_timer = t;
        self.pev().iuser4 |= 1;
    }
    fn set_close_combat_cooldown(&mut self, t: f32) {
        self.close_attack_cd = Some(t);
    }
    fn melee_crouch(&mut self, bits: i32) {
        let pev = self.pev();
        pev.iuser4 |= bits;
        pev.view_ofs = [0.0, 0.0, 16.0];
        self.stunt_recover = Some(2.0);
    }
    fn leave_wall_climb(&mut self) {
        let pev = self.pev();
        pev.view_ofs = [0.0, 0.0, if pev.flags & FL_DUCKING as i32 != 0 { 16.0 } else { 28.0 }];
        pev.gravity = 1.0;
        pev.friction = 1.0;
    }
}
