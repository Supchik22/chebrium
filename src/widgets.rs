//! Widgets the programs used to draw each by hand, gathered in one place so they look and behave the same everywhere:
//!
//! * [`scroll_area`] / [`Scrolled`] and [`scrollbar`]: a list or page that scrolls with the wheel or the touchpad, glides,
//!   and shows a thin bar with a thumb that can be dragged (hidden when everything fits).
//! * [`list_row`]: the background of a row in a list: the accent under a selected row, a soft wash under the pointer.
//! * [`round_button`] with [`Round`]: a round icon button with a hover wash (toolbars, the controls over a picture).
//! * [`fit`] / [`fit_start`]: text cut short with an ellipsis so it fits a width.
//! * [`facts_panel`]: a glass card of name / value lines (a photo's or a film's details).
//!
//! Like the rest of [`crate::ui`] these are immediate mode: state stays in the program (a scroll offset is an `f32`
//! it keeps), the widgets only remember small things for their motion, keyed by where they are.

use crate::draw::Align;
use crate::ui::{inside, Cursor, Ui};
use std::ops::Range;

// ------------------------------------------------------------------------------------------------------------ text

/// `text`, cut short with an ellipsis at the end so it is no wider than `max` (shown as it is, not translated).
pub fn fit(ui: &mut Ui<'_>, text: &str, size: f32, bold: bool, max: f32) -> String {
    if ui.measure_raw(text, size, bold) <= max {
        return text.to_string();
    }
    let mut s = text.to_string();
    while !s.is_empty() && ui.measure_raw(&format!("{s}…"), size, bold) > max {
        s.pop();
    }
    format!("{s}…")
}

/// Like [`fit`], with the ellipsis at the start: for paths, where the end is what tells them apart.
pub fn fit_start(ui: &mut Ui<'_>, text: &str, size: f32, bold: bool, max: f32) -> String {
    if ui.measure_raw(text, size, bold) <= max {
        return text.to_string();
    }
    let mut s = text.to_string();
    while !s.is_empty() && ui.measure_raw(&format!("…{s}"), size, bold) > max {
        s.remove(0);
    }
    format!("…{s}")
}

// ------------------------------------------------------------------------------------------------- round buttons

/// How a [`round_button`] looks. Start from [`Round::ink`] (on the program's own surface) or [`Round::light`] (white,
/// over a picture or a film) and change what differs.
#[derive(Clone, Copy, Debug)]
pub struct Round {
    /// the circle's width
    pub d: f32,
    /// the glyph's size
    pub glyph: f32,
    /// the glyph's and the hover wash's colour; `None` = the theme's ink
    pub colour: Option<[f32; 3]>,
    /// how strong the wash under the pointer is
    pub wash: f32,
    /// the whole button fades with this (controls that hide when the pointer rests); below 0.3 it takes no presses
    pub alpha: f32,
    /// a disabled button is dimmed and takes no presses
    pub enabled: bool,
    /// a button that stands for something switched on sits on the accent
    pub on: bool,
}

impl Round {
    /// A button on the program's surface, in the theme's ink.
    pub fn ink(d: f32) -> Round {
        Round { d, glyph: (d * 0.56).round(), colour: None, wash: 0.10, alpha: 1.0, enabled: true, on: false }
    }

    /// A white button for over a picture or a film.
    pub fn light(d: f32) -> Round {
        Round { colour: Some([1.0; 3]), wash: 0.15, ..Round::ink(d) }
    }

    pub fn glyph(self, size: f32) -> Round {
        Round { glyph: size, ..self }
    }

    pub fn alpha(self, alpha: f32) -> Round {
        Round { alpha, ..self }
    }

    pub fn enabled(self, enabled: bool) -> Round {
        Round { enabled, ..self }
    }

    pub fn on(self, on: bool) -> Round {
        Round { on, ..self }
    }
}

