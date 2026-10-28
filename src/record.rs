//! Retained frames for an immediate-mode UI: record what the widgets draw, compare it with the last frame and
//! paint only what changed.
//!
//! A [`Recorder`] is a [`Draw`] that stores every call as a [`Cmd`]. A [`Frame`] keeps the pixels of the last frame
//! and its command list. [`Frame::update`] finds the commands that differ (the common head and tail of the two lists
//! are the same, so what lies between them is the change), clears the rectangle they touch and paints again the
//! commands that reach into it, clipped to that rectangle. The result is pixel-exact, and a frame in which nothing
//! changed (the pointer moved over a gap, a hover that looks the same) costs no painting at all.

use crate::canvas::Canvas;
use crate::draw::{Align, Draw};
use crate::soft::{Gfx, SoftDraw};

#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    Fill { r: [f32; 4], radius: f32, c: [f32; 3], alpha: f32 },
    Grad { r: [f32; 4], radius: f32, top: [f32; 3], bottom: [f32; 3], alpha: f32 },
    Glyph { cx: f32, cy: f32, size: f32, icon: String, c: [f32; 3], alpha: f32 },
    Text { x: f32, y: f32, text: String, size: f32, bold: bool, c: [f32; 3], alpha: f32, align: Align },
    /// limit the commands up to the matching `PopClip` to a box
    PushClip { r: [f32; 4] },
    PopClip,
    /// glass reads the pixels around it, so it is repainted whenever anything under or near it is
    Glass { r: [f32; 4], radius: f32, strength: f32, dark: bool },
    /// a picture: the same one (by id) at the same place is the same command, so a still picture costs nothing to keep
    Picture { r: [f32; 4], pic: std::sync::Arc<crate::picture::Pic>, src: [f32; 4], alpha: f32 },
}

/// A painter that only remembers what it is asked to draw.
pub struct Recorder<'a> {
    gfx: &'a mut Gfx,
    scale: f32,
    pub cmds: Vec<Cmd>,
}

impl<'a> Recorder<'a> {
    pub fn new(gfx: &'a mut Gfx, scale: f32) -> Self {
        Recorder { gfx, scale, cmds: Vec::with_capacity(256) }
    }
}

impl Draw for Recorder<'_> {
    fn fill(&mut self, r: [f32; 4], radius: f32, c: [f32; 3], alpha: f32) {
        self.cmds.push(Cmd::Fill { r, radius, c, alpha });
    }

    fn grad(&mut self, r: [f32; 4], radius: f32, top: [f32; 3], bottom: [f32; 3], alpha: f32) {
        self.cmds.push(Cmd::Grad { r, radius, top, bottom, alpha });
    }

    fn glyph(&mut self, cx: f32, cy: f32, size: f32, icon: &str, c: [f32; 3], alpha: f32) {
        self.cmds.push(Cmd::Glyph { cx, cy, size, icon: icon.to_string(), c, alpha });
    }

    fn text(&mut self, x: f32, y: f32, text: &str, size: f32, bold: bool, c: [f32; 3], alpha: f32, align: Align) {
        self.cmds.push(Cmd::Text { x, y, text: text.to_string(), size, bold, c, alpha, align });
    }

    fn measure(&mut self, text: &str, size: f32, bold: bool) -> f32 {
        self.gfx.text_width(text, size * self.scale, bold) / self.scale
    }

    fn picture(&mut self, r: [f32; 4], pic: &std::sync::Arc<crate::picture::Pic>, src: [f32; 4], alpha: f32) {
        self.cmds.push(Cmd::Picture { r, pic: pic.clone(), src, alpha });
    }

    fn glass(&mut self, r: [f32; 4], radius: f32, strength: f32, dark: bool) {
        self.cmds.push(Cmd::Glass { r, radius, strength, dark });
    }

    fn push_clip(&mut self, r: [f32; 4]) {
        self.cmds.push(Cmd::PushClip { r });
    }

    fn pop_clip(&mut self) {
        self.cmds.push(Cmd::PopClip);
    }
}

