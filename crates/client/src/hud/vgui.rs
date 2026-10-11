//! TS's VGUI1 text: the scheme manager's fonts and the label text drawing its viewport panels
//! use, drawn with engine fills since the port has no VGUI1.
//!
//! `CSchemeManager` (`0x1005c300`, from the viewport constructor `0x100607f0` with the screen
//! width) reads `<res>_textscheme.txt` (the largest of 320…1600 not above the width that loads,
//! `0x1005caf0`), then makes one font per distinct (`FontName`, `FontSize`, `FontWeight`): a
//! scheme that repeats an earlier one's three values shares its font. With `bitmapfonts` on, each
//! new font is a VGUI bitmap font loaded from `gfx\vgui\fonts\<width>_<SchemeName>.tga`, where
//! `<width>` is the screen width itself (`this+0xc`). Those files exist for 640, 800, 1024, 1152,
//! 1280 and 1600; at other widths (or with `bitmapfonts 0`) TS's VGUI falls back to the system's
//! Arial at `FontSize`, which the port can't draw, so it uses the bitmap rendered from the same
//! scheme instead: `<scheme res>_<SchemeName>.tga`, the scheme file's Arial at the same size.
//!
//! The bitmap font format is VGUI1's (`FontPlat_Bitmap`; see FWGS's `3rdparty/freevgui`, which
//! reimplements it): a 32-bit RLE TGA, 256 cells in one row (`cell width = width / 256`), stored
//! bottom row first. A pixel is ink when any channel is non-zero. A character's advance is its
//! rightmost ink column + 1, except the space (`cell width / 4 + 1`).
//!
//! Text is drawn as VGUI1's `TextImage::paint` does (`\r` skipped, `\n` starts a line, a space
//! advances only when more text follows on the line), opaque in the scheme's foreground colour.

use goldsrc::client::engine;

/// The resolutions `LoadFileByResolution` tries (`0x1009650c`, 9 entries).
const RESOLUTIONS: [i32; 9] = [320, 400, 512, 640, 800, 1024, 1152, 1280, 1600];

/// One `SchemeName` block of the text scheme (`0x7c`-byte records).
#[derive(Debug, Clone, PartialEq)]
pub struct Scheme {
    pub name: String,
    pub font_name: String,
    pub font_size: i32,
    pub font_weight: i32,
    /// RGBA as written in the file (the manager stores VGUI's inverted alpha, `255 - a`).
    pub fg: [u8; 4],
    pub bg: [u8; 4],
    /// Index into [`Schemes::fonts`].
    pub font: usize,
}

/// A VGUI1 bitmap font.
#[derive(Debug, Clone)]
pub struct BitmapFont {
    pub cell_wide: usize,
    pub cell_tall: usize,
    /// `b` of `getCharABCwide` (`a` = `c` = 0).
    pub widths: [i32; 256],
    /// Ink bits, `cell_wide × 256` per row, top row first.
    bits: Vec<bool>,
}

impl BitmapFont {
    /// `FontPlat_Bitmap::loadFrom32BitTGA`: `None` unless an RLE (type 10) 32-bit TGA.
    pub fn from_tga(data: &[u8]) -> Option<BitmapFont> {
        if data.len() < 18 || data[2] != 10 || data[16] != 32 {
            return None;
        }
        let id_len = data[0] as usize;
        let w = u16::from_le_bytes([data[12], data[13]]) as usize;
        let h = u16::from_le_bytes([data[14], data[15]]) as usize;
        if w < 256 || h == 0 {
            return None;
        }
        let mut px = vec![[0u8; 4]; w * h];
        let mut p = 18 + id_len;
        // Rows are read bottom-up, whatever the descriptor says.
        for y in (0..h).rev() {
            let mut x = 0;
            while x < w {
                let head = *data.get(p)?;
                p += 1;
                let run = (head & 0x7f) as usize + 1;
                if x + run > w {
                    return None;
                }
                if head & 0x80 != 0 {
                    let c: [u8; 4] = data.get(p..p + 4)?.try_into().ok()?;
                    p += 4;
                    px[y * w + x..y * w + x + run].fill(c);
                } else {
                    for i in 0..run {
                        px[y * w + x + i] = data.get(p..p + 4)?.try_into().ok()?;
                        p += 4;
                    }
                }
                x += run;
            }
        }
        let cw = w / 256;
        let mut f = BitmapFont { cell_wide: cw, cell_tall: h, widths: [0; 256], bits: vec![false; cw * 256 * h] };
        for ch in 0..256 {
            let mut right = 0;
            for y in 0..h {
                for x in 0..cw {
                    let ink = px[y * w + ch * cw + x] != [0; 4];
                    f.bits[y * cw * 256 + ch * cw + x] = ink;
                    if ink && x > right {
                        right = x;
                    }
                }
            }
            f.widths[ch] = if ch == b' ' as usize { (cw / 4) as i32 + 1 } else { right as i32 + 1 };
        }
        Some(f)
    }