/// A round icon button centred on (`cx`, `cy`): a wash eases in under the pointer, `on` puts it on the accent. Returns
/// true when it was pressed.
pub fn round_button(ui: &mut Ui<'_>, cx: f32, cy: f32, icon: &str, look: Round) -> bool {
    let d = look.d;
    let r = [cx - d / 2.0, cy - d / 2.0, d, d];
    let live = look.enabled && look.alpha > 0.3;
    let hov = live && ui.hit(r);
    if hov {
        ui.want_cursor(Cursor::Pointer);
    }
    let key = ui.id() * 64;
    let wash = ui.ease(key, if hov { 1.0 } else { 0.0 }, 22.0);
    let colour = look.colour.unwrap_or(ui.ink);
    if look.on {
        ui.fill(r, d / 2.0, ui.accent, 0.32 * look.alpha);
    }
    if wash > 0.004 {
        ui.fill(r, d / 2.0, colour, look.wash * wash * look.alpha);
    }
    let glyph_a = if look.enabled { (0.85 + 0.15 * wash) * look.alpha } else { 0.28 * look.alpha };
    ui.glyph(cx, cy, look.glyph, icon, colour, glyph_a);
    ui.click && hov
}

// ------------------------------------------------------------------------------------------------------ seek bars

/// What a [`seek_bar`] saw this frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct Seek {
    /// the pointer is on the bar (or near it)
    pub hover: bool,
    /// the bar is pressed and being dragged; `value` follows the pointer. The frame it turns false is the release.
    pub dragging: bool,
    /// where the pointer is along the bar, 0..1 (for a tip with the time under it)
    pub at: f32,
}

/// A thin bar for over a picture or a film: a seek line, a volume. A light rail, `fill` (the accent when `None`) up to
/// `value` (0..1), and a round knob that grows in, with the rail, while the pointer is on it. A press anywhere on it
/// starts a drag and `value` follows the pointer. Everything fades with `alpha`, and below 0.3 it takes no presses
/// (controls that hide while a film plays). `ui.slider_bare` is the same thing with the desktop's groove and knob, for
/// a program's own surface.
pub fn seek_bar(ui: &mut Ui<'_>, r: [f32; 4], value: &mut f32, alpha: f32, fill: Option<[f32; 3]>) -> Seek {
    let key = key_of(r) ^ 0x0200_0000_0000_0000;
    let cy = r[1] + r[3] / 2.0;
    let hot = [r[0] - 8.0, cy - 14.0, r[2] + 16.0, 28.0];
    let hover = alpha > 0.3 && ui.hit(hot);
    let dragging = if alpha > 0.3 { ui.grab(key, hot) } else { ui.grabbed(key) };
    let at = ((ui.mouse[0] - r[0]) / r[2].max(1.0)).clamp(0.0, 1.0);
    if hover || dragging {
        ui.want_cursor(Cursor::Pointer);
    }
    if dragging && (at - *value).abs() > 1e-4 {
        *value = at;
        ui.changed = true;
    }
    let grown = ui.ease(key + 1, if hover || dragging { 1.0 } else { 0.0 }, 18.0);
    let h = r[3] + 4.0 * grown;
    let y = cy - h / 2.0;
    ui.fill([r[0], y, r[2], h], h / 2.0, [1.0; 3], 0.24 * alpha);
    let fw = (r[2] * value.clamp(0.0, 1.0)).max(0.0);
    if fw > 0.5 {
        ui.fill([r[0], y, fw, h], h / 2.0, fill.unwrap_or(ui.accent), 0.98 * alpha);
    }
    if grown > 0.05 {
        let d = r[3] * 2.0 + 2.0;
        let kx = r[0] + fw;
        ui.fill([kx - d / 2.0, cy - d / 2.0, d, d], d / 2.0, [1.0; 3], grown * alpha);
    }
    Seek { hover, dragging, at }
}

// ----------------------------------------------------------------------------------------------------- list rows

/// The background of a row in a list: the accent under a selected row, and a wash that eases in under the pointer.
/// `key` is the row's own (an index or an id with some high bits, so it does not clash with other eased values);
/// `hovered` is whether the pointer is on it (for a row in a [`scroll_area`], ask [`Scrolled::hit`]).
pub fn list_row(ui: &mut Ui<'_>, r: [f32; 4], radius: f32, key: u64, hovered: bool, selected: bool) {
    let wash = ui.ease(key, if hovered { 1.0 } else { 0.0 }, 22.0);
    if selected {
        ui.fill(r, radius, ui.accent, 0.28);
    } else if wash > 0.01 {
        ui.fill(r, radius, ui.ink, 0.07 * wash);
    }
}