type Box4 = [i32; 4];

fn union(a: Option<Box4>, b: Box4) -> Option<Box4> {
    Some(match a {
        None => b,
        Some(a) => [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])],
    })
}

fn touches(a: Box4, b: Box4) -> bool {
    a[0] < b[2] && a[2] > b[0] && a[1] < b[3] && a[3] > b[1]
}

/// The pixels a command can change (physical px, with a margin for anti-aliasing).
fn bounds(cmd: &Cmd, gfx: &mut Gfx, s: f32) -> Box4 {
    let rect = |r: &[f32; 4]| [(r[0] * s).floor() as i32 - 2, (r[1] * s).floor() as i32 - 2, ((r[0] + r[2]) * s).ceil() as i32 + 2, ((r[1] + r[3]) * s).ceil() as i32 + 2];
    match cmd {
        Cmd::Fill { r, .. } | Cmd::Grad { r, .. } | Cmd::Picture { r, .. } => rect(r),
        // the blur reads a margin around the panel
        Cmd::Glass { r, .. } => {
            let m = 12.0 * s;
            [(r[0] * s - m).floor() as i32, (r[1] * s - m).floor() as i32, ((r[0] + r[2]) * s + m).ceil() as i32, ((r[1] + r[3]) * s + m).ceil() as i32]
        }
        Cmd::PushClip { r } => [(r[0] * s).floor() as i32, (r[1] * s).floor() as i32, ((r[0] + r[2]) * s).ceil() as i32, ((r[1] + r[3]) * s).ceil() as i32],
        Cmd::PopClip => [0, 0, 0, 0],
        Cmd::Glyph { cx, cy, size, .. } => {
            let half = (size * s).round().max(1.0) / 2.0 + 2.0;
            [(cx * s - half).floor() as i32, (cy * s - half).floor() as i32, (cx * s + half).ceil() as i32, (cy * s + half).ceil() as i32]
        }
        Cmd::Text { x, y, text, size, bold, align, .. } => {
            let r = gfx.line(text, size * s, *bold);
            let ox = match align {
                Align::Left => x * s,
                Align::Center => x * s - r.width / 2.0,
                Align::Right => x * s - r.width,
            };
            let top = y * s - r.ascent;
            [ox.round() as i32 - 2, top.round() as i32 - 2, ox.round() as i32 + r.w as i32 + 2, top.round() as i32 + r.h as i32 + 2]
        }
    }
}

fn paint(cmd: &Cmd, d: &mut SoftDraw) {
    match cmd {
        Cmd::Fill { r, radius, c, alpha } => d.fill(*r, *radius, *c, *alpha),
        Cmd::Grad { r, radius, top, bottom, alpha } => d.grad(*r, *radius, *top, *bottom, *alpha),
        Cmd::Glyph { cx, cy, size, icon, c, alpha } => d.glyph(*cx, *cy, *size, icon, *c, *alpha),
        Cmd::Text { x, y, text, size, bold, c, alpha, align } => d.text(*x, *y, text, *size, *bold, *c, *alpha, *align),
        Cmd::Picture { r, pic, src, alpha } => d.picture(*r, pic, *src, *alpha),
        Cmd::Glass { r, radius, strength, dark } => d.glass(*r, *radius, *strength, *dark),
        Cmd::PushClip { r } => d.push_clip(*r),
        Cmd::PopClip => d.pop_clip(),
    }
}

fn intersect(a: Box4, b: Box4) -> Box4 {
    [a[0].max(b[0]), a[1].max(b[1]), a[2].min(b[2]), a[3].min(b[3])]
}

