//! A context menu drawn inside the window: what a right click on a list or a canvas opens. It takes the entries of the
//! shell's menu bar ([`Item`]: labels, keys, check marks, icons, submenus, red destructive entries), pops in on a spring,
//! has one highlight per panel that glides between rows, opens submenus on hover and works from the keyboard.
//!
//! Immediate mode like the rest of the kit. Keep an `Option<PopMenu>` in the program; while it is `Some`, take the pointer
//! from the rest of the window at the start of the frame and give it back to the menu at the end:
//!
//! ```ignore
//! let held = self.menu.as_ref().map(|_| popmenu::shield(ui));
//! /* ... the window's own widgets, deaf to the pointer while the menu is open ... */
//! if let (Some(m), Some(held)) = (self.menu.as_mut(), held) {
//!     match m.draw(ui, held, size) {
//!         Outcome::Chosen(id) => { self.menu = None; self.command(id) }
//!         Outcome::Closed => self.menu = None,
//!         Outcome::Open => {}
//!     }
//!     host.redraw |= self.menu.as_ref().is_some_and(|m| m.animating());
//! }
//! ```
//! Keys go to [`PopMenu::key`] first while it is open (Esc, arrows, Enter).

use crate::appmenu::Item;
use crate::draw::Align;
use crate::input::Key;
use crate::spring::Spring;
use crate::ui::{inside, Ui};

const ROW: f32 = 28.0;
const SEP: f32 = 9.0;
const PAD: f32 = 6.0;
const RADIUS: f32 = 12.0;
const DANGER: [f32; 3] = [0.90, 0.29, 0.26];

/// What the menu did this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// still open
    Open,
    /// an entry was picked (the menu is done)
    Chosen(u32),
    /// a press outside it or Esc closed it
    Closed,
}

/// The pointer as the window had it before [`shield`] took it away.
pub struct Held {
    mouse: [f32; 2],
    click: bool,
    double: bool,
    scroll: f32,
}

/// Make the rest of the frame deaf to the pointer (the menu is over it); give the result to [`PopMenu::draw`].
pub fn shield(ui: &mut Ui<'_>) -> Held {
    let held = Held { mouse: ui.mouse, click: ui.click, double: ui.double, scroll: ui.scroll };
    ui.mouse = [-1000.0; 2];
    ui.click = false;
    ui.double = false;
    ui.scroll = 0.0;
    held
}

/// One panel's sliding highlight: where it is (y) and how strong (alpha).
struct Glide {
    y: Spring,
    a: Spring,
}

impl Glide {
    fn new() -> Glide {
        Glide { y: Spring::new(0.0, 2200.0, 1.0), a: Spring::new(0.0, 2400.0, 1.0) }
    }
}

pub struct PopMenu {
    /// where it was opened (the top-left corner, moved in when it would leave the window)
    pub at: [f32; 2],
    pub items: Vec<Item>,
    /// the submenu open at each depth: an index into that level's entries
    open: Vec<usize>,
    /// the keyboard's row in the deepest panel
    cursor: Option<usize>,
    pop: Spring,
    sub_pop: Spring,
    glides: Vec<Glide>,
    /// the panels as last drawn (a press outside all of them closes the menu)
    rects: Vec<[f32; 4]>,
    /// what a key did, reported by the next `draw`
    keyed: Option<Outcome>,
    /// the frame it opened on: the press that opened it is not a press outside it
    fresh: bool,
}

impl PopMenu {
    pub fn new(at: [f32; 2], items: Vec<Item>) -> PopMenu {
        let mut pop = Spring::new(0.0, 1100.0, 0.85);
        pop.target = 1.0;
        PopMenu { at, items, open: Vec::new(), cursor: None, pop, sub_pop: Spring::new(1.0, 1500.0, 0.8), glides: Vec::new(), rects: Vec::new(), keyed: None, fresh: true }
    }

    /// Something in it still moves: ask for another frame.
    pub fn animating(&self) -> bool {
        !self.pop.at_rest() || !self.sub_pop.at_rest() || self.glides.iter().any(|g| !g.y.at_rest() || !g.a.at_rest())
    }

    /// Is `p` on the menu (any of its panels)?
    pub fn covers(&self, p: [f32; 2]) -> bool {
        self.rects.iter().any(|r| inside(p, *r))
    }

