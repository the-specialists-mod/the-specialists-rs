//! `ambient_generic` (SDK sound.cpp; byte-identical in TS including the preset table) and `speaker`.
//! Sentence names (`!NAME`) aren't precached and go out as `!<index>` (`UTIL_EmitAmbientSound`).

use super::base::*;
use crate::world::{Cx, Entity, UseType};
use goldsrc::edict::EdictPtr;
use goldsrc::ext::entities::{ATTN_IDLE, ATTN_NORM, ATTN_STATIC, snd};
use goldsrc::server::{engine, intern};
use goldsrc::sys::*;
use shared::pm::plus_double;
use std::ffi::CStr;

pub const AMBIENT_SOUND_EVERYWHERE: i32 = 1;
pub const AMBIENT_SOUND_SMALLRADIUS: i32 = 2;
pub const AMBIENT_SOUND_MEDIUMRADIUS: i32 = 4;
pub const AMBIENT_SOUND_LARGERADIUS: i32 = 8;
pub const AMBIENT_SOUND_START_SILENT: i32 = 16;
pub const AMBIENT_SOUND_NOT_LOOPING: i32 = 32;

const LFO_SQUARE: i32 = 1;
const LFO_TRIANGLE: i32 = 2;
const LFO_RANDOM: i32 = 3;

/// `dynpitchvol_t`.
#[derive(Clone, Copy, Default)]
struct Dpv {
    preset: i32,
    pitchrun: i32,
    pitchstart: i32,
    spinup: i32,
    spindown: i32,
    volrun: i32,
    volstart: i32,
    fadein: i32,
    fadeout: i32,
    lfotype: i32,
    lforate: i32,
    lfomodpitch: i32,
    lfomodvol: i32,
    cspinup: i32,
    cspincount: i32,
    pitch: i32,
    spinupsav: i32,
    spindownsav: i32,
    pitchfrac: i32,
    vol: i32,
    fadeinsav: i32,
    fadeoutsav: i32,
    volfrac: i32,
    lfofrac: i32,
    lfomult: i32,
}

/// `rgdpvpreset`: preset, pitch, pstart, spinup, spindown, volrun, volstart, fadein, fadeout,
/// lfotype, lforate, modpitch, modvol, cspinup.
const PRESETS: [[i32; 14]; 27] = [
    [1, 255, 75, 95, 95, 10, 1, 50, 95, 0, 0, 0, 0, 0],
    [2, 255, 85, 70, 88, 10, 1, 20, 88, 0, 0, 0, 0, 0],
    [3, 255, 100, 50, 75, 10, 1, 10, 75, 0, 0, 0, 0, 0],
    [4, 100, 100, 0, 0, 10, 1, 90, 90, 0, 0, 0, 0, 0],
    [5, 100, 100, 0, 0, 10, 1, 80, 80, 0, 0, 0, 0, 0],
    [6, 100, 100, 0, 0, 10, 1, 50, 70, 0, 0, 0, 0, 0],
    [7, 100, 100, 0, 0, 5, 1, 40, 50, 1, 50, 0, 10, 0],
    [8, 100, 100, 0, 0, 5, 1, 40, 50, 1, 150, 0, 10, 0],
    [9, 100, 100, 0, 0, 5, 1, 40, 50, 1, 750, 0, 10, 0],
    [10, 128, 100, 50, 75, 10, 1, 30, 40, 2, 8, 20, 0, 0],
    [11, 128, 100, 50, 75, 10, 1, 30, 40, 2, 25, 20, 0, 0],
    [12, 128, 100, 50, 75, 10, 1, 30, 40, 2, 70, 20, 0, 0],
    [13, 50, 50, 0, 0, 10, 1, 20, 50, 0, 0, 0, 0, 0],
    [14, 70, 70, 0, 0, 10, 1, 20, 50, 0, 0, 0, 0, 0],
    [15, 90, 90, 0, 0, 10, 1, 20, 50, 0, 0, 0, 0, 0],
    [16, 120, 120, 0, 0, 10, 1, 20, 50, 0, 0, 0, 0, 0],
    [17, 180, 180, 0, 0, 10, 1, 20, 50, 0, 0, 0, 0, 0],
    [18, 255, 255, 0, 0, 10, 1, 20, 50, 0, 0, 0, 0, 0],
    [19, 200, 75, 90, 90, 10, 1, 50, 90, 2, 100, 20, 0, 0],
    [20, 255, 75, 97, 90, 10, 1, 50, 90, 1, 40, 50, 0, 0],
    [21, 100, 100, 0, 0, 10, 1, 30, 50, 3, 15, 20, 0, 0],
    [22, 160, 160, 0, 0, 10, 1, 50, 50, 3, 500, 25, 0, 0],
    [23, 255, 75, 88, 0, 10, 1, 40, 0, 0, 0, 0, 0, 5],
    [24, 200, 20, 95, 70, 10, 1, 70, 70, 3, 20, 50, 0, 0],
    [25, 180, 100, 50, 60, 10, 1, 40, 60, 2, 90, 100, 100, 0],
    [26, 60, 60, 0, 0, 10, 1, 40, 70, 3, 80, 20, 50, 0],
    [27, 128, 90, 10, 10, 10, 1, 20, 40, 1, 5, 10, 20, 0],
];

