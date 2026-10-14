//! TS 2.1's server cvars where they differ from 3.0's (`pfnGameInit` 0x1001e750, decomp
//! `docs/ts21/rules.md` §2), and the three cvars 2.1 polls that are server commands in 3.0.

use goldsrc::Global;
use goldsrc::server::engine;
use std::cell::RefCell;
use ts_server::gameplay::server::CvarDef;
use ts_server::rules::cvars;

const SERVER: i32 = 0x4; // FCVAR_SERVER

/// The cvars 2.1 registers and 3.0 doesn't, with 2.1's defaults (`cvar_t` structs from
/// 0x10093a0c). 3.0 defines `initial_cash`, `cashforwin`, `cashforlose` and `respawnnumber`
/// without registering them; 2.1 reads them live.
///
/// `respawntime` is "5" in 2.1 and "3" in 3.0; both clamp it to 5..30 where it is used, so 3.0's
/// default plays the same. `superjump` is defined in 2.1 but never registered (always 0), and
/// `mp_teammodels`/`botarmor` are 3.0's: the union of both sets is registered, and 2.1 never reads
/// them.
pub const CVARS: &[CvarDef] = &[
    ("mp_flashlight", "0", SERVER),
    ("initial_cash", "3000", SERVER),
    ("cashforwin", "1500", SERVER),
    ("cashforlose", "-1000", SERVER),
    ("respawnnumber", "1", SERVER),
];

/// `initial_cash.value`.
pub fn initial_cash() -> f32 {
    cvars::f(c"initial_cash")
}

/// `(int)initial_cash.value` (`__ftol`, truncated).
pub fn initial_cash_int() -> i32 {
    trunc(initial_cash())
}

/// `(int)cashforwin.value`.
pub fn cash_for_win() -> i32 {
    trunc(cvars::f(c"cashforwin"))
}

/// `(int)cashforlose.value`.
pub fn cash_for_lose() -> i32 {
    trunc(cvars::f(c"cashforlose"))
}

/// `respawnnumber.value` as `SetPlayerRespawnTime` (0x10042d50) uses it: at least 1.
pub fn respawn_number() -> f32 {
    let n = cvars::f(c"respawnnumber");
    if n <= 1.0 { 1.0 } else { n }
}

/// MSVC's `__ftol`: truncation toward zero; NaN and out of range give the integer indefinite.
pub fn trunc(v: f32) -> i32 {
    ts_server::entities::stock::maprules::x87_int(v)
}

/// A cvar of 2.1's that is a server command in 3.0 (`globalslow`, `weaponrestriction`,
/// `restartround`): TS 2.1 registers them as cvars (FCVAR_SERVER, defaults "0.0", "0", "0") and
/// `CTSGameRules::Think` (0x10043cc0) polls them every frame. The engine won't register a cvar
/// that has the name of a command, so the 3.0 commands stay and, in 2.1 mode, behave like the
/// cvar: `name` prints `"name" is "value"`, `name value` sets it ([`server_command`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Polled {
    GlobalSlow,
    WeaponRestriction,
    RestartRound,
}

impl Polled {
    pub fn from_name(name: &str) -> Option<Polled> {
        match name {
            "globalslow" => Some(Polled::GlobalSlow),
            "weaponrestriction" => Some(Polled::WeaponRestriction),
            "restartround" => Some(Polled::RestartRound),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Polled::GlobalSlow => "globalslow",
            Polled::WeaponRestriction => "weaponrestriction",
            Polled::RestartRound => "restartround",
        }
    }

    fn default(self) -> &'static str {
        match self {
            Polled::GlobalSlow => "0.0",
            Polled::WeaponRestriction | Polled::RestartRound => "0",
        }
    }

    fn slot(self) -> usize {
        self as usize
    }
}

/// The polled cvars' strings. Cvars outlive maps, so these do too.
static POLLED: Global<RefCell<[Option<String>; 3]>> = Global(RefCell::new([None, None, None]));

