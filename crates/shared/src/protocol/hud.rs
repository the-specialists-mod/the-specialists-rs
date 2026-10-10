//! Message payloads for the hud area (layouts: decomp server/architecture.md §3.2,
//! client/architecture-hud.md §3). Field order is the TS client's read order.
//!
//! Each payload has `decode` (what the client does) and `encode` (the matching server writes:
//! `B`=byte, `C`=char, `S`=short, `L`=long, `Co`=coord as `short(v*8)`, `Str`=NUL-terminated).
//! The server may use the encoders through `MsgWriter`-equivalent calls; tests use them to build
//! byte buffers.

/// Little-endian reader mirroring the client's `READ_*` (architecture-hud.md §3 table). Reading
/// past the end sets `bad` and yields zeros, like the SDK's `BEGIN_READ` family.
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
    pub bad: bool,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Reader { buf, pos: 0, bad: false }
    }
    fn take<const N: usize>(&mut self) -> [u8; N] {
        match self.buf.get(self.pos..self.pos + N) {
            Some(b) => {
                self.pos += N;
                b.try_into().unwrap()
            }
            None => {
                self.pos = self.buf.len();
                self.bad = true;
                [0; N]
            }
        }
    }
    pub fn byte(&mut self) -> u8 {
        self.take::<1>()[0]
    }
    pub fn char(&mut self) -> i8 {
        self.take::<1>()[0] as i8
    }
    pub fn short(&mut self) -> i16 {
        i16::from_le_bytes(self.take())
    }
    pub fn long(&mut self) -> i32 {
        i32::from_le_bytes(self.take())
    }
    /// `READ_COORD`: `short / 8`.
    pub fn coord(&mut self) -> f32 {
        self.short() as f32 * 0.125
    }
    /// `READ_STRING`: up to the NUL (or the end of the message). Latin-1 bytes are kept as-is
    /// by mapping each byte to the matching char.
    pub fn string(&mut self) -> String {
        let rest = &self.buf[self.pos.min(self.buf.len())..];
        let end = rest.iter().position(|&b| b == 0);
        let bytes = &rest[..end.unwrap_or(rest.len())];
        self.pos += bytes.len() + end.map_or(0, |_| 1);
        if end.is_none() && bytes.is_empty() {
            self.bad = true;
        }
        bytes.iter().map(|&b| b as char).collect()
    }
    pub fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }
}

/// Byte writer matching the server's `WRITE_*` calls.
#[derive(Default)]
pub struct Writer(pub Vec<u8>);

