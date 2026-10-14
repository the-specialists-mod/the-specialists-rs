//! Sprites and number drawing: `CHud::VidInit` (100206c0) sprite list from `sprites/hud.txt`,
//! the TS element sprites, and `DrawHudNumberTS` (1004c290).

use goldsrc::client::engine;
use goldsrc::sys::{HSPRITE, wrect_t};

/// TS red, used for warnings everywhere: (195, 45, 45).
pub const RED: [i32; 3] = [195, 45, 45];
/// TS greyed-out blue for "go back" rows: (100, 100, 205).
pub const BACK: [i32; 3] = [100, 100, 205];
/// TS green: (45, 195, 45).
pub const GREEN: [i32; 3] = [45, 195, 45];

pub fn half(c: [i32; 3]) -> [i32; 3] {
    [c[0] / 2, c[1] / 2, c[2] / 2]
}

/// hud.txt entries for the current resolution, with their sprite handles.
#[derive(Default)]
pub struct HudSprites {
    pub names: Vec<String>,
    pub handles: Vec<HSPRITE>,
    pub rects: Vec<wrect_t>,
    pub number_0: Option<usize>,
}

impl HudSprites {
    /// `CHud::VidInit`: `m_iRes` 320 below 640 px wide, else 640; load `sprites/<name>.spr`.
    pub fn load(width: i32) -> Self {
        let e = engine();
        let res = if width < 640 { 320 } else { 640 };
        let mut s = HudSprites::default();
        for entry in e.spr_get_list("sprites/hud.txt").into_iter().filter(|x| x.res == res) {
            s.handles.push(e.spr_load(&format!("sprites/{}.spr", entry.sprite)));
            s.names.push(entry.name);
            s.rects.push(entry.rc);
        }
        s.number_0 = s.index("number_0");
        s
    }

    /// `CHud::m_iFontHeight` (`0x100cf988`, set in `CHud::VidInit` at `0x100208a3`): the height
    /// of hud.txt's `number_0` rectangle (16 in TS's hud.txt), not the engine's character height.
    /// The train, the laser range and the voice talk icon are placed with it. Guarded: with no
    /// `number_0`, TS reads the rectangle before the table; the port answers 0.
    pub fn font_height(&self) -> i32 {
        self.number_0.and_then(|i| self.rects.get(i)).map_or(0, |r| r.bottom.wrapping_sub(r.top))
    }

    /// `CHud::GetSpriteIndex` (strncmp over 24 chars).
    pub fn index(&self, name: &str) -> Option<usize> {
        let n: String = name.chars().take(24).collect();
        self.names.iter().position(|x| x.chars().take(24).collect::<String>() == n)
    }
}

/// One HUD primitive of a layout a ruleset's drawing hook returns, which the HUD then draws (a
/// pure layout, so the hook can be tested).
#[derive(Clone, Debug, PartialEq)]
pub enum HudOp {
    /// `FillRGBA(x, y, w, h, r, g, b, a)`. Guarded: a width or height below 1 draws nothing.
    Fill { x: i32, y: i32, w: i32, h: i32, rgb: [i32; 3], a: i32 },
    /// `DrawHudString(x, y, ScreenWidth, text, r, g, b)`.
    Text { x: i32, y: i32, text: &'static str, rgb: [i32; 3] },
}

/// How a ruleset's status panel differs from TS 3.0's `DrawStatusPanel` (1004a3e0)
/// ([`ClientRules::status_look`](crate::gameplay::ClientRules::status_look)). The default is 3.0's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StatusLook {
    /// The stance sprite fills bottom-up by health, red above (3.0); false draws it whole in the
    /// HUD colour / 2.
    pub stance_health_fill: bool,
    /// The armour value as a number at (0x48, y + 0x18), the kevlar icon beside it at 0x68;
    /// false (3.0) draws the icon alone at 0x48.
    pub armor_number: bool,
    /// The team bar's box in the HUD colour, after which the HUD colour is `ScaleColors(.., a)`
    /// with this `a` (`(float)(a / 255.0f)`) for the name and the rest of the panel; `None` (3.0)
    /// draws the box and name in the team's colour.
    pub team_bar_scale: Option<i32>,
    /// Fixed "free slots" and "Credits" labels in place of `#HUD_FreeSlots` / `#HUD_Credits`.
    pub labels: Option<(&'static str, &'static str)>,
    /// The kung-fu and movement icons in the HUD colour / 1.5 (`ftol`); false (3.0) × the
    /// `double` 2/3, which truncates one lower for some channels.
    pub icons_divide: bool,
}