impl Dpv {
    fn from_preset(p: &[i32; 14]) -> Self {
        Dpv {
            preset: p[0],
            pitchrun: p[1],
            pitchstart: p[2],
            spinup: p[3],
            spindown: p[4],
            volrun: p[5],
            volstart: p[6],
            fadein: p[7],
            fadeout: p[8],
            lfotype: p[9],
            lforate: p[10],
            lfomodpitch: p[11],
            lfomodvol: p[12],
            cspinup: p[13],
            ..Default::default()
        }
    }
}

impl Dpv {
    /// `dynpitchvol_t`'s 25 ints in TS's order: the bytes of `m_dpv` (`FIELD_CHARACTER`, 100).
    fn bytes(&self) -> [u8; 100] {
        let f = self.fields();
        let mut b = [0u8; 100];
        for (chunk, x) in b.as_chunks_mut::<4>().0.iter_mut().zip(f) {
            *chunk = x.to_le_bytes();
        }
        b
    }

    fn set_bytes(&mut self, b: &[u8; 100]) {
        let mut f = [0i32; 25];
        for (x, chunk) in f.iter_mut().zip(b.as_chunks::<4>().0) {
            *x = i32::from_le_bytes(*chunk);
        }
        let [a0, a1, a2, a3, a4, a5, a6, a7, a8, a9, a10, a11, a12, a13, a14, a15, a16, a17, a18, a19, a20, a21, a22, a23, a24] = f;
        *self = Dpv {
            preset: a0,
            pitchrun: a1,
            pitchstart: a2,
            spinup: a3,
            spindown: a4,
            volrun: a5,
            volstart: a6,
            fadein: a7,
            fadeout: a8,
            lfotype: a9,
            lforate: a10,
            lfomodpitch: a11,
            lfomodvol: a12,
            cspinup: a13,
            cspincount: a14,
            pitch: a15,
            spinupsav: a16,
            spindownsav: a17,
            pitchfrac: a18,
            vol: a19,
            fadeinsav: a20,
            fadeoutsav: a21,
            volfrac: a22,
            lfofrac: a23,
            lfomult: a24,
        };
    }

    fn fields(&self) -> [i32; 25] {
        let d = self;
        [
            d.preset,
            d.pitchrun,
            d.pitchstart,
            d.spinup,
            d.spindown,
            d.volrun,
            d.volstart,
            d.fadein,
            d.fadeout,
            d.lfotype,
            d.lforate,
            d.lfomodpitch,
            d.lfomodvol,
            d.cspinup,
            d.cspincount,
            d.pitch,
            d.spinupsav,
            d.spindownsav,
            d.pitchfrac,
            d.vol,
            d.fadeinsav,
            d.fadeoutsav,
            d.volfrac,
            d.lfofrac,
            d.lfomult,
        ]
    }
}

/// `CAmbientGeneric`'s think (`m_pfnThink`).
#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum AmbientThink {
    #[default]
    None,
    /// `RampThink`, set by `Spawn`.
    Ramp,
    /// `SUB_Remove`: an ambient with no `message`.
    Remove,
}

#[derive(Default)]
pub struct AmbientGeneric {
    /// `m_flAttenuation` (+0x64).
    attenuation: f32,
    /// `m_dpv` (+0x68).
    dpv: Dpv,
    /// `m_fActive` (+0xcc).
    active: bool,
    /// `m_fLooping` (+0xd0).
    looping: bool,
    /// The `message` sample, interned (derived from `pev->message`).
    sample: Option<&'static CStr>,
    think: AmbientThink,
    /// `SetUse(ToggleUse)`, from `Spawn`.
    usable: bool,
}

