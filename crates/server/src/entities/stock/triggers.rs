//! Brush triggers and logic entities (SDK triggers.cpp; TS keeps them as in SDK 2.3):
//! `trigger_multiple`, `trigger_once`, `trigger_hurt`, `trigger_teleport`, `trigger_push`,
//! `trigger_gravity`, `trigger_auto`, `trigger_relay`, `trigger_changetarget`, `trigger_camera`,
//! `multi_manager`, `func_ladder`, `func_friction`; and the #95 `CBaseTrigger` kinds `trigger`,
//! `trigger_autosave`, `trigger_cdaudio`, `trigger_counter`, `trigger_endsection`,
//! `trigger_monsterjump`, plus `target_cdaudio`.

use super::base::*;
use crate::entities::{fire_targets, remove};
use crate::messages::{self, Msg};
use crate::world::{self, Cx, Entity, UseType};
use goldsrc::Global;
use goldsrc::edict::EdictPtr;
use goldsrc::ext::entities::{ATTN_NORM, EHandle, is_player, vec};
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::cell::RefCell;
use std::ffi::CStr;

pub const SF_TRIGGER_ALLOWMONSTERS: i32 = 1;
pub const SF_TRIGGER_NOCLIENTS: i32 = 2;
pub const SF_TRIGGER_PUSHABLES: i32 = 4;

pub const SF_TRIGGER_HURT_TARGETONCE: i32 = 1;
pub const SF_TRIGGER_HURT_START_OFF: i32 = 2;
pub const SF_TRIGGER_HURT_NO_CLIENTS: i32 = 8;
pub const SF_TRIGGER_HURT_CLIENTONLYFIRE: i32 = 16;
pub const SF_TRIGGER_HURT_CLIENTONLYTOUCH: i32 = 32;

pub const SF_TRIG_PUSH_ONCE: i32 = 1;
pub const SF_TRIGGER_PUSH_START_OFF: i32 = 2;

fn show_triggers() -> bool {
    engine().cvar_float(c"showtriggers") != 0.0
}

/// `CBaseTrigger::InitTrigger` (0xe67ec).
pub(super) fn init_trigger(cx: &mut Cx) {
    let v = cx.pev();
    if v.angles != [0.0; 3] {
        set_movedir(v);
    }
    v.solid = SOLID_TRIGGER as i32;
    v.movetype = MOVETYPE_NONE as i32;
    set_own_model(cx);
    if !show_triggers() {
        cx.pev().effects |= EF_NODRAW as i32;
    }
}

/// `CBaseTrigger::ToggleUse` (0xe7368).
fn toggle_use(cx: &mut Cx) {
    let eng = engine();
    let v = cx.pev();
    if v.solid == SOLID_NOT as i32 {
        v.solid = SOLID_TRIGGER as i32;
        eng.force_retouch();
    } else {
        v.solid = SOLID_NOT as i32;
    }
    let org = cx.pev().origin;
    eng.set_origin(cx.edict, org);
}

/// `DMG_RADIATION`: trigger_hurt with this type also drives the geiger counter.
pub const DMG_RADIATION: i32 = 1 << 18;

/// The `CBasePlayer` geiger members (`m_flgeigerRange` +0x984, `m_flgeigerDelay` +0x988,
/// `m_igeigerRangePrev` +0x98c), per client slot.
#[derive(Clone, Copy)]
struct Geiger {
    range: f32,
    next: f32,
    last: i32,
}

const GEIGER_IDLE: Geiger = Geiger { range: 1000.0, next: 0.0, last: 1000 };
static GEIGER: Global<RefCell<[Geiger; 33]>> = Global(RefCell::new([GEIGER_IDLE; 33]));

/// Geiger state for a (re)spawned player: `CBasePlayer::Precache` (0xda90c) resets the range,
/// `Spawn` (0x8f568) delays the first update by 2 s.
pub fn geiger_spawn(player: *mut edict_t) {
    let eng = engine();
    let now = eng.time();
    if let Some(g) = GEIGER.0.borrow_mut().get_mut(eng.index_of(player) as usize) {
        *g = Geiger { next: now + 2.0, ..GEIGER_IDLE };
    }
}

/// `CBasePlayer::UpdateGeigerCounter` (0xda1f0), run from `CheckSuitUpdate` in PreThink for
/// players with the HEV suit bit: every 0.25 s sends `Geiger` (range / 4) when it changed, then
/// lets the range decay back to 1000 (one chance in four), so it only stays low while a
/// radiation trigger keeps lowering it.
pub fn update_geiger_counter(player: *mut edict_t) {
    if player.is_null() || player.v().weapons >= 0 {
        return;
    }
    let eng = engine();
    let now = eng.time();
    let send = {
        let mut table = GEIGER.0.borrow_mut();
        let Some(g) = table.get_mut(eng.index_of(player) as usize) else { return };
        if g.next > now {
            return;
        }
        g.next = now + 0.25;
        let value = ((g.range * 0.25) as i32 as i16) as u8 as i32;
        let changed = value != g.last;
        g.last = value;
        if eng.random_long(0, 3) == 0 {
            g.range = 1000.0;
        }
        changed.then_some(value)
    };
    if let Some(value) = send {
        messages::one(Msg::Geiger, player).byte(value).end();
    }
}

/// `UTIL_ShowMessage`: `HudText` to one player.
pub fn show_message(text: &CStr, player: *mut edict_t) {
    if !is_player(player) {
        return;
    }
    messages::one(Msg::HudText, player).string(text).end();
}

/// `CBaseTrigger` and its brush subclasses. The kind is decided by the link type.
#[derive(Default)]
pub struct Trigger<const KIND: u8> {
    t: Toggle<Self>,
    /// `m_cTriggersLeft` (+0xcc), the `count` key.
    triggers_left: i32,
    damage_inflict: i32,
    /// `m_pfnUse`: `ToggleUse` (hurt, push, monsterjump), `CounterUse`, `EndSectionUse`.
    use_fn: Option<fn(&mut Self, &mut Cx, *mut edict_t)>,
}

