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
    /// the glass is made by the graphics card, over its picture: only the shadow around it is painted here
    GlassShadow { r: [f32; 4], radius: f32, strength: f32 },
    /// a picture: the same one (by id) at the same place is the same command, so a still picture costs nothing to keep
    Picture { r: [f32; 4], pic: std::sync::Arc<crate::picture::Pic>, src: [f32; 4], alpha: f32 },
}

/// A panel of glass the graphics card draws over its picture (`GpuLayer`): where, how round, how strong, in the dark look.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlassRect {
    pub r: [f32; 4],
    pub radius: f32,
    pub strength: f32,
    pub dark: bool,
    /// the theme's glass (`Appearance::glass_look`), its tint for this look and its border
    pub look: [f32; 7],
    pub tint: [f32; 4],
    pub border: [f32; 4],
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
    fn device_scale(&self) -> f32 {
        self.scale
    }
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
        // (the shadow reaches a few pixels beyond the panel, and moves with it: painted over the old shadow's place)
        Cmd::GlassShadow { r, .. } => {
            let m = 10.0 * s;
            [(r[0] * s - m).floor() as i32, (r[1] * s - m).floor() as i32, ((r[0] + r[2]) * s + m).ceil() as i32, ((r[1] + r[3]) * s + m).ceil() as i32]
        }
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
        Cmd::GlassShadow { r, radius, strength } => d.glass_shadow(*r, *radius, *strength),
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
    /// the pixels, when `update` keeps them here (the window runtime paints straight into its buffers instead, see
    /// `update_into`, and leaves this empty)
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
            // made when `update` needs it: a runtime that paints into its own buffers never does
            self.data = Vec::new();
            self.prev.clear();
            self.valid = false;
        }
    }

    /// Forget the last frame (the next update paints everything).
    pub fn invalidate(&mut self) {
        self.valid = false;
    }

    /// The pixels hold the last frame (false after `invalidate`, a new size, or a buffer `update_into` could not get).
    pub fn is_valid(&self) -> bool {
        self.valid
    }

    /// Bring the pixels up to date with `cmds`. Returns the box that changed (x0, y0, x1, y1; all the boxes repainted
    /// together), or `None` when the frame is the same as the last one and there is nothing to show.
    pub fn update(&mut self, cmds: Vec<Cmd>, gfx: &mut Gfx, bg: ([f32; 3], f32), scale: f32) -> Option<Box4> {
        let mut data = std::mem::take(&mut self.data);
        if data.len() != self.w * self.h * 4 {
            data = vec![0; self.w * self.h * 4];
            self.valid = false;
        }
        let changed = self.update_into(cmds, gfx, bg, scale, |_| Some(&mut data[..]));
        self.data = data;
        changed.and_then(|boxes| boxes.into_iter().reduce(|a, b| union(Some(a), b).expect("some")))
    }

    /// `update`, painting into pixels the caller keeps, and telling the boxes that changed apart: two changes far from
    /// each other (a bar at the top and a pill at the bottom fading in together) are two small boxes, not one that spans
    /// the window. Once the boxes are known, `target` is asked for a `w` x `h` buffer that holds the last frame's pixels
    /// everywhere outside them (the window runtime brings one of its shared-memory buffers up to date and hands it
    /// over, so there is no separate copy of the window to keep). It is not called when nothing changed; when it gives
    /// no buffer the frame is forgotten (the next update paints everything) and `None` is returned.
    pub fn update_into<'t>(&mut self, cmds: Vec<Cmd>, gfx: &mut Gfx, bg: ([f32; 3], f32), scale: f32, target: impl FnOnce(&[Box4]) -> Option<&'t mut [u8]>) -> Option<Vec<Box4>> {
        let bg_key = (crate::canvas::rgb8(bg.0), (bg.1 * 255.0) as u8);
        let (w, h) = (self.w as i32, self.h as i32);
        let clamp = |b: Box4| [b[0].max(0), b[1].max(0), b[2].min(w), b[3].min(h)];
        let boxes = effective(&cmds, gfx, scale);
        let items: Vec<(Cmd, Box4)> = cmds.into_iter().zip(boxes).collect();
        let mut damage = if !self.valid || self.bg != Some(bg_key) {
            vec![[0, 0, w, h]]
        } else {
            // a command is the same only if it is the same and sits in the same clip
            let head = self.prev.iter().zip(&items).take_while(|(a, b)| a == b).count();
            let tail = self.prev[head..].iter().rev().zip(items[head..].iter().rev()).take_while(|(a, b)| a == b).count();
            let (old, new) = (&self.prev[head..self.prev.len() - tail], &items[head..items.len() - tail]);
            if old.is_empty() && new.is_empty() {
                return None;
            }
            // only clip commands without area may have changed: then nothing is drawn differently
            group(old.iter().chain(new).map(|(_, b)| clamp(*b)).filter(|b| b[0] < b[2] && b[1] < b[3]))
        };
        if damage.is_empty() {
            self.prev = items;
            return None;
        }
        // glass reads pixels around it, so a panel touched by a box is repainted whole (else its unrepainted part would
        // be blurred twice); growing a box can reach another panel or another box, hence the loop
        loop {
            let mut grew = false;
            for d in damage.iter_mut() {
                for (c, b) in &items {
                    if matches!(c, Cmd::Glass { .. }) && touches(*b, *d) {
                        let g = union(Some(*d), clamp(*b)).expect("some");
                        grew |= g != *d;
                        *d = g;
                    }
                }
            }
            if !grew && !merge_touching(&mut damage) {
                break;
            }
        }
        let Some(data) = target(&damage).filter(|d| d.len() >= self.w * self.h * 4) else {
            self.valid = false;
            return None;
        };
        let mut canvas = Canvas::new(data, self.w, self.h);
        for d in &damage {
            paint_box(&items, &mut canvas, gfx, *d, (bg_key.0, bg.1), scale);
        }
        self.prev = items;
        self.bg = Some(bg_key);
        self.valid = true;
        Some(damage)
    }
}