    pub fn tall(&self) -> i32 {
        self.cell_tall as i32
    }

    /// `Font::getTextSize`: the widest line (a `\n` counts its own width too) and the line count
    /// times the height.
    pub fn text_size(&self, text: &[u8]) -> (i32, i32) {
        let (mut wide, mut tall, mut xx) = (0, self.tall(), 0);
        for &ch in text {
            if ch == 0 {
                break;
            }
            if ch == b'\n' {
                tall += self.tall();
                xx = 0;
            }
            xx += self.widths[ch as usize];
            wide = wide.max(xx);
        }
        (wide, tall)
    }

    /// Horizontal ink runs of character `ch`, relative to its cell: (x, y, length).
    fn runs(&self, ch: u8, mut out: impl FnMut(i32, i32, i32)) {
        let row = self.cell_wide * 256;
        for y in 0..self.cell_tall {
            let base = y * row + ch as usize * self.cell_wide;
            let mut x = 0;
            while x < self.cell_wide {
                if self.bits[base + x] {
                    let start = x;
                    while x < self.cell_wide && self.bits[base + x] {
                        x += 1;
                    }
                    out(start as i32, y as i32, (x - start) as i32);
                } else {
                    x += 1;
                }
            }
        }
    }

    /// `TextImage::paint` with the image as wide as `wrap` (VGUI sizes a label's text image to
    /// the text, so it never wraps unless the text is wider than itself): calls `fill(x, y, w)` for
    /// each 1-pixel-high ink run, relative to the image's top-left.
    pub fn paint(&self, text: &[u8], wrap: i32, mut fill: impl FnMut(i32, i32, i32)) {
        let end = text.iter().position(|&c| c == 0).unwrap_or(text.len());
        let text = &text[..end];
        let (mut x, mut y) = (0, 0);
        let tall = self.tall();
        let mut i = 0;
        while i < text.len() {
            let ch = text[i];
            match ch {
                b'\r' => {}
                b'\n' => {
                    x = 0;
                    y += tall;
                }
                b' ' => {
                    if !matches!(text.get(i + 1), None | Some(b'\n' | b'\r')) {
                        x += self.widths[b' ' as usize];
                        if x > wrap {
                            x = 0;
                            y += tall;
                        }
                    }
                }
                _ => {
                    let word = text[i..].iter().position(|c| matches!(c, b'\n' | b'\r' | b' ')).unwrap_or(text.len() - i);
                    let len: i32 = text[i..i + word].iter().map(|&c| self.widths[c as usize]).sum();
                    if x + len > wrap {
                        x = 0;
                        y += tall;
                    }
                    for &c in &text[i..i + word] {
                        self.runs(c, |rx, ry, n| fill(x + rx, y + ry, n));
                        x += self.widths[c as usize];
                    }
                    i += word;
                    continue;
                }
            }
            i += 1;
        }
    }
}

/// Tokens as `COM_ParseFile` splits them: whitespace-separated, `"…"` quoted, `//` comments.
fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut it = text.chars().peekable();
    while let Some(&c) = it.peek() {
        if c.is_whitespace() {
            it.next();
        } else if c == '/' && text_has_comment(&mut it) {
            while it.next().is_some_and(|c| c != '\n') {}
        } else if c == '"' {
            it.next();
            out.push(it.by_ref().take_while(|&c| c != '"').collect());
        } else {
            let mut t = String::new();
            while let Some(&c) = it.peek() {
                if c.is_whitespace() {
                    break;
                }
                t.push(c);
                it.next();
            }
            out.push(t);
        }
    }
    out
}