pub const K_MULTIPLE: u8 = 0;
pub const K_ONCE: u8 = 1;
pub const K_HURT: u8 = 2;
pub const K_TELEPORT: u8 = 3;
pub const K_PUSH: u8 = 4;
pub const K_GRAVITY: u8 = 5;
/// `trigger`: a bare `CBaseTrigger` (its Spawn is `CBaseEntity::Spawn`, which does nothing).
pub const K_BASE: u8 = 6;
/// `trigger_autosave` (`CTriggerSave`).
pub const K_AUTOSAVE: u8 = 7;
/// `trigger_cdaudio` (`CTriggerCDAudio`).
pub const K_CDAUDIO: u8 = 8;
/// `trigger_counter` (`CTriggerCounter`).
pub const K_COUNTER: u8 = 9;
/// `trigger_endsection` (`CTriggerEndSection`).
pub const K_ENDSECTION: u8 = 10;
/// `trigger_monsterjump` (`CTriggerMonsterJump`).
pub const K_MONSTERJUMP: u8 = 11;

pub const SF_COUNTER_NOMESSAGE: i32 = 1;
pub const SF_ENDSECTION_USEONLY: i32 = 1;

/// `g_pGameRules->IsDeathmatch()`: false only for `CHalfLifeRules`, which `InstallGameRules`
/// (0xc6050) builds when `deathmatch` is 0.
pub(super) fn rules_deathmatch() -> bool {
    !crate::rules::state::with(|g| g.single_player)
}

/// The console text `CBaseTrigger::CounterUse` (0xe79b0) prints with `left` triggers still to
/// go (no newline, as in TS).
fn counter_message(left: i32) -> &'static str {
    match left {
        0 => "Sequence completed!",
        1 => "Only 1 more to go...",
        2 => "Only 2 more to go...",
        3 => "Only 3 more to go...",
        _ => "There are more to go...",
    }
}

impl<const KIND: u8> Mover for Trigger<KIND> {
    fn toggle(&mut self) -> &mut Toggle<Self> {
        &mut self.t
    }
}

