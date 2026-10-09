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

    /// `CHud::GetSpriteIndex` (strncmp over 24 chars).
    pub fn index(&self, name: &str) -> Option<usize> {
        let n: String = name.chars().take(24).collect();
        self.names.iter().position(|x| x.chars().take(24).collect::<String>() == n)
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
/// draw digit `d`, `None` to leave a blank cell, or skipped entirely. Pure, for tests.
pub fn number_cells(n: i32, flags: u8) -> Vec<Option<u8>> {
    let mut cells = Vec::new();
    // (divisor, threshold, reserve mask)
    let places = [(10000, 10000, 0x10u8), (1000, 1000, 0x18), (100, 100, 0x0c), (10, 10, 0x06)];
    if n > 0 {
        for (div, thr, mask) in places {
            if n >= thr {
                cells.push(Some(((n / div) % 10) as u8));
            } else if flags & mask != 0 {
                cells.push(if flags & num::LEADING_ZEROS != 0 { Some(0) } else { None });
            }
        }
        cells.push(Some((n % 10) as u8));
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
    let w = s.rects[n0].right - s.rects[n0].left;
    let mut x = x;
    for cell in number_cells(n, flags) {
        if let Some(d) = cell {
            let i = n0 + d as usize;
            if i < s.handles.len() {
                e.spr_set(s.handles[i], rgb);
                e.spr_draw_additive(0, x, y, Some(&s.rects[i]));
            }
        }
        x += w;
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_layout() {
        assert_eq!(number_cells(7, num::DIGITS_3), [None, None, Some(7)]);
        assert_eq!(number_cells(45, num::DIGITS_2 | num::LEADING_ZEROS), [Some(4), Some(5)]);
        assert_eq!(number_cells(5, num::DIGITS_2 | num::LEADING_ZEROS | num::DRAW_ZERO), [Some(0), Some(5)]);
        assert_eq!(number_cells(0, num::DIGITS_3), Vec::<Option<u8>>::new());
        assert_eq!(number_cells(0, num::DIGITS_2 | num::DRAW_ZERO), [None, Some(0)]);
        assert_eq!(number_cells(12345, 0), [Some(1), Some(2), Some(3), Some(4), Some(5)]);
    }

    #[test]
    fn sprite_names_cover_table() {
        for w in shared::weapons::tables().weapons() {
            assert!(weapon_sprite(w.id as usize).is_some(), "weapon {} has a HUD sprite", w.id);
        }
        assert_eq!(weapon_sprite(29), None, "briefcase");
    }
}
