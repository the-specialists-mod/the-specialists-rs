//! VGUI1's scrolling widgets as TS's panels use them, drawn with engine fills:
//!
//! * `CTFScrollPanel` (`0x1005a860`): VGUI1's `ScrollPanel` (Valve's `vgui.dll`; the code below
//!   follows `vgui.so`'s, which has symbols, and FWGS's `3rdparty/freevgui`, which reimplements
//!   it) with TS's 16×16 `CTFScrollButton`s (`0x1005a620`, paint `0x1005a6b0`) and `CTFSlider`s
//!   (paint `0x1005a750`) put into both scroll bars, whose own border, background and paint are
//!   switched off. The team menu's briefing and the MOTD window scroll with it.
//! * `CListBox` (the SDK's `vgui_listbox.cpp`, constructor `0x1005b490`, `InternalLayout`
//!   `0x1005b730`) with its `ScrollBar2` (`0x100153f0`) and `Slider2` (`0x10016690`): the
//!   scoreboard's player list.
//!
//! Input is VGUI1's: a slider press inside the nob starts a drag (the slider captures the mouse
//! until the release), a left release over a scroll button moves the value by the bar's step, and
//! a `CTFScrollButton` arms while the cursor is over it. Nothing here takes the mouse wheel: no
//! VGUI1 input signal in these panels handles `mouseWheeled` (`vgui.so`'s slider signal's is an
//! empty function, as is the SDK `Slider2`'s and TS's viewport's), so TS's panels don't scroll by
//! wheel.

use super::widgets::ViewportScheme;
use crate::hud::vgui::{Color, Image, Panel, Surface};
use std::rc::Rc;

/// x87 `cvttss2si`/`fistp` on a value stored as a `float`: truncation, and the "integer
/// indefinite" `0x80000000` for NaN and out-of-range values (Rust's `as` would saturate).
fn cvtt(v: f64) -> i32 {
    let f = v as f32;
    if f.is_nan() || !(-2147483648.0..2147483648.0).contains(&f) { i32::MIN } else { f as i32 }
}

/// Which slider class: VGUI1's own `vgui::Slider` (`CTFSlider` derives from it) or the SDK's
/// `vgui::Slider2` (the list box's). They differ in the nob's size: VGUI1's is proportional
/// (`size / window * size`), `Slider2`'s is the range window itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Vgui,
    Sdk2,
}

/// A slider (`vgui::Slider` / `vgui::Slider2`). It has no border, so its paint size is its size.
#[derive(Debug, Clone, PartialEq)]
pub struct Slider {
    pub kind: Kind,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub vertical: bool,
    pub value: i32,
    pub range: [i32; 2],
    pub window: i32,
    pub window_enabled: bool,
    pub button_offset: i32,
    pub nob: [i32; 2],
    dragging: bool,
    drag_start: [i32; 2],
    nob_drag_start: [i32; 2],
}

impl Slider {
    /// The constructor: range 0..299 (`0x12b`), value and window 0, window disabled.
    pub fn new(kind: Kind, x: i32, y: i32, w: i32, h: i32, vertical: bool) -> Slider {
        let mut s = Slider {
            kind,
            x,
            y,
            w,
            h,
            vertical,
            value: 0,
            range: [0, 299],
            window: 0,
            window_enabled: false,
            button_offset: 0,
            nob: [0; 2],
            dragging: false,
            drag_start: [0; 2],
            nob_drag_start: [0; 2],
        };
        s.recompute_nob();
        s
    }

    fn size(&self) -> i32 {
        if self.vertical { self.h } else { self.w }
    }

    /// `Panel::setBounds`: the position, then the virtual `setSize`.
    pub fn set_bounds(&mut self, x: i32, y: i32, w: i32, h: i32) {
        (self.x, self.y) = (x, y);
        self.set_size(w, h);
    }

    /// `setSize`: `Panel::setSize`, then `recomputeNobPosFromValue`.
    pub fn set_size(&mut self, w: i32, h: i32) {
        (self.w, self.h) = (w, h);
        self.recompute_nob();
    }

    /// The window the maths use: the range window (not below 0) when enabled, else the span.
    fn eff_window(&self) -> f32 {
        if self.window_enabled { (self.window as f32).max(0.0) } else { (self.range[1] - self.range[0]) as f32 }
    }

    /// `hasFullRange` (`vgui.so` `0x36300`, `Slider2` `0x10016b30`): the window fits the slider
    /// and the buttons; never with an empty window.
    pub fn has_full_range(&self) -> bool {
        let window = self.eff_window();
        window > 0.0 && window <= self.size() as f32 + self.button_offset as f32
    }

