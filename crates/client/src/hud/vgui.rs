//! TS's VGUI1 layer: the scheme manager's fonts and colours, VGUI1's TGA images, and the panel
//! painting its viewport panels use (fills, borders, label text and images), drawn with engine
//! fills since the port has no VGUI1.
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
//! advances only when more text follows on the line, a word that doesn't fit the image's width
//! starts a new line), in the colour's opacity.
//!
//! Panels follow VGUI1's `Panel` rules: a child's position is relative to its parent's position
//! plus the parent's border inset, every panel is clipped to its own bounds and its parents'
//! clip rectangle (`Panel::solve`), a border paints at the panel's origin and the background and
//! contents at the origin plus the inset. VGUI1 colours carry an inverted alpha (0 = opaque, 255 =
//! transparent); the engine's surface (FWGS `VGUI_SetupDrawingRect`) draws with `255 - a` and skips
//! fully transparent fills.

use goldsrc::client::engine;
use std::collections::HashMap;
use std::rc::Rc;

/// The resolutions `LoadFileByResolution` tries (`0x1009650c`, 9 entries).
const RESOLUTIONS: [i32; 9] = [320, 400, 512, 640, 800, 1024, 1152, 1280, 1600];

/// A VGUI1 colour: RGB and VGUI's inverted alpha (0 opaque, 255 transparent).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Color {
        Color { r, g, b, a }
    }
    /// From the file's RGBA (A = opacity), as the scheme manager's fix-up stores it.
    pub const fn from_rgba(c: [u8; 4]) -> Color {
        Color { r: c[0], g: c[1], b: c[2], a: 255 - c[3] }
    }
    pub const fn rgb(self) -> [u8; 3] {
        [self.r, self.g, self.b]
    }
    /// The opacity the engine's surface draws with.
    pub const fn opacity(self) -> u8 {
        255 - self.a
    }
}

/// One `SchemeName` block of the text scheme (`0x7c`-byte records).
#[derive(Debug, Clone, PartialEq)]
pub struct Scheme {
    pub name: String,
    pub font_name: String,
    pub font_size: i32,
    pub font_weight: i32,
    /// `getFgColor` etc.: VGUI colours (the manager stores `255 - A`). The border colour is the
    /// file's RGBA, as the manager leaves it.
    pub fg: Color,
    pub bg: Color,
    pub armed_fg: Color,
    pub armed_bg: Color,
    pub mousedown_fg: Color,
    pub mousedown_bg: Color,
    pub border: [u8; 4],
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
    /// Ink of each character as rectangles (x, y, w, h) relative to its cell: the 1-pixel rows'
    /// runs merged downwards, so a glyph costs a few fills.
    rects: Vec<Vec<(i32, i32, i32, i32)>>,
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
        if w < 256 || h == 0 || !tga_fits(data.len(), id_len, 4, w * h) {
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
        let mut f = BitmapFont { cell_wide: cw, cell_tall: h, widths: [0; 256], rects: Vec::with_capacity(256) };
        for ch in 0..256 {
            let ink = |x: usize, y: usize| px[y * w + ch * cw + x] != [0; 4];
            let mut right = 0;
            let mut rows: Vec<Vec<(i32, i32)>> = Vec::with_capacity(h);
            for y in 0..h {
                let mut runs = Vec::new();
                let mut x = 0;
                while x < cw {
                    if ink(x, y) {
                        let start = x;
                        while x < cw && ink(x, y) {
                            x += 1;
                        }
                        right = right.max(x - 1);
                        runs.push((start as i32, (x - start) as i32));
                    } else {
                        x += 1;
                    }
                }
                rows.push(runs);
            }
            f.widths[ch] = if ch == b' ' as usize { (cw / 4) as i32 + 1 } else { right as i32 + 1 };
            f.rects.push(merge_runs(&mut rows));
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

    /// The sum of `getCharABCwide` over `text` (`CLabelHeader::setText`'s gap).
    pub fn chars_wide(&self, text: &[u8]) -> i32 {
        text.iter().take_while(|&&c| c != 0).map(|&c| self.widths[c as usize]).sum()
    }

    /// `TextImage::paint`'s layout with the image `wrap` wide: calls `glyph(x, y, ch)` for each
    /// character drawn, relative to the image's top-left.
    pub fn layout(&self, text: &[u8], wrap: i32, mut glyph: impl FnMut(i32, i32, u8)) {
        let end = text.iter().position(|&c| c == 0).unwrap_or(text.len());
        let text = &text[..end];
        let (mut x, mut y) = (0, 0);
        let tall = self.tall();
        let mut i = 0;
        while i < text.len() {
            match text[i] {
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
                        glyph(x, y, c);
                        x += self.widths[c as usize];
                    }
                    i += word;
                    continue;
                }
            }
            i += 1;
        }
    }

    /// `TextImage::paint` with the image `wrap` wide: calls `fill(x, y, w, h)` for each ink
    /// rectangle, relative to the image's top-left.
    pub fn paint(&self, text: &[u8], wrap: i32, mut fill: impl FnMut(i32, i32, i32, i32)) {
        self.layout(text, wrap, |x, y, c| {
            for &(rx, ry, w, h) in &self.rects[c as usize] {
                fill(x + rx, y + ry, w, h);
            }
        });
    }

    /// `TextImage::getTextSizeWrapped`: the extent of the characters laid out `wrap` wide.
    pub fn text_size_wrapped(&self, text: &[u8], wrap: i32) -> (i32, i32) {
        let (mut w, mut h) = (0, 0);
        self.layout(text, wrap, |x, y, c| {
            w = w.max(x + self.widths[c as usize]);
            h = h.max(y + self.tall());
        });
        (w, h)
    }
}