/// `(101 - x) * 64` for a non-zero 0..100 ramp time.
fn ramp(x: i32) -> i32 {
    let x = x.clamp(0, 100);
    if x > 0 { (101 - x) * 64 } else { 0 }
}

impl AmbientGeneric {
    /// `CAmbientGeneric::Precache` (0xdec90): the sample, the modulation, and an active ambient's
    /// sound. Only a sample longer than one character (and no `!` sentence) is precached, but the
    /// sounds play `message` whatever its length.
    fn precache(&mut self, cx: &mut Cx) {
        let eng = engine();
        let name = kv_str(s(cx.pev().message));
        let n = intern(name);
        if name.len() > 1 && !name.starts_with('!') {
            eng.precache_sound(n);
        }
        self.sample = Some(n);
        self.init_modulation(cx);
        if cx.pev().spawnflags & AMBIENT_SOUND_START_SILENT == 0 && self.looping {
            self.active = true;
        }
        if self.active {
            let (vol, attn, pitch) = (self.dpv.vol as f32 * 0.01, self.attenuation, self.dpv.pitch);
            self.emit(cx, vol, attn, snd::SPAWNING, pitch);
            cx.pev().nextthink = plus_double(eng.time(), 0.1);
        }
    }

    fn emit(&self, cx: &mut Cx, vol: f32, attn: f32, flags: i32, pitch: i32) {
        if let Some(s) = self.sample {
            let org = cx.pev().origin;
            super::sentences::emit_ambient(cx.edict, org, s, vol, attn, flags, pitch);
        }
    }

    fn init_modulation(&mut self, cx: &mut Cx) {
        let d = &mut self.dpv;
        d.volrun = ((cx.pev().health * 10.0) as i32).clamp(0, 100);
        if d.preset != 0 && d.preset as usize <= PRESETS.len() {
            *d = Dpv::from_preset(&PRESETS[d.preset as usize - 1]);
            if d.spindown > 0 {
                d.spindown = (101 - d.spindown) * 64;
            }
            if d.spinup > 0 {
                d.spinup = (101 - d.spinup) * 64;
            }
            d.volstart *= 10;
            d.volrun *= 10;
            if d.fadein > 0 {
                d.fadein = (101 - d.fadein) * 64;
            }
            if d.fadeout > 0 {
                d.fadeout = (101 - d.fadeout) * 64;
            }
            d.lforate *= 256;
            d.fadeinsav = d.fadein;
            d.fadeoutsav = d.fadeout;
            d.spinupsav = d.spinup;
            d.spindownsav = d.spindown;
        }
        d.fadein = d.fadeinsav;
        d.fadeout = 0;
        d.vol = if d.fadein != 0 { d.volstart } else { d.volrun };
        d.spinup = d.spinupsav;
        d.spindown = 0;
        d.pitch = if d.spinup != 0 { d.pitchstart } else { d.pitchrun };
        if d.pitch == 0 {
            d.pitch = PITCH_NORM as i32;
        }
        d.pitchfrac = d.pitch << 8;
        d.volfrac = d.vol << 8;
        d.lfofrac = 0;
        d.lforate = d.lforate.abs();
        d.cspincount = 1;
        if d.cspinup != 0 {
            let inc = (255 - d.pitchstart) / d.cspinup;
            d.pitchrun = (d.pitchstart + inc).min(255);
        }
        if (d.spinupsav != 0 || d.spindownsav != 0 || (d.lfotype != 0 && d.lfomodpitch != 0)) && d.pitch == PITCH_NORM as i32 {
            d.pitch = PITCH_NORM as i32 + 1;
        }
    }