    /// `recomputeNobPosFromValue` (`vgui.so` `0x360a0`, `Slider2` `0x100168d0`). Nothing moves
    /// with an empty window. An empty span divides by zero, as TS's does, and the NaN truncates to
    /// `0x80000000`.
    fn recompute_nob(&mut self) {
        let window = self.eff_window();
        if window <= 0.0 {
            return;
        }
        let size = self.size();
        let span = (self.range[1] - self.range[0]) as f32;
        let offset = (self.value - self.range[0]) as f32;
        let (pos, nob_size) = match self.kind {
            Kind::Vgui => {
                // x87 throughout, each result stored as a float before the truncation.
                let (fsize, w, frac) = (size as f64, window as f64, offset as f64 / span as f64);
                let nob = fsize / w * fsize;
                ((fsize - nob) * frac, nob)
            }
            Kind::Sdk2 => {
                let fper = offset / span;
                let first = (size as f32 - window) * fper;
                (first as f64, window as f64)
            }
        };
        self.nob = [cvtt(pos), cvtt(pos as f32 as f64 + nob_size)];
        if self.nob[1] > size {
            self.nob = [size - cvtt(nob_size), size];
        }
    }

    /// `recomputeValueFromNobPos` (`vgui.so` `0x36200`, `Slider2` `0x10016a30`): only clamped at
    /// the top.
    fn recompute_value(&mut self) {
        let window = self.eff_window();
        let span = (self.range[1] - self.range[0]) as f32;
        let mut offset = (self.value - self.range[0]) as f64;
        if window > 0.0 {
            let (size, nob0) = (self.size() as f64, self.nob[0] as f64);
            offset = match self.kind {
                Kind::Vgui => nob0 / (size - size / window as f64 * size) * span as f64,
                Kind::Sdk2 => (span * (nob0 as f32 / (size as f32 - window))) as f64,
            };
        }
        let v = match self.kind {
            Kind::Vgui => cvtt_d(self.range[0] as f64 + offset + 0.5),
            Kind::Sdk2 => cvtt_d((offset as f32 + self.range[0] as f32) as f64 + 0.5),
        };
        self.value = v.min(self.range[1]);
    }

    /// `setValue`: clamped to the range; true if it changed (the int-change signal fires).
    pub fn set_value(&mut self, v: i32) -> bool {
        let old = self.value;
        self.value = v.max(self.range[0]).min(self.range[1]);
        self.recompute_nob();
        self.value != old
    }

    /// `setRange`: the maximum is raised to the minimum. The nob isn't moved.
    pub fn set_range(&mut self, min: i32, max: i32) {
        self.range = [min, max.max(min)];
    }

    /// `privateMousePressed` at slider-local (`x`, `y`): a press inside the nob starts a drag.
    fn press(&mut self, x: i32, y: i32) {
        let p = if self.vertical { y } else { x };
        if p < self.nob[0] || p >= self.nob[1] {
            return;
        }
        self.dragging = true;
        self.nob_drag_start = self.nob;
        self.drag_start = [x, y];
    }

    /// `privateCursorMoved` while dragging: the nob follows the cursor within the slider, the
    /// value follows the nob and the int-change signal fires (returns true).
    fn drag(&mut self, x: i32, y: i32) -> bool {
        if !self.dragging {
            return false;
        }
        let size = self.size();
        let d = if self.vertical { y - self.drag_start[1] } else { x - self.drag_start[0] };
        self.nob = [self.nob_drag_start[0] + d, self.nob_drag_start[1] + d];
        if self.nob[1] > size {
            self.nob = [size - (self.nob[1] - self.nob[0]), size];
        }
        if self.nob[0] < 0 {
            self.nob = [0, self.nob[1] - self.nob[0]];
        }
        self.recompute_value();
        true
    }
}

/// `__ftol`/`cvttsd2si` of a double.
fn cvtt_d(v: f64) -> i32 {
    if v.is_nan() || !(-2147483648.0..2147483648.0).contains(&v) { i32::MIN } else { v as i32 }
}

/// A scroll bar button's place and state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Button {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    /// `CTFScrollButton`'s `CHandler_CommandButtonHighlight`: armed while the cursor is over it.
    pub armed: bool,
}

/// A scroll bar (`vgui::ScrollBar` / `vgui::ScrollBar2`): a slider between two buttons.
#[derive(Debug, Clone, PartialEq)]
pub struct ScrollBar {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub visible: bool,
    pub slider: Slider,
    pub buttons: [Button; 2],
    /// `_buttonPressedScrollValue` (15 from both constructors).
    pub step: i32,
}

