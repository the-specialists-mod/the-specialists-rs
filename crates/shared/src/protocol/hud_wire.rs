//! Server-side encoders for the HUD messages (rules package). The client decodes the same payloads
//! with `protocol::hud` (HUD package); both follow the decomp layouts.

//! Message payloads for the hud area (layouts: decomp server/architecture.md §3.2,
//! client/architecture-hud.md §3).
//!
//! Every payload is a plain struct with `write` (server) and `read` (client). Both go through the
//! small [`Wire`] / [`WireRead`] traits so the server can map them onto `pfnWrite*` (the engine
//! encodes coords) and the client onto its message reader. `Bytes` implements both with the
//! GoldSrc encoding (little endian, coord = short ×8) for tests and tools.
//!
//! Field kinds: `string` is a Rust `String` for text the server makes itself (weapon and team
//! names); `bytes` is a `Vec<u8>` for text that comes from players, the map or the server's files
//! (chat, names, the MOTD), which TS writes byte for byte whatever its encoding
//! (`shared::text`).

/// Writer side of a user message (`WRITE_BYTE` ...).
pub trait Wire {
    fn byte(&mut self, v: i32);
    fn char(&mut self, v: i32);
    fn short(&mut self, v: i32);
    fn long(&mut self, v: i32);
    fn coord(&mut self, v: f32);
    /// `WRITE_STRING`: the bytes as given (TS writes player text unconverted; a NUL ends it).
    fn string(&mut self, s: &[u8]);
}

/// Reader side of a user message (`READ_BYTE` ...). Readers return 0 / "" past the end.
pub trait WireRead {
    fn byte(&mut self) -> u8;
    fn char(&mut self) -> i8;
    fn short(&mut self) -> i16;
    fn long(&mut self) -> i32;
    fn coord(&mut self) -> f32;
    /// `READ_STRING`: the bytes up to the NUL, unconverted.
    fn string(&mut self) -> Vec<u8>;
}

/// In-memory GoldSrc encoding, used by tests (and usable by tools).
#[derive(Default, Debug, Clone, PartialEq)]
pub struct Bytes {
    pub buf: Vec<u8>,
    pub pos: usize,
}

