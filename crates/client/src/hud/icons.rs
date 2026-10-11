//! `CHudStatusIcons` (SDK statusicons.cpp, element `+0x600` = `0x100cff20`): up to four hud.txt
//! sprites stacked up the left edge from mid-screen. TS keeps it for the `StatusIcon` message
//! and the TFC `Concuss` message (`dmg_concuss`), neither of which a TS server sends.

use super::draw::HudSprites;
use goldsrc::client::engine;
use goldsrc::sys::{HSPRITE, wrect_t};

/// `MAX_ICONSPRITES`.
pub const MAX_ICONS: usize = 4;
/// `MAX_ICONSPRITENAME_LENGTH`: the name buffer (`+0x14`, 24 bytes).
const NAME_LEN: usize = 24;

/// One `icon_sprite_t` (`0x30` bytes from `+0x14`).
#[derive(Debug, Clone, Copy, Default)]
pub struct Icon {
    pub name: [u8; NAME_LEN],
    pub spr: HSPRITE,
    pub rc: wrect_t,
    pub rgb: [u8; 3],
}

impl Icon {
    fn name(&self) -> &[u8] {
        let n = self.name.iter().position(|&b| b == 0).unwrap_or(NAME_LEN);
        &self.name[..n]
    }
}

#[derive(Debug, Clone, Default)]
pub struct StatusIcons {
    pub icons: [Icon; MAX_ICONS],
    /// `HUD_ACTIVE`: set only by the `StatusIcon` message's enable, cleared by `Reset`.
    pub active: bool,
}

impl StatusIcons {
    /// `Reset` (`0x1002f110`): clears the icons and `HUD_ACTIVE`.
    pub fn reset(&mut self) {
        *self = StatusIcons::default();
    }

    /// `EnableIcon` (`0x1002f210`): reuse the slot with this name (`stricmp`), else the first
    /// free one, else slot 0. The sprite comes from hud.txt by name (`GetSpriteIndex`); an
    /// unknown name leaves the handle 0. Returns true for a `grenade` icon, which plays
    /// `weapons/timer.wav`.
    pub fn enable(&mut self, name: &[u8], rgb: [u8; 3], sprites: &HudSprites) -> bool {
        let i = self
            .icons
            .iter()
            .position(|ic| ic.name().eq_ignore_ascii_case(name))
            .or_else(|| self.icons.iter().position(|ic| ic.spr == 0))
            .unwrap_or(0);
        let ic = &mut self.icons[i];
        let idx = sprites.index(&String::from_utf8_lossy(name));
        ic.spr = idx.map_or(0, |k| sprites.handles[k]);
        // TS copies the rectangle at index -1 for an unknown name (memory before the array);
        // the port leaves it zero.
        ic.rc = idx.map_or(wrect_t::default(), |k| sprites.rects[k]);
        ic.rgb = rgb;
        // `strcpy` into the 24-byte buffer; a longer name runs over the slot in TS. The port
        // keeps the first 23 bytes.
        ic.name = [0; NAME_LEN];
        let n = name.len().min(NAME_LEN - 1);
        ic.name[..n].copy_from_slice(&name[..n]);
        name.windows(7).any(|w| w == b"grenade")
    }

    /// `DisableIcon` (`0x1002f330`): clears the slot with this name.
    pub fn disable(&mut self, name: &[u8]) {
        if let Some(ic) = self.icons.iter_mut().find(|ic| ic.name().eq_ignore_ascii_case(name)) {
            *ic = Icon::default();
        }
    }

    /// `Draw` (`0x1002f130`): from `ScreenHeight / 2` up, each icon `height + 5` above the last,
    /// at x 5, additive.
    pub fn draw(&self, screen_height: i32) {
        let e = engine();
        let mut y = screen_height / 2;
        for ic in &self.icons {
            if ic.spr == 0 {
                continue;
            }
            y += (ic.rc.top - ic.rc.bottom) - 5;
            e.spr_set(ic.spr, [ic.rgb[0] as i32, ic.rgb[1] as i32, ic.rgb[2] as i32]);
            e.spr_draw_additive(0, 5, y, Some(&ic.rc));
        }
    }
}