impl Writer {
    pub fn byte(&mut self, v: u8) -> &mut Self {
        self.0.push(v);
        self
    }
    pub fn char(&mut self, v: i8) -> &mut Self {
        self.0.push(v as u8);
        self
    }
    pub fn short(&mut self, v: i16) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn long(&mut self, v: i32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn coord(&mut self, v: f32) -> &mut Self {
        self.short((v * 8.0) as i16)
    }
    pub fn string(&mut self, s: &str) -> &mut Self {
        self.0.extend(s.chars().map(|c| if (c as u32) < 256 { c as u8 } else { b'?' }));
        self.0.push(0);
        self
    }
}

macro_rules! payload {
    ($(#[$m:meta])* $name:ident { $($field:ident : $ty:ty = $read:ident),* $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, PartialEq, Default)]
        pub struct $name { $(pub $field: $ty),* }
        impl $name {
            pub fn decode(buf: &[u8]) -> Option<Self> {
                let mut r = Reader::new(buf);
                let v = $name { $($field: payload!(@read r, $read, $ty)),* };
                (!r.bad).then_some(v)
            }
            pub fn encode(&self) -> Vec<u8> {
                let mut w = Writer::default();
                $(payload!(@write w, $read, self.$field);)*
                w.0
            }
        }
    };
    (@read $r:ident, byte, $ty:ty) => { $r.byte() as $ty };
    (@read $r:ident, char, $ty:ty) => { $r.char() as $ty };
    (@read $r:ident, short, $ty:ty) => { $r.short() as $ty };
    (@read $r:ident, long, $ty:ty) => { $r.long() as $ty };
    (@read $r:ident, coord, $ty:ty) => { $r.coord() as $ty };
    (@read $r:ident, string, $ty:ty) => { $r.string() };
    (@write $w:ident, byte, $v:expr) => { $w.byte($v as u8) };
    (@write $w:ident, char, $v:expr) => { $w.char($v as i8) };
    (@write $w:ident, short, $v:expr) => { $w.short($v as i16) };
    (@write $w:ident, long, $v:expr) => { $w.long($v as i32) };
    (@write $w:ident, coord, $v:expr) => { $w.coord($v as f32) };
    (@write $w:ident, string, $v:expr) => { $w.string(&$v) };
}

// ---- CHudTSStatus messages (architecture-hud.md §3.2) -----------------------------------------

payload!(
    /// `TSHealth` (1004b730): byte hp.
    TsHealth { hp: u8 = byte });
payload!(
    /// `Train` (CHudTrain::MsgFunc_Train 10034df0): byte speed notch; 0 hides the train HUD.
    Train { pos: u8 = byte });
payload!(
    /// `HideWeapon` (CHudAmmo 1001a110): byte `m_iHideHUDDisplay` (HIDEHUD_WEAPONS 1, _ALL 4).
    HideWeapon { flags: u8 = byte });
payload!(
    /// `TSArmor` (1004b920): byte kevlar.
    TsArmor { armor: u8 = byte });
payload!(
    /// `TSState` (1004b950): 0 stand, 1 crouch, 2 prone.
    TsState { stance: u8 = byte });
payload!(
    /// `TSCash` (1004b980): short credits (signed read).
    TsCash { cash: i16 = short });
payload!(
    /// `TSSpace` (1004b9b0): byte free inventory slots.
    TsSpace { slots: u8 = byte });
payload!(
    /// `PwUp` (1004b9e0): short flags, byte value.
    PwUp { flags: u16 = short, value: u8 = byte });
payload!(
    /// `PwUpTime` (1004bad0): byte seconds left.
    PwUpTime { secs: u8 = byte });
payload!(
    /// `RoundTime` (1004bb20): byte seconds, byte mode (0 = 1 s tick, 1 = 0.1 s tick, 2 = frozen).
    RoundTime { secs: u8 = byte, mode: u8 = byte });
payload!(
    /// `Objective` (1004bb70): coord x, y, z, byte idx, string label.
    Objective { x: f32 = coord, y: f32 = coord, z: f32 = coord, idx: u8 = byte, label: String = string });
payload!(
    /// `ShowObj` (1004bc80): byte idx, byte visible.
    ShowObj { idx: u8 = byte, visible: u8 = byte });
payload!(
    /// `TSMessage` (1004c0a0): byte r, g, b, byte flags, byte time (tenths of a second; the client
    /// takes `|time|` capped at 100), string. Flags: 1 top, 8 bottom, 2 left, 4 right, 0x10 replace.
    TsMessage { r: u8 = byte, g: u8 = byte, b: u8 = byte, flags: u8 = byte, time: u8 = byte, text: String = string });
payload!(
    /// `PTakeDam` (1004bcd0): coord×3 pos, coord×3 dir, byte amount.
    PTakeDam { px: f32 = coord, py: f32 = coord, pz: f32 = coord, dx: f32 = coord, dy: f32 = coord, dz: f32 = coord, amount: u8 = byte });
payload!(
    /// `TSSlowMo` (1004bef0): coord = factor×100 (0 means 1.0).
    TsSlowMo { factor100: f32 = coord });
payload!(
    /// `TSPAward` (1004b790): byte client (1..64), byte award.
    TsPAward { client: u8 = byte, award: u8 = byte });
payload!(
    /// `SrvSett` (1004b840): byte key, short value.
    SrvSett { key: u8 = byte, value: i16 = short });
payload!(
    /// `TSBTime` (1004b8e0): byte on.
    TsBTime { on: u8 = byte });
payload!(
    /// `TSHVoice` (1004c180): 0 = successful.wav, 1 = failed.wav.
    TsHVoice { lost: u8 = byte });
payload!(
    /// `TSFlag` (1004ba50): client read order byte state, byte idx (1..15), byte team, byte carrier.
    TsFlag { state: u8 = byte, idx: u8 = byte, team: u8 = byte, carrier: u8 = byte });
payload!(
    /// `TSMBlur` (1004bf90): byte entity, byte mode (ignored).
    TsMBlur { entity: u8 = byte, mode: u8 = byte });
payload!(
    /// `PlInfo` (1004b7e0): byte client, byte key (0 = laser colour), byte value.
    PlInfo { client: u8 = byte, key: u8 = byte, value: u8 = byte });
payload!(
    /// `TSFade` (1004c1e0): short duration, short hold (both 4.12 fixed seconds), short flags,
    /// byte r, g, b, a.
    TsFade { duration: u16 = short, hold: u16 = short, flags: u16 = short, r: u8 = byte, g: u8 = byte, b: u8 = byte, a: u8 = byte });

/// `TSPSys` (1004bfd0): only type 3 (wind) carries data: long seed, long yaw, coord strength.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TsPSys {
    pub kind: u8,
    pub seed: i32,
    pub yaw: i32,
    pub strength: f32,
}
impl TsPSys {
    pub fn decode(buf: &[u8]) -> Option<Self> {
        let mut r = Reader::new(buf);
        let kind = r.byte();
        let mut v = TsPSys { kind, ..Default::default() };
        if kind == 3 {
            v.seed = r.long();
            v.yaw = r.long();
            v.strength = r.coord();
        }
        (!r.bad).then_some(v)
    }
}

/// `TSFrags` (1004bd50): coord×3 origin, coord×3 size, byte type; type 0 adds coord×6 (hit pos,
/// dir), long damage bits, coord damage.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TsFrags {
    pub origin: [f32; 3],
    pub size: [f32; 3],
    pub kind: u8,
    pub hit_pos: [f32; 3],
    pub hit_dir: [f32; 3],
    pub dmg_bits: i32,
    pub damage: f32,
}
impl TsFrags {
    pub fn decode(buf: &[u8]) -> Option<Self> {
        let mut r = Reader::new(buf);
        let mut v = TsFrags::default();
        for o in &mut v.origin {
            *o = r.coord();
        }
        for o in &mut v.size {
            *o = r.coord();
        }
        v.kind = r.byte();
        if v.kind == 0 {
            for o in &mut v.hit_pos {
                *o = r.coord();
            }
            for o in &mut v.hit_dir {
                *o = r.coord();
            }
            v.dmg_bits = r.long();
            v.damage = r.coord();
        }
        (!r.bad).then_some(v)
    }
}

