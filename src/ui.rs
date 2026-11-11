//! The immediate-mode widget kit.
//!
//! A program owns a [`State`] (pointer, drag, text editing, running animations) and each frame builds a [`Ui`]
//! over a painter, calls the widgets it wants (`ui.button(..)`, `ui.toggle(..)`, ...) and reads their results.
//! Nothing is retained between frames except what `State` holds, so layout is plain code.
//!
//! ```ignore
//! let dt = state.begin_frame();
//! let mut ui = Ui::new(&mut painter, &mut state, Style::default(), dt);
//! if ui.button([20.0, 20.0, 120.0, 32.0], "Apply", true) { apply(); }
//! let edit_seen = ui.edit_seen;
//! state.end_frame(edit_seen);
//! if state.animating() { request_another_frame(); }
//! ```

use crate::anim::Anims;
use crate::draw::{Align, Draw};
use crate::input::{Key, KeyMods};
use crate::style::Style;
use std::time::Instant;

pub fn inside(p: [f32; 2], r: [f32; 4]) -> bool {
    p[0] >= r[0] && p[0] <= r[0] + r[2] && p[1] >= r[1] && p[1] <= r[1] + r[3]
}

/// What lives between frames.
/// The pointer's shape over the thing it is on. Widgets ask for one while the pointer is on them (`Ui::want_cursor`); the
/// runtime hands it to the compositor, which draws it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Cursor {
    #[default]
    Default,
    Text,
    Pointer,
    Grab,
    Grabbing,
    Move,
    /// the sides of a column or a window: left-right arrows
    ResizeH,
    ResizeV,
    ResizeNwse,
    ResizeNesw,
    NotAllowed,
    Wait,
    Crosshair,
}

pub struct State {
    /// the shape the widgets of the last frame asked for
    pub cursor: Cursor,
    pub mouse: [f32; 2],
    pub held: bool,
    /// the pointer went down since the last frame
    pub click: bool,
    /// that press was the second of a double click
    pub double: bool,
    /// the modifier keys held right now (kept up to date by the host)
    pub mods: KeyMods,
    /// wheel movement since the last frame, in logical pixels (positive scrolls down)
    pub scroll: f32,
    last_click: Option<(Instant, [f32; 2])>,
    drag: Option<u64>,
    /// a text field being edited: (field key, text so far)
    edit: Option<(String, String)>,
    /// Enter (true) or Esc (false) was pressed; the field handles it on the next frame
    edit_end: Option<bool>,
    /// the caret (a character index into the text being edited) and the other end of the selection, if any
    edit_cur: usize,
    edit_sel: Option<usize>,
    /// text a Ctrl+C / Ctrl+X in a field wants on the clipboard, and a Ctrl+V that wants the clipboard's text: the runtime
    /// carries both out
    pub clip_out: Option<String>,
    pub want_paste: bool,
    pub anims: Anims,
    last: Option<Instant>,
}

impl Default for State {
    fn default() -> State {
        State { cursor: Cursor::Default, mouse: [-1000.0; 2], held: false, click: false, double: false, mods: KeyMods::default(), scroll: 0.0, last_click: None, drag: None, edit: None, edit_end: None, edit_cur: 0, edit_sel: None, clip_out: None, want_paste: false, anims: Anims::default(), last: None }
    }
}

impl State {
    pub fn pointer_move(&mut self, p: [f32; 2]) {
        self.mouse = p;
    }

    pub fn pointer_down(&mut self, p: [f32; 2]) {
        self.mouse = p;
        self.held = true;
        self.click = true;
        let now = Instant::now();
        self.double = self.last_click.is_some_and(|(t, q)| now.duration_since(t).as_millis() < 400 && (q[0] - p[0]).abs() < 6.0 && (q[1] - p[1]).abs() < 6.0);
        // a third quick press starts a new pair
        self.last_click = if self.double { None } else { Some((now, p)) };
    }

    pub fn pointer_scroll(&mut self, dy: f32) {
        self.scroll += dy;
    }

    /// Start editing the text field `key` with `text` already in it (rename on F2, say).
    pub fn begin_edit(&mut self, key: &str, text: &str) {
        self.edit = Some((key.to_string(), text.to_string()));
        // the whole text is selected: typing replaces it (a rename)
        self.edit_cur = text.chars().count();
        self.edit_sel = Some(0);
    }

    pub fn pointer_up(&mut self) {
        self.held = false;
        self.drag = None;
    }

