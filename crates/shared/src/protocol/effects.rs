//! Message payloads for the effects area (layouts: decomp server/architecture.md §3.2,
//! client/architecture-hud.md §3, client/rendering-effects-prediction.md §4.7, §5.5, §7.5).
//!
//! Each payload has `decode(&[u8])` (client) and `encode(&mut impl Sink)` (server: implement
//! [`Sink`] over the engine message writer; `Vec<u8>` implements it for tests). Coords are the
//! engine's `WRITE_COORD`: a little-endian short of `value * 8`.

/// Destination for `encode`: mirrors `WRITE_BYTE`/`WRITE_SHORT`/`WRITE_LONG`/`WRITE_COORD`.
pub trait Sink {
    fn byte(&mut self, v: i32);
    fn short(&mut self, v: i32);
    fn long(&mut self, v: i32);
    fn coord(&mut self, v: f32);
}

impl Sink for Vec<u8> {
    fn byte(&mut self, v: i32) {
        self.push(v as u8);
    }
    fn short(&mut self, v: i32) {
        self.extend_from_slice(&(v as i16).to_le_bytes());
    }
    fn long(&mut self, v: i32) {
        self.extend_from_slice(&v.to_le_bytes());
    }
    fn coord(&mut self, v: f32) {
        self.short((v * 8.0) as i32);
    }
}

/// Bounds-checked reader; `ok()` is false once a read ran past the end.
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
    bad: bool,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0, bad: false }
    }
    fn take<const N: usize>(&mut self) -> [u8; N] {
        match self.buf.get(self.pos..self.pos + N) {
            Some(b) => {
                self.pos += N;
                b.try_into().unwrap()
            }
            None => {
                self.bad = true;
                [0; N]
            }
        }
    }
    pub fn byte(&mut self) -> u8 {
        self.take::<1>()[0]
    }
    pub fn short(&mut self) -> i16 {
        i16::from_le_bytes(self.take())
    }
    pub fn long(&mut self) -> i32 {
        i32::from_le_bytes(self.take())
    }
    pub fn coord(&mut self) -> f32 {
        self.short() as f32 * 0.125
    }
    pub fn vec(&mut self) -> [f32; 3] {
        [self.coord(), self.coord(), self.coord()]
    }
    pub fn ok(&self) -> bool {
        !self.bad
    }
}

fn vec<S: Sink>(s: &mut S, v: [f32; 3]) {
    for c in v {
        s.coord(c);
    }
}

/// `TSFade` (size -1): TS's replacement for `ScreenFade`. Durations are in 1/4096 s, already
/// divided by the receiver's slow-motion multiplier. `flags` is sent but ignored by the client.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TsFade {
    pub duration: f32,
    pub hold: f32,
    pub flags: i16,
    pub rgba: [u8; 4],
}

impl TsFade {
    pub fn decode(b: &[u8]) -> Option<Self> {
        let mut r = Reader::new(b);
        let duration = r.short() as u16 as f32 / 4096.0;
        let hold = r.short() as u16 as f32 / 4096.0;
        let flags = r.short();
        let rgba = [r.byte(), r.byte(), r.byte(), r.byte()];
        r.ok().then_some(Self { duration, hold, flags, rgba })
    }
    pub fn encode<S: Sink>(&self, s: &mut S) {
        s.short(((self.duration * 4096.0) as i32).clamp(0, 0xffff));
        s.short(((self.hold * 4096.0) as i32).clamp(0, 0xffff));
        s.short(self.flags as i32);
        for c in self.rgba {
            s.byte(c as i32);
        }
    }
}

/// `TSPSys` (size -1), subtype 3 = wind (`CTSWind::BlowThink`). Other subtypes are ignored.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TsWind {
    pub seed: i32,
    pub yaw: i32,
    pub speed: f32,
}

impl TsWind {
    pub const SUBTYPE: u8 = 3;
    pub fn decode(b: &[u8]) -> Option<Self> {
        let mut r = Reader::new(b);
        if r.byte() != Self::SUBTYPE {
            return None;
        }
        let w = Self { seed: r.long(), yaw: r.long(), speed: r.coord() };
        r.ok().then_some(w)
    }
    pub fn encode<S: Sink>(&self, s: &mut S) {
        s.byte(Self::SUBTYPE as i32);
        s.long(self.seed);
        s.long(self.yaw);
        s.coord(self.speed);
    }
}

/// `PTakeDam` (13 bytes): blood/impact effect where a player was hit.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PTakeDam {
    pub pos: [f32; 3],
    pub dir: [f32; 3],
    /// Bit 7: the hit was absorbed by kevlar. Low 7 bits: `min(dmg / 2, 127)` (x8 for buckshot).
    pub amount: u8,
}

impl PTakeDam {
    pub fn kevlar(&self) -> bool {
        self.amount & 0x80 != 0
    }
    pub fn strength(&self) -> i32 {
        (self.amount & 0x7f) as i32
    }
    pub fn decode(b: &[u8]) -> Option<Self> {
        let mut r = Reader::new(b);
        let d = Self { pos: r.vec(), dir: r.vec(), amount: r.byte() };
        r.ok().then_some(d)
    }
    pub fn encode<S: Sink>(&self, s: &mut S) {
        vec(s, self.pos);
        vec(s, self.dir);
        s.byte(self.amount as i32);
    }
}

