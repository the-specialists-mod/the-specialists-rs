//! The Specialists server game logic (TS 3.0 port). The engine loads it through `ts-server-dll`,
//! installed as dlls/ts_i386.so, dlls/ts_amd64.so (FWGS 64-bit) and dlls/mp.dll.
//!
//! This crate is the game logic and contains no `unsafe` code. The engine entry points and
//! callbacks live in `ts-server-dll` (crates/server-dll), which converts the engine's raw
//! pointers and calls into the modules here. See docs/design.md §4 for the module map and hook
//! order, and AGENTS.md for the unsafe-code rules.

// Subsystems expose API for TS features that are dead in 3.0 (round and objective modes, cash)
// or kept for parity with the SDK; docs/implementation/integration.md lists what is unused.
#![allow(dead_code)]
#![deny(unsafe_code)]

pub mod bots;
pub mod commands;
pub mod damage;
pub mod entities;
pub mod messages;
pub mod net;
pub mod player;
pub mod rules;
pub mod stunt;
pub mod weapons;
pub mod world;

use goldsrc::server::engine;

/// Precache everything; called from worldspawn's spawn (the first entity of every map).
pub fn precache_all() {
    engine().precache_model(player::PLAYER_MODEL);
    // Engine water sounds: played when anything enters water, so precache them up front.
    for s in [c"player/pl_wade1.wav", c"player/pl_wade2.wav", c"player/pl_wade3.wav", c"player/pl_wade4.wav"] {
        engine().precache_sound(s);
    }
    stunt::precache();
    weapons::precache();
    rules::precache();
}