impl<const KIND: u8> Trigger<KIND> {
    fn multi_touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        let ov = other.v();
        let sf = cx.pev().spawnflags;
        if (ov.flags & FL_CLIENT as i32 != 0 && sf & SF_TRIGGER_NOCLIENTS == 0)
            || (ov.flags & FL_MONSTER as i32 != 0 && sf & SF_TRIGGER_ALLOWMONSTERS != 0)
            || (sf & SF_TRIGGER_PUSHABLES != 0 && s(ov.classname) == c"func_pushable")
        {
            self.activate_multi_trigger(cx, other);
        }
    }

    fn activate_multi_trigger(&mut self, cx: &mut Cx, activator: *mut edict_t) {
        let now = engine().time();
        if cx.pev().nextthink > now {
            return; // still waiting for reset time
        }
        if !is_master_triggered(self.t.master, activator) {
            return;
        }
        let me = cx.edict;
        let noise = cx.pev().noise;
        if noise != 0 {
            emit(me, CHAN_VOICE, s(noise), 1.0, ATTN_NORM, 0, PITCH_NORM as i32);
        }
        self.t.activator = EHandle::new(activator);
        self.t.use_targets(me, activator, UseType::Toggle, 0.0);
        let msg = cx.pev().message;
        if msg != 0 && is_player(activator) {
            show_message(s(msg), activator);
        }
        if self.t.wait > 0.0 {
            self.t.think = Some(Self::multi_wait_over);
            cx.pev().nextthink = now + self.t.wait;
        } else {
            self.t.touch = None;
            cx.pev().nextthink = now + 0.1;
            self.t.think = Some(Self::sub_remove);
        }
    }

    fn multi_wait_over(&mut self, _cx: &mut Cx) {
        self.t.think = None;
    }

    fn sub_remove(&mut self, cx: &mut Cx) {
        remove(cx.edict);
    }

    fn toggle_use_fn(&mut self, cx: &mut Cx, _activator: *mut edict_t) {
        toggle_use(cx);
    }

    /// `CBaseTrigger::CounterUse` (0xe79b0): counts `m_cTriggersLeft` down and runs
    /// `ActivateMultiTrigger` when it reaches 0. A `player` activator gets the progress text on
    /// the server console unless `SF_COUNTER_NOMESSAGE`.
    fn counter_use(&mut self, cx: &mut Cx, activator: *mut edict_t) {
        self.triggers_left = self.triggers_left.wrapping_sub(1);
        self.t.activator = EHandle::new(activator);
        if self.triggers_left < 0 {
            return;
        }
        // TS reads the classname through `m_hActivator->` and crashes on a NULL (or object-less)
        // activator; the port treats it as not a player.
        let a = self.t.activator.get();
        let message =
            !a.is_null() && a.has_private_data() && s(a.v().classname) == c"player" && cx.pev().spawnflags & SF_COUNTER_NOMESSAGE == 0;
        if message {
            engine().alert(ALERT_TYPE::at_console, counter_message(self.triggers_left));
        }
        if self.triggers_left == 0 {
            self.activate_multi_trigger(cx, a);
        }
    }

    /// `CTriggerSave::SaveTouch` (0xe94c8).
    fn save_touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        if !is_master_triggered(self.t.master, other) || !is_player(other) {
            return;
        }
        self.t.touch = None;
        remove(cx.edict);
        engine().server_command("autosave\n");
    }

    /// `CTriggerCDAudio::PlayTrack` (0xe6cd8): the track is `(int)health`.
    fn play_track(&mut self, cx: &mut Cx) {
        play_cd_track(cx.pev().health as i32);
        self.t.touch = None;
        remove(cx.edict);
    }

    /// `CTriggerEndSection::EndSectionUse` (0xe9560): a net client (player) or no activator.
    fn end_section_use(&mut self, cx: &mut Cx, activator: *mut edict_t) {
        if !activator.is_null() && !is_player(activator) {
            return;
        }
        self.use_fn = None;
        Self::end_section(cx);
    }

    /// `CTriggerEndSection::EndSectionTouch` (0xe967c).
    fn end_section_touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        if !is_player(other) {
            return;
        }
        self.t.touch = None;
        Self::end_section(cx);
    }

    /// `pfnEndSection(section)` when there is one, then `UTIL_Remove`.
    fn end_section(cx: &mut Cx) {
        let section = cx.pev().message;
        if section != 0 {
            engine().end_section(s(section));
        }
        remove(cx.edict);
    }

    /// `CTriggerMonsterJump::Touch` (0xe6ae4; a virtual override, so `SetTouch` doesn't matter):
    /// a monster is lifted 1 unit (no `SetOrigin`) and thrown along `movedir` at `speed`, plus
    /// `m_flHeight` up; the trigger turns itself off on its next think.
    fn monster_jump_touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        let ov = other.v();
        if ov.flags & FL_MONSTER as i32 == 0 {
            return;
        }
        ov.origin[2] += 1.0;
        if ov.flags & FL_ONGROUND as i32 != 0 {
            ov.flags &= !(FL_ONGROUND as i32);
        }
        let v = cx.pev();
        ov.velocity = vec::scale(v.movedir, v.speed);
        ov.velocity[2] += self.t.height;
        v.nextthink = engine().time();
    }

    /// `CTriggerMonsterJump::Think` (0xe6a9c; a virtual override).
    fn monster_jump_think(&mut self, cx: &mut Cx) {
        cx.pev().solid = SOLID_NOT as i32;
        let org = cx.pev().origin;
        engine().set_origin(cx.edict, org);
        self.t.think = None;
    }

    /// `CTriggerHurt::RadiationThink` (0xe703c): the nearest player who can see the trigger's
    /// centre gets their geiger range lowered to the distance (read by `update_geiger_counter`).
    fn radiation_think(&mut self, cx: &mut Cx) {
        let eng = engine();
        // The PVS test runs from the box centre: move origin there (and zero view_ofs) for it.
        let v = cx.pev();
        let (origin, view_ofs) = (v.origin, v.view_ofs);
        let center = vec::mid(v.absmin, v.absmax);
        v.origin = center;
        v.view_ofs = [0.0; 3];
        let player = eng.find_client_in_pvs(cx.edict);
        let v = cx.pev();
        v.origin = origin;
        v.view_ofs = view_ofs;
        if !player.is_null() {
            let pv = player.v();
            let range = vec::length(vec::sub(center, vec::mid(pv.absmin, pv.absmax)));
            let i = eng.index_of(player) as usize;
            if let Some(g) = GEIGER.0.borrow_mut().get_mut(i) {
                g.range = g.range.min(range);
            }
        }
        cx.pev().nextthink = eng.time() + 0.25;
    }

    /// `CBaseTrigger::HurtTouch` (SDK 2.3 multiplayer path; negative damage heals).
    fn hurt_touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        let eng = engine();
        let now = eng.time();
        let ov = other.v();
        if ov.takedamage == 0.0 {
            return;
        }
        let sf = cx.pev().spawnflags;
        let player = is_player(other);
        if sf & SF_TRIGGER_HURT_CLIENTONLYTOUCH != 0 && !player {
            return;
        }
        if sf & SF_TRIGGER_HURT_NO_CLIENTS != 0 && player {
            return;
        }
        let v = cx.pev();
        // Single player (`!IsMultiplayer()`) has no per-player mask: nothing is hurt again before
        // `dmgtime` except in the frame that started it.
        if crate::rules::state::single_player() {
            if v.dmgtime > now && now != v.pain_finished {
                return;
            }
        } else if v.dmgtime > now {
            if now != v.pain_finished {
                if player {
                    let mask = 1i32.wrapping_shl((eng.index_of(other) - 1) as u32);
                    if v.impulse & mask != 0 {
                        return;
                    }
                    v.impulse |= mask;
                } else {
                    return;
                }
            }
        } else {
            v.impulse = 0;
            if player {
                v.impulse |= 1i32.wrapping_shl((eng.index_of(other) - 1) as u32);
            }
        }
        let fldmg = v.dmg * 0.5;
        let me = cx.edict;
        if fldmg < 0.0 {
            super::items::take_health(other, -fldmg);
        } else {
            crate::damage::apply(other, &crate::damage::DamageInfo::new(me, me, fldmg, self.damage_inflict));
        }
        let v = cx.pev();
        v.pain_finished = now;
        v.dmgtime = now + 0.5;
        if v.target != 0 {
            if sf & SF_TRIGGER_HURT_CLIENTONLYFIRE != 0 && !player {
                return;
            }
            self.t.use_targets(me, other, UseType::Toggle, 0.0);
            if sf & SF_TRIGGER_HURT_TARGETONCE != 0 {
                cx.pev().target = 0;
            }
        }
    }

    fn teleport_touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        let eng = engine();
        let ov = other.v();
        if ov.flags & (FL_CLIENT | FL_MONSTER) as i32 == 0 {
            return;
        }
        if !is_master_triggered(self.t.master, other) {
            return;
        }
        let sf = cx.pev().spawnflags;
        if sf & SF_TRIGGER_ALLOWMONSTERS == 0 && ov.flags & FL_MONSTER as i32 != 0 {
            return;
        }
        let player = is_player(other);
        if sf & SF_TRIGGER_NOCLIENTS != 0 && player {
            return;
        }
        let target = eng.find_by_target_name(std::ptr::null_mut(), s(cx.pev().target));
        if target.is_null() {
            return;
        }
        let tv = target.v();
        let mut tmp = tv.origin;
        if player {
            tmp[2] -= ov.mins[2];
        }
        tmp[2] += 1.0;
        ov.flags &= !(FL_ONGROUND as i32);
        eng.set_origin(other, tmp);
        let ov = other.v();
        ov.angles = tv.angles;
        if player {
            ov.v_angle = tv.angles;
        }
        ov.fixangle = 1;
        ov.velocity = [0.0; 3];
        ov.basevelocity = [0.0; 3];
    }

    fn push_touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        let ov = other.v();
        if [MOVETYPE_NONE, MOVETYPE_PUSH, MOVETYPE_NOCLIP, MOVETYPE_FOLLOW].contains(&(ov.movetype as u32)) {
            return;
        }
        if ov.solid == SOLID_NOT as i32 || ov.solid == SOLID_BSP as i32 {
            return;
        }
        let v = cx.pev();
        let push = vec::scale(v.movedir, v.speed);
        if v.spawnflags & SF_TRIG_PUSH_ONCE != 0 {
            ov.velocity = vec::add(ov.velocity, push);
            if ov.velocity[2] > 0.0 {
                ov.flags &= !(FL_ONGROUND as i32);
            }
            remove(cx.edict);
        } else {
            let mut p = push;
            if ov.flags & FL_BASEVELOCITY as i32 != 0 {
                p = vec::add(p, ov.basevelocity);
            }
            ov.basevelocity = p;
            ov.flags |= FL_BASEVELOCITY as i32;
        }
    }

    fn gravity_touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        if !is_player(other) {
            return;
        }
        let g = cx.pev().gravity;
        (other.v().gravity = g);
    }
}