/// `TSFrags` (size -1, server `gmsgTSCreateDebris`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TsFrags {
    /// `CBreakable::Die` / `CBaseDoor` with glass material: shards fill the box.
    Glass { origin: [f32; 3], size: [f32; 3], hit: [f32; 3], dir: [f32; 3], dmg_bits: i32, force: f32 },
    /// `TSExplosion`: `v2 = (radius, 0, 0)`.
    Explosion { origin: [f32; 3], radius: f32 },
    /// No server sender in 3.0: sparks along `dir`.
    Sparks { origin: [f32; 3], dir: [f32; 3] },
}

impl TsFrags {
    pub fn decode(b: &[u8]) -> Option<Self> {
        let mut r = Reader::new(b);
        let origin = r.vec();
        let v2 = r.vec();
        let f = match r.byte() {
            0 => TsFrags::Glass { origin, size: v2, hit: r.vec(), dir: r.vec(), dmg_bits: r.long(), force: r.coord() },
            1 => TsFrags::Explosion { origin, radius: v2[0] },
            2 => TsFrags::Sparks { origin, dir: v2 },
            _ => return None,
        };
        r.ok().then_some(f)
    }
    pub fn encode<S: Sink>(&self, s: &mut S) {
        match *self {
            TsFrags::Glass { origin, size, hit, dir, dmg_bits, force } => {
                vec(s, origin);
                vec(s, size);
                s.byte(0);
                vec(s, hit);
                vec(s, dir);
                s.long(dmg_bits);
                s.coord(force);
            }
            TsFrags::Explosion { origin, radius } => {
                vec(s, origin);
                vec(s, [radius, 0.0, 0.0]);
                s.byte(1);
            }
            TsFrags::Sparks { origin, dir } => {
                vec(s, origin);
                vec(s, dir);
                s.byte(2);
            }
        }
    }
}

/// `TSMBlur` (2 bytes): start afterimage ghosts on an entity. Level 0..2.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TsMBlur {
    pub entindex: u8,
    pub level: u8,
}

impl TsMBlur {
    pub fn decode(b: &[u8]) -> Option<Self> {
        let mut r = Reader::new(b);
        let m = Self { entindex: r.byte(), level: r.byte() };
        r.ok().then_some(m)
    }
    pub fn encode<S: Sink>(&self, s: &mut S) {
        s.byte(self.entindex as i32);
        s.byte(self.level as i32);
    }
}

/// `TSSlowMo` (2 bytes): coord `scale * 100`; 0 means 1.0.
pub fn decode_slowmo(b: &[u8]) -> Option<f32> {
    let mut r = Reader::new(b);
    let v = r.coord() * 0.01;
    r.ok().then_some(if v == 0.0 { 1.0 } else { v })
}

/// `TSBTime` (1 byte): bullet-time flag.
pub fn decode_btime(b: &[u8]) -> Option<bool> {
    b.first().map(|&v| v != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips() {
        let f = TsFade { duration: 1.5, hold: 0.25, flags: 1, rgba: [255, 0, 0, 200] };
        let mut b = Vec::new();
        f.encode(&mut b);
        assert_eq!(b.len(), 10);
        assert_eq!(TsFade::decode(&b), Some(f));

        let w = TsWind { seed: 1234, yaw: -30, speed: 40.5 };
        let mut b = Vec::new();
        w.encode(&mut b);
        assert_eq!(TsWind::decode(&b), Some(w));
        b[0] = 1;
        assert_eq!(TsWind::decode(&b), None);

        let d = PTakeDam { pos: [1.0, -2.5, 3.0], dir: [0.0, 1.0, 0.0], amount: 0x80 | 12 };
        let mut b = Vec::new();
        d.encode(&mut b);
        assert_eq!(b.len(), 13, "PTakeDam is registered with size 13");
        let d2 = PTakeDam::decode(&b).unwrap();
        assert_eq!(d2, d);
        assert!(d2.kevlar());
        assert_eq!(d2.strength(), 12);

        for f in [
            TsFrags::Glass { origin: [1.0; 3], size: [2.0; 3], hit: [3.0; 3], dir: [0.0, 0.0, 1.0], dmg_bits: 2, force: 50.0 },
            TsFrags::Explosion { origin: [8.0, 0.0, 0.0], radius: 300.0 },
            TsFrags::Sparks { origin: [0.0; 3], dir: [1.0, 0.0, 0.0] },
        ] {
            let mut b = Vec::new();
            f.encode(&mut b);
            assert_eq!(TsFrags::decode(&b), Some(f));
        }
        assert_eq!(decode_slowmo(&[0, 0]), Some(1.0));
        let mut b = Vec::new();
        b.coord(35.0);
        assert_eq!(decode_slowmo(&b), Some(0.35));
        assert!(TsFade::decode(&[1, 2]).is_none());
    }
}