    fn ramp_think(&mut self, cx: &mut Cx) {
        let eng = engine();
        let mut pitch = self.dpv.pitch;
        let mut vol = self.dpv.vol;
        let mut flags = 0;
        let mut changed = false;
        let d = self.dpv;
        if d.spinup == 0 && d.spindown == 0 && d.fadein == 0 && d.fadeout == 0 && d.lfotype == 0 {
            return;
        }
        if d.spinup != 0 || d.spindown != 0 {
            let d = &mut self.dpv;
            let prev = d.pitchfrac >> 8;
            if d.spinup > 0 {
                d.pitchfrac += d.spinup;
            } else if d.spindown > 0 {
                d.pitchfrac -= d.spindown;
            }
            pitch = d.pitchfrac >> 8;
            if pitch > d.pitchrun {
                pitch = d.pitchrun;
                d.spinup = 0;
            }
            if pitch < d.pitchstart {
                d.spindown = 0;
                self.emit(cx, 0.0, 0.0, snd::STOP, 0);
                return;
            }
            pitch = pitch.clamp(1, 255);
            d.pitch = pitch;
            changed |= prev != pitch;
            flags |= snd::CHANGE_PITCH;
        }
        if self.dpv.fadein != 0 || self.dpv.fadeout != 0 {
            let d = &mut self.dpv;
            let prev = d.volfrac >> 8;
            if d.fadein > 0 {
                d.volfrac += d.fadein;
            } else if d.fadeout > 0 {
                d.volfrac -= d.fadeout;
            }
            vol = d.volfrac >> 8;
            if vol > d.volrun {
                vol = d.volrun;
                d.fadein = 0;
            }
            if vol < d.volstart {
                d.fadeout = 0;
                self.emit(cx, 0.0, 0.0, snd::STOP, 0);
                return;
            }
            vol = vol.clamp(1, 100);
            d.vol = vol;
            changed |= prev != vol;
            flags |= snd::CHANGE_VOL;
        }
        if self.dpv.lfotype != 0 {
            let d = &mut self.dpv;
            if d.lfofrac > 0x6fff_ffff {
                d.lfofrac = 0;
            }
            d.lfofrac = d.lfofrac.wrapping_add(d.lforate);
            let mut pos = d.lfofrac >> 8;
            if d.lfofrac < 0 {
                d.lfofrac = 0;
                d.lforate = d.lforate.abs();
                pos = 0;
            } else if pos > 255 {
                pos = 255;
                d.lfofrac = 255 << 8;
                d.lforate = -d.lforate.abs();
            }
            match d.lfotype {
                LFO_SQUARE => d.lfomult = if pos < 128 { 255 } else { 0 },
                LFO_RANDOM => {
                    if pos == 255 {
                        d.lfomult = eng.random_long(0, 255);
                    }
                }
                _ => d.lfomult = pos, // LFO_TRIANGLE and anything else
            }
            if d.lfomodpitch != 0 {
                let prev = pitch;
                pitch = (pitch + ((d.lfomult - 128) * d.lfomodpitch) / 100).clamp(1, 255);
                changed |= prev != pitch;
                flags |= snd::CHANGE_PITCH;
            }
            if d.lfomodvol != 0 {
                let prev = vol;
                vol = (vol + ((d.lfomult - 128) * d.lfomodvol) / 100).clamp(0, 100);
                changed |= prev != vol;
                flags |= snd::CHANGE_VOL;
            }
        }
        if flags != 0 && changed {
            if pitch == PITCH_NORM as i32 {
                pitch = PITCH_NORM as i32 + 1;
            }
            let attn = self.attenuation;
            self.emit(cx, vol as f32 * 0.01, attn, flags, pitch);
        }
        cx.pev().nextthink = plus_double(eng.time(), 0.2);
    }
}