    /// A key press while a text field is being edited. True when it was used.
    pub fn key(&mut self, k: Key, mods: KeyMods) -> bool {
        let Some((_, buf)) = self.edit.as_mut() else { return false };
        let len = buf.chars().count();
        self.edit_cur = self.edit_cur.min(len);
        let byte = |buf: &str, i: usize| buf.char_indices().nth(i).map_or(buf.len(), |(b, _)| b);
        // the selected span, as character indices
        let span = |cur: usize, sel: Option<usize>| sel.map(|a| (a.min(cur), a.max(cur))).filter(|(a, b)| a != b);
        match k {
            Key::Char(c) if mods.ctrl && !mods.alt => match c {
                'a' => {
                    self.edit_sel = Some(0);
                    self.edit_cur = len;
                }
                'c' | 'x' => {
                    if let Some((a, b)) = span(self.edit_cur, self.edit_sel) {
                        self.clip_out = Some(buf[byte(buf, a)..byte(buf, b)].to_string());
                        if c == 'x' {
                            buf.replace_range(byte(buf, a)..byte(buf, b), "");
                            self.edit_cur = a;
                            self.edit_sel = None;
                        }
                    }
                }
                'v' => self.want_paste = true,
                _ => {}
            },
            Key::Char(ch) if !mods.alt => {
                if let Some((a, b)) = span(self.edit_cur, self.edit_sel) {
                    buf.replace_range(byte(buf, a)..byte(buf, b), "");
                    self.edit_cur = a;
                }
                let at = byte(buf, self.edit_cur);
                buf.insert(at, ch);
                self.edit_cur += 1;
                self.edit_sel = None;
            }
            Key::Backspace | Key::Delete => {
                if let Some((a, b)) = span(self.edit_cur, self.edit_sel) {
                    buf.replace_range(byte(buf, a)..byte(buf, b), "");
                    self.edit_cur = a;
                } else if k == Key::Backspace && self.edit_cur > 0 {
                    // Ctrl takes a whole word
                    let mut from = self.edit_cur - 1;
                    if mods.ctrl {
                        let chars: Vec<char> = buf.chars().collect();
                        while from > 0 && chars[from].is_whitespace() {
                            from -= 1;
                        }
                        while from > 0 && !chars[from - 1].is_whitespace() {
                            from -= 1;
                        }
                    }
                    buf.replace_range(byte(buf, from)..byte(buf, self.edit_cur), "");
                    self.edit_cur = from;
                } else if k == Key::Delete && self.edit_cur < len {
                    buf.replace_range(byte(buf, self.edit_cur)..byte(buf, self.edit_cur + 1), "");
                }
                self.edit_sel = None;
            }
            Key::Left | Key::Right | Key::Home | Key::End => {
                let to = match k {
                    Key::Left => match span(self.edit_cur, self.edit_sel) {
                        Some((a, _)) if !mods.shift => a,
                        _ => self.edit_cur.saturating_sub(1),
                    },
                    Key::Right => match span(self.edit_cur, self.edit_sel) {
                        Some((_, b)) if !mods.shift => b,
                        _ => (self.edit_cur + 1).min(len),
                    },
                    Key::Home => 0,
                    _ => len,
                };
                if mods.shift {
                    self.edit_sel.get_or_insert(self.edit_cur);
                } else {
                    self.edit_sel = None;
                }
                self.edit_cur = to;
            }
            Key::Enter => self.edit_end = Some(true),
            Key::Esc => self.edit_end = Some(false),
            _ => {}
        }
        true
    }

    /// Text from the clipboard goes into the field being edited (one line: line breaks become spaces).
    pub fn insert_text(&mut self, text: &str) -> bool {
        let Some((_, buf)) = self.edit.as_mut() else { return false };
        let clean: String = text.chars().map(|c| if c == '\n' || c == '\r' || c == '\t' { ' ' } else { c }).collect();
        let len = buf.chars().count();
        let cur = self.edit_cur.min(len);
        let byte = |buf: &str, i: usize| buf.char_indices().nth(i).map_or(buf.len(), |(b, _)| b);
        let (a, b) = match self.edit_sel.map(|s| (s.min(cur), s.max(cur))) {
            Some((a, b)) if a != b => (a, b),
            _ => (cur, cur),
        };
        buf.replace_range(byte(buf, a)..byte(buf, b), &clean);
        self.edit_cur = a + clean.chars().count();
        self.edit_sel = None;
        true
    }

    pub fn editing(&self) -> bool {
        self.edit.is_some()
    }

    /// Seconds since the previous frame (at most 50 ms, so a stall does not make things jump).
    pub fn begin_frame(&mut self) -> f32 {
        let now = Instant::now();
        let dt = self.last.map_or(0.016, |t| now.duration_since(t).as_secs_f32()).min(0.05);
        self.last = Some(now);
        self.cursor = Cursor::Default;
        dt
    }

    /// Call after the widgets: drops an edit whose field was not drawn and consumes the click.
    pub fn end_frame(&mut self, edit_seen: bool) {
        if !edit_seen {
            self.edit = None;
            self.edit_end = None;
        }
        if !self.held {
            self.drag = None;
        }
        self.click = false;
        self.double = false;
        self.scroll = 0.0;
    }

    /// Is something still moving? When not, the program may stop drawing until the next input.
    pub fn animating(&mut self) -> bool {
        let moving = self.anims.take_moving();
        if !moving {
            self.last = None;
        }
        moving || self.drag.is_some()
    }
}

pub struct Ui<'a> {
    pub d: &'a mut dyn Draw,
    pub mouse: [f32; 2],
    pub held: bool,
    pub click: bool,
    pub double: bool,
    pub mods: KeyMods,
    pub scroll: f32,
    drag: &'a mut Option<u64>,
    edit: &'a mut Option<(String, String)>,
    edit_end: &'a mut Option<bool>,
    edit_cur: &'a mut usize,
    edit_sel: &'a mut Option<usize>,
    /// the field being edited was drawn this frame (otherwise its page is gone and the edit is dropped)
    pub edit_seen: bool,
    pub ink: [f32; 3],
    pub accent: [f32; 3],
    pub dark: bool,
    /// how switches, sliders and pickers are built (the desktop's choice)
    pub controls: crate::style::Controls,
    /// the colour the next widgets use instead of the accent (see `tinted`)
    tint: Option<[f32; 3]>,
    next: u64,
    /// the text field being drawn hides what is typed (a password)
    mask: bool,
    /// the field draws no plate of its own (it sits on one the program drew: a glass composer)
    bare: bool,
    /// a control changed a value this frame
    pub changed: bool,
    anim: &'a mut Anims,
    cursor: &'a mut Cursor,
    dt: f32,
}