fn text_has_comment(it: &mut std::iter::Peekable<std::str::Chars>) -> bool {
    let mut look = it.clone();
    look.next();
    look.peek() == Some(&'/')
}

/// `ParseRGBAFromString`: four integers.
fn rgba(s: &str) -> [u8; 4] {
    let mut c = [0u8; 4];
    for (o, v) in c.iter_mut().zip(s.split_whitespace()) {
        *o = v.parse::<i32>().unwrap_or(0) as u8;
    }
    c
}

/// The parsed text scheme and its fonts.
#[derive(Debug, Clone, Default)]
pub struct Schemes {
    /// The screen width it was built for.
    pub width: i32,
    /// The text scheme file's resolution (0 if none loaded).
    pub res: i32,
    pub schemes: Vec<Scheme>,
    pub fonts: Vec<Option<BitmapFont>>,
}

impl Schemes {
    /// `CSchemeManager::CSchemeManager(xres)` with `load` as `COM_LoadFile`.
    pub fn load(xres: i32, bitmap_fonts: bool, load: &dyn Fn(&str) -> Option<Vec<u8>>) -> Schemes {
        let mut s = Schemes { width: xres, ..Schemes::default() };
        let mut file = None;
        // `LoadFileByResolution`: from the largest resolution not above the width, downwards.
        for &r in RESOLUTIONS.iter().rev().filter(|&&r| r <= xres) {
            if let Some(f) = load(&format!("{r}_textscheme.txt")) {
                file = Some(f);
                s.res = r;
                break;
            }
        }
        if let Some(f) = file {
            s.schemes = parse(&String::from_utf8_lossy(&f));
        }
        if s.schemes.is_empty() {
            // "Default Scheme": Arial, size 0 (17 is only filled in for parsed schemes).
            s.schemes.push(Scheme {
                name: "Default Scheme".into(),
                font_name: "Arial".into(),
                font_size: 0,
                font_weight: 0,
                fg: [255; 4],
                bg: [0; 4],
                font: 0,
            });
        }
        for i in 0..s.schemes.len() {
            let same = (0..i).find(|&j| {
                let (a, b) = (&s.schemes[i], &s.schemes[j]);
                a.font_name.eq_ignore_ascii_case(&b.font_name) && a.font_size == b.font_size && a.font_weight == b.font_weight
            });
            if let Some(j) = same {
                s.schemes[i].font = s.schemes[j].font;
                continue;
            }
            let name = s.schemes[i].name.clone();
            let mut font = None;
            if bitmap_fonts {
                font = load(&format!("gfx/vgui/fonts/{xres}_{name}.tga")).and_then(|d| BitmapFont::from_tga(&d));
            }
            if font.is_none() && s.res > 0 {
                // The system-font fallback: the bitmap rendered from this scheme file's font.
                let res = s.res.max(640);
                font = load(&format!("gfx/vgui/fonts/{res}_{name}.tga")).and_then(|d| BitmapFont::from_tga(&d));
            }
            s.schemes[i].font = s.fonts.len();
            s.fonts.push(font);
        }
        s
    }

    /// `getSchemeHandle` (`0x1005cc60`): the scheme by name, else the first.
    pub fn get(&self, name: &str) -> &Scheme {
        self.schemes.iter().find(|s| s.name.eq_ignore_ascii_case(name)).unwrap_or(&self.schemes[0])
    }

    pub fn font(&self, scheme: &Scheme) -> Option<&BitmapFont> {
        self.fonts.get(scheme.font).and_then(Option::as_ref)
    }
}

