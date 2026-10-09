//! Server-side encoders for the HUD messages (rules package). The client decodes the same payloads
//! with `protocol::hud` (HUD package); both follow the decomp layouts.

//! Message payloads for the hud area (layouts: decomp server/architecture.md §3.2,
//! client/architecture-hud.md §3).
//!
//! Every payload is a plain struct with `write` (server) and `read` (client). Both go through the
//! small [`Wire`] / [`WireRead`] traits so the server can map them onto `pfnWrite*` (the engine
//! encodes coords) and the client onto its message reader. `Bytes` implements both with the
//! GoldSrc encoding (little endian, coord = short ×8) for tests and tools.

/// Writer side of a user message (`WRITE_BYTE` ...).
pub trait Wire {
    fn byte(&mut self, v: i32);
    fn char(&mut self, v: i32);
    fn short(&mut self, v: i32);
    fn long(&mut self, v: i32);
    fn coord(&mut self, v: f32);
    fn string(&mut self, s: &str);
}

/// Reader side of a user message (`READ_BYTE` ...). Readers return 0 / "" past the end.
pub trait WireRead {
    fn byte(&mut self) -> u8;
    fn char(&mut self) -> i8;
    fn short(&mut self) -> i16;
    fn long(&mut self) -> i32;
    fn coord(&mut self) -> f32;
    fn string(&mut self) -> String;
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
    fn string(&mut self, s: &str) {
        self.buf.extend(s.bytes().filter(|&b| b != 0));
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
    fn string(&mut self) -> String {
        let rest = self.buf.get(self.pos..).unwrap_or(&[]);
        let n = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
        let s = String::from_utf8_lossy(&rest[..n]).into_owned();
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
    (@w $w:ident, $v:expr, byte) => { $w.byte($v as i32) };
    (@w $w:ident, $v:expr, char) => { $w.char($v as i32) };
    (@w $w:ident, $v:expr, short) => { $w.short($v as i32) };
    (@w $w:ident, $v:expr, long) => { $w.long($v) };
    (@w $w:ident, $v:expr, coord) => { $w.coord($v) };
    (@w $w:ident, $v:expr, string) => { $w.string(&$v) };
    (@r $r:ident, byte) => { $r.byte() };
    (@r $r:ident, char) => { $r.char() };
    (@r $r:ident, short) => { $r.short() };
    (@r $r:ident, long) => { $r.long() };
    (@r $r:ident, coord) => { $r.coord() };
    (@r $r:ident, string) => { $r.string() };
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
    fn fade_fixed_point() {
        let f = TsFade::new([150, 10, 10], 75, 0.3, 0.15, fade::IN);
        assert_eq!(f.duration, (0.3f32 * 4096.0) as i16);
        let mut b = Bytes::default();
        f.write(&mut b);
        assert_eq!(b.buf.len(), 10);
    }
}