impl Default for StatusLook {
    fn default() -> Self {
        StatusLook { stance_health_fill: true, armor_number: false, team_bar_scale: None, labels: None, icons_divide: false }
    }
}

/// MSVC `__ftol` (`0x1006ad1c`): truncates toward zero; out of range (or NaN) gives the integer
/// indefinite value `0x80000000`. TS's x87 runs at 53-bit precision (the MSVC CRT default), so
/// callers compute the operand in `f64`.
pub fn ftol(v: f64) -> i32 {
    let t = v.trunc();
    if (-2147483648.0..=2147483647.0).contains(&t) { t as i32 } else { i32::MIN }
}

/// The engine drawing calls the SDK elements make (`CHudMessage`, `CHudStatusBar`,
/// `CHudDeathNotice`), behind a trait so tests can record them.
pub trait Canvas {
    /// `pfnDrawConsoleStringLen`: (width, height).
    fn console_size(&self, text: &[u8]) -> (i32, i32);
    /// `pfnDrawSetTextColor` (when `color` is set; otherwise the engine's current colour) and
    /// `pfnDrawConsoleString`: returns the x after the text.
    fn console_string(&mut self, x: i32, y: i32, color: Option<[f32; 3]>, text: &[u8]) -> i32;
    /// `pfnDrawCharacter` (`TextMessageDrawChar`).
    fn character(&mut self, x: i32, y: i32, ch: u8, rgb: [i32; 3]);
    /// `SPR_Set(gHUD.GetSprite(index), r, g, b)` and `SPR_DrawAdditive(0, x, y,
    /// &gHUD.GetSpriteRect(index))`.
    fn sprite(&mut self, index: usize, rgb: [i32; 3], x: i32, y: i32);
}

/// [`Canvas`] on the engine, with the hud.txt sprites.
pub struct EngineCanvas<'a>(pub &'a HudSprites);

impl Canvas for EngineCanvas<'_> {
    fn console_size(&self, text: &[u8]) -> (i32, i32) {
        engine().console_bytes_size(text)
    }
    fn console_string(&mut self, x: i32, y: i32, color: Option<[f32; 3]>, text: &[u8]) -> i32 {
        engine().draw_console_bytes(x, y, color, text)
    }
    fn character(&mut self, x: i32, y: i32, ch: u8, rgb: [i32; 3]) {
        engine().draw_character(x, y, ch, rgb);
    }
    fn sprite(&mut self, index: usize, rgb: [i32; 3], x: i32, y: i32) {
        let (Some(&h), Some(rc)) = (self.0.handles.get(index), self.0.rects.get(index)) else { return };
        let e = engine();
        e.spr_set(h, rgb);
        e.spr_draw_additive(0, x, y, Some(rc));
    }
}

/// Sprites the two TS elements load in `VidInit` (1004cb50, 10049d10).
#[derive(Default)]
pub struct TsSprites {
    pub stance: [HSPRITE; 3],
    pub health: HSPRITE,
    pub kevlar: HSPRITE,
    pub kungfu: HSPRITE,
    pub movement: HSPRITE,
    pub flag: HSPRITE,
    pub cursor: HSPRITE,
    pub compass: HSPRITE,
    pub uparrow: HSPRITE,
    pub downarrow: HSPRITE,
    /// Per weapon id: `sprites/weapons/*.spr` (index 0 = kung fu).
    pub weapons: Vec<HSPRITE>,
    pub item_installed: HSPRITE,
    pub item_not_installed: HSPRITE,
    pub cross_e: HSPRITE,
    pub cross2: HSPRITE,
    pub cross_t: HSPRITE,
    pub divestep: [HSPRITE; 3],
    pub wires: [HSPRITE; 3],
    pub wires_cut: [HSPRITE; 3],
}

