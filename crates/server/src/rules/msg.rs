//! Sending `shared::protocol` payloads through the engine's `pfnWrite*` calls, plus the TS text
//! helpers (`TSHudMessageOne/All`, `ClientPrint`, `SayText`).

use crate::messages::{self, Msg};
use goldsrc::ext::rules::SVC_HLTV;
use goldsrc::server::{MsgWriter, engine};
use goldsrc::sys::*;
use goldsrc::util::to_cstring;
use shared::protocol::hud_wire::{DirectorEvent, Wire};
use shared::protocol::rules::{SayText, TextMsg, TsMessage};

/// `Wire` over an open `MsgWriter`.
pub struct Out(Option<MsgWriter>);

impl Out {
    fn w(&mut self, f: impl FnOnce(MsgWriter) -> MsgWriter) {
        let w = self.0.take().expect("message already ended");
        self.0 = Some(f(w));
    }
}

impl Wire for Out {
    fn byte(&mut self, v: i32) {
        self.w(|w| w.byte(v))
    }
    fn char(&mut self, v: i32) {
        self.w(|w| w.char(v))
    }
    fn short(&mut self, v: i32) {
        self.w(|w| w.short(v))
    }
    fn long(&mut self, v: i32) {
        self.w(|w| w.long(v))
    }
    fn coord(&mut self, v: f32) {
        self.w(|w| w.coord(v))
    }
    fn string(&mut self, s: &str) {
        let c = to_cstring(s);
        self.w(|w| w.string(&c))
    }
}

/// Writes a payload into `w` and ends the message.
pub fn send(w: MsgWriter, f: impl FnOnce(&mut Out)) {
    let mut o = Out(Some(w));
    f(&mut o);
    if let Some(w) = o.0 {
        w.end();
    }
}

pub fn one(m: Msg, to: *mut edict_t, f: impl FnOnce(&mut Out)) {
    if to.is_null() {
        return;
    }
    send(messages::one(m, to), f)
}

pub fn all(m: Msg, f: impl FnOnce(&mut Out)) {
    send(messages::all(m), f)
}

pub fn pvs(m: Msg, origin: [f32; 3], f: impl FnOnce(&mut Out)) {
    send(messages::pvs(m, origin), f)
}

/// HLTV director event to spectator proxies: `MESSAGE_BEGIN(MSG_SPEC, 0x32)` with TS's old
/// `DRC_EVENT` layout. Normal clients never receive `MSG_SPEC` messages.
pub fn director_event(primary: i32, secondary: i32, flags: i32) {
    let m = DirectorEvent::event(primary, secondary, flags);
    send(MsgWriter::begin(MSG_SPEC as i32, SVC_HLTV, None, std::ptr::null_mut()), |w| m.write(w));
}

/// Truncates like TS's `char[128]` buffers.
fn clip(text: &str, max: usize) -> String {
    let mut s: String = text.chars().filter(|&c| c != '\0').collect();
    while s.len() > max {
        s.pop();
    }
    s
}

/// `TSHudMessageOne(r,g,b,effect,hold,text,pev)`: coloured HUD text to one player.
pub fn ts_message_one(to: *mut edict_t, rgb: [u8; 3], effect: u8, hold: u8, text: &str) {
    if to.is_null() || text.is_empty() {
        return;
    }
    let m = TsMessage { r: rgb[0], g: rgb[1], b: rgb[2], effect, hold, text: clip(text, 127) };
    one(Msg::TsMessage, to, |w| m.write(w));
}

/// `TSHudMessageAll`.
pub fn ts_message_all(rgb: [u8; 3], effect: u8, hold: u8, text: &str) {
    if text.is_empty() {
        return;
    }
    let m = TsMessage { r: rgb[0], g: rgb[1], b: rgb[2], effect, hold, text: clip(text, 127) };
    all(Msg::TsMessage, |w| m.write(w));
}

/// SDK `ClientPrint(pev, dest, msg)` (`TextMsg` to one).
pub fn text_one(to: *mut edict_t, dest: u8, text: &str) {
    let m = TextMsg { dest, text: clip(text, 190), params: Vec::new() };
    one(Msg::TextMsg, to, |w| m.write(w));
}

/// SDK `UTIL_ClientPrintAll(dest, msg)`.
pub fn text_all(dest: u8, text: &str) {
    let m = TextMsg { dest, text: clip(text, 190), params: Vec::new() };
    all(Msg::TextMsg, |w| m.write(w));
}

pub fn say_one(to: *mut edict_t, sender: i32, text: &str) {
    let m = SayText { sender: sender as u8, text: text.to_owned() };
    one(Msg::SayText, to, |w| m.write(w));
}

pub fn say_all(sender: i32, text: &str) {
    let m = SayText { sender: sender as u8, text: text.to_owned() };
    all(Msg::SayText, |w| m.write(w));
}

/// `UTIL_LogPrintf` (the engine prefixes the timestamp).
pub fn log(text: &str) {
    engine().log(text);
}