impl ScrollBar {
    fn new(kind: Kind, x: i32, y: i32, w: i32, h: i32, vertical: bool) -> ScrollBar {
        let slider = if vertical { Slider::new(kind, 0, w, w, h - w * 2, true) } else { Slider::new(kind, h, 0, w - h * 2, h, false) };
        let mut b = ScrollBar { x, y, w, h, visible: true, slider, buttons: [Button::default(); 2], step: 15 };
        b.validate();
        b
    }

    pub fn set_bounds(&mut self, x: i32, y: i32, w: i32, h: i32) {
        (self.x, self.y) = (x, y);
        self.set_size(w, h);
    }

    /// `setSize` (`vgui.so` `0x34d80`, `ScrollBar2::setSize` `0x100159e0`): the buttons at the
    /// ends, the slider between them.
    pub fn set_size(&mut self, w: i32, h: i32) {
        (self.w, self.h) = (w, h);
        if self.slider.vertical {
            self.slider.set_bounds(0, w, w, h - w * 2);
            self.buttons[0] = Button { x: 0, y: 0, w, h: w, ..self.buttons[0] };
            self.buttons[1] = Button { x: 0, y: h - w, w, h: w, ..self.buttons[1] };
        } else {
            self.slider.set_bounds(h, 0, w - h * 2, h);
            self.buttons[0] = Button { x: 0, y: 0, w: h, h, ..self.buttons[0] };
            self.buttons[1] = Button { x: w - h, y: 0, w: h, h, ..self.buttons[1] };
        }
    }

    /// `validate` (`vgui.so` `0x34b50`, `0x10016330`): the slider's button offset is the buttons'
    /// length, then `setSize` again.
    pub fn validate(&mut self) {
        let len = |b: &Button| if self.slider.vertical { b.h } else { b.w };
        self.slider.button_offset = len(&self.buttons[0]) + len(&self.buttons[1]);
        self.set_size(self.w, self.h);
    }

    /// `setSlider`: a new slider, then `validate`.
    fn set_slider(&mut self, s: Slider) {
        self.slider = s;
        self.validate();
    }

    /// `doButtonPressed`: up (0) or down (1) by the step; true if the value changed.
    pub fn do_button_pressed(&mut self, i: usize) -> bool {
        let v = self.slider.value + if i == 0 { -self.step } else { self.step };
        self.slider.set_value(v)
    }

    fn panel(&self, parent: &Panel) -> Panel {
        parent.child(self.x, self.y, self.w, self.h)
    }

    fn slider_panel(&self, parent: &Panel) -> Panel {
        self.panel(parent).child(self.slider.x, self.slider.y, self.slider.w, self.slider.h)
    }

    fn button_panel(&self, parent: &Panel, i: usize) -> Panel {
        let b = &self.buttons[i];
        self.panel(parent).child(b.x, b.y, b.w, b.h)
    }

    /// The button the screen point is over, if the bar is shown.
    fn button_at(&self, parent: &Panel, x: i32, y: i32) -> Option<usize> {
        if !self.visible {
            return None;
        }
        (0..2).find(|&i| within_clip(&self.button_panel(parent, i), x, y))
    }

    /// A mouse press: a slider press (drag) when over the slider. True if the bar took it.
    fn press(&mut self, parent: &Panel, x: i32, y: i32) -> bool {
        if !self.visible {
            return false;
        }
        let sp = self.slider_panel(parent);
        if within_clip(&sp, x, y) {
            self.slider.press(x - sp.x, y - sp.y);
            return true;
        }
        self.button_at(parent, x, y).is_some() || within_clip(&self.panel(parent), x, y)
    }

    /// The cursor moved: a dragging slider follows it (true if its value signal fired).
    fn drag(&mut self, parent: &Panel, x: i32, y: i32) -> bool {
        let sp = self.slider_panel(parent);
        self.slider.drag(x - sp.x, y - sp.y)
    }

    /// A release: the dragging slider lets go (the release is its alone, it captured the mouse),
    /// else a left release over a button presses it. Returns (taken, value changed).
    fn release(&mut self, parent: &Panel, x: i32, y: i32, left: bool) -> (bool, bool) {
        if self.slider.dragging {
            self.slider.dragging = false;
            return (true, false);
        }
        match self.button_at(parent, x, y) {
            Some(i) => (true, left && self.do_button_pressed(i)),
            None => (false, false),
        }
    }
}

/// Whether a screen point is inside a panel's clip rectangle (VGUI finds the panel under the
/// cursor among what's drawn).
fn within_clip(p: &Panel, x: i32, y: i32) -> bool {
    !p.clip.is_empty() && x >= p.clip.x0 && x < p.clip.x1 && y >= p.clip.y0 && y < p.clip.y1
}