// ---- CHudTSWeapons messages (architecture-hud.md §3.3) ----------------------------------------

payload!(
    /// `WeaponInfo` (1004f800): byte id (0xff = keep), byte clip, short reserve, byte fire mode,
    /// byte attachment flags.
    WeaponInfo { id: u8 = byte, clip: u8 = byte, reserve: i16 = short, mode: u8 = byte, attachments: u8 = byte });
payload!(
    /// `ClipInfo` (1004f780): byte clip.
    ClipInfo { clip: u8 = byte });
payload!(
    /// `WStatus` (1004f990): byte id, byte owned, byte clip.
    WStatus { id: u8 = byte, owned: u8 = byte, clip: u8 = byte });
payload!(
    /// `ActItems` (1004fa70): byte client (0..32), byte active attachment bits.
    ActItems { client: u8 = byte, bits: u8 = byte });
payload!(
    /// `NextStunt` (1004fac0): byte cooldown in tenths of a second.
    NextStunt { tenths: u8 = byte });
payload!(
    /// `Wires` (1004fb00): byte w0, w1, w2 (bomb-defuse wires; nonzero = intact).
    Wires { w0: u8 = byte, w1: u8 = byte, w2: u8 = byte });
payload!(
    /// `KFuPower` (1004f6f0): char -100..100.
    KFuPower { power: i8 = char });

/// `CustomWP` (1004fb50): `"%i %f %f %f %f %i"` = id, accuracy, view kick, accuracy kick,
/// damage, cost.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CustomWp {
    pub id: i32,
    pub accuracy: f32,
    pub view_kick: f32,
    pub acc_kick: f32,
    pub damage: f32,
    pub cost: i32,
}
impl CustomWp {
    pub fn decode(buf: &[u8]) -> Option<Self> {
        let s = Reader::new(buf).string();
        let mut it = s.split_whitespace();
        Some(CustomWp {
            id: it.next()?.parse().ok()?,
            accuracy: it.next()?.parse().ok()?,
            view_kick: it.next()?.parse().ok()?,
            acc_kick: it.next()?.parse().ok()?,
            damage: it.next()?.parse().ok()?,
            cost: it.next()?.parse().ok()?,
        })
    }
}

// ---- Stock messages with TS use --------------------------------------------------------------

payload!(
    /// `ScoreInfo` (10065090): byte client, short frags, short deaths, short class, short team.
    ScoreInfo { client: u8 = byte, frags: i16 = short, deaths: i16 = short, class: i16 = short, team: i16 = short });
payload!(
    /// `TeamInfo`: byte client, string team name.
    TeamInfo { client: u8 = byte, team: String = string });
payload!(
    /// `TeamScore`: string team, short frags, short deaths.
    TeamScore { team: String = string, frags: i16 = short, deaths: i16 = short });
payload!(
    /// `DeathMsg`: byte killer, byte victim, string weapon.
    DeathMsg { killer: u8 = byte, victim: u8 = byte, weapon: String = string });
payload!(
    /// `SayText`: byte client, string text.
    SayText { client: u8 = byte, text: String = string });