impl Entity for AmbientGeneric {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        let d = &mut self.dpv;
        let i = kv_i(value);
        match key.to_bytes() {
            b"preset" => d.preset = i,
            b"pitch" => d.pitchrun = i.clamp(0, 255),
            b"pitchstart" => d.pitchstart = i.clamp(0, 255),
            b"spinup" => {
                d.spinup = ramp(i);
                d.spinupsav = d.spinup;
            }
            b"spindown" => {
                d.spindown = ramp(i);
                d.spindownsav = d.spindown;
            }
            b"volstart" => d.volstart = i.clamp(0, 10) * 10,
            b"fadein" => {
                d.fadein = ramp(i);
                d.fadeinsav = d.fadein;
            }
            b"fadeout" => {
                d.fadeout = ramp(i);
                d.fadeoutsav = d.fadeout;
            }
            b"lfotype" => d.lfotype = if i > 4 { LFO_TRIANGLE } else { i },
            b"lforate" => d.lforate = i.clamp(0, 1000) * 256,
            b"lfomodpitch" => d.lfomodpitch = i.clamp(0, 100),
            b"lfomodvol" => d.lfomodvol = i.clamp(0, 100),
            b"cspinup" => d.cspinup = i.clamp(0, 100),
            _ => return false,
        }
        true
    }

    /// `CAmbientGeneric::Spawn` (0xdeb14).
    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        let sf = cx.pev().spawnflags;
        self.attenuation = if sf & AMBIENT_SOUND_EVERYWHERE != 0 {
            0.0
        } else if sf & AMBIENT_SOUND_SMALLRADIUS != 0 {
            ATTN_IDLE
        } else if sf & AMBIENT_SOUND_MEDIUMRADIUS != 0 {
            ATTN_STATIC
        } else if sf & AMBIENT_SOUND_LARGERADIUS != 0 {
            ATTN_NORM
        } else {
            ATTN_STATIC
        };
        let msg = s(cx.pev().message);
        if msg.is_empty() {
            // Removed a tenth of a second later (SetThink(SUB_Remove)).
            let o = cx.pev().origin;
            eng.alert(ALERT_TYPE::at_error, &format!("EMPTY AMBIENT AT: {:.6}, {:.6}, {:.6}\n", o[0], o[1], o[2]));
            cx.pev().nextthink = plus_double(eng.time(), 0.1);
            self.think = AmbientThink::Remove;
            return;
        }
        let v = cx.pev();
        v.solid = SOLID_NOT as i32;
        v.movetype = MOVETYPE_NONE as i32;
        self.think = AmbientThink::Ramp;
        v.nextthink = 0.0;
        self.usable = true;
        self.active = false;
        self.looping = sf & AMBIENT_SOUND_NOT_LOOPING == 0;
        self.precache(cx);
    }

    /// `CAmbientGeneric::Save` (0xdea74) / `Restore`: `CBaseEntity`'s part (think `RampThink` or
    /// `SUB_Remove`, use `ToggleUse`), then `"CAmbientGeneric"`.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut crate::save::Fields) {
        let funcs = crate::save::BaseFuncs { think: self.think != AmbientThink::None, use_: self.usable, ..Default::default() };
        let cleared = f.base_funcs(cx, funcs);
        if cleared.think {
            self.think = AmbientThink::None;
        }
        if cleared.use_ {
            self.usable = false;
        }
        f.table(&crate::save::tables::CAmbientGeneric);
        f.float(&mut self.attenuation);
        f.boolean(&mut self.active);
        f.boolean(&mut self.looping);
        let mut b = self.dpv.bytes();
        f.bytes(&mut b);
        self.dpv.set_bytes(&b);
    }

    /// `CAmbientGeneric::Precache` (0xdec90): it restarts the sound of an active looping ambient.
    fn restored(&mut self, cx: &mut Cx) {
        self.precache(cx);
    }

    fn think(&mut self, cx: &mut Cx) {
        match self.think {
            AmbientThink::Ramp => self.ramp_think(cx),
            AmbientThink::Remove => sub_remove(cx.edict),
            AmbientThink::None => {}
        }
    }

    /// `CAmbientGeneric::ToggleUse` (0xdf3f4), through `m_pfnUse`.
    fn use_(&mut self, cx: &mut Cx, _activator: *mut edict_t, _caller: *mut edict_t, kind: UseType, value: f32) {
        if !self.usable {
            return;
        }
        let now = engine().time();
        if kind != UseType::Toggle && ((self.active && kind == UseType::On) || (!self.active && kind == UseType::Off)) {
            return;
        }
        if kind == UseType::Set && self.active {
            let fraction = if value > 1.0 {
                1.0
            } else if value < 0.0 {
                0.01
            } else {
                value
            };
            self.dpv.pitch = (fraction * 255.0) as i32;
            let p = self.dpv.pitch;
            self.emit(cx, 0.0, 0.0, snd::CHANGE_PITCH, p);
            return;
        }
        if self.active {
            if self.dpv.cspinup != 0 {
                let d = &mut self.dpv;
                if d.cspincount <= d.cspinup {
                    d.cspincount += 1;
                    let inc = (255 - d.pitchstart) / d.cspinup;
                    d.spinup = d.spinupsav;
                    d.spindown = 0;
                    d.pitchrun = (d.pitchstart + inc * d.cspincount).min(255);
                    cx.pev().nextthink = plus_double(now, 0.1);
                }
            } else {
                self.active = false;
                cx.pev().spawnflags |= AMBIENT_SOUND_START_SILENT;
                let d = &mut self.dpv;
                if d.spindownsav != 0 || d.fadeoutsav != 0 {
                    d.spindown = d.spindownsav;
                    d.spinup = 0;
                    d.fadeout = d.fadeoutsav;
                    d.fadein = 0;
                    cx.pev().nextthink = plus_double(now, 0.1);
                } else {
                    self.emit(cx, 0.0, 0.0, snd::STOP, 0);
                }
            }
        } else {
            if self.looping {
                self.active = true;
            } else {
                self.emit(cx, 0.0, 0.0, snd::STOP, 0);
            }
            self.init_modulation(cx);
            let (vol, attn, pitch) = (self.dpv.vol as f32 * 0.01, self.attenuation, self.dpv.pitch);
            self.emit(cx, vol, attn, 0, pitch);
            cx.pev().nextthink = plus_double(now, 0.1);
        }
    }
}