impl<const KIND: u8> Entity for Trigger<KIND> {
    /// `CBaseTrigger::KeyValue` (0xe68bc); `CTriggerEndSection::KeyValue` (0xe96fc) takes
    /// `section` first.
    fn key_value(&mut self, cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"section" if KIND == K_ENDSECTION => cx.pev().message = alloc(value),
            b"damage" => cx.pev().dmg = kv_f(value),
            b"count" => self.triggers_left = kv_f(value) as i32,
            b"damagetype" => self.damage_inflict = kv_i(value),
            _ => return self.t.key_value(key, value),
        }
        true
    }

    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        match KIND {
            K_MULTIPLE | K_ONCE => {
                if KIND == K_ONCE {
                    self.t.wait = -1.0;
                }
                if self.t.wait == 0.0 {
                    self.t.wait = 0.2;
                }
                init_trigger(cx);
                self.t.touch = Some(Self::multi_touch);
            }
            K_HURT => {
                init_trigger(cx);
                self.t.touch = Some(Self::hurt_touch);
                if cx.pev().targetname != 0 {
                    self.use_fn = Some(Self::toggle_use_fn);
                }
                if self.damage_inflict & DMG_RADIATION != 0 {
                    self.t.think = Some(Self::radiation_think);
                    cx.pev().nextthink = eng.time() + eng.random_float(0.0, 0.5);
                }
                if cx.pev().spawnflags & SF_TRIGGER_HURT_START_OFF != 0 {
                    cx.pev().solid = SOLID_NOT as i32;
                }
                let org = cx.pev().origin;
                eng.set_origin(cx.edict, org);
            }
            K_TELEPORT => {
                init_trigger(cx);
                self.t.touch = Some(Self::teleport_touch);
            }
            K_PUSH => {
                if cx.pev().angles == [0.0; 3] {
                    cx.pev().angles[1] = 360.0;
                }
                init_trigger(cx);
                let v = cx.pev();
                if v.speed == 0.0 {
                    v.speed = 100.0;
                }
                if v.spawnflags & SF_TRIGGER_PUSH_START_OFF != 0 {
                    v.solid = SOLID_NOT as i32;
                }
                self.use_fn = Some(Self::toggle_use_fn);
                self.t.touch = Some(Self::push_touch);
                let org = cx.pev().origin;
                eng.set_origin(cx.edict, org);
            }
            K_GRAVITY => {
                init_trigger(cx);
                self.t.touch = Some(Self::gravity_touch);
            }
            // CTriggerSave::Spawn (0xe945c): REMOVE_ENTITY under the multiplayer rules.
            K_AUTOSAVE => {
                if rules_deathmatch() {
                    remove(cx.edict);
                    return;
                }
                init_trigger(cx);
                self.t.touch = Some(Self::save_touch);
            }
            // CTriggerCDAudio::Spawn (0xe6bec).
            K_CDAUDIO => init_trigger(cx),
            // CTriggerCounter::Spawn (0xe7b54): no InitTrigger, so no model and not solid.
            K_COUNTER => {
                self.t.wait = -1.0;
                if self.triggers_left == 0 {
                    self.triggers_left = 2;
                }
                self.use_fn = Some(Self::counter_use);
            }
            // CTriggerEndSection::Spawn (0xe95e4): REMOVE_ENTITY under the multiplayer rules.
            K_ENDSECTION => {
                if rules_deathmatch() {
                    remove(cx.edict);
                    return;
                }
                init_trigger(cx);
                self.use_fn = Some(Self::end_section_use);
                if cx.pev().spawnflags & SF_ENDSECTION_USEONLY == 0 {
                    self.t.touch = Some(Self::end_section_touch);
                }
            }
            // CTriggerMonsterJump::Spawn (0xe6a0c): SetMovedir even for zero angles, then
            // InitTrigger (which then sees zero angles and leaves movedir alone).
            K_MONSTERJUMP => {
                set_movedir(cx.pev());
                init_trigger(cx);
                let v = cx.pev();
                v.nextthink = 0.0;
                v.speed = 200.0;
                self.t.height = 150.0;
                if v.targetname != 0 {
                    v.solid = SOLID_NOT as i32;
                    let org = v.origin;
                    eng.set_origin(cx.edict, org);
                    self.use_fn = Some(Self::toggle_use_fn);
                }
            }
            // `trigger` (K_BASE): CBaseEntity::Spawn.
            _ => {}
        }
    }

    fn think(&mut self, cx: &mut Cx) {
        if KIND == K_MONSTERJUMP {
            self.monster_jump_think(cx);
        } else {
            run_think(self, cx);
        }
    }

    fn touch(&mut self, cx: &mut Cx, other: *mut edict_t) {
        match KIND {
            // CTriggerCDAudio::Touch (0xe6bb0), a virtual override.
            K_CDAUDIO => {
                if is_player(other) {
                    self.play_track(cx);
                }
            }
            K_MONSTERJUMP => self.monster_jump_touch(cx, other),
            _ => run_touch(self, cx, other),
        }
    }

    fn use_(&mut self, cx: &mut Cx, activator: *mut edict_t, _caller: *mut edict_t, _kind: UseType, _value: f32) {
        if KIND == K_CDAUDIO {
            // CTriggerCDAudio::Use (0xe6c10), a virtual override.
            self.play_track(cx);
        } else if let Some(f) = self.use_fn {
            f(self, cx, activator);
        }
    }
}

// ---- PlayCDTrack / target_cdaudio ------------------------------------------------------------------------

/// The client command `PlayCDTrack` (0xe6c34) sends for `track`: -1 pauses, 0..=30 plays,
/// anything else is out of range (`None`).
fn cd_command(track: i32) -> Option<String> {
    if (track as u32).wrapping_add(1) >= 0x20 {
        return None;
    }
    Some(if track == -1 { "cd pause\n".to_owned() } else { format!("cd play {track:3}\n") })
}

/// `PlayCDTrack` (0xe6c34): a `cd` command to the client in slot 1 (single-player code: other
/// players hear nothing).
pub fn play_cd_track(track: i32) {
    let eng = engine();
    let client = eng.entity_of_index(1);
    if client.is_null() {
        return;
    }
    match cd_command(track) {
        Some(cmd) => eng.client_command(client, &goldsrc::util::to_cstring(&cmd)),
        // TS passes no argument for the %d, so it prints whatever is on the stack; the port
        // prints the track.
        None => eng.alert(ALERT_TYPE::at_console, &format!("TriggerCDAudio - Track {track} out of range\n")),
    }
}

