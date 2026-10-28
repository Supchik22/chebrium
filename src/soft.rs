//! The software painter: [`Draw`] over a [`Canvas`], with the fonts and icons it needs cached in [`Gfx`].
//!
//! This is what makes a Chebrium program light: it needs no GPU, only a shared-memory buffer, and text and
//! icons are rasterised once and reused until they change.

use crate::canvas::{rgb8, Canvas};
use crate::draw::{Align, Draw};
use crate::lucide;
use crate::text::{Raster, TextFonts};
use std::collections::HashMap;

/// Fonts and caches shared by every frame of a program.
pub struct Gfx {
    pub fonts: TextFonts,
    lines: HashMap<(String, u32, bool), Raster>,
    icons: HashMap<(String, u32, [u8; 3]), Vec<u8>>,
}

impl Gfx {
    pub fn new() -> Gfx {
        Gfx { fonts: TextFonts::load(), lines: HashMap::new(), icons: HashMap::new() }
    }

    /// Width of a line of text in logical pixels (rasterised and cached, as it is going to be drawn anyway).
    pub fn text_width(&mut self, text: &str, size: f32, bold: bool) -> f32 {
        self.line(text, size, bold).width
    }

    pub(crate) fn line(&mut self, text: &str, size: f32, bold: bool) -> &Raster {
        // a program that shows endlessly new strings (a clock, a log) must not grow without bound
        if self.lines.len() > 2048 {
            self.lines.clear();
        }
        let key = (text.to_string(), (size * 2.0).round() as u32, bold);
        let fonts = &self.fonts;
        self.lines.entry(key).or_insert_with(|| fonts.rasterize(text, size, bold))
    }

    fn icon(&mut self, name: &str, px: u32, c: [u8; 3]) -> &[u8] {
        self.icons.entry((name.to_string(), px, c)).or_insert_with(|| lucide::rasterize(name, px, c))
    }
}

impl Default for Gfx {
    fn default() -> Gfx {
        Gfx::new()
    }
}

/// Paints logical pixels onto a canvas that is `scale` times larger (a HiDPI buffer).
pub struct SoftDraw<'a, 'b> {
    pub canvas: &'a mut Canvas<'b>,
    pub gfx: &'a mut Gfx,
    scale: f32,
    clips: Vec<[i32; 4]>,
}

impl<'a, 'b> SoftDraw<'a, 'b> {
    pub fn new(canvas: &'a mut Canvas<'b>, gfx: &'a mut Gfx) -> Self {
        SoftDraw { canvas, gfx, scale: 1.0, clips: Vec::new() }
    }

    pub fn scaled(mut self, scale: f32) -> Self {
        self.scale = scale;
        self
    }

    fn rect(&self, r: [f32; 4]) -> [f32; 4] {
        r.map(|v| v * self.scale)
    }
}

impl Draw for SoftDraw<'_, '_> {
    fn fill(&mut self, r: [f32; 4], radius: f32, c: [f32; 3], alpha: f32) {
        let r = self.rect(r);
        self.canvas.fill_round_rect(r[0], r[1], r[2], r[3], radius * self.scale, rgb8(c), alpha);
    }

    fn grad(&mut self, r: [f32; 4], radius: f32, top: [f32; 3], bottom: [f32; 3], alpha: f32) {
        let r = self.rect(r);
        self.canvas.fill_round_rect_grad(r[0], r[1], r[2], r[3], radius * self.scale, rgb8(top), rgb8(bottom), alpha);
    }

    fn glyph(&mut self, cx: f32, cy: f32, size: f32, icon: &str, c: [f32; 3], alpha: f32) {
        let px = (size * self.scale).round().max(1.0) as u32;
        let (cx, cy) = (cx * self.scale, cy * self.scale);
        let SoftDraw { canvas, gfx, .. } = self;
        let bmp = gfx.icon(icon, px, rgb8(c));
        canvas.blit_premul((cx - px as f32 / 2.0).round() as i32, (cy - px as f32 / 2.0).round() as i32, px as usize, px as usize, bmp, alpha);
    }

    fn text(&mut self, x: f32, y: f32, text: &str, size: f32, bold: bool, c: [f32; 3], alpha: f32, align: Align) {
        let (x, y, size) = (x * self.scale, y * self.scale, size * self.scale);
        let SoftDraw { canvas, gfx, .. } = self;
        let r = gfx.line(text, size, bold);
        let ox = match align {
            Align::Left => x,
            Align::Center => x - r.width / 2.0,
            Align::Right => x - r.width,
        };
        canvas.blit_coverage(ox.round() as i32, (y - r.ascent).round() as i32, r.w, r.h, &r.bitmap, rgb8(c), alpha);
    }

    fn measure(&mut self, text: &str, size: f32, bold: bool) -> f32 {
        self.gfx.text_width(text, size * self.scale, bold) / self.scale
    }

    fn glass(&mut self, r: [f32; 4], radius: f32, strength: f32, dark: bool) {
        let r = self.rect(r);
        self.canvas.glass_round_rect(r[0], r[1], r[2], r[3], radius * self.scale, dark, strength, self.scale);
    }

    fn picture(&mut self, r: [f32; 4], pic: &std::sync::Arc<crate::picture::Pic>, src: [f32; 4], alpha: f32) {
        self.canvas.blit_pic(self.rect(r), pic, src, alpha);
    }

    fn push_clip(&mut self, r: [f32; 4]) {
        let r = self.rect(r);
        let mut b = [r[0].floor() as i32, r[1].floor() as i32, (r[0] + r[2]).ceil() as i32, (r[1] + r[3]).ceil() as i32];
        if let Some(top) = self.clips.last() {
            b = [b[0].max(top[0]), b[1].max(top[1]), b[2].min(top[2]), b[3].min(top[3])];
        }
        self.clips.push(b);
        self.canvas.set_clip(b);
    }

    fn pop_clip(&mut self) {
        self.clips.pop();
        match self.clips.last() {
            Some(b) => self.canvas.set_clip(*b),
            None => self.canvas.reset_clip(),
        }
    }
}