/// `CTFScrollPanel`: VGUI1's `ScrollPanel` with TS's buttons and sliders. Its client panel holds
/// one child, the text panel at (0, 0), of size `content`.
#[derive(Debug, Clone, PartialEq)]
pub struct ScrollPanel {
    /// Position in the parent panel, and size.
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    /// `clientClip`'s size.
    pub clip: (i32, i32),
    /// The client panel: position (minus the scroll values) and size.
    pub client: (i32, i32, i32, i32),
    /// The text panel in the client, if any (`None` before one is added).
    pub content: Option<(i32, i32)>,
    pub hbar: ScrollBar,
    pub vbar: ScrollBar,
    /// `setScrollBarAutoVisible`: horizontal, vertical.
    pub auto: [bool; 2],
}

impl ScrollPanel {
    /// `CTFScrollPanel::CTFScrollPanel` (`0x1005a860`): `ScrollPanel(x, y, w, h)` (`vgui.so`
    /// `0x35c80`: the client clip 16 pixels short each way, both bars hidden and automatic, then
    /// `validate`), then each bar gets two 16×16 `CTFScrollButton`s and a new `CTFSlider` (each
    /// `setButton`/`setSlider` validates the bar).
    pub fn ctf(x: i32, y: i32, w: i32, h: i32) -> ScrollPanel {
        let mut hbar = ScrollBar::new(Kind::Vgui, 0, h - 16, w - 16, 16, false);
        let mut vbar = ScrollBar::new(Kind::Vgui, w - 16, 0, 16, h - 16, true);
        hbar.visible = false;
        vbar.visible = false;
        let mut p =
            ScrollPanel { x, y, w, h, clip: (w - 16, h - 16), client: (0, 0, w * 2, h * 2), content: None, hbar, vbar, auto: [true; 2] };
        p.validate();
        for (bar, slider) in [
            (&mut p.vbar, Slider::new(Kind::Vgui, 0, w - 1, w, h - w * 2 + 2, true)),
            (&mut p.hbar, Slider::new(Kind::Vgui, h, 0, w - h * 2, h, false)),
        ] {
            for i in 0..2 {
                bar.buttons[i] = Button { x: 0, y: 0, w: 16, h: 16, armed: false };
                bar.validate();
            }
            bar.set_slider(slider);
        }
        p
    }

    /// `setSize` (`vgui.so` `0x35ac0`): each automatic bar shows unless its slider has the full
    /// range (horizontal first), a shown bar takes its width or height from the client clip, the
    /// bars are placed along the clip's edges, and the client size is recomputed.
    pub fn set_size(&mut self, w: i32, h: i32) {
        (self.w, self.h) = (w, h);
        let (mut w, mut h) = (w, h);
        if self.auto[0] {
            self.hbar.visible = !self.hbar.slider.has_full_range();
        }
        if self.auto[1] {
            self.vbar.visible = !self.vbar.slider.has_full_range();
        }
        if self.vbar.visible {
            w -= self.vbar.w;
        }
        if self.hbar.visible {
            h -= self.hbar.h;
        }
        let vw = self.vbar.w;
        self.vbar.set_bounds(w, 0, vw, h);
        let hh = self.hbar.h;
        self.hbar.set_bounds(0, h, w, hh);
        self.clip = (w, h);
        self.recompute_client_size();
    }

    /// `recomputeClientSize` (`vgui.so` `0x35810`): the client is as large as its visible
    /// children reach; each bar's range is what the client exceeds the clip by, its window the
    /// client's size.
    fn recompute_client_size(&mut self) {
        let (cw, ch) = self.content.unwrap_or((0, 0));
        (self.client.2, self.client.3) = (cw.max(0), ch.max(0));
        self.hbar.slider.set_range(0, self.client.2 - self.clip.0);
        self.hbar.slider.window = self.client.2;
        self.vbar.slider.set_range(0, self.client.3 - self.clip.1);
        self.vbar.slider.window = self.client.3;
    }

    /// `validate` (`vgui.so` `0x35a00`): both range windows on, then `setSize` four times.
    pub fn validate(&mut self) {
        self.hbar.slider.window_enabled = true;
        self.vbar.slider.window_enabled = true;
        for _ in 0..4 {
            self.set_size(self.w, self.h);
        }
    }

    /// `setScrollBarVisible`, then `validate`.
    pub fn set_scroll_bar_visible(&mut self, h: bool, v: bool) {
        self.hbar.visible = h;
        self.vbar.visible = v;
        self.validate();
    }

    /// `setScrollBarAutoVisible`, then `validate`.
    pub fn set_scroll_bar_auto_visible(&mut self, h: bool, v: bool) {
        self.auto = [h, v];
        self.validate();
    }