/// Merges each row's runs (x, length) with identical runs in the rows below into rectangles.
fn merge_runs(rows: &mut [Vec<(i32, i32)>]) -> Vec<(i32, i32, i32, i32)> {
    let mut out = Vec::new();
    for y in 0..rows.len() {
        while let Some((x, n)) = rows[y].pop() {
            let mut h = 1;
            while let Some(i) = rows.get(y + h).and_then(|r| r.iter().position(|&r| r == (x, n))) {
                rows[y + h].remove(i);
                h += 1;
            }
            out.push((x, y as i32, n, h as i32));
        }
    }
    out
}

/// A VGUI1 `BitmapTGA`: RGBA rows, top first, with the alpha as uploaded to the surface.
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    pub wide: i32,
    pub tall: i32,
    px: Vec<[u8; 4]>,
}

impl Image {
    /// `BitmapTGA::loadTGA` (VGUI1; Valve's `vgui.so` at `0x23880`, and freevgui's reimplementation
    /// documents the same quirks): types 2 and 10, 24 or 32 bits, rows stored bottom-up. The alpha
    /// handling differs by type: uncompressed 32-bit pixels are inverted when `invert_alpha` is
    /// *off* and 24-bit ones are opaque; RLE 32-bit pixels are inverted when it is on and 24-bit
    /// ones are 0 (on) or 255 (off). `vgui_LoadTGA` (`0x1005b8d0`) passes true and
    /// `vgui_LoadTGANoInvertAlpha` (`0x1005ba50`) false.
    pub fn from_tga(data: &[u8], invert_alpha: bool) -> Option<Image> {
        if data.len() < 18 || !matches!(data[2], 2 | 10) || data[1] != 0 || !matches!(data[16], 24 | 32) {
            return None;
        }
        let (id_len, rle, bits) = (data[0] as usize, data[2] == 10, data[16]);
        let w = u16::from_le_bytes([data[12], data[13]]) as usize;
        let h = u16::from_le_bytes([data[14], data[15]]) as usize;
        // RLE packets repeat a pixel at most 128 times; raw pixels take `bits / 8` bytes each.
        let packet = if rle { 1 + bits as usize / 8 } else { 128 * (bits as usize / 8) };
        if !tga_fits(data.len(), id_len, packet, w * h) {
            return None;
        }
        let mut px = vec![[0u8; 4]; w * h];
        let mut p = 18 + id_len;
        let read = |p: &mut usize| -> Option<[u8; 4]> {
            let n = bits as usize / 8;
            let s = data.get(*p..*p + n)?;
            *p += n;
            let a = if bits == 24 {
                if rle && invert_alpha { 0 } else { 255 }
            } else if rle == invert_alpha {
                255 - s[3]
            } else {
                s[3]
            };
            Some([s[2], s[1], s[0], a])
        };
        // Pixels fill rows from the bottom; RLE packets may span rows.
        let total = w * h;
        let mut n = 0;
        let mut put = |n: &mut usize, c: [u8; 4]| {
            if *n < total {
                let (row, col) = (h - 1 - *n / w, *n % w);
                px[row * w + col] = c;
                *n += 1;
            }
        };
        while n < total {
            if rle {
                let head = *data.get(p)?;
                p += 1;
                let count = (head & 0x7f) as usize + 1;
                if head & 0x80 != 0 {
                    let c = read(&mut p)?;
                    for _ in 0..count {
                        put(&mut n, c);
                    }
                } else {
                    for _ in 0..count {
                        let c = read(&mut p)?;
                        put(&mut n, c);
                    }
                }
            } else {
                let c = read(&mut p)?;
                put(&mut n, c);
            }
        }
        Some(Image { wide: w as i32, tall: h as i32, px })
    }

    /// The pixel at (`x`, `y`); transparent black outside the image.
    pub fn pixel(&self, x: i32, y: i32) -> [u8; 4] {
        if !(0..self.wide).contains(&x) || !(0..self.tall).contains(&y) {
            return [0; 4];
        }
        self.px.get((y * self.wide + x) as usize).copied().unwrap_or([0; 4])
    }
}

/// Whether a TGA of `data_len` bytes can hold `pixels` pixels when each `packet` bytes give at
/// most 128 of them, and the image stays under [`MAX_TGA_PIXELS`]. Guarded: the header's 16-bit
/// width and height would otherwise size the pixel buffer (up to 17 GB) before the data runs out.
fn tga_fits(data_len: usize, id_len: usize, packet: usize, pixels: usize) -> bool {
    let avail = data_len.saturating_sub(18 + id_len);
    pixels <= MAX_TGA_PIXELS && pixels <= avail.div_ceil(packet).saturating_mul(128)
}