/// HUD sprite per weapon id (`s_weapon_info+0x86`, from client strings.tsv).
pub fn weapon_sprite(id: usize) -> Option<&'static str> {
    const NAMES: [&str; 38] = [
        "kungfu",
        "glock18",
        "beretta",
        "uzi",
        "m3",
        "m4",
        "mp5sd",
        "mp5k",
        "berettas",
        "mk23",
        "mk23_akimbo",
        "usas12",
        "desert",
        "ak47",
        "fnh",
        "aug",
        "uzi_akimbo",
        "skorpion",
        "m82",
        "pdw",
        "spas12",
        "gold",
        "glock22",
        "ump",
        "m61",
        "knife",
        "mossberg",
        "m16",
        "ruger",
        "",
        "fnh_akimbo",
        "bull",
        "m60",
        "sawedoff",
        "katana",
        "sealknife",
        "contender",
        "skorpion_akimbo",
    ];
    NAMES.get(id).copied().filter(|s| !s.is_empty())
}

impl TsSprites {
    pub fn load() -> Self {
        let e = engine();
        let l = |n: &str| e.spr_load(n);
        TsSprites {
            stance: [l("sprites/player/stand.spr"), l("sprites/player/crouched.spr"), l("sprites/player/prone.spr")],
            health: l("sprites/player/health.spr"),
            kevlar: l("sprites/player/kevlar.spr"),
            kungfu: l("sprites/player/kungfu.spr"),
            movement: l("sprites/player/movement.spr"),
            flag: l("sprites/player/flag.spr"),
            cursor: l("sprites/cursor.spr"),
            compass: l("sprites/compass.spr"),
            uparrow: l("sprites/uparrow.spr"),
            downarrow: l("sprites/downarrow.spr"),
            weapons: (0..38).map(|i| weapon_sprite(i).map_or(0, |n| l(&format!("sprites/weapons/{n}.spr")))).collect(),
            item_installed: l("sprites/weapons/item_installed.spr"),
            item_not_installed: l("sprites/weapons/item_not_installed.spr"),
            cross_e: l("sprites/cross_e.spr"),
            cross2: l("sprites/cross2.spr"),
            cross_t: l("sprites/cross_t.spr"),
            divestep: [l("sprites/player/divestept.spr"), l("sprites/player/divestepc.spr"), l("sprites/player/divestepb.spr")],
            wires: [l("sprites/wires/rwire.spr"), l("sprites/wires/gwire.spr"), l("sprites/wires/bwire.spr")],
            wires_cut: [l("sprites/wires/rwirecutted.spr"), l("sprites/wires/gwirecutted.spr"), l("sprites/wires/bwirecutted.spr")],
        }
    }
}

/// `DrawHudNumberTS` flags.
pub mod num {
    pub const DRAW_ZERO: u8 = 0x01;
    pub const DIGITS_2: u8 = 0x02;
    pub const DIGITS_3: u8 = 0x04;
    pub const DIGITS_4: u8 = 0x08;
    pub const DIGITS_5: u8 = 0x10;
    pub const LEADING_ZEROS: u8 = 0x20;
}

/// Digit layout of `DrawHudNumberTS`: for each of the five positions (10000s .. 1s), `Some(d)` to
/// draw the sprite `number_0 + d`, `None` to leave a blank cell, or skipped entirely. Pure, for
/// tests. The top place is `n / 10000` without `% 10` (`0x1004c290`), so from 100000 on TS draws
/// the hud.txt entry that many places after `number_0` (whatever follows `number_9`); the
/// drawing skips an index past the table.
pub fn number_cells(n: i32, flags: u8) -> Vec<Option<u32>> {
    let mut cells = Vec::new();
    // (divisor, threshold, reserve mask)
    let places = [(10000, 10000, 0x10u8), (1000, 1000, 0x18), (100, 100, 0x0c), (10, 10, 0x06)];
    if n > 0 {
        for (div, thr, mask) in places {
            if n >= thr {
                let d = if div == 10000 { n / div } else { (n / div) % 10 };
                cells.push(Some(d as u32));
            } else if flags & mask != 0 {
                cells.push(if flags & num::LEADING_ZEROS != 0 { Some(0) } else { None });
            }
        }
        cells.push(Some((n % 10) as u32));
        return cells;
    }
    if flags & num::DRAW_ZERO == 0 {
        return cells;
    }
    for (_, _, mask) in places {
        if flags & mask != 0 {
            cells.push(if flags & num::LEADING_ZEROS != 0 { Some(0) } else { None });
        }
    }
    cells.push(Some(0));
    cells
}