impl Wire for Bytes {
    fn byte(&mut self, v: i32) {
        self.buf.push(v as u8);
    }
    fn char(&mut self, v: i32) {
        self.buf.push(v as i8 as u8);
    }
    fn short(&mut self, v: i32) {
        self.buf.extend_from_slice(&(v as i16).to_le_bytes());
    }
    fn long(&mut self, v: i32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn coord(&mut self, v: f32) {
        Wire::short(self, (v * 8.0) as i32);
    }
    fn string(&mut self, s: &[u8]) {
        self.buf.extend(s.iter().take_while(|&&b| b != 0));
        self.buf.push(0);
    }
}

impl Bytes {
    fn take<const N: usize>(&mut self) -> [u8; N] {
        let out = self.buf.get(self.pos..self.pos + N).map(|b| b.try_into().unwrap()).unwrap_or([0; N]);
        self.pos += N;
        out
    }
}

impl WireRead for Bytes {
    fn byte(&mut self) -> u8 {
        self.take::<1>()[0]
    }
    fn char(&mut self) -> i8 {
        self.take::<1>()[0] as i8
    }
    fn short(&mut self) -> i16 {
        i16::from_le_bytes(self.take())
    }
    fn long(&mut self) -> i32 {
        i32::from_le_bytes(self.take())
    }
    fn coord(&mut self) -> f32 {
        WireRead::short(self) as f32 / 8.0
    }
    fn string(&mut self) -> Vec<u8> {
        let rest = self.buf.get(self.pos..).unwrap_or(&[]);
        let n = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
        let s = rest[..n].to_vec();
        self.pos += n + 1;
        s
    }
}

/// Defines a payload struct whose fields are written and read in declaration order.
macro_rules! payload {
    ($(#[$m:meta])* $name:ident { $($(#[$fm:meta])* $f:ident : $kind:ident),* $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Default, PartialEq)]
        pub struct $name { $($(#[$fm])* pub $f: payload!(@ty $kind)),* }
        impl $name {
            pub fn write(&self, w: &mut impl Wire) {
                $(payload!(@w w, self.$f, $kind);)*
            }
            pub fn read(r: &mut impl WireRead) -> Self {
                Self { $($f: payload!(@r r, $kind)),* }
            }
        }
    };
    (@ty byte) => { u8 };
    (@ty char) => { i8 };
    (@ty short) => { i16 };
    (@ty long) => { i32 };
    (@ty coord) => { f32 };
    (@ty string) => { String };
    (@ty bytes) => { Vec<u8> };
    (@w $w:ident, $v:expr, byte) => { $w.byte($v as i32) };
    (@w $w:ident, $v:expr, char) => { $w.char($v as i32) };
    (@w $w:ident, $v:expr, short) => { $w.short($v as i32) };
    (@w $w:ident, $v:expr, long) => { $w.long($v) };
    (@w $w:ident, $v:expr, coord) => { $w.coord($v) };
    (@w $w:ident, $v:expr, string) => { $w.string($v.as_bytes()) };
    (@w $w:ident, $v:expr, bytes) => { $w.string(&$v) };
    (@r $r:ident, byte) => { $r.byte() };
    (@r $r:ident, char) => { $r.char() };
    (@r $r:ident, short) => { $r.short() };
    (@r $r:ident, long) => { $r.long() };
    (@r $r:ident, coord) => { $r.coord() };
    (@r $r:ident, string) => { String::from_utf8_lossy(&$r.string()).into_owned() };
    (@r $r:ident, bytes) => { $r.string() };
}
pub(crate) use payload;

payload!(
    /// `TSHealth` (ONE): health clamped to 0..255. 0 triggers the client death cam.
    TsHealth { hp: byte }
);
payload!(
    /// `TSArmor` (ONE): `pev->armorvalue`.
    TsArmor { armor: byte }
);
payload!(
    /// `TSState` (ONE): stance 0 standing, 1 ducked, 2 prone/stunt (`iuser4 & 0x30`).
    TsState { state: byte }
);
payload!(
    /// `TSCash` (ONE): money.
    TsCash { cash: short }
);
payload!(
    /// `TSSpace` (ONE): free inventory slots (81 = nothing bought).
    TsSpace { space: byte }
);
payload!(
    /// `PwUp` (ONE): held power-up bits and seconds. The permanent flags 4 (kung fu) and 0x100
    /// (super jump) are sent on their own with `secs = 0`.
    PwUp { bits: short, secs: byte }
);
payload!(
    /// `PwUpTime` (ONE): seconds left of the power-up just activated.
    PwUpTime { secs: byte }
);
payload!(
    /// `TSSlowMo` (ONE): slow factor ×100 as a coord (client: `READ_COORD * 0.01`, 0 → 1.0).
    TsSlowMo { factor_x100: coord }
);
payload!(
    /// `TSBTime` (ONE): bullet-time flag (slow pause active).
    TsBTime { on: byte }
);
payload!(
    /// `TSMBlur` (PVS): motion-blur trail on an entity: 0 off, 1 short, 2 long.
    TsMBlur { entindex: byte, mode: byte }
);
payload!(
    /// `TSPAward` (ALL): scoreboard award/status: 0 none, 1 out until next round, 2 Killer,
    /// 3 Demolition, 4 Specialist, 10 The One, 11 respawn pending.
    TsPAward { entindex: byte, award: byte }
);
payload!(
    /// `PTakeDam` (PVS, 13 bytes): hit position, direction, `min(dmg/2,127)` (+128 on kevlar).
    PTakeDam { x: coord, y: coord, z: coord, dx: coord, dy: coord, dz: coord, amount: byte }
);
payload!(
    /// `TSFade` (ONE): ScreenFade layout. Times are 4.12 fixed point seconds.
    TsFade { duration: short, hold: short, flags: short, r: byte, g: byte, b: byte, a: byte }
);
payload!(
    /// `ActItems` (ALL): active weapon attachments of a player (laser, flashlight, akimbo 0x40).
    ActItems { entindex: byte, mask: byte }
);
payload!(
    /// `Spectator` (ALL/ONE): observer flag for the scoreboard and HUD.
    Spectator { entindex: byte, observer: byte }
);
payload!(
    /// `SetFOV` (ONE).
    SetFov { fov: byte }
);
payload!(
    /// `HideWeapon` (ONE).
    HideWeapon { flags: byte }
);
payload!(
    /// `Train` (ONE): `m_iTrain & 0xF`, the train speed notch (0 off, 1 neutral, 2-4 forward,
    /// 5 back). Sent by `CBasePlayer::UpdateClientData` (0x91d00) while `TRAIN_NEW` is set.
    Train { pos: byte }
);

payload!(
    /// HLTV director command (engine message 50, `MSG_SPEC`: proxies only). TS writes the
    /// pre-SDK-2.3 layout: no length byte, the command (`director::DRC_EVENT`), then the primary
    /// and secondary entity and the event flags (priority in the low bits, `DRC_FLAG_*` above).
    /// Senders: `CHalfLifeMultiplay::DeathNotice` (0x88f7c) and `CBasePlayer::TakeDamage` (0x8fa94).
    DirectorEvent { command: byte, primary: short, secondary: short, flags: long }
);

/// HLTV director commands and event flags in TS's (pre-SDK-2.3) numbering. The client's
/// `HUD_DirectorEvent` (0x1001ab60) receives the same `(command, primary, secondary, flags)`.
pub mod director {
    /// Director (re)started: reset the spectator HUD (`CHudSpectator::DirectorMessage` 0x10023380).
    pub const DRC_ACTIVE: u8 = 0;
    /// Camera change: primary/secondary target (`DirectorMessage`'s other handled command).
    pub const DRC_CAMERA: u8 = 2;
    /// Game event for the director to rank (deaths, damage).
    pub const DRC_EVENT: u8 = 3;
    /// `DeathNotice` priority: 7 | `DRC_FLAG_DRAMATIC` (0x20).
    pub const FLAGS_DEATH: i32 = 0x27;
    /// `TakeDamage` priority.
    pub const FLAGS_DAMAGE: i32 = 5;
}

impl DirectorEvent {
    /// A `DRC_EVENT` about `primary` (the victim) caused by `secondary`.
    pub fn event(primary: i32, secondary: i32, flags: i32) -> Self {
        Self { command: director::DRC_EVENT, primary: primary as i16, secondary: secondary as i16, flags }
    }
}

impl TsFade {
    /// SDK `UTIL_ScreenFade` encoding: seconds → 4.12 fixed point, clamped to 16 bits.
    pub fn new(color: [u8; 3], alpha: u8, duration: f32, hold: f32, flags: i16) -> Self {
        let fixed = |s: f32| (s * 4096.0).clamp(0.0, 65535.0) as u16 as i16;
        Self { duration: fixed(duration), hold: fixed(hold), flags, r: color[0], g: color[1], b: color[2], a: alpha }
    }
}

/// `TSFade` flags (SDK FFADE_*).
pub mod fade {
    pub const IN: i16 = 0;
    pub const OUT: i16 = 1;
    pub const MODULATE: i16 = 2;
    pub const STAYOUT: i16 = 4;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let p = PwUp { bits: 0x108, secs: 3 };
        let mut b = Bytes::default();
        p.write(&mut b);
        assert_eq!(b.buf, [0x08, 0x01, 3]);
        assert_eq!(PwUp::read(&mut b), p);

        let d = PTakeDam { x: 1.0, y: -2.0, z: 3.5, dx: 0.0, dy: 1.0, dz: 0.0, amount: 140 };
        let mut b = Bytes::default();
        d.write(&mut b);
        assert_eq!(b.buf.len(), 13, "PTakeDam is registered with size 13");
        assert_eq!(PTakeDam::read(&mut b), d);
    }

    #[test]
    fn slowmo_coord() {
        let mut b = Bytes::default();
        TsSlowMo { factor_x100: 35.0 }.write(&mut b);
        assert_eq!(b.buf.len(), 2, "TSSlowMo is registered with size 2");
        assert_eq!(TsSlowMo::read(&mut b).factor_x100, 35.0);
    }

    #[test]
    fn director_event_layout() {
        let mut b = Bytes::default();
        DirectorEvent::event(3, 7, director::FLAGS_DEATH).write(&mut b);
        assert_eq!(b.buf, [3, 3, 0, 7, 0, 0x27, 0, 0, 0], "byte 3, short victim, short killer, long 0x27");
        assert_eq!(DirectorEvent::read(&mut b), DirectorEvent { command: 3, primary: 3, secondary: 7, flags: 0x27 });
    }

    /// Reading past the end gives zeros and empty strings, however far.
    #[test]
    fn reads_past_the_end() {
        for buf in [vec![], vec![0xff; 3], vec![b'x'; 5]] {
            let mut b = Bytes { buf, pos: 0 };
            let _ = PTakeDam::read(&mut b);
            let _ = DirectorEvent::read(&mut b);
            for _ in 0..8 {
                let _ = (WireRead::string(&mut b), WireRead::long(&mut b), WireRead::coord(&mut b));
            }
            assert_eq!((WireRead::long(&mut b), WireRead::string(&mut b)), (0, Vec::new()));
        }
        let mut b = Bytes::default();
        TsFade::new([1, 2, 3], 4, f32::NAN, f32::INFINITY, fade::OUT).write(&mut b);
        assert_eq!(b.buf.len(), 10);
    }

    /// A Latin-1/CP1252 `é` (0xE9) goes out and comes back as that one byte, not as U+FFFD.
    #[test]
    fn strings_are_bytes() {
        let mut b = Bytes::default();
        Wire::string(&mut b, b"Ren\xe9: caf\xe9\n");
        Wire::string(&mut b, b"a\0b");
        assert_eq!(b.buf, b"Ren\xe9: caf\xe9\n\0a\0");
        assert_eq!(WireRead::string(&mut b), b"Ren\xe9: caf\xe9\n");
        assert_eq!(WireRead::string(&mut b), b"a");
    }

    #[test]
    fn fade_fixed_point() {
        let f = TsFade::new([150, 10, 10], 75, 0.3, 0.15, fade::IN);
        assert_eq!(f.duration, (0.3f32 * 4096.0) as i16);
        let mut b = Bytes::default();
        f.write(&mut b);
        assert_eq!(b.buf.len(), 10);
    }
}