/// The largest VGUI1 TGA the port decodes (64 MB of pixels); the shipped fonts are under a
/// million pixels.
const MAX_TGA_PIXELS: usize = 1 << 24;

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
            // "Default Scheme": Arial, size 0 (17 is only filled in for parsed schemes); white
            // foregrounds, the backgrounds left zero.
            let white = Color::from_rgba([255; 4]);
            let clear = Color::from_rgba([0; 4]);
            s.schemes.push(Scheme {
                name: "Default Scheme".into(),
                font_name: "Arial".into(),
                font_size: 0,
                font_weight: 0,
                fg: white,
                bg: clear,
                armed_fg: white,
                armed_bg: clear,
                mousedown_fg: white,
                mousedown_bg: clear,
                border: [0; 4],
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

    /// `getFont(getSchemeHandle(name))`.
    pub fn font_of(&self, name: &str) -> Option<&BitmapFont> {
        self.font(self.get(name))
    }
}

/// The scheme blocks (`0x1005c300`'s parse loop). `SchemeName` starts one; when the next starts,
/// the previous gets its defaults: an unset foreground is white and an unset background clear,
/// an unset armed colour copies the normal one and an unset mouse-down colour the armed one, an
/// unset size is 17 and an unset font Arial. `BorderColor` sets the mouse-down background's
/// "was set" flag, not its own (TS's code, as the SDK's). The last block gets no defaults: its
/// unset colours stay zero, as in TS's zeroed table.
fn parse(text: &str) -> Vec<Scheme> {
    #[derive(Default)]
    struct Raw {
        name: String,
        font_name: String,
        font_size: i32,
        font_weight: i32,
        c: [[u8; 4]; 6],
        border: [u8; 4],
        has: [bool; 6],
    }
    const FG: usize = 0;
    const BG: usize = 1;
    const ARMED_FG: usize = 2;
    const ARMED_BG: usize = 3;
    const MD_FG: usize = 4;
    const MD_BG: usize = 5;
    let finish = |r: &mut Raw| {
        if !r.has[FG] {
            r.c[FG] = [255; 4];
        }
        if !r.has[ARMED_FG] {
            r.c[ARMED_FG] = r.c[FG];
        }
        if !r.has[MD_FG] {
            r.c[MD_FG] = r.c[ARMED_FG];
        }
        if !r.has[BG] {
            r.c[BG] = [0; 4];
        }
        if !r.has[ARMED_BG] {
            r.c[ARMED_BG] = r.c[BG];
        }
        if !r.has[MD_BG] {
            r.c[MD_BG] = r.c[ARMED_BG];
        }
        if r.font_size == 0 {
            r.font_size = 17;
        }
        if r.font_name.is_empty() {
            r.font_name = "Arial".into();
        }
    };
    let t = tokens(text);
    let mut raw: Vec<Raw> = Vec::new();
    let mut i = 0;
    while i + 2 < t.len() {
        let (key, eq, val) = (&t[i], &t[i + 1], &t[i + 2]);
        if eq != "=" {
            break;
        }
        if key.eq_ignore_ascii_case("SchemeName") {
            if let Some(last) = raw.last_mut() {
                finish(last);
            }
            raw.push(Raw { name: val.chars().take(31).collect(), ..Raw::default() });
        } else if let Some(s) = raw.last_mut() {
            let mut colour = |k: usize| {
                s.c[k] = rgba(val);
                s.has[k] = true;
            };
            match key.to_ascii_lowercase().as_str() {
                "fontname" => s.font_name = val.chars().take(47).collect(),
                "fontsize" => s.font_size = val.trim().parse().unwrap_or(0),
                "fontweight" => s.font_weight = val.trim().parse().unwrap_or(0),
                "fgcolor" => colour(FG),
                "bgcolor" => colour(BG),
                "fgcolorarmed" => colour(ARMED_FG),
                "bgcolorarmed" => colour(ARMED_BG),
                "fgcolormousedown" => colour(MD_FG),
                "bgcolormousedown" => colour(MD_BG),
                "bordercolor" => {
                    s.border = rgba(val);
                    s.has[MD_BG] = true;
                }
                _ => {}
            }
        } else {
            break;
        }
        i += 3;
    }
    raw.into_iter()
        .map(|r| Scheme {
            name: r.name,
            font_name: r.font_name,
            font_size: r.font_size,
            font_weight: r.font_weight,
            fg: Color::from_rgba(r.c[FG]),
            bg: Color::from_rgba(r.c[BG]),
            armed_fg: Color::from_rgba(r.c[ARMED_FG]),
            armed_bg: Color::from_rgba(r.c[ARMED_BG]),
            mousedown_fg: Color::from_rgba(r.c[MD_FG]),
            mousedown_bg: Color::from_rgba(r.c[MD_BG]),
            border: r.border,
            font: 0,
        })
        .collect()
}

// ---- layout ---------------------------------------------------------------------------------

/// How TS scales its 640×480 VGUI1 layout to the screen (`XRES`/`YRES`), which depends on how
/// its `client.dll` was built ([`ClientRules::vgui_scale`](crate::gameplay::ClientRules::vgui_scale)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VguiScale {
    /// An optimised build (TS 3.0): `ScreenWidth * (1/640)f` and `ScreenHeight * (1/480)f`, the
    /// reciprocals as the floats TS multiplies by (`0x1007d43c`, `0x1007d438`, a hair above the
    /// exact values), and the few values TS spills to a `float` rounded there ([`spill`]).
    Reciprocal,
    /// An unoptimised build (TS 2.1): `ScreenWidth / 640.0f` and `ScreenHeight / 480.0f`
    /// (`fild; fdiv`), every expression kept on the x87 stack, so nothing is spilled.
    Division,
}

thread_local! {
    /// The scale for the last HUD ruleset seen (`SrvSett` 4), so the per-cell callers don't
    /// resolve the ruleset each time.
    static SCALE: std::cell::Cell<Option<(Option<i16>, VguiScale)>> = const { std::cell::Cell::new(None) };
    /// A scale forced by [`with_scale`].
    static FORCED: std::cell::Cell<Option<VguiScale>> = const { std::cell::Cell::new(None) };
}