/// `target_cdaudio` (`CTargetCDAudio`, 0xe6d60..0xe6f08): plays its CD track (`health`) when
/// used, or once the client in slot 1 is within `radius` (checked every 0.5 s), then removes
/// itself.
#[derive(Default)]
pub struct TargetCdAudio;

impl TargetCdAudio {
    /// `CTargetCDAudio::Play` (0xe6f08).
    fn play(cx: &mut Cx) {
        play_cd_track(cx.pev().health as i32);
        remove(cx.edict);
    }
}

impl Entity for TargetCdAudio {
    /// `CTargetCDAudio::KeyValue` (0xe6d60): only `radius` (`pev->scale`).
    fn key_value(&mut self, cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        if key == c"radius" {
            cx.pev().scale = kv_f(value);
            return true;
        }
        false
    }

    /// `CTargetCDAudio::Spawn` (0xe6dc8).
    fn spawn(&mut self, cx: &mut Cx) {
        let v = cx.pev();
        v.solid = SOLID_NOT as i32;
        v.movetype = MOVETYPE_NONE as i32;
        if v.scale > 0.0 {
            v.nextthink = engine().time() + 1.0;
        }
    }

    /// `CTargetCDAudio::Think` (0xe6e48): stops thinking while slot 1 has no edict.
    fn think(&mut self, cx: &mut Cx) {
        let eng = engine();
        let client = eng.entity_of_index(1);
        if client.is_null() {
            return;
        }
        let org = client.v().origin;
        let v = cx.pev();
        v.nextthink = eng.time() + 0.5;
        if vec::length(vec::sub(org, v.origin)) <= v.scale {
            Self::play(cx);
        }
    }

    /// `CTargetCDAudio::Use` (0xe6e24).
    fn use_(&mut self, cx: &mut Cx, _a: *mut edict_t, _c: *mut edict_t, _k: UseType, _v: f32) {
        Self::play(cx);
    }
}

// ---- func_ladder ------------------------------------------------------------------------------------

/// `func_ladder`: CONTENTS_LADDER brush, invisible unless `showtriggers`.
#[derive(Default)]
pub struct Ladder;

impl Entity for Ladder {
    fn key_value(&mut self, cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        // CBaseTrigger::KeyValue (damage/count/damagetype go nowhere useful on a ladder).
        match key.to_bytes() {
            b"damage" => cx.pev().dmg = kv_f(value),
            b"count" | b"damagetype" | b"lip" | b"wait" | b"master" | b"distance" | b"delay" | b"killtarget" => {}
            _ => return false,
        }
        true
    }

    fn spawn(&mut self, cx: &mut Cx) {
        let v = cx.pev();
        v.solid = SOLID_NOT as i32;
        v.skin = CONTENTS_LADDER;
        if !show_triggers() {
            v.rendermode = kRenderTransTexture as i32;
            v.renderamt = 0.0;
        }
        v.effects &= !(EF_NODRAW as i32);
        set_own_model(cx);
        cx.pev().movetype = MOVETYPE_PUSH as i32;
    }
}

// ---- func_friction -------------------------------------------------------------------------------------

/// `func_friction`: sets the toucher's friction (`modifier` percent).
#[derive(Default)]
pub struct FrictionModifier {
    friction: f32,
}

impl Entity for FrictionModifier {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        if key == c"modifier" {
            self.friction = kv_f(value) / 100.0;
            return true;
        }
        false
    }

    fn spawn(&mut self, cx: &mut Cx) {
        let v = cx.pev();
        v.solid = SOLID_TRIGGER as i32;
        set_own_model(cx);
        cx.pev().movetype = MOVETYPE_NONE as i32;
    }

    fn touch(&mut self, _cx: &mut Cx, other: *mut edict_t) {
        let ov = other.v();
        if ov.movetype != MOVETYPE_BOUNCEMISSILE as i32 && ov.movetype != MOVETYPE_BOUNCE as i32 {
            ov.friction = self.friction;
        }
    }
}

// ---- trigger_auto / trigger_relay ---------------------------------------------------------------------

fn trigger_state(v: &CStr) -> UseType {
    match kv_i(v) {
        0 => UseType::Off,
        2 => UseType::Toggle,
        _ => UseType::On,
    }
}

pub const SF_AUTO_FIREONCE: i32 = 1;
pub const SF_RELAY_FIREONCE: i32 = 1;

pub struct AutoTrigger {
    delay: Delay,
    globalstate: string_t,
    kind: UseType,
}

impl Default for AutoTrigger {
    fn default() -> Self {
        Self { delay: Delay::default(), globalstate: 0, kind: UseType::Off }
    }
}

impl Entity for AutoTrigger {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"globalstate" => self.globalstate = alloc(value),
            b"triggerstate" => self.kind = trigger_state(value),
            _ => return self.delay.key_value(key, value),
        }
        true
    }

    fn spawn(&mut self, cx: &mut Cx) {
        cx.pev().nextthink = engine().time() + 0.1;
    }

    fn think(&mut self, cx: &mut Cx) {
        if self.globalstate == 0 || global_state(s(self.globalstate)) == GlobalState::On {
            let me = cx.edict;
            self.delay.use_targets(me, me, self.kind, 0.0);
            if cx.pev().spawnflags & SF_AUTO_FIREONCE != 0 {
                remove(me);
            }
        }
    }
}

pub struct TriggerRelay {
    delay: Delay,
    kind: UseType,
}

impl Default for TriggerRelay {
    fn default() -> Self {
        Self { delay: Delay::default(), kind: UseType::Off }
    }
}

impl Entity for TriggerRelay {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        if key == c"triggerstate" {
            self.kind = trigger_state(value);
            return true;
        }
        self.delay.key_value(key, value)
    }

    fn use_(&mut self, cx: &mut Cx, _activator: *mut edict_t, _caller: *mut edict_t, _kind: UseType, _value: f32) {
        let me = cx.edict;
        self.delay.use_targets(me, me, self.kind, 0.0);
        if cx.pev().spawnflags & SF_RELAY_FIREONCE != 0 {
            remove(me);
        }
    }
}

// ---- trigger_changetarget -------------------------------------------------------------------------------

