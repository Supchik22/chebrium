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
    pub anims: Anims,
    last: Option<Instant>,
}

impl Default for State {
    fn default() -> State {
        State { cursor: Cursor::Default, mouse: [-1000.0; 2], held: false, click: false, double: false, mods: KeyMods::default(), scroll: 0.0, last_click: None, drag: None, edit: None, edit_end: None, anims: Anims::default(), last: None }
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
    }

    pub fn pointer_up(&mut self) {
        self.held = false;
        self.drag = None;
    }

    /// A key press while a text field is being edited. True when it was used.
    pub fn key(&mut self, k: Key, mods: KeyMods) -> bool {
        let Some((_, buf)) = self.edit.as_mut() else { return false };
        match k {
            Key::Char(c) if !mods.ctrl && !mods.alt => buf.push(c),
            Key::Backspace => {
                buf.pop();
            }
            Key::Enter => self.edit_end = Some(true),
            Key::Esc => self.edit_end = Some(false),
            _ => {}
        }
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
    /// a control changed a value this frame
    pub changed: bool,
    anim: &'a mut Anims,
    cursor: &'a mut Cursor,
    dt: f32,
}

impl<'a> Ui<'a> {
    pub fn new(d: &'a mut dyn Draw, state: &'a mut State, style: Style, dt: f32) -> Ui<'a> {
        let State { mouse, held, click, double, mods, scroll, drag, edit, edit_end, anims, cursor, .. } = state;
        Ui { d, mouse: *mouse, held: *held, click: *click, double: *double, mods: *mods, scroll: *scroll, drag, edit, edit_end, edit_seen: false, ink: style.ink, accent: style.accent, dark: style.dark, controls: style.controls, tint: None, next: 0, mask: false, changed: false, anim: anims, cursor, dt }
    }

    /// Start editing the text field `key` with `text` already in it (rename on F2, say).
    pub fn begin_edit(&mut self, key: &str, text: &str) {
        *self.edit = Some((key.to_string(), text.to_string()));
    }

    /// Is some text field being edited?
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

    /// Jump an eased value to `value` with no glide.
    pub fn snap(&mut self, key: u64, value: f32) {
        self.anim.set(key, value);
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

    /// A panel of glass for what floats over content (a control pill, a tooltip): the theme's tint, a faint light rim and a
    /// bright edge along the top. (The software painter cannot blur what is behind it; the shell does that for whole windows.)
    pub fn glass(&mut self, r: [f32; 4], radius: f32, strength: f32) {
        let base = if self.dark { [0.11, 0.11, 0.11] } else { [0.97, 0.98, 1.0] };
        self.fill([r[0] - 1.0, r[1] - 1.0, r[2] + 2.0, r[3] + 2.0], radius + 1.0, [1.0; 3], 0.10 * strength);
        self.fill(r, radius, base, 0.74 * strength);
        self.fill([r[0] + radius * 0.7, r[1] + 0.5, (r[2] - radius * 1.4).max(0.0), 1.0], 0.5, [1.0; 3], if self.dark { 0.20 } else { 0.9 } * strength);
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
                active = true;
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
            self.fill([r[0] - 1.5, r[1] - 1.5, r[2] + 3.0, r[3] + 3.0], 8.0, self.accent, 0.55);
        }
        self.fill(r, 7.0, self.ink, if active || hov { 0.12 } else { 0.07 });
        let hide = |s: &str| if self.mask { "•".repeat(s.chars().count()) } else { s.to_string() };
        let shown = match (active, self.edit.as_ref()) {
            (true, Some((_, buf))) => format!("{}|", hide(buf)),
            _ => hide(value),
        };
        self.text_raw(r[0] + 9.0, r[1] + r[3] / 2.0 + 4.5, &shown, 12.5, false, self.ink, 0.95, Align::Left);
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
        let (bed, a) = if self.dark { ([0.0, 0.0, 0.0], 0.34) } else { (self.ink, 0.17) };
        self.fill(r, radius, bed, a);
        if filled > 0.0 {
            self.fill(r, radius, self.accent_now(), filled);
        }
        // the shade along the upper edge, the light along the lower
        let depth = self.controls.depth.min(1.2);
        self.fill([r[0] + radius * 0.5, r[1], (r[2] - radius).max(0.0), (r[3] * 0.24).max(2.0)], radius * 0.4, [0.0; 3], (if self.dark { 0.16 } else { 0.07 }) * depth);
        self.fill([r[0] + radius * 0.7, r[1] + r[3] - 1.0, (r[2] - radius * 1.4).max(0.0), 1.0], 0.5, [1.0; 3], (if self.dark { 0.10 } else { 0.55 }) * depth);
    }