// ----------------------------------------------------------------------------------------------------- facts panel

/// A glass card at (`x`, `y`), `w` wide, with one line per (name, value): the names (translated) in a faint column,
/// the values as they are, cut short to fit; a value that is a path (starts with '/') loses its start, not its end.
/// Returns the card's height.
pub fn facts_panel(ui: &mut Ui<'_>, x: f32, y: f32, w: f32, lines: &[(&str, String)]) -> f32 {
    let h = 26.0 + lines.len() as f32 * 24.0;
    ui.glass([x, y, w, h], 14.0, 1.15);
    let ink = ui.ink;
    for (i, (k, v)) in lines.iter().enumerate() {
        let ly = y + 26.0 + i as f32 * 24.0;
        ui.text(x + 16.0, ly, k, 12.0, true, ink, 0.5, Align::Left);
        let v = if v.starts_with('/') { fit_start(ui, v, 12.0, false, w - 100.0) } else { fit(ui, v, 12.0, false, w - 100.0) };
        ui.text_raw(x + 88.0, ly, &v, 12.0, false, ink, 0.92, Align::Left);
    }
    h
}

// ------------------------------------------------------------------------------------------------------ scrolling

/// How wide the strip along a scroll area's right edge is where the bar lives (and takes presses).
pub const BAR_ZONE: f32 = 12.0;

/// The strip of `rect` where its scrollbar goes: along the right edge, a little in from the ends.
pub fn bar_track(rect: [f32; 4]) -> [f32; 4] {
    [rect[0] + rect[2] - BAR_ZONE, rect[1] + 3.0, BAR_ZONE, (rect[3] - 6.0).max(0.0)]
}

/// Move `offset` the least so that the span `top..bottom` (content coordinates) is in a view `viewport` tall: the
/// keyboard's cursor row stays in sight.
pub fn reveal(offset: &mut f32, top: f32, bottom: f32, viewport: f32) {
    if top < *offset {
        *offset = top;
    } else if bottom > *offset + viewport {
        *offset = bottom - viewport;
    }
}

/// The keys a scroll area keeps its motion under: from where it is, so each area on a screen has its own, and high
/// enough not to meet the small numbers the widgets hand out.
fn key_of(r: [f32; 4]) -> u64 {
    0x5C40_0000_0000_0000 ^ ((r[0].round() as i64 as u64 & 0xF_FFFF) << 24) ^ ((r[1].round() as i64 as u64 & 0xF_FFFF) << 4)
}

/// The thumb's length and its distance from the top of a `track_h` tall track.
fn thumb(track_h: f32, content: f32, viewport: f32, offset: f32) -> (f32, f32) {
    let max = (content - viewport).max(0.0);
    let len = (track_h * viewport / content.max(1.0)).clamp(28.0f32.min(track_h), track_h);
    let pos = if max > 0.0 { (track_h - len) * (offset / max).clamp(0.0, 1.0) } else { 0.0 };
    (len, pos)
}

/// The pointer on the bar: a press on the thumb drags it from where it was taken, a press beside it brings the thumb's
/// middle under the pointer and drags from there. A press on the bar is used up, so the content under it does not
/// take it. Returns (pointer on the bar, being dragged).
fn bar_input(ui: &mut Ui<'_>, key: u64, track: [f32; 4], content: f32, viewport: f32, offset: &mut f32) -> (bool, bool) {
    let max = (content - viewport).max(0.0);
    if max <= 0.0 || track[3] <= 0.0 {
        return (false, false);
    }
    let hot = ui.hit(track);
    let (len, pos) = thumb(track[3], content, viewport, *offset);
    if ui.click && hot {
        let ty = track[1] + pos;
        let anchor = if ui.mouse[1] >= ty && ui.mouse[1] <= ty + len { ui.mouse[1] - ty } else { len / 2.0 };
        ui.snap(key + 2, anchor);
    }
    let dragging = ui.grab(key + 1, track);
    if ui.click && hot {
        ui.click = false;
        ui.double = false;
    }
    if dragging {
        let anchor = ui.recall(key + 2).unwrap_or(len / 2.0);
        let room = (track[3] - len).max(1.0);
        *offset = ((ui.mouse[1] - anchor - track[1]) / room).clamp(0.0, 1.0) * max;
    }
    (hot, dragging)
}