#[derive(Default)]
pub struct ChangeTarget {
    delay: Delay,
    new_target: string_t,
}

impl Entity for ChangeTarget {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        if key == c"m_iszNewTarget" {
            self.new_target = alloc(value);
            return true;
        }
        self.delay.key_value(key, value)
    }

    fn use_(&mut self, cx: &mut Cx, _activator: *mut edict_t, _caller: *mut edict_t, _kind: UseType, _value: f32) {
        let target = s(cx.pev().target);
        let e = engine().find_by_target_name(std::ptr::null_mut(), target);
        if !e.is_null() {
            (e.v().target = self.new_target);
        }
    }
}

// ---- multi_manager ---------------------------------------------------------------------------------------

const MAX_MULTI_TARGETS: usize = 16;
const SF_MULTIMAN_THREAD: i32 = 1;
const SF_MULTIMAN_CLONE: i32 = i32::MIN;

#[derive(Default, Clone)]
pub struct MultiManager {
    targets: Vec<(string_t, f32)>,
    wait: f32,
    index: usize,
    start_time: f32,
    activator: EHandle,
    thinking: bool,
    usable: bool,
}

/// `UTIL_StripToken`: `"name#2"` -> `"name"`.
pub fn strip_token(key: &str) -> &str {
    key.split('#').next().unwrap_or("")
}

/// `CMultiManager::HasTarget`, for multisource registration.
pub fn multi_manager_has_target(e: *mut edict_t, name: &CStr) -> bool {
    world::with::<MultiManager, _>(e, |m| m.targets.iter().any(|(t, _)| s(*t) == name)).unwrap_or(false)
}

impl MultiManager {
    fn manager_use(&mut self, cx: &mut Cx, activator: *mut edict_t) {
        let sf = cx.pev().spawnflags;
        if sf & SF_MULTIMAN_CLONE == 0 && sf & SF_MULTIMAN_THREAD != 0 {
            // Clone: a copy runs this activation so the original can be re-used at once.
            let eng = engine();
            let e = eng.create_entity();
            if e.is_null() {
                return;
            }
            world::create::<MultiManager>(e.v());
            {
                let mut copy = *cx.pev();
                copy.pContainingEntity = e;
                copy.spawnflags |= SF_MULTIMAN_CLONE;
                *e.v() = copy;
            }
            let targets = self.targets.clone();
            world::with::<MultiManager, _>(e, |m| m.targets = targets);
            world::dispatch(e, |ent, ecx| {
                if let Some(m) = (ent as &mut dyn std::any::Any).downcast_mut::<MultiManager>() {
                    m.manager_use(ecx, activator);
                }
            });
            return;
        }
        self.activator = EHandle::new(activator);
        self.index = 0;
        self.start_time = engine().time();
        self.usable = false;
        self.thinking = true;
        cx.pev().nextthink = self.start_time;
    }

    fn manager_think(&mut self, cx: &mut Cx) {
        let now = engine().time();
        let t = now - self.start_time;
        let me = cx.edict;
        while self.index < self.targets.len() && self.targets[self.index].1 <= t {
            let (name, _) = self.targets[self.index];
            fire_targets(s(name), self.activator.get(), me, UseType::Toggle, 0.0);
            self.index += 1;
        }
        if self.index >= self.targets.len() {
            self.thinking = false;
            if cx.pev().spawnflags & SF_MULTIMAN_CLONE != 0 {
                remove(me);
                return;
            }
            self.usable = true;
        } else {
            cx.pev().nextthink = self.start_time + self.targets[self.index].1;
        }
    }
}

impl Entity for MultiManager {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        // Every key that isn't an entvars field (those never get here) or `wait` is a target.
        if key == c"wait" {
            self.wait = kv_f(value);
            return true;
        }
        if self.targets.len() < MAX_MULTI_TARGETS {
            let name = strip_token(kv_str(key));
            self.targets.push((alloc(&goldsrc::util::to_cstring(name)), kv_f(value)));
            return true;
        }
        false
    }

    fn spawn(&mut self, cx: &mut Cx) {
        cx.pev().solid = SOLID_NOT as i32;
        self.usable = true;
        // Bubble sort by delay (stable, like the SDK).
        let mut swapped = true;
        while swapped {
            swapped = false;
            for i in 1..self.targets.len() {
                if self.targets[i].1 < self.targets[i - 1].1 {
                    self.targets.swap(i, i - 1);
                    swapped = true;
                }
            }
        }
    }

    fn think(&mut self, cx: &mut Cx) {
        if self.thinking {
            self.manager_think(cx);
        }
    }

    fn use_(&mut self, cx: &mut Cx, activator: *mut edict_t, _caller: *mut edict_t, _kind: UseType, _value: f32) {
        if self.usable {
            self.manager_use(cx, activator);
        }
    }
}

// ---- trigger_camera --------------------------------------------------------------------------------------

pub const SF_CAMERA_PLAYER_POSITION: i32 = 1;
pub const SF_CAMERA_PLAYER_TARGET: i32 = 2;
pub const SF_CAMERA_PLAYER_TAKECONTROL: i32 = 4;

#[derive(Default)]
pub struct TriggerCamera {
    delay: Delay,
    player: EHandle,
    target: EHandle,
    path: EHandle,
    path_name: string_t,
    wait: f32,
    return_time: f32,
    stop_time: f32,
    move_distance: f32,
    target_speed: f32,
    initial_speed: f32,
    acceleration: f32,
    deceleration: f32,
    state: bool,
    following: bool,
}

/// `EnableControl(on)` on a player: FL_FROZEN.
pub fn enable_control(p: *mut edict_t, on: bool) {
    if is_player(p) {
        let v = p.v();
        if on {
            v.flags &= !(FL_FROZEN as i32);
        } else {
            v.flags |= FL_FROZEN as i32;
        }
    }
}

/// `path_corner` delay (`GetDelay`), 0 for anything else.
fn path_delay(e: *mut edict_t) -> f32 {
    world::with::<super::trains::PathCorner, _>(e, |p| p.wait).unwrap_or(0.0)
}

fn next_target(e: *mut edict_t) -> *mut edict_t {
    let t = e.v().target;
    if t == 0 { std::ptr::null_mut() } else { engine().find_by_target_name(std::ptr::null_mut(), s(t)) }
}

