//! Team Fortress messages TS still hooks although no TS server registers them: the viewport's
//! `ValClass`, `Feign`, `Detpack`, `BuildSt`, `RandomPC` and `AllowSpec` (hooked by
//! `CHud::Init` `0x10020170`, handled by `TeamFortressViewport`), and `CHudAmmoSecondary`'s
//! `SecAmmoVal`/`SecAmmoIcon` (element `+0x5c0` = `0x100cfee0`, whose `Draw` is TS's empty stub
//! `0x1001f8d0`). They only store values: the readers are the command menu's buttons
//! (`commandmenu.txt`, e.g. `0x1005a460` reads the valid classes and `RandomPC`), which the port
//! doesn't have.

use shared::protocol::hud::Reader;

/// `READ_BYTE` as the SDK's `parsemsg.cpp` gives it: -1 past the end.
fn read_byte(r: &mut Reader) -> i32 {
    let v = r.byte();
    if r.bad { -1 } else { v as i32 }
}

/// `READ_SHORT`, -1 past the end.
fn read_short(r: &mut Reader) -> i32 {
    let v = r.short();
    if r.bad { -1 } else { v as i32 }
}

/// The `TeamFortressViewport` fields the TFC messages set. The viewport is allocated without
/// clearing; `Initialize` (`0x10060aa0`, from every `HUD_VidInit`) zeroes the valid classes and
/// `m_iRandomPC`, and the port starts the others at 0 too.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ViewportTfc {
    /// `m_iValidClasses` (`+0x894`, 5 ints): `ValClass` (`0x10064da0`), five shorts.
    pub valid_classes: [i32; 5],
    /// `m_iIsFeigning` (`+0x8a8`): `Feign` (`0x10064ea0`).
    pub is_feigning: i32,
    /// `m_iIsSettingDetpack` (`+0x8ac`): `Detpack` (`0x10064ed0`).
    pub is_setting_detpack: i32,
    /// `m_iBuildState` (`+0x8b4`): `BuildSt` (`0x10064ff0`).
    pub build_state: i32,
    /// `m_iRandomPC` (`+0x8b8`): `RandomPC` (`0x10065020`).
    pub random_pc: i32,
    /// `m_iAllowSpectators` (`+0x890`): `AllowSpec` (`0x100653a0`); nothing in TS reads it.
    pub allow_spectators: i32,
}

impl ViewportTfc {
    /// The part of `TeamFortressViewport::Initialize` (`0x10060aa0`) that touches these fields.
    pub fn initialize(&mut self) {
        self.valid_classes = [0; 5];
        self.random_pc = 0;
    }

    /// Each handler reads its value(s) and, except `RandomPC`, calls `UpdateCommandMenu`
    /// (`0x10063840`), which the port has no command menu for; `AllowSpec` also has the team
    /// menu `Update` itself, which the port's immediate-mode team menu does every frame.
    pub fn apply(&mut self, msg: TfcMsg, buf: &[u8]) {
        let mut r = Reader::new(buf);
        match msg {
            TfcMsg::ValClass => self.valid_classes = [(); 5].map(|_| read_short(&mut r)),
            TfcMsg::Feign => self.is_feigning = read_byte(&mut r),
            TfcMsg::Detpack => self.is_setting_detpack = read_byte(&mut r),
            TfcMsg::BuildSt => self.build_state = read_byte(&mut r),
            TfcMsg::RandomPC => self.random_pc = read_byte(&mut r),
            TfcMsg::AllowSpec => self.allow_spectators = read_byte(&mut r),
        }
    }
}

/// The viewport's TFC messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TfcMsg {
    ValClass,
    Feign,
    Detpack,
    BuildSt,
    RandomPC,
    AllowSpec,
}

/// `MAX_SEC_AMMO_VALUES`.
pub const MAX_SEC_AMMO_VALUES: usize = 4;

/// `CHudAmmoSecondary` (`+0x5c0`).
#[derive(Debug, Clone, PartialEq)]
pub struct AmmoSecondary {
    /// `m_HUD_ammoicon` (`+0x14`): a hud.txt index, -1 when the name isn't there.
    pub icon: i32,
    /// `m_iAmmoAmounts` (`+0x18`): -1 = don't draw.
    pub amounts: [i32; MAX_SEC_AMMO_VALUES],
    /// `m_fFade` (`+0x28`).
    pub fade: f32,
    /// `HUD_ACTIVE`.
    pub active: bool,
}

impl Default for AmmoSecondary {
    /// `Init` (`0x1001a5e0`): no icon, every amount -1, then `Reset`.
    fn default() -> Self {
        AmmoSecondary { icon: 0, amounts: [-1; MAX_SEC_AMMO_VALUES], fade: 0.0, active: false }
    }
}

impl AmmoSecondary {
    /// `Reset` (`0x1001a640`).
    pub fn reset(&mut self) {
        self.fade = 0.0;
    }

