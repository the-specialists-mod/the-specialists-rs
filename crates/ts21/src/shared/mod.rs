//! TS 2.1's movement and gun rules, which the server and the client's prediction both run
//! (issue #100 §2). The gun (§2.3) is [`gun`] with its tables in [`tables`]; movement keeps TS
//! 3.0's bodies so far.

pub mod gun;
pub mod tables;

use crate::Ts21;
use ::shared::gameplay::PmRules;

impl PmRules for Ts21 {}