// ---- env_sound ---------------------------------------------------------------------------------------------

const SVC_ROOMTYPE: i32 = 37;

/// Per-player DSP state the SDK keeps on `CBasePlayer` (`m_pentSndLast`, `m_flSndRoomtype`,
/// `m_flSndRange`), indexed by player slot.
#[derive(Clone, Copy, Default)]
struct PlayerRoom {
    last: goldsrc::ext::entities::EHandle,
    roomtype: f32,
    range: f32,
}

static ROOMS: goldsrc::Global<std::cell::RefCell<[PlayerRoom; 33]>> =
    goldsrc::Global(std::cell::RefCell::new([PlayerRoom { last: goldsrc::ext::entities::EHandle::NULL, roomtype: 0.0, range: 0.0 }; 33]));

/// Forget a player's room (call on spawn/disconnect so a new client starts clean).
pub fn reset_player_room(index: i32) {
    if let Some(r) = ROOMS.0.borrow_mut().get_mut(index as usize) {
        *r = PlayerRoom::default();
    }
}

/// `env_sound`: sets a client's DSP room type (`SVC_ROOMTYPE`) while it's within `radius` and in
/// line of sight.
#[derive(Default)]
pub struct EnvSound {
    /// `m_flRadius` (+0x64).
    radius: f32,
    /// `m_flRoomtype` (+0x68).
    roomtype: f32,
}

impl EnvSound {
    /// `FEnvSoundInRange`.
    fn in_range(&self, cx: &mut Cx, p: *mut edict_t) -> Option<f32> {
        let eng = engine();
        let v = cx.pev();
        let a = goldsrc::ext::entities::vec::add(v.origin, v.view_ofs);
        let pv = p.v();
        let b = goldsrc::ext::entities::vec::add(pv.origin, pv.view_ofs);
        let tr = eng.trace_line(a, b, goldsrc::server::IGNORE_MONSTERS, cx.edict);
        if (tr.fInOpen != 0 && tr.fInWater != 0) || tr.flFraction != 1.0 {
            return None;
        }
        // mp.dll 0x1003bfc5..0x1003c011 (issue #171): the unrounded differences summed `z, y, x`, the
        // root compared unrounded (`<=`, a NaN is in range) and stored.
        let d = [0, 1, 2].map(|i| f64::from(a[i]) - f64::from(b[i]));
        let range = ((d[2] * d[2] + d[1] * d[1]) + d[0] * d[0]).sqrt();
        shared::pm::not_above87(range, f64::from(self.radius)).then_some(range as f32)
    }
}

impl Entity for EnvSound {
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"radius" => self.radius = kv_f(value),
            b"roomtype" => self.roomtype = kv_f(value),
            _ => return false,
        }
        true
    }

    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        cx.pev().nextthink = eng.time() + eng.random_float(0.0, 0.5);
    }

    /// `CEnvSound::Save` / `Restore`: `CBaseEntity`'s part (its `Think` is a virtual override, no
    /// member pointer), then `"CEnvSound"`. `Precache` is `CBaseEntity`'s.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut crate::save::Fields) {
        f.base_funcs(cx, crate::save::BaseFuncs::default());
        f.table(&crate::save::tables::CEnvSound);
        f.float(&mut self.radius);
        f.float(&mut self.roomtype);
    }

    fn think(&mut self, cx: &mut Cx) {
        let eng = engine();
        let now = eng.time();
        let p = eng.find_client_in_pvs(cx.edict);
        let fast = 'think: {
            if p.is_null() || eng.index_of(p) == 0 || !goldsrc::ext::entities::is_player(p) {
                break 'think false;
            }
            let idx = eng.index_of(p) as usize;
            let room = ROOMS.0.borrow().get(idx).copied().unwrap_or_default();
            if room.last.get() == cx.edict && !cx.edict.is_null() {
                if room.roomtype != 0.0 && room.range != 0.0 {
                    let r = self.in_range(cx, p);
                    let mut rooms = ROOMS.0.borrow_mut();
                    match r {
                        Some(range) => {
                            rooms[idx].range = range;
                            break 'think true;
                        }
                        None => {
                            rooms[idx].range = 0.0;
                            rooms[idx].roomtype = 0.0;
                            break 'think false;
                        }
                    }
                }
                break 'think false;
            }
            if let Some(range) = self.in_range(cx, p)
                && (range < room.range || room.range == 0.0)
            {
                ROOMS.0.borrow_mut()[idx] =
                    PlayerRoom { last: goldsrc::ext::entities::EHandle::new(cx.edict), roomtype: self.roomtype, range };
                goldsrc::server::MsgWriter::begin(MSG_ONE as i32, SVC_ROOMTYPE, None, p).short(self.roomtype as i32).end();
            }
            true
        };
        cx.pev().nextthink = now + if fast { 0.25 } else { 0.75 };
    }
}

