//! TS 2.1 on TS 3.0's wire (issue #100 §1.2; decomp `docs/ts21/client-protocol.md` §1-§3). The
//! server keeps 3.0's message table, sizes and encodings in 2.1 mode, so TS 3.0's client and the
//! port's client both play it; 2.1's values travel in 3.0's messages.
//!
//! * [`outgoing_message`] (`ServerRules::outgoing_message`) puts a message sent in 2.1's shape
//!   into 3.0's: `ScreenFade` goes out as `TSFade`, a 13-byte `TSFrags` type 0 gets 3.0's tail,
//!   a 2-byte `WStatus` its count.
//! * 2.1's own messages aren't in the table, so they can't be sent: `DelObj` is [`del_obj`]
//!   (`ShowObj(idx, 0)`); `Flashlight` and `FlashBat` are never sent (TS flashlights are weapon
//!   attachments, `ActItems`); 2.1 registered `GloSlow` and `KAlign` but never sent them.
//! * The player's state keeps 3.0's encodings, which the shared movement reads on both sides:
//!   `fuser1` is 1.0 at normal speed ([`time_scale`]) and `iuser4` has 3.0's stunt bits
//!   ([`stunt_bits`]). Code ported from 2.1 converts with these when it writes the state.
//!
//! `KFuPower` needs nothing: 2.1 wrote the bar (0..100) as a byte and 3.0 as a char, the same
//! byte on the wire. `Objective` needs no `ShowObj(idx, 1)`: TS 3.0's client makes the slot
//! visible itself (`mov byte [slot+0xa30], 1` at 0x1004bc50).

use goldsrc::server::{MsgOp, MsgWriter};
use ts_server::damage::bits::BULLET as DMG_BULLET;
use ts_server::messages::{self, Msg};
use ts_server::rules::msg;

/// `ServerRules::outgoing_message` in 2.1.
pub fn outgoing_message(m: Msg, w: MsgWriter, send: &mut dyn FnMut(MsgWriter)) {
    send(translate(m, w, messages::id));
}

/// 2.1's message shape `w` (of `m`) in 3.0's, with `id` the registered id of a message.
fn translate(m: Msg, mut w: MsgWriter, id: impl Fn(Msg) -> i32) -> MsgWriter {
    match m {
        // 2.1's `UTIL_ScreenFade` (FUN_10067180) sends `ScreenFade`. TS 3.0's client rewrites
        // the engine's fade every frame, so 3.0 sends the same 10-byte body as `TSFade`.
        Msg::ScreenFade => w.set_msg_type(id(Msg::TsFade)),
        // 2.1's type 0 (a breakable's glass, `CBreakable::Die` 0x10042540) is 13 bytes; 3.0's
        // client reads the hit position and direction, the damage bits and the damage after the
        // type, so a short one would give it garbage debris. 2.1's client (0x10081840) throws
        // every shard `(RandomFloat(-9, 9), RandomFloat(-9, 9), RandomLong(0, 10))`, which is
        // 3.0's bullet branch (`dmgbits & 0x08000002`, 0x10047cb0) with no force: so the tail is
        // a zero hit and direction, `DMG_BULLET` and a zero force (issue #232). Zero bits would
        // take 3.0's other branch, every shard flying toward +x, +y and +z.
        Msg::TsFrags if w.ops().len() == 7 && w.ops()[6] == MsgOp::Byte(0) => {
            let ops = w.ops_mut();
            ops.extend(std::iter::repeat_n(MsgOp::Coord(0.0), 6));
            ops.push(MsgOp::Long(DMG_BULLET));
            ops.push(MsgOp::Coord(0.0));
        }
        // 2.1's `WStatus` is `id, owned` (`CTSGun::UpdateClientData` 0x1004d8c0), and its client
        // puts a count of 1 in the equipment grid; 3.0's is registered with 3 bytes.
        Msg::WStatus if w.ops().len() == 2 => w.ops_mut().push(MsgOp::Byte(1)),
        _ => {}
    }
    w
}