    /// The entries of the panel at `depth` (0 = the menu itself).
    fn level(&self, depth: usize) -> &[Item] {
        let mut items: &[Item] = &self.items;
        for &i in self.open.iter().take(depth) {
            match items.get(i) {
                Some(it) if !it.sub.is_empty() => items = &it.sub,
                _ => return &[],
            }
        }
        items
    }

    fn usable(it: &Item) -> bool {
        !it.separator && it.enabled
    }

    /// A key while the menu is open. Always true: the window behind it does not act on keys meanwhile.
    pub fn key(&mut self, k: Key) -> bool {
        let depth = self.open.len();
        let n = self.level(depth).len();
        let step = |me: &PopMenu, from: Option<usize>, down: bool| -> Option<usize> {
            let items = me.level(depth);
            let mut i = from;
            for _ in 0..n {
                let next = match i {
                    None if down => 0,
                    None => n.checked_sub(1)?,
                    Some(c) if down => (c + 1) % n,
                    Some(c) => (c + n - 1) % n,
                };
                if Self::usable(&items[next]) {
                    return Some(next);
                }
                i = Some(next);
            }
            None
        };
        match k {
            Key::Esc => self.keyed = Some(Outcome::Closed),
            Key::Down => self.cursor = step(self, self.cursor, true),
            Key::Up => self.cursor = step(self, self.cursor, false),
            Key::Left => {
                if let Some(i) = self.open.pop() {
                    self.cursor = Some(i);
                } else {
                    self.keyed = Some(Outcome::Closed);
                }
            }
            Key::Right | Key::Enter => {
                if let Some(c) = self.cursor {
                    let it = self.level(depth)[c].clone();
                    if !it.sub.is_empty() {
                        self.open.push(c);
                        self.sub_pop = Spring::new(0.0, 1500.0, 0.8);
                        self.sub_pop.target = 1.0;
                        self.cursor = step(self, None, true);
                    } else if k == Key::Enter && Self::usable(&it) {
                        self.keyed = Some(Outcome::Chosen(it.id));
                    }
                }
            }
            _ => {}
        }
        true
    }

    /// The size a panel of these entries takes.
    fn panel_size(ui: &mut Ui<'_>, items: &[Item]) -> [f32; 2] {
        let mut w = 200.0f32;
        for it in items.iter().filter(|i| !i.separator) {
            let key = if !it.sub.is_empty() { 22.0 } else if it.key.is_empty() { 0.0 } else { ui.measure_raw(&it.key, 11.5, false) + 26.0 };
            w = w.max(ui.measure(&it.label, 13.0, false) + key + 54.0);
        }
        let h: f32 = items.iter().map(|i| if i.separator { SEP } else { ROW }).sum::<f32>() + 2.0 * PAD;
        [w.min(420.0), h]
    }

    /// Draw the menu over the window (`win` is its size) with the pointer [`shield`] kept for it, and handle the pointer.
    pub fn draw(&mut self, ui: &mut Ui<'_>, held: Held, win: [f32; 2]) -> Outcome {
        ui.mouse = held.mouse;
        ui.click = held.click && !self.fresh;
        ui.double = held.double;
        ui.scroll = held.scroll;
        self.fresh = false;
        let dt = ui.dt();
        self.pop.update(dt);
        self.sub_pop.update(dt);
        if let Some(o) = self.keyed.take() {
            return o;
        }
        let mut chosen = None;
        let mut rects = Vec::new();
        let mut depth = 0;
        let [sw, sh] = Self::panel_size(ui, &self.items);
        let x = self.at[0].min(win[0] - sw - 6.0).max(4.0);
        // near the bottom it opens upwards from the pointer
        let y = if self.at[1] + sh > win[1] - 6.0 { (self.at[1] - sh).max(4.0) } else { self.at[1] };
        let mut panel = [x, y, sw, sh];
        loop {
            if self.glides.len() <= depth {
                self.glides.push(Glide::new());
            }
            let items = self.level(depth).to_vec();
            if items.is_empty() {
                self.open.truncate(depth.saturating_sub(1));
                break;
            }
            let last = depth == self.open.len();
            let grow = if depth == 0 { self.pop.x } else if last { self.sub_pop.x } else { 1.0 };
            let next = self.draw_panel(ui, depth, &items, panel, grow, &mut chosen);
            rects.push(panel);
            match next {
                Some((row_y, sub)) if depth < self.open.len() => {
                    let [w, h] = Self::panel_size(ui, &sub);
                    let right = panel[0] + panel[2] - 4.0;
                    let sx = if right + w > win[0] - 4.0 { (panel[0] - w + 4.0).max(4.0) } else { right };
                    let sy = (row_y - PAD).min(win[1] - h - 4.0).max(4.0);
                    panel = [sx, sy, w, h];
                    depth += 1;
                }
                _ => break,
            }
        }
        self.rects = rects;
        if let Some(id) = chosen {
            ui.click = false;
            return Outcome::Chosen(id);
        }
        if ui.click {
            let on = self.covers(ui.mouse);
            ui.click = false;
            ui.double = false;
            if !on {
                return Outcome::Closed;
            }
        }
        Outcome::Open
    }