impl<'a> Ui<'a> {
    pub fn new(d: &'a mut dyn Draw, state: &'a mut State, style: Style, dt: f32) -> Ui<'a> {
        let State { mouse, held, click, double, mods, scroll, drag, edit, edit_end, edit_cur, edit_sel, anims, cursor, .. } = state;
        Ui { d, mouse: *mouse, held: *held, click: *click, double: *double, mods: *mods, scroll: *scroll, drag, edit, edit_end, edit_cur, edit_sel, edit_seen: false, ink: style.ink, accent: style.accent, dark: style.dark, controls: style.controls, tint: None, next: 0, mask: false, bare: false, changed: false, anim: anims, cursor, dt }
    }

    /// Start editing the text field `key` with `text` already in it (rename on F2, say).
    pub fn begin_edit(&mut self, key: &str, text: &str) {
        *self.edit = Some((key.to_string(), text.to_string()));
        *self.edit_cur = text.chars().count();
        *self.edit_sel = Some(0);
    }

    /// Is some text field being edited?
    /// The text of field `key` as it is being typed (None when that field is not being edited): a filter that narrows
    /// a list with every letter reads it, while `text_field` itself hands the text over only on Enter.
    pub fn edit_text(&self, key: &str) -> Option<&str> {
        self.edit.as_ref().filter(|(k, _)| k == key).map(|(_, t)| t.as_str())
    }

    /// Stop editing whatever field is being edited, keeping its old value (the program moved on: another folder).
    pub fn end_edit(&mut self) {
        *self.edit = None;
    }

    pub fn editing(&self) -> bool {
        self.edit.is_some()
    }

    /// Ask for a pointer shape while the pointer is on something (the first ask of a frame wins, except that a text
    /// beam beats a hand).
    pub fn want_cursor(&mut self, c: Cursor) {
        if *self.cursor == Cursor::Default || c == Cursor::Text {
            *self.cursor = c;
        }
    }

    /// Glide a value towards `target` (a sliding highlight, a switch knob ...): `speed` is how fast, per second.
    pub fn ease(&mut self, key: u64, target: f32, speed: f32) -> f32 {
        self.anim.ease(key, target, speed, self.dt)
    }

    /// Seconds since the previous frame: a program's own springs step by it.
    pub fn dt(&self) -> f32 {
        self.dt
    }

    /// Device pixels per logical pixel.
    pub fn px(&self) -> f32 {
        self.d.device_scale().max(0.25)
    }

    /// A logical coordinate moved onto the nearest device pixel boundary: an edge there is drawn crisp, not smeared
    /// over two pixels.
    pub fn on_grid(&self, v: f32) -> f32 {
        let s = self.px();
        (v * s).round() / s
    }

    /// A rectangle whose four edges sit on device pixel boundaries.
    pub fn grid_rect(&self, r: [f32; 4]) -> [f32; 4] {
        let (x0, y0, x1, y1) = (self.on_grid(r[0]), self.on_grid(r[1]), self.on_grid(r[0] + r[2]), self.on_grid(r[1] + r[3]));
        [x0, y0, x1 - x0, y1 - y0]
    }

    /// `n` whole device pixels, in logical pixels (at least one).
    pub fn dev_px(&self, n: f32) -> f32 {
        let s = self.px();
        (n * s).round().max(1.0) / s
    }

    /// Jump an eased value to `value` with no glide.
    pub fn snap(&mut self, key: u64, value: f32) {
        self.anim.set(key, value);
    }

    /// A value a widget keeps under `key` (set with `snap`, or the current point of an `ease`).
    pub fn recall(&self, key: u64) -> Option<f32> {
        self.anim.get(key)
    }

    /// A drag of a widget's own (a scrollbar's thumb): a press on `hot` makes `key` the thing being dragged, so no other
    /// control takes the pointer, and this is true for as long as the button stays down. `key` must not clash with the
    /// small numbers `id` hands out (use high bits).
    pub fn grab(&mut self, key: u64, hot: [f32; 4]) -> bool {
        if self.click && self.hit(hot) {
            *self.drag = Some(key);
        }
        self.held && *self.drag == Some(key)
    }

    /// Is the widget under `key` being dragged right now (see `grab`)?
    pub fn grabbed(&self, key: u64) -> bool {
        self.held && *self.drag == Some(key)
    }

    pub fn id(&mut self) -> u64 {
        self.next += 1;
        self.next
    }

    /// Is the pointer over `r`?
    pub fn hit(&self, r: [f32; 4]) -> bool {
        inside(self.mouse, r)
    }

    pub fn fill(&mut self, r: [f32; 4], radius: f32, c: [f32; 3], alpha: f32) {
        self.d.fill(r, radius, c, alpha);
    }

    pub fn grad(&mut self, r: [f32; 4], radius: f32, top: [f32; 3], bot: [f32; 3], alpha: f32) {
        self.d.grad(r, radius, top, bot, alpha);
    }

    pub fn glyph(&mut self, cx: f32, cy: f32, size: f32, name: &str, c: [f32; 3], a: f32) {
        self.d.glyph(cx, cy, size, name, c, a);
    }

    /// A line of text in the language in use: `t` is the English text (its key in the catalogue). Text that is data (a
    /// file name, a title, a number) goes through `text_raw`, which shows it as it is.
    #[allow(clippy::too_many_arguments)]
    pub fn text(&mut self, x: f32, y: f32, t: &str, size: f32, bold: bool, c: [f32; 3], a: f32, align: Align) {
        let shown = crate::i18n::tr(t);
        self.d.text(x, y, &shown, size, bold, c, a, align);
    }

    /// Text that is shown exactly as it is: names, titles, times, anything typed by the user.
    #[allow(clippy::too_many_arguments)]
    pub fn text_raw(&mut self, x: f32, y: f32, t: &str, size: f32, bold: bool, c: [f32; 3], a: f32, align: Align) {
        self.d.text(x, y, t, size, bold, c, a, align);
    }

    /// Width of `t` in the language in use.
    pub fn measure(&mut self, t: &str, size: f32, bold: bool) -> f32 {
        let shown = crate::i18n::tr(t);
        self.d.measure(&shown, size, bold)
    }

    /// Width of text shown as it is.
    pub fn measure_raw(&mut self, t: &str, size: f32, bold: bool) -> f32 {
        self.d.measure(t, size, bold)
    }

    /// A panel of glass for what floats over content (a control pill, a tooltip): over the program's own picture it
    /// blurs and bends what is behind it. (The software painter cannot see other windows; the shell frosts those.)
    pub fn glass(&mut self, r: [f32; 4], radius: f32, strength: f32) {
        self.d.glass(r, radius, strength, self.dark);
    }

    /// Clip what follows to `r` until `pop_clip` (clips nest).
    pub fn push_clip(&mut self, r: [f32; 4]) {
        self.d.push_clip(r);
    }

    pub fn pop_clip(&mut self) {
        self.d.pop_clip();
    }

    /// Draw (a part of) a picture into `r`.
    pub fn picture(&mut self, r: [f32; 4], pic: &std::sync::Arc<crate::picture::Pic>, src: [f32; 4], alpha: f32) {
        self.d.picture(r, pic, src, alpha);
    }

    pub fn button(&mut self, r: [f32; 4], label: &str, primary: bool) -> bool {
        let hov = self.hit(r);
        if hov {
            self.want_cursor(Cursor::Pointer);
        }
        if primary {
            self.fill(r, 8.0, self.accent, if hov { 0.95 } else { 0.85 });
            self.text(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0 + 4.5, label, 12.5, true, [1.0; 3], 1.0, Align::Center);
        } else {
            self.fill(r, 8.0, self.ink, if hov { 0.14 } else { 0.08 });
            self.text(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0 + 4.5, label, 12.5, false, self.ink, 0.95, Align::Center);
        }
        self.click && hov
    }

    /// A button with a Lucide glyph before its label (the glyph alone when the label is empty).
    pub fn icon_button(&mut self, r: [f32; 4], glyph: &str, label: &str, primary: bool) -> bool {
        let hov = self.hit(r);
        if hov {
            self.want_cursor(Cursor::Pointer);
        }
        let colour = if primary { [1.0; 3] } else { self.ink };
        if primary {
            self.fill(r, 8.0, self.accent, if hov { 0.95 } else { 0.85 });
        } else {
            self.fill(r, 8.0, self.ink, if hov { 0.14 } else { 0.08 });
        }
        let (gw, gap) = (15.0, if label.is_empty() { 0.0 } else { 7.0 });
        // a label that is too long for the button is cut short
        let room = r[2] - gw - gap - 14.0;
        let mut label = label.to_string();
        if !label.is_empty() && self.measure(&label, 12.5, primary) > room {
            while label.chars().count() > 1 && self.measure(&format!("{label}…"), 12.5, primary) > room {
                label.pop();
            }
            label.push('…');
        }
        let label = label.as_str();
        let tw = if label.is_empty() { 0.0 } else { self.measure(label, 12.5, primary) };
        let x0 = r[0] + ((r[2] - (gw + gap + tw)) / 2.0).max(4.0);
        let cy = r[1] + r[3] / 2.0;
        self.glyph(x0 + gw / 2.0, cy, gw, glyph, colour, if primary { 1.0 } else { 0.9 });
        if !label.is_empty() {
            self.text(x0 + gw + gap, cy + 4.5, label, 12.5, primary, colour, if primary { 1.0 } else { 0.95 }, Align::Left);
        }
        self.click && hov
    }

    /// A text field that shows dots instead of what is typed.
    pub fn password_field(&mut self, r: [f32; 4], key: &str, value: &mut String) -> bool {
        self.mask = true;
        let changed = self.text_field(r, key, value);
        self.mask = false;
        changed
    }

    /// A text field without a plate or focus ring of its own: only the text, the selection and the caret. For a field
    /// that sits on a panel the program draws itself (a glass composer, a search pill).
    pub fn text_field_bare(&mut self, r: [f32; 4], key: &str, value: &mut String) -> bool {
        self.bare = true;
        let changed = self.text_field(r, key, value);
        self.bare = false;
        changed
    }

    /// A single-line text field. Click to edit, Enter or a click elsewhere to apply, Esc to cancel.
    pub fn text_field(&mut self, r: [f32; 4], key: &str, value: &mut String) -> bool {
        let mut changed = false;
        let mut active = self.edit.as_ref().is_some_and(|(k, _)| k == key);
        if active {
            match self.edit_end.take() {
                Some(true) => {
                    if let Some((_, buf)) = self.edit.take() {
                        changed = buf != *value;
                        if changed {
                            *value = buf;
                        }
                    }
                    active = false;
                }
                Some(false) => {
                    *self.edit = None;
                    active = false;
                }
                None => {}
            }
        }
        let hov = self.hit(r);
        if hov {
            self.want_cursor(Cursor::Text);
        }
        if self.click {
            if hov && !active {
                *self.edit = Some((key.to_string(), value.clone()));
                *self.edit_cur = value.chars().count();
                *self.edit_sel = None;
                active = true;
            } else if hov && active {
                // a click inside puts the caret between the letters nearest to it
                if let Some((_, buf)) = self.edit.clone() {
                    let want = self.mouse[0] - (r[0] + 9.0);
                    let mut best = (f32::MAX, 0usize);
                    let n = buf.chars().count();
                    for i in 0..=n {
                        let prefix: String = buf.chars().take(i).collect();
                        let w = if self.mask { self.measure_raw(&"•".repeat(i), 12.5, false) } else { self.measure_raw(&prefix, 12.5, false) };
                        if (w - want).abs() < best.0 {
                            best = ((w - want).abs(), i);
                        }
                    }
                    *self.edit_cur = best.1;
                    *self.edit_sel = None;
                }
            } else if !hov && active {
                if let Some((_, buf)) = self.edit.take() {
                    changed |= buf != *value;
                    if buf != *value {
                        *value = buf;
                    }
                }
                active = false;
            }
        }
        if active {
            self.edit_seen = true;
        }
        if !self.bare {
            if active {
                self.fill([r[0] - 1.5, r[1] - 1.5, r[2] + 3.0, r[3] + 3.0], 8.0, self.accent, 0.55);
            }
            self.fill(r, 7.0, self.ink, if active || hov { 0.12 } else { 0.07 });
        }
        let hide = |s: &str, mask: bool| if mask { "•".repeat(s.chars().count()) } else { s.to_string() };
        let mask = self.mask;
        match (active, self.edit.clone()) {
            (true, Some((_, buf))) => {
                let len = buf.chars().count();
                let cur = (*self.edit_cur).min(len);
                let before = |n: usize| -> String { hide(&buf.chars().take(n).collect::<String>(), mask) };
                let x0 = r[0] + 9.0;
                // the selection, then the text, then the caret
                if let Some(a) = *self.edit_sel {
                    let (lo, hi) = (a.min(cur), a.max(cur).min(len));
                    if lo != hi {
                        let (xa, xb) = (x0 + self.measure_raw(&before(lo), 12.5, false), x0 + self.measure_raw(&before(hi), 12.5, false));
                        self.fill([xa, r[1] + 4.0, xb - xa, r[3] - 8.0], 3.0, self.accent, 0.4);
                    }
                }
                self.text_raw(x0, r[1] + r[3] / 2.0 + 4.5, &hide(&buf, mask), 12.5, false, self.ink, 0.95, Align::Left);
                let cx = x0 + self.measure_raw(&before(cur), 12.5, false);
                self.fill([cx, r[1] + 5.0, 1.5, r[3] - 10.0], 0.0, self.ink, 0.9);
            }
            _ => self.text_raw(r[0] + 9.0, r[1] + r[3] / 2.0 + 4.5, &hide(value, mask), 12.5, false, self.ink, 0.95, Align::Left),
        }
        if changed {
            self.changed = true;
        }
        changed
    }

    /// A slider whose track shows what the value does: `colour(t)` is the colour at position `t` (0 .. 1).
    pub fn gradient_slider(&mut self, r: [f32; 4], v: &mut f32, colour: &dyn Fn(f32) -> [f32; 3]) {
        let id = self.id();
        let hot = [r[0] - 8.0, r[1] - 8.0, r[2] + 16.0, r[3] + 16.0];
        if self.click && self.hit(hot) {
            *self.drag = Some(id);
        }
        if self.held && *self.drag == Some(id) {
            let nv = ((self.mouse[0] - r[0]) / r[2]).clamp(0.0, 1.0);
            if (nv - *v).abs() > 1e-4 {
                *v = nv;
                self.changed = true;
            }
        }
        let n = 28;
        let sw = r[2] / n as f32;
        for i in 0..n {
            let c = colour((i as f32 + 0.5) / n as f32);
            // the ends are rounded, the middle segments overlap by a hair so no seams show
            let rad = if i == 0 || i == n - 1 { r[3] / 2.0 } else { 0.0 };
            self.fill([r[0] + i as f32 * sw, r[1], sw + 0.8, r[3]], rad, c, 1.0);
        }
        let kx = r[0] + r[2] * v.clamp(0.0, 1.0);
        let cy = r[1] + r[3] / 2.0;
        self.fill([kx - 11.0, cy - 11.0, 22.0, 22.0], 11.0, [0.0; 3], 0.20);
        self.fill([kx - 10.0, cy - 10.0, 20.0, 20.0], 10.0, [1.0; 3], 1.0);
        self.fill([kx - 7.0, cy - 7.0, 14.0, 14.0], 7.0, colour(*v), 1.0);
    }

    /// The colour for a kind of thing: the accent, or with several accents its own.
    pub fn tone(&self, tone: crate::style::Tone) -> [f32; 3] {
        self.controls.tone(self.accent, tone)
    }

    /// Run `f` with the controls it draws in the colour of `tone` (switches, sliders, pickers).
    pub fn tinted<R>(&mut self, tone: crate::style::Tone, f: impl FnOnce(&mut Self) -> R) -> R {
        let saved = self.tint;
        self.tint = Some(self.tone(tone));
        let r = f(self);
        self.tint = saved;
        r
    }

    /// Like `tinted`, with a colour at hand.
    pub fn tinted_with<R>(&mut self, colour: [f32; 3], f: impl FnOnce(&mut Self) -> R) -> R {
        let saved = self.tint;
        self.tint = Some(colour);
        let r = f(self);
        self.tint = saved;
        r
    }

    fn accent_now(&self) -> [f32; 3] {
        self.tint.unwrap_or(self.accent)
    }

    fn flat(&self) -> bool {
        self.controls.kind == crate::style::ControlKind::Flat
    }

    /// A groove sunk into the surface (a switch's track, a slider's rail): a darker bed with its upper edge in shade, and
    /// the accent over it as far as `filled` (0..1) of its length. Flat controls have a plain fill instead.
    fn groove(&mut self, r: [f32; 4], radius: f32, filled: f32) {
        let radius = self.controls.radius(radius);
        if self.flat() {
            self.fill(r, radius, self.ink, 0.22 * (1.0 - filled));
            if filled > 0.0 {
                self.fill(r, radius, self.accent_now(), filled);
            }
            return;
        }
        // (in the dark a pale bed: a black one disappears into a dark card, and an off switch could not be seen)
        let (bed, a) = if self.dark { ([1.0, 1.0, 1.0], 0.16) } else { (self.ink, 0.17) };
        // sunk in: the bed darker towards its upper edge, as one gradient over the groove's own shape (a separate band
        // of shade could not follow the curve of the ends and would stop in a visible step)
        let depth = self.controls.depth.min(1.2);
        let top = bed.map(|v| v * (1.0 - (if self.dark { 0.45 } else { 0.6 }) * depth.min(1.0)));
        self.grad(r, radius, top, bed, a * (1.0 + 0.25 * depth));
        if filled > 0.0 {
            self.fill(r, radius, self.accent_now(), filled);
        }
    }

    /// The accent in a slider's groove up to where its knob is.
    fn groove_fill(&mut self, r: [f32; 4], radius: f32) {
        if r[2] < 1.0 {
            return;
        }
        let radius = self.controls.radius(radius);
        let a = self.accent_now();
        if self.flat() {
            self.fill([r[0], r[1], r[2].max(radius * 2.0), r[3]], radius, a, 1.0);
        } else {
            // sunk like the groove: darker towards the upper edge, one gradient over its own shape
            let k = 1.0 - 0.16 * self.controls.depth.min(1.2);
            self.grad([r[0], r[1], r[2].max(radius * 2.0), r[3]], radius, a.map(|v| v * k), a, 1.0);
        }
    }

    /// A round domed knob with a pale ring and a shadow, like the keys of the calculator (`c` its centre, `d` its width);
    /// a flat disc when the controls are flat.
    fn knob(&mut self, c: [f32; 2], d: f32, wash: f32) {
        self.knob_wh(c, d, d, wash);
    }

    /// A knob `w` wide and `h` tall: a disc when they are equal, a squashed pill when it is wider (slim controls).
    fn knob_wh(&mut self, c: [f32; 2], w: f32, h: f32, wash: f32) {
        let r = [c[0] - w / 2.0, c[1] - h / 2.0, w, h];
        if self.flat() {
            let rad = self.controls.radius(h / 2.0);
            self.fill(r, rad, [0.0; 3], 0.18);
            self.fill([r[0] + 1.0, r[1] + 1.0, w - 2.0, h - 2.0], (rad - 1.0).max(0.0), [1.0; 3], 1.0);
            return;
        }
        let (face, ring, shadow) = if self.dark { ([0.72, 0.76, 0.82], [0.88, 0.91, 0.95], ([0.0, 0.0, 0.0], 0.42)) } else { ([0.93, 0.95, 0.97], [1.0, 1.0, 1.0], ([0.30, 0.38, 0.46], 0.30)) };
        let wsh = self.ease(0x4b0b + (c[1] as u64) * 4096 + (c[0] as u64 / 64), wash, 20.0);
        let (round, depth) = (self.controls.round, self.controls.depth);
        crate::retro::key_body_shaped(self, r, face, ring, shadow, wsh, 0.0, round, depth);
    }

    pub fn toggle(&mut self, x: f32, y: f32, on: &mut bool) {
        // it takes the place of a 44 x 24 switch whatever its thickness: the track is lower when slim (centred in that
        // place) and the knob a squashed pill
        let hit = [x, y, 44.0, 24.0];
        let (tw, th) = (self.controls.thick(40.0, 44.0), self.controls.thick(16.0, 24.0));
        let r = [x + (44.0 - tw) / 2.0, y + (24.0 - th) / 2.0, tw, th];
        let hov = self.hit(hit);
        if hov {
            self.want_cursor(Cursor::Pointer);
        }
        let key = self.id() * 64;
        let t = self.ease(key, if *on { 1.0 } else { 0.0 }, 16.0);
        // a groove that fills with the accent as the knob goes over, and a round domed knob like a key of the calculator
        let saved = self.tint;
        if self.tint.is_none() {
            self.tint = Some(self.tone(crate::style::Tone::Success));
        }
        self.groove(r, th / 2.0, t);
        self.tint = saved;
        if self.flat() {
            // the knob stretches a little on its way over
            let (kh, kw) = (th - 4.0, self.controls.thick(17.0, 20.0));
            let travel = tw - 4.0 - kw;
            let stretch = 8.0 * 4.0 * t * (1.0 - t);
            let rad = self.controls.radius(kh / 2.0);
            self.fill([r[0] + 2.0 + travel * t - stretch * t, r[1] + 2.0, kw + stretch, kh], rad, [1.0; 3], 1.0);
        } else {
            let (kh, kw) = (self.controls.thick(14.0, 21.0), self.controls.thick(20.0, 21.0));
            let travel = tw - 3.0 - kw;
            let cy = r[1] + th / 2.0 - self.controls.thick(0.0, 1.0);
            self.knob_wh([r[0] + 1.5 + kw / 2.0 + travel * t, cy], kw, kh, if hov { 1.0 } else { 0.0 });
        }
        if self.click && hov {
            *on = !*on;
            self.changed = true;
        }
    }

    /// Horizontal slider; `r` is the track area.
    pub fn slider(&mut self, r: [f32; 4], v: &mut f32, min: f32, max: f32, unit: &str) {
        self.slider_bare(r, v, min, max);
        let cy = r[1] + r[3] / 2.0;
        self.text_raw(r[0] + r[2] + 18.0, cy + 4.5, &format!("{}{}", fmt_num(*v), unit), 12.0, false, self.ink, 0.7, Align::Left);
    }

    /// The slider without the number after it (a seek line, a volume): the same groove and knob. Returns true while
    /// the knob is being dragged, so a caller can act on the release (a player seeks once the pointer lets go).
    pub fn slider_bare(&mut self, r: [f32; 4], v: &mut f32, min: f32, max: f32) -> bool {
        let id = self.id();
        let hot = [r[0] - 6.0, r[1] - 8.0, r[2] + 12.0, r[3] + 16.0];
        if self.hit(hot) || *self.drag == Some(id) {
            self.want_cursor(Cursor::Pointer);
        }
        if self.click && self.hit(hot) {
            *self.drag = Some(id);
        }
        let (kw, kh) = if self.flat() { (self.controls.thick(18.0, 16.0), self.controls.thick(11.0, 16.0)) } else { (self.controls.thick(20.0, 21.0), self.controls.thick(13.0, 21.0)) };
        // the knob stays on the track: at either end its side is flush with the track's end
        let travel = (r[2] - kw).max(1.0);
        let x0 = r[0] + kw / 2.0;
        // how far the pointer is dragged past an end (px; minus: past the start), for the knob's squash
        let mut over = 0.0;
        if self.held && *self.drag == Some(id) {
            let raw = (self.mouse[0] - x0) / travel;
            let t = raw.clamp(0.0, 1.0);
            over = (raw - t) * travel;
            let nv = min + t * (max - min);
            if (nv - *v).abs() > 1e-4 {
                *v = nv;
                self.changed = true;
            }
        }
        let t = ((*v - min) / (max - min)).clamp(0.0, 1.0);
        let cy = r[1] + r[3] / 2.0;
        // a groove with the accent filling it to the knob's middle, and a round domed knob
        // slim: a thin rail and a knob squashed into a short pill
        let rail = if self.flat() { self.controls.thick(3.0, 6.0) } else { self.controls.thick(4.0, 8.0) };
        let track = self.grid_rect([r[0], cy - rail / 2.0, r[2], rail]);
        self.groove(track, track[3] / 2.0, 0.0);
        let mut kx = x0 + travel * t;
        // pressed against the end: the further past it, the flatter (with less and less give), and it springs back
        // when let go; the side against the end stays where it is
        let push = (1.0 - (-over.abs() / 40.0).exp()) * over.signum();
        let squash = self.ease(id * 64 + 7, push, if over != 0.0 { 30.0 } else { 14.0 });
        let sq = squash.abs().min(1.0);
        let (w, h) = (kw * (1.0 - 0.32 * sq), kh * (1.0 + 0.14 * sq));
        if squash > 0.0 {
            kx = r[0] + r[2] - w / 2.0;
        } else if squash < 0.0 {
            kx = r[0] + w / 2.0;
        }
        self.groove_fill([track[0], track[1], (kx - track[0]).max(0.0), track[3]], track[3] / 2.0);
        let held = *self.drag == Some(id);
        // at rest on the pixel grid (a knob between pixels has soft sides)
        let knob = [kx - w / 2.0, cy - h / 2.0, w, h];
        let knob = if sq < 1e-3 && !(held && self.held) { self.grid_rect(knob) } else { knob };
        self.knob_wh([knob[0] + knob[2] / 2.0, knob[1] + knob[3] / 2.0], knob[2], knob[3], if held || self.hit(hot) { 1.0 } else { 0.0 });
        held && self.held
    }

    /// Segmented control: a groove with a domed, ringed pill (like the knob of a switch) under the chosen label, which
    /// slides to the new choice with a little squeeze on the way; returns true when the selection changed.
    pub fn segmented(&mut self, r: [f32; 4], opts: &[&str], sel: &mut usize) -> bool {
        let n = opts.len().max(1) as f32;
        let flat = self.flat();
        // pixel perfect: the groove and the resting pill have every edge on a device pixel
        let r = self.grid_rect(r);
        self.groove(r, (r[3] / 2.0).min(14.0), 0.0);
        // the pill fills the groove's height edge to edge (no margin round it: any gap reads as a seam)
        let inset = 0.0;
        let sw = (r[2] - 2.0 * inset) / n;
        let base = self.id() * 64;
        let pos = self.ease(base, *sel as f32, 15.0);
        let travel = (pos - pos.round()).abs();
        let squeeze = 1.0 - 0.10 * (travel * 2.0).min(1.0);
        let (ph, py) = (r[3] - 2.0 * inset, r[1] + inset);
        let pill = [r[0] + inset + pos * sw + sw * (1.0 - squeeze) / 2.0, py + ph * (1.0 - squeeze) / 2.0, sw * squeeze, ph * squeeze];
        // at rest on the grid; while it glides it moves smoothly
        let pill = if travel < 1e-3 { self.grid_rect(pill) } else { pill };
        let (mut face, mut ring, shadow) = if self.dark { ([0.377, 0.377, 0.377], [0.58, 0.63, 0.72], ([0.0, 0.0, 0.0], 0.0)) } else { ([0.95, 0.96, 0.98], [1.0, 1.0, 1.0], ([0.30, 0.38, 0.46], 0.0)) };
        let accent_pick = self.controls.accent_pick;
        if accent_pick {
            face = [self.accent_now()[0] * 0.9 + 0.1, self.accent_now()[1] * 0.9 + 0.1, self.accent_now()[2] * 0.9 + 0.1];
            // the rim only a shade lighter than the face: a pale rim reads as a gap between the pill and the groove
            ring = face.map(|v| v + (1.0 - v) * 0.16);
        }
        if flat {
            let rad = self.controls.radius(6.0);
            self.fill(pill, rad, if accent_pick { self.accent_now() } else { [1.0; 3] }, if accent_pick { 1.0 } else if self.dark { 0.20 } else { 0.95 });
        } else {
            let (round, depth) = (self.controls.round, self.controls.depth);
            if self.dark && !accent_pick {
                crate::retro::key_body_dark_shadowed(self, pill, [0.299, 0.299, 0.299], 0.0, 0.0, round, depth, 0.0);
            } else {
                crate::retro::key_body_shaped(self, pill, face, ring, shadow, 0.0, 0.0, round, depth);
            }
        }
        let mut changed = false;
        for (i, o) in opts.iter().enumerate() {
            let sr = [r[0] + inset + i as f32 * sw, py, sw, ph];
            let selected = i == *sel;
            let hov = self.hit(sr);
            if hov {
                self.want_cursor(Cursor::Pointer);
            }
            // the label takes the selected look as the pill arrives under it
            let near = (1.0 - (pos - i as f32).abs()).clamp(0.0, 1.0);
            let hover = self.ease(base + 1 + i as u64, if hov && !selected { 1.0 } else { 0.0 }, 22.0);
            // on an accent-coloured pill the label is white
            let on_pill = if accent_pick { [1.0; 3] } else { self.ink };
            let text = [0, 1, 2].map(|c| self.ink[c] + (on_pill[c] - self.ink[c]) * near);
            self.text(sr[0] + sr[2] / 2.0, sr[1] + sr[3] / 2.0 + 4.5, o, 12.0, selected, text, 0.72 + 0.28 * near + 0.1 * hover, Align::Center);
            if self.click && hov && !selected {
                *sel = i;
                changed = true;
            }
        }
        if changed {
            self.changed = true;
        }
        changed
    }

    /// A row's label, vertically centred on `y`.
    pub fn row(&mut self, x: f32, y: f32, label: &str) {
        self.text(x, y + 4.5, label, 13.0, false, self.ink, 0.92, Align::Left);
    }

    /// A small heading above a group of rows.
    pub fn section(&mut self, x: f32, y: f32, label: &str) {
        self.text(x, y, label, 11.5, true, self.ink, 0.55, Align::Left);
    }
}