/// 2.1's `DelObj(idx)` (FUN_10046690, to everyone): the objective slot goes away. On 3.0's wire
/// that is `ShowObj(idx, 0)`.
pub fn del_obj(idx: u8) {
    msg::all(Msg::ShowObj, |w| shared::protocol::rules::ShowObj { idx, visible: 0 }.write(w));
}

/// A 2.1 time scale (`pev->fuser1`: 0 is normal speed, anything else the factor) in 3.0's
/// encoding (1.0 is normal speed). 2.1's movement scales the frame by `f >= 0.1 ? f : 0` unless
/// `f` is 0 (client pmove FUN_1005f4f0), 3.0's always, so only 0 differs: under 3.0's rule it
/// would freeze the player.
pub fn time_scale(f21: f32) -> f32 {
    if f21 == 0.0 { 1.0 } else { f21 }
}

/// 2.1's super-jump glide bit (`PM_CheckParamters` 0x10058a60).
const GLIDE_21: i32 = 0x80000;
/// 2.1's super-jump start bit (`PM_Jump` 0x10058680).
const SUPERJUMP_START_21: i32 = 0x100000;

/// 2.1 `iuser4` stunt bits in 3.0's layout. They match below 0x80000; 2.1's glide (0x80000)
/// and super-jump start (0x100000) are 3.0's 0x100000 and 0x200000 (`shared::ts::stunt`).
pub fn stunt_bits(b21: i32) -> i32 {
    use shared::ts::stunt::{SUPERJUMP_GLIDE, SUPERJUMP_START};
    let mut b = b21 & !(GLIDE_21 | SUPERJUMP_START_21);
    if b21 & GLIDE_21 != 0 {
        b |= SUPERJUMP_GLIDE;
    }
    if b21 & SUPERJUMP_START_21 != 0 {
        b |= SUPERJUMP_START;
    }
    b
}