/// The scheme blocks (`0x1005c300`'s parse loop): `SchemeName` starts one; a block with no
/// `FontSize` gets 17 and one with no `FontName` gets Arial when the next block starts.
fn parse(text: &str) -> Vec<Scheme> {
    let t = tokens(text);
    let mut out: Vec<Scheme> = Vec::new();
    let finish = |s: &mut Scheme| {
        if s.font_size == 0 {
            s.font_size = 17;
        }
        if s.font_name.is_empty() {
            s.font_name = "Arial".into();
        }
    };
    let mut i = 0;
    while i + 2 < t.len() {
        let (key, eq, val) = (&t[i], &t[i + 1], &t[i + 2]);
        if eq != "=" {
            break;
        }
        if key.eq_ignore_ascii_case("SchemeName") {
            if let Some(last) = out.last_mut() {
                finish(last);
            }
            out.push(Scheme {
                name: val.chars().take(31).collect(),
                font_name: String::new(),
                font_size: 0,
                font_weight: 0,
                fg: [255; 4],
                bg: [0; 4],
                font: 0,
            });
        } else if let Some(s) = out.last_mut() {
            match key.to_ascii_lowercase().as_str() {
                "fontname" => s.font_name = val.chars().take(47).collect(),
                "fontsize" => s.font_size = val.trim().parse().unwrap_or(0),
                "fontweight" => s.font_weight = val.trim().parse().unwrap_or(0),
                "fgcolor" => s.fg = rgba(val),
                "bgcolor" => s.bg = rgba(val),
                _ => {}
            }
        } else {
            break;
        }
        i += 3;
    }
    out
}

// ---- engine side ----------------------------------------------------------------------------

use goldsrc::Global;
use std::cell::RefCell;

static SCHEMES: Global<RefCell<Option<Schemes>>> = Global(RefCell::new(None));

/// Builds the scheme manager for the screen width (TS: the viewport's, which `HUD_VidInit`'s
/// `VGui_Startup` recreates when the resolution changes).
pub fn vid_init() {
    let e = engine();
    let w = e.screen().width;
    if SCHEMES.0.borrow().as_ref().is_some_and(|s| s.width == w) {
        return;
    }
    let bitmap = e.cvar_float(c"bitmapfonts") != 0.0;
    let s = Schemes::load(w, bitmap, &goldsrc::ext::render::load_file);
    *SCHEMES.0.borrow_mut() = Some(s);
}

/// Draws `text` in scheme `scheme`'s font at (`x`, `y`) in `rgb` (opaque), as a VGUI1 label's
/// text image paints it, clipped to the label's rectangle `clip` (x, y, w, h) as VGUI clips a
/// panel's painting. Returns false if the scheme has no font.
pub fn draw_text_clipped(scheme: &str, x: i32, y: i32, rgb: [u8; 3], text: &[u8], clip: (i32, i32, i32, i32)) -> bool {
    let s = SCHEMES.0.borrow();
    let Some(font) = s.as_ref().and_then(|s| s.font(s.get(scheme))) else { return false };
    let (w, _) = font.text_size(text);
    let e = engine();
    let c = [rgb[0] as i32, rgb[1] as i32, rgb[2] as i32];
    let (cx0, cy0, cx1, cy1) = (clip.0, clip.1, clip.0 + clip.2, clip.1 + clip.3);
    font.paint(text, w, |rx, ry, n| {
        let (py, x0, x1) = (y + ry, (x + rx).max(cx0), (x + rx + n).min(cx1));
        if py >= cy0 && py < cy1 && x0 < x1 {
            e.fill_blend(x0, py, x1 - x0, 1, c, 255);
        }
    });
    true
}

/// The size `Font::getTextSize` gives `text` in scheme `scheme`'s font.
pub fn text_size(scheme: &str, text: &[u8]) -> Option<(i32, i32)> {
    let s = SCHEMES.0.borrow();
    s.as_ref().and_then(|s| s.font(s.get(scheme))).map(|f| f.text_size(text))
}