    /// `recomputeScroll`, on either bar's value signal: the client moves by the values.
    fn recompute_scroll(&mut self) {
        (self.client.0, self.client.1) = (-self.hbar.slider.value, -self.vbar.slider.value);
    }

    pub fn panel(&self, parent: &Panel) -> Panel {
        parent.child(self.x, self.y, self.w, self.h)
    }

    /// The text panel in the client, clipped to the client clip.
    pub fn content_panel(&self, parent: &Panel) -> Option<Panel> {
        let (w, h) = self.content?;
        let clip = self.panel(parent).child(0, 0, self.clip.0, self.clip.1);
        Some(clip.child(self.client.0, self.client.1, self.client.2, self.client.3).child(0, 0, w, h))
    }

    /// The cursor moved: the scroll buttons arm under it and a dragging slider follows it.
    pub fn cursor_moved(&mut self, parent: &Panel, x: i32, y: i32) {
        let sp = self.panel(parent);
        for bar in [&mut self.hbar, &mut self.vbar] {
            let over = bar.button_at(&sp, x, y);
            for (i, b) in bar.buttons.iter_mut().enumerate() {
                b.armed = over == Some(i);
            }
        }
        let fired = self.hbar.drag(&sp, x, y) | self.vbar.drag(&sp, x, y);
        if fired {
            self.recompute_scroll();
        }
    }

    /// A mouse press at (`x`, `y`); true if a scroll bar took it.
    pub fn mouse_pressed(&mut self, parent: &Panel, x: i32, y: i32) -> bool {
        let sp = self.panel(parent);
        self.hbar.press(&sp, x, y) || self.vbar.press(&sp, x, y)
    }

    /// A mouse release at (`x`, `y`) (`left`: the left button); true if a scroll bar took it.
    pub fn mouse_released(&mut self, parent: &Panel, x: i32, y: i32, left: bool) -> bool {
        let sp = self.panel(parent);
        let (h, hc) = self.hbar.release(&sp, x, y, left);
        let (v, vc) = if h { (false, false) } else { self.vbar.release(&sp, x, y, left) };
        if hc || vc {
            self.recompute_scroll();
        }
        h || v
    }

    /// Paints the bars (after the client clip, in the scroll panel's child order): each shown
    /// bar's buttons, then its slider. `arrows` are the `CTFScrollButton` images: up, down, left,
    /// right.
    pub fn paint_bars(&self, s: &mut dyn Surface, parent: &Panel, vs: &ViewportScheme, arrows: &[Option<Rc<Image>>; 4]) {
        let sp = self.panel(parent);
        for (bar, imgs) in [(&self.hbar, [&arrows[2], &arrows[3]]), (&self.vbar, [&arrows[0], &arrows[1]])] {
            if !bar.visible {
                continue;
            }
            for (i, img) in imgs.iter().enumerate() {
                // `CTFScrollButton::paint`: the arrow in white, half transparent unless armed, at
                // the image's own position (0, 0): `Label::paint`, which would align it, isn't
                // called. Its background paints nothing.
                if let Some(img) = img {
                    let a = if bar.buttons[i].armed { 0 } else { 128 };
                    bar.button_panel(&sp, i).image(s, img, 0, 0, Color::new(255, 255, 255, a));
                }
            }
            ctf_slider_paint(s, &bar.slider_panel(&sp), &bar.slider, vs);
        }
    }
}

/// `CTFSlider::paintBackground` (`0x1005a750`): an `sc_secondary1` outline round the slider, and
/// the nob filled in `sc_primary2` and outlined in `sc_primary1`.
fn ctf_slider_paint(s: &mut dyn Surface, p: &Panel, sl: &Slider, vs: &ViewportScheme) {
    let (w, h) = p.paint_size();
    p.outlined_rect(s, 0, 0, w, h, vs.secondary1);
    let [a, b] = sl.nob;
    if sl.vertical {
        p.fill_rect(s, 0, a, w, b, vs.primary2);
        p.outlined_rect(s, 0, a, w, b, vs.primary1);
    } else {
        p.fill_rect(s, a, 0, b, h, vs.primary2);
        p.outlined_rect(s, a, 0, b, h, vs.primary1);
    }
}

/// The `CTFScrollButton` arrows: `LoadTGAForRes` (`0x1005a070`) of `arrowup`, `arrowdn`, `arrowlt`
/// and `arrowrt` (`0x100964a8`), `gfx/vgui/<res>_<name>.tga` with `<res>` 640, or 320 below 640
/// wide (`0x1005a010`), through `vgui_LoadTGA`. A missing file leaves the button blank.
pub fn ctf_arrows(screen_w: i32, load: &dyn Fn(&str) -> Option<Rc<Image>>) -> [Option<Rc<Image>>; 4] {
    let res = if screen_w < 640 { 320 } else { 640 };
    ["arrowup", "arrowdn", "arrowlt", "arrowrt"].map(|n| load(&format!("gfx/vgui/{res}_{n}.tga")))
}

