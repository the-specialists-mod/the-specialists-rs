//! TS 2.1's movement and gun rules, which the server and the client's prediction both run
//! (issue #100 §2): movement and the move speed in [`pm`]. The gun keeps TS 3.0's bodies so far.

pub mod pm;

use crate::Ts21;
use ::shared::gameplay::GunRules;

impl GunRules for Ts21 {}