fn approach(target: f32, value: f32, speed: f32) -> f32 {
    let delta = target - value;
    if delta > speed {
        value + speed
    } else if delta < -speed {
        value - speed
    } else {
        target
    }
}

impl TriggerCamera {
    fn follow_target(&mut self, cx: &mut Cx) {
        let eng = engine();
        let now = eng.time();
        let player = self.player.get();
        if player.is_null() {
            self.following = false;
            return;
        }
        let target = self.target.get();
        if target.is_null() || self.return_time < now {
            let pv = player.v();
            if pv.deadflag == DEAD_NO as i32 && pv.health > 0.0 {
                eng.set_view(player, player);
                enable_control(player, true);
            }
            let me = cx.edict;
            self.delay.use_targets(me, me, UseType::Toggle, 0.0);
            cx.pev().avelocity = [0.0; 3];
            self.state = false;
            self.following = false;
            return;
        }
        let v = cx.pev();
        let mut goal = eng.vec_to_angles(vec::sub(target.v().origin, v.origin));
        goal[0] = -goal[0];
        if v.angles[1] > 360.0 {
            v.angles[1] -= 360.0;
        }
        if v.angles[1] < 0.0 {
            v.angles[1] += 360.0;
        }
        let wrap = |d: f32| {
            let d = if d < -180.0 { d + 360.0 } else { d };
            if d > 180.0 { d - 360.0 } else { d }
        };
        let dx = wrap(goal[0] - v.angles[0]);
        let dy = wrap(goal[1] - v.angles[1]);
        let ft = eng.globals().frametime;
        v.avelocity[0] = dx * 40.0 * ft;
        v.avelocity[1] = dy * 40.0 * ft;
        if v.spawnflags & SF_CAMERA_PLAYER_TAKECONTROL == 0 {
            v.velocity = vec::scale(v.velocity, 0.8);
            if vec::length(v.velocity) < 10.0 {
                v.velocity = [0.0; 3];
            }
        }
        v.nextthink = now;
        self.camera_move(cx);
    }

    fn camera_move(&mut self, cx: &mut Cx) {
        let eng = engine();
        let now = eng.time();
        let ft = eng.globals().frametime;
        let path = self.path.get();
        if path.is_null() {
            return;
        }
        let v = cx.pev();
        self.move_distance -= v.speed * ft;
        if self.move_distance <= 0.0 {
            let pv = path.v();
            if pv.message != 0 {
                let me = cx.edict;
                fire_targets(s(pv.message), me, me, UseType::Toggle, 0.0);
                if pv.spawnflags & super::trains::SF_CORNER_FIREONCE != 0 {
                    pv.message = 0;
                }
            }
            let next = next_target(path);
            self.path = EHandle::new(next);
            let v = cx.pev();
            if next.is_null() {
                v.velocity = [0.0; 3];
            } else {
                let nv = next.v();
                if nv.speed != 0.0 {
                    self.target_speed = nv.speed;
                }
                let delta = vec::sub(nv.origin, v.origin);
                self.move_distance = vec::length(delta);
                v.movedir = vec::normalize(delta);
                self.stop_time = now + path_delay(next);
            }
        }
        let v = cx.pev();
        v.speed = if self.stop_time > now {
            approach(0.0, v.speed, self.deceleration * ft)
        } else {
            approach(self.target_speed, v.speed, self.acceleration * ft)
        };
        let fraction = 2.0 * ft;
        v.velocity = vec::add(vec::scale(v.movedir, v.speed * fraction), vec::scale(v.velocity, 1.0 - fraction));
    }
}

impl Entity for TriggerCamera {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"wait" => self.wait = kv_f(value),
            b"moveto" => self.path_name = alloc(value),
            b"acceleration" => self.acceleration = kv_f(value),
            b"deceleration" => self.deceleration = kv_f(value),
            _ => return self.delay.key_value(key, value),
        }
        true
    }

    fn spawn(&mut self, cx: &mut Cx) {
        let v = cx.pev();
        v.movetype = MOVETYPE_NOCLIP as i32;
        v.solid = SOLID_NOT as i32;
        v.renderamt = 0.0;
        v.rendermode = kRenderTransTexture as i32;
        self.initial_speed = v.speed;
        if self.acceleration == 0.0 {
            self.acceleration = 500.0;
        }
        if self.deceleration == 0.0 {
            self.deceleration = 500.0;
        }
    }

    fn think(&mut self, cx: &mut Cx) {
        if self.following {
            self.follow_target(cx);
        }
    }

    fn use_(&mut self, cx: &mut Cx, activator: *mut edict_t, _caller: *mut edict_t, kind: UseType, _value: f32) {
        let eng = engine();
        let now = eng.time();
        if !should_toggle(kind, self.state) {
            return;
        }
        self.state = !self.state;
        if !self.state {
            self.return_time = now;
            return;
        }
        let activator = if is_player(activator) { activator } else { eng.entity_of_index(1) };
        if !is_player(activator) {
            self.state = false;
            return;
        }
        self.player = EHandle::new(activator);
        self.return_time = now + self.wait;
        cx.pev().speed = self.initial_speed;
        self.target_speed = self.initial_speed;
        let sf = cx.pev().spawnflags;
        self.target = if sf & SF_CAMERA_PLAYER_TARGET != 0 { self.player } else { EHandle::new(next_target(cx.edict)) };
        if !self.target.is_valid() {
            return;
        }
        if sf & SF_CAMERA_PLAYER_TAKECONTROL != 0 {
            enable_control(activator, false);
        }
        let path =
            if self.path_name != 0 { eng.find_by_target_name(std::ptr::null_mut(), s(self.path_name)) } else { std::ptr::null_mut() };
        self.path = EHandle::new(path);
        self.stop_time = now;
        if !path.is_null() {
            let ps = path.v().speed;
            if ps != 0.0 {
                self.target_speed = ps;
            }
            self.stop_time += path_delay(path);
        }
        let av = activator.v();
        let (aorg, aofs, aang, avel, amodel) = (av.origin, av.view_ofs, av.angles, av.velocity, av.model);
        if sf & SF_CAMERA_PLAYER_POSITION != 0 {
            eng.set_origin(cx.edict, vec::add(aorg, aofs));
            let v = cx.pev();
            v.angles = [-aang[0], aang[1], 0.0];
            v.velocity = avel;
        } else {
            cx.pev().velocity = [0.0; 3];
        }
        eng.set_view(activator, cx.edict);
        eng.set_model(cx.edict, s(amodel));
        self.following = true;
        cx.pev().nextthink = now;
        self.move_distance = 0.0;
        self.camera_move(cx);
    }
}

