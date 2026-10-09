//! User messages, in TS 3.0 registration order with TS sizes (-1 = variable), so the original
//! TS client and this port interoperate. Source: decomp docs/server/architecture.md §3.
//! Payload layouts are documented in architecture.md §3.2 and client/architecture-hud.md §3;
//! per-area encoders/decoders live in submodules.

pub mod effects;
pub mod hud;
pub mod rules;
pub mod stunt;
pub mod weapons;

use std::ffi::CStr;

macro_rules! messages {
    ($($v:ident = $name:literal, $size:expr;)*) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[repr(u8)]
        pub enum Msg { $($v),* }

        /// (message, wire name, size) in registration order.
        pub const MESSAGES: &[(Msg, &CStr, i32)] = &[$((Msg::$v, $name, $size)),*];
    };
}

messages! {
    SelAmmo = c"SelAmmo", 4;
    CurWeapon = c"CurWeapon", 3;
    Geiger = c"Geiger", 1;
    Health = c"Health", 1;
    Damage = c"Damage", 12;
    Battery = c"Battery", 2;
    Train = c"Train", 1;
    HudText = c"HudText", -1;
    SayText = c"SayText", -1;
    TextMsg = c"TextMsg", -1;
    WeaponList = c"WeaponList", -1;
    ResetHUD = c"ResetHUD", 1;
    InitHUD = c"InitHUD", 0;
    GameTitle = c"GameTitle", 1;
    DeathMsg = c"DeathMsg", -1;
    ScoreInfo = c"ScoreInfo", 9;
    TeamInfo = c"TeamInfo", -1;
    TeamScore = c"TeamScore", -1;
    GameMode = c"GameMode", 1;
    Motd = c"MOTD", -1;
    ServerName = c"ServerName", -1;
    AmmoPickup = c"AmmoPickup", 2;
    WeapPickup = c"WeapPickup", 1;
    ItemPickup = c"ItemPickup", -1;
    HideWeapon = c"HideWeapon", 1;
    SetFov = c"SetFOV", 1;
    ShowMenu = c"ShowMenu", -1;
    ScreenShake = c"ScreenShake", 6;
    ScreenFade = c"ScreenFade", 10;
    TsFade = c"TSFade", -1;
    AmmoX = c"AmmoX", 2;
    TeamNames = c"TeamNames", -1;
    VguiMenu = c"VGUIMenu", 1;
    WeaponInfo = c"WeaponInfo", 6;
    ClipInfo = c"ClipInfo", 1;
    WStatus = c"WStatus", 3;
    ActItems = c"ActItems", 2;
    KFuPower = c"KFuPower", 1;
    TsHealth = c"TSHealth", 1;
    TsArmor = c"TSArmor", 1;
    TsState = c"TSState", 1;
    TsCash = c"TSCash", 2;
    TsSpace = c"TSSpace", 1;
    NextStunt = c"NextStunt", 1;
    PwUp = c"PwUp", 3;
    PwUpTime = c"PwUpTime", 1;
    RoundTime = c"RoundTime", 2;
    Objective = c"Objective", -1;
    ShowObj = c"ShowObj", 2;
    Wires = c"Wires", 3;
    SrvSett = c"SrvSett", 3;
    TsPAward = c"TSPAward", 2;
    TsBTime = c"TSBTime", 1;
    TsHVoice = c"TSHVoice", 1;
    TsFlag = c"TSFlag", 4;
    CustomWp = c"CustomWP", -1;
    Spectator = c"Spectator", 2;
    TsMBlur = c"TSMBlur", 2;
    TsMessage = c"TSMessage", -1;
    TsSlowMo = c"TSSlowMo", 2;
    PTakeDam = c"PTakeDam", 13;
    TsFrags = c"TSFrags", -1;
    PlInfo = c"PlInfo", 3;
    TsPSys = c"TSPSys", -1;
}

impl Msg {
    pub fn name(self) -> &'static CStr {
        MESSAGES[self as usize].1
    }
    pub fn size(self) -> i32 {
        MESSAGES[self as usize].2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_consistent() {
        for (i, (m, name, _)) in MESSAGES.iter().enumerate() {
            assert_eq!(*m as usize, i);
            assert!(name.to_bytes().len() <= 11, "{name:?} too long for the engine");
        }
        assert_eq!(Msg::TsSlowMo.name(), c"TSSlowMo");
    }
}