/// The cvar's string.
pub fn polled_string(c: Polled) -> String {
    POLLED.0.borrow()[c.slot()].clone().unwrap_or_else(|| c.default().to_owned())
}

/// The cvar's value (`atof`, as the engine's `Cvar_Set` computes it).
pub fn polled(c: Polled) -> f32 {
    atof(&polled_string(c))
}

/// `CVarSetFloat`/`Cvar_Set` on the cvar.
pub fn set_polled(c: Polled, value: &str) {
    POLLED.0.borrow_mut()[c.slot()] = Some(value.to_owned());
}

/// The engine's `Cvar_Command` for one of the [`Polled`] cvars: no argument prints the value,
/// otherwise the first argument becomes the string.
pub fn server_command(name: &str) -> bool {
    let Some(c) = Polled::from_name(name) else { return false };
    let e = engine();
    if e.cmd_argc() < 2 {
        e.print(&format!("\"{}\" is \"{}\"\n", c.name(), polled_string(c)));
    } else {
        set_polled(c, &e.cmd_argv(1).to_string_lossy());
    }
    true
}

/// C's `atof`: the longest leading number (after white space), 0 when there is none.
pub fn atof(s: &str) -> f32 {
    let t = s.trim_start();
    let b = t.as_bytes();
    let mut end = 0;
    let mut seen_digit = false;
    let mut seen_dot = false;
    if end < b.len() && (b[end] == b'+' || b[end] == b'-') {
        end += 1;
    }
    while end < b.len() {
        match b[end] {
            b'0'..=b'9' => seen_digit = true,
            b'.' if !seen_dot => seen_dot = true,
            _ => break,
        }
        end += 1;
    }
    if !seen_digit {
        return 0.0;
    }
    // An exponent only when digits follow it.
    let mut stop = end;
    if stop < b.len() && (b[stop] == b'e' || b[stop] == b'E') {
        let mut k = stop + 1;
        if k < b.len() && (b[k] == b'+' || b[k] == b'-') {
            k += 1;
        }
        let digits = b[k..].iter().take_while(|c| c.is_ascii_digit()).count();
        if digits > 0 {
            stop = k + digits;
        }
    }
    t[..stop].parse::<f64>().map(|v| v as f32).unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atof_reads_the_leading_number() {
        assert_eq!(atof("0.5"), 0.5);
        assert_eq!(atof("  2x"), 2.0);
        assert_eq!(atof("-1000"), -1000.0);
        assert_eq!(atof("abc"), 0.0);
        assert_eq!(atof(""), 0.0);
        assert_eq!(atof("1e2"), 100.0);
        assert_eq!(atof("3e"), 3.0);
        assert_eq!(atof(".5"), 0.5);
    }

    #[test]
    fn cvar_defaults_are_2_1s() {
        let find = |n: &str| CVARS.iter().find(|c| c.0 == n).map(|c| c.1);
        assert_eq!(find("initial_cash"), Some("3000"));
        assert_eq!(find("cashforwin"), Some("1500"));
        assert_eq!(find("cashforlose"), Some("-1000"));
        assert_eq!(find("respawnnumber"), Some("1"));
        assert_eq!(find("mp_flashlight"), Some("0"));
        // None of them is one of 3.0's, so registering the union never meets a duplicate.
        assert!(CVARS.iter().all(|c| ts_server::rules::cvars::CVARS.iter().all(|t| t.0 != c.0)));
    }

    #[test]
    fn polled_cvars_start_at_their_defaults() {
        assert_eq!(Polled::from_name("globalslow"), Some(Polled::GlobalSlow));
        assert_eq!(Polled::from_name("loadweapons"), None);
        assert_eq!(Polled::GlobalSlow.default(), "0.0");
        assert_eq!(Polled::RestartRound.default(), "0");
    }
}