/// `DrawHudNumberTS(x, y, flags, n, r, g, b)`: `number_0..9` sprites from hud.txt. Returns the x
/// after the number.
pub fn draw_number(s: &HudSprites, x: i32, y: i32, flags: u8, n: i32, rgb: [i32; 3]) -> i32 {
    let e = engine();
    let Some(n0) = s.number_0 else {
        // No sprites (bad content): fall back to text so numbers still show.
        let t = n.to_string();
        return e.draw_console_string(x, y, [rgb[0] as f32 / 255.0, rgb[1] as f32 / 255.0, rgb[2] as f32 / 255.0], &t);
    };
    // hud.txt rectangles are file data: C int arithmetic.
    let w = s.rects.get(n0).map_or(0, |r| r.right.wrapping_sub(r.left));
    let mut x = x;
    for cell in number_cells(n, flags) {
        if let Some(d) = cell {
            let i = n0 + d as usize;
            if let (Some(&h), Some(rc)) = (s.handles.get(i), s.rects.get(i)) {
                e.spr_set(h, rgb);
                e.spr_draw_additive(0, x, y, Some(rc));
            }
        }
        x = x.wrapping_add(w);
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_height_is_number_0s() {
        use goldsrc::sys::wrect_t;
        let mut s = HudSprites {
            names: vec!["d_skull".into(), "number_0".into()],
            handles: vec![],
            rects: vec![wrect_t { left: 0, right: 32, top: 0, bottom: 40 }, wrect_t { left: 0, right: 11, top: 4, bottom: 20 }],
            number_0: None,
        };
        assert_eq!(s.font_height(), 0, "guarded without number_0");
        s.number_0 = s.index("number_0");
        assert_eq!(s.font_height(), 16, "bottom - top of number_0, as CHud::VidInit sets m_iFontHeight");
    }

    #[test]
    fn number_layout() {
        assert_eq!(number_cells(7, num::DIGITS_3), [None, None, Some(7)]);
        assert_eq!(number_cells(45, num::DIGITS_2 | num::LEADING_ZEROS), [Some(4), Some(5)]);
        assert_eq!(number_cells(5, num::DIGITS_2 | num::LEADING_ZEROS | num::DRAW_ZERO), [Some(0), Some(5)]);
        assert_eq!(number_cells(0, num::DIGITS_3), Vec::<Option<u32>>::new());
        assert_eq!(number_cells(123456, 0), [Some(12), Some(3), Some(4), Some(5), Some(6)], "TS's top place has no % 10");
        assert_eq!(number_cells(0, num::DIGITS_2 | num::DRAW_ZERO), [None, Some(0)]);
        assert_eq!(number_cells(12345, 0), [Some(1), Some(2), Some(3), Some(4), Some(5)]);
        assert_eq!(number_cells(i32::MIN, 0xff), [Some(0); 5], "negatives draw as zero");
        assert_eq!(number_cells(i32::MAX, 0).len(), 5, "only the five low places");
        assert_eq!(ftol(f64::NAN), i32::MIN);
    }

    #[test]
    fn sprite_names_cover_table() {
        for w in shared::weapons::tables().weapons() {
            assert!(weapon_sprite(w.id as usize).is_some(), "weapon {} has a HUD sprite", w.id);
        }
        assert_eq!(weapon_sprite(29), None, "briefcase");
    }
}