/// The scoreboard's `CListBox` scrolling: `m_ItemOffset` and its `ScrollBar2`, whose `Slider2`
/// has a 50-pixel range window and whose buttons step by 1.
#[derive(Debug, Clone, PartialEq)]
pub struct ListBox {
    pub offset: i32,
    /// `m_iScrollMax` (-1 until `SetScrollRange`).
    pub scroll_max: i32,
    pub bar: ScrollBar,
}

impl Default for ListBox {
    /// `CListBox::CListBox` (`0x1005b490`): `ScrollBar2(0, 0, 0, 0, true)`, then its slider is
    /// replaced by `Slider2(0, 0, 10, 40, true)` with window 50, and the step set to 1.
    fn default() -> ListBox {
        let mut bar = ScrollBar::new(Kind::Sdk2, 0, 0, 0, 0, true);
        let mut s = Slider::new(Kind::Sdk2, 0, 0, 10, 40, true);
        s.window = 50;
        s.window_enabled = true;
        bar.set_slider(s);
        bar.step = 1;
        ListBox { offset: 0, scroll_max: -1, bar }
    }
}

impl ListBox {
    /// `SetScrollRange` (`0x1005b860`).
    pub fn set_scroll_range(&mut self, max: i32) {
        self.scroll_max = max;
        self.bar.slider.set_range(0, max);
    }

    /// `InternalLayout` (`0x1005b730`) for a `w`×`h` list whose items are `heights` tall: the
    /// bar goes at the right 15 pixels; items before the offset and from `GetScrollMax` on are
    /// hidden, the rest stack from the top at their own height. The bar shows when an item is
    /// scrolled off the top or the stack runs past the bottom. Returns each item's y (`None` if
    /// hidden).
    pub fn layout(&mut self, w: i32, h: i32, heights: &[i32]) -> Vec<Option<i32>> {
        self.bar.set_bounds(w - 15, 0, 15, h);
        let max = if self.scroll_max < 0 { heights.len() as i32 - 1 } else { self.scroll_max };
        let (mut need, mut y) = (false, 0);
        let mut out = Vec::with_capacity(heights.len());
        for (i, &t) in heights.iter().enumerate() {
            let i = i as i32;
            if i < self.offset {
                need = true;
                out.push(None);
            } else if i >= max {
                out.push(None);
            } else {
                out.push(Some(y));
                y += t;
                if y > h {
                    need = true;
                }
            }
        }
        self.bar.visible = need;
        out
    }

    /// `setPixelScroll` (`0x1005b700`), the bar's value signal: the offset follows the value.
    fn value_changed(&mut self) {
        self.offset = self.bar.slider.value;
    }

    /// The cursor moved (a dragging slider follows it).
    pub fn cursor_moved(&mut self, list: &Panel, x: i32, y: i32) {
        if self.bar.drag(list, x, y) {
            self.value_changed();
        }
    }

    /// A press; true if the scroll bar took it.
    pub fn mouse_pressed(&mut self, list: &Panel, x: i32, y: i32) -> bool {
        self.bar.press(list, x, y)
    }

    /// A release; true if the scroll bar took it.
    pub fn mouse_released(&mut self, list: &Panel, x: i32, y: i32, left: bool) -> bool {
        let (taken, changed) = self.bar.release(list, x, y, left);
        if changed {
            self.value_changed();
        }
        taken
    }