pub fn fmt_num(v: f32) -> String {
    if v.abs() >= 10.0 || (v - v.round()).abs() < 0.005 {
        format!("{:.0}", v)
    } else {
        format!("{:.2}", v)
    }
}

#[cfg(test)]
mod edit_tests {
    use super::*;

    fn editing(text: &str) -> State {
        let mut st = State::default();
        st.begin_edit("f", text);
        st
    }

    fn typed(st: &mut State, s: &str) {
        for c in s.chars() {
            st.key(Key::Char(c), KeyMods::default());
        }
    }

    fn ctrl() -> KeyMods {
        KeyMods { ctrl: true, ..KeyMods::default() }
    }

    #[test]
    fn typing_replaces_the_selected_text_and_backspace_erases() {
        let mut st = editing("photo.png");
        // begin_edit selects everything
        typed(&mut st, "ab");
        assert_eq!(st.edit.as_ref().unwrap().1, "ab");
        st.key(Key::Backspace, KeyMods::default());
        assert_eq!(st.edit.as_ref().unwrap().1, "a");
        st.key(Key::Backspace, KeyMods::default());
        st.key(Key::Backspace, KeyMods::default());
        assert_eq!(st.edit.as_ref().unwrap().1, "");
    }

    #[test]
    fn the_caret_moves_and_inserts_in_the_middle() {
        let mut st = editing("ac");
        st.key(Key::Home, KeyMods::default());
        st.key(Key::Right, KeyMods::default());
        typed(&mut st, "b");
        assert_eq!(st.edit.as_ref().unwrap().1, "abc");
        st.key(Key::Delete, KeyMods::default());
        assert_eq!(st.edit.as_ref().unwrap().1, "ab");
        st.key(Key::End, KeyMods::default());
        typed(&mut st, "ї");
        assert_eq!(st.edit.as_ref().unwrap().1, "abї", "letters outside ASCII work");
    }