/// The scheme's foreground colour (RGB).
pub fn fg(scheme: &str) -> [u8; 3] {
    let s = SCHEMES.0.borrow();
    s.as_ref().map_or([255, 170, 0], |s| {
        let c = s.get(scheme).fg;
        [c[0], c[1], c[2]]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tga(w: u16, h: u16, rows_bottom_up: &[&[[u8; 4]]]) -> Vec<u8> {
        let mut d = vec![0, 0, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        d.extend_from_slice(&w.to_le_bytes());
        d.extend_from_slice(&h.to_le_bytes());
        d.extend_from_slice(&[32, 0x20]);
        for row in rows_bottom_up {
            for px in row.chunks(128) {
                d.push(px.len() as u8 - 1);
                for p in px {
                    d.extend_from_slice(p);
                }
            }
        }
        d
    }

    #[test]
    fn bitmap_font_metrics() {
        const INK: [u8; 4] = [255, 255, 255, 0];
        // 256 cells of 4×2. 'A' (65) has ink at x=2 in the top row; '.' at x=0 in the bottom row.
        let mut top = vec![[0u8; 4]; 1024];
        let mut bottom = vec![[0u8; 4]; 1024];
        top[65 * 4 + 2] = INK;
        bottom[b'.' as usize * 4] = INK;
        let f = BitmapFont::from_tga(&tga(1024, 2, &[&bottom, &top])).unwrap();
        assert_eq!((f.cell_wide, f.tall()), (4, 2));
        assert_eq!((f.widths[65], f.widths[b'.' as usize], f.widths[b' ' as usize], f.widths[b'x' as usize]), (3, 1, 2, 1));
        assert_eq!(f.text_size(b"A A"), (8, 2));
        assert_eq!(f.text_size(b"A\nA"), (4, 4), "a newline counts its own width");
        let mut fills = Vec::new();
        f.paint(b"A.", 100, |x, y, n| fills.push((x, y, n)));
        assert_eq!(fills, vec![(2, 0, 1), (3, 1, 1)]);
        fills.clear();
        f.paint(b"A ", 100, |x, y, n| fills.push((x, y, n)));
        f.paint(b" .", 100, |x, y, n| fills.push((x, y, n)));
        assert_eq!(fills, vec![(2, 0, 1), (2, 1, 1)], "a trailing space doesn't advance; a leading one does");
    }

    /// The shipped Half-Life bitmap fonts decode (skipped when the game files aren't installed).
    #[test]
    fn decodes_the_shipped_fonts() {
        let dir = std::env::var("HOME").unwrap_or_default() + "/games/xash_win_orig/valve/gfx/vgui/fonts/";
        let Ok(data) = std::fs::read(dir.clone() + "800_primary button text.tga") else { return };
        let f = BitmapFont::from_tga(&data).unwrap();
        assert_eq!((f.cell_wide, f.tall()), (19, 13));
        assert!(f.widths[b'W' as usize] > f.widths[b'i' as usize] && f.widths[b'i' as usize] > 1);
        assert_eq!(f.widths[b' ' as usize], 5);
        let mut ink = 0;
        f.paint(b"Free-Look", 1000, |_, _, n| ink += n);
        assert!(ink > 100);
        // Rows are stored bottom-up: descenders land in the last rows.
        let mut rows = [0; 13];
        f.paint(b"gjy", 1000, |_, y, n| rows[y as usize] += n);
        assert!(rows[12] > 0 && rows[0] == 0);
    }

    #[test]
    fn scheme_parse_and_font_sharing() {
        let file = r#"// test
SchemeName = "Primary Button Text"
FontName = "Arial"
FontSize = 20
FgColor = "255 170 0 255"
SchemeName = "Scoreboard Text"
FontName = "Arial"
FontSize = 15
FontWeight = 0
SchemeName = "Briefing Text"
FontName = "Arial"
FontSize = 15
FgColor = "1 2 3 4"
SchemeName = "No Size"
"#;
        let loaded = std::cell::RefCell::new(Vec::new());
        let load = |p: &str| {
            loaded.borrow_mut().push(p.to_owned());
            (p == "800_textscheme.txt").then(|| file.as_bytes().to_vec())
        };
        let s = Schemes::load(1000, true, &load);
        assert_eq!(s.res, 800);
        assert_eq!(loaded.borrow()[0], "800_textscheme.txt", "largest resolution not above the width");
        assert_eq!(s.schemes.len(), 4);
        assert_eq!(s.get("briefing text").fg, [1, 2, 3, 4]);
        assert_eq!(s.get("briefing text").font, s.get("Scoreboard Text").font, "same name, size and weight share a font");
        assert_ne!(s.get("Primary Button Text").font, s.get("Scoreboard Text").font);
        assert_eq!(s.get("missing").name, "Primary Button Text");
        assert!(loaded.borrow().contains(&"gfx/vgui/fonts/1000_Primary Button Text.tga".to_owned()));
        assert!(loaded.borrow().contains(&"gfx/vgui/fonts/800_Primary Button Text.tga".to_owned()), "system-font fallback");
        assert!(!loaded.borrow().iter().any(|p| p.contains("Briefing")), "a shared font isn't loaded again");
    }
}