thread_local! {
    /// The last [`raise`] stamp.
    static RAISED: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// A panel's place in the viewport's child list: VGUI1's `Panel::setParent` (`vgui.so`
/// `0x32e80`) removes a panel from its parent and appends it again, so a panel parented later
/// paints over those parented before. The viewport's own panels (team menu, scoreboard,
/// spectator bar) are children from its constructor; the voice labels and talk icon
/// (`CVoiceStatus`) and the message windows (`CreateTextWindow` `0x10063c00`) are parented when
/// shown, and paint in the order of these stamps.
pub fn raise() -> u64 {
    RAISED.with(|r| {
        r.set(r.get() + 1);
        r.get()
    })
}

/// The HUD ruleset's [`VguiScale`] (or the one [`with_scale`] forces).
pub fn scale() -> VguiScale {
    if let Some(s) = FORCED.with(|f| f.get()) {
        return s;
    }
    let id = crate::hud::gameplay_id();
    SCALE.with(|c| match c.get() {
        Some((cached, s)) if cached == id => s,
        _ => {
            let s = crate::gameplay::hud_rules(id).vgui_scale();
            c.set(Some((id, s)));
            s
        }
    })
}

/// Runs `f` with the VGUI scale forced to `s` (tests; the previous one is restored).
pub fn with_scale<R>(s: VguiScale, f: impl FnOnce() -> R) -> R {
    let old = FORCED.with(|c| c.replace(Some(s)));
    let r = f();
    FORCED.with(|c| c.set(old));
    r
}

/// TS's `XRES` scale ([`VguiScale`]). The callers multiply by the 640-based size and truncate
/// where TS does (`__ftol`); the products are exact in `f64`, as in the x87's 53-bit precision.
pub fn xs(width: i32) -> f64 {
    match scale() {
        VguiScale::Reciprocal => width as f64 * 0.0015625f32 as f64,
        VguiScale::Division => width as f64 / 640.0,
    }
}

/// TS's `YRES` scale ([`VguiScale`]).
pub fn ys(height: i32) -> f64 {
    match scale() {
        VguiScale::Reciprocal => height as f64 * (1.0f32 / 480.0) as f64,
        VguiScale::Division => height as f64 / 480.0,
    }
}

/// `__ftol`: truncation towards zero.
pub fn ftol(v: f64) -> i32 {
    v as i32
}

/// A value TS 3.0 spills to a `float` on the stack (`fst DWORD PTR`) and reloads: MSVC's x87
/// code otherwise keeps double precision, so only these round to single precision. TS 2.1's
/// unoptimised code ([`VguiScale::Division`]) keeps the same values on the x87 stack.
pub fn spill(v: f64) -> f64 {
    match scale() {
        VguiScale::Reciprocal => v as f32 as f64,
        VguiScale::Division => v,
    }
}

/// `vgui::Label::Alignment`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    NorthWest,
    North,
    NorthEast,
    West,
    Center,
    East,
    SouthWest,
    South,
    SouthEast,
}

impl Align {
    fn column(self) -> i32 {
        self as i32 % 3
    }
    fn row(self) -> i32 {
        self as i32 / 3
    }
}

/// `Label::computeAlignment` for a label with text only: where text `tw`×`th` goes in a label
/// whose paint size is `w`×`h` (the text alignment doesn't matter without an image).
pub fn label_text_pos(align: Align, w: i32, h: i32, tw: i32, th: i32) -> (i32, i32) {
    let x = match align.column() {
        0 => 0,
        1 => w / 2 - tw / 2,
        _ => w - tw,
    };
    let y = match align.row() {
        0 => 0,
        1 => h / 2 - th / 2,
        _ => h - th,
    };
    (x, y)
}

/// `Label::getContentSize` for text only: VGUI pads the content by 4 on each side, so a
/// content-fitted label (`Label(text, x, y)`) is the text size plus 8.
pub fn label_content_size(tw: i32, th: i32) -> (i32, i32) {
    (tw + 8, th + 8)
}

/// A screen rectangle (x0, y0) inclusive to (x1, y1) exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl Rect {
    /// Corners wrap as VGUI's C ints do.
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x0: x, y0: y, x1: x.wrapping_add(w), y1: y.wrapping_add(h) }
    }
    pub fn intersect(self, o: Rect) -> Rect {
        Rect { x0: self.x0.max(o.x0), y0: self.y0.max(o.y0), x1: self.x1.min(o.x1), y1: self.y1.min(o.y1) }
    }
    pub fn is_empty(self) -> bool {
        self.x1 <= self.x0 || self.y1 <= self.y0
    }
}

/// Where the engine's VGUI surface draws: filled rectangles in an RGB and opacity.
pub trait Surface {
    fn fill(&mut self, x: i32, y: i32, w: i32, h: i32, rgb: [u8; 3], opacity: u8);
}

/// The engine: `pfnFillRGBABlend` (see `goldsrc::ext::hud`'s `fill_blend`).
pub struct EngineSurface;

impl Surface for EngineSurface {
    fn fill(&mut self, x: i32, y: i32, w: i32, h: i32, rgb: [u8; 3], opacity: u8) {
        engine().fill_blend(x, y, w, h, [rgb[0] as i32, rgb[1] as i32, rgb[2] as i32], opacity as i32);
    }
}

/// A VGUI1 panel's place on screen: position, size, border inset and clip rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Panel {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    /// The border's inset (1 for a `LineBorder`, else 0).
    pub inset: i32,
    pub clip: Rect,
}

impl Panel {
    /// The viewport: the whole screen.
    pub fn root(w: i32, h: i32) -> Panel {
        Panel { x: 0, y: 0, w, h, inset: 0, clip: Rect::new(0, 0, w, h) }
    }

    /// A child at (`x`, `y`) in this panel (`Panel::solve`: after this panel's inset), clipped to
    /// its bounds and this panel's clip rectangle.
    pub fn child(&self, x: i32, y: i32, w: i32, h: i32) -> Panel {
        let (ax, ay) = (self.x.wrapping_add(self.inset).wrapping_add(x), self.y.wrapping_add(self.inset).wrapping_add(y));
        Panel { x: ax, y: ay, w, h, inset: 0, clip: Rect::new(ax, ay, w, h).intersect(self.clip) }
    }

    /// The same panel with a 1-pixel `LineBorder`.
    pub fn bordered(self) -> Panel {
        Panel { inset: 1, ..self }
    }

    /// `Panel::getPaintSize`: the size inside the border.
    pub fn paint_size(&self) -> (i32, i32) {
        let border = self.inset.wrapping_mul(2);
        (self.w.wrapping_sub(border), self.h.wrapping_sub(border))
    }