    #[test]
    fn ctrl_a_c_x_v_use_the_clipboard() {
        let mut st = editing("hello world");
        st.key(Key::Char('a'), ctrl());
        st.key(Key::Char('c'), ctrl());
        assert_eq!(st.clip_out.take().as_deref(), Some("hello world"));
        st.key(Key::Char('x'), ctrl());
        assert_eq!(st.clip_out.take().as_deref(), Some("hello world"));
        assert_eq!(st.edit.as_ref().unwrap().1, "");
        st.key(Key::Char('v'), ctrl());
        assert!(st.want_paste);
        assert!(st.insert_text("pasted\ntext"));
        assert_eq!(st.edit.as_ref().unwrap().1, "pasted text");
        // Ctrl+Backspace takes a word
        st.key(Key::Backspace, ctrl());
        assert_eq!(st.edit.as_ref().unwrap().1, "pasted ");
    }

    #[test]
    fn shift_arrows_select_and_typing_replaces() {
        let mut st = editing("abcd");
        st.key(Key::End, KeyMods::default());
        let shift = KeyMods { shift: true, ..KeyMods::default() };
        st.key(Key::Left, shift);
        st.key(Key::Left, shift);
        typed(&mut st, "X");
        assert_eq!(st.edit.as_ref().unwrap().1, "abX");
    }
}