    /// `MsgFunc_SecAmmoVal` (`0x1001a690`): byte index (0..3), byte amount. Turns the element on,
    /// or off again when every amount is 0 or less; otherwise the icons light up (fade 200).
    pub fn msg_val(&mut self, buf: &[u8]) {
        let mut r = Reader::new(buf);
        let index = read_byte(&mut r);
        if !(0..MAX_SEC_AMMO_VALUES as i32).contains(&index) {
            return;
        }
        self.amounts[index as usize] = read_byte(&mut r);
        self.active = true;
        if self.amounts.iter().map(|&a| a.max(0)).sum::<i32>() == 0 {
            self.active = false;
            return;
        }
        self.fade = 200.0;
    }

    /// `MsgFunc_SecAmmoIcon` (`0x1001a650`): `GetSpriteIndex(READ_STRING())`.
    pub fn msg_icon(&mut self, buf: &[u8], sprite_index: impl Fn(&str) -> Option<usize>) {
        let name = Reader::new(buf).string();
        self.icon = sprite_index(&name).map_or(-1, |i| i as i32);
    }
}

/// Hooks the messages as `CHud::Init` (`0x10020170`) and `CHudAmmoSecondary::Init` do. Before
/// the first `HUD_VidInit` TS has no viewport and its hooks do nothing; the engine calls
/// `HUD_VidInit` before any server message.
pub fn hook() {
    let e = goldsrc::client::engine();
    macro_rules! viewport {
        ($name:literal, $m:ident) => {
            e.hook_user_msg($name, |b| super::with(|h| h.tfc.apply(TfcMsg::$m, b)))
        };
    }
    viewport!(c"ValClass", ValClass);
    viewport!(c"Feign", Feign);
    viewport!(c"Detpack", Detpack);
    viewport!(c"BuildSt", BuildSt);
    viewport!(c"RandomPC", RandomPC);
    viewport!(c"AllowSpec", AllowSpec);
    e.hook_user_msg(c"SecAmmoVal", |b| super::with(|h| h.sec_ammo.msg_val(b)));
    e.hook_user_msg(c"SecAmmoIcon", |b| {
        let index = |n: &str| super::gfx_ref(|g| g.sprites.index(n));
        super::with(|h| h.sec_ammo.msg_icon(b, index));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewport_values() {
        let mut v = ViewportTfc::default();
        v.apply(TfcMsg::ValClass, &[1, 0, 2, 0, 0xff, 0xff, 4, 0]);
        assert_eq!(v.valid_classes, [1, 2, -1, 4, -1], "signed shorts, -1 past the end");
        v.apply(TfcMsg::Feign, &[1]);
        v.apply(TfcMsg::Detpack, &[2]);
        v.apply(TfcMsg::BuildSt, &[0xff]);
        v.apply(TfcMsg::RandomPC, &[1]);
        v.apply(TfcMsg::AllowSpec, &[]);
        assert_eq!((v.is_feigning, v.is_setting_detpack, v.build_state, v.random_pc, v.allow_spectators), (1, 2, 255, 1, -1));
        v.initialize();
        assert_eq!((v.valid_classes, v.random_pc, v.is_feigning), ([0; 5], 0, 1));
    }

    /// Every TFC handler takes any payload.
    #[test]
    fn hostile_payloads() {
        let (mut v, mut a) = (ViewportTfc::default(), AmmoSecondary::default());
        for b in [&[][..], &[0xff; 1], &[0xff; 64], &[3, 0xff], &[0, 0x80, 0, 0x80]] {
            for m in [TfcMsg::ValClass, TfcMsg::Feign, TfcMsg::Detpack, TfcMsg::BuildSt, TfcMsg::RandomPC, TfcMsg::AllowSpec] {
                v.apply(m, b);
            }
            a.msg_val(b);
            a.msg_icon(b, |_| Some(usize::MAX));
        }
        assert_eq!(a.icon, -1, "an index past i32 wraps to -1 as TS's int");
    }

    #[test]
    fn secondary_ammo() {
        let mut a = AmmoSecondary::default();
        assert_eq!(a.amounts, [-1; 4]);
        a.msg_val(&[4, 9]);
        assert!(!a.active, "index 4 is out of range");
        a.msg_val(&[2, 0]);
        assert!(!a.active, "all amounts 0 or less: off");
        assert_eq!(a.fade, 0.0);
        a.msg_val(&[1, 5]);
        assert!(a.active);
        assert_eq!(a.fade, 200.0);
        a.reset();
        assert_eq!(a.fade, 0.0);
        assert!(a.active, "Reset only clears the fade");
        a.msg_icon(b"d_skull\0", |n| (n == "d_skull").then_some(3));
        assert_eq!(a.icon, 3);
        a.msg_icon(b"nope\0", |_| None);
        assert_eq!(a.icon, -1);
    }
}