    /// `Panel::isWithin`: a screen point inside the panel's bounds (not its clip).
    pub fn is_within(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.x.wrapping_add(self.w) && y >= self.y && y < self.y.wrapping_add(self.h)
    }

    fn fill_abs(&self, s: &mut dyn Surface, r: Rect, c: Color) {
        // FWGS `XashSurface::drawFilledRect` and GoldSrc both skip fully transparent colours.
        if c.a == 255 {
            return;
        }
        let r = r.intersect(self.clip);
        if !r.is_empty() {
            s.fill(r.x0, r.y0, r.x1.wrapping_sub(r.x0), r.y1.wrapping_sub(r.y0), c.rgb(), c.opacity());
        }
    }

    /// `drawFilledRect(x0, y0, x1, y1)` in the panel's paint coordinates (inside its inset).
    pub fn fill_rect(&self, s: &mut dyn Surface, x0: i32, y0: i32, x1: i32, y1: i32, c: Color) {
        let (ox, oy) = (self.x.wrapping_add(self.inset), self.y.wrapping_add(self.inset));
        let r = Rect { x0: ox.wrapping_add(x0), y0: oy.wrapping_add(y0), x1: ox.wrapping_add(x1), y1: oy.wrapping_add(y1) };
        self.fill_abs(s, r, c);
    }

    /// `drawOutlinedRect`: four 1-pixel fills (`Surface::drawOutlinedRect`).
    pub fn outlined_rect(&self, s: &mut dyn Surface, x0: i32, y0: i32, x1: i32, y1: i32, c: Color) {
        let (x0p, y0p, x1m, y1m) = (x0.wrapping_add(1), y0.wrapping_add(1), x1.wrapping_sub(1), y1.wrapping_sub(1));
        self.fill_rect(s, x0, y0, x1, y0p, c);
        self.fill_rect(s, x0, y1m, x1, y1, c);
        self.fill_rect(s, x0, y0p, x0p, y1m, c);
        self.fill_rect(s, x1m, y0p, x1, y1m, c);
    }

    /// `Panel::paintBackground`: the paint size in the background colour.
    pub fn paint_background(&self, s: &mut dyn Surface, bg: Color) {
        let (w, h) = self.paint_size();
        self.fill_rect(s, 0, 0, w, h, bg);
    }

    /// `LineBorder::paint`: drawn at the panel's own origin (no inset).
    pub fn line_border(&self, s: &mut dyn Surface, c: Color) {
        let (x, y, i) = (self.x, self.y, self.inset);
        let (xw, yh) = (x.wrapping_add(self.w), y.wrapping_add(self.h));
        let (xi, yi, xwi, yhi) = (x.wrapping_add(i), y.wrapping_add(i), xw.wrapping_sub(i), yh.wrapping_sub(i));
        self.fill_abs(s, Rect { x0: x, y0: y, x1: xw, y1: yi }, c);
        self.fill_abs(s, Rect { x0: x, y0: yhi, x1: xw, y1: yh }, c);
        self.fill_abs(s, Rect { x0: x, y0: yi, x1: xi, y1: yhi }, c);
        self.fill_abs(s, Rect { x0: xwi, y0: yi, x1: xw, y1: yhi }, c);
    }

    /// `TextImage::paint` of `text` at (`x`, `y`) in paint coordinates, laid out `wrap` wide.
    #[allow(clippy::too_many_arguments)] // TextImage's own parameters: position, colour, text, width
    pub fn text(&self, s: &mut dyn Surface, font: &BitmapFont, x: i32, y: i32, c: Color, text: &[u8], wrap: i32) {
        if c.a == 255 {
            return;
        }
        let (ox, oy) = (self.x.wrapping_add(self.inset).wrapping_add(x), self.y.wrapping_add(self.inset).wrapping_add(y));
        font.paint(text, wrap, |rx, ry, w, h| {
            let r = Rect::new(ox.wrapping_add(rx), oy.wrapping_add(ry), w, h).intersect(self.clip);
            if !r.is_empty() {
                s.fill(r.x0, r.y0, r.x1.wrapping_sub(r.x0), r.y1.wrapping_sub(r.y0), c.rgb(), c.opacity());
            }
        });
    }

    /// `Bitmap::paint` at (`x`, `y`) in paint coordinates: the texture modulated by the image's
    /// colour (`drawSetColor` before `drawTexturedRect`), each pixel's opacity scaled by the
    /// colour's. Runs of equal pixels share a fill.
    pub fn image(&self, s: &mut dyn Surface, img: &Image, x: i32, y: i32, c: Color) {
        let (ox, oy) = (self.x.wrapping_add(self.inset).wrapping_add(x), self.y.wrapping_add(self.inset).wrapping_add(y));
        let m = |t: u8, k: u8| ((t as u32 * k as u32 + 127) / 255) as u8;
        let out = |p: [u8; 4]| ([m(p[0], c.r), m(p[1], c.g), m(p[2], c.b)], m(p[3], c.opacity()));
        for py in 0..img.tall {
            let sy = oy.wrapping_add(py);
            if sy < self.clip.y0 || sy >= self.clip.y1 {
                continue;
            }
            let mut px = 0;
            while px < img.wide {
                let o = out(img.pixel(px, py));
                let start = px;
                px += 1;
                while px < img.wide && out(img.pixel(px, py)) == o {
                    px += 1;
                }
                if o.1 == 0 {
                    continue;
                }
                let r = Rect::new(ox.wrapping_add(start), sy, px - start, 1).intersect(self.clip);
                if !r.is_empty() {
                    s.fill(r.x0, r.y0, r.x1.wrapping_sub(r.x0), 1, o.0, o.1);
                }
            }
        }
    }
}