#[cfg(test)]
mod slider_tests {
    use super::*;
    use crate::canvas::Canvas;
    use crate::soft::{Gfx, SoftDraw};

    const W: usize = 240;
    const H: usize = 40;

    /// One frame of a slider over x 20..220; returns the pixels (BGRA) and the value.
    fn frame(st: &mut State, gfx: &mut Gfx, v: &mut f32) -> Vec<u8> {
        let mut buf = vec![0u8; W * H * 4];
        {
            let mut canvas = Canvas::new(&mut buf, W, H);
            let mut painter = SoftDraw::new(&mut canvas, gfx);
            let dt = st.begin_frame();
            let mut ui = Ui::new(&mut painter, st, Style::new(false, [0.2, 0.5, 1.0]), dt);
            ui.slider_bare([20.0, 10.0, 200.0, 20.0], v, 0.0, 1.0);
            let seen = ui.edit_seen;
            st.end_frame(seen);
        }
        if let Ok(dir) = std::env::var("TMP_SHOT_DIR") {
            let mut rgba = buf.clone();
            rgba.chunks_exact_mut(4).for_each(|p| p.swap(0, 2));
            let n = std::fs::read_dir(&dir).map(|d| d.count()).unwrap_or(0);
            let _ = image::save_buffer(std::path::Path::new(&dir).join(format!("slider-{n}.png")), &rgba, W as u32, H as u32, image::ColorType::Rgba8);
        }
        buf
    }

