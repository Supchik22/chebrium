//! A column of settings rows: a label on the left and a control on the right, all centred on one line, so every
//! page lines up the same way and pages do not carry their own pixel arithmetic.

use crate::draw::Align;
use crate::ui::Ui;

pub struct Form {
    pub x: f32,
    pub y: f32,
    pub cw: f32,
    /// distance between rows
    pub pitch: f32,
}

impl Form {
    pub fn new(x: f32, y: f32, cw: f32, pitch: f32) -> Form {
        Form { x, y, cw, pitch }
    }

    pub fn skip(&mut self, dy: f32) {
        self.y += dy;
    }

    pub fn slider(&mut self, ui: &mut Ui<'_>, label: &str, v: &mut f32, lo: f32, hi: f32, unit: &str) {
        ui.row(self.x, self.y, label);
        ui.slider([self.x + 150.0, self.y - 9.0, self.cw - 230.0, 18.0], v, lo, hi, unit);
        self.y += self.pitch;
    }

    pub fn toggle(&mut self, ui: &mut Ui<'_>, label: &str, v: &mut bool) {
        ui.row(self.x, self.y, label);
        ui.toggle(self.x + self.cw - 44.0, self.y - 12.0, v);
        self.y += self.pitch;
    }

    /// A segmented control over the values of `opts`; true when the choice changed.
    pub fn pick<T: Copy + PartialEq>(&mut self, ui: &mut Ui<'_>, label: &str, v: &mut T, opts: &[(T, &str)]) -> bool {
        ui.row(self.x, self.y, label);
        let names: Vec<&str> = opts.iter().map(|o| o.1).collect();
        let mut sel = opts.iter().position(|o| o.0 == *v).unwrap_or(0);
        let w = (self.cw - 150.0).min(120.0 * opts.len() as f32);
        let changed = ui.segmented([self.x + self.cw - w, self.y - 15.0, w, 30.0], &names, &mut sel);
        if changed {
            *v = opts[sel].0;
        }
        self.y += self.pitch;
        changed
    }

    /// `‹ value ›`: -1 / +1 when a side was pressed.
    pub fn stepper(&mut self, ui: &mut Ui<'_>, label: &str, value: &str) -> i32 {
        ui.row(self.x, self.y, label);
        let mid = self.x + 150.0 + (self.cw - 230.0) / 2.0;
        let step = if ui.button([self.x + 150.0, self.y - 12.0, 28.0, 24.0], "‹", false) {
            -1
        } else if ui.button([self.x + self.cw - 80.0, self.y - 12.0, 28.0, 24.0], "›", false) {
            1
        } else {
            0
        };
        ui.text(mid, self.y + 4.5, value, 12.5, false, ui.ink, 0.95, Align::Center);
        self.y += self.pitch;
        step
    }

    /// A text field; `hint` is shown to the right of it. True when the text was applied.
    pub fn text(&mut self, ui: &mut Ui<'_>, label: &str, key: &str, v: &mut String, hint: Option<&str>) -> bool {
        ui.row(self.x, self.y, label);
        let w = self.cw - 150.0 - if hint.is_some() { 110.0 } else { 0.0 };
        let changed = ui.text_field([self.x + 150.0, self.y - 13.0, w, 26.0], key, v);
        if let Some(h) = hint {
            ui.text(self.x + self.cw, self.y + 4.5, h, 11.5, false, ui.ink, 0.55, Align::Right);
        }
        self.y += self.pitch;
        changed
    }

    pub fn note(&mut self, ui: &mut Ui<'_>, text: &str, faint: bool) {
        ui.text(self.x, self.y, text, if faint { 12.0 } else { 13.0 }, false, ui.ink, if faint { 0.55 } else { 0.8 }, Align::Left);
        self.y += 24.0;
    }
}