/// Each command with the pixels it can really change: its own bounds cut by the clips in force around it.
fn effective(cmds: &[Cmd], gfx: &mut Gfx, scale: f32) -> Vec<Box4> {
    let mut stack: Vec<Box4> = Vec::new();
    cmds.iter()
        .map(|c| match c {
            Cmd::PushClip { .. } => {
                let b = bounds(c, gfx, scale);
                let b = stack.last().map_or(b, |top| intersect(b, *top));
                stack.push(b);
                b
            }
            Cmd::PopClip => {
                stack.pop();
                [0, 0, 0, 0]
            }
            _ => {
                let b = bounds(c, gfx, scale);
                stack.last().map_or(b, |top| intersect(b, *top))
            }
        })
        .collect()
}

/// The pixels of the last frame and the commands that made them.
#[derive(Default)]
pub struct Frame {
    pub w: usize,
    pub h: usize,
    pub data: Vec<u8>,
    /// the last frame's commands, each with the box it could change
    prev: Vec<(Cmd, Box4)>,
    /// what the pixels were cleared to, so a changed background repaints everything
    bg: Option<([u8; 3], u8)>,
    valid: bool,
}

impl Frame {
    /// A new size (or scale): the pixels are gone, the next update paints everything.
    pub fn resize(&mut self, w: usize, h: usize) {
        if (w, h) != (self.w, self.h) {
            self.w = w;
            self.h = h;
            self.data = vec![0; w * h * 4];
            self.prev.clear();
            self.valid = false;
        }
    }

    /// Forget the last frame (the next update paints everything).
    pub fn invalidate(&mut self) {
        self.valid = false;
    }