    /// The accent in a slider's groove up to where its knob is.
    fn groove_fill(&mut self, r: [f32; 4], radius: f32) {
        if r[2] < 1.0 {
            return;
        }
        let radius = self.controls.radius(radius);
        self.fill([r[0], r[1], r[2].max(radius * 2.0), r[3]], radius, self.accent_now(), 1.0);
        if !self.flat() {
            self.fill([r[0] + radius * 0.5, r[1], (r[2] - radius).max(0.0), (r[3] * 0.30).max(2.0)], radius * 0.4, [0.0; 3], 0.14 * self.controls.depth.min(1.2));
        }
    }

    /// A round domed knob with a pale ring and a shadow, like the keys of the calculator (`c` its centre, `d` its width);
    /// a flat disc when the controls are flat.
    fn knob(&mut self, c: [f32; 2], d: f32, wash: f32) {
        if self.flat() {
            let rad = self.controls.radius(d / 2.0);
            self.fill([c[0] - d / 2.0, c[1] - d / 2.0, d, d], rad, [0.0; 3], 0.18);
            self.fill([c[0] - d / 2.0 + 1.0, c[1] - d / 2.0 + 1.0, d - 2.0, d - 2.0], (rad - 1.0).max(0.0), [1.0; 3], 1.0);
            return;
        }
        let (face, ring, shadow) = if self.dark { ([0.72, 0.76, 0.82], [0.88, 0.91, 0.95], ([0.0, 0.0, 0.0], 0.42)) } else { ([0.93, 0.95, 0.97], [1.0, 1.0, 1.0], ([0.30, 0.38, 0.46], 0.30)) };
        let w = self.ease(0x4b0b + (c[1] as u64) * 4096 + (c[0] as u64 / 64), wash, 20.0);
        let (round, depth) = (self.controls.round, self.controls.depth);
        crate::retro::key_body_shaped(self, [c[0] - d / 2.0, c[1] - d / 2.0, d, d], face, ring, shadow, w, 0.0, round, depth);
    }

    pub fn toggle(&mut self, x: f32, y: f32, on: &mut bool) {
        let r = [x, y, 44.0, 24.0];
        let hov = self.hit(r);
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
        self.groove(r, 12.0, t);
        self.tint = saved;
        if self.flat() {
            // the knob stretches a little on its way over
            let stretch = 8.0 * 4.0 * t * (1.0 - t);
            let rad = self.controls.radius(10.0);
            self.fill([x + 2.0 + 20.0 * t - stretch * t, y + 2.0, 20.0 + stretch, 20.0], rad, [1.0; 3], 1.0);
        } else {
            self.knob([x + 12.0 + 20.0 * t, y + 11.0], 21.0, if hov { 1.0 } else { 0.0 });
        }
        if self.click && hov {
            *on = !*on;
            self.changed = true;
        }
    }