payload!(
    /// `ShowMenu` (CHudMenu): short valid slots, char display time, byte need-more, string.
    ShowMenu { slots: u16 = short, time: i8 = char, more: u8 = byte, text: String = string });
payload!(
    /// `MOTD` (10064f60): byte final chunk, string chunk.
    Motd { last: u8 = byte, text: String = string });
payload!(
    /// `Spectator` (10065250): byte client, byte observer.
    Spectator { client: u8 = byte, observer: u8 = byte });
payload!(
    /// `CurWeapon`: byte state, byte id, byte clip.
    CurWeapon { state: u8 = byte, id: i8 = char, clip: i8 = char });
payload!(
    /// `HudText`: string titles.txt key.
    HudText { text: String = string });

/// `TextMsg`: byte destination (1 notify, 2 console, 3 talk, 4 centre), then up to five strings
/// (message and `%s` arguments).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TextMsg {
    pub dest: u8,
    pub strings: Vec<String>,
}
impl TextMsg {
    pub fn decode(buf: &[u8]) -> Option<Self> {
        let mut r = Reader::new(buf);
        let dest = r.byte();
        let mut strings = Vec::new();
        while r.remaining() > 0 && strings.len() < 5 {
            strings.push(r.string());
        }
        (!r.bad).then_some(TextMsg { dest, strings })
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::default();
        w.byte(self.dest);
        for s in &self.strings {
            w.string(s);
        }
        w.0
    }
}

/// `TeamNames`: byte count, then that many strings.
pub fn decode_team_names(buf: &[u8]) -> Option<Vec<String>> {
    let mut r = Reader::new(buf);
    let n = r.byte();
    let v: Vec<String> = (0..n).map(|_| r.string()).collect();
    (!r.bad).then_some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ts_message_roundtrip() {
        let m = TsMessage { r: 255, g: 10, b: 0, flags: 8, time: 30, text: "#HUD_Hi".into() };
        let bytes = m.encode();
        assert_eq!(bytes, [255, 10, 0, 8, 30, b'#', b'H', b'U', b'D', b'_', b'H', b'i', 0]);
        assert_eq!(TsMessage::decode(&bytes), Some(m));
    }

    #[test]
    fn weapon_info_layout() {
        // B id, B clip, S reserve, B mode, B extra = 6 bytes (TS size 6).
        let w = WeaponInfo { id: 13, clip: 30, reserve: 90, mode: 1, attachments: 0x0a };
        let b = w.encode();
        assert_eq!(b.len(), 6);
        assert_eq!(b, [13, 30, 90, 0, 1, 0x0a]);
        assert_eq!(WeaponInfo::decode(&b), Some(w));
        assert_eq!(WeaponInfo::decode(&b[..5]), None);
    }

    #[test]
    fn coord_and_slowmo() {
        let b = TsSlowMo { factor100: 35.0 }.encode();
        assert_eq!(b, (35 * 8i16).to_le_bytes());
        assert_eq!(TsSlowMo::decode(&b).unwrap().factor100, 35.0);
    }

    #[test]
    fn objective_and_sizes() {
        let o = Objective { x: 1.5, y: -2.0, z: 64.0, idx: 3, label: "Base".into() };
        assert_eq!(Objective::decode(&o.encode()), Some(o));
        assert_eq!(PTakeDam::default().encode().len(), 13);
        assert_eq!(ScoreInfo::default().encode().len(), 9);
        assert_eq!(TsFade::default().encode().len(), 10);
        assert_eq!(PwUp::default().encode().len(), 3);
        assert_eq!(TsFlag::default().encode().len(), 4);
    }

    #[test]
    fn text_msg_and_custom_wp() {
        let t = TextMsg { dest: 4, strings: vec!["#Game_joined".into(), "Bob".into()] };
        assert_eq!(TextMsg::decode(&t.encode()), Some(t));
        let mut w = Writer::default();
        w.string("13 0.002500 5.500000 0.000100 39.000000 3000");
        let c = CustomWp::decode(&w.0).unwrap();
        assert_eq!((c.id, c.cost), (13, 3000));
        assert!((c.damage - 39.0).abs() < 1e-6);
    }

    #[test]
    fn frags_variants() {
        let mut w = Writer::default();
        w.coord(1.0).coord(2.0).coord(3.0).coord(10.0).coord(0.0).coord(0.0).byte(1);
        let f = TsFrags::decode(&w.0).unwrap();
        assert_eq!(f.kind, 1);
        assert_eq!(f.size[0], 10.0);
        w.0.pop();
        w.byte(0);
        assert!(TsFrags::decode(&w.0).is_none(), "type 0 needs the hit block");
    }
}