// ---- engine side ----------------------------------------------------------------------------

use goldsrc::Global;
use std::cell::RefCell;

type ImageCache = HashMap<(String, bool), Option<Rc<Image>>>;

static SCHEMES: Global<RefCell<Option<Schemes>>> = Global(RefCell::new(None));
static IMAGES: Global<RefCell<Option<ImageCache>>> = Global(RefCell::new(None));

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

/// Runs `f` with the scheme manager, if built.
pub fn with_schemes<R>(f: impl FnOnce(&Schemes) -> R) -> Option<R> {
    SCHEMES.0.borrow().as_ref().map(f)
}

/// `vgui_LoadTGA` (`invert_alpha`) / `vgui_LoadTGANoInvertAlpha` through `COM_LoadFile`, loaded
/// once per path.
pub fn image(path: &str, invert_alpha: bool) -> Option<Rc<Image>> {
    let mut cache = IMAGES.0.borrow_mut();
    let cache = cache.get_or_insert_with(HashMap::new);
    cache
        .entry((path.to_owned(), invert_alpha))
        .or_insert_with(|| goldsrc::ext::render::load_file(path).and_then(|d| Image::from_tga(&d, invert_alpha)).map(Rc::new))
        .clone()
}

/// Draws `text` in scheme `scheme`'s font at (`x`, `y`) in `rgb` (opaque), as a VGUI1 label's
/// text image paints it, clipped to the label's rectangle `clip` (x, y, w, h) as VGUI clips a
/// panel's painting. Returns false if the scheme has no font.
pub fn draw_text_clipped(scheme: &str, x: i32, y: i32, rgb: [u8; 3], text: &[u8], clip: (i32, i32, i32, i32)) -> bool {
    let s = SCHEMES.0.borrow();
    let Some(font) = s.as_ref().and_then(|s| s.font(s.get(scheme))) else { return false };
    let (w, _) = font.text_size(text);
    let panel = Panel { x: 0, y: 0, w: 0, h: 0, inset: 0, clip: Rect::new(clip.0, clip.1, clip.2, clip.3) };
    panel.text(&mut EngineSurface, font, x, y, Color::new(rgb[0], rgb[1], rgb[2], 0), text, w);
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
    s.as_ref().map_or([255, 170, 0], |s| s.get(scheme).fg.rgb())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// `Panel::setParent` appends: each panel parented later paints above (issue #277).
    #[test]
    fn raise_orders_panels() {
        let (a, b) = (raise(), raise());
        assert!(a < b);
    }

    /// A surface that records fills.
    #[derive(Default)]
    pub struct Record(pub Vec<(i32, i32, i32, i32, [u8; 3], u8)>);
    impl Surface for Record {
        fn fill(&mut self, x: i32, y: i32, w: i32, h: i32, rgb: [u8; 3], opacity: u8) {
            self.0.push((x, y, w, h, rgb, opacity));
        }
    }

    pub fn tga(w: u16, h: u16, rows_bottom_up: &[&[[u8; 4]]]) -> Vec<u8> {
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

    /// A 4×2-cell font: 'A' has ink at x=2 in the top row, '.' at x=0 in the bottom row.
    pub fn test_font() -> BitmapFont {
        const INK: [u8; 4] = [255, 255, 255, 0];
        let mut top = vec![[0u8; 4]; 1024];
        let mut bottom = vec![[0u8; 4]; 1024];
        top[65 * 4 + 2] = INK;
        bottom[b'.' as usize * 4] = INK;
        BitmapFont::from_tga(&tga(1024, 2, &[&bottom, &top])).unwrap()
    }

    #[test]
    fn bitmap_font_metrics() {
        let f = test_font();
        assert_eq!((f.cell_wide, f.tall()), (4, 2));
        assert_eq!((f.widths[65], f.widths[b'.' as usize], f.widths[b' ' as usize], f.widths[b'x' as usize]), (3, 1, 2, 1));
        assert_eq!(f.text_size(b"A A"), (8, 2));
        assert_eq!(f.text_size(b"A\nA"), (4, 4), "a newline counts its own width");
        let mut fills = Vec::new();
        f.paint(b"A.", 100, |x, y, w, h| fills.push((x, y, w, h)));
        assert_eq!(fills, vec![(2, 0, 1, 1), (3, 1, 1, 1)]);
        fills.clear();
        f.paint(b"A ", 100, |x, y, w, h| fills.push((x, y, w, h)));
        f.paint(b" .", 100, |x, y, w, h| fills.push((x, y, w, h)));
        assert_eq!(fills, vec![(2, 0, 1, 1), (2, 1, 1, 1)], "a trailing space doesn't advance; a leading one does");
        // A word that doesn't fit the image's width starts a new line.
        fills.clear();
        f.paint(b"A A", 5, |x, y, w, h| fills.push((x, y, w, h)));
        assert_eq!(fills, vec![(2, 0, 1, 1), (2, 2, 1, 1)]);
        assert_eq!(f.text_size_wrapped(b"A A", 5), (3, 4));
        assert_eq!(f.chars_wide(b"A.\0A"), 4);
    }

    #[test]
    fn glyph_runs_merge_into_rectangles() {
        let mut rows = vec![vec![(1, 2)], vec![(1, 2), (5, 1)], vec![(5, 1)], vec![(1, 2)]];
        let mut r = merge_runs(&mut rows);
        r.sort();
        assert_eq!(r, vec![(1, 0, 2, 2), (1, 3, 2, 1), (5, 1, 1, 2)]);
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
        f.paint(b"Free-Look", 1000, |_, _, w, h| ink += w * h);
        assert!(ink > 100);
        // Rows are stored bottom-up: descenders land in the last rows.
        let mut rows = [0; 13];
        f.paint(b"gjy", 1000, |_, y, w, h| {
            for r in y..y + h {
                rows[r as usize] += w;
            }
        });
        assert!(rows[12] > 0 && rows[0] == 0);
    }

    /// The shipped voice icons decode with TS's loaders (skipped without the game files).
    #[test]
    fn decodes_the_shipped_voice_icons() {
        let dir = std::env::var("HOME").unwrap_or_default() + "/games/xash_win_orig/valve/gfx/vgui/";
        let Ok(pl) = std::fs::read(dir.clone() + "icntlk_pl.tga") else { return };
        let pl = Image::from_tga(&pl, true).unwrap();
        assert_eq!((pl.wide, pl.tall), (32, 32));
        assert_eq!(pl.pixel(5, 15)[3], 255, "vgui_LoadTGA keeps an uncompressed icon's alpha");
        assert_eq!(pl.pixel(0, 0)[3], 0);
        let sp = Image::from_tga(&std::fs::read(dir.clone() + "640_speaker3.tga").unwrap(), false).unwrap();
        assert_eq!((sp.wide, sp.tall), (16, 16));
        assert!((0..16).any(|y| (0..16).any(|x| sp.pixel(x, y)[3] == 0)) && (0..16).any(|y| (0..16).any(|x| sp.pixel(x, y)[3] > 200)));
    }

    #[test]
    fn scheme_parse_and_font_sharing() {
        let file = r#"// test
SchemeName = "Primary Button Text"
FontName = "Arial"
FontSize = 20
FgColor = "255 170 0 255"
BgColorArmed = "255 170 0 67"
SchemeName = "Scoreboard Text"
FontName = "Arial"
FontSize = 15
FontWeight = 0
BorderColor = "1 1 1 1"
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
        let p = s.get("Primary Button Text");
        assert_eq!((p.fg, p.armed_fg, p.mousedown_fg), (Color::new(255, 170, 0, 0), p.fg, p.fg), "armed copies normal");
        assert_eq!((p.bg, p.armed_bg, p.mousedown_bg), (Color::new(0, 0, 0, 255), Color::new(255, 170, 0, 188), p.armed_bg));
        let sb = s.get("Scoreboard Text");
        assert_eq!(sb.fg, Color::new(255, 255, 255, 0), "unset foreground is white");
        assert_eq!((sb.border, sb.mousedown_bg), ([1; 4], Color::new(0, 0, 0, 255)), "BorderColor sets the mouse-down flag");
        assert_eq!(s.get("briefing text").fg, Color::new(1, 2, 3, 251));
        let last = s.get("No Size");
        assert_eq!((last.font_size, last.fg), (0, Color::new(0, 0, 0, 255)), "the last block gets no defaults");
        assert_eq!(s.get("briefing text").font, s.get("Scoreboard Text").font, "same name, size and weight share a font");
        assert_ne!(s.get("Primary Button Text").font, s.get("Scoreboard Text").font);
        assert_eq!(s.get("missing").name, "Primary Button Text");
        assert!(loaded.borrow().contains(&"gfx/vgui/fonts/1000_Primary Button Text.tga".to_owned()));
        assert!(loaded.borrow().contains(&"gfx/vgui/fonts/800_Primary Button Text.tga".to_owned()), "system-font fallback");
        assert!(!loaded.borrow().iter().any(|p| p.contains("Briefing")), "a shared font isn't loaded again");
    }

    #[test]
    fn tga_alpha_rules() {
        // 2×1 uncompressed 32-bit: alpha kept with invert on, inverted with it off.
        let mut d = vec![0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 1, 0, 32, 8];
        d.extend_from_slice(&[1, 2, 3, 200, 4, 5, 6, 0]);
        let on = Image::from_tga(&d, true).unwrap();
        assert_eq!((on.wide, on.tall, on.pixel(0, 0), on.pixel(1, 0)), (2, 1, [3, 2, 1, 200], [6, 5, 4, 0]));
        assert_eq!(Image::from_tga(&d, false).unwrap().pixel(0, 0)[3], 55);
        // RLE: the other way round; packets may span rows; rows are stored bottom-up.
        let mut r = vec![0, 0, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 2, 0, 32, 8];
        r.extend_from_slice(&[0x81, 9, 9, 9, 10]);
        let img = Image::from_tga(&r, false).unwrap();
        assert_eq!((img.pixel(0, 0), img.pixel(0, 1)), ([9, 9, 9, 10], [9, 9, 9, 10]));
        assert_eq!(Image::from_tga(&r, true).unwrap().pixel(0, 0)[3], 245);
        assert!(Image::from_tga(&r[..20], true).is_none(), "truncated");
    }

    /// Game-dir TGAs can claim any size: a 65535×65535 header with no data is refused before
    /// anything is allocated, and truncated or random files decode to `None` or an image.
    #[test]
    fn hostile_tgas() {
        for (kind, bits) in [(2u8, 24u8), (2, 32), (10, 24), (10, 32)] {
            let mut d = vec![0, 0, kind, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff, bits, 0];
            assert!(Image::from_tga(&d, true).is_none());
            assert!(BitmapFont::from_tga(&d).is_none());
            // One RLE packet can cover 128 pixels: enough data for a 128-wide row is accepted.
            d[12..16].copy_from_slice(&[128, 0, 1, 0]);
            d.extend_from_slice(&[0xff, 1, 2, 3, 4]);
            assert_eq!(Image::from_tga(&d, false).is_some(), kind == 10);
        }
        let mut seed = 0x0bad_c0de_dead_beefu64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for i in 0..400 {
            let n = 18 + (next() % 300) as usize;
            let mut d: Vec<u8> = (0..n).map(|_| next() as u8).collect();
            d[2] = [2, 10][i % 2];
            d[16] = [24, 32][(i / 2) % 2];
            d[1] = 0;
            d[13] %= 2;
            d[15] %= 2;
            if let Some(img) = Image::from_tga(&d, i % 3 == 0) {
                let _ = (img.pixel(-1, 0), img.pixel(img.wide, img.tall), img.pixel(i32::MAX, i32::MIN));
                Panel::root(64, 64).image(&mut Record::default(), &img, i32::MIN / 2, 3, Color::new(1, 2, 3, 4));
            }
            if let Some(f) = BitmapFont::from_tga(&d) {
                let _ = (f.text_size(&d), f.text_size_wrapped(&d, i32::MIN), f.chars_wide(&d));
            }
        }
    }

    /// Panels and rectangles at the ends of the int range draw without overflowing.
    #[test]
    fn panels_at_extreme_coordinates() {
        let ends = [i32::MIN, -1, 0, 1, i32::MAX];
        let mut s = Record::default();
        let font = test_font();
        for &a in &ends {
            for &b in &ends {
                let p = Panel::root(a, b).bordered().child(a, b, b, a).bordered();
                p.fill_rect(&mut s, a, b, b, a, Color::new(1, 1, 1, 0));
                p.outlined_rect(&mut s, a, b, b, a, Color::new(1, 1, 1, 0));
                p.line_border(&mut s, Color::new(1, 1, 1, 0));
                p.paint_background(&mut s, Color::new(1, 1, 1, 0));
                p.text(&mut s, &font, a, b, Color::new(1, 1, 1, 0), b"A. A", a);
                let _ = (p.is_within(a, b), p.paint_size(), Rect::new(a, b, a, b).is_empty());
            }
        }
    }

    /// A scheme file of random tokens parses into something usable.
    #[test]
    fn hostile_schemes() {
        let bad = "SchemeName = \nFontSize = 99999999999 FgColor = \"-5 999 x\" = = = // \n\"unterminated";
        let s = Schemes::load(i32::MAX, true, &|p: &str| p.ends_with("textscheme.txt").then(|| bad.as_bytes().to_vec()));
        assert!(!s.schemes.is_empty());
        let _ = (s.get("nope"), s.font_of(""));
        let s = Schemes::load(i32::MIN, false, &|_: &str| Some(vec![0xff; 64]));
        assert_eq!(s.get("x").name, "Default Scheme");
        assert_eq!(rgba("300 -1 1e9 nan 7"), [44, 255, 0, 0]);
    }

    #[test]
    fn panel_clipping_and_drawing() {
        let root = Panel::root(640, 480);
        let win = root.child(100, 50, 200, 100).bordered();
        let label = win.child(10, 10, 300, 20);
        assert_eq!((label.x, label.y), (111, 61), "children sit inside the parent's border");
        assert_eq!(label.clip, Rect { x0: 111, y0: 61, x1: 300, y1: 81 }, "clipped to the parent");
        assert_eq!(win.paint_size(), (198, 98));
        let mut s = Record::default();
        win.line_border(&mut s, Color::new(178, 118, 0, 0));
        assert_eq!(s.0[0], (100, 50, 200, 1, [178, 118, 0], 255));
        assert_eq!(s.0[3], (299, 51, 1, 98, [178, 118, 0], 255));
        s.0.clear();
        win.paint_background(&mut s, Color::new(0, 0, 0, 255));
        assert!(s.0.is_empty(), "fully transparent colours draw nothing");
        win.fill_rect(&mut s, 0, 0, 500, 500, Color::new(0, 0, 0, 100));
        assert_eq!(s.0[0], (101, 51, 199, 99, [0, 0, 0], 155), "paints inside the inset, clipped to the panel");
        s.0.clear();
        root.outlined_rect(&mut s, 0, 0, 10, 5, Color::new(1, 1, 1, 0));
        assert_eq!(s.0, vec![(0, 0, 10, 1, [1; 3], 255), (0, 4, 10, 1, [1; 3], 255), (0, 1, 1, 3, [1; 3], 255), (9, 1, 1, 3, [1; 3], 255)]);
        s.0.clear();
        root.text(&mut s, &test_font(), 5, 6, Color::new(255, 0, 0, 5), b"A", 100);
        assert_eq!(s.0, vec![(7, 6, 1, 1, [255, 0, 0], 250)]);
        assert!(label.is_within(111, 61) && !label.is_within(110, 61));
    }

    #[test]
    fn image_modulation() {
        let mut d = vec![0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 0, 1, 0, 32, 8];
        d.extend_from_slice(&[255, 255, 255, 255, 255, 255, 255, 255, 0, 0, 0, 0]);
        let img = Image::from_tga(&d, true).unwrap();
        let mut s = Record::default();
        Panel::root(640, 480).image(&mut s, &img, 10, 20, Color::new(255, 170, 0, 1));
        assert_eq!(s.0, vec![(10, 20, 2, 1, [255, 170, 0], 254)], "equal pixels share a fill; clear ones draw nothing");
    }

    #[test]
    fn label_and_scale_maths() {
        assert_eq!(label_text_pos(Align::West, 100, 20, 30, 10), (0, 5));
        assert_eq!(label_text_pos(Align::North, 101, 20, 31, 10), (35, 0));
        assert_eq!(label_text_pos(Align::East, 100, 21, 30, 10), (70, 5));
        assert_eq!(label_text_pos(Align::SouthEast, 100, 20, 30, 10), (70, 10));
        assert_eq!(label_content_size(30, 10), (38, 18));
        assert_eq!((ftol(xs(640) * 40.0), ftol(ys(480) * 32.0)), (40, 32));
        assert_eq!((ftol(xs(800) * 124.0), ftol(ys(600) * 24.0)), (155, 30));
        assert_eq!((ftol(xs(1024) * 176.0), ftol(ys(768) * 312.0)), (281, 499));
    }
}
