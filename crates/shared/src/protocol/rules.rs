//! Message payloads for the rules area (layouts: decomp server/architecture.md §3.2,
//! server/gamerules-entities.md §7, client/architecture-hud.md §3). See `hud` for the
//! `Wire`/`WireRead` traits.

use super::hud_wire::{Wire, WireRead, payload};

payload!(
    /// `ScoreInfo` (9 bytes): `byte idx, short frags, short deaths, short 0 (class), short team+1`.
    ScoreInfo { idx: byte, frags: short, deaths: short, class: short, team: short }
);
payload!(
    /// `DeathMsg`: killer (0 if not a client), victim, weapon name.
    DeathMsg { killer: byte, victim: byte, weapon: string }
);
payload!(
    /// `TeamInfo`: player index and team name.
    TeamInfo { idx: byte, team: string }
);
payload!(
    /// `TeamScore`: `string team, short score, short 0`.
    TeamScore { team: string, score: short, deaths: short }
);
payload!(
    /// `GameMode` (1): 0 DM, otherwise `g_teamplay`.
    GameMode { teamplay: byte }
);
payload!(
    /// `SrvSett` (3): key 0 realbullet, 1 usecash, 2 ammocount, 3 weapon restriction mask.
    SrvSett { key: byte, value: short }
);
payload!(
    /// `RoundTime` (2): seconds left, state (0 running, 1 tenths, 2 frozen/over).
    RoundTime { secs: byte, state: byte }
);
payload!(
    /// `TSMessage`: coloured HUD text. `effect` is the style (1, 8, 16 seen), `hold` tenths of a
    /// second. Text ≤ 127 chars; `#TS_` words are client localisation keys.
    TsMessage { r: byte, g: byte, b: byte, effect: byte, hold: byte, text: string }
);
payload!(
    /// `SayText`: sender index and the full line (starts with `\x02`, ends with `\n`).
    SayText { sender: byte, text: string }
);
payload!(
    /// `MOTD`: final flag and a chunk of at most 60 characters.
    Motd { last: byte, chunk: string }
);
payload!(
    /// `ServerName`: the `hostname` cvar.
    ServerName { name: string }
);
payload!(
    /// `TSFlag` (4): state (0 home, 1 taken, 2 dropped), flag id, team+1, holder entindex.
    TsFlag { state: byte, id: byte, team: byte, holder: byte }
);
payload!(
    /// `TSHVoice` (1): 0 your team won/captured, 1 lost.
    TsHVoice { lost: byte }
);
payload!(
    /// `TSPSys` type 3 (wind): `byte 3, long seed, long yaw, coord strength`.
    TsPSysWind { kind: byte, seed: long, yaw: long, strength: coord }
);
payload!(
    /// `VGUIMenu` (1): menu id (2 = team select).
    VguiMenu { menu: byte }
);
payload!(
    /// `ShowObj` (2).
    ShowObj { idx: byte, visible: byte }
);
payload!(
    /// `Objective` (variable, ONE; `SendObjectiveToHud` 0x7ac80): the objective's centre, its
    /// list index and the receiver's team text.
    Objective { x: coord, y: coord, z: coord, idx: byte, text: string }
);
payload!(
    /// `Wires` (3, ONE): the three bomb wires the player sees (0 cut, 1 defuse, 2 hurry, 3
    /// detonate).
    Wires { w0: byte, w1: byte, w2: byte }
);

/// `TeamNames`: `byte n, n × string`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TeamNames {
    pub names: Vec<String>,
}

impl TeamNames {
    pub fn write(&self, w: &mut impl Wire) {
        w.byte(self.names.len() as i32);
        for n in &self.names {
            w.string(n);
        }
    }
    pub fn read(r: &mut impl WireRead) -> Self {
        let n = r.byte();
        Self { names: (0..n).map(|_| r.string()).collect() }
    }
}

/// `TextMsg`: destination (1 notify, 2 console, 3 talk, 4 center) and up to 4 parameters after
/// the format string.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TextMsg {
    pub dest: u8,
    pub text: String,
    pub params: Vec<String>,
}

pub mod hud_dest {
    pub const NOTIFY: u8 = 1;
    pub const CONSOLE: u8 = 2;
    pub const TALK: u8 = 3;
    pub const CENTER: u8 = 4;
}

impl TextMsg {
    pub fn write(&self, w: &mut impl Wire) {
        w.byte(self.dest as i32);
        w.string(&self.text);
        for p in self.params.iter().take(4) {
            w.string(p);
        }
    }
    /// The reader can't know how many parameters follow; pass the count the message has
    /// (the client reads until the buffer ends).
    pub fn read(r: &mut impl WireRead, params: usize) -> Self {
        let dest = r.byte();
        let text = r.string();
        Self { dest, text, params: (0..params.min(4)).map(|_| r.string()).collect() }
    }
}