    /// The columns (x) where something is painted on the knob's row.
    fn painted(buf: &[u8], y: usize) -> (usize, usize) {
        let xs: Vec<usize> = (0..W).filter(|&x| buf[(y * W + x) * 4 + 3] > 20).collect();
        (*xs.first().unwrap_or(&0), *xs.last().unwrap_or(&0))
    }

    #[test]
    fn the_knob_ends_flush_with_the_track_and_squashes_when_pushed_past_it() {
        let (mut st, mut gfx) = (State::default(), Gfx::new());
        let mut v = 1.0;
        let buf = frame(&mut st, &mut gfx, &mut v);
        // at the end the knob does not stick out past the track (220), a pixel of anti-aliasing at most
        let (_, right) = painted(&buf, 23);
        assert!(right <= 220, "knob reaches x {right}");
        // dragged far past the end: still at the end, the value at most 1, the knob narrower than at rest
        // the knob's width on a row below the rail (only the knob is painted there)
        let knob_w = |buf: &[u8]| {
            let (l, r) = painted(buf, 23);
            r - l
        };
        let rest = knob_w(&buf);
        st.pointer_move([205.0, 20.0]);
        frame(&mut st, &mut gfx, &mut v);
        st.pointer_down([205.0, 20.0]);
        frame(&mut st, &mut gfx, &mut v);
        st.pointer_move([300.0, 20.0]);
        let mut last = Vec::new();
        for _ in 0..30 {
            std::thread::sleep(std::time::Duration::from_millis(4));
            last = frame(&mut st, &mut gfx, &mut v);
        }
        assert!(v <= 1.0 && v > 0.99, "{v}");
        let pushed = knob_w(&last);
        assert!(pushed + 3 < rest, "pushed {pushed} vs at rest {rest}");
        assert!(painted(&last, 23).1 <= 220);
        // let go: it springs back to its own width
        st.pointer_up();
        for _ in 0..80 {
            std::thread::sleep(std::time::Duration::from_millis(4));
            last = frame(&mut st, &mut gfx, &mut v);
        }
        assert_eq!(knob_w(&last), rest);
    }
}