/// New map: every client's room state starts clean.
pub fn reset_rooms() {
    *ROOMS.0.borrow_mut() = [PlayerRoom::default(); 33];
}

// ---- speaker -------------------------------------------------------------------------------------------------------

/// `speaker` spawnflag: start silent (the first announcement waits for a use).
pub const SPEAKER_START_SILENT: i32 = 1;

/// `CTalkMonster::g_talkWaitTime`: no announcement before this time. TS has no talking monsters,
/// so only speakers move it. A process-wide static, never reset across maps, as in TS.
static TALK_WAIT_TIME: goldsrc::Global<std::cell::Cell<f32>> = goldsrc::Global(std::cell::Cell::new(0.0));

/// The sentence group a `speaker` `preset` names (`CSpeaker::SpeakerThink` 0xe1af4); 0 is the
/// `message` key.
fn speaker_preset(preset: i32) -> Option<&'static CStr> {
    Some(match preset {
        1 => c"C1A0_",
        2 => c"C1A1_",
        3 => c"C1A2_",
        4 => c"C1A3_",
        5 => c"C1A4_",
        6 => c"C2A1_",
        7 => c"C2A2_",
        8 => c"C2A3_",
        9 => c"C2A4_",
        10 => c"C2A5_",
        11 => c"C3A1_",
        12 => c"C3A2_",
        _ => return None,
    })
}