    /// Horizontal slider; `r` is the track area.
    pub fn slider(&mut self, r: [f32; 4], v: &mut f32, min: f32, max: f32, unit: &str) {
        let id = self.id();
        let hot = [r[0] - 6.0, r[1] - 8.0, r[2] + 12.0, r[3] + 16.0];
        if self.hit(hot) || *self.drag == Some(id) {
            self.want_cursor(Cursor::Pointer);
        }
        if self.click && self.hit(hot) {
            *self.drag = Some(id);
        }
        if self.held && *self.drag == Some(id) {
            let t = ((self.mouse[0] - r[0]) / r[2]).clamp(0.0, 1.0);
            let nv = min + t * (max - min);
            if (nv - *v).abs() > 1e-4 {
                *v = nv;
                self.changed = true;
            }
        }
        let t = ((*v - min) / (max - min)).clamp(0.0, 1.0);
        let cy = r[1] + r[3] / 2.0;
        // a groove with the accent filling it, and a round domed knob on the end of the fill
        let rail = if self.flat() { 6.0 } else { 8.0 };
        self.groove([r[0], cy - rail / 2.0, r[2], rail], rail / 2.0, 0.0);
        self.groove_fill([r[0], cy - rail / 2.0, (r[2] * t).max(0.0), rail], rail / 2.0);
        let kx = r[0] + r[2] * t;
        let held = *self.drag == Some(id);
        self.knob([kx, cy], if self.flat() { 16.0 } else { 21.0 }, if held || self.hit(hot) { 1.0 } else { 0.0 });
        self.text_raw(r[0] + r[2] + 18.0, cy + 4.5, &format!("{}{}", fmt_num(*v), unit), 12.0, false, self.ink, 0.7, Align::Left);
    }

    /// Segmented control: a groove with a domed, ringed pill (like the knob of a switch) under the chosen label, which
    /// slides to the new choice with a little squeeze on the way; returns true when the selection changed.
    pub fn segmented(&mut self, r: [f32; 4], opts: &[&str], sel: &mut usize) -> bool {
        let n = opts.len().max(1) as f32;
        let flat = self.flat();
        self.groove(r, (r[3] / 2.0).min(14.0), 0.0);
        let sw = (r[2] - 6.0) / n;
        let base = self.id() * 64;
        let pos = self.ease(base, *sel as f32, 15.0);
        let travel = (pos - pos.round()).abs();
        let squeeze = 1.0 - 0.10 * (travel * 2.0).min(1.0);
        // (the pill's shadow falls three pixels below it: it is kept inside the groove)
        let (ph, py) = (r[3] - 6.0, r[1] + 2.0);
        let pill = [r[0] + 3.0 + pos * sw + sw * (1.0 - squeeze) / 2.0, py + ph * (1.0 - squeeze) / 2.0, sw * squeeze, ph * squeeze];
        let (mut face, mut ring, shadow) = if self.dark { ([0.377, 0.377, 0.377], [0.58, 0.63, 0.72], ([0.0, 0.0, 0.0], 0.45)) } else { ([0.95, 0.96, 0.98], [1.0, 1.0, 1.0], ([0.30, 0.38, 0.46], 0.28)) };
        let accent_pick = self.controls.accent_pick;
        if accent_pick {
            face = [self.accent_now()[0] * 0.9 + 0.1, self.accent_now()[1] * 0.9 + 0.1, self.accent_now()[2] * 0.9 + 0.1];
            ring = [self.accent_now()[0] * 0.45 + 0.55, self.accent_now()[1] * 0.45 + 0.55, self.accent_now()[2] * 0.45 + 0.55];
        }
        if flat {
            let rad = self.controls.radius(6.0);
            self.fill([pill[0], pill[1] - 1.0, pill[2], pill[3] + 2.0], rad, if accent_pick { self.accent_now() } else { [1.0; 3] }, if accent_pick { 1.0 } else if self.dark { 0.20 } else { 0.95 });
        } else {
            let (round, depth) = (self.controls.round, self.controls.depth);
            if self.dark && !accent_pick {
                crate::retro::key_body_dark(self, pill, [0.299, 0.299, 0.299], 0.0, 0.0, round, depth);
            } else {
                crate::retro::key_body_shaped(self, pill, face, ring, shadow, 0.0, 0.0, round, depth);
            }
        }
        let mut changed = false;
        for (i, o) in opts.iter().enumerate() {
            let sr = [r[0] + 3.0 + i as f32 * sw, py, sw, ph];
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