/// 3.0 `iuser4` stunt bits as 2.1 code reads them: the inverse of [`stunt_bits`]. 3.0's bits
/// 2.1 has no counterpart for (0x80000 frozen angles, 0x400000, the 0x800000 wall dive) are
/// dropped.
pub fn stunt_bits_21(b30: i32) -> i32 {
    use shared::ts::stunt::{FREEZE_ANGLES, NO_VIEW, SUPERJUMP_GLIDE, SUPERJUMP_START, WALLDIVE};
    let mut b = b30 & !(FREEZE_ANGLES | SUPERJUMP_GLIDE | SUPERJUMP_START | NO_VIEW | WALLDIVE);
    if b30 & SUPERJUMP_GLIDE != 0 {
        b |= GLIDE_21;
    }
    if b30 & SUPERJUMP_START != 0 {
        b |= SUPERJUMP_START_21;
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::protocol::MESSAGES;

    fn id(m: Msg) -> i32 {
        m as i32 + 64
    }

    fn writer(m: Msg) -> MsgWriter {
        MsgWriter::begin(2, id(m), None, std::ptr::null_mut())
    }

    #[test]
    fn the_wire_keeps_3_0s_table() {
        // 2.1-only messages aren't registered, so nothing can send them.
        for name in ["Flashlight", "FlashBat", "DelObj", "GloSlow", "KAlign"] {
            assert!(MESSAGES.iter().all(|(_, n, _)| n.to_bytes() != name.as_bytes()), "{name}");
        }
        // 3.0's sizes stay.
        let size = |m: Msg| MESSAGES.iter().find(|e| e.0 == m).unwrap().2;
        assert_eq!(size(Msg::WStatus), 3);
        assert_eq!(size(Msg::TsFrags), -1);
        assert_eq!(size(Msg::ScreenFade), 10);
        assert_eq!(size(Msg::TsFade), -1);
        assert_eq!(size(Msg::ShowObj), 2);
    }

    #[test]
    fn screen_fade_goes_out_as_ts_fade() {
        let w = writer(Msg::ScreenFade).short(4096).short(2048).short(1).byte(255).byte(0).byte(0).byte(128);
        let ops = w.ops().to_vec();
        let out = translate(Msg::ScreenFade, w, id);
        assert_eq!(out.msg_type(), id(Msg::TsFade));
        assert_eq!(out.ops(), &ops[..], "the same body");
    }

    #[test]
    fn short_breakable_debris_gets_3_0s_tail() {
        let mut w = writer(Msg::TsFrags);
        for c in [1.0, 2.0, 3.0, 4.0, 5.0, 6.0] {
            w = w.coord(c);
        }
        let out = translate(Msg::TsFrags, w.byte(0), id);
        assert_eq!(out.ops().len(), 7 + 8);
        assert_eq!(out.ops()[13], MsgOp::Long(2), "DMG_BULLET: 3.0's client scatters the shards as 2.1's does");
        assert_eq!(out.ops()[14], MsgOp::Coord(0.0), "no force");
        // 13 bytes, then 7 coords and a long.
        assert_eq!(out.ops().iter().map(MsgOp::size).sum::<usize>(), 13 + 18);
        // Types 1 and 2 are the same 13 bytes in both, and a 3.0 type 0 is left alone.
        let mut w = writer(Msg::TsFrags);
        for _ in 0..6 {
            w = w.coord(0.0);
        }
        assert_eq!(translate(Msg::TsFrags, w.clone().byte(1), id).ops().len(), 7);
        let mut long = w.byte(0);
        for _ in 0..6 {
            long = long.coord(1.0);
        }
        let long = long.long(64).coord(30.0);
        assert_eq!(translate(Msg::TsFrags, long, id).ops().len(), 15);
    }

    #[test]
    fn two_byte_wstatus_gets_a_count() {
        let out = translate(Msg::WStatus, writer(Msg::WStatus).byte(12).byte(1), id);
        assert_eq!(out.ops(), &[MsgOp::Byte(12), MsgOp::Byte(1), MsgOp::Byte(1)]);
        // 3.0's three bytes are left alone.
        let out = translate(Msg::WStatus, writer(Msg::WStatus).byte(24).byte(1).byte(3), id);
        assert_eq!(out.ops(), &[MsgOp::Byte(24), MsgOp::Byte(1), MsgOp::Byte(3)]);
    }

    #[test]
    fn other_messages_pass() {
        let w = writer(Msg::KFuPower).char(55);
        let out = translate(Msg::KFuPower, w, id);
        assert_eq!((out.msg_type(), out.ops()), (id(Msg::KFuPower), &[MsgOp::Char(55)][..]));
    }

    #[test]
    fn normal_speed_is_1_0() {
        assert_eq!(time_scale(0.0), 1.0);
        assert_eq!(time_scale(0.5), 0.5);
        // Below 0.1 both freeze.
        assert_eq!(time_scale(0.05), 0.05);
    }

    #[test]
    fn stunt_bits_move_to_3_0s_layout() {
        assert_eq!(stunt_bits(0x80000), 0x100000, "glide");
        assert_eq!(stunt_bits(0x100000), 0x200000, "super jump start");
        assert_eq!(stunt_bits(0x10 | 0x4 | 0x20000), 0x10 | 0x4 | 0x20000);
        assert_eq!(stunt_bits(0x80000 | 0x1), 0x100001);
        for b in [0, 0x1, 0x80000, 0x100000, 0x80000 | 0x70c31, 0x3ffff] {
            assert_eq!(stunt_bits_21(stunt_bits(b)), b, "{b:#x}");
        }
        // 3.0-only bits have no 2.1 meaning.
        assert_eq!(stunt_bits_21(0x80000 | 0x800000 | 0x400000), 0);
    }
}