/// The bar itself: a thin rounded thumb that widens and darkens while the pointer is on it or drags it.
fn bar_draw(ui: &mut Ui<'_>, key: u64, track: [f32; 4], content: f32, viewport: f32, shown: f32, hot: bool, dragging: bool) {
    if content - viewport <= 0.5 || track[3] <= 0.0 {
        return;
    }
    let lift = ui.ease(key + 3, if hot || dragging { 1.0 } else { 0.0 }, 18.0);
    let (len, pos) = thumb(track[3], content, viewport, shown);
    let bw = 4.0 + 3.0 * lift;
    let x = track[0] + track[2] - 3.0 - bw;
    let ink = ui.ink;
    if lift > 0.01 {
        ui.fill([x, track[1], bw, track[3]], bw / 2.0, ink, 0.06 * lift);
    }
    ui.fill([x, track[1] + pos, bw, len], bw / 2.0, ink, 0.26 + 0.22 * lift);
}

/// A scrollbar on its own, for a view that scrolls in its own way (a thread anchored at the bottom, a terminal's
/// history): `track` is the strip it lives in (see [`bar_track`]), `content` how tall everything is, `viewport` how
/// much of it shows, `offset` how far down the view is (0 = the top). Call it *before* drawing the content under the
/// strip so a press on the bar is not also taken by the content (or keep the content out of the strip). Returns
/// true while the thumb is dragged; `offset` is changed then.
pub fn scrollbar(ui: &mut Ui<'_>, track: [f32; 4], content: f32, viewport: f32, offset: &mut f32) -> bool {
    let key = key_of(track) ^ 0x0100_0000_0000_0000;
    let (hot, dragging) = bar_input(ui, key, track, content, viewport, offset);
    bar_draw(ui, key, track, content, viewport, *offset, hot, dragging);
    dragging
}

/// What a [`scroll_area`] shows this frame. Draw the content at [`Scrolled::y`] (or for the rows in
/// [`Scrolled::rows`]), then call [`Scrolled::end`].
#[must_use = "call `end` after the content: it closes the clip and draws the bar"]
pub struct Scrolled {
    /// the viewport
    pub rect: [f32; 4],
    /// how far down the content is shown this frame (it glides after the target the program keeps)
    pub offset: f32,
    /// the content's height and the furthest the offset goes
    pub content: f32,
    pub max: f32,
    key: u64,
    track: [f32; 4],
    bar_hot: bool,
    dragging: bool,
    /// the pointer as it was before `fence` hid it, put back by `end`
    fenced: Option<([f32; 2], bool, bool)>,
}

impl Scrolled {
    /// Make the widgets drawn inside deaf to a pointer that is outside the viewport or on the bar, until `end`: a
    /// settings page of sliders and switches that scrolls under a header must not take a press on the header. (A drag
    /// already going on keeps seeing the pointer, so a slider held while the pointer leaves the page does not jump.)
    pub fn fence(mut self, ui: &mut Ui<'_>) -> Scrolled {
        let away = !inside(ui.mouse, self.rect) || self.bar_hot || self.dragging;
        if away && self.fenced.is_none() {
            self.fenced = Some((ui.mouse, ui.click, ui.double));
            if !ui.held {
                ui.mouse = [-1000.0; 2];
            }
            ui.click = false;
            ui.double = false;
        }
        self
    }

    /// Where content `y` (0 = the top of the content) is on the screen.
    pub fn y(&self, content_y: f32) -> f32 {
        self.rect[1] - self.offset + content_y
    }

    /// The rows (of `n`, each `row_h` tall, from the top of the content) that are at least partly in view.
    pub fn rows(&self, row_h: f32, n: usize) -> Range<usize> {
        let first = ((self.offset / row_h).floor().max(0.0) as usize).min(n);
        let last = (((self.offset + self.rect[3]) / row_h).ceil() as usize + 1).min(n);
        first..last.max(first)
    }