    /// Paints the shown bar: the `ScrollBarButton`s (a grey `LineBorder`, a black background and
    /// the arrow in (140, 140, 140) centred, as `Label::paint` aligns an image in a text-less
    /// button) and the `Slider2` (`0x10016d30`): a dark grey track, a black nob with a grey frame
    /// whose bottom line sits just below it. `arrows` are `gfx/vgui/arrowup.tga` and
    /// `arrowdown.tga`.
    pub fn paint(&self, s: &mut dyn Surface, list: &Panel, arrows: &[Option<Rc<Image>>; 2]) {
        if !self.bar.visible {
            return;
        }
        for (i, img) in arrows.iter().enumerate() {
            let p = self.bar.button_panel(list, i).bordered();
            p.line_border(s, Color::new(60, 60, 60, 0));
            let (w, h) = p.paint_size();
            p.fill_rect(s, 0, 0, w, h, Color::new(0, 0, 0, 0));
            if let Some(img) = img {
                p.image(s, img, w / 2 - img.wide / 2, h / 2 - img.tall / 2, Color::new(140, 140, 140, 0));
            }
        }
        let p = self.bar.slider_panel(list);
        let (w, h) = p.paint_size();
        let [a, b] = self.bar.slider.nob;
        let grey = Color::new(60, 60, 60, 0);
        p.fill_rect(s, 0, 0, w, h, Color::new(40, 40, 40, 0));
        p.fill_rect(s, 0, a, w, b, Color::new(0, 0, 0, 0));
        p.fill_rect(s, 0, a, w, a + 1, grey);
        p.fill_rect(s, 0, b, w, b + 1, grey);
        p.fill_rect(s, 0, a + 1, 1, b, grey);
        p.fill_rect(s, w - 1, a + 1, w, b, grey);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hud::vgui::tests::Record;

    #[test]
    fn vgui_slider_maths() {
        let mut s = Slider::new(Kind::Vgui, 0, 0, 16, 100, true);
        assert_eq!(s.nob, [0, 33], "a disabled window is the span (299): 100 / 299 * 100 pixels");
        s.window_enabled = true;
        s.set_range(0, 100);
        s.window = 200;
        s.set_size(16, 100);
        assert_eq!(s.nob, [0, 50], "the nob is size / window * size");
        assert!(s.set_value(50) && s.nob == [25, 75]);
        assert!(s.set_value(500) && s.value == 100, "clamped to the range");
        assert_eq!(s.nob, [50, 100]);
        // Dragging: the nob follows the cursor, clamped to the slider; the value follows the nob.
        s.set_value(0);
        s.press(5, 10);
        assert!(s.drag(5, 30));
        assert_eq!((s.nob, s.value), ([20, 70], 40));
        s.drag(5, 500);
        assert_eq!((s.nob, s.value), ([50, 100], 100));
        s.drag(5, -500);
        assert_eq!((s.nob, s.value), ([0, 50], 0));
        // Full range: the window fits the slider and its buttons; an empty window never does.
        s.button_offset = 100;
        assert!(s.has_full_range());
        s.window = 0;
        assert!(!s.has_full_range());
        // An empty span: 0/0 truncates to 0x80000000, as the x87 does.
        let mut e = Slider::new(Kind::Vgui, 0, 0, 16, 100, true);
        e.window_enabled = true;
        e.window = 100;
        e.set_range(0, 0);
        e.set_size(16, 100);
        assert_eq!(e.nob, [i32::MIN, i32::MIN]);
        assert_eq!(e.range, [0, 0]);
        e.set_range(5, -3);
        assert_eq!(e.range, [5, 5], "the maximum is raised to the minimum");
    }

    #[test]
    fn slider2_maths() {
        let mut s = Slider::new(Kind::Sdk2, 0, 15, 15, 100, true);
        s.window = 50;
        s.window_enabled = true;
        s.set_range(0, 10);
        s.set_value(5);
        assert_eq!(s.nob, [25, 75], "a 50-pixel nob over the 50 free pixels");
        s.set_value(10);
        assert_eq!(s.nob, [50, 100]);
        s.press(0, 60);
        s.drag(0, 40);
        assert_eq!((s.nob, s.value), ([30, 80], 6));
    }

    #[test]
    fn ctf_scroll_panel_layout() {
        // A 200×100 panel holding a 150×300 text: only the vertical bar shows.
        let mut p = ScrollPanel::ctf(10, 20, 200, 100);
        assert!(!p.hbar.visible && !p.vbar.visible || p.content.is_none());
        p.content = Some((150, 300));
        p.validate();
        assert!(p.vbar.visible && !p.hbar.visible);
        assert_eq!(p.clip, (184, 100));
        assert_eq!((p.vbar.x, p.vbar.y, p.vbar.w, p.vbar.h), (184, 0, 16, 100));
        assert_eq!((p.vbar.buttons[1].y, p.vbar.slider.y, p.vbar.slider.h), (84, 16, 68));
        assert_eq!((p.vbar.slider.range, p.vbar.slider.window), ([0, 200], 300));
        // 68 * 68 / 300 = 15.4 pixels of nob at the top.
        assert_eq!(p.vbar.slider.nob, [0, 15]);
        // A wide text brings the horizontal bar too, which shortens the vertical one.
        p.content = Some((400, 300));
        p.validate();
        assert!(p.vbar.visible && p.hbar.visible);
        assert_eq!((p.clip, p.vbar.h, p.hbar.y, p.hbar.w), ((184, 84), 84, 84, 184));
        // Fits: no bars.
        p.content = Some((100, 50));
        p.validate();
        assert!(!p.vbar.visible && !p.hbar.visible);
        assert_eq!(p.clip, (200, 100));
    }

    #[test]
    fn ctf_scroll_panel_input() {
        let root = Panel::root(640, 480);
        let mut p = ScrollPanel::ctf(10, 20, 200, 100);
        p.content = Some((150, 300));
        p.validate();
        // The down button: a left release steps by 15.
        let (bx, by) = (10 + 184 + 4, 20 + 84 + 4);
        p.cursor_moved(&root, bx, by);
        assert!(p.vbar.buttons[1].armed && !p.vbar.buttons[0].armed);
        assert!(p.mouse_released(&root, bx, by, true));
        assert_eq!((p.vbar.slider.value, p.client.1), (15, -15));
        assert!(p.mouse_released(&root, bx, by, false), "the right button is taken but does nothing");
        assert_eq!(p.vbar.slider.value, 15);
        // Up past the top clamps to 0.
        p.mouse_released(&root, 10 + 190, 20 + 2, true);
        p.mouse_released(&root, 10 + 190, 20 + 2, true);
        assert_eq!((p.vbar.slider.value, p.client.1), (0, 0));
        // Drag the nob down by 34 pixels: half the free track.
        let ny = 20 + 16 + 2;
        assert!(p.mouse_pressed(&root, 10 + 190, ny));
        p.cursor_moved(&root, 10 + 190, ny + 26);
        assert_eq!(p.vbar.slider.nob, [26, 41]);
        assert_eq!((p.vbar.slider.value, p.client.1), (99, -99));
        assert!(p.mouse_released(&root, 0, 0, true), "the slider keeps the mouse until the release");
        assert!(!p.mouse_released(&root, 0, 0, true));
        assert!(!p.mouse_pressed(&root, 0, 0));
        // The text panel moves with the client, clipped to the client clip.
        let c = p.content_panel(&root).unwrap();
        assert_eq!((c.x, c.y, c.clip.y0, c.clip.y1), (10, 20 - 99, 20, 120));
    }

    #[test]
    fn ctf_bars_paint() {
        let root = Panel::root(640, 480);
        let mut p = ScrollPanel::ctf(0, 0, 200, 100);
        p.content = Some((150, 300));
        p.validate();
        let vs = ViewportScheme {
            primary1: Color::new(1, 0, 0, 0),
            primary2: Color::new(2, 0, 0, 0),
            primary3: Color::new(3, 0, 0, 0),
            secondary1: Color::new(4, 0, 0, 0),
            secondary2: Color::new(5, 0, 0, 0),
        };
        let mut s = Record::default();
        p.paint_bars(&mut s, &root, &vs, &[None, None, None, None]);
        // The slider outline (4 fills) at (184, 16) 16×68, the nob fill and its outline.
        assert_eq!(s.0[0], (184, 16, 16, 1, [4, 0, 0], 255));
        assert_eq!(s.0[4], (184, 16, 16, 15, [2, 0, 0], 255));
        assert_eq!(s.0.len(), 9);
    }

    #[test]
    fn list_box_layout_and_scroll() {
        let mut l = ListBox::default();
        assert_eq!((l.bar.step, l.bar.slider.window), (1, 50));
        l.set_scroll_range(4);
        let heights = [20; 74];
        // 4 rows of 20 in 100 pixels: all shown, no bar.
        let ys = l.layout(200, 100, &heights);
        assert_eq!(&ys[..5], &[Some(0), Some(20), Some(40), Some(60), None]);
        assert!(!l.bar.visible);
        assert_eq!((l.bar.x, l.bar.w, l.bar.slider.y, l.bar.slider.h), (185, 15, 15, 70));
        // Six rows overflow: the bar shows.
        l.set_scroll_range(6);
        let ys = l.layout(200, 100, &heights);
        assert_eq!(ys[5], Some(100));
        assert!(l.bar.visible);
        // The down button scrolls one row.
        let list = Panel::root(640, 480).child(0, 0, 200, 100);
        assert!(l.mouse_released(&list, 190, 95, true));
        assert_eq!(l.offset, 1);
        let ys = l.layout(200, 100, &heights);
        assert_eq!(&ys[..3], &[None, Some(0), Some(20)]);
        assert!(l.bar.visible, "a row scrolled off the top keeps the bar");
        assert_eq!(l.bar.slider.nob, [3, 53], "value 1 of 6 over 20 free pixels");
        let mut s = Record::default();
        l.paint(&mut s, &list, &[None, None]);
        assert_eq!(s.0[0], (185, 0, 15, 1, [60, 60, 60], 255), "the button's line border");
    }
}