// ---- info_teleport_destination / info_target / info_null etc. --------------------------------------------------

/// `CPointEntity` (info_target, info_teleport_destination, info_intermission...): just keeps its
/// origin/angles. `info_teleport_destination` and `info_target` are found by targetname.
#[derive(Default)]
pub struct PointEntity;

impl Entity for PointEntity {
    fn spawn(&mut self, cx: &mut Cx) {
        cx.pev().solid = SOLID_NOT as i32;
    }
}

/// `info_null`: removes itself.
#[derive(Default)]
pub struct NullEntity;

impl Entity for NullEntity {
    fn spawn(&mut self, cx: &mut Cx) {
        remove(cx.edict);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn cd_command() {
        assert_eq!(super::cd_command(-1).as_deref(), Some("cd pause\n"));
        assert_eq!(super::cd_command(0).as_deref(), Some("cd play   0\n"));
        assert_eq!(super::cd_command(7).as_deref(), Some("cd play   7\n"));
        assert_eq!(super::cd_command(30).as_deref(), Some("cd play  30\n"));
        assert_eq!(super::cd_command(31), None);
        assert_eq!(super::cd_command(-2), None);
        assert_eq!(super::cd_command(i32::MIN), None);
    }

    #[test]
    fn counter_message() {
        assert_eq!(super::counter_message(0), "Sequence completed!");
        assert_eq!(super::counter_message(1), "Only 1 more to go...");
        assert_eq!(super::counter_message(2), "Only 2 more to go...");
        assert_eq!(super::counter_message(3), "Only 3 more to go...");
        assert_eq!(super::counter_message(4), "There are more to go...");
    }

    #[test]
    fn strip_token() {
        assert_eq!(super::strip_token("door1#2"), "door1");
        assert_eq!(super::strip_token("door1"), "door1");
        assert_eq!(super::approach(10.0, 0.0, 3.0), 3.0);
        assert_eq!(super::approach(1.0, 0.0, 3.0), 1.0);
    }
}

// ---- env_global / env_message -----------------------------------------------------------------------------

pub const SF_GLOBAL_SET: i32 = 1;

/// `env_global`: sets a global state (read by masters, trigger_auto).
#[derive(Default)]
pub struct EnvGlobal {
    globalstate: string_t,
    trigger_mode: i32,
    initial_state: i32,
}

fn global_from(i: i32) -> GlobalState {
    match i {
        1 => GlobalState::On,
        2 => GlobalState::Dead,
        _ => GlobalState::Off,
    }
}

impl Entity for EnvGlobal {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"globalstate" => self.globalstate = alloc(value),
            b"triggermode" => self.trigger_mode = kv_i(value),
            b"initialstate" => self.initial_state = kv_i(value),
            _ => return false,
        }
        true
    }

    fn spawn(&mut self, cx: &mut Cx) {
        if self.globalstate == 0 {
            remove(cx.edict);
            return;
        }
        if cx.pev().spawnflags & SF_GLOBAL_SET != 0 && !global_in_table(s(self.globalstate)) {
            set_global_state(s(self.globalstate), global_from(self.initial_state));
        }
    }

    fn use_(&mut self, _cx: &mut Cx, _a: *mut edict_t, _c: *mut edict_t, _k: UseType, _v: f32) {
        let name = s(self.globalstate);
        let old = global_state(name);
        let new = match self.trigger_mode {
            0 => GlobalState::Off,
            1 => GlobalState::On,
            2 => GlobalState::Dead,
            _ => match old {
                GlobalState::On => GlobalState::Off,
                GlobalState::Off => GlobalState::On,
                st => st,
            },
        };
        set_global_state(name, new);
    }
}

pub const SF_MESSAGE_ONCE: i32 = 1;
pub const SF_MESSAGE_ALL: i32 = 2;

/// `env_message`: a titles.txt message (`HudText`) plus an optional sound.
#[derive(Default)]
pub struct EnvMessage;

impl Entity for EnvMessage {
    fn key_value(&mut self, cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        let v = cx.pev();
        match key.to_bytes() {
            b"messagesound" => v.noise = alloc(value),
            b"messagevolume" => v.scale = kv_f(value) * 0.1,
            b"messageattenuation" => v.impulse = kv_i(value),
            _ => return false,
        }
        true
    }

    fn spawn(&mut self, cx: &mut Cx) {
        let v = cx.pev();
        precache_sound_s(v.noise);
        v.solid = SOLID_NOT as i32;
        v.movetype = MOVETYPE_NONE as i32;
        v.speed = match v.impulse {
            1 => goldsrc::ext::entities::ATTN_STATIC,
            2 => ATTN_NORM,
            3 => 0.0,
            _ => goldsrc::ext::entities::ATTN_IDLE,
        };
        v.impulse = 0;
        if v.scale <= 0.0 {
            v.scale = 1.0;
        }
    }

    /// Only the worldspawn chapter title schedules a think: `SUB_CallUseToggle`, which uses the
    /// message with itself as the activator (`CWorld::Precache` 0xf449c).
    fn think(&mut self, cx: &mut Cx) {
        let me = cx.edict;
        self.use_(cx, me, me, UseType::Toggle, 0.0);
    }

    fn use_(&mut self, cx: &mut Cx, activator: *mut edict_t, _c: *mut edict_t, _k: UseType, _v: f32) {
        let eng = engine();
        let v = cx.pev();
        let msg = s(v.message);
        if v.spawnflags & SF_MESSAGE_ALL != 0 {
            for i in 1..=eng.max_clients() {
                show_message(msg, eng.player(i));
            }
        } else {
            let p = if is_player(activator) { activator } else { eng.entity_of_index(1) };
            show_message(msg, p);
        }
        let v = cx.pev();
        let (noise, vol, attn) = (v.noise, v.scale, v.speed);
        if noise != 0 {
            emit(cx.edict, CHAN_BODY, s(noise), vol, attn, 0, PITCH_NORM as i32);
        }
        let me = cx.edict;
        if cx.pev().spawnflags & SF_MESSAGE_ONCE != 0 {
            remove(me);
        }
        let target = cx.pev().target;
        sub_use_targets(me, target, 0, 0.0, me, UseType::Toggle, 0.0);
    }
}
