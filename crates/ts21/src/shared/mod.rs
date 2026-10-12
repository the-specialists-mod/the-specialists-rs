//! TS 2.1's movement and gun rules, which the server and the client's prediction both run
//! (issue #100 §2). Nothing is overridden yet: both keep TS 3.0's bodies.

use crate::Ts21;
use ::shared::gameplay::{GunRules, PmRules};

impl PmRules for Ts21 {}

impl GunRules for Ts21 {}
