//! TS 2.1's movement and gun rules, which the server and the client's prediction both run
//! (issue #100 §2): movement and the move speed in [`pm`]; the gun (§2.3) is [`gun`] with its
//! tables in [`tables`].

pub mod gun;
pub mod pm;
pub mod tables;