    /// Is the pointer on `r`, inside the viewport and not on the bar? (What a row asks before it lights up.)
    pub fn hit(&self, ui: &Ui<'_>, r: [f32; 4]) -> bool {
        ui.hit(r) && inside(ui.mouse, self.rect) && !self.bar_hot && !self.dragging
    }

    /// Can the content move at all (is there more than fits)?
    pub fn scrolls(&self) -> bool {
        self.max > 0.0
    }

    /// Close the clip the area opened and draw its bar over the content.
    pub fn end(self, ui: &mut Ui<'_>) {
        ui.pop_clip();
        if let Some((mouse, click, double)) = self.fenced {
            ui.mouse = mouse;
            // a press on the bar stays used up
            ui.click = click && !self.bar_hot;
            ui.double = double && !self.bar_hot;
        }
        bar_draw(ui, self.key, self.track, self.content, self.rect[3], self.offset, self.bar_hot, self.dragging);
    }
}

/// A viewport `rect` over content `content` tall, scrolled `offset` down (the program keeps it; 0 = the top). Takes the
/// wheel and the touchpad while the pointer is over it, keeps `offset` in range, lets the shown offset glide after it,
/// takes presses on its bar, and clips what follows to `rect` until [`Scrolled::end`] (which draws the bar, hidden
/// when everything fits). The program may move `offset` itself (to the top for a new folder, [`reveal`] for the
/// keyboard) before the call.
///
/// ```ignore
/// let s = widgets::scroll_area(ui, list, n as f32 * ROW_H, &mut self.scroll);
/// for i in s.rows(ROW_H, n) {
///     let r = [list[0], s.y(i as f32 * ROW_H), list[2] - widgets::BAR_ZONE, ROW_H];
///     let hov = s.hit(ui, r);
///     widgets::list_row(ui, r, 8.0, 30_000 + i as u64, hov, i == self.selected);
///     ...
/// }
/// s.end(ui);
/// ```
pub fn scroll_area(ui: &mut Ui<'_>, rect: [f32; 4], content: f32, offset: &mut f32) -> Scrolled {
    let key = key_of(rect);
    let max = (content - rect[3]).max(0.0);
    // the wheel is used up only when it moved something, so an area around this one can still scroll when it fits
    if ui.scroll != 0.0 && max > 0.0 && ui.hit(rect) {
        *offset += ui.scroll;
        ui.scroll = 0.0;
    }
    *offset = offset.clamp(0.0, max);
    let track = bar_track(rect);
    let (bar_hot, dragging) = bar_input(ui, key, track, content, rect[3], offset);
    // a dragged thumb stays under the pointer; the wheel and jumps glide
    let shown = if dragging {
        ui.snap(key, *offset);
        *offset
    } else {
        ui.ease(key, *offset, 18.0)
    };
    ui.push_clip(rect);
    Scrolled { rect, offset: shown, content, max, key, track, bar_hot, dragging, fenced: None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::Draw;
    use crate::style::Style;
    use crate::ui::State;

    /// A painter that draws nothing: these tests are about what the widgets do with the pointer.
    struct Blank;
    impl Draw for Blank {
        fn fill(&mut self, _: [f32; 4], _: f32, _: [f32; 3], _: f32) {}
        fn grad(&mut self, _: [f32; 4], _: f32, _: [f32; 3], _: [f32; 3], _: f32) {}
        fn glyph(&mut self, _: f32, _: f32, _: f32, _: &str, _: [f32; 3], _: f32) {}
        fn text(&mut self, _: f32, _: f32, _: &str, _: f32, _: bool, _: [f32; 3], _: f32, _: Align) {}
    }

    const LIST: [f32; 4] = [0.0, 0.0, 300.0, 200.0];

    /// One frame of a 1000 px tall list in a 200 px view; returns (shown offset, the click reached the content, the
    /// wheel was left over).
    fn frame(st: &mut State, offset: &mut f32, content: f32) -> (f32, bool, bool) {
        let mut d = Blank;
        let dt = st.begin_frame();
        let mut ui = Ui::new(&mut d, st, Style::default(), dt);
        let area = scroll_area(&mut ui, LIST, content, offset);
        let shown = area.offset;
        let clicked = ui.click && area.hit(&ui, LIST);
        let wheel_left = ui.scroll != 0.0;
        area.end(&mut ui);
        st.end_frame(false);
        (shown, clicked, wheel_left)
    }

    #[test]
    fn the_wheel_scrolls_within_range() {
        let mut st = State::default();
        let mut off = 0.0;
        st.pointer_move([100.0, 100.0]);
        st.pointer_scroll(120.0);
        let (_, _, left) = frame(&mut st, &mut off, 1000.0);
        assert_eq!(off, 120.0);
        assert!(!left, "the area used the wheel");
        st.pointer_scroll(5000.0);
        frame(&mut st, &mut off, 1000.0);
        assert_eq!(off, 800.0, "no further than the end");
        // content that fits: nothing moves and the wheel is left for whatever is around
        let mut off = 0.0;
        st.pointer_scroll(50.0);
        let (_, _, left) = frame(&mut st, &mut off, 150.0);
        assert_eq!(off, 0.0);
        assert!(left);
    }

    #[test]
    fn the_thumb_drags_and_its_press_is_not_the_contents() {
        let mut st = State::default();
        let mut off = 0.0;
        // the thumb is at the top of the track at the right edge; take it 10 px below its top
        let track = bar_track(LIST);
        let x = track[0] + track[2] - 5.0;
        let grab = [x, track[1] + 10.0];
        st.pointer_move(grab);
        st.pointer_down(grab);
        let (_, clicked, _) = frame(&mut st, &mut off, 1000.0);
        assert!(!clicked, "a press on the bar is the bar's");
        assert_eq!(off, 0.0, "taking the thumb does not move it");
        // drag it down by half the room it has: the view goes half way
        let (len, _) = thumb(track[3], 1000.0, 200.0, 0.0);
        let room = track[3] - len;
        st.pointer_move([x + 40.0, grab[1] + room / 2.0]);
        let (shown, _, _) = frame(&mut st, &mut off, 1000.0);
        assert!((off - 400.0).abs() < 0.5, "{off}");
        assert_eq!(shown, off, "a dragged thumb does not lag");
        st.pointer_up();
        frame(&mut st, &mut off, 1000.0);
        // a press on the content is the content's
        st.pointer_move([50.0, 50.0]);
        st.pointer_down([50.0, 50.0]);
        let (_, clicked, _) = frame(&mut st, &mut off, 1000.0);
        assert!(clicked);
        st.pointer_up();
    }

    #[test]
    fn a_press_beside_the_thumb_jumps_there() {
        let mut st = State::default();
        let mut off = 0.0;
        let track = bar_track(LIST);
        let p = [track[0] + 6.0, track[1] + track[3] - 2.0];
        st.pointer_move(p);
        st.pointer_down(p);
        frame(&mut st, &mut off, 1000.0);
        assert!((off - 800.0).abs() < 0.5, "pressed at the bottom: {off}");
    }

    #[test]
    fn a_seek_bar_follows_a_drag_and_lets_go() {
        let mut st = State::default();
        let r = [100.0, 50.0, 200.0, 6.0];
        let mut v = 0.1;
        let step = |st: &mut State, v: &mut f32, alpha: f32| {
            let mut d = Blank;
            let dt = st.begin_frame();
            let mut ui = Ui::new(&mut d, st, Style::default(), dt);
            let s = seek_bar(&mut ui, r, v, alpha, None);
            st.end_frame(false);
            s
        };
        st.pointer_move([150.0, 53.0]);
        st.pointer_down([150.0, 53.0]);
        let s = step(&mut st, &mut v, 1.0);
        assert!(s.dragging && (v - 0.25).abs() < 1e-3);
        st.pointer_move([400.0, 90.0]);
        let s = step(&mut st, &mut v, 1.0);
        assert!(s.dragging && v == 1.0, "past the end it stays at the end");
        st.pointer_up();
        let s = step(&mut st, &mut v, 1.0);
        assert!(!s.dragging);
        // hidden controls take no presses
        st.pointer_move([150.0, 53.0]);
        st.pointer_down([150.0, 53.0]);
        let s = step(&mut st, &mut v, 0.1);
        assert!(!s.dragging && v == 1.0);
    }
}