    /// Bring the pixels up to date with `cmds`. Returns the box that changed (x0, y0, x1, y1), or `None` when the
    /// frame is the same as the last one and there is nothing to show.
    pub fn update(&mut self, cmds: Vec<Cmd>, gfx: &mut Gfx, bg: ([f32; 3], f32), scale: f32) -> Option<Box4> {
        let bg_key = (crate::canvas::rgb8(bg.0), (bg.1 * 255.0) as u8);
        let full = [0, 0, self.w as i32, self.h as i32];
        let boxes = effective(&cmds, gfx, scale);
        let items: Vec<(Cmd, Box4)> = cmds.into_iter().zip(boxes).collect();
        let damage = if !self.valid || self.bg != Some(bg_key) {
            full
        } else {
            // a command is the same only if it is the same and sits in the same clip
            let head = self.prev.iter().zip(&items).take_while(|(a, b)| a == b).count();
            let tail = self.prev[head..].iter().rev().zip(items[head..].iter().rev()).take_while(|(a, b)| a == b).count();
            let (old, new) = (&self.prev[head..self.prev.len() - tail], &items[head..items.len() - tail]);
            if old.is_empty() && new.is_empty() {
                return None;
            }
            let mut dmg = None;
            for (_, b) in old.iter().chain(new) {
                if b[0] < b[2] && b[1] < b[3] {
                    dmg = union(dmg, *b);
                }
            }
            match dmg {
                Some(d) => [d[0].max(0), d[1].max(0), d[2].min(self.w as i32), d[3].min(self.h as i32)],
                // only clip commands without area changed: nothing is drawn differently
                None => {
                    self.prev = items;
                    return None;
                }
            }
        };
        if damage[0] >= damage[2] || damage[1] >= damage[3] {
            self.prev = items;
            return None;
        }
        // glass reads pixels around it, so a panel touched by the damage is repainted whole (else its unrepainted part
        // would be blurred twice); growing the box can reach another panel, hence the loop
        let mut damage = damage;
        loop {
            let mut grown = damage;
            for (c, b) in &items {
                if matches!(c, Cmd::Glass { .. }) && touches(*b, damage) {
                    grown = [grown[0].min(b[0].max(0)), grown[1].min(b[1].max(0)), grown[2].max(b[2].min(self.w as i32)), grown[3].max(b[3].min(self.h as i32))];
                }
            }
            if grown == damage {
                break;
            }
            damage = grown;
        }
        let mut canvas = Canvas::new(&mut self.data, self.w, self.h);
        canvas.clear_rect(damage, bg_key.0, bg.1);
        let mut painter = SoftDraw::new(&mut canvas, gfx).scaled(scale);
        // every command that reaches into the box is painted again, in order, so the result is exact; the clips are
        // replayed as they were, inside the box
        let mut clips: Vec<Box4> = Vec::new();
        for (c, b) in &items {
            match c {
                Cmd::PushClip { .. } => {
                    let cut = intersect(*b, damage);
                    clips.push(cut);
                    painter.canvas.set_clip(cut);
                }
                Cmd::PopClip => {
                    clips.pop();
                    painter.canvas.set_clip(clips.last().copied().unwrap_or(damage));
                }
                _ => {
                    if touches(*b, damage) {
                        if clips.is_empty() {
                            painter.canvas.set_clip(damage);
                        }
                        paint(c, &mut painter);
                    }
                }
            }
        }
        self.prev = items;
        self.bg = Some(bg_key);
        self.valid = true;
        Some(damage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame_of(f: impl Fn(&mut Recorder), gfx: &mut Gfx) -> Vec<Cmd> {
        let mut r = Recorder::new(gfx, 1.0);
        f(&mut r);
        r.cmds
    }

    fn scene(hover: bool, label: &str) -> impl Fn(&mut Recorder) + '_ {
        move |d| {
            d.fill([0.0, 0.0, 200.0, 40.0], 0.0, [0.2, 0.2, 0.3], 1.0);
            d.fill([10.0, 50.0, 100.0, 30.0], 8.0, [1.0; 3], if hover { 0.2 } else { 0.06 });
            d.text(20.0, 70.0, label, 13.0, false, [1.0; 3], 1.0, Align::Left);
            d.fill([0.0, 100.0, 200.0, 40.0], 0.0, [0.2, 0.2, 0.3], 1.0);
        }
    }

    fn pixel(buf: &[u8], w: usize, x: usize, y: usize) -> [u8; 4] {
        let o = (y * w + x) * 4;
        [buf[o], buf[o + 1], buf[o + 2], buf[o + 3]]
    }

    const BG: ([f32; 3], f32) = ([0.1, 0.1, 0.15], 1.0);

    /// Paint `cmds` from scratch on a fresh canvas.
    fn from_scratch(cmds: Vec<Cmd>, gfx: &mut Gfx) -> Vec<u8> {
        let mut f = Frame::default();
        f.resize(200, 150);
        f.update(cmds, gfx, BG, 1.0);
        f.data
    }

    #[test]
    fn an_identical_frame_paints_nothing() {
        let mut gfx = Gfx::new();
        let mut f = Frame::default();
        f.resize(200, 150);
        assert!(f.update(frame_of(scene(false, "Name"), &mut gfx), &mut gfx, BG, 1.0).is_some());
        assert_eq!(f.update(frame_of(scene(false, "Name"), &mut gfx), &mut gfx, BG, 1.0), None);
    }

    #[test]
    fn only_the_changed_part_is_repainted_and_the_result_is_exact() {
        let mut gfx = Gfx::new();
        let mut f = Frame::default();
        f.resize(200, 150);
        f.update(frame_of(scene(false, "Name"), &mut gfx), &mut gfx, BG, 1.0);
        // the hover wash changes: the damage is that row, not the bars above and below
        let dmg = f.update(frame_of(scene(true, "Name"), &mut gfx), &mut gfx, BG, 1.0).expect("something changed");
        assert!(dmg[1] >= 40 && dmg[3] <= 100, "damage {dmg:?} spills out of the row");
        assert_eq!(f.data, from_scratch(frame_of(scene(true, "Name"), &mut gfx), &mut gfx));
        // the text changes: again exact
        f.update(frame_of(scene(true, "Another name"), &mut gfx), &mut gfx, BG, 1.0);
        assert_eq!(f.data, from_scratch(frame_of(scene(true, "Another name"), &mut gfx), &mut gfx));
    }

    #[test]
    fn a_new_size_or_background_repaints_everything() {
        let mut gfx = Gfx::new();
        let mut f = Frame::default();
        f.resize(200, 150);
        f.update(frame_of(scene(false, "Name"), &mut gfx), &mut gfx, BG, 1.0);
        let other = ([0.9, 0.9, 0.9], 1.0);
        assert_eq!(f.update(frame_of(scene(false, "Name"), &mut gfx), &mut gfx, other, 1.0), Some([0, 0, 200, 150]));
        f.resize(220, 150);
        assert_eq!(f.update(frame_of(scene(false, "Name"), &mut gfx), &mut gfx, other, 1.0), Some([0, 0, 220, 150]));
    }

    #[test]
    fn a_clip_keeps_a_row_inside_its_box_and_diffs_exactly() {
        let mut gfx = Gfx::new();
        let list = |hot: bool| {
            move |d: &mut Recorder| {
                d.fill([0.0, 0.0, 200.0, 150.0], 0.0, [0.2, 0.2, 0.3], 1.0);
                d.push_clip([0.0, 40.0, 200.0, 60.0]);
                // a row that sticks out above and below the box
                d.fill([10.0, 20.0, 100.0, 120.0], 6.0, [1.0, 0.4, 0.4], if hot { 0.9 } else { 0.5 });
                d.pop_clip();
            }
        };
        let a = from_scratch(frame_of(list(false), &mut gfx), &mut gfx);
        // nothing leaks out of the clip
        assert_eq!(pixel(&a, 200, 50, 30), pixel(&a, 200, 150, 30), "above the box the background is untouched");
        assert_ne!(pixel(&a, 200, 50, 60), pixel(&a, 200, 150, 60), "inside the box the row is drawn");
        let mut f = Frame::default();
        f.resize(200, 150);
        f.update(frame_of(list(false), &mut gfx), &mut gfx, BG, 1.0);
        let dmg = f.update(frame_of(list(true), &mut gfx), &mut gfx, BG, 1.0).expect("the row changed");
        assert!(dmg[1] >= 40 && dmg[3] <= 100, "damage {dmg:?} is the clipped part, not the whole row");
        assert_eq!(f.data, from_scratch(frame_of(list(true), &mut gfx), &mut gfx));
    }

    #[test]
    fn glass_over_a_changing_backdrop_diffs_exactly_and_really_blurs() {
        let mut gfx = Gfx::new();
        let scene = |shift: f32| {
            move |d: &mut Recorder| {
                d.fill([0.0, 0.0, 200.0, 150.0], 0.0, [0.1, 0.1, 0.15], 1.0);
                // hard stripes under the panel: a blur must soften them
                for k in 0..10 {
                    d.fill([k as f32 * 20.0 + shift, 0.0, 10.0, 150.0], 0.0, [1.0, 0.3, 0.2], 1.0);
                }
                d.glass([30.0, 50.0, 140.0, 40.0], 16.0, 1.0, true);
            }
        };
        let mut f = Frame::default();
        f.resize(200, 150);
        f.update(frame_of(scene(0.0), &mut gfx), &mut gfx, BG, 1.0);
        // a stripe moves under the panel: only part of the panel is dirty, yet the result must equal a fresh paint
        f.update(frame_of(scene(3.0), &mut gfx), &mut gfx, BG, 1.0).expect("changed");
        assert_eq!(f.data, from_scratch(frame_of(scene(3.0), &mut gfx), &mut gfx));
        // inside the panel the stripes are smoothed: neighbouring pixels differ far less than the 255 step of a stripe edge
        let (a, b) = (pixel(&f.data, 200, 98, 70), pixel(&f.data, 200, 99, 70));
        assert!((a[2] as i32 - b[2] as i32).abs() < 20, "{a:?} {b:?}");
    }
}