/// Clear the box `damage` and paint again, in order, every command that reaches into it, clipped to it: the pixels come
/// out exactly as a paint of the whole frame would leave them there.
fn paint_box(items: &[(Cmd, Box4)], canvas: &mut Canvas, gfx: &mut Gfx, damage: Box4, bg: (crate::canvas::Rgb, f32), scale: f32) {
    let (bg_rgb, bg_a) = bg;
    // The box starts as the background, then the commands are painted over it. Panes that come first and cover the
    // box with one flat colour (the content pane under a list being scrolled) are worked out on one pixel instead:
    // their flat part `flat` is cleared straight to the result, and they are painted only in the strips left and
    // right of it. Same bytes, but the pixels are written once rather than once per pane.
    let bg_px = crate::canvas::clear_pixel(bg_rgb, bg_a);
    let (mut ground, mut flat, mut folded) = (bg_px, [damage[0], damage[2]], 0);
    for (i, (c, b)) in items.iter().enumerate() {
        match c {
            Cmd::PushClip { .. } | Cmd::PopClip => break,
            _ if !touches(*b, damage) => continue,
            Cmd::Fill { r, radius, c, alpha } => {
                let r = r.map(|v| v * scale);
                let Some((x0, x1, row_a)) = crate::canvas::flat_part(r[0], r[1], r[2], r[3], radius * scale, *alpha, damage) else { break };
                let (x0, x1) = (x0.max(flat[0]), x1.min(flat[1]));
                if x0 >= x1 {
                    break;
                }
                flat = [x0, x1];
                ground = crate::canvas::flat_pixel(crate::canvas::rgb8(*c), row_a, ground);
                folded = i + 1;
            }
            _ => break,
        }
    }
    let strips = [[damage[0], damage[1], flat[0], damage[3]], [flat[1], damage[1], damage[2], damage[3]]];
    canvas.clear_rect_to([flat[0], damage[1], flat[1], damage[3]], ground);
    for s in strips {
        canvas.clear_rect_to(s, bg_px);
    }
    if folded > 0 {
        let mut painter = SoftDraw::new(&mut *canvas, gfx).scaled(scale);
        for s in strips.into_iter().filter(|s| s[0] < s[2]) {
            for (c, b) in &items[..folded] {
                if touches(*b, s) {
                    painter.canvas.set_clip(s);
                    paint(c, &mut painter);
                }
            }
        }
    }
    let mut painter = SoftDraw::new(&mut *canvas, gfx).scaled(scale);
    // every command that reaches into the box is painted again, in order, so the result is exact; the clips are
    // replayed as they were, inside the box
    let mut clips: Vec<Box4> = Vec::new();
    for (c, b) in &items[folded..] {
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
}

/// The most boxes a frame is repainted in; more are joined (the ones that cost least to join first).
const MAX_BOXES: usize = 4;

fn area(b: Box4) -> i64 {
    (b[2] - b[0]).max(0) as i64 * (b[3] - b[1]).max(0) as i64
}

/// What joining two boxes into their bounding box costs: the pixels it adds that neither covers (negative when they
/// overlap).
fn waste(a: Box4, b: Box4) -> i64 {
    area(union(Some(a), b).expect("some")) - area(a) - area(b)
}

/// Gather the boxes of the changed commands into a few boxes to repaint: a box joins the group it is near (joining
/// would add little that is unchanged), groups far apart stay apart.
fn group(boxes: impl Iterator<Item = Box4>) -> Vec<Box4> {
    let near = |a: Box4, b: Box4| waste(a, b) <= (area(a) + area(b)) / 4 + 64 * 64;
    let mut out: Vec<Box4> = Vec::new();
    for b in boxes {
        match out.iter().enumerate().map(|(i, c)| (i, waste(*c, b))).min_by_key(|x| x.1) {
            Some((i, _)) if near(out[i], b) => out[i] = union(Some(out[i]), b).expect("some"),
            _ => out.push(b),
        }
    }
    // groups that grew into each other's neighbourhood join; then the cheapest pairs until few are left
    loop {
        let mut best: Option<(usize, usize, i64)> = None;
        for i in 0..out.len() {
            for j in i + 1..out.len() {
                let w = waste(out[i], out[j]);
                if best.is_none_or(|(_, _, bw)| w < bw) {
                    best = Some((i, j, w));
                }
            }
        }
        match best {
            Some((i, j, _)) if out.len() > MAX_BOXES || near(out[i], out[j]) || touches(out[i], out[j]) => {
                let b = out.swap_remove(j);
                out[i] = union(Some(out[i]), b).expect("some");
            }
            _ => break,
        }
    }
    out
}

/// Join boxes that overlap (a pixel must not be cleared after another box painted it). True when any were joined.
fn merge_touching(boxes: &mut Vec<Box4>) -> bool {
    let mut merged = false;
    'again: loop {
        for i in 0..boxes.len() {
            for j in i + 1..boxes.len() {
                if touches(boxes[i], boxes[j]) {
                    let b = boxes.swap_remove(j);
                    boxes[i] = union(Some(boxes[i]), b).expect("some");
                    merged = true;
                    continue 'again;
                }
            }
        }
        return merged;
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

    /// A window like the file manager's at 1.25x: a translucent pane, a glass sidebar, a toolbar, and a list of rows
    /// with an icon and three columns of text. `hover` is the row under the pointer, `scroll` the list's offset.
    fn files_like(d: &mut Recorder, hover: usize, scroll: f32) {
        let (w, h) = (1200.0, 775.0);
        d.fill([0.0, 0.0, w, h], 14.0, [0.97, 0.97, 0.98], 0.92);
        // the paper of the content, as the file manager lays it next to its sidebar
        d.fill([240.0, 0.0, w - 240.0, h], 0.0, [1.0; 3], 0.78);
        d.fill([12.0, 12.0, 220.0, h - 24.0], 12.0, [0.6, 0.65, 0.75], 0.5);
        for i in 0..10 {
            let y = 70.0 + i as f32 * 36.0;
            d.glyph(40.0, y, 16.0, "folder", [0.9, 0.5, 0.2], 1.0);
            d.text(60.0, y + 5.0, ["Home", "Desktop", "Documents", "Downloads", "Pictures", "Music", "Videos", "Trash", "This computer", "Data"][i], 14.0, false, [0.1; 3], 0.9, Align::Left);
        }
        d.grad([250.0, 0.0, w - 250.0, 60.0], 0.0, [1.0; 3], [0.95; 3], 0.8);
        d.text(300.0, 30.0, "mykhailo", 15.0, true, [0.1; 3], 1.0, Align::Left);
        d.push_clip([250.0, 110.0, w - 250.0, h - 140.0]);
        for i in 0..40 {
            let y = 110.0 + i as f32 * 32.0 - scroll;
            if y < 80.0 || y > h {
                continue;
            }
            if i % 2 == 1 {
                d.fill([260.0, y, w - 280.0, 30.0], 6.0, [0.5; 3], 0.04);
            }
            if i == hover {
                d.fill([260.0, y, w - 280.0, 30.0], 6.0, [0.2, 0.5, 1.0], 0.12);
            }
            d.glyph(285.0, y + 15.0, 18.0, "folder", [0.9, 0.5, 0.2], 1.0);
            d.text(310.0, y + 20.0, &format!("Folder number {i}"), 14.0, false, [0.1; 3], 1.0, Align::Left);
            d.text(720.0, y + 20.0, "Folder", 13.0, false, [0.4; 3], 1.0, Align::Left);
            d.text(860.0, y + 20.0, &format!("{} Oct 2026, 12:{:02}", i % 28 + 1, i), 13.0, false, [0.4; 3], 1.0, Align::Left);
        }
        d.pop_clip();
        d.glass([270.0, 70.0, 560.0, 36.0], 18.0, 1.0, false);
        d.text(290.0, 93.0, "New   Cut   Copy   Paste   Rename   Delete", 13.0, false, [0.2; 3], 1.0, Align::Left);
    }

    /// `cargo test --release -p chebrium --no-default-features bench_ -- --ignored --nocapture`: time recording and
    /// painting of hover, scroll and unchanged frames of `files_like` (what one frame of a list costs the processor).
    #[test]
    #[ignore]
    fn bench_frames() {
        let s = 1.25;
        let mut gfx = Gfx::new();
        let mut f = Frame::default();
        f.resize(1500, 969);
        let mut run = |name: &str, frames: usize, full: bool, scene: &dyn Fn(&mut Recorder, usize)| {
            // warm the caches as a running program has them
            for k in 0..3 {
                let mut r = Recorder::new(&mut gfx, s);
                scene(&mut r, k);
                f.update(r.cmds, &mut gfx, BG, s);
            }
            let t = std::time::Instant::now();
            let mut px = 0u64;
            for k in 0..frames {
                let mut r = Recorder::new(&mut gfx, s);
                scene(&mut r, k);
                if full {
                    f.invalidate();
                }
                if let Some(d) = f.update(r.cmds, &mut gfx, BG, s) {
                    px += ((d[2] - d[0]) * (d[3] - d[1])) as u64;
                }
            }
            let ms = t.elapsed().as_secs_f64() * 1000.0 / frames as f64;
            eprintln!("{name:>10}: {ms:.3} ms a frame, {}k px repainted a frame", px / frames as u64 / 1000);
        };
        run("idle", 200, false, &|r, _| files_like(r, 99, 0.0));
        run("hover", 200, false, &|r, k| files_like(r, k % 18, 0.0));
        run("scroll", 100, false, &|r, k| files_like(r, 99, (k % 50) as f32 * 4.0));
        run("full", 30, true, &|r, _| files_like(r, 99, 0.0));
    }

    #[test]
    fn a_list_scrolled_and_hovered_over_flat_panes_repaints_exactly() {
        let s = 1.25;
        let mut gfx = Gfx::new();
        let scratch = |gfx: &mut Gfx, hover: usize, scroll: f32| {
            let mut r = Recorder::new(gfx, s);
            files_like(&mut r, hover, scroll);
            let cmds = r.cmds;
            let mut f = Frame::default();
            f.resize(1500, 969);
            f.update(cmds, gfx, BG, s);
            f.data
        };
        let mut f = Frame::default();
        f.resize(1500, 969);
        for (k, (hover, scroll)) in [(99, 0.0), (3, 0.0), (4, 0.0), (4, 7.5), (9, 13.0), (99, 2.0)].into_iter().enumerate() {
            let mut r = Recorder::new(&mut gfx, s);
            files_like(&mut r, hover, scroll);
            let cmds = r.cmds;
            f.update(cmds, &mut gfx, BG, s);
            if k > 0 {
                assert!(f.data == scratch(&mut gfx, hover, scroll), "step {k}: hover {hover}, scroll {scroll}");
            }
        }
    }

    #[test]
    fn changes_far_apart_are_repainted_as_separate_boxes_exactly() {
        let mut gfx = Gfx::new();
        // a bar at the top and a pill with glass at the bottom fade in together, over stripes
        let scene = |a: f32| {
            move |d: &mut Recorder| {
                d.fill([0.0, 0.0, 200.0, 150.0], 0.0, [0.1, 0.1, 0.15], 1.0);
                for k in 0..10 {
                    d.fill([k as f32 * 20.0, 0.0, 10.0, 150.0], 0.0, [1.0, 0.3, 0.2], 1.0);
                }
                d.fill([0.0, 0.0, 200.0, 20.0], 0.0, [0.0; 3], 0.45 * a);
                d.text(10.0, 15.0, "name.png", 12.0, true, [1.0; 3], a, Align::Left);
                d.glass([50.0, 120.0, 100.0, 24.0], 12.0, a, true);
            }
        };
        let mut f = Frame::default();
        f.resize(200, 150);
        f.update(frame_of(scene(0.2), &mut gfx), &mut gfx, BG, 1.0);
        let mut data = f.data.clone();
        let boxes = f.update_into(frame_of(scene(0.7), &mut gfx), &mut gfx, BG, 1.0, |_| Some(&mut data[..])).expect("changed");
        assert_eq!(boxes.len(), 2, "{boxes:?}");
        assert!(boxes.iter().all(|b| b[3] <= 40 || b[1] >= 100), "{boxes:?}");
        assert!(data == from_scratch(frame_of(scene(0.7), &mut gfx), &mut gfx));
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