    /// One panel. Returns the row the open submenu hangs from (its y) and the submenu's entries.
    fn draw_panel(&mut self, ui: &mut Ui<'_>, depth: usize, items: &[Item], r: [f32; 4], grow: f32, chosen: &mut Option<u32>) -> Option<(f32, Vec<Item>)> {
        let (ink, accent, dark) = (ui.ink, ui.accent, ui.dark);
        let fade = (grow * 1.6).clamp(0.0, 1.0);
        // it grows from its corner: 92 % to full size, with the spring's slight overshoot
        let s = 0.92 + 0.08 * grow.clamp(0.0, 1.06);
        let card = [r[0], r[1], r[2] * s, r[3] * s];
        let card_bg = if dark { [0.155, 0.155, 0.165] } else { [0.985, 0.985, 0.99] };
        for (i, a) in [0.05f32, 0.035, 0.02].iter().enumerate() {
            let g = 2.0 + i as f32 * 3.0;
            ui.fill([card[0] - g, card[1] - g + 3.0, card[2] + 2.0 * g, card[3] + 2.0 * g], RADIUS + g, [0.0; 3], a * fade * if dark { 2.0 } else { 1.0 });
        }
        ui.fill([card[0] - 1.0, card[1] - 1.0, card[2] + 2.0, card[3] + 2.0], RADIUS + 1.0, ink, (if dark { 0.16 } else { 0.10 }) * fade);
        ui.fill(card, RADIUS, card_bg, 0.98 * fade);
        if fade < 0.05 {
            return None;
        }
        ui.push_clip(card);
        // a deeper submenu lying over this panel (as last drawn) takes the pointer: this one is under it there
        let covered = self.rects.iter().skip(depth + 1).take(self.open.len().saturating_sub(depth)).any(|d| inside(ui.mouse, *d));
        let mouse_here = inside(ui.mouse, r) && !covered;
        let deepest = depth == self.open.len();
        // which row the highlight is on: the pointer's, else the keyboard's, else the row whose submenu is open
        let mut target: Option<(f32, bool)> = None;
        let mut hang = None;
        let mut yy = r[1] + PAD;
        let mut rows = Vec::with_capacity(items.len());
        for (i, it) in items.iter().enumerate() {
            let h = if it.separator { SEP } else { ROW };
            rows.push([r[0] + PAD, yy, r[2] - 2.0 * PAD, h]);
            yy += h;
            let _ = i;
        }
        for (i, it) in items.iter().enumerate() {
            let row = rows[i];
            if it.separator {
                continue;
            }
            let hov = mouse_here && inside(ui.mouse, row);
            if hov {
                // the pointer decides which submenu is open from this panel
                if self.open.len() > depth && self.open[depth] != i || self.open.len() > depth + 1 {
                    self.open.truncate(depth);
                }
                if !it.sub.is_empty() && it.enabled && self.open.len() == depth {
                    self.open.push(i);
                    self.sub_pop = Spring::new(0.0, 1500.0, 0.8);
                    self.sub_pop.target = 1.0;
                } else if it.sub.is_empty() && self.open.len() > depth {
                    self.open.truncate(depth);
                }
                if deepest || self.open.len() == depth {
                    self.cursor = Some(i);
                }
                if it.enabled {
                    target = Some((row[1], it.danger));
                    if ui.click && it.sub.is_empty() {
                        *chosen = Some(it.id);
                    }
                }
            }
        }
        if target.is_none() {
            let open_here = self.open.get(depth).copied();
            let key_row = if depth == self.open.len() { self.cursor } else { None };
            if let Some(i) = key_row.or(open_here) {
                if let Some(it) = items.get(i).filter(|it| Self::usable(it)) {
                    target = Some((rows[i][1], it.danger));
                }
            }
        }
        let g = &mut self.glides[depth];
        match target {
            Some((ty, _)) => {
                if g.a.x < 0.05 {
                    g.y.x = ty;
                    g.y.v = 0.0;
                }
                g.y.target = ty;
                g.a.target = 1.0;
            }
            None => g.a.target = 0.0,
        }
        let dt = ui.dt();
        g.y.update(dt);
        g.a.update(dt);
        let (hy, ha) = (g.y.x, g.a.x.clamp(0.0, 1.0));
        let danger = target.is_some_and(|(_, d)| d);
        if ha > 0.01 {
            ui.fill([r[0] + PAD, hy, r[2] - 2.0 * PAD, ROW], 7.0, if danger { DANGER } else { accent }, 0.92 * ha * fade);
        }
        for (i, it) in items.iter().enumerate() {
            let row = rows[i];
            if it.separator {
                ui.fill([row[0] + 8.0, row[1] + SEP / 2.0, row[2] - 16.0, 1.0], 0.0, ink, 0.12 * fade);
                continue;
            }
            // the text turns white as the highlight passes under it
            let lit = if target.is_some_and(|(ty, _)| (ty - row[1]).abs() < 0.5) { ((1.0 - (hy - row[1]).abs() / ROW).clamp(0.0, 1.0)) * ha } else { 0.0 };
            let base = if it.danger && it.enabled { DANGER } else { ink };
            let c = [base[0] + (1.0 - base[0]) * lit, base[1] + (1.0 - base[1]) * lit, base[2] + (1.0 - base[2]) * lit];
            let a = if it.enabled { 0.95 } else { 0.35 } * fade;
            let cy = row[1] + ROW / 2.0;
            if it.checked {
                ui.glyph(row[0] + 13.0, cy, 13.0, "check", c, a);
            } else if !it.icon.is_empty() {
                ui.glyph(row[0] + 13.0, cy, 14.0, &it.icon, c, a * 0.9);
            }
            ui.text(row[0] + 30.0, cy + 4.5, &it.label, 13.0, false, c, a, Align::Left);
            if !it.sub.is_empty() {
                ui.glyph(row[0] + row[2] - 12.0, cy, 13.0, "chevron-right", c, a * 0.8);
                if self.open.get(depth) == Some(&i) {
                    hang = Some((row[1], it.sub.clone()));
                }
            } else if !it.key.is_empty() {
                ui.text_raw(row[0] + row[2] - 10.0, cy + 4.5, &it.key, 11.5, false, c, a * 0.55, Align::Right);
            }
        }
        ui.pop_clip();
        hang
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::Draw;
    use crate::style::Style;

    /// A painter that draws nothing and measures every letter 7 px wide.
    struct Null;
    impl Draw for Null {
        fn fill(&mut self, _: [f32; 4], _: f32, _: [f32; 3], _: f32) {}
        fn grad(&mut self, _: [f32; 4], _: f32, _: [f32; 3], _: [f32; 3], _: f32) {}
        fn glyph(&mut self, _: f32, _: f32, _: f32, _: &str, _: [f32; 3], _: f32) {}
        fn text(&mut self, _: f32, _: f32, _: &str, _: f32, _: bool, _: [f32; 3], _: f32, _: Align) {}
        fn measure(&mut self, t: &str, _: f32, _: bool) -> f32 {
            t.chars().count() as f32 * 7.0
        }
    }

    fn frame(state: &mut crate::ui::State, menu: &mut PopMenu) -> Outcome {
        let mut d = Null;
        let dt = state.begin_frame();
        let mut ui = Ui::new(&mut d, state, Style::new(false, [0.2, 0.5, 1.0]), dt.max(0.016));
        let held = shield(&mut ui);
        let out = menu.draw(&mut ui, held, [800.0, 600.0]);
        let seen = ui.edit_seen;
        state.end_frame(seen);
        state.click = false;
        out
    }

    fn items() -> Vec<Item> {
        vec![Item::new(1, "Open"), Item::sep(), Item::parent("Sort by", vec![Item::new(10, "Name"), Item::new(11, "Size")]), Item::new(2, "Move to Trash").danger(), Item::new(3, "Gone").enabled(false)]
    }

    #[test]
    fn a_click_on_an_entry_picks_it_and_outside_closes() {
        let mut st = crate::ui::State::default();
        let mut m = PopMenu::new([100.0, 100.0], items());
        assert_eq!(frame(&mut st, &mut m), Outcome::Open);
        // the first row sits PAD below the top
        st.pointer_down([150.0, 100.0 + PAD + ROW / 2.0]);
        st.pointer_up();
        assert_eq!(frame(&mut st, &mut m), Outcome::Chosen(1));
        let mut m = PopMenu::new([100.0, 100.0], items());
        frame(&mut st, &mut m);
        st.pointer_down([700.0, 500.0]);
        st.pointer_up();
        assert_eq!(frame(&mut st, &mut m), Outcome::Closed);
    }

    #[test]
    fn hovering_a_parent_opens_its_submenu_and_keys_walk_the_rows() {
        let mut st = crate::ui::State::default();
        let mut m = PopMenu::new([100.0, 100.0], items());
        frame(&mut st, &mut m);
        let sort_y = 100.0 + PAD + ROW + SEP + ROW / 2.0;
        st.pointer_move([150.0, sort_y]);
        frame(&mut st, &mut m);
        assert_eq!(m.open, vec![2]);
        assert_eq!(m.rects.len(), 2, "the submenu is drawn beside its row in the same frame");
        // moving onto a plain entry closes it again
        st.pointer_move([150.0, 100.0 + PAD + ROW / 2.0]);
        frame(&mut st, &mut m);
        assert!(m.open.is_empty() && m.rects.len() == 1);
        // the keyboard: into the submenu, down to Size, Enter
        let mut m = PopMenu::new([100.0, 100.0], items());
        m.key(Key::Down);
        m.key(Key::Down);
        assert_eq!(m.cursor, Some(2), "the separator is skipped");
        m.key(Key::Right);
        m.key(Key::Down);
        m.key(Key::Enter);
        assert_eq!(frame(&mut st, &mut m), Outcome::Chosen(11));
        // the disabled entry is never the cursor's
        let mut m = PopMenu::new([100.0, 100.0], items());
        m.key(Key::Up);
        assert_eq!(m.cursor, Some(3));
    }

    #[test]
    fn a_submenu_over_another_panel_keeps_the_pointer() {
        // at the right edge: Settings opens to the left, its Preset opens to the right again, over the first panel
        let preset = Item::parent("Preset", vec![Item::new(20, "Dolphin"), Item::new(21, "Icons"), Item::new(22, "Details")]);
        let settings = Item::parent("Settings", vec![preset, Item::new(30, "Sidebar")]);
        let list = vec![Item::new(1, "Zoom In"), Item::new(2, "Zoom Out"), Item::new(3, "Show Places Panel"), Item::new(4, "Show Status Bar"), settings, Item::new(5, "Terminal")];
        let mut st = crate::ui::State::default();
        let mut m = PopMenu::new([790.0, 100.0], list);
        frame(&mut st, &mut m);
        let r0 = m.rects[0];
        st.pointer_move([r0[0] + 30.0, r0[1] + PAD + 4.0 * ROW + ROW / 2.0]);
        frame(&mut st, &mut m);
        assert_eq!(m.open, vec![4]);
        let r1 = m.rects[1];
        st.pointer_move([r1[0] + 30.0, r1[1] + PAD + ROW / 2.0]);
        frame(&mut st, &mut m);
        assert_eq!(m.open, vec![4, 0]);
        let r2 = m.rects[2];
        assert!(r2[0] < r0[0] + r0[2] && r2[0] + r2[2] > r0[0], "the third panel lies over the first one");
        // the pointer on "Icons" (over a row of the first panel) stays in the submenu and picks it
        let x = (r0[0].max(r2[0]) + (r0[0] + r0[2]).min(r2[0] + r2[2])) / 2.0;
        let p = [x, r2[1] + PAD + ROW + ROW / 2.0];
        assert!(inside(p, r0), "{r0:?} {r1:?} {r2:?} {p:?}");
        st.pointer_move(p);
        frame(&mut st, &mut m);
        assert_eq!(m.open, vec![4, 0], "the first panel did not take the pointer");
        st.pointer_down(p);
        st.pointer_up();
        assert_eq!(frame(&mut st, &mut m), Outcome::Chosen(21));
    }
}
