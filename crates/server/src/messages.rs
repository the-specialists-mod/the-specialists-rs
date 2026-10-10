//! User message registration (TS order) and send helpers. Payload layouts per message are in the
//! decomp docs; each subsystem writes its own messages with these helpers.

use goldsrc::Global;
use goldsrc::server::{MsgWriter, engine};
use goldsrc::sys::*;
use shared::protocol::MESSAGES;
pub use shared::protocol::Msg;
use std::cell::Cell;

static IDS: Global<[Cell<i32>; 64]> = Global([const { Cell::new(0) }; 64]);

/// Registers every message in TS order. Called once from `GameInit`.
pub fn register_all() {
    for (m, name, size) in MESSAGES {
        IDS.0[*m as usize].set(engine().reg_user_msg(name, *size));
    }
}

pub fn id(m: Msg) -> i32 {
    IDS.0[m as usize].get()
}

/// Reliable message to one client.
pub fn one(m: Msg, to: *mut edict_t) -> MsgWriter {
    MsgWriter::begin(MSG_ONE as i32, id(m), None, to)
}

/// Unreliable message to one client (dropped if the channel is full).
pub fn one_unreliable(m: Msg, to: *mut edict_t) -> MsgWriter {
    MsgWriter::begin(MSG_ONE_UNRELIABLE as i32, id(m), None, to)
}

pub fn all(m: Msg) -> MsgWriter {
    MsgWriter::begin(MSG_ALL as i32, id(m), None, std::ptr::null_mut())
}

pub fn pvs(m: Msg, origin: [f32; 3]) -> MsgWriter {
    MsgWriter::begin(MSG_PVS as i32, id(m), Some(origin), std::ptr::null_mut())
}

pub fn pas(m: Msg, origin: [f32; 3]) -> MsgWriter {
    MsgWriter::begin(MSG_PAS as i32, id(m), Some(origin), std::ptr::null_mut())
}

const _: () = assert!(MESSAGES.len() <= 64);