/// `TSMessage` colours/styles the server uses (gamerules-entities.md conventions).
pub mod style {
    /// Kill/award lines: green, effect 1, 3 s.
    pub const GREEN: [u8; 3] = [0, 255, 0];
    /// Warnings: red-ish, effect 16, 2 s.
    pub const WARN: [u8; 3] = [195, 45, 45];
    /// Cheats/info: green-ish.
    pub const INFO: [u8; 3] = [45, 195, 45];
    /// The One announcements.
    pub const THE_ONE: [u8; 3] = [100, 150, 255];
}

#[cfg(test)]
mod tests {
    use super::super::hud_wire::Bytes;
    use super::*;

    /// The objective encoders match the client's decoders (`protocol::hud`).
    #[test]
    fn objective_payloads_decode() {
        use super::super::hud;
        let mut b = Bytes::default();
        Objective { x: 1.5, y: -2.0, z: 64.0, idx: 3, text: "Plant".into() }.write(&mut b);
        assert_eq!(hud::Objective::decode(&b.buf), Some(hud::Objective { x: 1.5, y: -2.0, z: 64.0, idx: 3, label: "Plant".into() }));
        let mut b = Bytes::default();
        Wires { w0: 3, w1: 0, w2: 2 }.write(&mut b);
        assert_eq!(b.buf.len(), 3, "Wires is registered with size 3");
        assert_eq!(hud::Wires::decode(&b.buf), Some(hud::Wires { w0: 3, w1: 0, w2: 2 }));
        let mut b = Bytes::default();
        ShowObj { idx: 0xff, visible: 1 }.write(&mut b);
        assert_eq!(hud::ShowObj::decode(&b.buf), Some(hud::ShowObj { idx: 0xff, visible: 1 }));
    }

    #[test]
    fn scoreinfo_size() {
        let mut b = Bytes::default();
        ScoreInfo { idx: 3, frags: -2, deaths: 7, class: 0, team: 1 }.write(&mut b);
        assert_eq!(b.buf.len(), 9, "ScoreInfo is registered with size 9");
        let s = ScoreInfo::read(&mut b);
        assert_eq!(s.frags, -2);
        assert_eq!(s.team, 1);
    }

    #[test]
    fn variable_messages() {
        let t = TeamNames { names: vec!["hitman".into(), "gordon".into()] };
        let mut b = Bytes::default();
        t.write(&mut b);
        assert_eq!(TeamNames::read(&mut b), t);

        let m = TsMessage { r: 0, g: 255, b: 0, effect: 8, hold: 30, text: "Round over".into() };
        let mut b = Bytes::default();
        m.write(&mut b);
        assert_eq!(TsMessage::read(&mut b), m);

        let mut b = Bytes::default();
        TextMsg { dest: hud_dest::TALK, text: "#Game_join".into(), params: vec!["bob".into()] }.write(&mut b);
        assert_eq!(TextMsg::read(&mut b, 1).params, ["bob"]);
    }

    /// The readers take any bytes: short, all 0xff, unterminated or non-UTF-8 strings.
    #[test]
    fn readers_survive_garbage() {
        let mut x = 0xdead_beef_cafe_f00du64;
        let mut next = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let mut bufs: Vec<Vec<u8>> = vec![vec![], vec![0xff], vec![0xff; 600], vec![b'x'; 40]];
        for _ in 0..1000 {
            let len = (next() % 40) as usize;
            bufs.push((0..len).map(|_| next() as u8).collect());
        }
        let rd = |b: &Vec<u8>| Bytes { buf: b.clone(), pos: 0 };
        for b in &bufs {
            let _ = ScoreInfo::read(&mut rd(b));
            let _ = DeathMsg::read(&mut rd(b));
            let _ = TeamInfo::read(&mut rd(b));
            let _ = TeamScore::read(&mut rd(b));
            let _ = SrvSett::read(&mut rd(b));
            let _ = RoundTime::read(&mut rd(b));
            let _ = TsMessage::read(&mut rd(b));
            let _ = SayText::read(&mut rd(b));
            let _ = Motd::read(&mut rd(b));
            let _ = TsFlag::read(&mut rd(b));
            let _ = TsPSysWind::read(&mut rd(b));
            let t = TeamNames::read(&mut rd(b));
            assert!(t.names.len() <= 255);
            let m = TextMsg::read(&mut rd(b), usize::MAX);
            assert!(m.params.len() <= 4);
        }
    }

    #[test]
    fn fixed_sizes_match_registration() {
        use super::super::Msg;
        let mut b = Bytes::default();
        SrvSett { key: 1, value: 1 }.write(&mut b);
        assert_eq!(b.buf.len() as i32, Msg::SrvSett.size());
        let mut b = Bytes::default();
        TsFlag::default().write(&mut b);
        assert_eq!(b.buf.len() as i32, Msg::TsFlag.size());
        let mut b = Bytes::default();
        RoundTime::default().write(&mut b);
        assert_eq!(b.buf.len() as i32, Msg::RoundTime.size());
    }
}