/// `CSpeaker::ToggleUse` (0xe1cfc): the new `nextthink` (on = 0.1 s from now, off = 0), or `None`
/// to leave it. The speaker is on while `nextthink > 0`.
fn speaker_toggle(nextthink: f32, kind: UseType, now: f32) -> Option<f32> {
    let active = nextthink > 0.0;
    if kind != UseType::Toggle && ((active && kind == UseType::On) || (!active && kind == UseType::Off)) {
        return None;
    }
    Some(if kind == UseType::On || (kind != UseType::Off && !active) { plus_double(now, 0.1) } else { 0.0 })
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum SpeakerThink {
    #[default]
    None,
    Speak,
    /// `SUB_Remove`: a speaker with nothing to say.
    Remove,
}

/// `speaker` (`CSpeaker`): announcements from a sentence group (`preset` 1-12, or the `message`
/// key) every 15-135 s while on, volume `health` / 10; a `!sentence` plays once.
#[derive(Default)]
pub struct Speaker {
    /// `m_preset` (+0x64).
    preset: i32,
    think: SpeakerThink,
    /// `SetUse(ToggleUse)`.
    usable: bool,
}

impl Speaker {
    /// `CSpeaker::Precache` (0xe1a98): a speaker that doesn't start silent speaks in 5-15 s.
    fn precache(cx: &mut Cx) {
        let eng = engine();
        if cx.pev().spawnflags & SPEAKER_START_SILENT == 0 {
            let r = eng.random_float(5.0, 15.0);
            cx.pev().nextthink = (r as f64 + eng.time() as f64) as f32;
        }
    }

    /// `CSpeaker::SpeakerThink` (0xe1af4).
    fn speak(&mut self, cx: &mut Cx) {
        let eng = engine();
        let now = eng.time();
        let me = cx.edict;
        let v = cx.pev();
        let vol = v.health * 0.1;
        let wait = TALK_WAIT_TIME.0.get();
        if wait >= now {
            let r = eng.random_float(5.0, 10.0);
            cx.pev().nextthink = (r as f64 + wait as f64) as f32;
            return;
        }
        // Guard: TS's switch has no default, so a preset outside 1-12 reads an uninitialised
        // pointer. The port plays nothing and logs the bad group.
        let name = if self.preset == 0 { s(v.message) } else { speaker_preset(self.preset).unwrap_or(c"") };
        let origin = v.origin;
        if name.to_bytes().first() == Some(&b'!') {
            super::sentences::emit_ambient(me, origin, name, vol, 0.3, 0, PITCH_NORM as i32);
            cx.pev().nextthink = 0.0;
        } else {
            if super::sentences::play_random(me, name, vol, 0.3, 0, PITCH_NORM as i32) < 0 {
                eng.alert(
                    ALERT_TYPE::at_console,
                    &format!("Level Design Error!\nSPEAKER has bad sentence group name: {}\n", name.to_string_lossy()),
                );
            }
            let r = eng.random_float(15.0, 135.0);
            let now = eng.time();
            cx.pev().nextthink = (r as f64 + now as f64) as f32;
            TALK_WAIT_TIME.0.set(now + 5.0);
        }
    }
}

impl Entity for Speaker {
    /// `CSpeaker::KeyValue` (0xe1d88).
    fn key_value(&mut self, _cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        if key != c"preset" {
            return false;
        }
        self.preset = kv_i(value);
        true
    }

    /// `CSpeaker::Spawn` (0xe1988) and `Precache` (0xe1a98).
    fn spawn(&mut self, cx: &mut Cx) {
        let eng = engine();
        let now = eng.time();
        let v = cx.pev();
        if self.preset == 0 && s(v.message).is_empty() {
            let o = v.origin;
            eng.alert(ALERT_TYPE::at_error, &format!("SPEAKER with no Level/Sentence! at: {:.6}, {:.6}, {:.6}\n", o[0], o[1], o[2]));
            v.nextthink = plus_double(now, 0.1);
            self.think = SpeakerThink::Remove;
            return;
        }
        v.solid = SOLID_NOT as i32;
        v.movetype = MOVETYPE_NONE as i32;
        self.think = SpeakerThink::Speak;
        v.nextthink = 0.0;
        self.usable = true;
        Self::precache(cx);
    }

    /// `CSpeaker::Save` / `Restore`: `CBaseEntity`'s part (think `SpeakerThink` or `SUB_Remove`,
    /// use `ToggleUse`), then `"CSpeaker"`.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut crate::save::Fields) {
        let funcs = crate::save::BaseFuncs { think: self.think != SpeakerThink::None, use_: self.usable, ..Default::default() };
        let cleared = f.base_funcs(cx, funcs);
        if cleared.think {
            self.think = SpeakerThink::None;
        }
        if cleared.use_ {
            self.usable = false;
        }
        f.table(&crate::save::tables::CSpeaker);
        f.int(&mut self.preset);
    }

    /// `CSpeaker::Precache` (0xe1a98).
    fn restored(&mut self, cx: &mut Cx) {
        Self::precache(cx);
    }

    fn think(&mut self, cx: &mut Cx) {
        match self.think {
            SpeakerThink::Speak => self.speak(cx),
            SpeakerThink::Remove => sub_remove(cx.edict),
            SpeakerThink::None => {}
        }
    }

    fn use_(&mut self, cx: &mut Cx, _a: *mut edict_t, _c: *mut edict_t, kind: UseType, _v: f32) {
        if !self.usable {
            return;
        }
        let now = engine().time();
        if let Some(t) = speaker_toggle(cx.pev().nextthink, kind, now) {
            cx.pev().nextthink = t;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speaker_toggle_states() {
        // Off (nextthink 0): toggle/on/set start it; off leaves it.
        assert_eq!(speaker_toggle(0.0, UseType::Toggle, 10.0), Some(10.1));
        assert_eq!(speaker_toggle(0.0, UseType::On, 10.0), Some(10.1));
        assert_eq!(speaker_toggle(0.0, UseType::Set, 10.0), Some(10.1));
        assert_eq!(speaker_toggle(0.0, UseType::Off, 10.0), None);
        // On: toggle/off/set stop it; on leaves it.
        assert_eq!(speaker_toggle(20.0, UseType::Toggle, 10.0), Some(0.0));
        assert_eq!(speaker_toggle(20.0, UseType::Off, 10.0), Some(0.0));
        assert_eq!(speaker_toggle(20.0, UseType::Set, 10.0), Some(0.0));
        assert_eq!(speaker_toggle(20.0, UseType::On, 10.0), None);
    }

    #[test]
    fn speaker_presets() {
        assert_eq!(speaker_preset(1), Some(c"C1A0_"));
        assert_eq!(speaker_preset(12), Some(c"C3A2_"));
        assert_eq!(speaker_preset(0), None);
        assert_eq!(speaker_preset(13), None);
    }
}
